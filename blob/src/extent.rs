// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The second object kind: fixed-offset pieces, copied whole, published at an
//! explicit snapshot. RFC 0058.
//!
//! # Why a second kind rather than a better chunker
//!
//! Because no chunker reaches this workload, and RFC 0062 turned that from a
//! measurement into arithmetic. Any boundary rule that reads a bounded window
//! of content and guarantees a minimum chunk size of `m` produces no content
//! boundary at all on exactly `p`-periodic content with `p < m`: the register
//! past the first sixty-four bytes is a function of `i mod p`, so the candidate
//! set is invariant under `+p`, and a candidate `p < m` behind another can
//! never be accepted. [`crate::chunk::CHUNK_MIN_BYTES`] is 16 KiB and database
//! pages are 4096 and 8192 bytes, so the two workloads RFC 0058 names — a
//! preallocated database file, a freshly provisioned virtual machine image —
//! are starved by a theorem rather than by an unlucky draw. On starved content
//! an edit re-chunks everything after it, which is a cost that scales with the
//! object, and a cost that scales with the object is not something a database
//! can plan against.
//!
//! So this module does not chunk. An extent's children are pieces at fixed
//! [`EXTENT_BYTES`] offsets, a write replaces every piece it touches whole, and
//! the cost of an edit is `2 x EXTENT_BYTES` however large the extent is.
//!
//! # What that costs, said in the number rather than around it
//!
//! A 4 KiB write copies and re-hashes a megabyte: 256 application bytes' worth
//! of work for one. Straddling a piece boundary it is 512x, which happens on
//! about 0.39% of 4 KiB writes at uniformly drawn offsets — `4095 / 1048576` —
//! so a run long enough to matter hits it, and `bench/src/bin/rechunk.rs`
//! records it as its own row rather than averaging it away. This kind is not
//! *good* at small random writes. It is *bounded* on them, and the whole of RFC
//! 0058 is the argument that bounded is worth two shapes in the format.
//!
//! # The snapshot boundary, and the one place this store has state
//!
//! [`Extent::snapshot`] is the only thing that publishes an extent. Between
//! snapshots the pieces are durable and hashed, and no hash names the current
//! bytes: a reader resolving the last published hash sees that snapshot whole,
//! and the owner reading through its own handle sees its own writes. That is
//! the one piece of state in this store that is not a function of a root, and
//! it is deliberate — RFC 0058 refused a per-piece dirty log precisely because
//! a log puts *durable bytes* in front of the store that no hash names, which
//! would leave RFC 0059's collector marking from something it cannot verify and
//! RFC 0060's verify-before-accept with nothing to verify against. What is in
//! front of the store here is a `Vec` of hashes in the owner's memory, and a
//! crash loses it and nothing else.
//!
//! A consequence worth naming rather than discovering: pieces written since the
//! last snapshot are named by no root, so an extent with unsnapshotted pieces
//! is a **transient root** for RFC 0059's mark phase, under the clause an open
//! publish uses. The held set is bounded by the extent's own size. Registering
//! it is `E2-B04`'s, because there is no collector here to register with; this
//! paragraph is where the obligation is written down.
//!
//! # Determinism
//!
//! Nothing here reads a clock, draws a value or asks an `Env` for anything.
//! [`EXTENT_BYTES`] is a compiled-in constant of the *writer* and not a
//! superblock field, for the reason `f_abi::store::ExtentHead` gives: every
//! piece but the last is exactly that long and a piece is a blob whose header
//! carries its length, so a reader recovers the granularity from piece 0.
//! [`Extent::open`] does exactly that rather than assuming this build's
//! constant, which is what makes the granularity reversal RFC 0058 names cost a
//! constant and not a format migration.

use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::{ExtentHead, PIECES_MAX, kind, refusal};

use crate::device::Device;
use crate::store::Store;

