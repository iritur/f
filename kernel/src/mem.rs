// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Physical memory.
//!
//! # The allocator, and why it has no metadata
//!
//! A free frame is storage nobody is using, so the free list lives *in* the free
//! frames: the first word of each one holds the address of the next. Allocation
//! is a load, a store and a decrement; freeing is the same in reverse. There is
//! no bitmap to size, no array to place, and no bootstrap problem where the
//! allocator needs an allocator.
//!
//! That matters more here than it would elsewhere. Phase 00 has no general heap
//! by design — a bump allocator and a slab, nothing more — so a structure whose
//! size depends on how much memory the machine turned out to have would have to
//! be either statically sized for a machine nobody has, or allocated from
//! something that does not exist yet.
//!
//! The cost is stated rather than hidden: a free frame must be *addressable*,
//! because the list is written into it. See [`IDENTITY_LIMIT`].
//!
//! # The link is not a plain pointer
//!
//! Storing the list inside the frames means the link word lives in memory that
//! has recently belonged to somebody else, and will belong to somebody else
//! again. A corrupted link is not a crash — it is a frame handed out at an
//! address the corruptor chose. So the stored word is the next address
//! exclusive-or'd with a per-boot value drawn from the [`Env`], which turns a
//! useful overwrite into an unpredictable one. The value comes from the
//! environment rather than from hardware so that a replayed run corrupts and
//! recovers identically; a defence that made the system irreproducible would
//! cost more than it bought.
//!
//! # Clean and dirty
//!
//! There are two lists, not one. A freed frame goes on the *dirty* list holding
//! whatever its last owner left. [`FrameAllocator::scrub`] moves frames from
//! there to the *clean* list, zeroing each as it goes, and
//! [`FrameAllocator::alloc_zeroed`] takes from the clean list when it can and
//! zeroes inline when it cannot.
//!
//! Scrubbing exists as a separate step because zeroing is memory bandwidth, and
//! memory bandwidth is a resource this system means to account for rather than
//! spend silently — the design has it as a batch-class consumer that runs when
//! nothing with a deadline wants the controller. There is no such scheduler at
//! M1, so nothing calls `scrub` in anger yet; what matters now is that the
//! interface is the one the scheduler will use, and that no frame reaches a new
//! owner still carrying the old owner's bytes.
//!
//! # What this is not
//!
//! Not the design in `docs/design/deadline-all-the-way-down.html` section 03,
//! which calls for a buddy allocator with per-CPU free lists and huge pages by
//! default. This is the M1 floor: one list pair, one core, one frame size.
//! [`Order`] exists ahead of the allocator that can honour it, so that the
//! callers written between now and then do not all have to be revisited when it
//! lands. Per-CPU sharding is `E0-B05` and lands before the second core exists,
//! because retrofitting it is the miserable refactor the boot document warns
//! about.

use f_env::Env;

/// Bytes per frame. One 4 KiB page, which is the only size M1 hands out.
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
/// Order 0 is one frame, order 9 is a 2 MiB page, order 18 is a gibibyte. The
/// allocator behind it can only satisfy order 0 today and says so by refusing
/// the rest; the type exists now because it appears in every signature that
/// touches a frame, and adding a parameter to those signatures later means
/// editing every call site under the time pressure of a half-finished buddy
/// allocator.
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

    /// How many bytes this order covers.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        FRAME_SIZE << self.0
    }
}

