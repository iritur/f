// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A reservation under adversarial load, and the two arms that make the first
//! result mean anything.
//!
//! `E1-B07`'s exit is two sentences and the second is the hard one: *an
//! over-subscribed reservation is refused with `ADMISSION`; a granted one meets
//! its deadline under adversarial load.* The first is arithmetic and
//! `f_abi::reserve` refuses it there. The second needs adversarial load to
//! exist, and needs somewhere it can be observed honestly.
//!
//! # Why here and not in a boot
//!
//! Because a deadline met is a *timing*, and under QEMU's translation backend a
//! timing is a property of the emulator. RFC 0038 recorded the number: the same
//! load measured between 18 and 260 timer ticks across runs, because each block
//! of guest code is compiled the first time it is reached and the local APIC's
//! deadline is host time. A boot that reported *the reservation met its
//! deadline* would be reporting that the host was not busy.
//!
//! This crate has a virtual clock. Nothing in it reads a host clock, every draw
//! comes from `f_env::Env`, and a seed reproduces a run byte for byte — so
//! *when* something happened is a **count of slots**, the same number on a fast
//! host and a slow one, and it is the same kind of number `claims/0005` gates on
//! and for the same reason. What this cannot say is how many nanoseconds a slot
//! is on real silicon. That half is `claims/0011`, `pending` on the machine
//! `E0-D10` owes, and it is named rather than smuggled in as an emulated
//! microsecond.
//!
//! # The three arms, and why two of them are controls
//!
//! A granted reservation that meets its deadline proves nothing on its own:
//! a model in which the adversary was never able to reach the reserved core
//! *by construction* would report zero misses while testing nothing. So:
//!
//! - [`Arm::Granted`] — the reservation is admitted, the adversary is placed by
//!   the table, and the property is that every period is met and no slot of the
//!   reserved core was ever taken.
//! - [`Arm::Unreserved`] — **the arm that says the mechanism does something.**
//!   The same component, the same adversary, the same seed, with the component
//!   in the soft class so no reservation stands between it and the load. It
//!   must **miss**. If it does not, the granted arm's zero is a property of the
//!   workload rather than of admission control, and this run is refused.
//! - [`Arm::OverSubscribed`] — the machine's capacity is granted, and then the
//!   same demand is made again. It must be refused in the `ADMISSION` domain
//!   and **nothing must run for it**: the control against a reservation that
//!   was admitted and then missed, which is the failure R08 says the word
//!   *deadline* must not be used for.
//!
//! # What the adversary does
//!
//! Every one of the four is drawn from `f_env::Env` and therefore reproducible
//! from the seed:
//!
//! - **Long non-preemptible stretches.** A worker that has started a stretch
//!   holds its core for the whole of it and cannot be taken off. The *length*
//!   is drawn, up to a bound deliberately longer than the reservation's slack,
//!   so a stretch that lands can be long enough to make a period late on its
//!   own and most are not. A fixed length would be a constant wearing a
//!   random-looking name: every count below would be the same at every seed,
//!   and the reproduction check that compares two seeds would pass on a model
//!   that never read one.
//! - **Bursts timed against the reservation's period.** The adversary asks the
//!   table for a core at every release instant, which is the worst moment for
//!   work to arrive rather than a random one, and starts a stretch on the ones
//!   the draw picks. Aimed, and not blanket: an adversary that occupied every
//!   free core for every whole period would leave nothing for the mid-period
//!   draw to do, and [`STRETCH_ODDS`] would be dead code in a model whose whole
//!   purpose is to vary.
//! - **A component that tries to exceed its own budget.** The job's demand is
//!   drawn over a range that reaches well past what was admitted — past the
//!   period itself — and the server clamps it at the budget and counts the
//!   clamp. The clamp is load-bearing rather than decorative, and
//!   [`without_the_clamp`] is what says so: it runs the same granted arm with
//!   the clamp removed and the reservation **misses**, because a job handed
//!   forty-five slots of work in a forty-slot period is late however
//!   exclusively it owns its core. Said plainly, because it is the subtlest of
//!   the four: on a whole-core reservation the clamp protects the component's
//!   own later periods rather than a neighbour, since RFC 0007's whole-core
//!   rule means there is no neighbour to protect. The day two reservations
//!   share a core — which this design forecloses — it would protect one from
//!   the other.
//! - **Work that competes for the same cores.** The adversary asks the table
//!   for every core on the machine, every period, and is refused the reserved
//!   ones. Those refusals are counted, and a run in which none happened is
//!   refused: an adversary that never tried is not an adversary.
//!
//! # What would make this green while the property was false
//!
//! `f_env::Env` returning a constant. That is the failure this file is built
//! against rather than one it discovered late: [`digest`] hashes what the model
//! *produced* and not the seed it was handed — hashing the seed would make two
//! seeds differ whatever the model did, which is the check
//! `xtask::admission_gate` performs believing it means something — and
//! `the_adversary_varies_with_the_seed` requires the counts to actually move
//! across seeds.

