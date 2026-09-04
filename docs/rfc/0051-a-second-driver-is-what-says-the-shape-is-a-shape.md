# RFC 0051: A second driver is what says the shape is a shape

- Status: accepted
- Date: 2026-09-04
- Affects: `user/virtio-net/`, `kernel/src/net.rs`, `kernel/src/arch/x86_64/virtio.rs`,
  `kernel/src/process.rs` (named, not changed), `ring/src/buffers.rs` (named, not
  changed), `sim/src/net.rs`, `xtask/src/main.rs`, RFC 0024, RFC 0033, RFC 0047,
  `docs/manifest.md`, TODO.md E1-B03 and E1-B04

## Decision

`user/virtio-net` is built to the shape `user/virtio-blk` established — a
component crate forbidding `unsafe`, a manifest written before the driver, a
scheduled ring-3 polling loop, registers reached through RFC 0033's `Window`,
device translations asked of the frame over RFC 0047's control-ring opcodes, and
buffers owned by one side at a time under RFC 0024 — **and this RFC records where
following it worked, where it did not, and what the frame turned out to owe a
second driver.** The shape holds. Five things do not carry over, all of them on
the receive direction, and one of them is a hole in a client's types that one
driver could not have found. Nothing in `abi/`, `ring/` or the frame's device
discovery was changed to make this driver exist, and that is the strongest single
result here.

## Context

E1-B02 produced a driver and, with RFC 0033 and RFC 0047, an argument that a
component can drive real hardware with no `unsafe` and copy nothing. One sample
cannot say whether the argument is about *drivers* or about *the block driver*:
every general claim in those RFCs was written from one example, and a general
claim written from one example is a description of that example. E1-B03 exists to
be the second sample.

The alternatives that were live at the start of it were the ordinary ones — a
driver that shared code with the block driver, a driver that shared a supervisor
with it, a `ring/` extended to hold what both need — and every one of them would
have destroyed the evidence by construction. A second driver that reuses the
first driver's code is not a second sample.

## What carried over unchanged, which is the result

- **`ring/src/device.rs`.** Not one line was added. `Window` and `Region` were
  written for a block driver and drive a network card with no change: four
  register structures, two virtqueues, descriptors, a `Release` publish and an
  `Acquire` consume. RFC 0033 asked for exactly this evidence and could not
  supply it from one driver.
- **`ring/src/registry.rs` and `ring/src/buffers.rs`.** The registration table,
  `Reach`, `Registered`, `Fixed`, `Idle` and `InFlight` are used verbatim on both
  directions of a protocol they were not designed against.
- **`kernel/src/arch/x86_64/virtio.rs`.** Two constants, no code. That module was
  written parameterised by device id, on an argument that named E1-B03 as the
  caller who would need it, and the argument was right.
- **`kernel/src/iommu.rs`, `kernel/src/process.rs`'s `prepare_driver`,
  `f_abi::deadline::inherit`, `f_abi::control`'s two opcodes, `docs/manifest.md`
  and `cargo xtask lint-manifests`.** All unchanged.
- **The manifest.** `user/virtio-net/manifest.toml` declares the same four
  register frames, the same sixty-four kibibytes of untyped, the same interrupt
  and powerbox, and reaches `private` by a different argument. The four register
  frames turned out to be a property of the *transport* rather than of the block
  device, which is a thing the first manifest could only assert.

## What did not carry over, and why each one is the receive direction

**One. An executor that answers where it reads.** The block driver's signature is
entry in, `Cqe` out, and every entry it accepts is a request a device owes an
answer to. A receive is accepted now and answered when a frame arrives — which
may be never — so `Driver::execute` answers `Answered::Later` and
`Driver::collect` produces the completion. A signature that could not say
*accepted, answer to follow* would have forced the driver to block inside its
executor waiting for a packet nobody promised, which is a service that stops
serving because nothing was sent to it.

**Two. A used element has to be read for its head.** One chain outstanding means
the head is a constant; a receive queue holds every posted buffer at once and the
head is the only thing that says which one filled. `Queue::harvest` therefore
answers a head as well as a length, and the head is a *device's word*: the driver
refuses one that does not name a slot it posted, because a driver that indexed
its own bookkeeping with it is a driver a device can steer into releasing a
buffer it is still writing.

