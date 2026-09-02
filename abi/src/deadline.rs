// SPDX-License-Identifier: Apache-2.0 OR MIT
//! How far a caller's deadline reaches, and what it may not do on the way.
//!
//! The resource document says the submission entry carries a deadline so that
//! a task blocking on another component's work does not wait behind batch work
//! in that component's queue — the deadline travels with the request through
//! every ring it crosses, and every resource scheduler orders by it. The same
//! document, in its own weakest-points section, says the mechanism can be
//! gamed: if inheriting a caller's deadline promotes work, a component that
//! always claims urgency wins. Its answer is to bound inheritance by the
//! caller's reservation and to design the bound *before* the first starvation
//! bug. This module is that bound, and RFC 0025
//! (`docs/rfc/0025-a-deadline-inherits-downward-and-decays.md`) is the argument
//! for its shape.
//!
//! # The rule in one paragraph
//!
//! A request arriving over a ring is served at **the less urgent of the class
//! it carries and the class the callee was admitted for**, with **the later of
//! the deadline it carries and its arrival plus the callee's floor**, and only
//! for **[`MAX_DEPTH`] rings** from the component that originated it. A caller
//! may not carry a class it was not itself admitted for: that entry is refused,
//! not demoted, because an entry that lies about its submitter is a malformed
//! entry and R04 says those are refused. Every other way the request falls
//! short of what it asked is reported in the result and, by the service, on the
//! completion as [`cflags::SHORTFALL`](crate::cflags::SHORTFALL) — served
//! differently is fine, served differently *silently* is the failure the whole
//! discipline exists to exclude.
//!
//! # Why the urgency has a scope
//!
//! Everything [`inherit`] returns is a property of one request. It is computed
//! from the entry and from what the callee already knows, it is carried by the
//! service while that request is in flight, and it ends when the request
//! completes. Nothing here can be stored against a *component*: the callee's
//! own work has the callee's own class and no deadline, whatever it served a
//! moment ago. That is what stops a service becoming permanently urgent by
//! having once had an urgent client — and it is why this is a pure function
//! rather than a scheduler with a memory.
//!
//! # What this module reads, and what it decides not to
//!
//! [`Sqe::class`] is read as two bytes: the low one is a class ordinal, one of
//! the four `class` constants and nothing else; the high one is how many rings
//! the request's urgency has already crossed. The earlier reading of the field
//! — a class in the high bits and a *priority ordinal* in the low — had no
//! reader and no writer, and RFC 0025 retires it on purpose: the scheduler's
//! primitive is a deadline because priority conflates urgency with importance,
//! and a priority sub-field would have reintroduced exactly the number the
//! design refuses to collapse into.
//!
//! Admission itself is not here. Whether a component holds the class it is
//! admitted for is decided at spawn (RFC 0007, RFC 0008) and arrives at a
//! service as a fact about the channel, which is why [`Admitted`] can only be
//! built from a valid ordinal and is a parameter rather than something read
//! off the wire. `E1-B07` builds the test that grants it; `E1-B06` orders the
//! device queues by what this returns.

use crate::{NO_DEADLINE, Sqe, class, error};

/// How many rings a caller's urgency may cross before it stops being the
/// caller's.
///
/// Four is a parameter, not a measurement: the deepest chain the epoch's
/// topology has is an application, an object store, an index and a block
/// driver, which is three crossings, and the fourth is headroom rather than a
/// forecast. A legitimate chain deeper than this, root-caused, is what raises
/// it (RFC 0025). Unit: rings crossed.
pub const MAX_DEPTH: u8 = 4;

/// What a request is served as once its urgency has run out of depth: batch,
/// whoever sent it, unless the callee is admitted for less.
///
/// Beyond the bound a request may still not be *promoted* — a caller that was
/// idle stays idle — so this is a ceiling on what the request may claim, not
/// the class it is assigned. Unit: none — a class ordinal.
pub const BEYOND_DEPTH: u16 = class::BATCH;

/// The bits of [`Sqe::class`] that hold the class ordinal.
const CLASS_MASK: u16 = 0x00FF;
/// Where the depth sits in [`Sqe::class`].
const DEPTH_SHIFT: u32 = 8;

/// The class ordinal in a `class` field, without the depth.
#[inline]
#[must_use]
pub const fn class_of(field: u16) -> u16 {
    field & CLASS_MASK
}

