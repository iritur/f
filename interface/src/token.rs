// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The typed design-token layer: what a theme may set, what it may set within
//! bounds, and what it may not touch at all.
//!
//! `node.rs` holds token *names* and can resolve none of them, which is
//! deliberate and is where this module picks up. Section 12 of
//! `docs/design/ring-scene-boot.html` states the rule in one sentence — *a
//! token becomes a value only here, and never in the application* — and this
//! file is the only place in the tree where that sentence is true. RFC 0079 is
//! the argument; this is the artefact.
//!
//! # Why a theme is clamped rather than refused
//!
//! The obvious design is to reject a theme that would produce an unreadable
//! interface, and keep the previous one. It is refused here, and the reason is
//! where this layer sits: **underneath every application at once**. A theme is
//! a preference, and a preference that can take the whole machine's interface
//! away is a system-wide outage with a settings screen for a cause. So a theme
//! that asks for something outside the bounds gets the bound, the interface
//! stays usable, and [`Report`] says what happened — which is the part that
//! makes this honest rather than merely safe. A layer that quietly fixed a
//! theme would be one nobody could debug, and the theme's author would never
//! learn that half of it does not survive contact with the checker.
//!
//! The rule that decides clamp-or-refuse is worth stating as a rule rather
//! than case by case: **what has a metric is clamped, what is nominal is
//! refused.** A size that is too small has a nearest legal size, and taking it
//! preserves the direction the theme was pointing. A colour that fails contrast
//! has a nearest legal colour along its own hue. A token name outside the
//! vocabulary has no nearest anything — guessing which token was meant is how a
//! design system acquires a colour nobody chose — so it is dropped, and named
//! in the [`Paint`] that would have carried it.
//!
//! # Why an ink resolves to a different colour on every ground
//!
//! Because contrast is a property of a *pair*, and a token layer that resolved
//! `text` to one colour has already lost the argument: the value it produced is
//! defensible only against the ground it happened to be checked on. So
//! [`Resolved`] holds six inks times three grounds, each clamped independently,
//! and [`Resolved::on`] takes both halves of the pair. This is what catches the
//! attack a naive layer cannot see — two colours that each pass a check in
//! isolation and are illegible together — because there is no point at which a
//! colour is checked in isolation.
//!
//! # Why the floor comes from the node and not from the token it wears
//!
//! `node.rs` puts no restriction on which token a node wears, so a
//! [`Role::Text`] node wearing `edge` is a tree an author can declare — and the
//! first two versions of this module held that node to `edge`'s three to one,
//! because the floor was read off the token. That is a floor an attacker moves:
//! the same node reads at 4 542 under [`Theme::DEFAULT`] and at 3 032 under a
//! theme that sets `surface-1` to `#767676`, and nothing here could tell the
//! difference, because the standard the node was held to travelled with the
//! token the theme had just moved. **A floor an attacker can move is not a
//! floor.**
//!
//! So the floor comes from [`Duty`], which comes from the node's role. A role a
//! caller can read glyphs on owes the text floor whatever token it wears; the
//! one role whose whole content is its own edge owes the non-text floor. An ink
//! the node may not wear is *refused* rather than clamped — the clamp-or-refuse
//! rule applied to a nominal thing, since `edge` moved to four and a half to one
//! is `text` and guessing that is how a design system acquires a colour nobody
//! chose — and [`Paint::refused`] carries the token that was declined, exactly as
//! [`Paint::unknown`] carries a name this vocabulary does not have.
//!
//! # Why a ground is free, and why a region still has an edge
//!
//! The three grounds are themeable with no bound, and the first draft of this
//! module drew the wrong conclusion from that: it never evaluated a ground
//! against a ground at all. A theme could then set `surface-1`, `surface-2` and
//! `field` to one colour, clear every floor in the file, produce no note, and
//! leave every grouped region and every text field with no boundary a reader
//! can see — which is the criterion [`CONTRAST_NONTEXT_X1000`] is taken from,
//! missed by the layer that cites it. `surface-2` is a raised ground *on*
//! `surface-1`; by this module's own words those are a pair.
//!
//! The fix is not to clamp the second ground against the first. That would
//! refuse [`Theme::DEFAULT`]'s own `#F2F2F2` on `#FFFFFF` — a raised surface a
//! shade off the one it sits on is the ordinary idiom, not an attack — and a
//! layer that refused it would be the decorative theme layer the paragraph above
//! argues against. WCAG 1.4.11 does not ask two backgrounds to differ from each
//! other; it asks the region to be *identifiable*, and a rule around it
//! identifies it.
//!
//! So [`Resolved::boundary`] answers the question rather than declining it: for
//! any two grounds it says whether they tell themselves apart, and carries the
//! colour of the rule that must be drawn when they do not — the `edge` ink
//! resolved on the raised ground, which has already cleared the non-text floor
//! against it. The obligation that creates downstream is one line, and it is
//! written into RFC 0079 rather than left implied: **a compositor putting one
//! ground on another, and finding [`Boundary::self_evident`] false, must draw
//! [`Boundary::edge_rgb`] between them.**
//!
//! The promise is one-sided on purpose, and the reason is arithmetic rather than
//! laziness: there are pairs of grounds against which *no* colour in the space
//! reaches three to one on both sides at once. `#515151` and `#9F9F9F` are such
//! a pair — they contrast 2.998 to 1 with each other, just under the floor, so a
//! rule between them is needed; and the best any rule does against the worse of
//! the two sides is 2.646 to 1. `no_colour_separates_two_grounds_on_both_sides`
//! exhibits it by sweeping every luminance, so that a reader proposing a
//! two-sided promise can see what it would cost before proposing it.
//!
//! # Why the operable floor is not one number
//!
//! [`CONTROL_MIN_PT_X10`] is eighteen points and absolute, which is what stops a
//! theme shrinking a control by shrinking the em. It says nothing about the
//! other direction, and the other direction is a hole the same size: at
//! seventy-two points of em — a legal text size at a legal density — an
//! eighteen-point control is a box a quarter the height of the word inside it,
//! and nothing about that is out of bounds. It is the same shape as the derived
//! em one level down, where a legal text size times a legal density is an
//! illegal em, and it needs the same answer.
//!
//! The em is not the whole of what has to fit, and that was the second hole of
//! the same shape. A control is drawn with its own edge; [`Metric::Stroke`] is
//! half an em of rule at its bound; and at the inflated bound that is
//! thirty-six points of rule around a seventy-two-point control — two of its own
//! rules with nothing between them. Both metrics were inside their bounds, and
//! `stroke` was the one number in this module related to nothing it had to fit
//! inside.
//!
//! So [`Resolved::operable_min_pt_x10`] is the larger of the absolute floor and
//! *the em with the two rules that bound it*, and [`Resolved::floor_pt_x10`]
//! applies it: a control is never smaller than a finger, and never smaller than
//! its own word inside its own edge. The same arithmetic gives the separator the
//! floor it did not have — a node whose whole content is a rule is at least as
//! thick as that rule — and a thicker rule therefore makes a *taller* control
//! rather than a smaller one, which is clamp-rather-than-refuse applied to a
//! size.
//!
//! # Why the arithmetic is integers, and where it could be wrong
//!
//! RFC 0004 forbids the floating-point types, so relative luminance and the
//! contrast ratio are fixed point end to end, with the scale in every name:
//! [`LINEAR_X100000`], `luminance_x100000`, [`contrast_x1000`]. The
//! sRGB-to-linear transfer function is a power function and is therefore a
//! 256-entry table rather than a computation — see that constant for its error
//! bound and for how a wrong entry would show up.
//!
//! This is exactly the class of arithmetic that is silently wrong. A contrast
//! ratio that is off by a tenth still looks like a plausible number, still
//! orders pairs correctly, and fails only for pairs near the floor — which are
//! the pairs a designer actually chooses, because that is where a palette is
//! prettiest. So the approximation is *pinned* by test against answers a reader
//! can check by hand rather than trusted: black on white is exactly 21, white
//! on white is exactly 1, and the two adjacent greys that straddle the floor on
//! white must land on opposite sides of it.

use crate::node::{Constraints, Node, Role, TokenName};

/// sRGB channel values, linearised, in hundred-thousandths.
///
/// Entry `c` is the IEC 61966-2-1 transfer function applied to `c / 255` and
/// multiplied by 100 000, rounded to nearest — `c / 12.92` below the knee at
/// 0.04045, and `((c + 0.055) / 1.055)` raised to 2.4 above it. It is a table
/// because that exponent is a power function, this tree has no floating-point
/// type to evaluate one with (RFC 0004), and a polynomial fit would be an
/// approximation whose error nobody could state.
///
/// # Its error, and how a wrong answer would show up
///
/// Each entry is within 0.5 of the exact value in these units, which is the
/// rounding and nothing else. Relative luminance is a convex combination of
/// three entries, so it inherits that half unit and gains at most one more from
/// the integer division: 1.5 in units of 1e-5. Propagated through
/// [`contrast_x1000`] — whose denominator is never below 5 000 and whose result
/// never exceeds 21 000 — that is under eight parts in a thousand, so **a
/// computed ratio is within 0.008 of the exact one**.
///
/// A wrong table would not look wrong. Contrast would still be 21 for black on
/// white, still 1 for a colour on itself, still monotonic in luminance, and
/// still ordered correctly for pairs that are obviously fine or obviously
/// terrible. It would fail only within a hair of the floor. That is why
/// `the_pair_that_straddles_the_floor` pins two adjacent greys on opposite
/// sides of it, rather than asserting a round number in the middle where any
/// wrong table would also pass.
pub const LINEAR_X100000: [u32; 256] = [
    0, 30, 61, 91, 121, 152, 182, 212, 243, 273, 304, 335, 368, 402, 439, 478, 518, 561, 605, 651,
    700, 750, 802, 857, 913, 972, 1033, 1096, 1161, 1229, 1298, 1370, 1444, 1521, 1600, 1681, 1764,
    1850, 1938, 2029, 2122, 2217, 2315, 2416, 2519, 2624, 2732, 2843, 2956, 3071, 3190, 3310, 3434,
    3560, 3689, 3820, 3955, 4092, 4231, 4374, 4519, 4667, 4817, 4971, 5127, 5286, 5448, 5613, 5781,
    5951, 6125, 6301, 6480, 6663, 6848, 7036, 7227, 7421, 7619, 7819, 8022, 8228, 8438, 8650, 8866,
    9084, 9306, 9531, 9759, 9990, 10224, 10462, 10702, 10946, 11193, 11444, 11697, 11954, 12214,
    12477, 12744, 13014, 13287, 13563, 13843, 14126, 14413, 14703, 14996, 15293, 15593, 15896,
    16203, 16513, 16827, 17144, 17465, 17789, 18116, 18447, 18782, 19120, 19462, 19807, 20156,
    20508, 20864, 21223, 21586, 21953, 22323, 22697, 23074, 23455, 23840, 24228, 24620, 25016,
    25415, 25818, 26225, 26636, 27050, 27468, 27889, 28315, 28744, 29177, 29614, 30054, 30499,
    30947, 31399, 31855, 32314, 32778, 33245, 33716, 34191, 34670, 35153, 35640, 36131, 36625,
    37124, 37626, 38133, 38643, 39157, 39676, 40198, 40724, 41254, 41789, 42327, 42869, 43415,
    43966, 44520, 45079, 45641, 46208, 46778, 47353, 47932, 48515, 49102, 49693, 50289, 50888,
    51492, 52100, 52712, 53328, 53948, 54572, 55201, 55834, 56471, 57112, 57758, 58408, 59062,
    59720, 60383, 61050, 61721, 62396, 63076, 63760, 64448, 65141, 65837, 66539, 67244, 67954,
    68669, 69387, 70110, 70838, 71569, 72306, 73046, 73791, 74540, 75294, 76052, 76815, 77582,
    78354, 79130, 79910, 80695, 81485, 82279, 83077, 83880, 84687, 85499, 86316, 87137, 87962,
    88792, 89627, 90466, 91310, 92158, 93011, 93869, 94731, 95597, 96469, 97345, 98225, 99110,
    100000,
];

/// The largest relative luminance any colour has, in hundred-thousandths.
///
/// White's, and it is a constant rather than a comment because it is what makes
/// `every_ground_admits_a_readable_ink` an exhaustion rather than a sample. The
/// three weights in [`luminance_x100000`] sum to 10 000 and no table entry
/// exceeds 100 000, so the quotient cannot either — which means a sweep of every
/// integer from zero to this value covers every ground in the twenty-four-bit
/// space *and a great many that no colour reaches*, with no gap for a minimum to
/// hide in. That is the difference between this bound and the argument it
/// replaced, which reasoned about colours and left the gaps between them
/// unswept.
pub const LUMINANCE_X100000_MAX: u32 = 100_000;

/// The contrast a theme's text must clear, times one thousand: 4.5 to 1.
///
/// # Where the number comes from
///
/// WCAG 2.1 success criterion 1.4.3, *Contrast (Minimum)*, for text below the
/// large-text threshold. It is cited rather than derived: nothing in this
/// project has measured legibility, and inventing a floor here would be
/// choosing a number because it was available.
///
/// # Why 4.5 and not 7
///
/// Because of an arithmetic fact this module depends on, and therefore states
/// rather than assumes. For a ground of relative luminance *L*, the best
/// contrast any foreground can reach is the larger of `(L + 0.05) / 0.05` and
/// `1.05 / (L + 0.05)` — black or white; nothing between them does better. The
/// two are equal where `L + 0.05` is the square root of 0.0525, and there both
/// come to the square root of 21, which is [`REACHABLE_X1000`]: about 4.582.
/// **There exists a ground — `#008909` is one — against which no colour in the
/// twenty-four-bit space reaches 4.583 to 1.**
///
/// So 4.5 is not merely a standard's number here. It is close to the largest
/// floor a *clamp* can always satisfy, and that is what makes clamping a
/// coherent policy rather than a best effort. A floor of 7, the same standard's
/// enhanced level, would be unreachable for a wide band of grounds, and this
/// module would have to refuse those grounds outright — which is the design it
/// declined in its own first paragraph. The margin is eighty-two parts in a
/// thousand. It is thin on purpose and it is written down so that anybody
/// proposing to raise the floor can see what it costs before proposing it.
pub const CONTRAST_TEXT_X1000: u32 = 4_500;

/// The contrast a boundary, rule or control edge must clear: 3 to 1.
///
/// WCAG 2.1 success criterion 1.4.11, *Non-text Contrast*. It is a second floor
/// rather than a relaxation of the first, because a separator held to text's
/// floor is a separator that dominates the text it separates. The number that
/// makes a thing legible and the number that makes it *visible* are different
/// numbers, and collapsing them would be this layer deciding a design question
/// it has no standing to decide.
pub const CONTRAST_NONTEXT_X1000: u32 = 3_000;

/// The contrast that is reachable against **every** ground, times one thousand.
///
/// The square root of 21 — 4 582.5757 in these units — **truncated, to 4 582**.
/// This is the ceiling on what a clamp can promise. It is a constant so that
/// `CONTRAST_TEXT_X1000 < REACHABLE_X1000` is something a test asserts rather
/// than something a reader has to rederive — and so that the day somebody raises
/// a floor above it, [`Note::ContrastUnreachable`] stops being defensive code
/// and starts firing.
///
/// # The rounding this constant got wrong once
///
/// It was 4 583, and the error was not cosmetic. Rounding a *ceiling* up states
/// that every ground admits a ratio it does not: `#008909` has a relative
/// luminance of 17 911 in [`LINEAR_X100000`]'s units, and against it black and
/// white both come to exactly 4 582 by this module's own arithmetic. The
/// assertion that no ground beats the ceiling was therefore false for a real
/// colour, and the sweep that was supposed to establish it did not find the
/// counterexample — because it swept 256 greys and a stride over the cube, and
/// the minimum lies strictly between the greys `#757575` (luminance 17 789) and
/// `#767676` (18 116). *Landing in the interval two swept values straddle is
/// not landing on a swept value*, which is the hole that argument had.
///
/// `every_ground_admits_a_readable_ink` now sweeps every integer luminance from
/// zero to [`LUMINANCE_X100000_MAX`] instead of every colour, and asserts
/// equality rather than an inequality. Contrast depends on a ground only
/// through its luminance, the sweep covers a superset of the luminances colours
/// reach, and a superset with no gaps cannot step over a minimum. Setting this
/// constant back to 4 583 turns that test red.
pub const REACHABLE_X1000: u32 = 4_582;

/// How many blend steps a clamp may take between an ink and its pole.
///
/// Sixty-four, which makes the smallest move one sixty-fourth of the distance to
/// the pole — about **sixteen** parts in a thousand, and at most four values in
/// any one channel — and bounds the work at sixty-four ratio computations per
/// pair, of which there are eighteen. The last step *is* the pole, which is why
/// the clamp always terminates with an answer.
///
/// The granularity is stated and no claim is made about whether anybody can see
/// it. An earlier draft of this line said four parts in a thousand and concluded
/// *below what anybody sees*; the arithmetic was wrong by a factor of four, and
/// the conclusion was a perceptual claim this project has not measured and has
/// no instrument for — the kind of number `claims/README.md` exists to keep out
/// of a document. What would reverse the count is evidence in the other
/// direction: a clamped ink that is visibly a different colour from the one the
/// theme asked for, where a finer step would have been close enough to pass
/// unremarked. That is a measurement `E3-B02` could take and this module cannot.
pub const CLAMP_STEPS: u32 = 64;

/// The smallest an operable node may be along its flow, in tenths of a point.
///
/// Eighteen points, which is twenty-four CSS pixels: WCAG 2.2 success criterion
/// 2.5.8, *Target Size (Minimum)*. It is **not themeable, and not expressed in
/// ems**, and both halves of that are the decision. Not themeable, because a
/// theme that can shrink a control to a point has removed the control while
/// leaving it in the tree, where every projection will faithfully report that
/// it is there. Not in ems, because the em is themeable down to nine points and
/// a floor that scaled with it would be a floor a theme could lower by the back
/// door — which is the whole mechanism this constant exists to close.
///
/// # It is a floor and not *the* floor
///
/// Both halves above argue the shrinking direction, and for a while this
/// constant was used as though the shrinking direction were the only one. It is
/// not: the em is themeable *up* to seventy-two points, and a legal text size at
/// a legal density reaches it, so an eighteen-point control can be asked to hold
/// seventy-two-point text. That is unlayoutable by inspection and neither metric
/// is out of bounds. The rule drawn around the control is a third number of the
/// same kind, and at its own bound it is half an em thick.
/// [`Resolved::operable_min_pt_x10`] is therefore the larger of this number and
/// the em with both of its rules, and it — not this constant — is what
/// [`Resolved::floor_pt_x10`] applies. This stays absolute so that the *lower*
/// end cannot move; the upper end is the em's business and the stroke's, because
/// at that end the question is no longer whether a finger can reach the control
/// but whether the control can contain its own word inside its own edge.
pub const CONTROL_MIN_PT_X10: i32 = 180;

/// How many font families a theme may name before the last resort.
pub const FONT_PREFERENCES_MAX: usize = 3;

/// The longest font family name, in bytes.
pub const FAMILY_NAME_MAX: usize = 32;

