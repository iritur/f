// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where this component's needs landed, written by the frame and read by the
//! component, in one page neither of them has to guess the shape of.
//!
//! # Why a component is told and does not compute
//!
//! `user/virtio-blk/src/routing.rs` makes the argument in full and
//! `user/virtio-net/src/routing.rs` records what writing it a second time
//! taught. This file adds one sentence to that record and does not repeat
//! either: **the constant is now held by three crates and the frame carries
//! three assertions**, which is the shape RFC 0051 said should move into `abi/`
//! at three. It has not moved, and RFC 0054 says why and who owes it.
//!
//! [`AT`] equals `f_virtio_blk::routing::AT` and `f_virtio_net::routing::AT`,
//! and all three equal `kernel::process::BLK_BOARD`. That is not sharing and it
//! is not a coincidence: the frame builds one driver shape, whose address plan
//! reserves a text region, a stack, a control ring, a data ring, a board, four
//! register pages and a queue region, and every driver scheduled into that shape
//! finds its board at the same address because there is one shape.
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
//! remapping unit's own fault-recording registers, and — this is the one that is
//! new at the third driver — it cannot lie about **what is on the screen**,
//! because the screen is on the other side of the emulator and the harness reads
//! it there. `cargo xtask gpu` turns on exactly that: a picture captured from
//! outside the machine, which no counter in this page can produce.

/// Which of this component's lives the frame asked for, in the low half of
/// `f_abi::door::Entry`.
///
/// Three, and the third is a provocation rather than a mode. A selector this
/// build does not name falls through to [`ANNOUNCE`], which is the life a
/// *spawn* into a place asks for — so a frame that forgot to set one gets an
/// announcement rather than a driver that drives nothing.
///
/// There is deliberately **no fourth selector for the negative control**. The
/// control this task needs is *the same driver, with the client asking for
/// nothing*, and that is a fact about the client rather than about the
/// component — a selector for it would be a driver with a mode in which it shows
/// nothing, which is a different experiment wearing the control's name.
pub mod life {
    /// Announce and end. What a spawn into a place asks for.
    pub const ANNOUNCE: u32 = 0;
    /// Serve the data ring until the frame says stop.
    pub const SERVE: u32 = 1;
    /// The same, and add [`at::BEYOND`](super::at::BEYOND) to the address a
    /// registration answered before it becomes the backing entry the device
    /// reads a frame out of.
    ///
    /// A separate selector rather than a flag, for the reason
    /// `Driver::provoke_escape` is a separate entry point: the provocation has
    /// to be greppable, and a driver whose data path took a branch on a mode
    /// word would be a data path with a provocation in it.
    pub const ESCAPE: u32 = 2;
}

