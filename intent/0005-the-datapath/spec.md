---
id: 0005
status: draft
reviewed_by: "pending: Dmitri Chudinov"
skills: determinism-review, frame-and-unsafe, licence-boundary, claims-registry, rfc-author, memory-ordering
---

# Spec: the datapath, and the apparatus that debugs it

Three virtio drivers leave the kernel and become components with manifests,
IOMMU domains and a supervisor that restarts them. The ring grows the two things
a driver needs and nothing else: buffers owned by one side at a time, and a
deadline that survives the crossing. `env/` stops being a hook and becomes a
simulator with virtual time, seeded ordering and modelled devices, which is what
turns a failure into a `(seed, commit)` pair. And the four numbers the datapath
exists to produce get registered — with their workloads, their baseline
configuration and their reproduction commands — and stay `pending`, because the
machine `claims/runner-class-A.md` specifies does not exist yet. Thirty-four
task ids, four movements, one gate.

## Behaviour

**Decide.** Four RFCs and two schemas, each landing immediately before the work
it would be expensive to redo without, and never at the head of the epoch.
RFC 0008 says a component exists only by a spawn from a manifest, holds one
control ring, and is paid for out of an `Untyped` that can be revoked — so there
is no fork, no signal, and no lifecycle to retrofit around the first
long-lived component. RFC 0005 gives every component a declared speculation
domain — `shared`, `private` or `hostile` — and makes R02 a field a lint reads
rather than a rule review remembers. RFC 0024 says a buffer is owned by one side
at a time and the typestate is what says which, with three misuses that fail to
compile. RFC 0025 says a deadline inherits downward and decays, so a callee is
never promoted above the class it was admitted for. Beside them, the manifest
schema with virtio-blk as its worked example and `lint-manifests` as the schema
made executable, and the tuned-Linux baseline as files a stranger can apply to a
machine rather than a sentence in a claim's `notes`. Afterwards: nothing in the
build movement has to invent a lifecycle, a domain, an ownership rule or a
deadline rule, and `cargo xtask release --dry-run` stops reporting the baseline
absent.

**Build.** A driver becomes a component in the ordinary sense. The kernel
programs an IOMMU and gives each component a domain, so a device told a wrong
address faults instead of corrupting; that is `E1-B01`, and it is first because
`E1-B02`, `E1-B03` and `E1-B04` are each a driver that would otherwise be a bus
master with no bound. virtio-blk reads and writes through a ring with zero
copies on the data path, counted rather than asserted; virtio-net puts packets
in and out and lands a receive in a buffer that was registered before the packet
arrived; virtio-gpu puts something on a framebuffer. A supervisor spawns them
from their manifests, delivers their control rings and restarts them under the
policy the manifest declares. Under all of that, four pieces of frame the
datapath makes load-bearing: the capability table stops being a fixed array and
becomes storage an `Untyped` pays for, with `QUOTA_EXHAUSTED` for a process that
cannot pay; the allocator grows orders, split, coalesce and per-CPU free lists,
so allocation takes no cross-core traffic on the hot path; admission control
gains a schedulability test that can refuse with `ADMISSION`, which is what R08
means by refusing to call a hint a deadline; and a user-level runtime takes
cores as an allocation and preempts only at allocation boundaries, so async work
under load produces zero kernel entries on the hot path. Two of the build tasks
are instrumentation rather than function: a splittable generator behind `Env`
before the simulator multiplies streams, and an unmap-under-churn workload that
either buys shootdown batching or closes the question with the number that says
one-page-one-IPI was already under the bound.

