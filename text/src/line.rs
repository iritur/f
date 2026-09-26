// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Line breaking: UAX #14's opportunities, and the ones the text path offers.
//!
//! # The level this conforms at
//!
//! **The default line breaking algorithm of UAX #14 section 6, untailored:
//! LB1's default resolution, then LB2 to LB31** — including LB8a, LB12a,
//! LB15a–LB15d, LB19a, LB20a, LB21a, LB21b, LB23a, LB28a, LB30a and LB30b,
//! which are the rules `LineBreakTest.txt`'s comments cite. Each rule below
//! carries its number. *Default* is the file's own word for what it tests, and
//! it is the whole of what is claimed: LB1 resolves `SA` to `AL` or `CM` by
//! `General_Category`, so a Thai sentence has **no** opportunity inside it,
//! which is `crate::corpus`'s Thai hazard stated rather than solved. Breaking
//! Thai between words needs a dictionary, which is a tailoring UAX #14 invites
//! and this tree has not argued. *What would reverse that:* the first surface
//! that sets Thai in a box narrower than its sentence; then the dictionary is
//! an import with a record, like every other table here.
//!
//! [`decisions`] is that algorithm, position by position, and the evidence is
//! `text/tests/break_conformance.rs`, which runs every case of the
//! specification's file over it and prints the count on every run.
//!
//! # What the text path is offered: [`opportunities`]
//!
//! The untailored algorithm breaks inside extended grapheme clusters. It
//! approximates a cluster with LB9 — *X (CM | ZWJ)\** — which is narrower than
//! UAX #29's. A combining mark after a space is one cluster by GB9 and two
//! units to LB9, since LB9 absorbs nothing into a space, so LB18 breaks between
//! them; a spacing mark of class `VF` after a letter LB28a does not count as a
//! base is one cluster by GB9a, and LB31 breaks it. `break_conformance` counts
//! every such opportunity in `LineBreakTest.txt`, by the rule that allowed it,
//! on every run, and the count is not zero. A line break inside a cluster
//! splits what a reader sees as one character across two lines, and a caret,
//! a selection and a hit test all move by clusters; so
//! [`opportunities`] yields a UAX #14 opportunity only where it is also a
//! cluster boundary, and nothing else. That is a tailoring of this tree's
//! (RFC 0139), and the narrowest one available: it only ever removes an
//! opportunity, never adds one, so a line this path sets is a line the default
//! algorithm could have set.
//!
//! *What would reverse the filter:* a script whose readers expect a break
//! between two scalars UAX #29 joins. None is known to this tree; the evidence
//! would be a corpus entry whose argued breaks the filter removes, and the
//! repair would be a tailoring of UAX #29 argued for that script rather than a
//! second route around the filter.
//!
//! A mandatory break is never removed, and needs no filtering: LB4 and LB5 put
//! one after `BK`, `CR`, `LF` and `NL`, every scalar of those four classes is a
//! `Control`, `CR` or `LF` to UAX #29, and GB4 breaks after every one of them.
//! A unit test holds that over the whole code space rather than over the four
//! scalars a reader would think of.
//!
//! # Why a lookahead and no loan
//!
//! Every rule UAX #14 writes with a look backwards — LB8 and LB14–LB17 across
//! spaces, LB15a's context before a quotation mark, LB20a and LB21a's word
//! before a hyphen, LB25's number, LB28a's base before a virama, LB30a's count
//! of regional indicators — is a regular language over what came before, so it
//! is carried forward as a running value, [`crate::bidi`]'s pattern. Five rules
//! also look one unit ahead (LB15b, LB15c, LB19a, LB25 and LB28a), read out of
//! the text the caller already holds, and one part of LB25 looks two ahead:
//! `(PO | PR) × OP IS NU`. LB9 makes a unit any length, but a scalar is read
//! into a unit once as the walk reaches it and at most once more, when that
//! part of LB25 looks past an `OP IS` — so the walk is linear, and nothing
//! grows with the text: the state is a few units.
//!
//! # Determinism
//!
//! Every decision is a comparison of enumerated values from the generated
//! tables. Nothing here rounds, and nothing observes time, randomness or
//! ordering; a decision is a pure function of the scalars.

use crate::east_asian_width::EastAsianWidth;
use crate::general_category::GeneralCategory;
use crate::grapheme::Clusters;
use crate::line_break::LineBreak;
use crate::property::{east_asian_width, extended_pictographic, general_category, line_break};

use LineBreak::{
    AI, AK, AL, AP, AS, B2, BA, BB, BK, CB, CJ, CL, CM, CP, CR, EB, EM, EX, GL, H2, H3, HH, HL, HY,
    ID, IN, IS, JL, JT, JV, LF, NL, NS, NU, OP, PO, PR, QU, RI, SA, SG, SP, SY, VF, VI, WJ, XX, ZW,
    ZWJ,
};

