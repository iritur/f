// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What this machine can reserve, and the boot that asks it.
//!
//! `f_abi::reserve` is the arithmetic and it reads a [`Machine`]. This is the
//! only place in the tree that fills one in from hardware, and everything below
//! is a `cpuid` leaf and an argument about what to do when the leaf is not
//! there. RFC 0050 is the decision; RFC 0007 is what it implements.
//!
//! # The rule this file follows, and it is the whole of it
//!
//! **Where the part does not say, the frame assumes the worst.** A leaf that is
//! absent is not a part with no sibling; it is a part whose sibling count this
//! build cannot read, and RFC 0007 rejects "grant it anyway and note it" louder
//! than it rejects anything else. So an unreadable topology becomes one
//! contention domain covering every core, which makes a hard-class reservation
//! *unobtainable* rather than obtainable on a guess. R04, applied to a machine
//! description.
//!
//! # What that costs, and it is the honest result of this task rather than a
//! disappointment
//!
//! **QEMU cannot host a hard-class reservation, and this build says so.** The
//! virtual CPU reports no thread level, no cache topology and no RDT allocation
//! leaf, so the frame's own core poisons the only contention domain there is and
//! [`Table::admit`] answers `ADMISSION/NO_CORE`. That is not a limitation being
//! worked around: it is RFC 0007's *waiving what the hardware cannot partition*
//! alternative being refused, on the one machine this tree can actually boot.
//! A build that granted here would be publishing a reservation whose four
//! components were never delivered, which is the passing test that produces a
//! number somebody believes.
//!
//! The consequence is that the *grant* path cannot be exercised by a boot on
//! this machine, and a demonstration that could only ever refuse is a
//! demonstration a stuck function would pass. So [`demonstrate`] runs the same
//! arithmetic twice — once against this machine, which must refuse and must name
//! which component it could not deliver, and once against a described part with
//! siblings and RDT, which must grant and must record all four as exercised.
//! That is `blk`'s inside-and-outside shape and `mutate`'s argument: a red
//! result with a defect proves nothing unless the same code is green without
//! one. The described half is **not a claim about this machine** and the boot
//! log says so on its own line.
//!
//! Where a reservation is granted and put under adversarial load is
//! `sim/src/reserve.rs`, and the reason it is there is in that file's first
//! paragraph: a deadline met is a timing, and under TCG a timing is a property
//! of the emulator.

use f_abi::manifest::HUGE_BYTES;
use f_abi::reserve::{Demand, Grant, Machine, Offers, Refusal, Table, obtained};

use crate::arch::x86_64::cpuid_subleaf;

/// The frame's own timer interval, which the CPU half of the schedulability
/// test refuses against.
///
/// It is `main::TIMER_HZ` read as a period. Stated here rather than imported so
/// that this file's own comment can say what it is *for*: the frame reaches a
/// core it has given away exactly once per interval — RFC 0038 — so a period
/// below this is a period the frame cannot observe, and a slack below it is a
/// slack the frame's own clock eats.
/// Unit: nanoseconds.
pub const TICK_NS: u64 = 1_000_000;

/// Cores the frame keeps for itself.
///
/// One: the boot processor. RFC 0005 rule 5 says the frame is not a kind and
/// cannot be declared, and this is the same sentence in the core accounting —
/// the core admission control runs on is not a core admission control may sell.
/// Unit: physical cores.
pub const FRAME_CORES: u32 = 1;

/// How much memory the frame will pre-fault and never reclaim.
///
/// Zero would make every demand refuse `ADMISSION/MEMORY` before the interesting
/// arithmetic ran, and any number here is a promise about an allocator that
/// does not yet do the pre-faulting: RFC 0007 requires huge pages that are
/// *faulted*, never migrated and never compacted, and `mem::FrameAllocator`
/// has buddy orders and no such pool. So this is what the frame is willing to
/// promise today and it is small on purpose.
///
/// *Reversal:* a pre-faulted, unmigratable pool in `mem.rs`, at which point this
/// is that pool's size and not a constant. Until then a hard-class grant on this
/// machine would be a promise about memory nobody pinned — which is one more
/// reason the machine below refuses one.
/// Unit: bytes.
pub const RESERVABLE_BYTES: u64 = 8 * HUGE_BYTES;

