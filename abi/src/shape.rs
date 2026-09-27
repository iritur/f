// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The `shape` protocol: one run of text in, glyphs and design-unit positions
//! out. RFC 0082's third property, as a type.
//!
//! # Why it is here
//!
//! RFC 0082 put the shaper behind the licence boundary, in a component of its
//! own, reached over a ring and by no other route. What crosses that ring is the
//! one coupling the permissive tree has to imported source (`LICENSING.md` rule
//! 1), and every other protocol in this tree is declared in this crate — so this
//! is `abi/src/shape.rs`, the second of the two files RFC 0082 names by path.
//! `user/shaper/manifest.toml` is the first, and its `[[ring]]` names this
//! protocol by the word [`PROTOCOL`].
//!
//! # Only integers cross, and each carries its scale in its name
//!
//! RFC 0082's third property: *whatever the import does internally stops at the
//! shim*. The import computes in floating point — `third_party/harfrust`'s
//! record says so — and nothing here can carry it: every field below is an
//! integer, every position is `_design_units`, the face's own grid, and the
//! grid itself crosses beside them in [`ReplyHead::units_per_em`]. Turning a
//! design unit into a pixel is `f_text::metric`'s rounding rule, applied once,
//! on the permissive side, where a claim can see it. `cargo xtask
//! lint-determinism` refuses a binary floating-point type in this file as it
//! does anywhere, and `cargo xtask lint-manifests` names this file as the one
//! the property is about (RFC 0141).
//!
//! **No size crosses, and that is a statement rather than an omission.** A
//! position in design units is the same number at every size, so the answer to
//! a request does not depend on one; a table that would make it depend — `trak`,
//! hinting — is not applied by the shim, and the day one is, the size crosses as
//! a fixed-point field here and not as a fraction the import is handed.
//!
//! # The face is named by its address, and the shaper checks it
//!
//! [`Request::face`] is a content address (`E3-B03b`): the SHA-256 of the face's
//! bytes. The request carries the name and not the bytes, because a face is
//! hundreds of kilobytes and a run is a hundred bytes. The shaper believes
//! neither the name nor whoever handed it the bytes: a name its manifest's
//! `[[face]]` tables do not declare is [`Refusal::FaceNotHeld`] before any byte
//! is read, and so are bytes that do not hash to the name. The declaration is
//! the check that matters — a hash says the bytes are the bytes somebody named,
//! not that anybody vouched for them, and the import sizes its allocations from
//! counts it reads out of the face (RFC 0141).
//!
//! # Fixed widths, and an unread byte is refused
//!
//! A request is [`Request::BYTES`] long whatever its text, and a glyph is
//! [`Glyph::BYTES`], encoded field by field in little-endian — the discipline
//! [`store`](crate::store) and [`objects`](crate::objects) keep, for their
//! reason: the layout is the encoding and not the compiler's opinion of a
//! struct. Every byte a decoder does not read — a feature slot past the count,
//! a text byte past the length — must be zero, and is refused otherwise. R04.
//!
//! # What is not here yet, and what runs today
//!
//! The envelope. A `shape` entry is an [`Sqe`](crate::Sqe) with
//! [`op::SHAPE`] whose registered buffer holds one [`Request`] and receives one
//! reply, and the rule for which fields of that envelope must be zero is owed to
//! the day the shaper's component serves a ring. It cannot today: its image does
//! not fit the text the frame maps for a spawned component (`UNSPAWNABLE` in
//! `xtask`, RFC 0141).
//!
//! So what runs is narrower than *reached over a ring*, and it is said here
//! because this file is where a reader would otherwise assume the rest. These
//! records are encoded, decoded and answered by `user/shaper`'s host test in one
//! process, with no ring, no [`Sqe`](crate::Sqe) and no component between the
//! two ends — `cargo xtask test-host` on x86-64 Linux, locally and in CI's
//! x86-64 test job. The same test is in CI's AArch64 job, on an AArch64 Linux
//! runner, and that is the only place its AArch64 half runs: it is evidence
//! once that job has run on a commit carrying it and printed its `shaped … on
//! aarch64` lines, and not before. The image a frame would load is built for
//! `x86_64-unknown-none`, linked and measured, and never executes — it is nine
//! times what a spawn shape maps, so no frame here can load it; for
//! `aarch64-unknown-none` the shim is compiled and nothing more. What would
//! change that is RFC 0141's `UNSPAWNABLE` reversal, and the envelope rule above
//! is written in the same diff. RFC 0141's table is the whole of it.
//!
//! # No clock, no floating point, no allocator
//!
//! RFC 0004. Nothing here reads a clock, draws, allocates or names a float.

