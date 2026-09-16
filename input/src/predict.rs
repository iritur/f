// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where the pointer will be at the next scanout, and how far wrong that is
//! allowed to be in each of the two directions it can be wrong in.
//!
//! The stamp `stamp.rs` takes says when the user moved. The number the input
//! path is judged by is `presented_at - stamped_at`, and by the time a frame
//! reaches the glass that difference is already spent: the event crossed a
//! ring, waited in a queue, and the compositor is now composing a frame that
//! will be scanned out some milliseconds from now. Nothing in this tree can
//! give those milliseconds back. What it can do is put the cursor where the
//! finger *will be* when the light comes out, which is this module.
//!
//! # The error is not one number, and this is the whole design
//!
//! A prediction is wrong in one of two ways and a user experiences them
//! differently, so they are bounded differently and they are never added
//! together.
//!
//! **Under-prediction is lag.** The cursor is behind the finger. It is the
//! failure the system has anyway — a predictor that predicts nothing at all
//! under-predicts by the whole frame's travel — and it degrades smoothly: twice
//! the lag is twice as annoying, and nobody can point at the moment it
//! happened.
//!
//! **Over-prediction is a snap-back.** The cursor runs past the finger and then
//! returns, and the return is a motion the user did not make. It does not
//! degrade smoothly, because the eye is a change detector: a cursor that
//! arrives late is a cursor, and a cursor that arrives early and reverses is a
//! *glitch*, which is a different category of complaint. It is also paid twice,
//! once going out and once coming back.
//!
//! So [`Deviation`] has two unsigned fields and no combined magnitude. The
//! absent accessor is deliberate and is the point of the type: the only thing
//! anyone would do with a single absolute error is compare it against a single
//! bound, and a single bound is the thing this module exists to refuse. It is
//! refused structurally — there is no method to call — rather than by a comment
//! asking reviewers to be careful.
//!
//! [`Bounds`] then carries the two numbers as a pair, and [`Bounds::stated`]
//! refuses a pair whose over-prediction bound is not *strictly tighter* than
//! its under-prediction bound. Written as a `const`, which is how the tests
//! below write it, that refusal is a compile error. An edit that collapsed the
//! asymmetry — the two numbers made equal, which is a single symmetric bound
//! wearing two names — does not build.
//!
//! The refusal is also driven to red at runtime, by `equal_bounds_are_refused`
//! and `a_looser_over_prediction_bound_is_refused`. That is not belt and
//! braces, it is the difference between a guard and a convention. This file
//! spent a round claiming the asymmetry was *removed rather than guarded*, and
//! it was not: relaxing the `<` to `<=` — one token — left every test in the
//! workspace green, so the next edit to set the two numbers equal would have
//! walked straight past a sentence that reads as protection. A
//! `#[should_panic]` test cannot be made green by weakening the comparison or
//! by deleting the assertion, which is what puts the guard itself under guard.
//!
//! # Every knob is turned toward lag
//!
//! Four decisions, each of which trades over-prediction away and buys
//! under-prediction with the proceeds.
//!
//! **First order only.** Velocity, never acceleration. A second-order fit
//! predicts a decelerating hand as still decelerating and a reversing hand as
//! still reversing, which is exactly where the overshoot is worst and exactly
//! where a curve fit is most confident. The cost is real — the module
//! under-predicts through every acceleration, which is the start of every
//! flick — and it is the cost chosen on purpose.
//!
//! **A second difference that may only subtract.** The window's older half and
//! its newer half imply two rates, and the ratio between them is the whole use
//! this module makes of a second difference: it is clamped at one, so a hand
//! speeding up is extrapolated exactly as far as a hand at a steady speed, and
//! a hand slowing down is extrapolated less in proportion to how much it has
//! slowed. Two halves that disagree about the *direction* are [`Held::Reversed`]
//! and extrapolate nothing at all. This is not the second-order term the
//! paragraph above refuses: it can take distance away and it has no arithmetic
//! that can add any, which is the difference between measuring confidence and
//! predicting a curve. The confidence is one number for both axes, taken on
//! whichever axis the older half was moving faster along: shortening one axis
//! and not the other would bend the predicted path off the direction the window
//! actually says the pointer is going, and the slower axis of a mostly-straight
//! movement is quantisation noise that changes sign every few reports.
//!
//! **A baseline, not a difference.** The velocity comes from the oldest and
//! newest samples in a window of [`WINDOW_SAMPLES`], not from the last two. Two
//! adjacent samples make the estimate as noisy as one report's jitter divided
//! by one report interval, and the extrapolation then multiplies that noise by
//! the horizon — at a 16 ms horizon on a 500 Hz device, eight times. A window
//! divides it. The price is that the estimate is of the velocity around the
//! middle of the window rather than at its end, so it lags the truth under
//! acceleration, which shows up as more lag.
//!
//! **A damped lead.** The extrapolation covers [`LEAD_NUMERATOR`] parts in
//! [`LEAD_DENOMINATOR`] of the time to the scanout and not the whole of it.
//! This is the knob that makes the two bounds different numbers rather than the
//! same number measured twice: shortening every predicted displacement
//! subtracts from the overshoot at a deceleration and adds to the lag at a
//! steady speed, and the amount is the margin the over-prediction bound below
//! is bought with. Claim the whole lead and the margin is zero.
//!
//! What four lag-ward knobs do *not* buy — and what an earlier version of this
//! file claimed they did — is monotonicity under a worse *input*. A knob is
//! turned; an input arrives. Halve the report rate over the same motion and the
//! worst over-prediction **rises**, on 1484 of the 4096 recordings the tests
//! below sweep and on the committed one, from 145 476 to 157 576 — against
//! 71 463 of extra lag on that same recording. The mechanism is not subtle: a
//! longer baseline lags the truth by more, and a velocity estimate that lags is
//! not merely late but *wrong*, wrong in whichever direction the hand has since
//! turned, and an extrapolation of a wrong velocity can run on. So the true
//! sentence is that degradation is spent *mostly* on lag, in a ratio this file
//! writes down rather than asserts in prose, and
//! `halving_the_report_rate_costs_mostly_lag_and_some_overshoot` is where the
//! two numbers live. The test that stood there before was named for the
//! stronger claim and checked only that the over-prediction stayed under a
//! bound that happened to have twenty per cent of slack in it.
//!
//! And every degradation path holds the last measured position rather than
//! guessing: see [`Held`]. When this module does not know, it lags. That is a
//! sentence about the code's shape and not a hope — [`Predicted::held`] is the
//! only constructor a [`Held`] reason can reach, and it copies the anchor.
//!
//! # The two numbers are a maximum over a named set, not a theorem
//!
//! The bounds the tests state are the worst deviation this predictor produced
//! over a swept set of generated recordings, rounded up to a whole pixel. They
//! are empirical, and the sentence they support is exactly this one: over those
//! recordings, at those report rates, nothing exceeded them. They are not
//! bounds over all motions. Draw more recordings and the maximum rises — over
//! eight of them the worst over-prediction is 4.8 px, over four thousand and
//! ninety-six it is 7.2 — which is what a maximum does, and is why the count
//! swept is written into the test beside the numbers it produced. An earlier
//! version of this file stated *one* recording's maximum as though it were a
//! bound, in prose that said the cursor *never* runs on by more than it; the
//! same generator exceeded that number on one draw in six.
//!
//! What a universal bound would have to look like is worth writing down,
//! because it is the reason there is not one here. Over-prediction does have a
//! closed form. A prediction is *ahead* only by some part of the step it took,
//! the step is at most the window's speed times the lead actually used, and
//! both of those are capped above — [`SPEED_CEILING_X65536_PER_MS`] times
//! [`LEAD_CEILING_NANOS`] damped by [`LEAD_NUMERATOR`]/[`LEAD_DENOMINATOR`],
//! which is 576 px on one axis. That is a theorem,
//! `the_only_universal_over_prediction_bound_is_the_two_ceilings_multiplied`
//! keeps it tied to the three constants it is derived from, and it is eighty
//! times the worst the sweep measured — true, and too loose to be a design
//! statement, which is the trade the whole section is about. Under-prediction has
//! no closed form at all: the truth is wherever the hand went, and a hand can
//! leave. So the instrument is a corpus, and the honest sentence names the
//! corpus.
//!
//! # The arithmetic, and its rounding
//!
//! Integers throughout, in the units `f_abi::input::PointerMotion` already
//! uses: position is 16.16 fixed point in an `i32`, time is nanoseconds in a
//! `u64`. There is no binary floating point of either width anywhere here, for
//! RFC 0004's reason and for a sharper one: the two architectures do not agree
//! on that arithmetic, and a cursor that landed on a different pixel on AArch64
//! than on x86-64 would be this whole path failing in the one way nobody would
//! think to look for.
//!
//! One step, given the window's oldest and newest samples and a scanout:
//!
//! ```text
//! lead    = scanout - newest.at                           (nanoseconds)
//! damped  = lead * LEAD_NUMERATOR / LEAD_DENOMINATOR      (nanoseconds)
//! rate    = half's displacement * 1e6 / half's span   (1/65536 px per ms)
//! trust   = min(1024, 1024 * |newer rate| / |older rate|)      (1024ths)
//! used    = damped * trust / 1024                         (nanoseconds)
//! base    = newest.at - oldest.at                         (nanoseconds)
//! step    = (newest.pos - oldest.pos) * used / base         (1/65536 px)
//! answer  = newest.pos + step                               (1/65536 px)
//! ```
//!
//! Every division truncates toward zero, and that direction is chosen rather
//! than inherited. Truncation shrinks `damped`, then `used`, then `step`, so
//! the rounding error lands on the lag side at every stage but one — `trust`'s
//! denominator, where a truncated older rate raises the ratio by at most a part
//! in a million and is then truncated back out by the multiplication below it.
//! Rounding to nearest would be a fraction of a 65 536th of a pixel more
//! accurate and would put half of that fraction on the overshoot side; the
//! asymmetry above says take the lag, and a rule that is followed even where it
//! does not matter is a rule that is still legible where it does.
//!
//! Overflow is conceivable in two places and happens in neither.
//! `displacement * 1e6` is at most a full `i32` span of pixels times a million,
//! which is forty bits short of `i64::MAX`; and `(newest.pos - oldest.pos) *
//! used` is bounded because the speed ceiling checked first holds the
//! displacement to [`SPEED_CEILING_X65536_PER_MS`] times
//! [`BASELINE_CEILING_NANOS`] while `used` is bounded by
//! [`LEAD_CEILING_NANOS`]. The ceiling is not only a plausibility check; it is
//! what makes that multiplication safe without a widening type.
//!
//! # Nothing here reads a clock, draws a value, or holds an `Env`
//!
//! [`Predictor`] is a pure function of the samples it was handed and the
//! scanout it was asked about. That is stronger than drawing deterministically
//! from a seed: a function with no source of entropy has nothing to seed and
//! nothing to reproduce. The seed in this file appears in exactly one place —
//! `CORPUS_SEED`, from which the corpus the tests measure against is generated
//! with `f_env::SeededEnv` and then written down as a `const` table, so that
//! the measurement reproduces on both architectures without a recording step,
//! and from which the sweep beside it draws every further recording by adding
//! an index.
//!
//! `cargo xtask lint-stamp` covers this file, and that is the second reason the
//! predictor takes no `Env`: a stage that could read the clock would sooner or
//! later compare the scanout against its own reading rather than against the
//! stamp the driver took, and the latency the path publishes would quietly
//! become the latency of a different event.
//!
//! A test that folded one recording twice and compared the two folds used to
//! stand below, named for this section. It has been deleted rather than
//! repaired, because no implementation could have made it fail: the crate
//! forbids `unsafe`, holds no statics, and [`Predictor`] is `Copy`, so two
//! folds over one table are identical whatever the arithmetic does. It read as
//! the check for this paragraph and observed nothing, which is worse than no
//! test at all. What checks the paragraph is `lint-stamp` above; what checks
//! the seeded draw is `the_recording_is_what_the_seed_produces`, which is the
//! one place an architecture that disagrees about `SeededEnv` can say so.
//!
//! # What this does not do
//!
//! It does not decode entries. There is no `From<f_abi::input::Entry>` here,
//! and the absence is deliberate: deciding which opcodes carry a position means
//! matching on an enum that is generated from one list in `abi/src/input.rs`,
//! and a seventh opcode would walk straight through the catch-all arm such a
//! match needs — which is the failure `interface/src/node.rs` records twice.
//! The stage that decodes an entry knows what it decoded; it builds a
//! [`Sample`] from it and hands it here.
//!
//! It also does not know when the next scanout is. That instant belongs to the
//! frame loop, is derived from the display's cadence, and arrives here as an
//! argument. A predictor that computed its own horizon would be a second
//! opinion about when the frame lands.
//!
//! # What would reverse this
//!
//! A display whose scanout instant is not known in advance — a variable refresh
//! panel driven by the compositor's own submit — at which point the horizon
//! stops being a time and becomes a distribution, and the two bounds below
//! become bounds on a distribution rather than on a corpus. Or a device that
//! reports its own velocity, which several digitisers do: then the window and
//! its baseline go away, the estimate stops lagging, and the damping argument
//! has to be re-made against a better estimator rather than inherited. Neither
//! reverses the asymmetry. The asymmetry is a fact about eyes.

