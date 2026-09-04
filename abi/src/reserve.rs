// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The reservation table, and the arithmetic that refuses.
//!
//! RFC 0007 says a granted hard-class reservation holds four things and that
//! *admission tests all four together or it is testing nothing*; RFC 0006 reads
//! the same table to compute idle depth; RFC 0025 says admission control is the
//! other half of its own sentence, granting the ceiling that module bounds by.
//! This is that table and that arithmetic, and RFC 0050
//! (`docs/rfc/0050-a-reservation-is-arithmetic-and-it-refuses.md`) is the
//! argument for its shape.
//!
//! # The rule in one paragraph
//!
//! A demand names a class, a speculation-domain kind, a number of **physical**
//! cores, a period, a budget within it, and the memory the component is made
//! of. Admission answers with a [`Grant`] recording, for each of RFC 0007's
//! four components, whether it was obtained **by partition**, **by exclusion**,
//! or is exclusive **by construction and unexercised** — or it refuses in the
//! `ADMISSION` domain of RFC 0010, naming which of the four could not be
//! delivered. There is no fourth answer. A reservation is never granted "with a
//! note": RFC 0007 rejects that alternative loudest, because exclusion costs
//! capacity, which is visible, and waiving costs a tail, which is not.
//!
//! # Why this is in `abi/` and not in `kernel/`
//!
//! Because three readers run it and none of them may run a second copy. The
//! frame refuses a spawn with it; the simulator explores it under adversarial
//! load; the host tests below are what say it refuses at all. Two
//! implementations of a schedulability test are two schedulability tests, and
//! the one that gets audited is never the one that ran. This is
//! [`crate::deadline`]'s slot exactly — arithmetic both sides perform over wire
//! quantities, in the crate whose whole purpose is to be correct against code
//! written by somebody else — and RFC 0025 named the pairing before either
//! existed.
//!
//! # What is arithmetic here and what is a fact about a machine
//!
//! Everything in [`Table::admit`] is arithmetic over [`Machine`] and
//! [`Demand`]. Nothing in it reads hardware, and that is what makes it testable
//! at all: a machine this tree cannot obtain is a [`Machine`] a test writes
//! down. What a real part offers — how many physical cores, whether the
//! extended topology leaf reports a thread level, whether the part can
//! partition cache ways or memory bandwidth between groups of cores — is the
//! frame's to discover and `kernel/src/admit.rs`'s to fill in. A [`Machine`]
//! that overstates a part grants reservations the part cannot keep, and no
//! arithmetic here can catch that. It is named in RFC 0050's honest limit
//! rather than left for somebody to assume away.

use crate::error;
use crate::manifest::{HUGE_BYTES, Record, class, domain};

/// The most reservations one table holds.
///
/// Eight, matching the frame's own place count: a reservation belongs to a
/// component and a component occupies a place, so a table larger than the
/// places would hold entries nothing could have been granted through. Growth is
/// a table that is bought rather than a larger array — RFC 0029's shape, and
/// the day it is wanted, `Table` is where it goes.
/// Unit: reservations.
pub const RESERVATIONS_MAX: usize = 8;

/// The most physical cores this table can name.
///
/// Sixty-four, because the core sets are bitmaps in a `u64` and a bitmap is
/// what makes *which cores* a fact rather than a count. A machine with more is
/// refused by [`Machine::check`] rather than silently truncated: a table that
/// dropped core 64 would hand out a core it does not know it has already given
/// away. R04.
/// Unit: physical cores.
pub const CORES_MAX: u32 = 64;

/// How a reservation obtained one of RFC 0007's four components.
///
/// Three values and not two, and the third is the one RFC 0005 rule 2 asks for
/// by name: a part that reports no thread-level sibling *satisfies the sibling
/// clause by construction*, and the supervisor records the mechanism as
/// **unexercised** rather than as satisfied, "because those are the same
/// admission and very different evidence."
///
/// RFC 0007 puts the same obligation on every measurement collected under a
/// reservation: the record travels with the number, and *a number collected
/// under a reservation that cannot show all four is not a number about this
/// system*. [`Grant::exercised`] is what a claim asks.
pub mod obtained {
    /// The hardware partitioned the resource between groups of cores.
    pub const PARTITION: u8 = 1;
    /// The hardware could not, so co-resident capacity is held idle instead.
    /// The expensive branch, deliberately.
    pub const EXCLUSION: u8 = 2;
    /// Exclusive by construction on this machine, with no mechanism run: a part
    /// with no sibling to exclude, or a frame that runs one component at a
    /// time. An admission, and not evidence.
    pub const UNEXERCISED: u8 = 3;

    /// A word for a log.
    #[must_use]
    pub const fn label(value: u8) -> &'static str {
        match value {
            PARTITION => "partition",
            EXCLUSION => "exclusion",
            UNEXERCISED => "unexercised",
            _ => "unknown",
        }
    }
}

/// What a machine offers for a resource RFC 0007 requires to be exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Offers {
    /// The part can partition it between groups of cores — Intel RDT's cache
    /// allocation and memory-bandwidth allocation, and their counterparts.
    Partition,
    /// It cannot. The reservation takes the co-resident cores that would
    /// contend for it and holds them idle. RFC 0007's *partition by exclusion*.
    Exclusion,
}

/// The part, as far as admission is concerned.
///
/// Every field is a fact about hardware, filled in by whoever can see it, and
/// nothing here reads any. A machine this tree cannot buy is a value a test
/// writes down, which is the whole reason the arithmetic lives away from the
/// frame.
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// Physical cores the part has, siblings not counted twice.
    /// Unit: physical cores, at most [`CORES_MAX`].
    pub physical_cores: u32,
    /// Logical processors per physical core, as the extended topology leaf's
    /// thread level reports it. One means the part has no sibling to exclude,
    /// which is the QEMU case RFC 0005 rule 2 requires to be recorded rather
    /// than counted as a satisfied mechanism.
    /// Unit: logical processors per physical core, at least 1.
    pub threads_per_core: u32,
    /// Physical cores sharing one last-level cache.
    /// Unit: physical cores, at least 1.
    pub cores_per_cache: u32,
    /// Physical cores sharing one memory controller.
    /// Unit: physical cores, at least 1.
    pub cores_per_bandwidth: u32,
    /// Whether the part can partition the last-level cache.
    /// Unit: none — a mechanism, not a quantity.
    pub cache: Offers,
    /// Whether the part can partition memory bandwidth.
    /// Unit: none — a mechanism, not a quantity.
    pub bandwidth: Offers,
    /// How many distinct partitions the part offers, where it offers any. Zero
    /// where it offers none, which is the same machine [`Offers::Exclusion`]
    /// describes and is checked against it.
    /// Unit: partitions.
    pub partitions: u32,
    /// Physical cores the frame keeps and never offers. At least one: the
    /// core admission itself runs on is not a core admission may sell.
    /// Unit: physical cores.
    pub frame_cores: u32,
    /// Memory the frame can pre-fault in huge pages and never reclaim,
    /// migrate or compact.
    ///
    /// **The hard class's pool, and only its.** `docs/manifest.md` scopes
    /// pre-faulted huge pages to the class that declares a reservation, and the
    /// soft class *is refused nothing at admission but memory* — where that
    /// memory is the account it was handed, checked by whoever holds the
    /// account rather than here. So a soft demand is not charged against this,
    /// and the reason it once was is worth keeping: charging it made two
    /// unrelated causes share one `ADMISSION/MEMORY` code, so a supervisor
    /// could not tell *your account is too small* from *the frame's pinned pool
    /// is full* — R07 — and it let eight soft places exhaust a pool that exists
    /// for reservations nobody had made.
    /// Unit: bytes, a multiple of [`HUGE_BYTES`].
    pub reservable_bytes: u64,
    /// The interval at which the frame's own clock reaches a core it has given
    /// away.
    ///
    /// **This is what makes the CPU half of the test able to refuse**, and it
    /// is a configured rate rather than a measured cost: `TIMER_HZ` is a
    /// constant the frame sets, so a period below it is a period the frame
    /// cannot observe and a slack below it is a slack the frame's own tick
    /// eats. RFC 0038 publishes those ticks as a bucket beside the hot path
    /// rather than subtracting them; this is the same number read as a cost.
    /// Unit: nanoseconds.
    pub tick_ns: u64,
}

