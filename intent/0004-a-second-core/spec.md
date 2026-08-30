---
id: 0004
status: agreed
reviewed_by: Dmitri Chudinov
skills: frame-and-unsafe, determinism-review, memory-ordering, rfc-author, claims-registry
---

# Spec: a second core, a mapping that goes with its name, and a program from outside

Every core the machine has is brought up and left waiting. A process is built by
the boot processor and run by another one, inside a timer window each of them
opens for itself. Revoking a capability that authorised a mapping takes the
mapping with it, everywhere. The first program a boot runs comes from the boot
loader rather than from the kernel image. And the one property of the capability
suite that could not have a fixture gets a build that breaks it instead.

## Behaviour

**Bringing a core up.** An application processor comes out of reset in real
mode, and the interrupt command register supplies its instruction pointer as one
byte: the page number of where to begin. So the boot processor writes a
trampoline into a fixed low page, sends the assert-`INIT` and two startup
interrupts the architecture prescribes with the delays it prescribes, and waits.
The trampoline walks real mode to protected mode to long mode and jumps to the
higher half — the boot stub's sequence again, in miniature, and written
separately rather than shared because the two differ in the one way that
matters: the stub builds transitional page tables and this does not. The
kernel's real address space already exists by then, so the arriving core loads
*that* into `CR3` and is in the finished address space from its first paged
instruction.

Bring-up is serial. One core is started and waited for before the next is
touched, because the trampoline is one page with one stack pointer in it.

**The on-ramp.** Enabling paging does not move the instruction pointer, so the
instruction after `mov %eax, %cr0` executes at the trampoline's own low physical
address. Exactly one page — present, readable, executable, not writable, not
reachable from ring 3 — is identity mapped into the kernel's address space for
the length of bring-up and withdrawn afterwards, tables and all, so the free
count comes back. It is in the lower half, which is otherwise empty in the
kernel's own space and is copied into no process's.

Withdrawing it is the first shootdown this kernel performs, and it is a real
one: the arriving cores walked those tables.

**Stacks.** A block per core, reserved by the linker script: guard, stack,
guard, fault stack. Reserved rather than allocated because of an ordering
problem with no good answer at run time — a stack needs a guard page, a guard
page is a hole in the kernel window, and the only code that can leave a hole
there is the mapper that *builds* the kernel window, which has finished long
before a core is started. The cost is `.bss` reserved on a machine that never
uses it, and it is stated in the linker script rather than discovered.

The double-fault stack becomes per core in the same change. Before it, two cores
taking a double fault would have written their exception frames to one address —
at the moment each was trying to report why it could not use its own stack.

**What crosses a core boundary.** Four `PerCpu<u64>` shards, and nothing else: a
mailbox, and a page, a sequence number and an acknowledgement for the shootdown.
Every one is a machine word in the slot of the core it is *about*, and every
access on both sides is an atomic with its ordering named at the access. RFC
0016 is the argument, and the ordering is not decoration — both protocols
publish something that is not the word being written. The mailbox publishes a
handoff structure; the shootdown publishes a page table edit. `Relaxed` passes
on x86-64 and corrupts on AArch64, which is the trap `CLAUDE.md` already records
about the ring.

**Where a process runs.** The boot processor builds it — an address space, four
pages, a capability table, a job — and another core runs it. That split is not
arbitrary: allocating and freeing are the frame allocator's, and the frame
allocator belongs to one core; taking a privilege-level transition is something
any core can do. So `prepare`, `execute` and `reap` are three functions on two
cores, and the allocator is reached from the running core through a shared
reference only, for exactly as long as the process is alive.

The running core arms its own timer, because the frame answers "have you run
long enough?" by counting ticks taken *out of ring 3*, and only this core's
timer can take one out of this core's ring 3. The two schedules are independent
— separate local APICs, separate deadlines, separate histograms — and neither is
a term in the other.

On a machine with one core the same code runs both, and the windows are
sequential rather than concurrent. The boot log says which shape it was.

**Revocation reaches the mapping.** A slot records the address its object was
mapped at. `CAP_MAP` writes it after the mapping exists and not before; a slot
that recorded an address the tables do not have would make the next revoke unmap
somebody else's page. A capability may be mapped once, and a second attempt is
refused rather than recorded — a real bound, and not the general answer, which
is a mapping database with an owner and a quota, which is E1's `Untyped`.

`Table::revoke` returns the addresses whose mappings the withdrawn capabilities
authorised, and does not undo them: undoing one is a page table edit followed by
an interrupt to every other core, and `cap.rs` knows about neither. The caller
clears the entry, invalidates locally, and shoots down. A shootdown that is not
acknowledged ends the machine, because the alternative is telling a process the
authority is gone when it is not.

**A program from outside.** `user/init` becomes a component: ordinary Rust with
no `unsafe`, compiled and linked separately, handed over by the loader as a
module, copied into a frame the process owns. The kernel runs it *and then* runs
the frame's own adversary, on every boot, so that every boot checks what M4 could
only assert — that a table cleared between processes does not let the second
resolve a handle the first held.

That last property is also what forced a change to the door: a component is now
**told** its starting handles in the word it is entered with, rather than
entitled to assume them. The second process on a core finds its capabilities at
the same indices and a later generation, and a component that assumed otherwise
is refused correctly for a reason that looks nothing like the mistake.

**The mutation build.** The kernel gains a feature that makes it wrong on
purpose — the capability table subscripts a handle's index instead of checking
it — and `cargo xtask mutate` requires a boot of the forging sweep to go red
with a kernel panic, then requires the same boot without the feature to go
green. RFC 0017 is the argument. Neither half means anything alone.

## What this is not

- **A scheduler.** One process at a time on one core, started and waited for.
  Placement is "the other core", which is the whole of the decision because
  there is nothing to decide between.