/// How many notes one [`Report`] holds before it starts counting instead.
///
/// Thirty-two, against a worst case of twenty-six — eighteen ink-and-ground
/// pairs, five metrics, three font preferences — so a theme that is wrong in
/// every way this module can see still fits. The margin is not generous by
/// accident: a report that truncated silently would reintroduce the silence
/// this module exists to remove, so [`Report::dropped`] counts what did not fit
/// and [`Report::is_clean`] refuses to call such a report clean.
pub const NOTES_MAX: usize = 32;

/// How many ground tokens there are.
pub const GROUND_COUNT: usize = 3;

/// How many ink tokens there are.
pub const INK_COUNT: usize = 6;

/// How many colour tokens there are altogether.
pub const TOKEN_COUNT: usize = GROUND_COUNT + INK_COUNT;

/// How many metrics there are, the derived one included.
pub const METRIC_COUNT: usize = 5;

/// A colour, and deliberately not a colour with an alpha channel.
///
/// An alpha channel would be the shortest route around everything below it: the
/// pair this module checks would stop being the pair that is painted, and a
/// theme could defeat a contrast floor without changing a single colour value.
/// Compositing is the scene's business — part II of the design document — and
/// it is downstream of the point at which a token has become a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    /// Red, as sRGB encodes it.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// The darker of the two poles a clamp can move an ink towards.
    pub const BLACK: Self = Self::new(0x00, 0x00, 0x00);

    /// The lighter one.
    pub const WHITE: Self = Self::new(0xFF, 0xFF, 0xFF);

    /// A colour from its three sRGB channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Relative luminance in hundred-thousandths, by the coefficients WCAG 2.1
/// states: 0.2126, 0.7152 and 0.0722 over the linearised channels.
///
/// Public because it is the quantity contrast actually depends on, and because
/// the ceiling on what a clamp can promise is a statement about *luminances*
/// rather than about colours — see [`best_reachable_x1000`], which is how that
/// ceiling is established without sampling the colour space.
#[must_use]
pub const fn luminance_x100000(colour: Rgb) -> u32 {
    let r = LINEAR_X100000[colour.r as usize] as u64;
    let g = LINEAR_X100000[colour.g as usize] as u64;
    let b = LINEAR_X100000[colour.b as usize] as u64;
    // The three weights sum to 10 000, so the quotient is bounded by 100 000 and
    // the whole expression stays inside a `u32` with three decimal orders to
    // spare. The widening is here so that an edit to the weights or to the
    // table's scale is caught by the bound above rather than by a wrap.
    ((2126 * r + 7152 * g + 722 * b) / 10_000) as u32
}

/// The contrast ratio between two colours, times one thousand.
///
/// Twenty-one thousand for black on white, one thousand for a colour on itself,
/// and never outside that range. The order of the arguments does not matter:
/// the ratio is defined over the lighter and the darker rather than over a
/// foreground and a background, which is why a pair can be checked before
/// anybody has decided which half is the text.
#[must_use]
pub const fn contrast_x1000(a: Rgb, b: Rgb) -> u32 {
    contrast_of_luminance_x1000(luminance_x100000(a), luminance_x100000(b))
}

/// The same ratio, over two relative luminances rather than two colours.
///
/// Split out of [`contrast_x1000`] rather than duplicated, because the ceiling
/// argument needs to ask the question of a luminance no colour has — every
/// integer between zero and [`LUMINANCE_X100000_MAX`] — and a second copy of
/// this expression is a second copy that could come to disagree with the first.
#[must_use]
pub const fn contrast_of_luminance_x1000(a_x100000: u32, b_x100000: u32) -> u32 {
    let la = a_x100000 as u64;
    let lb = b_x100000 as u64;
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    // 0.05 in the table's units is 5 000. It is the term that keeps the ratio
    // finite for black on black, and it is why the denominator cannot be zero
    // however wrong the table is.
    (((hi + 5_000) * 1_000) / (lo + 5_000)) as u32
}

/// The best contrast any colour reaches against a ground of this luminance.
///
/// Black or white; nothing between them does better, which is the arithmetic
/// fact [`CONTRAST_TEXT_X1000`] documents and the whole clamp-rather-than-refuse
/// policy rests on. The minimum of this function over every luminance a ground
/// can have is [`REACHABLE_X1000`], and it is a function rather than a comment
/// so that the minimum can be taken by a sweep with no gaps in it instead of by
/// a sample of the colour space that had one.
#[must_use]
pub const fn best_reachable_x1000(ground_x100000: u32) -> u32 {
    let to_black = contrast_of_luminance_x1000(ground_x100000, 0);
    let to_white = contrast_of_luminance_x1000(ground_x100000, LUMINANCE_X100000_MAX);
    if to_black >= to_white { to_black } else { to_white }
}

/// One channel moved `step` sixty-fourths of the way from `from` to `to`.
const fn toward(from: u8, to: u8, step: u32) -> u8 {
    let from = from as i32;
    let to = to as i32;
    // Both endpoints are channel values and `step` never exceeds `CLAMP_STEPS`,
    // so the result is inside 0..=255 by construction.
    (from + (to - from) * step as i32 / CLAMP_STEPS as i32) as u8
}

/// What happened to an ink that was asked to clear a floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Raised {
    /// The colour to use. Equal to the ink that went in when no move was needed,
    /// which is the common case and the one that must cost nothing.
    pub colour: Rgb,
    /// What it achieves against the ground it was checked on.
    pub contrast_x1000: u32,
    /// How many sixty-fourths it travelled. Zero means the theme's own colour
    /// survived untouched.
    pub steps: u8,
    /// Did it get there? False only for a floor above [`REACHABLE_X1000`], which
    /// this module's own floors are not.
    pub reached: bool,
}

/// Move `ink` along the line to whichever pole helps, stopping at the first step
/// that clears `floor_x1000`.
///
/// # Why the floor is a parameter
///
/// So that a test can ask what happens *above* [`REACHABLE_X1000`] and watch
/// [`Raised::reached`] go false. A failure path no test can reach is a comment.
///
/// # Why a linear scan and not a binary search
///
/// Because contrast along the blend is **not monotonic**. If the ink starts
/// lighter than the ground and the helpful pole is black, the ratio falls to one
/// as the ink passes through the ground's own luminance and climbs again after
/// it. A binary search over that is wrong in a way that returns a plausible
/// colour, which is the failure mode this whole module is written against. The
/// scan costs at most sixty-four ratio computations, and it finds the *smallest*
/// move that works — which is also the answer that disturbs the theme least.
///
/// # Why towards a pole rather than towards a computed target
///
/// Solving for the exact luminance that meets the floor and then finding a
/// colour with that luminance means choosing among a plane of colours, and this
/// module has no standing to choose a hue on a designer's behalf. Blending
/// towards black preserves the ratios between channels exactly; blending towards
/// white desaturates, which is a visible change and is precisely why
/// [`Note::ContrastRaised`] exists. The theme's intent survives in the direction
/// it pointed, and the report says how far it was pushed.
#[must_use]
pub fn raise_to_floor(ink: Rgb, ground: Rgb, floor_x1000: u32) -> Raised {
    let was = contrast_x1000(ink, ground);
    if was >= floor_x1000 {
        return Raised { colour: ink, contrast_x1000: was, steps: 0, reached: true };
    }
    let to_black = contrast_x1000(ground, Rgb::BLACK);
    let to_white = contrast_x1000(ground, Rgb::WHITE);
    let pole = if to_black >= to_white { Rgb::BLACK } else { Rgb::WHITE };
    let mut step = 1;
    while step <= CLAMP_STEPS {
        let candidate = Rgb::new(
            toward(ink.r, pole.r, step),
            toward(ink.g, pole.g, step),
            toward(ink.b, pole.b, step),
        );
        let now = contrast_x1000(candidate, ground);
        if now >= floor_x1000 {
            let steps = step as u8;
            return Raised { colour: candidate, contrast_x1000: now, steps, reached: true };
        }
        step += 1;
    }
    // The last step *is* the pole, so this is the best any colour can do against
    // this ground. Reaching here means the floor was set above what the ground
    // admits, and the caller is owed that fact rather than a silent best effort.
    let best = contrast_x1000(pole, ground);
    Raised { colour: pole, contrast_x1000: best, steps: CLAMP_STEPS as u8, reached: false }
}

/// The two floors a pair can be held to, passed rather than read from a pair of
/// constants.
///
/// # Why these are an argument
///
/// Because the branch that says *no colour reaches this floor against this
/// ground* has to be reachable by something other than an argument that it would
/// work. Both of this module's own floors are below [`REACHABLE_X1000`], so with
/// them [`resolve`] cannot produce [`Note::ContrastUnreachable`] however hostile
/// the theme — and a failure path no test can reach is a comment with a type on
/// it. [`resolve_with`] takes the floors, so a test can raise one past the
/// ceiling and watch the note arrive through the same code that would produce it
/// on the day somebody argues for WCAG's enhanced level. That day is RFC 0079's
/// stated reversal condition, and it is now something this module can be asked
/// to do rather than something it says it would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Floors {
    /// What a pair a caller reads glyphs in must clear.
    pub text_x1000: u32,
    /// What a pair a caller only has to see must clear.
    pub nontext_x1000: u32,
}

impl Floors {
    /// The two this module ships: [`CONTRAST_TEXT_X1000`] and
    /// [`CONTRAST_NONTEXT_X1000`].
    pub const STANDARD: Self =
        Self { text_x1000: CONTRAST_TEXT_X1000, nontext_x1000: CONTRAST_NONTEXT_X1000 };

    /// The floor a duty owes.
    #[must_use]
    pub const fn of(self, duty: Duty) -> u32 {
        match duty {
            Duty::Read => self.text_x1000,
            Duty::See => self.nontext_x1000,
        }
    }

    /// The larger of two duties' floors.
    ///
    /// A painted pair owes the duty of the node *and* the duty of the token it
    /// wears, and owing both means the larger of the two. It is a maximum rather
    /// than the node's alone so that the relation only ever tightens: a node
    /// that need only be seen, wearing an ink that must be read, gets the ink's
    /// floor rather than an excuse to lower it.
    #[must_use]
    pub const fn stricter(self, one: Duty, other: Duty) -> u32 {
        let one = self.of(one);
        let other = self.of(other);
        if one >= other { one } else { other }
    }
}

/// What a node asks of the ink it is painted in, and therefore which floor the
/// pair owes.
///
/// This is the rule the first two versions of this module did not have, and it
/// is worth stating as a rule rather than as a repair: **the floor belongs to
/// the node, not to the token it wears.** `node.rs`'s `check` puts no
/// restriction on `style`, so a [`Role::Text`] node wearing `edge` is a
/// declarable tree, and holding it to `edge`'s three to one made the standard a
/// thing a theme could move — 4 542 under [`Theme::DEFAULT`] and 3 032 under a
/// theme that sets `surface-1` to `#767676`.
///
/// There are two duties because the standard has two criteria, and which one a
/// node owes is a question about the node: can a caller end up reading glyphs
/// against this node's ground? For twenty-one of the twenty-two roles the answer
/// is yes — [`Role::Image`] included, because a projection is permitted to
/// describe a picture instead of drawing it and a description is glyphs. The
/// exception is the one role whose whole content *is* its own edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duty {
    /// A caller reads glyphs in it. WCAG 1.4.3, [`CONTRAST_TEXT_X1000`].
    Read,
    /// A caller only has to see that it is there. WCAG 1.4.11,
    /// [`CONTRAST_NONTEXT_X1000`].
    See,
}

impl Duty {
    /// Both of them.
    pub const ALL: [Self; 2] = [Self::Read, Self::See];

    /// What a report calls it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::See => "see",
        }
    }

    /// The floor it owes under [`Floors::STANDARD`].
    #[must_use]
    pub const fn floor_x1000(self) -> u32 {
        Floors::STANDARD.of(self)
    }

    /// What a node of this role asks of its ink.
    ///
    /// The match is exhaustive over all twenty-two roles, for
    /// [`Token::ink_for`]'s reason: a twenty-third role fails to compile here,
    /// and whoever adds it is asked whether a caller can read anything on it
    /// rather than inheriting an answer from a wildcard arm.
    ///
    /// This match and `ink_for` agree today — the one role that is
    /// [`Duty::See`] is the one role whose default ink is [`Token::Edge`] — and
    /// the agreement is load-bearing rather than tidy: [`Resolved::paint`]
    /// answers a refused ink with the role's own ink, so if the two ever
    /// disagreed the fallback could land on an ink that fails the floor it fell
    /// back for. `a_node_that_wears_nothing_is_already_at_its_own_floor` is
    /// where that is asserted.
    #[must_use]
    pub const fn of(role: Role) -> Self {
        match role {
            // The one role whose content is its own edge, which is also why
            // `ink_for` gives it `edge`. Holding a rule to text's floor would
            // make every separator darker than the text it separates, which is
            // the design question `CONTRAST_NONTEXT_X1000` refuses to decide by
            // collapsing the two floors into one.
            Role::Separator => Self::See,
            Role::Surface
            | Role::Group
            | Role::List
            | Role::Tree
            | Role::Item
            | Role::Table
            | Role::Row
            | Role::Cell
            | Role::Label
            | Role::Text
            | Role::Image
            | Role::Status
            | Role::Command
            | Role::Toggle
            | Role::Entry
            | Role::Choice
            | Role::Number
            | Role::Canvas
            | Role::Track
            | Role::Clip
            | Role::Marker => Self::Read,
        }
    }
}

/// The ground an ink is read on by default, and what the ink is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pairing {
    /// Which ground this ink is for, when the node declares none.
    pub ground: Token,
    /// What this ink is for, which is what decides its floor. A duty rather than
    /// a number, so that there is one place in this module where a floor is
    /// written down and [`Floors`] is it.
    pub duty: Duty,
}

impl Pairing {
    /// The floor this ink owes under [`Floors::STANDARD`].
    #[must_use]
    pub const fn floor_x1000(self) -> u32 {
        self.duty.floor_x1000()
    }
}

/// A colour token: the whole of what a node may wear, and the whole of what a
/// theme may colour.
///
/// Closed, and **not** `#[non_exhaustive]`, for `node.rs`'s reason applied one
/// layer down: a wildcard arm is where a token nobody resolved goes to become a
/// default grey, and a tenth token should break the build of everything that
/// resolves tokens rather than be absorbed by it. [`Token::from_name`] is
/// derived from [`Token::ALL`], so no string becomes a token by passing through
/// it.
///
/// Three of the nine are grounds and six are inks, and that split is the first
/// of this module's three categories: **a ground is themeable with no bound at
/// all**. Every twenty-four-bit value is a legal ground, because no ground is
/// unreadable on its own — readability is a property of a pair, and the ink is
/// the half that moves.
///
/// Two grounds *are* a pair, and that is the sentence this doc comment was
/// missing. It does not make either of them movable: what it makes is an
/// obligation to say whether they part, which is [`Resolved::boundary`], and a
/// rule to draw when they do not. Spending the one degree of freedom on the ink rather
/// than the ground is what keeps the theme layer from becoming decorative: a
/// theme still chooses the whole character of the palette, and what it does not
/// get to choose is a foreground that cannot be read on the background it has
/// just chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    /// The ground a surface presents on. Free.
    Surface1,
    /// A raised ground, for a region that sits on the first. Free, and the pair
    /// it makes with whatever it sits on is [`Resolved::boundary`]'s business.
    Surface2,
    /// The ground of something a caller types into. Free, and the same.
    Field,
    /// Text that carries the content. Held to the text floor.
    Text,
    /// Text that is secondary — a status, a hint, a unit. Held to the text
    /// floor, and that is the entry worth arguing with: *muted* is a designer's
    /// word for *lower contrast*, and this token exists so that the wish is
    /// expressible while the floor is not negotiable.
    TextMuted,
    /// Text that is being emphasised, on the raised ground.
    Emphasis,
    /// A rule, a boundary, the edge of a control. Held to the non-text floor —
    /// which is why a node that can carry glyphs may not wear it, and gets its
    /// role's ink instead with this one named in [`Paint::refused`]. See
    /// [`Duty`].
    Edge,
    /// Text inside a field.
    FieldText,
    /// Text inside a field that has gone wrong.
    FieldDanger,
}

impl Token {
    /// Every token, indexed by [`Token::index`].
    pub const ALL: [Self; TOKEN_COUNT] = [
        Self::Surface1,
        Self::Surface2,
        Self::Field,
        Self::Text,
        Self::TextMuted,
        Self::Emphasis,
        Self::Edge,
        Self::FieldText,
        Self::FieldDanger,
    ];

    /// Its position in [`Token::ALL`]. Dense, and the match is exhaustive, so a
    /// tenth token cannot be added without being given one.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Surface1 => 0,
            Self::Surface2 => 1,
            Self::Field => 2,
            Self::Text => 3,
            Self::TextMuted => 4,
            Self::Emphasis => 5,
            Self::Edge => 6,
            Self::FieldText => 7,
            Self::FieldDanger => 8,
        }
    }

    /// How a node spells it. Every one of these satisfies `TokenName`'s grammar,
    /// which is asserted by test rather than checked by eye.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Surface1 => "surface-1",
            Self::Surface2 => "surface-2",
            Self::Field => "field",
            Self::Text => "text",
            Self::TextMuted => "text.muted",
            Self::Emphasis => "emphasis",
            Self::Edge => "edge",
            Self::FieldText => "field.text",
            Self::FieldDanger => "field.danger",
        }
    }

    /// The token a name denotes, if the vocabulary has one.
    ///
    /// Derived from [`Token::ALL`] rather than written as a second match, so
    /// there is no name that resolves here and nowhere else.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|token| token.name() == name)
    }

    /// The ground this ink is read on by default and what it is for, or `None`
    /// if this token *is* a ground.
    ///
    /// *What it is for* rather than *the floor it owes*, because the floor of a
    /// painted pair is not a property of the token alone — see [`Duty`]. This
    /// says what the ink is held to when nothing else asks for more.
    #[must_use]
    pub const fn pairing(self) -> Option<Pairing> {
        let pairing = match self {
            Self::Surface1 | Self::Surface2 | Self::Field => return None,
            Self::Text | Self::TextMuted => Pairing { ground: Self::Surface1, duty: Duty::Read },
            Self::Emphasis => Pairing { ground: Self::Surface2, duty: Duty::Read },
            Self::Edge => Pairing { ground: Self::Surface1, duty: Duty::See },
            Self::FieldText | Self::FieldDanger => {
                Pairing { ground: Self::Field, duty: Duty::Read }
            }
        };
        Some(pairing)
    }

    /// Is this a ground rather than an ink?
    #[must_use]
    pub const fn is_ground(self) -> bool {
        self.pairing().is_none()
    }

    /// Which of the three ground columns this token is, if it is a ground.
    #[must_use]
    pub const fn ground_slot(self) -> Option<usize> {
        match self {
            Self::Surface1 => Some(0),
            Self::Surface2 => Some(1),
            Self::Field => Some(2),
            Self::Text
            | Self::TextMuted
            | Self::Emphasis
            | Self::Edge
            | Self::FieldText
            | Self::FieldDanger => None,
        }
    }

    /// The ink a role takes when its node wears no token of its own.
    ///
    /// The match is exhaustive over all twenty-two roles, which is the point: a
    /// twenty-third role added to `node.rs` fails to compile here, and whoever
    /// adds it is asked what colour it is rather than discovering later that it
    /// is whatever a wildcard arm said.
    #[must_use]
    pub const fn ink_for(role: Role) -> Self {
        match role {
            // The one structural role whose whole content is its own edge, and
            // therefore the one that takes the non-text floor.
            Role::Separator => Self::Edge,
            // A status reads as secondary by default, which is the single
            // stylistic opinion this module encodes. It is reversible by the
            // node: a status wearing `text` gets `text`.
            Role::Status => Self::TextMuted,
            // Anything a caller types into is read on the field's ground, so its
            // default ink must be the one paired with that ground.
            Role::Entry => Self::FieldText,
            Role::Surface
            | Role::Group
            | Role::List
            | Role::Tree
            | Role::Item
            | Role::Table
            | Role::Row
            | Role::Cell
            | Role::Label
            | Role::Text
            | Role::Image
            | Role::Command
            | Role::Toggle
            | Role::Choice
            | Role::Number
            | Role::Canvas
            | Role::Track
            | Role::Clip
            | Role::Marker => Self::Text,
        }
    }
}

