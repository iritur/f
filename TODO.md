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
- [x] **E0-D07** `M` Adopt the twelve rules in `CONTRIBUTING.md`, and mechanise the three that can be.
  *exit:* met. All twelve are in `CONTRIBUTING.md` with the enforcement named per rule, and three are executable: `lint-units` (R03), `lint-callbacks` (R05), `lint-claim-owners` (R09), all three in `lint`, in `verify` and in CI, each with a fixture in `xtask` that makes it fail.
  **R03 is a marker, not a vocabulary.** A check for unit *words* — does this doc mention nanoseconds, bytes, indices — passes on a sentence that happens to contain one and fails on a sentence saying the same thing differently. It trains people to include a keyword, which is worse than no lint because it looks like coverage. `Unit:` has to be written on purpose, and it makes the dimensionless case explicit too: `Unit: none` is a statement that a field is an identifier rather than a quantity, which is exactly the claim R03 exists to force out loud. Twenty-nine public fields in `abi/` now say one.
  **It caught `deadline` on its first run** — the field R03 was written about. Its doc comment discussed the unit, the epoch and the zero at length, and stated none of them; it read as a field whose units were settled because the paragraph was about units. That is the whole failure mode in one place.
  The epoch and the zero are not separately checked, and that is a stated limit: they are meaningful for some units and not others, and a lint demanding all three of an index would teach people to write three words to get past it.
  **The fixtures are strings, not files.** A broken file on disk is read by every other lint here too, so a fixture violating R03 would also have to satisfy the SPDX, unsafe and determinism checks — or be excluded from all of them, at which point it is excluded from the rule it exists to break. `lint-mutations` had to design around the same trap. The gap this leaves is real and stated: the fixtures cover the rule, not the file walk that feeds it.
  A ninth test asserts the tree currently *passes* all three. A lint that has never failed and a lint that has never passed are equally uninformative.
  **Nine rules are review, and the table says so.** R01 applies to this table: a rule with "review" beside it is a rule somebody has to remember, which is a plan, and plans are what the systems being criticised also had. A rule listed as mechanised that is not is worse than one honestly listed as review, because it is a check somebody believes is happening.
- [x] **E0-D08** `M` Write `RELEASING.md` — what a release is, what it contains, how a stranger reproduces it.
  *exit:* met. `RELEASING.md` is merged, and `cargo xtask release --dry-run` prints the manifest: eight contents in the order `docs/the-long-plan.html` section 08 lists them, six present today, each absent one naming the task that owes it. `cargo xtask release` without the flag refuses and says E0-R01 owns building the package.
  The command lists what would stop a release and deliberately **does not check** any of it. Running `verify` from inside a manifest print would make it cost a full boot, and this command's value is being cheap enough to run while thinking about whether a release is close.
  **It found a gap in the plan on its first run**, which is the argument for the command existing at all. *The baseline configuration* is one of the eight things the release contract requires, and no task in this file produced it — `claims/0001` names `linux-6.x-tuned` in prose, which is precisely the decay the contract warns about: prose ages into a stock comparison without anybody deciding it should. Now **E1-D06**, rather than a silent scope cut (A-07).
  The dry run hashes with `sha256sum` when the machine has it and prints the manifest without the hash column when it does not. A build tool growing a hash implementation, or refusing to print a manifest because coreutils is absent, are both worse than a missing column — and the package's own content addressing is E0-R01's job, not this command's.
- [x] **E0-D09** `S` Record the target-JSON decision: use `targets/x86_64-f.json` or delete it.
  Deleted. Nothing built it — its only two references in the whole tree were `BOOTSTRAP.md`'s gap table and this line — and everything it said that the built-in `x86_64-unknown-none` does not is two codegen flags already set in `.cargo/config.toml`, beside the paragraph explaining why the image does not link without them. The usual argument for a custom target does not apply here either: the build already passes `-Zbuild-std`, so the JSON was never buying the thing a JSON usually buys, and an unbuilt second copy of a target definition cannot fail loudly when the spec schema moves under it.
  *exit:* met. The file is gone; the reason and the reversal condition — a data layout, a linker flavour or an atomic width that the built-in plus rustflags cannot express — are a doc comment on `KERNEL_TARGET` in `xtask/src/main.rs`. The `BOOTSTRAP.md` gap row is struck through and marked done, which is where the question was actually being asked from.
- [>] **E0-D10** `M` Name the measurement machine — `runner-class-A` as a specification, not a hope.
  Found by reading this file's own graph: `E0-P05`, `E0-P06`, the red half of `E0-P01` and release 0.1 all wait on `runner-class-A`, and no task produces one. The class is named in `MEASUREMENT_ENVIRONMENTS` and in both pending claims' `[hardware]` sections, which is the same decay `E1-D06` exists to prevent — a machine that is only a name in prose cannot be obtained, and every epoch adds claims to the same queue (`E1-P10` adds four). `E5-D01` does this job for the phase-05 workstation; this is the same task for the claims runner, three releases earlier.
  *exit:* **the specification half is met; the machine half is a purchase order and not a commit.** `claims/runner-class-A.md` is the file: required capabilities with one worked example rather than a part number that ages out, the firmware settings that cannot be undone from an operating system, a kernel command line, a recipe per reservation component, and a seven-item checklist a stranger runs *before* setting the variable. Marked `[>]` and not `[x]`, because "a machine exists" is not something a file can be.
  Two things the writing settled. **All four components by partition is what makes the class class-A.** RFC 0007 permits partitioning by exclusion where the hardware cannot partition, and that stays a legitimate reservation — it is just not this one. A machine without RDT's MBA and CAT is a different class and needs a different name in `MEASUREMENT_ENVIRONMENTS` before it records anything, because reusing this one is exactly how a class decays into whatever hardware was to hand. That single requirement is also what rules out every desktop part: Intel dropped CAT from client silicon after Skylake-X.
  And **the environment variable is an assertion, not a measurement.** No code in this tree can tell a class-A machine from a laptop with `F_ENVIRONMENT` set by hand. What the tree does is fail closed on everything else and require RFC 0007's by-partition/by-exclusion record to travel with the number; what the file does is make the lie one a reviewer can catch by reading one page. Stating that plainly is the point of the file, not a caveat on it.
  Found while writing: `MEASUREMENT_ENVIRONMENTS` cited `claims/README.md` for what the class was, and `claims/README.md` did not mention runners at all. Both now point at the specification.
  *needs:* E0-D04 (RFC 0007 defines what the machine must be able to reserve)

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
- [>] **E0-B12** `L` The first ring: layout, cursor protocol, suppression, two opcodes — `NOP` and `WRITE_SERIAL`. **(M5)**
  The cursor protocol and the suppression have existed since M0; what was missing was everything that makes them a *channel*. Four things arrived together, and each one was load-bearing for the next.
  **The layout is now arithmetic rather than a table in a document.** `f_abi::layout::Layout` computes where every region of a mapping begins from `ring_size` alone, writes a `ChannelHeader` describing it, and — the direction that matters — *adopts* a header a peer wrote only if the offsets in it are the ones this build would have computed. Neither side trusts the other's arithmetic and both sides state it, so two peers that disagree are caught at setup rather than at the first read, where a cursor gets interpreted as an entry. The arena's length is deliberately not in the header: it runs to the end of the mapping, and how long the mapping is, is known to whoever mapped it and not to whoever wrote its first cache line.
  **Publication is one store for a batch, and it was thirty-two.** `Producer::batch` stages entries and makes them visible with a single `Release`; `submit` remains the one-entry case. This is not an optimisation added on spec — `claims/0001-ring-submit-latency.toml` names the symptom in its diagnosis section, under `flat_batch`: *the batch is not being amortised; you are publishing per entry*. That is precisely what the workload did, so the workload could not have detected it. `batch(&mut self)` takes an exclusive borrow so that a `submit` interleaved with an open batch is a compile error rather than a paragraph, and the kernel self-test found the value of that immediately: the borrow checker refused the weaker way of asking whether anything had leaked out early, and forced the question to be put to the consumer instead, which is the side whose answer means something.
  **The index ring is implemented, and it earns its place by being distrusted.** Today a producer allocates entries in cursor order, so the indirection is the identity mapping and is fair to challenge. It stays for three reasons in increasing order of weight: it is the wire layout, so leaving it out would leave a hole a peer built from the specification writes into; it is what will let an entry-array *pool* publish out of order without an ABI change; and it is one more untrusted integer. A slot number read out of shared memory is exactly as untrusted as a cursor, and `Consumer::pop` bounds-checks it on every entry — a check that is not theatre for an identity mapping, because it is the check that stays correct when the mapping stops being one.
  **Two opcodes, and the second one is the interesting one.** `NOP` is the protocol with the work removed, which is what makes it the thing worth measuring. `WRITE_SERIAL` reads a caller-chosen range of the inline arena — shared memory a peer may be scribbling on *while* it is read — so every byte is copied out volatile into the service's own memory before anything looks at it, and the range is bounds-checked against a length held privately. Section 06's rule is *never dereference anything from shared memory; indices only, bounds-checked against sizes held in your own private memory*, and the arena is the one region with no protocol governing when a peer may write it.
  **The published layout gave the completion ring no cursors.** Section 02 lists a completion ring of `32*M` bytes and stops there — no head, no tail, and therefore no way to distinguish an empty completion ring from a full one. It went unnoticed for two milestones because the submission half is the half the table gets right, and nothing had built the other half. Two cache lines were added and the index ring moved from `0x00C0` to `0x0140`. **RFC 0018** is the argument, including why the two lines went there rather than after the completion ring where they would have left the published offsets alone: preserving a stale offset by breaking the principle the offset came from is the wrong trade, and no peer has ever been built against the table as published.
  *exit:* **half met, and the other half belongs to `E0-P05`.** *One million NOPs in batches of 32* runs — `cargo xtask bench ring_submit`, through the batch path now rather than around it. *Under 50 ns per operation, recorded as a gating claim* cannot be established from here and the machinery says so rather than guessing: `F_ENVIRONMENT=container` refuses the measurement, because QEMU under TCG emulates a timer against a host clock it does not control and the container shares its cores with whatever else is running. The distribution is still drawn, and its shape moved — the band `E0-P04` recorded as carrying the p99 is now nearly empty — but a number from a refusing environment is not a number, and writing one here would be the exact failure the refusal exists to prevent. Claim 0001 stays `pending` until `E0-P05` runs it on `runner-class-A`.
  The kernel proves it on every boot. One frame, laid out by `f_abi::layout`, header written into the region and read back *out of the bytes* before being adopted — so the arithmetic is checked against memory rather than against itself. A batch of four publishes with one store; the service answers three and refuses the fourth for an unknown opcode; `"the ring is open"` reaches the boot log out of the channel's arena, on the line between the quotes, because an opcode put it there. Then a slot number is forged into the index ring and the channel must be reported corrupt.
  That last phase is worth recording, because the first version of it passed while checking nothing. It forged slot **zero**, and the consumer reads the position *its own cursor* names — four, after four entries were drained. A boot that reports a caught forgery it never looked for is `E0-B16`'s lesson in a new place: a check that cannot fail is worse than no check, and only running it the wrong way round finds out.
  Not done here, and deliberately: the channel is not yet *between two components*. Both ends are the kernel and the region is a frame rather than a shared mapping, which is `E0-B13`. A task that invented the mapping as well would have tested the layout and the mapping against each other and neither against the specification.
  **What `[>]` costs, said out loud.** `cargo xtask todo` reads this marker, so four tasks — `E0-B13`, `E0-B14`, `E0-B15`, `E0-P05` — now report as waiting on a measurement none of them needs. Three of them need the *code*, and the code is here and green. Only `E0-P05` needs the number. The exit line above was written to include `E0-P05`'s exit word for word, which is the actual defect: one criterion belonging to two tasks means one of them is always lying about its state. Fixing that is a change to what an exit *is* and belongs to whoever next argues about the exit vocabulary, not to a build task quietly rewriting its own success condition. Until then, read the graph's four blocked entries as blocked on `runner-class-A` — the machine `E0-D10` now owns — and not on anything in this tree.
  *needs:* E0-B11
