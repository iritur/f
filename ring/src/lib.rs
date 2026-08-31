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

use f_abi::{Cqe, Sqe, chan, error, flags, op};

mod doorbell;
mod mapping;

pub use doorbell::{Bell, Hardware, Path, Ringer, Silent};
pub use mapping::Mapping;

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
    /// Which entry each queued position names. Same length as `entries`.
    ///
    /// # Why the indirection is here and not optimised away
    ///
    /// Today a producer allocates entries in order, so this is the identity
    /// mapping and a reader is entitled to ask what it is for. Three answers,
    /// in increasing order of how much they matter.
    ///
    /// It is the layout, and the layout is wire. `f_abi::layout` places this
    /// ring between the cursors and the entry array because section 02 does;
    /// removing it here would leave a hole in the mapping that a peer built
    /// from the specification would write into.
    ///
    /// It is what lets a producer prepare entries out of order and publish a
    /// batch with one store — the reason io_uring has it. Nothing at this
    /// milestone allocates entries out of order, because nothing above the ring
    /// manages the entry array as a pool yet. When something does, it arrives
    /// without an ABI change.
    ///
    /// And it is one more thing the consumer has to *not trust*. A slot number
    /// read out of shared memory is untrusted input exactly like a cursor is,
    /// and [`Consumer::pop`] bounds-checks it on every entry. That check is not
    /// theatre for an identity mapping: it is the check that stays correct when
    /// the mapping stops being the identity.
    pub index: &'m [AtomicU32],
    /// The entry array. Length must be a power of two.
    pub entries: &'m [UnsafeCell<Sqe>],
}

impl<'m> Channel<'m> {
    fn mask(&self) -> Option<u32> {
        let len = u32::try_from(self.entries.len()).ok()?;
        if len == 0 || !len.is_power_of_two() {
            return None;
        }
        // A mapping whose index ring and entry array disagree about their
        // length is one where every slot number is checked against the wrong
        // bound. Refused at bind time rather than checked per entry.
        if self.index.len() != self.entries.len() {
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

        self.stage(head, entry);

        // Publishes the writes above. Paired with the consumer Acquire load,
        // this is a complete happens-before edge. Never weaken it to Relaxed:
        // it will pass every test on x86 and corrupt data on AArch64.
        #[cfg(not(feature = "mutate-relaxed-submission"))]
        self.chan.head.value.store(head.wrapping_add(1), Ordering::Release);

        // The deliberate defect the sentence above warns about, made runnable.
        // E0-P16's exit names exactly this weakening as what a model checker
        // must catch; the checker does not exist, and until it does the
        // AArch64 litmus job is what requires the suite to notice. That is
        // evidence about the stress suite and not about what RC11 permits, and
        // it should be described as exactly that.
        #[cfg(feature = "mutate-relaxed-submission")]
        self.chan.head.value.store(head.wrapping_add(1), Ordering::Relaxed);

        Ok(self.doorbell_wanted())
    }

    /// Prepare several entries and make them visible with one store.
    ///
    /// [`Producer::submit`] is this with one entry, and the difference is the
    /// whole reason batching exists: `submit` pays a `Release` store and a
    /// `flags` load per entry, and a batch pays them once for the lot. The
    /// diagnosis section of `claims/0001-ring-submit-latency.toml` names a flat
    /// response to batch size as a symptom with exactly one cause — *you are
    /// publishing per entry* — so a workload measuring that claim must go
    /// through here.
    ///
    /// Takes `&mut self` so that a `submit` cannot interleave with a batch that
    /// is still being filled. Nothing about the ring makes that unsound; it
    /// makes it a compile error instead of a rule in this paragraph.
    pub fn batch(&mut self) -> Batch<'_, 'm> {
        let base = self.chan.head.value.load(Ordering::Relaxed);
        Batch { producer: self, base, staged: 0 }
    }

    /// Write one entry into the slot a cursor position names, without
    /// publishing it.
    ///
    /// The index ring is written alongside the entry, because both must be
    /// visible to the consumer before the cursor that exposes them and both are
    /// covered by the same `Release` store.
    fn stage(&self, at: u32, entry: Sqe) {
        let slot = (at & self.mask) as usize;

        // SAFETY: `slot` is masked into range, and this producer holds the sole
        // right to write the slot named by an unpublished cursor position. The
        // consumer cannot observe either write before the Release store on
        // `head` makes the new position visible to it.
        unsafe {
            self.chan.entries[slot].get().write(entry);
        }

        // Relaxed: this store carries no ordering of its own, and needs none.
        // The `Release` on `head` that eventually publishes this position
        // orders it, exactly as it orders the entry write above. An `Ordering`
        // here would be a second barrier paying for the same edge.
        //
        // The value is the slot the entry went into. It is the identity today
        // because entries are allocated in cursor order; the consumer is not
        // told that and checks it regardless.
        #[allow(clippy::cast_possible_truncation)]
        self.chan.index[slot].store(slot as u32, Ordering::Relaxed);
    }

    /// Has the consumer asked to be woken?
    ///
    /// `Acquire` because a `true` answer is about to be acted on by ringing a
    /// doorbell, and the consumer's decision to sleep must not be observed
    /// before whatever it did to prepare for sleeping.
    ///
    /// # The fence, and why this is the only place in the ring with one
    ///
    /// The two ends run Dekker's algorithm here and nowhere else. The producer
    /// **stores** `head` and then **loads** `flags`; the consumer **stores**
    /// `flags` and then **loads** `head`. Each is a store to one location
    /// followed by a load of a *different* one, and that is the single
    /// reordering total store order permits — the store sits in the store
    /// buffer while the load is satisfied ahead of it. `Release` and `Acquire`
    /// do not forbid it; they are one-way barriers and this needs a two-way
    /// one.
    ///
    /// Without the fence, both sides can look and see nothing: the producer
    /// reads `flags` before its own publish is visible and concludes the
    /// consumer is awake, while the consumer reads `head` before that publish
    /// and concludes the ring is empty. The consumer sleeps holding work
    /// nobody will ring for. That is a **lost wakeup**, and it is not a data
    /// race — every value read is a value that was legitimately written. It is
    /// a hang.
    ///
    /// The consumer's half of the barrier already exists:
    /// [`Consumer::arm_wakeup`] is a `SeqCst` read-modify-write, which is a
    /// full barrier. This is the missing half. RFC 0020, and unlike the
    /// `Release`/`Acquire` pair this one is observable on x86-64 — store-load
    /// is exactly what that architecture reorders — so
    /// `a_sleeping_consumer_is_never_left_holding_work` catches it here rather
    /// than only on the arm runner.
    fn doorbell_wanted(&self) -> bool {
        #[cfg(not(feature = "mutate-no-doorbell-fence"))]
        core::sync::atomic::fence(Ordering::SeqCst);

        self.chan.flags.load(Ordering::Acquire) & chan::NEED_WAKEUP != 0
    }
}

/// Entries prepared but not yet visible to the consumer.
///
/// Dropping one without calling [`Batch::publish`] discards it, and that is
/// sound rather than merely tolerated: nothing was published, so the consumer
/// never saw a cursor position naming any of the staged slots, and the next
/// batch overwrites them. There is no partial state to unwind, which is a
/// property of publishing being a single store and is worth having on purpose.
pub struct Batch<'p, 'm> {
    producer: &'p Producer<'m>,
    /// The cursor position this batch starts at.
    base: u32,
    /// How many entries have been staged into it.
    staged: u32,
}

impl Batch<'_, '_> {
    /// Stage one more entry.
    ///
    /// # Errors
    ///
    /// [`RingError::Full`] when the ring cannot hold another entry, counting
    /// the ones already staged in this batch — a batch that overran would
    /// overwrite entries the consumer has not read. [`RingError::Corrupt`] for
    /// a peer cursor that is impossible.
    pub fn push(&mut self, entry: Sqe) -> Result<(), RingError> {
        let tail = self.producer.chan.tail.value.load(Ordering::Acquire);
        let capacity = self.producer.mask + 1;

        // Occupancy counted from the *end* of what this batch has staged, not
        // from the published head. The staged entries are invisible to the
        // consumer and so cannot be drained; treating them as free is how a
        // batch overwrites its own start.
        let used = self.base.wrapping_add(self.staged).wrapping_sub(tail);
        if used > capacity {
            return Err(RingError::Corrupt);
        }
        if used == capacity {
            return Err(RingError::Full);
        }

        self.producer.stage(self.base.wrapping_add(self.staged), entry);
        self.staged += 1;
        Ok(())
    }

