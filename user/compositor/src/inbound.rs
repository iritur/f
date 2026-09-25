// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The other end of the input driver's data channel, held by this component
//! and not by the frame.
//!
//! # What this file is, and what it replaces
//!
//! Until `E3-B04g` the frame held this end. It drained what `user/virtio-input`
//! submitted, decoded every entry, kept two coordinates and threw the rest away,
//! and then handed this component scene deltas built out of those coordinates —
//! so the compositor never saw an input entry, never saw a reading, and the late
//! latch had nothing to predict from. That was not a decision anybody argued
//! for; it was the one arrangement a machine with one worker core and no ring-3
//! supervisor that hands a component a peer could build. RFC 0124's *What the
//! consumer has to do* wrote down the four steps that move the end here, and
//! this file is those four steps:
//!
//! 1. **Hold the other end.** `crate::component` adopts the channel from two
//!    words on its own routing page — `crate::routing::at::INPUT_AT` and
//!    `INPUT_LEN` — as a server, and drains it every turn. Nothing here
//!    touches the ring; this file is what is done with an entry once it has
//!    been taken, which is what makes it testable on a host.
//! 2. **Decode with `f_abi::input::Event::decode`.** The frame's decoder,
//!    called here, and not a second one. It refuses an unstamped payload before
//!    it reads the body, and compares the whole envelope against the entry this
//!    build would have written.
//! 3. **Rebuild the reading from the field the entry carries.** Not in this file:
//!    [`crate::latch::LateLatch::drained`] is where `from_wire_nanos` is called,
//!    with the decoded event's field passed straight in, because that is the
//!    module `xtask`'s `lint-stamp` already watches the one other mint in.
//! 4. **Fold what was drained, and check it.** Every decoded event goes into a
//!    [`Crossing`]; the driver's own word arrives on the same ring, after its
//!    last event, under `f_abi::input::ATTEST`, and [`Inbound::agrees`] is the
//!    comparison.
//!
//! # What must not happen here
//!
//! A clock read when an entry arrives. There is no clock in this component to
//! read — RFC 0004 gives ring 3 neither a timer nor a port — and there is
//! nothing in this file that could subtract one number from another. The
//! arrival order is the ring's and the time is the driver's.
//!
//! # Why the attestation is checked here *and* by the frame
//!
//! Because the word reaches the two checks by two routes. This component reads
//! it off the ring; the frame reads it off the driver's routing page before it
//! reaps the driver, and compares this component's published fold against that.
//! A ring that carried the events faithfully and the attestation wrongly would
//! pass one and fail the other, and neither check computes the word it compares.

use f_abi::Sqe;
use f_abi::input::{Attested, Crossing, Entry, Event, PAYLOAD_BYTES};

/// Everything this component has taken off the input ring.
///
/// Counters and two words, with no ring in it: the ring is the component's, and
/// what is here is the arithmetic a host test can drive with entries it built
/// itself.
#[derive(Clone, Copy, Debug)]
pub struct Inbound {
    /// Whether the frame gave this component an input ring at all.
    ///
    /// The control's whole difference: `input=withheld` is the identical boot
    /// with the channel not connected, and a component that cannot say which
    /// run it was in would publish the same zeroes for *nothing arrived* and
    /// *nothing was connected*.
    connected: bool,
    /// Entries taken that were not an attestation. Unit: entries.
    entries: u64,
    /// Of those, entries the decoder took. Unit: entries.
    decoded: u64,
    /// Of those, entries the decoder refused. Unit: entries.
    refused: u64,
    /// Of the decoded ones, pointer motion. Unit: entries.
    motions: u64,
    /// What arrived, folded in the order it arrived.
    crossing: Crossing,
    /// The driver's word, as it arrived on the ring.
    attested: Option<Attested>,
    /// Attestations taken. One, or the check is not a check. Unit: entries.
    attestations: u64,
    /// Entries taken after an attestation.
    ///
    /// Not covered by the word that preceded them, so an agreement reached with
    /// any of these present would be an agreement about less than arrived.
    /// Unit: entries.
    unattested: u64,
}

impl Inbound {
    /// A component that was given no input ring.
    pub const UNCONNECTED: Self = Self {
        connected: false,
        entries: 0,
        decoded: 0,
        refused: 0,
        motions: 0,
        crossing: Crossing::new(),
        attested: None,
        attestations: 0,
        unattested: 0,
    };

    /// A component that was given one and has taken nothing off it yet.
    ///
    /// Written field by field rather than as an update of [`Self::UNCONNECTED`],
    /// for `crate::latch::LateLatch::riding`'s reason: a field added above and
    /// forgotten here is a build failure that names it.
    #[must_use]
    pub const fn connected() -> Self {
        Self {
            connected: true,
            entries: 0,
            decoded: 0,
            refused: 0,
            motions: 0,
            crossing: Crossing::new(),
            attested: None,
            attestations: 0,
            unattested: 0,
        }
    }

