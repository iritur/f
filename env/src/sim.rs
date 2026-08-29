// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The seed of layer 1: an [`Env`] that injects faults on a schedule the seed
//! decides.
//!
//! The full simulator — modelled devices, whole-system fault injection, seed
//! sweeps as a CI workload — arrives at phase 01, when there are drivers to
//! model. What exists here is the part that must not be retrofitted: the
//! *hook*. Once code asks its `Env` whether an operation should fail, adding a
//! new fault class is a change to this file. If code instead assumes operations
//! succeed, every call site has to be revisited later.
//!
//! # Protocol-aware injection
//!
//! Uniformly random faults spend most of their budget on uninteresting states.
//! [`Faults::should_fail`] takes a `site` label so injection can be aimed at
//! protocol transitions — the moment a capability is revoked, the instant a
//! ring epoch increments, the window between claim and publish. That is a
//! refinement current practice arrived at the hard way; taking it from the
//! start costs one parameter.
//!
//! See `docs/design/proving-ground.html` layer 1.

use crate::{Env, Instant, Scheduler, SeededEnv, WallTime};

/// What a simulated failure looks like to the code under test.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The operation reports failure. The caller must handle it.
    Fail,
    /// The operation takes far longer than usual but succeeds. Catches code
    /// that is correct but misses a deadline.
    Delay(u64),
    /// The peer on the other end of this channel restarted. Outstanding
    /// tokens are stale.
    PeerRestart,
}

/// Decides whether a given site fails on this run.
pub trait Faults {
    /// Should the operation at `site` fail right now?
    ///
    /// `site` is a stable label — `"ring.publish"`, `"cap.revoke"`,
    /// `"store.zone_reset"` — so a failing seed names where it struck.
    fn should_fail(&mut self, site: &'static str) -> Option<Fault>;
}

/// An `Env` that injects faults, driven entirely by its seed.
///
/// Two runs with the same seed inject the same faults at the same sites, which
/// is what makes a failing seed a complete bug report: it reproduces byte for
/// byte on any machine at that commit.
#[derive(Clone, Debug)]
pub struct SimEnv {
    inner: SeededEnv,
    /// Chance in a thousand that any given site faults.
    rate_per_mille: u32,
    injected: u32,
}

impl SimEnv {
    /// Construct a simulation environment.
    ///
    /// `rate_per_mille` of zero disables injection, which is the right setting
    /// for a run that is establishing a baseline rather than hunting.
    #[must_use]
    pub const fn new(seed: u64, tick_ns: u64, rate_per_mille: u32) -> Self {
        Self { inner: SeededEnv::new(seed, tick_ns), rate_per_mille, injected: 0 }
    }

    /// How many faults this run has injected. Reported alongside a failure so
    /// a seed's severity is visible.
    #[must_use]
    pub const fn injected(&self) -> u32 {
        self.injected
    }

    /// Advance the virtual clock without doing work.
    pub const fn advance(&mut self, nanos: u64) {
        self.inner.advance(nanos);
    }
}

impl Faults for SimEnv {
    fn should_fail(&mut self, _site: &'static str) -> Option<Fault> {
        if self.rate_per_mille == 0 {
            return None;
        }
        let roll = self.inner.next_u64() % 1000;
        if roll >= u64::from(self.rate_per_mille) {
            return None;
        }
        self.injected += 1;
        // The kind is also seed-derived, so a seed selects the whole trajectory
        // rather than only where it strikes.
        match self.inner.next_u64() % 3 {
            0 => Some(Fault::Fail),
            1 => Some(Fault::Delay(self.inner.next_u64() % 10_000)),
            _ => Some(Fault::PeerRestart),
        }
    }
}

impl Env for SimEnv {
    fn now(&self) -> Instant {
        self.inner.now()
    }

    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// Forwarded rather than defaulted. The default is `None`, and a simulator
    /// that answers `None` would make every path which stamps a wall-clock time
    /// take its no-clock branch under simulation and only ever take the other
    /// one on hardware — which is the branch nothing would then be testing.
    fn wall(&self) -> Option<WallTime> {
        self.inner.wall()
    }

    fn scheduler(&mut self) -> &mut dyn Scheduler {
        self.inner.scheduler()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long enough that a divergence between two seeds shows up, short enough
    /// to sit on the stack — this crate is compiled into the kernel, which has
    /// no heap, so its tests do not get one either.
    const TRACE: usize = 300;

    fn trace(seed: u64) -> ([Option<Fault>; TRACE], u32) {
        let mut env = SimEnv::new(seed, 10, 200);
        let sites = ["ring.publish", "cap.revoke", "store.zone_reset"];
        let mut out = [None; TRACE];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = env.should_fail(sites[i % sites.len()]);
        }
        (out, env.injected())
    }

    #[test]
    fn a_seed_reproduces_its_faults_exactly() {
        let (a, na) = trace(1234);
        let (b, nb) = trace(1234);
        assert_eq!(a, b, "same seed must inject the same faults at the same sites");
        assert_eq!(na, nb);
    }

    #[test]
    fn different_seeds_diverge() {
        let (a, _) = trace(1234);
        let (b, _) = trace(5678);
        assert_ne!(a, b, "different seeds must explore different trajectories");
    }

    #[test]
    fn the_simulator_keeps_the_seeded_wall_clock() {
        let a = SimEnv::new(4242, 10, 0);
        let b = SimEnv::new(4242, 10, 0);
        let stamp = a.wall().expect("a simulated run has a wall clock");
        assert_eq!(stamp.source, crate::WallSource::Simulated);
        assert_eq!(a.wall(), b.wall(), "a seed must reproduce its wall clock");
    }

    #[test]
    fn zero_rate_injects_nothing() {
        let mut env = SimEnv::new(42, 10, 0);
        for _ in 0..1000 {
            assert!(env.should_fail("ring.publish").is_none());
        }
        assert_eq!(env.injected(), 0);
    }

    #[test]
    fn injection_rate_is_roughly_as_configured() {
        let mut env = SimEnv::new(7, 1, 100); // 10%
        let n = 10_000;
        for _ in 0..n {
            let _ = env.should_fail("ring.publish");
        }
        let rate = f64::from(env.injected()) / f64::from(n);
        assert!((0.05..0.15).contains(&rate), "expected roughly 10% injection, got {rate:.3}");
    }
}
