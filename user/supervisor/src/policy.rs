// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What to do about a place whose occupant has ended.
//!
//! # Why this file is a move and not a rewrite
//!
//! It was `kernel::component::policy`, and RFC 0008 said from the beginning that
//! it did not belong there: *restart is the supervisor's act and the frame
//! provides only the mechanism.* The function was written for this day — it took
//! a record and a tally and no kernel state at all, precisely so that the move
//! would be a move — and `cargo xtask lint-owed` held `policy::decide(` in the
//! frame as a declared quantity so that the day it left, the build would say so.
//!
//! The body below is the body that was there. Not the order of the two tests,
//! which is load-bearing and commented where it happens; not the saturating
//! arithmetic; not [`Verdict`]'s three variants.
//!
//! # What crossing the boundary changed, which is where the tick comes from
//!
//! In the frame, `now` was the frame's own tick count, read where the frame
//! already had one. Here it is what the frame stamped on the notice that said
//! the occupant had gone. That is strictly better and it is part of why RFC 0008
//! wanted the move: a policy that reads a clock cannot be replayed, and one that
//! reads the notice it is answering can be driven by a seed. `sim/src/chaos.rs`
//! names this function as the thing it models.
//!
//! # Where the tally lives, which is the seam RFC 0076 names
//!
//! Not here. A supervisor is run when the frame has something to tell it
//! (RFC 0076), so nothing in this image survives between two runs — there are no
//! writable statics in a component and the frame clears the table on the way
//! out. The [`Budget`] therefore lives on the board, written back by the
//! supervisor and carried across runs by the frame.
//!
//! So the frame **stores** the policy's memory and the supervisor **decides**
//! with it. RFC 0008 objects to the frame deciding, and it does not; RFC 0076
//! records that this is the seam where somebody will one day argue it does. What
//! keeps the argument answerable is that the frame never reads these two numbers
//! — they are bytes it copies — and the day it interprets one is the day the
//! objection lands.
//!
//! # What `E3-B05e` added, and what it did not
//!
//! [`fate`] is new and it is the half of that line this crate can hold: a
//! supervisor that names `f_abi::control::cause::TIMEDOUT` over two numbers an
//! occupant published about itself, with nothing in `kernel/` computing it. RFC
//! 0123 is the argument for the word and for where it lands in the policy table.
//!
//! [`decide`] stopped taking two booleans on the same day, and that was not
//! tidying. The caller passed `true, false` for every death it was told about,
//! because the flags were never derived from anything — so a supervisor that
//! was told a component had *exited* would have restarted it as though it had
//! crashed, under a policy whose whole content is telling those two apart. The
//! cause has been on the wire since RFC 0008 and this is the first reader of it.
//!
//! **The delivery landed a wave later, and so did the finding it turned up.**
//! [`Liveness`] is now built on a running machine, out of
//! `crate::routing::at::LIVE`, which the frame fills by copying — RFC 0126. And
//! [`answer`] is the whole of what a run does about one row, lifted out of the
//! component so that it can be tested here: the first boot that consulted a
//! supervisor about an *occupied* place showed that the drain's cause word had
//! been zero on every death this tree had ever shown, because the frame never
//! put one on the notice — so every restart in every boot log came from the
//! branch for a place the frame built and never filled, and [`decide`] had never
//! run on a machine. A death whose notice carries no cause is now refused as R04
//! says, and the frame puts the cause there.

use f_abi::manifest::Record;

/// What the supervisor does next about a place whose occupant has ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Leave the place empty. The policy says so, or the death was a stop —
    /// which is the supervisor's own decision, and restarting after one would be
    /// the supervisor arguing with itself.
    Leave,
    /// Spawn again, after this many timer ticks.
    /// Unit of the payload: timer ticks, at the frame's own tick rate.
    Restart(u32),
    /// Retire the place: the budget ran out. Its endpoint is revoked in every
    /// holder's table and pending connects complete `PEER/GONE`.
    Retire,
}

