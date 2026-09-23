// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A face, as bytes a store hands back — and the smallest thing that can be
//! called reading one.
//!
//! # What this is, and what it deliberately is not
//!
//! `E3-B03b` says a face is *addressed by content hash and declared before it is
//! used*. That is a statement about **provenance**, not about OpenType: the
//! question it settles is whether a component may load a given run of bytes at
//! all, and the answer has to be decidable before a single byte moves. So the
//! part of a face this module knows is the part a loader must agree about with
//! whoever stocked it — a magic, a schema, a design-unit grid, and one advance
//! per glyph — and nothing else.
//!
//! **It is not a font parser and must not grow into one.** `E3-B03a` decides
//! where the shaper comes from and `E3-B03e` onwards is where a real table
//! directory, a `cmap` and a `glyf` arrive. When a shaper lands, the thing it
//! parses is a face this module has already *admitted*, and the split stays: one
//! reader decides whether these bytes are the ones the manifest named, another
//! decides what they mean. Collapsing the two would make the provenance check
//! depend on a parser's opinion, and a parser is exactly the thing that will one
//! day be handed a hostile file.
//!
//! *What would reverse this module:* a real face format arriving whose own
//! header carries a units-per-em and a glyph count. At that point [`Face::read`]
//! reads *that* header instead of this one and the format below is deleted
//! rather than kept beside it — two face formats in one tree is two readers with
//! different beliefs about one blob, which is the failure RFC 0030 wrote the
//! component file to avoid one level up.
//!
//! # Determinism
//!
//! No clock, no draw, no map, no float. An advance is a design-unit integer and
//! [`crate::metric`] is where it stops being one; this module never divides.

/// The first eight bytes of a face.
///
/// A blob store holds runs of bytes with no kind in them, so the only thing that
/// distinguishes a face from an object that happens to be the right length is a
/// pattern at its front. A blob that is not one is refused rather than read
/// approximately.
pub const MAGIC: u64 = 0x465f_4641_4345_0001;

/// The schema this build knows, and the only value a face may carry.
///
/// One. A later schema is refused rather than read approximately, for
/// `f_abi::manifest::SCHEMA`'s reason: a reader that guesses at fields it was
/// not written for is two readers with different beliefs about one face.
pub const SCHEMA: u32 = 1;

/// Bytes before the first advance.
///
/// Twenty-four, and every one of them is judged. There is no padding here on
/// purpose — the whole point of the task this serves is that the face's *bytes*
/// are its name, and a byte nobody reads is a byte two faces can differ in while
/// hashing to two addresses that mean one thing.
/// Unit: bytes.
pub const HEAD_BYTES: usize = 24;

/// Bytes in one advance.
/// Unit: bytes.
pub const ADVANCE_BYTES: usize = 2;

/// The most glyphs a face of this format may carry.
///
/// A thousand and twenty-four, and the bound is here because a length that is a
/// function of a count read out of the blob is a length the blob's author
/// chooses. It is not a statement about typography — a real face has far more —
/// it is a statement about what this format is for, which is a face small enough
/// that a component can hold one in a buffer it declared. The reversal is the
/// same one the module comment states: a real face format arrives and this
/// number goes with the rest of it.
/// Unit: glyphs.
pub const GLYPHS_MAX: usize = 1024;

/// The largest design-unit grid this build will believe.
///
/// Sixteen thousand three hundred and eighty-four, which is what a font format's
/// units-per-em is bounded at wherever anybody here has read one. Zero is
/// refused because every advance in the face would then be a division by it the
/// moment [`crate::metric`] scaled one.
/// Unit: design units per em.
pub const UNITS_PER_EM_MAX: u32 = 16_384;

/// Why a run of bytes is not a face.
///
/// One variant per thing a reader can disbelieve, for
/// `f_abi::manifest::Refusal`'s reason: a refusal that says only *malformed* is
/// a refusal somebody debugs by bisecting a blob.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// Shorter than a head, or a length the head's own count does not agree
    /// with — in either direction.
    Truncated,
    /// The first eight bytes are not [`MAGIC`]. Not a face.
    NotAFace,
    /// A schema this build does not know.
    Schema,
    /// A count is zero or past [`GLYPHS_MAX`].
    Count,
    /// A quantity is out of range: a zero units-per-em, or one past
    /// [`UNITS_PER_EM_MAX`].
    Quantity,
    /// A reserved field is not zero. R04.
    Reserved,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Truncated => "the blob is not the length the face it declares would be",
            Self::NotAFace => "the blob does not begin with the face magic",
            Self::Schema => "the face is written to a schema this build does not know",
            Self::Count => "the glyph count is zero or past this build's bound",
            Self::Quantity => "the design-unit grid is zero or past this build's bound",
            Self::Reserved => "a reserved field is not zero",
        }
    }
}

