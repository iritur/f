// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-B02`'s exit: a full fill-and-collect cycle, with write amplification
//! recorded.
//!
//! # Which half of the evidence this is, said first
//!
//! The exit reads *a full fill-and-collect cycle on a zoned device **or its
//! emulation**, with write amplification recorded*. **This is the emulation, in
//! the host, and no QEMU boot has run.** The development image's QEMU is
//! bookworm's 7.2 and zoned virtio-blk emulation is materially better from QEMU
//! 8; more to the point, the boot `intent/0006-state/spec.md` describes is of
//! `user/objects` over the blk driver's ring, and `user/objects` is `E2-B08`
//! and does not exist, while `user/virtio-blk` numbers the five zone opcodes
//! and answers two of seven. So the boot is owed and is owed to a task that has
//! not started, and saying which half is missing is worth more than a number
//! taken somewhere else and called the same thing.
//!
//! **The consequence for the number.** What is printed below is *not* claim
//! 0016's `device_bytes_per_app_byte`. That claim counts both sides with QEMU's
//! `query-blockstats` on an emulated zoned device inside a guest, against a
//! tuned Linux baseline on the identical device; a count taken at a different
//! boundary is a different count however similar it looks, and this file
//! registers nothing. What this number is for is the ratio's *shape* — how much
//! is data, how much copy-forward and how much block padding — which is a
//! property of the design rather than of a device, and which is the part a
//! reviewer of `E2-B02` can actually check.
//!
//! # The workload is phased, and that is not a convenience
//!
//! *Fill N zones with the collector held; collect to quiescence with the client
//! idle; count.* Copy-forward bytes depend on the live fraction at the moment
//! each zone is swept, so a collector running concurrently with a client makes
//! the count a function of the interleaving between two components rather than
//! of `(seed, commit)`. RFC 0059 requires the phasing and requires the explicit
//! `COLLECT` verb that makes it expressible: *collect now, and tell me when
//! there is nothing left to do*.
//!
//! # Why a target of its own rather than a `#[test]`
//!
//! `blob/tests/million.rs`'s reason, one crate over: the exit and the
//! per-commit gate differ by a *number* and by nothing else — how many objects
//! are published before the collector is let go — and a libtest harness would
//! swallow the argument that carries it, leaving the two to differ by a build
//! instead.
//!
//! ```text
//! cargo test -p f-zone --test cycle                        # the gate
//! cargo test --release -p f-zone --test cycle -- --objects 256   # a longer run
//! ```
//!
//! # The controls, and why a green run without them proves nothing
//!
//! A fill-and-collect that loses no data is green whether the reset guard is
//! evaluated or ignored, because nothing in a run that behaves ever asks the
//! guard a question it could answer wrongly. So the run ends by asking each
//! predicate a question it must answer *no* to: I1 over a sealed zone that
//! still holds reachable blocks, and the reset guard's third conjunct over a
//! zone a reader has resolved into. A predicate that has never returned false
//! is indistinguishable from one that cannot.

use std::collections::BTreeMap;
use std::process::exit;

use f_abi::store::{RootRecord, Superblock};
use f_abi::{NO_DEADLINE, class};
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{derive, label};
use f_env::{Env, SeededEnv};
use f_zone::collect::{Collector, Progress};
use f_zone::device::{Counted, ZonedMemory};
use f_zone::invariants;
use f_zone::map::{State, ZoneMap};
use f_zone::publish::{Publisher, latest_root};
use f_zone::queue::COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ;
use f_zone::roots::Roots;

/// The name this target answers to when cargo's harness protocol asks for one.
const TEST_NAME: &str = "cycle";

/// Objects published when nothing says otherwise. Unit: count of objects.
///
/// Sixty-four at a mean of 160 KiB is about ten megabytes of application bytes
/// over about eleven zones, which is what every `cargo test --workspace` pays.
/// It is not a token number: it has to fill enough zones that the sweep has
/// candidates to rank, and one zone is not a ranking.
const DEFAULT_OBJECTS: u64 = 64;

