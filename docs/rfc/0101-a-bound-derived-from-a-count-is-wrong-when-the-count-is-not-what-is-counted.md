# RFC 0101: A bound derived from a count is wrong the moment the count is not what is counted

- Status: accepted
- Date: 2026-09-22
- Affects: `abi/src/reserve.rs` (`RESERVATIONS_MAX`), `kernel/src/component.rs`
  (`PLACES_MAX`), `kernel/src/main.rs` (`MAX_RESERVED`)

## Decision

`RESERVATIONS_MAX` is sixteen and was eight. Eight was derived — *the frame's
own place count*, on the argument that a reservation belongs to a component and
a component occupies a place, so a table larger than the places would hold
entries nothing could have been granted through.

**The argument was sound and the number was still wrong, because the thing
counted is not the thing bounded.** `kernel::component::fill` puts the grant on
the place *after* the spawn, so a place that is torn down and refilled from the
same supply holds two entries for one place until the first is released. The
bound is therefore places **plus refills in flight**, and *places* was only ever
an underestimate that happened to hold.

The general rule this entry is named for: **a constant derived from a count is
only as good as the claim that the count is what is being counted, and that
claim decays silently.** It decays by addition — one more component — and the
failure surfaces nowhere near the addition.

## Context

`E3-B06c` added `user/panel`, the eighth component file. The boot failed with
`the component lifecycle: the spawn was refused admission before anything was
spent`, **on a component unrelated to the one that was added**. At seven
components the table was exactly full; the eighth overflowed it; the overflow
was reported correctly as a full table and read three subsystems away as an
admission refusal.

That distance is the finding. Nothing was wrong with the refusal, the reporting
or the arithmetic — the bound was one short and had been one short for as long
as a boot has refilled a place, and no boot had needed the entry until the
eighth component existed.

**A second bound of the same species was found and repaired in the same hour**,
which is what makes this a rule rather than an incident. `kernel/src/main.rs`'s
`MAX_RESERVED` was the literal `13` — three fixed ranges, one exclusion, and one
per module — chosen against the modules a boot *had* rather than against
`multiboot::MAX_MODULES`, which decides how many it *may* keep.
`reserved_ranges` stops at the bound and says nothing. Adding a component pushed
the eleventh module past the end and the frame allocator handed out the
generation module's memory; the symptom was `no module this machine was offered
folds to the root it was asked for`, with the address space's own root sitting
at the address that module had been loaded at. It is now
`5 + arch::x86_64::multiboot::MAX_MODULES`.

Two bounds, two different subsystems, one shape: a number written against what a
boot contained on the day it was written.

## Consequences

**What this makes easy.** Adding a component. Both bounds now grow with the
thing that actually decides them, so the next component costs a `COMPONENTS` row
and nothing else.

**What this makes hard, and it is the residue.** `abi` cannot see
`kernel::component::PLACES_MAX`. The frame holds the places; `abi` holds the
table both sides read; and the relation between them is written twice with
**nothing checking one against the other**. That is the real defect underneath
this one, and raising the constant does not close it — it buys slack. A boot
that refills more places than the surplus covers fails the same way, at a
distance, for the same reason.

**What was refused.** Two repairs other than the constant were live. Releasing a
place's grant at teardown so a refill never holds two would make the derivation
true again and is the principled fix; it changes the teardown path, which is
`E2-B06`'s ground and not this one's. RFC 0029's bought table removes the bound
entirely and is the long answer. Both are better than sixteen and both are
larger than the change that was needed to stop reporting an unrelated
component's spawn as refused.

## What would reverse this

**A boot that refills more places than the surplus covers.** Sixteen is eight
places and eight refills in flight; a chaos run with a deeper refill schedule,
or a supervisor that batches teardowns, reaches it. The symptom will be the same
one and just as far from its cause, which is the argument for the lint below
rather than for a larger number.

**The check that would close the residue rather than pad it:** a policy check
that reads `PLACES_MAX` out of `kernel/src/component.rs` and `RESERVATIONS_MAX`
out of `abi/src/reserve.rs` and refuses the pair when the second is not at least
twice the first. It is a source read of two constants in two crates — the shape
`lint-percpu` and `lint-stamp` already have — and it turns *this decays
silently* into *this decays at the next `cargo xtask lint`*. It is not written
here because a rule deserves its own task rather than a paragraph in the RFC
that motivates it.
