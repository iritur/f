# RFC 0038: A core is allocated to a runtime, and a kernel entry is counted against it

- Status: accepted
- Date: 2026-09-03
- Affects: `kernel/` (new: `runtime.rs` — `Allocation`, the reclaim shards and
  the demonstration; `process.rs` — `Entries`, four counting shards,
  `prepare_runtime`, `RING` and `WORK`; `arch/x86_64/idt.rs` — the three
  non-timer vectors count themselves; `state.rs` — one subtree and six
  nodes; `main.rs` — one stage behind a boot parameter), `abi/` (`Cqe::user_data`
  acquires a third reading, `control::reclaim` is new, `error::admission::RESERVED`
  is new), `xtask/src/main.rs` (`cargo xtask runtime`), `user/store/`
  (`report`, and the runtime that reports through it), RFC 0007 (whose
  reserved cores this refuses to reclaim), RFC 0008 (whose reclaim notice this
  builds), RFC 0016 (which this does *not* amend, and says why), RFC 0037
  (the safe adoption this stands on); `TODO.md` task E1-B08, whose exit this is
- Implements, and does not amend: `docs/design/deadline-all-the-way-down.html`
  section 02. The design page is ahead of the code by design and its sentence
  about allocating cores to runtimes is unchanged by this; what changes is that
  there is now something underneath it. Naming it under `Affects` would have
  claimed a diff this change does not contain.

## Decision

**The kernel allocates cores to runtimes. It counts what crosses into it while
a runtime holds one, in four buckets, and publishes all four.**

An `Allocation` is a set of cores and a reclaim promise per core. There is no
run queue in this kernel and adding one would be the reversal of this document.
Three rules govern it:

- **A core is held until it is given back.** A runtime schedules its own work
  inside its allocation with no kernel involvement, which is
  `deadline-all-the-way-down` section 02's sentence and is now a counted
  property rather than a design intention.
- **Preemption happens at allocation boundaries.** When the frame wants a core
  back it posts RFC 0008's reclaim notice — one per core, never one for several,
  and a promise that may only ever move earlier. The runtime acts on it at its
  next polling point and parks the work it already started. *Cleanly* is not the
  deadline it met; it is that its own queue was empty when it went.
- **A reclaim never names a core held under a hard-class reservation.** RFC 0007
  forecloses work-conserving scheduling of reserved capacity — *reserved and
  idle stays idle, and no later optimisation may quietly lend it out* — so such
  a reclaim is refused `ADMISSION/RESERVED` rather than served.

And the number that says the first rule holds: `process::Entries` counts door
calls, ring-3 faults, the one door call that ends a residency, timer interrupts
taken while ring 3 held the core, and every other interrupt taken while ring 3
held the core. **The hot path is the first two.**

### What the hot path excludes, and why, said before the number is quoted

An exclusion nobody can see is an exclusion nobody can check, so all five
buckets are published — `state::node::RUNTIME_HOT`, `RUNTIME_PROVOKED`,
`RUNTIME_BOUNDARY`, `RUNTIME_TICKS`, `RUNTIME_INTERRUPTS` — and a reader who
disagrees with where the line is drawn can move it.

**On the hot path.** A door call the runtime made, whether or not the frame
implements it. A fault it took, whether or not it meant to. Both are the code at
ring 3 reaching the frame in the middle of doing its work, which is exactly what
the architecture claims does not happen.

**Excluded: the `EXIT` that ends the residency.** It *is* the allocation
boundary rather than a crossing inside one. It is counted in its own bucket and
required to be **exactly one**, so a build in which the counting had stopped
publishes a zero there and fails rather than looking clean.

**Excluded: timer interrupts.** This is the arguable one and it is the one worth
arguing. A timer tick is the frame's own clock reaching a core it gave away.
Nothing the runtime does makes one happen or not happen; the rate is the frame's
and the count is a function of how long the run took. It is also, on the reclaim
boot, the *mechanism* by which the notice is delivered — which is the
distinction this whole model rests on, and it deserves to be visible rather than
summed away: **an interrupt happened and a preemption did not.** The runtime was
not rescheduled, its instruction stream was not redirected, and nothing it was
doing was abandoned. It found a completion entry at its next boundary and parked
there.

**Excluded: every other interrupt taken at ring 3.** The TLB shootdown another
core asked for, the doorbell, and the spurious vector the local APIC withdrew
between asserting it and having it acknowledged. The timer's argument covers all
three without being weakened by any of them: each is the frame, or another core,
reaching a core this one gave away, and nothing the code at ring 3 does makes
one happen. The spurious vector is the weakest member of that set — it is an
artefact of the interrupt controller rather than anybody's decision — and it is
counted with them rather than dropped, because a bucket with a judgement call in
it is still a number and a vector in no bucket is not.

