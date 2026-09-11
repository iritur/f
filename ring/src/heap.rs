// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A heap is a window with a policy.
//!
//! # Why it is here
//!
//! A component that wants `alloc` needs a `#[global_allocator]`, and that is an
//! `unsafe impl GlobalAlloc`. RFC 0001 permits `unsafe` in `abi`, `ring` and
//! `kernel` and forbids it everywhere else at compile time, so a component crate
//! cannot write one. `intent/0006-state`'s decision 4 settled where it goes:
//! here, as a bump-then-free-list over a region the frame granted, built by a
//! constructor that checks the region the way [`crate::device::Window`] checks a
//! window. RFC 0033's shape, one level up — a granted window is a safe accessor,
//! and a heap is a window with a policy.
//!
//! # Where the mutable state lives, and why that is the whole design
//!
//! In the region, and nowhere else.
//!
//! `xtask` refuses a component image carrying any writable symbol: the text page
//! is mapped read-only, so a mutable global is a page fault on first write in a
//! component that has no way to report one. But `#[global_allocator]` needs a
//! `static`. Those two requirements look irreconcilable and are not, because the
//! static does not have to *hold* anything: [`Heap`] is one `u64`, which is the
//! address, and every byte the allocator mutates is inside the region that
//! address names.
//!
//! Three consequences, each load-bearing:
//!
//! - The static is `Freeze` and contains no interior mutability, so it is a
//!   constant and lands in `.rodata` — `llvm-nm` class `r`, which is not in the
//!   eight classes `xtask` rejects. Measured before this file was written, with
//!   a throwaway crate built the way `xtask` builds a component: the resulting
//!   archive had **no symbol in any rejected class at all**.
//! - It holds an integer and not a pointer, so it can never acquire a
//!   relocation, so it can never fall into `user/init/link.ld`'s `.data.rel.ro`
//!   sweep and land in `.data` after all. That trap is why the field is a `u64`
//!   and not a `*mut u8`, and it is the kind of thing that would have been found
//!   by a build failure three months from now rather than by a sentence.
//! - Free blocks carry their own links, so there is **no side table**. A side
//!   table is the thing that would have wanted `.bss`, and not having one is why
//!   this design has no writable state to place rather than a clever place to
//!   put it.
//!
//! # Two constructors, and the split is a safety property
//!
//! [`Heap::COMPONENT`] is safe and names exactly one address. [`Heap::over`] is
//! `unsafe` and takes any address.
//!
//! The split is not tidiness. A safe `Heap::at(base)` would let safe component
//! code name the stack, the control ring or its own state tree: each satisfies
//! the sentence *this is a region the frame granted me* word for word, none
//! faults, and the allocator would hand out boxes overlapping live stack frames.
//! The safe constructor therefore names one constant and the general one is
//! `unsafe`, which a component forbidding `unsafe` cannot call — so the property
//! is enforced by the same rule that put this file here.
//!
//! # What a reader has to check
//!
//! One invariant: **the allocator writes the prologue and free blocks, and never
//! a live one.** A block that is handed out is not touched again until it comes
//! back through [`GlobalAlloc::dealloc`]. Everything else here is arithmetic.

use core::alloc::{GlobalAlloc, Layout};

/// Where the frame maps a component's heap.
///
/// Must equal `kernel::process::SPAWN_HEAP`, which asserts it at compile time so
/// that a disagreement fails to link rather than faulting at the first
/// allocation. That is the arrangement `kernel/src/blk.rs` has had for
/// `BLK_BOARD` since RFC 0047 and `kernel/src/runtime.rs` has for the three
/// runtime addresses.
///
/// One address for every component shape, so there is one assertion rather than
/// one per component. The *size* is not here: it is a per-component number its
/// manifest declares, and the frame records it in the prologue.
/// Unit: bytes, in the component's address space.
pub const AT: u64 = 0x0042_E000;

/// What the prologue occupies at the foot of the region.
///
/// Thirty-two bytes, and the first block starts after it — which is why offset
/// zero is never a valid block and can be spelled *no block* in the free list
/// without a sentinel.
/// Unit: bytes.
pub const HEADER_BYTES: u32 = 32;

/// The allocation granularity, and the minimum block.
///
/// Sixteen, because a free block stores two `u32`s in its own first eight bytes
/// and because sixteen is the alignment `alloc`'s own containers ask for at the
/// top of the range this serves. An allocation of one byte occupies sixteen, and
/// the waste is visible in [`Heap::live`] rather than hidden.
/// Unit: bytes.
pub const GRANULARITY: u32 = 16;