**Prove.** This is the epoch where the testing environment stops being three
layers of seven. The simulator runs a whole boot-to-workload under virtual time
with seeded ordering and modelled devices, and reproduces byte for byte from
`(seed, commit)`. Seven fault classes get scenarios, and each scenario gets a
system response that is asserted rather than observed. A nightly sweep runs N
seeds across M scenarios and minimises any failure to a reproduction command
with no human in the loop — the half of gate G1 that matters more. A hostile
peer writes arbitrary values to the header and the cursors, restarts
mid-operation and lies about its epoch, for a billion operations, with no panic,
no unsafety and no hang; a structure-aware entry fuzzer with coverage feedback
takes the validation path past 95% and commits its corpus. Kani proves the five
capability properties the negative suite samples, and then the ring's validation
paths, so that "no fuzzer found one" becomes "there is none". Driver chaos kills
each driver under sustained load and no client observes anything but latency.
Snapshot and restore make a failure at simulated minute 40 re-enter at minute 39.
The test taxonomy says, for every class of bug, which layer catches it and how
often that layer runs — and every gap in the table is either scheduled or
explicitly accepted. The AArch64 job builds and runs the same suite, and no test
is skipped there without a recorded reason. And four claims are registered for
the datapath: ring submit under load, doorbells per operation, copies per
operation, kernel entries per operation.

**Release.** The simulator ships as a tool rather than as a subdirectory: the
seed corpus, the scenario set, and a published command a third party runs
against their own checkout. Then release 0.2, which is the evidence package the
long plan describes — source at a tag, the claims snapshot, the baseline
configuration, the corpus, the honest-status page — and whose test is that
somebody outside the project runs a seed sweep and re-derives the four datapath
numbers from the package alone.

## Policy applied

Walked in the order `spec-from-intent` gives, because the five disqualify
designs at very different costs.

**1. Determinism.** The simulator is the whole of this section. A device model
that consults anything but `Env` is not a device model, it is a second source of
truth: every completion time, every ordering between two outstanding requests,
every injected failure and every byte a modelled device returns is drawn through
`f_env::Env`, and virtual time is `Env::now` rather than a clock the host
supplies. The obvious implementation is worth naming because it is the one
everybody reaches for and it is what the container currently does — run the real
QEMU device, wait on a real interrupt, assert on what came back. That is a
clock, an ordering and a scheduler the test does not control, and it produces a
suite that is green on Tuesday for reasons nobody can reconstruct on Wednesday.
It stays, as a boot, and it is not the simulator.

`E1-B11` is a determinism task disguised as a build task and it is why it ranks
where it does. `SeededEnv` is xorshift64 and `sim.rs` finalises its site draws
with FNV-1a; one seeded test cannot feel the difference and a sweep of thousands
of correlated streams can, and a sweep whose streams are secretly correlated
explores less than it reports. The splittable derivation lands before the corpus
exists, because a seed corpus is priced in the generator it was drawn from and
migrating one afterwards silently invalidates every recorded reproduction.

Two other places the policy bites. `E1-B08`'s user-level runtime schedules work,
and a scheduler that reads a clock to decide what runs next is a nondeterminism
the simulator cannot undo — so its ordering comes from `Env::scheduler` and its
preemption points are allocation boundaries, which are events rather than
instants. And `xtask` is checked by the same lint it implements: the sweep verb
takes its seeds as arguments and its ordering from a `BTreeMap`, and the day it
wants a `HashMap` for a corpus index is the day the corpus stops enumerating in
the same order twice.

**2. The frame.** A driver is a component and components forbid `unsafe` at
compile time, inherited from the workspace and not overridable without a visible
diff. So the interesting question of this epoch is how virtio-blk touches device
registers at all, and the answer has to be written down before somebody
concludes it needs a fourth crate in the frame.

It does not. The manifest routes the transport's register pages to the component
as a capability; the kernel maps them; and what the component holds is a typed
window handed to it by `ring/`, whose constructor checked the length and the
alignment exactly as `ring/src/mapping.rs` already checks a channel's region
before believing its header. The one volatile access per register is inside
`ring/`, which is in the frame, under a `// SAFETY:` comment discharging a
bound the constructor established. The component above it calls safe methods and
cannot name an address. The alternative — a door call per register write — is
rejected here rather than later: it is a crossing per MMIO access on the exact
path `E1-P10` measures, and it would make the claim a measurement of the
mitigation.

