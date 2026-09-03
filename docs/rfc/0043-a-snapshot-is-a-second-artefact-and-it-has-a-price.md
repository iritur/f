# RFC 0043: A snapshot is a second artefact, and it has a price

- Status: accepted
- Date: 2026-09-03
- Affects: `sim/` (`snap.rs`, `trace.rs`, `decide.rs`, `service.rs`, `client.rs`,
  `scenario.rs`, `lib.rs`), `env/src/split.rs`, `xtask` (`snapshot`),
  `claims/0007-snapshot-re-entry-saving.toml`, `docs/design/proving-ground.html`
  layer 1 and layer 4

## Decision

A running simulation can be written to a file at a decision point and re-entered
from it, and **there are two kinds of file because one kind cannot honestly do
both jobs**. A *whole* mark carries the artefact so far; a run restored from one
is indistinguishable from the run that replayed in every respect, the oracle's
verdict included, and reading it back costs the same order as replaying. A
*terse* mark carries the artefact's running hash and the live state instead; a
run restored from one produces the same digest and the same records from the cut
onward, is constant in size in the length of the run, is re-entered about a
hundred times faster — and is **refused by the oracle** rather than judged,
because every property in `sim/src/check.rs` reads a whole run.

What a terse mark is refused is a *verdict*, not a *reading*. `--resume` is a
source rather than a mode, so it takes the same `--check`, `--trace` and `--hash`
a scenario does, in either order: `f-sim --resume <mark> --trace` prints the
records from the cut onward and `--hash` answers the whole run's digest, while
`--check` says `partial` and stops. That distinction is what makes the fast half
of this decision a bisect tool rather than a digest comparison, and
`cargo xtask snapshot` holds it to that by requiring the buffer the run failed on
to be **in** the tail it re-entered. Without that requirement the sentence
*bisects in seconds rather than hours* would have been carried entirely by the
mark that costs a whole replay, and every assertion in the harness would have
stayed green with the failure moved to minute five.

Both are one wire format: fixed-width, little-endian, versioned, checksummed, and
refused when the file does not match this build. Three separate refusals, because
they are three different mistakes: a `FORMAT` number for a layout change, a
*build fingerprint* folding the label table, both scenario tables, the fault
classes, the registration and buffer geometry and the compiled-in deliberate
defects, and the *commit* the snapshot was taken at when the caller names one.

A snapshot is written between two steps, never inside one. An actor that has not
been taught to save refuses the whole file by name rather than writing a short
one.

## Context

`E1-P08` is one sentence: *a failure at simulated minute 40 is re-entered at
minute 39 without re-running the first 39*. What made it more than plumbing was
finding out which of the simulator's sources of nondeterminism are **state** and
which are **derivations**, and then finding out that the obvious design did not
pay.

**What travels, and what RFC 0026 had already decided.** The ordering stream and
the seven fault streams are pure: a decision's value is
`draw(seed, domain, site, occurrence)` and nothing else, so what a snapshot
carries for them is an occurrence count per site — a `BTreeMap` of a dozen
`(label, u64)` pairs. There is no generator state, no parent, and no tree. That
is RFC 0026's split-by-identity paying for itself a second time, and it was not
bought for this: it was bought so that adding a decision site would not
invalidate a recorded seed. The answer to *what is in a snapshot of a seeded
simulator* is not *the whole random tree*, and it is not because of a decision
taken a task earlier for a different reason.

The exception is exactly the part that is a chain rather than a derivation.
`World::draw` steps an `f_env::split::Stream`, which folds its own output back
into its state, so state `n` is reachable only by taking `n` steps. Those five
words travel, through `Stream::state`, which is the one addition this task made
to `env/`. Its documentation says why the two halves needed different answers,
and says what it does not weaken: a stream here is a reproducibility device and
never a secrecy one, `from_seed` is already public, and a caller who can read a
state could already build the same stream from the same seed.

**What could not be copied, and what was done instead.** Two of the simulator's
structures are real types from `ring/` with ownership rules that exist to make a
mistake unwritable, and a snapshot is not an exception to them.

`f_ring::registry::Table` has private slots because *a registration table that
could be copied would be two tables issuing one set of identifiers*. Widening
`ring/` so a snapshot could fabricate one would put a hole in the type RFC 0028
built to stop a peer naming a set it was never issued. So the table is not
copied: `sim::service::Service` keeps a journal of the operations that made it —
registrations and retirements, a handful per run — and a restore **replays them
through the real `Table::execute` against the real domain**. That is also what
the real system does: RFC 0008 has the frame revoke a dead component's buffer
sets, and a restarted component re-registers. The model of a restore is the shape
of a restore. The rebuilt domain is then compared against the one the file
recorded beside the journal, and a disagreement refuses the load.

`f_ring::buffers::Idle` and `InFlight` cannot be constructed by a caller either:
an `Idle` comes only from `BufferSet::carve` and an `InFlight` only from
`Idle::submit`, which is RFC 0024's *writing an id down should not be the
shortest path to a naming*. So what travels is each buffer's **index** and the
token it is out under, and `App::load` puts them back by doing what the client
did — bind the set, carve it, and submit into a sink that discards the entry. The
sink is the shape `f_ring::buffers` documents for exactly this: *a test can put a
recorder behind it and exercise the ownership rules with no ring at all*.

