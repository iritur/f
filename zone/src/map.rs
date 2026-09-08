// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The mapping: a flat block address space on top, zones underneath, and one
//! level of indirection between them so that copy-forward moves bytes without
//! moving a name.
//!
//! # Why there is an indirection at all
//!
//! Because copy-forward moves a live blob to a different zone, and every
//! address the store above holds for that blob would otherwise be wrong.
//! `f_blob::store::Store` allocates a *logical* block for each record it writes
//! and remembers where each hash went; it has no `relocate`, and giving it one
//! would make the store a second thing that knows about zones — exactly the
//! second opinion about an offset that `blob/src/store.rs` declines to have in
//! its own first paragraph.
//!
//! So the store's addresses never move and this map's do. RFC 0059's I2 says
//! the sweep must "repoint `location(h)` at the copy"; here `location(h)` is
//! the composition of the store's hash-to-logical map with this map's
//! logical-to-physical one, and repointing the second half *is* repointing
//! `location(h)`. The invariant is unchanged; what is factored is who holds
//! which half.
//!
//! # The limitation this shape has, said here rather than found later
//!
//! **A logical address is never reused.** `Store` owns the logical allocator —
//! it writes to the block after the last one it wrote — so a map underneath it
//! cannot hand a freed address back. Collection therefore recovers *physical*
//! capacity and not *logical* capacity, and a store that has written the
//! device's capacity once will be refused with [`refusal::FULL`] however empty
//! the device now is. That is enough for a fill-and-collect cycle and is not
//! enough for a store that runs. The fix is not a bigger address space: it is
//! for the logical allocator to be the zone layer's or the index's, which is
//! `E2-B03`'s business, and the reason it is not done here is that moving it
//! changes `f_blob::store::Store`'s allocation — a diff in a task that is
//! closed. *Reversal:* the first workload that outlives its address space.
//!
//! # Sequential fill, and where a record may end up
//!
//! Blocks arrive one at a time, because [`Device::write`] is one block wide,
//! and a record spanning several blocks arrives as several writes with nothing
//! marking where one record ends. **So a record may straddle a zone boundary**,
//! and nothing here prevents it: preventing it would mean the store handing
//! whole records to the zone layer, which is a change to a closed task's
//! interface. What copes with it is the mark, which sets a bit for every block
//! a reachable blob *occupies* rather than for the block it starts at —
//! `mark.rs`'s first paragraph is where that departure from RFC 0059 is argued,
//! and it is why I1 needs no side condition here.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::refusal;
use f_blob::device::Device;

use crate::device::{Kind, Zoned};

/// RFC 0059's `state(z)`, and it is not the device's write pointer.
///
/// The device knows whether bytes have been written; this says whether the
/// collector may take them. `Condemned` has no device counterpart at all, which
/// is the clearest evidence that the two could not have been one type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Reset, holding nothing, available to be opened.
    Free,
    /// Being filled. There is at most one.
    Open,
    /// Filled and finished. A candidate for the sweep.
    Sealed,
    /// Selected by the sweep. `location` will not hand this zone out again,
    /// which is what makes `reads_outstanding` monotone non-increasing from
    /// here — RFC 0059's own argument that the reset guard terminates.
    Condemned,
}

/// What the collector and the invariants know about one zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    /// Which zone this describes.
    /// Unit: zone index, zero-based.
    pub zone: u32,
    /// Where the collector believes the zone stands.
    /// Unit: none — a state, not a quantity.
    pub state: State,
    /// Reachable bytes in the zone, accumulated by the mark and never by a
    /// scan. Between full re-marks it is an *upper bound* — RFC 0059 pays that
    /// cost explicitly so that the sweep reclaims late and never early.
    /// Unit: bytes.
    pub live_bytes: u64,
    /// Readers that have resolved a hash into this zone and not finished. The
    /// third conjunct of the reset guard, and not optional: a reader that
    /// resolved before a relocation is a reader that believes this zone is
    /// live, and I2 is about beliefs as well as bytes.
    /// Unit: count of readers.
    pub reads_outstanding: u32,
}