/// The seed the draw sites derive from when nothing says otherwise.
/// Unit: none — a seed.
const DEFAULT_SEED: u64 = 1;

/// One generation in this many stays pinned. Unit: count of generations.
///
/// The retention policy the fill runs under, and the whole reason there is
/// anything to collect: every other generation is unpinned as soon as the next
/// one is durable, so the store fills with sealed zones whose live fraction has
/// fallen below `SWEEP_LIVE_FRACTION`. Eight is chosen to put the mean live
/// fraction near an eighth — comfortably under the quarter — so that the sweep
/// has work and the run is not a measurement of a threshold it sits exactly on.
const KEEP_EVERY: u64 = 8;

/// The modelled device's logical block. Unit: bytes.
const BLOCK_BYTES: usize = 512;

/// Blocks in one zone. Unit: count of blocks.
///
/// Two thousand and forty-eight of 512 bytes is a one-mebibyte zone, which is
/// four times `CHUNK_MAX_BYTES` — so a zone holds at least four maximal chunks
/// and the fill is not a sequence of one-record zones.
const ZONE_BLOCKS: u64 = 2048;

/// Zones the superblock and the two root zones occupy, and therefore the first
/// zone the mapping fills. Unit: count of zones.
const DATA_FROM: u32 = 3;

/// Free data zones to leave above what the fill needs. Unit: count of zones.
///
/// The sweep needs somewhere to evacuate into and the client needs somewhere to
/// keep writing, which is `COLLECT_FREE_ZONES_LOW` = 2; the rest is so that the
/// *fill* phase never hits `FULL`, because a fill that ran out would be
/// measuring this file's arithmetic rather than the design's.
const SLACK_ZONES: u32 = 8;

/// How far into the collect phase the client's urgent read is submitted.
/// Unit: count of collector steps.
///
/// It has to be *between* two steps and not before the cycle, because what the
/// bound is about is a read arriving while a collector operation is already
/// inside the device. A small number is chosen so that a run whose cycle turns
/// out shorter than expected fails loudly — the check below is that the cycle
/// outlasted this — rather than quietly never asking I3 anything.
const URGENT_READ_AFTER_STEPS: u64 = 8;

/// The shortest object this run draws. Unit: bytes.
const OBJECT_MIN_BYTES: usize = 128 * 1024;

/// The longest. Unit: bytes.
const OBJECT_MAX_BYTES: usize = 192 * 1024;

/// What one invocation was asked for.
struct Asked {
    /// Unit: count of objects.
    objects: u64,
    /// Unit: none — a seed.
    seed: u64,
    /// Answer the harness protocol's `--list` and run nothing.
    list: bool,
    /// A name filter that selected nothing.
    unselected: Option<String>,
}

/// Parse the command line.
///
/// This run's own options fail closed; cargo's harness protocol does not, for
/// `blob/tests/million.rs`'s reason: `cargo test` hands every target in a
/// package the same arguments, so a bare name filter aimed at another target
/// selects nothing here and exits zero.
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut objects = DEFAULT_OBJECTS;
    let mut seed = DEFAULT_SEED;
    let mut list = false;
    let mut filter: Option<String> = None;

    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        let mut value = || walk.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--objects" => objects = number(&value()?)?,
            "--seed" => seed = number(&value()?)?,
            "--list" => list = true,
            "--nocapture"
            | "--quiet"
            | "-q"
            | "--exact"
            | "--show-output"
            | "--ignored"
            | "--include-ignored"
            | "--force-run-in-process"
            | "--report-time"
            | "--test"
            | "--bench" => {}
            "--test-threads" | "--color" | "--format" | "--logfile" | "--skip" | "-Z" => {
                let _ = value()?;
            }
            other if other.starts_with('-') => return Err(format!("unknown argument: {other}")),
            other => filter = Some(other.to_string()),
        }
    }

    if let Some(name) = filter.filter(|name| !TEST_NAME.contains(name.as_str())) {
        return Ok(Asked { objects, seed, list: false, unselected: Some(name) });
    }
    if objects < KEEP_EVERY * 2 {
        return Err(format!(
            "--objects below {} publishes too few generations for anything to be unpinned, \
             so the collect phase would find nothing and be green having asserted nothing. R04.",
            KEEP_EVERY * 2
        ));
    }
    Ok(Asked { objects, seed, list, unselected: None })
}

