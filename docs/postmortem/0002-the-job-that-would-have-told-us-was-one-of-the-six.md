# Post-mortem 0002: the job that would have told us was one of the six

- Date of the incident: 2026-09-03 to 2026-09-09, six days.
- Date of this document: 2026-09-09
- Detected by: an agent reading `gh run list` while answering a different
  question — *are we ready for E3*. Recorded as an agent rather than as a
  person because it is the more useful fact: no check, no band and no
  notification reported any of it, the one mechanism built to report it is the
  mechanism that failed, and what found it was somebody asking an unrelated
  question of a tool that happens to list scheduled runs. That is luck, and it
  is not a detection strategy.
- Commits: caused by `aa4dd34`, which created the file with both faults in it,
  and carried forward by `b8f70fa`, `708ffa3`, `6c20bb7` and `743cb92`, each of
  which added another job with the first fault in it. Fixed by `8a09fc5`. The
  finding the repair uncovered is fixed separately.

## What happened

`.github/workflows/nightly.yml` was written on 2026-09-03 and has run five
times: 2026-09-05 through 2026-09-09, on the schedule, every one of them red.
It has never been green. Six of its nine container jobs failed on the first
line of their first step, every night, and the other three passed — so a run
was a third green, which reads as *a job is flaky* rather than as *this file has
never worked*.

The six are the six that open a step with `set -o pipefail`. That option is bash
and is not POSIX. A `run:` step in a `container:` job is handed `sh -e {0}` —
GitHub prints that line into the log of every such step, and it is in this
repository's own logs from the first run — and this image's `/bin/sh` is
`/usr/bin/dash`, which answers the option with `set: Illegal option -o pipefail`
and exit 2, before the command underneath it runs at all.

What each job then reported was not that. The step that refused wrote no report,
so the artifact step below it failed with `No files were found with the provided
path: prove-report.txt`, and that is the line at the bottom of the log and the
line in the job summary. Five of the six named a missing artifact. The visible
failure was one layer below the real one and pointed away from it.

The sixth is `sweep`, and `sweep` failed differently and worse. It is the only
job in the file that names its own `permissions:` block, because it is the only
one that opens an issue. A job that names `permissions:` *replaces* the default
set rather than adding to it, so naming `contents: read` and `issues: write`
dropped `packages: read` — which is what pulls the image the job declares two
lines above. It failed at `Initialize containers` with `Error response from
daemon: denied`, before any step of its own ran at all.

## What made it possible

Two faults with nothing in common, in one file, on one day.

The first is a shell assumption. Whoever wrote the steps wrote bash, and was
right about everything except which interpreter would read it. `ci.yml` runs the
same kind of step on a host runner, where the default resolves to bash, so the
habit was formed somewhere it works and carried to somewhere it does not. The
image has bash — `/usr/bin/bash`, and the Dockerfile sets it as the login shell
— so nothing was missing. It simply was not named.

The second is a GitHub semantic that reads backwards. `permissions:` looks
additive and is not. The job that needed one extra permission was the job that
asked for one, and asking is what took the others away.

## What made it invisible

This is the expensive half, and it is not the shell.

`sweep` is the job that opens the issue. The file's own header says why, at
length and correctly: *a nightly job that nobody reads is a job that fails
silently for a month, and a red cross on a schedule is the easiest thing in a
repository to stop seeing*, so a finding produces an issue and not only a red
cross, because that is the part that reaches a person. The reasoning is sound
and the mechanism was built. It was then disabled by the second fault, and the
two faults were independent: the shell one would have been noticed within a day
if `sweep` had opened an issue about anything at all, and the permissions one
would have been harmless if the other six jobs had been green.

`if: failure()` cannot run in a container that never started. So the one step in
this repository whose entire purpose is to turn a scheduled red into a
notification was, for six days, the step furthest from being able to run.

Three further things were true and none of them helped:

- `A-06` is a standing item that says *nightly sweeps and weekly checking stay
  green or stay loud*, at a daily cadence. It is a line in `TODO.md` and not a
  thing that executes, and a cadence nobody keeps is indistinguishable from one
  that is kept and finds nothing.
