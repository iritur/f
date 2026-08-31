# RFC 0013: Every component publishes a state tree

- Status: accepted
- Date: 2026-08-31
- Affects: `abi/`, `kernel/`, `bench/`, `claims/`, `env/src/sim.rs`, RFC 0004, RFC 0010, RFC 0011, and E0-B14 which builds v0 against it

## Decision

Every component — the frame included — publishes a hierarchical, typed state
tree into a mapping its readers can only read. It is the one answer to *what is
this thing doing right now*, and it is the same answer a claim, a test oracle
and a person get.

Six properties, and each one is a refusal of the ordinary alternative.

- **The tree is a map of memory that already exists, not a serialisation of
  it.** A node names a live word — a counter a subsystem was already
  maintaining — by its offset in the published region. Publishing costs the
  subsystem nothing beyond the store it was already doing, and there is no
  collect step, no sampling interval, and no second copy that can disagree with
  the first. This is the property everything else rests on, because it is what
  makes the tree affordable enough to leave on.

- **It is read, never delivered.** No callback, no push, no event. A reader
  maps the region and loads from it; a reader that wants change notification
  drains a ring at a polling point like everything else does. Rule R05 in
  `docs/what-must-be-stated.html` section 15, and RFC 0004's contract, both
  require this, and observability is historically the first subsystem to be
  granted an exception to it.

- **A snapshot is atomic per node and not across the tree.** Each node is a
  machine word, read once, never torn. Two nodes read in one pass may be from
  different instants, and the tree does not pretend otherwise. This is the
  decision most worth attacking and section *Context* argues it.

- **The hash is over bytes, not over interpretation.** A snapshot hash covers
  the raw published region in node-id order, so two readers of different ages
  computing a hash of the same bytes get the same hash even when one of them
  cannot name every node it hashed.

- **Node ids are permanent and the schema is separate.** Names, types, units
  and hierarchy live in a schema block published once per generation. The data
  block is fixed-width records with no strings in it. A retired node's id is
  never reused, for the reason `TODO.md` never reuses a task id: the id is what
  makes two readings across time comparable at all.

- **It works with no debug build, no debugger and no instrumentation flag.** A
  tree that only exists in a debug build describes a system nobody runs. There
  is one build.

## Context

The state tree is not in the original seven layers of `proving-ground.html`. It
was added as layer 07 because it is what every other layer reads: the
simulator's oracle, the claims registry's source, and the answer to what a
component is doing. `the-long-plan.html` records the same thing from the other
end — at E2, comparing two whole-system states is comparing two hashes, and
there is nothing to hash unless this exists first.

The gap register rates it *medium* and says the important part in one line:
cheap while the kernel is two thousand lines, painful at twenty. It is
scheduled M1 to M2 for that reason and for one more — M2 produces this
project's first real measurement and gates on it forever afterwards, and the
counters that make a jitter histogram into a claim need somewhere to be
published from. A claim and a live reading being the same number is not a
convenience; it is what stops the measurement apparatus from being a parallel
implementation of the system that can drift from it.

### The part that is genuinely contentious: what a snapshot means

The kernel's mutable state is per-CPU by standing decision, and nothing under
`kernel/` locks. Two cores reach the same slot in exactly four places, each a
machine word, each an atomic with its ordering named at the access; a fifth
needs an argument (RFC 0016). A state tree spanning eight cores is therefore a
read of eight sets of words that no single instant covers.

Three answers were live.

**A consistent cut, bought with a seqlock over the whole tree.** The reader
reads a generation, reads the region, reads the generation again, and retries if
it moved. It is the standard answer and it is wrong here, because the cost lands
on the writer: every counter update in the system acquires a fence it did not
previously need, on paths that exist to have no fences on them. It would make
the observability apparatus the most expensive thing in the hot path, which is
how observability ends up switched off, which is how it ends up describing a
configuration nobody runs. And it does not even buy what it appears to: a
seqlock spanning per-CPU regions written by cores that never coordinate is a
seqlock whose writer side is eight writers, so the reader retries until all
eight are quiet, and under load they are never all quiet.

