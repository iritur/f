# RFC 0060: A publish is a barrier sequence, and atomicity is not free

- Status: accepted
- Date: 2026-09-06
- Affects: `docs/design/deadline-all-the-way-down.html` section 04 (the
  *Durability, and the read path* paragraph, whose sentence *Atomicity is free
  because a root is a single write* this reverses); the `op` module of
  `user/virtio-blk/src/driver.rs`, and `kernel/src/blk.rs`, which imports it;
  `abi/src/store.rs`'s root record and its `check` field; `blob/src/device.rs`'s
  `Device::flush`; `zone/`'s mount, its two root zones and its collector's
  release point; RFC 0011 (a peer that does not offer the barrier), RFC 0012
  (the root and the root record are two things), RFC 0025 and RFC 0049 (what
  class a barrier is submitted as), RFC 0058 (the dirty log it refuses) and RFC
  0059 (the pin released at the second barrier); `E2-B01`, `E2-B02`, `E2-B03`,
  `E2-P01` and `E2-P07`. No task line in `TODO.md` names this RFC; that is
  reported to the originator rather than fixed by editing that file

## Decision

A publish is four device operations and not one: **write the blobs, `FLUSH`,
append the root record, `FLUSH`**. The first barrier is what makes the root
record's meaning true — every hash it names is on stable media before the record
naming them exists — and the second is what makes the record itself durable
before the next publish is allowed to start. To make that sequence expressible
the blk service's opcode space gains five numbers, fixed here and answered
later: `FLUSH` = 3, `ZONE_APPEND` = 4, `ZONE_FINISH` = 5, `ZONE_RESET` = 6 and
`ZONE_REPORT` = 7. And because a device may acknowledge a barrier it did not
honour, and nothing above it can tell, correctness does not rest on the barrier
at all: **a root record is believed only after its `check` field verifies and
after every child of its generation node resolves** — verify before accept. The
barrier is what makes a lost publish *rare*; the verify is what makes a lost
publish *a rollback to the previous generation* rather than a generation naming
blobs that are not there. We are choosing to pay two device barriers per publish
and a bounded tree resolution per mount, and to say out loud that section 04's
*atomicity is free because a root is a single write* was wrong.

## Context

`docs/design/deadline-all-the-way-down.html` section 04 has said since it was
written that the durability primitive is *publish this root: write the blobs,
then write the root*, that "atomicity is free because a root is a single write",
and that a crash mid-publish leaves the previous root intact with some
unreferenced blobs. The first and third clauses survive this RFC unchanged. The
second does not, and a reversal of something already written down is an RFC.

The sentence is wrong in two independent ways, and separating them is worth the
paragraph because only one of the two is about hardware.

**The single write was two things wearing one name.** RFC 0012 splits them: the
**root** is the thirty-two bytes the fold produces and it *identifies*; the
**root record** is the fixed-width record appended to a root zone and it
*durably orders*. Writing thirty-two bytes atomically is a property of a block
device; writing them *after* the blobs they name is not a property of anything
unless something asks for it. "Atomicity is free" answered the first question
and was quietly read as an answer to the second.

**Nothing in the tree could ask.** `user/virtio-blk` answers exactly two opcodes
— `op::READ` = 1 and `op::WRITE` = 2 — and there is no third. The only path to a
device is the blk driver's ring, so at the moment this spec was written there
was no way to express *persist what I have already written* on any path this
system has, and no way to express a zone reset either, which `f-zone` needs
before it can exist at all. A design whose durability argument requires an
operation the wire cannot carry is a design with a hole in it, and the hole was
invisible because the design page had declared the problem free.

What was true when this was decided. `f-hash` exists and names every blob,
object and generation. RFC 0012 has fixed the root record's fields, including
`check` (`Unit: none`, the SHA-256 over every preceding field of the record) and
`generation` (a one-based counter, zero reserved, and no timestamp anywhere —
RFC 0004). The two root zones and `ROOT_CARRY` = 16 are fixed by the spec, so
there is always a durable root on the device and a rollback always has somewhere
to go. `f-zone` does not exist; neither does `blob/src/store.rs`. That is the
point of writing this now: these five numbers are new wire that two peers read —
`kernel/src/blk.rs` imports `f_virtio_blk::driver` — and `TODO.md` ordering rule
1 puts the wire first, whatever else is ready, because the cost of changing it
rises the moment a second peer reads it.

### The alternatives that were live

