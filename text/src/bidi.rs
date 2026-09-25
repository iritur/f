// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Bidirectional reordering: UAX #9, the Unicode Bidirectional Algorithm, from
//! a paragraph's scalars to the order a line of them is displayed in.
//!
//! # The level this conforms at, named rather than implied
//!
//! **Full bidirectionality, through rule L2 inclusively.** UAX #9 section 4.2
//! sorts implementations by how many of the explicit formatting characters they
//! interpret, and this one interprets all nine — the embeddings, the overrides,
//! the isolates and their terminators — which is the class that section calls
//! *full bidirectionality*. *Through rule L2 inclusively* is the conformance
//! corpus's own phrase for what it can verify, in `BidiCharacterTest.txt`'s
//! header, and the rules that reaches are P2–P3, X1–X10, W1–W7, N0–N2, I1–I2,
//! L1 and L2. All of them are here, one function per group, and each function
//! is named after its rules so that a reader holding the specification can put
//! a finger on either and find the other.
//!
//! What that level leaves out, and where each part went:
//!
//! - **P1**, splitting text into paragraphs, is [`paragraph_len`]. The corpus
//!   does not exercise it — every case in both files is one paragraph — so the
//!   evidence for it is a unit test below and not the conformance run, and
//!   [`resolve`] refuses a second paragraph rather than silently resolving two
//!   as one.
//! - **L3**, combining marks after a right-to-left base, is not here. It is
//!   platform- and character-specific in `BidiTest.txt`'s words, both corpus
//!   files say it is out of their scope, and it is a statement about glyphs rather than about scalars: the
//!   renderer that draws a mark knows whether it wants it reordered, and this
//!   crate does not draw.
//! - **L4**, mirrored glyphs, is not here, for the same reason with one more.
//!   It needs `Bidi_Mirrored`, which would be a third generated table under
//!   `DERIVED_DATA` built from `BidiMirroring.txt`, and nothing in this tree
//!   draws a glyph yet to consume one. *What would reverse this:* the first
//!   renderer that draws a `(` inside a right-to-left run. Then it is a `SHAPES`
//!   row and a `DERIVED_DATA` row, never a list typed here.
//!
//! The evidence is `text/tests/bidi_conformance.rs`, which runs both of the
//! specification's corpus files over this module and prints what it ran, passed
//! and excluded on every run. The argued eight — `crate::corpus` — are below in
//! this module's own tests, each expectation derived with its rules named beside
//! it, which is RFC 0115's second home.
//!
//! # Why the caller lends the storage
//!
//! This crate is `no_std` and has no allocator, for [`crate::cache`]'s reason:
//! the heap arrives in `ring/` later, and until it does a buffer is something a
//! caller lends. The algorithm needs state per scalar — the class it arrived
//! with, the class it is resolving to, its level, and two links — and nothing
//! else that grows with the text, so the loan is one [`Slot`] per scalar and the
//! rest is bounded by the specification itself: the directional status stack by
//! BD2's depth of 125, the bracket stack by BD16's 63, both fixed arrays.
//!
//! The one structure that looks as if it needs a heap is an isolating run
//! sequence, which BD13 defines as a list of level runs, and a paragraph has a
//! list of those. It does not: a sequence is threaded through the slots as a
//! linked list — each slot names the next scalar in its sequence — so a
//! sequence can be as long as the paragraph without anybody allocating one, and
//! every rule that UAX #9 states as a backward search (W2, W5, W7, N0's context
//! before a bracket, N1's leading type) is carried forward as a running value
//! instead, which is also what keeps each pass linear.
//!
//! # Determinism
//!
//! A level is a small integer and an order is a permutation, so there is no
//! rounding anywhere in this module and nothing here observes time, randomness
//! or ordering. [`resolve`] is a pure function of its scalars and the direction
//! asked, which is the property RFC 0115's fourth obligation leans on: the same
//! corpus on the arm runner is the same comparison, and a divergence there
//! would be a defect rather than a float.

use crate::bidi_brackets::BracketType;
use crate::bidi_class::BidiClass;
use crate::property::{bidi_class, paired_bracket};

use BidiClass::{
    AL, AN, B, BN, CS, EN, ES, ET, FSI, L, LRE, LRI, LRO, NSM, ON, PDF, PDI, R, RLE, RLI, RLO, S,
    WS,
};

/// The deepest explicit embedding level, BD2. Unit: embedding levels.
///
/// The implicit rules can raise a scalar one or two above this — I1 and I2 act
/// after the explicit ones — so a resolved level is at most `MAX_DEPTH + 1`.
pub const MAX_DEPTH: u8 = 125;

/// Entries the directional status stack can hold: one per level from zero to
/// [`MAX_DEPTH`], plus the paragraph's own entry. Unit: stack entries.
const STATUS_ENTRIES: usize = MAX_DEPTH as usize + 2;

/// The bracket stack's capacity, BD16. Unit: stack entries.
///
/// Sixty-three is the specification's number and not a tuning: an opening
/// bracket that finds the stack full stops bracket pairing for the rest of the
/// isolating run sequence, and `BidiCharacterTest.txt` has cases at 62, 63 and
/// 64 nested pairs that tell a stack of 64 from one of 63.
const BRACKET_ENTRIES: usize = 63;

/// No scalar: the end of a linked sequence, an unmatched isolate, an unpaired
/// bracket. Also why a paragraph is at most `u32::MAX - 1` scalars.
const NONE: u32 = u32::MAX;

