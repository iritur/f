// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The unit discipline of the text path: what an advance is, what it is
//! measured in, and — the part that is actually a decision — **where it is
//! rounded**.
//!
//! RFC 0004 forbids both of the language's floating-point types, so a text
//! metric here is a fixed-point integer with its scale in its name. That much is
//! a rule and is not interesting; every module in this tree obeys it. What is
//! interesting is that forbidding the float does not answer the question the
//! float was hiding. A renderer that accumulated advances in a double still had
//! to put a glyph on a grid eventually, and the only reason nobody argued about
//! *when* is that the double made the wrong answer look close enough for a
//! while. Take it away and the question is in the open, which is where this
//! module wants it.
//!
//! # The decision
//!
//! **An advance is never rounded. A position is rounded once, when it is read.**
//!
//! There is exactly one division in this module. It lives in [`fine`], a private
//! module which is the only code in the crate that can see the integer inside an
//! accumulated position, and every public answer in pixels comes out of it. A
//! caller cannot round differently because a caller cannot get at a number to
//! round: [`Advance`] is design units and exposes none, [`Scale`] exposes none,
//! and the accumulator's unit is a private type. The only *distance* this
//! module hands out is a [`Px`], and it is already rounded. The one other
//! integer it returns is [`Pen::glyphs`], a count of glyphs, which is not a
//! length and divides into nothing.
//!
//! That is the third clause of `E3-B03d`'s exit taken structurally rather than
//! by convention. A rounding rule written in a doc comment and obeyed at each
//! call site is a rule the next call site gets wrong; a rounding rule that is
//! the only reachable arithmetic is a rule a later edit cannot walk past
//! without deleting it on purpose.
//!
//! # What that guarantee is, stated narrowly enough to be true
//!
//! It is not a claim that nobody anywhere can divide. Whoever calls
//! [`Scale::new`] passes `upem` and the em in and still has both afterwards; no
//! type here takes a number out of somebody's hands. The claim is about what
//! this module *hands out*, and it is worth stating in the case where that
//! bites. The consumer of a shaped run did not build the scale — the shaper did,
//! behind `E3-B03a`'s ring — so the run arrives as opaque [`Advance`]s and an
//! opaque [`Scale`], and the consumer's only route to a number is
//! [`Pen::position`]. **A run measured somewhere else cannot be re-rounded
//! here.** That is a property a caller doing its own arithmetic on its own
//! numbers never had and never claimed.
//!
//! `Debug` was the hole in exactly that case, and it is written down rather
//! than quietly closed because the next person will reach for the derive again.
//! Derived on [`Scale`] it printed `upem` beside the em; derived on [`Pen`] it
//! printed the accumulator. Either one hands a consumer both halves of the
//! arithmetic through a channel nobody thinks of as an interface, and it
//! typechecks. So the accumulator has **no `Debug` at all**, which turns
//! `#[derive(Debug)]` on either type that holds one into a compile error rather
//! than a leak, and the two hand-written impls print pixels — numbers that have
//! already been through the rounding.
//!
//! # Why the fine unit is one pixel over `upem * 64`
//!
//! The obvious fine unit is the one every shaper on the planet uses: a
//! sixty-fourth of a pixel, the 26.6 fixed point. It is the wrong unit *here*,
//! and the reason is that it does not divide the thing being converted. A face
//! states an advance as an integer count of its own design units; turning that
//! into sixty-fourths of a pixel is `units * em / upem`, which has a remainder,
//! and a remainder thrown away per glyph is exactly the per-glyph rounding this
//! module exists to not do.
//!
//! So the fine unit is chosen to make the conversion have no remainder at all:
//! **one fine unit is one pixel divided by `upem * 64`**, and an advance of `u`
//! design units at an em of `s` sixty-fourths of a pixel is exactly `u * s` fine
//! units. No division, no truncation, nothing discarded. The accumulator is a
//! wide signed integer of those, the bounds below prove it cannot overflow, and
//! the single division converts the whole prefix sum to pixels when somebody
//! asks for one.
//!
//! The cost is that the fine unit is not the same size for two faces with
//! different design grids, which is why it is not a type a caller may hold —
//! see [`fine`]. The benefit is that the per-glyph step is exact, and *exact* is
//! a much stronger thing to be able to say than *accurate to a sixty-fourth*.
//!
//! # The error, as an integer bound
//!
//! Let a run be `N` glyphs of `u_1..u_N` design units on a face of `upem` units
//! per em at an em of `s` sixty-fourths of a pixel. The exact position after the
//! first `k` of them is `s * (u_1 + .. + u_k) / (upem * 64)` pixels — a rational
//! number, which is the whole difficulty.
//!
//! This module computes that numerator exactly and divides once, rounding half
//! away from zero. So for every `k`, and for every `N`:
//!
//! ```text
//! 2 * |position_k * upem * 64  -  s * (u_1 + .. + u_k)|  <=  upem * 64
//! ```
//!
//! — every origin is within **half a pixel** of exact, and the bound does not
//! mention `N`. `every_origin_in_a_long_run_is_within_half_a_pixel` asserts that
//! inequality over a two-hundred-glyph run rather than over an example, because
//! an example is what a drifting implementation also passes.
//!
//! The two alternatives are worth stating as numbers, since the argument for
//! this one is entirely that theirs grow:
//!
//! - **Round each advance to a whole pixel, then sum.** Error up to `N / 2`
//!   pixels. A sixty-glyph line drifts up to thirty pixels — two ems at the
//!   sixteen-pixel em this module was written against. This is not a subtle bug;
//!   it is the one that makes justified text stop reaching the margin.
//! - **Round each advance to a sixty-fourth of a pixel, then sum.** Error up to
//!   `N / 128` pixels. A sixty-glyph line is off by under half a pixel and looks
//!   fine; an eight-hundred-glyph paragraph is off by six, and a cursor placed
//!   by re-measuring a prefix lands six pixels from the caret drawn by the
//!   renderer. That is the version that survives review, ships, and is diagnosed
//!   two years later.
//!
//! The property that kills both is that this module rounds the *prefix sums*
//! rather than summing the *roundings*. Each origin is derived from the exact
//! total before it, so errors do not compound: origin `k` is wrong by at most
//! half a pixel whatever origin `k - 1` did.
//!
//! # Why a run's width and the next glyph's origin are one function
//!
//! [`Pen::position`] is both. A separate `width` would be a second expression
//! producing a pixel count, and a second expression is a second rounding — which
//! would be discovered the day a line's width and the origin of the glyph after
//! it disagreed by one pixel, in a layout nobody could reproduce. One function,
//! and the two questions are the same question because they are the same number.
//!
//! # Why a pen is bound to one scale
//!
//! Because a shaped run *is* one face at one size with one feature set — that is
//! what makes it a run rather than a paragraph. A [`Pen`] therefore holds a
//! [`Scale`] and cannot be handed an advance from another face, which is also
//! what makes the fine unit coherent: every quantity inside one pen is in the
//! same unit by construction. A line of mixed faces is several pens, and the
//! coarse positions they produce compose, because a [`Px`] is a pixel whoever
//! measured it.
//!
//! That composition is the one place error still accumulates, and the bound
//! above does not cover it, so here it is: **`M` runs laid end to end are within
//! `M / 2` pixels of exact**, each contributing its own half. A two-face line is
//! within one pixel. A line that changed face at every glyph would be the
//! `N / 2` design this module rejects, arrived at from the other end.
//! `two_runs_compose_with_half_a_pixel_of_slack_each` exhibits a pair four
//! fifths of a pixel out, so the cost is a number a reader can weigh rather than
//! a caveat.
//!
//! It can be priced and not removed. Two faces have different fine units — one
//! pixel over `upem * 64`, and the `upem`s differ — so there is no exact
//! accumulator the two runs could share short of one whose unit divides both
//! grids, which is a common multiple of every `upem` on the line and is how an
//! exactness argument turns into an overflow one. What would reverse the choice
//! is a measurement: mixed-face lines drifting visibly at the run counts real
//! text has. That is `E3-B03f`'s to take, because it is the part that knows how
//! many runs a paragraph breaks into.
//!
//! # What is not decided here
//!
//! Subpixel positioning. This module's coarse unit is the whole device pixel,
//! which is the grid a raster target actually has. If `E3-B03g`'s atlas wants
//! glyphs at thirds of a pixel it changes `fine::Fine::round_to_px` and nothing
//! else, and that is the point of there being one of it.
//!
//! Hinting, which moves an outline rather than an advance and is a different
//! argument. Kerning and mark attachment, which arrive as more advances and need
//! no new unit — a negative one is legal here for exactly that reason, and so a
//! right-to-left run is a run of negative advances and rounds symmetrically,
//! which is why the rule is *half away from zero* and not *half up*.
//!
//! # Determinism
//!
//! This crate has no dependency, `f-env` included, so nothing here can observe a
//! clock, draw a random number or iterate a seeded map even by accident. Every
//! function in this module is a pure function of its arguments and of nothing
//! else — no state, no allocation, no interior mutability — which is a stronger
//! guarantee than a seed would be, because there is no seed to get wrong. RFC
//! 0004's allow-list gains nothing from this file and must not: an entry here
//! would mean a text metric had acquired a source of nondeterminism, and the
//! whole of the argument above is that a text metric is arithmetic on integers.

