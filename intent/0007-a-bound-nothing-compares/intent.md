---
id: 0007
status: draft        # draft | accepted | withdrawn | shipped
originator: Dmitri Chudinov
todo:                # TODO.md task IDs, once there are any
---

# Some of our published bounds are numbers nobody checks

*Filed from the adversarial audit of the E2 branch, 2026-09-07. Nothing in it
blocked the merge; this is the largest of what it found that was too big to fix
inside that pull request.*

## Problem

A claim in this registry is meant to be two things at once: a number somebody
measured, and a bound that number is not allowed to leave. The second half is
what makes the registry worth having — anybody can record a measurement, and
what stops a design from quietly getting worse is that the next run is compared
against the bound rather than merely printed beside it.

For four of our claims the second half does not happen. The command that
reproduces the claim starts the workload, the workload prints its numbers, and
the table of bounds in the claim file is read by nobody at all. A run can print
a number outside its own published bound and every check we have stays green.
Three of those four are marked as gating the build, which is the strongest word
the registry has.

This is not hypothetical and it is not new. It has already happened twice on
this branch, and both times the same way: a workload measured a number well
outside the bound its claim published, the registry said nothing, and the whole
local loop was green while a person read the published bound and believed it.
One of those became a design reversal, so the cost of the silence was not the
wrong number — it was the gap between the run that produced it and anybody
noticing, which was closed by an audit rather than by a command.

There is a second, quieter half of the same problem. Where the bound *is*
enforced today, it is enforced by a constant written into the workload's source
by hand — the same number, typed twice, in two files, with nothing holding them
together. That is the arrangement the comparison exists to end, and it fails in
the direction that is hardest to see: somebody tightening a claim's table
without touching the workload gets a green run, and somebody loosening the
workload's constant without touching the claim gets one too.

What makes this urgent rather than tidy is that the registry is what we point at
when we say this project does not overclaim. A bound nothing compares against is
prose with a decimal point in it, and we would rather not have published it.

## Proposed outcome

- Every bound in every claim is compared against the run that claims to measure
  it, and a number that leaves its bound turns that claim's command red naming
  the row that moved — not the first failure, all of them, so one run tells the
  whole story.
- A bound that no workload produces a number for is a failure and not a silence.
  Publishing a bound nothing emits is the state this whole change exists to end,
  and it must not be reachable by forgetting.
- No number is written down twice. A bound lives in the registry; a workload
  prints what it measured; nothing carries a copy of the other's number.
- The claims that gate the build actually gate it, in the sense that a
  contributor who breaks one finds out from a command rather than from a reader.

Observable when it is done: deliberately moving any published bound by one, in
the file, makes the corresponding command go red and name that row — and the
same is true of deliberately breaking the measurement instead.

## Affected users and systems

- `claims/`, and in particular the four entries whose bounds are currently
  compared with nothing: write amplification, the chunk-size distribution, the
  blob verification refusals and the index blocks per query.
- `xtask/`, which owns the claim commands and already does the comparison for
  about two thirds of the registry — so this is mostly the extension of a
  mechanism that exists rather than the invention of one.
- The four workloads behind those claims, which print their numbers today and
  in two cases also assert them from a hand-copied constant.
- Nobody's `docs/design/` page has to change. This publishes no new number and
  moves none.

## Constraints

- **No bound moves to make a comparison pass.** If wiring a claim up turns it
  red, the red is the finding and the number stays where it is until somebody
  argues it somewhere a reader can disagree with — which is an RFC, not a diff.
- Cost matters and is uneven. One of these workloads is a million-blob run and
  another is minutes of emulation, so *compared on every run* and *compared
  somewhere* are different asks, and the cheap ones should not be held up by the
  expensive ones.
- Some bounds in the registry are deliberately unmeasurable today: they name a
  boundary that does not exist yet, and their claims say so at length and are
  marked as not gating. Those are debts and must not be deleted to make a
  comparison total.
- The claims registry's own rules stand: a number that reaches a design page has
  a baseline, a workload and a one-command reproduction.

## Open questions

- What should a bound do when its claim is not gating and its boundary does not
  exist yet? Comparing it against a number taken somewhere else is exactly the
  substitution the registry exists to prevent, and deleting it loses the debt.
  There is probably a third state and we have not named it.
- Is *every row compared* the right rule, or is it *every row either compared or
  explicitly marked as owed*? The first is simpler to check and the second is
  what two of these files already do in prose.
- Where do the expensive comparisons run — the local loop, the gate, or a
  schedule — and who notices when a scheduled one goes red? We have just made
  the same choice once for the rollback claim and the reasoning should probably
  be written down once rather than argued per claim.
