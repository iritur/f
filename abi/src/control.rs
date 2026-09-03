// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The control channel: the four opcodes a component submits, the seven notices
//! the frame posts, and the pending state the notices are published *from*.
//!
//! # One ring, and the frame at the other end of it
//!
//! Every component has exactly one channel to the frame, created with it, with
//! the component at the producer end. It submits — the capability operations,
//! spawn, connect, stop, grant — and the frame completes. Every event a
//! component ever receives is a completion entry on that ring carrying
//! [`crate::cflags::NOTICE`], drained at a polling point. There is no handler,
//! no interrupted instruction stream and no second path in. RFC 0008, and R05.
//!
//! # Why a notice cannot be lost and cannot pile up
//!
//! Because it is not a queued event. The frame never waits on a component, so
//! it can never block on a full completion ring, and the two obvious answers to
//! a notice that does not fit are both wrong: dropping it makes the control
//! ring advisory, and killing a component for a full ring makes a busy
//! component a dead one.
//!
//! So a notice is **pending state that the frame publishes when there is room**,
//! and every kind has somewhere to be pending that is bounded by something the
//! component has already paid for:
//!
//! - the three handle notices live in the capability slot they concern, as a
//!   [`Pending`] with five states — bounded by the slots the component bought;
//! - a stop is one [`Promise`] per component that only ever moves *earlier*;
//! - a reclaim is one [`Promise`] per core in the component's allocation, so
//!   reclaiming two cores before a drain is two facts and never one;
//! - pressure and generation are one [`Grade`] each, latest wins.
//!
//! The frame publishes them in [`ORDER`], so the ring's depth bounds how much is
//! *visible* and never how much is *true*.
//!
//! # Where the state actually lives
//!
//! Here is the state machine; `kernel/src/cap.rs` is the storage. The split is
//! the one this crate always makes — nothing about *storage* is over there and
//! nothing about the *rules* is over here — and it earns its keep twice: the
//! rules are tested on the host at every collision, and the frame cannot
//! implement a fourth collision case by accident because there is no fourth
//! case to implement.

use crate::{Cqe, cflags};

/// The control channel's opcode space.
///
/// Separate from [`crate::op`], which is the frame's *data* vocabulary, because
/// an opcode space is per service and these are two services that happen to
/// share an implementor. Keeping them apart is what stops
/// `f_ring::Service::drain`'s `known` answering true for an opcode it cannot
/// execute — which would turn R04's refusal into a silence on the one ring
/// whose whole purpose is that nothing is silent.
///
/// The four capability operations are first because RFC 0015 named the opcode
/// that would retire each of them and this is that opcode. The four lifecycle
/// operations follow. RFC 0028 reserves `0xFE` and `0xFF` at the top of every
/// service's space for buffer registration, and nothing here approaches them.
pub mod op {
    /// "What is this handle?" Answers a kind, rights, object and extent.
    /// Retires `door::CAP_INSPECT`.
    pub const INSPECT: u8 = 0x10;
    /// "Mint me a weaker one." Retires `door::CAP_DERIVE`.
    pub const DERIVE: u8 = 0x11;
    /// "Take back everything I handed on from this." Retires
    /// `door::CAP_REVOKE`.
    pub const REVOKE: u8 = 0x12;
    /// "Map this object into this address space." Retires `door::CAP_MAP`.
    pub const MAP: u8 = 0x13;

