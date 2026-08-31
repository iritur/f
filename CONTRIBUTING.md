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

Both must pass. `lint` runs four checks that encode architectural decisions
rather than style preferences, and each failure message names where the
decision comes from.

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

The fourth check, `lint-percpu`, is not one of the three: it enforces a design
decision rather than a policy, which is why it is named here and not above.
Kernel state is per-CPU from the first allocation, behind `PerCpu<T>`, while
only one core is running — see `kernel/src/percpu.rs` and section 14 of
`docs/design/ring-scene-boot.html`. The reason it is mechanised at all is that
the cost of breaking it is not paid until the day a second core boots, which is
the worst day to start paying it.

## The twelve rules

These come from `docs/what-must-be-stated.html` section 15, which derived them
by looking at nine gaps in the design corpus and asking what discipline was
missing in each. That is why they are worth more than the nine fixes: the gaps
were not independent accidents, they were places where a discipline this
project already applies elsewhere was not applied here.

The last column is the part this repository cares about. **A rule listed as
mechanised that is not mechanised is worse than one honestly listed as review**,
because it is a check somebody believes is happening.

| | | Enforced by |
|---|---|---|
| **R01** Name the mechanism, not the intention | A drawback is answered when something makes it unavailable. "We will be careful about X" is a plan, and plans are what the systems being criticised also had. | review |
| **R02** A boundary the hardware speculates through is not a confidentiality boundary | The one place this architecture is currently weaker than the system it criticises, and it entered by nobody stating the rule. | review, until RFC 0005 and the topology check |
| **R03** Every quantity crossing the ABI states its unit, its epoch and its zero | `deadline: u64` shipped with none of the three, in the one crate whose entire purpose is to be correct against code written by somebody else. | **`cargo xtask lint-units`** |
| **R04** Fail closed | Unknown opcode, unknown flag, non-zero reserved field: refuse. Ignoring an unknown bit is how a protocol acquires two incompatible interpretations and no error. | review, plus the hostile-peer fuzzer at E1 |
| **R05** Nothing is delivered asynchronously | Every event is a ring entry drained at a polling point. This is what keeps the determinism contract whole, and it is why this system never needs the concept of async-signal-safety. | **`cargo xtask lint-callbacks`** |
| **R06** Nothing is inherited | Authority arrives by grant and never by descent. Inheritance is how a capability system quietly becomes an ambient one. | review, and E0-P08's negative suite for the part that runs |
| **R07** A refusal names its domain | The architecture asks callers to handle refusals as ordinary control flow. A caller that cannot tell *why* it was refused cannot do that. | the ABI: `abi::error`, RFC 0010 |
| **R08** Do not use the word *deadline* for a promise nothing can refuse | The word is the whole discipline. A hard class without admission control is a hint with a better name, and that is precisely how deadline scheduling became decorative elsewhere. | review; a hard-class path with no admission test is a bug |
| **R09** Every headline claim names the subsystem that owns it | Energy was in the first paragraph of the thesis and had no owning subsystem across five documents. That is how half a claim goes missing without anyone deciding to drop it. | **`cargo xtask lint-claim-owners`** |
| **R10** Peers negotiate; they never demand equality | Lockstep versioning contradicts the component model, and the component model is what three separate arguments rest on. | the ABI: `ChannelHeader::negotiate`, RFC 0011 |
| **R11** The apparatus ships with the thing it measures | Determinism and coverage instrumentation were built early for exactly this reason, and the reasoning was written down. The state tree is the same argument and was deferred anyway. | process: a milestone that produces a number also produces its instrument |
| **R12** A concession is written as a cost, never hidden in a metric | "Full system rollback: one reboot" is a concession dressed as a target. Reservations leaving capacity idle belongs beside the latency claim, not in a rebuttal after somebody runs a throughput benchmark. | review; the claims registry carries the cost beside the number |

Three are executable, and each has a fixture in `xtask` that breaks it — a lint
that has never failed is indistinguishable from a lint that cannot. The other
nine are review, and saying so is the point: **R01 applies to this table**. A
rule with "review" beside it is a rule somebody has to apply, which is a plan,
and this table is honest about which rows are plans.

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
