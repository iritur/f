// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-P02`: the chunker's properties, written before the chunker.
//!
//! # Why this file is committed before `blob/src/chunk.rs` exists
//!
//! `docs/sdlc.md` gives bug fixes the discipline of committing the failing test
//! first, so that the test tests the bug rather than the fix. The same
//! discipline is worth more applied to a *bound*: a property written after the
//! code passes is a property written around the code, and the way that failure
//! presents is a green suite over a design nobody checked. So this file is
//! committed in a state where it does not compile, naming an interface the plan
//! fixed and an implementation that does not exist yet.
//!
//! # Why the generator is a mixture and not uniform bytes
//!
//! With uniform bytes a candidate appears roughly every
//! [`CHUNK_TARGET_BYTES`], so every property below passes on the data the
//! design is good at while the workload it is bad at — zero runs, short
//! periods, the VM images and sparse database files `E2-D02` names — fails
//! unobserved. Four mixtures: uniform, zero-filled, periodic with a period
//! below the target size, and concatenations of those. The mixture is in every
//! failure message, because a bound that fails on one mixture and holds on
//! three is a fact about the design and not a flake.
//!
//! Objects, edits, lengths and the mixture choice are drawn from `Stream`
//! children at four named identities under RFC 0026, so that a fifth draw added
//! later moves none of the four that are here.

use std::collections::BTreeMap;

use f_blob::chunk::{
    CHUNK_MAX_BYTES, CHUNK_MIN_BYTES, CHUNK_TARGET_BYTES, Chunker, RESYNC_BOUND_BYTES,
};
use f_blob::gear::{GEAR, MASK};
use f_env::split::{Stream, label};

/// The seeds every property below is asserted across.
///
/// Recorded rather than drawn, because a seed set that moves is a seed set
/// nobody can be asked to reproduce. A failure names the seed it happened on
/// and that seed is in this list.
const SEEDS: &[u64] = &[1, 2, 3, 4, 5, 6, 7, 8];

/// The window the register sees, in bytes.
///
/// Bit 63 of the register is the highest bit the mask tests, and bit *k*
/// depends on the last *k+1* bytes, so a boundary decision is a function of the
/// last sixty-four bytes. This is the number the prefix half of the bound is
/// stated in, and — added to [`CHUNK_MIN_BYTES`] — the width of the window RFC
/// 0061 makes acceptance a predicate over.
const WINDOW_BYTES: usize = 64;

/// The four content mixtures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mixture {
    Uniform,
    Zero,
    Periodic,
    Concatenated,
}

impl Mixture {
    const ALL: [Self; 4] = [Self::Uniform, Self::Zero, Self::Periodic, Self::Concatenated];

    fn name(self) -> &'static str {
        match self {
            Self::Uniform => "uniform",
            Self::Zero => "zero-filled",
            Self::Periodic => "periodic",
            Self::Concatenated => "concatenated",
        }
    }
}

/// The four draw sites, split by identity from one seed.
///
/// Split by identity and not by draw, so that adding a fifth site later leaves
/// the four that are here answering exactly what they answered before. RFC
/// 0026.
struct Sites {
    object: Stream,
    edit: Stream,
    length: Stream,
    mixture: Stream,
}

impl Sites {
    fn new(seed: u64) -> Self {
        let root = Stream::from_seed(label("e2-p02")).split(seed);
        Self {
            object: root.split(label("e2-p02/object")),
            edit: root.split(label("e2-p02/edit")),
            length: root.split(label("e2-p02/length")),
            mixture: root.split(label("e2-p02/mixture")),
        }
    }
}

/// A value in `0..n`, by remainder.
///
/// The bias a remainder leaves is at most one part in `u64::MAX / n`, which for
/// every `n` here is smaller than anything these properties measure. What the
/// draw has to be is reproducible, and it is.
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
/// # Why the period leaves the generator
///
/// RFC 0062 asserts a property of the *content* rather than of a mixture name,
/// and the property is stated in the period: on a `p`-periodic object the
/// candidate count at or after [`WINDOW_BYTES`] is either zero or at least
/// `len / p − 1`. A generator that kept `p` to itself would leave that
/// assertion with nothing to compare against, and the mixture label — which is
/// what the assertion this replaces was keyed to — is exactly the thing RFC
/// 0062 says decides nothing.
struct Drawn {
    /// The content.
    bytes: Vec<u8>,
    /// The period the content repeats at, in bytes, or `None` when it has no
    /// single one.
    ///
    /// Only [`Mixture::Periodic`] has one. A concatenation has a period per run
    /// and none of its own; uniform and zero-filled content have none to have —
    /// a zero run repeats at every period, which is a different fact and is the
    /// one `gear`'s fixed-point test carries.
    period: Option<usize>,
}

/// `len` bytes of the named mixture.
///
/// Two streams, and which draw goes to which is the point: `content` answers
/// the bytes, `mixture` answers what kind of content comes next inside a
/// concatenation. Splitting them means a longer object does not shift the
/// sequence of run kinds, and a fifth kind added later does not shift the
/// bytes.
fn draw(kind: Mixture, content: &mut Stream, mixture: &mut Stream, len: usize) -> Drawn {
    let stream = &mut *content;
    match kind {
        Mixture::Uniform => Drawn { bytes: uniform_bytes(stream, len), period: None },
        Mixture::Zero => Drawn { bytes: vec![0u8; len], period: None },
        Mixture::Periodic => {
            // Below the target size, which is the case that matters: a period
            // above it would put a candidate in most periods and behave like
            // uniform content.
            let period = WINDOW_BYTES + below(stream, CHUNK_TARGET_BYTES - WINDOW_BYTES);
            let block = uniform_bytes(stream, period);
            let mut out = Vec::with_capacity(len);
            while out.len() < len {
                let take = (len - out.len()).min(period);
                out.extend_from_slice(&block[..take]);
            }
            Drawn { bytes: out, period: Some(period) }
        }
        Mixture::Concatenated => {
            let mut out = Vec::with_capacity(len);
            while out.len() < len {
                let run = (CHUNK_TARGET_BYTES + below(mixture, 8 * CHUNK_TARGET_BYTES))
                    .min(len - out.len());
                let inner = Mixture::ALL[below(mixture, 3)];
                out.extend_from_slice(&draw(inner, content, mixture, run).bytes);
            }
            Drawn { bytes: out, period: None }
        }
    }
}

/// Every boundary the chunker puts in `bytes`, ending with the object's end.
///
/// The last entry is the end of the object and is not a decision the content
/// made; every property below that talks about chunk sizes excludes it.
fn boundaries(bytes: &[u8]) -> Vec<usize> {
    let mut chunker = Chunker::new();
    let mut out: Vec<usize> = chunker.feed(bytes).collect();
    let tail = chunker.finish();
    assert_eq!(
        out.last().copied().unwrap_or(0) + tail,
        bytes.len(),
        "the cuts and the tail do not account for every byte fed"
    );
    if tail > 0 || out.is_empty() {
        out.push(bytes.len());
    }
    out
}

