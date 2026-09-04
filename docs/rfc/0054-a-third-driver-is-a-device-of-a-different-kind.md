# RFC 0054: A third driver is a device of a different kind

- Status: accepted
- Date: 2026-09-04
- Affects: `user/virtio-gpu/`, `kernel/src/gpu.rs`,
  `kernel/src/arch/x86_64/virtio.rs`, `kernel/src/arch/x86_64/serial.rs`,
  `kernel/src/main.rs`, `xtask/src/main.rs`, `sim/src/gpu.rs`,
  `kernel/src/blk.rs` and `kernel/src/net.rs` (one call site each),
  `sim/src/chaos.rs` (named, not changed), `.github/workflows/ci.yml`,
  `docs/test-taxonomy.md` and `.toml`, RFC 0024, RFC 0033, RFC 0047, RFC 0051,
  `docs/manifest.md`, TODO.md E1-B04

## Decision

`user/virtio-gpu` is built to the shape `user/virtio-blk` established and
`user/virtio-net` confirmed — a component crate forbidding `unsafe`, a manifest
written before the driver, a scheduled ring-3 polling loop, registers reached
through RFC 0033's `Window`, device translations asked of the frame over RFC
0047's control-ring opcodes, and buffers owned by one side at a time under RFC
0024 — **and this RFC records what changed when the device stopped being a
pipe.** A block device and a network interface both move opaque bytes. A display
controller takes structured commands, answers every one of them with a typed
response, and owns a scanout: a standing output that outlives the request which
produced it, and that lives outside the machine.

The shape holds. Four of RFC 0051's five *did not carry over* items turn out to
have been about **receiving** rather than about being a second driver, and this
driver lands back on the block driver's shape. What is new is four things RFC
0051 could not have predicted, and the last of them changes what a check in this
tree is allowed to be evidence of.

## Context

E1-B02 produced a driver. E1-B03 produced a second and RFC 0051 counted the
difference, ending with a prediction: *a third driver that needs something
neither of these two needed turns one of the "carried over unchanged" rows into a
change, and the row is where to look first.* E1-B04 is that third sample, chosen
to be a device of a genuinely different kind rather than a third pipe.

Two alternatives were live and both were refused for RFC 0051's reason. A driver
that shared code with either of the first two is not a third sample. And a
display driver that *rendered* — a framebuffer console, a window system — would
have been a component with a picture of its own, which answers a question about
graphics rather than about drivers.

## What carried over unchanged, which is again the result

- **`ring/src/device.rs`.** Not one line. `Window` and `Region` were written for
  a block driver, drove a network card unchanged at E1-B03, and drive a display
  controller unchanged here.
- **`ring/src/registry.rs` and `ring/src/buffers.rs`.** The registration table,
  `Reach`, `Registered`, `Fixed`, `Idle` and `InFlight` are used verbatim — and
  they are doing something at this driver they were not asked to do before. See
  *the ownership interval* below: an `InFlight` here spans a **sequence of six
  device commands** rather than one chain, and nothing in `f_ring::buffers` had
  to change to express that.
- **`abi/`.** Nothing. The one place a reader would expect a change is the
  entry: this protocol needs a geometry, and `Sqe::ext`'s two free words carry
  it.
- **`kernel/src/iommu.rs`, `kernel/src/process.rs`'s `prepare_driver`,
  `f_abi::deadline::inherit`, `f_abi::control`'s two opcodes, `docs/manifest.md`
  and `cargo xtask lint-manifests`.** All unchanged.
- **The manifest.** Four register frames for the third time, which is now a
  statement about the *transport* rather than an assertion made from one device.
  Sixty-four kibibytes of untyped for the third time, of which this driver uses
  twelve — and `user/virtio-gpu/manifest.toml` argues that the figure is the
  frame's shape rather than any driver's, which is a thing only a third manifest
  can say.

## The refusal all three drivers rest on is now watched happening

`VIRTIO_F_ACCESS_PLATFORM`, bit 33. Without it a virtio device addresses physical
memory by specification, the emulator obliges, and every isolation test passes
for the wrong reason — which is what `kernel/src/arch/x86_64/dma.rs` records
having cost E1-B01 a rebuild to discover. All three drivers refuse a device that
does not offer it.