/// A decimal or `0x`-prefixed hexadecimal number.
fn number(text: &str) -> Result<u64, String> {
    text.strip_prefix("0x")
        .map_or_else(|| text.parse::<u64>().ok(), |hex| u64::from_str_radix(hex, 16).ok())
        .ok_or_else(|| format!("`{text}` is not a number"))
}

/// One draw site, seeded by identity from the run's seed.
///
/// Split rather than chained under RFC 0026, so that a third draw site added
/// later moves neither of the two that are here — and so that object 40 is the
/// same object whatever the run before it did.
fn site(seed: u64, identity: &str) -> SeededEnv {
    SeededEnv::new(derive(seed, label(identity)), 1)
}

/// How long object `index` is. Unit: bytes.
fn length(lengths: &mut SeededEnv) -> usize {
    OBJECT_MIN_BYTES + (lengths.next_u64() % (OBJECT_MAX_BYTES - OBJECT_MIN_BYTES) as u64) as usize
}

/// `len` drawn bytes.
///
/// Uniform random, which makes two objects sharing a chunk something that does
/// not happen: the run counts device bytes against application bytes, and
/// deduplication between two drawn objects would make the ratio a measurement
/// of the draw.
fn content(bytes: &mut SeededEnv, len: usize, into: &mut Vec<u8>) {
    into.clear();
    while into.len() < len {
        into.extend_from_slice(&bytes.next_u64().to_le_bytes());
    }
    into.truncate(len);
}

/// The device this run needs, sized from the draw rather than guessed at.
fn geometry(seed: u64, objects: u64) -> (u32, Superblock) {
    // An over-estimate on purpose: the exact figure needs the chunker's cuts,
    // and a fill that hit `FULL` would be a failure of this file rather than of
    // the design. Sixteen blocks per object covers each chunk's header padding
    // and the object record itself.
    let mut lengths = site(seed, "e2-b02/length");
    let mut blocks = 1u64;
    for _ in 0..objects {
        blocks += (length(&mut lengths) / BLOCK_BYTES) as u64 + 16;
    }
    let zones = DATA_FROM + (blocks.div_ceil(ZONE_BLOCKS) as u32) + 1 + SLACK_ZONES;
    let layout = superblock_for_this_build(
        BLOCK_BYTES as u32,
        zones,
        ZONE_BLOCKS * BLOCK_BYTES as u64,
        1,
        2,
    );
    (zones, layout)
}

/// Everything one run measured.
struct Measured {
    /// Unit: bytes — bytes the client submitted, the denominator.
    app_bytes: u64,
    /// Unit: bytes — device bytes at the end of the fill phase.
    fill_bytes: u64,
    /// Unit: bytes — device bytes at the end of the collect phase.
    total_bytes: u64,
    /// Unit: bytes — of `total_bytes`, what copy-forward wrote.
    copied_bytes: u64,
    /// Unit: count of zones reset by the collector.
    zones_condemned: u64,
    /// Unit: count of zones the mapping may fill — every zone but the
    /// superblock's and the two root zones.
    ///
    /// Printed beside the count above because *ten zones reset* is a number and
    /// *ten of twenty* is a measurement: what the run has to show is that
    /// the collector reset a material share of the device rather than one zone
    /// and stopped, and a reader cannot tell those apart without the
    /// denominator. `claims/0016` records the pair.
    data_zones: u32,
    /// Unit: count of device operations.
    ops_ahead_of_urgent_read: u32,
    /// Unit: count of steps the collect phase took.
    steps: u64,
    /// What the device was asked for, at the boundary the counts are taken at.
    counted: Counted,
}

