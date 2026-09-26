// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Extended grapheme clusters: UAX #29's boundaries, from scalars to the units
//! a reader sees as one character.
//!
//! # The level this conforms at
//!
//! **Extended grapheme clusters, every rule of UAX #29's section 3.1.1 table
//! from GB1 to GB999, untailored** — GB1–GB8, GB9, GB9a, GB9b, GB9c, GB11,
//! GB12, GB13 and GB999, which are the rules `GraphemeBreakTest.txt`'s comments
//! cite by number. *Extended* rather than *legacy*: GB9a and GB9b are in, which
//! is what makes a Devanagari vowel sign or a Thai spacing mark part of the
//! cluster before it rather than a cluster of its own. Nothing is tailored,
//! because the corpus tests the default and a tailoring is a claim this tree
//! would have to argue per script (`crate::corpus`'s job, not this module's).
//!
//! The evidence is `text/tests/break_conformance.rs`, which runs every case of
//! the specification's file over [`cluster_boundaries`] and prints the count on
//! every run. The argued eight are this module's own tests, each derived with
//! its rules named.
//!
//! # Why a running state and no loan
//!
//! Every rule UAX #29 writes with a look backwards — GB9c's consonant, linker
//! and extenders, GB11's pictograph and extenders before a ZWJ, GB12 and GB13's
//! count of regional indicators — is a regular language over what came before,
//! so each is carried forward as a small running value rather than searched
//! for, and no rule looks ahead at all. So a boundary is decided the moment the
//! scalar after it arrives, [`Clusters`] fits in eight bytes, and unlike
//! `crate::bidi` nothing is lent: the whole of the working set is those eight
//! bytes, whatever the length of the text.
//!
//! # Determinism
//!
//! Every decision is a comparison of enumerated values from the generated
//! tables, so nothing here rounds and nothing observes time, randomness or
//! ordering; a boundary is a pure function of the scalars before and at it.

use crate::grapheme_break::GraphemeClusterBreak;
use crate::indic_conjunct_break::IndicConjunctBreak;
use crate::property::{extended_pictographic, grapheme_cluster_break, indic_conjunct_break};

use GraphemeClusterBreak::{
    CR, Control, Extend, L, LF, LV, LVT, Prepend, RegionalIndicator, SpacingMark, T, V, ZWJ,
};

/// The rule that decided one position, by the number UAX #29 gives it.
///
/// Returned so that a failure names the rule this module applied beside the
/// rule the corpus file names, which is the shortest route from a red case to
/// the line of the specification that disagrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "UAX #29's own rule numbers, which a reader searches for"
)]
pub enum Rule {
    /// Break at the start of text.
    GB1,
    /// Do not break between a CR and an LF.
    GB3,
    /// Break after a control, CR or LF.
    GB4,
    /// Break before a control, CR or LF.
    GB5,
    /// Do not break Hangul syllable sequences: L before L, V, LV or LVT.
    GB6,
    /// LV or V before V or T.
    GB7,
    /// LVT or T before T.
    GB8,
    /// Do not break before extending characters or ZWJ.
    GB9,
    /// Do not break before a spacing mark.
    GB9a,
    /// Do not break after a prepended character.
    GB9b,
    /// Do not break within an Indic conjunct: a consonant, then linkers and
    /// extenders with at least one linker, before a consonant.
    GB9c,
    /// Do not break within an emoji ZWJ sequence: a pictograph, extenders and a
    /// ZWJ, before a pictograph.
    GB11,
    /// Do not break within a pair of regional indicators at the start of text.
    GB12,
    /// Do not break within a pair of regional indicators after a non-indicator.
    GB13,
    /// Otherwise, break everywhere.
    GB999,
}

impl Rule {
    /// Whether this rule puts a boundary where it applies.
    #[must_use]
    pub const fn breaks(self) -> bool {
        matches!(self, Self::GB1 | Self::GB4 | Self::GB5 | Self::GB999)
    }
}