    /// Create a component: `cap` names the `Untyped` that pays, `ext` names the
    /// manifest by content hash, and the handles satisfying the manifest's
    /// needs follow in the arena in the manifest's order.
    ///
    /// Completes with one new handle in the submitter's table: an `Endpoint` to
    /// the place, carrying every right defined on one.
    pub const SPAWN: u8 = 0x14;
    /// Ask for a channel to whoever occupies an endpoint the submitter holds
    /// with `WRITE`.
    ///
    /// A connect on an *empty* place does not fail. It pends until a spawn
    /// refills the place, the place is retired, or the entry's own deadline
    /// passes — three outcomes and three answers, and the third is
    /// `error::peer::EMPTY`.
    pub const CONNECT: u8 = 0x15;
    /// End the occupant of an endpoint the submitter holds with `REVOKE`, by
    /// the entry's deadline.
    ///
    /// A stop with `NO_DEADLINE` is a promise nothing can refuse, and the frame
    /// refuses to make it: that is an `ARGUMENT` error. A stop whose deadline
    /// has already passed is a kill, spelled the same way as a polite stop so
    /// that the simulator's *kill this driver at a seeded moment* is one opcode
    /// rather than two paths through the frame.
    pub const STOP: u8 = 0x16;
    /// Derive a capability the submitter holds with `GRANT` into the table of
    /// the occupant of an endpoint. The powerbox's one operation.
    pub const GRANT: u8 = 0x17;

    /// Is this an opcode this build implements?
    ///
    /// R04: an unknown opcode is refused and never ignored, and that check
    /// needs one list to compare against rather than a match arm in each
    /// reader.
    #[must_use]
    pub const fn known(opcode: u8) -> bool {
        matches!(opcode, INSPECT | DERIVE | REVOKE | MAP | SPAWN | CONNECT | STOP | GRANT)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(opcode: u8) -> &'static str {
        match opcode {
            INSPECT => "inspect",
            DERIVE => "derive",
            REVOKE => "revoke",
            MAP => "map",
            SPAWN => "spawn",
            CONNECT => "connect",
            STOP => "stop",
            GRANT => "grant",
            _ => "unknown",
        }
    }
}

/// The seven notice kinds, as [`Cqe::result`] values on an entry carrying
/// [`cflags::NOTICE`].
///
/// Seven, and a version of the ABI that adds an eighth raises
/// [`crate::ABI_VERSION`] so that RFC 0011 keeps it off a channel whose peer
/// does not know it. A component that nonetheless meets a kind it cannot name
/// has found a frame bug and exits saying so, because R04 does not permit it to
/// skip the entry.
pub mod notice {
    /// A capability was placed in your table, and `ext` says which need or ask
    /// it satisfies. `user_data` is the new handle.
    pub const GRANTED: i32 = 1;
    /// A capability you held is gone. `user_data` is the dead handle; a
    /// submission carrying it earns `AUTHORITY/REVOKED`.
    pub const REVOKED: i32 = 2;
    /// The far end of a channel, or the occupant behind an endpoint, ended.
    /// `user_data` is the channel or endpoint handle and `ext` is the cause.
    pub const PEER_GONE: i32 = 3;
    /// End yourself by the deadline in `ext`. `user_data` is the control ring's
    /// own handle.
    pub const STOP: i32 = 4;
    /// The core named in `ext` leaves your allocation at the deadline also in
    /// `ext`. One notice per core, never one for several.
    pub const RECLAIM: i32 = 5;
    /// The pressure grade in `ext` changed for the account that pays for you.
    pub const PRESSURE: i32 = 6;
    /// The system generation is changing, or suspending. RFC 0006 and RFC 0012
    /// say what; RFC 0008 reserves the word.
    pub const GENERATION: i32 = 7;

    /// Is this a kind this build defines?
    #[must_use]
    pub const fn known(kind: i32) -> bool {
        matches!(kind, GRANTED | REVOKED | PEER_GONE | STOP | RECLAIM | PRESSURE | GENERATION)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(kind: i32) -> &'static str {
        match kind {
            GRANTED => "granted",
            REVOKED => "revoked",
            PEER_GONE => "peer gone",
            STOP => "stop",
            RECLAIM => "reclaim",
            PRESSURE => "pressure",
            GENERATION => "generation",
            _ => "unknown",
        }
    }
}

/// The order the frame publishes pending state in.
///
/// Slots ascending, then the stop, then reclaim by core ascending, then the two
/// grades. Fixed, because a seeded run has to reproduce it and because a
/// publication order chosen by whatever the frame noticed first is not an order
/// at all.
///
/// What is *not* promised is ordering across kinds as events: a component that
/// drains late sees a revoked notice for slot three before a peer-gone for slot
/// nine whatever order the events had. Ordering *within a slot* is promised and
/// is [`Pending`]'s rule 2. A component that needs to know *when* reads
/// [`Cqe::timestamp`], which every completion already carries.
pub const ORDER: [&str; 4] = ["slots ascending", "stop", "reclaim by core ascending", "grades"];