    /// How many entries are staged. Unit: entries.
    #[must_use]
    pub fn staged(&self) -> u32 {
        self.staged
    }

    /// Make every staged entry visible, with one `Release` store.
    ///
    /// Returns whether the caller should ring the doorbell — once for the
    /// batch, not once per entry, which is the other half of what batching is
    /// for.
    ///
    /// # Errors
    ///
    /// Never, today. The signature returns a `Result` because the doorbell
    /// answer is read from a peer-written word, and the day that word grows a
    /// value worth refusing this becomes the place that refuses it.
    pub fn publish(self) -> Result<bool, RingError> {
        if self.staged == 0 {
            // No store at all. A `Release` here would be a barrier publishing
            // nothing, and an empty batch is an ordinary outcome — a drain loop
            // that found no work is not an error.
            return Ok(false);
        }

        // The one store the whole batch pays for. Everything staged above is
        // ordered before it, on every architecture.
        #[cfg(not(feature = "mutate-relaxed-submission"))]
        self.producer.chan.head.value.store(self.base.wrapping_add(self.staged), Ordering::Release);

        // The batch path's half of the same defect. Both, because the batch is
        // where the indirection's two relaxed writes per entry are covered by
        // one store — the weakening that is hardest to reason about and the one
        // `a_batch_publishes_its_indirection_with_its_entries` exists for.
        #[cfg(feature = "mutate-relaxed-submission")]
        self.producer.chan.head.value.store(self.base.wrapping_add(self.staged), Ordering::Relaxed);

        Ok(self.producer.doorbell_wanted())
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

        // Which entry this queued position names. `Relaxed` is correct and not
        // a shortcut: the `Acquire` load of `head` above already orders every
        // write the producer made before its `Release` store, and this is one
        // of them. A second acquire would pay twice for one edge.
        let position = (tail & self.mask) as usize;
        let slot = self.chan.index[position].load(Ordering::Relaxed);

        // Untrusted input, and the reason the indirection is not free. A peer
        // that writes a slot number past the end of the array is asking this
        // side to read whatever follows it in the mapping. Refused, and the
        // channel is corrupt rather than repairable.
        if slot as usize >= self.chan.entries.len() {
            return Err(RingError::Corrupt);
        }
        let slot = slot as usize;

        // SAFETY: `slot` was just bounds-checked against the array's length.
        // The producer Release store on `head`, observed by the Acquire load
        // above, guarantees the write to this slot happened before this read.
        // It is read exactly once and copied out before any field is examined.
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

/// Build a completion carrying a refusal and the detail that names it.
///
/// `packed` is an [`f_abi::error`] result and `detail` is the per-domain
/// detail RFC 0010 says every refusal carries — the offending field, the
/// offending opcode, the offending index. A refusal with no detail is a
/// refusal the caller cannot act on.
#[must_use]
pub const fn refusal(user_data: u64, packed: i32, detail: u64, timestamp: u64) -> Cqe {
    Cqe { user_data, result: packed, flags: 0, timestamp, ext: detail }
}

/// Shared halves of one completion ring.
///
/// The mirror of [`Channel`], with the roles swapped: the service advances
/// `head` and the client advances `tail`. There is no index ring, because
/// section 02 places completions inline — a completion is half a cache line and
/// an indirection would cost more than it saved.
pub struct Completions<'m> {
    /// Advanced by the service, which posts completions.
    pub head: &'m Cursor,
    /// Advanced by the client, which reaps them.
    pub tail: &'m Cursor,
    /// The completion array. Length must be a power of two.
    pub slots: &'m [UnsafeCell<Cqe>],
}

impl Completions<'_> {
    fn mask(&self) -> Option<u32> {
        let len = u32::try_from(self.slots.len()).ok()?;
        if len == 0 || !len.is_power_of_two() {
            return None;
        }
        Some(len - 1)
    }
}

/// The service's end of a completion ring: the side that posts.
pub struct Poster<'m> {
    cq: Completions<'m>,
    mask: u32,
}

