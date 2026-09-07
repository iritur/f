// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-P01`: cut the power at every write boundary in a publish, and require the
//! mount to find either the old root or the new one and never a third thing.
//!
//! # The model, stated before it is used
//!
//! **A publish is a sequence of device operations. At any cut the model lands
//! every operation covered by a completed `FLUSH` and any subset of the rest, in
//! any order, with the cut falling inside an operation at a chosen
//! granularity.** Granularity is `block` or `byte`; mode is `honest`, which
//! respects `FLUSH`, or `lying`, which ignores it entirely. The re-run key is
//! `(seed, target, cut, granularity, mode)` and every draw in the model is
//! derived from it, so a failing cut reproduces byte for byte from the line the
//! failure prints.
//!
//! *An earlier draft of the model landed a prefix of the operations and nothing
//! else.* The spec rejected it and says why: a prefix model cannot produce a
//! device that lost an early write and kept a later one, which is exactly what a
//! write cache does on the way down, and a sweep that cannot produce that state
//! is a sweep that cannot find the bug that lives in it. So the subset and the
//! order are drawn, and the only thing the model refuses to reorder is what a
//! completed barrier covers — which is the whole content of what a barrier
//! promises.
//!
//! # Why the exit needs resolution and not only the `check` field
//!
//! *Every cut leaves either the old root or the new one, never a third thing.* A
//! root record whose `check` verifies and whose generation tree is not on the
//! device is a third thing: it names a state the machine cannot produce, and the
//! first read after it answers *not found*, which is indistinguishable from
//! *collected*. So the assertion is against [`f_zone::mount::mount`], which
//! answers the highest generation that verifies **and** resolves, and the
//! outcome is one of exactly two hashes.
//!
//! # The control, and why the headline property cannot be the one that finds it
//!
//! Build with `--features mutate-root-before-blobs` and the publish appends its
//! root record without waiting for the blobs it names. **The headline property
//! still holds**: the root that landed over missing blobs is refused by
//! resolution and the mount falls back, which is a rollback and not a third
//! state. That is not a weakness in the exit, it is what verify-before-accept is
//! for — and it means the property that catches the defect has to be the one the
//! barrier is actually about:
//!
//! > **in `honest` mode, no root is ever refused for non-resolution.**
//!
//! With the barrier issued, every operation before the root append is covered by
//! a completed `FLUSH` and therefore lands at every cut that includes the append.
//! Without it, they are a drawn subset. `cargo xtask cut` requires this run to go
//! red with the defect armed, green without it, and to print the cut point.
//!
//! # The four observations this run requires of itself
//!
//! A sweep that has never failed is indistinguishable from one that cannot, and
//! a sweep that never reached the interesting states is a sweep of the model. So
//! the run fails unless it saw all four:
//!
//! 1. a torn root record refused by its `check` field, at byte granularity;
//! 2. a `lying`-mode root refused by resolution, counted in
//!    `roots_refused_unresolved`;
//! 3. a cut inside a publish that crosses a root-zone wrap;
//! 4. exactly two distinct mount outcomes across the whole sweep.
//!
//! # Why a target of its own rather than a `#[test]`
//!
//! `zone/tests/cycle.rs`'s reason: the gate and the exit differ by *numbers* —
//! how many seeds, and whether every publish is swept or the four that matter —
//! and a libtest harness would swallow the arguments that carry them, leaving
//! the two to differ by a build instead.
//!
//! ```text
//! cargo test -p f-zone --test cut                                 # the gate
//! cargo test --release -p f-zone --test cut -- --seeds 32 --all   # the sweep
//! cargo xtask cut                                                 # both, and the control
//! ```
//!
//! # Where the model is, and where the plan said it would be
//!
//! The plan puts it in `env/src/sim/cut.rs`, wrapping `f_blob::device::Memory`.
//! It cannot go there: `f-blob` depends on `f-env` — `blob/src/gear.rs` derives
//! the gear table from `f_env::split` at compile time — so a model in `env/`
//! that named a blob device would be a dependency cycle. It cannot wrap
//! `Memory` either: a publish appends its root record to a *zone*, and the
//! operation a cut is most interesting inside is `ZONE_APPEND`, which that type
//! does not have. So the model wraps [`ZonedMemory`], and it lives beside the
//! sweep that is its only caller, which is where `blob/tests/million.rs` and
//! `zone/tests/cycle.rs` already put a harness. `zone/src/device.rs` gains one
//! function for it — `land`, what the media holds after a cut — because that is
//! a state and not an operation, and no interface above the device can express
//! it.

use std::process::exit;

use f_abi::store::{ObjectHead, RootRecord, Superblock, kind};
use f_blob::device::Device;
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{derive, label};
use f_env::{Env, SeededEnv};
use f_zone::device::{Kind, Report, Zoned, ZonedMemory};
use f_zone::map::ZoneMap;
use f_zone::mount::{Mounted, mount};
use f_zone::publish::{Publisher, ROOT_CARRY};
use f_zone::roots::Roots;

