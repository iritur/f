# TODO

The long list. Seven epochs, four movements each, one gate at the end of every
epoch and one release behind every gate.

Reasoning, gate definitions, the testing environment and the release contract
are in [`docs/the-long-plan.html`](docs/the-long-plan.html). This file is the
operational list: it is meant to be edited constantly and read by anyone who
wants to pick up work without asking permission.

---

## How this list works

**Four movements per epoch.** `D` decide, `B` build, `P` prove, `R` release.
They are separated here because the failure mode of this project is doing all of
`B` and none of `P` — so the balance has to be visible.

**They are not four phases, and `D` is not a queue to be drained first.** This
file is a dependency graph printed in reading order, and reading order is not
work order. For the work order, ask:

```
cargo xtask todo        # everything
cargo xtask todo E0     # one epoch
```

which computes what is available now and ranks it by how much each task
unblocks. That last number is the one nobody can work out by eye, because
`needs:` points backwards: the file tells you what a task waits for and never
what waits on it.

**Every task carries an exit.** `exit:` is the observation that closes the task —
a test that passes, a number recorded, a document merged. A task with no exit is
a wish and does not belong on this list. The one exception is the *always on*
section at the end, whose items never close: those carry a `cadence:` instead,
and an always-on item with neither is a rule nobody is keeping.

**Every task carries its blockers.** `needs:` names the task IDs that must be
done first. A task with no `needs:` and no `[ ]`-marked blocker upstream is
immediately available to anybody. That is what makes the list freely realizable
rather than a plan only its author can execute.

**IDs are permanent.** `E1-B04` means the same thing forever. Dropped tasks are
marked `[~]` with a one-line reason and stay in place; they are not deleted,
because the reason is the useful part.

## Ordering

Four rules, in priority order. They exist because "ready" and "next" are
different questions, and the graph only answers the first.

**1. The wire format goes first, whatever else is ready.** `E0-D01`, `E0-D02`
and `E0-D03` change `abi/`, which is free to change while one peer exists and
expensive once two do. Nothing else on this list has a cost that rises that
sharply with delay. This is the only rule that overrides the ranking.

**2. Otherwise, take what unblocks the most.** `cargo xtask todo` sorts by it.
A task holding up twenty others is the critical path; a task holding up nothing
is real work that can wait for a quiet afternoon however urgent it feels.

**3. A decision goes immediately before the work that would be expensive to redo
without it — never at the head of the epoch.** `E0-D04` exists to precede the M2
jitter gate, not to precede everything: written earlier it would be designing
against a measurement nobody has taken, which is the exact failure the design
documents warn about when they say the build order is the schedule.

**4. When two tasks are otherwise equal, prefer the one that produces
information.** Building teaches you things deciding cannot. `E0-B02` — one
milestone of work — produced thirteen findings about the tree, including two
policy lints that reported themselves and a litmus test that raced nothing.
No amount of prior deciding would have surfaced any of them.

**Status:** `[ ]` todo · `[>]` in progress · `[x]` done · `[~]` dropped.

**Sizes:** `S` under a day · `M` under a week · `L` under a month · `XL` longer,
and an `XL` that is not decomposed by the time it starts is a planning failure.

---

## E0 — Ground truth

*Phase 00, milestones M0–M6. The system boots, is deterministic, has
capabilities, and speaks one ring. Nothing here is novel; all of it is
load-bearing.*

**Effort:** 0.5–1.5 person-years · **Risk:** low · **Ends at:** gate G0, release 0.1

### Decide

- [x] **E0-D01** `S` Write RFC 0009 — three clocks, and only one of them orders anything.
  Monotonic `Instant` is the sole ordering clock; wall time is an injectable datum with a source and an uncertainty; civil time lives above the semantic layer.
  *exit:* RFC merged; `abi` documents `Sqe.deadline` as monotonic ns in the channel epoch with `NO_DEADLINE = 0`.
- [x] **E0-D02** `S` Write RFC 0010 — errors name a domain, cancellation is not an error.
  *exit:* RFC merged; `abi::error` exists with six domains; a decode round-trip test passes.
- [x] **E0-D03** `M` Write RFC 0011 — peers negotiate a version, they do not match one.
  *exit:* RFC merged; `ChannelHeader` carries version, floor, offered and required features inside the existing 64 bytes; `negotiate()` has tests for every refusal path.
  *needs:* E0-D02 (refusals are reported in the error space)
- [x] **E0-D04** `S` Write RFC 0007 — the hard class reserves whole cores and pre-faulted memory.
  Must precede the M2 jitter gate, or that gate measures an idle machine. It does: E0-B07 landed the timer, and this was written before the gate rather than after a number existed to design around.
  Four components — a physical core, a bandwidth allocation, a cache partition, pre-faulted huge pages — admitted together or the test is testing nothing. The branch worth disagreeing with is what happens on hardware that cannot partition one of them: the component is neither waived nor pretended, it is bought by holding the co-resident cores idle. That is the expensive answer on purpose. A waived component costs a tail nobody sees; exclusion costs capacity everybody does, and a cost that is visible is the one that gets argued with rather than discovered.
  *exit:* met. `docs/rfc/0007-reservations.md` is accepted, and `claims/0002-timer-jitter.toml` carries a `[reservation]` section naming all four and requiring every run to record whether each was obtained by partition or by exclusion — so the two easy ways to produce a flattering histogram, an idle machine and a quietly dropped component, are both excluded by the file rather than by the operator's care. E0-P06 is now waiting on a machine instead of on a decision.
- [x] **E0-D05** `S` Write RFC 0013 — every component publishes a state tree.
  The sentence worth attacking is what a snapshot means. The kernel's mutable state is per-CPU and nothing under `kernel/` locks, so a tree spanning eight cores is a read of eight sets of words that no single instant covers. A seqlock over the whole tree would buy a consistent cut and put a fence on every counter update in the system — the observability apparatus becoming the most expensive thing on the hot path, which is how observability ends up switched off and describing a configuration nobody runs. It would not even buy the cut: eight writers who never coordinate are never all quiet under load. So a snapshot is **atomic per node and not across the tree**, each subtree publishes the timestamp at which it was last written so skew is a number rather than an assumption, and cross-node consistency is bought only where quiescence is genuinely available — the simulator, and explicit snapshot boundaries.
  The other decision recorded rather than assumed: R04 says fail closed, and this deliberately does not. An older reader meeting a node type it does not know skips it and reports the count. R04 protects a *decision*, and a state tree reader takes no action and holds no authority — refusing to display anything because one node is newer than the tool is how observability gets bypassed for a debugger. The hash is over bytes rather than over interpretation, so a reader that skipped four nodes still hashed them.
  *exit:* met. `docs/rfc/0013-state-tree.md` is accepted. **The exit line was wrong and is corrected here rather than satisfied as written**: it named E0-B15, the doorbell, which does not read this. The task that needs a specification to build against is E0-B14, the state tree itself, and `needs:` on that task already said so.
  *needs:* — (`docs/what-must-be-stated.html` section 20 schedules it M1–M2, before the counters that make a jitter histogram into a claim have anywhere to be published from)
- [x] **E0-D06** `S` Write RFC 0006 — idle depth is computed from the reservation table.
  Design only at this epoch; implementation is E1/E5.
  Written after E0-D04 and not before it, because the arithmetic reads the table RFC 0007 defines. Energy was in the first paragraph of the thesis and owned by no subsystem in any of the five design documents; it now has one, and the owner is admission control rather than a new subsystem. The sentence to attack is that Linux's governors predict because Linux cannot know, and admission control means F knows — a predictor being what you build when the information is absent, and this architecture having already paid to make it present.
  *exit:* met. `docs/rfc/0006-energy.md` is accepted, and `bench/src/lib.rs` carries `idle_residency` beside `joules_per_op`, both `Metric::Unavailable`. The new one does not repeat the mistake the TESTING-STATUS commit found in its neighbours: it names E5-B07 rather than a milestone that has already arrived, and it says the absence is not a wiring gap — nothing idles, because the kernel spins between ticks on purpose.
  `apic::wait`'s comment said the idle policy did not exist. It does now, so the comment says what is actually still missing: the table, and the implementation.
- [ ] **E0-D07** `M` Adopt the twelve rules in `CONTRIBUTING.md`, and mechanise the three that can be.
  *exit:* `xtask lint` gains a unit-and-epoch check on public `abi` fields, a claims-ownership check, and a no-callback check; each fails a deliberately broken fixture.
- [ ] **E0-D08** `M` Write `RELEASING.md` — what a release is, what it contains, how a stranger reproduces it.
  *exit:* document merged; `cargo xtask release --dry-run` prints the manifest it would produce.
- [x] **E0-D09** `S` Record the target-JSON decision: use `targets/x86_64-f.json` or delete it.
  Deleted. Nothing built it — its only two references in the whole tree were `BOOTSTRAP.md`'s gap table and this line — and everything it said that the built-in `x86_64-unknown-none` does not is two codegen flags already set in `.cargo/config.toml`, beside the paragraph explaining why the image does not link without them. The usual argument for a custom target does not apply here either: the build already passes `-Zbuild-std`, so the JSON was never buying the thing a JSON usually buys, and an unbuilt second copy of a target definition cannot fail loudly when the spec schema moves under it.
  *exit:* met. The file is gone; the reason and the reversal condition — a data layout, a linker flavour or an atomic width that the built-in plus rustflags cannot express — are a doc comment on `KERNEL_TARGET` in `xtask/src/main.rs`. The `BOOTSTRAP.md` gap row is struck through and marked done, which is where the question was actually being asked from.

