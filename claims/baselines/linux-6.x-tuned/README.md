# `linux-6.x-tuned` — the baseline, as configuration

`claims/0001-ring-submit-latency.toml` says its number must be *at least 5x
less than an equivalent system call on a tuned Linux baseline*. This directory
is that baseline. Not a description of it: the files here are applied to a
machine by a script, and checked on a machine by another script, and the
difference between those two sentences is the whole reason `E1-D06` exists.

Before this, the baseline was one line of prose in `claims/0001`:

> io_uring enabled, sched_ext policy chosen for the workload, huge pages on,
> device features enabled. Configured by someone trying to win.

Every clause of that is a decision somebody would have to make again, the same
way, on a machine nobody can inspect afterwards. `RELEASING.md` names this as
the release content that **decays silently** — a tuned comparison becomes a
stock comparison as the baseline ages and nobody re-checks it — and prose
cannot be re-checked, because it cannot be run. That is the argument for a
directory of files, and it is also the standard this directory has to be held
to: every claim made here is either a line in a data file or it is prose that
will rot like the sentence it replaced.

## What is in here

| File | What it is |
| --- | --- |
| `cmdline.txt` | The kernel command line, one token per line. Nothing applies it; `apply.sh` checks it and prints what is missing. |
| `sysctl.conf` | Every sysctl, with its unit and its cost. A real sysctl file: `sysctl -p sysctl.conf` applies it. |
| `baseline.conf` | The settings that are neither a boot parameter nor a sysctl — governor, scheduling policy, transparent huge pages, irqbalance, the kernel version range, and the two sysctls whose *absence* satisfies this baseline. |
| `apply.sh` | Puts a machine into this configuration. Idempotent. Never edits a bootloader. |
| `verify.sh` | Asks a machine whether it still is this configuration. Exits non-zero naming every setting that drifted. Needs no privilege. |
| `lib.sh` | What both scripts have to know: how to read the three data files. Sourced rather than copied, because two parsers for one format is the same decay one level down. |

Three data files rather than one, because there are three interfaces — a
reboot, `sysctl -w`, and a write to sysfs — and a file that could hold only one
kind of setting would push the other two into script bodies, where a setting
stops being data and becomes something you have to read code to find.

**Everything is a number with its unit**, including the ones whose unit is *an
enumeration* and the ones whose unit is *a count*: `kernel.sched_rt_runtime_us
= -1` and `kernel.perf_event_paranoid = 0` and `vm.swappiness = 0` are three
different kinds of value that all look like small integers, and R03 is the rule
that quantities state their unit.

**A knob the kernel does not have is drift** (R04), and there are exactly two
exceptions, listed in `baseline.conf` with the argument beside them: a sysctl
that exists if and only if its feature was compiled in is one whose absence
means the feature cannot run, which is a configuration and not a gap. Nothing
joins that list without the same argument written down. `kernel.io_uring_disabled`
never does — a kernel that lacks it cannot say whether io_uring is on, and that
is exactly what the version floor is for.

**A line in these files that `lib.sh` cannot read is an error too**, checked
before either script does anything, and it is R04 aimed at this directory
rather than at the machine. A setting misspelled here — a capital letter in a
sysctl name, a missing `=`, `governer` for `governor` — is a setting `apply.sh`
never applies and `verify.sh` never checks, and both would still exit `0`. That
is the same "looks configured, measures nothing" failure the isolated-core
check guards against, arriving through the data files instead of the kernel.

**The kernel version is a range, in one place**, `baseline.conf`, with the
reason beside it: 6.6 because that is the release `kernel.io_uring_disabled`
first existed in, so a check for it can fail rather than merely be unable to
tell; below 7.0 because every other name in these files is an internal knob no
interface promises to keep.

## What "configured by someone trying to win" means, concretely

It means each of these, and each of them is a thing the baseline gets that
would be easy to quietly not give it:

- **io_uring is on** — `kernel.io_uring_disabled = 0` — on a distribution that
  may well ship it disabled, and **SQPOLL is permitted**, so the baseline is
  measured with a submission-queue poll thread rather than one system call per
  submission. That is the strongest form of the thing F is claiming to beat.
  `apply.sh` prints the `setcap cap_sys_nice` line that lets the poll thread be
  pinned; it does not run it, because it was not told which binary is the
  workload.
