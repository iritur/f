// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The late latch: the pointer re-read after the commit closed, predicted
//! forward to this frame's scanout, and patched into one transform node before
//! the frame's submission crosses.
//!
//! # The window, which is the whole of the task
//!
//! `E3-B01i` asks for a moment that is **after the commit closed and before the
//! submission crosses**, and this component has exactly one such moment. It is
//! not a moment this file chose: [`crate::waits::Waits::frame`] drives section
//! 08's chain in order — the application's signal, then the compositor's
//! submission, then the present engine's — and the gap between the first and the
//! second is the only place in a frame where the client's deltas are in the
//! graph and nothing has yet left this component for the display.
//!
//! So the window is not a comment saying *here*. It is a parameter of the
//! sequence: [`crate::waits::Between`] is called by the chain driver between
//! those two calls, with the number of wait entries the frame's trace holds at
//! that instant, and this module is what is called. A caller cannot latch
//! earlier or later without moving that call, which is one line in a function
//! that has nothing else in it.
//!
//! **That number is the record's key**, and it is why the evidence for this line
//! is a frame trace rather than a log line. `abi/src/trace.rs` names its own
//! extension shape — *a second record keyed by the entry ordinal, not a field
//! here* — and [`Latched::before_entry`] is that key. Zero means the latch
//! landed before the compositor's own wait was entered, which is the window; a
//! one or a two would mean the latch had drifted past a submission, and it is a
//! number a test can assert rather than a sequence a reviewer has to re-read.
//!
//! # What *and by nothing else* rests on, stated before it is relied on
//!
//! Nothing in this file. The clause is that the submitted frame differs from the
//! committed one in the latched node's transform and nowhere else, and what
//! carries it is `f_scene::arena::Arena::set_transform`: a function whose whole
//! signature is one [`SetTransform`] record, which names one node and six
//! numbers. It cannot create, cannot remove, cannot reparent and cannot touch a
//! paint or a path, because there is no argument in which it could be told to. A
//! test asserting *only the transform moved* over a door that could do more
//! would be asserting about the code path it happened to take; over this one it
//! is asserting about the type.
//!
//! The half that is genuinely evidence is in the tests: the whole graph is
//! encoded by `f_scene::encode` either side of the latch and the two byte images
//! are compared. That is `E3-B06f`'s instrument and it is trustworthy for
//! `E3-B06f`'s reason — the encoding is the canonical form of a frame, it is
//! what stage 2 of the ladder receives, and a difference the encoding cannot see
//! is a difference no renderer can see either.
//!
//! # The patch is the submitted frame's, and it leaves with it
//!
//! The latch writes into the retained graph during the window and
//! [`LateLatch::restore`] writes the client's committed transform back once the
//! chain driver returns — after the compositor's own submission has entered its
//! wait, which is when the frame has left this component. Until 2026-09-25 there
//! was no restore, and the graph kept the patch: on the next frame without a new
//! client `SetTransform`, what `LateLatch::patch` read as *committed* was this
//! component's own previous latch, which is the comparison that function's own
//! comment forbids. An audit found it by reading; every test closed one frame.
//!
//! **Why in the graph and restored, rather than never in the graph.** The other
//! placement — the patch only in the outgoing frame — needs an outgoing frame to
//! put it in, and this component has none: nothing draws, the graph *is* what a
//! submission would be read from, and there is no heap for a second copy of an
//! arena (`routing::HEAP_BYTES` leaves 6 528 bytes). A patch held beside the
//! graph as an overlay would make every future reader of *the submitted frame*
//! remember to apply it, and would leave the boot's *and by nothing else* — the
//! fold of the graph either side of the patch — with nothing patched to fold.
//! Restoring keeps the one door (`Arena::set_transform`, one node and six
//! numbers) as the only way the latch touches anything, in both directions.
//!
//! **And why that answers `E3-B06f` rather than only this file.** That line's
//! property is that the compositor cannot tell an authored delta from a
//! projected one: the retained graph is a function of the deltas a client sent
//! and of nothing else. A graph the latch had written into and left was neither
//! authored nor projected — a third source, and the one no client could see or
//! undo. Restored after submit, the retained graph between frames is the
//! client's again, byte for byte, which
//! `a_second_frame_is_latched_against_the_clients_transform_and_not_its_own`
//! asserts across two frames.
//!
//! *What would reverse this:* a renderer that encodes the submitted frame into
//! a buffer of its own. Then the patch belongs in that buffer, is never written
//! into the graph at all, and the restore is deleted rather than kept as a
//! second guard.
//!
//! # Why this component and not `interface/`
//!
//! Because `xtask`'s `INPUT_PATH` said `interface/` was "the frame loop and the
//! late-latch" and neither of those is there: `interface/` holds a vocabulary, a
//! ladder and a solver, and it holds no arena, no chain and no frame. The latch
//! needs all three. RFC 0120 moves the row and says what reverses it.
//!
//! # The one mint, named rather than hidden
//!
//! [`Aim::scanout_nanos`] becomes a `StampNanos` here, through
//! `f_input::stamp::StampNanos::from_wire_nanos` — the constructor `xtask`'s
//! `lint-stamp` watches the argument of, because a caller handing it a clock
//! reading of its own would have minted a second time source with the type
//! system's blessing.
//!
//! This is not that, and the difference is worth the paragraph. What is minted
//! is a **target**, not an event's time: the instant this frame is aimed at,
//! which `crate::pacing::scanout_after` computed from `routing::at::TICK_NANOS`
//! — the frame's clock, written into this component's page — and the display's
//! declared period. No question about *when the pointer moved* is answered by
//! it; every such answer in this module is the driver's stamp, carried across
//! one ring unchanged and rebuilt in [`LateLatch::drained`] from the decoded
//! entry's own field — or, in a host test, [`Reading::at_nanos`].
//!
//! *What would reverse this:* a display that reports its own scanout instant. On
//! that day the number arrives off a wire like the stamp does, [`Aim`] holds
//! what it was told rather than what this component computed, and the mint goes
//! away rather than moving.
//!
//! # There is no clock here and no floating point
//!
//! A position is 16.16 fixed point with its scale in every field name, a lead is
//! nanoseconds, and an ordinal is a count. RFC 0004. The only time this module
//! holds is a number somebody else read.

use f_abi::input::{Entry, Event, NOT_STAMPED};
use f_abi::scene::{NO_NODE, SetTransform};
use f_input::predict::{Predictor, Sample};
use f_input::stamp::StampNanos;
use f_scene::arena::Arena;

/// One position the device reported, as this component is told it.
///
/// A struct rather than three arguments, and that is not style. The stamp
/// reaches `f_input` through `StampNanos::from_wire_nanos`, whose argument
/// `lint-stamp` requires to be a field read or a literal — a bare local is
/// refused, because a local is where a helper's return value lands and the whole
/// point of the rule is that a helper's return value cannot get in. So the
/// number arrives as a field and is passed as one, which is the spelling the
/// rule sanctions and also the honest one: this is a reading somebody else took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// When the device reported it, on the stamp the driver took at interrupt
    /// time and on no other clock.
    ///
    /// `f_abi::input::NOT_STAMPED` — zero — is **the absence of a reading and
    /// not a reading of zero**, and [`LateLatch::observed`] refuses it for the
    /// reason that constant's own doc gives: otherwise a page of zeroes is a
    /// pointer at the origin at the beginning of time, which is a scene a
    /// compositor would draw.
    /// Unit: nanoseconds, in the channel's epoch.
    pub at_nanos: u64,
    /// Position along x.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub x_x65536: i32,
    /// Position along y.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub y_x65536: i32,
}

/// What this frame is aimed at.
///
/// One field, and a struct for [`Reading`]'s reason exactly — see the module's
/// *the one mint*.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aim {
    /// The scanout this frame is aimed at, which `crate::pacing::Decision`
    /// computed and this module does not re-derive.
    /// Unit: nanoseconds, in the channel's epoch.
    pub scanout_nanos: u64,
}

/// Why a frame carries no latch.
///
/// Every variant is a frame that went out with the transform the client
/// committed, which is the correct behaviour in all four cases and not a
/// failure. They are separate values because a compositor permanently in one of
/// them has a different problem in each, which is `f_input::predict::Held`'s
/// argument for its own six and is the same argument here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declined {
    /// No node has been named as the one that carries the pointer. Nothing is
    /// wrong: a client that has not asked for a latched node does not get one.
    NoNode,
    /// The device has reported nothing, so there is no position to latch. A
    /// pointer that has never moved has no place the compositor knows of, and
    /// inventing the origin would be drawing a cursor the device never put
    /// there.
    NoReport,
    /// The named node is not in the graph, or holds no transform record. A latch
    /// onto a node the client removed is the case that matters, and it is
    /// declined rather than refused: the client is allowed to remove its own
    /// node and the compositor is not allowed to mind.
    NoTransform,
    /// The graph refused the patch. This is this component contradicting itself
    /// — the node was there a statement ago and the record names it — and it is
    /// counted for `crate::waits::Published::refusals`' reason: a compositor
    /// that stopped serving over its own bookkeeping would have turned an
    /// accounting defect into a frozen cursor.
    Refused,
}

impl Declined {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoNode => "no node carries the pointer, so there is nothing to latch onto",
            Self::NoReport => "the device has reported no position, so there is nothing to latch",
            Self::NoTransform => "the node the pointer rides is not in the graph with a transform",
            Self::Refused => "the graph refused a transform it had just handed over",
        }
    }
}

