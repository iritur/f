// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The shim: the `shape` protocol on one side, HarfRust on the other, and the
//! place RFC 0082's third property is kept.
//!
//! # What this crate is
//!
//! RFC 0082 imports the shaper and builds it into a component image of its own.
//! HarfRust arrived as source (`third_party/harfrust/`), and source is not an
//! image: something has to decode a request, hand the run to the import, and
//! encode what comes back. That is this crate, and it is the whole of the
//! permissive code on the far side of the ring. It is permissive — the SPDX
//! line above is this tree's — because it is written here, and it links the
//! import because that is what a shim is; `LICENSING.md`'s table calls
//! `third_party/<name>/` *imported source and its shim*, and RFC 0141 says why
//! the shim is not under `third_party/` and why that does not move the boundary.
//!
//! # Where the floats stop
//!
//! Here. HarfRust computes in floating point inside [`shape`]; what leaves it is
//! [`harfrust::GlyphPosition`]'s four `i32`s, in design units, because no scale
//! is set — `ShapeOptions::scale` stays `None`, which HarfRust documents as
//! *positions and metrics returned in font units*. So the conversion this shim
//! does is a copy between two integer types, and there is no rounding here to
//! argue: the rounding rule is `f_text::metric`'s, on the permissive side. This
//! file names no floating-point type, and `lint-determinism` reads it like any
//! other.
//!
//! # What reaches the import, and what does not
//!
//! The run, its script and direction, the features the request names, and the
//! face's bytes — after the face has been checked three times: its address must
//! be one `manifest.toml`'s `[[face]]` declares ([`FACES`]), its SHA-256 must be
//! that address (`E3-B03b`: a face is addressed by content), and
//! `f_text::face::Face::read` must admit it. No language is set, so the import
//! has no locale to consult; no size is set, so nothing it computes depends on
//! one. That is RFC 0082's first property as far as this crate can hold it —
//! the rest is the manifest's, which routes this component no clock, no device
//! and no powerbox.
//!
//! # Why the declared address, and not only the hash
//!
//! Because a hash check says the bytes are the bytes somebody named, and not
//! that anybody vouched for them. A face with its layout tables altered and
//! re-hashed passes the hash and `Face::read` both, and HarfRust sizes vectors
//! from counts it reads out of `GSUB` and `GPOS`: one such face asks the heap
//! for twenty-one megabytes, a hundred and sixty-seven times what the manifest
//! gives it (measured out of tree, RFC 0141). So the import is handed only the
//! faces the manifest declares, whose bytes are pinned by the same hash and whose
//! heap peak was measured. What a face nobody declared would do to the import is
//! then not a question this component answers.
//!
//! Not a budget on lookup and subtable counts beside it, and that was decided
//! rather than skipped: with the address pinned, the counts the import reads are
//! Inter's, constants a budget would be checking against themselves; and a
//! budget would bound the two allocations one fuzz run found and not the
//! import's allocation in general, which the heap bounds instead — a refused
//! allocation is a panic, and a panic in the image is a fault its supervisor
//! restarts (`component.rs`). *What would reverse it:* a face admitted that
//! nobody in this tree vouches for — a download, a document's embedded font —
//! which is RFC 0005's `hostile` row, and a check on the counts before the
//! import is then the first of several.
//!
//! # Allocation
//!
//! HarfRust allocates. In the image, `f_ring::heap::Heap::COMPONENT` is the
//! allocator, over the `heap` region `user/shaper/manifest.toml` sizes — RFC
//! 0082's second property, *its own allocator, sized in its own manifest*, and
//! the reason this crate forbids `unsafe` and still has one. On the host the
//! test harness's allocator stands in.

#![no_std]
#![forbid(unsafe_code)]
// `core::intrinsics::abort`, which is one `ud2` on x86-64 and asks for no
// `unsafe`: how the image ends a panic in a fault rather than a spin
// (`component.rs` says why a fault). The image only, where `component` is.
#![cfg_attr(
    all(target_arch = "x86_64", target_os = "none", feature = "image"),
    feature(core_intrinsics),
    allow(internal_features)
)]

extern crate alloc;

use alloc::vec::Vec;

use f_abi::shape::{self, Glyph, Refusal, ReplyHead, Request};

