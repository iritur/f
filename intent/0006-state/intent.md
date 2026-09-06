---
id: 0006
status: accepted
originator: Dmitri Chudinov
todo: E2-D01, E2-D02, E2-D03, E2-D04, E2-B01, E2-B02, E2-B03, E2-B04, E2-B05, E2-B06, E2-B07, E2-B08, E2-B09, E2-P01, E2-P02, E2-P03, E2-P04, E2-P05, E2-P06, E2-P07, E2-P08, E2-P09, E2-P10, E2-R01, E1-B15
---

# The machine can say what it is running, and take it back

*"start to build E2 using claude's workflows for agents orcestration."*

## Problem

E1 built a datapath and left behind a machine with no memory. A driver reads
blocks off a virtio device in user space, the ring carries them without a copy,
and at the next boot none of it happened. Nothing that runs here has a name
that outlives the run. The only durable identity anything in this tree has is
the SHA-256 that `xtask/src/pack.rs` computes over a release archive, on the
host, after the fact — a name for the package on a shelf, and no name at all for
the machine that is booted from it. Two machines running "the same thing" have
no way to check, and one machine has no way to say what it was running
yesterday.

That is the first half. The second is that there is nothing to undo, and it is
the more expensive half because the design documents already sell it. Three
rows of `docs/design/lineage-and-debts.html` section 05 mark F *Ahead* of
Nix, Guix, git and Unison: the system as a pure function of one expression,
immutability enforced by the storage layer rather than observed by convention,
content addressing joined to zoned devices where the append-only shape stops
being a cost. Every one of those rows is about something that does not exist.
`docs/design/fast-path.html` section 06 says *you can change anything because
you can undo everything*; today you can change nothing and undo nothing. A
comparison that says *ahead* of a system that ships, on an axis this tree has
not started, is exactly the overclaim `lineage-and-debts` was written to
prevent.

The third piece is what the E1 machinery already promises and cannot yet keep.
A component that dies is respawned into its place — `E1-B05`'s endpoint as a
place rather than an instance — and a client waiting on that place waits for
the refill. That is restart, and it is the only update mechanism the system
has: a component that is to be *replaced*, by something newer, under load, has
no path at all except the whole machine going down and coming up different. So
"an update" currently means "a reboot", the metric in the long plan says so,
and nothing measured in E1 says how much of that is necessary.

Two debts carried from earlier epochs come due here, and both are named
because they are cheaper to pay before this epoch's work is built on them. RFC
0013 gave the state tree one test of worth — *whether E1's fault sweeps and
E2's state comparison actually consume it* — and the tree is still the frame's
alone: three drivers and the runtime hand the frame a tally and the frame
publishes it under its own root, and `E1-B15` is the task that was missing
until 2026-09-06. A whole-system state is one only when every component
publishes into it, and comparing two whole-system states is what E2 is for.
And the storage design's own page, `docs/design/deadline-all-the-way-down.html`
section 04, ends by naming the workload content addressing is bad at — small
random writes to a large mutable object — and calls the mitigation *a real
complication rather than an elegant unification*. Until 2026-09-06 the design
task for that mitigation had nothing that built it, so the honest paragraph was
on its way to becoming the only paragraph.

## Proposed outcome

Gate G2, which is three observations:

**Break the system deliberately and roll it back.** From the boot menu, to a
generation that is verified bit-identical to what it was, because a generation
is one hash and rollback is a root swap.

**Replace a running component without dropping work.** Under sustained load, no
client observes a dropped operation, and the state that crossed from the old
instance to the new one is verified rather than assumed.

**Cut power at every write boundary of a publish and never observe a state that
was not one of the two intended ones.** Across a full sweep of cut points and
seeds in the simulator: the old root or the new one, never a third thing.

Behind the gate, four things that were sentences become numbers or mechanisms.
The machine answers *what are you running* with one hash, and any modification
produces a different one. Bytes written to the device per byte written by the
application are a registered claim against a tuned Linux filesystem on the same
device, with the emulated device marked as emulated if that is what ran. Bytes
re-chunked and re-hashed per application byte are a registered claim on *both*
object kinds, so the workload the design is bad at has a number beside the
workload it is good at. And copies per read is zero, counted rather than
asserted. The rollback metric changes from *one reboot* to *one generation
swap, and a reboot only when the frame changed*.

Release 0.3 packages the storage and generation claims, the rollback and
live-swap demonstrations, and the attestation story, and the rollback
demonstration runs from the release image on a stranger's machine.

## Affected users and systems

Fewer crates than E1 touched, and a new tier above them:

