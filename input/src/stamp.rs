// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The one clock reading in the input path.
//!
//! An input event is stamped once, at the driver, at interrupt time. Every
//! stage after that — the ring, the compositor's queue, the prediction, the
//! late-latch — reads the stamp it was handed and takes no clock of its own.
//!
//! # Why once, stated as arithmetic rather than as tidiness
//!
//! The number this path exists to bound is `presented_at - stamped_at`: how
//! long after the user moved the mouse the screen showed it. Suppose a later
//! stage re-reads the clock and treats *that* as the event's time. The
//! published latency is then `presented_at - requeued_at`, and the difference
//! between the two is precisely the queueing and scheduling delay that stage
//! introduced. A second reading does not add noise to the measurement. It
//! **subtracts the regression out of it**, silently, in the direction that
//! makes the number look good — so the tree ends up with a latency figure that
//! improves as the compositor gets slower.
//!
//! That is why this is a rule with a lint behind it rather than a convention.
//! A convention is checked by whoever remembers it.
//!
//! # Why at interrupt time and not when the driver's task next runs
//!
//! Between the device raising its interrupt and the driver component being
//! scheduled there is a delay the user felt and the system did not choose.
//! Stamping at the point the driver's loop resumes moves that delay from
//! *inside* the measurement to outside it, which hides the single class of
//! regression this path is most likely to suffer: a scheduler change that
//! delays the input component. The stamp is taken in the interrupt path so
//! that a scheduler regression shows up as a worse number rather than as no
//! change at all.
//!
//! # What is removed, and what is only checked
//!
//! [`StampNanos`] has a private field, no `Default`, no `From<u64>`, and no
//! arithmetic that yields one. So no downstream stage can *fabricate* an input
//! timestamp: the type it would have to produce cannot be spelled from a
//! number it read itself. That much is removed rather than guarded.
//!
//! What is not removed is [`StampNanos::from_wire_nanos`], which the receive
//! half of the round trip needs — bytes arriving over a ring are a `u64` and
//! something has to turn them back into a stamp — and which a determined
//! caller could feed a clock reading of its own. Saying that plainly is worth
//! more than a signature that pretends otherwise: what stops
//! `from_wire_nanos(env.now().as_nanos())` is `cargo xtask lint-stamp`, which
//! refuses any clock reading anywhere on the input path except the one in this
//! file. The lint is the guard for exactly the case the type cannot remove,
//! and it has a fixture that makes it fail, because a lint nobody has watched
//! go red is a lint nobody has tested.
//!
//! That sentence was not enough on its own and the gap was a reviewer's rather
//! than a hypothesis. A clock reading does not have to be spelled at the call:
//! `from_wire_nanos(nanos_now(env))`, with the helper in a crate the lint does
//! not read, contains no spelling of `now` that any needle matches. So the
//! lint checks this constructor's *argument* as well as the path's needles —
//! a field read or a literal, and nothing that was computed here — and the
//! two-statement version of the same trick now costs a struct and a field
//! somebody has to write down. What the type removes, what the needles catch
//! and what the argument rule catches are three different sets, and this
//! paragraph is the only place that says so.
//!
//! # Under the simulator
//!
//! The stamp is `f_env::Env`'s virtual time and nothing else, so one seed
//! gives one sequence of stamps —
//! `the_sequence_a_seed_gives_is_written_down_here` pins that sequence to
//! literal numbers rather than to a second run of the same code, which is what
//! makes it a check on *both* architectures rather than on whichever one is
//! running. `f-input`'s row in the portability table answers `None` to both
//! questions, so that test runs on the arm runner too, and an architecture
//! that computed a different sequence fails there.
//!
//! # What would reverse this
//!
//! A device that stamps its own events — `E5-B06`, hardware-timestamped native
//! input. On that day the time source moves out of this tree and into the
//! device, this module becomes the *decoder* of a stamp rather than the taker
//! of one, and [`at_interrupt`] becomes the fallback for devices that cannot.
//! The rule survives that reversal unchanged; only the identity of the one
//! source moves. Nothing else should reverse it: a second source is a second
//! answer to *when did this happen*, and two answers is the bug above wearing
//! a different name.

use f_env::Env;

