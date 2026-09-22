// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Frame pacing: when the application should be woken, computed backwards from
//! the next scanout out of this compositor's own rolling p99.
//!
//! # This is a determinism property and not a measurement
//!
//! Nothing here reads a clock. Every function in this module is a pure function
//! of its arguments, and the one argument that is a time — [`Tick`] — is handed
//! in by whoever holds a clock, which for a component at ring 3 is never the
//! component: RFC 0004 puts every observation of time behind `f_env::Env`, a
//! component has no `Env` and no port to read one through, and
//! `crate::tree::Held::offer`'s own comment already says the zero it leaves in
//! `Cqe::timestamp` is a statement rather than an omission.
//!
//! So the frame reads the clock and writes it into the routing page —
//! [`crate::routing::at::TICK_NANOS`] — and this module turns a sequence of
//! readings into a wake time. That is what makes the exit's second clause
//! testable at all: *two runs from one seed compute the same wake time to the
//! tick on both architectures* is a claim about arithmetic over integers, and
//! the tests below drive it from an `f_env::SeededEnv` on the host, which
//! `cargo xtask test` runs on x86-64 and on AArch64. The boot is the third
//! observation and not the first — `kernel/src/compositor.rs` feeds the same
//! arithmetic from the frame's own seeded environment, so the numbers it prints
//! are a function of a seed rather than of how fast the host it ran on was.
//!
//! **No binary fraction appears anywhere below, and it is not a matter of
//! taste.** A percentile is the arithmetic somebody reaches for a floating-point
//! type to hold, the two architectures do not agree on binary fractions, and a
//! wake time that differed in its last nanosecond between the two jobs would
//! fail the exit's own clause while looking like a rounding problem. Every
//! quantity here is a `u64` of nanoseconds with its scale in its name.
//!
//! # How a p99 is taken over integers, and what it costs
//!
//! **Exactly, by nearest rank, over a fixed window of the last [`WINDOW`]
//! samples.** The rank is `ceil(99 * n / 100)` counting from the smallest, which
//! is [`rank`] and is integer arithmetic with no division of a fraction
//! anywhere.
//!
//! The obvious implementations were both refused and the reasons are worth
//! having. A streaming estimator — P², the usual answer — is floating point by
//! construction and is out on RFC 0004 alone. A histogram with integer buckets
//! is deterministic but is *approximate*, and its error depends on where the
//! bucket edges fall relative to the samples, which means the number this
//! publishes would depend on a table rather than on the frames; a compositor
//! that paced itself against a bucket edge is a compositor whose pacing moves
//! when somebody re-tunes a table nobody is looking at.
//!
//! What exactness costs is stated rather than hidden: [`WINDOW`] samples of
//! memory — one kibibyte at 128 — and one pass over the window per query. It is
//! one pass and not a sort, because of an arithmetic accident this module leans
//! on and checks: **at a window of 128, the 99th percentile is never deeper than
//! the second-largest sample**, whatever the window holds. [`depth`] is that
//! number and the `const` block below walks every reachable window size to prove
//! it, so raising [`WINDOW`] past the point where the claim stops holding is a
//! build failure rather than a wrong percentile. *What would reverse this:* a
//! window large enough that the p99 sits deeper than two — 200 samples is the
//! first — at which point this becomes a selection and the paragraph above stops
//! being an argument against a sort.
//!
//! # The window is zero-initialisable, which RFC 0100 requires of it
//!
//! [`Pacing::ZERO`] is all zeroes, so the constant a component starts from is a
//! `memset` over its heap rather than a kibibyte of `.rodata` copied out at run
//! time. That is the rule RFC 0100 states and the reason `scene/src/kind.rs`'s
//! discriminants are numbered from one; a structure this component holds that
//! could not be written as zeroes would be paid for twice, once in the image and
//! once in the heap, and `cargo xtask component`'s image bound is the only thing
//! in this tree that can see it.

use f_scene::degrade::{self, Criterion, Downgrades, Overrun};

