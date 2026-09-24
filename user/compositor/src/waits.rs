// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The compositor's end of section 08's chain, and one frame's waits recorded.
//!
//! # What this closes, said as the file that opened it said it
//!
//! `abi/src/trace.rs` names the direction that does not hold from `abi/`: *nothing
//! in a type makes a frame hand every submission to a recorder.* What it holds
//! instead is narrower and exact — the chain's two waiting doors,
//! [`Chain::compositor_waits_and_signals`] and [`Chain::present_waits`], take a
//! `&mut Trace` and record into it, so a frame **driven through the chain** cannot
//! enter a wait the trace does not name. This file is the driving. It is the only
//! place in this component that holds a [`Chain`], and every wait it enters goes
//! through one of those two doors, which is why the sentence *no implicit wait
//! appears in this compositor's frame trace* is a property of one small module
//! rather than of a reviewer's diligence.
//!
//! [`Trace::EMPTY`] is the first statement of [`Waits::frame`] and there is no
//! path past it. That placement is a decision and not tidiness: a frame's trace
//! has to be that frame's, and the obvious alternative — resetting where the
//! frame *opens*, at the first delta staged — is wrong here for a reason a reader
//! would not guess. `crate::tree::Held::offer` sets its open instant only when a
//! delta is **staged**, and a commit that arrives with no delta before it stages
//! nothing and still closes a frame. A reset hung on the first delta would carry
//! the previous frame's entries into that frame's record, and the record would
//! look complete. So the reset is where the frame is *identified*, which is the
//! close.
//!
//! # Whose progress this records, and whose it does not
//!
//! Both timelines are advanced from this side, and that asymmetry has to be
//! stated rather than left for a reader to find.
//!
//! The **compositor's** half is this component's own: it promises `m`, submits the
//! signal of it, and reaches it or does not. Nothing is assumed there.
//!
//! The **application's** half is advanced from *the arrival of the client's
//! commit*, which is a fact this component observes rather than one it is told.
//! A closed commit is the client's deltas having landed — that is what
//! `f_scene::commit::Batch::commit` answering `Ok` means — so
//! [`Chain::application_signals`] here records a signal that happened, not one
//! this file invented. What it is **not** is the client's own record: nothing on
//! this wire carries a `Submission`, so there is no `Timeline` arriving from the
//! application to adopt. *What would reverse this:* a client that publishes its
//! own timeline record, at which point the application's half arrives through
//! [`Timeline::adopt`] — whose own documentation already names this file as its
//! second caller — and this module stops advancing a timeline it does not own.
//!
//! # A timeout, and why this one is honest
//!
//! There is no timer in this component and no clock in it. RFC 0004 gives a
//! component neither, and a field that could only move if one existed would be a
//! published zero pretending to be a measurement — which is the failure
//! `E3-B01k` was written against and the reason `crate::routing::node::DEGRADED`
//! is a choice rather than a boolean.
//!
//! So a timeout here is defined out of two numbers this component already has and
//! is already checked on: **a wait still outstanding when the frame closed, on a
//! frame whose own cost estimate did not fit before its own deadline.** The
//! deadline is the client's, off the wire; the estimate is
//! `crate::pacing::Pacing`'s p99; and the comparison is the one
//! `crate::pacing::degraded` already makes. Nothing new observes anything.
//!
//! What makes the count reachable is that the compositor does not land its own
//! signal for such a frame, and that is the arguable part, so here is the
//! argument. The compositor's `m` means *this frame is composited*. Nothing in
//! this build draws — `E3-B02` owes the pixels — so there is no instant at which
//! `m` is genuinely reached, and the component has to decide. The decision it
//! takes is the one the pipeline would take: a frame that could not fit before
//! its deadline has missed the scanout it was paced against, so the value
//! promised for it is never reached and the present engine's wait on it stays
//! open. That is precisely the hang `abi/src/trace.rs` says it cannot prevent and
//! can only record — *a producer that promised a value and then died before
//! reaching it* — and the trace is what makes it readable.
//!
//! *What would reverse this:* a renderer. The day `E3-B02` exists, the landing
//! comes from the rasteriser finishing rather than from this comparison, and the
//! two lines in [`Waits::frame`] that decide it are deleted rather than adjusted.
//! Nothing else in this file moves.
//!
//! # What this is not
//!
//! It is not a restart policy. `E3-B05e` — *a timeout is a named fate, the
//! supervisor decides it* — is a different line, and RFC 0008 puts the deciding
//! above the frame. What this module does for it is supply the reading it would
//! decide on: a supervisor that has to choose a fate for a stuck frame needs to
//! know that a wait is outstanding, which value was last reached, and how many
//! times that has happened, and those are exactly the three words
//! `crate::routing::node` now carries. Nothing here acts on any of them.
//!
//! There is no floating point here. A timeline value is an ordinal, a wait count
//! is a count of waits, and the two spans are nanoseconds with their scale in
//! their names. RFC 0004.

