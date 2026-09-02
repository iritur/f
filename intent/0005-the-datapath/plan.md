---
id: 0005
status: in-progress
spec: ./spec.md
---

# Plan: thirty-four tasks, in the order the graph says

Not one pull request. E1 is an epoch and the tasks close individually against
their own exits — what this plan fixes is the shape of the diff for each
movement and the order the movements interleave in, so that a task landing on
its own does not find a file it needed built differently by the one before it.

Marked `NEW` means the path did not exist when E1 started. Some of the Decide
rows are already `NEW` and already merged: `d4be53e` landed RFCs 0005, 0008,
0024 and 0025 with the manifest schema, which is the first four rows of the
first group.

## Files

### Decide — the wire, the lifecycle and the baseline

```
docs/rfc/0008-no-fork-no-signals.md         NEW: E1-D01. Spawn from a manifest,
                                            one control ring, an Untyped that
                                            pays for the component
docs/rfc/0005-speculation-is-a-boundary.md  NEW: E1-D02. shared, private,
                                            hostile — assigned in the topology
docs/rfc/0024-a-buffer-is-owned-by-one-side.md
                                            NEW: E1-D03. The typestate, and the
                                            three misuses that must not compile
docs/rfc/0025-a-deadline-inherits-downward-and-decays.md
                                            NEW: E1-D05. What stops a component
                                            claiming urgency forever
docs/rfc/README.md                          the index row per RFC
abi/src/buf.rs                              NEW: how a buffer is named on the
                                            wire — a set id and an index, or an
                                            address behind the feature bit
abi/src/deadline.rs                         NEW: the quantity R03 says must
                                            state its unit, its epoch and its
                                            zero, which `deadline: u64` did not
abi/src/cap.rs                              the handles a spawn routes, and the
                                            kinds a driver holds
abi/src/lib.rs                              the modules, the feature bits, the
                                            error domains a refusal names
ring/src/buffers.rs                         NEW: E1-D03's types. Idle and
                                            InFlight, Fixed and Virtual naming
ring/src/lib.rs                             the module, and what a Channel now
                                            carries
docs/manifest.md                            NEW: E1-D04. The schema, field by
                                            field, and why it is closed
user/virtio-blk/manifest.toml               NEW: the worked example, written
                                            before the driver it describes
xtask/src/manifest.rs                       NEW: lint-manifests. A schema
                                            nothing checks is prose
claims/baselines/linux-6.x-tuned/           NEW: E1-D06. apply.sh, verify.sh,
                                            baseline.conf, cmdline.txt,
                                            sysctl.conf, lib.sh, README.md —
                                            configuration a stranger can run
RELEASING.md                                the baseline is a package member
```

### Build — the frame the datapath needs, then the drivers

