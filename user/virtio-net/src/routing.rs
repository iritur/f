// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where this component's needs landed, written by the frame and read by the
//! component, in one page neither of them has to guess the shape of.
//!
//! # Why a component is told and does not compute
//!
//! `f_abi::door::Entry` makes the argument for the *handles*: the frame tells a
//! component what it holds rather than letting it write the indices down,
//! because a second occupant of a place finds the same indices at a later
//! generation. The addresses are the same argument one layer out and it is
//! sharper, because two of them cannot be constants at all — a device's register
//! structures are wherever the device says they are, and the device address of
//! the component's queue memory is what a translation answered.
//!
//! `user/virtio-blk/src/routing.rs` says all of that first and this file does
//! not repeat the argument. What it does instead is record the one thing E1-B03
//! learned by writing the same page a second time.
//!
//! # The two drivers hold the same constant, and that is a finding
//!
//! [`AT`] equals `f_virtio_blk::routing::AT`, and both equal
//! `kernel::process::BLK_BOARD`. That is not a coincidence and it is not
//! sharing: the frame builds *one* driver shape — `process::prepare_driver` —
//! whose address plan reserves a text region, a stack, a control ring, a data
//! ring, a board, four register pages and a queue region, and every driver
//! scheduled into that shape finds its board at the same address because there
//! is one shape.
//!
//! Two things follow, and both belong here rather than in an RFC nobody reads
//! before editing this file:
//!
//! - **The one address a driver holds as a constant is now held by two crates**,
//!   and neither of them can see the other. `kernel/src/net.rs` carries the
//!   compile-time assertion for this one exactly as `kernel/src/blk.rs` carries
//!   it for the block driver's, because the kernel is the one artefact that
//!   links every definition. Two assertions, one per driver, and a third driver
//!   adds a third — which is the shape of a rule that should have moved into
//!   `abi/` and has not. RFC 0051.
//! - **The constants are named `BLK_`,** which is the frame naming a general
//!   shape after the first thing that fitted it. The names are not changed here:
//!   `kernel/src/process.rs` is a file other work is in, and a rename that
//!   touched every driver in the tree to make a comment true is a change with
//!   more risk than content. It is written down instead, which is the same trade
//!   `kernel/src/arch/x86_64/virtio.rs` made about `dma.rs`'s duplicated walk.
//!
//! # The second half is the component's, and the frame only reads it
//!
//! Offsets from [`REPORT`] up are written by the *component* and read by the
//! frame after the run. RFC 0013's *read, never delivered* — the frame watching
//! a component through memory it granted, costing the component nothing and
//! telling it nothing.
//!
//! A component that scribbles this half lies about its own counters and about
//! nothing else. It cannot lie about whether the device faulted, which is the
//! remapping unit's own fault-recording registers, and it cannot lie about
//! whether a frame arrived, because the frame that landed is in the *client's*
//! memory and the client reads it there. Those two are what the halves of
//! `cargo xtask net` actually turn on.

/// Which of this component's lives the frame asked for, in the low half of
/// `f_abi::door::Entry`.
///
/// Three, and the third is a provocation rather than a mode. A selector this
/// build does not name falls through to [`ANNOUNCE`], which is the life a
/// *spawn* into a place asks for — so a frame that forgot to set one gets an
/// announcement rather than a driver that drives nothing.
///
/// There is deliberately **no fourth selector for the negative control**. The
/// control this task needs is *the same driver, with the client sending
/// nothing*, and that is a fact about the client rather than about the
/// component — a selector for it would be a driver with a mode in which it does
/// not receive, which is a different experiment wearing the control's name.
pub mod life {
    /// Announce and end. What a spawn into a place asks for.
    pub const ANNOUNCE: u32 = 0;
    /// Serve the data ring until the frame says stop.
    pub const SERVE: u32 = 1;
    /// The same, and add [`at::BEYOND`] to the address a registration answered
    /// before it becomes a **receive** descriptor.
    ///
    /// A separate selector rather than a flag, for the reason
    /// `Driver::provoke_escape` is a separate entry point: the provocation has
    /// to be greppable, and a driver whose data path took a branch on a mode
    /// word would be a data path with a provocation in it.
    pub const ESCAPE: u32 = 2;
}

/// Where the frame maps this page in the component's address space.
///
/// The one number both sides hold, and — see the module comment — the same
/// number the block driver holds, because there is one driver shape in the
/// frame. It must equal `kernel::process::BLK_BOARD`, and `kernel/src/net.rs`
/// asserts that at compile time rather than saying it in a comment.
///
/// Unit: bytes, in the component's own address space.
pub const AT: u64 = 0x0041_5000;

