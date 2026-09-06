// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Claim 0017's workload: bytes re-chunked and re-hashed per application byte
//! written, on both object kinds, under the random-write workload `E2-D02`
//! names.
//!
//! # What this file supplies that `blob/tests/chunker.rs` cannot
//!
//! The **denominator**. That test measures the numerator — the distance from an
//! edit to the first boundary the two streams agree on again — against a `Vec`
//! in a test, and claim 0017 has said since it was registered that this is the
//! honest description of it. An *application byte* is defined once, in
//! `intent/0006-state/spec.md`, as a byte the client submitted, so a ratio needs
//! a client and a write path. This is that: a 4 KiB write at a drawn offset,
//! against a chunked object and against an extent, with the cost of each
//! recorded per edit.
//!
//! # The two kinds, measured on one workload rather than two
//!
//! The offsets and the payloads are drawn once per (seed, mixture, size) and
//! handed to both kinds. A run that drew separately would be two experiments
//! sharing a claim, and the only interesting comparison here — the same edit
//! costing 786 432 bytes on one kind and 1 048 576 on the other, and the second
//! number not moving when the object grows sixteenfold — would be a comparison
//! of two different edit sequences.
//!
//! # Why the mixture is iterated and why there is a fifth one
//!
//! Four mixtures are `E2-P02`'s: uniform, zero-filled, periodic below
//! [`CHUNK_TARGET_BYTES`], and concatenations of those. The fifth is **periodic
//! below [`CHUNK_MIN_BYTES`]**, and RFC 0062 owes it to this task by name: a
//! period below 16 KiB is starved under *every* boundary rule this design
//! permits — the candidate set of a predicate over a bounded window of
//! `p`-periodic content is invariant under `+p`, and a minimum of `m > p`
//! accepts none of it — and 4096- and 8192-byte database pages are inside that
//! class by arithmetic. A bench that drew only periods below the *target* would
//! report a bounded number two runs in three and hide the whole class the
//! second object kind was bought for. It is drawn at a period below 16 KiB, the
//! period is printed beside the number, and the 786 432-byte threshold does not
//! apply to it: on starved content the re-chunk cost scales with the object,
//! which is the design admitting what it admits, and a threshold there would be
//! either false or vacuous.
//!
//! # Where the threshold applies, and where it cannot
//!
//! Per edit, this classifies the edit the way `E2-P02` does — by the candidate
//! gap it sits in, not by the mixture's name, because RFC 0062's theorem says
//! the name decides nothing. An edit outside a starved run is one the published
//! bound `CHUNK_MAX_BYTES + RESYNC_BOUND_BYTES` = 786 432 must hold on, and the
//! primary rows are the maximum over exactly those. An edit inside a starved run
//! is one the bound's second clause excuses, and its cost is recorded per
//! mixture with no threshold over it. The count of each is printed, because a
//! primary taken over no edits at all is a green row asserting nothing — the
//! same vacuity `claims/0017`'s `resync_pairs_unstarved` exists to make visible.
//!
//! # No clock, and therefore no `Sample`
//!
//! Every number here is a count of bytes, and a count is the same on a fast host
//! and a slow one. `f_bench::Sample` is the harness for a *distribution of
//! times* and would print an empty histogram and a refusal beside these, which
//! would read as this workload having failed to measure something rather than as
//! it having measured something else. What is borrowed is
//! [`f_bench::Environment`], printed beside the counts so that a reader knows
//! which machine produced them and so that the claim may gate in the container
//! the way `claims/0005` and `claims/0014` do.

use std::env;

use f_bench::Environment;
use f_blob::chunk::{
    CHUNK_MAX_BYTES, CHUNK_MIN_BYTES, CHUNK_TARGET_BYTES, Chunker, RESYNC_BOUND_BYTES,
};
use f_blob::device::Memory;
use f_blob::extent::{EXTENT_BYTES, Extent};
use f_blob::gear::{GEAR, MASK};
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{Stream, label};
use f_hash::sha256;

/// The small object. Unit: bytes.
///
/// Eight mebibytes, and it is a floor rather than a taste: claim 0017's
/// `object_bytes_small` is `min = 8388608`, so a run that shrank it would report
/// a bound it did not test. It has to be large enough that the published
/// allowance — 786 432 bytes — is a small fraction of it, or "the sequences
/// agreed" would sometimes mean "the object ran out".
const OBJECT_BYTES_SMALL: usize = 8 * 1024 * 1024;

/// The large object. Unit: bytes.
///
/// Sixteen times the small one, which is what makes the pair of numbers a
/// *bound* rather than a description: a cost that scales with the object shows
/// up as a sixteenfold difference, and RFC 0058's whole promise for the extent
/// kind is that the two rows are equal.
const OBJECT_BYTES_LARGE: usize = 128 * 1024 * 1024;

/// One write. Unit: bytes.
///
/// The database page, and the size the skeptic's number is quoted at: 4096
/// application bytes costing 1 048 576 copied is the 256x RFC 0058 published
/// before anything was built. Fixed rather than drawn — claim 0017 pins it at
/// `min = max = 4096` — because a drawn write size would average the 256x
/// against writes large enough to fill a piece, which is the arithmetic this
/// claim exists to refuse.
const WRITE_BYTES: usize = 4096;

