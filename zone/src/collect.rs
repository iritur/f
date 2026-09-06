// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The collector: mark from `R`, sweep by live fraction, batch class, and a
//! reset that refuses rather than violates.
//!
//! # The shape, in one paragraph
//!
//! A cycle is a sequence of bounded steps. Each step is at most one collector
//! device operation plus bounded computation, each of those operations is
//! submitted at `class::BATCH` with `NO_DEADLINE`, and client writes and
//! publishes interleave freely between steps. The safety argument is short
//! because edges do not move: a blob's content is fixed at the moment it is
//! written, so there is no pointer write to race, and the only thing that
//! mutates is `R`. Roots added during a cycle are marked *within* the cycle —
//! [`Collector::step`] catches up at the top, before anything can condemn —
//! and roots removed during a cycle schedule a full re-mark that is not applied
//! until the next one. So `marked_roots ⊇ R` is an invariant of the cycle
//! rather than a race the sweep has to think about.
//!
//! # The refusal that is the point of this file
//!
//! Before it submits a reset the collector evaluates
//! [`crate::invariants::all_three`], and **refuses the cycle if any of them is
//! false**. Not `debug_assert`, not a log line, and not a comment: a collector
//! that would break its own invariant stops, and the store is left with one
//! extra sealed zone rather than with a root naming a zone that no longer holds
//! anything. The cost of the check is a range query over a bitmap and a walk of
//! the pinned set; the cost of not having it is `E2-P01`'s forbidden third
//! state, manufactured by the collector instead of by a power cut.
//!
//! # What starts a cycle, and what does not
//!
//! `zones_free < COLLECT_FREE_ZONES_LOW` = 2, or an explicit `COLLECT`. The 2
//! is derived: the client needs an open zone to keep writing into and the sweep
//! needs a destination zone that is not the one it is about to reset, so a
//! collector that started at one free zone can deadlock against its own writer.
//! There is no timer, and there could not be — a collector that runs "every so
//! often" reads a clock, and RFC 0004 says where clocks live.

use alloc::vec::Vec;

use f_abi::store::refusal;
use f_blob::store::Store;

use crate::device::Zoned;
use crate::invariants;
use crate::map::{State, ZoneMap};
use crate::mark::Mark;
use crate::queue::Queue;
use crate::roots::Roots;

/// The live fraction below which a sealed zone is worth sweeping, as the two
/// integers the comparison is made with.
///
/// `f(z) = live_bytes(z) / zone_bytes`, evaluated as `live_bytes(z) · 4 <
/// zone_bytes` so that 0.25 is exact and the collector contains no floating
/// point. Published as two constants rather than one ratio because a reader
/// checking the derivation against RFC 0059 is checking an inequality, and an
/// inequality written with a divide is an inequality with a rounding mode
/// nobody stated.
///
/// **0.25 is the largest threshold whose copy-forward term leaves claim 0016's
/// 1.5 with margin.** Reclaimed bytes per zone swept are `(1 − f) · zone_bytes`
/// and copied bytes are `f · zone_bytes`, so copy-forward costs `f/(1 − f)`
/// bytes written per new byte stored: 0.33 at 0.25, 1.00 at 0.5. The constant
/// and the claim move together or not at all.
/// Unit: none — the denominator of a fraction whose numerator is 1.
pub const SWEEP_LIVE_FRACTION_DENOMINATOR: u64 = 4;

/// How few free zones start a cycle.
///
/// Unit: count of zones. Derived, not chosen: one for the client to keep
/// writing into and one for the sweep to evacuate into, and a collector that
/// started at one can deadlock against its own writer.
pub const COLLECT_FREE_ZONES_LOW: u32 = 2;

/// Where a cycle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No cycle is running.
    Idle,
    /// Walking `R`.
    Marking,
    /// Choosing what to condemn.
    Selecting,
    /// Evacuating and resetting one zone.
    Sweeping,
}

/// One zone being swept.
#[derive(Debug)]
struct Sweep {
    /// Unit: zone index, zero-based.
    zone: u32,
    /// The live logical blocks still to copy out, in ascending physical order —
    /// the order a copy reads them in.
    /// Unit: block index, zero-based, logical.
    remaining: Vec<u64>,
    /// Whether the copies have been made durable and repointed.
    committed: bool,
}

