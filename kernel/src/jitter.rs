// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A distribution, kept in a form an interrupt handler can add to.
//!
//! # Why a histogram and not a mean
//!
//! `claims/README.md` rule 3: distributions, not summaries. A mean computed at
//! collection time destroys what cannot be recovered afterwards, and it
//! systematically under-reports exactly the stalls this architecture exists to
//! remove. The whole argument of the project is about a tail, and a tail is the
//! one thing an average cannot show.
//!
//! # Why the buckets are logarithmic
//!
//! Because the range is not known in advance and the useful precision is
//! *relative*. On real hardware a 1 kHz timer is late by tens or hundreds of
//! nanoseconds; under an emulator it is late by milliseconds. Fixed-width
//! buckets have to choose, and choosing wrongly does not produce a coarse
//! histogram — it produces one bar. That is not a distribution, and it was the
//! first thing this file did.
//!
//! So each octave is divided into eight, and every bucket is within an eighth
//! of its own value. One structure covers nanoseconds to tens of milliseconds
//! at a constant twelve and a half percent, which is the same trade every
//! serious latency histogram makes and for the same reason.
//!
//! # What the shape of this type is for
//!
//! It is written to be updated from inside an interrupt handler, on the sample
//! path being measured. That rules out most of what a histogram would otherwise
//! do:
//!
//! - **No allocation**, because there is no allocator and there should not be
//!   one on this path even when there is.
//! - **No floating point**, because the kernel does not save the floating-point
//!   registers across an interrupt and is not going to start in order to draw a
//!   graph.
//! - **No division.** Placing a sample is a leading-zero count, a shift and a
//!   mask — the logarithm is one instruction. A division here would be tens of
//!   cycles *inside the interval being measured*, which is measuring the
//!   instrument.
//!
//! Everything expensive — percentiles, the conversion out of counter ticks into
//! nanoseconds, the printing — happens afterwards, when the run is over and
//! nothing is being timed.
//!
//! # Units
//!
//! Ticks of whatever counter the caller is using. This module never learns what
//! a tick is worth; the conversion is applied at the end by whoever knows the
//! frequency, which keeps the arithmetic here exact and keeps a measured
//! frequency out of the hot path.

use core::fmt::Write;

/// How finely each octave is divided, as a shift. Eight sub-buckets.
///
/// This is the precision knob and it is the only one. Eight puts every sample
/// within 12.5% of its bucket's value, which resolves a 5 µs bound to about
/// half a microsecond — fine enough to see the bound, coarse enough that the
/// whole range fits in a few kilobytes of per-core state.
const SUB_BITS: u32 = 3;

/// Sub-buckets per octave.
const SUB: u64 = 1 << SUB_BITS;

/// How many buckets a histogram has.
///
/// The first [`SUB`]`* 2` count single ticks, because at the bottom of the
/// range a logarithm has nothing left to divide. Above that each octave takes
/// [`SUB`] of them, and 256 buys thirty octaves: a billion times the smallest
/// resolvable interval, which at a few gigahertz is one nanosecond at one end
/// and several seconds at the other.
///
/// That range is deliberately absurd for the thing being measured, and the
/// reason is that the first version was not. Sized to the 5 µs bound the
/// milestone cares about, every sample from an emulated machine landed past the
/// top and the histogram was a single bar — a correct summary of nothing. The
/// top bucket saturates and says so, but a distribution whose overflow is where
/// all the data went is not a distribution, and the cheapest way to never be in
/// that position again is a range nothing can plausibly leave. It costs a
/// kilobyte per core.
pub const BUCKETS: usize = 256;

/// Samples, bucketed by magnitude, plus the summaries a bucket cannot give.
#[derive(Clone, Copy)]
pub struct Histogram {
    /// Counts, indexed by [`Histogram::index`], saturating into the last.
    buckets: [u32; BUCKETS],
    /// Samples recorded.
    count: u64,
    /// Every sample added up, for an exact mean. At a thousand samples a second
    /// for a minute this cannot come close to overflowing, and it saturates
    /// rather than wrapping into a nonsense average.
    sum: u64,
    /// The smallest sample, exactly. `u64::MAX` while there are none.
    min: u64,
    /// The largest sample, exactly — which is the number a bucket cannot tell
    /// you and the one most worth knowing.
    max: u64,
}

impl Histogram {
    /// An empty histogram.
    ///
    /// `const` because these live in `static` per-CPU state, which has to be
    /// constructible before anything runs.
    #[must_use]
    pub const fn new() -> Self {
        Self { buckets: [0; BUCKETS], count: 0, sum: 0, min: u64::MAX, max: 0 }
    }