/// The smallest design grid a face may state, in units per em.
///
/// Sixteen. Not a typographic judgement — it is the bound that makes the
/// rounding's divisor at least 1 024, so the halving cannot divide by something
/// small enough for the doubling trick to be coarse. Real faces are 1 000 (CFF)
/// or 2 048 (TrueType) and nothing comes near this.
pub const UPEM_MIN: u16 = 16;

/// The largest design grid a face may state, in units per em.
///
/// Sixteen thousand three hundred and eighty-four, which is eight times the
/// largest grid in use. It is a bound for the overflow argument in [`PX_MAX`]
/// and for no other reason, and it is stated as a power of two so that the
/// argument is by exponents and a reader can check it without multiplying.
pub const UPEM_MAX: u16 = 16_384;

/// The smallest em a [`Scale`] may be built at, in sixty-fourths of a pixel.
///
/// One pixel. Below it every glyph in the face rounds to the same position and
/// the run has no metrics left to be right about; a caller asking for it has a
/// bug upstream, and returning [`NotAMetric::EmSizeOutOfRange`] says so at the
/// place the mistake was made rather than producing a line of zeroes.
pub const EM_PX_X64_MIN: i32 = 64;

/// The largest em a [`Scale`] may be built at, in sixty-fourths of a pixel.
///
/// One thousand and twenty-four pixels. `interface`'s token layer caps the em at
/// seventy-two points, and this is far above any density that reaches; the
/// number is here to bound the product in [`Pen::advance`], not to have an
/// opinion about display text.
pub const EM_PX_X64_MAX: i32 = 64 * 1_024;

