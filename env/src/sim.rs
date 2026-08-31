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
//! Each site draws from its own stream, keyed by the seed and by how many times
//! that site has been consulted. This is not a performance choice. A single
//! shared sequence makes a site's answer depend on how many *other* sites were
//! consulted first, so adding a fault check anywhere — in an unrelated
//! subsystem, on a path the failing scenario never enters — shifts every later
//! draw. The seed that reproduced a bug on Monday reproduces nothing on
//! Wednesday, and nobody can tell whether the bug was fixed. A seed is meant to
//! be a complete bug report; one that expires silently is worse than none.
//!
//! [`SimEnv::focused_on`] is the other half of what `site` was for: narrowing a
//! sweep to the transitions being investigated, without changing what it finds
//! at the sites it is still looking at.
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

/// A per-site counter, so each site advances independently of the others.
///
/// Sixteen sites, which is more than the tree has and fewer than a heap would
/// be needed for. This crate is compiled into the kernel and there is no
/// allocator; a fixed table is what that leaves. Overflowing it is not silent —
/// see [`SimEnv::should_fail`].
const SITES: usize = 16;

/// An `Env` that injects faults, driven entirely by its seed.
///
/// Two runs with the same seed inject the same faults at the same sites, which
/// is what makes a failing seed a complete bug report: it reproduces byte for
/// byte on any machine at that commit.
///
/// # Why each site has its own stream
///
/// The first version of this drew every decision from one sequence, so the
/// answer at a site depended on how many *other* sites had been consulted
/// before it. Two consequences, and the second is the one that matters.
///
/// It made `site` decorative: the label was taken and dropped, while the
/// module documentation claimed protocol-aware injection. Aiming at a
/// transition was not possible, because nothing could aim.
///
/// And it made a failing seed fragile in the worst way. Adding a fault check
/// anywhere — in an unrelated subsystem, in a code path the failing scenario
/// never enters — shifts every later draw, so the seed that reproduced a bug
/// on Monday reproduces nothing on Wednesday and nobody knows whether the bug
/// was fixed. A seed is supposed to be a bug report; that one was a bug report
/// with an expiry date nobody could see.
///
/// A site's decision now depends on the seed, the site, and how many times
/// *that* site has been consulted. Adding a site perturbs nothing.
#[derive(Clone, Debug)]
pub struct SimEnv {
    inner: SeededEnv,
    /// The seed, kept because per-site draws are computed from it rather than
    /// taken from `inner`'s stream — mixing the two would put the ordering
    /// dependency straight back.
    seed: u64,
    /// Chance in a thousand that any given site faults.
    rate_per_mille: u32,
    /// When non-empty, only these sites may fault.
    focus: [Option<&'static str>; SITES],
    /// Site labels in first-seen order, and how many times each was consulted.
    sites: [Option<&'static str>; SITES],
    counts: [u64; SITES],
    injected: u32,
    /// Sites seen past the table's capacity. Reported rather than ignored.
    overflowed: u32,
}

impl SimEnv {
    /// Construct a simulation environment.
    ///
    /// `rate_per_mille` of zero disables injection, which is the right setting
    /// for a run that is establishing a baseline rather than hunting.
    #[must_use]
    pub const fn new(seed: u64, tick_ns: u64, rate_per_mille: u32) -> Self {
        Self {
            inner: SeededEnv::new(seed, tick_ns),
            seed,
            rate_per_mille,
            focus: [None; SITES],
            sites: [None; SITES],
            counts: [0; SITES],
            injected: 0,
            overflowed: 0,
        }
    }

    /// Inject only at these sites.
    ///
    /// This is what the `site` parameter was always for. Uniformly random
    /// faults spend most of their budget on uninteresting states; aiming at the
    /// moment a capability is revoked, or the instant an epoch increments, is
    /// how a sweep finds the bug that lives in a transition rather than in a
    /// steady state.
    ///
    /// Sites outside the list never fault, and the ones inside keep the exact
    /// decisions they would have made unfocused — because a site's stream is
    /// its own. Narrowing a sweep therefore does not change what it finds where
    /// it is still looking, which is the property that makes narrowing a safe
    /// thing to do while debugging.
    #[must_use]
    pub const fn focused_on(mut self, sites: &[&'static str]) -> Self {
        let mut i = 0;
        while i < sites.len() && i < SITES {
            self.focus[i] = Some(sites[i]);
            i += 1;
        }
        self
    }

    /// How many faults this run has injected. Reported alongside a failure so
    /// a seed's severity is visible.
    #[must_use]
    pub const fn injected(&self) -> u32 {
        self.injected
    }

    /// Sites this run could not track, because the table is fixed.
    ///
    /// Non-zero means injection at those sites was *skipped*, not that it
    /// silently went elsewhere. A fault harness that quietly stops covering
    /// part of the system is worse than one that covers less and says so.
    #[must_use]
    pub const fn overflowed(&self) -> u32 {
        self.overflowed
    }

    /// Advance the virtual clock without doing work.
    pub const fn advance(&mut self, nanos: u64) {
        self.inner.advance(nanos);
    }

    /// Is this site in focus?
    fn in_focus(&self, site: &str) -> bool {
        let mut any = false;
        for name in self.focus.iter().flatten() {
            any = true;
            if *name == site {
                return true;
            }
        }
        // An empty focus means everything is in focus, which is what makes
        // `focused_on` opt-in rather than something every caller has to set.
        !any
    }

    /// This site's slot, allocating one on first sight.
    fn slot(&mut self, site: &'static str) -> Option<usize> {
        for (i, entry) in self.sites.iter().enumerate() {
            match entry {
                Some(name) if *name == site => return Some(i),
                Some(_) => {}
                None => {
                    self.sites[i] = Some(site);
                    return Some(i);
                }
            }
        }
        self.overflowed = self.overflowed.saturating_add(1);
        None
    }
}

/// One draw for a site, from the seed and the site's own occurrence count.
///
/// A small integer hash rather than a stream, because the whole point is that
/// this site's answer does not depend on what any other site did. Splitmix64's
/// finaliser: cheap, no state, and it decorrelates the low bits of the
/// concatenated inputs — which matters here because the inputs are a seed, a
/// short label and a small counter, all of which are highly structured.
fn draw(seed: u64, site: &str, occurrence: u64) -> u64 {
    // FNV-1a over the label, so two sites with a similar name do not share a
    // trajectory. The label is a compile-time constant in every caller, so this
    // is a handful of instructions on a path that only runs under simulation.
    let mut label: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in site.as_bytes() {
        label ^= u64::from(*byte);
        label = label.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let mut x = seed ^ label.rotate_left(17) ^ occurrence.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Faults for SimEnv {
    fn should_fail(&mut self, site: &'static str) -> Option<Fault> {
        if self.rate_per_mille == 0 || !self.in_focus(site) {
            return None;
        }
        let index = self.slot(site)?;
        let occurrence = self.counts[index];
        self.counts[index] = occurrence.wrapping_add(1);

        let roll = draw(self.seed, site, occurrence);
        if roll % 1000 >= u64::from(self.rate_per_mille) {
            return None;
        }
        self.injected += 1;

        // The kind is drawn from the same site-local stream, one occurrence on,
        // so a seed selects the whole trajectory at a site rather than only
        // where it strikes.
        let kind = draw(self.seed, site, occurrence ^ 0xffff_ffff_ffff_ffff);
        match kind % 3 {
            0 => Some(Fault::Fail),
            1 => Some(Fault::Delay(draw(self.seed, site, occurrence.wrapping_add(1)) % 10_000)),
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
    fn a_site_is_unaffected_by_what_other_sites_did() {
        // The property the per-site streams exist for, and the one a shared
        // sequence cannot have. A seed that reproduced a bug must keep
        // reproducing it when a fault check is added somewhere the failing
        // scenario never goes — otherwise a seed is a bug report with an
        // expiry date nobody can see.
        let mut alone = SimEnv::new(99, 10, 300);
        let mut interleaved = SimEnv::new(99, 10, 300);

        let mut a = [None; 40];
        for slot in &mut a {
            *slot = alone.should_fail("ring.publish");
        }

        let mut b = [None; 40];
        for slot in &mut b {
            // Two unrelated sites consulted between every draw, exactly as a
            // later commit adding checks elsewhere would do.
            let _ = interleaved.should_fail("store.zone_reset");
            let _ = interleaved.should_fail("cap.revoke");
            *slot = interleaved.should_fail("ring.publish");
        }

        assert_eq!(a, b, "another site's traffic changed this site's trajectory");
    }

    #[test]
    fn two_sites_do_not_share_a_trajectory() {
        // The other direction: per-site streams must not all be the same
        // stream. Sites keyed only by their counter would fault in lockstep,
        // which looks like reproducibility and is a harness testing one thing
        // three times.
        let mut env = SimEnv::new(2024, 10, 400);
        let mut publish = [None; 60];
        let mut revoke = [None; 60];
        for i in 0..60 {
            publish[i] = env.should_fail("ring.publish");
            revoke[i] = env.should_fail("cap.revoke");
        }
        assert_ne!(publish, revoke, "two sites drew identical trajectories");
    }

    #[test]
    fn focus_silences_everything_else() {
        let mut env = SimEnv::new(7, 10, 500).focused_on(&["cap.revoke"]);
        let mut struck = 0;
        for _ in 0..200 {
            assert!(env.should_fail("ring.publish").is_none(), "an unfocused site faulted");
            if env.should_fail("cap.revoke").is_some() {
                struck += 1;
            }
        }
        assert!(struck > 0, "the focused site never faulted, so nothing was proved");
    }

    #[test]
    fn focus_does_not_change_what_the_focused_site_sees() {
        // Narrowing a sweep while debugging must not change the answers where
        // it is still looking. If it did, focusing would be a different
        // experiment rather than a smaller one.
        let mut wide = SimEnv::new(31337, 10, 250);
        let mut narrow = SimEnv::new(31337, 10, 250).focused_on(&["ring.publish"]);

        for _ in 0..50 {
            let _ = wide.should_fail("cap.revoke");
            let w = wide.should_fail("ring.publish");
            let _ = narrow.should_fail("cap.revoke");
            let n = narrow.should_fail("ring.publish");
            assert_eq!(w, n, "focusing changed the focused site's trajectory");
        }
    }

    #[test]
    fn more_sites_than_the_table_holds_is_reported_not_hidden() {
        // A fault harness that quietly stops covering part of the system is
        // worse than one that covers less and says so: the sweep still reports
        // green, and the gap is invisible.
        let mut env = SimEnv::new(5, 10, 1000);
        const NAMES: [&str; 20] = [
            "s00", "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09", "s10", "s11",
            "s12", "s13", "s14", "s15", "s16", "s17", "s18", "s19",
        ];
        for name in NAMES {
            let _ = env.should_fail(name);
        }
        assert!(
            env.overflowed() >= 4,
            "sites past the table must be counted, got {}",
            env.overflowed()
        );
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
