# RFC 0020: Notification suppression needs a StoreLoad fence

- Status: accepted
- Date: 2026-08-31
- Affects: `ring/src/lib.rs`, `ring/tests/litmus.rs`, `docs/design/ring-scene-boot.html` section 03, and `E0-B15`, which is the task that would first have hit it

## Decision

The producer places a `SeqCst` fence between publishing an entry and reading the
consumer's `NEED_WAKEUP` flag. The consumer's half of the same barrier already
exists — `Consumer::arm_wakeup` is a `SeqCst` read-modify-write, which is a full
barrier — and this is the missing half.

Without it the suppression protocol has a lost wakeup. It is not a data race:
every value read was legitimately written, no sanitiser reports it, and the ring
stays perfectly consistent. It is a hang.

## Context

The two ends run Dekker's algorithm at exactly one place. The producer **stores**
`head` and then **loads** `flags`; the consumer **stores** `flags` and then
**loads** `head`. A store to one location followed by a load of a *different*
one is the single reordering total store order permits — the store sits in the
store buffer while the load is satisfied ahead of it — and `Release` and
`Acquire` do not forbid it. They are one-way barriers. This needs a two-way one.

The bad interleaving is both ends looking and seeing nothing:

1. The producer writes the entry and stores the advanced `head` with `Release`.
2. Its load of `flags` is satisfied out of order, before that store is visible,
   and reads `NEED_WAKEUP` clear. It concludes the consumer is draining and
   rings nothing.
3. The consumer sets `NEED_WAKEUP` and re-checks the ring — the second check the
   design document already prescribes — and its load of `head` does not yet see
   the producer's store. It concludes the ring is empty.
4. It sleeps. The entry is stranded and nothing will come for it.

The second check was believed to close this race, and it closes half of it. It
closes the case where the consumer's *flag write* is late. It does not close the
case where the consumer's *ring read* is early, because nothing ordered the
producer's two accesses against each other.

What was true when this was decided: nothing in the tree sleeps. Both ends of
the frame's channel are the kernel and it drains synchronously, so the defect
had never had an opportunity to fire. `E0-B15` is the task that gives the
doorbell somewhere to ring, which is the point at which a latent hang becomes a
hang, and it is why this was looked at now rather than after.

Two alternatives were live.

**Make the producer's `flags` load `SeqCst`.** It works, and it was rejected for
being less honest rather than for being slower: a `SeqCst` load compiles to a
plain `mov` on x86-64 and buys nothing there, so the ordering that actually
fixes this would be invisible at the point that needs it. The fence is a `mfence`
you can see, next to the comment explaining which reordering it forbids.

**The event-index design**, which virtio uses and which the design document
already cites: instead of a flag saying *wake me*, the consumer publishes the
cursor position it wants to be woken at, and the producer compares rather than
tests. That removes the algorithm rather than fencing it, and it is a better
answer. It was rejected *for now* because it is an ABI change to
`ChannelHeader`'s layout and to `chan::`, and making one under a defect is how a
wire format acquires a shape nobody chose.

## Consequences

A store-load fence on the submission path, paid once per `submit` and once per
`Batch::publish` — once per *batch*, not once per entry, which is the same
amortisation batching already exists for. `claims/0001` measures that path, and
its target was written before the fence existed; if the claim comes in near its
bound, this is one of the two things to look at, and it is a correctness cost
rather than a tunable one.

It also fixes the number `E0-B15` will report. Doorbells-per-operation counted
on a build with this defect would have been *lower* — a producer that wrongly
believes the consumer is awake rings less — so the missing fence would have
flattered exactly the metric the doorbell work exists to publish.

`ring/tests/litmus.rs` gains
`a_sleeping_consumer_is_never_left_holding_work`, and it is the first test in
that file whose defect is observable on x86-64: store-load is the reordering
that architecture performs, so this does not wait for the AArch64 runner. With
the fence removed it reports **58 971 lost wakeups in 500 000 rounds, first at
round 69** — an eight-in-a-thousand-rounds hang, not a rare interleaving.

The harness had to change to see it. A `std::sync::Barrier` ends in a futex
wakeup, which takes microseconds, and the window is one store buffer deep; two
threads lined up that loosely are never inside it together, and the test would
have reported a clean run on a broken build.

## What would reverse this

The event-index design landing, at which point the algorithm this fence protects
no longer exists and removing the fence is part of that change rather than a
weakening of this one. That is an ABI change and belongs with one.

Short of that: a measurement showing the fence is a material share of submission
cost on the machine `claims/0001` is taken on. That would not make the hang
acceptable — it would make the event index urgent.
