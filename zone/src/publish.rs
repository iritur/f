// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The half of RFC 0060's publish sequence `blob/` does not own: the root
//! record's append and the second `FLUSH`.
//!
//! # The sequence, and why it is one function
//!
//! **Write the blobs, `FLUSH`, append the root record, `FLUSH`.** The first
//! barrier is what makes the root record's meaning true — every hash it names
//! is on stable media before the record naming them exists — and the second is
//! what makes the record itself durable before the next publish is allowed to
//! start. `f_blob::store::Store::barrier` is the first; the last three steps
//! are [`Publisher::commit`], in that order, in one function, because RFC
//! 0059's transient root is released *after* the completion of the second
//! barrier and *after* the pinned set has adopted the root — and a sequence
//! split across three calls is a sequence a caller can get right on Monday.
//!
//! # Atomicity is not free, and correctness does not rest on the barrier
//!
//! A device may acknowledge a barrier it did not honour, and nothing above it
//! can tell. So the barrier buys the *rate* at which a publish survives a power
//! cut, and [`RootRecord`]'s `check` buys the *bound*: a torn record is refused
//! and the mount falls to the previous generation, which is a rollback rather
//! than a machine naming a state it cannot produce.
//!
//! # The root-zone wrap, and the one ordering in it that matters
//!
//! A sequential zone fills, and the only operation on a full one is a reset that
//! destroys every record in it. So there are two root zones. Records append to
//! the active one; when its remaining capacity falls below [`ROOT_CARRY`] = 16
//! records, the last sixteen are carried to the other zone, `FLUSH`, and the
//! other zone becomes active. The guaranteed rollback depth immediately after a
//! wrap is therefore sixteen generations rather than one.
//!
//! **The zone that is reset is the older one, and that is the whole safety
//! argument.** The spec's sentence is *carry, `FLUSH`, and only then reset the
//! full zone*, which reads as though the retiring zone is what gets cleared.
//! Clearing it is unnecessary and it is the one ordering a lying device could
//! turn into a machine with no root at all: acknowledge the carry's barrier,
//! land the reset, lose the carry. So [`wrap`] resets the *destination* — which
//! holds the oldest generations, and is empty on the first wrap — copies into
//! it, and leaves the retiring zone exactly as it is until the wrap after next
//! needs it. At no instant is there a root zone whose contents are both the
//! newest and unwritten elsewhere. `E2-P01`'s sweep cuts across a wrap in both
//! `honest` and `lying` mode, which is what says this holds rather than reads
//! well.
//!
//! # What is not here, and what that costs
//!
//! **The other half of verify-before-accept.** [`latest_root`] verifies a root
//! record's `check` and answers the highest generation that survives it. It
//! does *not* check that every child of the generation node resolves — that is
//! [`crate::mount::mount`]'s, because resolving needs a hash-to-block map and
//! this function has a device and no map. A caller holding a live store wants
//! this one; a caller that has just been handed a device wants that one.

use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::{RootRecord, refusal};
use f_blob::device::Device;
use f_blob::store::Store;
use f_hash::sha256;

use crate::device::{Pointer, Zoned};
use crate::map::ZoneMap;
use crate::roots::{Roots, WriteSet};

/// One publish in progress: an open write set, and where the store's address
/// space stood when it opened.
///
/// # Why the write set's blocks are a range and not a list the store keeps
///
/// `f_blob::store::Store` allocates sequentially from the first free block and
/// nothing is ever overwritten, so every block between the store's `free` when
/// the publish opened and its `free` now was appended by this publish. Reading
/// it off the allocator is exact and costs nothing; the alternative is the
/// store reporting each block as it writes it, which is a second interface into
/// a closed task for a number that is already there.
pub struct Publisher {
    set: WriteSet,
    /// The store's first free block when this publish last recorded its
    /// appends.
    /// Unit: block index, zero-based, logical.
    at: u64,
}

impl Publisher {
    /// Open a write set and take the transient entry that protects it.
    ///
    /// Taken *before* the first blob is written and not after, because RFC 0059
    /// releases at the second `FLUSH` and takes at the first `ZONE_APPEND`:
    /// there is no instant at which a written blob is named by nothing.
    pub fn open<Z: Zoned>(roots: &mut Roots, store: &Store<ZoneMap<Z>>) -> Self {
        Self { set: roots.open(), at: store.free() }
    }

    /// Which write set this is.
    #[must_use]
    pub const fn set(&self) -> WriteSet {
        self.set
    }