/// The rule that decided one position, by the number UAX #14 gives it.
///
/// Returned so that a failure names the rule applied here beside the rule the
/// corpus names, which is the shortest route from a red case to the line of
/// the specification that disagrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "UAX #14's own rule numbers, which a reader searches for"
)]
pub enum Rule {
    /// Always break at the end of text.
    LB3,
    /// Always break after a hard line break.
    LB4,
    /// CR × LF, and a mandatory break after CR, LF and NL.
    LB5,
    /// Do not break before a hard line break.
    LB6,
    /// Do not break before spaces or zero width space.
    LB7,
    /// Break before any character following a zero-width space, even after
    /// spaces.
    LB8,
    /// Do not break after a zero width joiner.
    LB8a,
    /// Do not break a combining character sequence.
    LB9,
    /// Do not break before or after a word joiner.
    LB11,
    /// Do not break after a non-breaking character.
    LB12,
    /// Do not break before a non-breaking character, except after spaces and
    /// hyphens.
    LB12a,
    /// Do not break before `]`, `!` or `/`, even after spaces.
    LB13,
    /// Do not break after `[`, even after spaces.
    LB14,
    /// Do not break after an unresolved initial quotation mark, even after
    /// spaces.
    LB15a,
    /// Do not break before an unresolved final quotation mark.
    LB15b,
    /// Break before a decimal mark that begins a number, after a space.
    LB15c,
    /// Otherwise, do not break before `;`, `,` or `.`, even after spaces.
    LB15d,
    /// Do not break between closing punctuation and a nonstarter, even with
    /// spaces between.
    LB16,
    /// Do not break within `——`, even with spaces between.
    LB17,
    /// Break after spaces.
    LB18,
    /// Do not break before or after a quotation mark that is not initial or
    /// final respectively.
    LB19,
    /// Unless surrounded by East Asian characters, do not break either side of
    /// any quotation mark.
    LB19a,
    /// Break before and after unresolved contingent breaks.
    LB20,
    /// Do not break after a word-initial hyphen.
    LB20a,
    /// Do not break before hyphen-minus, other hyphens, fixed-width spaces,
    /// small kana and other non-starters, or after acute accents.
    LB21,
    /// Do not break after the hyphen in Hebrew + hyphen + non-Hebrew.
    LB21a,
    /// Do not break between a solidus and Hebrew letters.
    LB21b,
    /// Do not break before an ellipsis.
    LB22,
    /// Do not break between digits and letters.
    LB23,
    /// Do not break between numeric prefixes and ideographs, or between
    /// ideographs and numeric postfixes.
    LB23a,
    /// Do not break between numeric prefix or postfix and letters.
    LB24,
    /// Do not break numbers.
    LB25,
    /// Do not break a Korean syllable.
    LB26,
    /// Treat a Korean syllable block like an ideograph.
    LB27,
    /// Do not break between alphabetics.
    LB28,
    /// Do not break inside the orthographic syllables of Brahmic scripts.
    LB28a,
    /// Do not break between numeric punctuation and alphabetics.
    LB29,
    /// Do not break between letters, numbers or ordinary symbols and opening
    /// or closing parentheses that are not East Asian.
    LB30,
    /// Break between two regional indicator symbols if and only if there are
    /// an even number of them before the break point.
    LB30a,
    /// Do not break between an emoji base, or an unassigned pictograph, and an
    /// emoji modifier.
    LB30b,
    /// Otherwise, break everywhere.
    LB31,
}

/// What a rule says about one position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `!`: the line must end here.
    Mandatory,
    /// `÷`: the line may end here.
    Allowed,
    /// `×`: the line may not end here.
    Prohibited,
}

/// One position's answer: the scalar index the break would be before, the
/// rule that decided it, and what it decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The index of the scalar the position is before; the text's length for
    /// the end. Unit: scalars.
    pub at: usize,
    /// The rule that decided it.
    pub rule: Rule,
    /// What the rule decided.
    pub outcome: Outcome,
}

const fn prohibited(rule: Rule) -> Option<(Rule, Outcome)> {
    Some((rule, Outcome::Prohibited))
}

const fn allowed(rule: Rule) -> Option<(Rule, Outcome)> {
    Some((rule, Outcome::Allowed))
}

/// LB1: the class the rest of the rules read. `AI`, `SG` and `XX` are `AL`;
/// `SA` is `CM` where it is a mark (`Mn` or `Mc`) and `AL` otherwise; `CJ` is
/// `NS`. Those are the default resolutions — the ones `LineBreakTest.txt` is
/// written against — and every one of them is a tailoring point this tree has
/// not taken.
fn resolved(c: char) -> LineBreak {
    match line_break(c) {
        AI | SG | XX => AL,
        SA => {
            if matches!(general_category(c), GeneralCategory::Mn | GeneralCategory::Mc) {
                CM
            } else {
                AL
            }
        }
        CJ => NS,
        other => other,
    }
}

/// One unit the pair rules see: a scalar, and after LB9 every `CM` and `ZWJ`
/// it absorbs. The properties are its first scalar's, because LB9 says to
/// treat the sequence *as if it were X*.
#[derive(Clone, Copy, Debug)]
struct Unit {
    /// The class after LB1 and LB10.
    class: LineBreak,
    /// The first scalar's index. Unit: scalars.
    start: usize,
    /// One past the last scalar LB9 absorbed. Unit: scalars.
    end: usize,
    /// `East_Asian_Width` is `F`, `W` or `H`: UAX #14's `$EastAsian`.
    east_asian: bool,
    /// `QU` and `General_Category=Pi`.
    initial_quote: bool,
    /// `QU` and `General_Category=Pf`.
    final_quote: bool,
    /// U+25CC, the one scalar LB28a names.
    dotted_circle: bool,
    /// `Extended_Pictographic` and `General_Category=Cn`, LB30b's second left
    /// side.
    unassigned_pictograph: bool,
    /// The last scalar is a ZWJ, which LB8a reads before LB9 absorbs it.
    ends_joined: bool,
}

impl Unit {
    /// The unit starting at `start`, or `None` at the end of text.
    fn at(text: &[char], start: usize) -> Option<Self> {
        let &c = text.get(start)?;
        let first = resolved(c);
        let mut end = start + 1;
        let mut last = first;
        // LB9: X (CM | ZWJ)* is X, where X is anything but BK, CR, LF, NL, SP
        // and ZW. A CM that nothing absorbs is X itself, so it absorbs too.
        if !matches!(first, BK | CR | LF | NL | SP | ZW) {
            while let Some(&m) = text.get(end) {
                let k = resolved(m);
                if !matches!(k, CM | ZWJ) {
                    break;
                }
                last = k;
                end += 1;
            }
        }
        let category = general_category(c);
        Some(Self {
            // LB10: a CM or ZWJ nothing absorbed is AL.
            class: if matches!(first, CM | ZWJ) { AL } else { first },
            start,
            end,
            east_asian: matches!(
                east_asian_width(c),
                EastAsianWidth::F | EastAsianWidth::W | EastAsianWidth::H
            ),
            initial_quote: first == QU && category == GeneralCategory::Pi,
            final_quote: first == QU && category == GeneralCategory::Pf,
            dotted_circle: c == '\u{25CC}',
            unassigned_pictograph: extended_pictographic(c) && category == GeneralCategory::Cn,
            ends_joined: last == ZWJ,
        })
    }

