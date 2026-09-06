// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `B` — the mark bitmap — and the walk that fills it.
//!
//! # What a set bit means here, and where that departs from RFC 0059
//!
//! RFC 0059 writes `B` as "one bit per block-aligned position in the store. A
//! set bit means *a blob starts here and is reachable*." **This crate sets a
//! bit for every block a reachable blob occupies, not only for the block it
//! starts at**, and the departure is deliberate rather than a slip.
//!
//! I1 is checked as *every bit of `B` in the condemned zone's range is clear*.
//! Under a bitmap of starts that predicate is satisfied by a zone holding the
//! second, third and last blocks of a live record whose first block is in the
//! previous zone — and every blob larger than one block has such blocks. So the
//! RFC's `B` makes I1 *insufficient*, not merely tight, and the shortfall is not
//! about anything exotic: `CHUNK_TARGET_BYTES` is 64 KiB and a block is 512
//! bytes, so the typical chunk is a hundred and twenty-nine blocks of which one
//! would carry a bit. Occupancy costs the same memory — one bit per block
//! either way, the 32 MiB per TiB claim 0019 counts — and makes the predicate
//! sufficient. That is the whole argument, and it is reported to RFC 0059's
//! author rather than fixed by editing that file.
//!
//! `live_bytes(z)` follows the same change and lands in the same place: RFC
//! 0059 adds "that blob's `content_bytes` rounded up to `block_bytes`" to the
//! zone the *bit* belongs to, which mis-attributes a straddling record wholly
//! to one zone; this adds one block's worth per occupied block, to the zone that
//! block is in. The two agree exactly whenever a record does not straddle, and
//! only the second is right when one does.
//!
//! # Generational, within a cycle. Not across one, and that is a build
//! limitation rather than a decision
//!
//! A walk that reaches an already-marked node stops, and that is sound because
//! a blob's content — and therefore every hash it names — is fixed at the
//! moment it is written. There is no old-to-young pointer because there is no
//! pointer write at all. [`Mark::seen`] is that rule and it holds within a
//! cycle.
//!
//! **Across cycles this build re-marks from scratch every time.** Carrying a
//! mark from one cycle into the next requires `live_bytes(z)` to be maintained
//! incrementally against additions *and* against copy-forward moving blocks
//! between zones, and this build recomputes it instead. The cost is a walk of
//! the live set per cycle rather than per removal; the direction is
//! conservative — every `live_bytes` is exact rather than an over-estimate, so
//! the sweep still never reclaims early. *Reversal:* a workload where the mark
//! is a measurable share of the cycle's device reads, which is a number
//! `E2-P03` can take and this crate cannot.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::{Header, ObjectHead, kind, refusal};
use f_blob::store::Store;

use crate::device::Zoned;
use crate::map::ZoneMap;
use crate::roots::Roots;

/// One bit per block-aligned position in the store.
///
/// A `Vec<u64>` and not a `BTreeSet<u64>`, and the reason is the resident cost
/// RFC 0059 states rather than a preference: one bit per device block is
/// 1/32768 of the device's capacity, 32 MiB per TiB, and a set would be that
/// times the size of a node. The bitmap is the collector's whole memory budget
/// and a small component's quota is sized for it.
#[derive(Debug)]
pub struct Bitmap {
    words: Vec<u64>,
    /// Unit: count of bits — one per block-aligned position.
    bits: u64,
}

impl Bitmap {
    /// A bitmap over `bits` positions, all clear.
    ///
    /// # Panics
    ///
    /// If the bitmap would be larger than this machine can allocate, which is
    /// the allocator's panic and not a refusal: a mount that asks for a bitmap
    /// it cannot have is a mount of a device this host cannot serve, and a
    /// `Result` here would let it carry on with a bitmap that does not cover
    /// the device.
    #[must_use]
    pub fn new(bits: u64) -> Self {
        let words = usize::try_from(bits.div_ceil(64)).expect("a device this host can address");
        Self { words: vec![0u64; words], bits }
    }

