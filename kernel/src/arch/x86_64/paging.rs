// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The kernel's own address space.
//!
//! # What this replaces
//!
//! The boot stub builds three static tables and maps the first gigabyte twice,
//! which is the least it can do and still reach long mode. That arrangement has
//! properties the kernel cannot keep: it is bounded at a gigabyte, the tables
//! live inside the kernel image rather than in memory the system allocated, and
//! every page of it is readable, writable and executable at once.
//!
//! # Two windows, with different jobs and different permissions
//!
//! **The direct map**, at [`PHYS_OFFSET`]. Every byte of usable physical memory
//! appears at `PHYS_OFFSET + physical`, so turning a frame number into something
//! the kernel can read is an addition rather than a mapping operation. It is
//! writable and never executable: it is where data lives, and nothing should
//! ever jump into it.
//!
//! **The kernel window**, at -2 GiB, where the image is linked. It maps the
//! image and nothing else — not the boot stub, not the rest of physical memory —
//! at four-kibibyte granularity, so that each section gets the permissions its
//! contents deserve.
//!
//! # Write exclusive-or execute
//!
//! Text is executable and not writable. Constants are neither. Data, stacks and
//! the tables themselves are writable and not executable. Nothing is both.
//!
//! This is cheap now and expensive later, which is the whole reason it is here
//! rather than on the list. Every milestone that ships without it adds code that
//! may quietly depend on being able to write its own instructions or execute its
//! own data, and the dependency is invisible until the day the permission
//! changes.
//!
//! Three processor features carry it, and [`enable_features`] turns on the two
//! that need turning on:
//!
//! - **No-execute**, which is a *reserved* bit until enabled — so a mapping
//!   built for it before enabling it faults on use rather than protecting
//!   anything.
//! - **Write-protect**, without which ring 0 may write a read-only page anyway,
//!   making half the rule a comment.
//! - **Global pages**, which is not a protection but belongs with them: the
//!   kernel window is in every address space that will ever exist, so its
//!   entries need not be flushed when `CR3` changes. Free here, and worth
//!   having in place before there is a second address space to switch to.
//!
//! **The device window**, at [`DEVICE_OFFSET`]. Memory-mapped registers, one
//! page at a time, uncacheable. It is a separate window rather than a hole in
//! the direct map because the direct map is built out of gibibyte and
//! mebibyte pages: on a machine with enough memory, the page holding a device
//! register would fall inside one of them, and asking for a four-kibibyte
//! mapping there is a collision rather than a mapping. A window of its own
//! cannot collide with anything, on any machine.
//!
//! # A process's address space
//!
//! [`UserSpace`] is the second kind. It is a top table of its own whose upper
//! half is *copied* from the kernel's, entry for entry, and whose lower half is
//! built one page at a time with [`USER`] set the whole way down. Copying the
//! upper half rather than sharing a pointer to it is what makes a system call
//! and an interrupt from ring 3 land on mappings that are already there: no
//! switch of the kernel's own view, and the timer keeps ticking across the
//! transition because its register window is in the half that was copied.
//!
//! The copy is a snapshot, and that is the one thing about it worth watching.
//! Every top-level slot the kernel will ever have is created before the first
//! process exists — the direct map, the device window and the kernel window are
//! one slot each — so a snapshot is currently exact. *Reversal:* the day the
//! kernel maps something into a top-level slot that did not exist at process
//! creation, every process built before that is missing it, and the symptom is
//! a kernel address that works in the kernel and faults inside a process.
//! Sharing has to become structural then: pre-allocate all 256 upper tables at
//! boot so that every root points at the same ones.
//!
//! # What this is still not
//!
//! No lazy mapping, no shootdown, and no unmapping — a process's pages are
//! given back by freeing its frames after its address space stops being the
//! one in `CR3`, which is sound for one process on one core and is not a
//! general answer. Those arrive with M4, when a page belongs to a capability
//! and a second core has to be told it has gone.

use super::cpuid;
use crate::mem::{Frame, FrameAllocator, Order};

/// Where physical memory appears in the kernel's address space.
///
/// The bottom of the higher half, which leaves the entire lower half for user
/// space and puts the direct map a long way from the kernel image — so a stray
/// physical address used as a virtual one lands in unmapped space rather than in
/// the middle of the kernel.
pub const PHYS_OFFSET: u64 = 0xFFFF_8000_0000_0000;