The descriptor rings are the harder half and they are not solved by the same
move. A virtqueue is memory the *device* reads and writes concurrently, which
makes it the channel-header problem again: bytes another party wrote, copied out
before they are believed, never validated in place. The accessors are therefore
`ring/`'s and the driver drives them; and the invariant that a device only ever
reaches memory somebody meant it to reach is **not** the driver's correctness at
all — it is `E1-B01`'s IOMMU domain. That division is the point and it should be
stated in those words: the type system bounds the driver, the IOMMU bounds the
device, and a system that has only one of the two has an unchecked half. It also
says what would reverse it — a platform with no IOMMU, where the whole driver
argument becomes a trust argument and `E1-B01`'s exit cannot be met.

Everything else stays where it is. The IOMMU page tables are `kernel/`. The
device models are `env/`, which forbids `unsafe` and does not need it, because a
modelled device is arithmetic over a queue. `cargo xtask unsafe` is expected to
move, and the number it reports is the one A-05 already watches.

**3. The licence boundary.** Untouched by this epoch, and that was checked
rather than assumed: no E1 task imports anything, the three drivers are written
here because RFC 0003 says so, and `third_party/` still contains one README.
The one place E1 comes near it is RFC 0005, whose `hostile` kind exists for
imported code and whose rule is that a `third_party/` image may declare
`private` or `hostile` and may never declare `shared`. That rule lands with no
image to apply it to, which is the correct order — the field exists before the
first import rather than being added under pressure with a driver already in
tree. `cargo xtask lint-licensing` passes throughout for the uninteresting
reason that there is nothing for it to catch, and the interesting statement is
that when there is, at E5, the check predates it.

**4. Evidence.** Per task, the evidence is the task's `exit:` line in `TODO.md`
and nothing weaker; the grouping by observing command is in *Evidence* below.
Two standing constraints from `claims-registry` shape the epoch rather than its
last week. Numbers first: `E1-P10`'s four claims are registered with statement,
workload, baseline and reproduction command at the moment the capability they
measure is built, not after — R11, and the reason `claims/` was built at M0 with
one entry. Numbers second: they stay `pending`. `claims/runner-class-A.md` is a
specification and not a machine, `E0-D10` is `[>]` for exactly that reason, and
`bench::Environment::classify` fails closed on anything that is not
`runner-class-A`. So this epoch produces four registered claims, a tuned
baseline that can be applied to a machine, and no publishable number — and
`docs/TESTING-STATUS.md` says so in the same change rather than in the next one.

**5. Decisions.** Four RFCs are owed and are being written by the Decide
movement now: **0008** (no fork, no signals — the component lifecycle), **0005**
(speculation is a boundary the language does not draw — three domain kinds),
**0024** (a buffer is owned by one side at a time), **0025** (a deadline
inherits downward and decays). Each contradicts or extends something already in
`docs/design/`, which is the test `CONTRIBUTING.md` sets. Two more may become
owed during the build and are named here so that they are noticed rather than
discovered: a decision about what a killed driver owes its in-flight buffers, if
`E1-P06` cannot be met by the typestate alone; and a decision about the emulated
machine, if `E1-B01` forces a second machine definition and with it a second
boot-log baseline. No RFC number is claimed for either — an RFC number written
before the RFC is a reservation, and the numbering is permanent.

## Not in scope

- **The object store, generation swap and attestation.** `E1-B13` makes the
  capability table pay for its own growth out of an `Untyped`, which is the same
  shape as a quota and will look like the beginning of the object model. It is
  not: E2 owns the store, the index and the swap, and `E2-D01` is where an
  update stops being a reboot.
- **Change-point detection over claim history.** The claims are registered here
  and the history starts accumulating here; the detector is E2's, because it
  needs a year of history to reason about and a threshold introduced as a
  stopgap trains everybody to ignore the signal.
- **The hardware lab, and any published number.** Netboot, power cycling,
  unattended bisect and external energy metering are E5. The four datapath
  claims are registered and pending; `E0-P05`, `E0-P06` and `E0-P18` own the
  machine question and they are E0's, not this epoch's to solve by re-scoping.
- **Proof of the frame.** `E1-P07` and `E1-P12` are bounded proof over the
  capability properties and the ring's validation paths. Deductive proof on the
  frame's invariants is E2, and deliberately narrow forever: the frame and the
  protocols, never the system.
