// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where this component's needs landed, written by the frame and read by the
//! component, in one page neither of them has to guess the shape of.
//!
//! # The sixth copy of a layout constant, and the two rows that are new
//!
//! `user/virtio-blk/src/routing.rs` argues why a component is told and does not
//! compute, and the four crates after it record what writing the argument again
//! taught. [`AT`] equals every other component's and all of them equal
//! `kernel::process::BOARD`; `kernel/src/input.rs` asserts it at compile time
//! rather than saying it in a comment. RFC 0051 said at two drivers that this
//! constant belongs in `abi/`, RFC 0054 said it again at three and
//! `user/panel/src/routing.rs` said it at five. This is the sixth and the
//! argument has not improved.
//!
//! What is new is [`at::STAMP_SEED`] and [`at::STAMP_TICK_NANOS`], and together
//! they are the task. Every other field on this page tells a component where
//! something is or how much of it there is. These two tell it **what its clock
//! is**, because it has none of its own: RFC 0004 offers a component no clock,
//! `virtio_input_event` carries no timestamp, and `f_abi::input::NOT_STAMPED` is
//! refused — so the number every entry this driver submits carries has to come
//! from somewhere, and the honest place for *somewhere* is the frame, written
//! down, in the open. `crate::clock` is the module that spends a page saying
//! exactly what that number is and is not.
//!
//! A reader who wants the short version: it is virtual time, it orders events
//! and reproduces from a seed, and it is not a measurement of any duration.
//! Hardware-timestamped native input is `E5-B06`.
//!
//! # The second half is the component's, and the frame only reads it
//!
//! Offsets from [`REPORT`] up are written by the *component* and read by the
//! frame after the run. RFC 0013's *read, never delivered* — the frame watching
//! a component through memory it granted, costing the component nothing and
//! telling it nothing.

/// Which of this component's lives the frame asked for, in the low half of
/// `f_abi::door::Entry`.
///
/// Two, where the three drivers before this one have three. The missing one is
/// the escape provocation, and it is missing rather than forgotten: on those
/// drivers the escape bends an address the device will *read* or write on a
/// transfer the driver asked for, and the control that makes it a measurement is
/// a boot verb that watches the remapping unit fault. This driver has the
/// sharper version of the same provocation available to it — every buffer it
/// posts is one the device writes whenever the user acts — and no boot to run it
/// in. A third selector with nothing exercising it would be unreached code in an
/// image RFC 0100 asks to stay small, and a provocation nobody has watched fire
/// is a provocation nobody has tested. `E3-B04` is the parent line that would
/// add it with a boot behind it.
pub mod life {
    /// Announce and end. What a spawn into a place asks for.
    pub const ANNOUNCE: u32 = 0;
    /// Drain the device and submit what it reports until the frame says stop.
    pub const SERVE: u32 = 1;
}

