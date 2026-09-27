// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E3-B03b0`'s exit: runs of text through the `shape` protocol, producing
//! fixed-point advances and offsets — and what those must be, argued from the
//! font's own tables rather than read off the shaper's first answer.
//!
//! # The runs, and why these three
//!
//! All in Inter 4.001 Regular (`third_party/inter/`), default features.
//!
//! - **`Tokyo -> Vienna`, Latin, left to right.** Both halves of what RFC 0082
//!   imports: **`GSUB`**, because Inter's `calt` — on by default, and Inter has
//!   no `liga` at all — ligates `-` `>` into one arrow glyph; and **`GPOS`**,
//!   because `T o` and `y o` are kerning pairs in Inter's `kern`. Every other
//!   adjacent pair has a zero adjustment, which is itself asserted.
//! - **The same run, right to left.** The direction is a field of the request
//!   and has to reach the import: the glyphs come back in visual order, the
//!   clusters descending, `>` mirrored to `<` so that `calt` makes the *left*
//!   arrow, and four different kerning pairs — so a shim that set every run left
//!   to right, or reversed nothing, is red on ids, clusters and width at once.
//! - **`h` and U+0301 COMBINING ACUTE ACCENT, Latin and then Cyrillic.** The
//!   only run here whose offsets are not zero, so the only one that can tell
//!   `x_offset` from `y_offset`, or either from nothing. Under `Cyrl` the same
//!   two glyphs come back unpositioned, because Inter's `GPOS` has no `mark`
//!   feature for `cyrl`: the script is a field of the request, and this is the
//!   run that shows it reaching the import rather than being guessed from the
//!   text or pinned to Latin.
//!
//! # The argument, from the font's own tables and not from the shaper
//!
//! Every number below was derived from `Inter-Regular.ttf` before HarfRust
//! shaped the run, by two readers that share no code with HarfRust or
//! `read-fonts`: fontTools 4.58.2 for the first run (2026-09-26), and for all
//! three a reader of the table layouts written from the OpenType 1.9
//! specification, out of this tree, that applies the default `ccmp` and `calt`
//! lookups and the `kern` and `mark` lookups by the specification's rules.
//!
//! - **Glyph ids** are `cmap`'s (subtable 3/1, format 4). The arrows are
//!   `GSUB` ligatures: lookup 47 — type 4, flag 0, under `calt` for `DFLT` and
//!   `latn` — holds `hyphen greater -> arrowright` (1805); right to left, lookup
//!   48 (chaining context, format 3) matches `less hyphen` and calls lookup 49,
//!   whose ligature is `arrowleft` (1800). Both arrows are also `cmap`'s own
//!   glyphs for U+2192 and U+2190, which this file checks. The other default
//!   `GSUB` lookups fire nowhere in these runs: `ccmp` 3, 5, 6, 8, 9 and 11 and
//!   `calt` 51 and 52 were applied to each run by the out-of-tree reader and
//!   changed nothing, and `calt` 47 and 48 fire only where named above. Unicode
//!   has no precomposed `h` with acute, so the import's normaliser composes
//!   nothing either.
//! - **Advances** are `hmtx`'s (`numberOfHMetrics` is 2,937, every glyph) plus
//!   `GPOS` lookup 1 — type 9 extending type 2, under `kern` beside lookups 2 and
//!   3, which adjust no pair in these runs. Left to right: `T` is first-glyph
//!   class 13 and `o` second-glyph class 2, **−160**; `y o` is 11 × 2, **−40**.
//!   Right to left the run is shaped in visual order, so the pairs are
//!   different ones: `o y` 1 × 10 **−40**, `y k` 11 × 1 **+10**, `k o` 26 × 2
//!   **−46**, `o T` 1 × 15 **−160**. A pair adjustment moves the *first* glyph's
//!   advance, in the order the import holds the buffer.
//! - **Offsets** are `GPOS` lookup 8's: `mark` for `latn` lists lookups 4 to 8,
//!   and 8 is the only one whose mark coverage holds `acutecomb`. Its anchor on
//!   `h` for mark class 0 is (248, 1490) and `acutecomb`'s mark anchor is (247,
//!   1118). The mark is drawn so the two anchors meet, from a pen that has
//!   already advanced past `h`: `x_offset` = 248 − 247 − 1211 = **−1210**, and
//!   `y_offset` = 1490 − 1118 = **372**. `acutecomb` advances 0 by `hmtx`.
//! - **Clusters** are byte offsets into the run. A ligature carries the least of
//!   its components' clusters, and a mark joins its base's (the import's default
//!   is grapheme clusters), so the accent's cluster is 0 and not 1.
//!
//! # What this file re-derives at run time, and what it asserts as argued
//!
//! A reader of this file should not have to trust the numbers above, so
//! [`Tables`] re-reads, with this file's own code: every glyph id from `cmap`,
//! every base advance from `hmtx`, the two arrows from `cmap`, `GPOS`'s
//! `mark` lookups for `latn` and `cyrl` from its script and feature lists, and
//! lookup 8's two anchors. The ligature rules and the kerning classes are
//! argued above and asserted as numbers: re-reading chaining contexts and class
//! pairs here would be writing a second shaper to test the first, and RFC 0082
//! imported the shaper so that this tree would not.
//!
//! **`y_advance` is the one field no run here can exercise.** It is zero in
//! horizontal text unless a `GPOS` value record carries a `YAdvance`, and none of
//! Inter's does (read out of tree), and the protocol carries no vertical
//! direction. So a shim that dropped it would pass every run in this file; what
//! holds the copy field by field is `src/lib.rs`'s own test, over a position
//! whose four fields all differ.
//!
//! # What runs where
//!
//! This is a host test of a `no_std` crate, in one process: the request is
//! encoded, [`f_shaper::serve`] decodes it, shapes it and encodes the reply, and
//! this file decodes that. There is no ring, no `Sqe` envelope and no component
//! between the two ends — the component is not spawnable (`UNSPAWNABLE`, RFC
//! 0141). It runs in `cargo xtask test-host`: on x86-64 Linux locally and in
//! CI's `tests (x86-64)` job, and on AArch64 Linux in `tests (AArch64, weak
//! memory)` on `ubuntu-24.04-arm`, which is the only place its AArch64 half
//! runs. Each run writes one `shaped … on <arch>:` line with every glyph id,
//! advance and offset, so that a log from either job can be cited. The
//! expectations are literals, so *identical on both* is each runner agreeing
//! with one written-down table rather than with the other runner, for
//! `f_text::cache`'s reason; HarfRust computes positions in floating point
//! internally, and a position that moved on one runner is red there — RFC
//! 0082's second reversal condition made executable for these runs.
//!
//! `IMPORT_READERS` in `xtask/src/main.rs` names this file: it opens a data
//! import by path, which is the second route RFC 0114 leaves into a data import.

