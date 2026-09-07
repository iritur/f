// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-B08`'s exit: copies per read, counted on both sides of the boundary, and
//! resident bytes per unit of work.
//!
//! # Which half of the evidence this is, said first
//!
//! The exit reads *copies per read is zero, counted rather than asserted;
//! resident bytes per unit of work recorded*. **This is the host, over a
//! modelled zoned device, and no QEMU boot has run.** The boot
//! `intent/0006-state/spec.md` describes is of `user/objects` over the blk
//! driver's ring; that driver answers two of RFC 0060's seven opcodes, this
//! crate is not yet a component for the reason `user/objects/src/lib.rs` gives
//! about the heap, and `zone/tests/cycle.rs` recorded the same gap one task
//! earlier. Saying which half is missing is worth more than a number taken
//! somewhere else and called the same thing.
//!
//! What is **not** modelled away is the thing the count is about. The
//! registration is `f_ring::registry::Table` — E1-B10's service-side table,
//! unchanged — so *was this destination one the caller registered* is answered
//! here by the same code that would answer it under a real driver. A count over
//! an inline buffer would have been the copy nobody counted, which is the
//! parenthesis `TODO.md` attaches to this task's `needs:` line.
//!
//! # The two readings, and why a green run without the controls proves nothing
//!
//! Both tallies are zero on a run that behaves, and both would be zero on a run
//! whose tallies had been deleted. So the run ends by making each of them
//! answer a question it must answer differently:
//!
//! - **The provocation.** A read is done the way a page cache would do it —
//!   into memory this component owns, then moved on into the caller's buffer —
//!   and both readings must go non-zero, by the same bytes, and must still
//!   agree. A tally that cannot move is not a tally.
//! - **The corruption.** One byte of a record on the device is flipped and the
//!   read must be refused `CONTENT`. The content hash is taken *in the caller's
//!   buffer*, which is the one thing a zero-copy read has to get right and the
//!   one thing a run that only ever reads back what it wrote never asks about.
//! - **The short run.** A read whose record does not fit in the buffers left in
//!   the set must be refused before the second block is read, not after.
//!
//! # The wiring was proved by breaking it, and here is what it printed
//!
//! A threshold nothing has ever failed is a threshold whose wiring nobody has
//! checked — `claims/0017` spent a whole task green over a bench that could not
//! reach it, and `claims/0018` was audited for the same defect one wave
//! earlier. So both halves were broken on purpose before this file was
//! committed, and put back:
//!
//! - **The threshold.** `ReadPath::read` was made to stage every record the way
//!   a page cache would. The run went red — `copies_per_read is 4096, over 0` —
//!   with the two readings moving together to **1 048 576 bytes**, which is
//!   256 reads × one 4096-byte block and is what this workload would cost if
//!   the exit's sentence were false.
//! - **The lint.** A second call to `f_objects::dma`'s one byte-mover was added
//!   inside `Landing::lend`, and `cargo xtask lint-datapath` reported both
//!   clauses: *called from `lend`, not from `provoke_copy`* and *called 2
//!   time(s)*.
//!
//! What that lint does **not** catch is worth saying in the same breath,
//! because the row is easy to over-read: it counts calls to a named function,
//! so a crate that grew a bare `copy_from_slice` on the read path would pass it
//! and fail the threshold above instead. The two checks are for two different
//! failures — a second mechanism, and a second copy — and neither substitutes
//! for the other.
//!
//! # Why a target of its own rather than a `#[test]`
//!
//! `blob/tests/million.rs`'s reason, two crates over: the exit is a
//! *measurement*, a libtest harness captures output, and the two numbers would
//! then be visible only to somebody who knew to pass `--nocapture` — which
//! makes an exit a thing the log does not contain.
//!
//! ```text
//! cargo test -p f-objects --test reads                            # the gate
//! cargo test --release -p f-objects --test reads -- --reads 4096  # a longer run
//! ```

use std::process::exit;

use f_abi::store::{Header, kind, refusal};
use f_blob::device::{Device, Memory};
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{derive, label};
use f_env::{Env, SeededEnv};
use f_hash::sha256;
use f_index::{Index, ns};
use f_objects::dma::Landing;
use f_objects::read::{ReadPath, mount};
use f_objects::resident::ONE;
use f_zone::device::ZonedMemory;
use f_zone::map::ZoneMap;

/// The name this target answers to when cargo's harness protocol asks for one.
const TEST_NAME: &str = "reads";

