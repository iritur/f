# RFC 0065: A component declares its state tree, and a spawn refuses one that does not

- Status: accepted
- Date: 2026-09-07
- Affects: `abi/src/manifest.rs`, `abi/src/state.rs`, `abi/src/lib.rs`,
  `kernel/src/component.rs`, `kernel/src/state.rs`, `kernel/src/process.rs`,
  `xtask/src/manifest.rs`, `docs/manifest.md`, every `manifest.toml`, `sim/`,
  RFC 0013, RFC 0008, RFC 0030, and E1-B15 which builds against it

## Decision

A component's state tree is **declared in its manifest**, written into the
component's own page by the frame at spawn, and **mounted under the frame's
root**; a manifest that declares no tree is refused at the spawn with
`ADMISSION/NO_STATE_TREE`.

Four parts, and each one is a refusal of the obvious alternative.

- **The declaration is in the manifest and not in the component's image.** RFC
  0013 says the schema block is published once per generation and that the data
  block has to be generated from *the same declaration the schema is* — a
  build-time obligation it creates and names as the only defence against the two
  drifting. `[[state]]` is that declaration: a node's permanent id, its name, its
  kind, its unit, and the name of the node it hangs under. Sixteen nodes at most,
  because the region is one frame.

- **The frame writes the description and the component writes the numbers.** At
  spawn the frame charges one frame to the component's account, writes the header
  and the schema block into it out of the record, maps it writable into the
  component at a fixed address, and never writes another word in it. The words
  the component fills are its own, and there is no path by which anything else
  fills them — a mapping somebody else writes that a component acts on is a
  control plane, and `what-must-be-stated.html` section 19 already declines one.

- **The frame's root names every component's tree.** A `mount` node is a new
  node kind whose word is the physical address of another published region, zero
  when nothing is there. The frame writes one per place and then *reads it back
  through the node it just wrote*, binding an ordinary reader to the address and
  requiring the header, the schema and every offset in them to validate.

- **A component that publishes nothing is refused at spawn.** Not at
  `Record::read` — a record with no nodes is well formed, and what is wrong with
  it is a decision the supervisor takes about what it will host. So it is an
  `ADMISSION` refusal with a code of its own, taken before a frame is spent,
  beside `ADMISSION/MEMORY` and for the same reason: a component does not start
  and then discover nobody can see it.

## Context

RFC 0013 was accepted at E0 and E0-B14 built it — for the frame. E1 then put
more *into* that tree without changing who owns it: three drivers and a runtime
each hand the frame a tally and the frame publishes it under its own root. Two
places in the tree recorded that as the debt it was — `user/store/src/report.rs`
explains why a runtime reports an exit status instead of a tree, and RFC 0038
reads a component's ring cursor out of the frame and says the honest version is
a tree the component publishes. Neither reversal had an owner.

What forced the question is E2. `the-long-plan.html` says that at E2, comparing
two whole-system states is comparing two hashes; `E2-P05` is that comparison, and
a whole-system state is one only when every component publishes into it. RFC
0013 states its own test of worth in the same terms — *whether E1's fault sweeps
and E2's state comparison actually consume it* — and neither could, because there
was one tree and it belonged to the frame.

### The part that is genuinely contentious: where the declaration lives

Three answers were live, and the argument between them is the whole of this RFC.

**A constant in the component's own crate, published by the component at
start.** The obvious answer, and the one that keeps the schema next to the
counters it describes. It fails on the refusal. A supervisor cannot refuse what
it cannot see, and it cannot see a constant inside an image it has not run — so
*every component publishes a tree* would become *every component that got as far
as its first instruction and remembered to*, which is the tolerance RFC 0013 was
written against, arrived at from the other side. It fails a second time on the
component that is spawned and never scheduled: this tree's boot builds four
places and schedules none of their occupants, and under this answer all four
would be unreadable — indistinguishable, from outside, from four components that
had crashed before publishing.