use std::io::Write as _;

use f_abi::shape::{self, Glyph, Refusal, ReplyHead, Request};

/// The face, by path from this crate.
const FACE_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../third_party/inter/Inter-Regular.ttf");

/// The face's address: `third_party/inter/PROVENANCE.md`'s SHA-256, which
/// `cargo xtask lint-licensing` holds to the bytes.
const FACE_SHA256: &str = "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82";

/// The run.
const RUN: &str = "Tokyo -> Vienna";

/// Inter's grid. `head.unitsPerEm`.
/// Unit: design units per em.
const UNITS_PER_EM: u32 = 2048;

/// One expected glyph: the scalar at its cluster, its id, its cluster, its
/// `hmtx` advance, and the kerning `GPOS` adds to it.
struct Expected {
    scalar: char,
    glyph: u32,
    cluster: u32,
    /// Unit: design units.
    hmtx_design_units: i32,
    /// Unit: design units.
    kern_design_units: i32,
}

/// Shorthand for one row of an argued table.
const fn row(scalar: char, glyph: u32, cluster: u32, hmtx: i32, kern: i32) -> Expected {
    Expected { scalar, glyph, cluster, hmtx_design_units: hmtx, kern_design_units: kern }
}

/// The argued answer, left to right. See the module comment for every number.
const EXPECTED: [Expected; 14] = [
    row('T', 411, 0, 1322, -160),
    row('o', 790, 1, 1228, 0),
    row('k', 727, 2, 1124, 0),
    row('y', 998, 3, 1151, -40),
    row('o', 790, 4, 1228, 0),
    row(' ', 1777, 5, 576, 0),
    // `-` and `>` ligated by `calt` lookup 47: the scalar column names the first
    // component, and the id is `cmap`'s for U+2192, not for `-`.
    row('-', 1805, 6, 1954, 0),
    row(' ', 1777, 8, 576, 0),
    row('V', 456, 9, 1413, 0),
    row('i', 689, 10, 496, 0),
    row('e', 614, 11, 1194, 0),
    row('n', 773, 12, 1210, 0),
    row('n', 773, 13, 1210, 0),
    row('a', 507, 14, 1150, 0),
];

/// The ligature's index in [`EXPECTED`], and the scalar whose `cmap` glyph it is.
const LIGATURE: (usize, char) = (6, '\u{2192}');

/// The run's width, left to right: the sum of `hmtx` less 200.
/// Unit: design units.
const WIDTH_DESIGN_UNITS: i32 = 15_632;

/// The argued answer, right to left: visual order, clusters descending, `>`
/// mirrored and ligated with `-` into the left arrow, and the four pairs the
/// module comment names.
const EXPECTED_RTL: [Expected; 14] = [
    row('a', 507, 14, 1150, 0),
    row('n', 773, 13, 1210, 0),
    row('n', 773, 12, 1210, 0),
    row('e', 614, 11, 1194, 0),
    row('i', 689, 10, 496, 0),
    row('V', 456, 9, 1413, 0),
    row(' ', 1777, 8, 576, 0),
    // `<` (the mirrored `>`, cluster 7) and `-` (cluster 6), ligated by `calt`
    // lookup 48 calling 49; the least cluster is `-`'s.
    row('-', 1800, 6, 1954, 0),
    row(' ', 1777, 5, 576, 0),
    row('o', 790, 4, 1228, -40),
    row('y', 998, 3, 1151, 10),
    row('k', 727, 2, 1124, -46),
    row('o', 790, 1, 1228, -160),
    row('T', 411, 0, 1322, 0),
];

/// The ligature's index in [`EXPECTED_RTL`], and the scalar whose glyph it is.
const LIGATURE_RTL: (usize, char) = (7, '\u{2190}');