    /// One entry off the ring, with the payload the arena held for it.
    ///
    /// Answers the event when there was one, so that the caller can hand a
    /// position to the latch. An attestation answers `None` and is recorded; an
    /// entry the decoder refused answers `None` and is counted. The payload of
    /// an attestation is not read — it has none, and `ATTEST` is outside the
    /// decoder's opcode space so that it cannot be folded into the crossing it
    /// attests to.
    pub fn take(&mut self, entry: &Sqe, payload: &[u8; PAYLOAD_BYTES]) -> Option<Event> {
        if let Some(said) = Crossing::attested(entry) {
            self.attestations = self.attestations.saturating_add(1);
            // The first one is kept. A second is a producer saying two things
            // about one crossing, and [`Inbound::agrees`] refuses on the count
            // rather than choosing which to believe.
            if self.attested.is_none() {
                self.attested = Some(said);
            }
            return None;
        }
        self.entries = self.entries.saturating_add(1);
        if self.attested.is_some() {
            self.unattested = self.unattested.saturating_add(1);
        }
        let Ok(event) = Event::decode(entry, payload) else {
            self.refused = self.refused.saturating_add(1);
            return None;
        };
        self.decoded = self.decoded.saturating_add(1);
        // In the order the entries came off the ring, and from the decoded event
        // rather than the arena's bytes: what is attested is that the *event*
        // crossed unchanged, which is the frame's rule when it held this end and
        // is kept rather than re-decided.
        self.crossing.absorb(&event);
        if matches!(event.body, Entry::PointerMotion(_)) {
            self.motions = self.motions.saturating_add(1);
        }
        Some(event)
    }

    /// Does what arrived match what the driver says it sent?
    ///
    /// Four things at once, and each is a different failure: exactly one
    /// attestation, nothing after it, the word, and the count. A fold of nothing
    /// agrees with nobody — `Crossing::agrees_with`'s own guard — so a run in
    /// which no event crossed answers `false` here rather than passing quietly.
    #[must_use]
    pub fn agrees(&self) -> bool {
        self.attestations == 1
            && self.unattested == 0
            && self.attested.is_some_and(|said| self.crossing.agrees_with_attested(said))
    }

    /// Whether this component was given an input ring.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// Entries taken that were not an attestation. Unit: entries.
    #[must_use]
    pub const fn entries(&self) -> u64 {
        self.entries
    }

    /// Entries the decoder took. Unit: entries.
    #[must_use]
    pub const fn decoded(&self) -> u64 {
        self.decoded
    }

    /// Entries the decoder refused. Unit: entries.
    #[must_use]
    pub const fn refused(&self) -> u64 {
        self.refused
    }

    /// Decoded entries that were pointer motion. Unit: entries.
    #[must_use]
    pub const fn motions(&self) -> u64 {
        self.motions
    }

    /// What arrived, folded. Unit: none — an `f_abi::input::Crossing`.
    #[must_use]
    pub const fn crossing(&self) -> Crossing {
        self.crossing
    }

    /// The driver's word as it arrived, or `None` when none did.
    #[must_use]
    pub const fn attested(&self) -> Option<Attested> {
        self.attested
    }

    /// Attestations taken. Unit: entries.
    #[must_use]
    pub const fn attestations(&self) -> u64 {
        self.attestations
    }

    /// Entries taken after the attestation. Unit: entries.
    #[must_use]
    pub const fn unattested(&self) -> u64 {
        self.unattested
    }
}

#[cfg(test)]
mod tests {
    use f_abi::input::{Key, PointerMotion, edge};
    use f_abi::{class, deadline, flags};

    use super::*;

    /// A stamp far from zero, so that no test here is about `NOT_STAMPED` by
    /// accident. Unit: nanoseconds.
    const FIRST_AT_NANOS: u64 = 100_000;

    /// The driver's tick in `cargo xtask input deliver`. Unit: nanoseconds.
    const TICK_NANOS: u64 = 100_000;

    /// What a driver submits: one event per position, stamped once each, with
    /// a key in the middle so that a fold watching motion alone would be caught.
    fn sent() -> [Event; 4] {
        let body = |at: usize| match at {
            2 => Entry::Key(Key { code: 30, transition: edge::PRESSED }),
            _ => Entry::PointerMotion(PointerMotion {
                x_x65536: 65_536 * (at as i32 + 1),
                y_x65536: -32_768 * (at as i32),
            }),
        };
        core::array::from_fn(|at| Event {
            user_data: 0,
            class: deadline::pack(class::SOFT, 0),
            payload_offset: (at * PAYLOAD_BYTES) as u32,
            flags: flags::NO_CQE,
            stamp_nanos: FIRST_AT_NANOS + TICK_NANOS * at as u64,
            body: body(at),
        })
    }

    /// The driver's side: its own fold over what it sent, as the attestation
    /// it puts on the ring.
    fn attestation(events: &[Event]) -> Sqe {
        let mut fold = Crossing::new();
        for event in events {
            fold.absorb(event);
        }
        fold.attestation(deadline::pack(class::SOFT, 0))
    }