/// Writes per object. Unit: count of writes.
///
/// Sixty-four, claim 0017's `writes_per_object` floor. Enough that the
/// straddling case — `4095 / 1048576` of uniformly drawn offsets — is a
/// possibility rather than a certainty, which is why the straddling row below
/// is measured at a placed boundary as well as counted over the draw.
const WRITES_PER_OBJECT: usize = 64;

/// Writes between snapshots on the extent path. Unit: count of writes.
///
/// Recorded beside the snapshot row rather than folded into the per-write
/// number, because RFC 0058 is explicit that folding it in would make the
/// per-write number a function of how often the bench snapshots — a knob, not a
/// property.
const SNAPSHOT_INTERVAL_WRITES: usize = 16;

/// The window the register sees. Unit: bytes.
///
/// Bit 63 is the highest bit the mask tests and bit *k* depends on the last
/// *k+1* bytes, so a boundary decision reads the last sixty-four bytes and
/// nothing earlier.
const WINDOW_BYTES: usize = 64;

/// The candidate spacing at or above which a cut can be forced. Unit: bytes.
///
/// RFC 0061's definition of a starved run, copied rather than reinvented:
/// `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` = 240 KiB is exactly the condition under
/// which the forced cut is reachable at all.
const STARVED_GAP_BYTES: usize = CHUNK_MAX_BYTES - CHUNK_MIN_BYTES;

/// The seeds this workload is taken over, unless an argument says otherwise.
///
/// Recorded rather than drawn, for `blob/tests/chunker.rs`'s reason: a seed set
/// that moves is a seed set nobody can be asked to reproduce.
const SEEDS: &[u64] = &[1, 2];

/// The five content mixtures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mixture {
    Uniform,
    Zero,
    PeriodicBelowTarget,
    PeriodicBelowMinimum,
    Concatenated,
}

impl Mixture {
    const ALL: [Self; 5] = [
        Self::Uniform,
        Self::Zero,
        Self::PeriodicBelowTarget,
        Self::PeriodicBelowMinimum,
        Self::Concatenated,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Zero => "zero-filled",
            Self::PeriodicBelowTarget => "periodic<target",
            Self::PeriodicBelowMinimum => "periodic<minimum",
            Self::Concatenated => "concatenated",
        }
    }
}

/// The four draw sites, split by identity from one seed.
///
/// The identities are `intent/0006-state/plan.md`'s, named there so that this
/// file could not quietly draw from somewhere else: `rechunk/object`,
/// `rechunk/mixture`, `rechunk/offset`, `rechunk/bytes`. Split by identity and
/// not by draw order, RFC 0026, so that a fifth site added later leaves the four
/// here answering what they answered before.
struct Sites {
    object: Stream,
    mixture: Stream,
    offset: Stream,
    bytes: Stream,
}

impl Sites {
    fn new(seed: u64) -> Self {
        let root = Stream::from_seed(label("e2-b09")).split(seed);
        Self {
            object: root.split(label("rechunk/object")),
            mixture: root.split(label("rechunk/mixture")),
            offset: root.split(label("rechunk/offset")),
            bytes: root.split(label("rechunk/bytes")),
        }
    }
}

/// A value in `0..n`, by remainder.
fn below(stream: &mut Stream, n: usize) -> usize {
    assert!(n > 0, "a draw below zero has no answer");
    (stream.next_u64() % n as u64) as usize
}

fn uniform_bytes(stream: &mut Stream, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        out.extend_from_slice(&stream.next_u64().to_le_bytes());
    }
    out.truncate(len);
    out
}

/// Drawn content, and the period it repeats at when it has one.
///
/// The period leaves the generator for RFC 0062's reason: the class the extent
/// kind exists for is named by `p` against [`CHUNK_MIN_BYTES`], and a generator
/// that kept `p` to itself would leave the printed number unreadable — a bounded
/// cost on a periodic object means one thing at `p` = 48 KiB and another at
/// `p` = 4 KiB.
struct Drawn {
    bytes: Vec<u8>,
    period: Option<usize>,
}

/// `len` bytes of the named mixture.
fn draw(kind: Mixture, content: &mut Stream, mixture: &mut Stream, len: usize) -> Drawn {
    match kind {
        Mixture::Uniform => Drawn { bytes: uniform_bytes(content, len), period: None },
        Mixture::Zero => Drawn { bytes: vec![0u8; len], period: None },
        Mixture::PeriodicBelowTarget => {
            let period = WINDOW_BYTES + below(content, CHUNK_TARGET_BYTES - WINDOW_BYTES);
            Drawn { bytes: repeated(content, period, len), period: Some(period) }
        }
        Mixture::PeriodicBelowMinimum => {
            // Below the minimum, which is the class RFC 0062's theorem covers:
            // no rule of this shape puts a content boundary in it, so the cost
            // of an edit scales with the object and the second object kind is
            // the only answer. The two workloads RFC 0058 names live at 4096
            // and 8192; the draw covers the whole class rather than those two
            // points, so that a run cannot be read as a statement about one
            // page size.
            let period = WINDOW_BYTES + below(content, CHUNK_MIN_BYTES - WINDOW_BYTES);
            Drawn { bytes: repeated(content, period, len), period: Some(period) }
        }
        Mixture::Concatenated => {
            let mut out = Vec::with_capacity(len);
            while out.len() < len {
                let run = (CHUNK_TARGET_BYTES + below(mixture, 8 * CHUNK_TARGET_BYTES))
                    .min(len - out.len());
                let inner = Mixture::ALL[below(mixture, 4)];
                out.extend_from_slice(&draw(inner, content, mixture, run).bytes);
            }
            Drawn { bytes: out, period: None }
        }
    }
}

