# RFC 0028: Registration is an operation, and its state is not shared

- Status: accepted
- Date: 2026-09-03
- Affects: `abi/src/buf.rs` (the `opcode` pair and `Request`), `abi/src/lib.rs`
  (`Sqe::buf_set`'s reading on a registration entry), `ring/src/registry.rs`,
  `ring/src/buffers.rs` (`Fixed` loses its public constructor);
  `docs/rfc/0024-a-buffer-is-owned-by-one-side.md`, which delegated the entry's
  layout and its opcode to `E1-B10`; `E1-B01`, which supplies the two IOMMU
  seams this defines, and `E1-B02` and `E1-B03`, which are the first services
  to answer on the opcodes

## Decision

RFC 0024 said what a registration *answers with* — a `SetId`, sixteen bits of
slot and sixteen of generation, and the refusals a bad one earns — and left
three things to the task that built it. This is those three.

**The entry, and the opcodes.** A registration is an ordinary submission.
`abi::buf::opcode::REGISTER` is `0xFE` and `UNREGISTER` is `0xFF`; the entry's
`cap` names the memory, `len` is the extent in bytes, `ext[0]` is the buffer
count, and every other field the opcode does not read must be zero and is
refused when it is not. `FIXED_BUF` is *clear* on a registration, because
nothing is registered yet and so the entry names no set, and *set* on an
unregistration, because `buf_set` is then exactly what the flag says it is.
The answer is a `SetId` in `Cqe::ext`, read by `SetId::from_completion`.

`Request::read` checks the *envelope* as well — the reserved word, then the
undefined flag bits, then the opcode, in `f_ring::execute`'s order and for
`f_ring::execute`'s reason. It has to, and this is the one thing about the entry
that is not obvious: a service dispatching on `opcode::is_registration` reaches
this function **instead of** its own executor, not after it, so an envelope
checked only in the executor is an envelope not checked at all on the one entry
that hands out an authority. The list of defined flags therefore moves to
`f_abi::flags::KNOWN`, where both readers of an envelope see the same one; two
lists would be one entry that is malformed on one path and legal on the other.

Two numbers, reserved across every service's opcode space, in the wire crate.
Section 05 says the opcode space is per-service and not global, and this is the
one operation that cuts across all of them: every service that takes a buffer
needs registration, none of them invented it, and a client that has learned to
register with one has learned to register with all. The alternative is a
number agreed per service, which means a client that must be *told* which entry
to write for each peer — and being told is a registrar, which is the global
namespace this system does not have. The top of the byte, because services
number their own opcodes upward from zero, so the two highest values are the
part of the space anything reaches last. Reserving a number is not offering the
operation: a service that does not register refuses these with
`ARGUMENT`/`UNKNOWN_OPCODE` like anything else it does not know, and the
frame's own `op::known` deliberately does not admit them.

**Where the registration lives.** In the service's own memory, in
`f_ring::registry::Table`, and nowhere else. Nothing about a registration — not
the slot, not the generation, not the in-flight bits — is written into the
shared region. `E0-B15` made the same call about the doorbell counts and the
sentence carries: evidence a peer can forge is not evidence. Here it is
stronger than a measurement problem. A generation a peer can write is a peer
that can un-revoke its own retired set, which turns the whole
`AUTHORITY`/`REVOKED` refusal into a suggestion.

The generation in a slot **retires rather than wrapping**, at
`SetId::RETIRED_GENERATION`, and a slot that reaches it is never filled again.
This is not a new decision; it is the one `abi/src/cap.rs` already made for the
handle a `SetId` is packed like, and failing to copy it would have been a
reversal of that decision by omission. RFC 0024 paid two bytes for a generation
so that a refilled slot could not name a different set under the same number; a
counter that wrapped would hand that failure back after 65 535 registrations of
one slot instead of after one — silently, and with a retired id resolving into
whatever memory occupies the slot now. Retirement converts a soundness hole into
running out of slots, and running out of slots is a thing a peer can be told:
`RESOURCE`/`QUOTA_EXHAUSTED`, the same refusal a full table already gives, with
the same detail, because *there is no slot for you* is what a peer must act on
and how the slot was spent is not its business. `Table::retired` reports the
cost, as `kernel/src/cap.rs` does for the same reason.

Every index a peer wrote is bounds-checked **and then masked** before it reaches
an array. The check is the refusal a correct peer gets; the mask is what a
mispredicted branch gets. RFC 0005 says a boundary the hardware speculates
through is not a confidentiality boundary, and a slot number confined only by a
branch is precisely such a boundary. The table's slot count is therefore a
power of two, so the mask is one `AND` rather than a clamp with a longer
dependency chain — the same requirement, for the same reason, that the ring
already places on its entry count.

**The seam the second path hangs from.** The registered path asks the frame for
a translation (`registry::Domains`); the shared-virtual-memory path asks whether
the device reaches an address by walking the submitter's page tables
(`registry::PageWalk`). Both are traits with no implementation in this epoch,
because `E1-B01` is the IOMMU and there is no hardware under either.
`registry::Transport` is what the two paths differ in on the service side —
exactly as `buffers::Naming` is on the client's — so one test body drives both,
and `registry::path::ALL` is counted by that body so a path declared and never
exercised is a failure rather than a silence.

And one consequence in the client's types: `buffers::Fixed` loses its public
constructor. Its only source is `Fixed::from_completion`, and the only source of
a completion carrying an id is `Table::issued`, which takes `&self` because the
table is the witness — an id its table does not hold earns the refusal
`slot_of` gives it.

**What that is worth is less than it sounds, and the RFC says so rather than
letting the code imply it.** It removes the one-expression forgery
(`Fixed(SetId::new(0, 1))`) and it does not remove forgery: a client can stand
up a `Table` of its own, register into it, and read its own answer back. Three
lines instead of one, and still a naming no service issued. RFC 0024 recorded
the invented id as the honest boundary of its own claim; this narrows that
boundary and does not move it. The claim that survives is the one that was
always doing the work — the id names a slot in *the service's* table or it names
nothing — and the compile error is a fence in front of the accident, not the
check. A version of this that actually closed the gap would need the id to be
unforgeable rather than merely inconvenient to forge, which means a capability
and not a number, and that is a different and much larger design.

## Context

RFC 0024 named `E1-B10` four times and left it the parts that need code to
settle. What was actually open when this was written:

- **Whose opcode space.** RFC 0024 says *a registration entry in that service's
  opcode space*, which is true and does not say which number. Left per service,
  the client's registration path becomes per peer.
- **Where the table lives.** Nothing said. The obvious wrong answer — the four
  reserved words of `ChannelHeader` — is obvious because it is free, and it is
  the same free that `E0-B15` refused for the doorbell counts.
- **What the shared-virtual-memory path is written against**, given that no
  device this epoch can boot has address translation services. RFC 0024 requires
  both paths to ship so that the measurement compares one thing; it does not say
  what the second one calls when there is nothing to call.

The alternatives that were live:

- **One opcode with a direction flag**, rather than two. Cheaper by a number and
  worse to read: an unregistration and a registration disagree about almost
  every field, and a flag that changes which fields exist is an opcode wearing a
  bit's clothes. The flags byte is also where `FIXED_BUF` already changes a
  field's meaning, and a second such bit in the same byte is how an envelope
  stops being an envelope.
- **A per-service opcode, agreed at setup.** Rejected above. Worth stating the
  cost of the choice made instead: two values of every service's opcode space
  are gone, forever, including for services that never register anything. A
  service that legitimately needs 254 opcodes is the reversal condition.
- **The set id in the completion's `result` rather than `ext`.** `result` is
  thirty-two bits and a `SetId` is thirty-two bits, so it fits exactly — and it
  fits by having no room for the sign that distinguishes a refusal, which is
  the one property `result` must keep. Not considered further.
- **A registration that takes effect without a completion**, with the client
  computing the id it was going to be given. That is a registrar again, in the
  client this time, and it makes the service's table a mirror of a client's
  arithmetic rather than the authority. The whole defence is that the id names a
  slot the service filled.
- **A generation that wraps.** One instruction cheaper and the only alternative
  that was ever a soundness question rather than a taste question. Rejected for
  the reason `abi/src/cap.rs` states in one sentence — a counter that wraps is a
  stale name that becomes valid again — and it is worth recording that the first
  draft of this task wrapped, that every test in it passed, and that the failure
  needs 65 535 registrations of one slot to appear. A property whose
  counterexample is that far out is a property no test finds by accident, which
  is the argument for taking the precedent rather than re-deriving it.
- **Keeping `Fixed`'s public constructor** and leaving the invented-id case to
  the service's refusal, as RFC 0024 wrote it. That was correct while nothing
  issued ids. Keeping it once something does would mean the type system knows
  where an id comes from and declines to say so.
- **A per-buffer in-flight bit on the virtual path too**, so the two paths refuse
  the same double submission. There is nothing to hang it on: the service has no
  registration on that path and an address is not an index into anything it
  holds. Inventing a side table keyed by address would be registration under
  another name, on the path whose entire point is not having one. RFC 0024
  already states the consequence and this RFC does not soften it.

## Consequences

Makes easy: a driver holds one `Table` and answers registration without knowing
which path its clients use; a client registers once and never names a set id it
was not given. The comparison `E1-B10` owes — pinned-and-registered against
shared-virtual, same workload — is a comparison of one thing, because both
paths run through one `Transport` and one test body.

Makes hard: a service with more than 254 opcodes of its own. A set larger than
`registry::BUFFERS_MAX`, which is 64 because the in-flight bits of a set are one
machine word; a client wanting more buffers wants more sets, which is RFC 0024's
own sentence with a number in it. A channel whose sets turn over more than
65 534 times per slot, which is the same kind of cost as `BUFFERS_MAX` and has
the same shape of answer: the table is one slot smaller, and `Table::retired`
says how many. And a registration table that grows: there is
no allocator here, so a service that runs out of slots refuses with
`RESOURCE`/`QUOTA_EXHAUSTED` rather than committing memory a peer chose the size
of — the argument `E1-B13` makes about the capability table, one size down.

Forecloses: a registration a peer can observe or influence the bookkeeping of.
There is nothing in the shared region to read, so there is nothing to race, and
a peer's only influence on the table is the entries it submits.

The honest cost, stated where the number will be: on this machine the
shared-virtual-memory path is a range comparison standing where a page walk will
be, so `claims/0004-buffer-registration-cost.toml`'s per-path ratio is not a
result. It is `pending` for that reason and the workload's own module docs say
which half will move when `E1-B01` lands.

## What would reverse this

- **A service that needs 254 opcodes.** Then the top of the byte is not free
  and registration moves — most plausibly to a negotiated opcode base carried in
  the channel header, which is an RFC 0011 addition behind a feature bit and
  reintroduces exactly the per-peer client this RFC refused. Worth doing at that
  point and not before, because the cost only exists once something pays it.
- **A device that can report which of its outstanding transfers touched an
  address.** That would give the virtual path something to hang an in-flight bit
  on, and the asymmetry this RFC declines to soften would stop being inherent.
  The test that would notice: `the_virtual_path_cannot_refuse_a_double_submission_and_says_so`
  is written to fail on the day it becomes false.
- **`E1-B10`'s measurement finding the two paths indistinguishable** on real
  hardware, which is already RFC 0024's reversal condition. Then `Transport` is
  carrying a distinction nothing pays for and one implementation is the design.
  What survives either way is where the table lives: that is an argument about
  forgery, not about cost.
- **A table that actually retires slots in service.** `Table::retired` is what
  would show it, and it is reported so that this arrives as a number rather than
  as a capacity bug. The fix is a wider generation field, which is an ABI change
  and an RFC of its own — never a wrap, because the wrap is the failure the
  generation exists to prevent and re-introducing it under load is
  re-introducing it exactly where it does most damage.
- **A second registration-shaped operation** — registering an interrupt, a
  completion group, a scheduling reservation — reaching for the same trick of
  reserving numbers at the top of every service's space. Two is a pair; three is
  a shadow opcode space with no owner, and at that point the right answer is a
  namespace and not a third pair of constants.