    /// LB28a's `(AK | ◌ | AS)`.
    const fn brahmic_base(&self) -> bool {
        matches!(self.class, AK | AS) || self.dotted_circle
    }
}

/// LB25's look backwards: how much of `NU (SY | IS)* (CL | CP)?` ends at
/// the unit before the position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Number {
    /// None of it.
    Outside,
    /// `NU (SY | IS)*`: a later NU starts the run again, which is the same.
    Digits,
    /// `NU (SY | IS)* (CL | CP)`.
    Closed,
}

/// Everything the pair rules need about the text before the position, carried.
#[derive(Clone, Copy, Debug)]
struct Behind {
    /// The unit before the one before the position, or `None` for `sot`.
    before: Option<Unit>,
    /// The last unit before the position that is not `SP`: LB8, LB14, LB15a,
    /// LB16 and LB17 all read *X SP\**.
    held: LineBreak,
    /// The held unit is an initial quotation mark in LB15a's left context.
    held_opens: bool,
    number: Number,
    /// An odd number of regional indicators ends before the position.
    odd_indicators: bool,
}

impl Behind {
    /// The state after the first unit, whose left is `sot`.
    const fn start(first: &Unit) -> Self {
        Self {
            before: None,
            held: first.class,
            // LB15a's `sot` alternative.
            held_opens: first.initial_quote,
            number: if matches!(first.class, NU) { Number::Digits } else { Number::Outside },
            odd_indicators: matches!(first.class, RI),
        }
    }

    /// The state once `b` has moved behind the position after `a`.
    fn step(&mut self, a: &Unit, b: &Unit) {
        if b.class != SP {
            self.held = b.class;
            self.held_opens =
                b.initial_quote && matches!(a.class, BK | CR | LF | NL | OP | QU | GL | SP | ZW);
        }
        self.number = match (b.class, self.number) {
            (NU, _) => Number::Digits,
            (SY | IS, Number::Digits) => Number::Digits,
            (CL | CP, Number::Digits) => Number::Closed,
            _ => Number::Outside,
        };
        self.odd_indicators = b.class == RI && !(a.class == RI && self.odd_indicators);
        self.before = Some(*a);
    }

