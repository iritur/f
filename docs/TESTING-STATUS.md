# Testing platform: what exists

`docs/design/proving-ground.html` specifies seven layers. This is where each one
actually stands, so the gap between the plan and the tree is visible rather than
assumed closed.

| Layer | Status | Where |
|---|---|---|
| **L0** Determinism substrate | **Built** | `env/src/lib.rs`, `env/src/contract.rs`, `xtask lint-determinism`, and a boot that runs the contract against the seeded and the hardware `Env` on the same run — `kernel/src/env.rs`, `kernel/src/main.rs` |
| **L1** Deterministic simulation | **Hook only** | `env/src/sim.rs` — seeded fault injection with protocol-aware site labels. No device models, no seed sweeps, and nothing has yet injected a fault at a named site: E0-P09. Full simulator at phase 01. |
| **L2** Concurrency and memory model | **Stress tests only** | `ring/tests/litmus.rs` plus an AArch64 CI job. **Not** a model check — RustMC is E0-P16, open. |
| **L3** Proof | **Absent** | Verus on the frame at phase 02; Kani on capability properties at M4. The frame must stop moving first. The nearest thing that exists is not proof and should not be mistaken for it: five capability properties checked at every boot against a real table and five broken on purpose, plus one build broken on purpose — evidence that the checks can fail, not that they are exhaustive. |
| **L4** Fuzzing | **Instrumentation only** | `xtask coverage`. No SQE generator, no snapshot harness, no hostile-peer fuzzer. Phase 01. |
| **L5** Performance regression | **Harness only** | `bench/` records distributions with p50/p99/p99.9 and marks the counters it cannot read as absent. No change-point detection — that needs commit history to reason about, phase 02. |
| **L6** Hardware in the loop | **Absent** | Photodiode rig at phase 03, when there is a compositor to measure. Correctly deferred. |
| **L7** Claims registry | **Built, three entries, none gating** | `claims/`, `xtask claims`, `xtask claim <name>` — 0001 and 0002 `pending`, 0003 `tracked` on purpose |

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
- `cargo xtask cap` — nine boots in which a process tries to hold authority it
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

- **The state tree publishes twelve nodes and nothing that varies with time.**
  Frame counts, cores, ring tallies, capability slots. Not the timer's counters,
  not a stamp, not a hash of anything live — the boot log is what
  `cargo xtask trace` hashes, and a tick count in it would make two runs of one
  commit disagree for a reason with nothing to do with the kernel. The exclusion
  is a decision with a reversal condition, not a gap: it lifts when the boot log
  stops being the reproduction artefact.
- **The user-interrupt doorbell is written and has never executed.** `Path` and
  `Bell` build it, negotiation gates it on `feature::USER_INTERRUPT_DOORBELL`,
  and `Bell::new` refuses to construct it on a machine that does not report the
  hardware — which is every machine this project can reach. QEMU's TCG backend
  implements no part of Intel's UINTR and no `-cpu` model advertises the bit, so
  what is tested is the *refusal* and the selection logic, and the instruction
  has never run. E1-B09 owns the hardware. Do not read the suppression test
  passing on three paths as three paths having run: two have.
- **The doorbell number is a boot count, not a measurement.** The boot line
  reports doorbells per thousand operations over the two operations the
  self-test performs. That is enough to show the count exists and is not always
  one; it is not *doorbells per operation under load*, which needs a workload
  and a machine, and it is deliberately not registered as a claim. E0-B15's exit
  says so.
- **The litmus tests are empirical, not exhaustive, and now there is a number
  for how much that costs.** They will not reliably catch a rare interleaving.
  RustMC (E0-P16) explores what the memory model *permits*; stress tests explore
  what one machine happened to do. Do not mistake a green litmus job for a proof
  of the ordering.

  This stopped being an argument and became a measurement. `mutate-relaxed-submission`
  and `mutate-relaxed-completion` weaken the two publishing stores from `Release`
  to `Relaxed`, and CI required the suite to fail with them on, on the AArch64
  runner — the machine where that weakening is a real defect. **The suite
  passed.** Both steps were removed as gates, because a gate asserting a
  probabilistic test catches a specific reordering goes red on a Tuesday for
  reasons nobody can reproduce.

  So the standing position is sharper than "the gap is real": the suite has been
  shown not to catch the exact defect it was written to guard against, on the
  exact hardware that defect is about. `mutate-no-doorbell-fence` is the one
  defect that *is* caught and does gate — store-load is a reordering the hardware
  performs rather than one it might, at eight rounds in a thousand.
- **`instructions_per_op` and `joules_per_op` still report `Unavailable`.** The
  harness carries the fields and marks them absent rather than omitting them, so
  a claim cannot quietly narrow to wall-clock only. The reasons now name owners
  rather than a milestone — the PMU is read where the machine is real, at
  E0-P05; the first defensible energy number is E5-P03's, by external meter —
  because the previous strings said "until M2" and outlived M2 by three
  milestones before this page caught it.
- **No claim gates.** 0001 and 0002 are `pending`: 0001 measures the host — no
  user interrupts, no registered buffers, no deadline class — and 0002 has a
  threshold and no number, because the only environment available emulates the
  timer against a host clock it does not control; `F_ENVIRONMENT=container` is
  how the harness already knows. 0003 is `tracked` and gates nothing by design.
  They exist so the workload and the threshold are version-controlled rather
  than written the day somebody wants a number. E0-P05 and E0-P06 are what move
  the pending two, and both now wait on E0-D10 — the machine.
- **`cargo xtask verify` is local, and local is not everything.** It runs the
  lints, the host tests, an AArch64 cross-*compile* of the four crates the arm
  job tests, a QEMU boot and the mutation harness. It cannot *run* the AArch64
  tests or the
  litmus job, which are exactly where L2 means anything — x86-64's total store
  order hides the entire class of bug the ring is exposed to. Those run in CI
  and nothing local substitutes for them.
- **Every layer above L2 is a plan.** L3 and L6 are absent, L4 is
  instrumentation with nothing feeding it, and L5 is a harness with no history
  behind it. This page is worth re-reading whenever one of those is described in
  the present tense somewhere else.