/// A number a theme sets, or one this module derives from two that it did.
///
/// This is the second category — **range-checked**: a theme may set it, and
/// outside the bounds the bound wins and [`Note::MetricClamped`] says so. The
/// bounds are targets rather than measurements; nothing in `E3` has been
/// measured, and the claims registry carries them on those terms.
///
/// Metrics are deliberately **not** [`Token`]s, so a node cannot wear one. A
/// node that could say `space` in its style set would be a node saying how much
/// room to leave, which is `x = 340` arriving by the one route `node.rs` left
/// open. `a_node_cannot_wear_a_metric` asserts the two vocabularies are
/// disjoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// The text size a theme asks for, in tenths of a point.
    TextSize,
    /// The display scale, in thousandths. One thousand is unscaled.
    Density,
    /// The spacing unit, in hundredths of the resolved em.
    Space,
    /// The thickness of a rule or a control's edge, in hundredths of the em.
    Stroke,
    /// The resolved em, in tenths of a point: [`Metric::TextSize`] times
    /// [`Metric::Density`]. **A theme cannot set this one**, and that is why it
    /// is a variant rather than a private local — a legal text size multiplied
    /// by a legal density can be an illegal em, so the product is checked as
    /// well as the factors, and the clamp on the product is reported under its
    /// own name rather than folded into one of theirs.
    Em,
}

impl Metric {
    /// Every metric, indexed by [`Metric::index`].
    pub const ALL: [Self; METRIC_COUNT] =
        [Self::TextSize, Self::Density, Self::Space, Self::Stroke, Self::Em];

    /// Its position in [`Metric::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::TextSize => 0,
            Self::Density => 1,
            Self::Space => 2,
            Self::Stroke => 3,
            Self::Em => 4,
        }
    }

    /// What a report calls it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TextSize => "text.size",
            Self::Density => "density",
            Self::Space => "space",
            Self::Stroke => "stroke",
            Self::Em => "em",
        }
    }

    /// The smallest value this metric may take.
    #[must_use]
    pub const fn low(self) -> i32 {
        match self {
            // Nine points. Below it the glyphs of a script with a large
            // character set stop being distinguishable at ordinary densities,
            // and a theme that asked for zero asked for no interface at all.
            Self::TextSize | Self::Em => 90,
            // Half scale. Under it a theme is not adapting to a display, it is
            // shrinking the machine.
            Self::Density => 500,
            // Zero. A dense theme with no padding is ugly and is not broken, so
            // this bound exists to refuse the negative value — which is not a
            // tighter layout, it is an overlapping one.
            Self::Space => 0,
            // Two hundredths of an em, and the number is derived rather than
            // chosen: [`Resolved::stroke_pt_x10`] is this metric times the em
            // over a hundred, [`Self::Em`]'s own floor is ninety, and one
            // hundredth of that is nought point nine — which truncates to zero.
            // Two is the smallest value whose product cannot reach zero at any
            // em this layer will resolve.
            //
            // It shared [`Self::Space`]'s arm and therefore its floor until
            // 2026-09-15, and the comment above — which is about padding and
            // says nothing about rules — was the whole justification a stroke
            // of zero ever had. The two metrics are not alike in the direction
            // that matters. A theme with no space is dense; a theme with no
            // stroke has **deleted an obligation this module hands somebody
            // else**. [`Boundary`] carries a colour and no thickness, so the
            // rule owed on every pair whose `self_evident` is false has exactly
            // one source for how thick it is, and it is this. And
            // [`Resolved::floor_pt_x10`] gives a [`Duty::See`] node its own
            // rule's thickness as a minimum height, so a stroke of zero puts
            // the separator back to a floor of nought — the state that floor
            // was added to close, reachable again through the other end of the
            // same number.
            //
            // The inflation direction was already answered, by
            // [`Resolved::operable_min_pt_x10`] counting two rules into what a
            // control must contain. This is the collapse direction, and it went
            // unanswered for the ordinary reason: every hostile theme in the
            // module attacked by making things enormous. [`HOSTILE_ERASED`] is
            // the one that attacks by making them vanish.
            //
            // What would reverse it: a display on which a rule is drawn by a
            // mechanism that does not take a thickness from here — a hairline
            // the device defines, or a boundary expressed as a shadow. Then the
            // thickness stops being this layer's to guarantee and the floor
            // belongs wherever the substitute is decided.
            Self::Stroke => 2,
        }
    }

    /// The largest value this metric may take.
    #[must_use]
    pub const fn high(self) -> i32 {
        match self {
            // Seventy-two points. Past it a control's own text is larger than
            // the controls around it and no arrangement of them fits.
            Self::TextSize | Self::Em => 720,
            // Four times. Past it the scale is not a display's.
            Self::Density => 4_000,
            // Four ems between siblings is already a layout with more space than
            // content in it.
            Self::Space => 400,
            // Half an em of rule. The bound is not what keeps a control
            // layoutable and this line used to say it was: at the bound the
            // rule is half the em, and two of them are the whole of an
            // eighteen-point control at a seventy-two-point em. What answers
            // that is `Resolved::operable_min_pt_x10`, which counts the rules,
            // so what this number bounds is how heavy a rule may be and not
            // whether there is room for one.
            Self::Stroke => 50,
        }
    }

    /// May a theme set this directly?
    ///
    /// False for [`Metric::Em`] alone, which is derived. A metric a theme could
    /// set *and* this module derives would be one number with two sources, and
    /// the two would disagree on the first day somebody changed either.
    #[must_use]
    pub const fn is_themeable(self) -> bool {
        !matches!(self, Self::Em)
    }
}

/// Why a font preference was not a family name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotAFamily {
    /// Nothing was named.
    Empty,
    /// Longer than [`FAMILY_NAME_MAX`].
    TooLong,
    /// It holds a byte outside printable ASCII — a control character, or
    /// anything this module could not show a reader when reporting the refusal.
    Unprintable,
    /// It is printable and holds no letter, which is how a font stack is spelled
    /// when it is meant to look populated and resolve to nothing.
    NoLetter,
}

/// A font family a theme asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamilyName {
    /// The bytes, zero-filled past `len`.
    bytes: [u8; FAMILY_NAME_MAX],
    /// How many of them are the name.
    len: u8,
}

impl FamilyName {
    /// Check `name` and keep it.
    ///
    /// # Errors
    ///
    /// [`NotAFamily`], which says which clause refused it. The last clause is
    /// the one doing the work: a name of nothing but spaces and punctuation is
    /// what a hostile theme supplies when it wants a full-looking stack that
    /// resolves to nothing.
    pub const fn new(name: &str) -> Result<Self, NotAFamily> {
        let src = name.as_bytes();
        if src.is_empty() {
            return Err(NotAFamily::Empty);
        }
        if src.len() > FAMILY_NAME_MAX {
            return Err(NotAFamily::TooLong);
        }
        let mut bytes = [0u8; FAMILY_NAME_MAX];
        let mut at = 0;
        let mut letters = 0;
        while at < src.len() {
            let byte = src[at];
            if !byte.is_ascii_graphic() && byte != b' ' {
                return Err(NotAFamily::Unprintable);
            }
            if byte.is_ascii_alphabetic() {
                letters += 1;
            }
            bytes[at] = byte;
            at += 1;
        }
        if letters == 0 {
            return Err(NotAFamily::NoLetter);
        }
        Ok(Self { bytes, len: src.len() as u8 })
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The error arm is unreachable: the grammar admits printable ASCII only.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or_default()
    }

    /// Its length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Never, for a constructed name.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The families to try, in order, ending in one a theme cannot remove.
///
/// This is the third category — **what a theme cannot break** — in its purest
/// form, and it is the one with nothing to report: the last resort is appended
/// structurally, so there is no state in which the stack is empty and therefore
/// no refusal to record. A guarantee that has to announce itself is a guarantee
/// that can be absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontStack {
    /// What the theme asked for, in order, with the refused entries removed.
    preferred: [Option<FamilyName>; FONT_PREFERENCES_MAX],
    /// How many of them survived.
    len: u8,
}

impl FontStack {
    /// The family that is always last and is never a theme's to choose.
    ///
    /// A name rather than a face, because this crate has no font machinery and
    /// should not pretend to. What makes it a *last* resort is that the
    /// projection is required to have something for it, and `E3-B03` is where
    /// that requirement lands.
    pub const LAST_RESORT: &'static str = "system-ui";

    /// What a theme that named nothing usable gets.
    pub const BARE: Self = Self { preferred: [None; FONT_PREFERENCES_MAX], len: 0 };

    /// Every family to try, in order, the last resort included.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.preferred[..self.len as usize]
            .iter()
            .flatten()
            .map(FamilyName::as_str)
            .chain(core::iter::once(Self::LAST_RESORT))
    }

    /// How many families there are. Never zero.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize + 1
    }

    /// Never. Present because a length without it is half an interface, and
    /// because the answer being a constant is the guarantee.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// How many of the theme's own preferences survived.
    #[must_use]
    pub const fn preferences(&self) -> usize {
        self.len as usize
    }

    /// Append a family that has already been checked.
    fn push(&mut self, family: FamilyName) {
        if (self.len as usize) < FONT_PREFERENCES_MAX {
            self.preferred[self.len as usize] = Some(family);
            self.len += 1;
        }
    }
}

/// What a theme asks for, before anything has checked it.
///
/// Every field here is **raw**. The metrics are wide signed integers and the
/// font preferences are ordinary strings, so a theme can express zero, a
/// negative, the largest value the type holds, and a name that is not a name.
/// That is deliberate, and it is the only way the demonstration means anything:
/// a theme type that could not spell a hostile value would have moved the
/// refusal into the type system, where it looks like a proof and is actually a
/// missing test. Narrowing happens in [`resolve`] and nowhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The ground a surface presents on.
    pub surface_1: Rgb,
    /// The raised ground.
    pub surface_2: Rgb,
    /// The ground of a field.
    pub field: Rgb,
    /// Content text.
    pub text: Rgb,
    /// Secondary text.
    pub text_muted: Rgb,
    /// Emphasised text.
    pub emphasis: Rgb,
    /// Rules and control edges.
    pub edge: Rgb,
    /// Text inside a field.
    pub field_text: Rgb,
    /// Text inside a field that has gone wrong.
    pub field_danger: Rgb,
    /// Text size in tenths of a point.
    pub text_size_pt_x10: i32,
    /// Display scale in thousandths.
    pub density_x1000: i32,
    /// Spacing unit in hundredths of the em.
    pub space_em_x100: i32,
    /// Rule thickness in hundredths of the em.
    pub stroke_em_x100: i32,
    /// Font families to try, most preferred first.
    ///
    /// An **empty** entry is absence rather than a fault, and is skipped without
    /// a note: the array is fixed, a theme naming one family has to spell the
    /// other two slots somehow, and reporting that as a refusal would make every
    /// reasonable theme dirty and [`Report::is_clean`] worthless. Anything else
    /// that is not a family name is dropped *with* a note, which is why
    /// [`NotAFamily::Empty`] is reachable from [`FamilyName::new`] and not from
    /// [`resolve`].
    pub fonts: [&'static str; FONT_PREFERENCES_MAX],
}

impl Theme {
    /// The theme this module would write if nobody else did.
    ///
    /// It exists to be resolved with an empty [`Report`], which
    /// `a_theme_this_layer_agrees_with_is_not_touched` asserts: a token layer
    /// whose own default needs clamping has a floor or a bound that is wrong,
    /// rather than a default that is unlucky.
    pub const DEFAULT: Self = Self {
        surface_1: Rgb::new(0xFF, 0xFF, 0xFF),
        surface_2: Rgb::new(0xF2, 0xF2, 0xF2),
        field: Rgb::new(0xFF, 0xFF, 0xFF),
        text: Rgb::new(0x1A, 0x1A, 0x1A),
        text_muted: Rgb::new(0x59, 0x59, 0x59),
        emphasis: Rgb::new(0x0B, 0x3D, 0x91),
        edge: Rgb::new(0x76, 0x76, 0x76),
        field_text: Rgb::new(0x1A, 0x1A, 0x1A),
        field_danger: Rgb::new(0xA4, 0x00, 0x0F),
        text_size_pt_x10: 105,
        density_x1000: 1_000,
        space_em_x100: 50,
        stroke_em_x100: 6,
        fonts: ["Inter", "Noto Sans", ""],
    };

    /// What this theme says a token's colour is, before any check.
    ///
    /// The match is exhaustive, so a tenth [`Token`] cannot be added without a
    /// field here to hold it — which is the cheapest way to make widening the
    /// vocabulary a conversation somebody has to have.
    #[must_use]
    pub const fn colour(&self, token: Token) -> Rgb {
        match token {
            Token::Surface1 => self.surface_1,
            Token::Surface2 => self.surface_2,
            Token::Field => self.field,
            Token::Text => self.text,
            Token::TextMuted => self.text_muted,
            Token::Emphasis => self.emphasis,
            Token::Edge => self.edge,
            Token::FieldText => self.field_text,
            Token::FieldDanger => self.field_danger,
        }
    }

    /// What this theme asks for a metric, or `None` for the derived one.
    #[must_use]
    pub const fn asked(&self, metric: Metric) -> Option<i32> {
        match metric {
            Metric::TextSize => Some(self.text_size_pt_x10),
            Metric::Density => Some(self.density_x1000),
            Metric::Space => Some(self.space_em_x100),
            Metric::Stroke => Some(self.stroke_em_x100),
            Metric::Em => None,
        }
    }
}

/// One decision this module made that the theme's author did not.
///
/// Every clamp and every refusal produces one. There is no path through
/// [`resolve`] that changes a value without adding a note, and that is the
/// property which makes this layer debuggable rather than merely safe. It is
/// asserted the only way it can be: by a hostile theme whose every hostility
/// must appear here by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// An ink did not clear its floor on a ground and was moved until it did.
    ContrastRaised {
        /// Which ink.
        ink: Token,
        /// On which ground. The same ink can be raised on one ground and left
        /// alone on another, and reading both notes together is how a theme's
        /// author discovers that two of their colours are now closer than they
        /// meant them to be.
        ground: Token,
        /// What the theme's own pair achieved.
        was_x1000: u32,
        /// What the resolved pair achieves.
        now_x1000: u32,
    },
    /// An ink could not be made to clear its floor on a ground at all.
    ///
    /// Impossible for this module's own floors, which are below
    /// [`REACHABLE_X1000`]. It is here because a floor is exactly the kind of
    /// constant somebody raises, and on the day it is raised past the ceiling
    /// this is what the run says instead of returning a colour that does not
    /// work.
    ContrastUnreachable {
        /// Which ink.
        ink: Token,
        /// On which ground.
        ground: Token,
        /// The best any colour achieves against that ground.
        best_x1000: u32,
    },
    /// A metric outside its bounds took the bound.
    MetricClamped {
        /// Which metric.
        metric: Metric,
        /// What the theme asked for.
        asked: i32,
        /// What it got.
        given: i32,
    },
    /// A font preference was not a family name and was dropped from the stack.
    FontDropped {
        /// Its position in the theme's preference list.
        at: u8,
        /// Which clause refused it.
        why: NotAFamily,
    },
}

/// Every decision [`resolve`] made, bounded by [`NOTES_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// The notes, in the order they were made.
    notes: [Option<Note>; NOTES_MAX],
    /// How many are real.
    len: u8,
    /// How many did not fit. Not expected to be anything but zero; counted
    /// because a report that could truncate silently would be the one place in
    /// this module where something happens and nothing says so.
    dropped: u16,
}

impl Report {
    /// Nothing to say.
    pub const EMPTY: Self = Self { notes: [None; NOTES_MAX], len: 0, dropped: 0 };

    /// How many notes it holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Does it hold none?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many notes did not fit.
    #[must_use]
    pub const fn dropped(&self) -> u16 {
        self.dropped
    }

    /// Did the theme survive resolution untouched?
    ///
    /// A report with dropped notes is never clean, whatever its length, because
    /// the one thing it cannot say is what it forgot.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.len == 0 && self.dropped == 0
    }

    /// Every note, in order.
    pub fn iter(&self) -> impl Iterator<Item = Note> + '_ {
        self.notes[..self.len as usize].iter().copied().flatten()
    }

    /// Is `note` among them?
    #[must_use]
    pub fn holds(&self, note: Note) -> bool {
        self.iter().any(|held| held == note)
    }

    /// Does it say anything about this metric?
    #[must_use]
    pub fn clamped(&self, metric: Metric) -> bool {
        self.iter().any(|note| match note {
            Note::MetricClamped { metric: named, .. } => named == metric,
            _ => false,
        })
    }

    /// Does it say this ink was moved on this ground?
    #[must_use]
    pub fn raised(&self, ink: Token, ground: Token) -> bool {
        self.iter().any(|note| match note {
            Note::ContrastRaised { ink: moved, ground: on, .. } => moved == ink && on == ground,
            _ => false,
        })
    }

    /// Record a decision, or count it if there is no room left.
    fn push(&mut self, note: Note) {
        if (self.len as usize) < NOTES_MAX {
            self.notes[self.len as usize] = Some(note);
            self.len += 1;
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }
}

/// A theme that has been checked: the only thing in this crate holding values
/// rather than names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// For each token, its colour on each of the three grounds. A ground's row
    /// holds its own colour three times, which costs nine bytes and removes the
    /// need for a second index function.
    colour: [[Rgb; GROUND_COUNT]; TOKEN_COUNT],
    /// For each ordered pair of grounds, whether they part and what rule is owed
    /// between them. Nine entries, measured once, for [`Resolved::boundary`]'s
    /// reason.
    boundaries: [[Boundary; GROUND_COUNT]; GROUND_COUNT],
    /// Each metric after its bound, indexed by [`Metric::index`].
    metric: [i32; METRIC_COUNT],
    /// The families to try.
    font: FontStack,
    /// The floors every pair above was checked against. Carried rather than
    /// assumed, so that [`Paint::floor_x1000`] describes the table a reader is
    /// holding rather than the constants this module happens to ship.
    floors: Floors,
}

