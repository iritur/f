// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-P03`: RFC 0059's three invariants, asserted as predicates over named
//! state at every step of a run in which collection, adversarial allocation and
//! a hard-class reader are interleaved.
//!
//! # What this asserts, and what it deliberately does not
//!
//! The exit is *all three hold while collection runs concurrently with
//! adversarial allocation and a hard-class reader*, and the thing that makes
//! that assertable rather than aspirational is RFC 0059's decision to write all
//! three as predicates over named state — the root set `R`, the mark bitmap
//! `B`, each zone's state and live byte count, and the driver's queue. So this
//! file asserts **the predicates**, at the instants the RFC states them, and not
//! a symptom:
//!
//! - **I1 — nothing reachable is swept.** `blobs(z) ∩ reach(R) = ∅` at the
//!   instant `ZONE_RESET(z)` is submitted. Asserted twice over, on purpose: once
//!   as the mechanism RFC 0059 says implies it — `marked_roots ⊇ R` with every
//!   bit of `B` in `z`'s range clear, which is
//!   [`invariants::nothing_reachable_is_swept`] — and once as the formula
//!   itself, over [`reach`], a walk this file does from the store's own
//!   hash-to-block map rather than from the collector's bitmap. The second is
//!   what stops the assertion being the collector marking its own homework: a
//!   mark that was wrong would satisfy the mechanism and fail the formula.
//! - **I2 — a reset zone holds no live blob.** `state(z) = free → ∀h :
//!   location(h) = (z, ·) is undefined`, over every zone, at every step
//!   boundary in the run — which is what *at every instant* means for a model
//!   whose instants are its steps. Plus the reset guard's third conjunct,
//!   `reads_outstanding(z) = 0`, captured *before* each step and asserted
//!   against what the step did: a `Reset` whose zone had a reader outstanding
//!   an instant earlier is the guard having failed, whatever the bytes say.
//! - **I3 — collection never starves a deadline-class read.**
//!   `collector_operations_ahead_of_hard_read ≤ 1`, at every step boundary,
//!   against the queue's own witness — and the run fails if that witness is
//!   *zero*, because a bound nothing ever approached is a bound this run did
//!   not test.
//!
//! What it does not assert is a timing. RFC 0059 says why in as many words: a
//! bound in time would be a clock, RFC 0004 says where clocks live, and every
//! timing in this project is `pending` on `E0-D10`'s machine. The currency here
//! is device operations, and the reversal condition that would change it — a
//! hard-class read missing its deadline with the count at 1, root-caused to the
//! *size* of the operation ahead of it — is not observable from a host model
//! and is not claimed to be.
//!
//! # What makes the workload adversarial
//!
//! A collector run against a quiescent store is a collector run against the
//! easy shape. Between every two collector steps this run lets a seeded
//! adversary take one client action, drawn through `f_env`'s `Scheduler` under
//! RFC 0004 so that a failure reproduces from `(seed, commit)`:
//!
//! - **allocation racing collection** — a publish that opens, writes an object,
//!   appends a root record and pins it, *while a cycle is in progress*. That is
//!   `R` growing mid-cycle, which RFC 0059 handles by marking roots added during
//!   a cycle within it, before any sweep in that cycle may condemn.
//! - **a multi-zone publish in flight during a sweep** — the case the first
//!   invariant's open-publish clause exists for, and the one the plan names.
//!   Opened *before* the first cycle begins and held until it has spanned more
//!   than one zone of its own **and** survived two `ZONE_RESET`s, so that it is
//!   in flight across sweeps rather than overlapping the tail of one. Without
//!   the transient entry the collector selects the zone it is filling — that
//!   zone is the emptiest in the store, by construction — resets it, and the
//!   root record lands afterwards naming a zone that no longer holds anything;
//!   the run goes red at the read-back after the commit.
//! - **removal mid-cycle** — an unpin, which schedules the full re-mark and
//!   makes every `live_bytes(z)` an upper bound until it runs. RFC 0059 says a
//!   removal is not applied until the cycle ends, and the direction is
//!   conservative: the sweep reclaims late, never early.
//! - **a hard-class reader** — an entry at `class::HARD` submitted into the
//!   same queue the collector's own operations go through, together with a
//!   *belief*: the reader resolves a pinned root to a zone and holds
//!   `reads_outstanding` on it across steps, which is what the reset guard's
//!   third conjunct is about. It is a real read as well as a queue entry: the
//!   bytes come back through the map and are verified against their own name.
//! - **batch-class client noise**, which I3 does not protect and which must not
//!   perturb the witness.
//! - **a backlog of the collector's own operations**, queued so that the next
//!   urgent entry has something to overtake. Without it I3 is unfalsifiable:
//!   `Queue::collector_operation` drives one operation at a time, so with a
//!   backlog of one the witness is bounded by the device's depth whatever the
//!   queue's ordering does, and the assertion passes on a queue that has
//!   stopped ordering by `f_abi::deadline::Inherited::rank`. That was measured
//!   rather than assumed — the ordering was replaced by arrival order and this
//!   run stayed green until the backlog existed.
//!
//! # Three things this run found that are not invariant failures
//!
//! Each is reported here rather than fixed, because each is a property of
//! `E2-B02`'s build or of RFC 0059's text and not of this property, and a
//! property test that edited the thing it is asserting against would be
//! asserting itself.
//!
//! - **`Publisher::appended` over-attributes under interleaved publishes.** Its
//!   doc comment says every block between the store's `free` when the publish
//!   opened and its `free` now was appended by *this* publish, and that is exact
//!   only while one publish is open. This run has several: a publish that
//!   commits between two of the held-open publish's appends has its blocks
//!   swept into the held-open publish's transient entry. The direction is
//!   conservative — `T` becomes a superset, so I1 protects more and never less —
//!   which is why it is a note and not a red run; what it costs is collection,
//!   because a zone the transient entry over-claims is a zone the sweep will
//!   not take. It is why this file measures the held publish's span from *its
//!   own* appends and not from `Roots::transient_blocks`.
//! - **`location` is not gated on condemnation.** RFC 0059 argues that the
//!   reset guard terminates because "condemnation stops `location` from handing
//!   out `z`, so after condemnation it is monotone non-increasing".
//!   [`ZoneMap`] has no such gate: a reader may resolve into a condemned zone
//!   and take `reads_outstanding` on it, and nothing refuses. The guard is
//!   still correct — it refuses the reset — but its *termination* rests on a
//!   caller convention rather than on the map, so a client that kept resolving
//!   into a condemned zone would hold the reset off indefinitely. This run
//!   keeps the convention: every belief it takes is taken before the zone it
//!   names is condemned.
//! - **I3 cannot be exceeded by the collector alone**, which is why the
//!   backlog above exists. Said again here because it is the sharpest thing
//!   this run learned about the invariant it was written to assert.
//!
//! # Why the instants are step boundaries
//!
//! This crate has no threads and must not have any: RFC 0004 says ordering
//! reaches the system through `Env` and nowhere else, so *concurrent* here means
//! *interleaved at a stated granularity* rather than *racing on a wall clock*.
//! The granularity is one collector step — at most one device operation plus
//! bounded computation, which is exactly the unit RFC 0059 says a cycle is a
//! sequence of — and one client action between each pair. Two runs of one seed
//! interleave identically, which is the property that makes a failure a
//! reproduction rather than an anecdote.
//!
//! # The controls
//!
//! A predicate that has never returned false is indistinguishable from one that
//! cannot, so the run ends by asking each of the three a question it must answer
//! *no* to, and fails if any of them says yes. It also fails if the
//! reachable set the oracle computed was empty, if no zone was ever reset, if
//! the multi-zone publish never spanned two zones, if no reset ever happened
//! while it was open, or if I3's witness stayed at zero — each of which is a way
//! for a green run to have asserted nothing.
//!
//! ```text
//! cargo test -p f-zone --test invariants                    # the gate
//! cargo test --release -p f-zone --test invariants -- --seeds 32
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::process::exit;