/// `len` bytes of a drawn block repeated at `period`.
fn repeated(content: &mut Stream, period: usize, len: usize) -> Vec<u8> {
    let block = uniform_bytes(content, period);
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let take = (len - out.len()).min(period);
        out.extend_from_slice(&block[..take]);
    }
    out
}

/// Every boundary the chunker puts in `bytes`, ending with the object's end.
fn boundaries(bytes: &[u8]) -> Vec<usize> {
    let mut chunker = Chunker::new();
    let mut out: Vec<usize> = chunker.feed(bytes).collect();
    let tail = chunker.finish();
    if tail > 0 || out.is_empty() {
        out.push(bytes.len());
    }
    out
}

/// Every position at which the register hits the mask.
///
/// A candidate is a property of the content and of the two constants and of
/// nothing the chunker is doing at the time, because the register is never reset
/// at a cut. That is what lets the starved-run classification below be stated
/// over the object rather than over a run of the algorithm.
fn candidates(bytes: &[u8]) -> Vec<usize> {
    let mut register: u64 = 0;
    let mut out = Vec::new();
    for (index, byte) in bytes.iter().enumerate() {
        register = (register << 1).wrapping_add(GEAR[*byte as usize]);
        if register & MASK == 0 {
            out.push(index + 1);
        }
    }
    out
}

/// The candidate sequence of the edited object, from the base object's.
///
/// A candidate at `p` is decided by the sixty-four bytes ending at `p`, so an
/// overwrite of `len` bytes at `at` can add or remove candidates only in
/// `(at, at + len + WINDOW_BYTES]`. Everything else is the base object's answer
/// and is unchanged. This is not an approximation and the region is not a
/// margin: it is the register's window, the same fact the prefix half of the
/// bound rests on. Recomputing the whole sequence per edit would be a second
/// full scan of the object beside the chunker's, for a result identical
/// everywhere — and at 128 MiB times sixty-four edits that is the difference
/// between a claim somebody runs and one they do not.
///
/// The local register is warmed from `at − WINDOW_BYTES`, which is exactly
/// enough: bit `k` depends on the last `k+1` bytes, so after sixty-four bytes
/// every bit the mask tests is the value the whole-object scan would have had.
fn candidates_after(base: &[usize], edited: &[u8], at: usize, len: usize) -> Vec<usize> {
    let dirty_to = (at + len + WINDOW_BYTES).min(edited.len());
    let warm = at.saturating_sub(WINDOW_BYTES);
    let mut register: u64 = 0;
    let mut local = Vec::new();
    for (offset, byte) in edited[warm..dirty_to].iter().enumerate() {
        register = (register << 1).wrapping_add(GEAR[*byte as usize]);
        let position = warm + offset + 1;
        if position > at && register & MASK == 0 {
            local.push(position);
        }
    }
    let mut out = Vec::with_capacity(base.len() + local.len());
    out.extend(base.iter().copied().filter(|&c| c <= at));
    out.extend(local);
    out.extend(base.iter().copied().filter(|&c| c > dirty_to));
    out
}

/// The end of the maximal starved run containing `at`.
///
/// Copied from `blob/tests/chunker.rs` with its semantics intact: the walk
/// starts behind `at`, because RFC 0061's word is *maximal* and a maximal run is
/// a property of the candidate sequence rather than of where the edit landed in
/// it. An answer at or before `at` means there is no starved run there.
fn starved_end(candidates: &[usize], length: usize, at: usize) -> usize {
    let mut previous = candidates.iter().copied().take_while(|&c| c <= at).last().unwrap_or(0);
    for &candidate in candidates.iter().filter(|&&c| c > at) {
        if candidate - previous < STARVED_GAP_BYTES {
            return previous;
        }
        previous = candidate;
    }
    length
}

/// The widest gap between consecutive candidates in `[from, to]`.
///
/// Unit: bytes. The diagnosis beside a violated bound: a gap at or above
/// [`STARVED_GAP_BYTES`] inside the region the two streams had to cross is a
/// starved run they entered *after* the edit, which [`starved_end`] evaluated at
/// the edit cannot see. A violation with no such gap is a different failure and
/// a worse one — it says the acceptance rule is reading something outside its
/// window.
fn widest_gap(candidates: &[usize], from: usize, to: usize) -> usize {
    let mut previous = from;
    let mut widest = 0;
    for &candidate in candidates.iter().filter(|&&c| c > from && c <= to) {
        widest = widest.max(candidate - previous);
        previous = candidate;
    }
    widest.max(to.saturating_sub(previous))
}

/// The earliest position from which two boundary sequences agree again.
///
/// The longest common suffix, and it always finds something: both sequences end
/// at the object's end, which is the worst case and means nothing after the edit
/// was preserved. `shift` is zero here and the parameter is gone with it — an
/// overwrite displaces nothing, which is the one way this workload differs from
/// `E2-P02`'s insertion and is why the same helper is written out rather than
/// shared.
fn resynchronised_at(old: &[usize], new: &[usize]) -> usize {
    let mut i = old.len();
    let mut j = new.len();
    let mut at = new.last().copied().unwrap_or(0);
    while i > 0 && j > 0 && old[i - 1] == new[j - 1] {
        at = new[j - 1];
        i -= 1;
        j -= 1;
    }
    at
}