/// One reading of the frame's clock, as the component received it.
///
/// A newtype and not a bare `u64`, because the three numbers in this module's
/// central sentence are all nanoseconds — the reading, the estimate and the
/// margin — and the one that is an *instant* is the one it would be worst to
/// pass in the wrong position. A duration subtracted from an instant is an
/// instant; two instants are never added; and the type is what says so at the
/// call site rather than a comment beside it.
/// Unit: nanoseconds, monotonic, in the frame's epoch — the clock `f_env::Env`
/// reports, and the same epoch `f_abi::scene::Frame::deadline` is stated in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tick(pub u64);

impl Tick {
    /// Nanoseconds since the frame's origin.
    #[must_use]
    pub const fn nanos(self) -> u64 {
        self.0
    }

    /// How long from `earlier` to here, saturating at zero.
    ///
    /// Saturating rather than checked, for `f_env::Instant::saturating_since`'s
    /// reason restated one layer up: a reading that appears to move backwards is
    /// a frame that filled the page in out of order or a peer that is wrong, and
    /// neither is a reason for a compositor to stop. What it costs is that a
    /// backwards clock reads as a frame that took no time, which is a low
    /// estimate rather than a panic — and a low estimate wakes the application
    /// late, which is the failure this whole module is about and is therefore
    /// the one a reader will look for.
    /// Unit: nanoseconds.
    #[must_use]
    pub const fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// How many frames the estimate is taken over.
///
/// A hundred and twenty-eight, and three separate things want it to be this
/// number rather than a round one. It is two seconds of a sixty-hertz display,
/// which is long enough that one slow frame does not move the estimate and short
/// enough that a compositor that has genuinely become slow says so within a
/// couple of seconds. It is a kibibyte, which is a heap cost a manifest can
/// carry without a conversation. And it is below the 200 at which [`depth`]
/// reaches three — the `const` block below is what holds that, and the module
/// comment says what reversing it costs.
/// Unit: samples.
pub const WINDOW: usize = 128;

/// Which percentile the estimate is.
///
/// Ninety-nine, which is `E3-B01h`'s own sentence and not a knob: the exit says
/// *the compositor's own rolling p99*, so a build that published a p95 under
/// this name would be publishing a different number with the right label.
/// Unit: percent.
pub const PERCENTILE: usize = 99;

/// The rank of the estimate among `held` samples, counting from the smallest.
///
/// Nearest rank — `ceil(P * n / 100)`, one-based — which is the definition that
/// needs no interpolation and therefore no fraction. `div_ceil` rather than
/// `(P * n + 99) / 100`: the two are the same map, clippy refuses the second on
/// sight, and the rounding is the part a reader has to check — a rank that
/// rounded *down* would answer with a lower sample than the percentile names,
/// which is an estimate that is too small, which wakes the application late.
///
/// Zero for a window that holds nothing, which is the one input for which there
/// is no rank and [`Pacing::estimate_nanos`] answers before asking.
/// Unit: none — a one-based position, not a quantity.
#[must_use]
pub const fn rank(held: usize) -> usize {
    if held == 0 { 0 } else { (PERCENTILE * held).div_ceil(100) }
}

/// How far the estimate is from the largest sample, counting the largest as one.
///
/// This is the number the one-pass implementation rests on: an estimate at depth
/// one is the largest sample and an estimate at depth two is the second largest,
/// and neither needs the window ordered. The `const` block below requires it to
/// stay at or below two for every window this build can reach.
/// Unit: none — a one-based position from the top.
#[must_use]
pub const fn depth(held: usize) -> usize {
    if held == 0 { 0 } else { held - rank(held) + 1 }
}

/// The two-register scan is correct for every window this build can hold.
///
/// A `const` block and not a test, for `scene/src/degrade.rs`'s reason about its
/// own terminator: a loop over the reachable window sizes is exactly what a test
/// would do, and a `const` block does it without producing an artefact first. A
/// [`WINDOW`] raised past the point where the claim fails stops this build with
/// the sentence below rather than silently publishing the wrong sample as a
/// percentile.
const _: () = {
    let mut held = 1;
    while held <= WINDOW {
        assert!(
            depth(held) <= 2,
            "the p99 of this window is deeper than the second-largest sample, so the one-pass \
             estimator in Pacing::estimate_nanos is answering with the wrong sample",
        );
        held += 1;
    }
};

/// The last [`WINDOW`] frame costs, and nothing else.
///
/// # Why a ring of samples rather than a running number
///
/// Because *rolling* is the word in the exit and a running number is not
/// rolling: an average, an exponentially weighted estimate and a high-water mark
/// all have the property that a frame from a minute ago is still in the answer.
/// What a compositor needs to know is what its recent frames cost, so that a
/// machine that has become slow paces itself differently from one that had a bad
/// second, and the only structure that forgets on a schedule is one that holds
/// the samples.
///
/// Zero-initialisable, which RFC 0100 requires of anything a component holds:
/// every field of [`Pacing::ZERO`] is a zero, so the constant costs a `memset`
/// and not a kibibyte of image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pacing {
    /// The samples, oldest overwritten first. Slots past `held` are zero and are
    /// never read. Unit: nanoseconds.
    costs_nanos: [u64; WINDOW],
    /// How many of them are real. Stops at [`WINDOW`]. Unit: samples.
    held: usize,
    /// Where the next sample goes. Unit: none — an index into `costs_nanos`.
    next: usize,
}

