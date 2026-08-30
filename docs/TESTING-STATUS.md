# Testing platform: what exists

`docs/design/proving-ground.html` specifies seven layers. This is where each one
actually stands, so the gap between the plan and the tree is visible rather than
assumed closed.

| Layer | Status | Where |
|---|---|---|
| **L0** Determinism substrate | **Built** | `env/src/lib.rs`, `env/src/contract.rs`, `xtask lint-determinism`, and a boot that runs the contract against the seeded and the hardware `Env` on the same run — `kernel/src/env.rs`, `kernel/src/main.rs` |
| **L1** Deterministic simulation | **Hook only** | `env/src/sim.rs` — seeded fault injection with protocol-aware site labels. No device models, no seed sweeps, and nothing has yet injected a fault at a named site: E0-P09. Full simulator at phase 01. |
| **L2** Concurrency and memory model | **Stress tests only** | `ring/tests/litmus.rs` plus an AArch64 CI job. **Not** a model check — RustMC at M5. |
| **L3** Proof | **Absent** | Verus on the frame at phase 02; Kani on capability properties at M4. The frame must stop moving first. The nearest thing that exists is not proof and should not be mistaken for it: five capability properties checked at every boot against a real table and five broken on purpose, plus one build broken on purpose — evidence that the checks can fail, not that they are exhaustive. |
| **L4** Fuzzing | **Instrumentation only** | `xtask coverage`. No SQE generator, no snapshot harness, no hostile-peer fuzzer. Phase 01. |
| **L5** Performance regression | **Harness only** | `bench/` records distributions with p50/p99/p99.9 and marks the counters it cannot read as absent. No change-point detection — that needs commit history to reason about, phase 02. |
| **L6** Hardware in the loop | **Absent** | Photodiode rig at phase 03, when there is a compositor to measure. Correctly deferred. |
| **L7** Claims registry | **Built, two entries, both `pending`** | `claims/`, `xtask claims`, `xtask claim <name>` |

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

## What the seven layers have no row for

Provoking the running system into failing on purpose. It is neither simulation
nor proof — it is the real kernel, in QEMU, being asked to do something that
must not work:

- `cargo xtask fault pf|ud|df|nx|wx|stack` — six kernel faults, each of which
  must be *reported* rather than survived. The exception path is the one piece
  of the kernel that only runs when something has already gone wrong, so it is
  either exercised deliberately or discovered to be broken at the worst moment.
- `cargo xtask user` — seven boots in which a process at ring 3 violates one
  isolation rule each. Six must fault and the kernel must survive every one; the
  seventh must not fault, which is what stops the other six passing for the
  wrong reason. In the CI gate.
- `cargo xtask cap` — eight boots in which a process tries to hold authority it
  was not granted. Seven are refused by the capability table with the exact code
  each escape earns; the eighth is not refused at all — the process revokes a
  capability it is entitled to revoke, reads the page that revoke unmapped, and
  is stopped by the processor. This is E0-P08 as runs. In the CI gate.
- `cargo xtask mutate` — a kernel built with one deliberate defect, booted into
  the forging sweep, required to go red with a panic in the log; then the same
  boot without the defect, required to go green. It is the other half of E0-P08:
  four of the five properties have a fixture that breaks them and runs at every
  boot, and the fifth cannot, because a fixture that panics takes the machine
  down rather than being caught. RFC 0017. In the CI gate.

This page used to say these were "the shape E0-P08 will take at M4" and were not
E0-P08, because there was no capability table. There is one now, and the two
commands above are that suite. What was true and remains true is the framing:
none of this is simulation, proof or fuzzing. It is the real kernel, in QEMU,
being asked to do something that must not work.

The absence of a row is worth stating rather than papering over. A taxonomy
built around simulation, proof and fuzzing has no natural home for "run the real
thing and try to break it", and that is where most of this project's evidence
currently comes from.

## The honest gaps

- **The litmus tests are empirical, not exhaustive.** They will not reliably
  catch a rare interleaving. RustMC explores what the memory model *permits*;
  stress tests explore what one machine happened to do. Do not mistake a green
  litmus job for a proof of the ordering.
- **`instructions_per_op` and `joules_per_op` still report `Unavailable`, and
  the reason they give has expired.** They say the counters are not wired until
  M2. M2 arrived at E0-B07 and they are still not wired; E0-P04 owns it. The
  harness carries the fields and marks them absent rather than omitting them, so
  a claim cannot quietly narrow to wall-clock only — but the string in
  `bench/src/lib.rs` names a milestone that has been and gone, which is the
  smaller version of the failure this page exists to prevent.
- **Both claims are `pending`, and neither gates anything.** 0001 measures the
  host: no user interrupts, no registered buffers, no deadline class. 0002 has a
  threshold and no number, because the only environment available emulates the
  timer against a host clock it does not control — `F_ENVIRONMENT=container` is
  how the harness already knows. They exist so the workload and the threshold
  are version-controlled rather than written the day somebody wants a number.
  E0-P05 and E0-P06 are what move them.
- **`cargo xtask verify` is local, and local is not everything.** It runs the
  lints, the host tests, a QEMU boot and the mutation harness. It cannot run the
  AArch64 tests or the
  litmus job, which are exactly where L2 means anything — x86-64's total store
  order hides the entire class of bug the ring is exposed to. Those run in CI
  and nothing local substitutes for them.
- **Every layer above L2 is a plan.** L3 and L6 are absent, L4 is
  instrumentation with nothing feeding it, and L5 is a harness with no history
  behind it. This page is worth re-reading whenever one of those is described in
  the present tense somewhere else.
