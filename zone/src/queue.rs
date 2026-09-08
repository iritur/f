// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `Q` — the driver's queue, modelled at the one width RFC 0059's third
//! invariant is about.
//!
//! # Why there is a queue in a library with no ring in it
//!
//! Because I3 is a statement about `Q` and about nothing else: *for every entry
//! submitted to the blk ring whose inherited class is more urgent than
//! `class::BATCH`, at most [`COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ`]
//! collector operations are handed to the device after it was submitted and
//! before it was.* An invariant over a thing this crate does not have is an
//! invariant nothing can fail, and `E2-P03`'s exit is *invariants*. So the
//! queue is here, modelled at exactly two properties — the order it hands
//! entries over in, and how many it lets inside the device at once — and at no
//! others. It is not a virtqueue, it moves no bytes, and it is not on the path
//! any byte takes.
//!
//! The ordering is not restated here either: [`Inherited::rank`] is the key a
//! device queue orders by, class first and then deadline with `NO_DEADLINE`
//! last within its class, and `E1-B06` made every resource scheduler use it. A
//! second transcription of that comparison is how two schedulers come to
//! disagree about which entry is more urgent.
//!
//! # The bound is derived and not chosen
//!
//! An entry already inside the device cannot be overtaken by anything (RFC
//! 0049, point 2). `f_virtio_blk::pending::IN_FLIGHT` is 1, because
//! `Driver::execute` offers one chain and polls the used ring until it comes
//! back. Every other collector operation is still in the driver's queue, where
//! it sorts below any entry above batch. So the collector work that can be
//! ahead of an urgent read is exactly the collector work already in the device,
//! which is at most `IN_FLIGHT`. The bound below is 1 because that constant is
//! 1, and `the_bound_is_the_drivers_chain_length` in this file's tests is what
//! makes that a fact rather than a transcription.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use f_abi::deadline::Inherited;
use f_abi::store::refusal;
use f_abi::{NO_DEADLINE, class};

/// How many collector operations may be handed to the device ahead of an entry
/// more urgent than batch.
///
/// Unit: count of device operations. Derived from
/// `f_virtio_blk::pending::IN_FLIGHT`, not chosen — see this module's last
/// paragraph. *Reversal:* `E1-B09` raising `IN_FLIGHT`, at which point this
/// constant is re-derived in the same diff, exactly as claim 0012's
/// `in_flight_above_one` note says happens to `batch_operations_overtaken`.
/// One constant, two claims and one invariant reading it, rather than three
/// numbers that agree by accident.
pub const COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ: u32 = 1;

/// How many entries the driver keeps inside the device at once.
///
/// Unit: count of device operations. The same number as the bound above and for
/// the same reason, named twice because they are two different claims that
/// happen to be equal: one is about the device's depth and the other is about
/// what an urgent read can be stuck behind. They move together, and a diff that
/// moved one would have to say why it did not move the other.
pub const IN_FLIGHT: u32 = 1;

/// One entry, at the width the ordering is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Unit: none — a class ordinal, smaller is more urgent.
    pub class: u16,
    /// Unit: nanoseconds, monotonic, in the channel's epoch. Zero is
    /// [`NO_DEADLINE`].
    pub deadline: u64,
    /// Submission order, which breaks a tie the rank does not.
    /// Unit: none — a sequence number, counting from one.
    pub seq: u64,
    /// Whether the collector submitted it. The whole of I3's subject.
    /// Unit: none — a fact about the submitter.
    pub collector: bool,
}

impl Entry {
    /// The key a device queue orders by.
    fn rank(&self) -> (u16, u64, u64) {
        let (class, deadline) =
            Inherited { class: self.class, deadline: self.deadline, depth: 0, shortfall: 0 }.rank();
        (class, deadline, self.seq)
    }

    /// Whether this entry is one I3 protects: strictly more urgent than batch.
    ///
    /// Stated for *any* entry above batch and not only for hard, because
    /// `user/virtio-blk` declares the soft class and RFC 0025's bound 1
    /// therefore serves every hard-class read as soft with `SHORTFALL` set. An
    /// invariant written only about hard entries would be vacuous against the
    /// driver this epoch actually has.
    const fn urgent(&self) -> bool {
        self.class < class::BATCH
    }
}

/// The driver's queue: what has been submitted, what is inside the device, and
/// how much collector work has got ahead of each urgent entry waiting.
#[derive(Debug, Default)]
pub struct Queue {
    pending: Vec<Entry>,
    in_flight: Option<Entry>,
    /// Per urgent entry still waiting, how many collector operations have been
    /// handed to the device since it was submitted.
    /// Unit: count of device operations, keyed by submission order.
    ahead: BTreeMap<u64, u32>,
    /// The largest such count this queue has ever completed with — I3's own
    /// witness, published as `ops_ahead_of_urgent_read`.
    /// Unit: count of device operations.
    worst: u32,
    /// Unit: none — the next sequence number.
    next: u64,
}