impl Machine {
    /// Is this a machine the table can reason about at all?
    ///
    /// Fail closed. A part described in a way this build cannot represent —
    /// more cores than the bitmaps hold, no frame core, a partition count that
    /// disagrees with what the part is said to offer — is refused rather than
    /// clamped, because every clamp here is a reservation granted against
    /// capacity that was invented at the boundary.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotSchedulable`], which is the honest domain: nothing about
    /// this machine can be scheduled if the machine cannot be described.
    pub fn check(&self) -> Result<(), Refusal> {
        if self.physical_cores == 0 || self.physical_cores > CORES_MAX {
            return Err(Refusal::NotSchedulable);
        }
        if self.threads_per_core == 0 || self.cores_per_cache == 0 || self.cores_per_bandwidth == 0
        {
            return Err(Refusal::NotSchedulable);
        }
        if self.frame_cores == 0 || self.frame_cores >= self.physical_cores {
            return Err(Refusal::NotSchedulable);
        }
        if self.tick_ns == 0 {
            return Err(Refusal::NotSchedulable);
        }
        // A part that says it partitions and offers no partition is a part that
        // does not partition, and the two spellings would grant different
        // reservations for one machine.
        let partitions_needed =
            matches!(self.cache, Offers::Partition) || matches!(self.bandwidth, Offers::Partition);
        if partitions_needed && self.partitions == 0 {
            return Err(Refusal::NotSchedulable);
        }
        Ok(())
    }

    /// Cores admission may ever offer. Unit: physical cores.
    #[must_use]
    pub const fn offerable(&self) -> u32 {
        self.physical_cores.saturating_sub(self.frame_cores)
    }
}

/// What a component asks admission for.
///
/// [`Demand::of`] reads it out of a compiled manifest, which is the only place
/// one legitimately comes from: RFC 0025 says the ceiling is declared in a
/// manifest and granted once at spawn, and a demand assembled anywhere else is
/// a component asking for a promise its own image did not state.
#[derive(Clone, Copy, Debug)]
pub struct Demand {
    /// Whole physical cores, both SMT siblings held.
    /// Unit: physical cores. Zero in the soft class.
    pub cores: u32,
    /// The period the test admits against.
    /// Unit: nanoseconds. Zero in the soft class.
    pub period_ns: u64,
    /// Execution time per period, across the cores asked for.
    /// Unit: nanoseconds. Zero in the soft class.
    pub budget_ns: u64,
    /// Everything the component is made of, pre-faulted in the hard class.
    /// Unit: bytes.
    pub memory_bytes: u64,
    /// The reservation class.
    /// Unit: none — a [`class`] constant.
    pub class: u8,
    /// RFC 0005's speculation-domain kind, which is the other reason a core is
    /// held whole.
    /// Unit: none — a [`domain`] constant.
    pub domain: u8,
}

impl Demand {
    /// What a compiled manifest asks for.
    #[must_use]
    pub const fn of(record: &Record) -> Self {
        Self {
            cores: record.cores,
            period_ns: record.cpu_period_ns,
            budget_ns: record.cpu_budget_ns,
            memory_bytes: record.memory_bytes,
            class: record.class,
            domain: record.domain,
        }
    }

    /// Whole physical cores this demand holds, before any exclusion.
    ///
    /// The hard class holds what it declared. The soft class holds none — with
    /// one exception, and it is the join RFC 0007 said to look for: RFC 0005
    /// gives a `hostile` component *a whole physical core for its lifetime,
    /// held as RFC 0007 holds one*, whatever class it declared. One mechanism,
    /// two claims; the alternative is a second core-accounting vocabulary that
    /// agrees with this one until the day it does not.
    /// Unit: physical cores.
    #[must_use]
    pub const fn whole_cores(&self) -> u32 {
        if self.class == class::HARD {
            self.cores
        } else if self.domain == domain::HOSTILE {
            1
        } else {
            0
        }
    }
}

/// Why admission said no.
///
/// Every one is in the `ADMISSION` domain of RFC 0010, and the code names which
/// of RFC 0007's components could not be delivered. That is R07 read strictly:
/// a caller that cannot tell *why* it was refused cannot handle the refusal as
/// ordinary control flow, and "the reservation was refused" on its own tells a
/// supervisor nothing about whether a smaller demand would fit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The arithmetic failed: a period the frame's own clock cannot observe, a
    /// slack its tick would eat, or a demand no machine could state.
    NotSchedulable,
    /// Not enough physical cores under the whole-core rule and whatever
    /// exclusion the part's missing partitioning costs.
    NoCore,
    /// A cache partition or a bandwidth allocation could not be obtained, by
    /// partition or by exclusion.
    NoBandwidth,
    /// The pre-faulted pool cannot hold what the component is made of.
    Memory,
}

impl Refusal {
    /// The packed `Cqe::result` this becomes on the wire.
    #[must_use]
    pub const fn code(self) -> i32 {
        error::pack(error::ADMISSION, self.reason())
    }

    /// The code within [`error::ADMISSION`].
    /// Unit: none — an `error::admission` constant.
    #[must_use]
    pub const fn reason(self) -> u16 {
        match self {
            Self::NotSchedulable => error::admission::NOT_SCHEDULABLE,
            Self::NoCore => error::admission::NO_CORE,
            Self::NoBandwidth => error::admission::NO_BANDWIDTH,
            Self::Memory => error::admission::MEMORY,
        }
    }

    /// A sentence for a log, naming what could not be satisfied.
    #[must_use]
    pub const fn why(self) -> &'static str {
        match self {
            Self::NotSchedulable => {
                "the period or the slack is smaller than the interval at which the frame's own \
                 clock reaches the core"
            }
            Self::NoCore => "not enough whole physical cores, counting what exclusion costs",
            Self::NoBandwidth => "no cache partition or bandwidth allocation is left to give",
            Self::Memory => "the pre-faulted pool cannot hold what the component is made of",
        }
    }
}

