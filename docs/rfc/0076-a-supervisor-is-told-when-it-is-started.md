# RFC 0076: A supervisor is told when it is started, not while it runs

- Status: accepted
- Date: 2026-09-13
- Affects: `kernel/src/component.rs` (the join, `policy`), `user/supervisor/`,
  `abi/src/control.rs`'s pending-state argument, `xtask/src/main.rs`'s
  `OWED_REVERSALS` (RFC 0008's row), `docs/rfc/0008`, `TODO.md` E1-B05

## Decision

A supervisor learns that an occupant died **from notices already on its control
ring when it is started**, and not from notices delivered while it runs. The
frame publishes a place's `PEER_GONE` into the holders' tables at teardown, as
it already does, pumps it onto the supervisor's ring, as it already does, and
*then* schedules the supervisor. The supervisor drains its ring as its first
act, calls `policy::decide`, and submits `op::SPAWN` or `op::STOP` for what it
decided.

This is what makes RFC 0008's *restart is the supervisor's act* implementable
without any new shared state, and it is why `policy::decide` moves out of
`kernel/src/component.rs` in the increment that accepts this.

The cost, stated in the decision rather than below it: **a supervisor cannot
react to a death that happens during its own run.** It will be told on its next.

## Context

RFC 0073 put `op::SPAWN` and `op::STOP` on a second control-ring server, and
PR #50 showed `user/supervisor` submitting one from ring 3 and the frame
answering it. What did not move was the policy, and the reason discovered there
is the reason this RFC exists.

**A component is told things by the frame writing into its capability table.**
`abi/src/control.rs` is explicit: a notice is not a queued event but *pending
state the frame publishes when there is room*, and the three handle notices —
`GRANTED`, `REVOKED`, `PEER_GONE` — live in the capability slot they concern, as
a `Pending`, bounded by the slots the component has already paid for. That is a
good design and this RFC does not change it.

But while a component runs, its capability table is the live one in
`cap::of(cpu)`, on that core. `CLAUDE.md` permits two cores to reach one place
in exactly four, each a machine word with its ordering named at the access, and
says a fifth needs an argument. A capability table is not a machine word. So the
boot processor cannot write pending state into a running component's table, and
therefore cannot tell a running component anything.

PR #50 worked around this by serving the supervisor's ring only *after* its core
finished. That is sound and it is a dead end for the policy: a supervisor that
is never told cannot decide.

### The three alternatives that were live

**Move where a place-death pends.** Keep `PEER_GONE` for a place on the `Place`
itself — which the frame owns — rather than in the holder's capability slot.
`pump` would read it from there and post the same entry. No table write, so the
frame could tell a running component.

Refused, and the reason is `abi/src/control.rs`'s rule 2: *peer death does not
swallow the grant.* A handle granted and widowed before the drain posts
`GRANTED` and then `PEER_GONE`, **in that order**, because the granted notice is
the only place the need index is ever stated. Today that ordering is guaranteed
structurally — one `Pending` state machine, one slot, one sequence. Split the two
across two homes and the ordering becomes a property of `f_abi::control::ORDER`
instead: a convention a future phase can be inserted into wrongly, in place of a
state machine that cannot express the wrong order. Trading a structural
guarantee for a documented one, to buy a capability this RFC gets for free, is
the wrong direction.

**A fifth shared word.** One per core, meaning *this core is parked inside a door
call and is not touching its own capability table*, set and cleared by the door
handler on that core — which is the only mutator — and read `Acquire` by the
boot processor before it writes the table.

This is a real design and it would work. It is refused as **premature**: it buys
the ability to tell a running component, nothing in this tree needs that yet, and
it costs the exact thing `CLAUDE.md`'s rule is protecting — the property that the
frame's per-CPU discipline can be checked by counting four places. A fifth is
affordable when something cannot be done without it. Here something can.

**Move the lifecycle onto the running core.** Have the core running the
supervisor serve its own ring, so no core reaches another's table at all. This is
the right long-term shape and it is a different task: the places, the account,
the reservations and the frame allocator's use here all live in a stack frame on
the boot processor, and moving them is a change to what `component::demonstrate`
*is*, not to how a supervisor is told.

## Consequences

**What it makes easy.** `policy::decide` moves above the frame with no new
mechanism: the supervisor reads `PEER_GONE` off its ring, reads the cause the
frame packed, and decides. Nothing in `abi/src/control.rs` changes. No new
shared state. `CLAUDE.md`'s count of four stays four.

**What it makes true that was not.** A supervisor must hold the endpoint of each
place it supervises, because that is the slot a place's `PEER_GONE` pends in.
The frame therefore grants those endpoints into the supervisor's table alongside
the account — the powerbox grant PR #50 introduced, one entry wider — and names
them on the board. This is not a workaround: a supervisor that did not hold an
endpoint to a place could not stop it either, and `op::STOP` already requires
`REVOKE` on one.

**What it makes hard.** A supervisor is now a thing the frame *runs when it has
something to say*, rather than a daemon with a loop. Its per-place restart
`Budget` therefore has to outlive any one of its runs, and the honest place for
that is the `Place` — which means the frame holds the policy's *memory* while the
supervisor holds its *decision*. RFC 0008 objects to the frame deciding, and it
does not decide; but this is the seam where somebody will one day argue it does,
and the argument should be had rather than discovered.

**What it forecloses.** Nothing, and that is most of the case for it. The fifth
word and the per-core lifecycle both remain available, and each becomes cheaper
rather than harder once the policy is above the frame and the thing it needs is
precisely stated.

**What it does not claim.** This is not a general answer to *how does the frame
tell a running component something*. It is an answer to *how is a supervisor
told*, and it works because a supervisor is the one component whose useful input
arrives between its runs rather than during them. A driver's does not, which is
why a driver's ring is served while it runs and this one's is not.

## What would reverse this

**A supervisor that must act inside one run.** Concretely: an occupant dies
while the supervisor is on a core, and the restart has to be submitted before
that run ends rather than at the next one. Nothing does this today, because the
boot runs one occupant at a time; a machine running two supervised components on
two cores would, and that is E1-P06's *under sustained load* becoming true.

The observation that says it: a boot log in which a `PEER_GONE` is published for
a place while the supervisor is scheduled, so that the notice waits for a run
that has to be arranged rather than one that was going to happen. At that point
the fifth word or the per-core lifecycle is owed, and this RFC is superseded
rather than amended — the choice between those two is a different argument and
should not be smuggled in as a fix.

The weaker signal, worth watching first: a restart `Budget` that the frame has to
interpret rather than merely store. Storing it is the seam above; reading it is
the frame deciding, and that is RFC 0008 being violated by a data structure.