/// One edit: where it went and what it put there.
///
/// Drawn once per (seed, mixture, size) and handed to both kinds, so that the
/// two numbers are about one workload.
struct Edit {
    at: usize,
    bytes: Vec<u8>,
}

/// The running maximum and total of one recorded quantity.
///
/// A maximum *and* a total, because the threshold is a maximum and the ratio
/// claim 0017 publishes is a total over a total. A mean of maxima would be
/// neither.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    edits: u64,
    max: u64,
    total: u64,
}

impl Tally {
    fn record(&mut self, value: u64) {
        self.edits += 1;
        self.max = self.max.max(value);
        self.total += value;
    }

    /// Bytes per application byte written.
    fn per_app_byte(&self, app_bytes: u64) -> f64 {
        if app_bytes == 0 { 0.0 } else { self.total as f64 / app_bytes as f64 }
    }
}

/// What one (seed, mixture, size) cell measured on the chunked kind.
#[derive(Clone, Copy, Debug, Default)]
struct Chunked {
    /// Edits whose position is not inside a starved run: the ones the published
    /// bound must hold on.
    unstarved: Tally,
    /// Edits inside a starved run: the ones its second clause excuses.
    starved: Tally,
    /// Bytes fed to `f-hash`, split the same way. Split and not aggregated,
    /// because the re-hash rows carry the same threshold as the re-chunk rows
    /// and a maximum taken across both classes would report a starved object's
    /// number under a threshold that cannot apply to it.
    hashed_unstarved: Tally,
    hashed_starved: Tally,
    /// Unstarved edits that exceeded the published bound.
    ///
    /// Counted rather than only maximised, because *how many* is the difference
    /// between a bound that is wrong and a bound that is wrong on a class
    /// somebody can name. Each one prints its own line as it happens.
    violations: Tally,
    /// The subset of the unstarved edits whose resynchronisation crossed no
    /// starved run.
    ///
    /// **This is a diagnosis and not a second primary, and the distinction is
    /// the whole reason it is here.** The published bound is stated over edits
    /// that are not *inside* a starved run, and `unstarved` above is exactly
    /// that set; this narrower set is the one whose two streams never had to
    /// cross a 240 KiB candidate gap on their way back to agreement. When the
    /// two numbers differ, the difference names the cause — a starved run
    /// entered *after* the edit, which `starved_end` evaluated at the edit
    /// cannot see — and it is recorded so that a reader can weigh the
    /// falsification rather than take a maximum on trust. It is not offered as
    /// a replacement threshold: a threshold taken over the edits that passed is
    /// the fitted number `claims/0017`'s `[diagnosis]` tells the next reader not
    /// to write.
    unstarved_crossing_nothing: Tally,
}

/// What one (seed, mixture, size) cell measured on the extent kind.
#[derive(Clone, Copy, Debug, Default)]
struct Extents {
    copied: Tally,
    hashed: Tally,
    pieces: Tally,
    /// The straddling subset, recorded separately because RFC 0058 requires it
    /// to be visible rather than averaged into the aligned case.
    straddling: Tally,
    /// How many of the writes above were placed at a piece boundary rather than
    /// drawn. Unit: count of writes. Printed so that a reader can subtract them
    /// from the drawn workload rather than take on trust that the straddling
    /// observations were not arranged — some of them were, and this says how
    /// many.
    placed: u64,
    /// The extent record's own rewrite, which is a snapshot cost and not a
    /// per-write one.
    snapshots: Tally,
}

fn main() {
    let geometry = Geometry::from_args();
    let environment = Environment::detect();

    println!("claim   bytes-rechunked-per-byte");
    println!("machine {}", environment.name());
    println!();
    println!("Counts only. Every number below is a count of bytes the writer scanned,");
    println!("copied or hashed, and is the same count on any machine — which is why this");
    println!("workload records no time and `f_bench::Sample` is not used here.");
    println!();
    geometry.report();
    println!();

    let mut app_bytes: u64 = 0;
    let mut chunked_small = Chunked::default();
    let mut chunked_large = Chunked::default();
    let mut extent_small = Extents::default();
    let mut extent_large = Extents::default();

    // The index and not the value decides which pair of accumulators a cell
    // lands in. Comparing against `geometry.large` would put both cells in the
    // large column on a smoke run where the two sizes were given the same
    // number, and the small column would report zero — a run that measured
    // something reporting that it measured nothing.
    for (which, &size) in [geometry.small, geometry.large].iter().enumerate() {
        let large = which == 1;
        println!("--- object {size} bytes ---");
        println!(
            "mixture            seed unstarved re-chunk max   starved starved max    pieces  \
             period"
        );
        for &seed in &geometry.seeds {
            for kind in Mixture::ALL {
                let mut sites = Sites::new(seed);
                let Drawn { bytes: object, period } =
                    draw(kind, &mut sites.object, &mut sites.mixture, size);
                let edits = draw_edits(&mut sites, object.len(), geometry.writes);
                app_bytes += (edits.len() * WRITE_BYTES) as u64;

                let label = format!("{} seed {seed} at {size}", kind.name());
                let chunked = measure_chunked(&object, &edits, &label);
                let extents = measure_extent(object, &edits, geometry.snapshot_interval);

                println!(
                    "{:<17} {:>5} {:>9} {:>11} {:>9} {:>11} {:>9}  {}",
                    kind.name(),
                    seed,
                    chunked.unstarved.edits,
                    chunked.unstarved.max,
                    chunked.starved.edits,
                    chunked.starved.max,
                    extents.pieces.max,
                    period.map_or_else(|| "none".to_string(), |p| p.to_string()),
                );

                let (into_chunked, into_extent) = if large {
                    (&mut chunked_large, &mut extent_large)
                } else {
                    (&mut chunked_small, &mut extent_small)
                };
                merge_chunked(into_chunked, &chunked);
                merge_extent(into_extent, &extents);
            }
        }
        println!();
    }

    report(&geometry, app_bytes, &chunked_small, &chunked_large, &extent_small, &extent_large);
}

