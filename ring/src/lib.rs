// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The ring: one primitive for every service in the system.
//!
//! # The two things that must be exactly right
//!
//! **Ordering.** A producer writes the entry, then publishes the cursor with a
//! `Release` store. A consumer reads the cursor with `Acquire`, then reads the
//! entry. That pair is the entire correctness argument. Using `Relaxed` works
//! on x86-64 and fails on AArch64 — the worst available failure mode, because
//! every test on an x86 laptop passes. CI runs both targets for this reason.
//!
//! **Cache-line separation.** Producer and consumer cursors occupy separate
//! cache lines. Sharing one costs roughly 100-150 cycles per operation through
//! false sharing, which turns a 30 ns submission into a 200 ns one. If the
//! first benchmark comes in five times slow, look here before anywhere else.
//!
//! # Shared memory is untrusted input
//!
//! A peer may be an imported driver component built from foreign C. It can
//! fault, hang, restart, or be compromised while holding a live channel. So:
//! no locks in the shared region ever; validate every cursor on every read;
//! indices only, never pointers; read each field exactly once; and never panic
//! on anything a peer wrote.
//!
//! See `docs/design/ring-scene-boot.html` sections 01-06.

#![no_std]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

use f_abi::{Cqe, Sqe, chan};

/// A cursor on its own cache line.
///
/// The padding is load-bearing, not decorative. See the module docs.
#[repr(C, align(64))]
#[derive(Debug)]
pub struct Cursor {
    value: AtomicU32,
    _pad: [u8; 60],
}

impl Cursor {
    /// A zeroed cursor.
    #[must_use]
    pub const fn new() -> Self {
        Self { value: AtomicU32::new(0), _pad: [0; 60] }
    }

    /// Read the raw cursor. For diagnostics and tests only — the protocol
    /// methods below apply the validation this does not.
    #[must_use]
    pub fn raw(&self) -> u32 {
        self.value.load(Ordering::Relaxed)
    }

    /// Force a cursor value. Used to set up a ring and by tests that need to
    /// exercise wrap-around or simulate a hostile peer.
    pub fn set(&self, v: u32) {
        self.value.store(v, Ordering::Release);
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<Cursor>() == 64);

/// Why a channel was refused or torn down.
///
/// Every variant is a condition a peer can cause. None of them is a panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RingError {
    /// No space. The caller retries or sheds load; it never blocks here.
    Full,
    /// The peer cursors are impossible. The channel is corrupt: tear it down,
    /// do not attempt repair.
    Corrupt,
    /// The peer restarted. Outstanding tokens are stale and must be discarded
    /// rather than matched against the new peer completions.
    EpochChanged,
}

/// Shared halves of one channel, borrowed by both ends.
///
/// Held separately from [`Producer`] and [`Consumer`] so the two halves can be
/// handed to different components while pointing at the same mapping.
pub struct Channel<'m> {
    /// Advanced by the producer only.
    pub head: &'m Cursor,
    /// Advanced by the consumer only.
    pub tail: &'m Cursor,
    /// Consumer state, read by the producer to decide whether to ring.
    pub flags: &'m AtomicU32,
    /// The entry array. Length must be a power of two.
    pub entries: &'m [UnsafeCell<Sqe>],
}

impl<'m> Channel<'m> {
    fn mask(&self) -> Option<u32> {
        let len = u32::try_from(self.entries.len()).ok()?;
        if len == 0 || !len.is_power_of_two() {
            return None;
        }
        Some(len - 1)
    }
}

/// Producer half of a single-producer, single-consumer ring.
///
/// One ring per client at a trust boundary. A multi-producer ring is
/// acceptable only among threads that fail together, because a producer that
/// dies mid-claim stalls a slot forever.
pub struct Producer<'m> {
    chan: Channel<'m>,
    mask: u32,
}

// SAFETY: a Producer holds the exclusive right to advance `head` and to write
// the slot that `head` names. It only ever reads `tail`. A Consumer holds the
// mirror-image rights. The two therefore never write the same location, so the
// type is safe to move between threads.
unsafe impl Send for Producer<'_> {}

impl<'m> Producer<'m> {
    /// Bind to a channel mapping.
    ///
    /// Returns `None` rather than panicking on anything a peer could have
    /// written, which includes a non-power-of-two entry count.
    #[must_use]
    pub fn new(chan: Channel<'m>) -> Option<Self> {
        let mask = chan.mask()?;
        Some(Self { chan, mask })
    }

