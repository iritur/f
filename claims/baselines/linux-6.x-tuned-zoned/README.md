# `linux-6.x-tuned-zoned` — the zoned baseline, as configuration

`claims/0016-write-amplification.toml` says its number must be *at most what a
tuned Linux filesystem writes on the identical zoned device*. This directory is
that filesystem, and that device.

It is the second baseline `E1-D06`'s rule has had to configure — *a baseline is
configuration in the tree or it is prose that decays* — and it is a sibling of
`claims/baselines/linux-6.x-tuned/` rather than an edit to it, for the reason
that directory's own reversal condition gives: a baseline is versioned with the
claim that cites it, and a claim needing something a configuration does not have
gets a new directory.

**Read this first: nothing here has been run, and the reason is not that nobody
tried.** The last section says exactly what is missing, what was measured rather
than assumed, and what it would cost. `claims/0016` is `pending` and stays
`pending`; there is no number in this directory and there is none in that file's
`ratio_vs_baseline` row.

## What is in here

| File | What it is |
| --- | --- |
| `device.conf` | The device both halves are measured on. Geometry, zone model, where it comes from, and the QEMU below which it cannot exist. |
| `filesystem.conf` | Which filesystem, how it is made, how it is mounted, and which mount options are forbidden. |
| `sysfs.conf` | The runtime knobs, keyed by the path they name: `queue_*` under `/sys/block`, `f2fs_*` under `/sys/fs/f2fs`. |
| `sysctl.conf` | The writeback and swap settings, with units. A real sysctl file. |
| `comparison.conf` | What both halves must do identically for the ratio to be a ratio. |
| `apply.sh` | Puts a guest into this configuration. Refuses a device that is not the one `device.conf` describes, and never runs `mkfs` without `--format`. |
| `verify.sh` | Says whether a guest is still in it, and exits non-zero naming the drift. |
| `lib.sh` | The one reader both scripts use, and the check that every line of every data file is a line one of them understands. |

## Why this is a different baseline and not the other one with a disk in it

The sibling directory tunes a machine for *latency*: isolated cores, a fixed
frequency, shallow idle states, interrupts moved somewhere else, real-time
scheduling without a throttle. Almost none of that changes how many bytes reach
a device. Copying it across would have produced a page of settings this claim's
number cannot be sensitive to, which is the same failure as a setting nobody
applies: it reads as rigour and checks nothing.

What this number *is* sensitive to is a different list, and it is the list in
these files: how much spare room each side is given, when a dirty page becomes a
device write, which victim a collector picks, where the durability point is, and
what the device's zones and blocks are.

The other difference is structural. The sibling runs on the bare-metal runner.
This one runs **inside a guest**, because the device is an emulated zoned
virtio-blk and the thing being compared against is a Linux that has to be on the
same device. So `apply.sh` and `verify.sh` are run in the guest, and the counter
they are configuring for is read from outside it.

## What "configured by someone trying to win" means here

Each of these is something the baseline gets that would be easy to quietly not
give it, and each is argued beside its value in the file it lives in:

- **f2fs in zoned mode, not ext4 over `dm-zoned`.** The peer is the mainline
  log-structured filesystem that appends, seals and garbage-collects, which is
  the same three verbs `zone/src/map.rs` and `zone/src/collect.rs` implement. A
  conventional filesystem over a shim would move the amplification into the shim
  and make the comparison meaningless in the baseline's favour.
- **Cost-benefit victim selection**, `f2fs_gc_idle = 2`. Greedy — take the
  segment with the fewest valid blocks — is F's own ranking under RFC 0059, and
  is the reason the modelled copy-forward term came out at 0.1225 rather than
  0.33. Giving the baseline greedy would be choosing its algorithm to match F's;
  cost-benefit additionally weights age and copies less on any workload with age
  skew.
- **A long victim search**, four times the default. This claim counts bytes, so
  search cost is free in the metric being measured and buys emptier victims.
- **Writeback held as long as the kernel will hold it.** More dirty pages
  absorbed in memory is fewer bytes at the device, and `comparison.conf` makes
  that a delay rather than an exclusion by requiring the counter to be read after
  `sync`.
- **Small files inlined**, `inline_data` and `inline_dentry`, and flushes merged.
- **Background GC on**, because a baseline measured with its collector off is a
  baseline that has not yet been asked to reuse anything.

## What is deliberately *not* tuned, and what it costs the baseline

