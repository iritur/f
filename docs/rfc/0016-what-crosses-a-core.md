# RFC 0016: Four words cross a core boundary, and nothing else does

- Status: accepted
- Date: 2026-08-30
- Affects: `kernel/`, `CLAUDE.md`, `docs/design/ring-scene-boot.html` section 14

## Decision

`ring-scene-boot` section 14 and `CLAUDE.md` both say that every mutable
`static` under `kernel/` is a `PerCpu<T>`, **so two cores never reach the same
slot and nothing there locks**. The second half of that sentence stops being
true at E0-B10, and this records exactly how far.

Four `PerCpu<u64>` shards in `kernel/src/smp.rs` are reached by a core that does
not own them: a mailbox, and a page, a sequence number and an acknowledgement
for the translation-buffer shootdown. Every one of them is a machine word, in
the slot of the core it is *about*, and every access on both sides is an atomic
with its ordering named at the access. Nothing else changes. There is still no
lock anywhere under `kernel/`, and a shard holding anything larger than a word
is still a shard exactly one core touches.

The rule as amended: **a shard is one core's, except for a word two cores
handshake through, and a handshake word is an atomic on both sides.**

## Context

Bringing up a second core needs two things a single-core kernel does not.

A core has to be told to do something and has to say when it is done. That is a
handshake, and a handshake cannot be per-core state by definition: somebody
writes where somebody else reads, or nothing is being communicated.

And a page taken out of a page table is still in every other core's translation
buffer until that core is told. Telling it is an inter-processor interrupt and
an acknowledgement, which is the same shape again.

Three alternatives were live.

**A lock.** Rejected for the reason section 14 gives: a lock in a per-CPU kernel
is a confession that the sharding is not believed, and one lock is how a kernel
acquires a second. Nothing here needs mutual exclusion anyway — the protocols
are a word written by one core and read by another, which is what atomics are
for.

**A separate module of shared state, outside `PerCpu`.** This is what most
kernels do and it is what `cargo xtask lint-percpu` exists to prevent. It would
have meant a `static AtomicU64` under `kernel/`, which is precisely the text the
lint greps for, and widening the lint to allow it would have widened it for
everything.

**Volatile reads and writes rather than atomics.** This is what `apic::TICKS`
and `process::IN_RING3` already do, and it is right *there*: those are a handler
and the code it interrupted, on one core, where the only hazard is the compiler
eliding an access. Across cores the hazard is different and volatile does not
address it. Volatile says "do not remove this access" and says nothing about
ordering, and ordering is the entire content of both protocols here — the
mailbox publishes a handoff structure, and the shootdown publishes a page table
edit. `Relaxed` would pass on x86-64, whose total store order hides it, and
corrupt on AArch64. That is the same trap `CLAUDE.md` already records about the
ring.

## Consequences

Easy: the shootdown is expressible at all, so revocation can reach a mapping.
That closes the largest gap in the capability system — `kernel/src/cap.rs`,
`process::revoke` and RFC 0015's risk section all said a revoked frame
capability withdrew the name and left the translation — and it closes it with a
real acknowledgement rather than a hope.

Also easy: a process can run on a core that is not the one holding the timer,
which is what makes "the timer kept its schedule while ring 3 held a core" a
statement about two cores rather than about one core's transitions.

Hard: the amended rule is a rule with an exception in it, and exceptions grow.
The defence is that the exception is stated in one file, is four words wide, and
is checked by the same lint as before — `lint-percpu` still fails on a `static`
carrying an atomic, because these are `PerCpu<u64>` and the atomic is created at
the access through `AtomicU64::from_ptr`. That is deliberate: it means adding a
fifth cross-core word is a diff in `smp.rs` that a reader can see, rather than a
new `static` somewhere else.

Foreclosed: nothing. A later kernel that genuinely needs shared mutable
structures — a run queue, a global frame pool — has to argue for them against
this, which is the intended cost.

## What would reverse this

A protocol that cannot be expressed in a word. The candidate is a scheduler:
migrating a task between cores means moving a structure, not signalling a flag,
and at that point either the structure is copied through a ring — which is what
this system is for — or the rule needs a bigger amendment than this one.

The other reversal is measurement. If the shootdown acknowledgement turns out to
dominate the cost of a revoke under real load, the answer is batching, which
means a queue of pending invalidations per core, which is a structure and not a
word.
