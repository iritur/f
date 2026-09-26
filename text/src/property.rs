// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Character properties, read out of the tables generated from Unicode's data.
//!
//! The eight generated files beside this one — [`crate::bidi_class`],
//! [`crate::bidi_brackets`], [`crate::line_break`], [`crate::grapheme_break`],
//! [`crate::east_asian_width`], [`crate::general_category`],
//! [`crate::indic_conjunct_break`] and [`crate::extended_pictographic`] — are
//! data under two licences, this tree's and Unicode's, because they are
//! Unicode's data re-spelled as Rust (RFC 0114). This module is the code that
//! reads them, and it is here rather than in them for that reason: a lookup
//! written into a generated file would be code under the data's terms too, and
//! the count of files in this tree that are not purely liftable would grow by
//! whatever the generator happened to emit. So the generated files hold values
//! and nothing runs in them, and every decision about *how* a value is found is
//! in this file, under this tree's terms, where a reviewer reads it as code.
//!
//! # What is checked at compile time
//!
//! The shape the lookups rest on: every table of runs starts at zero, ascends
//! strictly and stays inside the code space, and no two adjacent runs carry one
//! value — so a binary search over starts finds the one run a code point is in.
//! The bracket pairs ascend strictly, so a binary search finds one. A
//! regenerated table that broke either would not compile, which is earlier than
//! any test.
//!
//! # What is not here
//!
//! The algorithms. `E3-B03e` wrote UAX #9 against the first two lookups and
//! `E3-B03f` wrote UAX #14 and UAX #29 against the rest; this file answers
//! *what is this character* and nothing about what a paragraph, a line or a
//! cluster does with it. UAX #9's rule N0 also treats canonically equivalent
//! brackets as a pair — U+2329 and U+232A with U+3008 and U+3009 — and that is
//! the algorithm's rule, not a value [`paired_bracket`] invents. UAX #14's LB1
//! resolves `AI`, `SA`, `SG`, `XX` and `CJ` to other classes, and that is the
//! breaker's rule too: [`line_break`] answers with the file's value.

use crate::bidi_brackets::{BIDI_BRACKETS, BracketType};
use crate::bidi_class::{BIDI_CLASS, BidiClass};
use crate::east_asian_width::{EAST_ASIAN_WIDTH, EastAsianWidth};
use crate::extended_pictographic::EXTENDED_PICTOGRAPHIC;
use crate::general_category::{GENERAL_CATEGORY, GeneralCategory};
use crate::grapheme_break::{GRAPHEME_CLUSTER_BREAK, GraphemeClusterBreak};
use crate::indic_conjunct_break::{INDIC_CONJUNCT_BREAK, IndicConjunctBreak};
use crate::line_break::{LINE_BREAK, LineBreak};

/// One past the last code point. Unit: code points.
const CODE_SPACE: u32 = 0x11_0000;

/// The shape every table of runs must have, asserted where it is written so a
/// table that lost it fails to compile on the line naming it. A macro and not
/// a `const fn` because the values are eight different types and a `const fn`
/// cannot be generic over a comparison; `as u8` is the comparison, since every
/// value is a fieldless enum or a `bool`.
macro_rules! runs_are_well_formed {
    ($table:expr) => {
        const _: () = {
            let table = $table;
            assert!(!table.is_empty() && table[0].0 == 0, "the first run starts at zero");
            let mut i = 1;
            while i < table.len() {
                assert!(table[i - 1].0 < table[i].0, "runs ascend strictly");
                assert!(table[i - 1].1 as u8 != table[i].1 as u8, "runs are maximal");
                i += 1;
            }
            assert!(table[table.len() - 1].0 < CODE_SPACE, "runs stay in the code space");
        };
    };
}

runs_are_well_formed!(BIDI_CLASS);
runs_are_well_formed!(LINE_BREAK);
runs_are_well_formed!(GRAPHEME_CLUSTER_BREAK);
runs_are_well_formed!(EAST_ASIAN_WIDTH);
runs_are_well_formed!(GENERAL_CATEGORY);
runs_are_well_formed!(INDIC_CONJUNCT_BREAK);
runs_are_well_formed!(EXTENDED_PICTOGRAPHIC);

const _: () = {
    let mut j = 1;
    while j < BIDI_BRACKETS.len() {
        assert!((BIDI_BRACKETS[j - 1].0 as u32) < BIDI_BRACKETS[j].0 as u32, "pairs ascend");
        j += 1;
    }
};