R12: a cost is stated rather than hidden in the metric. Four settings here make
the baseline write more, and every one of them is the setting that makes the two
halves answer the same question:

| Setting | What it costs the baseline | Why it is not negotiable |
| --- | --- | --- |
| `fsync_mode=strict` | more metadata per durability point | F's publish is durable when it returns; f2fs's default `posix` fsync persists less than that |
| barriers on | a device flush per durability point | F issues `FLUSH` and the modelled cycle counts 138 of them |
| `compress` absent | every byte written uncompressed | F does not compress; a compressing baseline compares a compressor against a mapping |
| `overprovision_percent = 20`, the same on both sides | no extra spare room | spare room is the input write amplification is most sensitive to, so a side given more of it wins by arithmetic |

And one setting costs *F* rather than the baseline, which is here because a
comparison should err against the system making the claim:
`logical_block_bytes = 4096`. A record in F's store is padded to the logical
block, so the modelled fill term of 1.0111 — taken on a 512-byte block — gets
worse on this device by up to eight times the padding per record. 4096 is what
shipping zoned SSDs report and what f2fs uses internally whatever the device
says.

## Which claim compares against it

| Claim | How it uses this |
| --- | --- |
| `claims/0016-write-amplification.toml` | Directly, and only. `ratio_vs_baseline = { max = 1.0 }` is a ratio against a number taken in a guest in this configuration, on the device `device.conf` describes, and `[baseline] path` names this directory. That row has no number. |

## Applying it

`F_ZONED_DEV` and `F_MOUNT` have no defaults and never will. `apply.sh --format`
runs `mkfs` on whatever the first names, so a default would be a filesystem
created on whichever block device the author of this directory happened to have.

Inside the guest:

```bash
cd claims/baselines/linux-6.x-tuned-zoned
./apply.sh --dry-run                                       # every write, and none of them
sudo F_ZONED_DEV=/dev/vdb F_MOUNT=/mnt/baseline ./apply.sh
sudo F_ZONED_DEV=/dev/vdb F_MOUNT=/mnt/baseline ./apply.sh --format
```

`apply.sh` exits `2` when every knob is in force and there is no filesystem,
which is the expected result of the first run: making one is destructive and
needs `--format` in the same command line.

It **refuses a device whose geometry is not the one `device.conf` names**, and
writes nothing when it does. That refusal is the most important line in the
script: a baseline fully applied to the wrong geometry produces a number nobody
can tell from a good one.

Both scripts are committed executable. If the copy in front of you arrived
without the mode, run them as `bash ./apply.sh`.

## Verifying it applied

```bash
F_ZONED_DEV=/dev/vdb F_MOUNT=/mnt/baseline ./verify.sh; echo $?
```

One line per setting, `[ok]` or `[--]`, and a non-zero exit naming everything
that drifted. It needs no privilege.

It checks three things `apply.sh` does not, and all three are copies of a fact
that also lives somewhere else:

1. **That `claims/0016` still points here.** After a re-tune there are two
   directories and one claim, and the failure that produces is silent: the new
   directory is the one being verified and the old one is the one being compared
   against.
2. **That `comparison.conf` still agrees with `zone/tests/cycle.rs`.** The
   workload is the half of this baseline most likely to move without anybody
   noticing, because it moves when a *test* is edited rather than when a baseline
   is. Four constants are compared — the two object sizes, the retention period
   and the seed — and `BLOCK_BYTES` is deliberately excluded, because the
   modelled cycle's 512 and this device's 4096 are an argued difference rather
   than a drift.
3. **That the emulator can present a zoned virtio-blk at all.** Below the QEMU
   `device.conf` names there is no device, so neither half can run whatever the
   tuning says.

On a machine with no zoned device it prints the whole report and exits non-zero.
That is the intended output there, and it is what this directory produces today
on every machine this project has.

## What is missing, measured rather than assumed

This is the section that matters, and it is here rather than in a commit message
because `claims/0016` will be `pending` for longer than anybody's memory of why.

**The emulator has no zoned virtio-blk.** The development image is Debian
bookworm's QEMU 7.2, and 7.2 does not have the feature at all — not a worse
version of it. Measured, in the container, at `E2-P10`:

```
$ qemu-system-x86_64 -device virtio-blk-pci,help | grep -i zone
                                                       (no output)
$ qemu-system-x86_64 -blockdev driver=raw,...,file.zoned=host
qemu-system-x86_64: ... Parameter 'file.zoned' is unexpected
```