impl Measured {
    /// `device_bytes_per_app_byte`, in the parts a reader can check.
    ///
    /// Printed as three ratios rather than one, because the whole point of the
    /// number is its decomposition: 1.0-ish for the data plus block padding,
    /// `f/(1 − f)` for copy-forward, and the root records. A single figure
    /// hides which term moved.
    fn report(&self) {
        let per = |bytes: u64| bytes as f64 / self.app_bytes as f64;
        println!("  application bytes submitted      {:>14}", self.app_bytes);
        println!("  device bytes, fill phase         {:>14}", self.fill_bytes);
        println!("  device bytes, collect phase      {:>14}", self.total_bytes - self.fill_bytes);
        println!("  device bytes, total              {:>14}", self.total_bytes);
        println!("    of which ZONE_APPEND           {:>14}", self.counted.appended_bytes);
        println!("    of which positioned WRITE      {:>14}", self.counted.written_bytes);
        println!();
        println!("  fill bytes per app byte          {:>14.4}", per(self.fill_bytes));
        println!("  copy-forward bytes per app byte  {:>14.4}", per(self.copied_bytes));
        println!("  device_bytes_per_app_byte        {:>14.4}", per(self.total_bytes));
        println!();
        println!(
            "  zones reset by the collector     {:>14}  of {} data zones",
            self.zones_condemned, self.data_zones
        );
        println!("  ZONE_FINISH entries              {:>14}", self.counted.finishes);
        println!("  ZONE_RESET entries               {:>14}", self.counted.resets);
        println!("  FLUSH entries                    {:>14}", self.counted.flushes);
        println!("  collector steps                  {:>14}", self.steps);
        println!("  ops ahead of an urgent read      {:>14}", self.ops_ahead_of_urgent_read);
    }
}