/// Where the frame maps this page in the component's address space.
///
/// The one number every component crate holds — see the module comment — and it
/// must equal `kernel::process::BOARD`, which `kernel/src/input.rs` asserts at
/// compile time rather than saying in a comment.
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
/// that took a zero for a length would refuse rather than fault — which reads as
/// a device problem. The magic makes *the frame did not fill this in* a distinct
/// answer from *the frame said zero*.
///
/// Different from every other component's, and it has to be. All of them are
/// mapped at [`AT`] by the same driver shape, so a build that routed the wrong
/// image into the wrong supervisor would otherwise find a page whose magic
/// matched and whose fields meant something else. At four drivers the cheap
/// thing to get wrong is to make the fourth equal to one of the first three, and
/// `the_four_drivers_do_not_share_a_routing_magic` in `crate` is the check.
pub const MAGIC: u64 = 0x696E_705F_726F_7574;

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
    /// Routed and read by nothing, exactly as it is for all three drivers before
    /// this one, and the absence costs more here than on any of them:
    /// virtio-input's configuration space is a select/subselect window carrying
    /// the device's name, its physical identifier, and the bitmaps saying which
    /// event types and codes it can produce at all. A driver that read it would
    /// know whether it had been given a mouse or a keyboard before the first
    /// event arrived. `crate::transport` states what not reading it costs and why
    /// the cost is paid; the short version is that this driver decides what a
    /// record means from the record, so a device that produces something it does
    /// not translate is counted rather than mis-read.
    pub const CONFIG_OFFSET: u32 = 56;
    /// Its length. Unit: bytes.
    pub const CONFIG_LEN: u32 = 64;
    /// How far apart two queues' doorbells are inside the notification
    /// structure, as the device reported it.
    /// Unit: bytes per queue index. Zero is legal.
    ///
    /// Read and used even though this driver rings exactly one doorbell, for the
    /// display driver's reason unchanged: the doorbell it rings is queue
    /// **zero's**, and a multiplier of zero and a multiplier of four put it in
    /// the same place only for queue zero. A driver that ignored the field would
    /// work here and break the moment it touched the status queue.
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
    /// Where the ring this component **submits** on is.
    ///
    /// The component holds the client's end and its peer holds the server's,
    /// which is the opposite of the three drivers before it and is what an input
    /// driver is: nobody asks for an input event. `user/panel` is the other
    /// component in this position.
    /// Unit: bytes, in the component's address space.
    pub const DATA_AT: u32 = 136;
    /// How many bytes of it. Unit: bytes.
    pub const DATA_LEN: u32 = 144;
    /// The ABI version the frame negotiated on this component's behalf.
    /// Unit: none.
    pub const NEGOTIATED_VERSION: u32 = 152;
    /// The feature set beside it, whole.
    /// Unit: none — a bitmask of `f_abi::feature` constants.
    pub const NEGOTIATED_FEATURES: u32 = 160;
    /// The class this component was admitted for, from its manifest's
    /// `[reservation] class` by way of `f_abi::manifest::class::admitted`.
    ///
    /// Every entry this component submits carries it and none carries anything
    /// more urgent — RFC 0025 bound 2, read from the direction a *submitter* is
    /// on. The three drivers before this one read the same word to clamp what
    /// they serve; this one reads it to decide what it may claim, which is the
    /// same rule and the other end of it.
    /// Unit: none — an `f_abi::class` ordinal.
    pub const ADMITTED: u32 = 168;
    /// How many turns of its loop the component will spend with nothing on
    /// either ring and nothing from the device before it stops.
    ///
    /// **Load-bearing rather than a backstop**, which is the network driver's
    /// position and not the display driver's: nothing owes this component an
    /// event. A user who is not touching anything produces nothing, for as long
    /// as they like, and there is no answer outstanding whose absence would be a
    /// failure. So a run that ends here ended the ordinary way, and
    /// [`super::stopped::IDLE`] says so rather than pretending it was told.
    ///
    /// RFC 0046 — a hang is a count and a loop with no bound is a hang with an
    /// explanation. A count and not a duration, and here the reason is stronger
    /// than RFC 0004's usual one: this component *does* hold a clock, and it is
    /// virtual, so a duration measured against it would be a count wearing a
    /// unit. Unit: turns.
    pub const IDLE_SPINS: u32 = 176;
    /// The seed this component's clock is built from.
    ///
    /// Told rather than chosen, so that a run is reproducible from something the
    /// frame decided and a reader can find. `crate::clock` is where what this
    /// clock is — and what it is not — is written down at length.
    /// Unit: none — a seed.
    pub const STAMP_SEED: u32 = 184;
    /// How far this component's clock advances per report drained from the
    /// device.
    ///
    /// Zero is refused rather than defaulted: a clock that does not move stamps
    /// every event in a run alike, and every stage downstream would read a queue
    /// of events that all happened at once. `crate::component` refuses it where
    /// it refuses every other field it cannot state, before the device is
    /// started. Unit: nanoseconds per report.
    pub const STAMP_TICK_NANOS: u32 = 192;
    /// Where the pointer starts, along x: the accumulator's first value.
    ///
    /// **Told, because a relative device cannot supply it** — `crate::driver`'s
    /// *origin of the pointer* is the argument — and told by the frame, which
    /// is the one party that can then commit a transform there without having
    /// been told where the pointer is since. Zero is an ordinary place and is
    /// taken; what is refused is a word that is not an `i32` written as two's
    /// complement, since the accumulator is one.
    ///
    /// *Why it exists at all:* `E3-B01i`'s boot committed the pointer at the
    /// origin the driver could not be told about, `(0, 0)`, and over a commit
    /// of zero a latch that **added** its position to the committed translation
    /// submits the same frame as one that replaced it. A boot that starts the
    /// pointer anywhere else tells the two apart. RFC 0132.
    /// Unit: device pixels, scaled by 65 536, as two's complement in a word.
    pub const ORIGIN_X_X65536: u32 = 200;
    /// The same along y. Unit: as [`ORIGIN_X_X65536`].
    pub const ORIGIN_Y_X65536: u32 = 208;
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
    /// `Counters::records` — `virtio_input_event` records read off the device.
    /// Unit: records.
    pub const RECORDS: u32 = super::REPORT + 8;
    /// `Counters::reports` — `EV_SYN` boundaries, which is how many times the
    /// device said *that is one thing the user did*.
    ///
    /// Beside [`RECORDS`] rather than derived from it, because the ratio is the
    /// number a reader wants: a mouse moving reports two records per report and
    /// a keyboard one, so a run whose two counts are equal is a run that saw no
    /// motion. Unit: reports.
    pub const REPORTS: u32 = super::REPORT + 16;
    /// `Counters::stamped` — reports that took a stamp.
    ///
    /// **Required to equal [`REPORTS`]**, and published separately for exactly
    /// the reason `E3-B04a` exists: *every event is stamped once* is a claim, and
    /// a claim with one count behind it cannot be checked. Two counts that must
    /// agree can. Unit: reports.
    pub const STAMPED: u32 = super::REPORT + 24;
    /// `Counters::submitted` — entries this component put on its data ring.
    /// Unit: entries.
    pub const SUBMITTED: u32 = super::REPORT + 32;
    /// `Counters::dropped` — entries that were built and could not be submitted
    /// because the peer's ring was full.
    ///
    /// The counter the three drivers before this one have no counterpart for,
    /// and it is the price of being a producer nobody asked: a server with a full
    /// completion ring has a client that will notice, and an input event nobody
    /// is waiting for simply never happened as far as every later stage is
    /// concerned. Published rather than swallowed, because a compositor that
    /// seems to have missed a keystroke and one that was never sent it are the
    /// same symptom with different repairs. Unit: entries.
    pub const DROPPED: u32 = super::REPORT + 40;
    /// `Counters::ignored` — records this build does not translate.
    ///
    /// `EV_ABS`, `EV_MSC`, and every other evdev type this driver has no opcode
    /// for. Counted rather than refused, which is the one place this component
    /// deliberately does not apply R04's *refuse what you do not know*: a device
    /// producing a record type this build has no `f_abi::input` opcode for is not
    /// a peer sending nonsense, it is a device doing more than this driver was
    /// written for, and stopping the input of a machine over it would be worse
    /// than the absence. The number is what makes the absence visible.
    /// Unit: records.
    pub const IGNORED: u32 = super::REPORT + 48;
    /// `Counters::malformed` — entries this component built that its own format
    /// check refused.
    ///
    /// **Required to be zero.** It is the encoder checked against the decoder
    /// inside the component that owns the encoder: `f_abi::input::Event::check`
    /// is the same function a consumer's `decode` runs, so a non-zero here is
    /// this driver about to put an entry on a ring that the peer will refuse, and
    /// it is counted at the one place that can still tell which record produced
    /// it. Unit: entries.
    pub const MALFORMED: u32 = super::REPORT + 56;
    /// How far this component's clock was advanced by the end of the run.
    ///
    /// `crate::clock::Interrupt::at_nanos`, which is the tick times the number of
    /// reports and is therefore checkable from [`REPORTS`] and the routing page's
    /// own `STAMP_TICK_NANOS` — the point being that it *is* checkable, so a
    /// build whose clock had come unstuck from its reports is a boot that says
    /// so. Unit: nanoseconds of virtual time.
    pub const CLOCK_AT: u32 = super::REPORT + 64;
    /// What stopped the component, as one of the [`stopped`](super::stopped)
    /// constants. Unit: none — an ordinal.
    pub const OUTCOME: u32 = super::REPORT + 72;
    /// `Counters::spun` — turns of the loop that found nothing anywhere.
    /// Unit: turns.
    pub const SPUN: u32 = super::REPORT + 80;
    /// `Driver::crossing` — what this component put on the data ring, folded
    /// into one word by `f_abi::input::Crossing`.
    ///
    /// **The producer's half of the attestation, and the only field on this
    /// page that is about what the consumer received rather than about what
    /// this component did.** Every other number here is a tally the frame reads
    /// to learn what happened inside the driver; this one exists to be compared
    /// against a word the frame folded itself, out of the entries that arrived,
    /// with neither side holding the other's copy.
    ///
    /// Not a counter and therefore not a `[[state]]` row — `Driver::crossing`
    /// is where that is argued. It sits beside [`CLOCK_AT`], which is the other
    /// field here that is a value rather than a tally.
    /// Unit: none — a checksum.
    pub const CROSSING: u32 = super::REPORT + 88;
    /// How many entries went into [`CROSSING`].
    ///
    /// Published beside it rather than left to be inferred from [`SUBMITTED`],
    /// and the two are not the same claim: `SUBMITTED` is what this component
    /// counted, and this is what the fold absorbed. A driver whose two
    /// disagree has a submit path that counts an entry it did not fold or folds
    /// one it did not send, and that is worth a distinct sentence rather than
    /// being hidden inside a word that would simply not match.
    /// Unit: entries.
    pub const CROSSED: u32 = super::REPORT + 96;
    /// One where [`CROSSING`] also went onto the data ring as an attestation,
    /// after the last event, and zero where it did not.
    ///
    /// `E3-B04g`. The consumer is a component now and not the frame, it never
    /// sees this page, and on one worker core it runs after the frame has taken
    /// this page back — so the word travels on the ring the entries travelled
    /// on, under `f_abi::input::ATTEST`. Published here so that a consumer
    /// that finds no attestation and a driver that never sent one are two
    /// different sentences in the boot log. Unit: none — a flag.
    pub const ATTESTED: u32 = super::REPORT + 104;
    /// How many of the entries that crossed were pointer motion.
    ///
    /// A tally taken where [`CROSSING`] is taken, and for its reason: it is the
    /// number of positions the consumer should have been handed, stated by the
    /// side that sent them, so a boot can check the consumer's count without
    /// asking the consumer. Unit: entries.
    pub const MOTIONS: u32 = super::REPORT + 112;
    /// Where the accumulator ended up, along x.
    ///
    /// The number the harness that moved the pointer checks its own injection
    /// against, and the number the frame checks a compositor's latched
    /// position against. It was the frame's until `E3-B04g`, taken from the
    /// entries it drained; the frame drains nothing now, so it is the driver's
    /// own word, from the far side of the emulator from the harness and the far
    /// side of a ring from the compositor. Two's complement in a `u64` word.
    /// Unit: device pixels from the origin, scaled by 65 536.
    pub const POINTER_X_X65536: u32 = super::REPORT + 120;
    /// And along y. See [`POINTER_X_X65536`].
    /// Unit: device pixels from the origin, scaled by 65 536.
    pub const POINTER_Y_X65536: u32 = super::REPORT + 128;
}

