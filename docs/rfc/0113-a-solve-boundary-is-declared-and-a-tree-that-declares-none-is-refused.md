# RFC 0113: A solve boundary is declared, and a tree that declares none is refused

- Status: accepted
- Date: 2026-09-24
- Affects: `interface/src/solve.rs`, `interface/src/node.rs`'s `Constraints`,
  `docs/design/ring-scene-boot.html` sections 11 and 12, RFC 0077. `TODO.md`'s
  `E3-B06e`.

## Decision

The solve stage re-solves a change from the nearest **definite** ancestor of the
node that changed — definite meaning `min_em_x100` equal to `max_em_x100`, an
extent that cannot move — and a pass whose plan exceeds a stated [`Scope`] is
**refused** rather than performed slowly. Two things follow, and they are the
decision rather than its implementation. First, *where a cascade stops is
declared by the application*, not inferred by the solver. Second, a tree that
declares no definite node between a change and its root **cannot be laid out
incrementally and is told so**: ten thousand nodes hanging under one flexible
surface is a `Cascade`, which is a refusal with the offending node named, and not
a frame that arrives late.

That second half is a new obligation on the people RFC 0077 asked to adopt this
vocabulary, and RFC 0077 does not state it. Section 11's rule is *layout is
constraints, never coordinates*, and nothing in it says any constraint has to be
definite. So this is written down as a reversal rather than added as an
implementation detail of a module.

## Context

`E3-B06e`'s exit is *work proportional to the change — a delta touching five
nodes of ten thousand re-solves five subtrees, counted; a cascade past a stated
scope fails the test rather than slowing the frame*, and section 12's own note
says the same thing as a warning: **incrementality is the entire engineering risk
in this part**, and a delta touching five semantic nodes that triggers a full
solve means the simpler design wins outright. The note also names the mechanism
it expects — *dirty-tracking through the constraint graph, and constraints scoped
tightly enough that a local change cannot cascade globally*. The second clause of
that sentence is the one nobody had written a rule for. *Scoped tightly enough*
is a hope until something says which declarations are tight enough and what
happens to the ones that are not.

Two rules were live, and the one that was not taken is the more obvious of the
two.

**Stop the upward walk when the recomputed intrinsic equals the stored one.**
This is the standard technique and it is strictly better in the common case: a
leaf that grows by two tenths of a point inside a parent with slack does not
change that parent's intrinsic, so the walk stops one level up and the cost is
two nodes. It needs no new obligation on anybody's declaration and it would have
let this module ship without an RFC.

It was refused because **whether it fires depends on the values in the tree, not
on its shape.** The same declaration is incremental under one theme and quadratic
under the next; a designer moving a metric turns a passing test red with no code
having changed; and the bound cannot be stated at all, because the statement
would have to be *usually*. `Scope::DEFAULT`'s own doc comment says the number in
it is not measured, and this tree has enough experience of numbers that are not
measured — `claims/0033` is four thresholds with no numbers — to know the
difference between a bound and an expectation. An optimisation nobody can state
is not a guarantee, and the clause being paid here asks for a guarantee: *a
cascade past a stated scope fails the test*. A test cannot fail on a cascade whose
occurrence is a property of the current theme.

**Declared definiteness** was taken instead. It has the property the other rule
lacks and only that property: it is a fact about the declaration, so a tree either
admits incremental layout or does not, and which one it is does not change when a
colour does. A definite node's extent is the same number whatever its children do
and whatever its parent offers it, so the subtree under it is re-solvable alone —
and that is one comparison, `min == max`, rather than a second field in
`Constraints` and a second RFC against RFC 0077's vocabulary.

Two smaller decisions ride along and are recorded here so they are not rediscovered.

**The scope is checked against the plan, not against the work.** A refusal that
had to walk the subtree in order to decide to refuse it would have spent the
frame it was protecting, so every node carries a maintained count of its own
subtree and the plan is a sum over at most sixty-four of them. The cost is two
bytes a node and an ancestor walk of at most seventeen steps per declaration.
Where the plan is an over-estimate it is over on the safe side: a `Flow::Own`
subtree is counted and not descended into, so a canvas makes the plan larger than
the work, and the refusal is conservative rather than optimistic.

**Identity is read through a counted door.** Not part of the layout decision, and
it is here because the exit's word is *counted* and the first draft of this module
could have been lied to. A lookup that scanned every slot comparing identities
would have read ten thousand slots while every counter stayed where it was, so the
only way to tell one node from another now costs a counted read, and the fixed
index over the identifiers is what keeps that number small. That index is the
second array `scene::arena`'s size assertion refuses to grow, and `arena::find`
names *a measured frame in which this scan shows against section 12's budget* as
what would reverse its own choice. This stage is that budget, one layer up, and it
is the same decision taken the other way for a reason written down rather than a
different taste.

## Consequences

**Easy.** An application that wants a region's changes to stay local says so, in
one place, with a number it was going to choose anyway — a panel that is four
hundred tenths of a point wide is a boundary by being that. The property is
checkable: `Layout::is_boundary` answers it, and a projection or a lint could ask
the same question of a declared tree before anything is laid out. And a bad
declaration is discovered by whoever wrote it, at the first frame, with the node
named — rather than by a user, as a frame that is late sometimes.

**Hard.** An application whose tree is genuinely fluid all the way to the root —
everything sized by its contents, nothing pinned — gets a refusal and has to
change its declaration or widen its scope. It is the trade `scene::arena`'s
`NODES_MAX` makes and it is the same shape: the structure that refuses to do
everything is what forces the question the author has to answer anyway, which here
is *which part of this interface has a size*. The refusal is recoverable in one
call, because `Scope::UNBOUNDED` re-solves the whole thing and a first frame is a
full solve regardless.

**Foreclosed.** A declaration cannot ask for *scoped, but not a fixed size* — a
maximum alone is not a boundary here, and neither is a grow share. It could have
been a third field, and a third field would be a second way to say something the
vocabulary already has a way to say, which is the argument RFC 0077 makes about
roles applied to constraints.

## What would reverse this

A cascade bound that holds **without** a declared boundary. Concretely: a solver
that can state, in the shape of a bound rather than a usually, what one delta
costs on a tree with nothing definite in it. The intrinsic-equality early exit is
not that; a summarised constraint per subtree with a proof that a change cannot
escape it would be. On that observation the refusal below `Scope` is deleted,
`Demand::is_definite` stops being load-bearing, and this vocabulary stops carrying
an obligation it does not state in `node.rs`.

A second, weaker reversal, and the one to watch first because it will not look
like one: **a cross-axis field in `Constraints`**. This module solves one axis
because the vocabulary declares one — `min_em_x100` is documented as *measured
along the parent's flow* and there is no second measurement — so `Flow::Inline`,
`Flow::Block` and `Flow::Wrap` are the same arithmetic pointed in different
directions. A second axis makes definiteness a per-axis property, and a node
definite on one axis and fluid on the other is a boundary for half of a change.
That is a different decision and it needs its own entry.

And one measurement, which this project cannot take yet and should take the day it
can: the number in `Scope::DEFAULT`. It is five hundred and twelve because it is a
bound on what a declaration may ask for rather than a time on a machine, which is
what lets it be stated before `runner-class-A` exists. The first measured solve
pass replaces it with a number that has a claim behind it, and probably a
different one.