/// One run, and the status it earns.
#[expect(
    clippy::too_many_lines,
    reason = "the phases are fill, collect and count, and they share one store, one root set \
              and one collector. Splitting them into functions would mean passing all three \
              through every signature, and the phasing — which is what makes the number \
              schedule-independent — would stop being readable in one place."
)]
fn run(asked: &Asked) -> i32 {
    let (objects, seed) = (asked.objects, asked.seed);
    println!("cycle — {objects} object(s) into a modelled zoned device, from seed {seed:#018x}");
    println!("  (host emulation; no QEMU boot has run — see this file's first paragraph)");

    // Before anything is written: if I3's bound is no longer the driver's
    // number, every invariant this run goes on to assert is asserting the wrong
    // thing, and finding that out after ten megabytes of work is finding it out
    // late.
    if let Some(status) = the_bound_is_the_drivers_chain_length() {
        return status;
    }

    let (zones, layout) = geometry(seed, objects);
    let device = ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, zones);
    let map = match ZoneMap::new(device, DATA_FROM) {
        Ok(map) => map,
        Err(refusal) => return fail(&format!("the mapping refused the device: {refusal:#x}")),
    };
    let device_blocks = ZONE_BLOCKS * u64::from(zones);
    let mut store = match Store::format(map, &layout) {
        Ok(store) => store,
        Err(refusal) => return fail(&format!("the store refused the device: {refusal:#x}")),
    };
    let mut roots = Roots::new();
    let mut collector = Collector::new(device_blocks);

    // ---- fill, with the collector held ----------------------------------
    let mut lengths = site(seed, "e2-b02/length");
    let mut bytes = site(seed, "e2-b02/content");
    let mut object = Vec::new();
    let mut app_bytes = 0u64;
    let mut kept: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    let mut previous = [0u8; 32];
    let mut last: Option<([u8; 32], u64)> = None;

    for index in 0..objects {
        let len = length(&mut lengths);
        content(&mut bytes, len, &mut object);

        let mut publisher = Publisher::open(&mut roots, &store);
        let root = match store.put_object(&object) {
            Ok(root) => root,
            Err(refusal) => return fail(&format!("object {index} refused: {refusal:#x}")),
        };
        if let Err(refusal) = publisher.appended(&mut roots, &store) {
            return fail(&format!("recording object {index}'s appends refused: {refusal:#x}"));
        }
        let record = RootRecord {
            generation: index + 1,
            root,
            frame: [0u8; 32],
            module: root,
            previous,
            check: [0u8; 32],
        };
        if let Err(refusal) = publisher.commit(&mut store, &mut roots, &record) {
            return fail(&format!("publishing generation {} refused: {refusal:#x}", index + 1));
        }
        app_bytes += len as u64;
        previous = root;

        // Retention: the generation before this one is dropped unless it is one
        // of the kept ones. This is the whole source of garbage — nothing is
        // ever deleted, and a blob becomes unreachable only because the root
        // that named it stopped being pinned.
        if let Some((older, at)) = last.replace((root, index))
            && !at.is_multiple_of(KEEP_EVERY)
        {
            roots.unpin(&older);
        }
        if index.is_multiple_of(KEEP_EVERY) {
            kept.insert(root, index);
        }
    }
    if let Some((root, at)) = last {
        // The newest generation always stays: a store whose most recent publish
        // was collected is a store that lost the thing it was last asked to
        // keep.
        kept.insert(root, at);
    }

    let fill_bytes = store.device().device().counted().device_bytes();
    let sealed_after_fill =
        store.device().zones().filter(|zone| zone.state == State::Sealed).count();
    if sealed_after_fill < 2 {
        return fail(&format!(
            "the fill sealed {sealed_after_fill} zone(s); a sweep with fewer than two \
             candidates ranks nothing, so this run would be green having asserted nothing"
        ));
    }

    // ---- collect to quiescence, with the client idle ---------------------
    // Driven a step at a time rather than by `Collector::collect`, for one
    // reason: an urgent read has to be submitted *between two steps*, while the
    // collector's own operation is inside the device, or I3's witness is zero
    // on every run and the invariant is one nothing can fail. That is the
    // bound's own derivation — an entry already inside the device cannot be
    // overtaken — and it is the only shape in which this run tests it.
    collector.begin(&mut roots);
    let mut steps = 0u64;
    loop {
        let progress = match collector.step(&mut store, &roots) {
            Ok(progress) => progress,
            Err(refusal) => {
                return fail(&format!(
                    "the collector refused its own cycle: {refusal:#x} — which is an invariant \
                     it would have broken, and is the refusal, not a failure to have one"
                ));
            }
        };
        if progress == Progress::Done || progress == Progress::Waiting {
            break;
        }
        steps += 1;
        if steps == URGENT_READ_AFTER_STEPS {
            collector.queue_mut().submit_client(class::HARD, NO_DEADLINE);
        }
    }
    if let Err(refusal) = collector.queue_mut().settle() {
        return fail(&format!("settling the last collector operation refused: {refusal:#x}"));
    }
    if steps <= URGENT_READ_AFTER_STEPS {
        return fail(&format!(
            "the cycle took {steps} step(s), so the urgent read was never submitted and I3 \
             was not asked anything"
        ));
    }

    let published = collector.published(store.device(), &roots);
    let measured = Measured {
        app_bytes,
        fill_bytes,
        total_bytes: store.device().device().counted().device_bytes(),
        copied_bytes: published.copied_bytes,
        zones_condemned: published.zones_condemned,
        data_zones: zones - DATA_FROM,
        ops_ahead_of_urgent_read: published.ops_ahead_of_urgent_read,
        steps,
        counted: *store.device().device().counted(),
    };

    // ---- count, and check what the count is worth ------------------------
    println!();
    measured.report();
    println!();

    if published.zones_condemned == 0 {
        return fail("no zone was reset, so this run collected nothing and proved nothing");
    }
    if published.copied_bytes == 0 {
        return fail("no block was copied forward, so the survivor path never ran");
    }
    if published.unresolved != 0 {
        return fail(&format!(
            "{} hash(es) in the closure of R resolved to nothing — a root naming a blob that \
             is not there",
            published.unresolved
        ));
    }
    if !invariants::collection_never_starves_an_urgent_read(collector.queue()) {
        return fail(&format!(
            "I3: {} collector operation(s) ahead of an urgent read, against a bound of {}",
            published.ops_ahead_of_urgent_read, COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ
        ));
    }
    // And the other side of it, which is what stops I3 being an invariant
    // nothing can fail: the read really was behind the collector's in-flight
    // operation, so the witness is the bound rather than zero. A run reporting
    // zero here has not tested I3, it has failed to reach it.
    if published.ops_ahead_of_urgent_read != COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ {
        return fail(&format!(
            "the urgent read was behind {} collector operation(s) and not {} — I3 held \
             because nothing was ever ahead of anything, which tests nothing",
            published.ops_ahead_of_urgent_read, COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ
        ));
    }
    if !invariants::a_reset_zone_holds_no_live_blob(store.device()) {
        return fail("I2: a reset zone still resolves");
    }

    // Every kept generation still reads back, chunk by chunk, verified against
    // its own name. This is what says copy-forward moved bytes and repointing
    // moved names: a survivor that had been relocated and not repointed would
    // refuse here with `ADDRESS`, and one relocated wrongly with `CONTENT`.
    let mut lengths = site(seed, "e2-b02/length");
    let mut bytes = site(seed, "e2-b02/content");
    let mut read = vec![0u8; OBJECT_MAX_BYTES];
    let mut verified = 0u64;
    for index in 0..objects {
        let len = length(&mut lengths);
        content(&mut bytes, len, &mut object);
        let Some((root, _)) = kept.iter().find(|(_, at)| **at == index).map(|(r, a)| (*r, *a))
        else {
            continue;
        };
        match store.get_object(&root, &mut read) {
            Ok(moved) if moved == len && read[..len] == object[..] => verified += 1,
            Ok(moved) => return fail(&format!("generation {index} read back {moved} of {len}")),
            Err(refusal) => {
                return fail(&format!("generation {index} refused after collection: {refusal:#x}"));
            }
        }
    }
    if verified != kept.len() as u64 {
        return fail(&format!("{verified} of {} kept generations verified", kept.len()));
    }
    println!("  {verified} kept generation(s) read back and verified after collection");

    // The newest root record is still on the device and still verifies, which
    // is the half of verify-before-accept this crate has.
    match latest_root(&mut store) {
        Ok(Some(found)) if found.generation == objects => {}
        Ok(Some(found)) => {
            return fail(&format!(
                "the latest root is generation {}, not {objects}",
                found.generation
            ));
        }
        Ok(None) => return fail("no root record on the device verifies"),
        Err(refusal) => return fail(&format!("scanning the root zones refused: {refusal:#x}")),
    }

    // ---- the controls ----------------------------------------------------
    // Before the adversarial case rather than after it, because a control over
    // I1 needs the mark to be current and the adversarial publish pins a root
    // *after* the cycle it runs across. Asking I1 about a stale mark would get
    // "no" for a reason that is not the one the control is about, which is a
    // control that passes for the wrong reason on the day it should fail.
    if let Some(status) = controls(&mut store, &roots, &collector) {
        return status;
    }

    // ---- the adversarial case: a publish open across a sweep -------------
    if let Some(status) = adversarial(&mut store, &mut roots, &mut collector, seed, objects) {
        return status;
    }

    println!("cycle: ok");
    0
}

