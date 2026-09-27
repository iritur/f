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
//! whoever stocked it — a magic, a version, a design-unit grid, a glyph count,
//! and one advance per glyph — and nothing else.
//!
//! **It is not a font parser and must not grow into one.** The shaper parses a
//! face (RFC 0082, `user/shaper`), and what it parses is a face this module has
//! already *admitted*: `f_shaper::shape` refuses an address its manifest does
//! not declare, hashes the bytes against the address it was asked for, calls
//! [`Face::read`], and only then hands them to the import.
//! The split stays — one reader decides whether these bytes are a face this tree
//! will hold, another decides what they mean — because a provenance check that
//! depended on a parser's opinion would be a parser, and a parser is exactly the
//! thing that will one day be handed a hostile file.
//!
//! # The format gave way on the day a real face arrived
//!
//! Until `E3-B03b0` this module read a format of its own — an eight-byte magic,
//! a schema, a grid, a count and a list of advances — and said what would end
//! it: *a real face format arriving whose own header carries a units-per-em and
//! a glyph count. At that point `Face::read` reads that header instead of this
//! one and the format below is deleted rather than kept beside it.* Inter
//! arrived (`third_party/inter/`, 2,937 glyphs, which the old format's bound of
//! 1,024 refused), and its own header carries both. So this module now reads
//! the OpenType font file format — the `sfnt` table directory, and from it
//! `head`, `hhea`, `maxp` and `hmtx` — and the old format is gone rather than
//! kept beside it. RFC 0141.
//!
//! Four tables and not one more. `head` for the grid and the magic that says
//! the file is a font; `maxp` for the count; `hhea` and `hmtx` for the advances,
//! which is what the old format carried and what [`Face::advance_design_units`]
//! still answers. `cmap`, `glyf`, `GSUB` and `GPOS` are the shaper's.
//!
//! **What does not survive the change: *every byte is judged*.** The old format
//! could say it, because it had no byte a reader did not read. A real face has
//! hundreds of kilobytes this module never looks at — outlines, layout tables,
//! hinting programs — so the claim is narrowed to what still holds: every byte
//! is *named* (the content address covers all of them, which is `E3-B03b`'s
//! property and was never this module's), the directory is judged whole, every
//! table it names lies inside the blob, and nothing trails the last table but
//! the zero padding the format requires. A blob with a byte past that is two
//! addresses for one face, and is refused, as it was before.
//!
//! # Why TrueType outlines and nothing else
//!
//! [`Face::read`] admits a file whose `sfntVersion` is `0x00010000` — TrueType
//! outlines — and refuses `OTTO` (CFF outlines), a collection and every
//! wrapper. Not because the others are worse: because the one face this tree
//! holds is TrueType, `cargo xtask lint-licensing` admits a `.ttf` into a data
//! import only when its first four bytes say so, and a reader that admitted a
//! format nothing in the tree carries would be a claim with no instance.
//! *Reversal:* the first CFF face imported, which is one constant here and one
//! word in that lint's rule, argued in the same diff.
//!
//! # Determinism
//!
//! No clock, no draw, no map, no float. An advance is a design-unit integer and
//! [`crate::metric`] is where it stops being one; this module never divides.

/// The `sfntVersion` of a font with TrueType outlines: the only kind admitted.
/// Unit: none — a fixed bit pattern, big-endian in the file.
pub const SFNT_TRUETYPE: u32 = 0x0001_0000;

/// `head.magicNumber`, which every OpenType font carries at the same offset.
/// Unit: none — a fixed bit pattern.
pub const HEAD_MAGIC: u32 = 0x5f0f_3cf5;

/// Bytes before the first table record: `sfntVersion`, `numTables` and the
/// three search fields.
/// Unit: bytes.
pub const DIRECTORY_HEAD_BYTES: usize = 12;

/// Bytes in one table record: a tag, a checksum, an offset and a length.
/// Unit: bytes.
pub const TABLE_RECORD_BYTES: usize = 16;

/// Bytes in one `hmtx` long metric: an advance and a left side bearing.
/// Unit: bytes.
pub const METRIC_BYTES: usize = 4;

