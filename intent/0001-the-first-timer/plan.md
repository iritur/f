---
id: 0001
status: done
spec: ./spec.md
---

# Plan: the local APIC, a calibrated clock, and the first jitter histogram

Two changes, in two pull requests, because they fail differently. The first
takes over a device and can be wrong about hardware. The second builds a
measurement on top of it and can be wrong about arithmetic. Reviewing them
together means reviewing neither.

The seam between them is deliberate and is not only about size. Part one adds
nothing whose value varies between runs, so the boot log it produces is still a
fixture. Every measured number in this intent is in part two, along with the one
run that is allowed to print one.

## Part one — the kernel takes over its interrupt controller

### Files

```
kernel/src/arch/x86_64/port.rs      NEW: `inb`/`outb`, moved out of serial.rs
                                    because there is now a second device that
                                    speaks port I/O, and duplicating them would
                                    duplicate the SAFETY argument too
kernel/src/arch/x86_64/serial.rs    uses port.rs; its private copies go
kernel/src/arch/x86_64/pic.rs       NEW: the 8259 pair, remapped off the
                                    exception vectors and masked. Not a driver —
                                    the opposite of one
kernel/src/arch/x86_64/apic.rs      NEW: bring-up, the register window, the
                                    spurious vector, and a readback that proves
                                    the window is the device
kernel/src/arch/x86_64/paging.rs    DEVICE_OFFSET and `map_device`: one page
                                    into a live address space, uncacheable.
                                    E0-B04 deferred exactly this and said so
kernel/src/arch/x86_64/idt.rs       the spurious vector — the first gate in the
                                    table that is not an exception
kernel/src/arch/x86_64/mod.rs       `read_msr`/`write_msr`; the module list; the
                                    doc comment that says the APIC arrives at M2
kernel/src/main.rs                  bring the APIC up after the address space,
                                    report what was found, fail the boot if it
                                    could not be
intent/0001-the-first-timer/        NEW: this intent, its spec and this plan
TODO.md                             E0-B07 in progress, naming this intent
```

### Order

Whatever can fail against real hardware, first.

1. `port.rs`, and serial.rs onto it. Nothing else can be written until port I/O
   has one home.
2. `paging::map_device`. The APIC is unreachable without it, and a mapping
   written into the address space the caller is running in is the riskiest thing
   in part one.
3. `apic.rs` bring-up, which is what says whether step 2 worked.
4. `pic.rs`, before anything *can* enable interrupts rather than merely before
   anything wants to.
5. `idt.rs` spurious vector, so the APIC has somewhere to send what the
   architecture requires it to be able to send.
6. `main.rs`, then `TODO.md` and this plan.

### Proof

```
cargo xtask verify
```

Green before, green after, with one new line in the boot log naming where the
APIC was found and what it says it is — and no measured number in it, because
the log is a fixture. All six `cargo xtask fault` paths still report, because
this diff moves the legacy interrupt controllers and remaps their vectors, and
`df` and `stack` are the two that would notice if it moved them wrongly.

### Risks

- `map_device` writes into the address space the caller is running in. Under a
  window of its own it cannot collide with the direct map, which is the reason
  the window is separate rather than an offset into the existing one — and the
  reason is a machine with enough memory for the direct map's huge pages to
  cover a device address.
- The 8259 pair is remapped *and* masked. Masking alone leaves the spurious-IRQ
  path landing on an exception vector, where it would be reported as an
  exception nobody caused. A false report is worse than no report.
- A mapping that succeeds is not a mapping that works. The readback is there
  because a window pointing at the wrong page either faults or returns zeroes,
  and zeroes are a plausible-looking APIC.

## Part two — the timer, and the histogram

### Files

```
kernel/src/arch/x86_64/pit.rs       NEW: the 8254, gate-controlled and polled.
                                    The known reference, and the only clock in
                                    the tree whose frequency is a constant
kernel/src/jitter.rs                NEW: log-bucketed histogram, no float, no
                                    allocation, no division on the sample path;
                                    plus a boot self-test, because the kernel
                                    crate cannot be tested on the host
kernel/src/arch/x86_64/apic.rs      calibration against the 8254 in one pass
                                    over both clocks; arming both mechanisms
                                    from one absolute schedule; the tick handler
kernel/src/arch/x86_64/idt.rs       the timer vector
kernel/src/arch/x86_64/mod.rs       the pit module
kernel/src/arch/x86_64/multiboot.rs a command-line parameter with a value, so
                                    `timer=60` can say sixty
kernel/src/main.rs                  the fixed-length probe every boot runs, and
                                    the measurement run when asked for one
xtask/src/main.rs                   `cargo xtask timer [seconds]`; the QEMU
                                    invocation factored into one place now that
                                    there are three callers of it
claims/0002-timer-jitter.toml       NEW: the p99 bound, pending
CLAUDE.md                           the new command
TODO.md                             E0-B07 done, with what its exit actually met
intent/0001-the-first-timer/        this plan and its intent, closed out
```

### Order

1. `jitter.rs` first and alone. It is arithmetic, it is the part most likely to
   be quietly wrong, and it is the only part that can be reasoned about without
   a machine.
2. `pit.rs`, then calibration. Bounded and polled: the gate rises or the spin
   gives up, because a calibration that hangs is a boot with no output.
3. The schedule and the two arming paths.
4. The tick handler and the IDT vector — the point where the two halves meet.
5. `multiboot.rs`, `main.rs`, then `xtask`.
6. The claim, then `TODO.md` and `CLAUDE.md`.

### Proof

```
cargo xtask verify
cargo xtask timer 60
```

The first must stay green and its log must stay reproducible. The second must
print a histogram whose count is the number of ticks it waited for.

### Risks

- Re-arming from "now plus a period" instead of from the absolute deadline is
  the classic error, and it does not look wrong. It makes the timer drift, and
  it makes the jitter histogram look *better* than the truth, because lateness
  is then measured against a deadline that moved to accommodate it.
- A deadline already in the past must still deliver an interrupt, or the run
  stops dead. Both mechanisms need a floor for this, for different reasons.
- Bucket width. Planned as fixed-width buckets a quarter of a microsecond
  across, sized from the measured frequency, which is right for the 5 µs bound
  and wrong for everything else: under the emulator every sample landed past
  the top of the range and the histogram was one bar. Changed during the build
  to logarithmic buckets, eight to the octave — constant *relative* precision,
  no dependence on a calibrated frequency, and one structure that covers both
  real hardware and an emulator. Recorded here rather than quietly rewritten,
  because the reason the first design failed is the useful part.
- The spin waiting for the 8254's gate must be bounded, or a machine without a
  working channel 2 hangs in boot with no output — the exact failure E0-B06
  exists to have removed, and a poor thing to reintroduce one milestone later.
- Port 0x61 is a compatibility register with a speaker on the next bit along.
  Mask, do not assign.