/// Every position at which the register hits the mask.
///
/// A candidate is a property of the content and of the two constants, and of
/// nothing the chunker is doing at the time: the register is never reset at a
/// cut, so this sequence is the same whatever the boundaries turn out to be.
/// That is what lets the bound's second clause be stated over the object rather
/// than over a run of the algorithm.
///
/// **One mask and not the union of two.** RFC 0061: *"position `p` is a
/// candidate when `register(p) & MASK == 0` and for no other reason"*, because
/// normalised chunking's two masks were selected by the distance since the
/// previous boundary and that is the dependence the entry removes. There is
/// nothing generous left to read here — the chunker's own candidate test is
/// this line.
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

/// The candidate spacing at or above which a cut can be forced, in bytes.
///
/// RFC 0061 names the class the bound's second clause is about: *"a maximal run
/// in which no two consecutive candidates are closer than
/// `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` = 240 KiB"*. It is not an arbitrary
/// number and it is not tunable: a boundary is forced only after
/// [`CHUNK_MAX_BYTES`] with no accepted candidate, an accepted candidate needs
/// only [`CHUNK_MIN_BYTES`] of clearance behind it, so a gap this wide is
/// exactly the condition under which the forced cut can be reached at all.
const STARVED_GAP_BYTES: usize = CHUNK_MAX_BYTES - CHUNK_MIN_BYTES;

/// The end of the maximal *starved* run containing `at`.
///
/// Walk the candidates forward from the last one at or before `at`; while the
/// next is at least [`STARVED_GAP_BYTES`] beyond the previous, the run
/// continues; the first pair closer than that ends it, and the run ends at the
/// earlier member of that pair. If the candidates run out the run reaches the
/// end of the object. An answer **at or before `at`** means there is no starved
/// run at `at` at all, and the clause below collapses to the flat bound.
///
/// **This replaces `candidate_free_run_end`, and RFC 0061 is the sentence that
/// authorises the replacement**: *"the second clause covered content with no
/// candidates at all; it now covers content in which consecutive candidates are
/// never closer than `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` = 240 KiB, which is
/// exactly the condition under which a cut is forced."* That is a **narrowing**
/// of what the clause excuses on ordinary content — a candidate 2570 bytes past
/// the edit used to end the run and now does not even start one, so the clause
/// collapses to the flat bound and stops hiding a failure — while widening it
/// over genuinely sparse content, where the forced cut really does defeat
/// resynchronisation and the old wording did not say so.
///
/// # Why the walk starts behind `at` and not at it
///
/// RFC 0061's word is **maximal**, and a maximal run is a property of the
/// object's candidate sequence rather than of where the edit landed in it. A
/// walk seeded at `at` would ask *is the next candidate 240 KiB ahead of the
/// edit*, and the honest question is *is the gap the edit sits in 240 KiB
/// wide*: the same object edited ten kilobytes earlier would answer the first
/// question differently and the second identically, and it is the second that
/// decides whether a cut is forced. `candidate_free_run_end` had the same
/// semantics — its doc said *the run containing `at`* — so this is the reading
/// carried over, not a new one. Measured, on seed 7 uniform: the edit at
/// 846 791 sits in a 359 549-byte candidate gap running 656 108 → 1 015 657,
/// both streams force a cut inside it at different offsets, and a walk seeded
/// at `at` calls that content unstarved because the *next* candidate is only
/// 168 866 bytes ahead.
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

/// How far past `from` the sequences are allowed to take to agree again.
///
/// The bound as RFC 0061 restates it: [`RESYNC_BOUND_BYTES`] past `from`,
/// **or** the end of the enclosing starved run plus one chunk, whichever is
/// later. One chunk is [`CHUNK_MAX_BYTES`], because that is the largest a chunk
/// can be and the clause is about a region where every cut is forced at exactly
/// that. `RESYNC_BOUND_BYTES` did not move under RFC 0061 and neither did claim
/// 0017's threshold; what moved is which content the second clause excuses.
fn resync_allowance(candidates: &[usize], length: usize, from: usize) -> usize {
    let flat = from + RESYNC_BOUND_BYTES;
    let starved = starved_end(candidates, length, from) + CHUNK_MAX_BYTES;
    flat.max(starved)
}

/// The earliest position from which two boundary sequences agree under a shift.
///
/// `new` is `old` with `shift` bytes inserted, so after the sequences have
/// resynchronised every boundary in one is a boundary in the other displaced by
/// `shift`. The longest common suffix is what finds that, and it always finds
/// something: both sequences end at the object's end, which is the worst case
/// and means *nothing after the edit was preserved*.
fn resynchronised_at(old: &[usize], new: &[usize], shift: usize) -> usize {
    let mut i = old.len();
    let mut j = new.len();
    let mut at = new.last().copied().unwrap_or(0);
    while i > 0 && j > 0 && old[i - 1] + shift == new[j - 1] {
        at = new[j - 1];
        i -= 1;
        j -= 1;
    }
    at
}

/// A chunk hash and how many chunks carry it, keyed by hash.
///
/// `BTreeMap` and not the other one: RFC 0004, and this test is checked by the
/// same lint it would be violating.
fn chunk_census(bytes: &[u8], boundaries: &[usize]) -> BTreeMap<[u8; 32], (usize, usize)> {
    let mut census: BTreeMap<[u8; 32], (usize, usize)> = BTreeMap::new();
    let mut start = 0;
    for &end in boundaries {
        let hash = f_hash::sha256(&bytes[start..end]);
        let entry = census.entry(hash).or_insert((end - start, 0));
        entry.1 += 1;
        start = end;
    }
    census
}

/// An object of the drawn mixture, big enough for the bound to be able to fail.
///
/// Two megabytes and up. An object shorter than `RESYNC_BOUND_BYTES` past the
/// edit would let the sequences "agree" by running out of bytes, which is a
/// vacuous pass and the failure this apparatus is least able to see.
fn draw_object(sites: &mut Sites, kind: Mixture) -> Drawn {
    let length = 2 * 1024 * 1024 + below(&mut sites.length, 2 * 1024 * 1024);
    draw(kind, &mut sites.object, &mut sites.mixture, length)
}

