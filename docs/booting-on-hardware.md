# Booting on hardware

This kernel has only ever run under QEMU. Everything the boot log asserts is
asserted about an emulator, and `E0-P18` is the task that owns closing that.
This page is the procedure — written before the first hardware boot rather than
after it, so that the boot is reproducible rather than an anecdote somebody
remembers going well.

It is also a prerequisite nobody had written down. `E0-P05` and `E0-P06` both
need `runner-class-A`, and both need F running *on* it: `claims/runner-class-A.md`
says the reservation is carved "by F's own frame" for F's runs, and turns Secure
Boot off "because the kernel under test is not signed". Neither sentence makes
sense unless F boots on the metal.

## What the kernel expects from a machine

| | |
|---|---|
| **Loader** | multiboot **1** — header magic `0x1BADB002`, flags `0x3`. GRUB's `multiboot` command, not `multiboot2`. |
| **Modules** | exactly one: `user/init`, as the first module. Under QEMU this arrives as `-initrd`; under GRUB it is `module`. |
| **Console** | 16550 UART on **COM1, port `0x3F8`, 38400 baud, 8N1**. |
| **Video** | none. The multiboot header does not request a framebuffer, and the kernel writes to no display. |
| **Firmware** | Secure Boot off. The image is not signed. |

### The console is the whole interface, and 38400 is not a typo

`kernel/src/arch/x86_64/serial.rs` sets divisor 3, which is 38400 baud — not the
115200 most people reach for. There is no other output device. **On a machine
with no serial port you will see a black screen and have no way to tell a clean
boot from a triple fault**, which is the single most important thing on this
page.

That is also why a laptop is close to useless here and why server hardware is
the right target: a BMC gives you serial-over-LAN without a cable. A desktop
board with a COM header needs a bracket. A machine with neither cannot report.

### How a run ends

After `M0 ok` the kernel writes to port `0xF4` — QEMU's `isa-debug-exit` device,
which the launch configuration in `xtask` adds and which no real machine has —
and then falls into a halt loop. `exit_qemu` says so at the call site.

**So on hardware, success looks like the machine sitting there doing nothing
after printing `M0 ok`.** There is no reboot and no exit code. The exit code is
an emulator convenience, and on hardware the serial log is the whole result.

## The two files

```
target/x86_64-unknown-none/debug/f-kernel.elf32   the kernel, ELF32, multiboot 1
target/init/init.bin                              user/init, a flat blob, no headers
```

`target/` lives in a Docker volume and is not visible from Windows, so copy it
out first:

```
.\docker\dev.ps1 export
```

which leaves both under `.\target-export\`.

## The short way, on Arch

`tools/f-on-metal.sh` does Procedure A and the checks around it. Run `check`
first — it changes nothing and reports what would go wrong, which is worth more
than the install step, because the failure modes here are silent:

```sh
./tools/f-on-metal.sh check      # what this machine would do. No changes.
sudo ./tools/f-on-metal.sh build      # toolchain, build, and boot it under QEMU here first
sudo ./tools/f-on-metal.sh install    # add the entries beside Arch
sudo ./tools/f-on-metal.sh uninstall  # and take them away again
```

It refuses an ELF64 image — the easiest mistake, since cargo leaves one beside
the `.elf32` — backs up `grub.cfg` before regenerating, never touches
`GRUB_DEFAULT`, and writes `/etc/grub.d/45_f` rather than appending to
`40_custom`, so a second run cannot leave two entries behind. `build` boots the
result under QEMU on the target machine before you ask the firmware to, which
means a failure on metal has one fewer explanation.

The manual procedure is below, and it is what the script does; read it if the
script refuses something and you want to disagree with it.

## Procedure A — a GRUB entry on a machine that already runs Linux

This is the recommended route, and on `runner-class-A` it is the only one that
makes sense: that machine runs Linux anyway, because the baseline half of claim
0001 is *tuned Linux* and the pre-flight checklist is `lscpu` and `/proc/cpuinfo`.

It is also reversible. The machine still boots Linux by default; F is one entry
in the menu.

```sh
sudo mkdir -p /boot/f
sudo cp f-kernel.elf32 init.bin /boot/f/
```

Add to `/etc/grub.d/40_custom`:

```
menuentry "F — M0" {
    multiboot /boot/f/f-kernel.elf32
    module    /boot/f/init.bin
    boot
}
```

Then `sudo update-grub` (Debian) or `sudo grub2-mkconfig -o /boot/grub2/grub.cfg`
(Fedora, RHEL).

To see GRUB's own menu over serial as well — worth it, because a boot that fails
*before* the kernel starts is otherwise indistinguishable from one that fails
after — put this in `/etc/default/grub`:

```
GRUB_TERMINAL="serial console"
GRUB_SERIAL_COMMAND="serial --unit=0 --speed=38400 --word=8 --parity=no --stop=1"
```

## Procedure B — a USB stick

For a machine you do not want to touch the bootloader of. Legacy/BIOS boot is
the more reliable path for multiboot 1; UEFI GRUB supports it but is fussier,
and if the firmware misbehaves, CSM is the fallback worth trying before
concluding anything about the kernel.

**`/dev/sdX` below is destructive. Check it twice.**

```sh
sudo mkfs.vfat -F32 /dev/sdX1
sudo mount /dev/sdX1 /mnt
sudo grub-install --target=i386-pc --boot-directory=/mnt/boot /dev/sdX
sudo mkdir -p /mnt/boot/f
sudo cp f-kernel.elf32 init.bin /mnt/boot/f/
```

`/mnt/boot/grub/grub.cfg`:

```
serial --unit=0 --speed=38400 --word=8 --parity=no --stop=1
terminal_input serial console
terminal_output serial console
set timeout=5

