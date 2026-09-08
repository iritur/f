// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A generation swap: two phases, one machine word between them, and the
//! abandonment that is not a restart.
//!
//! [`crate::transfer`] is the *declaration* — what a component says about being
//! updated in place, decided from two manifests before a client's submissions
//! are held. This module is what happens after that decision says yes. RFC 0063
//! is the reasoning and RFC 0012 is why an update is a swap at all.
//!
//! # Why the rules are here and the storage is not
//!
//! The same split [`crate::control`] already makes about a notice: *here is the
//! state machine, `kernel/src/cap.rs` is the storage*. A swap has exactly two
//! implementors that must agree to the letter — the frame, which owns the place,
//! and the simulator, which drives one under sustained load — and a rule written
//! twice is a rule two builds can disagree about at the moment a client's work
//! is held. So [`Swap`] takes no memory, allocates nothing, and reads no state
//! record; it is the order the steps happen in and the set of ways they may
//! stop.
//!
//! It is not a wire type and it is deliberately not `repr(C)`. Nothing here
//! crosses a trust boundary — the [`Declaration`]s it compares already did, as
//! bytes inside [`crate::manifest::Record`] — so making it a layout would be
//! claiming a compatibility obligation this type does not have.
//!
//! # The two phases, and the boundary between them
//!
//! **Phase A is reversible, in its entirety.** The place stops delivering and
//! holds pends; the cursors are checked; the occupant asserts that it holds
//! nothing; it writes its state records into a window the *incoming* instance
//! bought out of its own account; the incoming instance acknowledges them. The
//! outgoing occupant is alive and still holds all its state for every one of
//! those steps, so any failure among them **abandons** the swap: the place
//! resumes delivering to the occupant it already had, and the client observes
//! added latency and nothing else.
//!
//! **Phase B is one machine word.** [`Routing::commit`] is a single `Release`
//! store, read by a single `Acquire` load, and a machine word carries either the
//! new value or the old one — so there is no halfway. After it the outgoing
//! occupant is retired and a failure of the incoming one is an ordinary fault
//! taking the supervisor's existing restart path.
//!
//! # Why an abandonment is counted under its own name
//!
//! Because the restart path answers a different question. It answers *the
//! occupant died*: it spends one of `max_restarts`, waits a backoff, and retires
//! the place when the budget runs out. An abandonment answers *the occupant is
//! alive and the swap did not happen*, and the occupant it would penalise is the
//! one that behaved correctly — so routing it through the restart path lets
//! `max_restarts` failed updates retire a healthy component's place, with the
//! manifest's own budget as the mechanism. [`Tally`] therefore has three
//! counters and never two.
//!
//! *Reversal:* an abandonment rate high enough to be a policy question. This
//! module says an abandonment costs a client latency and spends no budget, and
//! both are true per abandonment and neither is true of an unbounded number of
//! them. The repair is a budget on abandonment with its own count and its own
//! window, and it is an RFC, because *an abandonment is not a restart* is the
//! sentence it weakens. RFC 0063.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::transfer::Declaration;

/// The generation the routing word carries while a swap is in phase A.
///
/// Zero, and it is RFC 0012's own value rather than a sentinel chosen here: that
/// RFC publishes the generation counter as zero for the duration of a swap, so a
/// reader that saw zero learns that no root described the machine for an
/// interval — and that the root describing it afterwards is either the one that
/// did before (an abandonment) or the incoming one (a commit). A generation
/// ordinal counts from one for exactly this reason.
/// Unit: none — a generation ordinal, and never a live one.
pub const PAUSED: u32 = 0;

