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
//! This file said "that lands at M5" until E0-P16 went looking: M5 arrived at
//! E0-B12 and the checker did not, and it is not a crate — it needs GenMC built
//! against an LLVM the pinned toolchain's rustc agrees with. Until it exists,
//! these tests plus an AArch64 CI job are the coverage, and the gap should be
//! stated rather than assumed away.
//!
//! # The half that stands in for it, and how far it reaches
//!
//! Every ordering these tests guard has a deliberate defect behind a cargo
//! feature, because a stress test that has only ever passed cannot be told
//! apart from one that cannot fail. Two of the three are *not* gates, and that
//! is a result rather than a gap in the wiring:
//!
//! - `mutate-no-doorbell-fence` removes the StoreLoad fence in the suppression
//!   protocol. **Caught, on both runners, and it gates.** Store-load is a
//!   reordering total store order actually performs, so this shows up on an
//!   ordinary laptop at eight rounds in a thousand — RFC 0020.
//! - `mutate-relaxed-submission` and `mutate-relaxed-completion` weaken the two
//!   publishing stores. **Not caught.** They were run as gates on the AArch64
//!   runner for exactly one CI run — the machine where the weakening is a real
//!   defect — and the suite passed with both.
//!
//! That second result is this file's own first paragraph arriving as evidence,
//! and it is worth more than a green tick would have been. These tests sample
//! what one machine happened to do. They did not catch a `Release` store
//! weakened to `Relaxed` on hardware that is entitled to reorder it, and no
//! amount of iterating the harness would make that a *guarantee* — it would
//! only move the probability. E0-P16 is the task that closes this, and the
//! shape of what it must close is now measured rather than assumed.
//!
//! See `docs/design/proving-ground.html` layer 2.

use std::cell::UnsafeCell;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Barrier};
use std::thread;

use f_abi::{Cqe, Sqe};
use f_ring::{Channel, Collector, Completions, Consumer, Cursor, Poster, Producer};

/// Shared backing that outlives both threads.
///
/// This file keeps a fixture where the rest of the suite moved to
/// `f_ring::Mapping` at E0-B13, and the reason is `Sync`: a mapping holds a raw
/// base, so it is neither `Send` nor `Sync`, and every test here hands one ring
/// half to another thread. What is under test is also different. The header,
/// the offsets and the extent are E0-B13's subject; the subject here is one
/// `Release` store and one `Acquire` load, and a fixture that cannot be laid
/// out wrongly is the right fixture for a question that is not about layout.
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
            // As many completion slots as submission entries, per RFC 0018:
            // a completion ring that can fill is a service that has to drop an
            // answer somebody is waiting for.
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

/// **The load-bearing invariant, for the batch path.**
///
/// A batch publishes N entries with one `Release` store, and stages each of
/// them as *two* writes — the entry, and the index-ring slot naming it. Both
/// are `Relaxed`, and both are covered by that single store: `Release` is a
/// one-way barrier, so no prior write may be observed after it by a thread that
/// `Acquire`-loads the same location.
///
/// That argument is correct and it is also exactly the kind of argument that is
/// correct until somebody edits the code. The indirection is new and the
/// failure it introduces is silent on x86-64: a consumer that reads a stale
/// index-ring slot follows it to a slot the producer has not written yet, and
/// gets a whole, well-formed, *wrong* entry — no tearing, no corruption, no
/// panic. Total store order hides it completely.
///
/// So the payload is self-describing. Each entry carries the value the
/// indirection is supposed to lead to, and a consumer that followed a stale
/// index sees a mismatch rather than a plausible number.
#[test]
fn a_batch_publishes_its_indirection_with_its_entries() {
    const RING: usize = 64;
    const BATCH: u64 = 16;
    const COUNT: u64 = 120_000;

    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let producing = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut producer = Producer::new(shared.chan()).expect("power of two");
            barrier.wait();
            let mut sent = 0u64;
            while sent < COUNT {
                let mut batch = producer.batch();
                while batch.staged() < BATCH as u32 && sent + u64::from(batch.staged()) < COUNT {
                    let value = sent + u64::from(batch.staged());
                    let mut sqe = Sqe::ZERO;
                    sqe.user_data = value;
                    sqe.offset = value;
                    sqe.ext = [value, !value];
                    match batch.push(sqe) {
                        Ok(()) => {}
                        Err(f_ring::RingError::Full) => break,
                        Err(e) => panic!("producer saw {e:?}"),
                    }
                }
                let staged = u64::from(batch.staged());
                batch.publish().expect("publishing must not fail");
                if staged == 0 {
                    std::hint::spin_loop();
                }
                sent += staged;
            }
        })
    };

    let consuming = {
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
                            "STALE INDIRECTION: the entry reached through the index ring is not \
                             the one published at this position. Either the index store in \
                             ring::Producer::stage escaped the Release in Batch::publish, or \
                             the consumer's index load acquired ordering it must not need."
                        );
                        assert_eq!(
                            sqe.offset, sqe.user_data,
                            "TORN PUBLISH: offset does not match user_data. The single Release \
                             store in Batch::publish no longer covers every staged write."
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

    producing.join().expect("producer thread");
    consuming.join().expect("consumer thread");
}