/// One frame's latch: what the client committed, what was submitted, and where
/// in the frame's trace the difference was made.
///
/// The two positions are kept and the difference is derived, rather than the
/// other way round. A record holding the motion and one endpoint would make the
/// exit's *by exactly the motion injected* a subtraction this file had already
/// done, and the reader checking it would be checking this file's arithmetic
/// against itself. Both ends are here, [`Latched::moved_x_x65536`] is a
/// convenience, and a test is free to ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Latched {
    /// The node whose transform was patched.
    /// Unit: none — a node identifier.
    node: u32,
    /// How many entries the frame's trace held when the latch happened.
    ///
    /// Zero is the window: after the application's signal and before the
    /// compositor's own submission entered its wait. The module header is why
    /// this is the record's key rather than a sentence in a comment.
    /// Unit: wait entries.
    before_entry: u32,
    /// The translation along x the client committed.
    /// Unit: device pixels, scaled by 65 536.
    committed_tx_x65536: i64,
    /// And along y.
    /// Unit: device pixels, scaled by 65 536.
    committed_ty_x65536: i64,
    /// The translation along x that was submitted.
    /// Unit: device pixels, scaled by 65 536.
    latched_tx_x65536: i64,
    /// And along y.
    /// Unit: device pixels, scaled by 65 536.
    latched_ty_x65536: i64,
    /// How far forward the prediction was actually extrapolated, after the
    /// predictor's own damping. Zero when the position was held.
    /// Unit: nanoseconds.
    lead_nanos: u64,
    /// Whether the position submitted is an extrapolation or the last position
    /// the device actually reported.
    ///
    /// `f_input::predict::Basis::extrapolated`, carried rather than re-derived.
    /// It is the field that tells `E3-B01i`'s case from `E3-B04e`'s: a held
    /// position differs from the committed one by exactly the motion that
    /// arrived, and an extrapolated one differs by that motion plus the lead the
    /// predictor claimed.
    /// Unit: none — a flag.
    extrapolated: bool,
    /// Everything in the graph a renderer could see, folded, with this node's
    /// two translations left out — taken before the patch.
    ///
    /// **`E3-B01i`'s *and by nothing else*, in a form a boot can carry.** The
    /// host tests hold that clause by encoding the whole frame twice and
    /// comparing bytes; a running component cannot, because the encoder's
    /// buffer is `f_scene::encode::ENCODING_MAX` — 83 992 bytes — and this
    /// component's heap is its graph and its batch with 6 528 bytes to spare
    /// (`routing::HEAP_BYTES` less `HELD_BYTES` and `HEAP_OVERHEAD`, measured
    /// on 2026-09-25 rather than read off a comment). So
    /// the latch folds every field of every node the encoding carries, walked
    /// the way the encoding walks, with the two numbers the latch is allowed to
    /// change masked out; [`Latched::unmoved_after`] is the same fold after the
    /// patch, and the two must be one word. [`unmoved`] is the argument for
    /// what is covered and what is not.
    /// Unit: none — a checksum.
    unmoved_before: u64,
    /// The same fold, taken after the patch. Unit: none — a checksum.
    unmoved_after: u64,
    /// How many nodes the second fold walked, so that a fold of an empty walk —
    /// which agrees with itself about nothing — is a number a reader can refuse.
    /// Unit: nodes.
    walked: u32,
}

impl Latched {
    /// The node whose transform was patched.
    #[must_use]
    pub const fn node(&self) -> u32 {
        self.node
    }

    /// How many entries the frame's trace held when the latch happened.
    #[must_use]
    pub const fn before_entry(&self) -> u32 {
        self.before_entry
    }

    /// The translation along x the client committed.
    #[must_use]
    pub const fn committed_tx_x65536(&self) -> i64 {
        self.committed_tx_x65536
    }

    /// The translation along y the client committed.
    #[must_use]
    pub const fn committed_ty_x65536(&self) -> i64 {
        self.committed_ty_x65536
    }

    /// The translation along x that was submitted.
    #[must_use]
    pub const fn latched_tx_x65536(&self) -> i64 {
        self.latched_tx_x65536
    }

    /// The translation along y that was submitted.
    #[must_use]
    pub const fn latched_ty_x65536(&self) -> i64 {
        self.latched_ty_x65536
    }

    /// How far forward the prediction was extrapolated. Unit: nanoseconds.
    #[must_use]
    pub const fn lead_nanos(&self) -> u64 {
        self.lead_nanos
    }

    /// Whether the submitted position is an extrapolation.
    #[must_use]
    pub const fn extrapolated(&self) -> bool {
        self.extrapolated
    }

    /// The graph, folded with this node's translation masked, before the patch.
    /// Unit: none — a checksum.
    #[must_use]
    pub const fn unmoved_before(&self) -> u64 {
        self.unmoved_before
    }

    /// The same, after the patch. Equal to [`Latched::unmoved_before`] or the
    /// latch changed something other than the two numbers it is allowed to.
    /// Unit: none — a checksum.
    #[must_use]
    pub const fn unmoved_after(&self) -> u64 {
        self.unmoved_after
    }

    /// How many nodes the fold after the patch walked. Unit: nodes.
    #[must_use]
    pub const fn walked(&self) -> u32 {
        self.walked
    }

    /// How far the latch moved the node along x.
    ///
    /// Saturating, which cannot fire for any pair of positions a device can
    /// report — the two are 16.16 values widened to `i64` — and is written
    /// rather than an unchecked subtraction because a panic in a component is a
    /// component that stops serving.
    /// Unit: device pixels, scaled by 65 536.
    #[must_use]
    pub const fn moved_x_x65536(&self) -> i64 {
        self.latched_tx_x65536.saturating_sub(self.committed_tx_x65536)
    }

    /// How far the latch moved the node along y. Unit: device pixels, scaled by
    /// 65 536.
    #[must_use]
    pub const fn moved_y_x65536(&self) -> i64 {
        self.latched_ty_x65536.saturating_sub(self.committed_ty_x65536)
    }
}

/// The pointer this component has been told about, and the node it rides.
///
/// # Why the predictor is here and not on the far side of the wire
///
/// Because a prediction is about *when this frame will be scanned out*, and the
/// input path does not know that instant. What crosses to this component is
/// positions and their stamps; what this component adds is its own scanout. A
/// build in which the driver predicted and sent a position would be a build
/// whose cursor was aimed at whatever the driver guessed the display was doing.
///
/// That is also what makes `E3-B04e`'s two records worth comparing. Both sides
/// hold a `f_input::predict::Predictor` — one implementation, two instances, fed
/// by two different pieces of code from two different states — and neither
/// number is copied from the other. What the comparison can therefore catch is a
/// relay that dropped a report, reordered two, or handed on a position the
/// device never reported; what it cannot catch is a defect inside the predictor
/// itself, which is `input/src/predict.rs`'s own corpus to bound and is said
/// here rather than left for a reader to assume otherwise.
#[derive(Clone, Copy, Debug)]
pub struct LateLatch {
    /// The node whose transform the pointer rides, or [`NO_NODE`].
    node: u32,
    /// The window of reports this component has been told about.
    predictor: Predictor,
    /// The last frame's latch, or why there was none.
    last: Result<Latched, Declined>,
    /// The instant the last latch was aimed at. Unit: nanoseconds.
    aimed_at_nanos: u64,
    /// Reports accepted. Unit: reports.
    reports: u64,
    /// Reports the predictor refused as not newer than what it held.
    ///
    /// Published rather than dropped, because a relay that duplicated or
    /// reordered reports is exactly the defect this path exists not to have and
    /// it is invisible in every other number here: the prediction would still be
    /// made, from a window one report shorter than the run implies.
    /// Unit: reports.
    stale: u64,
    /// Readings carrying no stamp at all, which are refused before the
    /// predictor sees them.
    ///
    /// Separate from [`LateLatch::stale`] because they are different facts: a
    /// stale report is a position somebody reported twice, and an unstamped one
    /// is **nobody having reported anything** — the state of a page the frame
    /// has not written. `f_abi::input::Event::decode` refuses the same value on
    /// the wire for the same reason, and this is that refusal again at the two
    /// doors into this module, so that neither a hand-built event nor a
    /// harness's reading can put a page of zeroes into the predictor.
    /// Unit: readings.
    unstamped: u64,
    /// Frames that carried a latch. Unit: frames.
    latches: u64,
    /// Frames that did not. Unit: frames.
    declines: u64,
    /// The transform the client committed for the node this frame patched,
    /// owed back to the graph once the frame's submission has crossed.
    ///
    /// `Some` from a latch until [`LateLatch::restore`], and `None` otherwise.
    /// The module's *the patch is the submitted frame's* is why it exists.
    owed: Option<SetTransform>,
    /// Frames whose patch was taken back out of the graph after they were
    /// submitted. Equal to [`LateLatch::latches`] on every run in which the
    /// graph refused nothing it had just accepted. Unit: frames.
    restores: u64,
}

impl LateLatch {
    /// A compositor that has been told about no pointer and no node.
    ///
    /// A `const`, so the whole of the starting state is a zero initialiser —
    /// `crate::waits::Waits::ZERO`'s reason, and RFC 0100's arithmetic behind
    /// it. [`Declined::NoNode`] is the honest answer for a component nobody has
    /// named a node to, and it is the same answer the first frame would get.
    pub const ZERO: Self = Self {
        node: NO_NODE,
        predictor: Predictor::new(),
        last: Err(Declined::NoNode),
        aimed_at_nanos: 0,
        reports: 0,
        stale: 0,
        unstamped: 0,
        latches: 0,
        declines: 0,
        owed: None,
        restores: 0,
    };

