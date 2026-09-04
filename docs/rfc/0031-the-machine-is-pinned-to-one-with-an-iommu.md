# RFC 0031: The emulated machine is pinned to one that has an IOMMU

- Status: accepted
- Date: 2026-09-03
- Affects: `xtask/src/main.rs` (`MACHINE`, `machine_devices`, the `iommu` verb),
  every recorded boot log and the trace hash over it, the state tree's snapshot
  hash; `kernel/src/arch/x86_64/acpi.rs`, `pci.rs`, `vtd.rs` and `dma.rs`, which
  are new and have nothing to find without it; `kernel/src/iommu.rs`, which
  implements `f_ring::registry`'s `Domains` and `PageWalk`; RFC 0005, whose
  domain kinds this does **not** merge with an IOMMU domain; RFC 0008, whose
  `Untyped` teardown a domain's tables are given back by; `TODO.md` E1-B01, and
  E1-B02, E1-B03 and E1-B04, which are the three drivers this exists for

## Decision

Every boot `xtask` runs now names its machine explicitly, and the machine it
names has a PCI Express root complex and a DMA remapping unit:
`-machine q35,kernel-irqchip=split -device intel-iommu,intremap=on -net none`.
Until now there was no `-machine` argument at all, so the pin was whatever QEMU
defaults to — the 1996 desktop chipset — and it was implicit.

Three things follow, and the second is the cost.

1. **The kernel discovers the machine through ACPI rather than through the
   memory map.** The multiboot handoff shows both new windows as reserved
   ranges, and reading their addresses off it would be reading a coincidence:
   the map says *something is here* and only ACPI says *what*. The measurement
   that settled it is in this commit's own history — the reserved range at
   `0xFED1_C000` that looked like the remapping unit's registers is the
   chipset's root complex register block, and the unit is at `0xFED9_0000`,
   which appears nowhere in the memory map at all.

2. **The recorded boot log and the state-tree snapshot hash move once, in this
   commit.** Booting one image on both machines gives fourteen changed lines and
   six new ones. The memory map gains two regions — the PCIe configuration
   window at `0xB000_0000` and the chipset's root-complex block at
   `0xFED1_C000`, which is *not* the remapping unit — usable memory falls by
   four kibibytes and one frame goes with it, and the six new lines are the
   `acpi`, `pci`, `iommu`, `iommu caps`, `iommu walks` and `iommu on` stages.
   The snapshot hash moves because the frame counts in it do. Nothing in the
   tree pins those numbers — `cargo xtask trace` compares two live runs rather
   than a committed fixture, and `claims/snapshot.json` is about the claims
   registry — so the cost is paid entirely by whoever is reading a log from
   before this commit beside one from after it.

3. **A machine with no remapping unit must still boot.** `-machine pc` remains a
   machine somebody runs, and so is every machine whose firmware has the unit
   switched off. The kernel says which of the four things it could not find —
   root pointer, configuration-space window, `DMAR`, or a unit it will drive —
   and carries on with one protection fewer.

The `iommu` verb adds one further device, a modern virtio block device with
`iommu_platform=on`, because the exit criterion needs something that performs
DMA and the three drivers that would have supplied one are blocked on this task.
No other boot has it.

## Context

`machine_with` already argued a pin, at the memory size: *the kernel prints the
loader's memory map, so the machine's size is part of its output, and an
emulator default that moves between versions would move the boot log with it.*
That argument was made about `-m` and `-smp` and not about `-machine`, which is
the gap this closes: the chipset was pinned in exactly the sense that mattered
least, by nobody having said anything.

E1-B01's exit is that *a driver component provably cannot address memory outside
its grant; the attempt is a fault, not a corruption.* Every protection this
kernel has is a protection against the **processor** reaching memory it should
not. A device is not the processor: it has a bus master bit and a descriptor
ring, and it addresses memory without consulting a page table. So a driver at
ring 3 that can program a device can address every byte of the machine, and the
capability system it is running inside is decoration. There is no way to write
that protection, and no way to demonstrate it, on a machine with nowhere to put
a remapping unit.

Three alternatives were live.

**Leave the default and pass the machine only to a new verb.** Cheapest, and it
was rejected for the reason `mutate` exists: a protection that is only ever
exercised by the command written to exercise it is a protection that stops
working the first time somebody changes something else. Enabling translation on
every boot means every `fault`, `user`, `cap` and `mutate` boot runs with the
unit on, so a regression that broke translation would show up in eleven
capability boots rather than in one.

