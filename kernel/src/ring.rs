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
//! Since `E0-B13` the region is a [`Mapping`], and there are two of them over
//! the same frame: one end describes the channel and the other adopts it, each
//! learning where the rings are from the sixty-four bytes at the front and from
//! nothing else. The arithmetic never travels between the ends, which is the
//! property that makes the far end replaceable by a component later. The
//! version and the feature sets are negotiated rather than assumed, per RFC
//! 0011, and the two conclusions are required to agree — a check a single-ended
//! round trip cannot make.
//!
//! It is still not a channel between two *address spaces*. Both ends are this
//! kernel and the frame is not mapped into a process, which is what a component
//! needs and what the powerbox grant at `E1-D01` decides the shape of. What
//! this module can already say is the part that does not depend on that: the
//! bytes are laid out by the wire format, every number in the header is
//! disbelieved until checked, and both refusal paths are exercised on the
//! target rather than only on the host.
//!
//! Two phases exist to fail rather than to pass. One forges a slot number in
//! the index ring — the one thing the indirection adds to the attack surface —
//! and requires the channel to be reported corrupt rather than followed. The
//! other breaks the magic in the region's first word and requires the mapping
//! to be refused, in a build with no unwinding and no allocator, where a panic
//! is not an exception but the end of the boot.
//!
//! See `docs/design/ring-scene-boot.html` sections 02, 03 and 05.

use core::sync::atomic::Ordering;

use f_abi::{Sqe, error, op};
use f_ring::{
    Bell, Collector, Consumer, Drained, Hardware, Mapping, Path, Poster, Producer, RingError,
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
    /// The ABI version the two ends met at.
    /// Unit: none — an ABI version ordinal. Zero is not a version.
    pub version: u32,
    /// What the one drain did.
    pub drained: Drained,
    /// Whether the forged index ring was caught. Never false — a false here
    /// would have been a failure — and carried so the boot line can say the
    /// check ran rather than implying it.
    pub forgery_caught: bool,
    /// Whether a header made hostile on purpose was refused. Never false, and
    /// carried for the same reason as `forgery_caught`: a boot that reports a
    /// check it did not run is worse than one that reports nothing.
    pub header_refused: bool,
    /// The doorbell path this channel chose, from what was agreed and what the
    /// hardware reports.
    pub path: Path,
    /// Doorbells actually delivered to this core during the self-test.
    /// Unit: doorbells. One — the suppression protocol sends exactly one, and
    /// a boot that delivered none would have failed rather than reported zero.
    pub doorbells: u64,
    /// Doorbells per thousand operations, over the two the doorbell phase runs.
    ///
    /// Not the number `E0-B15` owes and not registered as a claim: two
    /// operations is not a load, and *doorbells per operation under load* is a
    /// question about a workload on a machine that can answer it. This is what
    /// the boot can honestly say — that the count exists and is not always one.
    /// Unit: doorbells per thousand operations.
    pub per_thousand: u64,
}

/// Why the frame's ring did not come up.
#[derive(Clone, Copy, Debug)]
pub enum Failure {
    /// No frame to build the channel in.
    NoFrame,
    /// The layout arithmetic refused a size this module chose. A bug here, not
    /// in a peer.
    Layout,
    /// A mapping refused its own header. Carries the structured refusal, which
    /// names whether the wire format, the version window or the layout
    /// arithmetic is the half that disagreed.
    Header(i32),
    /// The two ends adopted the same bytes and came to different conclusions,
    /// which is the one failure a single-ended round trip cannot see.
    Disagreed,
    /// A header made hostile on purpose was adopted rather than refused.
    HeaderAccepted,
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
    /// The channel negotiated a doorbell path this build cannot take. Carries
    /// the path, which is the only thing there is to say about it.
    NoPath(Path),
    /// A doorbell was sent to a consumer that had said it was draining.
    RangUnasked,
    /// A consumer that had asked to be woken was not rung.
    NotRung,
    /// A doorbell was sent and never arrived.
    NotDelivered,
}