use f_abi::error;
use f_abi::manifest::{HUGE_BYTES, class, domain};
use f_abi::reserve::{Demand, Grant, Machine, Offers, Table};
use f_env::{Env, SeededEnv};

/// How long one slot of the model is.
///
/// A tenth of the frame's 1 kHz tick, so that a tick is ten slots and the
/// schedulability test's *slack holds at least one tick* is a comparison this
/// model can straddle rather than one it always passes. Nothing here converts a
/// slot to a wall-clock number: it is the unit the counts are in, and
/// `claims/0011` is where a nanosecond eventually goes.
/// Unit: nanoseconds.
pub const SLOT_NS: u64 = 100_000;

/// The frame's own timer interval, which is what the CPU half of the
/// schedulability test refuses against.
/// Unit: nanoseconds.
pub const TICK_NS: u64 = 1_000_000;

/// How many periods one run covers.
///
/// Five hundred and twelve, which is enough that a stretch drawn at one in four
/// slots lands on a release instant many times over, and small enough that a
/// sweep can run thousands of seeds. It is a count and it is the same count on
/// every machine.
/// Unit: periods.
pub const PERIODS: u32 = 512;

/// The longest stretch a worker holds a core for, in slots.
///
/// The *bound*, and the length is drawn below it. Longer than the reservation's
/// slack on purpose: a bound below the slack would make the unreserved arm pass
/// by arithmetic rather than by scheduling, and a control that cannot fail is
/// not a control. Not longer than the period, because a stretch that outlasts
/// the period it began in occupies its core for every subsequent one too, and
/// an adversary that never lets go is a machine with fewer cores rather than an
/// adversary.
/// Unit: slots.
pub const MAX_STRETCH_SLOTS: u32 = 40;

/// How often a worker on a free core starts a fresh stretch mid-period, as one
/// in `n` slots.
/// Unit: none — a reciprocal probability.
const STRETCH_ODDS: u64 = 4;

/// How often the release-instant burst lands on a given free core, as one in
/// `n` cores.
///
/// Two rather than one, and the difference is the whole of the blocking review
/// finding this constant exists because of: a burst that took *every* free core
/// for a whole period left the mid-period draw unreachable, so [`STRETCH_ODDS`]
/// was dead code, `stretches` was a multiplication rather than a measurement,
/// and every number the claim gates on was the same at every seed.
/// Unit: none — a reciprocal probability.
const BURST_ODDS: u64 = 2;

/// How far past its budget the job is willing to ask, as a multiple of it.
///
/// Three, so the draw reaches forty-five slots in a forty-slot period: **past
/// the period, not merely past the budget.** That is what makes the clamp a
/// mechanism the model would notice losing rather than a counter beside an
/// unchanged result — see [`without_the_clamp`].
/// Unit: none — a multiplier on the budget.
const OVERRUN_REACH: u32 = 3;

/// The machine the model runs on.
///
/// Eight physical cores in two four-core cache domains, no sibling, no cache or
/// bandwidth partitioning. That is a small part described honestly rather than a
/// generous one described optimistically, and it is the shape that makes the
/// over-subscribed arm reachable: the frame sits in the first domain, so the
/// machine grants exactly one hard-class reservation and refuses the second.
///
/// `threads_per_core = 1` means the sibling clause is recorded
/// `obtained::UNEXERCISED` — RFC 0005 rule 2 — and every run says so, because
/// RFC 0007 says a number collected under a reservation that cannot show all
/// four is not a number about this system.
#[must_use]
pub const fn machine() -> Machine {
    Machine {
        physical_cores: 8,
        threads_per_core: 1,
        cores_per_cache: 4,
        cores_per_bandwidth: 4,
        cache: Offers::Exclusion,
        bandwidth: Offers::Exclusion,
        partitions: 0,
        frame_cores: 1,
        reservable_bytes: 16 * HUGE_BYTES,
        tick_ns: TICK_NS,
    }
}

