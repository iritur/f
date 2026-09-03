# RFC 0037: A channel is adopted for a call, not for a lifetime

- Status: accepted
- Date: 2026-09-03
- Affects: `ring/` (new: `adopt.rs` — `Adopted`, `Client`, `Server`, and the
  three `pub use`s beside `Mapping`; `mapping.rs` gains one `pub(crate) unsafe
  fn bound`), `user/store/` (new: `runtime.rs`, `report.rs`, a dependency on
  `f-ring`, and an `image` feature), `user/init/link.ld` (`.got` stops being
  expected to be empty), `xtask/src/main.rs` (`flat_image` links what cargo
  says it built, and builds a component with `panic=immediate-abort`),
  RFC 0033 (whose deferred half this is), RFC 0008 (whose control ring a
  component can now drive), RFC 0014 and RFC 0015 (whose reversal conditions
  this makes payable), RFC 0030 (whose named deferral this closes); `TODO.md`
  tasks E1-B08, which this is half of, and E1-B05, whose wall this is

## Decision

**A component adopts a channel for the length of one call, and believes its
header exactly once.**

`f_ring::adopt::Adopted::at` is a safe function taking a raw address and a
length. It runs the validation `f_ring::Mapping::adopt` already performs —
alignment, a header copied out with one volatile read, `ChannelHeader::negotiate`,
`Layout::adopt` — and keeps the four words that came out: the base, the layout,
the negotiated set and the peer's epoch. It holds **no reference into the shared
region and hands none out.** Every operation on it builds a `Mapping` over the
stored layout, uses it, and drops it before returning.

A crate that forbids `unsafe` can therefore drive a control ring and a data
ring. `user/store::runtime` is the first thing that does, and RFC 0038 is the
scheduler it needed to be worth doing.

### Why this is not RFC 0033's argument used a second time

RFC 0033 gave a component a safe accessor for a device's registers and its own
DMA region, and refused to give it one for a channel, in a sentence worth
repeating exactly: *a device window has one writer on this side, and a channel
is shared with a peer that may be hostile.* That refusal was correct and this
RFC does not overturn it. What it does is take the obligation apart.

`Mapping::adopt` is `unsafe` for two reasons that look like one:

1. **The region is mapped.** `base` names `len` bytes the frame mapped for this
   component in answer to a capability it holds. This is a *contract*, it is the
   one `f_abi::state::Reader::at` and `f_ring::device::Region::at` already keep,
   and its failure mode is a page fault at ring 3 — the defined machine outcome
   `cargo xtask user` is seven boots of.
2. **Nothing else holds a reference into the range.** This is the one a window
   does not have, and it is why `Mapping::at` would have been wrong.
   A `Mapping` hands out `&Cursor`, `&[AtomicU32]` and `&[UnsafeCell<Sqe>]` —
   real Rust references, borrowed from `&self`, which the language requires to
   point at live memory for their whole lifetime. A component holding one across
   the moment its supervisor revokes the channel holds a reference into unmapped
   memory, which is undefined behaviour rather than a fault. A `Window` hands
   out nothing at all: every access is a fresh volatile operation, so a revoked
   window is a page fault at the next touch and nothing worse.

Obligation 2 is discharged **structurally** here rather than promised: the
`Mapping` built inside `Adopted::at` dies before that function returns, and every
accessor builds one and drops it inside its own call. Between two calls there is
nothing to dangle; inside one there is a page fault if the mapping went. That
reduces a channel to exactly the contract RFC 0033 already accepted for a
window, and it is the whole of why this is a different argument and not the same
one told twice.

### Believing once, and re-checking always

The layout is read from the peer's header **once** and never again. That is the
answer to the hostile peer, and it is the opposite of what per-call re-adoption
would do: a peer that rewrote the header between two calls could otherwise move
the entry array under a component midway through a drain — a bounds check that
bounds nothing, which is the failure this tree keeps catching in its own work.

What *is* re-checked on every access is everything a peer can still move: the
cursors, the slot numbers in the index ring, the occupancy. That is
`Producer`, `Consumer`, `Poster` and `Collector` doing what they already did.
`Mapping`'s own sentence survives unchanged — *binding is the point past which
the arithmetic is known to describe the bytes, not the point past which the
bytes are known to be friendly* — and this RFC adds one: **believing happens
once and binding happens per call.**