- [x] **E0-B13** `M` Bind `Producer`/`Consumer` to a mapped shared region with a validated, negotiated `ChannelHeader`, replacing the borrowed-memory placeholder.
  `f_ring::Mapping` is the binding: a raw base and a length in, four checks in a fixed order, and either a channel or a structured refusal out. The order is the design. The address first, because alignment and room for sixty-four bytes are what make reading the header defined at all and are the one bound that cannot come from the header. Then the header, **copied out** — validating it in place would check one header and lay the rings out from another, which is a bounds check that bounds nothing, and this file records that failure twice already. Then negotiation, per RFC 0011. Then the layout, which refuses any offset that is not the one this build computes and takes the arena's extent from the mapping rather than from the peer.
  The placeholder that went away is worth naming, because it was hiding something. Every test built a `Channel` out of a struct with a cursor field and an entry array — a channel the borrow checker had already proved well-formed, where the offsets were Rust's problem and the wire format never got consulted. Its `sound_header()` claimed `sqe_offset: 64`, an offset no build has ever placed the entry array at, and nothing noticed for the length of a milestone. A header nothing binds is a header nothing checks.
  *exit:* met. `ring/tests/headers.rs` drives fifteen hostile headers into a real 4 KiB region — a zeroed page, a foreign magic, a magic one bit out, a ring with no slots, a ring size that is not a power of two, one larger than the index will reach, one self-consistent and too large for its mapping, an inverted version window, a dirty reserved word, an entry array over its own header, two off-by-a-line offsets, a peer from the future, one from before there was a version, and one requiring what we do not offer — and each is refused with the domain and code RFC 0010 names. Two more that cannot come from a header, an unaligned base and a region too short to hold one. Every case then rebinds a sound header over the same bytes and requires it to work: a teardown that poisoned the region would be a denial of service any peer could trigger with one word. The seeded sweep's third site is now `chan.bind` rather than `chan.negotiate` and goes through the same path.
  The kernel's channel is two `Mapping`s over one frame — one end describes it, the other adopts it, and they are required to agree, which is a check a single-ended round trip cannot make. The boot also breaks the magic in the region and requires the refusal, on the target, in a build with no unwinding and no allocator where a panic is not an exception but the end of the boot.
  Not done, and it is the half of the title this exit does not reach: the frame is not mapped into a *process*. `user/init` inherits `unsafe_code = "forbid"`, so a component cannot hold a raw base at all — the safe façade that would let it is the powerbox grant, and that is `E1-D01`'s to shape rather than a build task's to invent in passing.
  *needs:* E0-B12, E0-D03
- [x] **E0-B14** `M` State tree v0: the kernel publishes its counters into a read-only mapping.
  `f_abi::state` is the wire format against RFC 0013, and each of that document's six properties is something the format *cannot say*. There is no encode step, so there is no sampling interval and no second copy — a node names a live word and publishing is the store the subsystem was already making. There is no generation counter and no seqlock, because a snapshot is atomic per node and the format does not pretend two nodes are from one instant. The hash is over the data block in node-id order and never consults the schema, so a reader too old to name half the nodes computes the same number as one that can name them all — which is the property that makes two readings comparable across versions at all.
  **One node in every tree, forever, is a kind nothing names.** RFC 0013's single deliberate exception to R04 is that a reader skips and counts an unknown kind rather than refusing the tree, and a skip path that is never taken is a skip path nobody has tested. `id 63` costs eight bytes and takes it on every boot.
  `validate` refuses two things worth naming: two nodes sharing a word — two subsystems publishing into the same place, which reads as one of them being broken — and a **gap**, a word the snapshot hashes that no node describes, so the hash would move for a reason no reader could name.
  **The component reads it without `unsafe`, and that needed an argument rather than a wrapper.** `f_abi::state::Reader` is a *safe* function over an address the frame mapped, making the same case `f_abi::door::call` already makes: a component inherits `unsafe_code = "forbid"` and that property is enforced rather than asserted, so the instruction that reads a mapping lives on the frame's side of the boundary, in a crate reviewed as part of it. The obligation is discharged against a contract the frame keeps, and a component that invents an address gets a page fault — the defined machine outcome `cargo xtask user` is seven boots of. It is not sound by Rust's rules and the doc comment says so; what makes it acceptable is that the failure is the one the hardware is there to produce.
  **The linker enforced panic-freedom, which was not the plan and is better than the plan.** `user/init` links as one library with no `core` beside it, so anything the reader calls that is not inlined into the component is an undefined symbol rather than a warning. Three separate constructs failed: a range index (`slice_index_fail`), `Iterator::take` (`IndexRange::len`), and — the one nobody would guess — iterating the `[u8; 8]` from `to_le_bytes()`. The hash is now shifts, in one `fold` shared by both readers, because two readers of the same bytes disagreeing about the hash would make the whole mechanism worthless.
  *exit:* met. The kernel publishes twelve nodes into a frame, validates the header and schema by reading them back out of memory rather than reusing what it wrote, and its self-test asserts both halves: two snapshots with nothing in between agree, **and a snapshot after a deliberately bumped word disagrees** — a hash over bytes nothing writes agrees with itself forever, which is the defect `cargo xtask trace` exists to catch one layer down. `init` maps the tree at ring 3 through a granted capability, hashes it twice and exits zero; the frame renders every node to the boot log, including the one whose kind it will not interpret.
  **`cargo xtask cap state` is the ninth escape and the strongest evidence here.** The read-only mapping is a negative, and a component cannot demonstrate a negative about itself — a store would end it rather than return an answer. The probe maps the tree, *reads it successfully*, and then writes; the read is what makes the fault about the permission rather than about the address, which is `E0-B12`'s forged-slot-zero lesson in the one place it would recur.
  Nothing time-derived is published, deliberately: the boot log is what `cargo xtask trace` hashes, and a tick count in it would make two runs of one commit disagree for a reason that has nothing to do with the kernel. *Reversal:* a boot log that is no longer the reproduction artefact.
  *needs:* E0-B12, E0-D05, and E0-B13 — a real dependency this line did not name, in the shape `E0-P01` and `E0-P02` already use. The reader's copy-then-check discipline is `f_ring::Mapping`'s, arrived at for the same reason and worth being the same shape.