- **Trust the device: keep writing the blobs then the root, and add nothing.**
  The status quo, and free. It fails on the one case the store exists to
  survive: a writeback cache that reorders the root record ahead of the blobs
  turns a power cut into a root naming hashes that are not on media, and every
  read of that generation answers *not found* — indistinguishable from
  *collected*, which is the confusion `check` was already introduced to keep out
  of the record. Refused because the failure it admits is silent and permanent,
  and because we would be shipping a durability story we knew was untrue.
- **Turn the writeback cache off** (`VIRTIO_BLK_F_CONFIG_WCE`, writing through),
  so every completed write is already durable and no barrier is ever needed.
  Correct, and refused on cost: it makes *every* blob write pay a round trip to
  media, including the collector's copy-forward, which is batch-class work whose
  whole design is that it never competes with a deadline-class read. It is also
  a device-wide setting rather than something a publish can ask for, so there is
  no way to buy durability where it is needed and not elsewhere. It stays as the
  fallback for a device class that offers no barrier, and *What would reverse
  this* says when it becomes the default instead.
- **An intent log in front of the store**: write a *publish begins* record, then
  the blobs, then a commit record, and replay at mount. Refused for RFC 0058's
  reason, in RFC 0058's words: it puts durable bytes in front of the store that
  no hash names, so `E2-P01`'s exit — every cut leaves the old root or the new
  one, never a third thing — becomes *the old root plus a replay*, and the
  replay is the third thing. It also does not remove the barrier, it moves it:
  the commit record has to land after the blobs, by the same argument.
- **Verify only, and never flush at all.** Drop `FLUSH` and lean entirely on
  verify-before-accept: an unresolvable root is refused at mount, so the format
  is correct with no barrier anywhere. This is the alternative worth taking
  seriously, because it is *correct* — and it is refused on frequency rather
  than on correctness. With no barrier the device is free to reorder every
  publish, so an ordinary power cut loses the newest generation most of the time
  rather than rarely, and an update system whose new generation usually
  disappears at a power cut is not an update system. The barrier buys the
  *rate*; the verify buys the *bound*. Keeping both is the whole decision, and
  this alternative is what shows they answer different questions.
- **Positioned writes rather than `ZONE_APPEND`**, with the format maintaining
  its own copy of each zone's write pointer. Refused: two appenders then race a
  pointer the device also holds, and after a cut there are two pointers to
  reconcile with no record of which was ahead. The device already maintains the
  authoritative one; asking it to assign the position is strictly less state.

## Consequences

### The five opcodes, what each promises, and what it does not

These are **this service's opcode space**, as `ring-scene-boot` section 05 has
it: a storage ring and a compositor ring share the envelope and not the words.
They are deliberately *not* the virtio request type numbers, which are given
beside them so that a reader can check the mapping against the virtio
specification's block device section rather than take it on trust. `FLUSH` = 3
is not `VIRTIO_BLK_T_FLUSH` = 4, and that they differ is the point: a ring entry
is a ring entry until the driver translates it, and a build that had started
handing one straight to the device would be caught by arithmetic rather than by
luck. Numbering continues from `WRITE` = 2 and stays dense; zero still names
nothing, so a zeroed entry is still refused with
`ARGUMENT`/`UNKNOWN_OPCODE`, which is why the space started at one.

**`FLUSH` = 3** → `VIRTIO_BLK_T_FLUSH` = 4, available under
`VIRTIO_BLK_F_FLUSH` (feature bit 9).
*Promises:* when its completion is reported, every operation this driver had
already reported a completion for **before this entry was submitted** is on
stable media.
*Does not promise:* anything about operations still in flight; anything about
operations submitted after it; that the device told the truth; or any ordering
between two `FLUSH`es outstanding at once. It carries no buffer and no length —
`len` = 0 and no registered buffer name — because virtio has no ranged flush and
an entry that named a range would be a client believing in one. There is no
partial `FLUSH`: it completed or it did not.

**`ZONE_APPEND` = 4** → `VIRTIO_BLK_T_ZONE_APPEND` = 15.
*Promises:* the payload is written at the target zone's current write pointer,
the **device** chooses the position, and the completion carries that position
back. The submitter's `offset` names the zone's start and never the destination.
*Does not promise:* which of two concurrent appends lands first, that the
position is the one the submitter would have computed, or durability — that is
`FLUSH`'s, and the two are separate on purpose. The consequence that reaches
other files: an index records the position **the completion returned** and never
one the submitter derived, and RFC 0059's transient root is registered at a
publish's first `ZONE_APPEND`.

**`ZONE_FINISH` = 5** → `VIRTIO_BLK_T_ZONE_FINISH` = 22.
*Promises:* the zone becomes full and accepts no further append; its write
pointer is at the zone's end.
*Does not promise:* that the zone's contents are durable — a seal is not a
barrier — that the zone is now a collection candidate, or anything about live
bytes. The live fraction is the collector's tally under RFC 0059, not something
the device knows.

