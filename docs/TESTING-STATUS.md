# Testing platform: what exists

`docs/design/proving-ground.html` specifies seven layers. This is where each one
actually stands, so the gap between the plan and the tree is visible rather than
assumed closed.

| Layer | Status | Where |
|---|---|---|
| **L0** Determinism substrate | **Built** | `env/src/lib.rs`, `env/src/contract.rs`, `xtask lint-determinism`, and a boot that runs the contract against the seeded and the hardware `Env` on the same run — `kernel/src/env.rs`, `kernel/src/main.rs` |
| **L1** Deterministic simulation | **Built, above the frame** | `sim/` — virtual time, seeded ordering, device models for blk, net and gpu on the real ring types, component substitution, snapshot and restore, and nineteen scenarios that must each reproduce from their seed and move when the seed moves (`xtask sim`). Seven fault classes, each asserting a response rather than printing one (`sim/src/fault.rs`, RFC 0039). Seed sweeps with automatic minimisation to a pasteable reproduction (`xtask sweep`, RFC 0040). **The scope is the thing to read, not the status:** RFC 0032 decided the simulator runs the *components* and not the frame's instructions, so it will never catch a bug inside the frame's own algorithms — the boot half is `xtask trace --hash`, and `xtask sim --join` requires the two halves to be about one component set. |
| **L2** Concurrency and memory model | **Stress tests, and now bounded proof beside them** | `ring/tests/litmus.rs` plus an AArch64 CI job, unchanged. RustMC is still E0-P16 and still open, for the reason it always was. What is new is not a substitute for it: L3's proofs cover the ring's *validation* paths against arbitrary bytes, which is a different question from what the memory model permits. Two instruments, two questions. |
| **L3** Proof | **Built, and narrow on purpose** | `kernel/proofs`, `ring/proofs` and `abi/proofs`, run by `cargo xtask prove` — 31 Kani harnesses, on a nightly schedule; the first 27 in about 46 minutes, the four newest not yet timed on the schedule. The five capability properties are proved over the file the kernel ships (compiled a second time through `#[path]` against three stand-ins, RFC 0053), with handles unbounded across all 2³² and rights across the whole 256×256 lattice; table contents are bounded *by construction*, because a harness never writes a slot — it runs the real operations with symbolic operands, so no proof holds for a state the table cannot reach. The ring's peer-facing paths are proved against a region of 640 symbolic bytes handed to the real `adopt`, rather than a struct of fields a harness owns, which `ring/src/mapping.rs` names as the trap (RFC 0057). The admission arithmetic RFC 0050 put in `abi/src/reserve.rs` is proved over every demand and every machine shape up to eight physical cores — four sentences, the two about the walk again at sixty-four, one of which `mutate-overlapping-grant` has to break — because that RFC made it one implementation on purpose and one implementation has nothing independent to disagree with it. **Seven deliberate defects each fail the harness stating the property they break** — a proof that passes on a build with a known defect proves nothing. **Verus is on the frame as of 2026-09-21, as a second step of the same nightly job**: `cargo xtask verus` proves the three functions an untyped account's watermark is moved by — `carvable`, `carved` and `refunded` in `kernel/src/watermark.rs` — over every `u64`, with `mutate-saturating-refund` required to refute the conservation one. The annotations are in the shipped file rather than in a model beside it, which is RFC 0096 and is a different mechanism from the `#[path]` above: Kani verifies ordinary Rust, Verus verifies only what is inside `verus!{ … }`. What it reaches that the bounded checker cannot is the reason it is here: `kernel/proofs/src/mem.rs` sets `FRAME_SIZE` to 256 and the harnesses run an eight-slot table, so a `u64` overflow is outside Kani's space and always will be — and both preconditions `watermark.rs` now states were overflows the shipped code had never written down. It is three functions of arithmetic and not the frame: a deductive proof over anything in `cap.rs` that dereferences a raw pointer needs a specification for the memory it reaches, and that is a larger project than three postconditions. One image, x86-64 only, and no second job — `image_full` carries both checkers and `proof_schedule` refuses a second dependent. |
| **L4** Fuzzing | **Built, with two committed corpora** | `xtask hostile` — a hostile peer generated from a seed, a billion operations in about 49 s with no panic, no memory unsafety and no hang, where a run is episodes derived by identity so a finding at operation 999 999 999 replays in a millisecond (RFC 0046). A hang is a *count*, never a wall-clock timeout. `xtask entries` — a structure-aware submission-entry generator with coverage feedback, 87.5 % structure-aware because an entry's first check is a zero word and random bytes fail it with probability 1 − 2⁻³² (RFC 0048). `ring/corpus.txt` and `sim/corpus.txt` are in the tree and in the release package. Miri covers the memory-unsafety property at a much smaller count, and both numbers are reported rather than one being quoted. |
| **L5** Performance regression | **Harness, and a detector over the one series this project can record** | `bench/` records distributions with p50/p99/p99.9 and marks the counters it cannot read as absent. Change-point detection exists as of `E2-P09` — `cargo xtask history --changes` reads the series `history_append` has been writing since `E0-P11` and says where it stepped rather than where it crossed a bound, which is `claims/README.md` rule 5. **What it can read is one series and the reason is not the detector.** `coverage_percent` is a line count and is the same on any machine; every distribution in the registry is a timing or a ratio of timings, and `bench/src/lib.rs` refuses to record those outside a measurement environment — so on every machine this project can reach, the history holds coverage and stated gaps and no distributions. It reports and does not gate. The second half of `E2-P09`'s exit — that ordinary run-to-run noise over the same period is *not* detected — is about this system's noise and needs a gating claim that has run repeatedly on `runner-class-A`; it is a row in `docs/TECHNICAL-DEBT.md` rather than a number invented here. |
| **L6** Hardware in the loop | **Absent** | Photodiode rig at phase 03, when there is a compositor to measure. Correctly deferred. |
| **L7** Claims registry | **Built, thirty-six entries, twenty-one gating** | `claims/`, `xtask claims`, `xtask claim <name>`. The split is the honest part and it follows one rule: **a count may gate on this machine and a time may not.** Twenty-one gate and every one of them is a count — blast radius, hostile-peer operations, entry-validation coverage, admission refusals, deadline overtake, unmap churn, chunk-size distribution, blob-verification refusals, index blocks per query, cut outcomes, collector invariants, topology renderings per root, generation roots across paths, operations across a place swap, rollback comparisons, whole-system divergences localised, frame identities across boots, copies per operation, kernel entries per operation, bytes re-chunked per application byte written, ring crossings per UI frame — identical on any machine. Fourteen are `pending`; `0003` is `tracked` on purpose. Where a task produced both, the claim was split rather than weakened: 0005 gates and 0006 waits; 0012 gates and 0013 waits; 0014 gates and 0015 waits. **`pending` has two reasons and this row used to give one.** Most of the fifteen are times or ratios of times, `bench/src/lib.rs` refuses to record a measurement in a container, and those wait on `E0-D10`’s named machine. `claims/0022 copies-per-read` does not: it is a count of bytes, byte-identical on any host, and its own note says the machine it needs *is not a runner class* — it is a boot of `user/objects` as a component over the blk driver’s ring, which is a task in this tree. Every pending claim’s `[hardware] runner` nonetheless names `runner-class-A`, so the registry cannot presently tell a number waiting on silicon from one waiting on a task — `claims/0036 copies-per-operation` is the first to say so in its own `[hardware]` note, and it gates. |

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
- `cargo xtask cap` — **eleven** boots in which a process tries to hold authority
  it was not granted. Nine are refused by the capability table with the exact
  code each escape earns. Two are not refused by the table at all and both are
  load-bearing: `grant` uses its capabilities correctly and is refused nothing,
  which is what stops the other ten passing for the wrong reason, and `unmap`
  revokes a capability it is entitled to revoke, reads the page that revoke
  unmapped, and is stopped by the processor. This is E0-P08 as runs. In the CI
  gate. The count is the `ESCAPES` table in `xtask/src/main.rs`, which is where
  to read today's: this line said nine for as long as the table said eleven, and
  nothing in the tree compares the two.
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

