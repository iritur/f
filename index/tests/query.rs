// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-B03`'s exit, as a count: **device blocks read per query, against device
//! blocks read per tree walk over the same data.**
//!
//! # Why a count of blocks and not a count of boundaries
//!
//! Because a count of boundaries cannot differ. The exit says *a query returns a
//! hash without crossing a component boundary*, and inside one component a tree
//! walk crosses no boundary either — so ring crossings would be zero against
//! zero and the exit would be vacuous. The spec settles the measurement on
//! device blocks read, which is a count that *can* differ, and it reads the
//! exit's phrase as *the index is not a service* while flagging that reading as
//! weaker than the words. This file measures the flagged reading and does not
//! widen it.
//!
//! # The two sides, and why they are the same data counted the same way
//!
//! One workload: [`PATHS`] paths, each naming a 32-byte content address, the
//! address being SHA-256 over the path so that neither side stores a table the
//! other could differ from.
//!
//! - **The index.** The paths go into `f_index::Index` in the `path` namespace,
//!   the log is barriered, the device is handed back, and the index is mounted
//!   again from nothing. Then every path is looked up.
//! - **The tree walk.** The same paths go onto a second modelled device as an
//!   ordinary directory tree: one block per node, [`FANOUT`] entries per node,
//!   [`DEPTH`] levels, interior nodes naming child blocks and leaves naming
//!   hashes. Then every path is walked from the root.
//!
//! Both devices are the same `f_blob::device::Memory` behind the same
//! [`Counted`] wrapper, which counts `Device::read` and nothing else. One block
//! per read, both sides, so the two numbers are in one unit.
//!
//! # What is deliberately generous to the tree walk
//!
//! Its nodes are laid out contiguously by arithmetic, so it never reads a node
//! to find out where the next one is, and its fanout is chosen so that its whole
//! interior would fit in a cache this measurement does not model. A real
//! directory tree over a content-addressed store would read a blob header before
//! each node and verify each node's hash. Every one of those omissions makes the
//! baseline cheaper than the real thing, which is the direction an honest
//! comparison errs in: the index's margin is understated, not flattered.
//!
//! # What this does not measure
//!
//! Time. Nothing here reads a clock — the number is a count, reproducible on any
//! machine, and that is the whole reason the exit was settled on a count.

use std::process::exit;

use f_abi::store::refusal;
use f_blob::device::{Device, Memory};
use f_hash::sha256;
use f_index::{Index, ns};

/// The name this target answers to when cargo's harness protocol asks for one.
///
/// `harness = false` means the binary *is* the test, so there is exactly one and
/// this is what it is called. A filter aimed at another target in this package
/// is handed to this one too, and a target that ran its whole workload for a
/// filter that did not select it would make `cargo test <anything>` slow for no
/// reason. `blob/tests/million.rs` argues this split and it is the same split.
const TEST_NAME: &str = "query";

/// How many children a directory node in the baseline holds.
///
/// Unit: count of entries. Eight, because a leaf entry is a name and a 32-byte
/// hash — 40 bytes — and eight of them plus a count fit in one 512-byte block
/// with room to spare. A larger fanout would need a larger block or a spanning
/// node, and a spanning node is a second block read that would flatter this
/// measurement rather than test it.
const FANOUT: usize = 8;

/// How many levels the baseline tree has, and therefore how many components a
/// path has.
///
/// Unit: count of levels. Four, so the walk reads four blocks and the number is
/// visibly a *depth* rather than a constant somebody chose.
const DEPTH: usize = 4;

/// How many paths the workload holds.
///
/// Unit: count of paths. `FANOUT.pow(DEPTH)` — a full tree, so that every leaf
/// is full and no query is answered cheaply by a short node. Four thousand and
/// ninety-six, which is a second of host time and is the number printed beside
/// every count below.
const PATHS: usize = FANOUT.pow(DEPTH as u32);

/// How many bytes one path component is.
///
/// Unit: bytes. Fixed width, so the baseline's node encoding needs no length
/// prefix per entry: a length prefix would be a byte of decoding the index does
/// not have to do either, and the two sides are being compared on reads rather
/// than on decoding.
const COMPONENT_BYTES: usize = 8;

/// The modelled device's logical block, both sides.
///
/// Unit: bytes. The smallest a real device reports and the smallest this format
/// accepts, which is also the size the widest index entry was bounded against.
const BLOCK_BYTES: usize = 512;

/// How many blocks the index's log region is given.
///
/// Unit: count of blocks. Comfortably more than the workload needs, because the
/// number being measured is how many blocks the log *uses* and a region sized to
/// the answer would be a region that had assumed it.
const LOG_BLOCKS: u64 = 2048;

