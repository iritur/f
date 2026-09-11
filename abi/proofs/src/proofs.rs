// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The admission properties, as bounded proofs.
//!
//! # The sentences these are about
//!
//! RFC 0007 says a granted reservation holds four things and admission tests
//! all four together or it is testing nothing. RFC 0050 says the test is one
//! function, in `abi/`, and that it refuses. `abi/src/reserve.rs`'s own tests
//! establish that it refuses for seventeen demands somebody thought of. What is
//! below is the same set of sentences with the *somebody thought of* removed: a
//! solver is asked whether any machine and any demand make one of them false.
//!
//! Four harnesses, and each is one sentence:
//!
//! 1. [`admitting_an_arbitrary_demand`] — what a grant is. Every grant answers
//!    the demand it was made for, names only cores the frame offered, runs on
//!    no core it also holds idle, records one of the three ways each component
//!    was obtained and never a fourth, and — for the hard class — admits no
//!    period or slack the frame's clock cannot observe and no memory outside
//!    the pool or the huge-page grain. Every refusal names its domain. And the
//!    soft class is refused nothing but memory, which is the two-sided half.
//! 2. [`two_grants_never_share_a_core`] — the whole-core rule across two
//!    reservations, which is the sentence `mutate-overlapping-grant` has to
//!    break. Also: `reserved` answers exactly for the cores the grants hold.
//! 3. [`a_refusal_leaves_the_table_as_it_was`] — a refused demand changes
//!    nothing but the count of refusals, so a supervisor retrying with a
//!    smaller demand is testing against the table it thinks it is.
//! 4. [`a_release_gives_back_what_was_granted`] — grant, release, admit again
//!    reproduces the grant; a second release of the same grant is refused.
//!
//! # Why the assertions are two-sided where they can be
//!
//! For the reason `kernel/proofs` gives: a property written as *a bad demand is
//! refused* is satisfied by a table that refuses everything. So harness 1 also
//! asserts that a well-formed soft demand is *granted*, harness 4 that a
//! demand granted once is granted again, and every harness carries covers for
//! the answers it expects to reach.
//!
//! # What is bounded
//!
//! [`CORES`], and the module comment in `lib.rs` says why. Nothing else: every
//! field of every demand and every shape field of the machine is a full
//! symbolic value.

use f_abi::error;
use f_abi::manifest::{HUGE_BYTES, class, domain};
use f_abi::reserve::{CORES_MAX, Demand, Grant, Machine, Offers, Refusal, Table, obtained};

#[cfg(not(kani))]
use crate::kani;

/// The most physical cores a machine in these proofs has.
///
/// **Eight, and not [`CORES_MAX`], and this is the bound the walking proofs
/// are stated inside.** `Table::admit` searches for the lowest free run of
/// cores one step at a time, and `Grant::with_split` walks the run bit by bit;
/// a bounded model checker unrolls both. At eight cores the unrolling is a
/// formula the solver finishes in seconds; at sixty-four it is one the solver
/// finishes in minutes for a harness that admits once or twice, in hours for
/// one that admits four times, and not at all for the release round trip,
/// which ran the checker out of memory. `wide-machine` reruns the two whose
/// sentences are about the walk at the full width, so that *the bound binds
/// only the cost* is a check rather than an argument for exactly the
/// properties where the walk is the subject.
///
/// What eight reaches, which is the reason it is eight and not four: a frame
/// core, a cache domain of up to four that the frame's own domain makes
/// unofferable, and a second domain a reservation can land on. The
/// eight-core-machine-with-four-core-domains case RFC 0050 works through in
/// prose is inside this bound.
/// Unit: physical cores.
#[cfg(not(feature = "wide-machine"))]
pub const CORES: u32 = 8;

/// The same bound, widened to everything the bitmaps can name.
#[cfg(feature = "wide-machine")]
pub const CORES: u32 = CORES_MAX;