/// Logical processors per physical core, as the extended topology leaf's thread
/// level reports it.
///
/// Leaf 0x0B subleaf 0 is the SMT level and its `ebx` counts the logical
/// processors at and below it, which is the sibling count. RFC 0005 said this
/// was *one subleaf away* from what `smp::logical_processors` already reads;
/// this is that subleaf.
///
/// One when the leaf is absent, and that answer is load-bearing rather than a
/// default: [`Machine::threads_per_core`] of one makes the sibling clause
/// `obtained::UNEXERCISED`, which RFC 0005 rule 2 requires to be recorded rather
/// than counted as a satisfied mechanism. A part that cannot be asked is not a
/// part with no siblings, and the difference is what that record keeps.
/// Unit: logical processors per physical core.
#[must_use]
pub fn threads_per_core() -> u32 {
    // SAFETY: `cpuid` is unprivileged and has no memory effect. Leaf zero
    // reports the highest leaf this processor answers, which is what makes
    // asking for 0x0B a question rather than a guess.
    let (highest, _, _, _) = unsafe { cpuid_subleaf(0, 0) };
    if highest < 0x0B {
        return 1;
    }
    // SAFETY: as above, and the leaf is one this processor answers.
    let (_, ebx, ecx, _) = unsafe { cpuid_subleaf(0x0B, 0) };
    // `ecx`'s high byte is the level type: 1 is SMT. A subleaf reporting any
    // other type is a processor whose subleaf zero is not the thread level, and
    // reading its `ebx` as a sibling count would be reading a different
    // question's answer.
    if (ecx >> 8) & 0xFF != 1 || ebx == 0 {
        return 1;
    }
    ebx
}

/// Whether the part can partition its last-level cache and its memory
/// bandwidth between groups of cores.
///
/// Leaf 0x10 is the allocation-enumeration leaf: subleaf zero's `ebx` bit 1 is
/// cache allocation over L3 and bit 3 is memory-bandwidth allocation. The
/// answer is a pair of [`Offers`] and a count of classes of service, and where
/// the leaf is absent both are [`Offers::Exclusion`] with no classes — which is
/// what the hardware is saying rather than what the frame would prefer.
#[must_use]
pub fn partitioning() -> (Offers, Offers, u32) {
    // SAFETY: as `threads_per_core`.
    let (highest, _, _, _) = unsafe { cpuid_subleaf(0, 0) };
    if highest < 0x10 {
        return (Offers::Exclusion, Offers::Exclusion, 0);
    }
    // SAFETY: as above, and the leaf is one this processor answers.
    let (_, ebx, _, _) = unsafe { cpuid_subleaf(0x10, 0) };
    let cache = if ebx & (1 << 1) != 0 { Offers::Partition } else { Offers::Exclusion };
    let bandwidth = if ebx & (1 << 3) != 0 { Offers::Partition } else { Offers::Exclusion };

    // How many classes of service each resource offers, as its own subleaf's
    // `edx` low sixteen bits. The smaller of the two is what a reservation
    // needing both can have, and a resource that is not offered does not
    // constrain the other.
    let mut classes = u32::MAX;
    if matches!(cache, Offers::Partition) {
        // SAFETY: as above; subleaf 1 is the L3 resource's, and it exists
        // exactly when the bit above is set.
        let (_, _, _, edx) = unsafe { cpuid_subleaf(0x10, 1) };
        classes = classes.min((edx & 0xFFFF) + 1);
    }
    if matches!(bandwidth, Offers::Partition) {
        // SAFETY: as above; subleaf 3 is the bandwidth resource's.
        let (_, _, _, edx) = unsafe { cpuid_subleaf(0x10, 3) };
        classes = classes.min((edx & 0xFFFF) + 1);
    }
    if classes == u32::MAX {
        classes = 0;
    }
    (cache, bandwidth, classes)
}