    /// A compositor told which node the pointer rides.
    ///
    /// Told and not discovered, for `crate::tree::Plan`'s reason: a compositor
    /// that decided for itself which node was the cursor would be guessing at a
    /// client's scene. [`NO_NODE`] means *no latch on this run*, which is what a
    /// routing page nobody wrote says.
    ///
    /// A constructor rather than a setter, so the node is decided once where the
    /// component is assembled and there is no second assignment for a later
    /// frame to disagree with — `crate::tree::StartingRung`'s arrangement and
    /// RFC 0119's reason, at a smaller stake.
    /// Written field by field rather than as an update of [`LateLatch::ZERO`],
    /// so that a field added above and forgotten here is a build failure that
    /// names the field — `crate::waits::Waits::ZERO`'s device, for its reason.
    #[must_use]
    pub const fn riding(node: u32) -> Self {
        Self {
            node,
            predictor: Predictor::new(),
            last: Err(Declined::NoReport),
            aimed_at_nanos: 0,
            reports: 0,
            stale: 0,
            unstamped: 0,
            latches: 0,
            declines: 0,
            owed: None,
            restores: 0,
        }
    }

    /// The node the pointer rides.
    #[must_use]
    pub const fn node(&self) -> u32 {
        self.node
    }

    /// One position the device reported.
    ///
    /// `false` when the predictor refused it as not newer than the newest it
    /// holds, which is `f_input::predict::Predictor::observe`'s rule and not
    /// this function's: two reports sharing an instant would put a zero under
    /// two divisions downstream.
    pub fn observed(&mut self, reading: Reading) -> bool {
        // **Before the mint, and this line is a scar.** The routing page starts
        // as zeroes and the frame writes the pointer words only when it has a
        // stamped position to write, so an unguarded read turns a page nobody
        // wrote into a report of the origin at the beginning of time — which is
        // precisely the scene `f_abi::input::NOT_STAMPED`'s own doc says that
        // constant exists to stop. It was not caught by a test: it was caught by
        // `cargo xtask input deliver` printing *1 report(s) taken* on a boot
        // whose frame writes no position at all, against a verdict clause that
        // said it must be zero.
        if reading.at_nanos == NOT_STAMPED {
            self.unstamped += 1;
            return false;
        }
        // The field, not a local. The module's *the one mint* is the whole of
        // why this line is written this way, and `lint-stamp` is what keeps it
        // written this way.
        let at = StampNanos::from_wire_nanos(reading.at_nanos);
        self.sample(Sample { at, x_x65536: reading.x_x65536, y_x65536: reading.y_x65536 })
    }

    /// One entry this component took off the input ring itself, `E3-B04g`.
    ///
    /// **The route a position reaches this component by**, and the reason this
    /// is a second entry point rather than a caller building a [`Reading`]: the
    /// reading is rebuilt here, from the decoded event's own field passed
    /// straight into `from_wire_nanos`, which is RFC 0124's third step
    /// verbatim. A [`Reading`] in between would be a copy of the stamp into a
    /// struct this module chose, and the whole of `lint-stamp`'s argument rule
    /// is that the number reaching the constructor is the number that crossed.
    ///
    /// `false` for an event that carries no position — a key, a button, a
    /// scroll — and for one the predictor refused as not newer than what it
    /// holds, which is counted in [`LateLatch::stale`] and is exactly how a
    /// relay that duplicated or reordered motion shows up here.
    ///
    /// The unstamped guard is repeated although `Event::decode` has already
    /// refused `NOT_STAMPED`, and the repetition is deliberate: a caller that
    /// built an [`Event`] by hand rather than by decoding one would otherwise
    /// put a page of zeroes into the predictor through this door, and the scar
    /// on [`LateLatch::observed`] is about exactly that.
    pub fn drained(&mut self, event: &Event) -> bool {
        let Entry::PointerMotion(motion) = event.body else { return false };
        if event.stamp_nanos == NOT_STAMPED {
            self.unstamped += 1;
            return false;
        }
        let at = StampNanos::from_wire_nanos(event.stamp_nanos);
        self.sample(Sample { at, x_x65536: motion.x_x65536, y_x65536: motion.y_x65536 })
    }

    /// Hand one sample to the predictor and count what it said.
    fn sample(&mut self, sample: Sample) -> bool {
        let taken = self.predictor.observe(sample);
        if taken {
            self.reports += 1;
        } else {
            self.stale += 1;
        }
        taken
    }

    /// The last frame's latch, or why there was none.
    ///
    /// # Errors
    ///
    /// [`Declined`], which is a frame that went out with the client's own
    /// transform rather than a failure.
    pub const fn last(&self) -> Result<Latched, Declined> {
        self.last
    }

    /// Reports this component was told about and took. Unit: reports.
    #[must_use]
    pub const fn reports(&self) -> u64 {
        self.reports
    }

    /// Reports the predictor refused as not newer than what it held.
    /// Unit: reports.
    #[must_use]
    pub const fn stale(&self) -> u64 {
        self.stale
    }

    /// Readings carrying no stamp, refused before the predictor saw them.
    /// Unit: readings.
    #[must_use]
    pub const fn unstamped(&self) -> u64 {
        self.unstamped
    }

    /// Frames that carried a latch. Unit: frames.
    #[must_use]
    pub const fn latches(&self) -> u64 {
        self.latches
    }

    /// Frames that did not. Unit: frames.
    #[must_use]
    pub const fn declines(&self) -> u64 {
        self.declines
    }

    /// Frames whose patch was taken back out of the graph once submitted.
    /// Unit: frames.
    #[must_use]
    pub const fn restores(&self) -> u64 {
        self.restores
    }

    /// Give the graph back the transform the client committed, once the frame
    /// the latch patched has been submitted.
    ///
    /// Called by `crate::tree::Held::close` after `crate::waits::Waits::frame`
    /// returns, which is after the compositor's own submission has entered its
    /// wait — the frame has left, and the patch was that frame's. A frame that
    /// carried no latch owes nothing and this does nothing.
    ///
    /// **After the submission and not before the next latch**, and the
    /// difference is a scene. A restore deferred to the next frame's latch
    /// would leave a transform no client sent in the retained graph between
    /// frames, and would then write the old committed value *over* a transform
    /// the client had sent in the meantime — the client's newest word lost to
    /// the compositor's memory of its previous one.
    ///
    /// Through the same door as the patch, `f_scene::arena::Arena::set_transform`,
    /// which names one node and six numbers and can do nothing else. `false`
    /// where the graph refused it, which would be the graph refusing a node it
    /// accepted a statement ago; it is not counted in [`LateLatch::restores`],
    /// so a reader comparing that against [`LateLatch::latches`] sees it.
    pub fn restore(&mut self, graph: &mut Arena) -> bool {
        let Some(owed) = self.owed.take() else { return true };
        let restored = graph.set_transform(owed).is_ok();
        if restored {
            self.restores += 1;
        }
        restored
    }

    /// The instant the last latch was aimed at, so that the two records
    /// `E3-B04e` compares are asked the same question.
    ///
    /// It is published rather than kept, because the seam's whole value is that
    /// neither side derives its number from the other — and a seam in which the
    /// two sides silently predicted to *different instants* would agree about
    /// nothing, with the disagreement reading as a defect in the relay. So the
    /// compositor decides the instant, says which one it decided, and the other
    /// side asks its own predictor the same question. Nothing is sent and
    /// nothing is received: this is a reading a test and a boot take, which is
    /// `crate::waits::Published`'s arrangement.
    /// Unit: nanoseconds, in the channel's epoch.
    #[must_use]
    pub const fn aimed_at_nanos(&self) -> u64 {
        self.aimed_at_nanos
    }

    /// Re-read the pointer and patch it into the transform node.
    ///
    /// Called between the application's signal and the compositor's own
    /// submission, and nowhere else — [`crate::waits::Between`] is the door and
    /// the module header is why the window is a parameter of the sequence rather
    /// than a comment.
    ///
    /// `entered` is how many wait entries the frame's trace holds at this
    /// instant, which becomes [`Latched::before_entry`].
    ///
    /// # Errors
    ///
    /// [`Declined`], and every variant of it is a frame that went out with the
    /// transform the client committed. None of them is a failure.
    pub fn latch(
        &mut self,
        graph: &mut Arena,
        entered: usize,
        aim: Aim,
    ) -> Result<Latched, Declined> {
        self.aimed_at_nanos = aim.scanout_nanos;
        let outcome = self.patch(graph, entered, aim);
        if outcome.is_ok() {
            self.latches += 1;
        } else {
            self.declines += 1;
        }
        self.last = outcome;
        outcome
    }

