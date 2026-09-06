// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A blob is a run of bytes named by what it contains, and this crate is how a
//! run of bytes is cut into blobs.
//!
//! # What a blob is
//!
//! An object is split into chunks at boundaries the *content* chooses, each
//! chunk is named by SHA-256 over its bytes, and the object is the ordered list
//! of those names. Nothing about a chunk depends on where it sits in a file, in
//! which file, or on what was written before it, so two objects that share a
//! run of bytes share the chunks covering that run — which is the whole of
//! deduplication, and it falls out of the naming rather than being arranged.
//!
//! # The bound this crate promises, in the strength it has
//!
//! An edit at offset `X` of length `L` costs a bounded amount of re-chunking,
//! and the bound is two-sided.
//!
//! **Before the edit it is hard.** Nothing before the last boundary preceding
//! `X − 64` changes, by construction: the rolling register at a position is a
//! function of the last sixty-four bytes and of nothing else, so a boundary
//! decision taken before that window can see the edit is the same decision.
//!
//! **After the edit it is a bound with two clauses, and the second one is the
//! honest half.** The two boundary sequences agree from the first accepted
//! boundary at or after `X + L + CHUNK_MIN_BYTES + 64`, and in no case later
//! than [`chunk::RESYNC_BOUND_BYTES`] past `X + L`, *except* across a **starved**
//! run — a maximal run in which no two consecutive candidates are closer than
//! `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` = 240 KiB — where they agree only at the
//! end of that run plus one chunk.
//!
//! The first clause has a proof rather than a hope behind it, and RFC 0061 is
//! where it was bought: acceptance is a predicate over a window of
//! `CHUNK_MIN_BYTES + 64` bytes of content, so every decision taken past that
//! point reads only bytes the edit did not touch. The second is written here
//! rather than found by a user. Inside a starved run every boundary is forced
//! at [`chunk::CHUNK_MAX_BYTES`], a forced cut is by definition relative to the
//! previous boundary, so two streams offset by `L` do not resynchronise until
//! the run ends, and the design is bad on exactly that workload. A zero run is
//! the extreme case of it, and [`chunk::CHUNK_MAX_BYTES`] is the only thing
//! bounding a chunk there.
//!
//! *What an earlier draft of this paragraph said, and why it was wrong.* It
//! said the second clause covered content with **no candidates at all**, and
//! that the cause was maximum-size forcing and not the minimum — so that
//! acceptance independent of the previous boundary "does not fix a forced cut".
//! `E2-P02` measured that false on 2026-09-06: seven of thirty-two (seed,
//! mixture) pairs never resynchronised at all on content whose next candidate
//! was 2570 bytes past the edit, so the run clause was inert and the minimum
//! was the cause. RFC 0061 is the reversal. What survives from the draft is one
//! sentence — a forced cut is still relative to the previous boundary — and it
//! is why the second clause still exists, over the strictly smaller class of
//! *starved* rather than candidate-free content.
//!
//! # Determinism
//!
//! This crate names [`f_env::split`] at compile time and never at run time.
//! [`gear::GEAR`] and [`gear::MASK`] are `const`, derived by `const fn` from
//! one label each under RFC 0026's single derivation; a boundary is a function
//! of the bytes scanned and of those constants. Nothing here reads a clock,
//! draws a value or asks an `Env` for anything while it runs, and
//! `lint-determinism` finding nothing under `blob/` is the design rather than
//! an oversight. The tests draw their objects and their edits from a seeded
//! `Env`, which is the opposite obligation and is met in `blob/tests/`.
//!
//! # Why `alloc`
//!
//! An object's chunk list is variable-length — it is the first variable-length
//! thing in this workspace — and a fixed table would either bound object size
//! or waste the bound. Spec decision 4. The chunker itself allocates nothing:
//! [`chunk::Chunker`] is three words and streams.

#![no_std]

extern crate alloc;

pub mod chunk;
pub mod gear;
