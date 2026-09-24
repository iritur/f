# RFC 0109: A blocker is named before it is owed, and E3 owes no rows yet

- Status: accepted
- Date: 2026-09-24
- Affects: `docs/TECHNICAL-DEBT.md`, which gains two blocker sections and one
  section that is deliberately **not** the register; `docs/rfc/README.md`, one
  row; RFC 0093, whose register this extends without widening what may enter its
  table; `claims/runner-class-A.md` and `claims/photodiode-rig/`, cited as the
  two specifications this tree already holds for a machine it does not own, and
  the shape the third and fourth are owed in
- Describes but does not edit: `TODO.md`'s E3 section and
  `intent/0012-the-interface/spec.md`. Every sentence below about a line in
  either is a **reading**. Nothing here narrows an exit, because RFC 0084 puts
  an exit edit in the spec and this document touches no spec — see *What this
  decided not to do*

## Decision

E3 has fifty-four open lines. Roughly half of them cannot be realised on any
machine this project can reach: a GPU, the machine `claims/runner-class-A.md`
specifies, the rig `claims/photodiode-rig/` specifies, or an energy meter with no
specification at all. Today that fact lives only inside `needs:` lists, where a
reader meeting the epoch cannot see it.

**None of those lines becomes a register row, and that is the decision** — not an
omission, and not a wait for somebody with write access to `TODO.md`.

### One. Debt is a clause a closing task gave up, and an unstarted line gives up nothing

RFC 0093's register holds eighteen rows and every one of them came out of a task
that **closed**. The mechanism is in that RFC's own words: the clause is *struck
from the exit and moved, in the same diff*, and the task is then marked `[x]` if a
half survived or `[~]` if it was nothing but the measurement. A row is the receipt
for a narrowing, and a narrowing is something a diff does.

Not one E3 line has done that. Four E3 exits have been narrowed — `E3-B01b`,
`E3-B01d`, `E3-B04c` and `E3-B07a`, by RFC 0084 — and **none of them was narrowed
on hardware**: a quantifier that was false, a ring that is a model of a ring, a
maximum over a named corpus that had been published as a bound, and a boundary
that did not exist. Those are sentences that could not survive their own first
measurement, which is a fifth thing entirely and is recorded where it belongs, in
RFC 0084. They have no register row and should not: RFC 0093's four categories are
what a *virtual machine* may not answer for, and *a sentence that was wrong* is
not one of them.

So writing twenty-three rows today would mean one of two things, and both are
refused here.

**It would mean marking twenty-three unstarted lines `[~]` owed.** `E0-P05` is the
shape `[~]` was widened for, and its own line says what earns it: *"Everything it
needs from this tree exists"* — the workload runs, the distribution is drawn, the
route exists, and the one thing missing is a machine. `E3-P02` is not that. It
needs `E3-P01`, which needs five subtasks, three of which need parts; and
`E3-B01`, which needs twelve subtasks, three of which are open. Marking it owed
would record a line as closed-out on which nobody has begun, which is the scope
cut `A-07` forbids, committed by the register rather than by an exit.

**Or it would mean rows whose tasks do not cite the RFC**, which
`cargo xtask lint-debt` refuses by name: *these row(s) in docs/TECHNICAL-DEBT.md
name a task that does not cite RFC 0093 — either the task was never narrowed and
the row is a debt nobody owes, or the clause has been answered.* The first
disjunct is exactly right about these. The check is not in the way of this
document; it is agreeing with it.

### Two. What the page gains instead is the machines, named per line

A blocker does not become visible by becoming a row. It becomes visible by being
on the page. `docs/TECHNICAL-DEBT.md` already has the section for that — *The
blockers* — with six entries, and it already carries E3's rig: the photodiode
section names `E3-P02`, names `E3-R01` behind it, and separates the three
`E3-P01` subtasks that close on the near side of the purchase from the three that
do not. That paragraph is the model for this whole decision, and it was written
before any of those lines closed.

So the page gains **two blockers it did not have**:

- **A GPU.** Every line of `E3-B02`'s ladder above rung 3 waits on compute
  shaders, and so, at one remove, do the display projection, the text atlas and
  the reference-image comparison. The page had no entry for it at all.
- **An energy meter.** `E3-P07` is *energy per frame, external meter rather than a
  model*, and **no line in `TODO.md` acquires one**. That is the sharpest thing
  this document found and it is recorded as a finding below as well as a blocker.