/// Where memory-mapped device registers appear.
///
/// A window of its own, one PML4 slot wide — five hundred and twelve gibibytes,
/// which is every device address any machine this kernel runs on will have —
/// and a long way from both the direct map and the kernel image. A device
/// register at physical `p` is read and written at `DEVICE_OFFSET + p`.
///
/// Deliberately *not* an offset into the direct map. See the module comment:
/// the direct map is built from huge pages, and a four-kibibyte mapping inside
/// one of them is an error rather than a refinement.
pub const DEVICE_OFFSET: u64 = 0xFFFF_9000_0000_0000;

/// How far past [`DEVICE_OFFSET`] a device may sit: one top-level slot.
const DEVICE_WINDOW: u64 = 512 * GIB;

/// Bytes in a page.
const PAGE: u64 = 4096;

/// Bytes covered by one entry of a page directory.
const HUGE_PAGE: u64 = 2 * 1024 * 1024;

/// Bytes covered by one entry of a page-directory-pointer table.
const GIB: u64 = 1024 * 1024 * 1024;

/// Entries in every level of the table hierarchy.
const ENTRIES: usize = 512;

/// The bits of an entry that hold a physical address.
const ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// The entry is valid.
const PRESENT: u64 = 1 << 0;

/// Writes are permitted.
const WRITE: u64 = 1 << 1;

/// Ring 3 may reach this page.
///
/// The whole of the isolation, and it has to be set at *every* level: the
/// processor takes the logical and of the bit down the walk, so a leaf that
/// grants it under a table that does not is a page ring 3 cannot see. That
/// asymmetry is deliberate and is what makes the kernel's copied upper half
/// safe — nothing up there has this bit at any level.
const USER: u64 = 1 << 2;

/// Writes go straight through rather than into a cache.
const WRITE_THROUGH: u64 = 1 << 3;

/// The line is not cached at all.
///
/// With [`WRITE_THROUGH`] and the page-attribute-table bit clear, this selects
/// the uncacheable type under the default attribute table — which is what a
/// device register needs, and needs for correctness rather than for speed: a
/// cached read of a status register returns whatever it said the first time.
const CACHE_DISABLE: u64 = 1 << 4;

/// This entry is the page itself rather than a pointer to a finer table.
const PAGE_SIZE_BIT: u64 = 1 << 7;

/// The translation survives a `CR3` change. Only meaningful once page-global
/// enable is set, and only correct for a mapping that is identical in every
/// address space — which is exactly what a kernel mapping is.
const GLOBAL: u64 = 1 << 8;

/// Instruction fetches are refused. Legal only once no-execute is enabled;
/// before that the processor treats it as a reserved bit and faults on use.
const NO_EXECUTE: u64 = 1 << 63;

/// Flags for an entry pointing at another table.
///
/// Permissive on purpose: the restrictive bits live in the leaf, and a
/// no-execute bit here would apply to everything below it — which for the entry
/// covering the kernel would include the kernel's own text.
const TABLE: u64 = PRESENT | WRITE;

/// Flags for an entry pointing at another table in a process's lower half.
///
/// [`TABLE`] plus [`USER`], for the reason [`USER`] gives: the bit is anded
/// down the walk, so leaving it off here would make every leaf below
/// unreachable from ring 3 while looking, in the leaf, exactly right.
const USER_TABLE: u64 = TABLE | USER;

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __rwdata_start: u8;
    static __rwdata_end: u8;
    static __kernel_vma: u8;
    static __fault_stack_guard: u8;
    static __kernel_stack_guard: u8;
}

/// What the processor supports, and what was turned on because of it.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    /// A table entry may refuse instruction fetches. Enabled.
    pub nx: bool,
    /// A kernel mapping may survive a `CR3` change. Enabled.
    pub global: bool,
    /// The processor can tag translations with an address-space identifier.
    /// Detected and deliberately **not** enabled: there is nothing to switch
    /// between until user address spaces exist, and a feature enabled before it
    /// is needed is a feature nobody notices is wrong.
    pub pcid: bool,
    /// A page-directory-pointer entry may be a gibibyte-sized page. Not a
    /// protection — a saving, and one the direct map takes.
    pub gigabyte_pages: bool,
}

