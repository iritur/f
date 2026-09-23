# RFC 0107: The eighth door call is a component asking to stop

- Status: accepted
- Date: 2026-09-23
- Affects: `abi/src/door.rs` (`WAIT`), `kernel/src/process.rs` (`SYS_WAIT`, and
  the sentence this reverses), `kernel/src/doorbell.rs`, RFC 0014

## Decision

`f_abi::door::WAIT = 7` is the eighth call on the door. A component that has
nothing to do asks the frame to stop its core until something arrives; the frame
answers `Park`, and `kernel::doorbell::wait()` does `sti; hlt; cli`.

`kernel/src/process.rs` says, of the seven that were there: *adding an eighth
means arguing against both documents in writing, which is the intended cost.*
This is that argument.

**`hlt` is a ring-0 instruction with no unprivileged equivalent.** RFC 0014
narrows the door to what a component cannot do for itself, and this is the
cleanest example the tree has produced: a compositor that spins between frames
holds a core at full rate, and there is no instruction at ring 3 that stops it.
The alternative is not a different call — it is a component that busy-waits, and
`E3-B01h`'s pacing exists to make frame timing measurable, which a core pinned at
100% makes impossible.

It is also the call RFC 0014 predicts rather than forbids. That entry's own
reversal names the day `ANNOUNCE` and `PROGRESS` retire because a component is
started *with* a channel and told on it — which happened (RFC 0076) — so the
door's population was going to move. This adds one where two are owed removal,
and `lint-owed` still reports both.

## Context

The mechanism, because the hard part is not the call:

A component's idle turn **arms the data ring's wakeup flag, looks again, and only
then calls `WAIT`.** That order is the whole of the lost-wakeup argument: a
doorbell that lands between the last look and the `syscall` sets
`doorbell::PENDING`, which `wait()` test-and-clears, so the core answers `AWAKE`
rather than halting for a ring nobody will repeat.

**No fifth cross-core word was needed**, which is the constraint CLAUDE.md puts
on `kernel/src/smp.rs` and which this task was written to respect. The four new
per-core counters are `Relaxed` — they publish nothing, and an ordering there
would name an edge nothing depends on, which is `smp::bump`'s own argument. The
counters are read off the worker core **only after `smp::join_serviced` returns
`Ok`**, so the happens-before is the mailbox `Release`/`Acquire` pair `smp`
already pays for and already tests. Nothing in `ring/src/lib.rs`'s ordering was
touched, so no litmus test is owed.

## Consequences

**What this makes easy.** A compositor that sleeps. `cargo xtask compositor
wake` shows a client on the boot processor waking a parked compositor on core 1
— `core 1 halted 19 time(s), 9 of them ended by a doorbell` — which is cross-core
delivery observed for the first time, deferred to this line by `E0-B15` rather
than narrowed.

**What this makes hard.** Every future door call has to clear a bar this one
just raised, and that is the intent.

**What is argued and not observed, stated so nobody reads the boot as more than
it is.** The wakeup latch is correct by construction and fires nine times in
twenty-eight waits, and **nothing in this tree kills it**: the APIC timer is
armed on any core running a process, so a lost doorbell is rescued within a
tick. The `Ipi::to_self()` mutation measured exactly that — zero doorbells
delivered to core 1 and the component still completed, on twenty-five
timer-ended halts. Asserting that the race occurred is not something a boot may
require, so the latch's necessity rests on the memory model and on RFC 0020's
shape.

## What would reverse this

**A core with no timer.** The latch's untestability today is entirely because
something else rescues a missed doorbell. A tickless idle path — which is what
RFC 0006's computed idle depth eventually wants — removes the rescue and makes
the latch the only thing standing between a parked core and a hang. That is the
day this becomes observable, and the day to write the test.

**An unprivileged wait.** `UMWAIT`/`TPAUSE` exist on some x86-64 parts and stop a
logical core from ring 3. If the machine `E5-D01` names has them, the argument
above loses its load-bearing sentence — *there is no instruction at ring 3 that
stops it* — and the call should be reconsidered rather than kept out of habit.
That is also the reversal RFC 0014's narrowing asks for by name.