    /// Which bucket a sample belongs in.
    ///
    /// Below `2 * SUB` every value gets its own bucket: the logarithm has run
    /// out of resolution there, and those samples are small enough that the
    /// exactness is free. Above it, the octave picks the group of [`SUB`] and
    /// the top [`SUB_BITS`] bits below the leading one pick the member.
    const fn index(sample: u64) -> usize {
        if sample < SUB * 2 {
            return sample as usize;
        }
        // At least `SUB * 2`, so the logarithm is at least `SUB_BITS + 1` and
        // the shift below cannot be negative.
        let octave = sample.ilog2();
        let shift = octave - SUB_BITS;
        let mantissa = (sample >> shift) & (SUB - 1);
        let index = (octave - SUB_BITS) as usize * SUB as usize + mantissa as usize + SUB as usize;
        if index >= BUCKETS { BUCKETS - 1 } else { index }
    }

    /// One past the largest sample a bucket holds.
    ///
    /// The inverse of [`Self::index`], and written next to it so the two are
    /// read together: an edge that disagrees with the placement is a histogram
    /// that reports the wrong number for every percentile, silently.
    const fn edge(index: usize) -> u64 {
        if (index as u64) < SUB * 2 {
            return index as u64 + 1;
        }
        let above = index as u64 - SUB;
        let shift = (above / SUB) as u32;
        let mantissa = above % SUB;
        (SUB + mantissa + 1) << shift
    }

    /// Empty it.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Add one sample.
    ///
    /// A leading-zero count, a shift, a mask and three adds. Nothing here can
    /// fault, allocate, divide or block, which is what makes it callable from
    /// the handler whose lateness it is recording.
    pub fn record(&mut self, sample: u64) {
        let index = Self::index(sample);
        self.buckets[index] = self.buckets[index].saturating_add(1);
        self.count += 1;
        self.sum = self.sum.saturating_add(sample);
        if sample < self.min {
            self.min = sample;
        }
        if sample > self.max {
            self.max = sample;
        }
    }

    /// How many samples.
    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// The smallest sample, or zero if there are none.
    #[must_use]
    pub const fn min(&self) -> u64 {
        if self.count == 0 { 0 } else { self.min }
    }

    /// The largest sample.
    #[must_use]
    pub const fn max(&self) -> u64 {
        self.max
    }

    /// The exact mean, truncated. Zero if there are no samples.
    #[must_use]
    pub const fn mean(&self) -> u64 {
        match self.sum.checked_div(self.count) {
            Some(mean) => mean,
            None => 0,
        }
    }

    /// Samples that landed in the saturating top bucket.
    ///
    /// Reported rather than hidden: a histogram whose overflow is not empty is
    /// one whose percentiles are bounded by its range rather than by the data,
    /// and reading one without knowing that is how a coarse measurement gets
    /// quoted as a fine one.
    #[must_use]
    pub const fn overflow(&self) -> u32 {
        self.buckets[BUCKETS - 1]
    }