/// The name this target answers to when cargo's harness protocol asks for one.
const TEST_NAME: &str = "cut";

/// The modelled device's logical block. Unit: bytes.
const BLOCK_BYTES: usize = 512;

/// Blocks in one zone. Unit: count of blocks.
///
/// Forty, and the number is chosen against [`ROOT_CARRY`] rather than against a
/// device: a root zone must hold the carry *and* have room for more than the
/// carry afterwards, or every publish would wrap and the run would be measuring
/// a livelock. Forty gives a first wrap at generation 26 and another every
/// sixteen after it, which is close enough to the front that a sweep reaches one
/// without publishing all day.
const ZONE_BLOCKS: u64 = 40;

/// Zones the superblock and the two root zones occupy, and therefore the first
/// zone the mapping fills. Unit: count of zones.
const DATA_FROM: u32 = 3;

/// Free data zones above what the fill needs. Unit: count of zones.
const SLACK_ZONES: u32 = 4;

/// Component leaves under each generation node. Unit: count of objects.
///
/// Two, because the tree has to have a shape for a hole in it to be findable:
/// one leaf makes *the generation resolves* and *the leaf resolves* the same
/// sentence, and a cut that lost half a tree would be indistinguishable from one
/// that lost all of it.
const LEAVES: u64 = 2;

/// The shortest leaf this run draws. Unit: bytes.
///
/// Below `CHUNK_MIN_BYTES`, so a leaf is one chunk and the object record names
/// exactly one hash. That is deliberate: what this file sweeps is the *publish*,
/// and an object that chunked into a variable number of records would make the
/// number of write boundaries in a publish a function of the draw rather than of
/// the design. `blob/tests/chunker.rs` is where the chunker's own behaviour is
/// the subject.
const LEAF_MIN_BYTES: usize = 1024;

/// The longest. Unit: bytes.
const LEAF_MAX_BYTES: usize = 2048;

/// Generations published when nothing says otherwise. Unit: count of publishes.
///
/// Enough to cross a root-zone wrap and to keep going afterwards, which is the
/// third required observation. A run that published fewer would be green having
/// never reached the case the wrap exists for.
const DEFAULT_PUBLISHES: u64 = 28;

/// Seeds swept when nothing says otherwise. Unit: count of seeds.
const DEFAULT_SEEDS: u64 = 4;

/// The seed the sweep starts from. Unit: none — a seed.
const DEFAULT_SEED: u64 = 1;

/// At what granularity a cut falls inside an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Granularity {
    /// An operation landed whole or not at all. A root record fits in one block,
    /// so nothing tears here — which is why a torn-record requirement stated
    /// only at this granularity would be unsatisfiable.
    Block,
    /// The cut falls inside the last operation submitted, and a prefix of its
    /// bytes is on the media.
    Byte,
}

impl Granularity {
    /// The word the reproduction line uses.
    const fn name(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Byte => "byte",
        }
    }
}

/// What the device does with a `FLUSH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Every operation before a completed barrier is on the media. What RFC
    /// 0060 buys and what a device may be lying about.
    Honest,
    /// The barrier changed nothing. The format's correctness may not rest on
    /// this being false.
    Lying,
}

impl Mode {
    /// The word the reproduction line uses.
    const fn name(self) -> &'static str {
        match self {
            Self::Honest => "honest",
            Self::Lying => "lying",
        }
    }
}

/// One device operation, as the model replays it.
///
/// The bytes are held rather than re-derived because a replay must put back what
/// was submitted and not what a second run of the store would submit — those are
/// the same today and the difference is exactly what a sweep would stop being
/// able to see if the store ever became order-dependent.
#[derive(Clone, Debug)]
enum Op {
    /// A positioned write. The superblock's zone is the only one that takes one.
    Write {
        /// Unit: block index, zero-based, device-absolute.
        block: u64,
        bytes: Vec<u8>,
    },
    /// `ZONE_APPEND`, with the position the device assigned when it was
    /// submitted.
    Append {
        /// Unit: block index, zero-based, device-absolute.
        at: u64,
        bytes: Vec<u8>,
    },
    /// `ZONE_FINISH`.
    Finish {
        /// Unit: zone index, zero-based.
        zone: u32,
    },
    /// `ZONE_RESET` — the root-zone wrap's, and the only operation in a publish
    /// that destroys bytes.
    Reset {
        /// Unit: zone index, zero-based.
        zone: u32,
    },
    /// `FLUSH`. It moves no byte; what it does is decide which of the operations
    /// before it the model is not allowed to drop.
    Flush,
}

