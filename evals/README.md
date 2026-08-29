# evals/

Twenty-two tasks that check whether the agent configuration in this repository
still works. `cargo xtask evals` lists them; `cargo xtask eval` runs them.

## What is being measured

Not the model. The model is not ours to change.

What is ours is `CLAUDE.md`, `.claude/skills/`, `.claude/hooks/`,
`.claude/agents/` and `REVIEW.md` — and every one of those is a change whose
effect is invisible at the moment it is made. Editing a skill feels like editing
a document and behaves like editing a compiler flag. This suite is the only
thing that turns such an edit into an observation, which is why
`.github/workflows/agent-evals.yml` triggers on exactly those paths and why the
pass rate gates the pull request.

## Where the tasks come from

The suite was seeded from the standing policies — one task per rule that this
repository already refuses to negotiate, drawn from `CONTRIBUTING.md`, the RFCs
and `claims/README.md`. That is the floor, not the target: a seeded task proves
the policy is legible, and only a task that came from a real failure proves the
policy is load-bearing.

Everything added after the seeding comes from a mistake made more than once. The
escalation is:

1. A finding, fixed in the pull request.
2. The same finding again — a line in `CLAUDE.md` under *Common mistakes*.
3. A third time — a task here. The instruction is not working, and the loop is
   what needs fixing.

Production incidents skip the queue: an incident becomes a task in the same
change that fixes it, and `.claude/agents/incident-intent.md` names that as part
of the handling. An incident that produced no eval will happen again in a form
nobody recognises.

## Grading

Every prompt ends by demanding a verdict token, and grading is a substring test
for it, with an optional `forbid` token for the wrong answer.

This is crude on purpose. A grader that judges free text is another model, with
its own failure modes, sitting between a change and the evidence about it — and
when the suite disagrees with a change you want the argument to be about the
change. A task that cannot be reduced to a verdict token is a task that has not
been made specific enough yet.

The cost is real and worth stating: these tasks measure whether the policy is
*known*, not whether it is *followed* under pressure in a long session. That is
a gap, the hooks exist because of it, and nothing here closes it.

## Adding a task

Copy the shape of any file in `tasks/`:

```toml
status  = "active"                  # active | quarantined
defends = "one line: what breaks if this stops passing"
origin  = "where this came from — a review, an incident, an RFC"
expect  = "VERDICT: X"              # the token a correct answer contains
forbid  = "VERDICT: Y"              # optional: the token a wrong answer contains

prompt = """
The situation, in the second person, with no hint of which answer is wanted.
Then: answer with exactly one line and nothing else, and the two options.
"""
```

Rules that keep the suite honest:

- **The prompt must not signal the answer.** Order the options so the correct
  one is not always second, and describe the tempting path as genuinely
  tempting — "the build would go green", "it would take one line". A task whose
  wrong answer is obviously wrong measures nothing.
- **One policy per task.** A failure has to point at the instruction that
  failed.
- `status = "quarantined"` takes a task out of the gate without deleting it, and
  is a reviewable diff. Quarantining is for a task discovered to be ambiguous —
  never for one that keeps failing, which is the finding.

## Running it

```
cargo xtask eval                    # the suite
cargo xtask eval 11-ordering        # one task, by any substring of its name
```

Needs `claude` on PATH and a credential in the environment. The CI job skips
itself when the secret is absent rather than reporting green, because a suite
that silently passes when it did not run is worse than no suite.
