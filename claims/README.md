# Claims registry

Every number published in `docs/design/` corresponds to an entry here. The
documents should render their numbers from this directory rather than restating
them, so that a claim which stops holding cannot quietly persist in prose.

## Why this exists

`docs/design/proving-ground.html` layer 7. A claim without a named baseline, a
published workload and a one-command reproduction is an anecdote. This is that
requirement made mechanical.

## Rules

1. **The baseline is versioned with the claim.** This is what stops a tuned
   Linux comparison from decaying into a stock Linux comparison as the baseline
   configuration ages and nobody re-checks it.
2. **`status = "gating"` fails the build on regression.** `status = "tracked"`
   records without gating. Muting a claim is a status change in a reviewable
   diff, never a comment in CI config.
3. **Distributions, not summaries.** Store the full histogram. A mean computed
   at collection time destroys what cannot be recovered, and systematically
   under-reports the stalls this architecture exists to eliminate.
4. **Report instructions and joules per operation, not only nanoseconds.**
   Those survive a hardware change and are far less noisy.
5. **Regression detection is change-point, not threshold.** Thresholds either
   miss real regressions or fire until everyone mutes them.

## The machine

`runner-class-A.md`, beside this file. Every claim's `[hardware]` block names a
runner class, and `MEASUREMENT_ENVIRONMENTS` in `bench/src/lib.rs` names the one
class allowed to record — both of them cited this README for what the class
*was*, and this README did not say. It says it there now: the capabilities, the
firmware, the kernel command line, and how each of RFC 0007's four reservation
components is obtained, with the honest note that a file is not a machine and
that the environment variable is an assertion rather than a measurement.

## The first entry

Timer jitter p99, at milestone M2 — the first real measurement the project
produces. Registering it there rather than in a commit message is what makes
the apparatus grow with the system instead of being retrofitted onto it.
