# RFC 0009: Three clocks, and only one of them orders anything

- Status: accepted
- Date: 2026-08-28
- Affects: `abi/`, `env/`, RFC 0004

## Decision

- **Monotonic time** is `f_env::Instant`: nanoseconds since this `Env`'s origin,
  `u64`, never decreasing, and *not* stopped by suspend. It is the only clock
  with ordering authority, the only clock a deadline may be expressed in, and
  the only one the scheduler sees.
- **Wall time** is a datum, not a clock. `f_env::WallTime` is TAI nanoseconds
  carrying its source and an uncertainty, reached through `Env::wall()`, which
  returns `None` on a machine that has no trustworthy one. It may be stamped on
  objects and shown to people. It may never order a system event, drive a timer,
  or express a deadline.
- **Civil time** — zones, calendars, leap seconds, formatting — does not exist
  below the semantic layer. It is a projection, in exactly the sense the
  interface thesis uses the word.

`Sqe.deadline` is monotonic nanoseconds in the channel's epoch, and
`NO_DEADLINE` is zero. `Cqe.timestamp` is the same clock.

## Context

Linux offers `REALTIME`, `MONOTONIC`, `BOOTTIME`, `MONOTONIC_RAW` and `TAI`, and
a large family of bugs comes from picking the wrong one: timeouts that fire
early when someone sets the clock, timers that stop across suspend, leap seconds
that have taken down large fleets more than once, and a 32-bit epoch still being
repaired two decades after it was diagnosed. The underlying error is uniform —
one word, "time", covering three unrelated jobs: ordering events, measuring
duration, and naming a moment to a human.

F inherited none of that by accident and had not written the rule down. `Instant`
already carried the right semantics; the ABI shipped a `deadline: u64` with no
stated unit and no stated epoch, which is precisely the ambiguity two
independently written peers resolve differently — and the first symptom would be
a scheduling decision rather than a parse error.

## Consequences

No clock jump can affect scheduling, because the scheduler cannot see a clock
that jumps. No leap second exists inside F. `u64` nanoseconds runs to roughly
584 years, so the epoch problem does not recur.

Wall time under simulation is seeded like anything else, so a run that stamps
timestamps on objects stays byte-reproducible. This is why it is an `Env` method
and not a service call: a service could be asked without the substrate knowing,
and then a seed would stop reproducing a run.

`Env::wall()` returns an `Option`, so every caller has to decide what to do on a
machine with no clock. That is deliberate. The alternative is a plausible
fabricated number, and a fabricated timestamp is worse than a missing one
precisely because it is usable.

The cost is interop: data arriving from outside carries wall-clock ordering
assumptions, and F must project rather than adopt them. That is real work, and
the edge is the correct place to pay it.

## What would reverse this

A service needing wall-clock ordering across machines. That is a distributed
systems problem whose answer is a hybrid logical clock at that layer, not
re-privileging wall time inside the system.
