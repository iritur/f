// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The seven fault classes, each with a site, a scenario and an assertion.
//!
//! # What a fault class is here, and what it is not
//!
//! A class is three things and no more: a **site** the model consults, a
//! **scenario** in [`crate::scenario::SCENARIOS`] that arms it, and an
//! **assertion** in this file's tests that says in advance what the system must
//! do about it. It is not a subsystem. Everything underneath already existed
//! before this file did — `f_env::sim::Fault` is the vocabulary, `f_env::split`
//! is the derivation, [`crate::decide::domain::FAULTS`] is the reserved word the
//! draw is keyed at, and the device models are where the sites are.
//!
//! The three-part shape is the whole of `E1-P02`'s exit criterion. A scenario
//! that injects a fault and *prints* what happened is an observation: somebody
//! has to read it and decide whether it was right, and next month nobody does.
//! So each class below states its response first — this request is refused with
//! this error in this domain, this component restarts and its clients see only
//! latency, this buffer is never handed back — and the test is what makes it
//! true. The seven tests at the bottom of this file are the deliverable; the
//! machinery above them is what they needed.
//!
//! # Why the draw is keyed at `domain::FAULTS` and not taken through `SimEnv`
//!
//! `f_env::sim` is the hook this consumes, and it is deliberately consumed as a
//! *type* rather than as an environment. [`Fault`] is re-exported from there
//! rather than redefined, so there is one answer in this tree to *what does a
//! simulated failure look like*, and `ring/tests/faults.rs` and this file are
//! talking about the same three things.
//!
//! What is not reused is `SimEnv` itself, and the reason is structural: a
//! [`World`](crate::World) already **is** an `Env`, with the timeline's clock
//! and a randomness stream split off the run's seed. Putting a `SimEnv` inside
//! one would put a second clock and a second generator in a single run, and a
//! reproduction check cannot be taken over two sources of time. So the
//! derivation is [`crate::decide::draw`] at
//! [`domain::FAULTS`](crate::decide::domain::FAULTS) — the domain word `E1-P01`
//! spent in advance for exactly this commit, and `decide.rs` is where the
//! argument for spending it early is written out.
//!
//! # The property that survives adding the eighth class
//!
//! A class's answer depends on the run's seed, the class's own label, and how
//! many times **that class** has been consulted. Nothing else. Adding a class,
//! arming a class, or consulting a class on a path the failing run never enters
//! moves no other class's draws and no ordering decision — the first because
//! `f_env::split::derive` is injective in its identity, the second because the
//! fault domain is keyed before the site is.
//!
//! That is not tidiness. `env/src/sim.rs` spends four paragraphs on it and this
//! file inherits the argument whole: a seed is meant to be a complete bug
//! report, and a bug report that expires the next time somebody adds a fault
//! check somewhere else is worse than none.
//! [`tests::arming_a_class_that_never_fires_leaves_every_scenario_exactly_as_it_was`]
//! is the scenario-level form of it, and it is the one that would actually catch
//! a regression, because it runs the shipped table rather than a fixture.
//!
//! # A plan belongs to a scenario, so a reproduction stays two words
//!
//! What is broken and how often is a field of [`crate::scenario::Scenario`], not
//! a flag on the command line. A reproduction command is therefore still a
//! scenario's name and a seed — `f-sim --trace --seed 0x… <name>` — which is
//! what `E1-P03` has to be able to print out of a failing sweep and what
//! `E1-P08` has to be able to re-enter. A fault plan that lived on the command
//! line would make a failing seed an incomplete bug report, which is the one
//! thing this apparatus is built not to produce.
//!
//! RFC 0039 argues all three decisions, records the alternatives that were live,
//! and names what is deliberately not built — the other tear of a doorbell, a
//! partial write reported as a short used length, and the classes
//! `proving-ground.html` layer 1 lists that `E1-P02` does not. RFC 0032 is the
//! seam it sits on.

use std::collections::BTreeMap;

use crate::decide::{domain, draw};

/// What a simulated failure looks like, re-exported rather than redefined.
///
/// One vocabulary for the whole tree: `ring/tests/faults.rs` injects
/// `Fail`/`Delay`/`PeerRestart` into the real ring and this crate injects the
/// same three into the models. A second enum meaning the same three things is a
/// second enum somebody has to check still means them.
pub use f_env::sim::Fault;

/// What the trace calls the injector. At most [`crate::LABEL_WIDTH`] bytes.
///
/// A strike is **written into the artefact**, hashed with everything else, for
/// the reason `dev.rs` writes a dropped completion down rather than staying
/// silent: a simulator that broke something quietly would produce a trace that
/// reproduces perfectly and describes a run nobody can reason about. It is also
/// what `E1-P03` reads out of a failing run to say what was injected, where, and
/// at which occurrence of the class.
pub const ACTOR: &str = "fault";

/// How long a translation the remapping unit had to fault in may take. Unit:
/// nanoseconds.
///
/// Two orders of magnitude above every `service_ns` in the scenario table,
/// deliberately. A latency class whose delay vanished into a device's own
/// service spread would be a class nothing could assert about — the assertion is
/// *the run finished later and nothing else changed*, and that needs the delay
/// to be the dominant term rather than a plausible one. It is a model parameter
/// and not a measurement: nothing published rests on it.
const FAULT_IN_NS: u64 = 40_000;