    /// Record every block the store has appended since this was last called.
    ///
    /// Call it after each blob or after each object; calling it once at the end
    /// is *also* correct here, because nothing between the two points is
    /// visible to the collector until [`Collector::step`] looks — but it is
    /// called per object by `zone/tests/cycle.rs` so that a collector stepping
    /// between two objects of one publish sees the first one protected.
    ///
    /// [`Collector::step`]: crate::collect::Collector::step
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write set that is not open.
    pub fn appended<Z: Zoned>(
        &mut self,
        roots: &mut Roots,
        store: &Store<ZoneMap<Z>>,
    ) -> Result<(), i32> {
        for block in self.at..store.free() {
            roots.appended(self.set, block)?;
        }
        self.at = store.free();
        Ok(())
    }

    /// `FLUSH`, append the root record, `FLUSH`, then adopt-then-drop.
    ///
    /// Answers the record that was appended, so that a caller holds the
    /// generation number the device now carries rather than the one it asked
    /// for.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write set that is not open;
    /// [`refusal::FULL`] when neither root zone has room — see this module's
    /// note on `ROOT_CARRY`; whatever the device says about a barrier or an
    /// append.
    pub fn commit<Z: Zoned>(
        mut self,
        store: &mut Store<ZoneMap<Z>>,
        roots: &mut Roots,
        record: &RootRecord,
    ) -> Result<RootRecord, i32> {
        self.appended(roots, store)?;

        // Step 2. Every hash the record is about to name is on stable media
        // before the record naming them exists.
        //
        // The condition is a deliberate defect and not a policy: with
        // `mutate-root-before-blobs` armed this barrier is skipped, a cut can
        // then land a root record over blobs that did not, and `cargo xtask cut`
        // requires the sweep to find it. Without the defect there is no
        // configuration in which the barrier is not issued.
        if blobs_are_durable_before_the_root() {
            store.barrier()?;
        }

        let mut written = *record;
        written.check = sha256(&record.checked());
        let block_bytes = store.superblock().block_bytes as usize;
        let mut block = vec![0u8; block_bytes];
        block[..RootRecord::BYTES].copy_from_slice(&written.to_bytes());

        // Step 3. Appended and not written: the device assigns the position, so
        // there is one write pointer for a mount to read rather than two to
        // reconcile.
        let zone = root_zone_for_the_next_record(store)?;
        store.device_mut().append_reserved(zone, &block)?;

        // Step 4. And only now is the record itself durable.
        store.device_mut().flush()?;

        // Add, then drop. Not two calls: `Roots::adopt` is one function because
        // a one-line reordering here reintroduces a window in which the blobs
        // are named by nothing, and the window is exactly as long as a device
        // write.
        roots.adopt(self.set, written.root)?;
        Ok(written)
    }

    /// Give up on a publish that never became durable, releasing its transient
    /// entry and scheduling the full re-mark that costs.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write set that is not open.
    pub fn abandon(self, roots: &mut Roots) -> Result<(), i32> {
        roots.abandon(self.set)
    }
}

/// Whether the blobs a root record names are made durable before the record
/// that names them exists.
///
/// True, and RFC 0060's first barrier is the whole of the argument. It is a
/// function rather than a line of code so that a build can be made without it —
/// see [`blobs_are_durable_before_the_root`]'s armed twin below, and `cargo
/// xtask cut`, which requires the sweep to find what that build does.
#[cfg(not(feature = "mutate-root-before-blobs"))]
const fn blobs_are_durable_before_the_root() -> bool {
    true
}

/// The defect itself: append the root record without waiting for the blobs it
/// names.
///
/// A sweep that has never failed is indistinguishable from one that cannot. Every
/// cut in a correct publish leaves the old root or the new one, and it does so
/// *whether or not* the first barrier was issued — because a root whose blobs
/// did not land is refused by resolution and the mount falls back, which is a
/// rollback and not a third state. So the headline property cannot see this
/// defect, and the property that can is the one the barrier is actually for:
/// **in `honest` mode no root is ever refused for non-resolution**. That is what
/// `zone/tests/cut.rs` asserts and what this makes fail. RFC 0060, RFC 0040 for
/// why a defect lives in the shipped source behind a feature rather than in a
/// patch somebody has to remember.
#[cfg(feature = "mutate-root-before-blobs")]
const fn blobs_are_durable_before_the_root() -> bool {
    false
}

