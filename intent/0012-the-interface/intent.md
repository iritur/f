---
id: 0012
status: draft
originator: Dmitri Chudinov
todo: E3-00, E3-B01, E3-B02, E3-B03, E3-B04, E3-B05, E3-B06, E3-B07, E3-P01
---

# Decompose the interface epoch before it opens, and say out loud how little of it can start

*`E3-00` says the deliverable is an entry here rather than an edit to
`TODO.md`, because `docs/sdlc.md` puts an intent before a spec before a diff
and E2 had `intent/0006`. This is that entry. It is written the day the
epoch's four decisions landed and before any of its build work has, which is
the only moment at which a decomposition is neither a guess nor a
rationalisation.*

**Where `E3-00` stands, once.** Its exit is a property of `TODO.md`'s E3
section. Agents in this tree may not edit `TODO.md`, so no document written here
can close it. The decomposition is complete and is written in `TODO.md`'s line
format; the exit closes when the originator pastes it, and until then `E3-00` is
open. That is the whole of it, and nothing below argues it further.

*Revised twice, after two adversarial reviews. The dispositions are in
`spec.md`'s last section, one finding at a time.*

## Problem

E3 is twenty tasks, four of them `XL`, and this file's own rules say an `XL`
that is not decomposed by the time it starts is a planning failure. That is the
stated reason to do this work and it is the smaller half of the problem.

The larger half is that almost none of E3 can start, and the list currently
says so only in one paragraph that a reader has to find. Seven of its tasks
need hardware nobody in this project owns: a photodiode rig that has not been
built, an external energy meter, and a machine of a class `runner-class-A`
specifies and nobody has purchased. Fifteen of the registry's thirty-three rows
were `pending` on 2026-09-14 for that reason, and gate G3 *is* a time. That
sentence is written with its date and its denominator on purpose: `TODO.md`'s E3
preamble says *fifteen of thirty*, the first draft of this file repeated it, and
by the time a reviewer counted it was fifteen of thirty-one — because this
epoch's own claims kept landing underneath it. The figure that stays true is the
command: `grep -h '^status' claims/*.toml | sort | uniq -c`. Two of the fifteen
are not times at all — `claims/0034` is a ratio of integers and `claims/0035` a
share, and both say in their own opening paragraphs that this is why they could
gate where a timing claim could not.
`E3-B01` names two blockers, `E0-B12` and `E0-B15`, whose remaining halves are
a number on that absent machine and Intel UINTR, which QEMU's TCG backend
implements no part of. `E3-R01` sits behind three unshipped releases and three
unpassed gates. `E1-B15` and the supervisor `E3-B01` waits on were `[>]` when
the epoch's standing paragraph was written and one of them no longer is.

A reader who takes that in correctly concludes *not yet*. A reader who takes it
in slightly wrongly concludes *blocked*, which is a different word and a worse
one, because *blocked* is what a graph says about a task whose blockers nobody
has looked inside. The four `XL` tasks are each one line, and a line cannot
distinguish between the part of itself that needs a GPU and the part of itself
that needs an afternoon. `E3-B01` is a scene graph, a delta format, a commit
protocol, a pacing loop and a counting exercise; its exit is a *count*, and a
count gates on the machines this project already has. That distinction is
invisible at the resolution of one line, and it is the difference between an
epoch with nothing in it to do and an epoch where a third of the work needs no
machine anybody has to buy and a twelfth of it can be picked up this morning.

There is a third thing, and it is the reason to write this now rather than on
the day the first blocker lifts. The four decisions `E3-D01` through `E3-D04`
were made on 2026-09-14 and produced RFCs 0077 to 0080, four modules under
`interface/`, and one claim. They decide things the build tasks were written
before anybody knew: the vocabulary is a *closed* enum of twenty-two roles with
no `Other` and no extension range; a canvas is admitted only when it declares
its dimension, its selection and every occupant's placement; a theme is clamped
by a resolver that has already checked every ink against every ground; and the
renderer has four *named* rungs whose costs are registered as targets before
anything can produce one. A decomposition written yesterday would have said
*build the semantic layer*. One written today says *build the projection that
turns these twenty-two roles into phrases, and the day a twenty-third arrives
the compiler tells every projection*. If the decomposition waits until the work
starts, it will be written by whoever is holding the first unblocked task, from
the line rather than from the decision, and the decisions will have to be
rediscovered one at a time.