/// **The load-bearing invariant, on the completion ring.**
///
/// RFC 0018 built `Poster`/`Collector` as the mirror of `Producer`/`Consumer`
/// and inherited the ordering argument wholesale. That is the one kind of claim
/// this file exists not to take on faith: every test above it drives the
/// submission half only, so until E0-P17 the completion ring's `Release` store
/// had an argument and no evidence.
///
/// It is not the same code. A completion is 32 bytes rather than 64, it reaches
/// its slot without the index ring's indirection, and the two ends are the
/// other way round — the *service* owns `head` here and the client owns `tail`,
/// which is the reverse of the submission ring. A weakening on this side would
/// therefore not be caught by anything above.
///
/// The payload is self-describing across all four words a completion carries,
/// so a torn post shows up as a mismatch between fields rather than needing an
/// oracle: `result` and `timestamp` restate `user_data`, and `ext` restates its
/// complement. Those span the whole 32 bytes, which is what makes a partially
/// visible completion visible as one.
#[test]
fn posted_completion_is_fully_visible() {
    const RING: usize = 64;
    const COUNT: u64 = 200_000;

    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let posting = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let poster = Poster::new(shared.cq()).expect("power of two");
            barrier.wait();
            let mut posted = 0u64;
            while posted < COUNT {
                let cqe = Cqe {
                    user_data: posted,
                    result: posted as i32,
                    flags: 0,
                    timestamp: posted,
                    ext: !posted,
                };
                match poster.post(cqe) {
                    Ok(()) => posted += 1,
                    Err(f_ring::RingError::Full) => std::hint::spin_loop(),
                    Err(e) => panic!("poster saw {e:?}"),
                }
            }
        })
    };

    let collecting = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let collector = Collector::new(shared.cq()).expect("power of two");
            barrier.wait();
            let mut expect = 0u64;
            while expect < COUNT {
                match collector.take() {
                    Ok(Some(cqe)) => {
                        assert_eq!(
                            cqe.user_data, expect,
                            "completions must arrive in the order they were posted"
                        );
                        assert_eq!(
                            cqe.timestamp, cqe.user_data,
                            "TORN POST: timestamp does not match user_data. The Release store \
                             on the completion head in ring::Poster::post has been weakened, \
                             or a field was written after the cursor was advanced."
                        );
                        assert_eq!(
                            cqe.result, cqe.user_data as i32,
                            "TORN POST: result does not match user_data. See above."
                        );
                        assert_eq!(
                            cqe.ext, !cqe.user_data,
                            "TORN POST: ext does not match user_data. See above."
                        );
                        expect += 1;
                    }
                    Ok(None) => std::hint::spin_loop(),
                    Err(e) => panic!("collector saw {e:?}"),
                }
            }
        })
    };

    posting.join().expect("poster thread");
    collecting.join().expect("collector thread");
}

