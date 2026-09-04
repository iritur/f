// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The oracle: properties that must hold in **every** scenario at **every**
//! seed, so that a defect nobody wrote a test for is still caught by one.
//!
//! # Why a sweep needs a different kind of assertion
//!
//! [`crate::fault`] states a response per class and asserts it — *this
//! registration is refused with this code*, *this buffer comes home*. That is
//! the right shape for a fault class, because a class is a thing somebody
//! decided to inject and therefore a thing somebody can decide the answer to.
//! It is the wrong shape for a sweep. A sweep's whole premise is that the bug is
//! **not known in advance**: `E1-P03`'s exit is *an injected bug is found*, and
//! an injected bug that had to be named by an assertion written for it would be
//! found by the assertion rather than by the sweep.
//!
//! So the checks here name no scenario, no class, no device and no defect. Each
//! one is a sentence about the *artefact* — a property of any run of anything in
//! this crate — and the sweep's finding is the first of them that fails. RFC
//! 0040 argues the split and states what it costs.
//!
//! # What a check may read, and what it may not
//!
//! Only [`Outcome`] and [`Trouble`]: the trace's records, and the refusal a run
//! ended with. Never the model's internals, because a check that reached inside
//! a device would hold for the devices that exist and would have to be rewritten
//! for the next one — and the whole value of an oracle is that it is already
//! true about code nobody has written yet.
//!
//! Every check is therefore expressed over `app` records and their tokens, which
//! is the one vocabulary every peer in this crate shares
//! ([`crate::proto::wrote`]).
//!
//! **What that costs, said here rather than discovered later: a defect that
//! never reaches a client is invisible to this oracle.** A device that completed
//! in a different order, refused for a different reason, or wrote a different
//! `used_len` — while still answering every token exactly once, intact, and
//! leaving no client hanging — passes all five checks. That class is covered by
//! the digest (`cargo xtask sim` requires a scenario to reproduce and to move
//! when the seed moves) and by the seven per-class assertions in
//! [`crate::fault`], and it is not covered here. The three layers catch
//! different things and none of them subsumes another; RFC 0040 records the
//! division.
//!
//! # Which of these can actually fire, said as a number rather than assumed
//!
//! *A check that has never failed is indistinguishable from a check that
//! cannot*, and the distinction has two halves. The predicate has to be able to
//! say no — [`tests::every_check_has_a_run_that_fails_it`] forges a `Record`
//! vector per property and holds that — and the **plumbing** has to be walked,
//! because a check that reads a label `client.rs` has stopped writing is a check
//! that returns `None` forever while every test in the tree stays green. That
//! second half is [`tests::a_run_of_the_models_writes_what_every_check_reads`],
//! which runs a shipped scenario and requires the records each property reads to
//! be there.
//!
//! Beyond that, three of the five are falsifiable **end to end** by a defect in
//! the shipped source, and two are not. Stated here rather than left to be
//! discovered:
//!
//! - `held` and `bound` are tripped by `mutate-crossed-completion`, and
//!   `balance` by `mutate-silent-reset`, both in `sim/src/dev.rs`. `cargo xtask
//!   sweep --mutate` arms each and requires the sweep to find it.
//! - `intact` **cannot fire on any run of the models as they stand**, and that
//!   is a fact about the models rather than about the check: no device model in
//!   this crate writes into a client's data buffer — `bytes_mut` has two call
//!   sites and both are a client stamping its own pattern before lending. It is
//!   a guard for the model that does, and for the day `InFlight::complete`
//!   stops being the only way a buffer comes home. Leaving it in an unarmed
//!   state is the choice; deleting it would mean the first model to write into
//!   a client's buffer ships with nothing watching the ownership rule RFC 0024
//!   is for.
//! - `clock` is a structural invariant of the timeline rather than a property of
//!   the system, and no defect is arranged for it. What holds it is that it
//!   reads every record of every run, so a discrete-event loop that stopped
//!   being one would trip it in the sweep before anything else did.
//!
//! RFC 0042 records the count and what would change it.
//!
//! # The order is load-bearing
//!
//! A failing run's **signature** is the *first* check in [`CHECKS`] that fails,
//! and nothing else. That is what makes a signature stable while a minimiser
//! shrinks the run underneath it: a shrink that removed the third symptom of one
//! bug would still be the same bug by the first. The list is ordered tightest
//! first — the check that names the smallest, most local thing that went wrong
//! comes before the check that only says the run ended badly — so that the
//! sentence a report leads with is the sentence closest to the defect.
//!
//! Two distinct bugs that trip one check in one scenario are reported as one
//! finding. That is a real cost and it is the price of a signature that survives
//! shrinking; what limits it is that the corpus keeps a *trial* per finding, so
//! two bugs that minimise to two different trials still leave two entries
//! behind.

