// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Physical memory: a buddy allocator with per-CPU free lists, working in
//! huge-page grain.
//!
//! This is the allocator `docs/design/deadline-all-the-way-down.html` section
//! 03 names. The M1 floor it replaces — one clean list, one dirty list, one
//! frame size, one core — is gone rather than kept beside it, because two
//! structures holding free memory are two structures that can disagree about
//! how much there is. RFC 0027 is the decision, and what it argues is that the
//! two questions a buddy allocator normally answers with a bitmap and a lock
//! can be answered with neither.
//!
//! # The allocator, and why it still has no metadata
//!
//! A free block is storage nobody is using, so the free lists live *in* the
//! free blocks: the first word of each one holds the address of the next.
//! There is no bitmap to size, no array to place, and no bootstrap problem
//! where the allocator needs an allocator.
//!
//! That property survived the change from one list to nineteen, and it is the
//! part of this module most worth defending. A textbook buddy allocator keeps
//! a bit per pair per order so that freeing a block can ask *is my buddy also
//! free* in constant time. Two things here make that bitmap the wrong answer
//! rather than merely an expensive one:
//!
//! - It is proportional to how much memory the machine turned out to have —
//!   about one bit per frame, half a mebibyte on the sixteen-gibibyte machine
//!   in `docs/first-boot-outside-qemu.md` — so it must be allocated from
//!   something, and the only thing that could allocate it is this.
//! - It is *machine-wide*, and the free lists are not. A bitmap two cores
//!   both consult is either locked or racy, and this kernel has no locks
//!   (RFC 0016). Sharding it per core would mean a bit whose owner changes
//!   when a block does, which is the metadata problem again with a
//!   concurrency problem stapled to it.
//!
//! So the question is not answered on the free path at all. See below, and
//! RFC 0027.
//!
//! # Coalescing is a pass, not a test
//!
//! [`FrameAllocator::free`] pushes a block onto a list and does nothing else.
//! Buddies are found later, by [`FrameAllocator::coalesce`], which sorts each
//! order's list by address and pairs off neighbours that are buddies — no
//! metadata, no lookup, and no question asked about a block somebody else owns.
//!
//! Reading the *buddy's own first word* to see whether it looks free was
//! considered and refused. A block that is allocated belongs to somebody who
//! may write anything into it, including bytes that look exactly like a free
//! block's header; the masking cookie below is not a secret from a component
//! that can read the boot log, and an allocator whose correctness rests on a
//! guess about somebody else's bytes has no correctness at all.
//!
//! Deferring costs fragmentation between passes and buys a free path that is a
//! store and an increment. It is the same trade `scrub` already makes, for the
//! same reason: erasing and merging are memory bandwidth, and memory bandwidth
//! is a resource this system means to schedule rather than spend silently.
//!
//! # Huge pages by default
//!
//! [`Order::DEFAULT`] is [`Order::HUGE`], and it is the grain a *shard* is
//! refilled in rather than a size callers are made to ask for. A core that
//! wants one frame and has none takes two mebibytes off the frontier and
//! splits it down, so the 511 frames beside it are already the caller's core's
//! and already adjacent — which is what makes the coalescing pass able to put
//! the huge page back together afterwards. An allocator that took single
//! frames from the frontier would be one whose blocks had no buddies to find.
//!
//! # The frontier: frames nobody asked for are not written
//!
//! Threading a list at boot means writing one word into every frame, and on
//! the first machine with real memory that write was most of the boot: under a
//! hypervisor, touching a page is what makes the host commit it, and touching
//! every page of a 16 GiB guest serially cost two to three minutes before the
//! kernel had done anything (RFC 0023, `docs/first-boot-outside-qemu.md`).
//!
//! So [`FrameAllocator::add_region`] does not thread frames. It walks and
//! *decides* exactly as it always has — the per-frame filter is untouched —
//! and coalesces its acceptances into runs, held in a small array inside the
//! allocator. A block's link word is first written when it first reaches a
//! free list; a block taken straight off a run reaches its owner without this
//! allocator ever having written to it.
//!
//! The buddy orders made that cheaper rather than dearer. Refilling a shard
//! with a two-mebibyte block writes nine link words — one per buddy left
//! behind on the way down — and serves 512 frames. The old eager threading
//! wrote 512.
//!
//! # The link is not a plain pointer
//!
//! Storing the lists inside the blocks means the link word lives in memory
//! that has recently belonged to somebody else, and will belong to somebody
//! else again. A corrupted link is not a crash — it is a block handed out at
//! an address the corruptor chose. So the stored word is the next address
//! exclusive-or'd with a per-boot value drawn from the [`Env`], which turns a
//! useful overwrite into an unpredictable one. The value comes from the
//! environment rather than from hardware so that a replayed run corrupts and
//! recovers identically; a defence that made the system irreproducible would
//! cost more than it bought.
//!
//! It is a defence against corruption and **not** an authenticator. Nothing in
//! this module decides anything on the strength of a word it did not write.
//!
//! # Clean and dirty, through split and coalesce
//!
//! There are two sets of lists, not one. A freed block goes on the *dirty*
//! lists holding whatever its last owner left. [`FrameAllocator::scrub`] moves
//! blocks from there to the *clean* lists, zeroing each as it goes, and
//! [`FrameAllocator::alloc_zeroed`] takes from the clean lists when it can and
//! zeroes inline when it cannot.
//!
//! The invariant that makes this work at every order: **every byte of a block
//! on a clean list is zero, except its first eight, which hold its masked
//! link.** Splitting preserves it because splitting writes no byte of either
//! half except the link words the lists need — the lower half inherits its
//! parent's first word and the upper half's first word was zero — so the
//! children of a clean block are clean, and cleanliness is a fact about bytes
//! rather than a label carried beside them. Coalescing preserves it because
//! the pass zeroes the *upper* half's link word before merging, that word
//! being the one byte range inside the merged block that would otherwise not
//! be zero. `alloc_zeroed` clears the first word on the way out, and that is
//! the whole of the accounting.
//!
//! Scrubbing exists as a separate step because zeroing is memory bandwidth,
//! and the design has it as a batch-class consumer that runs when nothing with
//! a deadline wants the controller. There is no such scheduler yet; what
//! matters now is that the interface is the one the scheduler will use, and
//! that no block reaches a new owner still carrying the old owner's bytes.
//!
//! # Per-CPU, and the three paths
//!
//! The lists are a [`PerCpu`] shard. Allocation and freeing touch
//! `PerCpu::mine()` and nothing else, so two cores allocating at the same
//! instant write to different cache lines and different frames, and there is
//! no lock because there is nothing to exclude. That is the hot path and it is
//! *counted*, not asserted: [`FrameAllocator::served_count`] is what it
//! answered, and the two counters beside it are what it cost when it could
//! not.
//!
//! A shard that has nothing reaches past itself twice, in this order:
//!
//! - **The frontier**, which is machine-wide because the memory map is.
//!   [`FrameAllocator::refill_count`] counts it. A refill moves a whole
//!   default-grain block, so this is rare by construction rather than by hope.
//! - **Another core's shard**, when the frontier is spent.
//!   [`FrameAllocator::remote_count`] counts it, and a boot that never took
//!   this path says so with a zero.
//!
//! A block freed on a core that did not allocate it stays on the freeing
//! core's shard. That is deliberate and it is the reason there is no
//! remote-free queue: a queue would put cross-core traffic on the *free* path,
//! which is as hot as the allocation path and is reached from the same places.
//! The cost is that two buddies can end up on two shards and never merge,
//! which is fragmentation rather than incorrectness, and the steal path is
//! what stops it becoming a machine that has memory and cannot allocate. RFC
//! 0027 states both.
//!
//! Today exactly one core mutates this structure at a time — the allocator is
//! reached through one `&mut`, lent to a running process's core as a `&` that
//! only computes addresses ([`crate::process`]). The sharding is here before
//! the second mutator for the reason `PerCpu` itself was: retrofitting it is
//! the miserable refactor, and it arrives on the day the kernel is also being
//! debugged on two cores for the first time.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_env::Env;

use crate::arch::x86_64::current_cpu;
use crate::percpu::{MAX_CPUS, PerCpu};

/// Bytes per frame. One 4 KiB page, and the unit every order is a power of.
pub const FRAME_SIZE: u64 = 4096;

/// The top of the boot stub's identity map.
///
/// The stub maps the first gigabyte so that the jump to long mode is legal.
/// Until the kernel builds its own tables, a frame above this line is real
/// memory it cannot touch — so it cannot hold a free-list link either, and the
/// allocator counts it rather than pretending.
///
/// After [`FrameAllocator::rebind`] the limit is the direct map's, which is all
/// of memory, and those frames are claimed.
pub const IDENTITY_LIMIT: u64 = 1 << 30;

/// Nothing below this is ever handed out.
///
/// The first mebibyte is the real-mode interrupt table, the BIOS data area, the
/// extended BIOS data area, video memory and option ROMs. Some of it is
/// reported as usable and none of it is worth the argument.
pub const LOW_MEMORY_LIMIT: u64 = 1 << 20;

/// How large a run of frames is, as a power of two.
///
/// Order 0 is one frame, order 9 is a 2 MiB page, order 18 is a gibibyte.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Order(u8);

impl Order {
    /// One frame: 4 KiB.
    pub const FRAME: Self = Self(0);

    /// The grain the page tables actually use: 2 MiB.
    ///
    /// Named rather than spelled `Order::new(9)` at each call site, because the
    /// relationship between this constant and the page tables is the reason it
    /// is the default grain in the design, and a bare 9 does not say so.
    pub const HUGE: Self = Self(9);

    /// The grain a shard is refilled in when the caller did not say otherwise.
    ///
    /// The design's "huge pages by default" is this constant and not a default
    /// argument on [`FrameAllocator::alloc`]: a caller that wants one frame
    /// must be given one frame, or a page table would cost two mebibytes. What
    /// "by default" buys is that the frame it is given comes out of a huge
    /// block this core already owns, so its 511 neighbours are adjacent, on
    /// the same shard, and able to become a huge page again.
    ///
    /// *Reversal:* a machine whose translation buffer makes some other order
    /// the natural grain, or a workload that measurably wastes memory holding
    /// two mebibytes per core it never splits.
    pub const DEFAULT: Self = Self::HUGE;

    /// The largest order this system will ever name: one gibibyte.
    pub const MAX: u8 = 18;

    /// Name an order, or `None` past [`Self::MAX`].
    #[must_use]
    pub const fn new(order: u8) -> Option<Self> {
        if order <= Self::MAX { Some(Self(order)) } else { None }
    }