/// Where a delivery to a place goes, as one word two cores read.
///
/// # The fifth cross-core word, and the argument RFC 0016 asks for
///
/// `kernel/src/percpu.rs` makes every mutable static in the frame a `PerCpu<T>`,
/// and RFC 0016 names four exceptions — the mailbox and shootdown words in
/// `kernel/src/smp.rs` — each a machine word, each an atomic with its ordering
/// named at the access, and says a fifth needs an argument. This is the fifth
/// and the argument is that a place is not per-CPU and cannot become one: a
/// client on any core may submit to it, and *which occupant a submission is
/// delivered to* is one fact about one place rather than one fact per core. A
/// per-CPU routing word would be as many answers to that question as there are
/// cores, published at different instants, which is the state a swap exists to
/// make unreachable.
///
/// It is one word for the same reason the other four are: a word either carries
/// the new value or the old one. Phase B has no halfway state to be interrupted
/// in, and that is what lets phase A be abandoned rather than repaired.
///
/// # The ordering, and what `Relaxed` costs
///
/// [`Routing::commit`] is `Release` and [`Routing::delivering`] is `Acquire`,
/// and the pair is load-bearing rather than decorative. Everything the incoming
/// occupant built out of the transfer window — its registration table above all
/// — is written *before* the commit. A reader that saw the new generation
/// through a `Relaxed` load could deliver an entry to an instance whose table
/// has not been published to it, and the client would be answered `NO_SUCH_CAP`
/// for a buffer set it holds. That is a dropped operation produced by an
/// ordering, and it is invisible on x86-64 because total store order hides it.
///
/// `ring/tests/litmus.rs` is where that is a test rather than a paragraph, and
/// `mutate-relaxed-routing` in `abi/Cargo.toml` is the defect it is shown to
/// catch — because a stress test that has only ever passed cannot be told apart
/// from one that cannot fail.
#[derive(Debug)]
pub struct Routing {
    word: AtomicU32,
}

impl Routing {
    /// A place delivering to generation `generation`.
    ///
    /// `generation` is an ordinal counting from one; [`PAUSED`] is zero and is
    /// not a generation, so a routing word built at zero is a place that is
    /// holding pends before its first occupant has ever gone in. That is a
    /// legitimate state and it is the one a place is created in.
    #[must_use]
    pub const fn new(generation: u32) -> Self {
        Self { word: AtomicU32::new(generation) }
    }

    /// Which generation a submission arriving now is delivered to, or
    /// [`PAUSED`].
    ///
    /// `Acquire`, paired with [`Routing::commit`]'s `Release`: everything the
    /// occupant of the generation this answers built is visible to the caller
    /// once this has answered it.
    #[must_use]
    pub fn delivering(&self) -> u32 {
        self.word.load(Ordering::Acquire)
    }

    /// Whether a submission arriving now is delivered at all.
    #[must_use]
    pub fn open(&self) -> bool {
        self.delivering() != PAUSED
    }

    /// Stop delivering. Phase A begins.
    ///
    /// `Release` rather than `Relaxed` for the half of the pair that is easy to
    /// miss: everything the *outgoing* occupant did before the pause has to be
    /// visible to whoever observes the pause, or a swap could be decided against
    /// a view of the occupant that is older than the decision.
    pub fn pause(&self) {
        self.word.store(PAUSED, Ordering::Release);
    }

    /// Deliver to `generation` from now on. Phase B, and the whole of it.
    ///
    /// The one `Release` store this module rests on. Called after the incoming
    /// occupant has acknowledged the window and before the outgoing one is
    /// retired, and never in any other order — [`Swap::commit`] is what enforces
    /// that, so this is not an operation a caller reaches for on its own.
    pub fn commit(&self, generation: u32) {
        #[cfg(not(feature = "mutate-relaxed-routing"))]
        self.word.store(generation, Ordering::Release);
        // The deliberate defect. It is faster and it passes every functional
        // test on x86-64, which is the whole reason the litmus test exists.
        #[cfg(feature = "mutate-relaxed-routing")]
        self.word.store(generation, Ordering::Relaxed);
    }
}

