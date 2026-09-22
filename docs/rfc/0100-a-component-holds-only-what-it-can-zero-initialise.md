# RFC 0100: A component holds only what it can zero-initialise

- Status: accepted
- Date: 2026-09-22
- Affects: `scene/src/arena.rs` and `scene/src/kind.rs` (the sentence this
  reverses), `user/compositor/`, and the next three crates a component will link
  — `f-text`, `f-input`, `f-interface`

## Decision

**A data structure a component holds must be all zeroes when it is empty, or it
is paid for twice — once in the component's image and once in its heap.** The
rule is not a style preference and it is not about saving bytes in the abstract:
a component in this tree is an image the frame maps into a fixed reservation,
and a constant with one non-zero byte in it cannot be emitted as a zero
initialiser, so the whole of it lands in `.rodata` and is copied out at run time.

`scene/src/arena.rs`'s own doc said a `static` is *where a component's scene
actually lives*. **That sentence is withdrawn.** A component may hold no writable
`static` at all, so the scene reaches it through a `Box`, and the constant behind
that `Box` is the thing this RFC is about.

## Context

`E3-B01f` linked `f-scene` into a component for the first time —
`user/compositor`, the first thing in this epoch that is not a library nothing
runs. `Arena::EMPTY` is 131 096 bytes. The first build of the component was
**147 280 bytes against the 65 536 the frame maps**, and `cargo xtask component`
refused it.

Every byte of the difference was one enum's niche. `Slot` holds an
`Option<Created>`; `Created` holds a `Kind`; a six-variant enum numbered from
zero leaves its niche *above* the range, so `None` is encoded as a non-zero
value, so `Slot::EMPTY` is not all zeroes, so `Arena::EMPTY` is 131 096 bytes of
constant data that has to exist somewhere. It existed in the image.

The repair is to number `Kind`'s variants **from one**, which puts the niche at
zero, which makes `Slot::EMPTY` and therefore `Arena::EMPTY` all zeroes, which
makes `Box::new(Arena::EMPTY)` an allocation and a `memset`. The image is 15 992
bytes.

**The repair is not a trick played for a byte count, and that is the part worth
keeping.** `#[repr(u8)]` with the discriminants set to the wire constants means
the number a kind is on the wire and the number it is in the enum are now **one
value rather than two with a `match` between them**: `Kind::wire` reads the
discriminant instead of mapping to it, and a kind whose two numbers disagreed is
no longer expressible. What it costs is one subtraction in `Kind::index`, which
says so in its own doc comment and cannot underflow because `abi::scene::kind`
numbers from one and a `const` block walks every `u16` to establish it.

The alternative was live and was refused: raise the frame's reservation for this
component. That buys one component a bigger image and loses the finding. Three
more crates — `f-text`, `f-input`, `f-interface` — are queued to be linked into
components in this epoch, each with tables of its own, and a reservation raised
once is a reservation raised every time.

## Consequences

**What this makes easy.** A component's heap cost and its image cost stop being
the same number. A structure that zero-initialises is a `memset` at run time and
nothing at all at rest.

**What this makes hard, deliberately.** A sentinel that is not zero — an
`Option<NonZeroU32>` used the other way up, an enum whose first variant is
meaningful, a `[T; N]` whose `T::EMPTY` carries a tag — is now a decision with a
cost attached rather than a detail. The cost is visible at the moment it is
incurred, which is the whole point.

**What nothing in the permissive tree can check.** No crate forbidding `unsafe`
can read its own byte pattern, so `f-scene` cannot assert that `Arena::EMPTY` is
zeroes. **What holds the property is `cargo xtask component` refusing an image
larger than the frame's reservation, with the byte count in the message.** That
is a weaker check than an assertion and it is the honest one: a toolchain that
placed the niche elsewhere is a red build naming the number, and not a silent
regression. `scene/src/kind.rs` and `user/compositor/src/lib.rs` carry the two
halves of that sentence where a reader of either will meet it.

## What would reverse this

**A component that must hold a structure with no zero representation.** The
candidate is already named: a shaping cache keyed by string, face, size and
features (`E3-B03c`) may want a tombstone that is not zero, and a coverage atlas
(`E3-B03g`) has residency bits that are meaningful when set. If either turns out
to need a non-zero empty state, the answer is not to bend the type — it is to
say so here and to pay the image cost with the number written down.

**A frame that maps a component's image lazily**, so that `.rodata` that is
never read costs address space rather than a reservation. That would make this
rule an optimisation rather than an admission criterion, and the check that
holds it today — an image bound — would stop being the right instrument.

The observation that would say this RFC is simply wrong: a build in which
`Arena::EMPTY` is all zeroes and the image is large anyway. That would mean the
niche was never what cost the 81 KiB, and the measurement this whole entry rests
on was a coincidence.