use f_abi::store::{Header, ObjectHead, RootRecord, Superblock, kind, refusal};
use f_abi::{NO_DEADLINE, class};
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{derive, label};
use f_env::{Env, Scheduler, SeededEnv};
use f_zone::collect::{Collector, Progress};
use f_zone::device::ZonedMemory;
use f_zone::invariants;
use f_zone::map::{State, ZoneMap};
use f_zone::mark::Mark;
use f_zone::publish::Publisher;
use f_zone::queue::COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ;
use f_zone::roots::Roots;

/// The name this target answers to when cargo's harness protocol asks for one.
const TEST_NAME: &str = "invariants";

/// Seeds run when nothing says otherwise. Unit: count of seeds.
///
/// Four rather than one because the interleaving is drawn: a property asserted
/// against a single schedule is a property asserted against a schedule. Four
/// keeps the per-commit gate near the cost of `zone/tests/cycle.rs`, which is
/// the budget a gate has; a sweep passes `--seeds`.
const DEFAULT_SEEDS: u64 = 4;

/// The seed the draw sites derive from when nothing says otherwise.
/// Unit: none — a seed.
const DEFAULT_SEED: u64 = 1;

/// The modelled device's logical block. Unit: bytes.
const BLOCK_BYTES: usize = 512;

/// Blocks in one zone. Unit: count of blocks.
///
/// Five hundred and twelve of 512 bytes is a 256 KiB zone. Smaller than
/// `zone/tests/cycle.rs`'s mebibyte on purpose: that run is measuring write
/// amplification and wants a zone that holds four maximal chunks, this one is
/// asserting predicates and wants *many zones*, because every zone is another
/// selection, another condemnation and another reset instant at which I1 is
/// evaluated. It also puts a record across a zone boundary routinely, which is
/// the case `zone/src/mark.rs` departs from RFC 0059's bitmap-of-starts for.
const ZONE_BLOCKS: u64 = 512;

/// Zones the superblock and the two root zones occupy, and therefore the first
/// zone the mapping fills. Unit: count of zones.
const DATA_FROM: u32 = 3;

/// Free data zones to leave above what the draw needs. Unit: count of zones.
///
/// The sweep needs somewhere to evacuate into and the client needs somewhere to
/// keep writing, which is `COLLECT_FREE_ZONES_LOW` = 2; the rest is so that no
/// phase of this run hits `FULL`, because a run that ran out of address space
/// would be measuring this file's arithmetic rather than the design's. The
/// mapping's logical addresses are never reused — `zone/src/map.rs` states that
/// limitation — so the device is sized for everything the run will ever write
/// and not for what it holds at once.
const SLACK_ZONES: u32 = 8;

/// Generations published before the concurrent phase starts.
/// Unit: count of objects.
///
/// The garbage the collector is given something to do with. Twenty-four at a
/// mean of 72 KiB is about 1.7 MB over seven 256 KiB zones, which is enough
/// zones that selection ranks something rather than picking the only candidate.
const FILL_OBJECTS: u64 = 24;

/// Generations published *during* the concurrent phase, between collector
/// steps. Unit: count of objects.
const CONCURRENT_OBJECTS: u64 = 12;

/// Objects the multi-zone publish may write before the run gives up on it
/// spanning a second zone. Unit: count of objects.
const OPEN_PUBLISH_MAX_OBJECTS: u64 = 16;

/// Zones the multi-zone publish's own blocks must span before it may commit.
/// Unit: count of zones.
///
/// Two, because *more than one* is the whole shape of the case: a publish that
/// fits in one zone is protected by that zone being open, and a publish that
/// does not is protected by nothing except the transient root.
const OPEN_PUBLISH_ZONES: usize = 2;

/// Zone resets the multi-zone publish must survive before it may commit.
/// Unit: count of resets.
///
/// Two rather than one, because *in flight during a sweep* is a claim about a
/// publish that outlives a sweep and not about one that happened to overlap the
/// end of one. The publish is opened before the first cycle begins, so every
/// reset that cycle takes is a reset taken while a transient entry names blocks
/// the sweep is ranking zones against.
const OPEN_PUBLISH_RESETS: u64 = 2;

/// Cycles one seed's run must begin before it counts as having exercised the
/// seam between two of them. Unit: count of cycles.
///
/// A removal — an unpin, or an abandoned write set — is not applied until the
/// cycle ends, and `Collector::begin` is where the full re-mark it scheduled
/// runs. One cycle asserts the three inside a cycle and nothing about the seam,
/// which is where `marked_roots` is thrown away and rebuilt.
const MIN_CYCLES: u64 = 3;

/// One generation in this many stays pinned. Unit: count of generations.
///
/// The retention policy, and the whole source of garbage: nothing is ever
/// deleted, and a blob becomes unreachable only because the root that named it
/// stopped being pinned. Six puts the mean live fraction near a sixth,
/// comfortably under `SWEEP_LIVE_FRACTION` = 0.25, so the sweep has work and
/// the run is not a measurement of a threshold it sits exactly on.
const KEEP_EVERY: u64 = 6;

/// How many steps a reader's belief survives before it is released.
/// Unit: count of ticks.
///
/// Bounded because `Collector::step` answers `Waiting` for as long as a reader
/// believes the condemned zone is live, and a reader that never released would
/// be a livelock rather than a test. RFC 0059's own argument that the guard
/// terminates is that `reads_outstanding` is monotone non-increasing after
/// condemnation; this is the reader holding up its end of it.
const READER_HOLD_TICKS: u64 = 6;

/// How often the run forces the client's outstanding work forward whatever the
/// draw said. Unit: count of ticks.
///
/// Without it a seed whose draws never choose the publish action runs until the
/// tick ceiling and fails for a reason that is about the draw rather than about
/// the design. The forcing does not make the schedule deterministic in the sense
/// that matters — *when* within the window is still drawn, and so is everything
/// else — it bounds the run.
const FORCE_EVERY: u64 = 12;

/// The tick ceiling. Unit: count of ticks.
///
/// A run that reaches it has not finished its work, which is a failure and not a
/// pass: a loop that gave up quietly would be a run that asserted whatever it
/// happened to reach.
const MAX_TICKS: u64 = 400_000;

/// The shortest object this run draws. Unit: bytes.
const OBJECT_MIN_BYTES: usize = 48 * 1024;

/// The longest. Unit: bytes.
const OBJECT_MAX_BYTES: usize = 96 * 1024;

/// Zones that must be reset in one seed's run before it counts as having
/// collected anything. Unit: count of zones.
const MIN_RESETS: u64 = 2;

/// Zones the long-lived reader takes a belief on, before any of them has been
/// condemned. Unit: count of zones.
///
/// The reset guard's third conjunct is only exercised when a reader believes in
/// the zone the sweep is about to reset, and leaving that to the draw makes it
/// a coincidence: over 32 seeds it happened 25 times, and over 2 seeds it
/// happened not at all — a vacuity check that fires on the seed count rather
/// than on the design. So the reader takes a belief on the first few *sweep
/// candidates* — sealed, non-empty, under the threshold — before the sweep
/// reaches any of them, and holds each until the collector has answered
/// `Waiting` on it. `live_bytes(z) > 0` is exactly the statement that a
/// reachable blob resolves into `z`, so the belief is a resolution the reader is
/// entitled to; four of them is enough that whichever candidate the sweep takes
/// first is one of them, without depending on the order it takes them in.
const WATCHED_ZONES: usize = 4;

