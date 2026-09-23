// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Rung 1, stage 1: the marked subtrees flattened into a linear byte encoding
//! of paths, transforms and paints.
//!
//! Section 08 of `docs/design/ring-scene-boot.html` splits a frame into four
//! stages, and `interface/src/ladder.rs` names the renderers that run them.
//! Stage 1 is the only one of the four that is arithmetic on the CPU rather
//! than on the GPU, which is why it is the only one of rung 1 that can be
//! written and checked on a machine with no GPU at all — every other stage of
//! that rung needs a device to observe, and this one needs a byte comparison.
//!
//! # What it produces, and why bytes rather than a slice of records
//!
//! A flat little-endian byte stream: a header, then one record per node in the
//! order [`Sealed::walk`] handed them over, each record carrying the node's
//! identifier, its depth, its kind and whichever of its three property records
//! a delta has actually set. Stage 2 reads this and nothing else — it does not
//! get the graph — which is what makes *the encode stage is where the traversal
//! stops* a fact about the program rather than a discipline.
//!
//! Bytes rather than `&[SomeRecord]` because the exit clause is *byte-identical
//! on both architectures*, and a slice of Rust structs is not a byte image: its
//! padding, its field order and its alignment are the compiler's to choose, and
//! `#[repr(C)]` would only move the choice rather than remove it. Every integer
//! that reaches this stream goes through `to_le_bytes` on a type whose width is
//! written into its name — `u16`, `u32`, `u64`, `i64` — so the stream's content
//! is a function of the values and of nothing about the machine. **No `usize`
//! is ever written**, which is the one width that differs between the two
//! architectures this workspace targets and therefore the one mistake this
//! module could make that a single-architecture test would not see.
//!
//! What carries that clause in evidence is two things and neither of them is a
//! local run: `the_encoding_of_one_scene_is_this_byte_image` compares a whole
//! encoding against a hand-written literal, which is `abi/src/scene.rs`'s
//! device and is where a layout-dependent encoder fails; and the AArch64 half
//! of `tests (AArch64, weak memory)` is the runner that executes it on the
//! other architecture, because no hosted AArch64 binary runs in the development
//! container — `ARCH_RUN_GAP` in `xtask/src/main.rs` says so, and this module
//! inherits that gap rather than pretending to close it.
//!
//! # Only what was marked
//!
//! [`Encoder::node`] takes a [`Visit`] and nothing else. `crate::dirty` states
//! exactly what that buys and where it stops: a `Visit` carries no arena, so a
//! function whose only parameter is one cannot reach a node it was not handed,
//! and *the encoder visits only what the commit marked* is therefore a property
//! of this signature rather than of a convention. That sentence is the reason
//! [`encode`] exists as a free function and is three statements long: it is the
//! whole of the code in this crate that holds a `Sealed`, an `&Arena` and an
//! encoder at once, it holds the arena only long enough to hand it to
//! [`Sealed::walk`], and it is short enough to read. *What would reverse this:*
//! a stage that needs a node's parent's transform — composition down the tree —
//! at which point either `Visit` grows that answer or this signature does, and
//! the second of those is the one that costs the guarantee.
//!
//! The count is checked from the far side as well. [`Encoder::nodes`] is
//! incremented in [`Encoder::node`] and is therefore the number of records the
//! encoder *wrote*, not the number the walk said it handed over, so a test can
//! compare two numbers that were produced by two different pieces of code. The
//! attack that matters is that an encoder which walked the whole graph would
//! produce the same bytes as a correct one for a scene in which everything is
//! dirty, so the tests below encode a scene of a thousand nodes in which a
//! hundred and one are marked and assert both the count and the identity of
//! every node in the stream.
//!
//! # Why the buffer cannot overflow, and what it costs
//!
//! [`ENCODING_MAX`] is [`HEADER_BYTES`] plus [`NODES_MAX`] times the widest a
//! node record can be. `Sealed::walk` stops at [`NODES_MAX`] nodes — its
//! `halted` is that stop — so a walk cannot hand this encoder more records than
//! the buffer has room for, and [`Encoder::overflowed`] is a field that a legal
//! frame cannot set. It is still a field, and still written into the header's
//! flags, because *unreachable* is an argument about two constants and a loop
//! bound in another module, and an argument is worth less than a byte that says
//! the frame is not presentable.
//!
//! The cost is 82 KiB of buffer, which is the same order as `Arena::EMPTY`'s
//! 131 KiB and is paid the same way: [`Encoder::EMPTY`] is all zeroes, so a
//! component reaching it through a `Box` gets a zero initialiser rather than 84
//! KiB of image. RFC 0100 is that rule, and `cargo xtask component`'s 65 536
//! byte refusal is what enforces it — nothing in this crate can read a value's
//! bytes, because that needs `unsafe` and this crate forbids it. *What would
//! reverse this:* a frame budget in which one 82 KiB buffer per client is the
//! thing that does not fit, at which point the encoder streams into a ring
//! rather than into an array and `overflowed` becomes back-pressure.
//!
//! # No clock, no randomness, no float, no allocator
//!
//! Every byte here is a function of the values handed in. The order is the
//! walk's, which is paint order inside a subtree and mark order between them —
//! the client's own order, with nothing for a seed to fix. No binary floating
//! point: a transform is 16.16 fixed point with the scale in each field's name,
//! and a paint is an integer intensity. No allocation: the buffer is an array
//! whose length is a compile-time constant. RFC 0004.

// The same five as `crate::arena` and `crate::dirty`, for the same reason: this
// module is reachable from whatever a client submits, `panic = "abort"` is set
// in every profile, and a panic here is the screen going dark.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing
    )
)]

use core::mem::size_of;

use crate::arena::{Arena, NODES_MAX};
use crate::dirty::{Sealed, Visit, Visited};

/// The four bytes every encoding starts with: `SCNE`, little-endian.
///
/// A magic and not a version. Stage 2 is handed a buffer by a component that
/// could hand it anything, and *these are not scene bytes* is a different fact
/// from *these are scene bytes of a version I do not read* — collapsing the two
/// would make a mis-wired buffer look like a version skew, which is the one
/// diagnosis that sends a reader to the wrong file.
/// Unit: none — a tag, not a quantity.
pub const MAGIC: u32 = 0x454E_4353;

/// Which encoding this is.
///
/// One. It moves when a record's width or order moves, and the golden byte
/// image below is what makes that impossible to do silently.
/// Unit: none — a format version, not a quantity.
pub const VERSION: u16 = 1;