use crate::stamp::StampNanos;

/// How many reports the velocity estimate is taken across.
///
/// Four, which on a 500 Hz device spans 6 ms and on a 125 Hz device spans 24 ms
/// — a baseline long enough to divide a single report's jitter by three and
/// short enough that the velocity it describes is still roughly the current
/// one. Widening it buys smoothness and pays for it in estimate lag, which is
/// to say in the under-prediction bound; narrowing it does the reverse and pays
/// in the over-prediction bound, because report noise extrapolated over a frame
/// is indistinguishable from a real flick.
/// Unit: samples.
pub const WINDOW_SAMPLES: usize = 4;

/// The numerator of the fraction of the lead that is actually extrapolated.
/// Unit: none — the top of a ratio, with [`LEAD_DENOMINATOR`] below it.
pub const LEAD_NUMERATOR: u64 = 3;

/// The denominator of that fraction.
///
/// Three quarters, so a quarter of every frame's travel is deliberately left
/// unclaimed. That quarter is the margin the over-prediction bound is bought
/// with — see the module's *every knob is turned toward lag* — and it is the
/// single number to move when the two bounds need to trade against each other.
/// Unit: none — the bottom of a ratio.
pub const LEAD_DENOMINATOR: u64 = 4;

// A predictor that claimed the whole lead would have no margin against a
// deceleration at all, and the two bounds it could then honestly state would be
// one number measured twice. The ratio is asserted rather than trusted because
// raising it is a one-character edit no test would name.
const _: () = assert!(LEAD_NUMERATOR > 0 && LEAD_NUMERATOR < LEAD_DENOMINATOR);

/// How far ahead of the newest sample this module is willing to predict.
///
/// Thirty-two milliseconds: a shade under two frames at 60 Hz, so a compositor
/// that has already missed one frame still gets a prediction for the next one.
/// Past that, the reason the question is being asked is that something upstream
/// stalled, and a velocity measured before a stall is the least trustworthy
/// input there is — so beyond this the answer is the last measured position,
/// which is lag, rather than a long extrapolation, which is a cursor in a place
/// the finger never went.
/// Unit: nanoseconds.
pub const LEAD_CEILING_NANOS: u64 = 32_000_000;

/// How long the window may span before its velocity is not about now.
///
/// Forty milliseconds. Four samples from a 125 Hz device — the slowest report
/// rate anything still ships — span 24 ms, so the ceiling has to sit above
/// that; it sits below the 45 ms that four samples from a device reporting at
/// 67 Hz would span, which is not a device, it is a device that is dropping
/// reports. Beyond this the window is a record of a gesture that has already
/// finished.
/// Unit: nanoseconds.
pub const BASELINE_CEILING_NANOS: u64 = 40_000_000;

/// The fastest the window may imply the pointer is moving before the whole
/// estimate is discarded.
///
/// Twenty-four pixels per millisecond is 24 000 pixels per second, perhaps
/// twice the fastest flick a hand produces on a high-resolution mouse. The
/// check is not a speed limit on users; it is a teleport detector. A pointer
/// warped by the compositor, a touch contact identifier reused for a second
/// finger, or a driver that reset its position accumulator all arrive here as
/// one enormous displacement between two adjacent reports, and extrapolating
/// that would fling the cursor off the display for a frame. Discarding it costs
/// one frame of lag.
///
/// It is checked per axis rather than on the diagonal, because the diagonal
/// needs a square root and a square root of a fixed-point quantity is where
/// somebody reaches for the arithmetic RFC 0004 forbids. That makes the check
/// up to a factor of the square root of two more permissive than the honest
/// one, and it is worth saying which direction that error runs in: a movement
/// at 45 degrees can reach about 34 pixels a millisecond before either of its
/// axes does. Nobody moves a hand that fast either, and a teleport — which is
/// what this exists to catch — exceeds the ceiling by three or four orders of
/// magnitude and is caught on every axis at once.
/// Unit: 1/65536 of a pixel per millisecond.
pub const SPEED_CEILING_X65536_PER_MS: i64 = 24 * 65_536;

/// Nanoseconds in a millisecond, for the speed check's units.
/// Unit: nanoseconds per millisecond.
const NANOS_PER_MS: i64 = 1_000_000;

/// Where the pointer was, and when.
///
/// The position is in `f_abi::input::PointerMotion`'s units exactly, because it
/// is usually that record's two fields: 16.16 fixed point, device pixels from
/// the surface origin, signed so that a pointer which has left the surface
/// still has a position. Nothing is converted on the way in, so nothing can be
/// converted wrongly.
///
/// A touch contact and a stylus tip are the same shape and use the same type.
/// What they are not is interchangeable *within one predictor*: a [`Predictor`]
/// tracks one thing that moves, and feeding it two fingers would make the
/// velocity the difference between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// When the device reported this position, on the stamp the driver took at
    /// interrupt time and on no other clock.
    /// Unit: nanoseconds, in the channel's epoch — `f_input::stamp`'s subject.
    pub at: StampNanos,
    /// Position along x.
    /// Unit: device pixels from the surface origin, scaled by 65 536. Zero is
    /// the origin, which is a real place.
    pub x_x65536: i32,
    /// Position along y.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub y_x65536: i32,
}

/// Why a prediction is the last measured position rather than an extrapolation.
///
/// Every variant means the same thing about the output — the cursor is placed
/// where the device last actually said it was — and they differ only in what
/// could not be believed. They are separate values because a compositor
/// permanently in one of them has a different problem in each case, and a log
/// line saying *held* would not tell a stalled ring from a device that has only
/// just been plugged in.
///
/// This is a nested enum rather than a flat set of [`Basis`] variants on
/// purpose. A reason for holding cannot be added in a way that moves the
/// cursor, because the only place a `Held` value can be turned into a
/// [`Predicted`] is [`Predicted::held`], which copies the anchor and sets the
/// lead to zero. The possibility is removed rather than guarded against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// The scanout is not after the newest sample. There is nothing to predict
    /// forward to, and the answer is the sample itself.
    NoLead,
    /// Fewer than two samples. One position is not a velocity.
    ShortHistory,
    /// The window's oldest and newest samples share an instant — a duplicated
    /// report, or two reports the driver stamped from one interrupt. Dividing
    /// by that is the one arithmetic trap in this module, and it is closed here.
    NoBaseline,
    /// The window spans more than [`BASELINE_CEILING_NANOS`]. The velocity in
    /// it is a fact about a gesture that has already ended.
    StaleWindow,
    /// The scanout is more than [`LEAD_CEILING_NANOS`] away. Something upstream
    /// is late, and a velocity from before it went late is the worst available
    /// basis for a guess.
    FarScanout,
    /// The window implies a speed past [`SPEED_CEILING_X65536_PER_MS`]. A
    /// teleport, not a movement.
    ImplausibleSpeed,
    /// The window's older half and its newer half disagree about which way the
    /// pointer is going, or the newer half says it has stopped. The motion
    /// turned inside the window, and the one thing that is certain about the
    /// velocity across it is that it is not the velocity now. This is the
    /// moment a predictor is most tempted and most wrong: it is the top of a
    /// flick, the end of a drag, the instant a finger settles, and every one of
    /// those extrapolates into a cursor that runs on and comes back.
    Reversed,
}

impl Held {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoLead => "the scanout is not after the newest report",
            Self::ShortHistory => "one position is not a velocity",
            Self::NoBaseline => "the window's ends share an instant, so it has no baseline",
            Self::StaleWindow => "the window spans longer than a velocity survives",
            Self::FarScanout => "the scanout is further ahead than a velocity may be believed",
            Self::ImplausibleSpeed => "the window implies a teleport rather than a movement",
            Self::Reversed => "the motion turned inside the window, so its velocity is not now's",
        }
    }
}

/// What a [`Predicted`] rests on.
///
/// There is exactly one variant that moves the cursor away from the last
/// measured position, and it is the one that had everything it needed. That
/// ratio is the design and not an accident of how many failure modes somebody
/// happened to think of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// A velocity across the window, extrapolated forward by the damped lead.
    Extrapolated,
    /// The last measured position, held, because the named thing could not be
    /// believed.
    Held(Held),
}

impl Basis {
    /// Did this basis move the cursor off the last measured position?
    ///
    /// True only for [`Basis::Extrapolated`], and true there even when the
    /// velocity was zero and the position did not in fact change — this answers
    /// *was an extrapolation performed*, which is the question a consumer
    /// deciding whether to trust the number is asking.
    #[must_use]
    pub const fn extrapolated(self) -> bool {
        matches!(self, Self::Extrapolated)
    }
}

/// Where the pointer is predicted to be at a scanout, and what that rests on.
///
/// The fields are private because a public field is a constructor, and a
/// constructor here would be a way to produce a [`Predicted`] whose basis says
/// `Held` and whose position is somewhere the device never reported — which is
/// precisely the state the module's *every knob is turned toward lag* claims
/// cannot exist. `stamp.rs` closes the same door on `StampNanos` for the same
/// reason, and it is worth the accessors here as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Predicted {
    /// Predicted position along x.
    x_x65536: i32,
    /// Predicted position along y.
    y_x65536: i32,
    /// The newest position the device actually reported.
    anchor_x_x65536: i32,
    /// And its other axis.
    anchor_y_x65536: i32,
    /// The lead actually extrapolated over, after damping. Zero when held.
    lead_nanos: u64,
    /// What the fields above rest on.
    basis: Basis,
}

impl Predicted {
    /// Predicted position along x.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    #[must_use]
    pub const fn x_x65536(&self) -> i32 {
        self.x_x65536
    }

    /// Predicted position along y.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    #[must_use]
    pub const fn y_x65536(&self) -> i32 {
        self.y_x65536
    }

    /// The newest position the device actually reported — what the prediction
    /// was measured out from, and what a consumer that decides not to trust a
    /// prediction should draw instead.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    #[must_use]
    pub const fn anchor_x_x65536(&self) -> i32 {
        self.anchor_x_x65536
    }

    /// The other axis of the anchor.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    #[must_use]
    pub const fn anchor_y_x65536(&self) -> i32 {
        self.anchor_y_x65536
    }

