// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The determinism substrate.
//!
//! # The rule
//!
//! **Nothing in the system observes nondeterminism except through an [`Env`].**
//!
//! Time is not an instruction, it is a capability. Randomness is a capability.
//! Interrupt arrival, ring consumer ordering, core assignment and allocation
//! addresses are all decided by a policy that is real in production and seeded
//! under test.
//!
//! The contract this buys:
//!
//! ```text
//! (seed, commit_hash) -> byte-identical execution, always.
//! ```
//!
//! # Why this crate exists at milestone M0 and not later
//!
//! Every other testing layer can be added at ordinary cost. This one cannot:
//! retrofitting determinism means finding and re-plumbing every source of
//! nondeterminism in a system that has grown around their absence. That is why
//! no mainstream kernel has it and why none of them will get it.
//!
//! `cargo xtask lint-determinism` fails the build on any direct use of a
//! forbidden source. See `docs/design/proving-ground.html` section 04.

#![no_std]

pub mod contract;
pub mod sim;
pub mod split;

/// A point in time, in nanoseconds since an arbitrary origin.
///
/// Monotonic within one [`Env`]. Comparable only against instants from the
/// same `Env` — comparing a simulated instant to a hardware one is a bug the
/// type system deliberately does not catch, because both are just numbers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Instant(pub u64);

impl Instant {
    /// Nanoseconds since this `Env`'s origin.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Saturating difference. Never panics, never wraps: a clock that appears
    /// to move backwards is a peer bug or a simulator fault, not a reason to
    /// abort.
    #[must_use]
    pub const fn saturating_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// Decides every ordering the system would otherwise leave to chance.
///
/// In production this defers to the real scheduler. Under simulation it is
/// driven by the seed, which is what makes an interleaving reproducible.
pub trait Scheduler {
    /// Choose among `n` ready alternatives. Returns an index below `n`.
    ///
    /// Callers must treat any return value as valid input and must not assume
    /// fairness — a simulator will deliberately choose adversarially.
    fn choose(&mut self, n: u32) -> u32;

    /// A point at which the system would tolerate being preempted or reordered.
    /// Free in production; a decision point under simulation.
    fn yield_point(&mut self) {}
}

/// Where a wall-clock reading came from.
///
/// Carried with the value because a timestamp without provenance cannot be
/// reasoned about: "the firmware said so" and "a peer said so" are different
/// claims, and only one of them survives the peer being wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WallSource {
    /// The platform's own real-time clock.
    Firmware,
    /// Disciplined against an external reference.
    Network,
    /// Asserted by another machine, and no better than that machine.
    Peer,
    /// Produced by a seed. Reproducible, and true of nothing.
    Simulated,
}

/// A moment in civil time, as a *datum* rather than as a clock.
///
/// # This may not order anything
///
/// Wall time jumps. It is set by people, disciplined by daemons, carries leap
/// seconds in most encodings, and on many machines is simply wrong. Ordering a
/// system event by it is the family of bugs RFC 0009 exists to make
/// unavailable, so this type deliberately implements neither `Ord` nor
/// `PartialOrd`: sorting by wall time has to be spelled out as sorting by
/// [`WallTime::tai_nanos`], where a reader can see it and ask why.
///
/// [`Instant`] is the clock. This is a label.
///
/// TAI rather than UTC, because TAI has no leap seconds — which is the whole
/// reason the conversion belongs in the semantic layer, beside the calendar and
/// the time zone, and not down here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WallTime {
    /// TAI nanoseconds since the Unix epoch.
    pub tai_nanos: u64,
    /// How far wrong this could be. Zero is a claim, not a default: a source
    /// that does not know its own error should say so with a large number.
    pub uncertainty_nanos: u64,
    /// Who is asserting it.
    pub source: WallSource,
}

/// The only legitimate source of time, randomness and ordering.
pub trait Env {
    /// Current time. Virtual under simulation, hardware-derived in production.
    fn now(&self) -> Instant;

    /// Next pseudo-random value. Seeded under simulation.
    fn next_u64(&mut self) -> u64;

    /// The ordering policy.
    fn scheduler(&mut self) -> &mut dyn Scheduler;