/// A reservation that was granted, and how each of its four parts was obtained.
#[derive(Clone, Copy, Debug)]
pub struct Grant {
    /// The physical cores the component runs on, as a bitmap.
    /// Unit: none — bit *n* is physical core *n*.
    pub cores: u64,
    /// The physical cores held idle so that an unpartitionable resource is
    /// exclusive to the cores above. RFC 0007's expensive branch, counted so
    /// that R12 is satisfied by arithmetic rather than by a sentence: *they are
    /// meant to be sitting idle*, and this says how many.
    /// Unit: none — bit *n* is physical core *n*.
    pub excluded: u64,
    /// The period admitted against.
    /// Unit: nanoseconds. Zero where no CPU was reserved.
    pub period_ns: u64,
    /// The budget admitted.
    /// Unit: nanoseconds. Zero where no CPU was reserved.
    pub budget_ns: u64,
    /// The pre-faulted memory held for the life of the reservation.
    /// Unit: bytes.
    pub memory_bytes: u64,
    /// How the whole-core rule was met — the sibling half of RFC 0007's first
    /// component.
    /// Unit: none — an [`obtained`] constant.
    pub sibling: u8,
    /// How the last-level cache partition was obtained.
    /// Unit: none — an [`obtained`] constant.
    pub cache: u8,
    /// How the memory-bandwidth allocation was obtained.
    /// Unit: none — an [`obtained`] constant.
    pub bandwidth: u8,
    /// How the pre-faulted memory was obtained.
    /// Unit: none — an [`obtained`] constant.
    pub memory: u8,
    /// The class this reservation was admitted for, which is RFC 0025's
    /// ceiling: every channel the component opens reports this ordinal, and an
    /// entry above it is refused `ADMISSION/NOT_HELD`.
    /// Unit: none — a [`class`] constant.
    pub class: u8,
}

impl Grant {
    /// A grant holding nothing.
    pub const NONE: Self = Self {
        cores: 0,
        excluded: 0,
        period_ns: 0,
        budget_ns: 0,
        memory_bytes: 0,
        sibling: obtained::UNEXERCISED,
        cache: obtained::UNEXERCISED,
        bandwidth: obtained::UNEXERCISED,
        memory: obtained::UNEXERCISED,
        class: 0,
    };

    /// Whether every one of RFC 0007's four components ran a mechanism.
    ///
    /// **A grant this answers `false` for is still a grant**, and that is the
    /// point of recording rather than refusing: on a machine with no sibling
    /// there is nothing to exclude, and refusing would make the development
    /// machine unable to host what the production machine can. What it may not
    /// do is carry a measurement. RFC 0007: *a number collected under a
    /// reservation that cannot show all four is not a number about this
    /// system*, and this is the predicate a claim asks before recording one.
    #[must_use]
    pub const fn exercised(&self) -> bool {
        self.sibling != obtained::UNEXERCISED
            && self.cache != obtained::UNEXERCISED
            && self.bandwidth != obtained::UNEXERCISED
            && self.memory != obtained::UNEXERCISED
    }

    /// Whether this grant is the one that demand was admitted for.
    ///
    /// # Why a holder needs to ask
    ///
    /// Because **a reservation belongs to a place and not to an occupant**, and
    /// a holder that re-tests a demand it already holds a grant for is asking
    /// the table for a second copy of something it has. [`Table::admit`] would
    /// then find the cores in `taken` and refuse `ADMISSION/NO_CORE` — against
    /// the holder's own cores. RFC 0041 says a place survives its occupant and
    /// RFC 0007 says a reservation's pages are never reclaimed for its life, so
    /// a restart into the same place must keep what the place holds; this is
    /// the predicate that says it is the same thing being kept rather than a
    /// new demand wearing an old grant.
    ///
    /// Compared field by field rather than by identity, because a grant travels
    /// as a value: what makes it *this* demand's is that the class, the period,
    /// the budget, the memory and the whole-core count are the ones the record
    /// declares. A record that changed any of them is a different demand and is
    /// tested again — which is R04, and which is also what a place's manifest
    /// pin already refuses on other grounds.
    #[must_use]
    pub fn answers(&self, demand: &Demand) -> bool {
        self.class == demand.class
            && self.period_ns == demand.period_ns
            && self.budget_ns == demand.budget_ns
            && self.memory_bytes == demand.memory_bytes
            && self.cores.count_ones() == demand.whole_cores()
    }

    /// How many physical cores this reservation takes off the machine, held and
    /// idled together.
    /// Unit: physical cores.
    #[must_use]
    pub const fn footprint(&self) -> u32 {
        (self.cores | self.excluded).count_ones()
    }

    /// The slack between one release and the next.
    ///
    /// What RFC 0006 computes idle depth against: on entering idle the frame
    /// selects the deepest state whose worst-case exit latency fits inside this.
    /// `None` where no CPU was reserved, which is that RFC's honest limit —
    /// soft, batch and idle work states no period, contributes nothing to the
    /// arithmetic, and must not be given a computed depth it did not earn.
    /// Unit: nanoseconds.
    #[must_use]
    pub const fn slack_ns(&self) -> Option<u64> {
        if self.period_ns == 0 {
            return None;
        }
        Some(self.period_ns.saturating_sub(self.budget_ns))
    }
}

/// What the frame chose to idle at, and whether it computed the choice.
///
/// Two variants because RFC 0006 requires the difference to be visible: *the
/// frame idles on the shallowest state its observed wake pattern justifies and
/// **records that it is doing so**, so the difference between a computed
/// selection and a fallback is visible in the measurement rather than hidden
/// inside it. A claim collected under the fallback is not a claim about this
/// decision.*
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    /// The deepest state whose worst-case exit latency fits the slack to the
    /// earliest deadline on this core. The ordinal indexes the exit-latency
    /// table the caller supplied.
    Computed(u32),
    /// Nothing hard-class holds this core, so there is no earliest deadline to
    /// compute against and nothing here pretends there is one.
    Fallback,
}

/// Every reservation this machine has granted.
///
/// Held by whoever is admitting — the frame today, a supervisor component the
/// day RFC 0008's supervisor moves out of it — rather than in a static, for
/// `kernel/src/runtime.rs`'s reason: a table in a static is kernel-global
/// mutable state, and the frame's rule is that there is none that is not
/// per-CPU.
#[derive(Clone, Copy, Debug)]
pub struct Table {
    machine: Machine,
    held: [Grant; RESERVATIONS_MAX],
    count: usize,
    /// Cores any grant holds or idles, so that the whole-core rule is one mask
    /// test rather than a walk that could disagree with itself.
    taken: u64,
    /// Partitions handed out, where the part offers any.
    /// Unit: partitions.
    partitions_used: u32,
    /// Pre-faulted memory handed out. Unit: bytes.
    memory_used: u64,
    /// How many demands were admitted. Unit: admissions.
    admitted: u32,
    /// How many were refused, by reason, in [`Refusal`]'s own order.
    /// Unit: refusals.
    refused: [u32; 4],
}

impl Table {
    /// An empty table for a machine.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotSchedulable`] for a machine this build cannot describe.
    /// Fail closed at construction rather than at the first admission, because
    /// a table built over a machine nobody checked would refuse or grant for
    /// reasons that are about the description rather than about the part.
    pub fn new(machine: Machine) -> Result<Self, Refusal> {
        machine.check()?;
        Ok(Self {
            machine,
            held: [Grant::NONE; RESERVATIONS_MAX],
            count: 0,
            taken: 0,
            partitions_used: 0,
            memory_used: 0,
            admitted: 0,
            refused: [0; 4],
        })
    }