impl Failure {
    /// A line for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoFrame => "no frame for the channel",
            Self::Layout => "the layout refused a size this kernel chose",
            Self::Header(_) => "a mapping refused the header in its own region",
            Self::Disagreed => "the two ends of one region disagreed about the channel",
            Self::HeaderAccepted => "a hostile header was adopted rather than refused",
            Self::Bind => "a ring half would not bind to the region",
            Self::PublishedEarly => "a staged batch was visible before it was published",
            Self::Ring(_) => "the ring reported an error where the protocol allows none",
            Self::Drain(_) => "the drain did not answer what was submitted",
            Self::Answer(_) => "a completion did not say what the opcode promised",
            Self::ForgeryFollowed => "a forged index-ring slot was followed rather than refused",
            Self::NoPath(_) => "the doorbell path this channel agreed cannot be taken here",
            Self::RangUnasked => "a doorbell was sent to a consumer that was draining",
            Self::NotRung => "a sleeping consumer was not rung",
            Self::NotDelivered => "a doorbell was sent and never arrived",
        }
    }
}

/// Put a payload in the arena, the way a peer would before submitting.
///
/// A free function rather than a method on a region type, because the region
/// is `f_ring`'s now. Every offset this module used to compute for itself comes
/// out of the header the mapping carries, and what is left here is the one
/// thing a *peer* does: stage bytes where a submission will point at them.
fn place(mapping: &Mapping, at: u64, bytes: &[u8]) {
    let arena = mapping.arena_cells();
    for (i, byte) in bytes.iter().enumerate() {
        // SAFETY: the caller places a payload that fits — `run` is the only
        // caller and the arena is 2 KiB against a 16-byte payload. Nothing else
        // is reading the arena at this point: the service has not been drained
        // yet.
        unsafe { arena[at as usize + i].get().write(*byte) };
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
    // ----------------------------------------------------------------- setup
    //
    // One end describes the region and the other adopts it, and they are two
    // separate binds over the same bytes rather than one value used twice. That
    // is the whole shape of a channel: the arithmetic never travels between the
    // peers, only the sixty-four bytes at the front do. `Mapping::describe`
    // itself goes out through the wire format and back in, so a header this
    // build writes and cannot read is a boot failure rather than a peer's
    // problem much later.
    let region_len = u32::try_from(region_bytes).map_err(|_| Failure::Layout)?;

    // SAFETY: the frame was allocated zeroed by `self_test`, is frame-aligned —
    // which is stronger than the cache-line alignment the layout needs — and is
    // `region_bytes` long. Nothing outside this function holds a pointer into
    // it, and both mappings below hand out only atomics and `UnsafeCell`s,
    // which is what makes two ends over one region sound.
    let near = unsafe { Mapping::describe(base, region_len, ENTRIES, 0, 0, 0) }
        .map_err(Failure::Header)?;
    // SAFETY: as above. This is the far end, and it learns where everything is
    // from the header alone.
    let far = unsafe { Mapping::adopt(base, region_len, 0, 0) }.map_err(Failure::Header)?;

    if near.layout() != far.layout() || near.negotiated() != far.negotiated() {
        return Err(Failure::Disagreed);
    }
    let layout = far.layout();
    if layout.entries() != ENTRIES || layout.total() != region_len {
        return Err(Failure::Header(0));
    }

    place(&near, PAYLOAD_AT, PAYLOAD);

    // ------------------------------------------------------------------ bind
    //
    // The service takes the near end and the client the far one, which is the
    // arrangement a component will have. Nothing below can tell the difference,
    // and that is the point.
    let mut producer = Producer::new(far.channel()).ok_or(Failure::Bind)?;
    let consumer = Consumer::new(near.channel()).ok_or(Failure::Bind)?;
    // A second handle on the consumer's end, used for one thing: setting and
    // clearing NEED_WAKEUP in the doorbell phase, after `consumer` itself has
    // been given to the service. It never pops, and that restriction is the
    // whole justification — two halves that both drained would be the protocol
    // misuse `Producer::occupancy` documents one ring over.
    let armer = Consumer::new(near.channel()).ok_or(Failure::Bind)?;
    let poster = Poster::new(near.completions()).ok_or(Failure::Bind)?;
    let collector = Collector::new(far.completions()).ok_or(Failure::Bind)?;

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
    let mut service = Service::new(consumer, poster, near.arena(), SerialSink);
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
    far.channel().index[position].store(FORGED_SLOT, Ordering::Relaxed);
    let forgery_caught = matches!(service.drain(1, now), Err(RingError::Corrupt));
    if !forgery_caught {
        return Err(Failure::ForgeryFollowed);
    }

    let version = far.negotiated().version;

    // -------------------------------------------------------------- doorbell
    //
    // The suppression protocol, and one interrupt actually arriving. `f_ring`
    // owns *when* to ring and this owns *that it rang*, which is the split the
    // whole doorbell design rests on: three implementations of one function,
    // and a number that compares them.
    //
    // Last on this channel rather than first, and the reason is the forgery
    // phase above: what that made untrustworthy is the index ring, and what
    // this phase reads is the flags word and the producer's own cursor. Neither
    // was touched. Running it earlier would have moved the cursor the forgery
    // computes its position from — which is the mistake E0-B12 recorded, in the
    // one place it would recur.
    //
    // The path is chosen the way section 03 asks: from what was agreed and what
    // the hardware reports, and those are two questions rather than one. This
    // machine offers no user interrupts — QEMU's TCG backend implements none of
    // Intel's UINTR and no `-cpu` model advertises the bit — so the honest
    // `Hardware` here says so, and `Path::select` lands on the kernel path
    // without anything being downgraded behind anyone's back.
    let hardware = Hardware { user_interrupts: false, cross_core_interrupts: true };
    let path = Path::select(far.negotiated(), hardware);
    let mut bell =
        Bell::new(path, hardware, crate::doorbell::Ipi::to_self()).map_err(Failure::NoPath)?;

    // A draining consumer is never rung. First, because it is the property that
    // holds under load and the one a broken suppression check would still pass
    // if it only ever tested the other direction.
    armer.disarm_wakeup();
    let before = crate::doorbell::delivered();
    bell.submitted(producer.submit(entry(6, op::NOP)).map_err(Failure::Ring)?);
    if bell.rings() != 0 || crate::doorbell::delivered() != before {
        return Err(Failure::RangUnasked);
    }

    // And a sleeping one is rung exactly once — with the interrupt delivered,
    // not merely sent. Interrupts are off through the whole of boot until the
    // timer window, so this opens a window for them: the legacy controllers are
    // masked, the APIC timer is masked until `apic::start`, and every vector the
    // local APIC can deliver to has a gate. The only interrupt that can arrive
    // in here is the one this line sends.
    armer.arm_wakeup();
    bell.submitted(producer.submit(entry(7, op::NOP)).map_err(Failure::Ring)?);
    if bell.rings() != 1 {
        return Err(Failure::NotRung);
    }

    // SAFETY: as argued above — every deliverable vector has a gate, and this
    // is the boot core with nothing armed but what was just sent.
    unsafe { core::arch::asm!("sti", options(nostack)) };
    let deadline = crate::arch::x86_64::read_tsc()
        .saturating_add(crate::arch::x86_64::apic::tsc_khz().saturating_mul(10));
    while crate::doorbell::delivered() == before && crate::arch::x86_64::read_tsc() < deadline {
        core::hint::spin_loop();
    }
    // SAFETY: the window is closed here and boot carries on with interrupts off,
    // which is the state every line after this one was written against.
    unsafe { core::arch::asm!("cli", options(nostack)) };

    let doorbells = crate::doorbell::delivered().wrapping_sub(before);
    if doorbells == 0 {
        return Err(Failure::NotDelivered);
    }

    // ------------------------------------------------------- a hostile header
    //
    // Last, and after the channel is finished with, because this phase writes
    // over the header the mappings above were adopted from. `ring/tests/headers`
    // covers the fifteen ways a header can lie; what this one adds is that the
    // refusal happens *here* — on real memory, in a build with no unwinding and
    // no allocator, where a panic is not an exception but the end of the boot.
    // A refusal path that has only ever been taken on the host is a refusal path
    // with a target it has never run on.
    //
    // SAFETY: aligned, inside the frame, and every reference into the region is
    // dead — `service` and both mappings are past their last use.
    unsafe { base.cast::<u64>().write_volatile(!f_abi::CHANNEL_MAGIC) };
    // SAFETY: as the binds above. The bytes are now hostile, which is the
    // subject rather than a safety obligation: `adopt` dereferences nothing
    // derived from them unless it returns, and here it must not.
    let refused = unsafe { Mapping::adopt(base, region_len, 0, 0) };
    let header_refused = refused.is_err();
    if !header_refused {
        return Err(Failure::HeaderAccepted);
    }

    Ok(Report {
        path,
        doorbells,
        per_thousand: bell.per_thousand(),
        entries: layout.entries(),
        bytes: layout.total(),
        arena: layout.arena_len(),
        version,
        drained,
        forgery_caught,
        header_refused,
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