/// What a boundary table holds before it is measured.
///
/// Never observable: [`resolve_with`] overwrites all nine entries before it
/// hands the [`Resolved`] out. It exists because filling an array needs a value
/// to start from, and it is deliberately a boundary that claims nothing — no
/// rule, no contrast, not self-evident — so that an entry that somehow escaped
/// measurement would be read as *draw a black rule that does not work* rather
/// than as *these grounds part on their own*.
const UNMEASURED: Boundary = Boundary {
    over: Token::Surface1,
    under: Token::Surface1,
    grounds_x1000: 0,
    self_evident: false,
    edge_rgb: Rgb::BLACK,
    edge_x1000: 0,
    edge_under_x1000: 0,
};

/// The ground a token names: itself when it is one, and otherwise the ground the
/// ink it is paired with is read on.
///
/// One function rather than two matches, and that is the point. There were two,
/// and they disagreed: the column lookup fell back to the *ink argument's*
/// paired ground while [`Resolved::ground`] fell back to the *ground argument's*,
/// so `Resolved::contrast_x1000(Token::Edge, Token::FieldDanger)` compared the
/// edge as clamped against `surface-1` with `field`'s colour and returned a
/// number for a pair nobody had checked — under the hostile theme, 1.5 to 1
/// reported as though it were an answer. Everything that needs to know which
/// ground a pair is really against now asks here.
const fn ground_of(token: Token) -> Token {
    match token.pairing() {
        Some(pairing) => pairing.ground,
        None => token,
    }
}

/// What a token is for, and [`Duty::See`] for a ground.
///
/// A ground is never a foreground, so the second arm is not a default so much as
/// a statement that a ground owes nothing as an ink. Nothing reaches it — the
/// `ink` half of a [`Paint`] only ever holds an ink — and it is here so that the
/// answer to a question nobody asks is a conservative token rather than a panic.
const fn duty_of(token: Token) -> Duty {
    match token.pairing() {
        Some(pairing) => pairing.duty,
        None => Duty::See,
    }
}

/// The ground a pair is checked against.
///
/// The `ground` argument when it is a ground, and otherwise the ink's own — a
/// caller who passed an ink where a ground belongs gets a pair somebody argued
/// about rather than column zero.
const fn checked_ground(ink: Token, ground: Token) -> Token {
    if ground.is_ground() { ground } else { ground_of(ink) }
}

/// Which column of `Resolved::colour` a pair lands in.
fn slot_for(ink: Token, ground: Token) -> usize {
    // `checked_ground` always answers with a ground and every ground has a slot,
    // so the default is unreachable rather than a choice of column. It is spelled
    // `unwrap_or_default` and not `expect` because this runs per node per frame
    // and column zero is a readable answer.
    checked_ground(ink, ground).ground_slot().unwrap_or_default()
}

impl Resolved {
    /// The colour of `ink` when it is painted on `ground`.
    ///
    /// Both halves are required, because there is no such thing as the colour of
    /// an ink. Passing an ink as `ground` falls back to that ink's own pairing,
    /// which is the nearest thing to a right answer available.
    #[must_use]
    pub fn on(&self, ink: Token, ground: Token) -> Rgb {
        self.colour[ink.index()][slot_for(ink, ground)]
    }

    /// The colour of a ground. Given an ink, the ground it is paired with.
    #[must_use]
    pub fn ground(&self, token: Token) -> Rgb {
        self.colour[ground_of(token).index()][0]
    }

    /// What the resolved pair achieves. At or above the ink's floor, always.
    ///
    /// *Always* is a promise, so both halves of the pair are read off the same
    /// ground — `checked_ground`, which is also what picks the column. An ink
    /// passed where a ground belongs therefore gets the ratio of a pair this
    /// module checked, not the ratio of one colour against a different pair's
    /// background; `an_ink_where_a_ground_belongs_is_still_a_checked_pair`
    /// asserts it over every pair of inks in the vocabulary.
    #[must_use]
    pub fn contrast_x1000(&self, ink: Token, ground: Token) -> u32 {
        let ground = checked_ground(ink, ground);
        contrast_x1000(self.on(ink, ground), self.ground(ground))
    }

    /// How the region on `over` is told from the region on `under` it sits on.
    ///
    /// Either ground may be named by an ink, in which case it means that ink's
    /// own ground; `checked_ground`'s reason, applied here so that this method
    /// cannot be the second place a pair goes unchecked.
    ///
    /// # Why this is a lookup and not a calculation
    ///
    /// Because a compositor calls it for every ground it puts on another, every
    /// frame, and it used to recompute three contrast ratios — six relative
    /// luminances and three divisions — from the raw colours on each call. RFC
    /// 0079 tells the compositor it evaluates no contrast, and a method that
    /// evaluates contrast is not that promise however small the sum is. There
    /// are exactly nine ordered pairs of grounds; [`resolve_with`] measures all
    /// nine once. It is the eighty-one-byte ink table's argument applied to the
    /// other half of the module: remove the possibility rather than document
    /// against it.
    #[must_use]
    pub fn boundary(&self, over: Token, under: Token) -> Boundary {
        // `ground_of` always answers with a ground and every ground has a slot,
        // so the defaults are unreachable rather than a choice of row.
        // `slot_for`'s reason, spelled the same way for the same reason.
        let row = ground_of(over).ground_slot().unwrap_or_default();
        let column = ground_of(under).ground_slot().unwrap_or_default();
        self.boundaries[row][column]
    }

    /// Work one boundary out from the colours.
    ///
    /// Called once per ordered pair by [`resolve_with`] and by nothing else,
    /// which is what makes [`Resolved::boundary`] a lookup.
    fn measure_boundary(&self, over: Token, under: Token) -> Boundary {
        let over = ground_of(over);
        let under = ground_of(under);
        let over_rgb = self.ground(over);
        let under_rgb = self.ground(under);
        let grounds_x1000 = contrast_x1000(over_rgb, under_rgb);
        // The edge as resolved *on the raised ground*, which is the ground the
        // rule is drawn against and the one it has already cleared the non-text
        // floor on. Taking the lower ground's column instead would hand back a
        // rule that vanishes into the region it encloses — under the hostile
        // theme, an edge clamped for `surface-1` is 1.5 to 1 on `field`.
        let edge_rgb = self.on(Token::Edge, over);
        Boundary {
            over,
            under,
            grounds_x1000,
            self_evident: grounds_x1000 >= CONTRAST_NONTEXT_X1000,
            edge_rgb,
            edge_x1000: contrast_x1000(edge_rgb, over_rgb),
            edge_under_x1000: contrast_x1000(edge_rgb, under_rgb),
        }
    }

    /// A metric after its bound.
    #[must_use]
    pub fn metric(&self, metric: Metric) -> i32 {
        self.metric[metric.index()]
    }

    /// The resolved em, in tenths of a point. Between nine and seventy-two
    /// points whatever the theme asked for.
    #[must_use]
    pub fn em_pt_x10(&self) -> i32 {
        self.metric(Metric::Em)
    }

    /// The spacing unit, in tenths of a point. Never negative.
    #[must_use]
    pub fn space_pt_x10(&self) -> i32 {
        self.metric(Metric::Space) * self.em_pt_x10() / 100
    }

    /// The rule thickness, in tenths of a point. Never negative.
    #[must_use]
    pub fn stroke_pt_x10(&self) -> i32 {
        self.metric(Metric::Stroke) * self.em_pt_x10() / 100
    }

    /// The smallest an operable node may be under *this* theme, in tenths of a
    /// point.
    ///
    /// The larger of [`CONTROL_MIN_PT_X10`] and the em with the two rules that
    /// bound it. The first is a finger and does not move. The second is what the
    /// control has to contain: its own word, and the edge drawn around it.
    /// Neither number bounds the other — eighteen points is above the second for
    /// every ordinary theme and below it well before the em reaches its own
    /// bound — so the floor is the maximum rather than either of them.
    ///
    /// # Why the stroke is counted here
    ///
    /// Because it was counted nowhere. `stroke` was the one metric in this
    /// module related to nothing it had to fit inside: [`Metric::Stroke`] is
    /// half an em at its bound, so a theme at both bounds draws a thirty-six
    /// point rule, and a floor of the em alone made an operable node at its
    /// minimum exactly two of its own rules with no interior at all. Two legal
    /// metrics multiplied into an unusable control is the shape of defect the
    /// derived em exists to catch one level down, and it needs the same answer
    /// here: counting the rules makes a thicker rule produce a taller control
    /// rather than a smaller one.
    #[must_use]
    pub fn operable_min_pt_x10(&self) -> i32 {
        // Bounded by 720 + 2 * 360 with every metric at its bound, so the
        // saturation is for the edit that widens a bound rather than for today.
        let contained = self.em_pt_x10().saturating_add(self.stroke_pt_x10().saturating_mul(2));
        if contained > CONTROL_MIN_PT_X10 { contained } else { CONTROL_MIN_PT_X10 }
    }

    /// The floor this node's own minimum is held to, in tenths of a point.
    ///
    /// Zero for most of a tree, because most of a tree is text and containers
    /// and the only floor under those is the author's own declaration. Two
    /// floors are not zero, they are independent, and one node can owe both.
    ///
    /// An operable node owes [`Resolved::operable_min_pt_x10`]: a control is at
    /// least a finger and at least its own word inside its own edge.
    ///
    /// A node whose duty is [`Duty::See`] owes its own rule's thickness, and
    /// that is the floor the module did not have. Such a node's whole content
    /// *is* an edge — that is what the duty means — so a separator that declares
    /// no minimum was a rule thirty-six points thick and nought points tall,
    /// reported by the same [`Resolved`] that said how thick its rule was. The
    /// floor is expressed against the duty rather than against
    /// [`Role::Separator`] by name so that a role added as `See` gets it without
    /// anybody remembering this line.
    #[must_use]
    pub fn floor_pt_x10(&self, node: &Node) -> i32 {
        let mut floor = 0;
        if node.intent.is_some() {
            floor = self.operable_min_pt_x10();
        }
        if matches!(Duty::of(node.role), Duty::See) {
            let rule = self.stroke_pt_x10();
            if rule > floor {
                floor = rule;
            }
        }
        floor
    }

    /// The floors this theme was resolved against.
    #[must_use]
    pub const fn floors(&self) -> Floors {
        self.floors
    }

    /// The families to try.
    #[must_use]
    pub fn font(&self) -> &FontStack {
        &self.font
    }

    /// What a node is painted in.
    ///
    /// The node's own [`TokenSet`](crate::node::TokenSet) is read in declaration
    /// order: the first ground token it wears becomes its ground, the first ink
    /// token becomes its ink, and anything else is recorded in
    /// [`Paint::unknown`] rather than guessed at. A node wearing nothing gets its
    /// role's ink and that ink's paired ground, which is why a tree with no style
    /// at all still resolves to a pair that has been checked.
    ///
    /// # The ink a node may not wear
    ///
    /// The floor this pair is held to is the node's, from [`Duty::of`], and not
    /// the token's. So a node can declare an ink whose own floor is under what
    /// the node owes — `edge` on anything that carries glyphs is the case, and
    /// `node.rs` admits it — and that ink is **refused**: the node gets its
    /// role's ink and [`Paint::refused`] names the token that was declined.
    ///
    /// Refused and not clamped, by this module's own rule. A colour that misses
    /// a floor has a nearest legal colour along its own hue; a token that is the
    /// wrong token has no nearest anything, because `edge` raised to four and a
    /// half to one is `text` and choosing that on an author's behalf is how a
    /// design system acquires a colour nobody asked for. The alternative — hold
    /// the node to `edge`'s floor because `edge` is what it wears — is what this
    /// module did for two rounds, and it made the standard a thing the theme
    /// could move.
    #[must_use]
    pub fn paint(&self, node: &Node) -> Paint {
        let mut ground = None;
        let mut declared = None;
        let mut unknown = None;
        for name in node.style.iter() {
            match Token::from_name(name.as_str()) {
                Some(token) if token.is_ground() => {
                    if ground.is_none() {
                        ground = Some(token);
                    }
                }
                Some(token) => {
                    if declared.is_none() {
                        declared = Some(token);
                    }
                }
                None => {
                    if unknown.is_none() {
                        unknown = Some(name);
                    }
                }
            }
        }
        let duty = Duty::of(node.role);
        let owed = self.floors.of(duty);
        let refused = declared.filter(|token| self.floors.of(duty_of(*token)) < owed);
        let honoured = if refused.is_some() { None } else { declared };
        let ink_chosen = if honoured.is_some() { Chosen::Declared } else { Chosen::ByRole };
        let ground_chosen = if ground.is_some() { Chosen::Declared } else { Chosen::ByRole };
        let ink = match honoured {
            Some(token) => token,
            None => Token::ink_for(node.role),
        };
        let ground = match ground {
            Some(token) => token,
            // `ink_for` returns an ink and the branch above only ever stores
            // inks, so the pairing is always there. The arm costs nothing and
            // removes the one place in this module that would want a panic.
            None => match ink.pairing() {
                Some(pairing) => pairing.ground,
                None => Token::Surface1,
            },
        };
        Paint {
            ground,
            ink,
            duty,
            floor_x1000: self.floors.stricter(duty, duty_of(ink)),
            ground_rgb: self.ground(ground),
            ink_rgb: self.on(ink, ground),
            contrast_x1000: self.contrast_x1000(ink, ground),
            ground_chosen,
            ink_chosen,
            unknown,
            refused,
        }
    }

    /// How much room a node needs and may have, in tenths of a point.
    ///
    /// This is the layout half of the exit, and it has exactly two floors under
    /// it. The first is the application's own declaration: a node's
    /// `min_em_x100` is honoured in the unit it was written in, so a theme can
    /// scale it but cannot ignore it. The second is
    /// [`Resolved::floor_pt_x10`] — the operable minimum for anything operable,
    /// and its own rule's thickness for anything whose whole content is a rule,
    /// so that neither shrinking a metric nor inflating one produces a node that
    /// cannot hold what it is.
    ///
    /// What this deliberately does **not** do is police the declaration. A node
    /// that asks for six hundred ems gets six hundred ems: that is an author's
    /// mistake, it is visible in the author's own source, and `node.rs`'s
    /// `check` is where an author is refused. A token layer that second-guessed
    /// the application would be a second opinion about layout in the one place
    /// the design document says there is none.
    #[must_use]
    pub fn span(&self, node: &Node) -> Span {
        let em = i64::from(self.em_pt_x10());
        let declared = i64::from(node.layout.min_em_x100) * em / 100;
        // The product cannot exceed 471 852 while the em is bounded above by 720
        // and the declaration by a `u16`. The saturation is here so that widening
        // either bound is a large number rather than a wrapped one.
        let mut min = i32::try_from(declared).unwrap_or(i32::MAX);
        let floor_pt_x10 = self.floor_pt_x10(node);
        let raised_to_floor = min < floor_pt_x10;
        if raised_to_floor {
            min = floor_pt_x10;
        }
        let mut max = if node.layout.max_em_x100 == Constraints::UNBOUNDED {
            Span::UNBOUNDED
        } else {
            let ceiling = i64::from(node.layout.max_em_x100) * em / 100;
            i32::try_from(ceiling).unwrap_or(Span::UNBOUNDED)
        };
        // An operable node whose author capped it below the operable floor is the
        // one case where the two floors disagree, and the floor wins: a control
        // that cannot be operated is not a smaller control.
        let max_raised_to_min = max < min;
        if max_raised_to_min {
            max = min;
        }
        Span { min_pt_x10: min, max_pt_x10: max, floor_pt_x10, raised_to_floor, max_raised_to_min }
    }
}

/// How a region on one ground is told from the region it sits on.
///
/// The answer to the question a token layer is most tempted to skip, because
/// neither half of it is an ink: `surface-2` is documented as a raised ground
/// *for a region that sits on the first*, and a layer that only ever checked
/// inks against grounds would let a theme set all three grounds to one colour
/// and call the result readable. WCAG 1.4.11 — the criterion
/// [`CONTRAST_NONTEXT_X1000`] is taken from — is about exactly this: not text,
/// but the boundaries that say where one thing stops and the next begins.
///
/// # What it guarantees, and the one thing it does not
///
/// [`Boundary::self_evident`] is true when the two grounds clear the non-text
/// floor against each other, and then the region has an edge without anything
/// being drawn. When it is false — which includes every ordinary theme, since a
/// raised surface a shade off its parent is the normal idiom and not an attack —
/// [`Boundary::edge_rgb`] is a rule that clears the non-text floor **against the
/// ground it is drawn on**, and the compositor owes it. That much is always
/// available, because the ceiling on what a clamp can reach against any ground at
/// all is [`REACHABLE_X1000`], which is above three to one with room to spare.
///
/// What it does not guarantee is the other side: [`Boundary::edge_under_x1000`]
/// can be under the floor, and no choice of rule would fix it. There are pairs
/// of grounds against which no colour in the space reaches three to one on both
/// sides at once — `#515151` and `#9F9F9F` contrast 2.998 to 1 with each other,
/// so a rule is needed, and the best any rule does against the worse side is
/// 2.646 to 1. `no_colour_separates_two_grounds_on_both_sides` exhibits it by
/// sweeping every luminance. A future that wants a two-sided promise has to
/// bound the *grounds* — which is the clamp this module refused, and would refuse
/// [`Theme::DEFAULT`]'s own raised surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boundary {
    /// The ground of the region on top.
    pub over: Token,
    /// The ground it sits on.
    pub under: Token,
    /// What the two grounds achieve against each other.
    pub grounds_x1000: u32,
    /// Do they clear [`CONTRAST_NONTEXT_X1000`] on their own? When they do not,
    /// the rule below is not decoration and the compositor must draw it.
    pub self_evident: bool,
    /// The colour of that rule: `edge` as resolved on [`Boundary::over`].
    pub edge_rgb: Rgb,
    /// What the rule achieves against the ground it is drawn on. At or above
    /// [`CONTRAST_NONTEXT_X1000`], always.
    pub edge_x1000: u32,
    /// What it achieves against the lower ground. Reported rather than promised,
    /// for the reason on this type.
    pub edge_under_x1000: u32,
}

/// Where a node's ground or ink came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// The node wore the token.
    Declared,
    /// The node wore none, so its role decided.
    ByRole,
}

/// The pair a node is painted in, and where each half came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paint {
    /// The ground token.
    pub ground: Token,
    /// The ink token.
    pub ink: Token,
    /// What this node asks of its ink, from its role. The theme has no say in
    /// it, which is the point of it.
    pub duty: Duty,
    /// What the pair had to clear: the larger of this node's duty's floor and
    /// the ink's own, under the [`Floors`] the theme was resolved against.
    ///
    /// Read the floor from here rather than from the ink's [`Pairing`]. They
    /// agree for a node wearing nothing, and where they disagree it is because
    /// the node wears an ink held to less than the node owes — which is the
    /// case an assertion against the token's own floor cannot see.
    pub floor_x1000: u32,
    /// What the ground resolves to.
    pub ground_rgb: Rgb,
    /// What the ink resolves to **on that ground**.
    pub ink_rgb: Rgb,
    /// What the pair achieves. At or above [`Paint::floor_x1000`].
    pub contrast_x1000: u32,
    /// Where the ground came from.
    pub ground_chosen: Chosen,
    /// Where the ink came from. [`Chosen::ByRole`] both when the node wore no
    /// ink and when the ink it wore was refused; [`Paint::refused`] is what
    /// separates the two.
    pub ink_chosen: Chosen,
    /// The first token name the node wore that this vocabulary does not have.
    ///
    /// Reported here rather than guessed at, and reported on the value it would
    /// have affected rather than through a side channel, so a projection holding
    /// a `Paint` is holding the refusal too.
    pub unknown: Option<TokenName>,
    /// The ink this node wore and may not wear: one whose own floor is under
    /// what this node's [`Duty`] owes.
    ///
    /// A token in the vocabulary, unlike [`Paint::unknown`], which is why it is
    /// a [`Token`] and not a name — the author spelled something this module
    /// understands and declined, and a projection showing the author what
    /// happened can name both halves of that.
    pub refused: Option<Token>,
}