/// The largest advance a single glyph may state, in design units.
///
/// A million — sixty-four ems on the largest legal grid, which no glyph is.
/// Deliberately absurd: this bound exists to make the overflow argument
/// arithmetic rather than typographic, and a bound chosen to judge a face would
/// be this module deciding something about somebody else's font that it has no
/// way to know.
pub const ADVANCE_UNITS_MAX: i32 = 1 << 20;

/// The largest coarse position this module will represent, in pixels.
///
/// Sixteen million. Two orders past the widest line any display or paged
/// document has, and — the reason it is this and not larger — small enough that
/// `PX_MAX * UPEM_MAX * 64` is 2^44, which with the per-glyph bound of 2^36 from
/// [`ADVANCE_UNITS_MAX`] and [`EM_PX_X64_MAX`] keeps every intermediate in
/// [`Pen`] under 2^45 and therefore inside a sixty-four-bit signed integer with
/// eighteen bits to spare. **There is no saturating arithmetic anywhere in this
/// module and no wrapping one**, because a text metric that silently stopped
/// growing would be a line that silently stopped ending.
pub const PX_MAX: i32 = 1 << 24;

/// Why a number was not admitted as a metric.
///
/// Refusal rather than clamping, which is the opposite of what `interface`'s
/// token layer does and is the right way round for the same reason it gives: a
/// theme's colour has a nearest legal colour and taking it preserves what the
/// theme was pointing at, whereas an advance outside these bounds is not a
/// slightly-wrong advance, it is a face or a caller that is not telling the
/// truth. There is no nearest legal answer to *this glyph is sixty-five ems
/// wide*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotAMetric {
    /// The design grid is outside [`UPEM_MIN`]..=[`UPEM_MAX`], zero included —
    /// and zero is the one worth naming, because it is the divisor.
    UpemOutOfRange,
    /// The em is outside [`EM_PX_X64_MIN`]..=[`EM_PX_X64_MAX`].
    EmSizeOutOfRange,
    /// The advance's magnitude exceeds [`ADVANCE_UNITS_MAX`].
    AdvanceOutOfRange,
    /// The position's magnitude exceeds [`PX_MAX`], either because a [`Px`] was
    /// asked for directly or because a run grew there one glyph at a time.
    PositionOutOfRange,
}

/// A distance on the device's own grid, in whole pixels. **The coarse unit.**
///
/// This is the only integer this module hands out, and it is the only one it is
/// willing to: a pixel is a thing the target has, so a number in pixels has
/// already survived the decision this module exists to make. Everything upstream
/// of it — design units, ems, accumulated positions — is deliberately opaque.
///
/// It is signed because an origin can be negative: a right-to-left run advances
/// towards smaller coordinates, and a mark attaches behind the glyph it marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Px {
    px: i32,
}

impl Px {
    /// The origin.
    pub const ZERO: Self = Self { px: 0 };

    /// A length or a position measured somewhere else — a column width, a
    /// margin, the width a line must fit in.
    ///
    /// This is the way *in*, and there is deliberately no way in for a position:
    /// nothing in this module consumes a [`Px`] as a pen position, only as a
    /// limit. So a caller who did its own arithmetic on numbers it kept can
    /// certainly compute something, and cannot hand the result back as a text
    /// metric — which is the difference between a rule and a wish.
    ///
    /// # Errors
    ///
    /// [`NotAMetric::PositionOutOfRange`] past [`PX_MAX`] in either direction.
    pub const fn new(px: i32) -> Result<Self, NotAMetric> {
        if px > PX_MAX || px < -PX_MAX {
            return Err(NotAMetric::PositionOutOfRange);
        }
        Ok(Self { px })
    }

    /// The pixel count.
    #[must_use]
    pub const fn px(self) -> i32 {
        self.px
    }
}

/// How far a glyph moves the pen, in the face's own design units.
///
/// **Not a distance.** A count of design units means nothing without the grid it
/// counts on, and that grid lives in [`Scale`]; this type is the numerator and
/// nothing else. That is why it has no accessor: handing back the integer would
/// be handing back half of the arithmetic, and the other half is one
/// [`Scale::new`] away from being a caller's own rounding. The scale is in the
/// constructor's name rather than in a numeric suffix because it is *data* — the
/// face states it — and a name like `units_x1000` would be a lie on a
/// 2 048-unit face.
///
/// `Debug` is derived and prints the count. That is not a way around the
/// paragraph above: a count of design units with no `upem` and no em size cannot
/// be turned into a pixel by anybody, which is the same reason the accessor is
/// absent rather than a weaker version of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    design_units: i32,
}

