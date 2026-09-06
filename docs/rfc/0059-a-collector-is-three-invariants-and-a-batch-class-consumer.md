# RFC 0059: A collector is three invariants and a batch-class consumer

- Status: accepted
- Date: 2026-09-06
- Affects: `docs/design/deadline-all-the-way-down.html` section 04 (the
  *Collection* paragraph, and the threshold in *The zoned mapping*) and
  `docs/design/fast-path.html`'s bet-03 risk line *garbage collection becomes a
  real engineering problem*; `zone/src/mark.rs`, `zone/src/sweep.rs`,
  `zone/src/roots.rs` and `zone/tests/invariants.rs`; the objects service's
  opcode space and the pin namespace in `f-index`; `claims/0016`, whose
  threshold is derived from `SWEEP_LIVE_FRACTION`, and `claims/0012`, from
  whose `in_flight` the third invariant's bound is taken; `E2-B02`, `E2-B03`,
  `E2-B08` and `E2-P03`; RFC 0058, whose extents pin under this RFC's clause,
  and RFC 0060, whose second `FLUSH` is where a pin is released

## Decision

The collector marks from a root set, sweeps sealed zones whose live fraction
has fallen below `SWEEP_LIVE_FRACTION` = 0.25, and submits every one of its own
device operations as `class::BATCH` with no deadline. Three things are decided
beyond restating the design page. **First, the root set is not the pinned
roots. It is the pinned roots *and every open write set*** — a publish
registers a transient root at its first `ZONE_APPEND`, and releases it only
when the completion of RFC 0060's second `FLUSH` has made it durable *and*
that root has entered the pinned set, in that order. **Second, the third
invariant is given a number, because without one it is not an invariant**: a
hard-class read submitted while a sweep is in progress is handed to the device
with at most `collector_operations_ahead_of_hard_read` = 1 collector operation
ahead of it, which is `f_virtio_blk::pending::IN_FLIGHT` and not a number we
chose. **Third, all three are written as predicates over named state** — the
root set `R`, the mark bitmap `B`, each zone's state and live byte count, and
the driver's queue — so that `E2-P03` asserts a predicate at every step of a
run rather than watching for a symptom. What we are refusing, in one sentence:
a collector whose correctness argument is "the sweep is careful about
in-flight publishes". Care is not assertable; a root set is.

## Context

`docs/design/deadline-all-the-way-down.html` section 04 has said since it was
written that reference counting is the wrong mechanism, that the answer is to
mark from live roots and sweep zones by live fraction, that the mark is
incremental and generational, that the sweep is a batch-class consumer "so
collection can never starve a deadline-class read", and that roots are pinned
explicitly. Every clause of that is kept here. What none of it is, is
assertable. `E2-P03`'s exit is *invariants*, and three sentences in the
indicative mood are behaviour: a run that does not happen to lose data passes
all three.

What was true when this was decided. `f-blob` exists as `E2-B01` built it —
the chunker, the record kinds, the on-disk format, and blob locations returned
by `ZONE_APPEND` completions because the device assigns the position. RFC 0060
has fixed the publish sequence as *write the blobs, `FLUSH`, append the root
record, `FLUSH`*, and the five zone opcodes exist on the blk service. RFC 0025
has fixed what a class means on a ring and RFC 0049 has fixed where a device
queue applies it, with `IN_FLIGHT` = 1 written into the routing page and named
in claim 0012's threshold. RFC 0013 has fixed what publishing a number means.
`f-zone` does not exist; that is the point of writing this now, and *Ordering*
below says why.

Two things about the store make this collector unlike a language runtime's, and
both are load-bearing:

- **No edge ever changes.** A blob's content, and therefore every hash it
  names, is fixed at the moment it is written. There is no old-to-young pointer
  because there is no pointer write at all. The tricolour invariant, the write
  barrier and the whole apparatus of concurrent marking exist to cope with a
  mutator that moves an edge behind the marker's back, and none of that
  apparatus is needed here. The only thing that mutates is the *root set*.
- **Reachability decreases as well as increases.** Unpinning a snapshot can
  make a large subgraph garbage without any write happening. So a mark cannot
  be purely monotone across cycles, and this RFC pays that cost explicitly
  rather than hiding it (see *the full re-mark*, below).