impl Pacing {
    /// A compositor that has not closed a frame.
    ///
    /// A `const` of zeroes rather than a `Default`, and the difference is the
    /// image: RFC 0100's rule is about what the compiler can emit as a zero
    /// initialiser, and a constant is the thing it emits.
    pub const ZERO: Self = Self { costs_nanos: [0; WINDOW], held: 0, next: 0 };

    /// Record what one frame cost.
    ///
    /// Every sample, including a zero. A frame that took no measurable time is a
    /// frame that took no measurable time, and dropping it would make the window
    /// a record of the slow frames rather than of the frames — which is the
    /// biased estimator this whole module exists to avoid.
    /// Unit of `cost_nanos`: nanoseconds.
    pub const fn observe(&mut self, cost_nanos: u64) {
        self.costs_nanos[self.next] = cost_nanos;
        self.next += 1;
        if self.next == WINDOW {
            self.next = 0;
        }
        if self.held < WINDOW {
            self.held += 1;
        }
    }

    /// How many frames the estimate is over. Unit: samples.
    #[must_use]
    pub const fn held(&self) -> usize {
        self.held
    }

    /// The rolling p99 of what a frame has cost.
    ///
    /// Zero for a compositor that has closed no frame, and the zero is honest
    /// rather than a sentinel: a component with no samples has no evidence that
    /// a frame costs anything, so the wake time it computes is the scanout less
    /// the margin — the latest wake that is still safe under an estimate of
    /// nothing, which is exactly what *no evidence* should buy.
    ///
    /// One pass and two registers, because [`depth`] is at most two and the
    /// `const` block above is what says so. The scan is written to be correct
    /// for repeated values: two samples that are equal are the largest and the
    /// second largest, which is what sorting descending and indexing would give.
    /// Unit: nanoseconds.
    #[must_use]
    pub fn estimate_nanos(&self) -> u64 {
        if self.held == 0 {
            return 0;
        }
        let mut largest = 0;
        let mut second = 0;
        for cost in &self.costs_nanos[..self.held] {
            if *cost > largest {
                second = largest;
                largest = *cost;
            } else if *cost > second {
                second = *cost;
            }
        }
        if depth(self.held) == 1 { largest } else { second }
    }

    /// What this compositor would do about the frame that ends at `now`.
    ///
    /// The whole of `E3-B01h`'s first clause is [`Decision::wake_nanos`], and it
    /// is computed here rather than by the caller so that there is one
    /// subtraction in this tree rather than one per reader.
    #[must_use]
    pub fn decide(&self, now: Tick, period_nanos: u64, margin_nanos: u64) -> Decision {
        let scanout_nanos = scanout_after(now, period_nanos);
        let estimate_nanos = self.estimate_nanos();
        Decision {
            scanout_nanos,
            estimate_nanos,
            margin_nanos,
            wake_nanos: wake_nanos(scanout_nanos, estimate_nanos, margin_nanos),
        }
    }
}

/// When the application should start the frame that lands at the next scanout.
///
/// Four numbers rather than one, because the wake time alone cannot be checked
/// by anybody: the exit says *scanout minus the p99 estimate minus the margin*,
/// and a reader handed only the answer would have to take the subtraction on
/// trust. `crate::routing::reported` publishes all four for the same reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    /// The scanout this frame is aimed at. Unit: nanoseconds.
    pub scanout_nanos: u64,
    /// The rolling p99 of what a frame has cost. Unit: nanoseconds.
    pub estimate_nanos: u64,
    /// What is held back against an estimate being wrong. Unit: nanoseconds.
    pub margin_nanos: u64,
    /// Scanout, less the estimate, less the margin. Unit: nanoseconds.
    pub wake_nanos: u64,
}