use f_abi::sync::{Chain, Refusal, Timeline};
use f_abi::trace::Trace;

/// What happens in the one gap a frame has between its commit closing and its
/// first submission crossing.
///
/// # Why this is a parameter and not a call in [`Waits::frame`]
///
/// Because the window is the whole of `E3-B01i` and a comment saying *here* is
/// not a window. [`Waits::drive`] is the only place in this component that knows
/// the order of section 08's chain, so it is the only place that can hand out a
/// moment defined by that order — and handing it out as an argument means a
/// build that latched too early or too late has moved a line in a function whose
/// every statement is one of the chain's, in front of a reviewer, rather than
/// having drifted somewhere else in the component.
///
/// It also keeps [`Waits`] the only holder of the chain and the trace, which is
/// the one sentence `crate::waits` exists to be able to say. The implementor
/// gets a number — how many wait entries the trace holds at that instant — and
/// not the trace, so a door that records cannot be opened from here.
///
/// `()` implements it as nothing at all, which is what a test of the chain
/// alone passes.
pub trait Between {
    /// Called after the application's signal is admitted and before the
    /// compositor's own submission enters its wait.
    ///
    /// `entered` is how many entries the frame's trace holds at that instant. It
    /// is zero in this build, and it is passed rather than assumed because the
    /// day a stage submits before the compositor does, the record keyed with it
    /// says so and a test asserting zero goes red. Unit: wait entries.
    fn between_commit_and_submit(&mut self, entered: usize);
}

impl Between for () {
    fn between_commit_and_submit(&mut self, _entered: usize) {}
}

/// The application's timeline, as this component names it.
///
/// One rather than zero, because `f_abi::sync::NO_TIMELINE` is zero and a
/// timeline that decoded out of an untouched word would otherwise be the
/// application's. It is a local name and not a wire identity: nothing on this
/// wire carries a timeline record, so nobody can disagree with the number yet.
/// The day a client sends one, this constant becomes the identifier the two
/// peers have to agree on, and that is the day it belongs in `crate::routing`
/// beside the other things both sides read.
/// Unit: none — a timeline identifier, not a quantity.
pub const APPLICATION: u32 = 1;

/// The compositor's own timeline. See [`APPLICATION`].
/// Unit: none — a timeline identifier, not a quantity.
pub const COMPOSITOR: u32 = 2;

/// The chain this component starts with: two timelines that have promised
/// nothing and signalled nothing.
///
/// A `const` and not a constructor, which is what makes the two refusals
/// `f_abi::sync` states here build failures rather than run-time answers. Both
/// are about this file's own constants — an identifier of `NO_TIMELINE`, and one
/// timeline named twice — so neither can depend on anything that happens at run
/// time, and a `Result` threaded out to a component's entry point would be a
/// component deciding at ring 3 what a compiler already knows.
const CHAIN: Chain = {
    let application = match Timeline::declare(APPLICATION) {
        Ok(timeline) => timeline,
        Err(_) => panic!("the application's timeline identifier is NO_TIMELINE"),
    };
    let compositor = match Timeline::declare(COMPOSITOR) {
        Ok(timeline) => timeline,
        Err(_) => panic!("the compositor's timeline identifier is NO_TIMELINE"),
    };
    match Chain::new(application, compositor) {
        Ok(chain) => chain,
        Err(_) => panic!("the compositor's two timelines are one timeline"),
    }
};

