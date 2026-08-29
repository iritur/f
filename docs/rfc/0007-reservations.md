# RFC 0007: The hard class reserves whole cores and pre-faulted memory

- Status: accepted
- Date: 2026-08-29
- Affects: `claims/0002-timer-jitter.toml`, `kernel/`, `docs/design/deadline-all-the-way-down.html` sections 01–03, RFC 0005 and RFC 0006 when they are written

## Decision

A granted hard-class reservation holds four things, and admission tests all four
together or it is testing nothing:

- **A physical core**, not a hardware thread. Both siblings of an SMT pair are
  held by the reservation or the pair is not offered to it. A sibling running
  somebody else's work shares the store buffer, the L1 and the execution ports,
  and a schedulability test that counts threads has already conceded the tail it
  was run to bound.
- **A memory-bandwidth allocation**, on hardware that can partition bandwidth
  between groups of cores. Batch work is throttled to fit around it, which is
  the general form of the compositor cap the implementation companion places by
  hand.
- **A last-level cache partition** where the hardware offers one.
- **Pre-faulted memory**, in huge pages, which is never reclaimed, migrated or
  compacted for the life of the reservation. Not merely allocated: faulted, so
  that no page fault, no tier migration and no compaction pass can land inside
  a deadline.

Where the hardware cannot partition a resource, the frame does not waive the
component and does not pretend the hardware has it. It **partitions by
exclusion** instead: the co-resident cores that would contend for the
unpartitionable resource are held idle by the same reservation. This is the
expensive branch, deliberately, because the alternative is a reservation that
passes admission on paper and misses on the machine.

Every reservation therefore records, for each of the four, whether it was
obtained **by partition** or **by exclusion**, and that record travels with any
measurement collected under it. A number collected under a reservation that
cannot show all four is not a number about this system.

Admission is arithmetic and it refuses. A refusal is an admission-domain error
under RFC 0010, naming the component that could not be satisfied.

## Context

`docs/design/deadline-all-the-way-down.html` section 01 already makes admission
control the load-bearing part of the whole resource discipline — "a reservation
is granted only if the resource can prove it is schedulable, and refused
otherwise" — and section 03 already names bandwidth as a first-class
reservation. What none of the five documents said is what a granted reservation
actually holds. The gap register calls this out as threatening every latency
claim: admission control that ignores siblings, cache and reclaim can pass and
then miss, which is worse than not testing, because a passing test produces a
number somebody believes.

What was true when this was decided: E0-B07 landed the local APIC and the
TSC-deadline timer at M2, and `claims/0002-timer-jitter.toml` had been
registered at `pending` with a 5 µs p99 bound written down before any machine
in reach could produce a number for it. That claim is the first real
measurement the project makes and it gates from M2 onward. It is a statement
about what the core was protected from, and until this RFC there was no
statement to make. Written any earlier it would have been designing against a
measurement nobody had taken — which is why the ordering rules in `TODO.md` put
it here and not at the head of the epoch.

Three alternatives were live.

**A CPU-only reservation** — pin the work to a core, give it an EDF budget, stop
there. This is the classic answer and it is what `SCHED_DEADLINE` offers. It was
rejected because the misses that survive it are exactly the three resources it
does not mention: a sibling thread, a cache evicted by a neighbour, and a page
the reclaim path took back. A reservation that ignores those is a promise about
the scheduler rather than about the deadline.

**Statistical reservations with headroom** — admit optimistically, keep a margin,
tune the margin when something misses. Rejected because it converts an
arithmetic promise into a tuning parameter, and a tuning parameter is what the
word "deadline" degenerates into everywhere else. The whole reason F refuses
work is so that a granted reservation is met by construction.

**Waiving what the hardware cannot partition** — grant the reservation anyway on
a machine with no cache partitioning, and note it. Rejected, and it is the
alternative worth naming loudest: a silently waived component is precisely how a
passing admission test becomes a missed deadline, and "noted" decays to
"ignored" within one refactor. Exclusion costs capacity, which is visible;
waiving costs a tail, which is not.

## Consequences

**Easy.** Claim 0002 becomes meaningful: the reservation is now something the
claim can name, and a jitter run on an unreserved core is visibly a different
measurement rather than an inferior one. RFC 0005 will want an exclusive
physical core for confidentiality rather than for latency, and it can want the
same mechanism — one mechanism, two claims, which is the shape worth looking
for. RFC 0006 gets a table to read: the earliest deadline on each core is known
because every hard-class consumer stated a period to get admitted, so idle depth
is arithmetic rather than prediction.

**Hard.** Far fewer reservations are grantable than the core count suggests. The
whole-core rule alone halves the ceiling on an SMT machine before the frame has
taken anything for itself, and exclusion-partitioning takes more on hardware
without cache or bandwidth controls. Pre-faulted, unmigratable huge pages are
memory the tiering machinery cannot touch and the quota system cannot
over-commit, so a hard-class component's memory is a fixed cost from grant to
release.

**Forecloses.** Work-conserving scheduling of reserved capacity: reserved and
idle stays idle, and no later optimisation may quietly lend it out. Overcommit,
ballooning and reclaim over hard-class memory. Background compaction that walks
a reservation. And it forecloses the comfortable position where every component
asks for the hard class — the soft class has to be the easy path and the
pleasant one, or the discipline evaporates in exactly the way the resource
document warns.

This is a cost and it is written here as a cost, beside the claim it buys,
rather than kept for a rebuttal after somebody runs a throughput benchmark and
finds cores sitting idle. They are meant to be sitting idle.

## What would reverse this

The exclusion half is the expensive half and it is the half with a measurement
attached. Collect claim 0002's workload on the E5 target machine twice: once on
a core reserved under this policy, and once on a core reserved without sibling
exclusion and without a cache partition, with a hostile neighbour on the sibling
and a cache-thrashing batch load co-resident. If the second run stays inside the
5 µs p99 bound over the same sixty seconds, sibling and cache interference fits
inside the frame's budget on that hardware, and the reservation should shrink to
core plus memory rather than continue charging for protection that buys nothing.

The opposite observation reverses it in the other direction. If a realistic
workload mix on that machine cannot obtain enough hard-class reservations to run
— if the anti-work-conserving cost is large enough that the soft class becomes
the only usable class in practice — then the whole-core rule is a per-machine
policy that admission consults, not a frame rule, and this RFC is superseded by
one that says which machines can afford it.