/// The run's width, right to left: the same `hmtx` sum less 236.
/// Unit: design units.
const WIDTH_RTL_DESIGN_UNITS: i32 = 15_596;

/// The mark run: `h` and U+0301 COMBINING ACUTE ACCENT.
const MARK_RUN: &str = "h\u{301}";

/// `h` and `acutecomb`, as `cmap` numbers them.
const H: u32 = 670;
const ACUTE: u32 = 1770;

/// `h`'s `hmtx` advance.
/// Unit: design units.
const H_ADVANCE_DESIGN_UNITS: i32 = 1211;

/// The `GPOS` lookup that places `acutecomb` on `h`, and the lookups `mark`
/// lists for `latn`.
const MARK_LOOKUP: usize = 8;
const MARK_LOOKUPS_LATN: [usize; 5] = [4, 5, 6, 7, 8];

/// Lookup 8's base anchor on `h` for the accent's mark class, and the accent's
/// mark anchor.
/// Unit: design units, (x, y).
const H_ANCHOR: (i32, i32) = (248, 1490);
const ACUTE_ANCHOR: (i32, i32) = (247, 1118);

/// Where the accent is drawn relative to its own pen: the anchors meeting, less
/// the advance the pen has already made past `h`.
/// Unit: design units, (x, y).
const ACUTE_OFFSET: (i32, i32) =
    (H_ANCHOR.0 - ACUTE_ANCHOR.0 - H_ADVANCE_DESIGN_UNITS, H_ANCHOR.1 - ACUTE_ANCHOR.1);

// The arithmetic, spelled out once so a reader can check it against the comment.
const _: () = assert!(ACUTE_OFFSET.0 == -1210 && ACUTE_OFFSET.1 == 372);

fn face() -> Vec<u8> {
    std::fs::read(FACE_PATH).unwrap_or_else(|e| {
        panic!(
            "{FACE_PATH}: {e}\n\nThe face is a data import (third_party/inter/, RFC 0114). A \
             checkout without it cannot run this test; restore it from git rather than from \
             upstream, because PROVENANCE.md's SHA-256 is what it is held to."
        )
    })
}

fn address(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex");
    }
    out
}

/// The protocol, both ends, for a shaper that may load `faces`: encode a
/// request, serve it, decode the reply.
fn serve_declaring(
    faces: &[[u8; 32]],
    request: &Request,
    face: &[u8],
) -> Result<(ReplyHead, Vec<Glyph>), Refusal> {
    let wire = request.to_bytes();
    let mut reply = vec![0u8; shape::REPLY_BYTES_MAX];
    let len = f_shaper::serve(faces, &wire, face, &mut reply)?;
    let head = ReplyHead::from_bytes(reply[..ReplyHead::BYTES].try_into().expect("a head"))?;
    assert_eq!(
        len,
        ReplyHead::BYTES + head.glyphs as usize * Glyph::BYTES,
        "the reply's length is its head's"
    );
    let glyphs = (0..head.glyphs as usize)
        .map(|index| {
            let at = ReplyHead::BYTES + index * Glyph::BYTES;
            Glyph::from_bytes(reply[at..at + Glyph::BYTES].try_into().expect("a glyph"))
        })
        .collect();
    Ok((head, glyphs))
}

/// [`serve_declaring`] for the component's own declaration, [`f_shaper::FACES`].
fn through_the_protocol(
    request: &Request,
    face: &[u8],
) -> Result<(ReplyHead, Vec<Glyph>), Refusal> {
    serve_declaring(&f_shaper::FACES, request, face)
}

/// One line a CI log can be cited by: every glyph's id, cluster, advance and
/// offset. Written to the process's standard output directly rather than with
/// `println!`, because the test harness captures `println!` from a passing test
/// and `cargo xtask test-host` runs the harness without `--nocapture`; a direct
/// write is not captured, so the line reaches both CI jobs' logs.
fn cite(what: &str, head: &ReplyHead, glyphs: &[Glyph]) {
    let each: Vec<String> = glyphs
        .iter()
        .map(|g| {
            format!(
                "{}@{} {:+}/{:+} ({:+},{:+})",
                g.glyph,
                g.cluster,
                g.x_advance_design_units,
                g.y_advance_design_units,
                g.x_offset_design_units,
                g.y_offset_design_units
            )
        })
        .collect();
    let width: i32 = glyphs.iter().map(|g| g.x_advance_design_units).sum();
    let line = format!(
        "shaped {what} on {}: {} glyphs, {width} design units at {} to the em — \
         id@cluster x/y advance (x,y offset): {}\n",
        std::env::consts::ARCH,
        glyphs.len(),
        head.units_per_em,
        each.join(", ")
    );
    let _ = std::io::stdout().lock().write_all(line.as_bytes());
}

/// The tables the argued numbers could have been copied from, read by this
/// file.
///
/// Not `read-fonts` and not HarfRust: a table directory, `cmap` format 4,
/// `hmtx`, and the parts of `GPOS` that say which lookups `mark` applies and
/// where lookup 8 puts an accent, from the OpenType specification's own
/// layouts, big-endian.
struct Tables<'a> {
    bytes: &'a [u8],
    cmap4: &'a [u8],
    hmtx: &'a [u8],
    gpos: &'a [u8],
    metrics: usize,
}

