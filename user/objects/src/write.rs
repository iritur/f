// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The write half of the objects ring: a client's bytes, into an object.
//!
//! # Why this module exists, which is a denominator and not a feature
//!
//! `claims/0017` measures *bytes re-chunked and re-hashed per application byte
//! written*, and `intent/0006-state/spec.md` defines an application byte, once,
//! as **a byte the client submitted on the objects ring**. Until this module
//! there was no such byte anywhere in the tree: `bench/src/bin/rechunk.rs`
//! writes 4096 bytes at a drawn offset straight into an `f_blob::store::Store`
//! over a modelled device, which is a write *path* and not a client, and the
//! claim has been `pending` on exactly that distinction for two epochs. It says
//! so itself, at length, and `E2-B09`'s exit says the ratio must be counted at
//! that boundary "not with a better number at this one".
//!
//! So what this module adds is not a capability. It is the boundary the
//! existing number was always supposed to be taken across.
//!
//! # What it does and does not count
//!
//! [`Written::bytes`] is the denominator and is the sum of
//! [`f_abi::objects::Write::bytes`] — the field whose own documentation says
//! so. It is what the *client asked to write*, which is RFC 0058's rule: a
//! 4 KiB write is 4096 whether it lands aligned or straddling, whether the
//! piece was already dirty, and whatever the device does underneath.
//!
//! # What a write means here, which RFC 0098 decided
//!
//! It edits the object its channel is about, at `Write::offset`, and answers
//! with that object's new content address. The channel carries the subject
//! because the ABI leaves no alternative: a `Read` names its object by content
//! address and fits — `32 + 4 = 36` against a `PAYLOAD_BYTES` of 40 — and a
//! `Write` cannot, `32 + 8 + 4 = 44`, with the payload one stride for every
//! opcode.
//!
//! This service **establishes** objects and does not **edit** them, and the
//! difference is a mebibyte: `Extent::write` allocates a piece buffer of
//! `EXTENT_BYTES` whatever the object's size, against a 128 KiB heap. A
//! non-zero offset is refused rather than approximated, because a service that
//! met an edit it could not afford by storing the submitted bytes would answer
//! `Ok` to a client whose object it had silently replaced.
//!
//! **The numerator is deliberately not here.** Bytes re-chunked and re-hashed
//! are `f_blob`'s to count, across the geometry `claims/0017` names — 8 MiB and
//! 128 MiB objects, five mixtures, two seeds — and that geometry cannot run in
//! this component: `f_blob::extent::EXTENT_BYTES` is a mebibyte and
//! `Extent::write` allocates a piece of it, against the 128 KiB heap
//! `kernel/src/objects.rs` gives this place. A component that published a
//! re-chunk ratio over a few kilobytes and let it be read as `claims/0017`'s
//! would be answering a question about one geometry with a measurement from
//! another, which is the defect `claims/0016` and `claims/0019` each already
//! carry a paragraph about. The ratio stays in the bench; what moves here is
//! the denominator's boundary.
//!
//! # No copy that the read path would not have made
//!
//! The bytes are read out of the caller's own registered buffer through
//! [`crate::dma::Landing::fetch`], which resolves the same
//! `f_ring::registry::Table` `lend` resolves and refuses the same way. They are
//! handed to `Store::put_object` as a borrowed slice. The store copies them
//! into the record it builds — `f_blob::store` does that in one place and says
//! why — and **this module states that rather than publishing a zero beside
//! it**: a write path is not a zero-copy path, and the claim this feeds is
//! about re-chunking, not about copies. `claims/0022`'s zero is the read path's
//! and is not made here, not weakened here, and not extended to this direction.

use f_blob::device::Device;
use f_zone::device::Zoned;

use crate::read::ReadPath;