impl Verdict {
    /// This verdict as the ordinal the board carries.
    ///
    /// A number and not a `repr` on the enum, because the board is a wire format
    /// between two crates and an enum's discriminant is a Rust detail that a
    /// `#[derive]` somewhere else could move. R04's shape: the mapping is
    /// written down once, in both directions, and an ordinal this build does not
    /// name is refused rather than defaulted.
    /// Unit: none — an ordinal.
    #[must_use]
    pub const fn to_wire(self) -> u64 {
        match self {
            Self::Leave => 0,
            Self::Restart(_) => 1,
            Self::Retire => 2,
        }
    }
}

/// How many restarts have happened inside the current budget window, and when
/// the window opened.
///
/// The window is counted in what the frame's notices are timestamped in, which
/// is RFC 0008's sentence and RFC 0004's substrate keeping a call site it would
/// otherwise have lost: under the simulator a restart storm is a seeded scenario
/// and not a wall-clock accident.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// Restarts inside the window. Unit: restarts.
    pub used: u32,
    /// When the window opened. Unit: timer ticks, at the frame's own rate, as
    /// the notice this supervisor was answering reported them.
    pub opened: u64,
}

/// Decide.
///
/// `cause` is how the occupant ended, as `f_abi::control::cause` spells it and
/// as the peer-gone notice carried it; `now` is the tick the notice carried. A
/// window that has elapsed resets the count *before* the decision, which is the
/// order that matters: a component that fails once a day forever is restarted
/// forever, and `docs/manifest.md` says at length that a lifetime cap beside the
/// window is schema 2's and needs a workload — E1-P06 — to justify it.
///
/// # Why a cause and not two flags
///
/// It was `faulted: bool, exited: bool`, and the two flags were not derived from
/// anything: the caller passed `true, false` for **every** death, with a comment
/// saying an occupant the frame tore down is a death this supervisor treats as a
/// fault. That comment was the whole of the classification. A peer-gone notice
/// has carried the cause in `Cqe::ext` since RFC 0008 and nothing read it, so a
/// supervisor could not have told a crash from a planned shutdown had one
/// arrived — which is the distinction the cause word exists for and the reason
/// `E3-B05e` could not be built on top of two booleans.
#[must_use]
pub fn decide(record: &Record, budget: &mut Budget, cause: u64, now: u64) -> Verdict {
    if !record.restarts_after_cause(cause) {
        return Verdict::Leave;
    }
    if now.saturating_sub(budget.opened) >= u64::from(record.budget_window_ticks) {
        budget.used = 0;
        budget.opened = now;
    }
    if budget.used >= record.max_restarts {
        return Verdict::Retire;
    }
    let pause = record.backoff_ticks(budget.used);
    budget.used += 1;
    Verdict::Restart(pause)
}

/// What a supervisor can see of an occupant that has **not** ended.
///
/// Two numbers the occupant published about itself, out of its own state tree —
/// RFC 0013's *read, never delivered*. They are the compositor's
/// `f_compositor::routing::node::WAITS` and `node::TIMEOUTS`, and this type
/// deliberately does not name that component: what it describes is *a
/// synchronising occupant*, and the day a second one publishes the same two
/// words this reads them without being widened.
///
/// # Why two and not one
///
/// Because either alone counts something else, which is the same argument
/// `user/compositor/src/waits.rs` makes one layer down about the two halves it
/// requires before it counts a frame abandoned at all. A component with waits
/// outstanding and nothing abandoned is a pipeline in flight, which is what a
/// working compositor looks like at every instant between a commit and a
/// present. A component that abandoned a frame and has nothing outstanding now
/// is a component that missed one and carried on, which is a *late* frame and
/// is what `E3-B07`'s degradation policy exists to absorb — restarting for one
/// would be this supervisor answering a quality problem with a cold start.
///
/// # There is no clock here and no duration
///
/// RFC 0004 gives a supervisor no more of one than it gives a compositor. So a
/// timeout is not *how long* a wait has been outstanding; it is *a wait still
/// outstanding on a component that has just given a frame up*. The occupant did
/// the timing, in the only currency it has — its own frame deadlines against its
/// own cost estimate — and this reads the answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Liveness {
    /// Waits the occupant's last closed frame entered and did not get out of.
    ///
    /// A gauge over one frame and not a total, which is what makes *is it stuck
    /// now* answerable at all. Unit: waits.
    pub outstanding_waits: u64,
    /// Frames the occupant has abandoned since it started.
    ///
    /// A total, and it only ever rises within one occupant's life. Unit: frames.
    pub abandoned_frames: u64,
}

