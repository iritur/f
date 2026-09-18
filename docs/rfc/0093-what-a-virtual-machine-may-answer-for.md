# RFC 0093: what a virtual machine may answer for

- Status: accepted
- Date: 2026-09-18
- Affects: `TODO.md` E0–E2, `docs/TECHNICAL-DEBT.md`, `claims/README.md`,
  `claims/runner-class-A.md`, `docs/TESTING-STATUS.md`, `xtask`'s `lint-debt`

## Decision

Epochs E0 through E2 close against **what a virtual machine can honestly
establish**, and no further. Where an exit clause names a measurement that no
machine this project can reach may take, that clause is **narrowed** — struck
from the exit and moved, in the same diff, to a row in `docs/TECHNICAL-DEBT.md`
naming the blocker, what the virtual machine did establish in its place, and
what would close it. A task that still has a half worth closing on is then
marked `[x]`, and a task that was nothing but the measurement is marked `[~]`
— see *What a narrowed task is marked*, below.

Four things a virtual machine may **not** answer for, and a clause naming any
of them is a debt rather than a gap:

1. **A time.** `bench/src/lib.rs` refuses to record a measurement where
   `F_ENVIRONMENT` is not a recording class, and that refusal is the harness
   working rather than a feature missing. Fifteen claims are `pending` on it.
2. **A reservation obtained by partition.** `claims/runner-class-A.md`
   requires all four of RFC 0007's components by partition, which needs Intel
   RDT's CAT and MBA. No guest exposes a resctrl partition, and checklist item
   7 of that file is *no hypervisor flag in `/proc/cpuinfo`*.
3. **An instruction the emulator does not implement.** QEMU's TCG backend
   implements no part of Intel UINTR and no `-cpu` model advertises the bit, so
   `Bell::new`'s refusal of `Path::UserInterrupt` is the whole of what can be
   exercised.
4. **A second machine, or a second person.** One machine twice is not two
   machines, and a stranger reproducing a claim from the README is an
   observation about somebody else's afternoon.

Everything else — every count, every refusal, every boot to `M0 ok`, every
byte-identical reproduction — a virtual machine answers exactly as well as
silicon does, because the answer is a property of the program.

## Context

Thirty-six tasks stood open across E0–E2. Very few were half-built: the common
shape was a task whose code was finished, whose tests passed, and whose exit
line named a number that needs a machine nobody has bought. `E0-B12` is the
clearest instance — the ring is built, the workload runs, the distribution is
drawn, and the exit quotes `E0-P05`'s exit word for word, so one criterion
belongs to two tasks and one of them is always lying about its state.

Three things made the status quo worse than it looks.

**An exit nobody can meet stops being read.** This file records the fate of a
gate with no path to green twice already; `E0-B21` declined to make `unsafe`
percentage gating for exactly that reason, and said so on the verb. A graph
where a quarter of the nodes are blocked on a purchase order is a graph whose
`[>]` marker means *ask somebody*, which is the state `cargo xtask todo` exists
to remove.

**The registry cannot presently tell the two kinds of waiting apart.** All
fifteen `pending` claims name `runner-class-A` in `[hardware] runner`,
including `claims/0022 copies-per-read`, whose own note says in as many words
that *"the machine it does need is not a runner class: it is a boot of
`user/objects` as a component over the blk driver's ring"*. That is a task in
this tree. A number waiting on a commit and a number waiting on a purchase look
identical in the files, and the first gets treated as the second.

**Prose kept by attention rots, and had.** `docs/TESTING-STATUS.md` published
*"Built, fifteen entries, six gating"* against a registry holding thirty-three
and seventeen, in runs whose own output counted the claims correctly four lines
above. Any register written as a paragraph somebody updates when they remember
will arrive at the same place. This is the argument for the register being tied
to data and checked, not for it being unnecessary.

The live alternatives were to leave the tasks open, and to mark them `[x]`
without narrowing anything. The first is what has been happening. The second is
the scope cut this project's `A-07` exists to forbid — a criterion quietly
dropped is indistinguishable, six months later, from a criterion that was met.

## What a narrowed task is marked

Two shapes turn up, and they are not the same thing.

**A task with a half a virtual machine can answer** loses the clause it cannot
meet and is marked `[x]`. `E0-B12` is the type: the ring is built, the workload
runs, the distribution is drawn, and one clause — *under 50 ns per operation,
recorded as a gating claim* — goes to the register. What remains is met, and
saying so is accurate.