/// What one write cost and what it produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    /// Application bytes this entry submitted. Unit: bytes.
    ///
    /// The sum of this across a run is `claims/0017`'s denominator, taken at
    /// the ring rather than at a `Store` call in a host process.
    pub bytes: u64,
    /// The content address the store answered with.
    ///
    /// Carried back to the client in the completion's detail word, so that a
    /// client can ask for what it just wrote without this component holding a
    /// name on its behalf. It is SHA-256, because the store folds under
    /// SHA-256 — deliberately not `f_abi::manifest::ContentId`, which is FNV-1a
    /// and names a component file. RFC 0094 records what happens when two
    /// content addresses over the same bytes are allowed to look alike.
    /// Unit: bytes of a SHA-256 digest. Zero is no address, which no real
    /// digest collides with because SHA-256 over any input is not zero.
    pub hash: [u8; 32],
}

impl<Z: Zoned, I: Device> WritePath<'_, Z, I> {
    /// Put a client's bytes into the object store.
    ///
    /// # What `at` means, and why this service refuses a non-zero one
    ///
    /// **RFC 0098.** A `WRITE` edits the object its channel is about, at `at`,
    /// and answers with that object's *new* content address — a name is its
    /// content, so an edit produces a different object and a client that could
    /// not learn the new name would have lost the old one. Where the channel
    /// has no object yet, a write at offset zero **establishes** one, which is
    /// what this service does.
    ///
    /// It cannot do the other half, and the gap is a mebibyte rather than a
    /// matter of degree. `Extent::create` chunks what it is given and stores
    /// each piece, allocating no piece-sized buffer. `Extent::write` allocates
    /// `vec![0u8; piece_bytes]`, and `piece_bytes` is `EXTENT_BYTES` — a
    /// compile-time mebibyte — **whatever the object's size**, because
    /// `blob/src/extent.rs` argues that a piece size a reader has to be told is
    /// a format with a dial in it. `kernel/src/objects.rs` gives this component
    /// a 128 KiB heap.
    ///
    /// So an edit is **refused**, and refusing is the decision rather than a
    /// shortcut around it: a service that met an edit it could not afford by
    /// storing the submitted bytes as a fresh object would answer `Ok` to a
    /// client whose object it had silently replaced. The first cut of this arm
    /// dropped `at` entirely and did exactly that.
    ///
    /// # Errors
    ///
    /// [`f_abi::store::refusal::ADDRESS`] for an edit — a non-zero `at` — which
    /// this service cannot afford and will not approximate. `Store::put_object`'s
    /// unchanged otherwise: `NO_SPACE` where the device is full, `UNKNOWN` for a
    /// kind the format does not name. A refusal a client sees is one the store
    /// made, so a client chasing it reads `f_blob`'s rules rather than this
    /// crate's paraphrase of them.
    pub fn apply(&mut self, at: u64, bytes: &[u8]) -> Result<Written, i32> {
        if at != 0 {
            return Err(f_abi::store::refusal::ADDRESS);
        }
        let hash = self.path.store_mut().put_object(bytes)?;
        Ok(Written { bytes: bytes.len() as u64, hash })
    }

    /// Make the write durable.
    ///
    /// Separate from [`Self::apply`] and called once per drain rather than
    /// once per entry, which is the barrier discipline RFC 0060 sets: a
    /// barrier per write is a barrier per entry and turns a batch into a
    /// sequence.
    ///
    /// # Errors
    ///
    /// The store's.
    pub fn barrier(&mut self) -> Result<(), i32> {
        self.path.store_mut().barrier()
    }
}

/// The store, borrowed for the length of one write.
///
/// A borrow and not a second owner: the object store is the read path's, it is
/// mounted once, and two owners of one store is the arrangement that lets a
/// write land somewhere a read cannot see it. `f_blob::store::Store` is not
/// `Sync` and nothing here makes it so.
pub struct WritePath<'a, Z: Zoned, I: Device> {
    path: &'a mut ReadPath<Z, I>,
}

/// Borrow a mounted path for writing.
pub fn over<Z: Zoned, I: Device>(path: &mut ReadPath<Z, I>) -> WritePath<'_, Z, I> {
    WritePath { path }
}

/// The kind a client write is stored under.
///
/// One kind, and the client does not choose it. An opcode that let a peer name
/// the kind would let it name one the format reserves, and `Store::put`'s
/// `UNKNOWN` refusal would become the first thing every client had to learn.
/// `f_abi::objects::Write` carries no kind field for that reason.
pub const WRITTEN_KIND: u16 = f_abi::store::kind::CHUNK;