/// The flat address space the store writes to, and the zones underneath it.
pub struct ZoneMap<Z: Zoned> {
    device: Z,
    /// Unit: count of blocks.
    zone_blocks: u64,
    /// The first zone this map fills with data. Zones below it are the
    /// superblock's conventional zone and the two root zones, which are the
    /// publish path's and never the collector's.
    /// Unit: zone index, zero-based.
    data_from: u32,
    /// Where a logical block currently lives.
    /// Unit: block index, zero-based — logical to device-absolute physical.
    physical: BTreeMap<u64, u64>,
    /// The inverse, so a sweep can ask what a zone holds without scanning the
    /// whole map. Kept beside rather than derived, and every write to one is a
    /// write to the other in the same function — `relocate` and `recycle` are
    /// the only two, which is what makes "they cannot disagree" checkable by
    /// reading two functions rather than the file.
    /// Unit: block index, zero-based — device-absolute physical to logical.
    logical: BTreeMap<u64, u64>,
    /// Per-zone state, for the data zones only.
    zones: BTreeMap<u32, Zone>,
    /// The zone being filled, if one is open.
    /// Unit: zone index, zero-based.
    open: Option<u32>,
    /// Copies appended but not yet made durable and repointed: logical block to
    /// its new physical one.
    ///
    /// This is I2's ordering held as state rather than as a rule a caller
    /// keeps. [`ZoneMap::recycle`] refuses while this is non-empty, so *copy,
    /// `FLUSH`, repoint, only then reset* is enforced by the type: there is no
    /// order of calls in which a reset lands before the copies it depends on
    /// are durable.
    /// Unit: block index, zero-based — logical to device-absolute physical.
    copies: BTreeMap<u64, u64>,
}

impl<Z: Zoned> ZoneMap<Z> {
    /// A map over `device`, reserving `data_from` zones at the bottom for the
    /// superblock and the root zones.
    ///
    /// Logical block 0 is pre-mapped to physical block 0 — the superblock, in
    /// the conventional zone — because that is the one block this format
    /// rewrites and the one write that is not an append.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for a device with no data zone above the reserved
    /// ones, or one whose zone 0 is not conventional. The second is checked
    /// rather than assumed: `f_blob::store::Store::format` records the
    /// obligation that a superblock's zone be rewritable and says the mount is
    /// where it is discharged. This is that mount's half of it.
    pub fn new(device: Z, data_from: u32) -> Result<Self, i32> {
        if data_from == 0 || data_from >= device.zone_count() {
            return Err(refusal::MALFORMED);
        }
        let zone_blocks = device.zone_blocks();
        if zone_blocks == 0 {
            return Err(refusal::MALFORMED);
        }
        let mut zones = BTreeMap::new();
        for zone in data_from..device.zone_count() {
            zones.insert(
                zone,
                Zone { zone, state: State::Free, live_bytes: 0, reads_outstanding: 0 },
            );
        }
        let mut physical = BTreeMap::new();
        let mut logical = BTreeMap::new();
        physical.insert(0, 0);
        logical.insert(0, 0);
        Ok(Self {
            device,
            zone_blocks,
            data_from,
            physical,
            logical,
            zones,
            open: None,
            copies: BTreeMap::new(),
        })
    }

    /// A map over a device that already holds data: the mapping half of a
    /// mount, rebuilt from the device's own write pointers.
    ///
    /// # Why the logical numbering can be recovered at all
    ///
    /// Nothing on the device records which logical block a physical block is.
    /// It does not have to. This map fills zones in ascending index and appends
    /// within a zone in ascending offset, and a logical block is handed out by
    /// the store's allocator in ascending order — so *the physical order of the
    /// written data blocks is the logical order*, and a walk in that order
    /// re-derives the numbering exactly. That is a property of the fill and it
    /// is stated here because it is what the recovery rests on.
    ///
    /// **It is a property the collector breaks**, deliberately: `copy_forward`
    /// moves a logical block to a new zone and the two orders part company. So
    /// this constructor is honest only about a device no collection has run on,
    /// which is what `E2-P01`'s sweep has and what a general mount does not.
    /// *What would reverse this:* `E2-B03`'s index on the device, which records
    /// the association rather than re-deriving it, and is the thing a mount
    /// after a collection needs.
    ///
    /// # What is deliberately not recovered
    ///
    /// Every non-empty data zone comes back `Sealed` and no zone comes back
    /// `Open`, so the next write opens a fresh one rather than resuming a zone
    /// whose remaining capacity this map would have to trust the device about.
    /// `live_bytes` comes back zero, because reachable bytes are what RFC 0059's
    /// mark accumulates and a scan that guessed at them would be a second
    /// answer to a question the mark already owns.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] as [`ZoneMap::new`] gives it; whatever the device
    /// says about a report.
    pub fn remount(device: Z, data_from: u32) -> Result<Self, i32> {
        let mut map = Self::new(device, data_from)?;
        let mut logical = 1u64;
        for zone in data_from..map.device.zone_count() {
            let report = map.device.report(zone)?;
            for at in report.start..report.write_pointer {
                map.physical.insert(logical, at);
                map.logical.insert(at, logical);
                logical += 1;
            }
            if report.write_pointer > report.start {
                map.zones.insert(
                    zone,
                    Zone { zone, state: State::Sealed, live_bytes: 0, reads_outstanding: 0 },
                );
            }
        }
        Ok(map)
    }

