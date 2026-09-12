# RFC 0073: A supervisor is a component, and the frame answers its ring from a second server

- Status: accepted
- Date: 2026-09-12
- Affects: `kernel/src/component.rs`, which gains the server and loses the
  policy; `kernel/src/supervisor.rs`, which does *not* gain two arms and whose
  module comment says why; `abi/src/control.rs`'s `op::SPAWN` and `op::STOP`,
  specified since RFC 0008 and unimplemented until now; `user/supervisor/`,
  which does not exist yet; RFC 0008 (restart is the supervisor's act), RFC 0037
  (a channel is adopted for a call, which removed the wall this was blocked on),
  RFC 0044 (`PLACES_MAX`, `SUPERVISOR_ORDER` and the account as one deviation),
  RFC 0047 (a driver is scheduled and asks the frame for a translation) and
  RFC 0071 (what merged between the three driver supervisors and what did not);
  `E1-B05`, `E1-P06`, `E2-B05`, `E2-B06`, `E2-B10` and `E2-P04`; and the
  declared quantities `OWED_REVERSALS`, `CHAOS_GAP` and `HEAP_GAP` in
  `xtask/src/main.rs`, all three of which this eventually turns red on purpose.
  No task line in `TODO.md` names this RFC; that is reported to the originator
  rather than fixed by editing that file.

## Decision

`f_abi::control::op::SPAWN` and `op::STOP` are answered by a **second
control-ring server in `kernel/src/component.rs`**, holding the places, the
account, the supervisor's table and the reservations — and *not* by two more
arms on `Supervising::execute` in `kernel/src/supervisor.rs`, which is the
frame's only control-ring server today and is device-shaped by construction. The
two servers stay apart, answer disjoint opcode sets, and share no `match`. We
are choosing a second server over a wider one, and the price is a second
`execute` with a second refusal path that has to be tested on its own.

## Context

RFC 0008 decided that restart is the supervisor's act and the frame provides
only the mechanism. The policy has been sitting in the frame ever since, held
there by a wall that no longer exists: a supervisor at ring 3 has to drive a
control ring, driving one meant adopting a mapped channel, `Mapping::adopt` is
`unsafe`, and a `user/` crate may not write `unsafe`. **RFC 0037 ended that**,
and RFC 0047 showed it working in both directions on a real component. Four
components adopt a channel in safe code today. `kernel/src/component.rs` has
carried the corrected paragraph for some time; `TODO.md`'s `E1-B05` line still
states the dead wall, which is how this came to be re-examined at all.

So the question stopped being *can a component drive a ring* and became *where
does the frame answer one*. There was one obvious answer and it is wrong.

`Supervising` (`kernel/src/supervisor.rs`) borrows a remapping `Unit`, a
`vtd::Domain`, a `FrameAllocator` and — the detail that settles it — **the
client's** capability table, deliberately, so that a driver asking for a
translation is asking about somebody else's capability and gets somebody else's
rights. Both of its opcodes go through one `iommu::Grant`. Its own module
comment says there is no version of it that lives above the frame.

A spawn touches none of that. It needs the places array, the account the frames
are charged to, the **supervisor's** table — the opposite table from the one
`Supervising` carries on purpose — and the `Reservations`;
`component::spawn`'s signature is the list. Widening `Supervising` to carry
places and an account, for two opcodes that never touch a device, would put the
capability-granting path and the device-translation path behind one `match` on
one struct that borrows both tables.

That is the merge RFC 0071 refused one file over. It measured the duplication
between three driver supervisors and moved what was byte-identical, and it left
`Reported` alone because three types sharing a name and a mechanism are not one
type. The same argument applies here with more force: these two servers do not
even share a mechanism.

### The alternatives that were live

**A fifth and sixth arm on `Supervising`.** Cheapest diff, and it is the one
this rejects. It makes the struct the union of two unrelated jobs and makes
every future driver opcode arrive in a type that can spawn components.

**A router in front of both.** One `execute` dispatching by opcode range to two
inner servers. Rejected as premature with exactly two servers: a router is worth
its indirection at three, and RFC 0071's discipline is that the third occurrence
is what earns a merge. It is named in *What would reverse this* rather than
built now.

**The supervisor answering its own opcodes at ring 3.** Refused, and it is worth
saying why it is not even close. A spawn charges frames out of an account and
writes a capability into a table; a component that could do that for itself
would be a component that could mint capabilities, which is the one thing the
frame exists to be the only holder of.

## Consequences

**Easy.** The refusal path can be tested before any success path exists — the
server is built answering `UNKNOWN_OPCODE` to everything first, with a test
against it, so that the refusal has a test written against it before there is a
success path to hide it. Each opcode then arrives with the boot's existing
evidence unchanged: six deliberate spawn refusals stay six, and a spawn that
refuses differently through the ring than through the direct call is a red
build rather than a discovery.

**Easy.** `policy::decide` moves rather than being rewritten. It was written
over a `&Record`, a `&mut Budget` and a tick with no kernel state at all,
precisely so that this day would be a move. `cargo xtask lint-owed` holds
`policy::decide(` as a declared quantity and goes red the day the call goes.

**Hard.** Two servers means two refusal paths, and a refusal that drifts between
them is a real hazard — a component would learn that the same malformed entry
means two different things depending on which ring it arrived on. Both answer
through `f_ring::refusal` with `error::pack`, and that is the only shared thing
they are permitted to grow.

**Foreclosed.** A single frame-side control server. If a third arrives, this
decision is wrong and the router is owed.

**What it makes visible.** Three declared quantities go red when the last
increment lands, and all three are good news: `HEAP_GAP`, whose needle is the
sentence *a component is spawned into a place and never scheduled*; `CHAOS_GAP`,
once a driver is scheduled inside the place its manifest is spawned into; and
three of the four `OWED_REVERSALS` rows. `HEAP_GAP` is deliberately the
acceptance test for the scheduling increment: the boot's `peak 0 byte(s)`
becomes non-zero the first time an occupant executes, and `cargo xtask run`
refuses that and says it is the good ending.

## What would reverse this

**A third control-ring server.** Two servers split by kind is a design; three is
a split by accident, and the third one arriving means the axis was opcode
routing all along. Build the router then, put all three behind it, and supersede
this.

**A supervisor opcode that needs a device.** If any opcode a supervisor submits
turns out to need a `Domain` or a `Unit` — a spawn that must place a component's
memory in a device's domain at creation, say — then the two servers do share a
mechanism and the separation is costing a borrow that has to be threaded through
both. That would be an argument for merging behind one struct after all.

**The refusal paths drifting.** If the two `execute`s are ever observed to
answer the same malformed entry with two different packed errors, the split has
started costing correctness rather than clarity, and the shared refusal helper
is not enough. Measurable: a test that feeds one malformed `Sqe` to both servers
and requires the same `error::pack` for anything that is not genuinely opcode-
specific.