/// This machine, as admission control sees it.
///
/// Every unknown is resolved against the reservation. The contention domains
/// are the whole machine unless the part can partition, because nothing in this
/// build reads the cache topology leaf and a domain size guessed smaller than
/// the truth is a reservation that excludes the wrong neighbours.
///
/// *Reversal:* leaf 0x04's cache-topology subleaves, which report how many
/// logical processors share each level. Reading them is what makes
/// `cores_per_cache` a fact rather than a bound, and it belongs with the ACPI
/// MADT that `smp::logical_processors` already owes.
#[must_use]
pub fn machine() -> Machine {
    let threads = threads_per_core();
    let logical = crate::smp::logical_processors() as u32;
    let physical = (logical / threads.max(1)).max(1);
    let (cache, bandwidth, partitions) = partitioning();

    // One domain per machine where the part cannot partition, and one core per
    // domain where it can — because a partitioned resource needs no exclusion
    // and the grain collapses to one. Both readings are the same sentence: the
    // grain is the set of cores that would contend, and partitioning is what
    // makes that set a single core.
    let cores_per_cache = if matches!(cache, Offers::Partition) { 1 } else { physical.max(1) };
    let cores_per_bandwidth =
        if matches!(bandwidth, Offers::Partition) { 1 } else { physical.max(1) };

    Machine {
        physical_cores: physical,
        threads_per_core: threads,
        cores_per_cache,
        cores_per_bandwidth,
        cache,
        bandwidth,
        partitions,
        frame_cores: FRAME_CORES,
        reservable_bytes: RESERVABLE_BYTES,
        tick_ns: TICK_NS,
    }
}

/// An empty reservation table for this machine.
///
/// # Errors
///
/// [`Refusal::NotSchedulable`] for a machine `f_abi::reserve` cannot describe —
/// on this build, a single-core machine, where the frame's own core is the only
/// core and there is nothing to offer.
pub fn table() -> Result<Table, Refusal> {
    Table::new(machine())
}

/// The described part the second half of the demonstration runs against.
///
/// Sixteen cores in two eight-core cache domains, two threads each, with RDT
/// allocation. **Not a claim about any machine**, and the boot log says so: it
/// is here so that the same arithmetic that refuses above can be seen to grant,
/// because a refusal from a function that only ever refuses is not a refusal.
/// It is `mutate`'s argument and `blk`'s second boot, in a `const`.
const DESCRIBED: Machine = Machine {
    physical_cores: 16,
    threads_per_core: 2,
    cores_per_cache: 8,
    cores_per_bandwidth: 16,
    cache: Offers::Partition,
    bandwidth: Offers::Partition,
    partitions: 8,
    frame_cores: FRAME_CORES,
    reservable_bytes: 64 * HUGE_BYTES,
    tick_ns: TICK_NS,
};

/// The demand both halves are asked about.
///
/// One physical core, four milliseconds of period, one and a half of budget.
/// The same shape `sim::reserve` puts under load, so the two are asking one
/// question of one arithmetic rather than two questions that happen to agree.
const ASKED: Demand = Demand {
    cores: 1,
    period_ns: 4_000_000,
    budget_ns: 1_500_000,
    memory_bytes: HUGE_BYTES,
    class: f_abi::manifest::class::HARD,
    domain: f_abi::manifest::domain::SHARED,
};

/// What one boot's admission stage found.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// This machine, as the leaves reported it.
    pub machine: Machine,
    /// What this machine said about [`ASKED`]. `None` where it granted, which
    /// on this build would be the interesting result.
    pub here: Option<Refusal>,
    /// What the described part granted.
    pub there: Option<Grant>,
    /// What the described part said about the *second* identical demand, once
    /// its capacity was spent. The over-subscription, and it must be a refusal.
    pub over: Option<Refusal>,
    /// How many demands the described part's table admitted.
    /// Unit: admissions.
    pub admissions: u32,
    /// How many it refused. Unit: refusals.
    pub refusals: u32,
    /// The idle depth computed for the granted core against a four-state exit
    /// latency table. RFC 0006.
    /// Unit: none — a state ordinal.
    pub depth: u32,
    /// Whether the core no reservation holds answered *fallback* rather than a
    /// number. RFC 0006's honest limit, driven rather than described.
    pub fallback: bool,
}

/// A worst-case exit-latency table, ascending by depth.
///
/// **Made up, and it has to be said out loud.** RFC 0006 requires a *measured*
/// exit-latency table per platform and per state, says firmware-reported
/// latencies are famously optimistic, and says measuring it is real work
/// belonging to E5-B07. This is four numbers chosen so that the arithmetic has
/// something to select from; a computed depth resting on it is a demonstration
/// that the selection works and not a statement about any part.
/// Unit: nanoseconds.
const EXIT_LATENCY_NS: [u64; 4] = [1_000, 20_000, 400_000, 5_000_000];

