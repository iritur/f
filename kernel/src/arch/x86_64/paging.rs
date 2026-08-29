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
//! # What this is still not
//!
//! No per-process address spaces, no user pages, no lazy mapping, no shootdown,
//! and no unmapping at all. Those arrive with M3 and M4, when there is something
//! to isolate from something else and a second core to tell about it.

use crate::mem::{Frame, FrameAllocator, Order};

/// Where physical memory appears in the kernel's address space.
///
/// The bottom of the higher half, which leaves the entire lower half for user
/// space and puts the direct map a long way from the kernel image — so a stray
/// physical address used as a virtual one lands in unmapped space rather than in
/// the middle of the kernel.
pub const PHYS_OFFSET: u64 = 0xFFFF_8000_0000_0000;

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

/// One `cpuid` leaf, as `(ebx, ecx, edx)`.
///
/// # Safety
///
/// None beyond the instruction itself, which is unprivileged and has no memory
/// effect. `unsafe` because it is `asm!`.
unsafe fn cpuid(leaf: u32) -> (u32, u32, u32) {
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    // SAFETY: `rbx` is reserved by the compiler, so it is saved and restored
    // around the instruction rather than named as an output. The target
    // disables the red zone, so using the stack here is sound.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx:e}, ebx",
            "pop rbx",
            ebx = lateout(reg) ebx,
            inout("eax") leaf => _,
            out("ecx") ecx,
            out("edx") edx,
            options(preserves_flags),
        );
    }
    (ebx, ecx, edx)
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
}

impl BuildError {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NoFrames => "not enough frames to build an address space",
            Self::TooMuchMemory => "more physical memory than the direct map covers",
            Self::Overlap => "a mapping collided with a larger one",
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