/// Property 1. Every chunk but the last is inside the size bounds.
///
/// The minimum half now holds by construction and is asserted anyway. RFC
/// 0061's two lines: consecutive accepted positions `c` and `c'` have no
/// candidate in `[c' − CHUNK_MIN_BYTES, c')` and `c` is a candidate at or
/// before `c'`, so `c' − c ≥ CHUNK_MIN_BYTES`. A proof is a statement about the
/// rule and this is a statement about the code, and the two have been known to
/// differ — the suppression beside a forced cut is exactly where they would.
#[test]
fn every_chunk_but_the_last_lies_between_the_minimum_and_the_maximum() {
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let object = draw_object(&mut sites, kind).bytes;
            let cuts = boundaries(&object);

            let mut start = 0;
            for (index, &end) in cuts.iter().enumerate() {
                let size = end - start;
                let last = index + 1 == cuts.len();
                assert!(
                    last || (CHUNK_MIN_BYTES..=CHUNK_MAX_BYTES).contains(&size),
                    "seed {seed}, {} mixture, object {} bytes: chunk {index} is {size} bytes, \
                     outside [{CHUNK_MIN_BYTES}, {CHUNK_MAX_BYTES}]",
                    kind.name(),
                    object.len()
                );
                start = end;
            }
        }
    }
}

/// The fewest interior chunks the whole draw must produce.
///
/// `claims/0018`'s geometry row `interior_chunks_measured = { min = 500 }`, and
/// the reason every mean below is a distribution rather than an anecdote: a
/// draw that quietly shrank its objects would report a mean over a handful of
/// chunks and call it a distribution, which is the failure `claims/0014` names
/// in its own geometry row. Measured 859.
const INTERIOR_CHUNKS_MIN: usize = 500;

/// The fewest interior chunks the zero-filled mixture must produce on its own.
///
/// `claims/0018`'s `zero_filled_interior_chunks = { min = 87 }`, and the
/// positive control the assertion beside it needs: *every interior chunk of
/// candidate-free content is forced at the maximum* is satisfied vacuously by a
/// draw that produced none. Measured 87, which is what eight objects of two to
/// four megabytes cut at a fixed [`CHUNK_MAX_BYTES`] produce, so the number is
/// the draw's geometry rather than a description of the run.
const ZERO_FILLED_INTERIOR_CHUNKS_MIN: usize = 87;

/// The most interior chunks that may be forced at the maximum on uniform
/// content, per ten thousand.
///
/// RFC 0061's third reversal condition as the number `claims/0018` registers
/// it: *"the forced-cut fraction on uniform content exceeding one interior
/// chunk in ten"* means retiring normalised chunking cost more than the bound
/// bought, and the repair named there is content-only normalisation in a
/// further RFC — never this ceiling moved. The entry declined to invent the
/// threshold before the first measurement and said it gets one when it is
/// measured; it is measured at 166 per ten thousand, and this constant is
/// where the condition can fire.
const FORCED_PER_TEN_THOUSAND_UNIFORM_MAX: usize = 1000;

/// The same ceiling for the two mixtures that cut on content less often.
///
/// `claims/0018`: *"the two mixtures that cut on content less often get the
/// same ceiling rather than a looser one, because a ceiling per mixture chosen
/// after seeing the run is a description."* Measured 2148 periodic and 2222
/// concatenated. Zero-filled content is not in this class and gets no ceiling
/// here: it is asserted at exactly ten thousand per ten thousand below, which
/// is the stronger statement and a different one.
const FORCED_PER_TEN_THOUSAND_STARVING_MAX: usize = 5000;

