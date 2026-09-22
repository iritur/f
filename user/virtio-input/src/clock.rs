// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The one call to the one clock reading in the input path, and an honest
//! account of what the number it produces is.
//!
//! # What this file is for
//!
//! `input/src/stamp.rs` holds the rule — an input event is stamped once, at the
//! driver, at interrupt time, and every later stage reads the stamp it was
//! handed. It has held that rule since `E3-B04a`. What it did not have until
//! this crate existed was a **caller**: `at_interrupt` had none outside its own
//! `#[cfg(test)]` module and no crate in the workspace depended on `f-input`, so
//! three of the input path's four stages were places held open rather than
//! stages being checked. RFC 0099 narrowed the claim to what was measurable and
//! named `E3-B04d` as what would widen it again. This is that file, and
//! [`Interrupt::stamp`] is that call.
//!
//! There is exactly one of it, in exactly this file, and `cargo xtask
//! lint-stamp` counts it. Two calls is a second answer to *when did this
//! happen*; zero calls is the vacuity RFC 0099 was written about, returning
//! quietly. Both are red.
//!
//! # What the number is, stated in full because understating it would be the
//! defect
//!
//! **Virtual time from a seeded [`SeededEnv`], advanced one tick per report
//! drained from the device.** It is not a reading of a hardware counter, it is
//! not a wall clock, and it is not a timestamp the device took. Three separate
//! facts make that the only thing available here and each is worth its own
//! sentence:
//!
//! - **A component in this build has no clock.** RFC 0004 says nothing observes
//!   time except through `f_env::Env`, and the only `Env` with a hardware source
//!   behind it is `kernel/src/env.rs`'s `Hardware`, which reads the timestamp
//!   counter inside the frame. A ring-3 component cannot reach it: there is no
//!   opcode that asks the frame what time it is, and there could not usefully be
//!   one, because a ring round trip per event would put the queueing delay this
//!   path exists to measure *inside* the measurement. `user/virtio-gpu`'s
//!   routing page says the same thing one field at a time — *a count and not a
//!   duration, because RFC 0004 offers a component no clock*.
//! - **`virtio_input_event` carries no timestamp.** The evdev record the device
//!   writes is a type, a code and a value: eight bytes, and not one of them is a
//!   time. The `input_event` a Linux userspace program reads has a `timeval` in
//!   front of it, and that field is added by the *host kernel* when it drains the
//!   device — which is precisely the role this component has. So there is no
//!   device timestamp being discarded here. There is none to discard.
//! - **A stamp is mandatory.** `f_abi::input::NOT_STAMPED` is zero and
//!   `Event::decode` refuses it, deliberately, so that a page of zeroes is not a
//!   pointer sitting at the origin at the beginning of time. An unstamped event
//!   is not a thing this format can express, so a driver that had no clock at all
//!   could not submit anything.
//!
//! What the seeded number therefore buys, exactly: **ordering and
//! reproducibility**, and nothing else. Two events drained in one order carry
//! stamps in that order; one seed gives one sequence of stamps on both
//! architectures, which is the half of `E3-B04a`'s exit that says *under the
//! simulator the stamp is `Env`'s virtual time*. What it does **not** buy is
//! latency: `presented_at - stamped_at` computed across a stamp from this file
//! is the number of reports the device produced between the two, scaled by a
//! tick, and it is not milliseconds of anything. Any claim built on it would be
//! a claim about arithmetic this component performed on itself.
//!
//! # What would reverse this
//!
//! `E5-B06`, hardware-timestamped native input. On that day the stamp comes off
//! a device that took it, this module becomes a decoder rather than a taker, and
//! `input/src/stamp.rs`'s own *what would reverse this* section is the one that
//! moves. The rule survives unchanged; only the identity of the one source
//! moves, which is what that section already says.
//!
//! The nearer reversal, and the one a reader should expect first, is a frame
//! that delivers this component an interrupt (`E1-B09`) together with the
//! instant it arrived. That does not need a clock in ring 3 — the frame already
//! has one and is already in the interrupt path — and it would make the stamp a
//! measurement rather than an ordinal without giving a component any authority
//! it does not have. It is not built here because the interrupt is not delivered
//! here: this driver polls, exactly as the other three do, and stamping a
//! *polled* observation with the frame's clock would record when this component
//! next ran rather than when the user acted, which is the one substitution
//! `input/src/stamp.rs` spends a module arguing against.
//!
//! `E3-P01`'s photodiode rig is the instrument that would settle which of those
//! two numbers is worth publishing, and it measures the screen from outside the
//! machine precisely because nothing inside it can.

