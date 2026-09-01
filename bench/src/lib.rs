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
use std::path::{Path, PathBuf};

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

    /// The distribution, drawn.
    ///
    /// One row per octave rather than one per bucket. The recording resolution
    /// is two significant figures per power of two — 1024 rows, most of them
    /// empty — and a thousand-row table is not a thing anybody reads, so the
    /// rendering folds to the octave and says that it did. The full bucket list
    /// is what [`to_jsonl`](Histogram::to_jsonl) emits and what a third party
    /// re-analyses; this is for the person watching the run.
    ///
    /// Rows are the octaves that hold observations, in order, with no gaps
    /// elided: an empty octave between two full ones is a bimodal distribution,
    /// which is the shape `claims/0002-timer-jitter.toml` names in its own
    /// diagnosis section, and folding it away would hide the diagnosis.
    #[must_use]
    pub fn render(&self) -> String {
        if self.count == 0 {
            return "  (no observations)\n".to_string();
        }

        let mut rows: Vec<(usize, u64)> = Vec::new();
        for octave in 0..(BUCKET_COUNT / SUB_BUCKETS) {
            let lo = octave * SUB_BUCKETS;
            let total: u64 = self.buckets[lo..lo + SUB_BUCKETS].iter().sum();
            rows.push((octave, total));
        }

        let first = rows.iter().position(|&(_, n)| n > 0).unwrap_or(0);
        let last = rows.iter().rposition(|&(_, n)| n > 0).unwrap_or(0);
        let rows = &rows[first..=last];
        let peak = rows.iter().map(|&(_, n)| n).max().unwrap_or(1).max(1);

        const BAR: u64 = 40;
        let mut out = String::new();
        for &(octave, n) in rows {
            // An octave's own bounds, not the sub-bucket's. Below magnitude
            // four a sub-bucket index is the value's low bits rather than a
            // fraction of the octave, so `value_at` and `upper_at` both fold
            // there — and reading a row's range off them printed `4 .. 15` and
            // `8 .. 15` as two different rows. The octave is defined by its
            // magnitude and needs no reconstruction: it is exactly the values
            // whose leading bit is at that position.
            let (lo, hi) = if octave == 0 {
                (0u64, 1u64)
            } else {
                (1u64 << octave, (1u64 << (octave + 1)) - 1)
            };
            // Saturating at one column, so an octave holding a single
            // observation is visibly present rather than rounding to nothing.
            // The tail is the part of this picture that matters, and the tail
            // is made of small counts.
            let width = if n == 0 { 0 } else { ((n * BAR) / peak).max(1) };
            out.push_str(&format!(
                "  {lo:>12} ..{hi:>13} ns  {n:>9}  {}\n",
                "#".repeat(width as usize)
            ));
        }
        out
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

/// Environments in which a timing number is worth recording.
///
/// An allow-list rather than a deny-list, and the direction is the whole point.
/// A deny-list records by default, so every environment nobody thought of — a
/// new CI runner, a colleague's laptop, a virtual machine on a shared host —
/// produces a publishable-looking number until somebody notices. This list
/// records nothing by default, so adding an environment is a reviewable diff
/// with a reason in it, the way `DETERMINISM_ALLOW` in `xtask` is.
///
/// The names match `runner` in `claims/*.toml`, because they are the same
/// statement: the claim says which class of machine can defend it, and this
/// says which class of machine is allowed to speak.
const MEASUREMENT_ENVIRONMENTS: &[(&str, &str)] =
    &[("runner-class-A", "pinned bare metal, thermally stable — claims/runner-class-A.md")];

/// Whether this machine may record a timing measurement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Environment {
    /// A machine whose numbers a claim can rest on.
    Measurement {
        /// The `F_ENVIRONMENT` value, which is also a `runner` class in `claims/`.
        name: String,
        /// Why this class is defensible.
        why: &'static str,
    },
    /// Anything else. The workload still runs; the number is not recorded.
    Refused {
        /// What `F_ENVIRONMENT` said, or `"unset"`.
        name: String,
        /// What is wrong with measuring here, in a sentence a reader can check.
        why: &'static str,
    },
}

/// Why an unset variable refuses.
const WHY_UNSET: &str = "an environment that has not declared itself is not a measurement \
                         environment — set F_ENVIRONMENT";

/// Why the development container refuses.
const WHY_CONTAINER: &str = "QEMU under TCG emulates the timer against a host clock it does not control, and the \
     container shares its cores, cache and memory bandwidth with whatever else the machine \
     is doing — docker/README.md";

