// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The channel as bytes: binding the ring halves to a region a peer also holds.
//!
//! # What this replaces
//!
//! Everything else in this crate takes borrowed Rust references — a [`Cursor`]
//! here, a slice of entries there — and is correct partly because the borrow
//! checker already proved the regions do not overlap, are the length they say,
//! and are aligned. A real channel offers none of that. It is one range of
//! shared addresses, a header at the front that somebody else wrote, and an
//! obligation to disbelieve all of it. Until this module existed, every caller
//! assembled a [`Channel`] out of fields it owned, which is a fixture rather
//! than a mapping: it can only ever be laid out correctly.
//!
//! # What is checked, and in what order
//!
//! The order is not cosmetic. Each step is only meaningful once the one before
//! it has passed.
//!
//! 1. **The address.** Alignment, and a region long enough to hold a header.
//!    This is the only bound that cannot come from the header, because it is
//!    what makes reading the header defined at all.
//! 2. **The header, copied out.** One volatile read of 64 bytes onto this
//!    stack.
//! 3. **Negotiation.** [`ChannelHeader::negotiate`] — magic, ring size, the
//!    version window, the required-feature sets, the reserved words. RFC 0011:
//!    peers meet in the middle rather than demanding equality, so a component
//!    can be updated independently of the frame.
//! 4. **The layout.** [`Layout::adopt`] — the offsets the header claims must
//!    equal the ones this build computes, and every region must land inside the
//!    mapping. The arena is whatever is left over, taken from the mapping
//!    length and never from the header.
//!
//! # Why the header is copied before it is checked
//!
//! Because the peer can rewrite those bytes between any two reads. Validating
//! in place would check one header and then build a [`Layout`] out of a
//! different one — a bounds check that bounds nothing, which is the failure
//! this tree keeps catching in its own work. What is validated below and what
//! is used afterwards are the same 64 bytes, because they are this stack
//! frame's 64 bytes. The read is volatile so the compiler may not undo that by
//! re-reading a field out of shared memory after the check.
//!
//! # What binding does not prove
//!
//! That the peer behaves afterwards. A [`Mapping`] rules out a *structural*
//! lie, once, at setup. The cursors stay untrusted at every access, the slot
//! numbers in the index ring stay bounds-checked on every pop, and the arena
//! stays a byte range with no meaning attached. Binding is the point past which
//! the arithmetic is known to describe the bytes — not the point past which the
//! bytes are known to be friendly.
//!
//! *Reversal:* a peer that may resize a live channel. Then the layout stops
//! being a setup-time fact and every accessor here needs a generation to check
//! against, which is a different and considerably more expensive design.

use core::cell::UnsafeCell;
use core::sync::atomic::AtomicU32;

use f_abi::layout::{self, Layout};
use f_abi::{ChannelHeader, Cqe, Negotiated, Sqe, error};

use crate::{Arena, Channel, Completions, Cursor};

/// A validated channel mapping: one shared region, bound once.
///
/// Holds a raw base rather than a slice, on purpose. A slice would assert
/// exclusive access to memory whose whole point is that a peer writes to it;
/// every accessor below hands out atomics and [`UnsafeCell`] instead, which is
/// how shared mutation is spelled in this language.
pub struct Mapping {
    base: *mut u8,
    layout: Layout,
    agreed: Negotiated,
    epoch: u32,
}

impl Mapping {
    /// Write a header describing `entries` into a zeroed region, then adopt it.
    ///
    /// This is the side that *creates* a channel, and it deliberately goes out
    /// through the wire format and back in again rather than keeping the
    /// [`Layout`] it just computed. The arithmetic is then checked against the
    /// bytes instead of against itself, so a header this build writes and
    /// cannot read back is a build failure at boot rather than a peer's problem
    /// much later.
    ///
    /// # Errors
    ///
    /// A structured error per RFC 0010. `ARGUMENT/BAD_ADDRESS` if the region
    /// cannot hold a header at an alignment the layout can be stated against,
    /// `ARGUMENT/MALFORMED_HEADER` if `entries` and `len` describe a channel
    /// that does not fit, and anything [`Self::adopt`] refuses.
    ///
    /// # Safety
    ///
    /// `base` must be valid for reads and writes of `len` bytes for as long as
    /// the returned value lives, and the region must be zeroed. Zeroed is a
    /// real obligation and not tidiness: the cursors, the index ring and both
    /// entry arrays are reinterpreted in place, and all-zero is a bit pattern
    /// each of those types is valid at.
    pub unsafe fn describe(
        base: *mut u8,
        len: u32,
        entries: u32,
        epoch: u32,
        offers: u64,
        requires: u64,
    ) -> Result<Self, i32> {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

        Self::addressable(base, len)?;

        // Laid out with no arena, because the arena is whatever the mapping has
        // left over and the mapping is the caller's. `total()` is then the
        // smallest region this many entries can live in.
        let Some(layout) = Layout::new(entries, 0) else { return Err(malformed) };
        if layout.total() > len {
            return Err(malformed);
        }
        let header = layout.describe(epoch, offers, requires);

        // SAFETY: `addressable` established that `base` is line-aligned, which
        // is `ChannelHeader`'s 64-byte alignment, and that a whole header is
        // inside the region. Volatile because the peer reads these bytes: this
        // store may not be sunk past the point the channel is handed over.
        unsafe { base.cast::<ChannelHeader>().write_volatile(header) };

        // SAFETY: the caller's obligations are this function's obligations, and
        // the header just written is inside the region checked above.
        unsafe { Self::adopt(base, len, offers, requires) }
    }

