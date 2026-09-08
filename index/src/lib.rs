// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Names to hashes, answered in the caller's own address space.
//!
//! # What this crate is, and the word that carries the design
//!
//! *Embedded rather than a service.* A query is [`Index::get`] — a `&self`
//! method on a `BTreeMap` the caller already owns — and not an entry submitted
//! to a ring, not a door call, not a message. There is no server here to be
//! restarted, to be scheduled, or to be waited on, and there is nothing in this
//! crate that could become one without the caller noticing: a service has a
//! `&mut self` somewhere that turns into a queue, and the query path here holds
//! no `&mut` at all.
//!
//! What that buys is the thing `E2-B03` is measured on. A query reads **no
//! device blocks**, because the answer is already in memory; a tree walk over
//! the same names reads one block per level, every time, for every query.
//! `index/tests/query.rs` counts both through the same counter and prints both
//! numbers, because a ratio with no denominator is not a measurement.
//!
//! **The honest reading of the exit, which is weaker than its words.** The exit
//! says *without crossing a component boundary*. Inside one component a tree
//! walk crosses no boundary either, so a count of ring crossings would be zero
//! against zero and the exit would be vacuous. The spec therefore reads the
//! phrase as *the index is not a service* — one boundary between a client and
//! `user/objects` at `E2-B08`, and none between the index and the store — and
//! flags that reading as weaker than the words rather than burying it. This
//! crate builds the flagged reading. It does not widen it, and the comparison
//! it is measured by is a count that can differ.
//!
//! # Why a log and a map, rather than a tree on the device
//!
//! Because the two costs a name-to-hash store has are *asked* at very different
//! rates. A write happens when something is published; a read happens whenever
//! anything is resolved. A B-tree on the device pays a logarithmic number of
//! block reads on every read to keep the write cheap. A log pays one sequential
//! append per write and a whole-log scan **once**, at mount, and then every read
//! is free. That trade is only available because the device is
//! sequential-write-required anyway (RFC 0060), so the append-only half was
//! never a cost, and because the map fits in memory — which is spec decision 4's
//! `alloc`, and is the decision in this area most worth disagreeing with.
//!
//! *What would reverse this:* an index that does not fit in memory. The mount
//! cost is linear in the log and the residency cost is linear in the live set,
//! so the reversal arrives as a number — `index/tests/query.rs` prints the mount
//! cost beside the query cost precisely so that the day it stops being amortised
//! is a day somebody can point at.
//!
//! # Durability, and where it starts
//!
//! An entry is answerable the instant [`Index::set`] returns and is **durable**
//! only after [`Index::barrier`]. That is RFC 0060's sentence and not a
//! weakening of it: a block is written to the device when it fills, and the
//! partly-filled tail block is held in memory because the media this format is
//! for cannot be written twice at one address. So a mount after a crash finds
//! every sealed block and not the tail, and `index/src/index.rs` has a test that
//! requires exactly that rather than leaving it as a sentence.
//!
//! # The pin namespace
//!
//! RFC 0059 decided that a durable pin is *an entry in `f-index`'s pin
//! namespace* — a name for a root hash, which is what this crate is already
//! for. [`ns::PIN`] is that namespace, it is reserved to that RFC, and
//! [`Index::pin`], [`Index::unpin`] and [`Index::pinned_roots`] are the
//! vocabulary the collector's `P` is read through. Nothing else may take byte
//! four; a fifth namespace takes the next free byte and an RFC.
//!
//! What is deliberately *not* here is RFC 0059's refusal that a pin naming an
//! unresolvable root is rejected. Resolution is a walk over a generation tree
//! through the store, this crate holds neither, and an index that resolved would
//! be a second thing that could disagree with the resolver about what resolves.
//! `E2-B08`'s `PIN` opcode resolves and then calls [`Index::pin`], in that
//! order, and this comment is where that obligation is recorded.
//!
//! # Determinism
//!
//! `BTreeMap` and nothing else, per RFC 0004, and here the ordering is
//! load-bearing beyond the lint: [`Index::pinned_roots`] is the collector's root
//! set, a collector walks it, and a walk in seeded-hash order would make two
//! runs of one seed collect in two orders. Nothing in this crate reads a clock,
//! draws a value, or asks an `Env` for anything — at run time or at compile
//! time.
//!
//! # The frame
//!
//! No `unsafe`, and none is reachable: this crate is above the frame, RFC 0001
//! forbids it here at compile time, and every byte that arrives from a device is
//! decoded field by field by [`record::Entry::from_slice`] rather than viewed
//! through a cast.

#![no_std]

extern crate alloc;

pub mod index;
pub mod record;

pub use index::Index;
pub use record::{Entry, ns, op};