/// What says these bytes are a heap and not whatever was there before.
///
/// Read on every bind, for [`crate::mapping::Mapping`]'s reason: a component
/// that adopted a region nobody described would allocate out of arbitrary
/// memory, and the cheapest thing standing between it and that is a word.
const MAGIC: u64 = 0x465f_4845_4150_0001;

/// The layout this file writes and reads.
const VERSION: u16 = 1;

// Offsets into the prologue. Written here once and used by both the frame's
// `describe` and the component's allocator, because the two are the same file
// and cannot drift about the format — the trick `crate::mapping` plays, where
// the writer goes out through the wire layout and back in again.
const OFF_MAGIC: u32 = 0;
const OFF_BYTES: u32 = 8;
const OFF_VERSION: u32 = 12;
const OFF_FLAGS: u32 = 14;
const OFF_BUMP: u32 = 16;
const OFF_FREE: u32 = 20;
const OFF_LIVE: u32 = 24;
const OFF_PEAK: u32 = 28;

/// A free block's own two words, at its own foot.
const OFF_NEXT: u32 = 0;
const OFF_SIZE: u32 = 4;

/// Set when an allocation has been refused for want of room.
///
/// A gauge and not a counter, and it is never cleared: what a reader wants to
/// know is *did this component ever fail to allocate*, and a component that
/// recovered from one refusal and failed later is not a component that was fine.
pub const FLAG_STARVED: u16 = 1;

/// Describe a region as an empty heap.
///
/// The frame calls this before the component's first instruction, exactly as it
/// writes a state tree's schema before the first instruction. The component
/// never writes the prologue's shape, only its numbers.
///
/// # Safety
///
/// `at` must be valid for writes of [`HEADER_BYTES`] bytes and aligned to eight.
/// `bytes` is a value **recorded**, not an extent touched: this writes only the
/// prologue, so a wrong `bytes` is a heap that later hands out addresses outside
/// the region, which is the caller's obligation to get right and not something
/// this function can check.
pub unsafe fn describe(at: u64, bytes: u32) {
    // One block per store, which `multiple_unsafe_ops_per_block` requires and
    // which is the right shape anyway: each is a different word and each is
    // covered by the same clause of the caller's contract.
    //
    // SAFETY: the caller's contract — `at` is valid for writes of
    // `HEADER_BYTES` bytes and is eight-aligned, so this aligned word is inside.
    unsafe { ((at + u64::from(OFF_MAGIC)) as *mut u64).write_volatile(MAGIC) };
    // SAFETY: as above, four bytes at a four-aligned offset inside the prologue.
    unsafe { ((at + u64::from(OFF_BYTES)) as *mut u32).write_volatile(bytes) };
    // SAFETY: as above, two bytes at a two-aligned offset.
    unsafe { ((at + u64::from(OFF_VERSION)) as *mut u16).write_volatile(VERSION) };
    // SAFETY: as above.
    unsafe { ((at + u64::from(OFF_FLAGS)) as *mut u16).write_volatile(0) };
    // SAFETY: as above, four bytes.
    unsafe { ((at + u64::from(OFF_BUMP)) as *mut u32).write_volatile(HEADER_BYTES) };
    // SAFETY: as above.
    unsafe { ((at + u64::from(OFF_FREE)) as *mut u32).write_volatile(0) };
    // SAFETY: as above.
    unsafe { ((at + u64::from(OFF_LIVE)) as *mut u32).write_volatile(0) };
    // SAFETY: as above.
    unsafe { ((at + u64::from(OFF_PEAK)) as *mut u32).write_volatile(0) };
}

/// A component's heap, as an address.
///
/// See the module comment for why this holds an integer and nothing else, and
/// why that is what makes a `#[global_allocator]` and an empty `.data` possible
/// at the same time.
pub struct Heap {
    /// The foot of the region. Unit: bytes, in the holder's address space.
    at: u64,
}

impl Heap {
    /// The heap the frame maps for a component, at [`AT`].
    ///
    /// Safe, and the only constructor a component can reach. See the module
    /// comment on why the general one is `unsafe`.
    pub const COMPONENT: Self = Self { at: AT };

    /// A heap over a region the caller names.
    ///
    /// For `ring/`'s own tests and for the simulator, which drive the identical
    /// allocator over host memory — which is what makes the arithmetic testable
    /// without a boot.
    ///
    /// # Safety
    ///
    /// `base` must name a region that [`describe`] has described, readable and
    /// writable for the `bytes` that prologue records, aligned to eight, and
    /// reachable for as long as this value is used. Naming a region the holder
    /// does not own is the failure this being `unsafe` exists to prevent: every
    /// address in a component's map satisfies the *words* of that sentence and
    /// only one satisfies it in fact.
    #[must_use]
    pub const unsafe fn over(base: u64) -> Self {
        Self { at: base }
    }