### Build

- [x] **E0-B01** `M` **Green build.** Done in the container; the CI half is `E0-P01`.
  *exit:* met. The three commands pass in the development image — a machine whose only prerequisites are the pinned toolchain and QEMU — from a tree copied out of `git ls-files` into an empty directory with no `target/` in it, so nothing in the loop depends on something a previous build left behind. CI runs the same three commands; that it *stays* green on a pull request is `E0-P01`'s exit and not this one, which is the split the note above always claimed.
- [x] **E0-B02** `M` Bootloader handoff — **multiboot 1**, not Limine or multiboot 2: QEMU implements it in its own `-kernel` loader, so the handoff costs a header and a stub rather than a vendored binary or an ISO step. Memory map reaches the kernel; no framebuffer, which M1 does not need.
  *exit:* met. The kernel prints seven regions and 130 559 KiB usable, then exits 33 with `M0 ok`, and two runs of the same commit are byte-identical.
  *needs:* E0-B01
- [x] **E0-B03** `M` Physical frame allocator built from the boot memory map. **(M1)**
  An intrusive free list: the link lives in the free frame, so there is no metadata to size and no bootstrap problem where the allocator needs an allocator.
  *exit:* met, with the mapping half deferred and said so. 32 434 frames from the map; a thousand frames allocated, stamped, verified and freed in an `Env`-chosen order, ten rounds, free count bit-identical. **Map and unmap move to E0-B04**, which is where the frame takes ownership of the page tables — until then the boot stub's identity map is what makes a frame addressable, and frames above 1 GiB are counted as unreachable rather than handed out.
  *needs:* E0-B02
- [x] **E0-B04** `M` Take ownership of the page tables: higher-half kernel, direct physical map. **(M1)**
  Kernel relinked to -2 GiB and loaded low; the boot stub maps the first gibibyte twice so the jump between the two windows survives; the kernel then builds its own tables from allocator frames — a direct map of all usable physical memory at `0xFFFF800000000000` plus the kernel window — switches `CR3`, and drops the identity window entirely.
  *exit:* met. The M1 stress test passes *through the direct map*, which is the part that proves the switch. On a 2 GiB machine the pass that claims memory the identity window could not reach adds 262 112 frames, so the path is demonstrated rather than assumed.
  *needs:* E0-B03
- [x] **E0-B05** `S` `PerCpu<T>` in place before the second core exists, per the standing decision.
  The kernel had exactly three mutable statics — the GDT, the task state segment and the IDT — and all three are now slots in a `PerCpu<T>`, indexed by the initial APIC id the processor reports. `gdt::init` and `idt::init` install *this core's* tables and no other core's, which is the shape the application processors need and the reason the safety obligation on both changed from "once" to "once per core". `PerCpu::mine` hands out a raw pointer rather than a reference, because a per-CPU abstraction cannot see an interrupt handler reaching the same slot as the code it interrupted, and a safe `&mut` would be claiming otherwise.
  *exit:* met, from both sides. Boot prints `per-cpu core 0 of 8, slots distinct` before the tables are installed, having proved that a write through one slot is invisible to the other seven — a shard that returns the same pointer for every core looks perfectly correct on a machine with one. `cargo xtask lint-percpu` fails the build on a `static mut`, or on a `static` holding a cell, a lock or an atomic, anywhere under `kernel/` except `percpu.rs`; a probe file carrying four of them reported four findings, and the `PerCpu` and plain `const`-like statics beside them reported none. All six `cargo xtask fault` paths still report, including `df` and `stack`, which are the two that go through the interrupt stack table in the now-per-core task state segment. The cost is eight frames off the free count: seven extra copies of the interrupt table, 28 KiB.
  Sharded here and not everywhere, at the time: the double-fault stack the task state segments named still came from the linker script and there was exactly one of it, because a stack needs a guard page and a guard page needs the mapper. That belonged to `E0-B10` and the code said so where it was wrong rather than in this file — and `E0-B10` closed it, with a block per core reserved by the same linker script.
- [x] **E0-B06** `M` GDT, IDT, exception handlers with a register dump worth reading. **(M2)**
  Also fixed a live landmine: `GDTR` still pointed at the boot stub's table in low memory, which E0-B04 stopped mapping. Nothing had noticed because nothing had reloaded a segment or taken an interrupt — and the first thing to do either would have been this handler.
  *exit:* met. `cargo xtask fault pf|ud|df` boots into a deliberate fault; the report names the exception, decodes the error code, gives the faulting instruction and prints all fifteen registers. `df` proves the interrupt stack table: a fault with no usable stack is reported rather than resetting the machine.
  *needs:* E0-B04

- [x] **E0-B18** `M` A guard page under the kernel stack — and under the fault stack too.
  Both stacks moved out of `.bss` into a `.stacks` section in the linker script, each with an unmapped page below it. A guard page has to be somewhere the mapper can skip, and skipping a page inside `.bss` would mean skipping whatever else shared it.
  *exit:* met. `cargo xtask fault stack` overflows the stack and reports `EXCEPTION 8` with `rsp` at exactly `__kernel_stack_bottom` — the guard caught it. Before, the same overflow walked down through the descriptor tables and reset the machine with no output.
  Found by trying to provoke a double fault with a stack overflow: the stack grows straight down into `.bss`, where the descriptor tables live, so an overflow corrupts the machinery that would have reported it and the machine triple-faults with no output. The interrupt stack table cannot help — by then the IDT itself is gone.
  *needs:* E0-B06
- [x] **E0-B07** `M` Local APIC and TSC-deadline timer, calibrated against a known reference. **(M2)**
  `intent/0001-the-first-timer/`, in two parts. The first took over the
  interrupt controller: the local APIC mapped through a device window of its
  own, a spurious vector, and the 8259 pair remapped off the exception vectors
  and masked — because their default assignment puts a free-running IRQ 0 on
  the double-fault vector, and the first `sti` would have reported a double
  fault that never happened. The second built the schedule and the histogram.
  *exit:* met. `cargo xtask timer 60` runs 60 000 ticks at 1 kHz and prints a
  log-bucketed distribution of how late each one was, with p50, p99, p99.9 and
  an exact maximum. Every boot runs a hundred ticks of the same path and fails
  if it does not get all hundred, so the mechanism is checked on every run
  while the measurement stays out of the log — a boot log carrying a timing
  number is a fixture that fails at random, and two runs of this commit are
  still byte-identical.
  **Two things the exit does not say, and both matter.** The mechanism the task
  names is the one that cannot run here: QEMU's TCG backend refuses
  `tsc-deadline` and `x2apic` by name, so the timer runs on the APIC's own
  one-shot countdown and the TSC-deadline path is written, selected by `cpuid`,
  and unexercised until there is a runner with `/dev/kvm`. And the histogram a
  container produces is of the emulator — p99 lateness in the hundreds of
  microseconds against a 5 µs bound. `claims/0002-timer-jitter.toml` is
  `pending` for that reason; **E0-P06 still owns the number**.
  Found while building: `0b1011_0010` is the byte every 8254 calibration
  example uses, and its mode field selects mode *1*, not mode 0. With the gate
  raised after the counter is loaded it reads as already expired — a
  ten-millisecond interval that measures eighty microseconds, and a calibration
  wrong by two orders of magnitude that still produces a confident-looking
  number. The frequency was only obviously wrong because a second, independent
  measurement disagreed with it.
  *needs:* E0-B06
- [x] **E0-B08** `S` Wire the hardware `Env` to the one legitimate `rdtsc`; add the wall-clock capability from RFC 0009.
  *exit:* met. `kernel/src/env.rs` is the production `Env`: `now()` is the
  timestamp counter divided by the frequency `apic::calibrate` measured,
  reached through `arch::x86_64::read_tsc` rather than through a second call
  site, so `DETERMINISM_ALLOW` is unchanged and still names one file.
  `f_env::contract` is the new part worth arguing with — the properties every
  `Env` owes, written once as a function rather than as a test, because the
  seeded environment is checked on the host and the hardware one exists only
  inside a kernel with no host harness. The host tests run it against
  `SeededEnv` and `SimEnv` and against six environments broken on purpose, one
  per violation; the boot runs it against both environments on the same run and
  prints `env contract  arithmetic ok, seeded ok, hardware ok`.
  **Nothing consumes the hardware `Env` yet, and that is the honest state.**
  The boot still runs on the seed, because the boot log is a fixture and a
  fixture carrying a number from a real clock fails at random. The first caller
  with a genuine claim on real time is the scheduler at M3; wiring one before
  then would mean inventing consumers to make the type look used.
  Found while building: the obvious tick-to-nanosecond conversion,
  `ticks * 1_000_000 / tsc_khz`, overflows `u64` after about ninety minutes of
  uptime at 3.4 GHz — and does not fail, it wraps, so the monotonic clock jumps
  backwards by nine minutes on any boot that lives long enough. Dividing first
  and scaling the remainder is exact and lasts the 584 years the type allows.
  The self-test asserts the overflow case still overflows, so the reason for the
  split cannot be quietly deleted.
  On the wall clock: it is the CMOS, read once at boot and carried forward on
  the monotonic clock rather than re-read — re-reading a clock that people set
  is how a stamp moves backwards between two lines that both look right. It is
  stated as accurate to an hour, which looks absurd and is the honest number: an
  undisciplined oscillator's drift dominates the second of quantisation, and a
  bound in seconds would be a precision claim this system cannot support. A
  machine whose firmware keeps local time is wrong by whole hours and no bound
  of this shape covers that, which `rtc.rs` says outright.
  *needs:* E0-B07, E0-D01