impl Advance {
    /// A glyph that does not move the pen — a mark, or the zero-width joiner
    /// that `E3-B03f`'s cluster rules will have opinions about.
    pub const ZERO: Self = Self { design_units: 0 };

    /// An advance as the face states it.
    ///
    /// Negative is legal and is not an error case bolted on: right-to-left runs
    /// advance backwards and kerning pairs are routinely negative, so a type
    /// that refused them would push every bidi caller into doing its own sign
    /// arithmetic — outside this module, where the rounding is not.
    ///
    /// # Errors
    ///
    /// [`NotAMetric::AdvanceOutOfRange`] past [`ADVANCE_UNITS_MAX`] in either
    /// direction.
    pub const fn in_design_units(design_units: i32) -> Result<Self, NotAMetric> {
        if design_units > ADVANCE_UNITS_MAX || design_units < -ADVANCE_UNITS_MAX {
            return Err(NotAMetric::AdvanceOutOfRange);
        }
        Ok(Self { design_units })
    }
}

/// A face's design grid together with the size it is being set at.
///
/// The two numbers that turn an [`Advance`] into a distance, and they are one
/// type rather than two arguments because they are only ever meaningful
/// together: an em with no grid divides nothing, and a grid with no em measures
/// nothing. Neither is readable back out, for [`Advance`]'s reason — the pair is
/// exactly what a caller would need to round on its own.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    upem: u16,
    em_px_x64: i32,
    /// [`PX_MAX`] expressed in this scale's fine unit, computed once here so
    /// that [`Pen::advance`]'s bound check is a comparison and not a division.
    /// The module's claim that it divides in one place is only as good as the
    /// places that were tempted to divide and did not.
    limit: fine::Fine,
}

impl Scale {
    /// A face's grid at a size.
    ///
    /// # Errors
    ///
    /// [`NotAMetric::UpemOutOfRange`] or [`NotAMetric::EmSizeOutOfRange`]. Both
    /// bounds are checked here and nowhere else, which is what lets [`Pen`]
    /// state its overflow argument as arithmetic on constants: no `Scale` exists
    /// that does not satisfy them.
    pub const fn new(upem: u16, em_px_x64: i32) -> Result<Self, NotAMetric> {
        if upem < UPEM_MIN || upem > UPEM_MAX {
            return Err(NotAMetric::UpemOutOfRange);
        }
        if em_px_x64 < EM_PX_X64_MIN || em_px_x64 > EM_PX_X64_MAX {
            return Err(NotAMetric::EmSizeOutOfRange);
        }
        Ok(Self { upem, em_px_x64, limit: fine::Fine::of_px(PX_MAX, upem) })
    }
}

impl core::fmt::Debug for Scale {
    /// The em in whole pixels, and deliberately not the pair that produced it.
    ///
    /// Hand-written because the derive no longer compiles — [`fine::Fine`] has
    /// no `Debug` — and that is the point rather than an inconvenience: a
    /// derived one printed `upem` beside the em, which is every number a
    /// consumer needs to round a run its own way. The em in pixels answers the
    /// question a `Debug` is actually read for, *what size is this set at*, and
    /// is not invertible: it says nothing about the grid the advances are
    /// counted on, so it composes with an [`Advance`]'s printed design units to
    /// give nothing.
    ///
    /// It goes through the one rounding, like every other pixel here, because an
    /// advance of `upem` design units is exactly one em. That makes this a use
    /// of the arithmetic above rather than a second copy of it — a `Debug` impl
    /// that divided on its own would be precisely the second rounding this
    /// module is accepted on not having.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let em = fine::Fine::ZERO.plus_advance(self.upem as i32, self.em_px_x64);
        f.debug_struct("Scale").field("em_px", &em.round_to_px(self.upem)).finish_non_exhaustive()
    }
}