/// How long a completion may be held after the device finished it. Unit:
/// nanoseconds.
///
/// Smaller than [`FAULT_IN_NS`], on the same argument and one more: the two
/// latency classes sit at different points of the pipeline — one delays the work
/// and one delays the news — and two classes that were indistinguishable in a
/// trace except by their labels would be one class wearing two names.
const HELD_NS: u64 = 20_000;

/// The seven classes `E1-P02` names, and nothing else.
///
/// The list is closed on purpose. An eighth class is a `TODO.md` line, a
/// scenario and an assertion — the same three things every one of these has —
/// rather than a variant somebody adds because a model grew a branch.
///
/// Four of them are protocol events between components and belong here without
/// argument: [`Self::PeerGone`], [`Self::Doorbell`], [`Self::Partial`] and
/// [`Self::LateCqe`]. The other three are *frame* events, and what is modelled
/// is the **client-visible refusal** rather than the frame that refused — the
/// component asked for memory and was told no, the device named an address its
/// domain does not translate. RFC 0032 draws that seam and says why the
/// simulator sits on this side of it: the exit criterion asks for a system
/// *response*, the response is the component's, and the component is here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// **Allocation failure.** The frame refuses the memory a component asked
    /// for. Injected at the domain, so the refusal comes back through the real
    /// `f_ring::registry::Table` rather than being fabricated beside it.
    Alloc,
    /// **Translation fault.** A descriptor names an address the device's domain
    /// does not translate — the model's stand-in for the fault
    /// `kernel/src/arch/x86_64/dma.rs` provokes on real silicon.
    MapFault,
    /// **Device page-fault latency.** The translation is there, and getting it
    /// took far longer than the transfer did.
    FaultIn,
    /// **Peer death mid-operation.** The device stops with work outstanding.
    PeerGone,
    /// **Torn doorbell.** Publishing an entry and ringing the bell are two
    /// stores, and a torn pair is a bell with nothing behind it.
    Doorbell,
    /// **Partial write.** The payload landed and the device's last write — the
    /// status byte — did not.
    Partial,
    /// **Delayed completion.** The device finished, and the driver was told
    /// late.
    LateCqe,
}

impl Class {
    /// Every class, so a test can hold the set rather than a list that drifts.
    pub const ALL: &'static [Self] = &[
        Self::Alloc,
        Self::MapFault,
        Self::FaultIn,
        Self::PeerGone,
        Self::Doorbell,
        Self::Partial,
        Self::LateCqe,
    ];

    /// The stable label this class draws at and is written into the trace by.
    ///
    /// One name and not two. A site and a trace label that were different
    /// strings would be two spellings of one thing, and a person reading a
    /// minimised failure would need a lookup table between the site a report
    /// names and the record a trace holds. At most [`crate::LABEL_WIDTH`] bytes,
    /// because it goes in the trace's fixed-width `kind` column.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Alloc => "alloc",
            Self::MapFault => "mapfault",
            Self::FaultIn => "faultin",
            Self::PeerGone => "peergone",
            Self::Doorbell => "doorbell",
            Self::Partial => "partial",
            Self::LateCqe => "latecqe",
        }
    }

    /// What this class does when it strikes.
    ///
    /// A class has **one** kind, and it is a property of the class rather than a
    /// third draw. `f_env::sim::SimEnv` draws the kind as well as the strike,
    /// which is right for a sweep hunting anywhere; it is wrong here, because a
    /// class whose *allocation failure* was sometimes a delay would be a class
    /// with no response to state — and the exit criterion is exactly that the
    /// response is stated in advance.
    ///
    /// The two delays draw their magnitude from this class's own stream at the
    /// complement of the occurrence, so a seed selects the whole trajectory at a
    /// class rather than only where it strikes — the same construction
    /// `env/src/sim.rs` uses for the same reason. Never zero: a delay of zero is
    /// a fault that did not happen, and both latency classes assert that the
    /// clock moved.
    fn fault(self, seed: u64, occurrence: u64) -> Fault {
        let span =
            |ns: u64| Fault::Delay(1 + draw(seed, domain::FAULTS, self.label(), !occurrence) % ns);
        match self {
            Self::FaultIn => span(FAULT_IN_NS),
            Self::LateCqe => span(HELD_NS),
            Self::PeerGone => Fault::PeerRestart,
            Self::Alloc | Self::MapFault | Self::Doorbell | Self::Partial => Fault::Fail,
        }
    }
}