/// The first scanout strictly after `now`.
///
/// Scanouts are multiples of the period and there is no phase term, which is a
/// simplification and is written here rather than discovered: nothing in this
/// build knows when a real display last latched a frame, because the one
/// component that could say — `user/virtio-gpu` — does not report it and
/// `E3-B02f` is the task that reaches a scanout at all. *What would reverse
/// this:* a display driver that reports the instant of its last vblank, at which
/// point the phase arrives on the routing page beside the period and this
/// function takes it as a third argument.
///
/// Strictly after, and not at-or-after: a frame whose work ends exactly on a
/// scanout boundary has missed that scanout, and aiming it at the boundary it
/// has already reached would compute a wake time in the past for every frame
/// that lands on time.
///
/// A zero period is *this build was told nothing about a display*, and the
/// answer is `now` — which makes the wake time `now` less the estimate and the
/// margin, a time already gone, which is the one answer that cannot be mistaken
/// for a schedule.
/// Unit: nanoseconds.
#[must_use]
pub const fn scanout_after(now: Tick, period_nanos: u64) -> u64 {
    if period_nanos == 0 {
        return now.nanos();
    }
    let elapsed = now.nanos() / period_nanos;
    // Saturating, because a clock reading near the top of the range would
    // otherwise wrap the next boundary round to a small number — and a scanout
    // in the past is the one value this function must not be able to return.
    elapsed.saturating_add(1).saturating_mul(period_nanos)
}

/// The exit's sentence, as one expression.
///
/// *Scanout minus the p99 estimate minus the margin*, saturating at zero. Every
/// term is a `u64` of nanoseconds and each says so in its name, which is the
/// clause's *every term an integer with its scale in its name* and is why there
/// is no `Duration` and no scaled fraction anywhere in this module.
///
/// Saturating rather than checked: a wake time before the epoch is *wake now*,
/// and the caller that would have to handle a `None` is a serve loop with
/// nowhere to report one. What the saturation costs is that a compositor whose
/// estimate exceeds a whole scanout period computes a wake time of zero every
/// frame — which is *start immediately and you will still be late*, and is the
/// correct answer rather than a degenerate one.
/// Unit: nanoseconds.
#[must_use]
pub const fn wake_nanos(scanout_nanos: u64, estimate_nanos: u64, margin_nanos: u64) -> u64 {
    scanout_nanos.saturating_sub(estimate_nanos).saturating_sub(margin_nanos)
}

/// What this compositor did about a frame it could not fit.
///
/// # Why this is a criterion and not a flag
///
/// Because `E3-B07b` decided the downgrade order and the whole of that decision
/// is *which* effect goes first. A boolean published here would be a compositor
/// saying it degraded something and leaving the reader to guess what, which is
/// the shape `scene/src/degrade.rs` opens by refusing: an order that is
/// deterministic but unwritten is not a decision anybody made.
///
/// So the word names a choice from `Criterion::ORDER`, with two values below the
/// first criterion for the two answers that are not a criterion at all.
pub mod degraded {
    /// The frame fitted, so the policy was never asked.
    ///
    /// Distinct from [`SHORT`] and it must be: *nothing was degraded* and
    /// *everything was degraded and it was not enough* are the two ends of this
    /// module's range, and a build that spelled them the same way would publish
    /// its best frame and its worst under one number.
    pub const FITTED: u64 = 0;
    /// The policy was asked and had nothing left to give back —
    /// `f_scene::degrade::Downgrades::Short`.
    ///
    /// **This is what every over-budget frame in this build answers, and the
    /// reason is a wire format rather than a defect.** `abi/src/scene.rs` has six
    /// opcodes and none of them declares an effect, so no client can tell this
    /// compositor what an effect costs or what its fallback saves — and
    /// `f_scene::effect::Effect` has exactly one constructor, which takes those
    /// two words. The policy therefore runs over an empty slice and answers that
    /// it is short by the whole overrun, which is the truth about this build.
    /// *What would reverse this:* an effect declaration on the wire, at which
    /// point the criterion words start being reached and this one goes back to
    /// meaning what it says.
    pub const SHORT: u64 = 1;
    /// What [`super::criterion_word`] adds to `Criterion::priority()`.
    ///
    /// Two, so that the first criterion of the order is 2 and the two answers
    /// above keep 0 and 1. A reader turning this back into a criterion subtracts
    /// it; `super::criterion_word` is the only place either direction is
    /// written.
    pub const FIRST_CRITERION: u64 = 2;
}

