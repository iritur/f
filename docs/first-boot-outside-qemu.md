# The first boot outside QEMU

On 2026-09-01 this kernel booted to `M0 ok` on something that was not QEMU, for
the first time since the project began. This is the record of it: the log, what
it established, what it did **not** establish, and the two findings it produced.

**It was a VMware virtual machine, not bare metal, and the distinction is the
point of this page.** `E0-P18` asks for a machine that is not an emulator, and
this is not the machine that task is about — see *What this did not establish*.

## What ran

A minimal Arch Linux install in VMware, UEFI firmware, Secure Boot off, GRUB
installed beside systemd-boot, one virtual serial port at COM1. 16 GiB of RAM
and 2 vCPUs. The procedure was `docs/booting-on-hardware.md` via
`tools/f-on-metal.sh`, which is what made it reproducible rather than an
afternoon somebody remembers going well.

## The log

```
F — milestone M0
  abi version   1
  sqe size      64 bytes
  cqe size      32 bytes
  per-cpu       core 0 of 8, slots distinct
  exceptions    ok — breakpoint taken and returned
  loader        multiboot 1
    0x0000000000000000           4 KiB  acpi-nvs
    0x0000000000001000           4 KiB  reserved
    0x0000000000002000         632 KiB  usable
    0x00000000000c0000         256 KiB  reserved
    0x0000000000100000      223828 KiB  usable
    0x000000000db95000         116 KiB  acpi
    0x000000000dbb2000        4624 KiB  usable
    0x000000000e036000          24 KiB  acpi-nvs
    0x000000000e03c000       27852 KiB  usable
    0x000000000fb6f000         128 KiB  unknown
    0x000000000fb8f000         320 KiB  reserved
    0x000000000fbdf000         124 KiB  usable
    0x000000000fbfe000           4 KiB  acpi-nvs
    0x000000000fbff000     2887684 KiB  usable
    0x00000000ffc00000         168 KiB  reserved
    0x0000000100000000    13631488 KiB  usable
  regions       16
  usable        16776232 KiB
  loader says   0 KiB low, 223828 KiB high
  module        0x000000000053d000..0x000000000053d363  0 KiB
  seed          0xf00dbeefcafe1234
  env digest    0x50eb86cb7769f326
  env clock     800 ns
  determinism   ok
  frames        261504 free of 261504
  paging        no-execute on, global pages on, pcid available, and deliberately off, direct map in 1 GiB pages
  address space 0x000000003ffff000 root, direct map at 0xffff800000000000
  reclaimed     3932160 frame(s) above the old identity map
  frame alloc   ok
  frame hygiene ok — 0 clean, 4193659 dirty
  local apic    xapic at 0x00000000fee00000, version 0x15, 7 lvt entries
  jitter        ok
  clocks        measured against the 8254 over 10 ms; timer via tsc-deadline
  wall clock    firmware rtc, uncertain to 3600 s
  env contract  arithmetic ok, seeded ok, hardware ok
  cores         2 of 8 shards, each with its own tables and stacks
  capabilities  32 slots, 5 properties hold, 5 flawed tables caught
  ring wrote    "the ring is open" through WRITE_SERIAL
  ring          16 entries in 4096 B, 2176 B arena, two ends at ABI v1, 4 published with one store, 1 refused, forged slot caught, hostile header refused
  doorbell      KernelIpi, 1 delivered, 500 per 1000 operations, a draining consumer was not rung
  state tree    12 nodes, snapshot 0x00433d2855a11555, stable across a re-read
  state           1  frame = 0
  state           2  memory = 0
  state           3  total = 4193664
  state           4  free = 4193655
  state           5  topology = 0
  state           6  started = 2
  state           7  ring = 0
  state           8  executed = 4
  state           9  refused = 1
  state          10  caps = 0
  state          11  slots = 32
  state          63  reserved = <kind 238 not named by this build>
  process       layout ok, sysret selectors agree
  init          867 bytes from boot module 1 of 1
  provoking     a read of the kernel's direct map, from ring 3
  init process  core 1, 4 call(s) answered, 0 refused, ended with status 0
  user space    core 1, root 0x000000043fd09000, 3 kernel slot(s) shared
  user frames   8 given back, free count unchanged
  user caps     4 granted, 4 call(s) answered, 0 refused, 5 held at the end
  user process  announced itself, then ran until the frame had taken 8 tick(s) from ring 3
  user death    exception 14 at 0xffff800000000000, error 0x5, rip 0x0000000000400161 — killed
  timer         100 ticks at 1000 Hz, across another core's ring 3
M0 ok
```

