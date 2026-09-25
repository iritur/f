// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Character properties, read out of the tables generated from Unicode's data.
//!
//! [`crate::bidi_class`] and [`crate::bidi_brackets`] are data under two
//! licences — this tree's and Unicode's — because they are Unicode's data
//! re-spelled as Rust (RFC 0114). This module is the code that reads them, and
//! it is here rather than in them for that reason: a lookup written into a
//! generated file would be code under the data's terms too, and the count of
//! files in this tree that are not purely liftable would grow by whatever the
//! generator happened to emit. So the generated files hold values and nothing
//! runs in them, and every decision about *how* a value is found is in this
//! file, under this tree's terms, where a reviewer reads it as code.
//!
//! # What is checked at compile time
//!
//! The shape the lookups rest on: the runs start at zero, ascend strictly and
//! stay inside the code space, and no two adjacent runs carry one value — so a
//! binary search over starts finds the one run a code point is in. The bracket
//! pairs ascend strictly, so a binary search finds one. A regenerated table that
//! broke either would not compile, which is earlier than any test.
//!
//! # What is not here
//!
//! The algorithm. `E3-B03e` writes UAX #9 against these two functions; this file
//! answers *what is this character* and nothing about what a paragraph does with
//! it. UAX #9's rule N0 also treats canonically equivalent brackets as a pair —
//! U+2329 and U+232A with U+3008 and U+3009 — and that is the algorithm's rule,
//! not a value [`paired_bracket`] invents.

use crate::bidi_brackets::{BIDI_BRACKETS, BracketType};
use crate::bidi_class::{BIDI_CLASS, BidiClass};

/// One past the last code point. Unit: code points.
const CODE_SPACE: u32 = 0x11_0000;

const _: () = {
    assert!(!BIDI_CLASS.is_empty() && BIDI_CLASS[0].0 == 0, "the first run starts at zero");
    let mut i = 1;
    while i < BIDI_CLASS.len() {
        assert!(BIDI_CLASS[i - 1].0 < BIDI_CLASS[i].0, "runs ascend strictly");
        assert!(BIDI_CLASS[i - 1].1 as u8 != BIDI_CLASS[i].1 as u8, "runs are maximal");
        i += 1;
    }
    assert!(BIDI_CLASS[BIDI_CLASS.len() - 1].0 < CODE_SPACE, "runs stay in the code space");

    let mut j = 1;
    while j < BIDI_BRACKETS.len() {
        assert!((BIDI_BRACKETS[j - 1].0 as u32) < BIDI_BRACKETS[j].0 as u32, "pairs ascend");
        j += 1;
    }
};

/// The `Bidi_Class` of `c`, as the upstream file gives it — including for an
/// unassigned scalar, whose value is the file's own `@missing` default.
#[must_use]
pub fn bidi_class(c: char) -> BidiClass {
    let at = u32::from(c);
    // At least one start is at most `at`, because the first run starts at zero,
    // which the assertion above holds at compile time.
    let after = BIDI_CLASS.partition_point(|&(start, _)| start <= at);
    BIDI_CLASS[after - 1].1
}

/// The bracket `c` pairs with and which side it is on, or `None` when its
/// `Bidi_Paired_Bracket_Type` is `None`.
#[must_use]
pub fn paired_bracket(c: char) -> Option<(char, BracketType)> {
    let at = BIDI_BRACKETS.binary_search_by_key(&c, |&(bracket, _, _)| bracket).ok()?;
    let (_, pair, kind) = BIDI_BRACKETS[at];
    Some((pair, kind))
}

#[cfg(test)]
mod tests {
    use super::{BIDI_BRACKETS, BIDI_CLASS, BidiClass, BracketType, bidi_class, paired_bracket};

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
}
