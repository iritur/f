// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The loop: entries off the objects ring, content into the client's own
//! memory, and the count taken where the client asked.
//!
//! # What this is, in one sentence
//!
//! It is the four lines [`crate::service`]'s comment promised and the shape
//! `user/virtio-blk/src/component.rs` already had — pop, answer, post — with the
//! store underneath it in this component's own heap rather than on a device.
//!
//! # The three things this component is handed and could not invent
//!
//! **Where its rings are.** `crate::routing`, read off the page the frame fills
//! in before the first instruction. A component that held ring addresses as
//! constants would be a component that works until the frame's layout moves.
//!
//! **Where its client's memory is.** The region at `routing::at::BUFFERS_AT`,
//! which the frame granted and put in no remapping domain, reached through
//! `f_ring::Granted` and registered through `f_ring::registry::Table`. **This is
//! the memory `E2-B08` is about.** The content a read delivers lands here and
//! nowhere else, `Served::staged_bytes` is what says so, and the client reads
//! the bytes back out of its own side afterwards — so the zero is checked by the
//! side that would be harmed if it were wrong.
//!
//! **What to serve.** There is no device under this component, so it writes the
//! blob it is going to be asked for, into a store in its own heap, from a seed
//! the frame put on the board. The frame fills the same bytes from the same seed
//! and compares. That is what makes the read a read of something the client
//! asked for: two independent computations of one content, meeting in the middle
//! at a hash neither side chose.
//!
//! # What that does **not** demonstrate, said before anybody reads it as more
//!
//! A device. `intent/0006-state/spec.md` describes a boot of this component over
//! the blk driver's ring, and the store here is `f_zone::device::ZonedMemory` in
//! this component's heap — the same model `user/objects/tests/reads.rs` uses,
//! moved across a privilege boundary. What crosses a real ring in a real boot is
//! the **entry and the content**; what is still modelled is the device under the
//! store. `claims/0022` says which of its rows come from which.
//!
//! # R05, and why the control ring is drained first
//!
//! Because a stop is the one thing that ends this loop, and an entry taken after
//! it would be work done for a client the frame has already said is gone. Every
//! notice arrives at this one polling point; there is no second path in, and a
//! kind this build cannot name ends the run rather than being skipped.

use f_abi::control::{is_notice, notice};
use f_abi::objects::PAYLOAD_BYTES;
use f_abi::store::kind;
use f_abi::{Cqe, door, feature};
use f_blob::device::Memory;
use f_blob::store::{Store, superblock_for_this_build};
use f_index::Index;
use f_ring::adopt::{Adopted, Client, Server};
use f_ring::{Granted, Window};
use f_zone::device::ZonedMemory;
use f_zone::map::ZoneMap;

use crate::dma::Landing;
use crate::read::mount;
use crate::service::Service;
use f_abi::objects::board::{self as routing, PROVOKE, at, reported, stopped};

/// How many entries to take off the ring before answering.
///
/// One, and it is a decision rather than a simplification. `user/virtio-blk`
/// drains into a queue and then chooses an order, because a device has a depth
/// and reordering is what `E1-B06` is about. This component has no device and no
/// depth: a read is answered synchronously out of memory, so a queue here would
/// be a queue with nothing to reorder and a claim about scheduling that nothing
/// underneath it supports.
///
/// *What moves it:* the day the store is reached over the blk driver's ring. A
/// read then has a submission and a completion under it, several can be in
/// flight, and the order they are handed down in becomes a decision this loop
/// has to make. `E2-B02` is where that arrives.
/// Unit: entries.
const AT_A_TIME: u32 = 1;

/// Zones below the first data zone: the superblock's and the two root zones.
///
/// `zone/tests/cycle.rs`'s geometry, unchanged, because a component that used a
/// different one would be exercising a mapping no test covers.
/// Unit: count of zones.
const DATA_FROM: u32 = 3;

/// Blocks in one zone, for the store this component builds in its heap.
///
/// **Four, and the number is a heap bound rather than a storage decision.** The
/// device is `ZONE_BLOCKS * zones` blocks of whatever the board says a block is,
/// allocated out of the region the frame granted, and `ZonedMemory::new` builds
/// it with `vec![]` — which *aborts* when the allocator cannot answer rather
/// than returning `None`. So a geometry that does not fit is not a refusal with
/// a reason in `reported::OUTCOME`; it is a component that stops making progress
/// and a client that times out saying `the component published no registration
/// in time`, which is what sixteen blocks of 2048 across five zones produced on
/// the first run of this file — 160 KiB asked of a 64 KiB heap.
///
/// *What moves it:* `ZonedMemory` growing a fallible constructor, at which point
/// this can be sized for the workload and a heap that cannot hold it becomes
/// `NO_STORE` in the log.
/// Unit: count of blocks.
const ZONE_BLOCKS: u64 = 4;

