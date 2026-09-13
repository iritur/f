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
/// `faulted` and `exited` are how the occupant ended; `now` is the tick the
/// notice carried. A window that has elapsed resets the count *before* the
/// decision, which is the order that matters: a component that fails once a day
/// forever is restarted forever, and `docs/manifest.md` says at length that a
/// lifetime cap beside the window is schema 2's and needs a workload — E1-P06 —
/// to justify it.
#[must_use]
pub fn decide(
    record: &Record,
    budget: &mut Budget,
    faulted: bool,
    exited: bool,
    now: u64,
) -> Verdict {
    if !record.restarts_after(faulted, exited) {
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

#[cfg(test)]
mod tests {
    use super::{Budget, Verdict, decide};
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
            match decide(&record, &mut budget, true, false, now) {
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
        assert_eq!(decide(&record, &mut window, true, false, now), Verdict::Retire);
        let elapsed = now.saturating_add(u64::from(record.budget_window_ticks));
        assert!(matches!(decide(&record, &mut window, true, false, elapsed), Verdict::Restart(_)));
    }

    /// A stop is the supervisor's own decision, so restarting after one would be
    /// the supervisor arguing with itself. Two-sided: the same record restarts
    /// after a fault, so this is not a policy that refuses everything.
    #[test]
    fn a_place_is_not_refilled_after_a_death_its_policy_does_not_name() {
        let record = place(3, 3000);
        let mut budget = Budget::default();
        assert_eq!(decide(&record, &mut budget, false, true, 0), Verdict::Leave);
        assert!(matches!(decide(&record, &mut budget, true, false, 0), Verdict::Restart(_)));
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
}