/// Property 2. The mean chunk is within a factor of two of the target.
///
/// Asserted over the whole draw, which is what the target is a statement about.
/// The zero-filled mixture's own mean is exactly [`CHUNK_MAX_BYTES`] — four
/// times the target — because that content has no candidates and every cut is
/// forced, and that is asserted here as the specific fact it is rather than
/// left to be averaged away. A mixture whose mean drifted to the maximum
/// *without* being candidate-free would pass the aggregate and fail this.
///
/// # Every threshold `claims/0018` gates is asserted here, and until 2026-09-06
/// most were not
///
/// That claim is `status = "gating"` and its own header says this property is
/// where its thresholds are asserted. Counted exactly, one of its eleven rows
/// was: the aggregate band. A second, the zero-filled mixture's forced count,
/// was asserted as a *ratio* — forced equals that mixture's own chunk count —
/// which is not the row's `min = 87` and which implied the zero-filled mean
/// without writing it down. The other eight rows — the three per-mixture means,
/// the three forced-cut ceilings, the zero-filled positive control and the
/// geometry row — were *printed* and compared by whoever read the output.
/// `CONTRIBUTING.md`'s rule is that a check listed as mechanised and not
/// mechanised is worse than one honestly listed as review, *"because it is a
/// check somebody believes is happening"*: RFC 0061's third reversal condition
/// — one interior chunk in ten forced at the maximum on uniform content — could
/// have fired in a green run and nothing would have said so.
/// Every row is asserted below against a named constant, and each constant's
/// doc comment carries the sentence it was derived from rather than the
/// measurement it sits above.
#[test]
fn the_mean_chunk_is_within_a_factor_of_two_of_the_target() {
    let mut total_bytes = 0usize;
    let mut total_chunks = 0usize;
    let mut per_mixture = Vec::new();

    for kind in Mixture::ALL {
        let mut bytes = 0usize;
        let mut chunks = 0usize;
        let mut forced_at_the_maximum = 0usize;

        for &seed in SEEDS {
            let mut sites = Sites::new(seed);
            let object = draw_object(&mut sites, kind).bytes;
            let cuts = boundaries(&object);
            let mut start = 0;
            // The final chunk is where the object ended, not where the content
            // said to cut, so it is not evidence about the target.
            for &end in &cuts[..cuts.len().saturating_sub(1)] {
                bytes += end - start;
                chunks += 1;
                if end - start == CHUNK_MAX_BYTES {
                    forced_at_the_maximum += 1;
                }
                start = end;
            }
        }

        assert!(chunks > 0, "the {} mixture produced no interior chunk to measure", kind.name());
        // Per ten thousand, because integer arithmetic and a percentage would
        // round the interesting cases to zero. RFC 0061 left this number
        // without a threshold — *"inventing one before the first measurement is
        // how a threshold becomes a description; it gets one when it is
        // measured"* — and it is now measured, so the ceilings below are the
        // ones `claims/0018` registered from that run rather than new numbers.
        let forced_per_ten_thousand = forced_at_the_maximum * 10_000 / chunks;
        let mean = bytes / chunks;
        println!(
            "P2 {:>12} mean {mean} over {chunks} interior chunks, {forced_at_the_maximum} forced \
             at {CHUNK_MAX_BYTES} ({forced_per_ten_thousand} per ten thousand)",
            kind.name()
        );

        match kind {
            // The mixture that is an assertion rather than an average, in three
            // parts: every interior chunk forced, the mean that follows from
            // that, and the count that stops both from being vacuous.
            Mixture::Zero => {
                assert_eq!(
                    forced_at_the_maximum, chunks,
                    "zero-filled content has no candidates, so every one of its {chunks} interior \
                     chunks must be forced at {CHUNK_MAX_BYTES} bytes; {forced_at_the_maximum} \
                     were. A cut that is not forced here means the register's fixed point on a \
                     zero run hits a mask, which changes what the bound's second clause is about"
                );
                assert_eq!(
                    mean, CHUNK_MAX_BYTES,
                    "the zero-filled mixture's mean interior chunk is {mean} bytes and \
                     `claims/0018` states it as exactly {CHUNK_MAX_BYTES}, both bounds, because \
                     that row is an assertion about candidate-free content and not an average \
                     over it"
                );
                assert!(
                    chunks >= ZERO_FILLED_INTERIOR_CHUNKS_MIN,
                    "the zero-filled mixture produced {chunks} interior chunks, below the \
                     {ZERO_FILLED_INTERIOR_CHUNKS_MIN} `claims/0018` requires as the positive \
                     control for the two assertions above. Without it a draw that produced no \
                     zero-filled interior chunk at all would satisfy *every one of them is \
                     forced* by having none"
                );
            }
            // The mixture the target size is a statement about, and the one
            // RFC 0061's forced-cut reversal condition is stated over.
            Mixture::Uniform => {
                assert!(
                    (CHUNK_TARGET_BYTES / 2..=CHUNK_TARGET_BYTES * 2).contains(&mean),
                    "the uniform mixture's mean interior chunk is {mean} bytes, outside the \
                     factor-of-two band [{}, {}] `claims/0018` gates it in. This is the mixture \
                     the target is a statement about, so the aggregate band holding while this \
                     one does not means the aggregate is being carried by content the chunker \
                     cannot cut",
                    CHUNK_TARGET_BYTES / 2,
                    CHUNK_TARGET_BYTES * 2
                );
                assert!(
                    forced_per_ten_thousand <= FORCED_PER_TEN_THOUSAND_UNIFORM_MAX,
                    "{forced_per_ten_thousand} interior chunks per ten thousand are forced at \
                     {CHUNK_MAX_BYTES} on uniform content, above the \
                     {FORCED_PER_TEN_THOUSAND_UNIFORM_MAX} that is RFC 0061's third reversal \
                     condition — one interior chunk in ten. Retiring normalised chunking cost \
                     more than the bound bought, and the repair the entry names is a content-only \
                     normalisation in a further RFC: nested masks, a loose hit accepted only when \
                     none occurred in the preceding {CHUNK_TARGET_BYTES} bytes. It is not this \
                     ceiling"
                );
            }
            // The two mixtures whose means RFC 0061's second reversal condition
            // is stated over: below twice `CHUNK_MIN_BYTES` and `MASK_BITS` is
            // the wrong width. They have no upper bound in `claims/0018`,
            // because content the chunker cannot cut is *expected* to sit above
            // the target and the aggregate band is where that is priced.
            Mixture::Periodic | Mixture::Concatenated => {
                assert!(
                    mean >= 2 * CHUNK_MIN_BYTES,
                    "the {} mixture's mean interior chunk is {mean} bytes, below the {} \
                     `claims/0018` gates it above. That is RFC 0061's second reversal condition: \
                     `MASK_BITS` = 16 is the wrong width, the repair is the width and not the \
                     rule, and a width chosen after seeing this test is a fitted constant that \
                     needs its own entry saying so",
                    kind.name(),
                    2 * CHUNK_MIN_BYTES
                );
                assert!(
                    forced_per_ten_thousand <= FORCED_PER_TEN_THOUSAND_STARVING_MAX,
                    "{forced_per_ten_thousand} interior chunks per ten thousand are forced at \
                     {CHUNK_MAX_BYTES} on the {} mixture, above the \
                     {FORCED_PER_TEN_THOUSAND_STARVING_MAX} `claims/0018` gates it under. The \
                     ceiling is the same one uniform content gets a tenth of, because a ceiling \
                     per mixture chosen after seeing the run is a description",
                    kind.name()
                );
            }
        }

        per_mixture.push((kind, mean, chunks));
        total_bytes += bytes;
        total_chunks += chunks;
    }

    let mean = total_bytes / total_chunks;
    println!("P2 aggregate mean {mean} over {total_chunks} interior chunks");
    assert!(
        total_chunks >= INTERIOR_CHUNKS_MIN,
        "the draw produced {total_chunks} interior chunks, below the {INTERIOR_CHUNKS_MIN} \
         `claims/0018` requires for a mean over them to be a distribution. The system is fine and \
         the measurement has stopped measuring: the objects got smaller or the seeds got fewer"
    );
    assert!(
        (CHUNK_TARGET_BYTES / 2..=CHUNK_TARGET_BYTES * 2).contains(&mean),
        "mean chunk {mean} bytes over {total_chunks} chunks is not within a factor of two of \
         {CHUNK_TARGET_BYTES}; per mixture: {}",
        per_mixture
            .iter()
            .map(|(kind, mean, count)| format!("{} {mean} over {count}", kind.name()))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Property 3. Nothing before the last boundary preceding `X − 64` changes.
///
/// Exactly, and not approximately: the two prefixes are compared as sequences.
/// This is the half of the bound that is hard rather than probabilistic, and a
/// failure here is a failure of the window argument itself.
#[test]
fn nothing_before_the_last_boundary_preceding_the_window_changes() {
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let object = draw_object(&mut sites, kind).bytes;
            // In the first half, so that the object always has a tail longer
            // than the resynchronisation bound.
            let at = below(&mut sites.edit, object.len() / 2);
            let length = 1 + below(&mut sites.length, 128 * 1024);
            let inserted = uniform_bytes(&mut sites.edit, length);

            let mut edited = Vec::with_capacity(object.len() + length);
            edited.extend_from_slice(&object[..at]);
            edited.extend_from_slice(&inserted);
            edited.extend_from_slice(&object[at..]);

            let before = boundaries(&object);
            let after = boundaries(&edited);
            let frontier = at.saturating_sub(WINDOW_BYTES);
            let kept = before.iter().take_while(|&&b| b <= frontier).count();

            assert_eq!(
                before[..kept],
                after[..kept.min(after.len())],
                "seed {seed}, {} mixture, object {} bytes, edit of {length} bytes at {at}: the \
                 {kept} boundaries at or before {frontier} are not identical, so a decision taken \
                 outside the {WINDOW_BYTES}-byte window saw the edit",
                kind.name(),
                object.len()
            );
        }
    }
}

/// The most of the eight periodic pairs that may have their edit inside a
/// starved run.
///
/// Seven, and it is derived rather than measured — measured is five. On
/// `p`-periodic content the candidate set is all-or-nothing with
/// `P(empty) = e^(−p / 2^MASK_BITS)`, which averages 0.632 over a period drawn
/// uniformly from `[64, CHUNK_TARGET_BYTES)`, so a *correct* mask starves all
/// eight with probability `0.632^8` = 2.6%. Seven is therefore the largest
/// threshold eight seeds can carry without becoming a description of the run.
/// RFC 0062, and its failure mode is the regression that entry documents: a
/// mask widened without anybody noticing what it does to short periods —
/// `MASK_BITS` 14 to 16 moved `P(empty)` from 0.245 to 0.632. Raising this
/// number needs more seeds, not a bigger number.
const STARVED_PERIODIC_MAX: usize = 7;

