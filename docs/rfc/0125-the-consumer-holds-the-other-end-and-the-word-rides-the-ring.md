# RFC 0125: The consumer holds the other end, and the producer's word rides the ring

- Status: accepted
- Date: 2026-09-24
- Affects: `abi/src/input.rs` (`ATTEST`, `Attested`, `Crossing::attestation`,
  `Crossing::attested`, `Crossing::agrees_with_attested`),
  `user/virtio-input/src/{driver.rs,component.rs,routing.rs}`,
  `user/compositor/src/{inbound.rs,component.rs,latch.rs,tree.rs,routing.rs}`,
  `kernel/src/input.rs`, `kernel/src/compositor.rs`, `kernel/src/main.rs`'s
  input stage text, `xtask`'s `input` verb and `NOT_THE_FRAME`,
  `TODO.md`'s `E3-B04g`, `E3-B01i` and `E3-B04e`; pays the fourth reversal of
  RFC 0124

## Decision

On `cargo xtask input deliver` the **compositor holds the server end of the
input driver's data channel** and drains it itself: it decodes every entry with
`f_abi::input::Event::decode`, rebuilds the reading from the decoded event's own
field, folds what it drained into an `f_abi::input::Crossing`, and compares that
fold against the driver's. The frame lays the channel out and hands one end to
each component, and has no line that takes an entry off it.

Three things follow and are decided here rather than left implicit.

1. **The driver's word crosses on the ring its events crossed**, after the last
   event, as an entry under a new opcode, `f_abi::input::ATTEST` (`0x80`),
   outside the event space. It also stays on the driver's routing page, where
   `E3-B04f` put it, and the frame requires the compositor's fold to equal *that*
   copy — so the word reaches the check by two routes and a ring that carried it
   wrongly is a disagreement between them.
2. **On one worker core the ring is the buffer.** The two components still run
   one after the other; the driver submits onto a ring nobody drains, and the
   compositor drains it afterwards. The ring's sixteen entries are therefore the
   bound on a gesture, and the driver's `dropped` count is what a longer one
   produces — which the verdict refuses.
3. **The channel reaches the compositor through `process::ServerPlan::buffers`**,
   the one caller-held region that shape maps writable into a server. The
   compositor's manifest declares `payload = "inline"`, so that region was
   otherwise unused.

## Context

RFC 0124 wrote down what a consumer that was not the frame had to do, in four
steps, and named this as its fourth reversal: *the compositor draining the ring
itself … on that day the frame stops being the consumer, its fold moves to the
component*. `E3-B01i`'s boot half had been waiting on exactly that since the
same day, with a verdict clause written to go red on it.

Step 4 of RFC 0124 left one thing open: the driver's word *has to reach [the
consumer] by some route other than the driver's routing page — the frame takes
that page back when the driver is reaped. A ring or the driver's state tree.*

Four routes were live.

**The driver's state tree.** The route RFC 0124 said a reader would look in, and
not taken. A driver shape gets no tree of its own today — `DriverPlan::tree` is
the frame's tree, granted read-only — so the route needs `prepare_driver` to
map one, and on one worker core that page has to outlive the driver's reap for
the compositor to read it afterwards, which is a page the frame allocates and
maps into two components. That is the ring's arrangement with a second page and
a schema in it, and it is a change to `kernel/src/process.rs`'s driver shape
that this line did not need.

**The frame copies the word across.** It reads the driver's page before the
reap and writes the word onto the compositor's. Refused: the frame is then the
courier for the one number the consumer checks everything else against, which
is the arrangement `E3-B04g` exists to end, with a checksum in place of two
coordinates.

**A seventh event.** Refused, because an attestation is not something the
device did. It has no reading and must not look like it has one; putting it in
the event space would make `Event::decode` accept it and `Crossing::absorb`
fold it into the crossing it attests to.

**An entry outside the event space, on the ring.** Taken. The ring is the one
thing both ends hold for as long as either needs it, the entry arrives in order
after everything it covers, and the decoder already refuses an opcode it does
not know — so a consumer asks `Crossing::attested` first and hands the rest to
the decoder, and a malformed attestation is simply an entry that did not decode.
`Crossing::attested` compares all sixty-four bytes against the entry
`Crossing::attestation` would have written, which is `Event::decode`'s own rule
for the envelope.

The other open question was the core count. `E3-B04g`'s record said it needed
`E1-B05` *or a second worker core*, so that the two components could run at
once. Neither is needed for the exit as written: nothing in it says *at once*,
and the ring that carries the entries can hold them between the two runs. What
running at once would add is a latency, and every number on this path is
virtual time out of a seed — `user/virtio-input/src/clock.rs` — so it would add
nothing measurable either.

## Consequences

**What it makes easy.** The position the compositor's latch reads came off a
device by a route the frame never touched, and that is held three ways rather
than asserted once. Structurally: `kernel/src/input.rs` has no consumer of the
input ring, no decoder call, and no buffer an event could be kept in. By a lint:
`xtask`'s `NOT_THE_FRAME` gains a row refusing `Event::decode(` under `kernel/`,
whose third field requires `user/compositor/` to call it — so the day nothing
is the consumer is red as well as the day the frame is. And by a measurement:
the boot reads the ring's own cursors between the driver's reap and the
compositor's start and requires every entry the driver put there to still be
there. That last one exists because the first version of this boot printed
*this frame took 0 entries* as a literal, and a mutation that had the frame
take one went red on a sentence that said it had taken none.