/// A face, borrowed out of the bytes a store handed back.
///
/// Nothing is copied and nothing is allocated: the advances stay where the store
/// left them. That is the property that makes it affordable for a component to
/// re-read a face rather than cache a parse of it, which matters here for the
/// reason it matters in `f_abi::manifest` — a cached parse is a second belief
/// about a blob, and the address names the blob.
/// Two faces are equal when their grid and their advances are, which is the
/// only reading that agrees with the address: the bytes are the name, and there
/// is nothing in this type that is not in the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face<'a> {
    /// The design-unit grid. Unit: design units per em.
    units_per_em: u32,
    /// One advance per glyph, little-endian, as the blob carries them.
    /// Unit: bytes.
    advances: &'a [u8],
}

impl<'a> Face<'a> {
    /// Believe a run of bytes, or say why not.
    ///
    /// The length is checked **exactly**. A trailing byte is a byte no reader
    /// judges, and the whole of this task is that the bytes are the name: two
    /// blobs that differ only in a byte nobody reads would hash to two addresses
    /// naming one face, and then a declaration would not decide anything.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming which disbelief. Fail closed, R04.
    pub fn read(bytes: &'a [u8]) -> Result<Self, Refusal> {
        let head = bytes.get(..HEAD_BYTES).ok_or(Refusal::Truncated)?;
        let word = |at: usize| -> u32 {
            let mut out = [0u8; 4];
            out.copy_from_slice(&head[at..at + 4]);
            u32::from_le_bytes(out)
        };
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&head[..8]);
        if u64::from_le_bytes(magic) != MAGIC {
            return Err(Refusal::NotAFace);
        }
        if word(8) != SCHEMA {
            return Err(Refusal::Schema);
        }
        let units_per_em = word(12);
        if units_per_em == 0 || units_per_em > UNITS_PER_EM_MAX {
            return Err(Refusal::Quantity);
        }
        let glyphs = word(16) as usize;
        if glyphs == 0 || glyphs > GLYPHS_MAX {
            return Err(Refusal::Count);
        }
        if word(20) != 0 {
            return Err(Refusal::Reserved);
        }
        let advances = bytes
            .get(HEAD_BYTES..)
            .filter(|rest| rest.len() == glyphs * ADVANCE_BYTES)
            .ok_or(Refusal::Truncated)?;
        Ok(Self { units_per_em, advances })
    }

    /// The design-unit grid every advance in this face is in.
    /// Unit: design units per em.
    #[must_use]
    pub const fn units_per_em(&self) -> u32 {
        self.units_per_em
    }

    /// How many glyphs.
    /// Unit: glyphs.
    #[must_use]
    pub const fn glyphs(&self) -> usize {
        self.advances.len() / ADVANCE_BYTES
    }

    /// One glyph's advance, or `None` past the count.
    ///
    /// `None` and not zero, because zero is a real advance — a combining mark
    /// has one — so a reader that answered zero for a glyph this face does not
    /// carry would be handing back a plausible number for a question it could
    /// not answer.
    /// Unit: design units.
    #[must_use]
    pub fn advance_design_units(&self, glyph: usize) -> Option<u16> {
        let at = glyph.checked_mul(ADVANCE_BYTES)?;
        let pair = self.advances.get(at..at + ADVANCE_BYTES)?;
        Some(u16::from_le_bytes([pair[0], pair[1]]))
    }
}

/// How many bytes a face of `glyphs` glyphs occupies.
/// Unit: bytes.
#[must_use]
pub const fn bytes_for(glyphs: usize) -> usize {
    HEAD_BYTES + glyphs * ADVANCE_BYTES
}

