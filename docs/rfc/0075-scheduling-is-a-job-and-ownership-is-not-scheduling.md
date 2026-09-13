# RFC 0075: Scheduling is a job, and ownership is not scheduling

- Status: accepted
- Date: 2026-09-13
- Affects: `kernel/src/component.rs`, which gains the call that hands a place's
  occupant a core; `kernel/src/process.rs`, whose `Job`, `Prepared` and `reap`
  this draws a line between and does **not** merge; `kernel/src/runtime.rs`,
  whose module comment states the gap this closes and whose stated reversal this
  is; RFC 0008 (restart is the supervisor's act), RFC 0033 and RFC 0047 (a
  driver is scheduled and never spawned into a place), RFC 0044 (`PLACES_MAX`,
  `SUPERVISOR_ORDER` and the account as one deviation) and RFC 0073 (the frame
  answers a supervisor's ring from a second server); `E1-B05`, `E1-P06`,
  `E2-B10` and `E2-P05`; and the declared quantities `HEAP_GAP` and `CHAOS_GAP`
  in `xtask/src/main.rs`, both of which this turns red on purpose. No task line
  in `TODO.md` names this RFC; that is reported to the originator rather than
  fixed by editing that file.

## Decision

A place's occupant is handed a core by writing a `process::Job` built from its
`Instance`, and it is **never** turned into a `process::Prepared`. Scheduling
and ownership are separated on purpose and stay separated: `Job` is the whole
of what a core needs to run something — an address space root, an entry, a
stack top, an argument and a timer bound — while `Prepared` and `Instance` are
two *ownership records* that differ in where the memory came from and where it
goes back to. `Prepared` holds frames taken straight from the `FrameAllocator`
and is consumed by `reap`, which returns them to it. An `Instance` holds handles
derived from an `Account` and is consumed by `tear_down`, which refunds them to
that account. Both can produce a `Job`; neither may be turned into the other.
We are choosing two ownership records and one scheduling interface over one of
each, and the price is that `Job` is built in two places.

## Context

`kernel/src/runtime.rs` has carried the sentence this closes since RFC 0033:

> That is the same shape RFC 0033 recorded for `virtio-blk`, from the other
> side: there, a component is spawned into a place and never scheduled; here, a
> component is scheduled and never spawned into one. Joining the two is a
> supervisor that sizes an account from what it was routed and then hands the
> occupant a core.

Both halves work and neither reaches the other. `component::spawn` builds an
`Instance` — an address space, a capability table, a control ring, a state tree
and a heap, every page of it derived from one supplied `Untyped` so that
revoking that `Untyped` ends the component (RFC 0008). `runtime::demonstrate`
builds a parallel thing through `process::prepare_runtime`, whose pages come
from `paging::user_space(frames, kernel)` and go back through `reap`.

The obvious join — and the one this RFC exists to refuse — is to give `Instance`
a method that produces a `Prepared`, since the scheduled path already knows how
to run one of those. **It is a double free wearing a convenience method's
clothes.** `reap` returns `Prepared::pages` to the `FrameAllocator` and checks
the free count it recorded in `Prepared::before`. An occupant's pages are not
the allocator's to get back: they were charged to an account, they are owed to
that account, and `tear_down` already refunds them. A `Prepared` built from an
`Instance` is a value whose entire purpose is to be passed to a function that
must never see it, and the type system would not say so.

What made the join small was reading what `smp::run_on` actually needs. It does
not take a process. It publishes a `Job` into a per-CPU slot and the target core
reads it — `{ root, entry, stack, argument, hz, target }`, six fields, and an
`Instance` has or trivially knows every one of them. The scheduling interface
was already narrow enough; what was wide was the assumption that running
something meant owning it the way `prepare_runtime` owns it.

### The alternatives that were live

**`Instance::into_prepared`.** Rejected above. The sharpest version of the
objection: `Prepared::before` is a free count, and an occupant's spawn did not
change the free count in the way `reap` would then assert it had.

**An account-aware `reap`.** One function, two return disciplines, chosen by a
flag or by a variant. Rejected because the two are not variants of one act —
returning frames to an allocator and refunding handles to an account differ in
who is owed, what is checked and what a failure means. Two names for two things
is what this tree does everywhere else, and `Failure::Leaked` exists precisely
because the frame checks its own count of what it built against what it gave
back.

**Scheduling the occupant through a third plan.** `Plan`, `RuntimePlan` and
`DriverPlan` already exist, and a fourth is the obvious shape. Refused for now:
the three plans exist because a process, a runtime and a driver are *judged*
differently — a tally of refusals, an absence of crossings, a device's fault
registers — and an occupant is judged by what its supervisor observes, which is
the thing being built. A plan minted before there is a verdict to shape it would
be a plan shaped by the first caller.

## Consequences

**Easy.** The occupant keeps its account for its whole life. Nothing about the
spawn path changes: the same `Untyped` pays, the same teardown refunds, and
`revoke` on the account still ends the component, which is RFC 0008's sentence
and would have been quietly false under an `into_prepared`.

**Easy.** Both halves of `HEAP_GAP` close in one step. That constant's needle is
`kernel/src/runtime.rs`'s *a component is spawned into a place and never
scheduled*; the day an occupant is handed a core, `user/store`'s 64-byte box
executes for the first time, the boot's `peak 0 byte(s)` stops being zero, and
`cargo xtask run` refuses the change and says it is the good ending. The check
written in a previous increment is the acceptance test for this one.

**Hard.** `Job` is now built in two places and they can drift. What stops the
drift is that `Job`'s fields are the *only* thing a core reads, so a field added
for one caller and not the other is a core entering with a value nobody set —
which is a fault at ring 3 and not a silent divergence. That is a weaker
guarantee than a shared constructor and a stronger one than a comment, and if a
third caller arrives the constructor is owed.

**Foreclosed.** A single ownership record for everything the frame can run. This
says there are two and that the second is not a special case of the first.

**What it makes visible.** `CHAOS_GAP` narrows rather than closes: its needle is
`prepare_driver(` in `kernel/src/blk.rs`, and a driver scheduled inside the
place its manifest is spawned into is the next thing this makes possible, not
something it does. `E1-P06`'s remaining half is exactly that.

## What would reverse this

**A third caller that needs a `Job`.** Two callers building one six-field value
is a duplication a reader can hold; three is a constructor that should have
existed, and at that point `Job::for(space, entry, timing)` is owed and this
section is why.

**An occupant whose memory is genuinely the allocator's.** If a component ever
legitimately runs on pages that were not charged to an account — a frame-owned
rescue shell, say — then the two ownership records collapse into one with a
flag after all, and the argument above is wrong rather than incomplete.

**`Job` growing a field only one caller can fill.** That would mean the
scheduling interface had stopped being about *what a core needs to run* and
started being about *what kind of thing is running*, which is the ownership
distinction leaking into the scheduling one. Measurable: any field on `Job` that
one of its two construction sites sets to a constant placeholder.