- `abi/` gains the hash as a wire type, the generation root, and the
  state-transfer schema a component declares — fixed-width records, and
  nothing else;
- `env/` gains a power-cut model, so the simulator can stop a publish at a
  chosen write boundary and be run again from the same seed;
- `kernel/` gains measured boot into a root of trust, a place to mount every
  component's subtree under one root, and the quiescent point at which routing
  swaps — and loses the restart policy `E1-B05` left in it, if that reversal is
  paid on the way;
- a component tree that did not exist: the blob store, the index, the
  evaluator and the assembler — with the collector inside the store, reaching
  the zoned device the only way anything does, over the blk driver's ring;
- `user/virtio-blk` declares the first state-transfer protocol and is the
  first component swapped under load;
- every component the supervisor starts publishes a state tree of its own;
- `xtask/` gains the verbs that run the cut-point sweep, the two-machine
  two-date reproducibility job, and change-point detection over the stored
  history;
- `claims/` gains the write-amplification and re-chunking entries and a second
  baseline that is configuration rather than prose;
- `docker/`, because the emulated zoned device needs a newer QEMU than the
  image carries, and the image digest is in every reproduction.

`third_party/` is not touched. The licence boundary is not crossed by anything
in this epoch.

The `docs/design/` pages that have to change, which is the expensive part:
`deadline-all-the-way-down.html`, where section 04 stops being *the design
behind the assertion* and section 07's boot topology acquires an assembler that
exists, and where the write-amplification row of its claims table is rendered
from the registry or removed; `fast-path.html` section 06, where the first of
the three mechanisms becomes true and the other two are still owed to E3 and
E4; `lineage-and-debts.html` section 05, where the three *Ahead* rows are
either measured or downgraded — this is the page that is supposed to lose
honestly, and it should lose here if the numbers say so; and
`proving-ground.html`, where layer L8 moves off *E0 v0, E1 everywhere* to a
date, and layer L5's change-point detection stops being a rule in
`claims/README.md` and becomes a job. Outside `docs/design/`,
`docs/the-long-plan.html` carries gate G2 and the L8 row that says *E2, when
comparing two whole-system states is comparing two hashes*, and RFC 0013 names
E2's crash-consistency work as the evidence that would reverse its per-node
atomicity — so that section is read again at the end of this epoch, with the
sweep's findings in hand.

## Constraints

The three policies in `CONTRIBUTING.md` constrain everything and are not
restated. What is particular to this epoch:

- **The frame stays per-CPU and lock-free.** A store with a collector is the
  first subsystem in this tree that genuinely wants a lock — mark from live
  roots while a hard-class reader is in flight — and it does not get one in the
  frame. Every mutable `static` under `kernel/` is a `PerCpu<T>`, two cores
  meet in exactly four places, and a fifth needs an argument (RFC 0016). The
  store, the index, the evaluator and the assembler are components, and what
  the frame gains from E2 is a mount point, a swap point and a measured root.
  If something in this epoch cannot be built without the frame holding a lock,
  that is a finding to bring back here, not a line to write.
- **`abi/` is a wire format.** It gains the hash, the root and the transfer
  schema as fixed-width bytes, the way `E1` gave it the datapath fields and
  nothing else. No chunker, no evaluator, no policy, no string lives there. And
  ordering rule 1 still applies: the schema `E2-D04` defines is read by two
  peers the moment `E2-P08` swaps one, so it lands before the component that
  declares it.
- **One hash function, one notion of identity.** SHA-256 is already chosen,
  once, host-side, for the release address. The store needs the same function
  under `no_std` and it should be the same function: a release address and a
  blob address computed by two algorithms is two identities where RFC 0012
  wants one.
- **The claims registry is where numbers go.** Write amplification and bytes
  re-chunked are counts, and `claims/0005` showed a count can gate on the
  machines this project has; they are registered and gating here. Copies per
  read is a count too. Read-path latency and resident bytes under load are
  timings, and every timing claim in the registry is `pending` until
  `E0-D10`'s machine exists — the container refuses to record one and is right
  to. The second half of `E2-P09`'s exit, ordinary noise not detected, needs a
  gating claim that has run repeatedly on that machine, and that is `E0-P06`'s.
  The tuned-Linux baseline is configuration in the tree under the rule
  `E1-D06` made a task, or it decays into prose.
- **The development image's QEMU is Debian bookworm's 7.2.** Zoned virtio-blk
  emulation is materially better from QEMU 8, and `docker/README.md` names the
  one-line base change to trixie that `E2-B02` and `E2-P10` need. It is its own
  commit, because the image digest is in every reproduction command — and it is
  the moment `docker/README.md`'s admitted gap becomes this epoch's business:
  the base is repeatable but not pinned by digest, and `E2-P06` is the job that
  will name a non-reproducible input, so the image should not be the first
  input it names.