`E3-B01i`'s boot half rests on it the same day: a report taken, a latch at
entry zero of the frame's trace, and *latched minus committed* checked by the
harness against its own motion list. Its *and by nothing else* is carried into
the boot by a different instrument from the host tests', and that is said here
rather than left to be noticed. The host tests encode the frame twice and
compare bytes; a running compositor cannot, because the encoder's buffer is
`f_scene::encode::ENCODING_MAX` — 83 992 bytes — and its heap is its graph and
its batch with 6 528 bytes to spare. (This paragraph said *two and a half
kilobytes* until the number was measured on 2026-09-25: `HEAP_BYTES` 143 360,
less `HELD_BYTES` 136 768, less `HEAP_OVERHEAD` 64. The argument does not move —
the buffer is nearly thirteen times the room — but a number in an RFC is a number
somebody will cite.) So the latch folds every field
of every node the encoding carries, walked in the encoding's order, with the
pointer's two translations masked, either side of the patch
(`f_compositor::latch::unmoved`), and the frame requires the two words to be
one and the walk to have covered the whole graph. It is a second implementation
of the encoding's walk; a host test changes every field it folds and requires
every change but the two masked ones to move it, which is what keeps the two
lists in step. The control is `E3-B04d`'s shape: the identical boot with the
ring not connected, in which the compositor declines every frame.

**The boot's scene is the host fixture's, and it was not at first.** The
version of this boot written with the rest of this RFC committed the pointer as
the identity with a translation and had no other transform in the scene. Over
that scene a latch that wrote a fresh identity matrix around the translation —
the defect `f_compositor::latch` names at the patch, *a compositor deciding a
client's scale* — submits the same frame as the correct latch, and the fold
cannot see what is not different: that mutation was run and the boot went
**green**, `nothing else moved`. And a latch that patched every transform it
found is indistinguishable from one that patched the pointer when the pointer is
the only transform. So the boot now commits what the host tests commit: the
pointer as a shear with a scale (2, 17 000/65 536, −9 000/65 536, 3) at the
origin, and a sibling transform node, never moved, beside it. Both defects then
go red on the fold, with the counts and the harness's equality unchanged. The
origin stays the origin, because the client is never told a position and
*latched minus committed* has to be the harness's own sum.

**The three words on the compositor's routing page that carried a pointer are
retired.** `at::POINTER_AT_NANOS`, `POINTER_X_X65536` and `POINTER_Y_X65536`
were written by no frame ever — a frame writing a stamp into that page would be a
frame holding a reading — and the component read them every turn. Their offsets
(136, 144, 152) are not reused.

**What it makes hard.** A gesture longer than fifteen entries cannot cross on
this boot, and fails rather than truncating. The repair is the components
running at once, which is `E1-B05`; a larger ring is a larger bound and not a
repair.

**The compositor no longer parks while it holds an input ring.** The driver
submits unasked and rings nobody, so a compositor asleep on the scene ring's
doorbell would take the next position only when a client happened to submit.
No boot connects both today — `compositor=wake` connects no input ring and
`input=deliver` writes no doorbell — and the rule is in the component so that
the first boot that does is not a latch aimed at a stale pointer.

**The seam `E3-B04e` describes is not in a boot.** The motion `cargo xtask
input` injects reverses inside the predictor's window, so the latch *holds* the
newest position rather than extrapolating. That is what makes `E3-B01i`'s
*exactly* an equality, and it is also what makes the two-predictor comparison
vacuous in this boot — a held prediction on both sides agrees by copying one
sample. What the boot does give `E3-B04e` is the relay: the reports its latch
predicts from are fold-attested to be the reports the driver sent, in order. The
extrapolated half needs a second predictor instance on the input path's side of
the ring asked about the instant the compositor aims at, and nothing on that
side of the ring learns that instant today.

**What it forecloses.** Nothing about the frame's position on `INPUT_PATH`:
`kernel/` reads two positions and three folds and names no reading, and the day
it does it takes a row and goes red on the one `rdtsc` in this tree, exactly as
RFC 0124 left it.

## What would reverse this

- **The two components running at once** — `E1-B05`'s ring-3 supervisor handing
  a place's occupant a core and a peer, or a second worker core. Then the ring
  stops being the buffer, the gesture bound becomes a backpressure question, and
  the channel reaches the compositor as a ring its manifest names rather than as
  a region in a plan. The attestation does not change.
- **A transport that can say *these entries end here* by itself** — a sealed
  channel, or a completion the producer is owed on close. Then the word travels
  in the transport's own close and `ATTEST` is retired, not reused.
- **A consumer that must check the crossing while the producer runs.** One
  attestation at the end covers everything before it and nothing after; a
  streaming check needs periodic attestations with a stated relation between
  them, which is RFC 0124's *second attestation with the arithmetic written
  down*, and not a word updated in place.
- **A compositor with the heap for the encoder.** Then the boot compares the
  frame's own encoding either side of the latch, as the host tests do, and the
  masked fold goes away rather than being kept beside it.
- **A driver shape with a state tree of its own.** Then RFC 0124's preferred
  route exists, and the question is whether a reader wants the word where the
  tree is. The ring copy should stay: it is what arrives in order behind the
  entries it covers, and a tree word cannot say which entries it was read after.