## Every difference from QEMU, accounted for

`E0-P18` asks that the differences be explained rather than listed, so that a
reader can tell which lines are about the kernel and which are about the machine.
Each row below is a path that had **never executed anywhere** before this boot.

| | QEMU, `-m 128M -smp 2` | this boot |
|---|---|---|
| direct map | 2 MiB pages | **1 GiB pages** — the `pdpe1gb` branch, never taken |
| PCID | unavailable | **available**, and deliberately off |
| timer | `apic one-shot` | **`tsc-deadline`** — E0-B07's primary path, never once run |
| local APIC | version 0x14, 6 LVT entries | version 0x15, 7 LVT entries |
| memory map | 7 regions | **16 regions**, from real UEFI firmware |
| `mem_lower` | 639 KiB | **0 KiB** — UEFI GRUB reports nothing below 1 MiB |
| frames | 32 245 | **4 193 664** |
| reclaim pass | adds nothing | **adds 3 932 160 frames** |

Two of these are worth more than the rest. The TSC-deadline timer is what
`claims/0002-timer-jitter.toml` will eventually be measured against, and until
this boot no line of that path had ever executed — QEMU's TCG backend refuses
`tsc-deadline` by name. And the ring-3 provocation was answered by a real MMU:

```
user death    exception 14 at 0xffff800000000000, error 0x5, rip 0x... — killed
```

That is a process being stopped by hardware for reading the kernel's direct map,
rather than by an emulator agreeing to pretend.

**The trace hash necessarily differs and is not a determinism failure.** The
memory map and core count are in the boot log by design, which is why `xtask`
pins `-m 128M -smp 2`. `E0-P02` claims two runs of *one configuration* agree.

## Finding 1: a pass that had never run

```
reclaimed     3932160 frame(s) above the old identity map
```

The boot stub's identity window is 1 GiB (`mem::IDENTITY_LIMIT`). Frames above it
are counted unreachable until the direct map exists, and a second pass reclaims
them afterwards. On a 128 MiB emulator that pass has nothing to do, and its
comment in `kernel/src/main.rs` said so:

> Nothing was skipped on this machine; the pass exists so that the first machine
> with more than a gibibyte does not quietly lose the rest of it.

This was that machine. The pass worked — 261 504 + 3 932 160 = 4 193 664 frames,
which is the 16 GiB the firmware reported. Written correctly, first time, and
never executed until now: that is the part worth noticing, because it is equally
the shape of a pass that would *not* have worked.

## Finding 2: an assumption about hardware, written on an emulator

The boot was slow enough to look like a hang — the log stopped after
`address space` for long enough that the first diagnosis was a fault in the
address-space switch. It was not. `FrameAllocator::add_region` filters per frame
against the reserved ranges, and its doc comment justified that:

> A machine has tens of thousands of frames and a handful of reserved ranges, so
> the loop costs nothing at boot.

Four million frames, not tens of thousands. The scan ran once per frame per
range.

**Fixed by giving the fast path a ceiling, not by rewriting it.** The per-frame
filter still decides and is unchanged; it is simply not consulted where its
answer cannot be in doubt. Overlapping requires `frame < r.end`, so a frame at or
above every reserved end overlaps nothing, and one comparison replaces the whole
scan — which is nearly every frame, because reserved ranges are few and clustered
low, and on the second pass the largest of them is everything the first pass
already took.

Measured in QEMU at 4 GiB, five runs each, medians:

| | runs (ms) | median |
|---|---|---|
| before | 7308, 4729, 5917, 8678, 5307 | **5917** |
| after | 4395, 3192, 2957, 3759, 2250 | **3192** |

