// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A paragraph set in lines: the text path's three written stages composed,
//! and the shaper's seat left empty on purpose.
//!
//! `E3-B03k` asks for *a rendered paragraph*, and a paragraph is the first
//! thing in this tree that needs [`crate::bidi`], [`crate::line`] and
//! [`crate::metric`] **at once**: which scalars share a line is the breaker's
//! decision, how wide a line is is the pen's, and the order a line is shown in
//! is UAX #9's rule L2 over that line and not over the paragraph. Each of the
//! three was accepted against its own corpus; what was never written is the
//! order they run in, and that order is this module.
//!
//! # The order, and why it is this one
//!
//! 1. **Paragraphs first**, by P1 ([`bidi::paragraph_len`], which counts a CR
//!    LF as one separator). A paragraph separator ends a paragraph *and* a
//!    line — UAX #9's line rules read a line of one paragraph — and
//!    [`bidi::resolve`] refuses a second paragraph rather than carrying one's
//!    embeddings into the next, so the split is not optional.
//! 2. **Levels over the whole paragraph**, because X1–I2 read the paragraph.
//! 3. **Lines, greedily, at [`line::opportunities`] only, asked once over the
//!    whole text.** A line ends where that iterator offers and nowhere else,
//!    which is what carries RFC 0139's *a line breaks between clusters* into a
//!    set paragraph without restating it: this module never computes a break
//!    position of its own. *Over the whole text* and not per paragraph,
//!    because LB3 breaks at the end of whatever it is handed: asked about one
//!    paragraph, it offers that paragraph's end whether or not the text has an
//!    opportunity there, and a line would end on the slice's say-so rather
//!    than the text's.
//! 4. **L1 and L2 per line** ([`bidi::line_levels`], [`bidi::visual_order`]),
//!    because trailing whitespace goes to the paragraph level *at the end of a
//!    line*, and where the line ends was only known at step 3.
//!
//! Measuring happens in logical order and reordering after, which is UAX #9's
//! own advice (section 3.4) and the only order in which a line's width does
//! not depend on the reordering it is about to receive.
//!
//! # The shaper's seat
//!
//! RFC 0082 put the shaper behind the licence boundary and it has not landed,
//! so nothing in this tree can say how wide a run of scalars is in a real face.
//! This module does not pretend to: [`Setting::set`] takes the answer as a
//! parameter — *how many design units does this run advance the pen* — and
//! calls it once per segment between two opportunities, in logical order, with
//! the segment's trailing spaces asked about separately. Per segment rather
//! than per scalar because that is the unit a shaper answers in: a ligature or
//! a kerning pair lives inside a word and never across a break opportunity, so
//! a per-scalar question would be one no shaper can answer honestly.
//!
//! [`crate::cache`] warns against inventing a shaped-run type before a shaper
//! exists, because it would be this tree asserting what its first shaper
//! produces. An [`Advance`] per segment is not that type: it is the one number
//! any shaper must be able to give about a run it has shaped, and it is already
//! the type [`crate::metric`] accumulates. *What would reverse this:* a shaper
//! whose width for a line is not the sum of its segments' widths — kerning
//! across a space, which no mainstream face does and which the day it matters
//! makes the seat a per-line question instead.
//!
//! # Trailing spaces hang
//!
//! A segment ends after its spaces (LB18), and the spaces at the end of a line
//! are not ink: a line whose last word fits and whose space after it does not
//! is a line that fits. So the fit test is on a segment's body, and its
//! trailing run of `SP`, `BK`, `CR`, `LF` and `NL` is carried as a pending
//! advance that counts only if another segment follows on the same line. A
//! line's [`Line::width`] is therefore the pen at the end of its last body.
//!
//! # A segment wider than the measure
//!
//! It is set alone on a line and the line says so ([`Line::overfull`]). It is
//! **not** split, because splitting it would be a break [`line::opportunities`]
//! did not offer — inside a word, or inside a cluster — and this module's whole
//! claim is that it never makes one. Hyphenation is the honest repair and is a
//! tailoring nobody has argued.
//!
//! # Where the two specifications disagree: U+001C to U+001E
//!
//! Every paragraph separator but three is a place UAX #14 ends a line: LF, CR,
//! NEL and U+2029 are `LF`, `CR`, `NL` and `BK`, all mandatory breaks, and a
//! CR LF is one separator. The information separators U+001C, U+001D and
//! U+001E are class `B` to UAX #9 and class `CM` to UAX #14, which attaches
//! them to the scalar before and offers no break after them. So P1 ends a
//! paragraph where no line may end, a line may not cross a paragraph, and
//! there is no setting both specifications allow. [`Setting::set`] refuses the
//! text ([`Unset::SeparatorMidLine`]) rather than overruling either one
//! silently. *What would reverse this:* a tailoring — UAX #14 treating the
//! three as `BK`, or a higher-level protocol treating them as segment
//! separators — argued in an RFC, since either changes what a conformance file
//! is held to.
//!
//! # Determinism
//!
//! Integers throughout, no allocator, no map, no clock. Every pixel comes out
//! of [`Pen::position`], which is the one rounding RFC 0004's text rule allows,
//! and the fit test is [`Pen::would_fit`], which does not round at all.