/// GB11's look backwards, carried forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Emoji {
    /// Nothing GB11 can use.
    None,
    /// `\p{Extended_Pictographic} Extend*` ends here.
    Pictograph,
    /// `\p{Extended_Pictographic} Extend* ZWJ` ends here.
    Joined,
}

/// GB9c's look backwards, carried forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Conjunct {
    /// Nothing GB9c can use.
    None,
    /// A consonant and extenders, no linker yet.
    Consonant,
    /// A consonant, then extenders and linkers with at least one linker.
    Linked,
}

/// The state one boundary decision needs about everything before it.
#[derive(Clone, Copy, Debug)]
pub struct Clusters {
    /// The last scalar's property, or `None` before the first: GB1's `sot`.
    previous: Option<GraphemeClusterBreak>,
    /// An odd number of regional indicators ends here, uninterrupted.
    odd_indicators: bool,
    /// The indicators that end here began at the start of text, which is the
    /// difference between GB12 and GB13 and nothing else.
    indicators_from_start: bool,
    emoji: Emoji,
    conjunct: Conjunct,
}

// The module's claim that the whole working set is a few bytes, held where a
// field added to the state would break it.
const _: () = assert!(core::mem::size_of::<Clusters>() <= 8, "the state is eight bytes at most");

impl Default for Clusters {
    fn default() -> Self {
        Self::new()
    }
}

impl Clusters {
    /// The state at the start of text.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous: None,
            odd_indicators: false,
            indicators_from_start: false,
            emoji: Emoji::None,
            conjunct: Conjunct::None,
        }
    }

    /// The rule that decides whether a boundary falls before `c`, given every
    /// scalar pushed so far; then `c` is pushed. GB2, the boundary at the end
    /// of text, is the caller's, since nothing arrives after the last scalar.
    pub fn push(&mut self, c: char) -> Rule {
        let class = grapheme_cluster_break(c);
        let pictographic = extended_pictographic(c);
        let incb = indic_conjunct_break(c);
        let rule = self.decide(class, pictographic, incb);
        self.record(class, pictographic, incb);
        rule
    }

    fn decide(
        &self,
        class: GraphemeClusterBreak,
        pictographic: bool,
        incb: IndicConjunctBreak,
    ) -> Rule {
        let Some(previous) = self.previous else { return Rule::GB1 };
        match (previous, class) {
            (CR, LF) => Rule::GB3,
            (Control | CR | LF, _) => Rule::GB4,
            (_, Control | CR | LF) => Rule::GB5,
            (L, L | V | LV | LVT) => Rule::GB6,
            (LV | V, V | T) => Rule::GB7,
            (LVT | T, T) => Rule::GB8,
            (_, Extend | ZWJ) => Rule::GB9,
            (_, SpacingMark) => Rule::GB9a,
            (Prepend, _) => Rule::GB9b,
            _ if self.conjunct == Conjunct::Linked && incb == IndicConjunctBreak::Consonant => {
                Rule::GB9c
            }
            _ if self.emoji == Emoji::Joined && pictographic => Rule::GB11,
            // GB12 and GB13 are one rule with two left contexts, `sot` and a
            // non-indicator; which one applies changes only the name.
            (RegionalIndicator, RegionalIndicator) if self.odd_indicators => {
                if self.indicators_from_start { Rule::GB12 } else { Rule::GB13 }
            }
            _ => Rule::GB999,
        }
    }

    fn record(
        &mut self,
        class: GraphemeClusterBreak,
        pictographic: bool,
        incb: IndicConjunctBreak,
    ) {
        let continues = self.previous == Some(RegionalIndicator);
        self.odd_indicators = class == RegionalIndicator && !(continues && self.odd_indicators);
        self.indicators_from_start =
            if continues { self.indicators_from_start } else { self.previous.is_none() };
        self.emoji = if pictographic {
            Emoji::Pictograph
        } else {
            match (self.emoji, class) {
                (Emoji::Pictograph, Extend) => Emoji::Pictograph,
                (Emoji::Pictograph, ZWJ) => Emoji::Joined,
                _ => Emoji::None,
            }
        };
        self.conjunct = match (self.conjunct, incb) {
            (_, IndicConjunctBreak::Consonant) => Conjunct::Consonant,
            (Conjunct::Consonant | Conjunct::Linked, IndicConjunctBreak::Linker) => {
                Conjunct::Linked
            }
            (held @ (Conjunct::Consonant | Conjunct::Linked), IndicConjunctBreak::Extend) => held,
            _ => Conjunct::None,
        };
        self.previous = Some(class);
    }
}