/// Name the fate of an occupant that is still running, or answer that it has
/// none.
///
/// `seen` is the occupant's abandoned-frame count as this supervisor last read
/// it — its own memory, carried across runs the way [`Budget`] is and stored by
/// the frame the same way (RFC 0076: the frame stores the policy's memory and
/// never interprets it).
///
/// Answers a **packed** `f_abi::control::cause` word, ready to be carried on the
/// notice that tells this occupant's peers why it went: the cause in the low
/// half and, in the high half, the reading the judgement was taken on. A
/// decision published without the number behind it is a decision nobody can
/// check, which is `claims/README.md`'s rule applied to a fate.
///
/// # This is the whole of *the supervisor decides it*
///
/// `E3-B05e`'s exit ends with the clause that matters — *RFC 0008 is paid, so
/// the policy that decides is above the frame rather than inside it* — and this
/// function is where that is either true or a sentence. Nothing in `kernel/`
/// computes it and nothing in `kernel/` may: the frame's whole part is to copy
/// two words out of memory it granted and hand them over, exactly as it copies
/// [`Budget`] without reading it. The day something in the frame compares these
/// two numbers, RFC 0008's objection lands and this file is the evidence of what
/// was lost.
///
/// # Why there is no tolerance and no threshold
///
/// The obvious shape is *n abandoned frames within m ticks*, matching the
/// restart budget one field over. It is not written, and the reason is that
/// neither number exists: there is no workload in this tree that says how many
/// abandoned frames a healthy compositor produces, so an `n` chosen now would be
/// a constant with an argument instead of a measurement behind it — which is the
/// thing `claims/` refuses to publish. What *is* known is the shape of the two
/// readings, and `abandoned_frames > seen && outstanding_waits > 0` is the
/// weakest rule that uses both.
///
/// *What would reverse this:* a measurement. `E3-B05d` is the line that
/// establishes what a frame costs on a named machine; the day it says how often
/// a healthy pipeline gives one up, the tolerance becomes a declared field
/// beside `max_restarts` and this function takes it. It is deliberately not a
/// constant in this file in the meantime, because a constant is how a number
/// nobody measured becomes one everybody cites.
#[must_use]
pub fn fate(now: &Liveness, seen: u64) -> Option<u64> {
    // Strictly greater, which is also what makes a **fresh** occupant safe. A
    // restarted component publishes zeroes and this supervisor's memory of the
    // one before it does not reset in the same instant, so the first reading
    // after a refill is a smaller number against a larger `seen` — and a rule
    // written with `!=` would name a timeout for a component that has done
    // nothing at all.
    if now.abandoned_frames <= seen {
        return None;
    }
    if now.outstanding_waits == 0 {
        return None;
    }
    Some(f_abi::control::cause::pack(f_abi::control::cause::TIMEDOUT, now.abandoned_frames))
}

/// Everything this supervisor knows about one row on one run.
///
/// Gathered by the component out of its board and its drain, and handed to
/// [`answer`] whole, so that what the component does about a row is a function a
/// host test can drive rather than a branch inside a loop that only runs at
/// ring 3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The packed cause a peer-gone notice for this row carried, if one
    /// arrived — **including zero**, which is a notice that carried no cause and
    /// is not the same thing as no notice. Conflating the two is the defect RFC
    /// 0126 records: the drain stored `0` for *nothing died here* and the frame
    /// posted `0` for every death, so every death read as nothing.
    pub ended: Option<u64>,
    /// Whether the place has an occupant now, as the frame said.
    pub occupied: bool,
    /// Whether this run's assembler already submitted a spawn for the row.
    pub taken: bool,
    /// The tally the frame handed back.
    pub budget: Budget,
    /// What the occupant published about its own progress.
    pub liveness: Liveness,
    /// This supervisor's memory of the abandoned count. Unit: frames.
    pub seen: u64,
}