    /// Adopt a region whose header a peer wrote.
    ///
    /// # Errors
    ///
    /// A structured error per RFC 0010, and never a panic: every field read
    /// here is untrusted input, so refusing is the ordinary path rather than
    /// the exceptional one. `ARGUMENT/BAD_ADDRESS` for a region no header can
    /// be read from, `ARGUMENT/MALFORMED_HEADER` for one whose header is
    /// structurally wrong or whose offsets are not this build's,
    /// `PEER/VERSION_UNSUPPORTED` for a version window that does not overlap,
    /// and `PEER/FEATURE_REQUIRED` when either side requires what the other
    /// does not offer.
    ///
    /// # Safety
    ///
    /// `base` must be valid for reads and writes of `len` bytes for as long as
    /// the returned value lives, and the only references into that range may be
    /// ones handed out by this type — including by a second `Mapping` over the
    /// same bytes, which is what the far end of a channel is. That is why every
    /// accessor returns an atomic or an [`UnsafeCell`] and never a plain
    /// reference to anything a peer writes: two ends sharing a region is the
    /// intended use, not a violation of it.
    pub unsafe fn adopt(base: *mut u8, len: u32, offers: u64, requires: u64) -> Result<Self, i32> {
        Self::addressable(base, len)?;

        // SAFETY: `addressable` established alignment and that a whole header
        // is inside the region. See the module docs for why this is a copy and
        // why it is volatile — both are load-bearing rather than stylistic.
        let header = unsafe { base.cast::<ChannelHeader>().read_volatile() };

        let agreed = header.negotiate(offers, requires)?;
        let layout = Layout::adopt(&header, len)?;

        Ok(Self { base, layout, agreed, epoch: header.epoch })
    }

    /// Rebuild a mapping over bytes [`Self::adopt`] has already believed.
    ///
    /// # Why this exists, and why it is not a shortcut
    ///
    /// `f_ring::adopt` binds a channel *for one call*: it holds the layout it
    /// was validated at and rebuilds the mapping on every access, so no
    /// reference into memory a peer writes outlives the call that made it. That
    /// is what lets a component adopt a channel in safe code — RFC 0037 — and
    /// re-running [`Self::adopt`] there would be worse than slow: the header is
    /// the peer's, so a peer that rewrote it between two calls could move the
    /// entry array under a component midway through a drain. **Believing
    /// happens once; binding happens per call.**
    ///
    /// # Safety
    ///
    /// As [`Self::adopt`], and `layout`, `agreed` and `epoch` must be exactly
    /// what [`Self::adopt`] answered for these bytes at this `base`. Passing a
    /// layout computed for anything else is passing bounds that bound nothing,
    /// which is the failure the whole of this module exists to refuse.
    pub(crate) const unsafe fn bound(
        base: *mut u8,
        layout: Layout,
        agreed: Negotiated,
        epoch: u32,
    ) -> Self {
        Self { base, layout, agreed, epoch }
    }

