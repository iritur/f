# The second boot outside QEMU: the datapath on a machine

On 2026-09-05 this kernel booted to `M0 ok` outside QEMU for the second time,
carrying everything `E1` built. The first record —
`docs/first-boot-outside-qemu.md` — is about a kernel that got as far as one
user process. This one is about four components, a supervisor, and 32 GiB.

**It was a VMware machine again, so it is still not bare metal and `E0-P18`
still does not close on it.** What makes it worth a second page is that it ran
paths the first boot did not have, and that one of them found nothing where it
expected something.

## What ran

A VMware virtual machine, UEFI firmware, Secure Boot off, GRUB beside
systemd-boot, one serial port at COM1. **32 GiB and 2 vCPUs** — twice the memory
of the first boot. Installed by `tools/f-on-metal.sh`, which now carries every
component file rather than one, and booted the optimised image.

Five boot modules: `init.bin` and the four component files
`cargo xtask component` builds.

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
    0x0000000100000000    30408704 KiB  usable
  regions       16
  usable        33553448 KiB
  loader says   0 KiB low, 223828 KiB high
  module        0x0000000000595000..0x0000000000595363  0 KiB
  module        0x0000000000596000..0x00000000005977e8  5 KiB
  module        0x0000000000598000..0x000000000059c778  17 KiB
  module        0x000000000059d000..0x00000000005a1950  18 KiB
  module        0x00000000005a2000..0x00000000005a6d30  19 KiB
  seed          0xf00dbeefcafe1234
  env digest    0x74c02a9580a60021
  env clock     800 ns
  determinism   ok
  frames        261485 free of 261485
  paging        no-execute on, global pages on, pcid available, and deliberately off, direct map in 1 GiB pages
  address space 0x000000000fbff000 root, direct map at 0xffff800000000000
  reclaimed     8126464 frame(s) above the old identity map
  frame alloc   ok — orders 0..=18, 1035 split, 1563 merged
  frame shards  8 shards, 0 cross-core on the hot path, 21 refill(s), 1 forced
  frame hygiene ok — 0 clean, 8387944 dirty
  local apic    xapic at 0x00000000fee00000, version 0x15, 7 lvt entries
  jitter        ok
  clocks        measured against the 8254 over 10 ms; timer via tsc-deadline
  wall clock    firmware rtc, uncertain to 3600 s
  env contract  arithmetic ok, seeded ok, hardware ok
  cores         2 of 8 shards, each with its own tables and stacks
  acpi          none: no checksummed root pointer in either window
  capabilities  32 free slots, 128 more per page bought, 5 properties and 5 storage checks hold, 10 flawed tables caught
  ring wrote    "the ring is open" through WRITE_SERIAL
  ring          16 entries in 4096 B, 2176 B arena, two ends at ABI v1, 4 published with one store, 1 refused, forged slot caught, hostile header refused
  doorbell      KernelIpi, 1 delivered, 500 per 1000 operations, a draining consumer was not rung
  state tree    32 nodes, snapshot 0x93e6cf6bdb95d794, stable across a re-read
  state           1  frame = 0
  state           2  memory = 0
  state           3  total = 8387949
  state           4  free = 8387940
  state           5  topology = 0
  state           6  started = 2
  state           7  ring = 0
  state           8  executed = 4
  state           9  refused = 1
  state          10  caps = 0
  state          11  slots = 32
  state          12  served = 10815
  state          13  refill = 22
  state          14  remote = 1
  state          15  forced = 1
  state          16  iommu = 0
  state          17  domains = 0
  state          18  used = 0
  state          19  faults = 0
  state          20  blk = 0
  state          21  served = 0
  state          22  bytes = 0
  state          23  copies = 0
  state          24  provoked = 0
  state          25  runtime = 0
  state          26  hot = 0
  state          27  provoked = 0
  state          28  boundary = 0
  state          29  ticks = 0
  state          30  work = 0
  state          31  interrupts = 0
  state          63  reserved = <kind 238 not named by this build>
  supervisor    4 place(s) from 4 component file(s), one per file, each staked with an account its own manifest sized
  place         store: private, on_fault, 3 restart(s) in 3000 tick(s), 131072 B account
  admission     soft class, 65536 B footprint + 36864 B of declared need(s) against a 131072 B account — refused before anything is spent, never after
  spawn         place store epoch 0 — manifest 0xc4ca6f1cc7482842, 3 need(s) supplied, type, rights and quantity checked; 12 frame(s) from the account; control ring 4096 B
  place         virtio-blk: private, on_fault, 8 restart(s) in 60000 tick(s), 4194304 B account
  admission     soft class, 2097152 B footprint + 81920 B of declared need(s) against a 4194304 B account — refused before anything is spent, never after
  spawn         place virtio-blk epoch 0 — manifest 0xbacc90beaa621059, 4 need(s) supplied, type, rights and quantity checked; 26 frame(s) from the account; control ring 4096 B
  place         virtio-gpu: private, on_fault, 8 restart(s) in 60000 tick(s), 4194304 B account
  admission     soft class, 2097152 B footprint + 81920 B of declared need(s) against a 4194304 B account — refused before anything is spent, never after
  spawn         place virtio-gpu epoch 0 — manifest 0xe5d97b093e7db539, 4 need(s) supplied, type, rights and quantity checked; 27 frame(s) from the account; control ring 4096 B
  place         virtio-net: private, on_fault, 8 restart(s) in 60000 tick(s), 4194304 B account
  admission     soft class, 2097152 B footprint + 81920 B of declared need(s) against a 4194304 B account — refused before anything is spent, never after
  spawn         place virtio-net epoch 0 — manifest 0x0ab43df5ceef2bcd, 4 need(s) supplied, type, rights and quantity checked; 27 frame(s) from the account; control ring 4096 B
  connect       client -> place store: a channel opened, header epoch 0
  fault         place store epoch 0 stopped speaking: its control ring header no longer validates
  teardown      3 capabilit(ies) revoked of 32 slot(s), 12 frame(s) refunded to the account, 1 peer-gone notice(s)
  connect       client -> place store: the place is empty, the connect pends to a deadline 200 tick(s) out
  outcomes      a connect whose own deadline had passed earned PEER/EMPTY, which is not GONE: the place may yet be refilled
  refusals      5 spawn(s) refused on purpose, one per way a supply can be wrong: missing, undeclared, wrong type, short rights, short quantity
  restart       place store under on_fault — restart 1 of 3, backoff 8 tick(s)
  spawn         place store epoch 1 — nothing carried over: new table, new memory, new control ring
  resume        the pending connect completed: a channel to epoch 1, and the client observed only the wait
  stop          place store epoch 1 stopped against a deadline already behind it — a kill, and a second stop could not move it later
  teardown      3 capabilit(ies) revoked of 32 slot(s), 12 frame(s) refunded to the account, 1 peer-gone notice(s)
  retire        place store spent its budget of 3 restart(s) — retired, and 1 peer-gone notice(s) went to the endpoint's holders
  outcomes      a connect to a retired place earned PEER/GONE, arriving and already waiting: the place is not coming back
  budget        a window 3000 tick(s) wide: 3 restart(s) inside it retires the place, and the same count once it has elapsed does not
  teardown      place virtio-net — 4 capabilit(ies) revoked, 27 frame(s) refunded to its own account, 1 peer-gone notice(s)
  teardown      place virtio-gpu — 4 capabilit(ies) revoked, 27 frame(s) refunded to its own account, 1 peer-gone notice(s)
  teardown      place virtio-blk — 4 capabilit(ies) revoked, 26 frame(s) refunded to its own account, 1 peer-gone notice(s)
  notices       279 published in slot-then-stop-then-grade order over 26 round(s), 279 drained back at a polling point as 6 of 7 kind(s), 0 still owed
  supervisor    ok — 4 place(s), 5 spawn(s), 1 fault(s), 1 restart(s), 1 resumed, 0 client(s) lost, 8 probe(s) refused, 1 retired, 3 need(s) bound to nothing
  process       layout ok, sysret selectors agree
  init          867 bytes from boot module 1 of 5
  provoking     a read of the kernel's direct map, from ring 3
  init process  core 1, 4 call(s) answered, 0 refused, ended with status 0
  user space    core 1, root 0x000000000fc11000, 3 kernel slot(s) shared
  user frames   8 given back, free count unchanged
  user caps     4 granted, 4 call(s) answered, 0 refused, 5 held at the end
  user process  announced itself, then ran until the frame had taken 8 tick(s) from ring 3
  user death    exception 14 at 0xffff800000000000, error 0x5, rip 0x0000000000400175 — killed
  timer         100 ticks at 1000 Hz, across another core's ring 3
