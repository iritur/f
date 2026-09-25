# RFC 0124: A frame may fold a reading it may not read

- Status: accepted
- Date: 2026-09-24
- Affects: `abi/src/input.rs`, `kernel/src/input.rs`,
  `user/virtio-input/src/{driver.rs,routing.rs,component.rs}`, `xtask`'s
  `INPUT_PATH` regime (unchanged, and that is the point), `TODO.md`'s `E3-B01i`
  and `E3-B04` lines

## Decision

The frame may compute, hold and compare a **fold** over input entries that
crossed a ring — `f_abi::input::Crossing`, a checksum over each entry's opcode
and payload — although it may not read, name, carry or publish the reading those
payloads contain. A fold is not a reading: it is not ordered against anything,
it is not invertible, it cannot be subtracted from another number, and there is
no expression in `kernel/` that could turn one into a latency. What the fold
buys is the one property every count on this path leaves open — that the entries
which *arrived* are the entries that were *sent* — and it buys it as an
agreement between two words neither side computed from the other: the driver
folds every entry it puts on the ring and publishes its word; the frame folds
every entry it drains and requires the two to match. `kernel/` therefore stays
off `xtask`'s `INPUT_PATH`, with no exemption written and none needed, and the
day it genuinely names the stamp it gets a row and goes red on the one `rdtsc`
in this tree, exactly as before this RFC.

## Context

`E3-B01i` built the compositor's late latch and closed its host half on
2026-09-24. Its boot half did not close, on a blocker that entry names
precisely: the predictor needs stamps; the only route to the compositor in
`cargo xtask input deliver` is the frame; the frame may not hold a clock reading
— `kernel::input::Kept` is two numbers on purpose, "in a crate whose whole claim
on this path is that it never holds one" — and naming a stamp in `kernel/` would
need an `INPUT_PATH` row that the timestamp counter `kernel/src/smp.rs` reads to
bound a spin turns red immediately.

That blocker is real and this RFC does not lift it. What it observes is that the
blocker was doing two jobs at once. One is the job `E3-B04a` and RFC 0099 wrote
it for: **nothing downstream of the driver may decide when an event happened**,
because a second reading does not add noise to `presented_at - stamped_at`, it
subtracts the second stage's own queueing delay out of it, silently, in the
direction that makes the published figure improve as the system gets slower. The
other job it had drifted into is broader and nobody argued for it: *the frame
may not compute any function of the bytes an entry carries*. The second is what
stopped the boot from being able to say anything about the crossing at all.

What was true before this diff, in the `input=deliver` boot:

- The driver stamps once per report and submits `f_abi::input::Event` entries on
  a ring whose other end the frame holds.
- The frame drains, calls `Event::decode` on every entry — which already reads
  the stamp, inside `abi/`, and refuses `NOT_STAMPED` — keeps two coordinates,
  and throws the rest away.
- Nine clauses of `Report::driver_verdict` check **tallies**: records off the
  device, reports closed, readings taken, entries submitted, drained, decoded,
  refused, dropped, malformed. Every one of them is a count.

A relay that minted a reading on arrival satisfies all nine. So does one that
handed two entries on out of order, one that dropped an entry out of the middle,
and one that moved a coordinate by one. The first of those four is the precise
defect `input/src/stamp.rs` exists to prevent, and until this diff the boot that
exercises that path could not have detected it.

Three alternatives were live.

**A field on the relay.** Widen `Kept` to carry the stamp, and let the frame
write the three pointer words the compositor already reads. This is the shortest
diff and it is the one the blocker rules out: the frame would hold a reading,
`kernel/` would earn an `INPUT_PATH` row, and the row would be red on arrival.
It also makes `Kept`'s own comment false, which is a comment written to be load
bearing.

**Byte identity.** Have the frame compare the payload bytes it forwarded against
the bytes on the far side. This is sound and it is what a two-channel relay
should do; it was not available this wave, because the far side is
`user/compositor/`, which another builder held, and because with one worker core
there is no far side running at the same time.

**An attestation on each side.** Taken. It is the shape `E3-B04c` used for the
seam between the input path's prediction and the compositor's latch — two
records, neither computed from the other, sharing an implementation and not an
instance — and it is the shape `E3-B01j` used for the boundary-crossing count:
both sides count, neither derives from the other, and a deliberately miscounted
side goes red.

## Consequences

**What it makes easy.** A relay defect on the input path is now a red boot with
a sentence naming it, rather than a green boot and a latency figure nobody can
check. The mutation evidence is in the task's report: a frame that drops one
entry from its fold, a decoder that adds one nanosecond to every stamp it hands
back, and a driver that attests to one entry of the five it sent each produce a
different failure sentence, and the second of the three has counts that agree on
both sides — which is the case no tally in this file could ever have caught.

**What it makes easy next.** The compositor's drain is now a small diff rather
than a design question, because the consumer half is written down and has a
running example. See *What the consumer has to do*, below.

**What it makes hard.** A stage that is *entitled* to rewrite what the device
said cannot carry this word through. A coalescer, or an input router —
which no line in `TODO.md` owes yet; this sentence said `E3-B04` did until
2026-09-24, and it does not — may legitimately hand on one motion where two arrived,
and the honest repair is a second attestation with the arithmetic relating the
two written down — not a fold that stops watching the body. `Crossing`'s own
doc comment says so at the point somebody would otherwise widen it.

