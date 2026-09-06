// SPDX-License-Identifier: Apache-2.0 OR MIT
//! RFC 0059's three invariants, as predicates over named state.
//!
//! # Why they are functions in their own file and not assertions in the sweep
//!
//! Because `E2-P03`'s exit is *invariants*, and three sentences in the
//! indicative mood are behaviour: a run that does not happen to lose data
//! passes all three. What makes an invariant assertable is that somebody other
//! than the code it constrains can evaluate it, at an instant of their
//! choosing, over state they can name. So each of the three below is a pure
//! function of `R`, `B`, each zone's state and live byte count, and the
//! driver's queue — the state RFC 0059 lists, in that order — and the sweep
//! calls them rather than containing them.
//!
//! An invariant checked inside the function it constrains is an invariant that
//! can only be asserted by calling the thing it is about. That is how an
//! invariant becomes a description of the code, which is precisely what RFC
//! 0059 says it was written before `E2-B02` to prevent.
//!
//! # What each answers, and what it does not
//!
//! Each answers a `bool` and names nothing about *why*. A predicate that
//! returned a diagnosis would be a predicate whose shape depends on the
//! failures somebody imagined; a test that wants the diagnosis has the same
//! state the predicate does and can take it apart itself. What each *does*
//! carry is the RFC's own wording, so that a reader comparing the two is
//! comparing a formula with a formula.

use alloc::collections::BTreeSet;

use crate::device::Zoned;
use crate::map::{State, ZoneMap};
use crate::mark::Mark;
use crate::queue::{COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ, Queue};
use crate::roots::Roots;

/// **I1 — nothing reachable is swept.** At the instant `ZONE_RESET(z)` is
/// submitted,
///
/// ```text
/// blobs(z) ∩ reach(R) = ∅
/// ```
///
/// checked as the mechanism that implies it: `marked_roots ⊇ R` and every bit
/// of `B` in `z`'s range is clear.
///
/// `R` is the root set *at this instant*, transient entries included — which is
/// the whole of RFC 0059's first decision, and the reason `roots` is an
/// argument rather than something the mark remembers from when the cycle
/// started. A transient entry has no hash, so its half of `marked_roots ⊇ R` is
/// *every block it holds is marked*, which is the same statement one level down.
///
/// `B` is a bitmap of *occupancy* and not of starts — `mark.rs` argues that
/// departure, and it is what makes the second conjunct sufficient rather than
/// merely necessary for a record that spans blocks.
#[must_use]
pub fn nothing_reachable_is_swept<Z: Zoned>(
    map: &ZoneMap<Z>,
    mark: &Mark,
    roots: &Roots,
    zone: u32,
) -> bool {
    // marked_roots ⊇ P.
    let marked: &BTreeSet<[u8; 32]> = mark.marked_roots();
    if !roots.pinned().all(|root| marked.contains(root)) {
        return false;
    }
    // marked_roots ⊇ T, one level down: every block an open write set holds is
    // marked. A transient entry is a list of positions and not a hash, because
    // the hash that will name them is the thing that does not exist yet.
    for logical in roots.transient_blocks() {
        match map.physical(logical) {
            Some(physical) if mark.bits().get(physical) => {}
            _ => return false,
        }
    }
    // And every bit of B in z's range is clear.
    let blocks = map.device().zone_blocks();
    let from = u64::from(zone) * blocks;
    !mark.bits().any(from, from + blocks)
}

/// **I2 — a reset zone holds no live blob.** For every zone and at every
/// instant,
///
/// ```text
/// state(z) = free  →  ∀h : location(h) = (z, ·) is undefined
/// ```
///
/// Stronger than the sentence it comes from, deliberately: it forbids not only
/// live data in a reset zone but any *resolution* that lands in one, which is
/// the exact shape of the forbidden third state `E2-P01` sweeps for — a root
/// that names a zone the collector has already reset.
///
/// `location(h)` here is the store's hash-to-logical map composed with this
/// map's logical-to-physical one; the second half is the one a copy repoints,
/// so this checks the second half, which is where a stale resolution would
/// live. A hash the store still names whose logical block has no physical is
/// *undefined*, which is the predicate holding rather than failing.
///
/// The third conjunct of the reset guard is not in here, because it is not part
/// of I2's statement: `reads_outstanding(z) = 0` is a precondition of
/// submitting the reset, and [`no_reader_believes_the_zone`] is where it lives.
#[must_use]
pub fn a_reset_zone_holds_no_live_blob<Z: Zoned>(map: &ZoneMap<Z>) -> bool {
    map.zones().filter(|zone| zone.state == State::Free).all(|zone| map.holds(zone.zone).is_empty())
}

/// The reset guard's third conjunct: `reads_outstanding(z) = 0`.
///
/// A reader that resolved `h` to `z` before the relocation is a reader that
/// believes `z` is live, and I2 is about beliefs as well as bytes. The count is
/// bounded because condemnation stops `location` from handing `z` out, so after
/// condemnation it is monotone non-increasing — which is why a sweep that waits
/// on this terminates rather than spinning.
#[must_use]
pub fn no_reader_believes_the_zone<Z: Zoned>(map: &ZoneMap<Z>, zone: u32) -> bool {
    map.zone(zone).is_some_and(|held| held.reads_outstanding == 0)
}

/// **I3 — collection never starves a deadline-class read.** For every entry `e`
/// submitted to the blk ring whose inherited class is more urgent than
/// `class::BATCH`,
///
/// ```text
/// |{ c : c submitted by the collector, handed to the device
///        after e was submitted and before e was }|
///   ≤ collector_operations_ahead_of_hard_read = 1
/// ```
///
/// Stated for any entry above batch and not only for hard, because
/// `user/virtio-blk` declares the soft class and RFC 0025's bound 1 therefore
/// serves every hard-class read as soft with `SHORTFALL` set. An invariant
/// written only about hard entries would be vacuous against the driver this
/// epoch actually has.
///
/// The queue publishes the maximum it has ever observed, which is I3's own
/// witness, so this is a comparison and not a scan.
#[must_use]
pub const fn collection_never_starves_an_urgent_read(queue: &Queue) -> bool {
    queue.ops_ahead_of_urgent_read() <= COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ
}

/// All three at once, for a caller that wants one line rather than three.
///
/// `zone` is the zone about to be reset; a caller with no reset in hand passes
/// the zone it is asking about anyway, because I1 is a statement *about a
/// zone* and a version of it that quantified over all of them would be a
/// different and much weaker claim — every sealed zone has bits set in it, and
/// that is the design working.
#[must_use]
pub fn all_three<Z: Zoned>(
    map: &ZoneMap<Z>,
    mark: &Mark,
    roots: &Roots,
    queue: &Queue,
    zone: u32,
) -> bool {
    nothing_reachable_is_swept(map, mark, roots, zone)
        && a_reset_zone_holds_no_live_blob(map)
        && collection_never_starves_an_urgent_read(queue)
}