- **A measurement.** Nothing here publishes a number. Under the emulator the
  timer's p50 is two orders of magnitude past the bound claim 0002 names and the
  run-to-run spread is larger than any difference between builds; the claim
  stays `pending` and E0-P06 owns it.
- **Topology discovery.** The core count comes from `cpuid` leaf 0x0B, which is
  one package's and assumes dense APIC ids — the same assumption `current_cpu`
  already makes. The ACPI multiprocessor table is the right answer and arrives
  with E5.
- **A general loader.** The image is flat, one page, mapped at a fixed address.
  A component that outgrows a page or wants its data separate from its text
  needs a loader that reads headers, which is E5.
- **Two processes at once.** They are sequential. The isolation *between* two
  running processes is still unchecked, because there is still only ever one.

## Standing policies

**Determinism.** No new source of it. The two spin loops here read the
timestamp counter through `arch::read_tsc`, which is the one call site the
allow-list already has, and they are delays the architecture requires between
two writes to a hardware register rather than observations of time — nothing is
recorded, nothing reported, and no value reaches a decision a seed could
reproduce differently. The boot log stays byte-identical for a `(seed, commit)`
because a started core prints nothing.

**The frame.** Every new `unsafe` block is in `abi/`, `ring/` or `kernel/`, and
`abi` is where the door's calling stub landed. That placement is the interesting
one: a component may not contain `unsafe`, and making a system call is an
instruction, so the two instructions that cross the boundary live in the crate
whose entire subject is what crosses a trust boundary. `user/init` still
inherits `unsafe_code = "forbid"` and `lint-unsafe` still passes.

The same policy has a consequence that had not been noticed before: a crate that
forbids unsafe code cannot name an entry point, because `#[unsafe(no_mangle)]`
and `#[unsafe(link_section)]` are unsafe *attributes* in this edition. So the
component is a library, the placement is a linker script, and `cargo xtask init`
checks that the symbol which landed at the image's first byte is the one that
was meant to.

**Memory ordering.** Two new release-acquire pairs, both in `kernel/src/smp.rs`,
both documented at the store and at the load with what they publish. No existing
ordering changed, so no litmus test is owed.

**The licence boundary.** Untouched. Nothing here imports anything.

**Reversals need RFCs.** Two are owed and written. RFC 0016 amends what a
`PerCpu` shard means, because a handshake cannot be per-core state. RFC 0017
introduces a build that is wrong on purpose, because a property with no possible
fixture needs some other way to be falsifiable.

**Numbers need claims.** None are published. The jitter distributions taken
while writing this are in a container, which `docker/README.md` and
`claims/README.md` both say is not a measurement environment.

## Evidence

- `cargo xtask verify` — lint, test, boot, and the mutation harness, all green.
- `cargo xtask run` — the boot prints `cores 2 of 8 shards`, `init 224 bytes
  from boot module 1 of 1`, and two processes on core 1: the component ending
  with status 0, then the adversary killed by the provocation it was given.
- `cargo xtask cap` — eight boots. Seven as before, plus `cap=unmap`, where the
  process maps a frame, has the capability behind it revoked, reads the page
  anyway and takes a page fault at the address it had been reading a moment
  earlier.
- `cargo xtask user` — the seven M3 provocations, now on a core that is not the
  one holding the timer, and after a first process has already lived and died on
  it.
- `cargo xtask fault` — all six, unchanged.
- `cargo xtask mutate` — the defect is caught, and the same boot passes without
  it.
- `cargo xtask init` — the image is one page, has the right symbol at its first
  byte and no writable data in it.
- Two runs of one commit are byte-identical.

## Risks and reversal

**The core count is a guess with a good pedigree.** `cpuid` leaf 0x0B counts one
package. A two-socket machine is undercounted and its cores are never started;
sparse APIC ids would be worse, because `current_cpu` uses the initial APIC id
as a shard index and a sparse id past `MAX_CPUS` panics in `PerCpu::at`. *What
would reverse this:* the first machine that is not a single-package emulator,
which is E5, and the fix is the ACPI multiprocessor table.

**A mapping is recorded once per capability.** A frame capability legitimately
mapped at two addresses is an ordinary thing to want and is refused here. *What
would reverse this:* a component that needs it, at which point the slot's one
address becomes a list with an owner and a quota — E1.

**The deliberate defect is in the shipped source.** Behind a feature that is off
by default, in one function, with an `allow` that names itself and a lint that
refuses to let it become a default — and still there. It is the same trade the
flawed capability tables already make, made a second time. *What would reverse
this:* a host harness for kernel logic, which means the frame splitting across
crates, which is a bigger decision than either of these.

**The trampoline page is a fixed physical address.** Thirty-two kibibytes, chosen
because it is inside the region every machine reports as usable and below
everything the loader touches. Nothing checks that the firmware agrees. *What
would reverse this:* a machine where it is not free, which the memory map would
say and this kernel does not currently read for that purpose.

**A shootdown assumes one initiator at a time.** The request words live in the
target's slot and are written by whoever is asking, so two cores asking the same
core at once would overwrite each other's page and could both be satisfied by
one acknowledgement. Nothing can do that today — one process, on one core, and
it is the only thing that revokes — and `smp::shootdown` says so rather than
leaving it implied. *What would reverse this:* a second core that revokes, at
which point the answer is a queue of pending invalidations per core, which RFC
0016 already names as what would reverse its own rule.

**A shootdown that is not acknowledged ends the machine.** That is the correct
answer to "some core may still be reading through a mapping whose authority is
gone", and it is also a denial of service one broken core can inflict on the
whole machine. *What would reverse this:* a system with enough cores that losing
one should not be fatal, which is a fault-tolerance question this epoch does not
have.