### The state the invariants range over

- `P` — the **pinned set**: the mounted generation's root, plus every root hash
  named in `f-index`'s pin namespace. Durable.
- `T` — the **transient set**: one entry per open write set. An entry is *not*
  a hash; it is the list of block positions that write set has appended and
  that no durable root yet names. It cannot be a hash, because the hash that
  will name them is the thing that does not exist yet — which is the whole
  reason the clause is needed.
- `R = P ∪ T` — the root set.
- `B` — the **mark bitmap**: one bit per block-aligned position in the store,
  sized at mount from the superblock as `zones · (zone_bytes / block_bytes)`
  bits. A set bit means *a blob starts here and is reachable*. One bit per
  device block is 1/32768 of the device's capacity: 32 MiB of resident memory
  per TiB of store, and claim 0019 counts it.
- `marked_roots` — the roots whose walk has completed into the current `B`.
- `state(z) ∈ {free, open, sealed, condemned}` and, per zone, `live_bytes(z)`
  and `reads_outstanding(z)`.
- `f(z) = live_bytes(z) / zone_bytes`, evaluated as the integer comparison
  `live_bytes(z) · 4 < zone_bytes` so that 0.25 is exact and the collector
  contains no floating point.
- `Q` — the blk driver's queue, ordered by what `f_abi::deadline::inherit`
  returned (RFC 0049).

### The three invariants

**I1 — nothing reachable is swept.** At the instant `ZONE_RESET(z)` is
submitted,

    blobs(z) ∩ reach(R) = ∅

where `reach` is the transitive closure of the hash references in the store and
`R` is the root set *at that instant*, transient entries included. `E2-P03`
asserts the mechanism that implies it, which is checkable in one line:
`marked_roots ⊇ R` and every bit of `B` in `z`'s range is clear.

**I2 — a reset zone holds no live blob.** For every zone and at every instant,

    state(z) = free  →  ∀h : location(h) = (z, ·) is undefined

That is stronger than the sentence it comes from, deliberately: it forbids not
only live data in a reset zone but any *resolution* that lands in one, which is
the exact shape of the forbidden third state `E2-P01` sweeps for — a root that
names a zone the collector has already reset. It is bought by an ordering the
sweep may not reorder: copy the survivor to the destination zone, `FLUSH`,
repoint `location(h)` at the copy and clear the old bit, and only then submit
the reset. And the reset guard carries a third conjunct that is not optional:
`reads_outstanding(z) = 0`. A read that resolved `h` to `z` before the
relocation is a reader that believes `z` is live, and I2 is about beliefs as
well as bytes. The count is bounded because condemnation stops `location` from
handing out `z`, so after condemnation it is monotone non-increasing.

**I3 — collection never starves a deadline-class read.** For every entry `e`
submitted to the blk ring whose inherited class is more urgent than
`class::BATCH`,

    |{ c : c submitted by the collector, handed to the device
         after e was submitted and before e was }|
      ≤ collector_operations_ahead_of_hard_read = 1

*Stated for any entry above batch, not only for hard*, because
`user/virtio-blk` declares the soft class and RFC 0025 bound 1 therefore serves
every hard-class read as soft with `SHORTFALL` set. An invariant written only
about hard entries would be vacuous against the driver this epoch actually has.

### The number, and where it comes from

The design page's *can never starve* is a claim about a resource discipline,
and a test cannot assert it. A bound in time would make it assertable and would
also make it a clock, which RFC 0004 forbids and which would put this
invariant behind the machine `E0-D10` owes. So the bound is a count of device
operations, and the count is derived rather than invented:

An entry already inside the device cannot be overtaken by anything (RFC 0049,
point 2). `f_virtio_blk::pending::IN_FLIGHT` = 1, because `Driver::execute`
offers one chain and polls the used ring until it comes back. Every other
collector operation is still in the driver's queue, where it sorts below any
entry above batch. So the collector work that can be ahead of an urgent read is
exactly the collector work already in the device, which is at most
`IN_FLIGHT`. The number is 1 because that constant is 1, and **the row moves
when `E1-B09` raises it, re-derived in the same diff** — which is precisely
what claim 0012's `in_flight_above_one` note already says happens to
`batch_operations_overtaken` when that lands. One constant, two claims and one
invariant reading it, rather than three numbers that agree by accident.

