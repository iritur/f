// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-B01`'s first exit: write, read back and verify a million blobs.
//!
//! # Why this is a target of its own rather than a `#[test]`
//!
//! Because the exit and the per-commit gate differ by a *number* and by nothing
//! else. A million blobs into a `Memory` device is half a gigabyte of host
//! memory and a minute of hashing; ten thousand is five megabytes and a second,
//! and that is what every `cargo xtask test` can afford to pay. A libtest
//! harness would swallow the argument that carries the difference, and the two
//! runs would then have to differ by a build — a `cfg`, a feature, a second
//! file — which is how a gate and an exit come to be testing two different
//! programs. `ring/tests/hostile.rs` made this argument first and RFC 0046
//! records it; this is the same shape, one crate over.
//!
//! ```text
//! cargo test -p f-blob --test million                      # the gate: 10 000
//! cargo test --release -p f-blob --test million -- --blobs 1000000   # the exit
//! ```
//!
//! # The control, and why the run is worthless without it
//!
//! A million blobs written and read back is green whether the content hash is
//! checked or thrown away. Every byte comes out of the same memory it went
//! into, so the verifier is never asked a question it could answer wrongly —
//! and **a verifier that has never failed is indistinguishable from one that
//! cannot fail.** So the run ends by flipping one bit inside one stored blob's
//! *content* and reading every blob again: exactly one read must be refused,
//! and it must be refused with `f_abi::store`'s `refusal::CONTENT` rather than
//! with anything else that also happens to be an error. The bit is flipped in
//! the content and not at a drawn offset in the device, because a bit flipped
//! into a record's padding is refused by nothing and the control would pass
//! while proving nothing.
//!
//! # Where the content comes from
//!
//! From a seeded `f_env::Env`, split by identity under RFC 0026: one `Env` for
//! the lengths, one for the bytes, one for the choice of which blob to corrupt.
//! Split rather than chained, so that a fourth draw site added later moves none
//! of the three that are here — and so that the blob at index 700 000 is the
//! same blob whatever the run before it did.
//!
//! The content is uniform random and at least [`CONTENT_MIN_BYTES`] long, which
//! is a deliberate choice and not an arbitrary floor: it makes two drawn blobs
//! being byte-identical impossible in practice, so `store.records()` is
//! required to equal the blob count. Without that the run would silently accept
//! a store that deduplicated everything into one record and read it back a
//! million times.

use std::process::exit;

use f_abi::store::{Header, kind, refusal};
use f_blob::device::Memory;
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{derive, label};
use f_env::{Env, SeededEnv};

/// The name this target answers to when cargo's harness protocol asks for one.
///
/// `harness = false` means the binary *is* the test, so there is exactly one
/// and this is what it is called. It matters because a filter — `cargo test -p
/// f-blob chunker`, or anything nextest does — is handed to **every** test
/// target in the package, including the ones the filter is not about.
const TEST_NAME: &str = "million";

/// Blobs written when nothing says otherwise. Unit: count of blobs.
///
/// Ten thousand, which is what `cargo test --workspace` pays on every commit.
/// The exit is `--blobs 1000000` in release, and the two differ by this number
/// alone.
const DEFAULT_BLOBS: u64 = 10_000;

/// The seed the three draw sites derive from when nothing says otherwise.
/// Unit: none — a seed.
const DEFAULT_SEED: u64 = 1;

/// The modelled device's logical block. Unit: bytes.
///
/// The smallest a real device reports, and the smallest this format will accept
/// — every record fits in one block at this size, which is what lets a record's
/// magic be disbelieved after a single read.
const BLOCK_BYTES: usize = 512;

/// The shortest blob this run draws. Unit: bytes.
///
/// Thirty-two, so that two drawn blobs colliding byte for byte is not a thing
/// that happens: the run asserts that a blob count equals a record count, and a
/// short draw would make that assertion fail for a reason that is not a defect.
const CONTENT_MIN_BYTES: usize = 32;

/// The longest ordinary blob this run draws. Unit: bytes.
///
/// Four hundred, so that a header and its content fit in one block: a million
/// blobs is then a million blocks, which is the half-gigabyte the exit was
/// costed at rather than an unbounded multiple of it.
const CONTENT_MAX_BYTES: usize = 400;

/// One blob in this many is drawn long enough to span several blocks.
/// Unit: count of blobs.
///
/// A store whose every record is one block would never exercise the loop that
/// reads a record's second and later blocks, and that loop is where an
/// off-by-one costs a byte at a block boundary — which is exactly the defect
/// the content hash exists to catch and exactly the one a single-block workload
/// would never reach.
const LONG_EVERY: u64 = 1024;

