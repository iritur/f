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
//! # What is not here, and what that costs
//!
//! **`ROOT_CARRY` and the root-zone wrap.** The spec's design is two root
//! zones, records appending to the active one, and — when the active zone's
//! remaining capacity falls to `ROOT_CARRY` = 16 records — the last 16 records
//! carried to the other zone, `FLUSH`, and only then the full zone reset, so
//! that there is never a moment with no durable root. [`Publisher::commit`]
//! here appends to whichever root zone has room and **refuses with
//! [`refusal::FULL`] when neither does**. That is a store that stops publishing
//! rather than one that loses a root, which is the right way round to be
//! incomplete, and it is incomplete: `E2-B02`'s exit is a fill-and-collect
//! cycle and the wrap is not on the path to it. *Reversal:* the first workload
//! that publishes more than two root zones' worth of generations, which is
//! `E2-P01`'s cut-across-a-wrap case and needs the carry before it can run.
//!
//! **The other half of verify-before-accept.** [`latest_root`] verifies a root
//! record's `check` and answers the highest generation that survives it. It
//! does *not* check that every child of the generation node resolves, and that
//! is not an oversight to be fixed here: after a restart the store's
//! hash-to-block map is gone — it is a `BTreeMap` in memory, rebuilt by nothing
//! — so there is nothing to resolve *against* until `E2-B03` puts the index on
//! the device. A mount that claimed to have resolved a tree it could not look
//! up would be worse than one that says which half it did.

use alloc::vec;

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
        store.barrier()?;

        let mut written = *record;
        written.check = sha256(&record.checked());
        let block_bytes = store.superblock().block_bytes as usize;
        let mut block = vec![0u8; block_bytes];
        block[..RootRecord::BYTES].copy_from_slice(&written.to_bytes());

        // Step 3. Appended and not written: the device assigns the position, so
        // there is one write pointer for a mount to read rather than two to
        // reconcile.
        let zone = root_zone_with_room(store)?;
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

/// The active root zone: A while it has room, then B.
///
/// # Errors
///
/// [`refusal::FULL`] when neither has room.
fn root_zone_with_room<Z: Zoned>(store: &mut Store<ZoneMap<Z>>) -> Result<u32, i32> {
    let (a, b) = (store.superblock().root_zone_a, store.superblock().root_zone_b);
    for zone in [a, b] {
        let report = store.device_mut().device_mut().report(zone)?;
        if report.state != Pointer::Full {
            return Ok(zone);
        }
    }
    Err(refusal::FULL)
}

/// The highest generation on the device whose `check` verifies.
///
/// Both root zones are scanned from the device's own `ZONE_REPORT` write
/// pointers, which is the whole reason mount holds no mutable state on the
/// device: the format does not maintain a write pointer, so there is nothing to
/// reconcile after a cut.
///
/// Half of verify-before-accept, and this module's last paragraph says which
/// half and why the other one waits on `E2-B03`.
///
/// # Errors
///
/// Whatever the device says about a report or a read. A record that does not
/// decode is skipped rather than refused — that is the rollback the `check`
/// exists to make possible, not an error the caller can do anything about.
pub fn latest_root<Z: Zoned>(store: &mut Store<ZoneMap<Z>>) -> Result<Option<RootRecord>, i32> {
    let (a, b) = (store.superblock().root_zone_a, store.superblock().root_zone_b);
    let block_bytes = store.superblock().block_bytes as usize;
    let mut best: Option<RootRecord> = None;
    let mut block = vec![0u8; block_bytes];

    for zone in [a, b] {
        let report = store.device_mut().device_mut().report(zone)?;
        // The pointer of a sealed zone is at its end whatever room is left, so
        // scanning to it can read blocks nothing wrote. The device refuses
        // those, which is the loop's own bound rather than a length this code
        // would otherwise have to keep.
        for at in report.start..report.write_pointer {
            if store.device_mut().device_mut().read(at, &mut block).is_err() {
                continue;
            }
            if block.len() < RootRecord::BYTES {
                continue;
            }
            let mut raw = [0u8; RootRecord::BYTES];
            raw.copy_from_slice(&block[..RootRecord::BYTES]);
            let mut checked = [0u8; RootRecord::CHECKED_BYTES];
            checked.copy_from_slice(&raw[..RootRecord::CHECKED_BYTES]);
            let Ok(found) = RootRecord::from_bytes(&raw, &sha256(&checked)) else { continue };
            if best.is_none_or(|held| found.generation > held.generation) {
                best = Some(found);
            }
        }
    }
    Ok(best)
}