impl<'a> Tables<'a> {
    fn u16(bytes: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([bytes[at], bytes[at + 1]])
    }

    fn i16(bytes: &[u8], at: usize) -> i32 {
        i32::from(i16::from_be_bytes([bytes[at], bytes[at + 1]]))
    }

    fn at16(bytes: &[u8], at: usize) -> usize {
        usize::from(Self::u16(bytes, at))
    }

    fn u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn table(bytes: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        let count = Self::at16(bytes, 4);
        (0..count)
            .map(|index| 12 + index * 16)
            .find(|at| &bytes[*at..*at + 4] == tag)
            .map(|at| {
                let offset = Self::u32(bytes, at + 8) as usize;
                let length = Self::u32(bytes, at + 12) as usize;
                &bytes[offset..offset + length]
            })
            .unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)))
    }

    fn open(bytes: &'a [u8]) -> Self {
        let cmap = Self::table(bytes, b"cmap");
        let records = Self::at16(cmap, 2);
        let cmap4 = (0..records)
            .map(|index| 4 + index * 8)
            .find(|at| Self::u16(cmap, *at) == 3 && Self::u16(cmap, at + 2) == 1)
            .map(|at| &cmap[Self::u32(cmap, at + 4) as usize..])
            .expect("a Windows Unicode BMP subtable");
        assert_eq!(Self::u16(cmap4, 0), 4, "subtable 3/1 is format 4");
        let metrics = Self::at16(Self::table(bytes, b"hhea"), 34);
        let (hmtx, gpos) = (Self::table(bytes, b"hmtx"), Self::table(bytes, b"GPOS"));
        Self { bytes, cmap4, hmtx, gpos, metrics }
    }

    /// `cmap` format 4: segments of end codes, start codes, deltas and range
    /// offsets, searched linearly because there are only a few hundred.
    fn glyph(&self, scalar: char) -> u32 {
        let c = scalar as u32;
        let t = self.cmap4;
        let segs = Self::at16(t, 6) / 2;
        let ends = 14;
        let starts = ends + segs * 2 + 2;
        let deltas = starts + segs * 2;
        let offsets = deltas + segs * 2;
        for seg in 0..segs {
            let end = u32::from(Self::u16(t, ends + seg * 2));
            if c > end {
                continue;
            }
            let start = u32::from(Self::u16(t, starts + seg * 2));
            if c < start {
                return 0;
            }
            let delta = Self::u16(t, deltas + seg * 2);
            let range = Self::at16(t, offsets + seg * 2);
            let id = if range == 0 {
                (c as u16).wrapping_add(delta)
            } else {
                let at = offsets + seg * 2 + range + (c - start) as usize * 2;
                match Self::u16(t, at) {
                    0 => 0,
                    g => g.wrapping_add(delta),
                }
            };
            return u32::from(id);
        }
        0
    }

    /// `hmtx`: one advance per long metric, the last repeated after them.
    fn advance(&self, glyph: u32) -> i32 {
        let index = (glyph as usize).min(self.metrics - 1);
        i32::from(Self::u16(self.hmtx, index * 4))
    }

    fn units_per_em(&self) -> u32 {
        u32::from(Self::u16(Self::table(self.bytes, b"head"), 18))
    }

    /// The lookups `GPOS` applies for `feature` under `script`'s default
    /// language system: `None` when the script is not in the `ScriptList`,
    /// empty when it is and does not list the feature.
    fn gpos_lookups(&self, script: &[u8; 4], feature: &[u8; 4]) -> Option<Vec<usize>> {
        let t = self.gpos;
        let (scripts, features) = (Self::at16(t, 4), Self::at16(t, 6));
        let record = (0..Self::at16(t, scripts))
            .map(|index| scripts + 2 + index * 6)
            .find(|at| &t[*at..*at + 4] == script)?;
        let script_at = scripts + Self::at16(t, record + 4);
        let default = Self::at16(t, script_at);
        assert_ne!(default, 0, "a default language system");
        let langsys = script_at + default;
        let mut out = Vec::new();
        for index in 0..Self::at16(t, langsys + 4) {
            let feature_index = Self::at16(t, langsys + 6 + index * 2);
            let at = features + 2 + feature_index * 6;
            if &t[at..at + 4] == feature {
                let table = features + Self::at16(t, at + 4);
                out.extend((0..Self::at16(t, table + 2)).map(|k| Self::at16(t, table + 4 + k * 2)));
            }
        }
        Some(out)
    }

    /// A coverage table's index for `glyph`, formats 1 and 2.
    fn coverage(t: &[u8], at: usize, glyph: u32) -> Option<usize> {
        let count = Self::at16(t, at + 2);
        match Self::u16(t, at) {
            1 => (0..count).find(|index| u32::from(Self::u16(t, at + 4 + index * 2)) == glyph),
            2 => (0..count).find_map(|index| {
                let range = at + 4 + index * 6;
                let (start, end) =
                    (u32::from(Self::u16(t, range)), u32::from(Self::u16(t, range + 2)));
                (start..=end)
                    .contains(&glyph)
                    .then(|| Self::at16(t, range + 4) + (glyph - start) as usize)
            }),
            format => panic!("coverage format {format}"),
        }
    }

    /// Mark-to-base lookup `lookup` (type 4): the base anchor on `base` for
    /// `mark`'s class, and `mark`'s own anchor — `None` when no subtable covers
    /// both glyphs.
    fn mark_to_base(&self, lookup: usize, base: u32, mark: u32) -> Option<[(i32, i32); 2]> {
        let t = self.gpos;
        let list = Self::at16(t, 8);
        let at = list + Self::at16(t, list + 2 + lookup * 2);
        assert_eq!(Self::u16(t, at), 4, "lookup {lookup} is mark-to-base");
        let anchor = |at: usize| (Self::i16(t, at + 2), Self::i16(t, at + 4));
        (0..Self::at16(t, at + 4)).find_map(|index| {
            let sub = at + Self::at16(t, at + 6 + index * 2);
            assert_eq!(Self::u16(t, sub), 1, "MarkBasePos format 1");
            let m = Self::coverage(t, sub + Self::at16(t, sub + 2), mark)?;
            let b = Self::coverage(t, sub + Self::at16(t, sub + 4), base)?;
            let classes = Self::at16(t, sub + 6);
            let (marks, bases) = (sub + Self::at16(t, sub + 8), sub + Self::at16(t, sub + 10));
            let class = Self::at16(t, marks + 2 + m * 4);
            let mark_anchor = anchor(marks + Self::at16(t, marks + 2 + m * 4 + 2));
            let base_anchor = anchor(bases + Self::at16(t, bases + 2 + (b * classes + class) * 2));
            Some([base_anchor, mark_anchor])
        })
    }
}