    /// The order as a number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// How many frames this order covers.
    #[must_use]
    pub const fn frames(self) -> u64 {
        1 << self.0
    }

    /// How many bytes it covers.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        FRAME_SIZE << self.0
    }

    /// The order above this one, or `None` at [`Self::MAX`].
    const fn up(self) -> Option<Self> {
        if self.0 < Self::MAX { Some(Self(self.0 + 1)) } else { None }
    }

    /// The order below this one, or `None` at [`Self::FRAME`].
    const fn down(self) -> Option<Self> {
        if self.0 == 0 { None } else { Some(Self(self.0 - 1)) }
    }

    /// Where this order's list head lives.
    const fn index(self) -> usize {
        self.0 as usize
    }

    /// Is `addr` the base of a block of this order?
    const fn aligns(self, addr: u64) -> bool {
        addr.is_multiple_of(self.bytes())
    }

    /// Is a block of this order at `addr` the *lower* half of its pair?
    ///
    /// Only the lower half may absorb the upper, which is what makes the
    /// coalescing walk able to decide a pair by looking at two addresses.
    const fn is_lower(self, addr: u64) -> bool {
        addr & self.bytes() == 0
    }
}

/// How many orders there are, which is how long every per-order array is.
const ORDERS: usize = Order::MAX as usize + 1;

/// A run of physical frames, and how long it is.
///
/// A physical address with a name and a length, so that it cannot be passed
/// where a virtual address is meant and cannot be freed at the wrong size.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Frame {
    base: u64,
    order: Order,
}

impl Frame {
    /// The physical address of the first byte.
    #[must_use]
    pub const fn addr(self) -> u64 {
        self.base
    }

    /// How large this run is.
    #[must_use]
    pub const fn order(self) -> Order {
        self.order
    }

    /// How many bytes it covers.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.order.bytes()
    }

    /// Name a single frame by its physical address.
    ///
    /// For code that already holds a physical address for its own reasons —
    /// the page tables, which are frames but are never on a free list.
    #[must_use]
    pub const fn from_addr(addr: u64) -> Self {
        Self { base: addr, order: Order::FRAME }
    }

    /// Name a block by its physical address and its order.
    ///
    /// The general form of [`Self::from_addr`], and the shape a caller needs
    /// when it is handing part of a larger block back — see
    /// [`FrameAllocator::free`], which accepts a block returned as any set of
    /// aligned sub-blocks that tile it.
    #[must_use]
    pub const fn block(addr: u64, order: Order) -> Self {
        Self { base: addr, order }
    }
}

/// A range of physical memory that must not be handed out.
#[derive(Clone, Copy, Debug)]
pub struct Reserved {
    /// First byte.
    pub base: u64,
    /// One past the last byte.
    pub end: u64,
}

impl Reserved {
    /// A range from a base and a length, saturating rather than wrapping —
    /// these numbers come from a loader and are not to be trusted with
    /// arithmetic.
    #[must_use]
    pub const fn new(base: u64, len: u64) -> Self {
        Self { base, end: base.saturating_add(len) }
    }

    /// An empty range, which reserves nothing.
    ///
    /// Padding for a fixed-size reservation list that is not always full — a
    /// list on the stack has a length the type knows and a length the caller
    /// filled, and an entry that overlaps nothing is the honest way to say
    /// "this slot is unused" without a second count travelling alongside.
    #[must_use]
    pub const fn empty() -> Self {
        Self { base: 0, end: 0 }
    }

    const fn overlaps(&self, base: u64, end: u64) -> bool {
        base < self.end && self.base < end
    }
}

// --- reaching a frame ------------------------------------------------------

/// How a physical address is reached, and how a link word is masked.
///
/// Copied out of the allocator before any list is touched, so that the list
/// code borrows one field of the allocator and this borrows none — which is
/// what lets a shard be reached through a raw pointer while the machine-wide
/// frontier is held by `&mut`, without the two aliasing.
#[derive(Clone, Copy, Debug)]
struct Window {
    /// Added to a physical address to reach it.
    phys_offset: u64,
    /// One past the highest physical address currently reachable.
    limit: u64,
    /// Mixed into every stored link. See the module documentation.
    cookie: u64,
}

impl Window {
    /// Where a physical address can be read and written.
    fn virt(&self, addr: u64) -> *mut u8 {
        (addr + self.phys_offset) as *mut u8
    }

    /// Write the link word of a free block, masked.
    ///
    /// # Safety
    ///
    /// `block` must be addressable and owned by nobody but the allocator.
    unsafe fn write_link(&self, block: u64, next: u64) {
        let at = self.virt(block).cast::<u64>();
        // SAFETY: the caller has established that the block is unowned and
        // addressable, so its first word is ours.
        unsafe { at.write_volatile(next ^ self.cookie) };
    }

    /// Write a literal zero over the link word.
    ///
    /// Not `write_link(block, 0)`: that stores the cookie, which is a perfectly
    /// good link and a perfectly bad first word for a block that is about to be
    /// handed over as empty — or absorbed into a clean block that must be zero
    /// throughout.
    ///
    /// # Safety
    ///
    /// As [`Self::write_link`].
    unsafe fn clear_link(&self, block: u64) {
        let at = self.virt(block).cast::<u64>();
        // SAFETY: the caller has established that the block is unowned and
        // addressable, so its first word is ours.
        unsafe { at.write_volatile(0) };
    }

    /// Read the link word of a free block, unmasked.
    ///
    /// # Safety
    ///
    /// As [`Self::write_link`], and the word must be one `write_link` wrote.
    unsafe fn read_link(&self, block: u64) -> u64 {
        let at = self.virt(block).cast::<u64>();
        // SAFETY: as above; this word was written by `write_link`.
        let stored = unsafe { at.read_volatile() };
        stored ^ self.cookie
    }

    /// Write zeroes over a whole block.
    ///
    /// # Safety
    ///
    /// `block` must be addressable for `order.bytes()` and owned by nobody
    /// else.
    unsafe fn zero(&self, block: u64, order: Order) {
        let at = self.virt(block);
        let len = usize::try_from(order.bytes()).unwrap_or(usize::MAX);
        // SAFETY: the caller owns the block, so its whole extent is ours to
        // write, and `virt` is where it is mapped.
        unsafe { core::ptr::write_bytes(at, 0, len) };
    }
}

// --- the free lists --------------------------------------------------------

/// One head per order.
///
/// Reached by value rather than by `&mut u64`, because an index that `Order`
/// has already bounded is still an index clippy will not let this module
/// panic on, and `Option<&mut u64>` at every call site would be a bound
/// checked twice and handled once.
#[derive(Clone, Copy, Debug)]
struct Lists {
    heads: [u64; ORDERS],
}

impl Lists {
    const EMPTY: Self = Self { heads: [0; ORDERS] };

    /// The head of `order`'s list, or zero for none.
    ///
    /// Zero is unambiguous because nothing below [`LOW_MEMORY_LIMIT`] is ever
    /// on a list. An order this array cannot hold reads as an empty list,
    /// which cannot happen — `Order::new` refuses above `Order::MAX` and this
    /// array is `MAX + 1` long — and is the answer that costs nothing if it
    /// ever does.
    fn head(&self, order: Order) -> u64 {
        match self.heads.get(order.index()) {
            Some(head) => *head,
            None => 0,
        }
    }

    fn set_head(&mut self, order: Order, block: u64) {
        if let Some(head) = self.heads.get_mut(order.index()) {
            *head = block;
        }
    }
}

/// One core's free memory, and what reaching past it has cost.
#[derive(Clone, Copy, Debug)]
struct Shard {
    /// Blocks known to be zero except for their link word.
    clean: Lists,
    /// Blocks holding whatever their last owner left.
    dirty: Lists,
    clean_frames: u64,
    dirty_frames: u64,
    /// Allocations this shard answered out of its own lists.
    served: u64,
    /// Allocations that had to reach the machine-wide frontier.
    refills: u64,
    /// Allocations that had to reach another core's shard.
    remote: u64,
    /// Blocks split into two.
    splits: u64,
    /// Pairs of buddies merged into one.
    merges: u64,
}

impl Shard {
    const EMPTY: Self = Self {
        clean: Lists::EMPTY,
        dirty: Lists::EMPTY,
        clean_frames: 0,
        dirty_frames: 0,
        served: 0,
        refills: 0,
        remote: 0,
        splits: 0,
        merges: 0,
    };

    fn lists(&mut self, clean: bool) -> &mut Lists {
        if clean { &mut self.clean } else { &mut self.dirty }
    }

    fn frames(&mut self, clean: bool) -> &mut u64 {
        if clean { &mut self.clean_frames } else { &mut self.dirty_frames }
    }
}

/// Put a block on one of a shard's lists, without touching the frame counts.
///
/// The counts are the caller's business because splitting and coalescing move
/// blocks between orders without a frame entering or leaving the shard, and a
/// count updated inside the list operation would have to be undone by both.
///
/// # Safety
///
/// `block` must be an addressable block of `order`, aligned to it, that
/// nothing else owns.
unsafe fn list_push(window: &Window, lists: &mut Lists, order: Order, block: u64) {
    let next = lists.head(order);
    // SAFETY: the caller has established that the block is unowned and
    // addressable, so its first word is ours to use as the list link.
    unsafe { window.write_link(block, next) };
    lists.set_head(order, block);
}

/// Take the head of one of a shard's lists.
///
/// # Safety
///
/// Every block on the list must be addressable through the window, which is
/// the invariant [`FrameAllocator::rebind`] carries.
unsafe fn list_pop(window: &Window, lists: &mut Lists, order: Order) -> Option<u64> {
    let block = lists.head(order);
    if block == 0 {
        return None;
    }
    // SAFETY: this is a block the allocator put on its own list, so its first
    // word is the link `list_push` wrote there.
    let next = unsafe { window.read_link(block) };
    lists.set_head(order, next);
    Some(block)
}

/// Give a block to a shard: onto the list, and into the count.
///
/// # Safety
///
/// As [`list_push`], and a block given to a clean list must actually be zero
/// throughout.
unsafe fn give(window: &Window, shard: &mut Shard, order: Order, clean: bool, block: u64) {
    // SAFETY: the caller's guarantee, unchanged.
    unsafe { list_push(window, shard.lists(clean), order, block) };
    let frames = order.frames();
    let count = shard.frames(clean);
    *count = count.saturating_add(frames);
}

