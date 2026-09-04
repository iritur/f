---
id: 0005
status: accepted
originator: Dmitri Chudinov
todo: E1-D01, E1-D02, E1-D03, E1-D04, E1-D05, E1-D06, E1-B01, E1-B02, E1-B03, E1-B04, E1-B05, E1-B06, E1-B07, E1-B08, E1-B09, E1-B10, E1-B11, E1-B12, E1-B13, E1-B14, E1-P01, E1-P02, E1-P03, E1-P04, E1-P05, E1-P06, E1-P07, E1-P08, E1-P09, E1-P10, E1-P11, E1-P12, E1-R01, E1-R02
---

# The drivers move out, and the bugs stop being ours to find by hand

*"Implement all stages from E1 from TODO."*

## Problem

E0 ended with a system that boots, is deterministic, has capabilities, runs a
process on a core that is not the one holding the timer, and speaks one ring.
Every sentence in that list is true and none of them is about doing anything.
There is no device on the other end of anything. Nothing reads a disk, nothing
sends a packet, nothing draws. The only program that has ever crossed the frame
is a component the frame itself compiled and an adversary the frame itself
wrote, and both were handed their memory by the same allocator that afterwards
checked whether they had touched anything else.

That matters more than it sounds, because the numbers this project exists to
defend are datapath numbers. Copies per operation, kernel entries per operation,
doorbells per operation, submit latency under load: four claims, and not one of
them can be taken on a system where there is no operation.
`docs/design/fast-path.html` argues that the time goes in the crossings and the
copies rather than in the work; E0 built the crossing and put nothing on the far
side of it. A thesis about a datapath, with no datapath, is a design document.

The second half is the apparatus, and `docs/the-long-plan.html` section 04 is
blunt about why it lands here rather than later: deterministic simulation needs
components to substitute and devices to model, and neither exists before the
datapath. `docs/TESTING-STATUS.md` says three of the seven layers exist in some
form, and the honest reading of the other four is that they were deferred
correctly and are now due. The way a project like this one dies is not that it
fails to build the thing; it is that the cost of finding out *why* the thing is
wrong grows until it eats the schedule. E0 paid for the hooks — the determinism
substrate, the fault-injection site labels, coverage instrumentation, the claims
registry — precisely so that this is the epoch where they become a machine that
finds bugs while nobody is watching.

Then the uncomfortable one. A driver is the code most likely to be wrong and the
only code that talks to hardware, and today nothing stands between a bus master
and all of memory. The capability system can take back a name and, since
E0-B10, the mapping with it — and none of that reaches a device that was handed
an address an hour ago. `docs/the-long-plan.html` section 06 has one row with no
catching layer at all: a speculative read across a domain boundary, uncovered
"because the domains do not exist yet". Both of those are E1's, and both are
places where the architecture currently claims something the tree cannot
demonstrate.

## Proposed outcome

Gate G1, which is two observations and not one.

**A driver is killed under sustained load and the system does not notice.** Not
"restarts cleanly" — no client observes anything except added latency. That
sentence is only sayable if a driver is a component with its own address space,
its own IOMMU domain, a manifest saying what it may hold, a supervisor that
restarts it, and buffers whose ownership survives the death of one side.

**A bug injected into any component is found by an overnight seed sweep and
arrives as a reproduction command rather than as a symptom.** Not a stack trace
for somebody to chase: a `(seed, commit)` pair and one line that replays it,
minimised, with no human triage in between.

Behind the gate, release 0.2 — the datapath claims, the simulator with its seed
corpus and scenario set, and the fuzzing corpus, packaged so that a third party
runs the sweep and re-derives the four numbers from the package alone.

## Affected users and systems

Every crate in the workspace, which is unusual and is the honest answer:

- `abi/` gains the fields the datapath needs on the wire, and nothing else;
- `env/` gains a splittable generator and device models, and stops being a hook;
- `ring/` gains buffer ownership and the registered-buffer path;
- `kernel/` gains an IOMMU, admission control, a capability table that grows, an
  allocator with orders, and deadline propagation;
- a component tree outside `user/init` — three drivers and a supervisor;
- `xtask/` gains the verbs that run a sweep, a fuzzer and a proof;
- `claims/` gains four entries and a baseline that is configuration rather than
  prose;
- `bench/`, because a number measured through a driver is a different harness
  from a number measured in a loop.