    /// The latch itself, split out so that [`LateLatch::latch`]'s counters move
    /// whatever this answers — `crate::waits::Waits::frame`'s arrangement and
    /// for its reason: a frame that declined is still a frame, and a reader
    /// counting frames against latches is owed both numbers.
    fn patch(&mut self, graph: &mut Arena, entered: usize, aim: Aim) -> Result<Latched, Declined> {
        if self.node == NO_NODE {
            return Err(Declined::NoNode);
        }
        // The transform as the client committed it, read out of the graph rather
        // than remembered from the delta that set it. That is the difference
        // between this record and a plausible one: a compositor comparing its
        // own memory of the commit against its own patch would agree with itself
        // however wrong the graph was.
        let Ok(Some(committed)) = graph.transform_of(self.node) else {
            return Err(Declined::NoTransform);
        };
        // The field, not a local. See [`LateLatch::observed`].
        let scanout = StampNanos::from_wire_nanos(aim.scanout_nanos);
        let Some(predicted) = self.predictor.predict_at(scanout) else {
            return Err(Declined::NoReport);
        };
        // Everything but the two translations is the client's, copied across
        // untouched. A latch that rewrote the matrix would be a compositor
        // deciding a client's scale, and the exit's *and by nothing else* would
        // be false in the four numbers a reader is least likely to check.
        let patched = SetTransform {
            node: self.node,
            a_x65536: committed.a_x65536,
            b_x65536: committed.b_x65536,
            c_x65536: committed.c_x65536,
            d_x65536: committed.d_x65536,
            tx_x65536: i64::from(predicted.x_x65536()),
            ty_x65536: i64::from(predicted.y_x65536()),
        };
        // The graph as the client committed it, with the two numbers about to
        // change masked out, taken immediately before the patch; and again
        // immediately after. Nothing else runs between the two, so a difference
        // is this patch touching something it was not allowed to.
        let (unmoved_before, _) = unmoved(graph, self.node);
        if graph.set_transform(patched).is_err() {
            return Err(Declined::Refused);
        }
        // Owed back, whole: the client's own record and not a copy with its
        // translation re-derived, so the restore is the client's transform and
        // nothing this component computed.
        self.owed = Some(committed);
        let (unmoved_after, walked) = unmoved(graph, self.node);
        Ok(Latched {
            node: self.node,
            before_entry: u32::try_from(entered).unwrap_or(u32::MAX),
            committed_tx_x65536: committed.tx_x65536,
            committed_ty_x65536: committed.ty_x65536,
            latched_tx_x65536: patched.tx_x65536,
            latched_ty_x65536: patched.ty_x65536,
            lead_nanos: predicted.lead_nanos(),
            extrapolated: predicted.basis().extrapolated(),
            unmoved_before,
            unmoved_after,
            walked,
        })
    }
}

/// Where an unmoved fold starts. FNV-1a's offset basis, for
/// `f_abi::input::Crossing`'s reason: not zero, so a word nobody wrote is not a
/// fold of nothing. Unit: none — a checksum.
const UNMOVED_BASIS: u64 = 0xCBF2_9CE4_8422_2325;

/// FNV-1a's 64-bit prime. Unit: none.
const UNMOVED_PRIME: u64 = 0x0000_0100_0000_01B3;

/// Every field of every node a renderer could see, folded in the order the
/// encoding walks them, with `masked`'s two translations left out.
///
/// # What is covered, and why it is the encoding's list
///
/// The node's identifier, its depth, its kind, and each of its three property
/// records — transform, path, paint — with a presence byte in front of each,
/// which is `f_scene::encode`'s node record field for field. The walk is the
/// roots in paint order, each followed depth-first by its children in paint
/// order, which is the order a frame is encoded in. So a change this fold
/// cannot see is a change the encoding would not carry either, and the
/// encoding is the instrument `E3-B06f` and the host tests trust for the
/// reason that a difference it cannot see is a difference no renderer can.
///
/// What is left out is exactly two numbers: `masked`'s `tx_x65536` and
/// `ty_x65536`. Its other four matrix entries are folded, which is the half of
/// *and by nothing else* a reader is least likely to check — a latch that
/// wrote a fresh matrix rather than copying the client's would move this word.
///
/// # What it is not
///
/// A second implementation of the encoding's walk, and it says so rather than
/// pretending otherwise: a field added to the encoding and not here is a field
/// this cannot see, and the tests in this module are what keep the two lists
/// in step — `the_unmoved_fold_sees_every_field_but_the_two_it_masks` changes
/// every field this folds and requires every change but two to move it.
/// *What would reverse this:* a component with the heap for the encoder, at
/// which point the boot compares the encoding itself and this goes away.
///
/// Answers the fold and how many nodes it walked. Unit: none — a checksum; nodes.
#[must_use]
pub fn unmoved(graph: &Arena, masked: u32) -> (u64, u32) {
    let mut fold = Unmoved { word: UNMOVED_BASIS, walked: 0 };
    for root in graph.roots() {
        fold.visit(graph, root, 0, masked);
    }
    (fold.word, fold.walked)
}

/// The fold in progress.
struct Unmoved {
    word: u64,
    walked: u32,
}

