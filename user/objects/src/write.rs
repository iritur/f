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
//! # The subject is a field, and that is RFC 0098's decision rather than a
//! widening of it
//!
//! *The object its channel is about* is [`WritePath::subject`]. Where there is
//! one, a write edits it at `Write::offset` and answers the extent's new root.
//! Where there is none, a write at offset zero establishes an object and a
//! non-zero offset is **refused** — the same arm, unchanged, that shipped when
//! this module had no subject at all.
//!
//! **Which arm a caller gets is decided by what it could afford to construct**,
//! and that is the sentence RFC 0098 actually wrote: *an edit the service
//! cannot afford is refused and never quietly downgraded to something cheaper*.
//! `Extent::write` allocates a piece buffer of `EXTENT_BYTES` — a compile-time
//! mebibyte — whatever the object's size, against the 128 KiB heap
//! `kernel/src/objects.rs` gives this component. So the component hands this
//! path no subject and gets the refusing arm, on every boot, and there is
//! nothing it can pass that would change that: `serve.rs` constructs the
//! service and never calls [`crate::service::Service::about`].
//!
//! A host process has a heap that fits one, and `bench/src/bin/rechunk.rs` is
//! the caller that does. The affordability is a property of the caller, which
//! is the only reading under which the refusal is a decision rather than a
//! constant nobody chose.
//!
//! # The numerator is here now, and it is counted rather than computed
//!
//! Bytes re-chunked and re-hashed are `f_blob`'s: [`Written::cost`] is the
//! `f_blob::extent::Cost` that `Extent::write` answered, carried through
//! untouched. This module adds no arithmetic to it and keeps no running total —
//! `f_blob::extent::Cost`'s own comment says why the sum belongs to whoever
//! submitted, and [`crate::service::Served`] is where the sum is taken because
//! that is where a *client* can be said to have asked for one.
//!
//! What it does add is that both halves of `claims/0017`'s ratio now come off
//! the same boundary. Before this, the claim's denominator was a 4096-byte
//! write straight into a `Store` in the bench's own process — a write path and
//! not a client — while its numerator came from an `Extent` beside it. The two
//! were about the same bytes and neither was about a ring.
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
use f_blob::extent::{Cost, Extent};
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
    /// What re-chunking this write cost, as the extent counted it.
    ///
    /// `claims/0017`'s numerator, per entry. [`Cost::default`] — zeros — where
    /// the channel had no subject and the write established an object instead
    /// of editing one, because `Store::put_object` chunks content that had no
    /// previous chunking to move and there is no re-chunk to report. A zero
    /// here beside a non-zero [`Written::bytes`] is that case and not a lost
    /// measurement.
    /// Unit: bytes and a piece count — `f_blob::extent::Cost`'s three fields,
    /// each carrying its own on that type. Zero is an establish rather than an
    /// edit.
    pub cost: Cost,
    /// What naming the result cost, kept apart from [`Written::cost`].
    ///
    /// **Separate because folding it in would break the row this claim exists
    /// for.** A snapshot rewrites the extent's record, so its cost is
    /// proportional to the *piece count* and therefore to the object's size:
    /// 256 bytes for an 8 MiB extent and 4096 for a 128 MiB one. Added into the
    /// per-edit cost it would make `2 x EXTENT_BYTES at both sizes` — the
    /// identical 2 097 152 that is this claim's strongest evidence — into two
    /// numbers that differ by the object's size, which is precisely the
    /// property being denied.
    ///
    /// It is a real cost and is recorded rather than dropped: RFC 0098 requires
    /// the completion to carry the object's new content address, an extent has
    /// no address until it is snapshotted, so this is what that requirement
    /// costs per write.
    /// Unit: bytes and a piece count, as [`Written::cost`]. Zero is a write
    /// that published nothing, which is every write on a channel with no
    /// subject.
    pub published: Cost,
}