/// What this supervisor does about one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer {
    /// What it decided about refilling the place.
    pub verdict: Verdict,
    /// The tally as it leaves it.
    pub budget: Budget,
    /// A stop to submit against a live occupant, naming this packed cause —
    /// which is [`fate`]'s answer and nothing else.
    pub stop: Option<u64>,
    /// What to remember as `seen` next run. Unit: frames.
    pub seen: u64,
}

/// Answer one row.
///
/// Four cases, in an order that is load-bearing:
///
/// 1. **A row the assembler already took** is left: its spawn is on the ring.
/// 2. **A death** is [`decide`]'s, on the cause the notice carried. A notice
///    that carried *no* cause is decided on cause zero, which no policy
///    restarts after (R04) — so a frame that forgot to say why is a place left
///    empty and a red boot, rather than a restart that looks right.
/// 3. **A live occupant** is never respawned. Its only possible act is a stop,
///    and only when [`fate`] names one over what it published: the restart is
///    then decided on the *next* run, when the death comes back as a notice
///    carrying the word this run named. Deciding both here would be a verdict
///    taken on a word the wire never carried, which is the shape of the defect
///    in case 2.
/// 4. **An empty place with an untouched tally** is the frame handing over a
///    place it built and never filled, and is filled.
///
/// The memory moves only where there was an occupant to read: an empty place's
/// liveness is the frame's zeroes, and remembering them would forget the reading
/// a death was decided on.
#[must_use]
pub fn answer(record: &Record, facts: &Facts, now: u64) -> Answer {
    let mut budget = facts.budget;
    let seen = if facts.occupied { facts.liveness.abandoned_frames } else { facts.seen };
    let leave = |budget| Answer { verdict: Verdict::Leave, budget, stop: None, seen };
    if facts.taken {
        return leave(budget);
    }
    if let Some(packed) = facts.ended {
        let verdict = decide(record, &mut budget, f_abi::control::cause::of(packed), now);
        return Answer { verdict, budget, stop: None, seen };
    }
    if facts.occupied {
        return Answer { stop: fate(&facts.liveness, facts.seen), ..leave(budget) };
    }
    if budget == Budget::default() {
        return Answer { verdict: Verdict::Restart(0), budget, stop: None, seen };
    }
    leave(budget)
}

#[cfg(test)]
mod tests {
    use super::{Answer, Budget, Facts, Liveness, Verdict, answer, decide, fate};
    use f_abi::control::cause;
    use f_abi::manifest::{Record, restart};

    /// The manifest `user/store` declares, in the two fields this file reads.
    ///
    /// Built rather than parsed, and from `Record::EMPTY` rather than from a
    /// literal, so that a field added to the format arrives here as a zero the
    /// way it arrives everywhere else instead of as a compile error somebody
    /// fixes by copying a neighbour's value.
    fn place(max_restarts: u32, window: u32) -> Record {
        Record {
            restart: restart::ON_FAULT,
            max_restarts,
            budget_window_ticks: window,
            backoff_first_ticks: 8,
            backoff_max_ticks: 64,
            ..Record::EMPTY
        }
    }

    /// **This test was a loop in the frame.**
    ///
    /// `kernel/src/component.rs` drove `decide` until it returned something
    /// other than `Restart` and required that something to be `Retire`, inside
    /// the boot, as part of the retirement demonstration. That was a pure
    /// computation wearing a boot's clothes: no memory was spent, nothing was
    /// spawned, and the only thing under test was this function's arithmetic.
    /// It is a test here, and what stays in the boot is the *mechanism* —
    /// tearing the place down and refusing the connects.
    #[test]
    fn a_budget_spent_inside_its_window_retires_the_place() {
        let record = place(3, 3000);
        let mut budget = Budget::default();
        let mut now = 100;
        let mut restarts = 0;
        let verdict = loop {
            match decide(&record, &mut budget, cause::FAULT, now) {
                Verdict::Restart(pause) => {
                    restarts += 1;
                    // Advanced by the backoff the policy itself returned, which
                    // is what a supervisor does — and is why this converges
                    // rather than resetting the window on every round.
                    now = now.saturating_add(u64::from(pause));
                }
                other => break other,
            }
        };
        assert_eq!(verdict, Verdict::Retire);
        assert_eq!(restarts, 3, "the budget is spent exactly as many times as it declares");
    }