- [>] **E0-B15** `M` Doorbell: kernel IPI path first; user-interrupt path behind a negotiated feature bit.
  **Found before the task started, and fixed ahead of it: the suppression protocol had a lost wakeup.** The two ends run Dekker's algorithm at exactly one place — the producer stores `head` then loads `flags`, the consumer stores `flags` then loads `head` — and a store followed by a load of a *different* location is the one reordering total store order permits. `Release` and `Acquire` do not forbid it; they are one-way barriers and this needs a two-way one. Both ends could look and see nothing: the producer reads the flag before its own publish is visible and rings nothing, the consumer reads the ring before that publish and sleeps. The entry is stranded. It is not a data race — every value read was legitimately written, and no sanitiser finds it — it is a hang.
  The second check the design document prescribes closes half of it: the case where the consumer's *flag write* is late, and not the case where the consumer's *ring read* is early. That sentence is why RFC 0020 exists, and `docs/design/ring-scene-boot.html` section 03 published the pseudo-code without the fence, so the correction is in the document as well as in the code.
  It is the first defect in `ring/tests/litmus.rs` that does **not** need the arm runner, and CI then showed the stronger version of that: it needs the **x86** runner and is invisible on the arm one. `Release`/`Acquire` become `stlr`/`ldar` on AArch64, which are RCsc — a Store-Release followed by a Load-Acquire is ordered by the architecture, so there the fence is redundant and its absence changes nothing. On x86-64 both are a plain `mov` and nothing stops the load being satisfied out of the store buffer: **58 971 lost wakeups in 500 000 rounds, first at round 69**, eight in a thousand rather than a rare interleaving. Which machine can see a defect is a property of the reordering it depends on and not of how serious it is — a gate on the intuitive runner would have been green for the wrong reason.
  That does not make the fence an x86 workaround. What it buys is correctness under the *language's* model, where the reordering is permitted whatever a particular backend emits today. The harness had to change to see it at all: a `std::sync::Barrier` ends in a futex wakeup measured in microseconds, and the window is one store buffer deep, so a parked-thread barrier lines the two threads up thousands of times too loosely and reports a clean run on a broken build. A spin barrier finds it in sixty-nine rounds and runs two hundred times faster.
  Fixed now rather than with the rest of this task because nothing in the tree sleeps yet — both ends of the frame's channel are the kernel and it drains synchronously — so this is a latent hang that becomes a hang on the day this task gives the doorbell somewhere to ring. It would also have flattered the number below: a producer that wrongly believes the consumer is awake rings *less*, so doorbells-per-operation measured on the defective build would have looked better than the truth.
  **The typed doorbell is what section 03 asked for by name**: one function with three implementations — `Polling`, `KernelIpi`, `UserInterrupt` — selected at channel creation from what was agreed *and* what the hardware reports, which are two questions and not one. A feature bit says what the protocol permits; it says nothing about the silicon, and conflating them is how a channel gets negotiated into an instruction that faults. The requirement half needed no new code: a peer that cannot proceed without user interrupts never reaches path selection, because `negotiate` refused the channel at setup.
  The counts live with the sender and not in the header, and the reason is worth keeping: the obvious home is the four reserved words in `ChannelHeader`, which is memory the peer writes. Evidence of delivery a peer can forge is not evidence — and it would have cost an ABI version for a field that never needed to cross the boundary.
  **The kernel path is a real vector and a real delivery.** `DOORBELL_VECTOR` is the third in this table that exists because something is wanted, and it is the one that carries nothing: the shootdown next door says *which page* and needs two shared words to say it, and a doorbell says only *stop halting*. So it adds no fifth address two cores reach, and `CLAUDE.md`'s count stands — not by exemption but because the word does not exist. The boot sends it, opens an interrupt window with every deliverable vector gated and both the PICs and the APIC timer masked, and requires the count to advance: `doorbell KernelIpi, 1 delivered, 500 per 1000 operations, a draining consumer was not rung`.
  One suppression test, taking the path as an argument, because two tests written from one description drift the first time somebody edits one of them. It also asserts that a **batch is one operation and at most one doorbell** — the distinction the published number rests on, since charging per entry would make the figure fall as batch size rose and report that as suppression working when what was working was batching.
  *exit:* **two-thirds met, and the third is a number this machine may not take.** The kernel path is built, delivered and demonstrated at boot; the user-interrupt path is built to the point of *refusing to construct* and no further, because QEMU's TCG backend implements no part of Intel UINTR and no `-cpu` model advertises the bit — `E1-B09` owns the hardware, and `docs/TESTING-STATUS.md` now says the instruction has never run rather than letting a green suppression test imply three paths were exercised. "Both paths pass the same suppression test" is met in the sense the test can be run and not in the sense the second path has ever rung.
  The number is **deliberately not registered as a claim**. A count over the two operations a boot self-test performs is not *doorbells per operation under load*: that needs a workload and two cores actually contending, and under TCG with `-smp 2` it would measure the emulator's scheduling of two vCPUs against a host clock it does not control. Registering it would put a number in the registry that does not answer the question the registry asked — so the boot reports it, and the claim waits for `E0-D10`'s machine alongside 0001 and 0002.
  Cross-core delivery is also not proven, and the reason is the same one that kept the word count at four: observing another core's counter means reading another core's slot. It belongs with the component that will actually sleep on a doorbell.
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

- [>] **E0-P01** `M` CI pull-request gate under ten minutes: lint, test, run, claims.
  Half the budget was being spent twice: `ci` triggered on both `push` and
  `pull_request`, so every commit on a branch with a pull request open built the
  whole matrix — both AArch64 runners and the QEMU boot — against an identical
  tree. Fixed; the gate is now the pull request, and `main` is checked again
  after a merge because a merge commit is a tree nobody tested. Still open: the
  claims job, the ten-minute measurement, and the red half of the exit.
  The claims job exists now, and it is the fourth thing the gate was always supposed to run beside lint, test and run. It asserts everything about the registry except a number: every claim names an owner that exists (R09), every document citation matches the value the claim holds, the committed snapshot is not stale — a stale one is a commit asserting numbers it does not hold — and the release manifest still resolves.
  **The snapshot half was first written as a line of YAML, and it failed on a file that was byte-identical.** `git diff --quiet -- claims/snapshot.json` inside the container: git refuses a working tree owned by another uid, and `git diff` is one of the few commands that tolerates running outside a repository at all, so it renders that refusal as *warning: Not a git repository* and exits non-zero. The gate reported a stale snapshot; the snapshot was current. A check that names the wrong file is worse than no check, because the reader spends their afternoon on the file it named.
  Two things came out of it. The comparison moved into `cargo xtask lint-snapshot`, where it needs no repository and — the part that matters more — where a developer can run it before pushing; it is in `lint_all`, so `verify` covers it. And the image now marks the tree safe, because the same refusal was one step away from doing real damage: `xtask release --dry-run` read its version and commit with `unwrap_or("unknown")`, so it would have printed a clean-looking release manifest for a tree it could not identify, and passed. Both fields are fatal now. A release that cannot name its own tree is not a degraded release, it is a confident statement about nothing.
  *exit:* **half met, and the other half is blocked on a claim that gates.** The green half is written and runs; the red half — "red on a pull request that regresses a gating claim" — cannot be built, because no claim gates. 0001 and 0002 are `pending` on a measurement environment that does not exist here, and 0003 is `tracked` on purpose. That is `E0-P05` and `E0-P06`, and it is a real dependency this task's `needs:` did not name.
  **The ten-minute measurement is made: 2 m 56 s.** Run `33461047717` at `f51c45f`, twenty checks, all green, on a pull request. The budget has more than three times the headroom the number needed, and the reason is that the gate is wide rather than deep — thirteen jobs that mostly wait on the same image and then run at once, so the wall clock is the longest job (`kernel`, 1 m 27 s) and not the sum. That is worth writing down because it is what makes the budget survivable: adding a job costs nothing until it becomes the longest one.
  This commit adds two, `package` and `address`, and each builds the kernel — so the expectation is a wall clock still governed by `kernel` and a number that moves by seconds. The next run is what says, and if it does not, this line is the one that was wrong.
  Marked `[>]` alone now, and for the one reason left rather than for company: `E0-P02`, `E0-P07` and `E0-P14` all cleared on that run. What holds this open is the red half, which needs a claim that gates.
  *needs:* E0-B01, E0-P05, E0-P14
- [x] **E0-P02** `M` **The reproduction check.** Two runs of the same `(seed, commit)` on two different runners produce a byte-identical execution trace hash.
  `cargo xtask reproduce` boots the commit twice, hashes each serial log, and requires the two to agree — then builds the kernel with **one unseeded read of the timestamp counter** on the boot path and requires two runs to *disagree*. Both halves, for the reason `mutate` gives: a reproduction check that has only ever passed is indistinguishable from one that cannot fail, and this one is unusually easy to get wrong in that direction — a trace hashed over something that never varies agrees with itself forever.
  **The defect is the shape of the bug this whole apparatus exists for**, and it is worth looking at. It does not make the kernel fail. It boots, every assertion holds, it prints `M0 ok` and exits 33. `run`, `user`, `cap`, `panic` and `mutate` are all green on it. The only thing wrong is that two runs no longer agree — and until this command existed, nothing in the tree would ever have said so. That is why the defect is not in `MUTATIONS`: every entry there makes a boot go red, and this one makes a boot go green twice with two different answers.
  The kernel's own boot-time digest self-check is compiled out under that feature, whole. Leaving the comparison out but the computation in produced an unused-variable warning, and a defect build that does not compile cleanly is a defect build somebody disables.
  FNV-1a for the trace hash, and the choice is argued rather than defaulted: it has to be identical on two machines at one commit, which rules out `DefaultHasher` — the standard library reserves the right to change it and it is seeded per process. It does **not** have to be collision-resistant; nothing adversarial produces these traces, and the content-addressed *release* is a different problem with `sha256` as its answer at `E0-R01`.
  In `verify` as well as CI, because the failure it catches is the one nothing else in that loop can see.
  *exit:* **met.** Run `33461047717` at `f51c45f`: `execution trace (runner a)` and `execution trace (runner b)` both produced `0x8eda23049c554226`, the `reproduction` job compared the two artefacts and agreed, and the defect half ran in the same job and disagreed as required. Two machines, one commit, one hash.
  **Three environments, not two, and the third was not a CI job.** A development container on a Windows host — different kernel, different CPU, different filesystem, and Docker Desktop between them — produces `0x8eda23049c554226` as well. The two runners are two instances of one machine class and could have agreed for a reason that would not survive a third; this is the third, and it agrees. It is also the cheapest evidence in this file, because it is a command anybody reading this can run.
  **The number this line used to carry, `0x90a3830cce8ad586`, was not wrong — it was for a different tree**, and the difference is worth a sentence because a hash written down without its commit is the kind of number that gets treated as a constant and then defended. `E0-B13`, `E0-B14` and `E0-B15` each added output to the boot path, so the log this hashes grew and the hash moved. It is a property of a commit and reads like one now.
  Worth recording: **this job only became possible when every job started running in the same image.** Two runners could otherwise have had different QEMU versions, and QEMU's version is in the boot log — the check would have failed for a reason with nothing to do with the kernel, which is how a gate gets disabled. `E0-P14` was a prerequisite that this task's `needs:` did not name.
  *needs:* E0-B01, E0-P14