/// One class armed, as a scenario states it.
///
/// Two knobs and no more, because both are numbers a person reading a failing
/// seed's reproduction command has to be able to hold in their head — the rule
/// the scenario table itself is written under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Injection {
    /// Which class. Unit: none.
    pub class: Class,
    /// Consultations of this class that pass untouched before it is armed.
    /// Unit: consultations at this class, zero-based.
    ///
    /// What makes *mid-operation* mean something: a peer that dies on the first
    /// consultation dies before it has any work outstanding, which is a
    /// different class and a much weaker one.
    ///
    /// **Not a monotone shrink, and `E1-P03` must not treat it as one.** Raising
    /// it leaves this class's draws exactly where they were — `draw` is keyed by
    /// the occurrence and by nothing else — but a strike *changes the run that
    /// produces the consultations*, so which operation occurrence `k` names
    /// moves with it, and so does how many consultations there are at all.
    /// [`Class::PeerGone`] is the case that makes the point without an
    /// experiment: `Device::service` returns immediately once the device has
    /// reset, so the device is consulted exactly once past `after` and raising
    /// `after` removes nothing — it relocates the single strike onto a different
    /// operation. A minimiser that raised it, still saw a failure, and concluded
    /// the earlier strikes were not required would have lost the bug and kept a
    /// green-looking reproduction.
    ///
    /// The one handle that is monotone is dropping a whole [`Injection`] from a
    /// plan, and that one is asserted:
    /// [`tests::arming_a_class_that_never_fires_leaves_every_scenario_exactly_as_it_was`].
    pub after: u32,
    /// Once armed, one consultation in this many strikes. Unit: consultations;
    /// one is every consultation and zero is never.
    ///
    /// Zero is spelled out as an off switch rather than refused, exactly as
    /// `Scenario::lose_one_in` spells it out: a scenario that arms a class and
    /// turns it off is a scenario mid-edit, and refusing it would turn that edit
    /// into a compile error instead of a run that plainly says nothing happened.
    ///
    /// Also not a subset operation, and for a plainer reason than [`Self::after`]
    /// is not: the test is `draw(…) % one_in == 0`, so a different modulus
    /// selects a *different* set of occurrences rather than a smaller one.
    /// Raising it can drop the very strike that produced the failure.
    pub one_in: u32,
}

/// What a run injects, and the count of what it did.
///
/// One of these per run. It holds no clock and no generator: the instant comes
/// from the caller, because the clock belongs to the timeline, and the draw is a
/// pure function of the seed, the label and the occurrence — which is the
/// property the module documentation defends and the reason there is no state
/// here beyond the counters.
#[derive(Clone, Debug)]
pub struct Injector {
    seed: u64,
    plan: &'static [Injection],
    /// Consultations per class. A `BTreeMap` for RFC 0004's reason, and not a
    /// fixed table: this crate has a heap, so the sixteen-site limit
    /// `env/src/sim.rs` lives under — and reports overflow against, because it
    /// must — does not apply here.
    counts: BTreeMap<&'static str, u64>,
    struck: u32,
}