/// A service must never believe it has more room than the ring has.
///
/// The mirror of `occupancy_never_exceeds_capacity`, and the reason it is a
/// separate test rather than an assertion inside the one above: what is being
/// raced is the *pair of cursors*, and a test that posts and collects on one
/// thread never races them. `free()` reads `head` `Relaxed` and `tail`
/// `Acquire`, which is sound for the owner of `head` and for nobody else — the
/// same asymmetry `occupancy` has, one ring over, and the same intermittent
/// `Corrupt` on a healthy ring if a second `Poster` is ever handed out.
///
/// A number above capacity here is worse than a wrong number. `Service::drain`
/// asks `free()` before it takes work, so an over-count is a service that
/// accepts a submission it cannot answer — and a caller waiting forever for a
/// completion that was dropped.
#[test]
fn free_never_exceeds_capacity() {
    const RING: usize = 16;
    const COUNT: usize = 200_000;

    let shared = Arc::new(Shared::new(RING));
    let barrier = Arc::new(Barrier::new(2));

    let posting = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let poster = Poster::new(shared.cq()).expect("power of two");
            barrier.wait();
            for n in 0..COUNT {
                let mut cqe = Cqe::ZERO;
                cqe.user_data = n as u64;
                let _ = poster.post(cqe);
                match poster.free() {
                    Ok(room) => {
                        assert!(room as usize <= RING, "free {room} exceeds ring capacity {RING}")
                    }
                    Err(e) => panic!("free reported {e:?} on a healthy ring"),
                }
            }
        })
    };

    let collecting = {
        let shared = Arc::clone(&shared);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let collector = Collector::new(shared.cq()).expect("power of two");
            barrier.wait();
            for _ in 0..COUNT {
                let _ = collector.take();
            }
        })
    };

    posting.join().expect("poster thread");
    collecting.join().expect("collector thread");
}

/// A client writing an impossible cursor must produce `Corrupt`.
///
/// The completion ring's untrusted cursor is `tail`, and it is untrusted from
/// the *service's* side — which is the reversal that makes this worth its own
/// test rather than a second loop in `a_hostile_cursor_never_panics`. On the
/// submission ring the kernel disbelieves a client's `head`; here it
/// disbelieves the same client's `tail`, in the one direction where believing
/// it would have the service write outside the slots.
#[test]
fn a_hostile_client_cursor_never_panics() {
    let shared = Shared::new(8);
    let poster = Poster::new(shared.cq()).expect("power of two");
    let collector = Collector::new(shared.cq()).expect("power of two");

    // `u32::MAX` is deliberately not in this list, and its absence is the
    // interesting part. Cursors wrap, so with `head` at zero a `tail` of
    // `u32::MAX` is the *legitimate* state of a ring with one completion
    // outstanding across the wrap — `cursors_may_wrap` asserts exactly that.
    // What is impossible is a difference larger than the ring, and that is what
    // the check tests and all this file may assume. A hostile-cursor test that
    // included `u32::MAX` here would be asserting that a healthy ring is
    // corrupt, and it did: this list was written by copying the submission
    // ring's, where the same value *is* impossible because the roles of the two
    // cursors are the other way round.
    for bad in [1u32, 9, 100, u32::MAX / 2, u32::MAX - 9] {
        // A `tail` further from `head` than the ring is long: the client claims
        // to have reaped completions the service never posted.
        shared.cq_tail.set(bad);
        assert!(
            matches!(poster.free(), Err(f_ring::RingError::Corrupt)),
            "cq tail={bad} must be reported as Corrupt"
        );
        assert!(
            matches!(poster.post(Cqe::ZERO), Err(f_ring::RingError::Corrupt)),
            "cq tail={bad} must be refused rather than written past"
        );
    }

    // And the mirror, so the client is not the only end that is disbelieved: a
    // service cursor claiming more completions than the ring holds must not
    // send the collector to a slot that was never written.
    shared.cq_tail.set(0);
    for bad in [9u32, 100, u32::MAX / 2, u32::MAX] {
        shared.cq_head.set(bad);
        assert!(
            matches!(collector.take(), Err(f_ring::RingError::Corrupt)),
            "cq head={bad} must be reported as Corrupt"
        );
    }
}

/// Line two threads up at the same instant, `arrivals` deep.
///
/// A spin barrier rather than [`std::sync::Barrier`], and the difference is the
/// whole test below. A parked-thread barrier ends in a futex wakeup, which
/// takes microseconds; the window this test has to land in is one store buffer
/// deep, which is nanoseconds. Threads woken from a futex are lined up several
/// thousand times too loosely ever to be inside it together, so the test would
/// report a clean run on a build with the fence removed — a check that cannot
/// fail, which is the failure this tree keeps finding in its own work.
///
/// It also makes the test roughly two hundred times faster, which is how it can
/// afford enough rounds to be worth believing.
fn meet(gate: &AtomicU32, nth: usize) {
    let target = 2 * (nth as u32 + 1);
    gate.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    while gate.load(std::sync::atomic::Ordering::SeqCst) < target {
        std::hint::spin_loop();
    }
}