- [x] **E0-P03** `S` Coverage instrumentation reported per crate by `xtask coverage`, while the kernel is small enough for it to be trivial.
  `llvm-profdata` and `llvm-cov` come from the pinned toolchain's own sysroot, the way `llvm_tool` already took the linker and `objcopy`. That is the whole reason `cargo-llvm-cov` is not a prerequisite: a coverage number produced by a separately installed tool is a number whose version nobody pinned, which is the ambient dependency the container exists to remove — and it keeps the command working in the `dev` image rather than only in `full`.
  *exit:* met. Four crates and a total, printed by `cargo xtask coverage` and written to `target/coverage/summary.json`, which the CI job uploads as an artifact so a fall is answerable from two runs rather than from memory. Today: abi 85.42%, bench 58.25%, env 95.72%, ring 93.01%, total 84.24%.
  **Nothing here gates, on purpose.** A coverage threshold rewards tests written to touch lines rather than to catch anything, and this repository already has a mechanism for a number that must hold — `claims/`, with a baseline and a reproduction. So this one is published and watched. Lowering it is then a visible fact rather than a passing build.
  Two decisions inside the measurement, both of which change the number. `/tests/` is excluded, because an integration test measures its own execution and reports itself as covered — that raises the figure without covering anything, and the question being asked is how much of the *library* the tests reach. And a report row whose first path component is not a directory with a `Cargo.toml` in it is skipped rather than guessed at, so a change to the workspace layout shows up as a crate going missing from the table instead of as a plausible wrong total.
  Found while building: the second cargo invocation — `--no-run --message-format=json`, which asks where the test binaries landed — has to carry the same `RUSTFLAGS` as the first. Without it that is a different build with a different fingerprint, so cargo rebuilds everything uninstrumented and reports *those* binaries. llvm-cov then measures objects with no counters against a profile that has them, and reports zero coverage rather than an error.
- [x] **E0-P04** `M` Bench harness records full distributions with p50/p99/p99.9, plus instructions and joules per operation, marked absent until the counters exist.
  The recording half was already right — a log-linear histogram, percentiles that lean pessimistic, and `Metric::Unavailable` carrying why. What was missing is that none of it was *reported*: `Sample::report` printed six numbers on one line, which is a summary of a distribution that the reader never saw. `claims/README.md` rule 3 was stated and not kept.
  *exit:* met. `cargo xtask bench ring_submit` draws the distribution before the percentile line, one row per occupied octave, and writes the full bucket list to `claims/ring-submit-latency.local.jsonl` — a header object carrying the run summary and every metric's availability, then one object per non-empty bucket.
  **The drawing immediately earned itself.** The percentile line for this workload reads `p50=6 p99=43 p99.9=1855`, which looks like one distribution with a tail. The picture shows observations at every octave from 2 ns to 262 144 ns, including one at 151 µs — a host scheduler preemption in the middle of a batch, five orders of magnitude from the mode. That is a second mode rather than a tail, and the difference between "sometimes slow" and "two different things are happening" is the difference between two diagnoses with nothing in common. `claims/0002-timer-jitter.toml` already names `best_case_bimodal` in its diagnosis section; nothing in the output could previously have shown it.
  Interior empty octaves are drawn rather than elided for the same reason, and a bar saturates at one column rather than rounding to zero: the tail is made of small counts, and a bar that rounds away is a tail present in the data and absent from the picture.
  Found while building: an octave's range cannot be reconstructed from the sub-bucket bounds. Below magnitude four a sub-bucket index is the value's low bits rather than a fraction of the octave, so `value_at` and `upper_at` both fold there — and the first drawing printed `4 .. 15` and `8 .. 15` as two separate rows. Two rows claiming the same upper bound is a table that cannot be read, and it is the one kind of error in a drawing that a reader has no way to detect. The octave is defined by its magnitude and needs no reconstruction; a test now asserts the rows tile with no gap and no overlap.
  An absent metric serialises as `null` and not as zero. Zero is a measurement, and an absent metric that reads as zero is a claim nobody made.
- [ ] **E0-P05** `S` Claim 0001, ring submit latency, moves from `pending` to measured and `gating`.
  *exit:* `cargo xtask claims` reports it green; a deliberate 20% regression fails the build.
  *needs:* E0-B12, E0-P04, E0-D10 (the machine the number is taken on), E0-P18 (F running on it, which this line assumed and did not name)