    /// Read one `u32` from the region.
    fn get(&self, offset: u32) -> u32 {
        // SAFETY: the type's contract — `at` names a described region, and every
        // caller below derives `offset` from the prologue's own bounds.
        unsafe { ((self.at + u64::from(offset)) as *const u32).read_volatile() }
    }

    /// Write one `u32` into the region.
    fn put(&self, offset: u32, value: u32) {
        // SAFETY: as `get`, and the offsets written are the prologue's own words
        // or a *free* block's two — never a live block's bytes, which is the one
        // invariant this file asks a reader to check.
        unsafe { ((self.at + u64::from(offset)) as *mut u32).write_volatile(value) };
    }

    /// Does this region carry a heap this build can use?
    ///
    /// Checked on every bind rather than once, because a component that adopted
    /// an undescribed region would allocate out of arbitrary memory.
    #[must_use]
    pub fn valid(&self) -> bool {
        // SAFETY: the type's contract; a magic word is the first thing read and
        // the thing that says the rest may be.
        let magic = unsafe { ((self.at + u64::from(OFF_MAGIC)) as *const u64).read_volatile() };
        let version = {
            // SAFETY: as above, two bytes inside the prologue.
            unsafe { ((self.at + u64::from(OFF_VERSION)) as *const u16).read_volatile() }
        };
        magic == MAGIC && version == VERSION && self.get(OFF_BYTES) >= HEADER_BYTES
    }

    /// How many bytes the frame recorded for this region. Unit: bytes.
    #[must_use]
    pub fn bytes(&self) -> u32 {
        self.get(OFF_BYTES)
    }

    /// How many bytes are handed out right now. Unit: bytes.
    #[must_use]
    pub fn live(&self) -> u32 {
        self.get(OFF_LIVE)
    }

    /// The most that was ever handed out at once. Unit: bytes.
    #[must_use]
    pub fn peak(&self) -> u32 {
        self.get(OFF_PEAK)
    }

    /// Has an allocation ever been refused for want of room?
    #[must_use]
    pub fn starved(&self) -> bool {
        // SAFETY: the type's contract, two bytes inside the prologue.
        let flags = unsafe { ((self.at + u64::from(OFF_FLAGS)) as *const u16).read_volatile() };
        flags & FLAG_STARVED != 0
    }

    /// Round a request up to a whole number of blocks.
    ///
    /// `None` for a request this allocator will not serve: a size that does not
    /// fit a `u32`, or an alignment wider than [`GRANULARITY`]. Refusing a wide
    /// alignment is a decision rather than an oversight — every block is
    /// sixteen-aligned by construction, and serving thirty-two would mean
    /// padding a block and remembering that it was padded, which is a second
    /// thing to store in a design whose whole argument is that it stores
    /// nothing outside the region.
    fn rounded(layout: &Layout) -> Option<u32> {
        if layout.align() > GRANULARITY as usize {
            return None;
        }
        let size = u32::try_from(layout.size().max(1)).ok()?;
        size.checked_add(GRANULARITY - 1).map(|padded| padded & !(GRANULARITY - 1))
    }

    /// Take a block of `want` bytes, or answer zero.
    ///
    /// Address-ordered first fit: the free list is kept sorted by offset, so the
    /// block taken is the lowest one that fits. That is what makes an allocation
    /// order a function of the **live set** rather than of the history that
    /// produced it — two different sequences of allocations and frees that arrive
    /// at the same live set have the same free list, so the next allocation is
    /// the same address. `user/virtio-net/src/driver.rs` says a free list is an
    /// allocation order and an allocation order a component chose is a place a
    /// seeded run stops reproducing; this is the form that answers it.
    fn take(&self, want: u32) -> u32 {
        let mut previous = 0u32;
        let mut block = self.get(OFF_FREE);
        while block != 0 {
            let size = self.get(block + OFF_SIZE);
            let next = self.get(block + OFF_NEXT);
            if size >= want {
                let remainder = size - want;
                if remainder >= GRANULARITY {
                    // Split, and the tail keeps the block's place in the order.
                    let tail = block + want;
                    self.put(tail + OFF_SIZE, remainder);
                    self.put(tail + OFF_NEXT, next);
                    self.relink(previous, tail);
                } else {
                    // The remainder is smaller than a block, so it goes out with
                    // the allocation rather than becoming a fragment nothing can
                    // ever use. `live` counts what was taken, not what was asked
                    // for, which is what makes the waste visible.
                    self.relink(previous, next);
                    return block;
                }
                return block;
            }
            previous = block;
            block = next;
        }
        0
    }