M0 ok
```

## The arithmetic, checked

A boot log is only evidence if somebody adds it up, so:

| | |
|---|---|
| below the 1 GiB identity window | 261 485 frames |
| reclaimed above it | 8 126 464 frames |
| sum | **8 387 949**, which is `state 3 total` exactly |
| firmware's usable total | 33 553 448 KiB = 8 388 362 frames |
| difference | 413 frames — the kernel image, the five modules, the boot structures |

`state 4 free = 8387940`, nine frames out at the point the tree is read. The
`frame shards` line says 21 refills and `state 13` says 22, which is the state
tree being read after more allocation than the line was: a count that had gone
*down* would be the interesting one.

## What this boot ran that the first one could not

The first boot's kernel had twelve state nodes and one module. This one has
thirty-two and five, and the difference is `E1` in its entirety.

| | first boot, 2026-09-01 | this boot |
|---|---|---|
| memory | 16 GiB | **32 GiB** |
| modules | 1 | **5** |
| state tree | 12 nodes | **32 nodes** |
| reclaim pass | 3 932 160 frames | **8 126 464 frames** |
| frame allocator | `frame alloc ok` | **`orders 0..=18, 1035 split, 1563 merged`** — the buddy allocator of RFC 0023 |
| free lists | — | **8 shards, 0 cross-core on the hot path** |
| capabilities | 32 slots, 5 properties | **32 free slots + 128 per page bought, 5 properties and 5 storage checks, 10 flawed tables caught** |
| components | — | **4 places, 5 spawns, a fault, a restart, a resumed connect, a retirement** |

The supervisor is the part worth naming. Four places built from four manifests
the loader carried; a client connected; a component killed by corrupting its
control ring header; a connect that arrived at an empty place and *pended*; the
place refilled under its declared policy; the pending connect resuming at epoch
1 having observed only the wait; the restart budget spent and the place retired;
and a connect to the retired place earning `PEER/GONE`. Then 279 notices
published and 279 drained with none owed. All of that against real page tables
on a machine with eight million frames, and none of it had executed anywhere but
an emulator until this boot.

**Zero cross-core allocations on the hot path**, at 32 GiB, is the per-CPU
convention (RFC 0016) holding on a machine large enough for it to have failed.

## A prediction the boot confirmed

`tools/f-on-metal.sh` carries the component files **sorted**, because a
filesystem order is one that can differ between two machines carrying the same
files. `xtask` hands QEMU the hand-written `COMPONENTS` list. The two orders are
not the same:

```
xtask:            store, virtio-blk, virtio-net, virtio-gpu
f-on-metal.sh:    store, virtio-blk, virtio-gpu, virtio-net
```

`docs/booting-on-hardware.md` was changed on 2026-09-04 to say that places 2 and
3 would therefore swap on hardware, and that this is a difference to account for
rather than a fault, because a component is found by the magic in its record and
never by its position. This log has `virtio-gpu` at place 2 and `virtio-net` at
place 3. The prediction was written before the boot and the boot agreed with it,
which is the only order in which a prediction is worth anything.

## The finding: ACPI cannot be found under UEFI, so the IOMMU stage never ran

```
acpi          none: no checksummed root pointer in either window
```

Everything downstream is zero: `state 16..19` — `iommu`, `domains`, `used`,
`faults` — and the five `iommu` lines that `E1-B01` exists to print are absent.

**This is structural, not bad luck.** `kernel/src/arch/x86_64/acpi.rs` finds the
root system description pointer the only way multiboot 1 leaves available, and
its own module comment says so: the protocol hands over a memory map, a command
line and a module list, and *there is no field in it for the RSDP*. So the kernel
scans the two windows the ACPI specification names — the extended BIOS data area,
via the pointer at `0x040E`, and `0xE0000..0x100000`.

UEFI firmware does not put the RSDP in either. It publishes it in the UEFI
configuration table, which a multiboot 1 kernel never sees. So on **any** UEFI
machine, this kernel finds no RSDP, therefore no `MCFG` and no `DMAR`, therefore
no PCIe configuration space and no remapping unit — and the entire `E1-B01`
protection is unreachable, silently and by construction.

**The kernel's behaviour is correct and is not the finding.** It fails closed
(R04): an unreadable table is *absent*, never assumed-good. It prints the reason,
returns `None`, and carries on to `M0 ok`. A machine with no DMAR is a machine
with less protection, not a broken machine, and the same shape covers a machine
with no component file. Nothing here is a defect in the boot.

**The finding is that the coverage is zero and nobody had noticed.** Every
assertion `E1` makes about IOMMU confinement is an assertion about QEMU's
`q35` with `-device intel-iommu`. The one configuration in which this kernel has
ever met real firmware is a configuration in which that code cannot execute. The
first boot did not reveal it because the ACPI stage did not exist yet, and the
`iommu` verb has only ever run under the emulator that supplies the tables.

**What it costs, concretely.** `E5-B02` is "the imported graphics stack as an
isolated, IOMMU-confined, restartable component". On a UEFI workstation, as the
tree stands, there is no IOMMU to confine it with. That task cannot be attempted
until the boot protocol changes, and it did not say so.

Filed as `E5-D03`, and named as a `needs:` on `E5-B02`.

### Why the answer is probably not "add multiboot 2"

The obvious fix is multiboot 2, whose tags 14 and 15 carry exactly the RSDP this
scan is looking for. It keeps GRUB, keeps the ELF32 image, and keeps
`f-on-metal.sh` almost unchanged. It is the cheapest candidate and it should not
be adopted on that basis alone, for a reason already written down in
`kernel/src/arch/x86_64/boot.rs`:

> QEMU implements multiboot 1 in its own `-kernel` loader, so this handoff costs
> nothing but the header and this stub … Multiboot 2 would mean GRUB and an ISO.

`run`, `trace`, `fault`, `cap`, `user`, `mutate`, `blk`, `net`, `gpu` and `iommu`
all launch through `-kernel`. A protocol that path cannot load turns every gate
verb in this tree into an ISO build, which is the same objection
`docs/booting-on-hardware.md` already makes about teaching F an EFI stub. So the
first question `E5-D03` has to answer is a measurement, not a preference:
**what does the pinned QEMU's `-kernel` actually load?** If it loads multiboot 2,
the change is small. If it does not, multiboot 2 buys one field at the cost of
every fast path, and Limine — which supplies the RSDP *and* the framebuffer
`E5-B02` needs anyway, at the price of a binary inside the licence boundary —
becomes the better trade despite being the larger change.

There is also a third option that costs nothing in the tree and should be stated
rather than assumed away: boot the `E0-P18` machine through **CSM/BIOS**, where
the legacy windows exist and multiboot 1 is enough. That closes `E0-P18` and
`E0-P05`/`E0-P06` without touching a line of kernel code, and closes nothing at
`E5`, because a workstation bought in 2026 may have no CSM at all. It is a
fallback for the claims runner, not an answer for the deployment target.

None of these is decided here. What this boot established is that the decision
has a date on it now, and a task.

## What this did not establish

- **It is not bare metal, and `E0-P18` stays open.** Same reasoning as the first
  record: VMware runs guest instructions natively, so the CPU paths are real, but
  the firmware, the APIC and the UART are software models. This machine's
  firmware is the one that produced the ACPI finding, and a vendor's will differ
  again.
- **The datapath drivers were spawned, not driven.** `state 20..31` — the block,
  network and graphics counters — are all zero. Four places were built and four
  components were admitted, restarted and torn down; no device transfer happened,
  because this machine presents no virtio-pci device and, with no DMAR, could not
  have confined one if it had. `cargo xtask blk|net|gpu` remains emulator-only
  evidence.
- **No claim was measured.** `claims/0001` and `claims/0002` need
  `runner-class-A` and `F_ENVIRONMENT` was not set here.
- **The trace hash differs and that is not a determinism failure.** The memory
  map, the module list and the core count are in the boot log by design, which is
  why `xtask` pins `-m 128M -smp 2`. `E0-P02` claims two runs of *one
  configuration* agree.
- **`3 need(s) bound to nothing` is a property of the build, not of this
  machine.** It counts the three drivers' `irq` needs, which this build satisfies
  with a capability naming no object the machine has. QEMU reports three as well.