/// The reservation the model asks for.
///
/// Four milliseconds of period and one and a half of budget: forty slots and
/// fifteen, leaving twenty-five slots of slack — two and a half of the frame's
/// own ticks, so the schedulability test's floor is cleared with room rather
/// than exactly, and a run that fails is failing about scheduling.
#[must_use]
pub const fn demand() -> Demand {
    Demand {
        cores: 1,
        period_ns: 40 * SLOT_NS,
        budget_ns: 15 * SLOT_NS,
        memory_bytes: HUGE_BYTES,
        class: class::HARD,
        domain: domain::SHARED,
    }
}

/// Which of the three a run is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arm {
    /// The reservation is granted and the adversary is kept off it.
    Granted,
    /// The same load with the component in the soft class. It must miss.
    Unreserved,
    /// The machine's capacity is spent and the same demand is made again. It
    /// must be refused, and nothing must run for it.
    OverSubscribed,
}

impl Arm {
    /// The word a report and a command line share.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Unreserved => "unreserved",
            Self::OverSubscribed => "oversubscribed",
        }
    }

    /// Every arm, in the order a report prints them.
    pub const ALL: [Self; 3] = [Self::Granted, Self::Unreserved, Self::OverSubscribed];
}

/// What one arm produced. Every field is a count.
#[derive(Clone, Copy, Debug)]
pub struct Run {
    /// Which arm this was.
    pub arm: Arm,
    /// The seed it was drawn from.
    pub seed: u64,
    /// Whether the demand under test was admitted.
    pub admitted: bool,
    /// The packed refusal, where there was one. `f_abi::error` encoding.
    pub refusal: i32,
    /// Whether every one of RFC 0007's four components ran a mechanism, as the
    /// grant recorded it. False on this machine, and the report says so rather
    /// than letting a green run imply four mechanisms were exercised.
    pub exercised: bool,
    /// The physical core the component ran on.
    pub home: u32,
    /// Cores the grant held idle so that an unpartitionable resource was
    /// exclusive. R12: the cost beside the number. Unit: physical cores.
    pub excluded: u32,
    /// Periods that actually ran. Zero on the over-subscribed arm, and required
    /// to be.
    pub periods: u32,
    /// Periods in which the job received its whole budget before the next
    /// release. Unit: periods.
    pub met: u32,
    /// Periods in which it did not. Unit: periods.
    pub missed: u32,
    /// Slots of the component's own core that an adversary held while the
    /// component still owed work. Zero is the property on the granted arm.
    /// Unit: slots.
    pub stolen: u32,
    /// Placements the table refused because the core was reserved.
    /// Unit: placements.
    pub refused_placements: u32,
    /// Placements it allowed. Unit: placements.
    pub taken_placements: u32,
    /// Non-preemptible stretches the adversary began. Unit: stretches.
    pub stretches: u32,
    /// Stretches begun at a release instant, aimed rather than random.
    /// Unit: stretches.
    pub bursts: u32,
    /// Periods in which the job asked for more than it was admitted for and was
    /// clamped at its budget. Unit: periods.
    pub clamped: u32,
    /// Slots the reserved core spent idle because RFC 0007 forecloses lending
    /// reserved capacity out. *They are meant to be sitting idle*, and this is
    /// how many. Unit: slots.
    pub reserved_idle: u32,
    /// The smallest number of slots left between the job finishing and its next
    /// release, over every period that was met.
    ///
    /// **This is what "met its deadline" is expressed in**, and it is a count
    /// rather than a time on purpose. `u32::MAX` when nothing was met.
    /// Unit: slots.
    pub slack_min: u32,
}