and one section, *E3 is open, so its blockers are machines and not yet rows*,
which is machine-first: each machine, the lines that wait on it directly, the
lines that wait on one of those, and — per line — the half a virtual machine can
close, named concretely enough to start on tomorrow, or the honest admission that
there is none.

**Machine-first rather than line-first is the load-bearing choice**, and it also
keeps the section out of `lint-debt`'s way: `register_rows` makes a table line a
row by its **first cell**, so a table whose first cell is a machine is not a
register of narrowed clauses and cannot be mistaken for one by the check or by a
reader. That is a convenience, not the argument. The argument is that *which
machine* is the fact a reader needs and the fact a `needs:` list hides — `E5-D01`
appears six times in E3 and means *a GPU* every time, and `E5-D01` is a document.

### Three. A line blocked by a line that is blocked by hardware is recorded as that

Flattening the two kinds together is the failure this section exists to avoid,
because the second kind is where the cheap work is.

**`E3-B02h` is the specimen.** Rung 3 is the all-CPU raster. Its own exit says
*the control is that the test runs on a machine with no GPU at all, which is every
machine this project currently has* — so the subject of the test is available,
free, and sitting under this container. It waits on a GPU anyway, and its `needs:`
comment says why in one clause: *the image it must match is rung 1's, which is why
this line waits on a machine its own test does not use.* The blocker is not the
renderer. It is **the reference**.

That distinction has a price and a reversal, and both are nameable. RFC 0091
settled that an image may be compared to an image only where the artefact under
test is pixels; it did not settle *whose* pixels are the reference when the
canonical producer cannot run. A rung-3 raster checked bit-exactly against a
serial CPU reference of its own is a real, seeded, reproducible test that can be
written this week, and it is strictly weaker than *identical to rung 1* — the
whole point of `Fidelity::Exact` is a claim relating two implementations, and a
test comparing rung 3 to rung 3's own model does not make it. So the buildable
half exists, is worth building, and **narrowing the clause to it is not this
document's to do**; see below.

Five more lines are on the page for the same reason and each is named with what it
waits on rather than with a machine: `E3-B02l`, whose subject is a machine with no
compute shaders and which therefore comes off this page the day `E3-B02h` renders
at all; `E3-B03g` and `E3-B03h`, whose counts and size rule are CPU-side;
`E3-B03j`, whose comparison needs two images and no GPU; and `E3-B06g`, whose
exhaustive `Role` match is a compile-time property available today and whose
*reaches pixels* is not.

## Context

The standing instruction for this round was that work needing real hardware goes
into a technical-debt document with only the virtual-machine half realised. Read
quickly, that instruction says *write the rows*. Read against RFC 0093 it says
something narrower, and the difference is this document.

Three measurements were taken before deciding.

**`lint-debt` passes today and says how many.** The register holds eighteen rows
and `TODO.md` holds eighteen declarations, all in E0, E1 and E2. Every one is a
task that closed. There is no E3 declaration and no E3 row, and the two sets being
equal is the state this document preserves.

**Every E3 line that cannot be realised here is `[ ]`.** Not `[>]`, not `[x]` with
a struck clause — unstarted. The two E3 lines that spent time at `[>]` on
something a container could not observe, `E3-B01h` and `E3-B04a`, both closed, and
**neither closed on hardware**: `E3-B01h`'s *on both architectures* was answered
by the `tests (AArch64, weak memory)` job on `ubuntu-24.04-arm` at commit
`cbd58e8`, and `E3-B04a`'s *one time source in the whole path* was answered by
`E3-B04d` giving `at_interrupt` a caller in a boot. Both are the opposite of debt:
a gap named, watched, and paid by a runner that already existed. An arm runner is
free and a GPU is not, and the page should not learn to say those in the same
voice.

**Fourteen open E3 lines are reachable in this container**, and three of them are
being built while this is written. `E3-B05b`, `E3-B05c`, `E3-B05f`, `E3-B06d`,
`E3-B06e`, `E3-B06f`, `E3-B06h`, `E3-B06m`, `E3-B07c`, `E3-B07d`, `E3-B07g`,
`E3-B01i`, `E3-B01j` and `E3-B03c` name no machine, and a row for any of them
would be a false record that no check in this tree can catch. `lint-debt` compares
two sets of task ids; it cannot know whether a clause was reachable. The list
above is written into the register section for the same reason RFC 0093 wrote its
four categories down — so that a reviewer can refuse a row without knowing the
subsystem.