    /// The rule that decides the position between `a` and `b`, with `c` the
    /// unit after `b` (`None` for `eot`), in UAX #14's order: the first rule
    /// that matches decides. LB2 and LB3 are the walk's, and LB9 and LB10 are
    /// [`Unit::at`]'s.
    fn decide(&self, text: &[char], a: &Unit, b: &Unit, c: Option<&Unit>) -> (Rule, Outcome) {
        self.pair(text, a, b, c).unwrap_or((Rule::LB31, Outcome::Allowed))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one rule per statement, in the specification's order, is what lets a reader \
                  hold the order against the specification"
    )]
    fn pair(&self, text: &[char], a: &Unit, b: &Unit, c: Option<&Unit>) -> Option<(Rule, Outcome)> {
        let (x, y) = (a.class, b.class);
        let before = self.before.as_ref();
        // LB4: BK !
        if x == BK {
            return Some((Rule::LB4, Outcome::Mandatory));
        }
        // LB5: CR × LF; CR !; LF !; NL !
        if x == CR && y == LF {
            return prohibited(Rule::LB5);
        }
        if matches!(x, CR | LF | NL) {
            return Some((Rule::LB5, Outcome::Mandatory));
        }
        // LB6: × (BK | CR | LF | NL)
        if matches!(y, BK | CR | LF | NL) {
            return prohibited(Rule::LB6);
        }
        // LB7: × SP; × ZW
        if matches!(y, SP | ZW) {
            return prohibited(Rule::LB7);
        }
        // LB8: ZW SP* ÷
        if self.held == ZW {
            return allowed(Rule::LB8);
        }
        // LB8a: ZWJ ×
        if a.ends_joined {
            return prohibited(Rule::LB8a);
        }
        // LB11: × WJ; WJ ×
        if x == WJ || y == WJ {
            return prohibited(Rule::LB11);
        }
        // LB12: GL ×
        if x == GL {
            return prohibited(Rule::LB12);
        }
        // LB12a: [^SP BA HY HH] × GL
        if y == GL && !matches!(x, SP | BA | HY | HH) {
            return prohibited(Rule::LB12a);
        }
        // LB13: × CL; × CP; × EX; × SY
        if matches!(y, CL | CP | EX | SY) {
            return prohibited(Rule::LB13);
        }
        // LB14: OP SP* ×
        if self.held == OP {
            return prohibited(Rule::LB14);
        }
        // LB15a: (sot | BK | CR | LF | NL | OP | QU | GL | SP | ZW)
        //        [\p{Pi}&QU] SP* ×
        if self.held_opens {
            return prohibited(Rule::LB15a);
        }
        // LB15b: × [\p{Pf}&QU] ( SP | GL | WJ | CL | QU | CP | EX | IS | SY
        //        | BK | CR | LF | NL | ZW | eot)
        if b.final_quote
            && c.is_none_or(|c| {
                matches!(
                    c.class,
                    SP | GL | WJ | CL | QU | CP | EX | IS | SY | BK | CR | LF | NL | ZW
                )
            })
        {
            return prohibited(Rule::LB15b);
        }
        // LB15c: SP ÷ IS NU
        if x == SP && y == IS && c.is_some_and(|c| c.class == NU) {
            return allowed(Rule::LB15c);
        }
        // LB15d: × IS
        if y == IS {
            return prohibited(Rule::LB15d);
        }
        // LB16: (CL | CP) SP* × NS
        if matches!(self.held, CL | CP) && y == NS {
            return prohibited(Rule::LB16);
        }
        // LB17: B2 SP* × B2
        if self.held == B2 && y == B2 {
            return prohibited(Rule::LB17);
        }
        // LB18: SP ÷
        if x == SP {
            return allowed(Rule::LB18);
        }
        // LB19: × [QU - \p{Pi}]; [QU - \p{Pf}] ×
        if (y == QU && !b.initial_quote) || (x == QU && !a.final_quote) {
            return prohibited(Rule::LB19);
        }
        // LB19a: [^$EastAsian] × QU; × QU ([^$EastAsian] | eot);
        //        QU × [^$EastAsian]; (sot | [^$EastAsian]) QU ×
        if (y == QU && (!a.east_asian || c.is_none_or(|c| !c.east_asian)))
            || (x == QU && (!b.east_asian || before.is_none_or(|p| !p.east_asian)))
        {
            return prohibited(Rule::LB19a);
        }
        // LB20: ÷ CB; CB ÷
        if x == CB || y == CB {
            return allowed(Rule::LB20);
        }
        // LB20a: (sot | BK | CR | LF | NL | SP | ZW | CB | GL) (HY | HH) × (AL | HL)
        if matches!(x, HY | HH)
            && matches!(y, AL | HL)
            && before.is_none_or(|p| matches!(p.class, BK | CR | LF | NL | SP | ZW | CB | GL))
        {
            return prohibited(Rule::LB20a);
        }
        // LB21: × BA; × HH; × HY; × NS; BB ×
        if matches!(y, BA | HH | HY | NS) || x == BB {
            return prohibited(Rule::LB21);
        }
        // LB21a: HL (HY | HH) × [^HL]
        if matches!(x, HY | HH) && y != HL && before.is_some_and(|p| p.class == HL) {
            return prohibited(Rule::LB21a);
        }
        // LB21b: SY × HL
        if x == SY && y == HL {
            return prohibited(Rule::LB21b);
        }
        // LB22: × IN
        if y == IN {
            return prohibited(Rule::LB22);
        }
        // LB23: (AL | HL) × NU; NU × (AL | HL)
        if (matches!(x, AL | HL) && y == NU) || (x == NU && matches!(y, AL | HL)) {
            return prohibited(Rule::LB23);
        }
        // LB23a: PR × (ID | EB | EM); (ID | EB | EM) × PO
        if (x == PR && matches!(y, ID | EB | EM)) || (matches!(x, ID | EB | EM) && y == PO) {
            return prohibited(Rule::LB23a);
        }
        // LB24: (PR | PO) × (AL | HL); (AL | HL) × (PR | PO)
        if (matches!(x, PR | PO) && matches!(y, AL | HL))
            || (matches!(x, AL | HL) && matches!(y, PR | PO))
        {
            return prohibited(Rule::LB24);
        }
        // LB25, whose fifteen parts are four shapes:
        //   NU (SY | IS)* (CL | CP)? × (PO | PR)     the first six
        //   (PO | PR) × OP IS? NU                     the next four
        //   (PO | PR | HY | IS) × NU                  the next four
        //   NU (SY | IS)* × NU                        the last
        // The first and last read the number behind, carried in `number`. The
        // second looks past the OP and, when an IS follows it, past that too:
        // the one place anything here reads two units ahead.
        let a_number_opens = || match c {
            Some(c) if c.class == NU => true,
            Some(c) if c.class == IS => Unit::at(text, c.end).is_some_and(|d| d.class == NU),
            _ => false,
        };
        if (self.number != Number::Outside && matches!(y, PO | PR))
            || (matches!(x, PO | PR) && y == OP && a_number_opens())
            || (matches!(x, PO | PR | HY | IS) && y == NU)
            || (self.number == Number::Digits && y == NU)
        {
            return prohibited(Rule::LB25);
        }
        // LB26: JL × (JL | JV | H2 | H3); (JV | H2) × (JV | JT); (JT | H3) × JT
        if (x == JL && matches!(y, JL | JV | H2 | H3))
            || (matches!(x, JV | H2) && matches!(y, JV | JT))
            || (matches!(x, JT | H3) && y == JT)
        {
            return prohibited(Rule::LB26);
        }
        // LB27: (JL | JV | JT | H2 | H3) × PO; PR × (JL | JV | JT | H2 | H3)
        if (matches!(x, JL | JV | JT | H2 | H3) && y == PO)
            || (x == PR && matches!(y, JL | JV | JT | H2 | H3))
        {
            return prohibited(Rule::LB27);
        }
        // LB28: (AL | HL) × (AL | HL)
        if matches!(x, AL | HL) && matches!(y, AL | HL) {
            return prohibited(Rule::LB28);
        }
        // LB28a: AP × (AK | ◌ | AS); (AK | ◌ | AS) × (VF | VI);
        //        (AK | ◌ | AS) VI × (AK | ◌); (AK | ◌ | AS) × (AK | ◌ | AS) VF
        if (x == AP && b.brahmic_base())
            || (a.brahmic_base() && matches!(y, VF | VI))
            || (x == VI && before.is_some_and(Unit::brahmic_base) && (y == AK || b.dotted_circle))
            || (a.brahmic_base() && b.brahmic_base() && c.is_some_and(|c| c.class == VF))
        {
            return prohibited(Rule::LB28a);
        }
        // LB29: IS × (AL | HL)
        if x == IS && matches!(y, AL | HL) {
            return prohibited(Rule::LB29);
        }
        // LB30: (AL | HL | NU) × [OP - $EastAsian]; [CP - $EastAsian] × (AL | HL | NU)
        if (matches!(x, AL | HL | NU) && y == OP && !b.east_asian)
            || (x == CP && !a.east_asian && matches!(y, AL | HL | NU))
        {
            return prohibited(Rule::LB30);
        }
        // LB30a: sot (RI RI)* RI × RI; [^RI] (RI RI)* RI × RI
        if x == RI && y == RI && self.odd_indicators {
            return prohibited(Rule::LB30a);
        }
        // LB30b: EB × EM; [\p{Extended_Pictographic}&\p{Cn}] × EM
        if y == EM && (x == EB || a.unassigned_pictograph) {
            return prohibited(Rule::LB30b);
        }
        None
    }
}

