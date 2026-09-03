// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where this component's needs landed, written by the frame and read by the
//! component, in one page neither of them has to guess the shape of.
//!
//! # Why a component is told and does not compute
//!
//! `f_abi::door::Entry` already makes the argument for the *handles*: the frame
//! tells a component what it holds rather than letting it write the indices
//! down, because a second occupant of a place finds the same indices at a later
//! generation. The addresses are the same argument one layer out and it is
//! sharper, because two of them cannot be constants at all:
//!
//! - **The device's register structures are wherever the device says they
//!   are.** A modern virtio transport publishes four structures inside a
//!   base-address register at offsets and lengths the *device* chooses, and the
//!   notification multiplier is a number the device reports. A driver that
//!   hard-coded QEMU's layout would be a driver bound to one emulator.
//! - **The device address of its own queue memory is what a translation
//!   answered.** `kernel/src/iommu.rs` makes it the identity of a physical
//!   address today and writes down the reversal; a driver that assumed the
//!   identity would break on the day that changes, silently, by pointing a
//!   device somewhere plausible.
//!
//! So one page is mapped into the component, the frame fills it in before the
//! first instruction runs, and the component reads it through the same
//! bounds-checked accessor it reads a device register through. **Exactly one
//! address is agreed by both sides as a constant** — [`AT`] — and everything
//! else is data. That is the smallest surface a shared layout can have, and it
//! is deliberately not zero: something has to be first.
//!
//! # The second half is the component's, and the frame only reads it
//!
//! Offsets from [`REPORT`] up are written by the *component* and read by the
//! frame after the run. That is RFC 0013's *read, never delivered* used the way
//! `kernel/src/runtime.rs` already uses it — the frame watching a component
//! through memory it granted, costing the component nothing and telling it
//! nothing. It is how `blk/copies` reaches a boot log now that the code
//! producing it is on the other side of a privilege boundary: the counter is
//! the component's own and the frame reads it rather than being handed it.
//!
//! A component that scribbles this half lies about its own counters and about
//! nothing else. It cannot lie about whether the device faulted, which is the
//! remapping unit's own fault-recording registers, and it cannot lie about
//! whether the client's bytes match, which the client checks in its own memory.
//! Those two are what the halves of `cargo xtask blk` actually turn on.

/// Which of this component's lives the frame asked for, in the low half of
/// `f_abi::door::Entry`.
///
/// Three, and the third is a provocation rather than a mode. A selector this
/// build does not name falls through to [`ANNOUNCE`], which is the life this
/// component has always had and the one a *spawn* into a place still asks for —
/// so a frame that forgot to set one gets an announcement rather than a driver
/// that drives nothing.
pub mod life {
    /// Announce and end. What `component::demonstrate` spawns into a place.
    pub const ANNOUNCE: u32 = 0;
    /// Serve the data ring until the frame says stop.
    pub const SERVE: u32 = 1;
    /// The same, and add `beyond` to the address a registration answered
    /// before it becomes a descriptor.
    ///
    /// A separate selector rather than a flag, for the reason
    /// `Driver::provoke_escape` is a separate entry point: the provocation has
    /// to be greppable, and a driver whose data path took a branch on a mode
    /// word would be a data path with a provocation *in* it.
    pub const ESCAPE: u32 = 2;
}

/// Where the frame maps this page in the component's address space.
///
/// The one number both sides hold. It must equal `kernel::process::BLK_BOARD`,
/// and the kernel asserts that at compile time rather than saying it in a
/// comment — `kernel/src/blk.rs` holds the assertion, because the kernel is the
/// artefact that links both definitions and a comment is not a check.
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
/// that took a zero for a length would refuse rather than fault — which reads
/// as a device problem. The magic makes *the frame did not fill this in* a
/// distinct answer from *the frame said zero*.
pub const MAGIC: u64 = 0x626C_6B5F_726F_7574;

/// Byte offsets of the fields the frame writes, each a little-endian `u64`.
///
/// Slots rather than a `repr(C)` struct, because the two sides read and write
/// them through `f_ring::device::Window`, which is a bounds-checked volatile
/// accessor and not a reference — there is no struct to borrow. Eight bytes
/// each even where four would do, so that adding a field never moves one.
pub mod at {
    /// [`super::MAGIC`]. Unit: none.
    pub const MAGIC: u32 = 0;
    /// The common configuration structure, as an offset into the register
    /// window the frame mapped. Unit: bytes.
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
    pub const CONFIG_OFFSET: u32 = 56;
    /// Its length. Unit: bytes.
    pub const CONFIG_LEN: u32 = 64;
    /// How far apart two queues' doorbells are inside the notification
    /// structure, as the device reported it.
    /// Unit: bytes per queue index. Zero is legal.
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
    /// Where the control ring is. Unit: bytes, in the component's address
    /// space.
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
    ///
    /// Its own slot rather than the high half of the one above, because a
    /// feature set is sixty-four bits wide and packing it beside a version
    /// would have been a field that silently truncated on the day a feature bit
    /// above thirty-one was defined. R03: a quantity that cannot hold its own
    /// range is a quantity that will one day be wrong quietly.
    /// Unit: none — a bitmask of `f_abi::feature` constants.
    pub const NEGOTIATED_FEATURES: u32 = 160;
    /// How far past what a registration answered the [`super::life::ESCAPE`]
    /// life points the device.
    ///
    /// Told to the component rather than chosen by it, because the frame is
    /// what knows how far outside a grant is far enough to be outside it and
    /// near enough that the remapping unit has a table to fault it in. A
    /// component that picked its own displacement would be a provocation
    /// choosing its own difficulty.
    /// Unit: bytes.
    pub const BEYOND: u32 = 168;
}

/// Where the component's own half of the page starts.
///
/// Half a page in, so that neither side can reach the other's fields by an
/// arithmetic slip of a few bytes: the frame's writes stop long before here and
/// the component's start here. It is not protection — one page is one mapping
/// and the component may write all of it — it is distance, which is what makes
/// a misplaced offset a wrong *answer* rather than a corrupted one.
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
    /// The device's capacity as the component read it out of its own
    /// configuration window. Unit: sectors.
    pub const CAPACITY: u32 = super::REPORT + 56;
    /// How many entries the component took off its data ring.
    ///
    /// Beside [`SERVED`] rather than derived from it, because they are two
    /// different claims: one is what the component's own executor counted and
    /// the other is what its loop saw arrive. A build where the loop had
    /// stopped draining publishes the same `served` as one where it never
    /// started. Unit: entries.
    pub const DRAINED: u32 = super::REPORT + 64;
    /// What stopped the component, as one of the [`stopped`](super::stopped)
    /// constants. Unit: none — an ordinal.
    pub const OUTCOME: u32 = super::REPORT + 72;
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
    /// The routing page did not carry [`super::MAGIC`], so nothing after it
    /// was believed.
    pub const NO_ROUTING: u64 = 2;
    /// An address in the routing page could not be stated as a window, a
    /// region or a channel.
    pub const BAD_ROUTING: u64 = 3;
    /// The device did not start. `Driver::start`'s own refusal.
    pub const NO_DEVICE: u64 = 4;
    /// A ring stopped validating under the component, which is a peer that has
    /// stopped speaking.
    pub const NO_RING: u64 = 5;
    /// The zero-copy self-check refused, so the zero it stands behind would
    /// have been a zero nothing could move.
    pub const NO_SELF_CHECK: u64 = 6;
}
