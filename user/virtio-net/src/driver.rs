// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The network server: the entries it answers, the direction that inverts the
//! ownership question, and the counter that says the bytes went nowhere near
//! this component.
//!
//! # What a request names, and what it therefore cannot be
//!
//! A **registered buffer set and an index**, never an address and never a
//! payload. `user/virtio-net/manifest.toml` declares `payload = "registered"`
//! and this file is what makes that declaration a mechanism:
//! [`Registered::resolve`](f_ring::registry::Registered) answers a
//! [`Reach`](f_ring::registry::Reach), a `Reach` is an address and a length and
//! deliberately not a slice, and a `Reach` goes straight into a descriptor. At
//! no point does this component hold a reference to a client's bytes, so at no
//! point can it copy them. The other path is refused rather than absent: an
//! entry carrying an address earns `ARGUMENT/FEATURE_NOT_NEGOTIATED`, because
//! this channel did not agree shared virtual memory.
//!
//! All of that is `user/virtio-blk/src/driver.rs`'s paragraph and it survives
//! unedited, which is a result rather than a saving. What follows is the part
//! that does not survive.
//!
//! # Receive inverts the question, and here is exactly what that costs
//!
//! On the block driver every transfer is a *request the device owes an answer
//! to*: a chain goes out, the doorbell rings, the used ring is polled until it
//! comes back, the buffer is released and the completion is posted. One entry
//! in, one completion out, and the buffer is in the device's hands for the
//! duration of a function call.
//!
//! A receive is not that. The client posts a buffer and **nothing is owed**: a
//! frame arrives when a peer sends one, which may be never, and the device
//! writes into memory the client owns at a moment nothing in this system chose.
//! Three consequences, and each one is a place the block driver's shape had to
//! be replaced rather than reused.
//!
//! **A completion is not produced where the entry is read.** [`Driver::execute`]
//! answers [`Answered::Later`] for a receive, and [`Driver::collect`] is what
//! produces the completion when the frame lands. The block driver's executor
//! signature — entry in, `Cqe` out — cannot express *accepted, answer to
//! follow*, and a driver that had forced it to would have had to block inside
//! `execute` until a packet arrived, which is a service that stops serving
//! because nobody sent it anything.
//!
//! **Many buffers are in the device's hands at once, and the device says which
//! one it filled.** [`crate::queue::Queue::harvest`] answers a head as well as a
//! length for that reason, and [`Posted`] is the table this side keeps against
//! it. The head is a *device's word*: [`Driver::collect`] refuses one that does
//! not name a slot this driver posted, because a driver that indexed its own
//! bookkeeping with it is a driver a device can steer into releasing a buffer
//! that is still being written.
//!
//! **The device chooses the length.** This is the one that reaches the type
//! system, and [`Driver::collect`] puts the frame's length in
//! [`Cqe::result`](f_abi::Cqe::result) because that is the only place it can go.
//! What RFC 0024's typestate *does* express is who holds the buffer: an
//! [`InFlight`](f_ring::buffers::InFlight) has no method that reaches its bytes,
//! so a client cannot read a buffer the device is filling, which is precisely
//! the receive direction's version of the rule and it holds unchanged. What it
//! does not express is **how much of the returned buffer is valid**:
//! [`InFlight::complete`](f_ring::buffers::InFlight::complete) hands back an
//! `Idle` whose `bytes()` is the whole buffer, and the frame occupies a prefix
//! of it. On the block driver the two coincided by construction — a read's
//! length is the *request's*, chosen by the client — so the gap could not appear
//! there. It is not worked around in this driver, because a driver cannot work
//! around a hole in a client's types; it is written down in RFC 0051 as a third
//! entry for the list RFC 0024 already keeps of misuses that are neither a
//! compile error nor a wire refusal.
//!
//! # The counter, and why there are two of them
//!
//! Unchanged from the block driver, deliberately, because the property being
//! claimed is the same one. There is exactly one function in this crate that
//! moves bytes — [`stage`] — and it takes the tally it moves as an argument.
//! The data path never calls it, so [`Counters::copies`] is zero.
//! [`Driver::provoke_copy`] calls it against the driver's own control page, so
//! [`Counters::provoked`] is not. A build in which `stage` had been deleted, or
//! had stopped counting, would publish a zero in *both*, and
//! `cargo xtask lint-datapath` is what turns *exactly one* and *never on the
//! data path* from prose into a check with a fixture that breaks it.
//!
//! What is worth stating for this driver and not the last one: the zero covers
//! **received** bytes too, and that is the harder half. A transmit could have
//! been zero-copy by accident — the client's bytes go out and this side never
//! needs to look at them. A receive is memory a device filled that this
//! component is the *only* thing between the device and the client, and the
//! obvious implementation reads the frame here to find out how long it is. This
//! one does not: the length comes off the used ring, not out of the frame.

use f_abi::buf::{Name, SetId, opcode};
use f_abi::deadline::{Admitted, Callee, Caller, Inherited};
use f_abi::{Cqe, Negotiated, Sqe, cflags, error, flags};
use f_ring::device::Region;
use f_ring::registry::{Domains, Refusal, Registered, Table, Transport as _};
use f_ring::{completion, refusal};

use crate::Trouble;
use crate::queue::{DESC_NEXT, DESC_WRITE, Finished, QUEUE_BYTES, QUEUE_SIZE, Queue, index};
use crate::transport::{HEADER_BYTES, Transport, Windows};

/// The opcodes this service answers on.
///
/// Numbered from one and not from zero, for R04 rather than taste:
/// `f_abi::op::NOP` is zero in the frame's own vocabulary, and an entry that
/// arrived here zeroed — a slot pulled off a free list, a peer that memset an
/// entry — would otherwise be a *transmit of buffer zero*. Zero names nothing
/// here, so a zeroed entry is refused.
///
/// The space is this service's, as `ring-scene-boot` section 05 says: a storage
/// ring and a network ring share the envelope and not the words. That the two
/// drivers in this tree both number their first opcode 1 is a coincidence of
/// counting from one and not a shared vocabulary — `net`'s `1` is a transmit and
/// `blk`'s `1` is a read, and a client that confused them would be refused by
/// the length check rather than served the wrong operation.
pub mod op {
    /// Put the frame in the named buffer on the link. The device reads.
    pub const SEND: u8 = 1;

    /// Give the device the named buffer to write the next frame into. The
    /// device writes, at a moment nothing here chose.
    pub const RECV: u8 = 2;

    /// Is this an opcode this service implements?
    ///
    /// The negative answer is the one that matters: everything else is refused
    /// with `ARGUMENT/UNKNOWN_OPCODE` rather than being read as the nearest
    /// thing, which is R04 at the one place a client's mistake would otherwise
    /// become a frame on a wire.
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SEND | RECV)
    }
}

/// The largest frame this driver will hand to or take from the device.
///
/// Fifteen hundred and fourteen bytes: an Ethernet header and a 1500-byte
/// payload, which is the link's maximum with no segmentation offload negotiated
/// and no jumbo frames. Unit: bytes.
///
/// It is a **refusal bound on both directions and the reason differs by
/// direction**, which is the sort of thing that is invisible until it is
/// written down:
///
/// - On a transmit it bounds what a client may ask to send. A frame longer than
///   the link's maximum is one the device would refuse or fragment, and neither
///   is a thing this driver can report.
/// - On a receive it is a **minimum on the buffer**. `VIRTIO_NET_F_MRG_RXBUF`
///   is not negotiated — `crate::transport` says why — so one frame occupies one
///   buffer and a device handed a shorter one **truncates the frame silently**.
///   A driver that accepted a small receive buffer would be a driver that
///   delivers half a packet with a plausible length and no error, which is R04's
///   own failure: the refusal exists because the alternative is a wrong answer
///   rather than a missing one.
pub const FRAME_MAX: u32 = 1514;

/// The least a frame may be: a destination, a source and a type.
/// Unit: bytes.
///
/// Refused rather than padded. A driver that padded would be a driver putting
/// bytes of its own on a link, which is both a copy and a fabrication.
pub const FRAME_MIN: u32 = 14;

/// Bytes of the granted region this driver keeps for its own headers.
///
/// One page. It holds the transmit header the device reads, one receive header
/// per posted buffer, and the scratch [`Driver::provoke_copy`] moves bytes
/// through. None of it is ever a client's frame — the whole point of the file is
/// that there is no such place. Unit: bytes.
pub const CONTROL_BYTES: u32 = 4096;

/// The least a driver's granted region may be.
///
/// Two queues and the control page. `user/virtio-net/manifest.toml` declares
/// sixty-four kibibytes, which is this with room to spare, and the difference is
/// deliberate: a manifest sized to exactly what a build needs is a manifest that
/// has to change every time the build does. Unit: bytes.
pub const GRANT_BYTES: u32 = QUEUE_BYTES * 2 + CONTROL_BYTES;

/// Where the transmit header sits in the control page. Unit: bytes.
const TX_HEADER_AT: u32 = 0;

/// Bytes reserved per header slot. Unit: bytes.
///
/// Sixteen rather than twelve, so each slot starts eight-byte aligned. Nothing
/// in the header is eight bytes wide and the alignment is kept anyway, because a
/// layout whose alignment depends on the fields that happen to be in it is a
/// layout that breaks when a field is added. `sim/src/net.rs` chose sixteen for
/// the same reason, and the two agreeing is worth stating rather than sharing.
const HEADER_SLOT: u32 = 16;

/// Where the receive header slots start. Unit: bytes.
const RX_HEADERS_AT: u32 = 64;