- **The workload runs SCHED_FIFO at priority 80**, on an isolated core, with
  `kernel.sched_rt_runtime_us = -1` so it is not throttled. Left at the
  default, the real-time throttle inserts a 50 000 µs stall once per second
  into the baseline's tail — most of its p99.9, contributed by a setting nobody
  chose. The full argument for FIFO over a `sched_ext` scheduler, and the
  condition that would reverse it, is in `baseline.conf` beside the value.
- **Transparent huge pages are on** — `always`, with synchronous defrag — for
  the baseline half. This is the one setting here that deliberately disagrees
  with `claims/runner-class-A.md`, and the disagreement raises the baseline
  rather than lowering it. `baseline.conf` states why in full; the short form
  is that F maps its own pre-faulted 1 GiB pages under RFC 0007 and does not
  use Linux's THP at all, and giving the baseline 4 KiB pages on that account
  would be a comparison against a machine nobody configures.
- **The frequency is fixed and the idle states are shallow**, on both halves,
  so neither side's distribution is a distribution over machine states.
- **The interrupts are somewhere else.** irqbalance off, affinities pinned to
  the housekeeping set, and `isolcpus=managed_irq` for the driver-owned vectors
  that no affinity write can move.
- **Speculative-execution mitigations are on**, stated as a token in
  `cmdline.txt` rather than left as a default, and stated as a cost rather than
  hidden in the metric (R12). They are on the system-call and context-switch
  edges this claim measures, so they cost the baseline more than they cost F —
  which is precisely why turning them off would be the easiest way to make this
  comparison dishonest, and why the requirement is that the setting is
  identical on both sides and visible in both `/proc/cmdline`s.

## Which claims compare against it

| Claim | How it uses this |
| --- | --- |
| `claims/0001-ring-submit-latency.toml` | Directly. `ratio_vs_baseline = { min = 5.0 }` is a ratio against a number taken on a machine in this configuration, and `[baseline] path` names this directory. |
| `claims/0002-timer-jitter.toml` | Not at all, and says so: an absolute bound on this system's own behaviour has nothing to be a ratio against. What takes the baseline's place there is RFC 0007's reservation policy. |
| `claims/0003-boot-to-m0.toml` | Not at all. A Linux boot and reaching `M0 ok` are not the same event. |
| `E1-P10`'s four datapath claims | The reason this directory is in E1 rather than E0. Ring submit under load, doorbells, copies and kernel entries per operation are the first claims with a workload worth tuning a baseline *against*, and a baseline written before there was one would have been a guess with a filename. Those claims may find this configuration incomplete — a NIC's queue count and interrupt routing are not here yet — and the answer to that is the reversal condition below. |

This is a **baseline configuration and not a baseline number.** Nothing here
has been run, because nothing in this repository has a machine of the class
`claims/runner-class-A.md` specifies. The same honesty that file states applies
to this one: *a file is not a machine*. What a file can do is make the
comparison decidable, and make the alternative — somebody quoting a ratio
against whatever Linux was on the box — a thing a reviewer can catch by reading
one page.

## Applying it

The two CPU lists have no defaults and never will. CPU enumeration is the
firmware's business; `claims/runner-class-A.md` writes `4-15` as an example and
says in the same paragraph that it is wrong on most machines. Read the pairs
off the machine first:

```bash
cat /sys/devices/system/cpu/cpu*/topology/thread_siblings_list | sort -u
```

Then, on the machine:

```bash
cd claims/baselines/linux-6.x-tuned
./apply.sh --dry-run                       # every write it would make, and none of them
sudo F_MEASURED_CPUS=4-15 F_HOUSEKEEPING_CPUS=0-3 ./apply.sh
```

Both scripts are committed executable. If the copy in front of you arrived
without the mode — through an archive, or a checkout with `core.filemode=false`
— run them as `bash ./apply.sh`; nothing else about them changes.

`apply.sh` exits `2` on a machine it has not been applied to before: every
run-time setting is in place and the kernel command line is not, because the
command line needs a reboot and this script does not edit bootloaders. It
prints the exact tokens to add to `GRUB_CMDLINE_LINUX_DEFAULT`. Add them, run
`grub-mkconfig`, reboot, run it again; it exits `0`.

It is idempotent by construction rather than by checking first — every write is
an absolute value, never an increment and never an append — so running it twice
and running it ten times leave the machine where the first run left it.

`apply.sh` refuses a kernel outside the range in `baseline.conf`, and it takes
both siblings of an SMT pair or neither, because RFC 0007's first component is
a physical core and not a hardware thread.

## Verifying it applied

```bash
F_MEASURED_CPUS=4-15 F_HOUSEKEEPING_CPUS=0-3 ./verify.sh; echo $?
```