/// Collector operations the adversary queues at once, so that an urgent entry
/// has something to overtake. Unit: count of device operations.
///
/// Three, which is the smallest number that is neither the bound nor the
/// bound plus one: if the queue stopped ordering by
/// `f_abi::deadline::Inherited::rank`, an urgent entry submitted behind a
/// backlog of three would complete with three collector operations ahead of it,
/// and I3 would report 3 against a bound of 1. With one, I3 is bounded by the
/// device's depth whatever the ordering does — which is an invariant that
/// cannot fail, and RFC 0059 wrote three predicates to avoid exactly that.
const COLLECTOR_BACKLOG: u32 = 3;

/// What one invocation was asked for.
struct Asked {
    /// Unit: count of seeds.
    seeds: u64,
    /// Unit: none — the first seed; the rest are it plus one, and so on.
    seed: u64,
    /// Answer the harness protocol's `--list` and run nothing.
    list: bool,
    /// A name filter that selected nothing.
    unselected: Option<String>,
}

/// Parse the command line.
///
/// This run's own options fail closed; cargo's harness protocol does not, for
/// `zone/tests/cycle.rs`'s reason: `cargo test` hands every target in a package
/// the same arguments, so a bare name filter aimed at another target selects
/// nothing here and exits zero.
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut seeds = DEFAULT_SEEDS;
    let mut seed = DEFAULT_SEED;
    let mut list = false;
    let mut filter: Option<String> = None;

    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        let mut value = || walk.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--seeds" => seeds = number(&value()?)?,
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
        return Ok(Asked { seeds, seed, list: false, unselected: Some(name) });
    }
    if seeds == 0 {
        return Err("--seeds 0 asserts nothing".to_string());
    }
    Ok(Asked { seeds, seed, list, unselected: None })
}

/// A decimal or `0x`-prefixed hexadecimal number.
fn number(text: &str) -> Result<u64, String> {
    text.strip_prefix("0x")
        .map_or_else(|| text.parse::<u64>().ok(), |hex| u64::from_str_radix(hex, 16).ok())
        .ok_or_else(|| format!("`{text}` is not a number"))
}

/// One draw site, seeded by identity from the run's seed.
///
/// Split rather than chained under RFC 0026, so that a fourth draw site added
/// later moves neither of the three that are here — and so that object 9 is the
/// same object whatever the schedule before it did.
fn site(seed: u64, identity: &str) -> SeededEnv {
    SeededEnv::new(derive(seed, label(identity)), 1)
}

/// How long the next object is. Unit: bytes.
fn length(lengths: &mut SeededEnv) -> usize {
    OBJECT_MIN_BYTES + (lengths.next_u64() % (OBJECT_MAX_BYTES - OBJECT_MIN_BYTES) as u64) as usize
}

/// `len` drawn bytes.
///
/// Uniform random, so that two objects share no chunk: this run is about
/// reachability and a deduplicated chunk is one blob two roots reach, which
/// would make *whose* garbage a zone holds a question about the draw.
fn content(bytes: &mut SeededEnv, len: usize, into: &mut Vec<u8>) {
    into.clear();
    while into.len() < len {
        into.extend_from_slice(&bytes.next_u64().to_le_bytes());
    }
    into.truncate(len);
}