/// Why a shared cloud runner refuses.
const WHY_CI: &str = "a shared cloud instance cannot produce defensible tail latency — \
                      claims/0001-ring-submit-latency.toml";

/// Why anything unrecognised refuses.
const WHY_UNKNOWN: &str = "not in MEASUREMENT_ENVIRONMENTS in bench/src/lib.rs; adding it is a \
                           reviewable diff with a reason in it";

impl Environment {
    /// Read `F_ENVIRONMENT` and classify it.
    #[must_use]
    pub fn detect() -> Self {
        Self::classify(std::env::var("F_ENVIRONMENT").ok().as_deref())
    }

    /// The decision, separated from the environment it reads.
    ///
    /// Pure so that it can be tested. Setting a process environment variable
    /// from a test is `unsafe` in this edition and races every other test in
    /// the binary, so the choice is between a pure function and an untested
    /// policy — and this policy's whole job is to be right about a case nobody
    /// will exercise by hand.
    #[must_use]
    pub fn classify(value: Option<&str>) -> Self {
        let Some(name) = value.map(str::trim).filter(|v| !v.is_empty()) else {
            // Fail closed, and this is the case the rule exists for. An unset
            // variable is not evidence of bare metal; it is the state of every
            // machine that has never been told what it is, which includes every
            // new CI runner and every laptop. Recording by default here is
            // exactly how a number with no environment attached reaches a
            // document.
            return Self::Refused { name: "unset".to_string(), why: WHY_UNSET };
        };

        if let Some((matched, why)) = MEASUREMENT_ENVIRONMENTS.iter().find(|(n, _)| *n == name) {
            return Self::Measurement { name: (*matched).to_string(), why };
        }

        let why = match name {
            "container" => WHY_CONTAINER,
            "ci" => WHY_CI,
            _ => WHY_UNKNOWN,
        };
        Self::Refused { name: name.to_string(), why }
    }

    /// Whether a number taken here may be recorded.
    #[must_use]
    pub fn records(&self) -> bool {
        matches!(self, Self::Measurement { .. })
    }

    /// The name this environment reported.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Measurement { name, .. } | Self::Refused { name, .. } => name,
        }
    }

    /// Why it records, or why it will not.
    #[must_use]
    pub fn why(&self) -> &'static str {
        match self {
            Self::Measurement { why, .. } | Self::Refused { why, .. } => why,
        }
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