impl Run {
    /// Whether this arm produced what it exists to produce.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold.
    pub fn verdict(&self) -> Result<(), String> {
        match self.arm {
            Arm::OverSubscribed => {
                if self.admitted {
                    return Err("an over-subscribed reservation was admitted, which is the \
                                failure R08 says the word deadline must not be used for"
                        .into());
                }
                let Some((domain, _)) = error::unpack(self.refusal) else {
                    return Err("the over-subscribed demand was refused with something that is \
                                not a structured error"
                        .into());
                };
                if domain != error::ADMISSION {
                    return Err(format!(
                        "the refusal was in domain {domain} rather than ADMISSION, so a \
                         supervisor cannot tell a reservation it could not have from a \
                         malformed request"
                    ));
                }
                if self.periods != 0 {
                    return Err("the over-subscribed arm ran periods, so it was admitted and \
                                then missed rather than refused"
                        .into());
                }
                Ok(())
            }
            Arm::Granted => {
                if !self.admitted {
                    return Err("the reservation this arm is about was refused".into());
                }
                // The load has to have been real. Every one of these is a way
                // for a zero above to be about a workload that did nothing.
                if self.refused_placements == 0 {
                    return Err("the adversary never tried to take a reserved core, so nothing \
                                refused it and the zero below is about an adversary that did \
                                not exist"
                        .into());
                }
                if self.taken_placements == 0 {
                    return Err("the adversary was placed nowhere at all, so it ran no load".into());
                }
                if self.stretches == 0 || self.bursts == 0 {
                    return Err("no non-preemptible stretch was begun, or none was aimed at a \
                                release instant, so the load was not the adversarial one"
                        .into());
                }
                if self.clamped == 0 {
                    return Err("no period asked for more than its budget, so the clamp was \
                                never exercised"
                        .into());
                }
                if self.periods != PERIODS {
                    return Err("the run is shorter than the one it is named after".into());
                }
                // And the property.
                if self.stolen != 0 {
                    return Err(format!(
                        "{} slot(s) of a reserved core were taken by other work, which RFC \
                         0007 forecloses",
                        self.stolen
                    ));
                }
                if self.missed != 0 {
                    return Err(format!(
                        "{} of {} period(s) missed under adversarial load",
                        self.missed, self.periods
                    ));
                }
                if self.slack_min == u32::MAX {
                    return Err("no period was met, so there is no margin to report".into());
                }
                // R12, checked rather than printed. RFC 0007's exclusion is the
                // expensive branch and it is expensive in cores; a run in which
                // the reservation held nothing idle on a part that cannot
                // partition its cache is a run in which exclusion was not
                // applied — which is the waived component arriving as a green
                // result, and is the one way this arm could pass while the
                // thing it is about had not happened.
                if self.excluded == 0 {
                    return Err("the reservation held no core idle on a part with no cache \
                                partitioning, so exclusion was not applied and the grant is \
                                RFC 0007's waived component wearing a green result"
                        .into());
                }
                if self.reserved_idle == 0 {
                    return Err("the reserved core was never idle, so either the budget is the \
                                whole period — which the schedulability test refuses — or the \
                                counter that says reserved capacity stays idle does not move"
                        .into());
                }
                Ok(())
            }
            Arm::Unreserved => {
                if self.periods != PERIODS {
                    return Err("the control is shorter than the run it is a control for".into());
                }
                if self.stolen == 0 {
                    return Err("the adversary never reached the unreserved component's core, \
                                so this arm is not the control it claims to be — the granted \
                                arm's zero would hold with admission control removed"
                        .into());
                }
                if self.missed == 0 {
                    return Err("the same load missed nothing without a reservation, so the \
                                granted arm proves nothing about the reservation"
                        .into());
                }
                Ok(())
            }
        }
    }
}

/// One worker on one core.
#[derive(Clone, Copy, Default)]
struct Worker {
    /// Slots left in a stretch it may not be taken off. Unit: slots.
    stretch: u32,
}

/// Run one arm at one seed.
///
/// # Panics
///
/// Never on a machine [`machine`] describes; the `expect` is a statement that
/// the constant above is one `f_abi::reserve::Table` accepts, and a change that
/// broke it should stop the model rather than silently reshape it.
#[must_use]
pub fn run(arm: Arm, seed: u64) -> Run {
    model(arm, seed, true)
}

/// The same arm with the budget clamp removed, which must **miss**.
///
/// The control for the third of the adversary's four behaviours. A clamp
/// counted on a run whose outcome would be identical without it is a counter
/// and not a mechanism, and the review that read this file's first draft found
/// exactly that. Here the job's demand is drawn past the *period*, so a granted
/// reservation handed everything it asks for is late however exclusively it
/// owns its core — which is the state R08 says the word *deadline* must not be
/// used for, reached on purpose so that its absence in [`run`] means something.
///
/// # Panics
///
/// As [`run`].
#[must_use]
pub fn without_the_clamp(arm: Arm, seed: u64) -> Run {
    model(arm, seed, false)
}