- **A compositor.** `E1-B04` puts something on a framebuffer through a ring. The
  retained scene, path rasterisation, text and input prediction are E3, and the
  latency claim that justifies them is measured with a photodiode there.
- **Imported drivers.** RFC 0005's `hostile` kind and RFC 0003's isolation
  argument both anticipate them; E5 hosts the first one.
- **ABI compatibility across releases.** The wire grows in this epoch under
  ordering rule 1. The compatibility matrix against N-1 peers is E4, which the
  long plan already calls late.

## Evidence

Grouped by the command that observes it, because a list of exits with no runner
beside them is a list of intentions.

**`cargo xtask verify`** — the local gate, and the floor nothing in this epoch
may lower: the policy lints, the workspace tests on both architectures, the
kernel boot asserting its exit code, and the mutation build. `E1-D03`'s exit is
here in an unusual form — the compile-fail fixtures, where a misuse of a buffer
*fails to build* and the test is that it does.

**`cargo xtask lint`** — `lint-manifests` for `E1-D04`'s schema; `lint-units`
for R03 over every new quantity the datapath puts on the wire, deadlines
included; `lint-claims` and `lint-claim-owners` for `E1-P10`'s four entries;
`lint-determinism` for `E1-B11` and every device model; `lint-unsafe` for the
statement that three drivers and a supervisor were added to the tree and the
frame did not widen.

**`cargo xtask run`, `cap`, `user`, `fault`** — the boots. `E1-B01`'s exit is a
boot where a driver component addresses memory outside its grant and takes a
fault rather than corrupting something; `E1-B02`, `E1-B03` and `E1-B04` are each
a boot that moves data and a counter that says how many copies it took;
`E1-B13`'s exit is the whole negative suite passing at a size the fixed table
could not hold, plus a refusal with `QUOTA_EXHAUSTED`.

**`cargo xtask sweep`** (NEW) — `E1-P01`'s byte-identical boot-to-workload run,
`E1-P02`'s seven fault classes with asserted responses, `E1-P03`'s injected bug
found overnight and minimised to one line, `E1-P08`'s re-entry at simulated
minute 39. This verb is gate G1's second half and is what the nightly cadence
runs.

**`cargo xtask fuzz`** (NEW) — `E1-P04`'s billion hostile operations and
`E1-P05`'s coverage of the entry-validation path past 95%, from a corpus that is
committed to the tree and shipped as a release artifact.

**`cargo xtask kani`** (NEW) — `E1-P07`'s five capability properties and
`E1-P12`'s panic-freedom over the ring's validation paths, on the weekly
cadence, each with a mutation that must fail them.

**`cargo xtask claims` and `cargo xtask claim <name>`** — `E1-P10`'s four
entries, reported `pending` with their workloads and their reproduction
commands, and `E1-B14`'s unmap-under-churn workload recording shootdowns, IPIs
and p99 unmap cost beside them.

**`cargo xtask release --dry-run`** — `E1-D06`'s exit exactly: the tuned
baseline present in the package rather than described in a `notes` field.

**CI** — `E1-P11`'s AArch64 job running the same suite with no unexplained skip,
and the nightly and weekly workflows that make the sweep, the fuzzers and the
proofs a cadence rather than a command somebody remembers to run.

**A third party** — `E1-R01` and `E1-R02`, which are the only two exits in this
epoch that nobody here can close: somebody outside runs the published sweep
against their own checkout, and re-derives the four claims from the package.

## Risks and reversal

**The most likely thing to be wrong is that zero copies on the data path is not
achievable through this ring, and the number says so.** `E1-B02`'s exit says
zero, verified by counter. Every layer between a client and the disk is a place
a copy hides: the descriptor ring, the registered-buffer path, the completion,
and the IOMMU's own alignment requirements. *What would reverse this:* the
counter, on the workload `E1-P10` registers. If the answer is one copy rather
than none, the claim records one copy with the reason beside it, and R12 says
that goes next to the number rather than in a rebuttal after somebody measures
it themselves.