One line per setting, `[ok]` or `[--]`, and a non-zero exit naming everything
that drifted. It needs no privilege, so it runs from a job and from a cron
entry as easily as from a person.

It checks four things `apply.sh` does not:

1. **What the kernel isolated**, from `/sys/devices/system/cpu/isolated`,
   rather than what the command line asked it to isolate. A typo in `isolcpus=`
   is accepted silently at boot and leaves an empty isolated set, which is the
   one failure here that produces a machine looking configured and measuring
   nothing.
2. **The huge-page pool and the absence of swap**, from `/proc/meminfo`, with
   the expected size and count read out of `cmdline.txt` rather than restated.
3. **Where interrupts actually land** — `effective_affinity_list`, which is
   what the interrupt controller did, not `smp_affinity_list`, which is what
   was asked for. It also prints the interrupts *delivered* to measured cores
   since boot, as evidence rather than as drift: those counters do not reset,
   so a non-zero column is as likely to be the minute before `apply.sh` ran.
4. **The other copy of the command line.** `claims/runner-class-A.md` states
   the same list in prose for a reader, and when `verify.sh` can reach a
   checkout it compares the two. Two copies of one list is the decay this
   directory was written against; the only defence is a check that reads both.

## How `A-04` re-checks this

`A-04` is the standing item that keeps this honest: *re-tune every claim's
baseline, or the tuned-Linux comparison quietly becomes a stock-Linux
comparison*. Its cadence is **once per epoch, and whenever the baseline's own
version moves** — a kernel upgrade on the measurement machine is the second
trigger, and it is the one that arrives without anybody deciding anything.

The command is:

```bash
F_MEASURED_CPUS=... F_HOUSEKEEPING_CPUS=... ./verify.sh
```

Non-zero is the finding. There are three kinds of it and they are not the same
kind of work:

- **A setting drifted.** A distribution upgrade re-enabled irqbalance, a
  firmware update reset a governor, somebody put back four settings out of
  five. Re-run `apply.sh`. Nothing about the baseline changed, and any number
  taken between the drift and the fix is not a comparison.
- **The kernel moved out of the range.** Not a drifted machine — a different
  baseline. See the reversal condition below.
- **The two copies of the command line disagree.** One of them was edited and
  the other was not. `cmdline.txt` is the copy that runs.

Re-checking is not the whole of `A-04`. Re-*tuning* is the other half and it is
a judgement rather than a command: has Linux acquired something since the last
epoch that somebody trying to win would use, and this baseline does not?
`io_uring` registered ring descriptors, a `sched_ext` policy, a new zero-copy
receive path. A baseline that only ever gets re-verified and never re-tuned
decays exactly as fast as one nobody checks at all — more slowly, and in a way
that is harder to see, because the checks are green the whole time.

## The reversal condition

**When this baseline is re-tuned, it becomes a new versioned directory beside
this one. It is never edited in place.**

This mirrors the rule `docs/rfc/README.md` applies to reversals — a superseded
RFC is marked, never edited away — and it is the same rule for the same reason.
`claims/README.md` rule 1 says the baseline is versioned *with* the claim: a
number recorded in 2026 was compared against a machine configured a particular
way, and editing this directory afterwards silently re-dates every number that
ever cited it. The comparison would still be documented. It would be documented
against a configuration that did not exist when the measurement was taken,
which is worse than no documentation, because it reads as evidence.

So: a new directory, `claims/baselines/linux-<version>-tuned/`, a `path` update
in the claim that moves to it, and this one left where it is with the claims
that cite it still citing it. The three things that trigger that:

- **The kernel range in `baseline.conf` no longer holds.** A 7.0 kernel, or a
  6.x that removed one of these knobs.
- **A claim needs something this configuration does not have.** `E1-P10` is the
  live case: a datapath claim needs a device, and a NIC's queue count, its
  interrupt routing and its offload settings are not in these files. That is
  `E1-P10`'s directory to write, against `E1-P10`'s workload, which is the
  ordering argument `E1-D06` makes about itself one epoch earlier.
- **A tuner would now do something these files do not.** `A-04`'s other half,
  above.

What would reverse the *directory*, rather than its contents: a claim whose
`[baseline]` is genuinely not Linux. `claims/0002` and `0003` already have
`system = "none"` and are right to. If most claims end up there, the thing to
question is not this directory but whether `ratio_vs_baseline` was ever the
right shape for what this project is arguing.
