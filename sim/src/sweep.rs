// SPDX-License-Identifier: Apache-2.0 OR MIT
//! N seeds across M scenarios, and the shrink that turns a failure into a
//! command.
//!
//! # The one property everything here is built around
//!
//! **Which trials are run, in what order, and what verdict each gets is a
//! function of this module's arguments and nothing else.** Not of how many cores
//! the machine has, not of which worker finished first, not of the clock. Two
//! machines running one sweep of one commit find the same failures, minimise
//! them to the same trials, and print the same bytes — otherwise a sweep is a
//! machine's opinion rather than a commit's property, and the `(seed, commit)`
//! pair that every other check in this tree rests on would stop meaning anything
//! the moment a sweep quoted it.
//!
//! Two things a sweep legitimately wants are therefore kept strictly outside the
//! result:
//!
//! - **Parallelism** is [`Sweep::run`]'s `jobs` argument. It partitions a grid
//!   that was already laid out, each worker writes into slots nobody else can
//!   see, and the report is assembled in grid order rather than in completion
//!   order. [`tests::the_number_of_workers_cannot_reach_a_report`] runs the same
//!   sweep at one worker and at five and requires one report.
//! - **Wall-clock time** is not in this crate at all, and that is enforced
//!   rather than intended: `cargo xtask lint-determinism` scans `sim/` with no
//!   allow-list entry, so a host clock read here would fail the build. The
//!   elapsed cost of a sweep is measured by `cargo xtask sweep`, which is
//!   tooling and is allowed one, and it is printed beside the report rather
//!   than inside it.
//!
//! # A sweep is bounded by memory, and it refuses rather than discovers that
//!
//! Every trial leaks. `client.rs` hands each client a buffer region at
//! `'static` and the region is never handed back, which is sound where it was
//! written — *a component's buffer region is granted for the life of the
//! component, and a simulated component's life is the run* — and stops being
//! bounded the moment a run stops being a process. `--sweep` runs a million runs
//! in one process, so a bound that used to be *the process exits* has to become
//! a number.
//!
//! It is a number here rather than a hope, and it is computed from the shipped
//! table rather than measured on one machine: [`Trial::leak_bytes`] is
//! `clients x (buffer_bytes x BUFFERS + PER_CLIENT_OVERHEAD)`, [`Sweep::leak_bytes`]
//! sums it over the grid, and [`Sweep::over_budget`] refuses a grid whose total
//! passes [`LEAK_BUDGET`]. **R04, and the failure it is fail-closed against is
//! specific**: a nightly job that is killed for memory produces a red cross and
//! a truncated report, which is a nightly reporting *nothing* in the shape of a
//! nightly reporting a bug. Refusing to start, with the largest grid this
//! process will accept printed beside the refusal, is the same night's work
//! split into shards that finish.
//!
//! [`max_seeds`] is what a caller shards on, and `cargo xtask sweep` asks for it
//! with `f-sim --ceiling` rather than keeping a second copy of the arithmetic.
//! Sharding does not change *which* seeds are tried: [`Sweep::span`] takes a
//! half-open range of the same global derivation, so shard `k` runs exactly the
//! trials the unsharded sweep would have run at those indices, and a finding's
//! reported seed index is its index in the whole sweep.
//!
//! What this does **not** do is fix the leak, and saying so is the point of
//! writing the bound down: the fix is for `client.rs` to stop needing
//! `&'static mut`, which is a lifetime change through `Actor` and `World` that
//! safe Rust cannot fake — there is no way to recycle a `&'static mut` without
//! `unsafe`, and `sim/` inherits `forbid`. RFC 0042 records the choice and names
//! what would reverse it.
//!
//! # Why minimisation has to preserve a *signature* and not just a failure
//!
//! `fault.rs` spends two paragraphs warning that neither knob of an
//! [`Injection`] is a subset operation: raising `after` relocates the strike
//! onto a different operation rather than removing earlier ones, and raising
//! `one_in` selects a *different* set of occurrences rather than a smaller one.
//! Its warning names the exact failure — *a minimiser that raised it, still saw
//! a failure, and concluded the earlier strikes were not required would have
//! lost the bug and kept a green-looking reproduction.*
//!
//! So a candidate is accepted only when it fails **with the same signature**,
//! which is the first check in [`crate::check::CHECKS`] that fires and nothing
//! else. That is the strongest test available here and it is not a proof: a
//! candidate could trip the same check for a different reason, and this module
//! would call it the same bug. What the output promises is therefore exactly
//! what delta debugging promises and no more — *a smaller trial that fails the
//! same check* — and the promise is written into the report rather than left to
//! be assumed. RFC 0040.
//!
//! # What a minimum means here
//!
//! [`minimise`] stops when a whole pass over [`MOVES`] accepts nothing: the
//! result is **1-minimal** with respect to that table, ordered by [`Size`]. It
//! is not globally minimal and no delta-debugging minimiser is. The table is
//! short enough that removing each element in turn is exactly what ddmin's final
//! pass would do, so the partitioning phase is left out rather than written and
//! never exercised — a fault plan in this tree has at most seven entries and
//! every shipped one has at most one.
//!
//! The budget is a stated number rather than a hope: [`MINIMISE_BUDGET`]
//! candidates, after which the result carries `exhausted` and the report says
//! so. A minimiser that ran until it was done would be a nightly job with no
//! bound on it.

use std::collections::BTreeMap;

use f_env::split;

use crate::check::{Verdict, examine};
use crate::client::BUFFERS;
use crate::deploy::Deployment;
use crate::fault::{Class, Injection};
use crate::scenario::{SCENARIOS, Scenario};

/// How many candidate runs one minimisation may cost. Unit: runs.
///
/// Generous against what the moves below need — a fault plan of one entry over
/// five fields converges in tens — and bounded because this runs unattended. A
/// minimisation that hits it reports the smallest trial it reached and says the
/// budget was spent, which is a worse answer honestly labelled rather than a
/// better one nobody waited for.
pub const MINIMISE_BUDGET: u32 = 512;