    /// The device underneath, for a caller with business there — a counter, a
    /// report, a control that corrupts a byte.
    pub fn device_mut(&mut self) -> &mut Z {
        &mut self.device
    }

    /// The device underneath.
    #[must_use]
    pub const fn device(&self) -> &Z {
        &self.device
    }

    /// Give the device back and drop the mapping over it.
    ///
    /// [`f_blob::store::Store::into_device`]'s reason, one layer down: after a
    /// power cut the mapping describes a state that no longer exists, and
    /// [`ZoneMap::remount`] takes a device by value because there is exactly one
    /// map over a device and re-deriving it is what a mount *is*.
    #[must_use]
    pub fn into_device(self) -> Z {
        self.device
    }

    /// The first zone this map fills.
    ///
    /// Unit: zone index, zero-based.
    #[must_use]
    pub const fn data_from(&self) -> u32 {
        self.data_from
    }

    /// What the collector knows about every data zone, in zone order.
    ///
    /// Ordered because RFC 0004 says an order nobody stated is an order the map
    /// chose, and the sweep's tie-break is *ascending zone index*.
    pub fn zones(&self) -> impl Iterator<Item = &Zone> {
        self.zones.values()
    }

    /// What the collector knows about one zone.
    #[must_use]
    pub fn zone(&self, zone: u32) -> Option<&Zone> {
        self.zones.get(&zone)
    }

    /// How many data zones are `Free`.
    ///
    /// Unit: count of zones. RFC 0059 starts a cycle when this falls below
    /// `COLLECT_FREE_ZONES_LOW`, and the open zone is deliberately not counted:
    /// a zone being filled is not a destination a sweep may evacuate into.
    #[must_use]
    pub fn zones_free(&self) -> u32 {
        self.zones.values().filter(|zone| zone.state == State::Free).count() as u32
    }

    /// Where a logical block currently lives, or `None` for one nothing has
    /// written.
    ///
    /// Unit: block index, zero-based, device-absolute.
    #[must_use]
    pub fn physical(&self, logical: u64) -> Option<u64> {
        self.physical.get(&logical).copied()
    }

    /// Which zone a logical block currently lives in.
    ///
    /// Unit: zone index, zero-based.
    #[must_use]
    pub fn zone_of(&self, logical: u64) -> Option<u32> {
        let block = self.physical(logical)?;
        u32::try_from(block / self.zone_blocks).ok()
    }

    /// The logical blocks a zone currently holds, in ascending physical order.
    ///
    /// Physical order and not logical, because that is the order a copy-forward
    /// reads them in and the order an append will write them in — a sweep that
    /// walked them in logical order would seek across a zone it is about to
    /// discard.
    #[must_use]
    pub fn holds(&self, zone: u32) -> Vec<u64> {
        let start = u64::from(zone) * self.zone_blocks;
        self.logical.range(start..start + self.zone_blocks).map(|(_, logical)| *logical).collect()
    }

    /// Take a reader's belief that a zone is live, so the reset guard can see
    /// it.
    ///
    /// # Why this is a call rather than something `read` does for itself
    ///
    /// Because in a synchronous host model a read begins and ends inside
    /// [`Device::read`], so a count maintained there is zero at every instant
    /// the guard could look at it — and a conjunct that is always true is a
    /// conjunct that tests nothing. What the count is *about* is a reader that
    /// resolved a hash to a zone and has not finished with it, and that
    /// interval is longer than one block read on any device with a queue in
    /// front of it. So it is taken explicitly, by whoever resolved, and
    /// `zone/tests/cycle.rs` holds one across a sweep to prove the guard
    /// refuses.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone this map does not manage.
    pub fn hold_read(&mut self, zone: u32) -> Result<(), i32> {
        let held = self.zones.get_mut(&zone).ok_or(refusal::ADDRESS)?;
        held.reads_outstanding += 1;
        Ok(())
    }

