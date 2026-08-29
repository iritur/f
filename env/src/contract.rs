// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What every [`Env`] has to be true of, checked at runtime.
//!
//! # Why this is a function and not a test module
//!
//! There are two implementations of the trait and they cannot be tested the
//! same way. [`crate::SeededEnv`] is tested on the host, by asserting that a
//! seed reproduces its run byte for byte. The hardware one cannot be: being
//! nondeterministic is its whole job, it exists only inside the kernel, and the
//! kernel has no host test harness at all — `kernel/Cargo.toml` says why.
//!
//! So the properties both must have are written once, as a function that runs
//! anywhere. The tests at the bottom of this file call it with a seeded
//! environment; the kernel calls it at boot with the hardware one. A property
//! checked against only one implementation is a property the other one gets to
//! violate — and the one that gets to violate it is the one nobody can
//! reproduce.
//!
//! # What is checked, and what is deliberately not
//!
//! The clock does not go backwards, does not stop, and reports differences that
//! agree with its own ordering. The generator is not stuck. The ordering policy
//! answers inside the range it was given. Wall time, if the machine claims any,
//! does not change its mind about where the reading came from.
//!
//! Nothing here checks that wall time is not used to order events, because no
//! runtime check can see that. It is enforced by the type — [`crate::WallTime`]
//! implements neither `Ord` nor `PartialOrd` — and by review. RFC 0009.
//!
//! Nothing here checks the *quality* of the randomness either. Sixty-four draws
//! cannot tell a good stream from a poor one, and a statistical test against an
//! environment that is allowed to be adversarial is a test that fails at
//! random. The strong statement about randomness is the one the seeded
//! environment can make and does: the same seed produces the same stream.

use crate::Env;

/// How many times the clock is observed before it is called stopped.
///
/// Generous on purpose. A virtual clock advances by being used and a hardware
/// clock advances on its own, but a hardware clock read under emulation can
/// return the same nanosecond several times running — that is quantisation, not
/// a stopped clock, and the difference between the two is how long you are
/// prepared to wait. Bounded rather than patient forever, because the caller is
/// a boot sequence and a boot that hangs has no output.
const OBSERVATIONS: u32 = 1024;

/// How many values are drawn while looking for a stuck generator.
const DRAWS: u32 = 64;

/// A property an [`Env`] failed.
///
/// Distinct variants rather than one string, so the tests below can assert
/// *which* property a deliberately broken environment violated. A checker
/// nobody has watched fail is a checker nobody has tested.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Violation {
    /// Two consecutive observations of `now()` decreased.
    ClockWentBackwards,
    /// `now()` never advanced, however long it was watched.
    ClockStopped,
    /// [`crate::Instant::saturating_since`] disagreed with the order of the two
    /// instants it was given.
    DifferenceDisagreesWithOrder,
    /// Every value the generator produced was the same one.
    RandomnessIsStuck,
    /// `choose(n)` returned an index that is not below `n`.
    ChoiceOutOfRange,
    /// `choose(0)` or `choose(1)` returned something other than zero.
    DegenerateChoiceIsNotZero,
    /// Two readings of the wall clock disagreed about where they came from.
    WallProvenanceMoved,
}

impl Violation {
    /// A sentence for a log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::ClockWentBackwards => "the monotonic clock went backwards",
            Self::ClockStopped => "the monotonic clock never advanced",
            Self::DifferenceDisagreesWithOrder => {
                "saturating_since disagrees with the order of the instants it was given"
            }
            Self::RandomnessIsStuck => "the generator returned one value forever",
            Self::ChoiceOutOfRange => "the scheduler chose an index outside the range it was given",
            Self::DegenerateChoiceIsNotZero => "the scheduler chose non-zero from no alternatives",
            Self::WallProvenanceMoved => "two wall-clock readings claimed different sources",
        }
    }
}

/// Check an environment against the contract every implementation owes.
///
/// Takes `&mut dyn Env` rather than a generic parameter: the kernel calls this
/// once, on one environment, and a generic would put a second copy of the whole
/// check into a binary that has reason to care about its own size.
///
/// # Errors
///
/// The first [`Violation`] found. There is no value in collecting the rest — an
/// environment that fails any of these is not usable, and the first failure is
/// the one to debug.
pub fn check(env: &mut dyn Env) -> Result<(), Violation> {
    clock(env)?;
    randomness(env)?;
    ordering(env)?;
    wall(env)
}