/// One arm at one seed, with the budget clamp under a switch.
fn model(arm: Arm, seed: u64, clamp: bool) -> Run {
    let machine = machine();
    let mut table = Table::new(machine).expect("the model's machine is one the table accepts");
    let asked = demand();

    // Where the component would run if it were admitted, asked of a fresh
    // table so that the unreserved arm puts it on the same core the granted arm
    // would. A control on a different core would be a different experiment.
    let home = Table::new(machine)
        .expect("the model's machine is one the table accepts")
        .admit(&asked)
        .map_or(machine.frame_cores, |grant| grant.cores.trailing_zeros());

    let (admitted, refusal, grant) = match arm {
        Arm::Granted => match table.grant(&asked) {
            Ok(grant) => (true, 0, grant),
            Err(why) => (false, why.code(), Grant::NONE),
        },
        Arm::Unreserved => {
            // The same component with nothing reserved for it: soft class, no
            // CPU fields, which is what `docs/manifest.md` requires of a soft
            // manifest. It is admitted — the soft class is refused its memory
            // and nothing else — and it holds no core.
            let soft = Demand {
                cores: 0,
                period_ns: 0,
                budget_ns: 0,
                memory_bytes: asked.memory_bytes,
                class: class::SOFT,
                domain: asked.domain,
            };
            match table.grant(&soft) {
                Ok(grant) => (true, 0, grant),
                Err(why) => (false, why.code(), Grant::NONE),
            }
        }
        Arm::OverSubscribed => {
            // Spend the machine, then ask for the same thing again. This is
            // over-subscription rather than a malformed request: the first
            // demand is one the machine can keep and the second is one it
            // cannot, and telling those apart is the whole job.
            let first = table.grant(&asked);
            debug_assert!(first.is_ok(), "the first reservation is the one the machine can keep");
            match table.grant(&asked) {
                Ok(grant) => (true, 0, grant),
                Err(why) => (false, why.code(), Grant::NONE),
            }
        }
    };

    let mut run = Run {
        arm,
        seed,
        admitted,
        refusal,
        exercised: grant.exercised(),
        home,
        excluded: grant.excluded.count_ones(),
        periods: 0,
        met: 0,
        missed: 0,
        stolen: 0,
        refused_placements: 0,
        taken_placements: 0,
        stretches: 0,
        bursts: 0,
        clamped: 0,
        reserved_idle: 0,
        slack_min: u32::MAX,
    };

    // A demand that was refused runs nothing. That is the point of refusing
    // before anything is spent, and it is what the over-subscribed arm's
    // verdict checks: refused, not admitted-and-missed.
    if !admitted {
        return run;
    }

    let period_slots = (asked.period_ns / SLOT_NS) as u32;
    let budget_slots = (asked.budget_ns / SLOT_NS) as u32;

    // `tick_ns` is the model's own clock granularity, so the virtual clock
    // advances one slot per draw and nothing here reads a host clock.
    let mut env = SeededEnv::new(seed, SLOT_NS);
    let mut workers = [Worker::default(); 64];

    for period in 0..PERIODS {
        // ---- the release instant, and the burst aimed at it -----------------
        //
        // The adversary asks the table for every core on the machine at the
        // worst moment rather than at a random one. What refuses it is
        // `Table::reserved`, which is RFC 0007's *reserved and idle stays idle*
        // asked before the placement rather than after it.
        //
        // Both draws happen for every core the table did not refuse, whether or
        // not that core turns out to be free: the adversary decides where to
        // aim and how long to hold before it discovers what is already running
        // there, so its draw schedule is a property of the machine's shape
        // rather than of what its own earlier draws produced. A conditional
        // draw would still be deterministic; it would just be harder to reason
        // about, and this model exists to be reasoned about.
        for core in 0..machine.physical_cores {
            if table.reserved(core) {
                run.refused_placements = run.refused_placements.saturating_add(1);
                continue;
            }
            run.taken_placements = run.taken_placements.saturating_add(1);
            let aimed = env.next_u64().is_multiple_of(BURST_ODDS);
            let length = 1 + (env.next_u64() % u64::from(MAX_STRETCH_SLOTS)) as u32;
            let worker = &mut workers[core as usize];
            if worker.stretch == 0 && aimed {
                worker.stretch = length;
                run.stretches = run.stretches.saturating_add(1);
                run.bursts = run.bursts.saturating_add(1);
            }
        }

        // ---- the job, and what it asks for ----------------------------------
        //
        // Up to `OVERRUN_REACH` times what it was admitted for, which reaches
        // past the period and not merely past the budget. The server hands it
        // the budget and counts the clamp; what it asked for buys it nothing,
        // which is the whole of what a budget is — and `without_the_clamp` is
        // the run that shows the difference, because a clamp whose removal
        // changes no outcome is a counter rather than a mechanism.
        let wants = 1 + (env.next_u64() % u64::from(budget_slots * OVERRUN_REACH)) as u32;
        if wants > budget_slots {
            run.clamped = run.clamped.saturating_add(1);
        }
        let mut owed = if clamp { wants.min(budget_slots) } else { wants };

        // ---- the period ------------------------------------------------------
        let mut finished_at: Option<u32> = None;
        for slot in 0..period_slots {
            let blocked = workers[home as usize].stretch > 0;
            if owed > 0 {
                if blocked {
                    // A slot of the component's own core taken by other work.
                    // On the granted arm this cannot happen, because the
                    // placement above was refused; that it *would* happen is
                    // what the unreserved arm demonstrates.
                    run.stolen = run.stolen.saturating_add(1);
                } else {
                    owed -= 1;
                    if owed == 0 {
                        finished_at = Some(slot);
                    }
                }
            } else if table.reserved(home) {
                run.reserved_idle = run.reserved_idle.saturating_add(1);
            }

            // Every core's stretch advances, and a free core sometimes begins a
            // new one mid-period. A reserved core never does: the table is
            // asked again rather than the placement being remembered, so a
            // build in which `reserved` stopped answering would let a stretch
            // start on the reservation and the property would go red.
            //
            // This branch is reachable, which is less obvious than it looks: it
            // was not, in this file's first draft, because the burst above took
            // every free core for the whole of every period. `stretches` was
            // then exactly `periods * free_cores` — a multiplication that reads
            // like a measurement — and nothing the claim gates on moved with
            // the seed. `the_adversary_varies_with_the_seed` is the check that
            // says so now.
            for core in 0..machine.physical_cores {
                let reserved = table.reserved(core);
                let starts = env.next_u64().is_multiple_of(STRETCH_ODDS);
                let length = 1 + (env.next_u64() % u64::from(MAX_STRETCH_SLOTS)) as u32;
                let worker = &mut workers[core as usize];
                if worker.stretch > 0 {
                    worker.stretch -= 1;
                } else if !reserved && starts {
                    worker.stretch = length;
                    run.stretches = run.stretches.saturating_add(1);
                }
            }
        }

        run.periods = period + 1;
        match finished_at {
            Some(at) => {
                run.met = run.met.saturating_add(1);
                let slack = period_slots - at - 1;
                if slack < run.slack_min {
                    run.slack_min = slack;
                }
            }
            None => run.missed = run.missed.saturating_add(1),
        }
    }

    run
}