/// A multi-zone publish in flight during a sweep.
///
/// The case RFC 0059's transient root exists for, and the one `E2-P03` names as
/// its adversarial allocation. A publish large enough to fill a zone seals a
/// zone whose live fraction against the *pinned* roots is zero — it is the
/// emptiest zone in the store, by construction — and without the transient
/// entry the collector selects it, evacuates nothing, resets it, and the root
/// record lands afterwards naming a zone that no longer holds anything.
///
/// Answers `Some(status)` on failure and `None` when the case held.
fn adversarial(
    store: &mut Store<ZoneMap<ZonedMemory>>,
    roots: &mut Roots,
    collector: &mut Collector,
    seed: u64,
    objects: u64,
) -> Option<i32> {
    let mut bytes = site(seed, "e2-b02/adversarial");
    let mut object = Vec::new();
    let sealed_before = store.device().zones().filter(|zone| zone.state == State::Sealed).count();

    let mut publisher = Publisher::open(roots, store);
    let mut written: Vec<(usize, [u8; 32])> = Vec::new();
    // Enough to fill more than one zone, so that the publish seals a zone whose
    // pinned live fraction is zero — which is the whole shape of the case.
    let per_zone = (ZONE_BLOCKS * BLOCK_BYTES as u64) as usize;
    let mut submitted = 0usize;
    while submitted < per_zone * 2 {
        content(&mut bytes, OBJECT_MAX_BYTES, &mut object);
        let root = match store.put_object(&object) {
            Ok(root) => root,
            Err(refusal) => return Some(fail(&format!("the open publish refused: {refusal:#x}"))),
        };
        if let Err(refusal) = publisher.appended(roots, store) {
            return Some(fail(&format!("recording the open publish refused: {refusal:#x}")));
        }
        written.push((OBJECT_MAX_BYTES, root));
        submitted += OBJECT_MAX_BYTES;
    }
    let sealed_during = store.device().zones().filter(|zone| zone.state == State::Sealed).count();
    if sealed_during <= sealed_before {
        return Some(fail(
            "the open publish sealed no zone, so the adversarial case did not arise and this \
             run would be green having tested the easy shape",
        ));
    }

    // A full cycle with the write set open. The collector must not refuse — a
    // refusal here means it would have broken I1 — and must not take the
    // publish's zones.
    let steps = match collector.collect(store, roots) {
        Ok(steps) => steps,
        Err(refusal) => {
            return Some(fail(&format!(
                "the collector refused a cycle run against an open publish: {refusal:#x} — \
                 which is I1 declining to be broken, and means the transient root is not \
                 protecting what it should"
            )));
        }
    };

    let record = RootRecord {
        generation: objects + 1,
        root: written.last().map_or([0u8; 32], |(_, root)| *root),
        frame: [0u8; 32],
        module: [0u8; 32],
        previous: [0u8; 32],
        check: [0u8; 32],
    };
    if let Err(refusal) = publisher.commit(store, roots, &record) {
        return Some(fail(&format!("committing the open publish refused: {refusal:#x}")));
    }

    // And every blob it wrote is still there. Without the transient entry this
    // is where the run would go red, with `ADDRESS` from a zone the collector
    // reset while the publish was open.
    let mut read = vec![0u8; OBJECT_MAX_BYTES];
    for (len, root) in &written {
        match store.get_object(root, &mut read) {
            Ok(moved) if moved == *len => {}
            Ok(moved) => {
                return Some(fail(&format!("the open publish read back {moved} of {len}")));
            }
            Err(refusal) => {
                return Some(fail(&format!(
                    "a blob written by the open publish refused after a sweep: {refusal:#x} — \
                     the collector took data a transient root was protecting"
                )));
            }
        }
    }
    println!(
        "  a publish spanning {} zone(s) survived a {steps}-step cycle taken while it was open",
        sealed_during - sealed_before
    );
    None
}