/// How many seeds a sweep runs when it is not told. Unit: seeds.
///
/// Small enough that `cargo xtask sweep` is a thing somebody runs while
/// thinking; the nightly job passes a much larger number. A default nobody can
/// afford to run is a default that means the command has one setting.
pub const DEFAULT_SEEDS: u32 = 64;

/// What one client leaves behind beyond its buffer region. Unit: bytes.
///
/// The `BufferSet` itself, its `Fixed` naming and the allocator's bookkeeping
/// for two allocations. Deliberately generous — the measured excess over
/// `clients x buffer_bytes x BUFFERS` on the four-core development container is
/// about a quarter of this — because the number this feeds is a *refusal*
/// threshold, and a refusal threshold that under-counts is a refusal that does
/// not happen.
const PER_CLIENT_OVERHEAD: usize = 512;

/// What one process may leak before it refuses to sweep. Unit: bytes.
///
/// One gibibyte. Chosen against the smallest machine this is expected to run on
/// rather than against the largest: the nightly job runs on a GitHub-hosted
/// runner, which is the 7 GB class, and a sweep that took a sixth of it leaves
/// room for the toolchain, the page cache and the runner's own agent. The
/// four-core development container has 15.5 GiB and would tolerate six times
/// this, which is exactly why the number is not read from the machine — a bound
/// that changed with the host would make *this grid is too large* a statement
/// about a laptop.
///
/// Raising it is a decision about what a night may cost and is not free:
/// `xtask` shards on it, so a larger budget is fewer, longer processes.
pub const LEAK_BUDGET: u64 = 1 << 30;

/// The largest `seeds` a single process will accept over the first `scenarios`
/// scenarios of the shipped table. Unit: seeds.
///
/// Never zero: a ceiling of zero would be a command that cannot be run at all,
/// and one seed of the widest scenario in the table is under a megabyte.
#[must_use]
pub fn max_seeds(scenarios: usize) -> u32 {
    let per_seed: u64 = SCENARIOS
        .iter()
        .take(scenarios.min(SCENARIOS.len()))
        .map(|scenario| leak_of(scenario.clients, scenario.buffer_bytes))
        .sum();
    if per_seed == 0 {
        return u32::MAX;
    }
    u32::try_from((LEAK_BUDGET / per_seed).max(1)).unwrap_or(u32::MAX)
}

/// What one run of one scenario leaves behind. Unit: bytes.
///
/// `buffer_bytes` is clamped to one the way `App::new` clamps it, because a
/// scenario asking for zero-byte buffers still gets a region of [`BUFFERS`]
/// bytes and an accounting that said zero would under-count exactly the
/// scenarios that look cheapest.
fn leak_of(clients: u32, buffer_bytes: u32) -> u64 {
    let region = (buffer_bytes.max(1) as usize).saturating_mul(BUFFERS);
    u64::from(clients).saturating_mul((region + PER_CLIENT_OVERHEAD) as u64)
}

/// One run of one scenario, with the fields a minimiser is allowed to move.
///
/// The base scenario is named rather than copied so that a trial is always *some
/// shipped scenario, narrowed* — a trial that could invent a scenario would be a
/// reproduction command naming a run nobody can find in the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trial {
    /// The shipped scenario this narrows. Unit: none — a name in [`SCENARIOS`].
    pub scenario: &'static str,
    /// The seed. Unit: none — half of the `(seed, commit)` pair.
    pub seed: u64,
    /// How many clients submit. Unit: clients.
    pub clients: u32,
    /// How many operations each client keeps outstanding. Unit: operations.
    pub window: u32,
    /// How many operations each client issues. Unit: operations.
    pub operations: u32,
    /// What this trial breaks, and how often. Unit: see [`Injection`].
    pub injects: &'static [Injection],
}

impl Trial {
    /// The whole of a shipped scenario, at one seed, with nothing narrowed.
    #[must_use]
    pub const fn of(scenario: &'static Scenario, seed: u64) -> Self {
        Self {
            scenario: scenario.name,
            seed,
            clients: scenario.clients,
            window: scenario.window,
            operations: scenario.operations,
            injects: scenario.injects,
        }
    }

