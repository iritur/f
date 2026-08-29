# Testing platform: what exists

`docs/design/proving-ground.html` specifies seven layers. This is where each one
actually stands, so the gap between the plan and the tree is visible rather than
assumed closed.

| Layer | Status | Where |
|---|---|---|
| **L0** Determinism substrate | **Built** | `env/src/lib.rs`, `xtask lint-determinism`, boot self-check in `kernel/src/main.rs` |
| **L1** Deterministic simulation | **Hook only** | `env/src/sim.rs` — seeded fault injection with protocol-aware site labels. No device models, no seed sweeps. Full simulator at phase 01. |
| **L2** Concurrency and memory model | **Stress tests only** | `ring/tests/litmus.rs` plus an AArch64 CI job. **Not** a model check — RustMC at M5. |
| **L3** Proof | **Absent** | Verus on the frame at phase 02; Kani on capability properties at M4. The frame must stop moving first. |
| **L4** Fuzzing | **Instrumentation only** | `xtask coverage`. No SQE generator, no snapshot harness, no hostile-peer fuzzer. Phase 01. |
| **L5** Performance regression | **Harness only** | `bench/` records distributions with p50/p99/p99.9. No change-point detection — that needs commit history to reason about, phase 02. |
| **L6** Hardware in the loop | **Absent** | Photodiode rig at phase 03, when there is a compositor to measure. Correctly deferred. |
| **L7** Claims registry | **Built, one entry** | `claims/`, `xtask claims`, `xtask claim <name>` |

## What was deliberately built early

Three things, each because retrofitting them is far more expensive than adding
them now:

**The determinism substrate (L0).** The one property that cannot be retrofitted.
Everything else on this page depends on it.

**The fault-injection hook (L1).** Not the simulator — the *hook*. Once code
asks its `Env` whether an operation should fail, adding a fault class is a
change to one file. If code instead assumes success, every call site has to be
revisited later.

**Coverage instrumentation (L4).** Fuzzing without coverage feedback is close to
worthless, and instrumenting a mature kernel is painful. It costs almost nothing
while the kernel is two thousand lines.

## The honest gaps

- **The litmus tests are empirical, not exhaustive.** They will not reliably
  catch a rare interleaving. RustMC explores what the memory model *permits*;
  stress tests explore what one machine happened to do. Do not mistake a green
  litmus job for a proof of the ordering.
- **`instructions_per_op` and `joules_per_op` report `Unavailable`.** The
  counters are not wired until M2. The harness carries the fields and marks them
  absent rather than omitting them, so a claim cannot quietly narrow to
  wall-clock only.
- **Claim 0001 is `pending` and measures the host.** No user interrupts, no
  registered buffers, no deadline class. It exists so the workload is
  version-controlled alongside the claim rather than written the day someone
  wants a number.
- **Nothing here has been executed.** See `BOOTSTRAP.md`.
