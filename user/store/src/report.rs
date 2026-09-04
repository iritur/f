// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a runtime tells the frame on its way out, and the one place the two
//! sides agree about how to read it.
//!
//! # Why an exit status and not a state tree
//!
//! RFC 0013 says every component publishes a state tree, and a runtime
//! eventually will. What it needs *here* is smaller and has to survive one
//! thing a tree does not: the frame reads this after the component's address
//! space is gone. A status is one word carried out through the door's last
//! call, which is the boundary crossing this whole measurement is defined
//! against — so it costs nothing the counter is trying to count.
//!
//! *Reversal:* a runtime that publishes a tree of its own, at which point the
//! tallies are nodes and this module is one `code` field.
//!
//! # Why it lives in this crate rather than in `f_abi`
//!
//! Because it is not wire between peers. It is one component's report to the
//! frame that links it, which is the arrangement `user/virtio-blk` already has
//! and `kernel/Cargo.toml` already states a reversal for. Putting it in `f_abi`
//! would make one demonstration's encoding part of the ABI every future
//! component is judged against.

/// How many work items a runtime puts through its own executor.
///
/// Sixteen thousand, and the number is chosen against two bounds rather than
/// picked. Below it the run finishes inside one timer interval, so the tick
/// count the honest exclusion is stated against would be zero and the exclusion
/// would be untestable. Above sixty-five thousand it stops fitting in
/// [`pack`]'s sixteen bits, and a tally that saturates is a tally that lies.
/// Unit: work items.
pub const LOAD: u32 = 16_384;

/// How many work items one quantum submits before the runtime returns to its
/// polling point.
///
/// Eight, which is half a sixteen-entry ring — so a quantum always fits and the
/// executor never meets a full ring in the ordinary case. **This is the
/// allocation boundary**: a runtime is preempted between quanta and never
/// inside one, and it is what makes *park cleanly* expressible at all. A
/// runtime with a quantum of one would be a runtime that can always park and
/// never gets any work done; one with a quantum of the whole load could not
/// park before the load was over.
/// Unit: work items.
pub const QUANTUM: u32 = 8;

/// The selector a runtime is entered with to run its load.
/// Unit: none — a selector ordinal, in the low half of `f_abi::door::Entry`.
pub const RUN: u32 = 1;

/// The selector that runs the same load and makes one door call in the middle
/// of it on purpose.
///
/// It exists for the reason `state::node::MEMORY_FORCED` exists beside
/// `MEMORY_REMOTE`, and `BLK_PROVOKED` beside `BLK_COPIES`: **a counter nothing
/// in a boot can move is indistinguishable from a counter that does not work.**
/// The frame requires the hot-path count to be zero under [`RUN`] and non-zero
/// under this, so a build in which counting had stopped fails rather than looks
/// clean.
/// Unit: none — a selector ordinal.
pub const PROVOKE: u32 = 2;

/// The run did what it meant to.
pub const OK: u8 = 0;

/// The control ring would not adopt. The refusal is in the tally's `completed`
/// and `parked` fields, which are the two holes a failing run has no other use
/// for: see [`refusal`].
pub const NO_CONTROL: u8 = 1;

/// The work ring would not adopt.
pub const NO_WORK: u8 = 2;

/// A ring refused mid-loop: a cursor a peer made impossible, or a ring that
/// filled when the quantum says it cannot.
pub const RING_REFUSED: u8 = 3;

/// The runtime finished with work still outstanding on its own queue, which is
/// the opposite of parking cleanly.
pub const NOT_QUIET: u8 = 4;

/// A completion arrived carrying a notice kind this build cannot name.
///
/// R04 does not permit a component to skip the entry: an eighth notice kind
/// raises `f_abi::ABI_VERSION` so RFC 0011 keeps it off a channel whose peer
/// does not know it, and a component that meets one anyway has found a frame
/// bug and exits saying so.
pub const UNKNOWN_NOTICE: u8 = 5;

/// A completion arrived that answers no submission and carries no notice flag.
pub const STRAY_COMPLETION: u8 = 6;

/// Set when the runtime parked because it was told a core was being reclaimed.
/// Unit: none — a flag.
pub const RECLAIMED: u8 = 1 << 0;

