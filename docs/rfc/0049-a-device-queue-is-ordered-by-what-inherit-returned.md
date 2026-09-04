# RFC 0049: A device queue is ordered by what `inherit` returned

- Status: accepted
- Date: 2026-09-04
- Affects: `user/virtio-blk/src/pending.rs` (new), `user/virtio-blk/src/driver.rs`,
  `user/virtio-blk/src/component.rs`, `user/virtio-blk/src/routing.rs`,
  `user/virtio-blk/manifest.toml`; `kernel/src/blk.rs` and `kernel/src/main.rs`
  (three new halves and a boot parameter); `abi/src/manifest.rs`
  (`class::admitted`); `sim/src/dev.rs`, `sim/src/client.rs`,
  `sim/src/scenario.rs` (the `deadline` scenario); `xtask` (`deadline`,
  `DEADLINE_GAP`); `claims/0012`, `claims/0013`; `E1-B06`

## Decision

The block driver keeps a queue on **its own side** of the virtqueue, and what
leaves that queue next is whatever `f_abi::deadline::inherit` ranked first —
class, then deadline, then arrival. Nothing about the rule is decided here: RFC
0025 decided it and `abi/src/deadline.rs` is it. What this RFC decides is
*where* a resource scheduler applies it, *what it costs*, and *how a boot shows
that it happened*.

Four things follow, and each of them is a mechanism rather than an intention:

1. **The ordering is on the driver's side of the ring, because there is nowhere
   else.** A virtqueue is consumed in the order the driver posts. A request that
   has been offered belongs to the device and no scheduler above it can move it.
   So the only decision a driver has is *what to post next*, and
   `pending::Pending::take` is that decision.

2. **The cost is a number and it is published.**
   `f_virtio_blk::pending::IN_FLIGHT` is how many requests this driver keeps
   inside the device at once — one, because `Driver::execute` offers a chain and
   polls the used ring until it comes back — and it is the granularity of every
   overtake. A hard-class read arriving while `IN_FLIGHT` batch requests are
   already in the device waits for all of them, whatever its deadline says. The
   constant is written into the routing page, read back into the boot log, and
   named in `claims/0012`'s threshold, so a change to it is a change to the
   claim rather than a quiet loosening of it. R12.

3. **Every entry is admitted on the way into the queue, and a request served
   below what it asked says so.** `Driver::admit` calls `inherit` once, before
   the request has a rank at all; a class the submitting component was not
   admitted for is refused `ADMISSION`/`NOT_HELD` and never queued, and a
   request served at this driver's own class carries `cflags::SHORTFALL` on its
   completion. `user/virtio-blk/manifest.toml` declares the **soft** class, so
   on this tree every hard-class request the driver serves is served as soft and
   flagged — which is R08's sentence happening on the ordinary path rather than
   in a corner.

4. **The demonstration has a control that fails.** `cargo xtask deadline` is
   three boots of one client script against one driver, differing in the
   ordinals the frame writes into the driver's routing page: `ordered`, where
   the hard-class read submitted last must be handed to the device first;
   `arrival`, where the identical burst must put it last; and `unadmitted`,
   where a client admitted for the batch class writes `HARD` and must be
   refused. The overtake is read **twice** — the frame observes the order its
   own completion ring hands entries back in, the component counts what its
   queue put ahead of what — and the verdict requires the two to agree.

And two smaller decisions that a reader would otherwise re-litigate:

- **A hold, and it is called a fixture.** The frame tells the driver how many
  requests to accumulate before its first choice among them
  (`routing::at::HOLD`) and how many to serve before that applies
  (`HOLD_AFTER`). Without it, what is queued at the moment of a pick depends on
  how two cores raced, and the number the claim reports would be a different
  number every run — sometimes the right one for the wrong reason. It is told to
  the component for the reason `BEYOND` already is: a component choosing its own
  would be a demonstration choosing its own difficulty. It applies to one pick
  and is spent.