    /// How far forward this was actually extrapolated: the lead after the
    /// damping and after the window's own confidence, which is deliberately
    /// shorter than the time to the scanout and shorter again whenever the
    /// pointer was slowing down. Zero when the position was held.
    ///
    /// Exposed because it is the honest label for the number beside it. A
    /// consumer comparing this against the lead it asked for can see exactly
    /// how much of the frame the predictor declined to claim, which is a design
    /// decision it is entitled to disagree with rather than a fact hidden
    /// inside the arithmetic.
    /// Unit: nanoseconds.
    #[must_use]
    pub const fn lead_nanos(&self) -> u64 {
        self.lead_nanos
    }

    /// What this prediction rests on.
    #[must_use]
    pub const fn basis(&self) -> Basis {
        self.basis
    }

    /// The last measured position, held. The one door a [`Held`] reason can
    /// come through, and it cannot produce a moved cursor because it does not
    /// take a position.
    const fn held(newest: Sample, why: Held) -> Self {
        Self {
            x_x65536: newest.x_x65536,
            y_x65536: newest.y_x65536,
            anchor_x_x65536: newest.x_x65536,
            anchor_y_x65536: newest.y_x65536,
            lead_nanos: 0,
            basis: Basis::Held(why),
        }
    }
}

/// A window of recent reports, and the extrapolation over them.
///
/// Fixed size, no allocator, `Copy` samples: this is a ring of
/// [`WINDOW_SAMPLES`] slots and an index, meant to live inside whatever
/// per-device structure the stage that decodes entries already has. One
/// predictor per thing that moves — per pointer, per contact, per stylus — and
/// never one shared between two of them.
#[derive(Clone, Copy, Debug)]
pub struct Predictor {
    /// The ring. `None` until the slot has been written the first time.
    window: [Option<Sample>; WINDOW_SAMPLES],
    /// Where the next sample lands.
    next: usize,
    /// How many slots hold a sample, saturating at [`WINDOW_SAMPLES`].
    filled: usize,
}

impl Default for Predictor {
    fn default() -> Self {
        Self::new()
    }
}

impl Predictor {
    /// An empty window. Predicting from it answers `None`: there is no last
    /// measured position to fall back to, and inventing one would put a cursor
    /// at the origin before the device has said anything at all.
    #[must_use]
    pub const fn new() -> Self {
        Self { window: [None; WINDOW_SAMPLES], next: 0, filled: 0 }
    }

    /// How many reports the window holds.
    /// Unit: samples.
    #[must_use]
    pub const fn samples(&self) -> usize {
        self.filled
    }

    /// The newest report, if there has been one.
    #[must_use]
    pub const fn newest(&self) -> Option<Sample> {
        if self.filled == 0 {
            return None;
        }
        self.nth(self.filled - 1)
    }

    /// The `index`th report the window still holds, oldest first.
    ///
    /// The one place the ring's wrap is written. Every other reader — the
    /// oldest, the newest, the two halves the confidence is taken across —
    /// asks in oldest-first order and does not know there is a ring, which is
    /// what keeps the modulo arithmetic to a single line that can be wrong in
    /// only one way.
    const fn nth(&self, index: usize) -> Option<Sample> {
        if index >= self.filled {
            return None;
        }
        let start = if self.filled < WINDOW_SAMPLES { 0 } else { self.next };
        self.window[(start + index) % WINDOW_SAMPLES]
    }

    /// Take a report.
    ///
    /// Answers `false`, and stores nothing, for a sample that is not strictly
    /// newer than the newest one already held. A reordered or duplicated report
    /// must not become the newest, because the newest is the anchor every
    /// prediction is measured out from: accepting a stale one would move the
    /// cursor back to a position the user has already left, and the next report
    /// would move it forward again. That is a snap-back produced by the
    /// predictor itself rather than by a bad guess, which is the worse of the
    /// two.
    ///
    /// The dropped report is safe to ignore, which is why this is a plain
    /// `bool` and not a refusal a caller must handle: the window is unchanged
    /// and the next in-order report repairs everything. A caller that wants to
    /// count reordering — a driver auditing its own queue — has the answer.
    pub fn observe(&mut self, sample: Sample) -> bool {
        if let Some(newest) = self.newest()
            && sample.at <= newest.at
        {
            return false;
        }
        self.window[self.next] = Some(sample);
        self.next = (self.next + 1) % WINDOW_SAMPLES;
        if self.filled < WINDOW_SAMPLES {
            self.filled += 1;
        }
        true
    }

    /// Forget everything. For a gesture that ended — a contact lifted, a
    /// pointer that left the surface — where the next report starts a new
    /// motion and the velocity across the gap is not a velocity at all.
    pub const fn clear(&mut self) {
        *self = Self::new();
    }

    /// The oldest report still in the window.
    const fn oldest(&self) -> Option<Sample> {
        self.nth(0)
    }

    /// How much of the damped lead the window's own consistency justifies.
    ///
    /// The older half's rate against the newer half's, clamped so that it can
    /// only ever shorten — see the module's *a second difference that may only
    /// subtract*. The halves are the same number of intervals each, which is
    /// what stops the comparison from being a long smooth estimate against a
    /// single noisy interval. They overlap, and by how much depends on the
    /// parity: a single report when the window holds an odd number, a whole
    /// interval when it holds an even one, which is the shipped case — at
    /// [`WINDOW_SAMPLES`] of four the older half is reports 0 to 2 and the newer
    /// is 1 to 3, so they share the middle interval outright. That damps the
    /// ratio's sensitivity, in the direction of claiming more lead rather than
    /// less, and it is the price of halves that are equal in length; an odd
    /// window would buy the sharper comparison and pay for it in a shorter
    /// baseline. On a window of
    /// two reports there is only one interval and therefore nothing to compare,
    /// and the answer is full confidence, because *no evidence of slowing* is
    /// not *evidence of not slowing* but it is the only honest default for a
    /// factor that exists to take distance away.
    ///
    /// One factor for both axes, measured on whichever axis the older half was
    /// moving faster along. Two reasons, and the second one cost a rewrite. A
    /// single factor keeps the step collinear with the velocity the window
    /// measured, where shrinking one axis and not the other would bend the
    /// predicted path. And the axis that carries the motion is the only one
    /// whose rate means anything: a pointer travelling almost horizontally has
    /// a vertical rate that is quantisation noise, it changes sign constantly,
    /// and a factor that took the *smaller* of the two — which is what this
    /// function did first — threw away nearly every prediction the fast axis
    /// had earned. Measured over the recording below, that version recovered
    /// only a few per cent of the lag of not predicting at all, which is to say
    /// it was paying an over-prediction budget for nothing.
    fn trust_x1024(&self, newest: Sample) -> i64 {
        let half = self.filled / 2;
        let (Some(oldest), Some(middle), Some(newer_start)) =
            (self.nth(0), self.nth(half), self.nth(self.filled - 1 - half))
        else {
            return TRUST_X1024_FULL;
        };
        // Strictly positive: `observe` refuses a report that is not newer than
        // the newest held, so every pair of distinct slots differs in time.
        let older_span = widened(middle.at.since_nanos(oldest.at));
        let newer_span = widened(newest.at.since_nanos(newer_start.at));
        if older_span == 0 || newer_span == 0 {
            return TRUST_X1024_FULL;
        }
        let older_x = rate(i64::from(middle.x_x65536) - i64::from(oldest.x_x65536), older_span);
        let older_y = rate(i64::from(middle.y_x65536) - i64::from(oldest.y_x65536), older_span);
        let newer_x =
            rate(i64::from(newest.x_x65536) - i64::from(newer_start.x_x65536), newer_span);
        let newer_y =
            rate(i64::from(newest.y_x65536) - i64::from(newer_start.y_x65536), newer_span);
        if older_x.saturating_abs() >= older_y.saturating_abs() {
            axis_trust_x1024(older_x, newer_x)
        } else {
            axis_trust_x1024(older_y, newer_y)
        }
    }

    /// Where the pointer will be at `scanout`.
    ///
    /// `None` only when nothing has been reported yet. Every other answer
    /// carries a position: either an extrapolation, or the last measured
    /// position with a [`Held`] reason saying what could not be believed. There
    /// is no third outcome and in particular no error a caller has to decide
    /// what to draw for — a compositor asking this question is about to submit
    /// a frame and must put the cursor somewhere.
    #[must_use]
    pub fn predict_at(&self, scanout: StampNanos) -> Option<Predicted> {
        let newest = self.newest()?;

        // Saturating, inherited from `StampNanos::since_nanos`: a scanout at or
        // before the newest report is a zero lead rather than a negative one,
        // and the answer to *where will it be by then* when *then* has already
        // passed is where it is.
        let asked_nanos = scanout.since_nanos(newest.at);
        if asked_nanos == 0 {
            return Some(Predicted::held(newest, Held::NoLead));
        }
        if asked_nanos > LEAD_CEILING_NANOS {
            return Some(Predicted::held(newest, Held::FarScanout));
        }

        if self.filled < 2 {
            return Some(Predicted::held(newest, Held::ShortHistory));
        }
        let Some(oldest) = self.oldest() else {
            return Some(Predicted::held(newest, Held::ShortHistory));
        };

        let base_nanos = newest.at.since_nanos(oldest.at);
        if base_nanos == 0 {
            return Some(Predicted::held(newest, Held::NoBaseline));
        }
        if base_nanos > BASELINE_CEILING_NANOS {
            return Some(Predicted::held(newest, Held::StaleWindow));
        }

        let travel_x = i64::from(newest.x_x65536) - i64::from(oldest.x_x65536);
        let travel_y = i64::from(newest.y_x65536) - i64::from(oldest.y_x65536);
        if implausible(travel_x, base_nanos) || implausible(travel_y, base_nanos) {
            return Some(Predicted::held(newest, Held::ImplausibleSpeed));
        }

        // The damping. Multiplying before dividing keeps the lead exact to the
        // nanosecond; the ceiling above makes the product trivially small.
        let damped_nanos = asked_nanos * LEAD_NUMERATOR / LEAD_DENOMINATOR;

        // And the confidence, which can only shorten it further. Zero means the
        // window turned over inside itself, and there is no sensible fraction of
        // a velocity that has changed sign — so that is a refusal with a name
        // rather than a step of length zero wearing the label `Extrapolated`.
        let trust = self.trust_x1024(newest);
        if trust == 0 {
            return Some(Predicted::held(newest, Held::Reversed));
        }
        let used_nanos = damped_nanos * unsigned(trust) / unsigned(TRUST_X1024_FULL);
        let used = widened(used_nanos);
        let base = widened(base_nanos);

        // The last truncation. Both operands of the multiplication are bounded
        // by the two ceilings checked above, so this is nowhere near
        // `i64::MAX`; the division rounds toward zero, which shortens the step
        // and therefore errs toward lag.
        let step_x = travel_x * used / base;
        let step_y = travel_y * used / base;

        Some(Predicted {
            x_x65536: saturating_i32(i64::from(newest.x_x65536) + step_x),
            y_x65536: saturating_i32(i64::from(newest.y_x65536) + step_y),
            anchor_x_x65536: newest.x_x65536,
            anchor_y_x65536: newest.y_x65536,
            lead_nanos: used_nanos,
            basis: Basis::Extrapolated,
        })
    }
}

/// Full confidence: the newer half of the window is going at least as fast as
/// the older half, so nothing has been observed that would justify claiming
/// less of the lead.
///
/// A thousand and twenty-four rather than a hundred, because it is a shift and
/// because a percentage invites somebody to read the factor as a tuning dial
/// with a meaningful scale. It is a ratio of two measured rates, and the only
/// number in it that was chosen is the ceiling.
/// Unit: 1024ths, and never more than this.
const TRUST_X1024_FULL: i64 = 1024;

