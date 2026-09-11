# The third boot outside QEMU: E2 on a machine, and the half it did not run

On 2026-09-09 this kernel booted to `M0 ok` outside QEMU for the third time,
carrying `E2`. The two earlier records are `docs/first-boot-outside-qemu.md` —
one user process — and `docs/second-boot-outside-qemu.md` — four components and
a supervisor. This one is about the state trees, the frame measuring itself on
real firmware, and one thing that did **not** run and had never been noticed.

**It was a VMware machine again, so it is still not bare metal and `E0-P18`
still does not close on it.** It is also not the fourth attempt: on the same day
a VMware guest on a Threadripper 2990WX host stopped inside core bring-up and
never reached this far. That is RFC 0068's record and `E0-P18` carries it; this
page is the machine that worked.

## What ran

A VMware virtual machine on Arch Linux, UEFI firmware, one serial port at COM1,
**32 GiB and 2 vCPUs**. Installed by `tools/f-on-metal.sh`, booting the
optimised image at commit `a0b0ad2`. Five boot modules: `init.bin` and the four
component files.

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
  module        0x00000000005e6000..0x00000000005e63b0  0 KiB
  module        0x00000000005e7000..0x00000000005e89c8  6 KiB
  module        0x00000000005e9000..0x00000000005ed958  18 KiB
  module        0x00000000005ee000..0x00000000005f2b30  18 KiB
  module        0x00000000005f3000..0x00000000005f7f10  19 KiB
  seed          0xf00dbeefcafe1234
  env digest    0x74c02a9580a60021
  env clock     800 ns
  determinism   ok
  frames        261477 free of 261477
  paging        no-execute on, global pages on, pcid available, and deliberately off, direct map in 1 GiB pages
  address space 0x000000000fbff000 root, direct map at 0xffff800000000000
  frame         7c44ec099247f30100e5f8bf036364a2478d32c090583a014d27a654f1e19aa9 over 364544 bytes of text and rodata
  generation    none selected, so no root is published
  reclaimed     8126464 frame(s) above the old identity map
  frame alloc   ok — orders 0..=18, 1035 split, 1563 merged
  frame shards  8 shards, 0 cross-core on the hot path, 21 refill(s), 1 forced
  frame hygiene ok — 0 clean, 8387936 dirty
  local apic    xapic at 0x00000000fee00000, version 0x15, 7 lvt entries
  jitter        ok
  clocks        measured against the 8254 over 10 ms; timer via tsc-deadline
  wall clock    firmware rtc, uncertain to 3600 s
  env contract  arithmetic ok, seeded ok, hardware ok
  bring-up      2 logical processor(s) reported, 1 to start beside this one
  cores         2 of 8 shards, each with its own tables and stacks
  acpi          none: no checksummed root pointer in either window
  capabilities  32 free slots, 128 more per page bought, 5 properties and 5 storage checks hold, 10 flawed tables caught
  ring wrote    "the ring is open" through WRITE_SERIAL
  ring          16 entries in 4096 B, 2176 B arena, two ends at ABI v1, 4 published with one store, 1 refused, forged slot caught, hostile header refused
  doorbell      KernelIpi, 1 delivered, 500 per 1000 operations, a draining consumer was not rung
  supervisor    4 place(s) from 4 component file(s), one per file, each staked with an account its own manifest sized
  place         store: private, on_fault, 3 restart(s) in 3000 tick(s), 131072 B account
  admission     soft class, 65536 B footprint + 36864 B of declared need(s) against a 131072 B account — refused before anything is spent, never after
  spawn         place store epoch 0 — manifest 0x27ce995f37bd654e, 5 need(s) supplied, type, rights and quantity checked; 13 frame(s) from the account; control ring 4096 B
  state mount   place store -> frame root slot 0: root store, 5 node(s), snapshot 0x40d69e0cf0f65c45, read back through the root and not from what was written
  place         virtio-blk: private, on_fault, 8 restart(s) in 60000 tick(s), 4194304 B account
  admission     soft class, 2097152 B footprint + 81920 B of declared need(s) against a 4194304 B account — refused before anything is spent, never after
  spawn         place virtio-blk epoch 0 — manifest 0x0fdc9e16920e2ad5, 4 need(s) supplied, type, rights and quantity checked; 27 frame(s) from the account; control ring 4096 B
  state mount   place virtio-blk -> frame root slot 1: root blk, 5 node(s), snapshot 0x40d69e0cf0f65c45, read back through the root and not from what was written
  place         virtio-gpu: private, on_fault, 8 restart(s) in 60000 tick(s), 4194304 B account
  admission     soft class, 2097152 B footprint + 81920 B of declared need(s) against a 4194304 B account — refused before anything is spent, never after
  spawn         place virtio-gpu epoch 0 — manifest 0x4c41b2f3c349eb2e, 4 need(s) supplied, type, rights and quantity checked; 28 frame(s) from the account; control ring 4096 B
  state mount   place virtio-gpu -> frame root slot 2: root gpu, 5 node(s), snapshot 0x40d69e0cf0f65c45, read back through the root and not from what was written
  place         virtio-net: private, on_fault, 8 restart(s) in 60000 tick(s), 4194304 B account
  admission     soft class, 2097152 B footprint + 81920 B of declared need(s) against a 4194304 B account — refused before anything is spent, never after
  spawn         place virtio-net epoch 0 — manifest 0x571f2e14faabd7b0, 4 need(s) supplied, type, rights and quantity checked; 28 frame(s) from the account; control ring 4096 B
  state mount   place virtio-net -> frame root slot 3: root net, 5 node(s), snapshot 0x40d69e0cf0f65c45, read back through the root and not from what was written
  connect       client -> place store: a channel opened, header epoch 0
  fault         place store epoch 0 stopped speaking: its control ring header no longer validates
  teardown      5 capabilit(ies) revoked of 32 slot(s), 13 frame(s) refunded to the account, 1 peer-gone notice(s)
  connect       client -> place store: the place is empty, the connect pends to a deadline 200 tick(s) out
  outcomes      a connect whose own deadline had passed earned PEER/EMPTY, which is not GONE: the place may yet be refilled
  refusals      6 spawn(s) refused on purpose: 5 way(s) a supply can be wrong — missing, undeclared, wrong type, short rights, short quantity — and a manifest declaring no state tree, refused ADMISSION/NO_STATE_TREE
  restart       place store under on_fault — restart 1 of 3, backoff 8 tick(s)
  spawn         place store epoch 1 — nothing carried over: new table, new memory, new control ring, new state tree
  state mount   place store -> frame root slot 0: root store, 5 node(s), snapshot 0x40d69e0cf0f65c45, read back through the root and not from what was written
  resume        the pending connect completed: a channel to epoch 1, and the client observed only the wait
  stop          place store epoch 1 stopped against a deadline already behind it — a kill, and a second stop could not move it later
  teardown      5 capabilit(ies) revoked of 32 slot(s), 13 frame(s) refunded to the account, 1 peer-gone notice(s)
  retire        place store spent its budget of 3 restart(s) — retired, and 1 peer-gone notice(s) went to the endpoint's holders
  outcomes      a connect to a retired place earned PEER/GONE, arriving and already waiting: the place is not coming back
  budget        a window 3000 tick(s) wide: 3 restart(s) inside it retires the place, and the same count once it has elapsed does not
  teardown      place virtio-net — 4 capabilit(ies) revoked, 28 frame(s) refunded to its own account, 1 peer-gone notice(s)
  teardown      place virtio-gpu — 4 capabilit(ies) revoked, 28 frame(s) refunded to its own account, 1 peer-gone notice(s)
  teardown      place virtio-blk — 4 capabilit(ies) revoked, 27 frame(s) refunded to its own account, 1 peer-gone notice(s)
  notices       301 published in slot-then-stop-then-grade order over 30 round(s), 301 drained back at a polling point as 6 of 7 kind(s), 0 still owed
  supervisor    ok — 4 place(s), 5 spawn(s), 1 fault(s), 1 restart(s), 1 resumed, 0 client(s) lost, 9 probe(s) refused, 1 retired, 3 need(s) bound to nothing, 5 tree(s) mounted carrying 25 node(s), 1 refused for declaring none
  state tree    51 nodes, snapshot 0x33d33cffb1cca645, stable across a re-read
  state           1  frame = 0
  state           2  memory = 0
  state           3  total = 8387941
  state           4  free = 8387932
  state           5  topology = 0
  state           6  started = 2
  state           7  ring = 0
  state           8  executed = 4
  state           9  refused = 1
  state          10  caps = 0
  state          11  slots = 32
  state          12  served = 10843
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
  state          32  components = 0
  state          33  mounted = 0
  state          34  published = 5
  state          35  nodes = 25
  state          36  refused = 1
  state          37  place0 = <empty>
  state          38  place1 = <empty>
  state          39  place2 = <empty>
  state          40  place3 = <empty>
  state          41  generation = 0
  state          42  counter = 0
  state          43  root0 = 0x0000000000000000
  state          44  root1 = 0x0000000000000000
  state          45  root2 = 0x0000000000000000
  state          46  root3 = 0x0000000000000000
  state          47  frame0 = 0x01f3479209ec447c
  state          48  frame1 = 0xa2646303bff8e500
  state          49  frame2 = 0x013a5890c0328d47
  state          50  frame3 = 0xa99ae1f154a6274d
  state          63  reserved = <kind 238 not named by this build>
  process       layout ok, sysret selectors agree
  init          944 bytes from boot module 1 of 5
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