/// The fewest pairs of the thirty-two whose edit must fall *outside* a starved
/// run.
///
/// Four, as the positive control that stops the tight clause and the carriage
/// check above from passing vacuously — a suite in which everything is starved
/// asserts nothing. Derived: a uniform edit sits in a [`STARVED_GAP_BYTES`]
/// candidate gap with probability `e^(−3.75)` = 2.4%, so the eight uniform
/// seeds alone put four out of reach of anything but a broken generator.
/// Measured 13. RFC 0062.
const UNSTARVED_MIN: usize = 4;

/// The number of (seed, mixture) pairs the properties are asserted over.
const PAIRS: usize = SEEDS.len() * Mixture::ALL.len();

/// The fewest pairs whose allowance must reach past the edited object's own
/// end, where the published bound is unfalsifiable.
///
/// Eight, exactly, and derived from the same fixed point as the zero-filled row
/// below: zero-filled content has no candidate anywhere, so the starved run
/// containing the edit reaches the object's end, the allowance is that end plus
/// [`CHUNK_MAX_BYTES`], and `agreed <= allowed` cannot fail because `agreed` is
/// at most the end. A count below eight is the same event `gear`'s fixed-point
/// test guards from the other side.
const ALLOWANCE_PAST_THE_OBJECT_MIN: usize = SEEDS.len();

/// The most pairs whose allowance may reach past the edited object's own end.
///
/// Derived from [`UNSTARVED_MIN`] and not measured — measured is 14. A pair
/// whose edit is outside a starved run has `starved_end <= from`, so its
/// starved clause lands at most [`CHUNK_MAX_BYTES`] past the edit and the flat
/// clause carries the allowance; the edit is in the object's first half and the
/// insertion is under 128 KiB, so that allowance is below `len / 2 + 640 KiB`
/// and an object of at least two megabytes ends after it. Every unstarved pair
/// is therefore falsifiable, and at most `PAIRS − UNSTARVED_MIN` are not.
const ALLOWANCE_PAST_THE_OBJECT_MAX: usize = PAIRS - UNSTARVED_MIN;