/// A machine the table can be built for, every shape field chosen by the
/// solver.
///
/// The one assumption is [`Machine::check`]'s own — a table cannot be built
/// over a machine that fails it, so a harness over such a machine would prove
/// nothing about `admit` — plus the size bound above. Nothing else is assumed:
/// a domain width larger than the machine, a partition count of a billion, a
/// pool of zero bytes and a tick of one nanosecond are all in scope.
fn any_machine() -> Machine {
    let machine = Machine {
        physical_cores: kani::any(),
        threads_per_core: kani::any(),
        cores_per_cache: kani::any(),
        cores_per_bandwidth: kani::any(),
        cache: if kani::any::<bool>() { Offers::Partition } else { Offers::Exclusion },
        bandwidth: if kani::any::<bool>() { Offers::Partition } else { Offers::Exclusion },
        partitions: kani::any(),
        frame_cores: kani::any(),
        reservable_bytes: kani::any(),
        tick_ns: kani::any(),
    };
    kani::assume(machine.physical_cores <= CORES);
    kani::assume(machine.check().is_ok());
    machine
}

/// A demand with every field chosen by the solver, the unknown classes and
/// domains included.
fn any_demand() -> Demand {
    Demand {
        cores: kani::any(),
        period_ns: kani::any(),
        budget_ns: kani::any(),
        memory_bytes: kani::any(),
        class: kani::any(),
        domain: kani::any(),
    }
}

/// The cores admission may offer on this machine, as a bitmap: everything
/// below `physical_cores` that is not one of the frame's own.
///
/// Computed here from the machine's two counts and **not** by asking the
/// table, for the reason `ring/proofs` gives about `this_builds_header`: an
/// oracle that re-derives its expectation from the code under test is a change
/// detector, not a check.
fn offerable_mask(machine: &Machine) -> u64 {
    let all = if machine.physical_cores >= CORES_MAX {
        u64::MAX
    } else {
        (1_u64 << machine.physical_cores) - 1
    };
    // `frame_cores < physical_cores <= 64`, so the shift is in range.
    let frame = (1_u64 << machine.frame_cores) - 1;
    all & !frame
}

/// Field-by-field equality, because a grant travels as a value and derives no
/// `PartialEq`: what makes two grants the same is that every one of the ten
/// things they record agrees.
fn same(a: &Grant, b: &Grant) -> bool {
    a.cores == b.cores
        && a.excluded == b.excluded
        && a.period_ns == b.period_ns
        && a.budget_ns == b.budget_ns
        && a.memory_bytes == b.memory_bytes
        && a.sibling == b.sibling
        && a.cache == b.cache
        && a.bandwidth == b.bandwidth
        && a.memory == b.memory
        && a.class == b.class
}

/// Is this one of the three ways a component can be obtained, and not a
/// fourth?
fn one_of_three(how: u8) -> bool {
    how == obtained::PARTITION || how == obtained::EXCLUSION || how == obtained::UNEXERCISED
}

// ---------------------------------------------------------------------------
// 1. What a grant is, and what a refusal is.
// ---------------------------------------------------------------------------