The same QEMU *does* emulate zoned NVMe — `-device nvme-ns` carries the whole
`zoned.*` family — and that is worth writing down precisely because it is the
tempting wrong answer: a baseline on an emulated ZNS namespace and an F on a
virtio-blk would be two systems on two devices, and `claims/0016`'s sentence is
*on the identical device*. F has no NVMe driver, so there is no version of that
comparison that is one comparison.

`docker/README.md` names the fix — one line, `BASE=debian:trixie-slim` — and
`E2-P10` could not run it: the development container has no network egress. Also
measured: `curl http://deb.debian.org/debian/` fails to connect, and `cargo`
could not reach `static.crates.io` until the registry volume was seeded from an
already-warm one. An image build needs the base image, `apt` and `rustup`, and
none of the three is reachable from here.

**And the device would still not be free after that.** `device.conf` records the
question this could not settle: whether QEMU can synthesise a zoned virtio-blk
from a plain file, or whether its zoned support is passthrough of a host zoned
device only. On the second reading — which is what `backing = null_blk` assumes,
because it is the route that works either way — the host needs
`CONFIG_BLK_DEV_ZONED` and a privileged `modprobe null_blk zoned=1`. Under
Docker Desktop that is the WSL2 kernel, and this container is not privileged.

**The other half has nothing to measure yet either, and this is the larger
debt.** `ratio_vs_baseline` needs a numerator, and F has no boot that drives a
zoned device. `user/virtio-blk` numbers RFC 0060's five zone opcodes and answers
two of seven; `grep -rn zoned xtask/src` finds one doc comment and no verb; and
`user/objects/tests/reads.rs` says in its own header that it is the host over a
modelled zoned device and that no QEMU boot has run. So a trixie image would buy
a device with nothing on it.

**What it would cost, honestly.** Four things, in the order they unblock each
other, and only the first is small:

1. the image base change and a rebuild, on a machine with network. One line,
   one commit, one image digest through every reproduction.
2. a host zoned device — `null_blk zoned=1` on a kernel that has it, or real
   SMR/ZNS hardware — and a privileged container or a runner that is not a
   container. This is the step that cannot be paid inside Docker Desktop.
3. a guest for the baseline half: a Linux with `f2fs-tools` new enough for
   zoned `mkfs`, a root filesystem, and a way in and out of it. `blkzone` is in
   the development image; `mkfs.f2fs` is not.
4. F's own boot over zoned virtio-blk — the remaining five of RFC 0060's seven
   opcodes and a harness that reads `query-blockstats` over QMP. This is
   `E2-B02`'s boot, and it is the one that is measured in days rather than in
   commands.

Until all four exist, a number in `ratio_vs_baseline` would be a number taken
against something other than what this directory describes, and that is worse
than an empty row: an empty row is a debt and a wrong row is evidence.

## The reversal condition

**When this baseline is re-tuned, it becomes a new versioned directory beside
this one. It is never edited in place.**

The same rule as the sibling directory's, for the same reason: a number recorded
against a configuration is a number against *that* configuration, and editing
the directory afterwards silently re-dates every number that cited it.

This directory has a way to trip that rule its sibling does not, and it is worth
naming before it happens. Nothing here has been measured, so the first real run
will find something wrong — a knob that does not exist on the kernel that runs,
a geometry `mkfs.f2fs` refuses, a victim policy that turns out to copy more.
**Fixing those is an edit to this directory and not a new one**, because no
number has ever been taken against it: the rule protects published comparisons,
and there are none. The day `claims/0016` publishes a ratio, that stops being
true, and this paragraph is what a reader should check before assuming which
side of that line they are on.

The three ordinary triggers for a new directory afterwards:

- **The kernel range in `device.conf` no longer holds.**
- **The device changes.** A different zone size, a capacity below the zone size,
  a different logical block, real hardware instead of an emulated device — each
  is a different denominator, and `emulated = true` in `claims/0016`'s
  `[hardware]` block is the row that says which kind this one is.
- **A tuner would now do something these files do not.** `A-04`'s other half,
  once per epoch, and the one that decays invisibly because the checks stay green
  the whole time.

What would reverse the *directory*: F ceasing to be compared against a
filesystem at all. `claims/0016`'s `ratio_vs_baseline` is the only row that
needs this, and its threshold — `max = 1.0` — is the sentence *this store writes
no more than a tuned Linux filesystem does on the same device*. If that sentence
ever stops being one this project wants to make, the honest move is to withdraw
the row rather than to keep a baseline nothing compares against.
