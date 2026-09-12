---
id: 0009
status: draft
reviewed_by: "pending: Dmitri Chudinov"
skills: spec-from-intent, rfc-author, claims-registry, determinism-review, frame-and-unsafe, licence-boundary
---

# Spec: a reversal that names a page owes that page a pointer

Seven published pages — `docs/design/*.html`, `docs/what-must-be-stated.html`,
`docs/the-long-plan.html` — carry sentences an accepted entry in `docs/rfc/` has
overturned. The information needed to find them is already in the tree: an
entry's `- Affects:` bullet names the documents it changes, and twenty-eight
(entry, page) pairs are written there today. Eleven of those pairs end with the
page citing the entry's number; **seventeen do not**, and the seventeen are
where the stale sentences live. This spec makes that arithmetic a command: an
entry whose `Affects` names a published page owes that page a citation of its
own number, the backlog is declared as a shrinking set rather than closed in one
diff, and a page keeps a reversed argument by marking it and pointing at what
reversed it — which is what the citation *is*.

## What the intent asked for that is already paid

Commit `54e9f28` paid three of the intent's examples, all in section 04 of
`docs/design/deadline-all-the-way-down.html`. This spec proposes no work for
them:

- *Atomicity is free because a root is a single write* (`intent.md:23`) is gone.
  The page publishes the four-operation barrier sequence and the
  verify-before-accept rule, naming RFC 0060 —
  `docs/design/deadline-all-the-way-down.html:216` and `:218`.
- The unqualified re-chunking sentence now carries its bound and the class it is
  false on, naming RFC 0061 and RFC 0062 —
  `docs/design/deadline-all-the-way-down.html:204` and `:205`.
- The *Where this design is genuinely bad* note no longer calls the second
  object kind a future mitigation: it is *built rather than promised*, with
  `EXTENT_BYTES`, the straddle fraction and RFC 0058 —
  `docs/design/deadline-all-the-way-down.html:221`.

What that commit did not touch is the rest of the same page and the other six
documents. The intent's third example is untouched, and so is the second half of
RFC 0058's own `Affects` line.

## Behaviour

### 1. `cargo xtask lint-reversals`

A new lint, in `lint` and therefore in `verify`, beside `lint-owed`. It reads
every `docs/rfc/NNNN-*.md`, takes **only** the `- Affects:` bullet — up to the
next line beginning `- ` at column zero — extracts every `docs/**.html` path in
it, and requires the page at that path to contain the string `RFC NNNN`.

The stopping rule is not a detail. RFC 0038 already writes the distinction this
check needs: its `Affects` bullet names no page, and a *separate* bullet says
`- Implements, and does not amend: docs/design/deadline-all-the-way-down.html`
(`docs/rfc/0038-a-core-is-allocated-and-a-kernel-entry-is-counted.md:16`), with
the reason in the entry — *naming it under Affects would have claimed a diff
this change does not contain*. A parser that read the whole header block would
flag RFC 0038 and be wrong. The opt-out already exists, it was written by
somebody who hit the problem, and this check adopts it rather than inventing
one.

Failure names the entry, the page, and the sentence an editor should look for —
the entry's own title — and states the two ways to go green: cite the entry on
the page, or move the path out of `Affects` into an `Implements, and does not
amend` bullet.

### 2. The backlog is declared, not closed in one diff

Seventeen pairs are red today, so the lint cannot gate on the day it lands. It
carries `STALE_PROSE`, a `Gap`-shaped constant beside `OWED_REVERSALS`
(`xtask/src/main.rs:405`), `CHAOS_GAP`, `SWAP_GAP` and `HEAP_GAP`: one row per
unpaid pair, each with the reason it is still unpaid and the documents that
describe it. A pair in the list is reported and does not fail; a pair neither
cited nor listed fails. Paying one is a diff that edits the page **and** deletes
its row, which is `lint-owed`'s discipline pointed at prose.

`Gap`'s fourth field exists because a gap closed in five places and was
documented in one (`xtask/src/main.rs:473`). The same argument applies here,
with the same escape: half the documents that describe these rows are `TODO.md`,
which this tree's agents may not edit, so the field stays prose the build prints
rather than paths a lint would refuse.

### 3. What the backlog holds on day one

Seventeen pairs, of which six are verified stale rather than merely uncited.

| where | what it still says | what reversed it |
| --- | --- | --- |
| `docs/design/ring-scene-boot.html:187` | the channel layout puts the SQ index ring at `0x00C0`, and the completion ring has no cursors | RFC 0018. `abi/src/layout.rs:71` says so in the code: *Section 02 puts this at `0x00C0`. It is at `0x0140`* |
| `docs/design/fast-path.html:630` | metrics table: *Full system rollback — 1 reboot* | RFC 0012: *one generation swap, and a reboot only when the frame changed* |
| `docs/design/fast-path.html:515` | *atomic generations, one-boot rollback* | RFC 0012 |
| `docs/design/lineage-and-debts.html:323` | *atomic generations and one-boot rollback* | RFC 0012 |
| `docs/design/deadline-all-the-way-down.html:303` | *mutable extents are a patch rather than a resolution* | RFC 0058, which names this line in its own `Affects` |
| `docs/design/deadline-all-the-way-down.html:302` | deadline propagation *needs designing before it is implemented* | RFC 0025, and `abi/src/deadline.rs`, which is the design |

