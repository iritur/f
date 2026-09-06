# RFC 0012: An update is a generation swap, and the root is the attestation

- Status: accepted
- Date: 2026-09-06
- Affects: `docs/design/fast-path.html` (the rollback row of the metrics table
  and the two other places "one-boot rollback" appears),
  `docs/design/lineage-and-debts.html`'s storage row,
  `docs/design/deadline-all-the-way-down.html` section 04,
  `docs/what-must-be-stated.html` (the R12 row and the two rows that already
  quote "full system rollback: one reboot"), `abi/src/manifest.rs`'s
  `ContentId`, `abi/src/store.rs`, `kernel/linker.ld`'s exported boundaries,
  the frame's boot path and its published tree; RFC 0008, RFC 0013, RFC 0018,
  RFC 0030, RFC 0041, RFC 0060; and `E2-D01`, `E2-D04`, `E2-B04`, `E2-B05`,
  `E2-B06`, `E2-B07`, `E2-P07`, `E2-P08`, `E2-R01`

## Decision

An update is a generation swap and not a boot. A generation is a closed tree of
records with one thirty-two-byte SHA-256 root; the running system is whichever
generation the frame publishes; and moving from one generation to the next is
performed place by place under RFC 0041's protocol, with a reboot required in
exactly one case — the incoming root record's `frame` field differs from the
running one's. Everything else is swapped while the machine keeps running,
including a change to every component in the topology and every route between
them. The root is what the machine answers *what are you running* with, and it
is an attestation in one narrow sense only: it is a hash over every artefact the
machine was assembled from and over nothing else. It carries no signature, names
no verifier, proves nothing about a frame already modified to lie about itself,
and says nothing about mutable data. The rollback metric therefore stops being
*one reboot* and becomes **one generation swap, and a reboot only when the frame
changed** — two counts a demonstration records, with `reboots` required to equal
the frame-changed predicate exactly, so that the sentence fails in an artefact
rather than in an argument.

## What the words mean, mechanically

### Generation

The record tree `cargo xtask generation` compiles from canonical
`generation.toml` (`E2-B04`), folded post-order under SHA-256. Its `generation`
node has exactly two children: the frame hash and the topology hash. A
generation is a value and not a process — it exists before it runs, it can be
compiled on a machine that will never run it, and two machines compiling the
same source produce the same generation or one of them has a non-reproducible
input, which is `E2-P06`'s whole job. Generations are ordered by the
`generation` counter in the root record: publish order, one-based, zero
reserved. There is no timestamp anywhere in this, and RFC 0004 is why.

### Root

Two things, and conflating them is how *atomicity is free because a root is a
single write* came to be written. The **root** is the thirty-two bytes the fold
produces; it identifies. The **root record** is the fixed-width record appended
to a root zone — `generation`, `root`, `frame`, `module`, `previous`, `check` —
which durably orders, and which RFC 0060's barrier sequence exists to make
meaningful.

`frame` is a duplicate of a leaf that already sits under `root`. It is
duplicated deliberately, so that a swap can decide whether it needs a reboot by
comparing thirty-two bytes without resolving a tree, at the moment when the tree
may not be resolvable yet. It cannot drift: the fold covers it, so a record
whose `frame` disagrees with its own generation node fails resolution and is
refused at mount — the same refusal that catches a lying device.

`module` is the hash of the boot module packed for this root when the generation
was compiled, and not the module this root was booted from. A root that arrives
by swap is never booted, and a root with no module is a root no reboot can
reach; case 2 below depends on every published root having one, so the packing
happens at compile time for every generation whether or not any boot uses it.

### Frame, and what "the frame changed" is a hash over

The frame hash is SHA-256 over two named ranges of the linked kernel image, in
this order: `__text_start .. __text_end`, then `__rodata_start .. __rodata_end`.
Both boundaries are exported by `kernel/linker.ld` and both ranges are
page-aligned there already, because the mapper needs them to be; both are
immutable after load. The build side extracts those two ranges from the ELF; the
boot side recomputes over the same two ranges of itself, after the mapper has
made them read-only and not writable and before anything is published.
Disagreement is a refusal to publish a root, not a warning line.

What is deliberately *not* in the hash, stated so that nobody reads more into it
than it holds: `.boot` — unreachable after the address-space switch, so covering
it would mean hashing before the mapper exists — `.data` and `.got`
initialisers, `.bss`, `.stacks`, the multiboot command line including `f.root=`,
the boot module set, the loader and the firmware. A frame change confined to a
writable initial value is invisible here. That is why `cargo xtask mutate`'s
frame mutation must land in text or rodata, and it is one of the reversals
below.