- **No toolchain bump.** `rust-toolchain.toml` is pinned and bumping it as a
  side effect invalidates every claim. `E2-P04` wants Verus, and Verus pins its
  own rustc; RFC 0022's shape — the checker's toolchain in an image target only
  the checking job uses, already proven for Kani — is reused rather than
  re-argued.
- **Nothing here may make the existing suite optional.** The seven `user=`
  boots, the `cap=` boots, the six faults, the mutation build, the litmus job
  and the seed sweep are the floor. An epoch that adds a power-cut sweep is the
  epoch that would be tempted to argue the older sweeps have become redundant.
- **The request names a method as well as an epoch.** `docs/sdlc.md` says which
  stage an agent may close on its own, and `intent/README.md` says an agent
  does not write its own spec. This document is an agent's rendering of a
  one-line request, in the shape `intent/0005` set; it goes back to the
  originator before a `spec.md` exists, and `TODO.md` is not edited by anything
  that is not a person closing a task against its written exit.

## Open questions

- **Where does the blob store crate live, and what is it called?** `user/store`
  already exists and is package `f-store`: *the component a place holds —
  spawned from a manifest, killed, and spawned again*, the chaos occupant that
  claim 0005 kills three times per run. So the obvious name is taken, by the
  thing in the tree least like a store, and every manifest, boot-log fixture
  and claim workload names it. Rename the occupant and re-baseline the
  fixtures, or name the store something that is not `store`? And is it a
  component with a ring on both sides, or a `no_std` library the blk driver
  links — which is the same question as whether `E2-B03`'s *a query returns a
  hash without crossing a component boundary* has one boundary in it or two?
- **What is the evaluator's expression language, and how small can it be?**
  `E2-B04`'s exit asks only that the same expression yield the same root on two
  machines. Nix is a lazy functional language with two decades of evaluator
  behind it and `lineage-and-debts` already records *evaluation is slow* as its
  debt. The smallest thing that meets the exit is not a language at all: a tree
  of fixed-width records, compiled rather than parsed the way RFC 0030 compiles
  a manifest, hashed in one order. The largest is a language. Somewhere in
  between is a question this epoch has to answer on purpose, and the answer
  decides a second question: whether evaluation happens on the host at build
  time — in which case the machine never evaluates anything and only receives a
  root — or on the machine, in which case `E2-B07`'s identity includes an
  evaluator that has to be trusted.
- **Does the mutable-extent kind land in 0.3?** `E2-D02` designs it, `E2-B09`
  builds it, and `E2-R01` now lists `E2-B09` among its blockers, so the graph
  says yes. The design page calls it a real complication, and the number it
  produces is the one the skeptic reaches for first. If it slips, does 0.3 ship
  with the bad-workload number measured on the pure path only and the extent
  path recorded as absent — the honest and weaker shape `intent/0005` asked
  about for the doorbell — or does the release wait? That is a choice for the
  originator rather than a discovery for whoever gets there last.
- **Where does the assembler run?** Section 07 of `deadline-all-the-way-down`
  has the frame start one component, the assembler, and everything else follow
  from the root it is handed. `E1-B05` hit a wall on the way to that: a ring-3
  supervisor could not drive a control ring without `unsafe`, so the restart
  policy runs in the frame against RFC 0008, and `cargo xtask lint-owed`
  reports it unpaid. RFC 0037 has since removed the wall — a component adopts
  a channel for a call in safe code, and `E1-B08` closed on it — but the move
  itself has not happened, and `E1-B08`'s own line calls what remains *work,
  not a wall*. `E2-B05` needs `E1-B05`, which is `[>]`. Either the assembler
  waits for the policy to leave the frame and is written as the parent of a
  supervisor that lives at ring 3, or it lands in the frame beside the policy
  as a second owed reversal and both move later. The first is the design; the
  second is how E1 actually went. The graph does not choose between them.
- **What is a quiescent point under sustained load?** `E2-B06` swaps routing
  *at a quiescent point* and `E2-P08` forbids a dropped operation. Claim 0005's
  kill is allowed to discard the writes in flight and read back afterwards; a
  swap is not allowed to discard anything. Whether quiescence is a property the
  ring already gives — the cursors of RFC 0018, a producer that has stopped and
  a consumer that has caught up — or something each component has to declare in
  `E2-D04`'s schema decides whether the transfer protocol is a few words or a
  state machine, and whether the driver from E1 can declare one without being
  rewritten.