/// Split a block down to `order`, leaving each upper half on the list.
///
/// # Safety
///
/// `block` must be an addressable, `from`-aligned block of order `from` that
/// nothing else owns, and `to` must not exceed `from`.
unsafe fn split_down(
    window: &Window,
    shard: &mut Shard,
    clean: bool,
    block: u64,
    from: Order,
    to: Order,
) {
    let mut at = from;
    while at > to {
        let Some(lower) = at.down() else { break };
        let upper = block + lower.bytes();
        // SAFETY: the upper half of a block the caller owns, aligned to the
        // lower order by construction, and not otherwise reachable.
        unsafe { list_push(window, shard.lists(clean), lower, upper) };
        shard.splits = shard.splits.saturating_add(1);
        at = lower;
    }
}

/// Take a block of exactly `order` off one kind of list, splitting a larger
/// one if that is all there is.
///
/// # Safety
///
/// Every block on the shard's lists must be addressable through the window.
unsafe fn pop_split(window: &Window, shard: &mut Shard, order: Order, clean: bool) -> Option<u64> {
    let mut at = order;
    loop {
        // SAFETY: the caller's guarantee.
        if let Some(block) = unsafe { list_pop(window, shard.lists(clean), at) } {
            // SAFETY: the block is off the list, so nothing else holds it.
            unsafe { split_down(window, shard, clean, block, at, order) };
            let frames = order.frames();
            let count = shard.frames(clean);
            *count = count.saturating_sub(frames);
            return Some(block);
        }
        at = at.up()?;
    }
}

/// Cut `start..end` into aligned blocks and give them to a shard, dirty.
///
/// The largest block that both fits and is aligned, repeatedly — so a range of
/// N frames costs about two writes per order rather than one per frame, which
/// is what makes the two callers that use it (a memory map fragmented past the
/// runs array, and the alignment gap a huge-grain refill skips) cheap enough
/// not to need their own argument.
///
/// # Safety
///
/// Every frame in `start..end` must be addressable, unowned, and something the
/// allocator is entitled to hand out.
unsafe fn carve(window: &Window, shard: &mut Shard, start: u64, end: u64) {
    let mut at = start;
    while at < end {
        let mut order = Order::FRAME;
        while let Some(up) = order.up() {
            if !up.aligns(at) {
                break;
            }
            match at.checked_add(up.bytes()) {
                Some(stop) if stop <= end => order = up,
                _ => break,
            }
        }
        // SAFETY: `at` is inside the caller's range, aligned to `order`, and
        // the block it names ends at or before `end`.
        unsafe { give(window, shard, order, false, at) };
        at += order.bytes();
    }
}

// --- sorting a free list ---------------------------------------------------

/// Merge two address-sorted lists into one.
///
/// # Safety
///
/// Every block on either list must be addressable through the window and owned
/// by nobody but the allocator.
unsafe fn merge_sorted(window: &Window, mut a: u64, mut b: u64) -> u64 {
    if a == 0 {
        return b;
    }
    if b == 0 {
        return a;
    }
    let head = if a <= b {
        let block = a;
        // SAFETY: a block on a list the allocator wrote.
        a = unsafe { window.read_link(block) };
        block
    } else {
        let block = b;
        // SAFETY: as above.
        b = unsafe { window.read_link(block) };
        block
    };
    let mut tail = head;
    while a != 0 && b != 0 {
        let take = if a <= b {
            let block = a;
            // SAFETY: as above.
            a = unsafe { window.read_link(block) };
            block
        } else {
            let block = b;
            // SAFETY: as above.
            b = unsafe { window.read_link(block) };
            block
        };
        // SAFETY: `tail` is off both input lists and owned by the allocator.
        unsafe { window.write_link(tail, take) };
        tail = take;
    }
    let rest = if a != 0 { a } else { b };
    // SAFETY: as above.
    unsafe { window.write_link(tail, rest) };
    head
}

/// Sort a free list by address, ascending.
///
/// Bottom-up merge sort with the partial results held in a fixed array of
/// list heads, which is what makes it need no storage proportional to the
/// list: the blocks being sorted are the storage. `O(n log n)` link
/// operations, no recursion, and no allocation — the three properties that
/// matter in a kernel with a guard page and no heap.
///
/// # Safety
///
/// Every block on the list must be addressable through the window and owned by
/// nobody but the allocator.
unsafe fn sort(window: &Window, mut head: u64) -> u64 {
    // One slot per doubling. Sixty-four of them is more blocks than any
    // machine can address, so the carry below cannot bind.
    let mut pending = [0u64; 64];

    while head != 0 {
        // SAFETY: a block on a list the allocator wrote.
        let next = unsafe { window.read_link(head) };
        // SAFETY: the block is off the list now; its first word is ours.
        unsafe { window.write_link(head, 0) };

        let mut carry = head;
        for slot in &mut pending {
            if *slot == 0 {
                *slot = carry;
                carry = 0;
                break;
            }
            // SAFETY: two sorted lists of blocks the allocator owns.
            carry = unsafe { merge_sorted(window, *slot, carry) };
            *slot = 0;
        }
        if carry != 0 {
            // Every slot was consumed into `carry`, so every slot is now
            // empty. Putting it back rather than dropping it is what makes
            // the sentence above a statement about arithmetic and not a leak.
            if let Some(slot) = pending.first_mut() {
                *slot = carry;
            }
        }
        head = next;
    }

    let mut out = 0;
    for slot in pending {
        // SAFETY: two sorted lists of blocks the allocator owns.
        out = unsafe { merge_sorted(window, out, slot) };
    }
    out
}

/// Merge every buddy pair on one shard, smallest order first, up to `budget`
/// merges.
///
/// Smallest first because a pair merged at order k is a candidate at order
/// k+1, so one upward sweep finds everything a repeated sweep would.
///
/// # Safety
///
/// Every block on the shard's lists must be addressable through the window.
unsafe fn coalesce_shard(window: &Window, shard: &mut Shard, budget: u64) -> u64 {
    let mut merged = 0;
    let mut order = Order::FRAME;

    while let Some(up) = order.up() {
        for clean in [false, true] {
            if merged >= budget {
                break;
            }
            let head = shard.lists(clean).head(order);
            shard.lists(clean).set_head(order, 0);
            // SAFETY: every block on the list is one the allocator put there.
            let mut at = unsafe { sort(window, head) };

            while at != 0 {
                // SAFETY: as above.
                let next = unsafe { window.read_link(at) };
                let paired = next != 0
                    && merged < budget
                    && order.is_lower(at)
                    && next == at + order.bytes();
                if paired {
                    // SAFETY: as above.
                    let after = unsafe { window.read_link(next) };
                    if clean {
                        // The upper half's link word is the one byte range
                        // inside the merged block that is not zero, and the
                        // clean invariant is a statement about every byte.
                        // SAFETY: the block is off the list and unowned.
                        unsafe { window.clear_link(next) };
                    }
                    // SAFETY: the lower half is off the list, aligned to the
                    // order above by `is_lower`, and now names both halves.
                    unsafe { list_push(window, shard.lists(clean), up, at) };
                    shard.merges = shard.merges.saturating_add(1);
                    merged += 1;
                    at = after;
                } else {
                    // SAFETY: the block is off the list and unowned.
                    unsafe { list_push(window, shard.lists(clean), order, at) };
                    at = next;
                }
            }
        }
        order = up;
    }
    merged
}

// --- the frontier ----------------------------------------------------------

/// A run of accepted frames the allocator has never written to.
///
/// Produced by [`FrameAllocator::add_region`] coalescing the per-frame
/// filter's consecutive acceptances, consumed a block at a time when a shard
/// has nothing better. See the module documentation and RFC 0023.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// The next address to hand out.
    next: u64,
    /// One past the last byte of the run.
    end: u64,
}

/// How many runs the allocator can hold before falling back to carving them
/// onto a free list at boot.
///
/// A run ends where a reserved range, the addressable limit, or the region
/// does, so a real memory map produces a few dozen at most — sixteen regions
/// and twelve reservations on the machine that motivated this. The array is a
/// kibibyte inside the allocator, which lives on the boot stack.
const UNTOUCHED_RUNS: usize = 64;

/// Accepted memory nothing has been written to yet.
///
/// Machine-wide, because the memory map is. A shard reaches it when it has
/// nothing of its own, and [`FrameAllocator::refill_count`] is how often that
/// happened — the first of the two numbers the exit criterion asks for.
struct Frontier {
    /// Newest run last. Consumed from the last run that can serve the order
    /// asked for, ascending within it — deterministic, like everything else.
    runs: [Run; UNTOUCHED_RUNS],
    /// How many entries of `runs` are live. Live runs are never empty.
    count: usize,
    /// Frames inside `runs`, so the counts stay one field read.
    frames: u64,
}

impl Frontier {
    const EMPTY: Self =
        Self { runs: [Run { next: 0, end: 0 }; UNTOUCHED_RUNS], count: 0, frames: 0 };

    /// Record a run, or refuse when the array is full.
    fn accept(&mut self, start: u64, end: u64) -> bool {
        if self.count >= UNTOUCHED_RUNS {
            return false;
        }
        let Some(slot) = self.runs.get_mut(self.count) else { return false };
        *slot = Run { next: start, end };
        self.count += 1;
        self.frames = self.frames.saturating_add((end - start) / FRAME_SIZE);
        true
    }

    /// Take an aligned block of `order`, and say which frames were skipped to
    /// align it.
    ///
    /// Returns `(block, gap_start, gap_end)`. The gap is frames the alignment
    /// stepped over; the caller must carve them onto a free list, because they
    /// have left this structure's accounting and are still memory.
    fn take(&mut self, order: Order) -> Option<(u64, u64, u64)> {
        let size = order.bytes();
        let mut index = self.count;
        while index > 0 {
            index -= 1;
            let Some(run) = self.runs.get(index) else { continue };
            let start = run.next;
            let end = run.end;
            let Some(aligned) = start.checked_next_multiple_of(size) else { continue };
            let Some(stop) = aligned.checked_add(size) else { continue };
            if stop > end {
                continue;
            }

            self.frames = self.frames.saturating_sub((stop - start) / FRAME_SIZE);
            let spent = stop >= end;
            if let Some(run) = self.runs.get_mut(index) {
                run.next = stop;
            }
            if spent {
                // The emptied entry is replaced by the last live one rather
                // than shifting the array: a live run is never empty, and
                // which run is where is not a fact anything else depends on.
                let last = self.count - 1;
                if index != last
                    && let Some(moved) = self.runs.get(last).copied()
                    && let Some(slot) = self.runs.get_mut(index)
                {
                    *slot = moved;
                }
                self.count = last;
            }
            return Some((aligned, start, aligned));
        }
        None
    }
}