/// The most glyphs a face may carry.
///
/// Sixty-five thousand five hundred and thirty-five, which is the format's own
/// bound — `maxp.numGlyphs` is sixteen bits — and not a number this tree chose.
/// The old format's 1,024 was a statement about a face small enough to copy
/// into a buffer a component declared; this reader borrows and copies nothing,
/// so the only length it computes from a count is checked against the blob's
/// own length before it is used.
/// Unit: glyphs.
pub const GLYPHS_MAX: usize = u16::MAX as usize;

/// The smallest design-unit grid this build will believe.
///
/// Sixteen, which is the OpenType specification's lower bound for
/// `head.unitsPerEm` and [`crate::metric::UPEM_MIN`]: a face this module admits
/// is one the metric module can scale.
/// Unit: design units per em.
pub const UNITS_PER_EM_MIN: u32 = 16;

/// The largest design-unit grid this build will believe.
///
/// Sixteen thousand three hundred and eighty-four, the specification's upper
/// bound for `head.unitsPerEm` and [`crate::metric::UPEM_MAX`].
/// Unit: design units per em.
pub const UNITS_PER_EM_MAX: u32 = 16_384;

/// Why a run of bytes is not a face.
///
/// One variant per thing a reader can disbelieve, for
/// `f_abi::manifest::Refusal`'s reason: a refusal that says only *malformed* is
/// a refusal somebody debugs by bisecting a blob.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// Shorter than its directory, a table that runs past the end, a table too
    /// short for the fields read from it, or bytes after the last table other
    /// than its zero padding.
    Truncated,
    /// Not a font with TrueType outlines: the `sfntVersion` is something else,
    /// or `head.magicNumber` is not [`HEAD_MAGIC`].
    NotAFace,
    /// A table this module reads is written to a version it does not know.
    Schema,
    /// A count is out of range: no tables, no glyphs, or more long metrics
    /// than glyphs.
    Count,
    /// The design-unit grid is outside [`UNITS_PER_EM_MIN`] to
    /// [`UNITS_PER_EM_MAX`].
    Quantity,
    /// A reserved field is not zero. R04.
    Reserved,
    /// The directory names a table twice, or out of order — two answers to
    /// *where is `hmtx`*, which is two readers with different beliefs.
    Directory,
    /// A table this module reads is not in the directory.
    Missing,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Truncated => "the blob is not the length the face it declares would be",
            Self::NotAFace => "the blob is not a font with TrueType outlines",
            Self::Schema => "a table is written to a version this build does not know",
            Self::Count => "a count in the face is zero or out of range",
            Self::Quantity => "the design-unit grid is outside the format's bounds",
            Self::Reserved => "a reserved field is not zero",
            Self::Directory => "the table directory is out of order or names a table twice",
            Self::Missing => "a table this build reads is not in the face",
        }
    }
}

/// A big-endian word at `at`, or `None` past the end.
fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let pair = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([pair[0], pair[1]]))
}

/// A big-endian double word at `at`, or `None` past the end.
fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let quad = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([quad[0], quad[1], quad[2], quad[3]]))
}

/// A face, borrowed out of the bytes a store handed back.
///
/// Nothing is copied and nothing is allocated: the advances stay where the store
/// left them. That is the property that makes it affordable for a component to
/// re-read a face rather than cache a parse of it — a cached parse is a second
/// belief about a blob, and the address names the blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face<'a> {
    /// The design-unit grid. Unit: design units per em.
    units_per_em: u32,
    /// `maxp.numGlyphs`. Unit: glyphs.
    glyphs: u16,
    /// `hhea.numberOfHMetrics`. Unit: long metrics.
    metrics: u16,
    /// The `hmtx` table, checked long enough for both counts.
    /// Unit: bytes.
    hmtx: &'a [u8],
}