impl Metric {
    /// The metric as one JSON value.
    ///
    /// `null` for an unavailable metric rather than a zero or an omitted key.
    /// A zero is a measurement and this is not one; an omitted key makes an
    /// absent metric indistinguishable from a reader that forgot to look, which
    /// is the silent narrowing `Metric` exists to prevent. The reason is not
    /// carried here — it is a static string aimed at a person, it is printed
    /// beside the number, and putting prose in a data file invites somebody to
    /// parse it.
    #[must_use]
    pub fn to_json(self) -> String {
        match self {
            Self::Value(v) => format!("{v:.6}"),
            Self::Unavailable(_) => "null".to_string(),
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
    /// Whether this machine is permitted to record what it just measured.
    ///
    /// Read once, when the run begins, rather than when it ends: a check
    /// performed after the work is a check somebody can be tempted to skip
    /// having seen the number.
    pub environment: Environment,
}

impl Sample {
    /// Begin a claim run.
    #[must_use]
    pub fn new(claim: &'static str) -> Self {
        Self {
            claim,
            latency: Histogram::new(),
            // Both are secondary metrics of claim 0001, so the gap is reported
            // per run rather than hidden. The reasons name the tasks that end
            // them, not a milestone: the previous strings said "until M2", and
            // outlived M2 by three milestones before anybody noticed
            // (docs/TESTING-STATUS.md).
            instructions_per_op: Metric::Unavailable("PMU not wired; E0-P05's machine has one"),
            joules_per_op: Metric::Unavailable("no energy counter; external meter at E5-P03"),
            // Not a wiring gap: there is no idle state to be resident in until
            // the kernel stops spinning, and it does not stop until RFC 0006's
            // policy is implemented at E5-B07.
            idle_residency: Metric::Unavailable("nothing idles yet; RFC 0006, E5-B07"),
            environment: Environment::detect(),
        }
    }

    /// Print the result in the form the claims registry ingests.
    ///
    /// The distribution is drawn before the percentiles, and that order is the
    /// point. A percentile line is a summary of the shape above it, and a
    /// reader who sees only the summary cannot tell a long tail from a second
    /// mode — which is the difference between "sometimes slow" and "two
    /// different code paths", and the two have nothing in common as
    /// diagnoses. `claims/README.md` rule 3 says distributions, not summaries;
    /// printing only p50/p99/p99.9 was that rule stated and not kept.
    pub fn report(&self) {
        println!("claim   {}", self.claim);
        println!("machine {}", self.environment.name());
        println!();
        print!("{}", self.latency.render());
        println!();

        if self.environment.records() {
            println!("latency {}", self.latency);
        } else {
            // The drawing above still prints, and that is deliberate. It is how
            // anybody debugs a workload, and refusing to draw it would push
            // people to a second harness that does. What is refused is the
            // *summary* — the one line that gets copied into a document,
            // quoted in a review, or pasted into a chat with the environment
            // left behind. A distribution nobody can quote in a sentence is not
            // the failure mode this rule exists for.
            println!(
                "latency refused — {} is not a measurement environment",
                self.environment.name()
            );
            println!("        {}", self.environment.why());
        }

        println!("insn/op {}", self.instructions_per_op);
        println!("J/op    {}", self.joules_per_op);
        println!("idle    {}", self.idle_residency);
    }

    /// Write the full distribution where the registry can find it.
    ///
    /// One JSON object per line: a header carrying the run's summary and the
    /// availability of every metric, then one object per non-empty bucket. The
    /// header is first so a reader that only wants to know whether a run
    /// happened does not have to consume the distribution to find out, and the
    /// buckets are the part that cannot be reconstructed later.
    ///
    /// The file is `<claim>.local.jsonl`, which `.gitignore` excludes. That is
    /// deliberate and it is a boundary rather than an oversight: a measurement
    /// belongs to the machine that took it. What is versioned is the *claim* —
    /// its threshold, its baseline, its workload — and, once E0-P11 exists, the
    /// history that CI appends to. A raw distribution from somebody's laptop
    /// committed alongside them would be a number with no environment attached,
    /// which is the thing `F_ENVIRONMENT` exists to make impossible.
    ///
    /// # Errors
    ///
    /// If the directory cannot be created or the file cannot be written.
    pub fn persist(&self, dir: &Path) -> std::io::Result<PathBuf> {
        if !self.environment.records() {
            // `Other` rather than a bool return or a silent no-op: a caller
            // that ignores this gets nothing written and no file to point at,
            // which is the same outcome, and a caller that reports it gets a
            // sentence naming the machine. A silent no-op would leave a stale
            // file from an earlier run looking like this one's result.
            return Err(std::io::Error::other(format!(
                "{} is not a measurement environment: {}",
                self.environment.name(),
                self.environment.why()
            )));
        }
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.local.jsonl", self.claim));

        let mut out = String::new();
        out.push_str(&format!(
            "{{\"claim\":\"{}\",\"kind\":\"run\",\"n\":{},\"min\":{},\"p50\":{},\
             \"p99\":{},\"p999\":{},\"max\":{},\"instructions_per_op\":{},\
             \"joules_per_op\":{},\"idle_residency\":{}}}\n",
            self.claim,
            self.latency.count(),
            self.latency.min(),
            self.latency.quantile(0.50),
            self.latency.quantile(0.99),
            self.latency.quantile(0.999),
            self.latency.max(),
            self.instructions_per_op.to_json(),
            self.joules_per_op.to_json(),
            self.idle_residency.to_json(),
        ));
        out.push_str(&self.latency.to_jsonl(self.claim));

        std::fs::write(&path, out)?;
        Ok(path)
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
    fn a_drawn_octave_states_its_own_range() {
        // The bug this checks for printed `4 .. 15` and `8 .. 15` as two
        // different rows, because it reconstructed the range from sub-bucket
        // bounds — and below magnitude four a sub-bucket index is the value's
        // low bits rather than a fraction of the octave, so both folded to the
        // same number. Two rows claiming the same upper bound is a table that
        // cannot be read, and it is the only kind of error in a drawing that a
        // reader has no way to detect.
        let mut h = Histogram::new();
        for v in [3u64, 5, 9, 300] {
            h.record(v);
        }
        let drawn = h.render();

        let mut seen: Vec<(u64, u64)> = Vec::new();
        for line in drawn.lines() {
            let cells: Vec<&str> = line.split_whitespace().collect();
            // "<lo> .. <hi> ns <count> [bar]"
            let (Some(lo), Some(hi)) = (cells.first(), cells.get(2)) else { continue };
            let (Ok(lo), Ok(hi)) = (lo.parse::<u64>(), hi.parse::<u64>()) else { continue };
            assert!(lo <= hi, "row {lo}..{hi} is inverted");
            seen.push((lo, hi));
        }
        assert!(seen.len() >= 4, "expected a row per occupied octave, got {seen:?}");

        for pair in seen.windows(2) {
            let [(_, prev_hi), (next_lo, _)] = pair else { continue };
            assert_eq!(
                *next_lo,
                prev_hi + 1,
                "octaves must tile without gap or overlap, got {seen:?}"
            );
        }
    }

    #[test]
    fn every_observation_survives_into_the_drawing() {
        // A drawing that loses observations is worse than no drawing: it looks
        // like the distribution and is not it. The bar widths are a rendering
        // choice, the counts are not.
        let mut h = Histogram::new();
        for i in 1..=5_000u64 {
            h.record(i * 7);
        }
        let counted: u64 = h
            .render()
            .lines()
            .filter_map(|line| line.split_whitespace().nth(4))
            .filter_map(|cell| cell.parse::<u64>().ok())
            .sum();
        assert_eq!(counted, h.count(), "the drawing must account for every observation");
    }

    #[test]
    fn a_single_observation_is_still_drawn() {
        // The tail is made of small counts, and a bar that rounds to zero
        // columns is a tail that is present in the data and absent from the
        // picture — which is the failure this whole crate is about.
        let mut h = Histogram::new();
        for _ in 0..100_000 {
            h.record(10);
        }
        h.record(1_000_000);
        let drawn = h.render();
        let last = drawn.lines().next_back().unwrap_or_default();
        assert!(last.contains('#'), "the one slow observation drew no bar: {last}");
    }

    #[test]
    fn an_empty_histogram_draws_nothing_and_does_not_panic() {
        assert_eq!(Histogram::new().render().trim(), "(no observations)");
    }

    #[test]
    fn an_undeclared_environment_refuses() {
        // The case the allow-list exists for, and the one nobody exercises by
        // hand: a machine that has never been told what it is. Every new CI
        // runner and every fresh laptop starts here, so a deny-list would have
        // recorded on all of them.
        let e = Environment::classify(None);
        assert!(!e.records());
        assert_eq!(e.name(), "unset");

        // An empty or whitespace value is the same state wearing a value. A
        // CI expression that resolves to nothing is the ordinary way this
        // happens, and it must not read as a declaration.
        assert!(!Environment::classify(Some("")).records());
        assert!(!Environment::classify(Some("   ")).records());
        assert_eq!(Environment::classify(Some("")).name(), "unset");
    }

    #[test]
    fn the_development_container_refuses_and_says_why() {
        let e = Environment::classify(Some("container"));
        assert!(!e.records());
        assert_eq!(e.name(), "container");
        assert!(
            e.why().contains("docker/README.md"),
            "a refusal must point at the document that argues it, got: {}",
            e.why()
        );
    }

    #[test]
    fn an_unrecognised_environment_refuses_rather_than_records() {
        // The direction of the list. Something nobody has classified is not
        // thereby bare metal, and it must not be treated as such just because
        // this file has never heard of it.
        for name in ["laptop", "runner-class-B", "gitlab", "wsl2"] {
            let e = Environment::classify(Some(name));
            assert!(!e.records(), "{name} recorded, and nothing says it may");
            assert_eq!(e.name(), name);
        }
    }

    #[test]
    fn a_declared_measurement_environment_records() {
        // The other half, and the reason this is not simply "refuse always":
        // the rule has to let the real runner through, and the name it lets
        // through is the same `runner` class the claims name.
        let e = Environment::classify(Some("runner-class-A"));
        assert!(e.records());
        assert_eq!(e.name(), "runner-class-A");
    }

    #[test]
    fn a_refused_environment_writes_no_distribution() {
        // The refusal has to reach the artefact and not only the terminal. A
        // harness that prints a refusal and writes the file anyway has left a
        // number on disk for something else to pick up.
        let mut sample = Sample::new("test-claim");
        sample.environment = Environment::classify(Some("container"));
        sample.latency.record(42);

        let dir = std::env::temp_dir().join("f-bench-refusal-test");
        let err = sample.persist(&dir).expect_err("a refused environment must not write");
        assert!(
            err.to_string().contains("container"),
            "the error must name the machine, got: {err}"
        );
        assert!(
            !dir.join("test-claim.local.jsonl").exists(),
            "a refused run must leave no file behind"
        );
    }

    #[test]
    fn an_absent_metric_is_null_rather_than_zero() {
        // Zero is a measurement. An absent metric that serialises as zero is a
        // claim nobody made, and it is the exact silent narrowing `Metric`
        // exists to prevent.
        assert_eq!(Metric::Unavailable("no counters").to_json(), "null");
        assert_eq!(Metric::Value(1.5).to_json(), "1.500000");
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