**Only this one has ever seen the refusal happen.** The emulator always offers
the bit, so no boot in this tree can reach that branch, and until now *the device
is refused without it* was a sentence in three module comments and a line nothing
had executed. `user/virtio-gpu/src/transport.rs` carries a fixture that is a
device's common configuration structure in an array — `Window` is a
bounds-checked accessor over an address and nothing in it requires the address to
be a device — so a test publishes the feature word a device *would* have
published and requires `Transport::open` to refuse it, and requires the driver to
accept exactly the two bits it asked for when offered everything.

What the fixture cannot reach is written beside it rather than papered over:
memory takes every write and gives it back, so the two refusals `open` reaches by
reading a register *back* — a status register that does not clear, a device that
clears `FEATURES_OK` — are unreachable this way and are not claimed. What would
cover them is a window backed by a model that answers, which is `sim/`'s business
and not a driver crate's.

## What did not carry over, and none of it is what RFC 0051 predicted

### One. The frame's device discovery, in the one line that assumed a twin

`virtio::route` took a *transitional* PCI device id as an ordinary argument and
refused a machine that had one, on the reasoning that a transitional virtio
device addresses physical memory by specification and is architecturally outside
the remapping unit. That reasoning is right and is unchanged. What was wrong is
the assumption underneath the signature: **the display controller has no
transitional device id at all.** The transitional ids are the sixteen numbers
from `0x1000` assigned by the original specification, and every device defined
after the modern transport — the display controller among them — has a modern id
and nothing else.

The parameter is an `Option<u16>` now, and the change is one line in each of
three callers. It is a *widening of a refusal into a choice*, which is the
direction R04 says to be careful in, so `None` is documented to mean *this device
has no transitional form* — a fact about the specification — and never *do not
check*. The constant beside each modern id is what makes a caller that turned the
check off visible in the call.

**This is the whole of what the frame's device discovery owed a third driver**,
and it is worth measuring against RFC 0051's prediction: that file expected a
third driver to break one of the *carried over unchanged* rows, and it broke the
one nobody would have picked.

### Two. The ownership interval is a pair of commands, not a chain

RFC 0051's one sentence for a third driver was: *find the point at which the
device owns the buffer, and treat every refusal below it as a bug.* On both other
drivers that point is a **queue** event — `Queue::offer`'s publishing store hands
the device a descriptor and the used element hands it back — so the interval is
one chain long and a driver can reason about it entirely in terms of its
virtqueue.

A display does not work that way. `RESOURCE_ATTACH_BACKING` gives the device a
guest address that it **keeps**: the chain carrying that command completes
immediately, and the display goes on holding the mapping until a later, separate
`RESOURCE_DETACH_BACKING` takes it away. The interval spans four more chains that
each complete successfully in between.

So a driver that reasoned about ownership in terms of its queue — which is what
both other drivers do, correctly — would hand a client its buffer back while a
display controller was still entitled to read it, and would do so with every
chain accounted for and every completion in hand. `Driver::sequence` is written
as one function for exactly this reason: the interval is a scope, not a comment,
and there is no `?` inside it that could return between the attach and the
detach.

**`RESOURCE_DETACH_BACKING` is therefore not an extra command, it is the one that
makes the completion honest.** E1-B04's task description lists five commands —
create, attach, set scanout, transfer, flush — and a driver that sent only those
five would show the right picture and would never give the client its memory
back.

### Three. A driver that must not reset its device

Both other drivers reset the device on their way out, and both are right to: a
device left with a queue address pointing at a frame somebody else now owns is
the corruption the whole subsystem is about. `user/virtio-net` states the sharper
version — a network card writes into posted receive buffers when a packet
arrives, which is a thing no code in this system decides.

A reset on a display controller destroys every resource it holds and replaces the
scanout with nothing. **It blanks the screen.** A display driver whose last act
is a reset throws away the one thing it was asked to produce, and the picture
that E1-B04's exit criterion is about would be gone before anything could look at
it.

So `user/virtio-gpu` has no `stop`, and two facts make that safe. Both are
properties of the *kind* of device rather than of this code, which is why they
could not have been found from two pipes:

- **A display controller does nothing until it is told.** Every transfer it
  performs is one the driver asked for on a doorbell it rang. Nothing arrives at
  a display.
- **The frame takes the access away, not the driver.** `kernel/src/gpu.rs` clears
  the bus-master bit and detaches the function from its domain before it frees
  anything, exactly as the other two datapaths do.