/// Property 4. The sequences resynchronise inside the two-clause bound.
///
/// [`RESYNC_BOUND_BYTES`] past `X + L`, or the end of the enclosing *starved*
/// run plus one chunk, whichever is later. The second clause is not a hedge for
/// this test to be generous with: it is computed from the edited object's own
/// candidate sequence, so on content whose candidates are closer together than
/// [`STARVED_GAP_BYTES`] it collapses to the flat bound.
///
/// # Four assertions, and which sentence bought each
///
/// **The published bound.** `agreed <= allowed`, on every pair. Nothing about
/// it moved under RFC 0061 or RFC 0062, and it holds 32 of 32 — of which 18 are
/// pairs where it could have failed. See the section below before quoting the
/// 32 anywhere.
///
/// **The tighter clause where it applies.** RFC 0061: *"the flat clause gets
/// stronger where it applies: outside a starved run the sequences agree from
/// the first accepted boundary at or after `X + L + CHUNK_MIN_BYTES + 64` —
/// 16 448 bytes past the edit — rather than merely inside
/// `RESYNC_BOUND_BYTES`, because every acceptance decision from that point on
/// reads only content the edit did not touch."* That is the number the entry's
/// whole argument produces, and asserting only the 512 KiB outer allowance
/// would leave it unmeasured. It applies on 13 pairs and is violated on none.
///
/// **Carriage, keyed to the candidate gap and not to a mixture name.** What
/// stood here until 2026-09-06 was RFC 0061's requirement that *every* periodic
/// and concatenated pair land inside the flat clause. That requirement is
/// withdrawn — not weakened, withdrawn, by an entry that explains why it was
/// never satisfiable — and RFC 0062 is the sentence that replaces it: *"a pair
/// whose edit is not inside a starved run must be carried by the flat clause.
/// The mixture name decides nothing; the candidate gap the edit sits in decides
/// everything."* A reader should know what this check is and is not: outside a
/// starved run the allowance *is* the flat clause, so on those pairs this and
/// the published bound above coincide, and the value of stating it separately
/// is the message — a pair excused by a starved run it is not in would be
/// reported as the excuse it is rather than as a bound that held. The teeth
/// RFC 0062 adds are the two below.
///
/// **The structure that produces the split.** RFC 0062: *"On the drawn periodic
/// object — not the edited one, whose inserted uniform bytes contribute their
/// own candidates — count the candidates at positions at or after 64, where the
/// register is a pure function of `i mod p`, and assert that the count is
/// either zero or at least `len / p − 1`."* This is the all-or-nothing property
/// the whole starved class rests on, and it can fail: the day the register
/// stops being a function of the last sixty-four bytes, a periodic object
/// acquires a candidate count that is neither — and that is the same day the
/// *prefix* half of the published bound stops being true, which is why this
/// assertion is worth more than the distribution it replaced. The eight *edited*
/// periodic objects measure 41, 1, 1, 0, 77, 3, 83 and 1 candidates; the
/// assertion is what says the middle counts are the insertion's uniform bytes
/// and not the content's, because a period below 65 536 in an object above two
/// megabytes puts at least 32 candidates in it the moment one residue hits.
///
/// # The effective sample, and why *32 of 32* on its own overstates the run
///
/// On 14 of the 32 pairs the allowance reaches past the edited object's own end
/// — 8 zero-filled, 5 periodic, 1 concatenated — so `agreed > allowed` is
/// unsatisfiable there, `agreed` is at most the object's end by construction,
/// and on every one of those 14 it *is* the object's end: nothing after the
/// edit was preserved. The starved clause producing that allowance is RFC
/// 0062's decision and is not in question here. What is in question is the
/// arithmetic a reviewer needs in order to weigh it, and until 2026-09-06 the
/// count existed nowhere while *32 of 32, zero violations* was in this file, in
/// `claims/0017`, in RFC 0061's and 0062's confirming runs and in `TODO.md`.
/// The published bound is therefore exercised on 18 pairs and vacuous on 14;
/// the count is printed, thresholded against
/// [`ALLOWANCE_PAST_THE_OBJECT_MIN`] and [`ALLOWANCE_PAST_THE_OBJECT_MAX`], and
/// the effective sample is stated beside the 32 wherever the 32 is published.
/// `draw_object`'s own doc comment names this failure mode — *"the sequences
/// agree by running out of bytes, which is a vacuous pass and the failure this
/// apparatus is least able to see"* — and drawing bigger objects does not
/// remove it, because on candidate-free content the starved run grows with the
/// object.
///
/// # The split as counted data, with three thresholds
///
/// Which pairs are starved is recorded per mixture and asserted against
/// [`STARVED_PERIODIC_MAX`], the exact count of zero-filled pairs, and
/// [`UNSTARVED_MIN`]. None of the three is the measured value, each is derived
/// in its own doc comment, and together they are what `claims/0017` registers in
/// place of `resync_pairs_carried_by_the_flat_clause`.
///
/// # Why the failures are collected rather than panicked on
///
/// The per-pair checks record and none of them stop the loop, and the test fails
/// at the end on the collected list. A bound that fails is a fact about the
/// design and the useful form of that fact is *which (seed, mixture) pairs* —
/// the first panic tells a reader one pair and hides the other thirty-one, which
/// is how the draft bound survived as long as it did. Nothing is excused by
/// this: one entry in the list fails the test. The three thresholds are asserted
/// after that list, because the split is a statement about the content and is
/// only interpretable once the bound itself has held.
#[test]
fn the_boundary_sequences_resynchronise_within_the_bound() {
    let mut failures: Vec<String> = Vec::new();
    // The split, counted rather than described, and keyed by mixture so that
    // the three thresholds below are statements about named content. RFC 0062
    // makes these counts the rows `claims/0017` registers.
    let mut starved_pairs: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut unstarved_pairs = 0usize;
    // The pairs on which the headline assertion cannot fail, counted rather
    // than left for a reader to derive from the printed lines. See the doc
    // comment's section on the effective sample.
    let mut allowance_past_the_object = 0usize;
    // The three counts `claims/0017` registers that this property computed and
    // never named. **They were asserted here and unreadable from outside**,
    // which is how a threshold row comes to be a row nothing compares against:
    // `cargo xtask claim bytes-rechunked-per-byte` parses `name value` lines out
    // of this run and out of `bench/src/bin/rechunk.rs`, and a row neither of
    // them prints is a row the registry cannot check. Nothing here is asserted
    // differently for being printed — the assertions below are the ones that
    // were already here, unchanged.
    let mut within_the_published_bound = 0usize;
    let mut unstarved_missing_the_tight_clause = 0usize;
    let mut resync_bytes_max = 0usize;
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let Drawn { bytes: object, period } = draw_object(&mut sites, kind);
            let at = below(&mut sites.edit, object.len() / 2);
            let length = 1 + below(&mut sites.length, 128 * 1024);
            let inserted = uniform_bytes(&mut sites.edit, length);

            let mut edited = Vec::with_capacity(object.len() + length);
            edited.extend_from_slice(&object[..at]);
            edited.extend_from_slice(&inserted);
            edited.extend_from_slice(&object[at..]);

            let before = boundaries(&object);
            let after = boundaries(&edited);
            let agreed = resynchronised_at(&before, &after, length);
            // The **drawn** object's candidate set, taken here because the next
            // line turns `candidates` from a function into a binding, and only
            // for content that has a period — the structural clause below is a
            // statement about `i mod p` and there is nothing to say without a
            // `p`.
            let drawn_candidates = period.map(|_| candidates(&object));
            let candidates = candidates(&edited);
            let from = at + length;
            let flat = from + RESYNC_BOUND_BYTES;
            let starved = starved_end(&candidates, edited.len(), from);
            let allowed = resync_allowance(&candidates, edited.len(), from);
            let carried = if agreed <= flat { "flat" } else { "starved" };

            // The window the acceptance decision reads, placed past the edit:
            // from here on every decision in the edited stream sees only bytes
            // the edit did not touch, in both streams.
            let window_clear = from + CHUNK_MIN_BYTES + WINDOW_BYTES;
            let tight = after.iter().copied().find(|&b| b >= window_clear).unwrap_or(edited.len());

            // The drawn period is printed beside the candidate count, and RFC
            // 0062 asks for it by name so that `e^(−p / 2^MASK_BITS)` — the
            // probability that a periodic object has no candidate at all — can
            // be checked against the run by hand rather than taken on trust.
            let drawn_period =
                period.map_or_else(|| "none".to_string(), |period| format!("{period}"));
            // `agreed` is at most the object's end — `resynchronised_at`
            // returns the last boundary of the edited stream when nothing after
            // the edit survives — so an allowance at or past that end makes
            // `agreed > allowed` unsatisfiable and this pair contributes
            // nothing to the published bound. Printed per pair and counted, so
            // that the run reports the sample it actually tested.
            let vacuous = allowed >= edited.len();
            if vacuous {
                allowance_past_the_object += 1;
            }
            println!(
                "P4 seed {seed} {:>12} object {} drawn-period {drawn_period} edit {length}@{at} \
                 candidates {} agreed {agreed} flat {flat} starved-run-end {starved} allowed \
                 {allowed} tight {tight} carried-by {carried} bound-falsifiable {}",
                kind.name(),
                object.len(),
                candidates.len(),
                !vacuous
            );

            let pair = format!(
                "seed {seed}, {} mixture, object {} bytes, insertion of {length} bytes at {at}",
                kind.name(),
                object.len()
            );

            if agreed <= allowed {
                within_the_published_bound += 1;
            }
            // The outer allowance on its own, over the pairs it is stated over:
            // a starved pair is allowed to exceed it by the second clause, so a
            // maximum taken across both classes would report an excused number
            // under a threshold that cannot apply to it.
            if starved <= from {
                resync_bytes_max = resync_bytes_max.max(agreed.saturating_sub(from));
            }

            if agreed > allowed {
                failures.push(format!(
                    "{pair}: the boundary sequences agree again only at {agreed}, past the \
                     {allowed} the bound allows ({flat} flat, starved run to {starved} plus one \
                     chunk){}",
                    if agreed == edited.len() {
                        " — and that is the object's end, so nothing after the edit was preserved"
                    } else {
                        ""
                    }
                ));
            }

            // Carriage, restated over content instead of over labels. RFC 0062:
            // *"a pair whose edit is not inside a starved run must be carried by
            // the flat clause. The mixture name decides nothing; the candidate
            // gap the edit sits in decides everything."* This sits where the
            // withdrawn requirement sat — every periodic and concatenated pair
            // carried by the flat clause — so that a reader who comes looking
            // for it finds what is true rather than nothing.
            if starved <= from && carried != "flat" {
                failures.push(format!(
                    "{pair}: the edit is not inside a starved run — the run containing it ends at \
                     {starved}, at or before {from} — so the starved clause excuses nothing here \
                     and the flat clause's {flat} is the whole allowance. The sequences agreed at \
                     {agreed}, on an allowance of {allowed} ({} candidates in the whole edited \
                     object)",
                    candidates.len()
                ));
            }

            // `starved <= from` is the whole of "outside a starved run": the
            // run containing the edit ends at or before it, so no forced cut
            // can separate the two streams past this point and the window
            // argument applies unaided.
            if starved <= from && agreed > tight {
                unstarved_missing_the_tight_clause += 1;
                failures.push(format!(
                    "{pair}: the edit is not inside a starved run, so every acceptance decision \
                     at or after {window_clear} reads only untouched content and the sequences \
                     owe agreement by the first boundary there, which is {tight}. They agreed at \
                     {agreed}"
                ));
            }

            if starved > from {
                *starved_pairs.entry(kind.name()).or_default() += 1;
            } else {
                unstarved_pairs += 1;
            }

            // The structure that produces the split, asserted on the **drawn**
            // object rather than on the edited one. RFC 0062: *"count the
            // candidates at positions at or after 64, where the register is a
            // pure function of `i mod p`, and assert that the count is either
            // zero or at least `len / p − 1`."* The lower bound is exact rather
            // than generous: one residue hitting the mask puts a candidate at
            // every position congruent to it, `floor((len − r) / p) + 1` of
            // them in the object, and dropping the at most one that falls
            // inside the first sixty-four bytes leaves `floor(len / p) − 1` in
            // the worst case.
            if let (Some(period), Some(drawn)) = (period, drawn_candidates) {
                let settled = drawn.iter().filter(|&&c| c >= WINDOW_BYTES).count();
                let owed = (object.len() / period).saturating_sub(1);
                println!(
                    "P4 seed {seed} {:>12} drawn object {} period {period} candidates at or \
                     after {WINDOW_BYTES}: {settled}, all-or-nothing owes 0 or at least {owed}",
                    kind.name(),
                    object.len()
                );
                if settled > 0 && settled < owed {
                    failures.push(format!(
                        "{pair}: the drawn object repeats with period {period}, so past the \
                         first {WINDOW_BYTES} bytes the register is a function of the position \
                         modulo {period} and its candidate set is closed under adding {period}. \
                         That set must then be empty or hold at least {owed} positions; it holds \
                         {settled}. The register has stopped being a function of the last \
                         {WINDOW_BYTES} bytes, which puts the prefix half of the bound in \
                         question before anything about the starved clause is"
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} recorded failures over {PAIRS} (seed, mixture) pairs against the re-chunking bound as \
         RFC 0061 states it and RFC 0062 restates what it covers, of which {} pairs could fail its \
         headline clause at all:\n{}",
        failures.len(),
        PAIRS - allowance_past_the_object,
        failures.join("\n")
    );

    let split = starved_pairs
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    let starved_periodic = starved_pairs.get(Mixture::Periodic.name()).copied().unwrap_or(0);
    let starved_zero = starved_pairs.get(Mixture::Zero.name()).copied().unwrap_or(0);
    println!(
        "P4 split: {unstarved_pairs} of {PAIRS} pairs unstarved, starved per mixture: {split}"
    );
    println!(
        "P4 effective sample: the published bound could have failed on {} of {PAIRS} pairs; on \
         {allowance_past_the_object} the allowance reaches past the edited object's end and \
         `agreed <= allowed` is unsatisfiable",
        PAIRS - allowance_past_the_object
    );

    // The rows `claims/0017` registers, under the names it registers them
    // under, one per line so that the claim's reproduction can compare them
    // rather than trust that an assertion somewhere below did — and printed
    // *before* the three thresholds are asserted, so that a red threshold
    // reports the row that is red rather than only a panic.
    println!("resync_bytes_max {resync_bytes_max}");
    println!("resync_pairs_within_the_published_bound {within_the_published_bound}");
    println!("resync_pairs_where_the_allowance_exceeds_the_object {allowance_past_the_object}");
    println!(
        "resync_pairs_unstarved_missing_the_tight_clause {unstarved_missing_the_tight_clause}"
    );
    println!("resync_pairs_unstarved {unstarved_pairs}");
    println!("resync_pairs_starved_periodic {starved_periodic}");
    println!("resync_pairs_starved_zero_filled {starved_zero}");

    // The effective sample, thresholded from both sides. Neither number is the
    // measured 14: the floor is the exact count of zero-filled pairs, which are
    // candidate-free by the constraint on the mask's draw, and the ceiling is
    // what `UNSTARVED_MIN` leaves once every unstarved pair is falsifiable.
    assert!(
        allowance_past_the_object >= ALLOWANCE_PAST_THE_OBJECT_MIN,
        "the allowance reaches past the object's end on only {allowance_past_the_object} of \
         {PAIRS} pairs, below the {ALLOWANCE_PAST_THE_OBJECT_MIN} that is the exact count of \
         zero-filled pairs. Zero-filled content has no candidate anywhere, so its starved run \
         reaches the end and its allowance is that end plus {CHUNK_MAX_BYTES}; a count below \
         eight means that content has acquired a candidate, which is the same event the \
         zero-filled assertion above and `gear`'s fixed-point test both guard, and every object \
         hash ever written has changed. Split: {split}"
    );
    assert!(
        allowance_past_the_object <= ALLOWANCE_PAST_THE_OBJECT_MAX,
        "the allowance reaches past the object's end on {allowance_past_the_object} of {PAIRS} \
         pairs, above the {ALLOWANCE_PAST_THE_OBJECT_MAX} left by the {UNSTARVED_MIN} pairs that \
         must be unstarved — an unstarved pair's allowance is the flat clause and an object of \
         two megabytes and up ends after it, so it is falsifiable by construction. The published \
         bound is being asserted on a sample this small because the draw has stopped producing \
         content the chunker can cut, and reporting it as {PAIRS} of {PAIRS} would be reporting \
         more than the run measured. Split: {split}"
    );

    assert!(
        starved_periodic <= STARVED_PERIODIC_MAX,
        "{starved_periodic} of the {} periodic pairs are starved, above the \
         {STARVED_PERIODIC_MAX} RFC 0062 allows. At eight, `MASK_BITS` or the gear table has \
         drifted and the chunked kind has stopped serving periodic content altogether rather \
         than serving a third of it — a correct mask reaches eight with probability 0.632^8 = \
         2.6%. The threshold is derived; raising it needs more seeds. Split: {split}",
        SEEDS.len()
    );
    assert_eq!(
        starved_zero,
        SEEDS.len(),
        "{starved_zero} of the {} zero-filled pairs are starved, and the count owed is exact \
         rather than statistical: a zero run reaches the register's fixed point after \
         {WINDOW_BYTES} bytes, that fixed point hits no mask by the constraint RFC 0061 put on \
         the mask's draw, so zero-filled content has no candidate anywhere and every edit in it \
         is inside a starved run. A count below this is the same event `gear`'s fixed-point test \
         guards from the other side, and it has changed every object hash ever written. Split: \
         {split}",
        SEEDS.len()
    );
    assert!(
        unstarved_pairs >= UNSTARVED_MIN,
        "only {unstarved_pairs} of {PAIRS} pairs have their edit outside a starved run, below the \
         {UNSTARVED_MIN} RFC 0062 requires as the positive control. The tight clause and the \
         carriage check above are both conditioned on that, so a run in which everything is \
         starved asserts nothing about the bound it claims to measure. Split: {split}"
    );
}

/// The fewest deduplication pairs whose requirement must be non-zero.
///
/// Four, and derived the way [`UNSTARVED_MIN`] is rather than measured —
/// measured is 19. The requirement is zeroed exactly when the allowance covers
/// the whole two-megabyte shared run, which needs a starved run reaching from
/// the pad to within [`CHUNK_MAX_BYTES`] of the object's end; the eight uniform
/// pairs are drawn from content whose candidates arrive about every
/// [`CHUNK_TARGET_BYTES`], so a single [`STARVED_GAP_BYTES`] gap there has
/// probability `e^(−3.75)` = 2.4% and a chain of them spanning the object has
/// none worth writing down. Four of those eight is therefore a threshold
/// nothing but a broken generator reaches, and it is the positive control that
/// stops this property from passing on a draw it asserts nothing about.
const DEDUP_REQUIRING_MIN: usize = 4;

/// Property 5. Identical content in two objects yields identical chunk hashes.
///
/// The deduplication half. A run of content is placed at two different offsets
/// in two objects, and the bytes it covers must end up in chunks with the same
/// names — up to the same two-clause allowance, because the run has to
/// resynchronise before its chunks can agree, and inside a starved run it never
/// does. The allowance is [`resync_allowance`] and therefore RFC 0061's starved
/// clause, not the candidate-free one it replaced.
///
/// A reader should also know what a small shortfall here is *not* evidence of.
/// RFC 0061: deduplication is a byte-weighted average over a two-megabyte run
/// and resynchronisation is a statement about a single position, so a defect
/// that makes every boundary after an edit wrong can cost this average under
/// one per cent. This property and property 4 are not checks on each other,
/// which is why `E2-P02` asserts both.
///
/// # What this property owes on a starved pair, and why it is a count
///
/// `owed` is `shared.len()` less the allowance, so on a pair whose allowance
/// reaches two megabytes it is zero and `agreed >= owed` holds for `agreed = 0`.
/// That is 13 of the 32 pairs — 8 zero-filled and 5 periodic — and on 5 of them
/// the measured deduplication *is* zero: two placements of the same periodic run
/// at different offsets share no chunk at all, because every cut in both is
/// forced at [`CHUNK_MAX_BYTES`] and the two forced orbits differ in phase.
/// Reporting that as a pass, on the workload the deduplication half of `E2-B01`
/// is worst at, is the shape this file was audited for.
///
/// The repair is a count with a threshold and **not** a floor, and the reason is
/// RFC 0062's theorem rather than convenience: on content whose period is below
/// [`CHUNK_MIN_BYTES`] no boundary rule of this design produces a content
/// boundary, so zero really is what the design deduplicates there and any floor
/// above it would be a number the chunker cannot meet — invented to give a
/// vacuous pass the look of an assertion. What can be asserted is that the
/// vacuous pairs stay a minority of a known size: [`DEDUP_REQUIRING_MIN`] pairs
/// must carry a non-zero requirement, the per-pair line says which are which,
/// and the summary states the effective sample beside the 32. Measured 19 of 32
/// with a non-zero requirement, 5 deduplicating nothing.
#[test]
fn identical_content_in_two_objects_yields_identical_chunk_hashes() {
    // The effective sample, and the count of pairs on which the design's worst
    // case is visible rather than averaged. Both are printed at the end; the
    // first is thresholded.
    let mut requiring_pairs = 0usize;
    let mut deduplicating_nothing = 0usize;
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let shared = draw(kind, &mut sites.object, &mut sites.mixture, 2 * 1024 * 1024).bytes;
            let first_pad = 1 + below(&mut sites.length, 256 * 1024);
            let second_pad = 1 + below(&mut sites.length, 256 * 1024);
            let mut first = uniform_bytes(&mut sites.edit, first_pad);
            let mut second = uniform_bytes(&mut sites.edit, second_pad);
            first.extend_from_slice(&shared);
            second.extend_from_slice(&shared);

            let first_census = chunk_census(&first, &boundaries(&first));
            let second_census = chunk_census(&second, &boundaries(&second));
            let agreed: usize = first_census
                .iter()
                .filter_map(|(hash, (size, count))| {
                    second_census.get(hash).map(|(_, other)| size * count.min(other))
                })
                .sum();

            let allowance = [(&first, first_pad), (&second, second_pad)]
                .into_iter()
                .map(|(bytes, pad)| resync_allowance(&candidates(bytes), bytes.len(), pad) - pad)
                .max()
                .expect("two objects");
            let owed = shared.len().saturating_sub(allowance);

            if owed > 0 {
                requiring_pairs += 1;
            }
            if agreed == 0 {
                deduplicating_nothing += 1;
            }
            println!(
                "P5 seed {seed} {:>12} shared {} at {first_pad} and {second_pad} deduplicated \
                 {agreed} owed {owed} allowance {allowance} requires-something {}",
                kind.name(),
                shared.len(),
                owed > 0
            );

            assert!(
                agreed >= owed,
                "seed {seed}, {} mixture: {} bytes of shared content placed at {first_pad} and at \
                 {second_pad} deduplicated to {agreed} bytes, short of the {owed} the bound owes \
                 (allowance {allowance})",
                kind.name(),
                shared.len()
            );
        }
    }

    println!(
        "P5 effective sample: {requiring_pairs} of {PAIRS} pairs carry a non-zero requirement; on \
         {} the allowance covers the whole shared run and `agreed >= owed` holds at zero, \
         {deduplicating_nothing} of which deduplicated nothing at all",
        PAIRS - requiring_pairs
    );
    // The row `claims/0017` registers, under its own name, for the reason
    // property 4's seven rows are printed: a threshold nothing prints is a
    // threshold `cargo xtask claim` cannot compare against.
    println!("dedup_pairs_with_a_non_zero_requirement {requiring_pairs}");
    assert!(
        requiring_pairs >= DEDUP_REQUIRING_MIN,
        "only {requiring_pairs} of {PAIRS} pairs place a non-zero requirement on deduplication, \
         below the {DEDUP_REQUIRING_MIN} that the eight uniform pairs alone put out of reach of \
         anything but a broken generator. Below it this property asserts nothing on most of its \
         draw while reporting a pass on all of it, which is what it did until 2026-09-06 on 13 \
         pairs. Look at the generator before the chunker: a draw that stopped producing content \
         with candidates in it would do this, and so would an allowance that grew"
    );
}