/// Every row of an argued table checked against `cmap` and `hmtx`, the
/// ligature's id against `cmap`'s arrow, and the width against the sum.
fn check_against_the_font(
    tables: &Tables<'_>,
    run: &str,
    expected: &[Expected],
    ligature: (usize, char),
    width: i32,
) {
    for (index, want) in expected.iter().enumerate() {
        let scalar = run
            .char_indices()
            .find(|(at, _)| *at as u32 == want.cluster)
            .map(|(_, scalar)| scalar)
            .expect("every cluster is a scalar boundary");
        assert_eq!(scalar, want.scalar, "glyph {index}'s cluster is its scalar");
        let from = if index == ligature.0 { ligature.1 } else { scalar };
        assert_eq!(tables.glyph(from), want.glyph, "cmap maps {from:?} to glyph {}", want.glyph);
        assert_eq!(
            tables.advance(want.glyph),
            want.hmtx_design_units,
            "hmtx for glyph {}",
            want.glyph
        );
    }
    // And the arrow is what two glyphs became: `-`, `>` and `<` each map to a
    // glyph of their own, none of which is the arrow.
    for scalar in ['-', '>', '<'] {
        assert_ne!(tables.glyph(scalar), expected[ligature.0].glyph, "{scalar:?}");
    }
    let sum: i32 = expected.iter().map(|e| e.hmtx_design_units + e.kern_design_units).sum();
    assert_eq!(sum, width);
}

/// A shaped run against an argued table: ids, clusters and advances, and no
/// offset and no vertical advance anywhere — there is no mark in these runs.
fn check_run(glyphs: &[Glyph], expected: &[Expected], width: i32) {
    assert_eq!(glyphs.len(), expected.len(), "fifteen scalars, one ligature: fourteen glyphs");
    for (index, (got, want)) in glyphs.iter().zip(expected).enumerate() {
        let advance = want.hmtx_design_units + want.kern_design_units;
        assert_eq!(
            (got.glyph, got.cluster, got.x_advance_design_units),
            (want.glyph, want.cluster, advance),
            "glyph {index} ({:?}): (id, cluster, advance)",
            want.scalar
        );
        assert_eq!(
            (got.y_advance_design_units, got.x_offset_design_units, got.y_offset_design_units),
            (0, 0, 0),
            "glyph {index}: horizontal Latin with no marks moves nothing else"
        );
    }
    let sum: i32 = glyphs.iter().map(|g| g.x_advance_design_units).sum();
    assert_eq!(sum, width);
}

#[test]
fn the_argued_tables_are_the_fonts_own() {
    // The half of each table a reader might suspect was copied from an answer,
    // checked against the font by a reader that is not the shaper's.
    let bytes = face();
    let tables = Tables::open(&bytes);
    assert_eq!(tables.units_per_em(), UNITS_PER_EM);
    check_against_the_font(&tables, RUN, &EXPECTED, LIGATURE, WIDTH_DESIGN_UNITS);
    check_against_the_font(&tables, RUN, &EXPECTED_RTL, LIGATURE_RTL, WIDTH_RTL_DESIGN_UNITS);
    // Right to left is the same glyphs and clusters, reversed, but for the
    // arrow; only the kerning differs, pair by pair.
    for (ltr, rtl) in EXPECTED.iter().rev().zip(&EXPECTED_RTL) {
        assert_eq!((ltr.cluster, ltr.hmtx_design_units), (rtl.cluster, rtl.hmtx_design_units));
        if ltr.cluster == EXPECTED[LIGATURE.0].cluster {
            assert_ne!(ltr.glyph, rtl.glyph, "the arrow turns round");
        } else {
            assert_eq!(ltr.glyph, rtl.glyph, "cluster {}", ltr.cluster);
        }
    }
}