/// The protocol's name, as a manifest's `[[ring]] protocol` spells it.
/// Unit: none — a name.
pub const PROTOCOL: &str = "shape";

/// The one version of this protocol there is.
/// Unit: none — a version ordinal.
pub const VERSION: u32 = 1;

/// The opcode space, per service (section 05).
pub mod op {
    /// Shape one run: a [`Request`](super::Request) in, a reply out.
    /// Unit: none — an opcode.
    pub const SHAPE: u8 = 1;

    /// Is this an opcode this protocol defines?
    #[must_use]
    pub const fn known(opcode: u8) -> bool {
        matches!(opcode, SHAPE)
    }
}

/// The longest run a request carries.
///
/// A hundred and twenty-eight, and it is `f_text::cache::TEXT_BYTES_MAX` on
/// purpose: what crosses this ring is a cache miss (RFC 0082's *the cache is in
/// front of the boundary*), and a cache entry is at most that long. A run the
/// cache refuses never reaches the ring, and one this format could not carry
/// would be a miss nothing can answer. `abi` depends on nothing, so the two
/// numbers are one number written twice; `user/shaper`'s tests hold them equal.
/// Unit: bytes of UTF-8.
pub const TEXT_BYTES_MAX: usize = 128;

/// The most features one request names.
///
/// Eight, which is more than the five `f_text::cache::Feature` knows and fewer
/// than anybody has asked for in one run. A bound because the record is fixed
/// width, not because eight is a typographic fact.
/// Unit: features.
pub const FEATURES_MAX: usize = 8;

/// The most glyphs one reply carries.
///
/// Twice [`TEXT_BYTES_MAX`], because a shaper may produce more glyphs than
/// there are bytes — a decomposition in `ccmp`, a dotted circle inserted before
/// a stray mark — and a reply that silently dropped the tail would be a wrong
/// width with no refusal. Past it the shaper answers
/// [`Refusal::TooManyGlyphs`].
/// Unit: glyphs.
pub const GLYPHS_MAX: usize = 2 * TEXT_BYTES_MAX;

/// Which way the run is set.
///
/// Zero is not a direction, for `manifest::domain`'s reason: a zeroed record
/// must never decode as a request anybody meant.
pub mod direction {
    /// Left to right.
    /// Unit: none — a direction ordinal.
    pub const LEFT_TO_RIGHT: u8 = 1;
    /// Right to left.
    /// Unit: none — a direction ordinal.
    pub const RIGHT_TO_LEFT: u8 = 2;

    /// Is this a direction this protocol defines?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, LEFT_TO_RIGHT | RIGHT_TO_LEFT)
    }
}

/// Why a request was not answered.
///
/// A value rather than a packed integer, for [`objects`](crate::objects)'s
/// reason; [`Refusal::packed`] is the one place the mapping to RFC 0010's
/// domains is written, and it adds no code of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The record does not frame a request: a length past its bound, text that
    /// is not UTF-8, a feature tag that is not four printable ASCII bytes, or a
    /// script [`is_script`] refuses.
    Malformed,
    /// A byte the decoder does not read is not zero.
    Reserved,
    /// A direction outside [`direction`].
    UnknownDirection,
    /// The address names no face the shaper's manifest declares, or the bytes
    /// handed over under that name do not hash to it.
    FaceNotHeld,
    /// The bytes are the face named, and the face is not one this tree admits
    /// (`f_text::face::Face::read`) or the import could not read.
    NotAFace,
    /// The run shaped to more than [`GLYPHS_MAX`] glyphs, or to more than the
    /// reply buffer holds.
    TooManyGlyphs,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Malformed => "the record does not frame a shape request",
            Self::Reserved => "a byte this format does not read is not zero",
            Self::UnknownDirection => "the direction is not one this protocol defines",
            Self::FaceNotHeld => "the face address names nothing this shaper holds",
            Self::NotAFace => "the face named is not one this tree admits",
            Self::TooManyGlyphs => "the run shaped to more glyphs than a reply carries",
        }
    }

    /// The refusal as a packed [`error`](crate::error), for a completion.
    #[must_use]
    pub const fn packed(self) -> i32 {
        use crate::error::{self, argument, resource};
        match self {
            Self::Malformed | Self::NotAFace => {
                error::pack(error::ARGUMENT, argument::MALFORMED_HEADER)
            }
            Self::Reserved => error::pack(error::ARGUMENT, argument::RESERVED_NOT_ZERO),
            Self::UnknownDirection => error::pack(error::ARGUMENT, argument::UNKNOWN_FLAG),
            Self::FaceNotHeld => error::pack(error::ARGUMENT, argument::BAD_ADDRESS),
            Self::TooManyGlyphs => error::pack(error::RESOURCE, resource::QUOTA_EXHAUSTED),
        }
    }
}

