# REVIEW.md

What a review of this repository checks, in what order, and what it is allowed
to conclude. Claude reads this file on every pull request; a human reviewer can
read it as the checklist it is.

The order matters. A correctness finding on code that violates the licence
boundary is wasted work, so the policy pass runs first and can stop the rest.

## Pass 1 — policy

The three decisions that are not negotiable in review. `cargo xtask lint`
already fails the build on each of them, so a finding here means either the
lint has a hole or the diff widened an allow-list.

- Determinism: new nondeterminism outside `f_env::Env`, or a new
  `DETERMINISM_ALLOW` entry whose stated reason does not survive reading. RFC 0004.
- The frame: `unsafe` outside `abi/`, `ring/`, `kernel/`; a missing or
  hand-waving `// SAFETY:` comment; a crate quietly dropping the workspace
  lints. RFC 0001.
- The licence boundary: any path from the permissive tree into `third_party/`
  that is not a ring. `LICENSING.md`, RFC 0003.

An allow-list entry added in the same diff as the code that needs it is the
single most common way these policies erode. Say so when you see it.

## Pass 2 — bugs and logic

- Memory ordering: any change to a `Release`/`Acquire` pair without a litmus
  test that fails under the weaker ordering. Assume the reviewer's x86 laptop
  proves nothing.
- Arithmetic that can overflow, indexing that can panic, and `unwrap` on
  anything reachable from the kernel.
- Error paths: RFC 0010 structured errors, not stringly-typed ones.
- Tests that assert less after the diff than before: a deleted assertion, a new
  `#[ignore]`, a loosened bound, a shrunk iteration count. Say what coverage was
  lost, not that coverage was lost.

## Pass 3 — security

- `unsafe` blocks that trust a value crossing a trust boundary. Everything in
  `abi/` crosses one.
- Credentials, tokens or private hosts in the diff, including in tests and
  fixtures.
- New dependencies: `deny.toml` gates the licence and the advisory database,
  but neither gates *why*. A dependency added for one function is a finding.
- Anything in `third_party/` reachable other than over a ring.

## Pass 4 — compliance with spec and plan

- The diff matches the `plan.md` it claims to implement. Extra files are a
  finding; so is a plan step with no diff behind it.
- Numbers in prose have claims. A benchmark result in a PR description with no
  `claims/` entry is an anecdote (`claims/README.md`).
- Decisions that reverse something already written down have an RFC, and the
  RFC has a *What would reverse this* section with something in it.
- `TODO.md` tasks touched by the diff are marked, and every task still carries
  an `exit:`.

## Severity

- **blocking** — a policy violation, a correctness bug, a credential, or a
  weakened test. The pull request does not merge.
- **major** — the change is right but the evidence is missing: no claim, no RFC,
  no litmus test.
- **minor** — real, not urgent. Fix it or open an `intent/` for it.
- **nit** — at most five per review, and none at all on a diff that has a
  blocking finding. Beyond five, say "formatting nits omitted" and stop.

Excluded from review: `target/`, `third_party/` (imported verbatim — review the
import commit, not the code), `Cargo.lock`, `*.profraw`.

## What review may and may not do

Claude may push fixes to a branch it did not author the head commit of, when
asked by `@claude` on the pull request. It may not approve a pull request, and
it may not merge one. Branch protection requires a human approval that is not
the agent's, and that rule is enforced in the repository settings rather than
here, because a rule that lives only in a file the agent reads is a rule the
agent can be talked out of.

Findings that recur go into `CLAUDE.md` under *Common mistakes*, and findings
that recur after that become an eval in `evals/`. A third occurrence means the
instruction is not working and the loop is the thing to fix.
