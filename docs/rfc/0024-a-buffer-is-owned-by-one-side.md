# RFC 0024: A buffer is owned by one side

- Status: accepted
- Date: 2026-09-02
- Affects: `abi/src/buf.rs`, `abi/src/cap.rs`, `abi/src/lib.rs` (the
  `buf_set`/`buf_index` reading and one `ARGUMENT` code), `ring/src/buffers.rs`;
  `docs/design/ring-scene-boot.html` sections 04 and 05, section 04 gaining a
  sentence that says its cancelling `Drop` is not what was built; step 1 of RFC
  0008's teardown order; `E1-B01`, `E1-B02`, `E1-B03` and `E1-B10`, which build
  on it

## Decision

At any moment a buffer is held by exactly one side of a ring, and which side is
a fact the compiler knows on the client and the service checks on the wire.

**Registration.** A component registers a buffer set with a service *over the
service's ring*: a registration entry in that service's opcode space carries, in
`Sqe::cap`, a handle to memory the component holds with `GRANT`. The service
derives a child capability of type `CapType::BufferSet` from it — a child, so
that revoking the parent, or the component ending (RFC 0008, step 1), reaches
the registration — and on the registered path asks the frame to give its IOMMU
domain a translation for that memory (`E1-B01`). The completion's `ext` carries
an `abi::buf::SetId`: sixteen bits of registration slot and sixteen of
generation, generation from one, packed the way `cap::Handle` is and for the
same reasons. The id is the *channel's* name for that derived capability. It
means nothing anywhere else, and a zeroed entry names no set. `E1-B10` fixes the
registration entry's layout and its opcode; this RFC fixes what it answers with
and what the answer means.

**Ownership moves with a submission and returns with a completion.** On the
client, `f_ring::buffers` says this in types. A buffer is `Idle` — the only
state with a `bytes_mut` — or `InFlight`, which has no method reaching the
memory. `Idle::submit` takes the buffer by value and writes its name into the
entry itself; `InFlight::complete` gives the buffer back only for a completion
carrying its token and not flagged `MORE`. A refusal returns the buffer, and so
does a cancellation, because in both the service is finished with it. An `Idle`
comes into existence only through `BufferSet::carve`, which divides the region
the set was *bound over* — a set holds its memory, so the only bytes a set can
name are bytes it covers — and which borrows the set for as long as any of
those buffers lives. So a buffer cannot outlive the set that names it, the
region cannot be reused while a buffer is carved from it, and a set is carved
exactly once: a second carve would be two buffers with one name, and the
compiler refuses it.

**Two paths, one naming type parameter.** On the registered path
(`buffers::Fixed`, `flags::FIXED_BUF` set) the entry carries the set id and an
index and no address crosses the boundary. On the shared-virtual-memory path
(`buffers::Virtual`, `FIXED_BUF` clear) the same eight bytes are the buffer's
virtual address in the submitter's own space, low half in `buf_set`, and the
address-space identifier is the channel's rather than the entry's. That path is
behind `feature::SHARED_VIRTUAL_MEMORY`, RFC 0011 style: `BufferSet::bind`
refuses `Virtual` on a channel that did not negotiate the bit, and a service
reading an address on such a channel refuses the entry. What the two paths owe
each other is everything except the naming — the same `Idle`/`InFlight` types,
the same move and return, and one test body that takes the naming as a
parameter. On the virtual path nothing is registered with the device, and the
`BufferSet` is a ledger of who holds what rather than a record of a
registration; that ledger is the part section 04 says must survive when
registration does not.

**What the types make unrepresentable**, each with a `compile_fail` fixture on
the module, pinned to the error code so that rustdoc checks the reason and not
only the failure: writing an in-flight buffer (no method: `E0599`); naming
memory the set was not bound over, since an `Idle` has no public constructor and
the only ones that exist are pieces of the set's own region (`E0451`); carving
one set twice, which is how two buffers would come to carry one name (`E0499`);
and submitting the same buffer twice, or submitting one that is in flight (use
after move: `E0382`).

What the types do **not** make unrepresentable, and the honest boundary of the
paragraph above: that the `SetId` a set was bound with names a registration the
service ever made. Nothing issues one until `E1-B10`, so `BufferSet::bind` takes
it on trust and the refusal below is the whole of the defence. The rule the
types do carry — a buffer names memory its own set covers — is the half that
does not depend on a registration existing, and it is the half that stops a
client writing one region while the device transfers another.