// SAFETY: the same argument as `Producer`, with the roles swapped. A Poster
// holds the exclusive right to advance `head` and to write the slot it names,
// and only ever reads `tail`.
unsafe impl Send for Poster<'_> {}

impl<'m> Poster<'m> {
    /// Bind to a completion ring. `None` for a length that is not a power of
    /// two, which is something a peer could have written.
    #[must_use]
    pub fn new(cq: Completions<'m>) -> Option<Self> {
        let mask = cq.mask()?;
        Some(Self { cq, mask })
    }

    /// Completions this ring can still take.
    ///
    /// The number a service checks *before* taking work, which is the whole
    /// reason it is public. A service that pops a submission it has no room to
    /// complete has to drop the completion, block, or grow a queue of its own —
    /// and the first of those is a caller waiting forever for an answer that
    /// was thrown away.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] for a client cursor that is impossible.
    pub fn free(&self) -> Result<u32, RingError> {
        let head = self.cq.head.value.load(Ordering::Relaxed);
        let tail = self.cq.tail.value.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        if used > self.mask + 1 { Err(RingError::Corrupt) } else { Ok(self.mask + 1 - used) }
    }

    /// Post one completion.
    ///
    /// # Errors
    ///
    /// [`RingError::Full`] when the client has not reaped, and
    /// [`RingError::Corrupt`] for an impossible client cursor.
    pub fn post(&self, cqe: Cqe) -> Result<(), RingError> {
        if self.free()? == 0 {
            return Err(RingError::Full);
        }

        let head = self.cq.head.value.load(Ordering::Relaxed);
        let slot = (head & self.mask) as usize;

        // SAFETY: `slot` is masked into range, and this poster holds the sole
        // right to write the slot `head` names until the store below publishes
        // it. The client cannot observe the slot before that store.
        unsafe {
            self.cq.slots[slot].get().write(cqe);
        }

        // The same Release that the submission ring depends on, for the same
        // reason and with the same prohibition on weakening it.
        #[cfg(not(feature = "mutate-relaxed-completion"))]
        self.cq.head.value.store(head.wrapping_add(1), Ordering::Release);

        // The deliberate defect, and the reason it exists: RFC 0018 inherited
        // this ordering argument from the submission ring rather than proving
        // it, and a litmus test that has only ever passed cannot be told apart
        // from one that cannot fail. `posted_completion_is_fully_visible` is
        // the test; this is what makes it fail. It is invisible on x86-64,
        // where total store order hides it, which is why the CI job that
        // requires the failure runs only on the AArch64 runner.
        #[cfg(feature = "mutate-relaxed-completion")]
        self.cq.head.value.store(head.wrapping_add(1), Ordering::Relaxed);

        Ok(())
    }
}

/// The client's end of a completion ring: the side that reaps.
pub struct Collector<'m> {
    cq: Completions<'m>,
    mask: u32,
}

// SAFETY: mirror of the Poster argument.
unsafe impl Send for Collector<'_> {}

impl<'m> Collector<'m> {
    /// Bind to a completion ring. See [`Poster::new`].
    #[must_use]
    pub fn new(cq: Completions<'m>) -> Option<Self> {
        let mask = cq.mask()?;
        Some(Self { cq, mask })
    }