/// How many bytes the page is. One frame.
/// Unit: bytes.
pub const BYTES: u32 = 4096;

/// A word the frame writes first and the component checks before it believes
/// anything else here.
///
/// R04, at the one place a component reads a structure it did not build: a page
/// of zeroes is what an unmapped-and-then-mapped frame looks like, and a driver
/// that took a zero for a length would refuse rather than fault — which reads as
/// a device problem. The magic makes *the frame did not fill this in* a distinct
/// answer from *the frame said zero*.
///
/// Different from the block driver's, and it has to be. Both components are
/// mapped at [`AT`] by the same driver shape, so a build that routed the wrong
/// image into the wrong supervisor would otherwise find a page whose magic
/// matched and whose fields meant something else — a driver reading a block
/// driver's routing page as its own. One constant apart is what makes that a
/// refusal instead.
pub const MAGIC: u64 = 0x6E65_745F_726F_7574;

/// Byte offsets of the fields the frame writes, each a little-endian `u64`.
///
/// Slots rather than a `repr(C)` struct, because the two sides read and write
/// them through `f_ring::device::Window`, which is a bounds-checked volatile
/// accessor and not a reference — there is no struct to borrow. Eight bytes each
/// even where four would do, so that adding a field never moves one.
pub mod at {
    /// [`super::MAGIC`]. Unit: none.
    pub const MAGIC: u32 = 0;
    /// The common configuration structure, as an offset into the register window
    /// the frame mapped. Unit: bytes.
    pub const COMMON_OFFSET: u32 = 8;
    /// Its length. Unit: bytes.
    pub const COMMON_LEN: u32 = 16;
    /// The notification structure. Unit: bytes.
    pub const NOTIFY_OFFSET: u32 = 24;
    /// Its length. Unit: bytes.
    pub const NOTIFY_LEN: u32 = 32;
    /// The interrupt-status register. Unit: bytes.
    pub const ISR_OFFSET: u32 = 40;
    /// Its length. Unit: bytes.
    pub const ISR_LEN: u32 = 48;
    /// The device-specific configuration structure. Unit: bytes.
    ///
    /// Routed and never read, because `VIRTIO_NET_F_MAC` is not negotiated —
    /// `crate::transport` says what that costs. Present in the layout anyway,
    /// because the manifest declares four register frames and a page the frame
    /// stopped filling in would be a manifest describing a component nobody had
    /// checked against it.
    pub const CONFIG_OFFSET: u32 = 56;
    /// Its length. Unit: bytes.
    pub const CONFIG_LEN: u32 = 64;
    /// How far apart two queues' doorbells are inside the notification
    /// structure, as the device reported it.
    /// Unit: bytes per queue index. Zero is legal.
    ///
    /// The one routed number this driver uses that the block driver could have
    /// ignored: with one queue a multiplier multiplies nothing, and with two the
    /// receive and transmit doorbells are this many bytes apart. A driver that
    /// read it as zero would ring one doorbell for both queues and would
    /// transmit without ever receiving.
    pub const NOTIFY_MULTIPLIER: u32 = 72;
    /// Where the register window itself is.
    /// Unit: bytes, in the component's address space.
    pub const REGISTERS_AT: u32 = 80;
    /// How many bytes of it there are. Unit: bytes.
    pub const REGISTERS_LEN: u32 = 88;
    /// Where the queue memory is, for the component.
    /// Unit: bytes, in the component's address space.
    pub const QUEUES_AT: u32 = 96;
    /// Where the *device* addresses the same bytes — what a translation
    /// answered, and never assumed to equal the above.
    /// Unit: bytes, in the device's address space.
    pub const QUEUES_DEVICE_AT: u32 = 104;
    /// How many bytes of queue memory. Unit: bytes.
    pub const QUEUES_LEN: u32 = 112;
    /// Where the control ring is.
    /// Unit: bytes, in the component's address space.
    pub const CONTROL_AT: u32 = 120;
    /// How many bytes of it. Unit: bytes.
    pub const CONTROL_LEN: u32 = 128;
    /// Where the ring this component serves its client on is.
    /// Unit: bytes, in the component's address space.
    pub const DATA_AT: u32 = 136;
    /// How many bytes of it. Unit: bytes.
    pub const DATA_LEN: u32 = 144;
    /// The ABI version the frame negotiated on the client's behalf.
    /// Unit: none.
    pub const NEGOTIATED_VERSION: u32 = 152;
    /// The feature set beside it, whole.
    /// Unit: none — a bitmask of `f_abi::feature` constants.
    pub const NEGOTIATED_FEATURES: u32 = 160;
    /// How far past what a registration answered the [`super::life::ESCAPE`]
    /// life points the device, on a **receive** descriptor.
    ///
    /// Told to the component rather than chosen by it, because the frame is what
    /// knows how far outside a grant is far enough to be outside it and near
    /// enough that the remapping unit has a table to fault it in. A component
    /// that picked its own displacement would be a provocation choosing its own
    /// difficulty. Unit: bytes.
    pub const BEYOND: u32 = 168;
    /// The class this component was admitted for, from its manifest's
    /// `[reservation] class` by way of `f_abi::manifest::class::admitted`. A
    /// request is never served above it — RFC 0025 bound 1.
    /// Unit: none — an `f_abi::class` ordinal.
    pub const ADMITTED: u32 = 176;
    /// The class the *channel* reports about whoever submits on it. An entry
    /// claiming anything more urgent is refused `ADMISSION`/`NOT_HELD` — RFC
    /// 0025 bound 2 — and it is here, in the page the frame writes, precisely so
    /// that it is never a field of an entry.
    /// Unit: none — an `f_abi::class` ordinal.
    pub const CLIENT_ADMITTED: u32 = 184;
    /// The least time this component needs from arrival to completion for any
    /// request — RFC 0025 bound 3. Unit: nanoseconds.
    pub const FLOOR: u32 = 192;
    /// How many turns of its loop the component will spend waiting for a frame
    /// that nobody promised, before it stops waiting.
    ///
    /// **The field the block driver has no equivalent of, and the honest name
    /// for a thing that would otherwise be a hang.** Every other bound in either
    /// driver waits for an answer a device owes: a chain was offered, the
    /// doorbell rang, and the used ring will come back. A receive queue owes
    /// nothing — a frame arrives when a peer sends one, which may be never — so
    /// a driver with no interrupt and no bound is a driver that stops.
    ///
    /// It is *told* rather than chosen for [`BEYOND`]'s reason inverted: how
    /// long to wait for a network is a property of the machine and the backend,
    /// which the frame knows and the component cannot. A component that picked
    /// its own would be a fixture choosing its own patience.
    ///
    /// A count and not a duration, because RFC 0004 offers a component no clock
    /// — and because a count is the same number on every host, which is what
    /// keeps `cargo xtask trace`'s fixture a fixture. What it is *not* is a
    /// duration in disguise: `crate::driver::Counters::spun` publishes how many
    /// of these turns were actually spent, so a reader can see the bound and the
    /// use of it side by side. Unit: turns.
    pub const RECEIVE_SPINS: u32 = 200;
}

