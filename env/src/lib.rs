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
#[derive(Clone, Debug)]
pub struct SeededEnv {
    state: u64,
    now: u64,
    tick: u64,
}

impl SeededEnv {
    /// Construct from a seed. `tick_ns` is how far the virtual clock advances
    /// per observation, which makes time progress without ever consulting
    /// hardware.
    #[must_use]
    pub const fn new(seed: u64, tick_ns: u64) -> Self {
        Self {
            // Avoid the all-zero state, which is a fixed point for xorshift.
            state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed },
            now: 0,
            tick: tick_ns,
        }
    }

    /// Advance the virtual clock explicitly, for a test that needs a deadline
    /// to pass without doing work.
    pub const fn advance(&mut self, nanos: u64) {
        self.now = self.now.wrapping_add(nanos);
    }

    fn xorshift(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

impl Scheduler for SeededEnv {
    fn choose(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        // Modulo bias is acceptable here: the goal is a reproducible
        // adversarial choice, not a uniform one.
        (self.xorshift() % u64::from(n)) as u32
    }
}

impl Env for SeededEnv {
    fn now(&self) -> Instant {
        Instant(self.now)
    }

    fn next_u64(&mut self) -> u64 {
        self.now = self.now.wrapping_add(self.tick);
        self.xorshift()
    }

    fn wall(&self) -> Option<WallTime> {
        // Derived from the seeded state rather than read from anywhere, so a
        // run that stamps wall-clock timestamps is still byte-reproducible.
        // The uncertainty is a full second: under simulation this number is
        // reproducible and true of nothing, and the value should say so.
        Some(WallTime {
            tai_nanos: self.now.wrapping_mul(3).wrapping_add(self.state),
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