/// Why a component's peer ended, as the `ext` of a peer-gone notice.
///
/// RFC 0008 requires the cause to be carried and does not spell it. Three
/// causes, because there are three ways a component ends and a client that
/// cannot tell them apart cannot tell a crash from a planned shutdown — which
/// is exactly the distinction a restart policy is written in terms of.
pub mod cause {
    /// An exception at ring 3, or a control ring the component corrupted. The
    /// high half carries the vector.
    pub const FAULT: u64 = 1;
    /// The component asked to end. The high half carries its status.
    pub const EXIT: u64 = 2;
    /// A stop's deadline passed with the component still running, or arrived
    /// already past.
    pub const STOPPED: u64 = 3;
    /// The place was retired: its restart budget ran out, so there will be no
    /// further occupant and a connect will not pend.
    pub const RETIRED: u64 = 4;

    /// Pack a cause and its detail into one word.
    ///
    /// The detail is the vector for a fault and the status for an exit, and it
    /// is in the high half so that a reader matching on the cause alone is
    /// matching on a small number rather than on a mask it might get wrong.
    #[must_use]
    pub const fn pack(cause: u64, detail: u64) -> u64 {
        (detail << 32) | (cause & 0xFFFF_FFFF)
    }

    /// Which cause a packed word names.
    #[must_use]
    pub const fn of(packed: u64) -> u64 {
        packed & 0xFFFF_FFFF
    }

    /// The detail beside it.
    #[must_use]
    pub const fn detail(packed: u64) -> u64 {
        packed >> 32
    }

    /// Is this a cause a component may be told?
    #[must_use]
    pub const fn known(cause: u64) -> bool {
        matches!(cause, FAULT | EXIT | STOPPED | RETIRED)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(cause: u64) -> &'static str {
        match cause {
            FAULT => "fault",
            EXIT => "exit",
            STOPPED => "stopped",
            RETIRED => "retired",
            _ => "unknown",
        }
    }
}

/// What the frame owes a component about one capability slot.
///
/// Five states, so three bits, and the states are the whole protocol. RFC 0008
/// gives them as a table and gives three rules that make every collision total
/// — rather than one rule and two cases somebody else decides later — and both
/// the table and the rules are below, in code, because a state machine written
/// only in prose is a state machine two implementations disagree about.
///
/// The wire values are stable: this is stored in a capability slot, and a slot
/// is memory a component paid for and the frame writes.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pending {
    /// Nothing owed.
    #[default]
    Quiet = 0,
    /// One *granted* notice.
    Granted = 1,
    /// One *revoked* notice.
    Revoked = 2,
    /// One *peer gone* notice.
    PeerGone = 3,
    /// Both, granted first.
    GrantedThenPeerGone = 4,
}

impl Pending {
    /// The wire value, for a slot to store.
    #[must_use]
    pub const fn to_wire(self) -> u8 {
        self as u8
    }

