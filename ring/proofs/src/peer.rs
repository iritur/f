// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The bytes a peer wrote, drawn by a solver rather than written down.
//!
//! # Why the fixture is a region and not a struct
//!
//! `ring/src/mapping.rs` opens with the sentence this module is built around:
//! everything else in `f_ring` takes borrowed Rust references and is correct
//! partly because the borrow checker already proved the regions do not overlap,
//! are the length they say, and are aligned — so a [`Channel`](f_ring::Channel)
//! assembled out of fields a harness owns *can only ever be laid out
//! correctly*. That is a fixture, not a mapping, and a proof over one would be
//! a proof about a shape the hostile case never takes.
//!
//! So the fixture here is [`Region`]: one aligned run of bytes, every one of
//! them chosen by the solver, adopted through the real
//! [`Mapping::adopt`](f_ring::Mapping::adopt). The header, the two pairs of
//! cursors, the flags word, the index ring, both entry arrays and the arena are
//! then all *the same symbolic bytes*, which is exactly the relationship they
//! have in a channel a peer holds the far end of and is not a relationship a
//! struct of separate fields can express.
//!
//! # What the region's size bounds, and what it does not
//!
//! [`REGION`] is 640 bytes, and `f_abi::layout` decides what that admits:
//! `Layout::new` places the arena at 512 bytes for a ring of one entry and at
//! 576 for a ring of two, so a mapping this long holds a ring of **one or two**
//! entries and `Layout::adopt` refuses every larger `ring_size` a header could
//! claim. The solver chooses which, along with everything else.
//!
//! Nothing else in the fixture is bounded. The mapping length is any value up
//! to the region; the cursors are all 2^32 values, including the ones that make
//! occupancy wrap; the slot numbers in the index ring are all 2^32, which is
//! the whole point of the one path this crate has to be able to break; the
//! entries and the arena are arbitrary bytes.
//!
//! That the ring is small is therefore the bound, and it is stated where it
//! costs something rather than left as a footnote: a defect that needs three
//! queued entries to appear is outside these proofs and inside
//! `ring/tests/hostile.rs`, which runs the same code at larger ring sizes for a
//! billion operations. The two instruments are not substitutes, and RFC 0057
//! says so at more length.
//!
//! `wide-ring` is what stops that from being an argument: it grows the region
//! to hold a ring of eight, and `cargo xtask prove` runs the four harnesses
//! that read a region a second time under it. If the small ring was hiding
//! nothing they pass twice; if it was, that is where it is found rather than
//! reasoned about.
//!
//! # The one exception, named here so it is not a surprise
//!
//! [`Rings`] is a channel as separate fields, and `draining_an_arbitrary_channel`
//! is the single harness that uses it. Its own comment argues why — the short
//! version is that adopting a region costs a seventeen-deep `memcmp` unroll
//! which then bounds the drain loop as well, and that harness's property is
//! about the loop rather than about the layout.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

// The checker injects `kani` as an extern crate; without one, `crate::kani` is
// the shim that lets this file typecheck under the pinned toolchain. See
// `lib.rs` for what that build is for.
#[cfg(not(kani))]
use crate::kani;

use f_abi::buf::SetId;
use f_abi::{ChannelHeader, Cqe, Sqe, error};
use f_ring::buffers::Submitter;
use f_ring::registry::{Domains, PageWalk, Refusal};
use f_ring::{Channel, Completions, Cursor, RingError, Sink};

/// Bytes of shared mapping a harness owns. Unit: bytes.
///
/// See the module documentation for what this admits. Not a tuning constant:
/// it is the bound every proof over a mapping in this crate is stated inside,
/// and the reason it is 640 rather than a page is that a checker unrolls a
/// slice rather than summarising it.
#[cfg(not(feature = "wide-ring"))]
pub const REGION: usize = 640;

/// The same, large enough for a ring of eight.
///
/// `Layout::new(8, 0)` places the arena at 1152, so this holds a ring of one,
/// two, four or eight entries. Turned on by `cargo xtask prove`'s second pass,
/// which is what turns *the harnesses that do not walk the ring cannot depend
/// on its size* from a claim about the code into a check that fails when it
/// stops being true. Unit: bytes.
#[cfg(feature = "wide-ring")]
pub const REGION: usize = 1216;