The ring-layout row is the one to read first, and the intent's audit did not
find it. It is a **wire format**, published, wrong by 128 bytes, in the document
this project hands to anybody who might write a peer. It is the evidence for the
intent's own constraint that eight was one audit on one branch.

Three documents outside `docs/design/` carry RFC 0012's reversal and are in the
same backlog, all three unmodified since the initial import `df65e2f`:
`docs/what-must-be-stated.html:391` (F *advertises "full system rollback: one
reboot" as a target, which concedes the drawback rather than answering it*, over
an `Open` chip), `:888` (R12's worked example), and `:971` (an open-question row
premised on *the reboot the current metric concedes*). `CONTRIBUTING.md:72` is
R12 again. Line `:930` of `what-must-be-stated` already states RFC 0012's answer
in full, so that page contradicts itself four rows apart.

One row is weaker than the rest and is listed as such: `docs/design/fast-path.html:663`,
*garbage collection becomes a real engineering problem*. RFC 0059 names that
line in its `Affects` and does not falsify it — it answers it, with three
invariants and `SWEEP_LIVE_FRACTION`. The debt there is a missing pointer, not a
wrong sentence, and the row's reason field should say so.

### 4. A page keeps the argument by marking it

The check is satisfied by a citation, not by a deletion, and that is the point.
A page that wants to keep an argument whose conclusion moved keeps it and adds
the pointer — the shape `docs/design/ring-scene-boot.html:291` already uses for
RFC 0024 (*The `Drop` above is the shape and not what was built*) and `:234` for
RFC 0020 (*RFC 0020, which was written because this list said the second check
was enough*). Both were written by the diff that paid the reversal. The rule
this spec adds is that doing so stops being optional.

## Policy applied

**Determinism (RFC 0004).** The lint reads the filesystem and nothing else: no
clock, no randomness, no ordering to observe, so nothing reaches `f_env::Env`
and no `DETERMINISM_ALLOW` entry is added. Iteration is over `BTreeMap` and a
sorted path list, because `xtask` is checked by the lint it implements. It adds
**no sixth directory walker**: `documents()` (`xtask/src/main.rs:14141`) already
enumerates the pages that may cite a claim, it is the same set, and `CLAUDE.md`'s
scar about walkers carrying their own skip list is the reason to reuse it rather
than write one that reads four other checkouts.

**The frame (RFC 0001).** No `unsafe`, nothing in `abi/`, `ring/` or `kernel/`.
The one file outside `xtask/` and `docs/` that this touches is a doc comment:
`abi/src/layout.rs:71` should stop saying section 02 disagrees with it, once
section 02 no longer does.

**The licence boundary (RFC 0003).** Untouched. No `third_party/` import, no
ring crossed.

**Numbers come from the registry.** `lint-claims` (`xtask/src/main.rs:14196`)
already checks every `<span data-claim>` against `claims/`, and nothing here
edits one. Two backlog rows are numbers — `0x00C0` and *1 reboot* — and neither
is a `data-claim` span, which is why `lint-claims` never saw them. This spec
does **not** propose converting them. `0x00C0` is a wire offset and belongs
beside the layout it describes; *1 reboot* is a target RFC 0012 replaced with a
different target, and the replacement should be written as RFC 0012 wrote it,
naming `claims/0029` and `claims/0030` as where the two counts already live.

**An RFC is owed** — `E2-D07` below. Two things a future contributor would
otherwise re-litigate: that an entry's `Affects` bullet is load-bearing and no
longer purely prose, and that a page may keep a falsified argument provided it
is marked. The second is the intent's own second open question, answered rather
than carried.

## Not in scope

- **Reshaping any existing `Affects` line.** The intent forbids it and the
  directory is append-only (`docs/rfc/README.md`). The check parses what is
  written; where a path is named that should not have been, the fix is RFC
  0038's extra bullet, added by the entry's own author or not at all.
- **Deciding whether a sentence is *true*.** The check knows whether a page
  points at an entry, not whether it still says the reversed thing. RFC 0018
  proves the proxy is worth having and also shows its limit: a page could be
  silently corrected and still fail, and a page could cite an entry in one
  section while a second section still says the old thing. The honest statement
  is the intent's own — *this page has an unreviewed reversal against it* — and
  the failure message says exactly that.
- **A general number-to-claim check over `docs/`.** `lint-claims` covers cited
  numbers; uncited ones cannot be found without deciding which digits in prose
  are claims. One bounded piece is proposed as `E2-B14`: the seven rows of
  `fast-path` section 10's target table.
- **Editing `TODO.md`.** Task lines are proposed to the originator. The gap
  constants say why (`xtask/src/main.rs:473`), and RFC 0060 set the precedent of
  reporting a missing task line rather than writing one.