/// The bound the index's total device cost is held to, as a divisor of the tree
/// walk's.
///
/// Unit: none — a ratio. The measured ratio is far larger than this; the bound
/// is eight because eight is what the *design* promises — a log whose density
/// halved twice and a walk one level shallower would still be inside it — and a
/// bound set at the measurement is a bound that fails on the first honest change
/// to either side. A number that moves needs an RFC carrying a measurement; this
/// one is stated once, here, with the reason it is not tighter.
const TOTAL_ADVANTAGE_MIN: u64 = 8;

/// How many queries the index's mount cost may take to pay for itself.
///
/// Unit: count of queries. The index pays a linear scan once and then reads
/// nothing; the walk pays [`DEPTH`] blocks every time. This is the crossover,
/// and it is asserted because it is the number that says whether "amortised" is
/// a description or a hope.
const BREAK_EVEN_MAX: u64 = 256;

/// A device that counts the blocks read through it.
///
/// # Why the counter is here and not on `Memory`
///
/// Because both sides have to be counted by one thing. `Memory` counts writes
/// and barriers already and could have been given a third counter, but then the
/// baseline and the index would be counted by two instances of a counter in
/// another crate, and the claim "both sides counted the same way" would rest on
/// that crate not having been edited between them. One wrapper, wrapped around
/// both, makes it rest on nothing.
struct Counted<D: Device> {
    inner: D,
    /// Unit: count of blocks read.
    reads: u64,
}

impl<D: Device> Counted<D> {
    fn new(inner: D) -> Self {
        Self { inner, reads: 0 }
    }

    /// Unit: count of blocks read.
    fn reads(&self) -> u64 {
        self.reads
    }

    /// Start a phase. The measurement is per phase — build, mount, query — and a
    /// counter that ran across all three would answer a question nobody asked.
    fn reset(&mut self) {
        self.reads = 0;
    }
}

impl<D: Device> Device for Counted<D> {
    fn block_bytes(&self) -> usize {
        self.inner.block_bytes()
    }

    fn blocks(&self) -> u64 {
        self.inner.blocks()
    }

    fn read(&mut self, block: u64, into: &mut [u8]) -> Result<(), i32> {
        // Counted before the call and not after, so a refused read is still a
        // read: a measurement that only counted successes would let a side that
        // failed half its reads look cheap.
        self.reads += 1;
        self.inner.read(block, into)
    }

    fn write(&mut self, block: u64, from: &[u8]) -> Result<(), i32> {
        self.inner.write(block, from)
    }

    fn flush(&mut self) -> Result<(), i32> {
        self.inner.flush()
    }
}

/// One path component: the level it is at, and which of the [`FANOUT`] children
/// it is.
///
/// Distinct within a node and across levels, so a walk that compared against the
/// wrong level's name would fail rather than match by luck.
fn component(level: usize, child: usize) -> [u8; COMPONENT_BYTES] {
    let mut out = [b'-'; COMPONENT_BYTES];
    out[0] = b'd';
    out[1] = b'0' + level as u8;
    out[COMPONENT_BYTES - 1] = b'a' + child as u8;
    out
}

/// The `index`th path of the workload, as bytes.
///
/// The digits of `index` in base [`FANOUT`], most significant first, so that the
/// path and the position in the tree are the same fact written two ways — which
/// is what makes "the same data" true rather than asserted.
fn path(index: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(DEPTH * (COMPONENT_BYTES + 1));
    for level in 1..=DEPTH {
        if level > 1 {
            out.push(b'/');
        }
        let shift = DEPTH - level;
        out.extend_from_slice(&component(level, index / FANOUT.pow(shift as u32) % FANOUT));
    }
    out
}

/// What a path names.
///
/// Derived and not stored, so the index's answer and the walk's answer are
/// compared against one definition rather than against two tables that could
/// differ. Nothing is drawn: the workload is a full tree of fixed names, so
/// there is nothing here for a seed to decide and no `Env` is asked for one.
fn names(path: &[u8]) -> [u8; 32] {
    sha256(path)
}

/// The first block of level `level` in the baseline tree.
///
/// Unit: block index, zero-based. Levels are laid out one after another and
/// nodes within a level in order, so a child's address is arithmetic and the
/// walk never reads a node to find out where the next one is.
fn level_base(level: usize) -> u64 {
    (1..level).map(|earlier| FANOUT.pow(earlier as u32 - 1) as u64).sum()
}