/// The copy-on-write unit: one whole piece.
///
/// Unit: bytes. One mebibyte, and RFC 0058's argument for it is two-sided
/// because the piece is the unit of *contiguity* as well as of copying.
///
/// Downwards, 256 KiB would cut the per-write factor from 256x to 64x and loses
/// three ways: it is exactly [`crate::chunk::CHUNK_MAX_BYTES`], so the second
/// kind would have the same rewrite granularity as the first and would exist
/// for no structural gain in the case that motivated it; the piece list
/// quadruples, and the list is the thing rewritten at every snapshot; and a
/// 256 KiB append is small enough for per-operation overhead to show on the
/// device. Upwards, 4 MiB makes the factor 1024x to shrink a list that already
/// fits. One mebibyte is the smallest power of two strictly above
/// [`crate::chunk::CHUNK_MAX_BYTES`], and it is the size at which a 128 MiB
/// extent's 128 hashes are 4096 bytes — one block. That is an argument from
/// arithmetic and not from measurement, and RFC 0058 names the measurement that
/// would move it: a claim 0017 run whose snapshot row exceeds its per-write row
/// says the list has become the dominant term, and the answer is 4 MiB.
///
/// It is not a superblock field. The chunker's parameters are, because they
/// decide every object hash and a mount must refuse a device that disagrees;
/// this decides nothing a reader cannot recover from piece 0's own header, so
/// extents written at two granularities coexist and stay readable.
pub const EXTENT_BYTES: usize = 1024 * 1024;

/// What one write or one snapshot cost, in the two numbers claim 0017
/// registers.
///
/// # Why the cost is returned rather than accumulated inside the extent
///
/// Because the claim's denominator is *application bytes* — the bytes a client
/// submitted — and the only thing that knows how many of those there were is
/// the caller that submitted them. A counter living in here would make the
/// numerator a number the write path keeps about itself, which a reader would
/// then have to trust, and it would also have to be reset by somebody. Handing
/// the cost back per call makes the ratio an arithmetic the workload does in
/// the open, and makes a caller that never adds them up cost nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// Pieces rewritten by this call.
    ///
    /// Unit: count of pieces. RFC 0058's `pieces_touched`, and the row that
    /// says *why* the primary threshold is two megabytes rather than one: a
    /// write of `W <= EXTENT_BYTES` bytes at offset `O` rewrites
    /// `ceil((O mod EXTENT_BYTES + W) / EXTENT_BYTES)` pieces, which is 1 or 2.
    pub pieces: u32,
    /// Bytes moved into the new pieces, including every byte the client did not
    /// write.
    ///
    /// Unit: bytes. That last clause is the quantity this design is being
    /// honest about: a 4 KiB write moves a megabyte, and 1 048 572 of those
    /// bytes are bytes the client already had.
    pub copied: u64,
    /// Bytes fed to `f-hash` to name the new pieces.
    ///
    /// Unit: bytes. Equal to [`Cost::copied`] for a whole piece, and recorded
    /// separately anyway, because a future sub-piece scheme would move the two
    /// apart and one row would hide it.
    ///
    /// This counts the *write* side only. Verifying a piece that is read back
    /// also feeds `f-hash`, and that is a read-path cost belonging to the read
    /// claims: folding it in here would report 2x for an aligned write and make
    /// RFC 0058's *they are equal for a full piece* false for a reason that has
    /// nothing to do with the write path.
    pub hashed: u64,
}

impl Cost {
    /// The two costs of two calls, added.
    ///
    /// `saturating`, because a benchmark that panicked at its far end would be
    /// a workload with a length limit nobody stated — `bench/src/bin/unmap_churn.rs`
    /// carries the same reasoning for the same reason.
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        Self {
            pieces: self.pieces.saturating_add(other.pieces),
            copied: self.copied.saturating_add(other.copied),
            hashed: self.hashed.saturating_add(other.hashed),
        }
    }
}

/// An extent, and the owner's handle on it.
///
/// # Why this borrows the store per call instead of holding it
///
/// A handle that took the store for its lifetime would make an extent the only
/// thing a component could do with its store, which is not what an extent is:
/// the same store holds the objects, the generations and every other extent.
/// So the store is an argument, and what this value owns is the piece list —
/// which is exactly the state RFC 0058 says lives in front of the store between
/// snapshots, held in the owner's memory and lost by a crash.
#[derive(Clone, Debug)]
pub struct Extent {
    /// The extent's logical size. Unit: bytes. Fixed at creation: an extent
    /// does not grow, for the reason [`Extent::write`] gives.
    extent_bytes: u64,
    /// The current piece hashes in order — the owner's read-your-writes view,
    /// and what the next snapshot will publish.
    ///
    /// Unit: count of piece hashes; each entry is a content address.
    pieces: Vec<[u8; 32]>,
    /// The granularity this extent's pieces were written at.
    ///
    /// Unit: bytes. [`EXTENT_BYTES`] for anything this build creates, and
    /// whatever piece 0's header says for anything [`Extent::open`] read. The
    /// field exists so that a writer this build hands an extent written at
    /// another granularity replaces pieces at *that* granularity rather than
    /// silently re-cutting the object.
    piece_bytes: usize,
}

