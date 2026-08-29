---
id: 0002
status: agreed
reviewed_by: Dmitri Chudinov
skills: determinism-review, frame-and-unsafe, rfc-author
---

# Spec: user page tables, the ring-3 transition, and a door back

A process is an address space, two pages and a way of ending. The frame builds
one, enters it at privilege level three with the timer already running, answers
the calls it makes, and ends it — by its own request or by killing it for
something it did. Then it takes the memory back and checks that all of it came.

## Behaviour

**The address space.** A top-level table of its own, whose upper half is copied
entry for entry from the kernel's. Copying rather than sharing a pointer is what
makes a system call and an interrupt from ring 3 land on mappings that are
already there — the direct map, the device window the local APIC's registers are
in, and the kernel image. The lower half is built one page at a time with the
user bit set at *every* level, because the processor takes the logical and of
that bit down the walk and a leaf that grants what a parent withheld is a page
the process cannot see.

The copy is a snapshot and the module says so, with the reversal condition
stated: the day the kernel maps something into a top-level slot that did not
exist when a process was built, sharing has to become structural.

**The layout.** Text at 4 MiB, one unmapped guard page, then one stack page —
three consecutive pages inside one two-mebibyte region, so the whole address
space is four tables. Text is executable and not writable; the stack is writable
and never executable. Write-exclusive-or-execute, the same rule the kernel
applies to itself, applied from the side that can violate it.

**The transition out.** There is no instruction for "begin running at privilege
level three"; there is only the one for returning to where an interrupt came
from, and a frame that says it came from there. Five quadwords, pushed in one
place. One register is handed over — which violation to commit — and every other
is cleared, because a register still holding a kernel value on the far side is
an address the process was never granted.

**The transitions back.** `syscall`, and the interrupt table. The first uses
`swapgs` and a per-core block, because `syscall` switches no stack and arrives
with no free register; the second uses the task state segment's ring-0 stack
pointer. Both land on the same address, and it is the stack pointer of the call
that entered ring 3 — everything below it is free and everything above it is the
kernel call waiting for the process to be over. A fixed per-core stack would sit
*above* the live kernel frames and the first tick from ring 3 would overwrite
them.

**The calls.** Three, and RFC 0014 is the argument for why three and not zero
and not thirty. `ANNOUNCE`, which becomes the channel-setup handshake at M5.
`PROGRESS`, which asks the frame how much of the process's time it has taken and
is what a blocking wait on a ring replaces. `EXIT`. A call the frame does not
have is refused in the error space of RFC 0010, as an argument error.

**Ending.** A fault taken at ring 3 is recorded and the interrupt frame is
pointed back at the kernel, so the `iretq` that would have resumed the process
resumes the call that started it. `EXIT` does the same thing with a jump. Then
the kernel's address space goes back into `CR3` and every frame the process was
made of goes back on the free list — four tables and two pages — and the free
count before and after is compared.

**How long it runs.** The frame decides, by counting timer ticks taken *out of
ring 3*: the process polls `PROGRESS` and is told to stop after eight of them.
Not instructions, for the reason the constraints give. Bounded in time by the
same give-up the timer's own wait uses, so a timer that stops firing produces a
reported failure rather than a machine that hangs with no output.

**The provocations.** Seven, chosen by `user=` on the command line, six of which
must fault and one of which must not:

| `user=` | what the process does | what must happen |
| --- | --- | --- |
| `kernel` | reads the direct map | page fault, present, ring 3 |
| `null` | writes to address zero | page fault, not present |
| `text` | writes to its own text | page fault, present, write |
| `stack` | executes its own stack | page fault, instruction fetch |
| `priv` | executes `cli` | general protection fault |
| `call` | makes a call that does not exist | refused, then a clean exit |
| `exit` | nothing | a clean exit, nothing refused |

`kernel` is the default, so an ordinary boot exercises the isolation and not
only the transition — it is the violation whose failure would be worst, because
reading kernel memory from ring 3 is not a crash, it is a quiet success.

## Policy applied