**What observation would show the number is wrong.** The count bounds
*operations*, and an operation is not a unit of time. A copy-forward of a
`CHUNK_MAX_BYTES` = 256 KiB blob and a 4 KiB read are both one operation, and
being behind one of the former is sixty-four times the device work of being
behind one of the latter. So: a hard-class read missing its deadline in a run
where `collector_operations_ahead_of_hard_read` never exceeded 1, root-caused
to the size of the collector operation ahead of it, says the currency is wrong.
The answer then is a byte bound on a single collector operation —
`COLLECTOR_OP_MAX_BYTES`, with the count re-derived over it — and not a smaller
count, because the count is already at its floor. We cannot take that
measurement here: it needs a timing, and every timing in this project is
`pending` on `E0-D10`'s machine. Saying that plainly is better than picking a
byte bound now and pretending it was derived.

### What protects a publish that spans more than one zone

A publish writes blobs, and the blobs are unreachable until the root record
that names them is durable. A publish large enough to fill a zone therefore
seals a zone whose live fraction against the *pinned* roots is zero. Sealing is
harmless; condemnation is not. Without a clause, the collector selects that
zone as the emptiest sealed zone in the store — it is the emptiest, by
construction — evacuates nothing, resets it, and the root record lands minutes
later naming a zone that no longer holds anything. That is `E2-P01`'s forbidden
third state manufactured by the collector rather than by a power cut, and I1,
I2 and I3 all hold while it happens, because at every instant the collector
looked, nothing reachable was in that zone.

**What protects it is a pin, and specifically an implicit one taken by the
store on the publisher's behalf: an entry in `T`.** Not a nursery, and not a
rule about when a zone may be selected. Both were live and both are refused:

- **A nursery** — a zone class the collector never selects until it has aged
  out — puts the protection on the wrong axis. A publish can span any number
  of zones and take arbitrarily long, so a nursery either has to be as large
  as the largest publish (unknowable) or it fails on exactly the case it
  exists for. It also makes selection a function of age rather than of
  liveness, which re-introduces the time-like reasoning the rest of this
  design has removed, and it makes `live_bytes(z)` a number the sweep is not
  allowed to believe — which is the number claim 0016's threshold is derived
  from and the number the state tree publishes.
- **A selection rule** — "do not condemn a zone written to since the last
  quiescent point" — is a predicate over device history rather than over
  reachability, so it cannot be checked against I1's mark and `E2-P03` would be
  asserting two unrelated things. Made precise enough to be safe, it has to
  name which publish touched which zone; at that point it *is* the transient
  root, written less directly and stored somewhere the invariant cannot see.
- **The pin** costs one entry in a set the mark already ranges over. The
  publisher's appended positions are marked, `live_bytes(z)` is not zero, the
  zone is not selected, and even if it were, the reset guard finds bits set.
  One predicate covers the pinned case and the open case, and `E2-P03` asserts
  one thing.

**The handover is add-then-drop, and this is the sentence to guard.** The
transient entry is released only after the new root has entered `P`. Dropping
first — releasing at the root record's *append* rather than at the completion
of the second `FLUSH`, or releasing before the pinned set has adopted the
root — opens a window in which the blobs are named by nothing, and the window
is exactly as long as a device write. A one-line reordering reintroduces the
bug this clause exists for, so it is written here as an ordering and not as a
lifetime that happens to work out. RFC 0058's extents use this same clause: an
extent with unsnapshotted pieces is an open write set, held for as long as its
owner chooses not to snapshot, bounded by the extent's own size.

### The mark: incremental and generational, and what those words buy

*Generational* here is not a hypothesis about object lifetimes. It is the
consequence of the immutability above: **a walk that reaches an already-marked
node stops**, and that is sound because the subgraph under a hash can never
have changed since the walk that marked it. So the cost of marking a new
generation is proportional to the blobs that generation added, not to the size
of the store. The root record's `previous` field gives the walk its starting
point and the fold gives it a shape of tens of nodes above the leaves.