**This paragraph is a scar, and saying so is the point of it.** When this
document was first written there were four buckets and this section ended
*nothing else is excluded; there is no third category*. That sentence was false
when it was written. `interrupt_dispatch` handled the shootdown, the doorbell
and the spurious vector by returning immediately, without so much as reading the
saved code selector — so each of them, taken while ring 3 held a core, was a
kernel entry in no bucket at all. `Entries::total()` claimed to be every
crossing and was not; the boot log printed *21 in all* and it was not all.

Nothing went red, and the reason is worth keeping: the demonstration's boot
processor only waits while the runtime runs, so it issues none of the three.
The `blk`, `cap` and `user` boots do issue shootdowns, and the counters are
armed on every process this kernel builds. A count that is complete only on the
boot that reports it is not a complete count, it is a coincidence — and the
hot-path zero would have survived any of the three arriving, which is precisely
what makes their absence from the total a thing a reader could not have noticed.

So: five buckets, and the fifth is provoked rather than merely counted. On the
reclaim half the frame rings this core's own doorbell from inside the timer
handler, immediately after posting the notice, and `Report::verdict` requires
the bucket to be non-zero. That is `MEMORY_FORCED`'s argument beside
`MEMORY_REMOTE` a fourth time — a counter nothing in a boot can move is
indistinguishable from a counter that does not work — and it is also the
sharpest form of this half's own claim. A doorbell is the interrupt most easily
mistaken for a preemption: it arrives from outside, it is about work, and it
lands mid-quantum. What it does is resume the runtime exactly where it was, land
in a bucket that is not the hot path, and change nothing about where the parking
happens.

Nothing else is excluded, and this time the sentence has a check behind it:
every arm of `interrupt_dispatch` that can be reached from ring 3 now counts,
and `Entries::total()`'s own comment says that its being every crossing is a
claim about that function rather than about the arithmetic. A sixth arm that
does not count is the way this becomes untrue again.

### Where the reclaim notice comes from

The frame is not running while the core it gave away is running, so a notice
posted before entry is one the runtime meets at its first polling point with no
work behind it. That demonstrates parking, and not parking *under load*.

So the frame posts it from inside the timer handler on the core the runtime
holds, once the runtime has finished a quarter of its load. It knows how far
along the runtime is because it reads the completion cursor of the runtime's own
work ring — memory the frame granted, read and never delivered, which is RFC
0013's shape applied to the one number that says how far along a runtime is.

**Progress and not time**, and the first attempt is worth recording because it
was wrong for a reason that belongs to the measuring environment rather than to
the mechanism. Posting after N timer ticks measured nothing: QEMU's translation
backend compiles each block of guest code the first time it is reached and the
local APIC's deadline is host time, so the same load took 24 ticks on one run
and 260 on another, and a tick threshold was sometimes before the runtime's
first polling point and sometimes after most of its work. A run reporting *zero
completed, everything parked* is a run that demonstrated the wrong thing.
The bound on parking is in work items for the same reason: a runtime parks in
microseconds of guest time and the emulator's first execution of the exit path
costs milliseconds of host time. The tick figures are still printed, because
latency is what somebody will eventually want; they are not gated on, because
under an emulator they are a measurement of the emulator.

**What the reclaim half is tuned against, which is a property of the harness.**
The notice is delivered from a timer tick, so that half needs at least one
ring-3 tick to land between a quarter of the load and the end of it. Under QEMU
the whole load takes tens of ticks — fifteen to fifty-odd across the runs
measured — so a tick inside the window is not close. On hardware, or on a host
fast enough, sixteen thousand items of a purely in-memory self-queue finish
inside a millisecond, no tick lands in the window at all, and the half goes
**red** with *no reclaim notice was posted, so nothing was parked*. That is the
harness failing rather than the mechanism, and it is written on
`RECLAIM_AFTER_ITEMS` so that the day it happens the reader is not debugging the
scheduler. It is not made robust here because every way of doing so changes what
is being measured: a shorter timer period makes the tick exclusion a different
number, and a larger load stops fitting the sixteen bits the tally gives it. The
reversal is a one-shot APIC deadline armed from the runtime's own progress,
which E5's hardware will need anyway.

### Why none of this is a fifth cross-core word