// --- the allocator ---------------------------------------------------------

/// Free lists of physical frames, sharded by core.
///
/// The lists hold *physical* addresses, which is why the window exists: the
/// same lists are walked before and after the kernel takes over the page
/// tables, and only the way a block is reached changes. Under the boot stub's
/// identity window the offset is zero; under the direct map it is
/// [`crate::arch::x86_64::paging::PHYS_OFFSET`]. Nothing stored has to be
/// rewritten at the switch, which is the property that makes the switch a
/// two-field assignment rather than a migration.
pub struct FrameAllocator {
    /// One core's lists, and only that core's.
    shards: PerCpu<Shard>,
    /// Memory accepted and never written to.
    frontier: Frontier,
    /// How a physical address is reached and how a link is masked.
    map: Window,
    total: u64,
    unreachable: u64,
}

impl FrameAllocator {
    /// An allocator with nothing in it, addressing frames through the boot
    /// stub's identity window.
    ///
    /// `cookie` should come from [`Env`] rather than from a constant or from
    /// the hardware: from a constant it is not a defence, and from the hardware
    /// it is a source of nondeterminism the substrate does not permit.
    #[must_use]
    pub const fn new(cookie: u64) -> Self {
        Self {
            shards: PerCpu::new(Shard::EMPTY),
            frontier: Frontier::EMPTY,
            map: Window { phys_offset: 0, limit: IDENTITY_LIMIT, cookie },
            total: 0,
            unreachable: 0,
        }
    }

    /// Where a block can be read and written.
    ///
    /// The one place physical becomes virtual. Everything else in the kernel
    /// that needs to touch a frame asks here rather than assuming the two are
    /// equal — an assumption that was true until E0-B04 and is now false.
    #[must_use]
    pub fn virt(&self, frame: Frame) -> *mut u8 {
        self.map.virt(frame.addr())
    }

    /// Point the allocator at a new window onto physical memory.
    ///
    /// Called immediately after `CR3` changes, and paired with it: between the
    /// switch and this call the allocator's view of memory is wrong, so nothing
    /// may allocate or free in between.
    ///
    /// # Safety
    ///
    /// `phys_offset` must be the base of a mapping that covers every frame this
    /// allocator holds or will be given, and `limit` must not exceed what that
    /// mapping reaches — which is what
    /// [`AddressSpace::direct_limit`](crate::arch::x86_64::paging::AddressSpace::direct_limit)
    /// exists to report. Getting either wrong turns a free list into a walk
    /// through unmapped memory, discovered at the first allocation.
    pub unsafe fn rebind(&mut self, phys_offset: u64, limit: u64) {
        self.map.phys_offset = phys_offset;
        self.map.limit = limit;
        self.unreachable = 0;
    }

    /// Add up every shard's answer to one question.
    ///
    /// Reading a shard this core does not own, which is what the whole module
    /// is arranged to avoid — so it is worth being exact about why it is
    /// sound and why it is not the thing the exit criterion counts. It is
    /// sound because exactly one core mutates this structure at a time, and it
    /// is not counted because it is not on the allocation path: how much
    /// memory the *machine* has free is a machine-wide question, and no
    /// arrangement of shards makes it a local one.
    fn sum(&self, pick: fn(&Shard) -> u64) -> u64 {
        let mut total = 0u64;
        for cpu in 0..MAX_CPUS {
            // SAFETY: a shared read of a shard, on the one core that is
            // mutating this allocator, so no `&mut` to any slot is live.
            let shard = unsafe { &*self.shards.at(cpu) };
            total = total.saturating_add(pick(shard));
        }
        total
    }

    /// Frames available now, zeroed or not.
    #[must_use]
    pub fn free_count(&self) -> u64 {
        self.frontier.frames.saturating_add(self.sum(|s| s.clean_frames + s.dirty_frames))
    }

    /// Frames available and known to be zero.
    #[must_use]
    pub fn clean_count(&self) -> u64 {
        self.sum(|s| s.clean_frames)
    }

    /// Frames available that still hold what their last owner wrote.
    ///
    /// The frames on the frontier count here too: their last owner is the
    /// firmware, and "probably zero" is not a property this allocator hands
    /// anybody. The split between listed and untouched is an implementation
    /// fact, not a guarantee, so it is not published.
    #[must_use]
    pub fn dirty_count(&self) -> u64 {
        self.frontier.frames.saturating_add(self.sum(|s| s.dirty_frames))
    }

    /// Frames this allocator has ever been given.
    #[must_use]
    pub const fn total_count(&self) -> u64 {
        self.total
    }

    /// Usable frames that were skipped because they lie above
    /// [`IDENTITY_LIMIT`] and cannot currently be addressed.
    #[must_use]
    pub const fn unreachable_count(&self) -> u64 {
        self.unreachable
    }

    /// Allocations answered out of the calling core's own lists.
    ///
    /// The hot path, counted rather than asserted. Every one of these touched
    /// this core's shard and nothing else.
    #[must_use]
    pub fn served_count(&self) -> u64 {
        self.sum(|s| s.served)
    }

    /// Allocations that had to reach the machine-wide frontier.
    #[must_use]
    pub fn refill_count(&self) -> u64 {
        self.sum(|s| s.refills)
    }

    /// Allocations that had to reach another core's shard.
    ///
    /// The number the exit criterion is about: a boot whose allocation path
    /// took no cross-core traffic reads zero here.
    #[must_use]
    pub fn remote_count(&self) -> u64 {
        self.sum(|s| s.remote)
    }

    /// Blocks split into two, machine-wide.
    #[must_use]
    pub fn split_count(&self) -> u64 {
        self.sum(|s| s.splits)
    }

    /// Buddy pairs merged into one, machine-wide.
    #[must_use]
    pub fn merge_count(&self) -> u64 {
        self.sum(|s| s.merges)
    }

    /// How many free blocks of exactly `order` this core's shard holds.
    ///
    /// A walk, not a counter, and it exists for the self-test: the property
    /// "512 frames returned in pieces became one two-mebibyte block again" is
    /// not visible in any number the allocator keeps for its own sake, and a
    /// counter kept only so a test could read it would be a counter nothing
    /// maintains under pressure.
    #[must_use]
    pub fn free_blocks(&self, order: Order) -> u64 {
        let mut count = 0u64;
        // SAFETY: a shared read of this core's shard, on the one core mutating
        // this allocator.
        let shard = unsafe { &*self.shards.mine() };
        for lists in [&shard.clean, &shard.dirty] {
            let mut at = lists.head(order);
            while at != 0 {
                count = count.saturating_add(1);
                // SAFETY: a block on a list this allocator wrote.
                at = unsafe { self.map.read_link(at) };
            }
        }
        count
    }

    /// Add a region of usable memory, minus anything reserved.
    ///
    /// Filtering is per frame rather than by interval arithmetic, because the
    /// per-frame test cannot get the subtraction wrong — which interval
    /// arithmetic, done once, at four in the morning, can.
    ///
    /// It used to say the loop "costs nothing at boot" because "a machine has
    /// tens of thousands of frames and a handful of reserved ranges". The first
    /// machine this ever ran on outside an emulator had **four million** frames,
    /// and the scan ran once per frame per range. That sentence was an
    /// assumption about hardware written on a 128 MiB emulator, and
    /// `docs/first-boot-outside-qemu.md` is where it was measured.
    ///
    /// So the filter is only asked where its answer can be in doubt, and its
    /// acceptances are recorded as runs rather than written into the frames —
    /// two economies, each earned by its own measurement, neither a rewrite
    /// into interval arithmetic:
    ///
    /// - Overlapping requires `frame < r.end`, so at or above every reserved
    ///   end — and above [`LOW_MEMORY_LIMIT`] — a frame is usable with no
    ///   question to put, and the rest of the region is accepted as one run by
    ///   construction. Reserved ranges are few and clustered low, so this is
    ///   nearly everything.
    /// - Threading each accepted frame onto a free list wrote one word into
    ///   every frame of RAM, which under a hypervisor made the host commit the
    ///   whole guest at boot — minutes, measured, on the first 16 GiB machine.
    ///   RFC 0023.
    ///
    /// Frames arrive dirty. Most of memory is in fact zero at boot, but "the
    /// firmware probably left it that way" is not a property to hand a
    /// component, and claiming it here would make the guarantee depend on the
    /// machine rather than on the allocator.
    ///
    /// # Safety
    ///
    /// `base..base + len` must be memory the loader reported as usable, and the
    /// caller must have listed in `reserved` everything within it that is
    /// already spoken for: the kernel image, the loader's own structures, any
    /// module it loaded, and anything else still being read. A frame handed out
    /// from any of those is memory corruption with a delay fuse.
    pub unsafe fn add_region(&mut self, base: u64, len: u64, reserved: &[Reserved]) {
        let region_end = base.saturating_add(len);
        // One past the last whole frame in the region; the tail past it is
        // ignored, as it always was.
        let frames_end = region_end / FRAME_SIZE * FRAME_SIZE;
        let mut frame = base.next_multiple_of(FRAME_SIZE);

        // Below this line the filter must be asked; at or above it the answer
        // is known. `0` for an empty reserved list is correct: there is
        // nothing to overlap, and the low-memory bound still applies.
        let ceiling = reserved.iter().map(|r| r.end).max().unwrap_or(0);
        let threshold = ceiling.max(LOW_MEMORY_LIMIT);
        // One past the last byte an accepted frame may reach. The limit is
        // frame-aligned on every machine, and rounding down costs nothing
        // where it is not.
        let reach = self.map.limit / FRAME_SIZE * FRAME_SIZE;

        // The start of the run being coalesced, or zero for none. Zero cannot
        // be a run start, because nothing below `LOW_MEMORY_LIMIT` is ever
        // accepted.
        let mut run = 0u64;

        while frame < threshold && frame < frames_end {
            let end = frame + FRAME_SIZE;
            let usable =
                frame >= LOW_MEMORY_LIMIT && !reserved.iter().any(|r| r.overlaps(frame, end));

            if usable && end > reach {
                self.unreachable += 1;
            }
            if usable && end <= reach {
                if run == 0 {
                    run = frame;
                }
            } else if run != 0 {
                // SAFETY: every frame in `run..frame` passed the filter above,
                // inside a region the caller vouched for, below the limit.
                unsafe { self.accept(run, frame) };
                run = 0;
            }
            frame = end;
        }

        // From the threshold up every frame is usable by construction, so the
        // remainder is one extension of the open run — no frame is visited.
        let rest = frames_end.min(reach);
        if frame < rest {
            if run == 0 {
                run = frame;
            }
            frame = rest;
        }
        if run != 0 {
            // SAFETY: as above — per frame below the threshold, and by the
            // threshold's construction from there to `frame`.
            unsafe { self.accept(run, frame) };
        }
        if frame < frames_end {
            self.unreachable += (frames_end - frame) / FRAME_SIZE;
        }
    }

