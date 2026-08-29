// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Measurement harness for the claims registry.
//!
//! # The rule this crate exists to enforce
//!
//! **Record distributions, never summaries.** Averaging at collection time
//! destroys information that cannot be recovered afterwards, and a harness that
//! computes a mean while pausing to record it systematically under-reports
//! exactly the stalls this architecture exists to eliminate.
//!
//! There is deliberately no `mean()` on [`Histogram`]. If you want one, you can
//! compute it from the buckets — but p50, p99 and p99.9 are what the claims
//! registry stores, because an average hides the behaviour the whole design is
//! aimed at.
//!
//! # What a claim must report
//!
//! Nanoseconds alone conflate an algorithmic win with a clock speed. Every
//! claim reports instructions retired and joules per operation alongside time,
//! because those survive a hardware change and are far less noisy.
//!
//! At M0 the counter and energy sources are not wired — [`Sample`] carries the
//! fields and marks them absent, so a claim that cannot yet report them says so
//! rather than silently omitting them.
//!
//! See `docs/design/proving-ground.html` layer 5.

use std::fmt;

/// Log-linear histogram over nanosecond values.
///
/// Two significant figures per power of two, which is enough resolution for a
/// latency tail and cheap enough to record on the hot path without perturbing
/// what is being measured.
#[derive(Clone, Debug)]
pub struct Histogram {
    buckets: Vec<u64>,
    count: u64,
    min: u64,
    max: u64,
}

const SUB_BUCKETS: usize = 16;
const BUCKET_COUNT: usize = 64 * SUB_BUCKETS;

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    /// An empty histogram.
    #[must_use]
    pub fn new() -> Self {
        Self { buckets: vec![0; BUCKET_COUNT], count: 0, min: u64::MAX, max: 0 }
    }

    fn index(value: u64) -> usize {
        if value == 0 {
            return 0;
        }
        let magnitude = 63 - value.leading_zeros() as usize;
        let sub = if magnitude == 0 {
            0
        } else {
            // Top bits below the leading one, scaled into SUB_BUCKETS.
            ((value >> (magnitude.saturating_sub(4))) as usize) & (SUB_BUCKETS - 1)
        };
        (magnitude * SUB_BUCKETS + sub).min(BUCKET_COUNT - 1)
    }

    /// The smallest value that lands in this bucket.
    fn value_at(index: usize) -> u64 {
        let magnitude = index / SUB_BUCKETS;
        let sub = (index % SUB_BUCKETS) as u64;
        if magnitude == 0 {
            return sub;
        }
        (1u64 << magnitude) | (sub << magnitude.saturating_sub(4))
    }

    /// The largest value that lands in this bucket.
    ///
    /// This is the number a percentile reports, and the reason is in
    /// `quantile`'s documentation: for a latency histogram the low edge of a
    /// bucket is the optimistic edge.
    ///
    /// Below magnitude four a bucket holds exactly one value and the two bounds
    /// are equal. Above it the width doubles each octave, so the gap between
    /// them is the resolution — never worse than about 6% of the value.
    fn upper_at(index: usize) -> u64 {
        let magnitude = index / SUB_BUCKETS;
        if magnitude == 0 {
            // Bucket zero is the only one holding two values: `index` maps both
            // 0 and 1 into it. The rest of that row is unreachable.
            return Self::value_at(index).max(1);
        }
        let width = 1u64 << magnitude.saturating_sub(4);
        Self::value_at(index).saturating_add(width - 1)
    }

    /// Record one observation.
    pub fn record(&mut self, nanos: u64) {
        self.buckets[Self::index(nanos)] += 1;
        self.count += 1;
        self.min = self.min.min(nanos);
        self.max = self.max.max(nanos);
    }

    /// Observations recorded.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Smallest observation, or zero if empty.
    #[must_use]
    pub fn min(&self) -> u64 {
        if self.count == 0 { 0 } else { self.min }
    }

    /// Largest observation.
    #[must_use]
    pub fn max(&self) -> u64 {
        self.max
    }

    /// Value at the given quantile, in the range 0.0 to 1.0.
    ///
    /// Nearest-rank: the reported value is the bucket holding the
    /// `ceil(q * n)`-th observation.
    ///
    /// # Which edge of the bucket, and why it matters
    ///
    /// The bucket's **upper** bound, clamped to the largest value actually
    /// observed.
    ///
    /// A bucket says "between 992 and 1023 ns, forty times". Reporting 992
    /// understates every one of those forty observations; reporting 1023
    /// overstates them by at most the resolution. For a latency histogram the
    /// low edge is the *optimistic* edge, and `claims/README.md` is explicit
    /// that a summary which systematically under-reports stalls destroys
    /// exactly what this architecture exists to be measured on. So the number
    /// leans the way that cannot flatter us.
    ///
    /// The clamp to `max` costs nothing and buys two properties worth having:
    /// a percentile never exceeds a value that was actually seen, and
    /// `quantile(1.0)` is exactly `max` rather than the low edge of the bucket
    /// `max` happens to sit in.
    ///
    /// Both bounds are in the emitted bucket list, so a third party re-analysing
    /// the distribution can compute either convention and see which this is.
    #[must_use]
    pub fn quantile(&self, q: f64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let target = (q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64;
        let mut seen = 0u64;
        for (i, &n) in self.buckets.iter().enumerate() {
            seen += n;
            if seen >= target {
                return Self::upper_at(i).min(self.max);
            }
        }
        self.max
    }

    /// Emit the full bucket list, so a third party can re-analyse rather than
    /// trusting the percentiles computed here.
    #[must_use]
    pub fn to_jsonl(&self, label: &str) -> String {
        let mut out = String::new();
        // Both bounds, named. A single `ns` field would leave the reader
        // guessing which edge it is, and a re-analysis that guesses the other
        // one disagrees with the percentiles published beside it — quietly,
        // and in the flattering direction.
        for (i, &n) in self.buckets.iter().enumerate() {
            if n > 0 {
                out.push_str(&format!(
                    "{{\"claim\":\"{label}\",\"ns_lo\":{},\"ns_hi\":{},\"count\":{n}}}\n",
                    Self::value_at(i),
                    Self::upper_at(i)
                ));
            }
        }
        out
    }
}

