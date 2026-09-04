# RFC 0050: A reservation is arithmetic over a machine, and the machine is what the part says

- Status: accepted
- Date: 2026-09-04
- Affects: `abi/` (new: `reserve.rs` — `Machine`, `Demand`, `Table`, `Grant`,
  `Refusal`, `Depth`, `obtained`), `kernel/src/admit.rs` (new — the `cpuid`
  leaves and the boot stage), `kernel/src/component.rs` (`admit` stops refusing
  the hard class by class and starts refusing it by arithmetic; `spawn`, `fill`
  and `probe_refusals` carry the table), `kernel/src/main.rs` (one stage behind
  an `admission` boot parameter), `sim/` (new: `reserve.rs` — the adversarial
  load and its two controls), `xtask/src/main.rs` (`cargo xtask admission`),
  `claims/0010-admission-refusals.toml` and `claims/0011-reservation-margin.toml`
  (new), RFC 0007 (whose four components this implements), RFC 0006 (whose idle
  depth reads this table), RFC 0005 (whose rule 2 refusal this makes the same
  refusal), RFC 0025 (whose ceiling this grants), RFC 0038 (whose timer tick
  this reads as a cost); `TODO.md` task `E1-B07`, whose exit this is
- Implements, and does not amend: RFC 0007. Every clause below is that RFC's;
  what is new here is where the arithmetic lives, what a machine description is,
  and what the frame does when the part will not say.

## Decision

**Admission control is one function over a machine description and a demand, it
lives in `abi/`, and every unknown about the machine is resolved against the
reservation.**

Three parts, and the third is the one worth arguing about.

**One arithmetic, in `abi/`.** `f_abi::reserve::Table::admit` is the whole of
the schedulability test. The frame runs it to refuse a spawn; the simulator runs
it to explore the same refusals under load; the host tests run it to establish
that it refuses at all. Two implementations of a schedulability test are two
schedulability tests, and the one that gets audited is never the one that ran.
This is `abi::deadline`'s slot — arithmetic both sides perform over wire
quantities — and RFC 0025 named the pairing before either existed.

**The four components together, or nothing.** RFC 0007 says admission tests all
four or it is testing nothing, so one call tests the whole-core rule with its
sibling, the cache partition, the bandwidth allocation and the pre-faulted
memory, and the `Grant` it answers with records for each whether it was obtained
**by partition**, **by exclusion**, or is exclusive **by construction and
unexercised**. The third value is RFC 0005 rule 2's, word for word: a part that
reports no thread-level sibling satisfies the clause by construction and the
mechanism is recorded as unexercised, *because those are the same admission and
very different evidence*. `Grant::exercised` is the predicate a claim asks
before recording a number under a reservation.

**Where the part does not say, the frame assumes the worst.** A `cpuid` leaf
that is absent is not a part with no siblings and not a part whose cache domain
is one core; it is a part this build cannot ask. So an unreadable topology
becomes **one contention domain covering every core**, and because the frame's
own cores are the lowest ones, the domain the frame sits in is not offerable at
all. This is R04 applied to a machine description, and it is RFC 0007's
*waiving what the hardware cannot partition* alternative refused on the one
machine this tree can boot.

Two smaller decisions fall out and are stated so they are not re-litigated.

**The CPU half refuses against the frame's own clock, not against a utilisation
bound.** A hard-class reservation holds whole cores, so there is no packing
problem to solve and no place for a fudge factor. What there is instead is the
frame's tick: RFC 0038 established that the frame reaches a core it has given
away exactly once per timer interval and in no other way. So a **period** below
one tick interval is a period the frame cannot observe the boundaries of, and a
**slack** below one tick interval is a slack the frame's own clock eats. Both
are refused `ADMISSION/NOT_SCHEDULABLE`. Both sides of both comparisons are
declared quantities — the manifest's, and `TIMER_HZ` — so neither is a
measurement this machine cannot take.