    /// The machine this table admits against.
    #[must_use]
    pub const fn machine(&self) -> &Machine {
        &self.machine
    }

    /// The reservations granted so far.
    #[must_use]
    pub fn grants(&self) -> &[Grant] {
        self.held.get(..self.count).unwrap_or(&[])
    }

    /// How many demands this table admitted. Unit: admissions.
    #[must_use]
    pub const fn admissions(&self) -> u32 {
        self.admitted
    }

    /// How many it refused, altogether. Unit: refusals.
    #[must_use]
    pub const fn refusals(&self) -> u32 {
        let [a, b, c, d] = self.refused;
        a + b + c + d
    }

    /// How many it refused for one reason. Unit: refusals.
    #[must_use]
    pub const fn refusals_of(&self, why: Refusal) -> u32 {
        self.refused[Self::slot(why)]
    }

    const fn slot(why: Refusal) -> usize {
        match why {
            Refusal::NotSchedulable => 0,
            Refusal::NoCore => 1,
            Refusal::NoBandwidth => 2,
            Refusal::Memory => 3,
        }
    }

    /// Test a demand against the machine and everything already granted,
    /// without granting anything.
    ///
    /// **The four components together**, which is RFC 0007's whole instruction:
    /// a test that passed on cores and left cache to a later stage would be a
    /// test that admits a reservation the machine cannot keep, and the RFC says
    /// that is worse than not testing because a passing test produces a number
    /// somebody believes.
    ///
    /// The order the four are checked in is the order they are cheapest to
    /// refuse in, and it is observable — the code a caller sees names the
    /// *first* component that could not be delivered rather than all of them.
    /// That is deliberate and it is a real limit: a demand that fails three
    /// components is refused for one, and a supervisor retrying with a smaller
    /// demand may be refused again for a different reason. Naming all four
    /// would need a structure where RFC 0010 gives a domain and a code, and a
    /// second refusal is cheaper than a second ABI.
    ///
    /// # Errors
    ///
    /// [`Refusal`], naming which component could not be satisfied.
    pub fn admit(&self, demand: &Demand) -> Result<Grant, Refusal> {
        // R04, before anything else: a class this build does not know is not a
        // class that reserves nothing, it is a record this build cannot read.
        if !class::known(demand.class) || !domain::known(demand.domain) {
            return Err(Refusal::NotSchedulable);
        }

        // ---------------------------------------------------- the CPU half
        //
        // The frame's floor. RFC 0025 bound 3 says an inherited deadline is
        // never earlier than arrival plus the callee's floor — its worst-case
        // service time — and the frame's floor on a core it has given away is
        // one tick interval, because that is the granularity at which it
        // reaches the core at all. RFC 0038 is where that mechanism is: the
        // reclaim notice is posted from the timer handler, because the frame
        // is not running while the core it gave away is running.
        //
        // So two refusals, and both are counts of ticks rather than measured
        // costs:
        //
        // - A **period** below one tick is a period the frame cannot observe.
        //   Admitting it would be promising to enforce a budget the frame
        //   cannot see the boundaries of.
        // - A **slack** below one tick is a slack the frame's own clock eats.
        //   The tick lands on a reserved core whatever the component does —
        //   RFC 0038 publishes it as a bucket beside the hot path rather than
        //   subtracting it — so a reservation whose whole idle window is
        //   shorter than the interval between two ticks has no window at all.
        //
        // This is the arithmetic that refuses an over-subscribed demand in the
        // time dimension, and it is deliberately not a utilisation bound with a
        // fudge factor in it. `TIMER_HZ` is a constant the frame sets, so both
        // sides of both comparisons are declared quantities and neither is a
        // measurement this machine cannot take.
        let cpu = demand.class == class::HARD;
        if cpu {
            if demand.cores == 0 || demand.period_ns == 0 || demand.budget_ns == 0 {
                return Err(Refusal::NotSchedulable);
            }
            if demand.budget_ns > demand.period_ns {
                return Err(Refusal::NotSchedulable);
            }
            if demand.period_ns < self.machine.tick_ns {
                return Err(Refusal::NotSchedulable);
            }
            if demand.period_ns - demand.budget_ns < self.machine.tick_ns {
                return Err(Refusal::NotSchedulable);
            }
        } else if demand.cores != 0 || demand.period_ns != 0 || demand.budget_ns != 0 {
            // The soft class states no CPU demand — `docs/manifest.md` refuses
            // the three fields there — so a record carrying one is a record
            // this build did not read the way its author wrote it. R04.
            return Err(Refusal::NotSchedulable);
        }

        // ------------------------------------------------- the memory half
        //
        // The pool is the **hard class's**, and the scoping is the whole of
        // this comment. RFC 0007's memory component is pre-faulted huge pages
        // that are never reclaimed, migrated or compacted for the life of a
        // reservation; `docs/manifest.md` gives them to the class that declares
        // one and says the soft class *is refused nothing at admission but
        // memory* — the memory of its own account, which is not this. Charging
        // a soft demand here made two unrelated causes share one
        // `ADMISSION/MEMORY` code with no way to tell them apart, which is R07
        // read backwards, and let soft places exhaust a pool held for
        // reservations nobody had made.
        if demand.memory_bytes == 0 {
            return Err(Refusal::Memory);
        }
        if cpu {
            if !demand.memory_bytes.is_multiple_of(HUGE_BYTES) {
                // Pre-faulted, in huge pages, never reclaimed. A demand not in
                // the grain is one the pool cannot pre-fault whole.
                return Err(Refusal::Memory);
            }
            let Some(free) = self.machine.reservable_bytes.checked_sub(self.memory_used) else {
                return Err(Refusal::Memory);
            };
            if demand.memory_bytes > free {
                return Err(Refusal::Memory);
            }
        }

        // --------------------------------------------------- the core half
        let wanted = demand.whole_cores();
        if wanted == 0 {
            // Nothing is held, so nothing can be excluded and no partition is
            // spent. The soft class is refused its memory and nothing else,
            // which is what `docs/manifest.md` says the class means.
            return Ok(Grant {
                cores: 0,
                excluded: 0,
                period_ns: 0,
                budget_ns: 0,
                memory_bytes: demand.memory_bytes,
                // Exclusive by construction: the frame runs one component at a
                // time, so there is no co-resident anything and no mechanism
                // ran. An admission, and not evidence — which is exactly what
                // `UNEXERCISED` says. The memory is `UNEXERCISED` for the same
                // reason and it is not a technicality: nothing was pre-faulted,
                // pinned or held for this component, so a record saying the
                // memory component was *obtained* would be the one line of a
                // grant that lied.
                sibling: obtained::UNEXERCISED,
                cache: obtained::UNEXERCISED,
                bandwidth: obtained::UNEXERCISED,
                memory: obtained::UNEXERCISED,
                class: demand.class,
            });
        }

        // What exclusion costs, before asking whether it fits. Where the part
        // cannot partition a resource, the reservation holds every co-resident
        // core that would contend for it — so the demand grows to whole cache
        // domains, whole bandwidth domains, or both, **and lands on a domain
        // boundary**. This is the branch RFC 0007 calls expensive and takes
        // anyway, because *the alternative is a reservation that passes
        // admission on paper and misses on the machine*.
        //
        // The alignment is the half that is easy to leave out and would make
        // the whole thing decorative: four cores of exclusion that straddle two
        // cache domains have excluded nobody from either. It is also where the
        // cost lands hardest, and it is written here rather than discovered —
        // the frame's own cores are the lowest ones, so on a part with no cache
        // partitioning the domain the frame sits in is not offerable at all,
        // and an eight-core machine with four-core domains grants exactly one
        // hard-class reservation. That is RFC 0007's *far fewer reservations
        // are grantable than the core count suggests*, as arithmetic rather
        // than as prose.
        let grain = self.exclusion_grain();
        let footprint = round_up(wanted, grain);

        if footprint > self.machine.offerable() {
            return Err(Refusal::NoCore);
        }
        let Some(taken) = self.lowest_free(footprint, grain) else {
            return Err(Refusal::NoCore);
        };

        // ------------------------------------- the two partitionable halves
        //
        // A partition is spent once per reservation that needs one, and where
        // the part offers none the cost was already paid in cores above. So a
        // machine that partitions can run out of partitions before it runs out
        // of cores, which is what `NO_BANDWIDTH` is for.
        let partitions_wanted = (matches!(self.machine.cache, Offers::Partition) as u32)
            + (matches!(self.machine.bandwidth, Offers::Partition) as u32);
        if self.partitions_used + partitions_wanted > self.machine.partitions {
            return Err(Refusal::NoBandwidth);
        }

        Ok(Grant {
            cores: 0,
            excluded: 0,
            period_ns: demand.period_ns,
            budget_ns: demand.budget_ns,
            memory_bytes: demand.memory_bytes,
            sibling: if self.machine.threads_per_core > 1 {
                obtained::EXCLUSION
            } else {
                // RFC 0005 rule 2, word for word: a part that reports no
                // thread-level sibling satisfies the clause by construction,
                // and the mechanism is recorded as unexercised.
                obtained::UNEXERCISED
            },
            cache: match self.machine.cache {
                Offers::Partition => obtained::PARTITION,
                Offers::Exclusion => obtained::EXCLUSION,
            },
            bandwidth: match self.machine.bandwidth {
                Offers::Partition => obtained::PARTITION,
                Offers::Exclusion => obtained::EXCLUSION,
            },
            memory: obtained::PARTITION,
            class: demand.class,
        }
        .with_split(taken, wanted))
    }