## What this decided not to do

**It does not narrow `E3-B02h`**, and the reason is a rule rather than caution.
RFC 0084 settled that an exit is something already written down and that a
narrowed exit is *edited in the spec, with the RFC number beside it* — because a
narrowing that lives only in a document leaves
`intent/0012-the-interface/spec.md` asserting something the tree does not do, and
that file is a paste-ready handoff, so a false line there becomes a false line in
`TODO.md`. This document is confined to `docs/`. Writing the narrowing here and
not there would produce exactly the split RFC 0084 calls *a false line waiting for
a paste*.

That RFC's own recorded hazard is the other half of the reason: **a citation is
cheaper to write than a decision**, and it has now been caught four times, every
time by a reviewer reading the cited document to see whether it said what the
citation claimed. Twenty-three rows citing RFC 0093 for lines RFC 0093's own
mechanism does not cover would be that failure at scale, and it would be this
document's first act.

So the narrowing is **charged to a line rather than performed**, which is the
shape RFC 0084 used for `SET_EFFECT` and RFC 0102 later closed: the clause, the
half that survives it, and the machine are written on the page, and whoever opens
`E3-B02h` edits the spec, cites RFC 0093, and moves the clause into the table in
the same diff. The page's section says so per line, so that diff has a sentence to
move rather than one to invent.

**It does not add a row for `E3-B06l`.** Its exit asks for *a corpus of
applications ported by somebody who did not write the vocabulary*, which reads
like RFC 0093's fourth category — a second person. It is not one. RFC 0110 settled
the same requirement for `claims/0035` this week without buying anything: a corpus
is a directory with recorded provenance, and the module's own demonstration is
refused **by value** so that a flattering share is not expressible. What
`E3-B06l` lacks is applications, of which this tree has one declaration, and that
is unstarted work. The residue is named rather than padded over: RFC 0110's
by-value refusal has no analogue for an application, so *independence* there is
provenance a reader trusts rather than arithmetic a run performs, and nobody has
written the line that would write the corpus.

**It does not add rows for `E3-P04`, `E3-P05` or `E3-P06`.** Each waits only on
`E3-B06` or `E3-B07`, and each is listed in the new section as derived so that a
reader does not count it twice — but `E3-P05`'s own exit is a property of the
tree's tests, narrowed already by RFC 0091, and what it waits for is a UI suite
rather than a machine. Unstarted work behind unstarted work is not debt at either
depth.

## Consequences

**The register stays a receipt and does not become a queue.** RFC 0093's reversal
condition includes *the register growing past what a reader will read*, and says
the answer to that is not a longer page but re-arguing the narrowings.
Twenty-three rows for lines nobody has begun would have taken the page from
eighteen entries to forty-one in one diff, and the first thing a reader would
learn from it is that thirty-nine of the forty-one are things to do. The new
section is prose and a machine-first table on purpose: a reader can be told *what
this project does not know about itself* without the page pretending each sentence
was once paid for.

**`cargo xtask lint-debt` keeps meaning what it means.** It is the only check
behind RFC 0093 and its two failure messages are both about a receipt: a narrowing
with no row, and a row with no narrowing. Neither is a statement about a blocker.
Adding a third meaning to the same set comparison would have cost the two it has.

**Two blockers are visible for the first time and one of them has no
specification.** `claims/runner-class-A.md` is 209 lines and
`claims/photodiode-rig/` is twelve parts with an error bar stated before a part
exists. The GPU has `E5-D01`, whose exit is a bill of materials and whose line
carries no machine half at all. The energy meter has nothing — no line, no
directory, no stated error bar, and no `needs:` entry on the four exits that
depend on it. `E3-P01a` exists because a purchase with no specification is a
blocker nobody has costed; this is that, un-prevented, and it is now on the page
where somebody can decide whether to write the specification or drop the exits.

**What this does not touch.** No exit moves, no claim's status changes, no task's
marker changes, and `TODO.md` gains nothing. Eighteen rows in, eighteen rows out.

## What would reverse this

**A line closing on a narrowed clause, which is the reversal this is designed to
take one line at a time.** The day `E3-B02h` lands against a serial reference of
its own, the *identical to rung 1* clause is struck in
`intent/0012-the-interface/spec.md`, the line cites RFC 0093, and a row enters the
table in that diff — and the new section loses a line. The section shrinking as
the register grows is the correct behaviour and is what it is for. If it instead
grows while the register does not, the classification in it was wrong.