```
kernel/src/iommu.rs                         NEW: E1-B01. Domains, a page table
                                            per component, the fault handler
                                            that makes an escape a fault
kernel/src/arch/x86_64/pci.rs               NEW: enumerating the device and
                                            finding the BAR the manifest routes
kernel/src/arch/x86_64/paging.rs            device pages into a component's
                                            space; what a DMA-visible mapping is
kernel/src/cap.rs                           E1-B13: the table becomes storage an
                                            Untyped pays for; QUOTA_EXHAUSTED;
                                            the device and interrupt kinds
kernel/src/mem.rs                           E1-B12: orders above zero, split
                                            and coalesce, per-CPU free lists,
                                            Order::HUGE as the grain a shard is
                                            refilled in, the M1 pair retired
                                            rather than kept beside it. One
                                            file rather than the mem/buddy.rs
                                            this row first named, because the
                                            exit says the M1 structure is
                                            retired and two files holding free
                                            memory is the second structure that
                                            can disagree with the first
kernel/src/sched/mod.rs                     NEW: E1-B07. Where the classes live
kernel/src/sched/admission.rs               NEW: the schedulability test that
                                            can refuse, and ADMISSION as the
                                            refusal R08 requires
kernel/src/process.rs                       spawn from a manifest, the restart
                                            policy, the domain a component was
                                            declared in
kernel/src/ring.rs                          E1-B06: every resource scheduler
                                            orders by the same deadline field
kernel/src/doorbell.rs                      E1-B09: the user-interrupt path
                                            behind the negotiated bit
kernel/src/smp.rs                           E1-B14: batched shootdown, if the
                                            number buys it
ring/src/mmio.rs                            NEW: the typed register window. The
                                            one volatile access, in the frame,
                                            so the driver above needs none
ring/src/virtq.rs                           NEW: the descriptor ring, treated
                                            as memory a peer wrote — copied out
                                            before it is believed
ring/src/buffers.rs                         E1-B10: registered sets, and the
                                            shared-virtual-memory path
env/src/split.rs                            NEW: E1-B11. One derivation, shared
                                            by SeededEnv and the site draws
env/src/lib.rs                              SeededEnv on the new generator
env/src/sim.rs                              the site finaliser stops being a
                                            second, unrelated hash
user/virtio-blk/                            NEW: E1-B02. Cargo.toml, src/,
                                            link.ld — the path the manifest
                                            already fixes
user/virtio-net/                            NEW: E1-B03, and its manifest
user/virtio-gpu/                            NEW: E1-B04, and its manifest
user/supervisor/                            NEW: E1-B05, and its manifest. The
                                            component the fixed table broke on
user/runtime/                               NEW: E1-B08. Cores as an
                                            allocation; preemption only at
                                            allocation boundaries
Cargo.toml                                  the new members, and the forbid
                                            they inherit without opting out
xtask/src/main.rs                           a second machine definition if q35
                                            is needed; the driver boots
```

### Prove — the simulator, the fuzzers, the proofs, the claims

```
env/src/sim/mod.rs                          NEW: E1-P01. sim.rs becomes a
                                            directory; the hook keeps its API
env/src/sim/time.rs                         NEW: virtual time and seeded
                                            ordering under Env
env/src/sim/blk.rs                          NEW: the first device model, and
                                            the one E1-B02 is measured against
env/src/sim/net.rs                          NEW
env/src/sim/gpu.rs                          NEW
env/src/sim/faults.rs                       NEW: E1-P02's seven classes, each
                                            with a scenario and an asserted
                                            response
env/src/sim/snapshot.rs                     NEW: E1-P08. Minute 39 without the
                                            first 39
xtask/src/sweep.rs                          NEW: E1-P03. N seeds, M scenarios,
                                            minimisation to one line
xtask/src/fuzz.rs                           NEW: E1-P04 and E1-P05
xtask/src/kani.rs                           NEW: E1-P07 and E1-P12, with the
                                            checker's own toolchain — RFC 0022
fuzz/                                       NEW: targets and the committed
                                            corpus, which is a release artifact
ring/tests/hostile.rs                       NEW: the peer that lies about its
                                            epoch, for a billion operations
ring/src/lib.rs                             the #[cfg(kani)] harnesses for pop,
                                            take, adopt and execute
kernel/src/cap.rs                           the #[cfg(kani)] harnesses for the
                                            five properties
bench/src/bin/                              NEW benches: submit under load,
                                            doorbells, copies, kernel entries
claims/0004-ring-submit-under-load.toml     NEW: E1-P10
claims/0005-doorbells-per-operation.toml    NEW: E1-P10
claims/0006-copies-per-operation.toml       NEW: E1-P10
claims/0007-kernel-entries-per-operation.toml
                                            NEW: E1-P10
claims/0008-unmap-under-churn.toml          NEW: E1-B14's workload, registered
                                            before the batching it may buy
claims/snapshot.json                        the five new entries, pending
docs/test-taxonomy.md                       NEW: E1-P09. Which layer catches
                                            what, and how often it runs
docs/test-taxonomy.toml                     NEW: the same table as data, so a
                                            row cannot rot silently
.github/workflows/ci.yml                    E1-P11: the AArch64 job runs the
                                            suite, and a skip carries a reason
.github/workflows/nightly.yml               NEW: the sweeps and the fuzzers
.github/workflows/weekly.yml                NEW: the proofs
docs/TESTING-STATUS.md                      four layers move off their status
```