- [x] **E0-B09** `L` User page tables and the ring-3 transition; a `syscall` entry used strictly for channel setup. **(M3)**
  `intent/0002-something-that-is-not-the-kernel/`. A process is an address space
  whose upper half is a copy of the kernel's, two pages — text executable and
  not writable, stack writable and never executable, an unmapped guard between
  them — and two ways of ending. It is entered through a hand-built interrupt
  frame, answers three calls, and comes back either through `sysret` or through
  the interrupt table with its frame rewritten to resume the kernel call that
  started it.
  *exit:* met, and the *while* is the part worth reading. Every boot builds a
  process, enters ring 3, takes **eight timer ticks out of it**, watches it read
  the kernel's direct map, reports `exception 14 … error 0x5` — present, and
  refused — kills it, gives back all six frames with the free count unchanged,
  and then finishes the same hundred-tick window the whole thing ran inside. The
  window's assertion is the one that has covered every boot since M2: every tick
  the schedule asked for arrived. It now covers a window that contained user
  space, which is what "throughout" is worth here.
  `cargo xtask user` is the negative half and is in CI: seven boots, six
  violations that must fault and one that must not. The error codes are the
  evidence and each one is a different sentence — `0x5` reading the direct map
  (present, and ring 3 still cannot have it), `0x6` writing the null page,
  `0x7` writing its own text, `0x15` executing its own stack, and `EXCEPTION 13`
  for `cli`. A protection nothing tries to violate is a protection nobody has
  checked; these are the ring-3 half of what E0-B19 said.
  **What the exit does not say, and it matters.** The M2 jitter *bound* is not
  met here and was not met before user space existed: QEMU's TCG backend
  emulates the timer against a host clock it does not control, and p99 lateness
  is in the hundreds of microseconds against a 5 µs bound. E0-P06 still owns
  that number. What this task can honestly claim is that ring 3 did not cost the
  schedule a tick, and that is what is asserted.
  Decided rather than assumed, so RFC 0014 exists: the design document says the
  `syscall` entry is "used strictly for channel setup", which at M3 — with the
  ring three milestones away — authorises no calls at all and produces a process
  that can only die. The reading recorded is that the entry is a door and not an
  interface: a call may exist only if it cannot be an opcode on a ring, and each
  of the three names the thing that replaces it.
  Found while building: how long a process runs cannot be measured in
  instructions. A fixed loop count spans two orders of magnitude more timer ticks
  under emulation than on a machine, and the tick count is in the boot log —
  which is a fixture. So the frame counts ticks taken *out of ring 3* and the
  process asks when to stop, which makes the same commit produce the same log on
  both. That inverted the design: the interesting thing to do between arming a
  timer and stopping it stopped being waiting, so `apic::run` became
  `start`/`wait`/`stop`.
  Also found: the ring-0 stack in the task state segment cannot be a fixed
  per-core stack. Its top would be *above* the kernel frames that are live at the
  moment ring 3 is entered, so the first timer tick from a process would push an
  interrupt frame straight through them. It has to be the stack pointer of the
  call that entered ring 3 — which is the same address the system call entry and
  the resume point use, because all three are the same claim.
  *needs:* E0-B07
- [x] **E0-B10** `M` Load `user/init` from a boot module; start the application processors. **(M3)**
  `intent/0004-a-second-core/`. Every core the machine has, brought up to the
  same point the boot processor reached — its own descriptor tables, its own
  local APIC, its own system-call entry, its own stacks with guard pages under
  them — and then left waiting. A process is built by the boot processor and run
  by another one, inside a timer window each of them opens for itself.
  *exit:* met, in the shape the criterion asks for and with one number it cannot
  produce. `init` runs on core 1: 224 bytes, from boot module 1, ordinary Rust
  with no `unsafe` in it, compiled and linked separately, copied into a frame it
  was granted. The frame's own adversary runs after it, on the same core, which
  is how every boot now checks what M4 could only assert — a table cleared
  between processes does not let the second resolve a handle the first held.
  Core 0's schedule is asserted the way it always was: every tick the window
  asked for arrived, over a window that now spans another core's ring 3.
  **What "unaffected" cannot mean here.** The criterion asks whether the second
  core costs core 0's jitter anything, and this environment cannot answer it.
  Under TCG the p50 is around 200 µs against a 1 ms period — two orders of
  magnitude past the 5 µs bound claim 0002 names — and two runs of *one* build
  moved the p99 by 69%. Two runs before and two after are indistinguishable at
  that spread, except that the count of ticks a full period late is consistently
  higher with two vCPUs, which is what an emulator scheduling two of them on a
  host would do and says nothing about a machine. No number from it is
  publishable: `F_ENVIRONMENT=container`, and `claims/0002-timer-jitter.toml`
  stays `pending` for E0-P06 and hardware.
  **The gap E0-B11 named is closed.** Revoking a frame capability now takes the
  mapping with it: the entry is cleared, this core's translation invalidated,
  every other running core told and required to acknowledge. `cap=unmap` is the
  boot — the process maps a frame, has the capability behind it revoked, reads
  the page anyway, and takes a page fault at an address it was reading a moment
  earlier. It is the only one of the eight escapes the frame does not refuse:
  what stops it is the processor.
  Two decisions were made rather than assumed, so two RFCs exist. RFC 0016
  amends what a `PerCpu` shard means — four machine words are reached by a core
  that does not own them, because a handshake cannot be per-core state, and each
  is an atomic with its ordering named at the access. RFC 0017 is E0-P08's.
  Found while building, and the reason a component is now *told* its handles
  rather than entitled to know them: generations survive `clear_all`, so the
  second process on a core finds its capabilities at the same indices and a
  later generation. The component ran correctly and the adversary that followed
  it was refused on its very first call. `door::Entry` is the answer — one
  register, the first handle the frame granted — and it made the forging sweep's
  expectation depend on what ran before, which is better rather than worse: a
  handle at a generation below the slot's is refused as *revoked* rather than as
  unknown, so the tally now distinguishes "you had this once" from "this never
  existed".
  Also found: **a crate that forbids unsafe code cannot name its own entry
  point.** `#[unsafe(no_mangle)]` and `#[unsafe(link_section)]` are unsafe
  *attributes* in this edition and `forbid` cannot be overridden by an `allow`,
  so `user/init` cannot be a binary. The placement moved to `user/init/link.ld`,
  which puts the section the entry was compiled into at the image's first byte,
  and `cargo xtask init` checks that the symbol which landed there is that one.
  Two things fell out of it and both cost a cycle: with link-time optimisation
  on, a library's rlib carries bitcode rather than machine code — so the image
  linked to *nothing*, silently, and the failure looked exactly like the entry
  point having moved, hence `[profile.init]`; and a `staticlib` crate type is
  built for the host too, where a `no_std` crate has no panic handler to borrow.
  And a prediction that did not come true, corrected rather than left standing:
  `arch::x86_64::mod` said this task would move `current_cpu` into `GS`. It did
  not. `GS` is already the ring-3 entry block and the swap between its two
  halves happens on the system-call path and only there — the interrupt stubs do
  not swap — so a core index in `GS` would be right in a system call and would
  read a process's base in the timer handler, which is the one caller on the
  critical path. That is a change to the interrupt entry path, not to that
  function, and it now says so.
  *needs:* E0-B09