*Incremental* means a cycle is a sequence of bounded steps — each step at most
one device read plus bounded computation, each submitted as batch — with client
writes and publishes interleaved freely. The safety argument is short because
edges do not move: the only mutation is to `R`, and it is handled by making the
cycle's root set a superset of the root set at every instant inside it. Roots
added during a cycle (a new transient entry, a new `PIN`) are marked *within
the cycle*, before any sweep in that cycle may condemn a zone. Roots removed
during a cycle are not applied until the cycle ends. `marked_roots ⊇ R` is
therefore an invariant of the cycle and not a race the sweep has to think
about.

**The full re-mark, which is what removal costs.** A removal — `UNPIN`, or a
publish's transient entry being released — cannot decrement `live_bytes`
correctly without knowing which blobs *only* that root reached, and that
question is the one the design page says reference counting handles badly. So
we do not answer it incrementally: a removal clears nothing, and schedules a
**full re-mark**, which clears `B` and walks every root in `R` from scratch. It
runs at most once per cycle, and it costs one device read per reachable blob
record. Between a removal and the next full re-mark, every `live_bytes(z)` is
an *upper bound*, so every `f(z)` is an over-estimate, so the sweep reclaims
late and never early. Conservative in the direction that cannot lose data, and
the cost is stated rather than discovered: deleting a snapshot is a walk of the
live set, not a decrement.

### Live fraction: computed where, published where

`live_bytes(z)` is accumulated *by the mark*, not by scanning zones. When the
walk sets a bit it adds that blob's `content_bytes` rounded up to `block_bytes`
to the zone that bit belongs to; the bit is the deduplication, so a blob
reachable from four roots is counted once. The hash-to-zone map the walk needs
is the one `f-zone` builds from its own `ZONE_APPEND` completions, not
`f-index`'s copy of it — `E2-B02` does not depend on `E2-B03`, and a collector
that could not run without the index would invert the build order for no gain.
The denominator is the superblock's `zone_bytes`, the zone's capacity and not
its write pointer, because capacity is what a reset returns.

Selection: among zones with `state(z) = sealed` and `live_bytes(z) · 4 <
zone_bytes`, condemn the smallest `live_bytes` first, ties broken by ascending
zone index. The tie-break is stated because RFC 0004 says an order nobody
stated is an order the map chose.

A cycle starts on one of two events and never on a third: `zones_free <
COLLECT_FREE_ZONES_LOW` = 2, or an explicit `COLLECT` opcode. The 2 is
derived — the client needs an open zone to keep writing into and the sweep
needs a destination zone that is not the one it is about to reset, so a
collector that starts at one free zone can deadlock against its own writer.
There is no timer, and there could not be: a collector that runs "every so
often" reads a clock. The explicit opcode is not a convenience either; claim
0016's workload is phased — *fill with the collector held, collect to
quiescence with the client idle, count* — because copy-forward bytes otherwise
depend on the interleaving of two components, and that phasing requires a verb
that means *collect now, and tell me when there is nothing left to do*.