use core::iter::Peekable;

use crate::bidi::{self, BaseDirection, Paragraph, Slot};
use crate::line::{self, Opportunities};
use crate::line_break::LineBreak;
use crate::metric::{Advance, NotAMetric, Pen, Px, Scale};
use crate::property::line_break;

/// The most scalars one [`Setting`] holds.
///
/// Two hundred and fifty-six, which is above the largest text a declared node
/// can carry — `f_interface::node::TEXT_MAX` is 192 *bytes*, so at most 192
/// scalars — and `f_semantic::draw` holds that relation with a compile-time
/// assertion rather than a sentence, since this crate may not name that one.
/// A text past it is refused rather than truncated, for [`crate::cache`]'s
/// reason: a paragraph that stopped where a buffer did says the text ends
/// there.
/// Unit: scalars.
pub const SCALARS_MAX: usize = 256;

/// One line of a set paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    /// The first scalar on the line, as an index into [`Setting::scalars`].
    /// Unit: scalars.
    pub start: usize,
    /// One past the last. Always a position [`line::opportunities`] offers
    /// over the whole of [`Setting::scalars`] — by construction, since every
    /// line end is an item of that one iterator. Unit: scalars.
    pub end: usize,
    /// How far the pen travelled to the end of the line's last body, trailing
    /// spaces hanging. Unit: device pixels.
    pub width: Px,
    /// The embedding level of the paragraph the line is in: 0 or 1, and the
    /// side a consumer aligns the line to. Unit: embedding levels.
    pub level: u8,
    /// One segment wider than the measure sits here alone, unsplit.
    pub overfull: bool,
}

impl Line {
    /// A line nobody set: what a [`Setting`] fills its table with.
    pub const UNSET: Self = Self { start: 0, end: 0, width: Px::ZERO, level: 0, overfull: false };
}

/// Why a paragraph could not be set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unset {
    /// More scalars than [`SCALARS_MAX`]. Unit: scalars.
    TooLong {
        /// How many the text has, at least.
        scalars: usize,
    },
    /// UAX #9 declined; carried whole rather than paraphrased.
    Bidi(bidi::Refusal),
    /// An advance the seat returned, or a position a line reached, is not a
    /// metric; carried whole.
    Metric(NotAMetric),
    /// UAX #9 ends a paragraph at `at` and UAX #14 offers no line end there:
    /// the separator before it is U+001C, U+001D or U+001E. The module
    /// documentation says why this is refused rather than resolved.
    SeparatorMidLine {
        /// One past the separator: where the paragraph ends. Unit: scalars.
        at: usize,
    },
}

impl From<NotAMetric> for Unset {
    fn from(not: NotAMetric) -> Self {
        Self::Metric(not)
    }
}

impl From<bidi::Refusal> for Unset {
    fn from(refusal: bidi::Refusal) -> Self {
        Self::Bidi(refusal)
    }
}