    /// Take ownership of a run of frames without touching any of them.
    ///
    /// # Safety
    ///
    /// Every frame in `start..end` must satisfy what [`Self::add_region`]
    /// demands of an accepted frame: inside a vouched-for region, overlapping
    /// nothing reserved, at or above [`LOW_MEMORY_LIMIT`], below the limit.
    unsafe fn accept(&mut self, start: u64, end: u64) {
        self.total = self.total.saturating_add((end - start) / FRAME_SIZE);
        if self.frontier.accept(start, end) {
            return;
        }

        // A memory map fragmented past the runs array is carved onto the boot
        // core's lists instead — about two writes per order rather than one
        // per frame, which is why this branch stopped being the expensive one
        // when the orders arrived. RFC 0023 states the bound so the fallback
        // is a documented cost, not a surprise.
        let window = self.map;
        // SAFETY: the calling core's own shard, on the boot path, with no
        // interrupt handler in this kernel reaching the frame allocator — so
        // no second reference to this slot is live.
        let shard = unsafe { &mut *self.shards.mine() };
        // SAFETY: the caller's guarantee, frame by frame.
        unsafe { carve(&window, shard, start, end) };
    }

    /// Take a block of `order`, or `None` when there is none to be had.
    ///
    /// The caller owns it until it is handed back. Its contents are whatever
    /// the last owner left — possibly plus a link word this allocator wrote,
    /// possibly untouched since the firmware — so a caller that needs to know
    /// what is in it wants [`Self::alloc_zeroed`]. This path exists for the
    /// caller that is about to overwrite the whole block anyway, and it
    /// prefers a dirty block precisely so that the clean lists are left for the
    /// callers that cannot.
    ///
    /// The dirty lists are preferred over the frontier for a second reason
    /// stated in RFC 0023: recycling keeps allocation on memory the machine —
    /// or the hypervisor underneath it — has already committed.
    pub fn alloc(&mut self, order: Order) -> Option<Frame> {
        let (block, _) = self.take(current_cpu(), order, false)?;
        Some(Frame { base: block, order })
    }

    /// Take a block containing nothing but zeroes.
    ///
    /// From the clean lists where possible, and by zeroing a dirty block where
    /// not — so the guarantee holds on a machine that has never scrubbed, and
    /// costs nothing on one that has.
    ///
    /// This is the path anything crossing an ownership boundary must use. A
    /// block that reaches a new owner still holding the previous owner's bytes
    /// is an information leak between components, and — because a simulated run
    /// and a hardware run will have left different bytes there — a divergence
    /// that gets diagnosed as a bug in whatever reads it next.
    pub fn alloc_zeroed(&mut self, order: Order) -> Option<Frame> {
        let (block, clean) = self.take(current_cpu(), order, true)?;
        let window = self.map;
        if clean {
            // The block is zero everywhere except the link word `list_push`
            // wrote into its first eight bytes, which is the one thing left to
            // undo. Undoing it is a plain zero rather than a masked link — a
            // masked zero is the cookie, which is exactly the byte pattern this
            // step exists to remove.
            // SAFETY: the block has just been taken off the list, so nothing
            // else holds it, and it is addressable.
            unsafe { window.clear_link(block) };
        } else {
            // SAFETY: the block is off the list and unowned, so its whole
            // extent is ours to write.
            unsafe { window.zero(block, order) };
        }
        Some(Frame { base: block, order })
    }

    /// Give a block back.
    ///
    /// It goes on the calling core's dirty list holding whatever the caller
    /// left in it. That is not a licence to leave a secret there —
    /// [`Self::scrub`] and [`Self::alloc_zeroed`] are what stop it reaching
    /// anybody else — but the cost of erasing it is paid where it can be
    /// scheduled rather than in the middle of whatever was using the block.
    ///
    /// The core it goes to is the core that freed it, never the core that
    /// allocated it. RFC 0027 argues that: sending it home would be cross-core
    /// traffic on a path as hot as allocation, and it buys only the chance
    /// that a buddy is on the same shard.
    ///
    /// # Safety
    ///
    /// `frame` must name memory this allocator handed out and nothing else may
    /// reference it afterwards. It need not be *the* block that was handed
    /// out: a block may be returned whole, or as any set of aligned sub-blocks
    /// that exactly tile it, which is a property of the buddy structure and is
    /// what the coalescing pass exists to put back together. Freeing anything
    /// twice puts it on a list twice, and the second allocation of it will be
    /// handed to somebody who already has it.
    pub unsafe fn free(&mut self, frame: Frame) {
        debug_assert!(
            frame.order().aligns(frame.addr()),
            "a block freed at an order it is not aligned to has no buddy"
        );
        let window = self.map;
        // SAFETY: the calling core's own shard; no interrupt handler in this
        // kernel reaches the frame allocator, so no second reference is live.
        let shard = unsafe { &mut *self.shards.mine() };
        // SAFETY: the caller has established that this block is theirs to give
        // back, which makes writing its first word sound for the same reason it
        // was sound when the block was handed out.
        unsafe { give(&window, shard, frame.order(), false, frame.addr()) };
    }

    /// Merge buddies on the calling core's shard, up to `budget` merges.
    ///
    /// Returns how many pairs were merged. Bounded rather than exhaustive
    /// because this is a consumer of memory bandwidth like any other: the
    /// caller that eventually drives it is a batch-class task under the
    /// resource discipline, and a task that cannot be asked to do a bounded
    /// amount of work cannot be scheduled against a deadline at all.
    pub fn coalesce(&mut self, budget: u64) -> u64 {
        let window = self.map;
        // SAFETY: the calling core's own shard, as `free`.
        let shard = unsafe { &mut *self.shards.mine() };
        // SAFETY: every block on this shard's lists is one this allocator put
        // there, addressable through the window.
        unsafe { coalesce_shard(&window, shard, budget) }
    }

    /// Move up to `budget` frames from the dirty lists to the clean ones,
    /// zeroing each.
    ///
    /// Returns how many frames were moved, which is fewer than asked for
    /// exactly when there was nothing left to take. Bounded for the reason
    /// [`Self::coalesce`] is.
    pub fn scrub(&mut self, budget: u64) -> u64 {
        let window = self.map;
        let frontier = &mut self.frontier;
        // SAFETY: the calling core's own shard, as `free`.
        let shard = unsafe { &mut *self.shards.mine() };

        let mut done = 0u64;
        while done < budget {
            let left = budget - done;
            // The largest order the remaining budget can pay for, then
            // downward until the shard actually has something: `pop_split`
            // fails at order k exactly when nothing of order k or above is
            // there, so walking down is what turns "no huge block" into "one
            // frame" rather than into a refill nobody asked for.
            let mut want = Order::FRAME;
            while let Some(up) = want.up() {
                if up.frames() <= left {
                    want = up;
                } else {
                    break;
                }
            }

            let mut taken = None;
            loop {
                // SAFETY: every block on the lists is addressable.
                if let Some(block) = unsafe { pop_split(&window, shard, want, false) } {
                    taken = Some((block, want));
                    break;
                }
                match want.down() {
                    Some(down) => want = down,
                    None => break,
                }
            }

            let Some((block, order)) = taken else {
                // Nothing on the dirty lists at all. The frontier is dirty in
                // every sense that matters here, and scrub is the right place
                // to touch it: touching memory nobody asked for is this
                // path's job, bandwidth-accounted, so faulting a page in here
                // is a scheduled cost rather than a boot-time one.
                // SAFETY: this core's shard and the machine-wide frontier.
                if unsafe { refill(&window, frontier, shard, Order::FRAME) } {
                    continue;
                }
                break;
            };

            // SAFETY: the block is off the list, so nothing else holds it.
            unsafe { window.zero(block, order) };
            // SAFETY: as above — unowned, addressable, and now zero
            // throughout, which is what the clean lists promise.
            unsafe { give(&window, shard, order, true, block) };
            done = done.saturating_add(order.frames());
        }
        done
    }

    /// The whole allocation path, for one core's shard.
    ///
    /// `cpu` rather than always this core because the self-test drives a shard
    /// that is not its own in order to reach the remote path on a machine that
    /// would never reach it by accident. Every other caller passes
    /// `current_cpu()`.
    fn take(&mut self, cpu: usize, order: Order, prefer_clean: bool) -> Option<(u64, bool)> {
        let window = self.map;
        let frontier = &mut self.frontier;
        let shards = &self.shards;
        // SAFETY: one core mutates this allocator at a time, and within a core
        // nothing in the interrupt path reaches it — so no second reference to
        // this slot is live. `PerCpu::at` leaves exactly this obligation to
        // the access, and the self-test is the documented reason it is `at`
        // and not `mine`.
        let shard = unsafe { &mut *shards.at(cpu) };

        // Round one: this core's own memory, then the frontier, in the order
        // RFC 0023 argued for — recycled before untouched, so a hypervisor
        // guest keeps allocating on pages it has already committed.
        // SAFETY: every block on the shard's lists is addressable, and the
        // frontier holds memory `add_region` accepted.
        let hit = unsafe { attempt(&window, frontier, shard, order, prefer_clean) };
        if let Some((block, source, clean)) = hit {
            record(shard, source, false);
            return Some((block, clean));
        }

        // Round two: compact. Paid by the allocation that would otherwise
        // fail, which is the only caller that can afford an unbounded pass —
        // `coalesce` is the bounded one, for the scheduler.
        // SAFETY: as above.
        unsafe { coalesce_shard(&window, shard, u64::MAX) };
        // SAFETY: as above.
        let hit = unsafe { attempt(&window, frontier, shard, order, prefer_clean) };
        if let Some((block, source, clean)) = hit {
            record(shard, source, false);
            return Some((block, clean));
        }

        // Round three: another core's memory. The only path in this module
        // that touches a shard it does not own, and the one the counters exist
        // to make visible.
        // SAFETY: `cpu` names this shard and `steal` skips it, so the two
        // `&mut Shard` it holds are different slots of the same `PerCpu`.
        if unsafe { steal(&window, shards, cpu, order, shard) } {
            // SAFETY: as above.
            let hit = unsafe { attempt(&window, frontier, shard, order, prefer_clean) };
            if let Some((block, source, clean)) = hit {
                record(shard, source, true);
                return Some((block, clean));
            }
        }
        None
    }