impl Op {
    /// A word for the reproduction line and the report.
    fn name(&self) -> &'static str {
        match self {
            Self::Write { .. } => "WRITE",
            Self::Append { .. } => "ZONE_APPEND",
            Self::Finish { .. } => "ZONE_FINISH",
            Self::Reset { .. } => "ZONE_RESET",
            Self::Flush => "FLUSH",
        }
    }
}

/// A device that does what [`ZonedMemory`] does and writes down what it was
/// asked.
///
/// It is a wrapper and not a second model, for RFC 0034's reason applied one
/// level down: a device model is a peer on the real types, so the thing a
/// publish runs against during the recording pass has to be the same device it
/// runs against during a replay, or the trace is of a device nothing else uses.
struct Recorder {
    inner: ZonedMemory,
    ops: Vec<Op>,
}

impl Recorder {
    fn new(inner: ZonedMemory) -> Self {
        Self { inner, ops: Vec::new() }
    }
}

impl Device for Recorder {
    fn block_bytes(&self) -> usize {
        self.inner.block_bytes()
    }

    fn blocks(&self) -> u64 {
        self.inner.blocks()
    }

    fn read(&mut self, block: u64, into: &mut [u8]) -> Result<(), i32> {
        self.inner.read(block, into)
    }

    fn write(&mut self, block: u64, from: &[u8]) -> Result<(), i32> {
        self.inner.write(block, from)?;
        self.ops.push(Op::Write { block, bytes: from.to_vec() });
        Ok(())
    }

    fn flush(&mut self) -> Result<(), i32> {
        self.inner.flush()?;
        self.ops.push(Op::Flush);
        Ok(())
    }
}

impl Zoned for Recorder {
    fn zone_count(&self) -> u32 {
        self.inner.zone_count()
    }

    fn zone_blocks(&self) -> u64 {
        self.inner.zone_blocks()
    }

    fn kind(&self, zone: u32) -> Kind {
        self.inner.kind(zone)
    }

    fn report(&mut self, zone: u32) -> Result<Report, i32> {
        self.inner.report(zone)
    }

    fn append(&mut self, zone: u32, from: &[u8]) -> Result<u64, i32> {
        let at = self.inner.append(zone, from)?;
        // The position the device assigned, and not the zone: a replay that put
        // an append back by appending would give it whatever position the replay
        // had reached, and the model would be testing its own arithmetic rather
        // than the format's ordering.
        self.ops.push(Op::Append { at, bytes: from.to_vec() });
        Ok(at)
    }

    fn finish(&mut self, zone: u32) -> Result<(), i32> {
        self.inner.finish(zone)?;
        self.ops.push(Op::Finish { zone });
        Ok(())
    }

    fn reset(&mut self, zone: u32) -> Result<(), i32> {
        self.inner.reset(zone)?;
        self.ops.push(Op::Reset { zone });
        Ok(())
    }
}

/// What one invocation was asked for.
struct Asked {
    /// Unit: count of publishes.
    publishes: u64,
    /// Unit: count of seeds.
    seeds: u64,
    /// Unit: none — the first seed.
    seed: u64,
    /// Sweep every publish rather than the four that matter.
    all: bool,
    /// Narrow to one publish. Unit: count of publishes since the superblock.
    target: Option<u64>,
    /// Narrow to one cut point. Unit: count of device operations.
    cut: Option<usize>,
    /// Narrow to one granularity.
    granularity: Option<Granularity>,
    /// Narrow to one mode.
    mode: Option<Mode>,
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
    let mut asked = Asked {
        publishes: DEFAULT_PUBLISHES,
        seeds: DEFAULT_SEEDS,
        seed: DEFAULT_SEED,
        all: false,
        target: None,
        cut: None,
        granularity: None,
        mode: None,
        list: false,
        unselected: None,
    };
    let mut filter: Option<String> = None;

    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        let mut value = || walk.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--publishes" => asked.publishes = number(&value()?)?,
            "--seeds" => asked.seeds = number(&value()?)?,
            "--seed" => asked.seed = number(&value()?)?,
            "--target" => asked.target = Some(number(&value()?)?),
            "--cut" => asked.cut = Some(number(&value()?)? as usize),
            "--granularity" => {
                asked.granularity = Some(match value()?.as_str() {
                    "block" => Granularity::Block,
                    "byte" => Granularity::Byte,
                    other => return Err(format!("`{other}` is not block or byte")),
                });
            }
            "--mode" => {
                asked.mode = Some(match value()?.as_str() {
                    "honest" => Mode::Honest,
                    "lying" => Mode::Lying,
                    other => return Err(format!("`{other}` is not honest or lying")),
                });
            }
            "--all" => asked.all = true,
            "--list" => asked.list = true,
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
        asked.unselected = Some(name);
        asked.list = false;
        return Ok(asked);
    }
    if asked.publishes < 2 {
        return Err("--publishes below 2 leaves no publish with an older root behind it, \
                    so `the old one or the new one` would have only one answer and the \
                    sweep could not tell them apart. R04."
            .into());
    }
    if asked.seeds == 0 {
        return Err("--seeds 0 sweeps nothing and exits zero, which is the one result this \
                    run must not be able to produce. R04."
            .into());
    }
    Ok(asked)
}