use f_env::SeededEnv;
use f_input::stamp::StampNanos;

/// The clock this component stamps against, and the one call that reads it.
///
/// Holds a [`SeededEnv`] rather than borrowing one, because there is nowhere to
/// borrow it from: a component is handed capabilities and memory, not services,
/// and the two numbers this is built out of arrive in the routing page like
/// everything else the frame decides. `crate::routing::at::STAMP_SEED` and
/// `crate::routing::at::STAMP_TICK_NANOS` are those fields.
pub struct Interrupt {
    env: SeededEnv,
    /// How far virtual time moves per report drained from the device.
    /// Unit: nanoseconds per report.
    tick_nanos: u64,
    /// Where the clock above has been advanced to.
    ///
    /// **Not a second time source, and the distinction is the whole reason this
    /// field is documented rather than obvious.** It is not read from anywhere;
    /// it is the running total of what [`Interrupt::stamp`] has *told* the `Env`
    /// to advance by, and it starts at zero because a `SeededEnv` does. Asking
    /// the `Env` instead would mean reading its clock, and this file may contain
    /// exactly one clock reading — `cargo xtask lint-stamp` counts them, and the
    /// one it is counting is the `at_interrupt` call below.
    ///
    /// What it is for is the one decision [`Interrupt::stamp`] has to make
    /// before it reads: whether the next tick lands on
    /// `f_abi::input::NOT_STAMPED`. Unit: nanoseconds.
    at_nanos: u64,
}

impl Interrupt {
    /// Build the clock from what the frame wrote.
    ///
    /// `None` for a tick of zero, which is refused rather than defaulted: a
    /// clock that does not move gives every event in a run the same stamp, and
    /// every stage downstream would then read a queue of events that all
    /// happened at once. That is a routing page this build cannot honour, and
    /// `crate::component` refuses it where it refuses every other field it
    /// cannot state — before the device is started, rather than as a run whose
    /// stamps are all equal.
    ///
    /// The seed is not checked, because there is no bad one: `f_env::split`'s
    /// generator has no fixed point, which `env/src/lib.rs` says and tests.
    #[must_use]
    pub const fn new(seed: u64, tick_nanos: u64) -> Option<Self> {
        if tick_nanos == 0 {
            return None;
        }
        Some(Self { env: SeededEnv::new(seed, tick_nanos), tick_nanos, at_nanos: 0 })
    }

    /// Take the stamp for one report the device has just published.
    ///
    /// **The one call to `f_input::stamp::at_interrupt` in this workspace that
    /// is not a test.** Called once per report — once per `EV_SYN`, not once per
    /// evdev record — because `abi/src/input.rs` says two entries bearing one
    /// stamp are one instant and there is only one clock reading per instant to
    /// bear. A mouse that moved and pressed a button in the same report did both
    /// at one time, and stamping the two entries separately would invent an
    /// order the device did not report.
    ///
    /// The clock is advanced **before** the reading and not after, and that is
    /// load-bearing rather than tidy: `SeededEnv` starts at zero,
    /// `f_abi::input::NOT_STAMPED` is zero, and an entry carrying it is refused
    /// by the decoder. Advancing first is what makes the first event of a run a
    /// legal one instead of the one entry this component can produce and no peer
    /// can accept.
    pub fn stamp(&mut self) -> StampNanos {
        // The wrap. `SeededEnv::advance` is `wrapping_add`, so a run long enough
        // — or a tick large enough — lands exactly on zero, and zero is the one
        // value this type may not produce. Stepping one nanosecond further is
        // the whole repair: the sequence is as monotonic across the wrap either
        // way, and losing a nanosecond once every 2^64 of them is cheaper than
        // one entry per epoch that every stage downstream refuses.
        //
        // *Reversal:* a stamp type that can express *no stamp* out of band, at
        // which point zero stops being special and this branch goes. RFC 0028's
        // asymmetry is the shape that would allow it and `abi/src/input.rs`
        // argues why the wire does not have it today.
        let mut step = self.tick_nanos;
        if self.at_nanos.wrapping_add(step) == f_abi::input::NOT_STAMPED {
            step = step.wrapping_add(1);
        }
        self.at_nanos = self.at_nanos.wrapping_add(step);
        self.env.advance(step);
        f_input::stamp::at_interrupt(&self.env)
    }