/// The value of the run `c` is in. At least one start is at most `c`, because
/// every table's first run starts at zero, which is asserted above.
fn run_value<T: Copy>(table: &[(u32, T)], c: char) -> T {
    let at = u32::from(c);
    let after = table.partition_point(|&(start, _)| start <= at);
    table[after - 1].1
}

/// The `Bidi_Class` of `c`, as the upstream file gives it — including for an
/// unassigned scalar, whose value is the file's own `@missing` default.
#[must_use]
pub fn bidi_class(c: char) -> BidiClass {
    run_value(BIDI_CLASS, c)
}

/// The bracket `c` pairs with and which side it is on, or `None` when its
/// `Bidi_Paired_Bracket_Type` is `None`.
#[must_use]
pub fn paired_bracket(c: char) -> Option<(char, BracketType)> {
    let at = BIDI_BRACKETS.binary_search_by_key(&c, |&(bracket, _, _)| bracket).ok()?;
    let (_, pair, kind) = BIDI_BRACKETS[at];
    Some((pair, kind))
}

/// The `Line_Break` of `c` as the upstream file gives it, before UAX #14's LB1
/// resolves any of it.
#[must_use]
pub fn line_break(c: char) -> LineBreak {
    run_value(LINE_BREAK, c)
}

/// The `Grapheme_Cluster_Break` of `c`.
#[must_use]
pub fn grapheme_cluster_break(c: char) -> GraphemeClusterBreak {
    run_value(GRAPHEME_CLUSTER_BREAK, c)
}

/// The `East_Asian_Width` of `c`.
#[must_use]
pub fn east_asian_width(c: char) -> EastAsianWidth {
    run_value(EAST_ASIAN_WIDTH, c)
}

/// The `General_Category` of `c`, `Cn` for an unassigned scalar.
#[must_use]
pub fn general_category(c: char) -> GeneralCategory {
    run_value(GENERAL_CATEGORY, c)
}

/// The `Indic_Conjunct_Break` of `c`.
#[must_use]
pub fn indic_conjunct_break(c: char) -> IndicConjunctBreak {
    run_value(INDIC_CONJUNCT_BREAK, c)
}

/// Whether `c` has `Extended_Pictographic`.
#[must_use]
pub fn extended_pictographic(c: char) -> bool {
    run_value(EXTENDED_PICTOGRAPHIC, c)
}

#[cfg(test)]
mod tests {
    use super::{
        BIDI_BRACKETS, BIDI_CLASS, BidiClass, BracketType, EastAsianWidth, GeneralCategory,
        GraphemeClusterBreak, IndicConjunctBreak, LineBreak, bidi_class, east_asian_width,
        extended_pictographic, general_category, grapheme_cluster_break, indic_conjunct_break,
        line_break, paired_bracket,
    };

    /// One scalar per value, each looked up in the upstream file rather than
    /// remembered — every one is listed there explicitly — so this is the
    /// generated table agreeing with the file at twenty-three points, one per
    /// value, and not a second copy of the table.
    #[test]
    fn every_value_is_where_the_upstream_file_puts_it() {
        use BidiClass::{
            AL, AN, B, BN, CS, EN, ES, ET, FSI, L, LRE, LRI, LRO, NSM, ON, PDF, PDI, R, RLE, RLI,
            RLO, S, WS,
        };
        for (c, want) in [
            ('A', L),
            ('\u{05D0}', R),
            ('\u{0627}', AL),
            ('0', EN),
            ('\u{0660}', AN),
            (' ', WS),
            ('\n', B),
            ('\t', S),
            ('\u{0300}', NSM),
            ('\u{00AD}', BN),
            ('!', ON),
            ('+', ES),
            ('#', ET),
            (',', CS),
            ('\u{202A}', LRE),
            ('\u{202B}', RLE),
            ('\u{202C}', PDF),
            ('\u{202D}', LRO),
            ('\u{202E}', RLO),
            ('\u{2066}', LRI),
            ('\u{2067}', RLI),
            ('\u{2068}', FSI),
            ('\u{2069}', PDI),
        ] {
            assert_eq!(bidi_class(c), want, "U+{:04X}", u32::from(c));
        }
    }