- [x] **E0-B11** `L` Capability table: typed slots — Untyped, Frame, AddressSpace, Channel, Endpoint, Irq — with derive, copy and recursive revoke. **(M4)**
  `intent/0003-authority-that-can-be-taken-back/`. Thirty-two slots per process,
  a handle that is sixteen bits of index and sixteen of generation packed into
  the `u32` `Sqe.cap` already was, six rights that only ever narrow, and a
  derivation tree stored as parent handles rather than child lists — so revoke
  is a bounded walk of a fixed array with no recursion in it, and a parent link
  into a slot that has since been refilled reads as broken instead of naming the
  new occupant.
  *exit:* met. `cargo xtask cap` is E0-P08 as runs: seven boots at M4 and eight
  since E0-B10, six of which try one authority escape each and are refused with
  the exact code the escape earns, and one — `cap=grant` — which must not be
  refused at all. The frame is
  the judge, not the process: it counts answers by refusal code and compares
  against an exact tally, so a run turned down the right number of times for the
  wrong reasons fails. Every boot also runs the five properties against a real
  table **and against five tables broken on purpose**, one per property, and
  prints how many were caught.
  Decided rather than assumed, so RFC 0015 exists. RFC 0014 says a call may
  exist only if it cannot be an opcode on a ring, and this adds four that all
  could be — at M5. The reading recorded is that rule 1 governs permanent calls
  and rule 2 governs bridging ones: a ring is named by a `Channel` capability,
  so the table has to work before there is any ring to work it through, and each
  of the four names the opcode that retires it.
  The other decision worth reading is that **a copy is a child, not a sibling**.
  seL4 puts a copy beside its source, where revoking the source does not reach
  it. `docs/what-must-be-stated.html` lists *nothing can be revoked* as a
  structural drawback of the interface this replaces and answers it with
  recursive revocation — and a revoke a copy escapes does not answer it. The
  cost is that two holders of equal authority are not equal, and it is in the
  module comment rather than discovered later.
  **What the exit did not say, and it mattered.** Revoking a frame capability
  withdrew the *name* and left the *mapping*. Undoing the mapping needs an
  unmap, which needs a shootdown, which needs the second core — so it was
  E0-B10's to close, and until then a capability system that could take a name
  back was not yet one that could take the memory back. Said here rather than in
  a footnote, because it is the sentence somebody would otherwise assume the
  other way. **Closed by E0-B10**, and `cap=unmap` is the boot that says so.
  Found while building: `.Lprobe_bad_call` ended by *falling through* into
  `.Lprobe_exit` because that label happened to be next in the file. Seven new
  blocks went in between them, and `user=call` silently started running the
  capability control as well — a passing boot, with a tally two calls too high,
  and the only reason it was caught is that the tally is exact rather than a
  lower bound. A jump that depends on what is written underneath it moves when
  somebody edits above it.
  Also found: a fixture that breaks *two* things at once is caught by whichever
  check notices first, and the check it was written for stays unexercised. The
  masked-index table also collapsed the generation-versus-occupancy distinction,
  so it was caught by the revocation property instead of the totality one. The
  suite reports that as a distinct failure — caught by the wrong property — and
  the fix was to make every fixture share the real lookup and differ from it in
  exactly one step.
  And a lint reported itself: `lint-percpu` reads `&'static mut Table` as a
  mutable global, because it looks for the text `static mut`. Fixed in the lint
  rather than worked around in the kernel.
  *needs:* E0-B09
- [ ] **E0-B12** `L` The first ring: layout, cursor protocol, suppression, two opcodes — `NOP` and `WRITE_SERIAL`. **(M5)**
  *exit:* one million NOPs in batches of 32, under 50 ns per operation, recorded as a gating claim.
  *needs:* E0-B11
- [ ] **E0-B13** `M` Bind `Producer`/`Consumer` to a mapped shared region with a validated, negotiated `ChannelHeader`, replacing the borrowed-memory placeholder.
  *exit:* the hostile-header tests in `ring/tests` run against a real mapping; every invalid header tears the channel down without a panic.
  *needs:* E0-B12, E0-D03
- [ ] **E0-B14** `M` State tree v0: the kernel publishes its counters into a read-only mapping.
  *exit:* a user process reads the tree and prints it; the tree's snapshot hash is stable across a re-read with no intervening change.
  *needs:* E0-B12, E0-D05
- [ ] **E0-B15** `M` Doorbell: kernel IPI path first; user-interrupt path behind a negotiated feature bit.
  *exit:* both paths pass the same suppression test; doorbells per operation under load is recorded.
  *needs:* E0-B12
- [x] **E0-B16** `M` The development container: `docker/`, reading the toolchain pin from `rust-toolchain.toml` so the image cannot drift from the tree.
  *exit:* met, at the second attempt. `.\docker\dev.ps1 build` then `lint`, `test` and `run` all pass on a machine whose only prerequisite is Docker, under Windows PowerShell 5.1 and PowerShell 7 both. The environment stops being a step that is not in the tree.
  Marked done before it was, and worth recording as the second instance of the same lesson as `E0-B02`: the wrapper had never run once. `Compose`, `Exec` and `Xtask` each took a parameter named `$Args`, which is a PowerShell automatic variable — it binds nothing, silently, so every verb degenerated into a bare `docker compose -f <file>` that printed the usage screen and exited **0**. A failure that reports success is worse than one that does not, and only running the thing finds it. Underneath that, the script was saved as UTF-8 with no BOM, so 5.1 decoded its em-dashes as ANSI smart quotes and could not parse the file at all, despite the docstring promising 5.1 support.
- [x] **E0-B17** `S` Teach `xtask` to honour `CARGO_TARGET_DIR` instead of assuming `./target`.
  One function, `target_dir()`, because the assumption was wrong in three places and each failed differently. A relative value resolves against the **current working directory** and not against the workspace root, because that is what cargo documents and does — being tidier than cargo here would put the image in one place and look for it in another, which is the failure the task exists to remove.
  *exit:* met. `CARGO_TARGET_DIR=moved-target cargo xtask run` builds into `moved-target/` and boots from it to `M0 ok`.
  Found while fixing, and it is the third place rather than a fourth task: `xtask coverage` set `LLVM_PROFILE_FILE` to a *relative* path, which each test binary resolves against its own working directory rather than cargo resolving it against the workspace — so the profiles scattered into `abi/target/`, `env/target/`, `ring/target/` and `bench/target/`. The tree carried four of those directories, and `.gitignore` had been widened to an unanchored `target/` to hide them. The pattern stays, with the comment now saying the cause is fixed rather than describing it as the intended behaviour: an ignore rule that only worked by accident is worth keeping on purpose.
  Also: `rust_sources()` skipped a directory *named* `target`, so with the build output moved inside the tree every lint in `xtask` would have read generated sources and reported findings against them. It now skips the directory `target_dir()` actually names.
  *needs:* E0-B16

### Prove

- [ ] **E0-P01** `M` CI pull-request gate under ten minutes: lint, test, run, claims.
  Half the budget was being spent twice: `ci` triggered on both `push` and
  `pull_request`, so every commit on a branch with a pull request open built the
  whole matrix — both AArch64 runners and the QEMU boot — against an identical
  tree. Fixed; the gate is now the pull request, and `main` is checked again
  after a merge because a merge commit is a tree nobody tested. Still open: the
  claims job, the ten-minute measurement, and the red half of the exit.
  *exit:* the workflow is green on a pull request that changes one line, and red on a pull request that regresses a gating claim.
  *needs:* E0-B01
- [ ] **E0-P02** `M` **The reproduction check.** Two runs of the same `(seed, commit)` on two different runners produce a byte-identical execution trace hash.
  *exit:* CI job green; a deliberately introduced unseeded read of time turns it red.
  *needs:* E0-B01
- [x] **E0-P03** `S` Coverage instrumentation reported per crate by `xtask coverage`, while the kernel is small enough for it to be trivial.
  `llvm-profdata` and `llvm-cov` come from the pinned toolchain's own sysroot, the way `llvm_tool` already took the linker and `objcopy`. That is the whole reason `cargo-llvm-cov` is not a prerequisite: a coverage number produced by a separately installed tool is a number whose version nobody pinned, which is the ambient dependency the container exists to remove — and it keeps the command working in the `dev` image rather than only in `full`.
  *exit:* met. Four crates and a total, printed by `cargo xtask coverage` and written to `target/coverage/summary.json`, which the CI job uploads as an artifact so a fall is answerable from two runs rather than from memory. Today: abi 85.42%, bench 58.25%, env 95.72%, ring 93.01%, total 84.24%.
  **Nothing here gates, on purpose.** A coverage threshold rewards tests written to touch lines rather than to catch anything, and this repository already has a mechanism for a number that must hold — `claims/`, with a baseline and a reproduction. So this one is published and watched. Lowering it is then a visible fact rather than a passing build.
  Two decisions inside the measurement, both of which change the number. `/tests/` is excluded, because an integration test measures its own execution and reports itself as covered — that raises the figure without covering anything, and the question being asked is how much of the *library* the tests reach. And a report row whose first path component is not a directory with a `Cargo.toml` in it is skipped rather than guessed at, so a change to the workspace layout shows up as a crate going missing from the table instead of as a plausible wrong total.
  Found while building: the second cargo invocation — `--no-run --message-format=json`, which asks where the test binaries landed — has to carry the same `RUSTFLAGS` as the first. Without it that is a different build with a different fingerprint, so cargo rebuilds everything uninstrumented and reports *those* binaries. llvm-cov then measures objects with no counters against a profile that has them, and reports zero coverage rather than an error.
- [ ] **E0-P04** `M` Bench harness records full distributions with p50/p99/p99.9, plus instructions and joules per operation, marked absent until the counters exist.
  *exit:* `bench` output for one workload contains a histogram, not a mean.
- [ ] **E0-P05** `S` Claim 0001, ring submit latency, moves from `pending` to measured and `gating`.
  *exit:* `cargo xtask claims` reports it green; a deliberate 20% regression fails the build.
  *needs:* E0-B12, E0-P04
- [ ] **E0-P06** `M` Claim 0002, timer jitter: p99 under 5 µs for a 1 kHz timer over 60 seconds, gating from M2 onward.
  *exit:* recorded with the reservation conditions from RFC 0007 named in the claim.
  *needs:* E0-B07, E0-D04
- [ ] **E0-P07** `M` Litmus tests for the cursor protocol run in CI on x86-64 **and** AArch64.
  *exit:* both jobs green; the AArch64 job is not allowed to be advisory.
