// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The wire between two actors: real ABI entries, in the order one producer
//! wrote them.
//!
//! # Why the entries are not carried in a [`Message`](crate::Message)
//!
//! A `Message` is deliberately two `u64`s and a label, and the reason stage one
//! gives is worth keeping: it is the *occurrence* of a submission, and giving it
//! the wire type would mean the simulator's ordering machinery grew an opinion
//! about the ABI. An [`Sqe`] is sixty-four bytes. Widening every message in the
//! simulator so that the two channels which need one can carry it would make
//! every actor pay for the two, and it would put `f_abi` in the middle of
//! `time.rs`.
//!
//! So the entry goes on a wire the world holds, and the message says *there is
//! one for you*. That is not a workaround: it is what a ring **is** — memory two
//! components share, plus a doorbell that says something changed. The doorbell
//! is the message, the shared memory is here, and the two are separate for the
//! same reason they are separate in `f_ring`.
//!
//! # Order on the wire is the ring's order, and it is not the seed's
//!
//! One queue per **channel** — the ordered pair of sender and recipient — and
//! it is first in, first out. `f_ring` is single-producer, single-consumer and
//! delivers one producer's entries to its consumer in the order they were
//! published; a wire that let the seed reorder them would model something the
//! system forbids, and would find bugs that cannot happen while missing the ones
//! that can. `time.rs` groups the timeline by exactly the same pair for exactly
//! this reason, so the wire and the timeline agree by construction rather than
//! by two authors remembering the same rule.
//!
//! # What this is not
//!
//! It is not [`f_ring::Producer`] over a real [`Mapping`](f_ring::Mapping).
//! Standing up the real cursors would need one allocation two actors both hold,
//! which in safe Rust is an `Rc<RefCell<_>>` — a second shared-memory model
//! beside this one — and it would buy fidelity this stage cannot spend: the
//! hostile cursor values and the torn publish are `E1-P04`'s and `E1-P02`'s, and
//! both want the real header rather than a queue of entries.
//!
//! What is real here is everything the entries are made of and everything that
//! reads them: [`Sqe`], [`Cqe`], `f_ring::registry::Table`, `f_ring::buffers`.
//! `f_ring::buffers::Submitter` exists to be stood in for — its own
//! documentation says a test can put a recorder behind it and exercise the
//! ownership rules with no ring at all — and [`Post`] is that recorder with a
//! timeline behind it.
//!
//! RFC 0034 argues the whole arrangement — real entries, real registration, real
//! ownership types, a modelled transport — and this module is its transport
//! half.
//!
//! *Reversal:* `E1-P04` needs a peer that writes arbitrary cursor values, which
//! is a claim about the header and not about the entries. When that lands, this
//! becomes a `Mapping` two actors share and [`Post`] becomes a `Producer`;
//! nothing above it changes, because nothing above it names anything but
//! [`Sqe`] and [`Cqe`].

use std::collections::{BTreeMap, VecDeque};

use f_abi::{Cqe, Sqe};
use f_ring::RingError;
use f_ring::buffers::Submitter;

use crate::{ActorId, World};

/// A channel: who wrote, and who reads. The same pair `time.rs` groups by.
type Channel = (ActorId, ActorId);

/// Every entry in flight between actors, by channel.
///
/// `BTreeMap` because RFC 0004 forbids the hash map that would otherwise be
/// reached for, and `VecDeque` because a ring is a queue and this is the queue.
#[derive(Clone, Debug, Default)]
pub struct Wire {
    submissions: BTreeMap<Channel, VecDeque<Sqe>>,
    completions: BTreeMap<Channel, VecDeque<Cqe>>,
}

impl Wire {
    /// An empty wire.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a submission on the channel from `from` to `to`.
    pub fn post(&mut self, from: ActorId, to: ActorId, entry: Sqe) {
        self.submissions.entry((from, to)).or_default().push_back(entry);
    }

    /// Take the oldest submission on that channel.
    pub fn take(&mut self, from: ActorId, to: ActorId) -> Option<Sqe> {
        self.submissions.get_mut(&(from, to))?.pop_front()
    }

    /// How many submissions are waiting on that channel. Unit: entries.
    #[must_use]
    pub fn queued(&self, from: ActorId, to: ActorId) -> u32 {
        let len = self.submissions.get(&(from, to)).map_or(0, VecDeque::len);
        u32::try_from(len).unwrap_or(u32::MAX)
    }

    /// Put a completion on the channel from `from` to `to`.
    pub fn answer(&mut self, from: ActorId, to: ActorId, entry: Cqe) {
        self.completions.entry((from, to)).or_default().push_back(entry);
    }

    /// Take the oldest completion on that channel.
    pub fn reap(&mut self, from: ActorId, to: ActorId) -> Option<Cqe> {
        self.completions.get_mut(&(from, to))?.pop_front()
    }