/// Every position in `text` after the first, ascending, with the rule that
/// decided it: the untailored algorithm, which is what the conformance file
/// tests. Position 0 is LB2's (never a break) and is not yielded; position
/// `text.len()` is LB3's (always one). Empty text yields nothing.
///
/// The text path wants [`opportunities`], which is this less the breaks that
/// fall inside a cluster.
#[must_use]
pub fn decisions(text: &[char]) -> Decisions<'_> {
    let a = Unit::at(text, 0);
    Decisions {
        text,
        behind: a.as_ref().map(Behind::start),
        b: a.and_then(|a| Unit::at(text, a.end)),
        inside: 1,
        a,
        ended: text.is_empty(),
    }
}

/// The iterator [`decisions`] returns.
#[derive(Clone, Debug)]
pub struct Decisions<'a> {
    text: &'a [char],
    /// The unit before the next pair position; `None` only for empty text.
    a: Option<Unit>,
    /// The unit after it; `None` at the end of text.
    b: Option<Unit>,
    behind: Option<Behind>,
    /// The next position inside `a` to report, LB9's.
    inside: usize,
    /// LB3's position has been yielded.
    ended: bool,
}

impl Iterator for Decisions<'_> {
    type Item = Decision;

    fn next(&mut self) -> Option<Decision> {
        let (a, behind) = (self.a?, self.behind.as_mut()?);
        if self.inside < a.end {
            let at = self.inside;
            self.inside += 1;
            // Inside a unit LB9 holds, unless LB8a, which comes first, already
            // did: the scalar before is a ZWJ.
            let rule = if line_break(self.text[at - 1]) == ZWJ { Rule::LB8a } else { Rule::LB9 };
            return Some(Decision { at, rule, outcome: Outcome::Prohibited });
        }
        if let Some(b) = self.b {
            let c = Unit::at(self.text, b.end);
            let (rule, outcome) = behind.decide(self.text, &a, &b, c.as_ref());
            behind.step(&a, &b);
            self.a = Some(b);
            self.b = c;
            self.inside = b.start + 1;
            return Some(Decision { at: b.start, rule, outcome });
        }
        if self.ended {
            return None;
        }
        self.ended = true;
        Some(Decision { at: self.text.len(), rule: Rule::LB3, outcome: Outcome::Mandatory })
    }
}

/// One place the text path may end a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opportunity {
    /// The index of the scalar the line would end before; the text's length
    /// for the end. Unit: scalars.
    pub at: usize,
    /// The line must end here: LB3, LB4 or LB5.
    pub mandatory: bool,
    /// The rule that allowed it.
    pub rule: Rule,
}

/// Every place the text path may end a line in `text`, ascending: each UAX #14
/// opportunity that is also an extended grapheme cluster boundary, and no
/// other. The module documentation says why, and what would reverse it.
#[must_use]
pub fn opportunities(text: &[char]) -> Opportunities<'_> {
    let mut clusters = Clusters::new();
    if let Some(&first) = text.first() {
        // GB1's position, which `decisions` never yields: it is LB2's.
        let _ = clusters.push(first);
    }
    Opportunities { decisions: decisions(text), clusters }
}

/// The iterator [`opportunities`] returns.
#[derive(Clone, Debug)]
pub struct Opportunities<'a> {
    decisions: Decisions<'a>,
    /// UAX #29's state, one scalar behind the next decision: [`decisions`]
    /// yields every position from 1 to the end exactly once and in order, so
    /// the scalar a decision is before is pushed as it arrives.
    clusters: Clusters,
}

