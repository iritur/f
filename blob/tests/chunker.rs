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
use f_blob::gear::{GEAR, MASK_LOOSE, MASK_STRICT};
use f_env::split::{Stream, label};

/// The seeds every property below is asserted across.
///
/// Recorded rather than drawn, because a seed set that moves is a seed set
/// nobody can be asked to reproduce. A failure names the seed it happened on
/// and that seed is in this list.
const SEEDS: &[u64] = &[1, 2, 3, 4, 5, 6, 7, 8];

/// The window the register sees, in bytes.
///
/// Bit 63 of the register is the highest bit either mask tests, and bit *k*
/// depends on the last *k+1* bytes, so a boundary decision is a function of the
/// last sixty-four bytes. This is the number the prefix half of the bound is
/// stated in.
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

/// `len` bytes of the named mixture.
///
/// Two streams, and which draw goes to which is the point: `content` answers
/// the bytes, `mixture` answers what kind of content comes next inside a
/// concatenation. Splitting them means a longer object does not shift the
/// sequence of run kinds, and a fifth kind added later does not shift the
/// bytes.
fn draw(kind: Mixture, content: &mut Stream, mixture: &mut Stream, len: usize) -> Vec<u8> {
    let stream = &mut *content;
    match kind {
        Mixture::Uniform => uniform_bytes(stream, len),
        Mixture::Zero => vec![0u8; len],
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
            out
        }
        Mixture::Concatenated => {
            let mut out = Vec::with_capacity(len);
            while out.len() < len {
                let run = (CHUNK_TARGET_BYTES + below(mixture, 8 * CHUNK_TARGET_BYTES))
                    .min(len - out.len());
                let inner = Mixture::ALL[below(mixture, 3)];
                out.extend_from_slice(&draw(inner, content, mixture, run));
            }
            out
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

/// Every position at which the register hits *either* mask.
///
/// A candidate is a property of the content and of the two constants, and of
/// nothing the chunker is doing at the time: the register is never reset at a
/// cut, so this sequence is the same whatever the boundaries turn out to be.
/// That is what lets the bound's second clause be stated over the object rather
/// than over a run of the algorithm.
///
/// Either mask, not the one the chunker would have consulted at that distance
/// from its last cut. The union is the generous reading — it makes
/// candidate-free runs as short as they can honestly be — and the clause it
/// feeds is about content with *no candidates at all*.
fn candidates(bytes: &[u8]) -> Vec<usize> {
    let mut register: u64 = 0;
    let mut out = Vec::new();
    for (index, byte) in bytes.iter().enumerate() {
        register = (register << 1).wrapping_add(GEAR[*byte as usize]);
        if register & MASK_STRICT == 0 || register & MASK_LOOSE == 0 {
            out.push(index + 1);
        }
    }
    out
}

/// The end of the maximal candidate-free run containing `at`.
///
/// The first candidate at or after `at`, or the end of the object if there is
/// none. `at` inside a run of candidates answers `at` itself, which makes the
/// clause below collapse to the flat bound exactly where it should.
fn candidate_free_run_end(candidates: &[usize], length: usize, at: usize) -> usize {
    candidates.iter().copied().find(|&c| c >= at).unwrap_or(length)
}

/// How far past `from` the sequences are allowed to take to agree again.
///
/// The bound as the spec states it: [`RESYNC_BOUND_BYTES`] past `from`, **or**
/// the end of the enclosing candidate-free run plus one chunk, whichever is
/// later. One chunk is [`CHUNK_MAX_BYTES`], because that is the largest a chunk
/// can be and the clause is about a region where every cut is forced at exactly
/// that.
fn resync_allowance(candidates: &[usize], length: usize, from: usize) -> usize {
    let flat = from + RESYNC_BOUND_BYTES;
    let run = candidate_free_run_end(candidates, length, from) + CHUNK_MAX_BYTES;
    flat.max(run)
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
fn draw_object(sites: &mut Sites, kind: Mixture) -> Vec<u8> {
    let length = 2 * 1024 * 1024 + below(&mut sites.length, 2 * 1024 * 1024);
    draw(kind, &mut sites.object, &mut sites.mixture, length)
}

/// Property 1. Every chunk but the last is inside the size bounds.
#[test]
fn every_chunk_but_the_last_lies_between_the_minimum_and_the_maximum() {
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let object = draw_object(&mut sites, kind);
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

/// Property 2. The mean chunk is within a factor of two of the target.
///
/// Asserted over the whole draw, which is what the target is a statement about.
/// The zero-filled mixture's own mean is exactly [`CHUNK_MAX_BYTES`] — four
/// times the target — because that content has no candidates and every cut is
/// forced, and that is asserted here as the specific fact it is rather than
/// left to be averaged away. A mixture whose mean drifted to the maximum
/// *without* being candidate-free would pass the aggregate and fail this.
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
            let object = draw_object(&mut sites, kind);
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
        if kind == Mixture::Zero {
            assert_eq!(
                forced_at_the_maximum, chunks,
                "zero-filled content has no candidates, so every one of its {chunks} interior \
                 chunks must be forced at {CHUNK_MAX_BYTES} bytes; {forced_at_the_maximum} were. \
                 A cut that is not forced here means the register's fixed point on a zero run \
                 hits a mask, which changes what the bound's second clause is about"
            );
        }
        per_mixture.push((kind, bytes / chunks, chunks));
        total_bytes += bytes;
        total_chunks += chunks;
    }

    let mean = total_bytes / total_chunks;
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
            let object = draw_object(&mut sites, kind);
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

/// Property 4. The sequences resynchronise inside the two-clause bound.
///
/// `RESYNC_BOUND_BYTES` past `X + L`, or the end of the enclosing
/// candidate-free run plus one chunk, whichever is later. The second clause is
/// not a hedge for this test to be generous with: it is computed from the
/// edited object's own candidate sequence, so on content that has candidates it
/// collapses to the flat bound.
#[test]
fn the_boundary_sequences_resynchronise_within_the_bound() {
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let object = draw_object(&mut sites, kind);
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
            let candidates = candidates(&edited);
            let allowed = resync_allowance(&candidates, edited.len(), at + length);

            assert!(
                agreed <= allowed,
                "seed {seed}, {} mixture, object {} bytes, insertion of {length} bytes at {at}: \
                 the boundary sequences agree again only at {agreed}, past the {allowed} the \
                 bound allows ({} flat, run to {} plus one chunk){}",
                kind.name(),
                object.len(),
                at + length + RESYNC_BOUND_BYTES,
                candidate_free_run_end(&candidates, edited.len(), at + length),
                if agreed == edited.len() {
                    " — and that is the object's end, so nothing after the edit was preserved"
                } else {
                    ""
                }
            );
        }
    }
}

/// Property 5. Identical content in two objects yields identical chunk hashes.
///
/// The deduplication half. A run of content is placed at two different offsets
/// in two objects, and the bytes it covers must end up in chunks with the same
/// names — up to the same two-clause allowance, because the run has to
/// resynchronise before its chunks can agree, and on candidate-free content it
/// never does.
#[test]
fn identical_content_in_two_objects_yields_identical_chunk_hashes() {
    for &seed in SEEDS {
        for kind in Mixture::ALL {
            let mut sites = Sites::new(seed);
            let shared = draw(kind, &mut sites.object, &mut sites.mixture, 2 * 1024 * 1024);
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
}