/// A run of physical frames, and how long it is.
///
/// A physical address with a name and a length, so that it cannot be passed
/// where a virtual address is meant and cannot be freed at the wrong size. At
/// M1 physical and virtual happen to be related by one addition and every run
/// is one frame long; both facts are behind accessors so that they can stop
/// being true without an audit.
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
    /// the page tables, which are frames but are never on the free list.
    #[must_use]
    pub const fn from_addr(addr: u64) -> Self {
        Self { base: addr, order: Order::FRAME }
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

/// Free lists of physical frames: one holding zeroed frames, one holding frames
/// as their last owner left them.
///
/// The lists hold *physical* addresses, which is why `phys_offset` exists: the
/// same lists are walked before and after the kernel takes over the page
/// tables, and only the way a frame is reached changes. Under the boot stub's
/// identity window the offset is zero; under the direct map it is
/// [`crate::arch::x86_64::paging::PHYS_OFFSET`]. Nothing stored has to be
/// rewritten at the switch, which is the property that makes the switch a
/// two-field assignment rather than a migration.
#[derive(Debug)]
pub struct FrameAllocator {
    /// Head of the zeroed list, or zero for none. Zero is unambiguous because
    /// nothing below [`LOW_MEMORY_LIMIT`] is ever on either list.
    clean: u64,
    /// Head of the list of frames holding whatever was last written to them.
    dirty: u64,
    clean_free: u64,
    dirty_free: u64,
    total: u64,
    unreachable: u64,
    /// Added to a physical address to reach it.
    phys_offset: u64,
    /// One past the highest physical address currently reachable.
    limit: u64,
    /// Mixed into every stored link. See the module documentation.
    cookie: u64,
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
            clean: 0,
            dirty: 0,
            clean_free: 0,
            dirty_free: 0,
            total: 0,
            unreachable: 0,
            phys_offset: 0,
            limit: IDENTITY_LIMIT,
            cookie,
        }
    }

    /// Where a frame can be read and written.
    ///
    /// The one place physical becomes virtual. Everything else in the kernel
    /// that needs to touch a frame asks here rather than assuming the two are
    /// equal — an assumption that was true until E0-B04 and is now false.
    #[must_use]
    pub fn virt(&self, frame: Frame) -> *mut u8 {
        (frame.addr() + self.phys_offset) as *mut u8
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
        self.phys_offset = phys_offset;
        self.limit = limit;
        self.unreachable = 0;
    }

    /// Frames available now, zeroed or not.
    #[must_use]
    pub const fn free_count(&self) -> u64 {
        self.clean_free + self.dirty_free
    }

    /// Frames available and known to be zero.
    #[must_use]
    pub const fn clean_count(&self) -> u64 {
        self.clean_free
    }

    /// Frames available that still hold what their last owner wrote.
    #[must_use]
    pub const fn dirty_count(&self) -> u64 {
        self.dirty_free
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

    /// Add a region of usable memory, minus anything reserved.
    ///
    /// Filtering is per frame rather than by interval arithmetic. A machine has
    /// tens of thousands of frames and a handful of reserved ranges, so the loop
    /// costs nothing at boot and cannot get the subtraction wrong — which
    /// interval arithmetic, done once, at four in the morning, can.
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
        let mut frame = base.next_multiple_of(FRAME_SIZE);

        while frame.saturating_add(FRAME_SIZE) <= region_end {
            let end = frame + FRAME_SIZE;
            let usable =
                frame >= LOW_MEMORY_LIMIT && !reserved.iter().any(|r| r.overlaps(frame, end));

            if usable {
                if end > self.limit {
                    self.unreachable += 1;
                } else {
                    // SAFETY: the frame is inside a region the caller vouched
                    // for, overlaps nothing reserved, and lies below the
                    // addressable limit, so writing its first word is writing
                    // to memory nothing else owns.
                    unsafe { self.push_dirty(frame) };
                    self.total += 1;
                }
            }

            frame = end;
        }
    }

    /// Take a run of frames, or `None` when there is none to be had.
    ///
    /// The caller owns it until it is handed back. Its contents are whatever the
    /// last owner left, plus a link word this allocator wrote — a caller that
    /// needs to know what is in it wants [`Self::alloc_zeroed`]. This path
    /// exists for the caller that is about to overwrite the whole frame anyway,
    /// and it prefers a dirty frame precisely so that the clean list is left for
    /// the callers that cannot.
    ///
    /// Any order but [`Order::FRAME`] is refused. That is the M1 floor rather
    /// than a policy: splitting and coalescing is the buddy allocator, and a
    /// refusal is a better answer than a silently smaller allocation.
    pub fn alloc(&mut self, order: Order) -> Option<Frame> {
        if order != Order::FRAME {
            return None;
        }
        let frame = if self.dirty != 0 {
            // SAFETY: the list is non-empty and holds frames this allocator put
            // there, within the window `phys_offset` names.
            unsafe { self.pop_dirty() }?
        } else {
            // SAFETY: as above, for the other list.
            unsafe { self.pop_clean() }?
        };
        Some(Frame { base: frame, order })
    }

    /// Take a run of frames containing nothing but zeroes.
    ///
    /// From the clean list where possible, and by zeroing a dirty frame where
    /// not — so the guarantee holds on a machine that has never scrubbed, and
    /// costs nothing on one that has.
    ///
    /// This is the path anything crossing an ownership boundary must use. A
    /// frame that reaches a new owner still holding the previous owner's bytes
    /// is an information leak between components, and — because a simulated run
    /// and a hardware run will have left different bytes there — a divergence
    /// that gets diagnosed as a bug in whatever reads it next.
    pub fn alloc_zeroed(&mut self, order: Order) -> Option<Frame> {
        if order != Order::FRAME {
            return None;
        }

        if self.clean != 0 {
            // SAFETY: the clean list is non-empty and holds frames this
            // allocator put there.
            let frame = unsafe { self.pop_clean() }?;
            // The frame is zero everywhere except the link word `push_clean`
            // wrote into its first eight bytes, which is the one thing left to
            // undo. Undoing it is a plain zero rather than a masked link — a
            // masked zero is the cookie, which is exactly the byte pattern this
            // step exists to remove.
            // SAFETY: the frame has just been taken off the list, so nothing
            // else holds it, and it is addressable.
            unsafe { self.clear_link(frame) };
            return Some(Frame { base: frame, order });
        }

        // SAFETY: as above, for the dirty list.
        let frame = unsafe { self.pop_dirty() }?;
        // SAFETY: the frame is off the list and unowned, so its whole extent is
        // ours to write.
        unsafe { self.zero(frame) };
        Some(Frame { base: frame, order })
    }

    /// Give a run of frames back.
    ///
    /// It goes on the dirty list holding whatever the caller left in it. That
    /// is not a licence to leave a secret there — [`Self::scrub`] and
    /// [`Self::alloc_zeroed`] are what stop it reaching anybody else — but the
    /// cost of erasing it is paid where it can be scheduled rather than in the
    /// middle of whatever was using the frame.
    ///
    /// # Safety
    ///
    /// `frame` must have come from [`Self::alloc`] or [`Self::alloc_zeroed`] on
    /// this allocator, and nothing may reference it afterwards. Freeing a frame
    /// twice puts it on the list twice, and the second allocation of it will be
    /// handed to somebody who already has it.
    pub unsafe fn free(&mut self, frame: Frame) {
        debug_assert!(frame.order() == Order::FRAME, "no order but 0 is handed out at M1");
        // SAFETY: the caller has established that this frame is theirs to give
        // back, which makes writing its first word sound for the same reason it
        // was sound when the frame was added.
        unsafe { self.push_dirty(frame.addr()) };
    }

    /// Move up to `budget` frames from the dirty list to the clean one, zeroing
    /// each.
    ///
    /// Returns how many were moved, which is fewer than asked for exactly when
    /// the dirty list ran out. Bounded rather than exhaustive because this is a
    /// consumer of memory bandwidth like any other: the caller that eventually
    /// drives it is a batch-class task under the resource discipline, and a
    /// task that cannot be asked to do a bounded amount of work cannot be
    /// scheduled against a deadline at all.
    pub fn scrub(&mut self, budget: u64) -> u64 {
        let mut done = 0;
        while done < budget && self.dirty != 0 {
            // SAFETY: the dirty list is non-empty and holds frames this
            // allocator put there.
            let Some(frame) = (unsafe { self.pop_dirty() }) else { break };
            // SAFETY: the frame is off the list, so nothing else holds it.
            unsafe { self.zero(frame) };
            // SAFETY: as above — the frame is unowned and addressable, and its
            // first word becomes the link again.
            unsafe { self.push_clean(frame) };
            done += 1;
        }
        done
    }

    // --- the lists ---------------------------------------------------------

    /// Put a frame on the dirty list.
    ///
    /// # Safety
    ///
    /// `frame` must be a 4 KiB-aligned, addressable frame that nothing else
    /// owns.
    unsafe fn push_dirty(&mut self, frame: u64) {
        // SAFETY: the caller has established that this frame is unowned and
        // addressable, so its first word is ours to use as the list link.
        unsafe { self.write_link(frame, self.dirty) };
        self.dirty = frame;
        self.dirty_free += 1;
    }

    /// Put a zeroed frame on the clean list.
    ///
    /// # Safety
    ///
    /// As [`Self::push_dirty`], and the frame must actually be zero.
    unsafe fn push_clean(&mut self, frame: u64) {
        // SAFETY: as `push_dirty` — the caller owns the frame and it is
        // addressable.
        unsafe { self.write_link(frame, self.clean) };
        self.clean = frame;
        self.clean_free += 1;
    }

    /// Take the head of the dirty list.
    ///
    /// # Safety
    ///
    /// Every frame on the list must be addressable through [`Self::virt`],
    /// which is the invariant [`Self::rebind`] carries.
    unsafe fn pop_dirty(&mut self) -> Option<u64> {
        let frame = self.dirty;
        if frame == 0 {
            return None;
        }
        // SAFETY: `dirty` is a frame this allocator put on its own list, so its
        // first word is the link `push_dirty` wrote there.
        self.dirty = unsafe { self.read_link(frame) };
        self.dirty_free -= 1;
        Some(frame)
    }

    /// Take the head of the clean list.
    ///
    /// # Safety
    ///
    /// As [`Self::pop_dirty`].
    unsafe fn pop_clean(&mut self) -> Option<u64> {
        let frame = self.clean;
        if frame == 0 {
            return None;
        }
        // SAFETY: as `pop_dirty`, for the other list.
        self.clean = unsafe { self.read_link(frame) };
        self.clean_free -= 1;
        Some(frame)
    }

    /// Write the link word of a free frame, masked.
    ///
    /// # Safety
    ///
    /// `frame` must be addressable and owned by nobody but this allocator.
    unsafe fn write_link(&self, frame: u64, next: u64) {
        let at = self.virt(Frame::from_addr(frame)).cast::<u64>();
        // SAFETY: the caller has established that the frame is unowned and
        // addressable, so its first word is ours.
        unsafe { at.write_volatile(next ^ self.cookie) };
    }

    /// Write a literal zero over the link word.
    ///
    /// Not `write_link(frame, 0)`: that stores the cookie, which is a perfectly
    /// good link and a perfectly bad first word for a frame that is about to be
    /// handed over as empty.
    ///
    /// # Safety
    ///
    /// As [`Self::write_link`].
    unsafe fn clear_link(&self, frame: u64) {
        let at = self.virt(Frame::from_addr(frame)).cast::<u64>();
        // SAFETY: the caller has established that the frame is unowned and
        // addressable, so its first word is ours.
        unsafe { at.write_volatile(0) };
    }

    /// Read the link word of a free frame, unmasked.
    ///
    /// # Safety
    ///
    /// As [`Self::write_link`].
    unsafe fn read_link(&self, frame: u64) -> u64 {
        let at = self.virt(Frame::from_addr(frame)).cast::<u64>();
        // SAFETY: as above; this word was written by `write_link`.
        let stored = unsafe { at.read_volatile() };
        stored ^ self.cookie
    }

    /// Write zeroes over a whole frame.
    ///
    /// # Safety
    ///
    /// `frame` must be addressable and owned by nobody else.
    unsafe fn zero(&self, frame: u64) {
        let at = self.virt(Frame::from_addr(frame));
        // SAFETY: the caller owns the frame, so its whole extent is ours to
        // write, and `virt` is where it is mapped.
        unsafe { core::ptr::write_bytes(at, 0, FRAME_SIZE as usize) };
    }
}