**`ZONE_RESET` = 6** → `VIRTIO_BLK_T_ZONE_RESET` = 24.
*Promises:* the named zone's write pointer returns to the zone's start, and
every byte previously in it becomes unreadable.
*Does not promise:* that those bytes are unrecoverable in any adversarial sense
— that is `VIRTIO_BLK_T_SECURE_ERASE`, which this space does not carry and which
this opcode must not be read as — and it promises nothing about any other zone.
It is destructive and has no undo, which is why the root-zone wrap carries
`ROOT_CARRY` = 16 records to the other zone and flushes *before* it resets the
full one.

**`ZONE_REPORT` = 7** → `VIRTIO_BLK_T_ZONE_REPORT` = 16.
*Promises:* for each zone in the requested range, the device's own view of its
start, its type (conventional or sequential-write-required), its state and its
write pointer, landed in a registered buffer like any other read.
*Does not promise:* that the answer is still true when it is read. It is a
snapshot, and the only consumer permitted to act on one is **mount**, before
anything else is writing — which is exactly what makes *mount takes no mutable
pointer* affordable, because the pointer it needs is the device's and it is
asked for rather than maintained.

The zoned request types arrived in the virtio specification with
`VIRTIO_BLK_F_ZONED` (feature bit 17), later than 1.2; the numbers above are
what a reader checks against whichever version the image implements, and
`E2-B02` negotiates the feature bit rather than assuming it. A device offering
neither `VIRTIO_BLK_F_FLUSH` nor `VIRTIO_BLK_F_ZONED` is refused at mount under
RFC 0011's negotiation, not degraded silently into a store with no barrier.

**Five constants land now and no behaviour lands with them.** `op::known` is
*not* widened in this diff, so all five are refused with
`ARGUMENT`/`UNKNOWN_OPCODE` exactly as any unknown opcode is, and `cargo xtask
blk` — all three halves, including the two controls that must fail — is the
evidence that the datapath did not move. `E2-B02` is where they are answered.
Fixing the numbers before the behaviour is the whole of ordering rule 1: the
number is the expensive half to change, and it is free today.

### Verify before accept, and what a lying device costs

A device that acknowledges `FLUSH` without persisting is not detectable from
above. There is no probe, no read that distinguishes it, and no counter that
catches it — so the format does not try. It rests instead on a rule that is
cheap and total:

> A root record is believed only after (1) its `check` field verifies over every
> preceding field of the record, and (2) every child of its generation node
> resolves — the frame hash, the topology hash, and each component leaf beneath
> them.

Mount reads both root zones' write pointers with `ZONE_REPORT`, scans backwards
from each, and takes the **highest `generation` that satisfies both clauses**.
Nothing else is consulted; there is no in-format pointer to the current root, so
there is nothing that can be stale.

Clause (1) catches the torn record — a record whose magic landed and whose
`root` field is half-written names a hash that never existed, and *not found* is
indistinguishable from *collected*. Clause (2) catches the lying device: if the
blobs did not reach media, the generation node does not resolve, and the record
is refused however well-formed it is.

**So a lying device costs a rollback, not a corrupted generation.** The
half-published generation is not a third state — it is the previous root plus
some unreferenced blobs, which is exactly what section 04 always said a crash
mid-publish leaves, and the next collection reclaims them because nothing pins
them. The machine comes up on generation *n−1*, which is a generation that was
published, verified and booted before. Nobody loses a machine; somebody loses
the last update and is told so, which is why `roots_refused_unresolved` is a
count in `cargo xtask cut`'s artefact rather than a log line: the path is
exercised on every simulated cut rather than argued about.

**The cost of the rule is bounded by the generation tree and not by the blob
count.** A generation node has two children (RFC 0012), and the tree beneath is
the frame, the topology and one leaf per component file — tens of nodes for the
system this project ships, not the millions of chunks the store holds. Mount
resolves *candidates* and stops at the first that verifies, so the common case
is one tree. That bound is why the rule is affordable at all, and it is also the
reversal condition, stated below with the count that would move it.

### What this makes easy

- **Atomicity has a stated price, and the price does not grow.** Two barriers
  per publish, whatever the object count. "Free" was never checkable; "two"
  is — and `cargo xtask cut` counts device operations by opcode.
- **The collector gets a release point it can name.** RFC 0059's transient root
  is released when the *second* `FLUSH` completes and the root has entered the
  pinned set, in that order. Before this entry there was no event to point at.