/// `Table::admit` over every machine and every demand.
///
/// The two-sided clause is the last `if`: a soft demand with a known class and
/// domain, no CPU fields, some memory and no hostile domain **must** be
/// granted, because `docs/manifest.md` says the soft class is refused nothing
/// at admission but memory. Without it, a table that refused everything would
/// satisfy every other line in this harness.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(all(kani, not(feature = "wide-machine")), kani::unwind(12))]
#[cfg_attr(all(kani, feature = "wide-machine"), kani::unwind(68))]
fn admitting_an_arbitrary_demand() {
    let machine = any_machine();
    let Ok(table) = Table::new(machine) else { return };
    let demand = any_demand();

    match table.admit(&demand) {
        Ok(grant) => {
            assert!(grant.answers(&demand), "a grant that is not this demand's");
            assert!(grant.cores & grant.excluded == 0, "a core both run on and held idle");
            assert!(
                (grant.cores | grant.excluded) & !offerable_mask(&machine) == 0,
                "a grant naming a core the frame keeps or the part does not have"
            );
            assert!(
                one_of_three(grant.sibling)
                    && one_of_three(grant.cache)
                    && one_of_three(grant.bandwidth)
                    && one_of_three(grant.memory),
                "a fourth way of obtaining a component"
            );
            assert!(class::known(demand.class) && domain::known(demand.domain), "R04");
            assert!(grant.memory_bytes != 0, "a grant over no memory at all");

            if demand.class == class::HARD {
                assert!(
                    grant.period_ns >= machine.tick_ns
                        && grant.budget_ns <= grant.period_ns
                        && grant.period_ns - grant.budget_ns >= machine.tick_ns,
                    "a period or a slack the frame's own clock cannot observe was admitted"
                );
                assert!(
                    grant.memory_bytes.is_multiple_of(HUGE_BYTES)
                        && grant.memory_bytes <= machine.reservable_bytes,
                    "hard-class memory outside the grain or beyond the pool"
                );
                assert!(grant.cores.count_ones() == demand.cores, "not the cores declared");
                assert!(grant.memory == obtained::PARTITION, "pre-faulted memory not recorded");
                assert!(
                    (grant.sibling == obtained::EXCLUSION) == (machine.threads_per_core > 1),
                    "the sibling clause recorded against what the part reports"
                );
            } else {
                assert!(grant.period_ns == 0 && grant.budget_ns == 0, "a soft grant reserving CPU");
                assert!(
                    grant.cores == 0 || demand.domain == domain::HOSTILE,
                    "a soft grant holding a core for a component that is not hostile"
                );
                assert!(grant.excluded == 0 || grant.cores != 0, "idle cores held beside none");
            }
        }
        Err(why) => {
            assert_eq!(
                error::unpack(why.code()),
                Some((error::ADMISSION, why.reason())),
                "R07: a refusal names its domain and its component"
            );
            assert!(
                demand.class != class::SOFT
                    || !domain::known(demand.domain)
                    || demand.domain == domain::HOSTILE
                    || demand.cores != 0
                    || demand.period_ns != 0
                    || demand.budget_ns != 0
                    || why == Refusal::Memory,
                "the soft class was refused something other than memory"
            );
        }
    }

    kani::cover!(table.admit(&demand).is_ok(), "some demand is granted");
    kani::cover!(
        table.admit(&demand).is_ok() && demand.class == class::HARD,
        "some hard-class demand is granted"
    );
    kani::cover!(
        matches!(table.admit(&demand), Err(Refusal::NotSchedulable)),
        "refused: not schedulable"
    );
    kani::cover!(matches!(table.admit(&demand), Err(Refusal::NoCore)), "refused: no core");
    kani::cover!(
        matches!(table.admit(&demand), Err(Refusal::NoBandwidth)),
        "refused: no bandwidth"
    );
    kani::cover!(matches!(table.admit(&demand), Err(Refusal::Memory)), "refused: memory");
}

// ---------------------------------------------------------------------------
// 2. Two grants never share a core.
// ---------------------------------------------------------------------------

/// Two demands granted in sequence hold disjoint cores, spend no more of the
/// pool than it has, and `reserved` answers for exactly their union.
///
/// This is the harness `mutate-overlapping-grant` has to break: with the free-
/// run search no longer consulting `taken`, the second grant lands on the
/// first's cores and the first assertion below fails with the sentence
/// `xtask`'s `ABI_PROOF_HARNESSES` names.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(all(kani, not(feature = "wide-machine")), kani::unwind(12))]
#[cfg_attr(all(kani, feature = "wide-machine"), kani::unwind(68))]
fn two_grants_never_share_a_core() {
    let machine = any_machine();
    let Ok(mut table) = Table::new(machine) else { return };
    let first = any_demand();
    let second = any_demand();

    let Ok(a) = table.grant(&first) else { return };
    let outcome = table.grant(&second);
    let Ok(b) = outcome else {
        kani::cover!(a.footprint() > 0, "a second demand is refused after a first is held");
        return;
    };

    assert!((a.cores | a.excluded) & (b.cores | b.excluded) == 0, "two grants hold one core");
    if first.class == class::HARD && second.class == class::HARD {
        assert!(
            a.memory_bytes.saturating_add(b.memory_bytes) <= machine.reservable_bytes,
            "two grants pre-fault more than the pool holds"
        );
    }
    assert!(table.grants().len() == 2, "two grants were made and the table holds another count");

    let core: u32 = kani::any();
    kani::assume(core < CORES_MAX);
    let held = (a.cores | a.excluded | b.cores | b.excluded) & (1_u64 << core) != 0;
    assert!(table.reserved(core) == held, "`reserved` disagrees with what the grants hold");

    kani::cover!(a.footprint() > 0 && b.footprint() > 0, "two reservations each holding cores");
    kani::cover!(a.excluded != 0, "a grant that paid for exclusion");
}