/// Ask the processor what it offers, then turn on what the mappings need.
///
/// Returns what is actually in force, not what was asked for: on a machine
/// without no-execute the kernel still runs, with one protection fewer and a
/// log line saying so. Silently mapping without it would be worse — the tables
/// would look right and enforce nothing.
///
/// # Safety
///
/// Call once, on the boot processor, before any address space carrying these
/// bits is activated. The write-protect bit takes effect immediately and makes
/// read-only pages read-only for the kernel too, which would be a surprise to
/// code that had been writing one.
pub unsafe fn enable_features() -> Features {
    // SAFETY: `cpuid` has no memory effect and no privilege requirement.
    let (_, ecx1, edx1) = unsafe { cpuid(1) };
    // SAFETY: as above. A processor without the extended leaves predates long
    // mode and so cannot be running this.
    let (_, _, edx_ext) = unsafe { cpuid(0x8000_0001) };

    let features = Features {
        nx: edx_ext & (1 << 20) != 0,
        global: edx1 & (1 << 13) != 0,
        pcid: ecx1 & (1 << 17) != 0,
        gigabyte_pages: edx_ext & (1 << 26) != 0,
    };

    if features.nx {
        // EFER.NXE, in the extended feature register.
        // SAFETY: read-modify-write of one defined bit at ring 0, leaving every
        // other bit — including long-mode enable — as it was.
        unsafe {
            core::arch::asm!(
                "rdmsr",
                "or eax, {bit}",
                "wrmsr",
                bit = const 1u32 << 11,
                in("ecx") 0xC000_0080u32,
                lateout("eax") _,
                lateout("edx") _,
                options(nostack, preserves_flags),
            );
        }
    }

    if features.global {
        // CR4.PGE.
        // SAFETY: setting one defined bit of a control register at ring 0.
        unsafe {
            core::arch::asm!(
                "mov {tmp}, cr4",
                "or {tmp}, {bit}",
                "mov cr4, {tmp}",
                tmp = lateout(reg) _,
                bit = const 1u64 << 7,
                options(nostack, preserves_flags),
            );
        }
    }

    // CR0.WP. Without it a read-only page is read-only for user space and
    // advisory for the kernel, which makes write-exclusive-or-execute a
    // statement of intent rather than a rule. Unconditional: every processor
    // that can run this has it.
    // SAFETY: setting one defined bit of a control register at ring 0.
    unsafe {
        core::arch::asm!(
            "mov {tmp}, cr0",
            "or {tmp}, {bit}",
            "mov cr0, {tmp}",
            tmp = lateout(reg) _,
            bit = const 1u64 << 16,
            options(nostack, preserves_flags),
        );
    }

    features
}

/// A kernel address space, named by the physical address of its top table.
#[derive(Clone, Copy, Debug)]
pub struct AddressSpace {
    root: u64,
    direct_limit: u64,
}

impl AddressSpace {
    /// The physical address of the top-level table, which is what `CR3` holds.
    #[must_use]
    pub const fn root(&self) -> u64 {
        self.root
    }

    /// One past the highest physical address the direct map reaches.
    ///
    /// Reported rather than assumed, because the allocator has to be told where
    /// its window ends and the honest answer is whatever this actually mapped —
    /// rounded up to the grain it mapped with, which is more than was asked for
    /// and never less.
    #[must_use]
    pub const fn direct_limit(&self) -> u64 {
        self.direct_limit
    }
}