/// Set when the runtime's own queue was empty at the moment it exited.
///
/// This is what *cleanly* means and it is the half a deadline cannot express:
/// a runtime that stopped at the deadline with work still on its ring has
/// abandoned it rather than parked it.
/// Unit: none — a flag.
pub const QUIESCENT: u8 = 1 << 1;

/// What a runtime did, as the frame reads it back out of the exit status.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tally {
    /// Work items the runtime's own executor completed.
    /// Unit: work items, except under a `code` of [`NO_CONTROL`] or
    /// [`NO_WORK`], where it is the `f_abi::error` domain of the refusal that
    /// stopped the run. See [`refusal`].
    pub completed: u32,
    /// Work items it never started, because it parked first — or, on a run that
    /// failed to adopt, the reason it was refused with.
    /// Unit: work items, except under a `code` of [`NO_CONTROL`] or
    /// [`NO_WORK`], where it is a whole `f_abi::error` reason.
    pub parked: u32,
    /// Notices it drained off its control ring. Unit: notices.
    pub notices: u32,
    /// Door calls it made on purpose, before the one that ended it.
    /// Unit: kernel entries.
    pub provoked: u32,
    /// [`RECLAIMED`] and [`QUIESCENT`]. Unit: none — a bitmask.
    pub flags: u8,
    /// [`OK`], or which way it stopped. Unit: none — a status ordinal.
    pub code: u8,
}

impl Tally {
    /// Did it park because it was told to?
    #[must_use]
    pub const fn reclaimed(&self) -> bool {
        self.flags & RECLAIMED != 0
    }

    /// Was its own queue empty when it went?
    #[must_use]
    pub const fn quiescent(&self) -> bool {
        self.flags & QUIESCENT != 0
    }
}

/// The tally a run that could not adopt reports: nothing done, and the refusal
/// that stopped it.
///
/// **Two fields and not one, and the second is the scar.** The refusal used to
/// travel packed into `parked` alone as `(domain << 8) | (reason & 0xFF)`,
/// which is sixteen bits holding a twenty-four bit pair — so two reasons in one
/// domain differing only above bit seven compared equal, and the frame's check
/// that a scribbled header was refused *for the right reason* would have
/// accepted the wrong one. A comparison that cannot distinguish two values
/// accepts one of them by accident, which is R04 read backwards.
///
/// A run that never adopted a ring has completed no work, so `completed` is
/// zero by construction and the domain is free to sit there. That is why the
/// pair fits without a bit of either half being dropped, and why
/// [`refusal_of`] can hand both back whole.
#[must_use]
pub const fn refusal(code: u8, domain: u8, reason: u16) -> Tally {
    Tally {
        completed: domain as u32,
        parked: reason as u32,
        notices: 0,
        provoked: 0,
        flags: 0,
        code,
    }
}

/// The refusal a tally carries, if the run ended in one.
///
/// `None` for every other `code`, because on those two fields mean work items
/// and reading a refusal out of them would be inventing one.
#[must_use]
pub const fn refusal_of(tally: &Tally) -> Option<(u8, u16)> {
    match tally.code {
        NO_CONTROL | NO_WORK => Some((tally.completed as u8, tally.parked as u16)),
        _ => None,
    }
}

/// Was this run refused with exactly this domain and this reason?
///
/// The whole pair, compared whole. The frame asks it of the component's report
/// and the component builds that report with [`refusal`], so the two sides
/// cannot drift about where the halves live.
#[must_use]
pub const fn refused_with(tally: &Tally, domain: u8, reason: u16) -> bool {
    match refusal_of(tally) {
        Some((carried_domain, carried_reason)) => {
            carried_domain == domain && carried_reason == reason
        }
        None => false,
    }
}

/// Pack a tally into the one word `f_abi::door::EXIT` carries.
///
/// Saturating rather than wrapping, in every field. A tally that wrapped would
/// report a small number for a large one, which is the shape of lie a counter
/// exists to not tell; a saturated one reports its own ceiling, which a reader
/// can recognise.
#[must_use]
pub const fn pack(tally: Tally) -> u64 {
    let completed = if tally.completed > 0xFFFF { 0xFFFF } else { tally.completed } as u64;
    let parked = if tally.parked > 0xFFFF { 0xFFFF } else { tally.parked } as u64;
    let notices = if tally.notices > 0xFF { 0xFF } else { tally.notices } as u64;
    let provoked = if tally.provoked > 0xFF { 0xFF } else { tally.provoked } as u64;
    completed
        | (parked << 16)
        | (notices << 32)
        | (provoked << 40)
        | ((tally.flags as u64) << 48)
        | ((tally.code as u64) << 56)
}