**A spawn tests, a place keeps, and a place is not tested against what it
holds.** A reservation's lifetime is the *place*: RFC 0041 says a place survives
its occupant and RFC 0007 says a reservation's pages are never reclaimed for its
life, so a restart into the same place keeps the same cores rather than asking
for them again. The keeping is therefore one line where the place is built, and
every spawn tests against a table that lives as long as the supervisor — a
per-call table would answer every demand as if it were the first. The half that
is easy to leave out, and was: a restart is a spawn into a place that *already
holds* the grant it is about to ask for, and testing it would find its own cores
in `taken` and refuse `ADMISSION/NO_CORE` against its own reservation, on the
first fault, for every hard-class manifest on a part that can grant one. So the
grant is carried on the place and `Grant::answers` is what tells *this place's
own reservation* from *a second demand that looks like it* — field by field
against what the record declares, so a record whose demand changed is tested
again.

**The pinned pool is the hard class's.** `Machine::reservable_bytes` is what the
frame can pre-fault in huge pages and never reclaim, and RFC 0007 asks for that
on behalf of a *reservation*. `docs/manifest.md` says the soft class is refused
nothing at admission but memory, and that memory is the account it was handed —
checked by whoever holds the account, and carrying the detail RFC 0010 wants,
the bytes the account actually holds. Charging a soft demand against the pool as
well gave the frame two unrelated causes packed into one `ADMISSION/MEMORY` code
with no way to tell them apart, which is R07 read backwards, and let soft places
exhaust a pool held for reservations nobody had made. So the pool is charged in
the hard class and nowhere else, and a soft grant records its memory component
`unexercised` rather than `partition`, because nothing was pinned for it.

**A `hostile` component's whole core is RFC 0007's core.** RFC 0005 rule 2 gives
a `hostile` component a physical core for its lifetime *held as RFC 0007 holds
one*, and a cache partition or the exclusion where there is none. That is the
same demand travelling through the same table and refused with the same codes,
whatever class the component declared. One mechanism, two claims — the shape
RFC 0007 said to look for and RFC 0005 repeated. There is no second admission
vocabulary, and adding one would be the reversal.

## Context

What was true when this was decided.

`kernel/src/component.rs` refused every hard-class manifest
`ADMISSION/NOT_SCHEDULABLE` with a comment saying why: *a hard-class reservation
is admitted by RFC 0007's arithmetic, and there is no such arithmetic in this
build — E1-B07 is the task.* That was fail-closed and correct and it named
nothing: a supervisor could not tell a machine that was full from a machine that
was small from a frame that had not implemented the test.

The memory half already existed and already refused. `ADMISSION/MEMORY` is what
a spawn gets when the supplied `Untyped` holds less than the manifest's
`memory_bytes`, and `docs/manifest.md`'s `[reservation]` block already carried
`class`, `memory_bytes`, and — required in the hard class and refused in the
soft — `cores`, `cpu_period_ns` and `cpu_budget_ns`. `abi::manifest::Record`
already had all six fields with their units, and `cargo xtask lint-manifests`
already refused a budget above its period. So the *declaration* was complete a
task early and nothing read three of its fields.

`kernel/src/runtime.rs` already refused a reclaim naming a hard-class core
`ADMISSION/RESERVED` — RFC 0038 — which is RFC 0007's *reserved and idle stays
idle* enforced at the withdrawal. What had no home was the same rule asked
*before* a placement.

`abi::deadline` already bounded how far a caller's urgency reaches and said, in
RFC 0025's own words, that *admission control is the other half of the same
sentence: a reservation that would let a component be urgent without a period
and a budget is a reservation it refuses.* The ceiling it bounds by is a class
granted once at spawn, and nothing granted one.

The alternatives that were live:

- **A utilisation bound.** Sum `budget/period` over a core and refuse above
  one. This is the classic exact test for EDF with implicit deadlines and it is
  the wrong tool here, because RFC 0007's whole-core rule means no two
  reservations ever share a core: the sum has one term. Implementing it would
  have produced arithmetic that looks like a schedulability test and refuses
  nothing, which is R08's *hint with a better name* wearing a formula.
- **The arithmetic in `kernel/`.** Where the frame is. Rejected because the
  simulator cannot then explore it and the host tests cannot then run it, so the
  thing under adversarial load would be a model of admission control rather than
  admission control. RFC 0032 rejected exactly that shape for device models.