    /// **This test was four lines in the frame too**, and it is the one that
    /// says what a budget *is*: a count over a window, not a lifetime cap.
    /// `docs/manifest.md` says the second is schema 2's.
    #[test]
    fn the_same_count_retires_inside_the_window_and_does_not_once_it_has_elapsed() {
        let record = place(3, 3000);
        let now = 100;
        let mut window = Budget { used: record.max_restarts, opened: now };
        assert_eq!(decide(&record, &mut window, cause::FAULT, now), Verdict::Retire);
        let elapsed = now.saturating_add(u64::from(record.budget_window_ticks));
        assert!(matches!(decide(&record, &mut window, cause::FAULT, elapsed), Verdict::Restart(_)));
    }

    /// A stop is the supervisor's own decision, so restarting after one would be
    /// the supervisor arguing with itself. Two-sided: the same record restarts
    /// after a fault, so this is not a policy that refuses everything.
    #[test]
    fn a_place_is_not_refilled_after_a_death_its_policy_does_not_name() {
        let record = place(3, 3000);
        let mut budget = Budget::default();
        assert_eq!(decide(&record, &mut budget, cause::EXIT, 0), Verdict::Leave);
        assert!(matches!(decide(&record, &mut budget, cause::FAULT, 0), Verdict::Restart(_)));
    }

    /// The ordinals the board carries, in both directions, because a wire
    /// mapping tested in one direction is a mapping that can be half wrong.
    #[test]
    fn the_wire_ordinals_are_the_ones_the_frame_reads() {
        assert_eq!(Verdict::Leave.to_wire(), 0);
        assert_eq!(Verdict::Restart(0).to_wire(), 1);
        assert_eq!(Verdict::Retire.to_wire(), 2);
        // A restart's *pause* is deliberately not in the ordinal: the frame does
        // not wait, and a number that only one side used would be a field two
        // sides could disagree about for free.
        assert_eq!(Verdict::Restart(64).to_wire(), Verdict::Restart(0).to_wire());
    }

    /// The reading `user/compositor`'s serving boot leaves behind, as
    /// `E3-B05f` prints it: one wait outstanding, one frame abandoned.
    ///
    /// Written as the two numbers rather than as a call into that crate, because
    /// this supervisor must not depend on the compositor: what it reads is *a
    /// synchronising occupant's two words*, and a test that imported the
    /// compositor would be pinning this rule to one component.
    const STUCK: Liveness = Liveness { outstanding_waits: 1, abandoned_frames: 1 };

    /// A component that gave a frame up and is still waiting on it has a fate,
    /// and the fate carries the reading it was taken on.
    #[test]
    fn a_wait_outstanding_on_an_abandoned_frame_is_a_timeout() {
        let packed = fate(&STUCK, 0).expect("a fate");
        assert_eq!(cause::of(packed), cause::TIMEDOUT);
        assert_eq!(cause::detail(packed), 1, "the abandoned count the judgement was taken on");
        // And it is a cause a component may be told, which is the half that says
        // this word crossed the boundary rather than being invented here.
        assert!(cause::known(cause::of(packed)));
    }

    /// Each half alone is something else, and neither is a timeout.
    ///
    /// **The test with teeth**, and it is two rows rather than one because the
    /// two mutations it kills are different mistakes: a rule that dropped the
    /// outstanding check would restart a compositor for every late frame — which
    /// is `E3-B07`'s degradation policy answered with a cold start — and a rule
    /// that dropped the abandoned check would restart one for every frame in
    /// flight, which is every healthy compositor at almost every instant.
    ///
    /// **Each row says which half was lost**, and it says so because a mutation
    /// showed it did not: the first run of this test against a `fate` with the
    /// outstanding check deleted printed `left: Some(12884901893)` and named
    /// nothing, which sends a reader to the packing rather than to the rule.
    #[test]
    fn neither_half_alone_is_a_fate() {
        assert_eq!(
            fate(&Liveness { outstanding_waits: 0, abandoned_frames: 3 }, 0),
            None,
            "a frame given up and a pipeline that recovered is late, not stuck — the \
             outstanding-waits half of the rule is what refuses it"
        );
        assert_eq!(
            fate(&Liveness { outstanding_waits: 2, abandoned_frames: 0 }, 0),
            None,
            "a wait outstanding with nothing given up is a frame in flight, which is what \
             every healthy compositor looks like between a commit and a present — the \
             abandoned-frames half of the rule is what refuses it"
        );
        assert_eq!(fate(&Liveness::default(), 0), None, "a component that has done nothing");
    }