**Detect the unit and enable it only when a driver asks.** This is what a mature
kernel does and it is the right answer later. It was rejected now because it
makes the interesting state — translation on, tables live — the *unusual* one,
and E1-B02 is about to be written against whatever state is usual.

**Use a device that is behind the unit unconditionally, and skip virtio.** An
emulated network card would have done, and the reason virtio won is that
`user/virtio-blk/manifest.toml` already exists and E1-B02 is a virtio driver:
the fact this task found out the hard way — that virtio bypasses the IOMMU
unless the driver negotiates `VIRTIO_F_ACCESS_PLATFORM` — is a fact E1-B02 needs
and would otherwise have found out for itself, later, as a passing test that
proved nothing.

## Consequences

**Easy.** A remapping unit exists on every boot, so the kernel's IOMMU stage is
exercised by every `xtask` verb rather than by one. `cargo xtask iommu` is two
boots and the whole of E1-B01's evidence. The three driver tasks get an
interface with hardware behind it rather than a trait with test doubles behind
it.

**Hard, and this one was nearly missed.** The unit on the pinned machine reports
`ECAP.C` clear: its page walks do **not** snoop the processor's caches. Every
table the kernel writes for it — root, context, second level — goes through the
direct map, which is write-back cacheable, so each of those writes needs an
explicit cache-line flush before the unit is told to re-read it. The emulator
cannot exhibit the failure: QEMU reads guest RAM directly and has no cache to be
behind, so a build that skipped every flush passes `cargo xtask iommu` exactly as
a correct one does. That is the argument for handling it rather than observing
it — the machine that asserts the property is the machine that cannot punish
getting it wrong — and it is why `vtd::Coherency` is a type with the reasoning
attached rather than a bit read at one call site. The cost is a flush per entry
written and a flush per table allocated, ordered by one `mfence` ahead of the
invalidation that already followed every change.

**Hard.** Boot is longer by a bus walk and by 39 pages of device window mapped.
Enabling translation makes every device with no domain unable to address memory,
which is why `-net none` is part of the pin: the emulator's default network card
is a bus master nothing in this kernel drives, and the first packet it received
would be a fault nobody asked for. That option is the honest form of *this
kernel drives no devices that do DMA*, and it comes out on the day one of them
does.

**Foreclosed.** Nothing, but one thing is made visible that was previously
invisible: a boot on the older chipset now takes a different path through the
kernel, and the two paths have to be kept working. The `-machine pc` boot is not
in CI; what keeps it honest is that every refusal on that path is a printed line
rather than a silence.

## What would reverse this

- **A QEMU whose `q35` machine or `intel-iommu` device changes shape between
  versions often enough that the boot log stops being comparable across the
  container's own upgrades.** The pin exists to stop the log moving under the
  tree; if pinning it to a richer machine moves it *more*, the pin has bought
  the opposite of what it was for. `docker/Dockerfile` pins the emulator, so
  what this predicts is that the pin there becomes load-bearing rather than
  hygienic.

- **A boot-time cost that shows up in `claims/0002-timer-jitter.toml`.** The bus
  walk and the translation-enable happen before the timer window opens, so they
  should not reach the histogram at all. If they do, the discovery moves behind
  a boot parameter and the `iommu` verb becomes the only boot that has a unit —
  which is alternative one above, adopted for a measured reason rather than a
  cheap one.

- **A device this system must drive that offers only the legacy virtio
  interface.** Such a device cannot negotiate platform addressing and therefore
  cannot be isolated. What reverses then is not this RFC but whether that device
  is used at all, and the answer should be no.

- **A unit that reports page-walk coherency, or a measurement showing the
  flushes above dominating a map.** The first makes `vtd::Coherency` a branch
  that is never taken and costs nothing; the second is E1-B14's
  unmap-under-churn workload, and what it would buy is mapping the unit's tables
  uncacheable instead — trading a flush per write for every read of a table
  being a memory reference. Neither reverses the pin; both reverse how the pin
  is paid for.

- **A machine with several remapping units, at least one of them scoped.**
  `vtd::Unit::open` accepts a single unit without the include-all flag because
  QEMU describes its unit that way and translates for everything behind it
  anyway; it refuses several. A machine that needs the scopes read is the
  machine where that judgement stops holding, and the symptom is `dma=outside`
  completing.