/// The device this run needs, sized from the draw rather than guessed at.
///
/// An over-estimate on purpose, and in the direction that cannot make the run
/// lie: too many zones costs time, too few costs a `FULL` that would be a
/// failure of this file. Every object the run can possibly write is counted,
/// including the ones the multi-zone publish stops short of writing.
fn geometry(seed: u64) -> (u32, Superblock) {
    let mut lengths = site(seed, "e2-p03/length");
    let mut open_lengths = site(seed, "e2-p03/open-length");
    let mut blocks = 1u64;
    for _ in 0..FILL_OBJECTS + CONCURRENT_OBJECTS {
        blocks += (length(&mut lengths) / BLOCK_BYTES) as u64 + 16;
    }
    for _ in 0..OPEN_PUBLISH_MAX_OBJECTS {
        blocks += (length(&mut open_lengths) / BLOCK_BYTES) as u64 + 16;
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

/// `reach(R)`, as the set of logical blocks the closure of the root set
/// occupies — **computed from the store and not from the collector's mark**.
///
/// # Why this exists when `B` already says the same thing
///
/// Because I1's mechanism (`marked_roots ⊇ R`, and `B` clear over the zone) is
/// only equivalent to I1's formula (`blobs(z) ∩ reach(R) = ∅`) if the mark is
/// right, and a run that checked the mechanism alone would pass on a collector
/// whose walk missed a subtree. RFC 0059 authorises `E2-P03` to check the
/// mechanism; checking the formula beside it costs one walk per reset instant
/// and is what makes the mechanism's own correctness part of what is asserted.
///
/// The walk is this file's, from `Store::address` and `Store::head` — the
/// hash-to-block map the store keeps and the record headers on the device — and
/// it stops at an already-visited hash for the same reason the collector's does:
/// a blob's content is fixed at the moment it is written, so no edge under a
/// hash can have changed.
///
/// Transient entries are unioned in as themselves. They have no hash — the hash
/// that will name them is the thing that does not exist yet — so the only
/// statement available about them is the one RFC 0059 makes: the blocks an open
/// write set holds are in `R`.
///
/// # Errors
///
/// Whatever the store says about a read. A refusal here on a hash the store has
/// an address for is *itself* the violation this walk is looking for — it means
/// the blocks that record occupies are no longer mapped, which is a reachable
/// record in a zone somebody reset — and the caller reports it as one.
fn reach(
    store: &mut Store<ZoneMap<ZonedMemory>>,
    roots: &Roots,
) -> Result<BTreeSet<u64>, ([u8; 32], i32)> {
    let mut out = BTreeSet::new();
    let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
    let mut pending: Vec<[u8; 32]> = roots.pinned().copied().collect();
    let block_bytes = u64::from(store.superblock().block_bytes);

    while let Some(hash) = pending.pop() {
        if !seen.insert(hash) {
            continue;
        }
        // A hash the store holds nothing under is not a block anybody can have
        // swept, so it is not this walk's business; the collector counts it in
        // `unresolved` and the run checks that count separately.
        let Some(start) = store.address(&hash) else { continue };
        let header = store.head(&hash).map_err(|why| (hash, why))?;
        let blocks = (Header::BYTES as u64 + header.content_bytes).div_ceil(block_bytes);
        for offset in 0..blocks {
            out.insert(start + offset);
        }
        if header.kind != kind::OBJECT {
            continue;
        }
        let mut record = vec![0u8; header.content_bytes as usize];
        store.get(&hash, &mut record).map_err(|why| (hash, why))?;
        if record.len() < ObjectHead::HEAD_BYTES {
            return Err((hash, refusal::MALFORMED));
        }
        let mut raw = [0u8; ObjectHead::HEAD_BYTES];
        raw.copy_from_slice(&record[..ObjectHead::HEAD_BYTES]);
        let head = ObjectHead::from_bytes(&raw).map_err(|why| (hash, why))?;
        for index in 0..head.chunks as usize {
            let at = ObjectHead::HEAD_BYTES + index * 32;
            let mut child = [0u8; 32];
            child.copy_from_slice(&record[at..at + 32]);
            pending.push(child);
        }
    }
    out.extend(roots.transient_blocks());
    Ok(out)
}

/// What one seed's run measured. Every field is reported, because a claim
/// registrar reading *the invariants held* needs to know what they held against.
#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    /// Unit: count of collector cycles begun.
    cycles: u64,
    /// Unit: count of collector steps taken.
    steps: u64,
    /// Unit: count of client actions taken between collector steps.
    actions: u64,
    /// Unit: count of zones condemned — one sweep is one zone.
    sweeps: u64,
    /// Unit: count of `ZONE_RESET` submissions.
    resets: u64,
    /// Unit: count of resets submitted while a multi-zone publish was open.
    resets_while_open: u64,
    /// Unit: count of steps the reset guard held on an outstanding reader.
    waits: u64,
    /// Unit: count of hard-class entries submitted into the collector's queue.
    hard_reads: u64,
    /// Unit: count of those that also read bytes back and verified them.
    hard_reads_verified: u64,
    /// Unit: count of hard-class entries submitted into a queue that already
    /// had entries waiting — the ones that had to overtake something, and
    /// therefore the ones I3 is a claim about.
    hard_reads_overtaking: u64,
    /// Unit: count of batch-class client entries submitted — I3 protects none
    /// of them, and they are here so that it is asserted not to.
    batch_entries: u64,
    /// Unit: count of collector operations queued as backlog, so that an urgent
    /// entry has more than the in-flight one to overtake.
    backlog_entries: u64,
    /// I3's witness: the maximum collector operations observed ahead of an
    /// entry more urgent than batch.
    /// Unit: count of device operations.
    ops_ahead: u32,
    /// Unit: count of I1 evaluations at a `ZONE_RESET` instant — the mechanism
    /// and the formula, once each per reset.
    i1_checks: u64,
    /// Unit: count of I2 evaluations, one per step boundary.
    i2_checks: u64,
    /// Unit: count of I3 evaluations, one per step boundary.
    i3_checks: u64,
    /// Unit: count of blocks in the largest `reach(R)` the oracle computed.
    reach_max_blocks: u64,
    /// Unit: count of zones the multi-zone publish's own blocks spanned.
    open_publish_zones: u64,
    /// Unit: count of objects that publish wrote.
    open_publish_objects: u64,
    /// Unit: bytes the client submitted.
    app_bytes: u64,
    /// Unit: count of pinned generations read back and verified at the end.
    verified: u64,
}

impl Counts {
    /// Fold one seed's counts into a total.
    fn add(&mut self, other: &Self) {
        self.cycles += other.cycles;
        self.steps += other.steps;
        self.actions += other.actions;
        self.sweeps += other.sweeps;
        self.resets += other.resets;
        self.resets_while_open += other.resets_while_open;
        self.waits += other.waits;
        self.hard_reads += other.hard_reads;
        self.hard_reads_verified += other.hard_reads_verified;
        self.hard_reads_overtaking += other.hard_reads_overtaking;
        self.batch_entries += other.batch_entries;
        self.backlog_entries += other.backlog_entries;
        self.ops_ahead = self.ops_ahead.max(other.ops_ahead);
        self.i1_checks += other.i1_checks;
        self.i2_checks += other.i2_checks;
        self.i3_checks += other.i3_checks;
        self.reach_max_blocks = self.reach_max_blocks.max(other.reach_max_blocks);
        self.open_publish_zones += other.open_publish_zones;
        self.open_publish_objects += other.open_publish_objects;
        self.app_bytes += other.app_bytes;
        self.verified += other.verified;
    }

    /// The table a claim registrar reads.
    fn report(&self, what: &str) {
        println!("  {what}");
        println!("    collector cycles begun            {:>12}", self.cycles);
        println!("    collector steps                   {:>12}", self.steps);
        println!("    client actions between steps      {:>12}", self.actions);
        println!("    zones condemned (sweeps)          {:>12}", self.sweeps);
        println!("    ZONE_RESET submissions            {:>12}", self.resets);
        println!("    of those, with a publish open     {:>12}", self.resets_while_open);
        println!("    steps held by the third conjunct  {:>12}", self.waits);
        println!("    hard-class entries submitted      {:>12}", self.hard_reads);
        println!("    of those, bytes read and verified {:>12}", self.hard_reads_verified);
        println!("    of those, submitted behind work   {:>12}", self.hard_reads_overtaking);
        println!("    batch-class client entries        {:>12}", self.batch_entries);
        println!("    collector operations queued ahead  {:>11}", self.backlog_entries);
        println!("    max collector ops ahead of urgent {:>12}", self.ops_ahead);
        println!("    I1 evaluations (at a reset)       {:>12}", self.i1_checks);
        println!("    I2 evaluations (every step)       {:>12}", self.i2_checks);
        println!("    I3 evaluations (every step)       {:>12}", self.i3_checks);
        println!("    largest reach(R), in blocks       {:>12}", self.reach_max_blocks);
        println!("    multi-zone publish: zones spanned {:>12}", self.open_publish_zones);
        println!("    multi-zone publish: objects       {:>12}", self.open_publish_objects);
        println!("    application bytes submitted       {:>12}", self.app_bytes);
        println!("    pinned generations verified       {:>12}", self.verified);
    }
}

/// One pinned generation, as the reader and the retention policy need it.
#[derive(Clone, Copy, Debug)]
struct Generation {
    /// Unit: bytes, exactly 32 — the object's content address.
    root: [u8; 32],
    /// Unit: bytes — how long the object was.
    len: usize,
    /// Unit: none — which object of the run this was.
    index: u64,
}

/// The hard-class reader's outstanding belief.
///
/// A reader that resolved `h` to `z` and has not finished, which is the thing
/// the reset guard's third conjunct is about — and it is held across steps
/// rather than inside one, because in a synchronous host model a read that
/// begins and ends inside `Device::read` leaves the count at zero at every
/// instant the guard could look at it.
#[derive(Clone, Copy, Debug)]
struct Belief {
    /// Unit: zone index, zero-based — the zone the reader resolved into.
    zone: u32,
    /// The generation it resolved.
    generation: Generation,
    /// Unit: count of ticks — when the belief was taken.
    since: u64,
}

/// The multi-zone publish held open across a sweep.
struct OpenPublish {
    publisher: Publisher,
    /// The zones this publish's *own* appends landed in, at the moment it made
    /// them. Its own, and not `Roots::transient_blocks`, because
    /// `Publisher::appended` records every block the store allocated since it
    /// last looked — see this file's note on that — and a span measured from
    /// that would include another publish's zones.
    /// Unit: zone index, zero-based.
    zones: BTreeSet<u32>,
    /// Unit: count of objects written into this publish.
    objects: u64,
    /// What it wrote, for the read-back after it commits.
    /// Unit: bytes, and a content address.
    written: Vec<(usize, [u8; 32])>,
}

/// Everything one seed's run holds.
struct World {
    store: Store<ZoneMap<ZonedMemory>>,
    roots: Roots,
    collector: Collector,
    /// Currently pinned generations, in publish order.
    live: Vec<Generation>,
    /// Generations retention keeps, by index.
    /// Unit: none — object indices.
    kept: BTreeMap<u64, Generation>,
    counts: Counts,
}

/// Assert I2 and I3, which RFC 0059 states over *every* instant.
///
/// I1 is not here: it is a statement about a zone at the instant of its reset,
/// and a version of it quantified over every zone at every instant would be a
/// different and much weaker claim — every sealed zone has bits set in it, and
/// that is the design working.
fn at_this_instant(world: &mut World, when: &str) -> Result<(), String> {
    world.counts.i2_checks += 1;
    if !invariants::a_reset_zone_holds_no_live_blob(world.store.device()) {
        return Err(format!(
            "I2 ({when}): a zone in state free still resolves — `state(z) = free → \
             ∀h : location(h) = (z, ·) is undefined` is false"
        ));
    }
    world.counts.i3_checks += 1;
    if !invariants::collection_never_starves_an_urgent_read(world.collector.queue()) {
        return Err(format!(
            "I3 ({when}): {} collector operation(s) observed ahead of an entry above batch, \
             against a bound of {COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ}. RFC 0059's first \
             reversal condition, fired: an urgent entry reached the device behind collector \
             work the queue should have ranked below it",
            world.collector.queue().ops_ahead_of_urgent_read()
        ));
    }
    Ok(())
}

/// Assert I1 at the instant `ZONE_RESET(zone)` was submitted.
///
/// `held` is what the zone held an instant earlier, captured before the step
/// that reset it — because after the reset the mapping is gone and the question
/// *what was in there* has no answer. The root set does not change during a
/// collector step (`Collector::step` takes `&Roots`) and neither does the
/// store's hash-to-block map, so `reach(R)` computed here is `reach(R)` at that
/// instant.
fn i1_at_the_reset(world: &mut World, zone: u32, held: &[u64]) -> Result<(), String> {
    world.counts.i1_checks += 1;
    // The mechanism RFC 0059 says implies it: `marked_roots ⊇ R`, and every bit
    // of `B` in the zone's range clear.
    let mark: &Mark = world.collector.mark();
    if !invariants::nothing_reachable_is_swept(world.store.device(), mark, &world.roots, zone) {
        return Err(format!(
            "I1 (mechanism): zone {zone} was reset with `marked_roots ⊇ R` false or a bit of \
             B set in its range"
        ));
    }
    // And the formula itself, over a walk that is not the collector's mark.
    let reached = match reach(&mut world.store, &world.roots) {
        Ok(reached) => reached,
        Err((hash, why)) => {
            return Err(format!(
                "I1 (formula): walking reach(R) after zone {zone} was reset refused at \
                 {:02x}{:02x}… with {why:#x} — a record the root set reaches is no longer \
                 mapped, which is `blobs(z) ∩ reach(R) = ∅` false at the reset instant",
                hash[0], hash[1]
            ));
        }
    };
    world.counts.reach_max_blocks = world.counts.reach_max_blocks.max(reached.len() as u64);
    if reached.is_empty() {
        return Err(format!(
            "I1 (formula): reach(R) was empty at the reset of zone {zone}, so the \
             intersection is empty for a reason that is not the design's"
        ));
    }
    let intersect: Vec<u64> =
        held.iter().copied().filter(|block| reached.contains(block)).collect();
    if !intersect.is_empty() {
        return Err(format!(
            "I1 (formula): zone {zone} was reset holding {} block(s) in reach(R), the first \
             being logical block {} — `blobs(z) ∩ reach(R) = ∅` is false",
            intersect.len(),
            intersect[0]
        ));
    }
    Ok(())
}

/// Publish one object: open a write set, write it, record its appends, append a
/// root record, `FLUSH`, and adopt-then-drop.
///
/// The whole of RFC 0060's sequence, taken through `f_zone::publish`, because a
/// publish assembled differently here would be a second publish path and the
/// invariants would be asserted against the wrong one.
fn publish(
    world: &mut World,
    index: u64,
    generation: u64,
    lengths: &mut SeededEnv,
    bytes: &mut SeededEnv,
    scratch: &mut Vec<u8>,
) -> Result<Generation, String> {
    let len = length(lengths);
    content(bytes, len, scratch);
    let mut publisher = Publisher::open(&mut world.roots, &world.store);
    let root = world
        .store
        .put_object(scratch)
        .map_err(|why| format!("object {index} refused: {why:#x}"))?;
    publisher
        .appended(&mut world.roots, &world.store)
        .map_err(|why| format!("recording object {index}'s appends refused: {why:#x}"))?;
    let record = RootRecord {
        generation,
        root,
        frame: [0u8; 32],
        module: root,
        previous: [0u8; 32],
        check: [0u8; 32],
    };
    publisher
        .commit(&mut world.store, &mut world.roots, &record)
        .map_err(|why| format!("publishing generation {generation} refused: {why:#x}"))?;
    world.counts.app_bytes += len as u64;
    Ok(Generation { root, len, index })
}

/// Apply the retention policy: the generation before this one is dropped unless
/// it is one of the kept ones.
///
/// An unpin is a *removal*, which RFC 0059 says schedules the full re-mark and
/// is not applied until the cycle ends. Doing it during a cycle is therefore
/// part of what makes this workload adversarial rather than a side effect of
/// wanting garbage.
fn retire(world: &mut World, published: Generation) {
    if published.index.is_multiple_of(KEEP_EVERY) {
        world.kept.insert(published.index, published);
        world.live.push(published);
        return;
    }
    world.live.push(published);
    // Drop the newest generation that is not kept and is not this one, so that
    // the store always names what it was last asked to keep.
    let droppable = world
        .live
        .iter()
        .position(|held| held.index != published.index && !world.kept.contains_key(&held.index));
    if let Some(at) = droppable {
        let dropped = world.live.remove(at);
        world.roots.unpin(&dropped.root);
    }
}

/// One client action, drawn.
///
/// The draw is over what the client does and not over whether it does anything:
/// a schedule in which the adversary is idle is a schedule the collector is not
/// racing anything in, and this run has one of those already — it is called
/// `zone/tests/cycle.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// Publish a generation, mid-cycle.
    Publish,
    /// Write another object into the publish that is being held open.
    AppendOpen,
    /// Submit a hard-class entry, and take the belief that goes with it.
    HardRead,
    /// Finish the outstanding read: bytes back through the map, verified.
    FinishRead,
    /// Submit a batch-class entry, which I3 protects none of.
    BatchNoise,
    /// Put a backlog of the collector's own operations into the queue, so that
    /// the next urgent entry has something to overtake.
    Backlog,
    /// Let the collector have the tick.
    Idle,
}

