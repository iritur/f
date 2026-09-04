# RFC 0034: A modelled device is a peer on the real ring types

- Status: accepted
- Date: 2026-09-03
- Affects: `sim/`, RFC 0024, RFC 0028, RFC 0032, E1-P01, E1-P02, E1-P03,
  E1-P08, `ring/src/buffers.rs`

## Decision

The simulator's device models are **peers on the real types**. A client in
`sim/` holds `f_ring::buffers::BufferSet`, `Idle` and `InFlight`; every peer
holds a real `f_ring::registry::Table` over a real `f_ring::registry::Domains`;
and what crosses between them is a real `f_abi::Sqe` and a real `f_abi::Cqe`.
There is no simulated registration, no simulated buffer ownership and no
simulated completion type anywhere in the crate.

Four things follow from that, and each is a decision a contributor would
otherwise re-litigate:

1. **A `Message` does not carry an entry.** An ABI entry is sixty-four bytes and
   two channels in the crate need one; every other actor would pay for those
   two, and `time.rs` would grow an opinion about the ABI. So the entries live on
   a per-channel FIFO the world holds (`sim/src/wire.rs`) and the message is the
   doorbell that says one arrived. That is what a ring *is* — shared memory plus
   a doorbell — and the two are separate here for the same reason they are
   separate in `f_ring`.

2. **A client's buffer region is leaked at `'static`.** A `BufferSet` owns its
   region for `'m` and `carve` borrows the set for the same `'m`, so a set and
   the buffers carved from it cannot both be fields of a value that moves —
   which every actor is. `Box::leak` is safe Rust and it models the truth: a
   component's buffer region is granted for the life of the component and never
   handed back. RFC 0008 reclaims it on the real path; the run ending reclaims it
   here.

3. **A component's own ending is evidence its outstanding buffers are void.**
   `InFlight`'s drop is a bomb, and rightly: a *live* component that abandons a
   buffer the device is writing into is the bug RFC 0024 exists to make
   unwritable. `sim/src/client.rs`'s `Drop` is the other case — the component is
   what is ending — and RFC 0008 says the frame revokes its buffer sets and
   tears down its IOMMU domain when it does, which is exactly the condition
   `PeerGone` attests to. So the buffers are reclaimed there rather than dropped,
   and a run that ends with work outstanding reports `Trouble` instead of
   panicking with a message about a bug nobody wrote.

4. **A device that loses a completion resets.** This one is a *finding*, not a
   preference — see below.

## Context

RFC 0032 settled the shape: the simulator models the system above the frame and
does not build, link or execute any part of `kernel/`. It did not settle what the
models are made of, and there were two answers.

The cheap one is a parallel set of types: a simulated buffer handle, a simulated
registration table, a simulated completion. It compiles anywhere, has no lifetime
problems, and is what most simulators do.

It is also the failure RFC 0032 rejected shape (b) to avoid, arrived at from the
other side. A model of the buffer rules would check that the model agrees with
itself; the whole value of RFC 0024 is that the *compiler* refuses the misuse,
and a parallel type would have to reproduce that refusal in order to be checking
anything — at which point the thing being checked is the reproduction. Worse, the
refusals the real types give are the interesting ones: a set id no table issued,
an index past the set, a length past the buffer, and a buffer the device already
holds. That last is the double submission, which an ordinary harness cannot see
and a seeded reordering finds, and it exists only because
`f_ring::registry::Table` keeps a `lent` bitmap.

The cost of the real types is the lifetime arrangement in points 2 and 3, and it
was paid rather than routed around.

What is *not* real is the transport. `f_ring::Producer` over a real `Mapping`
would need one allocation two actors both hold, which in safe Rust is a second
shared-memory model beside the queue's, and it would buy fidelity this stage
cannot spend: hostile cursor values are E1-P04's and a torn publish is E1-P02's,
and both want the channel header rather than a queue of entries.
`f_ring::buffers::Submitter` exists to be stood in for — its own documentation
says a test can put a recorder behind it and exercise the ownership rules with no
ring at all — and `wire::Post` is that recorder with a timeline behind it.

## Consequences

**The models refuse what the system refuses, without anyone writing the
refusals.** Generation arithmetic, the speculation mask, the `lent` bitmap and
the `Fixed::from_completion` fence all apply to a simulated client for free, and
a change to any of them changes the simulator's behaviour — which is the right
coupling and the reason the crate depends on `f-ring` at all.

**A device that loses a completion must reset, and the model says so.** This is
what driving the real types found, and it is worth stating as a consequence
rather than a curiosity. RFC 0024 gives a client exactly one way to take a buffer
back without a completion — `PeerGone`, built only from evidence that every
outstanding token is void. There is no timeout in that design and no way for a
client to give up on its own. So a device that lost a completion and carried on
would leave its client holding memory it can never touch and never free: a hang,
and a quiet one. The models therefore do what a real device does when its queue
state and its driver's have come apart — retire the registrations, take the
translations with them so an already-started transfer faults rather than lands,
and tell the client. `E1-P02`'s *peer death mid-operation* is this path with a
different cause, and it is already built.

**`f_ring::buffers::PeerGone` wants a second constructor.** `PeerGone::of` takes
a `RingError`, so point 3 above passes `EpochChanged` to state a fact that is not
a ring error. It is sound — the component is ending, and RFC 0008's revocation is
what makes it sound — and the type would rather say so directly. That is a
finding for whoever owns `ring/src/buffers.rs`, recorded here rather than worked
around silently in `sim/`.

**A simulated peer can be wrong about the ABI in a way a simulated type could
not.** That is the point and it is also the cost: a change to `f_abi` that the
models do not follow shows up as a scenario failing rather than as a compile
error, and the failure will name a digest rather than a field. The mitigation is
that the digests are cheap to regenerate and a commit that changes them is
visible; the alternative — models that could not be wrong — is models that check
nothing.

**Every scenario's digest depends on `f_ring` and `f_abi`.** A commit to either
that changes what a table issues or what an entry carries changes every simulated
run's hash. That is correct — a `(seed, commit)` pair names a run at a commit —
and it is why the seed corpus E1-P03 accumulates is bound to a commit rather than
to a scenario name.

## What would reverse this

**A device model whose protocol the real types cannot express.** The concrete
shape: a device that needs a buffer named in a way `f_abi::buf::Name` has no
variant for — scatter-gather across two registrations is the candidate — where
the honest choices are to extend the ABI or to model the naming separately. The
first is the right answer and this RFC stands; the second would mean the crate
had grown a parallel type after all, and the reversal is then to say so in an RFC
rather than to let one appear a field at a time.

**A measurement that the lifetime arrangement costs more than it buys.** The
leak in point 2 is bounded — one region and one set per client per run — and a
sweep of a million seeds runs a process per seed, so it is bounded per process
too. If E1-P03 ends up running many scenarios inside one process and the leak
becomes the reason, the fix is an arena the run owns and hands out `&'run mut`
slices from, which changes `App`'s two `Box::leak` calls and nothing else. Worth
recording as the migration rather than discovering it under a memory limit.

**Point 3 stops being true.** If `ring/src/buffers.rs` grows the constructor its
finding asks for, `sim/src/client.rs`'s `Drop` uses it and this RFC's point 3
becomes a sentence about history. That is the good outcome and it needs no
reversal, only an edit.