/// Records carried to the other root zone when the active one is nearly full.
///
/// Unit: count of root records.
///
/// Sixteen, from the spec, and the number is a rollback depth rather than a
/// capacity: immediately after a wrap this is how many generations a mount can
/// still reach, and `E2-P07`'s demonstration needs somewhere to go whatever
/// point of the cycle a stranger's machine was cut at. One would satisfy the
/// letter of *there is never a moment with no durable root* and would leave a
/// machine one bad record away from having nowhere to roll back to.
pub const ROOT_CARRY: usize = 16;

/// The root zone the next record appends to, wrapping to the other one when this
/// one is nearly full.
///
/// The active zone is the one holding the highest generation, read from the
/// device rather than remembered: the format maintains no pointer of its own —
/// that is what makes a mount after a cut have nothing to reconcile — and a
/// cached answer would be the second copy RFC 0060 refused.
///
/// # Errors
///
/// [`refusal::FULL`] when a zone with room cannot be reached; whatever the
/// device says about a report, a read, an append, a reset or a barrier.
fn root_zone_for_the_next_record<Z: Zoned>(store: &mut Store<ZoneMap<Z>>) -> Result<u32, i32> {
    let (a, b) = (store.superblock().root_zone_a, store.superblock().root_zone_b);
    let (top_a, top_b) = (highest_in(store, a)?, highest_in(store, b)?);
    // Ties go to A, and the only tie is *both empty*: two records cannot carry
    // the same generation, because the generation is the publish counter.
    let (active, other) = if top_b > top_a { (b, a) } else { (a, b) };

    let report = store.device_mut().device_mut().report(active)?;
    let written = report.write_pointer - report.start;
    let remaining = report.blocks - written;
    if report.state != Pointer::Full && remaining >= ROOT_CARRY as u64 {
        return Ok(active);
    }
    wrap(store, active, other)
}

/// Carry the last [`ROOT_CARRY`] records from `active` into `other`, and answer
/// `other`.
///
/// The reset is of the destination and not of the zone being retired; this
/// module's second section is the argument for that, and it is the ordering the
/// whole wrap exists to get right.
fn wrap<Z: Zoned>(store: &mut Store<ZoneMap<Z>>, active: u32, other: u32) -> Result<u32, i32> {
    // A root zone that cannot hold the carry *and another carry after it* wraps
    // on every publish: the destination arrives holding `ROOT_CARRY` records,
    // and if that leaves it within `ROOT_CARRY` of full the next publish wraps
    // straight back. That is not a livelock — the generation counter still
    // advances — it is worse, a silent sixteenfold write amplification on the
    // root zones that nothing would report. So it is refused, and the refusal
    // says the geometry is wrong rather than letting the device deliver a
    // rollback depth of sixteen at a price nobody asked about. R04.
    //
    // *Where this check belongs is the mount*, which sees the geometry before
    // anything is published rather than at the first wrap, and `E2-B02`'s mount
    // is where the superblock's other geometry refusals already are. It is here
    // because that mount does not read `ROOT_CARRY` today; moving it is a
    // strictly better diff and it is owed.
    let destination = store.device_mut().device_mut().report(other)?;
    if destination.blocks < 2 * ROOT_CARRY as u64 {
        return Err(refusal::FULL);
    }

    let report = store.device_mut().device_mut().report(active)?;
    let mut carried: Vec<RootRecord> = Vec::new();
    let mut at = report.write_pointer;
    while at > report.start && carried.len() < ROOT_CARRY {
        at -= 1;
        if let Some(found) = record_at(store, at)? {
            carried.push(found);
        }
    }

    // The destination first. It holds the oldest generations this device still
    // has, every one of them older than the sixteen about to be written over
    // them, and it is empty on the first wrap.
    store.device_mut().device_mut().reset(other)?;

    let block_bytes = store.superblock().block_bytes as usize;
    let mut block = vec![0u8; block_bytes];
    // Ascending generation, so the carried run reads the way the zone it came
    // from did. A mount does not care — it takes the highest that verifies — and
    // a person reading a device image does.
    for record in carried.iter().rev() {
        block.fill(0);
        block[..RootRecord::BYTES].copy_from_slice(&record.to_bytes());
        store.device_mut().append_reserved(other, &block)?;
    }
    // And the carry is durable before anything is published on top of it.
    store.device_mut().flush()?;
    Ok(other)
}