**"The frame changed" means those two thirty-two-byte fields differ.** So a
reboot is required in exactly these cases and no others:

1. `incoming.frame == running.frame` — no reboot. Every component leaf that
   differs is a place-level swap; every route that differs is a routing change
   the assembler applies at the same quiescent points; the new root is published
   when the last place has swapped and its root record is durable.
2. `incoming.frame != running.frame` — reboot, selected with `f.root=<64 hex>`
   naming the module packed for the incoming root.

There is no third case, and in particular the assembler is inside the topology
rather than part of the frame: if its own leaf moves, that is case 1 and it is
swapped at its place like anything else. The alternative was a frame with a
component-shaped hole in it and a reboot condition with a list. If `E2-B06`
finds a class of component that cannot be swapped, the repair is to make it
swappable, or to give it a field of its own beside `frame` — and either is an
RFC, because "a reboot only when the frame changed" stops being true the moment
the condition has an *and* in it.

### Swap

Per place, under RFC 0041 and RFC 0008: the place stops delivering, client
submissions pend exactly as RFC 0008's connect pends against an empty place, the
old occupant drains to the quiescent point RFC 0018's cursors define, it writes
its `f_abi::transfer` records (`E2-D04`) — or it declared `restart_only`, which
is a restart at that place and is counted as one — the new occupant
acknowledges, the routing word swaps under one `Release` store and one `Acquire`
load, and the old occupant is retired. Nothing is discarded, which is the word
that separates this from claim 0005's kill.

A generation swap is the ordered set of those, plus the route changes the
topology declares, and it is **atomic at a place and not at the machine**. There
is an interval during which some places run the old occupants and others the
new. What is atomic machine-wide is the *published root*, and it is published
only after the last place has swapped. During the interval the frame publishes
the generation counter as **zero** — the value the format already reserves so
that a zeroed block is never a generation — and a reader that sees zero knows
that no root describes this machine at that instant.

That is also what makes a thirty-two-byte root publishable through RFC 0013's
tree at all. A node there is a machine word and a snapshot is atomic per node
and not across the tree; a root is four words. So the read is: load the counter;
if it is zero, a swap is in progress and there is no answer; otherwise load the
four root words and the four frame words, load the counter again, and accept
only if it did not change. The writer stores zero before the first place swaps
and the new counter after the last root record is durable. One extra word buys a
tree that can be read *during* an update rather than only when nothing is
happening.

### The rollback metric, as counts something records

`cargo xtask rollback` and `cargo xtask swap` each record, per run:
`generation_swaps` (completed swaps, a swap being complete when its root record
is durable), `reboots`, `frame_changed` (zero or one, the thirty-two-byte
compare), `places_swapped` and `places_restarted`, and `operations_dropped` and
`operations_redone`. The assertion the metric makes is
`generation_swaps == 1 && reboots == frame_changed`, with both operation counts
zero.

`places_restarted` is separate from `places_swapped` and not summed with it,
because a "swap" in which every place declared `restart_only` is a reboot in
instalments, and the artefact has to show that rather than report a swap.
Numbers reaching `docs/design/` need claims, so the day the rollback row carries
a measured count rather than a target it is registered beside claim 0021. This
RFC changes a sentence; it publishes no number.

### Two hash functions in one identity chain

The generation's `component` leaf is the SHA-256 of the component file — RFC
0030's record and image together, the same bytes `ContentId::of` runs over.
`abi::manifest::ContentId` stays FNV-1a over sixty-four bits, is not widened, is
not the low half of anything, and the generation does not carry it.

Two hash functions in one identity chain are acceptable exactly while each
covers a different hop with a different adversary, and the weaker one is
*dominated* on its own hop by the stronger one upstream. SHA-256 covers store to
boot module to assembler, where bytes can be supplied by somebody who wants a
particular outcome, and the assembler recomputes the fold over the module and
refuses a mismatch before anything is spawned. FNV-1a covers assembler to frame
at spawn, where the bytes were already checked against the root and the only
thing left to catch is accident: a truncated module, a mismatched record and
image, a place refilled from the wrong manifest. `ContentId`'s own doc comment
says it is honest about being nothing more, and states the condition that ends
it — a component file arriving from anywhere the boot loader did not put it.
This RFC's contribution is to say that E2 does not meet that condition, and why:
the store's bytes reach a place only through a module the assembler verified.