    /// Release a reader's belief.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone this map does not manage or one with no
    /// outstanding reader, which is a caller releasing twice.
    pub fn release_read(&mut self, zone: u32) -> Result<(), i32> {
        let held = self.zones.get_mut(&zone).ok_or(refusal::ADDRESS)?;
        held.reads_outstanding = held.reads_outstanding.checked_sub(1).ok_or(refusal::ADDRESS)?;
        Ok(())
    }

    /// Set a zone's live byte count, which is the mark's to set and nobody
    /// else's.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone this map does not manage.
    pub fn set_live_bytes(&mut self, zone: u32, bytes: u64) -> Result<(), i32> {
        let held = self.zones.get_mut(&zone).ok_or(refusal::ADDRESS)?;
        held.live_bytes = bytes;
        Ok(())
    }

    /// Move a zone to `Condemned`, so `location` stops handing it out.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone this map does not manage;
    /// [`refusal::MALFORMED`] for a zone that is not `Sealed` — a sweep that
    /// condemned the open zone would be evacuating the writer's own
    /// destination.
    pub fn condemn(&mut self, zone: u32) -> Result<(), i32> {
        let held = self.zones.get_mut(&zone).ok_or(refusal::ADDRESS)?;
        if held.state != State::Sealed {
            return Err(refusal::MALFORMED);
        }
        held.state = State::Condemned;
        Ok(())
    }

    /// Copy one logical block forward into the open zone, and **do not**
    /// repoint it.
    ///
    /// The copy is recorded and nothing above this map can see it yet, which is
    /// the first half of I2's ordering. [`ZoneMap::commit_copies`] is the
    /// second half and the only thing that repoints. Splitting them this way —
    /// rather than repointing here and asking the caller to flush afterwards —
    /// is what makes the ordering enforced instead of documented: a repoint
    /// before the barrier would name a block the device has not been told to
    /// keep, and the window is exactly as long as a device write.
    ///
    /// One device operation, so one step of an incremental cycle.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a logical block nothing has written;
    /// [`refusal::FULL`] when no zone is free to open; whatever the device says
    /// about a read or an append.
    pub fn copy_forward(&mut self, logical: u64) -> Result<u64, i32> {
        let from = self.physical.get(&logical).copied().ok_or(refusal::ADDRESS)?;
        let mut block = vec![0u8; self.device.block_bytes()];
        self.device.read(from, &mut block)?;
        let to = self.append_somewhere(&block)?;
        self.copies.insert(logical, to);
        Ok(to)
    }

    /// Make every outstanding copy durable, then repoint. The `FLUSH` in *copy,
    /// `FLUSH`, repoint, reset*.
    ///
    /// One barrier covers every copy taken since the last commit, and that is
    /// the ordering discharged rather than weakened: a barrier is not
    /// per-object, and what I2 requires is that no repoint precede the barrier
    /// covering the copy it names. A barrier per copied block would be one
    /// round trip per block on the work whose whole design is that it never
    /// competes with a deadline-class read.
    ///
    /// Answers how many copies were committed.
    ///
    /// # Errors
    ///
    /// Whatever the device says about the barrier. Nothing is repointed if it
    /// refuses.
    pub fn commit_copies(&mut self) -> Result<usize, i32> {
        if self.copies.is_empty() {
            return Ok(0);
        }
        self.device.flush()?;
        let done = core::mem::take(&mut self.copies);
        let moved = done.len();
        for (logical, to) in done {
            if let Some(from) = self.physical.insert(logical, to) {
                self.logical.remove(&from);
            }
            self.logical.insert(to, logical);
        }
        Ok(moved)
    }

    /// How many copies have been appended and not yet committed.
    ///
    /// Unit: count of blocks.
    #[must_use]
    pub fn copies_outstanding(&self) -> usize {
        self.copies.len()
    }

