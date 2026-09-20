# RFC 0097: A paused occupant keeps its core, because it keeps its stack

- Status: accepted
- Date: 2026-09-20
- Affects: `user/virtio-blk/src/component.rs` (`hand_over`),
  `user/virtio-blk/src/routing.rs`, `kernel/src/component.rs`
  (`demonstrate`, `serve_ring3`), `kernel/src/smp.rs`, `xtask/src/main.rs`
  (`-smp`); RFC 0016, RFC 0041, RFC 0063; and `E2-B06`, `E2-P08`

## Decision

RFC 0063 says a swap that fails in phase A abandons: *the place resumes
delivering to the occupant it already had, and the client observes added
latency and nothing else.* The frame reverses correctly — `cargo xtask blk
abandoned` shows the place taking back its occupant and the successor being
retired — and the client is now untouched, because the frame refuses to drive
one against a successor whose replay did not match. **What it cannot do is the
first half of that sentence.** The occupant it puts back has exited.

`f_virtio_blk::component::hand_over` returns `stopped::HANDED_OVER` and the
component's loop returns. So the frame keeps the instance's memory, its address
space, its capability table and its client's registrations, and has nothing
that can serve out of them.

**The repair is that the outgoing occupant pauses instead of ending, and a
paused occupant keeps its core.** It keeps its core because it keeps its
*stack*: `f_virtio_blk::driver::Driver` — which owns the
`f_ring::registry::Table` the client's `SetId`s name — is a local inside
`serve()`. Its lifetime is the call's. A component that returned and was
re-entered would rebuild that table empty, which is a restart wearing a
resume's name and is exactly what this decision exists to avoid.

So a swap needs **two application cores**, not one: the outgoing occupant holds
its core parked while the successor runs on another.

## Why not re-entry, which is the tempting alternative

Because it moves the client's registrations somewhere they can survive a
returned stack, and everywhere they could go is worse:

- **Into the component's heap.** `f_ring::heap::Heap` is at a fixed address and
  does survive, but the component would have to find an initialised heap on
  entry and adopt it rather than format it. A component that adopts memory it
  did not initialise is a component that believes a page — which is the
  `f_ring::adopt` problem RFC 0037 already has a protocol for, applied to a
  component's own state rather than to a peer's ring, and with no peer to
  blame when it is wrong.
- **Into the frame.** The frame would hold a component's registration table,
  which is the thing RFC 0008 spent a decision moving *out* of the frame.

Re-entry also makes the resume a different code path from the serve, and a
resume that runs code the serve does not is a resume whose correctness is not
the serve's.

## What it costs, stated rather than implied

**A second application core, on the boots that swap.** `-smp 2` is pinned in
`xtask` for the same reason the memory size is, and this does not unpin it:
the swap halves ask for one more, and every other boot is unchanged. A machine
with one application core cannot abandon a swap without a restart, and that is
a real limitation of a real deployment rather than a limitation of this
harness — a single-core machine should declare `restart_only`.

**A core held by a component that is doing nothing.** Between the hand-over and
the commit, one core is parked in a component's own loop. RFC 0041 prices a
place's reservation in cores and pages; this says a place being swapped costs
one more core for the length of phase A. It is bounded by the same deadline
every other wait in the frame is bounded by.

**A component that can be told two things at a quiescent point.** `hand_over`
currently answers one question — *are you being asked to hand over* — and now
has to wait for a second: *resume, or stop*. That is one more frame-written
word and one more poll at a point the component is already polling, and
`user/virtio-blk/manifest.toml`'s `[transfer]` table already names that point.

## Why the frame and not the supervisor

The parked core belongs to the *place*, and RFC 0041 already puts a place's
reservation in the frame. A supervisor deciding which core a paused occupant
holds would be a supervisor scheduling, which RFC 0008 says it does not do: it
decides *whether* a place is refilled, not *where* an occupant runs.

## What would reverse this

**A machine whose application cores are scarcer than its places.** This spends
a core per in-flight swap. One swap at a time per place is `f_abi::swap`'s own
bound, but a machine swapping several places at once spends several, and at
that point the honest answer is to serialise swaps machine-wide and say so —
which is a scheduling policy and belongs in the supervisor, not here.

**A component whose state is small enough to hand over whole.** The reason the
outgoing instance must stay alive is that its table cannot be reconstructed. A
component that could write its entire live state into the transfer window would
not need to be resumed at all, because the abandonment could rebuild it from
the window. `user/virtio-blk`'s journal is already most of the way there — it
is a history of deeds — and the day a component's journal is provably complete,
the pause is unnecessary for that component and `restart_only` stops being the
only alternative.

**`E2-P08` observing a paused occupant that cannot resume.** The whole claim
here is that a component which never leaves `serve()` still has everything it
needs. If a run shows a resumed occupant answering wrongly — a stale cursor, a
device that moved on, a queue the transport no longer agrees about — then
pausing is not free and the parked core was buying less than it cost.