    /// Admit a demand and keep what it was granted.
    ///
    /// The counters move on both outcomes, which is what makes the refusals a
    /// number rather than a control-flow detail: a build in which admission had
    /// stopped refusing publishes a zero here, and a claim whose threshold is a
    /// *minimum* on refusals is what goes red for it.
    ///
    /// # Errors
    ///
    /// [`Refusal`], as [`Table::admit`].
    pub fn grant(&mut self, demand: &Demand) -> Result<Grant, Refusal> {
        let outcome = self.admit(demand);
        let granted = match outcome {
            Ok(granted) => granted,
            Err(why) => {
                self.refused[Self::slot(why)] = self.refused[Self::slot(why)].saturating_add(1);
                return Err(why);
            }
        };
        if self.count >= RESERVATIONS_MAX {
            // The table is an object with a size, and a full one refuses in the
            // domain the size belongs to rather than overwriting a grant
            // somebody holds. RFC 0029 is the shape that makes this a table
            // bought rather than a constant raised.
            self.refused[Self::slot(Refusal::NoCore)] =
                self.refused[Self::slot(Refusal::NoCore)].saturating_add(1);
            return Err(Refusal::NoCore);
        }
        self.held[self.count] = granted;
        self.count += 1;
        self.taken |= granted.cores | granted.excluded;
        // Only the hard class spends the pre-faulted pool, for the reason
        // `admit` gives above: the pool is what RFC 0007 pins for a
        // reservation, and a soft component's memory is its account's.
        if granted.class == class::HARD {
            self.memory_used = self.memory_used.saturating_add(granted.memory_bytes);
        }
        if granted.footprint() > 0 {
            let wanted = (matches!(self.machine.cache, Offers::Partition) as u32)
                + (matches!(self.machine.bandwidth, Offers::Partition) as u32);
            self.partitions_used = self.partitions_used.saturating_add(wanted);
        }
        self.admitted = self.admitted.saturating_add(1);
        Ok(granted)
    }

    /// Give a reservation back.
    ///
    /// A reservation lives as long as the **place** it was granted to, not as
    /// long as the occupant: RFC 0041 says a place survives its occupant, and
    /// RFC 0007 says a hard-class reservation's memory is never reclaimed,
    /// migrated or compacted *for the life of the reservation* — so a restart
    /// into the same place keeps the same cores and the same pre-faulted pages,
    /// which is the whole reason those pages are pre-faulted. What ends a
    /// reservation is the place being destroyed.
    ///
    /// **Nothing in the frame calls this yet, and that is structural rather
    /// than an omission.** No place is destroyed within a boot — a retired
    /// place stays retired and keeps what it holds, which is what RFC 0007
    /// requires of it — so there is nothing to give back. The day a supervisor
    /// destroys a place, this is the call, and it is written and tested now so
    /// that the day it is needed the question is where to call it and not what
    /// it should do.
    ///
    /// Answers whether anything was given back, so a caller cannot mistake
    /// releasing nothing for releasing something.
    pub fn release(&mut self, grant: &Grant) -> bool {
        let Some(at) = self.grants().iter().position(|held| {
            held.cores == grant.cores
                && held.excluded == grant.excluded
                && held.memory_bytes == grant.memory_bytes
                && held.period_ns == grant.period_ns
        }) else {
            return false;
        };
        let given = self.held[at];
        for slot in at..self.count.saturating_sub(1) {
            self.held[slot] = self.held[slot + 1];
        }
        self.count -= 1;
        self.held[self.count] = Grant::NONE;
        self.taken &= !(given.cores | given.excluded);
        if given.class == class::HARD {
            self.memory_used = self.memory_used.saturating_sub(given.memory_bytes);
        }
        if given.footprint() > 0 {
            let spent = (matches!(self.machine.cache, Offers::Partition) as u32)
                + (matches!(self.machine.bandwidth, Offers::Partition) as u32);
            self.partitions_used = self.partitions_used.saturating_sub(spent);
        }
        true
    }

    /// Does any reservation hold this physical core?
    ///
    /// What a placement asks before putting work anywhere: RFC 0007 forecloses
    /// work-conserving scheduling of reserved capacity — *reserved and idle
    /// stays idle, and no later optimisation may quietly lend it out* — so a
    /// core that is held or idled by a grant is a core nothing else may be put
    /// on. `kernel/src/runtime.rs` refuses a *reclaim* of one; this is the same
    /// rule asked before the placement rather than after it.
    #[must_use]
    pub const fn reserved(&self, core: u32) -> bool {
        if core >= CORES_MAX {
            return true;
        }
        self.taken & (1_u64 << core) != 0
    }

