# RFC 0014: The system call entry is a door, not an interface

- Status: accepted
- Date: 2026-08-29
- Affects: `kernel/`, `docs/design/ring-scene-boot.html`, `docs/design/fast-path.html`

## Decision

The `syscall` entry exists so that a process can reach the frame when there is
no ring to reach it through. It is not an interface and does not accumulate one.
Three rules govern what may live behind it:

1. **A call may exist only if it cannot be an opcode on a ring.** Setting a
   channel up is the archetype: a process cannot submit on a ring it has not
   been given. Anything a process could ask for *through* a ring belongs there,
   whatever it costs to build the ring first.
2. **Every call added before the ring exists names the thing that replaces it.**
   At M3 there are three, and each carries that name in its documentation. A
   call with no successor named is a call somebody intends to keep.
3. **Nothing here is measured, tuned, or made fast.** The whole architectural
   claim of this system is that the hot path does not cross this boundary; work
   spent making the crossing cheaper is work arguing against the claim.

The three calls at M3 are `ANNOUNCE` — the process says it exists, which becomes
the channel-setup handshake at M5 — `PROGRESS`, which asks the frame how much of
the process's time it has taken and is what a blocking wait on a ring replaces,
and `EXIT`. A call the frame does not have is refused in the error space of RFC
0010, as an *argument* error: the process was permitted to ask.

## Context

`docs/design/ring-scene-boot.html` section 15 describes M3 as "user page tables,
the ring-3 transition, a `syscall` entry used strictly for channel setup and
never on a hot path". Read literally at the moment M3 is built, that sentence
authorises nothing at all: the ring lands at M5, so there is no channel to set
up and the only call that satisfies the sentence is one that cannot be written
yet. A process that could make no calls could not announce itself, could not be
told when to stop, and could not end other than by faulting.

So the sentence had to be read as intent rather than as a specification, and
this RFC records the reading — because the alternative readings are both bad and
both plausible to a later contributor. One is that the entry stays empty until
M5 and M3 ships a process that can only die. The other is that the sentence was
about hot paths only, and any call that is not hot is fair game, which is how
every system call interface in history reached its current size.

The live alternatives at the time:

- **A trap gate instead of `syscall`.** Cheaper to write and slower to use, and
  it would have to be replaced at M5 anyway. Rejected for costing the same
  argument twice.
- **No entry at all at M3: the timer ends the process.** It works, and it was
  most of a design before it was dropped. It makes the frame the only party with
  agency, which means the milestone proves the transition *out* of ring 0 and
  never the transition back on purpose — and the transition back on purpose is
  the half that has a bug in it.
- **Start the real interface now: an `Sqe`-shaped call.** Rejected as inventing
  a wire format for a channel that does not exist, which RFC 0011 already argues
  is the expensive kind of guess.

## Consequences

Easy: a process at M3 has exactly as much vocabulary as the milestone needs, and
the boot log can state deterministic facts about a run because the frame — not
the process's instruction count — decides when the process has run long enough.

Hard, on purpose: adding a fourth call requires arguing against rule 1 in
writing. That is the intended cost.

Foreclosed: the entry cannot become the system's interface by accretion, which
is the failure mode this document exists to make visible rather than to prevent.
It cannot be prevented by a rule — only made expensive.

## What would reverse this

Any of three observations.

**A call turns out to be needed on a hot path.** That is not a reason to
optimise the entry; it is evidence that the ring is not carrying something it
should, and `docs/design/fast-path.html` is wrong about what the fast path
contains. Fix the ring.

**Channel setup itself turns out to be frequent.** The design assumes a process
sets its channels up once and then never crosses this boundary again. A workload
that sets channels up per request — a driver that binds a ring per submission,
say — would make the assumption false and the entry hot, and the answer would be
a way to hand over a channel *on* a ring rather than a faster door.

**The three calls are still here after M5.** Rule 2 says each has a successor.
If the ring lands and `ANNOUNCE` and `PROGRESS` have not been retired, then rule
2 does not work and the rule that replaces it has to be enforceable — most
likely a lint that fails the build on a call this document has not named.