/// How many receive slots the frame's driver shape leaves room for.
///
/// **This is a bound on this driver's *stack*, and it is the largest single
/// thing E1-B03 found that the frame owes a second driver.** It is a separate
/// constant from [`RECEIVE_SLOTS`] so that it can be named, greppable and
/// checked: `cargo xtask lint-owed` carries it as a declared, unpaid deviation,
/// and the day the frame gives a driver a stack this constant goes and that
/// check goes red naming every document that describes the deviation.
///
/// # The measurement
///
/// `kernel::process` maps a scheduled driver **one page** of stack —
/// `SPAWN_STACK`, four kibibytes, with a guard page below it. A component has no
/// allocator, so everything a driver holds lives in that page: the registration
/// [`Table`], this array, the transport, two queues and the control region, in
/// [`Driver`], on the stack of [`crate::component::serve`], while
/// [`Driver::start`] is still building one.
///
/// At eight slots and sixteen registration sets, this driver's deepest frame
/// overran that page by **fifty-six bytes** — a page fault at the guard,
/// observed rather than reasoned about, `vector 14, error 0x6, address
/// 0x0000000000410fc8`. Four slots fit. That is how much headroom the shape
/// actually has, and it says something about the first driver as much as the
/// second: `user/virtio-blk` was already close to the same wall and nothing had
/// measured it.
///
/// # Why the number moved here rather than the page moving in the frame
///
/// Because moving the page moves `kernel::process::BLK_BOARD`, which is *the one
/// address a driver holds as a constant*, in two crates that cannot see each
/// other — and it changes the stack every other component shape is given, in a
/// file this task has no business rewriting. RFC 0051 argues the fix and names
/// its owner. What is refused is doing it quietly: a driver that shrank to fit
/// and said nothing would leave the next one to find the same wall from the
/// same distance.
///
/// Unit: buffers.
pub const RECEIVE_SLOTS_STACK_BOUND: usize = 4;

/// Receive buffers this driver will hold on the device's behalf at once.
///
/// Not an allocation and not a policy: it is the number of descriptor pairs the
/// layout reserves at the bottom of the receive queue, so slot `i` is
/// descriptors `2i` and `2i+1` and header slot `i`. A fixed assignment rather
/// than a free list, for `crate::queue`'s reason — a free list is an allocation
/// order, and an allocation order a component chose is a place a seeded run
/// stops reproducing.
///
/// **What decides the number is [`RECEIVE_SLOTS_STACK_BOUND`] and not this
/// protocol.** A network driver wants as many receive buffers posted as its
/// clients will give it, because a buffer that is not posted is a frame that is
/// dropped; four is what fits, and the constant above says what it fits *in*.
///
/// Unit: buffers.
pub const RECEIVE_SLOTS: usize = RECEIVE_SLOTS_STACK_BOUND;

/// Where [`Driver::provoke_copy`] moves bytes from. Unit: bytes.
const SCRATCH_FROM: u32 = 1024;

/// Where it moves them to. Unit: bytes.
const SCRATCH_TO: u32 = 2048;

/// How much it will move at once. Unit: bytes.
const SCRATCH_BYTES: u32 = 512;

const _: () = assert!(RX_HEADERS_AT + HEADER_SLOT * RECEIVE_SLOTS as u32 <= SCRATCH_FROM);
const _: () = assert!(SCRATCH_TO + SCRATCH_BYTES <= CONTROL_BYTES);
const _: () = assert!(SCRATCH_FROM + SCRATCH_BYTES <= SCRATCH_TO);
const _: () = assert!(2 * RECEIVE_SLOTS as u16 <= QUEUE_SIZE);

/// The two descriptors one transmit uses.
const TX_HEADER: u16 = 0;
const TX_DATA: u16 = 1;

/// How many times the transmit used ring is read before the frame is called
/// lost.
///
/// A count and not a duration, for the reason `vtd` gives at its own spin bound:
/// what is being waited for is a device, and a duration would need a clock —
/// which RFC 0004 does not offer a component and which would make this boot log
/// a different number on every host. Each turn reads the interrupt-status
/// register, which under emulation is an exit to the emulator and therefore a
/// point at which the device's own work can run.
///
/// It bounds a *transmit* and nothing else. The transmit queue owes an answer —
/// the device gives the descriptor back once it has taken the frame — so a bound
/// here is an anti-wedge measure. There is deliberately no equivalent for
/// receive: nothing owes this driver a packet, so a bound on waiting for one is
/// the *caller's* to choose and `crate::component` is where it is chosen, out of
/// a number the frame writes into the routing page.
///
/// *Reversal, and it has an owner:* the manifest declares an `irq`, and waiting
/// on it rather than spinning is E1-B09's.
const TRANSMIT_LIMIT: u32 = 2_000_000;

/// Registration slots this driver holds per channel.
///
/// Sixteen. A power of two because `f_ring::registry::Table` requires one — the
/// slot index is masked rather than clamped, RFC 0005 — and sixteen because the
/// manifest declares eight clients and a client with a transmit set and a
/// receive set wants two. A client that runs out is refused
/// `RESOURCE/QUOTA_EXHAUSTED`, which is a peer being told it asked for too much
/// rather than this component deciding how much memory to commit on its behalf.
pub const SETS: usize = 16;

/// What this component did, for the state tree to publish.
///
/// Counts and never durations: the boot log is a fixture that
/// `cargo xtask trace` hashes, and a number that moved with the host would take
/// the fixture with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Entries accepted without a refusal, which for a receive means *posted*
    /// rather than *answered*. Unit: entries.
    pub served: u32,
    /// Entries refused. Unit: entries.
    pub refused: u32,
    /// Bytes the device moved on behalf of clients. Unit: bytes.
    ///
    /// Counted from what the request named on a transmit and from what the
    /// device reported on a receive, because those are the two places the
    /// number actually is — a receive's length is the device's to choose and a
    /// transmit's is the client's. It is the number [`Counters::copies`] is zero
    /// beside, and a zero beside a zero says nothing.
    pub bytes: u64,
    /// Bytes this component copied on the data path. Unit: bytes.
    ///
    /// **Required to be zero**, and it is a structural property published as a
    /// number rather than a tally of something that happens: nothing on the data
    /// path can move it, which is the claim. The module comment says what keeps
    /// that true, and `cargo xtask lint-datapath` is what turns it into a check.
    pub copies: u64,
    /// Descriptors this component pointed past what a registration answered.
    /// Unit: descriptors.
    ///
    /// Zero on the data path, and moved on purpose by
    /// [`Driver::provoke_escape`], because an isolation proof whose provocation
    /// never ran is the same green as a protection that held.
    pub escaped: u32,
    /// Bytes moved through [`stage`] by [`Driver::provoke_copy`]. Unit: bytes.
    pub provoked: u64,
    /// Completions that carried [`f_abi::cflags::SHORTFALL`]. Unit: completions.
    ///
    /// Expected to be **large** rather than zero when a client submits in the
    /// hard class: `user/virtio-net/manifest.toml` declares the soft class, so
    /// every hard-class request this driver serves is served as soft and says
    /// so. R08.
    pub shortfall: u32,
    /// Entries refused `ADMISSION`/`NOT_HELD`: a class the submitting component
    /// was not admitted for. Unit: entries. RFC 0025 bound 2.
    pub unadmitted: u32,
    /// Frames handed to the transmit queue and taken back by it. Unit: frames.
    ///
    /// **Never evidence that a frame was delivered.** virtio-net's transmit
    /// queue publishes a used entry with no status anywhere: a frame the link
    /// dropped, a frame a switch discarded and a frame delivered intact are the
    /// same completion. `sim/src/net.rs` models exactly that silence and says
    /// why it is the protocol rather than a hole, and a driver that reported
    /// otherwise would be inventing information.
    pub sent: u32,
    /// Frames the receive queue gave back. Unit: frames.
    pub received: u32,
    /// Receive buffers handed to the device. Unit: buffers.
    ///
    /// Beside [`Counters::received`] rather than derived from it, and the
    /// difference between them is the interesting number: buffers posted and
    /// never filled are the ones the device still holds, which on this protocol
    /// is the resting state and not a leak.
    pub posted: u32,
    /// Turns of the receive poll that found nothing. Unit: turns.
    ///
    /// Published because it is the **cost** of having no interrupt, and R12 says
    /// a concession is written as a cost rather than hidden in a metric. A block
    /// driver's spin waits for an answer the device owes; this one waits for a
    /// packet nobody promised, and the difference is the whole argument for
    /// E1-B09 in one number.
    ///
    /// **The one counter in this structure that is not the same number on every
    /// host**, and the exception to the paragraph above this struct. It counts
    /// turns, but what ends the turning is a host's network backend answering,
    /// so it moves with the machine — measured between two thousand and one and
    /// a quarter million turns on the same runner. That is harmless only while
    /// `cargo xtask net` is outside `trace`'s fixture, which it is; whoever puts
    /// it inside one has to drop this number from the log first.
    pub spun: u64,
    /// Receive buffers given back to their clients as cancellations at
    /// teardown. Unit: buffers.
    ///
    /// The number [`Driver::cancel`] argues at length, and it is the one counter
    /// here with no counterpart in the block driver: a posted receive is a
    /// buffer with no answer owed, so a driver that stopped while holding one
    /// would leave its client holding an `InFlight` with none of RFC 0024's
    /// three exits available. A boot that never observed this move could not
    /// tell a driver that discharged that obligation from one that abandoned
    /// it — and abandoning it is not a leak, it is a client that aborts the next
    /// time it drops the buffer.
    pub cancelled: u32,
    /// Transfers that failed *after* their buffer was already with the device.
    /// Unit: transfers.
    ///
    /// **Required to be zero**, and it is the counter behind
    /// [`Driver::stopped`]. Once a chain has been offered the device owns the
    /// buffer, so a failure past that point has no refusal available to it — a
    /// refusal reaches the client's
    /// [`InFlight::complete`](f_ring::buffers::InFlight::complete), which hands
    /// the buffer back as an `Idle` the client may write while a network card
    /// holds a device-write descriptor into it. The driver puts the device in
    /// reset instead and ends, and the buffer comes back as a cancellation.
    ///
    /// A `u32` and not the `bool` this began as, and the reason is measurable
    /// rather than stylistic: a `bool` on [`Driver`] widened a structure that
    /// lives on a component's **one page** of stack by a whole word, and the
    /// component faulted its guard page eight bytes past the end. Here it fills
    /// padding this structure already had, and a number a boot can require to be
    /// zero is worth more than a flag it cannot see.
    /// `RECEIVE_SLOTS_STACK_BOUND` is the rest of that story.
    pub halted: u32,
}

/// What this component is admitted for, and what its channel says about the peer
/// submitting on it.
///
/// Not fields a driver chooses: `crate::routing` argues why a component is told
/// rather than assuming, and a ceiling is the one thing a component must not be
/// able to raise. Identical in shape to `f_virtio_blk::pending::Admission` and
/// deliberately not shared with it — the *rule* is `f_abi::deadline::inherit`
/// and lives in `abi/`, which is where a rule two components obey belongs; a
/// struct of three fields that each of them holds is not a rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Admission {
    /// The ceiling this component was admitted for, from its manifest.
    pub mine: Admitted,
    /// The ceiling the channel reports for whoever submits on it.
    pub client: Admitted,
    /// The least time this component needs from arrival to completion.
    /// Unit: nanoseconds.
    pub floor: u64,
}