**What stays a runtime refusal**, and the `abi::error` domain each earns under
RFC 0010. A set id nobody could have issued, or one the service never issued on
this channel — including one a client simply made up, which at E1 is every one
of them: `AUTHORITY`/`NO_SUCH_CAP`, detail the id. A set id whose generation
has been retired by de-registration: `AUTHORITY`/`REVOKED`. An index
past the set, a length past the buffer, or a buffer the service already holds
in flight: `ARGUMENT`/`BAD_ADDRESS`, whose text already says *already occupied*,
detail the offending field. An address on a channel that did not negotiate
shared virtual memory: `ARGUMENT`/`FEATURE_NOT_NEGOTIATED`, a new code, detail
the feature bit. A null address on one that did: `ARGUMENT`/`BAD_ADDRESS`. These
are the service's, because a type binds only the code compiled against it and
the far end of a ring is bound by nothing but its own checks — a peer that
writes raw entries is the hostile peer section 06 already assumes.

Two misuses are neither, and both are the caller's own bookkeeping.

**Lending one token to two buffers.** A completion is matched on `user_data`
and on nothing else, so two buffers in flight under one token are
indistinguishable and the first one asked takes the answer — which may be the
buffer the device is still writing. It cannot be a compile error, because a
token is a number the caller chooses and the type system does not count the
numbers a program has outstanding; it cannot be a wire refusal, because both
entries are well formed and the service has no view of which buffer the client
believes each token belongs to. So it is stated where the caller meets it, on
`Idle::submit`, and shown as a fixture —
`two_buffers_on_one_token_return_the_wrong_one` — so that a reader sees the
hazard rather than discovers it. A runtime that hands out tokens rather than
taking them, `E1-B08`'s, removes it; until one exists the obligation is written
down.

**Dropping an `InFlight`** cannot be a compile error in a language without
linear types, and it cannot be a wire refusal because nothing crosses the wire.
It is a drop bomb: the drop panics, and under this workspace's
`panic = "abort"` the component ends, at which point RFC 0008 revokes its buffer
sets and tears down its IOMMU domain so the transfer the device was still doing
faults rather than lands. The frame is the graveyard section 04 asks for. The
one way back from `InFlight` without a completion is `InFlight::reclaim`, which
takes a `PeerGone` — a token constructible only from evidence the peer's
outstanding completions are void, today `RingError::EpochChanged`.

## Context

Section 04 of `ring-scene-boot` sketched this in three paragraphs: register
once and receive a set id; on capable hardware not even once; and an API shape
in which the buffer is moved into a `Submitted` and returned only by
`complete`, with `Drop` submitting a cancel and moving the buffer to a runtime's
graveyard. Section 05 said the buffer triple *collapses to a plain address* under
shared virtual memory, and section 11 of the architecture document named
ergonomics — completion I/O with borrowed buffers — as the risk most likely to
sink the ring. RFC 0008 has since written *every registered buffer set — an
object under E1-D03, and so a capability* into its teardown order, ahead of this
RFC existing. So the decision was mostly made and unrecorded, which is the state
in which two implementers pick differently.

`E1-B02` and `E1-B03` are the first drivers, `E1-B10` is the task that builds
registration and the feature-gated path and measures the two, and `E1-B01` is
the IOMMU that neither exists without. None of them had a type to write against.
This RFC is written before them so that the first driver is written against the
ownership rules rather than the ownership rules being reverse-engineered from
the first driver.

The alternatives that were live:

- **Reference-counted buffers**, the shape most async runtimes reach for. The
  buffer stays alive while the device holds it, so nothing is freed underneath a
  transfer — and nothing stops the client *writing* it either, which is the
  same bug with the crash removed. Rejected because it hides the failure rather
  than excluding it.
- **Borrowed buffers whose lifetime is tied to the completion future.** Sound
  until the future is dropped, which is the case that matters; cancellation
  safety becomes a rule in the documentation. This is the failure section 04
  describes and the reason the design says *move*, not *borrow*.
- **A kernel-owned pool with copy-in and copy-out.** Solves ownership by having
  none to transfer, and contradicts the invariant the whole design rests on —
  buffers are handed over, never duplicated. Not considered further.
- **One path only.** Shared virtual memory alone excludes every device without
  address translation services, which is every virtio device and so every
  device this epoch has. Registration alone forgoes the measurement section 13
  wants — pinned-and-registered against shared-virtual, same workload — and
  that measurement is one of the cleaner results this system can produce.
  Both ship, and the naming is the only thing that differs, so that the
  measurement compares one thing.
- **The design's `Drop` that submits a cancel.** Correct, and it needs a
  runtime able to submit from inside a destructor and hold a graveyard until
  the cancel completes. No such runtime exists at E1; `E1-B08` is where one
  arrives. Adopting the shape without the runtime would have made `Drop` a
  silent leak. The drop bomb is the honest interim: loud, deterministic, and
  answered by teardown machinery that already exists in RFC 0008.