impl fmt::Display for Histogram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "n={} min={} p50={} p99={} p99.9={} max={}",
            self.count,
            self.min(),
            self.quantile(0.50),
            self.quantile(0.99),
            self.quantile(0.999),
            self.max()
        )
    }
}

/// A metric that may not be available on this machine yet.
///
/// Reporting `Unavailable` is the honest option and is what the claims registry
/// stores; silently omitting a metric is how a claim quietly narrows.
#[derive(Clone, Copy, Debug)]
pub enum Metric {
    /// Measured.
    Value(f64),
    /// The source is not wired on this platform. Carries why.
    Unavailable(&'static str),
}

impl fmt::Display for Metric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(v) => write!(f, "{v:.3}"),
            Self::Unavailable(why) => write!(f, "unavailable ({why})"),
        }
    }
}

/// One claim's result.
#[derive(Debug)]
pub struct Sample {
    /// Claim name, matching the file in `claims/`.
    pub claim: &'static str,
    /// The full latency distribution.
    pub latency: Histogram,
    /// Instructions retired per operation.
    pub instructions_per_op: Metric,
    /// Joules per operation.
    pub joules_per_op: Metric,
    /// Fraction of the run the core spent in an idle state, in `[0, 1]`.
    ///
    /// Absent, and absent for a reason rather than for want of wiring: nothing
    /// enters an idle state at all yet. The kernel spins between ticks and
    /// `apic::wait` says why — the idle-exit path would otherwise sit inside
    /// every jitter sample. RFC 0006 makes the depth computable from the
    /// reservation table and E5-B07 implements it, and that is the point at
    /// which this becomes a number.
    ///
    /// One fraction is a summary, which is the thing this crate exists to
    /// refuse. What RFC 0006 is actually about is residency *per state* — a
    /// core that idled deeply and a core that idled often and shallowly are the
    /// same scalar and are not the same result. This widens to a breakdown when
    /// there is a source to fill it; the scalar is what the registry can ingest
    /// today, and the gap is stated here rather than discovered by whoever
    /// first tries to publish an energy number.
    pub idle_residency: Metric,
}

impl Sample {
    /// Begin a claim run.
    #[must_use]
    pub fn new(claim: &'static str) -> Self {
        Self {
            claim,
            latency: Histogram::new(),
            // Wired at M2 alongside the performance counters. Until then the
            // claim reports the gap rather than pretending it does not exist.
            instructions_per_op: Metric::Unavailable("PMU not wired until M2"),
            joules_per_op: Metric::Unavailable("energy counters not wired until M2"),
            // Not a wiring gap: there is no idle state to be resident in until
            // the kernel stops spinning, and it does not stop until RFC 0006's
            // policy is implemented at E5-B07.
            idle_residency: Metric::Unavailable("nothing idles yet; RFC 0006, E5-B07"),
        }
    }