/// Why an address space could not be built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildError {
    /// The allocator ran out of frames. Every table is one frame, so this means
    /// a machine with almost no memory at all.
    NoFrames,
    /// More physical memory than one page-directory-pointer table can cover:
    /// five hundred and twelve gibibytes. Not a limit worth designing around
    /// until there is a machine that reaches it, and not one to discover by
    /// mapping the wrong thing.
    TooMuchMemory,
    /// A mapping was asked for where a larger page already sits. A bug in this
    /// file rather than a condition to handle, and reported rather than
    /// silently unmapping what was there.
    Overlap,
    /// A device was found at a physical address the device window does not
    /// reach. Five hundred and twelve gibibytes up, which no PC-class machine
    /// puts a register at — so this is a misread base address rather than an
    /// unusual machine, and mapping it anyway would put a device on top of
    /// whatever is in the next slot.
    DeviceOutOfWindow,
    /// A process's address space needed more tables than [`MAX_USER_TABLES`].
    /// Reported rather than silently dropped, because a table that is not on
    /// the list is a frame that is never given back — a leak that grows by one
    /// process and is invisible in every count except the free one.
    TooManyTables,
    /// A process was asked for a page in the kernel's half of the address
    /// space. Refused here rather than at the leaf, because at the leaf it
    /// would be a mapping with [`USER`] set on a kernel table — which is to say
    /// a hole in the isolation, made by arithmetic rather than by intent.
    NotUserAddress,
}

impl BuildError {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NoFrames => "not enough frames to build an address space",
            Self::TooMuchMemory => "more physical memory than the direct map covers",
            Self::Overlap => "a mapping collided with a larger one",
            Self::DeviceOutOfWindow => "a device sits beyond the device window",
            Self::TooManyTables => "a process's address space needs more tables than are tracked",
            Self::NotUserAddress => "a process was offered a page in the kernel's half",
        }
    }
}

/// Build the kernel's address space.
///
/// `highest_phys` is one past the last byte of usable physical memory. The
/// direct map covers `0..highest_phys` rounded up to the grain it maps with;
/// memory-mapped devices are deliberately outside it, to be mapped where they
/// are found at M2, when there is a device to find.
///
/// # Safety
///
/// The caller must still be running under a mapping in which a physical address
/// is directly addressable, because the tables are written through
/// `frames.virt()` before the space they describe exists. In practice: call this
/// while the boot stub's identity window is live, and call [`activate`]
/// immediately afterwards.
///
/// # Errors
///
/// [`BuildError`], which is fatal at M1: there is no address space to fall back
/// to and nowhere to report but the serial port.
pub unsafe fn build(
    frames: &mut FrameAllocator,
    highest_phys: u64,
    features: Features,
) -> Result<AddressSpace, BuildError> {
    let nx = if features.nx { NO_EXECUTE } else { 0 };
    let global = if features.global { GLOBAL } else { 0 };

    // SAFETY: the caller guarantees a mapping in which frames are addressable.
    let root = unsafe { table(frames) }?;

    // --- the direct map: every usable byte of physical memory, once ---------
    //
    // Writable, never executable, and global. One entry per gibibyte where the
    // processor has pages that large: fewer entries to walk, and far fewer
    // translation-buffer entries for the window every frame access goes through.
    let grain = if features.gigabyte_pages { GIB } else { HUGE_PAGE };
    let direct_limit = highest_phys.next_multiple_of(grain).max(grain);
    if direct_limit.div_ceil(GIB) > ENTRIES as u64 {
        return Err(BuildError::TooMuchMemory);
    }

    let data = PRESENT | WRITE | nx | global;
    let mut at = 0u64;
    while at < direct_limit {
        if features.gigabyte_pages {
            // SAFETY: as above.
            unsafe { map_gib(frames, root, PHYS_OFFSET + at, at, data)? };
        } else {
            // SAFETY: as above.
            unsafe { map_huge(frames, root, PHYS_OFFSET + at, at, data)? };
        }
        at += grain;
    }

    // --- the kernel window: the image, at the permissions it deserves -------
    //
    // Four-kibibyte pages, because the sections are four-kibibyte aligned and a
    // section sharing a page with another would get the union of their
    // permissions — which is the permission neither of them wanted.
    let vma = (&raw const __kernel_vma) as u64;
    let ranges = [
        // Executable, not writable.
        ((&raw const __text_start) as u64, (&raw const __text_end) as u64, PRESENT | global),
        // Neither.
        (
            (&raw const __rodata_start) as u64,
            (&raw const __rodata_end) as u64,
            PRESENT | nx | global,
        ),
        // Writable, not executable: data, the global offset table, `.bss` and
        // the stacks.
        (
            (&raw const __rwdata_start) as u64,
            (&raw const __rwdata_end) as u64,
            PRESENT | WRITE | nx | global,
        ),
    ];

    // Pages deliberately left out, each one below a stack. The stack above
    // grows down into a fault instead of into whatever was underneath it.
    let guards =
        [(&raw const __fault_stack_guard) as u64, (&raw const __kernel_stack_guard) as u64];

    for (start, end, flags) in ranges {
        let mut virt = start;
        while virt < end {
            if !guards.contains(&virt) {
                // SAFETY: as above. The physical address is the virtual one less
                // the offset the linker script placed the image at.
                unsafe { map_page(frames, root, virt, virt - vma, flags)? };
            }
            virt += PAGE;
        }
    }

    Ok(AddressSpace { root, direct_limit })
}