    /// Entries currently queued, or `Corrupt` if the peer cursor is impossible.
    ///
    /// Free-running cursors are allowed to wrap; occupancy is a wrapping
    /// subtraction, which stays correct across the wrap because unsigned
    /// arithmetic wraps the same way. A separate count would be a second piece
    /// of state that could disagree with the cursors, so there is not one.
    ///
    /// # Only the owner may ask
    ///
    /// This is sound for *this* producer and for no other observer. `head` is
    /// read `Relaxed` because the producer is its only writer, and `tail` only
    /// ever advances — so the difference is an over-estimate that still cannot
    /// exceed capacity. A second producer, or any onlooker holding a third
    /// reference to the channel, can read a stale `head` against a fresh
    /// `tail`, underflow the subtraction, and be told `Corrupt` about a
    /// perfectly healthy ring. That is a protocol misuse rather than a bug
    /// here: the ring is single-producer, single-consumer, and this is one of
    /// the places where that stops being a convention and starts mattering.
    pub fn occupancy(&self) -> Result<u32, RingError> {
        let head = self.chan.head.value.load(Ordering::Relaxed);
        let tail = self.chan.tail.value.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        if used > self.mask + 1 { Err(RingError::Corrupt) } else { Ok(used) }
    }

    /// Publish one entry. The entire hot path.
    ///
    /// Returns `Ok(true)` when the caller should ring the doorbell, which is
    /// true only when the consumer said it was about to sleep. Under load this
    /// is `false` every time and the channel becomes pure polling.
    pub fn submit(&self, entry: Sqe) -> Result<bool, RingError> {
        let head = self.chan.head.value.load(Ordering::Relaxed);
        let tail = self.chan.tail.value.load(Ordering::Acquire);

        let used = head.wrapping_sub(tail);
        if used > self.mask + 1 {
            return Err(RingError::Corrupt);
        }
        if used == self.mask + 1 {
            return Err(RingError::Full);
        }

        let slot = (head & self.mask) as usize;
        // SAFETY: `slot` is masked into range, and this producer holds the sole
        // right to write the slot named by `head` until it publishes below. The
        // consumer cannot observe this slot before the Release store makes the
        // new `head` visible to it.
        unsafe {
            self.chan.entries[slot].get().write(entry);
        }

        // Publishes the write above. Paired with the consumer Acquire load,
        // this is a complete happens-before edge. Never weaken it to Relaxed:
        // it will pass every test on x86 and corrupt data on AArch64.
        self.chan.head.value.store(head.wrapping_add(1), Ordering::Release);

        Ok(self.chan.flags.load(Ordering::Acquire) & chan::NEED_WAKEUP != 0)
    }
}

/// Consumer half of a single-producer, single-consumer ring.
pub struct Consumer<'m> {
    chan: Channel<'m>,
    mask: u32,
}

// SAFETY: mirror of the Producer argument. This half owns `tail` and reads only
// slots the producer has already published via its Release store.
unsafe impl Send for Consumer<'_> {}

impl<'m> Consumer<'m> {
    /// Bind to a channel mapping. See [`Producer::new`].
    #[must_use]
    pub fn new(chan: Channel<'m>) -> Option<Self> {
        let mask = chan.mask()?;
        Some(Self { chan, mask })
    }

    /// Take one entry, copying it out before anything is validated.
    ///
    /// The copy is deliberate. Validating in place and then reading again lets
    /// a peer change the value between the two — the classic time-of-check to
    /// time-of-use bug, and it appears in almost every first implementation.
    pub fn pop(&self) -> Result<Option<Sqe>, RingError> {
        let tail = self.chan.tail.value.load(Ordering::Relaxed);
        let head = self.chan.head.value.load(Ordering::Acquire);

        let used = head.wrapping_sub(tail);
        if used > self.mask + 1 {
            return Err(RingError::Corrupt);
        }
        if used == 0 {
            return Ok(None);
        }

        let slot = (tail & self.mask) as usize;
        // SAFETY: `slot` is masked into range. The producer Release store on
        // `head`, observed by the Acquire load above, guarantees the write to
        // this slot happened before this read. It is read exactly once and
        // copied out before any field is examined.
        let entry = unsafe { self.chan.entries[slot].get().read() };

        self.chan.tail.value.store(tail.wrapping_add(1), Ordering::Release);
        Ok(Some(entry))
    }

