// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame's own ring, and the two opcodes it answers.
//!
//! # What this is, and what it is not yet
//!
//! It is a real channel: one contiguous region laid out by `f_abi::layout`,
//! with the header written into its first cache line, the cursors on their own
//! lines, the index ring, both entry arrays and the inline arena at the offsets
//! the wire format names. A batch is staged and published with one store, the
//! service drains it, and every entry is answered — including the one that is
//! refused, which is the half of a protocol that is easy to leave untested.
//!
//! It is not yet a channel *between two components*. Both ends are the kernel,
//! the region is a frame the kernel allocated rather than a mapping a process
//! shares, and no header is negotiated with a peer that could disagree. That is
//! `E0-B13`, and the split is deliberate: this task owes the layout, the cursor
//! protocol and the opcodes, and a task that also invented the mapping would
//! have tested both against each other and neither against the specification.
//!
//! What makes the exercise worth running anyway is that the *validation* is
//! real. The header is written, read back out of the region, and adopted as if
//! a peer had written it — so the arithmetic is checked against the bytes and
//! not against itself. And the last phase forges a slot number in the index
//! ring, which is the one thing the indirection adds to the attack surface, and
//! requires the channel to be reported corrupt rather than followed.
//!
//! See `docs/design/ring-scene-boot.html` sections 02, 03 and 05.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

use f_abi::layout::{self, Layout};
use f_abi::{ChannelHeader, Cqe, Sqe, error, op};
use f_ring::{
    Arena, Channel, Collector, Completions, Consumer, Cursor, Drained, Poster, Producer, RingError,
    Service, Sink,
};

use crate::mem::{FrameAllocator, Order};

/// Entries in each ring of the frame's channel.
///
/// Sixteen, because the whole channel has to fit in one frame and the entry
/// array is 64 bytes an entry: sixteen entries is a kibibyte of submissions,
/// half that of completions, and leaves 2 304 bytes of arena. A larger ring
/// needs a larger region, which needs the mapping this milestone does not have.
const ENTRIES: u32 = 16;

/// What [`op::WRITE_SERIAL`] is asked to write, placed in the arena the way a
/// peer would place a payload.
const PAYLOAD: &[u8] = b"the ring is open";

/// Where in the arena the payload is put.
///
/// Not zero, so that a bounds check which happened to treat the arena as
/// starting at the operation's offset would be caught rather than agreed with.
const PAYLOAD_AT: u64 = 64;

/// An opcode this build does not implement, submitted on purpose.
///
/// R04 says an unknown opcode is refused and never ignored. A boot that only
/// ever submits opcodes it implements cannot tell the difference between a
/// service that refuses one and a service that drops it.
const NOT_AN_OPCODE: u8 = 0xEE;

/// A slot number no sixteen-entry ring has, forged into the index ring.
const FORGED_SLOT: u32 = 0xDEAD;

/// COM1, as somewhere for an opcode to write.
struct SerialSink;

impl Sink for SerialSink {
    fn write(&mut self, bytes: &[u8]) -> usize {
        crate::arch::x86_64::serial::Serial.write_bytes(bytes);
        // The UART is polled to completion, so it takes everything. A device
        // that could take less would report less, and the service already
        // treats a short answer as a partial completion rather than an error.
        bytes.len()
    }
}

/// What the boot self-test observed.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// Entries in each ring. Unit: entries.
    pub entries: u32,
    /// Bytes the channel occupies. Unit: bytes.
    pub bytes: u32,
    /// Bytes of inline arena. Unit: bytes.
    pub arena: u32,
    /// What the one drain did.
    pub drained: Drained,
    /// Whether the forged index ring was caught. Never false — a false here
    /// would have been a failure — and carried so the boot line can say the
    /// check ran rather than implying it.
    pub forgery_caught: bool,
}

/// Why the frame's ring did not come up.
#[derive(Clone, Copy, Debug)]
pub enum Failure {
    /// No frame to build the channel in.
    NoFrame,
    /// The layout arithmetic refused a size this module chose. A bug here, not
    /// in a peer.
    Layout,
    /// The header written into the region did not survive being read back and
    /// adopted, which means the wire format and the arithmetic disagree.
    Header,
    /// A ring half would not bind to the region.
    Bind,
    /// A staged batch became visible before it was published.
    PublishedEarly,
    /// The ring reported an error where the protocol allows none.
    Ring(RingError),
    /// The drain did not do what was submitted to it.
    Drain(Drained),
    /// A completion did not say what the opcode promised.
    Answer(i32),
    /// A forged slot number in the index ring was followed rather than refused.
    ForgeryFollowed,
}

