# RFC 0090: A granted region with no device attached is bytes

- Status: accepted
- Date: 2026-09-17
- Affects: `ring/src/device.rs` (`Granted`, new, beside `Window` and `Region`),
  `kernel/src/process.rs` (`prepare_server`, which maps the region this is
  about), `user/objects/src/serve.rs` and `kernel/src/objects.rs` (the first two
  users). **Not** `user/objects/src/dma.rs`, which was expected to need a second
  constructor and does not: `Landing::open` already takes `&mut [u8]` and
  `Granted::bytes_mut` already answers one, so the two meet with nothing in
  between — which is the sharpest evidence available that this is the accessor
  that was missing rather than a new shape being introduced
- Extends RFC 0033, which decided that a granted window is a safe accessor, and
  RFC 0024, whose `Reach` is *deliberately not a slice*. Neither is reversed:
  this adds a third case they did not have, and leaves both of their cases
  exactly as they were

## Decision

A region the frame granted to one component, with **no device attached to it**,
may be handed to safe code as `&mut [u8]`. `f_ring::device::Granted` is that
accessor: constructed from an address and a length like
[`Window`](../../ring/src/device.rs) and [`Adopted`](../../ring/src/adopt.rs)
are, with the same kind of contract stated on the constructor, and yielding
`bytes_mut(&mut self) -> &mut [u8]` whose exclusivity is the borrow checker's to
enforce from there on.

The clause that carries the decision is *with no device attached*. A `Window`
names a device's registers and a `Region` names memory a device is reading and
writing while the component runs — for both of those a `&mut [u8]` would assert
an exclusivity that is false, which is the argument RFC 0033 makes and which
stands untouched. A `Granted` names memory that is in no remapping domain, that
the frame mapped for exactly one component, and that the frame does not touch
between the spawn and the join. Those three facts together are ordinary
exclusive ownership, and refusing to say so in the type system does not make the
memory safer — it makes the component reach for a raw pointer, which it may not
write, or makes the frame do the copying, which RFC 0047 moved away from.

## Context

The objects transport is what forced this. `user/objects` answers
`objects::op::READ` by resolving a hash to a block and having the content land
in **the client's registered buffer** — that is the whole of `E2-B08`, and
`copies per read is zero` is a count about exactly that memory. In a boot the
client is the frame, the frame grants the component a region, and the component
writes into it.

Every path into registered memory in this tree begins from a `&mut [u8]` the
caller already holds: `f_ring::buffers::BufferSet::bind` takes one,
`f_objects::dma::Landing::open` takes one. That worked for as long as the holder
was a host test, which owns its memory outright, and for a driver, which never
touches the bytes at all — a driver resolves a `Reach` and writes it into a
descriptor, and the *device* moves them. `user/objects` is neither. There is no
DMA engine under it: the store is in the component's own heap, so the component
is genuinely the thing that moves the bytes, and it had nowhere to put them.

Three alternatives were live and the first two were refused.

**Put the client's buffers in the component's own heap** and let the frame read
them back through its direct map. It needs no new mechanism at all — the
component already has a heap and already owns it. Refused because it inverts the
sentence the exit turns on: *the caller's buffer is a registered one, or the
zero-copy count is a copy nobody counted* is `TODO.md`'s parenthesis on `E2-B08`,
and a buffer in the callee's heap is not the caller's buffer however carefully it
is registered. The number would still be zero and would no longer be about
anything.

**Move `Landing` onto `Region`** so the service side never needs a slice, which
means `f_blob::device::Device::read` stops taking `&mut [u8]` and starts taking
a word accessor. Refused on size and on honesty: it changes the shape of the
device trait in four crates to avoid stating one fact about one region, and the
fact is true — this memory *is* exclusively the component's for the length of
the run. A design that contorted to avoid saying so would be paying for a safety
property it already has.

**This**, which is a third accessor beside two that stay as they are. It was
taken because the distinction it rests on is real and checkable rather than a
convenience: *is a device attached* is a question about an IOMMU domain, the
frame is what answers it, and `prepare_server` is the one function in this tree
that maps a region into a component without putting it in one.

## Consequences

**Easy.** A component that is its own datapath can write into memory its client
granted it, in safe code, which is what makes `user/objects` able to serve at all.
`f_objects::dma::Landing` needs no change at all — the host test builds one over
a `Vec` it owns and the component builds one over `Granted::bytes_mut`, and both
reach the same type, the same `f_ring::registry::Table` and the same tally. The
count `E2-B08` is about is taken by one body of code on both paths, which is
worth more than the convenience: a boot and a host test that disagreed about
where a byte went would be two counts wearing one name.

**Hard.** There are now three accessors over frame-granted memory and a reader
has to pick. The picking is not free and the names do not fully carry it, so each
constructor states its own contract at length and this RFC is what the three
point at. A fourth would be a smell: the space is *registers*, *shared with a
device*, and *exclusively mine*, and that is the whole space.

**Foreclosed.** A `Granted` over memory a device is reading, which would be the
same mistake `Region` exists to prevent, wearing a type that says it is fine.
Nothing in the code can stop that — an address is an integer — so the constructor
carries the obligation in its contract the way `Adopted::at` carries *base names
len bytes the frame mapped for this component*, and `prepare_server` is where the
obligation is discharged on the only path that exists today.

Also foreclosed: the frame moving a component's bytes for it. That was available
as a fallback while nothing above the frame could reach a client's memory, and
`cargo xtask lint-datapath`'s `NOT_THE_FRAME` half already refuses it for the
three drivers. This closes the last shape where it would have been tempting.

## What would reverse this

**A `Granted` constructed over memory that was in a remapping domain.** The
obligation is stated and not checked, so the failure mode is a component holding
a `&mut [u8]` a device is writing under it — torn reads, and a `copies per read`
of zero that is a lie rather than a measurement. If that happens once, the answer
is not a stronger comment: it is that the frame hands out a token only
`prepare_server` can mint, so the constructor cannot be called on memory the
frame did not grant this way. That is a bigger change and it is not worth making
against zero occurrences.

**A second component wanting one for a region a device also reads.** The
decision's whole load is carried by *no device attached*; a caller who wants the
convenience for memory that has one is asking for `Region` with the checks off,
and the right answer there is that `Region` grew the accessor it actually needs —
a bounded copy in or out — rather than that this type widened.

**`f_blob::device::Device` changing shape for another reason.** If the device
trait stops taking `&mut [u8]` — for a real driver, for an async submission path,
for anything — then the second alternative above stops being expensive, and this
type loses its only user. It should go at that point rather than stay as a
convenience nothing needs.