    /// The state a wire value names, or `None` for one this build does not
    /// define. Fail closed, R04: a slot whose field is a value from a build
    /// that knew a sixth state is not read as *quiet*.
    #[must_use]
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Quiet),
            1 => Some(Self::Granted),
            2 => Some(Self::Revoked),
            3 => Some(Self::PeerGone),
            4 => Some(Self::GrantedThenPeerGone),
            _ => None,
        }
    }

    /// Is nothing owed?
    ///
    /// The one question the placement path asks: **a slot whose field is not
    /// quiet is not refilled**. That rule is what keeps a handle's generation
    /// honest under a pending notice — a *revoked* notice always names a handle
    /// whose slot has not been reissued, so a component can match it against
    /// what it holds rather than against whatever arrived in the meantime.
    #[must_use]
    pub const fn is_quiet(self) -> bool {
        matches!(self, Self::Quiet)
    }

    /// A capability was placed here.
    ///
    /// Only legal on a quiet slot, and the caller is the one that checks: this
    /// returns the state unchanged for anything else, so a frame that placed
    /// into a slot it owed something on would find the notice it owed still
    /// pending rather than silently replaced.
    #[must_use]
    pub const fn granted(self) -> Self {
        match self {
            Self::Quiet => Self::Granted,
            other => other,
        }
    }

    /// The capability here was revoked.
    ///
    /// Rule 1: **an undelivered grant that is revoked posts nothing, and the
    /// slot goes quiet.** A component that was never told it held something
    /// never held it, and telling it that a thing it does not know about is
    /// gone is a notice it cannot act on.
    ///
    /// Rule 3: **revoked is terminal and supersedes peer gone**, which is why
    /// the two never coexist. Both say stop using it, and only *revoked* names
    /// the refusal (`AUTHORITY/REVOKED`) a later submission carrying the handle
    /// will earn.
    #[must_use]
    pub const fn revoked(self) -> Self {
        match self {
            // Rule 1, and it reaches the collision too: the grant in
            // `GrantedThenPeerGone` is equally undelivered, so the component
            // never held this either.
            Self::Granted | Self::GrantedThenPeerGone => Self::Quiet,
            _ => Self::Revoked,
        }
    }

    /// The far end of what is here ended.
    ///
    /// Rule 2: **peer death does not swallow the grant.** A channel handle
    /// granted and widowed before the drain posts *granted* and then *peer
    /// gone*, in that order, because the granted notice is the only place the
    /// need or ask index is ever stated — collapse it and a component that
    /// asked the powerbox for two things cannot tell which of them died.
    #[must_use]
    pub const fn peer_gone(self) -> Self {
        match self {
            Self::Granted | Self::GrantedThenPeerGone => Self::GrantedThenPeerGone,
            // A revoked slot holds nothing whose peer can die, so this
            // collision does not arise in the frame; it is written as a
            // no-change rather than as an assertion because a `Pending` is
            // memory and the frame does not assert about memory.
            Self::Revoked => Self::Revoked,
            _ => Self::PeerGone,
        }
    }

    /// The next notice this slot owes, and what it owes afterwards.
    ///
    /// `None` when it owes nothing. The order within a slot is the promise rule
    /// 2 makes: granted, then peer gone.
    #[must_use]
    pub const fn drain(self) -> (Option<i32>, Self) {
        match self {
            Self::Quiet => (None, Self::Quiet),
            Self::Granted => (Some(notice::GRANTED), Self::Quiet),
            Self::GrantedThenPeerGone => (Some(notice::GRANTED), Self::PeerGone),
            Self::PeerGone => (Some(notice::PEER_GONE), Self::Quiet),
            Self::Revoked => (Some(notice::REVOKED), Self::Quiet),
        }
    }

    /// How many notices this state still owes. For a state tree, and for a test
    /// that wants to say *and nothing was lost* rather than *and something was
    /// delivered*.
    #[must_use]
    pub const fn owed(self) -> u32 {
        match self {
            Self::Quiet => 0,
            Self::GrantedThenPeerGone => 2,
            _ => 1,
        }
    }
}

/// A deadline somebody must meet: a stop, or a reclaim of one core.
///
/// **A promise may only ever move earlier.** A stop that arrives while a stop is
/// already pending never moves the deadline later: the frame keeps the earlier
/// of the two. A promise that can be silently relaxed by whoever made it is the
/// thing R08 refuses to call a deadline, and a component that has already begun
/// quiescing against one must not have it withdrawn under it.
///
/// This is why a promise is not a [`Grade`]. The argument for latest-wins holds
/// for a value that is *true of the component now*; it does not hold for a
/// deadline, because the earlier one is still true.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Promise {
    /// The deadline promised, or [`crate::NO_DEADLINE`] for none.
    /// Unit: nanoseconds, monotonic, in the control channel's epoch — the same
    /// clock and epoch as [`crate::Sqe::deadline`], and RFC 0009 governs it.
    deadline: u64,
    /// Whether a notice is still owed for it.
    /// Unit: none — a flag. Distinct from `deadline` being set, because a
    /// promise stays in force after its notice has been published: the
    /// component has been told, and the deadline has not moved.
    owed: bool,
}

