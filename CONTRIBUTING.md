# Contributing

## Before anything else

```
cargo xtask verify
```

Lint, then tests, then a kernel boot, failing cheapest first. It is the one
command that says whether the tree is green, and running it before asking for
review is the difference between a reviewer reading a change and a reviewer
being the test suite. The two halves are still there separately:

```
cargo xtask lint
cargo xtask test
```

Both must pass. `lint` runs three checks that encode architectural decisions
rather than style preferences, and each failure message names the RFC it comes
from.

## The three policies that are not negotiable in review

**Determinism.** Nothing observes time, randomness or ordering except through
`f_env::Env`. There is exactly one `rdtsc` in the tree. Adding a call site
means adding an allow-list entry with a reason, which is a reviewable diff.
See RFC 0004.

**The frame.** `unsafe` is permitted in `abi/`, `ring/` and `kernel/`, and
nowhere else. Every `unsafe` block carries a `// SAFETY:` comment discharging
the obligations its `# Safety` section states. Widening the frame requires an
RFC. See RFC 0001.

**The licence boundary.** The permissive tree never imports `third_party/`.
Imported code is reachable only over a ring. See `LICENSING.md` and RFC 0003.

## Where a change starts

Not in the editor. `intent/` holds one directory per change — what somebody
wanted, what we agreed it means, and how it gets built — and `docs/sdlc.md` is
the whole route from there to a tag. A one-line fix does not need the ceremony;
anything that would make somebody ask "why is this like this" in a year does.

If you are working with an agent, `CLAUDE.md` is what it reads first and
`.claude/` is the rest: the standing policies as skills, the guardrails as
hooks, and `evals/` as the check that any of it still works. All of it is
reviewed like code, because changing it changes every session afterwards and
nothing about that is visible at the moment you change it.

## When a change needs an RFC

If it changes something already written in `docs/design/`, or if a future
contributor would otherwise re-litigate it, it needs an entry in `docs/rfc/`.
Reversals especially: the design documents are rewritten as the design moves,
so the reasoning survives and the reversals do not unless they are recorded.

Copy `docs/rfc/0000-template.md`. The section that matters most is *What would
reverse this* — an RFC with nothing there is a preference wearing a decision's
clothes.

## When a change needs a claim

Any change that alters a number published in `docs/design/`. See
`claims/README.md`.

## Memory ordering

The ring's correctness rests on one `Release` store and one `Acquire` load.
`Relaxed` there passes every test on an x86 laptop and corrupts data on
AArch64. CI runs both targets, and a change to those orderings needs a litmus
test showing it fails under the weaker ordering.