/// When an input event happened, in nanoseconds on the taking [`Env`]'s clock.
///
/// The scale is in the type's name on purpose. A timestamp crosses three stages
/// and a wire before anything subtracts it from anything, and a bare `u64` that
/// is nanoseconds here and microseconds two crates away is a defect nothing in
/// the language will find. `nanos` is also the accessor's name and the wire
/// constructor's, so the scale is restated at every boundary the number crosses
/// rather than recorded once in a comment nobody is reading at the call site.
///
/// # Why this orders and `f_env::WallTime` does not
///
/// [`f_env::WallTime`] deliberately implements neither `Ord` nor `PartialOrd`,
/// because civil time jumps and sorting by it is a family of bugs. This orders,
/// and the difference is not a relaxation: a `StampNanos` is derived from
/// [`f_env::Instant`], which is monotonic within one `Env`, and every stamp on
/// one input path comes from one `Env` — which is what *one time source* means
/// here. Comparing two stamps taken under different `Env`s is meaningless, and
/// exactly as meaningless as comparing two `Instant`s that way; the type system
/// does not catch it there either, and this type does not claim a guarantee its
/// substrate does not have.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct StampNanos {
    /// Nanoseconds on the taking `Env`'s clock. Private, and that is the point:
    /// a public field is a constructor, and a constructor here is a second time
    /// source with extra steps.
    nanos: u64,
}

impl StampNanos {
    /// The stamp as a plain count of nanoseconds, for the wire and for
    /// arithmetic that has left the type behind.
    #[must_use]
    pub const fn nanos(self) -> u64 {
        self.nanos
    }

    /// How long after `earlier` this stamp was taken, in nanoseconds.
    ///
    /// Saturating, for [`f_env::Instant::saturating_since`]'s reason exactly: a
    /// pair that appears to run backwards is a peer sending nonsense or a
    /// simulator fault, and neither is worth a panic in an interrupt path. Zero
    /// is the honest answer to *how much later* when the answer is *not later*,
    /// and a zero in a latency histogram is visible, where an abort is a boot
    /// that ends.
    #[must_use]
    pub const fn since_nanos(self, earlier: Self) -> u64 {
        self.nanos.saturating_sub(earlier.nanos)
    }

    /// Rebuild a stamp that arrived as bytes.
    ///
    /// The receive half of the wire round trip, and the one way to hold a
    /// `StampNanos` without having read a clock. It exists because the wire
    /// carries a `u64` — `E3-B04b` puts that field in `abi/`, and `f-input`
    /// depends on `f-abi` rather than the other way round, so the conversion
    /// has to live on this side of that edge.
    ///
    /// It is also the one hole in the type's story, and pretending otherwise
    /// would be worse than naming it: a caller that passes its own clock
    /// reading here has minted a second time source, and nothing in this
    /// signature can tell that number from one that came off a ring. What can
    /// tell them apart is *where the reading was written*, so that is what is
    /// checked — `cargo xtask lint-stamp` refuses a clock reading anywhere on
    /// the input path except the single one in [`at_interrupt`].
    ///
    /// The sentence that used to end here said the abusive expression "contains
    /// one", and it does not have to. `from_wire_nanos(helper(env))` names no
    /// clock at all when `helper` lives in a crate the lint does not read, and
    /// `xtask`'s own doc asserted the opposite half of the same argument — that
    /// no constructor of this type takes a `u64` — while this function sat here
    /// being one. So the lint now reads what is *handed* to this function as
    /// well: on the input path the argument must be a field read or a literal.
    /// That is why a decode should pass `entry.stamp_nanos` straight in rather
    /// than binding it first; a bare local is refused, because a local is
    /// exactly where a helper's return value lands.
    #[must_use]
    pub const fn from_wire_nanos(nanos: u64) -> Self {
        Self { nanos }
    }
}