    /// The same reading twice is one fate and not two.
    ///
    /// Which is what `seen` is for: a supervisor consulted again about a
    /// component that has not moved since must not name a second timeout, or a
    /// stuck component would spend its whole restart budget on one stuck frame
    /// and the place would retire for a single missed deadline.
    #[test]
    fn a_reading_that_has_not_moved_is_not_a_second_fate() {
        let first = fate(&STUCK, 0).expect("a fate");
        assert_eq!(cause::detail(first), 1);
        assert_eq!(fate(&STUCK, 1), None, "the same abandoned frame, read again");
        // It moves again when the occupant gives up another frame.
        let second = fate(&Liveness { outstanding_waits: 1, abandoned_frames: 2 }, 1);
        assert_eq!(cause::of(second.expect("a fate")), cause::TIMEDOUT);
    }

    /// A refilled place's new occupant is not the old one's memory.
    ///
    /// **The case a rule written with `!=` gets wrong**, and it is reachable on
    /// the first consultation after every restart: the occupant that was
    /// restarted publishes zeroes, and this supervisor's memory of the one
    /// before it says three. A component that has closed no frame at all must
    /// not be the first thing a fresh supervisor kills.
    #[test]
    fn a_fresh_occupant_against_a_stale_memory_has_no_fate() {
        assert_eq!(fate(&Liveness { outstanding_waits: 1, abandoned_frames: 0 }, 3), None);
        assert_eq!(fate(&Liveness::default(), 3), None);
    }

    /// The whole chain, which is the sentence `E3-B05e` is written in: a reading
    /// becomes a named fate, and the fate becomes a restart out of the
    /// manifest's own policy.
    ///
    /// Three policies rather than one, because *the supervisor decides from the
    /// manifest* is only true if a different manifest decides differently — and
    /// a test that drove one policy would pass over a `fate` wired straight to a
    /// spawn.
    #[test]
    fn a_timeout_restarts_where_the_manifest_says_a_failure_does() {
        let packed = fate(&STUCK, 0).expect("a fate");
        let mut budget = Budget::default();
        let restarting = place(3, 3000);
        assert!(matches!(
            decide(&restarting, &mut budget, cause::of(packed), 100),
            Verdict::Restart(_)
        ));
        assert_eq!(budget.used, 1, "a timeout spends the same budget a fault does");

        // `never` leaves it, which is the two-sided half: this is a policy that
        // refuses something rather than one that says yes to everything.
        let mut never = Record { restart: restart::NEVER, ..place(3, 3000) };
        never.max_restarts = 0;
        never.budget_window_ticks = 0;
        never.backoff_first_ticks = 0;
        never.backoff_max_ticks = 0;
        let mut untouched = Budget::default();
        assert_eq!(decide(&never, &mut untouched, cause::of(packed), 100), Verdict::Leave);
        assert_eq!(untouched, Budget::default(), "a place left is a budget unspent");

        // And a place whose budget is already spent retires on a timeout exactly
        // as it does on a fault: the fate chooses *whether*, and the budget
        // chooses *how many*.
        let mut spent = Budget { used: 3, opened: 100 };
        assert_eq!(decide(&restarting, &mut spent, cause::of(packed), 100), Verdict::Retire);
    }