    /// Reset a condemned zone and drop every mapping that still names it.
    ///
    /// The guard is not here: [`crate::invariants::nothing_reachable_is_swept`]
    /// is the predicate, and a sweep calls it before it calls this. Splitting
    /// them is deliberate — an invariant checked inside the function it
    /// constrains is an invariant that can only be asserted by calling the
    /// thing it is about, which is how `E2-P03` would end up asserting the
    /// code rather than the property.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone this map does not manage;
    /// [`refusal::MALFORMED`] for a zone that is not `Condemned`, or for an
    /// uncommitted copy — the second is I2's ordering, refused here rather than
    /// left to a caller to sequence: a reset that landed before the barrier
    /// covering the copies out of this zone would be exactly the third state
    /// `E2-P01` sweeps for, manufactured by the collector.
    pub fn recycle(&mut self, zone: u32) -> Result<(), i32> {
        let held = *self.zones.get(&zone).ok_or(refusal::ADDRESS)?;
        if held.state != State::Condemned || !self.copies.is_empty() {
            return Err(refusal::MALFORMED);
        }
        let start = u64::from(zone) * self.zone_blocks;
        let stale: Vec<(u64, u64)> = self
            .logical
            .range(start..start + self.zone_blocks)
            .map(|(block, logical)| (*block, *logical))
            .collect();
        self.device.reset(zone)?;
        for (block, logical) in stale {
            self.logical.remove(&block);
            self.physical.remove(&logical);
        }
        self.zones
            .insert(zone, Zone { zone, state: State::Free, live_bytes: 0, reads_outstanding: 0 });
        Ok(())
    }

    /// Open a free zone to append into, sealing the one that was open.
    ///
    /// Ascending zone index, for RFC 0004's reason and not for a device's: an
    /// order nobody stated is an order the map chose, and two runs of the same
    /// workload must lay a device out identically or `E2-P06` has nothing to
    /// compare.
    ///
    /// # Errors
    ///
    /// [`refusal::FULL`] when no zone is free; whatever the device says about a
    /// `ZONE_FINISH`.
    pub fn open_zone(&mut self) -> Result<u32, i32> {
        if let Some(open) = self.open.take() {
            self.device.finish(open)?;
            if let Some(held) = self.zones.get_mut(&open) {
                held.state = State::Sealed;
            }
        }
        let next = self
            .zones
            .values()
            .find(|zone| zone.state == State::Free)
            .map(|zone| zone.zone)
            .ok_or(refusal::FULL)?;
        if let Some(held) = self.zones.get_mut(&next) {
            held.state = State::Open;
        }
        self.open = Some(next);
        Ok(next)
    }

    /// The zone being filled, if one is.
    ///
    /// Unit: zone index, zero-based.
    #[must_use]
    pub const fn open(&self) -> Option<u32> {
        self.open
    }

    /// Append to a zone this map does not manage — a root zone.
    ///
    /// The root zones are the publish path's and never the collector's: nothing
    /// in them is a blob, nothing in them is marked, and the sweep must not be
    /// able to select one. Keeping them out of `zones` is what makes that true
    /// by construction rather than by a filter somebody remembers to write, and
    /// this is the one door into them.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for zone 0 — which is conventional and holds the
    /// superblock — or for any zone this map *does* manage; whatever the device
    /// says about an append.
    pub fn append_reserved(&mut self, zone: u32, from: &[u8]) -> Result<u64, i32> {
        if zone == 0 || zone >= self.data_from {
            return Err(refusal::ADDRESS);
        }
        self.device.append(zone, from)
    }

    /// Append one block into the open zone, opening one if none is open and
    /// sealing a full one on the way.
    fn append_somewhere(&mut self, from: &[u8]) -> Result<u64, i32> {
        if self.open.is_none() {
            self.open_zone()?;
        }
        let open = self.open.ok_or(refusal::FULL)?;
        match self.device.append(open, from) {
            Err(full) if full == refusal::FULL => {
                // The zone filled. Seal it and open the next: a sealed zone is
                // what the sweep selects from, and sealing on the append that
                // did not fit is what makes "sealed" mean "the writer is
                // finished with it" rather than "some time passed".
                let next = self.open_zone()?;
                self.device.append(next, from)
            }
            other => other,
        }
    }
}

impl<Z: Zoned> Device for ZoneMap<Z> {
    fn block_bytes(&self) -> usize {
        self.device.block_bytes()
    }

    fn blocks(&self) -> u64 {
        // The superblock's block plus every data zone's capacity. Not the
        // device's own block count: the root zones are the publish path's and a
        // store that could allocate into them would overwrite a root record
        // with a chunk. And see the module's *limitation* paragraph for what
        // this number does and does not bound.
        1 + u64::from(self.device.zone_count() - self.data_from) * self.zone_blocks
    }