/// The accumulated position, and the one place a pixel is produced.
///
/// This module is private, and that is the whole of the structural guard
/// `E3-B03d` is accepted on. `Fine` is a newtype over a wide signed integer with
/// a private field, so the only code in this crate that can see that integer is
/// the code in this module — `Fine::round_to_px` and its four exact neighbours.
/// Rounding somewhere else does not need discipline to avoid; it does not
/// typecheck, because outside these forty lines there is nothing to round.
///
/// Keeping `Fine` out of the public interface is the other half. A fine unit is
/// one pixel over `upem * 64` and therefore means different things on two faces,
/// so a public one would be a number two callers could compare and be wrong.
/// Inside a [`Pen`], which holds exactly one [`Scale`], every `Fine` is in the
/// same unit by construction and there is nothing to get wrong.
///
/// What would reverse the exactness claim: a source of advances that is not an
/// integer count of design units — a variable-font instance whose interpolated
/// advances arrive already scaled, or a shaper that hands back 26.6 and keeps
/// its own remainder. Then the per-glyph conversion acquires a remainder after
/// all, and the honest repair is to carry that remainder in `Fine` rather than
/// drop it per glyph, because the accumulator is the one thing here that could.
mod fine {
    /// A position in fine units: one pixel divided by `upem * 64`.
    ///
    /// Exact. Every advance that reaches it is an integer product, never a
    /// quotient, so nothing has been discarded by the time it is read.
    ///
    /// **It has no `Debug`, and the absence is load-bearing.** A derived one
    /// prints the integer, and the integer is the unrounded number the whole
    /// module exists to not hand out — a consumer holding a [`super::Pen`] it
    /// did not build could read it out of a format string and divide by
    /// whatever it liked. Leaving the derive off makes `#[derive(Debug)]` on
    /// [`super::Scale`] and [`super::Pen`], both of which hold one, fail to
    /// compile. That closes the lazy route to the leak with the compiler and
    /// leaves only a hand-written impl, which is a diff a reviewer sees rather
    /// than a default nobody reads.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Fine {
        px_x_upem_x64: i64,
    }

    impl Fine {
        /// The pen before anything is placed.
        pub const ZERO: Self = Self { px_x_upem_x64: 0 };

        /// This position moved by one glyph. A product of two bounded integers
        /// added to a bounded sum — **the step that would be a rounding in any
        /// design with a coarser fine unit, and is not one here.**
        pub const fn plus_advance(self, design_units: i32, em_px_x64: i32) -> Self {
            Self { px_x_upem_x64: self.px_x_upem_x64 + design_units as i64 * em_px_x64 as i64 }
        }

        /// A whole-pixel length in fine units. Exact, and in this direction it
        /// always is: multiplying up never has a remainder.
        pub const fn of_px(px: i32, upem: u16) -> Self {
            Self { px_x_upem_x64: px as i64 * upem as i64 * 64 }
        }

        /// Is this position's distance from the origin within `limit`?
        ///
        /// Magnitude rather than signed order, so a right-to-left run measures
        /// the same as the left-to-right one it mirrors. A negative `limit`
        /// admits nothing, which is the honest answer to a negative length.
        pub const fn magnitude_at_most(self, limit: Self) -> bool {
            let magnitude =
                if self.px_x_upem_x64 < 0 { -self.px_x_upem_x64 } else { self.px_x_upem_x64 };
            magnitude <= limit.px_x_upem_x64
        }

        /// **The rounding.** Half away from zero, and the only division in the
        /// crate.
        ///
        /// Away from zero rather than up, because a right-to-left run is a run
        /// of negative advances and rounding half up would give it a different
        /// answer from its mirror image — a bidi bug that shows as a column of
        /// Arabic sitting a pixel off from the Latin beside it, in the half of
        /// the cases where a tie happens to land. Symmetry about the origin is
        /// worth more than agreeing with the way most languages spell *round*.
        ///
        /// The doubling is how *nearest* is expressed without a fraction:
        /// `(2m + d) / 2d` is `m / d` rounded to nearest with ties going up in
        /// magnitude, and the sign is reapplied afterwards. `2 * m` is at most
        /// 2^45 by [`super::PX_MAX`]'s argument, so the doubling cannot overflow
        /// either.
        pub const fn round_to_px(self, upem: u16) -> i32 {
            let divisor = upem as i64 * 64;
            let value = self.px_x_upem_x64;
            let (sign, magnitude) = if value < 0 { (-1, -value) } else { (1, value) };
            (sign * ((2 * magnitude + divisor) / (2 * divisor))) as i32
        }
    }
}

/// Where advances accumulate, and therefore where the rounding rule is stated.
///
/// One pen is one run: one face, one size, advances in order. It holds the exact
/// prefix sum in the fine unit and converts it to pixels only when somebody asks
/// for a pixel, which is [`Pen::position`] and is the only such place.
///
/// The sequence a caller writes is: read [`Pen::position`] for the origin of the
/// glyph about to be drawn, then [`Pen::advance`] to move past it. Reading is
/// not mutation, so a caller may ask where the pen is as often as it likes and
/// get the same answer, and a caller that measures a run and then lays it out
/// gets *the same numbers*, because measuring and laying out are the same
/// arithmetic run twice rather than two implementations that agree today.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Pen {
    scale: Scale,
    at: fine::Fine,
    glyphs: u32,
}

impl Pen {
    /// A pen at the origin of a run on this scale.
    #[must_use]
    pub const fn new(scale: Scale) -> Self {
        Self { scale, at: fine::Fine::ZERO, glyphs: 0 }
    }

    /// Move the pen past one glyph.
    ///
    /// # Errors
    ///
    /// [`NotAMetric::PositionOutOfRange`] if the run would pass [`PX_MAX`]. The
    /// pen is left untouched, so a caller that stops on the error has a run it
    /// can still read — and a caller that ignores it does not compile, because
    /// the workspace denies `unused_must_use`. That is the alternative to
    /// saturating: a run that quietly stopped growing is a line that quietly
    /// stopped ending, and the only thing worse than a refusal is a number that
    /// looks like an answer.
    pub const fn advance(&mut self, advance: Advance) -> Result<(), NotAMetric> {
        let next = self.at.plus_advance(advance.design_units, self.scale.em_px_x64);
        if !next.magnitude_at_most(self.scale.limit) {
            return Err(NotAMetric::PositionOutOfRange);
        }
        self.at = next;
        self.glyphs += 1;
        Ok(())
    }