The reset a driver owes at the *start* is not skipped: `Transport::open` writes
zero to `DEVICE_STATUS` before anything else. That is where a restarted display
driver blanks the screen, which
`user/virtio-gpu/manifest.toml`'s restart section names as the honest first act
of one. There is a `Transport::reset` with exactly one caller — the halt path
above, where the alternative to blanking the screen is telling a client its
memory is its own while a display is reading it.

### Four, and it is the one that changes what a check may claim: the result is outside the machine

`cargo xtask blk` reads bytes out of the client's buffer. `cargo xtask net` reads
bytes out of the client's buffer and records out of the remapping unit. Both are
inside the machine, and in both the kernel reaches its own verdict — which is
this tree's standing rule, because a harness that second-guessed the kernel would
be a second implementation of the check.

**A scanout cannot be read back.** The 2D display protocol has no
`TRANSFER_FROM_HOST`, so nothing in this system — not the client, not the frame,
not the driver — can observe what is on the screen. Every counter
`f_virtio_gpu::driver::Counters` publishes is a statement about commands the
display *accepted*, and a display that accepted all six and drew nothing would
move every one of them.

So E1-B04's exit is met by an observation this tree has not made before:

- The boot publishes one number, `Report::display_hash` — an FNV-1a-64 digest of
  the client's own pixels, in the order a screen capture reports them — and then
  **holds the machine still**.
- `cargo xtask gpu` captures the emulator's framebuffer over the monitor socket
  while the machine is still running, hashes what it got, and requires the two to
  agree.
- The kernel is then told, by a byte on the serial port, that the capture has
  been taken, and carries on to its own exit code.

Three things make that a comparison rather than two sides agreeing with each
other. The harness holds **no copy of the pattern** and no copy of the pixel
layout — the kernel holds both — so a wrong belief on the kernel's side produces
a capture that does not match. The capture is deleted before it is taken, so a
stale file from a previous run cannot be read as this run's. And the control is
`gpu=blank`: the identical client with the submission removed, whose pixels sit
in guest memory for the whole boot and must not reach the screen.

`gpu=escape` is the sharper control and it was not designed to be. It sets the
scanout and flushes, so the capture is the same size as `inside`'s and differs
only in its contents — which is the one shape a capture check can fail in that a
size comparison would miss.

**What the capture does not prove, and what covers it.** A screen capture makes
the emulator refresh its own surface, and a scanout shares the resource's image —
so the capture shows the client's pixels whether or not `RESOURCE_FLUSH` was ever
sent. A driver that dropped the flush would produce an identical capture. What
catches that is a count and not a picture: `Report::verdict` requires the display
to have answered **exactly six** commands, so a command dropped from
`Driver::sequence` has precisely one check between it and a green run. The number
is not decoration and must not be relaxed into a range.

**What it costs, stated as a cost.** The kernel now reads a byte from a serial
port, waits up to a minute for it, and the harness now spawns and watches a boot
instead of spawning and waiting for one. That is a second way to run the emulator
in `xtask`, and the file it lives in says at length that there is one place the
machine is described; the machine description was therefore *extracted* into
`emulator()` rather than copied, so there is still one. What is not solved is
that a check in this tree can now depend on the harness being alive: a boot whose
harness died holds still for a minute and says so, which is the direction to be
wrong in but is a new direction.

## What the emulator answered, for the third time

`gpu=escape` points the device one page past what the client's registration
answered, in the backing entry a display reads a frame out of. The remapping unit
faults it — `requester 0x0008 read 0x00000000002c7000, reason 0x06`, a read fault
at the address the driver invented — and **the display answers `OK`**. The attach
succeeds, the transfer copies a buffer holding none of the client's bytes, the
flush puts it on the screen, and the device reports six successful commands.

The first version of this task's verdict required `declined` to have moved, on
the reasoning that a virtio-gpu command carries a *typed response* and a backing
the device cannot map should come back as a refusal. It went red.

This is the third time this tree has learned the same sentence and
`kernel/src/arch/x86_64/dma.rs` wrote it first: *a completion is evidence the
device finished and never evidence that bytes moved.* RFC 0051 records the
network driver finding it again from the other direction — a used entry with a
length for a receive the unit refused. The display's version is the most
convincing of the three, because what it answers is not a status byte or a
silence but a structured response saying the command succeeded.