**A machine arriving, per machine, and this is where naming which one pays.** A
GPU retires five lines directly and eight more behind them; the rig retires two
subtasks and unblocks four; a class-A machine with F on it retires four times; and
an energy meter retires exactly one E3 line and three later ones. No single
purchase moves this section by more than half, which is a fact about E3 that a
single flat list of blockers cannot state.

**The section reaching a size where it is the queue.** It names twenty-two lines
today, of which nine are derived. If it reaches the whole epoch, the finding is
not about the page: it is that E3 was decomposed against machines this project had
decided not to buy, and the honest response is to re-argue the decomposition
rather than to keep a longer register of it.

**A reader finding a line in it that was reachable all along.** That is the
mistake this document can make, `lint-debt` cannot catch it, and the defence is
the list of fourteen reachable lines written beside the table so a reviewer can
see what was deliberately excluded. If one is found, the repair is to build it and
delete the line, with a note here saying which it was — the discipline RFC 0093
set for itself and then had to apply twice, to `E2-B02` and `E2-P10`.

## What this found and could not fix

`TODO.md` is not an agent's to edit. The readings below are handed to whoever
pastes, and they are findings rather than suggestions: each is a sentence in that
file that is wrong, or that makes a machine out of a document.

**One. Six E3 lines name `E5-D01` as their blocker, and `E5-D01` is a document.**
`E3-B02d`, `E3-B02e`, `E3-B02f`, `E3-B02g`, `E3-B02i` and `E3-B02k` carry it in
`needs:`; its exit is *a bill of materials anyone can buy, published with the
first hardware claim*. A bill of materials is not a GPU, so the day that line
closes, `cargo xtask todo` will rank five rasteriser stages as available and none
of them will be. `E0-D10` is the same shape handled correctly: its specification
half is met, its **machine half** is struck and is a register row, and the lines
that wait on class-A silicon name `E0-D10` *and* `E0-P18` together so that the
wait is on a boot rather than on a file. `E5-D01` has no machine half to narrow
and no row, and nothing in E3 names an id meaning *the GPU exists*. Either
`E5-D01` gains that clause — at which point it also gains a register row,
honestly, because it will have closed against less — or the six lines are waiting
on a task whose completion changes nothing about them.

**Two. `E3-P07` names an instrument that no line in the file provides.** *Energy
per frame, external meter rather than a model*, with
`needs: E3-B01, E0-D10, E1-D06`. The meter is in the title and in neither the
`needs:` nor anywhere else in `TODO.md`: there is no bill of materials, no
`claims/` directory, no stated error bar, and no task that buys one. Three other
exits depend on the same absent instrument — `E5-P03`, `E5-P04` and E7's
watts-per-token line — and `bench/src/lib.rs` carries `joules_per_op` as
`Metric::Unavailable` with RFC 0006 naming `E5-B07` as the day it stops being.
The rig was given six subtasks and a twelve-part specification precisely so that a
purchase would not be a blocker nobody had costed. The meter was given nothing.

**Three. `E3-B02j`'s `needs:` makes a decision wait on its implementation.** Its
exit is *one scene chosen and named in the claim's `[workload]` row with its
content hash, and the argument recorded is representativeness rather than
reachability* — a choice, an argument and a hash, all of which a text editor
produces. It waits on `E3-B02i`, rung 4, on a GPU. RFC 0088 settled that a decide
task's exit may not require its implementation; this is a build line whose exit is
a decision, and it is the same shape. Moving it would make the one piece of
`E3-B02` that is pure judgement available now, which is when a scene chosen for
representativeness rather than reachability is worth choosing.

**Two smaller readings, recorded without a recommendation.** `E3-P02`'s
`needs: E3-P01` reaches `E3-P01f`, the rig's method document — and `E3-B04`'s own
`needs:` comment already rejects that reasoning for itself in as many words: *a
document is not a prerequisite for a measurement*. One of the two siblings
corrected the blanket parent and the other did not. And `docs/rfc/README.md`'s row
for RFC 0103 ends *both `E3-B04a` and `E3-B04d` stay `[>]` on that*; both are
`[x]` since 2026-09-23, on `cargo xtask input deliver`. That row is append-only
evidence of a decision taken when the sentence was true, which is the treatment
two other paragraphs in that file already carry — so the repair is a sentence
appended, not an edit, and it is the coordinator's call rather than this
document's.