/// The word [`degraded`] publishes for one criterion.
///
/// The one place the offset is applied, so that a reader and a writer cannot
/// disagree about where the criteria start.
/// Unit: none — an ordinal in this component's tree.
#[must_use]
pub const fn criterion_word(criterion: Criterion) -> u64 {
    degraded::FIRST_CRITERION + criterion.priority() as u64
}

/// How many nanoseconds one unit of `f_scene`'s cost scale is.
///
/// `scene/src/effect.rs` declares estimates and savings in microseconds scaled
/// by a hundred, so one unit is ten nanoseconds. It is named here because this
/// module is where the two scales meet, and a conversion whose factor lives in a
/// comment is a conversion somebody will get backwards.
/// Unit: nanoseconds per `_us_x100` unit.
pub const NANOS_PER_US_X100: u64 = 10;

/// What the policy says about a frame whose estimate does not fit before its
/// deadline.
///
/// `remaining_nanos` is how long there is between now and the frame's own
/// deadline — the one a commit carried, which `f_abi::scene::Frame::deadline`
/// states and the wire refuses a commit without. A frame whose estimate fits
/// inside that has nothing to decide and answers [`degraded::FITTED`].
///
/// The conversion into `f_scene`'s scale rounds **up**, and the direction is the
/// decision: a frame over budget by less than ten nanoseconds is still over
/// budget, and rounding it down would publish *fitted* for a late frame — which
/// is the one wrong answer this word can give, because it is the one a reader
/// would not go looking behind.
/// Unit: none — a [`degraded`] ordinal.
#[must_use]
pub fn degraded_word(estimate_nanos: u64, remaining_nanos: u64) -> u64 {
    let over_nanos = estimate_nanos.saturating_sub(remaining_nanos);
    // `div_ceil` on the nanoseconds and then a narrowing that saturates: an
    // overrun wider than a `u32` of `_us_x100` units is forty-three seconds of
    // lateness, which is not a frame, and clamping it is a truer answer than
    // wrapping it round into a small one.
    let over_us_x100 = u32::try_from(over_nanos.div_ceil(NANOS_PER_US_X100)).unwrap_or(u32::MAX);
    let Some(overrun) = Overrun::new(over_us_x100) else {
        return degraded::FITTED;
    };
    // The policy, over the effects this frame declared — which is none of them,
    // for the reason `degraded::SHORT` states. It is called rather than
    // short-circuited so that the day an effect reaches this compositor the
    // answer changes here and nowhere else.
    match degrade::downgrade(&mut [], overrun) {
        Downgrades::Short { .. } => degraded::SHORT,
        // Unreachable while the slice is empty — `Covered` requires a prefix of
        // at least one effect — and written out rather than left to a wildcard,
        // because a wildcard arm is where the day this starts being reached goes
        // to be ignored. `taken` indexes the ordered slice, so with nothing left
        // behind the criterion that decided is the terminator, which is the last
        // line of `Criterion::ORDER`.
        Downgrades::Covered { .. } => criterion_word(Criterion::ORDER[Criterion::COUNT - 1]),
    }
}

#[cfg(test)]
mod tests {
    use f_env::{Env, SeededEnv};

    use super::*;

    /// The seed every run below is driven from.
    ///
    /// One `u64`, and the sequence of frame costs is a pure function of it. This
    /// is the seed the exit sentence means — *two runs from one seed compute the
    /// same wake time to the tick* — and it belongs to the workload rather than
    /// to the arithmetic, exactly as `scene/src/degrade.rs`'s does.
    const SEED: u64 = 0x5EED_B01B_0000_0001;

    /// How far the seeded environment's clock moves per draw. Unit: nanoseconds.
    const TICK_NANOS: u64 = 100;

