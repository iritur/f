# RFC 0042: A sweep states what it costs and what it cannot catch

- Status: accepted
- Date: 2026-09-03
- Affects: `sim/` (`sweep.rs` — the leak budget, the seed-index span, the vacuous
  report; `check.rs` — the falsifiability count and the run-anchored plumbing
  test; `dev.rs` — a second deliberate defect; `main.rs` — `--ceiling`, `--from`,
  a fail-closed argument parser and a self-judging reproduction line), `xtask`
  (`sweep_verb`, `sweep_ceiling`, a sharding `sweep`, `sweep_record`,
  `sweep_mutate`, `DEFECTS`), `.github/workflows/nightly.yml`. Amends RFC 0040
  and reverses none of it.

## Decision

**A sweep is bounded in memory by a stated number, refuses a grid it cannot
hold, and refuses a grid with nothing in it.** Three things follow, and each is
a place where the previous shape reported a green result for a run that had not
happened.

- **The leak is a budget rather than a hope.** `f-sim` runs a million trials in
  one process and every trial leaks its clients' buffer regions —
  `sim/src/client.rs` grants a component's region for the life of the component,
  and a simulated component's life is the run. That was bounded when a run was a
  process. It is bounded by nothing once a sweep is in-process, so
  `sweep::LEAK_BUDGET` is what bounds it: one gibibyte, computed against the
  shipped scenario table rather than measured on one machine, and a grid past it
  is *refused before a trial runs* with the largest acceptable `--seeds` printed
  beside the refusal. `cargo xtask sweep` shards on that ceiling, and a shard is
  a range of the same seed derivation — so the shards together try exactly what
  one process would have tried, in the same order, with each finding's seed
  index still an index in the whole sweep.
- **Zero is refused everywhere it means *nothing ran*.** `--seeds`,
  `--scenarios`, `--clients`, `--window`, `--ops` and `--jobs` all take at least
  one, and a report whose grid held no trials is an error rather than a clean
  sweep. `sweep::steps` already made this argument one level down — *a run with
  nothing in it produces a short trace and a perfectly stable digest, which is
  the one result a check must never report as a pass* — and it was not applied
  at the grid or at the command line.
- **A reproduction command judges itself.** The line a report prints is spelled
  `--check`, which exits non-zero and names the property that broke, rather than
  `--trace`, which prints seventy lines and exits zero. `cargo xtask sweep
  --mutate` takes the line off the report and runs it, requiring non-zero armed
  and zero disarmed, so what is asserted is the command and not the shape of a
  string.

**And the oracle says how much of itself is under test.** Three of its five
properties are now falsifiable end to end by a defect in the shipped source:
`mutate-crossed-completion` trips `held` and `mutate-silent-reset` trips
`balance`, with `bound` reachable by the second in runs where nothing was in
flight. `intact` **cannot fire on any run of the models as they stand**, because
no device model writes into a client's data buffer, and `clock` is a structural
invariant of the timeline with no defect arranged for it. Both are stated in
`check.rs` and counted here rather than left for the next reader to discover.

## Context

RFC 0040 built the sweep and argued the in-process form at length: *a process per
trial would cost tens of milliseconds of cargo and dynamic linking against a run
that takes under a millisecond, which is the difference between a nightly sweep
of a million trials and a nightly sweep of ten thousand.* That argument is
correct and is not reversed here. What it did not carry across was the other half
of `sweep::plan`'s own reasoning, which bounds a much smaller leak by observing
that *the process running it exits*: the same sentence applied to `client.rs` one
level up, and stopped being true.

Measured rather than reasoned about, on the four-core development container: at
100 000 trials of one scenario, `handshake` (buffer_bytes 0) peaks at 14 780 kB
of resident set and `blk` (buffer_bytes 512, eight buffers, two clients) peaks at
827 876 kB — about 8.3 KiB per trial, exactly two clients' worth, never released.
`cargo xtask sweep 65536`, the nightly default, rises monotonically to 6.06 GiB
before exiting. It is clean there because that machine has 15.5 GiB.
`nightly.yml` runs on a GitHub-hosted runner, which is the 7 GB class, and there
was no evidence the default had ever been run on one.

That matters more than a number: an out-of-memory sweep produces a red cross and
a truncated `sweep-report.txt`, and the job's `open an issue` step fires on
`failure()`. What would have reached a person every night is an issue containing
no finding — a nightly reporting *nothing* in the shape of a nightly reporting a
bug, which is the failure the workflow's own header says it exists to prevent.

Three alternatives to the ceiling were live.