/// A mapping-shaped region, aligned the way the frame maps one.
///
/// `align(64)` because `Mapping::adopt` refuses a base that is not
/// line-aligned, and a fixture that could not satisfy that check would prove
/// only that the check exists.
#[repr(C, align(64))]
pub struct Region {
    bytes: [u8; REGION],
}

impl Region {
    /// Every byte chosen by the solver.
    ///
    /// This is the hostile peer in one line: there is no well-formed part. The
    /// header may be anything, and so may everything the header describes.
    #[must_use]
    pub fn scribbled() -> Self {
        Self { bytes: kani::any() }
    }

    /// Where it starts.
    ///
    /// `&mut self` because the region is written through this pointer by the
    /// code under proof — a producer stages an entry, a consumer advances a
    /// cursor — so a shared borrow would be a fixture claiming the peer's half
    /// of a channel is read-only.
    pub fn base(&mut self) -> *mut u8 {
        self.bytes.as_mut_ptr()
    }
}

/// A mapping length the region can actually answer for.
///
/// Any value up to [`REGION`], chosen by the solver, and **not** a symbolic
/// `u32`: `Mapping::adopt`'s safety obligation is that the base names that many
/// mapped bytes, and a harness that passed a larger length would be proving
/// something about memory it does not own. The refusals a short mapping earns
/// are all reachable inside this bound — `Layout::adopt`'s `checked_sub` is
/// the one that fires — and [`crate::proofs`]'s `adopting_an_arbitrary_layout`
/// is where the length really is unbounded, because nothing there dereferences
/// anything.
#[must_use]
pub fn any_len() -> u32 {
    let len: u32 = kani::any();
    kani::assume(len as usize <= REGION);
    len
}

/// A header with every field chosen by the solver.
///
/// `ChannelHeader` is 64 bytes with no padding — `abi/src/lib.rs` asserts the
/// size, and the fields sum to it — so this is arbitrary header *bytes* and not
/// merely arbitrary header *fields*. The distinction would matter if there were
/// a hole, and the day one appears this comment is wrong rather than merely
/// optimistic.
#[must_use]
pub fn any_header() -> ChannelHeader {
    ChannelHeader {
        magic: kani::any(),
        features: kani::any(),
        features_required: kani::any(),
        abi_version: kani::any(),
        abi_version_min: kani::any(),
        ring_size: kani::any(),
        sqe_offset: kani::any(),
        cqe_offset: kani::any(),
        epoch: kani::any(),
        _reserved: [kani::any(), kani::any(), kani::any(), kani::any()],
    }
}

/// A submission with every field chosen by the solver, reserved word included.
///
/// Sixty-four bytes of anything, which is what a peer writes into an entry
/// slot. `_reserved` is reachable from here precisely so that a hostile fixture
/// can set it: R04 says an entry carrying one is refused rather than ignored,
/// and a harness that could not write one could not check that.
#[must_use]
pub fn any_sqe() -> Sqe {
    Sqe {
        opcode: kani::any(),
        flags: kani::any(),
        class: kani::any(),
        cap: kani::any(),
        user_data: kani::any(),
        deadline: kani::any(),
        offset: kani::any(),
        buf_set: kani::any(),
        buf_index: kani::any(),
        len: kani::any(),
        _reserved: kani::any(),
        ext: [kani::any(), kani::any()],
    }
}

/// A completion with every field chosen by the solver.
///
/// The one a *client* reads, which is the direction `ring/tests/hostile.rs`
/// reaches least: a service can be hostile too, and `SetId::from_completion` is
/// where a client believes one.
#[must_use]
pub fn any_cqe() -> Cqe {
    Cqe {
        user_data: kani::any(),
        result: kani::any(),
        flags: kani::any(),
        timestamp: kani::any(),
        ext: kani::any(),
    }
}

/// A buffer-set identifier chosen by the solver, over all 2^32 of them.
///
/// Including the shapes no service could have issued — generation zero, the
/// retired generation, a slot past the table — because those are the ones a
/// client that guesses produces, and `Table`'s own lookup is what refuses them.
#[must_use]
pub fn any_set() -> SetId {
    SetId::from_bits(kani::any())
}