/// Where the frame maps this page in the component's address space.
///
/// The one number all three driver crates hold — see the module comment — and it
/// must equal `kernel::process::BLK_BOARD`, which `kernel/src/gpu.rs` asserts at
/// compile time rather than saying in a comment.
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
/// Different from both of the others, and it has to be. All three components are
/// mapped at [`AT`] by the same driver shape, so a build that routed the wrong
/// image into the wrong supervisor would otherwise find a page whose magic
/// matched and whose fields meant something else. One constant apart is what
/// makes that a refusal instead, and at three drivers the cheap thing to get
/// wrong is to make the third equal to one of the first two.
pub const MAGIC: u64 = 0x6770_755F_726F_7574;

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
    /// Routed and read by nothing, exactly as it is for both other drivers, and
    /// the absence is worth a sentence here because a display device's
    /// configuration structure is the one place a reader would expect a driver
    /// to look: it carries `num_scanouts` and the pending-events word.
    /// `crate::transport` says what not reading it costs and why the cost is
    /// paid.
    pub const CONFIG_OFFSET: u32 = 56;
    /// Its length. Unit: bytes.
    pub const CONFIG_LEN: u32 = 64;
    /// How far apart two queues' doorbells are inside the notification
    /// structure, as the device reported it.
    /// Unit: bytes per queue index. Zero is legal.
    ///
    /// Read and used even though this driver rings exactly one doorbell, because
    /// the doorbell it rings is queue **zero's** and a multiplier of zero and a
    /// multiplier of four put it in the same place only for queue zero. A driver
    /// that ignored the field would work here and break the moment it touched
    /// the cursor queue, which is the sort of latent wrongness that is cheaper to
    /// not write than to find.
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
    /// life points the device, in the backing entry it reads a frame out of.
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
    /// How many turns of its loop the component will spend with nothing on
    /// either ring before it stops.
    ///
    /// **A backstop and not a mechanism**, which is the difference between this
    /// field and the network driver's `RECEIVE_SPINS` even though the two look
    /// identical. There, the bound is load-bearing: nothing owes a driver a
    /// packet, so the bound is what turns a wait into a count. Here every
    /// command this driver sends is owed an answer and the frame's own stop
    /// notice is what ends the loop, so a run that reaches this number is a run
    /// where the *frame* stopped serving — a different failure, wanting a
    /// different answer, and one this component cannot diagnose.
    ///
    /// It exists anyway because RFC 0046 says a hang is a count and a loop with
    /// no bound is a hang with an explanation. A count and not a duration,
    /// because RFC 0004 offers a component no clock. Unit: turns.
    pub const IDLE_SPINS: u32 = 200;
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
    /// `Counters::escaped`. Unit: backing entries.
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
    /// `Counters::shown` — frames flushed to a scanout. Unit: frames.
    pub const SHOWN: u32 = super::REPORT + 88;
    /// `Counters::commands` — display commands the device answered.
    /// Unit: commands.
    pub const COMMANDS: u32 = super::REPORT + 96;
    /// `Counters::declined` — commands the **device** answered with something
    /// other than success. Unit: commands.
    ///
    /// Beside [`REFUSED`] and never folded into it, and the distinction is the
    /// one this device makes available and neither of the others does: a block
    /// device answers a status byte and a network device answers nothing at all,
    /// while a display controller answers every command with a typed response
    /// naming what it disagreed with. A boot that could not tell *this driver
    /// refused a client* from *the display refused this driver* would be a boot
    /// reading its own arithmetic where a device's word is.
    pub const DECLINED: u32 = super::REPORT + 104;
    /// `Counters::resources` — resources created and never freed.
    /// Unit: resources.
    ///
    /// Published because it is a **cost** and R12 says a concession is written
    /// as a cost rather than hidden in a metric. This driver sends no
    /// `RESOURCE_UNREF`, for the reason `crate::driver` argues at
    /// `RESOURCES_MAX`, so this number is the display memory a long-running
    /// client would eventually exhaust.
    pub const RESOURCES: u32 = super::REPORT + 112;
    /// `Counters::spun` — turns of the loop that found nothing on either ring.
    /// Unit: turns.
    pub const SPUN: u32 = super::REPORT + 120;
    /// `Counters::halted` — operations that failed while the device still held
    /// a client's buffer as backing. Unit: operations.
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
    /// posted, or answered a command it was never sent.
    pub const BAD_DEVICE: u64 = 7;
    /// An operation failed while the device still held a client's buffer as the
    /// backing of a resource, and the detach that would have taken it back
    /// failed too.
    ///
    /// Its own outcome and not [`BAD_DEVICE`], because the two are read by
    /// somebody asking different questions. What makes it an outcome rather than
    /// a refusal is RFC 0024: a client told its buffer is its own again, while a
    /// display controller still holds a mapping it may read on its next refresh,
    /// has been told something false. `crate::driver::Driver::stopped` is where
    /// that is decided and `kernel/src/gpu.rs` is what requires it never to
    /// happen.
    pub const DEVICE_HOLDS: u64 = 8;
    /// The loop found nothing on either ring for [`super::at::IDLE_SPINS`]
    /// turns.
    ///
    /// Its own outcome rather than [`TOLD`], and that is the whole reason the
    /// bound is a backstop rather than a mechanism: on this driver a run that
    /// ends here is a run where the frame stopped serving, which is a different
    /// event from a run that was told to stop and must not be reported as one.
    /// The network driver folds the same bound into `TOLD` because there it *is*
    /// the ordinary ending.
    pub const IDLE: u64 = 9;
}