/// A displacement over a span, as a rate in the units the two halves are
/// compared in.
///
/// Milliseconds rather than nanoseconds in the denominator so the ratio below
/// has some resolution left in it: at nanoseconds a hand's velocity rounds to
/// zero and every window would compare zero against zero.
///
/// `span_nanos` must be non-zero. [`Predictor::trust_x1024`] is the only caller
/// and checks both spans immediately above the two calls, which is the one
/// place that guarantee can be read and the reason this takes a plain division
/// rather than carrying an `Option` no caller could act on.
/// Unit: 1/65536 of a pixel per millisecond.
fn rate(travel_x65536: i64, span_nanos: i64) -> i64 {
    travel_x65536.saturating_mul(NANOS_PER_MS) / span_nanos
}

/// The older half's rate against the newer half's, on one axis.
///
/// Clamped at [`TRUST_X1024_FULL`], which is the whole asymmetry in one line:
/// speeding up cannot buy extra distance, slowing down must give distance back,
/// and turning round gives all of it back.
fn axis_trust_x1024(older: i64, newer: i64) -> i64 {
    if older == 0 {
        // Starting from rest. There is no earlier rate to have fallen from, and
        // a hand that has just begun moving is the acceleration case, where this
        // module already under-predicts. Taking more away there would be paying
        // twice for one decision.
        return TRUST_X1024_FULL;
    }
    if newer == 0 || older.is_negative() != newer.is_negative() {
        return 0;
    }
    let ratio = newer.saturating_abs().saturating_mul(TRUST_X1024_FULL) / older.saturating_abs();
    if ratio > TRUST_X1024_FULL { TRUST_X1024_FULL } else { ratio }
}

/// A confidence factor back into the unsigned arithmetic the leads are in.
/// Bounded by [`TRUST_X1024_FULL`] and never negative, so the saturation is
/// unreachable for the same reason [`widened`]'s is.
fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Does this displacement over this span imply a teleport?
fn implausible(travel_x65536: i64, span_nanos: u64) -> bool {
    travel_x65536.saturating_abs().saturating_mul(NANOS_PER_MS)
        > widened(span_nanos).saturating_mul(SPEED_CEILING_X65536_PER_MS)
}

/// A nanosecond count as a signed quantity.
///
/// Every count reaching this has already been bounded by a ceiling, so the
/// saturation is unreachable; it is written rather than a cast because an
/// unreachable branch costs nothing and a cast whose wrapping nobody has
/// thought about costs whatever it eventually costs.
fn widened(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A position back into the wire's width, saturating rather than wrapping.
///
/// Saturation is the right failure here and wrapping is the wrong one: a
/// wrapped coordinate puts the cursor in the opposite corner of the coordinate
/// space, which is a frame of pure nonsense, where a saturated one puts it at
/// the far edge in the direction it was already going.
fn saturating_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value.is_negative() { i32::MIN } else { i32::MAX })
}

/// How wrong a prediction turned out to be, split by which way it was wrong.
///
/// Exactly one of the two is non-zero for a single axis. Both can be non-zero
/// for a single comparison, because a cursor can genuinely be past the finger
/// along x and short of it along y at the same instant, and both can be
/// non-zero once [`Deviation::worst`] has folded a recording — which is the
/// point, since a recording has a worst overshoot and a worst lag and they
/// happen at different moments.
///
/// # There is deliberately no combined magnitude
///
/// No `magnitude`, no `abs`, no `Ord`. The only use for a single number here is
/// to compare it against a single bound, and a single bound is defeated by the
/// predictor that does nothing at all: holding the last position over-predicts
/// by zero, always, so it wins any symmetric test against any predictor that
/// tries. `doing_nothing_has_a_perfect_ahead_bound` below is
/// that argument executed. The accessor is absent rather than discouraged,
/// because a discouraged accessor is one somebody calls.
///
/// # Why per axis, and why the two are added
///
/// The honest error is the length of a two-dimensional vector and needs a
/// square root, which for a 16.16 quantity is where somebody reaches for the
/// arithmetic RFC 0004 forbids. So each axis is measured on its own and the two
/// are **added**. Adding is the choice that matters, and taking the larger —
/// which was the first version of this — is the mistake it replaced: the larger
/// of the two axes is never bigger than the straight-line distance and can be a
/// factor of the square root of two smaller, so a bound stated that way would
/// quietly understate the error it claims to bound, by up to forty per cent, in
/// exactly the diagonal case a hand actually produces. The sum runs the other
/// way — never smaller than the straight-line distance, at most that same
/// factor larger — so a bound stated in these terms is the conservative
/// statement, and the arithmetic stays in integers on both architectures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deviation {
    /// How far the prediction ran past the truth, along the direction the
    /// pointer actually travelled.
    ahead_x65536: u32,
    /// How far the prediction fell short of the truth, along that same
    /// direction.
    behind_x65536: u32,
}

impl Deviation {
    /// A prediction that was exactly right, and the identity for
    /// [`Deviation::worst`].
    pub const NONE: Self = Self { ahead_x65536: 0, behind_x65536: 0 };

    /// How far the prediction ran past where the pointer actually went.
    /// Unit: 1/65536 of a device pixel.
    #[must_use]
    pub const fn ahead_x65536(self) -> u32 {
        self.ahead_x65536
    }

    /// How far the prediction fell short of where the pointer actually went.
    /// Unit: 1/65536 of a device pixel.
    #[must_use]
    pub const fn behind_x65536(self) -> u32 {
        self.behind_x65536
    }

    /// Measure a prediction against where the pointer turned out to be.
    ///
    /// The direction of travel — which is what makes *ahead* and *behind* mean
    /// anything — is taken from the prediction's own anchor to the truth: the
    /// pointer went from where it last actually was to where it actually ended
    /// up, and the prediction is ahead if it overshot along that direction and
    /// behind if it fell short.
    ///
    /// A truth equal to the anchor — the pointer did not move on that axis —
    /// makes any deviation *ahead*, and that is the substantive case rather
    /// than a degenerate one: the cursor was drawn somewhere the finger never
    /// was, and the next frame will snap it back. Assigning it to the tighter
    /// bound is the conservative reading and the one that matches what a user
    /// sees.
    #[must_use]
    pub fn between(predicted: &Predicted, truth_x_x65536: i32, truth_y_x65536: i32) -> Self {
        let x = axis(predicted.x_x65536, predicted.anchor_x_x65536, truth_x_x65536);
        let y = axis(predicted.y_x65536, predicted.anchor_y_x65536, truth_y_x65536);
        Self {
            ahead_x65536: x.ahead_x65536.saturating_add(y.ahead_x65536),
            behind_x65536: x.behind_x65536.saturating_add(y.behind_x65536),
        }
    }

    /// The worse of two deviations, in each direction separately.
    ///
    /// For folding a recording, where the two directions' worst moments are
    /// different moments. Separately, which is why this is not one comparison
    /// and a winner: a fold that kept whichever sample was worse overall would
    /// throw the other direction's worst case away, and the other direction's
    /// worst case is half of what this module is judged on.
    #[must_use]
    pub const fn worst(self, other: Self) -> Self {
        Self {
            ahead_x65536: if other.ahead_x65536 > self.ahead_x65536 {
                other.ahead_x65536
            } else {
                self.ahead_x65536
            },
            behind_x65536: if other.behind_x65536 > self.behind_x65536 {
                other.behind_x65536
            } else {
                self.behind_x65536
            },
        }
    }
}

/// One axis of [`Deviation::between`].
fn axis(predicted: i32, anchor: i32, truth: i32) -> Deviation {
    let travel = i64::from(truth) - i64::from(anchor);
    let error = i64::from(predicted) - i64::from(truth);
    let magnitude = u32::try_from(error.unsigned_abs()).unwrap_or(u32::MAX);
    // Opposite signs mean the prediction did not reach where the pointer went;
    // everything else — including a pointer that did not move on this axis, and
    // including an error of zero — counts toward the tighter bound.
    if error != 0 && travel != 0 && error.is_negative() != travel.is_negative() {
        Deviation { ahead_x65536: 0, behind_x65536: magnitude }
    } else {
        Deviation { ahead_x65536: magnitude, behind_x65536: 0 }
    }
}

/// A pair of bounds a [`Deviation`] may be held to, over-prediction first.
///
/// The type exists so that the two numbers travel together and so that
/// [`Bounds::stated`] can refuse the pair that would collapse them. Anyone who
/// wanted one symmetric number would have to write it twice, and writing it
/// twice does not compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// The over-prediction bound.
    ahead_x65536: u32,
    /// The under-prediction bound, which must be looser.
    behind_x65536: u32,
}

impl Bounds {
    /// State the two bounds, with the argument for each written beside the call.
    ///
    /// # Panics
    ///
    /// When the over-prediction bound is not strictly tighter than the
    /// under-prediction one. In a `const` — which is how a bound that gates
    /// anything should be written, and how the tests in this file write theirs
    /// — that is a compile error rather than a panic, and it is the whole
    /// reason this is a constructor instead of two public fields.
    ///
    /// The refusal is not a style preference. A cursor that runs ahead and
    /// snaps back is a glitch the eye finds by itself; a cursor that lags is a
    /// cursor. Equal bounds assert that those are the same experience, and if
    /// somebody genuinely believes that, the honest diff deletes this type and
    /// argues it in an RFC — not one that quietly passes the same number twice.
    #[must_use]
    pub const fn stated(ahead_x65536: u32, behind_x65536: u32) -> Self {
        assert!(
            ahead_x65536 < behind_x65536,
            "over-prediction must be bounded strictly tighter than under-prediction: \
             a snap-back is a different experience from lag, and equal bounds are one \
             symmetric bound wearing two names"
        );
        Self { ahead_x65536, behind_x65536 }
    }

    /// The over-prediction bound.
    /// Unit: 1/65536 of a device pixel.
    #[must_use]
    pub const fn ahead_x65536(self) -> u32 {
        self.ahead_x65536
    }

    /// The under-prediction bound.
    /// Unit: 1/65536 of a device pixel.
    #[must_use]
    pub const fn behind_x65536(self) -> u32 {
        self.behind_x65536
    }

    /// Is this deviation inside both bounds?
    #[must_use]
    pub const fn admits(self, deviation: Deviation) -> bool {
        deviation.ahead_x65536 <= self.ahead_x65536 && deviation.behind_x65536 <= self.behind_x65536
    }
}

#[cfg(test)]
mod tests {
    // The crate is `no_std`; the harness links `std` anyway, and this is what
    // puts the name in scope so the regeneration printer at the bottom can use
    // it. Under `#[cfg(test)]` only, so nothing shipped depends on it.
    extern crate std;

    use self::std::println;
    use super::*;
    use f_env::{Env, SeededEnv};

    /// One pixel, so the bounds below read as distances rather than as hashes.
    /// Unit: 1/65536 of a device pixel.
    const PX: u32 = 65_536;

    /// How often the recorded device reported.
    ///
    /// Two milliseconds: 500 Hz, an ordinary gaming mouse, and fast enough that
    /// the window spans 6 ms — well inside [`BASELINE_CEILING_NANOS`], so the
    /// recording exercises the extrapolation rather than the guards.
    /// `halving_the_report_rate_costs_mostly_lag_and_some_overshoot` runs the
    /// same motion at 250 Hz.
    /// Unit: nanoseconds.
    const SAMPLE_NANOS: u64 = 2_000_000;

    /// When the recording starts.
    ///
    /// Non-zero, because zero is `f_abi::input::NOT_STAMPED` on the wire, and a
    /// recording whose first sample could not be expressed as an entry would be
    /// a recording about a different system.
    /// Unit: nanoseconds.
    const EPOCH_NANOS: u64 = 5_000_000;