/// One receive buffer the device holds.
///
/// A fixed array of these rather than a queue, indexed by slot, because the slot
/// *is* the descriptor pair and the descriptor pair is what the device gives
/// back. There is nothing here to allocate.
#[derive(Clone, Copy)]
struct Posted {
    /// The client's own value on the entry that posted it.
    /// Unit: none — a token.
    token: u64,
    /// The set the buffer belongs to, kept so that it can be released when the
    /// frame lands.
    set: SetId,
    /// Which buffer of that set. Unit: buffers, zero-based.
    index: u32,
    /// What the entry asked for, which bounds what a completion may report.
    /// Unit: bytes.
    asked: u32,
    /// Whether the entry that posted it was served below what it asked for.
    ///
    /// One bit rather than the whole [`Inherited`], and that is a size decision
    /// with a reason behind it rather than a saving. This array lives on the
    /// component's stack — a component has no allocator, so there is nowhere
    /// else for it — and the frame's driver shape maps **one page** of stack.
    /// That page is what a second driver ran out of, and RFC 0051 records it as
    /// the frame's largest debt to this task. A slot that carried the whole of
    /// `inherit`'s answer would be forty bytes wider for two fields nothing
    /// here reads back.
    ///
    /// What is *not* saved by it: the flag is still decided once, at admission,
    /// and carried rather than recomputed. Recomputing at completion would be a
    /// second reading of the same fields at a different moment, which is how a
    /// request comes to be ordered as one thing and reported as another.
    fell_short: bool,
    /// Whether the device holds this slot's buffer.
    live: bool,
}

impl Posted {
    const EMPTY: Self = Self {
        token: 0,
        // Any id will do for an empty slot, and one the registration table
        // cannot have issued is chosen so that a bug reaching a dead slot is
        // refused by the table rather than resolved against something.
        set: SetId::from_bits(0),
        index: 0,
        asked: 0,
        fell_short: false,
        live: false,
    };

    /// The name this slot's buffer answers to.
    ///
    /// Rebuilt rather than stored, because the only names this channel can carry
    /// are registered ones — `Registered::resolve` refuses a `Name::Virtual` and
    /// the manifest declares no `shared_virtual` — so a stored `Name` would be an
    /// enum with one reachable variant, paid for on every slot.
    const fn name(self) -> Name {
        Name::Registered { set: self.set, index: self.index }
    }
}

/// The head descriptor of the chain receive slot `slot` owns, if it is a slot.
///
/// The assignment that replaces a free list, as a function rather than as a
/// formula written at the place it is used. It is a function because there are
/// **two** places and they are inverses of each other — [`Driver::post`] builds
/// a chain at this head, [`Driver::collect`] is handed a head by the device and
/// has to get back to the slot — and two open-coded formulas that must agree is
/// the one shape a test cannot check by writing the formula a third time. Review
/// found exactly that: a test named for this property that recomputed both sides
/// in its own body and asserted its own arithmetic against itself.
///
/// `None` for a slot this driver does not have, which is R04 at a number that is
/// this component's own: an index past the array is a bug here, and answering an
/// address for one would be answering a device about a chain that does not
/// exist.
const fn head_for(slot: usize) -> Option<u16> {
    if slot >= RECEIVE_SLOTS {
        return None;
    }
    // Fits: `RECEIVE_SLOTS` is small and the assertion beside the constants puts
    // `2 * RECEIVE_SLOTS` inside the queue.
    #[allow(clippy::cast_possible_truncation)]
    let head = (slot as u16) * 2;
    Some(head)
}

/// Which receive slot a head names, if it names one.
///
/// **A device's word goes in here**, which is why the answer is an `Option` and
/// not an index. An odd head is not a chain this driver builds and a head past
/// the slots is not one it has; either is a device steering this side's
/// bookkeeping, and [`Driver::collect`] refuses rather than follows.
const fn slot_for(head: u16) -> Option<usize> {
    if !head.is_multiple_of(2) {
        return None;
    }
    let slot = (head / 2) as usize;
    if slot >= RECEIVE_SLOTS {
        return None;
    }
    Some(slot)
}