    /// How many positions this bitmap covers.
    ///
    /// Unit: count of bits.
    #[must_use]
    pub const fn bits(&self) -> u64 {
        self.bits
    }

    /// Set one bit. Answers whether it was already set, which is the
    /// deduplication: a blob reachable from four roots is counted once.
    pub fn set(&mut self, at: u64) -> bool {
        if at >= self.bits {
            return true;
        }
        let word = (at / 64) as usize;
        let bit = 1u64 << (at % 64);
        let was = self.words[word] & bit != 0;
        self.words[word] |= bit;
        was
    }

    /// Whether one bit is set.
    #[must_use]
    pub fn get(&self, at: u64) -> bool {
        if at >= self.bits {
            return false;
        }
        self.words[(at / 64) as usize] & (1u64 << (at % 64)) != 0
    }

    /// Clear every bit — what a full re-mark begins with.
    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    /// Whether any bit in `from..to` is set. I1's second conjunct, in one call.
    #[must_use]
    pub fn any(&self, from: u64, to: u64) -> bool {
        (from..to.min(self.bits)).any(|at| self.get(at))
    }

    /// How many bits are set.
    ///
    /// Unit: count of bits.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.words.iter().map(|word| u64::from(word.count_ones())).sum()
    }
}

/// One reachable record, as the sweep needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// The record's first block, in the store's address space.
    /// Unit: block index, zero-based, logical.
    pub start: u64,
    /// How many blocks the record occupies.
    /// Unit: count of blocks.
    pub blocks: u64,
    /// What the record is called.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub hash: [u8; 32],
}

/// What the walk produced.
#[derive(Debug)]
pub struct Mark {
    /// `B`.
    bits: Bitmap,
    /// `marked_roots` — the roots whose walk has completed into the current
    /// `B`.
    roots: BTreeSet<[u8; 32]>,
    /// Every reachable record, keyed by its first logical block.
    live: BTreeMap<u64, Extent>,
    /// `live_bytes(z)`, accumulated by the walk and never by a scan.
    live_bytes: BTreeMap<u32, u64>,
    /// Hashes still to walk.
    pending: Vec<[u8; 32]>,
    /// Hashes already walked. A walk that reaches one of these stops.
    seen: BTreeSet<[u8; 32]>,
    /// Unit: count of hashes in `R`'s closure that the store holds nothing
    /// under. Zero on a store nothing has damaged; anything else is a root
    /// naming a blob that is not there, which is the state `E2-P01` sweeps for.
    unresolved: u64,
    /// Unit: count of device reads the walk has issued.
    reads: u64,
}

impl Mark {
    /// A cleared mark over a device of `blocks` blocks.
    #[must_use]
    pub fn new(blocks: u64) -> Self {
        Self {
            bits: Bitmap::new(blocks),
            roots: BTreeSet::new(),
            live: BTreeMap::new(),
            live_bytes: BTreeMap::new(),
            pending: Vec::new(),
            seen: BTreeSet::new(),
            unresolved: 0,
            reads: 0,
        }
    }

    /// `B`.
    #[must_use]
    pub const fn bits(&self) -> &Bitmap {
        &self.bits
    }

    /// `marked_roots`.
    #[must_use]
    pub const fn marked_roots(&self) -> &BTreeSet<[u8; 32]> {
        &self.roots
    }

    /// Every reachable record.
    #[must_use]
    pub const fn live(&self) -> &BTreeMap<u64, Extent> {
        &self.live
    }

    /// `live_bytes(z)` for one zone.
    ///
    /// Unit: bytes.
    #[must_use]
    pub fn live_bytes(&self, zone: u32) -> u64 {
        self.live_bytes.get(&zone).copied().unwrap_or(0)
    }

    /// Reachable bytes across every zone.
    ///
    /// Unit: bytes.
    #[must_use]
    pub fn live_bytes_total(&self) -> u64 {
        self.live_bytes.values().sum()
    }