    /// A sixty-hertz frame. Unit: nanoseconds.
    const PERIOD_NANOS: u64 = 16_666_667;

    /// What is held back against the estimate being wrong. Unit: nanoseconds.
    ///
    /// The margin is a policy this task does not decide. What it has to be for
    /// these tests is a number that is neither zero nor a multiple of the
    /// period, so that a build which dropped the term moves the answer.
    const MARGIN_NANOS: u64 = 1_300_000;

    /// How many frames the runs below close. Unit: frames.
    ///
    /// More than [`WINDOW`], so the ring wraps and the estimate is genuinely
    /// over the *last* hundred and twenty-eight rather than over everything the
    /// run ever saw. A corpus that never filled the window would be a corpus
    /// that could not tell the two apart.
    const FRAMES: usize = 300;

    /// The widest frame cost the corpus draws. Unit: nanoseconds.
    ///
    /// Eight milliseconds, which straddles half a sixty-hertz period: a corpus
    /// entirely inside the budget would never move the wake time off the scanout
    /// less the margin.
    const WIDEST_NANOS: u64 = 8_000_000;

    /// One run of the whole computation, driven from a seeded environment.
    ///
    /// The `Env` supplies both halves of the input: the cost of each frame is a
    /// draw, and the clock the decision is taken at is that environment's own —
    /// so *the same seed* really does mean *the same inputs*, rather than the
    /// same inputs plus a clock the test happened to fix.
    fn run(seed: u64) -> Decision {
        let mut env = SeededEnv::new(seed, TICK_NANOS);
        let mut pacing = Pacing::ZERO;
        for _ in 0..FRAMES {
            pacing.observe(env.next_u64() % WIDEST_NANOS);
        }
        pacing.decide(Tick(env.now().as_nanos()), PERIOD_NANOS, MARGIN_NANOS)
    }

    /// The scanout [`SEED`]'s run is aimed at. Unit: nanoseconds.
    ///
    /// A literal and not a recomputation, for `scene/src/degrade.rs`'s reason: a
    /// value compared against itself agrees on any machine, and what the exit
    /// asks for is that x86-64 and AArch64 agree about *these* numbers. `cargo
    /// xtask test` runs this module on both, so one set of literals asserted by
    /// both jobs is the whole of *on both architectures* — and nothing local can
    /// observe the second half, which is why it is written here rather than
    /// claimed in a commit message.
    const SCANOUT_NANOS: u64 = 16_666_667;

    /// The p99 that run produces. Unit: nanoseconds.
    const ESTIMATE_NANOS: u64 = 7_818_258;

    /// And the wake time the two of them and the margin make. Unit: nanoseconds.
    const WAKE_NANOS: u64 = SCANOUT_NANOS - ESTIMATE_NANOS - MARGIN_NANOS;

    #[test]
    fn two_runs_from_one_seed_compute_the_same_wake_time_to_the_tick() {
        assert_eq!(run(SEED), run(SEED), "one seed produced two different wake times");
        assert_ne!(
            run(SEED).estimate_nanos,
            run(SEED ^ 0xFFFF).estimate_nanos,
            "two seeds produced one estimate, so the window is not reading its samples",
        );
    }

    #[test]
    fn the_wake_time_is_the_scanout_less_the_estimate_less_the_margin() {
        let decided = run(SEED);
        assert_eq!(decided.scanout_nanos, SCANOUT_NANOS, "the scanout moved");
        assert_eq!(decided.estimate_nanos, ESTIMATE_NANOS, "the p99 estimate moved");
        assert_eq!(decided.margin_nanos, MARGIN_NANOS);
        // The clause, spelled as the exit spells it rather than as the code
        // does: a build whose `wake_nanos` dropped a term passes an equality
        // against its own arithmetic and fails this one.
        assert_eq!(
            decided.wake_nanos,
            SCANOUT_NANOS - ESTIMATE_NANOS - MARGIN_NANOS,
            "the wake time is not the scanout less the estimate less the margin",
        );
        assert_eq!(decided.wake_nanos, WAKE_NANOS, "the wake time this seed produces moved");
    }