    /// The shipped scenario this trial narrows.
    ///
    /// # Panics
    ///
    /// Never in practice: a `Trial` is only ever built from a member of
    /// [`SCENARIOS`], and the name is carried rather than reconstructed. The
    /// expectation is here rather than an `Option` because a trial naming a
    /// scenario that does not exist is a bug in this file and not a condition a
    /// caller can do anything about.
    #[must_use]
    pub fn base(&self) -> &'static Scenario {
        crate::scenario::find(self.scenario).expect("a trial names a shipped scenario")
    }

    /// This trial as a scenario the simulator can run.
    #[must_use]
    pub fn narrowed(&self) -> Scenario {
        let mut scenario = *self.base();
        scenario.clients = self.clients;
        scenario.window = self.window;
        scenario.operations = self.operations;
        scenario.injects = self.injects;
        scenario
    }

    /// Run it.
    ///
    /// # Errors
    ///
    /// [`crate::Trouble`], which [`examine`] turns into a finding of its own
    /// rather than into a panic.
    pub fn run(&self, deployment: &Deployment) -> Result<crate::Outcome, crate::Trouble> {
        self.narrowed().run_on(self.seed, deployment)
    }

    /// The verdict this trial earns.
    #[must_use]
    pub fn verdict(&self, deployment: &Deployment) -> Verdict {
        examine(&self.run(deployment))
    }

    /// Does this trial run the scenario the table ships, unnarrowed?
    #[must_use]
    pub fn is_whole(&self) -> bool {
        let base = self.base();
        self.clients == base.clients
            && self.window == base.window
            && self.operations == base.operations
            && self.injects == base.injects
    }

    /// The arguments that replay this trial, as one line a stranger can paste.
    ///
    /// Only what moved. A trial nothing narrowed prints `--seed <n> <scenario>`,
    /// which is the reproduction RFC 0039 says a failing seed must be — a
    /// scenario's name and a seed, with the fault plan in the table where the
    /// compiler checks it.
    ///
    /// A **narrowed** trial carries its narrowing on the line, and that is not
    /// the shape RFC 0039 refused. What it refused was a scenario whose plan
    /// lived on the command line, because then a seed alone would be an
    /// incomplete bug report; this line names every field it changed, so it is
    /// complete by construction and the shipped table is untouched.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let base = self.base();
        let mut out = vec!["--seed".to_string(), format!("{:#018x}", self.seed)];
        let mut field = |flag: &str, value: u32, was: u32| {
            if value != was {
                out.push(flag.to_string());
                out.push(value.to_string());
            }
        };
        field("--clients", self.clients, base.clients);
        field("--window", self.window, base.window);
        field("--ops", self.operations, base.operations);
        if self.injects != base.injects {
            // Cleared first and then restated, whatever the base holds, so the
            // line says the whole plan rather than a difference a reader has to
            // apply. A reproduction that has to be diffed against a table is one
            // that stops working when the table moves.
            out.push("--no-inject".to_string());
            for injection in self.injects {
                out.push("--inject".to_string());
                out.push(format!(
                    "{}:{}:{}",
                    injection.class.label(),
                    injection.after,
                    injection.one_in
                ));
            }
        }
        out.push(self.scenario.to_string());
        out
    }

    /// The one line, spelled as a command.
    ///
    /// `--check` and not `--trace`, and the difference is the whole of what a
    /// reproduction command is for. `--trace` prints the artefact and exits
    /// zero, so a stranger who pastes it is handed seventy lines and left to
    /// find the failure in them — which is the triage E1-P03's exit forbids, one
    /// step removed. `--check` runs the same trial, names the property that
    /// broke, prints the evidence and exits non-zero, so the command *judges
    /// itself* and a shell can too. The artefact is one word away and the report
    /// says which word.
    #[must_use]
    pub fn command(&self) -> String {
        format!("cargo run -q -p f-sim -- --check {}", self.argv().join(" "))
    }

    /// What this trial leaves behind at `'static` when it is run. Unit: bytes.
    ///
    /// The module documentation is where the bound this feeds is argued. Read
    /// off the trial rather than off the shipped scenario, because a minimiser
    /// that halved a client count halved this too.
    #[must_use]
    pub fn leak_bytes(&self) -> u64 {
        leak_of(self.clients, self.base().buffer_bytes)
    }
}

/// How big a trial is, for the minimiser's purposes.
///
/// Ordered, and the order is the whole specification of what *smaller* means
/// here. Lexicographic and in this sequence on purpose:
///
/// 1. **armed classes**, because a bug that needs no injection at all is a
///    different and much more interesting bug than one that needs three;
/// 2. **strikes actually made**, which is what shrinks when `one_in` rises — the
///    observed count out of the run rather than the rate that was armed, because
///    the rate is a knob and the count is what happened;
/// 3. **operations**, the scenario's length;
/// 4. **clients**, 5. **window**, 6. the sum of the `after` offsets, which is
///    *how late injection begins*.
///
/// A tuple rather than a scalar so that no weighting has to be invented: a
/// weighting would be a judgement about which axis matters, and the ordering
/// above is that judgement stated where it can be read.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Size {
    /// Classes armed. Unit: injections.
    pub armed: usize,
    /// Faults the run actually injected. Unit: strikes.
    pub strikes: u32,
    /// Operations each client issues. Unit: operations.
    pub operations: u32,
    /// Clients. Unit: clients.
    pub clients: u32,
    /// Operations each client keeps outstanding. Unit: operations.
    pub window: u32,
    /// The sum of every injection's `after`. Unit: consultations.
    pub late: u64,
}

impl Size {
    /// The size of a trial, given what the run it produced injected.
    #[must_use]
    pub fn of(trial: &Trial, strikes: u32) -> Self {
        Self {
            armed: trial.injects.len(),
            strikes,
            operations: trial.operations,
            clients: trial.clients,
            window: trial.window,
            late: trial.injects.iter().map(|i| u64::from(i.after)).sum(),
        }
    }

    /// The line a report states the minimum in.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{} class(es) armed, {} strike(s), {} operation(s), {} client(s), window {}",
            self.armed, self.strikes, self.operations, self.clients, self.window
        )
    }
}

/// One way to make a trial smaller.
///
/// A named function rather than a closure so that a report can say which move
/// was still available when a budget ran out, and so that
/// [`tests::every_move_shrinks_something`] can hold the table against the
/// definition of [`Size`] — a move that made a trial *larger* would turn the
/// fixpoint below into a loop.
pub struct Move {
    /// What it is called, in a report. Unit: none.
    pub name: &'static str,
    /// Every candidate this move offers against one trial, largest step first.
    ///
    /// Largest step first so that the greedy pass takes the biggest accepted
    /// step rather than crawling: halving an operation count converges in
    /// logarithmically many accepted candidates where decrementing converges in
    /// linearly many, and a nightly minimiser pays for the difference.
    pub candidates: fn(&Trial) -> Vec<Trial>,
}

/// Every way a trial may be made smaller.
///
/// Ordered to match [`Size`], so the first move tried is the one that shrinks
/// the axis the ordering cares about most. Nothing here raises `after` and
/// nothing lowers `one_in`: `fault.rs` states that neither direction is a subset
/// operation, and the two directions that *do* reduce [`Size`] — earlier
/// injection and fewer strikes — are the two the table offers.
pub const MOVES: &[Move] = &[
    Move { name: "drop-class", candidates: drop_class },
    Move { name: "fewer-strikes", candidates: fewer_strikes },
    Move { name: "shorter", candidates: shorter },
    Move { name: "fewer-clients", candidates: fewer_clients },
    Move { name: "narrower", candidates: narrower },
    Move { name: "earlier", candidates: earlier },
];

