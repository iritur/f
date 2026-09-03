# RFC 0032: The simulator runs the components, not the instructions

- Status: accepted
- Date: 2026-09-03
- Affects: `sim/`, `xtask`, `docs/design/proving-ground.html` layer 1,
  `docs/TESTING-STATUS.md` L1, RFC 0004, E1-P01, E1-P02, E1-P03, E1-P08,
  E1-R01

## Decision

The deterministic simulator is a host executable model of the system **above the
frame** — components, the rings between them, and the devices at the far end of
those rings — driven by a seeded environment. It is the new crate `f-sim`
(`sim/`). It does not build, link or execute any part of `kernel/`, and it does
not contain a second implementation of anything the frame implements.

The exit criterion for E1-P01 says *a whole boot-to-workload run executes under
simulation and reproduces byte-identically from `(seed, commit)`*. Under this
decision that sentence is answered by **two commands and one seam, all three
stated rather than left to be inferred**:

- `cargo xtask trace --hash` boots the real kernel in QEMU and prints one hash.
  That is the **boot** half, it already exists (E0-P02), and it is the only
  evidence about the frame that this project has or plans to have.
- `cargo xtask sim` runs each scenario in the simulator and prints one hash per
  scenario. That is the **workload** half.
- The seam between them is the component manifests. The frame's boot ends by
  spawning components from compiled manifest records (RFC 0030), and a scenario
  is where the same component set is picked up. **That join is not built.**
  Today a scenario is a table of integers and reads no manifest, so the seam is
  a stated location rather than a shared artefact, and closing it is the third
  stage of E1-P01. What this RFC settles now is *where* the seam is, so that the
  remaining work is a scenario source rather than a change of shape — and so
  that nobody reads "boot-to-workload" as a claim about one process.

Both halves use one hash function, one printed form (`{:#018x}`), and one
default seed. The two copies of the hash function are held together by a fixture
that both sides assert against — `the_digest_is_the_one_xtask_hashes_boot_logs_with`
in `sim/src/trace.rs` and `the_sim_digest_is_the_one_this_file_hashes_boot_logs_with`
in `xtask/src/main.rs`, over one string against one constant.

## Context

`kernel/Cargo.toml` sets `test = false` on the kernel binary and says why: the
kernel is `no_std` with its own panic handler, a host test harness links `std`,
and two crates would then claim the `panic_impl` lang item. So "a whole
boot-to-workload run under simulation" cannot mean "run `kernel/src/main.rs` in a
host process" without a decision that reverses that comment. Three shapes were
live.

**(a) Simulate above the frame** — what this RFC decides. Cheapest, and its
honest cost is that the word *boot* in the exit criterion is answered by a
different command than the word *workload*.

**(b) Make the frame's logic host-buildable.** The parts of `kernel/` that are
algorithms rather than instructions — the capability table, the buddy allocator,
the ring service, the component lifecycle — move behind a `cfg` or a feature, or
into a separate `no_std` library crate both the kernel and the simulator link, so
they compile for the host against a mocked architecture layer.

Rejected, for three reasons of increasing weight.

The first is the Cargo.toml comment, which is a real build fact and not a
preference. Working around it is a crate split, and a crate split is a change to
the frame's shape made for the benefit of a test harness.

The second is that a mocked architecture layer is a second machine, and nothing
checks it against the first. The paging code, the APIC, the multiboot map and the
VT-d unit are the parts of the kernel most likely to be wrong, and they are
exactly the parts a mock replaces. A green simulation of a kernel running on a
machine nobody built is the shape of evidence this project exists to refuse.

The third is that the tree has already made this argument once, in E0-P09's
closing note about fault classes: *writing them here would mean writing a second
capability table to inject into, and a test of a model of the system is not a
test of the system*. That sentence was written about one subsystem. It
generalises, and generalising it is what this RFC does.

**(c) Model the machine** — a virtual machine monitor in Rust. Rejected before
the work started, and RFC 0031 is why it stays rejected: the machine this project
targets is already pinned to one QEMU configuration (`q35` with `intel-iommu`),
which means the machine is already modelled, by somebody else, more completely
than this project could afford to. A second one would be a larger project than
the kernel.

What made (a) the right answer rather than merely the cheap one is that the four
things the simulator has to contain are all above the frame. Virtual time is the
model's. Seeded ordering is about which component acts first. Device models are
device *protocols* as a client sees them. Component substitution is a property of
clients. None of the four needs the frame's instructions to be executing, and the
one thing that does — *does the frame itself reproduce* — already has an answer
that runs the real code on the real emulator.

## Consequences

**The simulator will never catch a bug inside the frame's own algorithms.** This
is the cost, and it is not small. A defect in the buddy allocator's coalescing
pass, in the capability table's revocation walk, or in the page-table walker is
invisible from here. What covers those is stated so the gap has owners rather
than being a silence: the boot suite (`cargo xtask run`, `orders`, `user`, `cap`,
`iommu`), the mutation harness (`cargo xtask mutate`, RFC 0017), and the bounded
proofs at E1-P07 and E1-P12. A reader who wants to know what checks the frame
should look there and will find it, rather than looking here and finding a
simulator that appears to cover it.