use std::collections::BTreeMap;

use crate::client::App;
use crate::proto::wrote;
use crate::trace::Record;
use crate::{Outcome, Trouble};

/// The sequence number a registration's token carries, as `client.rs` mints it.
///
/// Repeated here rather than imported because it is `client.rs`'s private
/// business and this module is a *reader* of traces rather than a party to the
/// protocol. [`tests::the_registration_sequence_is_still_the_one_the_client_mints`]
/// is what keeps the two equal, by running a scenario and requiring exactly one
/// excluded token per client rather than by trusting the constant.
const REGISTRATION: u32 = u32::MAX;

/// Is this token an operation's, rather than the registration's?
const fn operation(token: u64) -> bool {
    (token & 0xFFFF_FFFF) as u32 != REGISTRATION
}

/// One property, and what a run that breaks it looks like.
pub struct Check {
    /// What a report calls it. Unit: none — a stable label, and the whole of a
    /// finding's signature.
    pub name: &'static str,
    /// The property, in one line, phrased as what must be true.
    pub what: &'static str,
    /// `None` when the property holds; otherwise one line naming the evidence.
    pub holds: fn(&Outcome) -> Option<String>,
}

/// Every property a run of anything in this crate must have.
///
/// Ordered tightest first — see the module documentation for why the order is
/// part of the contract rather than a listing convenience.
pub const CHECKS: &[Check] = &[
    Check {
        name: "held",
        what: "a client is never told about a token it does not hold",
        holds: held,
    },
    Check {
        name: "intact",
        what: "a buffer that comes back holds the bytes its client put in it",
        holds: intact,
    },
    Check {
        name: "balance",
        what: "each operation is answered exactly as often as it was issued",
        holds: balance,
    },
    Check { name: "bound", what: "every client that registered also finished", holds: bound },
    Check { name: "clock", what: "the clock never goes backwards", holds: clock },
];

/// What a run was found to be.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Nothing to report.
    Clean,
    /// One property failed. The first one, in [`CHECKS`] order.
    Failed(Finding),
}

impl Verdict {
    /// The signature, or `None` for a clean run.
    ///
    /// A finding's whole identity: two failures with one signature are one
    /// finding for the purposes of reporting, minimising and the corpus.
    #[must_use]
    pub fn signature(&self) -> Option<&'static str> {
        match self {
            Self::Clean => None,
            Self::Failed(finding) => Some(finding.check),
        }
    }

    /// Did anything fail?
    #[must_use]
    pub const fn failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// One property, failed, with the evidence that says so.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finding {
    /// Which check. Unit: none — a stable label from [`CHECKS`], and the
    /// signature.
    pub check: &'static str,
    /// The property that was supposed to hold.
    pub what: &'static str,
    /// One line of evidence out of this particular run.
    pub evidence: String,
}

/// Examine one run.
///
/// A [`Trouble`] is a finding too, and it gets a check name of its own rather
/// than being folded into the run: a scenario that did not terminate and a
/// scenario that answered the wrong token are different bugs, and a sweep that
/// called both *failed* would minimise one into the other.
#[must_use]
pub fn examine(result: &Result<Outcome, Trouble>) -> Verdict {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(trouble) => {
            let (check, what) = match trouble {
                Trouble::Budget(_) => ("budget", "a run finishes inside its step budget"),
                Trouble::NoSuchActor(_) => ("actor", "every message names an actor that exists"),
                // Not a bug in the system: a sweep asked for the deployment
                // scenario without building the components first. Reported as a
                // finding rather than swallowed, because the alternative is a
                // sweep that silently covers one scenario fewer than it says it
                // does — and R04 is the rule the rest of this tree is under.
                Trouble::NeedsDeployment => {
                    ("deployment", "a deployment scenario is given a component set")
                }
            };
            return Verdict::Failed(Finding { check, what, evidence: trouble.message() });
        }
    };
    if let Some(carried) = outcome.trace.carried() {
        // **A partial artefact is refused rather than judged.** Every property
        // below reads the whole of a run — which tokens were issued, which
        // clients finished — so a trace that begins part-way through would fail
        // `balance` and `bound` for every operation that was answered before the
        // cut. That is not a bug found, it is the oracle being asked a question
        // it does not answer, and answering it anyway is the *plausible and
        // wrong* result RFC 0043 is written against.
        //
        // A terse snapshot (`f-sim --scan --terse`) is what produces one. It is
        // for re-entering a run cheaply and comparing digests; a run that is to
        // be *judged* is re-entered from a whole mark, which costs more and says
        // everything.
        return Verdict::Failed(Finding {
            check: "partial",
            what: "a run is judged over the whole of its artefact",
            evidence: format!(
                "this artefact begins {} record(s) in, at {} ns — it came from a terse \
                 snapshot. Re-enter from a whole mark to judge the run.",
                carried.records, carried.at_ns
            ),
        });
    }
    for check in CHECKS {
        if let Some(evidence) = (check.holds)(outcome) {
            return Verdict::Failed(Finding { check: check.name, what: check.what, evidence });
        }
    }
    Verdict::Clean
}

