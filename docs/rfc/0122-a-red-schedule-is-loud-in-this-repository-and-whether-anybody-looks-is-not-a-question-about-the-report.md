# RFC 0122: A red schedule is loud in this repository, and whether anybody looks is not a question about the report

- Status: accepted
- Date: 2026-09-24
- Affects: `.github/workflows/nightly.yml`, `weekly.yml`, `maintain.yml`,
  `security-scan.yml`, `agent-evals.yml`; `ops/alarm.sh`, `ops/alarm-post.sh`;
  `xtask/src/main.rs` (`lint-schedules`); `TODO.md` A-06; `docs/postmortem/0003`

## Decision

Every workflow in this repository with a `schedule:` trigger carries one `alarm`
job that names every other job in the file in `needs:`, runs `if: always()`, and
opens a GitHub issue when any of them failed — commenting on the open thread
rather than opening a second one when the same set fails again, and closing the
thread on a green run. `cargo xtask lint-schedules` runs in `verify` and in the
pull-request gate and refuses a scheduled workflow whose alarm does not watch
every job in its file, does not run both halves of the mechanism, or does not
post on `schedule`.

This **reverses a per-job decision made three times**. `nightly.yml`'s `prove`
job says *"It opens no issue"*; the `cut` job's header argues that a
deterministic failure on a fixed input has nothing a notification would carry
that the report does not; and `rollback`'s header repeats that argument by
reference. Each of those is a correct statement about the *content* of a
notification and each was taken as settling the question of whether to send one.
It does not settle it. Whether a report is self-explanatory and whether anybody
reads it are different questions, and this repository answered the first one
three times while the second went unanswered twice.

Those three arguments are not deleted and the jobs are not changed: the `sweep`
job's issue still carries the smallest reproduction it found, the `runners` and
`dates` jobs' issues still name the leaf that moved, and `prove`, `cut`,
`rollback`, `rechunk` and `gating-claims` still open nothing of their own. What
is added is a floor under all of them.

## Context

`docs/postmortem/0003`: on 2026-09-23 the nightly's `rollback` job went red at
03:11 UTC and nobody read it for a day. The repair had been committed on
2026-09-22 — the day *before* the run — on a branch, so the schedule reported a
defect nobody could connect to its fix. `docs/postmortem/0002` is the first
occurrence, and its shape is worse: five consecutive red nights, unread, in which
`sweep` — the one job in the file built to reach a person — was itself among the
six broken by a `set -o pipefail` under dash, so the mechanism that would have
told somebody was the mechanism that was broken.

`TODO.md`'s **A-06** says *nightly sweeps and weekly checking stay green or stay
loud. A muted job is a deleted job with extra steps.* On 2026-09-24 that item
recorded its own failure in as many words: the cheap half of 0003's lesson was
built — the four boot-path bounds became `cargo xtask lint-bounds`, four file
reads inside `verify`, RFC 0116 — and *the mechanism is earned and is not built*.
**A-09** had already added the line to `CLAUDE.md`'s *Common mistakes*, which is
that section's rule working: a mistake made twice earns a line. **A-11** asks
that every incident produce a change to something that *runs*. A line in
`CLAUDE.md` is not something that runs.

What was live as an alternative, and is what made this wait two post-mortems:
per-job notification, decided per job on the content of that job's report. Eight
of the nightly's fourteen jobs had that conversation and seven of them decided
no. The seven were right about their reports.

## Consequences

**What it makes easy.** A red schedule now arrives where this project already
looks. The body carries the job, the commit the run tested, the first failing
line in this tree's own failure vocabulary, the last forty lines of the failing
log, and — the part 0003 is actually about — a
`git log --oneline <tested sha>..<default branch>` when the branch has moved past
the commit under test, with the sentence saying why to read it. That is the one
piece of information that would have closed 0003 in a minute instead of a day.

**What it makes hard, deliberately.** Muting. Turning this off is a diff to a
workflow *and* a red `lint-schedules`, and adding an unwatched job to the nightly
is a red pull request rather than a quiet gap. A-06's sentence is that a muted
job is a deleted job with extra steps; the point of the lint is that muting one
now costs the same as deleting it.

**What it costs.** One more job per scheduled workflow, on the bare runner, which
is seconds. It is not in a container: the image is a thing that can fail, and a
watcher that needs the image cannot report the image failing — which is
`docs/postmortem/0002` in one sentence, since `sweep`'s issue-opening step was
unreachable on five red nights because naming `permissions:` there dropped
`packages: read` and the container could not be pulled. A step can only report
the job it is in. This is a job outside the ones it watches.

**Two residues, named here rather than discovered.** The alarm cannot report
itself — no job in a workflow can watch itself — and it reads `ops/alarm.sh` out
of the commit under test, so a commit that breaks that script silences its own
alarm. Both are covered as well as they can be from inside the repository: the
presence and the shape of the alarm are checked per commit by `lint-schedules`,
and the decision and the message are tested per commit by `xtask`'s
`schedule_alarm` module against fixtures. What is left uncovered is the three
`gh` calls, and only a real failure can exercise those. That is stated rather
than implied, because implying it is how 0002 happened.

**What is not claimed.** That anybody reads a GitHub notification either. This
moves a red run from a tab somebody has to open to a notification everybody is
subscribed to, and that is a strictly better place and not a guarantee. The
observation that would settle it is below.

## What would reverse this

Three observations, in the order they are likely.

1. **A third unread red schedule, this time with an open issue in the
   repository.** That would say the delivery route was never the constraint and
   the mechanism is theatre, and the honest next move is not a fourth
   notification channel but moving the check into `verify` — which is what 0003's
   cheap half already did for the bounds, and what `CLAUDE.md` now states as the
   rule: *if a check rests on arithmetic between two constants, it is a file read
   and belongs in `verify`.*

2. **Issues nobody closes, or a reader who has learned to skip them.** Both are
   measurable by looking: more than one open `<workflow> is red:` thread at a
   time means the signature is too fine, and a thread with thirty comments and no
   reply means the mechanism has been muted by arrangement rather than by a diff
   — A-06's exact failure, arrived at from the other side. The answer then is
   fewer alarms and not more: raise the floor so that only a job whose failure is
   a defect in this tree can open one.

3. **An intermittent failure.** Everything here assumes a red run stays red until
   somebody fixes it, which is true of both post-mortems and is what makes
   closing on green correct. The first genuinely flaky scheduled job makes the
   close wrong — it would close a real thread on the next lucky night — and
   `ops/alarm-post.sh` carries that reversal condition at the point it would have
   to change.