    /// Write every entry in flight out.
    ///
    /// Per channel and, within a channel, in the order the producer published —
    /// which is the ring's guarantee and therefore the one thing about this
    /// structure a snapshot must not get wrong. A restore that reordered one
    /// producer's entries would put the model in a state the real ring cannot
    /// reach, and a bug found from there would be a bug in the snapshot.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) {
        out.count(self.submissions.len());
        for ((from, to), queue) in &self.submissions {
            out.u32(from.0);
            out.u32(to.0);
            out.count(queue.len());
            for entry in queue {
                out.sqe(entry);
            }
        }
        out.count(self.completions.len());
        for ((from, to), queue) in &self.completions {
            out.u32(from.0);
            out.u32(to.0);
            out.count(queue.len());
            for entry in queue {
                out.cqe(entry);
            }
        }
    }

    /// Read one back.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        let mut wire = Self::new();
        let channels = input.count(12, "more submission channels than the file could hold");
        for _ in 0..channels {
            let from = ActorId(input.u32());
            let to = ActorId(input.u32());
            let count = input.count(60, "more submissions than the file could hold");
            let mut queue = VecDeque::with_capacity(count);
            for _ in 0..count {
                queue.push_back(input.sqe());
            }
            wire.submissions.insert((from, to), queue);
        }
        let channels = input.count(12, "more completion channels than the file could hold");
        for _ in 0..channels {
            let from = ActorId(input.u32());
            let to = ActorId(input.u32());
            let count = input.count(32, "more completions than the file could hold");
            let mut queue = VecDeque::with_capacity(count);
            for _ in 0..count {
                queue.push_back(input.cqe());
            }
            wire.completions.insert((from, to), queue);
        }
        wire
    }
}

/// A [`Submitter`] that puts the entry on the wire, and refuses when the ring
/// is full.
///
/// The reason [`f_ring::buffers::Idle::submit`] takes a `Submitter` rather than
/// a `Producer`: the ownership rules do not care what is on the far end, and a
/// buffer refused by a full ring comes back to its owner rather than being lost.
/// `depth` is what makes that path run here — a submission into a wire already
/// holding `depth` entries is [`RingError::Full`], which is the same answer the
/// real ring gives and the same one the client's back-pressure path was written
/// against.
pub struct Post<'w> {
    world: &'w mut World,
    from: ActorId,
    to: ActorId,
    depth: u32,
}

impl<'w> Post<'w> {
    /// A submitter that writes onto the channel from `from` to `to`, refusing
    /// once `depth` entries are unread.
    pub fn new(world: &'w mut World, from: ActorId, to: ActorId, depth: u32) -> Self {
        Self { world, from, to, depth }
    }
}

impl Submitter for Post<'_> {
    fn submit(&mut self, entry: Sqe) -> Result<bool, RingError> {
        if self.world.wire().queued(self.from, self.to) >= self.depth {
            return Err(RingError::Full);
        }
        self.world.wire().post(self.from, self.to, entry);
        // `true`: the consumer always wants to be rung here, because the
        // recipient is an actor and the only thing that wakes one is a message.
        // A suppressed doorbell is `f_ring`'s optimisation over a *running*
        // consumer, and a modelled consumer that is not running has nothing to
        // suppress. `E1-B15`'s doorbell counts are measured on the real ring.
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_abi::Sqe;

    fn entry(token: u64) -> Sqe {
        Sqe { user_data: token, ..Sqe::ZERO }
    }

    #[test]
    fn one_channel_keeps_its_order() {
        // The ring's own guarantee. If the wire could reorder one producer's
        // entries, the simulator would explore an interleaving `f_ring`'s
        // single-producer discipline makes impossible.
        let mut wire = Wire::new();
        for token in 0..8 {
            wire.post(ActorId(1), ActorId(2), entry(token));
        }
        let mut seen = Vec::new();
        while let Some(sqe) = wire.take(ActorId(1), ActorId(2)) {
            seen.push(sqe.user_data);
        }
        assert_eq!(seen, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn two_channels_do_not_share_a_queue() {
        let mut wire = Wire::new();
        wire.post(ActorId(1), ActorId(2), entry(10));
        wire.post(ActorId(3), ActorId(2), entry(20));
        // The reverse direction of the first channel is a third channel and is
        // empty: a wire is directed, exactly as a ring is.
        assert!(wire.take(ActorId(2), ActorId(1)).is_none());
        assert_eq!(wire.take(ActorId(1), ActorId(2)).map(|s| s.user_data), Some(10));
        assert_eq!(wire.take(ActorId(3), ActorId(2)).map(|s| s.user_data), Some(20));
        assert_eq!(wire.queued(ActorId(1), ActorId(2)), 0);
    }

    #[test]
    fn completions_travel_the_other_way_and_do_not_mix() {
        let mut wire = Wire::new();
        wire.post(ActorId(1), ActorId(2), entry(1));
        wire.answer(ActorId(2), ActorId(1), f_abi::Cqe { user_data: 9, ..f_abi::Cqe::ZERO });
        assert_eq!(wire.queued(ActorId(1), ActorId(2)), 1, "a completion counted as a submission");
        assert_eq!(wire.reap(ActorId(2), ActorId(1)).map(|c| c.user_data), Some(9));
        assert!(wire.reap(ActorId(2), ActorId(1)).is_none());
    }

    #[test]
    fn a_full_wire_refuses_and_the_buffer_is_not_lost() {
        // The property `Idle::submit` is built around, checked at the submitter
        // rather than through it: a refusal is a retry, so nothing may be
        // consumed by one.
        let mut world = World::new(1);
        let mut post = Post::new(&mut world, ActorId(0), ActorId(1), 2);
        assert_eq!(post.submit(entry(0)), Ok(true));
        assert_eq!(post.submit(entry(1)), Ok(true));
        assert_eq!(post.submit(entry(2)), Err(RingError::Full));
        assert_eq!(world.wire().queued(ActorId(0), ActorId(1)), 2);
    }
}