/// Draw one.
fn draw(choices: &mut SeededEnv) -> Action {
    match choices.choose(9) {
        0 | 1 => Action::Publish,
        2 | 3 => Action::AppendOpen,
        4 => Action::HardRead,
        5 => Action::FinishRead,
        6 => Action::BatchNoise,
        7 => Action::Backlog,
        _ => Action::Idle,
    }
}

/// One seed's run, and everything it measured.
fn run_seed(seed: u64) -> Result<Counts, String> {
    let (zones, layout) = geometry(seed);
    let device = ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, zones);
    let map = ZoneMap::new(device, DATA_FROM)
        .map_err(|why| format!("the mapping refused the device: {why:#x}"))?;
    let device_blocks = ZONE_BLOCKS * u64::from(zones);
    let store = Store::format(map, &layout)
        .map_err(|why| format!("the store refused the device: {why:#x}"))?;
    let mut world = World {
        store,
        roots: Roots::new(),
        collector: Collector::new(device_blocks),
        live: Vec::new(),
        kept: BTreeMap::new(),
        counts: Counts::default(),
    };

    let mut lengths = site(seed, "e2-p03/length");
    let mut bytes = site(seed, "e2-p03/content");
    let mut open_lengths = site(seed, "e2-p03/open-length");
    let mut open_bytes = site(seed, "e2-p03/open-content");
    let mut choices = site(seed, "e2-p03/schedule");
    let mut scratch = Vec::new();

    // ---- the fill: garbage for the collector to have an opinion about ----
    for index in 0..FILL_OBJECTS {
        let published =
            publish(&mut world, index, index + 1, &mut lengths, &mut bytes, &mut scratch)?;
        retire(&mut world, published);
        at_this_instant(&mut world, "during the fill")?;
    }
    let sealed = world.store.device().zones().filter(|zone| zone.state == State::Sealed).count();
    if sealed < 3 {
        return Err(format!(
            "the fill sealed {sealed} zone(s); a sweep with fewer than three candidates ranks \
             almost nothing, so this run would be green having asserted little"
        ));
    }

    // ---- the concurrent phase --------------------------------------------
    let mut published = FILL_OBJECTS;
    // Opened before the first cycle begins, so that it is in flight across every
    // sweep that cycle takes rather than across the tail of one. Its transient
    // entry names only what the store allocates from here on, so the fill's
    // sealed zones — which is where the garbage is — stay rankable and the sweep
    // has work to do while the publish is open. That is the case RFC 0059's
    // open-publish clause exists for, held for as long as it takes.
    let mut open = Some(OpenPublish {
        publisher: Publisher::open(&mut world.roots, &world.store),
        zones: BTreeSet::new(),
        objects: 0,
        written: Vec::new(),
    });
    let mut open_done = false;
    let mut belief: Option<Belief> = None;
    // The long-lived reader's beliefs, taken on sweep candidates before the
    // sweep reaches any of them, and released one at a time as the guard holds
    // on each. Its whole job is to make the third conjunct's exercise a fact of
    // the run rather than a fact about the seed.
    let mut watched: BTreeSet<u32> = BTreeSet::new();
    let mut watch_taken = false;
    let mut ticks = 0u64;
    let mut idle_cycles = 0u64;

    world.collector.begin(&mut world.roots);
    world.counts.cycles += 1;

    loop {
        ticks += 1;
        if ticks > MAX_TICKS {
            return Err(format!(
                "the run reached {MAX_TICKS} ticks with {} object(s) unpublished and the \
                 multi-zone publish {}: it did not finish its work, which is a failure and \
                 not a pass",
                FILL_OBJECTS + CONCURRENT_OBJECTS - published,
                if open_done { "committed" } else { "still open" }
            ));
        }

        // ---- one collector step, with the reset instant captured ---------
        let condemned = world
            .store
            .device()
            .zones()
            .find(|zone| zone.state == State::Condemned)
            .map(|zone| (zone.zone, zone.reads_outstanding));
        let held: Vec<u64> =
            condemned.map(|(zone, _)| world.store.device().holds(zone)).unwrap_or_default();

        let progress =
            world.collector.step(&mut world.store, &world.roots).map_err(|why| match why {
                refusal::MALFORMED => format!(
                    "the collector refused its own cycle at tick {ticks}: it would have broken \
                     one of the three, and stopped instead. That is the refusal working, and it \
                     is still a failure of this run — the invariants are supposed to hold, not \
                     to be declined"
                ),
                other => format!("the collector step refused at tick {ticks}: {other:#x}"),
            })?;
        world.counts.steps += 1;

        match progress {
            Progress::Reset => {
                let (zone, readers) = condemned.ok_or_else(|| {
                    format!("a reset at tick {ticks} with no condemned zone before the step")
                })?;
                // The reset guard's third conjunct, asserted against what the
                // step actually did rather than against the collector's own
                // evaluation of it.
                if readers != 0 {
                    return Err(format!(
                        "the reset guard: zone {zone} was reset with {readers} reader(s) \
                         outstanding an instant earlier — `reads_outstanding(z) = 0` is not \
                         optional, and I2 is about beliefs as well as bytes"
                    ));
                }
                i1_at_the_reset(&mut world, zone, &held)?;
                world.counts.resets += 1;
                if open.is_some() {
                    world.counts.resets_while_open += 1;
                }
            }
            Progress::Selected => {
                if condemned.is_none()
                    && world.store.device().zones().any(|zone| zone.state == State::Condemned)
                {
                    world.counts.sweeps += 1;
                }
            }
            Progress::Waiting => {
                world.counts.waits += 1;
                // The guard held on a zone the reader believes in, which is the
                // conjunct doing its job. The belief is released now — RFC
                // 0059's argument that the guard terminates is that
                // `reads_outstanding` is monotone non-increasing after
                // condemnation, and a reader that never let go would be a
                // livelock rather than a test.
                if let Some((zone, _)) = condemned
                    && watched.remove(&zone)
                {
                    world
                        .store
                        .device_mut()
                        .release_read(zone)
                        .map_err(|why| format!("releasing the watch on {zone}: {why:#x}"))?;
                }
            }
            Progress::Published => {
                // `live_bytes(z)` has just been published, so the sweep
                // candidates are now visible — and none of them has been
                // condemned yet, which is the instant a belief has to be taken
                // at if it is to be a belief formed before condemnation.
                if !watch_taken {
                    watch_taken = true;
                    let zone_bytes = ZONE_BLOCKS * BLOCK_BYTES as u64;
                    let candidates: Vec<u32> = world
                        .store
                        .device()
                        .zones()
                        .filter(|zone| zone.state == State::Sealed)
                        .filter(|zone| zone.live_bytes > 0 && zone.live_bytes * 4 < zone_bytes)
                        .map(|zone| zone.zone)
                        .take(WATCHED_ZONES)
                        .collect();
                    for zone in candidates {
                        world.store.device_mut().hold_read(zone).map_err(|why| {
                            format!("taking the watch on zone {zone} refused: {why:#x}")
                        })?;
                        watched.insert(zone);
                    }
                }
            }
            Progress::Done => {
                idle_cycles += 1;
            }
            Progress::Marked | Progress::Copied | Progress::Committed => {}
        }
        at_this_instant(&mut world, "after a collector step")?;

        // ---- one client action, between two collector steps --------------
        let forced = ticks.is_multiple_of(FORCE_EVERY);
        let mut action = draw(&mut choices);
        if forced {
            // Bounded progress, so that a schedule which never draws the
            // publish action ends rather than running to the ceiling.
            action = if published < FILL_OBJECTS + CONCURRENT_OBJECTS {
                Action::Publish
            } else if open.as_ref().is_some_and(|held| held.zones.len() < OPEN_PUBLISH_ZONES) {
                Action::AppendOpen
            } else {
                action
            };
        }
        // A belief is bounded: RFC 0059's argument that the guard terminates is
        // that `reads_outstanding` is monotone non-increasing after
        // condemnation, and this is the reader holding up its end.
        if belief.is_some_and(|held| ticks.saturating_sub(held.since) >= READER_HOLD_TICKS) {
            action = Action::FinishRead;
        }

        world.counts.actions += 1;
        match action {
            Action::Publish if published < FILL_OBJECTS + CONCURRENT_OBJECTS => {
                let generation = published + 1;
                let made = publish(
                    &mut world,
                    published,
                    generation,
                    &mut lengths,
                    &mut bytes,
                    &mut scratch,
                )?;
                retire(&mut world, made);
                published += 1;
            }
            Action::AppendOpen
                if open.as_ref().is_some_and(|held| held.objects < OPEN_PUBLISH_MAX_OBJECTS) =>
            {
                let held = open.as_mut().ok_or("the open publish vanished")?;
                let len = length(&mut open_lengths);
                content(&mut open_bytes, len, &mut scratch);
                let from = world.store.free();
                let root = world
                    .store
                    .put_object(&scratch)
                    .map_err(|why| format!("the open publish refused an object: {why:#x}"))?;
                let to = world.store.free();
                for block in from..to {
                    if let Some(zone) = world.store.device().zone_of(block) {
                        held.zones.insert(zone);
                    }
                }
                held.publisher
                    .appended(&mut world.roots, &world.store)
                    .map_err(|why| format!("recording the open publish refused: {why:#x}"))?;
                held.objects += 1;
                held.written.push((len, root));
                world.counts.app_bytes += len as u64;
            }
            Action::HardRead if belief.is_none() && !world.live.is_empty() => {
                let at = choices.choose(world.live.len() as u32) as usize;
                let generation = world.live[at];
                // The reader resolves the root to a zone — `location(h)`, whose
                // second half is this map's — and tells the map it believes it.
                let Some(logical) = world.store.address(&generation.root) else { continue };
                let Some(zone) = world.store.device().zone_of(logical) else { continue };
                world
                    .store
                    .device_mut()
                    .hold_read(zone)
                    .map_err(|why| format!("taking a read on zone {zone} refused: {why:#x}"))?;
                // And the entry, at the class I3 is about, into the same queue
                // the collector's own operations go through.
                let deadline = 1_000 + choices.choose(1_000) as u64;
                let waiting = world.collector.queue().depth();
                world.collector.queue_mut().submit_client(class::HARD, deadline);
                world.counts.hard_reads += 1;
                if waiting > 0 {
                    world.counts.hard_reads_overtaking += 1;
                }
                belief = Some(Belief { zone, generation, since: ticks });
            }
            Action::FinishRead => {
                if let Some(held) = belief.take() {
                    let mut into = vec![0u8; held.generation.len];
                    let read = world.store.get_object(&held.generation.root, &mut into).map_err(
                        |why| {
                            format!(
                                "the hard-class reader's object {} refused after {} tick(s) of \
                                 collection: {why:#x} — the collector took data a pinned root \
                                 was naming",
                                held.generation.index,
                                ticks - held.since
                            )
                        },
                    )?;
                    if read != held.generation.len {
                        return Err(format!(
                            "the hard-class reader read {read} bytes of object {}, which is {}",
                            held.generation.index, held.generation.len
                        ));
                    }
                    world
                        .store
                        .device_mut()
                        .release_read(held.zone)
                        .map_err(|why| format!("releasing the read refused: {why:#x}"))?;
                    world.counts.hard_reads_verified += 1;
                }
            }
            Action::BatchNoise => {
                world.collector.queue_mut().submit_client(class::BATCH, NO_DEADLINE);
                world.counts.batch_entries += 1;
            }
            Action::Backlog => {
                // RFC 0059's derivation says the collector work an urgent entry
                // can be stuck behind is *the work already inside the device*,
                // because "every other collector operation is still in the
                // driver's queue, where it sorts below any entry above batch."
                // A queue that never holds more than one collector operation
                // cannot tell that argument from a coincidence: with a backlog
                // of one, I3 is bounded by 1 whatever the queue's ordering does,
                // and the assertion is unfalsifiable. So the adversary puts a
                // backlog there, and the next urgent entry has to overtake it.
                //
                // The urgent entry goes in behind it *in the same action*,
                // because the next collector step drains the whole backlog —
                // `Queue::collector_operation` serves everything of lower rank
                // before its own entry — so a read submitted a tick later would
                // arrive at an empty queue and overtake nothing. Which is
                // exactly the shape I3 is about: *an entry submitted while
                // collector work is queued*, and it has to be built rather than
                // waited for.
                for _ in 0..COLLECTOR_BACKLOG {
                    world.collector.queue_mut().submit_collector();
                    world.counts.backlog_entries += 1;
                }
                let deadline = 1_000 + choices.choose(1_000) as u64;
                let waiting = world.collector.queue().depth();
                world.collector.queue_mut().submit_client(class::HARD, deadline);
                world.counts.hard_reads += 1;
                if waiting > 0 {
                    world.counts.hard_reads_overtaking += 1;
                }
            }
            Action::Publish | Action::AppendOpen | Action::HardRead | Action::Idle => {}
        }
        at_this_instant(&mut world, "after a client action")?;

        // ---- the multi-zone publish commits once it has been the case ----
        let ready = open.as_ref().is_some_and(|held| {
            held.zones.len() >= OPEN_PUBLISH_ZONES
                && world.counts.resets_while_open >= OPEN_PUBLISH_RESETS
        });
        if ready && let Some(held) = open.take() {
            world.counts.open_publish_zones = held.zones.len() as u64;
            world.counts.open_publish_objects = held.objects;
            let record = RootRecord {
                generation: published + 1,
                root: held.written.last().map_or([0u8; 32], |(_, root)| *root),
                frame: [0u8; 32],
                module: [0u8; 32],
                previous: [0u8; 32],
                check: [0u8; 32],
            };
            held.publisher
                .commit(&mut world.store, &mut world.roots, &record)
                .map_err(|why| format!("committing the multi-zone publish refused: {why:#x}"))?;
            // And every blob it wrote is still there. Without the transient
            // entry this is where the run goes red, with `ADDRESS` from a zone
            // the collector reset while the publish was open.
            let mut into = vec![0u8; OBJECT_MAX_BYTES];
            for (len, root) in &held.written {
                let read = world.store.get_object(root, &mut into).map_err(|why| {
                    format!(
                        "a blob the multi-zone publish wrote refused after {} sweep(s) taken \
                         while it was open: {why:#x} — the collector took data a transient \
                         root was protecting, which is RFC 0059's last reversal condition",
                        world.counts.resets_while_open
                    )
                })?;
                if read != *len {
                    return Err(format!("the multi-zone publish read back {read} of {len}"));
                }
            }
            open_done = true;
            at_this_instant(&mut world, "after the multi-zone publish committed")?;
        }

        // ---- when there is nothing left to do ----------------------------
        let client_done = published == FILL_OBJECTS + CONCURRENT_OBJECTS && open_done;
        if !world.collector.running() {
            if client_done
                && world.counts.cycles >= MIN_CYCLES
                && world.counts.resets >= MIN_RESETS
                && belief.is_none()
            {
                // Whatever the sweep never reached, the reader lets go of: a
                // belief left outstanding is a zone the next run of this store
                // could not reset, and this run has finished asking questions.
                for zone in core::mem::take(&mut watched) {
                    world
                        .store
                        .device_mut()
                        .release_read(zone)
                        .map_err(|why| format!("releasing the watch on {zone}: {why:#x}"))?;
                }
                break;
            }
            if idle_cycles > 8 && world.counts.resets < MIN_RESETS && client_done {
                return Err(format!(
                    "the collector found nothing to sweep in {idle_cycles} consecutive \
                     cycles and reset {} zone(s), against a floor of {MIN_RESETS}: this run \
                     collected almost nothing and asserted I1 almost never",
                    world.counts.resets
                ));
            }
            world.collector.begin(&mut world.roots);
            world.counts.cycles += 1;
        }
    }

    // The driver drains what is left. In this model the queue only moves when
    // the collector takes an operation — `Queue::collector_operation` is what
    // calls `hand_over` — so a client entry submitted after the last collector
    // step would otherwise sit there forever, which is an artefact of the model
    // and not starvation: a real driver's queue is drained by its own poll. The
    // drain is here rather than the check being relaxed, because *every entry
    // completes* is the thing that makes I3's witness cover every entry, and a
    // run that quietly left one out would be reporting a maximum over a subset.
    let queue = world.collector.queue_mut();
    loop {
        queue.settle().map_err(|why| format!("settling refused: {why:#x}"))?;
        if queue.depth() == 0 {
            break;
        }
        queue.hand_over().map_err(|why| format!("draining the queue refused: {why:#x}"))?;
    }
    world.counts.ops_ahead = world.collector.queue().ops_ahead_of_urgent_read();
    at_this_instant(&mut world, "after the queue drained")?;

    // ---- what the run is worth ------------------------------------------
    if world.counts.open_publish_zones < OPEN_PUBLISH_ZONES as u64 {
        return Err(format!(
            "the publish held open spanned {} zone(s) of its own, so this run tested the shape \
             a single open zone already protects rather than the one the transient root exists \
             for",
            world.counts.open_publish_zones
        ));
    }
    let published_tree = world.collector.published(world.store.device(), &world.roots);
    if published_tree.unresolved != 0 {
        return Err(format!(
            "{} hash(es) in the closure of R resolved to nothing — a root naming a blob that \
             is not there, which is `E2-P01`'s forbidden third state",
            published_tree.unresolved
        ));
    }
    if world.collector.queue().depth() != 0 {
        return Err(format!(
            "{} entr(ies) were left waiting in the queue, so some client entry was never \
             served and I3 was measured against a queue that had stopped draining",
            world.collector.queue().depth()
        ));
    }

    // Every generation retention kept still reads back, chunk by chunk, each
    // verified against its own name. This is the semantic content of I1: what
    // was reachable is still there, and copy-forward moved bytes without moving
    // a name.
    let mut into = vec![0u8; OBJECT_MAX_BYTES];
    let kept: Vec<Generation> = world.kept.values().copied().collect();
    for generation in kept {
        let read = world.store.get_object(&generation.root, &mut into).map_err(|why| {
            format!("kept generation {} refused after collection: {why:#x}", generation.index)
        })?;
        if read != generation.len {
            return Err(format!(
                "kept generation {} read back {read} of {}",
                generation.index, generation.len
            ));
        }
        world.counts.verified += 1;
    }

    controls(&mut world)?;
    Ok(world.counts)
}