/// How many blocks the baseline tree occupies.
///
/// Unit: count of blocks.
fn tree_blocks() -> u64 {
    level_base(DEPTH + 1)
}

/// Write the baseline directory tree.
///
/// Interior node: a `u16` count, then that many entries of a component name and
/// a `u64` child block. Leaf node: a `u16` count, then that many entries of a
/// component name and a 32-byte hash. No magic and no length, because this is a
/// baseline and not a format — giving it the store's verify-before-accept would
/// be giving it work the index is not being charged for either.
fn build_tree<D: Device>(device: &mut D) -> Result<(), i32> {
    let mut block = vec![0u8; BLOCK_BYTES];
    for level in 1..=DEPTH {
        let nodes = FANOUT.pow(level as u32 - 1);
        let leaf = level == DEPTH;
        let width = if leaf { COMPONENT_BYTES + 32 } else { COMPONENT_BYTES + 8 };
        for node in 0..nodes {
            block.fill(0);
            block[0..2].copy_from_slice(&(FANOUT as u16).to_le_bytes());
            for child in 0..FANOUT {
                let at = 2 + child * width;
                block[at..at + COMPONENT_BYTES].copy_from_slice(&component(level, child));
                let value = at + COMPONENT_BYTES;
                if leaf {
                    // The leaf's entries are the paths whose last component is
                    // this child: the node index *is* the path index divided by
                    // the fanout, which is the arithmetic `path` writes forwards.
                    let which = node * FANOUT + child;
                    block[value..value + 32].copy_from_slice(&names(&path(which)));
                } else {
                    let below = level_base(level + 1) + (node * FANOUT + child) as u64;
                    block[value..value + 8].copy_from_slice(&below.to_le_bytes());
                }
            }
            device.write(level_base(level) + node as u64, &block)?;
        }
    }
    device.flush()
}

/// Walk the baseline tree for one path and answer what it names.
///
/// One block read per level, every level, every time — which is the number this
/// measurement is against.
fn walk<D: Device>(device: &mut D, path: &[u8]) -> Result<[u8; 32], i32> {
    let mut block = vec![0u8; BLOCK_BYTES];
    let mut at = 0u64;
    for (level, wanted) in path.split(|byte| *byte == b'/').enumerate() {
        device.read(at, &mut block)?;
        let count = u16::from_le_bytes([block[0], block[1]]) as usize;
        let leaf = level + 1 == DEPTH;
        let width = if leaf { COMPONENT_BYTES + 32 } else { COMPONENT_BYTES + 8 };
        let mut found = None;
        for entry in 0..count {
            let start = 2 + entry * width;
            if &block[start..start + COMPONENT_BYTES] == wanted {
                found = Some(start + COMPONENT_BYTES);
                break;
            }
        }
        let value = found.ok_or(refusal::ADDRESS)?;
        if leaf {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&block[value..value + 32]);
            return Ok(hash);
        }
        at = u64::from_le_bytes(block[value..value + 8].try_into().map_err(|_| refusal::ADDRESS)?);
    }
    Err(refusal::ADDRESS)
}

/// Say what went wrong and leave non-zero.
///
/// A named function rather than `assert!`, so that every failure this run can
/// produce prints the two counts it is about — a bound that fails without its
/// numbers beside it is a bound somebody has to re-run to understand.
fn require(held: bool, what: &str) {
    if !held {
        println!("query: FAILED — {what}");
        exit(1);
    }
}