impl Failure {
    /// A line for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoFrame => "no frame for the channel",
            Self::Layout => "the layout refused a size this kernel chose",
            Self::Header => "the header did not survive a round trip through the region",
            Self::Bind => "a ring half would not bind to the region",
            Self::PublishedEarly => "a staged batch was visible before it was published",
            Self::Ring(_) => "the ring reported an error where the protocol allows none",
            Self::Drain(_) => "the drain did not answer what was submitted",
            Self::Answer(_) => "a completion did not say what the opcode promised",
            Self::ForgeryFollowed => "a forged index-ring slot was followed rather than refused",
        }
    }
}

/// One channel region, and where everything in it is.
///
/// Every accessor below is a raw-pointer cast into a region this kernel
/// allocated and no one else holds. They are grouped here rather than spread
/// through [`self_test`] so that the offsets are computed in one place, from
/// [`Layout`], and never spelled twice.
struct Region {
    base: *mut u8,
    layout: Layout,
}

impl Region {
    /// The address of one offset into the region.
    fn at(&self, offset: u32) -> *mut u8 {
        // SAFETY: every offset passed here comes from `Layout`, which refused
        // to produce one past `total()`, and the region is `total()` bytes or
        // more — checked in `self_test` before this type is built.
        unsafe { self.base.add(offset as usize) }
    }

    fn head(&self) -> &Cursor {
        // SAFETY: `layout::HEAD` is 64 bytes into a frame-aligned region, so
        // the pointer meets `Cursor`'s 64-byte alignment. The frame was
        // allocated zeroed, and a zeroed `Cursor` is a valid one. Nothing else
        // holds a reference into this region.
        unsafe { &*self.at(layout::HEAD).cast::<Cursor>() }
    }

    fn tail(&self) -> &Cursor {
        // SAFETY: as `head`, at the next line.
        unsafe { &*self.at(layout::TAIL).cast::<Cursor>() }
    }

    fn flags(&self) -> &AtomicU32 {
        // SAFETY: `layout::FLAGS` is four-byte aligned by construction and the
        // region is zeroed, which is a valid `AtomicU32`.
        unsafe { &*self.at(layout::FLAGS).cast::<AtomicU32>() }
    }

    fn cq_head(&self) -> &Cursor {
        // SAFETY: as `head`. RFC 0018 put this line here.
        unsafe { &*self.at(layout::CQ_HEAD).cast::<Cursor>() }
    }

    fn cq_tail(&self) -> &Cursor {
        // SAFETY: as `head`, at the next line.
        unsafe { &*self.at(layout::CQ_TAIL).cast::<Cursor>() }
    }

    fn index(&self) -> &[AtomicU32] {
        // SAFETY: the index ring is `entries` four-byte slots at a four-byte
        // aligned offset, inside the region, zeroed, and unaliased.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.sq_index_offset()).cast::<AtomicU32>(),
                self.layout.entries() as usize,
            )
        }
    }

    fn entries(&self) -> &[UnsafeCell<Sqe>] {
        // SAFETY: `Layout` places the entry array on a cache line, which is
        // `Sqe`'s alignment, and reserves `entries * 64` bytes for it inside
        // the region. Every byte is zero, and `Sqe::ZERO` is that bit pattern.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.sqe_offset()).cast::<UnsafeCell<Sqe>>(),
                self.layout.entries() as usize,
            )
        }
    }

    fn slots(&self) -> &[UnsafeCell<Cqe>] {
        // SAFETY: as `entries`. A `Cqe` is 32 bytes and needs 32-byte
        // alignment, which a line-aligned offset satisfies.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.cqe_offset()).cast::<UnsafeCell<Cqe>>(),
                self.layout.entries() as usize,
            )
        }
    }

    fn arena(&self) -> &[UnsafeCell<u8>] {
        // SAFETY: the arena is the tail of the region, `arena_len()` bytes of
        // it, and a byte needs no alignment.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.arena_offset()).cast::<UnsafeCell<u8>>(),
                self.layout.arena_len() as usize,
            )
        }
    }

    /// Put a payload in the arena, the way a peer would before submitting.
    fn place(&self, at: u64, bytes: &[u8]) {
        let arena = self.arena();
        for (i, byte) in bytes.iter().enumerate() {
            // SAFETY: the caller places a payload that fits — `self_test` is
            // the only caller and the arena is 2 KiB against a 16-byte payload.
            // Nothing else is reading the arena at this point: the service has
            // not been drained yet.
            unsafe { arena[at as usize + i].get().write(*byte) };
        }
    }
}