- [ ] **E0-P06** `M` Claim 0002, timer jitter: p99 under 5 µs for a 1 kHz timer over 60 seconds, gating from M2 onward.
  *exit:* recorded with the reservation conditions from RFC 0007 named in the claim.
  *needs:* E0-B07, E0-D04, E0-D10 (the machine the number is taken on), E0-P18 (as above — the reservation is carved by F's own frame, so F has to be running)
- [ ] **E0-P18** `M` Boot F on a machine that is not an emulator, and keep the log.
  Found by reading `E0-P05` and `E0-P06` against `claims/runner-class-A.md`. Both need `runner-class-A`, and both need F running **on** it rather than beside it: the reservation is carved "by F's own frame" for F's runs, and the firmware section turns Secure Boot off "because the kernel under test is not signed". Neither sentence means anything unless F boots on the metal — so a hardware boot was a prerequisite of two tasks, assumed by both and owned by none, which is the promised-layer-with-no-owner decay that made `E0-P16` exist and is the same decay one epoch earlier.
  **This is not the E5 machine and does not belong there.** `E5 — Real hardware` is the workstation: the deployment target, the graphics shim, benchmarks on real devices. This is the claims runner, needed at E0 because `E0-R04` cannot ship without two numbers that cannot be taken anywhere else. Putting it at E5 would mean releasing 0.1 having never once run outside an emulator.
  **The gap it closes is total rather than partial.** Every assertion the boot log makes is currently an assertion about QEMU: the APIC enumeration, the memory map, the UART, the AP startup, `M0 ok`. Not one of them has been observed against real firmware even once, and `docs/TESTING-STATUS.md` is the page that has to stop implying otherwise.
  **A found defect, recorded before the machine exists.** `claims/0002-timer-jitter.toml` registers its reproduction as `cargo xtask claim timer-jitter`, and that path ends in `boot()`, which runs `qemu-system-x86_64`. The claim needs a reservation carved by F's own frame on class-A hardware; QEMU on that machine is not that. So either the harness learns to drive a hardware boot and collect its log, or the claim stops naming a command that cannot produce it. Same shape as the defect `E0-R02` already found here — a registry whose one command is not the command.
  The procedure is `docs/booting-on-hardware.md`, written **before** the first boot rather than after it, because a boot nobody can repeat is an anecdote. It carries the two things most likely to cost an afternoon: there is no video output at all, and the console is 38400 baud rather than the 115200 everybody reaches for — so on a machine with no serial the screen is black and a clean boot is indistinguishable from a triple fault.
  *exit:* `M0 ok` reaches a serial console from a machine that is not an emulator, and the log is kept in `docs/postmortem/` beside the QEMU one with every difference accounted for — memory map, core count, `cores` versus `present`, and the trace hash, which **must** differ and is not a determinism failure when it does. The procedure that produced it is in `docs/` and names the loader, the module, the serial parameters and the firmware settings, so that a second person reproduces the boot rather than the debugging.
  *needs:* E0-B10 (the boot module path this loads)
  **Deliberately not blocked on `E0-D10`**, and the graph should say so. `E0-P05` and `E0-P06` need class-A silicon because they need RDT partitioning; *this* needs an x86-64 machine with a serial port and Secure Boot off, which is a machine somebody may already own. Listing `E0-D10` here would make the one remaining E0 task that can be started this week look like it were waiting on a purchase order. It is the cheapest unblocked item in the epoch and the only one whose result nobody can predict.
- [x] **E0-P07** `M` Litmus tests for the cursor protocol run in CI on x86-64 **and** AArch64.
  The job is now a two-entry matrix rather than an AArch64-only job, so the litmus suite runs `--release` on both architectures. The x86-64 half is not redundant: it is the **control**, and it is what makes a red arm run mean "this is about weak memory" rather than "this test is broken". Without it a failure on the arm runner has two explanations and no way to choose between them.
  `fail-fast: false` on the matrix, deliberately. The default would cancel the AArch64 job the moment x86-64 went red, throwing away the one result that distinguishes *the ring is broken* from *the ring is broken on weak memory* — different bugs, different fixes.
  Neither job carries `continue-on-error`, and the comment in `ci.yml` says why it may not: an advisory job that goes red is a job that gets ignored on the second Tuesday, and this one exists to be believed. Checked mechanically while writing it — no job in `ci.yml` is advisory.
  *exit:* **met.** Run `33461047717` at `f51c45f`: `memory-ordering litmus (x86-64, total store order)` and `memory-ordering litmus (AArch64, weak memory)` both green, eight tests each, `--release`, on a pull request.
  **What the two runners turned out to be worth is not what this entry expected**, and that is the result rather than a footnote to it. The control was written to make a red arm run mean *weak memory* rather than *broken test*. It has instead been the thing that located a defect on the wrong runner: `mutate-no-doorbell-fence` was gated on both, went red on AArch64 because the suite *passed* there, and the reason is that `stlr`/`ldar` are RCsc and order the store-load this defect needs. The gate moved to x86-64 — the only one in this file that does. A matrix is worth having because it tells you which machine can see a thing, and that is not always the machine you would guess.
  The standing gap is unchanged and worth restating rather than letting the green ticks imply otherwise: these are **stress tests, not a model check**. RustMC — `E0-P16`, now that M5 is here — is what explores what RC11 permits; two runners plus the unit tests are what exists until then.
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
- [x] **E0-P09** `M` Exercise the fault-injection hook: one seeded fault class per subsystem that exists, using the protocol-aware site labels already in `env/src/sim.rs`.
  *exit:* met. `ring/tests/faults.rs`, in `cargo xtask test` and on both CI runners: a seeded run injects at `ring.publish`, `ring.consume` and `chan.negotiate`, every injected fault comes back as a refusal a caller can act on, and the same seed produces a byte-identical trace.
  **The site labels were decorative and this found it.** `should_fail` took `site` and dropped it — the module documentation claimed protocol-aware injection, and there was no aiming, because there was nothing to aim with. Worse, every decision came from one shared sequence, so a site's answer depended on how many *other* sites had been consulted first. That makes a failing seed fragile in the way that matters least visibly: adding a fault check anywhere — an unrelated subsystem, a path the failing scenario never enters — shifts every later draw, so the seed that reproduced a bug on Monday reproduces nothing on Wednesday and nobody can tell whether the bug was fixed. **A seed is supposed to be a complete bug report; that one had an expiry date nobody could see.** Each site now draws from its own stream, keyed by the seed, the label and that site's own occurrence count, and a test asserts a site's trajectory survives unrelated traffic.
  `SimEnv::focused_on` is the other half of what `site` was always for: narrowing a sweep to the transitions under investigation. It does not change what the focused site sees, which is the property that makes narrowing safe to do while debugging rather than a different experiment.
  **One class per subsystem that exists, and the third is deliberately absent.** The ring and the channel header have host harnesses and both are exercised. The capability table does not — it lives in `kernel/`, which has no host harness at all, and `kernel/Cargo.toml` says why — so its fault classes belong to `E1-P02`. Writing them here would mean writing a second capability table to inject into, and a test of a model of the system is not a test of the system.
  `f-env` is a **dev**-dependency of `ring` and never a dependency. A fault check inside `submit` is a branch on the one path this whole design exists to keep short; the tests drive the ring from outside while a seeded `Env` decides what the world does to it.
  The site table is fixed at sixteen, because this crate compiles into a kernel with no allocator. Overflow is counted and reported rather than ignored: a fault harness that quietly stops covering part of the system still reports green, and that gap is invisible.
- [x] **E0-P10** `S` `xtask claims` publishes a machine-readable snapshot, and the design documents render their numbers from it instead of restating them.
  *exit:* met, and demonstrated rather than asserted. Changing `ns_per_op_p99.max` from 50 to 40 in `claims/0001` and touching no document: `cargo xtask lint` goes red naming the file, the citation and both values; `cargo xtask claims --render` rewrites the sentence in `docs/design/ring-scene-boot.html`; lint goes green. Reverting the claim moves the document back.
  **A marked span, not a generated page.** The documents are written by hand and should stay that way — what is wrong with a restated number is not that prose contains numbers, it is that nothing connects the two, so a claim can move or go red while the sentence arguing from it stays confidently in place. `<span data-claim="ring-submit-latency:threshold.ns_per_op_p99.max">50</span>` is the smallest thing that creates the connection: the prose is still prose, and the number in it has an owner.
  **The check is the half that matters.** Rendering on demand would let a document sit stale until somebody remembered to re-render. `lint-claims` is in `lint`, so changing a threshold without re-rendering is a red build — the same discipline the determinism and licensing lints already apply to their own policies. A citation naming a claim that does not exist, or a field that resolves to nothing, is an error rather than a blank: a document rendering an empty space where a number should be is the one outcome this mechanism exists to prevent.
  Two citations today, and the second cost the prose something worth recording. `docs/design/ring-scene-boot.html` said the M2 gate was *p99 under 5 µs*; the claim holds `ns_late_p99 = { max = 5_000 }`, and there is no unit conversion in the renderer. The sentence now reads *under 5000 ns*, which is uglier. That is the right trade and not a limitation to be fixed later: a renderer that converts units is a renderer that can disagree with the registry about what a number means, which is the failure being designed out.
  `claims/snapshot.json` is written every time `xtask claims` runs, so it cannot be older than the last time anybody looked at the registry, and it is committed — it is the answer to *what did this commit claim*, and that belongs in the history rather than in a build directory.
- [x] **E0-P11** `S` Store the full measurement history from the first measurement, so change-point detection has something to reason about at phase 02.
  **"Survives a rebase" rules out the obvious design**, and that is the whole of this task. A history every branch appends to conflicts on every rebase — both sides added a line at the end of one file — and worse, a rebase *rewrites* the commits those lines name, so the surviving history refers to objects nobody has. So branches do not write it: `cargo xtask history append` runs in the post-merge job on `main`, against a commit that is already permanent, and a feature branch can be rebased any number of times without touching a file it never had a line in. The cost, stated rather than hidden: a measurement taken on a branch is not in the history until that branch merges, which is the right way round — a number from a commit that was later rewritten is a number about a tree nobody has.
  Every record carries a schema version, because this file is meant to be read years later by a detector that does not exist yet, and the one thing such a reader cannot recover is what an old line meant.
  A refusing environment still writes a record, with an empty `claims` list and the environment named. A gap that is stated is something a trend can reason about; a gap that is simply missing is a hole a change-point detector will read straight across. Coverage is in every record regardless, because a line count is the same on any machine — which is exactly why it is the one measurement a shared CI runner can contribute.
  *exit:* met, with one part deliberately not wired and said so here rather than discovered later. The command works, CI runs it on `main` after a merge, and the result is uploaded as an artifact — it is **not** pushed back to the repository. Committing to `main` from CI is a decision about who may write to the default branch, and a workflow file should not make it on the project's behalf. Until somebody makes it, the durable copy is the artifact and the in-tree file is written by whoever runs a measurement.
  Found while building, and it is a hole in `E0-B16`'s container rather than in this task: **git did not work inside the development image at all.** `/work` is a bind mount whose ownership Docker Desktop fakes, so git's `dubious ownership` guard refused every command with the same fatal. Nothing before this had asked git anything, so the gap was invisible. The entrypoint now marks the one directory the image exists to build.
- [x] **E0-P12** `S` Panic path test: a kernel panic prints something useful and exits with a code CI can distinguish from success and from a hang.
  A panic exited `Exit::Failure`, the same code as a kernel that had decided an assertion did not hold. Those are different events — one is the report working, the other is a bug in the frame that can happen *inside* the code which would have written the report — so a panic is now `Exit::Panic`, 37. The distinction has to be in the exit code rather than in the log, because the log is the thing a panic is most likely to have interrupted.
  **Nothing bounded a boot at all**, and that is the half of this task that was really missing. `machine()` used `status()`/`output()`, which wait forever, so a kernel that stopped making progress held the runner until the CI job's own timeout killed it — presenting as a job that timed out somewhere during "build", with no log and no clue. It now spawns, polls, kills and reports `Ending::TimedOut`. Three endings, not two: a harness that models *finished* and *not finished yet* has nowhere to put a boot that will never finish.
  The budget is counted sleeps rather than a deadline off a clock, and not to route around the determinism lint — `xtask/` is exempt from it. Sleep drift makes the real budget at least the nominal one, which errs towards waiting too long; a computed deadline can expire early on a loaded machine and call a slow boot a hang. This bound separates *slow* from *never*, and only one of its two failure directions is survivable.
  *exit:* met. `cargo xtask panic` boots three fixtures and requires all three to be distinguishable: clean exits 33 and says `M0 ok`; `panic` exits 37 with `KERNEL PANIC` **and its formatted message** in the log; `hang` spins and is killed by the harness after 20 s. In `verify` and in CI. Asserting all three rather than only the panic is the same argument `mutate` makes: an assertion that a panic exits 37 proves nothing unless a clean boot does not.
  The panic fixture formats a value on purpose. A panic printing only a location proves the handler ran; one that formats a number proves the handler can still reach the formatting machinery, which is the part most likely to be what broke — and the assertion checks for the message, not just the banner.
  **It immediately sharpened something else.** `cargo xtask mutate` required its defect boot to exit 35 or 0, and the defect — a removed bounds check — panics. That boot now exits 37, and the log assertion was carrying the entire weight of distinguishing "went red for the right reason" from "went red". It still carries it, and is no longer carrying it alone.
- [x] **E0-P13** `S` Record boot time to `M0 ok` as a tracked, non-gating claim.
  *exit:* met. `claims/0003-boot-to-m0.toml`, `status = "tracked"`, reproduced by `cargo xtask claim boot-to-m0` — ten boots, each a fresh machine, one observation each, through the same harness every other claim uses.
  **The measurement had to stay out of the boot log**, and that is the whole shape of this task. The log is a fixture: two runs of a commit produce the same bytes, it is asserted, and every reproduction check rests on it. A duration in it would be different on every run. So the kernel stamps the timestamp counter as the first instruction of `kmain`, and prints the delta **only** when the command line says `boottime` — the same answer `timer=` already gives for the jitter histogram, and the second instance of the same lesson: a boot log carrying a measurement is a fixture that fails at random. Checked rather than assumed: two plain `cargo xtask run` boots are still byte-identical, and the line is absent from both.
  Measured inside the kernel because nothing outside it can see where boot begins. The stated boundary is honest and narrow: from the first instruction of `kmain` to `M0 ok`, excluding the loader and the emulator's own start-up. The arithmetic divides before scaling, for the reason E0-B08 recorded — the obvious form overflows a `u64` after ninety minutes of uptime and *wraps* — because leaving the wrong pattern in the tree next to a comment explaining why it is wrong is how it gets copied.
  No baseline, deliberately. A Linux boot and this are not the same event, so a ratio would be two definitions compared and dressed as a result. The comparison that means something is against this project's own earlier self, which is what a tracked claim plus E0-P11's history is for. The threshold is present but loose: set where a change is certainly a mistake rather than where it is interesting.
  Under emulation the number is around 1 ms and `F_ENVIRONMENT` refuses to record it, which is E0-P15 doing its job on the first claim written after it. The distribution is still drawn — a number worth looking at and a number worth publishing are different things.
- [x] **E0-P14** `S` CI runs inside the development image rather than installing tools per job.
  Every job in `ci.yml` runs in the development image, the kernel job's `apt-get install qemu-system-x86` is gone, and the dependency job runs the `full` image's `cargo-deny` at the version `docker/Dockerfile` pins instead of the version an action chose — which matters most in the one job whose whole subject is dependencies.
  Two architectures are built **natively** and assembled into one manifest, rather than one build under binfmt emulation. `docker/README.md` said the AArch64 half was one `--platform linux/arm64` away; it is not. Emulated, it means installing a Rust nightly and a QEMU inside an emulated userland — tens of minutes, and failures with nothing to do with this tree. Both jobs naming one tag is the property worth paying for: the AArch64 job exists to disagree with the x86-64 one, and two environments sharing one name is the worst possible state for a job whose job is disagreement.
  **The first attempt shipped a gate with a prerequisite the gate did not produce, and CI said so immediately.** `ci.yml` named a tag published by a separate `image.yml` on pushes to `main`; on a tree where that had never fired, all ten jobs failed at *Initialize containers* with `manifest unknown`, before a single step ran. That is the release contract's own rule — *no step exists that is not in the tree* — broken by the change that was citing it, because a workflow somebody has to remember to dispatch is institutional knowledge with a YAML file in front of it. `image.yml` is deleted, and the image is three jobs in `ci.yml` that every other job waits on.
  The tag is derived from the **files that define the environment** rather than from the commit: `env-<hash of Dockerfile, entrypoint, toolchain pin>`. A commit that does not touch the environment is a cache hit, and CI is pinned to an immutable tag rather than to `:latest` — which closes the gap the first version of this entry admitted to and deferred.
  Found while fixing, and it would have been the next failure: `env` is not a context a job's `container:` key may read — only `github`, `needs`, `strategy`, `matrix`, `vars` and `inputs` are. The `full` image was spelled with `${{ env.REGISTRY }}` and would have resolved to an empty registry. Both image names are now computed once in the `environment` job and arrive through `needs`.
  *exit:* **met.** Run `33461047717` at `f51c45f`, twenty checks green on a pull request. The parts this repository could state on its own — every job names the image, no job installs a tool — were true when this was written; what was missing was the observation that it works, and `E0-B16` is why that was worth waiting for rather than asserting. The wrapper it records had never run once and reported success the whole time.
  **The thing this unblocked is worth more than the thing it asked for.** An image per job is a tidiness argument until two jobs have to agree on a number: `E0-P02` compares a boot log hashed on two runners, and QEMU's version is *in* that log. Without one image the check would have failed for a reason with nothing to do with the kernel, which is how a gate gets muted in its first week. Both of those cleared on this run, and the order was not a coincidence.
  *needs:* E0-B16
- [x] **E0-P15** `S` The claim harness refuses to record a timing measurement when `F_ENVIRONMENT` says it is not a measurement environment.
  An **allow-list**, `MEASUREMENT_ENVIRONMENTS` in `bench/src/lib.rs`, and the direction is the whole of it. A deny-list records by default, so every environment nobody thought of — a new CI runner, a colleague's laptop, a virtual machine on a shared host — produces a publishable-looking number until somebody notices. This records nothing by default and adding a machine is a reviewable diff with a reason, which is what `DETERMINISM_ALLOW` already does for the other policy nobody can enforce by attention. The names are the same `runner` classes the claims name, because they are the same statement: the claim says which class of machine can defend it, and this says which class may speak.
  **Unset refuses.** That is the case the rule exists for and the one nobody exercises by hand — an unset variable is not evidence of bare metal, it is the state of every machine that has never been told what it is. An empty or whitespace value is the same state wearing a value, which is what a CI expression resolving to nothing produces, and it is refused too.
  What is refused is the **summary**, not the drawing. The distribution still prints, because it is how anybody debugs a workload and refusing to draw it would push people to a second harness that does. The one line that gets copied into a document, quoted in a review or pasted into a chat with the environment left behind is the line that does not appear. And the refusal reaches the artefact as well as the terminal: `Sample::persist` writes nothing and returns an error naming the machine, because a harness that prints a refusal and writes the file anyway has left a number on disk for something else to pick up.
  *exit:* met, all three ways. In the container: `latency refused — container is not a measurement environment`, with QEMU's emulated timer and the shared cores named, and no file written. Unset: refused, naming the omission. `F_ENVIRONMENT=runner-class-A`: `latency n=31250 min=3 p50=6 p99=15 p99.9=247 max=51425`, written to `claims/ring-submit-latency.local.jsonl`.
  *needs:* E0-B16, E0-P04
- [>] **E0-P16** `M` RustMC on the cursor protocol: the model check the litmus tests are not.
  Promised "at M5" by `docs/TESTING-STATUS.md` and `docs/design/proving-ground.html` since before there was a ring, and M5 arrived at `E0-B12` with no task ID owning it — a promised layer with no owner, which is the decay this file polices everywhere else. The surface doubled at the same moment: the completion ring is a second `Release`/`Acquire` pair, and the batch path covers two relaxed writes per entry with one store. The litmus tests sample what one machine happened to do; RustMC explores what RC11 permits, which is the only kind of evidence that reaches the interleaving no runner has produced yet.
  *exit:* **not met, and it cannot be met under the current toolchain pin.** RustMC is not a crate. It is an extension of GenMC that model-checks Rust by handing rustc-emitted LLVM IR to GenMC's interpreter, so it needs GenMC built from source against an LLVM major that the pinned rustc's bundled LLVM agrees with — and `rust-toolchain.toml` pins `nightly-2026-08-01`, which `CLAUDE.md` forbids moving as a side effect of another change. Whether the two line up is the load-bearing unknown and the first thing to find out; if they do not, the choice is an RFC accepting a second toolchain used only by the checker, or waiting.
  Neither `loom` nor `shuttle` is the answer wearing this task's id. They explore a C11-like model under a `std` harness, which is not nothing and is not *what RC11 permits*; adopting one and closing this would be the "different claim wearing this task's id" the exit was written to prevent.
  **The honest fraction is built, and it turned out to be the half nobody had noticed was missing.** This exit says a model checker must catch *weakening the publishing store to `Relaxed`* — and until now no such defect existed in the tree, so the criterion could not have been attempted even with a checker in hand. `mutate-relaxed-submission` now weakens both the single-entry and the batch store.
  **And running it settled the question this task exists to ask.** The defect was put in front of the stress suite as a CI gate on the AArch64 runner, and the suite passed with it on. So the case for this task is no longer *the litmus tests might miss something*; it is *the litmus tests were given the exact defect this exit names, on the exact hardware it is about, and did not see it.* The fixture a checker needs is in the tree and the gap it has to close has a measurement beside it.
  Also corrected: `.claude/skills/memory-ordering/SKILL.md` and `ring/tests/litmus.rs` both said RustMC "lands at M5". M5 arrived at `E0-B12` and the checker did not, which is the promised-layer-with-no-owner decay this file polices — the sentence that made this task exist, still being told two files away.
  **The load-bearing unknown is measured, and the answer is no in both directions at once.** The pinned `nightly-2026-08-01` is rustc 1.99.0-nightly bundling **LLVM 22.1.8**; RustMC requires **LLVM 21** and `nightly-2025-08-20`; upstream GenMC supports 15 through 20. GenMC's README states the rule that makes this binding — *"GenMC must be compiled against the same LLVM major version used by the Rust installation"* — so there is no configuration of the two that meets in the middle. RustMC is already a major ahead of upstream GenMC and still one behind this tree.
  **So the choice the exit named is made, and it is not the one that costs nothing.** Waiting has no trigger: RustMC pins a *specific* nightly because it is a research tool tracking its own LLVM, while this tree bumps its pin as a reviewable commit whenever there is reason to. Both ends move, and they move apart. RFC 0022 accepts a second toolchain, pinned by the checker rather than by us, in an image target only the checking job uses — the primary pin does not move, and because a checker reports a verdict rather than a number, nothing about this reaches `claims/`.
  **And the task is not what this entry thought it was.** `cargo rustmc test` compiles a crate's *test targets* and explores their interleavings exhaustively, so the existing suite cannot be handed to it: `ring/tests/litmus.rs` runs 500 000 rounds across two threads, and exhaustive exploration of that is not a long run but a non-terminating one. What this task owes is **the small tests a checker can exhaust** — two threads, one or two entries, a handful of operations — kept beside the stress suite rather than replacing it. The two answer different questions, and the file that describes the stress half should stop implying a checker would simply run it.
  **The feasibility fact is measured, and it holds.** `nightly-2025-08-20` is rustc 1.91.0-nightly with **LLVM 21.1.0** — RustMC's requirement confirmed from the toolchain rather than from its README — and under it `f-ring` builds, library and test targets and the `f-env` dev-dependency, with the whole suite passing: 27 + 4 + 6 + 8 tests and the doctests, `--release`, all green. Nothing in `ring` or `abi` needs a compiler newer than the checker's, so RFC 0022's option is live rather than merely plausible.
  Two things about the environment came out of measuring it. **The development image refuses a second toolchain at run time on purpose** — `/opt/rustup` is `a+rX` and the entrypoint drops root, so the pin cannot drift from under a build — which means the checker's toolchain has to be *built into an image*, the shape RFC 0022 proposed before this was known. And **`RUSTC` alone does not move a build**: `rustdoc` resolves separately, comes back through `rust-toolchain.toml` as the pinned compiler, meets rlibs built by the other one, and fails with `E0514` naming a compiler nobody asked for. That cost one confusing run and is written down so it costs nobody else one.
  *what remains:* the image target, and the small tests. Neither is blocked on anything outside this repository any more, which is the change this cycle made — the blocker was a question, it has an answer, and the answer was checked.
- [x] **E0-P17** `S` Litmus stress for the completion ring, on both architectures.
  RFC 0018 built `Poster`/`Collector` as the mirror of `Producer`/`Consumer` and inherited the ordering argument wholesale — and every test in `ring/tests/litmus.rs` still drives the submission half only, so the inherited argument is the one kind of claim that suite exists not to take on faith. The standing rule is that a new `Release`/`Acquire` pair owes a stress test that fails under the weaker ordering; the completion ring added two.
  Three tests, and the third was not planned. `posted_completion_is_fully_visible` is the mirror invariant with a payload self-describing across all four words a completion carries — `result` and `timestamp` restate `user_data`, `ext` restates its complement — so a partially visible completion is visible as one. `free_never_exceeds_capacity` races the two completion cursors on two threads, which is the only way to race them; an over-count there is worse than a wrong number, because `Service::drain` asks `free()` before it takes work, so it is a service accepting a submission it cannot answer.
  **The third test was wrong first, and the completion ring is where it had to be.** `a_hostile_client_cursor_never_panics` was written by copying the submission ring's case list, which includes `u32::MAX`. On the completion ring that is not hostile: cursors wrap, and with `head` at zero a `tail` of `u32::MAX` is the legitimate state of a ring with one completion outstanding across the wrap — `cursors_may_wrap` asserts exactly that. The two rings have their cursors' roles the other way round, which is the thing the copy silently dropped. The list is now the differences a wrap cannot explain, with the reason written beside it.
  The defect is a feature rather than a paragraph: `mutate-relaxed-completion` in `ring/Cargo.toml` weakens the posting store to `Relaxed`. It was a CI gate on the AArch64 runner for one run, and **the suite passed with it on** — see the exit below, which is where that stopped being a wiring problem and became the answer.
  Found while wiring it: `lint-mutations` read `kernel/Cargo.toml` and nothing else, while `DEFECTS` is one list on purpose — its own comment says a second list is how the second defect gets forgotten. The kernel held the only defect until now. It reads every manifest in the workspace.
  *exit:* **one half met, and the other half answered in the negative — which is worth more than it sounds.** Green on both architectures: met. Eight litmus tests pass `--release` on x86-64 and on the AArch64 runner, three of them new and driving `Poster`/`Collector`.
  *"and weakening the posting store to `Relaxed` has been shown to make them fail"*: **shown not to.** The gate ran, on the arm runner, on the machine where the weakening is a real defect, and the suite passed. That is not a wiring failure — `mutate-no-doorbell-fence` proves the same feature mechanism catches a defect it can see — it is this suite's own first paragraph arriving as evidence. Stress tests sample what one machine happened to do, and iterating the harness would move the probability rather than produce a guarantee.
  So the gate is gone and the finding is written down everywhere the removal would otherwise read as a check somebody muted. The exit as written cannot be met by this suite at all; what it asks for is a model checker, which is `E0-P16`, and the size of the gap between them is now measured rather than assumed. **Whoever rewrites this exit should read that as the result rather than as a failure to reach it.**
  *exit, rewritten, and closed against the rewrite:* the completion ring is driven by the same suite that drives the submission ring, on both architectures, and **the suite's catching power is stated rather than assumed** — one of the three defects it is pointed at, which one, and on which runner. Green at run `33461047717`, `f51c45f`: eight tests, `--release`, x86-64 and AArch64.
  That is a weaker criterion than the one this task was written with, and it is the one the evidence supports. The stronger criterion did not go unmet through lack of effort; it was asked of CI twice and answered twice, and both answers are above. What is owed now is a model check, and `E0-P16` owns it with the two fixtures already built and waiting — which is a better position than this task started in, where the exit named a defect that did not exist.
  *needs:* E0-B12

- [>] **E0-R01** `M` `cargo xtask release` produces the full package: source tag, claims snapshot, content-addressed QEMU image, seed corpus, baseline configurations, and a dependency manifest.
  It builds: one `.tar`, a `MANIFEST` naming every file and its SHA-256, and one content address over the archive. Twenty-four files and one address, from `cargo xtask release`.
  **The crux was the two contents no E0 task produces**, and getting it wrong in either direction was easy. Shipping the package with them missing makes the contract advisory, and shortening the list to fit the work is the silent scope cut `A-07` stands against and `E0-D08` already caught once. So the requirement is a **predicate over the registry**: a content is owed when the claim that needs it publishes a number. The baseline and the seed corpus both serve `ring-submit-latency`, which is `pending`; the day it is not, the packager refuses and names `E1-D06`. A scope cut becomes a gate with a known trigger, and the trigger is a status change in `claims/` rather than an edit to the tool. RFC 0021, which also states the consequence rather than leaving it to be found: `E0-R04` needs `E0-P05`, `E0-P05` makes 0001 gating, and a gating 0001 publishes `ratio_vs_baseline` against `linux-6.x-tuned` — so release 0.1 pulls `E1-D06` forward or does not publish that ratio.
  **SHA-256 and the archive writer are in the tree, and that is the contract's own rule rather than taste.** `release --dry-run` computed hashes by shelling out to `sha256sum`, so the content address of a release depended on which coreutils the machine had — and on a machine without it the manifest simply printed no hash column. An address that is sometimes absent is not an address. `xtask/src/pack.rs` is both, checked against FIPS 180-4's published vectors, in a file whose whole subject is that it has no variable fields: no mtime, no uid, no user name, no directory order, and no compression, because a deflate stream carries its encoder's version into the bytes.
  The source is `git archive` of the commit rather than a walk of the working tree — it takes its file list from the tree object and its mtimes from the commit, so it cannot pick up an untracked file and cannot vary with when the checkout happened.
  *exit:* **half met, and the other half is now asked rather than deferred.** `cargo xtask release --twice` packages the same tree twice and requires one address; it passes, and the command says in its own output why that is the weaker half — directory order, uid, path and clock are all constant within one machine. The real question needs two runners at one commit, and `cargo xtask release --address` plus the `package` and `address` jobs are what ask it: one line per runner, compared by a third job. Still `[>]`, because a workflow that has not run is a wrapper that has not run, and `E0-B16` is this file's scar for treating those as the same thing.
  **The same-path precondition was stated here as an argument and is now a measurement**, which matters because the whole job is built on it. The same tree, same image, same commit, packaged at two paths:
  ```
  /work        e544abc2009007758433d33c51e00650190b045d060f09677be29c4be76cbc13
  /elsewhere   91189800d63270033f6160ab4b0c0b2290a4ea67149edec9399e17f3c98b611a
  ```
  Neither number can be re-derived at a later commit, and the reason is worth keeping: the line recording it is inside the source archive the address is taken over, so writing it down changed it. An address in the tree it addresses is never a constant, which makes these two evidence of a *difference* rather than values to check against — and is a decent argument for why the check compares two runners at one commit instead of comparing either against something written down.
  A container job's workspace is `/__w/<repo>/<repo>`, fixed by the runner rather than chosen by this workflow, so the precondition holds by construction — and the comparison checks it anyway and refuses to report an address difference as a finding when the paths differ, because a precondition holding by construction is one nobody notices stopping.
  **`CARGO_TARGET_DIR` is not one of the paths that matters**, and that was worth the second measurement: same source path, target directory moved to `/tmp/t`, same address. So the constraint is on the checkout and not on the build directory, which is the useful half of the finding — someone will want to relocate the target directory for caching, and this says they can.
  Also measured while asking: **a from-scratch kernel build at one path reproduces the address exactly.** Deleting `target/x86_64-unknown-none` and rebuilding gives `e544abc2…` again, so the image is byte-reproducible under a pinned toolchain and the two-runner check has something to pass rather than something to discover.
  *needs:* E0-D08, E0-P02
- [>] **E0-R02** `M` `cargo xtask reproduce <claim>` re-runs any published claim from a clean checkout.
  The verb is a **dispatcher over the registry** and not a reimplementation per claim: the claim file says how it is reproduced, and this reads that and does it. Anything else is two accounts of how a number was taken, which is the decay the registry exists to prevent. It prints the claim, the commit — with a `(dirty)` marker, because a number quoted from a tree nobody can identify is the defect `release --dry-run` was carrying at `E0-P01` — the machine class it needs, what this machine is, and then runs the claim's own command.
  **The three endings are words, not exit codes, and an honest refusal exits zero.** On every machine this project can reach the refusal path is the one that runs: `F_ENVIRONMENT` is `container` in the development image and unset everywhere else, and the harness fails closed on both, so the workload runs, the distribution is drawn and printed, and recording is refused with the reason. Making that red would paint every local run red, which is how a check gets muted — this file has the scar twice.
  **The name collision was real and is resolved.** `reproduce` meant the `E0-P02` determinism check, which is now `cargo xtask trace` and `trace --hash`; its artefact is a trace hash and its subject is the determinism contract, while *reproduce a number* is the word `RELEASING.md`, the long plan section 09 and `proving-ground` layer 7 all already use. The old spelling answers with a message naming where it went rather than an alias, because an alias rots and a signpost is read exactly when it is needed.
  **Two live defects fell out of it.** `claim_run` resolved a name by reading `claims/` unsorted and unfiltered and taking the first `ends_with` hit — so it could match `README.md`, and among two real candidates it picked whatever the filesystem handed back first; it has an exact-then-unique-suffix resolver now, with an ambiguous name an error that lists the candidates. And **`cargo xtask claim timer-jitter` had never worked**: the workload binary was derived as `name.replace('-', "_")`, with a `strip_prefix` special case bolted on for the one claim that did not fit, and it asked cargo for a `timer_jitter` binary that has never existed. Claim 0002 also published `cargo xtask timer 60` as its reproduction, so the registry's one command was not the registry's one command. `ROUTES` is now a table of three, and `lint-reproduce` is what stops it drifting from the registry again.
  `lint-reproduce` is `RELEASING.md` gate 2 made executable, and it asserts four things: the command exists; it is `cargo xtask claim <this claim's own name>`, so no claim can publish somebody else's command or a step outside the tree; the name routes to something that runs; and `[hardware] runner` names a class with a specification file beside it — which only became checkable when `E0-D10` wrote one.
  Found while writing the README section: `docker\dev.ps1` had no `verify` verb. The supported environment could not run the command `CLAUDE.md` tells every session to run before review without falling through to `x`.
  *exit:* **the mechanical half is met and the human half is not, and only a human can close it.** "A person who has never seen the repository reproduces claim 0001 from the README in under thirty minutes, including toolchain install" is an observation about somebody else's afternoon. What exists now: a README section that is the whole route, one command, and a lint that keeps it true. What is owed: a named person who has not read this tree running it on a machine of their own, with the wall-clock time and where it went recorded here. Until then `[>]`.
  Also unmet in the packaged sense — *unpack the release archive, run one command* — which is `E0-R01`'s. The reading this task can guarantee is the other one: the reproduction needs nothing outside the repository, and `lint-reproduce` is what asserts it.
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

- [x] **E0-B21** `S` An xtask verb computes the unsafe percentage A-05 reports.
  A-05 fires for the first time at release 0.1 and there is no tool behind it: nothing counts lines inside `unsafe` against RFC 0001's under-5% target and 10% reversal trigger, so the first report would be somebody's grep — a rule kept by attention, which is the failure `lint-unsafe` already exists to prevent for the same policy. Cheap now and central later: E1 imports drivers behind the boundary, and `E5-D02` must state a fallback's cost to this exact metric in advance rather than discover it.
  *exit:* met. `cargo xtask unsafe` prints both shares and a row per frame crate; `--by-file` prints every file that contributes, sorted by how much. A-05 names the verb and carries the standing number.
  It reports and does not gate, and that is a decision rather than an omission: RFC 0001's trigger is *"exceeds 10% of the codebase **by phase 02**"*, and the phase is half the condition. A verb that went red at phase 00 would be a gate with no path to green, which this file records the fate of twice. The reversal is written on the verb: at phase 02 the same number stops being a trajectory and `lint_all` gains a line.
  **The first answer was wrong, and the number is what caught it.** The kernel came out at 32%, which was the first thing about the result that looked implausible. `#[unsafe(no_mangle)]` is the 2024 attribute form and opens no block; the scanner read it as one, attached the next brace in the file — the body of the function being annotated — and counted `kmain` entire. A second scanner defect went the same way: this file's own error messages are string literals continued across lines with a trailing backslash, and one of them contains the words `lint-unsafe`, which a line-at-a-time scanner read as the keyword. Both are why the counting rule is written down and why the verb understands comments, raw strings and the difference between `'a'` and the lifetime in `Producer<'m>`.
  Also fixed in passing: `lint-snapshot` existed since `E0-P01` and was not in `cargo xtask help`. A verb nobody can find is a verb nobody runs.

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
- [ ] **E1-D06** `M` The tuned-Linux baseline, as configuration rather than as prose.
  Found by `cargo xtask release --dry-run` at E0-D08: the release contract requires the baseline configuration in the package, and nothing in this file produced it. `claims/0001` says `linux-6.x-tuned` with a sentence of notes, which is the decay the contract exists to prevent — a tuned comparison becomes a stock comparison as the baseline ages and nobody re-checks it, and prose cannot be re-checked because it cannot be run.
  Belongs in E1 rather than E0: the first claim it has to configure a baseline *for* is the datapath set at `E1-P10`, and a baseline written before there is a workload to tune it against is a guess with a filename. `A-04` is the standing item that keeps it honest afterwards.
  *exit:* the tuned baseline is a file in the tree that a stranger can apply to a machine and get the configuration a claim was compared against; `cargo xtask release --dry-run` reports it present.

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
  *needs:* E1-D01, E1-D04, E1-B13 (a supervisor is the component the fixed table breaks on)
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
- [ ] **E1-B11** `S` A splittable generator behind `Env`, before the simulator multiplies streams.
  `SeededEnv` runs on xorshift64 and `sim.rs` finalises its site draws with FNV-1a — both chosen for reproducibility, neither for statistical quality, and both say so. One seeded test cannot feel the difference; a nightly sweep of thousands of seeds across correlated streams can, and a sweep whose streams are secretly correlated explores less than it reports. A splittable design — SplitMix64-derived streams under a PCG- or SFC-class generator — is the standard answer, and the `Env` trait makes it a one-crate change. Seeds bind to a commit, so nothing recorded breaks: a new generator is a new commit. Before `E1-P01` and not after, because the seed corpus the simulator accumulates is priced in the generator it was drawn from.
  *exit:* `SeededEnv` and the site-draw finaliser share one derivation; the existing per-site independence test still holds and a new one bounds cross-stream correlation; `E1-P01` is built on it rather than migrated to it.
- [ ] **E1-B12** `L` The allocator the design names: buddy orders, per-CPU free lists, huge pages by default.
  `deadline-all-the-way-down` section 03 has specified this since before M1 and no task owned it — `mem.rs` says "this is the M1 floor" and points at a design document, which is a promise with no owner now that the floor is load-bearing. `Order` has been in every signature since M1, so the call sites are ready; what arrives here is orders above zero with split and coalesce, per-CPU lists so two cores allocating never meet, and `Order::HUGE` as the default grain. The drivers can run on order-0 frames through scatter-gather, so this does not gate them; what it buys is the huge-page default and uncontended allocation, both of which the datapath claims will price.
  *exit:* order-9 and order-18 allocations succeed, split and coalesce under an adversarial alloc/free workload; allocation takes no cross-core traffic on the hot path, counted rather than asserted; the M1 free-list pair is retired rather than kept beside it.
  *needs:* E0-B10
- [ ] **E1-B13** `M` The capability table becomes an object: growth paid by `Untyped`, quota made real.
  `cap.rs` records its own reversal condition — a component that legitimately holds more than [`TABLE_SLOTS`], which is E1's first real supervisor — and the supervisor is this epoch. So the table stops being a fixed `PerCpu` array and becomes storage an `Untyped` capability pays for, which is the same change as giving a process a quota. The revocation walk stays iterative and bounded; what changes is whose memory bounds it.
  *exit:* a process holds more than the fixed count with the growth debited from its `Untyped`; the five properties and the whole negative suite pass at the new size; a process that cannot pay is refused with `QUOTA_EXHAUSTED` rather than served from kernel reserve.
  *needs:* E1-D01
- [ ] **E1-B14** `M` Shootdown batching, bought by a number or closed by one.
  Every revoke-unmap today is one page, one IPI, one spin on an acknowledgement — correct, and priced for a kernel that unmaps rarely. The datapath changes the rate: registered buffers cycle, and a driver restart unmaps a component's whole grant page by page. Batching is what mature kernels do here, and it is also exactly what rule 3 forbids designing before the measurement exists — so the workload comes first and the number decides.
  *exit:* an unmap-under-churn workload exists beside the `E1-P10` claims and records shootdowns, IPIs and p99 unmap cost; then either batching lands with the improvement measured on the same workload, or this task closes `[~]` with the number that says one-page-one-IPI was already under the bound.
  *needs:* E1-B02

### Prove — this is the epoch where the testing environment becomes real

- [ ] **E1-P01** `XL` **The deterministic simulator.** Virtual time, seeded scheduling and ordering, device models for blk, net and gpu, and component substitution.
  *exit:* a whole boot-to-workload run executes under simulation and reproduces byte-identically from `(seed, commit)`.
  *needs:* E0-P02, E1-B11 (the seed corpus is priced in the generator it was drawn from)
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
- [ ] **E1-P12** `M` Kani on the ring's validation paths: panic-freedom proved, not sampled.
  `E1-P07` proves the capability properties; the same tooling reaches the other structure a hostile peer feeds bytes to. `pop`, `take`, `Layout::adopt` and `execute` each promise that nothing a peer writes produces a panic, and today that promise rests on fuzzing that samples and a clippy wall that guards this crate's own code. A bounded proof over arbitrary header bytes, cursors and entries is cheap for code this small, and it is the difference between "no fuzzer found one" and "there is none".
  *exit:* the proofs run in CI on the schedule `E1-P07` establishes; reintroducing the unchecked index that `mutate` removes fails a proof, not only a boot.
  *needs:* E1-P07

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
- [ ] **A-05** Report the unsafe-code percentage against the under-5% target. RFC 0001 reverses at 10%, by phase 02.
  *mechanism:* `cargo xtask unsafe`, and `--by-file` for the reason it moved, which is almost always one file. The counting rule is a doc comment on the verb rather than a convention: lines inside an `unsafe` block, an `unsafe fn` body or an `unsafe impl` header, over lines with code on them, with comments and string literals taken out.
  *standing:* **12.8% of the tree and 19.4% of the frame** at the end of E0, against a 5% target. Over the figure in RFC 0001's reversal condition and not over the condition, which is a phase-02 one. What the number is made of is the report: `paging.rs` is 64.6% and `port.rs` is 95.7%, and a tree that is almost entirely a kernel has no denominator yet. E1 is what supplies one, by putting drivers above the frame.
  It was **14.2% when `E0-B21` first computed it**, and the fall is the item working rather than the tree improving: `E0-B14` and `E0-B15` added a wire format, a reader, a doorbell and their tests, almost none of which is `unsafe`, so the denominator grew. Saying which way it moved and why is the whole of what this item asks for, and the first time it was asked the answer was already interesting. `docs/design/lineage-and-debts.html` says F holds its frame "to a few percent" — an intent the code is not at, and the gap is this item's to report rather than the document's to quietly restate.
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
