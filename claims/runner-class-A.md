# `runner-class-A` — the machine a claim may be taken on

`claims/*.toml` name this class in their `[hardware]` block, and
`MEASUREMENT_ENVIRONMENTS` in `bench/src/lib.rs` names it as the one value of
`F_ENVIRONMENT` that is allowed to record a timing measurement. Until E0-D10
that was the whole of it: a name in two places and a parenthesis reading
*pinned bare metal, thermally stable*. This file is the specification behind the
name.

It exists because a machine that is only a name cannot be obtained. Every epoch
adds claims to the same queue — `E0-P05` and `E0-P06` are waiting on it now,
`E1-P10` adds four more — and a queue whose head is a phrase in prose is a queue
that never moves.

## What this file is, and what it is not

It is a specification complete enough that a stranger could assemble an
equivalent machine and defend a number taken on it.

It is **not a machine**. Nothing in this repository can buy hardware, and the
half of `E0-D10`'s exit that reads *a machine exists* is not met by a file. What
a file can do is make the purchase decidable, and make the alternative —
somebody setting `F_ENVIRONMENT=runner-class-A` on a laptop — a thing a reviewer
can catch by reading one page.

That failure mode is the reason for the whole apparatus and it is worth naming
plainly: **the environment variable is an assertion, not a measurement.** No
code in this tree can tell a class-A machine from a workstation with the
variable set by hand. What the tree can do is refuse everything else by default
(`bench::Environment::classify` fails closed on an unset or unrecognised value),
and require that the record of *how each reservation component was obtained*
travels with every number, per RFC 0007. A number whose record says "by
exclusion" for a component this file requires by partition was not taken here.

## What the machine must be able to do

RFC 0007 fixes the requirement, and it is not a wish-list: a granted hard-class
reservation holds four things, and *admission tests all four together or it is
testing nothing*. A machine that cannot supply all four cannot host a claim
about this system, because the claim is a statement about what the core was
protected from.

| Component | How it is obtained here | Mechanism |
| --- | --- | --- |
| A physical core | **by partition** | `isolcpus` plus both SMT siblings taken as a unit |
| Memory bandwidth | **by partition** | `resctrl` MBA (Intel RDT) or SMBA (AMD) |
| Last-level cache | **by partition** | `resctrl` CAT (Intel) or L3 QoS (AMD) |
| Pre-faulted memory | **by partition** | 1 GiB hugepages, populated and locked |

All four **by partition** is what makes this class-A. RFC 0007 permits
partitioning *by exclusion* — holding the co-resident cores idle — where the
hardware cannot partition a resource, and that remains a legitimate reservation.
It is not this class. A machine without MBA or CAT is a different class and
needs a different name in `MEASUREMENT_ENVIRONMENTS` before it records anything,
with its own file beside this one. Reusing this name for it is how the class
decays into whatever hardware was to hand, which is the decay the registry
exists to prevent.

## Hardware

Stated as required capabilities with one worked example, because a single part
number ages out and a capability list does not. Anything that satisfies the
capability column is an equivalent machine.

| Requirement | Why | Worked example |
| --- | --- | --- |
| Server-class x86-64 with RDT/QoS: CAT **and** MBA | The two components above that consumer silicon does not offer. Intel dropped CAT from client parts after Skylake-X; this is the single constraint that rules out a desktop | Intel Xeon Silver 4410Y (Sapphire Rapids) or AMD EPYC 9124 (Genoa) |
| Single socket | A NUMA crossing is a variable the claim did not intend to measure, and a two-socket machine makes every number a question about which socket | one populated socket, second empty or absent |
| Invariant TSC, TSC-deadline, x2APIC | `claims/0002` is about the APIC timer, and `kernel/src/env.rs` reads the one legitimate `rdtsc`. QEMU's TCG backend refuses `tsc-deadline` and `x2apic` by name, which is why a container run measures the emulator | any part in the two families above |
| 1 GiB pages | Pre-faulted memory in huge pages, per RFC 0007. 2 MiB works and leaves more TLB pressure in the measurement than the design intends | `pdpe1gb` in `/proc/cpuinfo` |
| SMT present and enabled | So that "both siblings are held or the pair is not offered" is a property the machine can demonstrate rather than one disabled into vacuity | 2 threads per core |
| ECC memory, one DIMM per channel, all channels populated | An uncorrected bit flip inside a 60-second jitter run is a number nobody can explain; an unbalanced channel population is a bandwidth partition that does not mean what it says | 4× or 8× DDR5 RDIMM, identical |
| IOMMU | `E1-B01` needs it, and a claims machine that has to be re-specified one epoch later is this task done twice | `intel_iommu=on` / AMD-Vi |
| Cooling with headroom, fixed fan curve | *Thermally stable* is the phrase the class has carried since it was only a phrase. It means the 600th second of a run is the same machine as the first | tower cooler rated above TDP, no thermal throttling under sustained all-core load |
| No other tenant, no hypervisor | The class is bare metal. A virtual machine on a shared host cannot produce defensible tail latency, which `WHY_CI` already says | the machine, and nothing else |

## Firmware

Set before the kernel command line, because several of these cannot be undone
from an operating system.

- **Turbo / boost: off.** A frequency that varies with temperature and with how
  many cores are busy makes every number a distribution over machine states
  rather than over the code. This is the setting that matters most and it is the
  one people forget.
- **Uncore / Infinity Fabric frequency: fixed**, not governed.
- **C-states: C1 maximum.** Deeper states are exit latency inside a deadline.
- **P-states: fixed**, or left to the kernel with `intel_pstate=disable` below —
  one or the other, never both half-done.