- **Fix the leak.** The right answer, and not available in safe Rust.
  `BufferSet::bind` takes the region for `'m` and `carve` reborrows the set for
  the same `'m`, so a set and the buffers carved from it cannot both be fields of
  a value that moves; the region and the set are therefore at `'static`.
  Recycling a `&'static mut` needs either `unsafe` — which `sim/` inherits
  `forbid` on, and which CLAUDE.md permits only in `abi/`, `ring/` and `kernel/`
  — or a lifetime parameter threaded through `Actor`, `World` and every actor in
  the crate, with an arena to hand out disjoint borrows, and every safe arena is
  a dependency. Deferred, with the reversal condition stated below.
- **A process per trial.** Rejected again for RFC 0040's reason: it converts a
  five-minute nightly into a multi-hour one.
- **Guess a smaller default.** Rejected because it leaves the fail-open in
  place. A sweep small enough today is a sweep that is killed the first time
  somebody adds a scenario with wider buffers, and it would be killed silently.

The second defect has a different history. Review observed that four of the
oracle's five properties had only ever been shown to fail on hand-built `Record`
vectors — `client.rs` was never executed — so a change that stopped the client
writing what a check reads would leave the property dead while every test stayed
green and every sweep printed `clean`. That is the shape of false pass this epoch
has had twice already: an assertion that holds because the path it observes is
not being walked. Two answers, and both are taken: a test that runs a shipped
scenario and requires the records each check reads to be present, and a second
deliberate defect that makes a second property fail on a run of the models.

## Consequences

**Easy.** A nightly that finishes on the machine it runs on, and says what it
costs: `cargo xtask sweep 65536` is 1 179 648 trials in seven processes, 226 s at
four workers, and a peak resident set of 941 268 kB — 0.90 GiB — in the largest
of them, polled every two seconds through `/proc`. The same sweep unsharded ran
285 s and reached 6.06 GiB, so the ceiling costs nothing and buys the runner. A grid that is too large is a refusal naming the
largest one that is not, so the failure mode is a message rather than a kill.
Every count flag is fail-closed, so a corpus entry cannot be vacuously green. A
reproduction line a stranger pastes exits non-zero and names the property, and
the harness proves that by running it rather than by matching a prefix.

**Hard.** A sharded sweep prints one report per shard, so *smallest first* is
smallest within a shard rather than across the night. That is a real loss and it
is bounded: the shards are in seed order, so the first report is from the
earliest seeds, and `nightly.yml` titles its issue from the first `finding 1` in
the file. Merging findings across shards would mean parsing the report or
teaching `xtask` the `Size` ordering, and both put a second copy of the
minimiser's judgement outside the crate that owns it. Not worth it while a
million-trial night is seven shards; worth revisiting at seventy.

**Foreclosed.** Nothing. The ceiling is a constant and the sharding is a loop; a
`client.rs` that stopped leaking would make both dead code, which is the outcome
this is arranged to allow.

**Stated rather than left to be found.** `intact` is a check that no run of the
current models can fail. It is kept because deleting it means the first model
that writes into a client's buffer ships with nothing watching the ownership rule
RFC 0024 is for, and because `every_check_has_a_run_that_fails_it` shows the
predicate works. It is a guard, not a property under test, and this is where that
is written down.

## What would reverse this

- **`client.rs` stops leaking.** If `Actor` and `World` grow a lifetime, or
  `ring/src/buffers.rs` grows a constructor that lets a set hand its region back,
  then `LEAK_BUDGET`, `max_seeds`, `--from`, `--ceiling` and the sharding loop
  all become unnecessary in one change. Delete them; do not keep a ceiling that
  no longer bounds anything, because a bound nobody can reach is a bound nobody
  maintains.
- **The measured leak stops matching the model.** `Trial::leak_bytes` is
  arithmetic over the scenario table plus a per-client overhead of 512 bytes. If
  a sweep is ever killed for memory *inside* the ceiling, the model is wrong and
  the number to change is the overhead — with the measurement that showed it, in
  a claim.
- **A sweep with one shard stops being the ordinary case.** The smallest-first
  ordering is the exit criterion's *no human triage* made mechanical, and it
  holds per shard. If the default sweep ever shards, merge the reports before
  this loss becomes the normal reading experience.
- **`intact` becomes reachable.** The first device model that writes into a
  client's data buffer makes it a property under test rather than a guard, and
  this section is where the count changes.
- **A defect stops being reached.** `mutate-crossed-completion` needs two
  consecutive coalescing decisions and `mutate-silent-reset` needs a device to
  fall over. Either becoming unreachable makes `sweep --mutate` fail loudly with
  that sentence in its message, which is the intended failure — but somebody then
  owes a new defect rather than a smaller assertion.