/// The inheritance depth in a `class` field.
#[inline]
#[must_use]
pub const fn depth_of(field: u16) -> u8 {
    (field >> DEPTH_SHIFT) as u8
}

/// Build a `class` field. What a component writes when it originates a request
/// (depth zero) and what a service writes downstream (the depth
/// [`inherit`] handed it).
#[inline]
#[must_use]
pub const fn pack(class: u16, depth: u8) -> u16 {
    ((depth as u16) << DEPTH_SHIFT) | (class & CLASS_MASK)
}

/// Is this one of the four class ordinals? Anything else is refused: R04.
#[inline]
#[must_use]
pub const fn is_class(ordinal: u16) -> bool {
    ordinal == class::HARD
        || ordinal == class::SOFT
        || ordinal == class::BATCH
        || ordinal == class::IDLE
}

/// A class a component was admitted for.
///
/// The one input to [`inherit`] that does not come off the wire. A component's
/// ceiling is declared in its manifest and granted at spawn — a hard-class
/// reservation after RFC 0007's test, a soft-class standing as a right the
/// supervisor routes, batch for a component that declares nothing, which is
/// also what [`Sqe::ZERO`] writes — and it reaches a service as a fact about
/// the channel the request arrived on. Constructible only from a valid
/// ordinal, so a service that holds one holds something it need not re-check.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Admitted(u16);

impl Admitted {
    /// From an ordinal. `None` for a value that names no class.
    #[must_use]
    pub const fn new(ordinal: u16) -> Option<Self> {
        if is_class(ordinal) { Some(Self(ordinal)) } else { None }
    }

    /// The ordinal.
    #[inline]
    #[must_use]
    pub const fn class(self) -> u16 {
        self.0
    }
}

/// The caller's side of a crossing: what the entry says, and what the
/// channel says about who wrote it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Caller {
    /// [`Sqe::class`] as written: class ordinal in the low byte, depth in the
    /// high byte. Unit: none — see [`class_of`] and [`depth_of`] for the two
    /// readings. Zero is `class::HARD` at depth zero, and is refused from any
    /// caller not admitted for the hard class.
    pub class: u16,
    /// [`Sqe::deadline`] as written. Unit: nanoseconds, monotonic, in the
    /// channel's epoch — RFC 0009. Zero is [`NO_DEADLINE`].
    pub deadline: u64,
    /// The class the caller was admitted for on this channel. Known to the
    /// service from the grant, never from the entry.
    /// Unit: none — a class ordinal.
    pub admitted: Admitted,
}

impl Caller {
    /// Read the two fields off an entry and pair them with what the channel
    /// knows about the submitter.
    #[must_use]
    pub const fn of(entry: &Sqe, admitted: Admitted) -> Self {
        Self { class: entry.class, deadline: entry.deadline, admitted }
    }
}

/// The callee's side of a crossing: what it is admitted for, when the entry
/// arrived, and the least it needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Callee {
    /// The class this component was admitted for. A request is never served
    /// above it. Unit: none — a class ordinal.
    pub admitted: Admitted,
    /// When the entry was observed, on the same clock as the deadline.
    /// Unit: nanoseconds, monotonic, in the channel's epoch — RFC 0009. Zero
    /// is the clock's origin and is a legal arrival.
    pub arrival: u64,
    /// The least time this component needs from arrival to completion for any
    /// request — its worst-case service time, or a bound it stands behind. An
    /// inherited deadline is never earlier than arrival plus this, so a
    /// deadline of one nanosecond buys a caller nothing over an honest one.
    /// Unit: nanoseconds. Zero is a callee that claims to need no time, which
    /// disables the floor and is the callee's problem.
    pub floor: u64,
}

/// What a request is served as inside the callee, and what it lost on the way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Inherited {
    /// The class the request runs at here. Unit: none — a class ordinal,
    /// smaller is more urgent. Never smaller than the callee's admitted class.
    pub class: u16,
    /// The deadline the request runs against here, and the one the callee
    /// writes into any entry it submits on the request's behalf.
    /// Unit: nanoseconds, monotonic, in the channel's epoch — RFC 0009. Zero is
    /// [`NO_DEADLINE`]: nothing was inherited, and the callee orders the
    /// request however it likes within its class.
    pub deadline: u64,
    /// The depth to write into any entry the callee submits on the request's
    /// behalf. Unit: rings crossed, counting this one. Saturates at
    /// [`MAX_DEPTH`].
    pub depth: u8,
    /// How the service fell short of what the entry asked, as a mask of the
    /// [`shortfall`] constants. Unit: none — a bitmask. Zero is served exactly
    /// as asked, and anything else is reported on the completion.
    pub shortfall: u8,
}