/// A paragraph, set: its scalars, its lines and each line's visual order.
///
/// Lent by the caller, for [`crate::cache::Slot`]'s reason: this crate has no
/// allocator, so the buffers are a value the caller places. Every array is
/// [`SCALARS_MAX`] long and only the prefix [`Setting::set`] reports is
/// meaningful.
///
/// **It is 12 560 bytes** on both of this tree's targets — [`SETTING_BYTES`],
/// asserted below, so the number moves with the struct and not with this
/// sentence. That is more than three quarters of a component's four pages of
/// stack, and `new` returning it by value can hold two copies at once in an
/// unoptimised build, so **a component does not make one on its stack**. It
/// goes where `user/compositor` puts its arena, for that crate's reason — a
/// component may hold no writable static — which is a `Box` in the heap the
/// frame mapped. The line table is half of it, one [`Line`] per scalar
/// because a text of line feeds is a line per scalar. *What would reverse
/// this:* a caller that needs it on a stack, at which point the line table's
/// indices become `u16` — [`SCALARS_MAX`] fits — and the size is re-asserted.
#[derive(Clone, Debug)]
pub struct Setting {
    /// The text, decoded once. A working copy of the caller's string for the
    /// length of one [`Setting::set`], because UAX #9 and UAX #14 read scalars
    /// and a `&str` is bytes; it is never an input to anything but the three
    /// stages, and [`Setting::scalars`] hands it back so a caller can hold it
    /// to the string it came from.
    scalars: [char; SCALARS_MAX],
    len: usize,
    slots: [Slot; SCALARS_MAX],
    levels: [u8; SCALARS_MAX],
    /// Per line, `order[start..end]` is that line left to right, as indices
    /// into `scalars`.
    order: [u32; SCALARS_MAX],
    lines: [Line; SCALARS_MAX],
    count: usize,
}

/// The size of a [`Setting`], which its documentation states. Unit: bytes.
pub const SETTING_BYTES: usize = 12_560;

// Both targets are 64-bit; a 32-bit one would halve the `usize` fields and is
// a different sentence, so the assertion is not written for it.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    core::mem::size_of::<Setting>() == SETTING_BYTES,
    "a Setting's documented size moved: restate it, and re-read who can hold one"
);

impl Default for Setting {
    fn default() -> Self {
        Self::new()
    }
}