/// The M1 done-criterion, as code.
///
/// > Allocate and map a thousand random frames, write and verify a pattern,
/// > unmap and free, ten times over — and the allocator's free count at the end
/// > is bit-identical to the start.
///
/// Two departures from that sentence, both deliberate and both stated here
/// rather than discovered later:
///
/// **No mapping.** The boot stub identity-maps the first gigabyte, so a frame
/// below that line is already addressable. Mapping and unmapping is `E0-B04`,
/// which is when the frame takes ownership of the page tables, and this test
/// grows an `unmap` step then.
///
/// **Two words per frame, not four thousand.** The property under test is that
/// frames are distinct, addressable and never handed out twice — not that RAM
/// stores bytes. Writing the frame's own address into its first and last words
/// catches a duplicate handout, a partial overlap and an off-by-one at either
/// end, while a full-frame memset would spend forty mebibytes of emulated
/// writes per run to catch nothing extra.
///
/// The order frames are freed in comes from the `Env`, so it is adversarial,
/// reproducible, and the same on every machine that runs the same seed.
///
/// # Errors
///
/// A sentence for the serial log, since at M1 there is nowhere else to report.
pub fn self_test(alloc: &mut FrameAllocator, env: &mut dyn Env) -> Result<(), &'static str> {
    const ROUNDS: usize = 10;
    const BATCH: usize = 1000;

    let start_free = alloc.free_count();
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
            unsafe { stamp(at, frame.addr(), salt) };
        }

        for &addr in &held {
            let at = alloc.virt(Frame::from_addr(addr));
            // SAFETY: this caller still owns every frame in `held` — none has
            // been freed yet in this round.
            if !unsafe { stamped(at, addr, salt) } {
                return Err("a frame did not read back what was written to it");
            }
        }

        // Free in an order the environment chooses, so the list is exercised
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
        return Err("free count did not return to where it started");
    }

    Ok(())
}