    /// Take one completion, copying it out before anything is examined.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] for a service cursor that is impossible.
    pub fn take(&self) -> Result<Option<Cqe>, RingError> {
        let tail = self.cq.tail.value.load(Ordering::Relaxed);
        let head = self.cq.head.value.load(Ordering::Acquire);

        let used = head.wrapping_sub(tail);
        if used > self.mask + 1 {
            return Err(RingError::Corrupt);
        }
        if used == 0 {
            return Ok(None);
        }

        let slot = (tail & self.mask) as usize;
        // SAFETY: `slot` is masked into range. The service's Release store on
        // `head`, observed by the Acquire load above, orders the write to this
        // slot before this read. Copied out whole, once.
        let cqe = unsafe { self.cq.slots[slot].get().read() };

        self.cq.tail.value.store(tail.wrapping_add(1), Ordering::Release);
        Ok(Some(cqe))
    }
}

/// The inline arena: the payload area of a channel mapping.
///
/// # Why every read goes through here
///
/// Because this is the one region of the mapping a peer may be writing *while*
/// the service reads it, and it is the region with no protocol governing when.
/// The entry array has the cursor protocol: an entry the consumer can see is
/// one the producer has finished with. The arena has nothing of the kind — a
/// peer can scribble over the bytes an in-flight operation named, and a
/// correct one that crashed mid-write leaves half a payload behind.
///
/// So the arena is never borrowed as a slice and never read twice. Bytes are
/// copied out volatile into the service's own memory, and everything after that
/// works on the copy. What the service acts on is then *a* value the peer wrote
/// at some point, which is the strongest statement available and is enough:
/// nothing here interprets the bytes, and an opcode that did would still be
/// reasoning about its own copy rather than about memory somebody else holds.
#[derive(Clone, Copy)]
pub struct Arena<'m>(&'m [UnsafeCell<u8>]);

impl<'m> Arena<'m> {
    /// Wrap the arena region of a mapping.
    #[must_use]
    pub const fn new(bytes: &'m [UnsafeCell<u8>]) -> Self {
        Self(bytes)
    }

    /// Bytes in the arena. Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Is the arena empty? A channel whose opcodes carry everything inline.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Copy `out.len()` bytes starting at `offset` into `out`.
    ///
    /// Returns `false` without touching `out` when the range is not wholly
    /// inside the arena. The caller has already been refused at that point;
    /// this is the second bound rather than the first, and it is here so that
    /// an opcode which forgot to check cannot read past the region.
    #[must_use]
    pub fn copy_out(&self, offset: usize, out: &mut [u8]) -> bool {
        let Some(end) = offset.checked_add(out.len()) else { return false };
        if end > self.0.len() {
            return false;
        }
        for (i, byte) in out.iter_mut().enumerate() {
            // SAFETY: `offset + i` is inside the slice, checked above. Volatile
            // because a peer may be writing this byte concurrently: the read is
            // not elided, not merged with another, and not repeated. The value
            // is whatever the peer had written by then, which is exactly the
            // guarantee the module docs claim and no more.
            *byte = unsafe { self.0[offset + i].get().read_volatile() };
        }
        true
    }
}

/// Where [`op::WRITE_SERIAL`] sends its bytes.
///
/// A trait rather than a concrete port so that the frame's serial driver, a
/// test's buffer and — later — a component's own sink are the same code path.
/// The service is the interesting part and it should not be reimplemented per
/// destination.
pub trait Sink {
    /// Write as much of `bytes` as the destination will take, and answer how
    /// much that was.
    ///
    /// A short answer is a partial write and is reported as one: the completion
    /// carries the count actually written, which is what
    /// [`Cqe::result`](f_abi::Cqe::result) means on the non-negative side.
    /// Returning less than `bytes.len()` is not an error and must not be
    /// treated as one by an implementation that is merely busy.
    fn write(&mut self, bytes: &[u8]) -> usize;
}

/// The largest payload copied out of the arena in one go.
///
/// Section 02 sizes the inline arena for "payloads under ~256 bytes", so this
/// is one payload's worth of stack. A longer write is not refused; it is copied
/// in several passes, which costs a loop and keeps the service's stack a
/// constant that a bare-metal caller can reason about.
const CHUNK: usize = 256;

/// Every submission flag this build knows.
///
/// R04 refuses an unknown flag rather than ignoring it, and that check needs
/// something to compare against. Kept beside the executor so that adding a flag
/// without teaching the executor about it is a test failure rather than a bit
/// that is silently accepted.
const KNOWN_FLAGS: u8 = flags::LINK | flags::DRAIN | flags::FIXED_BUF | flags::NO_CQE;

