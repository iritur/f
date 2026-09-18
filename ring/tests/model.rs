// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The small tests a model checker can exhaust.
//!
//! # What this file is for
//!
//! `ring/tests/litmus.rs` samples what one machine happened to do across half a
//! million rounds. This file states the same properties over the smallest
//! program that can still exhibit them, so that a checker exploring *every*
//! interleaving the memory model permits terminates.
//!
//! The two answer different questions and neither replaces the other. A stress
//! test finds a reordering the hardware in front of it actually performs; an
//! exhaustive check finds one RC11 permits and no machine here has yet
//! produced. `E0-P16` exists because the second has never run, and this file is
//! the half of it that needs no new toolchain.
//!
//! # Why these are ordinary `#[test]` functions
//!
//! Because they have to earn their place before the checker arrives and keep it
//! afterwards. `cargo rustmc test` explores a crate's *test targets*, so these
//! are the targets — but they also run under `cargo test` on every machine
//! today, which means a change that breaks the property is caught by the
//! ordinary suite rather than waiting for a nightly that does not exist yet.
//!
//! A file that only a tool nobody has can run is a file nobody edits correctly.
//!
//! # The rule every test here follows: no unbounded wait
//!
//! Every loop in `litmus.rs` spins until it gets what it is waiting for. That is
//! right for a stress test and fatal for an exhaustive one — a checker
//! exploring a spin has to decide the loop terminates, and the honest way to
//! avoid asking it that is not to write the loop.
//!
//! So each test here does a **fixed** number of operations and asserts about
//! what it observed, including the case where it observed nothing. *Nothing yet*
//! is a legal outcome of a race and is asserted as such rather than retried
//! away. Where a property needs the other thread to have finished, the threads
//! are joined first and the assertion is made after — which is a different
//! claim from the racing one and is labelled as such.
//!
//! # What these tests do NOT cover
//!
//! The four mutations `RING_PROOF_BLIND` names. Those are about orderings a
//! *sequential* checker cannot see, and Kani is sequential. A checker that
//! explores concurrency — which is what `E0-P16` is about — is exactly the
//! instrument that would see them, so when it arrives these are the targets to
//! point it at first. Until then `mutate-relaxed-submission` and its three
//! siblings are declared gaps rather than covered ones.
//!
//! See `docs/design/proving-ground.html` layer 2, RFC 0020, RFC 0022.

use std::cell::UnsafeCell;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Barrier};
use std::thread;

use f_abi::{Cqe, Sqe};
use f_ring::{Channel, Collector, Completions, Consumer, Cursor, Poster, Producer};

/// The smallest ring the protocol admits.
///
/// Two, because the cursor protocol requires a power of two and one would leave
/// no interleaving to explore: a ring of one is full the moment it is not empty,
/// so a producer and a consumer never overlap in it and the property under test
/// cannot fail. Two is the first size at which they can.
const RING: usize = 2;

/// Shared backing, the same shape `litmus.rs` uses and for the same reason.
///
/// A `f_ring::Mapping` holds a raw base and is neither `Send` nor `Sync`, and
/// every test here hands one half of a ring to another thread. The subject is
/// one `Release` store and one `Acquire` load; a fixture that cannot be laid out
/// wrongly is the right fixture for a question that is not about layout.
struct Shared {
    head: Cursor,
    tail: Cursor,
    flags: AtomicU32,
    index: Vec<AtomicU32>,
    entries: Vec<UnsafeCell<Sqe>>,
    cq_head: Cursor,
    cq_tail: Cursor,
    slots: Vec<UnsafeCell<Cqe>>,
}

// SAFETY: the ring protocol is what makes concurrent access sound — the
// producer writes only the slot `head` names before publishing it, and the
// consumer reads only slots the producer has already published. This impl
// asserts exactly the property the tests below exist to check.
unsafe impl Sync for Shared {}

impl Shared {
    fn new(n: usize) -> Self {
        Self {
            head: Cursor::new(),
            tail: Cursor::new(),
            flags: AtomicU32::new(0),
            index: (0..n).map(|_| AtomicU32::new(0)).collect(),
            entries: (0..n).map(|_| UnsafeCell::new(Sqe::ZERO)).collect(),
            cq_head: Cursor::new(),
            cq_tail: Cursor::new(),
            slots: (0..n).map(|_| UnsafeCell::new(Cqe::ZERO)).collect(),
        }
    }