    /// Where the pen is, in whole pixels — **the one rounding, read**.
    ///
    /// This is the origin of the next glyph and, once the last advance is in, it
    /// is the run's width. One function for both, because two functions would be
    /// two roundings and the day they disagreed by a pixel would be a day nobody
    /// could reproduce. It is derived from the exact prefix sum rather than from
    /// the previous rounded position, which is the property that keeps the error
    /// at half a pixel however long the run gets.
    #[must_use]
    pub const fn position(&self) -> Px {
        // Within `PX_MAX` by `advance`'s check on every step that could have
        // moved it, so the fallible constructor would be noise at the one call
        // site that is structurally safe.
        Px { px: self.at.round_to_px(self.scale.upem) }
    }

    /// Would the run still be within `limit` after one more glyph?
    ///
    /// The question `E3-B03f`'s line breaker asks, and it is answered **without
    /// rounding at all**: both sides go into the fine unit, where the comparison
    /// is exact. Rounding first and comparing would let a run whose true width
    /// is a third of a pixel over the margin round down onto it and fit, which
    /// is how a line acquires one glyph too many and the paragraph below it
    /// reflows. `a_run_a_third_of_a_pixel_over_does_not_fit` is that case.
    #[must_use]
    pub const fn would_fit(&self, next: Advance, limit: Px) -> bool {
        let after = self.at.plus_advance(next.design_units, self.scale.em_px_x64);
        after.magnitude_at_most(fine::Fine::of_px(limit.px, self.scale.upem))
    }

    /// Is the run within `limit` as it stands?
    #[must_use]
    pub const fn fits(&self, limit: Px) -> bool {
        self.would_fit(Advance::ZERO, limit)
    }

    /// How many glyphs the pen has passed.
    ///
    /// Diagnostic, and the count a caller needs to say *this many glyphs fit*.
    /// It cannot overflow before [`Pen::advance`] refuses: a run that reached
    /// four billion glyphs passed [`PX_MAX`] long before, unless every one of
    /// them were zero-width, and a four-billion-mark cluster is not a thing this
    /// module owes an answer to.
    #[must_use]
    pub const fn glyphs(&self) -> u32 {
        self.glyphs
    }
}

impl core::fmt::Debug for Pen {
    /// Where the pen is and how many glyphs got it there — the two questions a
    /// reader of a `Debug` has, and both already rounded.
    ///
    /// Hand-written for [`Scale`]'s reason one level up: the derive would print
    /// the accumulator, which is the unrounded prefix sum, and it does not
    /// compile because [`fine::Fine`] has no `Debug`. Nothing is lost by the
    /// substitution — every field printed here is one a public accessor already
    /// returns, so this impl widens no interface, which is the test a `Debug`
    /// should pass and the derived one did not.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pen")
            .field("scale", &self.scale)
            .field("position_px", &self.position().px())
            .field("glyphs", &self.glyphs)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write as _;

    /// A 1 000-unit face, the grid a CFF outline is stated on.
    const UPEM: u16 = 1_000;

    /// Sixteen pixels of em, in sixty-fourths of one. With [`UPEM`] this makes
    /// the fine-to-pixel divisor 64 000.
    const EM: i32 = 16 * 64;

    /// The advance with no tidy answer in either unit: 521 units is 533 504 fine
    /// units, which is 8.336 pixels — not a whole pixel and not a whole
    /// sixty-fourth of one, which is what makes it worth measuring with.
    const AWKWARD_UNITS: i32 = 521;

    /// `upem * 64`: what a fine count is divided by to reach a pixel.
    const DIVISOR: i64 = UPEM as i64 * 64;

    fn scale() -> Scale {
        Scale::new(UPEM, EM).expect("a 1 000-unit face at sixteen pixels is inside every bound")
    }

    fn advance(units: i32) -> Advance {
        Advance::in_design_units(units).expect("inside ADVANCE_UNITS_MAX")
    }

    /// A fixed buffer to render a `Debug` into, because this crate is `no_std`
    /// and there is therefore no `format!`. It refuses rather than grows: the
    /// output this module produces is far shorter than the buffer, and a
    /// truncating one would let an assertion about what is *absent* pass by
    /// having cut it off.
    struct Line {
        bytes: [u8; 128],
        len: usize,
    }