impl Iterator for Opportunities<'_> {
    type Item = Opportunity;

    fn next(&mut self) -> Option<Opportunity> {
        loop {
            let d = self.decisions.next()?;
            // GB2 at the end; otherwise the cluster rule before this scalar.
            let boundary =
                self.decisions.text.get(d.at).is_none_or(|&c| self.clusters.push(c).breaks());
            let keep = match d.outcome {
                Outcome::Prohibited => false,
                Outcome::Mandatory => true,
                Outcome::Allowed => boundary,
            };
            if keep {
                return Some(Opportunity {
                    at: d.at,
                    mandatory: d.outcome == Outcome::Mandatory,
                    rule: d.rule,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Outcome::{Allowed, Mandatory, Prohibited};
    use super::{Outcome, Rule, decisions, opportunities};
    use crate::corpus::Script;
    use crate::grapheme_break::GraphemeClusterBreak;
    use crate::line_break::LineBreak;
    use crate::property::{grapheme_cluster_break, line_break};

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

    /// The argued eight, RFC 0115's second home for this pass: each arm is the
    /// sample's line-break opportunities as scalar indices, derived by rule.
    /// The same list is the answer for [`decisions`] and [`opportunities`],
    /// because no entry has a mark after a space or a `Prepend`, which is
    /// what the second would remove. Exhaustive over [`Script`].
    fn argued(script: Script) -> &'static [usize] {
        match script {
            // Letters are AL (LB28 joins them), a space is SP: LB7 holds the
            // position before it and LB18 breaks after it. `g.` is AL then IS,
            // held by LB15d. Spaces at 3, 9, 15, 19, 25, 30, 34 and 39; the
            // end is LB3's.
            Script::Latin => &[4, 10, 16, 20, 26, 31, 35, 40, 44],
            // HL throughout (LB28), one space: LB18 after it, LB3 at the end.
            Script::Hebrew => &[5, 9],
            // AL throughout (LB28), one space.
            Script::Arabic => &[6, 13],
            // The consonants are AL, the virama and the vowel signs CM: LB9
            // folds each mark into the letter before it and LB28 joins the
            // letters, so the only break inside the text is after the space.
            Script::Devanagari => &[7, 13],
            // Every scalar is SA. LB1 makes the letters AL and the marks
            // (U+0E31, U+0E35, Mn) CM, so LB28 and LB9 hold every position:
            // no opportunity but the end. This is the corpus entry's hazard,
            // stated in the module documentation rather than solved.
            Script::Thai => &[12],
            // ID after ID: nothing holds it, so LB31 breaks between each.
            Script::Han => &[1, 2, 3, 4],
            // U+D55C and U+AE00 are H3 (LVT syllables): LB26 joins H3 only to
            // a following JT, so LB31 breaks between them. After the space
            // (LB18), the jamo are JL JV JT JL JV JT: LB26 joins JL to JV and
            // JV to JT, and nothing joins JT to JL, so LB31 breaks there.
            Script::Hangul => &[1, 3, 6, 9],
            // AL (LB28), a space (LB18), NU NU (LB25), a space (LB18), AL.
            Script::Mixed => &[6, 9, 10],
        }
    }

    #[test]
    fn every_corpus_entry_breaks_as_argued() {
        for script in Script::ALL {
            let (text, n) = scalars(script.sample());
            let mut got = [0usize; ROOM + 1];
            let mut k = 0;
            for d in decisions(&text[..n]) {
                if d.outcome != Outcome::Prohibited {
                    got[k] = d.at;
                    k += 1;
                }
            }
            assert_eq!(&got[..k], argued(script), "{} untailored", script.name());
            k = 0;
            for o in opportunities(&text[..n]) {
                got[k] = o.at;
                k += 1;
            }
            assert_eq!(&got[..k], argued(script), "{} on the text path", script.name());
        }
    }

    /// The decision at `position` in a text written as code points.
    fn at<const N: usize>(code_points: [u32; N], position: usize) -> (Rule, Outcome) {
        let text = code_points.map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
        decisions(&text)
            .find(|d| d.at == position)
            .map(|d| (d.rule, d.outcome))
            .expect("a position after the first and no later than the end")
    }

    #[test]
    fn every_position_is_decided_once_in_order() {
        let text = ['a', '\u{0308}', '\u{200D}', ' ', '\u{0308}', '1', '\r', '\n', 'b'];
        let mut want = 1;
        for d in decisions(&text) {
            assert_eq!(d.at, want);
            want += 1;
        }
        assert_eq!(want, text.len() + 1);
        assert_eq!(decisions(&[]).count(), 0);
        assert_eq!(opportunities(&[]).count(), 0);
    }

    /// The filter's premise for mandatory breaks, over every scalar rather than
    /// the four a reader thinks of: whatever LB4 or LB5 breaks after, GB4
    /// breaks after too.
    #[test]
    fn every_mandatory_break_class_is_a_cluster_control() {
        let mut seen = 0u32;
        for c in (0..=0x10_FFFFu32).filter_map(char::from_u32) {
            if matches!(
                line_break(c),
                LineBreak::BK | LineBreak::CR | LineBreak::LF | LineBreak::NL
            ) {
                seen += 1;
                assert!(
                    matches!(
                        grapheme_cluster_break(c),
                        GraphemeClusterBreak::Control
                            | GraphemeClusterBreak::CR
                            | GraphemeClusterBreak::LF
                    ),
                    "U+{:04X}",
                    u32::from(c)
                );
            }
        }
        assert!(seen >= 7, "{seen}");
    }

    /// The filter at work, on the smallest text that needs it: a combining mark
    /// after a space is LB10's AL, so LB18 breaks before it, and GB9 keeps it
    /// with the space. LB28 then joins the mark, as AL, to `b`.
    #[test]
    fn a_mark_after_a_space_is_a_break_only_before_the_filter() {
        let text = ['a', ' ', '\u{0308}', 'b'];
        let mut raw = [(0, Rule::LB31, Outcome::Allowed); 4];
        for (slot, d) in raw.iter_mut().zip(decisions(&text)) {
            *slot = (d.at, d.rule, d.outcome);
        }
        assert_eq!(
            raw,
            [
                (1, Rule::LB7, Outcome::Prohibited),
                (2, Rule::LB18, Outcome::Allowed),
                (3, Rule::LB28, Outcome::Prohibited),
                (4, Rule::LB3, Outcome::Mandatory),
            ]
        );
        let mut kept = opportunities(&text);
        assert_eq!(kept.next().map(|o| (o.at, o.mandatory)), Some((4, true)));
        assert_eq!(kept.next(), None);
    }

    // The argued arms. `LineBreakTest.txt` passes whole, and still does not
    // reach every alternative of every rule: each test below is one an audit
    // on 2026-09-26 showed the file silent on (a mutant deleting the
    // alternative stayed green), with the expected decision derived from the
    // rule's text in UAX #14 17.0 and written beside it — not copied from what
    // this module answers. Each also carries the nearest input the rule does
    // *not* reach, so a rule widened to fit is red too. `break_conformance`
    // prints which of the file's own rule labels have no witness there and
    // names the test here that stands in for each.

    /// LB4 and LB5 are *mandatory*, `!` in UAX #14's notation, and not merely
    /// allowed. The conformance file writes both as `÷`, so this and the
    /// harness's check derived from the property are what hold the difference.
    #[test]
    fn a_hard_line_break_is_mandatory() {
        // LB4: BK !. U+2028 is BK.
        assert_eq!(at([0x61, 0x2028, 0x62], 2), (Rule::LB4, Mandatory));
        // LB5: CR !; LF !; NL !. U+000D is CR, U+000A LF, U+0085 NL.
        assert_eq!(at([0x61, 0x0D, 0x62], 2), (Rule::LB5, Mandatory));
        assert_eq!(at([0x61, 0x0A, 0x62], 2), (Rule::LB5, Mandatory));
        assert_eq!(at([0x61, 0x85, 0x62], 2), (Rule::LB5, Mandatory));
        // LB5: CR × LF, and the mandatory break is after the pair.
        assert_eq!(at([0x61, 0x0D, 0x0A, 0x62], 2), (Rule::LB5, Prohibited));
        assert_eq!(at([0x61, 0x0D, 0x0A, 0x62], 3), (Rule::LB5, Mandatory));
        // LB6: × (BK | CR | LF | NL), the position before one.
        assert_eq!(at([0x61, 0x0A, 0x62], 1), (Rule::LB6, Prohibited));
        // The text path carries the difference: a hard break is mandatory
        // there, and a break after a space (LB18) is not.
        let mut kept = opportunities(&['a', '\n', 'b']).map(|o| (o.at, o.mandatory, o.rule));
        assert_eq!(kept.next(), Some((2, true, Rule::LB5)));
        assert_eq!(kept.next(), Some((3, true, Rule::LB3)));
        assert_eq!(kept.next(), None);
        let mut kept = opportunities(&['a', ' ', 'b']).map(|o| (o.at, o.mandatory, o.rule));
        assert_eq!(kept.next(), Some((2, false, Rule::LB18)));
        assert_eq!(kept.next(), Some((3, true, Rule::LB3)));
    }

    /// LB15a, every alternative of its left context: `(sot | BK | CR | LF |
    /// NL | OP | QU | GL | SP | ZW) [\p{Pi}&QU] SP* ×`. The file reaches `sot`,
    /// BK, OP, SP and ZW, and not CR, LF, NL, QU or GL. U+201C is QU and Pi.
    /// In each text the position after the space is LB15a's, where LB18
    /// (`SP ÷`) would otherwise break; with a letter as the context LB15a does
    /// not apply, and LB18 breaks.
    #[test]
    fn lb15a_holds_after_an_opening_quote_in_every_context() {
        for context in [0x2028, 0x0D, 0x0A, 0x85, 0x28, 0x22, 0xA0, 0x20, 0x200B] {
            let got = at([context, 0x201C, 0x20, 0x61], 3);
            assert_eq!(got, (Rule::LB15a, Prohibited), "U+{context:04X}");
        }
        assert_eq!(at([0x201C, 0x20, 0x61], 2), (Rule::LB15a, Prohibited), "sot");
        assert_eq!(at([0x62, 0x201C, 0x20, 0x61], 3), (Rule::LB18, Allowed));
    }

    /// LB15b, every alternative of what follows the closing quote: `×
    /// [\p{Pf}&QU] (SP | GL | WJ | CL | QU | CP | EX | IS | SY | BK | CR | LF |
    /// NL | ZW | eot)`. The file reaches SP, CL, CP, IS, BK, ZW and `eot`, and
    /// not GL, WJ, QU, EX, SY, CR, LF or NL. U+201D is QU and Pf. In `a`, a
    /// space, U+201D and the follower, the position before the quote is after
    /// a space, so LB15b holds where LB18 would break; with a letter following,
    /// LB15b does not apply and LB18 breaks.
    #[test]
    fn lb15b_holds_before_a_closing_quote_before_every_follower() {
        for follower in [
            0x20, 0xA0, 0x2060, 0x7D, 0x22, 0x29, 0x21, 0x2C, 0x2F, 0x2028, 0x0D, 0x0A, 0x85,
            0x200B,
        ] {
            let got = at([0x61, 0x20, 0x201D, follower], 2);
            assert_eq!(got, (Rule::LB15b, Prohibited), "U+{follower:04X}");
        }
        assert_eq!(at([0x61, 0x20, 0x201D], 2), (Rule::LB15b, Prohibited), "eot");
        assert_eq!(at([0x61, 0x20, 0x201D, 0x62], 2), (Rule::LB18, Allowed));
    }

    /// LB19a's third part, `QU × [^$EastAsian]`, where it alone decides: an
    /// East Asian character before the quote, so the fourth part, `(sot |
    /// [^$EastAsian]) QU ×`, does not apply. U+4E00 is W, U+201D is QU and Pf
    /// (so LB19's `[QU - \p{Pf}] ×` does not hold after it either), `a` is Na.
    /// With an ideograph after the quote too, no part applies and LB31 breaks.
    #[test]
    fn lb19a_holds_after_a_quote_before_a_character_that_is_not_east_asian() {
        assert_eq!(at([0x4E00, 0x201D, 0x61], 2), (Rule::LB19a, Prohibited));
        assert_eq!(at([0x4E00, 0x201D, 0x4E00], 2), (Rule::LB31, Allowed));
    }

    /// LB20a, every alternative of its left context: `(sot | BK | CR | LF | NL
    /// | SP | ZW | CB | GL) (HY | HH) × (AL | HL)`. The file reaches `sot` and
    /// SP, and not BK, CR, LF, NL, ZW, CB or GL. U+002D is HY and U+2010 HH.
    /// After a letter the hyphen is not word-initial, and LB31 breaks after it.
    #[test]
    fn lb20a_holds_after_a_word_initial_hyphen_in_every_context() {
        for context in [0x2028, 0x0D, 0x0A, 0x85, 0x20, 0x200B, 0xFFFC, 0xA0] {
            assert_eq!(at([context, 0x2D, 0x61], 2), (Rule::LB20a, Prohibited), "U+{context:04X}");
            let got = at([context, 0x2010, 0x05D0], 2);
            assert_eq!(got, (Rule::LB20a, Prohibited), "U+{context:04X} HH");
        }
        assert_eq!(at([0x2D, 0x61], 1), (Rule::LB20a, Prohibited), "sot");
        assert_eq!(at([0x62, 0x2D, 0x61], 2), (Rule::LB31, Allowed));
    }

    /// LB21a as 17.0 writes it, `HL (HY | HH) × [^HL]`. U+05D0 is HL, U+2010 is
    /// HH (a class new in 17.0), `a` is AL. The file reaches the HY half only.
    /// With HL after the hyphen the rule does not apply: LB20a's context does
    /// not include HL, and LB31 breaks.
    #[test]
    fn lb21a_holds_after_a_hebrew_hyphen_of_either_class() {
        assert_eq!(at([0x05D0, 0x2010, 0x61], 2), (Rule::LB21a, Prohibited));
        assert_eq!(at([0x05D0, 0x2D, 0x61], 2), (Rule::LB21a, Prohibited));
        assert_eq!(at([0x05D0, 0x2010, 0x05D1], 2), (Rule::LB31, Allowed));
    }

    /// LB25's four parts that look past an opening bracket — `PO × OP NU`,
    /// `PO × OP IS NU`, `PR × OP NU` and `PR × OP IS NU`, the file's 25.07,
    /// 25.08, 25.10 and 25.11, of which it reaches only 25.10. The two with
    /// `IS` are the one place this module reads two units ahead, and the file
    /// holds that lookahead in neither direction. U+0024 is PR, U+0025 PO, `(`
    /// OP, `.` IS, `5` NU. With a letter where the digit belongs no part
    /// applies — LB30 holds `(` only after AL, HL or NU — and LB31 breaks.
    ///
    /// LB9 runs before LB25, so the units LB25's `OP IS NU` reads are LB9's:
    /// `X (CM | ZWJ)*` is X for any X but BK, CR, LF, NL, SP and ZW. With
    /// U+0308 (CM) after the `.`, the text is still PR (or PO), OP, IS, NU to
    /// LB25, and 25.11 (or 25.08) holds position 1; the digit two units ahead
    /// is the scalar after the mark, not the mark. With U+0308 after the `(`
    /// the same holds one unit earlier. And with a letter after the marked
    /// `.`, no part applies and LB31 breaks, as above. A re-audit on
    /// 2026-09-26 found a mutant reading the scalar after the IS's first,
    /// rather than after its unit, green over the whole file.
    #[test]
    fn lb25_looks_past_an_opening_bracket_and_a_decimal_mark() {
        for affix in [0x24, 0x25] {
            assert_eq!(at([affix, 0x28, 0x2E, 0x35], 1), (Rule::LB25, Prohibited), "U+{affix:04X}");
            assert_eq!(at([affix, 0x28, 0x35], 1), (Rule::LB25, Prohibited), "U+{affix:04X}");
            assert_eq!(at([affix, 0x28, 0x2E, 0x61], 1), (Rule::LB31, Allowed), "U+{affix:04X}");
            assert_eq!(at([affix, 0x28, 0x61], 1), (Rule::LB31, Allowed), "U+{affix:04X}");
            let marked = at([affix, 0x28, 0x2E, 0x0308, 0x35], 1);
            assert_eq!(marked, (Rule::LB25, Prohibited), "U+{affix:04X}, a mark on the IS");
            let marked = at([affix, 0x28, 0x0308, 0x2E, 0x35], 1);
            assert_eq!(marked, (Rule::LB25, Prohibited), "U+{affix:04X}, a mark on the OP");
            let marked = at([affix, 0x28, 0x2E, 0x0308, 0x61], 1);
            assert_eq!(marked, (Rule::LB31, Allowed), "U+{affix:04X}, a mark and a letter");
        }
    }

    /// LB25's closed number, `NU (SY | IS)* (CL | CP) × (PO | PR)` — the
    /// file's 25.01 to 25.04, of which it reaches only the two before PR — and
    /// what a closing bracket ends. After it a digit is not the same number:
    /// the last part, `NU (SY | IS)* × NU`, has no bracket in it, and `(PO | PR
    /// | HY | IS) × NU` does not name CL. Nor does a solidus after the bracket
    /// continue it, since `(SY | IS)*` sits before the bracket. U+007D is CL,
    /// `)` CP, `/` SY, `,` IS, `%` PO, `$` PR.
    #[test]
    fn lb25_ends_a_number_at_its_closing_bracket() {
        assert_eq!(at([0x31, 0x7D, 0x25], 2), (Rule::LB25, Prohibited), "25.01");
        assert_eq!(at([0x31, 0x29, 0x25], 2), (Rule::LB25, Prohibited), "25.02");
        assert_eq!(at([0x31, 0x2C, 0x32, 0x29, 0x24], 4), (Rule::LB25, Prohibited), "25.04");
        // 1 } 2: and LB30's `[CP - $EastAsian] ×` is CP's, not CL's.
        assert_eq!(at([0x31, 0x7D, 0x32], 2), (Rule::LB31, Allowed));
        // 1 } / 2.
        assert_eq!(at([0x31, 0x7D, 0x2F, 0x32], 3), (Rule::LB31, Allowed));
    }

    /// LB28a's third part, `(AK | ◌ | AS) VI × (AK | ◌)`: AS is on its left and
    /// not on its right. U+1B05 is AK, U+1B44 VI, U+1B50 AS, U+25CC the dotted
    /// circle. After AK VI, an AK or a dotted circle is held; an AS is not, no
    /// later rule holds it, and LB31 breaks.
    #[test]
    fn lb28a_holds_after_a_virama_before_a_consonant_and_not_before_an_aksara_start() {
        assert_eq!(at([0x1B05, 0x1B44, 0x1B05], 2), (Rule::LB28a, Prohibited));
        assert_eq!(at([0x1B05, 0x1B44, 0x25CC], 2), (Rule::LB28a, Prohibited));
        assert_eq!(at([0x1B05, 0x1B44, 0x1B50], 2), (Rule::LB31, Allowed));
    }

    /// `$EastAsian` is `[\p{ea=F}\p{ea=W}\p{ea=H}]`, and H is not idle: U+FF62,
    /// the halfwidth left corner bracket, is the one OP of width H in 17.0, so
    /// LB30's `(AL | HL | NU) × [OP - $EastAsian]` does not hold it after a
    /// letter and LB31 breaks. Before `(`, of width Na, LB30 holds.
    #[test]
    fn lb30_lets_a_halfwidth_bracket_break_after_a_letter() {
        assert_eq!(at([0x61, 0xFF62], 1), (Rule::LB31, Allowed));
        assert_eq!(at([0x61, 0x28], 1), (Rule::LB30, Prohibited));
    }
}