/// Flags in the header's third field.
pub mod flag {
    /// The walk stopped early, so this encoding is *part* of a frame.
    ///
    /// A consumer that presents it draws a picture with last frame's content in
    /// the gap. `crate::dirty::Visited::halted` is where it comes from and says
    /// why stopping is nonetheless the safe direction.
    /// Unit: none — a bit, not a quantity.
    pub const HALTED: u16 = 1 << 0;

    /// The encoding did not fit, so records are missing from the end of it.
    ///
    /// Unreachable while [`ENCODING_MAX`](super::ENCODING_MAX) is sized from
    /// [`NODES_MAX`](crate::arena::NODES_MAX); present so that being wrong
    /// about that is a byte in the stream rather than a short buffer nobody
    /// notices.
    /// Unit: none — a bit, not a quantity.
    pub const OVERFLOWED: u16 = 1 << 1;
}

/// Which of a node's three property records this node record carries.
///
/// A bitset and not three lengths, because a node with no transform is the
/// common case by a wide margin — the scene of a thousand in the tests has one
/// transform and one paint between all thousand of them — and a fixed-width
/// record would spend seventy bytes per node saying *nothing here*.
pub mod present {
    /// A `SetTransform`'s six fixed-point numbers follow, first.
    /// Unit: none — a bit, not a quantity.
    pub const TRANSFORM: u16 = 1 << 0;
    /// A `SetPath`'s offset, length and fill rule follow, second.
    /// Unit: none — a bit, not a quantity.
    pub const PATH: u16 = 1 << 1;
    /// A `SetPaint`'s four intensities and its stroke width follow, third.
    /// Unit: none — a bit, not a quantity.
    pub const PAINT: u16 = 1 << 2;
}

/// The header: magic, version, flags, frame token, node count, total length.
///
/// The last two are not known when the header is written and are patched when
/// the frame is closed. They are in the header rather than in a trailer because
/// a consumer that has to find the end of the stream before it can size its own
/// work has read the stream twice.
/// Unit: bytes.
pub const HEADER_BYTES: usize = size_of::<u32>()
    + size_of::<u16>()
    + size_of::<u16>()
    + size_of::<u64>()
    + size_of::<u32>()
    + size_of::<u32>();

/// A node record's fixed prefix: identifier, depth, kind, presence.
/// Unit: bytes.
pub const NODE_BYTES: usize =
    size_of::<u32>() + size_of::<u32>() + size_of::<u16>() + size_of::<u16>();

/// A transform, when one is present: six 16.16 numbers and no node identifier.
///
/// The identifier is dropped deliberately — it is four bytes of the prefix
/// already, and a record that carried it twice would be a stream in which the
/// two copies can disagree. That is the one way this encoding is not a
/// transcription of `f_abi::scene`'s records, and it is stated here because a
/// reader comparing the two widths will otherwise think a field went missing.
/// Unit: bytes.
pub const TRANSFORM_BYTES: usize = 6 * size_of::<i64>();

/// A path, when one is present: offset, length, fill rule. No identifier, for
/// [`TRANSFORM_BYTES`]'s reason.
/// Unit: bytes.
pub const PATH_BYTES: usize = size_of::<u32>() + size_of::<u32>() + size_of::<u16>();

/// A paint, when one is present: four intensities and a stroke width. No
/// identifier, for [`TRANSFORM_BYTES`]'s reason.
/// Unit: bytes.
pub const PAINT_BYTES: usize = 4 * size_of::<u16>() + size_of::<u32>();

/// The most one node can cost: the prefix and all three property records.
/// Unit: bytes.
pub const NODE_MAX_BYTES: usize = NODE_BYTES + TRANSFORM_BYTES + PATH_BYTES + PAINT_BYTES;

/// The buffer, sized so that a legal walk cannot fill it.
///
/// The module's *why the buffer cannot overflow* is the argument: `Sealed::walk`
/// halts at [`NODES_MAX`] nodes, so this is one node record's worth of room for
/// every node the arena can hold, and a header.
/// Unit: bytes.
pub const ENCODING_MAX: usize = HEADER_BYTES + NODES_MAX * NODE_MAX_BYTES;

// Every width against the types it is made of, rather than against a numeral
// alone. This ties each width to the integer type whose `to_le_bytes` actually
// writes it, which is the thing that must not drift. Both architectures agree
// on all four of these sizes by definition, and `usize` — the one they do not
// agree on — appears in none of them, which is the property the exit clause
// rests on. The four totals are then stated as numerals as well, because the
// golden byte image below is laid out by hand against exactly those numbers and
// a width that moved without it moving is the defect that image exists to
// catch.
const _: () = {
    assert!(size_of::<u16>() == 2, "a u16 is not two bytes");
    assert!(size_of::<u32>() == 4, "a u32 is not four bytes");
    assert!(size_of::<u64>() == 8, "a u64 is not eight bytes");
    assert!(size_of::<i64>() == 8, "an i64 is not eight bytes");
    assert!(HEADER_BYTES == 24, "the header moved and the golden byte image did not");
    assert!(NODE_BYTES == 12, "a node prefix moved and the golden byte image did not");
    assert!(TRANSFORM_BYTES == 48, "a transform moved and the golden byte image did not");
    assert!(PATH_BYTES == 10, "a path moved and the golden byte image did not");
    assert!(PAINT_BYTES == 12, "a paint moved and the golden byte image did not");
    assert!(
        ENCODING_MAX >= HEADER_BYTES + NODES_MAX * NODE_MAX_BYTES,
        "the buffer is smaller than the widest frame the arena can produce"
    );
};

/// Where the three header fields that are written twice start.
///
/// Three and not six. The magic, the version and the frame token are written
/// once, in order, by [`Encoder::open`], and an offset for them would be a
/// second statement of the order the writer already is — the kind of constant
/// that goes on agreeing with nothing. These three are written twice, as zero
/// when the frame opens and for real when it closes, and a patch that landed at
/// the wrong offset would silently corrupt the field beside it rather than
/// failing. The `const` block below is what ties them back to the widths of the
/// fields they follow.
mod at {
    /// The flags: after the magic and the version.
    /// Unit: bytes from the start of the encoding.
    pub const FLAGS: usize = 6;
    /// How many node records follow: after the frame token.
    /// Unit: bytes from the start of the encoding.
    pub const NODES: usize = 16;
    /// How long the whole encoding is: the header's last field.
    /// Unit: bytes from the start of the encoding.
    pub const BYTES: usize = 20;
}