impl Promise {
    /// Nothing promised.
    pub const NONE: Self = Self { deadline: crate::NO_DEADLINE, owed: false };

    /// Promise this deadline, keeping the earlier of it and whatever stood.
    ///
    /// Answers whether the promise actually moved. A second stop that would
    /// have relaxed the first changes nothing and says so, which is what lets
    /// the frame complete that submission with *which deadline it kept* rather
    /// than with a bare success the submitter would misread.
    pub const fn promise(&mut self, deadline: u64) -> bool {
        if self.deadline != crate::NO_DEADLINE && deadline >= self.deadline {
            return false;
        }
        self.deadline = deadline;
        self.owed = true;
        true
    }

    /// The deadline in force, if any.
    #[must_use]
    pub const fn deadline(&self) -> Option<u64> {
        if self.deadline == crate::NO_DEADLINE { None } else { Some(self.deadline) }
    }

    /// Publish the notice, if one is owed, and answer the deadline it carries.
    ///
    /// The promise itself stays: a component that has been told to stop by a
    /// deadline is still under that deadline after it has drained the entry.
    pub const fn drain(&mut self) -> Option<u64> {
        if !self.owed {
            return None;
        }
        self.owed = false;
        Some(self.deadline)
    }

    /// Is a notice still owed?
    #[must_use]
    pub const fn is_owed(&self) -> bool {
        self.owed
    }
}

/// A value that is true of the component *now*: a pressure grade, a system
/// generation.
///
/// **Latest wins.** A grade that changes twice before the component drains once
/// is one notice carrying the second value, because the first was never true of
/// anything the component could still act on. Two words per component, and the
/// argument for collapsing them is exactly the argument that does *not* hold
/// for a core or a deadline.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Grade {
    /// The grade itself.
    /// Unit: per kind — a pressure grade for the pressure notice, a system
    /// generation ordinal for the generation notice. The notice kind is what
    /// says which, and a `Grade` does not carry its own kind because the two
    /// live in named fields rather than in a list.
    value: u64,
    /// Whether a notice is still owed for it.
    /// Unit: none — a flag.
    owed: bool,
}

impl Grade {
    /// No grade, and nothing owed. The state every component opens in.
    pub const NONE: Self = Self { value: 0, owed: false };

    /// This is the grade now.
    ///
    /// Answers whether it changed. A grade set to what it already was owes
    /// nothing: a notice that says a value did not move is a completion entry
    /// spent on nothing, and the ring's depth is what bounds how much is
    /// visible.
    pub const fn set(&mut self, value: u64) -> bool {
        if self.value == value && !self.owed {
            return false;
        }
        let changed = self.value != value;
        self.value = value;
        self.owed = true;
        changed
    }

    /// The grade in force.
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }

    /// Publish the notice, if one is owed.
    pub const fn drain(&mut self) -> Option<u64> {
        if !self.owed {
            return None;
        }
        self.owed = false;
        Some(self.value)
    }

    /// Is a notice still owed?
    #[must_use]
    pub const fn is_owed(&self) -> bool {
        self.owed
    }
}

/// Build the completion entry that carries one notice.
///
/// One constructor, so that the flag, the two readings of `user_data` and the
/// placement of the kind in `result` cannot be got right in one place and wrong
/// in another. `handle` is what the notice concerns — a capability handle for
/// the three that are about a slot, and the control ring's own handle for a
/// stop — and `ext` is whatever the kind carries.
#[must_use]
pub const fn entry(kind: i32, handle: u64, ext: u64, timestamp: u64) -> Cqe {
    Cqe { user_data: handle, result: kind, flags: cflags::NOTICE, timestamp, ext }
}