/// A fault plan, at `'static`, because that is what a world is armed with.
///
/// The leak is bounded and stated rather than hidden: a minimisation costs at
/// most [`MINIMISE_BUDGET`] candidates, each plan is at most seven `Injection`s
/// of a few bytes, and the process running it exits. `client.rs` leaks a buffer
/// region for a related reason — a component's memory is granted for the life of
/// the component, and a simulated component's life is the run — and this is the
/// same shape one level up: a plan is granted for the life of the trial, and the
/// trial's life is the process.
///
/// The alternative was widening [`crate::World::arm`] to take an owned plan,
/// which would put an allocation on the path of every run in order to serve the
/// minimiser, and the minimiser is the rarer caller by four orders of magnitude.
fn plan(injections: Vec<Injection>) -> &'static [Injection] {
    Box::leak(injections.into_boxed_slice())
}

/// Remove one armed class.
///
/// The one handle `fault.rs` says *is* monotone: a class that is not armed is
/// consulted and answers nothing without advancing anything, so dropping one
/// leaves every other class's draws exactly where they were.
fn drop_class(trial: &Trial) -> Vec<Trial> {
    (0..trial.injects.len())
        .map(|at| {
            let mut rest = trial.injects.to_vec();
            rest.remove(at);
            Trial { injects: plan(rest), ..*trial }
        })
        .collect()
}

/// Strike less often, by raising one class's rate.
///
/// Raising `one_in` selects a *different* set of occurrences rather than a
/// smaller one, which `fault.rs` is explicit about. That is why the candidate is
/// offered rather than assumed: it is accepted only if the same check still
/// fires, and rejected otherwise — which is the whole of the guard against the
/// failure that file warns of.
fn fewer_strikes(trial: &Trial) -> Vec<Trial> {
    let mut out = Vec::new();
    for (at, injection) in trial.injects.iter().enumerate() {
        if injection.one_in == 0 {
            continue;
        }
        for rate in [16u32, 8, 4, 3, 2] {
            if rate <= injection.one_in {
                continue;
            }
            let mut rest = trial.injects.to_vec();
            rest[at] = Injection { one_in: rate, ..*injection };
            out.push(Trial { injects: plan(rest), ..*trial });
        }
    }
    out
}

/// Issue fewer operations, halving first.
fn shorter(trial: &Trial) -> Vec<Trial> {
    steps(trial.operations).map(|operations| Trial { operations, ..*trial }).collect()
}

/// Run fewer clients.
fn fewer_clients(trial: &Trial) -> Vec<Trial> {
    steps(trial.clients).map(|clients| Trial { clients, ..*trial }).collect()
}

/// Keep fewer operations outstanding.
fn narrower(trial: &Trial) -> Vec<Trial> {
    steps(trial.window).map(|window| Trial { window, ..*trial }).collect()
}

/// Begin injecting earlier.
///
/// The direction `fault.rs` permits: lowering `after` cannot relocate a strike
/// onto work that has not happened, because it only ever un-skips consultations
/// the class was already going to have.
fn earlier(trial: &Trial) -> Vec<Trial> {
    let mut out = Vec::new();
    for (at, injection) in trial.injects.iter().enumerate() {
        for after in steps(injection.after.saturating_add(1)).map(|n| n.saturating_sub(1)) {
            let mut rest = trial.injects.to_vec();
            rest[at] = Injection { after, ..*injection };
            out.push(Trial { injects: plan(rest), ..*trial });
        }
    }
    out
}

/// The values below `from` worth trying, biggest step first: halves, then one
/// less, down to one.
///
/// Deduplicated and never zero, because a scenario with no clients, no window or
/// no operations is not a smaller version of the run — it is a different run
/// with nothing in it, and a run with nothing in it produces a short trace and a
/// perfectly stable digest, which `Trouble::NeedsDeployment` already names as the
/// one result a check must never report as a pass.
fn steps(from: u32) -> impl Iterator<Item = u32> {
    let mut seen: Vec<u32> = Vec::new();
    let mut at = from;
    while at > 1 {
        at /= 2;
        seen.push(at.max(1));
    }
    if from > 1 {
        seen.push(from - 1);
    }
    seen.sort_unstable();
    seen.dedup();
    seen.into_iter().rev().filter(move |value| *value >= 1 && *value < from)
}

/// A failure, shrunk.
#[derive(Clone, Debug)]
pub struct Minimal {
    /// The smallest trial found that fails the same check.
    pub trial: Trial,
    /// Its size, in the terms [`Size`] states.
    pub size: Size,
    /// Candidate runs spent. Unit: runs.
    pub spent: u32,
    /// Did the budget run out before a whole pass accepted nothing?
    ///
    /// Reported rather than hidden: a minimum that was not reached is a
    /// different answer from one that was, and the difference is exactly whether
    /// the word *minimal* in the report is true.
    pub exhausted: bool,
    /// Did the minimal trial produce the same artefact on a second run?
    ///
    /// The minimiser's own negative control. A failure that does not reproduce
    /// is not a smaller bug report, it is a broken one, and the report says so
    /// rather than printing a command that works one time in two.
    pub stable: bool,
}

