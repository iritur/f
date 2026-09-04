// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The capability properties, proved rather than sampled.
//!
//! # What this crate is
//!
//! `kernel/src/cap.rs`, compiled a second time — the same file, not a copy —
//! against three stand-ins small enough for a bounded model checker to finish,
//! plus the harnesses that state the five properties over *arbitrary* handles
//! and *arbitrary* table contents within a stated bound.
//!
//! The five are the same five `cap::properties` checks at every boot. The
//! difference is the quantifier, and it is the whole point of the task:
//! `properties::forged` sweeps every index against eight generations and finds
//! nothing, which says *no handle we tried resolved*. [`proofs::forged`] says
//! *no handle resolves*, over all 2³² of them, because a solver was asked
//! whether one could.
//!
//! # Why it is not a workspace member
//!
//! Because it is compiled by a toolchain this tree does not pin. RFC 0022
//! decided that a checker's toolchain requirement is the checker's business
//! and gets its own image target rather than a move of `rust-toolchain.toml`;
//! Kani goes further than RustMC did and ships a rustc of its own, which
//! `cargo kani setup` installs. A crate inside the workspace would be built by
//! `cargo xtask test` under the pinned nightly, which would either be a second
//! build of `cap.rs` that proves nothing or a build failure nobody asked for.
//! `Cargo.toml`'s `exclude` is what keeps the two apart, and deleting this
//! directory is the whole of undoing it.
//!
//! # What is proved, and inside what bound
//!
//! Stated once here and argued in RFC 0053.
//!
//! * **Handles are unbounded.** Every harness that takes a handle takes
//!   `Handle::from_bits(kani::any())` — all thirty-two bits, index and
//!   generation, including the shapes no component could hold.
//! * **Rights are unbounded.** The narrowing harness quantifies over all
//!   256 × 256 pairs of held and asked bitmaps, undefined bits included.
//! * **Table contents are bounded by construction, not by assumption.** A
//!   harness builds its table by running real operations with symbolic
//!   operands. It never writes a slot directly, so no proof here can hold for
//!   a state the table cannot reach — which is the failure mode of a proof
//!   over a hand-built structure.
//! * **The table's size is bounded, and by a smaller page than the kernel
//!   buys.** [`mem::FRAME_SIZE`] says which and why, and RFC 0053 says what
//!   that does and does not cover.
//!
//! # The half that makes the other half mean anything
//!
//! `cargo xtask prove` runs these twice: once on a clean build, where every
//! harness must pass, and once with `--features mutate-unchecked-index`, where
//! `proofs::total_lookup` must **fail**, naming `cap.rs` where it does. That feature is the same
//! deliberate defect `cargo xtask mutate` boots — `kernel/src/cap.rs` carries
//! it, and the `cfg` in `Table::resolve` reads this crate's feature when
//! `cap.rs` is compiled here. A proof that passes on a build with a known
//! defect in the code it is about is not a proof of anything, and neither half
//! of the pair means anything alone. RFC 0017 is the argument; this is the
//! third instrument to make it.

#![cfg_attr(not(kani), allow(dead_code))]

pub mod mem;
pub mod pages;
pub mod percpu;

/// The frame's capability table, compiled from the file the kernel ships.
///
/// `#[path]` and not a copy. A copy would drift, and the first thing it would
/// stop containing is the defect this crate has to be able to fail on.
#[path = "../../src/cap.rs"]
pub mod cap;

#[cfg(kani)]
mod proofs;