Truncating the SHA-256 into the sixty-four-bit id was the live alternative, and
it is refused for two reasons. It would make the chain one function on paper
while leaving the spawn check with sixty-four bits of preimage and thirty-two of
collision resistance — a weak check wearing a strong name, and the first reader
who notices "it is SHA-256 already" is the reader who deletes the full check
upstream as redundant. And it costs: `ContentId::of` is `const` and links into
`user/init`, which links as one library with no `core` beside it, so a `const`
SHA-256 there is pages of code inside every component image to answer a question
FNV-1a already answers.

The price of keeping them apart, stated rather than discovered: there is no
single identity to look a component up by. A sixty-four-bit id in a spawn log is
not a key into the store, and any tool going from a running occupant to the
bytes it came from goes through the assembler's table.

## What the attestation does not prove

To whom, first. To a person or a component reading RFC 0013's mapping on this
machine, and to a reviewer holding the release package who recompiles
`generation.toml` and compares the root to the one the machine publishes. To
nobody else: there is no key, no signature, no nonce and no freshness, so the
root means nothing over a network. Anyone who can produce the bytes can produce
the hash, and a machine can report a root it is not running.

Five things it does not prove, named here so that `E2-R01`'s attestation story
is a sentence and not a suggestion:

- **That the frame measuring itself is the frame the hash names.** A self-hash
  is a claim by the thing being measured. It catches a modified image booted
  honestly, which is exactly what `cargo xtask mutate`'s frame mutation
  exercises; it cannot catch an image modified to report the old digest. The
  residual closes with a measurement taken before the frame runs, which is a TPM
  and E5's hardware.
- **That memory still matches the image.** The recomputation happens once, at
  boot. A frame compromised at run time publishes the digest it computed before
  it was.
- **Freshness.** The same value read twice says nothing about the interval
  between the reads.
- **That the source somebody read compiled to these artefacts.** That is
  reproducibility and it is `E2-P06`'s; the root is the thing compared, not the
  evidence for it.
- **Anything about data.** The root names the frame and the topology. Bytes
  under it are named at a snapshot boundary and not between them (RFC 0058), and
  the command line, the `f.root=` selection and the module set are outside the
  hash entirely.

## Context

`TODO.md` reserved the number 0012 for this decision when the E2 line was
written, which is why this directory runs 0011, 0013. The gap has stood for two
epochs and this entry closes it.

What was true. `docs/design/fast-path.html` published *full system rollback: 1
reboot* as a target, and `docs/what-must-be-stated.html`'s R12 held that up as
the project's own example of a concession dressed as a target. Since then RFC
0041 made a place outlive its occupant, and E1 measured it: claims 0005 and 0006
kill a driver under sustained load, a new instance takes the place, and no
client observes a dropped operation. Once a place survives its occupant, the
reboot in that metric is not a limit — it is a policy nobody had revisited. E2
builds the two things needed to revisit it: a root that describes a whole system
(`E2-B04`, `E2-B05`) and a transfer protocol (`E2-D04`).

Alternatives that were live at the moment this was decided:

**Keep "one reboot".** It is honest, it is already published, and it costs
nothing to leave alone. Refused for the mirror image of the reason R12 states: a
concession that has stopped being true also hides something, and a metric that
understates what the machine can do is as much a misdescription as one that
overstates it. The cost of the refusal is recorded — the new metric is two
numbers where the old one was one, and one of the two is conditional.

**Reboot never: an updatable frame.** Live patching, or a second frame started
beside the first. Refused: the frame owns the address space every place lives
in, and there is no place above it to hold pends while it drains, so the
quiescent point this design rests on does not exist for the frame itself. A
live-patched frame would also have to attest to something it changed after it
measured it. A reboot on the frame's own change is a boundary expressible in one
comparison, and the frame is the one component whose restart cost this project
already measures.

**Decide the reboot by diffing two generation trees.** Walk both, reboot if
anything under the frame subtree moved. Refused: it requires resolving two trees
before knowing whether a swap is possible, at a moment when the device may hold
only two root records and the incoming one may not resolve at all — RFC 0060's
verify-before-accept is precisely the case where it does not. A duplicated field
turns the decision into a compare, and because the fold covers the duplicate it
cannot disagree without the root being refused anyway.

**Attest with a signature now.** Refused: without a measurement the frame cannot
influence, a signature over a self-computed digest adds a key and proves nothing
further. Naming the residual is worth more than shipping the key.