/// Nothing a frame's last owner wrote survives into the next owner's hands.
///
/// Both paths are exercised, because they are different code and only one of
/// them is ever tested by accident: `alloc_zeroed` when the clean list is empty
/// and it must zero the frame itself, and `alloc_zeroed` after
/// [`FrameAllocator::scrub`] when the frame is already zero and the only thing
/// left in it is the allocator's own link word — the one byte-range a zeroing
/// path is most likely to forget, because the allocator put it there itself.
///
/// The whole frame is checked rather than a sample of it. Eight frames is
/// thirty-two kibibytes of reads, which is nothing, and a partial check here
/// would be testing the test rather than the property.
///
/// # Errors
///
/// A sentence for the serial log.
pub fn hygiene_test(alloc: &mut FrameAllocator) -> Result<(), &'static str> {
    const BATCH: usize = 8;
    /// Not zero, not a plausible pointer, and not symmetric under the byte
    /// order — so a partial erase reads differently from both a clean frame and
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
        if !unsafe { all_zero(alloc.virt(frame)) } {
            return Err("alloc_zeroed handed back a frame that was not zero");
        }
    }
    for &addr in &held {
        // SAFETY: as above.
        unsafe { alloc.free(Frame::from_addr(addr)) };
    }

    // And again through the scrubbed path, where the frame is already zero and
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
        if !unsafe { all_zero(alloc.virt(frame)) } {
            return Err("a scrubbed frame still held the allocator's link word");
        }
    }
    for &addr in &held {
        // SAFETY: as above.
        unsafe { alloc.free(Frame::from_addr(addr)) };
    }

    if alloc.free_count() != start_free {
        return Err("free count did not return to where it started");
    }

    // Orders the buddy allocator will satisfy and this one cannot must be
    // refused rather than quietly served at the wrong size.
    if alloc.alloc(Order::HUGE).is_some() || alloc.alloc_zeroed(Order::HUGE).is_some() {
        return Err("an order this allocator cannot satisfy was not refused");
    }

    Ok(())
}

