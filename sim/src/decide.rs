// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Every point where two things could happen in either order, and the record
//! that makes each one nameable.
//!
//! # Why a decision has to have a name
//!
//! Two tasks downstream of this one need to talk about a single decision inside
//! a run. `E1-P03` minimises a failing sweep down to a reproduction command, and
//! a minimiser that can only say *this seed fails* has nothing to shrink.
//! `E1-P08` re-enters a run at a point, and a point that cannot be named cannot
//! be re-entered. So a decision is not just made here, it is written down.
//!
//! Each one is written down twice, under two names, because the two names
//! answer different questions and a report that carries only one of them is a
//! report that stops being true at the next commit:
//!
//! - **The ordinal** is this decision's position in this run. It is what
//!   `E1-P08` re-enters at and what a bisect counts to. It is meaningful for one
//!   `(seed, commit)` pair and for no other, because a commit that consults a
//!   new decision site anywhere shifts every ordinal after it.
//! - **The site and its occurrence** name the decision across commits: *the
//!   fourth time `service.next` was asked*. Adding a site elsewhere does not
//!   move it, which is the whole property [`draw`] is built to have.
//!
//! # The derivation, and the domain reserved beside it
//!
//! [`draw`] is `f_env::split::derive` applied three times: the run's seed is
//! keyed by a *domain*, that by a *site*, and that by the site's own occurrence
//! count. `derive` is injective in each argument, so no two sites share a stream
//! and no two occurrences at one site share a value. RFC 0026 is the argument
//! and `env/src/sim.rs` is where the two-level form of it already lives; this
//! adds the third level, and the third level is the point.
//!
//! [`domain::FAULTS`] is the second domain, and `sim/src/fault.rs` is the only
//! thing that draws there — one draw per class consultation, keyed by the
//! class's label and its own occurrence. The word was reserved a task before it
//! was spent, and the argument for reserving it early is the reason it is now
//! spendable at all: without a domain of their own the first fault draw would
//! have had to key off a site label — colliding with the ordering draw at the same site, and moving
//! every interleaving a seed had already selected. Every seed recorded before
//! `E1-P02` would have silently stopped reproducing its run, which is the exact
//! failure `env/src/sim.rs` spends four paragraphs removing. A domain word is
//! the cheapest possible time to pay for it, and `E1-P01` was that time:
//! `E1-B11` was placed before `E1-P01` on the same reasoning, because a seed
//! corpus is priced in the derivation it was drawn from. The bill came due one
//! task later and was zero, which is what a word costing nothing looks like when
//! it is spent on time.

use std::collections::BTreeMap;

use f_env::split;

/// The top level of the derivation: what kind of question is being asked.
///
/// A domain is a stable string, mixed before the site is, so that two questions
/// asked at one site draw from two unrelated streams. Adding a domain perturbs
/// nothing, exactly as adding a site does not.
pub mod domain {
    /// Which of several things that could happen in either order happens first.
    pub const ORDERING: &str = "sim.ordering";

    /// Whether an operation fails, and how. `sim/src/fault.rs` is the only
    /// caller: each of its seven classes draws here at its own label and its own
    /// occurrence, so a class's answer is independent of every ordering decision
    /// and of every other class. The module documentation says why the word was
    /// reserved a task before anything drew at it.
    pub const FAULTS: &str = "sim.faults";
}

/// One draw, from a seed, a domain, a site and that site's occurrence count.
///
/// Pure: the same four arguments always give the same value, and no state
/// anywhere moves. That is what makes a site's answer independent of how much
/// traffic every other site saw, and it is the property a recorded seed rests
/// on.
///
/// Unit: none — an opaque 64-bit value. A caller that wants a bounded choice
/// reduces it; [`Decisions::decide`] is the only such caller here.
#[must_use]
pub fn draw(seed: u64, domain: &str, site: &str, occurrence: u64) -> u64 {
    let by_domain = split::derive(seed, split::label(domain));
    let by_site = split::derive(by_domain, split::label(site));
    split::derive(by_site, occurrence)
}

/// One interleaving decision, as it is written down.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Decision {
    /// Position in this run, counting from zero. Unit: decisions. Meaningful
    /// for one `(seed, commit)` pair only — see the module documentation.
    pub ordinal: u32,
    /// The virtual clock when it was made. Unit: nanoseconds, on the
    /// simulator's own clock, whose zero is the start of the run.
    pub at_ns: u64,
    /// Where in the model the decision was taken. Unit: none — a stable label.
    pub site: &'static str,
    /// How many times this site had been asked before. Unit: occurrences at
    /// this site, zero-based.
    pub occurrence: u64,
    /// How many alternatives there were. Unit: alternatives; never below two,
    /// because a decision with one alternative is not recorded.
    pub arity: u32,
    /// Which one was taken. Unit: an index below `arity`, zero-based.
    pub taken: u32,
}