/// Ask each predicate a question it must answer *no* to.
///
/// A predicate that has never returned false is indistinguishable from one that
/// cannot. This is `blob/tests/million.rs`'s flipped bit, one level up: there
/// the control is a corrupted byte and a refused read; here it is a state the
/// invariant forbids and a predicate that says so.
fn controls(
    store: &mut Store<ZoneMap<ZonedMemory>>,
    roots: &Roots,
    collector: &Collector,
) -> Option<i32> {
    let Some(sealed) = store
        .device()
        .zones()
        .find(|zone| zone.state == State::Sealed && zone.live_bytes > 0)
        .map(|zone| zone.zone)
    else {
        return Some(fail(
            "no sealed zone holds reachable bytes, so I1 has nothing to refuse and the \
             control cannot run",
        ));
    };
    if invariants::nothing_reachable_is_swept(store.device(), collector.mark(), roots, sealed) {
        return Some(fail(&format!(
            "I1 permitted a reset of zone {sealed}, which holds reachable blocks — the \
             predicate is not discriminating and every green run above it is worthless"
        )));
    }

    let Some(free) =
        store.device().zones().find(|zone| zone.state == State::Free).map(|zone| zone.zone)
    else {
        return Some(fail("no free zone, so the positive half of the I1 control cannot run"));
    };
    if !invariants::nothing_reachable_is_swept(store.device(), collector.mark(), roots, free) {
        return Some(fail(&format!("I1 refused a reset of zone {free}, which holds nothing")));
    }

    if !invariants::no_reader_believes_the_zone(store.device(), free) {
        return Some(fail(&format!("zone {free} began the control with a reader outstanding")));
    }
    if let Err(refusal) = store.device_mut().hold_read(free) {
        return Some(fail(&format!("taking a read on zone {free} refused: {refusal:#x}")));
    }
    if invariants::no_reader_believes_the_zone(store.device(), free) {
        return Some(fail(&format!(
            "the reset guard's third conjunct permitted zone {free} with a reader outstanding"
        )));
    }
    if let Err(refusal) = store.device_mut().release_read(free) {
        return Some(fail(&format!("releasing the read on zone {free} refused: {refusal:#x}")));
    }

    println!("  the controls: I1 refused a live zone, and the reset guard refused a held one");
    None
}