    /// The horizon every prediction below is made over.
    ///
    /// Sixteen milliseconds, one frame at 60 Hz, which is what *the next
    /// scanout* means on a display of that cadence, and [`HORIZON_SAMPLES`]
    /// reports of the recorded device. It has to be a whole number of samples
    /// so that the truth at the horizon is a recorded position rather than an
    /// interpolation between two — measuring a predictor against an
    /// interpolation would be measuring it against another predictor.
    /// Unit: nanoseconds.
    const HORIZON_NANOS: u64 = 16_000_000;

    /// The horizon, in recorded reports.
    /// Unit: samples.
    const HORIZON_SAMPLES: usize = (HORIZON_NANOS / SAMPLE_NANOS) as usize;

    /// How many reports the recording holds.
    /// Unit: samples.
    const SAMPLES: usize = 48;

    /// The seed the recording below was taken under, and the base every
    /// further recording of the sweep is drawn from by adding an index.
    const CORPUS_SEED: u64 = 0x3B04_C0DE;

    /// The kick the acceleration takes each report, in each direction.
    ///
    /// The scale that makes the walk look like a hand rather than like noise:
    /// with the two decays below, the recording reaches about a pixel and a
    /// half of travel per report, which at 500 Hz is some 800 pixels a second —
    /// an ordinary deliberate movement, neither a flick nor a crawl. It is the
    /// one number here that was chosen by looking at what came out, and the
    /// thing it was chosen against is the travel per frame — which is also the
    /// scale the two bounds below are read against, since what not predicting
    /// costs *is* the frame's travel. On the committed recording that is about
    /// fifteen pixels; across the sweep it reaches sixty-four, where the walk
    /// spends a frame against [`SPEED_LIMIT_X65536`] on both axes at once.
    /// Unit: 1/65536 of a pixel per report per report.
    const JERK_X65536: u64 = 3_600;

    /// How fast the acceleration decays toward zero: one part in this per
    /// report, so sixteen gives it a correlation time of about sixteen reports
    /// — 32 ms, two frames. A hand that starts speeding up keeps speeding up
    /// for about that long rather than jittering, and the recording contains
    /// several such phases, which is what puts both accelerations and
    /// decelerations inside it.
    const ACCEL_DECAY: i64 = 16;

    /// The same for the velocity, and far slower: some 90 ms of correlation, so
    /// that a hand which is moving keeps moving and the only thing that stops
    /// it is an acceleration the other way. This is what makes the recording
    /// predictable enough to be worth predicting at all — a velocity that
    /// decorrelated inside one frame would make every predictor equally bad and
    /// the measurement meaningless — and it is also the assumption the whole
    /// module rests on, stated here where it can be disagreed with.
    const VELOCITY_DECAY: i64 = 4_096;

    /// The acceleration the walk may not exceed, so that one unlucky run of
    /// draws cannot produce a recording about a different device.
    /// Unit: 1/65536 of a pixel per report per report.
    const ACCEL_LIMIT_X65536: i64 = 32_768;

    /// The speed the walk may not exceed: four pixels per report, which is
    /// 2000 pixels a second — fast but human, and well inside
    /// [`SPEED_CEILING_X65536_PER_MS`], so the recording never trips the
    /// teleport guard.
    /// Unit: 1/65536 of a pixel per report.
    const SPEED_LIMIT_X65536: i64 = 262_144;

    /// Where the recording starts. In the middle of a notional display rather
    /// than at the origin, so a sign error anywhere in the arithmetic shows up
    /// as a wrong number instead of being hidden by zero.
    /// Unit: 1/65536 of a pixel.
    const START_X_X65536: i64 = 400 * 65_536;

    /// The other axis of the start.
    /// Unit: 1/65536 of a pixel.
    const START_Y_X65536: i64 = 300 * 65_536;

    /// A recording: one position per report, on the grid [`stamp_of`] lays out.
    ///
    /// A type alias so that every helper below takes the recording it measures
    /// rather than reaching for [`CORPUS`] directly — which is what lets
    /// `the_recording_is_the_table_this_file_prints` run the whole measurement
    /// over a freshly generated recording and print the numbers to paste, so
    /// that re-deriving the table and the two constants after a change to the
    /// generator is a copy rather than a transcription.
    /// Unit: device pixels from the surface origin, scaled by 65 536, as
    /// (x, y).
    type Recording = [(i32, i32); SAMPLES];

    /// The motion the tests measure against: 48 positions, 2 ms apart.
    ///
    /// # How this was generated, and why it is written down
    ///
    /// By [`record`] under [`CORPUS_SEED`], once, and then pasted here. The
    /// generator is a jerk-limited random walk — a seeded kick to the
    /// acceleration each report, the acceleration decaying toward zero, the
    /// velocity integrating it — which is a crude model of a hand and has the
    /// two properties the measurement needs. The velocity persists for tens of
    /// milliseconds, so extrapolating it is not hopeless; and the motion
    /// accelerates, decelerates and reverses inside the recording, so both
    /// directions of error actually occur. A constant-velocity recording would
    /// give a predictor with no acceleration term a perfect score and prove
    /// nothing.
    ///
    /// It is a `const` table rather than a call to the generator for
    /// `stamp.rs`'s reason exactly: comparing two runs of one generator on one
    /// machine proves the generator is a function, which is not the question.
    /// These numbers were produced on x86-64, and
    /// `the_recording_is_what_the_seed_produces` is what makes the AArch64
    /// runner disagree out loud if `f_env::SeededEnv` computes something else
    /// there. None of them is a measurement of a machine, and none may leave
    /// this file.
    /// Unit: device pixels from the surface origin, scaled by 65 536, as
    /// (x, y).
    const CORPUS: Recording = [
        (26_215_265, 19_658_548),
        (26_214_868, 19_651_002),
        (26_215_129, 19_638_976),
        (26_218_002, 19_623_938),
        (26_222_589, 19_609_191),
        (26_227_508, 19_594_188),
        (26_231_913, 19_578_511),
        (26_233_144, 19_564_227),
        (26_232_423, 19_551_290),
        (26_230_317, 19_536_323),
        (26_224_979, 19_519_372),
        (26_214_016, 19_501_287),
        (26_196_091, 19_478_803),
        (26_171_003, 19_451_719),
        (26_138_722, 19_421_164),
        (26_097_004, 19_388_761),
        (26_048_044, 19_357_974),
        (25_993_934, 19_326_801),
        (25_931_995, 19_297_273),
        (25_864_704, 19_270_379),
        (25_792_510, 19_244_538),
        (25_713_332, 19_222_804),
        (25_627_102, 19_206_227),
        (25_533_617, 19_193_737),
        (25_433_551, 19_186_611),
        (25_329_145, 19_186_266),
        (25_221_747, 19_194_673),
        (25_114_468, 19_208_947),
        (25_006_070, 19_225_488),
        (24_898_960, 19_241_214),
        (24_794_623, 19_258_800),
        (24_693_084, 19_281_382),
        (24_597_282, 19_308_063),
        (24_503_331, 19_339_074),
        (24_411_965, 19_376_905),
        (24_325_493, 19_417_692),
        (24_246_338, 19_460_703),
        (24_173_327, 19_502_206),
        (24_108_587, 19_545_118),
        (24_049_529, 19_586_187),
        (23_994_459, 19_622_275),
        (23_943_484, 19_657_181),
        (23_896_016, 19_693_544),
        (23_853_216, 19_730_321),
        (23_814_373, 19_765_153),
        (23_781_551, 19_796_998),
        (23_755_620, 19_829_079),
        (23_738_572, 19_860_673),
    ];

    /// Generate the recording from a seed.
    ///
    /// The `Env` supplies the motion and nothing else: the stamps are a regular
    /// grid computed arithmetically, because the truth at the horizon has to
    /// land on a recorded report. Report jitter is therefore a real thing this
    /// does not model, and
    /// `halving_the_report_rate_costs_mostly_lag_and_some_overshoot` is the
    /// test that covers a different report rate instead.
    fn record(seed: u64) -> Recording {
        let mut env = SeededEnv::new(seed, 1);
        let mut out = [(0i32, 0i32); SAMPLES];
        let (mut px, mut py) = (START_X_X65536, START_Y_X65536);
        let (mut vx, mut vy) = (0i64, 0i64);
        let (mut ax, mut ay) = (0i64, 0i64);
        for slot in &mut out {
            ax = jerked(ax, env.next_u64());
            ay = jerked(ay, env.next_u64());
            vx = (vx - vx / VELOCITY_DECAY + ax).clamp(-SPEED_LIMIT_X65536, SPEED_LIMIT_X65536);
            vy = (vy - vy / VELOCITY_DECAY + ay).clamp(-SPEED_LIMIT_X65536, SPEED_LIMIT_X65536);
            px += vx;
            py += vy;
            *slot = (
                i32::try_from(px).expect("the walk stays inside a display"),
                i32::try_from(py).expect("the walk stays inside a display"),
            );
        }
        out
    }

    /// One report's worth of jerk, decay and clamp on one axis.
    fn jerked(accel: i64, draw: u64) -> i64 {
        let span = 2 * JERK_X65536 + 1;
        let kick = widened(draw % span) - widened(JERK_X65536);
        (accel - accel / ACCEL_DECAY + kick).clamp(-ACCEL_LIMIT_X65536, ACCEL_LIMIT_X65536)
    }

    /// The stamp of the `index`th report of the recording.
    fn stamp_of(index: usize) -> StampNanos {
        StampNanos::from_wire_nanos(EPOCH_NANOS + index as u64 * SAMPLE_NANOS)
    }

    /// The sample the `index`th report of the recording is.
    fn sample_of(recording: &Recording, index: usize) -> Sample {
        let (x_x65536, y_x65536) = recording[index];
        Sample { at: stamp_of(index), x_x65536, y_x65536 }
    }

    /// The scanout one horizon after the `index`th report.
    fn scanout_after(index: usize) -> StampNanos {
        StampNanos::from_wire_nanos(stamp_of(index).nanos() + HORIZON_NANOS)
    }

    /// Run the recording past a predictor, asking at each report where the
    /// pointer will be one horizon later, and fold each answer against the
    /// position the recording says it actually reached.
    ///
    /// `stride` is how many recorded reports pass per report the predictor
    /// sees: one is the full 500 Hz, two is a device reporting half as often
    /// over the same motion.
    fn measured(recording: &Recording, stride: usize) -> Deviation {
        let mut predictor = Predictor::new();
        let mut worst = Deviation::NONE;
        let mut index = 0;
        while index + HORIZON_SAMPLES < SAMPLES {
            assert!(predictor.observe(sample_of(recording, index)), "the recording is in order");
            if predictor.samples() >= WINDOW_SAMPLES {
                let predicted =
                    predictor.predict_at(scanout_after(index)).expect("a report has been observed");
                let (truth_x, truth_y) = recording[index + HORIZON_SAMPLES];
                worst = worst.worst(Deviation::between(&predicted, truth_x, truth_y));
            }
            index += stride;
        }
        worst
    }

    /// The same fold for the predictor that does not predict: the cursor is
    /// drawn where the device last said it was.
    fn measured_without_predicting(recording: &Recording) -> Deviation {
        let mut worst = Deviation::NONE;
        let mut index = WINDOW_SAMPLES - 1;
        while index + HORIZON_SAMPLES < SAMPLES {
            let held = Predicted::held(sample_of(recording, index), Held::ShortHistory);
            let (truth_x, truth_y) = recording[index + HORIZON_SAMPLES];
            worst = worst.worst(Deviation::between(&held, truth_x, truth_y));
            index += 1;
        }
        worst
    }