    /// Hashes in the closure of `R` that the store holds nothing under.
    ///
    /// Unit: count of hashes.
    #[must_use]
    pub const fn unresolved(&self) -> u64 {
        self.unresolved
    }

    /// Device reads the walk has issued.
    ///
    /// Unit: count of device reads.
    #[must_use]
    pub const fn reads(&self) -> u64 {
        self.reads
    }

    /// Whether the walk has anything left to do.
    #[must_use]
    pub fn walking(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Clear everything: what a full re-mark begins with.
    pub fn restart(&mut self) {
        self.bits.clear();
        self.roots.clear();
        self.live.clear();
        self.live_bytes.clear();
        self.pending.clear();
        self.seen.clear();
        self.unresolved = 0;
    }

    /// Add a root to walk. Roots added during a cycle are marked *within* the
    /// cycle, before any sweep in it may condemn — which is what makes
    /// `marked_roots ⊇ R` an invariant of the cycle rather than a race the
    /// sweep has to think about.
    pub fn walk(&mut self, root: [u8; 32]) {
        if !self.seen.contains(&root) {
            self.pending.push(root);
        }
    }

    /// One step of the walk: at most one record read plus bounded computation.
    ///
    /// *Bounded by the record and not by one block*, which is a hair's width
    /// looser than RFC 0059's "at most one device read": an object's record is
    /// read whole so that its chunk list can be decoded, and a record is at
    /// most `CHUNK_MAX_BYTES` = 256 KiB. Reading a chunk list block by block
    /// and holding a cursor is `E2-B08`'s resolver's shape, not this walk's, and
    /// building it here would be building it twice.
    ///
    /// Answers whether a step was taken.
    ///
    /// # Errors
    ///
    /// Whatever the store says about a read that was not a missing hash — a
    /// hash the store holds nothing under is counted in [`Mark::unresolved`]
    /// and the walk carries on, because refusing the whole cycle over one
    /// missing blob would leave a store that cannot collect *and* cannot be
    /// repaired.
    pub fn step<Z: Zoned>(&mut self, store: &mut Store<ZoneMap<Z>>) -> Result<bool, i32> {
        let Some(hash) = self.pending.pop() else { return Ok(false) };
        if !self.seen.insert(hash) {
            return Ok(true);
        }
        let Some(start) = store.address(&hash) else {
            self.unresolved += 1;
            self.roots.insert(hash);
            return Ok(true);
        };

        let header = store.head(&hash)?;
        self.reads += 1;
        let block_bytes = store.superblock().block_bytes as u64;
        let blocks = (Header::BYTES as u64 + header.content_bytes).div_ceil(block_bytes);
        self.occupy(store, Extent { start, blocks, hash });

        if header.kind == kind::OBJECT {
            self.children(store, &hash, &header)?;
        }
        self.roots.insert(hash);
        Ok(true)
    }

    /// Push an object's chunks onto the work list.
    fn children<Z: Zoned>(
        &mut self,
        store: &mut Store<ZoneMap<Z>>,
        hash: &[u8; 32],
        header: &Header,
    ) -> Result<(), i32> {
        let content_bytes =
            usize::try_from(header.content_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        let mut content = vec![0u8; content_bytes];
        store.get(hash, &mut content)?;
        self.reads += 1;
        if content.len() < ObjectHead::HEAD_BYTES {
            return Err(refusal::MALFORMED);
        }
        let mut raw = [0u8; ObjectHead::HEAD_BYTES];
        raw.copy_from_slice(&content[..ObjectHead::HEAD_BYTES]);
        let head = ObjectHead::from_bytes(&raw)?;
        if head.bytes() != content.len() {
            return Err(refusal::MALFORMED);
        }
        for index in 0..head.chunks as usize {
            let at = ObjectHead::HEAD_BYTES + index * 32;
            let mut child = [0u8; 32];
            child.copy_from_slice(&content[at..at + 32]);
            self.walk(child);
        }
        Ok(())
    }

    /// Mark every block a record occupies and accrue its bytes to the zones
    /// those blocks are in.
    fn occupy<Z: Zoned>(&mut self, store: &Store<ZoneMap<Z>>, extent: Extent) {
        self.live.insert(extent.start, extent);
        let block_bytes = u64::from(store.superblock().block_bytes);
        for offset in 0..extent.blocks {
            let logical = extent.start + offset;
            let Some(physical) = store.device().physical(logical) else { continue };
            if !self.bits.set(physical) {
                let zone = (physical / store.device().device().zone_blocks()) as u32;
                *self.live_bytes.entry(zone).or_insert(0) += block_bytes;
            }
        }
    }

    /// Mark the blocks an open write set has appended, which have no hash to
    /// walk from.
    ///
    /// This is the whole of RFC 0059's first decision in code: an entry in `T`
    /// is protected by being marked, so the zone it lies in has a non-zero
    /// `live_bytes` and is not selected, and even if it were the reset guard
    /// would find bits set. One predicate covers the pinned case and the open
    /// case.
    pub fn occupy_transient<Z: Zoned>(
        &mut self,
        store: &Store<ZoneMap<Z>>,
        blocks: &BTreeSet<u64>,
    ) {
        let block_bytes = u64::from(store.superblock().block_bytes);
        for logical in blocks {
            let Some(physical) = store.device().physical(*logical) else { continue };
            if !self.bits.set(physical) {
                let zone = (physical / store.device().device().zone_blocks()) as u32;
                *self.live_bytes.entry(zone).or_insert(0) += block_bytes;
            }
        }
    }

    /// Recompute `B` and every `live_bytes(z)` from the live set and the map as
    /// it now stands.
    ///
    /// Called after a copy-forward has been committed, because a copy moves a
    /// block between zones and both numbers are about where a block *is*. It is
    /// a recomputation and not an adjustment for the reason a full re-mark is a
    /// walk and not a decrement: an adjustment is a second piece of arithmetic
    /// that has to agree with the first, and the two disagree the day somebody
    /// adds a case.
    pub fn rebuild<Z: Zoned>(&mut self, store: &Store<ZoneMap<Z>>, roots: &Roots) {
        self.bits.clear();
        self.live_bytes.clear();
        let extents: Vec<Extent> = self.live.values().copied().collect();
        for extent in extents {
            self.occupy(store, extent);
        }
        self.occupy_transient(store, &roots.transient_blocks());
    }

    /// Publish every `live_bytes(z)` into the map, where the sweep and the
    /// invariants read it.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone the map does not manage, which cannot
    /// happen for a zone a block was found in and is checked rather than
    /// assumed.
    pub fn publish<Z: Zoned>(&self, map: &mut ZoneMap<Z>) -> Result<(), i32> {
        let zones: Vec<u32> = map.zones().map(|zone| zone.zone).collect();
        for zone in zones {
            map.set_live_bytes(zone, self.live_bytes(zone))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Bitmap;

    #[test]
    fn a_bit_answers_once_and_the_second_set_is_the_deduplication() {
        let mut bits = Bitmap::new(200);
        assert!(!bits.set(7), "not previously set");
        assert!(bits.set(7), "and now it is");
        assert!(bits.get(7));
        assert_eq!(bits.count(), 1);
    }

    #[test]
    fn a_range_query_is_i1s_second_conjunct() {
        let mut bits = Bitmap::new(200);
        bits.set(70);
        assert!(bits.any(64, 128));
        assert!(!bits.any(0, 64));
        assert!(!bits.any(128, 200));
        bits.clear();
        assert!(!bits.any(0, 200));
        assert_eq!(bits.count(), 0);
    }

    #[test]
    fn a_position_the_device_does_not_have_is_neither_set_nor_read() {
        let mut bits = Bitmap::new(64);
        assert!(bits.set(64), "out of range answers as already set, so nothing is counted");
        assert!(!bits.get(64));
        assert_eq!(bits.count(), 0);
        assert_eq!(bits.bits(), 64);
    }
}