/// Why a swap stopped in phase A.
///
/// A closed set, and it is closed on purpose: every value here is a state the
/// place can be *left in* correctly — the outgoing occupant alive, its state
/// untouched, the pends about to drain to it — and a sixth reason that did not
/// have that property would be a fault rather than an abandonment. R04 says an
/// outcome this build does not name is refused rather than guessed at, and the
/// refusal here is that there is no way to construct one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Abandoned {
    /// The two declarations do not permit a transfer at all: a mode, a
    /// state-record schema or a record width that disagree.
    ///
    /// Decided before anything drains, which is [`Declaration::transfers_to`]'s
    /// entire reason for existing — so a swap that reaches any other value here
    /// has already stopped a client, and one that reaches this one has not.
    Incompatible,
    /// The rings were not empty: a submission unread or a completion undrained.
    ///
    /// The precondition the frame checks without asking anybody. RFC 0018's
    /// cursors are necessary and not sufficient, which is why there is a second
    /// value below it.
    RingsNotEmpty,
    /// The occupant did not assert that it holds nothing.
    ///
    /// The half the cursors cannot see: a request taken off the ring and handed
    /// to a device is behind both cursors and behind the driver. RFC 0041 names
    /// the set — work accepted and not answered, keyed by the token — and this
    /// is the occupant saying it is empty.
    NotQuiescent,
    /// The outgoing occupant wrote more records than the window holds.
    ///
    /// The one incompatibility [`Declaration::transfers_to`] cannot see, because
    /// it is arithmetic over a count the outgoing occupant has not reported yet:
    /// the incoming instance bought a window for its own `records_max` and the
    /// outgoing one writes at most its own. RFC 0063 says this is an abandonment
    /// and not a loss, and the reason is that nothing has moved yet.
    WindowTooSmall,
    /// The incoming instance refused the records or faulted reading them.
    ///
    /// Including the record whose check word does not match its content: a
    /// transfer that did not cross intact is refused by the *reader* and never
    /// half-applied, because the alternative is an occupant serving a client out
    /// of state it could not verify.
    Refused,
}

impl Abandoned {
    /// The reason as a stable label, for a log and for a trace column.
    /// Unit: none.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Incompatible => "unfit",
            Self::RingsNotEmpty => "rings",
            Self::NotQuiescent => "busy",
            Self::WindowTooSmall => "window",
            Self::Refused => "refused",
        }
    }
}

/// How far a swap has got.
///
/// Ordered, and the order is the protocol: a step may only be taken from the
/// phase before it, and [`Swap`] refuses out of order rather than tolerating it.
/// A protocol that accepted its steps in any order would be a protocol whose
/// implementors can each have understood it differently and still pass.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Phase {
    /// Both declarations agree. Nothing has been held yet.
    Planned,
    /// The place has stopped delivering and the cursors have been checked.
    Drained,
    /// The occupant has asserted that it holds nothing.
    Quiescent,
    /// The records are in the window.
    Written,
    /// The incoming instance has read them and accepted them.
    Acknowledged,
    /// The routing word carries the incoming generation. Phase B is over.
    Committed,
}

/// One swap at one place, as the order its steps happen in.
///
/// It holds no memory and no occupant: what it owns is the *sequence*, which is
/// the part two implementors would otherwise each invent. The window, the
/// records and the instances are the caller's, because the frame allocates
/// nothing here and the simulator's actors are not the frame's.
#[derive(Clone, Copy, Debug)]
pub struct Swap {
    outgoing: Declaration,
    incoming: Declaration,
    phase: Phase,
    records: u32,
}

impl Swap {
    /// Decide whether this swap may happen, from two manifests and nothing else.
    ///
    /// The expensive decision taken while every client is still being served —
    /// which is [`crate::transfer`]'s whole proposition. A place whose
    /// declarations disagree is a place that will *restart*, planned rather than
    /// discovered halfway with a client's submissions already held.
    ///
    /// # Errors
    ///
    /// [`Abandoned::Incompatible`], and it is the only value reachable here.
    pub const fn plan(outgoing: &Declaration, incoming: &Declaration) -> Result<Self, Abandoned> {
        if !outgoing.transfers_to(incoming) {
            return Err(Abandoned::Incompatible);
        }
        Ok(Self { outgoing: *outgoing, incoming: *incoming, phase: Phase::Planned, records: 0 })
    }