    /// Where this component has advanced its clock to. Unit: nanoseconds.
    ///
    /// The mirror described on [`Interrupt::at_nanos`], published so that a
    /// caller that wants to know how far the run has got does not have to read
    /// the `Env` to find out — which would be a second clock reading in whatever
    /// file asked.
    #[must_use]
    pub const fn at_nanos(&self) -> u64 {
        self.at_nanos
    }

    /// How far virtual time moves per report. Unit: nanoseconds per report.
    ///
    /// Published so that a reader of a boot log can turn a stamp back into a
    /// count of reports, which is the only thing it is: this is the scale of a
    /// number that is an ordinal, and a reader who did not have it would be
    /// looking at nanoseconds that are not nanoseconds of anything.
    #[must_use]
    pub const fn tick_nanos(&self) -> u64 {
        self.tick_nanos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tick a reader can do arithmetic on by eye.
    const TICK_NANOS: u64 = 1_000;

    #[test]
    fn a_clock_that_does_not_move_is_refused_rather_than_defaulted() {
        assert!(Interrupt::new(7, 0).is_none(), "a tick of zero stamps every event alike");
        assert!(Interrupt::new(7, 1).is_some(), "one nanosecond is a clock");
    }

    #[test]
    fn the_first_stamp_of_a_run_is_not_the_one_value_the_wire_refuses() {
        // `SeededEnv` starts at zero and `f_abi::input::NOT_STAMPED` is zero, so
        // a driver that read the clock before advancing it would submit exactly
        // one entry per run that no decoder will accept — and it would be the
        // first, which is the one a boot looks at.
        let mut clock = Interrupt::new(0x1_4E17, TICK_NANOS).expect("a tick");
        assert_eq!(clock.stamp().nanos(), TICK_NANOS);
        assert_ne!(clock.stamp().nanos(), f_abi::input::NOT_STAMPED);
    }

    #[test]
    fn the_wrap_onto_zero_is_stepped_over_rather_than_submitted() {
        // Reachable, and reached here: two ticks of 2^63 land exactly on zero.
        // The branch is in `stamp` because the alternative is one refused entry
        // somewhere past the heat death of the run, which is the kind of bug
        // that is found by a peer rather than by a test.
        let mut clock = Interrupt::new(1, 1 << 63).expect("a tick");
        assert_eq!(clock.stamp().nanos(), 1 << 63);
        let after = clock.stamp().nanos();
        assert_ne!(
            after,
            f_abi::input::NOT_STAMPED,
            "the wrap must not produce an unstamped event"
        );
        assert_eq!(after, 1, "one nanosecond past the wrap, and the mirror says so too");
        assert_eq!(clock.at_nanos(), after);
    }

    #[test]
    fn stamps_advance_once_per_report_and_never_backwards() {
        let mut clock = Interrupt::new(3, TICK_NANOS).expect("a tick");
        let mut last = clock.stamp();
        for _ in 0..16 {
            let next = clock.stamp();
            assert!(next > last, "{next:?} does not follow {last:?}");
            assert_eq!(next.since_nanos(last), TICK_NANOS, "one report is one tick");
            last = next;
        }
    }

    #[test]
    fn one_seed_gives_one_sequence_and_the_tick_is_the_whole_of_it() {
        // The honesty test. This clock is seeded, and a reader who assumed the
        // seed perturbs the *stamps* would be reading noise into a sequence that
        // has none: `SeededEnv::advance` does not draw, so the seed decides the
        // generator and the tick decides the times. Saying so as an assertion is
        // what keeps `clock`'s module comment from being the only place it is
        // true.
        let run = |seed: u64| {
            let mut clock = Interrupt::new(seed, TICK_NANOS).expect("a tick");
            let mut out = [0u64; 8];
            for slot in &mut out {
                *slot = clock.stamp().nanos();
            }
            out
        };
        assert_eq!(run(1), run(1), "a seed must reproduce its stamps exactly");
        assert_eq!(run(1), run(2), "and the stamps are the tick, not the draw stream");
        assert_eq!(run(1), [1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000, 8_000]);
    }
}