### Release

```
docs/design/fast-path.html                  the datapath sections stop being
                                            proposals
docs/design/proving-ground.html             layers 1 to 4
docs/design/deadline-all-the-way-down.html  the allocator and the admission test
docs/design/ring-scene-boot.html            buffer ownership and the doorbell
docs/design/lineage-and-debts.html          what is still owed, minus what is not
README.md                                   what a stranger runs first
RELEASING.md                                the 0.2 package contents
intent/0005-the-datapath/                   NEW: this intent, its spec, this plan
TODO.md                                     thirty-four status changes, and the
                                            intent named on each line
```

## Order

The order `cargo xtask todo E1` computes, with the reason for each rule from
`TODO.md`'s *Ordering* section beside it. The numbers are that command's, taken
before any of E1 was marked done.

1. **The wire first, whatever else is ready** — rule 1, the only rule that
   overrides the ranking. `E1-D01` (unblocks 10) and `E1-D04` (unblocks 8) are
   also the top of the ranking, so rule 1 and rule 2 agree here; `E1-D03` and
   `E1-D05` unblock one task each and go with them anyway, because they change
   `abi/` and a change to `abi/` is cheap while one peer exists and expensive
   once two do. `E1-D02` unblocks nothing in the graph and is here for the same
   reason: it puts a field in the manifest format, and a format gains fields
   before it has instances or not at all.
2. **`E1-B11`, the splittable generator** — unblocks 7, and rule 3 in its
   sharpest form. It has to precede `E1-P01` rather than follow it, because the
   seed corpus the simulator accumulates is priced in the generator it was drawn
   from and migrating one afterwards invalidates every recorded reproduction
   without failing anything.
3. **`E1-B01`, the IOMMU** — unblocks 5. Three drivers wait on it, and a driver
   built before it is a bus master with no bound, which means every test written
   against it is a test of a system that will not exist.
4. **`E1-B13` → `E1-B05` → `E1-B08` and `E1-P06`.** The capability table has
   recorded its own reversal condition since M4 — a component that legitimately
   holds more than the fixed count — and the supervisor is that component. So
   the table grows first, the supervisor is built on it, and the runtime and the
   chaos suite follow the supervisor because neither has anything to spawn or to
   kill without it.
5. **`E1-B01` → `E1-B02`, `E1-B03`, `E1-B04` → `E1-B06` and `E1-B14`.** blk
   first among the three, because it is the device `E1-P10`'s workloads use and
   the one `E1-B14`'s churn workload needs; net and gpu after, as the second and
   third instances that say the container is a shape. Deadline propagation and
   the unmap-under-churn workload both need a device queue to be measured in.
6. **`E1-B11` → `E1-P01` → `E1-P02` and `E1-P08` → `E1-P03` → `E1-R01` →
   `E1-R02`.** The long pole, and the one that produces gate G1's second half.
   `E1-P02` and `E1-P08` are independent of each other and both need the
   simulator; `E1-P03` needs the fault classes to sweep across; the release
   tasks need a sweep somebody outside can run.
7. **`E1-P07` → `E1-P12`.** The capability proofs bring the checker and its
   toolchain; the ring proofs then cost a harness rather than a decision.
8. **`E1-B09` waits on `E0-B15`**, which is `[>]` and not this epoch's. If it is
   still open when `E1-P10` is otherwise ready, the spec's decision 6 applies.
9. **Everything that unblocks nothing** — `E1-D06`, `E1-B07`, `E1-B12`,
   `E1-P04`, `E1-P05`, `E1-P09`, `E1-P11` — is real work taken when it fits,
   by rule 2's second half. Two of them are worth pulling earlier than "when it
   fits" by rule 4, because they produce information: `E1-P11` will say which of
   the new crates does not compile off x86-64, and that is a class of finding
   the local loop is structurally unable to see; `E1-B12` will say whether the
   allocator's per-CPU lists change the numbers `E1-P10` is about to register,
   and finding that out afterwards means measuring twice.