impl Inherited {
    /// The `class` field for an entry submitted downstream on this request's
    /// behalf.
    #[inline]
    #[must_use]
    pub const fn class_field(&self) -> u16 {
        pack(self.class, self.depth)
    }

    /// The key a device queue orders by, ascending: class first, then deadline,
    /// with [`NO_DEADLINE`] last within its class.
    ///
    /// `E1-B06` is the task that makes every resource scheduler use it; it is
    /// here so that the ordering the RFC describes is one line rather than a
    /// convention each scheduler restates.
    #[inline]
    #[must_use]
    pub const fn rank(&self) -> (u16, u64) {
        let deadline = if self.deadline == NO_DEADLINE { u64::MAX } else { self.deadline };
        (self.class, deadline)
    }

    /// Did the request get less than it asked for?
    #[inline]
    #[must_use]
    pub const fn fell_short(&self) -> bool {
        self.shortfall != 0
    }
}

/// The ways a request is served below what it asked for. Each is a fact the
/// caller can act on and none is an error: the request ran.
pub mod shortfall {
    /// Served at a less urgent class than the entry carried, because the
    /// callee is admitted for less. The hard-class read at a soft-class
    /// service.
    pub const CLASS: u8 = 1 << 0;
    /// Served against a deadline later than the entry carried, because the
    /// entry's was inside the callee's floor — already late, or asking for
    /// less time than any request here takes.
    pub const LATE: u8 = 1 << 1;
    /// The request's urgency had crossed [`super::MAX_DEPTH`] rings and ended
    /// here: no deadline, and no class above [`super::BEYOND_DEPTH`].
    pub const DEPTH: u8 = 1 << 2;
}

/// Decide what a request is served as when it crosses a ring.
///
/// Pure: the answer is a function of the entry, the channel's knowledge of the
/// submitter, and the callee's own admission and clock. Nothing is remembered
/// between calls, which is the property that makes urgency end with the
/// request that carried it.
///
/// Refusals, each an `ARGUMENT` or `ADMISSION` error under RFC 0010 with the
/// offending `class` field as its detail:
/// - the low byte names no class, or the high byte is a depth nobody could
///   have written — `ARGUMENT`/[`BAD_CLASS`](error::argument::BAD_CLASS);
/// - the class is more urgent than the caller was admitted for —
///   `ADMISSION`/[`NOT_HELD`](error::admission::NOT_HELD). Refused rather
///   than demoted, because a caller that could write `HARD` and be served
///   at its ceiling with a flag would have nothing to lose by writing it on
///   every entry, and nothing to lose is how urgency becomes a default.
///
/// # Errors
///
/// The packed error and the field it names, as a service writes them into a
/// completion's `result` and `ext`.
pub const fn inherit(caller: &Caller, callee: Callee) -> Result<Inherited, (i32, u64)> {
    let detail = caller.class as u64;
    let claimed = class_of(caller.class);
    let depth = depth_of(caller.class);
    if !is_class(claimed) || depth > MAX_DEPTH {
        return Err((error::pack(error::ARGUMENT, error::argument::BAD_CLASS), detail));
    }
    // Smaller is more urgent, so "more urgent than admitted" is a smaller
    // ordinal than the ceiling.
    if claimed < caller.admitted.class() {
        return Err((error::pack(error::ADMISSION, error::admission::NOT_HELD), detail));
    }

    let mut shortfall = 0;

    // The callee's admission is a ceiling: never served above it, and told so.
    let mut class = max_u16(claimed, callee.admitted.class());
    if class != claimed {
        shortfall |= shortfall::CLASS;
    }

    // The floor: never earlier than arrival plus what any request here takes.
    // A deadline in the past lands here too, which is what "a late request of
    // whatever class it claimed" means in practice.
    let mut deadline = caller.deadline;
    if deadline != NO_DEADLINE {
        let earliest = callee.arrival.saturating_add(callee.floor);
        if deadline < earliest {
            deadline = earliest;
            shortfall |= shortfall::LATE;
        }
    }

    // The depth bound: the caller's urgency reaches MAX_DEPTH rings and no
    // further. Past it the request is batch work with no deadline — still
    // never promoted, so an idle caller stays idle — and the counter saturates
    // so that nothing downstream can restart the chain.
    let next_depth = if depth < MAX_DEPTH {
        depth + 1
    } else {
        let bounded = max_u16(class, BEYOND_DEPTH);
        if bounded != class || deadline != NO_DEADLINE {
            shortfall |= shortfall::DEPTH;
        }
        class = bounded;
        deadline = NO_DEADLINE;
        MAX_DEPTH
    };

    Ok(Inherited { class, deadline, depth: next_depth, shortfall })
}