What stands instead is the unit's own fault record, at the address the driver
invented and on the direction a display reads, and a capture taken outside the
machine. Neither is a device's word.

## Where the model and the driver agree, and the two places the model was wrong

`sim/src/gpu.rs` models a display controller and predates this driver. They agree
on the twenty-four-byte control header and where its flags and fence identifier
sit in it, on the two command numbers they both name — `RESOURCE_CREATE_2D` and
`TRANSFER_TO_HOST_2D`; the model does not send the other four — on
`RESP_OK_NODATA`, on zero not being a response any device sends, on a fenced
creation and an unfenced transfer, and — independently, for the same stated
reason — on **never sending `RESOURCE_UNREF`**. The model's
argument is that *a driver that freed as it went would never reach the display's
limit, and the limit is the refusal worth modelling*; the driver's is that a
resource is freed when the surface it draws is dropped and nothing in this system
owns a surface. Two files reaching the same choice by different routes is worth
more than either reaching it.

They disagreed in one place and the model was wrong: `RESP_ERR_OUT_OF_MEMORY` was
`0x1202`, which is `VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID` in the
specification's enumeration. The model was answering *no such scanout* to a
display that had run out of room. It is fixed, and the paragraph beside it
records why nothing in `sim/` could have caught it: the tests there assert against
the constant, so they agreed with the model whatever it said, and `harvest`
passes the device's number through unchanged — which is right, and which is also
why a wrong number travels the whole way.

They disagreed in a second, larger place, and the chaos scenario found it rather
than a reader. `sim/src/chaos.rs`'s `spawner` says what a respawn means — *the
same function, called again, with no state carried over from the instance that
died* — which is right for a driver and wrong for a device, and `Gpu` was the
only `Protocol` in that crate with state to notice: `Net` and `Blk` are unit
structs. So every kill destroyed the modelled display's resources and the
model's driver then transferred into them, producing refusals a client cannot
retry on a scenario whose whole claim is that a client observes nothing but
latency. **Adding a component whose ring protocol is `gpu` is what ran that model
in the deployment for the first time**; the table in `sim/src/deploy.rs` had
carried the row since before there was a display driver.

The fix is to split the one field in two: `Gpu::live` is what the *display*
holds and `Gpu::asked` is what the *driver* asked for, and the model's driver
now decides whether to create or to transfer from its own record rather than
from the sequence number. On an uninterrupted run the two are the same thing;
they come apart exactly where a driver restarts, which is where a real driver
also holds no identifiers and creates rather than transferring into something it
cannot name — which is what `user/virtio-gpu` does for every frame. Every one of
the model's five tests passes unchanged, including the one whose whole point is
that a transfer after a refused creation has nothing to copy into: a creation the
display refused is in `asked` and not in `live`, which is the other place the two
come apart.

What is *not* fixed is `spawner` itself. Making a device's state outlive its
driver is a change to that function's contract and to a gating claim's scenario,
and it belongs to whoever owns it; what is done here is the smaller, truer thing,
which is to stop the model's driver depending on state a restart destroys.

There is a third, smaller disagreement that is not a defect in either. The
model's whole reason for existing is that *some of a device's completions are
ordered and the rest are not*, and this driver cannot exercise it: it sends one
command at a time and waits, so there is never a later completion for an earlier
one to be overtaken by. The fence flag is set on the two commands the model
fences anyway, because a driver that pipelines needs it exactly there and a flag
added later is a flag added by somebody who has to rediscover the argument.

## What the frame owed a third driver that it did not owe the first two

**One page of stack, and this driver does not reach it.** RFC 0051 measured the
wall by hitting it: the network driver's posted-buffer table overran the frame's
one page of stack by fifty-six bytes at eight receive slots, and
`RECEIVE_SLOTS_STACK_BOUND` is the declared, unpaid deviation that records it.
This driver holds no per-buffer table at all — a display command is a request the
device answers, so nothing is outstanding between entries — and its `Driver` is
smaller than either of the other two. That is a **negative result worth
recording**: the wall belongs to the receive direction rather than to drivers,
and RFC 0051's nomination of E1-B04 as the natural owner of the fix was made on
the assumption that a third driver would measure it a third time. It did not need
to.