/// Where the component's own half of the page starts.
///
/// Half a page in, so that neither side can reach the other's fields by an
/// arithmetic slip of a few bytes: the frame's writes stop long before here and
/// the component's start here. It is not protection — one page is one mapping
/// and the component may write all of it — it is distance, which is what makes a
/// misplaced offset a wrong *answer* rather than a corrupted one.
/// Unit: bytes.
pub const REPORT: u32 = 2048;

/// Byte offsets of the fields the component writes.
pub mod reported {
    /// [`super::MAGIC`] again, written last, so that a frame reading a page the
    /// component never reached finds a zero rather than a plausible tally.
    pub const MAGIC: u32 = super::REPORT;
    /// `Counters::served`. Unit: entries.
    pub const SERVED: u32 = super::REPORT + 8;
    /// `Counters::refused`. Unit: entries.
    pub const REFUSED: u32 = super::REPORT + 16;
    /// `Counters::bytes`. Unit: bytes.
    pub const BYTES: u32 = super::REPORT + 24;
    /// `Counters::copies`, the number this whole subsystem is about.
    /// Unit: bytes.
    pub const COPIES: u32 = super::REPORT + 32;
    /// `Counters::escaped`. Unit: descriptors.
    pub const ESCAPED: u32 = super::REPORT + 40;
    /// `Counters::provoked`. Unit: bytes.
    pub const PROVOKED: u32 = super::REPORT + 48;
    /// How many entries the component took off its data ring.
    ///
    /// Beside [`SERVED`] rather than derived from it, because they are two
    /// different claims: one is what the component's own executor counted and
    /// the other is what its loop saw arrive. A build where the loop had stopped
    /// draining publishes the same `served` as one where it never started.
    /// Unit: entries.
    pub const DRAINED: u32 = super::REPORT + 56;
    /// What stopped the component, as one of the [`stopped`](super::stopped)
    /// constants. Unit: none — an ordinal.
    pub const OUTCOME: u32 = super::REPORT + 64;
    /// `Counters::shortfall`. Unit: completions.
    pub const SHORTFALL: u32 = super::REPORT + 72;
    /// `Counters::unadmitted`. Unit: entries.
    pub const UNADMITTED: u32 = super::REPORT + 80;
    /// `Counters::sent` — frames handed to the transmit queue and taken back.
    /// **Never evidence of delivery.** Unit: frames.
    pub const SENT: u32 = super::REPORT + 88;
    /// `Counters::received` — frames the receive queue gave back. Unit: frames.
    pub const RECEIVED: u32 = super::REPORT + 96;
    /// `Counters::posted` — receive buffers handed to the device. Unit: buffers.
    ///
    /// Beside [`RECEIVED`] and not derived from it: the difference is what the
    /// device is still holding, which on this protocol is the resting state
    /// rather than a leak, and a boot that could not tell *posted and not
    /// filled* from *never posted* could not tell a working receive path from an
    /// absent one.
    pub const POSTED: u32 = super::REPORT + 104;
    /// `Counters::spun` — turns of the receive poll that found nothing.
    /// Unit: turns.
    ///
    /// The cost of having no interrupt, published because R12 says a concession
    /// is written as a cost rather than hidden in a metric. Read beside
    /// [`super::at::RECEIVE_SPINS`], which is the bound it was allowed.
    pub const SPUN: u32 = super::REPORT + 112;
    /// `Counters::cancelled` — receive buffers given back to their clients as
    /// cancellations at teardown. Unit: buffers.
    ///
    /// **The number this driver has and the block driver cannot have.** A
    /// posted receive is a buffer with no answer owed, so a driver that stopped
    /// while holding one would leave its client with an `InFlight` that has none
    /// of RFC 0024's three exits available — not a completion, because none is
    /// coming; not `reclaim`, because the peer is alive; not a drop, because a
    /// dropped in-flight buffer ends the component. `driver::Driver::cancel` is
    /// where the obligation is discharged and this is how a boot sees that it
    /// was.
    pub const CANCELLED: u32 = super::REPORT + 120;
    /// `Counters::halted` — transfers that failed after their buffer was with
    /// the device. Unit: transfers.
    ///
    /// Zero on every boot this tree runs, and published rather than folded into
    /// [`OUTCOME`] because the two answer different questions: the outcome says
    /// the loop ended on `DEVICE_HOLDS`, and this says how many transfers found
    /// themselves past the point where a refusal would have handed a client back
    /// a buffer a device still holds.
    pub const HALTED: u32 = super::REPORT + 128;
}

