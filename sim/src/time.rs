// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Virtual time: a clock the model owns, and the queue that moves it.
//!
//! # Nothing here reads a host clock
//!
//! Time advances because the model advances it. [`Timeline::next`] takes the
//! earliest instant anything is due at and moves the clock to it; there is no
//! other way for the clock to move and no way at all to move it backwards,
//! because the only way to put work into the queue is [`Timeline::send`], which
//! adds a delay to the clock's current value. `cargo xtask lint-determinism`
//! passes over this crate with no allow-list entry, and the reason is
//! structural rather than careful.
//!
//! # Order within a channel is the ring's; order across channels is the seed's
//!
//! Several messages can be due at one instant, and choosing between them is the
//! interleaving decision this whole crate exists to make reproducible. What it
//! must not do is choose *freely*, because that would model something the
//! system forbids: `f_ring` is single-producer, single-consumer, and a ring
//! delivers one producer's entries to its consumer in the order they were
//! published. A simulator that reordered two submissions from one client would
//! find bugs that cannot happen and miss the ones that can.
//!
//! So the due set is grouped by **channel** — the ordered pair of sender and
//! recipient — and the seed chooses which channel goes next, never which entry
//! within one. Within a channel it is first in, first out. That is exactly the
//! guarantee the ring gives and exactly the freedom the hardware has, and
//! writing it as the queue's shape means nothing above has to remember it.
//!
//! The decision is recorded at the site [`CHANNEL`], so a failure that depends
//! on cross-channel ordering names the site it depends on.

use crate::decide::Decisions;
use crate::{ActorId, Message};

use std::collections::BTreeMap;

/// The site the cross-channel ordering decision is recorded at.
///
/// A constant rather than a literal at the call site, because `E1-P03` reports
/// this string to a person and `E1-P02` may focus a sweep on it.
pub const CHANNEL: &str = "timeline.channel";

/// One message, waiting for its instant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pending {
    /// Who it is for. Unit: none — an actor index in the simulation that
    /// scheduled it.
    pub to: ActorId,
    /// What it says. Unit: none — see [`Message`].
    pub message: Message,
}

/// The clock, and everything waiting on it.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    now: u64,
    /// Instant to the messages due at it, in arrival order within each instant.
    /// A `BTreeMap` because the next instant is its first key, and because
    /// RFC 0004 forbids the hash map that would otherwise be reached for.
    due: BTreeMap<u64, Vec<Pending>>,
    scheduled: u64,
}

impl Timeline {
    /// A clock at zero with nothing due.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The virtual clock. Unit: nanoseconds since the start of the run, which
    /// is this clock's zero and its epoch. It is comparable with nothing
    /// outside this run.
    #[must_use]
    pub fn clock(&self) -> u64 {
        self.now
    }

    /// How many messages this run has scheduled. Unit: messages.
    #[must_use]
    pub fn scheduled(&self) -> u64 {
        self.scheduled
    }

    /// Is there anything left to do?
    #[must_use]
    pub fn idle(&self) -> bool {
        self.due.is_empty()
    }

    /// Put a message in the queue, `delay_ns` from now.
    ///
    /// There is no way to schedule at an absolute instant, and that absence is
    /// the mechanism rather than a missing convenience: an absolute instant can
    /// be in the past, a delay cannot, and a clock that can be sent backwards is
    /// the one failure this crate must not have. A delay of zero is the same
    /// instant, which is legitimate and is where the interleaving decisions come
    /// from. R01 — name the mechanism, not the intention.
    ///
    /// `delay_ns` is nanoseconds. Saturating, so a model that computes an
    /// absurd delay produces a message at the end of time rather than one in the
    /// past.
    pub fn send(&mut self, delay_ns: u64, to: ActorId, message: Message) {
        let at = self.now.saturating_add(delay_ns);
        self.due.entry(at).or_default().push(Pending { to, message });
        self.scheduled = self.scheduled.saturating_add(1);
    }

    /// When the next message is due, without taking it. Unit: nanoseconds.
    ///
    /// `None` when nothing is due, which is the same answer [`Timeline::idle`]
    /// gives in a different shape. It exists because `E1-P08` places a cut *in
    /// simulated time* and has to decide whether to stop before a step without
    /// taking that step — a decision made after the message was taken would be a
    /// cut in the middle of a step, and there is no such place.
    #[must_use]
    pub fn peek(&self) -> Option<u64> {
        self.due.keys().next().copied()
    }