- **A plain index for the set id.** Cheaper by two bytes of packing and one
  comparison, and it makes a de-registered set's slot, refilled, name a
  different set under the same number with no event anywhere. The generation
  is what `cap::Handle` already pays for the same reason, and a second
  identifier space in the same entry with weaker rules would be the odd one
  out.
- **The address in `ext` rather than over `buf_set`/`buf_index`.** Leaves the
  triple's meaning fixed across paths at the cost of a second place to look.
  Section 05 says the triple collapses, the eight bytes are already
  eight-byte aligned at offset 32, and `ext` stays the opcode's. Kept as
  written.
- **A within-buffer offset.** io_uring's fixed buffers take one. Here an
  operation names a buffer or a prefix of it, and a sub-range is a smaller
  buffer. Refused for now because the field does not exist in the entry and
  adding one under a feature bit is an RFC 0011 addition that can wait for a
  workload — see below.
- **A seventh `CapType`, or none.** RFC 0008 already treats a set as a capability,
  and `cap.rs` argues that a type added when its object arrives is a type the
  table's shape was not designed for. So `CapType::BufferSet` is added now, with
  no object behind it until `E1-B10`, on the same argument that put `Channel`,
  `Endpoint` and `Irq` there ahead of theirs.

## Consequences

Makes easy: a driver resolves a buffer name in one call, `abi::buf::Name::read`,
with the negotiated feature set in hand, and gets either a reading or the packed
refusal to post. A client cannot tear its own buffer, cannot name memory
outside the set it bound, and cannot lend one buffer twice, and learns each of
these from the compiler rather than from a service's refusal. `Batch` is a
`Submitter`, so buffers go out in a batch with no second API. `E1-B10`'s
comparison is a comparison of one thing, because the test that says both paths
hold is one function.

Makes hard: an operation over part of a buffer, which is a smaller buffer or a
later RFC. A buffer set holds its region for the life of any buffer carved from
it and is carved once, so a set is neither resized nor re-divided in place — it
is a new set, and a client that wants two geometries over one registration wants
two sets. Until `E1-B10` issues set ids, a client can bind a set with an id it
invented; the types still confine it to the memory it bound, so the failure is a
refusal from the service rather than a transfer into the wrong region, but the
sentence *a buffer not registered cannot be named* is not true of the client
alone and this RFC does not claim it is. The virtual path puts an address on
the wire, and an address is deterministic only if the allocation behind it is;
a seeded run that traces entries will show the difference, and `f_env::Env` is
where that determinism has to come from. On the registered path the service
keeps one in-flight bit per registered buffer to refuse the double submission
it cannot otherwise see; that is a cost per buffer and is written here beside
the rule it pays for. On the virtual path the service
has no registration to keep that bit against, so a client that bypasses the types
and submits the same address twice is refused by nothing — it tears its own
memory, which is its own bug and not the service's breach, and this RFC states
that rather than claiming a protection the path does not have.

Forecloses: a buffer both sides may write at once. There is no shared state
between `Idle` and `InFlight`, and a device that legitimately needs one — a fence
page, a ring inside a buffer — is not a buffer under this RFC. And it forecloses
naming a buffer by hand from a client that uses the ownership types: `submit`
overwrites `buf_set`, `buf_index` and `FIXED_BUF`, and a caller's own values
there are discarded on purpose.

## What would reverse this

- **The ergonomics failure section 11 fears, measured.** `E1-B02`'s exit counts
  copies on the data path. If drivers or their clients wrap the ownership API in
  a copy to escape the move — the count going non-zero for that reason and not
  for a hardware one — then the type shape has cost what it was meant to buy,
  and the answer is the hidden layer section 04 describes, not a weaker rule.
- **A device class that needs two writers.** One real driver that has to share a
  page writable with the device while the client also writes it is a third
  ownership state this RFC says does not exist. It would need its own RFC and
  its own type, not a relaxation of these two.
- **A runtime that can cancel from `Drop`.** When `E1-B08` gives a component a
  runtime able to submit a cancel inside a destructor and keep the buffer until
  the cancel completes, the drop bomb becomes the design's graveyard and this
  RFC's interim answer is superseded for components that run on it. The bomb
  stays for components that do not.
- **A workload that scatters within one buffer.** Then the within-buffer offset
  arrives as an RFC 0011 addition behind a feature bit, and *a sub-range is a
  smaller buffer* stops being the rule. The measurement that would show it:
  `E1-P10`'s copies-per-operation claim rising because clients copy into a
  prefix to satisfy the prefix rule.
- **`E1-B10`'s measurement finding the two paths indistinguishable** on the
  hardware this system targets. Then the naming parameter is carrying a
  distinction nothing pays for, and one path — the registered one, since it is
  the one every device can take — is the design. That would simplify the code
  and not the rule: a buffer would still be owned by one side.
