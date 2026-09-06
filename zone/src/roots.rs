// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The root set: `R = P ∪ T`, and the one ordering that guards it.
//!
//! # The sentence this file exists to make unwritable
//!
//! *The sweep is careful about in-flight publishes.* Care is not assertable; a
//! root set is. RFC 0059's first decision is that the root set is not the
//! pinned roots — it is the pinned roots **and every open write set** — and the
//! whole of that decision lives here, in a type where an open write set is a
//! member of a set the mark ranges over rather than a condition the sweep
//! remembers to check.
//!
//! # Add-then-drop, and why it is an ordering and not a lifetime
//!
//! [`Roots::adopt`] pins the new root and *then* releases the transient entry,
//! in that order, in one function that has no other. Dropping first — releasing
//! at the root record's append rather than at the completion of RFC 0060's
//! second `FLUSH`, or releasing before the pinned set has adopted the root —
//! opens a window in which the blobs are named by nothing, and the window is
//! exactly as long as a device write. A one-line reordering reintroduces that
//! bug, so the two steps are not two calls a caller sequences correctly.
//!
//! # `T` holds positions and not hashes
//!
//! An entry in the transient set is the list of logical blocks a write set has
//! appended and that no durable root yet names. It cannot be a hash: the hash
//! that will name them is the thing that does not exist yet, which is the whole
//! reason the clause is needed.
//!
//! # `PIN` is not available here
//!
//! A pin is durable and durable means an entry in `f-index`'s pin namespace,
//! which is `E2-B03`. Until then `P` holds the mounted root plus whatever this
//! in-memory set was told, and `zone/tests/cycle.rs` is the only thing that
//! tells it. The spec writes that ordering down explicitly so that nobody reads
//! `PIN` as available in `E2-B02`, and this paragraph is where a reader of the
//! code meets it.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use f_abi::store::refusal;

/// One open write set, named so that the entry it holds can be found again.
///
/// A newtype rather than a bare `u64` because the one thing a caller must not
/// do is release somebody else's entry, and two `u64` arguments in the same
/// call are two arguments that can be swapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WriteSet(
    /// Unit: none — an identity, not a quantity. Counted from one so that zero
    /// names no write set.
    pub u64,
);

/// `R = P ∪ T`.
#[derive(Debug, Default)]
pub struct Roots {
    /// `P` — the pinned set. Durable in the design; in memory until `E2-B03`.
    pinned: BTreeSet<[u8; 32]>,
    /// `T` — one entry per open write set, each the logical blocks that write
    /// set has appended.
    transient: BTreeMap<WriteSet, Vec<u64>>,
    /// Unit: none — the next identity to hand out.
    next: u64,
    /// Unit: count of full re-marks owed. RFC 0059: a removal clears nothing
    /// and schedules a full re-mark, because deciding which blobs *only* that
    /// root reached is the question reference counting answers badly.
    remarks_owed: u64,
}

impl Roots {
    /// An empty root set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `P`, in hash order.
    pub fn pinned(&self) -> impl Iterator<Item = &[u8; 32]> {
        self.pinned.iter()
    }

    /// How many roots are pinned.
    ///
    /// Unit: count of roots.
    #[must_use]
    pub fn roots_pinned(&self) -> usize {
        self.pinned.len()
    }

    /// How many write sets are open.
    ///
    /// Unit: count of write sets.
    #[must_use]
    pub fn roots_transient(&self) -> usize {
        self.transient.len()
    }

    /// Every logical block held by an open write set, in ascending order.
    ///
    /// Unit: block index, zero-based, logical.
    #[must_use]
    pub fn transient_blocks(&self) -> BTreeSet<u64> {
        self.transient.values().flatten().copied().collect()
    }

    /// Whether a full re-mark is owed.
    ///
    /// A removal — an unpin, or in principle any shrinking of `R` — makes every
    /// `live_bytes(z)` an upper bound rather than a count, and the only honest
    /// repair is to clear the bitmap and walk every root again. Answering
    /// *whether* it is owed rather than doing it here keeps the walk in the one
    /// place that has a store to walk.
    #[must_use]
    pub const fn remark_owed(&self) -> bool {
        self.remarks_owed > 0
    }

    /// Record that a full re-mark has been done.
    ///
    /// Unit: count of full re-marks performed by this call — always one, and
    /// the counter is cleared rather than decremented because one walk answers
    /// every removal that preceded it.
    pub const fn remarked(&mut self) {
        self.remarks_owed = 0;
    }

    /// Pin a root. `PIN` on the wire, when there is a wire.
    ///
    /// Adding a root cannot invalidate a mark — RFC 0059's cycle marks roots
    /// added during it before any sweep in that cycle may condemn — so this
    /// schedules nothing.
    pub fn pin(&mut self, root: [u8; 32]) -> bool {
        self.pinned.insert(root)
    }

