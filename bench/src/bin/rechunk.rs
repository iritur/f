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
//! Per edit, this classifies the edit by the candidate gaps around it and not
//! by the mixture's name, because RFC 0062's theorem says the name decides
//! nothing. **Three classes and not two, which is RFC 0064 and the correction
//! this file carries**: an edit *inside* a starved run is excused by the bound's
//! second clause; an edit *outside* every starved run whose flat window meets
//! one is excused by the same clause for the same reason, because the forced cut
//! that separates the two streams is in front of the edit rather than under it;
//! and an edit that is neither is one the published flat bound
//! `CHUNK_MAX_BYTES + RESYNC_BOUND_BYTES` = 786 432 must hold on. The primary
//! rows are the maximum over exactly the third class.
//!
//! Until RFC 0064 the first two were one predicate evaluated at the edit, and
//! this file measured 1 138 541 bytes against a 786 432-byte threshold on 5 of
//! 158 in-scope edits at 8 MiB — every one of them on concatenated content
//! ahead of a candidate gap of 645 661 or 576 301 bytes. That is what the third
//! class is for, and the second class is recorded rather than dropped: its
//! costs, its counts, and the second clause asserted over it at zero, because a
//! class excused from one clause and asserted under none is a class that cannot
//! go red. The count of each is printed, and the third has a floor
//! ([`CLEAR_EDITS_MIN`]) — a primary taken over no edits at all is a green row
//! asserting nothing, the same vacuity `claims/0017`'s `resync_pairs_unstarved`
//! exists to make visible.
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

use f_abi::buf::SetId;
use f_abi::flags;
use f_abi::objects::{Entry as Asked, Request, Write as WriteRecord};
use f_bench::Environment;
use f_blob::chunk::{
    CHUNK_MAX_BYTES, CHUNK_MIN_BYTES, CHUNK_TARGET_BYTES, Chunker, RESYNC_BOUND_BYTES,
};
use f_blob::device::Memory;
use f_blob::extent::{Cost, EXTENT_BYTES, Extent};
use f_blob::gear::{GEAR, MASK};
use f_blob::store::{Store, superblock_for_this_build};
use f_env::split::{Stream, label};
use f_hash::sha256;
use f_index::Index;
use f_objects::dma::Landing;
use f_objects::read::mount;
use f_objects::service::Service;
use f_zone::device::ZonedMemory;
use f_zone::map::ZoneMap;

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

/// The modelled device's logical block. Unit: bytes.
///
/// Four kibibytes, which is the block `intent/0006-state/spec.md` derives claim
/// 0016's padding term at and therefore the block every number in this epoch is
/// about. `user/objects/tests/reads.rs` uses the same one for the same reason.
const BLOCK_BYTES: usize = 4096;

/// Blocks in one zone. Unit: count of blocks — one mebibyte a zone.
const ZONE_BLOCKS: u64 = 256;

/// The zones below the first data zone: the superblock's conventional zone and
/// the two root zones. Unit: count of zones. `zone/tests/cycle.rs`'s geometry.
const DATA_FROM: u32 = 3;

/// Buffers in the client's registered set. Unit: count of buffers.
///
/// Two, each [`WRITE_BYTES`] wide. One would do — this workload has one write
/// outstanding at a time — and two exist so that the set is a *set*: a
/// registration of one buffer cannot tell an index that is checked from an
/// index that is ignored, and `f_ring::registry::Table::resolve` refusing an
/// index past the set is the check this workload's bytes pass through.
const RING_BUFFERS: u32 = 2;

/// Blocks the index's log region is given. Unit: count of blocks.
///
/// The write path never resolves a name, so this is the smallest region
/// `Index::mount` accepts rather than a figure derived from the workload — and
/// it is here at all because `f_objects::read::ReadPath` owns an index and a
/// service is put in front of a *read* path.
const LOG_BLOCKS: u64 = 64;

/// The index's own block. Unit: bytes — the smallest this format accepts.
const LOG_BLOCK_BYTES: usize = 512;

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