/// Why the component's loop ended.
///
/// Written into [`reported::OUTCOME`] so that a boot can tell a driver that
/// drained its device and was told to stop from one that fell out of its loop
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
    /// The device published a used element naming a chain this driver never
    /// posted, or one shorter than the record it was required to write.
    pub const BAD_DEVICE: u64 = 6;
    /// The loop found nothing for [`super::at::IDLE_SPINS`] turns.
    ///
    /// **An ordinary ending and not a failure**, which is the network driver's
    /// position: nothing owes this component an event, so a user who stopped
    /// touching the machine produces exactly this. It is its own outcome rather
    /// than folded into [`TOLD`] because a boot that stopped serving and a user
    /// who stopped typing are different events, and a verb that could not tell
    /// them apart would pass on a frame that had gone away.
    pub const IDLE: u64 = 7;
    /// The routing page named a clock this component cannot hold — a tick of
    /// zero.
    ///
    /// Its own outcome rather than [`BAD_ROUTING`], because it is the one field
    /// on that page whose wrong value produces a run that *works*: every entry
    /// would carry the same stamp, every stage downstream would read them as
    /// simultaneous, and nothing would fail. A boot that could not distinguish
    /// *the frame wrote a nonsense address* from *the frame wrote a clock that
    /// does not move* would be reading the second as the first and repairing the
    /// wrong thing. `crate::clock` is the argument.
    pub const NO_CLOCK: u64 = 8;
}