/// One OpenType feature, as a request names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feature {
    /// The feature's tag, as the font's `FeatureList` spells it.
    /// Unit: none — four printable ASCII bytes, e.g. `kern`.
    pub tag: [u8; 4],
    /// Its value: zero turns it off, one turns it on, and a larger number
    /// chooses an alternate where the feature has several.
    /// Unit: none — an OpenType feature value.
    pub value: u32,
}

impl Feature {
    /// An empty slot, which is what every slot past the count must be.
    pub const NONE: Self = Self { tag: [0; 4], value: 0 };
}

/// One run to shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// The face, by content address.
    /// Unit: bytes, exactly 32 of them — a SHA-256 in FIPS 180-4 order.
    pub face: [u8; 32],
    /// The run's script, as ISO 15924 spells it: one capital and three small
    /// letters, which [`is_script`] holds every constructor and decoder to.
    /// Unit: none — four ASCII letters, e.g. `Latn`.
    pub script: [u8; 4],
    /// Which way the run is set: a [`direction`] constant.
    /// Unit: none — a direction ordinal.
    pub direction: u8,
    /// How many of [`Request::features`] are in use; the rest are
    /// [`Feature::NONE`].
    /// Unit: features, at most [`FEATURES_MAX`].
    pub feature_count: u8,
    /// Features turned on or off beside the ones the shaper applies by default.
    /// Unit: none — see [`Feature`].
    pub features: [Feature; FEATURES_MAX],
    /// How many bytes of [`Request::text`] are the run; the rest are zero.
    /// Unit: bytes, at most [`TEXT_BYTES_MAX`].
    pub text_bytes: u16,
    /// The run, UTF-8.
    /// Unit: bytes of UTF-8.
    pub text: [u8; TEXT_BYTES_MAX],
}

impl Request {
    /// The encoded width of a request.
    /// Unit: bytes.
    pub const BYTES: usize = 32 + 4 + 1 + 1 + 2 + FEATURES_MAX * 8 + TEXT_BYTES_MAX;

    /// A request for `text` in the face at `face`, left to right, in `script`,
    /// with no features beyond the defaults.
    ///
    /// # Errors
    ///
    /// [`Refusal::Malformed`] for a run longer than [`TEXT_BYTES_MAX`] or a
    /// script [`is_script`] refuses.
    pub fn new(face: [u8; 32], script: [u8; 4], text: &str) -> Result<Self, Refusal> {
        if text.len() > TEXT_BYTES_MAX || !is_script(script) {
            return Err(Refusal::Malformed);
        }
        let mut bytes = [0u8; TEXT_BYTES_MAX];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Ok(Self {
            face,
            script,
            direction: direction::LEFT_TO_RIGHT,
            feature_count: 0,
            features: [Feature::NONE; FEATURES_MAX],
            text_bytes: text.len() as u16,
            text: bytes,
        })
    }

    /// The run, as the text it is. Every constructor and decoder has checked
    /// it, so a request that exists holds UTF-8 of the length it says.
    #[must_use]
    pub fn text(&self) -> &str {
        let len = usize::from(self.text_bytes).min(TEXT_BYTES_MAX);
        core::str::from_utf8(&self.text[..len]).unwrap_or("")
    }

    /// The features in use.
    #[must_use]
    pub fn features(&self) -> &[Feature] {
        &self.features[..usize::from(self.feature_count).min(FEATURES_MAX)]
    }