/// Cargo's harness protocol, in the two parts of it that matter here.
///
/// `--list` names the one test and runs nothing; a bare filter that does not
/// select this target exits zero having run nothing, because `cargo test` hands
/// every target in a package the same arguments. The flags libtest owns are
/// accepted and ignored. There are no options of this run's own: the workload is
/// fixed, because the exit is the measurement and a measurement with a size
/// argument is two measurements.
fn selected(args: &[String]) -> bool {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--list" => {
                println!("{TEST_NAME}: test");
                exit(0);
            }
            "--test-threads" | "--color" | "--format" | "--logfile" | "--skip" | "-Z" => {
                let _ = rest.next();
            }
            other if other.starts_with('-') => {}
            other if !TEST_NAME.contains(other) => {
                println!("query: 0 tests run (filter `{other}` selected nothing here)");
                return false;
            }
            _ => {}
        }
    }
    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !selected(&args) {
        return;
    }

    // ---- the index -------------------------------------------------------
    let device = Counted::new(Memory::new(BLOCK_BYTES, LOG_BLOCKS));
    let mut index = Index::mount(device, 0, LOG_BLOCKS).expect("a log region on this device");
    for which in 0..PATHS {
        let name = path(which);
        index.set(ns::PATH, &name, &names(&name)).expect("a name this format holds");
    }
    index.barrier().expect("the model's barrier cannot fail");
    let log_blocks = index.blocks_used();

    // Mounted again from nothing, so what is measured is the cost of arriving at
    // an answerable index rather than the cost of having built one.
    let mut device = index.into_device();
    device.reset();
    let mut index = Index::mount(device, 0, LOG_BLOCKS).expect("a remount");
    let mount_blocks = index.device().reads();

    index.device_mut().reset();
    let mut answered = 0usize;
    for which in 0..PATHS {
        let name = path(which);
        if index.get(ns::PATH, &name) == Some(names(&name)) {
            answered += 1;
        }
    }
    let query_blocks = index.device().reads();

    // ---- the tree walk ---------------------------------------------------
    let blocks = tree_blocks();
    let mut tree = Counted::new(Memory::new(BLOCK_BYTES, blocks));
    build_tree(&mut tree).expect("a tree this device holds");
    tree.reset();
    let mut walked = 0usize;
    for which in 0..PATHS {
        let name = path(which);
        if walk(&mut tree, &name) == Ok(names(&name)) {
            walked += 1;
        }
    }
    let walk_blocks = tree.reads();

    // ---- the numbers -----------------------------------------------------
    let index_total = mount_blocks + query_blocks;
    let break_even = index_total.div_ceil(DEPTH as u64);
    println!("query: E2-B03, device blocks read per query against per tree walk");
    println!(
        "  workload            {PATHS} paths, fanout {FANOUT}, depth {DEPTH}, \
         {COMPONENT_BYTES}-byte components, {BLOCK_BYTES}-byte blocks"
    );
    println!("  index log           {log_blocks} blocks written");
    println!("  index mount         {mount_blocks} blocks read, once");
    // Hundredths and tenths, as integers: every ratio here is a count over a
    // count and is printed as one, so the report is the same on every machine
    // that ran the same workload. `lint-determinism` refuses a float here.
    let hundredths = |num: u64, den: u64| (num / den, num * 100 / den % 100);
    let (per_query, per_query_frac) = hundredths(query_blocks, PATHS as u64);
    let (per_walk, per_walk_frac) = hundredths(walk_blocks, PATHS as u64);
    // A mount that read nothing would divide by zero; the bounds below are
    // what say whether that happened, and this line only has to print.
    let advantage_tenths = walk_blocks * 10 / index_total.max(1);
    println!(
        "  index queries       {query_blocks} blocks read over {PATHS} queries \
         ({per_query}.{per_query_frac:02} per query)"
    );
    println!(
        "  tree walk           {walk_blocks} blocks read over {PATHS} walks \
         ({per_walk}.{per_walk_frac:02} per walk)"
    );
    println!(
        "  totals              index {index_total} against walk {walk_blocks} \
         ({}.{}x), break-even at {break_even} queries",
        advantage_tenths / 10,
        advantage_tenths % 10
    );
    println!("  tree on device      {blocks} blocks");

    // ---- the bounds ------------------------------------------------------
    require(
        answered == PATHS && walked == PATHS,
        &format!(
            "the two sides do not answer the same thing: index {answered}, walk {walked}, \
             of {PATHS}. A cost comparison between two things that disagree is not a \
             measurement."
        ),
    );
    require(
        query_blocks == 0,
        &format!(
            "a query read {query_blocks} device blocks; the exit is that it reads none, \
             because the answer is already in the caller's address space"
        ),
    );
    require(
        walk_blocks == (PATHS * DEPTH) as u64,
        &format!(
            "the baseline read {walk_blocks} blocks for {PATHS} walks of depth {DEPTH}, \
             not {}. A walk that read fewer is not walking, and the comparison would be \
             against nothing.",
            PATHS * DEPTH
        ),
    );
    require(
        index_total * TOTAL_ADVANTAGE_MIN <= walk_blocks,
        &format!(
            "index {index_total} blocks against walk {walk_blocks}, which is under the \
             {TOTAL_ADVANTAGE_MIN}x this design promises. A bound moves by an RFC \
             carrying a measurement and not by editing this line."
        ),
    );
    require(
        break_even <= BREAK_EVEN_MAX,
        &format!(
            "the mount cost pays for itself after {break_even} queries, against a bound of \
             {BREAK_EVEN_MAX}. `Amortised` is then a hope rather than a description."
        ),
    );

    println!("query: ok  ({PATHS} queries at 0 blocks, {PATHS} walks at {DEPTH} blocks each)");
}