    fn read(&mut self, block: u64, into: &mut [u8]) -> Result<(), i32> {
        let at = self.physical.get(&block).copied().ok_or(refusal::ADDRESS)?;
        self.device.read(at, into)
    }

    fn write(&mut self, block: u64, from: &[u8]) -> Result<(), i32> {
        if let Some(at) = self.physical.get(&block).copied() {
            // A block this map has already placed. Only the conventional zone
            // takes a second write, and the device refuses the rest — which is
            // the check rather than a condition written twice.
            let zone = u32::try_from(at / self.zone_blocks).map_err(|_| refusal::ADDRESS)?;
            if self.device.kind(zone) != Kind::Conventional {
                return Err(refusal::ADDRESS);
            }
            return self.device.write(at, from);
        }
        let at = self.append_somewhere(from)?;
        self.physical.insert(block, at);
        self.logical.insert(at, block);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), i32> {
        self.device.flush()
    }
}

/// Every logical block this map holds, in ascending logical order.
///
/// A free function rather than a method because it is the mark's view of the
/// map and not the map's own: a full re-mark clears the bitmap and walks every
/// root, and this is what tells it which positions exist to be cleared.
pub fn placed<Z: Zoned>(map: &ZoneMap<Z>) -> BTreeSet<u64> {
    map.physical.keys().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::{State, ZoneMap, placed};
    use crate::device::{Zoned, ZonedMemory};
    use alloc::vec;
    use alloc::vec::Vec;
    use f_abi::store::refusal;
    use f_blob::device::Device;

    const BLOCK: usize = 512;

    /// Four blocks a zone, six zones, one of them reserved. Small enough that a
    /// zone fills in four writes, which is what makes sealing testable without
    /// a workload.
    fn map() -> ZoneMap<ZonedMemory> {
        ZoneMap::new(ZonedMemory::new(BLOCK, 4, 6), 1).expect("a device with data zones")
    }

    #[test]
    fn the_superblock_is_the_one_block_that_takes_a_second_write() {
        let mut map = map();
        let block = vec![1u8; BLOCK];
        assert_eq!(map.write(0, &block), Ok(()));
        assert_eq!(map.write(0, &block), Ok(()), "the conventional zone is rewritable");
        assert_eq!(map.physical(0), Some(0));

        assert_eq!(map.write(1, &block), Ok(()));
        assert_eq!(map.write(1, &block), Err(refusal::ADDRESS), "and nothing else is");
    }

    #[test]
    fn filling_a_zone_seals_it_and_opens_the_next() {
        let mut map = map();
        let block = vec![2u8; BLOCK];
        for logical in 1..=5 {
            map.write(logical, &block).expect("five blocks into four-block zones");
        }
        assert_eq!(map.zone(1).map(|z| z.state), Some(State::Sealed));
        assert_eq!(map.zone(2).map(|z| z.state), Some(State::Open));
        assert_eq!(map.open(), Some(2));
        assert_eq!(map.holds(1).len(), 4);
        assert_eq!(map.holds(2), vec![5]);
    }

    /// The ordering I2 guards, from both sides: a copy is invisible until the
    /// barrier, and visible from the same logical name afterwards.
    #[test]
    fn a_copy_is_not_repointed_until_the_barrier_and_then_it_is() {
        let mut map = map();
        let mut block = vec![0u8; BLOCK];
        block[0] = 0xAB;
        map.write(1, &block).expect("one block");
        map.open_zone().expect("seal zone 1 and open zone 2");

        let was = map.physical(1).expect("logical 1 is placed");
        let moved = map.copy_forward(1).expect("a copy into the open zone");
        assert_ne!(moved, was);
        assert_eq!(map.physical(1), Some(was), "nothing above the map can see the copy yet");
        assert_eq!(map.copies_outstanding(), 1);

        assert_eq!(map.commit_copies(), Ok(1));
        assert_eq!(map.physical(1), Some(moved));
        assert_eq!(map.zone_of(1), Some(2));
        assert!(map.holds(1).is_empty(), "the old zone no longer holds it");

        let mut read = vec![0u8; BLOCK];
        map.read(1, &mut read).expect("the same logical block");
        assert_eq!(read, block, "the same bytes, one zone over");
    }

    #[test]
    fn a_reset_is_refused_while_a_copy_out_of_the_zone_is_not_durable() {
        let mut map = map();
        let block = vec![4u8; BLOCK];
        map.write(1, &block).expect("one block into zone 1");
        map.open_zone().expect("seal it");
        map.condemn(1).expect("sealed becomes condemned");
        map.copy_forward(1).expect("evacuate it");

        assert_eq!(map.recycle(1), Err(refusal::MALFORMED), "the copy is not durable yet");
        map.commit_copies().expect("the barrier");
        map.recycle(1).expect("and now the reset is allowed");
    }

    #[test]
    fn recycling_drops_every_mapping_that_named_the_zone() {
        let mut map = map();
        let block = vec![3u8; BLOCK];
        map.write(1, &block).expect("one block into zone 1");
        map.open_zone().expect("seal it");

        assert_eq!(map.recycle(1), Err(refusal::MALFORMED), "a sealed zone is not condemned");
        map.condemn(1).expect("sealed becomes condemned");
        map.recycle(1).expect("a condemned zone resets");

        assert_eq!(map.zone(1).map(|z| z.state), Some(State::Free));
        assert_eq!(map.physical(1), None);
        assert!(map.holds(1).is_empty());
        assert_eq!(placed(&map), [0].into_iter().collect(), "only the superblock is left");
        let mut read = vec![0u8; BLOCK];
        assert_eq!(map.read(1, &mut read), Err(refusal::ADDRESS));
    }

    #[test]
    fn a_reader_that_resolved_into_a_zone_is_visible_to_the_guard() {
        let mut map = map();
        map.hold_read(1).expect("a data zone");
        assert_eq!(map.zone(1).map(|z| z.reads_outstanding), Some(1));
        map.release_read(1).expect("the same reader, finished");
        assert_eq!(map.zone(1).map(|z| z.reads_outstanding), Some(0));
        assert_eq!(map.release_read(1), Err(refusal::ADDRESS), "releasing twice is a defect");
    }

    #[test]
    fn a_device_with_no_data_zone_is_refused_rather_than_mounted_empty() {
        let device = ZonedMemory::new(BLOCK, 4, 2);
        assert_eq!(device.zone_count(), 2);
        assert!(ZoneMap::new(ZonedMemory::new(BLOCK, 4, 2), 2).is_err());
        assert!(ZoneMap::new(ZonedMemory::new(BLOCK, 4, 2), 0).is_err());
    }

    #[test]
    fn the_address_space_is_the_data_zones_and_not_the_device() {
        let map = map();
        // Five data zones of four blocks, plus the superblock's own.
        assert_eq!(map.blocks(), 1 + 5 * 4);
        assert_eq!(map.zones_free(), 5);
    }

    #[test]
    fn a_remount_re_derives_the_logical_order_from_the_physical_one() {
        let mut map = map();
        // Six blocks into four-block zones, so the fill crosses a zone boundary
        // and the recovered numbering has to follow it.
        for logical in 1..=6u64 {
            let mut block = vec![0u8; BLOCK];
            block[0] = logical as u8;
            map.write(logical, &block).expect("room in the data zones");
        }
        let placed_before: Vec<(u64, Option<u64>)> =
            (1..=6).map(|logical| (logical, map.physical(logical))).collect();

        let mut back = ZoneMap::remount(map.into_device(), 1).expect("the same device");
        assert_eq!(
            placed_before,
            (1..=6).map(|logical| (logical, back.physical(logical))).collect::<Vec<_>>(),
            "every logical block is where the fill put it"
        );
        // And the bytes are readable through the recovered map, which is the
        // only thing a mount actually needs of it.
        let mut read = vec![0u8; BLOCK];
        back.read(6, &mut read).expect("the last block written");
        assert_eq!(read[0], 6);
        // Nothing comes back open: the next write opens a fresh zone rather than
        // resuming one whose remaining capacity this map would have to trust the
        // device about.
        assert_eq!(back.open(), None);
    }

    #[test]
    fn a_remount_of_an_untouched_device_finds_nothing_and_says_so() {
        let mut back = ZoneMap::remount(ZonedMemory::new(BLOCK, 4, 6), 1).expect("a device");
        assert_eq!(back.physical(1), None);
        assert_eq!(back.zones_free(), 5, "every data zone is free");
        assert_eq!(placed(&back).len(), 1, "the superblock's block and nothing else");
        let mut read = vec![0u8; BLOCK];
        assert_eq!(back.read(1, &mut read), Err(refusal::ADDRESS));
    }
}