/// A decimal or `0x`-prefixed hexadecimal number.
fn number(text: &str) -> Result<u64, String> {
    text.strip_prefix("0x")
        .map_or_else(|| text.parse::<u64>().ok(), |hex| u64::from_str_radix(hex, 16).ok())
        .ok_or_else(|| format!("`{text}` is not a number"))
}

/// One draw site, seeded by identity under RFC 0026.
///
/// Split rather than chained, so that adding a fourth site later moves neither
/// of the three that are here and generation 12 is the same generation whatever
/// the run before it drew.
fn site(seed: u64, identity: &str) -> u64 {
    derive(seed, label(identity))
}

/// The bytes of one component leaf.
///
/// Keyed by generation and by which leaf, so a leaf is a pure function of
/// `(seed, generation, index)` — which is what lets the replay pass re-run the
/// publishes before the target and get the identical device back.
fn leaf(seed: u64, generation: u64, which: u64) -> Vec<u8> {
    let identity = derive(derive(site(seed, "e2-p01/leaf"), generation), which);
    let mut env = SeededEnv::new(identity, 1);
    let span = (LEAF_MAX_BYTES - LEAF_MIN_BYTES) as u64;
    let len = LEAF_MIN_BYTES + (env.next_u64() % span) as usize;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        out.extend_from_slice(&env.next_u64().to_le_bytes());
    }
    out.truncate(len);
    out
}