**Per-node atomicity, and say so.** Chosen. A value is never torn; a pair of
values is never promised to be simultaneous. The tree publishes, per subtree,
the monotonic timestamp at which that subtree was last written, so the *skew* a
reader is looking at is a number it can read rather than a property it has to
assume. Cross-node consistency is then bought where it is actually available:
in the simulator, where virtual time stops and a quiesced tree is a genuine cut,
and at explicit snapshot boundaries, which is what `proving-ground.html` means
by *hashed at snapshot boundaries*. E2's two-hash comparison is a comparison of
two quiesced trees, and that is the configuration in which it is meaningful.

**No tree at all; keep counters private and export them per subsystem.** This
is the status quo of every system this project is arguing with, and it is
rejected for the reason the gap register gives: it is not that per-subsystem
counters are unreadable, it is that they are not the *same* numbers the claims
and the tests read, so there are three implementations of the system's opinion
of itself and no rule about which one is right.

### Fail closed, and the one place this does not

Rule R04 says: unknown opcode, unknown flag, non-zero reserved field — refuse.
An older reader meeting a node type it does not know is the shape of case R04
exists for, and this RFC deliberately does not refuse there. The reader skips
the node, counts what it skipped, and reports the count.

The distinction is what happens next. R04 protects a decision: a request acted
on under two incompatible interpretations is a correctness failure, so refusing
is the only safe reading. A state tree reader takes no action and holds no
authority — it displays. Refusing to display anything because one node is newer
than the tool would make every old tool useless against every new system, which
is the failure mode that gets observability bypassed in favour of a debugger.
The skipped count is what keeps this honest: a reader that cannot name half the
tree says so, out loud, rather than presenting a confident partial view.

The hash is unaffected, and that is why it is defined over bytes. A reader that
skipped four nodes still hashed them.

## Consequences

**Easy.** A claim's number and a live reading become the same number by
construction, which is what the claims registry needs before an energy or
occupancy figure can be published at all. The simulator gains an oracle it did
not have to be told about: divergence between two runs is a hash mismatch that
descends to the subtree that differs, which is E2-P05 and is otherwise a
person reading two logs. A component acquires a debugging story that does not
require a debugger, which matters most for the components that cannot have one —
an imported driver behind a licence boundary, and anything running in ring 3 on
a machine with no host.

**Hard.** Node ids being permanent is a real obligation, and it is the one that
decays quietly: the cost of getting an id wrong is paid years later by whoever
compares two histories. The schema block is a second artefact that has to stay
truthful about the data block, and the only defence against the two drifting is
that the data block is generated from the same declaration the schema is — which
is a build-time obligation this RFC creates and E0-B14 has to discharge.

**The honest limit.** Everything above is about *reading*. This RFC says
nothing about a mutable path, and `what-must-be-stated.html` section 19 already
declines one: a writable state tree is a control plane, and a control plane
reached by writing to a mapping is an authority path that has escaped the
capability system. If something needs to be changed, it is an opcode on a ring
with a capability behind it.

**Forecloses.** A metrics daemon. A push-based telemetry protocol. A debug
build with counters the release build does not have. Sampling profilers as the
primary route to what a component is doing. And any future argument that
observability deserves an exception to R05, because this is that exception being
asked for and declined.

## What would reverse this

**Per-node atomicity turning out to be too weak.** The claim being made is that
consistent cuts are needed only where quiescence is available. If, during E1's
seeded-fault sweeps or E2's crash-consistency work, a real divergence is
repeatedly missed because the only way to see it was a consistent cut of a
*live* machine under load — not a quiesced one — then the position is wrong.
The evidence would be concrete: a class of bug found by other means whose
signature was present in the tree but only in the relationship between two nodes
read at the same instant. That would justify a per-subtree seqlock, writer-side
cost accepted, and this RFC should be superseded rather than quietly extended.

**The economy failing.** If publishing turns out not to be free — if the
published region's layout constraints force a subsystem to keep a counter
somewhere other than where it wants it, and the observed cost of that is
measurable in claim 0001 or claim 0002 — then the tree has stopped being a map
of existing memory and become a serialisation with extra steps. At that point
the honest move is a periodic copy with a stated interval, which is a worse
design admitted rather than a better design pretended.

**Nobody reading it.** If, at the E0 gate, the tree exists and every claim,
test and diagnosis still reaches for a serial log instead, then this is
apparatus built for its own sake. The measurement is whether E1's fault sweeps
and E2's state comparison actually consume it. Apparatus that nothing consumes
should be deleted, and deleting it is cheaper at two thousand lines than at
twenty thousand — which is the same argument that put it at M1.
