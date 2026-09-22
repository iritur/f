# RFC 0105: An `XL` without a decomposition is refused by a machine, from the day its epoch is decomposed

- Status: accepted
- Date: 2026-09-23
- Affects: `CONTRIBUTING.md` (the review table gains R13), `xtask/src/main.rs`
  (`lint-decomposition`, and `lint_all`), `.github/workflows/ci.yml`

## Decision

A rule leaves `CONTRIBUTING.md`'s **review** column and becomes an executable
check. It is the first row of that table to move, and the table's own subject.

`TODO.md` has said since E0 that an `XL` not decomposed by the time it starts is
a planning failure, and `E3-00`, `E4-00`, `E5-00` and `E6-00` each carry *this
epoch contains no `XL` task without a decomposition* as their exit. **Nothing
observed any of the four.** Every subtask line in E3 could have been deleted with
the whole local loop green — which is R01, *name the mechanism, not the
intention*, applied to the file that schedules the work.

`cargo xtask lint-decomposition` fails on any task sized `XL` whose `needs:`
names fewer than five ids extending its own id, in an epoch whose `E<n>-00` is
`[x]`. It is in `lint_all`, so `cargo xtask verify` runs it and `lint-gate`
requires the workflow to name it.

## Context

Two choices are decisions rather than readings, and are recorded so the next
reader does not re-litigate them.

**The condition is the `E<n>-00` box, not a date or a status.** Without it the
check is red the day it lands, on E4, E5 and E6 — whose `XL` lines are precisely
what those epochs' decomposition tasks are *for*. Gating on the box means the
rule arrives exactly when the epoch claims to be ready for it, and it means an
epoch cannot tick its decomposition task while leaving an `XL` undecomposed,
which is the pair of failures that would otherwise trade places.

**Five, and it is a floor rather than a target.** `E3-00`'s own text says each
`XL` becomes *five to fifteen tasks with exits*. Five is the smallest number that
is not obviously a gesture; a parent with four children is a parent somebody
split once and stopped. It is a bound this file states, not a measurement, and
it should move on a measurement — an `XL` that genuinely decomposes into four —
rather than on an argument about it.

This check is also the one that would have caught `E3-B08` itself. That line was
specified in full in `intent/0012-the-interface/plan.md`, argued in four places
in `spec.md`, and **never pasted into `TODO.md`** — a decomposition whose own
output was incomplete, found by a person a week later. The rule one level up is
what it now enforces.

## Consequences

**What this makes easy.** Opening an epoch. The tool says what is undecomposed
instead of a reviewer noticing, and the fixture in `xtask` means the rule is
tested rather than trusted.

**What this makes hard.** Adding an `XL` to an opened epoch without doing the
work. That is the intent.

**What it costs `CONTRIBUTING.md`.** The review table is now a table with a moved
row, and the column a row sits in is load-bearing: *review* means a person, and
*check* means a command. A row that moves and leaves its old text is the kind of
stale sentence this tree keeps finding, so the row says which check holds it.

## What would reverse this

**An `XL` that genuinely decomposes into fewer than five.** Then five is wrong,
and the honest repair is the number and a sentence saying what the measurement
was — not an exemption list, which is how a check becomes advisory.

**A `needs:` convention that stops meaning containment.** The check reads
*fewer than five ids extending its own id*, which assumes a child's id extends
its parent's. `E3-B07h` is a child filed late and it follows the convention; a
decomposition that named its children some other way would make this check blind
while looking green, and that is the failure mode to watch. The narrower
observation: any `XL` whose children do not share its id prefix.