/// The highest generation in one root zone, or zero for a zone holding none.
///
/// Zero is safe as *none* because [`RootRecord::from_bytes`] refuses generation
/// zero: a zeroed block is never a generation, which is the reason that field's
/// zero is reserved.
///
/// Unit: count of publishes since the superblock.
fn highest_in<Z: Zoned>(store: &mut Store<ZoneMap<Z>>, zone: u32) -> Result<u64, i32> {
    let report = store.device_mut().device_mut().report(zone)?;
    let mut highest = 0u64;
    for at in report.start..report.write_pointer {
        if let Some(found) = record_at(store, at)? {
            highest = highest.max(found.generation);
        }
    }
    Ok(highest)
}

/// The root record at one device-absolute block, or `None` for a block that
/// holds no record this build believes.
///
/// A block that does not decode is `None` and not an error: that is the rollback
/// the `check` field exists to make possible, and a torn record is the case it
/// was written for. A block the *device* refuses is also `None` — a sealed
/// zone's pointer sits at its end whatever room is left, so a scan to the
/// pointer can reach blocks nothing ever wrote.
///
/// # Errors
///
/// Nothing today: every failure is folded into `None` by the paragraph above.
/// The signature is fallible because a real device answers a read with a
/// refusal that is worth telling a caller about, and widening it later would be
/// a change to every caller.
pub(crate) fn record_at<Z: Zoned>(
    store: &mut Store<ZoneMap<Z>>,
    block: u64,
) -> Result<Option<RootRecord>, i32> {
    let block_bytes = store.superblock().block_bytes as usize;
    if block_bytes < RootRecord::BYTES {
        return Ok(None);
    }
    let mut buffer = vec![0u8; block_bytes];
    if store.device_mut().device_mut().read(block, &mut buffer).is_err() {
        return Ok(None);
    }
    let mut raw = [0u8; RootRecord::BYTES];
    raw.copy_from_slice(&buffer[..RootRecord::BYTES]);
    let mut checked = [0u8; RootRecord::CHECKED_BYTES];
    checked.copy_from_slice(&raw[..RootRecord::CHECKED_BYTES]);
    Ok(RootRecord::from_bytes(&raw, &sha256(&checked)).ok())
}