    /// How many recordings the swept bounds below are stated over.
    ///
    /// Four thousand and ninety-six of them, seeds [`CORPUS_SEED`] upward, of
    /// which [`CORPUS`] is the first. The count belongs here because it is part
    /// of every sentence the bounds appear in — what they bound is *these*
    /// recordings, and a different count is a different claim. It is large
    /// because a maximum over one draw is not a maximum: the version of this
    /// file that swept a single recording stated an over-prediction bound the
    /// same generator then exceeded on one draw in six. It is finite because
    /// there is no universal bound to reach for instead; see the module's *the
    /// two numbers are a maximum over a named set*. The whole sweep costs about
    /// a sixth of a second unoptimised, and sweeping ten times as many moves
    /// the maxima by a few per cent.
    /// Unit: recordings.
    const SWEPT_CORPORA: u64 = 4_096;

    /// The `index`th recording of the sweep. Index zero is [`CORPUS`], and
    /// `the_recording_is_what_the_seed_produces` is what ties it to the table.
    fn swept(index: u64) -> Recording {
        record(CORPUS_SEED + index)
    }

    /// Every question the sweep is asked, answered in one pass.
    ///
    /// One pass rather than one per question, so that every number below is
    /// about the same recordings: two tests that disagreed would then be
    /// disagreeing about the predictor rather than about which corpora each of
    /// them happened to draw.
    struct Sweep {
        /// The worst deviation anywhere in the sweep, at the recorded rate.
        full: Deviation,
        /// The same at half that rate.
        halved: Deviation,
        /// The same for the predictor that does not predict.
        nothing: Deviation,
        /// Recordings where the predictor's lag is not smaller than what doing
        /// nothing costs on that same recording.
        /// Unit: recordings.
        not_better_than_nothing: u64,
        /// Recordings where the predictor's lag is two thirds of doing
        /// nothing's or less.
        /// Unit: recordings.
        within_two_thirds_of_nothing: u64,
        /// The largest share of doing-nothing's lag the predictor's lag reaches
        /// on any one recording. Past a thousand is a recording on which
        /// predicting was worse than not predicting.
        /// Unit: thousandths.
        worst_lag_share_permille: u64,
        /// Recordings where halving the report rate *raises* the worst
        /// over-prediction rather than only the lag.
        /// Unit: recordings.
        halving_raises_overshoot: u64,
    }

    impl Sweep {
        /// Take it.
        fn taken() -> Self {
            let mut sweep = Self {
                full: Deviation::NONE,
                halved: Deviation::NONE,
                nothing: Deviation::NONE,
                not_better_than_nothing: 0,
                within_two_thirds_of_nothing: 0,
                worst_lag_share_permille: 0,
                halving_raises_overshoot: 0,
            };
            for index in 0..SWEPT_CORPORA {
                let recording = swept(index);
                let full = measured(&recording, 1);
                let halved = measured(&recording, 2);
                let nothing = measured_without_predicting(&recording);
                sweep.full = sweep.full.worst(full);
                sweep.halved = sweep.halved.worst(halved);
                sweep.nothing = sweep.nothing.worst(nothing);
                if halved.ahead_x65536() > full.ahead_x65536() {
                    sweep.halving_raises_overshoot += 1;
                }
                // Every recording this generator draws moves, so doing nothing
                // always lags by something and the share has a denominator.
                let alone = u64::from(nothing.behind_x65536());
                assert!(alone > 0, "a recording that never moved would make the share meaningless");
                let share = u64::from(full.behind_x65536()) * 1_000 / alone;
                if share > sweep.worst_lag_share_permille {
                    sweep.worst_lag_share_permille = share;
                }
                if full.behind_x65536() >= nothing.behind_x65536() {
                    sweep.not_better_than_nothing += 1;
                }
                if share * 3 <= 2_000 {
                    sweep.within_two_thirds_of_nothing += 1;
                }
            }
            sweep
        }
    }

    /// The two bounds, with the argument for each beside it.
    ///
    /// Both are the sweep's own maxima rounded up to a whole pixel, and the
    /// module's *the two numbers are a maximum over a named set* says how far
    /// that licence runs: over [`SWEPT_CORPORA`] recordings of this generator,
    /// at the recorded rate and at half of it, nothing exceeded them. Nothing
    /// claims a motion outside that set obeys them. The four observed maxima
    /// are written out below as their own constants, so that a change to the
    /// arithmetic has to restate them rather than slide under the rounding —
    /// which is exactly what the rounding hid last time.
    ///
    /// How much that disclaimer is worth was measured rather than guessed. The
    /// same generator, over 4096 recordings the sweep does *not* contain, at
    /// both report rates: the eight-pixel bound holds on all 8192 of those
    /// measurements, worst 512 893, and the thirty-two-pixel bound is exceeded
    /// once, at 2 119 748. One in eight thousand, against the one in six that
    /// broke the single-recording bound this pair replaced. The bound is
    /// deliberately *not* widened to swallow that one recording: a number moved
    /// until its counterexample fits is a number that bounds nothing, and the
    /// honest repair for a claim that fails off its set is to say where the set
    /// ends and how often it fails past it.
    ///
    /// What the *pair* carries is the asymmetry, and the asymmetry is not
    /// empirical. It is a fact about eyes, argued at the top of this module,
    /// and it is why [`Bounds::stated`] refuses a pair that collapses it.
    ///
    /// **Over-prediction, eight pixels.** The tighter of the two by a factor of
    /// four, because what it bounds is a cursor arriving where the finger never
    /// went and then leaving again: the user is shown the error twice, and the
    /// second showing is a motion nobody made. It is the number to argue with
    /// first — a compositor that draws a large cursor has a reason to want it
    /// lower, and lowering it means claiming less lead ([`LEAD_NUMERATOR`] over
    /// [`LEAD_DENOMINATOR`] is the knob) and watching the bound below rise.
    ///
    /// **Under-prediction, thirty-two pixels.** Looser, because lag is the
    /// failure the system has anyway and it degrades smoothly. The number to
    /// read it against is not the eight but what *not predicting* costs on the
    /// same recordings, which the sweep measures at up to sixty-four pixels.
    /// `doing_nothing_has_a_perfect_ahead_bound` is where that comparison is
    /// made, and it is made per recording rather than maximum against maximum,
    /// because two maxima are two different recordings.
    ///
    /// Writing them as a `const` is what makes [`Bounds::stated`]'s refusal a
    /// compile error here; `equal_bounds_are_refused` is what makes it a
    /// failing test everywhere else.
    const BOUNDS: Bounds = Bounds::stated(8 * PX, 32 * PX);

    /// The worst over-prediction the committed recording produces.
    /// Unit: 1/65536 of a device pixel.
    const CORPUS_WORST_AHEAD_X65536: u32 = 145_476;

    /// The worst under-prediction the committed recording produces.
    /// Unit: 1/65536 of a device pixel.
    const CORPUS_WORST_BEHIND_X65536: u32 = 544_394;

    /// The worst over-prediction the committed recording produces when the
    /// reports arrive half as often.
    /// Unit: 1/65536 of a device pixel.
    const CORPUS_HALVED_AHEAD_X65536: u32 = 157_576;

    /// And the worst under-prediction there.
    /// Unit: 1/65536 of a device pixel.
    const CORPUS_HALVED_BEHIND_X65536: u32 = 615_857;

    /// The worst over-prediction anywhere in the sweep, at the recorded rate.
    /// Unit: 1/65536 of a device pixel.
    const SWEPT_WORST_AHEAD_X65536: u32 = 471_635;

    /// The worst under-prediction anywhere in the sweep, at that rate.
    /// Unit: 1/65536 of a device pixel.
    const SWEPT_WORST_BEHIND_X65536: u32 = 1_824_513;

    /// The worst over-prediction anywhere in the sweep at half that rate.
    /// Unit: 1/65536 of a device pixel.
    const SWEPT_HALVED_AHEAD_X65536: u32 = 517_792;

    /// The worst under-prediction anywhere in the sweep at half that rate.
    /// Unit: 1/65536 of a device pixel.
    const SWEPT_HALVED_BEHIND_X65536: u32 = 2_057_997;

    /// The worst under-prediction not predicting at all costs anywhere in the
    /// sweep: sixty-four pixels, which is the generator's own speed clamp over
    /// a horizon, on both axes at once.
    /// Unit: 1/65536 of a device pixel.
    const SWEPT_NOTHING_BEHIND_X65536: u32 = 4_194_304;

    /// Recordings in the sweep on which the predictor lags at least as much as
    /// doing nothing would have. Six of four thousand and ninety-six, and the
    /// number is here rather than absent because *the prediction always earns
    /// its over-prediction budget* is the sentence this file would otherwise be
    /// read as making.
    /// Unit: recordings.
    const SWEPT_NOT_BETTER_THAN_NOTHING: u64 = 6;

    /// Recordings on which it cuts the lag to two thirds of doing nothing's or
    /// better, which is the trade the over-prediction budget is spent for.
    /// Unit: recordings.
    const SWEPT_WITHIN_TWO_THIRDS: u64 = 3_961;

    /// The worst share of doing-nothing's lag the predictor reaches on any one
    /// recording. Over a thousand, so on its worst recording the prediction is
    /// half again as far behind as not predicting would have been.
    /// Unit: thousandths.
    const SWEPT_WORST_LAG_SHARE_PERMILLE: u64 = 1_584;

    /// Recordings on which halving the report rate raises the worst
    /// over-prediction. A third of them, which is the number that refutes the
    /// name this file's degradation test used to carry.
    /// Unit: recordings.
    const SWEPT_HALVING_RAISES_OVERSHOOT: u64 = 1_484;

    /// How much looser the one universal over-prediction bound is than the
    /// worst the sweep measured: eighty times. That ratio is the argument for
    /// stating maxima over a named set instead of the theorem.
    /// Unit: none — a ratio of two distances.
    const UNIVERSAL_BOUND_OVER_SWEPT_WORST: i64 = 80;

    #[test]
    fn the_recording_is_what_the_seed_produces() {
        // Literal numbers against the generator, not the generator against
        // itself. The arm runner computing a different recording here means
        // `f_env::SeededEnv` disagrees across architectures, which is a bug
        // below this crate, and this is where it surfaces.
        assert_eq!(record(CORPUS_SEED), CORPUS);
    }

    #[test]
    fn a_seed_reproduces_its_recording_and_a_different_seed_does_not() {
        assert_eq!(record(CORPUS_SEED), record(CORPUS_SEED), "a seed must reproduce its motion");
        assert_ne!(record(CORPUS_SEED), record(CORPUS_SEED + 1), "seeds must diverge");
    }

    #[test]
    fn over_the_committed_recording_the_two_errors_are_bounded_separately() {
        // The committed table, which is the only measurement in this file that
        // is the same integers on both architectures without running the
        // generator. The exact numbers first, so a change to the arithmetic has
        // to restate them rather than slide under a bound.
        let worst = measured(&CORPUS, 1);
        assert_eq!(
            (worst.ahead_x65536(), worst.behind_x65536()),
            (CORPUS_WORST_AHEAD_X65536, CORPUS_WORST_BEHIND_X65536),
            "the recording's worst deviation moved"
        );
        assert!(BOUNDS.admits(worst), "{worst:?} is outside {BOUNDS:?}");
        // What this test does not say, said out loud: one recording's maximum
        // is not a bound, and for a whole round this file's prose said it was.
        // `over_the_swept_recordings_the_two_errors_are_bounded_separately` is
        // the test that carries the word *bounded*.
    }