// ---------------------------------------------------------------------------
// 3. A refusal leaves the table as it was.
// ---------------------------------------------------------------------------

/// A refused demand changes the count of refusals and nothing else.
///
/// Observed three ways rather than by comparing the struct, because the table
/// is opaque on purpose: the number of grants, whether a symbolic core is
/// reserved, and the answer a symbolic probe demand receives. A refusal that
/// spent a partition or a byte of the pool without recording a grant would be
/// invisible to the first two and caught by the third.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(all(kani, not(feature = "wide-machine")), kani::unwind(12))]
#[cfg_attr(all(kani, feature = "wide-machine"), kani::unwind(68))]
fn a_refusal_leaves_the_table_as_it_was() {
    let machine = any_machine();
    let Ok(mut table) = Table::new(machine) else { return };
    let first = any_demand();
    let _ = table.grant(&first);

    let probe = any_demand();
    let core: u32 = kani::any();
    kani::assume(core < CORES_MAX);
    let grants_before = table.grants().len();
    let reserved_before = table.reserved(core);
    let probe_before = table.admit(&probe);
    let refusals_before = table.refusals();

    let second = any_demand();
    let Err(why) = table.grant(&second) else { return };

    assert!(table.grants().len() == grants_before, "a refusal added a grant");
    assert!(table.reserved(core) == reserved_before, "a refusal took or freed a core");
    assert!(table.refusals() == refusals_before + 1, "a refusal was not counted exactly once");
    assert!(table.refusals_of(why) >= 1, "a refusal was counted under another reason");
    match (table.admit(&probe), probe_before) {
        (Ok(now), Ok(then)) => {
            assert!(same(&now, &then), "a refusal changed what a probe is granted")
        }
        (Err(now), Err(then)) => assert!(now == then, "a refusal changed why a probe is refused"),
        _ => panic!("a refusal changed whether a probe is admitted"),
    }

    kani::cover!(grants_before == 1, "the refusal was against a table holding a grant");
    kani::cover!(probe_before.is_ok(), "the probe is granted before and after");
}

// ---------------------------------------------------------------------------
// 4. A release gives back what was granted.
// ---------------------------------------------------------------------------

/// Grant, release, admit again: the same grant. And the same grant cannot be
/// released twice.
///
/// The round trip is the property `Table::release` exists to have — it is
/// written and tested before anything in the frame calls it, so that the day a
/// supervisor destroys a place the question is where to call it and not what
/// it does — and it is the property a leak would break: a partition or a byte
/// of the pool not given back makes the second `admit` answer differently.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(all(kani, not(feature = "wide-machine")), kani::unwind(12))]
#[cfg_attr(all(kani, feature = "wide-machine"), kani::unwind(68))]
fn a_release_gives_back_what_was_granted() {
    let machine = any_machine();
    let Ok(mut table) = Table::new(machine) else { return };
    let demand = any_demand();
    let Ok(granted) = table.grant(&demand) else { return };

    assert!(table.release(&granted), "a grant the table holds was not given back");
    assert!(table.grants().is_empty(), "a released table still holds something");
    assert!(!table.release(&granted), "one grant was given back twice");

    let core: u32 = kani::any();
    kani::assume(core < CORES_MAX);
    assert!(!table.reserved(core), "a released core is still reserved");

    match table.admit(&demand) {
        Ok(again) => assert!(same(&again, &granted), "the table did not return to its state"),
        Err(_) => panic!("a demand once granted is refused after its release"),
    }

    kani::cover!(granted.footprint() > 0, "a reservation holding cores made the round trip");
    kani::cover!(granted.footprint() == 0, "a soft grant made the round trip");
}