/// Shrink a failing trial while it keeps failing the same check.
///
/// Greedy over [`MOVES`] to a fixpoint: each pass tries every move's candidates
/// in order, takes the first that fails with `signature` **and is smaller by
/// [`Size`]**, and starts again. When a whole pass accepts nothing the trial is
/// 1-minimal with respect to the table.
///
/// # Determinism
///
/// The candidate order is fixed, nothing is drawn, nothing is timed, and the
/// deployment is the same one the sweep ran. Two calls on one failure therefore
/// answer the same trial, and
/// [`tests::minimising_twice_answers_the_same_trial`] is what says so.
#[must_use]
pub fn minimise(failing: &Trial, signature: &str, deployment: &Deployment) -> Minimal {
    let mut best = *failing;
    let mut strikes = strikes_of(&best, deployment);
    let mut spent = 0u32;
    let exhausted = loop {
        let mut moved = false;
        for step in MOVES {
            for candidate in (step.candidates)(&best) {
                if spent >= MINIMISE_BUDGET {
                    break;
                }
                spent += 1;
                let result = candidate.run(deployment);
                if examine(&result).signature() != Some(signature) {
                    continue;
                }
                let injected = result.map_or(0, |outcome| outcome.injected);
                if Size::of(&candidate, injected) >= Size::of(&best, strikes) {
                    // A move that did not make the trial smaller by the stated
                    // ordering is not a shrink, whatever it changed. Refusing it
                    // is what makes this loop terminate rather than oscillate
                    // between two trials that each fail the same check.
                    continue;
                }
                best = candidate;
                strikes = injected;
                moved = true;
                break;
            }
            if moved || spent >= MINIMISE_BUDGET {
                break;
            }
        }
        if spent >= MINIMISE_BUDGET {
            break true;
        }
        if !moved {
            break false;
        }
    };

    let first = best.run(deployment);
    let second = best.run(deployment);
    let stable = match (&first, &second) {
        (Ok(a), Ok(b)) => a.trace.text() == b.trace.text(),
        (Err(a), Err(b)) => a == b,
        _ => false,
    };
    Minimal { trial: best, size: Size::of(&best, strikes), spent, exhausted, stable }
}

/// How many faults a trial's run injected. Unit: strikes.
fn strikes_of(trial: &Trial, deployment: &Deployment) -> u32 {
    trial.run(deployment).map_or(0, |outcome| outcome.injected)
}

/// One distinct bug, as a sweep reports it.
#[derive(Clone, Debug)]
pub struct Found {
    /// The check that fired. Unit: none — the signature.
    pub signature: &'static str,
    /// The property that was supposed to hold.
    pub what: &'static str,
    /// The scenario it was found in.
    pub scenario: &'static str,
    /// The first seed, in grid order, that produced it.
    pub seed: u64,
    /// That seed's index in the sweep. Unit: seeds, zero-based.
    pub at: u32,
    /// How many trials in this sweep produced this signature. Unit: trials.
    pub occurrences: u32,
    /// One line of evidence out of the first failing run.
    pub evidence: String,
    /// The shrink.
    pub minimal: Minimal,
}

/// What one sweep produced.
#[derive(Clone, Debug)]
pub struct Report {
    /// Trials run. Unit: trials.
    pub trials: u32,
    /// Per scenario, in table order: the name, trials run, trials that failed.
    pub tally: Vec<(&'static str, u32, u32)>,
    /// One per distinct `(scenario, signature)`, in grid order.
    pub found: Vec<Found>,
}

impl Report {
    /// Did the sweep find anything?
    ///
    /// A vacuous report is not clean and this does not say so: [`Report::vacuous`]
    /// is the separate question, because *nothing failed* and *nothing ran* are
    /// different facts and a caller that conflated them would be the failure
    /// below.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.found.is_empty()
    }

    /// Did the grid collapse to nothing?
    ///
    /// R04, and the argument [`steps`] already makes one level down: *a run with
    /// nothing in it produces a short trace and a perfectly stable digest*, which
    /// is the one result a check must never report as a pass. A grid with no
    /// trials in it is the same mistake one level up — `--seeds 0` and
    /// `--scenarios 0` each produce one — and the answer is the same, a refusal
    /// rather than a green line. `f-sim` refuses the arguments that would build
    /// one; this is what catches a grid that collapsed for a reason nobody has
    /// thought of yet.
    #[must_use]
    pub const fn vacuous(&self) -> bool {
        self.trials == 0
    }

    /// How many distinct checks fired across the whole sweep. Unit: signatures.
    ///
    /// Not the same number as `found.len()`, and the difference is worth
    /// printing: findings are kept per `(scenario, signature)` because each one
    /// reproduces separately, but several scenarios tripping one check is
    /// usually one bug seen from several angles. Saying both numbers is what
    /// stops a reader counting thirteen bugs where there is one.
    #[must_use]
    pub fn signatures(&self) -> usize {
        let mut seen: Vec<&str> = self.found.iter().map(|f| f.signature).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    }
}

/// A grid of scenarios and seeds, and the sweep over it.
pub struct Sweep {
    scenarios: Vec<&'static Scenario>,
    seeds: Vec<u64>,
    /// Where this sweep's first seed sits in the whole derivation. Unit: seeds,
    /// zero-based.
    ///
    /// Zero for an unsharded sweep. It is carried rather than recomputed so that
    /// a finding's reported index is its index in the *sweep somebody asked
    /// for*, not in the shard that happened to run it — a shard boundary is a
    /// fact about a machine and must not reach a report.
    from: u32,
}

impl Sweep {
    /// The first `scenarios` scenarios of [`SCENARIOS`], at `seeds` seeds
    /// derived from `base`.
    ///
    /// The seeds are `base` itself and then `split::derive(base, i)`, which is
    /// the derivation every other stream in this crate is built from — so a
    /// sweep's seed set is reproducible from two numbers rather than from a
    /// file, and the tree's own `DEFAULT_SEED` is the first trial of the default
    /// sweep rather than a value the sweep happens to miss.
    #[must_use]
    pub fn new(base: u64, seeds: u32, scenarios: usize) -> Self {
        Self::span(base, 0, seeds, scenarios)
    }

    /// The same grid, restricted to seed indices `[from, from + seeds)`.
    ///
    /// A shard. The seed at index `i` is the seed the unsharded sweep would have
    /// run at index `i` and no other, which is what makes sharding a decision
    /// about processes rather than about coverage: six shards of eleven thousand
    /// seeds try exactly the sixty-six thousand seeds one process would have
    /// tried, in the same order, and each reports the indices it ran.
    #[must_use]
    pub fn span(base: u64, from: u32, seeds: u32, scenarios: usize) -> Self {
        let take = scenarios.min(SCENARIOS.len());
        let first = u64::from(from);
        Self {
            scenarios: SCENARIOS.iter().take(take).collect(),
            seeds: (first..first.saturating_add(u64::from(seeds)))
                .map(|i| if i == 0 { base } else { split::derive(base, i) })
                .collect(),
            from,
        }
    }