/// Where the driver keeps the number I3's bound is derived from.
const DRIVER_PENDING: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../user/virtio-blk/src/pending.rs");

/// RFC 0059's bound is `f_virtio_blk::pending::IN_FLIGHT`, and this is what
/// makes that a fact rather than a transcription.
///
/// # Why a file read and not a dependency
///
/// `zone/Cargo.toml` argues it at length: a dev-dependency on `f-virtio-blk`
/// links a second `#[panic_handler]` into this binary under
/// `cargo clippy --workspace --all-targets`, because that run builds the driver
/// as a member with its own default features. Reading the source is the shape
/// `xtask`'s `the_record_layout_matches_the_abi` already uses against
/// `abi/src/transfer.rs`, and it has the property the dependency was wanted
/// for: the day `E1-B09` raises `IN_FLIGHT` without re-deriving this bound in
/// the same diff, this run goes red and says so, which is exactly what RFC 0059
/// asks of that task.
///
/// It is textual, which bounds what it can claim: a constant moved to another
/// file, or spelled with a different type, goes past it. That is the same limit
/// every textual check in this tree states about itself, and it is worth
/// stating rather than pretending is closed.
fn the_bound_is_the_drivers_chain_length() -> Option<i32> {
    let needle = "pub const IN_FLIGHT: u32 = ";
    let text = match std::fs::read_to_string(DRIVER_PENDING) {
        Ok(text) => text,
        Err(why) => return Some(fail(&format!("reading {DRIVER_PENDING}: {why}"))),
    };
    let Some(at) = text.find(needle) else {
        return Some(fail(&format!(
            "`{needle}` is not in {DRIVER_PENDING} — the constant I3's bound is derived from \
             has moved, and this check has stopped checking anything"
        )));
    };
    let rest = &text[at + needle.len()..];
    let Some(end) = rest.find(';') else {
        return Some(fail("the driver's IN_FLIGHT has no terminating semicolon"));
    };
    let found = match rest[..end].trim().parse::<u32>() {
        Ok(found) => found,
        Err(why) => return Some(fail(&format!("the driver's IN_FLIGHT is not a number: {why}"))),
    };
    if found != COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ {
        return Some(fail(&format!(
            "the driver keeps {found} operation(s) inside the device and I3's bound is {}. \
             RFC 0059: the row moves when `E1-B09` raises `IN_FLIGHT`, re-derived in the same \
             diff — one constant, two claims and one invariant reading it",
            COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ
        )));
    }
    None
}

/// Report a failure the way this run reports one, and answer its status.
fn fail(why: &str) -> i32 {
    println!("cycle: FAILED — {why}");
    1
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = match parse(&args) {
        Ok(asked) => asked,
        Err(why) => {
            eprintln!("cycle: {why}");
            exit(2);
        }
    };
    if let Some(name) = asked.unselected {
        println!("running 0 tests  (filter `{name}` selected nothing in this target)");
        exit(0);
    }
    if asked.list {
        println!("{TEST_NAME}: test");
        println!("1 test, 0 benchmarks");
        exit(0);
    }
    exit(run(&asked));
}