/// The seed the gate runs at.
///
/// A constant and not a draw: a recorded number belongs to a `(seed, commit)`
/// pair, and a run whose seed came from anywhere else is a run nobody can
/// reproduce. Unit: none — a seed.
const SEED: u64 = 0x0000_0000_e2b0_8000;

/// Reads in the unit of work when nothing says otherwise.
///
/// Unit: count of reads. `resident_bytes_per_read_byte` is a residency divided
/// by the work it was held against, so the work has to be stated with the
/// number — which is the whole of what *per unit of work* asks for.
const READS: usize = 256;

/// How many bytes each blob holds.
///
/// Unit: bytes. Sixteen thousand, chosen so that a record — this plus a
/// fifty-two-byte header — needs four whole blocks and leaves 332 bytes of
/// padding in the last one. Padding on purpose: a blob sized to a block
/// boundary would never exercise the tail, and the tail is where a reader that
/// hashed the padding along with the content would go wrong.
const BLOB_BYTES: usize = 16_000;

/// The modelled device's logical block.
///
/// Unit: bytes. Four kibibytes, which is the block `intent/0006-state/spec.md`
/// derives claim 0016's padding term at and therefore the block every number in
/// this epoch is about.
const BLOCK_BYTES: usize = 4096;

/// Blocks in one zone. Unit: count of blocks — one mebibyte a zone.
const ZONE_BLOCKS: u64 = 256;

/// The zones below the first data zone: the superblock's conventional zone and
/// the two root zones. Unit: count of zones. `zone/tests/cycle.rs`'s geometry.
const DATA_FROM: u32 = 3;

/// Buffers in the caller's registered set.
///
/// Unit: count of buffers. Eight, each one block, so a four-block record fits
/// with room left over — and so that a read starting at buffer five has
/// nowhere to put its fourth block, which is the short-run control.
const BUFFERS: u32 = 8;

/// Blocks the index's log region is given.
///
/// Unit: count of blocks. Comfortably more than the workload needs, because
/// what is recorded is what the mount *costs* and a region sized to the answer
/// would be a region that had assumed it.
const LOG_BLOCKS: u64 = 4096;

/// The index's own block. Unit: bytes — the smallest this format accepts.
const LOG_BLOCK_BYTES: usize = 512;

/// The threshold `intent/0006-state/spec.md` derives for copies per read.
///
/// Unit: bytes per read. Zero, and it is one of the two thresholds in this
/// epoch that has a natural zero — claim 0005's precedent — so it needed no
/// derivation and gets none.
const COPIES_PER_READ_MAX: u64 = 0;

/// The threshold for the system's residency per application byte.
///
/// Unit: millionths — [`ONE`] is 1.0. The spec's `resident_bytes_per_read_byte`
/// ≤ 1.0, with the sentence it is testing beside it: *a page cache holding a
/// second copy would make it at least 2*.
const RESIDENT_PER_READ_BYTE_MAX: u64 = ONE;

/// One draw site, seeded by identity from the run's seed.
///
/// Split rather than chained under RFC 0026, so that a draw site added later
/// moves neither of the ones that are here — and so that blob 40 is the same
/// blob whatever the run before it did.
fn site(seed: u64, identity: &str) -> SeededEnv {
    SeededEnv::new(derive(seed, label(identity)), 1)
}

/// `BLOB_BYTES` drawn bytes.
///
/// Uniform, which makes two blobs sharing content something that does not
/// happen: the store deduplicates by hash, and two identical blobs would be one
/// record, so a draw that collided would silently shorten the workload.
fn content(bytes: &mut SeededEnv, into: &mut Vec<u8>) {
    into.clear();
    while into.len() < BLOB_BYTES {
        into.extend_from_slice(&bytes.next_u64().to_le_bytes());
    }
    into.truncate(BLOB_BYTES);
}

/// The path blob `which` is filed under.
fn path(which: usize) -> Vec<u8> {
    format!("/objects/{which:08}").into_bytes()
}

fn fail(why: &str) -> ! {
    println!("\nreads: FAILED — {why}");
    exit(1);
}

/// A decimal rendering of a millionths figure, for the log only.
///
/// The arithmetic that produced the number is integer — `resident.rs` says why
/// — and this is the last step before a human reads it.
fn micro(value: u64) -> String {
    format!("{}.{:06}", value / ONE, value % ONE)
}

