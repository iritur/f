# How work moves through this repository

Six stages, from somebody wanting something to the system running it and telling
us what happened. Each one names its artifact, who closes it, and what an agent
is allowed to do without asking.

The reason to write this down is the same reason `CONTRIBUTING.md` exists: most
of it is already how the project works, and the parts that are not become
visible the moment somebody tries to follow the document.

```
  1 plan      intent/NNNN-name/intent.md      a person wants something
  2 design    intent/NNNN-name/spec.md        we agree what it means
  3 build     plan.md, then a diff            it gets made
  4 test      cargo xtask verify              it checks its own work
  5 deploy    REVIEW.md, then a tag           it leaves
  6 maintain  ops/bands.yaml, evals/          it tells us what happened
```

## 1 — Plan

**Artifact:** `intent/NNNN-short-name/intent.md`. **Closed by:** the originator.

Problem, proposed outcome, affected users and systems, constraints, open
questions. Written in the originator's own words; `intent/README.md` has the
rules and `intent/0000-template/` has the shape.

Git history carries the author, the timestamp and the approval, so none of those
are fields in the file. What we watch: how long it takes to get from a
conversation to a committed intent, and how many accepted intents survive
contact with a spec.

## 2 — Design

**Artifact:** `spec.md`, beside the intent. **Closed by:** the originator, by
reading it.

The `spec-from-intent` skill drives this, and it applies the standing policies
while the design is still free rather than discovering them in review:
determinism, the frame, the licence boundary, evidence, and whether an RFC is
owed. Which skills applied is recorded in the spec's frontmatter, because a spec
written before a skill existed was written under different rules and it should
be possible to tell.

The spec goes back to the originator before a plan starts. This is the review
that is still cheap.

## 3 — Build

**Artifacts:** `plan.md`, then a diff. **Closed by:** the engineer.

Plan first, from an agreed spec, naming every file it will touch and the order
of work. Then the diff, which is expected to match — `REVIEW.md` pass 4 treats
an unlisted file as a finding, and a plan step with no diff behind it as the
same.

What the agent reads while doing it:

- `CLAUDE.md` — commands, conventions, architecture, and the mistakes that have
  happened twice. One page. It grows when a mistake recurs and shrinks when
  something in it becomes a hook instead.
- `.claude/skills/` — the standing policies, each written for whoever is about
  to break one.
- `.claude/agents/` — scoped helpers: an independent `policy-auditor`, an
  `rfc-scribe`, a `claim-registrar`.
- `.claude/hooks/` — the part that does not depend on being read. Protected
  paths, credentials, weakened tests, determinism at the keystroke, the release
  boundary.

Independent tasks run in separate worktrees. `git worktree add ../F-<task>` is
the whole ceremony; the build volumes are per-checkout so two sessions do not
fight over `target/`.

## 4 — Test

**Artifact:** a green `cargo xtask verify`. **Closed by:** the session itself.

One command: lint, then test, then boot, failing cheapest first. A session
verifies its own work before a human is asked to look, because otherwise the
human is the test suite.

Bug fixes go the other way round: the failing test is written first, from the
report, before the cause is understood — so that it tests the bug rather than
the fix. `.claude/hooks/tests-hold.sh` closes the escape routes that make this
advice optional: `#[ignore]`, an assertion deleted down to `assert!(true)`, a
litmus repeat count quietly lowered.

There is no visual check here, because there is nothing to look at. The
equivalent is the boot log: `cargo xtask run` asserts on QEMU's exit code, and
the log itself is byte-identical for a given `(seed, commit)`, so a diff of it is
the screenshot diff this project gets.

Two gaps stated rather than papered over. `verify` is local, so it cannot run
the AArch64 tests or the litmus job, and those are where the ring's ordering
means anything. And the eval suite measures whether a policy is *known*, not
whether it is *followed* in hour three of a long session — which is why the
hooks exist.

The suite itself is `evals/`: twenty-two tasks, run by `cargo xtask eval`, gated
in `.github/workflows/agent-evals.yml` on any diff to `CLAUDE.md`, `.claude/` or
`REVIEW.md`. Those are changes whose effect is otherwise invisible at the moment
they are made.

## 5 — Deploy

**Artifacts:** a review against `REVIEW.md`, then a tag. **Closed by:** a human
who did not write the change.

Review runs four passes in a fixed order — policy, bugs, security, compliance —
with a severity ladder and a nit cap. `.github/workflows/claude-review.yml` runs
it on every pull request and answers `@claude` on comments. It may push fixes.
It may not approve, and it may not merge; branch protection enforces that in the
repository settings, because a rule that lives only in a file the agent reads is
a rule the agent can be talked out of.