/// The geometry, and the arguments that may shrink it.
///
/// Defaults are claim 0017's, so the published reproduction command takes the
/// claim's own numbers and nothing else. The arguments exist for the reason
/// `blob/tests/million.rs`'s `--blobs` does — a full run is minutes of hashing
/// and an agent changing this file needs a smoke run — and every one of them is
/// printed, so a run that shrank the object cannot report a bound it did not
/// test.
struct Geometry {
    small: usize,
    large: usize,
    writes: usize,
    snapshot_interval: usize,
    seeds: Vec<u64>,
}

impl Geometry {
    fn from_args() -> Self {
        let mut geometry = Self {
            small: OBJECT_BYTES_SMALL,
            large: OBJECT_BYTES_LARGE,
            writes: WRITES_PER_OBJECT,
            snapshot_interval: SNAPSHOT_INTERVAL_WRITES,
            seeds: SEEDS.to_vec(),
        };
        let args: Vec<String> = env::args().skip(1).collect();
        let mut at = 0;
        while at + 1 < args.len() {
            let value = &args[at + 1];
            match args[at].as_str() {
                "--small" => geometry.small = value.parse().expect("--small takes bytes"),
                "--large" => geometry.large = value.parse().expect("--large takes bytes"),
                "--writes" => geometry.writes = value.parse().expect("--writes takes a count"),
                "--seeds" => {
                    let n: usize = value.parse().expect("--seeds takes a count");
                    geometry.seeds = (1..=n as u64).collect();
                }
                other => panic!("unknown argument {other}"),
            }
            at += 2;
        }
        geometry
    }

    fn report(&self) {
        println!("object_bytes_small        {}", self.small);
        println!("object_bytes_large        {}", self.large);
        println!("write_bytes               {WRITE_BYTES}");
        println!("writes_per_object         {}", self.writes);
        println!("snapshot_interval_writes  {}", self.snapshot_interval);
        println!("seeds                     {:?}", self.seeds);
        println!("mixtures                  {}", Mixture::ALL.len());
        println!("extent_bytes              {EXTENT_BYTES}");
        println!(
            "chunk_min/target/max      {CHUNK_MIN_BYTES}/{CHUNK_TARGET_BYTES}/{CHUNK_MAX_BYTES}"
        );
        println!("resync_bound_bytes        {RESYNC_BOUND_BYTES}");
    }
}

/// The edits, drawn once and used by both kinds.
///
/// Offsets are uniform over the whole object rather than over its first half,
/// because the quantity this claim publishes is the cost of an edit and there is
/// no reason an edit near the end should be excluded — `E2-P02` draws over the
/// first half so that the *allowance* has room past the edit, and that is a
/// property of an assertion this file does not make.
fn draw_edits(sites: &mut Sites, length: usize, writes: usize) -> Vec<Edit> {
    (0..writes)
        .map(|_| Edit {
            at: below(&mut sites.offset, length - WRITE_BYTES),
            bytes: uniform_bytes(&mut sites.bytes, WRITE_BYTES),
        })
        .collect()
}

