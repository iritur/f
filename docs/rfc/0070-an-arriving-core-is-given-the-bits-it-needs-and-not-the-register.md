# RFC 0070: An arriving core is given the bits it needs, not the register

- Status: accepted
- Date: 2026-09-10
- Affects: `kernel/src/arch/x86_64/ap.rs`, `kernel/src/smp.rs`, `kernel/src/main.rs`,
  `kernel/src/arch/x86_64/idt.rs`, `xtask/src/main.rs` (`cargo xtask cores`),
  `docs/booting-on-hardware.md`, `TODO.md` (`E0-P18`)

## Decision

The trampoline hands an arriving core **`SCE | LME | NXE`** out of the boot
processor's `IA32_EFER`, and not the register. The bit this exists to drop is
**`LMA`, bit 10** — *long mode active* — which is a status bit the processor
maintains, reads back as 1 on a core already in long mode, and is written by
that trampoline into a core that is in thirty-two-bit protected mode with paging
off, where long mode is by definition not active.

Two instruments land with it, because the failure this fixes was undiagnosable
from a serial log and that is a defect of its own.

An arriving core **writes its progress into one byte** of the trampoline page,
and the boot processor **watches that byte while it waits** and prints each new
value under `f.bringup`. The ordering is the point: a core that dies in the
trampoline dies with no descriptor table and no handler, which is a triple fault
and, on a hypervisor, the whole virtual machine stopping — boot processor
included. A report printed afterwards is never printed at all. The give-up path
prints the last stage unconditionally, which is free and answers the other case.

And a fault whose error code names a **descriptor** now prints that descriptor,
read out of the table the processor was actually using, together with `GDTR`;
and a fault at an `iretq` prints the five words that `iretq` was about to
consume.

## Context

On 2026-09-09 and again on 2026-09-10, a VMware guest on a Threadripper 2990WX
host would not boot. RFC 0068 had already made a core that never answers
survivable and put the census in the log ahead of the stage; that turned the
failure from silence into three logs, and the three logs are what made this
findable.

- **32 vCPUs**: the log ends at `bring-up 32 logical processor(s) reported, 7 to
  start beside this one`.
- **2 vCPUs**: the log ends at `bring-up 2 … 1 to start beside this one`.
- **`f.cores=1`**: the boot runs to completion and dies later, elsewhere, and
  differently — see below.

The first two put the failure inside bring-up with the boot processor's own
line as the last thing on the wire, and the third proved bring-up was the only
thing wrong with that part of the boot: with no core started, everything through
the supervisor, the state trees and the ring ran exactly as it does under QEMU.

The trampoline's parameters are the only machine-dependent input to that stage
that this kernel composes rather than reads. `CR3`, `RSP` and the entry point
are addresses out of the image. `CR4` is copied and has no read-only bits.
`IA32_EFER` is copied, and it has one: `LMA`. Writing a 1 to a read-only status
bit whose current value is 0 is a thing a processor may ignore and a thing a
processor may raise `#GP` for, and the trampoline is the one place in this
kernel where a `#GP` cannot be reported — it is taken before there is an
interrupt descriptor table on that core.

Every machine this kernel had booted on tolerated it. That is not evidence that
it was correct; it is the reason it survived eleven months. **The write was
wrong on every one of those machines too** — the value being written was never
the value being asked for — and this is the ordinary shape of a latent defect:
correct-looking code, a permissive first host, and a second host that is not.

## Consequences

Easy: an arriving core is given a value that is a statement of intent — these
three bits, each of which has a sentence saying why the core cannot do without
it — rather than a snapshot of another core's register that happens to work.
`NXE` is the one that would bite hardest if it were dropped: it has to match the
boot processor's, or the no-execute bit in the page tables the core is about to
load becomes a reserved bit and every kernel page that sets it faults.

Hard: a bit outside the three that a core turns out to need has to be added
here, deliberately, rather than arriving for free. That is the trade, and it is
the right way round — the register is what this stops copying.

What the instruments cost: eight stores in the trampoline and a byte in a page
that already holds five parameters, a poll loop on the boot processor that runs
only under `f.bringup`, and a handful of lines in a fault report that only ever
runs on the fatal path. A healthy boot log is unchanged. `cargo xtask cores`
gained an assertion on the give-up line, so the one report a dying core has is
exercised on every `verify` rather than only when it is needed.

Foreclosed: nothing.

## What this does not fix, and is not claimed to

**The `f.cores=1` boot on that machine dies too, and this is not why.** It runs
to ring 3 and takes `EXCEPTION 13 — general protection fault` with error code
`0x30` at `rip 0xffffffff80106129`, which is the `iretq` at the end of
`isr_common`: an interrupt taken while the process was running, returning to
ring 3. `rcx` and `r11` hold the process's own instruction pointer and flags,
which is what that frame should hold.

Error code `0x30` names GDT entry 6 — the ring-3 stack descriptor,
`0x00CF_F200_0000_FFFF`, which is a textbook flat ring-3 data segment, at an
index inside a table whose limit is 63, in a table this kernel writes eight
descriptors into. **And the same selector had already been loaded successfully
on that boot**, by the `iretq` in `enter_user` that put the process into ring 3
in the first place. Everything static about it reads correct.

So the second defect is real, is not this one, and is not guessed at here. What
lands instead is the instrument that will name it: the next boot prints the
descriptor the processor actually read and the frame the `iretq` was actually
handed. `intent/0010` carries the finding for triage.

*Named on 2026-09-12, by those instruments, on the next boot: the descriptor was
correct and the selector in the frame was `0x30`, not `0x33` — `sysret` on an
AMD host had loaded the `IA32_STAR` field plus eight as written, where Intel and
QEMU force the privilege bits. RFC 0074 is the fix.*

## What would reverse this

A machine where a core needs an `IA32_EFER` bit outside `SCE | LME | NXE` to
arrive. The symptom would be a core that reaches `STAGE_LONG_MODE` and then
faults on something the boot processor does not — most likely a paging feature
whose enable lives in that register. The repair is a fourth bit in the mask with
its own sentence, not a return to copying the register: the register still has
`LMA` in it.

For the instruments: a boot log that has become hard to read because bring-up
prints too much would reverse the *unconditional* give-up line, not the flag —
the flag is already off by default. And if a future loader gives the frame a
way to report from an arriving core directly, the byte in the trampoline page
is retired in favour of it, because a core that can say what happened does not
need another core to watch it.