/// The device this run needs, sized from the workload rather than guessed at.
fn geometry(seed: u64, publishes: u64) -> (u32, Superblock) {
    // An over-estimate on purpose: the exact figure needs the chunker's cuts and
    // the zone seals, and a fill that hit `FULL` would be a failure of this file
    // rather than of the design.
    let mut blocks = 1u64;
    for generation in 1..=publishes {
        for which in 0..LEAVES {
            blocks += (leaf(seed, generation, which).len() / BLOCK_BYTES) as u64 + 3;
        }
        blocks += 1;
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

/// Publish one generation: two component leaves, a generation node naming them,
/// and RFC 0060's barrier sequence over the lot.
fn publish_one<Z: Zoned>(
    store: &mut Store<ZoneMap<Z>>,
    roots: &mut Roots,
    seed: u64,
    generation: u64,
    previous: [u8; 32],
) -> Result<RootRecord, i32> {
    let mut publisher = Publisher::open(roots, store);
    let mut children: Vec<[u8; 32]> = Vec::new();
    let mut object_bytes = 0u64;
    for which in 0..LEAVES {
        let bytes = leaf(seed, generation, which);
        object_bytes += bytes.len() as u64;
        children.push(store.put_object(&bytes)?);
    }

    // The generation node: a head and the hashes of its children, which is the
    // shape `zone/src/mount.rs` walks and the paragraph there says why it is
    // this shape until `E2-B07` writes a folded tree.
    let head = ObjectHead { object_bytes, chunks: children.len() as u32 };
    let mut content = Vec::with_capacity(head.bytes());
    content.extend_from_slice(&head.to_bytes());
    for child in &children {
        content.extend_from_slice(child);
    }
    let root = store.put(kind::GENERATION, &content)?;

    publisher.appended(roots, store)?;
    let record =
        RootRecord { generation, root, frame: [0u8; 32], module: root, previous, check: [0u8; 32] };
    publisher.commit(store, roots, &record)
}

/// Publish generations `1..=through` into a fresh device.
///
/// Answers the store and every record it wrote, in generation order.
fn publish_through<Z: Zoned>(
    device: Z,
    layout: &Superblock,
    seed: u64,
    through: u64,
) -> Result<(Store<ZoneMap<Z>>, Vec<RootRecord>), i32> {
    let map = ZoneMap::new(device, DATA_FROM)?;
    let mut store = Store::format(map, layout)?;
    let mut roots = Roots::new();
    let mut written = Vec::new();
    let mut previous = [0u8; 32];
    for generation in 1..=through {
        let record = publish_one(&mut store, &mut roots, seed, generation, previous)?;
        previous = record.root;
        written.push(record);
    }
    Ok((store, written))
}

/// One publish's operation trace, and the two roots a cut inside it may leave.
struct Recorded {
    /// Unit: count of publishes since the superblock.
    target: u64,
    ops: Vec<Op>,
    /// The root a cut that landed nothing leaves, or `None` when the target is
    /// the first publish and there is nothing behind it.
    /// Unit: none — a content address.
    old: Option<[u8; 32]>,
    /// The root a cut that landed everything leaves.
    /// Unit: none — a content address.
    new: [u8; 32],
    /// Whether this publish crossed a root-zone wrap, which is the third
    /// required observation and is read off the trace rather than predicted.
    crosses_a_wrap: bool,
}

/// Run the workload up to and including `target`, recording what the device was
/// asked to do during that last publish.
fn record(seed: u64, target: u64, layout: &Superblock, zones: u32) -> Result<Recorded, i32> {
    let device = Recorder::new(ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, zones));
    let (mut store, records) = publish_through(device, layout, seed, target - 1)?;
    let from = store.device_mut().device_mut().ops.len();

    let mut roots = Roots::new();
    // The transient-root bookkeeping is re-opened here rather than carried
    // across, because what the cut model needs from this pass is the *device*
    // trace and the collector never runs in this file: `zone/tests/cycle.rs` is
    // where `Roots` is the subject.
    let previous = records.last().map_or([0u8; 32], |record| record.root);
    let record = publish_one(&mut store, &mut roots, seed, target, previous)?;

    let ops: Vec<Op> = store.device_mut().device_mut().ops[from..].to_vec();
    let crosses_a_wrap = ops.iter().any(|op| matches!(op, Op::Reset { .. }));
    Ok(Recorded {
        target,
        ops,
        old: records.last().map(|record| record.root),
        new: record.root,
        crosses_a_wrap,
    })
}

/// Put one operation on the media, or the first `torn` bytes of it.
fn land(device: &mut ZonedMemory, op: &Op, torn: Option<usize>) -> Result<(), i32> {
    match op {
        Op::Write { block, bytes } | Op::Append { at: block, bytes } => {
            let take = torn.unwrap_or(bytes.len()).min(bytes.len());
            device.land(*block, &bytes[..take])
        }
        Op::Finish { zone } => device.finish(*zone),
        Op::Reset { zone } => device.reset(*zone),
        // A barrier moves no byte. What it does is decide which operations
        // before it the model may not drop, and that decision is made by the
        // caller before this is reached.
        Op::Flush => Ok(()),
    }
}

/// The identity every draw for one cut is derived from: the re-run key itself.
fn key(
    seed: u64,
    target: u64,
    cut: usize,
    granularity: Granularity,
    mode: Mode,
    what: &str,
) -> u64 {
    let flavour = u64::from(granularity == Granularity::Byte) << 1 | u64::from(mode == Mode::Lying);
    derive(derive(derive(derive(site(seed, what), target), cut as u64), flavour), 0)
}

/// Apply the model: land what a completed barrier covers, then a drawn subset of
/// the rest in a drawn order, with the last operation torn at byte granularity.
///
/// Answers how many operations landed, for the report.
fn apply_the_cut(
    device: &mut ZonedMemory,
    recorded: &Recorded,
    cut: usize,
    granularity: Granularity,
    mode: Mode,
    seed: u64,
) -> Result<usize, i32> {
    let prefix = &recorded.ops[..cut];
    // Everything before the last completed `FLUSH` in the prefix. In `lying`
    // mode there is no such thing, which is the whole of what that mode is.
    let covered = match mode {
        Mode::Honest => {
            prefix.iter().rposition(|op| matches!(op, Op::Flush)).map_or(0, |at| at + 1)
        }
        Mode::Lying => 0,
    };
    for op in &prefix[..covered] {
        land(device, op, None)?;
    }

    let mut subset = SeededEnv::new(key(seed, recorded.target, cut, granularity, mode, SUBSET), 1);
    let mut chosen: Vec<usize> =
        (covered..cut).filter(|_| subset.next_u64().is_multiple_of(2)).collect();

    // A drawn order, and it is drawn even though today it changes nothing: no
    // publish writes one block twice, so landing distinct positions in any order
    // gives one device. The day a publish rewrites a block — the superblock is
    // the only rewritable one this format has — the order is already a draw and
    // not a change to this file.
    let mut order = SeededEnv::new(key(seed, recorded.target, cut, granularity, mode, ORDER), 1);
    for index in (1..chosen.len()).rev() {
        let swap = (order.next_u64() % (index as u64 + 1)) as usize;
        chosen.swap(index, swap);
    }

    // The cut falls *inside* the last operation submitted. At block granularity
    // that operation landed whole or not at all; at byte granularity a prefix of
    // its bytes is on the media, which is the only way a torn root record — one
    // of the four required observations — can be produced at all.
    let torn_at = cut.checked_sub(1);
    let mut tear = SeededEnv::new(key(seed, recorded.target, cut, granularity, mode, TEAR), 1);
    let bytes_landed = (tear.next_u64() % BLOCK_BYTES as u64) as usize;

    for index in &chosen {
        let torn = if granularity == Granularity::Byte && Some(*index) == torn_at {
            Some(bytes_landed)
        } else {
            None
        };
        land(device, &recorded.ops[*index], torn)?;
    }
    Ok(covered + chosen.len())
}

/// The draw site names, so that a typo in one is a compile error rather than a
/// second stream.
const SUBSET: &str = "e2-p01/subset";
const ORDER: &str = "e2-p01/order";
const TEAR: &str = "e2-p01/tear";

/// What a mount after a cut found, in the only three shapes the exit admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// The root that was there before the publish began.
    Old,
    /// The root the publish was writing.
    New,
    /// Anything else, which is what the exit forbids.
    Third,
}