/// Bring up the frame's channel, run both opcodes across it, and tear it down.
///
/// Runs once, on the boot core, before user space exists. What it proves is
/// that the layout in `f_abi` and the protocol in `f_ring` agree with each
/// other against real memory rather than against a test fixture.
///
/// # Errors
///
/// A [`Failure`] naming which step did not hold. Every one of them is a bug in
/// this kernel rather than something a peer caused — there is no peer yet —
/// which is why they are all fatal to the boot.
pub fn self_test(frames: &mut FrameAllocator, now: u64) -> Result<Report, Failure> {
    let frame = frames.alloc_zeroed(Order::FRAME).ok_or(Failure::NoFrame)?;
    let result = run(frames.virt(frame), frame.bytes(), now);

    // Given back whether or not the test passed, so that a failure here does
    // not also show up as a frame leak in the line above it and send the next
    // reader after the wrong thing.
    // SAFETY: the region was allocated here, nothing else was given a pointer
    // into it, and `run` has returned, so every reference into it is dead.
    unsafe { frames.free(frame) };

    result
}

/// The body of [`self_test`], with the frame's lifetime handled by the caller.
fn run(base: *mut u8, region_bytes: u64, now: u64) -> Result<Report, Failure> {
    // ---------------------------------------------------------------- layout
    //
    // Laid out with no arena, then adopted back with the region's true length,
    // which is what fills the arena in. That is the same path a peer's channel
    // takes and it is deliberately not the shortcut of computing the arena here
    // — the arena's extent is a property of the mapping, and asking the mapping
    // is the only way to learn it that stays correct when the mapper is not us.
    let described = Layout::new(ENTRIES, 0).ok_or(Failure::Layout)?;
    let header = described.describe(0, 0, 0);

    // SAFETY: the region is frame-aligned, so it meets `ChannelHeader`'s
    // 64-byte alignment, and is at least one frame long — far more than the
    // header's 64 bytes. Nothing else holds a pointer into it.
    unsafe { base.cast::<ChannelHeader>().write(header) };

    // Read back out of the region rather than reused from above. The value that
    // matters is the one in the bytes, and a round trip through memory is what
    // would catch a layout whose Rust type and whose wire image disagree.
    // SAFETY: just written, aligned, and `ChannelHeader` is plain data.
    let written = unsafe { base.cast::<ChannelHeader>().read() };

    let region_len = u32::try_from(region_bytes).map_err(|_| Failure::Layout)?;
    let layout = Layout::adopt(&written, region_len).map_err(|_| Failure::Header)?;
    if layout.entries() != ENTRIES || layout.total() != region_len {
        return Err(Failure::Header);
    }

    let region = Region { base, layout };
    region.place(PAYLOAD_AT, PAYLOAD);

    // ------------------------------------------------------------------ bind
    let channel = || Channel {
        head: region.head(),
        tail: region.tail(),
        flags: region.flags(),
        index: region.index(),
        entries: region.entries(),
    };
    let completions =
        || Completions { head: region.cq_head(), tail: region.cq_tail(), slots: region.slots() };

    let mut producer = Producer::new(channel()).ok_or(Failure::Bind)?;
    let consumer = Consumer::new(channel()).ok_or(Failure::Bind)?;
    let poster = Poster::new(completions()).ok_or(Failure::Bind)?;
    let collector = Collector::new(completions()).ok_or(Failure::Bind)?;

    // --------------------------------------------------------------- publish
    //
    // Four entries, one store. Two that do nothing, one that writes, and one
    // this build does not implement — so the drain below has to produce three
    // answers and one refusal rather than four of anything.
    let mut batch = producer.batch();
    batch.push(entry(1, op::NOP)).map_err(Failure::Ring)?;
    batch.push(entry(2, op::NOP)).map_err(Failure::Ring)?;

    let mut write = entry(3, op::WRITE_SERIAL);
    write.offset = PAYLOAD_AT;
    write.len = PAYLOAD.len() as u32;
    batch.push(write).map_err(Failure::Ring)?;

    batch.push(entry(4, NOT_AN_OPCODE)).map_err(Failure::Ring)?;

    // The property batching exists for, checked rather than assumed: nothing is
    // visible until the single store below. Asked of the *consumer* rather than
    // of the producer's own occupancy, which is both the stronger question and
    // the only one available — `Producer::batch` takes `&mut self` precisely so
    // that the producer cannot be interrogated while a batch is open, and the
    // borrow checker refuses the weaker version of this check.
    if consumer.pop().map_err(Failure::Ring)?.is_some() {
        return Err(Failure::PublishedEarly);
    }
    batch.publish().map_err(Failure::Ring)?;

    // ----------------------------------------------------------------- drain
    //
    // The bytes `WRITE_SERIAL` writes land on the boot log between the quotes
    // printed here. They are on that line because an opcode put them there, not
    // because this function formatted them — which is the whole demonstration,
    // and would be invisible if the payload were printed afterwards.
    crate::kprint!("  ring wrote    \"");
    let mut service = Service::new(consumer, poster, Arena::new(region.arena()), SerialSink);
    let drained = service.drain(ENTRIES, now).map_err(Failure::Ring)?;
    crate::kprintln!("\" through WRITE_SERIAL");

    if drained != (Drained { executed: 4, completed: 4, refused: 1 }) {
        return Err(Failure::Drain(drained));
    }

    // --------------------------------------------------------------- answers
    check(&collector, 0)?;
    check(&collector, 0)?;
    check(&collector, PAYLOAD.len() as i32)?;

    let refused = collector.take().map_err(Failure::Ring)?.ok_or(Failure::Answer(0))?;
    if refused.error() != Some((error::ARGUMENT, error::argument::UNKNOWN_OPCODE))
        || refused.ext != u64::from(NOT_AN_OPCODE)
    {
        return Err(Failure::Answer(refused.result));
    }

    // --------------------------------------------------------------- forgery
    //
    // The one thing the index ring adds to the attack surface. A peer that
    // cannot write an impossible cursor — the check that has existed since M0 —
    // can still write an impossible *slot*, and the consumer must refuse to
    // follow it. Done last because a channel that has been lied to is torn
    // down rather than repaired.
    //
    // The slot forged is the one the consumer will *read*, which is the
    // position its own cursor names — four, after four entries were drained —
    // and not slot zero. Forging zero is the version of this test that passes
    // whatever the consumer does, because the consumer never looks there; it
    // was written that way first and reported a clean boot for a check that had
    // not run. The position is computed rather than written down for the same
    // reason.
    let position = (drained.executed & layout.mask()) as usize;
    producer.submit(entry(5, op::NOP)).map_err(Failure::Ring)?;
    region.index()[position].store(FORGED_SLOT, Ordering::Relaxed);
    let forgery_caught = matches!(service.drain(1, now), Err(RingError::Corrupt));
    if !forgery_caught {
        return Err(Failure::ForgeryFollowed);
    }

    Ok(Report {
        entries: layout.entries(),
        bytes: layout.total(),
        arena: layout.arena_len(),
        drained,
        forgery_caught,
    })
}

/// One submission, with the fields nothing here varies already set.
fn entry(token: u64, opcode: u8) -> Sqe {
    let mut sqe = Sqe::ZERO;
    sqe.user_data = token;
    sqe.opcode = opcode;
    sqe
}

/// Take the next completion and require it to carry `expect`.
fn check(collector: &Collector<'_>, expect: i32) -> Result<(), Failure> {
    let cqe = collector.take().map_err(Failure::Ring)?.ok_or(Failure::Answer(0))?;
    if cqe.result != expect {
        return Err(Failure::Answer(cqe.result));
    }
    Ok(())
}