/// How much room a node needs and may have, in tenths of a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// The smallest it may be.
    pub min_pt_x10: i32,
    /// The largest worth giving it, or [`Span::UNBOUNDED`].
    pub max_pt_x10: i32,
    /// The floor this node was held to, from [`Resolved::floor_pt_x10`]. Zero
    /// for a node no floor applies to.
    ///
    /// A number rather than one flag per floor. Two floors can apply to one node
    /// — an operable separator owes both — and a pair of booleans would leave a
    /// reader of that node unable to say which minimum it got.
    pub floor_pt_x10: i32,
    /// Was the author's own declaration under that floor?
    pub raised_to_floor: bool,
    /// Was the maximum raised to meet the minimum?
    pub max_raised_to_min: bool,
}

impl Span {
    /// As much room as there is: `Constraints::UNBOUNDED` carried through rather
    /// than turned into a number, because *as much as there is* is not
    /// forty-seven thousand points, and a projection that read it as one would
    /// cap a window at a size nobody chose.
    pub const UNBOUNDED: i32 = i32::MAX;
}

/// Turn a theme into values against the floors this module ships, and say what
/// had to be decided on its behalf.
///
/// [`resolve_with`] with [`Floors::STANDARD`], which is what every caller
/// outside this module's own tests wants.
#[must_use]
pub fn resolve(theme: &Theme) -> (Resolved, Report) {
    resolve_with(theme, Floors::STANDARD)
}

/// The same, against floors the caller chooses.
///
/// # Why a caller may choose them
///
/// Not because a theme may: nothing a theme sets reaches here. Because
/// [`Note::ContrastUnreachable`] is otherwise a branch nothing can enter —
/// both standard floors are below [`REACHABLE_X1000`], so every pair is always
/// reachable — and a failure path no test can reach is a comment. See
/// [`Floors`].
///
/// The order is load-bearing in one place: the metrics are clamped **before**
/// the em is derived from two of them, so the multiplication is over values that
/// are already inside their bounds and cannot overflow. The arithmetic is
/// widened anyway, which is belt and braces on purpose — the clamp is the thing
/// a future edit widens, and the wider type is what stops that edit from
/// becoming a wrap.
#[must_use]
pub fn resolve_with(theme: &Theme, floors: Floors) -> (Resolved, Report) {
    let mut report = Report::EMPTY;

    let mut metric = [0i32; METRIC_COUNT];
    for named in Metric::ALL {
        if let Some(asked) = theme.asked(named) {
            metric[named.index()] = clamp(named, asked, &mut report);
        }
    }
    let product = i64::from(metric[Metric::TextSize.index()])
        * i64::from(metric[Metric::Density.index()])
        / 1_000;
    let asked_em = i32::try_from(product).unwrap_or(i32::MAX);
    metric[Metric::Em.index()] = clamp(Metric::Em, asked_em, &mut report);

    let mut colour = [[Rgb::BLACK; GROUND_COUNT]; TOKEN_COUNT];
    for token in Token::ALL {
        match token.pairing() {
            // A ground is free: whatever the theme said, in every column.
            None => colour[token.index()] = [theme.colour(token); GROUND_COUNT],
            Some(pairing) => {
                let floor = floors.of(pairing.duty);
                for ground in Token::ALL {
                    let Some(slot) = ground.ground_slot() else { continue };
                    let asked = theme.colour(token);
                    let under = theme.colour(ground);
                    let raised = raise_to_floor(asked, under, floor);
                    colour[token.index()][slot] = raised.colour;
                    if !raised.reached {
                        report.push(Note::ContrastUnreachable {
                            ink: token,
                            ground,
                            best_x1000: raised.contrast_x1000,
                        });
                    } else if raised.steps > 0 {
                        report.push(Note::ContrastRaised {
                            ink: token,
                            ground,
                            was_x1000: contrast_x1000(asked, under),
                            now_x1000: raised.contrast_x1000,
                        });
                    }
                }
            }
        }
    }

    let mut font = FontStack::BARE;
    for (at, asked) in theme.fonts.into_iter().enumerate() {
        // An unused slot is not a hostile theme; see `Theme::fonts`.
        if asked.is_empty() {
            continue;
        }
        match FamilyName::new(asked) {
            Ok(family) => font.push(family),
            Err(why) => report.push(Note::FontDropped { at: at as u8, why }),
        }
    }

    let mut resolved = Resolved {
        colour,
        boundaries: [[UNMEASURED; GROUND_COUNT]; GROUND_COUNT],
        metric,
        font,
        floors,
    };
    // The nine ordered pairs of grounds, measured once so that a compositor
    // asking for one is reading rather than computing. Written into a copy and
    // assigned back, because `measure_boundary` reads the colours off the same
    // value the table lives in.
    let mut boundaries = resolved.boundaries;
    for over in Token::ALL {
        let Some(row) = over.ground_slot() else { continue };
        for under in Token::ALL {
            let Some(column) = under.ground_slot() else { continue };
            boundaries[row][column] = resolved.measure_boundary(over, under);
        }
    }
    resolved.boundaries = boundaries;
    (resolved, report)
}

/// Take the bound if the ask is outside it, and say so if it was.
fn clamp(metric: Metric, asked: i32, report: &mut Report) -> i32 {
    let given = if asked < metric.low() {
        metric.low()
    } else if asked > metric.high() {
        metric.high()
    } else {
        asked
    };
    if given != asked {
        report.push(Note::MetricClamped { metric, asked, given });
    }
    given
}

/// The two numbers `claims/0035-theme-refusals.toml` publishes, over a corpus of
/// themes — or `None` when the corpus has no themes in it.
///
/// `(themes_resolving_clean_per_thousand, decisions_per_theme_x100)`.
///
/// # Why this is here and not in the command that prints it
///
/// Because the division and the emptiness rule have to be one definition.
/// `claims/0034` asks the same of the canvas rate in so many words and
/// `crate::node::canvas_census` is the answer there: the per-entry refusal and
/// the corpus-level refusal are the *same function*, so the two cannot come to
/// disagree about what *empty* means. The direction they would disagree in is the
/// one that does damage quietly — a clean zero, or a flattering share, reported
/// over a corpus nobody assembled.
///
/// `None` and not `Some((0, 0))`, for exactly that reason. Nought clean themes in
/// nought themes is not a share; it is the absence of one, and a zero here would
/// read as *this layer corrects every theme it is shown*, which is `claims/0035`'s
/// floor firing about a corpus that does not exist.
///
/// # What a decision is, and why a dropped note is one
///
/// The second number counts `Report::len() + Report::dropped()` and not the
/// length alone. A note that did not fit in the report was still a decision this
/// layer made on somebody's behalf, and counting only what fitted would make the
/// mean *fall* as the layer got noisier — which is precisely backwards for a row
/// bounded above. `Report::dropped` is published beside this number rather than
/// folded into it, because a truncated report is the one thing `claims/0035`'s
/// `[diagnosis]` says to read first.
///
/// # The rounding, which is part of the number
///
/// The share **truncates** and the mean **rounds up**, and the two differ because
/// the rows differ. The mean is bounded above only, so truncation moves every
/// borderline corpus to the side that flatters this layer — a mean of 4.009 notes
/// reported as 400 is a ceiling of 400 not firing on a corpus that met it — and
/// rounding away from the bound is the same rule `claims/0034` applies to its
/// rate and for the same reason. The share is bounded at *both* ends, so no
/// rounding direction is uniformly flattering: truncation makes the floor a
/// hundredth of a point stricter and the ceiling a hundredth of a point more
/// lenient, and the error is under one part in a thousand whatever the corpus
/// size, which is below the resolution either bound was argued at — `claims/0035`
/// picks two hundred and nine hundred as thirds of the range, not as knife edges.
///
/// *What would reverse the share's rule:* a corpus small enough that one theme
/// moves it across a bound. At that size the share is not readable at all and the
/// answer is a run that refuses for being too small, not a cleverer rounding.
#[must_use]
pub fn census(themes: &[Theme]) -> Option<(u32, u32)> {
    if themes.is_empty() {
        return None;
    }
    let mut clean: u64 = 0;
    let mut decisions: u64 = 0;
    for theme in themes {
        let (_, report) = resolve(theme);
        if report.is_clean() {
            clean += 1;
        }
        decisions += report.len() as u64 + u64::from(report.dropped());
    }
    let total = themes.len() as u64;
    let share = clean * 1_000 / total;
    let mean = (decisions * 100).div_ceil(total);
    // Both fit: the share is at most a thousand, and the mean is at most a
    // hundred times `NOTES_MAX` plus whatever one resolution can drop, which is
    // bounded by the eighteen pairs, the five metrics and the three font slots.
    // The conversions are written rather than inferred so that a future corpus
    // reader cannot silently wrap one.
    Some((u32::try_from(share).unwrap_or(u32::MAX), u32::try_from(mean).unwrap_or(u32::MAX)))
}

// ---------------------------------------------------------------------------
// The demonstration. Seven themes, published rather than hidden in the tests,
// because a corpus has to be refusable by a machine — RFC 0110.
// ---------------------------------------------------------------------------

/// A theme this module has no quarrel with, on the other side of the palette
/// from [`Theme::DEFAULT`].
///
/// It is here for one assertion and it is an important one: two themes that
/// look nothing alike both survive resolution untouched. A token layer that
/// clamped everything into one safe palette would pass every readability
/// test in this file, and would have made the theme layer decorative — which
/// is the failure the exit is written to catch from the other side.
pub const DARK: Theme = Theme {
    surface_1: Rgb::new(0x12, 0x12, 0x12),
    surface_2: Rgb::new(0x1E, 0x1E, 0x1E),
    field: Rgb::new(0x0A, 0x0A, 0x0A),
    text: Rgb::new(0xE8, 0xE8, 0xE8),
    text_muted: Rgb::new(0xA0, 0xA0, 0xA0),
    emphasis: Rgb::new(0x7F, 0xB3, 0xFF),
    edge: Rgb::new(0x6E, 0x6E, 0x6E),
    field_text: Rgb::new(0xE8, 0xE8, 0xE8),
    field_danger: Rgb::new(0xFF, 0x8A, 0x80),
    text_size_pt_x10: 110,
    density_x1000: 1_250,
    space_em_x100: 62,
    stroke_em_x100: 8,
    fonts: ["Source Sans 3", "", ""],
};

/// The theme the exit asks for: hostile in every dimension this module has.
///
/// Each field is an attack with a name rather than a bad value picked at
/// random, and they are listed here so a reader can check the demonstration
/// is not merely inconvenient.
///
/// - `surface_1` is a mid grey near where black and white are equally bad,
///   which leaves the clamp a hundred and twenty-three parts in a thousand
///   above the floor. A hostile theme that picked a dark ground would be
///   handing the clamp an easy problem. It is *not* the tightest ground in
///   the space — that one is not a grey, and [`HOSTILE_FLAT`] is where it is
///   attacked with.
/// - `text` is *identical to its ground*: one to one, the worst ratio there
///   is, and the one a layer checking colours in isolation cannot see.
/// - `text_muted` is `#787878`, a perfectly ordinary grey any designer might
///   write, and illegible on `#767676`. The *legal individually, unreadable
///   in combination* case, stated plainly.
/// - `emphasis` is white on white and `field_text` is black on black, each
///   one value away from being exactly so, because the exactly-equal case is
///   the one a naive equality check would catch.
/// - `field_danger` is a saturated red on a black ground. It is here to be
///   *survived* rather than merely fixed: `a_clamp_is_not_a_collapse` asserts
///   it is still red afterwards, on every ground.
/// - `text_size_pt_x10` is zero — text with no size.
/// - `density_x1000` is the most negative value the type holds, which is the
///   scale factor that overflows, in the direction where negating it
///   overflows too.
/// - `space_em_x100` is a large negative: spacing that does not tighten a
///   layout, it overlaps it.
/// - `stroke_em_x100` is the largest value the type holds.
/// - `fonts` names three things and none of them is a name: spaces, a control
///   character, and a name too long to hold. A stack that looks populated and
///   resolves to nothing. The empty string is deliberately *not* among them —
///   an unused slot is absence rather than an attack, and `Theme::fonts` says
///   why it is skipped in silence.
pub const HOSTILE: Theme = Theme {
    surface_1: Rgb::new(0x76, 0x76, 0x76),
    surface_2: Rgb::new(0xFF, 0xFF, 0xFF),
    field: Rgb::new(0x00, 0x00, 0x00),
    text: Rgb::new(0x76, 0x76, 0x76),
    text_muted: Rgb::new(0x78, 0x78, 0x78),
    emphasis: Rgb::new(0xFF, 0xFF, 0xFE),
    edge: Rgb::new(0x76, 0x76, 0x76),
    field_text: Rgb::new(0x00, 0x00, 0x01),
    field_danger: Rgb::new(0xD0, 0x20, 0x20),
    text_size_pt_x10: 0,
    density_x1000: i32::MIN,
    space_em_x100: -30_000,
    stroke_em_x100: i32::MAX,
    fonts: ["   ", "\u{1}", "a name of thirty-three characters"],
};

/// The same hostility from the other direction, and it is not a duplicate.
///
/// [`HOSTILE`] attacks by collapse — zero, negative, identical — and a layer
/// clamped against only that is half a layer, because every one of its bounds
/// has two ends. This theme takes a legal palette and asks for the largest
/// number the type holds in every metric, which is also the only way to reach
/// the clamp on the *derived* em: seventy-two points at four times scale is
/// four times the largest legal em, and both factors were legal when they
/// were multiplied. A layer that checked only the factors would ship it.
pub const HOSTILE_INFLATED: Theme = Theme {
    surface_1: Rgb::new(0xFF, 0xFF, 0xFF),
    surface_2: Rgb::new(0xF2, 0xF2, 0xF2),
    field: Rgb::new(0xFF, 0xFF, 0xFF),
    text: Rgb::new(0x1A, 0x1A, 0x1A),
    text_muted: Rgb::new(0x59, 0x59, 0x59),
    emphasis: Rgb::new(0x0B, 0x3D, 0x91),
    edge: Rgb::new(0x76, 0x76, 0x76),
    field_text: Rgb::new(0x1A, 0x1A, 0x1A),
    field_danger: Rgb::new(0xA4, 0x00, 0x0F),
    text_size_pt_x10: i32::MAX,
    density_x1000: i32::MAX,
    space_em_x100: i32::MAX,
    stroke_em_x100: i32::MAX,
    fonts: ["Inter", "a name of thirty-three characters", ""],
};

/// The third hostility, and the one this module did not answer at all until
/// a reviewer pointed at it: **every ground is the same colour.**
///
/// Nothing here is out of any bound. Three free grounds set to one value is
/// what the first category permits in so many words, and every ink still
/// clears its floor against them, so the whole of the colour half of this
/// file passed a theme in which no grouped region and no text field had a
/// boundary anybody could see. [`Resolved::boundary`] is the answer and
/// `a_region_that_sits_on_another_is_always_told_from_it` is where it is
/// asserted.
///
/// The colour is chosen as well: `#008909` has the lowest
/// [`best_reachable_x1000`] of anything in the twenty-four-bit space, so this
/// theme is also the tightest possible test of the clamp itself — black and
/// white both come to exactly [`REACHABLE_X1000`] against it, eighty-two
/// parts in a thousand above the text floor and not one more.
///
/// Its metrics and fonts are the default's on purpose. An attack on the
/// ground vocabulary should produce notes about colours and nothing else,
/// and a theme hostile in every dimension at once could not show that.
pub const HOSTILE_FLAT: Theme = Theme {
    surface_1: Rgb::new(0x00, 0x89, 0x09),
    surface_2: Rgb::new(0x00, 0x89, 0x09),
    field: Rgb::new(0x00, 0x89, 0x09),
    text: Rgb::new(0x00, 0x89, 0x09),
    text_muted: Rgb::new(0x0A, 0x8F, 0x12),
    emphasis: Rgb::new(0x00, 0x89, 0x09),
    edge: Rgb::new(0x00, 0x89, 0x09),
    field_text: Rgb::new(0x00, 0x89, 0x09),
    field_danger: Rgb::new(0xB0, 0x00, 0x20),
    text_size_pt_x10: 105,
    density_x1000: 1_000,
    space_em_x100: 50,
    stroke_em_x100: 6,
    fonts: ["Inter", "", ""],
};

/// The fourth, and the only one in which the boundary machinery has a
/// *choice* to make: **two grounds that differ and do not part.**
///
/// [`HOSTILE_FLAT`] collapses the grounds, and when two grounds are the same
/// colour every column of the resolved table holds the same rule, so a
/// boundary drawn from the wrong one looks right. Here they are 2.645 to 1
/// apart — visibly different, under the non-text floor — and the `edge` ink
/// resolves to `#A3A3A3` on the raised ground and `#5B5B5B` on the one below
/// it. Only the first is a rule; the second is 1.168 to 1 against the region
/// it would be enclosing. That is the pair
/// `a_region_that_sits_on_another_is_always_told_from_it` names, so that
/// taking the wrong column is an edit somebody notices.
///
/// Its metrics and fonts are the default's, for [`HOSTILE_FLAT`]'s reason.
pub const HOSTILE_ALIKE: Theme = Theme {
    surface_1: Rgb::new(0x00, 0x00, 0x00),
    surface_2: Rgb::new(0x51, 0x51, 0x51),
    field: Rgb::new(0x00, 0x00, 0x00),
    text: Rgb::new(0x00, 0x00, 0x00),
    text_muted: Rgb::new(0x11, 0x11, 0x11),
    emphasis: Rgb::new(0x00, 0x00, 0x00),
    edge: Rgb::new(0x00, 0x00, 0x00),
    field_text: Rgb::new(0x00, 0x00, 0x00),
    field_danger: Rgb::new(0x2A, 0x00, 0x00),
    text_size_pt_x10: 105,
    density_x1000: 1_000,
    space_em_x100: 50,
    stroke_em_x100: 6,
    fonts: ["Inter", "", ""],
};

/// The fifth, and the only one that attacks by subtraction.
///
/// The other four make things enormous or make them the same colour. This
/// one asks for nothing: no stroke, no space, the smallest text it can name
/// and the lowest density. It exists because
/// [`Metric::Stroke`]'s floor shared [`Metric::Space`]'s until 2026-09-15,
/// and a bound nothing pushes down on is a bound nobody has read. A theme
/// asking for `stroke_em_x100 = 0` was inside every bound, produced no
/// [`Note`], and left [`Report::is_clean`] true while taking to zero both
/// the thickness of the rule [`Boundary`] owes and the floor
/// [`Resolved::floor_pt_x10`] gives every [`Duty::See`] node.
///
/// Its colours are the default's, so that what this theme demonstrates is
/// its metrics and nothing else — for [`HOSTILE_FLAT`]'s reason in the
/// other direction.
pub const HOSTILE_ERASED: Theme = Theme {
    surface_1: Rgb::new(0xFF, 0xFF, 0xFF),
    surface_2: Rgb::new(0xF2, 0xF2, 0xF2),
    field: Rgb::new(0xFF, 0xFF, 0xFF),
    text: Rgb::new(0x1A, 0x1A, 0x1A),
    text_muted: Rgb::new(0x5E, 0x5E, 0x5E),
    emphasis: Rgb::new(0x00, 0x3A, 0x8C),
    edge: Rgb::new(0x6A, 0x6A, 0x6A),
    field_text: Rgb::new(0x1A, 0x1A, 0x1A),
    field_danger: Rgb::new(0x8C, 0x00, 0x14),
    text_size_pt_x10: 0,
    density_x1000: 0,
    space_em_x100: 0,
    stroke_em_x100: 0,
    fonts: ["Inter", "", ""],
};

