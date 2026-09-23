# RFC 0108: A declaration most components leave empty is a section of the component file, not a field of the record

- Status: accepted
- Date: 2026-09-23
- Affects: `abi/src/manifest.rs`, `xtask/src/manifest.rs`, `docs/manifest.md`,
  and RFC 0030's pattern

## Decision

`[[face]]` is declared in a **section of the component file after the image**,
addressed the way the image is, and **not** as a fixed-width slot array inside
`Record`. `Record` is still 2 696 bytes and every pinned offset is unmoved.

This reverses a pattern four schemas deep. Since RFC 0030 every declaration a
manifest carries has been a slot array in `Record` — `[[capability]]`,
`[[ring]]`, `[transfer]`, `[[device]]`, `[[state]]` — and `[[face]]` is the
first that is not. The rule this entry sets, for the next person adding one:
**a declaration most components leave empty does not belong in the record.**

## Context

The first attempt put it in `Record`, met both of `E3-B03b`'s exit clauses under
mutation, and **made every boot in the tree fail**. Four sixty-four-byte entries
and a count grew `Record` from 2 696 to 2 952 bytes.

`Record` is not just a type. It is **a bound on somebody else's stack**:
`f_assembler::topology::Instance` owns one by value and `Record::read_unaligned`
returns one, so the assembler builds a temporary the size of this type per
member of a generation. Marginal at 2 696; over the guard page at 2 952.

What a reader saw was none of that. The supervisor's occupant took `exception 14
... error 0x6` the instant it was scheduled, the boot reported *told of 0
death(s); decided leave; submitted 0 spawn(s)*, and the printed failure was `the
component lifecycle: a spawn named a place it may not occupy` — which reads as a
restart policy answering `never` off a manifest that says `on_fault`. Three
subsystems from its cause.

**The repair the symptom suggests is the one the tree refuses.**
`kernel/src/process.rs`'s `SPAWN_STACK_PAGES` says: *a guard page that fires is
evidence; the first question it deserves is what is the faulting instruction,
and not how much more stack would make this stop* — and raising it is a
cross-crate move pinned by three compile-time assertions. Raising the
supervisor's heap does not touch it either; the fault address is identical at
64 KiB and 80 KiB.

So the question became *where a declaration lives*, and the answer is that a
record every component pays for should carry what every component has. A face
list is empty for eight of the nine components in this tree.

The alternatives, weighed rather than assumed: one face instead of four is a
smaller record and a real restriction to argue, and still grows a type nobody
should be growing; making `Instance` stop owning a `Record` by value fixes the
assembler and leaves the growth, which is a change to somebody else's crate to
buy room in this one.

## Consequences

**What this makes easy.** A declaration whose size is a property of the
component rather than of the schema. A component with forty faces costs forty
faces; a component with none costs nothing.

**What this makes hard.** Reading a declaration now means reading the component
file rather than the record, so a consumer needs the file and not just the
record it parsed out of one. That is the cost, it is real, and it is the same
shape as the image itself — which is the precedent this follows rather than
invents.

**What it does not change.** `Record` is 2 696 bytes and the offsets three other
things pin are where they were. `cargo xtask run` reaches `M0 ok`.

## What would reverse this

**A declaration most components *do* fill.** The rule is about emptiness, not
about faces: a field every component carries belongs in the record, where its
cost is honest and its offset is pinned. If `[[face]]` becomes that — every
component declaring a typeface — this entry is wrong and the field should move
back.

**`Instance` ceasing to own a `Record` by value.** That removes the stack bound
which is most of this argument's force. The growth would still be real and still
paid by every component, so the rule would stand on economy rather than on a
guard page — a weaker argument, and worth re-reading rather than assuming it
survives.