/// Monotonicity, progress, and differences that agree with the order.
fn clock(env: &mut dyn Env) -> Result<(), Violation> {
    let start = env.now();
    let mut last = start;
    let mut advanced = false;

    for _ in 0..OBSERVATIONS {
        // A draw, deliberately, so that the loop is a workload for both kinds
        // of clock: the virtual one advances because the environment was used,
        // the hardware one because time passed.
        let _ = env.next_u64();
        let now = env.now();

        if now < last {
            return Err(Violation::ClockWentBackwards);
        }
        if now.saturating_since(last) != now.as_nanos() - last.as_nanos() {
            return Err(Violation::DifferenceDisagreesWithOrder);
        }
        // The saturating half of the name: an earlier instant is not a negative
        // duration, it is zero. A clock that answers otherwise has a subtraction
        // that wraps, and a wrapped duration is an enormous one.
        if last.saturating_since(now) != 0 {
            return Err(Violation::DifferenceDisagreesWithOrder);
        }
        if now > start {
            advanced = true;
        }
        last = now;
    }

    if last.saturating_since(last) != 0 {
        return Err(Violation::DifferenceDisagreesWithOrder);
    }
    if !advanced {
        return Err(Violation::ClockStopped);
    }
    Ok(())
}

/// The weakest honest statement about a generator: it is not a constant.
fn randomness(env: &mut dyn Env) -> Result<(), Violation> {
    let first = env.next_u64();
    for _ in 1..DRAWS {
        if env.next_u64() != first {
            return Ok(());
        }
    }
    Err(Violation::RandomnessIsStuck)
}

/// The ordering policy answers inside the range it was handed.
///
/// The degenerate cases are checked first and separately. `choose(0)` asks which
/// of no alternatives to take, which has no correct answer and one defensible
/// one — and a caller that has to special-case a policy's reply to an empty set
/// will eventually forget to.
fn ordering(env: &mut dyn Env) -> Result<(), Violation> {
    let sched = env.scheduler();

    if sched.choose(0) != 0 || sched.choose(1) != 0 {
        return Err(Violation::DegenerateChoiceIsNotZero);
    }

    // Powers of two, two that are not, and the largest value the parameter can
    // hold — which is where an implementation that computes its modulus in a
    // narrower type gives itself away.
    for n in [2u32, 3, 8, 64, 1000, u32::MAX] {
        for _ in 0..DRAWS {
            if sched.choose(n) >= n {
                return Err(Violation::ChoiceOutOfRange);
            }
        }
    }

    // Free in production, a decision point under simulation, and a call that
    // has to be sound in both.
    sched.yield_point();
    Ok(())
}