#[test]
fn the_mark_run_is_argued_from_gpos() {
    let bytes = face();
    let tables = Tables::open(&bytes);
    assert_eq!(tables.glyph('h'), H);
    assert_eq!(tables.glyph('\u{301}'), ACUTE);
    assert_eq!(tables.advance(H), H_ADVANCE_DESIGN_UNITS);
    assert_eq!(tables.advance(ACUTE), 0, "the accent advances nothing");
    // Which lookups place marks for which script — the difference the Cyrillic
    // run below turns on.
    assert_eq!(tables.gpos_lookups(b"latn", b"mark"), Some(MARK_LOOKUPS_LATN.to_vec()));
    assert_eq!(tables.gpos_lookups(b"cyrl", b"mark"), Some(Vec::new()), "cyrl lists no mark");
    assert_eq!(tables.gpos_lookups(b"cyrl", b"mkmk"), Some(Vec::new()), "nor mkmk");
    assert!(!tables.gpos_lookups(b"cyrl", b"kern").unwrap_or_default().is_empty(), "only kern");
    // Of those, exactly one places this accent on this base, at these anchors.
    for lookup in MARK_LOOKUPS_LATN {
        let found = tables.mark_to_base(lookup, H, ACUTE);
        if lookup == MARK_LOOKUP {
            assert_eq!(found, Some([H_ANCHOR, ACUTE_ANCHOR]), "lookup {lookup}");
        } else {
            assert_eq!(found, None, "lookup {lookup} does not place acutecomb on h");
        }
    }
}

#[test]
fn one_run_through_the_shape_protocol_produces_the_argued_advances() {
    let bytes = face();
    let request = Request::new(address(FACE_SHA256), *b"Latn", RUN).expect("a request");
    let (head, glyphs) = through_the_protocol(&request, &bytes).expect("the shaper answers");
    assert_eq!(head.units_per_em, UNITS_PER_EM);
    check_run(&glyphs, &EXPECTED, WIDTH_DESIGN_UNITS);
    cite(&format!("{RUN:?} Latn left to right"), &head, &glyphs);
}

#[test]
fn right_to_left_reaches_the_import() {
    let bytes = face();
    let mut request = Request::new(address(FACE_SHA256), *b"Latn", RUN).expect("a request");
    request.direction = shape::direction::RIGHT_TO_LEFT;
    let (head, glyphs) = through_the_protocol(&request, &bytes).expect("the shaper answers");
    assert_eq!(head.units_per_em, UNITS_PER_EM);
    check_run(&glyphs, &EXPECTED_RTL, WIDTH_RTL_DESIGN_UNITS);
    assert!(glyphs.windows(2).all(|w| w[0].cluster > w[1].cluster), "clusters descend");
    cite(&format!("{RUN:?} Latn right to left"), &head, &glyphs);
}

#[test]
fn a_mark_is_placed_by_its_anchors_and_the_script_decides_whether() {
    let bytes = face();
    let base = Glyph {
        glyph: H,
        cluster: 0,
        x_advance_design_units: H_ADVANCE_DESIGN_UNITS,
        y_advance_design_units: 0,
        x_offset_design_units: 0,
        y_offset_design_units: 0,
    };
    let mark = |(x, y): (i32, i32)| Glyph {
        glyph: ACUTE,
        cluster: 0,
        x_advance_design_units: 0,
        y_advance_design_units: 0,
        x_offset_design_units: x,
        y_offset_design_units: y,
    };

    let request = Request::new(address(FACE_SHA256), *b"Latn", MARK_RUN).expect("a request");
    let (head, glyphs) = through_the_protocol(&request, &bytes).expect("the shaper answers");
    assert_eq!(glyphs, [base, mark(ACUTE_OFFSET)], "Latn: lookup 8's anchors meet");
    cite("\"h\\u{301}\" Latn left to right", &head, &glyphs);

    // The control: `cyrl` lists no `mark` lookup, so the same two glyphs come
    // back where the pen left them. A shim that pinned the script to Latin, or
    // left it for the import to guess from the text, answers the line above.
    let request = Request::new(address(FACE_SHA256), *b"Cyrl", MARK_RUN).expect("a request");
    let (head, glyphs) = through_the_protocol(&request, &bytes).expect("the shaper answers");
    assert_eq!(glyphs, [base, mark((0, 0))], "Cyrl: no mark feature, no offset");
    cite("\"h\\u{301}\" Cyrl left to right", &head, &glyphs);
}

#[test]
fn the_defaults_are_what_did_it() {
    // The control: the same run with `kern` and `calt` off. If the shaper
    // answered the argued table whatever it was asked, the table would prove
    // nothing about `GSUB` or `GPOS`.
    let bytes = face();
    let mut request = Request::new(address(FACE_SHA256), *b"Latn", RUN).expect("a request");
    request.features[0] = shape::Feature { tag: *b"kern", value: 0 };
    request.features[1] = shape::Feature { tag: *b"calt", value: 0 };
    request.feature_count = 2;
    let (_, glyphs) = through_the_protocol(&request, &bytes).expect("the shaper answers");
    assert_eq!(glyphs.len(), RUN.len(), "no ligature: one glyph per scalar");
    let tables = Tables::open(&bytes);
    for (glyph, scalar) in glyphs.iter().zip(RUN.chars()) {
        assert_eq!(glyph.glyph, tables.glyph(scalar));
        assert_eq!(
            glyph.x_advance_design_units,
            tables.advance(glyph.glyph),
            "no kerning on {scalar:?}"
        );
    }
}

