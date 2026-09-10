---
id: 0010
status: draft        # draft | accepted | withdrawn | shipped
originator: Dmitri Chudinov
todo:                # TODO.md task IDs, once there are any
---

# One machine refuses the ring-3 stack selector it has already accepted

*Filed from the fourth and fifth boot attempts outside QEMU, 2026-09-09 and
2026-09-10, on a VMware guest hosted by a Threadripper 2990WX. RFC 0070 fixes
the other defect those boots found and explicitly does not claim this one.*

## Problem

Booted with `f.cores=1`, F runs the whole way to ring 3 on that machine — the
supervisor, four places, five state trees, the ring, the notices, all of it
byte-identical to the same commit under QEMU — and then dies:

```
  provoking     a read of the kernel's direct map, from ring 3

EXCEPTION 13 — general protection fault
  error         0x0000000000000030
  rip           0xffffffff80106129   cs 0x0008
  rsp           0xffffffff8018e498   ss 0x0000
  rcx 0x0000000000400352  r11 0x0000000000000246  rbp 0x0000000000402ff8
```

`0xffffffff80106129` is the `iretq` at the end of `isr_common`, disassembled out
of the same optimised image the machine booted. `rcx` and `r11` hold a ring-3
instruction pointer and a ring-3 flags word, and `rbp` a ring-3 stack address:
this is an interrupt that was taken while the process was running, returning to
it.

Error code `0x30` is not a flag word. On a general-protection fault it is a
**selector**: GDT, entry 6, which is the ring-3 stack segment.

## Why this is worth a directory rather than a fix

Everything about that descriptor reads correct, and reading is all anyone has
done:

- it is `0x00CF_F200_0000_FFFF` — present, ring 3, data, writable, flat;
- entry 6 is inside a table of eight whose limit is 63;
- `kernel/src/arch/x86_64/gdt.rs` writes all eight before loading `GDTR`;
- one write per slot, and the boot's own `process layout ok, sysret selectors
  agree` line passed;
- and **the same selector had already been loaded on that boot**, by the `iretq`
  in `enter_user` that put the process into ring 3 in the first place. A
  descriptor that is wrong is wrong both times.

QEMU does not reproduce it, including under `-cpu EPYC`, which does reproduce
every other difference that machine's log shows — `pcid unavailable`, six
local-APIC LVT entries, the one-shot timer backend. So the reproduction is a
machine, not a model, and the next person to guess at it will be guessing at the
same four things.

## What has been done instead of guessing

RFC 0070 lands the instrument. On the next boot of that machine the fault report
prints the descriptor **the processor actually read**, out of the table `GDTR`
actually names, and the five words the `iretq` was actually handed. Those two
lines separate the three live explanations — a table that is not the one this
kernel built, a frame whose selectors are not the ones it thinks it pushed, and
a processor applying a check the others do not — and no reading of the tree can.

## What would make this shippable

A boot of that machine on a build carrying RFC 0070's report, and the two lines
it prints. If they show a descriptor that is not `0x00CF_F200_0000_FFFF`, this
becomes a memory-corruption bug and the question is what wrote there. If they
show the descriptor intact and a frame whose `cs` is not a ring-3 selector, it
is a frame bug. If both are exactly what this tree intends, it is a
processor-behaviour difference and belongs in `docs/rfc/` with the manual
reference beside it.

**Triage is a human step**, and this is filed rather than fixed for that reason:
there are three candidate causes, they need different fixes, and the boot that
distinguishes them costs one reboot of a machine somebody owns.