impl Injector {
    /// An injector that breaks nothing, which is what every scenario shipped
    /// before this file had.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { seed, plan: &[], counts: BTreeMap::new(), struck: 0 }
    }

    /// Arm the classes a scenario states.
    pub fn arm(&mut self, plan: &'static [Injection]) {
        self.plan = plan;
    }

    /// How many faults this run has injected. Unit: faults.
    ///
    /// Reported beside a failure so that a seed's severity is visible, which is
    /// what `f_env::sim::SimEnv::injected` is for on the other side of the tree.
    #[must_use]
    pub const fn struck(&self) -> u32 {
        self.struck
    }

    /// How many times a class has been consulted. Unit: consultations.
    ///
    /// The number a strike is written into the trace with, so a report can say
    /// *the fourth time `partial` was asked* — the name that survives a commit
    /// adding a class somewhere else, which an ordinal does not.
    #[must_use]
    pub fn consulted(&self, class: Class) -> u64 {
        *self.counts.get(class.label()).unwrap_or(&0)
    }

    /// Consult one class. `None` is the ordinary answer.
    ///
    /// A class this run did not arm answers `None` **without advancing
    /// anything**, so a model may consult every class on every path and a run
    /// that arms none is byte for byte the run it was before the consultation
    /// was written. That is what
    /// [`tests::arming_a_class_that_never_fires_leaves_every_scenario_exactly_as_it_was`]
    /// checks against the shipped table rather than against a fixture.
    pub fn strike(&mut self, class: Class) -> Option<Fault> {
        let armed = *self.plan.iter().find(|injection| injection.class == class)?;
        let site = class.label();
        let occurrence = self.consulted(class);
        self.counts.insert(site, occurrence.wrapping_add(1));

        if occurrence < u64::from(armed.after) || armed.one_in == 0 {
            return None;
        }
        // Modulo bias, on the argument `decide::Decisions::decide` makes: the
        // goal is a reproducible adversarial answer rather than a uniform one,
        // and the bias is bounded by `one_in / 2^64`.
        if !draw(self.seed, domain::FAULTS, site, occurrence)
            .is_multiple_of(u64::from(armed.one_in))
        {
            return None;
        }
        self.struck = self.struck.saturating_add(1);
        Some(class.fault(self.seed, occurrence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::Protocol;
    use crate::proto::wrote;
    use crate::scenario::{Peer, find};
    use crate::trace::Record;
    use crate::{DEFAULT_SEED, LABEL_WIDTH, Outcome};

    /// The seeds every assertion below is made at.
    ///
    /// Three rather than one, and one of them the tree's default. A response
    /// asserted at a single seed is a response asserted against one
    /// interleaving, and the whole point of the apparatus is that there are
    /// many.
    const SEEDS: [u64; 3] = [DEFAULT_SEED, 7, 0x5EED_5EED_5EED_5EED];

    fn run(name: &str, seed: u64) -> Outcome {
        find(name)
            .unwrap_or_else(|| panic!("`{name}` is not a scenario"))
            .run(seed)
            .unwrap_or_else(|why| panic!("`{name}` at {seed:#018x}: {}", why.message()))
    }

    /// Every record one actor wrote under one label.
    fn records<'o>(outcome: &'o Outcome, actor: &str, kind: &str) -> Vec<&'o Record> {
        outcome
            .trace
            .records()
            .iter()
            .filter(|record| record.actor == actor && record.kind == kind)
            .collect()
    }

    /// How many faults of one class this run injected. Unit: strikes.
    fn strikes(outcome: &Outcome, class: Class) -> usize {
        records(outcome, ACTOR, class.label()).len()
    }

    /// What the client wrote under one label. Unit: records.
    fn client(outcome: &Outcome, kind: &str) -> usize {
        records(outcome, crate::client::App::NAME, kind).len()
    }

    /// Every completion the client was told about, as `(token, what it said)`,
    /// sorted.
    ///
    /// The comparison both latency classes make against a clean baseline, and it
    /// is this rather than a count of completions on purpose: *the same number
    /// of operations finished* and *the same operations finished, saying the
    /// same things* are different claims, and only the second one is what
    /// `nothing else moved` means. Two tests asserted that sentence and only one
    /// of them meant it, which is how the helper came to be lifted out.
    fn told(outcome: &Outcome) -> Vec<(u64, u64)> {
        let mut seen: Vec<(u64, u64)> = records(outcome, crate::client::App::NAME, wrote::DONE)
            .iter()
            .map(|record| (record.token, record.detail))
            .collect();
        seen.sort_unstable();
        seen
    }

    /// Operations the scenario's clients issue between them. Unit: operations.
    fn issued(name: &str) -> usize {
        let scenario = find(name).expect("a scenario this file names");
        (scenario.clients * scenario.operations) as usize
    }

    /// The detail a client writes for a refusal the device answered with its own
    /// status byte still unwritten.
    ///
    /// `App::reap` records `(domain << 16) | code`, and `Blk::harvest` packs the
    /// status byte it read as the code. `0xFF` is `STATUS_NONE`, which no device
    /// defines and the driver writes itself — so this number is exactly *the
    /// device answered nothing into this byte*.
    const NOTHING_WRITTEN: u64 = ((f_abi::error::DEVICE as u64) << 16) | 0xFF;

    /// The client's own number, out of a token `App::token` minted.
    const fn who(token: u64) -> u32 {
        (token >> 32) as u32
    }

    // ------------------------------------------------------------- the machinery

    #[test]
    fn every_class_label_fits_the_trace_column_and_no_two_share_a_word() {
        // A label wider than the column shifts every field after it, and the
        // value that shifts it is by definition the one nobody tested with. Two
        // classes with one spelling is two failures a reader of a trace cannot
        // tell apart — and an assertion about one of them would pass for the
        // other, which is the failure that matters in this file.
        let mut seen = vec![ACTOR];
        for class in Class::ALL {
            assert!(
                class.label().len() <= LABEL_WIDTH,
                "`{}` is {} bytes and the column is {LABEL_WIDTH}",
                class.label(),
                class.label().len()
            );
            seen.push(class.label());
        }
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "two labels share a spelling");
    }

    #[test]
    fn a_class_draws_on_its_own_stream_and_on_nothing_elses() {
        // The property `env/src/sim.rs` spends four paragraphs defending, held
        // for classes: a seed that reproduced a bug must keep reproducing it
        // when a class is consulted somewhere the failing run never goes.
        const ALONE: &[Injection] = &[Injection { class: Class::Partial, after: 0, one_in: 2 }];
        const CROWDED: &[Injection] = &[
            Injection { class: Class::Partial, after: 0, one_in: 2 },
            Injection { class: Class::Alloc, after: 0, one_in: 2 },
            Injection { class: Class::Doorbell, after: 0, one_in: 3 },
        ];

        let mut alone = Injector::new(0x99);
        alone.arm(ALONE);
        let expected: Vec<Option<Fault>> = (0..64).map(|_| alone.strike(Class::Partial)).collect();

        let mut crowded = Injector::new(0x99);
        crowded.arm(CROWDED);
        let got: Vec<Option<Fault>> = (0..64)
            .map(|_| {
                // Two other classes consulted between every draw, exactly as a
                // later commit adding sites elsewhere would do.
                let _ = crowded.strike(Class::Alloc);
                let _ = crowded.strike(Class::Doorbell);
                crowded.strike(Class::Partial)
            })
            .collect();

        assert_eq!(expected, got, "another class's traffic moved this class's trajectory");
    }

    #[test]
    fn a_class_nobody_armed_costs_nothing_at_all() {
        // Consulting is free, which is what lets a model ask every class on
        // every path. If an unarmed consultation advanced a counter, arming a
        // class later would find it at a different occurrence than the seed that
        // recorded the failure did.
        let mut quiet = Injector::new(1);
        for _ in 0..100 {
            assert!(quiet.strike(Class::Alloc).is_none());
        }
        assert_eq!(quiet.struck(), 0);
        assert_eq!(quiet.consulted(Class::Alloc), 0, "an unarmed class kept a count");
    }

    #[test]
    fn arming_a_class_that_never_fires_leaves_every_scenario_exactly_as_it_was() {
        // The scenario-level form of the independence property, over the shipped
        // table rather than over a fixture — which is the version that would
        // actually catch the regression. A class armed past every consultation
        // it will ever get is what *adding* a class looks like to a scenario
        // that does not use it, and a recorded seed has to survive that or the
        // corpus expires silently.
        //
        // Compared as records and decisions rather than as a digest, and the
        // difference is deliberate: the artefact's header names what was
        // *armed*, so the digest of a run with a plan differs from one without
        // even when nothing fired. That is the header doing its job — an
        // artefact produced under injection has to say so, or it gets quoted
        // later as a clean run. What must not move is the run.
        const NEVER: &[Injection] =
            &[Injection { class: Class::MapFault, after: u32::MAX, one_in: 1 }];

        let unmoved = |name: &str, quiet: &Outcome, loaded: &Outcome| {
            assert_eq!(
                quiet.trace.records(),
                loaded.trace.records(),
                "arming a class that cannot fire moved `{name}`"
            );
            assert_eq!(
                quiet.log, loaded.log,
                "arming a class that cannot fire moved `{name}`'s interleaving"
            );
        };

        for scenario in crate::scenario::SCENARIOS {
            if scenario.peer == Peer::Deployment || !scenario.injects.is_empty() {
                continue;
            }
            let mut armed = *scenario;
            armed.injects = NEVER;
            let loaded = armed.run(DEFAULT_SEED).expect("terminates");
            assert_eq!(loaded.injected, 0, "a class armed past every consultation fired");
            unmoved(scenario.name, &scenario.run(DEFAULT_SEED).expect("terminates"), &loaded);
        }

        // And the case the loop above cannot make: a scenario that already
        // injects, gaining a *second* class that cannot fire. That is what the
        // eighth class will look like to `partial` on the day somebody adds it,
        // and it is the shape in which the property is easiest to lose — a
        // second class sharing a counter with the first would move exactly here
        // and nowhere else.
        const SHIPPED: Injection = Injection { class: Class::Partial, after: 1, one_in: 2 };
        const AND_ANOTHER: &[Injection] =
            &[SHIPPED, Injection { class: Class::MapFault, after: u32::MAX, one_in: 1 }];

        let partial = *find("partial").expect("the scenario");
        assert_eq!(partial.injects, [SHIPPED], "the `partial` scenario's plan moved");
        let mut crowded = partial;
        crowded.injects = AND_ANOTHER;
        unmoved(
            "partial",
            &partial.run(DEFAULT_SEED).expect("terminates"),
            &crowded.run(DEFAULT_SEED).expect("terminates"),
        );
    }

    #[test]
    fn every_class_has_a_scenario() {
        // The exit criterion's first half, checked rather than counted by hand.
        // A class with no scenario is a branch in a model that nothing ever
        // takes, which is worse than an absent class because it looks like
        // coverage.
        for class in Class::ALL {
            assert!(
                crate::scenario::SCENARIOS
                    .iter()
                    .any(|scenario| scenario.injects.iter().any(|i| i.class == *class)),
                "no scenario arms `{}`",
                class.label()
            );
        }
    }

    #[test]
    fn every_armed_scenario_actually_strikes_and_writes_it_down() {
        // A scenario that arms a class and never reaches it is a scenario whose
        // assertion holds for the wrong reason. Checked at every seed the tests
        // below use, because a class that fires at one seed and not another
        // would make those assertions conditional without saying so.
        for scenario in crate::scenario::SCENARIOS.iter().filter(|s| !s.injects.is_empty()) {
            for seed in SEEDS {
                let outcome = run(scenario.name, seed);
                for injection in scenario.injects {
                    assert!(
                        strikes(&outcome, injection.class) > 0,
                        "`{}` at {seed:#018x} armed `{}` and it never fired",
                        scenario.name,
                        injection.class.label()
                    );
                }
            }
        }
    }

    #[test]
    fn a_class_a_protocol_does_not_read_is_never_consulted_there() {
        // The third shape of an unexercised site, and the one the two tests
        // above would not catch between them: a class armed against a device
        // that ignores it. `Class::Partial` reaches a protocol only through
        // `Bus::writes_land`, and only `blk` asks — a network interface writes
        // nothing back into control memory, which is a protocol fact rather than
        // an omission. Were the consultation ungated, arming `Partial` against
        // `net` would strike, write the strike into the hashed artefact, and
        // change nothing whatever about the run: a site consulted and not
        // exercised, passing for coverage, which is exactly the row
        // `docs/test-taxonomy.md` calls *a fault-injection site that is never
        // exercised*.
        //
        // `Protocol::HONOURS` gates it, so such a scenario strikes zero times
        // and `every_armed_scenario_actually_strikes_and_writes_it_down` turns
        // it red. This test is what makes that gate load-bearing rather than
        // decorative.
        const AIMED_WRONG: &[Injection] =
            &[Injection { class: Class::Partial, after: 0, one_in: 1 }];

        let mut misarmed = *find("net").expect("the scenario");
        assert!(
            !<crate::net::Net as Protocol>::HONOURS.contains(&Class::Partial),
            "`net` now reads this class, so this test is asserting nothing"
        );
        misarmed.injects = AIMED_WRONG;
        assert_eq!(
            misarmed.run(DEFAULT_SEED).expect("terminates").injected,
            0,
            "a device struck a class its protocol never reads"
        );

        // And the identical plan against the device that does read it fires, so
        // the zero above is the gate rather than a scenario that never reached
        // the site at all — which is the way this test would otherwise go green
        // for the wrong reason.
        let mut aimed = *find("partial").expect("the scenario");
        assert!(<crate::blk::Blk as Protocol>::HONOURS.contains(&Class::Partial));
        aimed.injects = AIMED_WRONG;
        assert!(
            aimed.run(DEFAULT_SEED).expect("terminates").injected > 0,
            "the same plan against the device that reads the class did nothing"
        );
    }

    #[test]
    fn every_injected_scenario_reproduces_from_its_seed_and_moves_when_the_seed_does() {
        // `E1-P01`'s exit criterion, which a fault class must not break:
        // injection is a function of the seed like everything else, so a run
        // under it is still one artefact per `(seed, commit)` and still a
        // different one at a different seed. `cargo xtask sim` makes the same
        // claim across two processes, over every scenario including these.
        for scenario in crate::scenario::SCENARIOS.iter().filter(|s| !s.injects.is_empty()) {
            let first = scenario.run(DEFAULT_SEED).expect("terminates");
            let second = scenario.run(DEFAULT_SEED).expect("terminates");
            assert_eq!(
                first.trace.text(),
                second.trace.text(),
                "`{}` produced two artefacts from one seed",
                scenario.name
            );
            let moved = (1..=8u64)
                .filter(|step| {
                    scenario.run(DEFAULT_SEED ^ step).expect("terminates").digest()
                        != first.digest()
                })
                .count();
            assert_eq!(moved, 8, "`{}` ignored a seed change", scenario.name);
        }
    }

    // ---------------------------------------------------------------- the seven

    #[test]
    fn allocation_failure_refuses_the_registration_and_the_client_issues_nothing() {
        // **The response.** A component refused the memory it asked for is told
        // so on the ring it asked over, and it does not go on to submit entries
        // naming a set it does not hold — which the peer would refuse one at a
        // time, turning one refusal into a run's worth of them.
        //
        // The refusal is the real one: `Grants::map` declines, so
        // `f_ring::registry::Table::register` never fills a slot, and the
        // completion the client reads is the one the real table built. Nothing
        // here fabricates an error beside the type that would have produced it.
        for seed in SEEDS {
            let outcome = run("alloc", seed);
            assert_eq!(strikes(&outcome, Class::Alloc), 1, "seed {seed:#018x}: not one refusal");
            let hungry = who(records(&outcome, ACTOR, Class::Alloc.label())[0].token);

            // The client that was refused: no set, no work, and it stops.
            assert!(
                records(&outcome, crate::client::App::NAME, wrote::BOUND)
                    .iter()
                    .all(|record| who(record.token) != hungry),
                "seed {seed:#018x}: a client bound a set the frame refused it"
            );
            assert!(
                records(&outcome, crate::client::App::NAME, wrote::ISSUE)
                    .iter()
                    .all(|record| who(record.token) != hungry),
                "seed {seed:#018x}: a client submitted against a set it does not hold"
            );

            // And the other one is untouched. An allocation failure is one
            // component's and not the run's, and this is the assertion that says
            // the refusal was contained rather than merely delivered.
            let clients = find("alloc").expect("the scenario").clients as usize;
            assert_eq!(
                client(&outcome, wrote::BOUND),
                clients - 1,
                "seed {seed:#018x}: the refusal reached a client it was not aimed at"
            );
            assert_eq!(
                client(&outcome, wrote::FINISHED),
                clients,
                "seed {seed:#018x}: a refused client did not stop, which is a hang"
            );
        }
    }

    #[test]
    fn a_translation_fault_is_refused_as_a_device_error_and_never_as_a_transfer() {
        // **The response.** A descriptor the domain does not translate is
        // refused, and the refusal reaches the client as `DEVICE` carrying the
        // status byte the *driver* wrote before it offered the chain. That is
        // the whole of what `STATUS_NONE` exists for: the device answered
        // nothing into that byte, so what comes back is *nothing was written
        // here* rather than *the device reported a failure* — two different
        // claims, and only one of them is true.
        //
        // The client must not retry it. `RESOURCE` is back-pressure and `DEVICE`
        // is not, and a client that retried a translation fault would reissue a
        // request whose address its domain still does not translate, forever.
        for seed in SEEDS {
            let outcome = run("mapfault", seed);
            let faulted = strikes(&outcome, Class::MapFault);

            // The device refused, once per fault, and wrote it down as the thing
            // it is rather than as a request it served.
            assert_eq!(
                records(&outcome, crate::blk::Blk::NAME, wrote::NOREACH).len(),
                faulted,
                "seed {seed:#018x}: a faulted translation was served anyway"
            );

            // The client heard about every one of them, as a device error.
            let told = records(&outcome, crate::client::App::NAME, wrote::REFUSED)
                .iter()
                .filter(|record| record.detail == NOTHING_WRITTEN)
                .count();
            assert_eq!(
                told, faulted,
                "seed {seed:#018x}: {faulted} translation faults reached the client as {told}"
            );

            // Nothing was lost and nothing hung: every operation is accounted
            // for exactly once, as a completion or as a refusal.
            assert_eq!(
                client(&outcome, wrote::DONE) + faulted,
                issued("mapfault"),
                "seed {seed:#018x}: an operation was neither completed nor refused"
            );
            assert_eq!(client(&outcome, wrote::FINISHED), 1);
        }
    }

    #[test]
    fn a_page_fault_costs_latency_and_costs_nothing_else() {
        // **The response.** A translation the unit had to fault in is slow and
        // is not a failure: every operation still completes, no buffer is lost,
        // and the only thing that moved is the clock. This is the class whose
        // whole value is the *negative* half — a model where a delay quietly
        // dropped work would pass a test that only counted completions.
        //
        // The comparison is against the same scenario with nothing armed, which
        // is the only honest baseline. A threshold in nanoseconds would be a
        // number nobody could defend and would have to be a claim.
        let scenario = *find("faultin").expect("the scenario");
        let mut quiet = scenario;
        quiet.injects = &[];

        for seed in SEEDS {
            let injured = scenario.run(seed).expect("terminates");
            let clean = quiet.run(seed).expect("terminates");

            assert_eq!(
                told(&injured).len(),
                issued("faultin"),
                "seed {seed:#018x}: a page fault lost an operation"
            );
            assert_eq!(
                told(&injured),
                told(&clean),
                "seed {seed:#018x}: a page fault changed what completed"
            );
            assert_eq!(
                client(&injured, wrote::REFUSED),
                0,
                "seed {seed:#018x}: a page fault was reported to the client as a refusal"
            );
            assert!(
                injured.finished_ns > clean.finished_ns,
                "seed {seed:#018x}: the faulting run finished at {} and the clean one at {}",
                injured.finished_ns,
                clean.finished_ns
            );
        }
    }

    #[test]
    fn a_peer_that_dies_mid_operation_gives_every_buffer_back_and_says_nothing_after() {
        // **The response.** The device resets, the client is told, and every
        // buffer it had out comes home — RFC 0024 gives it no other way to take
        // one back, so a peer that died quietly would leave its client holding
        // memory it can never touch and never free, which is a hang with a clean
        // trace.
        //
        // And nothing arrives afterwards. A completion after the reset would be
        // an answer about a token the client has already reclaimed, which is the
        // one thing that would make `PeerGone` unsound: two owners of one
        // buffer, and one of them a device.
        for seed in SEEDS {
            let outcome = run("peergone", seed);
            assert_eq!(strikes(&outcome, Class::PeerGone), 1, "seed {seed:#018x}: not one death");

            let reset = records(&outcome, crate::blk::Blk::NAME, wrote::RESET);
            assert_eq!(reset.len(), 1, "seed {seed:#018x}: a peer died and carried on");
            let died = reset[0].at_ns;

            assert!(
                client(&outcome, wrote::RECLAIM) > 0,
                "seed {seed:#018x}: the peer died with nothing outstanding, so this proves nothing"
            );
            assert_eq!(
                client(&outcome, wrote::FINISHED),
                1,
                "seed {seed:#018x}: the client did not stop after its peer died"
            );
            assert!(
                records(&outcome, crate::client::App::NAME, wrote::DONE)
                    .iter()
                    .all(|record| record.at_ns <= died),
                "seed {seed:#018x}: a completion arrived after the peer was gone"
            );
        }
    }

    #[test]
    fn a_torn_doorbell_is_recorded_and_no_operation_is_lost_or_repeated() {
        // **The response.** A doorbell with nothing behind it costs a poll and
        // is written down. It is not an error — on a real ring a spurious
        // wake-up is ordinary, and a model that treated one as a fault would
        // report a bug where the system has a design.
        //
        // What must hold is exactly-once. Publishing an entry and ringing the
        // bell are two stores, and a torn pair must not become a request served
        // twice or a request served never. That is asserted over the *tokens*,
        // because a count alone would pass for a run that served one request
        // twice and dropped another.
        //
        // The other tear — an entry with no bell — is not modelled, and the
        // reason is worth stating rather than leaving as a silence: this model's
        // device takes one entry per doorbell, so a lost bell would be a lost
        // entry rather than a late one, and what that exercises is the model's
        // own shape. A peer that lies about its cursors is `E1-P04`.
        for seed in SEEDS {
            let outcome = run("doorbell", seed);
            let torn = strikes(&outcome, Class::Doorbell);

            // Every extra bell found an empty wire and said so: `detail` is
            // `u64::MAX`, which is what `Device::submit` writes for a doorbell
            // with nothing behind it.
            let empty = records(&outcome, crate::blk::Blk::NAME, crate::proto::kind::SUBMIT)
                .iter()
                .filter(|record| record.detail == u64::MAX)
                .count();
            assert_eq!(empty, torn, "seed {seed:#018x}: {torn} torn bells, {empty} empty polls");

            let mut completed: Vec<u64> = records(&outcome, crate::client::App::NAME, wrote::DONE)
                .iter()
                .map(|record| record.token)
                .collect();
            assert_eq!(
                completed.len(),
                issued("doorbell"),
                "seed {seed:#018x}: an operation was lost behind a torn doorbell"
            );
            completed.sort_unstable();
            let before = completed.len();
            completed.dedup();
            assert_eq!(before, completed.len(), "seed {seed:#018x}: an operation completed twice");
        }
    }

    #[test]
    fn a_partial_write_is_refused_rather_than_reported_as_a_transfer() {
        // **The response.** The device wrote the payload and did not write the
        // status byte, and the driver reads back the `0xFF` it wrote itself. A
        // used length is *not* evidence that bytes moved — `dma.rs` records this
        // emulator reporting a successful completion for a transfer the
        // remapping unit refused — so the driver answers `DEVICE` carrying the
        // status it actually found, and the client counts the operation over
        // rather than retrying it.
        //
        // This is the class it would be easiest to model wrongly. A device that
        // reported a *short used length* instead would exercise nothing:
        // `Blk::harvest` reads the status byte and not the length, deliberately,
        // so a fault aimed at the length would pass with the client unable to
        // tell anything had happened.
        for seed in SEEDS {
            let outcome = run("partial", seed);
            let torn = strikes(&outcome, Class::Partial);

            // The device believes it served them — the label is `served` — and
            // that disagreement between the two ends is the whole class.
            assert!(
                records(&outcome, crate::blk::Blk::NAME, wrote::SERVED).len() >= torn,
                "seed {seed:#018x}: a partial write was never published"
            );

            let refused = records(&outcome, crate::client::App::NAME, wrote::REFUSED)
                .iter()
                .filter(|record| record.detail == NOTHING_WRITTEN)
                .count();
            assert_eq!(
                refused, torn,
                "seed {seed:#018x}: {torn} partial writes reached the client as {refused} refusals"
            );

            // The ownership half of this class is *not* asserted here, and
            // saying so is better than an assertion that reads as though it
            // were. `App::reap` stamps a `done` with `u64::MAX` when the first
            // byte of a returned buffer is not the one the client wrote — but
            // no device model in this crate ever writes into a client buffer,
            // because `Protocol::serve` is handed descriptors and a `Reach`, and
            // this crate has no type that turns a `Reach` into bytes. That
            // absence is deliberate and is the same one `f_ring::registry::Reach`
            // is built around. So the check would hold in every record of every
            // scenario, injected or not, and an assertion that cannot fail is
            // indistinguishable from one nobody wrote. What actually forbids a
            // device from touching a buffer it was not lent is
            // `f_ring::buffers`' types and `E1-B01`'s remapping unit, asserted
            // where those live.
            //
            // What *is* asserted here is the accounting: every operation is a
            // completion or a refusal, exactly once, and the client stopped.
            assert_eq!(
                client(&outcome, wrote::DONE) + torn,
                issued("partial"),
                "seed {seed:#018x}: an operation was neither completed nor refused"
            );
            assert_eq!(client(&outcome, wrote::FINISHED), 1);
        }
    }

    #[test]
    fn a_delayed_completion_arrives_late_and_arrives_whole() {
        // **The response.** A completion the device finished and held is still a
        // completion: it arrives, it carries the same answer, and the only thing
        // the client can observe is that it took longer. That is the sentence
        // `E1-P06`'s exit will need — *no client observes anything except added
        // latency* — asserted here at the one place in this epoch where there is
        // a model to assert it against.
        //
        // Held after the device published rather than before it served, which is
        // what separates this class from `faultin`: one delays the work and one
        // delays the news.
        let scenario = *find("latecqe").expect("the scenario");
        let mut quiet = scenario;
        quiet.injects = &[];

        for seed in SEEDS {
            let injured = scenario.run(seed).expect("terminates");
            let clean = quiet.run(seed).expect("terminates");

            assert_eq!(
                told(&injured).len(),
                issued("latecqe"),
                "seed {seed:#018x}: a held completion was never delivered"
            );
            assert_eq!(
                told(&injured),
                told(&clean),
                "seed {seed:#018x}: holding a completion changed what it said"
            );
            assert_eq!(
                client(&injured, wrote::REFUSED),
                0,
                "seed {seed:#018x}: a held completion was reported as a refusal"
            );
            assert!(
                injured.finished_ns > clean.finished_ns,
                "seed {seed:#018x}: holding every completion did not move the clock"
            );
        }
    }
}