/// Every record one actor wrote under one label.
fn wrote<'o>(outcome: &'o Outcome, actor: &str, kind: &str) -> impl Iterator<Item = &'o Record> {
    outcome.trace.records().iter().filter(move |r| r.actor == actor && r.kind == kind)
}

/// A client is never told about a token it does not hold.
///
/// `App::reap` writes `refused` with a detail of `u64::MAX` in exactly one
/// situation: a completion arrived, every in-flight buffer was asked whether the
/// token was theirs, and none of them said yes. Its own comment calls that *a
/// peer with a bug worth seeing*, and this is the check that sees it.
///
/// The registration's token is excluded because `bind` writes the same label at
/// it for a refused registration, which is an answer rather than a mismatch —
/// the `alloc` fault class ships a scenario that does exactly that on purpose.
fn held(outcome: &Outcome) -> Option<String> {
    let stray = wrote(outcome, App::NAME, wrote::REFUSED)
        .find(|r| r.detail == u64::MAX && operation(r.token))?;
    Some(format!(
        "at {} ns client {} was told about token {:#018x}, which it did not hold",
        stray.at_ns, stray.who, stray.token
    ))
}

/// A buffer that comes back holds the bytes its client put in it.
///
/// `App::reap` compares the first byte against the pattern it stamped before
/// lending, and writes `done` with a detail of `u64::MAX` when they differ. That
/// is the ownership rule of RFC 0024 observed rather than assumed: the buffer
/// came back from the same place it went.
fn intact(outcome: &Outcome) -> Option<String> {
    let torn = wrote(outcome, App::NAME, wrote::DONE).find(|r| r.detail == u64::MAX)?;
    Some(format!(
        "at {} ns token {:#018x} came back holding bytes its client did not write",
        torn.at_ns, torn.token
    ))
}

/// Each operation is answered exactly as often as it was issued.
///
/// Two failures in one property, and they are one property rather than two
/// because they are the same arithmetic seen from either side: an answer with no
/// issue behind it is a peer inventing work, and an issue with no answer behind
/// it is a buffer the client can never take back — which RFC 0024 makes
/// unreachable except through `PeerGone`, and `reclaim` is what a run writes
/// when it takes that route.
///
/// A `full` record is deliberately not counted: the ring refused before anything
/// left, the buffer came straight back, and nothing was issued.
fn balance(outcome: &Outcome) -> Option<String> {
    let mut ledger: BTreeMap<u64, i64> = BTreeMap::new();
    for record in outcome.trace.records() {
        if record.actor != App::NAME || !operation(record.token) {
            continue;
        }
        let step = match record.kind {
            wrote::ISSUE => 1,
            wrote::DONE | wrote::REFUSED | wrote::RECLAIM => -1,
            _ => continue,
        };
        let count = ledger.entry(record.token).or_insert(0);
        *count += step;
        if *count < 0 {
            return Some(format!(
                "at {} ns token {:#018x} was answered more often than it was issued",
                record.at_ns, record.token
            ));
        }
    }
    let (token, left) = ledger.iter().find(|(_, count)| **count > 0)?;
    Some(format!("token {token:#018x} was issued {left} time(s) and never answered"))
}