    /// Allocate on a named core's shard. For [`self_test`] only.
    fn alloc_on(&mut self, cpu: usize, order: Order) -> Option<Frame> {
        let (block, _) = self.take(cpu, order, false)?;
        Some(Frame { base: block, order })
    }

    /// Free onto a named core's shard. For [`self_test`] only.
    ///
    /// # Safety
    ///
    /// As [`Self::free`], and `cpu` must be a core this kernel shards for.
    unsafe fn free_on(&mut self, cpu: usize, frame: Frame) {
        let window = self.map;
        // SAFETY: one core mutates this allocator at a time, so no second
        // reference to any slot is live.
        let shard = unsafe { &mut *self.shards.at(cpu) };
        // SAFETY: the caller's guarantee, as `free`.
        unsafe { give(&window, shard, frame.order(), false, frame.addr()) };
    }

    /// Hide the frontier, and return what it takes to put it back.
    ///
    /// The self-test's way of reaching the remote path. A machine large enough
    /// to exhaust the frontier is not a machine the boot fixture can be, and a
    /// path that only a large machine reaches is a path that gets debugged on
    /// a large machine — which is the shape of the miscompile in
    /// `docs/first-boot-outside-qemu.md`. Only the count is moved; the entries
    /// above it are untouched, and nothing may accept a region in between.
    fn withhold_frontier(&mut self) -> (usize, u64) {
        let saved = (self.frontier.count, self.frontier.frames);
        self.frontier.count = 0;
        self.frontier.frames = 0;
        saved
    }

    /// Put back what [`Self::withhold_frontier`] took.
    fn restore_frontier(&mut self, saved: (usize, u64)) {
        self.frontier.count = saved.0;
        self.frontier.frames = saved.1;
    }
}

/// Which path answered an allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Source {
    /// This core's own lists. The hot path.
    Shard,
    /// The machine-wide frontier.
    Frontier,
}

/// Charge one allocation to the path that answered it.
fn record(shard: &mut Shard, source: Source, stolen: bool) {
    if stolen {
        shard.remote = shard.remote.saturating_add(1);
    } else {
        match source {
            Source::Shard => shard.served = shard.served.saturating_add(1),
            Source::Frontier => shard.refills = shard.refills.saturating_add(1),
        }
    }
}

/// One pass at answering an allocation from this core's memory.
///
/// Returns the block, which path answered, and whether it is clean.
///
/// The order of the three sources is the whole of the policy, and it differs
/// by what the caller asked for. A caller that wants zeroes takes clean memory
/// first, because the alternative is zeroing. A caller that does not takes
/// dirty memory first and untouched memory second, because a recycled block is
/// already committed by the hypervisor and a clean one is worth more to the
/// next caller who cannot zero.
///
/// # Safety
///
/// Every block on the shard's lists must be addressable through the window,
/// and the frontier must hold memory `add_region` accepted.
unsafe fn attempt(
    window: &Window,
    frontier: &mut Frontier,
    shard: &mut Shard,
    order: Order,
    prefer_clean: bool,
) -> Option<(u64, Source, bool)> {
    if prefer_clean {
        // SAFETY: the caller's guarantee.
        if let Some(block) = unsafe { pop_split(window, shard, order, true) } {
            return Some((block, Source::Shard, true));
        }
    }
    // SAFETY: as above.
    if let Some(block) = unsafe { pop_split(window, shard, order, false) } {
        return Some((block, Source::Shard, false));
    }
    // SAFETY: as above.
    if unsafe { refill(window, frontier, shard, order) } {
        // SAFETY: the refill put a block of at least `order` on the dirty
        // lists, so this cannot fail for want of memory.
        if let Some(block) = unsafe { pop_split(window, shard, order, false) } {
            return Some((block, Source::Frontier, false));
        }
    }
    if !prefer_clean {
        // SAFETY: as above.
        if let Some(block) = unsafe { pop_split(window, shard, order, true) } {
            return Some((block, Source::Shard, true));
        }
    }
    None
}

/// Move one block off the frontier and onto a shard.
///
/// The grain is [`Order::DEFAULT`] and falls back downward: a run too short or
/// too badly aligned for a huge block still has frames in it, and an allocator
/// that stranded them would be one whose free count stopped meaning anything
/// on a fragmented memory map.
///
/// # Safety
///
/// The frontier must hold memory `add_region` accepted, addressable through
/// the window.
unsafe fn refill(
    window: &Window,
    frontier: &mut Frontier,
    shard: &mut Shard,
    order: Order,
) -> bool {
    let mut want = if order > Order::DEFAULT { order } else { Order::DEFAULT };
    loop {
        if let Some((block, gap_start, gap_end)) = frontier.take(want) {
            // The frames the alignment stepped over have left the frontier's
            // accounting and are still memory. Carving them is what keeps the
            // free count exact across a refill.
            // SAFETY: the gap is inside a run `add_region` accepted.
            unsafe { carve(window, shard, gap_start, gap_end) };
            // SAFETY: as above, for the aligned block itself.
            unsafe { give(window, shard, want, false, block) };
            return true;
        }
        let Some(down) = want.down() else { return false };
        if down < order {
            return false;
        }
        want = down;
    }
}

/// Take a block of at least `order` from some other core's shard.
///
/// The last resort, and the only place in this module where a core reaches
/// memory another core owns. Smallest block first, ascending core order among
/// the cores that hold one, so that a steal takes as little as it can and
/// takes it from a predictable place: a stolen block is memory that stops
/// being able to merge with its buddy, and the cheapest steal is the one that
/// gives up the least.
///
/// The order is the outer loop and the core is the inner one, which is the
/// expensive way round — every order asked of every core is `MAX_CPUS` list
/// heads read rather than one — and it is the way round the sentence above
/// requires. Core-major would exhaust the first core's nineteen orders before
/// looking at the second, so it could take a gibibyte from one core while the
/// next held the single frame that was asked for. This path runs when the
/// frontier is spent and nowhere else; a hundred and fifty two reads of memory
/// nobody is contending for is not a cost worth trading the property away for.
///
/// # Safety
///
/// `me` must be the shard the caller already holds `mine` for — this function
/// skips it, which is what keeps the two `&mut Shard` disjoint — and one core
/// must be mutating this allocator at a time.
unsafe fn steal(
    window: &Window,
    shards: &PerCpu<Shard>,
    me: usize,
    order: Order,
    mine: &mut Shard,
) -> bool {
    let mut at = order;
    loop {
        for cpu in 0..MAX_CPUS {
            if cpu == me {
                continue;
            }
            // SAFETY: a slot the caller has guaranteed is not `mine`, on the
            // one core mutating this allocator.
            let victim = unsafe { &mut *shards.at(cpu) };
            for clean in [false, true] {
                // SAFETY: every block on that shard's lists is addressable.
                if let Some(block) = unsafe { list_pop(window, victim.lists(clean), at) } {
                    let frames = at.frames();
                    let count = victim.frames(clean);
                    *count = count.saturating_sub(frames);
                    // SAFETY: the block is off the other shard's list, so
                    // nothing holds it, and it is addressable.
                    unsafe { give(window, mine, at, clean, block) };
                    return true;
                }
            }
        }
        match at.up() {
            Some(up) => at = up,
            None => break,
        }
    }
    false
}

// --- the tests that run at boot --------------------------------------------

/// What [`self_test`] observed, for the boot log.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// The largest order this machine could actually serve.
    pub largest: u8,
    /// Blocks split into two over the whole test.
    pub splits: u64,
    /// Buddy pairs merged over the whole test.
    pub merges: u64,
    /// Cross-core allocations over everything the boot has allocated so far.
    ///
    /// The number the exit criterion is about. It is a running total taken
    /// *before* the last phase rather than a total at the end, because that
    /// last phase deliberately provokes the remote path — and a figure that
    /// mixed the two would say the boot took cross-core traffic without saying
    /// which of it was asked for. Everything it covers is the real allocation
    /// path: the page tables, the state tree, the ring, and four phases of
    /// adversary.
    pub hot_remote: u64,
    /// Allocations that reached the machine-wide frontier.
    pub refills: u64,
    /// Cross-core allocations the last phase deliberately provoked.
    pub steals: u64,
}

/// The M1 done-criterion, the adversary that replaced it, and the two orders
/// the design names.
///
/// > Allocate and map a thousand random frames, write and verify a pattern,
/// > unmap and free, ten times over — and the allocator's free count at the end
/// > is bit-identical to the start.
///
/// That sentence is phase one and is unchanged. What the buddy orders added is
/// four more phases, because a single-order allocator cannot leak a block in
/// the ways this one can: a split whose remainder is dropped, a merge that
/// counts one block twice, a refill whose alignment gap is stranded, and a
/// steal that debits nobody. Every one of those is a *leak of a fixed number
/// of frames per iteration*, which is why every phase ends by requiring the
/// free count to be exactly what it was.
///
/// Two departures from the original sentence survive, both deliberate:
///
/// **No mapping.** The boot stub identity-maps the first gigabyte and the
/// direct map covers the rest by the time this runs, so a block is already
/// addressable. Mapping and unmapping is exercised by `cargo xtask user`.
///
/// **Two words per block, not four thousand.** The property under test is that
/// blocks are distinct, addressable and never handed out twice — not that RAM
/// stores bytes. Writing a block's own identity into its first and last words
/// catches a duplicate handout, a partial overlap and an off-by-one at either
/// end, and at order 9 and above the *last* word is the one that catches a
/// split that returned a block shorter than it claimed.
///
/// Every choice the adversary makes comes from the `Env`, so the sequence is
/// reproducible and the same on every machine that runs the same seed.
///
/// # Errors
///
/// A sentence for the serial log, since at M1 there is nowhere else to report.
pub fn self_test(alloc: &mut FrameAllocator, env: &mut dyn Env) -> Result<Report, &'static str> {
    let start_free = alloc.free_count();
    let start_splits = alloc.split_count();
    let start_merges = alloc.merge_count();
    let start_refills = alloc.refill_count();

    round_trip(alloc, env, start_free)?;
    let salt = env.next_u64();
    adversary(alloc, env, salt)?;
    let largest = largest_order(alloc, salt)?;
    coalesces_back(alloc, env)?;

    let hot_remote = alloc.remote_count();
    let steals = provoke_remote(alloc)?;

    if alloc.free_count() != start_free {
        return Err("free count did not return to where it started");
    }

    Ok(Report {
        largest,
        splits: alloc.split_count().saturating_sub(start_splits),
        merges: alloc.merge_count().saturating_sub(start_merges),
        hot_remote,
        refills: alloc.refill_count().saturating_sub(start_refills),
        steals,
    })
}