RFC 0016 names four `PerCpu<u64>` shards in `smp.rs` that two cores reach and
says a fifth needs an argument. The shards here are not a fifth, for the reason
`process::JOB`, `process::OUTCOME` and `cap::TABLE` are not either: the boot
processor writes them into an *idle* core's slot before the mailbox handoff and
reads them after it, so every access is ordered by the `Release`/`Acquire` pair
`smp` already owns. While the runtime runs, the only code touching them is the
timer handler on the core that owns them. Nothing here is a handshake, so
nothing here needs an atomic — and RFC 0016's own reversal condition, *a
protocol that cannot be expressed in a word; the candidate is a scheduler*,
remains unpaid because this scheduler moves no structure between cores.

## Context

What was true when this was decided.

`deadline-all-the-way-down` section 02 has said since before M1 that the kernel
allocates cores to runtimes and preempts only at allocation boundaries. Nothing
above the frame could do it, and the reason was not scheduling: driving a ring
means adopting a mapped channel and a `user/` crate may not write `unsafe`.
RFC 0037 is that half. This is the other.

Three tasks were waiting on it and each had written down what it was waiting
for. `E1-B05` left RFC 0008's restart policy in the frame and wrote
`component::policy::decide` as one function over a record and a tally, taking no
kernel state, so that moving it would be a move. `E1-B02` built a driver in a
crate that forbids `unsafe` and had the frame call `Driver::execute`, with
RFC 0033 stating the reversal as a grep. `E1-P06` needs a driver killed *under
sustained load*, and there was no load because there was no scheduler.

The alternatives that were live:

- **A run queue and a tick-driven scheduler.** What every other system does, and
  it is the thing this design is arguing against rather than a shape it can
  borrow. A runtime preempted mid-task cannot park cleanly by construction, and
  the whole of section 02 is that it should be able to.
- **A notice posted before entry.** Simpler by a long way, and it demonstrates
  the wrong thing: a runtime told at its first polling point has no work to
  park.
- **A door call for the runtime to ask whether it should yield.** One crossing
  per quantum, which is the measurement inverted. It is also `PROGRESS`, which
  RFC 0008 retires in favour of a blocking wait on a ring.
- **Counting only door calls and calling that the hot path.** Rejected because a
  page fault is a crossing the runtime did not choose, and a component that
  reached memory it was not given would publish a clean hot path while dying.
- **Subtracting the timer ticks and publishing one number.** Rejected under R12:
  a concession is written as a cost, never hidden in a metric. The four buckets
  are the cost written down.
- **Making the reclaim a signal.** Rejected by RFC 0008 before this task
  existed, and its reasoning is the reason the notice works here: *reclaim is
  exactly the case that needs a deadline rather than an interrupt, and an
  interrupt is the mechanism that makes cleanly impossible.*

## Consequences

**Easy.** A component runs at ring 3 with its own polling loop, which is the
sentence three tasks were waiting for. `cargo xtask runtime` is four boots and
the exit criterion is one of them: sixteen thousand work items through a
component's own executor, **zero kernel entries on the hot path**, one crossing
at the allocation boundary, and the timer ticks published beside them. RFC 0008
finally has a component *acting* on a notice rather than the frame draining one
on its behalf.

**And the zero can be moved, which is what makes it a measurement.**
`runtime=provoke` runs the same load and makes one door call in the middle of
it. The frame requires the hot-path count to be non-zero **and to equal what the
component says it made** — two numbers taken on opposite sides of the boundary —
so a build in which the counting had stopped publishes zero on both halves and
fails. That is the shape `blk copies` and `blk provoked` already have, and
`state::node::MEMORY_FORCED` before them. The counters are armed on every
process this kernel builds and not only on a runtime's, so `user/init`'s four
door calls are counted by the same code on every boot.

**Hard.** The frame reads a component's own ring cursor to know how far along it
is. That is memory the frame granted and the read costs the runtime nothing, but
it is the frame knowing something about a component's internals, and the
honest version of it is a state tree the component publishes under RFC 0013.
The reversal is written on the shard.

The parking bound is in work items and the latency is not gated on. That is the
right call in an emulator and it is a gap: nothing here says a runtime parks
*fast*, only that it parks within one quantum of being told. E5's hardware is
where that number becomes real, and `PARK_WITHIN_ITEMS` is where to put a time
beside it.