**A task that is nothing but the measurement** has no half to close on.
`E0-P05` is *claim 0001 moves from pending to gating*, and its whole exit is
`cargo xtask claims` reporting green. Narrowing it to nothing and marking it
`[x]` would be recording a task as done having done none of it — the exact
dishonesty this RFC claims to be guarding against, committed by the RFC itself
on its first application.

So those are marked **`[~]`**, whose meaning this RFC widens. `TODO.md`'s legend
reads *`[~]` dropped*, and the file explains it: *"Dropped tasks are marked
`[~]` with a one-line reason and stay in place; they are not deleted, because
the reason is the useful part."* A task owed to a machine nobody has bought is
not dropped, and the honest word is **owed**. The mechanism is already right —
the ID is permanent, the entry stays in place, the reason is what a reader
wants, and `cargo xtask todo` already stops treating it as a blocker — so what
changes is the legend and not the machinery.

`TODO.md`'s status line therefore becomes *`[~]` dropped or owed*, with the
one-line reason distinguishing them: a dropped task says why nobody should do
it, and an owed task names the register row and the machine.

The distinction matters more than it looks. A reader scanning for what is left
in E0 should see `E0-P05` as *waiting on silicon*, not as *finished* and not as
*abandoned*, and none of `[ ]`, `[x]` or a deleted line says that.

## Consequences

**A narrowing is a reversal and is written as one.** RFC 0084 settled that an
exit is something already written down, so each task narrowed under this RFC
cites it in the body, beside the clause it lost. The register row is the other
half: the clause is not deleted, it is moved somewhere a reader can find every
one of them at once.

**The register is checked rather than remembered.** `cargo xtask lint-debt`
compares two sets: the tasks whose `TODO.md` entry cites this RFC, and the rows
in the register. A narrowing with no row is a clause deleted rather than moved,
which is the one thing this RFC rests on; a row naming no narrowing is a debt
nobody owes, or a clause that has been answered and whose row should have gone
in the diff that answered it.

Both fixes are edits to `docs/TECHNICAL-DEBT.md`, and that is deliberate rather
than convenient: `Gap`'s own documentation warns that a check satisfiable only
by editing a file this tree's agents may not touch is a check that gets
switched off, and half of `TODO.md` is exactly that file.

What it does not check is whether the clause in the row is the clause the task
actually lost. Nothing short of a reader can do that, and putting a tick beside
it would be claiming coverage over the one part that needs a person.

**This makes a whole epoch closable and makes one kind of dishonesty easier.**
The risk is real and worth stating plainly: a task narrowed under this RFC is a
task somebody decided was hard to measure. The defence is that the narrowing is
in the diff, the clause is in the register, and the register is checked — none
of which stops somebody narrowing an exit that a virtual machine could in fact
have answered. What stops that is review, and review is what the four-item list
in the Decision exists to make possible: a clause that is not a time, a
partition, an unimplemented instruction or a second machine has no business in
the register, and a reviewer can say so without knowing the subsystem.

**A virtual machine boot is evidence, not a rubber stamp.** Three boots outside
QEMU are recorded — `docs/first-`, `second-` and `third-boot-outside-qemu.md` —
and on the same day as the third, a VMware guest on a Threadripper 2990WX host
stopped inside core bring-up and never reached `M0 ok`. RFC 0068 is that
record. Two hypervisors disagreeing about the same image is exactly the class of
finding the emulator cannot produce, so the guest boots are not a formality
being waved through.

**What this does not touch.** No claim's status changes, no threshold moves and
nothing becomes `gating` that was not. The fifteen `pending` claims stay
`pending`, because this RFC is about what a *task* may close against, not about
what a *number* may be recorded from. `bench/src/lib.rs` refuses exactly what it
refused before.

## What would reverse this

**A class-A machine existing.** This RFC is scaffolding around an absence. The
day `claims/runner-class-A.md`'s seven-item checklist passes on a machine
somebody can run a job on, every row whose blocker is that machine comes out of
the register, the exits that lost clauses to it get them back, and the tasks
reopen at the clause they were narrowed at rather than from the beginning —
which is what the register storing the clause verbatim is for.

**The register growing past what a reader will read.** It has one row per
blocked clause today. If it reaches a size where nobody opens it, it has become
the queue it was meant to replace, and the answer is not a longer page: it is
that E0–E2 were closed too early and the narrowings should be re-argued one at
a time.

**A clause turning out to be answerable after all.** `E2-B02` and `E2-P10` are
the live candidates and are deliberately *not* in the register on hardware
grounds: zoned virtio-blk emulation needs QEMU 8, which is a change to the
development image and not a machine. Anything that reaches the register and is
later shown to have been buildable is a mistake this RFC made, and the reversal
is to build it and delete the row, with a note here saying which one it was.