/// Ask each predicate a question it must answer *no* to.
///
/// A predicate that has never returned false is indistinguishable from one that
/// cannot, and every assertion above this is worthless if these three do not
/// discriminate.
fn controls(world: &mut World) -> Result<(), String> {
    // I1 over a sealed zone that still holds reachable blocks.
    let live = world
        .store
        .device()
        .zones()
        .find(|zone| zone.state == State::Sealed && zone.live_bytes > 0)
        .map(|zone| zone.zone);
    let Some(live) = live else {
        return Err("no sealed zone holds reachable bytes, so I1 has nothing to refuse and the \
             control cannot run"
            .to_string());
    };
    let mark = world.collector.mark();
    if invariants::nothing_reachable_is_swept(world.store.device(), mark, &world.roots, live) {
        return Err(format!(
            "I1 permitted a reset of zone {live}, which holds reachable blocks — the predicate \
             is not discriminating and every green step above it is worthless"
        ));
    }

    // And the positive half, so the control is not just a predicate that says no.
    let free = world
        .store
        .device()
        .zones()
        .find(|zone| zone.state == State::Free)
        .map(|zone| zone.zone)
        .ok_or("no free zone, so the positive half of the I1 control cannot run")?;
    let mark = world.collector.mark();
    if !invariants::nothing_reachable_is_swept(world.store.device(), mark, &world.roots, free) {
        return Err(format!("I1 refused a reset of zone {free}, which holds nothing"));
    }

    // The reset guard's third conjunct, over a zone a reader believes in.
    if !invariants::no_reader_believes_the_zone(world.store.device(), free) {
        return Err(format!("zone {free} began the control with a reader outstanding"));
    }
    world
        .store
        .device_mut()
        .hold_read(free)
        .map_err(|why| format!("taking a read on zone {free} refused: {why:#x}"))?;
    if invariants::no_reader_believes_the_zone(world.store.device(), free) {
        return Err(format!(
            "the reset guard's third conjunct permitted zone {free} with a reader outstanding"
        ));
    }
    world
        .store
        .device_mut()
        .release_read(free)
        .map_err(|why| format!("releasing the read on zone {free} refused: {why:#x}"))?;
    Ok(())
}