    impl Line {
        const EMPTY: Self = Self { bytes: [0; 128], len: 0 };

        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.bytes[..self.len]).expect("debug output here is ASCII")
        }
    }

    impl core::fmt::Write for Line {
        fn write_str(&mut self, text: &str) -> core::fmt::Result {
            for &byte in text.as_bytes() {
                if self.len == self.bytes.len() {
                    return Err(core::fmt::Error);
                }
                self.bytes[self.len] = byte;
                self.len += 1;
            }
            Ok(())
        }
    }

    fn rendered(value: &dyn core::fmt::Debug) -> Line {
        let mut line = Line::EMPTY;
        write!(line, "{value:?}").expect("the buffer holds this module's debug output");
        line
    }

    fn run_of(units: i32, glyphs: u32) -> Pen {
        let mut pen = Pen::new(scale());
        for _ in 0..glyphs {
            pen.advance(advance(units)).expect("inside PX_MAX");
        }
        pen
    }

    #[test]
    fn an_empty_run_is_at_the_origin() {
        let pen = Pen::new(scale());
        assert_eq!(pen.position(), Px::ZERO);
        assert_eq!(pen.glyphs(), 0);
    }

    /// The bound the module doc states, asserted over every prefix of a long run
    /// rather than over a chosen one. A drifting implementation passes any
    /// single example near the start; this fails on the first prefix where the
    /// accumulated error crosses half a pixel, and a per-glyph rounder crosses
    /// it within the first handful.
    #[test]
    fn every_origin_in_a_long_run_is_within_half_a_pixel() {
        let mut pen = Pen::new(scale());
        for k in 0..=200i64 {
            let exact_numerator = k * i64::from(AWKWARD_UNITS) * i64::from(EM);
            let got = i64::from(pen.position().px());
            let error = (got * DIVISOR - exact_numerator).abs();
            assert!(2 * error <= DIVISOR, "glyph {k}: off by {error} of {DIVISOR} fine units");
            pen.advance(advance(AWKWARD_UNITS)).expect("inside PX_MAX");
        }
    }

    /// The same bound, held against the length of the run: the error does not
    /// grow with `N`, which is the entire claim.
    #[test]
    fn the_error_does_not_grow_with_the_length_of_the_run() {
        let mut pen = Pen::new(scale());
        for n in 1..=500i64 {
            pen.advance(advance(AWKWARD_UNITS)).expect("inside PX_MAX");
            let exact_numerator = n * i64::from(AWKWARD_UNITS) * i64::from(EM);
            let error = (i64::from(pen.position().px()) * DIVISOR - exact_numerator).abs();
            assert!(2 * error <= DIVISOR, "after {n} glyphs: off by {error} of {DIVISOR}");
        }
    }

    /// What the rejected design would have produced, as a number. Each glyph is
    /// 8.336 pixels; rounded per glyph it is 8, and a hundred of them are 800
    /// against a true 833.6. The gap is thirty-four pixels — two ems of a line
    /// that has gone missing — and the point of asserting it is that the number
    /// is large enough to see and small enough to look plausible in a screenshot.
    #[test]
    fn rounding_each_advance_instead_would_lose_thirty_four_pixels_in_a_hundred_glyphs() {
        let accumulated = run_of(AWKWARD_UNITS, 100).position().px();
        assert_eq!(accumulated, 834);

        let one_glyph = run_of(AWKWARD_UNITS, 1).position().px();
        assert_eq!(one_glyph, 8);
        assert_eq!(accumulated - one_glyph * 100, 34);
    }

    /// Half away from zero, both ways. At a ten-pixel em on this face an advance
    /// of fifty units is exactly half a pixel and one of a hundred and fifty is
    /// exactly one and a half, so all four ties are reachable by an integer.
    #[test]
    fn a_tie_rounds_away_from_zero_in_both_directions() {
        let ten_px = Scale::new(UPEM, 10 * 64).expect("inside every bound");
        for (units, want) in [(50, 1), (-50, -1), (150, 2), (-150, -2)] {
            let mut pen = Pen::new(ten_px);
            pen.advance(advance(units)).expect("inside PX_MAX");
            assert_eq!(pen.position().px(), want, "{units} design units at a ten-pixel em");
        }
    }

    /// The mirror property that motivates the rule: a run and its negation land
    /// on negated positions, so a right-to-left line is not a pixel narrower
    /// than the left-to-right line it reverses.
    #[test]
    fn a_right_to_left_run_mirrors_its_left_to_right_twin_exactly() {
        for glyphs in 0..40 {
            let forwards = run_of(AWKWARD_UNITS, glyphs).position().px();
            let backwards = run_of(-AWKWARD_UNITS, glyphs).position().px();
            assert_eq!(forwards, -backwards, "after {glyphs} glyphs");
        }
    }

    /// The fit test does not round, and this is the case that tells the two
    /// apart. One glyph of 521 units is 8.336 pixels; its rounded width is 8, so
    /// a line breaker that rounded first would seat it in an eight-pixel column.
    #[test]
    fn a_run_a_third_of_a_pixel_over_does_not_fit() {
        let eight = Px::new(8).expect("inside PX_MAX");
        let pen = Pen::new(scale());
        assert!(!pen.would_fit(advance(AWKWARD_UNITS), eight));
        assert_eq!(run_of(AWKWARD_UNITS, 1).position(), eight);
    }

    /// And an exact fit is a fit: 500 units at a sixteen-pixel em is eight
    /// pixels and no remainder. Without this the test above would also pass on
    /// an implementation that refused everything.
    #[test]
    fn a_run_that_is_exactly_the_limit_fits() {
        let eight = Px::new(8).expect("inside PX_MAX");
        let pen = Pen::new(scale());
        assert!(pen.would_fit(advance(500), eight));
        assert!(pen.fits(Px::ZERO));
    }

    /// A limit measures a length, so a negative one admits nothing — including
    /// the empty run, which is the boundary a magnitude comparison could get
    /// wrong by testing a signed order instead.
    #[test]
    fn a_negative_limit_admits_nothing() {
        let pen = Pen::new(scale());
        assert!(!pen.fits(Px::new(-1).expect("inside PX_MAX")));
    }

    /// The pen refuses rather than wraps, and refuses without moving. The scale
    /// is the worst case the bounds admit — the largest grid at the largest em
    /// with the largest advance — so this also exercises the arithmetic at the
    /// corner the overflow argument is about.
    #[test]
    fn a_run_that_would_pass_the_bound_is_refused_and_leaves_the_pen_where_it_was() {
        let widest = Scale::new(UPEM_MAX, EM_PX_X64_MAX).expect("the corner is legal");
        let step = advance(ADVANCE_UNITS_MAX);
        let mut pen = Pen::new(widest);
        loop {
            let before = pen;
            if pen.advance(step).is_err() {
                assert_eq!(pen, before, "a refused advance moved the pen");
                break;
            }
            assert!(pen.position().px() <= PX_MAX, "a position passed PX_MAX without refusing");
        }
        assert!(pen.glyphs() > 0, "the corner scale should admit at least one glyph");
    }

    #[test]
    fn a_scale_outside_the_bounds_is_refused_by_name() {
        assert_eq!(Scale::new(0, EM), Err(NotAMetric::UpemOutOfRange));
        assert_eq!(Scale::new(UPEM_MIN - 1, EM), Err(NotAMetric::UpemOutOfRange));
        assert_eq!(Scale::new(UPEM_MAX + 1, EM), Err(NotAMetric::UpemOutOfRange));
        assert_eq!(Scale::new(UPEM_MAX, EM_PX_X64_MAX + 1), Err(NotAMetric::EmSizeOutOfRange));
        assert_eq!(Scale::new(UPEM, EM_PX_X64_MIN - 1), Err(NotAMetric::EmSizeOutOfRange));
        assert!(Scale::new(UPEM_MIN, EM_PX_X64_MIN).is_ok());
    }

    #[test]
    fn an_advance_or_a_position_outside_the_bounds_is_refused_by_name() {
        assert_eq!(
            Advance::in_design_units(ADVANCE_UNITS_MAX + 1),
            Err(NotAMetric::AdvanceOutOfRange)
        );
        assert_eq!(
            Advance::in_design_units(-ADVANCE_UNITS_MAX - 1),
            Err(NotAMetric::AdvanceOutOfRange)
        );
        assert_eq!(Px::new(PX_MAX + 1), Err(NotAMetric::PositionOutOfRange));
        assert_eq!(Px::new(-PX_MAX - 1), Err(NotAMetric::PositionOutOfRange));
        assert!(Advance::in_design_units(ADVANCE_UNITS_MAX).is_ok());
        assert!(Px::new(-PX_MAX).is_ok());
    }

    /// The two types that hold the accumulator print pixels and not the numbers
    /// a consumer would need to round its own way.
    ///
    /// **This test is the witness and not the guard.** The guard is that
    /// `fine::Fine` has no `Debug`, so restoring `#[derive(Debug)]` on either
    /// type does not compile — a test cannot be walked past by an edit that
    /// never runs it, and a missing trait impl cannot be walked past at all.
    /// What this pins is the route the compiler leaves open: a hand-written impl
    /// that prints the pair anyway. The grid is 1 000 and the em is 1 024
    /// sixty-fourths, so either of those appearing is that impl having been
    /// written.
    #[test]
    fn neither_a_scale_nor_a_pen_prints_a_number_that_has_not_been_rounded() {
        for line in [rendered(&scale()), rendered(&run_of(AWKWARD_UNITS, 100))] {
            let text = line.as_str();
            assert!(text.contains("em_px: 16"), "should say what size it is set at: {text}");
            assert!(!text.contains("1000"), "printed the design grid: {text}");
            assert!(!text.contains("1024"), "printed the em in its fine unit: {text}");
            assert!(!text.contains("53"), "printed the accumulated prefix sum: {text}");
        }
        assert!(rendered(&run_of(AWKWARD_UNITS, 100)).as_str().contains("position_px: 834"));
    }

    /// The bound the module doc prices rather than hides. One pen is within half
    /// a pixel however long its run; two pens laid end to end can be further out
    /// than that, because each rounds its own end. Asserting the slack is there
    /// is the honest version of *coarse positions compose* — they do, and this is
    /// what it costs, in the units the rest of the module argues in.
    #[test]
    fn two_runs_compose_with_half_a_pixel_of_slack_each() {
        let one = i64::from(run_of(AWKWARD_UNITS, 100).position().px());
        let exact_numerator = 100 * i64::from(AWKWARD_UNITS) * i64::from(EM);

        let alone = (one * DIVISOR - exact_numerator).abs();
        assert!(2 * alone <= DIVISOR, "one run is within half a pixel: {alone} of {DIVISOR}");

        let composed = ((one + one) * DIVISOR - 2 * exact_numerator).abs();
        assert!(2 * composed > DIVISOR, "two runs should be able to exceed half a pixel");
        assert!(2 * composed <= 2 * DIVISOR, "and never more than half a pixel each");
    }

    /// Reading the pen is not moving it, which is what lets a caller measure a
    /// run and then lay it out and get the same numbers rather than two answers
    /// that happen to agree.
    #[test]
    fn asking_where_the_pen_is_does_not_move_it() {
        let pen = run_of(AWKWARD_UNITS, 7);
        let first = pen.position();
        assert_eq!(first, pen.position());
        assert_eq!(first, pen.position());
        assert_eq!(pen.glyphs(), 7);
    }
}