/// `core::cmp::max` is not `const` for `u16`.
const fn max_u16(a: u16, b: u16) -> u16 {
    if a > b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARD: Admitted = Admitted(class::HARD);
    const SOFT: Admitted = Admitted(class::SOFT);
    const BATCH: Admitted = Admitted(class::BATCH);
    const IDLE: Admitted = Admitted(class::IDLE);

    const ARRIVAL: u64 = 1_000_000;
    const FLOOR: u64 = 50_000;

    fn callee(admitted: Admitted) -> Callee {
        Callee { admitted, arrival: ARRIVAL, floor: FLOOR }
    }

    fn caller(class: u16, deadline: u64, admitted: Admitted) -> Caller {
        Caller { class: pack(class, 0), deadline, admitted }
    }

    #[test]
    fn the_class_field_packs_and_unpacks() {
        for ordinal in [class::HARD, class::SOFT, class::BATCH, class::IDLE] {
            for depth in 0..=MAX_DEPTH {
                let field = pack(ordinal, depth);
                assert_eq!(class_of(field), ordinal);
                assert_eq!(depth_of(field), depth);
            }
        }
        // The constants already in use are depth-zero fields of themselves,
        // which is what keeps `class: class::SOFT` meaning what it always did.
        assert_eq!(pack(class::SOFT, 0), class::SOFT);
        assert_eq!(class_of(Sqe::ZERO.class), class::BATCH);
        assert_eq!(depth_of(Sqe::ZERO.class), 0);
    }

    #[test]
    fn a_deadline_never_gets_earlier_by_inheritance() {
        let deadlines = [1, ARRIVAL - 1, ARRIVAL, ARRIVAL + FLOOR - 1, ARRIVAL + FLOOR, u64::MAX];
        for original in deadlines {
            for admitted in [HARD, SOFT, BATCH, IDLE] {
                let got = inherit(&caller(class::HARD, original, HARD), callee(admitted)).unwrap();
                assert!(got.deadline >= original, "{original} became {}", got.deadline);
            }
        }
    }

    #[test]
    fn a_deadline_outside_the_floor_is_never_later_than_the_callers() {
        for original in [ARRIVAL + FLOOR, ARRIVAL + FLOOR + 1, ARRIVAL + 10 * FLOOR, u64::MAX] {
            let got = inherit(&caller(class::SOFT, original, SOFT), callee(SOFT)).unwrap();
            assert_eq!(got.deadline, original);
            assert!(!got.fell_short());
        }
    }

    #[test]
    fn a_deadline_inside_the_floor_is_floored_and_says_so() {
        // Past, at arrival, and inside the floor all land on the same value:
        // the earliest this callee can honestly promise. That is what stops a
        // deadline of one nanosecond from being a priority with a better name.
        for original in [1, ARRIVAL - 1, ARRIVAL, ARRIVAL + FLOOR - 1] {
            let got = inherit(&caller(class::SOFT, original, SOFT), callee(SOFT)).unwrap();
            assert_eq!(got.deadline, ARRIVAL + FLOOR);
            assert_eq!(got.shortfall, shortfall::LATE);
        }
        // With no floor the callee promises anything, including the past.
        let no_floor = Callee { admitted: SOFT, arrival: ARRIVAL, floor: 0 };
        let got = inherit(&caller(class::SOFT, ARRIVAL - 1, SOFT), no_floor).unwrap();
        assert_eq!(got.deadline, ARRIVAL);
        assert_eq!(got.shortfall, shortfall::LATE);
        let got = inherit(&caller(class::SOFT, ARRIVAL, SOFT), no_floor).unwrap();
        assert_eq!(got.deadline, ARRIVAL);
        assert!(!got.fell_short());
    }

    #[test]
    fn no_deadline_is_inherited_as_no_deadline() {
        let got = inherit(&caller(class::HARD, NO_DEADLINE, HARD), callee(HARD)).unwrap();
        assert_eq!(got.deadline, NO_DEADLINE);
        assert!(!got.fell_short(), "nothing was asked, so nothing fell short");
        assert_eq!(got.rank(), (class::HARD, u64::MAX), "and it sorts last within its class");
    }

    #[test]
    fn a_callee_is_never_promoted_above_its_admitted_class() {
        let deadline = ARRIVAL + 2 * FLOOR;
        for admitted in [HARD, SOFT, BATCH, IDLE] {
            for claimed in [class::HARD, class::SOFT, class::BATCH, class::IDLE] {
                let got = inherit(&caller(claimed, deadline, HARD), callee(admitted)).unwrap();
                assert!(
                    got.class >= admitted.class(),
                    "{claimed} at a {admitted:?} callee ran at {}",
                    got.class
                );
                assert_eq!(got.class, claimed.max(admitted.class()));
                // Demotion is reported; being served as asked is not.
                assert_eq!(got.shortfall & shortfall::CLASS != 0, claimed < admitted.class());
                // The deadline is kept: a hard read at a soft service is served
                // best-effort against the same instant, not against nothing.
                assert_eq!(got.deadline, deadline);
            }
        }
    }

    #[test]
    fn a_batch_request_at_a_soft_service_is_served_as_batch() {
        // The other half of the same rule: the callee's class is a ceiling and
        // not a floor. Otherwise every soft service would promote whatever it
        // touched, and inheritance would be the leak it exists to plug.
        let got = inherit(&caller(class::BATCH, NO_DEADLINE, BATCH), callee(SOFT)).unwrap();
        assert_eq!(got.class, class::BATCH);
        assert!(!got.fell_short());
    }

    #[test]
    fn a_class_the_caller_does_not_hold_is_refused_not_demoted() {
        for (claimed, admitted) in [
            (class::HARD, SOFT),
            (class::HARD, BATCH),
            (class::HARD, IDLE),
            (class::SOFT, BATCH),
            (class::SOFT, IDLE),
            (class::BATCH, IDLE),
        ] {
            let entry = caller(claimed, ARRIVAL + FLOOR, admitted);
            assert_eq!(
                inherit(&entry, callee(HARD)),
                Err((
                    error::pack(error::ADMISSION, error::admission::NOT_HELD),
                    entry.class as u64
                ))
            );
        }
        // A zeroed class field claims HARD, and a caller not admitted for HARD
        // is refused for it. That is why `Sqe::ZERO` writes BATCH explicitly,
        // and why a hostile peer gets nothing by zeroing the entry.
        let zeroed = Caller { class: 0, deadline: NO_DEADLINE, admitted: BATCH };
        assert!(inherit(&zeroed, callee(HARD)).is_err());
        assert!(inherit(&Caller { admitted: HARD, ..zeroed }, callee(HARD)).is_ok());
    }

    #[test]
    fn a_field_that_names_no_class_is_refused() {
        for bad in [4u16, 0x00FF, 0x0104 | 0x00F0] {
            let entry = Caller { class: bad, deadline: NO_DEADLINE, admitted: HARD };
            assert_eq!(
                inherit(&entry, callee(IDLE)),
                Err((error::pack(error::ARGUMENT, error::argument::BAD_CLASS), bad as u64))
            );
        }
        assert!(Admitted::new(4).is_none());
        assert!(Admitted::new(0x0100 | class::SOFT).is_none(), "a depth is not a class");
    }

    #[test]
    fn the_depth_bound_is_enforced() {
        // Walk a hard-class request down a chain of hard-class services, each
        // forwarding what `inherit` hands it. The deadline survives exactly
        // MAX_DEPTH crossings and then ends.
        let deadline = ARRIVAL + 100 * FLOOR;
        let mut field = pack(class::HARD, 0);
        for hop in 1..=MAX_DEPTH {
            let entry = Caller { class: field, deadline, admitted: HARD };
            let got = inherit(&entry, callee(HARD)).unwrap();
            assert_eq!(got.depth, hop);
            assert_eq!(got.class, class::HARD);
            assert_eq!(got.deadline, deadline);
            assert!(!got.fell_short(), "hop {hop} lost nothing");
            field = got.class_field();
        }
        assert_eq!(depth_of(field), MAX_DEPTH);

        // The crossing after the bound: batch, no deadline, reported.
        let entry = Caller { class: field, deadline, admitted: HARD };
        let got = inherit(&entry, callee(HARD)).unwrap();
        assert_eq!(got.class, BEYOND_DEPTH);
        assert_eq!(got.deadline, NO_DEADLINE);
        assert_eq!(got.shortfall, shortfall::DEPTH);
        assert_eq!(got.depth, MAX_DEPTH, "the counter saturates");

        // And it stays ended: the next service reads the saturated depth and
        // gets the same answer, so the chain cannot restart itself.
        let again = inherit(&Caller { class: got.class_field(), ..entry }, callee(HARD)).unwrap();
        assert_eq!(again, got);

        // A depth nobody could have written is malformed, not merely deep.
        let forged = Caller { class: pack(class::HARD, MAX_DEPTH + 1), deadline, admitted: HARD };
        assert_eq!(
            inherit(&forged, callee(HARD)),
            Err((error::pack(error::ARGUMENT, error::argument::BAD_CLASS), forged.class as u64))
        );
    }

    #[test]
    fn beyond_the_depth_bound_nothing_is_promoted_either() {
        // An idle caller five rings deep is still idle, not batch: the bound
        // is a ceiling on urgency, not a class assignment.
        let entry =
            Caller { class: pack(class::IDLE, MAX_DEPTH), deadline: NO_DEADLINE, admitted: IDLE };
        let got = inherit(&entry, callee(HARD)).unwrap();
        assert_eq!(got.class, class::IDLE);
        assert!(!got.fell_short(), "nothing was lost, so nothing is reported");

        // And a batch request with no deadline, however deep, loses nothing.
        let entry =
            Caller { class: pack(class::BATCH, MAX_DEPTH), deadline: NO_DEADLINE, admitted: BATCH };
        assert!(!inherit(&entry, callee(SOFT)).unwrap().fell_short());
    }

    #[test]
    fn a_completed_requests_urgency_does_not_leak() {
        // The service is a soft-class component. A hard-class request arrives
        // and is served with its deadline; then the request completes and the
        // service turns to its own work, and to a batch request from somebody
        // else. Neither sees anything of the first.
        //
        // What this proves is the shape of the rule, not a scheduler: `inherit`
        // has no state to leak from, so the only place urgency can live is in
        // the value a service holds for a request in flight, and `E1-B06`'s
        // service is where dropping that value at completion is tested with a
        // queue behind it.
        let service = callee(SOFT);
        let urgent = inherit(&caller(class::HARD, ARRIVAL + 2 * FLOOR, HARD), service).unwrap();
        assert_eq!(urgent.class, class::SOFT);
        assert_eq!(urgent.deadline, ARRIVAL + 2 * FLOOR);

        let later = Callee { arrival: ARRIVAL + 3 * FLOOR, ..service };
        let own = inherit(&caller(class::SOFT, NO_DEADLINE, SOFT), later).unwrap();
        assert_eq!(own.deadline, NO_DEADLINE, "the service's own work has no inherited deadline");
        assert_eq!(own.class, class::SOFT);

        let other = inherit(&caller(class::BATCH, NO_DEADLINE, BATCH), later).unwrap();
        assert_eq!(other.class, class::BATCH);
        assert_eq!(other.deadline, NO_DEADLINE);
        assert!(other.rank() > own.rank(), "and it sorts behind the service's own work");
    }

    #[test]
    fn a_hard_read_outranks_queued_batch_work() {
        // The failure the rule exists to prevent, as an ordering: at a block
        // driver admitted for the hard class, a hard-class read with a deadline
        // sorts ahead of batch compaction that arrived first, and batch work
        // with no deadline sorts last of all.
        let driver = callee(HARD);
        let compaction = inherit(&caller(class::BATCH, NO_DEADLINE, BATCH), driver).unwrap();
        let read = inherit(&caller(class::HARD, ARRIVAL + FLOOR, HARD), driver).unwrap();
        let soft = inherit(&caller(class::SOFT, ARRIVAL + FLOOR, SOFT), driver).unwrap();
        assert!(read.rank() < soft.rank());
        assert!(soft.rank() < compaction.rank());
        assert_eq!(compaction.rank().1, u64::MAX);
    }
}