    #[test]
    fn over_the_swept_recordings_the_two_errors_are_bounded_separately() {
        let sweep = Sweep::taken();
        let full = sweep.full;
        assert_eq!(
            (full.ahead_x65536(), full.behind_x65536()),
            (SWEPT_WORST_AHEAD_X65536, SWEPT_WORST_BEHIND_X65536),
            "the sweep's worst deviation moved"
        );
        assert!(BOUNDS.admits(full), "{full:?} is outside {BOUNDS:?}");
        // And the separation is load-bearing rather than decorative. The worst
        // over-prediction is less than a third of the worst under-prediction,
        // so a single symmetric bound set where this pair's lag bound sits
        // would admit an overshoot several times anything these recordings
        // produced — which is the whole reason there are two numbers.
        assert!(
            u64::from(full.ahead_x65536()) * 3 < u64::from(full.behind_x65536()),
            "two bounds are ceremony unless the two maxima differ: {full:?}"
        );
    }

    #[test]
    fn halving_the_report_rate_costs_mostly_lag_and_some_overshoot() {
        // The one degradation that does not trip a guard: the same motion
        // reported half as often. This test used to be named *costs lag and not
        // overshoot*, and that was false — halving raises the worst
        // over-prediction on the committed recording and on a third of the
        // sweep. It passed because it checked the over-prediction against a
        // bound with twenty per cent of slack rather than against the number
        // its own name claimed it beat. What is true is a ratio, so the ratio
        // is what is asserted and both of its ends are pinned.
        let full = measured(&CORPUS, 1);
        let halved = measured(&CORPUS, 2);
        assert_eq!(
            (halved.ahead_x65536(), halved.behind_x65536()),
            (CORPUS_HALVED_AHEAD_X65536, CORPUS_HALVED_BEHIND_X65536),
            "the halved recording's worst deviation moved"
        );
        let overshoot_cost = halved.ahead_x65536() - full.ahead_x65536();
        let lag_cost = halved.behind_x65536() - full.behind_x65536();
        assert_eq!((overshoot_cost, lag_cost), (12_100, 71_463), "the split of the cost moved");
        assert!(
            lag_cost > overshoot_cost * 5,
            "mostly lag is the claim, and about six to one is where it stands: \
             {lag_cost} of lag against {overshoot_cost} of overshoot"
        );
        // Over the sweep, the part of the old claim that survived: the stated
        // bounds still hold at half the rate. And the part that did not,
        // counted. A predictor that genuinely spent every degradation on lag
        // would drive the count to zero and this assertion red, which is an
        // outcome to hope for rather than a sentence to write before it is
        // true.
        let sweep = Sweep::taken();
        assert_eq!(
            (sweep.halved.ahead_x65536(), sweep.halved.behind_x65536()),
            (SWEPT_HALVED_AHEAD_X65536, SWEPT_HALVED_BEHIND_X65536),
            "the sweep's worst halved deviation moved"
        );
        assert!(BOUNDS.admits(sweep.halved), "{:?} is outside {BOUNDS:?}", sweep.halved);
        assert_eq!(
            sweep.halving_raises_overshoot, SWEPT_HALVING_RAISES_OVERSHOOT,
            "how often half the reports buy overshoot moved"
        );
    }

    #[test]
    fn doing_nothing_has_a_perfect_ahead_bound() {
        // The argument for two bounds rather than one, executed over the sweep.
        // Holding the last measured position never over-predicts — it cannot,
        // it never puts the cursor anywhere the device did not report — so it
        // passes any over-prediction bound at all, including zero. Judged by a
        // single symmetric error it therefore beats every predictor that tries,
        // which is why a single symmetric error is the wrong instrument.
        let sweep = Sweep::taken();
        let nothing = sweep.nothing;
        assert_eq!(nothing.ahead_x65536(), 0, "holding the last position cannot over-predict");
        assert_eq!(
            nothing.behind_x65536(),
            SWEPT_NOTHING_BEHIND_X65536,
            "what not predicting costs moved"
        );
        assert!(
            !BOUNDS.admits(nothing),
            "the lag bound must be one that doing nothing fails, or it bounds nothing: {nothing:?}"
        );
        // And what the prediction buys with its over-prediction budget, counted
        // per recording. Per recording rather than maximum against maximum,
        // because two maxima are two different recordings and a predictor that
        // was worse on every one of them could still win that comparison. The
        // earlier version of this test made exactly that comparison, on a
        // single recording, and concluded that the lag bound was at most two
        // thirds of what doing nothing costs. Over the sweep that is true of
        // 3961 recordings, false of the rest, and on six of them the prediction
        // is the worse of the two.
        assert_eq!(
            (sweep.within_two_thirds_of_nothing, sweep.not_better_than_nothing),
            (SWEPT_WITHIN_TWO_THIRDS, SWEPT_NOT_BETTER_THAN_NOTHING),
            "what the prediction buys per recording moved"
        );
        assert_eq!(
            sweep.worst_lag_share_permille, SWEPT_WORST_LAG_SHARE_PERMILLE,
            "the worst recording for the predictor moved"
        );
    }

    #[test]
    #[should_panic(expected = "strictly tighter")]
    fn equal_bounds_are_refused() {
        // The refusal the module's asymmetry rests on, driven to red. For a
        // round it was an `assert!` nothing executed: a reviewer replaced its
        // condition with `true` and then set this file's two bounds equal, and
        // all twenty-five tests stayed green. Neither edit can pass this one.
        let _ = Bounds::stated(9 * PX, 9 * PX);
    }

    #[test]
    #[should_panic(expected = "strictly tighter")]
    fn a_looser_over_prediction_bound_is_refused() {
        // The other side of it. A pair that bounds the snap-back more loosely
        // than the lag is the asymmetry inverted, which is a different claim
        // about eyes and belongs in an RFC rather than in a constructor call.
        let _ = Bounds::stated(9 * PX, 3 * PX);
    }

    #[test]
    fn the_only_universal_over_prediction_bound_is_the_two_ceilings_multiplied() {
        // The one over-prediction bound in this file that is a theorem rather
        // than a maximum, kept beside the empirical pair so the difference
        // between them stays visible. A prediction is *ahead* only by part of
        // the step it took; the step is the window's travel scaled by the used
        // lead over the baseline, which is at most the speed ceiling times the
        // used lead; and the used lead is at most the lead ceiling, damped.
        // Multiply the three constants: 576 px on one axis, for any motion
        // whatever, which is what the word *bounded* means when it is earned
        // rather than measured.
        let damped_nanos = LEAD_CEILING_NANOS * LEAD_NUMERATOR / LEAD_DENOMINATOR;
        let damped = i64::try_from(damped_nanos).expect("a lead is nanoseconds and small");
        let ceiling_x65536 = SPEED_CEILING_X65536_PER_MS * damped / NANOS_PER_MS;
        assert_eq!(ceiling_x65536, 576 * i64::from(PX), "the derived universal bound moved");
        // And why it is not the stated bound: it is eighty times the worst the
        // sweep measured, which makes it true and useless. A ratio that fell
        // toward one would mean the ceilings had become the binding constraint
        // and the stated pair should be replaced by this arithmetic.
        assert_eq!(
            ceiling_x65536 / i64::from(SWEPT_WORST_AHEAD_X65536),
            UNIVERSAL_BOUND_OVER_SWEPT_WORST,
            "the distance between the theorem and the measurement moved"
        );
    }

    #[test]
    fn a_held_prediction_is_the_last_measured_position_and_nothing_else() {
        // The invariant is structural — `Predicted::held` is the only door a
        // `Held` reason can come through, and it does not take a position — so
        // this checks the consequence across the whole recording rather than
        // enumerating the reasons. Enumerating them is exactly the test a
        // seventh reason walks past.
        let mut predictor = Predictor::new();
        for index in 0..SAMPLES {
            let sample = sample_of(&CORPUS, index);
            assert!(predictor.observe(sample), "the recording is in order");
            let predicted =
                predictor.predict_at(scanout_after(index)).expect("a report has been observed");
            if !predicted.basis().extrapolated() {
                assert_eq!(predicted.x_x65536(), sample.x_x65536);
                assert_eq!(predicted.y_x65536(), sample.y_x65536);
                assert_eq!(predicted.lead_nanos(), 0);
            }
        }
    }

    #[test]
    fn an_empty_predictor_has_nothing_to_say() {
        // `None` rather than the origin. A cursor at (0, 0) before the device
        // has reported anything is a position somebody invented.
        let predictor = Predictor::new();
        assert_eq!(predictor.predict_at(stamp_of(0)), None);
        assert_eq!(predictor.samples(), 0);
    }

    #[test]
    fn one_report_is_not_a_velocity() {
        let mut predictor = Predictor::new();
        assert!(predictor.observe(sample_of(&CORPUS, 0)));
        let predicted =
            predictor.predict_at(scanout_after(0)).expect("one report is still a position");
        assert_eq!(predicted.basis(), Basis::Held(Held::ShortHistory));
    }

    #[test]
    fn a_reordered_report_does_not_become_the_anchor() {
        // The predictor's own snap-back, which is worse than a bad guess
        // because the user did nothing to cause it.
        let mut predictor = Predictor::new();
        for index in 0..WINDOW_SAMPLES {
            assert!(predictor.observe(sample_of(&CORPUS, index)));
        }
        let newest = predictor.newest().expect("four reports are in");
        assert!(!predictor.observe(sample_of(&CORPUS, 1)), "a stale report must be dropped");
        assert!(
            !predictor.observe(sample_of(&CORPUS, WINDOW_SAMPLES - 1)),
            "and so must a duplicate"
        );
        assert_eq!(predictor.newest(), Some(newest));
    }

    #[test]
    fn a_scanout_that_is_not_ahead_gets_the_position_it_asked_about() {
        let mut predictor = Predictor::new();
        for index in 0..WINDOW_SAMPLES {
            assert!(predictor.observe(sample_of(&CORPUS, index)));
        }
        let newest = predictor.newest().expect("four reports are in");
        let predicted = predictor.predict_at(newest.at).expect("a report has been observed");
        assert_eq!(predicted.basis(), Basis::Held(Held::NoLead));
        // And a scanout in the past, which a late frame loop genuinely asks
        // about, saturates to the same answer rather than extrapolating
        // backwards.
        let past = StampNanos::from_wire_nanos(newest.at.nanos() - SAMPLE_NANOS);
        let backwards = predictor.predict_at(past).expect("a report has been observed");
        assert_eq!(backwards.basis(), Basis::Held(Held::NoLead));
        assert_eq!(backwards.x_x65536(), newest.x_x65536);
    }

    #[test]
    fn a_scanout_past_the_ceiling_is_held_rather_than_extrapolated() {
        let mut predictor = Predictor::new();
        for index in 0..WINDOW_SAMPLES {
            assert!(predictor.observe(sample_of(&CORPUS, index)));
        }
        let newest = predictor.newest().expect("four reports are in");
        // Literal milliseconds on both sides of the ceiling rather than
        // `LEAD_CEILING_NANOS + 1`, which tracked the constant instead of
        // checking it: written that way the test stayed green with the ceiling
        // raised a hundredfold, so it observed the comparison and not the
        // number. Thirty-three milliseconds must be held and thirty-one must
        // not, which pins the constant between them the way the fifteen
        // millisecond spacing pins `BASELINE_CEILING_NANOS` below.
        let far = StampNanos::from_wire_nanos(newest.at.nanos() + 33_000_000);
        let predicted = predictor.predict_at(far).expect("a report has been observed");
        assert_eq!(predicted.basis(), Basis::Held(Held::FarScanout));
        assert_eq!(predicted.x_x65536(), newest.x_x65536);
        let near = StampNanos::from_wire_nanos(newest.at.nanos() + 31_000_000);
        let inside = predictor.predict_at(near).expect("a report has been observed");
        assert_eq!(
            inside.basis(),
            Basis::Extrapolated,
            "a scanout inside the ceiling is predicted"
        );
    }