    /// Civil time, if this machine has any business claiming to know it.
    ///
    /// `None` is the honest answer on a machine with no trustworthy clock, and
    /// the reason this returns an `Option` at all: the alternative is a
    /// plausible fabricated number, and a fabricated timestamp is worse than a
    /// missing one precisely because it is usable.
    ///
    /// It is a method on `Env` rather than a service call so that a run which
    /// stamps timestamps stays reproducible from its seed. A service could be
    /// asked without the substrate knowing, and then a seed would stop
    /// reproducing the run.
    ///
    /// The default is `None`: an implementation acquires a wall clock
    /// deliberately.
    fn wall(&self) -> Option<WallTime> {
        None
    }
}

/// A deterministic `Env` driven entirely by a seed.
///
/// Two runs with the same seed produce the same instants, the same random
/// values and the same orderings. This is the environment every test uses.
///
/// The generator is [`split::Stream`], which is also what `sim.rs` derives its
/// per-site draws from — one derivation for the whole crate rather than two that
/// each admitted in a comment that they were chosen for reproducibility and not
/// for statistical quality. RFC 0026 says why that admission stopped being
/// affordable at the point the simulator began multiplying streams.
#[derive(Clone, Debug)]
pub struct SeededEnv {
    stream: split::Stream,
    now: u64,
    tick: u64,
}

/// The identity the wall clock is derived at.
///
/// The ASCII of `wall`. A wall-clock reading is a different stream under the
/// same seed rather than a function of the draw stream's state, so stamping a
/// timestamp and drawing a value cannot correlate, and neither one moves the
/// other. That is the same argument `sim.rs` makes for sites, applied to the two
/// things one `Env` hands out.
const WALL_IDENTITY: u64 = 0x7761_6c6c;

impl SeededEnv {
    /// Construct from a seed. `tick_ns` is how far the virtual clock advances
    /// per observation, in nanoseconds, which makes time progress without ever
    /// consulting hardware.
    ///
    /// There is no guard against a zero seed any more. xorshift needed one
    /// because all-zero is a fixed point of a linear map; [`split::Stream`] has
    /// no fixed point at all, because its counter advances whatever the rest of
    /// the state is doing. `split.rs` states the argument and tests it.
    #[must_use]
    pub const fn new(seed: u64, tick_ns: u64) -> Self {
        Self { stream: split::Stream::from_seed(seed), now: 0, tick: tick_ns }
    }

    /// Advance the virtual clock explicitly, for a test that needs a deadline
    /// to pass without doing work.
    pub const fn advance(&mut self, nanos: u64) {
        self.now = self.now.wrapping_add(nanos);
    }
}

impl Scheduler for SeededEnv {
    fn choose(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        // Modulo bias is acceptable here: the goal is a reproducible
        // adversarial choice, not a uniform one. The bias is bounded by
        // n / 2^64 and is invisible for any `n` a scheduler will ever be given;
        // what would not be invisible is a generator whose low bits were poor,
        // which is what `choose_is_balanced_enough_to_be_a_choice` checks.
        (self.stream.next_u64() % u64::from(n)) as u32
    }
}

impl Env for SeededEnv {
    fn now(&self) -> Instant {
        Instant(self.now)
    }

    fn next_u64(&mut self) -> u64 {
        self.now = self.now.wrapping_add(self.tick);
        self.stream.next_u64()
    }

    fn wall(&self) -> Option<WallTime> {
        // Derived from the seed rather than read from anywhere, so a run that
        // stamps wall-clock timestamps is still byte-reproducible. It is derived
        // at its own identity rather than from the generator's live state, so
        // that the number a run stamps does not depend on how many values it
        // happened to draw first — the same independence `sim.rs` gives sites.
        // The uncertainty is a full second: under simulation this number is
        // reproducible and true of nothing, and the value should say so.
        Some(WallTime {
            tai_nanos: self
                .now
                .wrapping_mul(3)
                .wrapping_add(split::derive(self.stream.origin(), WALL_IDENTITY)),
            uncertainty_nanos: 1_000_000_000,
            source: WallSource::Simulated,
        })
    }