## Proposed outcome

Every `XL` in E3 has between five and fifteen tasks behind it, each with an
exit that is an observation and not an intention, and each with a `needs:` that
is honest about what it waits for — which for most of them is not what the
parent line says it is. The four `L` tasks where coarseness is doing the same
damage — the input path, explicit synchronisation, the degradation policy and
the rig — get the same treatment. Seventy-two tasks, and the useful property
is not the number but that each one of them can be picked up by somebody who
was not in this conversation.

And each `XL` **names its own subtasks in `needs:`**, which sounds like a detail
and is the difference between a decomposition and a list. `xtask`'s task struct
has no notion of a parent; a relation that is not an edge is not in the graph at
all, and without those edges the seventy-two lines are peers that happen to share
a prefix, invisible to the one command nominated as this work's evidence. The
first draft omitted them and predicted the wrong number for that reason.

Three statements that anybody can check, which is what the decomposition is
*for*:

**Six of the seventy-two can be started the morning this lands**, and none of
the six is a renderer. They are a wire format, a backend-capability vocabulary
RFC 0080 deliberately left in prose, a decision about where a text shaper comes
from, an input timestamping rule, a decision about what happens when two
vocabulary versions meet, and a bill of materials for the rig.

**Twenty-six need no task that is not already done** — every path back through
their blockers ends inside the seventy-two or at a task that is `[x]` today,
which is `E1-B04`, `E1-B07`, `E1-B15`, `E1-B16` and `E0-B13`. All six of the
photodiode rig's lines are among them, which is what makes the rig the one thing
in E3 that could be *finished* this year, and which is why it is deliberately not
blocked on the machine it will measure.

That sentence used to end *and no hardware and no purchase*, and it was false in
the same breath that listed a photodiode, an amplifier, a digitiser and an
injector. The clause is gone rather than softened, and what replaces it is the
distinction it was hiding: **these twenty-six wait on no *task*, and five of them
wait on money.** `TODO.md` has no line that says *buy a photodiode*, so no
`needs:` can name one, and the rig is at once the epoch's only finishable task
and a task nobody can start without spending. The file can express one of those
and not the other, which is worth saying once rather than contradicting twice.

**Fifteen cannot start this decade unless somebody buys a machine.** They are
the GPU rungs of the ladder and the two lines below them, the text atlas and its
reference images, and the four measurements that need `runner-class-A` or the
rig. They are named individually rather than left inside four `XL` lines, so that
the purchase order has a list attached to it and nobody has to re-derive which
work the money unblocks. Two of the fifteen are worth the purchaser's attention
first: the all-CPU rung and the descent onto it are in this list because the
image they must match is the GPU rung's, and their own tests run on a machine
with no GPU at all.

The remaining thirty-one sit behind exactly two external tasks, both `[>]` and both
with their mechanism already in the tree: the supervisor, whose restart policy
left the frame on 2026-09-13, and the blob store, which the text path needs in
order to address a font face by hash. Naming those two as the epoch's real
front door is worth more than the list, because they are the two places where
finishing something small opens something large.

And one thing that is not a task: this epoch should not repeat what
`intent/0008` found about the last one. E2's four headline numbers are all
`pending`, each for a good reason, and the finding was that four good reasons
with no owner add up to a storage story that is argued rather than measured. E3
starts from a worse position — `claims/0033` is four thresholds with no numbers
and no machine, and it was registered that way on purpose — so the answer
cannot be *measure earlier*. It is that every claim this epoch registers names,
in its own file, the task whose exit produces its number, and that the epoch's
release names the numbers it ships without. The decomposition supplies the
first half: four of the seventy-two tasks exist only to move a claim off
`pending` or to register one, and each says which. Three of the four now point at
rows that exist — `claims/0033`, and `0034` and `0035`, which were registered
while this entry was in review — and the fourth registers the boundary-crossing
count, which is a count rather than a time and may therefore gate on the machines
this project already has.