/// What this component publishes about its own synchronisation.
///
/// Three of these have a node in `crate::routing::node` and four do not, and the
/// division is `crate::routing::reported`'s own: the three say what a reader of a
/// *running machine* wants — *is anything still waiting, what has been reached,
/// and how often has a frame been abandoned* — and the four are about the
/// **record's own integrity**, which is evidence about this component and belongs
/// where the frame checks it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// Waits the last frame entered and did not get out of.
    ///
    /// `f_abi::trace::Trace::unreleased`, over the trace of the last frame that
    /// closed. A gauge and not a total: it is the state of one frame, and a
    /// running total would answer *has this ever happened* where a reader of a
    /// live machine is asking *is it happening now*. [`Published::timeouts`] is
    /// the total, and the two are published together for that reason.
    /// Unit: waits.
    pub outstanding: u64,
    /// The highest value that has landed on the compositor's **own** timeline.
    ///
    /// `f_abi::sync::Timeline::signalled`, which moves at the landing and never
    /// at the submission — that is the field's own rule and the reason this is
    /// worth publishing rather than deriving from the frame count: a component
    /// that had submitted three signals and reached one would publish three
    /// frames and a one here.
    ///
    /// The compositor's and not the application's, because it is the only
    /// timeline this component produces. The application's is advanced from the
    /// arrival of a commit, so republishing it would be republishing
    /// `crate::routing::node::FRAMES` under a second name.
    /// Unit: none — a timeline value, which is an ordinal.
    pub signalled: u64,
    /// Frames that closed with a wait outstanding and no room left to satisfy
    /// it.
    ///
    /// The module header defines this and argues it. A counter, because a frame
    /// that was abandoned stays abandoned however the next one goes.
    /// Unit: frames — UI frames.
    pub timeouts: u64,
    /// Waits the traces of every frame so far have named, summed.
    ///
    /// Two per frame in this build — the compositor's on the application's
    /// timeline and the present engine's on the compositor's — so the frame can
    /// require exactly that and catch a stage that waited and recorded nothing.
    /// It is the count `E3-B05b`'s *names every wait* is checkable through: a
    /// trace with the right `unreleased` and half the entries would satisfy every
    /// other word here.
    /// Unit: waits.
    pub traced: u64,
    /// Waits the traces could not hold, summed.
    ///
    /// `f_abi::trace::Trace::dropped_waits`. Non-zero means the bound's
    /// derivation has stopped being true — a fourth waiting stage, or one stage
    /// submitting twice — and RFC 0101 is why it is published rather than
    /// asserted: the decay is silent and surfaces three subsystems away, and this
    /// is the record whose subject it is.
    /// Unit: waits.
    pub dropped: u64,
    /// One while every trace so far has named every wait its frame entered.
    ///
    /// `f_abi::trace::Trace::complete`, and held across frames rather than read
    /// off the last one: a build that dropped a wait in the first frame and none
    /// afterwards would publish a clean last trace. Read beside
    /// [`Published::dropped`] for `crate::routing::reported::CLEAN`'s reason —
    /// a flag and a count that cannot disagree without one of them being
    /// published unread.
    /// Unit: none — a flag.
    pub complete: u64,
    /// Refusals this component's own chain produced.
    ///
    /// Zero, always, on a build whose arithmetic is right: every value offered to
    /// the chain here is this file's own frame ordinal, and the refusals
    /// `f_abi::sync` states are about a value that is not monotonic, not
    /// promised, or above a ceiling. A non-zero word is this component
    /// contradicting itself, which the frame requires to be zero — and which is
    /// why the refusal is counted rather than ignored or turned into a panic: a
    /// compositor that stopped serving a client because its own bookkeeping
    /// disagreed would have turned an accounting defect into a black screen.
    /// Unit: refusals.
    pub refusals: u64,
}

/// The chain, the trace of the frame that just closed, and what the two publish.
///
/// # Why it is a type and not three fields of `Held`
///
/// Because the invariant is between them. A trace belongs to one frame and the
/// chain outlives every frame, so a build that held the two side by side has to
/// remember to reset one and not the other — and the failure is silent, because a
/// stale trace decodes and reads as a complete record of the wrong frame.
/// [`Waits::frame`] is the only thing that touches either, so there is one place
/// the pairing can be got wrong and it is nine lines long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waits {
    /// Section 08's chain: the application's timeline and the compositor's.
    chain: Chain,
    /// The waits of the frame that closed last.
    ///
    /// All zeroes when it is [`Trace::EMPTY`], which is not a curiosity here but
    /// the reason this field costs a component nothing: RFC 0100 found that one
    /// enum numbered from zero cost this image 131 KiB, and `f_abi::trace::Stage`
    /// is numbered from one precisely so that `Option<Waited>`'s niche is at zero
    /// and an empty trace is a `memset` rather than a copy out of `.rodata`.
    trace: Trace,
    /// What the two of them publish.
    published: Published,
}