**The manifest declares it; the frame writes the schema; the component writes
the words.** Chosen. It buys the refusal, it buys a readable tree with zeros in
it before the first instruction, and it puts the declaration inside the content
hash a spawn names — so a component whose *account of itself* changed is a
different component, which is the rule already applied to its code. It costs
something real and the cost is stated rather than discovered: a node's name and
hierarchy are now chosen where the manifest is written and the counter is kept
somewhere else, and the two can drift. That is the same cost RFC 0013 already
accepted for having a schema block at all; what makes it survivable is that the
ids are permanent, so a drifted *name* is a documentation bug and never a
comparison that silently means something else.

**No declaration; the frame discovers the tree by looking at the page.** The
frame maps a page, the component publishes into it whenever it likes, and the
frame validates whatever it finds. This is the most flexible and it is the one
that cannot fail closed: an empty page and a page a component has not got round
to are the same bytes, so *refused* and *not yet* are indistinguishable, and the
refusal that keeps the whole thing honest becomes a timeout somebody has to pick
a number for.

### Why a schema bump rather than a reserved byte read leniently

`Record` had three reserved bytes and this takes one. RFC 0030 priced a schema
bump when it made a manifest compiled rather than parsed — every component file
is rebuilt — and this is the second change to pay it. The alternative was to
read a zero in a schema-2 record as *declares no tree* and let it through, which
is the same silence this RFC exists to refuse, with a version number on it.

## Consequences

**Easy.** `E2-P05` becomes possible: there is a root, every component hangs
under it, and comparing two whole-system states is comparing two hashes reached
by walking one tree. A fault sweep can assert a component's response out of the
component's own state rather than out of a log — `sim/src/fault.rs` does it, and
that is RFC 0013's test of worth being taken rather than restated. A component
acquires a debugging story that survives never being scheduled.

**Hard.** Every manifest grows a section and every component file grows 448
bytes. A component whose honest account of itself is more than sixteen nodes
cannot have one without moving a constant and a page count. And a node declared
in a manifest with no counter behind it is a lie nothing checks — the frame
cannot know what a word counts, so `[[state]]` can describe a component more
generously than the component describes itself. The only thing that catches that
is the same thing that catches an over-generous claim: somebody reading both.

**The honest limit, and it is the reason `E1-B15`'s second clause is not fully
paid.** Publishing is now possible for every component and *taken* by none of
them: the four components this tree spawns are never scheduled, so their trees
are readable and full of zeros. `user/store/src/report.rs` still packs its tally
through the door, because the runtime path builds its process a different way and
does not map this page; RFC 0038 still reads a ring cursor out of the frame. Both
reversals now have somewhere to land and neither has landed, and saying so is
better than a mechanism whose first consumer is a document.

**Forecloses.** A component that is observable only while running. A supervisor
that hosts something it cannot read. A discovery protocol for state trees. And
any future argument that observability is optional per component, because this is
that argument being made and declined.

## What would reverse this

**The declaration drifting from the counters, measurably.** The claim is that a
manifest-side declaration is worth the split because it buys a refusal that
cannot be evaded. If, over E2 and E3, components accumulate nodes whose names
stopped describing what the component stores — found by reading a tree and a
crate side by side, more than once — then the split has cost more than it bought.
The remedy is not to move the declaration back: it is to generate the manifest's
`[[state]]` from the component's own source at build time, which keeps the
refusal and removes the second place to be wrong. That is a build step, an RFC,
and a change to `cargo xtask component` rather than to the frame.

**Sixteen nodes turning out to be the wrong bound.** If a component's honest
account of itself does not fit — a driver with a per-queue subtree, a store with
a node per zone class — then the region stops being one frame. The evidence is a
manifest somebody wanted to write and could not, and the change is
`STATE_NODES_MAX`, `FIXED_PARTS` and a page count in `process::SPAWN_TREE`.
Nothing about the format moves, which is why the bound is a constant and not a
property of the wire.

**Nobody publishing into the trees.** This RFC makes it possible for every
component to publish and requires none of them to publish anything in
particular. If, at the E2 gate, every mounted tree is still all zeros — because
no component's own loop ever writes a word — then what was built is a schema
distribution mechanism and not an observability one, and the mounts should be
deleted along with RFC 0013's apparatus on that RFC's own terms: *apparatus that
nothing consumes should be deleted, and deleting it is cheaper at two thousand
lines than at twenty thousand*. The measurement is a boot in which at least one
mounted tree's snapshot differs from the snapshot of the same tree at the moment
it was mounted.