- **A machine description that guesses.** Assume two threads per core because
  most parts have two; assume a cache domain of four because that is common.
  Rejected loudest, and it is the same rejection RFC 0007 makes: a guessed
  domain size excludes the wrong neighbours, and a reservation that excluded the
  wrong neighbours passes admission on paper and misses on the machine. The cost
  of assuming the worst is that QEMU grants nothing, and that cost is visible.
- **Granting on QEMU anyway, with a note.** The tempting one, because it would
  let the boot exercise the grant path. Rejected for the reason RFC 0007 names
  when it rejects it: *a silently waived component is precisely how a passing
  admission test becomes a missed deadline, and "noted" decays to "ignored"
  within one refactor.* The boot exercises the grant path against a **described**
  part instead, on a line that says it is not this machine.
- **A reservation table in a `static`.** Simpler than threading one through
  `spawn`. Rejected by the frame's own rule — kernel state is per-CPU and a
  reservation is machine-wide, so it is neither — and by RFC 0044, which already
  says a supervisor's tables belong to the supervisor. It lives on
  `component::demonstrate`'s stack beside the capability table, and the day
  there is a supervisor component it moves with it.
- **A fresh table per admission.** Much the smallest diff, and wrong in the way
  that is hardest to see: every demand would be answered as if it were the
  first, so a second hard-class component would be admitted onto cores the first
  already holds. A schedulability test that passes because it has forgotten is
  the exact failure this epoch's review has caught three times.

## Consequences

**Easy.** A refusal names which of RFC 0007's four components could not be
delivered, so a supervisor can tell *retry smaller* from *retry elsewhere* from
*never on this machine*. RFC 0006's idle depth becomes a function call:
`Table::idle_depth` returns the deepest state whose exit latency fits the slack
to the earliest deadline on the core, and returns `Depth::Fallback` — not a
number — for a core no hard-class reservation holds, which is that RFC's honest
limit made a value a caller cannot ignore. RFC 0025's ceiling is a field of the
`Grant`. And the whole thing is a pure function over two structs, so a machine
this tree cannot buy is a machine a test writes down.

**Hard, and it is the headline.** **QEMU cannot host a hard-class reservation
and this build says so.** The virtual CPU reports no thread level, no cache
topology and no RDT allocation leaf, so there is one contention domain, the
frame is in it, and `Table::admit` answers `ADMISSION/NO_CORE`. Every hard-class
manifest is therefore refused on the machine this tree boots, which is the same
outcome the blanket refusal had and a different sentence: it is now a fact about
the part rather than a fact about the implementation, and it changes the day
somebody boots F on silicon with RDT.

**What that costs the tests, said rather than left.** The *grant* path cannot be
exercised by a boot here, and a stage that could only ever refuse would pass on
a build whose admission control had become a function returning `Err`. So
`cargo xtask admission`'s boot half asks the same arithmetic about a described
part with siblings and RDT, requires it to grant, requires all four components
to record a mechanism, and requires a second demand to be refused. That is
`blk`'s inside-and-outside and `mutate`'s argument, and the described half is
labelled on its own line every time it is printed, because a number about a
machine nobody has is a number somebody will otherwise quote.

**And the memory is a promise nothing keeps yet.** RFC 0007 requires pre-faulted
huge pages that are never reclaimed, migrated or compacted. `mem::FrameAllocator`
has buddy orders and no such pool, so `admit::RESERVABLE_BYTES` is a constant
rather than a pool's size, and it is one more reason a hard-class grant on this
machine would be a promise about memory nobody pinned. The reversal is written
on the constant.

**What makes the model evidence, and what would have made it decoration.** The
load in `sim/src/reserve.rs` is judged by its own counts, so the question that
has to be answered about it is *what input would make every arm green while the
property was false* — and the answer is `f_env::Env` returning a constant. Three
things stand against that and each of them was absent from the first draft.
`digest` hashes what the model produced and **not the seed it was handed**: a
digest that ate the seed makes two seeds differ for every implementation that
could exist, including one that never read a seed, which turns the
differing-seeds check in `xtask::admission_gate` into a tautology. The stretch
*lengths* and the cores the release-instant burst lands on are drawn rather than
fixed, so the counts move; a burst that blanketed every free core for a whole
period made the mid-period draw unreachable and `stretches` a multiplication.
And the budget clamp has a control of its own — `without_the_clamp` runs the
granted arm with it removed and the reservation misses — because a counter whose
removal changes no outcome is not a mechanism. `claims/0010`'s thirteen
thresholds are read out of the run by `xtask::admission_reached` rather than
restated beside it, which is `claims/0008`'s and `claims/0009`'s lint a third
time.

