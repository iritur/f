# RFC 0025: A deadline inherits downward and decays

- Status: accepted
- Date: 2026-09-02
- Affects: `abi/src/deadline.rs` (new), `abi/src/lib.rs` (the reading of
  `Sqe::class`, `cflags::SHORTFALL`, `error::admission::NOT_HELD`,
  `error::argument::BAD_CLASS`); `docs/design/deadline-all-the-way-down.html`
  sections 02 and 09; `E1-B06`, which orders device queues by what this
  decides, and `E1-B07`, which grants the admission this bounds by

## Decision

A request that crosses a ring carries its caller's class and deadline, and the
service acting on its behalf orders its own queues — and its own downstream
submissions — by them. That inheritance is bounded in four ways, each of which
is arithmetic in `abi::deadline::inherit` rather than a policy a scheduler
applies:

1. **By the callee's admission.** A request is served at the less urgent of the
   class it carries and the class the callee was admitted for. A hard-class
   read arriving at a soft-class service runs as soft, against the same
   deadline, and the completion says so. It is never promoted: a batch request
   arriving at a soft-class service runs as batch. The callee's class is a
   ceiling, and a ceiling that was also a floor would make every service
   promote whatever it touched.

2. **By the caller's admission.** An entry whose class is more urgent than its
   submitter holds on that channel is *refused* — `ADMISSION`/`NOT_HELD` — not
   demoted. The ceiling a component holds is declared in its manifest and
   granted at spawn: a hard-class reservation after RFC 0007's test, a soft
   standing as a right the supervisor routes, batch for a component that
   declares nothing. It reaches the service as a fact about the channel and
   never as a field of the entry, so the entry cannot raise it.

3. **By time.** An inherited deadline is never earlier than the request's
   arrival plus the callee's floor — its worst-case service time — and,
   outside that floor, never later than the caller's. A deadline already in
   the past, or inside the floor, is floored and the completion says so. This
   is what makes a deadline of one nanosecond worth exactly as much as an
   honest one: without it, an absurd deadline sorts first in every queue and
   the field is a priority with a better name.

4. **By depth.** The caller's urgency reaches `MAX_DEPTH` rings — four — from
   the component that originated the request, counted in the high byte of
   `Sqe::class`, and ends there: the request continues as batch work, whoever
   sent it, with no deadline, and the counter saturates so nothing downstream
   can restart the chain. A depth past the bound is a value no conforming
   service writes and is refused as malformed.

And urgency has a scope. What `inherit` returns is a property of *one request*,
held by the service while the request is in flight and dropped at its
completion. A component's own work has the component's own class and no
deadline, whatever it served a moment ago; a request it once carried leaves
nothing behind. Admission control (`E1-B07`) is the other half of the same
sentence: a reservation that would let a component be urgent without a period
and a budget is a reservation it refuses.

Every way a request is served below what it asked — class, time, depth — is
reported on the completion as `cflags::SHORTFALL` and counted by the service in
its state tree. Served differently is a fact the caller can handle; served
differently silently is the failure the whole discipline exists to exclude.

## Context

`deadline-all-the-way-down` section 02 says why the submission entry has a
deadline field at all: a task that blocks on another component's completion
should not wait behind batch work in that component's queue, so the deadline
travels with the request through every ring it crosses and each resource
scheduler orders by it. Section 09 of the same document, listing where the
design is weakest, says the mechanism can be gamed — a component that always
claims urgency wins — and that bounding inheritance by the caller's own
reservation "needs designing before it is implemented rather than after the
first starvation bug." `E1-D05` is that sentence made a task, and `E1-B06`, the
task that makes every device queue order by the field, is blocked on it.

What was true when this was decided: nothing in the tree read `Sqe::class`. The
field's own documentation described a class in the high bits and a priority
ordinal in the low bits; the four `class` constants are 0 through 3 and
`user/init` writes them whole; and `Sqe::ZERO` writes `BATCH` rather than zero
without saying why. R03's lint had made the field state a unit but not a
reading, and the two failures the rule prevents were both a scheduler away:

- **Starvation of batch work behind a permanently urgent client.** A soft-class
  compositor that stamps every request with a deadline of *now* — or a
  component that writes `HARD` because nothing refuses it — sorts ahead of
  storage compaction in every queue, forever, and compaction never runs. Bound
  1 and bound 2 close the class half of this; bound 3 closes the deadline half;
  the scope rule stops the service that carried the requests from becoming a
  second such client.
- **A hard-class read stuck behind batch work in a device queue.** The
  inversion the field exists to prevent. Inheritance closes it — but only
  bounded inheritance closes it *without opening the first failure*, which is
  why the two are one RFC.

The alternatives that were live:

- **Inherit nothing; every service runs at its own class.** Simplest, and it is
  the inversion the resource document's section 02 rejects: a hard-class task
  waits behind batch work in a storage queue and misses its deadline through a
  resource it never contended for directly.
- **Priority inheritance, the classic answer.** A holder is raised to the
  priority of whoever waits on it, across whatever chain forms. Rejected
  because it is a property of *components* rather than of *requests*: the
  holder stays raised while anything waits, unrelated requests are promoted
  because they share a queue with an urgent one, and there is no depth. It is
  the shape that made deadline scheduling decorative elsewhere, and it is what
  the scope rule is written against.