impl<Z: Zoned, I: Device> WritePath<'_, Z, I> {
    /// Put a client's bytes into the object store.
    ///
    /// # What `at` means, and what decides whether it may be non-zero
    ///
    /// **RFC 0098.** A `WRITE` edits the object its channel is about, at `at`,
    /// and answers with that object's *new* content address — a name is its
    /// content, so an edit produces a different object and a client that could
    /// not learn the new name would have lost the old one. Where the channel
    /// has no object yet, a write at offset zero **establishes** one.
    ///
    /// So there are two arms, and which one a caller gets was fixed when it
    /// constructed the path rather than when it submitted:
    ///
    /// - **With a subject**, `at` is an offset into that extent and the write
    ///   edits it. `Extent::write` refuses an offset past the end and refuses
    ///   to grow, both for reasons that file gives. The extent is snapshotted
    ///   afterwards, because RFC 0098 requires the answer to carry the object's
    ///   new address and an extent has none until it is.
    /// - **With no subject**, a non-zero `at` is **refused**. Honouring it means
    ///   holding an extent, `Extent::write` allocates a piece buffer of
    ///   `EXTENT_BYTES` whatever the object's size, and the component this crate
    ///   ships as has a 128 KiB heap. Refusing is the decision and not a
    ///   shortcut around it: a service that met an edit it could not afford by
    ///   storing the submitted bytes as a fresh object would answer `Ok` to a
    ///   client whose object it had silently replaced. The first cut of this arm
    ///   dropped `at` entirely and did exactly that.
    ///
    /// # Errors
    ///
    /// [`f_abi::store::refusal::ADDRESS`] for an edit this path has no subject
    /// for — a non-zero `at` — which it cannot afford and will not approximate,
    /// and the same code from `Extent::write` for an edit that runs past a
    /// subject's end. `Store::put_object`'s unchanged otherwise: `NO_SPACE`
    /// where the device is full, `UNKNOWN` for a kind the format does not name.
    /// A refusal a client sees is one the store made, so a client chasing it
    /// reads `f_blob`'s rules rather than this crate's paraphrase of them.
    pub fn apply(&mut self, at: u64, bytes: &[u8]) -> Result<Written, i32> {
        let submitted = bytes.len() as u64;
        let Some(extent) = self.subject.as_deref_mut() else {
            if at != 0 {
                return Err(f_abi::store::refusal::ADDRESS);
            }
            let hash = self.path.store_mut().put_object(bytes)?;
            return Ok(Written {
                bytes: submitted,
                hash,
                cost: Cost::default(),
                published: Cost::default(),
            });
        };
        let store = self.path.store_mut();
        let cost = extent.write(store, at, bytes)?;
        let (hash, published) = extent.snapshot(store)?;
        Ok(Written { bytes: submitted, hash, cost, published })
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
    /// The object this channel is about, where it has one.
    ///
    /// `None` is a channel with no object yet, which is every channel this
    /// crate's component half ever opens. See the module comment: the
    /// distinction is what a caller could afford to construct, and the whole of
    /// RFC 0098's refusal rests on it.
    subject: Option<&'a mut Extent>,
}

/// Borrow a mounted path for writing, and tell it what its channel is about.
///
/// `subject` is `None` for a channel with no object — the component's case, and
/// the case in which a non-zero offset is refused.
pub fn over<'a, Z: Zoned, I: Device>(
    path: &'a mut ReadPath<Z, I>,
    subject: Option<&'a mut Extent>,
) -> WritePath<'a, Z, I> {
    WritePath { path, subject }
}

/// The kind a client write is stored under.
///
/// One kind, and the client does not choose it. An opcode that let a peer name
/// the kind would let it name one the format reserves, and `Store::put`'s
/// `UNKNOWN` refusal would become the first thing every client had to learn.
/// `f_abi::objects::Write` carries no kind field for that reason.
pub const WRITTEN_KIND: u16 = f_abi::store::kind::CHUNK;
