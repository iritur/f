# RFC 0074: The ring-3 field of `IA32_STAR` carries its privilege level

- Status: accepted
- Date: 2026-09-12
- Affects: `kernel/src/arch/x86_64/gdt.rs`, `kernel/src/process.rs` (`self_test`),
  `kernel/src/arch/x86_64/idt.rs`, `kernel/Cargo.toml`, `xtask/src/main.rs`
  (`cargo xtask mutate`), `TODO.md` (`E0-P18`), RFC 0070's open second defect

## Decision

The field of `IA32_STAR` that `sysret` adds its offsets to is written **with its
two privilege bits set** — `0x2B`, not `0x28` — so that plain addition yields the
ring-3 stack selector `0x33` and the ring-3 code selector `0x3B` on every
processor, whether or not that processor also forces the bits.

`process::self_test` checks the field by **addition and not by forced OR**: the
field plus eight must *equal* `USER_DATA` and the field plus sixteen must
*equal* `USER_CODE`. A field that only agrees once the bits are forced is
refused before anything enters ring 3, because the forcing is one vendor's
behaviour and the check exists to model the other.

`mutate-sysret-base-without-privilege` rebuilds the defect as it was, and
`cargo xtask mutate` requires that build to be refused by the self-test with the
line naming it, and the clean build to boot green.

## Context

The second defect RFC 0070 left open, and did not guess at. On 2026-09-10 a
VMware guest on a Threadripper 2990WX host, booted with `f.cores=1`, reached
ring 3 and took `EXCEPTION 13` with error code `0x30` at the `iretq` in
`isr_common`. The descriptor at GDT entry 6 was, by every reading of the source,
a correct flat ring-3 data segment; the same selector had already been loaded by
the `iretq` in `enter_user` on the same boot; and QEMU would not reproduce it
under any `-cpu`, `EPYC` included. So RFC 0070 landed instruments instead of a
fix: the fault report prints the descriptor the processor actually read and the
five words an `iretq` was handed.

On 2026-09-12 the same machine booted the image carrying those instruments, four
ways — 2 vCPUs, 32 vCPUs, `f.cores=1`, and `f.bringup` — and every one printed
the same thing:

```
  provoking     a read of the kernel's direct map, from ring 3

EXCEPTION 13 — general protection fault
  error         0x0000000000000030
  rip           0xffffffff80106129   cs 0x0008
  selector      gdt entry 6 (selector 0x0030)
  gdtr          base 0xffffffff801659c0 limit 0x003f
  descriptor    0x00cff3000000ffff
  returning to  rip 0x000000000040035e  cs 0x003b  ss 0x0030
                rsp 0x0000000000405f40  rflags 0x0000000000000286
  ...
  rcx 0x0000000000400352 ... r11 0x0000000000000246
```

Both instruments answered. The descriptor is `0x00CF_F300_0000_FFFF`: present,
data, writable, privilege level three, accessed — the descriptor this kernel
installs, with the accessed bit the processor set when it loaded it. The table
was never wrong. The frame is the answer: **`cs 0x003b` and `ss 0x0030`**. The
code selector requests level three and the stack selector requests level zero,
and the architecture's `iretq` refuses that pairing with `#GP` and the stack
selector as its error code — which is exactly the fault, at exactly the
instruction, with exactly the code. `rcx` and `r11` hold a return address and
flags a `syscall` saved, so the process had made a system call and come back
through `sysret` before the interrupt that this `iretq` was returning from.

Where `0x30` came from is one line of `gdt.rs`:

```rust
pub const STAR: u64 = ((USER_BASE as u64) << 48) | ((KERNEL_CODE as u64) << 32);
```

with `USER_BASE = 0x28`. Intel's `sysret` loads `SS` with the field plus eight
**and forces the requested privilege level to three**, and so does QEMU's,
whichever vendor it is told to emulate. AMD's `sysret` loads the field plus
eight. `0x28 + 8 = 0x30`, a ring-3 process running with a stack selector that
says ring 0. Nothing in the process notices — a 64-bit stack segment's
attributes are not consulted — and nothing in the kernel notices until an
interrupt arrives while the process is running and the `iretq` that would resume
it is handed a frame the hardware pushed with `ss = 0x30` in it. That is the
`iretq` at `isr_common`, the one this kernel takes on every timer tick out of
ring 3, and it is why the first `iretq` — `enter_user`'s, with `0x33` pushed by
hand — had worked.

The rest of the evidence agrees. The 2026-09-10 boot only reached this with
`f.cores=1` because on the other two the machine died in bring-up first, which
RFC 0070 fixed; on 2026-09-12 all four reached it. Neither of the VMware
machines this kernel had booted on before had an AMD host. And the emulator
this tree runs its ring-3 suites on — `cargo xtask user`, `cargo xtask cap`,
every `verify` — could not have shown it at any point in the project's life,
because the instruction that differs is the one it emulates the Intel way.

The alternatives that were live:

- **Leave the field and force the bits in the kernel's own `sysret` path.**
  There is no such path: `sysret` loads `SS` itself and there is no instruction
  between it and ring 3. The only place the bits can be put is the register.
- **Load `SS` again on every kernel entry from ring 3.** That would hide the
  defect rather than remove it, at a cost on the one path — an interrupt out of
  ring 3 — that RFC 0016's whole argument is about keeping bare.
- **Call it a VMware defect and boot on Intel.** The behaviour is AMD's
  documented one and the one KVM models for AMD guests; the hypervisor is
  running the instruction natively. A kernel that only boots where a stronger
  vendor's courtesy hides its mistake has not been ported to the architecture,
  only to one implementation of it.

## Consequences

Easy: `sysret` produces the same two selectors on both vendors, and the
self-test says so by the weaker vendor's arithmetic, so a future change to the
selector layout that only works once the bits are forced is refused on the
emulator — the machine that cannot otherwise tell.

Hard: nothing. The value written differs in two bits, and both of them were
always meant to be there. There is no path in this kernel on which a field
without them was the right value.

Foreclosed: writing a `sysret` field that expects the processor to finish it.
The forcing Intel applies is a courtesy this kernel no longer depends on, and
the mutation exists so that nobody re-introduces the dependence by tidying.

What the mutation costs: one feature that is on nowhere, one entry in
`DEFECTS`, and one more boot pair in `cargo xtask mutate` — a red boot that
ends at `process       layout ok` replaced by a `FAIL:` line, and a green one.

What this does *not* establish: that the machine boots to `M0 ok`. The fix is
tested on the emulator, where the defect it removes is invisible, and by the
self-test, which models the vendor the emulator is not. The next boot on that
machine is the demonstration, and `E0-P18` carries it. If that boot fails
further on, it fails somewhere this tree has never been, and the fault report
will say where.

## What would reverse this

A processor on which `sysret` treats a field with the privilege bits set as
anything other than the selector with those bits set — the architecture manuals
of both vendors say it does not, and Linux has written the field this way since
long mode existed. Or a `sysret` that ceases to load `SS` from `IA32_STAR` at
all, in which case the field is not the interface and this RFC is about a
register nothing reads.

For the self-test: a vendor whose `sysret` computes the stack selector from
something other than the field plus eight would make the addition the check
performs a check of the wrong arithmetic, and the repair is a second arithmetic
in the same test with its own sentence, not the return of the forced OR.
