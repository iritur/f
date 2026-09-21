# RFC 0098: A write edits the object its channel is about, and an edit it cannot afford is refused

- Status: accepted
- Date: 2026-09-20
- Affects: `abi/src/objects.rs` (`Write`, `op::known`),
  `user/objects/src/write.rs`, `user/objects/src/service.rs`,
  `kernel/src/objects.rs` (the `written` half), `bench/src/bin/rechunk.rs`,
  `claims/0017`; RFC 0037, RFC 0058, RFC 0060; and `E2-B09`, `E2-B08`

## Decision

A `WRITE` on the objects ring **edits the object its channel is about**, at
`f_abi::objects::Write::offset`, and its completion carries the object's **new**
content address. Where the channel has no object yet, a write at offset zero
**establishes** one. An edit the service cannot afford is **refused** and never
quietly downgraded to something cheaper.

Three facts force each part of that sentence, and none of them was obvious
before they were counted.

## Why the channel is the object, and not the client's choice

`Read` names what it wants by content address and fits: `32 + 4 = 36` against a
`PAYLOAD_BYTES` of 40. `Write` cannot do the same — `32 + 8 + 4 = 44` — and the
payload is one stride for every opcode, so widening it for one is widening it
for all.

**So a write has no way to say which object it means.** That is not an omission
to repair later; it is the shape the ABI already has, and the honest reading is
that the channel carries the subject. `user/objects` already behaves that way on
the read side: `serve.rs` stocks one blob and publishes its address at
`board::reported::HASH`, and the boot's client reads *that* object.

The alternative — a fifth opcode that opens an object and makes the channel
about it — is a real design and is not refused here on its merits. It is refused
*for now* because it buys nothing this claim needs: a channel already has
exactly one object, and an opcode to say so would be ceremony around a fact.

## Why editing a content-addressed object is not a contradiction

A content-addressed object's name is its content, so nothing is edited in
place: a write produces a *different* object with a *different* name.
`f_blob::extent::Extent` is exactly that — it replaces the pieces a write
touches and yields a new root — and RFC 0058 already prices it at `2 ×
EXTENT_BYTES` per edit however large the extent is.

What follows is that a write's completion **must** carry the new address, or the
client has edited something it can no longer name. This build already answers
with it in `Cqe::ext`.

## Why a boot can establish an object and cannot edit one

`Extent::create` chunks the bytes it is given — `bytes.chunks(EXTENT_BYTES)` —
and stores each piece. It allocates no piece-sized buffer.