/// The two rings of one channel, as separate fields rather than as a mapping.
///
/// **The one fixture in this crate that is a struct, and the exception is
/// argued rather than convenient.** The module comment above says why a proof
/// over a struct of fields is a proof about a channel the hostile case never
/// produces, and that stands for every harness whose property is about the
/// *layout*. It is not the property `draining_an_arbitrary_channel` is about:
/// that one is about the loop, and specifically about the loop having a
/// ceiling the caller chose.
///
/// What a region costs there is arithmetic rather than taste. Adopting one runs
/// `ChannelHeader::is_valid`, which compares four reserved words — sixteen
/// bytes, which a checker turns into a `memcmp` loop it must unroll seventeen
/// times. `kani::unwind` is one number for the whole harness, so seventeen is
/// then also the bound on `Service::drain`'s own loop, and every one of those
/// seventeen unrolled iterations carries an inlined `f_ring::execute`. That is
/// where twenty minutes went. Without a header there is no `memcmp`, the bound
/// drops to what the drain loop actually needs, and the same sentence is proved
/// in a fraction of the time.
///
/// So: the cursors are still all 2^32 values, the index ring's slot numbers are
/// still all 2^32, the entries are still arbitrary — the *only* thing this
/// fixture takes on trust is that the regions are where `Mapping` says they
/// are, and that is exactly what `popping_an_arbitrary_entry`,
/// `taking_an_arbitrary_completion`, `submitting_against_an_arbitrary_cursor`
/// and `adopting_arbitrary_bytes` prove over a region. RFC 0057.
pub struct Rings<const N: usize> {
    head: Cursor,
    tail: Cursor,
    flags: AtomicU32,
    index: [AtomicU32; N],
    entries: [UnsafeCell<Sqe>; N],
    cq_head: Cursor,
    cq_tail: Cursor,
    slots: [UnsafeCell<Cqe>; N],
}

impl<const N: usize> Rings<N> {
    /// Every cursor, every slot number and every entry chosen by the solver.
    #[must_use]
    pub fn scribbled() -> Self {
        let rings = Self {
            head: Cursor::new(),
            tail: Cursor::new(),
            flags: AtomicU32::new(kani::any()),
            index: [const { AtomicU32::new(0) }; N],
            entries: [const { UnsafeCell::new(Sqe::ZERO) }; N],
            cq_head: Cursor::new(),
            cq_tail: Cursor::new(),
            slots: [const { UnsafeCell::new(Cqe::ZERO) }; N],
        };
        rings.head.set(kani::any());
        rings.tail.set(kani::any());
        rings.cq_head.set(kani::any());
        rings.cq_tail.set(kani::any());
        for slot in 0..N {
            rings.index[slot].store(kani::any(), Ordering::Relaxed);
            // SAFETY: the harness owns these cells and no consumer, producer,
            // poster or collector exists yet — this is the peer's half of the
            // channel being written before either end is bound to it, which is
            // the only moment at which a fixture may touch them directly.
            unsafe { rings.entries[slot].get().write(any_sqe()) };
            // SAFETY: as above.
            unsafe { rings.slots[slot].get().write(any_cqe()) };
        }
        rings
    }

    /// The submission ring, as a consumer or producer takes it.
    #[must_use]
    pub const fn channel(&self) -> Channel<'_> {
        Channel {
            head: &self.head,
            tail: &self.tail,
            flags: &self.flags,
            index: &self.index,
            entries: &self.entries,
        }
    }

    /// The completion ring, as a poster or collector takes it.
    #[must_use]
    pub const fn completions(&self) -> Completions<'_> {
        Completions { head: &self.cq_head, tail: &self.cq_tail, slots: &self.slots }
    }
}

/// An arena's worth of bytes, as the shared cells a service reads through.
///
/// Used by the harnesses that drive `f_ring::execute` directly rather than
/// through a mapping, where the arena is part of the region.
pub struct Bytes<const N: usize> {
    cells: [UnsafeCell<u8>; N],
}

impl<const N: usize> Bytes<N> {
    /// An arena whose every byte the solver chose.
    #[must_use]
    pub fn scribbled() -> Self {
        let bytes: [u8; N] = kani::any();
        Self { cells: bytes.map(UnsafeCell::new) }
    }

    /// The cells, as `f_ring::Arena` takes them.
    #[must_use]
    pub const fn cells(&self) -> &[UnsafeCell<u8>; N] {
        &self.cells
    }
}