    /// How far this swap has got. Unit: none.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// How many records crossed, once they have. Unit: records.
    #[must_use]
    pub const fn records(&self) -> u32 {
        self.records
    }

    /// How large a window the incoming instance has to buy, in bytes.
    ///
    /// The **incoming** declaration's product and never the outgoing one's: the
    /// account that pays is the account that survives, which is why an
    /// abandonment has no cleanup path of its own — it is the revocation RFC
    /// 0008 already describes. `cargo xtask lint-manifests` checks the same
    /// product against the component's own `memory_bytes` on one file.
    /// Unit: bytes.
    #[must_use]
    pub const fn window_bytes(&self) -> u64 {
        self.incoming.window_bytes()
    }

    /// The rings are empty: producer stopped, consumer caught up, on both.
    ///
    /// The precondition the frame checks without asking anybody. Necessary and
    /// not sufficient — see [`Swap::asserted`].
    ///
    /// # Errors
    ///
    /// [`Abandoned::RingsNotEmpty`] when they are not, and
    /// [`Abandoned::Refused`] when this is called out of order.
    pub const fn drained(&mut self, empty: bool) -> Result<(), Abandoned> {
        if !matches!(self.phase, Phase::Planned) {
            return Err(Abandoned::Refused);
        }
        if !empty {
            return Err(Abandoned::RingsNotEmpty);
        }
        self.phase = Phase::Drained;
        Ok(())
    }

    /// The occupant asserts that it holds nothing it has not answered.
    ///
    /// The half no cursor can supply, and the reason there is no cursors-only
    /// mode. It is asked *after* the rings are empty and never instead of it:
    /// the frame believes the assertion only once its own check agrees, so a
    /// component that answered `true` unconditionally still cannot be swapped
    /// while its client has work on the wire.
    ///
    /// # Errors
    ///
    /// [`Abandoned::NotQuiescent`], or [`Abandoned::Refused`] out of order.
    pub const fn asserted(&mut self, holds_nothing: bool) -> Result<(), Abandoned> {
        if !matches!(self.phase, Phase::Drained) {
            return Err(Abandoned::Refused);
        }
        if !holds_nothing {
            return Err(Abandoned::NotQuiescent);
        }
        self.phase = Phase::Quiescent;
        Ok(())
    }

    /// The outgoing occupant has written `records` records into the window.
    ///
    /// Bounded by the **incoming** instance's `records_max`, because that is the
    /// window that was bought. A swap into an occupant that will hold fewer
    /// records than the outgoing one wrote is the one incompatibility two
    /// manifests cannot see, and it stops here with everything still reversible.
    ///
    /// # Errors
    ///
    /// [`Abandoned::WindowTooSmall`], or [`Abandoned::Refused`] out of order.
    pub const fn wrote(&mut self, records: u32) -> Result<(), Abandoned> {
        if !matches!(self.phase, Phase::Quiescent) {
            return Err(Abandoned::Refused);
        }
        if records > self.incoming.records_max || records > self.outgoing.records_max {
            return Err(Abandoned::WindowTooSmall);
        }
        self.records = records;
        self.phase = Phase::Written;
        Ok(())
    }

    /// The incoming instance has read the records and accepted them.
    ///
    /// `accepted` is the incoming occupant's own answer and not the frame's: the
    /// frame reads no record, so whether the bytes meant anything is a question
    /// only the other build of that component can answer. A refusal here is
    /// still an abandonment — the outgoing occupant is alive and untouched, and
    /// the place resumes delivering to it.
    ///
    /// # Errors
    ///
    /// [`Abandoned::Refused`], which is also what an out-of-order call earns:
    /// the two are the same outcome for the place and separating them would be a
    /// distinction with no consequence.
    pub const fn acknowledged(&mut self, accepted: bool) -> Result<(), Abandoned> {
        if !matches!(self.phase, Phase::Written) || !accepted {
            return Err(Abandoned::Refused);
        }
        self.phase = Phase::Acknowledged;
        Ok(())
    }