/// Blocks the index's log region is given.
///
/// Sixteen, for [`ZONE_BLOCKS`]' reason and with the same failure mode: this is
/// `LOG_BLOCK_BYTES` times this many, out of the same heap.
/// Unit: count of blocks.
const LOG_BLOCKS: u64 = 16;

/// The index's own block. Unit: bytes — the smallest the format accepts.
const LOG_BLOCK_BYTES: usize = 512;

/// Everything the board said, in the types that use it.
struct Parts {
    control: Client,
    data: Server,
    buffers: Granted,
    /// One buffer's width, which is also the store's block. Unit: bytes.
    block_bytes: usize,
    /// How many buffers the client's region holds. Unit: count of buffers.
    buffers_count: u32,
    /// How much content to put in the blob. Unit: bytes.
    blob_bytes: usize,
    /// What to fill it with. Unit: none — a seed.
    seed: u64,
}

/// Serve the objects ring until the frame says stop.
///
/// `selector` is `board::SERVE` or `board::PROVOKE`; nothing else reaches here,
/// because `component::start` calls this for those two alone. The provocation is
/// a whole life rather than a flag on an entry, which is `user/virtio-blk`'s
/// reason: a grep for the second call site below finds every way this component
/// can stage a byte, and there are exactly two.
///
/// Every failure ends the run with a reason in [`reported::OUTCOME`] rather than
/// a panic, and the reason matters: a component that stopped because its board
/// was blank and one that was told to stop look identical from outside, and only
/// one of them is the run the boot asked for.
pub fn serve(selector: u32) -> ! {
    let Ok(board) = Window::at(routing::AT, routing::BYTES) else {
        // Nothing to report *into*, so the status word is all there is.
        end(stopped::BAD_ROUTING)
    };
    // R04 at the one place this component reads a structure it did not build. A
    // page of zeroes is what a frame that was mapped and never filled in looks
    // like, and a zero length taken for a length reads as a client problem
    // rather than as a frame that did not speak.
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        report(&board, None, stopped::NO_ROUTING);
        end(stopped::NO_ROUTING)
    }

    let Some(parts) = laid_out(&board) else {
        report(&board, None, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    };
    // Destructured rather than borrowed, because the client's region has to be
    // borrowed mutably for the length of the run and a `Parts` holding it would
    // then be borrowed for the length of the run as well — which is the borrow
    // graph `f_ring::Adopted` records avoiding, arrived at from the other side.
    let Parts { control, data, mut buffers, block_bytes, buffers_count, blob_bytes, seed } = parts;

    // The client's memory, registered. `Landing::open` takes the bytes
    // `Granted::bytes_mut` answers and there is nothing in between — RFC 0090
    // is the decision that made that sentence possible, and the fact that the
    // host test's `Landing` over a `Vec` and this one over a granted region are
    // the same type is what keeps one tally for two paths.
    let Ok((mut landing, set)) = Landing::open(buffers.bytes_mut(), buffers_count) else {
        report(&board, None, stopped::NO_REGISTRATION);
        end(stopped::NO_REGISTRATION)
    };
    if landing.stride() != block_bytes {
        // The one geometry check this component makes for itself, because
        // `ReadPath::read` refuses a set whose stride is not the block and a
        // refusal there would read as a defect in the read path rather than as
        // a board the frame filled in wrongly.
        report(&board, None, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    }

    // What this component will be asked for. Built here rather than handed over,
    // because there is no device: the store is in this component's own heap and
    // the blob is written into it from the seed the frame chose.
    let Some((mut service, hash)) = stocked(block_bytes, blob_bytes, seed) else {
        report(&board, None, stopped::NO_STORE);
        end(stopped::NO_STORE)
    };

    // What the client needs before it can name anything: which set was issued,
    // how wide a buffer is, how long the content is, and the hash to ask for.
    // Written before the loop and before the magic, so a client that reads the
    // magic has all four.
    let _ = board.write64(reported::SET, u64::from(set.bits()));
    let _ = board.write64(reported::BUFFERS, u64::from(buffers_count));
    let _ = board.write64(reported::STRIDE, block_bytes as u64);
    let _ = board.write64(reported::CONTENT_BYTES, blob_bytes as u64);
    for (word, chunk) in hash.chunks_exact(8).enumerate() {
        let mut eight = [0u8; 8];
        eight.copy_from_slice(chunk);
        let _ = board.write64(reported::HASH + (word as u32) * 8, u64::from_le_bytes(eight));
    }
    // The magic for the half the client reads *before* the run, so that the
    // client knows the set id is a set id and not a zero. The counts below are
    // written again at the end, over the same words, and the client reads them
    // after the join rather than during it.
    let _ = board.write64(reported::MAGIC, routing::REPORTED_MAGIC);

    let mut notices: u64 = 0;
    let outcome = loop {
        // The control ring first. R05, and the reason is in the module comment.
        match drain(&control, &mut notices) {
            Ok(true) => break stopped::TOLD,
            Ok(false) => {}
            Err(why) => break why,
        }

        let mut served = 0;
        while served < AT_A_TIME {
            let taken = match data.pop() {
                Ok(taken) => taken,
                Err(_) => break,
            };
            let Some(entry) = taken else { break };
            // The payload, out of the channel's inline arena at the offset the
            // entry named. Copied into this component's own stack and not
            // borrowed: forty bytes a peer wrote, which every field of
            // `Request::decode` is then free to disbelieve.
            let mut payload = [0u8; PAYLOAD_BYTES];
            if !arena_copy(&data, &entry, &mut payload) {
                // An entry whose payload is not inside the arena. Answered
                // rather than dropped — a request that vanished is a client
                // waiting forever, and a service may not do that quietly —
                // and answered by the same decoder, which refuses it for the
                // framing rather than for a reason this loop invented.
                payload = [0u8; PAYLOAD_BYTES];
            }
            // Zero, and it is a literal for `user/virtio-blk`'s reason: this
            // crate observes no clock, RFC 0004, and the determinism lint would
            // refuse a call to one. What a completion's timestamp would mean
            // here is the frame's business and the frame is the client.
            // Two call sites, and the `if` is what makes them two: the data
            // path is the first and the provocation is the second. A grep for
            // `provoke_staged_answer` finds every way this component moves a
            // byte through a buffer its client did not register.
            let answer = if selector == PROVOKE {
                service.provoke_staged_answer(&entry, &payload, &mut landing, 0)
            } else {
                service.answer(&entry, &payload, &mut landing, 0)
            };
            if data.post(answer).is_err() {
                break;
            }
            served += 1;
        }
        if served == 0 {
            core::hint::spin_loop();
        }
    };

    report(&board, Some((&service, notices)), outcome);
    end(outcome)
}

/// Read the board and state everything it names.
///
/// `None` for any address that cannot be stated as a window, a region or a
/// channel — which is a frame that filled this page in wrongly, and is refused
/// here rather than dereferenced to find out.
fn laid_out(board: &Window) -> Option<Parts> {
    let control = Adopted::at(
        board.read64(at::CONTROL_AT).ok()?,
        u32::try_from(board.read64(at::CONTROL_LEN).ok()?).ok()?,
        feature::CONTROL_EVENTS,
        feature::CONTROL_EVENTS,
    )
    .ok()?
    .client();

    let data = Adopted::at(
        board.read64(at::DATA_AT).ok()?,
        u32::try_from(board.read64(at::DATA_LEN).ok()?).ok()?,
        0,
        0,
    )
    .ok()?
    .server();

    let buffers_at = board.read64(at::BUFFERS_AT).ok()?;
    let buffers_len = u32::try_from(board.read64(at::BUFFERS_LEN).ok()?).ok()?;
    let buffers = Granted::at(buffers_at, buffers_len).ok()?;

    let block_bytes = usize::try_from(board.read64(at::BLOCK_BYTES).ok()?).ok()?;
    if block_bytes == 0 || !buffers_len.is_multiple_of(u32::try_from(block_bytes).ok()?) {
        return None;
    }
    let buffers_count = buffers_len / u32::try_from(block_bytes).ok()?;
    let blob_bytes = usize::try_from(board.read64(at::BLOB_BYTES).ok()?).ok()?;

    Some(Parts {
        control,
        data,
        buffers,
        block_bytes,
        buffers_count,
        blob_bytes,
        seed: board.read64(at::SEED).ok()?,
    })
}

/// Build the store, write the blob, and answer the service over it with the
/// blob's content address.
///
/// The whole of what stands in for a device. `None` for a geometry no store can
/// be built at, or a heap that could not hold one — which is a board the frame
/// filled in with numbers this component's account cannot pay for, and is a
/// refusal rather than a fault.
fn stocked(
    block_bytes: usize,
    blob_bytes: usize,
    seed: u64,
) -> Option<(Service<ZonedMemory, Memory>, [u8; 32])> {
    let record_blocks = (f_abi::store::Header::BYTES + blob_bytes).div_ceil(block_bytes);
    // One zone for the record plus a spare, above the three the format reserves.
    let data_zones = (record_blocks as u64 + 1).div_ceil(ZONE_BLOCKS) as u32 + 1;
    let zones = DATA_FROM + data_zones;

    let device = ZonedMemory::new(block_bytes, ZONE_BLOCKS, zones);
    let map = ZoneMap::new(device, DATA_FROM).ok()?;
    let layout = superblock_for_this_build(
        u32::try_from(block_bytes).ok()?,
        zones,
        ZONE_BLOCKS * block_bytes as u64,
        1,
        2,
    );
    let mut store = Store::format(map, &layout).ok()?;

    // The content, from the seed. A function and not a draw — `routing::content_byte`
    // says why at length — so that the client can compute the same bytes without
    // anything crossing between them except the hash.
    let mut blob = alloc::vec::Vec::new();
    blob.try_reserve_exact(blob_bytes).ok()?;
    for offset in 0..blob_bytes {
        blob.push(routing::content_byte(seed, offset));
    }
    let hash = store.put(kind::CHUNK, &blob).ok()?;
    store.barrier().ok()?;
    drop(blob);

    let log = Memory::new(LOG_BLOCK_BYTES, LOG_BLOCKS);
    let index = Index::mount(log, 0, LOG_BLOCKS).ok()?;

    Some((Service::over(mount(store, index)), hash))
}

/// Copy one entry's payload out of the channel's inline arena.
///
/// `false` when the entry does not frame a payload inside it, which is left for
/// `Request::decode` to refuse: this function's job is to get bytes, and
/// deciding that an entry is malformed in two places would be two answers to one
/// question.
fn arena_copy(data: &Server, entry: &f_abi::Sqe, into: &mut [u8; PAYLOAD_BYTES]) -> bool {
    let Ok(offset) = usize::try_from(entry.offset) else { return false };
    data.copy_out(offset, into)
}

/// Take completions off the control ring until it is empty.
///
/// **This is the polling point.** Notices are counted on the way past; nothing
/// is discarded, and a kind this build cannot name ends the run rather than
/// being skipped — R04, and `f_abi::control::notice::known` is the one list that
/// says which kinds exist.
///
/// `Ok(true)` means stop.
fn drain(control: &Client, notices: &mut u64) -> Result<bool, u64> {
    loop {
        let taken = match control.take() {
            Ok(taken) => taken,
            Err(_) => return Err(stopped::NO_RING),
        };
        let Some(entry) = taken else { return Ok(false) };
        if is_notice(&entry) {
            if !notice::known(entry.result) {
                return Err(stopped::BAD_NOTICE);
            }
            *notices = notices.saturating_add(1);
            if entry.result == notice::STOP {
                return Ok(true);
            }
            continue;
        }
        // A completion for a request this component never made. There is none
        // in this build; dropped rather than mistaken for one.
        let _: Cqe = entry;
    }
}

/// Write what this component did into the half of the board that is its own.
///
/// The magic goes last, which is the whole of the discipline: a frame that reads
/// a page this function never finished finds a zero rather than a plausible
/// tally. RFC 0013's *read, never delivered* — the frame takes these numbers out
/// of memory it granted, and this component is never asked for them.
fn report(board: &Window, served: Option<(&Service<ZonedMemory, Memory>, u64)>, outcome: u64) {
    if let Some((service, notices)) = served {
        let counts = service.served();
        let _ = board.write64(reported::ENTRIES, counts.entries);
        let _ = board.write64(reported::READS, counts.reads);
        let _ = board.write64(reported::DELIVERED, counts.delivered_bytes);
        let _ = board.write64(reported::STAGED, counts.staged_bytes);
        let _ = board.write64(reported::REFUSED, counts.refused);
        let _ = board.write64(reported::NOTICES, notices);
    }
    let _ = board.write64(reported::OUTCOME, outcome);
    let _ = board.write64(reported::MAGIC, routing::REPORTED_MAGIC);
}

/// End, and do not come back.
fn end(status: u64) -> ! {
    let _ = door::call(door::EXIT, status, 0);
    // `EXIT` does not return. If it ever did, the frame would have a component
    // it believes is over and a core still inside it, so the only honest thing
    // left is to stop moving.
    park()
}

/// Stop, without ending. Reached only where continuing would be worse.
fn park() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