- **`docs/postmortem/`, `docs/sdlc.md`, `README.md`, `docs/*-boot-outside-qemu.md`.**
  Not published reasoning about decisions; outside the check's set.

## Evidence

- **A named test.** `xtask`'s suite gains a fixture for the parser, on
  `gap_holds_under`'s precedent (`xtask/src/main.rs:495`: *a check that has
  never failed is indistinguishable from a check that cannot*). The fixture is a
  directory with four entries and two pages: one entry citing, one not, one
  using the `Implements, and does not amend` bullet, and one naming a path that
  does not exist. The fourth is an error rather than a pass — a declaration
  nobody can check is what `gap_holds` already refuses
  (`xtask/src/main.rs:505`).
- **A red build, demonstrated rather than asserted.** Appending
  ` docs/design/proving-ground.html` to any accepted entry's `Affects` bullet
  turns `cargo xtask lint` red naming the entry, the page and the entry's title;
  reverting it turns it green. This is `E0-B21`'s own exit shape
  (`TODO.md:575`).
- **A document that can say something it could not.** `CONTRIBUTING.md`'s rule
  table gains a row, and R12's last column moves from `review` to
  **`cargo xtask lint-reversals`** for the half a lint can carry. R12's worked
  example is itself one of the stale quotations, so that row is edited by the
  diff that pays it.
- **No claim.** This produces no number worth a baseline and a reproduction. The
  count of unpaid rows is printed by the lint, which is where a count that must
  fall belongs; registering it would turn a shrinking backlog into a threshold
  to argue about.

## Risks and reversal

**Most likely to be wrong: citation is a weak proxy, and seventeen pairs may not
be seventeen problems.** Some will be entries that named a page and changed
nothing on it, and triaging them is most of the work — the first task's honest
output could be fourteen `Implements, and does not amend` bullets and three page
edits. That would still be worth doing, because the distinction would be written
where it can be read, but it is not what the intent expected. The observation
that would say so: the triage in `E2-B12` producing more opt-outs than edits.

**Second: the rule can be satisfied by a footnote.** `RFC 0012` pasted at the
bottom of `fast-path.html` passes the check and fixes nothing, and the check
cannot tell. What stops it is review, and saying so is the point —
`CONTRIBUTING.md:55`, *a rule claimed as mechanised that is not mechanised is
worse than one honestly listed as review*.

**Third: this could become the rule the intent forbids.** A check that says *a
page must cite an entry* is one bad step from *a page may only describe what
exists*, which would delete what the pages are for. The guard is structural: the
check reads `docs/rfc/` and never the code, so nothing in it can compare a page
to a tree. A later change that gives it one is the reversal condition, and the
RFC says so.

**Reversal condition for `E2-D07`.** If a year of entries produces a stream of
`Implements, and does not amend` bullets and almost no page edits, `Affects` was
the wrong field to make load-bearing, and the answer is a narrower one — a
`Reverses:` bullet written only when prose is falsified — at the cost of a field
that does not exist on seventy-two entries and cannot be back-filled.

## Open questions, carried or answered

- *How much can a machine check?* **Answered as the intent guessed.** The
  mechanism is *this page has an unreviewed reversal against it*, not *this
  sentence is wrong*, and the failure message uses those words.
- *What does a page do with a good argument whose conclusion changed?*
  **Answered by section 4**: it keeps it and marks it, and the mark is the
  citation the check reads. The cost to the reader on every future read is real
  and accepted, because `docs/design/ring-scene-boot.html:291` has been paying
  it since RFC 0024 and it reads well.
- *Do the `Affects` lines need a shape first, and is that a separate change?*
  **Answered: no, and no.** Twenty-eight pairs parse today out of prose written
  to taste, because every one of them spells a real path. The only shape needed
  is the stopping rule at the end of the bullet, and RFC 0038 already wrote it.
- *Is there the same drift against the claims registry?* **Carried, narrowed.**
  For cited numbers, no — `lint-claims` gates. For uncited ones, at least partly
  yes: `fast-path.html:630` publishes a target in a table where six siblings
  publish targets and none of the seven cites a claim. `E2-B14` is the bounded
  piece; whether the other target tables are the same is not answered here.

## Decided on the originator's behalf

Four, each a place where a review is worth the time:

1. **The check is a citation of the RFC number, not a marker syntax.** A
   `data-reversed` attribute would be more precise and could name the sentence.
   Rejected because the pages already cite entries in prose in four places and
   none of them would parse.
2. **The backlog is a constant in `xtask`, not a field in each entry.** It
   follows `OWED_REVERSALS`; the alternative edits seventeen append-only files.
3. **Scope is the seven `.html` documents `documents()` already returns**, which
   pulls in `what-must-be-stated` and `the-long-plan` — pages the intent did not
   name but RFC 0012 does.
4. **`CONTRIBUTING.md:72` is paid with the pages**, because R12's example is one
   of the stale quotations, and leaving it means the rule against hiding a
   concession in a metric is illustrated by a metric this project withdrew.