#[test]
fn a_script_spelled_right_that_the_import_does_not_know_is_answered() {
    // `abi/src/shape.rs` says so: `Qaaa` is private use, the import has no
    // shaper for it, and it is shaped with the default one — against Inter's
    // `DFLT` tables, which list what `latn` lists, so the answer is the argued
    // one. A spelling `is_script` refuses never reaches the shaper.
    let bytes = face();
    let request = Request::new(address(FACE_SHA256), *b"Qaaa", RUN).expect("a request");
    let (_, glyphs) = through_the_protocol(&request, &bytes).expect("answered, not refused");
    check_run(&glyphs, &EXPECTED, WIDTH_DESIGN_UNITS);
    let mut wire = Request::new(address(FACE_SHA256), *b"Latn", RUN).expect("a request").to_bytes();
    wire[32..36].copy_from_slice(b"abcd");
    let mut reply = vec![0u8; shape::REPLY_BYTES_MAX];
    assert_eq!(
        f_shaper::serve(&f_shaper::FACES, &wire, &bytes, &mut reply),
        Err(Refusal::Malformed)
    );
}

#[test]
fn the_faces_the_shim_loads_are_the_ones_its_manifest_declares() {
    // `f_shaper::FACES` and `manifest.toml`'s `[[face]]` tables are one list
    // written twice; this is what holds them equal.
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/manifest.toml"))
        .expect("the shaper's manifest");
    let mut declared = Vec::new();
    let mut in_face = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_face = line == "[[face]]";
        } else if let Some(hex) = line.strip_prefix("hash = \"sha256:") {
            assert!(in_face, "a hash outside a [[face]] table: {line}");
            declared.push(address(hex.trim_end_matches('"')));
        }
    }
    assert_eq!(declared, f_shaper::FACES.to_vec());
    assert_eq!(f_shaper::FACES, [address(FACE_SHA256)], "Inter, and nothing else");
}

#[test]
fn a_face_is_believed_by_its_address_and_not_by_whoever_handed_it_over() {
    let bytes = face();
    let mut wrong = address(FACE_SHA256);
    wrong[0] ^= 1;
    let request = Request::new(wrong, *b"Latn", RUN).expect("a request");
    assert_eq!(through_the_protocol(&request, &bytes).err(), Some(Refusal::FaceNotHeld));

    // The declared name over bytes that are not the declared face.
    let request = Request::new(address(FACE_SHA256), *b"Latn", RUN).expect("a request");
    assert_eq!(through_the_protocol(&request, b"not Inter").err(), Some(Refusal::FaceNotHeld));

    // Bytes named by their own hash, which nobody declared: refused before
    // anything reads them, whatever they are.
    let junk = b"not a face at all";
    let request = Request::new(f_hash::sha256(junk), *b"Latn", RUN).expect("a request");
    assert_eq!(through_the_protocol(&request, junk).err(), Some(Refusal::FaceNotHeld));
    // The same bytes, declared: now they reach the admission check, which
    // refuses them.
    assert_eq!(
        serve_declaring(&[f_hash::sha256(junk)], &request, junk).err(),
        Some(Refusal::NotAFace)
    );
}

#[test]
fn a_face_the_import_would_read_and_this_tree_does_not_admit_is_refused() {
    // Inter with one of `hhea`'s reserved words set, and re-hashed. HarfRust
    // reads neither that word nor `hhea` to open a face, so the import would
    // shape it — with `f_text::face::Face::read` taken out of `src/lib.rs`
    // this run answers the argued fourteen glyphs; `Face::read` refuses it,
    // R04, and so does the shim.
    let mut bytes = face();
    let hhea = (0..Tables::at16(&bytes, 4))
        .map(|index| 12 + index * 16)
        .find(|at| &bytes[*at..*at + 4] == b"hhea")
        .map(|at| Tables::u32(&bytes, at + 8) as usize)
        .expect("an hhea table");
    // `reserved1`'s low byte: the first of the four words `Face::read` requires
    // to be zero.
    bytes[hhea + 25] = 1;
    assert_eq!(f_text::face::Face::read(&bytes).err(), Some(f_text::face::Refusal::Reserved));
    let name = f_hash::sha256(&bytes);
    let request = Request::new(name, *b"Latn", RUN).expect("a request");
    // Undeclared, it never reaches either reader.
    assert_eq!(through_the_protocol(&request, &bytes).err(), Some(Refusal::FaceNotHeld));
    // Declared, it reaches `Face::read`, which is the check that refuses it.
    assert_eq!(serve_declaring(&[name], &request, &bytes).err(), Some(Refusal::NotAFace));
}

