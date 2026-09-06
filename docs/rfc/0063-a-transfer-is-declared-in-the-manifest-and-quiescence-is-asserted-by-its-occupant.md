# RFC 0063: A transfer is declared in the manifest, and quiescence is asserted by its occupant

- Status: accepted
- Date: 2026-09-07
- Affects: `abi/src/transfer.rs` (new), `abi/src/manifest.rs` (`SCHEMA`,
  `Record`), `xtask/src/manifest.rs` (the checker, the writer and the record
  offsets), `docs/manifest.md`, `sim/src/deploy.rs`,
  `user/virtio-blk/manifest.toml`, `user/store/manifest.toml`,
  `user/virtio-gpu/manifest.toml`, `user/virtio-net/manifest.toml`; RFC 0008,
  RFC 0012, RFC 0018, RFC 0028, RFC 0030, RFC 0041; and `E2-D04`, `E2-B05`,
  `E2-B06`, `E2-P08`

## Decision

A component declares, in its manifest, whether it can be updated in place. The
declaration is `[transfer]`, it is **required** the way `[restart]` is, and it
has two modes and not three: `restart_only`, which hands nothing over, and
`in_place`, which does. Four fields, every one of them with a unit —
`mode`, `schema` (the component's own state-record ordinal), `record_bytes`
(the fixed width of one record) and `records_max` (the most it will hand over).

Around those four, five mechanical answers.

**Quiescence is asserted by the occupant, over a ring-empty precondition the
frame checks.** RFC 0018's cursors are necessary and not sufficient, so there is
no mode in which quiescence is read off them alone.

**What crosses is `records` opaque records of `record_bytes` each, at most
`records_max`, into a window the incoming instance bought out of its own
`Untyped` account** before the outgoing instance is asked to drain. The frame
allocates nothing and reads no record.

**A transfer that fails before the routing word is stored is abandoned, not
failed**: the place resumes delivering to the outgoing occupant, the running
generation keeps the place, and the client observes added latency and nothing
else. An abandonment is not a restart and does not spend the restart budget.

**`restart_only` gets a restart at the place**, counted as RFC 0012's
`places_restarted` and not as a swap. It costs the client every registration it
held.

**Compatibility is decided from two manifests before anything drains**: both
sides `in_place`, the same `schema`, the same `record_bytes`. Anything else is a
restart, decided while every client is still being served.

The manifest schema goes from 1 to 2, and every component file in the tree is
rebuilt.

## What the words mean, mechanically

### What a component declares

Three things were on the table — *can be transferred*, *cannot be transferred*,
*transferable only from a named quiescent point* — and the third is refused as a
separate value because it is what the first one already means. See *Quiescence*
below: there is no in-place transfer that does not wait for a point the occupant
names, so a third mode would leave behind a second one — a cursors-only
transfer — that no component in this tree can correctly use. A mode nothing may
use is a mode the first reader will use anyway.

So the wire is `abi::transfer::Declaration`, sixteen bytes inside
`abi::manifest::Record`:

| field | unit | under `restart_only` |
| --- | --- | --- |
| `schema` | none — a state-record schema ordinal, at least 1 | zero, and refused if not |
| `record_bytes` | bytes, a positive multiple of 8 | zero, and refused if not |
| `records_max` | count of records, at least 1 | zero, and refused if not |
| `mode` | none — `restart_only` = 1, `in_place` = 2; zero is not a mode | — |

Eight is `RECORD_ALIGN` and it is derived rather than chosen: the window is
records laid end to end and read in place, so a width that is not a multiple of
the widest scalar's alignment puts every other record on an odd boundary, which
is what `Refusal::Unaligned` already refuses one record up.

`schema` is the *component's* ordinal and not this crate's. Two components that
both write `1` have said nothing to each other; the only comparison it is ever
in is against another build of the same component. That is the field that makes
a state record a format rather than a memory layout, and it is why the record's
byte layout can be left to `E2-B06` without leaving the declaration incomplete.

The table is **required**, which is stricter than the word *default* the E2 spec
used. The record's zero is not `restart_only` and never becomes it: a zeroed
`mode` is refused by `Record::read`, exactly as a zeroed `domain` and a zeroed
`restart` are. A default is how a component acquires a property nobody chose,
and a place refilled from a newer manifest is the most expensive place in this
system for two readers to have chosen differently. `mode = "restart_only"` is
one line and it is honest.

### Quiescence, against RFC 0018's cursors

**The cursors are a precondition, not the property.** They say the submission
ring is empty and the completion ring is drained: producer stopped, consumer
caught up, on both. The frame can check that without asking anybody, and it
does.

What they cannot say is that the *occupant* is empty. RFC 0041 names the set
that makes this concrete: the occupant "holds the work it has accepted and not
answered, keyed by the token", and a kill discards it. A request that has been
taken off the ring and handed to a device is behind both cursors and behind the
driver; both rings read empty and an operation is in flight. A swap that trusted
the cursors there would retire an occupant holding a write the device had not
acknowledged, which is the durability failure `Chaos::lazy` exists to catch.

So quiescence is **both**, and the second half is an assertion each component
makes. That is the answer the E2 spec's *Risks* section said would be needed if
the cursors turned out not to be enough, and it costs less than that section
feared, because asserting is not the same as growing a state machine: the
component says *I hold nothing* at a point it already reaches, and the frame
believes it only after the cursors agree.

**And the E1 virtio-blk driver can declare one without a rewrite**, which is the
question this decision had to answer before the word was worth writing.
`component::serve` already has the point: it takes the control ring, drains the
data ring into `Pending`, and picks. At the top of that loop `Pending::is_empty()`
is the whole of what the component has accepted and not answered — because
`pending::IN_FLIGHT` is one and `Driver::execute` polls its chain back out of the
device before it returns. No new state, no second flag, one existing predicate.

That is a property of `IN_FLIGHT == 1` and not of the design, and it is the
first reversal below.

### What crosses, and out of whose account

`records` records of `record_bytes`, laid end to end, opaque to the frame. The
frame knows the width and the bound because those two size the window; it never
reads a field, because the only reader is another build of the same component.

The window is allocated by the **incoming** instance, out of **its own**
`Untyped`, before the outgoing instance is asked to drain, and granted to the
outgoing instance as a window it may write — RFC 0033's shape, a granted window
is a safe accessor.

The three alternatives, and why they lose:

**The frame allocates it.** Refused, and this is the one that had to be refused
explicitly. RFC 0008 rests on a component being paid for out of an `Untyped` that
can be revoked; memory the frame allocates on a component's behalf is memory no
revocation reaches, and a swap that could be attempted repeatedly would be an
unbounded frame allocation reachable from above the frame. A hole in the account
model, opened for a transient buffer.

**The outgoing instance allocates it.** Refused: the outgoing account is the one
about to be revoked. Retiring the occupant would either destroy the state the
incoming instance is reading or force the frame to keep a revoked account alive,
which is the first alternative wearing a different name.

**The incoming instance allocates it.** Taken. The account that pays is the
account that survives; the window is part of the incoming component's own
declared footprint, so `record_bytes * records_max <= memory_bytes` is a lint on
one file — refused by `cargo xtask lint-manifests` and again by `Record::read`,
rather than discovered at the swap with a client's submissions already held. And
abandoning a swap is exactly revoking the incoming account: no separate cleanup
path, because there is nothing the frame owns to clean up.

### When it fails halfway

There are two phases and the boundary between them is a single machine word.

**Phase A — drain, write, acknowledge.** The place stops delivering and holds
pends (RFC 0008's connect against an empty place, RFC 0041's place); the
cursors are checked; the occupant asserts; it writes its records; the incoming
instance acknowledges. Everything here is reversible, because the outgoing
occupant is *alive and still holds all its state*. Any failure — the occupant
never asserts, it writes more records than the window holds, the incoming
instance refuses them or faults — **abandons** the swap: the place resumes
delivering to the outgoing occupant and the pends drain to it.

Which epoch owns the place: the running generation, unchanged. Never a mixture.
RFC 0012 publishes the generation counter as zero for the duration of a swap and
restores the running counter here, so a reader that saw zero learns that no root
described the machine for an interval and that the root that describes it now is
the one that did before.

What the client observes: added latency, bounded by RFC 0041's own bound — the
control run's worst operation plus the manifest's backoff ladder. No lost
operation, no duplicate, no wrong answer. A client cannot distinguish an
abandoned swap from a slow one, and does not need to.

**Phase B — the routing word.** One `Release` store, read by an `Acquire` load.
A machine word either carries the new value or the old one, so there is no
halfway. After it, the outgoing occupant is retired and the state belongs to the
incoming one; a failure of the incoming instance from that moment is a *fault*,
and it takes the supervisor's existing restart path at the place, with the
transferred state lost because the instance that owned it died. That is the
fifth cross-core word RFC 0016 says needs an argument, and `E2-B06` owes the
argument and the litmus test.

**Why an abandonment is not the fault path the supervisor already has.** They
answer different questions. The restart path answers *the occupant died*: it
spends one of `max_restarts`, waits a backoff, and retires the place when the
budget is exhausted. An abandonment answers *the occupant is alive and the swap
did not happen*, and the occupant that would be penalised is the one that
behaved correctly. Route it through the restart path and `max_restarts` failed
updates retire a healthy component's place — an update failure taking down a
working driver, with the manifest's own budget as the mechanism. So an
abandonment spends no restart, waits no backoff, and is counted under its own
name beside RFC 0012's `places_swapped` and `places_restarted`. `E2-B06` is
where that counter lands and `E2-P08` is what makes it non-zero on purpose.

### What `restart_only` gets, and what it costs a client

A restart at the place: pends held, the occupant torn down, a new one spawned
from the incoming manifest, the pends delivered to it. RFC 0012 already counts
it — `places_restarted`, deliberately not summed with `places_swapped`, because
a swap in which every place declared `restart_only` is a reboot in instalments
and the artefact has to show that rather than report a swap.

The cost to a client is nameable and is the reason `in_place` exists.
Registration is an operation (RFC 0028) and a `SetId` names a slot in *this
instance's* table; a fresh instance's table has never been filled, so every id a
client still holds is answered `NO_SUCH_CAP` and every buffer set has to be
registered again before a single read can be submitted. Nothing is corrupted and
no operation is lost — a refusal is an answer, which is what RFC 0041's first
claim requires — but the client does work again, and it does it at a moment
nobody asked it to.

That cost is currently **unmeasured**. Claims 0005 and 0006 measure a kill under
load and do not count re-registrations; no claim in the registry counts them.
Stating that here rather than implying a number is the point: `E2-P08` is where
a count could be taken, and this RFC publishes none.

## Context

`E2-D04`'s exit is *schema merged; the virtio-blk driver from E1 declares one,
and `E2-P08` swaps it*, and its position in the graph is what shaped this. RFC
0012 had already made an update a generation swap and named `f_abi::transfer` as
the thing that crosses; RFC 0041 had already made a place outlive its occupant;
RFC 0008 had already made a component's whole footprint revocable. What was
missing was the sentence joining them: whether the incoming occupant of a place
may inherit anything, and what it is allowed to inherit it out of.

The E2 spec says, in as many words, that `E2-D04` "is a schema in `abi/` and not
an RFC", and names four fields. This entry exists anyway, for two reasons that
are not the schema. The first is the schema bump: `abi::manifest::SCHEMA` and
the 2 216-byte record assertion are things already written down, `docs/manifest.md`
says a schema other than 1 is refused, and RFC 0030 priced a bump as *a rebuild
of every component file*. Changing that is a reversal by rule. The second is
that four questions had to be answered before the four fields meant anything —
whose account pays, what happens halfway, what quiescence is measured against,
what a `restart_only` component costs — and every one of them is a decision a
future contributor would otherwise re-litigate at `E2-B06`, with a client's
submissions held while they did.

Two alternatives to the schema bump were live.

**Keep the declaration out of the record and read it from the manifest source.**
Refused: nothing above the boot loader reads TOML at run time, by RFC 0030's own
decision, and the assembler reads a generation tree whose `component` leaf is a
hash and not a table. A declaration the assembler cannot see is a declaration
that decides nothing.

**Spend the reserved bytes.** `Record::_reserved` is three bytes and
`Ring::_reserved` is five; the declaration is sixteen. Nowhere near, and a
declaration split across two reserved holes would be exactly the unjudged-byte
problem the record's no-padding assertion exists to prevent.

One alternative to the required table was live, and it is the E2 spec's own
word. **Make `[transfer]` optional and read its absence as `restart_only`.**
Refused: a reader of a manifest then cannot tell *decided `restart_only`* from
*never thought about it*, and those two differ exactly at the moment somebody
tries to update the component. The record's default stays `restart_only` in the
sense that matters — it is the value a component declares when it has done no
work — and the lint requires it to be written.

## Consequences

Makes easy: deciding whether a swap is possible before it costs anything. Three
field comparisons over two manifests, taken by the assembler from the generation
tree, with every client still being served. A component that cannot be swapped
is a restart that was *planned* rather than a swap that failed.

Makes easy, second: a transfer that is paid for by something revocable. Nothing
in this design gives the frame a buffer to own, so abandonment has no cleanup
path of its own — it is the revocation RFC 0008 already describes.

Makes hard: changing a state record's shape. `record_bytes` is compared for
equality, so a build that widens its record is not a superset of the one before
it; it is a `schema` bump and a restart at that place for one generation. That
is deliberate — the alternative is two builds reading the same bytes differently
— and it is the cost a component pays for being updatable at all.

Makes hard, second: schema 1 component files. Every one of them is refused. In
this tree that is four manifests and a rebuild; outside it there are none,
because nothing has ever shipped a component file. This is the last moment that
sentence is true, and it is why the bump lands now rather than at `E2-B06`.

Forecloses: a cursors-only transfer. There is no way to declare *swap me
whenever the rings are empty*, and a component that wants one has to assert
anyway. That is a real loss for a genuinely stateless component, and it is
priced at one predicate that returns `true`.

Costs sixteen bytes per component file, and moves every offset after byte 104 in
the record. Both are compile-time constants mirrored in `xtask`, and
`the_record_layout_matches_the_abi` reads `abi/src/transfer.rs` and fails when
either moves.

## What would reverse this

**`E1-B09` raising `IN_FLIGHT` above one.** This is the near one and it has an
owner. The virtio-blk driver can assert quiescence today because
`pending::IN_FLIGHT` is one and `Driver::execute` polls its chain back out of
the device before it returns, so an empty `Pending` is an empty driver. When the
driver waits on its interrupt instead of spinning, chains are outstanding inside
the device and an empty `Pending` no longer means what the manifest comment says
it means. The declaration does not change; the assertion grows a second term
over the used ring, and `claims/0012`'s `in_flight` row is the number that moves
with it. If that second term turns out not to be expressible in the driver's own
loop, then quiescence is not assertable by a driver and this decision is wrong
in its central claim.

**`E2-P08` observing a dropped, doubled or wrongly-answered operation across a
swap.** The E2 spec named this as the observation that would turn
`f_abi::transfer` from a record into a protocol, and it still is. The three
labels are RFC 0041's and `sim/src/chaos.rs` already refuses on each separately,
so the evidence would arrive named.

**An abandonment rate high enough to be a policy question.** This RFC says an
abandonment costs a client latency and nothing else, and that it spends no
restart budget. Both are true per abandonment and neither is true of an
unbounded number of them: a swap that abandons every time is an update that
never lands, with no budget to stop it retrying. If `E2-P08` or a later
deployment shows swaps abandoning repeatedly, the repair is a budget on
abandonment — its own count and its own window, not `max_restarts` — and it is
an RFC, because "an abandonment is not a restart" is the sentence it would
weaken.

**Every manifest in the tree saying `restart_only` except one.** Then `in_place`
is a field with a single user and the honest question is whether it is a
property of components or a property of one driver. The test is RFC 0051's — a
second driver is what says a shape is a shape — and `user/virtio-gpu` is the
second driver. `docs/manifest.md` carries this as a reversal condition on the
schema so that it is read by whoever writes the next manifest.

**A component whose state does not fit a fixed-width record.** The format says
*declare the widest record you will write and pad*, and a component whose state
is genuinely variable-length — a name, a path, a list — pays for its worst case
in every swap. If that padding becomes the dominant cost of a transfer, the
answer is a length field per record and not a variable record, and it is a
`Declaration` change and therefore this document's reversal rather than a
component's local decision.