impl Setting {
    /// Nothing set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scalars: ['\0'; SCALARS_MAX],
            len: 0,
            slots: [Slot::EMPTY; SCALARS_MAX],
            levels: [0; SCALARS_MAX],
            order: [0; SCALARS_MAX],
            lines: [Line::UNSET; SCALARS_MAX],
            count: 0,
        }
    }

    /// Set `text` in lines no wider than `measure`, and answer the lines.
    ///
    /// `advance` is the shaper's seat: given a run of scalars in logical order,
    /// how many design units it moves the pen, on the grid `scale` was made
    /// with. The module documentation says why it is a parameter and why it is
    /// asked per segment.
    ///
    /// # Errors
    ///
    /// [`Unset::TooLong`] past [`SCALARS_MAX`], [`Unset::SeparatorMidLine`]
    /// where the two specifications disagree, and whatever [`bidi::resolve`]
    /// or the pen refuses, carried whole. Nothing set by an earlier call
    /// survives a refusal, and neither does a partial setting of this one.
    pub fn set<M>(
        &mut self,
        text: &str,
        base: BaseDirection,
        scale: Scale,
        measure: Px,
        advance: M,
    ) -> Result<&[Line], Unset>
    where
        M: FnMut(&[char]) -> Result<Advance, NotAMetric>,
    {
        self.len = 0;
        self.count = 0;
        if let Err(unset) = self.lay(text, base, scale, measure, advance) {
            self.len = 0;
            self.count = 0;
            return Err(unset);
        }
        Ok(&self.lines[..self.count])
    }

    /// [`Setting::set`]'s body, which may stop half way; `set` clears what it
    /// left.
    fn lay<M>(
        &mut self,
        text: &str,
        base: BaseDirection,
        scale: Scale,
        measure: Px,
        mut advance: M,
    ) -> Result<(), Unset>
    where
        M: FnMut(&[char]) -> Result<Advance, NotAMetric>,
    {
        for c in text.chars() {
            if self.len == SCALARS_MAX {
                return Err(Unset::TooLong { scalars: text.chars().count() });
            }
            self.scalars[self.len] = c;
            self.len += 1;
        }

        let Self { scalars, len, slots, levels, order, lines, count } = self;
        let text = &scalars[..*len];
        // One iterator over the whole text, which every paragraph draws its
        // line ends from in turn: the module documentation's step 3.
        let mut offered = line::opportunities(text).peekable();
        let mut from = 0;
        while from < text.len() {
            let to = from + bidi::paragraph_len(&text[from..]);
            let paragraph = bidi::resolve(&text[from..to], base, &mut slots[from..to])?;
            let mut into = Lines { into: &mut *lines, count: &mut *count, level: paragraph.level };
            let span = Span { text, from, to };
            fill(span, &mut offered, scale, measure, &mut advance, &mut into)?;
            from = to;
        }

        for line in &lines[..*count] {
            let (start, end) = (line.start, line.end);
            let paragraph = Paragraph { level: line.level };
            bidi::line_levels(&slots[start..end], paragraph, &mut levels[start..end])?;
            bidi::visual_order(&levels[start..end], &mut order[start..end])?;
            for place in &mut order[start..end] {
                // `start` is below `SCALARS_MAX`, so it fits the index type
                // `visual_order` writes, and the sum is below it too.
                *place += start as u32;
            }
        }
        Ok(())
    }

    /// The scalars the last [`Setting::set`] read, in logical order.
    #[must_use]
    pub fn scalars(&self) -> &[char] {
        &self.scalars[..self.len]
    }

    /// The lines the last [`Setting::set`] produced, top to bottom.
    #[must_use]
    pub fn lines(&self) -> &[Line] {
        &self.lines[..self.count]
    }

    /// `line` left to right: indices into [`Setting::scalars`], leftmost
    /// first. This is what a rasteriser walks, and the only order it may walk.
    #[must_use]
    pub fn visual(&self, line: &Line) -> &[u32] {
        let end = line.end.min(self.len);
        &self.order[line.start.min(end)..end]
    }

    /// The level each scalar of `line` is shown at, after L1, in logical order.
    #[must_use]
    pub fn levels(&self, line: &Line) -> &[u8] {
        let end = line.end.min(self.len);
        &self.levels[line.start.min(end)..end]
    }
}

/// Where [`fill`] puts the lines of one paragraph.
struct Lines<'a> {
    into: &'a mut [Line; SCALARS_MAX],
    count: &'a mut usize,
    level: u8,
}

impl Lines<'_> {
    fn push(&mut self, start: usize, end: usize, pen: &Pen, overfull: bool) -> Result<(), Unset> {
        // Unreachable: every line holds at least one scalar, because a UAX #14
        // opportunity is never at position zero (LB2), and there are at most
        // `SCALARS_MAX` scalars. Answered rather than indexed so a mistake in
        // that argument is a refusal and not a panic in a projection.
        let slot = self.into.get_mut(*self.count).ok_or(Unset::TooLong { scalars: end })?;
        *slot = Line { start, end, width: pen.position(), level: self.level, overfull };
        *self.count += 1;
        Ok(())
    }
}

/// Is `c` a scalar that hangs at the end of a line rather than taking room on
/// it: a space, or a mandatory break.
fn hangs(c: char) -> bool {
    matches!(
        line_break(c),
        LineBreak::SP | LineBreak::BK | LineBreak::CR | LineBreak::LF | LineBreak::NL
    )
}

/// One paragraph of a text: `text[from..to]`, in the whole text's indices.
#[derive(Clone, Copy)]
struct Span<'t> {
    text: &'t [char],
    from: usize,
    to: usize,
}