## Checked against the same commit under QEMU

The comparison is the point of keeping the log, so it was made rather than
asserted: `a0b0ad2` was built in a scratch worktree and booted here, and the two
logs were diffed line by line.

**Every machine-independent line is byte-identical.** The supervisor summary,
`notices 301 published … 301 drained back … 0 still owed`, `refusals 6
spawn(s)`, the capability line, the ring line, the doorbell line, `user caps`,
`user frames`, `timer`, `process layout ok`, `init process`, and `state tree 51
nodes`. That last one is also what identifies the image: this is that commit.

Five structural differences, and every one of them is accounted for.

| difference | why |
| --- | --- |
| `acpi none`, and therefore no `pci`, no `iommu vt-d`, no `iommu caps`, no `iommu walks`, no `iommu on`, and `state 17 domains = 0` | multiboot 1 has no field for the ACPI root pointer and UEFI leaves none in the legacy windows. The second boot found this; `E5-D03` owns it |
| `generation none selected, so no root is published`, `state 41`–`46` zero, and no `measurement agrees…` line | the entries `f-on-metal.sh` writes carry no `f.root=`. See below |
| five boot modules rather than six | the sixth under QEMU is the packed generation, same cause |
| `reclaimed 8126464 frame(s) above the old identity map` appears only here | QEMU boots at 128 MiB, so the pass above the 1 GiB identity window has nothing to add. The first boot outside QEMU is where that pass first did anything at all |
| the frame hash, and `state 47`–`50` | hardware runs the optimised image — 364 544 bytes of text and rodata — and `cargo xtask run` the debug one, at 1 118 208. Different binaries, so different measurements |