/// Is every byte of this frame zero?
///
/// # Safety
///
/// The caller must own the frame, and `at` must be where it is mapped.
unsafe fn all_zero(at: *mut u8) -> bool {
    for byte in 0..FRAME_SIZE as usize {
        let cell = at.wrapping_add(byte);
        // SAFETY: the caller owns the frame and `at` is where it is mapped, so
        // every byte inside it is readable.
        if unsafe { cell.read_volatile() } != 0 {
            return false;
        }
    }
    true
}

/// Write a frame's own identity into its first and last words.
///
/// # Safety
///
/// The caller must own the frame, and `at` must be where it is mapped.
unsafe fn stamp(at: *mut u8, phys: u64, salt: u64) {
    let (first, last) = word_pair(at);
    let value = phys ^ salt;
    // SAFETY: the caller owns the frame, which is below the identity map, so
    // this address is writable and aliased by nothing.
    unsafe { first.write_volatile(value) };
    // SAFETY: as above; the last word is inside the same frame.
    unsafe { last.write_volatile(!value) };
}

/// Does a frame still hold what [`stamp`] wrote?
///
/// # Safety
///
/// The caller must own the frame, and `at` must be where it is mapped.
unsafe fn stamped(at: *mut u8, phys: u64, salt: u64) -> bool {
    let (first, last) = word_pair(at);
    let value = phys ^ salt;
    // SAFETY: the caller owns the frame, so this address is readable.
    let head = unsafe { first.read_volatile() };
    // SAFETY: as above.
    let tail = unsafe { last.read_volatile() };
    head == value && tail == !value
}

/// The first and last word of a frame, given where it is mapped.
fn word_pair(at: *mut u8) -> (*mut u64, *mut u64) {
    let first = at.cast::<u64>();
    let last = at.wrapping_add(FRAME_SIZE as usize - 8).cast::<u64>();
    (first, last)
}