// The three patched offsets against the fields that precede them, so that
// inserting a header field ahead of one of them fails the build rather than
// making `close` overwrite its neighbour. This is the check the offsets exist
// for: a numeral that nothing compares against is a numeral that drifts.
const _: () = {
    assert!(
        at::FLAGS == size_of::<u32>() + size_of::<u16>(),
        "the flags are not after the version"
    );
    assert!(
        at::NODES == at::FLAGS + size_of::<u16>() + size_of::<u64>(),
        "the node count is not after the frame token"
    );
    assert!(at::BYTES == at::NODES + size_of::<u32>(), "the length is not after the node count");
    assert!(
        at::BYTES + size_of::<u32>() == HEADER_BYTES,
        "the length is not the header's last field"
    );
};

/// One frame's encoding, and the thing that writes it.
///
/// Neither `Clone` nor `Copy`: 82 KiB, and a second copy of a frame's encoding
/// is a second answer to what is on the screen.
///
/// **Zero-initialisable, which is RFC 0100's rule and not a nicety.** Every
/// field of [`Encoder::EMPTY`] is zero — an array of zero bytes, two zero
/// counts and two false flags — so a component that holds one through a `Box`
/// pays for it in `.bss` rather than in its image. `user/compositor`'s image
/// once grew by more than its whole reservation for exactly this mistake made
/// one layer down, and `crate::kind::Kind`'s discriminants carry the other half
/// of that story.
pub struct Encoder {
    /// The encoding so far.
    bytes: [u8; ENCODING_MAX],
    /// How much of `bytes` is meaningful.
    /// Unit: bytes.
    len: usize,
    /// How many node records have been written.
    ///
    /// Incremented in [`Encoder::node`], in the same function that writes the
    /// record, so it is the number of records in the stream rather than a
    /// number kept beside it. `crate::dirty::Visited::nodes` is the same count
    /// taken on the other side of the closure, and the two being separately
    /// derived is what makes comparing them worth doing.
    /// Unit: nodes.
    nodes: u32,
    /// Has a frame been opened and not yet closed?
    ///
    /// A visit that arrives outside a frame is dropped rather than appended to
    /// whatever was there, because a node record written before a header is a
    /// stream whose first four bytes are a node identifier.
    open: bool,
    /// Did a record not fit?
    ///
    /// The module's *why the buffer cannot overflow* says this is unreachable
    /// from a legal walk, and why it exists anyway.
    overflowed: bool,
}

impl Encoder {
    /// An encoder holding nothing.
    ///
    /// A `const` and all zeroes, on `Arena::EMPTY`'s terms and for RFC 0100's
    /// reason: it can initialise a `static` or be zeroed into a `Box`, which is
    /// where a compositor's per-client encoder actually lives.
    pub const EMPTY: Self =
        Self { bytes: [0; ENCODING_MAX], len: 0, nodes: 0, open: false, overflowed: false };

    /// Begin a frame, and forget whatever the last one left.
    ///
    /// The frame token is `f_abi::scene::Commit::frame_token`, carried through
    /// [`Sealed::frame_token`] so that a consumer of these bytes can say which
    /// frame it drew without being handed the commit again.
    pub fn open(&mut self, frame_token: u64) {
        self.len = 0;
        self.nodes = 0;
        self.open = true;
        self.overflowed = false;
        self.u32(MAGIC);
        self.u16(VERSION);
        // Patched by `close`, which is the only place the real values are
        // known. Zero rather than a sentinel: a stream that never reached its
        // `close` reads as *nothing unusual, no nodes*, which is the reading
        // that makes a consumer draw nothing rather than draw rubbish.
        self.u16(0);
        self.u64(frame_token);
        self.u32(0);
        self.u32(0);
    }

    /// Encode one node.
    ///
    /// **The parameter is a [`Visit`] and nothing else.** The module's *only
    /// what was marked* is what that is for, and `crate::dirty`'s *what allowed
    /// to walk means here* is how far it goes: a `Visit` holds no arena, so
    /// this function cannot reach a node the commit did not mark, and no test
    /// is needed to say so because no expression exists that would.
    ///
    /// The three property records follow the prefix in the order of their
    /// [`present`] bits, ascending, so a reader needs no lookahead and no
    /// per-record tag — the bitset it has already read says what is coming and
    /// in what order.
    pub fn node(&mut self, visit: &Visit<'_>) {
        if !self.open {
            return;
        }
        let transform = visit.transform();
        let path = visit.path();
        let paint = visit.paint();

        let mut flags = 0;
        if transform.is_some() {
            flags |= present::TRANSFORM;
        }
        if path.is_some() {
            flags |= present::PATH;
        }
        if paint.is_some() {
            flags |= present::PAINT;
        }

        self.u32(visit.node());
        self.u32(visit.depth());
        self.u16(visit.kind().wire());
        self.u16(flags);

        if let Some(set) = transform {
            // The node identifier is not written: it is the prefix's first
            // field already. `TRANSFORM_BYTES` says the same thing as a width.
            self.i64(set.a_x65536);
            self.i64(set.b_x65536);
            self.i64(set.c_x65536);
            self.i64(set.d_x65536);
            self.i64(set.tx_x65536);
            self.i64(set.ty_x65536);
        }
        if let Some(set) = path {
            self.u32(set.geometry_offset);
            self.u32(set.geometry_bytes);
            self.u16(set.fill_rule);
        }
        if let Some(set) = paint {
            self.u16(set.red_x65535);
            self.u16(set.green_x65535);
            self.u16(set.blue_x65535);
            self.u16(set.alpha_x65535);
            self.u32(set.stroke_width_x65536);
        }
        self.nodes = self.nodes.saturating_add(1);
    }

    /// Close the frame, and patch the header with what the walk turned out to
    /// be.
    ///
    /// [`Visited`] rather than three arguments, because it is what
    /// [`Sealed::walk`] answers with and re-typing it here would be a second
    /// place the walk's result is described.
    pub fn close(&mut self, walked: Visited) {
        if !self.open {
            return;
        }
        self.open = false;
        let mut flags = 0;
        if walked.halted {
            flags |= flag::HALTED;
        }
        if self.overflowed {
            flags |= flag::OVERFLOWED;
        }
        self.patch(at::FLAGS, &flags.to_le_bytes());
        self.patch(at::NODES, &self.nodes.to_le_bytes());
        // `len` is a `usize` and the header's field is a `u32`, so this is the
        // one place this module converts a width — and it is a conversion that
        // cannot be lossy here, because `ENCODING_MAX` is 82 KiB and `u32::MAX`
        // is four billion. `try_from` rather than `as` so that being wrong
        // about that produces a length nothing can read rather than a length
        // that is quietly the wrong number.
        let bytes = u32::try_from(self.len).unwrap_or(u32::MAX);
        self.patch(at::BYTES, &bytes.to_le_bytes());
    }