/// Every number `claims/0010` publishes about the model, under the name the
/// claim publishes it under.
///
/// # Why the mapping is here and not in the printer
///
/// Because a claim's metric and the field it is derived from belong in one
/// file. `sim/src/main.rs` prints a table for a reader; this is the row a tool
/// reads, and `xtask::admission_reached` checks every one of them against the
/// registry's `[threshold]`. A metric that exists only inside a format string
/// is a published number nobody can check, and a threshold with no metric
/// behind it is a minimum nobody enforces — which is what `claims/0008` and
/// `claims/0009` each grew a lint about, and this is the third.
///
/// The two the boot owns — `machine_grants` and `described_grants` — are not
/// here, because they are facts about a part rather than about this model, and
/// `kernel/src/main.rs` prints them under the same names.
#[must_use]
pub fn metrics(runs: &[Run; 3]) -> [(&'static str, u64); 12] {
    let granted = &runs[0];
    let unreserved = &runs[1];
    let over = &runs[2];
    [
        ("deadlines_missed_granted", u64::from(granted.missed)),
        ("reserved_slots_stolen", u64::from(granted.stolen)),
        // One demand was made and one was refused, and the second half of that
        // sentence is `periods_run_oversubscribed`: a refusal that ran nothing
        // is what separates *refused* from *admitted and then late*.
        ("oversubscribed_refusals", u64::from(!over.admitted)),
        ("periods_run_oversubscribed", u64::from(over.periods)),
        ("deadlines_missed_unreserved", u64::from(unreserved.missed)),
        ("unreserved_slots_stolen", u64::from(unreserved.stolen)),
        ("placements_refused", u64::from(granted.refused_placements)),
        ("stretches_started", u64::from(granted.stretches)),
        ("bursts_at_release", u64::from(granted.bursts)),
        ("budget_overruns_clamped", u64::from(granted.clamped)),
        ("cores_held_idle", u64::from(granted.excluded)),
        ("reserved_slots_idle", u64::from(granted.reserved_idle)),
    ]
}

/// All three arms at one seed.
#[must_use]
pub fn sweep(seed: u64) -> [Run; 3] {
    [run(Arm::Granted, seed), run(Arm::Unreserved, seed), run(Arm::OverSubscribed, seed)]
}

/// Every arm's verdict, and the one sentence that ties them together.
///
/// # Errors
///
/// The first arm that did not hold, named.
pub fn verdict(runs: &[Run; 3]) -> Result<(), String> {
    // The controls first, because if either fails then the granted arm's zero
    // is not evidence — the same ordering `chaos::verdict` uses and for the
    // same reason.
    for arm in [Arm::OverSubscribed, Arm::Unreserved, Arm::Granted] {
        let run = runs.iter().find(|run| run.arm == arm).ok_or("an arm did not run")?;
        run.verdict().map_err(|why| format!("{}: {why}", arm.name()))?;
    }

    // And the sentence the three make together, checked rather than narrated:
    // the same seed, the same adversary, one reservation between them.
    let granted = &runs[0];
    let unreserved = &runs[1];
    if granted.seed != unreserved.seed || granted.home != unreserved.home {
        return Err("the control ran a different experiment: a different seed or a different \
                    core, either of which makes the comparison meaningless"
            .into());
    }
    if granted.stretches == 0 || unreserved.stretches == 0 {
        return Err("one of the two arms had no adversary in it".into());
    }
    Ok(())
}

/// One number over every count in the sweep, so two processes can be compared
/// without parsing a report.
///
/// FNV-1a, as `chaos::digest` is, because the property being checked is that two
/// runs agree and not that the hash is strong.
///
/// # What is deliberately not in it
///
/// **The seed.** `chaos::digest` hashes only what its runs produced and this
/// does the same, because the two questions a caller asks a digest are *did one
/// seed reproduce* and *did two seeds differ* — and a digest that ate the seed
/// answers the second one yes for every implementation that could exist,
/// including one that ignored the seed entirely. That is precisely the check
/// `xtask::admission_gate` performs, and eating the seed made it unfailable.
/// This is a hash over the model's output; the seed is its input.
#[must_use]
pub fn digest(runs: &[Run; 3]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for run in runs {
        eat(run.arm as u64);
        eat(u64::from(run.admitted));
        eat(run.refusal as u64);
        eat(u64::from(run.exercised));
        eat(u64::from(run.home));
        eat(u64::from(run.excluded));
        eat(u64::from(run.periods));
        eat(u64::from(run.met));
        eat(u64::from(run.missed));
        eat(u64::from(run.stolen));
        eat(u64::from(run.refused_placements));
        eat(u64::from(run.taken_placements));
        eat(u64::from(run.stretches));
        eat(u64::from(run.bursts));
        eat(u64::from(run.clamped));
        eat(u64::from(run.reserved_idle));
        eat(u64::from(run.slack_min));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_granted_reservation_meets_every_deadline_under_the_load() {
        let runs = sweep(0x1234_5678_9abc_def0);
        runs[0].verdict().unwrap();
        assert_eq!(runs[0].missed, 0);
        assert_eq!(runs[0].stolen, 0);
        assert_eq!(runs[0].met, PERIODS);
    }

    #[test]
    fn the_same_load_misses_without_the_reservation() {
        // The arm that makes the one above evidence. If this ever stops
        // failing, the granted arm's zero has become a property of the workload
        // and the whole file is decoration.
        let runs = sweep(0x1234_5678_9abc_def0);
        runs[1].verdict().unwrap();
        assert!(runs[1].missed > 0, "the control did not miss");
        assert!(runs[1].stolen > 0, "the adversary never reached the unreserved core");
    }

    #[test]
    fn an_over_subscribed_reservation_is_refused_and_runs_nothing() {
        let runs = sweep(0x1234_5678_9abc_def0);
        runs[2].verdict().unwrap();
        assert!(!runs[2].admitted);
        assert_eq!(
            f_abi::error::unpack(runs[2].refusal),
            Some((error::ADMISSION, f_abi::reserve::Refusal::NoCore.reason()))
        );
        assert_eq!(runs[2].periods, 0, "a refused reservation runs nothing at all");
    }

    #[test]
    fn every_arm_holds_across_a_spread_of_seeds() {
        // A property that holds at one seed is a property that holds at one
        // seed. The sweep is what makes it a claim.
        for seed in 0..64_u64 {
            let runs = sweep(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1));
            verdict(&runs).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        }
    }

    #[test]
    fn one_seed_reproduces_one_run() {
        // The second half is a real assertion now and was not before: `digest`
        // used to eat `run.seed`, so two different seeds produced two different
        // digests whatever the model did — including a model that never read
        // one. The hash is over the output.
        let seed = 0xf00d_beef_cafe_1234;
        assert_eq!(digest(&sweep(seed)), digest(&sweep(seed)));
        assert_ne!(digest(&sweep(seed)), digest(&sweep(seed ^ 1)));
    }

    #[test]
    fn the_adversary_varies_with_the_seed() {
        // **The check that says this model is a model.** Ask what input would
        // make every arm, every threshold and both reproduction guards green
        // while the property was false, and the answer is `f_env::Env`
        // returning a constant: the load would be the same load at every seed,
        // the granted arm's zero would be arithmetic rather than scheduling,
        // and nothing else in this file would notice.
        //
        // So: over a spread of seeds, the counts have to *move*. Every one of
        // these is a number `claims/0010` gates on or a number one of those is
        // derived from, and a constant in any of them is a branch that is not
        // being taken.
        let seeds: [u64; 8] = [1, 2, 5, 17, 0xdead_beef, 0xf00d_beef_cafe_1234, 99, 0x5eed];
        let mut stretches = std::collections::BTreeSet::new();
        let mut missed = std::collections::BTreeSet::new();
        let mut idle = std::collections::BTreeSet::new();
        let mut clamped = std::collections::BTreeSet::new();
        let mut digests = std::collections::BTreeSet::new();
        for seed in seeds {
            let runs = sweep(seed);
            verdict(&runs).unwrap_or_else(|why| panic!("seed {seed:#x}: {why}"));
            stretches.insert(runs[0].stretches);
            missed.insert(runs[1].missed);
            idle.insert(runs[0].reserved_idle);
            clamped.insert(runs[0].clamped);
            digests.insert(digest(&runs));
        }
        assert!(
            stretches.len() > 1,
            "the adversary began the same number of stretches at every seed"
        );
        assert!(missed.len() > 1, "the control missed the same number of periods at every seed");
        assert!(
            idle.len() > 1,
            "the reserved core was idle for the same number of slots at every seed"
        );
        assert!(
            clamped.len() > 1,
            "the job overran its budget the same number of times at every seed"
        );
        assert_eq!(digests.len(), seeds.len(), "two seeds produced one run");
    }

    #[test]
    fn the_mid_period_stretch_is_reachable() {
        // `STRETCH_ODDS` was dead code in this file's first draft, because the
        // release-instant burst took every free core for the whole period.
        // A count of stretches above the most the burst alone could begin is
        // what says the branch runs at all.
        let runs = sweep(0x1234_5678_9abc_def0);
        let free = machine().physical_cores - runs[0].excluded - 1;
        let burst_ceiling = PERIODS * free;
        assert!(runs[0].stretches > 0 && runs[0].bursts > 0, "the adversary began nothing");
        assert!(
            runs[0].stretches > runs[0].bursts,
            "every stretch was begun at a release instant, so the mid-period draw is unreachable"
        );
        assert!(
            runs[0].bursts < burst_ceiling,
            "the burst landed on every free core in every period, which is a constant rather \
             than a draw"
        );
    }

    #[test]
    fn without_the_clamp_the_granted_reservation_misses() {
        // **The control for the clamp.** `budget_overruns_clamped` is a
        // `{ min = 1 }` row in a gating claim, and a minimum on a counter whose
        // removal changes no outcome is a number that counts a draw rather than
        // a mechanism. Here the same seed, the same adversary and the same
        // grant, with the clamp taken out, produce the failure the granted arm
        // exists to not have: work that outlasts its period.
        let seed = 0x1234_5678_9abc_def0;
        let clamped = run(Arm::Granted, seed);
        let unclamped = without_the_clamp(Arm::Granted, seed);
        assert_eq!(clamped.missed, 0, "the granted arm missed with the clamp in place");
        assert!(
            unclamped.missed > 0,
            "the granted arm met every deadline with the budget clamp removed, so the clamp is \
             a counter rather than a mechanism and the claim's minimum on it is free"
        );
        assert!(clamped.clamped > 0, "no period asked for more than its budget");
    }

    #[test]
    fn the_grant_says_the_sibling_mechanism_was_never_exercised() {
        // RFC 0005 rule 2 and RFC 0007's measurement rule together: this
        // machine has no sibling, so the grant cannot show all four, so the
        // report may not imply four mechanisms ran.
        let runs = sweep(1);
        assert!(!runs[0].exercised, "a part with no sibling exercised no sibling mechanism");
    }

    #[test]
    fn the_reservation_leaves_capacity_idle_and_the_run_says_how_much() {
        // R12. Three cores held idle by exclusion, and the slots the reserved
        // core itself spent idle. They are meant to be sitting idle.
        let runs = sweep(7);
        assert_eq!(runs[0].excluded, 3);
        assert!(runs[0].reserved_idle > 0);
    }
}