/// The positive control on RFC 0064's scope. Unit: count of edits, per object
/// size.
///
/// **A scope that excuses everything asserts nothing**, and RFC 0064 widens
/// what the flat clause does not cover, so the count of edits it still covers
/// is a threshold rather than a printed line. Sixty-four is derived and not
/// measured: the uniform mixture alone contributes `SEEDS.len() *
/// WRITES_PER_OBJECT` = 128 edits per size, and on uniform content the
/// candidate gaps are geometric with mean `2^MASK_BITS` = 65 536 bytes, so an
/// edit is inside a starved run with the length-biased probability
/// `(1 + r)e^(−r)` = 0.112 at `r = STARVED_GAP_BYTES / 65536` = 3.75, and a
/// starved gap begins inside the 512 KiB window with probability
/// `1 − e^(−8 e^(−r))` = 0.172. That leaves 0.736 of them clear, or about 94 of
/// the 128, with a standard deviation near five. Sixty-four is six standard
/// deviations below that and is reachable by a broken generator and by nothing
/// else — the same shape `claims/0017`'s `resync_pairs_unstarved = { min = 4 }`
/// is derived in, one level down.
const CLEAR_EDITS_MIN: u64 = 64;

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

/// The end of the last starved run reaching the flat clause's own window, if
/// one does.
///
/// **This is RFC 0064's scope predicate and the reason it exists.** RFC 0061
/// states the second clause of the bound over a starved run the two boundary
/// sequences travel *across*; this workload, and `blob/tests/chunker.rs` before
/// it, evaluated that clause at the edit alone — [`starved_end`] answers *is the
/// edit inside a run*, which is a predicate at a point where the bound is a
/// statement over an interval. An edit in cuttable content 100 KiB ahead of a
/// 645 KiB candidate gap is outside every starved run and its two streams cross
/// one anyway, which is a forced cut at two different phases and the same
/// desynchronisation RFC 0062's theorem describes, reached from outside rather
/// than from inside.
///
/// The window is `[from, from + RESYNC_BOUND_BYTES]` and it is the flat clause's
/// own allowance rather than a fitted one: what the flat clause promises is
/// agreement inside that interval, so the content it can promise it over is
/// content with no forced cut in it. Anything shorter would be a window chosen
/// to enlarge the sample, which is the fitted number `claims/0017`'s
/// `[diagnosis]` tells the next reader not to write.
///
/// The gaps are measured whole and not clipped to the window: a run that begins
/// before `from` or ends after the window's end forces its cut inside the window
/// all the same, and a gap measured only over its intersection would report a
/// run that can force a cut as one that cannot.
fn starved_run_meeting_the_window(
    candidates: &[usize],
    length: usize,
    from: usize,
) -> Option<usize> {
    let to = (from + RESYNC_BOUND_BYTES).min(length);
    let mut previous = 0usize;
    let mut last_start = None;
    for &candidate in candidates.iter().chain(core::iter::once(&length)) {
        // The gap is `(previous, candidate]`; it meets `[from, to]` when it
        // ends at or after `from` and begins at or before `to`.
        if candidate.saturating_sub(previous) >= STARVED_GAP_BYTES
            && candidate >= from
            && previous <= to
        {
            last_start = Some(previous);
        }
        previous = candidate;
    }
    // Consecutive starved gaps are one run, and the clause is about the run's
    // end rather than the gap's — so the walk continues from the last gap that
    // meets the window, which is exactly what `starved_end` does.
    last_start.map(|start| starved_end(candidates, length, start))
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
    /// Edits the published flat bound is stated over: no starved run meets the
    /// window that clause allows itself. RFC 0064, and the rows
    /// `bytes_rechunked_per_edit_chunked_*` are the maximum over exactly these.
    clear: Tally,
    /// The same edits' hashing, kept apart from the classes below for the
    /// reason the re-chunk rows are: the two carry the same threshold, and a
    /// maximum taken across classes would report an excused number under a
    /// threshold that cannot apply to it.
    hashed_clear: Tally,
    /// Edits outside every starved run whose flat window meets one — the class
    /// RFC 0064 found and named, and the class this file measured under the
    /// flat threshold until it did.
    ///
    /// Recorded with no flat threshold and *with* the second clause over it:
    /// `near_second_clause_violations` is the falsifiable half, because a class
    /// excused from one clause and asserted under none is a class that cannot
    /// go red.
    near: Tally,
    hashed_near: Tally,
    /// Edits whose position is not inside a starved run: `clear` and `near`
    /// together, kept because it is what this file reported before RFC 0064 and
    /// the difference between the two is the whole of that entry's measurement.
    unstarved: Tally,
    /// Edits inside a starved run: the ones its second clause excuses.
    starved: Tally,
    /// Bytes fed to `f-hash`, split the same way. Split and not aggregated,
    /// because the re-hash rows carry the same threshold as the re-chunk rows
    /// and a maximum taken across both classes would report a starved object's
    /// number under a threshold that cannot apply to it.
    hashed_unstarved: Tally,
    hashed_starved: Tally,
    /// Edits in the `clear` class that exceeded the published flat bound.
    ///
    /// Counted rather than only maximised, because *how many* is the difference
    /// between a bound that is wrong and a bound that is wrong on a class
    /// somebody can name. Each one prints its own line as it happens, and under
    /// RFC 0064 a single one of them falsifies the flat clause on the content
    /// that clause is stated over — there is no run left to blame.
    violations: Tally,
    /// Edits in the `near` or `starved` classes that exceeded the **second**
    /// clause: `max(edit_end + RESYNC_BOUND_BYTES, run_end + CHUNK_MAX_BYTES)`,
    /// with the run taken from the content rather than from where the two
    /// streams happened to agree.
    ///
    /// This is what keeps the excused classes falsifiable. RFC 0061 says the
    /// sequences agree at the end of the run plus one chunk; if they do not,
    /// the second clause is wrong and the bound has no clause left that holds.
    second_clause_violations: Tally,
    /// The subset of the unstarved edits whose resynchronisation crossed no
    /// starved run.
    ///
    /// **This is a diagnosis and not a threshold**, and it is kept beside
    /// `clear` rather than replaced by it because the two ask different
    /// questions: `clear` asks whether a forced cut was *possible* anywhere in
    /// the flat clause's own window, before the chunker ran, and this asks
    /// whether one was actually crossed on the way back to agreement. The first
    /// is the scope of a bound and the second is what happened. When they
    /// differ, the difference is the count of edits the scope excuses and the
    /// run did not need excused.
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
    ///
    /// **One per write since `E2-B09` closed, and that is RFC 0098's price
    /// rather than a knob.** A completion has to carry the object's new content
    /// address and an extent has none until it is snapshotted, so the interval
    /// this bench used to choose is no longer the bench's to choose. It stays
    /// its own row for the reason it always had one: a snapshot is proportional
    /// to the piece count and therefore to the *object*, and folding it into
    /// the per-write cost would turn `2 x EXTENT_BYTES at both sizes` into two
    /// numbers sixteen times apart.
    snapshots: Tally,
    /// Application bytes the **component** counted, off
    /// `f_objects::Served::written_bytes`.
    ///
    /// `claims/0017`'s denominator, and the second of two sums neither derived
    /// from the other: this workload knows how many bytes it drew and the
    /// service knows how many a client submitted, and the run requires them to
    /// agree. Unit: bytes.
    submitted: u64,
    /// Entries the service answered for this cell. Unit: count of entries.
    ///
    /// The count beside the byte sum, for `f_objects::dma::Landed`'s stated
    /// reason: a zero beside a zero says nothing.
    answered: u64,
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
            "mixture            seed     clear re-chunk max      near    near max   starved \
             starved max  period"
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
                let extents = measure_extent(object, &edits);

                println!(
                    "{:<17} {:>5} {:>9} {:>11} {:>9} {:>11} {:>9} {:>11}  {}",
                    kind.name(),
                    seed,
                    chunked.clear.edits,
                    chunked.clear.max,
                    chunked.near.edits,
                    chunked.near.max,
                    chunked.starved.edits,
                    chunked.starved.max,
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
    seeds: Vec<u64>,
}

impl Geometry {
    fn from_args() -> Self {
        let mut geometry = Self {
            small: OBJECT_BYTES_SMALL,
            large: OBJECT_BYTES_LARGE,
            writes: WRITES_PER_OBJECT,
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
        println!("snapshot_per_write        1 (RFC 0098: a completion carries the new address)");
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
        // The run the *bound* is stated over, which is not always the run the
        // edit sits in. RFC 0064: a starved run anywhere in the window the flat
        // clause allows itself forces a cut the two streams reach at different
        // phases, and the clause cannot promise agreement across it whether the
        // edit was inside the run or a hundred kilobytes ahead of it.
        let meeting = starved_run_meeting_the_window(&marks, edited.len(), edit_end);
        // The second clause, evaluated on the content: the end of that run plus
        // one chunk, or the flat allowance, whichever is later. `run_end` and
        // `meeting` agree whenever the edit is inside a run, because a run
        // containing `edit_end` meets any window starting there.
        let second_clause =
            meeting.map(|end| (edit_end + RESYNC_BOUND_BYTES).max(end + CHUNK_MAX_BYTES));
        if run_end > edit_end {
            out.starved.record(rechunked);
            out.hashed_starved.record(hashed);
            record_second_clause(
                &mut out,
                second_clause,
                agreed,
                rechunked,
                label,
                edit.at,
                "inside",
            );
        } else if let Some(end) = meeting {
            out.unstarved.record(rechunked);
            out.hashed_unstarved.record(hashed);
            out.near.record(rechunked);
            out.hashed_near.record(hashed);
            record_second_clause(
                &mut out,
                second_clause,
                agreed,
                rechunked,
                label,
                edit.at,
                "near",
            );
            // Printed whenever this class costs more than the flat clause
            // allows, because that is RFC 0064's whole measurement and a run
            // that only reported the maximum would hand the next reader a
            // number and no way to reach the edit behind it.
            if rechunked > (CHUNK_MAX_BYTES + RESYNC_BOUND_BYTES) as u64 {
                println!(
                    "  ~ {label} edit at {} re-chunked {rechunked} ({region_start} -> {agreed}); \
                     outside every starved run, and a run ending at {end} meets the flat clause's \
                     own {RESYNC_BOUND_BYTES}-byte window — RFC 0064's class, second clause \
                     allows {}",
                    edit.at,
                    second_clause.unwrap_or(0),
                );
            }
            let crossed = widest_gap(&marks, edit_end, agreed);
            if crossed < STARVED_GAP_BYTES {
                out.unstarved_crossing_nothing.record(rechunked);
            }
        } else {
            out.unstarved.record(rechunked);
            out.hashed_unstarved.record(hashed);
            out.clear.record(rechunked);
            out.hashed_clear.record(hashed);
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
                let allowed = edit_end + RESYNC_BOUND_BYTES;
                println!(
                    "  ! {label} edit at {} re-chunked {rechunked} ({region_start} -> {agreed}); \
                     no starved run meets the flat clause's window, so its allowance {allowed} is \
                     the whole of it; widest candidate gap crossed {crossed}",
                    edit.at,
                );
            }
        }

        edited[edit.at..edit.at + saved.len()].copy_from_slice(&saved);
    }
    out
}

/// The second clause, checked on the two classes the flat one excuses.
///
/// **A class excused from one clause and asserted under none is a class that
/// cannot go red**, which is the failure mode RFC 0064 is most exposed to: it
/// widens what the flat clause does not cover, and a widening that reports no
/// number is a widening nobody can argue with. RFC 0061 says the two sequences
/// agree at the end of the starved run plus one chunk, so that is asserted here
/// — over the run the *content* has, not over where the streams happened to
/// agree, because an allowance derived from the outcome would be satisfied by
/// any outcome.
fn record_second_clause(
    out: &mut Chunked,
    allowed: Option<usize>,
    agreed: usize,
    rechunked: u64,
    label: &str,
    at: usize,
    class: &str,
) {
    let Some(allowed) = allowed else { return };
    if agreed > allowed {
        out.second_clause_violations.record(rechunked);
        println!(
            "  !! {label} edit at {at} ({class} a starved run) agreed only at {agreed}, past the \
             {allowed} the bound's second clause allows — the run's end plus one chunk. RFC 0061's \
             second clause is what is wrong here, and it is the last clause the bound has"
        );
    }
}

/// The extent kind: the same edits, submitted by a client across a real objects
/// ring.
///
/// # Why this goes through a service rather than straight to the store
///
/// Because `claims/0017` is *bytes re-chunked and re-hashed per application
/// byte written*, `intent/0006-state/spec.md` defines an application byte once
/// — **a byte the client submitted on the objects ring** — and until `E2-B09`
/// closed, this function called `Extent::write` directly and the denominator
/// was `edits.len() * WRITE_BYTES`, an arithmetic the bench did for itself.
/// That is a write path and not a client, and the claim said so for two epochs.
///
/// So the bytes go into a registered buffer, an `f_abi::Sqe` names that buffer,
/// `f_objects::service::Service` answers it, and both halves of the ratio are
/// read back off `f_objects::Served` afterwards. The store underneath is the
/// same `f_blob::extent::Extent`, reached by the same call — what changed is
/// who called it and where the count was taken.
///
/// # Why a boot cannot be this, which is RFC 0098 and not a shortcut
///
/// `Extent::write` allocates a piece buffer of `EXTENT_BYTES` — a compile-time
/// mebibyte — whatever the object's size, and `kernel/src/objects.rs` gives
/// `user/objects` a 128 KiB heap. So a boot's channel has no subject, a
/// non-zero offset on it is refused, and this workload's edits belong in a host
/// process over the same `Service`. A ring is not a boot; the spec says ring.
///
/// The pieces are read back and compared against the bytes the workload expects
/// before any number is reported: a measurement of a write path that produced
/// the wrong bytes would be a measurement of nothing.
fn measure_extent(mut expected: Vec<u8>, edits: &[Edit]) -> Extents {
    let mut out = Extents::default();
    let mut store = a_store(expected.len(), edits.len());
    // The object is taken by value and then mutated into the bytes the workload
    // expects, so that a 128 MiB run holds one copy of it and not two. A second
    // copy is not free at this size and buys nothing: the base object has no
    // reader left once the pieces are written.
    let extent = Extent::create(&mut store, &expected).expect("a device sized for the object");

    let (_, first) = extent.snapshot(&mut store).expect("a record fits");
    out.snapshots.record(first.copied);

    // The index the read path owns and this workload never asks anything of.
    // A service is put in front of a *read* path, and a read path resolves
    // names; nothing here has a name to resolve.
    let log = Memory::new(LOG_BLOCK_BYTES, LOG_BLOCKS);
    let index = Index::mount(log, 0, LOG_BLOCKS).expect("a log region this build sized itself");
    let mut service = Service::over(mount(store, index));
    // What the channel is about. `serve.rs` never makes this call, which is the
    // whole of why a boot refuses an edit and this does not.
    service.about(extent);

    let mut region = vec![0u8; RING_BUFFERS as usize * WRITE_BYTES];
    let (mut landing, set) =
        Landing::open(&mut region, RING_BUFFERS).expect("a region that divides into its buffers");

    for (which, edit) in edits.iter().enumerate() {
        let cost =
            submit(&mut service, &mut landing, set, which as u64, edit.at as u64, &edit.bytes);
        expected[edit.at..edit.at + edit.bytes.len()].copy_from_slice(&edit.bytes);

        out.copied.record(cost.copied);
        out.hashed.record(cost.hashed);
        out.pieces.record(u64::from(cost.pieces));
        if cost.pieces > 1 {
            out.straddling.record(cost.copied);
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
        let cost = submit(&mut service, &mut landing, set, edits.len() as u64, at, &bytes);
        expected[at as usize..at as usize + WRITE_BYTES].copy_from_slice(&bytes);
        assert_eq!(cost.pieces, 2, "a write across a piece boundary rewrites two pieces");
        out.copied.record(cost.copied);
        out.hashed.record(cost.hashed);
        out.pieces.record(u64::from(cost.pieces));
        out.straddling.record(cost.copied);
        out.placed += 1;
    }

    // The component's own totals, read once at the end rather than accumulated
    // here: a denominator this file added up would be this file's arithmetic
    // again, and the whole of what moved is that it is not.
    let served = *service.served();
    out.submitted = served.written_bytes;
    out.answered = served.writes;
    assert_eq!(
        served.rechunked_bytes, out.copied.total,
        "the service's re-chunk total and the sum of what it answered per entry disagree"
    );
    assert_eq!(
        out.submitted,
        out.answered * WRITE_BYTES as u64,
        "the service counted application bytes this workload did not submit"
    );

    // The last write's answer is the extent's published name. Taken from the
    // completion's own detail rather than from the handle this file holds,
    // because a client reads back what it was *told*.
    let published = served.written_hash;

    // Verify, piece by piece so that nothing here holds a second copy of a
    // 128 MiB object. `Store::get` has already checked each piece against its
    // own name; what this adds is that the pieces are the bytes the *workload*
    // wrote, which no hash inside the store can say.
    let store = service.path_mut().store_mut();
    let reader = Extent::open(store, &published).expect("a published extent");
    assert_eq!(reader.extent_bytes(), expected.len() as u64);
    let mut window = vec![0u8; EXTENT_BYTES];
    let mut at = 0usize;
    while at < expected.len() {
        let take = EXTENT_BYTES.min(expected.len() - at);
        reader.read(store, at as u64, &mut window[..take]).expect("inside the extent");
        assert_eq!(&window[..take], &expected[at..at + take], "the extent read back wrong at {at}");
        at += take;
    }

    out
}

/// One client write across the ring, and what the service said it cost.
///
/// # The three steps, and why none of them is a shortcut
///
/// The client fills its own registered buffer through
/// [`f_objects::dma::Landing::fill`] — resolved through the same
/// `f_ring::registry::Table` the service resolves it through, so a buffer this
/// workload could not name is one the service would not read. It submits an
/// `f_abi::Sqe` naming that buffer by set and index, never by address. The
/// service answers, and the numbers come out of `f_objects::Served` rather than
/// out of anything this function computed.
///
/// Every refusal is a panic and not a recorded zero. A run in which the ring
/// refused half the writes and reported a bounded cost over the other half
/// would be a green row about a workload that did not happen.
fn submit(
    service: &mut Service<ZonedMemory, Memory>,
    landing: &mut Landing<'_>,
    set: SetId,
    token: u64,
    at: u64,
    bytes: &[u8],
) -> Cost {
    if let Err(code) = landing.fill(set, 0, bytes) {
        panic!("the client could not fill its own registered buffer: {code:#x}");
    }
    let (entry, payload) = Request {
        user_data: token,
        cap: 0,
        class: 0,
        deadline: 0,
        payload_offset: 0,
        buf_set: set.bits(),
        buf_index: 0,
        flags: flags::FIXED_BUF,
        body: Asked::Write(WriteRecord { offset: at, bytes: bytes.len() as u32 }),
    }
    .encode();

    let before = *service.served();
    let answer = service.answer(&entry, &payload, landing, 0);
    let after = *service.served();

    assert_eq!(answer.user_data, token, "the completion carries somebody else's token");
    if let Some((domain, code)) = answer.error() {
        panic!("write {token} at {at} was refused {domain:#x}/{code:#x}");
    }
    assert_eq!(
        answer.result,
        bytes.len() as i32,
        "the completion states a length the client did not submit"
    );
    assert_eq!(after.writes, before.writes + 1, "one entry answered one write");

    Cost {
        pieces: u32::try_from(after.pieces_rewritten - before.pieces_rewritten)
            .expect("a piece count that fits the wire's own width"),
        copied: after.rechunked_bytes - before.rechunked_bytes,
        hashed: after.rehashed_bytes - before.rehashed_bytes,
    }
}

/// A device with room for the extent, its rewrites and its snapshots.
///
/// Sized from the workload rather than guessed, because a `refusal::FULL` in the
/// middle of a measurement would be a shorter run reported as a complete one.
///
/// **Zoned since `E2-B09` closed**, because `f_objects::read::ReadPath` mounts a
/// `ZoneMap` and a service is put in front of a read path. The arithmetic below
/// is unchanged and the zones are derived from it: what a zone changes is where
/// a block lives, and this function's job is only to have enough of them.
fn a_store(object_bytes: usize, writes: usize) -> Store<ZoneMap<ZonedMemory>> {
    let block_bytes = BLOCK_BYTES as u64;
    let per_piece = EXTENT_BYTES as u64 / block_bytes + 1;
    // Every write may rewrite two pieces, and one extra placed write besides.
    let blobs = object_bytes.div_ceil(EXTENT_BYTES) as u64 + 2 * writes as u64 + 4;
    // One snapshot per write, each rewriting the extent's record — a hash a
    // piece, so the largest of them is 128 x 32 bytes and a block holds it.
    let blocks = blobs * per_piece + 4 * writes as u64 + 64;
    let zones = DATA_FROM + blocks.div_ceil(ZONE_BLOCKS) as u32 + 1;
    let device = ZonedMemory::new(BLOCK_BYTES, ZONE_BLOCKS, zones);
    let map = ZoneMap::new(device, DATA_FROM).expect("a device this build mapped itself");
    let layout =
        superblock_for_this_build(BLOCK_BYTES as u32, zones, ZONE_BLOCKS * block_bytes, 1, 2);
    Store::format(map, &layout).expect("a device this build formatted itself")
}

fn merge_tally(into: &mut Tally, from: &Tally) {
    into.edits += from.edits;
    into.max = into.max.max(from.max);
    into.total += from.total;
}

fn merge_chunked(into: &mut Chunked, from: &Chunked) {
    merge_tally(&mut into.clear, &from.clear);
    merge_tally(&mut into.hashed_clear, &from.hashed_clear);
    merge_tally(&mut into.near, &from.near);
    merge_tally(&mut into.hashed_near, &from.hashed_near);
    merge_tally(&mut into.unstarved, &from.unstarved);
    merge_tally(&mut into.starved, &from.starved);
    merge_tally(&mut into.hashed_unstarved, &from.hashed_unstarved);
    merge_tally(&mut into.hashed_starved, &from.hashed_starved);
    merge_tally(&mut into.violations, &from.violations);
    merge_tally(&mut into.second_clause_violations, &from.second_clause_violations);
    merge_tally(&mut into.unstarved_crossing_nothing, &from.unstarved_crossing_nothing);
}

fn merge_extent(into: &mut Extents, from: &Extents) {
    merge_tally(&mut into.copied, &from.copied);
    merge_tally(&mut into.hashed, &from.hashed);
    merge_tally(&mut into.pieces, &from.pieces);
    merge_tally(&mut into.straddling, &from.straddling);
    merge_tally(&mut into.snapshots, &from.snapshots);
    into.placed += from.placed;
    into.submitted += from.submitted;
    into.answered += from.answered;
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
    // **The same quantity, counted by the component that was submitted to**,
    // and the row `E2-B09` was open for. `intent/0006-state/spec.md` defines an
    // application byte as one the client submitted on the objects ring; this is
    // the sum `f_objects::Served::written_bytes` kept while answering, and it
    // includes the placed straddling writes because those crossed the ring too.
    //
    // Two sums, neither derived from the other: this file drew the edits and
    // the service counted the entries, and the assertion below is what makes
    // the pair evidence rather than one number printed twice.
    let at_the_ring = extent_small.submitted + extent_large.submitted;
    let placed = extent_small.placed + extent_large.placed;
    println!("application_bytes_written_at_the_ring        {at_the_ring}");
    let answered = extent_small.answered + extent_large.answered;
    println!("entries_answered_at_the_ring                 {answered}");
    println!("  = the drawn total plus the {placed} placed straddling write(s), which cross the");
    println!("  same ring. The chunked kind has no ring number and is not owed one: it has");
    println!("  no write path at all — `measure_chunked` re-chunks an object and undoes the");
    println!("  edit, which is the numerator's own measurement. The extent kind is where a");
    println!("  client submits, and it is the kind this claim's headline rows come from.");
    println!();
    println!("bytes_rechunked_per_edit_chunked_small       {}", chunked_small.clear.max);
    println!("bytes_rechunked_per_edit_chunked_large       {}", chunked_large.clear.max);
    println!("bytes_rehashed_per_edit_chunked_small        {}", chunked_small.hashed_clear.max);
    println!("bytes_rehashed_per_edit_chunked_large        {}", chunked_large.hashed_clear.max);
    println!("  threshold                                  {published} (all four, and the same");
    println!("  at both sizes, so a number that scales fails)");
    println!("  The four rows above are taken over the edits RFC 0064 says the flat clause");
    println!("  is stated over: no starved run meets the {RESYNC_BOUND_BYTES}-byte window that");
    println!("  clause allows itself. That is a predicate on the content and the edit, fixed");
    println!("  before the chunker runs; it is not the set of edits that passed.");
    println!("rechunk_edits_clear_of_a_starved_run_small   {}", chunked_small.clear.edits);
    println!("rechunk_edits_clear_of_a_starved_run_large   {}", chunked_large.clear.edits);
    println!("rechunk_edits_near_a_starved_run_small       {}", chunked_small.near.edits);
    println!("rechunk_edits_near_a_starved_run_large       {}", chunked_large.near.edits);
    println!("bytes_rechunked_per_edit_near_starved_small  {}", chunked_small.near.max);
    println!("bytes_rechunked_per_edit_near_starved_large  {}", chunked_large.near.max);
    println!("  no flat threshold on those two, and the second clause instead: the edit is");
    println!("  outside every starved run and its two streams cross one anyway, which is a");
    println!("  forced cut at two phases and the desynchronisation RFC 0062 proves — reached");
    println!("  from outside the run rather than from inside it. RFC 0064 is that entry.");
    println!(
        "rechunk_edits_exceeding_the_second_clause     {}",
        chunked_small.second_clause_violations.edits + chunked_large.second_clause_violations.edits
    );
    println!("  the excused classes' own threshold, and it is zero: RFC 0061 says the two");
    println!("  sequences agree at the end of the starved run plus one chunk.");
    println!(
        "  edits inside a starved run (second clause)  {} small, {} large",
        chunked_small.starved.edits, chunked_large.starved.edits
    );
    println!(
        "  of the clear edits, violating the flat bound {} small, {} large",
        chunked_small.violations.edits, chunked_large.violations.edits
    );
    println!(
        "  edits outside a starved run in total        {} small, {} large",
        chunked_small.unstarved.edits, chunked_large.unstarved.edits
    );
    println!(
        "  their re-chunk maximum                      {} small, {} large",
        chunked_small.unstarved.max, chunked_large.unstarved.max
    );
    println!(
        "  unstarved edits crossing no starved run     {} small, {} large",
        chunked_small.unstarved_crossing_nothing.edits,
        chunked_large.unstarved_crossing_nothing.edits
    );
    println!("  The last three lines are the falsification RFC 0064 was written from, kept");
    println!("  where a reader finds them: `edits outside a starved run` is what this file");
    println!("  measured the flat threshold over until that entry, and the maximum beside it");
    println!("  is the number that exceeded it. `crossing no starved run` asks what actually");
    println!("  happened rather than what the content allowed, and is a diagnosis and never");
    println!("  a threshold: a bound taken over the edits that passed is a bound fitted to");
    println!("  its measurement.");
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
    println!("  one snapshot per write, because RFC 0098 makes the completion carry the");
    println!("  object's new address and an extent has none until it is snapshotted. It is");
    println!("  its own row and not part of the per-write cost: a snapshot scales with the");
    println!("  piece count, so folding it in would make the two rows above differ by the");
    println!("  object's size — which is the property they exist to deny.");
    println!();
    println!("--- per application byte written, which is what the claim's name says ---");
    println!(
        "chunked, edits clear of a starved run        {:.1} small, {:.1} large",
        chunked_small.clear.per_app_byte(chunked_small.clear.edits * WRITE_BYTES as u64),
        chunked_large.clear.per_app_byte(chunked_large.clear.edits * WRITE_BYTES as u64)
    );
    println!(
        "chunked, edits near a starved run            {:.1} small, {:.1} large",
        chunked_small.near.per_app_byte(chunked_small.near.edits * WRITE_BYTES as u64),
        chunked_large.near.per_app_byte(chunked_large.near.edits * WRITE_BYTES as u64)
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
        ("bytes_rechunked_per_edit_chunked_small", chunked_small.clear.max, published),
        ("bytes_rechunked_per_edit_chunked_large", chunked_large.clear.max, published),
        ("bytes_rechunked_per_edit_extent_small", extent_small.copied.max, extent_bound),
        ("bytes_rechunked_per_edit_extent_large", extent_large.copied.max, extent_bound),
        ("bytes_rehashed_per_edit_extent_small", extent_small.hashed.max, extent_bound),
        ("bytes_rehashed_per_edit_extent_large", extent_large.hashed.max, extent_bound),
        ("bytes_rehashed_per_edit_chunked_small", chunked_small.hashed_clear.max, published),
        ("bytes_rehashed_per_edit_chunked_large", chunked_large.hashed_clear.max, published),
        ("pieces_touched_max", extent_large.pieces.max.max(extent_small.pieces.max), 2),
        // The excused classes' own row, and the reason RFC 0064 is a scope and
        // not an excuse: zero, over both sizes and both classes.
        (
            "rechunk_edits_exceeding_the_second_clause",
            chunked_small.second_clause_violations.edits
                + chunked_large.second_clause_violations.edits,
            0,
        ),
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
    //
    // Under RFC 0064 the set is narrower than it was — an edit merely *near* a
    // starved run is excused too — so the control is stated over the narrower
    // set, and at a floor rather than at one edit. `CLEAR_EDITS_MIN` is where
    // that floor comes from and why it is a number and not a `> 0`.
    //
    // The floor scales down with a shrunken geometry rather than failing on it,
    // because the arguments exist so that an agent changing this file can run it
    // in a minute and a control that only holds at full size would be deleted by
    // the second person who met it. What catches a shrunken run is the claim's
    // own geometry rows — `writes_per_object`, `object_bytes_small` and
    // `object_bytes_large` are thresholds too, and `cargo xtask claim` compares
    // them — so the smoke run reports its numbers and the registry still refuses
    // to publish them.
    let floor = ((geometry.seeds.len() * geometry.writes) as u64 / 2).min(CLEAR_EDITS_MIN);
    for (size, chunked) in [("small", chunked_small), ("large", chunked_large)] {
        assert!(
            chunked.clear.edits >= floor,
            "only {} of the {size} object's edits are clear of every starved run, below the \
             {floor} the uniform draw alone puts out of reach of anything but a broken \
             generator. The published bound is being asserted on a sample that has quietly \
             emptied, which is the same vacuity `claims/0017`'s `resync_pairs_unstarved` row \
             exists to make visible, one level down",
            chunked.clear.edits
        );
    }
    assert!(
        extent_small.straddling.edits > 0 && extent_large.straddling.edits > 0,
        "no straddling write was measured, so the row that says why the extent threshold is \
         two megabytes rather than one is a row over no observations"
    );

    // **The two denominators, required to be equal**, and an assertion rather
    // than a verdict row because the verdict table compares `measured <=
    // threshold` and this needs both directions. A workload that drew one total
    // and a service that counted another is a run in which some writes did not
    // reach the ring, and every ratio above it would then be taken over a
    // submission that did not happen.
    assert_eq!(
        at_the_ring,
        app_bytes + placed * WRITE_BYTES as u64,
        "the client drew {app_bytes} bytes of edits and placed {placed} more write(s), and \
         the service counted {at_the_ring}. Two sums of the same submissions disagreeing is \
         not something this workload may report the larger of"
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