## Affected users and systems

Nothing is built here, which makes the affected list short and unusual: this
change is a decomposition and an argument, and the code it describes is E3's.

`TODO.md` is the document this exists for, and **it is not edited by this
work** — the twenty lines it decomposes and the seventy-two it proposes are
the originator's to paste, and `plan.md`'s *Handoff* is that paste written out as
ten numbered edits with every replacement line in full. Eight of the ten are
corrections to lines that are wrong today: one because a reversal was paid the
day before yesterday, one because two exits in this epoch contradict each other
and the narrowing needs an RFC, and one because three registry counts moved under
the paragraph that quotes them. The tenth is the only one that asks for code, and
it is the one this entry cannot pretend to have done: nothing in the tree
observes `E3-00`'s exit, and `plan.md` names the line that would.

`interface/` is where the four decisions live and where three of the proposals
would land first — a backend-capability vocabulary RFC 0080 owes by name, a
linearisation that generalises `canvas.rs`'s to a whole tree, and a lint. The
lint is this decomposition's proposal and not RFC 0079's request: what that RFC
names is a *signal* — *any type outside this module that holds an `Rgb` without
the `Token` pair it came from* — and `E3-B06d` proposes turning the signal into
something that can go red. `abi/` gains four entry formats before it gains
anything else, because ordering rule 1 puts a wire format first and E3 has four
of them: scene deltas, semantic deltas, input events and timeline semaphores.
`claims/` is owed **one** row and was owed three when this was first written:
the canvas-escape rate RFC 0077 registers as its own reversal condition and the
theme-refusal count RFC 0079 asks for by name both arrived as `claims/0034` and
`claims/0035` on 2026-09-14, and both already name the task that moves them. What
is still owed is the boundary-crossing count that is `E3-B01`'s exit, and it is a
count rather than a time. All three have a `ROUTES` entry in
`xtask/src/main.rs` as of this writing — `Route::Unbuilt("E3-B02")`,
`Route::Unbuilt("E3-B06l")` and `Route::Unbuilt("E3-B01")` — so `cargo xtask
claims` refuses each of them by naming the task that owes a workload, which is
the registry working. An earlier draft of this file reported the last two as
missing; they were added while it was in review. `docs/rfc/`
is owed three decisions. Two sit immediately in front of the work they protect
rather than at the head of the epoch — what happens when two vocabulary versions
meet across a ring, and where the text shaper comes from — and the third is the
narrowing that resolves `E3-B03`'s exit against `E3-P05`'s, which is a reversal
of something already written down and is the originator's to trigger.

The `docs/design/` pages that would have to change, which is the expensive part
and is E3's rather than this entry's: `ring-scene-boot.html` sections 08 and
10, where RFC 0080 has already recorded one narrowing and where a fifth rung or
a deleted hybrid would record another; section 11, whose `Node` struct this
crate departs from by one field with the reversal written down; section 12,
whose four-stage projection chain is what `E3-B06`'s subtasks are; and section
13, which is the only place in the tree that states what would falsify this
pillar and should be read again at the end of the epoch with whatever the
canvas-escape rate says.

`third_party/` is touched by exactly one proposal and it is the text shaper. If
the answer is *import one*, the licence boundary is crossed for the first time
since it was drawn, over a ring and by no other route; if the answer is *write
one*, that is a decision with a cost that should be paid on purpose rather than
arrived at by nobody wanting to file the RFC.

## Constraints

The three policies in `CONTRIBUTING.md` constrain everything and are not
restated. What is particular here:

- **This entry may not edit `TODO.md`.** The epoch's own preamble says the
  deliverable of `E3-00` is an `intent/` entry, and `intent/README.md` says an
  agent does not write its own spec and does not close its own task. So the
  decomposition is a proposal in `TODO.md`'s exact line format, ready to paste,
  and the act of pasting is a person's.