/// Execute one submission and produce the completion it earns.
///
/// `now` is the timestamp to stamp the completion with, on the monotonic clock
/// [`Sqe::deadline`](f_abi::Sqe::deadline) is measured against. It is passed in
/// rather than read, because this crate observes no clock: RFC 0004, and the
/// determinism lint would refuse a call to one.
///
/// `None` means no completion is owed — [`flags::NO_CQE`] on an entry that
/// succeeded. A refusal always completes, whatever the flags say, and the
/// reason is in that flag's own documentation.
///
/// # The order of the checks
///
/// The envelope before the operation: reserved field, then flags, then opcode.
/// An entry with a non-zero reserved word is malformed whatever it claims to
/// be, and reporting the opcode first would tell a caller its opcode was wrong
/// when it was not.
pub fn execute<S: Sink>(entry: &Sqe, arena: &Arena<'_>, sink: &mut S, now: u64) -> Option<Cqe> {
    let refuse = |domain: u8, code: u16, detail: u64| {
        Some(refusal(entry.user_data, error::pack(domain, code), detail, now))
    };

    if entry._reserved != 0 {
        return refuse(
            error::ARGUMENT,
            error::argument::RESERVED_NOT_ZERO,
            u64::from(entry._reserved),
        );
    }

    let unknown = entry.flags & !KNOWN_FLAGS;
    if unknown != 0 {
        return refuse(error::ARGUMENT, error::argument::UNKNOWN_FLAG, u64::from(unknown));
    }

    if !op::known(entry.opcode) {
        return refuse(error::ARGUMENT, error::argument::UNKNOWN_OPCODE, u64::from(entry.opcode));
    }

    // Neither opcode names an object, so `cap` is not read — which `Sqe::cap`
    // already says is what an absent capability looks like. The first opcode
    // that names one arrives with the table plumbing behind it.
    //
    // Neither reads `class` or `deadline` either: the frame executes in arrival
    // order at this milestone. Ordering by deadline is the scheduler's, and an
    // executor that pretended to honour a deadline it does not look at would be
    // worse than one that plainly does not.
    let result = match entry.opcode {
        op::NOP => 0,
        op::WRITE_SERIAL => match write_serial(entry, arena, sink) {
            Ok(written) => written,
            Err((code, detail)) => return refuse(error::ARGUMENT, code, detail),
        },
        // Unreachable: `op::known` above admits exactly the two arms. Not a
        // panic — this crate's contract is that nothing a peer writes produces
        // one, and an opcode is the most peer-controlled field there is.
        _ => {
            return refuse(
                error::ARGUMENT,
                error::argument::UNKNOWN_OPCODE,
                u64::from(entry.opcode),
            );
        }
    };

    if entry.flags & flags::NO_CQE != 0 {
        return None;
    }
    Some(completion(entry.user_data, result, now))
}

/// [`op::WRITE_SERIAL`]. `Err` is a code and its detail.
fn write_serial<S: Sink>(entry: &Sqe, arena: &Arena<'_>, sink: &mut S) -> Result<i32, (u16, u64)> {
    // A count that cannot be stated in the completion is refused rather than
    // truncated. `Cqe::result` is an `i32` and this is the one place where the
    // width of a wire field becomes a bound on what may be asked for.
    if entry.len > i32::MAX as u32 {
        return Err((error::argument::BAD_ADDRESS, u64::from(entry.len)));
    }

    let Ok(offset) = usize::try_from(entry.offset) else {
        return Err((error::argument::BAD_ADDRESS, entry.offset));
    };
    let len = entry.len as usize;
    let within = offset.checked_add(len).is_some_and(|end| end <= arena.len());
    if !within {
        return Err((error::argument::BAD_ADDRESS, entry.offset));
    }

    let mut buffer = [0u8; CHUNK];
    let mut written = 0usize;
    while written < len {
        let take = core::cmp::min(CHUNK, len - written);
        let chunk = &mut buffer[..take];
        if !arena.copy_out(offset + written, chunk) {
            // Unreachable given the bound above, and refused rather than
            // asserted: the arena's own check is the one that is still correct
            // if this function's arithmetic ever stops being.
            return Err((error::argument::BAD_ADDRESS, entry.offset));
        }

        let accepted = sink.write(chunk);
        written += accepted;
        if accepted < take {
            // A short write is a partial completion, not a failure. Stop here
            // and report the count: a sink that took less than it was offered
            // will not take more by being asked again in the same breath.
            break;
        }
    }

    // Fits an `i32` because `len` did, checked above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    Ok(written as i32)
}

/// What one call to [`Service::drain`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Drained {
    /// Submissions taken off the ring. Unit: entries.
    pub executed: u32,
    /// Completions posted. Unit: entries. Lower than `executed` exactly when an
    /// entry carried [`flags::NO_CQE`] and succeeded.
    pub completed: u32,
    /// Entries refused rather than executed. Unit: entries.
    pub refused: u32,
}

/// One end of one channel, executing the frame's opcodes.
///
/// Holds the consumer of the submission ring and the poster of the completion
/// ring — which is one side of the channel, not both, and so keeps the
/// single-producer single-consumer discipline the whole protocol rests on.
pub struct Service<'m, S: Sink> {
    submissions: Consumer<'m>,
    completions: Poster<'m>,
    arena: Arena<'m>,
    sink: S,
}

impl<'m, S: Sink> Service<'m, S> {
    /// Bind a service to the two rings and the arena of one channel.
    pub const fn new(
        submissions: Consumer<'m>,
        completions: Poster<'m>,
        arena: Arena<'m>,
        sink: S,
    ) -> Self {
        Self { submissions, completions, arena, sink }
    }

