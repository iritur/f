---
id: 0002
status: shipped
originator: Dmitri Chudinov
todo: E0-B09
---

# There is nothing in this system that the kernel has to be protected from

## Problem

Every line of code that has ever run on this machine has been the kernel's, at
privilege level zero, in one address space, with every page it can name mapped
for it. That is not a small gap in the story — it is the story. Phase 00 exists
to prove exactly two propositions, and the first of them is that isolation holds
under adversarial use. Nothing has been adversarial yet, because there has been
nothing other than the kernel.

So every protection built so far is asserted rather than checked from the side
that matters. The page tables carry a bit that says who may read a page and
nothing has ever been on the wrong side of it. The descriptor tables describe a
privilege level nothing has ever been at. The exception handler has been proved
to survive a fault the kernel caused on purpose, which is a different event from
a fault somebody else caused and a different answer is owed to it: when the
kernel faults the machine should stop, and when somebody else faults only they
should.

There is a second problem underneath, and it is the one that gets found late.
The timer is the only measurement this project has, and it has only ever been
measured against an idle kernel spinning in a loop. If holding the core at a
lower privilege level costs the schedule anything, nobody would currently know.

## Proposed outcome

Something runs on this machine that is not the kernel, in its own address space,
unable to reach anything it was not given. It does something it is not allowed
to do. It stops, and the machine does not — and the timer, running throughout,
delivers every tick it was scheduled to.

Observable: a boot where a process announces itself, is preempted by the timer a
stated number of times, faults deliberately, is reported and killed, and the
kernel goes on to finish normally. And a command that provokes each protection
in turn, so that "isolation holds" is a set of runs rather than a sentence.

## Affected users and systems

`kernel/` throughout, and mostly in new files: the transition and the system
call entry, the program on the other side, and what a process is. The descriptor
tables gain the ring-3 half they have always had a comment about. The page
tables gain a second kind of address space. The interrupt dispatcher gains the
one branch that distinguishes a process dying from a kernel dying.

`docs/design/ring-scene-boot.html` describes M3 and does not change.
`docs/design/proving-ground.html` calls isolation-under-adversarial-use the
first of the two propositions phase 00 exists to prove; this is where that stops
being a plan.

Nothing above the frame changes. `user/init` is still a placeholder — loading it
from a boot module is the next task and is deliberately not this one.

## Constraints

The boot log is byte-identical for a given seed and commit, and that must stay
true. A process's lifetime is therefore not allowed to be measured in
instructions: how long a fixed number of instructions takes differs by two
orders of magnitude between the emulator this runs in and a machine, and any
number derived from it would move.

The timer's schedule is the measurement everything else in the epoch rests on.
It has to keep running while a process holds the core, and the boot has to be
able to say so.

The 5 µs jitter bound belongs to M2 and is not met in this environment for
reasons that predate user space. This change may not quietly acquire it.

## Open questions

- Whether a process at this milestone should be able to make any system call at
  all. The design document says the entry is for channel setup, and there are no
  channels yet, so the honest reading of it authorises nothing — which produces
  a process that can only die.
- Whether the frames a dead process leaves behind should be reclaimed now, with
  no capability system to say who owned them, or left until there is one.
- What a fault at ring 3 should print. The register dump exists and is worth
  reading; it is also a hundred bytes of serial output inside a tick interval.
