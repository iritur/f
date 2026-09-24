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
//! it; every such answer in this module is [`Reading::at_nanos`], which is the
//! driver's stamp carried across two rings unchanged.
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

use f_abi::input::NOT_STAMPED;
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
    /// the wire for the same reason, and this is that refusal at the one place
    /// a position reaches this component without crossing a ring.
    /// Unit: readings.
    unstamped: u64,
    /// Frames that carried a latch. Unit: frames.
    latches: u64,
    /// Frames that did not. Unit: frames.
    declines: u64,
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
        let taken = self.predictor.observe(Sample {
            at,
            x_x65536: reading.x_x65536,
            y_x65536: reading.y_x65536,
        });
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
        if graph.set_transform(patched).is_err() {
            return Err(Declined::Refused);
        }
        Ok(Latched {
            node: self.node,
            before_entry: u32::try_from(entered).unwrap_or(u32::MAX),
            committed_tx_x65536: committed.tx_x65536,
            committed_ty_x65536: committed.ty_x65536,
            latched_tx_x65536: patched.tx_x65536,
            latched_ty_x65536: patched.ty_x65536,
            lead_nanos: predicted.lead_nanos(),
            extrapolated: predicted.basis().extrapolated(),
        })
    }
}

#[cfg(test)]
mod tests {
    use f_abi::Sqe;
    use f_abi::scene::{Commit, CreateNode, Delta, Entry, PAYLOAD_BYTES, kind};
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
    /// it, and the frame that reaches the graph is the client's frame with one
    /// transform patched.
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
        assert_eq!(
            held.graph().transform_of(POINTER),
            Ok(Some(committed(
                latched.latched_tx_x65536() as i32,
                latched.latched_ty_x65536() as i32
            ))),
            "the graph does not hold what the record says was submitted"
        );
        assert_eq!(
            held.graph().transform_of(STILL),
            Ok(None),
            "a node nobody named acquired a transform"
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
    /// an instance, and what that buys is stated rather than overclaimed: the
    /// comparison catches a relay that dropped a report, reordered two, handed
    /// on a position the device never reported, or committed a transform that is
    /// not the newest report — and it does not catch a defect inside the
    /// predictor, which is `input/src/predict.rs`'s own bounded corpus.
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
