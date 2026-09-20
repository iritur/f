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
    /// Announce and end.
    ///
    /// What `component::demonstrate` spawns into a place, and — since the
    /// `blk=place` half — what it then hands a core. So this selector is no
    /// longer only the life a spawn *names*: it is the one a place's occupant
    /// has actually run, which is why it stays a selector of its own rather
    /// than becoming the absence of one.
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
    /// Read the device's own configuration window, report what it said, and
    /// end.
    ///
    /// # What this life is for, and what it is not
    ///
    /// It is the narrowest thing a component can do with a register window: no
    /// queue, no ring, no client, no device reset. It reads the capacity the
    /// device published when the machine was built and writes it into
    /// [`super::reported::CAPACITY`].
    ///
    /// **It exists because a place's occupant can now run.** The frame supplies
    /// that place with a device window it cannot carve — `kernel/src/component.
    /// rs`'s `Supplied` — and the log line saying so is the *frame's* account of
    /// what it did. This is the component's, and the two are different claims: a
    /// window that was mapped into the wrong address space, at the wrong
    /// address, or uncached at neither would produce the same frame line and a
    /// different number here.
    ///
    /// A separate selector rather than a flag on [`SERVE`], for [`ESCAPE`]'s
    /// reason one line up.
    pub const IDENTIFY: u32 = 3;
}

/// Where the frame maps this page in the component's address space.
///
/// The one number both sides hold. It must equal `kernel::process::BOARD`,
/// and the kernel asserts that at compile time rather than saying it in a
/// comment — `kernel/src/blk.rs` holds the assertion, because the kernel is the
/// artefact that links both definitions and a comment is not a check.
///
/// Unit: bytes, in the component's own address space.
pub const AT: u64 = 0x0041_8000;

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

    // --- what E1-B06 added: the order, and what bounds it ---------------------
    //
    // Five slots and every one of them is *told* rather than assumed, for the
    // reason the module comment already gives about addresses and the sharper
    // one RFC 0025 gives about ceilings: a component that wrote down its own
    // admitted class would be a component that could raise it, which is the
    // whole of bound 2 undone by a constant.

    /// Which order the driver hands work to the device in, as a
    /// `crate::pending::Order` ordinal — zero is arrival, one is rank.
    ///
    /// Zero is the order that claims nothing, so a frame that did not fill this
    /// in gets a driver that reorders nothing rather than one that reorders by
    /// a field it was never told to trust. R04.
    /// Unit: none — an ordinal.
    pub const ORDERING: u32 = 176;
    /// The class this component was admitted for, from its manifest's
    /// `[reservation] class` by way of `f_abi::manifest::class::admitted`.
    /// A request is never served above it — RFC 0025 bound 1.
    /// Unit: none — an `f_abi::class` ordinal.
    pub const ADMITTED: u32 = 184;
    /// The class the *channel* reports about whoever submits on it. An entry
    /// claiming anything more urgent is refused `ADMISSION`/`NOT_HELD` — RFC
    /// 0025 bound 2 — and it is here, in the page the frame writes, precisely
    /// so that it is never a field of an entry.
    /// Unit: none — an `f_abi::class` ordinal.
    pub const CLIENT_ADMITTED: u32 = 192;
    /// The least time this component needs from arrival to completion for any
    /// request — RFC 0025 bound 3.
    /// Unit: nanoseconds.
    pub const FLOOR: u32 = 200;
    /// How many requests the driver accumulates before it makes its first
    /// choice among them. Unit: requests. Zero is a driver that serves whatever
    /// it has.
    ///
    /// **A fixture, and it is the honest name for it.** An overtake is only
    /// observable when there is something to overtake, and what is queued at
    /// the moment of a pick otherwise depends on how two cores raced — so a
    /// boot that submitted a burst and hoped would report a different number
    /// every run and would sometimes report the right one for the wrong reason.
    /// This makes the queue's contents at the first pick a fact the frame
    /// chose, which is what turns *the read overtook some batch work* into *the
    /// read overtook six*. It is told to the component for the same reason
    /// [`BEYOND`] is: a component choosing its own would be a demonstration
    /// choosing its own difficulty.
    pub const HOLD: u32 = 208;
    /// How many requests the driver serves normally before [`HOLD`] applies at
    /// all. Unit: requests.
    ///
    /// A client has to register a buffer set and put something on the disk
    /// before it can read it back, and a hold that caught those would be a
    /// client waiting for a completion the driver is holding for a burst the
    /// client has not sent yet — a deadlock arrived at through a fixture. So
    /// the frame says how long its own prelude is, and the hold arms after it.
    pub const HOLD_AFTER: u32 = 216;

    // --- what a generation swap is told, and it is told before it runs -------
    //
    // **The frame cannot ask a running component anything.** R05 is why: nothing
    // is delivered to a component while it holds a core, so there is no message
    // that means *stop at your next quiescent point*. What there is instead is
    // this page, written before the core is given out and read by the component
    // whenever it likes — which is exactly the arrangement every other field
    // here has, and is why the swap protocol needs no new opcode and no new
    // ring.
    //
    // `user/virtio-blk/manifest.toml` predicted the point this is read at, in
    // advance and by name: *the driver needs a point in its own loop where it
    // holds nothing, and it has one already — at the top of that loop
    // `Pending::is_empty()` is the whole of what this component has accepted and
    // not answered.* RFC 0063.

    /// Where the transfer window is in this component's address space, or zero
    /// for an instance nobody is swapping.
    ///
    /// Bought out of the **incoming** instance's account, never the outgoing
    /// one's — `abi/src/swap.rs` is explicit that the account which pays is the
    /// account which survives — and mapped into both for the length of phase A.
    /// Unit: bytes.
    pub const WINDOW_AT: u32 = 224;

    /// How many bytes of it. Unit: bytes.
    pub const WINDOW_LEN: u32 = 232;

    /// Non-zero when this instance is being asked to hand over.
    ///
    /// Read at the quiescent point and nowhere else. An instance told this
    /// writes its journal into the window, reports what it wrote, and ends —
    /// which is the *outgoing* half of RFC 0063's phase A.
    /// Unit: none — a flag.
    pub const HAND_OVER: u32 = 240;

    /// How many records are waiting in the window for this instance to replay.
    ///
    /// Read once, before the first entry is taken off any ring, which is the
    /// *incoming* half. Zero for an instance that is not succeeding anybody,
    /// which is every instance in every boot that is not swapping.
    /// Unit: records.
    pub const REPLAY: u32 = 248;

    /// Where this instance's own state tree is mapped, or zero if it has none.
    ///
    /// **Zero is the common case and is why this is a slot rather than a
    /// constant.** The tree page exists only on the `component::spawn` path:
    /// `process::prepare_driver` and `process::prepare_server` map the control
    /// ring, the board, the registers, the queues and the heap, and not this.
    /// A component that assumed the address would fault at ring 3 on every
    /// boot that is not a place, with nothing in the fault naming the cause.
    /// So the frame says where it is, or says there is none, and the component
    /// publishes only when told.
    ///
    /// Unit: bytes — a virtual address in this instance's own space.
    pub const TREE_AT: u32 = 256;
}