Each step goes to a green `cargo xtask verify` before the next one starts,
which is the discipline intent 0004 used and the reason its findings were
findings rather than a debugging session at the end.

## Proof

```
cargo xtask verify
```

is the gate before any of it is offered for review, and it is expected to stay
green at every step rather than at the end. Per movement, the commands that
observe the exits:

```
cargo xtask lint-manifests                 E1-D04
cargo xtask release --dry-run              E1-D06 — the baseline present
cargo test -p f-ring buffers               E1-D03, including the compile-fail
                                           fixtures, where the test is that a
                                           misuse does not build
cargo xtask run                            E1-B02, E1-B03, E1-B04 — data moves,
                                           and the copy counter says how
cargo xtask cap                            E1-B01's escape, E1-B13 at the new
                                           table size
cargo xtask timer 60                       E1-B07 — a granted reservation meets
                                           its deadline under adversarial load
cargo xtask sweep                          E1-P01, E1-P02, E1-P03, E1-P08
cargo xtask fuzz                           E1-P04, E1-P05
cargo xtask kani                           E1-P07, E1-P12
cargo xtask claims                         E1-P10's four, pending, with their
                                           workloads and reproduction commands
cargo xtask claim unmap-under-churn        E1-B14 — the number that either buys
                                           batching or closes the task
```

Two exits no command here can close, and they are the two that matter most:
`E1-R01` and `E1-R02` are met by somebody outside this project running the
published sweep and re-deriving the four claims from the package. A command that
proves that from inside the checkout would be proving something else.

## Risks

Build risks, as distinct from design risks — those are the spec's.

**The q35 machine re-baselines the boot log.** If `E1-B01` needs
`-machine q35,kernel-irqchip=split`, every fixture comparing a boot log byte for
byte moves at once, and a re-baselining that happens in the same commit as a
functional change hides the functional change. The order is: move the machine,
re-baseline, verify green with no behaviour change, and only then program an
IOMMU.

**`env/src/sim.rs` becomes a directory.** The module keeps its public API and
`kernel/src/env.rs` keeps compiling, or the change is bigger than it looks. A
file-to-directory move plus an API change in one commit is the shape of a
refactor nobody can review.

**Five new crates inherit `unsafe_code = "forbid"` and have to keep inheriting
it.** The scar `CLAUDE.md` records is that a component cannot name its own entry
point, so each driver is a library with a linker script and a `[profile.init]`
build — and link-time optimisation on a bare-metal library links to an empty
image, silently, with a failure that looks like the entry point having moved.
Five new images is five chances to pay that again.

**The AArch64 job is where a component that only compiles on x86-64 is found.**
`cargo xtask test` cross-compiles the crates that job tests; the list of crates
grows by five here and the cross-compile list has to grow with it, or the local
loop goes on being green while CI is not.

**Kani brings its own toolchain.** RFC 0022 already records this for the model
checker: a verifier pinned to a different LLVM than this tree's is a second
toolchain in the build, and `rust-toolchain.toml` may not move to accommodate it
because moving it invalidates every claim. The proofs run in their own job with
their own pin, or they do not run.

**The corpus is committed to the tree and the tree is not a corpus store.**
`E1-P05` says the corpus is a release artifact; a coverage-guided fuzzer will
produce more of it than anybody wants in git history. The minimised corpus is
what is committed, and the minimisation has to exist before the first sweep
rather than after the repository has grown.

**Four claims registered before the machine exists is four files that can rot.**
`lint-claims` and `lint-claim-owners` are what stop that, and both have to pass
on a `pending` entry with no number in it — which is a state the linter has seen
twice and is about to see five more times.

**The target volume is shared.** Two sessions building the workspace at once
wait on a lock; a build that appears to hang is usually that. It is a cost of
the container and not a symptom.