/// The chunked kind: what one edit costs the writer that has to re-chunk.
///
/// Each edit is applied to the base object, measured, and undone, so that the
/// sixty-four numbers are sixty-four independent statements about the same
/// object rather than one statement about an object that drifted. That is the
/// same shape `E2-P02` measures and is what makes the two comparable.
///
/// The region a writer must rescan runs from the last boundary at or before the
/// edit — every boundary at or before the write's offset is unchanged, because a
/// decision at position `p` reads only bytes at or before `p` — to the first
/// position at which the two boundary sequences agree again and stay agreed.
/// Everything in that region is chunked and hashed; nothing outside it moves.
fn measure_chunked(object: &[u8], edits: &[Edit], label: &str) -> Chunked {
    let mut out = Chunked::default();
    let before = boundaries(object);
    let base_candidates = candidates(object);
    let mut edited = object.to_vec();

    for edit in edits {
        let saved = edited[edit.at..edit.at + edit.bytes.len()].to_vec();
        edited[edit.at..edit.at + edit.bytes.len()].copy_from_slice(&edit.bytes);

        let edit_end = edit.at + edit.bytes.len();
        let after = boundaries(&edited);
        let region_start = before.iter().copied().take_while(|&b| b <= edit.at).last().unwrap_or(0);
        // Where the writer may stop, and it is the later of two things. The
        // first is where the sequences agree again. The second is the end of the
        // chunk the edit fell in, and it is the floor rather than a rounding:
        // an edit that moved no boundary at all still costs the chunk it landed
        // in, because the chunk's bytes changed and its name is over its bytes.
        // Without it, `resynchronised_at`'s backward walk reports zero for such
        // an edit — the suffixes agree from the object's start — and zero is not
        // what a writer pays.
        let ends_the_edited_chunk =
            after.iter().copied().find(|&b| b >= edit_end).unwrap_or(edited.len());
        let agreed = resynchronised_at(&before, &after).max(ends_the_edited_chunk);
        let rechunked = agreed.saturating_sub(region_start) as u64;

        // The hashing a writer cannot avoid: every chunk in the rewritten region
        // has to be named before anything can be said about whether it is new,
        // so the bytes hashed are the bytes rescanned. They are recorded as two
        // rows anyway, for RFC 0058's reason — a future scheme that hashed less
        // than it scanned would move them apart and one row would hide it.
        let mut hashed = 0u64;
        let mut at = region_start;
        for &boundary in after.iter().filter(|&&b| b > region_start && b <= agreed) {
            let _ = sha256(&edited[at..boundary]);
            hashed += (boundary - at) as u64;
            at = boundary;
        }

        let marks = candidates_after(&base_candidates, &edited, edit.at, edit.bytes.len());
        let run_end = starved_end(&marks, edited.len(), edit_end);
        if run_end > edit_end {
            out.starved.record(rechunked);
            out.hashed_starved.record(hashed);
        } else {
            out.unstarved.record(rechunked);
            out.hashed_unstarved.record(hashed);
            let crossed = widest_gap(&marks, edit_end, agreed);
            if crossed < STARVED_GAP_BYTES {
                out.unstarved_crossing_nothing.record(rechunked);
            }
            // A violation of the published bound on content the bound applies
            // to, named where it happened rather than left as a maximum. RFC
            // 0061's `[diagnosis]` says this is the observation it most wants
            // somebody to go looking for, and a run that reported only the
            // maximum would hand the next reader a number and no way to reach
            // the edit behind it.
            //
            // What is printed beside it is the diagnosis: `E2-P02`'s own
            // allowance is `max(edit_end + RESYNC_BOUND_BYTES, starved_end +
            // CHUNK_MAX_BYTES)` evaluated at the edit, so on an unstarved edit
            // it is the flat clause — and the widest candidate gap between the
            // edit and re-agreement says whether the region the streams had to
            // cross was starved even though the region they started in was not.
            if rechunked > (CHUNK_MAX_BYTES + RESYNC_BOUND_BYTES) as u64 {
                out.violations.record(rechunked);
                let allowed = (edit_end + RESYNC_BOUND_BYTES).max(run_end + CHUNK_MAX_BYTES);
                println!(
                    "  ! {label} edit at {} re-chunked {rechunked} ({region_start} -> {agreed}); \
                     E2-P02 allowance {allowed}, starved_end {run_end}, widest candidate gap \
                     crossed {crossed}",
                    edit.at,
                );
            }
        }

        edited[edit.at..edit.at + saved.len()].copy_from_slice(&saved);
    }
    out
}

/// The extent kind: the same edits through the real write path.
///
/// A real [`Store`] over a modelled device rather than arithmetic over a `Vec`,
/// because the number this claim publishes is what the *write path* costs and a
/// model of it would be a second implementation nobody checks against the first.
/// The pieces are read back and compared against the bytes the workload expects
/// before any number is reported: a measurement of a write path that produced
/// the wrong bytes would be a measurement of nothing.
fn measure_extent(mut expected: Vec<u8>, edits: &[Edit], snapshot_interval: usize) -> Extents {
    let mut out = Extents::default();
    let mut store = a_store(expected.len(), edits.len());
    // The object is taken by value and then mutated into the bytes the workload
    // expects, so that a 128 MiB run holds one copy of it and not two. A second
    // copy is not free at this size and buys nothing: the base object has no
    // reader left once the pieces are written.
    let mut extent = Extent::create(&mut store, &expected).expect("a device sized for the object");

    let (_, first) = extent.snapshot(&mut store).expect("a record fits");
    out.snapshots.record(first.copied);

    for (index, edit) in edits.iter().enumerate() {
        let cost = extent.write(&mut store, edit.at as u64, &edit.bytes).expect("an offset inside");
        expected[edit.at..edit.at + edit.bytes.len()].copy_from_slice(&edit.bytes);

        out.copied.record(cost.copied);
        out.hashed.record(cost.hashed);
        out.pieces.record(u64::from(cost.pieces));
        if cost.pieces > 1 {
            out.straddling.record(cost.copied);
        }
        if (index + 1) % snapshot_interval == 0 {
            let (_, snapshot) = extent.snapshot(&mut store).expect("a record fits");
            out.snapshots.record(snapshot.copied);
        }
    }

    // One *placed* straddling write per cell, because a drawn one is a
    // `4095 / 1048576` = 0.39% event and sixty-four draws miss it more often
    // than not. RFC 0058 refuses an aligned-offset workload in as many words —
    // that would be fitting the measurement to the threshold — so the case is
    // **added** rather than the draw being bent towards it, and it is counted in
    // every row a drawn write is counted in, because it is a legal 4 KiB write
    // and its cost is what a legal 4 KiB write can cost. How many writes were
    // placed rather than drawn is printed, so that nobody has to guess which
    // observations were arranged.
    if expected.len() > EXTENT_BYTES {
        let at = EXTENT_BYTES as u64 - (WRITE_BYTES / 2) as u64;
        let bytes = vec![0x5Au8; WRITE_BYTES];
        let cost = extent.write(&mut store, at, &bytes).expect("an offset inside");
        expected[at as usize..at as usize + WRITE_BYTES].copy_from_slice(&bytes);
        assert_eq!(cost.pieces, 2, "a write across a piece boundary rewrites two pieces");
        out.copied.record(cost.copied);
        out.hashed.record(cost.hashed);
        out.pieces.record(u64::from(cost.pieces));
        out.straddling.record(cost.copied);
        out.placed += 1;
    }

    let (published, last) = extent.snapshot(&mut store).expect("a record fits");
    out.snapshots.record(last.copied);

    // Verify, piece by piece so that nothing here holds a second copy of a
    // 128 MiB object. `Store::get` has already checked each piece against its
    // own name; what this adds is that the pieces are the bytes the *workload*
    // wrote, which no hash inside the store can say.
    let reader = Extent::open(&mut store, &published).expect("a published extent");
    assert_eq!(reader.extent_bytes(), expected.len() as u64);
    let mut window = vec![0u8; EXTENT_BYTES];
    let mut at = 0usize;
    while at < expected.len() {
        let take = EXTENT_BYTES.min(expected.len() - at);
        reader.read(&mut store, at as u64, &mut window[..take]).expect("inside the extent");
        assert_eq!(&window[..take], &expected[at..at + take], "the extent read back wrong at {at}");
        at += take;
    }

    out
}