impl Extent {
    /// Write `bytes` as an extent's pieces and answer a handle on it.
    ///
    /// Nothing is published: the pieces are durable blobs after the next
    /// barrier, and no hash names the extent until [`Extent::snapshot`] is
    /// called. That is the same asymmetry RFC 0058 describes and not an
    /// oversight — an extent that published itself at creation would have one
    /// boundary rule at creation and another for ever after.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] for content that would need more than
    /// [`PIECES_MAX`] pieces; otherwise whatever `Store::put` returns.
    pub fn create<D: Device>(store: &mut Store<D>, bytes: &[u8]) -> Result<Self, i32> {
        if bytes.len().div_ceil(EXTENT_BYTES) > PIECES_MAX {
            return Err(refusal::UNKNOWN);
        }
        let mut pieces = Vec::with_capacity(bytes.len().div_ceil(EXTENT_BYTES));
        for piece in bytes.chunks(EXTENT_BYTES) {
            // `kind::CHUNK` and not a fifth kind: RFC 0058 forecloses adding
            // one, a piece is a run of bytes named by its content, and that is
            // the whole of what that kind means. Two pieces that are
            // byte-identical are one record, which is the deduplication this
            // kind keeps — a clone shares every piece until it is written.
            pieces.push(store.put(kind::CHUNK, piece)?);
        }
        Ok(Self { extent_bytes: bytes.len() as u64, pieces, piece_bytes: EXTENT_BYTES })
    }