    /// The encoding, as the bytes a consumer is handed.
    ///
    /// Empty until [`Encoder::close`] has run — the header's length and node
    /// count are not written before then, and handing out a stream whose header
    /// says zero nodes while its body holds a thousand is the one shape of this
    /// buffer that could be read wrongly without failing.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        if self.open { &[] } else { self.bytes.get(..self.len).unwrap_or(&[]) }
    }

    /// How many node records the encoder wrote.
    /// Unit: nodes.
    #[must_use]
    pub const fn nodes(&self) -> u32 {
        self.nodes
    }

    /// How long the encoding is, header included.
    /// Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Did this frame encode no nodes at all?
    ///
    /// A frame that dirtied nothing, which `Sealed::is_clean` answers one stage
    /// earlier and more cheaply. Stated as the node count rather than as
    /// `len == 0`, which is never true of a frame that opened: the header is
    /// there either way.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes == 0
    }

    /// Did a record not fit?
    #[must_use]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Append two bytes.
    fn u16(&mut self, value: u16) {
        self.put(&value.to_le_bytes());
    }

    /// Append four bytes.
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes.
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes of a signed quantity, two's complement.
    ///
    /// `to_le_bytes` on `i64` is two's complement on every target, and that is
    /// a language guarantee rather than a platform one — which is exactly why
    /// the transform's fields go through it rather than through a cast to
    /// `u64` that somebody would have to check.
    fn i64(&mut self, value: i64) {
        self.put(&value.to_le_bytes());
    }

    /// The one place bytes land.
    ///
    /// A checked slice rather than an index, and the `None` arm is a real case
    /// rather than one a reader has to argue is unreachable — the module's *why
    /// the buffer cannot overflow* is that argument, and this is the byte that
    /// survives it being wrong.
    fn put(&mut self, bytes: &[u8]) {
        match self.bytes.get_mut(self.len..self.len.saturating_add(bytes.len())) {
            Some(slot) => {
                slot.copy_from_slice(bytes);
                self.len = self.len.saturating_add(bytes.len());
            }
            None => self.overflowed = true,
        }
    }

    /// Overwrite a field that was reserved when the frame opened.
    ///
    /// Only ever the header's, and only ever a field [`Encoder::open`] wrote
    /// zeroes into — so the offsets in `at` are the whole of what this can
    /// reach, and a patch past the end of a buffer that has a header is not a
    /// case this can be in.
    fn patch(&mut self, offset: usize, bytes: &[u8]) {
        if let Some(slot) = self.bytes.get_mut(offset..offset.saturating_add(bytes.len())) {
            slot.copy_from_slice(bytes);
        }
    }
}