/// The seeded ordering policy, and the log of what it decided.
///
/// One of these per run. It holds no clock — the caller passes the instant in,
/// because the clock belongs to the timeline and a decision that could read a
/// clock of its own would be a second clock.
#[derive(Clone, Debug)]
pub struct Decisions {
    seed: u64,
    /// Occurrences per site. A `BTreeMap` and not a fixed table: this crate has
    /// a heap, so the sixteen-site limit `env/src/sim.rs` lives under —
    /// and reports overflow against, because it must — does not apply here.
    /// `BTreeMap` and not a hash map because iteration order is seeded per
    /// process, which is RFC 0004's whole subject.
    counts: BTreeMap<&'static str, u64>,
    log: Vec<Decision>,
    /// Decisions taken before this log begins. Unit: decisions.
    ///
    /// Zero for every run that starts at the beginning. Non-zero only after a
    /// *terse* restore, where the log itself did not travel — `trace::Carried`
    /// argues why, and the argument is the same one twice: a decision log is a
    /// **record** and not state. What the next draw depends on is `counts`, and
    /// what the log is for is a person reading a failing run afterwards. So the
    /// cheap snapshot keeps the state and drops the record, and this is what
    /// stops the ordinals restarting at zero when it does.
    carried: u64,
}

impl Decisions {
    /// A fresh policy for one run.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { seed, counts: BTreeMap::new(), log: Vec::new(), carried: 0 }
    }

    /// Choose among `arity` alternatives at `site`, and write the choice down.
    ///
    /// # A single alternative is not a decision
    ///
    /// `arity` below two answers zero, records nothing, and does not advance the
    /// site's occurrence count. That is not an optimisation. If a degenerate
    /// call were recorded, an ordinal would depend on how many one-element
    /// choices a commit happened to reach, and a site's occurrence count would
    /// depend on how often a queue happened to hold exactly one item — so both
    /// names in the module documentation would move for reasons that have
    /// nothing to do with the run's ordering. `f_env::Scheduler` already
    /// specifies zero as the answer for zero and one alternatives, and
    /// `env/src/contract.rs` checks it, so this agrees with the contract rather
    /// than inventing a rule.
    pub fn decide(&mut self, at_ns: u64, site: &'static str, arity: u32) -> u32 {
        if arity <= 1 {
            return 0;
        }
        let occurrence = self.counts.entry(site).or_insert(0);
        let at = *occurrence;
        *occurrence = at.wrapping_add(1);

        // Modulo bias, on the same argument `f_env::SeededEnv::choose` makes:
        // the goal is a reproducible adversarial choice, not a uniform one, and
        // the bias is bounded by `arity / 2^64`.
        let taken = (draw(self.seed, domain::ORDERING, site, at) % u64::from(arity)) as u32;

        // Saturating, so that a run long enough to overflow the ordinal reports
        // a wrong ordinal rather than panicking mid-run. It would take four
        // billion decisions; the budget in `scenario.rs` stops a run long before
        // that, and this is here so the two limits cannot disagree silently.
        //
        // Counted from `taken` and not from the log's length, because a run
        // restored from a terse snapshot has a log that starts part-way through
        // and an ordinal that must not.
        let ordinal = self.taken();
        self.log.push(Decision { ordinal, at_ns, site, occurrence: at, arity, taken });
        taken
    }

    /// Every decision this run has taken, in order.
    #[must_use]
    pub fn log(&self) -> &[Decision] {
        &self.log
    }

    /// Write this policy out.
    ///
    /// **The counts are the whole of the seeded state, and that is RFC 0026
    /// paying for itself a second time.** There is no generator here to
    /// capture: [`draw`] is pure in `(seed, domain, site, occurrence)`, so a
    /// site's next answer is a function of a number this map already holds. A
    /// design that had split by *drawing from a parent* would have to write out
    /// a tree of generator states, and every one of them would be a chance to
    /// lose a field. `env/src/split.rs` is where the same observation is made
    /// from the other side, about the one stream in this crate that is a chain.
    ///
    /// The log travels because the artefact does: `E1-P03` reports a failing
    /// run's decisions and a restored run that had forgotten its own prefix
    /// would report half of them.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer, terse: bool) {
        out.count(self.counts.len());
        for (site, count) in &self.counts {
            out.label(site);
            out.u64(*count);
        }
        if terse {
            // The counts are the state and the log is the record. A terse
            // snapshot keeps the first and drops the second, which is what makes
            // it constant in the length of the run — and what a reader of a
            // re-entered run gives up is the decisions taken before the cut.
            out.u64(self.taken().into());
            out.count(0);
            return;
        }
        out.u64(self.carried);
        out.count(self.log.len());
        for decision in &self.log {
            out.u32(decision.ordinal);
            out.u64(decision.at_ns);
            out.label(decision.site);
            out.u64(decision.occurrence);
            out.u32(decision.arity);
            out.u32(decision.taken);
        }
    }

    /// Read one back, under the seed the snapshot's header names.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>, seed: u64) -> Self {
        let sites = input.count(12, "more decision sites than the file could hold");
        let mut counts = BTreeMap::new();
        for _ in 0..sites {
            let site = input.label();
            counts.insert(site, input.u64());
        }
        let carried = input.u64();
        let taken = input.count(28, "more decisions than the file could hold");
        let mut log = Vec::with_capacity(taken);
        for _ in 0..taken {
            log.push(Decision {
                ordinal: input.u32(),
                at_ns: input.u64(),
                site: input.label(),
                occurrence: input.u64(),
                arity: input.u32(),
                taken: input.u32(),
            });
        }
        Self { seed, counts, log, carried }
    }

    /// How many decisions have been taken. Unit: decisions.
    ///
    /// The run's count and not the log's: after a terse restore they differ, and
    /// the one everything outside this file means is the run's.
    #[must_use]
    pub fn taken(&self) -> u32 {
        let held = u64::try_from(self.log.len()).unwrap_or(u64::MAX);
        u32::try_from(self.carried.saturating_add(held)).unwrap_or(u32::MAX)
    }

    /// Decisions taken before this log begins. Unit: decisions.
    #[must_use]
    pub const fn carried(&self) -> u64 {
        self.carried
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_reproduces_its_decisions_exactly() {
        let run = |seed| {
            let mut d = Decisions::new(seed);
            for i in 0..200u64 {
                let _ = d.decide(i * 10, "timeline.channel", 3);
                let _ = d.decide(i * 10, "service.next", 5);
            }
            d.log().to_vec()
        };
        assert_eq!(run(42), run(42), "a seed must reproduce its interleaving");
        assert_ne!(run(42), run(43), "different seeds must take different interleavings");
    }

    #[test]
    fn a_choice_is_inside_the_range_it_was_given() {
        let mut d = Decisions::new(7);
        for arity in [2u32, 3, 8, 64, 1000, u32::MAX] {
            for _ in 0..64 {
                assert!(d.decide(0, "site", arity) < arity);
            }
        }
    }

    #[test]
    fn a_single_alternative_is_not_recorded() {
        // The property both names in the module documentation rest on. If a
        // degenerate call were recorded, an ordinal would move whenever a queue
        // happened to hold one item rather than two.
        let mut d = Decisions::new(1);
        assert_eq!(d.decide(0, "site", 0), 0);
        assert_eq!(d.decide(0, "site", 1), 0);
        assert_eq!(d.taken(), 0, "a decision with no alternatives was written down");

        // And the site's occurrence count did not move either, so the first
        // real decision at this site is its zeroth.
        let _ = d.decide(0, "site", 2);
        assert_eq!(d.taken(), 1);
        assert_eq!(d.log()[0].occurrence, 0, "a degenerate call consumed an occurrence");
    }

    #[test]
    fn a_sites_answers_do_not_depend_on_what_other_sites_did() {
        // The reason `draw` keys by site and occurrence rather than stepping one
        // shared stream, taken straight from `env/src/sim.rs`: a seed that
        // reproduced a bug must keep reproducing it when a decision point is
        // added somewhere the failing scenario never goes.
        let mut alone = Decisions::new(99);
        let mut crowded = Decisions::new(99);

        let mut a = [0u32; 40];
        for slot in &mut a {
            *slot = alone.decide(0, "service.next", 4);
        }

        let mut b = [0u32; 40];
        for slot in &mut b {
            let _ = crowded.decide(0, "later.one", 3);
            let _ = crowded.decide(0, "later.two", 7);
            *slot = crowded.decide(0, "service.next", 4);
        }

        assert_eq!(a, b, "a decision site was moved by traffic at another site");
    }

    #[test]
    fn the_fault_domain_cannot_move_an_ordering_decision() {
        // What the reserved word buys, checked rather than asserted. The same
        // site, the same occurrence, the same seed, two domains: two unrelated
        // answers, so `E1-P02` adding a fault draw at `service.next` cannot
        // shift the interleaving a recorded seed already selected.
        let seed = 0xC0DE_1234_5678_9ABC;
        let mut same = 0;
        for occurrence in 0..256 {
            let ordering = draw(seed, domain::ORDERING, "service.next", occurrence);
            let faults = draw(seed, domain::FAULTS, "service.next", occurrence);
            assert_ne!(ordering, faults, "the two domains collided at occurrence {occurrence}");
            if ordering % 4 == faults % 4 {
                same += 1;
            }
        }
        // A weak statement on purpose: two independent streams agree on a
        // two-bit reduction about a quarter of the time, so anything close to
        // 256 would mean they are one stream wearing two names. The strong
        // cross-stream bound lives in `env/src/split.rs`, where the derivation
        // this calls is defined and tested.
        assert!(same < 128, "the two domains agreed {same} times in 256, which is not two streams");
    }

    #[test]
    fn an_ordinal_counts_the_run_and_an_occurrence_counts_the_site() {
        let mut d = Decisions::new(5);
        let _ = d.decide(10, "a", 2);
        let _ = d.decide(20, "b", 2);
        let _ = d.decide(30, "a", 2);

        let log = d.log();
        assert_eq!(log.len(), 3);
        assert_eq!((log[0].ordinal, log[0].site, log[0].occurrence), (0, "a", 0));
        assert_eq!((log[1].ordinal, log[1].site, log[1].occurrence), (1, "b", 0));
        assert_eq!((log[2].ordinal, log[2].site, log[2].occurrence), (2, "a", 1));
        assert_eq!(log[2].at_ns, 30, "a decision records the instant it was taken");
    }
}