impl Waits {
    /// A compositor that has synchronised nothing.
    ///
    /// A `const`, so that the whole of this type's starting state is a zero
    /// initialiser and none of it is decided at run time. `Chain` is two
    /// timelines with a zero ceiling each, which admits no wait at all —
    /// `f_abi::sync::Timeline::declare` says why that is the right start rather
    /// than an awkward one.
    pub const ZERO: Self = Self {
        chain: CHAIN,
        trace: Trace::EMPTY,
        // Written field by field rather than as an update of a zeroed constant,
        // so that a field added to [`Published`] and forgotten here is a build
        // failure that names the field. Every one of them is zero except
        // `complete`: a component that has traced nothing has dropped nothing,
        // and starting that flag at zero would publish *incomplete* about a
        // compositor nobody had asked anything of.
        published: Published {
            outstanding: 0,
            signalled: 0,
            timeouts: 0,
            traced: 0,
            dropped: 0,
            complete: 1,
            refusals: 0,
        },
    };

    /// What this component publishes about its own synchronisation.
    #[must_use]
    pub const fn published(&self) -> &Published {
        &self.published
    }

    /// The trace of the frame that closed last.
    ///
    /// Shared and never exclusive, for `crate::tree::Held::graph`'s reason: a
    /// `&mut Trace` handed out here would be a second place a wait could be
    /// recorded, and *every wait went through the chain* is the one sentence this
    /// module exists to be able to say.
    #[must_use]
    pub const fn trace(&self) -> &Trace {
        &self.trace
    }

    /// The chain as it stands, for a reader that wants the ceilings too.
    #[must_use]
    pub const fn chain(&self) -> &Chain {
        &self.chain
    }

    /// One closed frame's worth of synchronisation.
    ///
    /// `ordinal` is which frame this is — the count of frames this component has
    /// closed, including this one — and it is the value both timelines take.
    /// `fitted` is whether the frame's own cost estimate fitted before the
    /// deadline the client put on it, which `crate::tree::Held::close` has
    /// already decided and which this function does not re-derive: two copies of
    /// that comparison would be two opinions about one frame, and
    /// `crate::routing::reported::LATE` is where the frame checks the other one.
    ///
    /// The three chain calls are in section 08's order and every one of them goes
    /// through a door that takes the trace. There is no fourth call and no
    /// `Submission::EMPTY` in this file, which is what the module header means by
    /// the sentence being a property of the module.
    ///
    /// A refusal is counted and the frame goes on. The whole run is this
    /// component's own arithmetic over its own frame ordinals, so a refusal here
    /// is a defect in this file rather than a client's mistake — and a compositor
    /// that stopped serving over one would have turned a wrong number into a
    /// stopped screen. The frame requires [`Published::refusals`] to be zero,
    /// which is where a defect is supposed to be found.
    pub fn frame(&mut self, ordinal: u64, fitted: bool, between: &mut dyn Between) {
        // First, and with nothing before it. The module header argues the
        // placement against the alternative that looks more natural.
        self.trace = Trace::EMPTY;
        if let Err(_refusal) = self.drive(ordinal, fitted, between) {
            self.published.refusals += 1;
        }
        self.published.outstanding = self.trace.unreleased() as u64;
        self.published.signalled = self.chain.compositor().signalled();
        self.published.traced += self.trace.len() as u64;
        self.published.dropped += u64::from(self.trace.dropped_waits());
        if !self.trace.complete() {
            self.published.complete = 0;
        }
        // A frame abandoned: a wait this frame entered and did not get out of, on
        // a frame that had no room left. Both halves, because either alone would
        // count something else — an outstanding wait on a frame that fitted is an
        // ordinary pipeline still running, and a frame that did not fit with
        // every wait released is a frame that was late and got there anyway.
        if self.published.outstanding > 0 && !fitted {
            self.published.timeouts += 1;
        }
    }