/// Phase one: the M1 criterion, at one order, unchanged.
fn round_trip(
    alloc: &mut FrameAllocator,
    env: &mut dyn Env,
    start_free: u64,
) -> Result<(), &'static str> {
    const ROUNDS: usize = 10;
    const BATCH: usize = 1000;

    if start_free < BATCH as u64 {
        return Err("fewer free frames than the self-test needs");
    }

    let mut held = [0u64; BATCH];

    for _ in 0..ROUNDS {
        let salt = env.next_u64();

        for slot in &mut held {
            let frame = alloc.alloc(Order::FRAME).ok_or("allocator ran dry mid-round")?;
            *slot = frame.addr();
            let at = alloc.virt(frame);
            // SAFETY: the frame was just handed to this caller and nothing else
            // holds it, so its first and last words are ours to write.
            unsafe { stamp(at, Order::FRAME, frame.addr(), salt) };
        }

        for &addr in &held {
            let frame = Frame::from_addr(addr);
            let at = alloc.virt(frame);
            // SAFETY: this caller still owns every frame in `held` — none has
            // been freed yet in this round.
            if !unsafe { stamped(at, Order::FRAME, addr, salt) } {
                return Err("a frame did not read back what was written to it");
            }
        }

        // Free in an order the environment chooses, so the lists are exercised
        // rather than merely reversed. Fisher-Yates, backwards.
        for i in (1..BATCH).rev() {
            let j = env.scheduler().choose((i + 1) as u32) as usize;
            held.swap(i, j);
        }

        for &addr in &held {
            // SAFETY: each address came from `alloc` in this round, is being
            // returned exactly once, and nothing references it now.
            unsafe { alloc.free(Frame::from_addr(addr)) };
        }
    }

    if alloc.free_count() != start_free {
        return Err("free count did not return to where it started after one order");
    }
    Ok(())
}

/// Orders the adversary draws from.
///
/// Weighted towards the small end because that is what a kernel asks for, and
/// carrying [`Order::HUGE`] twice because a mix with no huge requests in it
/// never forces the splitter to give one back.
const MIX: [u8; 8] = [0, 0, 0, 1, 2, 4, 9, 9];

/// Phase two: an adversarial alloc/free sequence across several orders.
fn adversary(alloc: &mut FrameAllocator, env: &mut dyn Env, salt: u64) -> Result<(), &'static str> {
    const SLOTS: usize = 48;
    const ROUNDS: usize = 4;
    const STEPS: usize = 384;

    let before = alloc.free_count();
    // Address and order of what each slot holds; a zero address is an empty
    // slot, which is unambiguous because nothing below the low-memory limit is
    // ever handed out.
    let mut held = [(0u64, 0u8); SLOTS];

    for _ in 0..ROUNDS {
        for _ in 0..STEPS {
            let slot = env.scheduler().choose(SLOTS as u32) as usize;
            let Some(&(addr, order)) = held.get(slot) else { continue };

            if addr == 0 {
                let pick = env.scheduler().choose(MIX.len() as u32) as usize;
                let Some(&wanted) = MIX.get(pick) else { continue };
                let Some(order) = Order::new(wanted) else { continue };
                // A refusal is not a failure here: the adversary is allowed to
                // ask for more than the machine has, and what must not happen
                // is a *wrong* answer rather than a refusal.
                let Some(frame) = alloc.alloc(order) else { continue };
                if !order.aligns(frame.addr()) {
                    return Err("a block was handed out unaligned to its own order");
                }
                let at = alloc.virt(frame);
                // SAFETY: just handed over, nothing else holds it, so its
                // first and last words are ours.
                unsafe { stamp(at, order, frame.addr(), salt) };
                if let Some(entry) = held.get_mut(slot) {
                    *entry = (frame.addr(), wanted);
                }
            } else {
                let Some(order) = Order::new(order) else { continue };
                let frame = Frame::block(addr, order);
                let at = alloc.virt(frame);
                // SAFETY: this caller still owns the block.
                if !unsafe { stamped(at, order, addr, salt) } {
                    return Err("a block did not read back what was written to it");
                }
                // SAFETY: allocated above, returned exactly once, unreferenced.
                unsafe { alloc.free(frame) };
                if let Some(entry) = held.get_mut(slot) {
                    *entry = (0, 0);
                }
            }
        }
        alloc.coalesce(u64::MAX);
    }

    for &(addr, order) in &held {
        if addr == 0 {
            continue;
        }
        let Some(order) = Order::new(order) else { continue };
        // SAFETY: allocated above, returned exactly once, unreferenced.
        unsafe { alloc.free(Frame::block(addr, order)) };
    }
    alloc.coalesce(u64::MAX);

    if alloc.free_count() != before {
        return Err("the adversarial workload did not give every frame back");
    }
    Ok(())
}

/// Phase three: the largest order this machine can actually serve.
///
/// Upward from [`Order::HUGE`] and stopping at the first refusal, rather than
/// downward from [`Order::MAX`], because a refusal costs a full compaction
/// pass and probing downward would pay for nine of them on every machine
/// smaller than a gibibyte — which the boot fixture is.
///
/// A machine with less than a gibibyte cannot serve order 18 and the test does
/// not pretend otherwise: what it requires is order 9, and what it reports is
/// how far up this machine reached, so the boot log carries the answer rather
/// than a claim.
///
/// The other half of the exit criterion is order 18, which no boot on the
/// 128 MiB fixture can reach. `cargo xtask orders` boots this image on a
/// machine that has a gibibyte and requires this number to be 18 — the
/// reporting here is what that command reads, and the two together are why the
/// order-18 half is checked by something rather than reproduced by hand.
fn largest_order(alloc: &mut FrameAllocator, salt: u64) -> Result<u8, &'static str> {
    let before = alloc.free_count();
    let mut largest = 0u8;
    let mut want = Order::HUGE;

    while let Some(frame) = alloc.alloc(want) {
        if !want.aligns(frame.addr()) {
            return Err("a large block was handed out unaligned to its own order");
        }
        let at = alloc.virt(frame);
        // SAFETY: just handed over and nothing else holds it. The *last* word
        // is the one that matters here: a split that returned a block shorter
        // than it claimed writes outside somebody else's memory or reads back
        // wrong, and both are this stamp.
        unsafe { stamp(at, want, frame.addr(), salt) };
        // SAFETY: as above.
        if !unsafe { stamped(at, want, frame.addr(), salt) } {
            return Err("a large block did not read back what was written to it");
        }
        // SAFETY: allocated just above, returned exactly once.
        unsafe { alloc.free(frame) };
        largest = want.get();
        match want.up() {
            Some(up) => want = up,
            None => break,
        }
    }

    alloc.coalesce(u64::MAX);
    if alloc.free_count() != before {
        return Err("a large block was not fully given back");
    }
    if largest < Order::HUGE.get() {
        return Err("an order-9 allocation was refused on a machine with the memory for one");
    }
    Ok(largest)
}

/// Phase four: a huge block returned in 512 pieces becomes a huge block again.
///
/// The precise coalescing property, and the one a free count cannot see: an
/// allocator that lost the buddy relationship entirely would still return
/// every frame and still balance.
///
/// What it counts is *bytes held at or above* [`Order::HUGE`], not blocks on
/// the order-9 list, and the difference is not pedantry — it is a machine
/// size. On the 128 MiB fixture a shard is refilled with 2 MiB blocks, so the
/// re-formed block has no free buddy and stops at order 9. On a machine with
/// gibibytes the shard holds blocks above the default grain, the allocation
/// below split one of them, and the re-formed block immediately merges with
/// the sibling that split left behind — so the order-9 list is back where it
/// started and the memory is a 4 MiB block. Counting the order-9 list would
/// call that a failure, which is how `cargo xtask orders` found it: a
/// coalescing test that forbids coalescing.
///
/// The sum is exact rather than a bound, because merging never moves bytes
/// *out* of the range: whatever the pieces become, they are somewhere at or
/// above order 9, and nothing else on the shard has a new partner to merge
/// with — the phase begins with a full pass.
fn coalesces_back(alloc: &mut FrameAllocator, env: &mut dyn Env) -> Result<(), &'static str> {
    const PIECES: usize = 512;

    alloc.coalesce(u64::MAX);
    let before = alloc.free_count();

    let big = alloc.alloc(Order::HUGE).ok_or("no 2 MiB block for the coalescing check")?;
    let base = big.addr();
    let huge_before = held_at_or_above(alloc, Order::HUGE);
    let frames_before = alloc.free_blocks(Order::FRAME);

    let mut order = [0u16; PIECES];
    for (index, slot) in order.iter_mut().enumerate() {
        *slot = index as u16;
    }
    for i in (1..PIECES).rev() {
        let j = env.scheduler().choose((i + 1) as u32) as usize;
        order.swap(i, j);
    }

    for &piece in &order {
        let addr = base + u64::from(piece) * FRAME_SIZE;
        // SAFETY: `base` names a 2 MiB block this caller owns; each piece is
        // an aligned sub-block of it and is returned exactly once, which is
        // the shape `free` documents as legal.
        unsafe { alloc.free(Frame::from_addr(addr)) };
    }

    if alloc.free_blocks(Order::FRAME) < frames_before + PIECES as u64 {
        return Err("frames returned in pieces did not reach the order-0 list");
    }

    let merged = alloc.coalesce(u64::MAX);
    if merged == 0 {
        return Err("nothing merged after a block was returned in pieces");
    }
    if alloc.free_blocks(Order::FRAME) > frames_before {
        return Err("frames returned in pieces did not all merge upward");
    }
    if held_at_or_above(alloc, Order::HUGE) != huge_before + Order::HUGE.bytes() {
        return Err("512 frames returned in pieces did not become a 2 MiB block again");
    }
    if alloc.free_count() != before {
        return Err("returning a block in pieces changed the free count");
    }
    Ok(())
}

/// How many bytes this core's shard holds in blocks of at least `order`.
///
/// A walk of every list from `order` upward, for the same reason
/// [`FrameAllocator::free_blocks`] is a walk: the allocator keeps no number
/// like this for its own sake, and one kept only so a test could read it would
/// be a number nothing maintains under pressure.
fn held_at_or_above(alloc: &FrameAllocator, order: Order) -> u64 {
    let mut bytes = 0u64;
    let mut at = order;
    loop {
        bytes = bytes.saturating_add(alloc.free_blocks(at).saturating_mul(at.bytes()));
        match at.up() {
            Some(up) => at = up,
            None => return bytes,
        }
    }
}