/// The faces this component may load: the addresses `manifest.toml`'s `[[face]]`
/// tables declare, in the order they are written there.
///
/// A constant, and the manifest's copy is the one a reader looks at; the two are
/// one list written twice, and `tests/run.rs` holds them equal. *What would
/// reverse it:* the supervisor handing a component its declared faces at spawn,
/// which it cannot do for a component no frame spawns (`UNSPAWNABLE`); then this
/// constant goes and [`serve`]'s `faces` is what the supervisor handed over.
/// Unit: none — SHA-256 content addresses.
pub const FACES: [[u8; 32]; 1] =
    [address("40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82")];

/// A content address from the sixty-four hex digits a manifest writes after
/// `sha256:`, at compile time, so that a mistyped digit is a build failure.
const fn address(hex: &str) -> [u8; 32] {
    const fn digit(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => panic!("a face address is lower-case hex"),
        }
    }
    let hex = hex.as_bytes();
    assert!(hex.len() == 64, "a face address is sixty-four hex digits");
    let mut out = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = digit(hex[index * 2]) << 4 | digit(hex[index * 2 + 1]);
        index += 1;
    }
    out
}

// The allocator, on the machine and in the image only, for `user/compositor`'s
// reason.
#[cfg(all(target_os = "none", feature = "image"))]
#[global_allocator]
static HEAP: f_ring::heap::Heap = f_ring::heap::Heap::COMPONENT;

// The entry, x86-64's for the door's reason, on the machine only, and not
// spawnable by any frame today: `UNSPAWNABLE` in `xtask/src/main.rs` says why and
// what reverses it. `target_os = "none"` as well as the architecture, because this
// crate has an integration test and the host's `std` brings a panic handler of its
// own.
#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "image"))]
pub mod component;

/// Shape one run into `out`, and answer the grid and how many glyphs.
///
/// `faces` is what this component may load — [`FACES`] in the component — and
/// `face` is what the blob store handed back for the request's address.
///
/// # Errors
///
/// - Whatever [`Request::from_bytes`] refuses in `request`: a [`Request`] built
///   field by field meets the decoder's every check here, so this function and
///   [`serve`] refuse the same requests. [`Refusal::Malformed`] for a script
///   [`shape::is_script`] refuses is one of them; a script spelled right that
///   the import has no shaper for is shaped by its default shaper, not refused.
/// - [`Refusal::FaceNotHeld`] when the address is not in `faces`, or `face` does
///   not hash to it.
/// - [`Refusal::NotAFace`] when this tree does not admit the face, or the
///   import cannot read it.
/// - [`Refusal::TooManyGlyphs`] when the run shapes to more glyphs than
///   [`shape::GLYPHS_MAX`] or than `out` holds.
pub fn shape(
    faces: &[[u8; 32]],
    request: &Request,
    face: &[u8],
    out: &mut [Glyph],
) -> Result<(ReplyHead, usize), Refusal> {
    answer(faces, &Request::from_bytes(&request.to_bytes())?, face, out)
}

/// [`shape`], for a request the decoder has already admitted.
fn answer(
    faces: &[[u8; 32]],
    request: &Request,
    face: &[u8],
    out: &mut [Glyph],
) -> Result<(ReplyHead, usize), Refusal> {
    // The declaration first, then the name, then the shape, then the import:
    // each is a cheaper disbelief than the next, and the import is handed
    // nothing the first three did not pass.
    if !faces.contains(&request.face) {
        return Err(Refusal::FaceNotHeld);
    }
    if f_hash::sha256(face) != request.face {
        return Err(Refusal::FaceNotHeld);
    }
    let admitted = f_text::face::Face::read(face).map_err(|_| Refusal::NotAFace)?;
    let font = harfrust::FontRef::new(face).map_err(|_| Refusal::NotAFace)?;

    // `None` only for a zero tag, which the decoder has refused; mapped rather
    // than unwrapped, because the import's signature is what it is and a panic
    // here would be a fault for a request the protocol calls malformed.
    let script = harfrust::Script::from_iso15924_tag(harfrust::Tag::new(&request.script))
        .ok_or(Refusal::Malformed)?;
    // The decoder admitted these two and no third.
    let direction = match request.direction {
        shape::direction::RIGHT_TO_LEFT => harfrust::Direction::RightToLeft,
        _ => harfrust::Direction::LeftToRight,
    };
    let features: Vec<harfrust::Feature> = request
        .features()
        .iter()
        .map(|feature| harfrust::Feature::new(harfrust::Tag::new(&feature.tag), feature.value, ..))
        .collect();

    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(request.text());
    buffer.set_direction(direction);
    buffer.set_script(script);

    let data = harfrust::ShaperData::new(&font);
    let shaper = data.shaper(&font).build();
    let shaped = shaper.shape(buffer, harfrust::ShapeOptions::new().features(&features));

    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    let count = infos.len();
    if count > shape::GLYPHS_MAX || count > out.len() || positions.len() != count {
        return Err(Refusal::TooManyGlyphs);
    }
    for ((slot, info), at) in out.iter_mut().zip(infos).zip(positions) {
        *slot = glyph(info, at);
    }
    let head = ReplyHead { units_per_em: admitted.units_per_em(), glyphs: count as u32 };
    Ok((head, count))
}