Twelve gaps in this tree are *declared quantities* rather than sentences —
`OWED_REVERSALS`, `CHAOS_GAP`, `DATAPATH_GAP`, `CHURN_GAP`, `REVOKE_GAP`,
`SWAP_GAP`, `JOIN_GAP`, `DEADLINE_GAP`, `DEADLINE_DEPTH_GAP`,
`REPRODUCE_RUN_GAP`, `PROVE_RUN_GAP` and `ARCH_RUN_GAP`, all in
`xtask/src/main.rs`. Each is a constant a lint or a test compares against the
tree, so each goes red both when it grows and when the reason for it stops being
true — eight of them through one function, `gap_holds`, and the other four
through readers of their own. **This paragraph said four for two epochs**, which
is the failure mode the mechanism was built against arriving in the page that
describes it.

Two of the twelve are now empty and neither emptiness is a weaker check.
`RECEIVE_SLOTS_STACK_BOUND` was paid on 2026-09-11 — the frame gave a driver four
pages of stack and the constant, its `OWED_REVERSALS` row and the wall it
described all went together. `JOIN_GAP` is `&[]` since RFC 0044: the boot now
spawns a place per component file, and `hold_the_gap` requires *equality*, so a
component the simulator runs and this boot does not spawn is red with nothing to
compare against. What an empty list can no longer exercise is the other
direction — a stale entry going stale — and that half is held by a test against a
list it supplies itself, which says so rather than implying the constant still
covers it.