    /// The destination [`op::WRITE_SERIAL`] writes to.
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    /// Execute up to `budget` entries, and answer what happened.
    ///
    /// # Why there is a budget
    ///
    /// Because the alternative is a loop whose length a peer chooses. A service
    /// that drains until the ring is empty can be held on this core for as long
    /// as a producer keeps submitting, which is a denial of service written as
    /// a `while` loop. The budget is what makes the time this call takes a
    /// property of the caller rather than of the peer.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] when either ring's peer cursor is impossible. The
    /// channel is torn down at that point and not repaired.
    pub fn drain(&mut self, budget: u32, now: u64) -> Result<Drained, RingError> {
        let mut done = Drained::default();

        for _ in 0..budget {
            // Room to answer, before taking the question. An entry popped and
            // then not completed is a caller waiting forever for a reply that
            // was dropped on the floor, which is the one failure a ring must
            // not have.
            if self.completions.free()? == 0 {
                break;
            }

            let Some(entry) = self.submissions.pop()? else { break };
            done.executed += 1;

            if let Some(cqe) = execute(&entry, &self.arena, &mut self.sink, now) {
                if cqe.is_error() {
                    done.refused += 1;
                }
                self.completions.post(cqe)?;
                done.completed += 1;
            }
        }

        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed-size backing so the tests need no allocator, and so this module
    // stays buildable in a `no_std` test configuration later.
    //
    // `pub(crate)` because `doorbell`'s tests drive a real ring through a real
    // producer and consumer, and a second fixture there would be a second
    // opinion about what a well-formed channel is.
    pub(crate) struct Backing<const N: usize> {
        head: Cursor,
        tail: Cursor,
        flags: AtomicU32,
        index: [AtomicU32; N],
        entries: [UnsafeCell<Sqe>; N],
        cq_head: Cursor,
        cq_tail: Cursor,
        slots: [UnsafeCell<Cqe>; N],
        arena: [UnsafeCell<u8>; ARENA],
    }

    /// Enough arena for a payload that spans more than one [`CHUNK`], so the
    /// chunking loop in `write_serial` is exercised rather than assumed.
    const ARENA: usize = CHUNK * 2 + 16;

    impl<const N: usize> Backing<N> {
        pub(crate) fn new() -> Self {
            Self {
                head: Cursor::new(),
                tail: Cursor::new(),
                flags: AtomicU32::new(0),
                index: [const { AtomicU32::new(0) }; N],
                entries: [const { UnsafeCell::new(Sqe::ZERO) }; N],
                cq_head: Cursor::new(),
                cq_tail: Cursor::new(),
                slots: [const {
                    UnsafeCell::new(Cqe { user_data: 0, result: 0, flags: 0, timestamp: 0, ext: 0 })
                }; N],
                arena: [const { UnsafeCell::new(0) }; ARENA],
            }
        }

        pub(crate) fn chan(&self) -> Channel<'_> {
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

        fn arena(&self) -> Arena<'_> {
            Arena::new(&self.arena)
        }

        /// Fill the arena with a payload, the way a peer would.
        fn fill(&self, at: usize, bytes: &[u8]) {
            for (i, b) in bytes.iter().enumerate() {
                // SAFETY: the test owns the arena and no service is running.
                unsafe { self.arena[at + i].get().write(*b) };
            }
        }
    }

    fn entry(id: u64) -> Sqe {
        let mut s = Sqe::ZERO;
        s.user_data = id;
        s
    }

    /// A sink that keeps what it was given, and can be told to take less.
    ///
    /// Fixed capacity and no allocator, because this crate is `no_std` and a
    /// test that needed one would be testing a configuration the kernel never
    /// builds.
    struct Recorder {
        seen: [u8; ARENA],
        len: usize,
        /// Total bytes this sink will accept before going short.
        limit: usize,
    }

    impl Recorder {
        fn new() -> Self {
            Self { seen: [0; ARENA], len: 0, limit: usize::MAX }
        }

        fn taking(limit: usize) -> Self {
            Self { limit, ..Self::new() }
        }

        fn written(&self) -> &[u8] {
            &self.seen[..self.len]
        }
    }

    impl Sink for Recorder {
        fn write(&mut self, bytes: &[u8]) -> usize {
            let take = core::cmp::min(self.limit, bytes.len()).min(self.seen.len() - self.len);
            self.seen[self.len..self.len + take].copy_from_slice(&bytes[..take]);
            self.len += take;
            self.limit -= take;
            take
        }
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

    #[test]
    fn an_index_ring_the_wrong_length_is_refused() {
        // Not a `Backing`, because the whole point is a mapping whose two
        // arrays disagree — which `Backing` cannot express.
        let head = Cursor::new();
        let tail = Cursor::new();
        let flags = AtomicU32::new(0);
        let index = [const { AtomicU32::new(0) }; 4];
        let entries = [const { UnsafeCell::new(Sqe::ZERO) }; 8];
        let chan = || Channel {
            head: &head,
            tail: &tail,
            flags: &flags,
            index: &index,
            entries: &entries,
        };

        assert!(Producer::new(chan()).is_none());
        assert!(Consumer::new(chan()).is_none());
    }

    #[test]
    fn a_batch_becomes_visible_all_at_once() {
        let b = Backing::<8>::new();
        let mut p = Producer::new(b.chan()).unwrap();
        let c = Consumer::new(b.chan()).unwrap();

        let mut batch = p.batch();
        for i in 0..4u64 {
            batch.push(entry(i)).unwrap();
        }
        assert_eq!(batch.staged(), 4);

        // The whole reason for the type: nothing is visible until publish.
        assert!(c.pop().unwrap().is_none(), "a staged batch must not be readable");

        batch.publish().unwrap();
        for i in 0..4u64 {
            assert_eq!(c.pop().unwrap().unwrap().user_data, i);
        }
        assert!(c.pop().unwrap().is_none());
    }