    /// Unpin a root, and schedule the full re-mark it costs. `UNPIN` on the
    /// wire, when there is a wire.
    pub fn unpin(&mut self, root: &[u8; 32]) -> bool {
        let removed = self.pinned.remove(root);
        if removed {
            self.remarks_owed += 1;
        }
        removed
    }

    /// Open a write set and take the transient entry that protects it.
    ///
    /// Taken at the write set's *first* append and not at its last, because the
    /// blobs it protects are unreachable from the first one onwards.
    pub fn open(&mut self) -> WriteSet {
        self.next += 1;
        let set = WriteSet(self.next);
        self.transient.insert(set, Vec::new());
        set
    }

    /// Record a block this write set has appended.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write set that is not open, which is a caller
    /// appending after its publish completed.
    pub fn appended(&mut self, set: WriteSet, block: u64) -> Result<(), i32> {
        self.transient.get_mut(&set).ok_or(refusal::ADDRESS)?.push(block);
        Ok(())
    }

    /// **The guarded ordering.** Pin the new root, then release the write set —
    /// never the other way round, and never as two calls.
    ///
    /// Called only when the completion of RFC 0060's second `FLUSH` has made
    /// the root record durable. Calling it earlier is the same defect as
    /// reordering its two lines, and the caller is
    /// [`crate::publish::Publisher::commit`], which is the only place in this
    /// crate that has seen that completion.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write set that is not open.
    pub fn adopt(&mut self, set: WriteSet, root: [u8; 32]) -> Result<(), i32> {
        if !self.transient.contains_key(&set) {
            return Err(refusal::ADDRESS);
        }
        // Add.
        self.pinned.insert(root);
        // Then drop. Between these two lines the blobs are named twice; before
        // the first they are named once; after the second they are named once.
        // There is no ordering of these two statements in which they are named
        // zero times, and that is the whole property.
        self.transient.remove(&set);
        Ok(())
    }

    /// Abandon a write set without pinning anything — a publish that failed
    /// before its root record was durable.
    ///
    /// This *is* a removal and schedules the full re-mark, for the same reason
    /// an unpin does: the blocks it protected may now be reachable from
    /// nothing, and no incremental arithmetic can say which.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a write set that is not open.
    pub fn abandon(&mut self, set: WriteSet) -> Result<(), i32> {
        self.transient.remove(&set).ok_or(refusal::ADDRESS)?;
        self.remarks_owed += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Roots;
    use f_abi::store::refusal;

    #[test]
    fn a_write_set_is_in_the_root_set_from_its_first_append() {
        let mut roots = Roots::new();
        let set = roots.open();
        roots.appended(set, 7).expect("an open write set");
        roots.appended(set, 8).expect("and again");
        assert_eq!(roots.roots_transient(), 1);
        assert_eq!(roots.transient_blocks(), [7, 8].into_iter().collect());
        assert_eq!(roots.roots_pinned(), 0, "nothing durable names them yet");
    }

    /// The one property the ordering exists for, asserted as a count at every
    /// point rather than as a comment about a window.
    #[test]
    fn the_blocks_are_named_by_something_before_during_and_after_the_handover() {
        let mut roots = Roots::new();
        let set = roots.open();
        roots.appended(set, 11).expect("an open write set");
        let named_before = roots.transient_blocks().len() + roots.roots_pinned();
        assert_eq!(named_before, 1);

        roots.adopt(set, [3u8; 32]).expect("the second FLUSH completed");
        assert_eq!(roots.roots_transient(), 0);
        assert_eq!(roots.roots_pinned(), 1);
        assert!(!roots.remark_owed(), "adding a root invalidates no mark");

        assert_eq!(roots.adopt(set, [3u8; 32]), Err(refusal::ADDRESS), "and only once");
    }

    #[test]
    fn a_removal_schedules_the_full_re_mark_and_an_addition_does_not() {
        let mut roots = Roots::new();
        roots.pin([1u8; 32]);
        assert!(!roots.remark_owed());
        assert!(roots.unpin(&[1u8; 32]));
        assert!(roots.remark_owed(), "a removal cannot decrement live_bytes correctly");
        roots.remarked();
        assert!(!roots.remark_owed());

        assert!(!roots.unpin(&[1u8; 32]), "unpinning what is not pinned changes nothing");
        assert!(!roots.remark_owed(), "and schedules nothing");
    }

    #[test]
    fn an_abandoned_write_set_schedules_the_re_mark_a_failed_publish_costs() {
        let mut roots = Roots::new();
        let set = roots.open();
        roots.appended(set, 4).expect("an open write set");
        roots.abandon(set).expect("a publish that never became durable");
        assert_eq!(roots.roots_transient(), 0);
        assert!(roots.remark_owed());
        assert_eq!(roots.abandon(set), Err(refusal::ADDRESS));
    }
}
