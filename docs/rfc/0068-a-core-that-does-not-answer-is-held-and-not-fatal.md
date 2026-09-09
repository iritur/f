# RFC 0068: A core that does not answer is held, and not fatal

- Status: accepted
- Date: 2026-09-09
- Affects: `kernel/src/smp.rs`, `kernel/src/arch/x86_64/ap.rs`, `kernel/src/main.rs`,
  `xtask/src/main.rs` (`cargo xtask cores`), `.github/workflows/ci.yml`,
  `docs/booting-on-hardware.md`, `TODO.md` (`E0-P18`)

## Decision

A core that the boot processor starts and that does not arrive inside the
give-up bound is **reset, counted and stepped over**, and the boot carries on to
the next core. It used to end the boot. The count of held cores is reported in
the boot log as its own sentence, separate from the cores this kernel leaves
asleep because it has no shard for them, because the two absences have different
causes and only one of them is a decision this kernel made.

Two things move with it. The boot log states the census — how many logical
processors the processor reported, and how many of those bring-up is about to
touch — **before** the stage runs rather than after, because that stage is the
one place in the boot where the machine can die with nothing left in the log. And
a core given up on is sent an `INIT` with no startup interrupt behind it, so it
is held in wait-for-startup rather than merely assumed absent.

## Context

`smp::start` refused the boot on any core it started that never reached kernel
code, and said why: *a kernel that cannot start the cores it can see is running
on a machine it has misread, and the next thing it would do is hand a process to
a core that is not there.*

The second half of that was never true. `first_worker` chooses among cores whose
mailbox says `READY`, and a core that never arrived never wrote one; `started()`
counts the same way. The refusal protected nothing that was not already
protected by the mailbox, and it cost every machine whose processor reports more
logical processors than answer.

That is not an exotic machine. `logical_processors`'s own fallback describes it
in the file — *the maximum addressable ids per package, which is a power of two
that is at least the count and may be more* — and goes on to say that *a core
that is not really there simply never arrives, which is what the give-up bound in
`start` is for*. The give-up bound existed; the caller made reaching it fatal. A
callee's doc comment and its one caller had disagreed since the file was written,
and nothing in this tree could see it, because QEMU is told two cores and has
two.

What made it visible was a machine. On 2026-09-09 this kernel was booted on a
VMware guest on a Threadripper 2990WX host. The log stops after `env contract`
and the guest's virtual processor entered the shutdown state — a triple fault —
with nothing else printed. The three differences the log did show against the
machine that works are all one difference: `pcid unavailable`, six local-APIC
LVT entries rather than seven, and `timer via apic one-shot` rather than
`tsc-deadline` are what an AMD host looks like through VMware's CPUID, and
`-cpu EPYC` reproduces all three under QEMU and boots green. So the timer
backend is not the fault, and neither is anything else the log named.

What is left is that the only machine-dependent inputs to that stage are the
boot processor's own apic id, the timestamp counter's rate, and the number
`logical_processors` returns. The first two were printed and are ordinary. This
RFC is about the third, and about the fact that the stage which consumes it is
the stage that cannot report.

**This is not a claim that the Threadripper's boot is explained.** It is a claim
that the boot could not have survived a processor that over-reports, that it
could not have said so if that is what happened, and that both of those are
defects on their own. The next boot on that machine will say what the processor
reported before it decides anything, which is the evidence that does not exist
today.

## Consequences

Easy: a machine whose apic ids are sparse, or whose hypervisor rounds a topology
up to a power of two, boots on the cores it has instead of refusing. F starts
eight of a sixteen-core machine already; it now also starts two of a machine that
claims eight. Bring-up costs one give-up bound per absent core — a hundred
milliseconds each, up to seven — which is paid once, on a machine that would
otherwise not boot at all.

Hard: bring-up is serial because the trampoline holds one stack pointer, and
stepping over a core means the next `wake` overwrites it. A core that was slow
rather than absent would then arrive on its successor's stack. That is why an
abandoned core is held rather than left: the `INIT` puts it back in
wait-for-startup, and the mailbox is re-read afterwards, so a core that answered
in the window between the two is a refusal (`ArrivedHeld`) rather than a core
counted as running that is not.

One thing came with it that is the same assumption one level down, and is
recorded here rather than in a second RFC: bring-up started its loop at apic id 1
because the boot processor is apic id 0, which firmware is under no obligation to
arrange. On a machine where it is not, that loop sends an `INIT` to the core it is
running on, and a core that resets itself between two instructions takes the
machine down with nothing in the log — the same symptom this RFC is about,
reached a different way. The loop now skips the core it is on. It still does not
*start* apic id 0 on such a machine, because the stack blocks are indexed by apic
id and index zero has none; that machine boots on one fewer core than it has, and
the boot log shows it as arithmetic that does not add up rather than as a
sentence.

Foreclosed: nothing. The real answer to *which cores does this machine have* is
the ACPI MADT, which `logical_processors` already names as its reversal and which
arrives with `E5-D03` and a root pointer this loader does not hand over. This
decision makes the interim honest; it is not a substitute for the census.

One lever came with it, and it is the part that is for a person rather than for
the kernel. `f.cores=<n>` on the boot line caps how many cores a boot uses, read
by the same `parameter_u32` mechanism `timer=` already uses. It exists because
everything else here is a change to what F does *after* it has a log, and a
machine that dies in bring-up has none: `f.cores=1` skips the stage entirely and
produces a whole log on the core the firmware started. It is deliberately not
part of `f_abi::boot`'s grammar — that grammar exists because a generation root is
written by one tool and read by another, and a debug switch nothing else writes
does not belong in the definition of an on-disk format.

What it costs the gate: three boots, in `cargo xtask verify` and in CI's kernel
job. The first is a machine that reports eight logical processors and answers
with two, which must reach `M0 ok` and must say it held six. The second is the
same machine with all eight present, which must start eight and hold none —
without it, a kernel that had quietly stopped starting any core at all would
pass. The third is `f.cores=1` on a machine with eight, because a parameter
nothing exercises is a parameter that has stopped working by the time somebody
with a dead machine reaches for it.

## What would reverse this

A machine where a core arrives after the give-up bound and after the `INIT` that
was supposed to hold it. That is the one assumption this decision rests on that
the hardware could refuse, and its symptom is `ArrivedHeld` in a boot log, or —
worse and quieter — two cores on one stack. If that is ever observed, the fix is
not to restore the refusal: it is to make the trampoline compute its own stack
from its apic id, at which point stepping over a core costs nothing and the hold
becomes an optimisation.

The other reversal is the good one: `E5-D03` lands the MADT, `logical_processors`
becomes a census rather than a bound, and a core that was enumerated by firmware
and does not answer becomes a real failure again — because then the machine did
tell us it was there. `cargo xtask cores` is the check that would have to be
re-argued at that point rather than deleted; a firmware list and a processor's
own count disagreeing is still a machine worth booting.