/// Write a face into `into`, and answer how long it is.
///
/// # Why a writer is in the permissive tree at all
///
/// Because the thing being demonstrated is *addressing*, and a demonstration
/// whose subject is a file checked into the tree demonstrates the tree's file
/// rather than the addressing. A face composed here is a face whose bytes are a
/// function of its arguments, so the content address a manifest declares is
/// reproducible from the source by anybody, on either architecture, with no
/// binary in the repository — which is what makes the agreement between a
/// manifest and the component that stocks the face checkable at all.
///
/// *What would reverse it:* a real face in `third_party/`, reached over a ring
/// under RFC 0003. Then the address in a manifest names an imported file, this
/// function has no caller outside a test, and it goes.
///
/// # Errors
///
/// [`Refusal::Truncated`] if `into` is smaller than [`bytes_for`] says, and the
/// same refusals [`Face::read`] would produce for arguments it would not
/// believe — checked here rather than written and discovered later, because a
/// writer that can emit a blob its own reader refuses is a writer that will.
pub fn compose(into: &mut [u8], units_per_em: u32, advances: &[u16]) -> Result<usize, Refusal> {
    if units_per_em == 0 || units_per_em > UNITS_PER_EM_MAX {
        return Err(Refusal::Quantity);
    }
    if advances.is_empty() || advances.len() > GLYPHS_MAX {
        return Err(Refusal::Count);
    }
    let total = bytes_for(advances.len());
    let out = into.get_mut(..total).ok_or(Refusal::Truncated)?;
    out[..8].copy_from_slice(&MAGIC.to_le_bytes());
    out[8..12].copy_from_slice(&SCHEMA.to_le_bytes());
    out[12..16].copy_from_slice(&units_per_em.to_le_bytes());
    out[16..20].copy_from_slice(&(advances.len() as u32).to_le_bytes());
    out[20..24].copy_from_slice(&0u32.to_le_bytes());
    for (index, advance) in advances.iter().enumerate() {
        let at = HEAD_BYTES + index * ADVANCE_BYTES;
        out[at..at + ADVANCE_BYTES].copy_from_slice(&advance.to_le_bytes());
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The face every test here composes, and the one `user/objects` stocks.
    const ADVANCES: [u16; 4] = [0, 512, 1024, 600];

    /// One byte wrong in a composed face, and what reading it should earn.
    ///
    /// A named type rather than the tuple spelled at the use site, for the
    /// reason `f_abi::manifest::tests::Lie` is one: a signature a reader has to
    /// parse before they can read the cases is a signature in the way.
    type Lie = (&'static str, fn(&mut [u8]), Refusal);

    fn composed(into: &mut [u8]) -> usize {
        compose(into, 1000, &ADVANCES).expect("the fixture is one this writer admits")
    }

    #[test]
    fn a_composed_face_reads_back_as_what_was_written() {
        let mut buffer = [0u8; 64];
        let len = composed(&mut buffer);
        assert_eq!(len, bytes_for(ADVANCES.len()));
        let face = Face::read(&buffer[..len]).expect("a face this writer wrote");
        assert_eq!(face.units_per_em(), 1000);
        assert_eq!(face.glyphs(), ADVANCES.len());
        for (glyph, expect) in ADVANCES.iter().enumerate() {
            assert_eq!(face.advance_design_units(glyph), Some(*expect));
        }
        assert_eq!(face.advance_design_units(ADVANCES.len()), None, "past the count is not zero");
    }

    #[test]
    fn every_structural_lie_is_refused() {
        // One byte wrong at a time, because a fixture that breaks two things is
        // caught by whichever check notices first and the check it was written
        // for stays unexercised. `f_abi::manifest`'s own table says the same.
        let mut sound = [0u8; 64];
        let len = composed(&mut sound);
        let cases: &[Lie] = &[
            ("magic", |b| b[0] ^= 1, Refusal::NotAFace),
            ("schema", |b| b[8] = 2, Refusal::Schema),
            ("a zero grid", |b| b[12..16].copy_from_slice(&0u32.to_le_bytes()), Refusal::Quantity),
            (
                "a grid past the bound",
                |b| b[12..16].copy_from_slice(&(UNITS_PER_EM_MAX + 1).to_le_bytes()),
                Refusal::Quantity,
            ),
            ("no glyphs", |b| b[16..20].copy_from_slice(&0u32.to_le_bytes()), Refusal::Count),
            (
                "more glyphs than the bound",
                |b| b[16..20].copy_from_slice(&(GLYPHS_MAX as u32 + 1).to_le_bytes()),
                Refusal::Count,
            ),
            ("a reserved word", |b| b[20] = 1, Refusal::Reserved),
            (
                "a count the blob cannot carry",
                |b| b[16..20].copy_from_slice(&5u32.to_le_bytes()),
                Refusal::Truncated,
            ),
        ];
        for (what, break_it, expect) in cases {
            let mut broken = sound;
            break_it(&mut broken[..len]);
            assert_eq!(
                Face::read(&broken[..len]),
                Err(*expect),
                "{what} was not refused the way it should be"
            );
        }
    }

    #[test]
    fn a_trailing_byte_is_refused_rather_than_ignored() {
        // The load-bearing one: the bytes are the name, so a blob with a byte
        // nobody judges is a blob two addresses could name one meaning of.
        let mut buffer = [0u8; 64];
        let len = composed(&mut buffer);
        assert!(Face::read(&buffer[..len]).is_ok());
        assert_eq!(Face::read(&buffer[..len + 1]), Err(Refusal::Truncated));
        assert_eq!(Face::read(&buffer[..len - 1]), Err(Refusal::Truncated));
    }

    #[test]
    fn the_writer_refuses_what_its_own_reader_would() {
        let mut buffer = [0u8; 64];
        assert_eq!(compose(&mut buffer, 0, &ADVANCES), Err(Refusal::Quantity));
        assert_eq!(compose(&mut buffer, UNITS_PER_EM_MAX + 1, &ADVANCES), Err(Refusal::Quantity));
        assert_eq!(compose(&mut buffer, 1000, &[]), Err(Refusal::Count));
        assert_eq!(compose(&mut buffer[..8], 1000, &ADVANCES), Err(Refusal::Truncated));
    }

    #[test]
    fn one_advance_changed_is_a_different_face() {
        // The whole of the undeclared half, at this level: the two blobs are
        // both readable, both well formed, and differ in two bytes. Nothing
        // about *reading* one tells them apart, which is why the declaration is
        // an address and not a shape.
        let mut first = [0u8; 64];
        let mut second = [0u8; 64];
        let len = composed(&mut first);
        let mut other = ADVANCES;
        other[1] += 1;
        compose(&mut second, 1000, &other).expect("also a face");
        assert!(Face::read(&first[..len]).is_ok());
        assert!(Face::read(&second[..len]).is_ok());
        assert_ne!(first, second);
    }
}
