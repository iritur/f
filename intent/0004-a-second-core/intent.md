---
id: 0004
status: shipped
originator: Dmitri Chudinov
todo: E0-B10, E0-P08
---

# The mapping goes with the name, and the program comes from outside

## Problem

Two things were left open at M4, and they turned out to be the same thing.

The first is stated in four places rather than buried, because it is the
sentence somebody would otherwise assume the other way round: **revoking a frame
capability withdraws the name and leaves the mapping.** `paging.rs` says it,
`process.rs` says it, `intent/0003`'s risk section says it, and the `E0-B11`
entry in `TODO.md` says it. A component whose authority had been taken back went
on reading the page through a translation nobody could remove. That is the
largest gap in the capability system, and a capability system that can take a
name back but not the memory is not yet answering the drawback it exists to
answer.

Undoing a mapping needs an unmap. An unmap needs a shootdown — a page taken out
of a table is still in every other core's translation buffer until that core is
told. And a shootdown needs somebody to tell. So the gap was not that one core
made it hard; it was that one core made it **unfalsifiable**. A kernel that
edited the table and skipped the interrupt would have passed every test it could
have been given.

The second is that nothing this system runs at ring 3 comes from outside the
kernel. `arch::x86_64::probe` is sixty instructions of hand-written assembly in
the kernel's own `.rodata` — the frame's adversary, there because M3 needed
*something* at ring 3 before there was a loader. `user/init` exists as a library
that proves one structural property and is never executed. The loader has been
handing modules over since M1 and the kernel has been reserving their memory and
reading none of it.

There is a third problem underneath, and it is E0-P08's. Four of the capability
suite's five properties have a fixture that breaks them, checked in and run at
every boot. The fifth — *a process cannot make the kernel panic by trying* —
cannot: a fixture that panics takes the machine down rather than being caught,
and there is no host harness for kernel logic to catch it in. It was marked `[>]`
with the gap named rather than claimed as met.

## Proposed outcome

A second core, and everything that becomes possible once there is one.

- Every core the machine has is brought up to the same point the boot processor
  reached: its own descriptor tables, its own local APIC, its own system-call
  entry, its own stacks with guard pages under them.
- A process runs on a core that is not the one holding the timer, so "the timer
  kept its schedule while ring 3 held a core" becomes a statement about two
  cores rather than about one core's transitions.
- Revoking a capability that authorised a mapping takes the mapping with it: the
  entry is cleared, this core's translation is invalidated, every other running
  core is told and has to acknowledge, and a process that reads the page
  afterwards takes a page fault. There is a boot that does exactly that and
  fails if it does not.
- `user/init` becomes a real component: ordinary Rust with no `unsafe` in it,
  compiled and linked separately, handed to the machine by the boot loader as a
  file, and copied into a frame it was granted. It runs before the frame's own
  adversary on every boot.
- The fifth property gets the second half of its exit criterion by a different
  mechanism from the other four: a kernel built with one deliberate defect,
  booted, and required to go red.

## Affected users and systems

The frame, and only the frame. Nothing above it changes shape — a component
still sees a door with seven calls behind it. What changes for a component is
that it is now *told* its starting handles rather than entitled to assume them,
because a second process on a core finds its capabilities at a later generation.

## Constraints

- The boot log stays a fixture. Two cores writing to one serial port produce
  interleaved bytes, so a started core prints nothing and the boot processor
  says what it found afterwards.
- No lock. `ring-scene-boot` section 14 and `CLAUDE.md` both say every mutable
  static under `kernel/` is a `PerCpu<T>` and nothing there locks, and a
  handshake between cores cannot obey that as written. Whatever crosses a core
  boundary has to be small enough to argue for.
- The measurement environment is still not one. Nothing here produces a number
  anybody may quote; `claims/0002-timer-jitter.toml` stays pending.

## Open questions

- Does a second core cost core 0's schedule anything on a real machine? Nothing
  here can answer that: under an emulator the p50 is two orders of magnitude
  past the bound the claim names, and the run-to-run spread is larger than any
  difference between builds. E0-P06 owns the number and it needs hardware.
- How many cores does the machine have? Answered from `cpuid`, which is one
  package's count and assumes dense APIC ids. The right answer is the ACPI
  multiprocessor table, and it arrives with E5 naming a real machine.