    /// One scenario, by name, at `seeds` seeds derived from `base`.
    #[must_use]
    pub fn just(scenario: &'static Scenario, base: u64, seeds: u32) -> Self {
        let mut sweep = Self::new(base, seeds, 0);
        sweep.scenarios = vec![scenario];
        sweep
    }

    /// What this whole grid will leave behind at `'static`. Unit: bytes.
    #[must_use]
    pub fn leak_bytes(&self) -> u64 {
        let per_seed: u64 = self
            .scenarios
            .iter()
            .map(|scenario| leak_of(scenario.clients, scenario.buffer_bytes))
            .sum();
        per_seed.saturating_mul(self.seeds.len() as u64)
    }

    /// Is this grid larger than one process may hold?
    ///
    /// Asked before a trial runs, and the answer is a refusal rather than a
    /// warning: see the module documentation for what an out-of-memory nightly
    /// looks like to the person it reaches.
    #[must_use]
    pub fn over_budget(&self) -> bool {
        self.leak_bytes() > LEAK_BUDGET
    }

    /// How many trials this grid holds. Unit: trials.
    #[must_use]
    pub fn size(&self) -> usize {
        self.scenarios.len() * self.seeds.len()
    }

    /// Every trial, in grid order: scenario outermost, seed innermost.
    #[must_use]
    pub fn trials(&self) -> Vec<Trial> {
        self.scenarios
            .iter()
            .flat_map(|scenario| self.seeds.iter().map(move |seed| Trial::of(scenario, *seed)))
            .collect()
    }

    /// Run every trial, then minimise the first failure of each distinct
    /// `(scenario, signature)`.
    ///
    /// `jobs` is how many threads share the grid and is a cost knob only: the
    /// grid is laid out first, each worker fills slots nobody else touches, and
    /// everything below reads the slots in grid order.
    #[must_use]
    pub fn run(&self, jobs: usize, deployment: &Deployment) -> Report {
        let trials = self.trials();
        let mut verdicts: Vec<Verdict> = vec![Verdict::Clean; trials.len()];
        let chunk = trials.len().div_ceil(jobs.max(1)).max(1);

        std::thread::scope(|scope| {
            for (slots, work) in verdicts.chunks_mut(chunk).zip(trials.chunks(chunk)) {
                scope.spawn(move || {
                    for (slot, trial) in slots.iter_mut().zip(work) {
                        *slot = trial.verdict(deployment);
                    }
                });
            }
        });

        let mut tally: Vec<(&'static str, u32, u32)> = Vec::new();
        // Keyed by `(scenario, signature)` in a `BTreeMap` for RFC 0004's
        // reason, and read back below in grid order rather than in key order —
        // the map is here to group, not to order.
        let mut first: BTreeMap<(&'static str, &'static str), (u32, u32, Trial, String)> =
            BTreeMap::new();
        let per = self.seeds.len().max(1);

        for (index, (trial, verdict)) in trials.iter().zip(&verdicts).enumerate() {
            let at = u32::try_from(index % per).unwrap_or(u32::MAX).saturating_add(self.from);
            match tally.last_mut() {
                Some((name, total, _)) if *name == trial.scenario => *total += 1,
                _ => tally.push((trial.scenario, 1, 0)),
            }
            let Verdict::Failed(finding) = verdict else { continue };
            if let Some((_, _, failures)) = tally.last_mut() {
                *failures += 1;
            }
            first
                .entry((trial.scenario, finding.check))
                .and_modify(|(_, seen, _, _)| *seen += 1)
                .or_insert((at, 1, *trial, finding.evidence.clone()));
        }

        let mut found: Vec<Found> = first
            .into_iter()
            .map(|((scenario, signature), (at, occurrences, trial, evidence))| Found {
                signature,
                what: what(signature),
                scenario,
                seed: trial.seed,
                at,
                occurrences,
                evidence,
                minimal: minimise(&trial, signature, deployment),
            })
            .collect();
        // **Smallest first**, and that ordering is the exit criterion's *no human
        // triage* made mechanical: a reader of a nightly report should be able
        // to act on the first entry, and the first entry should be the tightest
        // reproduction the sweep has. Grid order is the tie-break — the scenario
        // as the table lists it, then the seed the signature was first seen at —
        // so the sort stays total and stays a function of the arguments.
        //
        // A report ordered by the map's keys would be ordered alphabetically by
        // signature, which is stable and says nothing about where to look.
        found.sort_by_key(|f| (f.minimal.size, position(f.scenario), f.at, f.signature));

        Report { trials: u32::try_from(trials.len()).unwrap_or(u32::MAX), tally, found }
    }
}

/// Where a scenario sits in the shipped table. Unit: none — an index.
fn position(name: &str) -> usize {
    SCENARIOS.iter().position(|scenario| scenario.name == name).unwrap_or(usize::MAX)
}

/// The property one signature stands for.
fn what(signature: &str) -> &'static str {
    crate::check::CHECKS
        .iter()
        .find(|check| check.name == signature)
        .map_or("the run did not finish", |check| check.what)
}