/// Everything the sweep counts.
#[derive(Default)]
struct Counted {
    /// Unit: count of cuts.
    cuts: u64,
    /// Unit: count of cuts leaving the old root.
    old: u64,
    /// Unit: count of cuts leaving the new root.
    new: u64,
    /// Unit: count of cuts at byte granularity that left a torn root record.
    torn: u64,
    /// Unit: count of root records refused by resolution in `lying` mode.
    lying_unresolved: u64,
    /// Unit: count of cuts inside a publish that crosses a root-zone wrap.
    across_a_wrap: u64,
    /// Unit: count of device operations replayed.
    landed: u64,
}

/// The device every cut in one run is replayed onto, and the workload it was
/// sized for.
///
/// Held together because they are one decision: the geometry is computed from
/// the publish count, and a cut replayed onto a device sized for a different
/// workload is a cut of something else. It is also what keeps [`one_cut`]'s
/// signature to the five things that vary per cut.
struct Fixture {
    layout: Superblock,
    /// Unit: count of zones.
    zones: u32,
    /// Unit: count of publishes.
    publishes: u64,
}

/// Sweep one cut point, and answer a finding when the exit does not hold.
fn one_cut(
    fixture: &Fixture,
    seed: u64,
    recorded: &Recorded,
    cut: usize,
    granularity: Granularity,
    mode: Mode,
    counted: &mut Counted,
) -> Result<(), String> {
    let repro = || {
        format!(
            "cargo test -p f-zone --test cut -- --seed {seed} --seeds 1 \
             --publishes {} --target {} --cut {cut} --granularity {} --mode {}",
            fixture.publishes,
            recorded.target,
            granularity.name(),
            mode.name()
        )
    };
    let refuse = |what: &str, refusal: i32| {
        format!("{what} refused with {refusal:#x}\n  reproduce: {}", repro())
    };

    // The publishes before the target, replayed from scratch. Re-run rather than
    // snapshotted: a snapshot of a device model is a second representation of
    // its state, and this file would then be asserting that the two agree.
    let device = ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, fixture.zones);
    let (store, _) = publish_through(device, &fixture.layout, seed, recorded.target - 1)
        .map_err(|refusal| refuse("the publishes before the cut", refusal))?;
    let mut device = store.into_device().into_device();

    let landed = apply_the_cut(&mut device, recorded, cut, granularity, mode, seed)
        .map_err(|refusal| refuse("landing the cut", refusal))?;

    let (found, _store) =
        mount(device, DATA_FROM).map_err(|refusal| refuse("the mount after the cut", refusal))?;

    counted.cuts += 1;
    counted.landed += landed as u64;
    if recorded.crosses_a_wrap {
        counted.across_a_wrap += 1;
    }
    if granularity == Granularity::Byte {
        counted.torn += found.roots_refused_check;
    }
    if mode == Mode::Lying {
        counted.lying_unresolved += found.roots_refused_unresolved;
    }

    let outcome = outcome_of(&found, recorded);
    match outcome {
        Outcome::Old => counted.old += 1,
        Outcome::New => counted.new += 1,
        Outcome::Third => {
            return Err(format!(
                "a cut left a third thing.\n  \
                 the publish was generation {}, the cut fell after {cut} of {} operation(s) \
                 ({}), at {} granularity in {} mode.\n  \
                 the old root was {}, the new root is {}, and the mount answered {}.\n  \
                 reproduce: {}",
                recorded.target,
                recorded.ops.len(),
                recorded.ops.get(cut.saturating_sub(1)).map_or("nothing", Op::name),
                granularity.name(),
                mode.name(),
                recorded.old.map_or_else(|| "nothing".into(), hex),
                hex(recorded.new),
                found.root.map_or_else(
                    || "nothing".into(),
                    |record| format!("generation {} at {}", record.generation, hex(record.root))
                ),
                repro()
            ));
        }
    }

    // The barrier's own property, and the only one that can see a publish which
    // stopped issuing it. A root refused for non-resolution in `honest` mode
    // means a root record was on the media over blobs that were not, which the
    // first `FLUSH` exists to make impossible.
    if mode == Mode::Honest && found.roots_refused_unresolved > 0 {
        return Err(format!(
            "an honest device refused {} root record(s) because the generation tree \
             under them did not resolve.\n  \
             the publish was generation {}, the cut fell after {cut} of {} operation(s) \
             ({}), at {} granularity.\n  \
             RFC 0060's first barrier is what makes this impossible: every hash a root \
             record names is on stable media before the record naming them exists. A \
             build that skips it can produce this, and `mutate-root-before-blobs` is \
             that build.\n  \
             reproduce: {}",
            found.roots_refused_unresolved,
            recorded.target,
            recorded.ops.len(),
            recorded.ops.get(cut.saturating_sub(1)).map_or("nothing", Op::name),
            granularity.name(),
            repro()
        ));
    }
    Ok(())
}

