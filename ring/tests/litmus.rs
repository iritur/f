// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Litmus tests: permanent regression guards on the ring's memory ordering.
//!
//! # Why these exist as a separate file
//!
//! The ring's correctness rests on one `Release` store and one `Acquire` load.
//! Weakening either to `Relaxed` makes the code *faster* and *still passes every
//! functional test on x86-64*, because total store order hides the bug. It then
//! corrupts data on AArch64.
//!
//! These tests are the thing that stops a future contributor from making that
//! change. They are named and documented so a failure explains itself rather
//! than looking like flakiness.
//!
//! # What these tests are and are not
//!
//! They are **stress tests**: many threads, many iterations, checking an
//! invariant that a reordering would violate. That is empirical and will not
//! catch a rare interleaving reliably.
//!
//! They are **not** a model check. Layer 2 of the testing plan calls for
//! RustMC — a stateless model checker built on GenMC that explores what the
//! RC11 memory model *permits*, rather than what one machine happened to do.
//! That lands at M5. Until then these tests plus an AArch64 CI job are the
//! coverage, and the gap should be stated rather than assumed away.
//!
//! See `docs/design/proving-ground.html` layer 2.

use std::cell::UnsafeCell;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Barrier};
use std::thread;

use f_abi::Sqe;
use f_ring::{Channel, Consumer, Cursor, Producer};

/// Shared backing that outlives both threads.
struct Shared {
    head: Cursor,
    tail: Cursor,
    flags: AtomicU32,
    entries: Vec<UnsafeCell<Sqe>>,
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
            entries: (0..n).map(|_| UnsafeCell::new(Sqe::ZERO)).collect(),
        }
    }

    fn chan(&self) -> Channel<'_> {
        Channel { head: &self.head, tail: &self.tail, flags: &self.flags, entries: &self.entries }
    }
}

/// **The load-bearing invariant.**
///
/// A consumer that observes an advanced `head` must also observe every byte the
/// producer wrote to that slot before advancing it. With `Release`/`Acquire`
/// that is guaranteed. With `Relaxed` the entry read may see a stale slot, and
/// on a weakly ordered machine it will.
///
/// The payload here is redundant on purpose: `user_data`, `offset` and both
/// `ext` words carry the same value, so a torn publish shows up as a mismatch
/// between fields rather than needing an oracle.
#[test]
fn published_entry_is_fully_visible() {
    const RING: usize = 64;
    const COUNT: u64 = 200_000;

    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let producer_side = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let producer = Producer::new(shared.chan()).expect("power of two");
            barrier.wait();
            let mut sent = 0u64;
            while sent < COUNT {
                let mut sqe = Sqe::ZERO;
                sqe.user_data = sent;
                sqe.offset = sent;
                sqe.ext = [sent, !sent];
                match producer.submit(sqe) {
                    Ok(_) => sent += 1,
                    Err(f_ring::RingError::Full) => std::hint::spin_loop(),
                    Err(e) => panic!("producer saw {e:?}"),
                }
            }
        })
    };

    let consumer_side = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let consumer = Consumer::new(shared.chan()).expect("power of two");
            barrier.wait();
            let mut expect = 0u64;
            while expect < COUNT {
                match consumer.pop() {
                    Ok(Some(sqe)) => {
                        assert_eq!(
                            sqe.user_data, expect,
                            "entries must arrive in publication order"
                        );
                        assert_eq!(
                            sqe.offset, sqe.user_data,
                            "TORN PUBLISH: offset does not match user_data. \
                             The Release/Acquire pair in ring::submit/pop has been \
                             weakened, or a field was written after the cursor was \
                             advanced."
                        );
                        assert_eq!(
                            sqe.ext,
                            [sqe.user_data, !sqe.user_data],
                            "TORN PUBLISH: ext words do not match user_data. See above."
                        );
                        expect += 1;
                    }
                    Ok(None) => std::hint::spin_loop(),
                    Err(e) => panic!("consumer saw {e:?}"),
                }
            }
        })
    };

    producer_side.join().expect("producer thread");
    consumer_side.join().expect("consumer thread");
}

/// A consumer must never observe more entries than were published, however the
/// two cursors are interleaved. This is the invariant that a missing bounds
/// check would break, and it holds independently of the ordering question.
#[test]
fn occupancy_never_exceeds_capacity() {
    const RING: usize = 16;
    const COUNT: u64 = 100_000;

    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    // The producer observes its own occupancy, and the consumer runs on the
    // other thread. Both halves of that are load-bearing.
    //
    // `occupancy` reads `head` `Relaxed` and `tail` `Acquire`, which is sound
    // for the cursor's *owner* and for nobody else: the producer wrote `head`
    // itself, and `tail` only ever advances, so the difference can only be an
    // over-estimate that is still within capacity. A third thread holding a
    // second `Producer` sees a stale `head` against a fresh `tail`, underflows
    // the subtraction, and reports `Corrupt` on a perfectly healthy ring —
    // intermittently, which is the worst way to find out. This test did that,
    // and it also ran submit and pop on one thread, so it never raced the two
    // cursors it exists to race.
    let producing = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let producer = Producer::new(shared.chan()).expect("power of two");
            barrier.wait();
            for _ in 0..COUNT {
                let _ = producer.submit(Sqe::ZERO);
                match producer.occupancy() {
                    Ok(used) => assert!(
                        used as usize <= RING,
                        "occupancy {used} exceeds ring capacity {RING}"
                    ),
                    Err(e) => panic!("occupancy reported {e:?} on a healthy ring"),
                }
            }
        })
    };

    let consuming = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let consumer = Consumer::new(shared.chan()).expect("power of two");
            barrier.wait();
            for _ in 0..COUNT {
                let _ = consumer.pop();
            }
        })
    };

    producing.join().expect("producer thread");
    consuming.join().expect("consumer thread");
}

/// A hostile peer writing an impossible cursor must produce `Corrupt`, never a
/// panic and never an out-of-bounds access. This is the executable form of the
/// survival rules: a peer must not be able to halt us by writing an integer.
#[test]
fn a_hostile_cursor_never_panics() {
    let shared = Shared::new(8);
    let consumer = Consumer::new(shared.chan()).expect("power of two");
    let producer = Producer::new(shared.chan()).expect("power of two");

    for bad in [9u32, 100, u32::MAX / 2, u32::MAX] {
        shared.head.set(bad);
        assert!(
            matches!(consumer.pop(), Err(f_ring::RingError::Corrupt)),
            "head={bad} must be reported as Corrupt"
        );
        assert!(
            matches!(producer.occupancy(), Err(f_ring::RingError::Corrupt)),
            "head={bad} must be reported as Corrupt"
        );
    }
}