**`E1-B14` is the shape every uncertain item in this epoch should take, and it
is worth naming as the model.** Its exit is written so that both outcomes close
it: the workload lands first, and then *either* batching arrives with the
improvement measured on that same workload, *or* the task closes `[~]` with the
number that says one-page-one-IPI was already under the bound. That is a task
that cannot be resolved by opinion, and it is the correct answer to ordering
rule 3 — a decision belongs immediately before the work it would be expensive to
redo, and never before the measurement that would settle it. Where anything else
in this epoch turns into an argument about whether a mechanism is worth its
cost, this is the shape to convert it into.

**The IOMMU may not be reachable from the harness at all.** The intent's open
question is not rhetorical: `machine_with` has never passed `-machine`, so every
boot in this project's history has been on `pc-i440fx`, which has no IOMMU
model, and q35 changes the PCI topology, the debug-exit device's home and the
interrupt controller. The boot log is a byte-compared fixture. *What would
reverse this:* the first `q35` boot. If the log cannot be made stable across the
two machines, the answer is two machine definitions and two baselines rather
than one machine and a relaxed comparison — a fixture that stops being compared
exactly is a fixture that has stopped working.

**The simulator is `XL` and undecomposed, and undecomposed `XL`s are how a
schedule disappears.** `E1-P01` gates `E1-P02`, `E1-P03`, `E1-P08`, `E1-R01` and
`E1-R02` — five tasks and the second half of the gate. *What would reverse this:*
the first piece that does not close on its own observation. The decomposition
proposed in the intent is four pieces each with an exit; if the first of them
cannot be stated that way, the task is bigger than the estimate rather than
behind schedule.

**Nothing here produces a defensible number, and that is the second time in a
row.** E0 ended with two pending claims and a specification for a machine that
does not exist; E1 adds four more to the same queue. The risk is not technical,
it is that "pending" stops being read as a state and starts being read as a
style. *What would reverse this:* `E0-P18` and `E0-D10`'s machine half. Until
then `docs/TESTING-STATUS.md` carries the count, and a claim that has been
`pending` across two epochs should be uncomfortable to look at.

**A supervisor that restarts a driver is a new way to lose data quietly.**
`E1-P06`'s exit says no client observes anything except added latency, and the
in-flight buffers at the moment of death are where that sentence is either true
or a slogan. *What would reverse this:* a chaos run where a client observes a
short read rather than a delay. RFC 0024 already names the drop bomb and the
teardown as the answer; if it is not sufficient, cancellation becomes a concept
this system has, which is a decision and therefore an RFC.

## Decisions taken on the originator's behalf

Flagged rather than buried, because these are the places a review is worth the
originator's time.

1. **All thirty-four E1 tasks are in scope for one intent**, rather than one
   intent per movement. The reason is the same one intent 0004 gives: none of
   the prove movement can be observed without the build movement, and splitting
   them produces a record where the interesting dependency is invisible.
2. **The four datapath claims are registered `pending` rather than deferred to
   E0's machine landing.** Registering the apparatus with the capability is R11;
   deferring registration until a number exists is how a claim gets the evidence
   that happened to be available.
3. **MMIO is a typed window from `ring/`, not a door call per access.** Stated
   under *The frame* with its cost. This is the decision most likely to be
   re-litigated and it is the one that most directly shapes `E1-P10`.
4. **The descriptor ring is treated as hostile memory**, on the same footing as
   a channel header a peer wrote, and the device's reach is bounded by the IOMMU
   rather than by the driver's correctness.
5. **`E1-B04` (virtio-gpu) stays in the epoch.** Carried from the intent's open
   question with a reason rather than resolved silently: three instances is what
   makes the driver container a shape rather than a special case, and the
   framebuffer boot is cheap. If it slips, it slips as an E3 task and not as a
   silent omission.
6. **`E1-B09` is not allowed to hold the epoch.** Also carried from the intent
   rather than answered: if `E0-B15` has not landed the user-interrupt path when
   `E1-P10` is otherwise ready, the doorbell claim records the kernel path
   measured and the user-interrupt path absent, with the hardware named. The
   originator may prefer to wait, which is why it is here.
7. **The `XL` decomposition is proposed and not adopted.** Whether `E1-P01`
   becomes four task ids is a change to `TODO.md`, and `TODO.md` is not this
   record's to edit.