/// The highest generation on the device whose `check` verifies.
///
/// Both root zones are scanned from the device's own `ZONE_REPORT` write
/// pointers, which is the whole reason mount holds no mutable state on the
/// device: the format does not maintain a write pointer, so there is nothing to
/// reconcile after a cut.
///
/// Half of verify-before-accept, and this module's last paragraph says which
/// half and why. [`crate::mount::mount`] is the other half and answers the
/// highest generation that verifies *and* resolves.
///
/// # Errors
///
/// Whatever the device says about a report or a read. A record that does not
/// decode is skipped rather than refused — that is the rollback the `check`
/// exists to make possible, not an error the caller can do anything about.
pub fn latest_root<Z: Zoned>(store: &mut Store<ZoneMap<Z>>) -> Result<Option<RootRecord>, i32> {
    let (a, b) = (store.superblock().root_zone_a, store.superblock().root_zone_b);
    let mut best: Option<RootRecord> = None;

    for zone in [a, b] {
        let report = store.device_mut().device_mut().report(zone)?;
        // The pointer of a sealed zone is at its end whatever room is left, so
        // scanning to it can read blocks nothing wrote. The device refuses
        // those, which is the loop's own bound rather than a length this code
        // would otherwise have to keep.
        for at in report.start..report.write_pointer {
            let Some(found) = record_at(store, at)? else { continue };
            if best.is_none_or(|held| found.generation > held.generation) {
                best = Some(found);
            }
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::{Publisher, ROOT_CARRY, latest_root, record_at};
    use crate::device::{Pointer, Zoned, ZonedMemory};
    use crate::map::ZoneMap;
    use crate::roots::Roots;
    use alloc::vec;
    use alloc::vec::Vec;
    use f_abi::store::{RootRecord, kind, refusal};
    use f_blob::store::{Store, superblock_for_this_build};

    const BLOCK: usize = 512;

    /// Root zones of twice [`ROOT_CARRY`] plus eight blocks, which is the
    /// smallest geometry [`super::wrap`] accepts plus a margin: a destination
    /// arrives holding the carry, and if that left it within `ROOT_CARRY` of
    /// full the next publish would wrap straight back.
    const ZONE: u64 = 2 * ROOT_CARRY as u64 + 8;

    /// The publish before the first wrap. Unit: count of publishes.
    const BEFORE_THE_WRAP: u64 = ZONE - ROOT_CARRY as u64 + 1;

    fn store() -> Store<ZoneMap<ZonedMemory>> {
        let layout = superblock_for_this_build(BLOCK as u32, 12, ZONE * BLOCK as u64, 1, 2);
        let map = ZoneMap::new(ZonedMemory::new(BLOCK, ZONE, 12), 3).expect("data zones");
        Store::format(map, &layout).expect("a device this build can format")
    }

    /// Publish one generation whose root is a blob that exists.
    fn publish(
        store: &mut Store<ZoneMap<ZonedMemory>>,
        roots: &mut Roots,
        generation: u64,
        previous: [u8; 32],
    ) -> RootRecord {
        let mut publisher = Publisher::open(roots, store);
        let content = vec![generation as u8; 64];
        let root = store.put(kind::CHUNK, &content).expect("room for a chunk");
        publisher.appended(roots, store).expect("an open write set");
        let record = RootRecord {
            generation,
            root,
            frame: [0u8; 32],
            module: root,
            previous,
            check: [0u8; 32],
        };
        publisher.commit(store, roots, &record).expect("a publish that fits")
    }

    #[test]
    fn a_wrap_carries_the_last_records_and_leaves_the_retiring_zone_intact() {
        let mut store = store();
        let mut roots = Roots::new();
        let mut previous = [0u8; 32];
        let mut written: Vec<RootRecord> = Vec::new();
        for generation in 1..=BEFORE_THE_WRAP + 3 {
            let record = publish(&mut store, &mut roots, generation, previous);
            previous = record.root;
            written.push(record);
        }

        assert_eq!(
            latest_root(&mut store).expect("a device that reports"),
            written.last().copied(),
            "the highest generation is the last one published"
        );

        // Zone A is untouched by the wrap and zone B holds the carry plus what
        // was published after it. That is this module's whole safety argument:
        // the zone that is cleared is the older one.
        let a = store.device_mut().device_mut().report(1).expect("zone A");
        let b = store.device_mut().device_mut().report(2).expect("zone B");
        assert_eq!(
            a.write_pointer - a.start,
            BEFORE_THE_WRAP,
            "A holds every record it was written and the wrap reset it not at all"
        );
        assert_ne!(a.state, Pointer::Empty);
        assert!(b.write_pointer > b.start, "the wrap moved publishing into B");

        // The guaranteed rollback depth: at least ROOT_CARRY records survive in
        // the two zones together, which is what the carry is for.
        let mut reachable = 0u64;
        for zone in [1u32, 2] {
            let report = store.device_mut().device_mut().report(zone).expect("a root zone");
            for at in report.start..report.write_pointer {
                if record_at(&mut store, at).expect("a read").is_some() {
                    reachable += 1;
                }
            }
        }
        assert!(
            reachable >= ROOT_CARRY as u64,
            "{reachable} record(s) survive the wrap, which is fewer than ROOT_CARRY"
        );
    }

    #[test]
    fn a_root_zone_too_small_for_two_carries_is_refused_rather_than_thrashed() {
        // A root zone of exactly ROOT_CARRY blocks holds the carry and nothing
        // after it, so a wrap into one would wrap straight back on the next
        // publish and every publish after it would copy sixteen records.
        let zone = ROOT_CARRY as u64;
        let layout = superblock_for_this_build(BLOCK as u32, 12, zone * BLOCK as u64, 1, 2);
        let map = ZoneMap::new(ZonedMemory::new(BLOCK, zone, 12), 3).expect("data zones");
        let mut store = Store::format(map, &layout).expect("a device this build can format");
        let mut roots = Roots::new();

        // The first publish fits: an empty zone of ROOT_CARRY blocks has exactly
        // ROOT_CARRY of room and does not wrap.
        let first = publish(&mut store, &mut roots, 1, [0u8; 32]);
        assert_eq!(first.generation, 1);

        // The second finds one block gone and wraps, and the wrap refuses rather
        // than delivering a rollback depth of sixteen at a price nobody asked
        // about.
        let mut publisher = Publisher::open(&mut roots, &store);
        let root = store.put(kind::CHUNK, &[7u8; 64]).expect("room for a chunk");
        publisher.appended(&mut roots, &store).expect("an open write set");
        let record = RootRecord {
            generation: 2,
            root,
            frame: [0u8; 32],
            module: root,
            previous: first.root,
            check: [0u8; 32],
        };
        assert_eq!(publisher.commit(&mut store, &mut roots, &record), Err(refusal::FULL));
        // And the store that refused to publish did not lose the root it had.
        assert_eq!(latest_root(&mut store).expect("a device that reports"), Some(first));
    }
}