/// A class by the label it draws at, for the `--inject` flag.
///
/// Fail closed: a misspelled class is refused rather than dropped, because a
/// replay that silently armed nothing would print a green result for a trial
/// nobody ran.
#[must_use]
pub fn class(label: &str) -> Option<Class> {
    Class::ALL.iter().copied().find(|class| class.label() == label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_SEED;
    use crate::check::Finding;
    use crate::scenario::find;

    /// Scenarios a test can sweep with no build behind them: everything up to
    /// the deployment scenario, which needs component files.
    fn without_deployment() -> usize {
        SCENARIOS.iter().position(|s| s.needs_components()).unwrap_or(SCENARIOS.len())
    }

    /// A failing trial, manufactured without a deliberate defect compiled in.
    ///
    /// The deployment scenario over an empty component set refuses, and
    /// `examine` gives that refusal a signature of its own. So the minimiser has
    /// a real failure to shrink on every ordinary build — which matters, because
    /// a shrink exercised only under `cargo xtask sweep --mutate` is a shrink
    /// that `cargo test` never runs.
    fn failing() -> (Trial, &'static str) {
        let scenario = SCENARIOS.iter().find(|s| s.needs_components()).expect("a deployment");
        (Trial::of(scenario, DEFAULT_SEED), "deployment")
    }

    #[test]
    fn the_number_of_workers_cannot_reach_a_report() {
        // The property the whole module is arranged around. If this ever fails,
        // a sweep has become a statement about a machine rather than about a
        // commit, and every `(seed, commit)` pair a sweep prints is worthless.
        let sweep = Sweep::new(DEFAULT_SEED, 8, SCENARIOS.len());
        let deployment = Deployment::default();
        let one = sweep.run(1, &deployment);
        let many = sweep.run(5, &deployment);
        assert_eq!(one.trials, many.trials);
        assert_eq!(one.tally, many.tally);
        assert_eq!(one.found.len(), many.found.len());
        for (a, b) in one.found.iter().zip(&many.found) {
            assert_eq!((a.signature, a.scenario, a.seed), (b.signature, b.scenario, b.seed));
            assert_eq!((a.at, a.occurrences), (b.at, b.occurrences));
            assert_eq!(a.evidence, b.evidence);
            assert_eq!(a.minimal.trial, b.minimal.trial);
            assert_eq!((a.minimal.size, a.minimal.spent), (b.minimal.size, b.minimal.spent));
        }
    }

    #[test]
    fn the_seed_set_is_a_function_of_its_two_arguments() {
        let a = Sweep::new(7, 32, 3).trials();
        assert_eq!(a, Sweep::new(7, 32, 3).trials());
        assert_ne!(Sweep::new(8, 32, 3).trials(), a, "the base seed changed nothing");
        assert_eq!(a[0].seed, 7, "the base seed is not the first trial of the sweep");
        let seeds: std::collections::BTreeSet<u64> = a.iter().map(|t| t.seed).collect();
        assert_eq!(seeds.len(), 32, "a sweep of 32 seeds ran fewer than 32 distinct ones");
    }

    #[test]
    fn a_clean_tree_sweeps_clean() {
        // The other half of the mutation harness, in the unit suite: the sweep
        // has to be quiet on a tree with nothing wrong with it, or every nightly
        // run is a false alarm and the job is turned off within a month.
        let scenarios = without_deployment();
        let report = Sweep::new(DEFAULT_SEED, 12, scenarios).run(2, &Deployment::default());
        assert!(
            report.clean(),
            "a sweep of the shipped scenarios found {:?}",
            report.found.iter().map(|f| (f.scenario, f.signature)).collect::<Vec<_>>()
        );
        assert_eq!(report.trials, 12 * u32::try_from(scenarios).expect("a small table"));
    }

    #[test]
    fn a_sweep_that_finds_something_names_it_once_and_minimises_it() {
        // The deployment scenario with no components refuses at every seed, so
        // a sweep including it finds one signature many times and reports it
        // once — which is the grouping the exit criterion's *no human triage*
        // rests on.
        let deployment = Deployment::default();
        let scenario = SCENARIOS.iter().find(|s| s.needs_components()).expect("a deployment");
        let report = Sweep::just(scenario, DEFAULT_SEED, 6).run(2, &deployment);
        assert_eq!(report.found.len(), 1, "one signature was reported as several findings");
        let found = &report.found[0];
        assert_eq!(found.signature, "deployment");
        assert_eq!(found.occurrences, 6);
        assert_eq!(found.at, 0, "the smallest seed index in the grid is not the one reported");
        assert!(found.minimal.stable, "a failure that does not reproduce was called minimal");
        assert!(!found.minimal.exhausted, "the budget ran out on a trivial shrink");
    }

    #[test]
    fn minimising_twice_answers_the_same_trial() {
        // Required by the exit criterion in as many words: running the minimiser
        // twice on one failure has to give the same answer, or the reproduction
        // command in a nightly report is one of several the sweep might have
        // printed.
        let deployment = Deployment::default();
        let (trial, signature) = failing();
        let first = minimise(&trial, signature, &deployment);
        let second = minimise(&trial, signature, &deployment);
        assert_eq!(first.trial, second.trial);
        assert_eq!(first.size, second.size);
        assert_eq!(first.spent, second.spent);
    }

    #[test]
    fn a_minimum_is_one_minimal_against_the_table() {
        // What the word *minimal* in a report is allowed to mean: no single move
        // in `MOVES` produces a smaller trial that still fails the same check.
        // Checked by trying every one of them against the answer rather than by
        // trusting the fixpoint that produced it.
        let deployment = Deployment::default();
        let (trial, signature) = failing();
        let minimal = minimise(&trial, signature, &deployment);
        assert!(!minimal.exhausted, "the budget ran out, so this is not a minimum");
        for step in MOVES {
            for candidate in (step.candidates)(&minimal.trial) {
                let result = candidate.run(&deployment);
                if examine(&result).signature() != Some(signature) {
                    continue;
                }
                let injected = result.map_or(0, |o| o.injected);
                assert!(
                    Size::of(&candidate, injected) >= minimal.size,
                    "`{}` still had a smaller candidate",
                    step.name
                );
            }
        }
    }

    #[test]
    fn every_move_shrinks_something() {
        // A move that made a trial larger by the stated ordering would turn the
        // fixpoint into a loop, and a move that produced no candidate at all
        // would be a knob in the table that nothing turns — the same failure
        // `scenario.rs`'s `every_field_changes_the_run` exists to keep out one
        // level down.
        let scenario = find("mapfault").expect("a shipped scenario with a plan and room to shrink");
        let trial = Trial { clients: 4, window: 4, operations: 12, ..Trial::of(scenario, 1) };
        for step in MOVES {
            let candidates = (step.candidates)(&trial);
            assert!(!candidates.is_empty(), "`{}` offered no candidate", step.name);
            for candidate in candidates {
                assert!(
                    Size::of(&candidate, 0) < Size::of(&trial, 1),
                    "`{}` offered a candidate that is not smaller",
                    step.name
                );
            }
        }
    }

    #[test]
    fn an_empty_grid_is_not_a_pass() {
        // The fail-open R04 exists to stop: `--seeds 0` and `--scenarios 0` each
        // collapse the grid, and a report that answered `clean` to a sweep that
        // ran nothing would be a green result standing for no property at all.
        // Held on the report rather than only at the command line, because the
        // command line is one of the ways a grid can end up empty and not the
        // only one.
        for sweep in [Sweep::new(DEFAULT_SEED, 0, 4), Sweep::new(DEFAULT_SEED, 4, 0)] {
            assert_eq!(sweep.size(), 0);
            let report = sweep.run(2, &Deployment::default());
            assert!(report.vacuous(), "a grid with no trials in it did not say so");
            assert!(report.clean(), "an empty grid found something");
        }
        let ran = Sweep::new(DEFAULT_SEED, 2, 2).run(2, &Deployment::default());
        assert!(!ran.vacuous(), "a grid with trials in it was called empty");
    }

    #[test]
    fn a_shard_runs_the_trials_the_whole_sweep_would_have() {
        // The property sharding rests on. A shard is a range of the *same*
        // derivation, so six shards cover exactly what one process would have
        // covered — otherwise splitting a nightly for memory would quietly
        // change which seeds a commit has been swept at, and the seed index a
        // finding reports would mean something different in every shard.
        let whole = Sweep::new(DEFAULT_SEED, 12, 3).trials();
        let mut sharded = Sweep::span(DEFAULT_SEED, 0, 5, 3).trials();
        sharded.extend(Sweep::span(DEFAULT_SEED, 5, 7, 3).trials());
        // Grid order is scenario-outermost, so the concatenation of two shards
        // is not the whole sweep's order — the sets are what has to be equal.
        let mut left: Vec<(&str, u64)> = whole.iter().map(|t| (t.scenario, t.seed)).collect();
        let mut right: Vec<(&str, u64)> = sharded.iter().map(|t| (t.scenario, t.seed)).collect();
        left.sort_unstable();
        right.sort_unstable();
        assert_eq!(left, right, "a shard boundary changed which seeds are tried");
    }

    #[test]
    fn a_grid_states_what_it_will_leak_and_refuses_to_exceed_it() {
        // The bound that replaced *the process exits*. Checked in both
        // directions, because a budget only ever refuses grids that are too
        // large and a test that only saw it accept would pass on a budget of
        // infinity.
        let scenarios = SCENARIOS.len();
        let ceiling = max_seeds(scenarios);
        assert!(ceiling > 0, "the ceiling refuses every grid");
        assert!(
            !Sweep::new(DEFAULT_SEED, ceiling, scenarios).over_budget(),
            "the largest grid the ceiling allows is over budget"
        );
        assert!(
            Sweep::new(DEFAULT_SEED, ceiling.saturating_add(1), scenarios).over_budget(),
            "one seed past the ceiling is still accepted, so the ceiling is not one"
        );
        // And the arithmetic is about the run rather than about the table: a
        // trial with half the clients leaks half as much, which is what makes a
        // minimised trial cheap to replay.
        let scenario = find("blk").expect("a shipped scenario with clients");
        let whole = Trial::of(scenario, DEFAULT_SEED);
        let half = Trial { clients: 1, ..whole };
        assert!(whole.leak_bytes() > half.leak_bytes(), "leak accounting ignores the trial");
        assert!(half.leak_bytes() > 0, "a client that binds a region leaks nothing");
    }

    #[test]
    fn a_replayed_trial_says_the_whole_of_itself() {
        // The reproduction command is the deliverable, so what it carries is
        // asserted rather than eyeballed: a whole scenario prints a seed and a
        // name, and a narrowed one prints every field it moved.
        let scenario = find("partial").expect("a shipped scenario with a plan");
        let whole = Trial::of(scenario, 0x1234);
        assert!(whole.is_whole());
        assert_eq!(whole.argv(), vec!["--seed", "0x0000000000001234", "partial"]);

        let narrowed = Trial { clients: 1, operations: 2, injects: &[], ..whole };
        assert!(!narrowed.is_whole());
        let line = narrowed.argv().join(" ");
        assert!(line.contains("--ops 2"), "{line}");
        assert!(line.contains("--no-inject"), "{line}");
        assert!(line.ends_with(" partial"), "{line}");
        // `--check` and not `--trace`: the printed line has to judge itself, or
        // a stranger who pastes it reads an artefact instead of a verdict.
        assert!(narrowed.command().starts_with("cargo run -q -p f-sim -- --check "));
    }

    #[test]
    fn a_finding_names_the_property_it_broke() {
        // A report that printed a check's name and not its sentence would be a
        // report somebody has to look things up to read, which is the triage the
        // exit criterion forbids.
        for check in crate::check::CHECKS {
            assert_eq!(what(check.name), check.what);
        }
        assert_eq!(what("budget"), "the run did not finish");
    }

    #[test]
    fn a_verdict_carries_its_evidence() {
        let finding = Finding { check: "held", what: "x", evidence: "at 1 ns".to_string() };
        assert_eq!(Verdict::Failed(finding).signature(), Some("held"));
        assert!(!Verdict::Clean.failed());
        assert!(!Size::of(&failing().0, 0).line().is_empty());
    }

    #[test]
    fn a_class_is_named_by_the_label_it_draws_at() {
        for known in Class::ALL {
            assert_eq!(class(known.label()), Some(*known));
        }
        assert_eq!(class("nosuchclass"), None, "an unknown class was accepted");
    }
}