- [x] **E0-P08** `L` The capability negative suite, as code. A process cannot name a capability it was not given, forge a handle, use a revoked handle, exceed granted rights, or panic the kernel by trying. **(M4 exit criterion)**
  Both halves, at last. The suite is in the gate — `cargo xtask cap`, eight
  boots, and the five properties run at every boot against a real table and
  against five broken on purpose. All five hold, and all five now have a
  mutation that makes them fail.
  Four of the five have a fixture, checked in as `cap::properties::Flaw` and
  asserted to be caught by the property it breaks and no other. The fifth could
  not have one of that shape, and that was the whole of what this task still
  owed: a fixture that panics takes the machine down rather than being caught,
  and there is no host harness for kernel logic to catch it in —
  `kernel/Cargo.toml` says why there is not.
  So the fifth has a **build** instead. `cargo xtask mutate` compiles the kernel
  with `mutate-unchecked-index`, which removes the bounds check from the
  capability table's handle lookup, boots it into the forging sweep, and
  requires the boot to go red *with a kernel panic in the log* — then compiles
  it without and requires the same boot to go green. Neither half means anything
  alone: a red boot with a defect proves nothing if the same boot is red without
  one, and a green boot proves nothing about whether the suite can fail. It runs
  in `cargo xtask verify` and in CI.
  Decided rather than assumed, so RFC 0017 exists. The mechanism is a different
  one from the other four and is argued as one, including the part that is a
  real cost: the defect is in the shipped source, behind a feature that is off
  by default, in one function, with an `allow` that names itself and
  `cargo xtask lint-mutations` refusing to let it become a default. That is the
  same trade `properties::Flawed` already makes, made a second time.
  It also differs from the real lookup in exactly one step, which is the lesson
  E0-B11 recorded the hard way: a fixture that breaks two things at once is
  caught by whichever check notices first, and the check it was written for
  stays unexercised. Everything else about a lookup — the generation, the
  occupancy, the refusal codes — is shared with the function it defects.
  Worth recording as a scar, because it is invisible until it bites: the module
  denies `indexing_slicing` rather than *forbidding* it, and that is now
  load-bearing. Tighten it to `forbid` and the mutation build stops compiling,
  which means property five quietly loses its second half — the harness would
  fail to build rather than fail to catch.
  The compile-time half stays and is not redundant: `deny(indexing_slicing,
  unwrap_used, expect_used, panic, unreachable)` over the module is what stops
  the construct being written by accident, and the mutation build is what says
  it would be noticed if it were.
  *exit:* met. All five hold, and each has a mutation that makes it fail —
  four fixtures and one build.
  *needs:* E0-B11
- [ ] **E0-P09** `M` Exercise the fault-injection hook: one seeded fault class per subsystem that exists, using the protocol-aware site labels already in `env/src/sim.rs`.
  *exit:* a seeded run injects a failure at a named site and the system handles it; the same seed reproduces it.
- [ ] **E0-P10** `S` `xtask claims` publishes a machine-readable snapshot, and the design documents render their numbers from it instead of restating them.
  *exit:* changing a claim value changes the rendered document with no separate edit.
- [ ] **E0-P11** `S` Store the full measurement history from the first measurement, so change-point detection has something to reason about at phase 02.
  *exit:* every CI run appends its distributions to a versioned history; the history survives a rebase.
- [ ] **E0-P12** `S` Panic path test: a kernel panic prints something useful and exits with a code CI can distinguish from success and from a hang.
  *exit:* three CI assertions — clean exit, panic exit, timeout — each triggered by a fixture.
- [ ] **E0-P13** `S` Record boot time to `M0 ok` as a tracked, non-gating claim.
  *exit:* the number is in `claims/` with `status = "tracked"`.
- [ ] **E0-P14** `S` CI runs inside the development image rather than installing tools per job.
  *exit:* every job in `ci.yml` uses the published image; the per-job `apt-get install` disappears; "works on my machine" and "works in CI" become the same statement.
  *needs:* E0-B16
- [ ] **E0-P15** `S` The claim harness refuses to record a timing measurement when `F_ENVIRONMENT` says it is not a measurement environment.
  *exit:* a claim run inside the container reports refused-with-a-reason rather than a number; the same run on a bare-metal host records normally.
  *needs:* E0-B16, E0-P04

### Release

- [ ] **E0-R01** `M` `cargo xtask release` produces the full package: source tag, claims snapshot, content-addressed QEMU image, seed corpus, baseline configurations, and a dependency manifest.
  *exit:* the package builds twice on two machines and hashes identically.
  *needs:* E0-D08, E0-P02
- [ ] **E0-R02** `M` `cargo xtask reproduce <claim>` re-runs any published claim from a clean checkout.
  *exit:* a person who has never seen the repository reproduces claim 0001 from the README in under thirty minutes, including toolchain install.
  *needs:* E0-R01
- [x] **E0-R03** `S` Update `BOOTSTRAP.md` — the honest-status section stops saying "never compiled".
  *exit:* the page describes what is actually true on the day of release.
- [ ] **E0-R04** `S` **Release 0.1.** Two gating claims, the reproduction command, and the honest-status page.
  *exit:* the tag exists, the package is attached, and the claims snapshot is in it.
  *needs:* E0-P05, E0-P06, E0-R01, E0-R02, E0-R03

- [x] **E0-B19** `M` Write exclusive-or execute on the kernel mapping, and a direct map that is never executable.
  The kernel window is now 4 KiB pages with per-section permissions from the linker script: text executable and not writable, constants neither, everything else writable and not executable. `CR0.WP` makes the read-only half apply to ring 0, `EFER.NXE` makes the no-execute half legal, and both are reported at boot rather than assumed.
  *exit:* met, from both sides. `cargo xtask fault nx` jumps into the direct map and faults with the instruction-fetch bit set on a page that is present and readable; `cargo xtask fault wx` writes to `kmain` and faults with the write bit set. A protection nothing tries to violate is a protection nobody has checked.

- [x] **E0-B20** `S` Gibibyte pages for the direct map where the processor has them.
  *exit:* met. `-cpu max` reports "direct map in 1 GiB pages"; the default `qemu64` model has no such feature and falls back to 2 MiB, so both paths are exercised without a flag.

> ### Gate G0
> A capability-restricted user process communicates with the kernel entirely
> through a ring, touches nothing it was not handed, and the whole suite runs
> under one command in CI with timer jitter and ring throughput gating every
> commit. Plus the two things phase 00 does not currently promise and should:
> `(seed, commit)` reproduces byte for byte on someone else's machine, and a
> stranger can reproduce both published numbers.

---

## E1 — The datapath

*Phase 01. Rings everywhere, drivers in user space, the simulator that makes
everything after this cheap to debug.*

**Effort:** 1–2 person-years · **Risk:** medium · **Ends at:** gate G1, release 0.2

### Decide

- [ ] **E1-D01** `M` Write RFC 0008 — no fork, no signals: spawn from a manifest, one control ring, the powerbox grant.
  *exit:* RFC merged before a second long-lived component exists, because retrofitting a lifecycle is worse than designing one.
- [ ] **E1-D02** `M` Write RFC 0005 — speculation is a boundary the language does not draw. Three domain kinds, assigned in the topology.
  *exit:* RFC merged before any untrusted or imported code is hosted; the topology format carries a domain field.
- [ ] **E1-D03** `M` Settle buffer registration and ownership transfer, including the shared-virtual-memory path.
  *exit:* the ownership rules are expressed in types, and a misuse fails to compile in a fixture.
- [ ] **E1-D04** `M` Record the driver-container shape: declaratively routed capabilities, typed protocol, declared restart policy.
  *exit:* one manifest schema, with a worked example for virtio-blk.
- [ ] **E1-D05** `S` Deadline inheritance bounds — how far a caller's deadline propagates, and what stops a component claiming urgency forever.
  *exit:* the rule is written before the first starvation bug, as the resource document asks.

### Build

- [ ] **E1-B01** `L` IOMMU configuration and per-component domains.
  *exit:* a driver component provably cannot address memory outside its grant; the attempt is a fault, not a corruption.
  *needs:* E0-B11
- [ ] **E1-B02** `L` virtio-blk in user space.
  *exit:* read and write through a ring, zero copies on the data path, verified by counter.
  *needs:* E1-B01
- [ ] **E1-B03** `L` virtio-net in user space.
  *exit:* packets in and out through a ring; receive lands in a registered buffer.
  *needs:* E1-B01
- [ ] **E1-B04** `L` virtio-gpu in user space, minimal.
  *exit:* something appears on the framebuffer, submitted through a ring.
  *needs:* E1-B01
- [ ] **E1-B05** `L` Component supervisor: spawn from a manifest, restart policy, control ring delivery.
  *exit:* E1-P06 passes.
  *needs:* E1-D01, E1-D04
- [ ] **E1-B06** `M` Deadline propagation across rings; every resource scheduler orders by the same field.
  *exit:* a hard-class read overtakes queued batch work in a device queue, measurably.
  *needs:* E1-B02, E1-D05
- [ ] **E1-B07** `L` Admission control v1: CPU reservations with a schedulability test that can refuse.
  *exit:* an over-subscribed reservation is refused with `ADMISSION`; a granted one meets its deadline under adversarial load.
  *needs:* E0-D04
- [ ] **E1-B08** `L` User-level runtime: cores allocated to runtimes, preemption only at allocation boundaries, park-cleanly notice on reclaim.
  *exit:* async work under load produces zero kernel entries on the hot path, counted.
  *needs:* E1-B05