/// Where receive slot `slot`'s header sits in the control page. Unit: bytes.
const fn header_at(slot: usize) -> Option<u32> {
    if slot >= RECEIVE_SLOTS {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let index = slot as u32;
    Some(RX_HEADERS_AT + index * HEADER_SLOT)
}

/// What answering one entry produced.
///
/// Two variants, and the second is the whole reason this type exists rather than
/// a bare `Cqe`. The block driver's executor could answer every entry where it
/// read it; a receive is accepted now and completed when a frame arrives, and a
/// signature that could not say so would have forced the driver to block inside
/// its executor waiting for a packet nobody promised.
#[derive(Clone, Copy, Debug)]
pub enum Answered {
    /// Post this completion now. Every refusal is one of these, and so is every
    /// transmit and every registration.
    Now(Cqe),
    /// The entry was accepted and its buffer is with the device.
    /// [`Driver::collect`] produces the completion.
    Later,
}

// There is deliberately no third variant for *the device can no longer be
// driven*, and the reason is measured rather than aesthetic. It is asked
// separately — [`Driver::stopped`], read once per turn by the caller — because
// this type is returned by value on the stack of a component the frame gives
// **one page** of, and a third variant carrying an `Option<Cqe>` widened every
// frame that holds one by eight bytes. That was enough on its own: the component
// faulted its guard page eight bytes past the end, `vector 14, error 0x6,
// address 0x0000000000410ff8`. So did the `bool` that replaced it, which is why
// the flag is now [`Counters::halted`] — a `u32` in padding this driver was
// already paying for. `RECEIVE_SLOTS_STACK_BOUND` says what the page costs and
// RFC 0051 names who owes the fix; until then, a question asked once a turn is
// cheaper than an answer carried in every return.

/// The network driver.
///
/// Holds its transport, two queues, its control page, its registrations and the
/// slots the device is holding. In particular it holds no mapping of any
/// client's memory, and there is no field here that could.
pub struct Driver {
    transport: Transport,
    receive: Queue,
    transmit: Queue,
    control: Region,
    table: Table<SETS>,
    agreed: Negotiated,
    admission: Admission,
    posted: [Posted; RECEIVE_SLOTS],
    counters: Counters,
}

impl Driver {
    /// Bring the device up over the windows and the region the supervisor
    /// routed.
    ///
    /// `granted` is the one untyped region `user/virtio-net/manifest.toml`
    /// declares, already translated in this component's device domain by the
    /// spawn — which is why the driver does not ask [`Domains`] for it: putting
    /// a component's own declared needs in its domain is the spawn's work, and a
    /// driver that mapped its own queue would be a driver deciding what it was
    /// granted.
    ///
    /// The split is this driver's, as the manifest says it is: the receive
    /// queue, the transmit queue, then the control page.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a region smaller than [`GRANT_BYTES`], and
    /// anything [`Transport::open`] refuses — including
    /// [`Trouble::NoPlatformAddressing`], which is the refusal that keeps this
    /// driver from running with no isolation at all.
    pub fn start(
        windows: Windows,
        granted: Region,
        agreed: Negotiated,
        admission: Admission,
    ) -> Result<Self, Trouble> {
        if granted.len() < GRANT_BYTES {
            return Err(Trouble::Layout);
        }
        let receive_region = granted.slice(0, QUEUE_BYTES)?;
        let transmit_region = granted.slice(QUEUE_BYTES, QUEUE_BYTES)?;
        let control = granted.slice(QUEUE_BYTES * 2, CONTROL_BYTES)?;

        let transport = Transport::open(windows, QUEUE_SIZE)?;
        let receive = Queue::over(receive_region, index::RECEIVE, transport.size(index::RECEIVE)?)?;
        let transmit =
            Queue::over(transmit_region, index::TRANSMIT, transport.size(index::TRANSMIT)?)?;
        // Both queues' addresses go in before either is enabled, and that
        // ordering is the whole reason `open` and `run` are two calls. A device
        // told to enable a queue whose address registers still hold their reset
        // values is a device pointed at physical address zero — and for the
        // *receive* queue that is a device that will write there as soon as
        // anything arrives on the link, with no request outstanding and nothing
        // in this driver having asked for it.
        transport.queue_at(
            index::RECEIVE,
            receive.device_desc()?,
            receive.device_avail()?,
            receive.device_used()?,
        )?;
        transport.queue_at(
            index::TRANSMIT,
            transmit.device_desc()?,
            transmit.device_avail()?,
            transmit.device_used()?,
        )?;
        transport.run()?;

        Ok(Self {
            transport,
            receive,
            transmit,
            control,
            table: Table::new(),
            agreed,
            admission,
            posted: [Posted::EMPTY; RECEIVE_SLOTS],
            counters: Counters::default(),
        })
    }

    /// What this component has done. Unit: see [`Counters`].
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// What this component is admitted for, and what its channel says about the
    /// peer submitting on it.
    #[must_use]
    pub const fn admission(&self) -> Admission {
        self.admission
    }

    /// Registrations currently live. Unit: buffer sets.
    #[must_use]
    pub fn registrations(&self) -> usize {
        self.table.live()
    }

    /// Has a failure on the data path put the device back in reset?
    ///
    /// **Asked once a turn by the caller, and the answer ends its loop.** It is
    /// set at the two places a transfer can fail *after* its buffer is already
    /// with the device — a doorbell that could not be rung on an offered chain,
    /// and a frame the device never took — and neither is reportable as a
    /// refusal, because a refusal reaches the client's
    /// [`InFlight::complete`](f_ring::buffers::InFlight::complete), which hands
    /// the buffer back as an `Idle` the client may write while a network card
    /// holds a device-write descriptor into it. The only safe route from there
    /// is [`quiesce`](Driver::quiesce) and [`cancel`](Driver::cancel), which is
    /// the caller's teardown, which is why this is the caller's question.
    ///
    /// Once it is true every entry is refused rather than served, so a caller
    /// that ignored it drives nothing rather than driving a device in reset.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        self.counters.halted != 0
    }

    /// Receive buffers the device is holding right now. Unit: buffers.
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.posted.iter().filter(|slot| slot.live).count()
    }

    /// Decide what one entry is served as here, before it is acted on.
    ///
    /// The answer is `f_abi::deadline::inherit`'s and this adds nothing to it
    /// except the counting and the completion a refusal owes. It is a method on
    /// the driver rather than a free function so that the refusal is *tallied*:
    /// a peer claiming urgency it does not hold is a fact worth a number.
    ///
    /// `now` is passed in rather than read: this crate observes no clock, so the
    /// only caller passes zero and RFC 0025's bound 3 is a constant floor rather
    /// than one measured from arrival.
    ///
    /// # Errors
    ///
    /// The completion to post instead of acting on the entry. Already counted,
    /// so a caller writes it to the ring and does nothing else.
    pub fn admit(&mut self, entry: &Sqe, now: u64) -> Result<Inherited, Cqe> {
        let decided = f_abi::deadline::inherit(
            &Caller::of(entry, self.admission.client),
            Callee { admitted: self.admission.mine, arrival: now, floor: self.admission.floor },
        );
        match decided {
            Ok(order) => Ok(order),
            Err((packed, detail)) => {
                self.counters.refused = self.counters.refused.saturating_add(1);
                if error::unpack(packed).is_some_and(|(domain, code)| {
                    domain == error::ADMISSION && code == error::admission::NOT_HELD
                }) {
                    self.counters.unadmitted = self.counters.unadmitted.saturating_add(1);
                }
                Err(refusal(entry.user_data, packed, detail, now))
            }
        }
    }

    /// Put the device back in reset.
    ///
    /// The **first** half of stopping, and it is first for a reason that only
    /// exists on this driver: a device left able to write into posted receive
    /// buffers needs no request in flight to corrupt one, only a packet. Until
    /// this has run, no buffer the device holds may be given back to its client,
    /// because *given back* means the client may write it.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn quiesce(&self) -> Result<(), Trouble> {
        self.transport.stop()
    }

    /// Give one posted receive buffer back to its client, as a cancellation.
    ///
    /// The **second** half of stopping, and the part that has no counterpart in
    /// the block driver at all. That driver answers every entry it accepts,
    /// because every entry it accepts is a request a device owes an answer to; a
    /// posted receive is a buffer with no answer owed, so a driver that stopped
    /// while holding one would leave its client holding an
    /// [`InFlight`](f_ring::buffers::InFlight) that can never be completed.
    ///
    /// **That is not a tidiness problem, it is a wall.** RFC 0024 gives an
    /// in-flight buffer exactly three exits: a completion carrying its token,
    /// [`reclaim`](f_ring::buffers::InFlight::reclaim) on evidence the peer is
    /// gone, and a drop that panics. A live, healthy peer that simply has
    /// nothing to give back is none of the three, and
    /// [`PeerGone`](f_ring::buffers::PeerGone) cannot be constructed from *the
    /// service stopped politely* — only from an epoch change. So the obligation
    /// has to be discharged from this side, and this is where.
    ///
    /// `cflags::CANCELLED` and not an error, because RFC 0010 says cancellation
    /// is a flag rather than a refusal and `InFlight::complete` returns the
    /// buffer on one. The result is zero bytes, which is the truth: no frame
    /// arrived.
    ///
    /// Answers `None` when the device holds nothing. A caller drains it in a
    /// loop, and calling it before [`Driver::quiesce`] would be handing a client
    /// back a buffer a live device is still pointed at.
    pub fn cancel(&mut self, now: u64) -> Option<Cqe> {
        let slot = self.posted.iter().position(|slot| slot.live)?;
        let held = self.posted.get(slot).copied()?;
        let clear = self.posted.get_mut(slot)?;
        *clear = Posted::EMPTY;
        // The registration goes back whatever else happens. A refused release is
        // this side's own bookkeeping gone wrong and is reported on the
        // completion rather than swallowed, because a client whose buffer index
        // is permanently lent has no way to find that out otherwise.
        let released = Registered::bind(self.agreed, &mut self.table)
            .map_err(|packed| (packed, 0))
            .and_then(|mut path| path.release(held.name()));
        self.counters.cancelled = self.counters.cancelled.saturating_add(1);
        let mut answer = match released {
            Ok(()) => {
                let mut answer = completion(held.token, 0, now);
                answer.flags |= cflags::CANCELLED;
                answer
            }
            Err((packed, detail)) => refusal(held.token, packed, detail, now),
        };
        self.mark_shortfall(&mut answer, held.fell_short);
        Some(answer)
    }

    /// Answer one entry.
    ///
    /// Two vocabularies meet here and the dispatch order is RFC 0028's: the two
    /// registration opcodes are handled *instead of* this service's executor
    /// rather than after it, which is why [`Table::execute`] checks the envelope
    /// itself. Everything else is this service's own.
    ///
    /// `now` is passed in rather than read. This crate observes no clock — RFC
    /// 0004 — and a driver that stamped its own completions would be a component
    /// with a second opinion about time.
    pub fn execute<D: Domains>(
        &mut self,
        entry: &Sqe,
        order: Inherited,
        domains: &mut D,
        now: u64,
    ) -> Answered {
        // The literal is the whole point, and it is the same shape as [`stage`]'s
        // tally-as-an-argument: the address that reaches a descriptor is the one
        // a registration answered, plus a displacement this path passes as a
        // constant zero. There is no field to set and no branch to take.
        self.answer(entry, order, domains, now, 0)
    }

    /// Answer one entry with `beyond` bytes added to the address the
    /// registration resolved to, before it becomes a descriptor.
    ///
    /// **A provocation, and it is the receive direction's version of the one
    /// `user/virtio-blk` performs.** That driver's escape points the device at
    /// memory it was never granted and the device *reads* it; the visible
    /// consequence of an unrefused read is a sector's worth of somebody else's
    /// bytes arriving in a client's buffer, which is bad and is at least
    /// bounded by a request. This one applies the displacement to a **receive**
    /// descriptor, so what an unrefused escape produces is the device *writing*
    /// into memory this component was never granted, at a moment nothing here
    /// chose, for as long as the buffer stays posted. The remapping unit is what
    /// refuses it, because a `Reach` is an address and a length and nothing in
    /// this crate's types stops a driver adding to one.
    ///
    /// [`Counters::escaped`] counts the descriptors this produced, so a boot can
    /// require that the provocation ran rather than inferring it from a fault it
    /// did not see.
    pub fn provoke_escape<D: Domains>(
        &mut self,
        entry: &Sqe,
        order: Inherited,
        domains: &mut D,
        now: u64,
        beyond: u64,
    ) -> Answered {
        self.answer(entry, order, domains, now, beyond)
    }

    fn answer<D: Domains>(
        &mut self,
        entry: &Sqe,
        order: Inherited,
        domains: &mut D,
        now: u64,
        beyond: u64,
    ) -> Answered {
        // Nothing is served once the device is in reset. R04, and it is the
        // cheap half of what [`Driver::stopped`] is for: the expensive half is
        // that the caller stops, and this is what makes a caller that did not
        // stop harmless rather than a component offering chains to a device that
        // has been reset out from under them.
        if self.stopped() {
            self.counters.refused = self.counters.refused.saturating_add(1);
            let mut cqe = refusal(entry.user_data, Trouble::NotResponding.packed(), 0, now);
            self.report_shortfall(&mut cqe, order);
            return Answered::Now(cqe);
        }

        if opcode::is_registration(entry.opcode) {
            let mut cqe = self.table.execute(entry, domains, now);
            if cqe.is_error() {
                self.counters.refused = self.counters.refused.saturating_add(1);
            } else {
                self.counters.served = self.counters.served.saturating_add(1);
            }
            self.report_shortfall(&mut cqe, order);
            return Answered::Now(cqe);
        }

        let outcome = match entry.opcode {
            op::SEND => self.transmit(entry, now).map(Some),
            op::RECV => self.post(entry, order, beyond).map(|()| None),
            _ => Err((
                error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
                u64::from(entry.opcode),
            )),
        };

        match outcome {
            Ok(Some(mut cqe)) => {
                self.counters.served = self.counters.served.saturating_add(1);
                self.report_shortfall(&mut cqe, order);
                Answered::Now(cqe)
            }
            Ok(None) => {
                // Counted as served where it was *accepted*, not where it is
                // answered, and the two are different events for exactly one
                // opcode. `Counters::received` is the other half and the gap
                // between them is what the device is still holding.
                self.counters.served = self.counters.served.saturating_add(1);
                Answered::Later
            }
            Err((packed, detail)) => {
                self.counters.refused = self.counters.refused.saturating_add(1);
                let mut cqe = refusal(entry.user_data, packed, detail, now);
                self.report_shortfall(&mut cqe, order);
                Answered::Now(cqe)
            }
        }
        // Whether either of the two methods above put the device in reset is
        // [`Driver::stopped`]'s to answer, asked by the caller once a turn. Not
        // carried out of here on this value, for the reason stated beside
        // [`Answered`]: this component has one page of stack and the widening
        // cost more of it than the driver had.
    }

    /// Mark a completion with what the request lost on the way.
    ///
    /// One place rather than one per producer, because *every* answer this
    /// service gives owes the flag and a per-branch version is a branch somebody
    /// adds without it — which is the silent demotion RFC 0025 forecloses,
    /// arrived at by a missing line rather than by a decision. On a refusal as
    /// well as on a success: a request demoted to this service's class and
    /// *then* refused for its length was still demoted, and a client
    /// re-submitting it needs to know the class it will be served at next time.
    fn report_shortfall(&mut self, cqe: &mut Cqe, order: Inherited) {
        self.mark_shortfall(cqe, order.fell_short());
    }

    /// The same, from a bit that was decided at admission rather than from the
    /// whole of what `inherit` answered.
    ///
    /// Two entry points to one line, and the second exists because a receive is
    /// answered somewhere other than where it was admitted: [`Posted`] carries
    /// the bit across that gap and says why it carries a bit rather than an
    /// [`Inherited`]. The decision is still made once, at admission.
    fn mark_shortfall(&mut self, cqe: &mut Cqe, fell_short: bool) {
        if fell_short {
            cqe.flags |= cflags::SHORTFALL;
            self.counters.shortfall = self.counters.shortfall.saturating_add(1);
        }
    }

    /// One frame out, all the way to the device and back.
    ///
    /// Synchronous, exactly as a block transfer is, and for the reason a block
    /// transfer is: the transmit queue owes an answer. The device takes the
    /// frame and gives the descriptor back, and this side has nothing else to do
    /// with the buffer until it does.
    ///
    /// What the answer is **not** is evidence of delivery. See
    /// [`Counters::sent`].
    fn transmit(&mut self, entry: &Sqe, now: u64) -> Result<Cqe, Refusal> {
        envelope(entry)?;
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        if entry.len < FRAME_MIN || entry.len > FRAME_MAX {
            return Err((bad, u64::from(entry.len)));
        }
        // Nothing on this protocol has an offset. Refused rather than ignored: a
        // field a peer filled in and this side skipped is two peers with
        // different beliefs about what was asked, and a client that meant to
        // send from part of a buffer wants a smaller buffer.
        if entry.offset != 0 {
            return Err((
                error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
                entry.offset,
            ));
        }

        let name = Name::read(entry, self.agreed.features)?;
        let mut path =
            Registered::bind(self.agreed, &mut self.table).map_err(|packed| (packed, 0))?;
        let reach = path.resolve(name, entry.len)?;

        let outcome = self.round_trip(reach.address, entry.len);
        if outcome.is_err() {
            // The chain was offered and the doorbell rung, so the device may
            // still be reading the client's bytes — and the next statement hands
            // the buffer back, which means the client may write it. This is the
            // read-direction half of `post`'s point-of-no-return rule and it has
            // the same fix: put the device in reset first, so that *given back*
            // is true when it is said. A driver that then carried on would be
            // driving a device it had just reset, so it does not: `stopped` ends
            // the loop, through the [`Driver::stopped`] its caller asks.
            let _ = self.quiesce();
            self.counters.halted = self.counters.halted.saturating_add(1);
        }
        // The buffer goes back to the client whatever happened, and before the
        // outcome is looked at. A refusal that left a buffer lent is a client
        // that can never submit that index again.
        let released = Registered::bind(self.agreed, &mut self.table)
            .map_err(|packed| (packed, 0))?
            .release(name);
        outcome?;
        released?;

        self.counters.bytes = self.counters.bytes.saturating_add(u64::from(entry.len));
        self.counters.sent = self.counters.sent.saturating_add(1);
        Ok(completion(entry.user_data, entry.len as i32, now))
    }

    /// Build the transmit chain, offer it, ring the doorbell, and wait for the
    /// used ring.
    fn round_trip(&mut self, at: u64, len: u32) -> Result<(), Refusal> {
        // The header the device reads: no offload of any kind, so every field is
        // zero and `num_buffers` is one — a frame the driver has done nothing to
        // and the device must send whole. The same six writes `sim/src/net.rs`
        // makes from the model's side, which is what makes the two agree by
        // construction rather than by inspection.
        let mut byte = 0;
        while byte < HEADER_BYTES {
            self.control.put8(TX_HEADER_AT + byte, 0).map_err(|packed| (packed, 0))?;
            byte += 1;
        }
        self.control.put16(TX_HEADER_AT + 10, 1).map_err(|packed| (packed, 0))?;

        let header_at = self.control.device_at(TX_HEADER_AT).map_err(|packed| (packed, 0))?;

        // Two descriptors, both device-*read*: this is a transmit and a transmit
        // queue has nothing the device writes. A driver that marked either of
        // them writable would be a driver expecting an answer where the protocol
        // has none — `sim/src/net.rs` says the same sentence about the model.
        self.transmit
            .describe(TX_HEADER, header_at, HEADER_BYTES, DESC_NEXT, TX_DATA)
            .map_err(|why| (why.packed(), 0))?;
        self.transmit.describe(TX_DATA, at, len, 0, 0).map_err(|why| (why.packed(), 0))?;
        self.transmit.offer(TX_HEADER).map_err(|why| (why.packed(), 0))?;
        self.transport.kick(index::TRANSMIT).map_err(|why| (why.packed(), 0))?;

        let mut left = TRANSMIT_LIMIT;
        loop {
            if let Some(done) = self.transmit.harvest().map_err(|why| (why.packed(), 0))? {
                // The device's word about which chain it finished, checked
                // against the one chain this queue has out. A transmit queue
                // with one chain outstanding makes this cheap; it is checked
                // anyway, because the receive queue has to and a rule applied on
                // one queue is a rule somebody will find not applied on the
                // other.
                if done.head != TX_HEADER {
                    // The variant that exists for exactly this: a device naming
                    // a chain this driver never posted. It was reachable only on
                    // the receive queue until review pointed out that the arm
                    // here refused with a code the `Trouble` enum does not
                    // assign — so a client could not tell it from the arm below.
                    return Err((Trouble::Device.packed(), u64::from(done.head)));
                }
                return Ok(());
            }
            if left == 0 {
                // A device that never took the frame, and its own code: *never
                // answered* and *answered about a chain that does not exist* are
                // different failures and a client told the same thing for both
                // cannot retry one and give up on the other. R07. The detail is
                // the poll bound rather than a status — there is no status,
                // which is the point.
                return Err((Trouble::NotTaken.packed(), u64::from(TRANSMIT_LIMIT)));
            }
            left -= 1;
            // Reads a register, which is an exit to the emulator. See
            // `TRANSMIT_LIMIT`.
            let _ = self.transport.poke().map_err(|why| (why.packed(), 0))?;
        }
    }

    /// Give the device one buffer to write the next frame into.
    ///
    /// Answers nothing: the completion is [`Driver::collect`]'s, when a frame
    /// arrives. That is the signature difference this whole driver turns on, and
    /// the module comment says why a shape that could not express it would have
    /// forced this method to block.
    fn post(&mut self, entry: &Sqe, order: Inherited, beyond: u64) -> Result<(), Refusal> {
        envelope(entry)?;
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        // The **minimum**, and the direction is the opposite of the transmit
        // check above. `FRAME_MAX` says why at length: with `MRG_RXBUF` absent,
        // a device handed a shorter buffer truncates the frame and reports the
        // truncated length as though it were the frame, which is a wrong answer
        // rather than a missing one.
        if entry.len < FRAME_MAX {
            return Err((bad, u64::from(entry.len)));
        }
        if entry.offset != 0 {
            return Err((
                error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
                entry.offset,
            ));
        }

        let Some(slot) = self.posted.iter().position(|slot| !slot.live) else {
            // Every slot is with the device. A peer being told it asked for too
            // much, rather than this component deciding to hold more buffers
            // than its layout reserves descriptors for.
            return Err((
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                RECEIVE_SLOTS as u64,
            ));
        };

        let name = Name::read(entry, self.agreed.features)?;

        // Narrowed here rather than after the chain is out, and the move is the
        // fix rather than the tidying. This slot keeps a set and an index
        // because that is the only kind of name this channel can carry — R04, so
        // an entry that somehow named an address must not be recorded as buffer
        // zero of set zero — but a check that *refuses* has to happen while a
        // refusal is still free, which is to say before `offer`.
        let Name::Registered { set, index } = name else { return Err((bad, 0)) };
        // Both derived offsets, taken before anything is offered for the same
        // reason. `head_for` is also what makes `slot` an index this driver has,
        // so the recording below cannot fall off the array.
        let (Some(head), Some(header)) = (head_for(slot), header_at(slot)) else {
            return Err((bad, slot as u64));
        };

        let mut path =
            Registered::bind(self.agreed, &mut self.table).map_err(|packed| (packed, 0))?;
        let reach = path.resolve(name, entry.len)?;

        // The one line where a component's arithmetic decides what a device is
        // pointed at. On the data path `beyond` is a literal zero and this is
        // the address the frame answered; on the escape life it is not, and what
        // refuses the result is the remapping unit rather than anything here.
        let at = reach.address.wrapping_add(beyond);
        if beyond != 0 {
            self.counters.escaped = self.counters.escaped.saturating_add(1);
        }

        let header_device_at = match self.control.device_at(header) {
            Ok(at) => at,
            Err(packed) => {
                // Still this side's buffer: nothing described and nothing
                // offered. Given back rather than left lent, because a client
                // whose index stays lent can never submit it again.
                let _ = Registered::bind(self.agreed, &mut self.table)
                    .map_err(|packed| (packed, 0))?
                    .release(name);
                return Err((packed, 0));
            }
        };

        // Both descriptors are device-*write*: this is a receive, and the header
        // is something the device fills in as much as the frame is. A driver
        // that marked the header read-only would be a driver telling the device
        // to write a frame it may not describe.
        let described = self
            .receive
            .describe(head, header_device_at, HEADER_BYTES, DESC_NEXT | DESC_WRITE, head + 1)
            .and_then(|()| self.receive.describe(head + 1, at, entry.len, DESC_WRITE, 0))
            .and_then(|()| self.receive.offer(head))
            .map_err(|why| (why.packed(), 0));
        if let Err(refused) = described {
            // `offer` is the last of the three and it is the one that publishes,
            // so a failure anywhere in the chain means the device was never told:
            // the buffer is still this side's to give back.
            let _ = Registered::bind(self.agreed, &mut self.table)
                .map_err(|packed| (packed, 0))?
                .release(name);
            return Err(refused);
        }

        // --- the point of no return, and everything fallible is above it ----
        //
        // From `offer`'s publishing store the **device** owns this buffer. A
        // refusal answered from here on reaches the client's
        // `InFlight::complete`, which hands the buffer back as an `Idle` the
        // client may write — while a network card holds a device-write
        // descriptor pointing into it, with no request outstanding and nothing
        // in this system timing when it writes. Review found three refusals
        // below this line; the ordering above is what removed them, and this
        // paragraph is what stops a fourth being added.
        match self.posted.get_mut(slot) {
            Some(held) => {
                *held = Posted {
                    token: entry.user_data,
                    set,
                    index,
                    asked: entry.len,
                    fell_short: order.fell_short(),
                    live: true,
                };
            }
            // Unreachable: `head_for` refused every index outside the array.
            // Handled rather than indexed because this component has no panic
            // handler, and handled by *stopping* rather than by refusing,
            // because a buffer the device holds and this side has no record of
            // is the one thing `cancel` cannot give back.
            None => self.counters.halted = self.counters.halted.saturating_add(1),
        }
        self.counters.posted = self.counters.posted.saturating_add(1);

        // The doorbell, and its failure is not a refusal. It reads
        // `QUEUE_NOTIFY_OFF * notify_multiplier` — two words the *device*
        // published — so a device describing itself inconsistently could
        // otherwise decide that a client gets its buffer back while the device
        // still holds a write descriptor into it. The chain is already offered
        // and a device may take it with no notification at all, so the honest
        // answer is to stop: `quiesce` then `cancel` is the one path that gets
        // this buffer back safely.
        if self.transport.kick(index::RECEIVE).is_err() {
            self.counters.halted = self.counters.halted.saturating_add(1);
        }
        Ok(())
    }

    /// Take one frame the device has finished writing, if there is one.
    ///
    /// **The polling point for the receive direction**, and it is a polling
    /// point rather than a delivery for R05's reason: nothing in this system is
    /// delivered asynchronously, so a frame becomes visible when this driver
    /// looks and not when it arrives.
    ///
    /// Answers the completion to post. The length in it is the device's — the
    /// used element's, minus the header — and it is the *only* place a client
    /// can learn how much of its buffer is a frame. The module comment says what
    /// that costs at the client's types and where it is written down.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a used ring this driver cannot read, and
    /// [`Trouble::Device`] for a used element naming a chain this driver never
    /// posted — a device steering this side's bookkeeping, refused rather than
    /// followed.
    pub fn collect(&mut self, now: u64) -> Result<Option<Cqe>, Trouble> {
        let Some(Finished { head, written }) = self.receive.harvest()? else {
            self.counters.spun = self.counters.spun.saturating_add(1);
            // A register read, and the reason is not the value. Under emulation
            // it is an exit to the emulator, which is the point at which the
            // device's own work — and on this path the whole of the host's
            // network backend — can make progress. A receive poll that only read
            // memory would be a poll that never gave the thing it is waiting for
            // a chance to happen, which is a hang that looks like an empty link.
            let _ = self.transport.poke()?;
            return Ok(None);
        };

        // The device's word, checked before it indexes anything, and checked by
        // the *inverse of the function that produced it* rather than by a second
        // copy of the arithmetic. An odd head is not a chain this driver builds
        // and a head past the slots is not one it has. R04 at the one place a
        // device could otherwise choose which client's buffer this component
        // releases.
        let Some(slot) = slot_for(head) else { return Err(Trouble::Device) };
        let Some(held) = self.posted.get(slot).copied().filter(|held| held.live) else {
            return Err(Trouble::Device);
        };
        let Some(clear) = self.posted.get_mut(slot) else { return Err(Trouble::Device) };
        *clear = Posted::EMPTY;

        // The buffer goes back to the client before anything is judged, for the
        // reason `transmit` gives: a refusal that left a buffer lent is a client
        // that can never submit that index again.
        let released = Registered::bind(self.agreed, &mut self.table)
            .map_err(|_| Trouble::Layout)?
            .release(held.name());

        let mut answer = match (released, frame_length(written, held.asked)) {
            (Err((packed, detail)), _) => refusal(held.token, packed, detail, now),
            (Ok(()), Err((packed, detail))) => refusal(held.token, packed, detail, now),
            (Ok(()), Ok(len)) => {
                self.counters.bytes = self.counters.bytes.saturating_add(u64::from(len));
                self.counters.received = self.counters.received.saturating_add(1);
                // `len as i32` is the frame's length and never the buffer's, and
                // it fits because `frame_length` refused anything above
                // `held.asked`, which `post` refused above `FRAME_MAX`.
                completion(held.token, len as i32, now)
            }
        };
        self.mark_shortfall(&mut answer, held.fell_short);
        Ok(Some(answer))
    }

    /// Move [`SCRATCH_BYTES`] bytes inside this component's own control page,
    /// counting them.
    ///
    /// **Not part of the data path, and it exists so that the zero on the data
    /// path is a measurement.** The same argument `kernel/src/mem.rs` makes with
    /// `provoke_remote`: a counter nothing in a boot can move is
    /// indistinguishable from a counter that does not work, so the boot moves
    /// one on purpose and publishes it beside the one that must stay at zero.
    ///
    /// It touches the control page, which holds virtio-net headers and has never
    /// held a client's frame — there is no code in this crate that could put one
    /// there.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a control page too short, which
    /// [`Driver::start`] has already made unreachable.
    pub fn provoke_copy(&mut self) -> Result<(), Trouble> {
        stage(&self.control, SCRATCH_FROM, SCRATCH_TO, SCRATCH_BYTES, &mut self.counters.provoked)
    }
}