#[test]
fn the_auditors_hostile_faces_never_reach_the_import() {
    // The out-of-tree fuzz run that found the import asking the heap for 14 to
    // 22 megabytes (RFC 0141): 1,500 copies of Inter, each with one to eight
    // bytes changed inside one of ten tables and named by its own hash. The
    // generator is replayed here from its seed, and the faces it logged are
    // served: the seven that made `ShaperData::new` size one vector past the
    // manifest's whole heap from a `GSUB` or `GPOS` count, and the sixteen that
    // shaped with a heap peak past 131,072 bytes. Every one is refused as
    // undeclared, before it is hashed, parsed or handed over.
    const HEAP_EXHAUSTING: [(usize, &[u8; 4]); 7] = [
        (52, b"GSUB"),
        (174, b"GSUB"),
        (496, b"GSUB"),
        (802, b"GSUB"),
        (983, b"GPOS"),
        (1166, b"GSUB"),
        (1255, b"GSUB"),
    ];
    const OVER_BUDGET: [(usize, &[u8; 4]); 16] = [
        (72, b"GSUB"),
        (262, b"GSUB"),
        (444, b"GSUB"),
        (447, b"GSUB"),
        (558, b"GSUB"),
        (562, b"GSUB"),
        (678, b"GSUB"),
        (697, b"GPOS"),
        (700, b"GSUB"),
        (796, b"GSUB"),
        (871, b"GSUB"),
        (948, b"GSUB"),
        (1073, b"GSUB"),
        (1130, b"GPOS"),
        (1284, b"GPOS"),
        (1419, b"GSUB"),
    ];
    enum Change {
        To(u8),
        Flip(u64),
    }
    let inter = face();
    let names: [&[u8; 4]; 10] =
        [b"GSUB", b"GPOS", b"GDEF", b"cmap", b"hmtx", b"maxp", b"hhea", b"head", b"OS/2", b"post"];
    let targets: Vec<(&[u8], usize, usize)> = (0..Tables::at16(&inter, 4))
        .map(|index| 12 + index * 16)
        .filter(|at| names.iter().any(|name| &inter[*at..*at + 4] == *name))
        .map(|at| {
            let (offset, length) = (Tables::u32(&inter, at + 8), Tables::u32(&inter, at + 12));
            (&inter[at..at + 4], offset as usize, length as usize)
        })
        .collect();
    assert_eq!(targets.len(), names.len(), "Inter carries all ten");
    // xorshift64 from the auditor's seed: a fixed sequence, not a draw.
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut served = 0;
    for iteration in 0..1_500 {
        let (tag, offset, length) = targets[(next() as usize) % targets.len()];
        let mut changes = Vec::new();
        for _ in 0..1 + (next() % 8) {
            let span = if next() % 2 == 0 { length.min(2048) } else { length };
            let at = offset + (next() as usize) % span.max(1);
            changes.push(match next() % 4 {
                0 => (at, Change::To(0xff)),
                1 => (at, Change::To(0)),
                2 => (at, Change::Flip(next() % 8)),
                _ => (at, Change::To(next() as u8)),
            });
        }
        let Some((_, logged)) =
            HEAP_EXHAUSTING.iter().chain(&OVER_BUDGET).find(|(logged, _)| *logged == iteration)
        else {
            continue;
        };
        assert_eq!(tag, *logged, "face {iteration}: the replay changes the table that was logged");
        let mut hostile = inter.clone();
        for (at, change) in changes {
            hostile[at] = match change {
                Change::To(byte) => byte,
                Change::Flip(bit) => hostile[at] ^ (1 << bit),
            };
        }
        assert_ne!(hostile, inter, "face {iteration} is not Inter");
        let request = Request::new(f_hash::sha256(&hostile), *b"Latn", RUN).expect("a request");
        assert_eq!(
            through_the_protocol(&request, &hostile).err(),
            Some(Refusal::FaceNotHeld),
            "face {iteration}"
        );
        served += 1;
    }
    assert_eq!(served, HEAP_EXHAUSTING.len() + OVER_BUDGET.len());
}

#[test]
fn a_reply_that_does_not_fit_is_refused_rather_than_cut() {
    let bytes = face();
    let request = Request::new(address(FACE_SHA256), *b"Latn", RUN).expect("a request");
    let mut small = vec![0u8; ReplyHead::BYTES + (EXPECTED.len() - 1) * Glyph::BYTES];
    assert_eq!(
        f_shaper::serve(&f_shaper::FACES, &request.to_bytes(), &bytes, &mut small),
        Err(Refusal::TooManyGlyphs)
    );
}

#[test]
fn a_panic_in_the_image_is_a_fault_and_not_a_spin() {
    // `src/component.rs` is compiled for the image only, so no host test can
    // run its panic handler; this holds its text to what its module comment
    // argues — `ud2` through `core::intrinsics::abort`, never a loop — and the
    // entry to the door's `EXIT` before it.
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/component.rs"))
        .expect("the entry's source");
    let handler = source.split("#[panic_handler]").nth(1).expect("a panic handler");
    let body = &handler[..handler.find("\n}").expect("its body ends")];
    assert!(body.contains("core::intrinsics::abort()"), "the handler faults:\n{body}");
    assert!(!body.contains("loop"), "the handler does not spin:\n{body}");
    let start = source.split("pub fn start").nth(1).expect("the entry");
    let start = &start[..start.find("\n}").expect("its body ends")];
    assert!(start.contains("door::call(door::EXIT"), "the entry exits:\n{start}");
    assert!(!start.contains("loop"), "the entry does not spin:\n{start}");
}