/// Switch to an address space.
///
/// # Safety
///
/// Every address the caller is currently using — the instruction pointer, the
/// stack, and anything it touches before it re-establishes its bearings — must
/// be mapped in `space`. The kernel window exists precisely so that this is true
/// of the code performing the switch: it does not move.
///
/// The previous tables become unreferenced immediately. They are the boot stub's
/// own `.bss` rather than allocator frames, so nothing leaks and nothing needs
/// freeing; they are part of the image that is never read again, and after this
/// no longer even mapped.
pub unsafe fn activate(space: &AddressSpace) {
    // SAFETY: the caller has established that the code and stack in use are
    // mapped in the new space. Writing CR3 flushes every non-global entry —
    // and nothing the boot stub mapped was global, so nothing of it survives.
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) space.root, options(nostack, preserves_flags));
    }
}

/// Map one page of device registers into a live address space.
///
/// Returns the address the registers can be reached at, which is
/// [`DEVICE_OFFSET`] plus the physical address — offset within the page
/// included, so a base that is not page-aligned still returns the right place
/// to read.
///
/// # Why this one runs after the switch and [`build`] runs before it
///
/// [`build`] writes tables describing an address space that is not active yet,
/// so nothing it writes can be stale. This writes into the space the caller is
/// running in. Two consequences, and both are handled here rather than left to
/// the caller: the page must be invalidated afterwards, because a not-present
/// entry may already be cached negatively; and every table this touches is
/// reached through the direct map, so the allocator must already have been
/// rebound onto it.
///
/// # Errors
///
/// [`BuildError::DeviceOutOfWindow`] if the device is past the window,
/// [`BuildError::NoFrames`] if a table cannot be allocated, and
/// [`BuildError::Overlap`] never — nothing else maps into this window, which is
/// the reason it is a window of its own.
///
/// # Safety
///
/// `space` must be the address space currently in `CR3`, `frames` must have
/// been rebound onto its direct map, and `phys` must name device registers
/// rather than ordinary memory: this mapping is uncacheable and writable, and
/// pointing it at memory somebody else owns aliases that memory with different
/// caching, which is a corruption the hardware will not report.
pub unsafe fn map_device(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    phys: u64,
    features: Features,
) -> Result<u64, BuildError> {
    if phys >= DEVICE_WINDOW {
        return Err(BuildError::DeviceOutOfWindow);
    }

    let page = phys & !(PAGE - 1);
    let virt = DEVICE_OFFSET + page;

    let nx = if features.nx { NO_EXECUTE } else { 0 };
    let global = if features.global { GLOBAL } else { 0 };
    let flags = PRESENT | WRITE | CACHE_DISABLE | WRITE_THROUGH | nx | global;

    // SAFETY: the caller has guaranteed the allocator is rebound onto the
    // direct map of the active space, which is what makes every table this
    // walks readable and writable.
    unsafe { map_page(frames, space.root, virt, page, flags) }?;

    // The entry was not present a moment ago, and a not-present translation may
    // have been cached as such. One instruction, and skipping it is the kind of
    // bug that reproduces on one machine in ten.
    // SAFETY: invalidating a page is architecturally valid at ring 0 for any
    // address, mapped or not.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags));
    }

    Ok(DEVICE_OFFSET + phys)
}

/// How many tables one process's address space may need.
///
/// A process at M3 has one text page, one guard and one stack page inside a
/// single two-mebibyte region, which is four tables: the top one, and one at
/// each level below it. Eight is that with room for a second region, and it is
/// a bound rather than a limit worth designing around — E0-B11 gives a process
/// an `AddressSpace` capability with a quota behind it, and this array is what
/// that replaces.
pub const MAX_USER_TABLES: usize = 8;