/// Take the stamp. The one clock reading in the whole input path.
///
/// Called from the driver's interrupt path and from nowhere else — a second
/// call site is a second reading of the clock however it is spelled, which is
/// why `at_interrupt(` is one of the needles `cargo xtask lint-stamp` looks for
/// rather than only the spellings of `now`.
///
/// # Why a trait object rather than a generic
///
/// There is one call site in the shipped tree, so monomorphising buys nothing,
/// and `&dyn Env` makes the clock visibly a *capability handed in* at the place
/// a reader is looking for an ambient one. A generic parameter would read as
/// plumbing; this reads as the argument RFC 0004 is making.
///
/// `&self` on the clock, so this is callable from an interrupt path holding the
/// `Env` shared: taking a stamp draws nothing and advances nothing, which is
/// the other half of why stamping does not perturb a seeded run.
#[must_use]
pub fn at_interrupt(env: &dyn Env) -> StampNanos {
    // The only reading of a clock in the input path. Everything downstream
    // reads the value this returns; `cargo xtask lint-stamp` is what keeps that
    // sentence true as the path grows stages.
    StampNanos { nanos: env.now().as_nanos() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_env::SeededEnv;

    /// How far virtual time moves per observation in these tests. Any value
    /// does; a microsecond makes the numbers below readable as times rather
    /// than as hashes.
    const TICK_NANOS: u64 = 1_000;

    /// The seed the golden sequence below was taken under.
    const SEED: u64 = 0x1_4E17;

    /// Eight events, with the virtual clock advanced between them by a gap the
    /// seed decides.
    ///
    /// The gap is drawn from the `Env` rather than taken from a constant
    /// because a fixed gap would make the sequence an arithmetic series, which
    /// reproduces on any machine whether or not the generator underneath does —
    /// a test that passes for a reason other than the one it claims.
    fn sequence(seed: u64) -> [u64; 8] {
        let mut env = SeededEnv::new(seed, TICK_NANOS);
        let mut out = [0u64; 8];
        for slot in &mut out {
            let gap = env.next_u64() % 4_000_000;
            env.advance(gap);
            *slot = at_interrupt(&env).nanos();
        }
        out
    }

    #[test]
    fn the_stamp_is_the_env_clock_and_not_a_number_beside_it() {
        // The exit sentence's *under the simulator the stamp is `Env` virtual
        // time*, as an equality rather than as a correlation: not close to it,
        // not derived from it, the same number.
        let mut env = SeededEnv::new(7, TICK_NANOS);
        for _ in 0..16 {
            assert_eq!(at_interrupt(&env).nanos(), env.now().as_nanos());
            let _ = env.next_u64();
        }
    }

    #[test]
    fn the_sequence_a_seed_gives_is_written_down_here() {
        // Literal numbers, not a second call to `sequence`. Comparing two runs
        // on one machine proves the code is a function; it says nothing about
        // whether the other architecture computes the same function, which is
        // the half of the exit sentence that matters and the half `CLAUDE.md`
        // records losing twice. These eight numbers were produced on x86-64; if
        // the arm runner disagrees with any of them, the disagreement is a bug
        // below this crate and this is the assertion that surfaces it.
        assert_eq!(sequence(SEED), GOLDEN);
    }

    /// The sequence [`SEED`] gives, written down rather than recomputed.
    ///
    /// Nanoseconds of virtual time, so the run spans about twelve milliseconds
    /// of it — the last stamp is 12_090_707 ns — eight events with seeded gaps
    /// under four milliseconds. The number is restated here rather than
    /// rounded in prose because the prose is the only part of this a reader
    /// checks by eye, and it said thirteen. None of these numbers is a claim
    /// about a machine and none
    /// of them may leave this file: they are what `f_env::SeededEnv` computes
    /// from `SEED`, which is a fact about the generator and not a measurement
    /// of anything.
    const GOLDEN: [u64; 8] =
        [810_971, 2_074_208, 3_284_892, 5_251_471, 8_124_151, 8_262_288, 11_114_031, 12_090_707];

    #[test]
    fn a_seed_reproduces_its_run_and_a_different_seed_does_not() {
        assert_eq!(sequence(SEED), sequence(SEED), "a seed must reproduce its stamps exactly");
        assert_ne!(sequence(SEED), sequence(SEED + 1), "different seeds must diverge");
    }

    #[test]
    fn stamps_do_not_run_backwards_along_one_env() {
        // Not a property of `StampNanos` — it is a property of `Instant`, which
        // this inherits by reading it and by doing nothing else to it. The test
        // is here because the inheriting is the claim: an `at_interrupt` that
        // mixed anything at all into the number would break this without
        // touching `f-env`.
        let stamps = sequence(SEED);
        for pair in stamps.windows(2) {
            assert!(pair[1] >= pair[0], "{pair:?} runs backwards");
        }
    }

    #[test]
    fn the_difference_of_two_stamps_is_a_duration_and_never_a_panic() {
        let early = StampNanos::from_wire_nanos(1_000);
        let late = StampNanos::from_wire_nanos(4_500);
        assert_eq!(late.since_nanos(early), 3_500);
        // Backwards, which a peer can produce and which must not end a boot.
        assert_eq!(early.since_nanos(late), 0);
    }

    #[test]
    fn a_stamp_survives_the_wire_unchanged() {
        // The round trip `E3-B04b` will put bytes either side of: whatever the
        // entry format does to the number, the stamp it rebuilds has to be the
        // stamp that was taken, or the latency computed at the far end is the
        // latency of a different event.
        let env = SeededEnv::new(3, TICK_NANOS);
        let taken = at_interrupt(&env);
        assert_eq!(StampNanos::from_wire_nanos(taken.nanos()), taken);
    }
}