    /// Phase B: store the routing word, and the swap is over.
    ///
    /// The only caller of [`Routing::commit`] that a place should have. It
    /// refuses to store the word from any phase but [`Phase::Acknowledged`],
    /// which is the whole of what makes the boundary between the two phases a
    /// property of this type rather than of whoever wrote the loop.
    ///
    /// # Errors
    ///
    /// [`Abandoned::Refused`] when the swap has not been acknowledged. Nothing
    /// is stored in that case, so the place is still delivering to whichever
    /// generation it was.
    pub fn commit(&mut self, routing: &Routing, generation: u32) -> Result<(), Abandoned> {
        if !matches!(self.phase, Phase::Acknowledged) {
            return Err(Abandoned::Refused);
        }
        routing.commit(generation);
        self.phase = Phase::Committed;
        Ok(())
    }
}

/// What a generation's swaps cost, counted apart.
///
/// Three counters and never two, and never one sum. RFC 0012 already separates
/// the first two — *a swap in which every place declared `restart_only` is a
/// reboot in instalments, and the artefact has to show that rather than report a
/// swap* — and RFC 0063 adds the third for the reason [`Abandoned`] exists: an
/// update that never lands and a component that keeps dying are two different
/// findings with two different first debugging steps.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tally {
    /// Places whose occupant handed its state to a newer one in place.
    /// Unit: places.
    pub places_swapped: u32,
    /// Places that were torn down and refilled from the incoming manifest
    /// because one side declared [`crate::transfer::mode::RESTART_ONLY`].
    /// Deliberately not summed with the row above it. Unit: places.
    pub places_restarted: u32,
    /// Swaps that stopped in phase A with the outgoing occupant still serving.
    ///
    /// **Not a failure on its own, and not a restart.** It spends no restart
    /// budget, waits no backoff and costs the client latency; what would make it
    /// a policy question is a *rate*, which is this counter's reason for
    /// existing. Unit: swaps.
    pub swaps_abandoned: u32,
}