**Determinism (RFC 0004).** The boot log stays a fixture, and the whole design
of the process's lifetime follows from that: the frame counts ticks and the
process asks. The one clock reached directly is `arch::x86_64::read_tsc`, the
existing allow-listed site, used for the give-up bound — so `DETERMINISM_ALLOW`
does not grow and there is still one `rdtsc` in the tree. The measured numbers
that do vary — how many polls, how late each tick was — appear only under
`timer=<seconds>`, which nothing asserts on.

**The frame (RFC 0001).** All of it is `kernel/`. Four obligations are new and
each is discharged where it is taken: an interrupt frame built by hand and
returned through; a `GS` invariant that exists only inside one file; an interrupt
frame *rewritten* so that returning from it goes somewhere else; and frames
freed while the tables that described them were in `CR3` a few instructions ago.

**Per-CPU state.** A process's state is one `PerCpu<State>`, and the count of
ticks taken from ring 3 is a `PerCpu<u64>` outside it — for exactly the reason
`apic::TICKS` is outside `apic::Timer`: two paths touch it, the handler and a
system call, so every access is volatile through the raw pointer and never
through a reference that would be claiming otherwise.

**Reversals need RFCs.** One is owed and written: RFC 0014, on what the system
call entry is for. The design document says the entry is "used strictly for
channel setup", which at M3 authorises nothing, and a reading had to be recorded
rather than assumed.

**Numbers need claims.** None are published. The jitter bound is still M2's,
still `pending`, and still E0-P06's to move.

## Not in scope

- **Loading a real component.** `user/init` from a boot module is E0-B10. The
  program here is a flat blob assembled into the kernel image, with no loader,
  no relocations and no crate behind it.
- **A second core.** E0-B10.
- **A scheduler.** One process at a time, on the core that starts it, for its
  whole life. That is what a system with no scheduler can honestly say.
- **Capabilities.** E0-B11. A process's frames are tracked in a fixed array
  because there is no derivation tree to track them in yet, and the array says
  so.
- **PCID.** Detected, reported, deliberately off. There is one address space
  switch in each direction per boot; tagging translations buys nothing at that
  rate and an identifier that is wrong is a process reading another's memory
  through a stale translation, which is the one failure in the paging code with
  no fault behind it.
- **Unmapping.** A process's pages go away because its whole address space does.
  Shootdown arrives with the second core.

## Evidence

- `cargo xtask run` — an ordinary boot builds a process, runs it, takes eight
  ticks out of ring 3, watches it read the kernel's direct map, reports the
  fault, kills it, gives back six frames with the free count unchanged, and
  finishes the hundred-tick timer window it was all inside.
- `cargo xtask user` — all seven provocations, each in its own boot. Six must
  fault and the kernel must survive every one; the seventh must not fault, which
  is what stops the other six passing for the wrong reason.
- `cargo xtask timer <seconds>` — the histogram, now of a window that contained
  ring 3, with the count of ticks taken from it printed beside the total.
- Two runs of one commit are still byte-identical.

## Risks and reversal

**The upper half is a snapshot.** Stated in `paging`, with its symptom: a kernel
address that works in the kernel and faults inside a process. *What would
reverse this:* the kernel gaining a top-level slot after processes exist, at
which point all 256 upper tables get pre-allocated at boot and every root points
at the same ones.

**One process, one core, no scheduler.** `run` holds the core for the process's
whole life. Everything about ending a process — the resume point, the stack, the
entry block — is one per core because there is one process. *What would reverse
this:* the second process, which is E0-B10's problem and where the entry block
becomes per-process rather than per-core.

**The jitter bound is not asserted and must not look asserted.** What is
asserted is that the schedule delivered every tick it asked for across a window
that contained ring 3. The 5 µs p99 is not met here, was not met before user
space, and is E0-P06's to own. The boot log says how many ticks and not how late
they were, which is the same split M2 made for the same reason.

**A process that is killed leaves a stale ring-0 stack pointer in the task state
segment.** Harmless while nothing enters ring 3 again without going through the
entry path, and the entry path always rewrites it. It is named here because it
is the kind of thing that stops being harmless when a second process exists.
