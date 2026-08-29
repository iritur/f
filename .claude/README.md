# .claude/

The agent-facing half of the development loop. Everything here is version
controlled and reviewed like code, because it decides what an agent is allowed
to do in this repository and what it is told before it does it.

`settings.json` is checked in and applies to everyone working in this tree.
Personal preferences go in `.claude/settings.local.json`, which is not tracked.

```
settings.json      permissions and hook wiring for the whole team
skills/            the standing policies, written for whoever is about to break one
agents/            scoped helpers for jobs that recur
hooks/             the guardrails that do not depend on being read
```

## The division of labour

**Skills are instructions.** They apply when a model reads them and decides to
follow them, which is most of the time and not all of it. Use them for the
things that need judgment: what a `// SAFETY:` comment owes the reader, when a
claim is an anecdote, whether a decision needs an RFC.

**Hooks are mechanism.** They apply whether or not anything was read. Use them
for the small set of things that must not depend on a model's cooperation — the
protected paths, credentials in a diff, a weakened test, the release boundary.
Each one is a few milliseconds and blocks with a message that says what to do
instead, because a refusal with no route out of it gets worked around.

The overlap is deliberate. `determinism-guard.sh` and the `determinism-review`
skill are the same policy said twice: the skill explains it, the hook catches it,
`cargo xtask lint-determinism` is authoritative. If the hook and the lint ever
disagree, the lint is right — it walks the whole tree and owns the allow-list.

**Agents are scope.** A subagent gets its own context and a narrower tool set,
which is what makes an independent audit independent. `policy-auditor` is
read-only on purpose.

## Running the hooks by hand

They read one JSON object on stdin and say yes or no by exit code — 0 allows,
2 blocks and hands stderr to the agent. So they are testable:

```bash
echo '{"tool_name":"Edit","tool_input":{"file_path":"third_party/x.c"}}' \
  | bash .claude/hooks/protected-paths.sh; echo "exit $?"
```

`bash` has to be on PATH. On Windows that is Git Bash, which is present anyway
because the container and the Bash tool both need it.

## Changing any of this

A change to `settings.json`, to a skill, or to a hook is a change to how every
future session behaves, and it is not observable in the way a code change is. So
it is gated the same way: `.github/workflows/agent-evals.yml` re-runs the eval
suite on any diff under this directory or to `CLAUDE.md`, and a drop in the pass
rate fails the pull request. `evals/README.md` says what the suite contains and
how to add to it.