    #[test]
    fn a_dropped_batch_publishes_nothing() {
        let b = Backing::<8>::new();
        let mut p = Producer::new(b.chan()).unwrap();
        let c = Consumer::new(b.chan()).unwrap();

        {
            let mut batch = p.batch();
            batch.push(entry(7)).unwrap();
        }
        assert!(c.pop().unwrap().is_none(), "dropping a batch discards it");

        // And the ring is still usable, at the same cursor position.
        p.submit(entry(9)).unwrap();
        assert_eq!(c.pop().unwrap().unwrap().user_data, 9);
    }

    #[test]
    fn a_batch_counts_its_own_staged_entries_against_capacity() {
        let b = Backing::<4>::new();
        let mut p = Producer::new(b.chan()).unwrap();

        let mut batch = p.batch();
        for i in 0..4u64 {
            batch.push(entry(i)).unwrap();
        }
        // The fifth would overwrite the first, which the consumer has not seen
        // because nothing has been published at all.
        assert_eq!(batch.push(entry(4)), Err(RingError::Full));
    }

    #[test]
    fn a_forged_index_is_refused_not_followed() {
        let b = Backing::<4>::new();
        let p = Producer::new(b.chan()).unwrap();
        let c = Consumer::new(b.chan()).unwrap();

        p.submit(entry(1)).unwrap();
        // A peer points a published position at a slot outside the array. The
        // cursors are entirely legal; only the indirection is hostile.
        b.index[0].store(99, Ordering::Relaxed);
        // `matches!` for the same reason the hostile-cursor test above gives:
        // comparing the `Ok` side would need `Sqe: PartialEq`, and what the
        // wire types derive is an ABI decision.
        assert!(matches!(c.pop(), Err(RingError::Corrupt)));
    }

    #[test]
    fn completions_round_trip_and_report_full() {
        let b = Backing::<2>::new();
        let poster = Poster::new(b.cq()).unwrap();
        let collector = Collector::new(b.cq()).unwrap();

        assert_eq!(poster.free().unwrap(), 2);
        poster.post(completion(1, 0, 10)).unwrap();
        poster.post(completion(2, 0, 20)).unwrap();
        assert_eq!(poster.post(completion(3, 0, 30)), Err(RingError::Full));

        assert_eq!(collector.take().unwrap().unwrap().user_data, 1);
        assert_eq!(collector.take().unwrap().unwrap().user_data, 2);
        assert!(collector.take().unwrap().is_none());
    }

    #[test]
    fn nop_completes_with_the_callers_token() {
        let b = Backing::<4>::new();
        let mut sink = Recorder::new();
        let cqe = execute(&entry(0xABCD), &b.arena(), &mut sink, 77).expect("nop completes");

        assert_eq!(cqe.user_data, 0xABCD);
        assert_eq!(cqe.result, 0);
        assert_eq!(cqe.timestamp, 77, "the completion is stamped with the clock it was given");
        assert!(!cqe.is_error());
        assert_eq!(sink.written(), b"", "nop touches nothing");
    }

    #[test]
    fn write_serial_copies_the_arena_range_it_was_given() {
        let b = Backing::<4>::new();
        b.fill(8, b"hello, ring");

        let mut sqe = entry(1);
        sqe.opcode = op::WRITE_SERIAL;
        sqe.offset = 8;
        sqe.len = 11;

        let mut sink = Recorder::new();
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.result, 11);
        assert_eq!(sink.written(), b"hello, ring");
    }