- **Mount holds no mutable state on the device.** No superblock pointer to
  re-point, no current-root field to tear: the write pointers come from
  `ZONE_REPORT` and the choice is a scan plus a verify.
- **`f-zone` becomes expressible at all.** Sequential fill, seal, copy-forward
  and reset are four operations on the wire rather than four sentences in a
  design document.

### What this makes hard

- **A publish serialises on two device round trips.** A consumer publishing per
  small write pays them per write, and no amount of parallelism inside the store
  removes them. Batching publishes is the consumer's business; `E2-B03`'s index
  is where a batch of changes becomes one publish.
- **Mount does real work per candidate root**, and a device that produces many
  unverifiable candidates in a row — a badly behaved device, or a long run of
  cut publishes — makes mount walk. The scan is bounded by the records surviving
  in the two root zones, which is where the count in *What would reverse this*
  comes from.
- **The wire got five words wider**, and every widening is something two peers
  can come to disagree about. Putting them in `user/virtio-blk`'s own `op`
  module rather than in `abi/` is the mitigation and not an accident:
  `kernel/src/blk.rs` imports that module, so there is one definition rather
  than two, and a second place to look up an opcode is how two peers come to
  disagree about one.

### What this forecloses

- **A ranged or per-object barrier.** There is one `FLUSH` and it covers
  everything already completed. A caller cannot make one object durable and
  leave another volatile, and a future design that wants to will be reversing
  this entry rather than adding a flag.
- **`VIRTIO_BLK_T_ZONE_RESET_ALL` (26), and the operations around it.**
  Reset-all destroys every zone including both root zones and the superblock,
  which no correct caller of this store ever wants; that it cannot be *said* on
  this ring is the protection, and it is deliberate rather than an omission.
  `ZONE_OPEN` (18) and `ZONE_CLOSE` (20) are left out for the opposite reason:
  they manage an open-zone resource limit this design does not yet manage, and
  numbering them before there is a policy to spend them on would be fixing a
  decision nobody has taken.
- **A second reader of the on-disk format that accepts a root without resolving
  it.** A rescue tool, a host-side inspector, a bootloader shortcut — each would
  have to implement clause (2) or be a reader with weaker rules than the mount,
  which is how two readers come to disagree about which generation is current.
- **Durability on a device that offers no barrier.** Refused at mount under RFC
  0011 rather than degraded, because a store that silently runs without its
  barrier publishes a durability property it does not have.

## What would reverse this

- **Mount-time resolution measured, on some device class, as the dominant cost
  of coming up.** As a count, because a duration would need a clock (RFC 0004):
  the **device reads a mount performs to resolve one candidate root exceeding
  the device reads the rest of boot performs**, on a generation tree from a real
  system rather than from a fixture. At that point verify-before-accept is no
  longer cheap-and-total, and the choice becomes to trust `FLUSH`, or to verify
  lazily behind the boot, and to register a claim for whichever is taken — none
  of which this entry permits as it stands. This is the reversal the spec names
  and the most likely one; it arrives with generation trees of thousands of
  leaves, which is a system considerably larger than the one being built.
- **Barriers dominating the operation mix.** If more than half the device
  operations a real workload issues are `FLUSH`, the two-barrier sequence is the
  workload rather than its safety margin, and the answer is group commit across
  publishes — which changes what "a publish" is and therefore reverses this
  entry rather than tuning it. `cargo xtask cut`'s artefact counts operations by
  opcode so that this is observable rather than suspected.
- **`roots_refused_unresolved` non-zero on a run with no injected lie.** The
  `honest` device model completes a `FLUSH` only after the writes it covers, so
  a refused root there means either the sequence is not what this entry says or
  the model is not what a device does. Either way something written here is
  false, and finding that out from a red suite is worth more than finding it out
  from a machine.
- **A device class we intend to support where `FLUSH` is by design a lie** — a
  shared writeback cache with no power-loss protection, acknowledging from
  volatile memory. Then the barrier buys no rate at all, *verify only* above
  becomes the honest description of what we are doing, and the fallback is the
  one refused on cost here: turn the writeback cache off and pay per write.
- **`ZONE_APPEND` unavailable on a device class that must be supported.** If the
  emulated device `E2-B02` drives does not offer `VIRTIO_BLK_F_ZONED`, the
  clause *the device assigns the position* has no mechanism, and the format
  either maintains its own write pointers — the alternative refused above, with
  its two-pointer reconciliation after a cut — or the store does not run there.
  That is a finding for `E2-B02` to report against this entry, not a threshold
  to widen.