**The measurement that forced two formats.** The first design carried the whole
artefact, and it was correct: every scenario, cut at seven points, at three
seeds, restored into a run identical to the replay. It was also not a saving. On
the four-core development container, over a run of half a million steps and six
hundred thousand records:

| | wall clock |
|---|---|
| replay from zero | 911 ms |
| re-enter at minute 39, whole mark | 472 ms |
| re-enter at minute 39, terse mark | 9 ms |

Reading a record back costs the same *order* as taking a step. A snapshot that
carries the artefact is therefore linear in the prefix exactly as a replay is,
and the constant factor between them was under two. *Bisects in seconds rather
than hours* is not a statement a factor of two supports.

The saving comes from the artefact not travelling, and what that costs is
precisely the part of the artefact that is not there. `check.rs`'s five
properties read whole runs — which tokens were issued, which clients finished —
so a trace beginning part-way through fails `balance` and `bound` for every
operation answered before the cut. Those are findings about the snapshot rather
than about the system, and answering them would be the *plausible and wrong*
result this whole module is written against. So `examine` refuses a partial
artefact with a signature of its own instead.

**A third measurement, taken along the way.** Marking a run at every simulated
minute first cost sixteen seconds against a one-second run, because
`Trace::digest` rebuilt the whole artefact as a string at every mark. The fold is
now remembered and only the records since the last one are folded, which made a
scan cost one fold of the run rather than one per mark — and, incidentally,
halved the cost of `f-sim --hash` on a long run. It is a *cache* and not an
eagerly-maintained field because `cargo xtask sweep` puts a million trials
through the oracle and hashes none of them.

**Two live alternatives, both rejected.** A snapshot could have been the run's
*inputs* replayed to a point — no state at all, and no saving either, which is
the thing being bought. Or the oracle could have been made incremental, so that a
terse mark carried check state rather than records; that is a change to what the
oracle *is*, it would have to be re-argued whenever a property is added, and RFC
0040 landed the oracle three days earlier on the premise that a property names no
scenario and reads only the trace.

## Consequences

**What this makes easy.** A long run is investigated near its failure for the
price of the tail. A minimiser that today costs up to `MINIMISE_BUDGET` whole
re-runs has a mechanism to re-enter instead, and the two features meet at
`sweep::Trial`, which is what a snapshot's header carries. A person holding a
snapshot needs nothing else: the file carries the fault plan and every actor, so
a restored deployment run does not need the component files that produced it.

**What this makes hard, and the costs stated as costs.**

- A whole mark is linear in the length of the run — thirty-one megabytes at
  minute thirty-nine of `soak`. `--keep` bounds what stays on disk by dropping
  the oldest and `--after` avoids writing the early ones at all, and both give up
  the same thing: a bisect wanting an early mark scans again. Nothing is lost
  that a second scan cannot produce, because a run is a function of
  `(seed, commit)`.
- A terse mark cannot be judged. `f-sim --resume --check` on one reports
  `partial` and a non-zero status, which is fail-closed and is also a surprise
  the first time. It can still be *read*: `--trace` prints the tail.
- **Two numbers are gated, and they point in opposite directions.** The terse
  re-entry has a floor — at least ten times cheaper than replaying — and the
  whole re-entry has a *ceiling*, four times the replay. The ceiling exists
  because the whole mark is not a saving and is not expected to become one; what
  it rules out is the judgeable half quietly becoming several replays, which
  would retire it without anything going red. A floor there would be a floor at
  roughly one and would go red on container noise. Both are in `claims/0007` and
  both are printed by `cargo xtask snapshot` every run.
- **A timing is the best of three runs rather than one**, and every one of the
  three has to produce the same bytes. A single sample on a shared four-core
  container measures the container: an early run of the harness reported 85 ms
  for a re-entry that costs 8 ms warm, mostly paging the binary in, and failed
  the ten-fold floor for a reason that was not a regression. A minimum is the
  right statistic for a cost floor because noise only ever adds, and requiring
  the three outputs to agree makes the repetition a reproduction check rather
  than three stopwatches.
- **A size read out of a snapshot is bounded before it is believed.** A client's
  buffer width arrives from a file nothing in the process wrote and is
  multiplied by the set size and handed to an allocator; a `u32` one bit away
  from `512` asks for four gigabytes, and an allocation nobody survives is the
  process dying with a message about `alloc` rather than a refusal naming a
  field. `client::MAX_BUFFER_BYTES` is the stated bound, a test holds every
  shipped scenario inside it, and the same rule retired three `.max(1)` clamps
  on the way in — a domain's room, a peer's depth, a service's depth. A clamp is
  a *repair*, and a repaired file restores into a world that is plausible and is
  not the world the file described, which is the one failure this whole RFC is
  written against. It was worse than that for `Grants`: `Service::load` rebuilds
  a registration table and asks the recorded domain whether the rebuild landed,
  and a clamp applied to both sides made that second opinion agree with itself.