/// The longest of those. Unit: bytes.
const LONG_MAX_BYTES: usize = 4096;

/// Zones the modelled device declares, and their size.
///
/// Recorded in the superblock and used by nothing here: this crate cannot see a
/// zone, and `f-zone` at `E2-B02` is what will. They are named rather than left
/// zero so that the superblock this run writes is the superblock a real format
/// writes, field for field.
/// Unit: count of zones.
const ZONES: u32 = 64;

/// Unit: bytes.
const ZONE_BYTES: u64 = 1 << 20;

/// What one invocation was asked for.
struct Asked {
    /// Unit: count of blobs.
    blobs: u64,
    /// Unit: none — a seed.
    seed: u64,
    /// Answer the harness protocol's `--list` and run nothing.
    list: bool,
    /// A name filter that selected nothing. Carried rather than refused, so
    /// [`main`] can answer it the way a harness does.
    unselected: Option<String>,
}

/// Parse the command line.
///
/// This run's own options fail closed — a misspelled `--blobs` that quietly ran
/// the default would make a green line mean the gate when somebody asked for
/// the exit. Cargo's harness protocol does not: `cargo test` hands every target
/// in a package the same arguments, so a bare name filter aimed at another
/// target selects nothing here and exits zero, `--list` names the one test, and
/// the flags libtest owns are accepted and ignored. `ring/tests/hostile.rs`
/// argues this split at length and it is the same split.
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut blobs = DEFAULT_BLOBS;
    let mut seed = DEFAULT_SEED;
    let mut list = false;
    let mut filter: Option<String> = None;

    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        let mut value = || walk.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--blobs" => blobs = number(&value()?)?,
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
        return Ok(Asked { blobs, seed, list: false, unselected: Some(name) });
    }
    if blobs == 0 {
        return Err("--blobs 0 asks for a run that writes nothing, which is green because \
                    it asserted nothing. R04."
            .to_string());
    }
    Ok(Asked { blobs, seed, list, unselected: None })
}

/// A decimal or `0x`-prefixed hexadecimal number.
fn number(text: &str) -> Result<u64, String> {
    text.strip_prefix("0x")
        .map_or_else(|| text.parse::<u64>().ok(), |hex| u64::from_str_radix(hex, 16).ok())
        .ok_or_else(|| format!("`{text}` is not a number"))
}

/// One draw site, seeded by identity from the run's seed.
///
/// A `SeededEnv` and not a bare `Stream`, because what this file draws is test
/// *data* and the substrate that hands out test data is `Env` — the same one a
/// simulated run would use. Nothing here reads its clock; the tick is a
/// nanosecond because an `Env` has one and this is the smallest honest value
/// for a run that never asks what time it is.
fn site(seed: u64, identity: &str) -> SeededEnv {
    SeededEnv::new(derive(seed, label(identity)), 1)
}

/// A value in `0..n`, by remainder.
///
/// The bias is at most one part in `u64::MAX / n`, which is smaller than
/// anything this run measures. What the draw has to be is reproducible from the
/// seed, and it is.
fn below(env: &mut SeededEnv, n: usize) -> usize {
    (env.next_u64() % n as u64) as usize
}

/// How long blob `index` is. Unit: bytes.
///
/// The length is drawn from its own site rather than from the content site, so
/// that the bytes a blob holds do not depend on how long the blob before it
/// was. That independence is what makes a failure reproducible by index.
fn length(lengths: &mut SeededEnv, index: u64) -> usize {
    if index.is_multiple_of(LONG_EVERY) {
        CONTENT_MIN_BYTES + below(lengths, LONG_MAX_BYTES - CONTENT_MIN_BYTES)
    } else {
        CONTENT_MIN_BYTES + below(lengths, CONTENT_MAX_BYTES - CONTENT_MIN_BYTES)
    }
}

/// `len` drawn bytes.
fn content(bytes: &mut SeededEnv, len: usize, into: &mut Vec<u8>) {
    into.clear();
    while into.len() < len {
        into.extend_from_slice(&bytes.next_u64().to_le_bytes());
    }
    into.truncate(len);
}

/// How many blocks a record of `len` content bytes occupies.
/// Unit: count of blocks.
fn blocks_for(len: usize) -> u64 {
    (Header::BYTES + len).div_ceil(BLOCK_BYTES) as u64
}