/// How much of the state-tree page this component may write.
///
/// One frame, which is what `component::spawn` maps at `process::SPAWN_TREE`
/// and what `process::MODULE_MAX`'s neighbours are all sized in. It is here
/// rather than in the component because it is a fact about the mapping the
/// frame makes, and the component is the party that must not invent it.
/// Unit: bytes.
pub const TREE_BYTES: u32 = 4096;

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

    // --- what E1-B06 added: what the ordering did ----------------------------

    /// `Counters::shortfall`. Unit: completions.
    pub const SHORTFALL: u32 = super::REPORT + 80;
    /// `Counters::unadmitted`. Unit: entries.
    pub const UNADMITTED: u32 = super::REPORT + 88;
    /// `Pending::overtaken` — waiting requests that something more urgent was
    /// handed to the device ahead of. Unit: requests.
    ///
    /// The component's own reading of what its ordering did. The frame has a
    /// second and independent one — the order the completions came back in,
    /// observed in its own address space — and `kernel/src/blk.rs` requires the
    /// two to agree. A counter a component increments is a counter that
    /// component could be wrong about, which is the same reason `drained` sits
    /// beside `served` rather than being derived from it.
    pub const OVERTAKEN: u32 = super::REPORT + 96;
    /// `Pending::deepest` — the most requests that were ever waiting at once.
    /// Unit: requests.
    ///
    /// What the overtake above was performed *out of*. An overtake count with
    /// no queue depth beside it cannot be told from a queue that was never
    /// deep, which is the same defect `provoked` takes out of the copy counter.
    pub const QUEUED_MAX: u32 = super::REPORT + 104;
    /// `crate::pending::IN_FLIGHT` — how many requests were inside the device
    /// at once, and therefore how coarse every overtake above is.
    /// Unit: requests.
    ///
    /// Published because it is the *cost* of the number beside it and R12 says
    /// a concession is written as a cost rather than hidden in a metric: a
    /// request already handed to the device cannot be overtaken by anything.
    pub const IN_FLIGHT: u32 = super::REPORT + 112;
    /// The `crate::pending::Order` ordinal the component actually used.
    /// Unit: none — an ordinal.
    ///
    /// Reported rather than assumed equal to what the frame wrote in
    /// [`super::at::ORDERING`], because they are two different claims: one is
    /// what the boot asked for and the other is what a component that may have
    /// read its routing page wrongly did. A control run that silently used the
    /// ordering it was supposed to be a control for would pass every check that
    /// compares the two halves.
    pub const ORDERED: u32 = super::REPORT + 120;

    /// Whether this instance held nothing it had accepted and not answered at
    /// the moment it was asked to hand over.
    ///
    /// **The occupant's own assertion, and the half no cursor can supply.** RFC
    /// 0018's cursors say the rings are empty; they cannot say the *driver* is,
    /// because an entry taken off a ring and not yet answered is in neither. The
    /// frame asks this only after its own ring-empty check has agreed, so an
    /// instance that answered yes unconditionally still could not be swapped
    /// with work on the wire.
    /// Unit: none — a flag.
    pub const QUIESCENT: u32 = super::REPORT + 128;

    /// How many records this instance wrote into the transfer window.
    /// Unit: records.
    pub const RECORDS: u32 = super::REPORT + 136;

    /// How many records the incoming instance replayed into its own table.
    ///
    /// Counted on this side of the boundary and compared against what the
    /// outgoing instance said it wrote. Two tallies of one number, neither
    /// derived from the other, which is `claims/0012`'s discipline and the
    /// reason a swap can say *nothing was lost* rather than *nothing was
    /// reported lost*.
    /// Unit: records.
    pub const REPLAYED: u32 = super::REPORT + 144;

    /// Which build of this component wrote this page.
    ///
    /// One for the ordinary image and two for the one built with the
    /// `successor` feature. It is the only difference between them, and its
    /// whole purpose is to let a boot say *the occupant of this place is not the
    /// one that was in it a moment ago* — which a content address in the frame's
    /// log also says, but from the frame's side. This is the component's own
    /// answer, and a swap that reported one and not the other would be a swap
    /// nobody had checked from both ends.
    /// Unit: none — an ordinal.
    pub const GENERATION: u32 = super::REPORT + 152;

    /// Non-zero once this instance has finished replaying and **before it has
    /// served anybody**.
    ///
    /// **A count is not enough and that is the whole reason this exists.**
    /// [`REPLAYED`] is a number, and zero is both *replayed nothing* and *has
    /// not started* — which are the two cases a frame most needs to tell apart,
    /// because the first is a swap that must be abandoned and the second is a
    /// swap still in progress. So the instance writes this after the replay and
    /// before the first entry it takes off any ring, and a frame that sees it
    /// knows the number beside it is final.
    ///
    /// RFC 0063 has the incoming instance acknowledge before it serves anybody.
    /// This word is what makes that orderable from the frame's side: it can
    /// read the count, acknowledge or abandon, and only then let a client
    /// submit — so a swap that fails costs the client latency and nothing else.
    /// Unit: none — a flag.
    pub const REPLAY_DONE: u32 = super::REPORT + 160;
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
    /// The component was asked only to identify its device, and did.
    ///
    /// An outcome of its own rather than [`TOLD`], because *it ran to the end
    /// of what it was asked* and *a stop notice arrived* are two different
    /// things, and a reader who could not tell them apart could not tell an
    /// `IDENTIFY` run from a `SERVE` run that was stopped before it served
    /// anything.
    pub const IDENTIFIED: u64 = 7;

    /// The instance reached a quiescent point, wrote its journal into the
    /// transfer window, and ended so that its successor could take the place.
    ///
    /// Distinct from [`TOLD`], which is an instance that was asked to stop and
    /// did. A swap is not a stop: the place keeps its clients, its endpoint and
    /// its reservation, and what ends is one occupant of it. RFC 0012 requires
    /// the two never be summed and this is where they stop being the same word.
    pub const HANDED_OVER: u64 = 8;

    /// The instance was asked to hand over and could not honestly do it.
    ///
    /// Its journal had overflowed, so the history it would have handed on is
    /// shorter than the history it lived. The frame abandons the swap and the
    /// place restarts, which costs every client its registrations and costs
    /// nothing else — and is strictly better than a successor that believes it
    /// inherited a table it did not.
    pub const CANNOT_HAND_OVER: u64 = 9;
}