    /// Declare that this consumer is about to sleep.
    ///
    /// The caller **must** poll once more after this returns before actually
    /// sleeping. That second check closes the race where a producer published
    /// between the last drain and this flag becoming visible.
    pub fn arm_wakeup(&self) {
        self.chan.flags.fetch_or(chan::NEED_WAKEUP, Ordering::SeqCst);
    }

    /// Declare that this consumer is actively draining and needs no doorbell.
    pub fn disarm_wakeup(&self) {
        self.chan.flags.fetch_and(!chan::NEED_WAKEUP, Ordering::SeqCst);
    }
}

/// Build a completion for an operation that carries no payload.
#[must_use]
pub const fn completion(user_data: u64, result: i32, timestamp: u64) -> Cqe {
    Cqe { user_data, result, flags: 0, timestamp, ext: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed-size backing so the tests need no allocator, and so this module
    // stays buildable in a `no_std` test configuration later.
    struct Backing<const N: usize> {
        head: Cursor,
        tail: Cursor,
        flags: AtomicU32,
        entries: [UnsafeCell<Sqe>; N],
    }

    impl<const N: usize> Backing<N> {
        fn new() -> Self {
            Self {
                head: Cursor::new(),
                tail: Cursor::new(),
                flags: AtomicU32::new(0),
                entries: [const { UnsafeCell::new(Sqe::ZERO) }; N],
            }
        }

        fn chan(&self) -> Channel<'_> {
            Channel {
                head: &self.head,
                tail: &self.tail,
                flags: &self.flags,
                entries: &self.entries,
            }
        }
    }

    fn entry(id: u64) -> Sqe {
        let mut s = Sqe::ZERO;
        s.user_data = id;
        s
    }

    #[test]
    fn round_trip_preserves_order() {
        let b = Backing::<8>::new();
        let p = Producer::new(b.chan()).unwrap();
        let c = Consumer::new(b.chan()).unwrap();

        for i in 0..8u64 {
            p.submit(entry(i)).unwrap();
        }
        for i in 0..8u64 {
            assert_eq!(c.pop().unwrap().unwrap().user_data, i);
        }
        assert!(c.pop().unwrap().is_none());
    }

    #[test]
    fn full_is_reported_not_overwritten() {
        let b = Backing::<4>::new();
        let p = Producer::new(b.chan()).unwrap();
        for _ in 0..4 {
            p.submit(Sqe::ZERO).unwrap();
        }
        assert_eq!(p.submit(Sqe::ZERO), Err(RingError::Full));
    }

    #[test]
    fn cursors_may_wrap() {
        let b = Backing::<4>::new();
        // Start near the u32 wrap so the arithmetic is actually exercised.
        b.head.set(u32::MAX - 1);
        b.tail.set(u32::MAX - 1);
        let p = Producer::new(b.chan()).unwrap();
        let c = Consumer::new(b.chan()).unwrap();

        for i in 0..16u64 {
            p.submit(entry(i)).unwrap();
            assert_eq!(c.pop().unwrap().unwrap().user_data, i);
        }
    }

    #[test]
    fn a_hostile_cursor_is_reported_not_trusted() {
        let b = Backing::<4>::new();
        let c = Consumer::new(b.chan()).unwrap();
        let p = Producer::new(b.chan()).unwrap();

        // A peer claims to have published far more than the ring can hold.
        b.head.set(9999);
        // `matches!` rather than `assert_eq!`: comparing the Ok side would
        // require `Sqe: PartialEq`, and what the wire types derive is an ABI
        // decision rather than something a test reaches in and takes.
        assert!(matches!(c.pop(), Err(RingError::Corrupt)));
        assert_eq!(p.occupancy(), Err(RingError::Corrupt));
    }

    #[test]
    fn non_power_of_two_is_refused() {
        let b = Backing::<6>::new();
        assert!(Producer::new(b.chan()).is_none());
        assert!(Consumer::new(b.chan()).is_none());
    }

    #[test]
    fn doorbell_only_when_consumer_armed() {
        let b = Backing::<8>::new();
        let p = Producer::new(b.chan()).unwrap();
        let c = Consumer::new(b.chan()).unwrap();

        c.disarm_wakeup();
        assert!(!p.submit(Sqe::ZERO).unwrap(), "draining consumer needs no doorbell");

        c.arm_wakeup();
        assert!(p.submit(Sqe::ZERO).unwrap(), "sleeping consumer must be woken");
    }
}
