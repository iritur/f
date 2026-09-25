// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The input path's own prediction: `E3-B04e`'s seam, seen from the driver's
//! side of the ring.
//!
//! # Why a driver predicts at all, and what it does *not* do with the answer
//!
//! `user/compositor/src/latch.rs` argues that the predictor belongs in the
//! compositor, because a prediction is about *when this frame will be scanned
//! out* and a build in which the driver predicted and sent a position would be
//! a cursor aimed at whatever the driver guessed the display was doing. That
//! argument stands, and nothing here contradicts it: **this prediction is never
//! sent**. It goes onto this component's routing page, beside the accumulator's
//! end, for the frame to read after the run, and no entry this driver submits
//! carries any of it.
//!
//! What it is for is the one sentence `E3-B04e`'s exit asks for and nothing
//! else in the tree could supply: *the latched value is the predicted one,
//! asserted from both sides of the seam*. The compositor answers *where will
//! the pointer be at my scanout* from the reports it decoded off the ring. This
//! module answers the same question from the reports this driver stamped,
//! before any of them crossed. One implementation — `f_input::predict` — and two
//! instances, fed by two pieces of code from two states, neither copied from the
//! other. The host test in `latch.rs` has always compared two instances; until
//! RFC 0134 both of them were fed from one local array, so it could not see a
//! relay, and the boot saw a relay and no prediction.
//!
//! # The instant is told, and it is a target
//!
//! On one worker core the driver has been reaped before the compositor mints
//! its aim, so the aim itself cannot reach this component — and a frame that
//! carried it here would first have had to learn it. What the frame *can* say
//! beforehand is what it already says to the compositor: when the display it
//! declares scans out. So it writes that instant onto this page,
//! `routing::at::SCANOUT_AT_NANOS`, and the boot requires the compositor's own
//! aim to equal it rather than assuming it. The frame names no reading and holds
//! none; the instant is a statement about a display, which RFC 0120 calls a
//! target, and it reaches `f_input` here exactly as the compositor's reaches it
//! there — as a field, [`Asked::scanout_nanos`], which is the spelling
//! `cargo xtask lint-stamp`'s mint rule sanctions.
//!
//! # A held prediction proves nothing, and the boot says so
//!
//! Both sides of a ring holding the newest report agree by copying one sample.
//! [`Foreseen::extrapolated`] is published so that the boot can refuse that
//! agreement on the gesture that is meant to extrapolate, which is the refusal
//! the host test makes and the boot did not.

use f_input::predict::{Predictor, Sample};
use f_input::stamp::StampNanos;

/// The scanout this component's predictor is asked about, as the frame told it.
///
/// A struct with one field, for `f_compositor::latch::Aim`'s reason exactly:
/// the number reaches `StampNanos::from_wire_nanos` as a field read, which is
/// what `lint-stamp` requires of that constructor's argument on the input path,
/// and it is honestly a field — a word off a page the frame wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// Zero is nothing told. Unit: nanoseconds, in the channel's epoch.
    pub scanout_nanos: u64,
}

/// What the input path's predictor answered.
///
/// Every field is published as one routing-page word. The value and the anchor
/// are both kept so that *how far the prediction moved the pointer* is a
/// subtraction the reader makes, not one this component did first — `Latched`'s
/// arrangement on the other side of the seam, and for its reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Foreseen {
    /// The instant asked about. Unit: nanoseconds, in the channel's epoch.
    pub for_nanos: u64,
    /// Predicted position along x. Unit: device pixels, scaled by 65 536.
    pub x_x65536: i32,
    /// Predicted position along y. Unit: device pixels, scaled by 65 536.
    pub y_x65536: i32,
    /// The newest report the prediction stands on, along x.
    /// Unit: device pixels, scaled by 65 536.
    pub anchor_x_x65536: i32,
    /// And along y. Unit: device pixels, scaled by 65 536.
    pub anchor_y_x65536: i32,
    /// How far forward it extrapolated, after damping. Unit: nanoseconds.
    pub lead_nanos: u64,
    /// Whether it extrapolated at all. Unit: none — a flag.
    pub extrapolated: bool,
}

/// The input path's predictor and what it has been fed.
#[derive(Clone, Copy, Debug)]
pub struct Forecast {
    predictor: Predictor,
    /// Positions the predictor took. Unit: reports.
    reports: u64,
    /// Positions it refused as not newer than the newest it held. A driver
    /// stamps each report once on a clock that only moves forward, so a non-zero
    /// here is this component's own clock contradicting itself.
    /// Unit: reports.
    stale: u64,
}

impl Default for Forecast {
    fn default() -> Self {
        Self::new()
    }
}