    /// Point `previous`'s successor at `block`, where zero means the list head.
    fn relink(&self, previous: u32, block: u32) {
        if previous == 0 {
            self.put(OFF_FREE, block);
        } else {
            self.put(previous + OFF_NEXT, block);
        }
    }

    /// Put a block back, keeping the list ordered and coalescing both sides.
    ///
    /// Immediate coalescing rather than deferred, for the reason first fit is
    /// address-ordered: a free list that depended on *when* a block came back
    /// would make the next allocation a function of history.
    fn give(&self, block: u32, size: u32) {
        let mut previous = 0u32;
        let mut next = self.get(OFF_FREE);
        while next != 0 && next < block {
            previous = next;
            next = self.get(next + OFF_NEXT);
        }

        let mut at = block;
        let mut span = size;

        // Forward: the block that follows, if it starts where this one ends.
        if next != 0 && at + span == next {
            span += self.get(next + OFF_SIZE);
            next = self.get(next + OFF_NEXT);
        }
        // Backward: the block before, if it ends where this one starts.
        if previous != 0 && previous + self.get(previous + OFF_SIZE) == at {
            span += self.get(previous + OFF_SIZE);
            at = previous;
            previous = self.walk_to(at);
        }

        self.put(at + OFF_SIZE, span);
        self.put(at + OFF_NEXT, next);
        self.relink(previous, at);
    }

    /// The block whose successor is `block`, or zero for the head.
    ///
    /// Walked rather than remembered, because the list is singly linked and a
    /// backward coalesce is the only caller. A doubly linked list would cost
    /// four more bytes in every free block and save this walk, and the walk is
    /// bounded by the number of free blocks — which is the number this design is
    /// betting stays small.
    fn walk_to(&self, block: u32) -> u32 {
        let mut previous = 0u32;
        let mut at = self.get(OFF_FREE);
        while at != 0 && at != block {
            previous = at;
            at = self.get(at + OFF_NEXT);
        }
        previous
    }
}

// SAFETY: `Heap` is an integer and nothing else. There is no interior
// mutability in the type, so sharing one across threads shares an address; what
// it addresses is a region the holder's contract says is theirs. A component is
// single-threaded by construction — it runs on one core with no second entry
// path — and the host tests hold one per region.
unsafe impl Sync for Heap {}

// SAFETY: the obligations `GlobalAlloc` states, discharged in order.
//
// - `alloc` answers either null or a pointer to `layout.size()` bytes, aligned
//   to `layout.align()`: every block is a multiple of `GRANULARITY` from a
//   `GRANULARITY`-aligned base, and an alignment wider than that is refused by
//   `rounded` rather than served badly.
// - A block is not handed out twice: `take` removes it from the free list before
//   returning it, and the bump only ever moves forward.
// - `dealloc` is only called with a pointer this allocator answered and the same
//   layout, which is the caller's obligation and is what lets a live block carry
//   no header of its own.
// - Re-entrancy: nothing in here allocates. The free list lives in the blocks it
//   describes, so there is no container to grow.
unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(want) = Self::rounded(&layout) else { return core::ptr::null_mut() };
        if !self.valid() {
            return core::ptr::null_mut();
        }

        let mut at = self.take(want);
        if at == 0 {
            // Nothing in the free list fits, so take new ground.
            let bump = self.get(OFF_BUMP);
            let Some(end) = bump.checked_add(want) else { return core::ptr::null_mut() };
            if end > self.bytes() {
                self.starve();
                return core::ptr::null_mut();
            }
            self.put(OFF_BUMP, end);
            at = bump;
        }

        let live = self.get(OFF_LIVE).saturating_add(want);
        self.put(OFF_LIVE, live);
        if live > self.get(OFF_PEAK) {
            self.put(OFF_PEAK, live);
        }
        (self.at + u64::from(at)) as *mut u8
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let Some(size) = Self::rounded(&layout) else { return };
        let at = (pointer as u64).wrapping_sub(self.at);
        let Ok(offset) = u32::try_from(at) else { return };
        if offset < HEADER_BYTES || offset >= self.bytes() {
            return;
        }
        self.give(offset, size);
        self.put(OFF_LIVE, self.get(OFF_LIVE).saturating_sub(size));
    }
}

impl Heap {
    /// Record that an allocation was refused for want of room.
    fn starve(&self) {
        // SAFETY: the type's contract, two bytes inside the prologue.
        let flags = unsafe { ((self.at + u64::from(OFF_FLAGS)) as *const u16).read_volatile() };
        // SAFETY: as above.
        unsafe {
            ((self.at + u64::from(OFF_FLAGS)) as *mut u16).write_volatile(flags | FLAG_STARVED);
        }
    }
}