/// Encode one sealed frame.
///
/// The driver, and deliberately the whole of the code in this crate that holds
/// a [`Sealed`], an `&Arena` and an [`Encoder`] at the same time. It holds the
/// arena for exactly one expression — the one that hands it to
/// [`Sealed::walk`] — and the encoder never sees it, which is the module's
/// *only what was marked* stated as a shape rather than as a rule.
///
/// Answers what the walk did, so that a caller can compare it against
/// [`Encoder::nodes`] without the encoder having to restate it.
pub fn encode(into: &mut Encoder, sealed: &Sealed, arena: &Arena) -> Visited {
    into.open(sealed.frame_token());
    let walked = sealed.walk(arena, |visit| into.node(&visit));
    into.close(walked);
    walked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{Applied, Hung};
    use crate::dirty::{Dirty, Marked};
    use crate::kind::Kind;
    use f_abi::NO_DEADLINE;
    use f_abi::scene::{
        CreateNode, Delta, Entry, NO_NODE, RemoveNode, SetPaint, SetPath, SetTransform, fill, op,
    };

    /// The frame every encoding below belongs to.
    ///
    /// Distinct in all eight bytes, so a header that wrote the token at the
    /// wrong offset or in the wrong byte order cannot produce the same image.
    const FRAME: u64 = 0x0BAD_F00D_0000_0001;

    /// A deadline far enough from zero to be unmistakably a deadline.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// How many groups the scene of a thousand has.
    const GROUPS: u32 = 10;

    /// How many leaves hang under each group.
    const PER_GROUP: u32 = 100;

    /// The scene of a thousand, actually built.
    const SCENE_NODES: usize = 1 + GROUPS as usize + (GROUPS * PER_GROUP) as usize;

    /// The one root of the scene of a thousand.
    const ROOT: u32 = 1;

    /// One past the largest identifier that scene uses, so a per-node tally can
    /// be an array rather than a map this crate could not allocate.
    const IDS: usize = 2101;

    /// The identifier of a group.
    const fn group(nth: u32) -> u32 {
        10 + nth
    }

    /// The identifier of a leaf under a group.
    const fn leaf(under: u32, nth: u32) -> u32 {
        1000 + under * 100 + nth
    }

    /// Every identifier the scene of a thousand uses.
    fn every_node() -> impl Iterator<Item = u32> {
        core::iter::once(ROOT)
            .chain((1..=GROUPS).map(group))
            .chain((1..=GROUPS).flat_map(|under| (1..=PER_GROUP).map(move |nth| leaf(under, nth))))
    }

    /// A delta around a body, with an envelope that is legal for it.
    fn delta(body: Entry) -> Delta {
        let scheduled = op::carries_deadline(body.opcode()) == Some(true);
        Delta {
            user_data: 0,
            class: 0,
            deadline: if scheduled { SCHEDULED_AT } else { NO_DEADLINE },
            payload_offset: 0,
            flags: 0,
            body,
        }
    }

    /// The delta that hangs a node at the end of its parent's children.
    fn hang(node: u32, parent: u32, kind: Kind) -> Delta {
        delta(Entry::CreateNode(CreateNode { node, parent, before: NO_NODE, kind: kind.wire() }))
    }

    /// Build the scene of a thousand: a root, ten groups, a hundred leaves each.
    fn build_a_thousand(arena: &mut Arena) {
        let created = Ok(Applied::Hung(Hung::Created));
        assert_eq!(arena.apply(&hang(ROOT, NO_NODE, Kind::Layer)), created);
        for under in 1..=GROUPS {
            assert_eq!(arena.apply(&hang(group(under), ROOT, Kind::Transform)), created);
            for nth in 1..=PER_GROUP {
                assert_eq!(arena.apply(&hang(leaf(under, nth), group(under), Kind::Draw)), created);
            }
        }
        assert_eq!(arena.live(), SCENE_NODES);
    }

    /// Is `node` the root, or under it?
    ///
    /// The second opinion, and deliberately the other algorithm: the walk that
    /// feeds the encoder descends by child links and this ascends by parent
    /// links, so the two agreeing is worth more than either alone.
    fn under(arena: &Arena, root: u32, node: u32) -> bool {
        let mut at = node;
        loop {
            if at == root {
                return true;
            }
            match arena.parent_of(at) {
                Ok(NO_NODE) | Err(_) => return false,
                Ok(parent) => at = parent,
            }
        }
    }

    /// One node record, read back out of the stream.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct Record {
        node: u32,
        depth: u32,
        kind: u16,
        present: u16,
        transform: Option<[i64; 6]>,
        path: Option<(u32, u32, u16)>,
        paint: Option<(u16, u16, u16, u16, u32)>,
    }

    /// The header, read back out of the stream.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Header {
        magic: u32,
        version: u16,
        flags: u16,
        frame_token: u64,
        nodes: u32,
        bytes: u32,
    }

    /// Reads a stream back by the widths this module declares.
    ///
    /// **Not a second encoder.** It consumes bytes rather than producing them,
    /// and it is what makes the count and the identity of the nodes in a stream
    /// observable at all — an assertion about [`Encoder::nodes`] alone would be
    /// an assertion about a counter. What it deliberately cannot catch is a
    /// width that moved, because it reads the same constants the writer wrote
    /// with; the golden byte image is what catches that, and saying so here is
    /// better than letting a reader take this for a second opinion on layout.
    struct Cursor<'a> {
        raw: &'a [u8],
        at: usize,
    }

    impl<'a> Cursor<'a> {
        fn new(raw: &'a [u8]) -> Self {
            Self { raw, at: 0 }
        }

        fn take(&mut self, bytes: usize) -> &'a [u8] {
            let from = self.at;
            self.at += bytes;
            &self.raw[from..self.at]
        }

        fn u16(&mut self) -> u16 {
            u16::from_le_bytes(self.take(2).try_into().expect("two bytes"))
        }

        fn u32(&mut self) -> u32 {
            u32::from_le_bytes(self.take(4).try_into().expect("four bytes"))
        }

        fn u64(&mut self) -> u64 {
            u64::from_le_bytes(self.take(8).try_into().expect("eight bytes"))
        }

        fn i64(&mut self) -> i64 {
            i64::from_le_bytes(self.take(8).try_into().expect("eight bytes"))
        }

        fn header(&mut self) -> Header {
            Header {
                magic: self.u32(),
                version: self.u16(),
                flags: self.u16(),
                frame_token: self.u64(),
                nodes: self.u32(),
                bytes: self.u32(),
            }
        }

        fn record(&mut self) -> Record {
            let node = self.u32();
            let depth = self.u32();
            let kind = self.u16();
            let present = self.u16();
            // The three in the order their presence bits ascend, which is the
            // order the encoder writes them in and the only order a reader
            // holding no per-record tag can use.
            let transform = (present & present::TRANSFORM != 0)
                .then(|| [self.i64(), self.i64(), self.i64(), self.i64(), self.i64(), self.i64()]);
            let path = (present & present::PATH != 0).then(|| (self.u32(), self.u32(), self.u16()));
            let paint = (present & present::PAINT != 0)
                .then(|| (self.u16(), self.u16(), self.u16(), self.u16(), self.u32()));
            Record { node, depth, kind, present, transform, path, paint }
        }
    }

    /// Read a whole stream, handing each record to `each` with its position.
    ///
    /// A callback rather than a collection because this crate has no allocator
    /// and its tests do not get one either. Two things are asserted here rather
    /// than at every call site: the cursor lands exactly on the end of the
    /// slice, and the header's own length field is that length — so *the
    /// header's node count is the number of records* and *the header's length
    /// is the stream's* are facts about the bytes rather than about the writer.
    fn read<F: FnMut(usize, Record)>(raw: &[u8], mut each: F) -> Header {
        let mut cursor = Cursor::new(raw);
        let header = cursor.header();
        for ix in 0..header.nodes as usize {
            let record = cursor.record();
            each(ix, record);
        }
        assert_eq!(cursor.at, raw.len(), "the header's node count is not the stream's length");
        assert_eq!(header.bytes as usize, raw.len(), "the header's length is not the length");
        header
    }

    /// Copy an encoding out, so that a second one can be made with the same
    /// encoder.
    ///
    /// One encoder per test rather than two: [`ENCODING_MAX`] is 82 KiB and a
    /// test thread's stack is not the place to put two of them.
    fn snapshot(out: &Encoder, into: &mut [u8]) -> usize {
        let bytes = out.bytes();
        into.get_mut(..bytes.len())
            .expect("the snapshot buffer is too small")
            .copy_from_slice(bytes);
        bytes.len()
    }

    // ------------------------------------------------------------------
    // The scene the golden byte image is taken of: two nodes, one of each
    // property record, every field distinct and non-zero so that a writer which
    // dropped one, or wrote two in the wrong order, cannot produce these bytes.
    // ------------------------------------------------------------------

    /// The image's root: a layer, with a transform on it.
    const IMAGE_ROOT: u32 = 3;

    /// The image's leaf: a draw, with a path and a paint on it.
    const IMAGE_LEAF: u32 = 7;

    /// The transform the image's root carries.
    ///
    /// A shear as well as a scale, and two of the six negative, for
    /// `SetTransform::SPECIMEN`'s reason: a matrix whose off-diagonal entries
    /// are zero cannot tell a writer that emits `b` from one that has stopped
    /// emitting it, and a matrix with no negative entry cannot tell two's
    /// complement from a sign-magnitude mistake.
    const IMAGE_TRANSFORM: SetTransform = SetTransform {
        node: IMAGE_ROOT,
        a_x65536: 65_536,
        b_x65536: 32_768,
        c_x65536: -32_768,
        d_x65536: 131_072,
        tx_x65536: -196_608,
        ty_x65536: 3_407_872,
    };

    /// The path the image's leaf carries.
    const IMAGE_PATH: SetPath = SetPath {
        node: IMAGE_LEAF,
        geometry_offset: 0x2000,
        geometry_bytes: 96,
        fill_rule: fill::NON_ZERO,
    };

    /// The paint the image's leaf carries.
    ///
    /// Five distinct non-zero values, and the stroke width is the field that
    /// made the rule in `abi/src/scene.rs`: it is the record's last, and a zero
    /// there makes *the encoder wrote this field* and *the encoder stopped
    /// before this field* the same observation.
    const IMAGE_PAINT: SetPaint = SetPaint {
        node: IMAGE_LEAF,
        red_x65535: 0x1111,
        green_x65535: 0x2222,
        blue_x65535: 0x3333,
        alpha_x65535: 0x4444,
        stroke_width_x65536: 0x0001_8000,
    };

    /// Build the two-node scene the golden image is taken of.
    fn build_the_image_scene(arena: &mut Arena) {
        let created = Ok(Applied::Hung(Hung::Created));
        assert_eq!(arena.apply(&hang(IMAGE_ROOT, NO_NODE, Kind::Layer)), created);
        assert_eq!(arena.apply(&hang(IMAGE_LEAF, IMAGE_ROOT, Kind::Draw)), created);
        assert_eq!(arena.set_transform(IMAGE_TRANSFORM), Ok(()));
        assert_eq!(arena.set_path(IMAGE_PATH), Ok(()));
        assert_eq!(arena.set_paint(IMAGE_PAINT), Ok(()));
    }

    /// The encoding of that scene, byte for byte, written by hand.
    ///
    /// **This is the architecture clause's evidence and not a regression
    /// guard.** Nothing local can run the AArch64 half — `ARCH_RUN_GAP` says
    /// there is no hosted AArch64 binary in the development container — so what
    /// carries *byte-identical on both architectures* is this literal plus the
    /// `tests (AArch64, weak memory)` job that runs it on the other machine.
    /// Every byte below was derived from the declared widths and the values
    /// above rather than copied out of a run, which is the whole point: an
    /// encoder that wrote native-endian integers, or that let the compiler
    /// choose a field order, agrees with this array on one architecture and not
    /// on the other — and an image taken from a green x86-64 run would have
    /// agreed with it on both.
    ///
    /// 118 bytes: a 24-byte header, a 60-byte record for the root (12 + 48),
    /// and a 34-byte record for the leaf (12 + 10 + 12).
    const IMAGE: [u8; 118] = [
        // -- header ----------------------------------------------------
        0x53, 0x43, 0x4E, 0x45, // magic 0x454E_4353, "SCNE"
        0x01, 0x00, // version 1
        0x00, 0x00, // flags: neither halted nor overflowed
        0x01, 0x00, 0x00, 0x00, 0x0D, 0xF0, 0xAD, 0x0B, // frame 0x0BAD_F00D_0000_0001
        0x02, 0x00, 0x00, 0x00, // two node records
        0x76, 0x00, 0x00, 0x00, // 118 bytes in all
        // -- node 3, the layer, at depth 0 -----------------------------
        0x03, 0x00, 0x00, 0x00, // node 3
        0x00, 0x00, 0x00, 0x00, // depth 0
        0x03, 0x00, // kind 3, layer
        0x01, 0x00, // present: transform
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // a = 65_536
        0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // b = 32_768
        0x00, 0x80, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // c = -32_768
        0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, // d = 131_072
        0x00, 0x00, 0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // tx = -196_608
        0x00, 0x00, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00, // ty = 3_407_872
        // -- node 7, the draw, at depth 1 ------------------------------
        0x07, 0x00, 0x00, 0x00, // node 7
        0x01, 0x00, 0x00, 0x00, // depth 1
        0x04, 0x00, // kind 4, draw
        0x06, 0x00, // present: path and paint, in that order
        0x00, 0x20, 0x00, 0x00, // geometry_offset 0x2000
        0x60, 0x00, 0x00, 0x00, // geometry_bytes 96
        0x01, 0x00, // fill_rule 1, non-zero winding
        0x11, 0x11, // red
        0x22, 0x22, // green
        0x33, 0x33, // blue
        0x44, 0x44, // alpha
        0x00, 0x80, 0x01, 0x00, // stroke_width 0x0001_8000
    ];

    #[test]
    fn the_encoding_of_one_scene_is_this_byte_image() {
        // The exit's first clause, as far as one machine can take it.
        let mut arena = Arena::EMPTY;
        build_the_image_scene(&mut arena);

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, IMAGE_ROOT), Marked::Added);
        let sealed = dirty.at_commit(FRAME);

        let mut out = Encoder::EMPTY;
        let walked = encode(&mut out, &sealed, &arena);
        assert_eq!(walked.nodes, 2);
        assert!(!out.overflowed());

        assert_eq!(out.bytes(), &IMAGE[..], "the encoding is not the byte image stated here");
        assert_eq!(out.len(), IMAGE.len());
        assert_eq!(
            IMAGE.len(),
            HEADER_BYTES + (NODE_BYTES + TRANSFORM_BYTES) + (NODE_BYTES + PATH_BYTES + PAINT_BYTES),
            "the image's length is not what the declared widths add up to"
        );
    }

    #[test]
    fn the_image_reads_back_as_the_records_it_was_made_from() {
        // The other direction over the same bytes. The literal above is a
        // string of hex until something says what it means, and a reader that
        // recovers every field from it is what says it.
        let mut got = [Record::default(); 2];
        let header = read(&IMAGE, |ix, record| {
            if let Some(slot) = got.get_mut(ix) {
                *slot = record;
            }
        });
        assert_eq!(
            header,
            Header {
                magic: MAGIC,
                version: VERSION,
                flags: 0,
                frame_token: FRAME,
                nodes: 2,
                bytes: 118,
            }
        );

        let root = got[0];
        assert_eq!((root.node, root.depth, root.kind), (IMAGE_ROOT, 0, Kind::Layer.wire()));
        assert_eq!(root.present, present::TRANSFORM);
        assert_eq!(
            root.transform,
            Some([
                IMAGE_TRANSFORM.a_x65536,
                IMAGE_TRANSFORM.b_x65536,
                IMAGE_TRANSFORM.c_x65536,
                IMAGE_TRANSFORM.d_x65536,
                IMAGE_TRANSFORM.tx_x65536,
                IMAGE_TRANSFORM.ty_x65536,
            ])
        );
        assert_eq!(root.path, None);
        assert_eq!(root.paint, None);

        let leaf = got[1];
        assert_eq!((leaf.node, leaf.depth, leaf.kind), (IMAGE_LEAF, 1, Kind::Draw.wire()));
        assert_eq!(leaf.present, present::PATH | present::PAINT);
        assert_eq!(leaf.transform, None);
        assert_eq!(
            leaf.path,
            Some((IMAGE_PATH.geometry_offset, IMAGE_PATH.geometry_bytes, IMAGE_PATH.fill_rule))
        );
        assert_eq!(
            leaf.paint,
            Some((
                IMAGE_PAINT.red_x65535,
                IMAGE_PAINT.green_x65535,
                IMAGE_PAINT.blue_x65535,
                IMAGE_PAINT.alpha_x65535,
                IMAGE_PAINT.stroke_width_x65536,
            ))
        );
    }

    /// The encoding of one group of the scene of a thousand: a header and a
    /// hundred and one bare prefixes.
    /// Unit: bytes.
    const ONE_GROUP_BYTES: usize = HEADER_BYTES + (1 + PER_GROUP as usize) * NODE_BYTES;

    #[test]
    fn the_same_scene_encoded_twice_is_the_same_bytes() {
        // Determinism stated over the encoder rather than inferred from the
        // absence of a clock: two frames, one scene, one byte comparison.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut out = Encoder::EMPTY;
        let mut first = [0u8; ONE_GROUP_BYTES];

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, group(4)), Marked::Added);
        encode(&mut out, &dirty.at_commit(FRAME), &arena);
        let len = snapshot(&out, &mut first);
        assert_eq!(len, ONE_GROUP_BYTES);

        let mut again = Dirty::CLEAN;
        assert_eq!(again.mark(&arena, group(4)), Marked::Added);
        encode(&mut out, &again.at_commit(FRAME), &arena);
        assert_eq!(out.bytes(), &first[..], "two encodings of one scene are not the same bytes");
    }

    #[test]
    fn the_encoder_holds_only_what_the_commit_marked() {
        // The exit's second clause, in the shape `E3-B01e`'s own tests use: a
        // count, and then the identity of every node behind the count.
        //
        // The scene is chosen so that an encoder which walked everything cannot
        // produce these bytes. A hundred and one of a thousand and eleven nodes
        // are marked, so *the stream holds 101 records* and *the stream holds
        // 1011 records* differ in length, in node count and in content — which
        // is what the test after this one asserts directly.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let changed = group(4);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, changed), Marked::Added);
        let sealed = dirty.at_commit(FRAME);

        let mut out = Encoder::EMPTY;
        let walked = encode(&mut out, &sealed, &arena);

        let expected = 1 + PER_GROUP as usize;
        assert!(expected * 9 < SCENE_NODES, "the marked subtree is not a small part of the scene");

        // Three numbers from three places: the walk's own tally, the encoder's
        // count, and the number of records actually in the bytes.
        let mut seen = [0u8; IDS];
        let mut records = 0;
        let header = read(out.bytes(), |_, record| {
            records += 1;
            assert!(record.present == 0, "a node of this subtree carries a property record");
            if let Some(count) = seen.get_mut(record.node as usize) {
                *count = count.saturating_add(1);
            }
        });
        assert_eq!(walked.nodes, expected, "the walk visited more or fewer than the subtree");
        assert_eq!(out.nodes() as usize, expected, "the encoder wrote a different number");
        assert_eq!(records, expected, "the stream holds a different number");
        assert_eq!(header.nodes as usize, expected);
        assert_eq!(header.frame_token, FRAME);
        assert_eq!(header.flags, 0);

        // And not a count that happens to match: exactly those nodes, each
        // exactly once, and nothing outside the marked subtree at all. The
        // membership test ascends parent links, which is not how the walk that
        // fed the encoder found them.
        for node in every_node() {
            let want = u8::from(under(&arena, changed, node));
            assert_eq!(
                seen.get(node as usize).copied(),
                Some(want),
                "node {node} is in the stream {want} time(s) too few or too many"
            );
        }

        // The length is arithmetic rather than coincidence: no node of this
        // subtree carries a property record, so each costs exactly a prefix.
        assert_eq!(out.len(), ONE_GROUP_BYTES);
    }

    #[test]
    fn the_whole_scene_is_a_different_and_longer_encoding() {
        // The reviewer's attack, written down. An encoder that ignored the
        // dirty set and walked the whole graph would produce *these* bytes for
        // the scene in the test above, so that test is only worth its clause if
        // the two encodings are distinguishable at all.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut out = Encoder::EMPTY;
        let mut marked = [0u8; ONE_GROUP_BYTES];

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, group(4)), Marked::Added);
        encode(&mut out, &dirty.at_commit(FRAME), &arena);
        let marked_len = snapshot(&out, &mut marked);

        let mut everything = Dirty::CLEAN;
        assert_eq!(everything.mark(&arena, NO_NODE), Marked::Whole);
        let walked = encode(&mut out, &everything.at_commit(FRAME), &arena);

        assert_eq!(walked.nodes, SCENE_NODES);
        assert_eq!(out.nodes() as usize, SCENE_NODES);
        assert!(
            out.len() > marked_len,
            "the whole scene encodes to no more bytes than one subtree of it"
        );
        assert_ne!(out.bytes(), &marked[..]);

        // Every node of the scene, once each, when everything is marked — so
        // the difference above is the dirty set and not an encoder that drops
        // nodes.
        let mut seen = [0u8; IDS];
        read(out.bytes(), |_, record| {
            if let Some(count) = seen.get_mut(record.node as usize) {
                *count = count.saturating_add(1);
            }
        });
        assert!(every_node().all(|node| seen.get(node as usize).copied() == Some(1)));
    }

    #[test]
    fn a_frame_that_dirtied_nothing_encodes_a_header_and_no_records() {
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let dirty = Dirty::CLEAN;
        let sealed = dirty.at_commit(FRAME);
        assert!(sealed.is_clean());

        let mut out = Encoder::EMPTY;
        let walked = encode(&mut out, &sealed, &arena);
        assert_eq!(walked.nodes, 0);
        assert!(out.is_empty());
        assert_eq!(out.len(), HEADER_BYTES);

        let mut records = 0;
        let header = read(out.bytes(), |_, _| records += 1);
        assert_eq!(header.magic, MAGIC);
        assert_eq!(header.nodes, 0);
        assert_eq!(header.bytes as usize, HEADER_BYTES);
        assert_eq!(records, 0);
    }

    #[test]
    fn opening_a_second_frame_forgets_the_first() {
        // The encoder is held across frames by whatever component owns it —
        // that is why it is a `const EMPTY` rather than something an encode
        // call returns — so a frame that inherited the last one's bytes is the
        // defect that shape invites.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut out = Encoder::EMPTY;
        let mut first = Dirty::CLEAN;
        assert_eq!(first.mark(&arena, ROOT), Marked::Added);
        encode(&mut out, &first.at_commit(FRAME), &arena);
        assert_eq!(out.nodes() as usize, SCENE_NODES);

        let second = Dirty::CLEAN;
        encode(&mut out, &second.at_commit(FRAME + 1), &arena);
        assert_eq!(out.nodes(), 0);
        assert_eq!(out.len(), HEADER_BYTES);
        let header = read(out.bytes(), |_, _| ());
        assert_eq!(header.frame_token, FRAME + 1);
    }

    #[test]
    fn a_visit_outside_a_frame_is_not_appended_to_the_last_one() {
        // `Encoder::node` is public, so a caller may drive the walk itself
        // rather than through `encode`. One that forgot to open a frame would
        // otherwise write node records over a closed stream, and the stream
        // would then state a length and a node count that are no longer its
        // own.
        let mut arena = Arena::EMPTY;
        build_the_image_scene(&mut arena);

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, IMAGE_ROOT), Marked::Added);
        let sealed = dirty.at_commit(FRAME);

        let mut out = Encoder::EMPTY;
        encode(&mut out, &sealed, &arena);
        let closed = out.len();

        // The same walk again, with no `open` in front of it.
        sealed.walk(&arena, |visit| out.node(&visit));
        assert_eq!(out.len(), closed, "a visit outside a frame lengthened the encoding");
        assert_eq!(out.bytes(), &IMAGE[..]);
    }

    #[test]
    fn the_buffer_holds_the_widest_frame_the_arena_can_produce() {
        // `overflowed` is unreachable from a legal walk, and this is the
        // arithmetic that says so rather than a comment claiming it: every node
        // the arena can hold, carrying all three property records, still fits.
        assert_eq!(NODE_MAX_BYTES, NODE_BYTES + TRANSFORM_BYTES + PATH_BYTES + PAINT_BYTES);
        assert_eq!(NODE_MAX_BYTES, 82);
        assert_eq!(ENCODING_MAX, HEADER_BYTES + NODES_MAX * NODE_MAX_BYTES);

        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, NO_NODE), Marked::Whole);

        let mut out = Encoder::EMPTY;
        let walked = encode(&mut out, &dirty.at_commit(FRAME), &arena);
        assert!(!walked.halted);
        assert!(!out.overflowed());
        assert!(out.len() <= ENCODING_MAX);
    }

    #[test]
    fn a_node_carries_the_property_records_a_delta_set_on_it_and_no_others() {
        // The presence bitset over all eight combinations, because the order
        // the three records are written in is a rule a reader cannot discover
        // from the bytes and can only check against.
        let mut arena = Arena::EMPTY;
        let created = Ok(Applied::Hung(Hung::Created));
        assert_eq!(arena.apply(&hang(ROOT, NO_NODE, Kind::Layer)), created);
        for nth in 0..8u32 {
            let node = 100 + nth;
            assert_eq!(arena.apply(&hang(node, ROOT, Kind::Draw)), created);
            if nth & 1 != 0 {
                assert_eq!(arena.set_transform(SetTransform { node, ..IMAGE_TRANSFORM }), Ok(()));
            }
            if nth & 2 != 0 {
                assert_eq!(arena.set_path(SetPath { node, ..IMAGE_PATH }), Ok(()));
            }
            if nth & 4 != 0 {
                assert_eq!(arena.set_paint(SetPaint { node, ..IMAGE_PAINT }), Ok(()));
            }
        }

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, ROOT), Marked::Added);
        let mut out = Encoder::EMPTY;
        encode(&mut out, &dirty.at_commit(FRAME), &arena);

        // The root carries nothing, and then one child per combination.
        let mut expected_len = HEADER_BYTES + NODE_BYTES;
        for nth in 0..8u32 {
            expected_len += NODE_BYTES;
            expected_len += if nth & 1 != 0 { TRANSFORM_BYTES } else { 0 };
            expected_len += if nth & 2 != 0 { PATH_BYTES } else { 0 };
            expected_len += if nth & 4 != 0 { PAINT_BYTES } else { 0 };
        }

        let mut children = 0;
        let header = read(out.bytes(), |ix, record| {
            if ix == 0 {
                assert_eq!(record.node, ROOT);
                assert_eq!(record.present, 0);
                return;
            }
            let nth = (ix - 1) as u32;
            children += 1;
            assert_eq!(record.node, 100 + nth);
            assert_eq!(record.present, nth as u16, "the presence bits are not the three deltas");
            assert_eq!(record.transform.is_some(), nth & 1 != 0);
            assert_eq!(record.path.is_some(), nth & 2 != 0);
            assert_eq!(record.paint.is_some(), nth & 4 != 0);
        });
        assert_eq!(header.nodes, 9);
        assert_eq!(children, 8);
        assert_eq!(out.len(), expected_len, "the stream is not as long as its records say");
    }

    #[test]
    fn a_marked_node_the_frame_then_removed_encodes_nothing() {
        // `Sealed::walk`'s rule, observed from the encoder's side: a mark names
        // a root, and a stream holding a record for a node the scene no longer
        // has is a picture drawn from a graph that is gone.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let doomed = leaf(5, 5);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, doomed), Marked::Added);
        assert_eq!(
            arena.apply(&delta(Entry::RemoveNode(RemoveNode { node: doomed }))),
            Ok(Applied::Removed(1))
        );
        assert!(!arena.holds(doomed));

        let mut out = Encoder::EMPTY;
        let walked = encode(&mut out, &dirty.at_commit(FRAME), &arena);
        assert_eq!(walked.nodes, 0);
        assert_eq!(out.nodes(), 0);
        assert_eq!(out.len(), HEADER_BYTES);
    }
}