/// What a page in a process's half is for.
///
/// The two permissions a process can be given, named rather than composed by
/// the caller. Flag arithmetic stays in this file: a caller that builds its own
/// leaf is a caller that can forget [`USER`] — or remember it in the leaf and
/// forget it in the tables, which fails in the direction that looks like a bug
/// in the process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UserPage {
    /// Executable and not writable. A process's text, and the only thing it
    /// may execute.
    Text,
    /// Writable and never executable. A process's stack, and later its data.
    Data,
}

impl UserPage {
    /// The leaf entry's flags, address excluded.
    ///
    /// Never [`GLOBAL`]. A global entry survives the `CR3` write that leaves
    /// this address space, which for a page belonging to one process is
    /// precisely the mapping that must not survive it.
    const fn flags(self, features: Features) -> u64 {
        let nx = if features.nx { NO_EXECUTE } else { 0 };
        match self {
            Self::Text => PRESENT | USER,
            Self::Data => PRESENT | USER | WRITE | nx,
        }
    }
}

/// One process's address space, and the frames it is made of.
///
/// The frame list is the whole of the ownership model at M3, and it is honest
/// about being small: a process's tables are given back by freeing exactly the
/// frames that were allocated for them, so the list has to be complete or the
/// free count does not come back. E0-B11 replaces it with a capability
/// derivation tree, where the same question is asked of every object rather
/// than of page tables alone.
pub struct UserSpace {
    root: u64,
    tables: [Frame; MAX_USER_TABLES],
    count: usize,
    shared: usize,
}

impl UserSpace {
    /// The physical address of the top-level table, which is what `CR3` holds.
    #[must_use]
    pub const fn root(&self) -> u64 {
        self.root
    }

    /// How many of the kernel's top-level slots this space carries a copy of.
    ///
    /// Reported so the boot log can say it. A number that changes when the
    /// kernel gains a window is the earliest visible sign of the snapshot
    /// problem the module comment names.
    #[must_use]
    pub const fn shared_slots(&self) -> usize {
        self.shared
    }

    /// Every frame this address space is built from.
    ///
    /// Its pages are not here: those are the caller's, because the caller
    /// allocated them and knows what is in them.
    #[must_use]
    pub fn tables(&self) -> &[Frame] {
        &self.tables[..self.count]
    }

    /// Note a frame as one this space will have to give back.
    fn record(&mut self, frame: Frame) -> Result<(), BuildError> {
        if self.count == MAX_USER_TABLES {
            return Err(BuildError::TooManyTables);
        }
        self.tables[self.count] = frame;
        self.count += 1;
        Ok(())
    }
}

/// Build an address space for a process: the kernel's upper half, and nothing
/// else yet.
///
/// The lower half is empty. Every page in it arrives through [`map_user`], one
/// at a time, which is the only way a process gets anything at all.
///
/// # Errors
///
/// [`BuildError::NoFrames`] if the top table cannot be allocated.
///
/// # Safety
///
/// `kernel` must be the address space currently in `CR3` and `frames` must be
/// rebound onto its direct map, because both tables are read and written
/// through it.
pub unsafe fn user_space(
    frames: &mut FrameAllocator,
    kernel: &AddressSpace,
) -> Result<UserSpace, BuildError> {
    // SAFETY: the caller's guarantee that frames are addressable.
    let root = unsafe { table(frames) }?;
    let mut space =
        UserSpace { root, tables: [Frame::from_addr(0); MAX_USER_TABLES], count: 0, shared: 0 };
    space.record(Frame::from_addr(root))?;

    // The upper half, entry for entry. Not a copy of half a page: only the
    // present entries are taken, so the count is the number of kernel windows
    // rather than the number of slots that exist.
    for slot in ENTRIES / 2..ENTRIES {
        // SAFETY: `kernel.root` is a table this module built and `slot` is in
        // range by construction.
        let entry = unsafe { read(frames, kernel.root, slot) };
        if entry & PRESENT == 0 {
            continue;
        }
        // SAFETY: as above, into the table allocated a few lines up, which
        // nothing else has seen.
        unsafe { write(frames, root, slot, entry) };
        space.shared += 1;
    }

    Ok(space)
}