/// How long the frame in a used element is, given what the entry asked for.
///
/// A free function rather than a method because it is arithmetic on two numbers
/// and neither of them is the driver's: one is a device's word and the other is
/// a client's. Both are checked, and the refusals are different on purpose —
/// R07: a client that asked for a buffer the device overran cannot act on the
/// same code as a device that answered nonsense.
///
/// # Errors
///
/// `DEVICE` for a used length that does not even hold a header, which is a
/// device describing a frame that cannot exist. `ARGUMENT`/`BAD_ADDRESS` for one
/// longer than the buffer the entry named, which is the device having written
/// past what this driver told it — reported rather than clamped, because a
/// clamped length is a client told a plausible number about a buffer that was
/// overrun.
fn frame_length(written: u32, asked: u32) -> Result<u32, Refusal> {
    let Some(len) = written.checked_sub(HEADER_BYTES) else {
        return Err((Trouble::ShortUsed.packed(), u64::from(written)));
    };
    if len > asked {
        return Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), u64::from(len)));
    }
    Ok(len)
}

/// Move `len` bytes from `from` to `to` inside one region, adding them to
/// `tally`.
///
/// **The only function in this crate that moves bytes**, and the tally is an
/// argument rather than a field so that *which* counter moved says which caller
/// ran. [`Counters::copies`] is the data path's and no caller on the data path
/// passes it; [`Counters::provoked`] is the boot's own self-check's. A reader who
/// wants to disagree with *zero copies on the data path* should start by
/// searching this crate for calls to this function, which is a search with one
/// result — and `cargo xtask lint-datapath` runs that search on every `lint`.
///
/// Byte at a time rather than through a slice, and that is not a performance
/// statement: a [`Region`] hands out no slice at all, for the reason
/// `f_ring::device` gives — a slice asserts exclusive access to memory something
/// else may be writing, and on this driver's control page the something else is
/// a network card.
///
/// # Errors
///
/// [`Trouble::Register`] for a range outside the region.
fn stage(region: &Region, from: u32, to: u32, len: u32, tally: &mut u64) -> Result<(), Trouble> {
    let mut moved = 0;
    while moved < len {
        let byte = region.get8(from.saturating_add(moved))?;
        region.put8(to.saturating_add(moved), byte)?;
        moved += 1;
    }
    *tally = tally.saturating_add(u64::from(len));
    Ok(())
}

