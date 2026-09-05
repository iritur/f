# Testing platform: what exists

`docs/design/proving-ground.html` specifies seven layers. This is where each one
actually stands, so the gap between the plan and the tree is visible rather than
assumed closed.

| Layer | Status | Where |
|---|---|---|
| **L0** Determinism substrate | **Built** | `env/src/lib.rs`, `env/src/contract.rs`, `xtask lint-determinism`, and a boot that runs the contract against the seeded and the hardware `Env` on the same run — `kernel/src/env.rs`, `kernel/src/main.rs` |
| **L1** Deterministic simulation | **Built, above the frame** | `sim/` — virtual time, seeded ordering, device models for blk, net and gpu on the real ring types, component substitution, snapshot and restore, and nineteen scenarios that must each reproduce from their seed and move when the seed moves (`xtask sim`). Seven fault classes, each asserting a response rather than printing one (`sim/src/fault.rs`, RFC 0039). Seed sweeps with automatic minimisation to a pasteable reproduction (`xtask sweep`, RFC 0040). **The scope is the thing to read, not the status:** RFC 0032 decided the simulator runs the *components* and not the frame's instructions, so it will never catch a bug inside the frame's own algorithms — the boot half is `xtask trace --hash`, and `xtask sim --join` requires the two halves to be about one component set. |
| **L2** Concurrency and memory model | **Stress tests, and now bounded proof beside them** | `ring/tests/litmus.rs` plus an AArch64 CI job, unchanged. RustMC is still E0-P16 and still open, for the reason it always was. What is new is not a substitute for it: L3's proofs cover the ring's *validation* paths against arbitrary bytes, which is a different question from what the memory model permits. Two instruments, two questions. |
| **L3** Proof | **Built, and narrow on purpose** | `kernel/proofs` and `ring/proofs`, run by `cargo xtask prove` — 27 Kani harnesses in about 46 minutes, on a nightly schedule. The five capability properties are proved over the file the kernel ships (compiled a second time through `#[path]` against three stand-ins, RFC 0053), with handles unbounded across all 2³² and rights across the whole 256×256 lattice; table contents are bounded *by construction*, because a harness never writes a slot — it runs the real operations with symbolic operands, so no proof holds for a state the table cannot reach. The ring's peer-facing paths are proved against a region of 640 symbolic bytes handed to the real `adopt`, rather than a struct of fields a harness owns, which `ring/src/mapping.rs` names as the trap (RFC 0057). **Six deliberate defects each fail the harness stating the property they break** — a proof that passes on a build with a known defect proves nothing. Verus on the frame is still phase 02. |
| **L4** Fuzzing | **Built, with two committed corpora** | `xtask hostile` — a hostile peer generated from a seed, a billion operations in about 49 s with no panic, no memory unsafety and no hang, where a run is episodes derived by identity so a finding at operation 999 999 999 replays in a millisecond (RFC 0046). A hang is a *count*, never a wall-clock timeout. `xtask entries` — a structure-aware submission-entry generator with coverage feedback, 87.5 % structure-aware because an entry's first check is a zero word and random bytes fail it with probability 1 − 2⁻³² (RFC 0048). `ring/corpus.txt` and `sim/corpus.txt` are in the tree and in the release package. Miri covers the memory-unsafety property at a much smaller count, and both numbers are reported rather than one being quoted. |
| **L5** Performance regression | **Harness only** | `bench/` records distributions with p50/p99/p99.9 and marks the counters it cannot read as absent. No change-point detection — that needs commit history to reason about, phase 02. |
| **L6** Hardware in the loop | **Absent** | Photodiode rig at phase 03, when there is a compositor to measure. Correctly deferred. |
| **L7** Claims registry | **Built, fifteen entries, six gating** | `claims/`, `xtask claims`, `xtask claim <name>`. The split is the honest part and it follows one rule: **a count may gate on this machine and a time may not.** Six gating — blast radius, hostile-peer operations, entry-validation coverage, admission refusals, deadline overtake, unmap churn — are all counts, identical on any machine. Eight `pending` are all times or ratios of times, because `bench/src/lib.rs` refuses to record a measurement in a container and that refusal is the harness working. Where a task produced both, the claim was split rather than weakened: 0005 gates and 0006 waits; 0012 gates and 0013 waits; 0014 gates and 0015 waits. Every `pending` timing waits on the same thing — `E0-D10`'s named machine. |

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

## What E1 added, and what it did not

Three of the seven layers moved from *absent* or *hook only* to built, and one
sentence is worth keeping in front of the rest: **none of it makes the frame's
own instructions observable to anything but QEMU.** RFC 0032 states that as the
simulator's scope rather than as a limitation discovered later, and the boot
suite, the mutation harness and L3's proofs are what cover the frame instead.