/// Map one page into a process's half of its address space.
///
/// # Errors
///
/// [`BuildError::NotUserAddress`] if `virt` is not in the lower half,
/// [`BuildError::NoFrames`] or [`BuildError::TooManyTables`] if a table cannot
/// be made or recorded, and [`BuildError::Overlap`] if something is already
/// there.
///
/// # Safety
///
/// As [`user_space`], and `space` must not be the address space currently in
/// `CR3`: this writes entries that were not present, and a live space would
/// need each of them invalidated.
pub unsafe fn map_user(
    frames: &mut FrameAllocator,
    space: &mut UserSpace,
    virt: u64,
    phys: u64,
    kind: UserPage,
    features: Features,
) -> Result<(), BuildError> {
    // The lower half, and canonical. Everything at or above this is the
    // kernel's, and the hole in between is not an address at all.
    if virt >= 1 << 47 {
        return Err(BuildError::NotUserAddress);
    }

    let root = space.root;
    // SAFETY: the caller's guarantee, passed down.
    let pdpt = unsafe { descend_user(frames, space, root, slot_of(virt, 39)) }?;
    // SAFETY: as above.
    let pd = unsafe { descend_user(frames, space, pdpt, slot_of(virt, 30)) }?;
    // SAFETY: as above.
    let pt = unsafe { descend_user(frames, space, pd, slot_of(virt, 21)) }?;

    // SAFETY: as above.
    let existing = unsafe { read(frames, pt, slot_of(virt, 12)) };
    if existing & PRESENT != 0 {
        return Err(BuildError::Overlap);
    }
    // SAFETY: as above.
    unsafe { write(frames, pt, slot_of(virt, 12), phys | kind.flags(features)) };
    Ok(())
}

/// Follow an entry in a process's half, creating a table if there is none.
///
/// Deliberately not [`descend`], and the difference is two things that both
/// have to be true at once: the entry it writes carries [`USER`], because the
/// processor ands that bit down the walk and a leaf cannot grant what a parent
/// withheld; and the frame it allocates is recorded, because a process's tables
/// are given back when it dies and one that was never written down is one that
/// never comes back.
///
/// # Safety
///
/// As [`build`].
unsafe fn descend_user(
    frames: &mut FrameAllocator,
    space: &mut UserSpace,
    parent: u64,
    slot: usize,
) -> Result<u64, BuildError> {
    // SAFETY: the caller's guarantee; `parent` is a table this module made.
    let existing = unsafe { read(frames, parent, slot) };
    if existing & PRESENT != 0 {
        if existing & PAGE_SIZE_BIT != 0 {
            return Err(BuildError::Overlap);
        }
        return Ok(existing & ADDRESS_MASK);
    }

    // SAFETY: as above.
    let fresh = unsafe { table(frames) }?;
    space.record(Frame::from_addr(fresh))?;
    // SAFETY: as above.
    unsafe { write(frames, parent, slot, fresh | USER_TABLE) };
    Ok(fresh)
}

/// Switch to a process's address space, or back out of one.
///
/// # Safety
///
/// `root` must be a top-level table this module built whose upper half is the
/// kernel's, because the code performing the switch and the stack under it are
/// both up there. Everything [`activate`] says applies here too; this is the
/// same instruction with a weaker argument type, because a process's space is
/// not an [`AddressSpace`] and the switch back to the kernel's is.
pub unsafe fn switch(root: u64) {
    // SAFETY: the caller has established that the kernel window and the direct
    // map are in `root`, which is what makes the instruction after this one
    // fetchable. Non-global entries are flushed, which for a process's lower
    // half is the point.
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
    }
}

/// Map one four-kibibyte page.
///
/// # Safety
///
/// As [`build`]: frames must be addressable through `frames.virt()`.
unsafe fn map_page(
    frames: &mut FrameAllocator,
    root: u64,
    virt: u64,
    phys: u64,
    flags: u64,
) -> Result<(), BuildError> {
    // SAFETY: the caller's guarantee, passed down.
    let pdpt = unsafe { descend(frames, root, slot_of(virt, 39)) }?;
    // SAFETY: as above.
    let pd = unsafe { descend(frames, pdpt, slot_of(virt, 30)) }?;
    // SAFETY: as above.
    let pt = unsafe { descend(frames, pd, slot_of(virt, 21)) }?;
    // SAFETY: as above.
    unsafe { write(frames, pt, slot_of(virt, 12), phys | flags) };
    Ok(())
}