/// Wall time, if this environment claims any, is claimed consistently.
///
/// Two readings, microseconds apart. What they may not do is disagree about
/// *where the reading came from*: a source that changes between two adjacent
/// calls is an environment that does not know what it is reading, and provenance
/// is the whole reason [`crate::WallTime`] carries a source at all.
///
/// A machine that acquires a trustworthy clock — a network discipline that
/// completes, a peer that answers — goes from `None` to `Some` at that moment,
/// and that is legitimate rather than a violation. So the *absence* of a clock
/// is not compared; only the provenance of two readings that both exist.
fn wall(env: &dyn Env) -> Result<(), Violation> {
    let (Some(first), Some(second)) = (env.wall(), env.wall()) else {
        return Ok(());
    };
    if first.source != second.source || first.uncertainty_nanos != second.uncertainty_nanos {
        return Err(Violation::WallProvenanceMoved);
    }
    // Nothing is asserted about the value. It is a datum, and the point of
    // RFC 0009 is that this system does not order anything by it — here
    // included.
    Ok(())
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;
    use crate::sim::SimEnv;
    use crate::{Instant, Scheduler, SeededEnv, WallSource, WallTime};

    #[test]
    fn the_seeded_environment_satisfies_the_contract() {
        for seed in [0, 1, 42, u64::MAX] {
            let mut env = SeededEnv::new(seed, 10);
            assert_eq!(check(&mut env), Ok(()), "seed {seed} failed the contract");
        }
    }

    #[test]
    fn the_simulator_satisfies_the_contract_while_injecting() {
        // Injection on, because a simulator that satisfies the contract only
        // while it is not doing its job satisfies nothing.
        let mut env = SimEnv::new(1234, 10, 200);
        assert_eq!(check(&mut env), Ok(()));
    }

    /// Which property this environment breaks. Everything it does not break it
    /// does correctly, so that a test cannot pass for the wrong reason.
    #[derive(Clone, Copy)]
    enum Break {
        Stopped,
        Backwards,
        StuckRandom,
        OutOfRange,
        DegenerateChoice,
        FickleWall,
    }

    struct Faulty {
        how: Break,
        clock: Cell<u64>,
        state: u64,
        firmware: Cell<bool>,
    }

    impl Faulty {
        fn new(how: Break) -> Self {
            Self {
                how,
                clock: Cell::new(1_000_000),
                state: 0x2545_F491_4F6C_DD1D,
                firmware: Cell::new(true),
            }
        }
    }

    impl Scheduler for Faulty {
        fn choose(&mut self, n: u32) -> u32 {
            match self.how {
                Break::DegenerateChoice => 1,
                // In range for the degenerate cases, so the out-of-range
                // failure is reported for the reason the test names.
                Break::OutOfRange if n > 1 => n,
                _ => 0,
            }
        }
    }

    impl Env for Faulty {
        fn now(&self) -> Instant {
            let at = self.clock.get();
            match self.how {
                Break::Stopped => Instant(at),
                Break::Backwards => {
                    self.clock.set(at - 1);
                    Instant(at)
                }
                _ => {
                    self.clock.set(at + 10);
                    Instant(at)
                }
            }
        }

        fn next_u64(&mut self) -> u64 {
            match self.how {
                Break::StuckRandom => 7,
                _ => {
                    self.state ^= self.state << 13;
                    self.state ^= self.state >> 7;
                    self.state ^= self.state << 17;
                    self.state
                }
            }
        }

        fn wall(&self) -> Option<WallTime> {
            let firmware = self.firmware.get();
            match self.how {
                Break::FickleWall => {
                    self.firmware.set(!firmware);
                    Some(WallTime {
                        tai_nanos: 1_788_006_896_000_000_000,
                        uncertainty_nanos: 1_000_000_000,
                        source: if firmware { WallSource::Firmware } else { WallSource::Network },
                    })
                }
                _ => None,
            }
        }

        fn scheduler(&mut self) -> &mut dyn Scheduler {
            self
        }
    }

    fn violation(how: Break) -> Violation {
        check(&mut Faulty::new(how)).expect_err("this environment is broken on purpose")
    }

    #[test]
    fn a_stopped_clock_is_caught() {
        assert_eq!(violation(Break::Stopped), Violation::ClockStopped);
    }

    #[test]
    fn a_clock_running_backwards_is_caught() {
        assert_eq!(violation(Break::Backwards), Violation::ClockWentBackwards);
    }

    #[test]
    fn a_stuck_generator_is_caught() {
        assert_eq!(violation(Break::StuckRandom), Violation::RandomnessIsStuck);
    }

    #[test]
    fn a_choice_outside_the_range_is_caught() {
        assert_eq!(violation(Break::OutOfRange), Violation::ChoiceOutOfRange);
    }

    #[test]
    fn a_non_zero_choice_from_nothing_is_caught() {
        assert_eq!(violation(Break::DegenerateChoice), Violation::DegenerateChoiceIsNotZero);
    }

    #[test]
    fn a_wall_clock_that_changes_its_story_is_caught() {
        assert_eq!(violation(Break::FickleWall), Violation::WallProvenanceMoved);
    }

    #[test]
    fn every_violation_has_a_sentence() {
        let all = [
            Violation::ClockWentBackwards,
            Violation::ClockStopped,
            Violation::DifferenceDisagreesWithOrder,
            Violation::RandomnessIsStuck,
            Violation::ChoiceOutOfRange,
            Violation::DegenerateChoiceIsNotZero,
            Violation::WallProvenanceMoved,
        ];
        for v in all {
            assert!(!v.message().is_empty(), "{v:?} has no message");
        }
    }
}