- **`admission_control` is still not declared in the manifest.** That file
  predicted `E1-B06` would add it. It does not, and the reason is R08 rather
  than an oversight: `f_abi::feature::ADMISSION_CONTROL` means *the consumer
  will refuse a deadline it cannot meet rather than accept one it will miss*,
  and this driver floors and reports instead — which is exactly what RFC 0025
  chose at the ABI. Declaring the bit would be claiming a refusal nothing
  performs. `E1-B07` is the task that builds a refusal, and the manifest says so
  where the prediction used to be.

## Context

What was true when this was decided. `abi/src/deadline.rs` held the rule and its
tests, and nothing ordered anything by it: `driver.rs` said in its own module
comment that `Sqe::deadline` and `Sqe::class` were *carried into the completion
untouched and order nothing*, and `user/virtio-blk/manifest.toml`'s `features`
list was empty for that reason. `RFC 0047` had just made the driver a scheduled
ring-3 component with its own polling loop, which is what made a driver-side
queue possible at all — before it, the loop that would hold the queue ran in the
frame.

The alternatives that were live:

- **Order in the device model and not in the driver.** The simulator's
  `Device::service` already picks which completion to publish next, and putting
  the rule there would have been one edit. Rejected because it is a model of the
  wrong thing: a real device does not read `Sqe::class`, it reads descriptors,
  and a scheduler that only exists where no hardware is would be a rule the
  boot cannot show and the sweep explores against nothing.
- **Order the virtqueue by rewriting the available ring.** A driver could
  reorder descriptors it has already offered. Rejected: the available ring is
  shared with the device, a device may have read any prefix of it at any moment,
  and a driver that rewrote it would be racing the hardware to save a
  microsecond. The bound in decision 2 is the honest version of what this would
  have bought.
- **Let the executor sort, with no separate queue.** `Driver::execute` could
  have taken a slice and chosen. Rejected because the choice is not the
  executor's: `pending.rs` is a scheduler and `driver.rs` is a transport, and
  one file doing both is the file nobody can change safely. It is also what
  makes the unit tests possible — the ordering is a pure function over entries,
  tested without a device.
- **Measure the overtake with the driver's counter alone.** Simplest, and it is
  a number a component publishes about itself. The client's own reading — which
  completion came back first — costs one array and cannot be forged by the
  component, and requiring the two to agree is what makes the pair evidence.
  This epoch has recorded four separate greens-for-the-wrong-reason; one number
  from one side would have been the fifth shape.
- **Run the burst and hope, rather than holding.** Rejected on determinism: the
  driver drains on another core, so without a hold the number of requests
  waiting at the first pick is a race, and a run that happened to queue nothing
  would report *the read came back first* while having overtaken nothing at all.
  The hold is a fixture and is named one.
- **A deeper queue, sized to the client's ring.** `Pending` was first written as
  `[Option<Waiting>; 16]`, which is what a client's ring holds. A component's
  stack is one page — `kernel::process::STACK` — a submission entry is
  sixty-four bytes and sixty-four-byte aligned, and the array overflowed into
  the guard page and killed the driver before it answered anything. The queue is
  eight, holds its entries and their ranks in parallel arrays, and carries a
  `const` assertion against a stated budget. The failure is recorded in
  `pending::CAPACITY` because it presented as *the driver did not answer a
  completion inside the bound*, which is five seconds of looking at the wrong
  thing.

## Consequences

**Easy.** A second service that has to order by deadline copies eleven lines:
call `inherit` on arrival, keep `(entry, Inherited, arrival)`, pick the minimum
of `rank()`, set `SHORTFALL` when `fell_short()`. Every bound RFC 0025 states
comes with it, because none of them is re-implemented. The simulator shares the
same call, so `E1-P02`'s fault classes and `E1-P03`'s sweep reach the ordering
for free: `sim`'s `deadline` scenario is one client whose urgent operations must
reach a busy device ahead of its own batch work, and every seed the sweep draws
is a seed against that too.

