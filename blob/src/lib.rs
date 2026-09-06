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
//! honest half.** The two boundary sequences resynchronise within
//! [`chunk::RESYNC_BOUND_BYTES`] of `X + L`, *or* at the end of the enclosing
//! candidate-free run plus one chunk, whichever is later.
//!
//! The second clause is written here rather than found by a user. In content
//! with no candidates at all — a zero run, a period below the target size,
//! which is a VM image and a sparse database file and not a corner case — every
//! boundary is forced at [`chunk::CHUNK_MAX_BYTES`]. A forced cut is by
//! definition relative to the previous boundary, so two streams offset by `L`
//! do not resynchronise until the run ends, and the design is bad on exactly
//! that workload.
//!
//! *The cause is maximum-size forcing and not the minimum.* An earlier draft
//! blamed the minimum, and the reversal it wrote down — acceptance that does
//! not depend on the previous boundary — does not fix a forced cut, because a
//! forced cut has no acceptance in it at all. The paragraph is here so that
//! nobody reaches for that fix expecting this clause to go away.
//!
//! # Determinism
//!
//! This crate names [`f_env::split`] at compile time and never at run time.
//! [`gear::GEAR`] and the two masks are `const`, derived by `const fn` from one
//! label under RFC 0026's single derivation; a boundary is a function of the
//! bytes scanned and of those constants. Nothing here reads a clock, draws a
//! value or asks an `Env` for anything while it runs, and `lint-determinism`
//! finding nothing under `blob/` is the design rather than an oversight. The
//! tests draw their objects and their edits from a seeded `Env`, which is the
//! opposite obligation and is met in `blob/tests/`.
//!
//! # Why `alloc`
//!
//! An object's chunk list is variable-length — it is the first variable-length
//! thing in this workspace — and a fixed table would either bound object size
//! or waste the bound. Spec decision 4. The chunker itself allocates nothing:
//! [`chunk::Chunker`] is two words and streams.

#![no_std]

extern crate alloc;

pub mod chunk;
pub mod gear;