/// The cargo harness protocol, in the two clauses this target needs.
///
/// `blob/tests/million.rs` argues the split and it is the same split: a filter
/// meant for another target is handed to this one too, and a target that ran
/// its whole workload for a filter that did not select it would make
/// `cargo test <anything>` slow for no reason.
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
            "--reads" => {
                let _ = rest.next();
            }
            other if other.starts_with('-') => {}
            other if !TEST_NAME.contains(other) => {
                println!("reads: 0 tests run (filter `{other}` selected nothing here)");
                return false;
            }
            _ => {}
        }
    }
    true
}

/// How many reads this run does.
fn reads_wanted(args: &[String]) -> usize {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--reads" {
            return rest.next().and_then(|n| n.parse().ok()).unwrap_or(READS);
        }
    }
    READS
}

type Mounted = ReadPath<ZonedMemory, Memory>;

/// Write `reads` blobs, file each under a path, and answer the read path over
/// them together with the hashes in the order they were written.
fn build(reads: usize) -> (Mounted, Vec<[u8; 32]>, Vec<Vec<u8>>) {
    let blocks_per_record = (Header::BYTES + BLOB_BYTES).div_ceil(BLOCK_BYTES) as u64;
    let data_blocks = blocks_per_record * reads as u64 + 1;
    let data_zones = data_blocks.div_ceil(ZONE_BLOCKS) as u32 + 1;
    let zones = DATA_FROM + data_zones;

    let device = ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, zones);
    let map = match ZoneMap::new(device, DATA_FROM) {
        Ok(map) => map,
        Err(refusal) => fail(&format!("the mapping refused the device: {refusal:#x}")),
    };
    let layout = superblock_for_this_build(
        BLOCK_BYTES as u32,
        zones,
        ZONE_BLOCKS * BLOCK_BYTES as u64,
        1,
        2,
    );
    let mut store = match Store::format(map, &layout) {
        Ok(store) => store,
        Err(refusal) => fail(&format!("the store refused the device: {refusal:#x}")),
    };

    let log = Memory::new(LOG_BLOCK_BYTES, LOG_BLOCKS);
    let mut index = match Index::mount(log, 0, LOG_BLOCKS) {
        Ok(index) => index,
        Err(refusal) => fail(&format!("the index refused its log region: {refusal:#x}")),
    };

    let mut bytes = site(SEED, "e2-b08/content");
    let mut blob = Vec::new();
    let mut hashes = Vec::with_capacity(reads);
    let mut written = Vec::with_capacity(reads);
    for which in 0..reads {
        content(&mut bytes, &mut blob);
        let hash = match store.put(kind::CHUNK, &blob) {
            Ok(hash) => hash,
            Err(refusal) => fail(&format!("blob {which} was refused: {refusal:#x}")),
        };
        if let Err(refusal) = index.set(ns::PATH, &path(which), &hash) {
            fail(&format!("the index refused blob {which}'s name: {refusal:#x}"));
        }
        hashes.push(hash);
        written.push(blob.clone());
    }
    if let Err(refusal) = store.barrier() {
        fail(&format!("the barrier was refused: {refusal:#x}"));
    }
    if let Err(refusal) = index.barrier() {
        fail(&format!("the index's barrier was refused: {refusal:#x}"));
    }

    let mut reader = mount(store, index);
    // After the workload rather than before it: the two maps grew while it ran,
    // and a residency figure taken at construction would be a figure about an
    // empty component.
    reader.remount();
    (reader, hashes, written)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !selected(&args) {
        return;
    }
    let reads = reads_wanted(&args);
    if reads == 0 {
        fail("a run of no reads has no measurement in it");
    }

    let (mut reader, hashes, written) = build(reads);
    let mut region = vec![0u8; BUFFERS as usize * BLOCK_BYTES];
    let (mut landing, set) = match Landing::open(&mut region, BUFFERS) {
        Ok(pair) => pair,
        Err(refusal) => fail(&format!("the region was refused as a set: {refusal:#x}")),
    };

    // ---- the measured phase: nothing but reads --------------------------
    for (which, hash) in hashes.iter().enumerate() {
        let read = match reader.read(hash, &mut landing, set, 0) {
            Ok(read) => read,
            Err(refusal) => fail(&format!("read {which} was refused: {refusal:#x}")),
        };
        if read.content_at != Header::BYTES {
            fail("the content did not begin where the header ends, which is a moved record");
        }
        if read.content_bytes != BLOB_BYTES {
            fail(&format!("read {which} delivered {} bytes, not {BLOB_BYTES}", read.content_bytes));
        }
        // The caller's own bytes, read back out of the caller's own buffer —
        // which is the whole of what a zero-copy read hands over.
        let filled = match landing.contents(0, read.buffers) {
            Some(filled) => filled,
            None => fail("the run of buffers the read reported does not exist"),
        };
        let delivered = &filled[read.content_at..read.content_at + read.content_bytes];
        if delivered != written[which].as_slice() {
            fail(&format!("read {which} delivered bytes that are not the ones written"));
        }
        // Hash to zone and offset, asserted rather than assumed: a placement
        // whose zone were always the same number would be a mapping that had
        // stopped mapping, and a run of 256 records over 1 MiB zones crosses
        // several.
        if read.placement.zone < DATA_FROM {
            fail("a record was placed in a zone reserved for the superblock or a root");
        }
        if read.placement.offset_bytes != read.placement.block * BLOCK_BYTES as u64 {
            fail("the offset and the block disagree");
        }
    }

    let counters = *reader.counters();
    let landed = *landing.landed();
    let resident = *reader.resident();
    let copies_per_read = counters.staged_bytes / counters.reads.max(1);
    let zones_touched = {
        let mut seen = std::collections::BTreeSet::new();
        for hash in &hashes {
            if let Some(placement) = reader.locate(hash) {
                seen.insert(placement.zone);
            }
        }
        seen.len()
    };

    println!("reads: E2-B08, the read path — seed {SEED:#018x}");
    println!("\n  workload");
    println!("    reads                              {reads}");
    println!("    blob                               {BLOB_BYTES} bytes");
    println!("    block                              {BLOCK_BYTES} bytes");
    println!("    registered set                     {BUFFERS} buffers of one block");
    println!("    zones the records landed in        {zones_touched}");
    println!("    application bytes delivered        {}", counters.content_bytes);
    println!("    blocks the driver submitted        {}", counters.blocks);

    println!("\n  copies per read — two readings of one event");
    println!("    driver: bytes to an unregistered destination     {}", counters.staged_bytes);
    println!("    device: bytes through an unregistered buffer     {}", landed.unregistered_bytes);
    println!("    device: bytes straight into the caller's buffer  {}", landed.registered_bytes);
    println!("    device: transfers into the caller's buffer       {}", landed.transfers);
    println!("    copies_per_read                                  {copies_per_read}");

    let held = resident.held_per_read_byte_micro().unwrap_or(0);
    let system = resident.system_per_read_byte_micro().unwrap_or(0);
    let mounted = resident.mount_per_read_byte_micro().unwrap_or(0);
    println!(
        "\n  resident bytes per unit of work — {reads} reads, {} bytes",
        resident.delivered_bytes()
    );
    println!(
        "    component held (peak)                            {} bytes",
        resident.held_bytes()
    );
    println!(
        "    mount payload (index map + store map)            {} bytes",
        resident.mount_bytes()
    );
    println!("    held_bytes_per_read_byte                         {}", micro(held));
    println!("    resident_bytes_per_read_byte (system)            {}", micro(system));
    println!("    mount_bytes_per_read_byte                        {}", micro(mounted));

    // ---- the thresholds -------------------------------------------------
    if counters.reads != reads as u64 {
        fail(&format!("{} reads completed, not {reads}", counters.reads));
    }
    if copies_per_read > COPIES_PER_READ_MAX {
        fail(&format!("copies_per_read is {copies_per_read}, over {COPIES_PER_READ_MAX}"));
    }
    if !reader.readings_agree(&landing) {
        fail("the driver's reading and the device model's do not agree");
    }
    if landed.registered_bytes != counters.blocks * BLOCK_BYTES as u64 {
        fail("the device landed a different number of bytes than the driver submitted blocks");
    }
    if landed.transfers != counters.blocks {
        fail("the device completed a different number of transfers than the driver submitted");
    }
    if system > RESIDENT_PER_READ_BYTE_MAX {
        fail(&format!(
            "resident_bytes_per_read_byte is {}, over {}",
            micro(system),
            micro(RESIDENT_PER_READ_BYTE_MAX)
        ));
    }
    if zones_touched < 2 {
        fail("every record landed in one zone, so the mapping was never asked a question");
    }

    // ---- the controls ---------------------------------------------------
    println!("\n  controls");

    // A read that cannot fit. Refused before the second block is read.
    let last = BUFFERS - 1;
    match reader.read(&hashes[0], &mut landing, set, last) {
        Err(code) if code == refusal::SHORT_BUFFER => {
            println!("    a record with nowhere to land       refused SHORT_BUFFER");
        }
        Err(code) => fail(&format!("a short run was refused {code:#x}, not SHORT_BUFFER")),
        Ok(_) => fail("a record was read into buffers the set does not have"),
    }

    // A byte flipped underneath the format. The hash is taken in the caller's
    // buffer, so this is the one control that says that check runs.
    let placement = match reader.locate(&hashes[0]) {
        Some(placement) => placement,
        None => fail("the first blob has no placement"),
    };
    // The first content byte of the record, which is inside the block the
    // placement names. A byte in the *second* block would be a byte at a
    // physical address this run has assumed rather than asked for: a record's
    // blocks are consecutive logically and the mapping is free to place them
    // anywhere, so the only offset a placement licenses is its own.
    let at = placement.block as usize * BLOCK_BYTES + Header::BYTES;
    if let Err(code) = reader.store_mut().device_mut().device_mut().flip(at) {
        fail(&format!("the device refused a flip at {at}: {code:#x}"));
    }
    match reader.read(&hashes[0], &mut landing, set, 0) {
        Err(code) if code == refusal::CONTENT => {
            println!("    a byte flipped in a record          refused CONTENT");
        }
        Err(code) => fail(&format!("a corrupted record was refused {code:#x}, not CONTENT")),
        Ok(_) => fail("a corrupted record was accepted, so the content hash is not checked"),
    }
    if let Err(code) = reader.store_mut().device_mut().device_mut().flip(at) {
        fail(&format!("the device refused the repair at {at}: {code:#x}"));
    }

    // The provocation. Both readings must move, by the same bytes, and must
    // still agree — a tally that cannot move is not a tally, and a zero
    // recorded above is worth exactly what this paragraph is worth.
    let before_driver = reader.counters().staged_bytes;
    let before_device = landing.landed().unregistered_bytes;
    let moved = match reader.provoke_second_copy(&hashes[0], &mut landing, 0) {
        Ok(moved) => moved,
        Err(code) => fail(&format!("the provocation was refused: {code:#x}")),
    };
    let after_driver = reader.counters().staged_bytes;
    let after_device = landing.landed().unregistered_bytes;
    if after_driver != before_driver + moved {
        fail("the driver's reading did not move by what the provocation moved");
    }
    if after_device != before_device + moved {
        fail("the device model's reading did not move by what the provocation moved");
    }
    if !reader.readings_agree(&landing) {
        fail("the two readings disagree after the provocation");
    }
    if after_driver == 0 {
        fail("the provocation moved nothing, so the zero above is a default and not a count");
    }
    println!("    the page cache, provoked            both readings {after_driver} bytes");

    // And the residency the provocation costs, which is the other half of the
    // same argument: the number above was zero because nothing was held, not
    // because nothing could be.
    let provoked = reader.resident().system_per_read_byte_micro().unwrap_or(0);
    if provoked <= system {
        fail("the residency figure did not move when this component held a block");
    }
    println!(
        "    resident_bytes_per_read_byte then    {} (was {})",
        micro(provoked),
        micro(system)
    );

    // The record the provocation moved is the record it read, so the caller
    // ends up with the same bytes by the expensive route. A control that had
    // moved the wrong bytes would be a control that proved the tally moves and
    // nothing about what it counts.
    let cached = match landing.buffer(0) {
        Some(cached) => cached,
        None => fail("buffer zero is not in the set"),
    };
    let mut first_block = vec![0u8; BLOCK_BYTES];
    if let Err(code) = reader.store_mut().device_mut().read(placement.logical, &mut first_block) {
        fail(&format!("re-reading the first block was refused: {code:#x}"));
    }
    if cached != first_block.as_slice() {
        fail("the provocation delivered bytes that are not the record's");
    }
    // And the record still hashes to its name, which says the flip was repaired
    // rather than merely un-noticed.
    if sha256(&written[0]) != hashes[0] {
        fail("the workload's own bookkeeping does not agree with the store's names");
    }

    println!("\nreads: ok — copies_per_read {copies_per_read}, both readings agreeing at zero;");
    println!("       resident_bytes_per_read_byte {} over {reads} reads", micro(system));
}