    /// Everything off the ring into a fresh consumer, in the order given.
    fn drained(arrived: &[(Sqe, [u8; PAYLOAD_BYTES])]) -> (Inbound, u64) {
        let mut inbound = Inbound::connected();
        let mut handed = 0;
        for (entry, payload) in arrived {
            if inbound.take(entry, payload).is_some() {
                handed += 1;
            }
        }
        (inbound, handed)
    }

    fn wire(events: &[Event]) -> [(Sqe, [u8; PAYLOAD_BYTES]); 5] {
        let mut out = [(attestation(events), [0u8; PAYLOAD_BYTES]); 5];
        for (slot, event) in out.iter_mut().zip(events) {
            *slot = event.encode();
        }
        out
    }

    /// **`E3-B04g`'s fourth step, on a host.** Four events and the driver's
    /// word arrive; every event is decoded and handed on, the attestation is
    /// not, and the two words agree.
    #[test]
    fn what_the_driver_sent_is_what_this_component_drained() {
        let events = sent();
        let (inbound, handed) = drained(&wire(&events));
        assert_eq!(handed, 4);
        assert_eq!(inbound.entries(), 4);
        assert_eq!(inbound.decoded(), 4);
        assert_eq!(inbound.refused(), 0);
        assert_eq!(inbound.motions(), 3, "the key is an event and not a position");
        assert_eq!(inbound.attestations(), 1);
        assert!(inbound.agrees());
        assert_eq!(inbound.crossing().absorbed(), 4);
    }

    /// A relay between the two that minted a reading on arrival — the one
    /// defect this path exists not to have — decodes cleanly, keeps every
    /// count, and does not agree.
    #[test]
    fn a_reading_minted_between_the_driver_and_here_does_not_agree() {
        let events = sent();
        let mut restamped = events;
        for event in &mut restamped {
            event.stamp_nanos += 1;
        }
        let mut arrived = wire(&restamped);
        // The driver's word is over what it sent, not over what arrived.
        arrived[4] = (attestation(&events), [0; PAYLOAD_BYTES]);
        let (inbound, handed) = drained(&arrived);
        assert_eq!(handed, 4, "every count agrees, which is the point");
        assert_eq!(inbound.refused(), 0);
        assert!(!inbound.agrees());
    }

    /// One dropped out of the middle, and two handed on out of order.
    #[test]
    fn a_dropped_or_reordered_entry_does_not_agree() {
        let events = sent();
        let whole = wire(&events);

        let dropped = [whole[0], whole[1], whole[3], whole[4]];
        let (inbound, _) = drained(&dropped);
        assert!(!inbound.agrees());

        let reordered = [whole[1], whole[0], whole[2], whole[3], whole[4]];
        let (inbound, handed) = drained(&reordered);
        assert_eq!(handed, 4);
        assert!(!inbound.agrees());
    }

    /// No attestation, two, or an event after one: each is a run this
    /// component cannot vouch for, and none of them is resolved by choosing.
    #[test]
    fn an_attestation_that_is_missing_repeated_or_not_last_does_not_agree() {
        let events = sent();
        let whole = wire(&events);

        let (inbound, _) = drained(&whole[..4]);
        assert_eq!(inbound.attested(), None);
        assert!(!inbound.agrees());

        let (inbound, _) = drained(&[whole[0], whole[1], whole[2], whole[3], whole[4], whole[4]]);
        assert_eq!(inbound.attestations(), 2);
        assert!(!inbound.agrees());

        let (inbound, _) = drained(&[whole[0], whole[1], whole[2], whole[4], whole[3]]);
        assert_eq!(inbound.unattested(), 1);
        assert!(!inbound.agrees());
    }

    /// An entry with no reading on it is refused by the decoder here, counted,
    /// never handed to the latch — and the words then disagree, because the
    /// driver folded an event this component could not.
    #[test]
    fn an_unstamped_entry_is_refused_and_counted_rather_than_handed_on() {
        let events = sent();
        let mut arrived = wire(&events);
        arrived[1].1[..8].copy_from_slice(&f_abi::input::NOT_STAMPED.to_le_bytes());
        let (inbound, handed) = drained(&arrived);
        assert_eq!(handed, 3);
        assert_eq!(inbound.refused(), 1);
        assert!(!inbound.agrees());
    }

    /// A component given no ring drained nothing and says so in a way that is
    /// not the same as *connected and nothing arrived*.
    #[test]
    fn an_unconnected_component_is_not_one_that_drained_nothing() {
        let unconnected = Inbound::UNCONNECTED;
        let idle = Inbound::connected();
        assert!(!unconnected.is_connected());
        assert!(idle.is_connected());
        assert!(!unconnected.agrees());
        assert!(!idle.agrees(), "a fold of nothing agrees with nobody");
    }
}