The state snapshot differs — `0x33d33cffb1cca645` here against
`0x4188e3c09c845928` under QEMU — and `E0-P18`'s exit says it **must**. It is
not a determinism failure: the tree carries this machine's frame count, its
allocator counters and its generation roots, and two machines that produced one
snapshot would mean the tree had stopped reading what the run publishes.

## What this boot ran that the second one could not

- **The frame measured its own text and rodata against real firmware**, and
  printed the result: `7c44ec09…` over 364 544 bytes. `E2-B07` had only ever
  measured itself under an emulator.
- **The state trees mounted.** Five trees, twenty-five nodes, each read back
  *through the root* rather than from what was written — `E1-B15` and RFC 0065,
  on real page tables.
- **A manifest declaring no state tree was refused**, `ADMISSION/NO_STATE_TREE`,
  which brings the deliberate refusals on this machine to six.
- **The ring-3 provocation was answered by a real MMU again**: `exception 14 at
  0xffff800000000000, error 0x5`.
- **`bring-up 2 logical processor(s) reported, 1 to start beside this one`** is
  RFC 0068's census line, printed ahead of the stage rather than after it. This
  machine needed nothing it offers — two reported, two started, none held — and
  that is worth recording, because a line that only ever appears on a broken
  machine is a line nobody has checked on a working one.

## The finding: a machine installed by the script cannot say which generation it is

`generation none selected, so no root is published`, and `state 42 counter = 0`
with all four roots zero.

This is not a fault in the boot. It is the entries: `tools/f-on-metal.sh` wrote
a `multiboot` line with no `f.root=` and no `f.frame=` on it, so the frame
measured itself, had nothing to compare against, and correctly published
nothing. RFC 0012 reserves zero for exactly this — *no root describes this
machine at this instant* — and the frame said so rather than inventing a number.

What makes it worth a section is what it means for `E2`. **Selection and the
declaration comparison are the halves of RFC 0012 that hardware had never run**,
in three boots outside QEMU, and nothing said so: `cargo xtask attest` and every
QEMU boot carry both tokens, so the paths are exercised constantly on an
emulator and had been exercised nowhere else. A gate that is green everywhere it
is measured and unmeasured where it matters is the shape `E0-P18` exists to
find, and this is the third time this task has found one.

Both tokens or neither, never one: `HalfADeclaration` in `kernel/src/measure.rs`
refuses a command line naming a generation without the frame it was compiled
against, because half of that statement is not a weaker statement but a
different one. So the fix could not be "add `f.root=`" — the script needs the
frame hash too, and that comes out of a module's own record tree, which is a
format and not a filename.

It is fixed in the same change as this record. `cargo xtask generation
--install` now also writes `generations.tsv` — the roots and frame hashes, and
no layout — and `f-on-metal.sh install --generations` reads it, installs the
modules, and writes one entry per generation resolved against **this** machine's
GRUB path. That split is deliberate and is argued at `MENU_DATA` in
`xtask/src/generation.rs`: the fragment that command already emitted hardcodes
`/boot/f/`, which is wrong on a machine whose `/boot` is its own partition, and
`docs/postmortem/0001` is the record of `--install` having shipped that exact
failure once already.

## What this did not establish

- **Bare metal.** A VMware guest is not the machine `claims/runner-class-A.md`
  describes, and `E0-P18` stays open.
- **The IOMMU.** Unreachable under UEFI by construction, `E5-D03`.
- **Two machines on two dates.** `E2-P06`'s clause needs a second host; this is
  one host on one day.
- **That the generation half now works on hardware.** The fix is tested under
  QEMU and staged with `DESTDIR`; nobody has yet booted a machine from an entry
  this script wrote with `f.root=` on it. That is the next boot's job, and it is
  the first thing to check when there is one.