impl<'a> Face<'a> {
    /// Believe a run of bytes, or say why not.
    ///
    /// The directory is judged whole — every record, in order, inside the blob —
    /// before any table is read, and the blob must end where its last table
    /// does, padded with zeros to a multiple of four as the format requires.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming which disbelief. Fail closed, R04.
    pub fn read(bytes: &'a [u8]) -> Result<Self, Refusal> {
        let version = be32(bytes, 0).ok_or(Refusal::Truncated)?;
        if version != SFNT_TRUETYPE {
            return Err(Refusal::NotAFace);
        }
        let tables = usize::from(be16(bytes, 4).ok_or(Refusal::Truncated)?);
        if tables == 0 {
            return Err(Refusal::Count);
        }
        let directory = DIRECTORY_HEAD_BYTES + tables * TABLE_RECORD_BYTES;
        if bytes.len() < directory {
            return Err(Refusal::Truncated);
        }
        let mut end = directory;
        let mut previous: Option<u32> = None;
        let (mut head, mut hhea, mut hmtx, mut maxp) = (None, None, None, None);
        for index in 0..tables {
            let at = DIRECTORY_HEAD_BYTES + index * TABLE_RECORD_BYTES;
            let tag = be32(bytes, at).ok_or(Refusal::Truncated)?;
            if previous.is_some_and(|before| before >= tag) {
                return Err(Refusal::Directory);
            }
            previous = Some(tag);
            let offset = be32(bytes, at + 8).ok_or(Refusal::Truncated)? as usize;
            let length = be32(bytes, at + 12).ok_or(Refusal::Truncated)? as usize;
            let stop = offset.checked_add(length).ok_or(Refusal::Truncated)?;
            let table = bytes.get(offset..stop).ok_or(Refusal::Truncated)?;
            end = end.max(stop);
            match &tag.to_be_bytes() {
                b"head" => head = Some(table),
                b"hhea" => hhea = Some(table),
                b"hmtx" => hmtx = Some(table),
                b"maxp" => maxp = Some(table),
                _ => {}
            }
        }
        // Nothing after the last table but the format's own padding. The
        // content address covers these bytes; this is what makes two blobs
        // that differ only in them two faces rather than one face with two
        // names.
        let padded = end.next_multiple_of(4);
        if bytes.len() != padded || bytes[end..].iter().any(|b| *b != 0) {
            return Err(Refusal::Truncated);
        }

        let head = head.ok_or(Refusal::Missing)?;
        if be16(head, 0).ok_or(Refusal::Truncated)? != 1 {
            return Err(Refusal::Schema);
        }
        if be32(head, 12).ok_or(Refusal::Truncated)? != HEAD_MAGIC {
            return Err(Refusal::NotAFace);
        }
        let units_per_em = u32::from(be16(head, 18).ok_or(Refusal::Truncated)?);
        if !(UNITS_PER_EM_MIN..=UNITS_PER_EM_MAX).contains(&units_per_em) {
            return Err(Refusal::Quantity);
        }

        let maxp = maxp.ok_or(Refusal::Missing)?;
        if !matches!(be32(maxp, 0).ok_or(Refusal::Truncated)?, 0x0000_5000 | 0x0001_0000) {
            return Err(Refusal::Schema);
        }
        let glyphs = be16(maxp, 4).ok_or(Refusal::Truncated)?;
        if glyphs == 0 {
            return Err(Refusal::Count);
        }

        let hhea = hhea.ok_or(Refusal::Missing)?;
        if be16(hhea, 0).ok_or(Refusal::Truncated)? != 1 {
            return Err(Refusal::Schema);
        }
        // Four reserved words and `metricDataFormat`, all zero by the
        // specification's own words.
        for at in [24, 26, 28, 30, 32] {
            if be16(hhea, at).ok_or(Refusal::Truncated)? != 0 {
                return Err(Refusal::Reserved);
            }
        }
        let metrics = be16(hhea, 34).ok_or(Refusal::Truncated)?;
        if metrics == 0 || metrics > glyphs {
            return Err(Refusal::Count);
        }

        let hmtx = hmtx.ok_or(Refusal::Missing)?;
        let needed = usize::from(metrics) * METRIC_BYTES + usize::from(glyphs - metrics) * 2;
        if hmtx.len() < needed {
            return Err(Refusal::Truncated);
        }
        Ok(Self { units_per_em, glyphs, metrics, hmtx })
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
        self.glyphs as usize
    }

    /// One glyph's advance, or `None` past the count.
    ///
    /// `None` and not zero, because zero is a real advance — a combining mark
    /// has one — so a reader that answered zero for a glyph this face does not
    /// carry would be handing back a plausible number for a question it could
    /// not answer. A glyph past `hhea.numberOfHMetrics` takes the last long
    /// metric's advance, as the format says.
    /// Unit: design units.
    #[must_use]
    pub fn advance_design_units(&self, glyph: usize) -> Option<u16> {
        if glyph >= self.glyphs() {
            return None;
        }
        let index = glyph.min(usize::from(self.metrics) - 1);
        be16(self.hmtx, index * METRIC_BYTES)
    }
}