**A serial port that can be read.** `kernel/src/arch/x86_64/serial.rs` has
printed since M0 and never read. It reads now, once, polled, because a check
whose subject is outside the machine needs the machine to be told when the
outside has looked. R05 is not bent: it is a poll at a point the kernel chose,
not a delivery.

**And the merge RFC 0051 promised, which is declared and unpaid.** That RFC said:
*what would merge them is a third driver, at which point the shared half moves out
of both and neither is closed evidence any more.* There are three drivers now and
`kernel/src/gpu.rs` is `kernel/src/blk.rs`'s `Registers`, `Supervising`,
`Reported`, `declared` and `order_for` for the third time.

It is not merged, and the reason is not that the work is large. It is that the
merge rewrites `kernel/src/blk.rs` — the evidence E1-B02's exit rests on — inside
a task whose own evidence is a picture on a screen, so a defect introduced by the
merge would be found by the wrong check or by none. `OWED_REVERSALS` in xtask
carries it as a declared deviation naming `struct Supervising` in `blk.rs`, so
the day somebody does move it the build names every document that says it is
still there. **Read that row before deleting it**: emptying it because the work
*could* be done is the failure the constant exists to prevent.

The same is true, for the third time, of `routing::AT`: three crates hold one
address, the kernel carries three compile-time assertions, and the layout belongs
in `abi/`. The argument has not improved by being made a third time, which is
itself the argument for stopping making it.

## What this demonstration shows, and what it does not

`cargo xtask gpu` boots three halves. `inside` puts a sixteen-by-sixteen pattern
from a client's registered buffer onto scanout zero through one ring and six
display commands, and the harness finds those pixels on the emulator's own
framebuffer, byte for byte, with `copies = 0` beside a non-zero `provoked` and
the client's buffer returned unwritten. `blank` is the identical client with the
submission removed and the screen must not hold the pattern. `escape` points the
device one page past what the registration answered and requires a read fault at
the invented address and a screen that does not hold the pattern.

What it does not show: any refresh rate, any partial update, more than one client
drawing at once, what a display does when two resources want one scanout, a
second format, a cursor, or a resource ever being freed. It does not show that a
*person* would see the picture — what it shows is that the emulator's display
surface holds the client's bytes, which is as far outside the machine as this tree
currently reaches. And it says nothing about a real display: QEMU's virtio-gpu is
a model, and the host-side copy `TRANSFER_TO_HOST_2D` makes is a copy no counter
in this tree can see.

## What would reverse this

- **A fourth driver that is a fourth kind.** This RFC says the shape survives a
  device that answers structured commands. A device that *initiates* — an input
  device, a timer source, anything that raises an interrupt with nothing
  outstanding — is the next kind, and `user/virtio-net`'s `irq` need is where it
  starts.
- **A display driver with a client that keeps a surface.** The moment something
  in this system holds a display resource for longer than one frame,
  `RESOURCE_UNREF` has a lifetime to hang on, `RESOURCES_MAX` becomes the size of
  a table rather than the end of the road, and the ownership interval above
  becomes a lifetime rather than a scope.
- **The supervisor merge.** `OWED_REVERSALS` goes red on the diff that does it,
  and this section is one of the documents it will name.
- **A device model whose state outlives its driver.** `sim/src/chaos.rs`'s
  `spawner` rebuilds a peer from nothing on every refill, which is right for a
  component and wrong for the device below it. The split between `Gpu::live` and
  `Gpu::asked` makes the model's driver survive that; it does not make the
  *display* survive it. The first scenario that needs a device to remember
  something across its driver's death — a disk that is not `volatile` already
  needs it, and models it separately — is what turns `spawner`'s contract into
  the thing that changes.
- **A capture that can be taken from inside the machine.** `VIRTIO_GPU_F_VIRGL`
  or a blob resource gives the guest a route to what the host holds, at which
  point the kernel could reach its own verdict about the picture and the harness
  would stop being part of the check. That would be a strictly better shape and
  it is not available in the 2D protocol.
- **A second display in the machine.** `cargo xtask gpu` passes `-vga none` so
  that there is exactly one console and the capture cannot take the wrong one. A
  boot that legitimately wanted two displays makes that argument false, and the
  fix is naming the console in the capture — one more number to get wrong, which
  is why it is not done now.