    /// Encode. Little-endian, field by field, nothing skipped.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Request::BYTES] {
        let mut out = [0u8; Request::BYTES];
        out[0..32].copy_from_slice(&self.face);
        out[32..36].copy_from_slice(&self.script);
        out[36] = self.direction;
        out[37] = self.feature_count;
        out[38..40].copy_from_slice(&self.text_bytes.to_le_bytes());
        for (index, feature) in self.features.iter().enumerate() {
            let at = 40 + index * 8;
            out[at..at + 4].copy_from_slice(&feature.tag);
            out[at + 4..at + 8].copy_from_slice(&feature.value.to_le_bytes());
        }
        let at = 40 + FEATURES_MAX * 8;
        out[at..at + TEXT_BYTES_MAX].copy_from_slice(&self.text);
        out
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] naming the first disbelief: a length past its bound,
    /// text that is not UTF-8, a tag that is not a tag, an unknown direction,
    /// or a byte past a count that is not zero.
    pub fn from_bytes(raw: &[u8; Request::BYTES]) -> Result<Self, Refusal> {
        let mut face = [0u8; 32];
        face.copy_from_slice(&raw[0..32]);
        let script = [raw[32], raw[33], raw[34], raw[35]];
        let direction = raw[36];
        let feature_count = raw[37];
        let text_bytes = u16::from_le_bytes([raw[38], raw[39]]);
        if !is_script(script) {
            return Err(Refusal::Malformed);
        }
        if !direction::known(direction) {
            return Err(Refusal::UnknownDirection);
        }
        if usize::from(feature_count) > FEATURES_MAX || usize::from(text_bytes) > TEXT_BYTES_MAX {
            return Err(Refusal::Malformed);
        }
        let mut features = [Feature::NONE; FEATURES_MAX];
        for (index, slot) in features.iter_mut().enumerate() {
            let at = 40 + index * 8;
            let tag = [raw[at], raw[at + 1], raw[at + 2], raw[at + 3]];
            let value = u32::from_le_bytes([raw[at + 4], raw[at + 5], raw[at + 6], raw[at + 7]]);
            if index < usize::from(feature_count) {
                if !is_tag(tag) {
                    return Err(Refusal::Malformed);
                }
            } else if tag != [0; 4] || value != 0 {
                return Err(Refusal::Reserved);
            }
            *slot = Feature { tag, value };
        }
        let at = 40 + FEATURES_MAX * 8;
        let mut text = [0u8; TEXT_BYTES_MAX];
        text.copy_from_slice(&raw[at..at + TEXT_BYTES_MAX]);
        let len = usize::from(text_bytes);
        if text[len..].iter().any(|b| *b != 0) {
            return Err(Refusal::Reserved);
        }
        if core::str::from_utf8(&text[..len]).is_err() {
            return Err(Refusal::Malformed);
        }
        Ok(Self { face, script, direction, feature_count, features, text_bytes, text })
    }
}

/// Is this a script as ISO 15924 spells one: a capital and three small letters,
/// `Latn`, `Cyrl`, `Qaaa`?
///
/// Stricter than a tag, and the strictness is the point. The import is lenient:
/// it folds `latn`, `LATN` and `lATN` into one script and any other four bytes
/// into *unknown*, which it shapes with its default shaper rather than refuse,
/// so a request that named no script anybody can read would be answered as if
/// it had asked for none. The spelling is judged here, once, on both sides of
/// the ring. A code spelled right that names no script the import knows —
/// `Qaaa`, a private-use code — is shaped by the default shaper, as HarfBuzz
/// does: answered, not refused, and said so here rather than left to be found.
#[must_use]
pub const fn is_script(tag: [u8; 4]) -> bool {
    tag[0].is_ascii_uppercase()
        && tag[1].is_ascii_lowercase()
        && tag[2].is_ascii_lowercase()
        && tag[3].is_ascii_lowercase()
}

/// Four printable ASCII bytes: what an OpenType feature tag is.
const fn is_tag(tag: [u8; 4]) -> bool {
    let mut i = 0;
    while i < 4 {
        if tag[i] < 0x20 || tag[i] > 0x7e {
            return false;
        }
        i += 1;
    }
    true
}

/// The head of a reply: the grid the positions are on, and how many follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyHead {
    /// The face's design-unit grid, from its own `head` table. What turns every
    /// `_design_units` field below into a distance, on the permissive side.
    /// Unit: design units per em.
    pub units_per_em: u32,
    /// How many [`Glyph`] records follow.
    /// Unit: glyphs, at most [`GLYPHS_MAX`].
    pub glyphs: u32,
}

impl ReplyHead {
    /// The encoded width of a reply's head.
    /// Unit: bytes.
    pub const BYTES: usize = 8;

