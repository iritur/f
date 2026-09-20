# RFC 0095: An account defers a refund it cannot take from the top

- Status: accepted
- Date: 2026-09-20
- Affects: `kernel/src/cap.rs` (`Table::refund`), `kernel/src/component.rs`
  (`Place`, `tear_down`); RFC 0008, RFC 0016, RFC 0041, RFC 0044, RFC 0063;
  and `E2-B06`, `E2-P08`

## Decision

An account is a watermark. `Table::refund` gives back the top of it and
nothing else, and its own doc comment states the precondition that makes that
sound:

> this is sound because a place has one occupant at a time, so the frames an
> instance was made of are exactly the last ones retyped and nothing has been
> retyped since.

**That precondition is withdrawn**, because `E2-B06`'s *instantiate alongside*
requires a place to hold two occupants at once, and the moment it does the
outgoing instance's frames are no longer the last ones retyped — the incoming
instance's are. The repair is **not** the free list that comment names as the
general answer. It is narrower and it is bounded by the protocol:

> A refund the watermark cannot take is **deferred on the place** and applied
> when the frames above it have gone. A place carries at most one deferred
> refund, because `f_abi::swap` admits at most one incoming instance at a time.

`Table::refund` is unchanged and keeps refusing a refund that would take the
region below where it started. What changes is that `component::tear_down` no
longer assumes it may call it: it asks whether this instance's frames are the
top, and records the bytes against the place when they are not.

## Why not a free list per account

`Table::refund`'s comment is right that the general answer is a free list with
an owner and a quota, and right that the frame does not get to invent one on a
component's behalf. A free list is a structure whose size depends on how
fragmented an account has become, which is a policy question about a
component's own memory, and RFC 0008 puts that question in the supervisor.

This RFC does not answer the general question and does not need to. A swap is
two instances, not *n*: `f_abi::swap::Swap` is a state machine over exactly two
`Declaration`s, phase B is one machine word, and after it the outgoing occupant
is retired. So the number of live refunds an account can owe is one, the
structure is one `u64` on the place, and the ordering is the protocol's rather
than the allocator's.

**The bound is checked and not assumed.** A second deferral against a place
that already holds one is a refusal — `Failure::Account` — rather than an
addition, because two deferred refunds mean either a third instance or a
teardown that ran twice, and both are bugs whose symptom would otherwise be an
account quietly leaking memory it believes it gave back.

## What this costs, stated in the shape a reader will meet it

**An account can be at its floor and still refuse a spawn.** Between the
outgoing instance's teardown and the incoming instance's, the account's
`extent` does not reflect the deferred bytes: the memory is spent as far as the
watermark is concerned, and a component whose account is sized for exactly one
instance cannot spawn a third during that window. `user/virtio-blk`'s account
is 4 MiB against a 32-frame instance, so the window is wide there and the cost
is arithmetic rather than a wall. A manifest sized to the frame would meet it.

**A deferred refund is memory the component owns and cannot use.** It is not a
leak — the account gets it back, and `demonstrate`'s free-count check still
balances against the allocator — but between the two teardowns it is
unavailable, and a reader looking at `extent` during that interval is looking
at a number that is true about the watermark and misleading about the account.

## Why this is a reversal and not a detail

Because the sentence it withdraws is load-bearing in two other places.
`kernel/src/component.rs` orders its teardown "last charged first" and cites
that comment for why. `docs/rfc/0044` prices a place's account on the
assumption that an instance's frames come back whole when it dies. Both stay
true; what stops being true is that they are true *by construction*. After this
they are true because the swap protocol bounds the interleaving, which is a
weaker guarantee resting on a different argument, and a future change that
allowed two swaps to overlap would break it silently without this paragraph.

## The failure this was found by, which is worth keeping

The transfer window was charged to the account in `demonstrate`, after the
place had already spawned its first occupant. Tearing that occupant down
rewound the watermark over the window, and the successor was handed the
window's page as its board — reporting `stopped::NO_ROUTING`, a component
reading its board and finding somebody else's bytes.

Nothing refused. No capability was invalid, no mapping faulted, and the boot
log's only sign was a status word. That is what a watermark rewinding over live
memory looks like from the outside, and it is why this RFC exists rather than a
comment: the repair there was placement — the window is the *place's* and is
charged at `fill`, under every generation — and placement is exactly what
*alongside* cannot use.

## What would reverse this

**A swap that can overlap another swap at one place.** The bound here is one
deferred refund because `f_abi::swap` admits one incoming instance. A protocol
that pipelined two would make the deferral a list, and a list is the free list
this RFC declined to invent — at which point the honest answer is the one
`Table::refund`'s comment already names, with an owner and a quota, and it
belongs in the supervisor rather than the frame.

**An account that spends a deferred refund's interval at its floor in
practice.** The cost above is arithmetic while accounts are sized generously.
A manifest sized to one instance turns it into a refusal at exactly the moment
a swap is trying to finish, and the right repair is then to refund the outgoing
instance *before* the incoming one is spawned — which is the build this tree
had before *alongside*, and would mean this decision bought nothing.

**`E2-P08` observing an abandonment that costs a client its registrations.**
The whole point of instantiating alongside is that phase A stays reversible to
the end: the outgoing occupant is alive, so an abandonment resumes it. If a run
shows an abandonment after the incoming instance exists still costing a client
its table, then the two instances are not independent in the way this decision
assumes, and the deferral is hiding a coupling rather than enabling one.