**Three. A wait with no answer owed needs a bound that is told.** Every bound in
the block driver waits for something a device owes. Nothing owes a driver a
packet, so a driver with no interrupt and no bound is a driver that hangs, and
calling the hang *waiting for traffic* is exactly the failure RFC 0046 names.
`routing::at::RECEIVE_SPINS` is that bound, told by the frame because how long to
wait for a network is a property of the machine, and `Counters::spun` publishes
how much of it was spent so the bound and its use are read together.

**Four, and it is the one with content: a service now owes its clients their
buffers back at teardown.** RFC 0024 gives an in-flight buffer exactly three
exits — a completion carrying its token, `reclaim` on evidence the peer is gone,
and a drop that ends the component. A posted receive that no frame ever fills is
none of the three: the peer is alive and healthy and simply has nothing to give,
and `PeerGone` cannot be constructed from *the service stopped politely*. So
`Driver::cancel` gives every posted buffer back with `cflags::CANCELLED` after the
device is in reset, and `cargo xtask net silent` requires it to have happened.
Nothing in RFC 0024 states that obligation, because no service that answers every
entry it accepts has it.

**Five, and review found it rather than the writing: on this direction a refusal
is not free.** The block driver may refuse an entry at any point in serving it,
because it holds the client's buffer for the whole of one function call and gives
it back either way. A receive hands the buffer to the device partway through:
`Queue::offer`'s publishing store is the moment the device owns it, and from
there a refusal is *not a smaller answer than a completion, it is a wrong one* —
a refusal reaches the client's `InFlight::complete`, which hands the buffer back
as an `Idle` the client may write, while a network card holds a device-write
descriptor pointing into it and nothing in this system decides when it writes.

The first version of this driver had three refusals below that line, and the
sharpest was the doorbell: `Transport::kick` computes its offset from
`QUEUE_NOTIFY_OFF * notify_multiplier`, two words the *device* published, so a
device describing itself inconsistently could decide that a client got a buffer
back. The fix has two halves and both are general. Everything fallible moves
above the offer — the name is narrowed, the slot is found, both derived offsets
are computed — so that nothing below it can fail. And a failure that happens
below it anyway is answered by *stopping* rather than by refusing:
`Driver::stopped` — asked once a turn by the component's loop, which then ends —
and the buffer comes back through `quiesce` and `cancel` like every other posted
receive, which is the exit **Four** above had to invent and which now has a
second user. `Counters::halted` is the number behind it, published and required
to be zero on every half, because a mechanism a boot cannot see is a mechanism a
boot cannot tell from an absence.

That flag has a cost worth recording beside the stack paragraph below, because it
is the same wall. It was an `Answered` variant first and a `bool` on `Driver`
second, and **each of those cost eight bytes of the one page and each faulted the
guard**, at `0x0000000000410ff8` both times. It is a `u32` in padding
`Counters` was already paying for, which fits. A driver whose ownership
discipline is bounded by its stack is the sharpest form the frame's debt has
taken so far.

The transmit direction has the milder version of the same thing and the same fix:
a chain the device may still be reading is a client that must not be told it owns
its buffer again, so a failed `round_trip` puts the device in reset before the
registration is released. What a third driver should take from this is one
sentence: **find the point at which the device owns the buffer, and treat every
refusal below it as a bug.** Nothing in the ABI marks that point, and nothing in
`ring/` can — it is a property of the queue discipline a driver chose.

## The gap in RFC 0024's typestate that a second driver found

RFC 0024's rule is *a buffer is held by one side at a time, and the compiler says
which*. In the receive direction that rule holds unchanged and is doing more work
than it ever did for the block driver: an `InFlight` has no method that reaches
its bytes, so a client cannot read a buffer a **network card** may write into at
any moment until a frame arrives. That is the clause E1-B03's exit is about and
it is expressed by the types, not by care.

What the types do not express is **how much of the returned buffer is valid**.
`InFlight::complete` hands back an `Idle` whose `bytes()` is the whole buffer; a
received frame occupies a prefix of it, and the length is in the completion's
`result`. Nothing requires a client to read it. A client that treats
`idle.bytes()` as *the frame* reads its own stale bytes past the frame's end and
cannot tell them from received data.

**This could not appear on the block driver**, and the reason is worth stating
rather than dismissing: a block read's length is the *request's*, chosen by the
client, so buffer length and valid length coincide by construction. Receive is
the first direction in this system where the **device** chooses the length.