impl Tally {
    /// Places this generation changed the occupant of, however it changed it.
    ///
    /// A sum, offered once here rather than computed at three call sites — and
    /// it is a sum of the two that *happened* and never of all three, because an
    /// abandonment left the place exactly as it was. Unit: places.
    #[must_use]
    pub const fn places_refilled(&self) -> u32 {
        self.places_swapped.saturating_add(self.places_restarted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::mode;

    /// A declaration the block driver's manifest actually carries.
    const BLK: Declaration = Declaration {
        schema: 1,
        record_bytes: 32,
        records_max: 16,
        mode: mode::IN_PLACE,
        _reserved: [0; 3],
    };

    fn planned() -> Swap {
        Swap::plan(&BLK, &BLK).expect("two identical in-place declarations")
    }

    fn through_phase_a(swap: &mut Swap, records: u32) -> Result<(), Abandoned> {
        swap.drained(true)?;
        swap.asserted(true)?;
        swap.wrote(records)?;
        swap.acknowledged(true)
    }

    #[test]
    fn a_restart_only_side_is_refused_before_anything_is_held() {
        assert_eq!(
            Swap::plan(&BLK, &Declaration::RESTART_ONLY).unwrap_err(),
            Abandoned::Incompatible
        );
        assert_eq!(
            Swap::plan(&Declaration::RESTART_ONLY, &BLK).unwrap_err(),
            Abandoned::Incompatible
        );
    }

    #[test]
    fn the_routing_word_pauses_and_commits() {
        let routing = Routing::new(1);
        assert_eq!(routing.delivering(), 1);
        assert!(routing.open());
        routing.pause();
        assert_eq!(routing.delivering(), PAUSED);
        assert!(!routing.open(), "a paused place delivers to nobody and holds its pends");

        let mut swap = planned();
        through_phase_a(&mut swap, 3).expect("phase A");
        swap.commit(&routing, 2).expect("phase B");
        assert_eq!(routing.delivering(), 2);
        assert_eq!(swap.phase(), Phase::Committed);
        assert_eq!(swap.records(), 3);
    }

    /// Every step refuses to be taken out of order, and the word is not stored.
    #[test]
    fn phase_b_cannot_be_reached_without_phase_a() {
        let routing = Routing::new(1);
        let mut swap = planned();
        assert_eq!(swap.commit(&routing, 2).unwrap_err(), Abandoned::Refused);
        assert_eq!(
            routing.delivering(),
            1,
            "a refused commit must not have stored the routing word: the place is still \
             delivering to the occupant it had"
        );
        assert_eq!(swap.asserted(true).unwrap_err(), Abandoned::Refused);
        assert_eq!(swap.wrote(1).unwrap_err(), Abandoned::Refused);
        assert_eq!(swap.acknowledged(true).unwrap_err(), Abandoned::Refused);
    }

    /// Each of the four phase-A refusals, reached on purpose.
    #[test]
    fn every_abandonment_is_reachable_and_leaves_the_word_alone() {
        let routing = Routing::new(7);

        let mut rings = planned();
        assert_eq!(rings.drained(false).unwrap_err(), Abandoned::RingsNotEmpty);

        let mut busy = planned();
        busy.drained(true).expect("empty rings");
        assert_eq!(busy.asserted(false).unwrap_err(), Abandoned::NotQuiescent);

        let mut wide = planned();
        wide.drained(true).expect("empty rings");
        wide.asserted(true).expect("quiescent");
        assert_eq!(wide.wrote(17).unwrap_err(), Abandoned::WindowTooSmall);

        let mut refused = planned();
        refused.drained(true).expect("empty rings");
        refused.asserted(true).expect("quiescent");
        refused.wrote(16).expect("the declared bound exactly");
        assert_eq!(refused.acknowledged(false).unwrap_err(), Abandoned::Refused);

        assert_eq!(
            routing.delivering(),
            7,
            "no abandonment reaches the routing word — that is what makes phase A reversible"
        );
    }

    /// The window is the incoming instance's product and not the outgoing one's.
    #[test]
    fn the_window_is_bought_by_the_account_that_survives() {
        let incoming = Declaration { records_max: 4, ..BLK };
        let swap = Swap::plan(&BLK, &incoming).expect("records_max is not compared");
        assert_eq!(swap.window_bytes(), 128, "four records of thirty-two bytes");
        assert_eq!(BLK.window_bytes(), 512, "and the outgoing side's own bound is larger");
    }

    /// The one incompatibility two manifests cannot see, stopped where nothing
    /// has moved yet.
    #[test]
    fn a_smaller_incoming_bound_abandons_rather_than_overruns() {
        let incoming = Declaration { records_max: 4, ..BLK };
        let mut swap = Swap::plan(&BLK, &incoming).expect("compatible on the three fields");
        assert_eq!(through_phase_a(&mut swap, 5).unwrap_err(), Abandoned::WindowTooSmall);
        assert_eq!(swap.records(), 0, "nothing crossed, so nothing is counted as having");
    }

    #[test]
    fn the_three_counters_are_never_one_number() {
        let tally = Tally { places_swapped: 2, places_restarted: 1, swaps_abandoned: 4 };
        assert_eq!(tally.places_refilled(), 3, "an abandonment refilled nothing");
    }

    #[test]
    fn every_abandonment_has_a_label_and_no_two_share_one() {
        let all = [
            Abandoned::Incompatible,
            Abandoned::RingsNotEmpty,
            Abandoned::NotQuiescent,
            Abandoned::WindowTooSmall,
            Abandoned::Refused,
        ];
        for (at, one) in all.iter().enumerate() {
            assert!(!one.label().is_empty());
            assert!(one.label().len() <= 8, "a trace column is not a sentence");
            for other in &all[at + 1..] {
                assert_ne!(one.label(), other.label(), "two reasons with one name");
            }
        }
    }
}