/// The address the harness's domain hands out for every registration.
///
/// A constant rather than a symbolic value, and the choice is load-bearing:
/// `Table::resolve` answers `address + index * stride`, so a symbolic base
/// would let that arithmetic overflow on the *clean* build for a reason that
/// has nothing to do with the code — a frame that answered `u64::MAX` for a
/// mapping. A real domain answers an address a device can reach, and this is
/// one. Unit: bytes, in the device's address space.
pub const DEVICE_BASE: u64 = 0x0000_0001_0000_0000;

/// A domain that answers, or refuses, as the solver chooses.
///
/// Both, because a [`Domains`] that always succeeded would leave
/// `Table::register`'s refusal path — the one that must leave no
/// half-registration and spend no generation — unproved, and one that always
/// refused would leave everything after it unreachable. The `cover` statements
/// in the harnesses are what say both halves were taken rather than assumed.
pub struct Domain {
    /// Translations this domain has given out and not taken back.
    /// Unit: translations. A harness asserts this returns to zero, which is
    /// what says a refused or retired registration left nothing behind.
    pub outstanding: i32,
}

impl Domain {
    /// A domain with nothing mapped.
    #[must_use]
    pub const fn empty() -> Self {
        Self { outstanding: 0 }
    }
}

impl Domains for Domain {
    fn map(&mut self, _cap: u32, _len: u32) -> Result<u64, Refusal> {
        if kani::any() {
            return Err((error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED), 0));
        }
        self.outstanding += 1;
        Ok(DEVICE_BASE)
    }

    fn unmap(&mut self, _cap: u32, _address: u64, _len: u32) {
        self.outstanding -= 1;
    }
}

/// A page walk whose answer the solver chooses.
///
/// Which is the honest model of it: `registry`'s own module documentation says
/// no hardware this project can boot answers this question, so a proof that
/// assumed an answer would be assuming the half that does not exist.
pub struct Walk;

impl PageWalk for Walk {
    fn reaches(&self, _address: u64, _len: u32) -> bool {
        kani::any()
    }
}

/// A sink that takes what it is offered, up to a limit the solver chooses.
///
/// The limit is the point. `write_serial` treats a short write as a partial
/// completion and stops, and a sink that always took everything would leave
/// that branch unproved — which is the branch where `written` and `len` can
/// disagree and therefore the only place its arithmetic is interesting.
pub struct Bucket {
    /// Bytes this sink will still accept. Unit: bytes.
    pub limit: usize,
    /// Bytes it has taken. Unit: bytes.
    pub taken: usize,
}

impl Bucket {
    /// A sink with a capacity the solver chose.
    #[must_use]
    pub fn any() -> Self {
        Self { limit: kani::any(), taken: 0 }
    }
}

/// Something a named entry can be handed to, that answers as the solver
/// chooses and remembers what it was given.
///
/// `f_ring::buffers::Submitter` is a trait for this reason — its own comment
/// says a test can stand a recorder in it and exercise the ownership rules with
/// no ring at all — and a proof wants the same thing more strongly: the client
/// side of RFC 0024 is about *which side may touch the bytes*, and putting a
/// real producer under it would pay for a mapping the property does not
/// mention.
pub struct Lane {
    /// The entry the last submission carried, as the ownership types wrote it.
    pub last: Option<Sqe>,
    /// Whether this lane refuses. Chosen by the solver, because a submission
    /// that always succeeded would leave `Idle::submit`'s hand-the-buffer-back
    /// path unproved — and that path is the one that has to give the buffer
    /// back rather than lose it.
    pub refuses: bool,
}

impl Lane {
    /// A lane that answers as the solver chooses.
    #[must_use]
    pub fn any() -> Self {
        Self { last: None, refuses: kani::any() }
    }
}

impl Submitter for Lane {
    fn submit(&mut self, entry: Sqe) -> Result<bool, RingError> {
        self.last = Some(entry);
        if self.refuses {
            return Err(RingError::Full);
        }
        Ok(kani::any())
    }
}

impl Sink for Bucket {
    fn write(&mut self, bytes: &[u8]) -> usize {
        let take = core::cmp::min(self.limit, bytes.len());
        self.limit -= take;
        self.taken += take;
        take
    }
}