- The tree already declares that it cannot see this. `PROVE_RUN_GAP` says in as
  many words that *whether GitHub runs the `prove` job at all* is outside
  anything this tree can execute, and `lint-remap` says the same of the weekly
  job. Both are honest, both are correct, and between them they describe the
  blind spot precisely without anybody standing in it.
- `E1-P03` is `[x]` on the strength of the sweep existing. It ran green in the
  pull request that added it, at the corpus, in `ci.yml` — a different job in a
  different file — and the scheduled one it is actually about had not run,
  because a schedule does not run on a branch.

## What was true that we believed was not

That the nightly was running. Everything downstream of that belief was drawn on
it. `claims/0030` is described on `E2-R01`'s line as gating *nightly*, and the
alternative of hanging it on the release gate was explicitly refused there, with
the argument that *a claim checked once per release is a claim that goes red at
the worst moment*. That argument is right. The job it chose instead had never
run. The same holds for `cut`, which is gate G2's headline property on a
schedule, and for `entries` and `hostile`, the two fuzzers whose whole value is
inputs the pull-request gate never draws.

One thing was true in the other direction, and the repair established it on the
first working night: the `prove` job runs on a GitHub runner. `lint-proofs`
carries a declared gap saying the `full` image had only ever been built on one
machine, which needed `--network=host`, and that whether it builds on a runner
was unknown. It builds, Kani runs, and the job is green. Somebody who wants to
can narrow that gap.

## What changed

- lint / hook / test / eval / band: `defaults: run: shell: bash` at the top of
  `nightly.yml`, named once for the whole file rather than on six steps, because
  the seventh occurrence is otherwise somebody writing the same line into a new
  job in six months. `packages: read` added to `sweep`, with the replacement
  semantics of `permissions:` written beside it. Both in `8a09fc5`.
- the RFC, if a decision changed: none. Nothing here reverses a decision. The
  file's header was right about what it needed to do and was prevented from
  doing it.
- the claim, if a number was wrong: none was published. `claims/0030` was
  described as gating on a schedule that was not running, which cost the claim
  six days of evidence rather than making it wrong.

The repair was verified by running it. `gh workflow run nightly` on the branch
is the only way to observe a scheduled workflow before it reaches the default
branch: eleven of twelve jobs green, `sweep` among them, having pulled its image
and swept.

The twelfth is the finding below.

## What the repair immediately found

`entries` went red on the first night it was able to run, at base
`0x510e527fade682d1`, and reproduced from its own report at the first attempt.
The finding is a defect in the fuzzer's own oracle and not in the ring.
`Reached::reach_is_inside` computed the address a buffer *should* resolve to as
`DEVICE_BASE + index * stride`, which assumes the set under test is the first
thing the harness's translation ever mapped. `Pinned::map` hands out a fresh
page per mapping — as any translation must, or two sets would alias — and
`World::Full` registers `SLOTS` = 4 of them, so the set being checked sat three
pages above `DEVICE_BASE`. The ring resolved `0x430a0`, which is right, and the
oracle wanted `0x400a0` and called it wrong.

Worth recording that the fuzzer was right to go red and the ring was never
wrong, because the two are easy to conflate later. Worth recording too what
found it: not a new test, but 4 194 304 cases at a base the pull-request gate
never draws, which is the argument the `entries` job's own comment makes for
existing at all. That argument was tested on the first night it could be, and
it held.

## What we chose not to change

**Nothing that notices a nightly is absent, or has been red for N consecutive
runs.** That is the real gap and this document does not close it. `A-06` remains
a line in `TODO.md` rather than something that executes, and the same six-day
silence is available tomorrow to any job that fails before its notifying step.
What is written here is the argument for building it. The shape is probably a
check that reads the last N scheduled conclusions and is run from somewhere
other than that schedule, because a watcher living inside the thing it watches
is this incident with one more layer.

**The `permissions:` semantics are not linted.** One job in one file names a
block today. A check over every workflow refusing a `permissions:` block on a
job with a `container:` unless it names `packages: read` would be cheap, and
would have caught this. It is not written, because a lint drawn from one
occurrence tends to encode that occurrence. The reversal condition is a second
job needing its own permissions, at which point write it.