    /// **The defect RFC 0126 records, as a row.** A peer-gone notice that
    /// carried no cause is a death, and it is not a restart.
    ///
    /// Before this, the drain stored the notice's `ext` as *what ended this
    /// place* and read zero as *nothing did* — and the frame posted zero on every
    /// death, so a place whose occupant had faulted was filled by the branch for
    /// a place the frame had never filled, with the tally untouched. Every boot
    /// log said `restart 0 of 3`. The first half here is that death decided on
    /// cause zero and left; the second is the same place with the cause the
    /// notice should have carried, which restarts and spends the budget.
    #[test]
    fn a_death_whose_notice_carried_no_cause_is_not_a_restart() {
        let record = place(3, 3000);
        let silent = answer(&record, &Facts { ended: Some(0), ..Facts::default() }, 100);
        assert_eq!(
            silent.verdict,
            Verdict::Leave,
            "a notice with no cause on it was read as a place nothing died in"
        );
        assert_eq!(silent.budget, Budget::default());

        let said = answer(&record, &Facts { ended: Some(cause::FAULT), ..Facts::default() }, 100);
        assert!(matches!(said.verdict, Verdict::Restart(_)));
        assert_eq!(said.budget.used, 1, "a restart decided by `decide` spends the budget");
    }

    /// A place with a live occupant is never refilled, and the rule that fills a
    /// place the frame built and left empty must not reach it.
    ///
    /// The two rows differ in one bit — whether the frame says there is an
    /// occupant — and the tally is untouched in both, which is exactly the
    /// reading that used to mean *fill this*.
    #[test]
    fn an_untouched_tally_fills_an_empty_place_and_not_an_occupied_one() {
        let record = place(3, 3000);
        let empty = answer(&record, &Facts::default(), 0);
        assert_eq!(empty.verdict, Verdict::Restart(0));
        let occupied = answer(&record, &Facts { occupied: true, ..Facts::default() }, 0);
        assert_eq!(
            occupied,
            Answer { verdict: Verdict::Leave, budget: Budget::default(), stop: None, seen: 0 }
        );
    }

    /// A stuck occupant is stopped, naming the fate; it is not restarted on the
    /// same run, and the restart follows when the death comes back carrying the
    /// word that was named.
    ///
    /// **The two runs `E3-B05e`'s boot performs, in the order it performs them.**
    /// The second run is handed back the tally and the memory the first one left,
    /// exactly as the frame hands them back, and the reading the frame clears
    /// with the occupant.
    #[test]
    fn a_stuck_occupant_is_stopped_for_its_fate_and_restarted_when_told_of_it() {
        let record = place(3, 3000);
        let first =
            answer(&record, &Facts { occupied: true, liveness: STUCK, ..Facts::default() }, 7);
        assert_eq!(first.verdict, Verdict::Leave, "a live occupant is not respawned over");
        let named = first.stop.expect("a stuck occupant is stopped");
        assert_eq!(cause::of(named), cause::TIMEDOUT);
        assert_eq!(first.seen, 1, "the reading is remembered, so it is one fate");
        assert_eq!(first.budget, Budget::default(), "nothing is spent on a stop");

        let second = answer(
            &record,
            &Facts {
                ended: Some(named),
                budget: first.budget,
                seen: first.seen,
                ..Facts::default()
            },
            7,
        );
        assert!(matches!(second.verdict, Verdict::Restart(_)));
        assert_eq!(second.budget.used, 1, "the timeout spent the budget a fault would");
        assert_eq!(second.stop, None);
        assert_eq!(second.seen, 1, "an empty place's zeroes do not overwrite the memory");
    }

    /// The same stuck reading on a second run is not a second stop, and a
    /// reading that is late rather than stuck is never one.
    #[test]
    fn a_reading_already_seen_or_merely_late_asks_for_no_stop() {
        let record = place(3, 3000);
        let again = answer(
            &record,
            &Facts { occupied: true, liveness: STUCK, seen: 1, ..Facts::default() },
            0,
        );
        assert_eq!(again.stop, None, "one abandoned frame is one fate");
        let late = Liveness { outstanding_waits: 0, abandoned_frames: 1 };
        let late =
            answer(&record, &Facts { occupied: true, liveness: late, ..Facts::default() }, 0);
        assert_eq!(late.stop, None, "a frame given up and a pipeline that recovered");
        assert_eq!(late.verdict, Verdict::Leave);
    }
}
