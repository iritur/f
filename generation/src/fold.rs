// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The post-order Merkle fold: a checked tree in, one root hash out.
//!
//! # The rule, once
//!
//! **A node's hash is the SHA-256 of its encoding with each child replaced by
//! that child's hash.** Everything below is that sentence applied three times,
//! and there is deliberately no second rule: a special case for one kind — a
//! leaf that is transparent, say, because hashing a hash adds no information —
//! would be exactly the sort of thing that reads as a simplification and turns
//! into a second implementation of the format.
//!
//! # Why there is no allocator and no explicit stack
//!
//! Because the tree's depth is fixed by its four kinds. A `generation` holds a
//! `bytes` leaf and a `topology`; a `topology` holds `component` leaves and
//! routes; a leaf holds nothing. No kind can hold a kind that can hold it, so
//! the deepest tree the encoding can express is three nodes tall — and a fold
//! over a tree of known depth is loops, not recursion. Nothing here allocates,
//! nothing here can run away, and no bound has to be invented for a recursion
//! that does not exist.
//!
//! That argument is also the argument against a fifth node kind that nests: it
//! would not be a patch to this file, it would be the deletion of the reason
//! this file needs no heap. RFC, not patch.
//!
//! # Why the fold streams
//!
//! A topology's folded encoding is longer than its stored one — each child is
//! replaced by thirty-two bytes that are not in the buffer — so a fold that
//! built the encoding first would need a buffer whose size is a function of the
//! member count, which is the allocator this crate does not have. Streaming into
//! one [`Sha256`] state avoids it and is the same shape `f-blob`'s chunker
//! needs, which is why `hash/` exposes a streaming state at all.
//!
//! No padding is ever hashed. That is true because every field width in
//! `f_abi::store` sums to that node's stated width, asserted there at compile
//! time — not because anything here is careful.

use f_abi::store::{Generation, Leaf, Topology};
use f_hash::Sha256;

use crate::record::Tree;

/// The root of a checked tree.
///
/// The generation node's folded encoding is its header, the frame child's hash
/// and the topology child's hash — the frame hash and the topology hash and
/// nothing else, which is the sentence the spec fixes and the reason no store
/// parameter can reach a root. Geometry lives in the superblock, and a root that
/// moved because a disk was replaced would name a generation nothing is running.
#[must_use]
pub fn root(tree: &Tree<'_>) -> [u8; 32] {
    let mut state = Sha256::new();
    state.update(&Generation::to_bytes());
    state.update(&leaf(&tree.frame()));
    state.update(&topology(tree));
    state.finish()
}

/// One leaf's hash: the SHA-256 of its own encoding.
///
/// Not the content address it carries. The two are different on purpose — the
/// address names bytes in a store, the leaf hash names *this leaf naming those
/// bytes under this name* — and conflating them would make renaming a component
/// invisible to the root.
#[must_use]
pub fn leaf(node: &Leaf) -> [u8; 32] {
    let mut state = Sha256::new();
    state.update(&node.to_bytes());
    state.finish()
}

/// The topology's hash: its head, then every member's hash in canonical order,
/// then every route.
///
/// Members before routes because the fold is post-order and the members are the
/// children; a route is a field of this node and not a child of it, which is
/// what makes moving a route a change to the topology's hash and not to
/// anybody's leaf.
#[must_use]
pub fn topology(tree: &Tree<'_>) -> [u8; 32] {
    let mut state = Sha256::new();
    state.update(&tree.head().to_bytes());
    for index in 0..tree.members() {
        if let Some(member) = tree.member(index) {
            state.update(&leaf(&member));
        }
    }
    for index in 0..tree.routes() {
        if let Some(route) = tree.route(index) {
            state.update(&route.to_bytes());
        }
    }
    state.finish()
}

/// The stored width of the head a topology's fold begins with, restated here so
/// that a change to it stops this file rather than silently changing every root.
const _: () = assert!(Topology::HEAD_BYTES == 12);