Published under RFC 0013 in `user/objects`' own tree, under a `collector`
subtree: `cycle`, `full_remarks`, `roots_pinned`, `roots_transient`,
`zones_free`, `zones_sealed`, `zones_condemned`, `live_bytes_total`,
`copied_bytes` and `ops_ahead_of_urgent_read` (the maximum observed, which is
I3's own witness). Beside them a fixed array of per-zone `live_bytes` gauges,
`COLLECTOR_ZONE_NODES` = 4096 of them, with `zones_unpublished` counting any
remainder — a tree of unbounded width is not a tree of permanent node ids, and
truncating without a count is the failure RFC 0013's skipped-node count exists
to prevent. There are no floats in the tree: `live_bytes(z)` and the
superblock's `zone_bytes` are both published and the reader divides, so the
number a claim reports and the number the collector compares are the same
number, which is the whole reason RFC 0013 exists.

### `SWEEP_LIVE_FRACTION` = 0.25

Reclaimed bytes per zone swept are `(1 − f) · zone_bytes` and copied bytes are
`f · zone_bytes`, so the copy-forward cost is `f/(1 − f)` bytes written per new
byte stored. At 0.25 that is 0.33, and claim 0016's threshold is built from it:
1.0 for the data, 0.33 for copy-forward, ≤ 0.06 for block padding, plus the
root record, totalling 1.39 against a threshold of 1.5. At 0.5 the term is
1.00 and the total is over 2.0, which falsifies claim 0016's own threshold; at
0.125 the term is 0.14 but a zone must be seven-eighths dead before anything
reclaims it, so free capacity is scarce and every cycle starts under pressure.
**0.25 is the largest threshold whose copy-forward term leaves claim 0016's 1.5
with margin.** That is the derivation, and it is why the constant and the claim
move together or not at all.

### Pinning a root, on the wire

A pin is an **opcode on the objects ring with a capability behind it**, not a
field, not a flag and not a write to a mapping. Three operations, whose numbers
are assigned contiguously when the objects service's `op` module is first
written in `E2-B08`'s diff so that the space is designed once rather than
grown:

- `PIN`, carrying a 32-byte generation root hash. Refuses if the root does not
  resolve — every child of its generation node, exactly RFC 0060's
  verify-before-accept — so a pin can never name a generation that is already
  gone.
- `UNPIN`, the same hash. Schedules the full re-mark above.
- `COLLECT`, which starts a cycle and completes when the collector has nothing
  left to do.

**A pin is durable, and durable means an entry in `f-index`'s pin namespace** —
a name for a root hash, which is what the index is already for and what the
design page already means by *snapshot retention is expressed by pinning the
root*. Three alternatives were refused. A pin held only by the channel that
took it dies with its holder, so a component restart under claim 0005's policy
would silently collect a snapshot a person asked to keep. A pin bit in the root
record cannot exist: a root zone is sequential-write-required, so a bit written
there could never be cleared. A pin written into the state tree is a control
plane reached by writing to a mapping, which RFC 0013's *honest limit* declines
in as many words. The index costs nothing new and buys one more thing: because
a pin is a name and not a record in a zone that wraps, **a pinned generation
survives the root-zone wrap** — the root record may age out of both root zones
while the generation remains reachable by hash. That is the mechanism behind
the spec's sentence that retention beyond the wrap *is* pinning.

Until `E2-B03` lands, `P` holds exactly the mounted root and `f-zone`'s host
tests use an in-memory pin set. That ordering is stated so that nobody reads
`PIN` as available in `E2-B02`.

### Ordering: why this sits immediately before `E2-B02`

`TODO.md` ordering rule 3 puts a decision immediately before the work that
would be expensive to redo without it, and never at the head of an epoch. This
is that position exactly. The three invariants constrain the *shape* of
`f-zone`, not its details: the transient root set decides who calls the zone
layer and when, the mark bitmap is sized at mount and is the collector's whole
memory budget, the reset guard's three conjuncts decide the sweep's ordering of
device operations, and the batch class decides what the collector writes into
every entry it submits. A collector built first and given invariants afterwards
would have them reverse-engineered from it, which is how an invariant becomes a
description of the code. It is also not written earlier than this: RFC 0060 had
to fix the publish sequence before "when is a pin released" had an answer, and
`E2-B01` had to fix `CHUNK_MAX_BYTES` before the size of one collector
operation was a number. And `SWEEP_LIVE_FRACTION` is owed before claim 0016 is
registered, because that claim's threshold is derived from it.

### The alternatives that were live

- **Reference counting.** Refused by the design page and refused again here in
  this design's terms: a count beside an immutable blob is durable state that
  the publish sequence must order, so RFC 0060's four steps become six, and it
  still does not answer *which zone is worth sweeping* without the per-zone
  tally the mark already produces. It buys incremental removal and spends
  crash consistency.
- **Stop-the-world marking at quiescence** — collect only when no publish is
  open. Simple and safe, and it starves the collector to death under a
  continuous writer, at which point the store wedges at zero free zones. That
  is a liveness failure worse than the latency it avoids, and it would make the
  phased workload of claim 0016 the only configuration in which the collector
  ever ran.
- **The idle class instead of batch.** Section 01 says idle work "runs only on
  resources nothing else wants", which under a sustained writer is never. Batch
  is the correct class: it yields to everything with a deadline and still runs.
  Note that this needs saying on the wire — RFC 0025's scope rule says a
  component's own work carries the component's own class, and `user/objects` is
  not admitted for batch, so **the collector writes `class::BATCH` and
  `NO_DEADLINE` into every entry it submits**, which bound 2 permits because a
  ceiling is a ceiling.
- **A nursery, or an age-based selection rule.** Refused above.
- **A larger sweep threshold.** Refused by claim 0016's arithmetic, above.

## Consequences

**Easy.** `E2-P03` becomes three assertions over state the collector already
keeps, each readable from the published tree: `marked_roots ⊇ R` with `B`
clear over the condemned zone, no resolution into a `free` zone, and
`ops_ahead_of_urgent_read ≤ 1`. Its adversarial case — a multi-zone publish in
flight during a sweep — is a transient entry in a set, so the test asserts a
membership rather than watching for corruption. Snapshot retention is a name in
the index and needs no format. Claim 0016's threshold and the collector's
constant are the same constant. The collector needs no new scheduling
machinery: it is a client of a queue that already orders by class.

**Hard.** The root set must be *exact*, and that obligation falls on every
future writer, not only on the publish path: any code that writes a blob before
a durable root names it must take a transient entry, or it is writing data the
collector may take. RFC 0058's extents already needed it, which is the evidence
that this is a general rule and not a special case. Unpinning costs a full
re-mark — a walk of the live set — so deleting snapshots is not cheap and a
component that pins and unpins in a loop makes the collector do nothing else.
Between full re-marks every live fraction is an over-estimate, so a capacity
emergency cannot be fixed by sweeping harder; it is fixed by a full re-mark
first. And the mark bitmap is resident memory proportional to the device: 32
MiB per TiB, which claim 0019 counts and which a small component's quota must
be sized for.

**Forecloses.** Reference counting, again and for the reasons above. A
stop-the-world mark. Collection triggered by elapsed time, by a duty cycle, or
by any other clock. A pin that lives in a channel, a mapping, or a zone. A
collector that runs at the idle class, or at its component's own class rather
than at batch. A sweep that decides what to condemn from anything other than
`live_bytes`, which forecloses every "it has been a while since we touched that
zone" heuristic. And a `zone/` that reclaims early to hit a capacity target:
every conservatism in this design points the same way, and a change that made
one of them point the other way would be a change to I1.

## What would reverse this

- **`E2-P03` observing `collector_operations_ahead_of_hard_read ≥ 2`** with
  every entry inside its admission and `IN_FLIGHT` still 1. That is not a
  tuning failure; it means an urgent entry reached the device behind collector
  work that the queue should have ranked below it, and the fault is in RFC
  0049's application or in a collector entry that did not carry `class::BATCH`.
  One such run, reproduced from its seed, and I3 is asserting the wrong thing.
- **A hard-class read missing its deadline with the count at 1**, root-caused
  to the size of the single collector operation ahead of it. Then the count is
  the wrong currency, a collector operation gets `COLLECTOR_OP_MAX_BYTES`, and
  the bound is re-derived over bytes. This is the reversal we cannot currently
  observe, and it waits on `E0-D10`'s machine like every other timing.
- **Claim 0016 measuring `device_bytes_per_app_byte` above 1.5 while every
  swept zone's live fraction at condemnation was at or below 0.25.** Then
  `f/(1 − f)` is not where the write amplification is coming from, the
  derivation that produced both 0.25 and 1.5 is wrong, and finding the missing
  term matters more than moving either number.
- **`zones_free` reaching 0 in claim 0016's fill phase while the mean live
  fraction across sealed zones stays above 0.25.** Then 0.25 is too aggressive
  a threshold — the store fills with zones the collector is not permitted to
  touch — and the threshold rises, trading claim 0016's number for capacity.
  That trade should be made against a measurement, which is what this bullet
  names.
- **The full re-mark costing more device reads per reclaimed byte than
  copy-forward costs in writes.** That is the observation that would
  rehabilitate reference counting: it would mean that maintaining liveness on
  removal is cheaper than recomputing it, which is precisely the trade
  section 04 refused. If it appears, this RFC is superseded by one that says
  so, rather than a counter being added beside a mark.
- **A blob resolving into a reset zone, once, traced to the collector rather
  than to a power cut.** `E2-P01` exists to find the cut's version of this and
  `roots_refused_unresolved` counts it; a collector-caused instance means the
  pin is in the wrong place — most likely a drop-then-add — and it is worth
  reopening the nursery, because the argument above assumes the ordering is
  respected and one such defect is evidence that it is not respectable.