/// Why the component's loop ended.
///
/// Written into [`reported::OUTCOME`] so that a boot can tell a driver that
/// served its client and was told to stop from one that fell out of its loop
/// because something it read did not make sense. Both exit; only one of them is
/// the run the boot asked for, and a status word that could not tell them apart
/// would make every refusal in this component read as success.
pub mod stopped {
    /// The frame's stop notice arrived and the loop ended on it.
    pub const TOLD: u64 = 1;
    /// The routing page did not carry [`super::MAGIC`], so nothing after it was
    /// believed.
    pub const NO_ROUTING: u64 = 2;
    /// An address in the routing page could not be stated as a window, a region
    /// or a channel.
    pub const BAD_ROUTING: u64 = 3;
    /// The device did not start. `Driver::start`'s own refusal.
    pub const NO_DEVICE: u64 = 4;
    /// A ring stopped validating under the component, which is a peer that has
    /// stopped speaking.
    pub const NO_RING: u64 = 5;
    /// The zero-copy self-check refused, so the zero it stands behind would have
    /// been a zero nothing could move.
    pub const NO_SELF_CHECK: u64 = 6;
    /// The device published a used element naming a chain this driver never
    /// posted.
    ///
    /// Its own outcome rather than folded into [`NO_DEVICE`], because it is the
    /// one failure in this component that is a *device steering the driver*
    /// rather than a device failing to start — and the two want different
    /// answers from whoever reads the log. R07 applied to a status word.
    pub const BAD_DEVICE: u64 = 7;
    /// A transfer failed after its buffer was already with the device, so the
    /// driver put the device in reset rather than answering a refusal.
    ///
    /// Its own outcome and not [`BAD_DEVICE`], because the two are read by
    /// somebody asking different questions: `BAD_DEVICE` is a device that
    /// answered about a chain nobody posted, and this is a driver that could not
    /// finish an operation whose buffer it had already handed over — a doorbell
    /// that could not be rung on an offered chain, or a frame the device never
    /// took. What makes it an *outcome* rather than a refusal is RFC 0024: past
    /// the point a chain is offered, a refusal would give a client back a buffer
    /// the device still holds, so the only honest end is to stop and give it
    /// back as a cancellation.
    pub const DEVICE_HOLDS: u64 = 8;
}