impl Unmoved {
    fn mix(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.word = (self.word ^ u64::from(byte)).wrapping_mul(UNMOVED_PRIME);
        }
    }

    /// One node and everything under it. Recursive, and the depth is the
    /// graph's, which `f_scene::arena::NODES_MAX` bounds.
    fn visit(&mut self, graph: &Arena, node: u32, depth: u32, masked: u32) {
        self.walked = self.walked.saturating_add(1);
        self.mix(u64::from(node));
        self.mix(u64::from(depth));
        self.mix(graph.kind_of(node).map_or(u64::MAX, |kind| u64::from(kind.wire())));
        match graph.transform_of(node) {
            Ok(Some(t)) => {
                self.mix(1);
                for value in [t.a_x65536, t.b_x65536, t.c_x65536, t.d_x65536] {
                    self.mix(value as u64);
                }
                if node != masked {
                    self.mix(t.tx_x65536 as u64);
                    self.mix(t.ty_x65536 as u64);
                }
            }
            _ => self.mix(0),
        }
        match graph.path_of(node) {
            Ok(Some(p)) => {
                self.mix(1);
                self.mix(u64::from(p.geometry_offset));
                self.mix(u64::from(p.geometry_bytes));
                self.mix(u64::from(p.fill_rule));
            }
            _ => self.mix(0),
        }
        match graph.paint_of(node) {
            Ok(Some(p)) => {
                self.mix(1);
                for value in [p.red_x65535, p.green_x65535, p.blue_x65535, p.alpha_x65535] {
                    self.mix(u64::from(value));
                }
                self.mix(u64::from(p.stroke_width_x65536));
            }
            _ => self.mix(0),
        }
        if let Ok(children) = graph.children(node) {
            for child in children {
                self.visit(graph, child, depth.saturating_add(1), masked);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use f_abi::Sqe;
    use f_abi::scene::{Commit, CreateNode, Delta, Entry, PAYLOAD_BYTES, SetPaint, SetPath, kind};
    use f_abi::trace::Stage;
    use f_input::predict::Predictor;
    use f_interface::token::Theme;
    use f_scene::commit::Batch;
    use f_scene::dirty::Dirty;
    use f_scene::encode::{Encoder, encode};

    use crate::pacing::Tick;
    use crate::tree::{FRAME_DELTAS_MAX, Held, Plan};
    use crate::waits::{Between, Waits};

    use super::*;

    /// The layer the pointer hangs under.
    const ROOT: u32 = 1;

    /// The transform node the pointer rides, and the only node anything in this
    /// module patches.
    const POINTER: u32 = 2;

    /// A sibling of the pointer that nothing touches.
    ///
    /// It is here for *and by nothing else*: a scene of one moving node cannot
    /// tell a latch that patched one transform from one that rewrote the frame,
    /// because the two produce the same bytes.
    const STILL: u32 = 3;

    /// A sixty-hertz frame. Unit: nanoseconds.
    const PERIOD_NANOS: u64 = 16_666_667;

    /// What the wake time holds back. Unit: nanoseconds.
    const MARGIN_NANOS: u64 = 1_000_000;

    /// The frame token every encoding in this module is taken under.
    ///
    /// Distinct in every byte that is not zero, for `semantic/src/emit.rs`'s
    /// reason: a header that wrote the token at the wrong offset cannot produce
    /// the same image.
    const IMAGE_FRAME: u64 = 0x0BAD_F00D_0000_0002;

    /// Where the client committed the pointer.
    /// Unit: device pixels, scaled by 65 536.
    const COMMITTED_X_X65536: i32 = 640 * 65_536;

    /// And along y. Unit: device pixels, scaled by 65 536.
    const COMMITTED_Y_X65536: i32 = 360 * 65_536;

    /// The motion the harness injects between the commit and the submission.
    ///
    /// Not round in either axis and different between them, so a latch that
    /// copied one axis into the other, or rounded, or scaled, produces a
    /// different number rather than the same one.
    /// Unit: device pixels, scaled by 65 536.
    const INJECTED_X_X65536: i32 = 7 * 65_536 + 13_579;

    /// And along y. Unit: device pixels, scaled by 65 536.
    const INJECTED_Y_X65536: i32 = -3 * 65_536 - 2_468;

    /// When the device reported the position the client committed.
    /// Unit: nanoseconds.
    const COMMITTED_AT_NANOS: u64 = 5_000_000;

    /// When it reported the injected one. Unit: nanoseconds.
    const INJECTED_AT_NANOS: u64 = 5_100_000;

    /// One delta, encoded the way a client encodes one.
    fn wire(body: Entry, deadline: u64) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        Delta { user_data: 1, class: 0, deadline, payload_offset: 0, flags: 0, body }.encode()
    }

    /// The transform record the client commits for the pointer.
    ///
    /// The matrix is a shear as well as a scale and none of its four entries is
    /// the identity, which is the half of *and by nothing else* a reader is
    /// least likely to check: a latch that wrote a fresh matrix rather than
    /// copying the client's would pass every assertion about the translation.
    fn committed(tx_x65536: i32, ty_x65536: i32) -> SetTransform {
        SetTransform {
            node: POINTER,
            a_x65536: 2 * 65_536,
            b_x65536: 17_000,
            c_x65536: -9_000,
            d_x65536: 3 * 65_536,
            tx_x65536: i64::from(tx_x65536),
            ty_x65536: i64::from(ty_x65536),
        }
    }

    /// The scene these tests commit: a layer, the node the pointer rides, a
    /// sibling that never moves, and the commit that closes the frame.
    fn scene(tx_x65536: i32, ty_x65536: i32) -> [(Sqe, [u8; PAYLOAD_BYTES]); 5] {
        [
            wire(
                Entry::CreateNode(CreateNode {
                    node: ROOT,
                    parent: NO_NODE,
                    before: NO_NODE,
                    kind: kind::LAYER,
                }),
                0,
            ),
            wire(
                Entry::CreateNode(CreateNode {
                    node: POINTER,
                    parent: ROOT,
                    before: NO_NODE,
                    kind: kind::TRANSFORM,
                }),
                0,
            ),
            wire(
                Entry::CreateNode(CreateNode {
                    node: STILL,
                    parent: ROOT,
                    before: NO_NODE,
                    kind: kind::TRANSFORM,
                }),
                0,
            ),
            wire(Entry::SetTransform(committed(tx_x65536, ty_x65536)), 0),
            wire(Entry::Commit(Commit { frame_token: IMAGE_FRAME }), 900),
        ]
    }

    /// What the frame tells the component in these tests.
    fn plan(pointer_node: u32) -> Plan {
        Plan {
            backend_bits: 0,
            scanout_period_nanos: PERIOD_NANOS,
            margin_nanos: MARGIN_NANOS,
            pointer_node,
            // The compositor's declared cap. Nothing in this module comes near
            // it: the latch is about one transform, not about how many.
            deltas_per_frame_max: 50,
        }
    }

    /// The whole graph as bytes.
    ///
    /// `f_scene::encode` over the subtree under [`ROOT`], which is every node
    /// these tests build. It is the instrument for *and by nothing else* and it
    /// is trustworthy for `E3-B06f`'s reason: the encoding is the canonical form
    /// of a frame, it is what stage 2 of the ladder receives, and a difference
    /// the encoding cannot see is a difference no renderer can see either.
    ///
    /// The marks are taken fresh on both sides of every comparison, so the two
    /// images are the same walk of the same nodes and a byte that differs is a
    /// value that differs rather than a traversal that did.
    fn image(arena: &Arena, into: &mut Encoder) {
        let mut marks = Dirty::CLEAN;
        marks.mark(arena, ROOT);
        encode(into, &marks.at_commit(IMAGE_FRAME), arena);
    }

    /// Where two byte images differ, as the first and last index that does.
    ///
    /// `None` when they are identical. Written rather than asserted against a
    /// hand-computed offset because an offset written here would be this test's
    /// copy of `f_scene::encode`'s layout, and the layout is what the comparison
    /// is supposed to be independent of.
    fn differ(before: &[u8], after: &[u8]) -> Option<(usize, usize)> {
        assert_eq!(before.len(), after.len(), "the latch changed the length of the frame");
        let first = before.iter().zip(after).position(|(a, b)| a != b)?;
        let last = before.iter().zip(after).rposition(|(a, b)| a != b)?;
        Some((first, last))
    }

    /// A harness that injects a known motion **inside** the window and then
    /// latches.
    ///
    /// This is what makes the clause's *between commit and submit* literal
    /// rather than approximate. `crate::waits::Between` is called by the chain
    /// driver after the application's signal has been admitted — the commit has
    /// closed — and before the compositor's own submission enters its wait, so a
    /// report observed in here arrived strictly inside that gap. Nothing outside
    /// the component can reach that instant, which is why the injector is a
    /// `Between` and not a call before `Held::offer`.
    struct Inject<'g, 'l> {
        /// The graph the latch patches.
        graph: &'g mut Arena,
        /// The pointer.
        latch: &'l mut LateLatch,
        /// The position the device reports inside the window.
        reading: Reading,
        /// What the frame is aimed at.
        aim: Aim,
        /// How many trace entries the window found, recorded so the test can
        /// assert on it rather than on the order of two calls in another file.
        entered: usize,
    }

    impl Between for Inject<'_, '_> {
        fn between_commit_and_submit(&mut self, entered: usize) {
            self.entered = entered;
            self.latch.observed(self.reading);
            let _latched = self.latch.latch(self.graph, entered, self.aim);
        }
    }

    /// The committed frame, in an arena, with the pointer at the committed
    /// position and nothing latched.
    fn commit_a_frame(graph: &mut Arena, batch: &mut Batch<FRAME_DELTAS_MAX>) {
        let mut held = Held::new(graph, batch, plan(NO_NODE), &Theme::DEFAULT);
        for (at, (entry, payload)) in
            scene(COMMITTED_X_X65536, COMMITTED_Y_X65536).iter().enumerate()
        {
            let answer = held.offer(entry, payload, Tick(1_000 + 100 * at as u64));
            assert!(answer.is_some_and(|cqe| cqe.result == 0), "the graph refused delta {at}");
        }
        assert_eq!(held.counters().frames, 1);
    }

    /// **`E3-B01i`'s exit, both clauses, over one frame.**
    ///
    /// The latched transform differs from the committed one by exactly the
    /// motion injected between commit and submit — asserted as an equality
    /// against the injected numbers rather than as a range — and the whole frame
    /// is otherwise byte for byte the frame the client committed.
    ///
    /// The prediction is **held** here rather than extrapolated, and that is the
    /// case this clause is true in: one report is not a velocity, so the
    /// predictor answers the position the device last reported and the
    /// difference is the motion and nothing else. An extrapolating latch differs
    /// by the motion *plus* the lead the predictor claimed, which is `E3-B04e`'s
    /// subject and is asserted there. Saying so is the point rather than a
    /// caveat: the two lines want different arithmetic out of one mechanism.
    #[test]
    fn the_latch_differs_from_the_commit_by_exactly_the_injected_motion() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);

        let mut before = Encoder::EMPTY;
        image(&graph, &mut before);

        let mut latch = LateLatch::riding(POINTER);
        let mut waits = Waits::ZERO;
        let mut inject = Inject {
            graph: &mut graph,
            latch: &mut latch,
            reading: Reading {
                at_nanos: INJECTED_AT_NANOS,
                x_x65536: COMMITTED_X_X65536 + INJECTED_X_X65536,
                y_x65536: COMMITTED_Y_X65536 + INJECTED_Y_X65536,
            },
            // At the newest report, so the lead is zero and the predictor holds.
            // `f_input::predict::Held::NoLead` is the reason it answers the
            // sample itself, and it is named here because the whole of this
            // test's arithmetic rests on it.
            aim: Aim { scanout_nanos: INJECTED_AT_NANOS },
            entered: usize::MAX,
        };
        waits.frame(1, true, &mut inject);
        let entered = inject.entered;

        // The window. Zero entries means the latch landed after the commit
        // closed and before the compositor's own submission entered its wait,
        // and the two entries afterwards are the two submissions that crossed
        // after it.
        assert_eq!(entered, 0, "the latch did not land between the commit and the first submit");
        assert_eq!(waits.trace().len(), 2, "the compositor's wait and the present engine's");
        assert_eq!(
            waits.trace().entry(0).expect("the compositor's wait").waiter(),
            Stage::Compositor
        );

        let latched = latch.last().expect("the frame latched");
        assert_eq!(latched.node(), POINTER);
        assert_eq!(latched.before_entry(), 0, "the record's own key says the same thing");
        assert!(!latched.extrapolated(), "one report is not a velocity");
        assert_eq!(latched.lead_nanos(), 0);
        assert_eq!(latched.committed_tx_x65536(), i64::from(COMMITTED_X_X65536));
        assert_eq!(latched.committed_ty_x65536(), i64::from(COMMITTED_Y_X65536));

        // *By exactly the motion injected.* An equality against the two numbers
        // the harness injected, on both axes.
        assert_eq!(latched.moved_x_x65536(), i64::from(INJECTED_X_X65536));
        assert_eq!(latched.moved_y_x65536(), i64::from(INJECTED_Y_X65536));

        // *And by nothing else.* Two statements of it, because they fail
        // differently. The first is that the submitted frame is byte for byte
        // the frame a client would have committed had it sent the latched
        // transform itself — built in a second arena from the same deltas, so a
        // latch that touched anything at all is a byte that differs. The second
        // is that the difference between the committed image and the submitted
        // one is one contiguous run no longer than the two translations, which
        // is what fails when a latch rewrites the matrix beside them.
        let mut after = Encoder::EMPTY;
        image(&graph, &mut after);

        let mut expected_graph = Arena::EMPTY;
        let mut expected_batch = Batch::new();
        {
            let mut held =
                Held::new(&mut expected_graph, &mut expected_batch, plan(NO_NODE), &Theme::DEFAULT);
            for (at, (entry, payload)) in scene(
                COMMITTED_X_X65536 + INJECTED_X_X65536,
                COMMITTED_Y_X65536 + INJECTED_Y_X65536,
            )
            .iter()
            .enumerate()
            {
                let answer = held.offer(entry, payload, Tick(1_000 + 100 * at as u64));
                assert!(answer.is_some_and(|cqe| cqe.result == 0), "delta {at}");
            }
        }
        let mut expected = Encoder::EMPTY;
        image(&expected_graph, &mut expected);
        assert_eq!(
            after.bytes(),
            expected.bytes(),
            "the latched frame is not the frame a client sending that transform would have got"
        );

        // The boot's form of the same clause, over the same frame: the fold with
        // the two translations masked did not move, and it walked every node.
        assert_eq!(latched.unmoved_before(), latched.unmoved_after());
        assert_eq!(latched.walked(), 3, "the root, the pointer and the sibling");

        let (first, last) =
            differ(before.bytes(), after.bytes()).expect("the latch changed nothing");
        assert!(
            last - first < 2 * size_of::<i64>(),
            "the frame differs across {} bytes, which is more than the two translations",
            last - first + 1
        );
    }

    /// A frame with no latch on it is the frame the client committed, to the
    /// byte.
    ///
    /// The control for the test above, and it is the one that fails when a latch
    /// patches a node nobody named: a compositor that defaulted to some node
    /// would move something here.
    #[test]
    fn a_compositor_told_no_node_submits_the_committed_frame_unchanged() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);

        let mut before = Encoder::EMPTY;
        image(&graph, &mut before);

        let mut latch = LateLatch::ZERO;
        let mut waits = Waits::ZERO;
        let mut inject = Inject {
            graph: &mut graph,
            latch: &mut latch,
            reading: Reading {
                at_nanos: INJECTED_AT_NANOS,
                x_x65536: COMMITTED_X_X65536 + INJECTED_X_X65536,
                y_x65536: COMMITTED_Y_X65536 + INJECTED_Y_X65536,
            },
            aim: Aim { scanout_nanos: INJECTED_AT_NANOS },
            entered: usize::MAX,
        };
        waits.frame(1, true, &mut inject);

        assert_eq!(latch.last(), Err(Declined::NoNode));
        assert_eq!(latch.latches(), 0);
        assert_eq!(latch.declines(), 1);
        assert_eq!(latch.reports(), 1, "the report was still taken");

        let mut after = Encoder::EMPTY;
        image(&graph, &mut after);
        assert_eq!(before.bytes(), after.bytes(), "a frame nobody latched is not the client's");
    }

    /// A frame whose own signal the chain refuses has no window, so nothing is
    /// latched into it.
    ///
    /// The case `crate::waits`' own
    /// `a_frame_ordinal_that_does_not_move_is_refused_and_counted` produces, from
    /// this side: a latch that ran before the application's signal was admitted
    /// would be patching a frame the chain never accepted, and the trace of that
    /// frame is empty rather than half full.
    #[test]
    fn a_frame_the_chain_refused_has_no_window_in_it() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);

        let mut before = Encoder::EMPTY;
        image(&graph, &mut before);

        let mut latch = LateLatch::riding(POINTER);
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        {
            let mut inject = Inject {
                graph: &mut graph,
                latch: &mut latch,
                reading: Reading {
                    at_nanos: INJECTED_AT_NANOS,
                    x_x65536: COMMITTED_X_X65536 + INJECTED_X_X65536,
                    y_x65536: COMMITTED_Y_X65536 + INJECTED_Y_X65536,
                },
                aim: Aim { scanout_nanos: INJECTED_AT_NANOS },
                entered: usize::MAX,
            };
            // The same ordinal twice, which the chain refuses at its first call.
            waits.frame(1, true, &mut inject);
            assert_eq!(inject.entered, usize::MAX, "the window opened on a refused frame");
        }
        assert_eq!(waits.published().refusals, 1);
        assert_eq!(latch.latches(), 0, "a refused frame was latched into");

        let mut after = Encoder::EMPTY;
        image(&graph, &mut after);
        assert_eq!(before.bytes(), after.bytes());
    }

    /// The component's own window, driven the way the machine drives it.
    ///
    /// The three tests above reach the window through a harness, because nothing
    /// outside this component can be inside it. This one asserts the production
    /// path: `crate::tree::Held::close` builds the window, the chain driver calls
    /// it, the frame submitted is the client's frame with one transform patched,
    /// and the graph left behind once it has gone is the client's again.
    ///
    /// What it cannot assert is the injection's *instant*: the last moment a
    /// caller outside the component can report a position is before the commit
    /// entry is offered, so the motion here arrives before the commit closes and
    /// is *re-read* after it. That is the honest half of this line and it is
    /// stated rather than glossed — the clause's literal reading is the harness
    /// tests', and this is the demonstration that the mechanism they drive is the
    /// one the component runs.
    #[test]
    fn the_components_own_window_latches_the_frame_it_closes() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan(POINTER), &Theme::DEFAULT);
        let entries = scene(COMMITTED_X_X65536, COMMITTED_Y_X65536);

        // The position the client is about to commit, reported first so that the
        // committed transform and the newest report are the same place.
        assert!(held.reported(Reading {
            at_nanos: COMMITTED_AT_NANOS,
            x_x65536: COMMITTED_X_X65536,
            y_x65536: COMMITTED_Y_X65536,
        }));
        for (at, (entry, payload)) in entries.iter().enumerate() {
            if at + 1 == entries.len() {
                // After the last delta of the frame and before the commit that
                // closes it.
                assert!(held.reported(Reading {
                    at_nanos: INJECTED_AT_NANOS,
                    x_x65536: COMMITTED_X_X65536 + INJECTED_X_X65536,
                    y_x65536: COMMITTED_Y_X65536 + INJECTED_Y_X65536,
                }));
            }
            let answer = held.offer(entry, payload, Tick(1_000 + 100 * at as u64));
            assert!(answer.is_some_and(|cqe| cqe.result == 0), "the graph refused delta {at}");
        }

        let latched = held.latch().last().expect("the component's own window latched");
        assert_eq!(latched.node(), POINTER);
        assert_eq!(latched.before_entry(), 0);
        assert_eq!(held.latch().latches(), 1);
        assert_eq!(held.latch().reports(), 2);
        assert_eq!(held.latch().stale(), 0);
        // What was submitted is the record's; what is retained is the client's.
        // The patch left with the frame it was made for, which the module's
        // *the patch is the submitted frame's* argues.
        assert_eq!(
            (latched.latched_tx_x65536(), latched.latched_ty_x65536()),
            (
                i64::from(COMMITTED_X_X65536 + INJECTED_X_X65536),
                i64::from(COMMITTED_Y_X65536 + INJECTED_Y_X65536)
            ),
            "the record does not say the injected position was submitted"
        );
        assert_eq!(held.latch().restores(), 1, "the patch was not taken back out");
        assert_eq!(
            held.graph().transform_of(POINTER),
            Ok(Some(committed(COMMITTED_X_X65536, COMMITTED_Y_X65536))),
            "the retained graph holds the submitted patch rather than what the client committed"
        );
        assert_eq!(
            held.graph().transform_of(STILL),
            Ok(None),
            "a node nobody named acquired a transform"
        );
    }

    /// **A second frame with no new transform in it is latched against what the
    /// client committed, and not against the last frame's latch.**
    ///
    /// The audit's finding, and the test that was not written: every test above
    /// closes one frame, and `the_boots_motion_is_held_and_not_extrapolated`
    /// latches four times over one graph without once asking what it read as
    /// *committed*. A latch that leaves its patch in the retained graph reads
    /// its own previous patch there on the next frame — the comparison
    /// `LateLatch::patch`'s own comment forbids — and the graph between frames
    /// holds a transform no client sent.
    #[test]
    fn a_second_frame_is_latched_against_the_clients_transform_and_not_its_own() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan(POINTER), &Theme::DEFAULT);

        // Frame one: the client commits the pointer, and the device has moved.
        assert!(held.reported(Reading {
            at_nanos: COMMITTED_AT_NANOS,
            x_x65536: COMMITTED_X_X65536 + INJECTED_X_X65536,
            y_x65536: COMMITTED_Y_X65536 + INJECTED_Y_X65536,
        }));
        for (at, (entry, payload)) in
            scene(COMMITTED_X_X65536, COMMITTED_Y_X65536).iter().enumerate()
        {
            let answer = held.offer(entry, payload, Tick(1_000 + 100 * at as u64));
            assert!(answer.is_some_and(|cqe| cqe.result == 0), "the graph refused delta {at}");
        }
        let first = held.latch().last().expect("frame one latched");
        assert_eq!(first.committed_tx_x65536(), i64::from(COMMITTED_X_X65536));
        assert_ne!(
            first.moved_x_x65536(),
            0,
            "frame one moved nothing, so frame two proves nothing"
        );

        // Between frames the retained graph is what the client authored. The
        // latch's patch was the submitted frame's and has left with it.
        assert_eq!(
            held.graph().transform_of(POINTER),
            Ok(Some(committed(COMMITTED_X_X65536, COMMITTED_Y_X65536))),
            "the retained graph holds a transform no client sent"
        );

        // Frame two: no transform from the client, the device moved again, and
        // the frame is the commit alone.
        assert!(held.reported(Reading {
            at_nanos: INJECTED_AT_NANOS,
            x_x65536: COMMITTED_X_X65536 + 2 * INJECTED_X_X65536,
            y_x65536: COMMITTED_Y_X65536 + 2 * INJECTED_Y_X65536,
        }));
        let (entry, payload) =
            wire(Entry::Commit(Commit { frame_token: IMAGE_FRAME + 1 }), 1_000_000_000);
        let answer = held.offer(&entry, &payload, Tick(2_000));
        assert!(answer.is_some_and(|cqe| cqe.result == 0), "the second commit was refused");
        assert_eq!(held.counters().frames, 2);
        assert_eq!(held.latch().latches(), 2);

        let second = held.latch().last().expect("frame two latched");
        assert_eq!(
            (second.committed_tx_x65536(), second.committed_ty_x65536()),
            (i64::from(COMMITTED_X_X65536), i64::from(COMMITTED_Y_X65536)),
            "frame two's committed transform is the compositor's own last latch, so the latch \
             compared its memory against itself"
        );
        assert_eq!(
            held.graph().transform_of(POINTER),
            Ok(Some(committed(COMMITTED_X_X65536, COMMITTED_Y_X65536))),
            "after frame two the retained graph holds a transform no client sent"
        );
    }

    /// **`E3-B04e`'s exit.** The latched value is the predicted one, and the two
    /// records agree on the value and on how far it moved.
    ///
    /// # Which side produces which number
    ///
    /// The **input path's** record is a `f_input::predict::Predicted`, produced
    /// by a predictor this test feeds from the reports as the driver stamped
    /// them. Its value is `x_x65536` and its movement is that against its own
    /// `anchor`, which is the newest report *it* holds.
    ///
    /// The **compositor's** record is a [`Latched`], produced inside the window
    /// by a different predictor holding a different window, and its movement is
    /// against the transform read back **out of the graph** — the number the
    /// client committed, not a number this component remembered.
    ///
    /// Neither is computed from the other. They share an implementation and not
    /// an instance, and what that buys is stated rather than overclaimed: two
    /// predictor instances, one reached through the late latch inside
    /// [`Waits::frame`]'s window and one called directly, agree on the value, on
    /// how far it moved, on the lead and on the instant aimed at. That catches a
    /// latch that hands the graph something other than what its predictor
    /// answered, or asks it about another instant. It does not catch a defect
    /// inside the predictor, which is `input/src/predict.rs`'s own bounded
    /// corpus.
    ///
    /// **It has no relay in it.** Both predictors are fed by loops over one local
    /// array, and the client commits that array's newest report, so a dropped,
    /// reordered or invented report — or a committed transform that is not the
    /// newest — cannot appear between the two sides here. A seam that catches
    /// those needs the reports to reach the compositor by a route the test does
    /// not write, which is `E3-B04g`.
    ///
    /// The basis is asserted to be an **extrapolation**, because a held
    /// prediction would make both sides agree by copying the same sample and the
    /// seam would prove nothing.
    #[test]
    fn the_latched_value_is_the_predicted_one_from_both_sides() {
        // A constant velocity along both axes, four reports a hundred
        // microseconds apart — `kernel/src/input.rs`'s own tick, so this is the
        // motion the boot's device produces rather than a shape invented here.
        const STEP_NANOS: u64 = 100_000;
        // Ten device pixels a millisecond along x and five back along y, which
        // is under `f_input::predict::SPEED_CEILING_X65536_PER_MS`: a faster
        // fixture is a teleport rather than a movement and the predictor holds
        // the position instead of extrapolating, which is the assertion below.
        const STEP_X_X65536: i32 = 65_536;
        const STEP_Y_X65536: i32 = -32_768;
        const REPORTS: usize = 4;

        let mut reports = [Reading::default(); REPORTS];
        for (at, report) in reports.iter_mut().enumerate() {
            let step = at as i32;
            *report = Reading {
                at_nanos: COMMITTED_AT_NANOS + STEP_NANOS * at as u64,
                x_x65536: COMMITTED_X_X65536 + STEP_X_X65536 * step,
                y_x65536: COMMITTED_Y_X65536 + STEP_Y_X65536 * step,
            };
        }
        let newest = reports[REPORTS - 1];
        // Half a frame past the newest report, which is where a compositor that
        // has just closed a frame is aiming.
        let scanout_nanos = newest.at_nanos + PERIOD_NANOS / 2;

        // --- the input path's record ----------------------------------------
        let mut path = Predictor::new();
        for report in &reports {
            assert!(path.observe(Sample {
                at: StampNanos::from_wire_nanos(report.at_nanos),
                x_x65536: report.x_x65536,
                y_x65536: report.y_x65536,
            }));
        }
        let predicted = path
            .predict_at(StampNanos::from_wire_nanos(scanout_nanos))
            .expect("the input path predicted");
        assert!(predicted.basis().extrapolated(), "a held prediction makes this seam vacuous");

        // --- the compositor's record ----------------------------------------
        //
        // The client commits the newest report, which is what makes the two
        // movements the same question asked of two states.
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        {
            let mut held = Held::new(&mut graph, &mut batch, plan(NO_NODE), &Theme::DEFAULT);
            for (at, (entry, payload)) in scene(newest.x_x65536, newest.y_x65536).iter().enumerate()
            {
                let answer = held.offer(entry, payload, Tick(1_000 + 100 * at as u64));
                assert!(answer.is_some_and(|cqe| cqe.result == 0), "delta {at}");
            }
        }
        let mut latch = LateLatch::riding(POINTER);
        for report in &reports {
            assert!(latch.observed(*report));
        }
        let mut waits = Waits::ZERO;
        let mut inject = Inject {
            graph: &mut graph,
            latch: &mut latch,
            // The window's own report is the newest one again, which the
            // predictor refuses as not newer — so this latch predicts from
            // exactly the four reports the input path holds and not five.
            reading: newest,
            aim: Aim { scanout_nanos },
            entered: usize::MAX,
        };
        waits.frame(1, true, &mut inject);
        assert_eq!(inject.entered, 0);
        assert_eq!(latch.stale(), 1, "the repeated report was refused rather than held twice");

        let latched = latch.last().expect("the frame latched");

        // The value.
        assert_eq!(latched.latched_tx_x65536(), i64::from(predicted.x_x65536()));
        assert_eq!(latched.latched_ty_x65536(), i64::from(predicted.y_x65536()));
        // And how far it moved: the compositor's against what the client
        // committed, the input path's against its own anchor.
        assert_eq!(
            latched.moved_x_x65536(),
            i64::from(predicted.x_x65536()) - i64::from(predicted.anchor_x_x65536())
        );
        assert_eq!(
            latched.moved_y_x65536(),
            i64::from(predicted.y_x65536()) - i64::from(predicted.anchor_y_x65536())
        );
        assert_eq!(latched.lead_nanos(), predicted.lead_nanos());
        assert!(latched.extrapolated());
        // And the movement is not zero, which is what makes the two equalities
        // above statements rather than two ways of writing the same sample.
        assert!(latched.moved_x_x65536() > 0, "the prediction ran the cursor nowhere");
        assert_eq!(latch.aimed_at_nanos(), scanout_nanos);
    }

    /// A decoded event is a report when it carries a position, and nothing
    /// else when it does not — and the reading it becomes is the one it carried.
    #[test]
    fn an_event_off_the_ring_is_a_report_only_when_it_is_a_position() {
        use f_abi::input::{Entry, Key, PointerMotion, edge};
        let event = |stamp_nanos: u64, body: Entry| Event {
            user_data: 0,
            class: 0,
            payload_offset: 0,
            flags: f_abi::flags::NO_CQE,
            stamp_nanos,
            body,
        };
        let motion = |x: i32| Entry::PointerMotion(PointerMotion { x_x65536: x, y_x65536: -x });

        let mut latch = LateLatch::riding(POINTER);
        assert!(latch.drained(&event(COMMITTED_AT_NANOS, motion(COMMITTED_X_X65536))));
        assert!(!latch.drained(&event(
            INJECTED_AT_NANOS,
            Entry::Key(Key { code: 30, transition: edge::PRESSED })
        )));
        // Not newer than what is held: stale, and counted as such.
        assert!(!latch.drained(&event(COMMITTED_AT_NANOS, motion(0))));
        // A hand-built event with no reading on it is refused here as well as
        // at the decoder.
        assert!(!latch.drained(&event(NOT_STAMPED, motion(0))));
        assert_eq!((latch.reports(), latch.stale(), latch.unstamped()), (1, 1, 1));

        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);
        let latched = latch
            .latch(&mut graph, 0, Aim { scanout_nanos: COMMITTED_AT_NANOS })
            .expect("one report latches");
        assert_eq!(latched.latched_tx_x65536(), i64::from(COMMITTED_X_X65536));
        assert_eq!(latched.latched_ty_x65536(), -i64::from(COMMITTED_X_X65536));
    }

    /// **The arithmetic `cargo xtask input deliver` rests on.** The motion that
    /// harness injects, accumulated as the driver accumulates it and stamped as
    /// the driver stamps it, is held rather than extrapolated at every scanout
    /// the boot's compositor could aim at — so the latched translation is the
    /// newest position exactly, and *latched minus committed* is the motion.
    ///
    /// Why it holds is the motion's shape and not luck: along x it runs out by
    /// ten pixels and back by six inside the predictor's window, and a window
    /// whose two halves disagree about direction extrapolates nothing —
    /// `f_input::predict::Held::Reversed`. The boot's verdict requires the
    /// latch to report *held*, and this is the test that says it will; a
    /// harness motion changed to a steady one would turn that clause red, and
    /// the repair is then `E3-B04e`'s arithmetic in the verdict rather than a
    /// weaker clause.
    ///
    /// The motion is written twice — here and in `xtask`'s `MOTIONS` — because
    /// a component test cannot read a host tool's constant. The boot is what
    /// catches the two drifting apart: its verdict refuses an extrapolated latch.
    #[test]
    fn the_boots_motion_is_held_and_not_extrapolated() {
        const MOTIONS: [(i32, i32); 5] = [(3, 5), (-1, 2), (10, -4), (2, 7), (-6, -6)];
        // `kernel/src/input.rs`'s `STAMP_TICK_NANOS`: the driver's clock
        // advances one tick before each report's reading.
        const TICK_NANOS: u64 = 100_000;

        let mut latch = LateLatch::riding(POINTER);
        let (mut x_x65536, mut y_x65536) = (0i32, 0i32);
        for (at, (dx, dy)) in MOTIONS.iter().enumerate() {
            x_x65536 += dx * 65_536;
            y_x65536 += dy * 65_536;
            assert!(latch.observed(Reading {
                at_nanos: TICK_NANOS * (at as u64 + 1),
                x_x65536,
                y_x65536,
            }));
        }
        assert_eq!((x_x65536, y_x65536), (8 * 65_536, 4 * 65_536), "the harness's sum");

        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);
        // At the newest report, just after it, a whole frame after it, and past
        // the lead ceiling: every instant a sixty-hertz compositor could aim a
        // first frame at, whatever its clock read when the commit landed.
        for scanout_nanos in [
            TICK_NANOS * MOTIONS.len() as u64,
            TICK_NANOS * MOTIONS.len() as u64 + 1,
            PERIOD_NANOS,
            2 * PERIOD_NANOS,
        ] {
            let latched = latch
                .latch(&mut graph, 0, Aim { scanout_nanos })
                .expect("the boot's motion latches");
            assert!(!latched.extrapolated(), "extrapolated when aimed at {scanout_nanos}");
            assert_eq!(latched.lead_nanos(), 0);
            assert_eq!(latched.latched_tx_x65536(), i64::from(x_x65536));
            assert_eq!(latched.latched_ty_x65536(), i64::from(y_x65536));
            // Four latches over one graph, and every one of them against the
            // client's transform — which this loop never asked until the audit
            // found a latch that read its own last patch here.
            assert_eq!(
                (latched.committed_tx_x65536(), latched.committed_ty_x65536()),
                (i64::from(COMMITTED_X_X65536), i64::from(COMMITTED_Y_X65536)),
                "aimed at {scanout_nanos}, the latch compared against its own previous patch"
            );
            assert!(latch.restore(&mut graph), "the graph refused the client's own transform");
        }
        assert_eq!(latch.restores(), latch.latches());
    }

    /// One way to change the committed frame, for the test below.
    type Change = fn(&mut Arena);

    /// Nothing, as a [`Change`].
    fn unchanged(_: &mut Arena) {}

    /// The pointer's committed transform with one field moved.
    fn moved_pointer(graph: &mut Arena, change: fn(SetTransform) -> SetTransform) {
        let committed = committed(COMMITTED_X_X65536, COMMITTED_Y_X65536);
        graph.set_transform(change(committed)).expect("the pointer is in the graph");
    }

    /// The path the path rows start from.
    const PATH: SetPath =
        SetPath { node: POINTER, geometry_offset: 64, geometry_bytes: 32, fill_rule: 1 };

    /// The paint the paint rows start from.
    const PAINT: SetPaint = SetPaint {
        node: STILL,
        red_x65535: 1,
        green_x65535: 2,
        blue_x65535: 3,
        alpha_x65535: 4,
        stroke_width_x65536: 5,
    };

    fn with_path(graph: &mut Arena) {
        graph.set_path(PATH).expect("the pointer is in the graph");
    }

    fn with_paint(graph: &mut Arena) {
        graph.set_paint(PAINT).expect("the sibling is in the graph");
    }

    /// The committed frame, then `setup`, then `change`, folded with the
    /// pointer's translation masked.
    fn folded(setup: Change, change: Change, masked: u32) -> (u64, u32) {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);
        setup(&mut graph);
        change(&mut graph);
        unmoved(&graph, masked)
    }

    /// **The instrument the boot carries *and by nothing else* with, driven to
    /// red.** Every field [`unmoved`] folds is changed once against a baseline
    /// that already holds the record the field belongs to, and every change but
    /// the two it masks must move the word — and those two must not. A walker
    /// that dropped a field, masked more than two numbers, or masked the wrong
    /// node fails here rather than in a boot that would have gone green over it.
    #[test]
    fn the_unmoved_fold_sees_every_field_but_the_two_it_masks() {
        let rows: [(&str, bool, Change, Change); 18] = [
            ("tx", false, unchanged, |g| {
                moved_pointer(g, |t| SetTransform { tx_x65536: t.tx_x65536 + 1, ..t })
            }),
            ("ty", false, unchanged, |g| {
                moved_pointer(g, |t| SetTransform { ty_x65536: t.ty_x65536 - 1, ..t })
            }),
            ("a", true, unchanged, |g| {
                moved_pointer(g, |t| SetTransform { a_x65536: t.a_x65536 + 1, ..t })
            }),
            ("b", true, unchanged, |g| {
                moved_pointer(g, |t| SetTransform { b_x65536: t.b_x65536 + 1, ..t })
            }),
            ("c", true, unchanged, |g| {
                moved_pointer(g, |t| SetTransform { c_x65536: t.c_x65536 + 1, ..t })
            }),
            ("d", true, unchanged, |g| {
                moved_pointer(g, |t| SetTransform { d_x65536: t.d_x65536 + 1, ..t })
            }),
            ("a sibling's transform", true, unchanged, |g| {
                let t = committed(COMMITTED_X_X65536, COMMITTED_Y_X65536);
                g.set_transform(SetTransform { node: STILL, ..t }).expect("the sibling");
            }),
            ("a path", true, unchanged, with_path),
            ("the path's offset", true, with_path, |g| {
                g.set_path(SetPath { geometry_offset: 65, ..PATH }).expect("the pointer");
            }),
            ("the path's length", true, with_path, |g| {
                g.set_path(SetPath { geometry_bytes: 33, ..PATH }).expect("the pointer");
            }),
            ("the path's rule", true, with_path, |g| {
                g.set_path(SetPath { fill_rule: 2, ..PATH }).expect("the pointer");
            }),
            ("a paint", true, unchanged, with_paint),
            ("red", true, with_paint, |g| {
                g.set_paint(SetPaint { red_x65535: 9, ..PAINT }).expect("the sibling");
            }),
            ("green", true, with_paint, |g| {
                g.set_paint(SetPaint { green_x65535: 9, ..PAINT }).expect("the sibling");
            }),
            ("blue", true, with_paint, |g| {
                g.set_paint(SetPaint { blue_x65535: 9, ..PAINT }).expect("the sibling");
            }),
            ("alpha", true, with_paint, |g| {
                g.set_paint(SetPaint { alpha_x65535: 9, ..PAINT }).expect("the sibling");
            }),
            ("stroke", true, with_paint, |g| {
                g.set_paint(SetPaint { stroke_width_x65536: 9, ..PAINT }).expect("the sibling");
            }),
            ("a node", true, unchanged, |g| {
                let (entry, payload) = wire(
                    Entry::CreateNode(CreateNode {
                        node: 9,
                        parent: ROOT,
                        before: NO_NODE,
                        kind: kind::TRANSFORM,
                    }),
                    0,
                );
                let delta = Delta::decode(&entry, &payload).expect("a node delta");
                g.apply(&delta).expect("the graph takes a ninth node");
            }),
        ];
        for (what, moves, setup, change) in rows {
            let (baseline, walked) = folded(setup, unchanged, POINTER);
            assert_eq!(walked, 3, "{what}: the baseline walked the three nodes");
            let (after, _) = folded(setup, change, POINTER);
            if moves {
                assert_ne!(after, baseline, "{what} changed and the fold did not move");
            } else {
                assert_eq!(after, baseline, "{what} is masked and the fold moved");
            }
        }

        // And the mask is one node's: the same frame masked on a different node
        // folds the pointer's translation.
        assert_ne!(
            folded(unchanged, unchanged, STILL).0,
            folded(unchanged, unchanged, POINTER).0,
            "the mask followed the wrong node"
        );
    }

    /// A latch onto a node the client removed is declined, and the frame goes
    /// out with the scene the client committed.
    #[test]
    fn a_node_the_graph_does_not_hold_is_declined_rather_than_refused() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);

        let mut latch = LateLatch::riding(POINTER + 100);
        assert!(latch.observed(Reading {
            at_nanos: INJECTED_AT_NANOS,
            x_x65536: COMMITTED_X_X65536,
            y_x65536: COMMITTED_Y_X65536,
        }));
        assert_eq!(
            latch.latch(&mut graph, 0, Aim { scanout_nanos: INJECTED_AT_NANOS }),
            Err(Declined::NoTransform)
        );
        assert_eq!(latch.declines(), 1);
    }

    /// **A page the frame never wrote is not a pointer at the origin.**
    ///
    /// The scar. `f_abi::input::NOT_STAMPED`'s own doc names this scene — *a
    /// pointer at the origin at the beginning of time, which is a scene the
    /// compositor would draw* — and this component read the routing page every
    /// turn and handed whatever was there to its predictor. On a boot whose
    /// frame writes no position at all, `cargo xtask input deliver` printed *1
    /// report(s) taken, 1 frame(s) latched*, and the latched transform put the
    /// client's node at the origin.
    ///
    /// No test caught it, because every test in this module reports a position
    /// before it asks for one. This is the one that starts from a zeroed page,
    /// which is the state every run starts in.
    #[test]
    fn a_page_the_frame_never_wrote_is_not_a_pointer_at_the_origin() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);

        let mut before = Encoder::EMPTY;
        image(&graph, &mut before);

        let mut latch = LateLatch::riding(POINTER);
        // Exactly what `component::serve` reads out of a page nobody has
        // written: three zeroes.
        assert!(!latch.observed(Reading::default()), "a zeroed page became a report");
        assert_eq!(latch.reports(), 0);
        assert_eq!(latch.stale(), 0, "an absent reading is not a repeated one");
        assert_eq!(latch.unstamped(), 1);
        assert_eq!(
            latch.latch(&mut graph, 0, Aim { scanout_nanos: INJECTED_AT_NANOS }),
            Err(Declined::NoReport)
        );

        let mut after = Encoder::EMPTY;
        image(&graph, &mut after);
        assert_eq!(
            before.bytes(),
            after.bytes(),
            "a page of zeroes moved the client's node to the origin"
        );
    }

    /// A pointer the device has never reported is not latched to the origin.
    #[test]
    fn a_pointer_that_has_reported_nothing_is_not_placed_at_the_origin() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        commit_a_frame(&mut graph, &mut batch);

        let mut before = Encoder::EMPTY;
        image(&graph, &mut before);

        let mut latch = LateLatch::riding(POINTER);
        assert_eq!(
            latch.latch(&mut graph, 0, Aim { scanout_nanos: INJECTED_AT_NANOS }),
            Err(Declined::NoReport)
        );

        let mut after = Encoder::EMPTY;
        image(&graph, &mut after);
        assert_eq!(before.bytes(), after.bytes());
    }
}
