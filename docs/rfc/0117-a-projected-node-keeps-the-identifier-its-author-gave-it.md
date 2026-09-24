# RFC 0117: A projected node keeps the identifier its author gave it

- Status: accepted
- Date: 2026-09-24
- Affects: `semantic/src/emit.rs` (the rule and its refusal), `interface/src/node.rs`
  (`NodeId`, which this puts an obligation on), `interface/src/solve.rs` (one
  correction of a reading, below), `semantic/Cargo.toml`

## Decision

**A node's scene identifier is the identifier its author chose, and an
application whose identifiers do not fit in a `u32` cannot be drawn.**
`f_semantic::emit` refuses such a tree with `Refused::Unprojectable` naming the
node. It does not renumber, and it holds no table from one name to another.

`f_interface::node::NodeId` is a `u64` and `f_abi::scene::CreateNode::node` is a
`u32` with zero reserved, so the two spaces do not fit and something has to
give. The alternative is a projection that mints scene identifiers and keeps a
map: every author identifier gets a second name, chosen by this stage, known to
nothing else.

The reason that alternative is refused is `E3-B06f`'s own exit. The compositor
must not be able to tell an authored delta from a projected one, and **that is a
statement about identifiers as much as about bytes**: a delta whose node numbers
were invented by the projection is a delta no author could have written, because
no author knows them. A test comparing an authored stream against a projected
one would then have to be written against the projection's allocation order,
which is the projection marking its own homework. With the identifier crossing
unchanged, the authored fixture names the same nodes the declaration does, and
the equality is between two things that were written independently of each
other's internals.

It also has a consequence past this task, and it is the larger one: every stage
downstream — the dirty set, the encoder, a debugger reading a frame, a remote
peer, the screen reader looking at the same tree — names a node by the name the
application gave it. A renumbering stage is a join every one of them would have
to perform.

## Context

RFC 0077 froze the vocabulary and says a `NodeId` is stable, author-assigned and
survives a redesign. It says nothing about range, because nothing then consumed
one. This entry is the first consumer with a narrower space, and the shape is
RFC 0113's exactly: a *declaration* acquires an obligation that the vocabulary
does not state, so it is written down where an adopter can find it rather than
discovered as a refusal.

What an author loses is four billion and change. What they would have to be
doing to notice: deriving identities from something wider than a `u32` — a
pointer, a hash, a timestamp, a database key — which is a real thing to do and
is exactly what the refusal is for. The answer is that such an author keeps a
map of their own, which is the same map this stage declined to keep, held by the
one party that knows which of its objects are on screen.

Two smaller decisions ride with this and are recorded here rather than left to
be found in a module.

- **No role projects to `Kind::Semantic`, and that is a decision rather than an
  omission.** That kind exists so that role, label, value and relations travel
  in the scene tree in the same commit as the drawing — section 07 calls it the
  cheapest correct decision in the document. The wire has seven opcodes and not
  one of them sets a role, so a semantic node emitted today would say *something
  meaningful is here* and be unable to say what, at the cost of doubling the
  graph. None is emitted. The reversal condition is an eighth opcode in RFC
  0102's shape, at which point the projection grows one delta per node and
  `f_semantic::Registry` stops being the only route by which a meaning reaches a
  reader.
- **`f-semantic` takes `f-scene`, and the direction is the whole argument.**
  `scene/` does not take `f-interface` and must not: a retained graph that
  depended on the meaning layer would be a graph the meaning layer could take
  down with it, and part III's one request of part II is that it cannot.
  Deleting `interface` and `semantic` leaves `scene` standing. The reversal to
  watch is a row for `f-interface` or `f-semantic` appearing in
  `scene/Cargo.toml`.

## Consequences

**What this makes easy.** One name per node, everywhere. A frame dumped by the
compositor is readable against the application's own declaration with no join. A
test can author a delta stream by hand and compare it against a projected one,
which is the exit.

**What this makes hard, deliberately.** An application with a wide identifier
space has to narrow it before it can be drawn, and it finds out at the
projection rather than at the declaration — `Layout::declare` accepts a `u64`
happily, because layout has no wire under it. That is one stage later than ideal
and it is not free: the sentence is in `emit`'s module documentation and in this
entry, and the earliest a check could move is `node::check`, which is the
vocabulary and does not know a wire exists.

**What it forecloses.** A compositor-allocated identifier, which
`f_abi::scene::CreateNode` already forecloses for its own reason — a client that
had to wait for one could not build a frame without a round trip per node — and
this is the same refusal one layer up.

## What would reverse this

**An author with a real reason to number past `u32::MAX`.** Not a hypothetical
one: an application in the corpus, refused, whose identifiers are derived from
something this stage has no business asking them to change. At that point the
honest repair is a wider node identifier on the wire, which is an ABI change
argued as one, and not a table in this stage — because the table is the thing
this entry refuses and adding it later under pressure would be the same mistake
with an excuse.

**A second projection that needs its own node space.** If the display projection
and the remote projection both need to put nodes in one graph and their authors'
identifiers can collide, then one name per node is no longer possible and the
question becomes whose space wins. The observation would be two trees in one
compositor with overlapping identities; the answer then is a per-client space in
the graph, which is `f_scene::arena`'s to state, not this stage's.

## Appendix: one correction of a reading, not a reversal

`Layout::placed_pt_x10`'s offset is a **cursor and not a coordinate**:
`Layout::share` starts each parent's cursor at that parent's own offset, so the
number a node carries is the sum of every offset above it. A `Kind::Transform`
composes down the tree, so sending that sum as a translation places every nested
subtree at its ancestors' offsets twice over. The projection therefore subtracts
the parent's offset, `Placed::origin_pt_x10` is that number handed over so the
subtraction has both terms, and both doc comments now say so.

This is written here because it was **found by the fixture rather than by the
code**: the authored second frame was written from first principles — a child of
a section sits at nothing, the next at 1 050 — and the projection disagreed,
because it was passing the solver's number through. A fixture whose numbers had
been read off a run would have agreed with the defect. That is the argument for
writing the authored side by hand, made by the one thing that can make it.