`Extent::write` does: `vec![0u8; self.piece_bytes]`, and `piece_bytes` is
`EXTENT_BYTES` — a compile-time mebibyte — for anything this build creates,
**whatever the object's size**. That constant is deliberate and `blob/src/
extent.rs` argues for it: a piece size is a property of the writer and not a
dial.

So the two halves of this decision cost different amounts, and the difference is
a mebibyte rather than a matter of degree:

| | allocates a piece buffer | affordable to `user/objects` at a 128 KiB heap |
| --- | --- | --- |
| establish, at offset zero | no | yes |
| edit, at any offset | yes, `EXTENT_BYTES` | no |

**This is why the refusal is part of the decision and not an implementation
detail.** A service that met an edit it could not afford by storing the
submitted bytes as a fresh object would answer `Ok` to a client whose object it
had silently replaced. That is precisely what the first cut of this arm did —
it dropped `offset` entirely — and a client asking to write at 4096 was told
`Ok` and got something else.

## Where the measurement lives, which is the part that changes a claim

`claims/0017` measures bytes re-chunked and re-hashed per application byte
written, over edits at drawn offsets into an 8 MiB and a 128 MiB object, and
`intent/0006-state/spec.md` defines an application byte as one the client
submitted **on the objects ring**.

Every one of those edits allocates a mebibyte. No component heap this project
would sanely configure can run that workload, and raising one to suit a
benchmark would be a component sized by its own measurement.

It does not have to. The spec says *a ring*, not *a boot*. `f-objects` is a
workspace member whose library builds and tests on the host — only its image
half is gated on `target_os = "none"` — so a host process can put a real
`Service` over a real `f_ring` channel with a real `registry::Table`, at this
claim's own geometry, and take both halves of the ratio at the boundary the
spec names. That is where `claims/0017`'s workload belongs, and the guest's
memory was never what stood in the way.

**What the boot keeps is the transport claim and not the ratio.** `cargo xtask
objects written` establishes objects at offset zero and asserts that the bytes
crossed and that both sides counted the same — which is a statement about a
ring, and is all a boot of that size can honestly make.

## What building it established, on 2026-09-21

This entry was accepted the day before the workload that needed it was built, so
what the build found is recorded here rather than left to be inferred from a
diff.

**The refusal moved from a constant to a property of the caller, and that is
this decision rather than a widening of it.** The sentence above is *an edit the
service cannot afford is refused*, and the first implementation read it as *an
edit is refused*: `WritePath::apply` returned `refusal::ADDRESS` for every
non-zero offset, unconditionally, and there was nothing a caller could pass that
would change it. The subject is a field now — `Service::about` sets it,
`WritePath::subject` holds it, an edit against it is answered with the extent's
new root, and a channel with no subject refuses a non-zero offset exactly as
before.

**No boot's behaviour changed, and that is checkable rather than asserted.**
`user/objects/src/serve.rs` constructs the service and never calls
`Service::about`, so every channel in a boot has no subject, and `cargo xtask
objects written` puts the same four whole-object writes across the same ring
with the same refusal behind them. The 128 KiB heap in `kernel/src/objects.rs`
is unchanged and was never raised.

**What could afford it was a host process, which is what the table above already
implied and did not say.** `bench/src/bin/rechunk.rs` holds an `Extent` at
`claims/0017`'s own geometry — 8 MiB and 128 MiB objects, five mixtures, two
seeds — fills a registered buffer, submits an `f_abi::Sqe`, and reads both
halves of the ratio back off `f_objects::Served`. That claim is `gating` as of
the same day, on `application_bytes_written_at_the_ring = 5 324 800` asserted
equal to the total the workload drew.

**One cost arrived that this entry did not forecast.** *The completion carries
the object's new content address* is a requirement, and an extent has no address
until it is snapshotted — so there is one snapshot per write, where the bench
used to choose an interval. It is recorded as its own row and deliberately not
folded into the per-edit cost: a snapshot is proportional to the piece count and
therefore to the object, so adding it in would turn `2 x EXTENT_BYTES at both
object sizes` into two numbers sixteen times apart, which is the property the
extent kind exists to have.

## What would reverse this

**A `PAYLOAD_BYTES` that fits a hash and an offset.** Then a write can name its
object, the channel stops being the subject, and this decision is a workaround
for a stride. The stride is a wire number and RFC 0060's ordering rule applies:
free while one peer exists, expensive once two do.

**A piece size that is a property of the extent rather than of the writer.**
`EXTENT_BYTES` being compiled in is what makes an edit unaffordable at any
geometry. If a small object could carry a small piece, a boot could edit one and
the table above collapses — but `blob/src/extent.rs` argues that a piece size a
reader has to be told is a format with a dial in it, and that argument is not
weakened by this claim wanting one.

**An opcode that opens an object.** If a client ever needs a channel to be about
an object it chose rather than one it was given, the fifth opcode arrives and
*the channel is the object* becomes *the channel is the object you opened*. This
decision is compatible with that and does not anticipate it.

**`E2-B09` measuring a ratio in a boot.** If the claim's rows are ever taken
from a boot rather than a host process, then either a heap was grown to fit a
benchmark or a piece size became a dial, and both are reversals of something
else that should be argued where it lives rather than here.
