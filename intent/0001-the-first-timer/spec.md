---
id: 0001
status: agreed
reviewed_by: Dmitri Chudinov
skills: determinism-review, frame-and-unsafe, claims-registry
---

# Spec: the local APIC, a calibrated clock, and the first jitter histogram

The kernel takes ownership of its local APIC, measures both the timestamp
counter and the APIC's own timer against the 8254 — a fixed-frequency reference
that is on every PC-class machine and is not derived from either of them — and
then drives a 1 kHz interrupt from an absolute schedule. Each tick records how
late it was against the deadline it was supposed to land on. Sixty seconds of
that is a histogram.

## Behaviour

**Bring-up.** `IA32_APIC_BASE` names the local APIC's physical page. It is
mapped as a device — uncacheable, writable, never executable — in a window of
its own, and the APIC is software-enabled with a spurious-interrupt vector
installed. A machine whose firmware has already put the APIC in x2APIC mode is
reported and refused rather than driven through a register window that is not
there.

**Calibration.** Channel 2 of the 8254 is gated for a known interval with its
output polled, not raised as an interrupt. The timestamp counter and the APIC
timer's own count are sampled at both ends of that interval. Two frequencies
come out, and both are checked against a plausibility band wide enough that
only a broken measurement fails it. The spin is bounded: a machine where the
gate never rises reports a calibration failure instead of hanging in boot.

**The timer.** A deadline schedule in timestamp-counter units, `t0 + n·period`,
re-armed every tick from the absolute value rather than from "now plus a
period", so that a late tick does not push the next one later. Two mechanisms
carry it, chosen by what the processor advertises:

- `IA32_TSC_DEADLINE`, where the deadline is written to the hardware directly;
- the APIC timer in one-shot mode, where the remaining interval is converted
  into APIC ticks with the frequency measured above.

Both are the same schedule. Only the arming differs, and the log says which one
is in force.

**Measurement.** Lateness is `tsc_on_entry − deadline`, bucketed into a
fixed-width histogram held per core. Bucket width is a power of two of
timestamp-counter ticks chosen at calibration to sit near 256 ns, so that
recording a sample is a shift and a compare and not a division. p50, p99 and
p99.9 are reported as the upper edge of the bucket that contains them; the
minimum, the mean and the maximum are exact.

**Two runs, not one.** An ordinary boot arms the timer, waits for a fixed
hundred ticks, disarms it, and prints only facts that do not vary: which
mechanism, that calibration was plausible, how many ticks were waited for. The
measurement run — `cargo xtask timer [seconds]`, sixty by default — is entered
from the command line and prints the histogram.

## Policy applied

**Determinism (RFC 0004).** A jitter measurement is an observation of real time
and cannot come from `f_env::Env`; that is the point of it. It reads the one
allow-listed `rdtsc` site, `arch::x86_64::read_tsc`, through the existing
function rather than by adding a call site — so `DETERMINISM_ALLOW` does not
grow. What determinism does force is the split above: the reproducible boot
prints no measured number, because the boot log is a fixture and a fixture that
carries a timing number is a fixture that fails at random. E0-B08 is where the
hardware clock reaches `Env` properly.

**The frame (RFC 0001).** All of this is `kernel/`, where `unsafe` is
permitted. Three obligations are new and each is discharged where it is taken:
a device mapping written into a live address space, port I/O to a timer that
predates every rule about port I/O, and a handler that reaches per-core state
the interrupted code may also be holding.

**Per-CPU state.** Every mutable byte the timer owns — the schedule, the
histogram, the tick count, the register window — is one `PerCpu<Timer>`. The
handler and the code it interrupted reach the same slot, which is the case
`percpu.rs` says no per-CPU abstraction can see: so the slot is reached through
the raw pointer with volatile accesses, and never through a `&mut` that would
be claiming otherwise.

**Numbers need claims.** The p99 bound is registered as `claims/0002` before it
is measured, status `pending`, so that the target is on record ahead of any
number anybody is tempted by.

## Not in scope

- **Making the claim gate.** E0-P06, which needs RFC 0007 (E0-D04) first: a
  jitter bound means nothing without saying what the core was reserved from.
- **Routing anything else through the APIC.** No I/O APIC, no inter-processor
  interrupts, no device interrupts. E0-B10 and E0-B15.
- **x2APIC.** Detected, reported, refused. It buys addressing above 255 cores
  and this kernel shards for eight.
- **A tick that does anything.** The handler counts and returns. There is no
  scheduler for it to drive until M3.
- **A hardware `Env`.** E0-B08.

## Evidence

- `cargo xtask run` — the boot reaches the timer, waits a fixed number of ticks
  and reports; a timer that never fires hangs nothing, because the wait is
  bounded and reports what it got.
- `cargo xtask timer` — sixty seconds at 1 kHz, and a histogram at the end.
- `claims/0002-timer-jitter.toml` — the threshold, on record, `pending`.

## Risks and reversal

**The environment cannot produce the number.** QEMU's TCG backend advertises
neither TSC-deadline nor x2APIC — it refuses both by name — and emulates the
APIC timer against a host clock it does not control. So the mechanism this task
names is, locally, the one that cannot run, and the histogram a container
produces measures the emulator. That is why the second mechanism exists and why
the claim stays `pending`: a number from here is not a claim, and
`F_ENVIRONMENT=container` is already how the harness knows.

*What would reverse this:* a runner with `/dev/kvm`, where TSC-deadline is real
and the one-shot path becomes the fallback it is meant to be.

**Calibration against the 8254 is itself emulated.** Under TCG both the
reference and the thing being measured come from the same host clock, so
agreement between them proves the arithmetic and not the hardware. Stated
rather than concealed; on a real machine the two are independent.

**A `cpuid` per tick.** `current_cpu()` serialises, and the handler calls it to
find its own slot — inside the interval being measured. It is bounded and it is
in every sample equally, so it biases the histogram rather than adding a tail.
`arch::x86_64::mod` already predicted this: E0-B10 replaces the lookup with a
`GS`-relative read and the cost goes away.

No RFC is owed. Nothing here reverses anything written down; the design
documents ask for exactly this and the second timer mechanism is an addition
under the same principle, not a departure from it.