**Hard.** A driver now has state between requests, which a snapshot has to carry
— `sim/src/dev.rs` writes its pending queue out and reads it back, and a
component's would have to. The queue is bounded by a component's stack rather
than by its client's ring, so a client can have entries published that the
driver has not taken yet. And the frame has to know two ceilings and a floor to
fill in a routing page, which is plumbing `E1-B05`'s supervisor inherits.

**Forecloses.** A second ordering policy in a driver: there is one call and the
day a service re-derives the order from `Sqe::class` itself, it has opted out of
four bounds and nothing says so. A silent demotion: `answer` sets the flag for
every completion this service produces, on refusals as well as successes, so a
new branch cannot forget it. And a deployment that ordered by deadline *without*
saying what it cost: the in-flight depth is required to equal the constant the
claim is bounded by, so a driver that grew a deeper pipeline fails the boot until
the claim is re-derived.

## What would reverse this

- **A driver that waits on its interrupt.** `E1-B09` replaces the poll loop, at
  which point more than one chain outstanding is worth having, `IN_FLIGHT` stops
  being one, and the overtake gets coarser by exactly that factor.
  `claims/0012`'s `in_flight` threshold is what moves with it, and
  `batch_operations_overtaken`'s minimum is re-derived in the same diff; the
  rule does not move.
- **A component that can read a clock.** `DEADLINE_GAP` in `xtask` declares what
  no boot here shows: RFC 0025's third bound floors a deadline at *arrival plus
  the callee's floor*, and the arrival a component can supply is zero, so an
  absurd deadline still sorts ahead of an honest one at this driver. The
  simulator does exercise it, because the model's clock is the model's own. The
  day the literal zero in `component.rs` goes, the gap goes red and this
  paragraph is what to rewrite.
- **A service that forwards.** What this RFC settles is *bounds 1 and 2* — a
  request is served at the less urgent of the class it carries and the class
  this driver holds, and a class its submitter does not hold is refused. It
  settles nothing about **bound 4**, the depth decay, and that is now a declared
  quantity rather than a sentence: `DEADLINE_DEPTH_GAP` in `xtask` reads the
  tree for a caller of `Inherited::class_field` outside `abi/` and finds none.
  There is no chain here — the block driver is a leaf, every entry that reaches
  a scheduler was written at depth zero by whoever originated it, and
  `Inherited::rank` does not read the depth — so `inherit` runs the decay
  arithmetic and nothing in this tree can observe the result. The cheap way to
  close it is a `sim` scenario with two services in a chain; the honest way is a
  component that submits downstream on a caller's behalf, which is `E1-B05`'s or
  `E1-B07`'s. Either one makes the gap go red, and the diff that closes it owes
  a boot or a scenario in which a request loses its urgency at `MAX_DEPTH` —
  because a bound that is reachable and unexercised is worse than one that is
  neither. `docs/test-taxonomy.toml`'s `deadline-inheritance-unbounded` row is
  the same statement in the tree's own map and stays a gap until then.
- **A hold that stops being a fixture.** If a later boot can queue a known burst
  without being told to hold — a supervisor that starts a driver after its
  client has submitted, most likely — then `HOLD` and `HOLD_AFTER` are two
  fields the routing page does not need, and removing them makes the
  demonstration stronger rather than weaker.
- **Starvation the ordering caused.** `claims/0012` counts what the hard-class
  read overtook and says nothing about what the batch work waited. If a chaos
  run or a nightly sweep shows batch work measurably starved behind a legitimate
  hard-class client — every request inside its admission and inside RFC 0025's
  bounds — then the bounds are the wrong bounds, and that is RFC 0025's own
  reversal condition rather than this one's. What changes here is that the
  driver's queue needs a second key, and this RFC is superseded by the one that
  says which.