menuentry "F — M0" {
    multiboot /boot/f/f-kernel.elf32
    module    /boot/f/init.bin
    boot
}
```

For UEFI, substitute:

```sh
sudo grub-install --target=x86_64-efi --efi-directory=/mnt --boot-directory=/mnt/boot --removable
```

## Command-line parameters

The kernel reads its options from the multiboot command line, so a GRUB entry
passes them the same way `-append` does under QEMU:

```
multiboot /boot/f/f-kernel.elf32 timer=60
```

`timer=<seconds>` runs the 1 kHz timer and prints the jitter histogram — which
is what `E0-P06` needs on `runner-class-A`, and the reason this page exists at
all. `fault=pf|ud|df|nx|wx|stack` provokes a deliberate fault. Both are read by
`kernel/src/main.rs` from the same `BootInfo`.

## The boot log will not match CI, and that is not a regression

`xtask` pins `-m 128M` and `-smp 2` deliberately, because the kernel prints the
loader's memory map and the number of cores it started — so the machine's shape
is part of its output, and an emulator default that moved between versions would
move the boot log with it.

Real hardware has a different memory map and a different core count, so **the
trace hash will differ from the one CI agrees on**. `E0-P02`'s claim is that two
runs of one configuration produce one hash, not that hardware matches an
emulator. Do not read a different hash on hardware as the determinism contract
failing; read two different hashes from two runs *on that machine* that way.

## Limits you will meet, and which kind each one is

These are four different sorts of number and it is worth not confusing them.

| | Value | Kind |
|---|---|---|
| `-m 128M`, `-smp 2` | | **Fixture pins.** Not kernel limits at all — QEMU launch parameters chosen so the boot log is reproducible. Irrelevant on hardware. |
| `MAX_REGIONS` | 256 | **A bound on untrusted input.** The memory map is length-prefixed and a corrupt length is a loop that never ends. "QEMU reports a handful of regions; a real machine reports tens." |
| `MAX_MODULES` | 8 | **A bound on untrusted input**, and a ninth module is *reported* as dropped rather than ignored — because a module nobody reserved is one the frame allocator hands out from under its owner. |
| `CMDLINE_MAX` | 128 | Same kind. A longer command line is truncated rather than rejected, on the grounds that a parameter that does not take effect is visible and a refusal to boot is not. |
| `MAX_CPUS` | 8 | **A real capacity choice with a real cost**, and the one to watch. |

### `MAX_CPUS` is logical processors, per socket

It counts **hardware threads**, not sockets and not physical cores:
`logical_processors()` reads CPUID leaf `0x0B` subleaf 1, which counts every
logical processor in the package — SMT siblings included. With SMT on, which
`runner-class-A` requires, eight threads is four physical cores.

Cores past the eighth are **left asleep on purpose**. `start` clamps with
`present.min(MAX_CPUS)`, and a boot that clamps says so on a line of its own —
which is what you will see on any machine worth measuring on. On a 64-thread
part:

```
  cores         8 of 8 shards, each with its own tables and stacks
  note          the processor reports 64 — 56 left asleep, past MAX_CPUS