**A defect found and not fixed, named rather than left.** `f_abi::door::Entry`
tells a component its first handle and lets the rest follow by index *at the
first handle's generation*. That arithmetic is sound only while every process
shape the frame builds grants the same number of capabilities, because a slot
advances a generation each time it is cleared and refilled. A runtime granted
three left slot three a generation behind slots zero to two on a core that then
ran an ordinary process, and the fourth handle that process was told it held
resolved to nothing — presenting as a component refusing to map the state tree,
which is about as far from the cause as a symptom gets. `prepare_runtime` grants
four, and the paragraph beside it says why the count is load-bearing. The
structural fix is for `Table::clear_all` to raise every slot to the table's
generation floor; it is not made here because
`kernel/src/arch/x86_64/probe.rs` names `Handle::FIRST_GENERATION` as a literal
in `cap=unowned` and counts refusals by generation in `cap=forge`, so raising
the floor moves what E0-P08's negative suite is asserting. That is a task with
those two fixtures in it, and it is not this one.

**What is still true and was supposed to stop being true.** `kernel/src/blk.rs`
still calls `f_virtio_blk::driver::Driver::execute`, and RFC 0033's reversal
condition names exactly that grep. What this task delivers is the *scheduling*
half: a component runs at ring 3 and drives its own rings. What it does not
deliver is the routing half for a driver — four device register windows and a
DMA region mapped into a spawned component's address space, with the IOMMU
domain programmed for it — which is a mapping job in `kernel/src/blk.rs` and
`kernel/src/arch/x86_64/virtio.rs` rather than a scheduling one. Saying that
plainly is better than a task that claims a grep it did not move. The same is
true of `component::policy::decide`: it still runs in the frame, it still takes
a record and a tally and no kernel state, and what it now lacks is a supervisor
component to be moved *into* rather than a mechanism to be moved *by*.

**A sentence that was wrong for a day.** The four-bucket version of this
document said *nothing else is excluded*, and three interrupt vectors were.
It is recorded above rather than quietly corrected, because the failure mode is
the interesting part: the number that was wrong was not the hot-path zero — none
of the three is a runtime's work reaching the frame — it was the *total*, which
is the number whose whole job is to make the exclusions subtractable. An
exclusion you can see is checkable; an entry in no bucket at all is not even an
exclusion.

**Forecloses.** A run queue in the frame. A scheduler that migrates a task
between cores, which RFC 0016 names as the thing that would need a structure
rather than a word. A reclaim of a reserved core. And the position that the
hot-path number can be published without its exclusions: they are four nodes in
the state tree and a section in this document, and a later build that summed
them would be deleting evidence rather than simplifying a report.

## What would reverse this

**A runtime that cannot park inside any bound the frame can name.** RFC 0008
already wrote this one and it is the right one: *then reclaim degenerates into
preemption, and the resource document's user-level runtime model is what is
wrong, not the notice.* The measurement exists now — how much further a runtime
got after it was told, in work items — and the frame's count of reclaims that
became preemptions is its twin. Today that count is structurally zero because
nothing preempts; the day something does, it is a node.

**A quantum that has to be small enough to make parking prompt, and large enough
to make the work worth doing.** The quantum is eight work items here and the
choice was free. On a real workload the two pressures are opposed — a small
quantum parks quickly and drains the control ring constantly, a large one gets
work done and holds a core past its deadline — and if no single value serves
both, then the quantum is a *declared* property of a component rather than a
constant in its runtime, and the manifest is where it goes.

**Polling costing more than the delivery it replaces.** RFC 0008's own reversal
condition, and this is the first build that can measure it: draining the control
ring at every allocation boundary is a load from a mapping, and E1-P10's
kernel-entries-per-operation claim is where it would show. If it does, the
answer is not signals — it is E1-B09's suppression and user-interrupt doorbell
applied to the control ring, which this design leaves available precisely
because the control ring is a ring like any other.

**The timer stopping being the frame's only way to reach a core it gave away.**
The reclaim is posted from the timer handler because there is nothing else. On
hardware with user interrupts, or on a machine where the frame keeps a core of
its own, the frame could post a notice without interrupting anything — at which
point the timer-tick exclusion above stops being about delivery and becomes
purely about timekeeping, and this document's second exclusion should be
re-argued rather than inherited.

**A component that legitimately holds more than one core.** Everything here is
written for it — the allocation is per core, the promise is per core, and the
notice names one core — and none of it is *exercised* for it, because the
demonstration holds one core and reserves a second only so that a reclaim has
something to be refused about. The day a runtime holds four, the thing to check
is the one RFC 0008 names: that reclaiming core 3 and then core 7 before a drain
is two facts and neither displaces the other. If it turns out that a runtime
cannot usefully act on them one at a time, the notice is the wrong granularity
and the answer is a notice that names a set.