`cargo xtask lint-owed` is the sharpest of them: it lists reversal conditions
that have fallen due and are unpaid, and it fails the day one is paid and nobody
updates the list. That is the difference between a debt and a wish, and it is the
mechanism this page would otherwise have to describe in prose.

## What E3 has added since this page was last read

Read on 2026-09-24, against three waves that landed in two days. None of it moves
a layer's status; what it changes is what the existing layers are *about*, which
is why it belongs here rather than in a row.

- **A frame trace with no clock in it** — `abi/src/trace.rs`, `E3-B05b`, RFC 0111.
  `abi/src/sync.rs` names three outcomes of a wait and says of the third that *a
  hang writes no log line*. This is that nothing replaced by a record: every wait
  a submission carries, with its value and the timeline whose producer owed it,
  and a `dropped_waits` count so a trace that could not hold them all says so
  instead of looking complete. It does not prevent the hang. It is what makes one
  readable afterwards, and it is the only thing `E3-B05`'s negative — *no implicit
  wait appears in a frame trace* — can rest on. There is no clock in it, which is
  RFC 0004 rather than an omission: a trace carrying a timestamp is one two runs
  of a commit disagree about.

- **A solve stage and an emit stage, and one scene fed both ways** —
  `interface/src/solve.rs` (`E3-B06e`, RFC 0113) and `semantic/src/emit.rs`
  (`E3-B06f`, RFC 0117). The solve boundary is *declared*, and a tree that
  declares none is refused rather than solved with a default; the count that
  checks it comes from a walk the solver never runs, which is what stops the
  solver grading its own homework. `E3-B06f`'s exit is the sharper one: one scene
  fed through both routes produces the same bytes. That is a determinism property
  of the interface layer and the simulator has no row for it — RFC 0032 put the
  frame's own instructions outside the simulator's scope, and this is the
  mirror-image gap one layer up.

- **A crossing counted on both sides** — `claims/0038`, `E3-B01j`, gating.
  Sixteen ring entries over two UI frames, eight out and eight back, counted by
  the frame and by the component and required to agree before the rows print.
  Two things about it are worth reading before it is cited: a crossing is **one
  entry in one direction** rather than one publish, argued in the claim's own
  header against the neighbouring decision about doorbells; and the instrument
  submits one delta at a time, so 8.0 crossings per frame is a **floor on the
  honest figure and not the figure**. `E3-B01`'s *under ten per UI frame* belongs
  to the parent task and this claim explicitly refuses to borrow it.

- **A boot-path lint** — `cargo xtask lint-bounds`, `E3` hotfix, RFC 0116, and it
  is on this page because of how it arrived. Four bounds on the boot path were
  each larger than a count kept in another file, every relation was written in a
  comment, and the last time one decayed the only observer was a six-boot nightly
  claim — a day later, and the repair was already on a branch
  (`docs/postmortem/0003`). The relation is arithmetic over two integers in two
  files, so it is now four file reads inside `verify`. The general rule it
  produced is in `CLAUDE.md`: if a check rests on arithmetic between two
  constants, it is a file read and belongs in `verify`, and the expensive workload
  stays nightly and stops being the only observer.

- **A red schedule is now loud** — `ops/alarm.sh`, an `alarm` job in all five
  scheduled workflows, and `cargo xtask lint-schedules`, RFC 0122. The second half
  of the same incident: the report said everything and nobody opened it. Twenty-six
  jobs across five schedules are now watched by a job that opens one issue when
  any of them fails and closes it when the workflow is green, and the lint refuses
  a scheduled workflow whose alarm does not watch every job in its file. What is
  *not* covered is the three `gh` calls, which only a real failure can exercise;
  the decision and the message are tested against fixtures in `cargo xtask test`.

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
  candidate protocols, and the third boot reproduced it exactly.