/// A device with room for the extent, its rewrites and its snapshots.
///
/// Sized from the workload rather than guessed, because a `refusal::FULL` in the
/// middle of a measurement would be a shorter run reported as a complete one.
fn a_store(object_bytes: usize, writes: usize) -> Store<Memory> {
    let block_bytes: u64 = 4096;
    let per_piece = EXTENT_BYTES as u64 / block_bytes + 1;
    // Every write may rewrite two pieces, and one extra placed write besides.
    let blobs = object_bytes.div_ceil(EXTENT_BYTES) as u64 + 2 * writes as u64 + 4;
    let blocks = blobs * per_piece + 4 * writes as u64 + 64;
    let device = Memory::new(block_bytes as usize, blocks);
    let layout = superblock_for_this_build(block_bytes as u32, 1, blocks * block_bytes, 0, 0);
    Store::format(device, &layout).expect("a device this build formatted itself")
}

fn merge_tally(into: &mut Tally, from: &Tally) {
    into.edits += from.edits;
    into.max = into.max.max(from.max);
    into.total += from.total;
}

fn merge_chunked(into: &mut Chunked, from: &Chunked) {
    merge_tally(&mut into.unstarved, &from.unstarved);
    merge_tally(&mut into.starved, &from.starved);
    merge_tally(&mut into.hashed_unstarved, &from.hashed_unstarved);
    merge_tally(&mut into.hashed_starved, &from.hashed_starved);
    merge_tally(&mut into.violations, &from.violations);
    merge_tally(&mut into.unstarved_crossing_nothing, &from.unstarved_crossing_nothing);
}

fn merge_extent(into: &mut Extents, from: &Extents) {
    merge_tally(&mut into.copied, &from.copied);
    merge_tally(&mut into.hashed, &from.hashed);
    merge_tally(&mut into.pieces, &from.pieces);
    merge_tally(&mut into.straddling, &from.straddling);
    merge_tally(&mut into.snapshots, &from.snapshots);
    into.placed += from.placed;
}