    #[test]
    fn the_estimate_is_the_ninety_ninth_percentile_by_nearest_rank() {
        // A hundred samples, one per nanosecond from 1 to 100, offered in an
        // order that is not their order — so an implementation that answered
        // with the last sample, the largest, or the mean gives a different
        // number. The rank of the p99 of a hundred is 99, which is the
        // second-largest, which is 99.
        let mut pacing = Pacing::ZERO;
        for step in 0..100u64 {
            pacing.observe((step * 37) % 100 + 1);
        }
        assert_eq!(rank(100), 99);
        assert_eq!(depth(100), 2);
        assert_eq!(pacing.estimate_nanos(), 99);

        // And with one sample the rank is one and the depth is one, so the
        // estimate is that sample — the case a two-register scan gets wrong if
        // it always answers with the second register.
        let mut one = Pacing::ZERO;
        one.observe(4_200);
        assert_eq!(depth(1), 1);
        assert_eq!(one.estimate_nanos(), 4_200);
    }

    #[test]
    fn the_window_forgets_on_a_schedule() {
        // **Two** enormous frames, then a full window of small ones. A rolling
        // estimate has forgotten both; a high-water mark has not, and that is
        // the difference the word *rolling* is carrying in the exit.
        //
        // Two rather than one, and the second is not decoration: the p99 of a
        // full window sits at depth two, so a single surviving outlier is the
        // sample the percentile steps over and a one-frame version of this test
        // stays green over a window that never wraps at all. That mutation was
        // run and it survived the one-frame form, which is why there are two.
        let mut pacing = Pacing::ZERO;
        pacing.observe(900_000_000);
        pacing.observe(900_000_000);
        for _ in 0..WINDOW {
            pacing.observe(1_000);
        }
        assert_eq!(pacing.held(), WINDOW);
        assert_eq!(pacing.estimate_nanos(), 1_000, "the window is still holding a forgotten frame");
    }

    #[test]
    fn a_compositor_that_has_closed_no_frame_wakes_at_the_scanout_less_the_margin() {
        let pacing = Pacing::ZERO;
        let decided = pacing.decide(Tick(0), PERIOD_NANOS, MARGIN_NANOS);
        assert_eq!(decided.estimate_nanos, 0);
        assert_eq!(decided.scanout_nanos, PERIOD_NANOS);
        assert_eq!(decided.wake_nanos, PERIOD_NANOS - MARGIN_NANOS);
    }

    #[test]
    fn the_scanout_is_the_next_boundary_and_never_the_one_just_passed() {
        assert_eq!(scanout_after(Tick(0), PERIOD_NANOS), PERIOD_NANOS);
        // Exactly on a boundary is *that boundary has gone*, which is the half a
        // build using at-or-after gets wrong, and the half that produces a wake
        // time in the past for every frame that lands on time.
        assert_eq!(scanout_after(Tick(PERIOD_NANOS), PERIOD_NANOS), 2 * PERIOD_NANOS);
        assert_eq!(scanout_after(Tick(PERIOD_NANOS + 1), PERIOD_NANOS), 2 * PERIOD_NANOS);
        // A period nobody declared, and a reading at the top of the range.
        assert_eq!(scanout_after(Tick(77), 0), 77);
        assert_eq!(scanout_after(Tick(u64::MAX), PERIOD_NANOS), u64::MAX);
    }

    #[test]
    fn a_frame_that_fits_degrades_nothing_and_one_that_does_not_is_short() {
        assert_eq!(degraded_word(1_000, 2_000), degraded::FITTED);
        assert_eq!(degraded_word(2_000, 2_000), degraded::FITTED, "an estimate that just fits");
        // The rounding direction: one nanosecond over is a tenth of one unit of
        // `f_scene`'s scale, and rounding it down would answer `FITTED` for a
        // late frame.
        assert_eq!(degraded_word(2_001, 2_000), degraded::SHORT, "a nanosecond over rounds up");
        // And a frame with no deadline left at all is short by its whole
        // estimate rather than by nothing.
        assert_eq!(degraded_word(5_000_000, 0), degraded::SHORT);
    }

    #[test]
    fn the_criterion_words_are_distinct_from_the_two_answers_that_are_not_criteria() {
        for criterion in Criterion::ORDER {
            let word = criterion_word(criterion);
            assert!(word >= degraded::FIRST_CRITERION);
            assert_ne!(word, degraded::FITTED);
            assert_ne!(word, degraded::SHORT);
        }
    }
}