**1.85×**, and the ranges do not overlap — the slowest run after the change is
faster than the fastest run before it. Frame counts are identical either way,
which is what makes it a speed change rather than a behaviour change.

Wall-clock under TCG on a loaded host is noisy, which is exactly why this is five
runs each rather than one. A single pair taken at 16 GiB gave 77 s before and
147 s after, which is host contention rather than a result, and is recorded here
because it is the reading that would have been published if nobody had repeated
it.

## What is left, and not fixed here

**The image is a debug build**, and most of what remains is that. `RELEASING.md`
ships `target/<target>/debug/f-kernel.elf32`, so every bounds check and overflow
check in a four-million-iteration loop is in the boot path. Whether the release
package should carry an optimised image is a separate decision with its own
consequences for `E0-R01`'s content addressing, and it is not made here.

**The remaining cost is the allocator's design and is not a defect.** The free
list is written into the frames themselves — no bitmap to size, no array to
place, and no bootstrap problem — which means initialising it must write one word
into every frame. On a 16 GiB machine that is 4 million writes and 16 GiB of
memory traffic, and no amount of filtering removes it. A different structure
would trade that for a different cost; this page is not the place to choose one.

## Follow-up, the same day: the optimised image had never booted either

Chasing the rest of the boot time led straight to the obvious fix — boot the
optimised image instead of the debug one — and the obvious fix did not boot:

```
FAIL: bringing up a core: a core was started and never reached kernel code
  core          1
```

Same kernel, same machine, same QEMU; the only difference was `--release`. The
investigation is worth recording because every intermediate conclusion was
wrong in an instructive way. The QEMU interrupt log showed the second core
taking its INIT and then nothing, which looked like a lost startup interrupt;
the compiled trampoline install, the IPI sequence and its delays, and the
mailbox polling loop were all read in disassembly and all correct. Freezing
the failed machine settled it: the second core was *running kernel code*,
parked and healthy — polling the boot processor's mailbox slot instead of its
own.

The bug was `cpuid()`'s inline assembly. It saved `rbx` with a push, captured
the result with `mov {out:e}, ebx`, and restored with a pop — and under
optimisation the register allocator hands the output operand `ebx` itself,
because that makes the capture a deletable no-op. The pop then overwrites the
result with whatever the caller had in `rbx`, which on a core fresh out of the
trampoline is zero. So `current_cpu()` on the application processor returned
0: it read the right handoff (a second inlined copy happened to get a
different register), reported ready in core 0's slot, and parked watching
core 0's mailbox, while the boot processor watched slot 1 for a word that
never came. The fix is an exchange instead of a capture-then-restore, which is
correct under both allocations. Debug builds never coalesce the copy, which is
why eleven weeks of green runs said nothing about it.

This is Finding 1's shape with the sign flipped: a path nobody had ever
executed, except this one was wrong. The optimised build of this kernel had
never been booted by anything — `cargo xtask` builds, tests and ships the
debug image — so the miscompile sat in every release build ever produced and
no fixture could have seen it. `f-on-metal.sh build` now builds the optimised
image *and boots it under QEMU* before installing it, because it is the image
hardware gets: measured at 4 GiB, five runs, the debug image boots in a median
of 3818 ms and the optimised one in 2220 ms, and the gap widens with memory.
What the release *package* carries is still `RELEASING.md`'s decision and is
still not made here.

## What this did not establish

- **It is not bare metal.** VMware runs guest instructions natively, so the CPU
  paths above are real, but the firmware, the APIC and the UART are all software
  models. Vendor firmware is where the surprises live, and none of it was met.
- **`E0-P18` stays open.** Its exit says "a machine that is not an emulator", and
  while a hypervisor is arguably not one, the task exists for contact with real
  firmware. Closing it on this would be closing it on the rehearsal.
- **No claim was measured.** `claims/0001` and `claims/0002` need
  `runner-class-A`, and `F_ENVIRONMENT` was not set here. The TSC-deadline timer
  having *run* is not the jitter number having been *taken*.
