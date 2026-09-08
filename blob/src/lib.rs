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
//! **What else is in that class, which is the part a reader has to be told —
//! RFC 0062.** It is not only zero runs. It contains **every exactly-periodic
//! object whose period is below [`chunk::CHUNK_MIN_BYTES`] = 16 KiB**, for
//! every acceptance rule this design permits, and the reason is arithmetic
//! rather than a measurement. Past the first sixty-four bytes the register at
//! position `i` is a function of `i mod p`, so the candidate set `B` of any
//! predicate over a bounded window of content satisfies `B + p = B` away from
//! the object's ends; if `B` holds a position it holds one `p` bytes behind it,
//! and `p < CHUNK_MIN_BYTES` means that candidate can never be accepted. So the
//! object has no content boundary at all, whatever the mask is — narrower,
//! wider, nested, or local maxima, whose minimum is its own window and
//! therefore starves *more* periods, not fewer. The workloads RFC 0058 names
//! have periods of 4096 and 8192 bytes and are inside the class by that
//! arithmetic.
//!
//! A zero run is the extreme case and not the representative one, and the
//! difference is which of the two a future draw could take away: a zero run is
//! starved because one register fixed point misses one drawn mask, while
//! periodic content below the minimum is starved by a statement about minimum
//! chunk sizes that no draw and no rule reaches. RFC 0061 forecast that
//! periodic content below the *target* size had left this class; RFC 0062
//! withdraws that sentence, on RFC 0061's own confirming run, and leaves the
//! rule, the bound and [`chunk::RESYNC_BOUND_BYTES`] exactly where RFC 0061 put
//! them.
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
//!
//! # The format, and where its types are
//!
//! [`store::Store`] writes blobs to a [`device::Device`] and reads them back,
//! and it owns none of the record types it writes: the superblock, the blob
//! header, the object head and the root record are `f_abi::store`'s, decoded
//! field by field by validating constructors. They are there and not here
//! because the only use anybody has for `#[repr(C)]` over them is to view
//! device bytes as a struct, that view is a pointer cast, and RFC 0001 puts a
//! pointer cast inside the frame — which this crate is not in. So nothing here
//! casts, and no byte a device wrote is read as a field until one of those
//! constructors returned `Ok`.
//!
//! # The second object kind, and why this crate carries two shapes
//!
//! [`extent`] is the other half of what a blob can be, and it exists because of
//! the paragraph above rather than beside it. On starved content the chunked
//! kind's cost per edit scales with the object, and RFC 0062 showed that class
//! is not a corner: every exactly-periodic object with a period below
//! [`chunk::CHUNK_MIN_BYTES`] is in it, for every rule this design permits, and
//! 4096- and 8192-byte database pages are two of them. So an **extent** is an
//! object whose children are fixed [`extent::EXTENT_BYTES`] pieces rather than
//! content-defined chunks, whose copy-on-write unit is one whole piece, and
//! which is published only at an explicit snapshot. An edit costs
//! `2 x EXTENT_BYTES` and that number does not move with the object.
//!
//! It is a second kind and not a generalisation of the first, which is RFC
//! 0058's decision and the thing this crate is carrying the cost of: the same
//! bytes stored both ways have two different hashes, a consumer picks a kind
//! when it creates the object, and changing its mind is a full rewrite. What is
//! bought is a bound on the workload content addressing is worst at; what is
//! given up is deduplication between objects that were not cloned from one
//! another. Neither shape covers the other, and a later design that finds the
//! one that does reverses RFC 0058 rather than widening a kind.
//!
//! What is *not* here is a mount and a zone. A publish is *write the blobs,
//! `FLUSH`, append the root record, `FLUSH`* (RFC 0060), and only the first
//! half of that sentence is expressible without knowing what a zone is:
//! [`store::Store::barrier`] is where this crate's half ends and `E2-B02`'s
//! `f-zone` begins.

#![no_std]

extern crate alloc;

pub mod chunk;
pub mod device;
pub mod extent;
pub mod gear;
pub mod store;