- [ ] **E1-B09** `M` User-interrupt doorbell where the hardware offers it, with the kernel path as the negotiated fallback.
  *exit:* both paths measured; the notification-cost claim is recorded with the hardware named.
  *needs:* E0-B15
- [ ] **E1-B10** `M` Registered buffer sets, and the shared-virtual-memory path behind its feature bit.
  *exit:* both paths pass the same ownership tests; the registration cost is measured.
  *needs:* E1-D03

### Prove — this is the epoch where the testing environment becomes real

- [ ] **E1-P01** `XL` **The deterministic simulator.** Virtual time, seeded scheduling and ordering, device models for blk, net and gpu, and component substitution.
  *exit:* a whole boot-to-workload run executes under simulation and reproduces byte-identically from `(seed, commit)`.
  *needs:* E0-P02
- [ ] **E1-P02** `L` Fault classes in the simulator: allocation failure, translation fault, device page-fault latency, peer death mid-operation, torn doorbell, partial write, delayed completion.
  *exit:* each class has a scenario, and each scenario has a system response that is asserted rather than observed.
  *needs:* E1-P01
- [ ] **E1-P03** `M` Nightly seed sweeps: N seeds across M scenarios, with automatic minimisation of any failure to a reproduction command.
  *exit:* an injected bug is found by the sweep and reported as `(seed, commit)` plus a one-line repro, with no human triage.
  *needs:* E1-P02
- [ ] **E1-P04** `L` Hostile-peer fuzzer: a peer that writes arbitrary values to the shared header and cursors, restarts mid-operation, and lies about its epoch.
  *exit:* one billion hostile operations with no kernel panic, no memory unsafety, and no hang.
  *needs:* E0-B13
- [ ] **E1-P05** `L` Structure-aware submission-entry fuzzer with coverage feedback; the corpus is committed to the tree.
  *exit:* coverage of the entry-validation path exceeds 95%, and the corpus is a release artifact.
  *needs:* E0-P03
- [ ] **E1-P06** `M` Driver chaos: kill each driver component at random under sustained load.
  *exit:* no client observes anything except added latency; the blast-radius claim becomes gating.
  *needs:* E1-B05
- [ ] **E1-P07** `L` Kani on the capability properties — the same five the negative suite asserts, proved rather than sampled.
  *exit:* the proofs run in CI on a schedule, and a mutation to the capability code fails them.
  *needs:* E0-P08
- [ ] **E1-P08** `M` Simulator snapshot and restore, so a long scenario bisects in seconds rather than hours.
  *exit:* a failure at simulated minute 40 is re-entered at minute 39 without re-running the first 39.
  *needs:* E1-P01
- [ ] **E1-P09** `M` Write the test taxonomy: for each class of bug, which layer catches it and how often that layer runs.
  *exit:* the table exists, and every gap in it is either scheduled or explicitly accepted.
- [ ] **E1-P10** `M` Claims for the datapath: ring submit under load, doorbells per operation, copies per operation, kernel entries per operation.
  *exit:* four claims, gating, each with a tuned-Linux baseline where one exists.
  *needs:* E1-B09
- [ ] **E1-P11** `M` Cross-architecture CI: the AArch64 job builds and runs the same suite under emulation.
  *exit:* green, and no test is skipped on AArch64 without a recorded reason.

### Release

- [ ] **E1-R01** `M` Publish the simulator as a usable tool, with the seed corpus and the scenario set.
  *exit:* a third party runs a seed sweep against their own checkout using the published command.
  *needs:* E1-P03
- [ ] **E1-R02** `S` **Release 0.2.** The datapath claims, the simulator, the fuzzing corpus.
  *exit:* a third party runs a seed sweep and the four datapath claims from the package alone.

> ### Gate G1
> A driver is killed under sustained load and the system does not notice.
> A bug injected into any component is found by an overnight seed sweep and
> arrives as a reproduction command rather than as a symptom.

---

## E2 — State

*Phase 02. The object store, the declarative system, and the two properties
that follow from them: rollback that works and update that does not reboot.*

**Effort:** 1.5–3 person-years · **Risk:** medium · **Ends at:** gate G2, release 0.3

### Decide

- [ ] **E2-D01** `M` Write RFC 0012 — an update is a generation swap, and the root hash is the attestation.
  *exit:* RFC merged; the rollback metric changes from "one reboot" to "one generation swap, and a reboot only when the frame changed".
- [ ] **E2-D02** `M` Mutable extents: the design for the workload content addressing is worst at, written as a second object kind rather than as a unification.
  *exit:* design merged, naming the granularity, the snapshot boundary, and the workload it is bad at.
- [ ] **E2-D03** `S` Garbage collection policy: mark from live roots, sweep by live fraction, batch class, roots pinned explicitly.
  *exit:* written, with the three invariants E2-P03 will assert stated as invariants rather than as behaviour.
- [ ] **E2-D04** `M` The state-transfer protocol a component implements to be updated in place, and what it declares when it cannot.
  *exit:* schema merged; the virtio-blk driver from E1 declares one, and E2-P08 swaps it.
  *needs:* E2-D01

### Build

- [ ] **E2-B01** `L` Blob store: content addressing, content-defined chunking, the on-disk format.
  *exit:* write, read back and verify a million blobs; an edit near the start of a large object re-chunks only a bounded region.
- [ ] **E2-B02** `L` The zoned mapping: sequential fill, seal, copy-forward, reset — the collector and the device agreeing for once.
  *exit:* a full fill-and-collect cycle on a zoned device or its emulation, with write amplification recorded.
  *needs:* E2-B01
- [ ] **E2-B03** `L` The index: paths, metadata and semantic attributes to hashes, embedded rather than a service.
  *exit:* a query returns a hash without crossing a component boundary, measured against a tree walk over the same data.
  *needs:* E2-B01
- [ ] **E2-B04** `L` The configuration evaluator: one expression to one generation root.
  *exit:* the same expression evaluates to the same root hash on two machines, checked by E2-P06.
- [ ] **E2-B05** `L` The assembler: instantiate a topology from a root, route capabilities declaratively, bind drivers by declared properties.
  *exit:* boot is a pure function of one hash — the same root produces a byte-identical topology, and a driver that fails to start leaves its subtree unstarted rather than failing the boot.
  *needs:* E2-B04, E1-B05
- [ ] **E2-B06** `L` Generation swap: instantiate alongside, transfer state, swap routing at a quiescent point, retire.
  *exit:* E2-P08 passes.
  *needs:* E2-D04, E2-B05
- [ ] **E2-B07** `M` Measured boot into a root of trust; the generation root hash is the machine's identity.
  *exit:* the machine answers "what are you running" with one hash, and any modification produces a different one.
  *needs:* E2-B05
- [ ] **E2-B08** `M` The read path: hash to zone and offset, direct memory access into the caller's buffer, no page cache second copy.
  *exit:* copies per read is zero, counted rather than asserted; resident bytes per unit of work recorded.
  *needs:* E2-B02

### Prove

- [ ] **E2-P01** `L` Crash-consistency torture: a power-cut model in the simulator, cutting at every write boundary in a publish.
  *exit:* every cut leaves either the old root or the new one, never a third thing, across a full sweep of cut points and seeds.
  *needs:* E1-P01, E2-B01
- [ ] **E2-P02** `M` Property tests for chunking and deduplication.
  *exit:* an edit at offset X re-chunks a bounded region around X and nothing after it, for randomly generated edits and object sizes.
- [ ] **E2-P03** `M` Collector invariants as properties: nothing reachable is swept, a reset zone holds no live blob, collection never starves a deadline-class read.
  *exit:* all three hold while collection runs concurrently with adversarial allocation and a hard-class reader.
  *needs:* E2-B02
- [ ] **E2-P04** `L` Verus on the frame's invariants, now that the frame has stopped moving.
  *exit:* the chosen invariants are proved, the proofs run on a schedule, and a mutation to the frame fails them.
- [ ] **E2-P05** `M` Whole-system state comparison: two runs, two hashes, and a diff that descends to the divergent subtree.
  *exit:* an injected divergence is localised to a named subtree automatically, with no human reading a log.
  *needs:* E0-B14, E1-P01
- [ ] **E2-P06** `M` Reproducible builds verified across two machines and two dates.
  *exit:* identical generation root hash, checked weekly; a non-reproducible input fails the job and names itself.
- [ ] **E2-P07** `M` Rollback test.
  *exit:* break the system deliberately, roll back from the boot menu, and verify the restored generation is bit-identical to what it was.
  *needs:* E2-B05
- [ ] **E2-P08** `M` Live-swap test.
  *exit:* replace a running component under sustained load; no client observes a dropped operation, and the state transfer is verified rather than assumed.
  *needs:* E2-B06
- [ ] **E2-P09** `M` Change-point detection over the stored measurement history, replacing thresholds.
  *exit:* a 3% regression injected into the history is detected; ordinary run-to-run noise over the same period is not.
  *needs:* E0-P11
- [ ] **E2-P10** `M` The write-amplification claim, on zoned hardware if available and on an emulated device clearly marked if not.
  *exit:* bytes written to the device per byte written by the application, against a tuned Linux filesystem on the same device.
  *needs:* E2-B02