    /// Encode.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; ReplyHead::BYTES] {
        let mut out = [0u8; ReplyHead::BYTES];
        out[0..4].copy_from_slice(&self.units_per_em.to_le_bytes());
        out[4..8].copy_from_slice(&self.glyphs.to_le_bytes());
        out
    }

    /// Decode.
    ///
    /// # Errors
    ///
    /// [`Refusal::Malformed`] for a zero grid or a count past [`GLYPHS_MAX`].
    pub fn from_bytes(raw: &[u8; ReplyHead::BYTES]) -> Result<Self, Refusal> {
        let units_per_em = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let glyphs = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        if units_per_em == 0 || glyphs as usize > GLYPHS_MAX {
            return Err(Refusal::Malformed);
        }
        Ok(Self { units_per_em, glyphs })
    }
}

/// One shaped glyph.
///
/// Four positions, as the shaping model has them: how far this glyph moves the
/// pen, and where it is drawn relative to the pen. Each is an integer on the
/// face's grid; none has been scaled, rounded or hinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The glyph, as the face numbers it.
    /// Unit: none — a glyph index into the face named by the request.
    pub glyph: u32,
    /// The first byte of the run this glyph was shaped from. A ligature carries
    /// the cluster of its first component.
    /// Unit: bytes into the request's text.
    pub cluster: u32,
    /// How far this glyph advances the pen horizontally.
    /// Unit: design units, on [`ReplyHead::units_per_em`]'s grid.
    pub x_advance_design_units: i32,
    /// How far this glyph advances the pen vertically — zero in horizontal text.
    /// Unit: design units.
    pub y_advance_design_units: i32,
    /// Where the glyph is drawn relative to the pen, horizontally.
    /// Unit: design units.
    pub x_offset_design_units: i32,
    /// Where the glyph is drawn relative to the pen, vertically.
    /// Unit: design units.
    pub y_offset_design_units: i32,
}

impl Glyph {
    /// The encoded width of a glyph.
    /// Unit: bytes.
    pub const BYTES: usize = 24;

    /// Encode.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Glyph::BYTES] {
        let mut out = [0u8; Glyph::BYTES];
        out[0..4].copy_from_slice(&self.glyph.to_le_bytes());
        out[4..8].copy_from_slice(&self.cluster.to_le_bytes());
        out[8..12].copy_from_slice(&self.x_advance_design_units.to_le_bytes());
        out[12..16].copy_from_slice(&self.y_advance_design_units.to_le_bytes());
        out[16..20].copy_from_slice(&self.x_offset_design_units.to_le_bytes());
        out[20..24].copy_from_slice(&self.y_offset_design_units.to_le_bytes());
        out
    }

    /// Decode. Every bit pattern is a glyph; what a peer may not do is claim
    /// more of them than the head says, which [`ReplyHead`] bounds.
    #[must_use]
    pub fn from_bytes(raw: &[u8; Glyph::BYTES]) -> Self {
        let word = |at: usize| [raw[at], raw[at + 1], raw[at + 2], raw[at + 3]];
        Self {
            glyph: u32::from_le_bytes(word(0)),
            cluster: u32::from_le_bytes(word(4)),
            x_advance_design_units: i32::from_le_bytes(word(8)),
            y_advance_design_units: i32::from_le_bytes(word(12)),
            x_offset_design_units: i32::from_le_bytes(word(16)),
            y_offset_design_units: i32::from_le_bytes(word(20)),
        }
    }
}

/// The largest reply: a head and [`GLYPHS_MAX`] glyphs.
/// Unit: bytes.
pub const REPLY_BYTES_MAX: usize = ReplyHead::BYTES + GLYPHS_MAX * Glyph::BYTES;

// The widths are sums of fields, so a field added to a type without its width
// moving is a build failure rather than a byte two peers disagree about.
const _: () = assert!(Request::BYTES == 232);
const _: () = assert!(Glyph::BYTES == 6 * 4);
const _: () = assert!(ReplyHead::BYTES == 2 * 4);
const _: () = assert!(TEXT_BYTES_MAX <= u16::MAX as usize);
const _: () = assert!(FEATURES_MAX <= u8::MAX as usize);

#[cfg(test)]
mod tests {
    use super::*;

    const FACE: [u8; 32] = [7; 32];

    fn sample() -> Request {
        let mut request = Request::new(FACE, *b"Latn", "Tokyo -> Vienna").expect("a request");
        request.features[0] = Feature { tag: *b"kern", value: 0 };
        request.feature_count = 1;
        request
    }