/// One paragraph's lines, greedily, at the opportunities the text path offers
/// over the whole text.
///
/// `offered` is that one iterator, positioned at the paragraph's first
/// opportunity; this takes the ones up to the paragraph's end and leaves the
/// rest. The paragraph's end must be one of them, and ends a line.
fn fill<M>(
    span: Span<'_>,
    offered: &mut Peekable<Opportunities<'_>>,
    scale: Scale,
    measure: Px,
    advance: &mut M,
    lines: &mut Lines<'_>,
) -> Result<(), Unset>
where
    M: FnMut(&[char]) -> Result<Advance, NotAMetric>,
{
    let Span { text, from, to } = span;
    let mut start = from;
    let mut pen = Pen::new(scale);
    // The trailing spaces of the last body on this line: counted only if a
    // body follows them here.
    let mut pending = Advance::ZERO;
    let mut placed = false;
    let mut overfull = false;
    let mut segment = from;

    while let Some(opportunity) = offered.next_if(|o| o.at <= to) {
        let run = &text[segment..opportunity.at];
        let tail = run.iter().rev().take_while(|&&c| hangs(c)).count();
        let (body, spaces) = run.split_at(run.len() - tail);
        let body = if body.is_empty() { Advance::ZERO } else { advance(body)? };
        let spaces = if spaces.is_empty() { Advance::ZERO } else { advance(spaces)? };

        let mut trial = pen;
        trial.advance(pending)?;
        if placed && !trial.would_fit(body, measure) {
            lines.push(start, segment, &pen, overfull)?;
            start = segment;
            pen = Pen::new(scale);
            trial = pen;
            overfull = false;
        }
        trial.advance(body)?;
        overfull |= !trial.fits(measure);
        pen = trial;
        pending = spaces;
        placed = true;

        // A paragraph's end ends a line even where UAX #14 only allows one
        // there, because the line rules read a line of one paragraph.
        if opportunity.mandatory || opportunity.at == to {
            lines.push(start, opportunity.at, &pen, overfull)?;
            start = opportunity.at;
            pen = Pen::new(scale);
            pending = Advance::ZERO;
            placed = false;
            overfull = false;
        }
        segment = opportunity.at;
    }
    if segment == to { Ok(()) } else { Err(Unset::SeparatorMidLine { at: to }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seat, filled with a constant: every scalar is half an em. **Not a
    /// shaper** — a stand-in that makes a line's width a count a reader can do
    /// in their head, so that what these tests hold is where lines end and in
    /// what order they are shown, which is this module's, and not a glyph,
    /// which is the import's.
    fn monospace(run: &[char]) -> Result<Advance, NotAMetric> {
        Advance::in_design_units(500 * i32::try_from(run.len()).unwrap_or(i32::MAX))
    }

    /// A 1000-unit grid at a 32-pixel em, so every scalar is sixteen pixels.
    fn scale() -> Scale {
        Scale::new(1000, 32 * 64).expect("a scale")
    }

    fn px(px: i32) -> Px {
        Px::new(px).expect("a width")
    }

    fn set(text: &str, measure: i32) -> Setting {
        let mut setting = Setting::new();
        setting
            .set(text, BaseDirection::FirstStrong, scale(), px(measure), monospace)
            .expect("sets");
        offered_and_tiled(&setting);
        setting
    }

    /// What [`Line::end`] promises, held over every setting these tests make:
    /// the lines tile the text in order, and each ends where
    /// [`line::opportunities`] offers over the *whole* text — not over the
    /// paragraph the line is in, which is the promise a paragraph split could
    /// break without anything else here noticing.
    fn offered_and_tiled(setting: &Setting) {
        let scalars = setting.scalars();
        let mut at = 0;
        for line in setting.lines() {
            assert_eq!(line.start, at, "the lines do not tile: {:?}", setting.lines());
            assert!(
                line::opportunities(scalars).any(|o| o.at == line.end),
                "a line ends at {} and the whole text offers no break there: {:?}",
                line.end,
                setting.lines()
            );
            at = line.end;
        }
        assert_eq!(at, scalars.len(), "the lines stop short of the text");
    }

    /// Does `line` hold exactly `want`, in logical order?
    fn is(setting: &Setting, line: &Line, want: &str) -> bool {
        want.chars().eq(setting.scalars()[line.start..line.end].iter().copied())
    }

    /// Every line ends where the text path offered to end one, and each line
    /// is as full as it could be: the next segment would not have fitted.
    #[test]
    fn a_line_ends_only_at_an_opportunity_and_only_when_the_next_word_does_not_fit() {
        let text = "Sound plays through the device until another is chosen";
        let setting = set(text, 320);
        let scalars = setting.scalars();
        let offered = |at: usize| line::opportunities(scalars).any(|o| o.at == at);
        let lines = setting.lines();
        assert_eq!(lines.len(), 3, "twenty scalars of room in fifty-five");
        for line in lines {
            assert!(offered(line.end), "a line ended at {} and nothing offered it", line.end);
            assert!(line.width.px() <= 320, "a line is {} px in 320", line.width.px());
        }
        assert!(is(&setting, &lines[0], "Sound plays through "));
        assert!(is(&setting, &lines[1], "the device until "));
        assert!(is(&setting, &lines[2], "another is chosen"));
        assert_eq!(lines[0].width.px(), 19 * 16, "the space after `through` hangs");
    }

    /// A mandatory break ends a line with room left on it, and a paragraph
    /// separator ends a paragraph as well.
    #[test]
    fn a_mandatory_break_ends_a_line_that_had_room() {
        let setting = set("one\ntwo", 1000);
        let lines = setting.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].start, lines[0].end), (0, 4));
        assert_eq!(lines[0].width.px(), 3 * 16, "the line feed hangs");
        assert_eq!((lines[1].start, lines[1].end), (4, 7));
    }

    /// A word wider than the measure is set alone and said to overflow; it is
    /// never split where no opportunity was offered.
    #[test]
    fn a_word_wider_than_the_measure_is_set_alone_and_not_split() {
        let setting = set("a extraordinarily b", 64);
        let lines = setting.lines();
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].overfull);
        assert!(is(&setting, &lines[1], "extraordinarily "));
        assert!(lines[1].overfull, "fifteen scalars in four is overfull");
        assert!(!lines[2].overfull);
    }

    /// A right-to-left word inside a left-to-right paragraph is shown reversed
    /// on its line and nowhere else: N1 gives the space between it and the
    /// Latin after it no direction of its own, N2 gives it the paragraph's, and
    /// L2 reverses the run at level 1 alone.
    #[test]
    fn a_line_is_shown_in_the_order_uax_9_gives_that_line() {
        let setting = set("named \u{05E9}\u{05DC}\u{05D5}\u{05DD} until", 1000);
        let lines = setting.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].level, 0, "the first strong scalar is `n`");
        let shown = setting.visual(&lines[0]);
        assert_eq!(&shown[..6], &[0, 1, 2, 3, 4, 5], "`named ` is as written");
        assert_eq!(&shown[6..10], &[9, 8, 7, 6], "the Hebrew word is reversed");
        assert_eq!(&shown[10..], &[10, 11, 12, 13, 14, 15], "` until` is as written");
    }

    /// A right-to-left paragraph is level 1 and its line is shown from the
    /// right: the first scalar read is the last one drawn from the left.
    #[test]
    fn a_right_to_left_paragraph_is_shown_from_the_right() {
        let setting = set("\u{05D0}\u{05D1} \u{05D2}\u{05D3}", 1000);
        let line = setting.lines()[0];
        assert_eq!(line.level, 1);
        assert_eq!(setting.visual(&line), &[4, 3, 2, 1, 0]);
    }

    /// CR LF is one line end, after the LF: GB3 makes the pair one cluster
    /// and LB5 forbids a break inside it, and P1 counts it as one separator.
    /// It used to end a line between the two and add an empty one.
    #[test]
    fn cr_lf_ends_one_line_after_the_lf() {
        let setting = set("one\r\ntwo", 1000);
        let lines = setting.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!((lines[0].start, lines[0].end), (0, 5));
        assert_eq!(lines[0].width.px(), 3 * 16, "both halves of the newline hang");
        assert_eq!((lines[1].start, lines[1].end), (5, 8));
        // A lone CR, NEL and U+2029 each still end a paragraph and a line.
        for (text, end) in [("one\rtwo", 4), ("one\u{85}two", 4), ("one\u{2029}two", 4)] {
            let setting = set(text, 1000);
            assert_eq!(setting.lines().len(), 2, "{text:?}");
            assert_eq!(setting.lines()[0].end, end, "{text:?}");
        }
    }

    /// U+001C to U+001E end a paragraph to UAX #9 and are combining marks to
    /// UAX #14, so no line may end after one: refused, whole, and never a line
    /// ending where the text offers none.
    #[test]
    fn an_information_separator_is_refused_rather_than_broken_after() {
        for text in ["one\u{1C}two", "one\u{1D}two", "one\u{1E}two"] {
            let mut setting = set("kept", 1000);
            let refused =
                setting.set(text, BaseDirection::FirstStrong, scale(), px(1000), monospace);
            assert_eq!(refused, Err(Unset::SeparatorMidLine { at: 4 }), "{text:?}");
            assert!(setting.lines().is_empty() && setting.scalars().is_empty());
        }
        // Where UAX #14 does allow a break after one — before an ideograph,
        // by LB31 — the paragraph's end is an offered position, and it ends
        // the line although nothing made the break mandatory.
        let setting = set("a\u{1C}\u{4E2D}", 1000);
        let lines = setting.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!((lines[0].end, lines[1].end), (2, 3));
        assert_eq!((lines[0].level, lines[1].level), (0, 0));
    }

    /// A combining mark after a space belongs to the space's cluster (GB9),
    /// and UAX #14 alone would break before it (LB10 makes it `AL`, LB18
    /// breaks after `SP`). A line never starts with it: the break is before
    /// the space's cluster ends, or not at all.
    #[test]
    fn a_line_never_starts_inside_a_cluster() {
        let setting = set("aaaa \u{0301}bbbb", 80);
        for line in setting.lines() {
            assert_ne!(
                setting.scalars().get(line.start).copied(),
                Some('\u{0301}'),
                "{:?}",
                setting.lines()
            );
        }
    }

    /// The fit is exact on the design grid and rounds nothing: 5005 units a
    /// letter on a 1000-unit grid at 32 px is 160.16 px, so two letters are
    /// 320.32 px and do not fit in 320. Rounding each position first and
    /// comparing whole pixels would call it 320 and set one line.
    #[test]
    fn a_third_of_a_pixel_over_the_measure_breaks_the_line() {
        let seat = |run: &[char]| -> Result<Advance, NotAMetric> {
            let letters = run.iter().filter(|c| !c.is_whitespace()).count();
            Advance::in_design_units(5005 * i32::try_from(letters).unwrap_or(i32::MAX))
        };
        let mut setting = Setting::new();
        let lines =
            setting.set("a b", BaseDirection::FirstStrong, scale(), px(320), seat).expect("sets");
        assert_eq!(lines.len(), 2, "320.32 px does not fit in 320: {lines:?}");
        let lines =
            setting.set("a b", BaseDirection::FirstStrong, scale(), px(321), seat).expect("sets");
        assert_eq!(lines.len(), 1, "and does fit in 321: {lines:?}");
    }

    /// Past the bound is a refusal, and an earlier setting does not survive it.
    #[test]
    fn a_text_past_the_bound_is_refused_whole() {
        let mut setting = set("kept", 100);
        let buffer = [b'x'; SCALARS_MAX + 1];
        let text = core::str::from_utf8(&buffer).expect("ascii");
        let refused = setting.set(text, BaseDirection::FirstStrong, scale(), px(100), monospace);
        assert_eq!(refused, Err(Unset::TooLong { scalars: SCALARS_MAX + 1 }));
        assert!(setting.lines().is_empty() && setting.scalars().is_empty());
    }
}
