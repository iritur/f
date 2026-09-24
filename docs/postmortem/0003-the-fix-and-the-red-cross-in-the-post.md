# Post-mortem 0003: the fix and the red cross crossed in the post

- Date of the incident: 2026-09-23 03:11 UTC to 2026-09-24. One night red, one
  day unread.
- Date of this document: 2026-09-24
- Detected by: a wave coordinator handing the red run to a builder as a task.
  Not by a person reading the run, not by a band, not by a notification — the job
  opens no issue by design, which `nightly.yml`'s own header states and
  `claims/0030`'s `[workload]` note calls the residue every scheduled job has:
  *a red cross on a schedule is the easiest thing in a repository to stop
  seeing.* It was.
- Commits: caused by `1dbb5ac` raising `multiboot::MAX_MODULES` from eight to
  sixteen while `kernel/src/main.rs`'s `MAX_RESERVED` stayed the literal `13`;
  surfaced on `b74807ca`, the merge of pull request #76; the cause was fixed in
  `84b10b7` **the day before the run went red**, on a branch, and reached `main`
  in `a03b966`. The detection gap this document is actually about is fixed here.

## What happened

The nightly job *a generation broken, rolled back, and compared three ways* was
green on 2026-09-20, 21 and 22, and red on 2026-09-23 at 03:11 UTC on
`b74807ca`. It printed:

```
xtask: the boot selecting f.root=a7912d2f… f.frame=e824b3ac… exited Some(35)
  rather than 33.
FAIL: the generation: no module this machine was offered folds to the root it
  was asked for
    offered     1 boot module(s), 0 of them refused
xtask: 11 finding(s) against claims/0030-rollback-comparisons.toml:
```

Eleven findings: one for the workload failing, and ten of *a threshold no
workload printed*, because the workload died at step five of seven and the rows
are the last thing it prints. All ten were symptoms of the one.

`cargo xtask rollback` step five offers **both** generations as boot modules and
lets the token choose. Eleven modules were placed. `MAX_RESERVED` was thirteen;
`reserved_ranges` filled three fixed ranges and then ten modules, hit the bound,
`break`, and returned. The eleventh module was never reserved, the frame
allocator was handed a region with a live module inside it, and the first caller
— the page-table builder — got its first frame. The same boot log says so:

```
  address space 0x0000000000386000 root, direct map at 0xffff800000000000
  module        0x0000000000386000..0x00000000003b45e0  185 KiB
```

By the time `f_generation::select` looked, that module's `MODULE_MAGIC` had been
overwritten with a page table, so it was not a boot module any more and the
count was one instead of two. The frame reported exactly what it saw.

Meanwhile, on 2026-09-22 — a day *before* the red run — commit `84b10b7`
rederived the bound as `5 + multiboot::MAX_MODULES` and RFC 0101 wrote the
diagnosis down in the same words. `b74807ca` was the merge of an earlier state of
the same branch. So the night the job reported the defect, the repair was already
written and reviewed somewhere else, and nothing connected the two.

## What made it possible

A bound written against what a boot contained on the day it was written. RFC 0101
already names the species and this is its third instance; what this incident adds
is the second half of the mechanism: **`reserved_ranges` stopped at the bound and
said nothing.** The list it returned was byte-for-byte the shape of a list nothing
had been dropped from, so every caller downstream was correct about wrong input.

That is why the symptom appeared in the generation fold. Three subsystems
separated the cause from the sentence in the log, and the sentence in the log
named the one subsystem that was working.

## What made it invisible

Two things, and the second is the expensive one.

**The route is in the nightly and nowhere else.** `cargo xtask rollback` is three
generation builds and six boots, about five minutes, and `claims/0030` argues at
length — correctly — that this does not belong in `verify` or in the
pull-request gate. Five green local gates said nothing, because nothing in any of
them offers a boot more than ten modules, and nothing in any of them evaluates
`MAX_RESERVED` against `MAX_MODULES`: that relation lived in a doc comment.