    /// Unassigned scalars, which the file lists nowhere and gives values only
    /// through its `@missing` lines. These are the cases a generator that chose
    /// its own default would get wrong while every assigned scalar stayed right.
    #[test]
    fn an_unassigned_scalar_carries_the_files_default() {
        assert_eq!(bidi_class('\u{05FF}'), BidiClass::R, "unassigned Hebrew");
        assert_eq!(bidi_class('\u{07BF}'), BidiClass::AL, "unassigned Thaana");
        assert_eq!(bidi_class('\u{20CF}'), BidiClass::ET, "unassigned Currency Symbols");
        assert_eq!(bidi_class('\u{0378}'), BidiClass::L, "unassigned Greek");
        assert_eq!(bidi_class('\u{10FFFF}'), BidiClass::BN, "a noncharacter");
    }

    #[test]
    fn every_value_occurs_and_is_declared_once() {
        for (i, value) in BidiClass::ALL.iter().enumerate() {
            assert!(BIDI_CLASS.iter().any(|(_, v)| v == value), "{value:?} has no run");
            assert!(!BidiClass::ALL[..i].contains(value), "{value:?} is declared twice");
        }
        assert_eq!(BidiClass::ALL.len(), 23, "UAX #9 names twenty-three classes");
    }

    /// Every pair is a pair both ways and one of each kind, which is what
    /// rule N0 reads the table for. The upstream file states it in prose; this
    /// is the prose checked against the data.
    #[test]
    fn every_bracket_pairs_back_with_the_opposite_type() {
        for &(bracket, pair, kind) in BIDI_BRACKETS {
            let (back, other) = paired_bracket(pair).expect("the pair is listed");
            assert_eq!(back, bracket, "U+{:04X}", u32::from(bracket));
            assert_ne!(kind, other, "U+{:04X}", u32::from(bracket));
        }
        assert_eq!(paired_bracket('('), Some((')', BracketType::Open)));
        assert_eq!(paired_bracket(']'), Some(('[', BracketType::Close)));
        assert_eq!(paired_bracket('a'), None);
    }

    /// The six tables `E3-B03f` added, each at points the upstream files list
    /// explicitly, and each at one point only a default reaches — the case a
    /// generator that chose its own default would get wrong. The conformance
    /// run is the exhaustive evidence; this is the lookup reading the right
    /// table at all.
    #[test]
    fn the_break_properties_are_where_their_files_put_them() {
        assert_eq!(line_break(' '), LineBreak::SP);
        assert_eq!(line_break('\u{2010}'), LineBreak::HH, "Unicode 17's unambiguous hyphen");
        assert_eq!(line_break('\u{0E01}'), LineBreak::SA);
        assert_eq!(line_break('\u{3400}'), LineBreak::ID);
        assert_eq!(line_break('\u{0378}'), LineBreak::XX, "unassigned: the file's default");
        assert_eq!(grapheme_cluster_break('\u{0308}'), GraphemeClusterBreak::Extend);
        assert_eq!(grapheme_cluster_break('\u{1F1E6}'), GraphemeClusterBreak::RegionalIndicator);
        assert_eq!(grapheme_cluster_break('\u{AC01}'), GraphemeClusterBreak::LVT);
        assert_eq!(grapheme_cluster_break('a'), GraphemeClusterBreak::Other, "the default");
        assert_eq!(east_asian_width('\u{4E00}'), EastAsianWidth::W);
        assert_eq!(east_asian_width('\u{FF01}'), EastAsianWidth::F);
        assert_eq!(east_asian_width('\u{0378}'), EastAsianWidth::N, "the default");
        assert_eq!(general_category('\u{0E31}'), GeneralCategory::Mn);
        assert_eq!(general_category('\u{00AB}'), GeneralCategory::Pi);
        assert_eq!(general_category('\u{0378}'), GeneralCategory::Cn, "listed, not defaulted");
        assert_eq!(indic_conjunct_break('\u{0915}'), IndicConjunctBreak::Consonant);
        assert_eq!(indic_conjunct_break('\u{094D}'), IndicConjunctBreak::Linker);
        assert_eq!(indic_conjunct_break('a'), IndicConjunctBreak::None, "the default");
        assert!(extended_pictographic('\u{00A9}'));
        assert!(extended_pictographic('\u{1FFFD}'), "reserved, and listed as pictographic");
        assert!(!extended_pictographic('a'), "the file's sentence for everything unlisted");
    }
}