/// One glyph as the import answered it, as the protocol carries it: a copy
/// between integer types, field to field, and nothing else.
///
/// A function of its own so that its test can hand it a position whose four
/// fields all differ. `tests/run.rs` shapes real runs, and in those
/// `y_advance` is always zero — Inter carries no `GPOS` `YAdvance` and the
/// protocol no vertical direction — so a copy that dropped or swapped it would
/// pass every run there.
fn glyph(info: &harfrust::GlyphInfo, at: &harfrust::GlyphPosition) -> Glyph {
    Glyph {
        glyph: info.glyph_id,
        cluster: info.cluster,
        x_advance_design_units: at.x_advance,
        y_advance_design_units: at.y_advance,
        x_offset_design_units: at.x_offset,
        y_offset_design_units: at.y_offset,
    }
}

/// Answer one encoded request with one encoded reply, and say how long it is.
///
/// This is the protocol end to end on one side of the ring: bytes in, bytes out,
/// and nothing between them a peer could observe except the reply. `faces` is
/// what this component may load — [`FACES`] in the component. `face` is what the
/// blob store handed back for the request's address — the shim checks it rather
/// than trusting whoever looked it up.
///
/// # Errors
///
/// Whatever [`Request::from_bytes`] or [`shape`] refuses, and
/// [`Refusal::TooManyGlyphs`] when `reply` cannot hold the answer.
pub fn serve(
    faces: &[[u8; 32]],
    request: &[u8; Request::BYTES],
    face: &[u8],
    reply: &mut [u8],
) -> Result<usize, Refusal> {
    let request = Request::from_bytes(request)?;
    let mut glyphs = [Glyph {
        glyph: 0,
        cluster: 0,
        x_advance_design_units: 0,
        y_advance_design_units: 0,
        x_offset_design_units: 0,
        y_offset_design_units: 0,
    }; shape::GLYPHS_MAX];
    let (head, count) = answer(faces, &request, face, &mut glyphs)?;
    let total = ReplyHead::BYTES + count * Glyph::BYTES;
    let out = reply.get_mut(..total).ok_or(Refusal::TooManyGlyphs)?;
    out[..ReplyHead::BYTES].copy_from_slice(&head.to_bytes());
    for (index, glyph) in glyphs[..count].iter().enumerate() {
        let at = ReplyHead::BYTES + index * Glyph::BYTES;
        out[at..at + Glyph::BYTES].copy_from_slice(&glyph.to_bytes());
    }
    Ok(total)
}

// A run the cache holds is a run this protocol carries: one number written twice
// across two crates that may not depend on each other, held equal here, in the
// one crate that takes both.
const _: () = assert!(shape::TEXT_BYTES_MAX == f_text::cache::TEXT_BYTES_MAX);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_position_reaches_its_own_field() {
        let mut info = harfrust::GlyphInfo::default();
        info.glyph_id = 1805;
        info.cluster = 6;
        let mut at = harfrust::GlyphPosition::default();
        at.x_advance = 1;
        at.y_advance = -2;
        at.x_offset = 3;
        at.y_offset = -4;
        assert_eq!(
            glyph(&info, &at),
            Glyph {
                glyph: 1805,
                cluster: 6,
                x_advance_design_units: 1,
                y_advance_design_units: -2,
                x_offset_design_units: 3,
                y_offset_design_units: -4,
            }
        );
    }
}