`Mapping::bound` exists for that and for nothing else. It is `pub(crate)` and
`unsafe`, and its obligation is that the layout it is handed is the one
`Mapping::adopt` computed for those bytes.

### Two roles, because one end is not both

`Adopted::client` submits and reaps; `Adopted::server` drains and answers. The
split is the single-producer single-consumer discipline the whole protocol rests
on, made a type rather than a paragraph — the same reason `Service` holds one
side of a channel and not both.

A component that genuinely is both ends of one region adopts it twice and says
so in two values. A runtime's executor is exactly that, and `Mapping`'s own
safety note already permits it in the sentence it was written for: *two ends
sharing a region is the intended use, not a violation of it*.

## Context

What was true when this was decided.

`E0-B13` recorded the wall, `E1-B05` recorded it again, and RFC 0033 narrowed it
by half. A component inherits `unsafe_code = "forbid"` — enforced by the
compiler, backstopped by `cargo xtask lint-unsafe` — so it cannot dereference
anything. Driving a ring means adopting a mapped channel, and `Mapping::adopt`
is `unsafe`. The consequences were three, all of them recorded rather than
discovered: RFC 0008's restart policy ran in the frame, where that document says
it does not belong; `ANNOUNCE`, `PROGRESS` and the four capability calls stayed
on the door although RFC 0014's and RFC 0015's reversal conditions had fallen
due; and `kernel/src/blk.rs` called `Driver::execute` where a scheduled
component would call it from its own polling loop.

The alternatives were live and each loses for a different reason.

- **Make `Mapping::adopt` safe.** The obvious answer and the wrong one. It hands
  out references with `&self` lifetimes into memory the frame can unmap, so a
  safe constructor would make *holding* a `Mapping` across a revocation into
  undefined behaviour rather than a fault — and a component cannot know when its
  supervisor revokes a channel. It would also be RFC 0033's argument applied to
  a case that document explicitly excluded, which is the move R01 exists to
  refuse.
- **Rebuild the whole binding on every access.** Sound, and it hands a hostile
  peer the ability to move the layout under a live drain. It also pays a
  negotiation on the hot path, which is the one path this architecture exists to
  keep short.
- **A macro in `abi/` that expands to `unsafe` in the component.** Rejected for
  the reason RFC 0033 rejected it: `cargo xtask lint-unsafe` is textual, a macro
  is how a property enforced by grep stops being enforced, and the resulting
  `unsafe` would live in a crate nobody reviews as the frame.
- **A syscall per ring operation.** This is what the door already is, and the
  entire architecture is the claim that the hot path does not cross it. It would
  also make the exit criterion of E1-B08 unmeetable by construction, which is a
  useful way to see that it is the wrong answer.
- **Leave it, and keep the supervisor's policy in the frame.** What E1-B05 did,
  for one task, deliberately. Keeping it would mean the component model is a
  document rather than a mechanism, and RFC 0033's own reversal condition says
  so: *if E1-B08 lands and `kernel/src/blk.rs` still calls `Driver::execute`,
  then* a channel is not a window *was a reason to defer rather than a
  distinction.*

## Consequences

**Easy.** A component drives its own rings. `user/store::runtime` is 200 lines
of ordinary safe Rust that adopts a control ring, adopts its own work ring
twice, and drains notices at a polling point — and it contains no `unsafe`,
which is a property of the build rather than of the review. RFC 0008's notice
path finally has a component *acting* on a notice rather than the frame draining
one on its behalf, which is the gap `component::demonstrate` wrote down and
could not close. Five host tests in `ring/src/adopt.rs` drive both ends of a
channel, a notice arriving at a polling point, a scribbled header being refused,
and an address no mapping can be stated against.

**Hard, and paid in the build rather than in the design.** A component that
links `f_ring` is no longer one archive on a linker command line, and three
things fell out of that, each of which is now written down where it happened:

- `xtask::flat_image` links what cargo says it built, read out of
  `--message-format=json` rather than found by walking a directory. The old
  comment — *one library and nothing else, which is a claim about the component*
  — was true and stopped being true, and the linker said so with an undefined
  symbol, which is the failure mode that made it safe to state.
- A component's image is built with `panic = immediate-abort`. The formatting
  machinery a panic message needs is several kilobytes bought for a string
  nobody can read: a component has no serial port, no unwinder, and a
  `#[panic_handler]` whose whole body is a halt. With the strategy, a panic is
  `ud2` — an invalid-opcode fault at ring 3 that the frame reports with a
  vector, an address and an instruction pointer, and that RFC 0008 already names
  as one of the three ways a component ends. Strictly more information, in a
  smaller image.
- `user/init/link.ld` stops expecting `.got` to be empty. Eight bytes of
  linker-resolved addresses appear, `lld` gives their output section the
  writable flag whatever the script says, and the image's own end marker landed
  in it — so `cargo xtask component` reported `__image_end` as writable data.
  The two boundary markers are excluded by name and the check keeps its whole
  meaning: `.data` and `.bss` must still be empty, and a real mutable global
  still has a name of its own.

**The honest cost, and it is a number.** The `store` image is 3904 bytes of the
4096 the frame maps — 192 bytes of headroom for the first component that drives
a ring, and one of those bytes was bought back by deleting a redundant
occupancy read. A component's text is one page: `xtask`'s `INIT_MAX`, with
`kernel::process::GUARD` at the next one. That is a real bound and it will be
hit, and what stands between it and a mysterious boot is that `cargo xtask
component` refuses the build and names the number.

The answer when it is hit is a loader that maps as many pages as an image's
headers ask for — E5, and a change to the frame's layout rather than to this
decision. What is *not* the answer is trimming a driver to fit, and the
observation to act on is the first component that cannot be written honestly in
the space.

**Forecloses.** A component holding a live `Mapping` across a call. A component
that re-reads its channel's header. And the argument that `unsafe` has to be
widened for a driver to exist: the frame's partition is unchanged, every
`unsafe` block behind this is in `ring/`, and `cargo xtask unsafe` measures the
same three crates it measured before.

## What would reverse this

**A per-call binding that shows up on the hot path.** The whole point is that a
runtime's work never crosses a boundary; if rebuilding a `Mapping` per operation
turns out to cost enough to show up in E1-P10's ring-submit claim, then the
right answer is a binding that lives longer and a rule about when a component
must drop it — a scoped guard rather than a per-call rebuild — and this RFC's
structural discharge of obligation 2 becomes a rule somebody has to keep. The
measurement is `claims/0001-ring-submit-latency.toml` collected through
`Adopted` rather than through `Mapping`, and the number to compare is the one
the frame's own side already publishes.

**A component that needs to hold a reference into a channel.** Nothing here
does, because nothing here needs a slice. If a component appears that genuinely
must — a zero-copy parser over the arena, say — then the accessor is the wrong
shape, and the answer is a scoped borrow whose lifetime the type enforces rather
than an escape hatch. The observation is a `user/` crate acquiring a `[lints]`
override, which is the same diff RFC 0033 named and the same one to catch.

**A revocation that is not a page fault.** This rests on the frame unmapping a
revoked channel, so that a component reaching into one dies at the hardware. If
a path ever appears where a channel is revoked and the *mapping* stays — a
lazily-torn-down teardown, a reclaim that only withdraws the capability — then
between two calls there is memory a component may still touch and a peer may
have reused, and the contract's failure mode stops being the defined one.
`kernel/src/smp.rs`'s shootdown is what makes it a fault today, and a batching
change there (`E1-B14`) is where to look.

**A second way into a channel that the `unsafe` boundary does not see.**
`lint-unsafe` is textual and `Adopted::at` is safe. If a helper ever appears in
`ring/` that yields a slice into a mapping, or a `Mapping` handed across a call
that a component can hold, then the property is being kept by review again. The
answer is not a lint clause; it is making the accessors take a closure so no
value escapes, paying the `lint-callbacks` argument that costs. The observation
is a finding this arrangement did not catch, which is a thing to write down when
it happens rather than to argue about now.