/// One run, and the status it earns.
fn run(asked: &Asked) -> i32 {
    let blobs = asked.blobs;
    let seed = asked.seed;
    println!("million — {blobs} blob(s) into a modelled device, from seed {seed:#018x}");

    // The device is sized from the draw rather than guessed at, which costs one
    // pass over the length site and buys a device with no slack in it: a run
    // that failed with `FULL` because the estimate was short would be a failure
    // of this file rather than of the store.
    let mut lengths = site(seed, "e2-b01/length");
    let mut blocks = 1;
    for index in 0..blobs {
        blocks += blocks_for(length(&mut lengths, index));
    }
    let device = Memory::new(BLOCK_BYTES, blocks);
    let layout = superblock_for_this_build(BLOCK_BYTES as u32, ZONES, ZONE_BYTES, 1, 2);
    let mut store = match Store::format(device, &layout) {
        Ok(store) => store,
        Err(why) => {
            println!("\nfinding 1  the device could not be formatted: {why}");
            return 1;
        }
    };

    // Which blob the control corrupts, drawn before the write pass so that the
    // pass can keep that one blob's name as it goes rather than redrawing the
    // whole run to find it again.
    let mut target = site(seed, "e2-b01/target");
    let corrupt = target.next_u64() % blobs;

    let mut lengths = site(seed, "e2-b01/length");
    let mut bytes = site(seed, "e2-b01/content");
    let mut buffer: Vec<u8> = Vec::with_capacity(LONG_MAX_BYTES);
    let mut corrupted: Option<([u8; 32], usize)> = None;
    let mut content_bytes: u64 = 0;

    for index in 0..blobs {
        let len = length(&mut lengths, index);
        content(&mut bytes, len, &mut buffer);
        let hash = match store.put(kind::CHUNK, &buffer) {
            Ok(hash) => hash,
            Err(why) => {
                println!("\nfinding 1  blob {index} of {len} byte(s) was refused: {why}");
                return 1;
            }
        };
        content_bytes += len as u64;
        if index == corrupt {
            corrupted = Some((hash, len));
        }
    }

    // The first half of RFC 0060's publish sequence ends here. Nothing appends a
    // root record — that is `f-zone`'s — but the barrier is what the store owes
    // and it is counted rather than assumed.
    if let Err(why) = store.barrier() {
        println!("\nfinding 1  the barrier was refused: {why}");
        return 1;
    }

    let mut problems: Vec<String> = Vec::new();
    if store.records() as u64 != blobs {
        problems.push(format!(
            "the store holds {} record(s) for {blobs} drawn blob(s). Uniform content of at \
             least {CONTENT_MIN_BYTES} bytes does not collide, so either the draw is not \
             what this file thinks it is or two distinct blobs shared a record",
            store.records()
        ));
    }

    let read = read_every_blob(&mut store, seed, blobs);
    if read.refusals > 0 {
        problems.push(format!(
            "{} of {blobs} blob(s) did not read back: first at index {:?} with {:?}",
            read.refusals, read.first_refused, read.first_refusal
        ));
    }
    if read.mismatches > 0 {
        problems.push(format!(
            "{} blob(s) read back bytes that are not the bytes written, first at index {:?}. \
             The content hash verified, so this is the reassembly and not the device",
            read.mismatches, read.first_mismatch
        ));
    }

    // The control. Everything above is green on a store that never checks a
    // content hash; this is the part that is not.
    let Some((hash, len)) = corrupted else {
        problems.push(format!("blob {corrupt} was never written, so nothing was corrupted"));
        report(&problems, blobs, blocks, content_bytes, &store, &read, None);
        return 1;
    };
    let Some(block) = store.address(&hash) else {
        problems.push(format!("blob {corrupt} has no address, so nothing could be flipped"));
        report(&problems, blobs, blocks, content_bytes, &store, &read, None);
        return 1;
    };
    let inside = below(&mut target, len);
    let at = block as usize * BLOCK_BYTES + Header::BYTES + inside;
    if let Err(why) = store.device_mut().flip(at) {
        problems.push(format!("the control could not flip byte {at}: {why}"));
        report(&problems, blobs, blocks, content_bytes, &store, &read, None);
        return 1;
    }

    let after = read_every_blob(&mut store, seed, blobs);
    if after.refusals != 1 {
        problems.push(format!(
            "one bit was flipped inside blob {corrupt}'s content and {} read(s) were refused. \
             Exactly one is what a working verifier gives; zero means the content hash is not \
             checked at all, and more than one means a blob's bytes are not its own",
            after.refusals
        ));
    }
    if after.first_refusal != Some(refusal::CONTENT) {
        problems.push(format!(
            "the corrupted blob was refused with {:?} and not with refusal::CONTENT ({}). A \
             refusal for the wrong reason is a verifier that is right by accident",
            after.first_refusal,
            refusal::CONTENT
        ));
    }
    if after.first_refused != Some(corrupt) {
        problems.push(format!(
            "blob {corrupt} was corrupted and blob {:?} was refused",
            after.first_refused
        ));
    }
    if after.mismatches > 0 {
        problems.push(format!(
            "{} blob(s) returned bytes that are not their own *without* being refused, which \
             is the verifier missing a corruption rather than reporting one",
            after.mismatches
        ));
    }

    report(&problems, blobs, blocks, content_bytes, &store, &read, Some(&after));
    if problems.is_empty() {
        println!("\nfindings   none");
        return 0;
    }
    for (index, problem) in problems.iter().enumerate() {
        println!("\nfinding {}  {problem}", index + 1);
    }
    println!(
        "  repro      cargo test --release -p f-blob --test million -- \
         --seed {seed:#018x} --blobs {blobs}"
    );
    1
}