    /// The earliest deadline on a core, as a period.
    ///
    /// RFC 0006: *every hard-class consumer states a period in order to be
    /// admitted, so the frame knows the earliest deadline on each core.*
    /// Nothing is predicted, because the set of things that will wake this core
    /// is the set of things that were admitted to it.
    /// Unit: nanoseconds. `None` where nothing hard-class holds the core.
    #[must_use]
    pub fn earliest_deadline_ns(&self, core: u32) -> Option<u64> {
        let mut earliest: Option<u64> = None;
        for grant in self.grants() {
            if core >= CORES_MAX || grant.cores & (1_u64 << core) == 0 || grant.period_ns == 0 {
                continue;
            }
            earliest = Some(match earliest {
                Some(seen) if seen <= grant.period_ns => seen,
                _ => grant.period_ns,
            });
        }
        earliest
    }

    /// The slack to that deadline. Unit: nanoseconds.
    #[must_use]
    pub fn slack_ns(&self, core: u32) -> Option<u64> {
        let mut slack: Option<u64> = None;
        for grant in self.grants() {
            if core >= CORES_MAX || grant.cores & (1_u64 << core) == 0 {
                continue;
            }
            let Some(theirs) = grant.slack_ns() else { continue };
            slack = Some(match slack {
                Some(seen) if seen <= theirs => seen,
                _ => theirs,
            });
        }
        slack
    }

    /// The deepest idle state whose worst-case exit latency fits the slack.
    ///
    /// `exit_latency_ns` is the per-platform, per-state table RFC 0006 says is
    /// real work to measure and belongs to E5-B07, ascending by depth, index
    /// zero being the shallowest. Nothing here measures it and nothing here
    /// invents it: a caller with no measured table passes the one state it can
    /// stand behind, and gets depth zero.
    ///
    /// A core with no hard-class reservation answers [`Depth::Fallback`] rather
    /// than a number, which is RFC 0006's honest limit made a value a caller
    /// cannot ignore.
    #[must_use]
    pub fn idle_depth(&self, core: u32, exit_latency_ns: &[u64]) -> Depth {
        let Some(slack) = self.slack_ns(core) else { return Depth::Fallback };
        let mut deepest = 0;
        for (depth, latency) in exit_latency_ns.iter().enumerate() {
            if *latency <= slack {
                deepest = depth as u32;
            } else {
                break;
            }
        }
        Depth::Computed(deepest)
    }

    /// The grain a reservation's core footprint is rounded and aligned to.
    ///
    /// One where the part partitions everything RFC 0007 asks it to, and the
    /// larger of the two contention domains where it does not. Not the product
    /// and not the sum: a reservation that owns a whole cache domain and a
    /// whole bandwidth domain owns the larger of the two, because on every
    /// topology this tree targets one contains the other.
    ///
    /// *Reversal:* a part whose cache and bandwidth domains interleave rather
    /// than nest, at which point the footprint is a set rather than a run and
    /// this is a lattice rather than a maximum.
    /// Unit: physical cores.
    const fn exclusion_grain(&self) -> u32 {
        let mut grain = 1;
        if matches!(self.machine.cache, Offers::Exclusion) && self.machine.cores_per_cache > grain {
            grain = self.machine.cores_per_cache;
        }
        if matches!(self.machine.bandwidth, Offers::Exclusion)
            && self.machine.cores_per_bandwidth > grain
        {
            grain = self.machine.cores_per_bandwidth;
        }
        grain
    }

    /// The lowest free run of `count` physical cores aligned to `grain`, as a
    /// bitmap.
    ///
    /// Lowest-first and contiguous, because a reservation obtained by exclusion
    /// has to be a *domain* — the cores that share a cache or a memory
    /// controller are contiguous in every topology this tree targets, and a
    /// scattered set would exclude the wrong neighbours. It skips the frame's
    /// own cores, which are the lowest ones: the boot processor is core zero
    /// and the frame is not a kind that can be given away.
    const fn lowest_free(&self, count: u32, grain: u32) -> Option<u64> {
        let step = if grain == 0 { 1 } else { grain };
        let mut base = round_up(self.machine.frame_cores, step);
        while base + count <= self.machine.physical_cores {
            let mask = run_mask(base, count);
            if self.taken & mask == 0 {
                return Some(mask);
            }
            base += step;
        }
        None
    }
}

impl Grant {
    /// Split a footprint into what the component runs on and what is held idle
    /// beside it.
    ///
    /// The first `wanted` cores of the run are the reservation; the rest are
    /// the exclusion RFC 0007 charges for and R12 requires to be visible as a
    /// cost rather than hidden in a metric.
    const fn with_split(mut self, footprint: u64, wanted: u32) -> Self {
        let mut running = 0_u64;
        let mut left = wanted;
        let mut bit = 0;
        while bit < CORES_MAX && left > 0 {
            if footprint & (1_u64 << bit) != 0 {
                running |= 1_u64 << bit;
                left -= 1;
            }
            bit += 1;
        }
        self.cores = running;
        self.excluded = footprint & !running;
        self
    }
}

/// `value` rounded up to a multiple of `grain`, saturating.
const fn round_up(value: u32, grain: u32) -> u32 {
    if grain <= 1 {
        return value;
    }
    let over = value % grain;
    if over == 0 { value } else { value.saturating_add(grain - over) }
}