/// Phase five: reach the one path that crosses a core boundary, and require it
/// to say so.
///
/// A counter that cannot move is indistinguishable from a counter that works,
/// which is the defect `Tree::self_test` exists to catch one layer up. The
/// remote path is reached on a real machine only when the frontier is spent,
/// and the boot fixture has 128 MiB and never spends it — so the frontier is
/// withheld, and a shard with nothing is asked for a frame.
fn provoke_remote(alloc: &mut FrameAllocator) -> Result<u64, &'static str> {
    const TAKE: usize = 4;
    /// A core that is not the boot core. `MAX_CPUS` is eight and this is a
    /// shard, not a running core, so nothing has to be started for it to be
    /// reachable — which is the point of preparing shards before cores.
    const OTHER: usize = 1;

    if MAX_CPUS < 2 {
        return Ok(0);
    }

    let before_free = alloc.free_count();
    let before_remote = alloc.remote_count();
    let saved = alloc.withhold_frontier();

    let mut taken = [0u64; TAKE];
    for slot in &mut taken {
        let frame = alloc
            .alloc_on(OTHER, Order::FRAME)
            .ok_or("a shard with nothing could not reach another core's")?;
        *slot = frame.addr();
    }

    let steals = alloc.remote_count().saturating_sub(before_remote);
    for &addr in &taken {
        // SAFETY: each came from `alloc_on` just above, is returned exactly
        // once, and nothing references it.
        unsafe { alloc.free_on(0, Frame::from_addr(addr)) };
    }
    alloc.restore_frontier(saved);

    if steals == 0 {
        return Err("an allocation crossed a core boundary and did not count it");
    }
    if alloc.free_count() != before_free {
        return Err("a cross-core allocation lost frames");
    }
    Ok(steals)
}

/// Nothing a block's last owner wrote survives into the next owner's hands.
///
/// Both paths are exercised at both grains, because they are different code
/// and only one of them is ever tested by accident: `alloc_zeroed` when the
/// clean lists are empty and it must zero the block itself, and `alloc_zeroed`
/// after [`FrameAllocator::scrub`] when the block is already zero and the only
/// thing left in it is the allocator's own link word — the one byte range a
/// zeroing path is most likely to forget, because the allocator put it there
/// itself.
///
/// The huge-page half is the one the buddy orders added, and it is where the
/// clean invariant can fail without the order-0 half noticing: a merged clean
/// block carries its upper half's link word in the middle of itself, which is
/// a byte range no frame-sized check ever reads.
///
/// The whole block is checked rather than a sample of it. A partial check here
/// would be testing the test rather than the property.
///
/// # Errors
///
/// A sentence for the serial log.
pub fn hygiene_test(alloc: &mut FrameAllocator) -> Result<(), &'static str> {
    const BATCH: usize = 8;
    /// Not zero, not a plausible pointer, and not symmetric under the byte
    /// order — so a partial erase reads differently from both a clean block and
    /// an untouched one.
    const PATTERN: u8 = 0xA5;

    let start_free = alloc.free_count();
    if start_free < BATCH as u64 {
        return Err("fewer free frames than the hygiene test needs");
    }

    let mut held = [0u64; BATCH];

    // Take frames, fill them completely, and give them back. This is the
    // previous owner.
    for slot in &mut held {
        let frame = alloc.alloc(Order::FRAME).ok_or("allocator ran dry")?;
        *slot = frame.addr();
        let at = alloc.virt(frame);
        // SAFETY: the frame was just handed over and nothing else holds it, so
        // its whole extent is ours to write.
        unsafe { core::ptr::write_bytes(at, PATTERN, FRAME_SIZE as usize) };
    }
    for &addr in &held {
        // SAFETY: each address came from `alloc` above and is returned once.
        unsafe { alloc.free(Frame::from_addr(addr)) };
    }

    // The next owner, with nothing scrubbed: `alloc_zeroed` has to do the
    // erasing itself.
    for slot in &mut held {
        let frame = alloc.alloc_zeroed(Order::FRAME).ok_or("allocator ran dry")?;
        *slot = frame.addr();
        // SAFETY: the frame was just handed to this caller, so reading every
        // byte of it is reading memory nothing else owns.
        if !unsafe { all_zero(alloc.virt(frame), Order::FRAME) } {
            return Err("alloc_zeroed handed back a frame that was not zero");
        }
    }
    for &addr in &held {
        // SAFETY: as above.
        unsafe { alloc.free(Frame::from_addr(addr)) };
    }

    // And again through the scrubbed path, where the block is already zero and
    // the link word is the only thing that has been written since.
    let scrubbed = alloc.scrub(BATCH as u64);
    if scrubbed != BATCH as u64 {
        return Err("scrub did not clean the frames it was given a budget for");
    }
    if alloc.clean_count() < BATCH as u64 {
        return Err("scrubbed frames did not reach the clean list");
    }

    for slot in &mut held {
        let frame = alloc.alloc_zeroed(Order::FRAME).ok_or("allocator ran dry")?;
        *slot = frame.addr();
        // SAFETY: as above.
        if !unsafe { all_zero(alloc.virt(frame), Order::FRAME) } {
            return Err("a scrubbed frame still held the allocator's link word");
        }
    }
    for &addr in &held {
        // SAFETY: as above.
        unsafe { alloc.free(Frame::from_addr(addr)) };
    }

    // The huge grain, both ways. A block zeroed inline, and a block assembled
    // by the coalescing pass out of clean halves — the second being where an
    // interior link word would survive.
    let big = alloc.alloc(Order::HUGE).ok_or("no 2 MiB block for the hygiene test")?;
    let at = alloc.virt(big);
    // SAFETY: just handed over, nothing else holds it.
    unsafe { core::ptr::write_bytes(at, PATTERN, Order::HUGE.bytes() as usize) };
    // SAFETY: allocated above, returned exactly once.
    unsafe { alloc.free(big) };

    let zeroed = alloc.alloc_zeroed(Order::HUGE).ok_or("no 2 MiB block to zero")?;
    // SAFETY: just handed over, so every byte of it is ours to read.
    if !unsafe { all_zero(alloc.virt(zeroed), Order::HUGE) } {
        return Err("alloc_zeroed handed back a 2 MiB block that was not zero");
    }
    // Give it back as two halves, scrub them both clean, and let the pass put
    // them together: the merged block's middle word is the allocator's own
    // link, and it must not survive into the next owner.
    let half = Order::HUGE.down().ok_or("order 9 has no order below it")?;
    // SAFETY: `zeroed` is a 2 MiB block this caller owns; each half is an
    // aligned sub-block of it and is returned exactly once.
    unsafe { alloc.free(Frame::block(zeroed.addr(), half)) };
    // SAFETY: as above, for the upper half.
    unsafe { alloc.free(Frame::block(zeroed.addr() + half.bytes(), half)) };
    // A budget of exactly one half, twice. `scrub` takes the largest dirty
    // block the budget can pay for, so a budget of a whole huge page would
    // let it clean some *other* block and leave these two dirty — and the
    // property under test is what happens when the pass merges two clean
    // halves, not what happens when it merges two dirty ones.
    alloc.scrub(half.frames());
    alloc.scrub(half.frames());
    alloc.coalesce(u64::MAX);

    let merged = alloc.alloc_zeroed(Order::HUGE).ok_or("no 2 MiB block after scrubbing")?;
    // SAFETY: just handed over, so every byte of it is ours to read.
    if !unsafe { all_zero(alloc.virt(merged), Order::HUGE) } {
        return Err("a merged clean block still held the allocator's link word inside it");
    }
    // SAFETY: allocated above, returned exactly once.
    unsafe { alloc.free(merged) };

    alloc.coalesce(u64::MAX);
    if alloc.free_count() != start_free {
        return Err("free count did not return to where it started");
    }

    // An order past the largest this system will ever name is not an order.
    // The M1 floor refused every order above zero here; what is left to check
    // is the bound the *type* keeps, which is the one a caller can still get
    // wrong.
    if Order::new(Order::MAX + 1).is_some() {
        return Err("an order above the largest this system names was accepted");
    }

    Ok(())
}

/// Is every byte of this block zero?
///
/// Word at a time rather than byte at a time: two mebibytes of volatile byte
/// reads is two million emulated loads, and the property is about the bytes
/// either way.
///
/// # Safety
///
/// The caller must own the block, and `at` must be where it is mapped.
unsafe fn all_zero(at: *mut u8, order: Order) -> bool {
    let words = order.bytes() / 8;
    let base = at.cast::<u64>();
    let mut index = 0u64;
    while index < words {
        let cell = base.wrapping_add(index as usize);
        // SAFETY: the caller owns the block and `at` is where it is mapped, so
        // every word inside it is readable.
        if unsafe { cell.read_volatile() } != 0 {
            return false;
        }
        index += 1;
    }
    true
}

/// Write a block's own identity into its first and last words.
///
/// # Safety
///
/// The caller must own the block, and `at` must be where it is mapped.
unsafe fn stamp(at: *mut u8, order: Order, phys: u64, salt: u64) {
    let (first, last) = word_pair(at, order);
    let value = phys ^ salt ^ u64::from(order.get());
    // SAFETY: the caller owns the block, so this address is writable and
    // aliased by nothing.
    unsafe { first.write_volatile(value) };
    // SAFETY: as above; the last word is inside the same block.
    unsafe { last.write_volatile(!value) };
}

/// Does a block still hold what [`stamp`] wrote?
///
/// # Safety
///
/// The caller must own the block, and `at` must be where it is mapped.
unsafe fn stamped(at: *mut u8, order: Order, phys: u64, salt: u64) -> bool {
    let (first, last) = word_pair(at, order);
    let value = phys ^ salt ^ u64::from(order.get());
    // SAFETY: the caller owns the block, so this address is readable.
    let head = unsafe { first.read_volatile() };
    // SAFETY: as above.
    let tail = unsafe { last.read_volatile() };
    head == value && tail == !value
}

/// The first and last word of a block, given where it is mapped.
fn word_pair(at: *mut u8, order: Order) -> (*mut u64, *mut u64) {
    let first = at.cast::<u64>();
    let offset = usize::try_from(order.bytes()).unwrap_or(usize::MAX).saturating_sub(8);
    let last = at.wrapping_add(offset).cast::<u64>();
    (first, last)
}
