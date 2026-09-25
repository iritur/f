# RFC 0137: The wake half waits for an event, not for a count to move

- Status: accepted
- Date: 2026-09-25
- Affects: `kernel/src/compositor.rs` (`waited_for_park`, `drive`,
  `drive_batch`, `Trouble::NotParked`, `Half::Wake`, one clause of
  `wake_verdict`), `kernel/src/doorbell.rs` (`wait` clears the latch after a
  halt), `user/compositor/src/` (`reported::PARKED_TAKEN`; the serve loop counts
  what it pops and publishes the count at every ask), `xtask`'s compositor
  harness (the timeout message). Corrects what `E3-B01g`'s record and RFC
  0107's row say the latch count measured and how long the timer rescues a lost
  doorbell.

## Decision

`cargo xtask compositor wake` waits, before each submission, for the component
to have **asked to stop its core with every entry it was sent already taken** —
`reported::PARKED_TAKEN`, the number of entries the component had popped at its
last ask, equal to the number the client has submitted. That wait has **no
clock**. It succeeds on the event, and fails with `Trouble::NotParked` only when
the component's outcome word says it left its loop without making the ask. A
component that does neither is holding its core for ever, and the harness's
boot timeout, which is already the bound on *never*, reports it. Separately,
the doorbell's wakeup latch is **cleared when a wait that halted returns**,
because the doorbell that ended the halt was consumed by ending it; and
`wake_verdict` requires the spared waits to be no more than the deliveries that
ended no halt.

## Context

The half failed intermittently with *the component never stopped its core, so
nothing was asleep*. Measured on `b47462d`, before any change: **9 of 20** runs
red, every one on that line. The bound was `EXIT_MICROS`, half a second of wall
time on the boot processor's timestamp counter, around a wait for
`reported::PARKED` to exceed a value.

**It was not slowness.** Before the batch, `drive_batch` read `PARKED` fresh and
waited for it to move once more, reasoning that *every entry above moved it and
what this needs is one more park after the last of them*. The component asks
within a turn of answering, so by the time the client read the count, the ask
after the last entry had usually happened and the core was halted. One more ask
then needed that halt to end, and nothing rings a component whose client has not
submitted. The only other waker is the occupant's timer, which
`component::OCCUPANT_TICKS` arms for sixty-four ticks at a thousand hertz and
then disarms. A run that reached the batch after those ticks waited on a core
that was asleep and would stay asleep, and reported that the component had never
stopped it. The half-second bound turned a hang into a red line. It did not
cause the failure, and a longer one would have waited longer for the same
nothing.

The cause was confirmed before anything was repaired. A pause of a tenth of a
second before the old read, which spends the timer's ticks and lets the
component halt first, made the half red **three runs in three**. The client's
own print said it had read the count at 15, 41 and 54 and was waiting for one
more, while `HALTED` said 6, 32 and 45. Passing runs showed the same thing from
the other side: up to 46 halts ended by something other than a doorbell, which
is the timer spending its budget.

**The latch was counting itself.** All eleven passing runs printed *8* or *9
wait(s) spared by a latched one* beside *6 to 10 of them ended by a doorbell*,
out of 10 delivered. `doorbell::answer` latches every delivery, including the
one that ends a halt, and `wait` did not clear the latch afterwards. So every
halt that a doorbell ended was followed by a spared wait that nobody had rung
for. `E3-B01g` read the count as the race the latch exists for (*the wakeup
latch fires nine times in twenty-eight waits*), and RFC 0107 repeats it. Most
of those spares were this instead. It also mattered to the wait above: the ask
the client saw after an answer was, most of the time, a spared one, so the next
submission reached a core that was still running.

The live alternatives were:

- **Raise `EXIT_MICROS`.** The half waits longer for a park that is not coming,
  and its result becomes a property of the host.
- **Carry the count from `drive` into `drive_batch`.** This fixes the batch, but
  each wait could still be satisfied by an ask made *before* the entry it
  follows was taken. *A client's commit woke a parked compositor* would stay a
  likely interleaving rather than something that happened.
- **Wake the core on purpose to get the extra ask.** That puts a ring in the
  harness, and the verdict would then count it.

A word naming the event is the only option whose truth does not depend on when
the client happened to read it.

## Consequences

The wait that was failing no longer depends on wall-clock time. What it waits
for is an event the component produces one turn after an answer the client has
already reaped, with no timer and no second waker in the path. After the repair,
**0 of 20** runs were red. On the final tree, with the verdict clause below,
**0 of 20** were red again. The tenth-of-a-second pause that made the old wait
fail three in three leaves the new one green three in three.

The counts now mean what their names say. The paused runs printed *10 of them
ended by a doorbell, 0 wait(s) spared* against 10 delivered. Every doorbell
ended a halt, so every submission reached a stopped core. Across the 40 runs
after the repair, 6 to 10 of the 9 or 10 deliveries ended a halt and 0 to 4
waits were spared. That spare count is now the race and nothing else, and a boot
still may not require it to be non-zero. `wake_verdict` gains one clause that
holds only of a latch that is consumed: spares no more than `delivered −
woken`, because a spare consumes a doorbell that no halt consumed. Every one of
the 20 runs before the repair broke it (9 spares against 0 to 4). Keeping the
latch after a halt, as the tree did before, turns it red.

The mutations that give the rest a run:

- **A component that never parks.** It is red on `NotParked`, through its own
  idle backstop and outcome word. It does not hang.
- **A component that publishes one entry fewer than it took.** It is red on the
  harness's boot timeout, whose message now names this case. That is the cost
  written below, observed.
- **`Ipi::to_self()`**, the control `E3-B01g` names. It is still red, but on
  *a core was given a place's occupant to run and never reported back*, not on
  the verdict's delivery clause. The timer that used to rescue the component
  through that mutation's run has run out of ticks before the run does.

What it costs: one more word on the board, written at every ask. A wedged
component, one that neither asks nor leaves its loop, is now reported by the
harness's 180-second boot timeout instead of a half-second one. That is slower
news about a failure that has not occurred outside a mutation, traded for no
news at all about a host's speed. `Trouble::NotParked` is now a finding about
the component and never about the harness, which is the opposite of what its
comment used to say. The completion wait in `drive` keeps its wall-clock bound.
Now that the timer's rescue runs out, it is what turns a lost doorbell into a
red line, and it did not fire in any of the 43 unmutated runs here.

What it forecloses: reading `reported::PARKED` as the thing to wait on. It is
still published, and the verdict still requires it to equal the halts plus the
spares.

**Two sentences elsewhere are now known to be too strong.** *The APIC timer
rescues a lost doorbell within a tick* (`E3-B01g`, RFC 0107) is true for the
occupant's first sixty-four ticks and false after them. After that, a lost
doorbell leaves a halted compositor with an entry on its ring, and the
completion bound reports it. And *the latch fires nine times in twenty-eight
waits* was mostly the latch outliving the halt it ended.

## What would reverse this

- **A non-blocking read of the worker's mailbox in `kernel/src/smp.rs`.** A
  component that faulted without writing its outcome would then be a red line
  in `waited_for_park` rather than a boot timeout, and the wait would use that
  as its failure bound too.
- **A waiter that does not look at its ring after every return from
  `f_abi::door::WAIT`.** That would reverse the latch clearing, because for such
  a waiter a consumed doorbell and a pending one would differ.
- **A timer armed for an occupant's whole life, or a tickless idle** (which RFC
  0107 names). Either changes which of the two sentences above is true. It does
  not change this decision.