- **SMT: enabled.** See above.
- **Hardware prefetchers: left at their defaults, and recorded.** Turning them
  off makes numbers cleaner and less true.
- **Secure Boot: off**, because the kernel under test is not signed.
- **Watchdog timers: off.**

## Kernel command line

The reservation is carved by the host Linux for the baseline runs and by F's own
frame for F's; the isolation below is what makes the *baseline* half honest, and
what keeps a stray kernel thread out of a core F has reserved.

```
isolcpus=domain,managed_irq,4-15
nohz_full=4-15
rcu_nocbs=4-15 rcu_nocb_poll
irqaffinity=0-3
intel_pstate=disable cpufreq.default_governor=performance
processor.max_cstate=1 intel_idle.max_cstate=0
tsc=reliable clocksource=tsc
nmi_watchdog=0 nosoftlockup
transparent_hugepage=never
default_hugepagesz=1G hugepagesz=1G hugepages=16
intel_iommu=on iommu=pt
audit=0
skew_tick=1
mitigations=auto
```

This block is prose: it can be read and it cannot be run, so it cannot be found
to have stopped being true.
`claims/baselines/linux-6.x-tuned/cmdline.txt` is the same list as data, and it
is the copy that gets applied and checked — `E1-D06`. `verify.sh` beside it
compares the two whenever it can reach a checkout, because two copies of one
list with nothing comparing them is how a machine ends up being neither.

Cores `0-3` are the housekeeping set and take every interrupt, every RCU
callback and every kernel thread. Cores `4-15` are the measurement set and are
what a reservation is granted out of. The two lists must be stated as sibling
pairs on the machine's own topology, read from
`/sys/devices/system/cpu/cpu*/topology/thread_siblings_list` — the numbering
above is an example and is wrong on a machine whose enumeration differs, which
is most of them.

**Speculative-execution mitigations are left on, and recorded with the number.**
This is a judgement rather than an oversight, so the reasoning is here: the
baseline is *"tuned Linux, configured by someone trying to win"*, and the only
requirement that makes a comparison mean anything is that the setting is
**identical on both sides**. Turning them off flatters both sides and describes
a machine nobody deploys. *Reversal:* a claim whose statement is explicitly
about mitigation cost, in which case the setting moves into that claim's
`[baseline]` block and is stated there rather than here.

## Reserving, per component

Read as the recipe a reservation follows on this machine, and as what a
measurement's RFC 0007 record must be able to say.

**A physical core, by partition.** Take both entries of a
`thread_siblings_list` or neither. A schedulability test that counts threads has
already conceded the tail it was run to bound. The housekeeping set never
overlaps a reserved pair.

**Memory bandwidth, by partition.** `resctrl`, mounted at `/sys/fs/resctrl`. A
control group per reservation, with an `MB` value in its `schemata`. Batch work
is throttled to fit around it. Verify the machine actually has it —
`grep -o 'mba\|mbm_total' /proc/cpuinfo` on Intel, `MBA` in
`/sys/fs/resctrl/info/` on either — because a mount that succeeds without the
feature gives a control group that silently limits nothing.

**Last-level cache, by partition.** The same `resctrl` group, with an `L3` mask.
Masks must not overlap between the measurement group and the housekeeping group;
an overlapping mask is a partition in name only.

**Pre-faulted memory, by partition.** 1 GiB pages reserved at boot, mapped with
`MAP_POPULATE` and `mlock`ed. Faulted, not merely allocated — RFC 0007 is
specific — so that no fault, no tier migration and no compaction pass can land
inside a deadline. `transparent_hugepage=never` so nothing is promoted or
compacted underneath. No swap.

The tuned-Linux baseline turns transparent huge pages back on at run time, for
its half of a comparison only, and
`claims/baselines/linux-6.x-tuned/baseline.conf` argues for the asymmetry
beside the setting. The short form: F maps its own 1 GiB pages and never uses
Linux's, so `never` is right here — and a baseline handed 4 KiB pages on that
account would be a machine nobody configures, so `always` is right there. The
asymmetry raises the baseline, which is the direction a reader who suspects
this apparatus of flattering F should check first.

## Verifying the machine is what it says

A stranger should be able to answer all of these before setting the environment
variable. The list is here rather than in a script because the script is
`E0-R02`'s to write, and a checklist that is honest is worth more now than a
script that is not written yet.

1. `lscpu` reports one socket, two threads per core, and a fixed `CPU MHz`
   across a sustained all-core load.
2. `/proc/cpuinfo` flags include `tsc_deadline_timer`, `x2apic`,
   `constant_tsc`, `nonstop_tsc`, `pdpe1gb`.
3. `/sys/fs/resctrl/info/L3/cbm_mask` and `/sys/fs/resctrl/info/MB/` both exist.
4. `/proc/meminfo` shows `Hugepagesize: 1048576 kB` and the reserved count.
5. `cat /sys/devices/system/cpu/cpu4/topology/thread_siblings_list` names a pair
   that is entirely inside the measurement set.
6. `dmesg | grep -i 'thermal\|throttl'` is empty after a sustained load.
7. The machine is not a guest: no hypervisor flag in `/proc/cpuinfo`.

Then, and only then, `F_ENVIRONMENT=runner-class-A`.

## What would change this file

A claim that needs something this machine does not have. `E1-P10` adds four
claims about the datapath, and a datapath claim needs a device — at which point
the network card, its queue count and its interrupt routing join the table
above, and this file gets a second worked example rather than a rewrite.

`E5-D01` names the phase-05 workstation and is the same task three releases
later, for a different purpose: that one is a bill of materials for graphics
work, this one is a bill of materials for defensible tails. When both exist,
whether they should be one machine is a question worth asking once.