// The layout [`compose`] writes: a directory of four records, then the four
// tables in tag order, each at a multiple of four.
const HEAD_BYTES: usize = 54;
const HHEA_BYTES: usize = 36;
const MAXP_BYTES: usize = 6;
const COMPOSED_TABLES: usize = 4;
const COMPOSED_DIRECTORY: usize = DIRECTORY_HEAD_BYTES + COMPOSED_TABLES * TABLE_RECORD_BYTES;

/// How many bytes a face of `glyphs` glyphs occupies as [`compose`] writes it.
/// Unit: bytes.
#[must_use]
pub const fn bytes_for(glyphs: usize) -> usize {
    let head = COMPOSED_DIRECTORY;
    let hhea = head + HEAD_BYTES.next_multiple_of(4);
    let hmtx = hhea + HHEA_BYTES;
    let maxp = hmtx + glyphs * METRIC_BYTES;
    (maxp + MAXP_BYTES).next_multiple_of(4)
}

/// The OpenType table checksum: the sum of the table's big-endian words, the
/// last one padded with zeros.
fn checksum(table: &[u8]) -> u32 {
    table.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// Write a face into `into`, and answer how long it is.
///
/// # What it writes
///
/// The smallest file [`Face::read`] admits: a directory and four tables —
/// `head`, `hhea`, `hmtx` and `maxp` — with the grid and the advances given and
/// every other field zero or the specification's fixed value. It has no `cmap`,
/// no outlines and no layout tables, so no shaper and no rasteriser would do
/// anything with it; what it is for is *addressing*, and it is a real instance
/// of the one format this module reads rather than a second format kept beside
/// it. Its checksums are right, because a file that claimed to be a font with a
/// wrong one would be a lie about a format this module otherwise tells the
/// truth about.
///
/// # Why a writer is in the permissive tree at all
///
/// Because the thing being demonstrated is *addressing*, and the frame cannot
/// reach the face that is not composed. `E3-B03b`'s boot has the frame compose
/// the declared face and hash it itself; the frame cannot read
/// `third_party/inter/` (RFC 0081, RFC 0082), so the face whose address it
/// computes has to be one it can build from constants. A face composed here is a
/// function of its arguments, so the content address a manifest declares is
/// reproducible from the source by anybody, on either architecture, with no
/// binary in the repository.
///
/// *What would reverse it:* a boot in which the frame checks an address it did
/// not compute — a face that arrives from outside the build, stocked by a
/// component and declared by a hash the frame is handed rather than derives.
/// Then `user/objects/manifest.toml`'s `[[face]]` names an imported file, this
/// function has no caller outside a test, and it goes. That is `E3-B03b`'s
/// boot changing shape, and it is not this function's to decide.
///
/// # Errors
///
/// [`Refusal::Truncated`] if `into` is smaller than [`bytes_for`] says, and the
/// same refusals [`Face::read`] would produce for arguments it would not
/// believe — checked here rather than written and discovered later, because a
/// writer that can emit a blob its own reader refuses is a writer that will.
pub fn compose(into: &mut [u8], units_per_em: u32, advances: &[u16]) -> Result<usize, Refusal> {
    if !(UNITS_PER_EM_MIN..=UNITS_PER_EM_MAX).contains(&units_per_em) {
        return Err(Refusal::Quantity);
    }
    if advances.is_empty() || advances.len() > GLYPHS_MAX {
        return Err(Refusal::Count);
    }
    let total = bytes_for(advances.len());
    let out = into.get_mut(..total).ok_or(Refusal::Truncated)?;
    out.fill(0);
    let glyphs = advances.len() as u16;

    let head_at = COMPOSED_DIRECTORY;
    let hhea_at = head_at + HEAD_BYTES.next_multiple_of(4);
    let hmtx_at = hhea_at + HHEA_BYTES;
    let maxp_at = hmtx_at + advances.len() * METRIC_BYTES;
    let hmtx_len = advances.len() * METRIC_BYTES;

    // The directory: four tables, so the search fields are those of the
    // largest power of two not above four — the specification's arithmetic.
    out[0..4].copy_from_slice(&SFNT_TRUETYPE.to_be_bytes());
    out[4..6].copy_from_slice(&(COMPOSED_TABLES as u16).to_be_bytes());
    out[6..8].copy_from_slice(&64u16.to_be_bytes());
    out[8..10].copy_from_slice(&2u16.to_be_bytes());
    out[10..12].copy_from_slice(&0u16.to_be_bytes());

    // `head`: version 1.0, revision 1.0, the magic, the grid, and a direction
    // hint of 2 (the value the specification says to use).
    let head = &mut out[head_at..head_at + HEAD_BYTES];
    head[0..2].copy_from_slice(&1u16.to_be_bytes());
    head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&HEAD_MAGIC.to_be_bytes());
    head[18..20].copy_from_slice(&(units_per_em as u16).to_be_bytes());
    head[48..50].copy_from_slice(&2i16.to_be_bytes());

    // `hhea`: version 1.0, the widest advance, a caret straight up, and every
    // long metric one per glyph.
    let widest = advances.iter().copied().max().unwrap_or(0);
    let hhea = &mut out[hhea_at..hhea_at + HHEA_BYTES];
    hhea[0..2].copy_from_slice(&1u16.to_be_bytes());
    hhea[10..12].copy_from_slice(&widest.to_be_bytes());
    hhea[18..20].copy_from_slice(&1i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&glyphs.to_be_bytes());

    // `hmtx`: each advance, and a zero side bearing.
    for (index, advance) in advances.iter().enumerate() {
        let at = hmtx_at + index * METRIC_BYTES;
        out[at..at + 2].copy_from_slice(&advance.to_be_bytes());
    }

    // `maxp` version 0.5, which is the count and nothing else.
    out[maxp_at..maxp_at + 4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    out[maxp_at + 4..maxp_at + 6].copy_from_slice(&glyphs.to_be_bytes());

    let records = [
        (*b"head", head_at, HEAD_BYTES),
        (*b"hhea", hhea_at, HHEA_BYTES),
        (*b"hmtx", hmtx_at, hmtx_len),
        (*b"maxp", maxp_at, MAXP_BYTES),
    ];
    for (index, (tag, at, len)) in records.iter().enumerate() {
        let sum = checksum(&out[*at..*at + *len]);
        let record = DIRECTORY_HEAD_BYTES + index * TABLE_RECORD_BYTES;
        out[record..record + 4].copy_from_slice(tag);
        out[record + 4..record + 8].copy_from_slice(&sum.to_be_bytes());
        out[record + 8..record + 12].copy_from_slice(&(*at as u32).to_be_bytes());
        out[record + 12..record + 16].copy_from_slice(&(*len as u32).to_be_bytes());
    }
    // `head.checkSumAdjustment`, over the whole file with the field zero, as
    // the specification defines it.
    let adjustment = 0xb1b0_afbau32.wrapping_sub(checksum(out));
    out[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
    Ok(total)
}

/// The two faces this tree composes, and the one place their advances are
/// written.
///
/// # Why this is in `text/` and not in the component that stocks them
///
/// Because three crates have to agree on them and only two of those could share
/// a definition any other way. `user/objects` composes the declared face and
/// puts it in its store; `user/objects/manifest.toml` declares the *address* of
/// that composition; and `kernel/src/objects.rs` computes the same address for
/// itself, refuses one the record does not declare, and reads the other back.
/// A second transcription of these numbers anywhere would be a second face with
/// a different address, and the symptom would be a read that resolves to nothing
/// rather than a diff anybody can see.
///
/// # Why there are two of them
///
/// Because a boot that only ever loads a declared face cannot tell a working
/// declaration from a check that answers yes. [`UNDECLARED`] is the same face
/// with one advance one design unit larger: it composes, it hashes, it is put in
/// the same store, [`Face::read`] believes it — and no `[[face]]` entry names
/// its address. Everything about it is real except the permission, which is the
/// only variable `E3-B03b`'s second clause is about.
///
/// *What would reverse this module:* [`compose`]'s reversal, and the same day.
pub mod fixture {
    use super::{Refusal, bytes_for, compose};

    /// The design-unit grid both faces are written on.
    ///
    /// A thousand, which is the CFF convention rather than TrueType's 2048. The
    /// choice matters only in that it is written down once: the address in
    /// `user/objects/manifest.toml` is a function of this number, so moving it
    /// is a red test rather than a silent second face.
    /// Unit: design units per em.
    pub const UNITS_PER_EM: u32 = 1000;

    /// The advances of the face `user/objects/manifest.toml` declares.
    ///
    /// Four, and the values are chosen to be distinguishable rather than
    /// typographic: a zero advance, because zero is a real advance and a reader
    /// answering it for a glyph it does not carry has to be caught; and three
    /// that differ from each other, so that a reader returning the wrong index
    /// is caught too.
    /// Unit: design units.
    pub const DECLARED: [u16; 4] = [0, 512, 1024, 600];

    /// The same face with one advance one design unit larger, declared by
    /// nothing.
    ///
    /// One unit and not a hundred, deliberately: the difference has to be the
    /// smallest one the format can carry, so that what separates the two faces
    /// is their *address* and nothing a reader could plausibly notice about
    /// their contents.
    /// Unit: design units.
    pub const UNDECLARED: [u16; 4] = [0, 513, 1024, 600];

    /// How many bytes either of them occupies, and therefore how large a buffer
    /// a caller needs.
    /// Unit: bytes.
    pub const BYTES: usize = bytes_for(DECLARED.len());

    /// Write the declared face into `into`, and answer how long it is.
    ///
    /// # Errors
    ///
    /// [`Refusal::Truncated`] if `into` is shorter than [`BYTES`]. No other
    /// refusal is reachable for these constants, and the `const` block below
    /// this module is what holds them to that.
    pub fn declared(into: &mut [u8]) -> Result<usize, Refusal> {
        compose(into, UNITS_PER_EM, &DECLARED)
    }

    /// Write the undeclared twin into `into`, and answer how long it is.
    ///
    /// # Errors
    ///
    /// As [`declared`].
    pub fn undeclared(into: &mut [u8]) -> Result<usize, Refusal> {
        compose(into, UNITS_PER_EM, &UNDECLARED)
    }
}

// The twin differs from the declared face and differs by one advance, checked
// by the machine rather than by reading two lists. A fixture whose two halves
// drifted into being the same face would make `E3-B03b`'s refusal a boot that
// refused the face it had just loaded, and every assertion about it would still
// pass.
const _: () = assert!(fixture::DECLARED.len() == fixture::UNDECLARED.len());
// And the two things that make `Truncated` the only refusal either writer can
// answer, which is what their doc comments claim: a grid inside the format's
// bounds and a glyph count inside them.
const _: () =
    assert!(fixture::UNITS_PER_EM >= UNITS_PER_EM_MIN && fixture::UNITS_PER_EM <= UNITS_PER_EM_MAX);
const _: () = assert!(!fixture::DECLARED.is_empty() && fixture::DECLARED.len() <= GLYPHS_MAX);
const _: () = assert!(fixture::DECLARED[1] != fixture::UNDECLARED[1]);
const _: () = assert!(fixture::DECLARED[0] == fixture::UNDECLARED[0]);
const _: () = assert!(fixture::DECLARED[2] == fixture::UNDECLARED[2]);
const _: () = assert!(fixture::DECLARED[3] == fixture::UNDECLARED[3]);
// The bounds this module reads against are the metric module's: a face admitted
// here is one `crate::metric::Scale` can be built for.
const _: () = assert!(UNITS_PER_EM_MIN == crate::metric::UPEM_MIN as u32);
const _: () = assert!(UNITS_PER_EM_MAX == crate::metric::UPEM_MAX as u32);

#[cfg(test)]
mod tests {
    use super::*;

    /// The face every test here composes, and the one `user/objects` stocks.
    ///
    /// Taken from [`fixture`] rather than spelled again, because a second
    /// transcription is a second face.
    const ADVANCES: [u16; 4] = fixture::DECLARED;

    /// Room for a composed face, with some to spare for the trailing-byte test.
    const ROOM: usize = fixture::BYTES + 8;

    /// One byte wrong in a composed face, and what reading it should earn.
    type Lie = (&'static str, fn(&mut [u8]), Refusal);

    fn composed(into: &mut [u8]) -> usize {
        compose(into, 1000, &ADVANCES).expect("the fixture is one this writer admits")
    }

    /// Where a composed face's four tables start, as `compose` lays them out.
    const HEAD_AT: usize = COMPOSED_DIRECTORY;
    const HHEA_AT: usize = HEAD_AT + 56;
    const HMTX_AT: usize = HHEA_AT + HHEA_BYTES;
    const MAXP_AT: usize = HMTX_AT + 4 * METRIC_BYTES;

    #[test]
    fn a_composed_face_reads_back_as_what_was_written() {
        let mut buffer = [0u8; ROOM];
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
    fn a_composed_face_carries_the_checksums_the_format_defines() {
        // The whole file sums to the specification's constant once
        // `checkSumAdjustment` is in place, and each record's checksum is its
        // table's.
        let mut buffer = [0u8; ROOM];
        let len = composed(&mut buffer);
        assert_eq!(checksum(&buffer[..len]), 0xb1b0_afba);
        for index in 0..COMPOSED_TABLES {
            let record = DIRECTORY_HEAD_BYTES + index * TABLE_RECORD_BYTES;
            let sum = be32(&buffer, record + 4).expect("a checksum");
            let at = be32(&buffer, record + 8).expect("an offset") as usize;
            let length = be32(&buffer, record + 12).expect("a length") as usize;
            let mut table = [0u8; ROOM];
            table[..length].copy_from_slice(&buffer[at..at + length]);
            if &buffer[record..record + 4] == b"head" {
                table[8..12].fill(0);
            }
            assert_eq!(checksum(&table[..length]), sum, "table {index}");
        }
    }

    #[test]
    fn every_structural_lie_is_refused() {
        // One byte wrong at a time, because a fixture that breaks two things is
        // caught by whichever check notices first and the check it was written
        // for stays unexercised.
        let mut sound = [0u8; ROOM];
        let len = composed(&mut sound);
        let cases: &[Lie] = &[
            ("a CFF face", |b| b[0..4].copy_from_slice(b"OTTO"), Refusal::NotAFace),
            ("a collection", |b| b[0..4].copy_from_slice(b"ttcf"), Refusal::NotAFace),
            ("no tables", |b| b[4..6].copy_from_slice(&0u16.to_be_bytes()), Refusal::Count),
            (
                "a table past the end",
                |b| b[12 + 3 * 16 + 12..12 + 3 * 16 + 16].copy_from_slice(&4096u32.to_be_bytes()),
                Refusal::Truncated,
            ),
            (
                "two tables out of order",
                |b| {
                    for i in 0..4 {
                        b.swap(12 + i, 12 + 16 + i);
                    }
                },
                Refusal::Directory,
            ),
            ("head's magic", |b| b[HEAD_AT + 12] ^= 1, Refusal::NotAFace),
            ("head's version", |b| b[HEAD_AT + 1] = 2, Refusal::Schema),
            (
                "a grid below the format's",
                |b| b[HEAD_AT + 18..HEAD_AT + 20].copy_from_slice(&15u16.to_be_bytes()),
                Refusal::Quantity,
            ),
            (
                "a grid past the format's",
                |b| b[HEAD_AT + 18..HEAD_AT + 20].copy_from_slice(&16_385u16.to_be_bytes()),
                Refusal::Quantity,
            ),
            ("maxp's version", |b| b[MAXP_AT + 2] = 0x60, Refusal::Schema),
            ("no glyphs", |b| b[MAXP_AT + 4..MAXP_AT + 6].fill(0), Refusal::Count),
            ("hhea's version", |b| b[HHEA_AT + 1] = 2, Refusal::Schema),
            ("a reserved word in hhea", |b| b[HHEA_AT + 25] = 1, Refusal::Reserved),
            ("a metric data format", |b| b[HHEA_AT + 33] = 1, Refusal::Reserved),
            (
                "more long metrics than glyphs",
                |b| b[HHEA_AT + 34..HHEA_AT + 36].copy_from_slice(&5u16.to_be_bytes()),
                Refusal::Count,
            ),
            (
                "a count hmtx cannot carry",
                |b| {
                    b[MAXP_AT + 4..MAXP_AT + 6].copy_from_slice(&6u16.to_be_bytes());
                },
                Refusal::Truncated,
            ),
            ("a table this module reads, renamed", |b| b[12 + 16 * 2] = b'j', Refusal::Missing),
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
        // The load-bearing one that survived the format: a blob with a byte past
        // its last table is a blob two addresses could name one face with.
        let mut buffer = [0u8; ROOM];
        let len = composed(&mut buffer);
        assert!(Face::read(&buffer[..len]).is_ok());
        assert_eq!(Face::read(&buffer[..len + 4]), Err(Refusal::Truncated));
        assert_eq!(Face::read(&buffer[..len - 1]), Err(Refusal::Truncated));
        // And the padding is the format's zero, not a byte anybody may set.
        buffer[len - 1] = 1;
        assert_eq!(Face::read(&buffer[..len]), Err(Refusal::Truncated));
    }

    #[test]
    fn the_writer_refuses_what_its_own_reader_would() {
        let mut buffer = [0u8; ROOM];
        assert_eq!(compose(&mut buffer, 0, &ADVANCES), Err(Refusal::Quantity));
        assert_eq!(compose(&mut buffer, UNITS_PER_EM_MAX + 1, &ADVANCES), Err(Refusal::Quantity));
        assert_eq!(compose(&mut buffer, 1000, &[]), Err(Refusal::Count));
        assert_eq!(compose(&mut buffer[..8], 1000, &ADVANCES), Err(Refusal::Truncated));
    }

    #[test]
    fn one_advance_changed_is_a_different_face() {
        let mut first = [0u8; ROOM];
        let mut second = [0u8; ROOM];
        let len = composed(&mut first);
        let mut other = ADVANCES;
        other[1] += 1;
        compose(&mut second, 1000, &other).expect("also a face");
        assert!(Face::read(&first[..len]).is_ok());
        assert!(Face::read(&second[..len]).is_ok());
        assert_ne!(first, second);
    }

    #[test]
    fn a_glyph_past_the_long_metrics_takes_the_last_one() {
        // `hmtx`'s own rule, which a real face uses and the composed one does
        // not: fewer long metrics than glyphs, and the rest share the last
        // advance.
        let mut buffer = [0u8; ROOM];
        let len = composed(&mut buffer);
        buffer[HHEA_AT + 34..HHEA_AT + 36].copy_from_slice(&2u16.to_be_bytes());
        let face = Face::read(&buffer[..len]).expect("two long metrics and two short ones");
        assert_eq!(face.advance_design_units(0), Some(0));
        assert_eq!(face.advance_design_units(1), Some(512));
        assert_eq!(face.advance_design_units(2), Some(512));
        assert_eq!(face.advance_design_units(3), Some(512));
        assert_eq!(face.advance_design_units(4), None);
    }

    #[test]
    fn the_two_fixture_faces_are_both_faces_and_are_not_the_same_one() {
        let mut declared = [0u8; fixture::BYTES];
        let mut twin = [0u8; fixture::BYTES];
        let one = fixture::declared(&mut declared).expect("the declared fixture composes");
        let two = fixture::undeclared(&mut twin).expect("the undeclared fixture composes");
        assert_eq!(one, two, "the two fixtures are the same length");
        assert_eq!(one, fixture::BYTES);
        assert_ne!(declared, twin, "two faces that compose to one run of bytes are one face");

        let left = Face::read(&declared[..one]).expect("the declared fixture is a face");
        let right = Face::read(&twin[..two]).expect("the undeclared fixture is a face too");
        assert_eq!(left.units_per_em(), right.units_per_em());
        assert_eq!(left.glyphs(), right.glyphs());
        assert_ne!(left, right, "the twin differs where the format can see it");
    }

    #[test]
    fn the_twin_differs_only_where_one_advance_and_the_checksums_say_it_does() {
        // The old format's twin differed in one byte. A real format carries
        // checksums over what changed, so the twin now differs in the advance's
        // low byte and in the three words that sum it: `hmtx`'s record, and
        // `head.checkSumAdjustment`, which covers the whole file.
        let mut declared = [0u8; fixture::BYTES];
        let mut twin = [0u8; fixture::BYTES];
        let len = fixture::declared(&mut declared).expect("composes");
        let _ = fixture::undeclared(&mut twin).expect("composes");
        let differing: [usize; 1] = [HMTX_AT + METRIC_BYTES + 1];
        for at in 0..len {
            let in_advance = differing.contains(&at);
            let in_a_checksum = (12 + 2 * 16 + 4..12 + 2 * 16 + 8).contains(&at)
                || (HEAD_AT + 8..HEAD_AT + 12).contains(&at);
            if !in_advance && !in_a_checksum {
                assert_eq!(
                    declared[at], twin[at],
                    "byte {at} is neither the advance nor a checksum"
                );
            }
        }
        assert_ne!(declared[HMTX_AT + METRIC_BYTES + 1], twin[HMTX_AT + METRIC_BYTES + 1]);
    }
}