/// Where the driver keeps the number I3's bound is derived from.
const DRIVER_PENDING: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../user/virtio-blk/src/pending.rs");

/// RFC 0059's bound is `f_virtio_blk::pending::IN_FLIGHT`, and this is what
/// makes that a fact rather than a transcription.
///
/// The same check `zone/tests/cycle.rs` runs, and it is here as well rather than
/// only there because this file is the one that *asserts* I3: a bound checked in
/// a target somebody can run separately is a bound this target could be green
/// without. `zone/Cargo.toml` argues at length why it is a file read and not a
/// dependency, and states the limit — a constant moved to another file, or
/// spelled with a different type, goes past it.
fn the_bound_is_the_drivers_chain_length() -> Result<(), String> {
    let needle = "pub const IN_FLIGHT: u32 = ";
    let text = std::fs::read_to_string(DRIVER_PENDING)
        .map_err(|why| format!("reading {DRIVER_PENDING}: {why}"))?;
    let at = text.find(needle).ok_or_else(|| {
        format!(
            "`{needle}` is not in {DRIVER_PENDING} — the constant I3's bound is derived from \
             has moved, and this check has stopped checking anything"
        )
    })?;
    let rest = &text[at + needle.len()..];
    let end = rest.find(';').ok_or("the driver's IN_FLIGHT has no terminating semicolon")?;
    let found = rest[..end]
        .trim()
        .parse::<u32>()
        .map_err(|why| format!("the driver's IN_FLIGHT is not a number: {why}"))?;
    if found != COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ {
        return Err(format!(
            "the driver keeps {found} operation(s) inside the device and I3's bound is \
             {COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ}. RFC 0059: the row moves when `E1-B09` \
             raises `IN_FLIGHT`, re-derived in the same diff"
        ));
    }
    Ok(())
}