Five gaps in this tree are *declared quantities* rather than sentences —
`JOIN_GAP`, `CHAOS_GAP`, `DEADLINE_GAP`, `OWED_REVERSALS` and
`RECEIVE_SLOTS_STACK_BOUND`. Each is a constant a lint compares against the
tree, so each goes red both when it grows and when the reason for it stops being
true. `cargo xtask lint-owed` is the sharpest of them: it lists reversal
conditions that have fallen due and are unpaid, and it fails the day one is paid
and nobody updates the list. That is the difference between a debt and a wish,
and it is the mechanism this page would otherwise have to describe in prose.

## The honest gaps

- **The IOMMU stage has never met real firmware, and under UEFI it cannot.**
  `E1-B01`'s confinement is the newest thing on this page and its coverage
  outside QEMU is zero — not low, zero. The second boot printed
  `acpi none: no checksummed root pointer in either window` and left every
  IOMMU state node at zero, because multiboot 1 has no field for the root
  system description pointer and UEFI does not leave one in the two legacy
  windows this kernel is able to scan. That is structural: it will happen on
  every UEFI machine, including the one `E0-P18` is waiting for. The kernel
  fails closed and says so, which is R04 working rather than a defect, so
  nothing goes red — which is exactly why it belongs on this page. `E5-D03`
  owns it; `docs/second-boot-outside-qemu.md` has the reasoning and the three
  candidate protocols.

- **The state tree publishes thirty-two nodes and nothing that varies with time.**
  Frame counts, cores, ring tallies, capability slots, and since E1 the
  datapath's own tallies — copies on the data path, kernel entries per bucket,
  blast radius, overtakes, invalidations. Not the timer's counters,
  not a stamp, not a hash of anything live — the boot log is what
  `cargo xtask trace` hashes, and a tick count in it would make two runs of one
  commit disagree for a reason with nothing to do with the kernel. The exclusion
  is a decision with a reversal condition, not a gap: it lifts when the boot log
  stops being the reproduction artefact.
- **This kernel has never run on bare metal.** It has run outside QEMU exactly
  twice, both on VMware machines: 2026-09-01, recorded in
  `docs/first-boot-outside-qemu.md`, and 2026-09-05 carrying all of E1, in
  `docs/second-boot-outside-qemu.md`. The first says in its own opening that a
  hypervisor is not the machine `E0-P18` is about, and the second says it again.
  Everything else this page
  reports is an assertion about an emulator: the APIC enumeration, the memory
  map, the UART, the application-processor startup, `M0 ok`, and now every
  datapath result too — the remapping unit, the three drivers, the framebuffer
  capture. This is still the largest single gap on this page and it is still
  easy to miss, because nothing here is *failing*: the tests pass, the boots are
  green, and the subject of nearly every one of them is an emulator.
  The second boot moved part of it and sharpened the rest. The component
  supervisor is no longer emulator-only evidence — four places, five spawns, a
  fault, a restart, a connect resuming across the gap, a retirement, and zero
  cross-core allocations on the hot path, all against real page tables at
  32 GiB. The datapath itself did not move an inch: `state 20..31` were zero,
  because the machine presented no virtio device to drive.
  E1 made this sharper rather than softer. Six claims now gate, and they gate
  because they are counts that do not depend on the machine — but every number
  about *time* in this project remains unmeasured, and `bench/src/lib.rs`
  refusing to record one here is the only reason that fact is visible. `E0-P18` owns closing it and
  `docs/booting-on-hardware.md` is the procedure. Two consequences worth
  reading before that boot rather than after: the trace hash **will** differ on
  hardware, because the memory map and core count are in the log by design and
  QEMU's are pinned; and `MAX_CPUS` is eight logical processors, so a larger
  machine has the rest left asleep and says so on its own `note` line. Eight is
  a measured choice rather than a leftover — `docs/booting-on-hardware.md` has
  the cost curve and why raising it buys admission capacity that nothing can
  use yet.
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
  When RustMC does land it will carry a caveat of its own, and it is better
  stated here in advance than discovered next to a result: it runs under its own
  toolchain (RFC 0022), so it checks the protocol **as written** and not the
  machine code this tree ships. Atomic orderings are specified by the language
  rather than chosen by the backend, which is why that is a small gap — but it
  is a gap, and a citation of a model-check result owes the reader this sentence.

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
  defect that *is* caught and does gate — at eight rounds in a thousand, on the
  **x86-64** runner.

  That last word is the second surprise. Store-load is the one reordering total
  store order performs and the one AArch64 forbids: `Release`/`Acquire` become
  `stlr`/`ldar` there, which are RCsc, so a Store-Release followed by a
  Load-Acquire is already ordered and removing the fence changes nothing
  observable. The one defect in this suite that does not need the arm runner is
  the one that needs the x86 runner instead. Which machine can see a defect is a
  property of the reordering it depends on, not of how serious it is.
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