    #[test]
    fn a_request_reads_back_as_what_was_written() {
        let request = sample();
        let bytes = request.to_bytes();
        let back = Request::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, request);
        assert_eq!(back.text(), "Tokyo -> Vienna");
        assert_eq!(back.features(), &[Feature { tag: *b"kern", value: 0 }]);
    }

    #[test]
    fn every_unread_byte_is_refused() {
        let sound = sample().to_bytes();
        let text_at = 40 + FEATURES_MAX * 8;
        let cases: &[(&str, usize, u8, Refusal)] = &[
            ("a byte past the text", text_at + 20, b'x', Refusal::Reserved),
            ("a tag in an unused feature slot", 40 + 8, b'l', Refusal::Reserved),
            ("a value in an unused feature slot", 40 + 8 + 4, 1, Refusal::Reserved),
            ("a zero direction", 36, 0, Refusal::UnknownDirection),
            ("a third direction", 36, 3, Refusal::UnknownDirection),
            ("a script that is not a tag", 32, 0, Refusal::Malformed),
            ("a script spelled in small letters", 32, b'l', Refusal::Malformed),
            ("a script with a capital inside it", 34, b'T', Refusal::Malformed),
            ("more features than there are slots", 37, FEATURES_MAX as u8 + 1, Refusal::Malformed),
            ("a length past the text", 39, 1, Refusal::Malformed),
            ("text that is not UTF-8", text_at, 0xff, Refusal::Malformed),
        ];
        for (what, at, value, expect) in cases {
            let mut broken = sound;
            broken[*at] = *value;
            assert_eq!(Request::from_bytes(&broken), Err(*expect), "{what}");
        }
    }

    #[test]
    fn a_script_is_spelled_as_iso_15924_spells_it() {
        for good in [*b"Latn", *b"Cyrl", *b"Qaaa", *b"Zyyy"] {
            assert!(is_script(good), "{good:?}");
            assert!(Request::new(FACE, good, "a").is_ok(), "{good:?}");
        }
        for bad in [*b"latn", *b"LATN", *b"abcd", *b"La1n", *b"Lat ", [0; 4]] {
            assert!(!is_script(bad), "{bad:?}");
            assert_eq!(Request::new(FACE, bad, "a"), Err(Refusal::Malformed), "{bad:?}");
        }
    }

    #[test]
    fn a_run_longer_than_a_cache_entry_is_not_a_request() {
        let long = [b'x'; TEXT_BYTES_MAX + 1];
        let long = core::str::from_utf8(&long).expect("ASCII");
        assert_eq!(Request::new(FACE, *b"Latn", long), Err(Refusal::Malformed));
        assert!(Request::new(FACE, *b"Latn", &long[..TEXT_BYTES_MAX]).is_ok());
    }

    #[test]
    fn a_glyph_and_a_head_read_back() {
        let glyph = Glyph {
            glyph: 1805,
            cluster: 6,
            x_advance_design_units: -160,
            y_advance_design_units: 0,
            x_offset_design_units: i32::MIN,
            y_offset_design_units: i32::MAX,
        };
        assert_eq!(Glyph::from_bytes(&glyph.to_bytes()), glyph);
        let head = ReplyHead { units_per_em: 2048, glyphs: 14 };
        assert_eq!(ReplyHead::from_bytes(&head.to_bytes()), Ok(head));
        let past = ReplyHead { units_per_em: 2048, glyphs: GLYPHS_MAX as u32 + 1 };
        assert_eq!(ReplyHead::from_bytes(&past.to_bytes()), Err(Refusal::Malformed));
        let flat = ReplyHead { units_per_em: 0, glyphs: 1 };
        assert_eq!(ReplyHead::from_bytes(&flat.to_bytes()), Err(Refusal::Malformed));
    }

    #[test]
    fn every_refusal_packs_to_an_existing_code() {
        for refusal in [
            Refusal::Malformed,
            Refusal::Reserved,
            Refusal::UnknownDirection,
            Refusal::FaceNotHeld,
            Refusal::NotAFace,
            Refusal::TooManyGlyphs,
        ] {
            assert!(refusal.packed() < 0, "{}", refusal.message());
            assert!(crate::error::unpack(refusal.packed()).is_some());
        }
    }

    #[test]
    fn one_opcode_and_nothing_else() {
        assert!(op::known(op::SHAPE));
        assert!(!op::known(0));
        assert!(!op::known(2));
    }
}