/// Every theme this module ships: the seven each structural assertion below is
/// made against, and the seven a corpus may not be made of.
///
/// Named for what is true of all of them rather than for how many there are.
/// `THE_SEVEN` would have been a count in a name, and the count is the thing most
/// likely to change here — an eighth hostility is one reviewer away, and a
/// constant whose name went stale on the day it was extended is the shape of rot
/// this table exists to remove somewhere else.
///
/// Two legal and five hostile, because an assertion made only against a
/// hostile theme cannot tell *this layer holds* from *this layer clamps
/// everything*. The five hostile ones attack five different things —
/// collapse, inflation, grounds that are one colour, grounds that differ
/// without parting, and metrics asked for as nothing — and none of them is a
/// rearrangement of another.
///
/// # Why this is published rather than a fixture, RFC 0110
///
/// Because `claims/0035-theme-refusals.toml` is a share over a corpus of themes
/// *written by somebody not working on this module*, and a run that cannot
/// recognise this module's own themes cannot refuse them. Until RFC 0110 these
/// seven were `#[cfg(test)]` consts and the claim's protection against being
/// measured over them was a sentence in the claim file — which is the shape of
/// guard this repository has twice recorded the fate of.
///
/// So they are values, and `cargo xtask claim theme-refusals` compares every
/// corpus entry against this table and refuses a corpus that is this module's
/// own output. The refusal is by **value and not by name**, because a name is
/// something a corpus entry writes and a value is not.
///
/// It is one table and not two: the tests below reach it through
/// `use super::SHIPPED as THEMES`, so a theme added to the argument is a
/// theme the refusal knows about and there is no second list to forget. That is
/// the whole of what makes this arrangement not rot, and the reversal condition
/// is its inverse — a second table of demonstration themes anywhere, at which
/// point the refusal is reading one of them and passing the other.
pub const SHIPPED: [(&str, Theme); 7] = [
    ("default", Theme::DEFAULT),
    ("dark", DARK),
    ("hostile", HOSTILE),
    ("hostile-inflated", HOSTILE_INFLATED),
    ("hostile-flat", HOSTILE_FLAT),
    ("hostile-alike", HOSTILE_ALIKE),
    ("hostile-erased", HOSTILE_ERASED),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{
        CapRef, Content, Flow, NodeId, Quantity, Reading, Relation, StateSet, Text, TokenSet, Unit,
        check,
    };

    use super::SHIPPED as THEMES;

    fn content(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits within TEXT_MAX"))
    }

    fn styled(names: &[&str]) -> TokenSet {
        let mut set = TokenSet::EMPTY;
        for name in names {
            set = set
                .push(TokenName::new(name).expect("a token name, not a value"))
                .expect("within TOKENS_MAX");
        }
        set
    }

    /// The interface the demonstration runs against.
    ///
    /// A settings panel, because it is the interface where *style is tokens,
    /// never values* is most tempting to break, and because it exercises every
    /// category at once: a surface declaring a ground, a group declaring a
    /// different ground *and* an ink, controls declaring nothing and taking their
    /// role's ink, an entry on the field's ground, a separator held to the
    /// non-text floor, and one node wearing a token name this vocabulary does not
    /// have.
    ///
    /// The output group declares `surface-2` and no separator around it, which is
    /// also on purpose: it is the region whose only claim to being a region is its
    /// ground, so whether it reads as one is
    /// [`Resolved::boundary`]'s answer rather than the tree's.
    /// `a_region_that_sits_on_another_is_always_told_from_it` walks this tree
    /// parent by parent for exactly that.
    ///
    /// The mute toggle is capped below the operable floor on purpose — an author
    /// asking for a fifth-of-an-em control, which is under the floor at every
    /// legal em — because that is the one case where the application's
    /// declaration and the floor disagree, and the answer belongs in a test
    /// rather than in a paragraph.
    ///
    /// The hint wears `edge`, which is the tree round two's blocking finding was
    /// written from: a node that carries glyphs wearing the one ink held to the
    /// non-text floor. Nothing refuses that tree — `check` says nothing about
    /// `style` — so it belongs in the demonstration rather than in a note about
    /// what an author should not do.
    fn panel() -> [Node; 10] {
        const SURFACE: NodeId = NodeId::new(1);
        const OUTPUT: NodeId = NodeId::new(2);
        const VOLUME_LABEL: NodeId = NodeId::new(3);
        const VOLUME: NodeId = NodeId::new(4);
        const MUTE: NodeId = NodeId::new(5);
        const RULE: NodeId = NodeId::new(6);
        const NAME: NodeId = NodeId::new(7);
        const APPLIED: NodeId = NodeId::new(8);
        const CAPTION: NodeId = NodeId::new(9);
        const HINT: NodeId = NodeId::new(10);

        let percent = |value: i64| Quantity::whole(value, Unit::Ratio);

        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_content(content("Sound"))
                .with_layout(Constraints::flowing(Flow::Block))
                .with_style(styled(&["surface-1"])),
            Node::new(OUTPUT, SURFACE, Role::Group)
                .with_content(content("Output"))
                .with_layout(Constraints::flowing(Flow::Block).at_least(1200))
                .with_style(styled(&["surface-2", "emphasis"])),
            Node::new(VOLUME_LABEL, OUTPUT, Role::Label).with_content(content("Volume")),
            Node::new(VOLUME, OUTPUT, Role::Number)
                .with_content(Content::Value(
                    Reading::bounded(percent(70), percent(0), percent(100)).stepped(percent(5)),
                ))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0054))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(600).growing(1))
                .with_relation(Relation::LabelledBy(VOLUME_LABEL))
                .expect("within RELATIONS_MAX"),
            Node::new(MUTE, OUTPUT, Role::Toggle)
                .with_content(content("Mute"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0055))
                .with_layout(Constraints::flowing(Flow::Own).at_most(20))
                .with_relation(Relation::Controls(VOLUME))
                .expect("within RELATIONS_MAX"),
            Node::new(RULE, SURFACE, Role::Separator),
            Node::new(NAME, SURFACE, Role::Entry)
                .with_content(content("Living room"))
                .with_state(StateSet::ENABLED.with(StateSet::INVALID))
                .with_intent(CapRef::new(0x0061))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(1000))
                .with_style(styled(&["field.danger"])),
            Node::new(APPLIED, SURFACE, Role::Status)
                .with_content(content("Applied"))
                .with_style(styled(&["text.muted"])),
            // The node wearing a name nobody defined. It is in the tree rather
            // than in a test of its own because the interesting question is not
            // whether the name is rejected — it is whether a tree containing one
            // still paints, and whether the projection is told.
            Node::new(CAPTION, SURFACE, Role::Text)
                .with_content(content("Changes apply immediately."))
                .with_style(styled(&["brand.pink"])),
            // The node wearing an ink its role may not wear. `Role::Text` is
            // legal, `edge` is legal, `check` has nothing to say about the
            // combination, and under `HOSTILE` the two together used to paint at
            // 3 032 — below the floor this module cites for text, taken there by
            // the theme and not by the author. It is in the tree for the
            // caption's reason: the interesting question is not whether the ink
            // is declined, it is whether a tree containing one still paints
            // readably and whether the projection is told what happened.
            Node::new(HINT, SURFACE, Role::Text)
                .with_content(content("Muting does not change the level."))
                .with_style(styled(&["edge"])),
        ]
    }

    /// The three grounds.
    fn grounds() -> impl Iterator<Item = Token> {
        Token::ALL.into_iter().filter(|token| token.is_ground())
    }

    /// Every way one ground can sit on another, the nine of them.
    ///
    /// Ordered, and the three same-ground entries are kept rather than skipped:
    /// a group wearing `surface-1` inside a surface wearing `surface-1` is the
    /// commonest region in any tree and is exactly the one with no colour change
    /// to rely on.
    fn boundaries() -> impl Iterator<Item = (Token, Token)> {
        grounds().flat_map(|over| grounds().map(move |under| (over, under)))
    }

    /// Every ink, with each ground it can be read on and the floor it owes.
    fn pairs() -> impl Iterator<Item = (Token, Token, u32)> {
        Token::ALL
            .into_iter()
            .filter_map(|ink| ink.pairing().map(|pairing| (ink, pairing.floor_x1000())))
            .flat_map(|(ink, floor)| {
                Token::ALL
                    .into_iter()
                    .filter(|token| token.is_ground())
                    .map(move |ground| (ink, ground, floor))
            })
    }

    /// The property the exit's *reported, not silent* clause turns into:
    /// **nothing changes without a note, and nothing unchanged gets one.**
    ///
    /// One half alone is worthless. A layer that reported everything would pass
    /// the first and drown the reader; a layer that reported nothing would pass
    /// the second and be the silence this module exists to remove. Asserting both
    /// over all four themes is what makes [`Report`] a description of what
    /// happened rather than a log.
    fn every_change_is_reported(name: &str, theme: &Theme) {
        let (resolved, report) = resolve(theme);
        assert_eq!(report.dropped(), 0, "{name}: the report itself overflowed");
        for (ink, ground, floor) in pairs() {
            let (ink_name, ground_name) = (ink.name(), ground.name());
            let asked = theme.colour(ink);
            let was = contrast_x1000(asked, theme.colour(ground));
            let given = resolved.on(ink, ground);
            if was >= floor {
                assert_eq!(
                    given, asked,
                    "{name}: {ink_name} on {ground_name} was moved needlessly"
                );
                assert!(
                    !report.raised(ink, ground),
                    "{name}: {ink_name} on {ground_name} was reported and not moved"
                );
            } else {
                assert_ne!(given, asked, "{name}: {ink_name} on {ground_name} was left illegible");
                assert!(
                    report.raised(ink, ground),
                    "{name}: {ink_name} on {ground_name} was moved in silence"
                );
                assert!(
                    resolved.contrast_x1000(ink, ground) >= floor,
                    "{name}: {ink_name} on {ground_name} is still under its floor"
                );
            }
        }
        for metric in Metric::ALL {
            let given = resolved.metric(metric);
            assert!(given >= metric.low(), "{name}: {} is under its bound", metric.name());
            assert!(given <= metric.high(), "{name}: {} is over its bound", metric.name());
            if let Some(asked) = theme.asked(metric) {
                assert_eq!(
                    report.clamped(metric),
                    given != asked,
                    "{name}: {} was clamped and reported inconsistently",
                    metric.name()
                );
            }
        }
    }

    #[test]
    fn every_token_is_reachable_from_all() {
        // The teeth `node.rs` puts on `Role`, for the same reason: a tenth token
        // must be given an index by an exhaustive match, that index must land on
        // itself in `ALL`, and `ALL`'s length must equal the count. There is no
        // way to satisfy all three by accident.
        assert_eq!(Token::ALL.len(), TOKEN_COUNT);
        for (at, token) in Token::ALL.into_iter().enumerate() {
            assert_eq!(token.index(), at);
            assert_eq!(Token::ALL[token.index()], token);
            assert_eq!(Token::from_name(token.name()), Some(token));
        }
        assert_eq!(Token::ALL.into_iter().filter(|token| token.is_ground()).count(), GROUND_COUNT);
        assert_eq!(Token::ALL.into_iter().filter(|t| !t.is_ground()).count(), INK_COUNT);
        for (at, ground) in Token::ALL.into_iter().filter(|token| token.is_ground()).enumerate() {
            assert_eq!(ground.ground_slot(), Some(at), "{} is columned wrongly", ground.name());
        }
    }

    #[test]
    fn a_token_is_spelled_the_way_a_node_may_spell_it() {
        // `node.rs` refuses a token name that looks like a value, and this module
        // must not widen that grammar by the back door of naming a token
        // something no node could wear. A name that fails here is a token no
        // application could ever ask for.
        for token in Token::ALL {
            assert!(
                TokenName::new(token.name()).is_ok(),
                "{} is not a name a node may wear",
                token.name()
            );
        }
        assert_eq!(Token::from_name("brand.pink"), None);
        assert_eq!(Token::from_name("ff0000"), None);
        assert_eq!(Token::from_name(""), None);
        assert_eq!(Token::from_name("Text"), None, "names are exact, not case-folded");
    }

    #[test]
    fn a_node_cannot_wear_a_metric() {
        // The two vocabularies are disjoint, and this is where that is enforced
        // rather than intended. A node able to say `space` in its style set would
        // be a node saying how much room to leave, which is a coordinate wearing
        // a token's name.
        for metric in Metric::ALL {
            assert_eq!(
                Token::from_name(metric.name()),
                None,
                "{} is both a metric and a token",
                metric.name()
            );
        }
        assert!(Metric::ALL.into_iter().all(|m| m.is_themeable() == (m != Metric::Em)));
    }

    #[test]
    fn black_on_white_and_a_colour_on_itself() {
        // The two answers everybody knows, which is exactly why they are worth
        // pinning: an approximation that got either wrong would be obviously
        // wrong, and one that gets both right can still be wrong everywhere else
        // — which is what the next test is for.
        assert_eq!(contrast_x1000(Rgb::BLACK, Rgb::WHITE), 21_000);
        assert_eq!(contrast_x1000(Rgb::WHITE, Rgb::BLACK), 21_000);
        assert_eq!(contrast_x1000(Rgb::WHITE, Rgb::WHITE), 1_000);
        assert_eq!(contrast_x1000(Rgb::BLACK, Rgb::BLACK), 1_000);
    }

    #[test]
    fn the_pair_that_straddles_the_floor() {
        // Two greys one value apart, on white, one either side of 4.5 to 1. This
        // is the assertion that pins the table: a luminance approximation wrong
        // by even one per cent puts both of these on the same side of the floor,
        // and every other test in this file would still pass. The exact ratios by
        // the published formula are 4.5422 and 4.4781, and the integers below are
        // those to within the bound stated on `LINEAR_X100000`.
        let grey = |value: u8| Rgb::new(value, value, value);
        assert_eq!(contrast_x1000(grey(0x76), Rgb::WHITE), 4_542);
        assert_eq!(contrast_x1000(grey(0x77), Rgb::WHITE), 4_478);
        assert!(contrast_x1000(grey(0x76), Rgb::WHITE) >= CONTRAST_TEXT_X1000);
        assert!(contrast_x1000(grey(0x77), Rgb::WHITE) < CONTRAST_TEXT_X1000);
    }

    /// The tightest ground in the twenty-four-bit space.
    ///
    /// Black and white both reach exactly [`REACHABLE_X1000`] against it and
    /// nothing reaches more. Named once here and used by three tests, so that a
    /// reader who wants to disagree with the number has one colour to check.
    const TIGHTEST_GROUND: Rgb = Rgb::new(0x00, 0x89, 0x09);

    #[test]
    fn every_ground_admits_a_readable_ink() {
        // The fact the clamp policy rests on, and the exhaustion is over
        // *luminances* rather than over colours. That is the whole repair: an
        // earlier version swept 256 greys and a stride over the cube and argued
        // that any colour's luminance "lands in the interval the greys cover" —
        // true, and not the same as landing on a swept value. The minimum of
        // `best_reachable_x1000` lies strictly between the greys `#757575` and
        // `#767676`, so the sweep stepped over it and the ceiling it confirmed
        // was one part too high.
        //
        // Contrast depends on a ground only through its luminance, and
        // `LUMINANCE_X100000_MAX` bounds every luminance a colour can have, so
        // every integer in that range is a superset of the grounds that exist.
        // A superset with no gaps in it cannot step over a minimum.
        let mut tightest = u32::MAX;
        let mut tightest_at = 0;
        for luminance in 0..=LUMINANCE_X100000_MAX {
            let best = best_reachable_x1000(luminance);
            if best < tightest {
                tightest = best;
                tightest_at = luminance;
            }
        }
        // Equality, not `>=`. An inequality passes when the constant is set too
        // low as well as when it is right, and a ceiling nobody can reach is as
        // wrong as one somebody can beat.
        assert_eq!(tightest, REACHABLE_X1000, "the ceiling is {tightest} at {tightest_at}");
        const { assert!(CONTRAST_TEXT_X1000 < REACHABLE_X1000) };
        const { assert!(CONTRAST_NONTEXT_X1000 < REACHABLE_X1000) };

        // And a colour that stands on it, so the bound above is not a statement
        // about luminances nothing has. `#008909` is that colour, it is the one
        // the old sweep walked past, and it is `HOSTILE_FLAT`'s ground.
        assert_eq!(luminance_x100000(TIGHTEST_GROUND), tightest_at);
        assert_eq!(best_reachable_x1000(luminance_x100000(TIGHTEST_GROUND)), REACHABLE_X1000);

        // The operational half: the clamp actually reaches the floor on real
        // colours, the hardest ink against a ground being the ground itself.
        let sweep = |ground: Rgb| {
            assert!(luminance_x100000(ground) <= LUMINANCE_X100000_MAX);
            let raised = raise_to_floor(ground, ground, CONTRAST_TEXT_X1000);
            assert!(raised.reached, "{ground:?} admits no readable ink");
            assert!(raised.contrast_x1000 >= CONTRAST_TEXT_X1000);
            assert!(best_reachable_x1000(luminance_x100000(ground)) >= REACHABLE_X1000);
        };
        sweep(TIGHTEST_GROUND);
        for value in 0..=u8::MAX {
            sweep(Rgb::new(value, value, value));
        }
        for r in 0..16u16 {
            for g in 0..16u16 {
                for b in 0..16u16 {
                    sweep(Rgb::new((r * 17) as u8, (g * 17) as u8, (b * 17) as u8));
                }
            }
        }
    }

    #[test]
    fn a_floor_above_the_ceiling_is_refused_rather_than_approximated() {
        // What makes `Note::ContrastUnreachable` a live branch rather than a
        // comment. Raise the floor past what any colour can reach against the
        // tightest ground there is, and the clamp says so instead of returning a
        // colour that does not work — which is the behaviour this module would
        // need on the day somebody argues for the enhanced contrast level.
        //
        // Why the shipped floors cannot reach it, said over the duties rather
        // than over the two constants, so that a third duty would have to answer
        // the same question. This is the clamp-rather-than-refuse argument in
        // one line: every floor this module holds anything to is reachable
        // against every ground there is.
        for duty in Duty::ALL {
            assert!(
                Floors::STANDARD.of(duty) < REACHABLE_X1000,
                "{} owes {} against a ceiling of {REACHABLE_X1000}",
                duty.name(),
                Floors::STANDARD.of(duty)
            );
        }

        let raised = raise_to_floor(TIGHTEST_GROUND, TIGHTEST_GROUND, 7_000);
        assert!(!raised.reached);
        // The equality is the tie between the constant and the clamp: against
        // this ground the best the clamp can do *is* the ceiling, so a ceiling
        // written one part high or one part low turns this red as well.
        assert_eq!(raised.contrast_x1000, REACHABLE_X1000);
        assert!(raised.colour == Rgb::BLACK || raised.colour == Rgb::WHITE);

        // And through `resolve` itself, which is the half that was missing.
        // `Note::ContrastUnreachable` has one producer and it is in
        // `resolve_with`; with the standard floors nothing can enter it, because
        // both of them are under the ceiling, so the note used to be witnessed
        // by a `Note` a test constructed by hand — which asserts that a struct
        // literal type-checks. `Floors` is an argument so that the branch is
        // entered by the code that would enter it in earnest, on the day
        // somebody adopts the enhanced level. Deleting the `push` below the
        // `!raised.reached` test in `resolve_with` now turns this red.
        let enhanced = Floors { text_x1000: 7_000, nontext_x1000: CONTRAST_NONTEXT_X1000 };
        let (_, report) = resolve_with(&HOSTILE_FLAT, enhanced);
        assert!(report.holds(Note::ContrastUnreachable {
            ink: Token::Text,
            ground: Token::Surface1,
            best_x1000: REACHABLE_X1000,
        }));
        // Every text ink on every ground, and the five of them are the whole of
        // what owes the raised floor: `edge` owes the non-text one, which was
        // left where it was, and is clamped rather than refused.
        assert_eq!(
            report.iter().filter(|note| matches!(note, Note::ContrastUnreachable { .. })).count(),
            15,
            "the floor above the ceiling reached a different set of pairs"
        );
        assert!(report.raised(Token::Edge, Token::Surface1));
        assert!(!report.is_clean());
    }

    #[test]
    fn no_colour_separates_two_grounds_on_both_sides() {
        // Why `Boundary`'s promise is one-sided, exhibited rather than asserted.
        // These two greys contrast 2.998 to 1 with each other — under the
        // non-text floor, so a rule between them is owed — and no colour in the
        // space clears that floor against both of them. The sweep is over every
        // luminance for the same reason the ceiling's is: a rule is a colour, a
        // colour's contrast depends only on its luminance, and a sample of
        // colours could step over the answer.
        let lower = luminance_x100000(Rgb::new(0x51, 0x51, 0x51));
        let upper = luminance_x100000(Rgb::new(0x9F, 0x9F, 0x9F));
        assert_eq!(contrast_of_luminance_x1000(lower, upper), 2_998);
        assert!(contrast_of_luminance_x1000(lower, upper) < CONTRAST_NONTEXT_X1000);

        let mut best = 0;
        for rule in 0..=LUMINANCE_X100000_MAX {
            let worse = contrast_of_luminance_x1000(rule, lower)
                .min(contrast_of_luminance_x1000(rule, upper));
            best = best.max(worse);
        }
        assert_eq!(best, 2_646, "a two-sided rule became possible");
        assert!(
            best < CONTRAST_NONTEXT_X1000,
            "the boundary promise could be two-sided after all, and this module \
             is claiming less than it can deliver"
        );
    }

    #[test]
    fn a_theme_this_layer_agrees_with_is_not_touched() {
        // The anti-decorative assertion, and the one that stops this module from
        // passing its own exit by clamping everything into one safe palette. A
        // theme inside the bounds comes out byte for byte as it went in, and the
        // report has nothing to say.
        for (name, theme) in [("default", Theme::DEFAULT), ("dark", DARK)] {
            let (resolved, report) = resolve(&theme);
            assert!(report.is_clean(), "{name}: {report:?}");
            for (ink, ground, _) in pairs() {
                assert_eq!(resolved.on(ink, ground), theme.colour(ink), "{name}/{}", ink.name());
            }
            for metric in Metric::ALL {
                if let Some(asked) = theme.asked(metric) {
                    assert_eq!(resolved.metric(metric), asked, "{name}/{}", metric.name());
                }
            }
        }
    }

    #[test]
    fn two_legal_themes_resolve_to_two_different_interfaces() {
        // The other half of the same argument. Both survive untouched *and* they
        // disagree about every colour and every metric, so a theme is still
        // choosing something.
        let (light, _) = resolve(&Theme::DEFAULT);
        let (dark, _) = resolve(&DARK);
        for (ink, ground, _) in pairs() {
            assert_ne!(
                light.on(ink, ground),
                dark.on(ink, ground),
                "{} on {} is the same in both themes",
                ink.name(),
                ground.name()
            );
        }
        assert_ne!(light.em_pt_x10(), dark.em_pt_x10());
        assert_ne!(light.space_pt_x10(), dark.space_pt_x10());
        assert_ne!(light.font().iter().next(), dark.font().iter().next());
    }

    #[test]
    fn the_demonstration_tree_is_declarable() {
        // Everything below is resolved against this tree, so it has to be one an
        // author could actually write. Otherwise the demonstration is against a
        // shape the vocabulary refuses.
        let tree = panel();
        assert_eq!(check(&tree), Ok(()));
    }

    #[test]
    fn a_hostile_theme_cannot_produce_an_unreadable_interface() {
        // The first half of `E3-D03`'s exit. Every ink clears its floor on every
        // ground, and every node of a real tree paints in a pair that clears the
        // floor *the node* owes — including the node wearing a token name that
        // does not exist, the node wearing an ink its role may not wear, and the
        // two nodes whose ground is not the one their ink was paired with.
        let tree = panel();
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            for (ink, ground, floor) in pairs() {
                let got = resolved.contrast_x1000(ink, ground);
                assert!(
                    got >= floor,
                    "{name}: {} on {} is {got} against a floor of {floor}",
                    ink.name(),
                    ground.name()
                );
            }
            for node in &tree {
                let paint = resolved.paint(node);
                // The floor comes from the node's role and from the two
                // constants, and from nothing a theme can reach. What stood here
                // was `paint.ink.pairing().floor_x1000`, which let the token the
                // node happened to wear supply its own standard: the hint wears
                // `edge`, `edge` owes three to one, and a theme that sets
                // `surface-1` to `#767676` took that node to 3 032 with this
                // assertion green. A floor an attacker can move is not a floor.
                let floor = match Duty::of(node.role) {
                    Duty::Read => CONTRAST_TEXT_X1000,
                    Duty::See => CONTRAST_NONTEXT_X1000,
                };
                assert!(
                    paint.floor_x1000 >= floor,
                    "{name}: node {} is held to {} against a role that owes {floor}",
                    node.id.value(),
                    paint.floor_x1000
                );
                assert!(
                    paint.contrast_x1000 >= floor,
                    "{name}: node {} paints at {} against a floor of {floor}",
                    node.id.value(),
                    paint.contrast_x1000
                );
                // Measured from the two colours a compositor would actually put
                // on the screen, rather than from the method that chose them.
                // The two `assert_eq!`s that stood here re-evaluated `paint`'s
                // own expressions with `paint`'s own arguments and could not go
                // red for any edit; this catches the shape of bug that one was
                // written for — a ratio computed from one pair and labelled with
                // another — because the ratio and the floor now come from
                // different places.
                assert!(
                    contrast_x1000(paint.ink_rgb, paint.ground_rgb) >= floor,
                    "{name}: node {} paints a pair at {} against a floor of {floor}",
                    node.id.value(),
                    contrast_x1000(paint.ink_rgb, paint.ground_rgb)
                );
                assert_eq!(
                    paint.contrast_x1000,
                    contrast_x1000(paint.ink_rgb, paint.ground_rgb),
                    "{name}: node {} reports a ratio for a pair it is not painting",
                    node.id.value()
                );
            }
        }
    }

    #[test]
    fn a_theme_cannot_lower_the_floor_by_moving_a_token() {
        // Round two's blocking finding, and the shape of it is what is worth
        // keeping in front of a reader: the defect was not a wrong number, it
        // was a floor the attacker chose. `node.rs`'s `check` says nothing about
        // `style`, so `Role::Text` wearing `edge` is a declarable tree; `edge`
        // owes three to one; and the assertion the exit rested on read its floor
        // off `paint.ink.pairing()`. So the standard travelled with the token
        // the theme had just moved, and the same node reading 4 542 under the
        // default theme and 3 032 under the hostile one was green both times.
        let hint = panel().into_iter().find(|node| node.id == NodeId::new(10)).expect("the hint");
        assert_eq!(hint.role, Role::Text, "the attacking node stopped carrying text");
        assert!(
            hint.style.contains(Token::Edge.name()),
            "the attacking node stopped wearing the ink that made the attack"
        );

        // The two numbers the finding was written from, so that the repair is
        // checkable against what it repaired. `edge` *as a rule* is still
        // allowed to be under the text floor — that is what the second floor is
        // for — and the hostile theme still takes it there.
        let (default, _) = resolve(&Theme::DEFAULT);
        let (hostile, _) = resolve(&HOSTILE);
        assert_eq!(default.contrast_x1000(Token::Edge, Token::Surface1), 4_542);
        assert_eq!(hostile.contrast_x1000(Token::Edge, Token::Surface1), 3_032);
        assert!(hostile.contrast_x1000(Token::Edge, Token::Surface1) < CONTRAST_TEXT_X1000);

        // And what the node gets is not that. The ink is declined by name, the
        // role's own ink is used, and the pair clears the text floor under every
        // theme — against the constant, which no theme reaches, rather than
        // against a floor read back out of the token.
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            // The floors the table was built against are the ones this module
            // ships. A `resolve` that quietly lowered one would take every floor
            // asserted below with it, and nothing else here would notice.
            assert_eq!(resolved.floors(), Floors::STANDARD, "{name}");
            let paint = resolved.paint(&hint);
            assert_eq!(paint.refused, Some(Token::Edge), "{name}: the ink was honoured");
            assert_eq!(paint.ink, Token::ink_for(Role::Text), "{name}");
            assert_eq!(paint.ink_chosen, Chosen::ByRole, "{name}");
            assert_eq!(paint.duty, Duty::Read, "{name}");
            assert_eq!(paint.floor_x1000, CONTRAST_TEXT_X1000, "{name}");
            assert!(
                paint.contrast_x1000 >= CONTRAST_TEXT_X1000,
                "{name}: the hint paints at {}",
                paint.contrast_x1000
            );
            assert!(
                contrast_x1000(paint.ink_rgb, paint.ground_rgb) >= CONTRAST_TEXT_X1000,
                "{name}: the hint's own two colours are {}",
                contrast_x1000(paint.ink_rgb, paint.ground_rgb)
            );
        }

        // The refusal is a refusal and not a blanket: the same ink on the one
        // role whose content *is* an edge is honoured, under the hostile theme,
        // at the lower floor. A `paint` that declined `edge` everywhere would
        // pass everything above and would have deleted the non-text floor.
        let rule = panel().into_iter().find(|node| node.id == NodeId::new(6)).expect("the rule");
        let painted = hostile.paint(&rule);
        assert_eq!(painted.duty, Duty::See);
        assert_eq!(painted.ink, Token::Edge);
        assert_eq!(painted.refused, None);
        assert_eq!(painted.floor_x1000, CONTRAST_NONTEXT_X1000);
        assert!(painted.contrast_x1000 < CONTRAST_TEXT_X1000, "the rule became text");
    }

    #[test]
    fn a_node_that_wears_nothing_is_already_at_its_own_floor() {
        // What makes the refusal above safe rather than circular.
        // `Resolved::paint` answers a refused ink with `Token::ink_for(role)`,
        // so if a role's own ink were held to less than the role's duty owes,
        // the fallback would land on an ink that fails the floor it fell back
        // for. `Duty::of` and `ink_for` are separate matches on purpose — a
        // twenty-third role has to answer both — and this is where they are made
        // to agree rather than observed to.
        for role in Role::ALL {
            let duty = Duty::of(role);
            let pairing = Token::ink_for(role).pairing().expect("a role's ink is an ink");
            assert!(
                pairing.floor_x1000() >= duty.floor_x1000(),
                "{}: its own ink owes {} against a role that owes {}",
                role.name(),
                pairing.floor_x1000(),
                duty.floor_x1000()
            );
        }

        // The same thing said as a paint, under the theme that attacks colour:
        // a node wearing nothing is never refused and never under its floor,
        // whatever its role.
        let (resolved, _) = resolve(&HOSTILE);
        for role in Role::ALL {
            let node = Node::new(NodeId::new(1), NodeId::UNNAMED, role);
            let paint = resolved.paint(&node);
            assert_eq!(paint.refused, None, "{}", role.name());
            assert_eq!(paint.ink_chosen, Chosen::ByRole, "{}", role.name());
            assert!(
                paint.contrast_x1000 >= paint.floor_x1000,
                "{}: paints at {} against {}",
                role.name(),
                paint.contrast_x1000,
                paint.floor_x1000
            );
        }
    }

    #[test]
    fn a_region_that_sits_on_another_is_always_told_from_it() {
        // The half of "unreadable" that a token layer checking only inks against
        // grounds never asks about. `surface-2` is a raised ground *on*
        // `surface-1`; `field` is a ground inside whatever holds it. Those are
        // pairs, and WCAG 1.4.11 — the criterion `CONTRAST_NONTEXT_X1000` is
        // taken from — is about exactly them. Before `Resolved::boundary`,
        // `HOSTILE_FLAT` passed every test in this file with all three grounds
        // set to one colour.
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            for (over, under) in boundaries() {
                let boundary = resolved.boundary(over, under);
                assert_eq!(boundary.over, over);
                assert_eq!(boundary.under, under);
                assert_eq!(
                    boundary.self_evident,
                    boundary.grounds_x1000 >= CONTRAST_NONTEXT_X1000,
                    "{name}: {} on {} disagrees with itself",
                    over.name(),
                    under.name()
                );
                // The promise. Either the grounds part on their own, or there is
                // a rule that clears the non-text floor against the region it
                // encloses. There is no third answer and no theme that reaches
                // one.
                assert!(
                    boundary.self_evident || boundary.edge_x1000 >= CONTRAST_NONTEXT_X1000,
                    "{name}: {} on {} has no boundary: grounds {}, rule {}",
                    over.name(),
                    under.name(),
                    boundary.grounds_x1000,
                    boundary.edge_x1000
                );
                assert_eq!(boundary.edge_rgb, resolved.on(Token::Edge, over), "{name}");
            }
        }

        // A theme whose three grounds are one colour: nothing parts on its own,
        // every boundary is owed a rule, and every rule is there. This is the
        // assertion the flat theme exists for, and the one that goes red if
        // `self_evident` is ever computed as anything but the grounds' own ratio.
        let (flat, _) = resolve(&HOSTILE_FLAT);
        for (over, under) in boundaries() {
            let boundary = flat.boundary(over, under);
            assert_eq!(boundary.grounds_x1000, 1_000, "{} on {}", over.name(), under.name());
            assert!(!boundary.self_evident);
            assert!(boundary.edge_x1000 >= CONTRAST_NONTEXT_X1000);
            assert_eq!(boundary.edge_x1000, boundary.edge_under_x1000, "one colour, two answers");
        }

        // And the theme where the *column* matters, which the flat one cannot
        // test: two grounds 2.645 to 1 apart, so a rule is owed and the two
        // grounds resolve `edge` to two different colours. The rule is the raised
        // ground's, and the last assertion is what keeps that from being a
        // coincidence — it says in so many words that the other column would
        // fail, so an edit taking it turns this test red rather than passing
        // quietly.
        let (alike, _) = resolve(&HOSTILE_ALIKE);
        let boundary = alike.boundary(Token::Surface2, Token::Surface1);
        assert!(!boundary.self_evident, "the grounds parted after all");
        assert_eq!(boundary.grounds_x1000, 2_645);
        assert!(boundary.edge_x1000 >= CONTRAST_NONTEXT_X1000);
        assert_eq!(boundary.edge_rgb, alike.on(Token::Edge, Token::Surface2));
        assert_ne!(
            alike.on(Token::Edge, Token::Surface1),
            alike.on(Token::Edge, Token::Surface2),
            "the two columns agree, and the choice below has stopped being a choice"
        );
        assert!(
            contrast_x1000(alike.on(Token::Edge, Token::Surface1), alike.ground(Token::Surface2))
                < CONTRAST_NONTEXT_X1000,
            "the lower ground's rule would have worked, and this test no longer \
             distinguishes the two columns"
        );
        // The side that is reported rather than promised, read off the ground it
        // names. Here it happens to hold; the test above this one is the pair
        // where nothing could make it hold.
        assert_eq!(boundary.edge_under_x1000, contrast_x1000(boundary.edge_rgb, Rgb::BLACK));

        // And the tree, which is where it has to hold to mean anything: the
        // output group declares `surface-2` inside a surface declaring
        // `surface-1`, and the entry sits on `field`. Every node's ground against
        // its parent's, under every theme.
        let tree = panel();
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            for node in &tree {
                let Some(parent) = tree.iter().find(|other| other.id == node.parent) else {
                    continue;
                };
                let boundary =
                    resolved.boundary(resolved.paint(node).ground, resolved.paint(parent).ground);
                assert!(
                    boundary.self_evident || boundary.edge_x1000 >= CONTRAST_NONTEXT_X1000,
                    "{name}: node {} has no boundary against node {}",
                    node.id.value(),
                    parent.id.value()
                );
            }
        }
    }

    #[test]
    fn an_ink_where_a_ground_belongs_is_still_a_checked_pair() {
        // `Resolved::on`'s documentation advertises the fallback, and nothing
        // exercised it. Worse, the fallback disagreed with itself: the column
        // came from the *ink* argument's pairing and the ground colour from the
        // *ground* argument's, so the ratio returned described neither pair.
        // Under `HOSTILE`, `contrast_x1000(Edge, FieldDanger)` was 1 524 —
        // reported through a method whose documentation says it is never below
        // the ink's floor.
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            for ink in Token::ALL.into_iter().filter(|token| !token.is_ground()) {
                let floor = ink.pairing().expect("an ink has a pairing").floor_x1000();
                for other in Token::ALL.into_iter().filter(|token| !token.is_ground()) {
                    let got = resolved.contrast_x1000(ink, other);
                    assert!(
                        got >= floor,
                        "{name}: {} against {} is {got}, under a floor of {floor}",
                        ink.name(),
                        other.name()
                    );
                    // And the answer is the ink's own pair, not some third thing.
                    assert_eq!(got, resolved.contrast_x1000(ink, ground_of(ink)), "{name}");
                }
            }
        }
    }

    #[test]
    fn every_note_this_module_can_make_is_reached_by_a_test() {
        // The teeth here are the *match*, not the count. It is exhaustive and has
        // no wildcard, so a fifth `Note` cannot be added without somebody saying
        // in this test which attack produces it — which is the question worth
        // forcing, because a note nothing reaches is a note that is wrong and
        // nobody finds out. The count is what makes the answer checkable, and
        // every one of the four is now witnessed by a resolution.
        let mut seen = [false; 4];
        let mut witness = |note: Note| {
            let at = match note {
                Note::ContrastRaised { .. } => 0,
                Note::ContrastUnreachable { .. } => 1,
                Note::MetricClamped { .. } => 2,
                Note::FontDropped { .. } => 3,
            };
            seen[at] = true;
        };
        for (_, theme) in THEMES {
            let (_, report) = resolve(&theme);
            for note in report.iter() {
                witness(note);
            }
        }
        // `ContrastUnreachable` is not reachable at this module's own floors —
        // both are under `REACHABLE_X1000`, which is the whole argument for
        // clamping rather than refusing — so it is reached the way it will be
        // reached in earnest: through `resolve_with`, at a floor above the
        // ceiling. What stood here handed the match a `Note` this test built
        // itself, which set the slot whatever `resolve` did and would have gone
        // on passing with the branch that produces it deleted.
        let (_, enhanced) = resolve_with(
            &HOSTILE_FLAT,
            Floors { text_x1000: 7_000, nontext_x1000: CONTRAST_NONTEXT_X1000 },
        );
        for note in enhanced.iter() {
            witness(note);
        }
        assert_eq!(seen, [true; 4], "a note this module can make is reached by nothing");
    }

    #[test]
    fn a_theme_may_not_erase_the_rule_it_owes() {
        // The collapse direction of the one metric that decides whether a
        // boundary exists at all.
        //
        // `Metric::Stroke` shared `Metric::Space`'s arm in `low`, and therefore
        // its floor of zero, whose comment is about padding and says nothing
        // about rules. So `stroke_em_x100 = 0` was inside every bound, produced
        // no `Note`, and left `Report::is_clean` true — while `Boundary` carries
        // a colour and no thickness, so the rule owed on every pair whose
        // `self_evident` is false had one source for how thick it was and that
        // source said nought; and `Resolved::floor_pt_x10` hands a `Duty::See`
        // node its own rule's thickness, so the separator's floor went with it.
        // Every hostile theme here attacked by inflation, so nothing pushed on
        // the low end. `HOSTILE_ERASED` is what pushes on it.
        let tree = panel();
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            let stroke = resolved.stroke_pt_x10();
            assert!(stroke > 0, "{name}: the rule this theme owes is {stroke} tenths of a point");
            for node in &tree {
                if matches!(Duty::of(node.role), Duty::See) {
                    let span = resolved.span(node);
                    assert!(
                        span.floor_pt_x10 >= stroke,
                        "{name}: node {} is a rule with a floor of {}",
                        node.id.value(),
                        span.floor_pt_x10
                    );
                }
            }
        }

        // The theme that asked for nothing, read out rather than asserted in
        // the abstract: it is clamped, and it is *told* that it was. A layer
        // that quietly repaired this would be one nobody could debug, which is
        // the same rule `every_change_is_reported` holds the colours to.
        let (erased, report) = resolve(&HOSTILE_ERASED);
        assert!(report.clamped(Metric::Stroke), "a stroke of nought was taken in silence");
        assert_eq!(erased.metric(Metric::Stroke), Metric::Stroke.low());
        assert!(!report.is_clean(), "a theme that erased the rule resolved clean");

        // And the floor is the smallest one that works rather than a number
        // somebody liked: at the smallest legal em, one hundredth of an em
        // truncates to nothing and two does not. If `Metric::Em`'s floor ever
        // moves, this is the line that says so.
        assert_eq!(Metric::Em.low() * (Metric::Stroke.low() - 1) / 100, 0);
        assert!(Metric::Em.low() * Metric::Stroke.low() / 100 > 0);
    }

    #[test]
    fn a_hostile_theme_cannot_produce_an_unlayoutable_interface() {
        // The second half. The em inside its bounds however the factors were
        // spelled; every node's minimum exactly what its declaration and its
        // floors say; every control with room for its own word inside its own
        // rules; every rule at least as thick as itself; no span inverted.
        //
        // The two assertions that used to stand here for spacing and stroke were
        // `>= 0` over a `Metric::low` of zero times a positive em — a thing that
        // cannot fail — and they were the entirety of what this file said about
        // `stroke`. What replaces them relates the stroke to what it has to fit
        // inside, which is what it was related to nowhere: at
        // `HOSTILE_INFLATED` the rule is 360 tenths of a point and the operable
        // floor was 720, so a control at its minimum was exactly two of its own
        // rules with no interior at all, and the tree's separator reported a
        // minimum of nought against the same 360-tenth rule.
        let tree = panel();
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            let em = resolved.em_pt_x10();
            assert!(em >= Metric::Em.low(), "{name}: em is {em}");
            assert!(em <= Metric::Em.high(), "{name}: em is {em}");
            let stroke = resolved.stroke_pt_x10();
            let operable = i64::from(resolved.operable_min_pt_x10());
            for node in &tree {
                let at = node.id.value();
                let span = resolved.span(node);
                assert!(
                    span.max_pt_x10 >= span.min_pt_x10,
                    "{name}: node {at} has an inverted span"
                );
                // The declaration in the unit it was written in, at *this*
                // theme's em rather than at the smallest legal one — the weaker
                // form was implied by the operable floor and tested nothing —
                // and the floors restated from the rule rather than read back
                // out of `Resolved::floor_pt_x10`, which is the thing under
                // test.
                let declared = i64::from(node.layout.min_em_x100) * i64::from(em) / 100;
                let mut floor = 0;
                if node.intent.is_some() {
                    floor = operable;
                }
                if node.role == Role::Separator {
                    floor = floor.max(i64::from(stroke));
                }
                assert_eq!(
                    i64::from(span.min_pt_x10),
                    declared.max(floor),
                    "{name}: node {at} is not the larger of its declaration and its floor"
                );
                assert_eq!(
                    i64::from(span.floor_pt_x10),
                    floor,
                    "{name}: node {at} reports a floor that is not the one it was held to"
                );
                assert_eq!(
                    span.raised_to_floor,
                    declared < floor,
                    "{name}: node {at} reports the wrong reason for its minimum"
                );
                if node.intent.is_some() {
                    // A control is a finger, and it is its own word *inside its
                    // own edge*. The first half was the round-one finding — an
                    // eighteen-point control holding seventy-two-point text. The
                    // second is round two's: at the inflated bound the two rules
                    // are the whole control, and both metrics are legal.
                    assert!(
                        span.min_pt_x10 >= CONTROL_MIN_PT_X10,
                        "{name}: operable node {at} is {} tenths of a point",
                        span.min_pt_x10
                    );
                    assert!(
                        i64::from(span.min_pt_x10) - 2 * i64::from(stroke) >= i64::from(em),
                        "{name}: operable node {at} is {} tenths of a point, which is {em} of \
                         text with two rules of {stroke} around it",
                        span.min_pt_x10
                    );
                }
                if matches!(Duty::of(node.role), Duty::See) {
                    // A node whose whole content is a rule is at least as thick
                    // as that rule. The separator's minimum was nought under
                    // every theme, including the one where its own rule is 360.
                    assert!(
                        span.min_pt_x10 >= stroke,
                        "{name}: node {at} is {} tenths of a point of a {stroke}-tenth rule",
                        span.min_pt_x10
                    );
                }
            }
        }
    }

    #[test]
    fn nothing_changes_in_silence_and_nothing_unchanged_is_reported() {
        for (name, theme) in THEMES {
            every_change_is_reported(name, &theme);
        }
    }

    #[test]
    fn every_hostility_is_named_in_the_report() {
        // `every_change_is_reported` asserts the *property*; this asserts that
        // the hostile theme actually exercises it, which is the difference
        // between a demonstration and a vacuous one. Every assertion below is one
        // of the attacks listed on `HOSTILE`.
        let (_, report) = resolve(&HOSTILE);
        assert!(!report.is_clean());
        assert_eq!(report.dropped(), 0);

        for metric in Metric::ALL {
            assert!(report.clamped(metric), "{} was not clamped", metric.name());
        }
        assert!(report.holds(Note::MetricClamped {
            metric: Metric::TextSize,
            asked: 0,
            given: 90,
        }));
        assert!(report.holds(Note::MetricClamped {
            metric: Metric::Density,
            asked: i32::MIN,
            given: 500,
        }));
        assert!(report.holds(Note::MetricClamped {
            metric: Metric::Space,
            asked: -30_000,
            given: 0,
        }));
        // The derived one. Nine points of text at half scale is four and a half
        // points of em, which is under the floor — and neither factor was out of
        // range by the time this was computed, which is the whole reason the
        // product is checked separately from them.
        assert!(report.holds(Note::MetricClamped { metric: Metric::Em, asked: 45, given: 90 }));

        assert!(report.raised(Token::Text, Token::Surface1), "an ink equal to its ground survived");
        assert!(report.raised(Token::TextMuted, Token::Surface1), "the legal-alone grey survived");
        assert!(report.raised(Token::Emphasis, Token::Surface2), "white on white survived");
        assert!(report.raised(Token::FieldText, Token::Field), "black on black survived");
        assert!(report.raised(Token::Edge, Token::Surface1), "an invisible rule survived");

        assert!(report.holds(Note::FontDropped { at: 0, why: NotAFamily::NoLetter }));
        assert!(report.holds(Note::FontDropped { at: 1, why: NotAFamily::Unprintable }));
        assert!(report.holds(Note::FontDropped { at: 2, why: NotAFamily::TooLong }));
    }

    #[test]
    fn the_inflated_theme_reaches_bounds_the_collapsing_one_cannot() {
        // Why there are two hostile themes. Every metric here is clamped from
        // above, the derived em included.
        let (resolved, report) = resolve(&HOSTILE_INFLATED);
        assert!(report.holds(Note::MetricClamped {
            metric: Metric::TextSize,
            asked: i32::MAX,
            given: 720,
        }));
        assert!(report.holds(Note::MetricClamped { metric: Metric::Em, asked: 2_880, given: 720 }));
        assert_eq!(resolved.em_pt_x10(), Metric::Em.high());
        assert_eq!(resolved.metric(Metric::Space), Metric::Space.high());
        assert_eq!(resolved.metric(Metric::Stroke), Metric::Stroke.high());
        // Its palette is the default's, so the colour half of the report must be
        // empty: an attack on one half of this layer does not produce noise from
        // the other, and a report that cried about everything would be no more
        // usable than one that said nothing.
        assert!(!report.iter().any(|note| matches!(
            note,
            Note::ContrastRaised { .. } | Note::ContrastUnreachable { .. }
        )));
        assert!(report.holds(Note::FontDropped { at: 1, why: NotAFamily::TooLong }));
    }

    #[test]
    fn a_clamp_is_not_a_collapse() {
        // The other way a token layer fails this exit: by satisfying every floor
        // and giving every token the same colour, at which point the theme is
        // decorative and the interface is unreadable in a different sense. The
        // hostile theme's one saturated colour stays red on every ground, and its
        // inks do not converge.
        let (resolved, _) = resolve(&HOSTILE);
        for ground in Token::ALL.into_iter().filter(|token| token.is_ground()) {
            let danger = resolved.on(Token::FieldDanger, ground);
            assert!(
                danger.r > danger.g && danger.r > danger.b,
                "the red stopped being red on {}: {danger:?}",
                ground.name()
            );
        }
        let on_light = [
            resolved.on(Token::Text, Token::Surface2),
            resolved.on(Token::FieldText, Token::Surface2),
            resolved.on(Token::FieldDanger, Token::Surface2),
        ];
        assert_ne!(on_light[0], on_light[1]);
        assert_ne!(on_light[1], on_light[2]);
        assert_ne!(on_light[0], on_light[2]);
    }

    #[test]
    fn a_ground_is_free_and_stays_free() {
        // The first category, asserted rather than merely described. No ground is
        // ever moved — including `HOSTILE_FLAT`'s, the tightest in the space,
        // which leaves the clamp eighty-two parts in a thousand of room — because
        // the ink is the half that moves, and a theme that could not choose its
        // own backgrounds would not be a theme.
        //
        // Free is not the same as unexamined, and for a while this module
        // conflated them: a ground is never *moved*, and since `Resolved::boundary`
        // it is evaluated against the grounds it sits on.
        // `a_region_that_sits_on_another_is_always_told_from_it` is that half.
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            for ground in Token::ALL.into_iter().filter(|token| token.is_ground()) {
                assert_eq!(
                    resolved.ground(ground),
                    theme.colour(ground),
                    "{name}: {} was moved",
                    ground.name()
                );
            }
        }
    }

    #[test]
    fn a_font_stack_always_resolves_to_something() {
        // The third category in the form that has nothing to report. The hostile
        // theme names three things and none of them is a name; the stack is still
        // one family long.
        let (hostile, report) = resolve(&HOSTILE);
        assert_eq!(hostile.font().preferences(), 0);
        assert_eq!(hostile.font().len(), 1);
        assert!(!hostile.font().is_empty());
        assert_eq!(hostile.font().iter().next(), Some(FontStack::LAST_RESORT));
        assert_eq!(report.iter().filter(|n| matches!(n, Note::FontDropped { .. })).count(), 3);

        // And it is a *last* resort rather than the only one: a theme that names
        // something usable keeps it, in front.
        let (default, _) = resolve(&Theme::DEFAULT);
        let stack: [&str; 3] = [
            default.font().iter().next().expect("a first family"),
            default.font().iter().nth(1).expect("a second family"),
            default.font().iter().nth(2).expect("the last resort"),
        ];
        assert_eq!(stack, ["Inter", "Noto Sans", FontStack::LAST_RESORT]);
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            let last = resolved.font().iter().last().expect("never empty");
            assert_eq!(last, FontStack::LAST_RESORT, "{name}: the last resort was displaced");
        }
    }

    #[test]
    fn a_name_outside_the_vocabulary_is_reported_and_not_guessed() {
        // The refusal half of the clamp-or-refuse rule. `brand.pink` is a
        // perfectly legal token *name* — `node.rs` accepted it — and this module
        // has no token for it. Guessing would put a colour nobody chose into an
        // interface; dropping it silently would leave the author wondering why
        // their token does nothing. So the node paints in its role's ink, and the
        // paint carries the name that did not resolve.
        let tree = panel();
        let (resolved, _) = resolve(&Theme::DEFAULT);
        let caption = tree.iter().find(|n| n.id == NodeId::new(9)).expect("the caption");
        let paint = resolved.paint(caption);
        assert_eq!(paint.ink, Token::ink_for(Role::Text));
        assert_eq!(paint.ink_chosen, Chosen::ByRole);
        assert_eq!(paint.ground_chosen, Chosen::ByRole);
        assert!(paint.unknown.is_some_and(|name| name.as_str() == "brand.pink"));
        // A name outside the vocabulary and an ink inside it that this node may
        // not wear are two different refusals, reported in two different fields,
        // because an author fixing the first looks for a typo and an author
        // fixing the second looks for the right token.
        assert_eq!(paint.refused, None);

        // A node that declared both halves gets both, and says so.
        let group = tree.iter().find(|n| n.id == NodeId::new(2)).expect("the group");
        let paint = resolved.paint(group);
        assert_eq!(paint.ground, Token::Surface2);
        assert_eq!(paint.ink, Token::Emphasis);
        assert_eq!(paint.ground_chosen, Chosen::Declared);
        assert_eq!(paint.ink_chosen, Chosen::Declared);
        assert_eq!(paint.unknown, None);
        assert_eq!(paint.refused, None);

        // And a node that declared nothing gets its role's ink on that ink's own
        // ground, which is the path most of a real tree takes.
        let label = tree.iter().find(|n| n.id == NodeId::new(3)).expect("the label");
        let paint = resolved.paint(label);
        assert_eq!(paint.ink, Token::Text);
        assert_eq!(paint.ground, Token::Surface1);
        assert_eq!(paint.ink_chosen, Chosen::ByRole);
    }

    #[test]
    fn the_operable_floor_is_out_of_a_themes_reach() {
        // The clearest member of the third category. There is no metric that
        // spells it, so there is nothing for a theme to set; and because the
        // absolute half is not in ems, a theme cannot lower it by shrinking the em
        // either. The mute toggle is the case that proves it: its author capped it
        // at a fifth of an em, and under every theme it comes out at the floor with
        // `max_raised_to_min` saying the cap was overruled.
        //
        // *The floor* rather than *the constant*, and the difference is the
        // finding this test used to assert away. It asserted `min_pt_x10 ==
        // CONTROL_MIN_PT_X10` under all themes including `HOSTILE_INFLATED`,
        // where the em is seventy-two points — an eighteen-point control holding
        // seventy-two-point text, written down as correct. The floor is the
        // larger of the absolute number and what the control has to contain,
        // and what it has to contain is its word *and the rules around it*.
        let tree = panel();
        let mute = tree.iter().find(|n| n.id == NodeId::new(5)).expect("the toggle");
        for (name, theme) in THEMES {
            let (resolved, _) = resolve(&theme);
            let floor = resolved.operable_min_pt_x10();
            let span = resolved.span(mute);
            assert!(span.raised_to_floor, "{name}: the toggle kept an unusable minimum");
            assert_eq!(span.min_pt_x10, floor, "{name}");
            assert_eq!(span.floor_pt_x10, floor, "{name}");
            assert!(span.max_raised_to_min, "{name}: the span stayed inverted");
            assert_eq!(span.max_pt_x10, floor, "{name}");
            assert!(floor >= CONTROL_MIN_PT_X10, "{name}: the absolute half was lowered");
            assert!(
                floor - 2 * resolved.stroke_pt_x10() >= resolved.em_pt_x10(),
                "{name}: the control has no room for its own text inside its own edge"
            );
        }

        // And both halves are live: under an ordinary theme the absolute number
        // wins, and under a theme that drives every metric to its bound what the
        // control has to contain does. A floor that was only ever one of the two
        // would pass every assertion above and fail this one.
        let (ordinary, _) = resolve(&Theme::DEFAULT);
        assert_eq!(ordinary.operable_min_pt_x10(), CONTROL_MIN_PT_X10);
        let (inflated, _) = resolve(&HOSTILE_INFLATED);
        assert_eq!(inflated.em_pt_x10(), 720);
        // Half an em of rule at a seventy-two-point em, which is the number the
        // floor did not count: it was 720, the em alone, so an operable node at
        // its minimum was exactly two of its own rules and nothing else.
        assert_eq!(inflated.stroke_pt_x10(), 360);
        assert_eq!(inflated.operable_min_pt_x10(), 1_440);
        assert_eq!(inflated.span(mute).min_pt_x10, 1_440);
        // A node already asking for more than the floor keeps its own number, so
        // the floor is a floor rather than a size.
        let (resolved, _) = resolve(&Theme::DEFAULT);
        let entry = tree.iter().find(|n| n.id == NodeId::new(7)).expect("the entry");
        let span = resolved.span(entry);
        assert!(!span.raised_to_floor);
        assert_eq!(span.min_pt_x10, 1_050);
        assert_eq!(span.max_pt_x10, Span::UNBOUNDED);
    }

    #[test]
    fn a_family_name_says_which_clause_refused_it() {
        assert_eq!(FamilyName::new(""), Err(NotAFamily::Empty));
        assert_eq!(FamilyName::new("   "), Err(NotAFamily::NoLetter));
        assert_eq!(FamilyName::new("-- 12 --"), Err(NotAFamily::NoLetter));
        assert_eq!(FamilyName::new("\u{7f}"), Err(NotAFamily::Unprintable));
        assert_eq!(FamilyName::new("a name of thirty-three characters"), Err(NotAFamily::TooLong));
        let family = FamilyName::new("Noto Sans").expect("a family name");
        assert_eq!(family.as_str(), "Noto Sans");
        assert_eq!(family.len(), 9);
        assert!(!family.is_empty());
    }
}