    /// Print the result in the form the claims registry ingests.
    pub fn report(&self) {
        println!("claim   {}", self.claim);
        println!("latency {}", self.latency);
        println!("insn/op {}", self.instructions_per_op);
        println!("J/op    {}", self.joules_per_op);
        println!("idle    {}", self.idle_residency);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_are_monotonic() {
        let mut h = Histogram::new();
        for i in 1..=10_000u64 {
            h.record(i);
        }
        let p50 = h.quantile(0.50);
        let p99 = h.quantile(0.99);
        let p999 = h.quantile(0.999);
        assert!(p50 <= p99, "p50 {p50} must not exceed p99 {p99}");
        assert!(p99 <= p999, "p99 {p99} must not exceed p99.9 {p999}");
        assert!(p999 <= h.max());
    }

    #[test]
    fn a_tail_is_not_hidden() {
        // The failure this crate exists to prevent: a handful of very slow
        // observations among many fast ones must move p99.9 and must not be
        // averaged away.
        // Eleven slow observations in ten thousand, not ten. Ten would be
        // exactly 0.1%, which puts p99.9 precisely on the boundary: the
        // nearest-rank quantile is the 9990th value, the 9990th value is the
        // last fast one, and the test would fail while the histogram was
        // right. The property being checked is that the tail survives, so the
        // dataset has to sit inside the tail rather than on its edge.
        let mut h = Histogram::new();
        for _ in 0..9_989 {
            h.record(10);
        }
        for _ in 0..11 {
            h.record(1_000_000);
        }
        assert!(h.quantile(0.50) < 100, "the common case stays fast");
        assert!(
            h.quantile(0.999) > 1000,
            "the tail must survive into p99.9, got {}",
            h.quantile(0.999)
        );
    }

    #[test]
    fn empty_histogram_does_not_panic() {
        let h = Histogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.quantile(0.99), 0);
        assert_eq!(h.min(), 0);
    }

    #[test]
    fn buckets_round_trip_within_resolution() {
        for v in [1u64, 7, 64, 1000, 50_000, 1 << 30] {
            let i = Histogram::index(v);
            let lo = Histogram::value_at(i);
            let hi = Histogram::upper_at(i);
            assert!(lo <= v, "bucket lower bound {lo} must not exceed {v}");
            assert!(hi >= v, "bucket upper bound {hi} must not fall below {v}");
            // Two significant figures per octave: never more than ~12% low.
            assert!(v - lo <= v / 8 + 1, "resolution too coarse for {v}, got {lo}");
        }
    }

    #[test]
    fn a_percentile_is_never_optimistic() {
        // The property the upper bound exists for, checked against the exact
        // answer rather than against another approximation: a reported
        // percentile must be at least the true one. Under-reporting a tail is
        // the failure this crate exists to prevent, and reporting the low edge
        // of a bucket does it silently, on every number, in the flattering
        // direction.
        let mut values: Vec<u64> = (1..=10_000u64).map(|i| (i * 37) % 9_973).collect();
        let mut h = Histogram::new();
        for &v in &values {
            h.record(v);
        }
        values.sort_unstable();

        for q in [0.5f64, 0.9, 0.99, 0.999, 1.0] {
            let rank = (q * values.len() as f64).ceil() as usize;
            let exact = values[rank.clamp(1, values.len()) - 1];
            let reported = h.quantile(q);
            assert!(
                reported >= exact,
                "p{q} reported {reported}, true value {exact} — a percentile \
                 must never be optimistic"
            );
            // And not conservative past the point of being useless: one
            // bucket, never more.
            assert!(
                reported <= exact + exact / 8 + 1,
                "p{q} reported {reported} against a true {exact} — that is \
                 more than one bucket of pessimism"
            );
        }
    }

    #[test]
    fn the_hundredth_percentile_is_the_maximum() {
        // Not a tautology: with the bucket's lower bound it was not true, and
        // a p100 below the largest observation is the clearest possible signal
        // that a summary is losing the tail.
        let mut h = Histogram::new();
        for v in [3u64, 91, 1_004, 7, 65_537] {
            h.record(v);
        }
        assert_eq!(h.quantile(1.0), 65_537);
        assert_eq!(h.quantile(1.0), h.max());
    }
}