- **Every proposed task carries an exit.** `CLAUDE.md` records *a task with no
  exit is a wish* as a mistake that has happened twice. An exit is an
  observation somebody else could make: a test that passes, a number recorded, a
  boot that reaches a stage. "Implemented" is not one.
- **No exit belongs to two tasks.** `E0-B12`, `E1-B01` and `E1-B05` each
  recorded the same defect — an exit written as another task's observation means
  one of the two is always lying about its state. This decomposition does not
  add a fourth instance, and it names the one already in the epoch.
- **Ordering rule 3.** A decision goes immediately before the work it protects
  and never at the head of the epoch. The two RFCs this decomposition proposes
  are therefore subtasks of the build tasks that need them, not new entries in
  the `Decide` section — whether they are promoted to `E3-D05` and `E3-D06` is
  the originator's call and the argument is the same either way.
- **Nothing in E3 has been measured, and every number written here is a
  target.** `claims/0033` says so about itself in its first paragraph. A
  decomposition that wrote a number into an exit would be inventing evidence at
  the cheapest possible moment.
- **The decisions are inherited rather than re-argued.** Where a proposed task
  would contradict RFC 0077, 0078, 0079 or 0080, it is not proposed; where one
  pays a debt those RFCs record, it says which debt. A subtask that could have
  been written before 2026-09-14 has not used the decisions it is downstream of.
- **`interface/` is `no_std` with no allocator, and it depends on nothing.**
  That is what makes part III abandonable without taking part II down, which is
  the one property section 13 asks the compositor for. Any proposal that gives
  this crate a dependency has broken the property the epoch's own falsification
  route relies on.

## Open questions

- **Does the first compositor sleep on a doorbell, and does that retire
  `E0-B15`'s unproven clause?** That task's line says cross-core delivery is not
  proven and that it belongs with the component that will actually sleep on a
  doorbell. The compositor is that component — it is the first thing in this
  system that parks waiting for a peer's commit. So a piece of E0 closes inside
  E3, which the graph has no way to say. Should `E0-B15`'s remaining clause be
  moved onto the subtask that would observe it, or does it stay where it is and
  get closed from a distance?
- **Which half of the text contradiction gives way?** `E3-B03`'s exit asks for
  reference images in CI and `E3-P05`'s asks that no test in the tree compare
  images. Both are defensible: a rasteriser's artefact *is* pixels, and an
  interface assertion that reaches for a screenshot is the brittleness `E3-P05`
  exists to end. The narrowing is one sentence in whichever exit loses, and it
  needs an RFC because a written exit is being reversed. Nobody should discover
  this while writing the test.
- **Is the vocabulary's twenty-third role an E3 event?** RFC 0077 says a missing
  role is an RFC and a redundant one is forever, and bets that twenty-two is
  defensible against three interfaces. The corpus that would say otherwise is
  `E3-B06`'s, and the epoch has no task that ports a real application to the
  vocabulary. Should it? A canvas-escape rate computed over trees this project
  wrote itself is a number about this project's taste.
- **Who owns the rig — this epoch or the hardware lab?** `E3-P01` is the only
  task in E3 that can be finished without anything else lifting, and it is also
  the only one that needs money and a soldering iron rather than a compiler.
  `E5-P01` builds a hardware lab three releases later. Either the rig is built
  now by whoever is here, or it waits and G3 waits with it, and that choice is
  a purchase decision rather than an engineering one.
- **Does `E3-B02` really want four renderers, or three and a promise?** RFC 0080
  names the hybrid as the rung it is least sure of and says what would delete
  it. The decomposition proposes all four, because that is what the decision
  says; but nine of the twelve `E3-B02` subtasks cannot be finished without a
  GPU somewhere behind them, and the first
  machine that runs any of them will also produce the observation that could
  delete a rung. Building the hybrid before that machine exists is building the
  thing most likely to be deleted, and doing it anyway is a choice worth making
  deliberately.