```

**That `note` is correct behaviour, not a fault.** It appears only when
`present > cores`, because a log that reported just the number started would be
hiding which of the two it was.

### Why it is eight, and what raising it would cost

This is the first question that line provokes, so the answer is here rather than
waiting to be re-derived. The constant was raised to 64 for a Threadripper
2990WX, measured, and put back.

The cost is linear, and exactly so — these are arrays indexed by the constant
plus `linker.ld`'s `AP_CORES * AP_STACK_STRIDE`:

```
resident(N) = 438 566 + 64 072 × N bytes          62.6 KiB per core
```

| MAX_CPUS | resident | AP spin on a 64-thread machine |
|---|---|---|
| 2 | 553 KiB | 10 ms |
| **8** | **929 KiB** | **73 ms** |
| 16 | 1.40 MiB | 156 ms |
| 32 | 2.37 MiB | 322 ms |
| 64 | 4.33 MiB | 655 ms |

Built at 8, 16 and 64; the model came from the first and third and predicted the
second to the byte. Most of the 62.6 KiB is not `PerCpu` — 56 KiB is one guarded
AP stack block, reserved in the image because a guard page needs the mapper that
builds the kernel window. The ~10.4 ms is a hardcoded sequential spin in
`ap::wake`: 10 ms after `INIT`, 200 µs after each `STARTUP`, whatever the core
actually does.

**The two costs have different shapes.** Memory tracks the *constant* and is
paid on every machine, including single-core ones. Boot time tracks
`present.min(MAX_CPUS)` — the cores that exist — so a high ceiling on a small
machine costs memory and no time at all. Measured: the same kernel booted
`-smp 1/2/8` in 1054/990/879 ms, all noise around each other.

It stays at eight because a ceiling is not a speedup. Nothing here schedules
work above two cores, so the cores a larger ceiling admits would have nothing to
do. When it does pay it will pay as **admission capacity** — RFC 0007 reserves a
core whole, with its SMT sibling and a cache partition — rather than as
throughput.

What the 64 build did establish, and worth knowing before someone assumes
otherwise: the sharding scales. It brought up `64 of 64 shards, each with its
own tables and stacks` and reached `M0 ok`, and at `-smp 96` it started 64 and
reported the other 32 asleep. **Under emulation, which is not evidence about
hardware** — that is what this page is for.

`AP_CORES` in `linker.ld` must equal `MAX_CPUS - 1`, and
`arch::x86_64::ap::self_test` checks it at boot against the linker's own
symbols, so changing one and forgetting the other is a refused boot naming the
problem rather than a corrupted stack.

Two assumptions come with it, both named in the code rather than discovered
here: the count is **one package's**, so a two-socket machine is undercounted;
and APIC ids are assumed **dense and small**, because `current_cpu` shards on the
initial APIC id. The reversal is stated — parse the ACPI MADT, which multiboot 1
does not hand over and which has to be found in the BIOS area — and it retires
both assumptions in one change, at E5.

### Memory

There is no 128 MB anything in the kernel. Before the direct map is built the
allocator's ceiling is `IDENTITY_LIMIT`, 1 GiB, the boot stub's identity map;
after `rebind` it is the direct map's, up to 512 GiB — one page-directory-pointer
table, which `BuildError` calls "not a limit worth designing around". Frames
above the ceiling are counted as `unreachable` rather than handed out, so a large
machine reports rather than misbehaves.

## What a hardware boot does and does not buy

**Does:** the first evidence that any of this survives contact with real
firmware, real APIC enumeration, a real memory map and a real UART. None of that
has been observed even once.

**Does not:** close `E0-P05` or `E0-P06`. Those need `runner-class-A`
specifically — RDT CAT and MBA rule out every desktop part, since Intel dropped
CAT from client silicon after Skylake-X — and they need the reservation
conditions of RFC 0007 recorded with the number. A clean boot on a spare desktop
is worth having and is not a claim.

## Known gap: the claim harness cannot do this yet

`claims/0002-timer-jitter.toml` registers its reproduction as
`cargo xtask claim timer-jitter`, and that path ends in `boot()`, which runs
`qemu-system-x86_64`. The claim requires a reservation carved by F's own frame on
class-A hardware, and QEMU on that machine is not that.

So one of two things is owed before `E0-P06` can be taken at all: the harness
learns to drive a hardware boot and collect the log, or the claim's reproduction
command stops naming a command that cannot produce it. This is the same shape of
defect as the one `E0-R02` already found in this file — a registry whose one
command is not the command — and it is recorded here rather than left to be
discovered on the machine.