/// Refuse an entry this service will not read, in the order `f_ring::execute`
/// fixes: the reserved word, then the flags, then the opcode.
///
/// The order is not cosmetic. An entry with a non-zero reserved word is
/// malformed whatever it claims to be, and reporting the opcode first would tell
/// a caller its opcode was wrong when it was not. R04, and R07: each earns its
/// own code because a client that cannot tell which of them happened cannot
/// handle it as ordinary control flow.
fn envelope(entry: &Sqe) -> Result<(), Refusal> {
    if entry._reserved != 0 {
        return Err((
            error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
            u64::from(entry._reserved),
        ));
    }
    let unknown = entry.flags & !flags::KNOWN;
    if unknown != 0 {
        return Err((
            error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
            u64::from(unknown),
        ));
    }
    if !op::known(entry.opcode) {
        return Err((
            error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
            u64::from(entry.opcode),
        ));
    }
    // Fields this service does not read, refused rather than skipped. `cap` is
    // the registration path's and never a transfer's, and `ext` is nobody's yet.
    let unread = u64::from(entry.cap) | entry.ext[0] | entry.ext[1];
    if unread != 0 {
        return Err((error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO), unread));
    }
    Ok(())
}

/// Build the entry that puts the frame in one buffer of a registered set on the
/// link.
///
/// Beside the driver rather than in a client, for the reason
/// `f_ring::registry::registration` sits beside the table that answers it: two
/// accounts of where a field goes is one too many, and a client that had to
/// write these by hand would be a client that can get the envelope wrong.
#[must_use]
pub fn send(token: u64, len: u32) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.opcode = op::SEND;
    entry.user_data = token;
    entry.len = len;
    entry
}

/// Build the entry that gives the device one buffer of a registered set to write
/// the next frame into.
#[must_use]
pub fn recv(token: u64, len: u32) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.opcode = op::RECV;
    entry.user_data = token;
    entry.len = len;
    entry
}

#[cfg(test)]
mod tests {
    use f_ring::device::Window;

    use super::*;

    /// A control page at a descriptor's alignment. As `queue`'s fixture, and for
    /// the same reason: an alignment the compiler happened to give is a test
    /// that passes for a reason nobody chose.
    #[repr(align(16))]
    struct Owned([u8; CONTROL_BYTES as usize]);

    impl Owned {
        const fn new() -> Self {
            Self([0; CONTROL_BYTES as usize])
        }

        fn region(&mut self) -> Region {
            Region::at(self.0.as_mut_ptr() as usize as u64, 0x5000_0000, CONTROL_BYTES)
                .expect("an aligned region")
        }
    }

    #[test]
    fn the_only_function_that_moves_bytes_moves_whichever_tally_it_is_given() {
        // The test that makes `copies = 0` worth reading. Both tallies go
        // through one function, so a zero in one of them is a statement about
        // its callers rather than about the counter — and a build where `stage`
        // stopped counting would fail here rather than publishing two zeroes.
        let mut owned = Owned::new();
        let region = owned.region();
        for byte in 0..SCRATCH_BYTES {
            region.put8(SCRATCH_FROM + byte, 0xA5).expect("inside the page");
        }

        let mut copies = 0u64;
        let mut provoked = 0u64;
        stage(&region, SCRATCH_FROM, SCRATCH_TO, SCRATCH_BYTES, &mut provoked).expect("inside");
        assert_eq!(provoked, u64::from(SCRATCH_BYTES));
        assert_eq!(copies, 0, "the tally that was not passed did not move");

        stage(&region, SCRATCH_FROM, SCRATCH_TO, SCRATCH_BYTES, &mut copies).expect("inside");
        assert_eq!(copies, u64::from(SCRATCH_BYTES), "and it moves when it is");

        assert_eq!(region.get8(SCRATCH_TO), Ok(0xA5));
        assert_eq!(region.get8(SCRATCH_TO + SCRATCH_BYTES - 1), Ok(0xA5));
    }

