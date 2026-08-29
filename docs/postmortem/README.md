# Post-mortems

What went wrong, what we learned, and what changed so that the same thing fails
loudly next time. One file per incident, `NNNN-short-name.md`, from
`0000-template.md`.

Not a blame directory. The question this repository asks after an incident is
not who did it but *what made it possible and what made it invisible* — those
are two separate failures and the second one is usually the more expensive.

## What counts as an incident

At M0 there is no production service, so the incidents available are the ones a
research vehicle actually has:

- a claim that was published and turned out to be wrong;
- a policy that was violated and the lint did not catch it;
- a determinism failure — the same seed and commit producing two different runs;
- a boot that broke and stayed broken for more than a day;
- an agent that did something nobody sanctioned, whatever the outcome.

The last one belongs here even when nothing broke. A near miss that produced a
good result is the cheapest evidence available about where the guardrails end.

## The rule that makes this directory worth having

**Every post-mortem ends in a change to something that runs.** A lint, a hook, a
test, an eval, a band in `ops/bands.yaml`. A post-mortem whose only output is
"be more careful" documents the incident and prevents nothing, and the next
occurrence will not be recognised as the same thing.

For anything an agent was involved in, the change is usually an eval — see
`evals/README.md`, which treats an incident as skipping the queue straight to a
task. For anything a policy should have caught, the change is to the policy's
check rather than to its wording.

## Linkage

A post-mortem names the commit that caused it, the commit that fixed it, and the
`evals/`, `claims/` or `docs/rfc/` entries that came out of it. If it produced
none of those, say so and say why — that sentence is the useful part.