    /// Refuse an address the layout arithmetic cannot even be stated against.
    fn addressable(base: *mut u8, len: u32) -> Result<(), i32> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);

        // Every fixed region in `f_abi::layout` is placed on a cache line
        // measured from the first byte of the mapping, so a base that is not
        // itself line-aligned makes every one of those offsets a lie — and the
        // cursors would be misaligned, which is undefined behaviour rather than
        // a slow channel. Checked here rather than left to the caller's safety
        // obligation, because the caller is usually repeating an address a peer
        // chose.
        if !(base as usize).is_multiple_of(layout::LINE as usize) {
            return Err(bad);
        }

        // The header says how large the mapping ought to be, so the header
        // itself is the one thing whose room has to be established first.
        if len < core::mem::size_of::<ChannelHeader>() as u32 {
            return Err(bad);
        }

        Ok(())
    }

    /// What the two sides agreed to speak.
    #[must_use]
    pub const fn negotiated(&self) -> Negotiated {
        self.agreed
    }

    /// The layout this mapping was adopted at.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// The peer's epoch at the moment of binding.
    ///
    /// Carried rather than re-read: a channel is bound to one epoch, and a peer
    /// that restarts produces a *different* channel whose outstanding tokens
    /// are all stale. Comparing a later reading against this is how that is
    /// noticed.
    /// Unit: restarts of the writing peer.
    #[must_use]
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    /// The submission ring: cursors, index ring and entry array.
    #[must_use]
    pub fn channel(&self) -> Channel<'_> {
        Channel {
            head: self.cursor(layout::HEAD),
            tail: self.cursor(layout::TAIL),
            flags: self.flags(),
            index: self.index(),
            entries: self.entries(),
        }
    }

    /// The completion ring. RFC 0018 gave it its own pair of cursors.
    #[must_use]
    pub fn completions(&self) -> Completions<'_> {
        Completions {
            head: self.cursor(layout::CQ_HEAD),
            tail: self.cursor(layout::CQ_TAIL),
            slots: self.slots(),
        }
    }

    /// The inline arena, as an operation sees it.
    #[must_use]
    pub fn arena(&self) -> Arena<'_> {
        Arena::new(self.arena_cells())
    }

    /// The inline arena as raw shared bytes, which is how a submitter stages a
    /// payload into it before submitting.
    ///
    /// Separate from [`Self::arena`] because the two directions are different
    /// rights: [`Arena`] is the read side an operation is handed, and this is
    /// the write side the peer uses.
    #[must_use]
    pub fn arena_cells(&self) -> &[UnsafeCell<u8>] {
        // SAFETY: the arena is the tail of the mapping — `arena_len()` bytes
        // starting at `arena_offset()`, both fixed by the adopted `Layout`,
        // which refused any header whose regions did not fit. A byte needs no
        // alignment, and `UnsafeCell<u8>` is valid at every bit pattern.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.arena_offset()).cast::<UnsafeCell<u8>>(),
                self.layout.arena_len() as usize,
            )
        }
    }

    /// The address of one offset into the mapping.
    fn at(&self, offset: u32) -> *mut u8 {
        // SAFETY: every offset reaching here comes from the adopted `Layout`,
        // which refused to produce one outside `total()`, and `adopt` refused a
        // mapping shorter than `total()`.
        unsafe { self.base.add(offset as usize) }
    }

    fn cursor(&self, offset: u32) -> &Cursor {
        // SAFETY: the four cursor offsets are whole multiples of `LINE` and the
        // base is line-aligned, so the pointer meets `Cursor`'s 64-byte
        // alignment. A zeroed cursor is a valid one, and every bit pattern of
        // the `AtomicU32` inside it is valid thereafter — which is the property
        // that lets a peer write it while this reference exists.
        unsafe { &*self.at(offset).cast::<Cursor>() }
    }

    fn flags(&self) -> &AtomicU32 {
        // SAFETY: `layout::FLAGS` sits four bytes into a line-aligned cursor
        // line, so it is four-byte aligned, and every bit pattern is a valid
        // `AtomicU32`.
        unsafe { &*self.at(layout::FLAGS).cast::<AtomicU32>() }
    }

    fn index(&self) -> &[AtomicU32] {
        // SAFETY: `entries` four-byte slots at a line-aligned offset, inside
        // the mapping by the adopted layout.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.sq_index_offset()).cast::<AtomicU32>(),
                self.layout.entries() as usize,
            )
        }
    }

    fn entries(&self) -> &[UnsafeCell<Sqe>] {
        // SAFETY: `Layout` places the entry array on a cache line, which is
        // `Sqe`'s alignment, and reserves `entries * 64` bytes for it inside
        // the mapping. `UnsafeCell<Sqe>` is valid at every bit pattern, which
        // is what makes this sound over memory a peer is writing.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.sqe_offset()).cast::<UnsafeCell<Sqe>>(),
                self.layout.entries() as usize,
            )
        }
    }

    fn slots(&self) -> &[UnsafeCell<Cqe>] {
        // SAFETY: as `entries`. A `Cqe` is 32 bytes and needs 32-byte
        // alignment, which a line-aligned offset satisfies. The completion ring
        // has as many slots as the submission ring, so it cannot fill.
        unsafe {
            core::slice::from_raw_parts(
                self.at(self.layout.cqe_offset()).cast::<UnsafeCell<Cqe>>(),
                self.layout.entries() as usize,
            )
        }
    }
}