/// Every client that registered also finished.
///
/// One `register` per client and one `finished` per client, so the two counts
/// are equal in any run that ends. A client short of a `finished` is a client
/// still holding buffers when the timeline went idle — a hang, in a simulator
/// where a hang looks like a short trace rather than like a stuck process, which
/// is exactly why it needs a check rather than a stopwatch.
fn bound(outcome: &Outcome) -> Option<String> {
    let registered = wrote(outcome, App::NAME, wrote::REGISTER).count();
    let finished = wrote(outcome, App::NAME, wrote::FINISHED).count();
    if registered == finished {
        return None;
    }
    Some(format!(
        "{registered} client(s) registered and {finished} finished, so {} stopped with work \
         outstanding",
        registered.saturating_sub(finished)
    ))
}

/// The clock never goes backwards.
///
/// The timeline's own invariant, checked against the artefact rather than
/// against the timeline: a run whose records are out of order is one where the
/// discrete-event loop has stopped being one, and every later property is read
/// off a trace that no longer describes a sequence.
fn clock(outcome: &Outcome) -> Option<String> {
    let mut previous = 0;
    for record in outcome.trace.records() {
        if record.at_ns < previous {
            return Some(format!("a record at {} ns follows one at {previous} ns", record.at_ns));
        }
        previous = record.at_ns;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_SEED;
    use crate::scenario::{Peer, SCENARIOS, Scenario, find};
    use crate::trace::Trace;

    /// Every scenario a test can run without a build behind it.
    fn self_contained() -> impl Iterator<Item = &'static Scenario> {
        SCENARIOS.iter().filter(|scenario| scenario.peer != Peer::Deployment)
    }

    #[test]
    fn every_shipped_scenario_is_clean_at_a_spread_of_seeds() {
        // The claim the sweep rests on, made small enough to run in the unit
        // suite: if the oracle fired on the shipped models it would be an oracle
        // about this crate's own bugs rather than about the system's, and every
        // finding a sweep reported would need a human to sort into real and not.
        // That is exactly the triage `E1-P03`'s exit forbids.
        for scenario in self_contained() {
            for step in 0..16u64 {
                let seed = f_env::split::derive(DEFAULT_SEED, step);
                let verdict = examine(&scenario.run(seed));
                assert_eq!(
                    verdict,
                    Verdict::Clean,
                    "`{}` at {seed:#018x} is not clean",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn no_two_checks_share_a_signature() {
        // The signature is a name, so two checks with one name would be two
        // findings a minimiser could shrink into each other.
        let mut names: Vec<&str> = CHECKS.iter().map(|check| check.name).collect();
        names.extend(["budget", "actor", "deployment"]);
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(before, names.len(), "two checks share a signature");
    }

    /// A run built by hand, so that a check can be shown to fail.
    fn forged(records: &[Record]) -> Outcome {
        let mut trace = Trace::new();
        for record in records {
            trace.push(*record);
        }
        Outcome {
            seed: 0,
            steps: 0,
            decisions: 0,
            finished_ns: 0,
            trace,
            log: Vec::new(),
            injected: 0,
        }
    }

    const fn app(at_ns: u64, kind: &'static str, token: u64, detail: u64) -> Record {
        Record { at_ns, who: 1, actor: App::NAME, kind, token, detail }
    }

    /// A token that is an operation's rather than the registration's.
    const OPERATION: u64 = 7;

    /// A token the registration is minted at, for client zero.
    const REGISTERED: u64 = REGISTRATION as u64;

    #[test]
    fn every_check_has_a_run_that_fails_it() {
        // A check that has never failed is indistinguishable from a check that
        // cannot, which is the argument `trace_check` makes at length about a
        // deliberate defect and the reason `sim_check` runs a second seed. Held
        // here per property rather than once for the set: a table of five checks
        // where four can fail is a table with one decoration in it.
        let issue = app(10, wrote::ISSUE, OPERATION, 1);
        let cases: &[(&str, Vec<Record>)] = &[
            ("held", vec![app(10, wrote::REFUSED, OPERATION, u64::MAX)]),
            ("intact", vec![issue, app(20, wrote::DONE, OPERATION, u64::MAX)]),
            ("balance", vec![issue]),
            ("bound", vec![app(0, wrote::REGISTER, REGISTERED, 0)]),
            (
                "clock",
                vec![
                    app(0, wrote::REGISTER, REGISTERED, 0),
                    app(0, wrote::REGISTER, REGISTERED, 0),
                    app(20, wrote::FINISHED, 0, 0),
                    app(10, wrote::FINISHED, 0, 0),
                ],
            ),
        ];
        for (expected, records) in cases {
            let verdict = examine(&Ok(forged(records)));
            assert_eq!(
                verdict.signature(),
                Some(*expected),
                "the run written to fail `{expected}` produced {verdict:?}"
            );
        }
    }

    #[test]
    fn a_run_that_did_not_finish_is_its_own_signature() {
        // Not folded into a property of the trace: a scenario that ran out of
        // budget and a scenario that answered the wrong token are different
        // bugs, and a minimiser told they were one would shrink either into the
        // other.
        assert_eq!(examine(&Err(Trouble::Budget(64))).signature(), Some("budget"));
        assert_eq!(examine(&Err(Trouble::NoSuchActor(3))).signature(), Some("actor"));
        assert_eq!(examine(&Err(Trouble::NeedsDeployment)).signature(), Some("deployment"));
    }

    #[test]
    fn a_run_of_the_models_writes_what_every_check_reads() {
        // The other half of `every_check_has_a_run_that_fails_it`, and the half
        // that was missing. That test forges `Record`s and shows the
        // *predicates* can fire; it never runs `client.rs`, so it would stay
        // green if the client stopped writing the records a check reads — a
        // renamed label, a dropped `finished`, a different sentinel — and four
        // of the five properties would go quietly dead while every test in the
        // tree passed and every sweep printed `clean`.
        //
        // That is the shape of false pass this epoch has already had twice: an
        // assertion that holds while the path it observes is not walked. So the
        // plumbing is checked against a run, the way
        // `the_registration_sequence_is_still_the_one_the_client_mints` checks
        // the constant against a run.
        let scenario = find("blk").expect("a shipped scenario");
        let outcome = scenario.run(DEFAULT_SEED).expect("terminates");

        // `bound` reads these two counts. A run in which neither label is ever
        // written satisfies `registered == finished` at zero and asserts nothing.
        let registered = wrote(&outcome, App::NAME, wrote::REGISTER).count();
        let finished = wrote(&outcome, App::NAME, wrote::FINISHED).count();
        assert_eq!(registered, scenario.clients as usize, "`bound` counts no registrations");
        assert_eq!(finished, registered, "`bound` would have fired on a shipped scenario");

        // `intact` reads `done` records and their detail. Zero of them is a run
        // in which the ownership comparison never happened.
        let done: Vec<&Record> = wrote(&outcome, App::NAME, wrote::DONE).collect();
        assert!(!done.is_empty(), "`intact` has no completion to read");
        assert!(
            done.iter().all(|r| r.detail != u64::MAX),
            "`intact` would have fired on a shipped scenario"
        );

        // `balance` reads the issue/answer arithmetic, so both sides have to be
        // present: a run with issues and no answers, or answers and no issues,
        // is a run this property is not being tested by.
        let issued =
            wrote(&outcome, App::NAME, wrote::ISSUE).filter(|r| operation(r.token)).count();
        assert!(issued > 0, "`balance` has no issue to count");
        assert!(done.iter().any(|r| operation(r.token)), "`balance` has no answer to count");

        // `held` reads `refused` with a detail of `u64::MAX`, and a clean run has
        // none — which is the property. What is asserted here is that the label
        // it filters on is one the client still writes at all, since a check
        // filtering on a label nothing writes is a check that cannot fire.
        assert!(
            crate::proto::wrote::REFUSED != crate::proto::wrote::DONE,
            "`held` and `intact` read one label"
        );

        // `clock` reads every record's timestamp, so it needs a trace with more
        // than one record in it to be a statement about ordering.
        assert!(outcome.trace.records().len() > 1, "`clock` has one record to order");
    }

    #[test]
    fn the_registration_sequence_is_still_the_one_the_client_mints() {
        // `REGISTRATION` above is a copy of a constant `client.rs` keeps
        // private, and a copy that stopped matching would make `held` and
        // `balance` read a registration as an operation — quietly, and in the
        // direction of a false finding. Checked against a run rather than
        // against the constant: exactly one token per client is excluded, and it
        // is the one the client registered at.
        let scenario = find("blk").expect("a shipped scenario");
        let outcome = scenario.run(DEFAULT_SEED).expect("terminates");
        let registrations: Vec<u64> =
            wrote(&outcome, App::NAME, wrote::REGISTER).map(|record| record.token).collect();
        assert_eq!(registrations.len(), 2, "`blk` has two clients");
        for token in registrations {
            assert!(!operation(token), "a registration token was read as an operation's");
        }
    }
}