    #[test]
    fn a_window_that_spans_too_long_is_held_rather_than_extrapolated() {
        // A device reporting every 15 ms: four of those span 45 ms, past the
        // ceiling, and what the ends of that window agree about is a gesture
        // that has finished.
        let mut predictor = Predictor::new();
        let mut at = EPOCH_NANOS;
        for &(x_x65536, y_x65536) in CORPUS.iter().take(WINDOW_SAMPLES) {
            let sample = Sample { at: StampNanos::from_wire_nanos(at), x_x65536, y_x65536 };
            assert!(predictor.observe(sample));
            at += 15_000_000;
        }
        let scanout = StampNanos::from_wire_nanos(at);
        let predicted = predictor.predict_at(scanout).expect("a report has been observed");
        assert_eq!(predicted.basis(), Basis::Held(Held::StaleWindow));
    }

    #[test]
    fn a_teleport_is_not_a_velocity() {
        // A contact identifier reused for a second finger, or a warped pointer:
        // one report says the pointer crossed the display between two
        // interrupts. Extrapolating that flings the cursor off the edge for a
        // frame.
        let mut predictor = Predictor::new();
        for index in 0..WINDOW_SAMPLES - 1 {
            assert!(predictor.observe(sample_of(&CORPUS, index)));
        }
        let anchor = sample_of(&CORPUS, WINDOW_SAMPLES - 1);
        let warped = Sample { x_x65536: anchor.x_x65536 + 4_000 * 65_536, ..anchor };
        assert!(predictor.observe(warped));
        let predicted = predictor
            .predict_at(scanout_after(WINDOW_SAMPLES - 1))
            .expect("a report has been observed");
        assert_eq!(predicted.basis(), Basis::Held(Held::ImplausibleSpeed));
    }

    #[test]
    fn the_damped_lead_is_shorter_than_the_lead_it_was_asked_about() {
        // The knob, observable from outside, on the one motion where the
        // confidence factor is exactly full: a steady speed, where the two
        // halves of the window imply the same rate. The prediction then covers
        // three quarters of the time to the scanout and three quarters of the
        // distance, and a consumer is entitled to see that rather than infer it.
        let mut predictor = Predictor::new();
        let step = 2 * 65_536;
        for index in 0..WINDOW_SAMPLES {
            let index_i32 = i32::try_from(index).expect("the window is small");
            assert!(predictor.observe(Sample {
                at: stamp_of(index),
                x_x65536: index_i32 * step,
                y_x65536: 0,
            }));
        }
        let newest = predictor.newest().expect("the window is full");
        let predicted = predictor
            .predict_at(scanout_after(WINDOW_SAMPLES - 1))
            .expect("a report has been observed");
        assert_eq!(predicted.basis(), Basis::Extrapolated);
        assert_eq!(predicted.lead_nanos(), HORIZON_NANOS * LEAD_NUMERATOR / LEAD_DENOMINATOR);
        assert!(predicted.lead_nanos() < HORIZON_NANOS);
        // Eight reports of horizon at two pixels a report is sixteen pixels of
        // travel; three quarters of it is twelve, and the arithmetic is exact
        // because nothing here needs rounding.
        let travelled = predicted.x_x65536() - newest.x_x65536;
        assert_eq!(travelled, 12 * 65_536);
    }

    #[test]
    fn a_motion_that_turns_inside_the_window_extrapolates_nothing() {
        // The moment a predictor is most tempted: the pointer went one way and
        // is now going the other, and the velocity across the whole window is a
        // number that describes neither. Carrying it forward is the snap-back
        // the over-prediction bound exists to bound, so it is refused by name.
        let mut predictor = Predictor::new();
        let out_and_back = [0i32, 2, 4, 1];
        for (index, step) in out_and_back.iter().enumerate() {
            assert!(predictor.observe(Sample {
                at: stamp_of(index),
                x_x65536: step * 65_536,
                y_x65536: 0,
            }));
        }
        let predicted = predictor
            .predict_at(scanout_after(WINDOW_SAMPLES - 1))
            .expect("a report has been observed");
        assert_eq!(predicted.basis(), Basis::Held(Held::Reversed));
    }

    #[test]
    fn slowing_down_gives_distance_back_and_speeding_up_does_not_take_more() {
        // The clamp, from both sides. A window whose newer half is half the
        // speed of its older half is extrapolated half as far; a window whose
        // newer half is twice the speed is extrapolated exactly as far as a
        // steady one, because the factor cannot exceed full confidence. That
        // asymmetry is the whole of *a second difference that may only
        // subtract*, and a clamp removed would show up here rather than as a
        // user watching a cursor overrun a flick.
        let carried = |steps: [i32; WINDOW_SAMPLES]| {
            let mut predictor = Predictor::new();
            for (index, step) in steps.iter().enumerate() {
                assert!(predictor.observe(Sample {
                    at: stamp_of(index),
                    x_x65536: step * 65_536,
                    y_x65536: 0,
                }));
            }
            let newest = predictor.newest().expect("the window is full");
            let predicted = predictor
                .predict_at(scanout_after(WINDOW_SAMPLES - 1))
                .expect("a report has been observed");
            predicted.x_x65536() - newest.x_x65536
        };
        // Six pixels across the window, six intervals of damped lead over three
        // of baseline: twelve pixels carried, and the halves agree, so nothing
        // is given back.
        assert_eq!(carried([0, 2, 4, 6]), 12 * 65_536);
        // Seven pixels across the window and the newer half twice the rate of
        // the older: fourteen pixels carried, which is the same full-confidence
        // answer a steady window gets, because the clamp refuses to turn
        // *faster than before* into *further than the window says*. The
        // unclamped answer would be twenty-eight — the ratio is 2048 in
        // 1024ths, and it is the clamp that throws the second factor of two
        // away. Calling fourteen *the unclamped answer*, which this comment did
        // until a reviewer checked it, understated the clamp by exactly the
        // thing the clamp does.
        assert_eq!(carried([0, 1, 3, 7]), 14 * 65_536);
        // The same seven pixels across the window, but the newer half is half
        // the rate of the older: the lead is halved with it, and seven pixels
        // are carried instead of fourteen. This is the direction the clamp does
        // not block, and it is the one that buys the over-prediction bound.
        assert_eq!(carried([0, 4, 6, 7]), 7 * 65_536);
    }

    #[test]
    fn a_deviation_keeps_the_two_directions_apart() {
        let anchor = Sample { at: stamp_of(0), x_x65536: 0, y_x65536: 0 };
        let held = Predicted::held(anchor, Held::NoLead);
        // The pointer went to +10 px and the cursor stayed at the origin: lag.
        let behind = Deviation::between(&held, 10 * 65_536, 0);
        assert_eq!((behind.ahead_x65536(), behind.behind_x65536()), (0, 10 * PX));
        // A cursor put at +10 px on an axis the pointer never left: overshoot,
        // which is the case a signed error measured against travel would have
        // had to divide by zero to classify.
        let ran_on = predicted_at(10 * 65_536, 0);
        let ahead = Deviation::between(&ran_on, 0, 0);
        assert_eq!((ahead.ahead_x65536(), ahead.behind_x65536()), (10 * PX, 0));
        // Folding the two keeps both, which is the whole reason `worst` is not
        // one comparison and a winner.
        let folded = behind.worst(ahead);
        assert_eq!((folded.ahead_x65536(), folded.behind_x65536()), (10 * PX, 10 * PX));
        // And one comparison can be both at once: the cursor is past the finger
        // along x and short of it along y, which is a real thing a diagonal
        // movement does and which the two fields hold without argument.
        let both = Deviation::between(&predicted_at(10 * 65_536, 2 * 65_536), 0, 5 * 65_536);
        assert_eq!((both.ahead_x65536(), both.behind_x65536()), (10 * PX, 3 * PX));
        // The two axes are **added**, not maximised, and these two cases are
        // where the difference shows. Every case above is wrong on one axis
        // only, so a version of `between` that took the larger of the two would
        // pass all of them — and one did, until this pair was written: the
        // corpus's own worst moment happens to be single-axis too, so the
        // recording could not tell the two apart either. The module's *why per
        // axis, and why the two are added* is the argument; this is the
        // observation of it, in both directions.
        let diagonal_ahead = Deviation::between(&predicted_at(10 * 65_536, 4 * 65_536), 0, 0);
        assert_eq!((diagonal_ahead.ahead_x65536(), diagonal_ahead.behind_x65536()), (14 * PX, 0));
        let diagonal_behind = Deviation::between(&held, 10 * 65_536, 5 * 65_536);
        assert_eq!((diagonal_behind.ahead_x65536(), diagonal_behind.behind_x65536()), (0, 15 * PX));
    }

    /// A prediction at a position, anchored at the origin. For the deviation
    /// tests, which are about the measurement rather than about the
    /// extrapolation that produced it.
    fn predicted_at(x_x65536: i32, y_x65536: i32) -> Predicted {
        Predicted {
            x_x65536,
            y_x65536,
            anchor_x_x65536: 0,
            anchor_y_x65536: 0,
            lead_nanos: 1,
            basis: Basis::Extrapolated,
        }
    }

    #[test]
    fn the_recording_is_the_table_this_file_prints() {
        // The regeneration instruction, executable. `cargo test -p f-input
        // the_recording_is_the_table -- --nocapture` prints the `CORPUS` body
        // and the two worst-case constants beside it, in the form they are
        // pasted in, so that re-deriving them after a change to the generator
        // is a copy rather than a transcription. It runs the whole measurement
        // over a freshly generated recording rather than over `CORPUS`, which
        // is the only way the printed numbers can be the ones the change
        // actually produces. It asserts nothing: the assertions are
        // `the_recording_is_what_the_seed_produces` and the two bound tests,
        // and a printer that also judged would be the generator grading its own
        // homework.
        let fresh = record(CORPUS_SEED);
        for (x, y) in fresh {
            println!("({x}, {y}),");
        }
        let worst = measured(&fresh, 1);
        let halved = measured(&fresh, 2);
        println!("CORPUS_WORST_AHEAD_X65536 = {}", worst.ahead_x65536());
        println!("CORPUS_WORST_BEHIND_X65536 = {}", worst.behind_x65536());
        println!("CORPUS_HALVED_AHEAD_X65536 = {}", halved.ahead_x65536());
        println!("CORPUS_HALVED_BEHIND_X65536 = {}", halved.behind_x65536());
        println!(
            "halved costs = {:?}",
            (
                halved.ahead_x65536() - worst.ahead_x65536(),
                halved.behind_x65536() - worst.behind_x65536(),
            )
        );
        // And the sweep, which is the set the bounds are actually stated over,
        // so that re-deriving them after a change to the generator or to
        // `SWEPT_CORPORA` is the same copy rather than a second procedure.
        let sweep = Sweep::taken();
        println!("SWEPT_WORST_AHEAD_X65536 = {}", sweep.full.ahead_x65536());
        println!("SWEPT_WORST_BEHIND_X65536 = {}", sweep.full.behind_x65536());
        println!("SWEPT_HALVED_AHEAD_X65536 = {}", sweep.halved.ahead_x65536());
        println!("SWEPT_HALVED_BEHIND_X65536 = {}", sweep.halved.behind_x65536());
        println!("SWEPT_NOTHING_BEHIND_X65536 = {}", sweep.nothing.behind_x65536());
        println!("SWEPT_NOT_BETTER_THAN_NOTHING = {}", sweep.not_better_than_nothing);
        println!("SWEPT_WITHIN_TWO_THIRDS = {}", sweep.within_two_thirds_of_nothing);
        println!("SWEPT_WORST_LAG_SHARE_PERMILLE = {}", sweep.worst_lag_share_permille);
        println!("SWEPT_HALVING_RAISES_OVERSHOOT = {}", sweep.halving_raises_overshoot);
    }
}