- `sim::service::Service` now keeps a journal, so a peer's memory grows with the
  number of registrations it has made. Every scenario in both tables registers
  once per client and re-registers only after a reset, so it is two or three
  entries; a workload that registered per operation would make it linear in the
  run, and that is the day this becomes a slot-level accessor in `ring/` with an
  argument to go with it.
- The label table in `snap.rs` is a table somebody has to add to. A label missing
  from it is refused **at save time by name**, which is a message naming the file
  to edit rather than a snapshot that restores into a different run, and
  `tests::every_label_a_run_can_write_is_in_the_table` runs every scenario at
  four seeds and holds every label it produced against the table.
- `Actor::save` has a default that refuses. That is a runtime refusal where a
  required method would have been a compile error, and the reason is that `Actor`
  is implemented outside the set of things a scenario installs — `chaos.rs` wraps
  actors in places, a test installs a stub. What keeps the shipped path honest is
  that `snap`'s tests snapshot every scenario in both tables at several cuts, so
  an actor that reaches a scenario without a save turns the suite red. **The
  chaos harness's actors are not saveable today, and a run of one refuses to be
  snapshotted by name.**
- `scenario::LONG` is a second scenario table, split from the first by cost. A
  forty-minute scenario in `SCENARIOS` would be multiplied by sixty-four seeds in
  every sweep and run three times per commit by `cargo xtask sim`, and it would
  change the header `sim/corpus.txt` is regenerated from. Both tables are found
  by `scenario::find` and both are fingerprinted into `snap::build`; only the
  first is swept.

**What the test does not prove, said here rather than discovered.** A field that
is saved, restored, and influences nothing after any tested cut is a field the
differential test cannot distinguish from a field that is absent. Seven cuts per
scenario per seed across both tables is a lot of cuts, and it is still sampling.
The second guard is the byte round-trip — save, restore, save again, require the
same bytes — which catches a field lost in the *load* whether or not it matters.
Between the two, a field is caught either by changing the run or by changing the
file; what neither catches is a field that does neither.

That residual is not hypothetical and it was measured rather than assumed. Three
fields were dropped on purpose, one at a time, and the suite asked:

- `App::issued` read back one short — **caught**, four tests, the first at step
  fourteen of `blk`;
- `Queue::taken`, the device's `last_avail_idx`, read back with its low bit
  cleared — **caught**, at step twenty-nine of `blk`;
- `Device::reset` read back as `false` — **not caught**, and it is the honest
  case. A device that has fallen over has cleared its jobs and told its client,
  the client reclaims and finishes, and nothing sends the device another
  message — so the flag is genuinely unobservable after the cut in every
  scenario either table ships. A field nothing can observe is a field this
  cannot see, and that sentence is the residual stated exactly.

The byte round-trip catches the third anyway, and that is why there are two
guards rather than one: the *load* dropped it, so the second save differed from
the first. What would survive both is a field lost in the *save* that also
affects nothing — and a field that affects nothing after any cut is not
observable by any means available here.

One more dependency worth writing down: a client's buffer **bytes** do not
travel. The only byte the client cares about is the pattern it stamps before
lending, it stamps it again on every submission, and `App::load` stamps it for
every buffer that is out. That is sound while no device model can reach a
client's data buffer — `Reach` is an address and a length and nothing in this
crate turns one into memory, which RFC 0041 records from the other side. A model
that could write into a client's buffer would break this, and `check::intact` is
the property that would notice.

## What would reverse this

**The two formats collapse into one** the day either half of the trade stops
being true. If the oracle becomes incremental — carrying its own state across a
cut rather than reading the whole trace — then a terse mark is judgeable and the
whole mark has no reason to exist. If a record becomes an order of magnitude
cheaper to read than a step is to take, then the whole mark is cheap enough and
the terse mark has no reason to exist. Measure it the way this RFC did: the
numbers are in `claims/0007`, produced by `cargo xtask snapshot`, which prints
both halves of the ratio every time it runs.

**The journal becomes an accessor** if any workload registers buffer sets per
operation. `Service::deeds` is bounded today by *registrations*, not by
operations, and the moment those coincide the replay stops being cheap. The fix
is then a slot-level save on `ring::registry::Table` and an argument about why a
table that can be written from bytes is still the type RFC 0028 built.

**The format grows a compatibility promise** if a snapshot ever outlives the
checkout that made it. Today it is refused by number rather than misread, and
that is right because these files live for minutes. A tree that shipped
snapshots as artefacts — a corpus of *states* beside the corpus of seeds — would
need a reader that can interpret two layouts, and that is a second definition of
what a run is.

**The build fingerprint stops being enough** the day a model's behaviour changes
without any table moving. The commit is the guard for that case and it is only
consulted when a caller supplies one; `cargo xtask snapshot` always does. If
snapshots start being passed between people rather than between two commands in
one script, the commit should become mandatory the way `--sweep`'s already is.