/// Every extended grapheme cluster boundary in `text`, ascending, as the index
/// of the scalar the boundary is before — `text.len()` for the end.
///
/// The start and the end are boundaries (GB1, GB2) unless the text is empty,
/// which has none: UAX #29 says so in as many words, and a caller placing a
/// caret in empty text has one position without asking this.
#[must_use]
pub fn cluster_boundaries(text: &[char]) -> ClusterBoundaries<'_> {
    ClusterBoundaries { text, at: 0, rules: Clusters::new() }
}

/// The iterator [`cluster_boundaries`] returns.
#[derive(Clone, Debug)]
pub struct ClusterBoundaries<'a> {
    text: &'a [char],
    /// The next scalar to push, or `len + 1` once the end has been yielded.
    at: usize,
    rules: Clusters,
}

impl Iterator for ClusterBoundaries<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while let Some(&c) = self.text.get(self.at) {
            let at = self.at;
            self.at += 1;
            if self.rules.push(c).breaks() {
                return Some(at);
            }
        }
        // GB2: a boundary at the end of any text that has one.
        if self.at == self.text.len() && !self.text.is_empty() {
            self.at += 1;
            return Some(self.text.len());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Clusters, Rule, cluster_boundaries};
    use crate::corpus::Script;

    /// The largest sample in the corpus, in scalars, with room. Unit: scalars.
    const ROOM: usize = 64;

    fn scalars(sample: &str) -> ([char; ROOM], usize) {
        let mut out = ['\0'; ROOM];
        let mut n = 0;
        for c in sample.chars() {
            out[n] = c;
            n += 1;
        }
        (out, n)
    }

    /// The argued eight: RFC 0115's second home, for this pass. Each arm is
    /// the boundaries its sample has, as scalar indices, derived by rule.
    /// Exhaustive over [`Script`], so an entry added with no expectation here
    /// does not compile.
    fn argued(script: Script) -> &'static [usize] {
        match script {
            // Letters, spaces and a full stop are all `Other`: GB999 between
            // every pair, so every scalar is a cluster. 44 scalars, 45 bounds.
            Script::Latin => &[
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43,
                44,
            ],
            // Unvocalised Hebrew and a space: `Other` throughout, GB999.
            Script::Hebrew => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            // Unvocalised Arabic: `Other` throughout. Joining is the shaper's;
            // a cluster boundary does not know a letter joins.
            Script::Arabic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
            // न म स ् त े ␠ द ु न ि य ा. GB9 keeps U+094D (Extend, InCB=Linker)
            // and U+0947 (Extend) with what precedes them; GB9c keeps U+0924
            // (InCB=Consonant) after स् (Consonant, Linker) — so स्ते is one
            // cluster, the conjunct the corpus entry names. GB9 keeps U+0941;
            // GB9a keeps U+093F (SpacingMark) and U+093E (SpacingMark) with
            // their consonants. Clusters: न, म, स्ते, ␠, दु, नि, या.
            Script::Devanagari => &[0, 1, 2, 6, 7, 9, 11, 13],
            // ส ว ั ส ด ี ช า ว โ ล ก. U+0E31 and U+0E35 are Extend (GB9),
            // U+0E32 is `Other` — SARA AA is a letter, not a mark, in this
            // property — and U+0E42 SARA O is `Other` too, not `Prepend`,
            // so nothing joins across it. Clusters: ส, วั, ส, ดี, ช, า, ว, โ,
            // ล, ก.
            Script::Thai => &[0, 1, 3, 4, 6, 7, 8, 9, 10, 11, 12],
            // Ideographs are `Other`: GB999 between each.
            Script::Han => &[0, 1, 2, 3, 4],
            // U+D55C U+AE00, a space, then U+1112 U+1161 U+11AB U+1100 U+1173
            // U+11AF. The precomposed pair is LVT then LVT, and GB8 joins LVT
            // only to a following T, so they part (GB999). Each jamo syllable
            // is L, V, T: GB6 joins L to V, GB7 joins V to T, and T before L
            // parts. So the same two syllables are two clusters whichever way
            // they were written.
            Script::Hangul => &[0, 1, 2, 3, 6, 9],
            // Arabic, a space, two digits, a space, a Latin letter: `Other`
            // throughout, GB999. Direction is not a cluster property.
            Script::Mixed => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        }
    }

    #[test]
    fn every_corpus_entry_clusters_as_argued() {
        for script in Script::ALL {
            let (text, n) = scalars(script.sample());
            let mut got = [0usize; ROOM + 1];
            let mut k = 0;
            for at in cluster_boundaries(&text[..n]) {
                got[k] = at;
                k += 1;
            }
            assert_eq!(&got[..k], argued(script), "{}", script.name());
        }
    }

    #[test]
    fn empty_text_has_no_boundary_and_one_scalar_has_two() {
        assert_eq!(cluster_boundaries(&[]).count(), 0);
        let mut one = cluster_boundaries(&['a']);
        assert_eq!((one.next(), one.next(), one.next()), (Some(0), Some(1), None));
    }

    /// The three look-backs, each at the length where a searched version and a
    /// carried one could disagree: GB12/13 on the third indicator, GB11 after
    /// extenders, GB9c across an extender after the linker.
    #[test]
    fn the_carried_rules_hold_past_their_first_step() {
        let mut rules = Clusters::new();
        let flags = ['\u{1F1E6}', '\u{1F1E8}', '\u{1F1E6}', '\u{1F1E8}', '\u{1F1E6}'];
        let got: [Rule; 5] = flags.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB12, Rule::GB999, Rule::GB12, Rule::GB999]);
        let mut rules = Clusters::new();
        let after = ['a', '\u{1F1E6}', '\u{1F1E8}', '\u{1F1E6}'];
        let got: [Rule; 4] = after.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB999, Rule::GB13, Rule::GB999]);

        let mut rules = Clusters::new();
        let family = ['\u{1F468}', '\u{1F3FB}', '\u{200D}', '\u{1F469}'];
        let got: [Rule; 4] = family.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB9, Rule::GB11]);

        let mut rules = Clusters::new();
        let conjunct = ['\u{0915}', '\u{094D}', '\u{0300}', '\u{0937}'];
        let got: [Rule; 4] = conjunct.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB9, Rule::GB9c]);
    }

    // Two argued arms `GraphemeBreakTest.txt` does not reach: an audit on
    // 2026-09-26 found a mutant widening each rule green over the whole file.
    // The expected rules are derived from UAX #29 17.0's text, beside it.

    /// GB9c is `\p{InCB=Consonant} [\p{InCB=Extend}\p{InCB=Linker}]*
    /// \p{InCB=Linker} [\p{InCB=Extend}\p{InCB=Linker}]* × \p{InCB=Consonant}`,
    /// so a scalar with no InCB value ends the run. क (InCB Consonant), the
    /// virama (Extend, InCB Linker), `a` (Other, InCB None), क: GB9 keeps the
    /// virama with क; before `a` nothing holds (GB999); before the second क the
    /// run was ended by `a`, so GB9c does not hold and GB999 breaks.
    ///
    /// And the run must *start* at an InCB=Consonant: the rule's first term
    /// is not starred. `a` (InCB None), U+0300 (Extend, InCB Extend), the
    /// virama (Extend, InCB Linker), क: GB9 keeps both marks with `a`, and the
    /// bracketed run `Extend Linker` is there before क, but nothing before it
    /// is a Consonant, so GB9c does not hold and GB999 breaks before क. A
    /// re-audit on 2026-09-26 found a mutant that lets an Extend start the run
    /// green over the whole file.
    #[test]
    fn gb9c_ends_at_a_scalar_outside_the_conjunct() {
        let mut rules = Clusters::new();
        let broken = ['\u{0915}', '\u{094D}', 'a', '\u{0915}'];
        let got: [Rule; 4] = broken.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB999, Rule::GB999]);

        let mut rules = Clusters::new();
        let unstarted = ['a', '\u{0300}', '\u{094D}', '\u{0915}'];
        let got: [Rule; 4] = unstarted.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB9, Rule::GB999]);
    }

    /// GB11 is `\p{Extended_Pictographic} Extend* ZWJ × \p{Extended_Pictographic}`:
    /// the ZWJ is the scalar before the pictograph, and an Extend after it
    /// ends the sequence. U+1F468, ZWJ, U+0308 (Extend), U+1F469: GB9 holds the
    /// ZWJ and the mark with the man, and before the woman nothing holds, so
    /// GB999 breaks — and the text path, whose LB31 breaks there too (LB8a is
    /// the ZWJ's, and the mark is between), offers that break. Without the
    /// mark GB11 joins them, and the only opportunity is the end.
    ///
    /// `Extend*` is Grapheme_Cluster_Break=Extend and nothing else, and ZWJ is
    /// a value of its own: so between the pictograph and the one ZWJ GB11
    /// allows, neither a second ZWJ nor a ZWJ with extenders after it may
    /// stand. U+1F468, ZWJ, ZWJ, U+1F469: GB9 holds both joiners with the man;
    /// before the woman, `ExtPict Extend* ZWJ` would have to end at the second
    /// ZWJ with only extenders between it and a pictograph, and the first ZWJ
    /// is not an extender, so GB999 breaks. U+1F468, ZWJ, U+0308, ZWJ, U+1F469:
    /// the same, with a ZWJ and a mark between the man and the last ZWJ, and
    /// GB999 again. A re-audit on 2026-09-26 found a mutant accepting each
    /// green over the whole file; the text path cannot show these two, since
    /// LB8a holds every position after a ZWJ.
    #[test]
    fn gb11_needs_the_joiner_directly_before_the_pictograph() {
        let mut rules = Clusters::new();
        let parted = ['\u{1F468}', '\u{200D}', '\u{0308}', '\u{1F469}'];
        let got: [Rule; 4] = parted.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB9, Rule::GB999]);
        let mut offered = crate::line::opportunities(&parted).map(|o| o.at);
        assert_eq!((offered.next(), offered.next(), offered.next()), (Some(3), Some(4), None));

        let mut rules = Clusters::new();
        let joined = ['\u{1F468}', '\u{200D}', '\u{1F469}'];
        let got: [Rule; 3] = joined.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB11]);
        let mut offered = crate::line::opportunities(&joined).map(|o| o.at);
        assert_eq!((offered.next(), offered.next()), (Some(3), None));

        let mut rules = Clusters::new();
        let doubled = ['\u{1F468}', '\u{200D}', '\u{200D}', '\u{1F469}'];
        let got: [Rule; 4] = doubled.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB9, Rule::GB999]);

        let mut rules = Clusters::new();
        let rejoined = ['\u{1F468}', '\u{200D}', '\u{0308}', '\u{200D}', '\u{1F469}'];
        let got: [Rule; 5] = rejoined.map(|c| rules.push(c));
        assert_eq!(got, [Rule::GB1, Rule::GB9, Rule::GB9, Rule::GB9, Rule::GB999]);
    }
}