**The costs are counted rather than described.** `Grant::excluded` is the cores
held idle so an unpartitionable resource is exclusive, and the model's
`reserved_idle` is the slots the reserved core itself spent idle. R12: *they are
meant to be sitting idle*, and these are how many. A reader who thinks the
anti-work-conserving cost is too high has the number to argue with.

**Forecloses.** A second admission vocabulary — RFC 0005's kind and RFC 0007's
class refuse through one call with one set of codes. Work-conserving scheduling
of reserved capacity, now refused before a placement as well as at a reclaim. A
reservation granted "with a note". A default machine description. And a
hard-class grant on a part that will not describe itself.

## What would reverse this

- **A part whose cache and bandwidth domains interleave rather than nest.** The
  exclusion grain is the larger of the two contention domains, which is right
  only while one contains the other. On a part where they cross, a footprint is
  a set rather than a run and the grain is a lattice. The remedy is a different
  `lowest_free`, not a smaller assumption.
- **The `cpuid` leaves being the wrong source.** `smp::logical_processors`
  already owes the ACPI MADT, and `cores_per_cache` here is a bound rather than
  a fact because nothing reads leaf 0x04. When the topology is read properly,
  the *assume the worst* rule stops costing anything on real parts, and the
  interesting question becomes whether it was ever load-bearing. If leaf 0x04 is
  present on every part this project targets, the rule survives as a fallback
  nobody reaches, and that should be said rather than left implying a policy
  that is doing work.
- **A machine on which the anti-work-conserving cost is unaffordable.** RFC
  0007's own second reversal, and this RFC is where it becomes measurable: an
  eight-core part with four-core domains and no partitioning grants exactly one
  hard-class reservation. If a realistic workload mix cannot obtain enough of
  them, the whole-core rule is a per-machine policy admission consults rather
  than a frame rule, and RFC 0007 is superseded by one that says which machines
  can afford it. The arithmetic here is what produces the evidence.
- **The frame's floor stopping being a tick.** The period and slack refusals
  rest on the frame reaching a given-away core only at its timer. RFC 0038's own
  reversal — a one-shot APIC deadline armed from a runtime's progress, or user
  interrupts, or a frame that keeps a core of its own — retires that, and then
  the floor is the arming cost rather than the tick interval, and both
  comparisons are re-derived rather than inherited.
- **The unreserved control ceasing to miss.** `sim/src/reserve.rs`'s second arm
  is what makes the first evidence: the same component, the same seed, the same
  adversary, with no reservation between them, must miss. If a change makes it
  stop missing, the granted arm's zero has become a property of the workload and
  the model is decoration until the adversary is made real again. That is a red
  build, deliberately, and it is the one failure in this design that would
  otherwise look like good news.
- **The adversary ceasing to vary.** The load is only a load while its counts
  move with the seed. `the_adversary_varies_with_the_seed` and
  `the_mid_period_stretch_is_reachable` are what say so, and a change that makes
  either of them pass on a model whose draws no longer reach a branch — a burst
  widened back to every free core, a stretch length pinned, an overrun range
  narrowed below the budget — is this design's decoration failure arriving
  green. The remedy is a draw that can land inside a period, not a wider
  threshold.
- **A soft component that does need pinned memory.** The pool is scoped to the
  hard class because RFC 0007 scopes pre-faulting to a reservation. A soft
  component that must not be migrated — a driver mapping a device window, say —
  is asking for the memory component of a reservation without the CPU half, and
  the answer is a class that says so in the manifest rather than a quiet second
  charge against a pool its class does not name.
- **Two reservations sharing a core.** Everything here is written for the
  whole-core rule: the core sets are disjoint by construction, the CPU test has
  one term, and budget enforcement protects a component's own later periods
  rather than a neighbour. A design that shares a core — which RFC 0007
  forecloses — needs the utilisation bound this RFC declined to write, and it
  needs it as an exact demand-bound test rather than as a sum.