### Release

- [ ] **E2-R01** `S` **Release 0.3.** Storage and generation claims; the rollback and live-swap demonstrations; the attestation story.
  *exit:* the package reproduces, and the rollback demonstration runs from the release image on a stranger's machine.

> ### Gate G2
> Break the system deliberately and roll it back. Replace a running component
> without dropping work. Cut power at every write boundary in a publish and
> never observe a state that was not one of the two intended ones.

---

## E3 — The interface

*Phase 03. The longest and hardest epoch, and where most projects die. Budget
accordingly and narrow early rather than late.*

**Effort:** 4–10 person-years · **Risk:** high · **Ends at:** gate G3, release 0.4

- [ ] **E3-00** `M` **Decompose this epoch before starting it.** Everything below is coarse on purpose; each task becomes five to fifteen tasks with exits when the epoch opens.
  *exit:* E3 contains no `XL` task without a decomposition.

### Decide

- [ ] **E3-D01** `L` The semantic node vocabulary, version 1 — small enough to be unambiguous, large enough that applications do not escape into opaque canvases.
  *exit:* the vocabulary expresses three deliberately chosen hard interfaces on paper — a settings panel, a file browser, a timeline — with no node marked "other".
- [ ] **E3-D02** `M` Canvas participation: what a self-rendering surface must still declare about its content.
  *exit:* the timeline case is expressible — clips, tracks, times, selections, addressable and scriptable while the pixels stay custom.
- [ ] **E3-D03** `M` The typed design-token layer: what is themeable, what is range-checked, what a theme cannot break.
  *exit:* a deliberately hostile theme cannot produce an unreadable or unlayoutable interface, demonstrated by test.
- [ ] **E3-D04** `S` The renderer fallback ladder, decided before it is needed.
  *exit:* written — compute path renderer, hybrid, CPU, tessellating floor — with the claim each rung costs named.

### Build

- [ ] **E3-B01** `XL` Retained scene graph for the whole machine; deltas in, no pixel buffers.
  *exit:* boundary crossings per UI frame under 10, counted rather than estimated.
  *needs:* E3-00
- [ ] **E3-B02** `XL` GPU compute path rasterisation over the chosen backend.
  *exit:* the CPU raster segment of the budget is zero, measured; the fallback ladder is exercised on hardware that cannot run the top rung.
  *needs:* E3-D04
- [ ] **E3-B03** `XL` Text: shaping, bidirectional layout, and the parts of this that are always underestimated.
  *exit:* a corpus of scripts and directions renders correctly against reference images, in CI.
- [ ] **E3-B04** `L` Input path: timestamped at the driver, predicted forward to the next scanout.
  *exit:* a full frame of latency removed, shown as before-and-after on the rig.
  *needs:* E3-P01
- [ ] **E3-B05** `L` Explicit synchronisation throughout — timeline semaphores, never implicit driver waits.
  *exit:* no implicit wait appears in a frame trace; frame time is bounded rather than typical.
- [ ] **E3-B06** `XL` The semantic layer and its projections: display, remote, screen reader, agent.
  *exit:* E3-P04 passes.
  *needs:* E3-D01
- [ ] **E3-B07** `L` Degradation policy: miss quality rather than the frame, and record which was chosen.
  *exit:* under 2x overload the frame rate holds and the quality reduction is visible in the state tree, per frame.

### Prove

- [ ] **E3-P01** `L` **The photodiode rig.** External input injection, photodiode capture, no software timestamp anywhere in the measurement path.
  *exit:* the rig reproduces a known measurement on a conventional machine within its own error bar — the instrument is calibrated before it is trusted.
- [ ] **E3-P02** `M` The latency claim: input to photon, p99 under 14 ms, on the rig, under load.
  *exit:* recorded as gating, with the load stated and any firmware-jitter observation reported beside it.
  *needs:* E3-P01
- [ ] **E3-P03** `M` The parity claim: a derived scene graph costs no more than an authored one.
  *exit:* published either way. A negative result invalidates the pillar and is reported as such rather than re-scoped.
  *needs:* E3-B06
- [ ] **E3-P04** `M` Four projections from one application with no projection-specific application code.
  *exit:* local display, a remote client at different density and refresh rate, a screen reader, and an agent driving declared intents — all from one unmodified application.
  *needs:* E3-B06
- [ ] **E3-P05** `M` Semantic-tree assertions replace screenshot diffing across the whole UI suite.
  *exit:* no test in the tree compares images; node identity survives a deliberate visual redesign.
- [ ] **E3-P06** `M` Frame-drop behaviour under adversarial load, with the degradation choice recorded per frame.
  *exit:* a sweep of load profiles produces no missed frame, only recorded quality reductions.
- [ ] **E3-P07** `M` Energy per frame, external meter rather than a model.
  *exit:* joules per frame against a tuned Linux compositor, same hardware, same content.

### Release

- [ ] **E3-R01** `S` **Release 0.4.** The latency claim, the four-projection demonstration, and the parity result whichever way it lands.
  *exit:* the rig's method is documented well enough for a third party to build one.

> ### Gate G3
> Input to photon under 14 ms at p99, measured by photodiode under load, and one
> application driving four projections with no projection-specific code.

---

## E4 — The platform

*Phase 04. The point at which somebody outside the team can build something.*

**Effort:** 2–4 person-years · **Risk:** medium · **Ends at:** gate G4, release 0.5

- [ ] **E4-00** `S` Decompose this epoch before starting it.
  *exit:* no `XL` task without a decomposition.

### Decide

- [ ] **E4-D01** `L` Interface definitions for every system service, with availability annotations from the first version rather than the third.
  *exit:* every service has one, and E4-P01 generates its compatibility matrix from them mechanically.
  *needs:* E0-D03
- [ ] **E4-D02** `M` The powerbox interaction: what a grant looks like to a person, given that the bet's stated failure mode is usability.
  *exit:* a person unfamiliar with capabilities grants access to one file and not to its directory, without being taught anything, with at least five participants.
- [ ] **E4-D03** `M` The package and generation model for third-party software.
  *exit:* a third-party generation is publishable and rollback-able by the same mechanism as a system one.

### Build

- [ ] **E4-B01** `XL` Component runtime and bindings for the chosen component model.
  *exit:* a component written in a second language runs, holds only routed authority, and is swappable without recompiling the system.
  *needs:* E4-D01
- [ ] **E4-B02** `L` The powerbox: pickers that are grants.
  *exit:* no interface exists by which a component asks for authority it was not routed; E4-P03 proves it.
  *needs:* E4-D02, E1-D01
- [ ] **E4-B03** `L` Developer tooling: build, run, debug, and read the state tree of your own component.
  *exit:* a newcomer goes from empty directory to running component in under an hour, timed with a real newcomer.
- [ ] **E4-B04** `M` Third-party generation publishing, content-addressed like everything else.
  *exit:* an outside build produces the same hash as ours, verified by E4-P04.

### Prove

- [ ] **E4-P01** `L` The compatibility matrix: current against N-1 peers in both directions, plus refusal tests below the floor.
  *exit:* green in CI; a deliberate incompatible change fails it with a `PEER` error naming the missing feature.
  *needs:* E0-D03
- [ ] **E4-P02** `M` A conformance suite generated from the interface definitions.
  *exit:* a component claiming an interface must pass it; a deliberately non-conforming component fails on the specific clause.
- [ ] **E4-P03** `M` The capability negative suite at component scale.
  *exit:* the five M4 properties hold across the full component topology, not just against the kernel.
- [ ] **E4-P04** `M` An outside party reproduces a build and gets the same hash.
  *exit:* done by someone with no access beyond the public tree, and their transcript is published.

### Release

- [ ] **E4-R01** `S` **Release 0.5.** The SDK, the conformance suite, the compatibility matrix.
  *exit:* published with a getting-started path that E4-B03 timed.

> ### Gate G4
> Someone outside the team ships something, against a negotiated ABI, without
> reading the kernel source.

---

## E5 — Real hardware

*Phase 05. Where benchmarks on emulated devices become evidence — and where the
plan is most likely to need its fallback.*

**Effort:** 2–5 person-years · **Risk:** high · **Ends at:** gate G5, release 0.6

- [ ] **E5-00** `S` Decompose this epoch before starting it, and re-cost it: the shim is the least predictable work in the plan.
  *exit:* no `XL` task without a decomposition, and a re-estimate recorded against the effort table.

### Decide

- [ ] **E5-D01** `M` Name the machine. One workstation, exactly specified.
  *exit:* a bill of materials anyone can buy, published with the first hardware claim, so a third party can reproduce on identical silicon.
- [ ] **E5-D02** `M` Shim scope, and the in-frame fallback policy if a graphics driver cannot be hosted out of frame.
  *exit:* the API surface to implement is enumerated, and the fallback is written down with its cost to the unsafe-code metric stated in advance rather than discovered.

### Build

- [ ] **E5-B01** `XL` The kernel-API shim, sized to the imported driver set.
  *exit:* unmodified upstream driver source compiles against it, and an upstream update applies without a port.
  *needs:* E5-D02
- [ ] **E5-B02** `XL` The imported graphics stack as an isolated, IOMMU-confined, restartable component.
  *exit:* it renders, it survives being killed under load, and E5-P02 has a number.
  *needs:* E5-B01