    /// An upper bound on the sample at the given quantile, in ticks.
    ///
    /// `quantile(99, 100)` is the p99. The answer is the *upper edge* of the
    /// bucket the sample falls in, so it is an over-estimate bounded by that
    /// bucket's width — which is the honest direction to be wrong in for a
    /// latency bound, and the reason the printed form says `<=`.
    ///
    /// In the saturating top bucket the edge is meaningless, so the largest
    /// sample seen is returned instead: still an upper bound, and exact.
    #[must_use]
    pub fn quantile(&self, numerator: u64, denominator: u64) -> u64 {
        if self.count == 0 {
            return 0;
        }

        // The rank being asked for, one-based, rounded up so that p99 of a
        // hundred samples is the ninety-ninth and not the ninety-eighth.
        let rank = (self.count * numerator).div_ceil(denominator).max(1);

        let mut seen = 0u64;
        for (index, count) in self.buckets.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                if index == BUCKETS - 1 {
                    return self.max;
                }
                return Self::edge(index);
            }
        }

        // Only reachable if `count` disagrees with the buckets, which would be
        // a bug in `record`. The largest sample is the safe answer.
        self.max
    }

    /// Ticks of a counter running at `khz` kilohertz, in nanoseconds.
    ///
    /// Multiply first, divide last: at a gigahertz a tick is under a
    /// nanosecond, so dividing first would report every sub-microsecond sample
    /// as zero. The product cannot overflow for any sample a timer this side of
    /// a stopped machine produces — a full second at 5 GHz is 5e9 ticks, and
    /// 5e9 · 1e6 is well inside sixty-four bits.
    #[must_use]
    pub const fn ticks_to_ns(ticks: u64, khz: u64) -> u64 {
        match ticks.saturating_mul(1_000_000).checked_div(khz) {
            Some(ns) => ns,
            None => 0,
        }
    }

    /// Print the distribution, converted through a counter running at `khz`.
    ///
    /// Empty buckets are skipped. That is a compression rather than an
    /// omission: every line carries its own upper edge, so a gap in the output
    /// is a gap in the data and reads as one.
    pub fn report(&self, khz: u64, into: &mut impl Write) {
        let _ =
            writeln!(into, "    samples       {}, bucketed to within {}%", self.count, 100 / SUB);

        if self.count == 0 {
            return;
        }

        let tallest = self.buckets.iter().copied().max().unwrap_or(1).max(1);

        for (index, count) in self.buckets.iter().enumerate() {
            if *count == 0 {
                continue;
            }
            let last = index == BUCKETS - 1;

            // Forty columns at the tallest bucket, scaled by integers. A bar
            // that would round to nothing still gets one column, so a bucket
            // with something in it never looks empty.
            let bar = ((u64::from(*count) * 40) / u64::from(tallest)).max(1) as usize;

            let _ = write!(
                into,
                "    {:>4} {:>9} ns  ",
                if last { ">" } else { "<=" },
                Self::ticks_to_ns(Self::edge(index), khz)
            );
            for _ in 0..bar {
                let _ = write!(into, "#");
            }
            let _ = writeln!(into, " {count}");
        }

        let _ = writeln!(
            into,
            "    p50           <= {} ns    p99 <= {} ns    p99.9 <= {} ns",
            Self::ticks_to_ns(self.quantile(50, 100), khz),
            Self::ticks_to_ns(self.quantile(99, 100), khz),
            Self::ticks_to_ns(self.quantile(999, 1000), khz),
        );
        let _ = writeln!(
            into,
            "    min {} ns, mean {} ns, max {} ns",
            Self::ticks_to_ns(self.min(), khz),
            Self::ticks_to_ns(self.mean(), khz),
            Self::ticks_to_ns(self.max(), khz),
        );

        if self.overflow() > 0 {
            let _ = writeln!(
                into,
                "    note          {} sample(s) past the top bucket; every percentile in it \
                 is bounded by the histogram's range and not by the data",
                self.overflow()
            );
        }
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Prove the arithmetic before a measurement depends on it.
///
/// The kernel cannot be tested on the host — it is `no_std` with its own panic
/// handler, so a test harness cannot link against it — which is why the tree's
/// other invariants are checked the same way, at boot, by
/// [`crate::percpu::self_test`] and [`crate::mem::self_test`]. This is the same
/// bargain: the properties that matter are asserted on the machine, on every
/// run, against a fixture built from known numbers.
pub fn self_test() -> Result<(), &'static str> {
    let mut hist = Histogram::new();

    // An empty histogram has to answer every question without dividing by its
    // own count. This is the check that would have caught the obvious first
    // version of `mean`.
    if hist.quantile(99, 100) != 0 || hist.mean() != 0 || hist.min() != 0 {
        return Err("an empty histogram did not answer with zero");
    }

    // The placement and the edge are inverses of each other. This is the
    // property everything else rests on: an edge that disagrees with the
    // placement gives a wrong answer for every percentile and never says so.
    // Checked across the linear region, the join, and twenty octaves above it.
    let mut sample = 0u64;
    while sample < 1 << 24 {
        let index = Histogram::index(sample);
        if index != BUCKETS - 1 {
            let edge = Histogram::edge(index);
            if sample >= edge {
                return Err("a sample landed in a bucket whose edge is below it");
            }
            if index > 0 && sample < Histogram::edge(index - 1) {
                return Err("a sample landed above the bucket that should hold it");
            }
        }
        // Every value up to the join, then a spread that lands on every
        // sub-bucket of every octave above it rather than only on the round
        // numbers, where an off-by-one hides.
        sample = if sample < 64 { sample + 1 } else { sample + (sample >> 5).max(1) };
    }

    // A sample past the top of the range must saturate into the last bucket
    // rather than index past the end of the array.
    if Histogram::index(u64::MAX) != BUCKETS - 1 {
        return Err("an enormous sample did not saturate into the top bucket");
    }

    // Ninety-nine samples of zero and one enormous outlier. The rank for p99 of
    // a hundred samples is the ninety-ninth, which is still a zero; p99.9
    // rounds up to the hundredth, which is the outlier. A histogram that
    // rounded the rank down would report the outlier at p99 and hide it at
    // p99.9 — the same mistake in both directions at once, and invisible
    // without a fixture.
    hist.reset();
    for _ in 0..99 {
        hist.record(0);
    }
    hist.record(u64::MAX);

    if hist.count() != 100 {
        return Err("a sample went missing");
    }
    if hist.quantile(99, 100) != 1 {
        return Err("p99 of ninety-nine zeroes is not the first bucket's edge");
    }
    if hist.quantile(999, 1000) != u64::MAX {
        return Err("p99.9 did not reach the outlier");
    }
    if hist.overflow() != 1 || hist.max() != u64::MAX {
        return Err("the top bucket did not saturate, or lost the largest sample");
    }
    if hist.min() != 0 {
        return Err("the exact summaries did not survive bucketing");
    }

    // A tick is a tick until somebody says otherwise, and the conversion is the
    // one place a measured frequency touches the arithmetic. At one megahertz a
    // tick is a microsecond.
    if Histogram::ticks_to_ns(3, 1_000) != 3_000 || Histogram::ticks_to_ns(1, 0) != 0 {
        return Err("the tick-to-nanosecond conversion is wrong");
    }

    Ok(())
}