    fn chan(&self) -> Channel<'_> {
        Channel {
            head: &self.head,
            tail: &self.tail,
            flags: &self.flags,
            index: &self.index,
            entries: &self.entries,
        }
    }

    fn cq(&self) -> Completions<'_> {
        Completions { head: &self.cq_head, tail: &self.cq_tail, slots: &self.slots }
    }
}

/// An entry whose every field restates `n`, so a torn publish is visible
/// without an oracle.
fn marked(n: u64) -> Sqe {
    let mut sqe = Sqe::ZERO;
    sqe.user_data = n;
    sqe.offset = n;
    sqe.ext = [n, !n];
    sqe
}

/// A publish is all of it or none of it, over one entry.
///
/// `litmus.rs` asserts this over two hundred thousand entries and a spin loop.
/// The same property needs exactly one entry and one attempt to pop: either the
/// consumer observes the advanced cursor, in which case it must observe every
/// byte written before it, or it observes nothing, which is a legal outcome of
/// the race and not a failure.
///
/// The redundancy in the payload is what makes the first case checkable without
/// knowing which thread ran first.
#[test]
fn one_published_entry_is_whole_or_absent() {
    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let producing = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let producer = Producer::new(shared.chan()).expect("power of two");
            barrier.wait();
            producer.submit(marked(1)).expect("a healthy ring");
        })
    };

    let consuming = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let consumer = Consumer::new(shared.chan()).expect("power of two");
            barrier.wait();
            // One attempt. Not a loop: `None` is an answer here.
            if let Some(sqe) = consumer.pop().expect("a healthy ring") {
                assert_eq!(sqe.user_data, 1, "the only entry published was 1");
                assert_eq!(
                    sqe.offset, sqe.user_data,
                    "TORN PUBLISH: offset does not match user_data. The Release/Acquire \
                     pair in ring::submit/pop has been weakened, or a field was written \
                     after the cursor was advanced."
                );
                assert_eq!(
                    sqe.ext,
                    [sqe.user_data, !sqe.user_data],
                    "TORN PUBLISH: ext words do not match user_data. See above."
                );
            }
        })
    };

    producing.join().expect("producer thread");
    consuming.join().expect("consumer thread");

    // After the join the race is over, so this is a different claim from the one
    // above and is made separately: the entry was published, so it is there.
    let consumer = Consumer::new(shared.chan()).expect("power of two");
    let left = consumer.pop().expect("a healthy ring");
    assert!(
        left.is_none() || left.expect("checked").user_data == 1,
        "after both threads have finished the ring holds the entry that was published, \
         or nothing if the consumer already took it"
    );
}

/// A batch is published by one store, so a consumer sees none of it or all of
/// it — never one entry of two.
///
/// This is the property that makes `Producer::batch` worth having, and it is the
/// one an exhaustive checker is best placed to break: a partial batch needs the
/// consumer to observe the cursor between two of the producer's stores, which a
/// stress test can only wait for and a checker can construct.
#[test]
fn a_batch_of_two_is_never_seen_as_one() {
    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let producing = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut producer = Producer::new(shared.chan()).expect("power of two");
            barrier.wait();
            let mut batch = producer.batch();
            batch.push(marked(1)).expect("room for two");
            batch.push(marked(2)).expect("room for two");
            batch.publish().expect("a healthy ring");
        })
    };

    let consuming = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let consumer = Consumer::new(shared.chan()).expect("power of two");
            barrier.wait();
            // Two attempts, fixed. If the first found an entry the batch was
            // published, and the second must therefore find the other one.
            let first = consumer.pop().expect("a healthy ring");
            let second = consumer.pop().expect("a healthy ring");
            if let Some(one) = first {
                assert_eq!(one.user_data, 1, "a batch arrives in the order it was staged");
                assert_eq!(one.ext, [one.user_data, !one.user_data], "TORN PUBLISH");
                let two = second.expect(
                    "PARTIAL BATCH: the first entry of a batch was visible and the second \
                     was not. `Batch::publish` makes both visible with a single Release \
                     store, so observing one means observing both. A per-entry publish has \
                     been reintroduced.",
                );
                assert_eq!(two.user_data, 2, "the second entry of the batch");
                assert_eq!(two.ext, [two.user_data, !two.user_data], "TORN PUBLISH");
            } else {
                assert!(
                    second.is_none(),
                    "the batch became visible between two pops, which is legal, but the \
                     first pop returning None and the second returning Some means the \
                     cursor moved by one rather than by two"
                );
            }
        })
    };

    producing.join().expect("producer thread");
    consuming.join().expect("consumer thread");
}