There is no production service here, so the tiers are:

| tier | what it is | gate |
| --- | --- | --- |
| development | `cargo xtask run`, QEMU on a laptop | none |
| staging | CI: both architectures, litmus, coverage, boot | green, or it does not merge |
| release | a tag, a claims re-run, a published number | `F_RELEASE_AUTHORIZATION`, set by a person |

`.claude/hooks/release-gate.sh` is the gate. It blocks `cargo publish`, tag
pushes and image pushes unless that variable names an authorisation, and it
blocks force-pushes and hard resets under any authorisation at all. An agent
cannot set the variable for itself: a gate the gated party can open is a log
entry.

Rollback is `git revert` plus a re-run of `cargo xtask claim` for anything the
change touched, and the claims registry is what makes that meaningful — a
revert that restores the code but not the number has not finished.

## 6 — Maintain

**Artifacts:** `intent.md` drafts, and evals. **Closed by:** whoever triages.

`ops/detect.sh` reads `ops/bands.yaml` and reports how far each signal's newest
sample sits from its own history. One standard deviation logs; two permits a
read-only diagnosis; three permits a pull request or a pre-approved runbook.
Nothing escalates further, and the release gate does not read that file.

Detection is arithmetic on purpose. An agent asked to watch a dashboard will
find something to say every time it looks.

`.github/workflows/security-scan.yml` runs weekly: `cargo deny` gates, and a
scan of the tree does not — it produces findings with a stated confidence for a
person to triage. Anything larger than a patch is written as an `intent.md` and
re-enters at stage 1.

Then the step that is easy to skip: **an incident becomes an eval, in the same
change that fixes it.** An incident that produced no eval will happen again in a
form nobody recognises. Post-mortems go in `docs/postmortem/`, which is a
lessons directory rather than a blame directory.

## Where artifacts live, and what links to what

This repository is authoritative. There is no ticket system to reconcile with,
and if one is ever introduced the linkage rule is the minimum: every artifact
notes the record id, every record contains the commit SHA.

Inside the tree, four identifier spaces already exist and now point at each
other:

- `intent/NNNN` — what somebody wanted.
- `TODO.md` task ids (`E2-B04`) — permanent, ranked by what they unblock. An
  intent that becomes work names its task ids; the task names the intent.
- `docs/rfc/NNNN` — decisions and reversals.
- `claims/NNNN` — numbers, with baselines and thresholds.

An intent that produces a decision gets an RFC. One that produces a number gets
a claim. One that produces neither produced a patch, which is fine.

## Adoption order

Six of these plays have no prerequisites and were adopted first: the intent
folder, `CLAUDE.md`, the skills, the hooks, the one-command feedback loop, and
the recurring scan. Everything else depends on one of them:

```
CLAUDE.md ──┬─> skills ──> spec-from-intent (stage 2)
            ├─> hooks ────> release gate (stage 5)
            └─> verify ───> eval suite ──> the eval gate on .claude/ diffs
intent/ ────┴─> review pass 4 (diff matches plan)
ops/bands.yaml ──> maintain workflow ──> incident becomes eval
```

The one ordering that matters: the eval suite has to exist before the
configuration it defends is worth changing quickly, and `verify` has to exist
before the eval suite means anything, because a suite that cannot say whether
the tree is green is measuring the tree and not the configuration.

## What is deliberately not adopted

Recorded because "we did not think of it" and "we decided against it" look
identical a year later, which is the same reason `docs/rfc/` exists.

- **Deploy, status and rollback exposed as tools.** There is no service to
  deploy. When there is, the tier table above is the shape it takes: development
  free, staging on green, release behind the authorisation — and the tools get
  scoped per environment rather than one tool that takes an environment
  argument, because the second kind gets passed the wrong argument eventually.
- **A chat integration.** This project has no shared incident channel, so an
  agent in one would have no audit trail to join and nothing to be paged about.
  The parts that would matter — a conversation, an authorisation and a fix all
  landing in the same record — are currently the pull request.
- **A separate intent repository.** One product, one repository, so `intent/`
  lives beside the code it argues about. The alternative is for organisations
  with many repositories and one product owner, which is not this.
- **An external tracker.** Nothing to reconcile with. If one arrives, the rule
  is the linkage minimum above and not a synchronisation job: every artifact
  notes the record id, every record contains the commit SHA.

## For a regulated deployment

`docs/managed-settings.example.json` is what this repository's settings look
like when they are administered centrally rather than committed: the allow-list
becomes the safe inner loop only, bypass mode is disabled, the sandbox has a
domain allowlist, and credential paths are blocked outright. It is an example
rather than live configuration, because managed settings are deployed by an
administrator to a machine, not merged into a repository.