impl Queue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum collector operations ever observed ahead of an urgent entry.
    ///
    /// Unit: count of device operations. This is the number I3 bounds and the
    /// number RFC 0059 publishes as `ops_ahead_of_urgent_read`.
    #[must_use]
    pub const fn ops_ahead_of_urgent_read(&self) -> u32 {
        self.worst
    }

    /// How many entries are waiting.
    ///
    /// Unit: count of entries.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.pending.len()
    }

    /// Submit an entry the collector owns.
    ///
    /// `class::BATCH` and `NO_DEADLINE` are written here rather than taken as
    /// arguments, because RFC 0025's scope rule says a component's own work
    /// carries the component's own class and `user/objects` is not admitted for
    /// batch — so the collector writes batch into every entry it submits, and a
    /// function that let a caller choose would be a function through which the
    /// collector could submit at its component's class.
    pub fn submit_collector(&mut self) -> u64 {
        self.submit(class::BATCH, NO_DEADLINE, true)
    }

    /// Submit an entry somebody else owns — a client read, at its own class.
    pub fn submit_client(&mut self, class: u16, deadline: u64) -> u64 {
        self.submit(class, deadline, false)
    }

    fn submit(&mut self, class: u16, deadline: u64, collector: bool) -> u64 {
        self.next += 1;
        let entry = Entry { class, deadline, seq: self.next, collector };
        if entry.urgent() {
            // What is already inside the device cannot be overtaken, so an
            // urgent entry starts life behind whatever is in there. This is the
            // only way the count ever becomes non-zero in a correct queue, and
            // it is exactly RFC 0059's derivation.
            let started_behind = u32::from(self.in_flight.is_some_and(|held| held.collector));
            self.ahead.insert(entry.seq, started_behind);
        }
        self.pending.push(entry);
        entry.seq
    }

    /// Hand the most urgent waiting entry to the device.
    ///
    /// Answers `None` when nothing is waiting, and refuses when the device is
    /// already holding [`IN_FLIGHT`] entries — which is what makes the depth a
    /// property of this type rather than a discipline its callers keep.
    ///
    /// # Errors
    ///
    /// [`refusal::FULL`] when the device already holds an entry.
    pub fn hand_over(&mut self) -> Result<Option<Entry>, i32> {
        if self.in_flight.is_some() {
            return Err(refusal::FULL);
        }
        let Some(at) = (0..self.pending.len()).min_by_key(|index| self.pending[*index].rank())
        else {
            return Ok(None);
        };
        let entry = self.pending.remove(at);
        if entry.collector {
            // Every urgent entry still waiting has just had one collector
            // operation put in front of it. In a queue that orders correctly
            // this loop never runs — an urgent entry would have been picked
            // instead — which is the point: it is the branch that would fire if
            // RFC 0049's application were wrong, and I3 is what would then go
            // red.
            for count in self.ahead.values_mut() {
                *count += 1;
            }
        }
        self.in_flight = Some(entry);
        Ok(Some(entry))
    }

    /// The device finished the entry it was holding.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] when the device was holding nothing, which is a
    /// caller completing twice.
    pub fn complete(&mut self) -> Result<Entry, i32> {
        let entry = self.in_flight.take().ok_or(refusal::ADDRESS)?;
        if entry.urgent()
            && let Some(count) = self.ahead.remove(&entry.seq)
        {
            self.worst = self.worst.max(count);
        }
        Ok(entry)
    }

    /// Complete whatever the device is holding, if it is holding anything.
    ///
    /// # Errors
    ///
    /// Whatever [`Queue::complete`] says.
    pub fn settle(&mut self) -> Result<(), i32> {
        if self.in_flight.is_some() {
            self.complete()?;
        }
        Ok(())
    }

    /// Submit one collector operation and get it as far as the device — serving
    /// anything more urgent that was waiting on the way, and **leaving the
    /// collector's own entry inside the device when it returns**.
    ///
    /// # Why it is left in flight rather than completed here
    ///
    /// Because that is the only shape in which I3 measures anything. A
    /// collector that submitted, waited and completed inside one call would
    /// never have work inside the device at the instant a client submits, so
    /// `started_behind` would be zero for every urgent entry and the witness
    /// would be zero on every run — an invariant that cannot fail, which is the
    /// thing RFC 0059 wrote three predicates to avoid. A real collector's
    /// completion arrives after it went back to work, and this models that: the
    /// entry is settled at the *next* operation, so an urgent read submitted
    /// between two collector steps starts behind exactly one, which is the
    /// bound's own derivation.
    ///
    /// A caller that has finished collecting calls [`Queue::settle`].
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] if the queue empties with the collector's own entry
    /// unserved, which cannot happen and is checked rather than assumed;
    /// otherwise whatever [`Queue::hand_over`] or [`Queue::complete`] says.
    pub fn collector_operation(&mut self) -> Result<(), i32> {
        self.settle()?;
        let mine = self.submit_collector();
        loop {
            let handed = self.hand_over()?.ok_or(refusal::ADDRESS)?;
            if handed.seq == mine {
                return Ok(());
            }
            // Something more urgent was waiting and the queue ranked it above
            // this entry, which is RFC 0049 working. It is served to
            // completion before the collector's own operation goes anywhere.
            self.complete()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ, IN_FLIGHT, Queue};
    use f_abi::store::refusal;
    use f_abi::{NO_DEADLINE, class};

    /// The two constants in this file are one number, said twice about two
    /// different things — the device's depth and what an urgent read can be
    /// stuck behind. That they are equal is not a coincidence to be discovered
    /// later.
    ///
    /// Whether either of them is still *the driver's* number is checked by
    /// `zone/tests/cycle.rs`, which reads `user/virtio-blk/src/pending.rs`;
    /// `zone/Cargo.toml` says why that is a file read rather than a dependency.
    #[test]
    fn the_bound_and_the_depth_are_one_number() {
        assert_eq!(COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ, IN_FLIGHT);
    }

    #[test]
    fn an_urgent_entry_is_handed_over_before_batch_work_that_was_waiting_first() {
        let mut queue = Queue::new();
        queue.submit_collector();
        queue.submit_collector();
        let read = queue.submit_client(class::HARD, 100);
        let first = queue.hand_over().expect("nothing in flight").expect("three waiting");
        assert_eq!(first.seq, read, "the queue orders by rank and not by arrival");
        queue.complete().expect("the device finished it");
        assert_eq!(queue.ops_ahead_of_urgent_read(), 0, "nothing was ahead of it");
    }

    /// The bound's own derivation, run: an urgent read submitted while one
    /// collector operation is inside the device starts behind exactly that one.
    #[test]
    fn an_urgent_read_starts_behind_what_is_already_inside_the_device() {
        let mut queue = Queue::new();
        queue.submit_collector();
        queue.hand_over().expect("nothing in flight").expect("one waiting");
        assert_eq!(queue.hand_over(), Err(refusal::FULL), "IN_FLIGHT is one");

        queue.submit_client(class::HARD, 50);
        queue.complete().expect("the collector operation came back");
        queue.hand_over().expect("the device is free").expect("the read");
        queue.complete().expect("and it came back");

        assert_eq!(queue.ops_ahead_of_urgent_read(), COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ);
    }

    /// I3 ranges over any entry above batch and not only over hard, because the
    /// driver declares soft and every hard read is served as soft.
    #[test]
    fn soft_is_protected_as_well_as_hard_and_batch_is_not() {
        let mut queue = Queue::new();
        queue.submit_collector();
        queue.hand_over().expect("nothing in flight").expect("one waiting");
        queue.submit_client(class::SOFT, NO_DEADLINE);
        queue.submit_client(class::BATCH, NO_DEADLINE);
        queue.complete().expect("the collector operation came back");

        let next = queue.hand_over().expect("free").expect("two waiting");
        assert_eq!(next.class, class::SOFT, "soft outranks batch");
        queue.complete().expect("done");
        assert_eq!(queue.ops_ahead_of_urgent_read(), 1, "the soft entry was behind one");

        let last = queue.hand_over().expect("free").expect("one waiting");
        assert_eq!(last.class, class::BATCH);
        queue.complete().expect("done");
        assert_eq!(queue.ops_ahead_of_urgent_read(), 1, "and the batch entry counted for nothing");
    }

    /// The whole reason the collector's entry is left in flight: an urgent read
    /// submitted between two collector steps starts behind exactly one, which
    /// is the bound rather than a number this file chose.
    #[test]
    fn a_read_submitted_between_two_collector_steps_starts_behind_exactly_one() {
        let mut queue = Queue::new();
        queue.collector_operation().expect("one step");
        assert_eq!(queue.depth(), 0, "nothing waiting");

        // The client, between two steps of the cycle.
        queue.submit_client(class::HARD, NO_DEADLINE);

        // The next step settles the first operation, serves the read because it
        // outranks batch, and only then gets its own entry to the device.
        queue.collector_operation().expect("the next step");
        assert_eq!(queue.ops_ahead_of_urgent_read(), COLLECTOR_OPERATIONS_AHEAD_OF_HARD_READ);

        queue.settle().expect("the cycle ends");
        assert_eq!(queue.depth(), 0);
        assert_eq!(queue.complete(), Err(refusal::ADDRESS), "the device holds nothing");
    }
}