**Nobody read the cross for a day.** The job opens no issue, on the deliberate
argument that a deterministic failure reproducing from a command in the claim has
nothing a notification would carry that the report does not. That argument is
about the *content* of a notification and is right; what it does not cover is
whether anybody looks. Post-mortem 0002 is the same directory's record of a
nightly that was red five nights running and read by nobody, and its lesson was
about the file's own reliability. This one is about the reading.

## What was true that we believed was not

That the expensive route was the only place this could be caught, because the
property needs a boot menu and a boot menu needs generations built.

It is not true. Two cheaper checks reach the same defect and neither needs a
build:

- The relation is arithmetic over two integers in two files. Evaluating it is a
  file read.
- By 2026-09-24 an *ordinary* boot places twelve modules, so the same
  `MAX_RESERVED` of thirteen now fails `cargo xtask run` — which `verify` runs.
  The defect had outgrown the expensive route and nothing noticed, because the
  cheap route could not tell a dropped reservation from a reservation it never
  had.

The believed-necessary cost was a consequence of the silence, not of the
property.

## What changed

- **lint:** `cargo xtask lint-bounds`, in `lint_all` and therefore in
  `cargo xtask verify`, and in `.github/workflows/ci.yml`'s policy job — four
  file reads, no build. It evaluates four boot-path bounds against the counts
  that decide them: `MAX_RESERVED` against `FIXED_RESERVED + MAX_MODULES`,
  `MAX_MODULES` against the width of a two-generation menu, `RESERVATIONS_MAX`
  against twice `PLACES_MAX`, and `PLACES_MAX` against `COMPONENTS`. Reverting
  `MAX_RESERVED` to `13` makes it red in one second; that is the mutation, and it
  is in the report this came from.
- **the frame:** `reserved_ranges` counts what did not fit and
  `refuse_dropped_reservations` ends the boot naming the bound. With
  `MAX_RESERVED` at thirteen, `cargo xtask run` now prints
  `FAIL: the reserved list: 2 module(s) did not fit in 13 range(s)` at the place
  it happened, instead of an identity failure three subsystems later.
- **the RFC:** RFC 0116, which is where *stops at the bound and says nothing* is
  reversed and where the residue is named — `FIXED_RESERVED` is a count of
  assignments in a function body and the lint cannot check it.
- **the claim:** `claims/0030`'s statement rows were stale — 65 348 module bytes
  and four component files, against 8 and a number that moves with every
  component image. Both thresholds are floors and neither gated wrongly, so
  nothing was published that was false; the *statement* was, and it now says
  when it was measured and why the number moves.
- **the document:** `docs/booting-on-hardware.md`'s limits table said
  `MAX_MODULES` was 8 and said a dropped module is *reported*. The first was
  stale by two commits and the second was true of the loader's table and false of
  the reservation list beside it, which is exactly the half that bit.

## What we chose not to change

**Moving `cargo xtask rollback` into `verify`.** Five minutes and three
generation builds per commit, for a property that moves when a *format* moves.
The claim's own `[workload]` note makes this argument and it is still right; what
was wrong was believing that made the nightly the only possible detector.

**Opening an issue from the nightly.** Considered, and the argument against it
in `nightly.yml`'s header still holds for the content. But the reading gap is
real and unaddressed by anything here: this document names it and does not close
it, which is the honest state. The next occurrence of *a red schedule nobody
read* is the second one, and that is the twice this repository's rules ask for
before a mechanism is built.

**A `const` assertion instead of the lint.** `MAX_RESERVED` is now derived from
`MAX_MODULES`, so an assertion tying those two is tautological — it would have
caught `1dbb5ac` and catches nothing today — and it says nothing about the other
three relations. The lint subsumes it and fails with a sentence instead of a
compiler note about an unmet const bound.

**Raising the bound and leaving the silence.** That is what bought the previous
day.