/// What one step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// One record walked into `B`.
    Marked,
    /// The mark finished and `live_bytes(z)` was published.
    Published,
    /// Zones were chosen, or none were.
    Selected,
    /// One block copied forward.
    Copied,
    /// The copies out of a zone were made durable and repointed.
    Committed,
    /// A zone was reset.
    Reset,
    /// The reset is held: a reader still believes the zone is live. RFC 0059's
    /// third conjunct, and the count is monotone non-increasing from
    /// condemnation, so this ends.
    Waiting,
    /// Nothing left to do. `COLLECT`'s completion.
    Done,
}

/// What the collector publishes, under RFC 0013, in `user/objects`' own tree.
///
/// A struct rather than ten accessors because RFC 0059 names them as one
/// subtree and a reader comparing this list against that one should be
/// comparing two lists. **There are no floats in it**: `live_bytes` and the
/// superblock's `zone_bytes` are both published and the reader divides, so the
/// number a claim reports and the number the collector compares are the same
/// number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// Unit: count of cycles started.
    pub cycle: u64,
    /// Unit: count of full re-marks.
    pub full_remarks: u64,
    /// Unit: count of roots.
    pub roots_pinned: u64,
    /// Unit: count of open write sets.
    pub roots_transient: u64,
    /// Unit: count of zones.
    pub zones_free: u32,
    /// Unit: count of zones.
    pub zones_sealed: u32,
    /// Unit: count of zones condemned since mount.
    pub zones_condemned: u64,
    /// Unit: bytes.
    pub live_bytes_total: u64,
    /// Unit: bytes.
    pub copied_bytes: u64,
    /// I3's own witness: the maximum collector operations ever observed ahead
    /// of an entry more urgent than batch.
    /// Unit: count of device operations.
    pub ops_ahead_of_urgent_read: u32,
    /// Hashes in the closure of `R` the store holds nothing under. Zero on a
    /// store nothing has damaged.
    /// Unit: count of hashes.
    pub unresolved: u64,
}

/// The collector.
pub struct Collector {
    mark: Mark,
    queue: Queue,
    phase: Phase,
    /// Zones chosen this cycle, most worth sweeping first.
    /// Unit: zone index, zero-based.
    chosen: Vec<u32>,
    sweeping: Option<Sweep>,
    /// Unit: count of cycles started.
    cycle: u64,
    /// Unit: count of full re-marks.
    full_remarks: u64,
    /// Unit: bytes.
    copied_bytes: u64,
    /// Unit: count of zones condemned since mount.
    zones_condemned: u64,
}

impl Collector {
    /// A collector over a device of `blocks` blocks — the bitmap's size, fixed
    /// at mount from the superblock as `zones · (zone_bytes / block_bytes)`
    /// bits, and the collector's whole memory budget.
    #[must_use]
    pub fn new(blocks: u64) -> Self {
        Self {
            mark: Mark::new(blocks),
            queue: Queue::new(),
            phase: Phase::Idle,
            chosen: Vec::new(),
            sweeping: None,
            cycle: 0,
            full_remarks: 0,
            copied_bytes: 0,
            zones_condemned: 0,
        }
    }

    /// The driver's queue, so that a client can submit an urgent read into the
    /// same queue the collector's operations go through — which is the only way
    /// I3 is about anything.
    pub const fn queue_mut(&mut self) -> &mut Queue {
        &mut self.queue
    }

    /// The driver's queue.
    #[must_use]
    pub const fn queue(&self) -> &Queue {
        &self.queue
    }

    /// The mark.
    #[must_use]
    pub const fn mark(&self) -> &Mark {
        &self.mark
    }