/// The direction a paragraph is asked to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseDirection {
    /// Level 0, whatever the text says.
    LeftToRight,
    /// Level 1, whatever the text says.
    RightToLeft,
    /// P2 and P3: the first strong scalar outside any isolate decides, and a
    /// paragraph with none is left to right. The corpus files call this
    /// *auto-LTR*.
    FirstStrong,
}

/// What a paragraph resolved to, as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paragraph {
    /// The paragraph embedding level: 0 or 1. Unit: embedding levels.
    pub level: u8,
}

/// Why [`resolve`], [`line_levels`] or [`visual_order`] declined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The slice lent for the result is not the length the input needs.
    /// Unit: scalars, both.
    WrongLength {
        /// How many were needed.
        need: usize,
        /// How many were lent.
        have: usize,
    },
    /// More scalars than an index here can name. Unit: scalars.
    TooLong {
        /// The length asked about.
        len: usize,
    },
    /// A paragraph separator before the last scalar. P1 ends a paragraph there,
    /// so the text is two paragraphs, and resolving them as one would carry the
    /// first one's embeddings into the second — X8 says they end. Split with
    /// [`paragraph_len`] and resolve each. Unit: the scalar's index.
    SecondParagraph {
        /// Where the separator is.
        at: usize,
    },
}

/// One scalar's state while its paragraph is resolved, and its result after.
///
/// Lent by the caller, one per scalar, for [`crate::cache::Slot`]'s reason. The
/// fields are private because five of them are working state that means
/// nothing between calls; what a caller reads is [`Slot::level`],
/// [`Slot::bidi_class`] and [`Slot::removed`].
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    /// The class the scalar arrived with, which L1, W1's note, N0's marks and
    /// X9 all consult after it has been resolved away.
    original: BidiClass,
    /// The class it is resolving to; the rules rewrite this one.
    class: BidiClass,
    /// Its embedding level. Unit: embedding levels.
    level: u8,
    /// The first scalar of a level run, BD7.
    run_start: bool,
    /// Reached from an isolate initiator in an earlier level run, so it
    /// continues that run's isolating run sequence rather than starting one.
    linked_in: bool,
    /// For an isolate initiator, its matching PDI, and for that PDI, the
    /// initiator (BD9). For an opening bracket, from N0 on, its closing one
    /// (BD16). Nothing else, so the two uses cannot meet: a bracket's class is
    /// ON and an isolate's never is.
    partner: u32,
    /// The next scalar in this scalar's isolating run sequence. While BD9 runs,
    /// the initiator below this one on the matching stack.
    next: u32,
}

impl Slot {
    /// A slot holding nothing, so a caller can write `[Slot::EMPTY; N]`.
    pub const EMPTY: Self = Self {
        original: L,
        class: L,
        level: 0,
        run_start: false,
        linked_in: false,
        partner: NONE,
        next: NONE,
    };

    /// The resolved embedding level, after I2 and before L1. Unit: embedding
    /// levels.
    ///
    /// A scalar X9 removes has no level in the specification; it is given its
    /// predecessor's, or the paragraph's at the start, so that every scalar has
    /// one and reordering never breaks a run where the text had none.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }

    /// The scalar's `Bidi_Class` as it arrived.
    #[must_use]
    pub const fn bidi_class(&self) -> BidiClass {
        self.original
    }