/// Is this completion a notice rather than an answer?
///
/// The one question a component asks of every entry it drains, and it is here
/// rather than written out at each drain site because there will be one such
/// site per component and they must all agree.
#[must_use]
pub const fn is_notice(cqe: &Cqe) -> bool {
    cqe.flags & cflags::NOTICE != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every state survives the wire, and nothing outside the five is read as
    /// one of them.
    #[test]
    fn the_five_states_are_closed() {
        let all = [
            Pending::Quiet,
            Pending::Granted,
            Pending::Revoked,
            Pending::PeerGone,
            Pending::GrantedThenPeerGone,
        ];
        for state in all {
            assert_eq!(Pending::from_wire(state.to_wire()), Some(state));
        }
        for value in 5u8..=u8::MAX {
            assert_eq!(Pending::from_wire(value), None, "{value} was read as a state");
        }
        // Three bits, which is what the slot has room for.
        assert!(all.iter().all(|s| s.to_wire() < 8));
    }

    /// RFC 0008 rule 1. A component that was never told it held something never
    /// held it.
    #[test]
    fn an_undelivered_grant_that_is_revoked_posts_nothing() {
        let state = Pending::Quiet.granted().revoked();
        assert_eq!(state, Pending::Quiet);
        assert_eq!(state.drain().0, None);
        assert_eq!(state.owed(), 0);

        // And it reaches the collision: the grant inside `GrantedThenPeerGone`
        // is equally undelivered.
        let state = Pending::Quiet.granted().peer_gone().revoked();
        assert_eq!(state, Pending::Quiet);
    }

    /// RFC 0008 rule 2. The granted notice is the only place the need or ask
    /// index is ever stated, so collapsing it loses which of two things died.
    #[test]
    fn peer_death_does_not_swallow_the_grant() {
        let state = Pending::Quiet.granted().peer_gone();
        assert_eq!(state, Pending::GrantedThenPeerGone);
        assert_eq!(state.owed(), 2);

        let (first, state) = state.drain();
        assert_eq!(first, Some(notice::GRANTED));
        let (second, state) = state.drain();
        assert_eq!(second, Some(notice::PEER_GONE), "the order within a slot is promised");
        assert_eq!(state, Pending::Quiet);
        assert_eq!(state.drain().0, None);
    }

    /// RFC 0008 rule 3. Both say stop using it, and only *revoked* names the
    /// refusal a later submission carrying the handle will earn.
    #[test]
    fn revoked_is_terminal_and_supersedes_peer_gone() {
        let state = Pending::Quiet.peer_gone().revoked();
        assert_eq!(state, Pending::Revoked);
        assert_eq!(state.drain().0, Some(notice::REVOKED));

        // The reverse does not arise in the frame — a revoked slot holds
        // nothing whose peer can die — and if it is reached anyway it does not
        // downgrade the refusal the component will earn.
        let state = Pending::Quiet.peer_gone().revoked().peer_gone();
        assert_eq!(state, Pending::Revoked);
    }

    /// The rule that keeps a generation honest under a pending notice.
    #[test]
    fn a_slot_that_is_not_quiet_is_not_refilled() {
        for state in [Pending::Granted, Pending::Revoked, Pending::PeerGone] {
            assert!(!state.is_quiet());
            assert_eq!(state.granted(), state, "a placement into {state:?} changed it");
        }
        assert!(Pending::Quiet.is_quiet());
        assert_eq!(Pending::Quiet.granted(), Pending::Granted);
    }

    /// Draining is total: every state reaches quiet in as many steps as it
    /// owes, and no state loops.
    #[test]
    fn every_state_drains_to_quiet_in_the_steps_it_owes() {
        for start in [
            Pending::Quiet,
            Pending::Granted,
            Pending::Revoked,
            Pending::PeerGone,
            Pending::GrantedThenPeerGone,
        ] {
            let mut state = start;
            let mut steps = 0;
            while let (Some(kind), next) = state.drain() {
                assert!(notice::known(kind), "a state produced a kind nobody defines");
                state = next;
                steps += 1;
                assert!(steps <= 2, "{start:?} owes more than two notices");
            }
            assert_eq!(state, Pending::Quiet);
            assert_eq!(steps, start.owed(), "{start:?} delivered a different count than it owed");
        }
    }

    /// R08's sentence, as a test: a promise nothing can relax.
    #[test]
    fn a_promise_only_ever_moves_earlier() {
        let mut promise = Promise::NONE;
        assert_eq!(promise.deadline(), None);

        assert!(promise.promise(1_000));
        assert_eq!(promise.deadline(), Some(1_000));

        // Later is refused and says so, which is what lets the frame complete
        // the second stop with *which deadline it kept*.
        assert!(!promise.promise(2_000));
        assert_eq!(promise.deadline(), Some(1_000));
        // Equal is not earlier.
        assert!(!promise.promise(1_000));

        assert!(promise.promise(500));
        assert_eq!(promise.deadline(), Some(500));
    }

    /// A promise stays in force after its notice is published; a component told
    /// to stop is still under the deadline it drained.
    #[test]
    fn draining_a_promise_publishes_it_and_does_not_lift_it() {
        let mut promise = Promise::NONE;
        promise.promise(1_000);
        assert!(promise.is_owed());
        assert_eq!(promise.drain(), Some(1_000));
        assert_eq!(promise.drain(), None, "one promise, two notices");
        assert_eq!(promise.deadline(), Some(1_000), "the deadline was lifted by being told");
        assert!(!promise.is_owed());
    }

    /// The argument that holds for a grade and not for a deadline.
    #[test]
    fn a_grade_is_latest_wins() {
        let mut grade = Grade::NONE;
        assert!(grade.set(3));
        // Twice before a drain is one notice, carrying the second.
        assert!(grade.set(5));
        assert_eq!(grade.drain(), Some(5));
        assert_eq!(grade.drain(), None);
        assert_eq!(grade.value(), 5);

        // A grade set to what it already is owes nothing new.
        assert!(!grade.set(5));
        assert!(!grade.is_owed());
    }

    /// The flag is the whole ABI change, and both readings of `user_data` are
    /// one constructor's business.
    #[test]
    fn a_notice_is_a_completion_that_answers_no_submission() {
        let cqe = entry(notice::PEER_GONE, 0x0001_0007, cause::pack(cause::FAULT, 14), 42);
        assert!(is_notice(&cqe));
        assert_eq!(cqe.result, notice::PEER_GONE);
        assert_eq!(cqe.user_data, 0x0001_0007, "user_data is the handle on a notice");
        assert_eq!(cause::of(cqe.ext), cause::FAULT);
        assert_eq!(cause::detail(cqe.ext), 14);
        assert_eq!(cqe.timestamp, 42);

        // An ordinary completion is not one, which is the half that makes the
        // flag worth having.
        assert!(!is_notice(&Cqe::ZERO));
    }

    /// R04, on the two closed sets this module owns.
    #[test]
    fn an_unknown_opcode_or_kind_is_refused_rather_than_ignored() {
        for opcode in [0u8, 1, 0x0F, 0x18, 0xFE, 0xFF] {
            assert!(!op::known(opcode), "{opcode:#04x} is not one of the eight");
            assert_eq!(op::label(opcode), "unknown");
        }
        for opcode in [op::INSPECT, op::DERIVE, op::REVOKE, op::MAP] {
            assert!(op::known(opcode));
        }
        for opcode in [op::SPAWN, op::CONNECT, op::STOP, op::GRANT] {
            assert!(op::known(opcode));
        }
        for kind in [0i32, 8, -1, i32::MAX] {
            assert!(!notice::known(kind));
        }
        for cause in [0u64, 5, u64::MAX] {
            assert!(!cause::known(cause));
        }
    }

    /// The publication order is fixed and is stated once.
    #[test]
    fn the_publication_order_is_the_one_rfc_0008_fixes() {
        assert_eq!(ORDER, ["slots ascending", "stop", "reclaim by core ascending", "grades"]);
    }
}