- **Selection and the declaration comparison have never run on hardware.** Three
  boots outside QEMU, and every one of them printed `generation none selected, so
  no root is published`: the entries `tools/f-on-metal.sh` wrote carried no
  `f.root=`, so the frame measured itself and had nothing to compare against.
  Under QEMU both tokens are on every command line, so RFC 0012's other half is
  exercised constantly on an emulator and had been exercised nowhere else. Found
  by the third boot and fixed in the same change — `f-on-metal.sh install
  --generations` now writes entries that carry both — but **the fix is tested
  under QEMU and staged, not booted**: no machine has yet started from an entry
  carrying `f.root=`. `docs/third-boot-outside-qemu.md` is the record.

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
  three times, all on VMware machines: 2026-09-01, recorded in
  `docs/first-boot-outside-qemu.md`; 2026-09-05 carrying all of E1, in
  `docs/second-boot-outside-qemu.md`; and 2026-09-09 carrying E2, in
  `docs/third-boot-outside-qemu.md`. Three further attempts on a Threadripper
  host, 2026-09-09 to 2026-09-12, did not reach `M0 ok` at all: the first died
  inside core bring-up, and RFC 0068 and RFC 0070 are what came of it; the last
  ran to ring 3 and refused its own stack selector at the first interrupt,
  because `sysret` on an AMD host loads what `IA32_STAR` says and the emulator
  had been forcing the privilege bits this kernel forgot — RFC 0074. That fix is
  tested here only by a self-test that models the vendor QEMU is not; the boot
  that demonstrates it has not happened yet. Each record says in its own opening
  that a hypervisor is not the machine `E0-P18` is about.
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
- **No *time* is measured, and that is the gap the gating claims do not close.**
  This bullet used to read *No claim gates*, and it was describing a registry of
  three entries. Twenty-one of thirty-six gate now — the L7 row above has the
  list — and every one of them is a count, because the rule the split follows is
  *a count may gate on this machine and a time may not*. So the sentence worth
  keeping is the narrower one: `0001` measures the host and is `pending` — no
  user interrupts, no registered buffers, no deadline class — `0002` has a
  threshold and no number, because the only environment available emulates the
  timer against a host clock it does not control, and `F_ENVIRONMENT=container` is
  how the harness already knows to refuse. `0003` is `tracked` and gates nothing
  by design. Fourteen entries are `pending` and most of them are times or ratios
  of times waiting on `E0-D10`'s named machine; `claims/0022` and `claims/0036`
  are the two that say in their own `[hardware]` notes that what they wait on is
  a *task* rather than silicon, which is the distinction the registry otherwise
  cannot express. A thirty-six-entry registry with twenty-one gates and no
  measured time is a stronger position than this bullet used to describe and a
  weaker one than the count suggests, and both halves of that are the point.
- **`cargo xtask verify` is local, and local is not everything.** It runs the
  lints, the host tests, an AArch64 cross-*compile* of the four crates the arm
  job tests, a QEMU boot and the mutation harness. It cannot *run* the AArch64
  tests or the
  litmus job, which are exactly where L2 means anything — x86-64's total store
  order hides the entire class of bug the ring is exposed to. Those run in CI
  and nothing local substitutes for them.
- **L6 is the only absent layer, and this bullet was the stalest line on the
  page.** It read *Every layer above L2 is a plan. L3 and L6 are absent, L4 is
  instrumentation with nothing feeding it, and L5 is a harness with no history
  behind it* — written when it was true, left standing through E1, E2 and most of
  E3, and contradicted by three rows of the table at the top of this same file for
  most of that time. L3 is built and narrow on purpose, L4 is built with two
  committed corpora, L5 has a change-point detector over the one series this
  project can record. **L6 is absent** — the photodiode rig is at phase 03, and
  `claims/0033`'s specification is what exists of it. That is one layer, not four.

  The lesson is about this page rather than about the layers. A document whose
  summary paragraph disagrees with its own table is worse than one with no
  summary, because the summary is what gets quoted; **A-01** exists to stop the
  plan being mistaken for the state of the tree and the plan was in the last
  bullet of the page A-01 owns. `cargo xtask lint-testing-status` compares the L7
  row against the registry and is the only sentence here anything checks; the rest
  is read by people, which is why it decayed and why re-reading it is a task with
  a cadence rather than a task with an exit.