/// Which of the two intended states this mount landed in.
fn outcome_of(found: &Mounted, recorded: &Recorded) -> Outcome {
    match (found.root, recorded.old) {
        // Nothing at all, before the first publish. That *is* the old state:
        // the device had no root when this publish began, so a cut that landed
        // nothing leaves it with none. For any later publish there is a root
        // behind this one and `None` is the third thing.
        (None, None) => Outcome::Old,
        (None, Some(_)) => Outcome::Third,
        (Some(record), old) => {
            if record.root == recorded.new && record.generation == recorded.target {
                Outcome::New
            } else if old.is_some_and(|old| record.root == old)
                && record.generation == recorded.target - 1
            {
                Outcome::Old
            } else {
                Outcome::Third
            }
        }
    }
}

/// A hash, short enough to read in a failure.
fn hex(hash: [u8; 32]) -> String {
    let mut out = String::with_capacity(16);
    for byte in &hash[..8] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The publishes a default run sweeps every cut point of.
///
/// Four rather than all of them, and each is here for a reason a longer run does
/// not have to argue: the first publish, which is the only one with no root
/// behind it; one in the middle, which is the ordinary case; the publish that
/// crosses a root-zone wrap, which is the one that resets a zone; and the one
/// after it, which is the first publish into a zone that has just been carried
/// into. `--all` sweeps every publish and is what `cargo xtask cut` runs.
fn targets(asked: &Asked, wrap_at: Option<u64>) -> Vec<u64> {
    if let Some(one) = asked.target {
        return vec![one];
    }
    if asked.all {
        return (1..=asked.publishes).collect();
    }
    let mut out = vec![1, asked.publishes / 2];
    if let Some(at) = wrap_at {
        out.push(at);
        if at < asked.publishes {
            out.push(at + 1);
        }
    }
    out.retain(|target| *target >= 1 && *target <= asked.publishes);
    out.sort_unstable();
    out.dedup();
    out
}

/// Which publish first crosses a root-zone wrap, found by publishing and
/// watching for a `ZONE_RESET` rather than by predicting it from arithmetic.
///
/// Predicting it would mean this file holding a second copy of `ROOT_CARRY`'s
/// consequences, and the two would part company the day the wrap's rule
/// changed — which is exactly the day the sweep most needs to still be aiming at
/// it.
fn first_wrap(seed: u64, publishes: u64, layout: &Superblock, zones: u32) -> Option<u64> {
    let device = Recorder::new(ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, zones));
    let map = ZoneMap::new(device, DATA_FROM).ok()?;
    let mut store = Store::format(map, layout).ok()?;
    let mut roots = Roots::new();
    let mut previous = [0u8; 32];
    for generation in 1..=publishes {
        let before = store.device_mut().device_mut().ops.len();
        let record = publish_one(&mut store, &mut roots, seed, generation, previous).ok()?;
        previous = record.root;
        let reset = store.device_mut().device_mut().ops[before..]
            .iter()
            .any(|op| matches!(op, Op::Reset { .. }));
        if reset {
            return Some(generation);
        }
    }
    None
}