    /// The three submissions, in order, through the chain.
    ///
    /// Split out so that [`Waits::frame`]'s bookkeeping runs whatever this
    /// answers: a refusal half way through leaves a trace with one entry in it,
    /// and a reader is owed that entry rather than the previous frame's record.
    ///
    /// # Errors
    ///
    /// Whatever `f_abi::sync` refuses. Every one of them is this component
    /// contradicting itself — see [`Published::refusals`].
    fn drive(
        &mut self,
        ordinal: u64,
        fitted: bool,
        between: &mut dyn Between,
    ) -> Result<(), Refusal> {
        // The application reached `ordinal`, which is the client's commit having
        // arrived. The submission is built and dropped: this component is not the
        // application and has nothing to submit on its behalf, and what the call
        // is here for is the promise it raises — without which the wait below is
        // refused as unreachable, which is `f_abi::sync`'s whole point.
        let _signalled = self.chain.application_signals(ordinal)?;
        // The window, and it is here rather than a statement earlier or later for
        // a reason each neighbour states. Earlier is before the commit has been
        // admitted at all — a frame whose own signal the chain refuses has no
        // frame to patch, and `a_frame_ordinal_that_does_not_move_is_refused_and
        // _counted` is the case that would then latch onto a frame that never
        // happened. Later is after a submission has crossed, which is what *late
        // latch* means the opposite of. `Between`'s own doc is the rest.
        between.between_commit_and_submit(self.trace.len());
        let _composited =
            self.chain.compositor_waits_and_signals(ordinal, ordinal, &mut self.trace)?;
        let _presented = self.chain.present_waits(ordinal, &mut self.trace)?;
        // The application's signal really landed, because the commit really
        // arrived. This releases the compositor's own wait in the trace.
        self.chain.landed(APPLICATION, ordinal, &mut self.trace)?;
        // And the compositor's, only for a frame that fitted. The module header
        // is the argument and names what deletes these two lines.
        if fitted {
            self.chain.landed(COMPOSITOR, ordinal, &mut self.trace)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use f_abi::trace::Stage;

    use super::*;

    /// A compositor that has done nothing says so, and says the one thing that is
    /// not a zero.
    #[test]
    fn nothing_synchronised_is_zero_and_complete() {
        let waits = Waits::ZERO;
        assert_eq!(waits.published().outstanding, 0);
        assert_eq!(waits.published().signalled, 0);
        assert_eq!(waits.published().timeouts, 0);
        assert_eq!(waits.published().traced, 0);
        assert_eq!(waits.published().dropped, 0);
        assert_eq!(waits.published().refusals, 0);
        // A component that has traced nothing has dropped nothing. The alternative
        // — starting at zero and being set on the first complete frame — would
        // publish *incomplete* for a compositor nobody had asked anything of.
        assert_eq!(waits.published().complete, 1);
        assert!(waits.trace().is_empty());
    }

    /// One frame that fitted leaves nothing outstanding, and the two waits it
    /// entered are both in the record.
    #[test]
    fn a_frame_that_fitted_leaves_no_wait_open() {
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        assert_eq!(waits.trace().len(), 2, "the compositor's wait and the present engine's");
        assert_eq!(waits.published().traced, 2);
        assert_eq!(waits.published().outstanding, 0);
        assert_eq!(waits.published().signalled, 1);
        assert_eq!(waits.published().timeouts, 0);
        assert_eq!(waits.published().refusals, 0);
        assert_eq!(waits.published().complete, 1);
    }

    /// The two waits are the two stages section 08 names, and neither is the
    /// application's — which has no wait, because it is the first stage.
    #[test]
    fn the_waits_are_the_two_stages_that_wait() {
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        let first = waits.trace().entry(0).expect("the compositor's wait");
        let second = waits.trace().entry(1).expect("the present engine's wait");
        assert_eq!(first.waiter(), Stage::Compositor);
        assert_eq!(first.timeline(), APPLICATION, "the compositor waits on the application's");
        assert_eq!(second.waiter(), Stage::Present);
        assert_eq!(second.timeline(), COMPOSITOR, "the present engine waits on the compositor's");
        assert!(waits.trace().entry(2).is_none(), "a third wait nobody entered");
    }

    /// A frame that did not fit leaves the present engine's wait open and counts
    /// the frame as abandoned — and the compositor's own timeline does **not**
    /// move, which is the half a reader would otherwise have to take on trust.
    #[test]
    fn a_frame_that_did_not_fit_leaves_the_present_engine_waiting() {
        let mut waits = Waits::ZERO;
        waits.frame(1, false, &mut ());
        assert_eq!(waits.published().outstanding, 1);
        assert_eq!(waits.published().timeouts, 1);
        assert_eq!(waits.published().signalled, 0, "the value promised for it was never reached");
        assert_eq!(waits.published().traced, 2, "both waits are still named");
        assert_eq!(waits.published().refusals, 0);
        let second = waits.trace().entry(1).expect("the present engine's wait");
        assert!(!second.released(), "the wait that is the hang");
        let first = waits.trace().entry(0).expect("the compositor's wait");
        assert!(first.released(), "the commit arrived, so the application's signal landed");
    }

    /// Two frames, the second abandoned, which is the serving boot's shape.
    ///
    /// The clause with teeth is the last one: `signalled` is **one** after two
    /// frames. A build that moved the timeline at the submission rather than at
    /// the landing would say two, and every other word here would agree with it.
    #[test]
    fn the_second_frame_abandoned_leaves_the_first_frames_value_reached() {
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        waits.frame(2, false, &mut ());
        assert_eq!(waits.published().traced, 4, "two waits per frame, over two frames");
        assert_eq!(waits.published().outstanding, 1);
        assert_eq!(waits.published().timeouts, 1);
        assert_eq!(waits.published().signalled, 1);
        assert_eq!(waits.published().complete, 1);
    }

    /// A frame that fits after an abandoned one reaches its own value, and the
    /// outstanding count goes back to zero — which is the wake boot's shape and
    /// the reason `outstanding` is a gauge.
    #[test]
    fn a_frame_after_an_abandoned_one_reaches_its_own_value() {
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        waits.frame(2, false, &mut ());
        waits.frame(3, true, &mut ());
        assert_eq!(waits.published().outstanding, 0, "this frame's record, not the run's");
        assert_eq!(waits.published().timeouts, 1, "the run's total, which does not go back");
        assert_eq!(waits.published().signalled, 3, "and the skipped value is skipped");
        assert_eq!(waits.published().traced, 6);
        assert_eq!(waits.published().refusals, 0);
    }

    /// Every frame starts with an empty record.
    ///
    /// The assertion is on `len`, and it is the one that fails when the reset
    /// moves: a build that reset nowhere would reach four entries after two
    /// frames and the `unreleased` count of the second frame would include the
    /// first frame's released waits — which read correctly, which is what makes
    /// the absence of a reset survivable and therefore worth a test.
    #[test]
    fn each_frame_traces_its_own_waits_and_not_the_run() {
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        assert_eq!(waits.trace().len(), 2);
        waits.frame(2, true, &mut ());
        assert_eq!(waits.trace().len(), 2, "the second frame's two, and not four");
        waits.frame(3, true, &mut ());
        assert_eq!(waits.trace().len(), 2);
        assert_eq!(waits.published().traced, 6, "the run's total is the sum the frames reported");
    }

    /// The chain refuses a frame ordinal that does not move, and the refusal is
    /// counted rather than swallowed.
    ///
    /// Which is not a case any caller in this component can produce —
    /// `crate::tree::Counters::frames` only rises — and is the reason the counter
    /// exists: a word nothing can move is indistinguishable from a word that does
    /// not work, and this is the test that moves it.
    #[test]
    fn a_frame_ordinal_that_does_not_move_is_refused_and_counted() {
        let mut waits = Waits::ZERO;
        waits.frame(1, true, &mut ());
        waits.frame(1, true, &mut ());
        assert_eq!(waits.published().refusals, 1);
        // And the refusal recorded nothing, which is `f_abi::sync`'s own rule: a
        // wait that was refused was never entered. The application's signal is
        // what is refused here — it is the first call — so this frame's trace is
        // empty rather than half full.
        assert_eq!(waits.trace().len(), 0);
        assert_eq!(waits.published().traced, 2, "the first frame's two, and no more");
    }
}