/// The completion half, which RFC 0018 built as the mirror and which inherited
/// its ordering argument wholesale.
///
/// An inherited argument is the one kind of claim this suite exists not to take
/// on faith, so the mirror gets its own bounded test rather than a sentence
/// saying it is symmetric.
#[test]
fn one_posted_completion_is_whole_or_absent() {
    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let posting = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let poster = Poster::new(shared.cq()).expect("power of two");
            barrier.wait();
            // Every word restates the same value, for `marked`'s reason: `ext`
            // carries its complement so a half-written completion is visible as
            // one without needing an oracle.
            poster
                .post(Cqe { user_data: 1, result: 1, timestamp: 1, flags: 0, ext: !1u64 })
                .expect("a healthy ring");
        })
    };

    let collecting = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let collector = Collector::new(shared.cq()).expect("power of two");
            barrier.wait();
            if let Some(cqe) = collector.take().expect("a healthy ring") {
                assert_eq!(cqe.user_data, 1, "the only completion posted was 1");
                assert_eq!(
                    cqe.result, 1,
                    "TORN POST: result does not match user_data. The Release/Acquire pair \
                     in ring::post/take has been weakened. RFC 0018."
                );
                assert_eq!(cqe.timestamp, 1, "TORN POST: timestamp. See above.");
                assert_eq!(cqe.ext, !1u64, "TORN POST: ext. See above.");
            }
        })
    };

    posting.join().expect("poster thread");
    collecting.join().expect("collector thread");
}

/// The lost wakeup, in one round.
///
/// This is the defect RFC 0020 is about and the reason this file matters more
/// than its size suggests. The producer stores `head` then loads `flags`; the
/// consumer stores `flags` then loads `head`. A store followed by a load of a
/// *different* location is the one reordering total store order permits, and
/// `Release`/`Acquire` do not forbid it — they are one-way barriers and this
/// needs a two-way one.
///
/// `litmus.rs` finds it in sixty-nine rounds on x86-64 and **never** on
/// AArch64, where `stlr`/`ldar` are RCsc and the architecture orders the pair
/// for free. So the stress test's power depends on which machine runs it, which
/// is exactly the dependency an exhaustive checker removes: RC11 permits the
/// reordering whatever a backend emits today.
///
/// One round, and the two outcomes are enumerated rather than counted.
#[test]
fn a_sleeping_consumer_is_never_left_holding_work_in_one_round() {
    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));
    let rang = Arc::new(AtomicU32::new(0));

    let producing = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        let rang = Arc::clone(&rang);
        thread::spawn(move || {
            let producer = Producer::new(shared.chan()).expect("power of two");
            barrier.wait();
            let wanted = producer.submit(Sqe::ZERO).expect("a healthy ring");
            rang.store(u32::from(wanted), std::sync::atomic::Ordering::Relaxed);
        })
    };

    let consuming = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let consumer = Consumer::new(shared.chan()).expect("power of two");
            barrier.wait();
            consumer.arm_wakeup();
            consumer.pop().expect("a healthy ring").is_some()
        })
    };

    producing.join().expect("producer thread");
    let got = consuming.join().expect("consumer thread");
    let rung = rang.load(std::sync::atomic::Ordering::Relaxed) != 0;

    assert!(
        rung || got,
        "LOST WAKEUP. The producer published an entry, read NEED_WAKEUP as clear and rang \
         nothing, while the consumer armed NEED_WAKEUP, saw an empty ring, and would now \
         sleep. The entry is stranded and nothing will come for it. The StoreLoad fence in \
         ring::Producer::doorbell_wanted has been removed or weakened — Release and Acquire \
         do not forbid this reordering, and it is the one total store order performs. \
         RFC 0020."
    );
}