/// Map one two-mebibyte page.
///
/// # Safety
///
/// As [`build`].
unsafe fn map_huge(
    frames: &mut FrameAllocator,
    root: u64,
    virt: u64,
    phys: u64,
    flags: u64,
) -> Result<(), BuildError> {
    // SAFETY: the caller's guarantee, passed down.
    let pdpt = unsafe { descend(frames, root, slot_of(virt, 39)) }?;
    // SAFETY: as above.
    let pd = unsafe { descend(frames, pdpt, slot_of(virt, 30)) }?;
    // SAFETY: as above.
    unsafe { write(frames, pd, slot_of(virt, 21), phys | flags | PAGE_SIZE_BIT) };
    Ok(())
}

/// Map one gibibyte page.
///
/// # Safety
///
/// As [`build`], and the processor must have agreed it supports pages this size.
unsafe fn map_gib(
    frames: &mut FrameAllocator,
    root: u64,
    virt: u64,
    phys: u64,
    flags: u64,
) -> Result<(), BuildError> {
    // SAFETY: the caller's guarantee, passed down.
    let pdpt = unsafe { descend(frames, root, slot_of(virt, 39)) }?;
    // SAFETY: as above.
    unsafe { write(frames, pdpt, slot_of(virt, 30), phys | flags | PAGE_SIZE_BIT) };
    Ok(())
}

/// The index into the table at a given level.
const fn slot_of(virt: u64, shift: u32) -> usize {
    ((virt >> shift) & 0x1FF) as usize
}

/// Follow an entry to the next table, creating one if there is none.
///
/// # Safety
///
/// As [`build`].
unsafe fn descend(
    frames: &mut FrameAllocator,
    parent: u64,
    slot: usize,
) -> Result<u64, BuildError> {
    // SAFETY: the caller's guarantee; `parent` is a table this module made.
    let existing = unsafe { read(frames, parent, slot) };

    if existing & PRESENT != 0 {
        // A larger page already covers this address. There is no finer table to
        // descend into, and creating one would silently unmap what is there.
        if existing & PAGE_SIZE_BIT != 0 {
            return Err(BuildError::Overlap);
        }
        return Ok(existing & ADDRESS_MASK);
    }

    // SAFETY: as above.
    let fresh = unsafe { table(frames) }?;
    // SAFETY: as above.
    unsafe { write(frames, parent, slot, fresh | TABLE) };
    Ok(fresh)
}

/// A frame to use as a table: allocated, and guaranteed to be zero.
///
/// [`FrameAllocator::alloc_zeroed`] rather than `alloc`, and the distinction is
/// not hygiene here but correctness: an entry made of whatever the last owner
/// left is a mapping to an arbitrary physical address with arbitrary
/// permissions, and it would be *present* about half the time.
///
/// # Safety
///
/// As [`build`].
unsafe fn table(frames: &mut FrameAllocator) -> Result<u64, BuildError> {
    let frame = frames.alloc_zeroed(Order::FRAME).ok_or(BuildError::NoFrames)?;
    Ok(frame.addr())
}

/// Read one entry of a table.
///
/// # Safety
///
/// `table` must be a frame this kernel owns, addressable through
/// `frames.virt()`, and `slot` must be below [`ENTRIES`].
unsafe fn read(frames: &FrameAllocator, table: u64, slot: usize) -> u64 {
    let at = frames.virt(Frame::from_addr(table)).cast::<u64>().wrapping_add(slot);
    // SAFETY: the caller owns the table and the slot is in range, so this
    // address is inside one frame that nothing else is using.
    unsafe { at.read_volatile() }
}

/// Write one entry of a table.
///
/// # Safety
///
/// As [`read`].
unsafe fn write(frames: &FrameAllocator, table: u64, slot: usize, entry: u64) {
    let at = frames.virt(Frame::from_addr(table)).cast::<u64>().wrapping_add(slot);
    // SAFETY: as above.
    unsafe { at.write_volatile(entry) };
}