- [ ] **E5-B03** `L` Native NVMe with zoned support — the code path the storage claims live on.
  *exit:* the E2 storage claims re-measured on real hardware, with the emulated numbers retained beside them.
- [ ] **E5-B04** `L` Native network interface driver with zero-copy receive.
  *exit:* packets per second per core and joules per packet, against a tuned Linux using its own zero-copy path.
- [ ] **E5-B05** `M` Native audio, on the deadline path.
  *exit:* round trip under 5 ms held under adversarial load, measured externally.
- [ ] **E5-B06** `M` Native input path with interrupt-time timestamping.
  *exit:* the E3 latency claim holds on real hardware, on the rig.
- [ ] **E5-B07** `L` Power management implementing RFC 0006: computed idle depth, device power states, ring-quiesced suspend and resume.
  *exit:* E5-P03 and E5-P04 pass.
  *needs:* E0-D06

### Prove

- [ ] **E5-P01** `L` Hardware lab automation: netboot, serial capture, power cycling, unattended bisect on real silicon.
  *exit:* a regression on hardware is bisected overnight with no human present, and reports a commit.
- [ ] **E5-P02** `M` The isolation-cost claim: what confining a modern graphics driver actually costs. Nobody has published this.
  *exit:* a number, with the same driver measured in-frame and out-of-frame on the same machine.
  *needs:* E5-B02
- [ ] **E5-P03** `M` Energy claims on real silicon, external meter, against a tuned Linux with a deliberately chosen governor.
  *exit:* joules per operation and idle-state residency for at least three workload classes.
  *needs:* E5-B07
- [ ] **E5-P04** `M` Suspend and resume as an automated test.
  *exit:* ten thousand cycles, unattended, with no failure and no unaccounted state.
  *needs:* E5-B07
- [ ] **E5-P05** `L` Long soak: a week of continuous mixed workload with the state tree recorded throughout.
  *exit:* no drift in memory, latency distribution or zone utilisation across the week; the recording is a release artifact.

### Release

- [ ] **E5-R01** `S` **Release 0.6.** Every claim re-measured on real hardware, with the emulated numbers retained for comparison.
  *exit:* each claim carries both numbers and the difference is discussed rather than hidden.

> ### Gate G5
> An unmodified vendor driver runs confined, restartable, with no ambient
> authority, on real hardware — and the cost of that confinement is a published
> number.

---

## E6 — The AI substrate

*Phase 06. The newest research surface, and the one with no settled answers
anywhere.*

**Effort:** 2–5 person-years · **Risk:** high · **Ends at:** gate G6, release 0.7

- [ ] **E6-00** `S` Decompose this epoch before starting it, and re-survey the field first: this is the area most likely to have moved since the design was written.
  *exit:* decomposition merged, with the prior art re-surveyed and the design amended or reaffirmed in writing.

### Decide

- [ ] **E6-D01** `M` Accelerator admission on hardware that cannot be preempted mid-kernel, and how the submission quantum is bounded.
  *exit:* written, including the honest case — where a vendor runtime will not cooperate, the quantum is whatever it chooses and every number measured through it is reported as bounded by it.
- [ ] **E6-D02** `M` Key-value cache as a memory tier: residency, eviction and sharing across tenants.
  *exit:* expressed as tier policy and bandwidth class under the existing resource discipline, adding no new machinery.
- [ ] **E6-D03** `M` Agents as capability principals: scoping, revocation, and the audit trail immutable storage gives for free.
  *exit:* an agent's authority is a routed set and its actions are a generation history; both are demonstrated revocable and reversible.

### Build

- [ ] **E6-B01** `XL` Heterogeneous co-scheduling across CPU, GPU and accelerator with quality-of-service classes.
  *exit:* E6-P01 passes.
  *needs:* E6-D01
- [ ] **E6-B02** `L` Weights mapped from the content-addressed store rather than copied per process.
  *exit:* E6-P02 passes.
- [ ] **E6-B03** `L` The semantic index as a capability-gated service.
  *exit:* a query is authorised per grant; an unauthorised query is an `AUTHORITY` refusal, not an empty result.
- [ ] **E6-B04** `M` Agent principals with scoped, revocable, auditable capability sets.
  *exit:* revoking an agent mid-task stops it, and every action it took is enumerable and reversible from the generation history.
  *needs:* E6-D03

### Prove

- [ ] **E6-P01** `M` The concurrency claim: inference and audio both meeting their deadlines on one machine — the case current systems fail.
  *exit:* audio round trip holds under sustained inference, and the same workload on a tuned Linux is shown missing it.
- [ ] **E6-P02** `S` The sharing claim: ten components, one model, one physical copy.
  *exit:* resident bytes measured. The cheapest result in the plan to demonstrate, and among the most legible.
- [ ] **E6-P03** `M` Joules per token against a tuned baseline.
  *exit:* external meter; model and prompt set published with the number.
- [ ] **E6-P04** `M` The interference matrix: what each tenant class does to each other tenant class.
  *exit:* published as a matrix rather than a headline, including the cells where F does badly.

### Release

- [ ] **E6-R01** `S` **Release 0.7.**
  *exit:* the concurrency claim reproduces on the named machine from the package.

> ### Gate G6
> Concurrent inference and audio, both inside their deadlines, with weights
> shared from the store — measured, against a baseline configured to win.

---

## E7 — The result

*Not a phase. The point of all of it: the claims, defended.*

**Effort:** 0.5–1 person-year · **Risk:** the honest kind · **Ends at:** gate G7, release 1.0

- [ ] **E7-01** `L` Defend each novelty item with a number, a named baseline, a published workload and a reproduction command — or withdraw it in writing.
  *exit:* every item in the novelty list is either a defended claim in `claims/` or a written withdrawal explaining what was learned. Nothing is left asserted.
- [ ] **E7-02** `M` The independent replication guide: what hardware, what steps, what to expect, what varies.
  *exit:* a third party follows it end to end without contacting us, and reports where it was ambiguous.
- [ ] **E7-03** `M` Invite an outside party to reproduce the headline claims.
  *exit:* their numbers are published beside ours, including the disagreements, unedited.
- [ ] **E7-04** `M` The write-up, with the "where we lost" section given the same care as the results.
  *exit:* it states what F is permanently behind on — assurance, drivers, completeness, field evidence — as plainly as the comparison document already does.
- [ ] **E7-05** `S` Freeze the claims registry at 1.0; tag every claim with the release it was last defended in.
  *exit:* no claim in the registry is older than the release that publishes it.
- [ ] **E7-06** `S` **Release 1.0.**
  *exit:* gate G7.

> ### Gate G7
> A competent skeptic tries to refute a published claim, using only what is in
> the release package, and fails.

---

## Always on

Not epoch work, and never finished. These decay if nobody touches them, and each
one is cheap to maintain and expensive to restore. They carry a cadence rather
than an exit, because they do not close.

- [ ] **A-01** `docs/TESTING-STATUS.md` reflects reality. It is the page that stops the plan from being mistaken for the state of the tree.
  *cadence:* every release, and every time a layer changes status.
- [ ] **A-02** Every reversal gets an RFC. Superseded RFCs are marked, never edited away.
  *cadence:* every decision that changes something already written down.
- [ ] **A-03** Re-check every immature dependency's fallback. The table exists; it goes stale silently.
  *cadence:* once per epoch.
- [ ] **A-04** Re-tune every claim's baseline, or the tuned-Linux comparison quietly becomes a stock-Linux comparison.
  *cadence:* once per epoch, and whenever the baseline's own version moves.
- [ ] **A-05** Report the unsafe-code percentage against the under-5% target. RFC 0001 reverses at 10%.
  *cadence:* every release.
- [ ] **A-06** Nightly sweeps and weekly checking stay green or stay loud. A muted job is a deleted job with extra steps.
  *cadence:* daily and weekly; muting one is a reviewable diff.
- [ ] **A-07** No silent scope cuts. Anything dropped moves to `[~]` with its reason, in place.
  *cadence:* whenever something is dropped, which is the moment it is easiest not to do.
- [ ] **A-08** Re-read the risk register: the fallbacks, the reversal triggers, and the two claims that would falsify a bet if they came back wrong.
  *cadence:* once per epoch, out loud, with the whole team.
- [ ] **A-09** `CLAUDE.md` stays one page and stays true. A mistake made twice earns a line under *Common mistakes*; a mistake made a third time means the instruction is not working and becomes a task in `evals/`.
  *cadence:* every repeat finding, which is the moment it is easiest to fix once and forget.
- [ ] **A-10** The eval suite runs, and the floor in `evals/suite.toml` is not lowered to make a change pass. Lowering it is a decision about what this repository accepts.
  *cadence:* every diff to `CLAUDE.md`, `.claude/` or `REVIEW.md`, and weekly on the schedule.
- [ ] **A-11** Every incident produces a post-mortem in `docs/postmortem/` and a change to something that *runs* — a lint, a hook, a test, an eval, a band. An incident that produced no eval will happen again in a form nobody recognises.
  *cadence:* every incident, including the near misses where nothing broke.
- [ ] **A-12** The linkage holds both ways: an accepted `intent/` names its task IDs, and the task names the intent. Either half alone rots silently, and the pair is the only route from a ranked task back to the argument for it.
  *cadence:* whenever an intent is accepted or a task is added.