/// One invocation, and the status it earns.
fn run(asked: &Asked) -> i32 {
    println!(
        "invariants — RFC 0059's I1, I2 and I3 over {} seed(s) from {:#018x}",
        asked.seeds, asked.seed
    );
    println!("  (host model; collection, allocation and a hard-class reader interleaved)");
    println!();

    // Before anything is written: if I3's bound is no longer the driver's
    // number, every invariant this run goes on to assert is asserting the wrong
    // thing.
    if let Err(why) = the_bound_is_the_drivers_chain_length() {
        println!("invariants: FAILED — {why}");
        return 1;
    }

    let mut total = Counts::default();
    for step in 0..asked.seeds {
        let seed = asked.seed.wrapping_add(step);
        match run_seed(seed) {
            Ok(counts) => {
                println!(
                    "  seed {seed:#018x}: {} reset(s), {} sweep(s), {} hard-class read(s), \
                     max {} op(s) ahead",
                    counts.resets, counts.sweeps, counts.hard_reads, counts.ops_ahead
                );
                total.add(&counts);
            }
            Err(why) => {
                println!();
                println!("invariants: FAILED — seed {seed:#018x}: {why}");
                println!(
                    "  reproduce with: cargo test -p f-zone --test invariants -- \
                     --seed {seed} --seeds 1"
                );
                return 1;
            }
        }
    }

    println!();
    total.report("totals across every seed");
    println!();

    // The vacuity checks. Each of these is a way for a green run to have
    // asserted nothing, and each is a failure rather than a note.
    if total.resets == 0 {
        println!("invariants: FAILED — no zone was reset, so I1 was never evaluated");
        return 1;
    }
    if total.resets_while_open == 0 {
        println!(
            "invariants: FAILED — no reset was taken while a multi-zone publish was open, so \
             the case the first invariant's open-publish clause exists for did not arise"
        );
        return 1;
    }
    if total.open_publish_zones < OPEN_PUBLISH_ZONES as u64 {
        println!(
            "invariants: FAILED — the publish held open spanned {} zone(s), so it was not the \
             multi-zone case",
            total.open_publish_zones
        );
        return 1;
    }
    if total.hard_reads_verified == 0 {
        println!("invariants: FAILED — no hard-class read read any bytes back");
        return 1;
    }
    if total.hard_reads_overtaking == 0 {
        println!(
            "invariants: FAILED — every hard-class entry was submitted into an empty queue, so \
             none of them had anything to overtake and I3 was a bound nothing approached"
        );
        return 1;
    }
    if total.backlog_entries == 0 {
        println!(
            "invariants: FAILED — the collector never had a backlog, so I3 could not have \
             exceeded the device's depth whatever the queue's ordering did"
        );
        return 1;
    }
    if total.waits == 0 {
        println!(
            "invariants: FAILED — the reset guard's third conjunct never held a reset, so \
             `reads_outstanding(z) = 0` was a conjunct that was always true and tested nothing"
        );
        return 1;
    }
    if total.ops_ahead != COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ {
        println!(
            "invariants: FAILED — the largest number of collector operations ever observed \
             ahead of an urgent entry was {}, and not {COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ}: \
             at zero, I3 held because nothing was ever ahead of anything, which tests nothing; \
             above it, I3 is false",
            total.ops_ahead
        );
        return 1;
    }

    println!(
        "  I1 asserted at {} reset instant(s), as the mechanism and as the formula over an \
         independent walk",
        total.i1_checks
    );
    println!("  I2 asserted at {} step boundaries", total.i2_checks);
    println!("  I3 asserted at {} step boundaries, witness {}", total.i3_checks, total.ops_ahead);
    println!("invariants: ok");
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = match parse(&args) {
        Ok(asked) => asked,
        Err(why) => {
            eprintln!("invariants: {why}");
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