    #[test]
    fn write_serial_chunks_a_payload_larger_than_its_buffer() {
        let b = Backing::<4>::new();
        let payload: [u8; CHUNK + 32] = core::array::from_fn(|i| (i % 251) as u8);
        b.fill(0, &payload);

        let mut sqe = entry(1);
        sqe.opcode = op::WRITE_SERIAL;
        sqe.offset = 0;
        sqe.len = payload.len() as u32;

        let mut sink = Recorder::new();
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.result, payload.len() as i32);
        assert_eq!(sink.written(), &payload[..]);
    }

    #[test]
    fn a_short_sink_is_a_partial_completion_and_not_a_failure() {
        let b = Backing::<4>::new();
        b.fill(0, b"abcdefgh");

        let mut sqe = entry(1);
        sqe.opcode = op::WRITE_SERIAL;
        sqe.len = 8;

        let mut sink = Recorder::taking(3);
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert!(!cqe.is_error(), "a short write is a count, not an error");
        assert_eq!(cqe.result, 3);
        assert_eq!(sink.written(), b"abc");
    }

    #[test]
    fn a_range_outside_the_arena_is_refused() {
        let b = Backing::<4>::new();
        let mut sink = Recorder::new();

        // Past the end.
        let mut sqe = entry(1);
        sqe.opcode = op::WRITE_SERIAL;
        sqe.offset = (ARENA - 4) as u64;
        sqe.len = 16;
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.error(), Some((error::ARGUMENT, error::argument::BAD_ADDRESS)));

        // And an offset that would overflow the addition rather than merely
        // exceed the bound, which is the version a bounds check written as
        // `offset + len > arena` gets wrong.
        let mut sqe = entry(2);
        sqe.opcode = op::WRITE_SERIAL;
        sqe.offset = u64::MAX;
        sqe.len = 8;
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.error(), Some((error::ARGUMENT, error::argument::BAD_ADDRESS)));
        assert_eq!(sink.written(), b"", "a refused write writes nothing");
    }

    #[test]
    fn the_envelope_is_checked_before_the_opcode() {
        let b = Backing::<4>::new();
        let mut sink = Recorder::new();

        // All three wrong at once. The reserved field is reported, because an
        // entry that is not a well-formed entry is not an entry with a bad
        // opcode.
        let mut sqe = entry(1);
        sqe.opcode = 200;
        sqe.flags = 1 << 7;
        sqe._reserved = 5;
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.error(), Some((error::ARGUMENT, error::argument::RESERVED_NOT_ZERO)));
        assert_eq!(cqe.ext, 5, "a refusal names the offending value");

        sqe._reserved = 0;
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.error(), Some((error::ARGUMENT, error::argument::UNKNOWN_FLAG)));
        assert_eq!(cqe.ext, 1 << 7);

        sqe.flags = 0;
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).unwrap();
        assert_eq!(cqe.error(), Some((error::ARGUMENT, error::argument::UNKNOWN_OPCODE)));
        assert_eq!(cqe.ext, 200);
    }

    #[test]
    fn no_cqe_skips_a_success_and_never_a_refusal() {
        let b = Backing::<4>::new();
        let mut sink = Recorder::new();

        let mut sqe = entry(1);
        sqe.flags = flags::NO_CQE;
        assert!(execute(&sqe, &b.arena(), &mut sink, 0).is_none(), "a quiet success is quiet");

        sqe.opcode = 77;
        let cqe = execute(&sqe, &b.arena(), &mut sink, 0).expect("a refusal is never skipped");
        assert_eq!(cqe.error(), Some((error::ARGUMENT, error::argument::UNKNOWN_OPCODE)));
    }

    #[test]
    fn the_service_drains_a_batch_and_answers_each_entry() {
        let b = Backing::<8>::new();
        b.fill(0, b"ok");

        let mut producer = Producer::new(b.chan()).unwrap();
        let mut batch = producer.batch();
        batch.push(entry(1)).unwrap();
        let mut write = entry(2);
        write.opcode = op::WRITE_SERIAL;
        write.len = 2;
        batch.push(write).unwrap();
        let mut bad = entry(3);
        bad.opcode = 99;
        batch.push(bad).unwrap();
        batch.publish().unwrap();

        let mut service = Service::new(
            Consumer::new(b.chan()).unwrap(),
            Poster::new(b.cq()).unwrap(),
            b.arena(),
            Recorder::new(),
        );
        let done = service.drain(16, 5).unwrap();
        assert_eq!(done, Drained { executed: 3, completed: 3, refused: 1 });
        assert_eq!(service.sink().written(), b"ok");

        let collector = Collector::new(b.cq()).unwrap();
        assert_eq!(collector.take().unwrap().unwrap().result, 0);
        assert_eq!(collector.take().unwrap().unwrap().result, 2);
        let refused = collector.take().unwrap().unwrap();
        assert_eq!(refused.error(), Some((error::ARGUMENT, error::argument::UNKNOWN_OPCODE)));
    }

    #[test]
    fn the_budget_bounds_the_work_a_peer_can_ask_for() {
        let b = Backing::<8>::new();
        let p = Producer::new(b.chan()).unwrap();
        for i in 0..8u64 {
            p.submit(entry(i)).unwrap();
        }

        let mut service = Service::new(
            Consumer::new(b.chan()).unwrap(),
            Poster::new(b.cq()).unwrap(),
            b.arena(),
            Recorder::new(),
        );
        assert_eq!(service.drain(3, 0).unwrap().executed, 3, "the budget is a ceiling");
        assert_eq!(service.drain(100, 0).unwrap().executed, 5, "and not a quota");
    }

    #[test]
    fn the_service_takes_no_entry_it_cannot_answer() {
        // A completion ring smaller than the submission ring, which
        // `f_abi::layout` will not lay out and a hostile peer can still hand
        // over. The service must stop rather than drop an answer.
        let b = Backing::<4>::new();
        let p = Producer::new(b.chan()).unwrap();
        for i in 0..4u64 {
            p.submit(entry(i)).unwrap();
        }

        let small = Completions { head: &b.cq_head, tail: &b.cq_tail, slots: &b.slots[..2] };
        let mut service = Service::new(
            Consumer::new(b.chan()).unwrap(),
            Poster::new(small).unwrap(),
            b.arena(),
            Recorder::new(),
        );

        let done = service.drain(4, 0).unwrap();
        assert_eq!(done.executed, 2, "two answers of room means two questions taken");
        assert_eq!(done.completed, 2);

        // The two it did not take are still on the ring, in order.
        let c = Consumer::new(b.chan()).unwrap();
        assert_eq!(c.pop().unwrap().unwrap().user_data, 2);
        assert_eq!(c.pop().unwrap().unwrap().user_data, 3);
    }
}