**What it forecloses.** It does not foreclose the row. If `kernel/` ever reads
the stamp — names the type, names the wire field, depends on `f-input` — it goes
on `INPUT_PATH` and the build goes red, and this RFC is not a reason to argue
otherwise. The line this draws is between *reading the number* and *checksumming
the bytes it is one of*, and it is drawn where `Event::decode` already stands:
that function reads the stamp too, inside `abi/`, and the frame has called it
since `E3-B04d` without anybody calling the frame a stage.

**A cost found by writing this down, which is worth a paragraph of its own.**
The first draft of this decision put the consumer's four steps in
`kernel/src/input.rs`'s module comment, where a reader of the boot would find
them. That is a red build, and the failure is instructive rather than annoying:
`cargo xtask lint-stamp` decides membership of the input path by looking for the
stamp's vocabulary in a crate's **text, prose included**, so a paragraph in
`kernel/` naming the stamp's type and its wire field earns `kernel/` a row. The
observed failure was

```
xtask: 1 finding(s) on the input path:
  kernel/  handles an input timestamp and has no INPUT_PATH row: it carries
  `f_input`, which is a use of that crate from source.
```

which is the rule working. The frame may not so much as *describe* the number.
So the description lives here, in a document that is not a workspace member, and
`kernel/src/input.rs` carries a pointer and a sentence in words it is allowed to
use.

## What the consumer has to do

Written here so that the next builder does not re-derive it, and here rather
than in the frame for the reason above. Four steps, in this order.

1. **Hold the other end of the driver's data channel.** The driver holds the
   client's end and submits; the server end is a channel like any other, adopted
   with `f_ring::adopt::Adopted::at(address, length, 0, 0)` from two words on a
   routing page and drained with `f_ring::Consumer::new`. The frame holds it
   today only because `E1-B05`'s ring-3 supervisor cannot yet hand a place's
   occupant a core *and* a peer, which is `CHAOS_GAP`. `user/panel` is the
   component this shape should be copied from; it is the other client component
   in the tree, and a driver's loop is the wrong model.

2. **Decode with `f_abi::input::Event::decode`.** The same function the frame
   calls. It refuses a payload whose first eight bytes are
   `f_abi::input::NOT_STAMPED` before it reads the body at all, and it compares
   all sixty-four bytes of the arriving `Sqe` against the entry this build would
   have written. A component that read the payload some other way would be a
   second decoder, and `NOT_STAMPED` would then have two opinions.

3. **Rebuild the reading with `f_input::stamp::StampNanos::from_wire_nanos`,
   passing `event.stamp_nanos` straight in.** Not through a local. `cargo xtask
   lint-stamp` requires that constructor's argument on the input path to be a
   field read or a literal, and the reason is not pedantry: a local is exactly
   where a helper's return value lands, which is the two-statement spelling of
   minting a reading. `user/compositor/` is already on `INPUT_PATH` and already
   mints one `StampNanos` for the scanout it aims at (RFC 0120), so this is a
   second call under a rule that crate already keeps.

4. **Fold what it drained, and check it.** A `Crossing` in the compositor,
   absorbing every entry it decodes, is the consumer half the frame holds today.
   What it compares against is the driver's word, and that word has to reach it
   by some route other than the driver's routing page — the frame takes that page
   back when the driver is reaped. A ring or the driver's state tree; the tree is
   where a reader would look, and `Driver::crossing`'s comment names it as the
   reversal of not having a `[[state]]` row today.

What the consumer must **not** do is read a clock when an entry arrives and
treat that as the event's time. That is the one thing on this path that is wrong
and looks right, which is why there is a lint and not a convention.

## What would reverse this

- **A stage between the driver and the consumer that is entitled to rewrite what
  the device said.** A coalescer or an input router, which no line in `TODO.md` owes yet. Its arrival does not make
  this wrong; it makes one attestation insufficient, and the repair is a second
  word with a stated relation to the first. Watch for it arriving as a quiet
  widening of the fold to ignore the body, which is the same defect wearing this
  RFC's own clothes.
- **A peer that is not this machine's own driver.** The fold is FNV-1a, which
  anybody can invert, and that is fine for a checksum between two stages of one
  machine and worthless against an adversary who chooses the entries. On the day
  the producer is untrusted this becomes a content address, moves to `f-hash`,
  is taken over a transcript, and costs what `claims/` would have to measure.
- **`kernel/` reading the stamp for any reason at all.** Then the frame is a
  stage and not a courier, it takes an `INPUT_PATH` row, and the row is red on
  the timestamp counter `kernel/src/smp.rs` reads. That is the rule working and
  this RFC is not an argument against it.
- **The compositor draining the ring itself**, which is `E3-B04g` and
  the thing this makes cheap. On that day the frame stops being the consumer,
  its fold moves to the component, and what is left in `kernel/src/input.rs` is
  a supervisor that no longer decodes anything. Nothing here should survive that
  unchanged except `Crossing` and the four steps above.
  **Appended 2026-09-24: this reversal is paid, by RFC 0125.** The compositor
  drains the ring and holds the fold; `kernel/src/input.rs` decodes nothing and
  takes nothing off that ring; the driver's word reaches the compositor on the
  ring itself, under `f_abi::input::ATTEST`, which is step 4's *a ring* rather
  than its *state tree*. The decision above stands as written — the frame still
  compares folds it did not compute, now the compositor's against the driver's
  routing-page copy — and the four steps were followed as written, which is the
  most a section written to be copied can be asked for.