    fn scheduler(&mut self) -> &mut dyn Scheduler {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_run() {
        let run = |seed| {
            let mut env = SeededEnv::new(seed, 10);
            let mut out = [0u64; 32];
            for slot in &mut out {
                *slot = env.next_u64() ^ env.now().as_nanos();
            }
            out
        };
        assert_eq!(run(42), run(42), "a seed must reproduce its run exactly");
        assert_ne!(run(42), run(43), "different seeds must diverge");
    }

    #[test]
    fn choose_is_in_range_and_reproducible() {
        let mut a = SeededEnv::new(7, 1);
        let mut b = SeededEnv::new(7, 1);
        for _ in 0..1000 {
            let x = a.scheduler().choose(5);
            assert!(x < 5);
            assert_eq!(x, b.scheduler().choose(5));
        }
    }

    #[test]
    fn choose_handles_degenerate_n() {
        let mut env = SeededEnv::new(1, 1);
        assert_eq!(env.scheduler().choose(0), 0);
        assert_eq!(env.scheduler().choose(1), 0);
    }

    #[test]
    fn wall_time_is_seeded_like_everything_else() {
        let a = SeededEnv::new(99, 10);
        let b = SeededEnv::new(99, 10);
        assert_eq!(a.wall(), b.wall(), "a seed must reproduce its wall clock too");

        let other = SeededEnv::new(100, 10);
        assert_ne!(a.wall(), other.wall(), "different seeds must diverge");

        let stamp = a.wall().expect("the seeded env has a clock");
        assert_eq!(stamp.source, WallSource::Simulated);
        assert!(stamp.uncertainty_nanos > 0, "a simulated clock is not precise");
    }

    #[test]
    fn choose_is_balanced_enough_to_be_a_choice() {
        // `choose` reduces a draw modulo `n`, so what a scheduler actually sees
        // is the generator's *low* bits, and nothing else in this crate looks at
        // them alone. A generator biased there gives a simulator that explores
        // one branch while reporting that it explored both, which is the same
        // failure as a correlated stream wearing a different hat.
        //
        // Integer arithmetic and a derived band, like every statistic in this
        // crate: 4096 draws of `choose(2)` is Binomial(4096, 1/2), mean 2048 and
        // a standard deviation of 32, so five sigma is 160. It detects a stuck
        // low bit and a heavy bias. It does not detect a pattern in the sequence
        // of choices, which is a different test and a different generator flaw.
        const DRAWS: u32 = 4096;
        const MEAN: u32 = DRAWS / 2;
        const BAND: u32 = 160;

        let mut env = SeededEnv::new(0xB1A5, 1);
        let mut ones = 0;
        for _ in 0..DRAWS {
            ones += env.scheduler().choose(2);
        }
        assert!(
            ones.abs_diff(MEAN) <= BAND,
            "choose(2) picked 1 {ones} times in {DRAWS}, which is outside {MEAN} +/- {BAND}"
        );
    }

    #[test]
    fn the_wall_clock_does_not_depend_on_how_many_values_were_drawn() {
        // It is derived at its own identity rather than from the live generator
        // state, so a component that stamps a timestamp reads the same number
        // whether or not something unrelated drew first. The old form mixed the
        // generator's state in, which made a stamp depend on traffic elsewhere
        // — the shape of bug `sim.rs` removed for sites.
        let mut busy = SeededEnv::new(0x5EED, 10);
        let quiet = SeededEnv::new(0x5EED, 10);
        for _ in 0..64 {
            let _ = busy.next_u64();
        }
        let a = busy.wall().expect("the seeded env has a clock");
        let b = quiet.wall().expect("the seeded env has a clock");
        // The clock has moved, so only the seeded part is compared: subtract the
        // contribution the elapsed time makes.
        let strip = |w: WallTime, at: u64| w.tai_nanos.wrapping_sub(at.wrapping_mul(3));
        assert_eq!(
            strip(a, busy.now().as_nanos()),
            strip(b, quiet.now().as_nanos()),
            "a draw moved the wall clock's seeded component"
        );
    }

    #[test]
    fn time_never_goes_backwards() {
        let mut env = SeededEnv::new(99, 3);
        let mut last = env.now();
        for _ in 0..1000 {
            let _ = env.next_u64();
            let now = env.now();
            assert!(now >= last);
            last = now;
        }
    }
}