- **Unbounded propagation with honest peers.** The deadline travels as far as
  the request does, and components are trusted to claim only what they hold.
  This is "we will be careful about X", which R01 says is a plan. The hostile
  peer `ring-scene-boot` section 06 already assumes can write any class it
  likes; a rule that depends on it not doing so is no rule.
- **Bound by depth alone, or by class alone.** Each leaves one of the two
  failures open. Depth without the caller's admission lets a component be
  urgent for four rings on every request; class without depth lets a
  legitimately hard-class component's urgency reach the whole system. Both
  bounds, plus the time floor, are the smallest set that closes both failures.
- **Refuse rather than floor a deadline the callee cannot meet.** Under R08 a
  hard-class deadline the callee cannot promise is a candidate for
  `ADMISSION`. Not adopted at this layer: a request already in flight has
  already been admitted end to end by whoever admitted its caller, and a
  refusal here would fail the caller's deadline as surely as serving late — a
  service may still refuse, and a floored deadline that is also flagged is what
  gives it the information to. The ABI rule floors and reports; whether a
  particular service refuses on `LATE` is that service's policy, stated in its
  manifest.
- **Demote rather than refuse a class the caller does not hold.** Serve at the
  ceiling, set the flag. Rejected because a caller that loses nothing by
  writing `HARD` writes it on every entry, and a flag that is always set is a
  flag nobody reads. Refusal costs the lying caller its request, which is the
  cost that makes it stop.
- **Reset depth at the bound rather than saturate it.** The request past the
  bound is "the callee's own work", so its downstream entries start at depth
  zero and carry the callee's own class. Coherent, and it makes `MAX_DEPTH` a
  bound on one component's reach rather than on the chain — and the chain is
  what the starvation bug is made of. Saturation is the stricter reading and
  the one whose behaviour a reader can predict from the constant alone.
- **A depth field of its own, or the reserved word.** `_reserved` is zero under
  R04 and spending it is an ABI version under RFC 0011; a new field the same.
  The high byte of `class` was already documented as holding something other
  than the class, had no reader, and has room for a count sixty-three times
  larger than the bound. The packing costs nothing on the wire and the four
  constants keep meaning what they meant — `class::SOFT` is a depth-zero field
  of itself.
- **Keep the priority ordinal.** The field could hold depth *and* a priority.
  Retired instead, and not as a side effect: the resource document's scheduler
  primitive is `(class, deadline, energy cost)` precisely because priority
  conflates urgency with importance, and a priority sub-field would have put
  back the one number the design refuses to collapse into. Within a class, the
  deadline is the order and `NO_DEADLINE` sorts last.

## Consequences

**Easy.** `E1-B06` is a queue keyed on `Inherited::rank()` and a service loop
that calls `inherit` on arrival, forwards `class_field()` and `deadline`
downstream, and sets `SHORTFALL` on the completion when `fell_short()`; the
ordering it must measure — a hard-class read overtaking queued batch work — is
already a unit test here, as a rank comparison. `E1-B07` has a definition of
what it is granting: the ordinal a channel's `Admitted` carries. The simulator
can inject a peer that writes `HARD` everywhere, or a depth past the bound, and
assert the refusal by domain rather than by symptom. A shortfall is a counter
in a state tree, so the day a service is being served below its class is
visible on a dashboard rather than in a post-mortem.

**Hard.** Every service must know its own floor — a worst-case service time it
stands behind — and a service that says zero has disabled bound 3 for itself.
Every channel must carry the submitter's admitted class from the grant to the
service, which is plumbing `E1-B05`, `E1-B06` and `E1-B07` share and none of
them owned before. The hard class is only ever as deep as the shallowest
admission on the path: a hard-class caller whose storage service is soft-class
gets soft-class storage and a flag, and the fix is to admit the service, not to
lower the bound. And a chain deeper than four rings loses its deadline at the
fifth, which the topology of this epoch never reaches but a later one might.

**Forecloses.** Priority inheritance across unrelated requests: nothing here can
raise a component, only a request. Unbounded chains: four rings and the counter
saturates. A component setting its own class: the ceiling comes from the grant
and an entry above it is refused. A priority ordinal inside the class field. A
deadline in the past as a way to the front of a queue. And silent demotion,
anywhere: a service that serves a request below its class without the flag is
in breach of this RFC, and the state-tree count is how a reader finds out.

## What would reverse this

- **A legitimate chain deeper than four.** A request in the tree's topology
  that crosses more than `MAX_DEPTH` rings and needs its deadline at the far
  end, root-caused to the topology and not to a service that should have been
  merged. That raises the constant by an RFC naming the chain; it does not
  remove the bound.
- **A starvation bug the bounds did not prevent.** Batch work measurably
  starved under `E1-P06`'s chaos load or a nightly sweep, with every request on
  the path inside its admission and inside the bounds. That means the bounds
  are the wrong bounds — most likely that bound 3's floor is too weak a limit
  on deadline density and a per-channel *rate* of urgent requests is needed
  beside it — and this RFC is superseded by one that says which.
- **The flag nobody reads.** If `SHORTFALL` is set on a large share of
  completions in a realistic run, the class ceilings in the topology are lying
  about what the system can serve, and the honest fix is in admission (`E1-B07`)
  or the topology — not in loosening bound 1 so the flag goes quiet.
- **A hard-class miss traced to a floored deadline.** If a hard-class request
  misses because a service floored its deadline and served it rather than
  refusing under `ADMISSION`, the "floor and report" choice above should become
  "refuse for the hard class" at the ABI rather than at each service's
  discretion. One such miss, root-caused, is enough.