/// Ask this machine, and then a described one, the same question.
///
/// # Errors
///
/// [`Refusal`] where the described part could not be described, which would be
/// a defect in the constant above rather than a fact about anything.
pub fn demonstrate() -> Result<Report, Refusal> {
    let machine = machine();
    // This machine. It may refuse and it may grant, and which one it does is
    // reported rather than required: a build that required a refusal here would
    // go red the day somebody boots it on a part with RDT, which is the result
    // this whole file is trying to make reachable.
    let here = Table::new(machine).map_or_else(Some, |table| table.admit(&ASKED).err());

    // And the described part, which must grant, must record all four components
    // as exercised, and must then refuse a second demand its capacity cannot
    // hold.
    let mut described = Table::new(DESCRIBED)?;
    let there = described.grant(&ASKED).ok();
    // Spend what is left, one core at a time, so that the refusal below is an
    // over-subscription rather than an arithmetic error.
    let mut big = ASKED;
    big.cores = DESCRIBED.physical_cores - FRAME_CORES;
    let over = described.grant(&big).err();

    let (depth, fallback) = match there {
        Some(grant) => {
            let core = grant.cores.trailing_zeros();
            let depth = match described.idle_depth(core, &EXIT_LATENCY_NS) {
                f_abi::reserve::Depth::Computed(depth) => depth,
                f_abi::reserve::Depth::Fallback => u32::MAX,
            };
            // A core nothing hard-class holds. The last one, which no grant
            // above reached.
            let idle = DESCRIBED.physical_cores - 1;
            let fallback = matches!(
                described.idle_depth(idle, &EXIT_LATENCY_NS),
                f_abi::reserve::Depth::Fallback
            );
            (depth, fallback)
        }
        None => (u32::MAX, false),
    };

    Ok(Report {
        machine,
        here,
        there,
        over,
        admissions: described.admissions(),
        refusals: described.refusals(),
        depth,
        fallback,
    })
}

impl Report {
    /// Whether this boot produced what the stage exists to produce.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold.
    pub fn verdict(&self) -> Result<(), &'static str> {
        let Some(grant) = self.there else {
            return Err("the described part refused a reservation it can hold, so the arithmetic \
                        refuses everything and the refusal on this machine means nothing");
        };
        if !grant.exercised() {
            return Err("the described part granted a reservation that cannot show all four of \
                        RFC 0007's components, so the record is not being written");
        }
        if grant.cores.count_ones() != ASKED.cores {
            return Err("the grant holds a number of cores that is not the number asked for");
        }
        let Some(over) = self.over else {
            return Err("an over-subscribed reservation was admitted, which is the failure R08 \
                        says the word deadline must not be used for");
        };
        if !matches!(over, Refusal::NoCore) {
            return Err("the over-subscribed demand was refused for a reason that is not the \
                        absence of cores, so the refusal is about something else");
        }
        if self.admissions != 1 || self.refusals != 1 {
            return Err("the counters disagree with the two answers above, so a build that had \
                        stopped counting would publish the same report");
        }
        if self.depth == u32::MAX {
            return Err("no idle depth was computed for a core under a reservation, so RFC \
                        0006's arithmetic has nothing to read");
        }
        if !self.fallback {
            return Err("a core with no hard-class reservation was given a computed idle depth, \
                        which is a number it did not earn — RFC 0006's honest limit");
        }
        // And this machine. Not required to refuse — see `demonstrate` — but
        // required to have *answered*, which is what a named refusal is.
        if let Some(why) = self.here
            && matches!(why, Refusal::NotSchedulable)
            && self.machine.offerable() > 0
        {
            // A machine with cores to offer that refuses for the arithmetic
            // rather than for the capacity is a machine description this build
            // built wrong, and it would hide every other answer behind itself.
            return Err("this machine refused for the arithmetic rather than for its capacity, \
                        which means the machine description is wrong rather than the machine \
                        being small");
        }
        Ok(())
    }

    /// A word for the boot log: how the sibling clause was met, here.
    #[must_use]
    pub fn sibling_here(&self) -> &'static str {
        if self.machine.threads_per_core > 1 {
            obtained::label(obtained::EXCLUSION)
        } else {
            obtained::label(obtained::UNEXERCISED)
        }
    }
}