**One hash function, by truncation.** Refused above, and it is the alternative
most likely to be proposed again, because "two hash functions in one identity
chain" sounds like an oversight and reads as one until the two hops are named.

## Consequences

What this makes easy:

- *What are you running* is thirty-two bytes, and comparing two machines is
  comparing two numbers. `E2-P05`'s whole-system comparison and `E2-P06`'s
  two-machine job read the same value from opposite ends.
- The reboot decision is arithmetic on the incoming root record, available
  before the tree is resolved and before anything is instantiated.
- A rollback across a frame change is a boot argument and not a recovery
  procedure, because every published root has a module packed for it.
- Delta transfer between machines (E4) has something to name: the leaves are
  content addresses, so two generations differ by exactly the leaves that
  differ.

What this makes hard:

- Every component in a topology that intends to swap owes an `f_abi::transfer`
  declaration, and `restart_only` — the honest default — makes its place a
  restart. A generation swap over a topology of `restart_only` components is a
  reboot in instalments, and the artefact says so.
- The frame carries `f-hash` on its boot path and must recompute before anything
  could have modified text or rodata. If the frame ever gains boot-time text
  patching, the recomputation moves in front of it or the two fields disagree
  and no root is published at all.
- Publishing a four-word root through a per-node-atomic tree needs the counter
  bracket above. A reader that ignores it reads a torn root during a swap, and
  that is the one failure mode this decision creates in RFC 0013's tree.
- A conditional metric is harder to publish than an unconditional one. Every
  place the sentence appears has to carry the condition with it, which is why
  the condition is a field and not a paragraph.

What this forecloses:

- **Live frame update**, for as long as this stands. Anybody who wants it
  reverses this RFC first, which is the reason for writing it down rather than
  leaving the reboot as an unexamined habit.
- **Imperative update.** No path changes a running system without a generation
  compiled somewhere first, so there is no in-place repair on a machine with no
  host to compile on. A rescue tool would be a second reader of the on-disk
  format, which RFC 0060 already refuses.
- **Partial rollback.** Rolling one component back means compiling a generation
  that differs in one leaf and swapping to it. There is no way to move one place
  backwards and leave the root meaning anything.
- **A spawn identity that is also a store key**, which is what keeping the two
  hash functions apart costs.
- **Remote attestation on this root.** It is not made possible by this RFC and
  must not be described as though it were.

## What would reverse this

- **`cargo xtask rollback` recording `reboots == 1` with `frame_changed == 0`,
  or `generation_swaps > 1`, on any run.** The metric is then falsified by its
  own demonstration and the old sentence goes back.
- **Claim 0021's `generation-swap-pause` p99 at or above the same machine's
  measured pause for rebooting into the target generation.** Then a swap costs
  what the reboot it replaces cost, *one generation swap* is a worse sentence
  than *one reboot*, and `what-must-be-stated`'s own row for this claim gets its
  second outcome: live update is not reachable and the metric was honest after
  all. Both numbers come from verbs this epoch builds — `swap` and `rollback` —
  on one machine, so the comparison is a subtraction rather than an argument.
- **`E2-P08` recording a dropped or redone operation across a swap under
  sustained load**, with the place holding pends as designed. RFC 0041's place
  is then sufficient for a restart and not for a swap, and the difference is
  state transfer: `E2-D04` grows into a protocol and *swap* here is the wrong
  mechanism rather than an unfinished one.
- **A frame mutation that boots with the published `frame` unchanged.** If
  `cargo xtask mutate` gains a mutation in `.data`, `.got` or `.boot` that
  survives, the coverage is narrower than "the frame changed" claims, and either
  the artefact list widens — which means hashing before the address-space switch
  — or the sentence does.
- **A second condition appearing on the reboot side.** `E2-B06` finding a
  component class that cannot be swapped, the assembler being the candidate,
  turns this into "a reboot when the frame or X changed". Every such X needs its
  own field in the root record, and the second field is the observation that the
  frame was not the boundary this RFC claims.
- **A component file reaching a spawn on a path where no SHA-256 was verified**
  — a network fetch, a second stage, a rescue tool, or E4's delta transfer
  landing bytes into a place without going through a module the assembler
  checked. RFC 0030's `ContentId` reversal fires at that moment, because the
  dominance argument is about the paths that exist and not about the hash. An
  observed FNV-1a collision between two component files in one tree says the
  same thing sooner and more embarrassingly.
- **A consumer of the root appearing that needs freshness or a signature.** Then
  the root is a build identifier wearing an attestation's name, and the title of
  this RFC is the thing to change.