impl Forecast {
    /// A predictor that has been told nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self { predictor: Predictor::new(), reports: 0, stale: 0 }
    }

    /// One position the device reported, at the stamp this driver took for it.
    ///
    /// The sample carries the `StampNanos` the decoder opened the report with —
    /// the reading itself, not a number rebuilt from the entry's wire field —
    /// which is what makes this the source side of the seam.
    pub fn observe(&mut self, sample: Sample) -> bool {
        let taken = self.predictor.observe(sample);
        if taken {
            self.reports = self.reports.saturating_add(1);
        } else {
            self.stale = self.stale.saturating_add(1);
        }
        taken
    }

    /// Positions taken. Unit: reports.
    #[must_use]
    pub const fn reports(&self) -> u64 {
        self.reports
    }

    /// Positions refused as not newer. Unit: reports.
    #[must_use]
    pub const fn stale(&self) -> u64 {
        self.stale
    }

    /// Where the pointer will be at the instant asked about.
    ///
    /// `None` where nothing was told or nothing was reported, and neither is
    /// answered with the origin: a prediction to the beginning of time, or of a
    /// pointer that never moved, is a number nobody measured.
    #[must_use]
    pub fn at(&self, asked: Asked) -> Option<Foreseen> {
        if asked.scanout_nanos == 0 {
            return None;
        }
        // The field, not a local — the module's *the instant is told*.
        let scanout = StampNanos::from_wire_nanos(asked.scanout_nanos);
        let predicted = self.predictor.predict_at(scanout)?;
        Some(Foreseen {
            for_nanos: asked.scanout_nanos,
            x_x65536: predicted.x_x65536(),
            y_x65536: predicted.y_x65536(),
            anchor_x_x65536: predicted.anchor_x_x65536(),
            anchor_y_x65536: predicted.anchor_y_x65536(),
            lead_nanos: predicted.lead_nanos(),
            extrapolated: predicted.basis().extrapolated(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `kernel/src/input.rs`'s `STAMP_TICK_NANOS`. Unit: nanoseconds per report.
    const TICK_NANOS: u64 = 100_000;
    /// That boot's display period, and its first scanout. Unit: nanoseconds.
    const SCANOUT_NANOS: u64 = 16_666_667;
    const SCALE: i32 = 65_536;

    fn fed(motions: &[(i32, i32)]) -> (Forecast, (i32, i32)) {
        let mut forecast = Forecast::new();
        let (mut x, mut y) = (640 * SCALE, 360 * SCALE);
        for (at, (dx, dy)) in motions.iter().enumerate() {
            x += dx * SCALE;
            y += dy * SCALE;
            assert!(forecast.observe(Sample {
                at: StampNanos::from_wire_nanos(TICK_NANOS * (at as u64 + 1)),
                x_x65536: x,
                y_x65536: y,
            }));
        }
        (forecast, (x, y))
    }

    #[test]
    fn nothing_told_is_nothing_predicted() {
        let (forecast, _) = fed(&[(1, 1), (1, 1)]);
        assert_eq!(forecast.at(Asked { scanout_nanos: 0 }), None);
        assert_eq!(Forecast::new().at(Asked { scanout_nanos: SCANOUT_NANOS }), None);
    }

    /// `xtask`'s `LEAD_MOTIONS`, which `cargo xtask input lead` injects: it does
    /// not turn inside the window, so this side extrapolates — and it runs the
    /// pointer forward of its anchor on both axes, which is what makes the boot's
    /// agreement a statement rather than a copy.
    #[test]
    fn the_lead_gesture_extrapolates_on_both_axes() {
        let (forecast, end) = fed(&[(3, 1), (2, 2), (1, 2), (3, 2), (2, 1)]);
        let seen = forecast.at(Asked { scanout_nanos: SCANOUT_NANOS }).expect("a prediction");
        assert!(seen.extrapolated, "the lead gesture held, so the boot's seam would be vacuous");
        assert_eq!((seen.anchor_x_x65536, seen.anchor_y_x65536), end);
        assert!(seen.x_x65536 > end.0 && seen.y_x65536 > end.1, "{seen:?}");
        assert!(seen.lead_nanos > 0);
        assert_eq!(seen.for_nanos, SCANOUT_NANOS);
        assert_eq!((forecast.reports(), forecast.stale()), (5, 0));
    }

    /// `xtask`'s `MOTIONS`, which `cargo xtask input deliver` injects: it turns
    /// along x inside the window, so this side holds — the same answer
    /// `f_compositor`'s `the_boots_motion_is_held_and_not_extrapolated` requires
    /// of the other side.
    #[test]
    fn the_turning_gesture_holds() {
        let (forecast, end) = fed(&[(3, 5), (-1, 2), (10, -4), (2, 7), (-6, -6)]);
        let seen = forecast.at(Asked { scanout_nanos: SCANOUT_NANOS }).expect("a prediction");
        assert!(!seen.extrapolated);
        assert_eq!((seen.x_x65536, seen.y_x65536), end);
        assert_eq!(seen.lead_nanos, 0);
    }
}