    /// Whether a cycle is running.
    #[must_use]
    pub fn running(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Whether the store is under enough pressure to start a cycle by itself.
    #[must_use]
    pub fn should_collect<Z: Zoned>(map: &ZoneMap<Z>) -> bool {
        map.zones_free() < COLLECT_FREE_ZONES_LOW
    }

    /// What this collector publishes.
    #[must_use]
    pub fn published<Z: Zoned>(&self, map: &ZoneMap<Z>, roots: &Roots) -> Published {
        Published {
            cycle: self.cycle,
            full_remarks: self.full_remarks,
            roots_pinned: roots.roots_pinned() as u64,
            roots_transient: roots.roots_transient() as u64,
            zones_free: map.zones_free(),
            zones_sealed: map.zones().filter(|zone| zone.state == State::Sealed).count() as u32,
            zones_condemned: self.zones_condemned,
            live_bytes_total: self.mark.live_bytes_total(),
            copied_bytes: self.copied_bytes,
            ops_ahead_of_urgent_read: self.queue.ops_ahead_of_urgent_read(),
            unresolved: self.mark.unresolved(),
        }
    }

    /// Start a cycle. `COLLECT` on the wire, when there is a wire.
    ///
    /// Every cycle in this build begins with a full re-mark — `mark.rs`'s
    /// second paragraph says why that is a build limitation and not a decision,
    /// and which direction it errs in.
    pub fn begin(&mut self, roots: &mut Roots) {
        self.cycle += 1;
        self.full_remarks += 1;
        self.mark.restart();
        self.chosen.clear();
        self.sweeping = None;
        for root in roots.pinned() {
            self.mark.walk(*root);
        }
        roots.remarked();
        self.phase = Phase::Marking;
    }

    /// One step of the cycle.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] when a reset would break one of the three
    /// invariants — the refusal this file exists for; otherwise whatever the
    /// store, the map or the queue says.
    pub fn step<Z: Zoned>(
        &mut self,
        store: &mut Store<ZoneMap<Z>>,
        roots: &Roots,
    ) -> Result<Progress, i32> {
        if self.phase == Phase::Idle {
            return Ok(Progress::Done);
        }
        // Roots added since the cycle started are marked within it, before
        // anything may condemn. One check at the top of the step covers both
        // halves of `R`: a new pin goes back onto the work list, and a new
        // transient entry is marked where it lies.
        self.catch_up(store, roots);

        match self.phase {
            Phase::Idle => Ok(Progress::Done),
            Phase::Marking => self.marking(store, roots),
            Phase::Selecting => self.selecting(store),
            Phase::Sweeping => self.sweeping(store, roots),
        }
    }

    /// Run until there is nothing left to do — which is what `COLLECT` means:
    /// *collect now, and tell me when there is nothing left to do*.
    ///
    /// Answers how many steps it took.
    ///
    /// # Errors
    ///
    /// Whatever [`Collector::step`] says. A `Waiting` step with no reader ever
    /// releasing would loop, which is why the count is answered: a caller that
    /// wants a bound has one, and `reads_outstanding` is monotone
    /// non-increasing after condemnation, so a released reader ends it.
    pub fn collect<Z: Zoned>(
        &mut self,
        store: &mut Store<ZoneMap<Z>>,
        roots: &mut Roots,
    ) -> Result<u64, i32> {
        self.begin(roots);
        let mut steps = 0u64;
        loop {
            match self.step(store, roots)? {
                Progress::Done => {
                    self.phase = Phase::Idle;
                    // The last collector operation is still inside the device —
                    // `Queue::collector_operation` leaves it there so that I3
                    // measures something — and a cycle that ended leaves
                    // nothing there.
                    self.queue.settle()?;
                    return Ok(steps);
                }
                Progress::Waiting => {
                    self.queue.settle()?;
                    return Ok(steps);
                }
                _ => steps += 1,
            }
        }
    }

    /// Mark anything in `R` the current mark has not reached.
    fn catch_up<Z: Zoned>(&mut self, store: &mut Store<ZoneMap<Z>>, roots: &Roots) {
        let mut added = false;
        for root in roots.pinned() {
            if !self.mark.marked_roots().contains(root) {
                self.mark.walk(*root);
                added = true;
            }
        }
        // A transient entry has no hash to walk: its blocks are marked where
        // they lie, which is the same protection one level down and is why one
        // predicate covers the pinned case and the open case.
        self.mark.occupy_transient(store, &roots.transient_blocks());
        if added {
            self.phase = Phase::Marking;
        }
    }

    fn marking<Z: Zoned>(
        &mut self,
        store: &mut Store<ZoneMap<Z>>,
        roots: &Roots,
    ) -> Result<Progress, i32> {
        if self.mark.walking() {
            self.queue.collector_operation()?;
            self.mark.step(store)?;
            return Ok(Progress::Marked);
        }
        self.mark.occupy_transient(store, &roots.transient_blocks());
        self.mark.publish(store.device_mut())?;
        self.phase = Phase::Selecting;
        Ok(Progress::Published)
    }

    fn selecting<Z: Zoned>(&mut self, store: &mut Store<ZoneMap<Z>>) -> Result<Progress, i32> {
        let zone_bytes =
            store.device().device().zone_blocks() * u64::from(store.superblock().block_bytes);
        let mut candidates: Vec<(u64, u32)> = store
            .device()
            .zones()
            .filter(|zone| zone.state == State::Sealed)
            .filter(|zone| zone.live_bytes * SWEEP_LIVE_FRACTION_DENOMINATOR < zone_bytes)
            .map(|zone| (zone.live_bytes, zone.zone))
            .collect();
        // Smallest `live_bytes` first, ties broken by ascending zone index. The
        // tie-break is stated because RFC 0004 says an order nobody stated is an
        // order the map chose — and two runs of one workload must lay a device
        // out identically or `E2-P06` has nothing to compare.
        candidates.sort_unstable();
        self.chosen = candidates.into_iter().map(|(_, zone)| zone).collect();
        if self.chosen.is_empty() {
            self.phase = Phase::Idle;
            return Ok(Progress::Done);
        }
        self.phase = Phase::Sweeping;
        Ok(Progress::Selected)
    }

    fn sweeping<Z: Zoned>(
        &mut self,
        store: &mut Store<ZoneMap<Z>>,
        roots: &Roots,
    ) -> Result<Progress, i32> {
        if self.sweeping.is_none() {
            let Some(zone) = self.chosen.pop() else {
                // Re-select rather than stop: a sweep changes every
                // `live_bytes` it touched, so the next zone worth taking is a
                // question about the store as it now is.
                self.phase = Phase::Selecting;
                return Ok(Progress::Selected);
            };
            store.device_mut().condemn(zone)?;
            let remaining = Self::live_in(store, &self.mark, zone);
            self.sweeping = Some(Sweep { zone, remaining, committed: false });
            return Ok(Progress::Selected);
        }

        let sweep = self.sweeping.as_mut().ok_or(refusal::MALFORMED)?;
        let zone = sweep.zone;

        if let Some(logical) = sweep.remaining.pop() {
            self.queue.collector_operation()?;
            store.device_mut().copy_forward(logical)?;
            self.copied_bytes += u64::from(store.superblock().block_bytes);
            return Ok(Progress::Copied);
        }

        if !sweep.committed {
            sweep.committed = true;
            self.queue.collector_operation()?;
            store.device_mut().commit_copies()?;
            // A copy moved blocks between zones, so `B` and every
            // `live_bytes(z)` are about somewhere else now. Recomputed rather
            // than adjusted: an adjustment is a second piece of arithmetic that
            // has to agree with the first.
            self.mark.rebuild(store, roots);
            self.mark.publish(store.device_mut())?;
            return Ok(Progress::Committed);
        }

        // The reset guard's third conjunct. A reader that resolved into this
        // zone before the relocation believes it is live; the count cannot rise
        // after condemnation, so waiting ends.
        if !invariants::no_reader_believes_the_zone(store.device(), zone) {
            return Ok(Progress::Waiting);
        }

        // And the refusal this file exists for. Evaluated at the instant
        // `ZONE_RESET` would be submitted, over the root set at that instant.
        if !invariants::all_three(store.device(), &self.mark, roots, &self.queue, zone) {
            return Err(refusal::MALFORMED);
        }

        self.queue.collector_operation()?;
        store.device_mut().recycle(zone)?;
        self.zones_condemned += 1;
        self.sweeping = None;
        Ok(Progress::Reset)
    }

    /// The live logical blocks a zone holds, in the order a copy reads them.
    fn live_in<Z: Zoned>(store: &Store<ZoneMap<Z>>, mark: &Mark, zone: u32) -> Vec<u64> {
        let map = store.device();
        let mut live: Vec<u64> = map
            .holds(zone)
            .into_iter()
            .filter(|logical| map.physical(*logical).is_some_and(|at| mark.bits().get(at)))
            .collect();
        // Popped from the back, so reverse to copy in ascending physical order.
        live.reverse();
        live
    }
}

#[cfg(test)]
mod tests {
    use super::{COLLECT_FREE_ZONES_LOW, Collector, SWEEP_LIVE_FRACTION_DENOMINATOR};
    use crate::device::ZonedMemory;
    use crate::map::ZoneMap;

    #[test]
    fn the_threshold_is_an_integer_comparison_and_the_ratio_is_a_quarter() {
        // 0.25 exactly, with no floating point anywhere near the collector.
        assert_eq!(SWEEP_LIVE_FRACTION_DENOMINATOR, 4);
        let zone_bytes = 1u64 << 20;
        let quarter = zone_bytes / 4;
        assert!(!((quarter) * SWEEP_LIVE_FRACTION_DENOMINATOR < zone_bytes), "0.25 is not below");
        assert!((quarter - 1) * SWEEP_LIVE_FRACTION_DENOMINATOR < zone_bytes, "just under is");
    }

    #[test]
    fn pressure_is_counted_in_free_zones_and_not_in_elapsed_anything() {
        let map = ZoneMap::new(ZonedMemory::new(512, 4, 6), 1).expect("a device with data zones");
        assert_eq!(map.zones_free(), 5);
        assert!(!Collector::should_collect(&map));
        assert_eq!(COLLECT_FREE_ZONES_LOW, 2);
    }
}