/// **The lost wakeup.**
///
/// The one place in the ring where the two ends run Dekker's algorithm. The
/// producer stores `head` and then loads `flags`; the consumer stores `flags`
/// and then loads `head`. Store-then-load-of-a-different-location is the single
/// reordering total store order permits, and `Release`/`Acquire` do not forbid
/// it — they are one-way barriers, and this needs a two-way one.
///
/// The bad outcome is both ends seeing nothing. The producer reads `flags`
/// before its own publish is visible and concludes the consumer is awake; the
/// consumer reads `head` before that publish and concludes the ring is empty.
/// Then it sleeps, holding work nobody will ring for. Every value read was
/// legitimately written, so this is not a data race and no sanitiser finds it.
/// It is a hang.
///
/// # Why this one is unlike every other test in this file
///
/// The rest guard a `Release`/`Acquire` pair that x86-64 satisfies for free,
/// which is why the AArch64 job exists and why this file keeps saying so.
/// Store-load is the reordering x86-64 *does* perform, so this defect is
/// visible on an ordinary laptop — and `mutate-no-doorbell-fence` is expected
/// to fail here rather than only on the arm runner.
#[test]
fn a_sleeping_consumer_is_never_left_holding_work() {
    const ROUNDS: usize = 500_000;

    let shared = Arc::new(Shared::new(8));
    let gate = Arc::new(AtomicU32::new(0));
    // What each side saw. Written before the round's closing meet and read
    // after it, so the barrier is what orders them and reading them is not part
    // of the race being measured.
    let rang = Arc::new(AtomicU32::new(0));

    let producing = {
        let shared = Arc::clone(&shared);
        let gate = Arc::clone(&gate);
        let rang = Arc::clone(&rang);
        thread::spawn(move || {
            let producer = Producer::new(shared.chan()).expect("power of two");
            for round in 0..ROUNDS {
                meet(&gate, round * 2);
                let wanted = producer.submit(Sqe::ZERO).expect("a healthy ring");
                rang.store(u32::from(wanted), std::sync::atomic::Ordering::Relaxed);
                meet(&gate, round * 2 + 1);
            }
        })
    };

    let consuming = {
        let shared = Arc::clone(&shared);
        let gate = Arc::clone(&gate);
        let rang = Arc::clone(&rang);
        thread::spawn(move || {
            let consumer = Consumer::new(shared.chan()).expect("power of two");
            let mut lost = 0usize;
            let mut first = ROUNDS;

            for round in 0..ROUNDS {
                // Disarmed outside the window, so that the only store to
                // `flags` inside it is the arming one the algorithm races.
                consumer.disarm_wakeup();

                meet(&gate, round * 2);
                consumer.arm_wakeup();
                let got = consumer.pop().expect("a healthy ring").is_some();
                meet(&gate, round * 2 + 1);

                let rung = rang.load(std::sync::atomic::Ordering::Relaxed) != 0;
                if !rung && !got {
                    lost += 1;
                    first = first.min(round);
                }

                // Reset for the next round. Both threads are past the closing
                // meet and neither touches the ring again until the next one.
                shared.head.set(0);
                shared.tail.set(0);
            }

            // Counted rather than panicked on sight, because the rate is the
            // useful number. It says how many rounds a real system would run
            // before it hung, and a defect that appears once in a hundred
            // thousand is exactly the kind that reaches production.
            assert_eq!(
                lost, 0,
                "LOST WAKEUP in {lost} of {ROUNDS} rounds, first at {first}. The producer \
                 published an entry, read NEED_WAKEUP as clear and rang nothing, while the \
                 consumer armed NEED_WAKEUP, saw an empty ring, and would now sleep. The entry \
                 is stranded and nothing will come for it. The StoreLoad fence in \
                 ring::Producer::doorbell_wanted has been removed or weakened — Release and \
                 Acquire do not forbid this reordering, and it is the one total store order \
                 performs. RFC 0020."
            );
        })
    };

    producing.join().expect("producer thread");
    consuming.join().expect("consumer thread");
}