It is not worked around in the driver, because a driver cannot patch a hole in a
client's types. It belongs on RFC 0024's own list of misuses that are neither a
compile error nor a wire refusal — beside the dropped `InFlight` and the token
lent to two buffers — and it is a third entry for that list. The shape of a fix
is a state between `InFlight` and `Idle` that carries the length the completion
reported and hands out only that prefix; the cost of it is a fourth state on a
type whose whole value is that it has few, and the reason it is not done here is
that E1-B03 has one client and a type change with one user is a type change with
no evidence.

## The second gap: a registration has no direction

A registration is a whole set; a direction is a property of one operation. So the
translation a driver asks the frame for is read-write for every buffer in a set,
whichever way any one buffer is used, and **a set a client only ever transmits
from is nonetheless writable by the device**. On the block driver this was
invisible: the same set is read and written by the same requests. On a network
driver transmit and receive are structurally different queues, and a client that
wanted its transmit buffers device-read-only has no field to say so —
`f_abi::buf::opcode::REGISTER` carries a capability, a length and a buffer count,
and `iommu::Grant::map` derives writability from the capability's own `WRITE`
right rather than from what the buffers are for.

Recorded rather than fixed, because the fix is an ABI change: either a rights
field on the registration entry, or the convention that a client registers two
sets and holds the transmit one through a read-only `Frame` capability. The
second needs no ABI change at all and is available today, which is the honest
reason this is a gap and not a defect: nothing prevents a careful client, and
nothing makes a careless one safe.

## What the frame owed a second driver, measured

**One page of stack, and it is not enough.** `kernel::process::SPAWN_STACK` maps a
scheduled driver four kibibytes with a guard page below it. A component has no
allocator, so a driver's registration table, its posted-buffer array, its
transport, its queues and its control region all live in that page — in `Driver`,
on the stack of `component::serve`, while `Driver::start` is still building one.
At eight receive slots and sixteen registration sets this driver's deepest frame
overran the page by **fifty-six bytes**: a page fault at the guard, `vector 14,
error 0x6, address 0x0000000000410fc8`, observed rather than reasoned about. Four
slots fit.

So `RECEIVE_SLOTS_STACK_BOUND` is a constant in a driver crate that is a bound on
the *frame*, and `cargo xtask lint-owed` carries it as a declared unpaid
deviation so that the day the frame gives a driver a stack, the build says which
documents describe a wall that is gone. The number that matters beside it is the
one nobody had: `user/virtio-blk` was already close to the same wall and nothing
had measured it.

The fix is not taken here because it is larger than it looks. Growing the stack
moves `SPAWN_STACK_TOP`, which moves `SPAWN_CONTROL`, which moves
`kernel::process::BLK_BOARD` — *the one address a driver holds as a constant* —
in two crates that cannot see each other, and it changes the stack every other
component shape is given. That is `kernel/src/process.rs` and
`kernel/src/component.rs` work, with `user/virtio-blk`'s closed evidence
underneath it, and it belongs to whoever next touches the driver shape. E1-B04 is
the natural owner: a third driver makes the same measurement a third time.

**The supervisor, entire.** `kernel/src/net.rs` is `kernel/src/blk.rs`'s
`Registers`, `Supervising`, `Reported`, `declared` and `order_for`, adapted only
in which manifest name they look for and which counters they read. That is the
largest duplication this task produced and it is deliberate for the reason
`kernel/src/arch/x86_64/virtio.rs` gives about `dma.rs`: `blk.rs` is the evidence
a closed task's exit rests on, and refactoring it to share a supervisor with a
later task changes closed evidence for the convenience of the later task. *What
would merge them is a third driver*, at which point the shared half moves out of
both and neither is closed evidence any more.

**Two constants that should be one address.** `f_virtio_blk::routing::AT` and
`f_virtio_net::routing::AT` are the same number, because the frame builds one
driver shape and every driver in it finds its board at the same address. Each
crate holds it independently and `kernel/src/blk.rs` and `kernel/src/net.rs` each
carry a compile-time assertion, because the kernel is the one artefact that links
every definition. A third driver adds a third assertion. The layout belongs in
`abi/`, where a wire layout two crates agree on belongs; it is not moved here for
the same reason the supervisor is not.