    /// Whether X9 removes it: the embeddings, the overrides, PDF and BN. The
    /// corpus marks these `x`, and nothing in the specification orders them.
    #[must_use]
    pub const fn removed(&self) -> bool {
        removed_by_x9(self.original)
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// X9's list.
const fn removed_by_x9(class: BidiClass) -> bool {
    matches!(class, RLE | LRE | RLO | LRO | PDF | BN)
}

const fn isolate_initiator(class: BidiClass) -> bool {
    matches!(class, LRI | RLI | FSI)
}

/// The strong direction of a level: L for even, R for odd.
const fn direction_of(level: u8) -> BidiClass {
    if level.is_multiple_of(2) { L } else { R }
}

/// The length of the first paragraph in `text`, its separator included: P1.
///
/// A paragraph separator stays with the paragraph it ends, so this is one past
/// the first `B`, or the whole text when there is none.
#[must_use]
pub fn paragraph_len(text: &[char]) -> usize {
    text.iter().position(|&c| bidi_class(c) == B).map_or(text.len(), |at| at + 1)
}

/// Resolve one paragraph's embedding levels: P2 and P3 when asked, then X1
/// through I2.
///
/// `work` must be exactly as long as `text`; each slot's result is then read
/// through [`Slot::level`]. The line rules are separate — [`line_levels`] and
/// [`visual_order`] — because which scalars are on a line is the breaker's to
/// decide, and L1 reads the line rather than the paragraph.
///
/// # Errors
///
/// [`Refusal::WrongLength`] if `work` is not as long as `text`,
/// [`Refusal::TooLong`] past `u32::MAX - 1` scalars, and
/// [`Refusal::SecondParagraph`] for a paragraph separator anywhere but last.
pub fn resolve(
    text: &[char],
    base: BaseDirection,
    work: &mut [Slot],
) -> Result<Paragraph, Refusal> {
    let n = text.len();
    if n >= NONE as usize {
        return Err(Refusal::TooLong { len: n });
    }
    if work.len() != n {
        return Err(Refusal::WrongLength { need: n, have: work.len() });
    }
    for (i, (&c, slot)) in text.iter().zip(work.iter_mut()).enumerate() {
        let class = bidi_class(c);
        if class == B && i + 1 != n {
            return Err(Refusal::SecondParagraph { at: i });
        }
        *slot = Slot { original: class, class, ..Slot::EMPTY };
    }

    match_isolates(work);
    let level = match base {
        BaseDirection::LeftToRight => 0,
        BaseDirection::RightToLeft => 1,
        BaseDirection::FirstStrong => first_strong(work, 0, n).unwrap_or(0),
    };
    explicit_levels(work, level);
    link_sequences(work);

    let mut i = 0;
    while i < n {
        let slot = work[i];
        if !slot.removed() && slot.run_start && !slot.linked_in {
            resolve_sequence(text, work, i, level);
        }
        i += 1;
    }
    implicit_levels(work);
    level_removed(work, level);
    Ok(Paragraph { level })
}

/// BD9: pair every isolate initiator with the PDI that closes it.
///
/// A stack threaded through the initiators' own `next` fields rather than an
/// array, because nesting has no bound here — isolates past the depth limit
/// still match textually — and a lent slot per scalar is the only storage there
/// is. Each field is cleared as its initiator leaves the stack.
fn match_isolates(work: &mut [Slot]) {
    let mut top = NONE;
    let mut i = 0;
    while i < work.len() {
        match work[i].original {
            LRI | RLI | FSI => {
                work[i].next = top;
                top = i as u32;
            }
            PDI if top != NONE => {
                let opener = top as usize;
                top = work[opener].next;
                work[opener].next = NONE;
                work[opener].partner = i as u32;
                work[i].partner = opener as u32;
            }
            _ => {}
        }
        i += 1;
    }
    while top != NONE {
        let opener = top as usize;
        top = work[opener].next;
        work[opener].next = NONE;
    }
}

/// P2 and P3 over `from..to`: the level the first strong scalar asks for,
/// skipping every isolate, or `None` when there is no strong scalar.
///
/// An isolate is skipped to its matching PDI; one with no PDI runs to the end
/// of the paragraph, so nothing after it counts. X5c uses the same search for an
/// FSI, over the scalars it encloses.
fn first_strong(work: &[Slot], from: usize, to: usize) -> Option<u8> {
    let mut i = from;
    while i < to {
        match work[i].original {
            L => return Some(0),
            R | AL => return Some(1),
            LRI | RLI | FSI => {
                if work[i].partner == NONE {
                    return None;
                }
                i = work[i].partner as usize;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// One entry of the directional status stack, X1.
#[derive(Clone, Copy)]
struct Status {
    /// Unit: embedding levels.
    level: u8,
    /// L or R under an override, or `None` when neutral.
    overriding: Option<BidiClass>,
    /// Pushed by an isolate initiator rather than an embedding or override.
    isolate: bool,
}

/// X1 through X8: every explicit formatting character applied, in one pass.
///
/// The three counters are the specification's own — overflow isolates,
/// overflow embeddings, valid isolates — and the branch order below is X5a
/// through X7 in the order the text states them, because the order is where the
/// subtle cases live: an embedding that overflows inside an overflowed isolate
/// is counted by neither, which is what makes a later PDF inert.
fn explicit_levels(work: &mut [Slot], paragraph: u8) {
    let n = work.len();
    let mut stack = [Status { level: paragraph, overriding: None, isolate: false }; STATUS_ENTRIES];
    let mut depth = 1usize;
    let mut overflow_isolates = 0usize;
    let mut overflow_embeddings = 0usize;
    let mut valid_isolates = 0usize;

    let mut i = 0;
    while i < n {
        let top = stack[depth - 1];
        let class = work[i].original;
        match class {
            RLE | LRE | RLO | LRO | RLI | LRI | FSI => {
                let isolate = isolate_initiator(class);
                // X5c: an FSI is an RLI or an LRI by P2 and P3 over what it
                // encloses.
                let rtl = match class {
                    RLE | RLO | RLI => true,
                    FSI => {
                        let end = match work[i].partner {
                            NONE => n,
                            pdi => pdi as usize,
                        };
                        first_strong(work, i + 1, end) == Some(1)
                    }
                    _ => false,
                };
                if isolate {
                    // X5a, X5b: the initiator sits at the level outside it, and
                    // takes the override it is inside.
                    work[i].level = top.level;
                    if let Some(overriding) = top.overriding {
                        work[i].class = overriding;
                    }
                }
                let level = if rtl { (top.level + 1) | 1 } else { (top.level + 2) & !1 };
                if level <= MAX_DEPTH && overflow_isolates == 0 && overflow_embeddings == 0 {
                    if isolate {
                        valid_isolates += 1;
                    }
                    let overriding = match class {
                        LRO => Some(L),
                        RLO => Some(R),
                        _ => None,
                    };
                    stack[depth] = Status { level, overriding, isolate };
                    depth += 1;
                } else if isolate {
                    overflow_isolates += 1;
                } else if overflow_isolates == 0 {
                    overflow_embeddings += 1;
                }
            }
            PDI => {
                // X6a.
                if overflow_isolates > 0 {
                    overflow_isolates -= 1;
                } else if valid_isolates > 0 {
                    overflow_embeddings = 0;
                    while !stack[depth - 1].isolate {
                        depth -= 1;
                    }
                    depth -= 1;
                    valid_isolates -= 1;
                }
                let top = stack[depth - 1];
                work[i].level = top.level;
                if let Some(overriding) = top.overriding {
                    work[i].class = overriding;
                }
            }
            PDF => {
                // X7. Removed by X9, so it takes no level here.
                if overflow_isolates == 0 {
                    if overflow_embeddings > 0 {
                        overflow_embeddings -= 1;
                    } else if !top.isolate && depth >= 2 {
                        depth -= 1;
                    }
                }
            }
            // X8: only ever last, which `resolve` holds.
            B => work[i].level = paragraph,
            // X9 removes it; `level_removed` gives it a level afterwards.
            BN => {}
            // X6.
            _ => {
                work[i].level = top.level;
                if let Some(overriding) = top.overriding {
                    work[i].class = overriding;
                }
            }
        }
        i += 1;
    }
}

/// X9 and X10's BD13: thread every isolating run sequence through the slots.
///
/// X9's removed scalars are stepped over, so they break no level run. Within a
/// level run each scalar links to the next; a level run that ends in an isolate
/// initiator with a matching PDI links to that PDI, whose own run then
/// continues the sequence; and a scalar reached that way is marked, so that
/// [`resolve`] starts a sequence only at a run nobody links into.
fn link_sequences(work: &mut [Slot]) {
    let mut previous = NONE;
    let mut i = 0;
    while i < work.len() {
        if !work[i].removed() {
            if previous == NONE {
                work[i].run_start = true;
            } else {
                let p = previous as usize;
                if work[p].level == work[i].level {
                    work[p].next = i as u32;
                } else {
                    work[i].run_start = true;
                    if isolate_initiator(work[p].original) && work[p].partner != NONE {
                        let pdi = work[p].partner;
                        work[p].next = pdi;
                        work[pdi as usize].linked_in = true;
                    }
                }
            }
            previous = i as u32;
        }
        i += 1;
    }
}

/// The scalar after `i` in its sequence.
fn after(work: &[Slot], i: usize) -> Option<usize> {
    match work[i].next {
        NONE => None,
        next => Some(next as usize),
    }
}

/// Set every scalar from `from` up to but not including `to` — or to the
/// sequence's end, for `None` — to `class`.
fn set_span(work: &mut [Slot], from: usize, to: Option<usize>, class: BidiClass) {
    let mut at = Some(from);
    while let Some(i) = at {
        if Some(i) == to {
            return;
        }
        work[i].class = class;
        at = after(work, i);
    }
}

/// Everything X10 says to do to one isolating run sequence: W1–W7, N0–N2 and
/// I1–I2, in that order, each a pass of its own so that a reader can hold each
/// against its rule.
fn resolve_sequence(text: &[char], work: &mut [Slot], start: usize, paragraph: u8) {
    let level = work[start].level;

    // X10: sos from the level before the sequence, eos from the one after —
    // each the scalar's neighbour in the paragraph, not in the sequence, and
    // X9's removed scalars not counted.
    let before = work[..start].iter().rev().find(|s| !s.removed()).map_or(paragraph, |s| s.level);
    let sos = direction_of(before.max(level));
    let mut last = start;
    while let Some(next) = after(work, last) {
        last = next;
    }
    let following = if isolate_initiator(work[last].original) {
        // An initiator that ends a sequence has no matching PDI, and X10 says
        // the paragraph is what follows it.
        paragraph
    } else {
        work[last + 1..].iter().find(|s| !s.removed()).map_or(paragraph, |s| s.level)
    };
    let eos = direction_of(following.max(level));

    weak_types(work, start, sos, eos);
    bracket_pairs(text, work, start, sos, direction_of(level));
    neutral_types(work, start, sos, eos, direction_of(level));
}

/// W1 through W7.
fn weak_types(work: &mut [Slot], start: usize, sos: BidiClass, eos: BidiClass) {
    // W1: a mark takes the class before it, or ON after an isolate initiator
    // or a PDI, or sos at the start.
    let mut previous = sos;
    let mut at = Some(start);
    while let Some(i) = at {
        let class = work[i].class;
        if class == NSM {
            work[i].class = previous;
        } else {
            previous = if matches!(class, LRI | RLI | FSI | PDI) { ON } else { class };
        }
        at = after(work, i);
    }

    // W2: a European number after Arabic letters, back to the last strong
    // class or sos, is an Arabic number.
    let mut strong = sos;
    let mut at = Some(start);
    while let Some(i) = at {
        match work[i].class {
            L | R | AL => strong = work[i].class,
            EN if strong == AL => work[i].class = AN,
            _ => {}
        }
        at = after(work, i);
    }

    // W3: Arabic letters are R from here on.
    let mut at = Some(start);
    while let Some(i) = at {
        if work[i].class == AL {
            work[i].class = R;
        }
        at = after(work, i);
    }

    // W4: one separator between two numbers of one kind joins them — ES only
    // between European numbers, CS between either kind.
    let mut previous: Option<BidiClass> = None;
    let mut at = Some(start);
    while let Some(i) = at {
        let next = after(work, i);
        let class = work[i].class;
        if let (Some(before), Some(n)) = (previous, next)
            && matches!(class, ES | CS)
        {
            let following = work[n].class;
            if before == EN && following == EN {
                work[i].class = EN;
            } else if class == CS && before == AN && following == AN {
                work[i].class = AN;
            }
        }
        previous = Some(work[i].class);
        at = next;
    }

    // W5: a run of terminators touching a European number is European numbers.
    // sos and eos are L or R, so only a neighbour inside the sequence can be
    // the number.
    let mut run: Option<usize> = None;
    let mut touching = false;
    let mut previous = sos;
    let mut at = Some(start);
    while let Some(i) = at {
        let class = work[i].class;
        if class == ET {
            if run.is_none() {
                run = Some(i);
                touching = previous == EN;
            }
        } else if let Some(first) = run.take()
            && (touching || class == EN)
        {
            set_span(work, first, Some(i), EN);
        }
        previous = work[i].class;
        at = after(work, i);
    }
    if let Some(first) = run
        && (touching || eos == EN)
    {
        set_span(work, first, None, EN);
    }

    // W6: separators and terminators left over are neutral.
    let mut at = Some(start);
    while let Some(i) = at {
        if matches!(work[i].class, ES | ET | CS) {
            work[i].class = ON;
        }
        at = after(work, i);
    }

    // W7: a European number after L, back to the last strong class or sos,
    // is L.
    let mut strong = sos;
    let mut at = Some(start);
    while let Some(i) = at {
        match work[i].class {
            EN if strong == L => work[i].class = L,
            L | R => strong = work[i].class,
            _ => {}
        }
        at = after(work, i);
    }
}

/// N0's view of a class: L, or R for R and both kinds of number, or `None`.
const fn strong_for_brackets(class: BidiClass) -> Option<BidiClass> {
    match class {
        L => Some(L),
        R | AL | EN | AN => Some(R),
        _ => None,
    }
}

/// The bracket two canonically equivalent brackets agree on.
///
/// BD16 matches a closing bracket against an opening one's pair *or anything
/// canonically equivalent to it*, and among the brackets there are exactly two
/// such: U+2329 and U+232A decompose to U+3008 and U+3009. Written here rather
/// than read from a table because this crate has no decomposition data and two
/// values do not justify a third generated file; `BidiCharacterTest.txt`'s
/// section on brackets with canonical equivalents is what checks them. *What
/// would reverse this:* a Unicode version adding a third, which that section
/// would find, or a normalisation table arriving for any other reason.
const fn canonical_bracket(c: char) -> char {
    match c {
        '\u{2329}' => '\u{3008}',
        '\u{232A}' => '\u{3009}',
        other => other,
    }
}

/// N0: paired brackets resolved as units, BD14 to BD16 to find them.
fn bracket_pairs(
    text: &[char],
    work: &mut [Slot],
    start: usize,
    sos: BidiClass,
    embedding: BidiClass,
) {
    // BD16: find the pairs. Only a scalar whose class is ON *now* is a bracket
    // — one under an override is L or R and pairs with nothing. Each opening
    // bracket's partner is set to its closing one.
    let mut stack = [('\0', 0usize); BRACKET_ENTRIES];
    let mut depth = 0usize;
    let mut any = false;
    let mut at = Some(start);
    'pairing: while let Some(i) = at {
        if work[i].class == ON
            && let Some((pair, kind)) = paired_bracket(text[i])
        {
            match kind {
                BracketType::Open => {
                    if depth == BRACKET_ENTRIES {
                        // BD16: a full stack stops pairing for the rest of the
                        // sequence, and the pairs already found stand.
                        break 'pairing;
                    }
                    stack[depth] = (canonical_bracket(pair), i);
                    depth += 1;
                }
                BracketType::Close => {
                    let closing = canonical_bracket(text[i]);
                    let mut k = depth;
                    while k > 0 {
                        k -= 1;
                        if stack[k].0 == closing {
                            work[stack[k].1].partner = i as u32;
                            depth = k;
                            any = true;
                            break;
                        }
                    }
                }
            }
        }
        at = after(work, i);
    }
    if !any {
        return;
    }

    // N0 proper, in the order of the opening brackets, which a forward walk
    // is. `preceding` is the strong class before the walk's position — sos to
    // begin with — and a pair resolved earlier has already rewritten what the
    // walk passes, which is the specification's *taking into account
    // resolutions of earlier bracket pairs*.
    let opposite = if embedding == L { R } else { L };
    let mut preceding = sos;
    let mut at = Some(start);
    while let Some(i) = at {
        if work[i].original == ON && work[i].partner != NONE {
            let close = work[i].partner as usize;
            let mut inside_embedding = false;
            let mut inside_opposite = false;
            let mut j = after(work, i);
            while let Some(k) = j
                && k != close
            {
                match strong_for_brackets(work[k].class) {
                    Some(d) if d == embedding => {
                        inside_embedding = true;
                        break;
                    }
                    Some(_) => inside_opposite = true,
                    None => {}
                }
                j = after(work, k);
            }
            // N0 b, c.1, c.2; and d, no strong class inside, leaves the pair
            // for N1.
            let resolved = if inside_embedding {
                Some(embedding)
            } else if inside_opposite {
                Some(if preceding == opposite { opposite } else { embedding })
            } else {
                None
            };
            if let Some(class) = resolved {
                for bracket in [i, close] {
                    work[bracket].class = class;
                    // The marks that followed a bracket before W1 made them
                    // ON follow it into its new class.
                    let mut mark = after(work, bracket);
                    while let Some(m) = mark
                        && work[m].original == NSM
                    {
                        work[m].class = class;
                        mark = after(work, m);
                    }
                }
            }
        }
        if let Some(strong) = strong_for_brackets(work[i].class) {
            preceding = strong;
        }
        at = after(work, i);
    }
}

/// N1 and N2: a run of neutrals and isolate formatting characters takes the
/// direction on both sides of it when they agree, and the embedding direction
/// when they do not. Numbers count as R on either side.
fn neutral_types(
    work: &mut [Slot],
    start: usize,
    sos: BidiClass,
    eos: BidiClass,
    embedding: BidiClass,
) {
    let mut leading = sos;
    let mut run: Option<usize> = None;
    let mut at = Some(start);
    while let Some(i) = at {
        let class = work[i].class;
        if matches!(class, B | S | WS | ON | LRI | RLI | FSI | PDI) {
            if run.is_none() {
                run = Some(i);
            }
        } else {
            let here = if matches!(class, EN | AN) { R } else { class };
            if let Some(first) = run.take() {
                set_span(work, first, Some(i), if leading == here { here } else { embedding });
            }
            leading = here;
        }
        at = after(work, i);
    }
    if let Some(first) = run {
        set_span(work, first, None, if leading == eos { eos } else { embedding });
    }
}

/// I1 and I2, over the whole paragraph once every sequence's classes are
/// resolved.
///
/// Not per sequence, although X10 lists them there, because a later sequence's
/// sos and eos are read from its neighbours' levels — the *explicit* levels X1
/// to X8 gave them — and raising a neighbour first would hand the next sequence
/// an implicit level instead. The two files hold 148 cases between them that
/// tell the two orders apart; the first this module failed was an Arabic number
/// before an RLE, whose raised level made the embedding's sos L.
fn implicit_levels(work: &mut [Slot]) {
    for slot in work.iter_mut().filter(|slot| !slot.removed()) {
        if slot.level.is_multiple_of(2) {
            match slot.class {
                R => slot.level += 1,
                AN | EN => slot.level += 2,
                _ => {}
            }
        } else if matches!(slot.class, L | EN | AN) {
            slot.level += 1;
        }
    }
}

/// A level for every scalar X9 removed: its predecessor's, or the paragraph's
/// at the start. The specification assigns none; one is given so that every
/// scalar can be placed, and the predecessor's is the choice that introduces no
/// level boundary the text did not already have.
fn level_removed(work: &mut [Slot], paragraph: u8) {
    let mut previous = paragraph;
    for slot in work.iter_mut() {
        if slot.removed() {
            slot.level = previous;
        }
        previous = slot.level;
    }
}

/// L1: the levels of one line, from its paragraph's resolved slots.
///
/// `line` is the slots of the scalars on the line, in logical order, and `out`
/// receives one level each. Segment and paragraph separators go to the
/// paragraph level, and so does any run of whitespace and isolate formatting
/// characters before one of them or at the end of the line. A scalar X9
/// removed is transparent to that run, since for the algorithm it is not there.
///
/// # Errors
///
/// [`Refusal::WrongLength`] if `out` is not as long as `line`.
pub fn line_levels(line: &[Slot], paragraph: Paragraph, out: &mut [u8]) -> Result<(), Refusal> {
    if out.len() != line.len() {
        return Err(Refusal::WrongLength { need: line.len(), have: out.len() });
    }
    let mut trailing = true;
    for (slot, level) in line.iter().zip(out.iter_mut()).rev() {
        *level = slot.level;
        match slot.original {
            S | B => {
                *level = paragraph.level;
                trailing = true;
            }
            WS | LRI | RLI | FSI | PDI | RLE | LRE | RLO | LRO | PDF | BN => {
                if trailing {
                    *level = paragraph.level;
                }
            }
            _ => trailing = false,
        }
    }
    Ok(())
}

/// L2: the visual order of a line, left to right, from its levels.
///
/// `order[k]` receives the index into `levels` of the scalar displayed `k`-th
/// from the left. From the highest level down to the lowest odd one, every
/// maximal run at that level or above is reversed.
///
/// # Errors
///
/// [`Refusal::WrongLength`] if `order` is not as long as `levels`, and
/// [`Refusal::TooLong`] past `u32::MAX - 1` scalars.
pub fn visual_order(levels: &[u8], order: &mut [u32]) -> Result<(), Refusal> {
    let n = levels.len();
    if n >= NONE as usize {
        return Err(Refusal::TooLong { len: n });
    }
    if order.len() != n {
        return Err(Refusal::WrongLength { need: n, have: order.len() });
    }
    for (k, place) in order.iter_mut().enumerate() {
        *place = k as u32;
    }
    let (Some(&highest), Some(&lowest)) = (levels.iter().max(), levels.iter().min()) else {
        return Ok(());
    };
    let mut level = highest;
    while level >= (lowest | 1) {
        let mut k = 0;
        while k < n {
            if levels[order[k] as usize] >= level {
                let from = k;
                while k < n && levels[order[k] as usize] >= level {
                    k += 1;
                }
                order[from..k].reverse();
            } else {
                k += 1;
            }
        }
        level -= 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BaseDirection, MAX_DEPTH, Paragraph, Refusal, Slot, line_levels, paragraph_len, resolve,
        visual_order,
    };
    use crate::corpus::Script;

    /// The largest sample in the corpus, in scalars, with room. Unit: scalars.
    const ROOM: usize = 64;

    /// Resolve `text` and lay it out as one line: the paragraph, the L1 levels
    /// and the L2 order, in fixed arrays because this crate has no allocator.
    fn one_line(
        text: &[char],
        base: BaseDirection,
    ) -> (Paragraph, [u8; ROOM], [u32; ROOM], [Slot; ROOM]) {
        let n = text.len();
        let mut work = [Slot::EMPTY; ROOM];
        let paragraph = resolve(text, base, &mut work[..n]).expect("resolves");
        let mut levels = [0u8; ROOM];
        line_levels(&work[..n], paragraph, &mut levels[..n]).expect("levels");
        let mut order = [0u32; ROOM];
        visual_order(&levels[..n], &mut order[..n]).expect("order");
        (paragraph, levels, order, work)
    }

    fn scalars(sample: &str) -> ([char; ROOM], usize) {
        let mut out = ['\0'; ROOM];
        let mut n = 0;
        for c in sample.chars() {
            out[n] = c;
            n += 1;
        }
        (out, n)
    }

    /// What this pass expects of one corpus entry, and how it was derived.
    struct Argued {
        /// The paragraph level, which P2 and P3 decide from the sample.
        paragraph: u8,
        /// One level per scalar, after L1.
        levels: &'static [u8],
        /// The L2 order, as indices into the sample.
        order: &'static [u32],
    }

    /// The argued eight: RFC 0115's second home.
    ///
    /// Every expectation is under [`BaseDirection::FirstStrong`] — the direction
    /// a sample asks for itself — and each arm names the rules that produce it,
    /// so a reader checks the derivation rather than trusting the numbers. The
    /// match is exhaustive over [`Script`] on purpose: an entry added to the
    /// corpus with no expectation here does not compile, because a reordering
    /// pass silently untested on a new script reads exactly like one that
    /// passes on it. That is the price RFC 0115 names — an expectation about one
    /// sample in two places — paid where the compiler collects it.
    fn argued(script: Script) -> Argued {
        match script {
            // P2/P3: `T` is L, so the paragraph is 0. Every letter is L at 0
            // (I1 leaves L alone at an even level); each space and the final
            // `.` sit between L and L or L and eos, which is L at level 0, so
            // N1 makes them L. One run at 0, and L2 reverses nothing.
            Script::Latin => Argued {
                paragraph: 0,
                levels: &[0; 44],
                order: &[
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
                    42, 43,
                ],
            },
            // P2/P3: U+05E9 is R, so the paragraph is 1. The letters are R at
            // an odd level, which I2 leaves at 1; the space is between R and R
            // and N1 makes it R. One run at 1, which L2 reverses whole.
            Script::Hebrew => {
                Argued { paragraph: 1, levels: &[1; 9], order: &[8, 7, 6, 5, 4, 3, 2, 1, 0] }
            }
            // P2/P3: U+0645 is AL, so the paragraph is 1. W3 makes every AL an
            // R; from there it is Hebrew's derivation — N1 for the space, I2
            // leaves R at 1, L2 reverses the one run. Joining changes nothing
            // here, which is the point of having Hebrew beside it.
            Script::Arabic => Argued {
                paragraph: 1,
                levels: &[1; 13],
                order: &[12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
            },
            // P2/P3: U+0928 is L, so 0. U+094D, U+0947 and U+0941 are NSM and
            // W1 gives each the class before it, which is L; U+093F is a
            // spacing mark and L already. N1 for the space. Reordering the
            // vowel sign is the shaper's, not this pass's: storage order is
            // display order at the level of scalars, and L2 has nothing to do.
            Script::Devanagari => Argued {
                paragraph: 0,
                levels: &[0; 13],
                order: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            },
            // P2/P3: U+0E2A is L, so 0. U+0E31 and U+0E35 are NSM and W1 makes
            // them L. No neutral at all, so N1 has nothing to do either.
            Script::Thai => Argued {
                paragraph: 0,
                levels: &[0; 12],
                order: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            },
            // P2/P3: every ideograph is L, so 0, and I1 leaves them there.
            Script::Han => Argued { paragraph: 0, levels: &[0; 4], order: &[0, 1, 2, 3] },
            // P2/P3: the syllables and the jamo are all L, so 0; N1 for the
            // space. Composition is invisible to this pass, which sees nine
            // scalars and nine levels whichever way the syllable was written.
            Script::Hangul => {
                Argued { paragraph: 0, levels: &[0; 9], order: &[0, 1, 2, 3, 4, 5, 6, 7, 8] }
            }
            // P2/P3: U+0627 is AL, so the paragraph is 1 — the first strong
            // scalar decides, and it is Arabic. W3: the letters are R, at 1 by
            // I2. W2: `4` and `2` are EN with AL the last strong class before
            // them, so they are AN, and I2 raises AN at an odd level to 2. The
            // space after the word is between R and AN, which N1 counts as R,
            // so R at 1. The space before `F` is between AN, counted as R, and
            // L: they disagree, so N2 gives it the embedding direction, R, at
            // 1. `F` is L at an odd level and I2 raises it to 2.
            //
            // L2: reverse the runs at 2 — `42` and `F` — then everything at 1
            // or above, which is the whole line. So `F` is leftmost, then the
            // space, then `42` reading left to right, then the space and the
            // word, right to left. The corpus entry's own words — the Latin
            // *back at level 0* — describe a left-to-right paragraph, which is
            // [`mixed_in_a_left_to_right_paragraph`]; in the paragraph the
            // sample asks for itself, the Latin is at 2.
            Script::Mixed => Argued {
                paragraph: 1,
                levels: &[1, 1, 1, 1, 1, 1, 2, 2, 1, 2],
                order: &[9, 8, 6, 7, 5, 4, 3, 2, 1, 0],
            },
        }
    }

    #[test]
    fn every_corpus_entry_resolves_as_argued() {
        for script in Script::ALL {
            let (text, n) = scalars(script.sample());
            let want = argued(script);
            assert_eq!(want.levels.len(), n, "{}: one expected level per scalar", script.name());
            assert_eq!(want.order.len(), n, "{}: one expected place per scalar", script.name());
            let (paragraph, levels, order, _) = one_line(&text[..n], BaseDirection::FirstStrong);
            assert_eq!(paragraph.level, want.paragraph, "{}: paragraph level", script.name());
            assert_eq!(&levels[..n], want.levels, "{}: levels", script.name());
            assert_eq!(&order[..n], want.order, "{}: order", script.name());
        }
    }

    /// The corpus entry's sentence, *the Arabic at level 1, the digits at level
    /// 2 … and the Latin back at level 0*, is true of a left-to-right
    /// paragraph. X1: the paragraph is 0. W3 then I1: the Arabic is R at an
    /// even level, raised to 1. W2 then I1: `42` is AN, raised by two to 2. N1:
    /// the first space is between R and AN-as-R, so R at 1. N2: the second is
    /// between AN-as-R and L, which disagree, so it takes the embedding
    /// direction, L, at 0 — and so does `F`, by I1. L2 reverses `42` at 2, then
    /// the run at 1 or above, which carries `42` with it still reading left to
    /// right.
    #[test]
    fn mixed_in_a_left_to_right_paragraph() {
        let (text, n) = scalars(Script::Mixed.sample());
        let (paragraph, levels, order, _) = one_line(&text[..n], BaseDirection::LeftToRight);
        assert_eq!(paragraph.level, 0);
        assert_eq!(&levels[..n], &[1, 1, 1, 1, 1, 1, 2, 2, 0, 0]);
        assert_eq!(&order[..n], &[6, 7, 5, 4, 3, 2, 1, 0, 8, 9]);
    }

    /// P1, which the corpus cannot reach: both files hold one paragraph per case.
    #[test]
    fn a_paragraph_separator_ends_a_paragraph_and_stays_with_it() {
        let text = ['a', '\u{2029}', '\u{05D0}'];
        assert_eq!(paragraph_len(&text), 2);
        assert_eq!(paragraph_len(&text[2..]), 1);
        assert_eq!(paragraph_len(&[]), 0);
        let mut work = [Slot::EMPTY; 3];
        assert_eq!(
            resolve(&text, BaseDirection::FirstStrong, &mut work),
            Err(Refusal::SecondParagraph { at: 1 }),
            "two paragraphs resolved as one would carry X8's embeddings across"
        );
        let first = resolve(&text[..2], BaseDirection::FirstStrong, &mut work[..2]).unwrap();
        let second = resolve(&text[2..], BaseDirection::FirstStrong, &mut work[2..]).unwrap();
        assert_eq!((first.level, second.level), (0, 1));
    }

    #[test]
    fn a_loan_of_the_wrong_length_is_refused() {
        let mut work = [Slot::EMPTY; 2];
        assert_eq!(
            resolve(&['a'], BaseDirection::LeftToRight, &mut work),
            Err(Refusal::WrongLength { need: 1, have: 2 })
        );
        let mut out = [0u8; 1];
        assert_eq!(
            line_levels(&work, Paragraph { level: 0 }, &mut out),
            Err(Refusal::WrongLength { need: 2, have: 1 })
        );
        let mut order = [0u32; 3];
        assert_eq!(
            visual_order(&[0, 1], &mut order),
            Err(Refusal::WrongLength { need: 2, have: 3 })
        );
        assert_eq!(resolve(&[], BaseDirection::FirstStrong, &mut []), Ok(Paragraph { level: 0 }));
    }

    /// BD2's bound, pinned exactly. `BidiTest.txt` does overflow with explicit
    /// embeddings — its runs of LRE at lines 497561, 497567, 497579 and 497585
    /// are the twelve cases that go red when the bound is off by one — so the
    /// corpus is the evidence for the rule; this test is the one that names the
    /// number. RLE and LRE alternate, so each valid push raises the level by one:
    /// 125 pushes succeed, the last four overflow, and the letter sits at 125,
    /// odd, where I2 raises an L to 126. An assertion of *at least 124* passed
    /// under the off-by-one it exists to catch, which an audit showed on
    /// 2026-09-25.
    #[test]
    fn no_level_passes_the_depth_limit() {
        let mut text = ['\u{202B}'; 130];
        for (i, c) in text.iter_mut().enumerate() {
            if i % 2 == 1 {
                *c = '\u{202A}';
            }
        }
        text[129] = 'a';
        let mut work = [Slot::EMPTY; 130];
        resolve(&text, BaseDirection::LeftToRight, &mut work).unwrap();
        assert!(work.iter().all(|s| s.level() <= MAX_DEPTH + 1));
        assert_eq!(
            work[129].level(),
            MAX_DEPTH + 1,
            "125 pushes, then I2 raises the letter by one"
        );
    }
}