/// A bitmap of `count` cores from `base`.
const fn run_mask(base: u32, count: u32) -> u64 {
    if count == 0 || base >= CORES_MAX {
        return 0;
    }
    let width = if count >= CORES_MAX { CORES_MAX } else { count };
    let ones = if width >= 64 { u64::MAX } else { (1_u64 << width) - 1 };
    ones << base
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A part with everything: siblings, cache and bandwidth partitioning.
    /// The machine RFC 0007 was written for.
    const RICH: Machine = Machine {
        physical_cores: 16,
        threads_per_core: 2,
        cores_per_cache: 8,
        cores_per_bandwidth: 16,
        cache: Offers::Partition,
        bandwidth: Offers::Partition,
        partitions: 8,
        frame_cores: 1,
        reservable_bytes: 64 * HUGE_BYTES,
        tick_ns: 1_000_000,
    };

    /// A part with none of it: no sibling to exclude, no cache partitioning, no
    /// bandwidth partitioning. This is the machine the tests run on, and the
    /// reason the `UNEXERCISED` record exists.
    const POOR: Machine = Machine {
        physical_cores: 8,
        threads_per_core: 1,
        cores_per_cache: 4,
        cores_per_bandwidth: 4,
        cache: Offers::Exclusion,
        bandwidth: Offers::Exclusion,
        partitions: 0,
        frame_cores: 1,
        reservable_bytes: 8 * HUGE_BYTES,
        tick_ns: 1_000_000,
    };

    const fn hard(cores: u32, period_ns: u64, budget_ns: u64) -> Demand {
        Demand {
            cores,
            period_ns,
            budget_ns,
            memory_bytes: HUGE_BYTES,
            class: class::HARD,
            domain: domain::SHARED,
        }
    }

    const fn soft() -> Demand {
        Demand {
            cores: 0,
            period_ns: 0,
            budget_ns: 0,
            memory_bytes: 4096,
            class: class::SOFT,
            domain: domain::PRIVATE,
        }
    }

    #[test]
    fn a_machine_nobody_can_describe_is_refused_rather_than_clamped() {
        let mut broken = RICH;
        broken.physical_cores = CORES_MAX + 1;
        assert_eq!(Table::new(broken).err(), Some(Refusal::NotSchedulable));

        let mut no_frame = RICH;
        no_frame.frame_cores = 0;
        assert_eq!(Table::new(no_frame).err(), Some(Refusal::NotSchedulable));

        // A part that says it partitions and offers no partition is a part that
        // does not partition. Two spellings for one machine would grant
        // different reservations depending on which was read.
        let mut lying = RICH;
        lying.partitions = 0;
        assert_eq!(Table::new(lying).err(), Some(Refusal::NotSchedulable));
    }

    #[test]
    fn the_frames_own_clock_refuses_a_period_it_cannot_observe() {
        let table = Table::new(RICH).unwrap();
        // A period below one tick interval.
        assert_eq!(table.admit(&hard(1, 500_000, 100_000)).err(), Some(Refusal::NotSchedulable));
        // Exactly one tick, with no slack left over: the tick itself eats it.
        assert_eq!(table.admit(&hard(1, 1_000_000, 900_000)).err(), Some(Refusal::NotSchedulable));
        // And the one that fits: two ticks of period, one of budget.
        assert!(table.admit(&hard(1, 2_000_000, 1_000_000)).is_ok());
    }

    #[test]
    fn an_over_subscribed_demand_is_refused_and_the_reason_names_the_component() {
        let mut table = Table::new(RICH).unwrap();
        // More cores than the machine offers, counting the frame's own.
        let refusal = table.grant(&hard(16, 10_000_000, 1_000_000)).unwrap_err();
        assert_eq!(refusal, Refusal::NoCore);
        assert_eq!(refusal.reason(), error::admission::NO_CORE);
        assert_eq!(
            error::unpack(refusal.code()),
            Some((error::ADMISSION, error::admission::NO_CORE))
        );
        assert_eq!(table.refusals_of(Refusal::NoCore), 1);
        assert_eq!(table.admissions(), 0);
    }

    #[test]
    fn a_second_reservation_cannot_have_the_first_ones_cores() {
        let mut table = Table::new(RICH).unwrap();
        let first = table.grant(&hard(8, 10_000_000, 1_000_000)).unwrap();
        assert_eq!(first.cores.count_ones(), 8);
        // Fifteen offerable cores, eight gone, eight asked for.
        assert_eq!(table.grant(&hard(8, 10_000_000, 1_000_000)).err(), Some(Refusal::NoCore));
        // And what is left does fit.
        assert!(table.grant(&hard(7, 10_000_000, 1_000_000)).is_ok());
        assert_eq!(table.admissions(), 2);
    }

    #[test]
    fn exclusion_costs_cores_and_the_cost_is_visible() {
        // The poor machine cannot partition its cache, so one core's worth of
        // reservation takes a whole four-core domain — and it takes the second
        // one, because the frame sits in the first. Eight cores, one
        // reservation, and the second is refused. That is not a small machine
        // being awkward: it is RFC 0007's consequence section as arithmetic.
        let mut table = Table::new(POOR).unwrap();
        let granted = table.grant(&hard(1, 10_000_000, 1_000_000)).unwrap();
        assert_eq!(granted.cores, 1 << 4, "the second domain, because the frame is in the first");
        assert_eq!(
            granted.excluded.count_ones(),
            3,
            "three cores held idle, and they are the cost"
        );
        assert_eq!(granted.footprint(), 4);
        assert_eq!(granted.cache, obtained::EXCLUSION);
        assert_eq!(granted.bandwidth, obtained::EXCLUSION);
        // Every core of that domain is now reserved, including the three that
        // are idle. RFC 0007 forecloses lending them out.
        for core in 4..8 {
            assert!(table.reserved(core), "core {core} is idle and reserved, and stays that way");
        }
        assert_eq!(table.grant(&hard(1, 10_000_000, 1_000_000)).err(), Some(Refusal::NoCore));
        assert_eq!(table.refusals_of(Refusal::NoCore), 1);
    }

    #[test]
    fn a_part_with_no_sibling_records_the_mechanism_as_unexercised() {
        // RFC 0005 rule 2. The admission is the same and the evidence is not,
        // and this is the difference being kept.
        let table = Table::new(POOR).unwrap();
        let granted = table.admit(&hard(1, 10_000_000, 1_000_000)).unwrap();
        assert_eq!(granted.sibling, obtained::UNEXERCISED);
        assert!(!granted.exercised(), "a grant that cannot show all four may not carry a number");

        let rich = Table::new(RICH).unwrap();
        let granted = rich.admit(&hard(1, 10_000_000, 1_000_000)).unwrap();
        assert_eq!(granted.sibling, obtained::EXCLUSION);
        assert!(granted.exercised());
    }

    #[test]
    fn a_hostile_component_takes_a_whole_core_whatever_class_it_declared() {
        // RFC 0005's table read through RFC 0007's mechanism: one mechanism,
        // two claims, which is the shape RFC 0007 said to look for.
        let mut table = Table::new(RICH).unwrap();
        let mut demand = soft();
        demand.domain = domain::HOSTILE;
        let granted = table.grant(&demand).unwrap();
        assert_eq!(granted.cores.count_ones(), 1);
        assert_eq!(
            granted.class,
            class::SOFT,
            "the class is what it declared; the core is the kind's"
        );

        // And a soft `shared` component beside it takes none.
        let granted = table.grant(&soft()).unwrap();
        assert_eq!(granted.footprint(), 0);
    }

    #[test]
    fn the_soft_class_does_not_spend_the_hard_classs_pinned_pool() {
        // `docs/manifest.md` scopes pre-faulted huge pages to the class that
        // declares a reservation. A pool of one page and four soft components
        // that would each fill it: all four are admitted, because none of them
        // is asking for any of it. What refuses a soft component its memory is
        // the *account* it was handed, two lines away in `component::admit`,
        // and it carries the detail RFC 0010 wants — the bytes the account
        // actually holds — which this refusal never could.
        let mut poor = POOR;
        poor.reservable_bytes = HUGE_BYTES;
        let mut table = Table::new(poor).unwrap();
        for _ in 0..4 {
            table.grant(&soft()).expect("a soft component does not ask for the pinned pool");
        }
        assert_eq!(table.refusals_of(Refusal::Memory), 0);

        // And the hard class still spends it, once, and is then refused for the
        // pool rather than for anything else: cores are left on this machine
        // and the pool is not.
        let mut roomy = POOR;
        roomy.reservable_bytes = HUGE_BYTES;
        roomy.cores_per_cache = 1;
        roomy.cores_per_bandwidth = 1;
        let mut table = Table::new(roomy).unwrap();
        assert!(table.grant(&hard(1, 10_000_000, 1_000_000)).is_ok());
        assert_eq!(table.grant(&hard(1, 10_000_000, 1_000_000)).err(), Some(Refusal::Memory));
        assert_eq!(table.refusals_of(Refusal::Memory), 1);

        // A demand for nothing is still refused, in both classes: a component
        // made of no bytes is a record this build did not read.
        let mut nothing = soft();
        nothing.memory_bytes = 0;
        assert_eq!(table.admit(&nothing).err(), Some(Refusal::Memory));
    }

    #[test]
    fn a_place_that_keeps_its_grant_is_not_refused_its_own_cores() {
        // **The restart case**, and the one this predicate exists for. A place
        // is granted a reservation; its occupant faults; the supervisor spawns
        // into the same place. If that spawn re-tested the demand it would be
        // refused `ADMISSION/NO_CORE` — against the cores the place already
        // holds — which is admission control refusing a component its own
        // reservation. RFC 0041's place survives its occupant, so the grant is
        // what the place keeps and `answers` is how the holder knows the grant
        // it kept is the grant this record asks for.
        let mut table = Table::new(POOR).unwrap();
        let demand = hard(1, 10_000_000, 1_000_000);
        let held = table.grant(&demand).unwrap();

        // The table would refuse it a second time, and it is right to: a second
        // *demand* is a second reservation.
        assert_eq!(table.admit(&demand).err(), Some(Refusal::NoCore));
        // But the holder is not making one.
        assert!(held.answers(&demand), "the place's own grant does not answer its own record");

        // A record that changed what it asks for is a different demand and is
        // tested again, whatever the place is holding. R04.
        assert!(!held.answers(&hard(1, 10_000_000, 2_000_000)));
        assert!(!held.answers(&hard(2, 10_000_000, 1_000_000)));
        let mut fatter = demand;
        fatter.memory_bytes = HUGE_BYTES * 2;
        assert!(!held.answers(&fatter));
        assert!(!Grant::NONE.answers(&demand));

        // And a soft place's grant answers its own record too, which is the
        // path every place in this tree actually takes today.
        let mut soft_table = Table::new(POOR).unwrap();
        let held = soft_table.grant(&soft()).unwrap();
        assert!(held.answers(&soft()));
    }

    #[test]
    fn a_soft_record_carrying_a_cpu_demand_is_refused_rather_than_ignored() {
        // R04. `docs/manifest.md` refuses the three CPU fields in the soft
        // class, so a record that has them is one this build did not read the
        // way its author wrote it.
        let table = Table::new(RICH).unwrap();
        let mut demand = soft();
        demand.cores = 1;
        assert_eq!(table.admit(&demand).err(), Some(Refusal::NotSchedulable));
    }

    #[test]
    fn hard_class_memory_is_refused_outside_the_huge_page_grain() {
        let table = Table::new(RICH).unwrap();
        let mut demand = hard(1, 10_000_000, 1_000_000);
        demand.memory_bytes = HUGE_BYTES + 4096;
        assert_eq!(table.admit(&demand).err(), Some(Refusal::Memory));
    }

    #[test]
    fn partitions_run_out_before_cores_do() {
        let mut scarce = RICH;
        scarce.partitions = 2;
        let mut table = Table::new(scarce).unwrap();
        // Each grant spends one cache partition and one bandwidth allocation.
        assert!(table.grant(&hard(1, 10_000_000, 1_000_000)).is_ok());
        assert_eq!(
            table.grant(&hard(1, 10_000_000, 1_000_000)).err(),
            Some(Refusal::NoBandwidth),
            "cores are left and partitions are not, which is what this code is for"
        );
    }

    #[test]
    fn idle_depth_is_computed_from_the_table_and_says_when_it_was_not() {
        // RFC 0006: the deepest state whose worst-case exit latency fits the
        // slack. The table is a caller's, ascending, and nothing here invents
        // one.
        let states = [1_000_u64, 20_000, 400_000, 5_000_000];
        let mut table = Table::new(RICH).unwrap();
        let granted = table.grant(&hard(1, 10_000_000, 1_000_000)).unwrap();
        let core = granted.cores.trailing_zeros();
        assert_eq!(table.slack_ns(core), Some(9_000_000));
        assert_eq!(table.idle_depth(core, &states), Depth::Computed(3));

        // A tighter reservation on the same machine gets a shallower state,
        // and the arithmetic is the whole of the reason.
        let mut tighter = Table::new(RICH).unwrap();
        let granted = tighter.grant(&hard(1, 10_000_000, 8_000_000)).unwrap();
        let core = granted.cores.trailing_zeros();
        assert_eq!(tighter.slack_ns(core), Some(2_000_000));
        assert_eq!(tighter.idle_depth(core, &states), Depth::Computed(2));

        // And a core with nothing hard-class on it gets no number at all.
        assert_eq!(table.idle_depth(15, &states), Depth::Fallback);
        assert_eq!(table.earliest_deadline_ns(15), None);
    }

    #[test]
    fn the_earliest_deadline_on_a_core_is_the_smallest_period_admitted_to_it() {
        let mut table = Table::new(RICH).unwrap();
        let first = table.grant(&hard(1, 10_000_000, 1_000_000)).unwrap();
        let second = table.grant(&hard(1, 4_000_000, 1_000_000)).unwrap();
        assert_ne!(first.cores, second.cores, "the whole-core rule; nobody shares");
        assert_eq!(table.earliest_deadline_ns(first.cores.trailing_zeros()), Some(10_000_000));
        assert_eq!(table.earliest_deadline_ns(second.cores.trailing_zeros()), Some(4_000_000));
    }

    #[test]
    fn a_released_reservation_gives_back_everything_it_took() {
        let mut table = Table::new(POOR).unwrap();
        let granted = table.grant(&hard(1, 10_000_000, 1_000_000)).unwrap();
        assert_eq!(table.grant(&hard(1, 10_000_000, 1_000_000)).err(), Some(Refusal::NoCore));
        assert!(table.release(&granted));
        for core in 0..POOR.physical_cores {
            assert!(!table.reserved(core), "core {core} was given back with the reservation");
        }
        // Including the three that were only ever idle: an exclusion that is
        // not given back is capacity nobody can ever have again.
        assert!(table.grant(&hard(1, 10_000_000, 1_000_000)).is_ok());
        // And releasing something nobody granted is answered rather than
        // silently doing nothing to the accounting.
        assert!(!table.release(&Grant::NONE));
    }

    #[test]
    fn the_table_is_an_object_with_a_size_and_a_full_one_refuses() {
        let mut table = Table::new(RICH).unwrap();
        for _ in 0..RESERVATIONS_MAX {
            table.grant(&soft()).unwrap();
        }
        assert_eq!(table.grant(&soft()).err(), Some(Refusal::NoCore));
        assert_eq!(table.admissions(), RESERVATIONS_MAX as u32);
    }

    #[test]
    fn a_reserved_core_is_never_offered_to_anything_else() {
        // RFC 0007 forecloses work-conserving scheduling of reserved capacity.
        // This is the predicate a placement asks, and the exclusion half is in
        // it: a core held idle is as unavailable as a core being used.
        let mut table = Table::new(POOR).unwrap();
        let granted = table.grant(&hard(1, 10_000_000, 1_000_000)).unwrap();
        assert_eq!(granted.footprint(), 4);
        // The grant runs on core 4 and idles 5, 6 and 7. All four are reserved
        // and none may be lent; the frame's own domain is not a reservation and
        // is not reported as one.
        assert_eq!(granted.cores, 1 << 4);
        assert_eq!(granted.excluded, (1 << 5) | (1 << 6) | (1 << 7));
        for core in 0..4 {
            assert!(!table.reserved(core), "core {core} is the frame's domain, not a reservation");
        }
        for core in 4..8 {
            assert!(table.reserved(core), "core {core} is held or idled by the grant");
        }
        assert!(
            table.reserved(CORES_MAX),
            "a core off the end is reserved, because it is not ours"
        );
    }
}