/// What one read pass saw.
///
/// Counts and not a `Result`, because the run has to keep going after the first
/// refusal to answer the question the control asks: *how many* reads were
/// refused, not whether any were.
#[derive(Default)]
struct Pass {
    /// Unit: count of blobs read back byte for byte.
    verified: u64,
    /// Unit: count of reads refused.
    refusals: u64,
    /// Unit: count of reads that returned the wrong bytes without a refusal.
    mismatches: u64,
    first_refused: Option<u64>,
    first_refusal: Option<i32>,
    first_mismatch: Option<u64>,
}

/// Read every blob back by its hash and compare it against the content the seed
/// draws.
///
/// The content is *redrawn* rather than kept from the write pass, which is what
/// makes the exit affordable: a million blobs held in memory beside the device
/// would double the run's cost to hold a copy of what a seed already reproduces
/// exactly.
fn read_every_blob(store: &mut Store<Memory>, seed: u64, blobs: u64) -> Pass {
    let mut lengths = site(seed, "e2-b01/length");
    let mut bytes = site(seed, "e2-b01/content");
    let mut drawn: Vec<u8> = Vec::with_capacity(LONG_MAX_BYTES);
    let mut read = vec![0u8; LONG_MAX_BYTES];
    let mut pass = Pass::default();

    for index in 0..blobs {
        let len = length(&mut lengths, index);
        content(&mut bytes, len, &mut drawn);
        let hash = f_hash::sha256(&drawn);
        match store.get(&hash, &mut read[..len]) {
            Ok(moved) if moved == len && read[..len] == drawn[..] => pass.verified += 1,
            Ok(_) => {
                pass.mismatches += 1;
                pass.first_mismatch.get_or_insert(index);
            }
            Err(why) => {
                pass.refusals += 1;
                pass.first_refused.get_or_insert(index);
                pass.first_refusal.get_or_insert(why);
            }
        }
    }
    pass
}

/// Print what the run did.
///
/// Every line is a count and none of them is a duration: this binary reads no
/// clock, and how long a run took belongs beside the report rather than inside
/// it — the same split `ring/tests/hostile.rs` keeps and RFC 0040 argues.
fn report(
    problems: &[String],
    blobs: u64,
    blocks: u64,
    content_bytes: u64,
    store: &Store<Memory>,
    read: &Pass,
    after: Option<&Pass>,
) {
    let line = |name: &str, value: u64| println!("  {name:<24}{value:>14}");

    println!("\nthe device");
    line("block bytes", BLOCK_BYTES as u64);
    line("blocks", blocks);
    line("blocks written", store.device().writes());
    line("barriers", store.device().flushes());

    println!("\nthe store");
    line("blobs written", blobs);
    line("records", store.records() as u64);
    line("content bytes", content_bytes);
    line("first free block", store.free());

    println!("\nread back");
    line("verified", read.verified);
    line("refused", read.refusals);
    line("wrong bytes, no refusal", read.mismatches);

    if let Some(after) = after {
        println!("\nafter one bit was flipped");
        line("verified", after.verified);
        line("refused", after.refusals);
        line("wrong bytes, no refusal", after.mismatches);
    }

    println!("\nproblems {}", problems.len());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = match parse(&args) {
        Ok(asked) => asked,
        Err(why) => {
            eprintln!(
                "million: {why}\n\n\
                 usage: [--blobs <n>] [--seed <n>]\n\
                 \x20      cargo's own harness arguments — a name filter, --list,\n\
                 \x20      --nocapture and the rest of libtest's flags — are answered\n\
                 \x20      rather than refused."
            );
            exit(2);
        }
    };
    if asked.list {
        println!("{TEST_NAME}: test");
        println!();
        println!("1 test, 0 benchmarks");
        return;
    }
    if let Some(name) = &asked.unselected {
        println!(
            "million — 0 blob(s): the filter `{name}` selects no test in this target, so this\n\
             \x20         run wrote none. That is a harness answering a filter and it asserts\n\
             \x20         nothing about the store."
        );
        return;
    }
    exit(run(&asked));
}