**E1-P02's fault classes divide along the same seam, and the division is
visible.** Peer death mid-operation, torn doorbell, partial write and delayed
completion are protocol events between components and belong here. Allocation
failure and translation fault are frame events, and what the simulator models is
the *client-visible refusal* — a component asked for memory and was refused —
rather than the allocator that refused. That is the right level for E1-P02's own
exit, which asks for a *system response* that is asserted rather than observed:
the response is the component's, and the component is here.

**Two reproduction checks, and a person has to know which is which.** That is a
real cost of the seam and the reason it is written down in three places rather
than one: the crate documentation, `xtask`'s help, and this RFC. Mitigating it is
what the shared hash function, the shared printed form and the shared default
seed are for. A failure now names its half — `trace` red means the frame, `sim`
red means the model — which is more than a single merged check could have said.

**The simulator's `Env` cannot satisfy `f_env::contract::check`, and that is
recorded as a test rather than worked around.** `contract::clock` requires the
clock to advance while it draws values, and its own documentation gives the
reason it can require that: "a virtual clock advances by being used and a
hardware clock advances on its own". A discrete-event clock is a third kind. It
must not advance under draws, because the timeline sets it to each message's
instant and a clock that had run ahead would be moved *backwards* by the next
dispatch — the one thing the contract exists to forbid. So `World` fails the
contract's first property and holds the rest, and
`the_env_contract_assumes_a_clock_this_one_is_not` in `sim/src/scenario.rs`
asserts exactly that, so the day somebody teaches `contract::check` about a clock
its caller advances, a test tells them the simulator was waiting.

**Order within a channel is the ring's and order across channels is the
seed's.** The timeline groups everything due at one instant by the ordered pair
of sender and recipient and lets the seed choose only between those groups. That
is a modelling decision with teeth in both directions: a simulator free to
reorder one producer's submissions would find bugs that `f_ring`'s
single-producer discipline makes impossible, and one that fixed the order across
channels would explore a single interleaving while reporting that it explored
them all.

**A decision carries two names, and both are needed.** Its *ordinal* is its
position in this run, which is what E1-P08 re-enters at; its *site and
occurrence* name it across commits, which is what E1-P03 reports. A minimised
failure quotes both, because an ordinal shifts the moment a commit consults a new
decision site and a site-occurrence pair does not.

**A domain word is spent before anything needs it.** `decide::draw` keys the
seed by a *domain* before the site, and `domain::FAULTS` is reserved with nothing
drawing from it. Paying for it now is the same argument that put E1-B11 before
E1-P01: without it, E1-P02's first fault draw would key off a site label,
collide with the ordering draw at that site, and move every interleaving a
recorded seed had already selected — so every seed in the corpus would stop
reproducing its run, silently.

**`f-sim` is not in the AArch64 cross-check**, because that check uses
`aarch64-unknown-none`, which has no `std`. Nothing in this crate is compiled
into the system, so nothing in it can be wrong on a machine the system runs on.

**What is foreclosed.** Anything that requires the frame's own instructions to be
under the simulator's control: single-stepping the kernel, injecting a fault
inside a page-table walk, or checking a capability invariant against the real
table under an adversarial interleaving. The last of those is the one worth
naming, because it is genuinely attractive and it is E1-P07's rather than this
crate's.

## What would reverse this

**A bug class in the frame's own logic that only an adversarial interleaving
reveals, and that neither the boot suite nor the model checkers can reach.** The
concrete shape: a capability or allocator defect found on hardware or in a
long-running boot, which `cargo xtask cap`, `cargo xtask mutate` and a Kani
harness all fail to reproduce, and whose reproduction requires choosing the order
of two operations inside the kernel. One such bug is an anecdote; two are a
measurement, and two would say the boot suite's ordering coverage is the gap
rather than its property coverage.

The reversal is shape (b) and it has a stated form, so that it is a move rather
than a rewrite: a `no_std` library crate holding the frame's algorithms, linked
by both `kernel/` and `sim/`, with the architecture layer behind a trait the
kernel implements with instructions and the simulator implements with a model.
`kernel/Cargo.toml`'s `test = false` comment then changes, and it changes for a
reason somebody measured rather than because a harness wanted it.

A second, weaker reversal: if the seam turns out to be where failures actually
live — if a run reproduces on both sides of it and the system still behaves
differently — then the seam is in the wrong place, and the evidence for that is a
failure that neither check can see. That is the observation worth watching for,
and it is the reason the seam is placed at an object both halves can be made to
read rather than at a sentence in a document. It is also the reason the join
being unbuilt is stated in the decision above rather than in a footnote: an
unbuilt seam cannot be shown to be in the wrong place.