/// The rows, printed under the names claim 0017 registers them under.
fn report(
    geometry: &Geometry,
    app_bytes: u64,
    chunked_small: &Chunked,
    chunked_large: &Chunked,
    extent_small: &Extents,
    extent_large: &Extents,
) {
    let published = (CHUNK_MAX_BYTES + RESYNC_BOUND_BYTES) as u64;
    let extent_bound = 2 * EXTENT_BYTES as u64;

    println!("=== the rows claim 0017 registers ===");
    println!();
    // The drawn workload's total, and the word is load-bearing: the placed
    // straddling writes are counted in the per-kind ratios below through their
    // own edit counts, and folding them in here would make one denominator mean
    // two things.
    println!("application_bytes_written (drawn)            {app_bytes}");
    println!();
    println!("bytes_rechunked_per_edit_chunked_small       {}", chunked_small.unstarved.max);
    println!("bytes_rechunked_per_edit_chunked_large       {}", chunked_large.unstarved.max);
    println!("bytes_rehashed_per_edit_chunked_small        {}", chunked_small.hashed_unstarved.max);
    println!("bytes_rehashed_per_edit_chunked_large        {}", chunked_large.hashed_unstarved.max);
    println!("  threshold                                  {published} (all four, and the same");
    println!("  at both sizes, so a number that scales fails)");
    println!(
        "  edits the bound was exercised on            {} small, {} large",
        chunked_small.unstarved.edits, chunked_large.unstarved.edits
    );
    println!(
        "  edits inside a starved run (no threshold)   {} small, {} large",
        chunked_small.starved.edits, chunked_large.starved.edits
    );
    println!(
        "  of the unstarved edits, violating that bound {} small, {} large",
        chunked_small.violations.edits, chunked_large.violations.edits
    );
    println!(
        "  unstarved edits crossing no starved run     {} small, {} large",
        chunked_small.unstarved_crossing_nothing.edits,
        chunked_large.unstarved_crossing_nothing.edits
    );
    println!(
        "  their re-chunk maximum                      {} small, {} large",
        chunked_small.unstarved_crossing_nothing.max, chunked_large.unstarved_crossing_nothing.max
    );
    println!("  That last pair is a diagnosis and not a second threshold: a bound taken");
    println!("  over the edits that passed is a bound fitted to its measurement. It is");
    println!("  here because when it differs from the row above it, the difference names");
    println!("  the cause — a starved run the two streams entered *after* the edit, which");
    println!("  `starved_end` evaluated at the edit cannot see.");
    println!("bytes_rechunked_per_edit_starved_small       {}", chunked_small.starved.max);
    println!("bytes_rechunked_per_edit_starved_large       {}", chunked_large.starved.max);
    println!("bytes_rehashed_per_edit_starved_small        {}", chunked_small.hashed_starved.max);
    println!("bytes_rehashed_per_edit_starved_large        {}", chunked_large.hashed_starved.max);
    println!("  no threshold, deliberately: on starved content the number scales with");
    println!("  the object, which is what RFC 0062 proves and what the second object");
    println!("  kind exists for. A threshold here would be false or vacuous.");
    println!();
    println!("bytes_rechunked_per_edit_extent_small        {}", extent_small.copied.max);
    println!("bytes_rechunked_per_edit_extent_large        {}", extent_large.copied.max);
    println!("bytes_rehashed_per_edit_extent_small         {}", extent_small.hashed.max);
    println!("bytes_rehashed_per_edit_extent_large         {}", extent_large.hashed.max);
    println!("  threshold                                  {extent_bound} (2 x EXTENT_BYTES)");
    println!(
        "pieces_touched_max                           {}",
        extent_large.pieces.max.max(extent_small.pieces.max)
    );
    println!(
        "bytes_rechunked_per_edit_extent_straddling   {}",
        extent_small.straddling.max.max(extent_large.straddling.max)
    );
    println!(
        "  straddling writes seen                     {} small, {} large",
        extent_small.straddling.edits, extent_large.straddling.edits
    );
    println!(
        "  of those, placed rather than drawn          {} small, {} large",
        extent_small.placed, extent_large.placed
    );
    println!(
        "extent_snapshot_bytes_max                    {} small, {} large",
        extent_small.snapshots.max, extent_large.snapshots.max
    );
    println!("  snapshot_interval_writes                   {}", geometry.snapshot_interval);
    println!();
    println!("--- per application byte written, which is what the claim's name says ---");
    println!(
        "chunked, unstarved edits                     {:.1} small, {:.1} large",
        chunked_small.unstarved.per_app_byte(chunked_small.unstarved.edits * WRITE_BYTES as u64),
        chunked_large.unstarved.per_app_byte(chunked_large.unstarved.edits * WRITE_BYTES as u64)
    );
    println!(
        "chunked, starved edits                       {:.1} small, {:.1} large",
        chunked_small.starved.per_app_byte(chunked_small.starved.edits * WRITE_BYTES as u64),
        chunked_large.starved.per_app_byte(chunked_large.starved.edits * WRITE_BYTES as u64)
    );
    println!(
        "extent                                       {:.1} small, {:.1} large",
        extent_small.copied.per_app_byte(extent_small.copied.edits * WRITE_BYTES as u64),
        extent_large.copied.per_app_byte(extent_large.copied.edits * WRITE_BYTES as u64)
    );
    println!();

    let verdicts = [
        ("bytes_rechunked_per_edit_chunked_small", chunked_small.unstarved.max, published),
        ("bytes_rechunked_per_edit_chunked_large", chunked_large.unstarved.max, published),
        ("bytes_rechunked_per_edit_extent_small", extent_small.copied.max, extent_bound),
        ("bytes_rechunked_per_edit_extent_large", extent_large.copied.max, extent_bound),
        ("bytes_rehashed_per_edit_extent_small", extent_small.hashed.max, extent_bound),
        ("bytes_rehashed_per_edit_extent_large", extent_large.hashed.max, extent_bound),
        ("bytes_rehashed_per_edit_chunked_small", chunked_small.hashed_unstarved.max, published),
        ("bytes_rehashed_per_edit_chunked_large", chunked_large.hashed_unstarved.max, published),
        ("pieces_touched_max", extent_large.pieces.max.max(extent_small.pieces.max), 2),
    ];
    let mut red = 0;
    for (name, measured, threshold) in verdicts {
        let verdict = if measured <= threshold { "green" } else { "RED" };
        if measured > threshold {
            red += 1;
        }
        println!("{verdict:>5}  {name} = {measured}, threshold {threshold}");
    }

    // The positive control, and it is an assertion rather than a printed line
    // because a run in which nothing was unstarved reports every threshold above
    // as green while asserting nothing about any of them. It is the same vacuity
    // `claims/0017`'s `resync_pairs_unstarved` row exists to make visible, one
    // level down.
    assert!(
        chunked_small.unstarved.edits > 0 && chunked_large.unstarved.edits > 0,
        "no edit at either size landed outside a starved run, so the published bound was \
         asserted on nothing. Look at the generator before the chunker: a draw that stopped \
         producing uniform content does exactly this"
    );
    assert!(
        extent_small.straddling.edits > 0 && extent_large.straddling.edits > 0,
        "no straddling write was measured, so the row that says why the extent threshold is \
         two megabytes rather than one is a row over no observations"
    );

    println!();
    if red == 0 {
        println!("every threshold green");
    } else {
        println!("{red} threshold(s) RED — do not move one to fit; RFC 0058's reversal");
        println!("conditions name what each means, and a bound moves only by an RFC");
        println!("carrying the measurement");
    }
}
