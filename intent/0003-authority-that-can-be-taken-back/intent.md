---
id: 0003
status: shipped
originator: Dmitri Chudinov
todo: E0-B11, E0-P08
---

# A process can be stopped from doing things, and cannot yet be given any

## Problem

M3 proved the half of isolation that is about refusal. A process reads the
kernel's direct map and is killed; it writes its own text and is killed; it runs
a privileged instruction and is killed. Six protections, six faults, and a
machine that survives all of them.

What none of that shows is the other half, which is the one the architecture
actually claims. Every one of those refusals is a page table saying no. A page
table can only say no about memory, it can only say it about memory that is
already mapped or already absent, and it says the same thing to every process
forever. There is no way to hand a process something, no way to hand it
something weaker than what you hold, and above all no way to take it back.

That last one is the specific claim. The corpus lists *nothing can be revoked*
as a structural drawback of the interface this project exists to replace — a
descriptor, once passed, is gone — and answers it with a derivation tree. Right
now that answer is a sentence in a document. Nothing in the tree can revoke
anything, because nothing in the tree can grant anything.

There is a second problem underneath, and it is the one that decides whether any
of this is believable. The exit criterion for phase 00 is a negative suite: a
process cannot name a capability it was not given, cannot forge a handle, cannot
use a revoked handle, cannot exceed granted rights, and cannot make the kernel
panic by trying. Five properties. A suite of five checks that have never been
seen to fail is five checks nobody knows are wired up — and a capability system
whose test suite passes against a broken table is worse than no test suite,
because it is the same amount of evidence carrying a much stronger claim.

## Proposed outcome

A process holds capabilities. It can look at one, mint a weaker one from it,
mint a copy, and withdraw everything it minted — and the memory it reaches is
memory a capability let it reach, rather than memory that happened to be mapped
before it started.

Then it tries to cheat, on purpose, one way per run: naming something it was
never given, making handles up, using one after it was withdrawn, asking for
more than it holds, and asking for so much that the frame runs out. Every one is
refused, the refusals say *why*, and the machine finishes normally every time.

And the suite is shown to be capable of failing. Somewhere in the tree there are
tables that are wrong on purpose — one per property — and the boot says how many
of them were caught.

## Affected users and systems

`kernel/`, mostly in one new file: what a capability table is and what may be
done to one. The process gains a table, some memory it can only reach by
presenting a capability, and four more things it may ask the frame for. The page
tables gain a way to map something into an address space that is *running*,
which they have not needed before.

`abi/` gains the part that crosses the boundary: what a handle is made of, the
six kinds of object, and the rights. That is an ABI change and should be
reviewed as one.

`docs/design/ring-scene-boot.html` describes M4 and does not change.

Nothing above the frame changes. There is still one process, still on one core,
still with no scheduler.

## Constraints

The boot log is byte-identical for a given seed and commit, and that must stay
true. Anything the capability suite prints has to be a fixed number — which
means the frame has to be the judge of what happened rather than the process,
and the counts have to be exact rather than "at least".

The free count has to come back. A process that can ask the frame for a mapping
is a process that can ask the frame to spend memory, and the existing assertion
— every frame a process was made of goes back — is what stops that from being
discovered as a slow leak later.

The 5 µs jitter bound still belongs to M2 and is still not met in this
environment. This change may not quietly acquire it.

Adding anything to the system call entry is expensive by design. RFC 0014 says
the entry is a door and not an interface, and it says adding a call means
arguing against that in writing. This wants four.

## Open questions

- Whether a copy of a capability should be reachable by a revoke of the thing it
  was copied from. seL4 says no. The corpus's complaint about descriptors says
  yes, loudly, and they cannot both be right here.
- Whether a process should be able to ask for memory it does not already have a
  page table for — which is really the question of when a quota exists.
- Whether the deliberately broken tables belong in the shipped image. They are
  the only way to check the checks, and there is no host test harness for the
  kernel to hide them in.
- What "cannot make the kernel panic by trying" is testable as, given that the
  fixture which would prove it is a fixture that crashes the machine.