/// One run, and the status it earns.
#[expect(
    clippy::too_many_lines,
    reason = "the sweep is one nest — seeds, targets, granularities, modes, cut points — and \
              every level of it shares the geometry, the recording and the counters. Splitting \
              it would mean passing all three through five signatures, and the one thing a \
              reader has to be able to check about a sweep is what it actually covers."
)]
fn run(asked: &Asked) -> i32 {
    println!(
        "cut — {} publish(es), {} seed(s) from {:#018x}, both granularities and both modes",
        asked.publishes, asked.seeds, asked.seed
    );

    let (zones, layout) = geometry(asked.seed, asked.publishes);
    let fixture = Fixture { layout, zones, publishes: asked.publishes };
    println!(
        "  device: {zones} zone(s) of {ZONE_BLOCKS} block(s) of {BLOCK_BYTES} bytes, \
         data from zone {DATA_FROM}, ROOT_CARRY = {ROOT_CARRY}"
    );

    let wrap_at = first_wrap(asked.seed, asked.publishes, &fixture.layout, zones);
    match wrap_at {
        Some(at) => println!("  the first root-zone wrap is at generation {at}"),
        None => {
            return fail(&format!(
                "no publish in {} crossed a root-zone wrap, so the third required \
                 observation is unreachable and a green run would be a sweep of the \
                 model. Publish more generations, or make ZONE_BLOCKS smaller.",
                asked.publishes
            ));
        }
    }

    let chosen = targets(asked, wrap_at);
    println!("  sweeping every cut point of publish(es) {chosen:?}");
    println!();

    let mut counted = Counted::default();
    let mut findings: Vec<String> = Vec::new();
    let granularities = match asked.granularity {
        Some(one) => vec![one],
        None => vec![Granularity::Block, Granularity::Byte],
    };
    let modes = match asked.mode {
        Some(one) => vec![one],
        None => vec![Mode::Honest, Mode::Lying],
    };

    for step in 0..asked.seeds {
        // Derived rather than added, so that consecutive seeds are not
        // consecutive streams: `--seeds 4` and four runs of `--seed` at the
        // values it printed have to be the same sweep.
        let seed = if step == 0 { asked.seed } else { derive(asked.seed, step) };
        for target in &chosen {
            let recorded = match record(seed, *target, &fixture.layout, zones) {
                Ok(recorded) => recorded,
                Err(refusal) => {
                    return fail(&format!(
                        "recording publish {target} at seed {seed:#018x} refused with \
                         {refusal:#x}"
                    ));
                }
            };
            let cuts: Vec<usize> = match asked.cut {
                Some(one) => vec![one.min(recorded.ops.len())],
                // Zero — nothing landed — through every operation. The upper end
                // is the whole publish, which must leave the new root and is the
                // control on the lower end leaving the old one.
                None => (0..=recorded.ops.len()).collect(),
            };
            for granularity in &granularities {
                for mode in &modes {
                    for cut in &cuts {
                        if let Err(finding) = one_cut(
                            &fixture,
                            seed,
                            &recorded,
                            *cut,
                            *granularity,
                            *mode,
                            &mut counted,
                        ) {
                            findings.push(finding);
                        }
                    }
                }
            }
        }
    }

    println!("  cuts swept                       {:>10}", counted.cuts);
    println!("  device operations replayed       {:>10}", counted.landed);
    println!("  cuts leaving the old root        {:>10}", counted.old);
    println!("  cuts leaving the new root        {:>10}", counted.new);
    println!("  cuts inside a wrapping publish   {:>10}", counted.across_a_wrap);
    println!("  torn root records refused        {:>10}  (byte granularity)", counted.torn);
    println!("  roots refused unresolved         {:>10}  (lying mode)", counted.lying_unresolved);
    println!();

    if !findings.is_empty() {
        for finding in findings.iter().take(8) {
            println!("finding: {finding}\n");
        }
        return fail(&format!(
            "{} of {} cut(s) did not leave one of the two intended states, or left a \
             root an honest device could not have written.",
            findings.len(),
            counted.cuts
        ));
    }

    // The four observations, checked rather than hoped for. A narrowed run —
    // one cut, one mode — cannot reach them and does not pretend to: what it is
    // for is reproducing a finding, and it says so.
    if asked.cut.is_some() || asked.granularity.is_some() || asked.mode.is_some() {
        println!(
            "cut: ok — narrowed to one point, so the four required observations were not \
             asked for. The full sweep is what makes them."
        );
        return 0;
    }
    let distinct = u64::from(counted.old > 0) + u64::from(counted.new > 0);
    for (seen, what) in [
        (counted.torn > 0, "a torn root record refused by its `check` field, at byte granularity"),
        (
            counted.lying_unresolved > 0,
            "a root refused by resolution in `lying` mode, counted in roots_refused_unresolved",
        ),
        (counted.across_a_wrap > 0, "a cut inside a publish that crosses a root-zone wrap"),
        (distinct == 2, "exactly two distinct mount outcomes"),
    ] {
        if !seen {
            return fail(&format!(
                "the sweep never observed: {what}.\n\n\
                 A sweep that has never failed is indistinguishable from one that cannot, \
                 and a sweep whose artefact shows none of these is a sweep of the model. \
                 Widen it — more seeds, more publishes — or find out why the case has \
                 become unreachable, which is a finding about the format and not about \
                 this file."
            ));
        }
    }

    println!(
        "cut: ok — every one of {} cut(s) left the old root or the new one, and the \
         generation tree under it resolved.",
        counted.cuts
    );
    println!(
        "     the four required observations were made: a torn record refused, a lying \
         device's root refused by resolution, a cut across a wrap, and two outcomes."
    );
    0
}

/// Report a failure the way this target's caller reads one.
fn fail(why: &str) -> i32 {
    println!("\ncut: FAILED\n  {why}");
    1
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = match parse(&args) {
        Ok(asked) => asked,
        Err(why) => {
            println!("cut: {why}");
            exit(2);
        }
    };
    if let Some(name) = &asked.unselected {
        println!("cut: no test matched `{name}`");
        exit(0);
    }
    if asked.list {
        println!("{TEST_NAME}: test");
        exit(0);
    }
    exit(run(&asked));
}