`third_party/` is not touched. The imported graphics stack is E5, and the
licence boundary is not crossed by anything in this epoch.

The `docs/design/` pages that have to change, which is the expensive part:
`fast-path.html`, where the datapath sections stop being proposals;
`proving-ground.html`, where layers 1 to 4 move off *planned*;
`deadline-all-the-way-down.html`, where the allocator and the admission test
acquire an implementation; `ring-scene-boot.html`, for buffer ownership and the
doorbell; and `lineage-and-debts.html`, which is the page that says what is
still owed and will owe less.

## Constraints

The three policies in `CONTRIBUTING.md` constrain everything and are not
restated. What is particular to this epoch:

- **The wire goes first.** `TODO.md`'s ordering rule 1 overrides the ranking:
  a change to `abi/` is cheap while one peer exists and expensive once two do,
  and E1 is the epoch that produces the second peer. Every field a driver needs
  on the wire lands before the driver that reads it.
- **No toolchain bump.** `rust-toolchain.toml` is pinned, and bumping it as a
  side effect of something else invalidates every claim (`claims/README.md`).
- **The measurement environment is still not one.** `claims/runner-class-A.md`
  specifies the machine; `E0-D10` is `[>]` because the specification half is met
  and the machine half is a purchase order rather than a commit. `E0-P05` and
  `E0-P06` wait on that machine and on `E0-P18`. So the four datapath claims are
  *registered* — statement, workload, baseline, reproduction command — and stay
  `pending` until there is a machine. A number taken in the container is a
  number about the container, and `docker/README.md` already says so.
- **Nothing here may make the existing suite optional.** Seven `user=` boots,
  eight `cap=` boots, six faults, the mutation build and the litmus job are the
  floor. An epoch that adds a simulator is exactly the epoch that would be
  tempted to argue the boots have become redundant.

## Open questions

- **`E1-B09` is blocked on work that is in flight.** `E0-B15` is `[>]`: the
  suppression fence and the typed doorbell landed, the user-interrupt path
  behind the negotiated feature bit did not. `E1-B09` measures both paths and
  `E1-P10`'s doorbell claim wants the number. Does E1 wait for it, or does the
  claim ship with the kernel path measured and the user-interrupt path recorded
  as absent? The second is honest and weaker, and it is a choice somebody has to
  make rather than discover.
- **`E1-B01` needs an IOMMU, and the harness has no machine that has one.**
  `machine_with` in `xtask/src/main.rs` pins the memory size, `-smp 2`,
  `isa-debug-exit` and the serial line — and passes no `-machine` at all, so
  every boot this project has ever taken has been on QEMU's default, which is
  `pc-i440fx` (7.2 in the container) and has no IOMMU model at all. Getting one
  means `-machine q35,kernel-irqchip=split` with `-device
  intel-iommu,intremap=on`, and q35 is a different machine: a different PCI
  topology, a different home for the debug-exit device, an interrupt controller
  split in two. The boot log is a fixture compared byte for byte, so this is not
  a flag — it is a second machine definition and a re-baselined log. One machine
  for the whole epoch, or two, with q35 used only by the boots that need a
  device?
- **`E1-P01` is `XL` and is not decomposed**, which this list calls a planning
  failure by the time such a task starts. It carries virtual time, seeded
  scheduling and ordering, three device models and component substitution in one
  line. Offered as a question rather than as an answer: four pieces, each
  closing on its own observation — virtual time and a scheduler under `Env`; one
  device model, blk, because `E1-B02` is the first driver, proved by a run that
  reproduces byte for byte; the other two device models; and component
  substitution, which is what makes the simulator worth having and is also the
  piece that cannot exist before the supervisor does. Does that become four task
  ids, or does `E1-P01` stay one task with four exits?
- **Which driver earns its place in this epoch?** blk and net carry claims.
  `E1-B04`, virtio-gpu, carries none until E3 has a compositor to put on it, and
  its exit is "something appears on the framebuffer". It may be right as the
  third instance that shows the driver container is a shape and not a special
  case. It may equally be E3's, done once instead of twice.
- **What does a killed driver owe its clients?** `E1-P06` says no client
  observes anything but latency. The buffers in flight at the moment of death
  are the hard case, and whatever is decided there decides whether the ownership
  types are sufficient or whether cancellation has to exist as a concept.