**What the frame did *not* owe.** No second device window mechanism: `virtio::route`
was already parameterised. No second requester-id machinery: `pci::Survey::find`
distinguishes two virtio functions by device id. No second IOMMU domain
mechanism: `Unit::domain` hands out a domain per call and the network datapath
takes one exactly as the block datapath does. What is *not* shown is two domains
**live at once**, because only one driver runs per boot — that is a real gap and
it belongs to whoever first schedules two drivers together.

## Where the model and the device disagree

`sim/src/net.rs` models a virtio-net device, and this driver was written to agree
with it. They agree on everything the model covers: the twelve-byte modern header
with `num_buffers` regardless of `MRG_RXBUF`, `GSO_NONE` with every header field
zero, both transmit descriptors device-read, a sixteen-byte control slot per
header for alignment, and — the model's whole reason for existing — a transmit
used entry that carries no status, so the driver reports what it handed over and
never what the device said.

They disagree in one place, and the model predicted the disagreement in the wrong
shape. Its reversal reads: *the first component that receives rather than
transmits, which is E2's network stack. What changes then is the client, not this
file: `serve` gains a receive queue and the driver gains a path that posts buffers
with no outstanding token.* Three things about that are now known:

- It has fallen due at E1-B03 rather than at E2.
- The client did **not** change. `f_ring::buffers` was used unmodified.
- A receive is posted **with** an outstanding token, not without one. The ring
  protocol stays request-and-answer, and RFC 0024's typestate requires the token:
  a tokenless post leaves the client holding an `InFlight` that no completion can
  ever return.

The model is not extended here — a receive queue in it is a change to the
simulator's device model, its fault classes and its snapshot tags, and it wants
its own task — but its reversal paragraph is corrected, because a reversal
condition that has fallen due and still describes the future is the exact rot
RFC 0036 and `lint-owed` exist to prevent.

## What the demonstration shows, and what it does not

`cargo xtask net` boots three halves. `inside` posts a receive buffer, transmits a
hand-formed address-resolution request, and requires the reply to land in the
registered buffer with its **target hardware address equal to the one this boot
invented** — which the host's backend could not have produced without the request.
`silent` is the identical client with the transmit removed and requires nothing to
land and the posted buffer to come back as a cancellation. `escape` transmits and
has the driver point the device past what the registration answered on the
**receive** descriptor, and requires the remapping unit to fault a **write** at
the address the driver invented while the client's buffer keeps its poison.

What it does not show: throughput, many buffers in flight, a frame larger than the
one it sends, behaviour on a busy link, or anything about a real network — the
backend is QEMU's user-mode stack with `restrict=on`, so nothing this boot sends
leaves the host. It also cannot show that the transmit was *delivered*:
virtio-net answers a transmit with no status at all, so the only evidence here
that a frame left the machine is that something outside it answered.

One thing the `escape` half found is worth keeping: **the emulator publishes a
used entry with a length for a receive the remapping unit refused.**
`kernel/src/arch/x86_64/dma.rs` recorded the same about the block device — *a
completion is evidence the device finished and never evidence that bytes moved* —
and the first version of this task's verdict asserted the opposite and went red.
The check that stands is the client's own memory and the unit's fault record at
the address the driver invented, and neither of those is a device's word.

## What would reverse this

- **A third driver that does not fit.** This RFC says the shape is a shape on two
  samples. E1-B04 is the third, and a driver that needs something neither of these
  two needed — a second live IOMMU domain, an interrupt rather than a poll, a
  device whose register window is not four pages — turns one of the *carried over
  unchanged* rows above into a change, and the row is where to look first.
- **The frame giving a driver a stack.** `RECEIVE_SLOTS_STACK_BOUND` goes,
  `lint-owed` goes red, and the number in this driver becomes a protocol decision
  instead of a frame one.
- **A client that reads past a received frame.** The gap in RFC 0024's typestate
  is recorded here on the argument that a type change with one user has no
  evidence behind it. The first client that gets it wrong is that evidence, and
  the fix is a state between `InFlight` and `Idle` carrying the reported length.
- **A registration that carries a direction.** If a client is ever shown reading
  its own transmit buffer back and finding a device's bytes in it, the second gap
  above stops being a note and becomes an ABI change.
- **The simulator growing a receive queue.** At that point the model and this
  driver are two implementations of one protocol rather than one and a half, and
  the disagreement section above is settled by a test rather than by a paragraph.