    /// Write this timeline out.
    ///
    /// Instants outermost and, inside each, the messages in arrival order —
    /// which is the order [`Timeline::next`] builds its channel list in, so the
    /// *set of alternatives* a restored run offers the seed is the set the
    /// original offered. A snapshot that wrote the queue in any other order
    /// would restore a world where one channel had quietly moved ahead of
    /// another, and the divergence would begin at the next tie.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) {
        out.u64(self.now);
        out.u64(self.scheduled);
        out.count(self.due.len());
        for (at, queue) in &self.due {
            out.u64(*at);
            out.count(queue.len());
            for pending in queue {
                crate::snap::write_message(out, pending.to, &pending.message);
            }
        }
    }

    /// Read one back.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        let now = input.u64();
        let scheduled = input.u64();
        let instants = input.count(12, "more instants than the file could hold");
        let mut due: BTreeMap<u64, Vec<Pending>> = BTreeMap::new();
        for _ in 0..instants {
            let at = input.u64();
            let count = input.count(28, "more messages at an instant than the file could hold");
            let mut queue = Vec::with_capacity(count);
            for _ in 0..count {
                let (to, message) = crate::snap::read_message(input);
                queue.push(Pending { to, message });
            }
            due.insert(at, queue);
        }
        Self { now, due, scheduled }
    }

    /// Take the next message, moving the clock to its instant.
    ///
    /// Returns `None` when nothing is due, which is how a run ends.
    pub fn next(&mut self, decisions: &mut Decisions) -> Option<Pending> {
        let at = *self.due.keys().next()?;
        self.now = at;

        let queue = self.due.get_mut(&at)?;

        // The distinct channels with work due at this instant, in the order
        // their first message arrived. Building this list in arrival order is
        // what makes the *set* of alternatives a function of the model rather
        // than of the map's internals, which matters because the seed indexes
        // into it.
        let mut channels: Vec<(ActorId, ActorId)> = Vec::new();
        for pending in queue.iter() {
            let channel = (pending.message.from, pending.to);
            if !channels.contains(&channel) {
                channels.push(channel);
            }
        }

        let arity = u32::try_from(channels.len()).unwrap_or(u32::MAX);
        let taken = decisions.decide(at, CHANNEL, arity) as usize;
        let chosen = *channels.get(taken)?;

        // The first message on the chosen channel, never a later one: this is
        // where per-channel order stops being a convention and becomes the
        // queue's shape.
        let index = queue.iter().position(|p| (p.message.from, p.to) == chosen)?;
        let pending = queue.remove(index);
        if queue.is_empty() {
            self.due.remove(&at);
        }
        Some(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(from: u32, kind: &'static str) -> Message {
        Message { from: ActorId(from), kind, token: 0, detail: 0 }
    }

    #[test]
    fn the_clock_starts_at_zero_and_moves_only_when_something_is_due() {
        let mut line = Timeline::new();
        let mut decisions = Decisions::new(1);
        assert_eq!(line.clock(), 0);
        assert!(line.idle());

        line.send(500, ActorId(0), message(0, "a"));
        assert_eq!(line.clock(), 0, "scheduling moved the clock");
        assert!(line.next(&mut decisions).is_some());
        assert_eq!(line.clock(), 500);
    }

    #[test]
    fn the_clock_never_goes_backwards() {
        let mut line = Timeline::new();
        let mut decisions = Decisions::new(0xABC);
        // Deliberately scheduled out of order, and at zero delay, which is the
        // case an absolute-instant interface would get wrong.
        for delay in [900u64, 100, 500, 0, 700, 0, 300] {
            line.send(delay, ActorId(0), message(1, "a"));
        }
        let mut last = 0;
        while let Some(_pending) = line.next(&mut decisions) {
            assert!(line.clock() >= last, "the clock went from {last} to {}", line.clock());
            last = line.clock();
        }
    }

    #[test]
    fn one_channels_messages_keep_their_order() {
        // The ring's guarantee, as the queue's shape. Three messages from one
        // sender to one recipient at one instant come out in the order they went
        // in, whatever the seed says.
        for seed in [1u64, 2, 3, 0xFFFF_FFFF_FFFF_FFFF] {
            let mut line = Timeline::new();
            let mut decisions = Decisions::new(seed);
            for token in 0..3u64 {
                line.send(
                    0,
                    ActorId(9),
                    Message { from: ActorId(1), kind: "submit", token, detail: 0 },
                );
            }
            let mut seen = Vec::new();
            while let Some(pending) = line.next(&mut decisions) {
                seen.push(pending.message.token);
            }
            assert_eq!(seen, vec![0, 1, 2], "seed {seed} reordered one channel");
        }
    }

    #[test]
    fn two_channels_are_ordered_by_the_seed() {
        // The other half: across channels the seed decides, and two seeds must
        // be able to decide differently or the simulator explores one order and
        // reports that it explored both.
        let order = |seed| {
            let mut line = Timeline::new();
            let mut decisions = Decisions::new(seed);
            for from in 0..4u32 {
                line.send(0, ActorId(9), message(from, "submit"));
            }
            let mut seen = Vec::new();
            while let Some(pending) = line.next(&mut decisions) {
                seen.push(pending.message.from.0);
            }
            seen
        };
        let a = order(1);
        assert_eq!(a.len(), 4);
        assert_eq!(a, order(1), "one seed gave two orders");
        let mut differs = false;
        for seed in 2..64u64 {
            if order(seed) != a {
                differs = true;
                break;
            }
        }
        assert!(differs, "no seed in sixty-three reordered four channels");
    }

    #[test]
    fn a_lone_channel_costs_no_decision() {
        // A decision with one alternative is not a decision, and recording one
        // would make every ordinal downstream depend on how busy the instant
        // happened to be. `decide.rs` argues it; this checks the caller does not
        // defeat it by asking anyway.
        let mut line = Timeline::new();
        let mut decisions = Decisions::new(4);
        line.send(10, ActorId(0), message(1, "a"));
        line.send(20, ActorId(0), message(1, "b"));
        while line.next(&mut decisions).is_some() {}
        assert_eq!(decisions.taken(), 0);
    }
}