    #[test]
    fn a_zeroed_entry_names_no_operation() {
        // The reason the opcodes start at one. An entry that was memset — a slot
        // off a free list, a peer that zeroed one — must not read as a transmit
        // of buffer zero.
        assert!(!op::known(0));
        assert_eq!(
            envelope(&Sqe::ZERO),
            Err((error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE), 0))
        );
    }

    #[test]
    fn the_envelope_is_refused_before_the_opcode_is_believed() {
        let reserved = error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO);
        let unknown_flag = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);

        let mut entry = send(1, 64);
        assert_eq!(envelope(&entry), Ok(()));

        let mut malformed = entry;
        malformed._reserved = 0xDEAD_BEEF;
        assert_eq!(envelope(&malformed), Err((reserved, 0xDEAD_BEEF)));

        let mut flagged = entry;
        flagged.flags |= 1 << 7;
        assert_eq!(envelope(&flagged), Err((unknown_flag, 1 << 7)));

        // Both at once: the reserved word first, because an entry with one is
        // malformed whatever else it says.
        let mut both = entry;
        both._reserved = 1;
        both.flags |= 1 << 6;
        assert_eq!(envelope(&both), Err((reserved, 1)));

        entry.cap = 3;
        assert_eq!(envelope(&entry), Err((reserved, 3)));
    }

    #[test]
    fn an_entry_this_service_builds_round_trips_through_its_own_envelope() {
        let asked = send(7, 42);
        assert_eq!(asked.opcode, op::SEND);
        assert_eq!(asked.user_data, 7);
        assert_eq!(asked.len, 42);
        assert_eq!(envelope(&asked), Ok(()));
        assert!(!opcode::is_registration(asked.opcode), "and it is not a registration");

        let asked = recv(8, FRAME_MAX);
        assert_eq!(asked.opcode, op::RECV);
        assert_eq!(envelope(&asked), Ok(()));
    }

    #[test]
    fn the_two_directions_bound_a_length_in_opposite_directions() {
        // The clause a reader will get backwards, asserted rather than
        // commented. A transmit is bounded *above* because the link cannot carry
        // more; a receive is bounded *below* because a device handed a short
        // buffer truncates the frame and reports the truncation as a length.
        const { assert!(FRAME_MIN < FRAME_MAX) };
        // A one-byte frame is not a frame.
        const { assert!(FRAME_MIN > 0) };
        // And the bound a receive is refused against is the transmit's ceiling,
        // which is what makes "any frame this link can carry fits" true rather
        // than approximately true.
        assert_eq!(FRAME_MAX, 1514);
    }

    #[test]
    fn a_received_length_is_the_device_word_minus_the_header_and_is_checked() {
        // The one piece of arithmetic in the receive path, and every one of its
        // three answers matters. A device that reported less than a header
        // described a frame that cannot exist; one that reported more than the
        // buffer wrote past what it was given; and the ordinary case has to come
        // out as the frame rather than the frame plus twelve.
        assert_eq!(frame_length(HEADER_BYTES + 42, 2048), Ok(42));
        assert_eq!(frame_length(HEADER_BYTES, 2048), Ok(0), "a zero-length frame is a frame");
        assert_eq!(
            frame_length(HEADER_BYTES - 1, 2048),
            Err((Trouble::ShortUsed.packed(), u64::from(HEADER_BYTES - 1)))
        );
        assert_eq!(
            frame_length(HEADER_BYTES + 2049, 2048),
            Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), 2049)),
            "a device that wrote past the buffer is reported, never clamped"
        );
    }

    #[test]
    fn every_receive_slot_owns_a_distinct_descriptor_pair_and_header() {
        // The assignment that replaces a free list. Two slots sharing either
        // would be two clients' buffers in one chain, which is the failure the
        // whole registration table exists to prevent arriving through the
        // driver's own arithmetic instead of through a peer.
        //
        // Driven through `head_for` and `header_at` rather than recomputed here.
        // The first version of this test wrote both formulas again in its own
        // body and asserted them against themselves, which is green for any
        // formula at all — including the two that break the driver, `head = slot`
        // and `slot = head`. That is why the arithmetic became two functions.
        let mut heads = [0u16; RECEIVE_SLOTS];
        let mut headers = [0u32; RECEIVE_SLOTS];
        for (slot, (head, header)) in heads.iter_mut().zip(headers.iter_mut()).enumerate() {
            *head = head_for(slot).expect("a slot this driver has");
            *header = header_at(slot).expect("a slot this driver has");
        }
        for (index, head) in heads.iter().enumerate() {
            for other in heads.iter().skip(index + 1) {
                assert_ne!(head, other);
            }
            // And a chain's two descriptors never collide with the next slot's.
            assert!(head + 1 < heads.get(index + 1).copied().unwrap_or(u16::MAX));
        }
        for (index, header) in headers.iter().enumerate() {
            for other in headers.iter().skip(index + 1) {
                assert!(header + HEADER_BYTES <= *other || other + HEADER_BYTES <= *header);
            }
        }
        // Every descriptor the assignment can produce is inside the queue.
        assert!(heads.last().copied().unwrap_or(0) + 1 < QUEUE_SIZE);
    }

    #[test]
    fn a_head_and_a_slot_are_inverses_and_a_device_word_is_refused() {
        // The property `collect` rests on, stated as a round trip rather than as
        // two formulas a reader has to compare. Every slot's head names that slot
        // back, and nothing else does.
        for slot in 0..RECEIVE_SLOTS {
            let head = head_for(slot).expect("a slot this driver has");
            assert_eq!(slot_for(head), Some(slot), "a head names the slot that built it");
        }
        assert_eq!(head_for(RECEIVE_SLOTS), None, "and there is no slot past the array");
        assert_eq!(header_at(RECEIVE_SLOTS), None);

        // A device's word, and the three ways it can be one this driver never
        // produced. Each is refused rather than reduced to a plausible slot.
        assert_eq!(slot_for(1), None, "an odd head is not a chain this driver builds");
        assert_eq!(slot_for(2 * RECEIVE_SLOTS as u16), None, "and this one is past the slots");
        assert_eq!(slot_for(u16::MAX), None);
    }

    /// A frame that answers every translation with one address and counts what
    /// it was asked.
    ///
    /// Standing in for the supervisor, and the only thing the driver depends on
    /// it for is that a registration acquires a translation. The address is
    /// deliberately not the region's own, so a test that confused a client's
    /// buffer with the driver's queue would produce an address no assertion
    /// below matches.
    struct Pinned {
        at: u64,
        mapped: u32,
        unmapped: u32,
    }

    impl Domains for Pinned {
        fn map(&mut self, _cap: u32, _len: u32) -> Result<u64, Refusal> {
            self.mapped += 1;
            Ok(self.at)
        }

        fn unmap(&mut self, _cap: u32, _address: u64, _len: u32) {
            self.unmapped += 1;
        }
    }

    /// A device made of memory: four register structures and the region the
    /// driver was granted.
    ///
    /// **The point of it is that the driver cannot tell.** Every register this
    /// transport touches is a load or a store through a [`Window`], so a window
    /// over an array answers the handshake exactly as an emulator's registers do
    /// — which is what makes it possible to drive `post` and `collect`, the two
    /// methods a boot exercises with one slot and this exercises with all of
    /// them, on a host with no device.
    ///
    /// What it does *not* model is a device that acts: nothing here fills a
    /// header or moves a frame. The used ring is written by hand below, which is
    /// the same fixture `queue`'s own tests use and for the same reason — a used
    /// element is the one input to this driver that a peer chooses.
    #[repr(align(16))]
    struct Machine {
        granted: [u8; GRANT_BYTES as usize],
        registers: [u8; 1024],
    }

    /// Where each register structure sits in the page above. Unit: bytes.
    const COMMON_AT: u32 = 0;
    const NOTIFY_AT: u32 = 256;
    const ISR_AT: u32 = 512;
    const CONFIG_AT: u32 = 768;
    const STRUCTURE_BYTES: u32 = 256;

    /// Where the frame says the driver's own region is, in the device's address
    /// space, and where it says the client's registered page is. Two numbers
    /// that are not each other and are not the host addresses either.
    const QUEUES_DEVICE_AT: u64 = 0x4000_0000;
    const CLIENT_DEVICE_AT: u64 = 0x5000_0000;

    /// The client's buffers, and how big each is. Unit: bytes, buffers.
    const CLIENT_BYTES: u32 = FRAME_MAX * RECEIVE_SLOTS as u32;
    const CLIENT_BUFFERS: u32 = RECEIVE_SLOTS as u32;

    impl Machine {
        const fn new() -> Self {
            Self { granted: [0; GRANT_BYTES as usize], registers: [0; 1024] }
        }

        /// Seed the three registers the handshake *reads* rather than writes.
        ///
        /// Everything else the transport reads back is something it wrote, which
        /// is what a memory-backed device gets right for free: a status register
        /// that keeps what was stored in it is a device that acknowledges, and a
        /// device that vetoed would be a different fixture.
        fn ready(&mut self) {
            let put32 = |bytes: &mut [u8], at: u32, value: u32| {
                let at = at as usize;
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            };
            let put16 = |bytes: &mut [u8], at: u32, value: u16| {
                let at = at as usize;
                bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
            };
            // `VIRTIO_F_VERSION_1` and `VIRTIO_F_ACCESS_PLATFORM`, in the upper
            // feature word. A fixture that offered neither would be refused by
            // `Transport::open`, which is its own test elsewhere.
            put32(&mut self.registers, COMMON_AT + 0x04, 0b11);
            put16(&mut self.registers, COMMON_AT + 0x12, 2);
            put16(&mut self.registers, COMMON_AT + 0x18, QUEUE_SIZE);
        }

        fn windows(&mut self) -> Windows {
            let base = self.registers.as_mut_ptr() as usize as u64;
            let window = |at: u32| {
                Window::at(base + u64::from(at), STRUCTURE_BYTES).expect("an aligned window")
            };
            Windows {
                common: window(COMMON_AT),
                notify: window(NOTIFY_AT),
                isr: window(ISR_AT),
                config: window(CONFIG_AT),
                notify_multiplier: 0,
            }
        }

        fn region(&mut self) -> Region {
            Region::at(self.granted.as_mut_ptr() as usize as u64, QUEUES_DEVICE_AT, GRANT_BYTES)
                .expect("an aligned region")
        }

        /// Publish one used element on the receive queue, as a device would.
        ///
        /// `written` is the header plus the frame, which is what the
        /// specification says the field holds and is the arithmetic
        /// [`frame_length`] undoes.
        fn device_finished(&mut self, index: u16, head: u16, written: u32) {
            // The receive queue is the first of the two in the granted region,
            // and its used ring is where `crate::queue` puts it.
            let used = 4096usize;
            let at = used + 4 + usize::from(index) * 8;
            self.granted[at..at + 4].copy_from_slice(&u32::from(head).to_le_bytes());
            self.granted[at + 4..at + 8].copy_from_slice(&written.to_le_bytes());
            self.granted[used + 2..used + 4]
                .copy_from_slice(&(index.wrapping_add(1)).to_le_bytes());
        }

        /// The address of descriptor `index`'s buffer, read back out of the
        /// descriptor table the driver wrote. Unit: bytes, device address space.
        fn descriptor_at(&self, index: u16) -> u64 {
            let at = usize::from(index) * 16;
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&self.granted[at..at + 8]);
            u64::from_le_bytes(bytes)
        }

        /// Descriptor `index`'s flags.
        fn descriptor_flags(&self, index: u16) -> u16 {
            let at = usize::from(index) * 16 + 12;
            u16::from_le_bytes([self.granted[at], self.granted[at + 1]])
        }
    }

    /// A driver over a memory-backed device, with one buffer set registered.
    ///
    /// Answers the driver and the set the client's entries name.
    fn driving(machine: &mut Machine, domains: &mut Pinned) -> (Driver, SetId) {
        machine.ready();
        let windows = machine.windows();
        let granted = machine.region();
        let agreed = Negotiated { version: 1, features: 0 };
        // The soft class, which is what `user/virtio-net/manifest.toml`
        // declares. Refused rather than approximated: `Admitted::new` answers
        // `None` for an ordinal that names no class.
        let admitted = Admitted::new(f_abi::class::SOFT).expect("a class");
        let admission = Admission { mine: admitted, client: admitted, floor: 0 };
        let mut driver =
            Driver::start(windows, granted, agreed, admission).expect("a device made of memory");

        // The registration goes through the driver's own executor rather than
        // through the table underneath it, because that is the path a client
        // takes — RFC 0028's dispatch, handled instead of this service's
        // executor and not after it.
        let asked = f_ring::registry::registration(1, 7, CLIENT_BYTES, CLIENT_BUFFERS);
        let order = driver.admit(&asked, 0).expect("a batch entry at a soft service");
        let Answered::Now(cqe) = driver.execute(&asked, order, domains, 0) else {
            panic!("a registration always completes")
        };
        let set = SetId::from_completion(&cqe).expect("the table's own answer");
        (driver, set)
    }

    /// The entry a client submits to post buffer `index` of `set`.
    fn posting(token: u64, set: SetId, index: u32) -> Sqe {
        let mut entry = recv(token, FRAME_MAX);
        Name::Registered { set, index }.write(&mut entry);
        entry
    }

    #[test]
    fn every_receive_slot_is_posted_and_the_device_says_which_one_filled() {
        // **The test this driver's central machinery did not have.** The boot
        // posts one receive buffer, so nothing anywhere reached slot one: the
        // `Posted` table, the `head = slot * 2` assignment, the `head / 2`
        // dispatch and the quota refusal were all argued at length and executed
        // with a single slot, which is green for a driver that cannot receive
        // into its second buffer at all.
        let mut machine = Machine::new();
        let mut domains = Pinned { at: CLIENT_DEVICE_AT, mapped: 0, unmapped: 0 };
        let (mut driver, set) = driving(&mut machine, &mut domains);
        assert_eq!(domains.mapped, 1, "the registration asked the frame for a translation");

        // Every slot, posted before any of them is answered — which is the shape
        // a transmit queue never has.
        for index in 0..CLIENT_BUFFERS {
            let entry = posting(100 + u64::from(index), set, index);
            let order = driver.admit(&entry, 0).expect("a batch entry");
            assert!(
                matches!(driver.execute(&entry, order, &mut domains, 0), Answered::Later),
                "a receive is accepted now and answered when a frame arrives",
            );
        }
        assert_eq!(driver.outstanding(), RECEIVE_SLOTS, "the device holds every slot");
        assert_eq!(driver.counters().posted, CLIENT_BUFFERS);

        // Each slot's data descriptor points at that client buffer and nothing
        // else, which is what `head = slot * 2` buys and what an overlapping
        // assignment would break: read out of the descriptor table the driver
        // wrote, at fixed offsets, rather than through the writer that wrote it.
        for slot in 0..RECEIVE_SLOTS {
            let head = head_for(slot).expect("a slot this driver has");
            let expected = CLIENT_DEVICE_AT + u64::from(slot as u32) * u64::from(FRAME_MAX);
            assert_eq!(
                machine.descriptor_at(head + 1),
                expected,
                "slot {slot}'s data descriptor names its own client buffer",
            );
            // Both halves of the chain are device-*write*, because this is the
            // direction the device writes.
            assert_eq!(machine.descriptor_flags(head) & DESC_WRITE, DESC_WRITE);
            assert_eq!(machine.descriptor_flags(head + 1) & DESC_WRITE, DESC_WRITE);
        }

        // A fifth is refused rather than served, and refused as a peer asking
        // for more than this component's layout reserves descriptors for.
        let extra = posting(200, set, 0);
        let order = driver.admit(&extra, 0).expect("a batch entry");
        let Answered::Now(refused) = driver.execute(&extra, order, &mut domains, 0) else {
            panic!("a refusal is answered where it is read")
        };
        assert_eq!(
            refused.error(),
            Some((error::RESOURCE, error::resource::QUOTA_EXHAUSTED)),
            "every slot is with the device",
        );

        // The device finishes the **last** slot first, which is legal and is the
        // case a driver keyed on arrival order gets wrong.
        let last = RECEIVE_SLOTS - 1;
        machine.device_finished(0, head_for(last).expect("a slot"), HEADER_BYTES + 64);
        let answer = driver.collect(0).expect("a used element this driver posted");
        let answer = answer.expect("a completion");
        assert_eq!(
            answer.user_data,
            100 + last as u64,
            "the completion carries the token of the slot the device named, and not the \
             token of the first buffer posted",
        );
        assert_eq!(
            answer.result, 64,
            "the frame's length, which is the used length minus \
             the header"
        );
        assert_eq!(driver.outstanding(), RECEIVE_SLOTS - 1);

        // And the first slot, second, so both ends of the array are exercised.
        machine.device_finished(1, head_for(0).expect("a slot"), HEADER_BYTES + 128);
        let answer = driver.collect(0).expect("a used element").expect("a completion");
        assert_eq!(answer.user_data, 100, "the first buffer posted, answered third");
        assert_eq!(answer.result, 128);

        // A slot answered is a slot free: the buffer went back to the client, so
        // the same index registers again.
        let again = posting(300, set, 0);
        let order = driver.admit(&again, 0).expect("a batch entry");
        assert!(matches!(driver.execute(&again, order, &mut domains, 0), Answered::Later));
    }

    #[test]
    fn a_used_element_naming_a_chain_this_driver_never_posted_is_refused() {
        // The device steering the driver, and the reason `slot_for` answers an
        // `Option`. A driver that followed a head of its device's choosing would
        // release a buffer the device is still writing into — which on this
        // direction is a network card writing into memory a client has been told
        // it owns again.
        let mut machine = Machine::new();
        let mut domains = Pinned { at: CLIENT_DEVICE_AT, mapped: 0, unmapped: 0 };
        let (mut driver, set) = driving(&mut machine, &mut domains);

        let entry = posting(100, set, 0);
        let order = driver.admit(&entry, 0).expect("a batch entry");
        assert!(matches!(driver.execute(&entry, order, &mut domains, 0), Answered::Later));

        // An odd head, which is never a chain this driver builds. A `Cqe` has
        // no equality, so the refusal is matched rather than compared — which is
        // the stronger assertion anyway: what matters is that no completion was
        // produced at all.
        machine.device_finished(0, 1, HEADER_BYTES + 64);
        assert_eq!(driver.collect(0).err(), Some(Trouble::Device));

        // A slot this driver has but never posted.
        machine.device_finished(1, head_for(RECEIVE_SLOTS - 1).expect("a slot"), HEADER_BYTES);
        assert_eq!(driver.collect(0).err(), Some(Trouble::Device));

        // And the buffer that *was* posted is still with the device, untouched
        // by either refusal.
        assert_eq!(driver.outstanding(), 1);
    }

    #[test]
    fn a_posted_buffer_comes_back_as_a_cancellation_and_only_once() {
        // The obligation the receive direction creates, exercised past the one
        // buffer a boot posts. RFC 0024 gives an in-flight buffer three exits
        // and *a live peer with nothing to give* is none of them, so every slot
        // the device still holds has to be handed back from this side.
        let mut machine = Machine::new();
        let mut domains = Pinned { at: CLIENT_DEVICE_AT, mapped: 0, unmapped: 0 };
        let (mut driver, set) = driving(&mut machine, &mut domains);

        for index in 0..CLIENT_BUFFERS {
            let entry = posting(100 + u64::from(index), set, index);
            let order = driver.admit(&entry, 0).expect("a batch entry");
            assert!(matches!(driver.execute(&entry, order, &mut domains, 0), Answered::Later));
        }

        driver.quiesce().expect("a device made of memory takes a reset");
        let mut given_back = 0;
        while let Some(answer) = driver.cancel(0) {
            assert_eq!(answer.flags & cflags::CANCELLED, cflags::CANCELLED);
            assert_eq!(answer.result, 0, "no frame arrived, which is what zero says");
            given_back += 1;
        }
        assert_eq!(given_back, CLIENT_BUFFERS, "every posted buffer, and each of them once");
        assert_eq!(driver.counters().cancelled, CLIENT_BUFFERS);
        assert_eq!(driver.outstanding(), 0);
        assert!(driver.cancel(0).is_none(), "and nothing is given back twice");
    }
}