/// Read one back.
#[must_use]
pub const fn unpack(status: u64) -> Tally {
    Tally {
        completed: (status & 0xFFFF) as u32,
        parked: ((status >> 16) & 0xFFFF) as u32,
        notices: ((status >> 32) & 0xFF) as u32,
        provoked: ((status >> 40) & 0xFF) as u32,
        flags: ((status >> 48) & 0xFF) as u8,
        code: ((status >> 56) & 0xFF) as u8,
    }
}

/// A word for a log.
#[must_use]
pub const fn label(code: u8) -> &'static str {
    match code {
        OK => "ok",
        NO_CONTROL => "the control ring would not adopt",
        NO_WORK => "the work ring would not adopt",
        RING_REFUSED => "a ring refused mid-loop",
        NOT_QUIET => "work was still outstanding at the exit",
        UNKNOWN_NOTICE => "a notice kind this build cannot name",
        STRAY_COMPLETION => "a completion answering nothing and flagged as nothing",
        _ => "a status this build does not name",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field survives the one word it has to cross in.
    #[test]
    fn a_tally_survives_the_door() {
        let tally = Tally {
            completed: 16_384,
            parked: 7,
            notices: 3,
            provoked: 1,
            flags: RECLAIMED | QUIESCENT,
            code: OK,
        };
        assert_eq!(unpack(pack(tally)), tally);
        assert!(unpack(pack(tally)).reclaimed());
        assert!(unpack(pack(tally)).quiescent());
    }

    /// A field past its ceiling reports the ceiling rather than a small number
    /// that looks like a success.
    #[test]
    fn a_field_past_its_ceiling_saturates_rather_than_wrapping() {
        let tally = Tally { completed: 0x1_0001, notices: 300, ..Tally::default() };
        let read = unpack(pack(tally));
        assert_eq!(read.completed, 0xFFFF);
        assert_eq!(read.notices, 0xFF);
    }

    /// A reason whose high byte is set survives, which the encoding this
    /// replaced could not do.
    ///
    /// The regression test for a real defect rather than a demonstration of an
    /// obvious property: `(domain << 8) | (reason & 0xFF)` reported
    /// `AUTHORITY/0x0101` and `AUTHORITY/0x0001` as the same sixteen bits, so
    /// a frame checking *which* refusal it got would have accepted either.
    #[test]
    fn a_refusal_keeps_every_bit_of_its_reason() {
        let one = refusal(NO_CONTROL, 3, 0x0101);
        let other = refusal(NO_CONTROL, 3, 0x0001);
        assert_ne!(one, other);
        assert_eq!(refusal_of(&unpack(pack(one))), Some((3, 0x0101)));
        assert_eq!(refusal_of(&unpack(pack(other))), Some((3, 0x0001)));
        assert!(refused_with(&unpack(pack(one)), 3, 0x0101));
        assert!(!refused_with(&unpack(pack(one)), 3, 0x0001));
    }

    /// A tally that is not a refusal does not answer as one, because on those
    /// runs the two fields hold work items.
    #[test]
    fn a_run_that_was_not_refused_carries_no_refusal() {
        let tally = Tally { completed: 5, parked: 1, code: OK, ..Tally::default() };
        assert_eq!(refusal_of(&tally), None);
        assert!(!refused_with(&tally, 0, 5));
    }

    /// The load fits the field it is reported in, which is the bound the
    /// constant's own comment claims.
    ///
    /// A `const` block rather than a runtime assertion, so that a load raised
    /// past the sixteen bits [`pack`] gives it is a compile error rather than a
    /// test failure — which is the difference between a saturating tally being
    /// impossible and being noticed.
    #[test]
    fn the_load_fits_the_tally_that_reports_it() {
        const { assert!(LOAD <= 0xFFFF) };
        const { assert!(QUANTUM > 0 && QUANTUM < LOAD) };
    }
}