    /// Resolve a published extent and answer a handle on it.
    ///
    /// The granularity comes from piece 0's own header rather than from
    /// [`EXTENT_BYTES`], which is RFC 0058's *the piece size is recoverable
    /// from the data* implemented rather than asserted. An extent written by a
    /// build with a different constant opens here and stays writable at its own
    /// granularity.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] when `hash` names a blob that is not an extent;
    /// [`refusal::MALFORMED`] when the piece count and the extent's own length
    /// cannot both be true of any granularity, which is a truncated or padded
    /// piece list; otherwise whatever `Store::get` and `Store::head` return,
    /// including [`refusal::CONTENT`].
    pub fn open<D: Device>(store: &mut Store<D>, hash: &[u8; 32]) -> Result<Self, i32> {
        let header = store.head(hash)?;
        if header.kind != kind::EXTENT {
            return Err(refusal::UNKNOWN);
        }
        let content_bytes =
            usize::try_from(header.content_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        let mut content = vec![0u8; content_bytes];
        store.get(hash, &mut content)?;

        if content.len() < ExtentHead::HEAD_BYTES {
            return Err(refusal::MALFORMED);
        }
        let mut raw = [0u8; ExtentHead::HEAD_BYTES];
        raw.copy_from_slice(&content[..ExtentHead::HEAD_BYTES]);
        let head = ExtentHead::from_bytes(&raw)?;
        if head.bytes() != content.len() {
            return Err(refusal::MALFORMED);
        }

        let mut pieces = Vec::with_capacity(head.pieces as usize);
        for index in 0..head.pieces as usize {
            let at = ExtentHead::HEAD_BYTES + index * 32;
            let mut piece = [0u8; 32];
            piece.copy_from_slice(&content[at..at + 32]);
            pieces.push(piece);
        }

        let extent_bytes = usize::try_from(head.extent_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        // The granularity, read from the data. Piece 0 is a whole piece unless
        // it is also the last one, so its length *is* the granularity; the
        // empty extent has none and needs none.
        let piece_bytes = match pieces.first() {
            None => EXTENT_BYTES,
            Some(first) => usize::try_from(store.head(first)?.content_bytes)
                .map_err(|_| refusal::SHORT_BUFFER)?,
        };
        // And the check the granularity buys: the pieces must tile the extent
        // at that granularity exactly. Without it a truncated list would be
        // read back as a shorter extent whose last piece happened to fit, which
        // is the failure a length field a device wrote is for.
        if piece_bytes == 0 || extent_bytes.div_ceil(piece_bytes) != pieces.len() {
            return Err(refusal::MALFORMED);
        }
        Ok(Self { extent_bytes: head.extent_bytes, pieces, piece_bytes })
    }

    /// The extent's logical size.
    ///
    /// Unit: bytes.
    #[must_use]
    pub const fn extent_bytes(&self) -> u64 {
        self.extent_bytes
    }

    /// How many pieces the extent has.
    ///
    /// Unit: count of pieces.
    #[must_use]
    pub fn pieces(&self) -> usize {
        self.pieces.len()
    }

    /// The granularity this extent's pieces are cut at.
    ///
    /// Unit: bytes.
    #[must_use]
    pub const fn piece_bytes(&self) -> usize {
        self.piece_bytes
    }

    /// Overwrite `bytes` at `at`, and answer what it cost.
    ///
    /// # Why a write cannot grow the extent
    ///
    /// Because growth is the one operation whose cost is not bounded by the
    /// granularity: appending past the last piece rewrites a partial piece and
    /// then allocates, and a `write` that sometimes did that would make the
    /// bound this kind exists for conditional on where the offset landed.
    /// Growth is [`Extent::create`] over the larger bytes, which is a rewrite
    /// and is priced as one. A write past the end is refused rather than
    /// truncated, because a short write a caller has to notice is a caller that
    /// will not.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write that runs past the extent's end;
    /// otherwise whatever `Store::get` and `Store::put` return, including
    /// [`refusal::CONTENT`] for a piece the device gave back wrong.
    pub fn write<D: Device>(
        &mut self,
        store: &mut Store<D>,
        at: u64,
        bytes: &[u8],
    ) -> Result<Cost, i32> {
        let end = at.checked_add(bytes.len() as u64).ok_or(refusal::ADDRESS)?;
        if end > self.extent_bytes {
            return Err(refusal::ADDRESS);
        }
        if bytes.is_empty() {
            return Ok(Cost::default());
        }

        let granularity = self.piece_bytes as u64;
        let first = (at / granularity) as usize;
        let last = ((end - 1) / granularity) as usize;
        let mut cost = Cost::default();
        let mut buffer = vec![0u8; self.piece_bytes];

        for index in first..=last {
            let piece_at = index as u64 * granularity;
            let piece_len = (self.extent_bytes - piece_at).min(granularity) as usize;
            let from = (at.max(piece_at) - piece_at) as usize;
            let to = (end.min(piece_at + piece_len as u64) - piece_at) as usize;
            // Where in the caller's buffer this piece's share begins. It is not
            // zero for any piece but the first, and a `whole` piece in the
            // middle of a multi-piece write is exactly the case where forgetting
            // that writes the wrong megabyte.
            let taken = (piece_at + from as u64 - at) as usize;

            if from == 0 && to == piece_len {
                // Nothing to read: the client's bytes are the piece. A writer
                // that read here would pay a megabyte of device traffic to
                // discard it, and the read claims would carry a read this
                // design never makes.
                buffer[..piece_len].copy_from_slice(&bytes[taken..taken + piece_len]);
            } else {
                let read = store.get(&self.pieces[index], &mut buffer)?;
                if read != piece_len {
                    return Err(refusal::MALFORMED);
                }
                buffer[from..to].copy_from_slice(&bytes[taken..taken + (to - from)]);
            }

            self.pieces[index] = store.put(kind::CHUNK, &buffer[..piece_len])?;
            cost = cost.and(Cost { pieces: 1, copied: piece_len as u64, hashed: piece_len as u64 });
        }
        Ok(cost)
    }

    /// Read `into.len()` bytes from `at`, and answer how many were delivered.
    ///
    /// The owner sees its own writes: this walks [`Extent::pieces`], which the
    /// last [`Extent::write`] updated, and not the last published snapshot.
    /// Read-your-writes holds inside the owning component and nowhere else, and
    /// that is the whole of RFC 0058's asymmetry.
    ///
    /// A read is block-granular inside a piece in the store this will finally
    /// have: nothing here forces a reader to fetch a megabyte because the
    /// writer had to write one. This implementation reads whole pieces because
    /// `Store::get` is whole-blob today — `E2-B08` is the ranged read — and
    /// that is a property of this caller rather than of the granularity.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a range running past the extent's end;
    /// otherwise whatever `Store::get` returns.
    pub fn read<D: Device>(
        &self,
        store: &mut Store<D>,
        at: u64,
        into: &mut [u8],
    ) -> Result<usize, i32> {
        let end = at.checked_add(into.len() as u64).ok_or(refusal::ADDRESS)?;
        if end > self.extent_bytes {
            return Err(refusal::ADDRESS);
        }
        if into.is_empty() {
            return Ok(0);
        }

        let granularity = self.piece_bytes as u64;
        let first = (at / granularity) as usize;
        let last = ((end - 1) / granularity) as usize;
        let mut buffer = vec![0u8; self.piece_bytes];
        let mut done = 0;

        for index in first..=last {
            let piece_at = index as u64 * granularity;
            let piece_len = store.get(&self.pieces[index], &mut buffer)?;
            let from = (at.max(piece_at) - piece_at) as usize;
            let to = (end.min(piece_at + piece_len as u64) - piece_at) as usize;
            into[done..done + (to - from)].copy_from_slice(&buffer[from..to]);
            done += to - from;
        }
        if done != into.len() {
            return Err(refusal::MALFORMED);
        }
        Ok(done)
    }

    /// Publish the extent: write the record that names its pieces in order, and
    /// answer its hash and what the record cost.
    ///
    /// # Why the barrier is here and not left to the caller
    ///
    /// RFC 0060's ordering, one level down: the record names pieces, so the
    /// pieces must be on stable media before the record that names them is, or
    /// a cut leaves a hash resolving to blobs that are not there. `Store::put`
    /// is not a barrier, so this issues one before writing the record and
    /// another after it. A caller that wanted to batch several extents into one
    /// pair of barriers is asking for a publish, and a publish is the root
    /// record's — `f-zone`'s, at `E2-B02`.
    ///
    /// The cost is returned separately from [`Extent::write`]'s for RFC 0058's
    /// reason: the record's rewrite is a *snapshot* cost, and folding it into
    /// the per-write number would make that number a function of how often the
    /// caller snapshots, which is a knob and not a property.
    ///
    /// # Errors
    ///
    /// Whatever `Store::put` and `Device::flush` return.
    pub fn snapshot<D: Device>(&self, store: &mut Store<D>) -> Result<([u8; 32], Cost), i32> {
        let head = ExtentHead {
            extent_bytes: self.extent_bytes,
            pieces: u32::try_from(self.pieces.len()).map_err(|_| refusal::UNKNOWN)?,
        };
        let mut content = Vec::with_capacity(head.bytes());
        content.extend_from_slice(&head.to_bytes());
        for piece in &self.pieces {
            content.extend_from_slice(piece);
        }

        store.barrier()?;
        let hash = store.put(kind::EXTENT, &content)?;
        store.barrier()?;
        let cost = Cost { pieces: 0, copied: content.len() as u64, hashed: content.len() as u64 };
        Ok((hash, cost))
    }
}

#[cfg(test)]
mod tests {
    use super::{Cost, EXTENT_BYTES, Extent};
    use crate::device::Memory;
    use crate::store::{Store, superblock_for_this_build};
    use alloc::vec;
    use alloc::vec::Vec;
    use f_abi::store::refusal;

    /// A device with room for `blobs` pieces and the records around them.
    fn a_store(blobs: u64) -> Store<Memory> {
        let block_bytes = 4096;
        let per_piece = EXTENT_BYTES as u64 / block_bytes + 2;
        let blocks = blobs * per_piece + 64;
        let device = Memory::new(block_bytes as usize, blocks);
        let layout = superblock_for_this_build(block_bytes as u32, 1, blocks * block_bytes, 0, 0);
        Store::format(device, &layout).expect("a device this build formatted itself")
    }

    /// Content with no run of identical bytes, so that a wrong offset shows up
    /// as wrong bytes rather than as the same byte from somewhere else.
    fn counted(len: usize) -> Vec<u8> {
        let mut counter: u64 = 0;
        (0..len)
            .map(|_| {
                counter = counter.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (counter >> 33) as u8
            })
            .collect()
    }

    /// The number the skeptic reaches for: a 4 KiB write costs one whole piece.
    ///
    /// Written as an assertion on [`Cost`] rather than as a comment, because
    /// 256x is what RFC 0058 published and a build that quietly copied less
    /// would be a different design wearing this one's claim.
    #[test]
    fn an_aligned_small_write_copies_exactly_one_piece() {
        let mut store = a_store(12);
        let bytes = counted(4 * EXTENT_BYTES);
        let mut extent = Extent::create(&mut store, &bytes).expect("room for four pieces");

        let write = vec![0xA5u8; 4096];
        let cost = extent.write(&mut store, 2 * EXTENT_BYTES as u64, &write).expect("inside");
        assert_eq!(
            cost,
            Cost { pieces: 1, copied: EXTENT_BYTES as u64, hashed: EXTENT_BYTES as u64 }
        );
        assert_eq!(cost.copied / write.len() as u64, 256, "the 256x RFC 0058 publishes");
    }

    /// The straddling write, which is why the threshold is two megabytes.
    ///
    /// RFC 0058 refused an aligned-offset workload in as many words — that
    /// would be fitting the measurement to the threshold — so the case is
    /// asserted here at the boundary where it is certain rather than waited for
    /// in a bench that draws offsets.
    #[test]
    fn a_write_across_a_piece_boundary_rewrites_two_and_no_more() {
        let mut store = a_store(12);
        let bytes = counted(4 * EXTENT_BYTES);
        let mut extent = Extent::create(&mut store, &bytes).expect("room for four pieces");

        let write = vec![0x5Au8; 4096];
        let at = EXTENT_BYTES as u64 - 2048;
        let cost = extent.write(&mut store, at, &write).expect("inside");
        assert_eq!(cost.pieces, 2, "ceil((O mod EXTENT_BYTES + W) / EXTENT_BYTES)");
        assert_eq!(cost.copied, 2 * EXTENT_BYTES as u64, "2 x EXTENT_BYTES and not more");
        assert_eq!(cost.hashed, cost.copied, "equal for whole pieces");

        let mut back = vec![0u8; 4096];
        extent.read(&mut store, at, &mut back).expect("read-your-writes");
        assert_eq!(back, write);
    }

    /// The bound is the granularity and not the object: the same edit costs the
    /// same on an extent thirty-two times larger.
    ///
    /// This is the exit's sentence as a test. If it ever fails, the copy unit
    /// has stopped being the piece and RFC 0058's first reversal condition has
    /// fired.
    #[test]
    fn the_cost_of_an_edit_does_not_grow_with_the_extent() {
        let mut small = a_store(8);
        let mut large = a_store(40);
        let four = counted(4 * EXTENT_BYTES);
        let thirty_two = counted(32 * EXTENT_BYTES);
        let mut a = Extent::create(&mut small, &four).expect("room");
        let mut b = Extent::create(&mut large, &thirty_two).expect("room");

        let write = vec![0x11u8; 4096];
        let one = a.write(&mut small, 3 * EXTENT_BYTES as u64 + 777, &write).expect("inside");
        let other = b.write(&mut large, 31 * EXTENT_BYTES as u64 + 777, &write).expect("inside");
        assert_eq!(one, other);
        assert!(one.copied <= 2 * EXTENT_BYTES as u64);
    }

    /// A snapshot names the bytes that were there when it was taken, and a
    /// write after it is invisible to that name.
    #[test]
    fn a_snapshot_publishes_the_bytes_of_its_moment_and_nothing_later() {
        let mut store = a_store(12);
        let bytes = counted(2 * EXTENT_BYTES + 1234);
        let mut extent = Extent::create(&mut store, &bytes).expect("room");

        let (published, cost) = extent.snapshot(&mut store).expect("a record fits");
        assert_eq!(cost.pieces, 0, "a snapshot rewrites no piece");
        assert_eq!(cost.copied, 12 + 3 * 32, "the head and three hashes");

        let after = vec![0xEEu8; 4096];
        extent.write(&mut store, 1024, &after).expect("inside");

        // The owner sees its own write.
        let mut mine = vec![0u8; 4096];
        extent.read(&mut store, 1024, &mut mine).expect("read-your-writes");
        assert_eq!(mine, after);

        // A reader resolving the published hash sees the snapshot, whole.
        let reader = Extent::open(&mut store, &published).expect("a published extent");
        assert_eq!(reader.extent_bytes(), bytes.len() as u64);
        assert_eq!(reader.piece_bytes(), EXTENT_BYTES, "recovered from piece 0's header");
        let mut theirs = vec![0u8; bytes.len()];
        reader.read(&mut store, 0, &mut theirs).expect("the whole extent");
        assert_eq!(theirs, bytes, "the last snapshot, and not a byte written since");

        // And the second snapshot names something else, which is what makes the
        // first assertion above a statement rather than a coincidence.
        let (again, _) = extent.snapshot(&mut store).expect("a second record");
        assert_ne!(again, published);
    }

    /// A write spanning whole pieces takes each piece's bytes from the right
    /// place in the caller's buffer.
    ///
    /// The case is here because it is the one the small-write workload never
    /// reaches and the one a wholly-covered piece gets wrong for free: a middle
    /// piece needs no read, so the branch that skips the read is also the branch
    /// with no old bytes to notice the mistake against, and every assertion
    /// about *cost* stays green while the bytes are wrong. It costs three
    /// megabytes of test to say so and it is worth them.
    #[test]
    fn a_write_over_several_whole_pieces_places_each_one_where_it_belongs() {
        let mut store = a_store(16);
        let bytes = counted(5 * EXTENT_BYTES);
        let mut extent = Extent::create(&mut store, &bytes).expect("room for five pieces");

        // From half a piece in to half a piece from the end of piece 3: piece 1
        // is partial, piece 2 is whole and in the middle, piece 3 is partial.
        let at = EXTENT_BYTES as u64 / 2;
        let written = counted(2 * EXTENT_BYTES + 7);
        let cost = extent.write(&mut store, at, &written).expect("inside");
        assert_eq!(cost.pieces, 3);

        let mut back = vec![0u8; written.len()];
        extent.read(&mut store, at, &mut back).expect("read-your-writes");
        assert_eq!(back, written, "a whole piece in the middle took the wrong megabyte");

        // And the bytes on either side are untouched, which is what says the
        // write did not smear into its neighbours.
        let mut before = vec![0u8; 64];
        extent.read(&mut store, 0, &mut before).expect("the head");
        assert_eq!(before, bytes[..64]);
        let tail_at = at + written.len() as u64;
        let mut after = vec![0u8; 64];
        extent.read(&mut store, tail_at, &mut after).expect("the tail");
        assert_eq!(after, bytes[tail_at as usize..tail_at as usize + 64]);
    }

    /// An extent does not grow, and says so rather than truncating.
    #[test]
    fn a_write_past_the_end_is_refused() {
        let mut store = a_store(4);
        let bytes = counted(EXTENT_BYTES + 16);
        let mut extent = Extent::create(&mut store, &bytes).expect("room");
        let write = vec![0u8; 4096];
        assert_eq!(extent.write(&mut store, bytes.len() as u64 - 8, &write), Err(refusal::ADDRESS));
        assert_eq!(extent.write(&mut store, u64::MAX, &write), Err(refusal::ADDRESS));
    }

    /// Two extents over identical bytes share every piece, and a write breaks
    /// the sharing for exactly one of them.
    ///
    /// This is the deduplication RFC 0058 keeps — a clone shares until it is
    /// written — and it is the half the fixed-offset rule does *not* give up.
    /// What it does give up is two independently built images sharing anything,
    /// and no test can assert an absence like that; the entry names it instead.
    #[test]
    fn a_clone_shares_every_piece_until_it_is_written() {
        let mut store = a_store(12);
        let bytes = counted(4 * EXTENT_BYTES);
        let original = Extent::create(&mut store, &bytes).expect("room");
        let records = store.records();
        let mut clone = Extent::create(&mut store, &bytes).expect("the same bytes again");
        assert_eq!(store.records(), records, "identical pieces are one record");

        clone.write(&mut store, 0, &vec![0x77u8; 4096]).expect("inside");
        assert_eq!(store.records(), records + 1, "one new piece and no more");
        assert_eq!(original.pieces(), clone.pieces());
    }
}
