// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Reading what the loader left behind.
//!
//! # This is untrusted input
//!
//! The structure below was written by code the kernel did not run and cannot
//! audit. It is the first untrusted input the system ever reads, and it is read
//! before there is a capability table, a ring, or anywhere to report a problem
//! except the serial port. So: every field is validated before use, every walk
//! is bounded, and a malformed structure produces a refusal rather than a
//! fault. The rule that a peer must not be able to halt the system by writing
//! an integer applies to the loader too — it is simply the first peer.
//!
//! Multiboot 1, as implemented by QEMU's `-kernel` loader. See
//! `kernel/src/arch/x86_64/boot.rs` for why that protocol and not another.

/// Flag bit 6 of the info structure: the memory map fields are populated.
const FLAG_MMAP: u32 = 1 << 6;

/// Flag bit 0: `mem_lower` and `mem_upper` are populated.
const FLAG_MEM: u32 = 1 << 0;

/// Flag bit 2: `cmdline` is populated.
const FLAG_CMDLINE: u32 = 1 << 2;

/// Flag bit 3: `mods_count` and `mods_addr` are populated.
const FLAG_MODS: u32 = 1 << 3;

/// Flag bit 12: the framebuffer fields are populated.
///
/// The loader sets this when it honoured the video request in the header. It is
/// the *loader's* assertion about its own structure, so it is read the way
/// every other flag here is read — as permission to look at those words, never
/// as a promise about what is in them. [`Framebuffer::parse`] disbelieves the
/// contents separately.
const FLAG_FRAMEBUFFER: u32 = 1 << 12;

/// The narrowest framebuffer worth believing in, per axis. Unit: pixels.
///
/// Not a mode list and not a policy: a guard against a structure whose geometry
/// words are leftovers. Anything a person could read text on clears it by two
/// orders of magnitude, and a loader reporting a one-pixel display has told us
/// something other than a display.
const FB_DIMENSION_MIN: u32 = 16;

/// The widest, for the same reason and in the same spirit. Unit: pixels.
///
/// Sixteen thousand is past any display that exists and still finite, which is
/// the only property a bound read out of untrusted memory needs. It is *not* a
/// statement about what this system will drive: the component that draws is
/// given the geometry the machine reported, and this is where a geometry that
/// could only be corruption stops.
const FB_DIMENSION_MAX: u32 = 16_384;

/// How many loaded modules this kernel will keep track of.
///
/// One is what E0-B10 needs: `user/init`. Eight is room for the handful a
/// generation might carry without making the handoff structure large enough to
/// care about, and a ninth is *reported* as dropped rather than silently
/// ignored — because a module nobody reserved is a module the frame allocator
/// hands out from underneath its owner.
///
/// Public because `crate::generation` sizes an array by it and asserts the two
/// are one number: a second array sized by a copy of this bound is a place a
/// change to it goes unnoticed.
/// Unit: count of modules.
pub const MAX_MODULES: usize = 8;

/// How much of a command line this kernel will read.
///
/// Long enough for the parameters this frame has any use for, short enough to
/// sit in a structure that is copied by value. A longer one is truncated rather
/// than rejected: a parameter that does not fit is a parameter that does not
/// take effect, which is visible, and refusing to boot over it would not be.
///
/// **This was 128 and the arithmetic that moved it is worth keeping.** RFC 0012
/// puts two digests on the command line — `f.root=` is 71 bytes and `f.frame=`
/// is 72 — and 143 of 128 would have truncated the second every time. The
/// failure mode is exactly the one the paragraph above calls visible and would
/// not have been: a truncated `f.frame=` parses as malformed rather than as
/// absent, so the boot refuses and says so. It was still the wrong number. What
/// is left over is room for the provocations every boot command in `xtask`
/// appends beside them, and 320 is 143 plus the longest of those with headroom
/// rounded to a cache line.
///
/// *Reversal:* a frame that reads its generation out of a root record on a
/// mounted store rather than off the command line. Then both tokens go and this
/// is a boot parameter buffer again.
const CMDLINE_MAX: usize = 320;

/// A cap on the memory-map walk.
///
/// The map is a length-prefixed list, and a corrupt length is a loop that never
/// ends. QEMU reports a handful of regions; a real machine reports tens. Two
/// hundred and fifty-six is far past either and still finite, which is the only
/// property that matters here.
const MAX_REGIONS: usize = 256;

/// Read one 32-bit word at a word offset from an untrusted base.
///
/// The offset arithmetic is `wrapping_add`, which is *safe*: computing a wild
/// pointer is not the dangerous act, dereferencing one is. That is also what
/// keeps each `unsafe` block below to exactly one operation, as the frame's
/// lint requires — and the lint is right, because a block wrapping two
/// operations has a SAFETY comment that covers whichever one the reader thinks
/// of first.
///
/// # Safety
///
/// `base + words * 4` must be a readable address. Every caller establishes that
/// by bounds-checking against the length the loader itself declared, before
/// calling.
unsafe fn word_at(base: *const u32, words: usize) -> u32 {
    let at = base.wrapping_add(words);
    // SAFETY: the caller has established that this address lies inside the
    // loader's structure, which the boot stub's identity mapping covers. The
    // read is 32 bits wide and the structure is 4-byte aligned by the protocol,
    // so this is a plain aligned load; `volatile` because the memory belongs to
    // the loader and the compiler may not assume anything about it.
    unsafe { at.read_volatile() }
}

/// What the loader says a region of physical memory is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionKind {
    /// Free for the kernel to use.
    Usable,
    /// In use by firmware or hardware. Not ours.
    Reserved,
    /// Firmware tables that may be reclaimed once they have been read.
    AcpiReclaimable,
    /// Must be preserved across sleep states.
    AcpiNvs,
    /// The firmware reports this memory as faulty.
    Defective,
    /// A type this kernel does not know. Treated as reserved, because the
    /// conservative reading of an unknown region is that it is not ours.
    Unknown(u32),
}

impl RegionKind {
    fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::Usable,
            2 => Self::Reserved,
            3 => Self::AcpiReclaimable,
            4 => Self::AcpiNvs,
            5 => Self::Defective,
            other => Self::Unknown(other),
        }
    }

    /// A short label for the boot report.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Reserved => "reserved",
            Self::AcpiReclaimable => "acpi",
            Self::AcpiNvs => "acpi-nvs",
            Self::Defective => "defective",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// A file the loader placed in memory for the kernel to find.
///
/// Multiboot calls these modules; from E0-B10 onward the first of them is
/// `user/init`. What matters to M1 is narrower than what they contain: this is
/// memory the loader wrote and still owns, sitting inside a region the same
/// loader reported as usable, and it must be reserved before the frame
/// allocator is told that region is free.
#[derive(Clone, Copy, Debug)]
pub struct Module {
    /// First byte.
    pub start: u64,
    /// One past the last byte.
    pub end: u64,
}

impl Module {
    /// Length in bytes.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Is there nothing here?
    ///
    /// Never true of a module this type will hand out — a zero-length entry is
    /// rejected at parse time — but the shape of the type invites the question,
    /// and a `len` without an `is_empty` beside it is a clippy lint and a
    /// reasonable one.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The bytes the loader placed here.
    ///
    /// `'static` and shared, and both halves of that are claims worth reading.
    /// The memory is reserved before the frame allocator is populated — see
    /// `main::reserved_ranges` — so it is never handed to anybody and never
    /// freed, which is what makes the lifetime honest rather than convenient.
    /// Shared, because nothing in this kernel writes a module: it is copied out
    /// of, once, into a frame a process owns.
    ///
    /// # Safety
    ///
    /// The direct map must be live and must cover this module's physical
    /// extent, and `frames` must already have been rebound onto it — which
    /// together are what make [`super::paging::PHYS_OFFSET`] plus a physical
    /// address a readable one. Reading a module before the switch, through the
    /// boot stub's identity window, would also work and is not what any caller
    /// does; requiring the later state keeps one answer rather than two.
    #[must_use]
    pub unsafe fn bytes(self) -> &'static [u8] {
        let at = (super::paging::PHYS_OFFSET + self.start) as *const u8;
        // SAFETY: the caller's guarantee that the direct map covers this extent.
        // The length came from a validated handoff where `end` is checked to be
        // greater than `start`, and the region is reserved for the life of the
        // kernel, so nothing else can be writing it.
        unsafe { core::slice::from_raw_parts(at, self.len() as usize) }
    }
}

/// One region of physical memory, as the loader describes it.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// First byte.
    pub base: u64,
    /// Length in bytes. Never zero — empty regions are skipped.
    pub len: u64,
    /// What it is for.
    pub kind: RegionKind,
}

/// How a pixel is built, for a framebuffer the loader called direct colour.
///
/// Positions are bit offsets within a pixel and sizes are bit counts, which is
/// how multiboot states it and how a consumer wants it: the two together are
/// the whole of a channel and neither is useful alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Channels {
    /// Bit position of the red field.
    pub red_at: u8,
    /// Bits of red.
    pub red_bits: u8,
    /// Bit position of the green field.
    pub green_at: u8,
    /// Bits of green.
    pub green_bits: u8,
    /// Bit position of the blue field.
    pub blue_at: u8,
    /// Bits of blue.
    pub blue_bits: u8,
}

impl Channels {
    /// Eight bits each, blue lowest — the layout whose bytes in memory read
    /// blue, green, red, unused.
    ///
    /// Named because it is the one the display driver already pins:
    /// `f_virtio_gpu::driver::FORMAT` is `B8G8R8X8_UNORM`, which is these
    /// positions exactly. A firmware surface that reports this needs no
    /// conversion to carry the same pixels the ring already carries, and one
    /// that reports anything else is a second format — which is a fact about
    /// the machine worth printing rather than discovering in a renderer.
    const BGRX: Self =
        Self { red_at: 16, red_bits: 8, green_at: 8, green_bits: 8, blue_at: 0, blue_bits: 8 };

    /// The same widths with red lowest: bytes read red, green, blue, unused.
    const RGBX: Self =
        Self { red_at: 0, red_bits: 8, green_at: 8, green_bits: 8, blue_at: 16, blue_bits: 8 };

    /// A short label for the boot report, or `other` for a layout with no name
    /// here. The positions are printed beside it either way, so `other` costs
    /// the reader nothing but a second of arithmetic.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::BGRX => "b8g8r8x8",
            Self::RGBX => "r8g8b8x8",
            _ => "other",
        }
    }
}

/// What kind of surface the loader set up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FramebufferKind {
    /// Indexed colour through a palette this kernel does not read.
    Indexed,
    /// Direct colour, with the channel layout the loader reported.
    Direct(Channels),
    /// EGA text cells, not pixels. A legal answer to a video request and not an
    /// answer to *this* one — the header asks for linear graphics — so a loader
    /// that returns it has done something other than what was asked.
    EgaText,
    /// A type this kernel does not know, kept as a number because the number is
    /// the only honest thing to say about it.
    Unknown(u8),
}

impl FramebufferKind {
    /// A short label for the boot report.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Indexed => "indexed",
            Self::Direct(channels) => channels.label(),
            Self::EgaText => "ega-text",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// A linear framebuffer the loader set up and handed over.
///
/// Every field here was written by the loader and has been bounds-checked, but
/// **nothing in this structure has been mapped, reserved or touched.** It is a
/// report. `intent/0011` step 1 is deliberately the reading and not the using:
/// what reserves this range from the frame allocator and maps it
/// write-combining is step 2, and keeping them apart is what makes a boot that
/// prints a wrong geometry a wrong line rather than a wrong mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Framebuffer {
    /// Physical address of the first pixel.
    pub addr: u64,
    /// Bytes from one row to the next. Not `width * bytes_per_pixel` in
    /// general — a surface is usually padded to something the scanout engine
    /// likes, and a consumer that assumed otherwise would skew the image.
    /// Unit: bytes.
    pub pitch: u32,
    /// Unit: pixels.
    pub width: u32,
    /// Unit: pixels.
    pub height: u32,
    /// Bits in one pixel. Unit: bits.
    pub bits_per_pixel: u8,
    /// How a pixel is built.
    pub kind: FramebufferKind,
}

impl Framebuffer {
    /// Bytes the whole surface occupies, pitch included.
    ///
    /// Saturating rather than checked because [`Self::parse`] has already
    /// refused a geometry whose product overflows; this is the arithmetic done
    /// twice rather than trusted once.
    /// Unit: bytes.
    #[must_use]
    pub const fn extent(self) -> u64 {
        (self.pitch as u64).saturating_mul(self.height as u64)
    }

    /// One past the last byte.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.addr.saturating_add(self.extent())
    }

    /// Read the framebuffer fields out of the loader's structure.
    ///
    /// Returns the refusal's own sentence on anything that does not describe a
    /// surface. Refusing is cheap here and expensive later: these numbers are
    /// the ones a future step will map and write through, so a pitch narrower
    /// than a row or a height that overflows the extent is a structure to
    /// disbelieve now rather than a fault in a component that trusted it.
    ///
    /// # Safety
    ///
    /// `base` must be the loader's validated info structure, as established by
    /// [`BootInfo::new`], and the caller must have seen [`FLAG_FRAMEBUFFER`] in
    /// its flags — which is what says these words exist at all.
    unsafe fn parse(base: *const u32) -> Result<Self, &'static str> {
        // Words 22 through 28 are byte offsets 88 through 115: the address as
        // two words, then pitch, width, height, and two words of packed bytes.
        // SAFETY: the caller's guarantee. The framebuffer fields are the tail of
        // the fixed-size structure whose earlier words `BootInfo::new` has
        // already read, and the flag it checked is the loader saying they are
        // populated.
        let addr_lo = unsafe { word_at(base, 22) };
        // SAFETY: as above.
        let addr_hi = unsafe { word_at(base, 23) };
        // SAFETY: as above.
        let pitch = unsafe { word_at(base, 24) };
        // SAFETY: as above.
        let width = unsafe { word_at(base, 25) };
        // SAFETY: as above.
        let height = unsafe { word_at(base, 26) };
        // SAFETY: as above.
        let packed_low = unsafe { word_at(base, 27) };
        // SAFETY: as above.
        let packed_high = unsafe { word_at(base, 28) };

        let addr = (u64::from(addr_hi) << 32) | u64::from(addr_lo);
        let bits_per_pixel = (packed_low & 0xFF) as u8;
        let raw_kind = ((packed_low >> 8) & 0xFF) as u8;

        // The same floor every other loader-owned address in this file gets: a
        // structure that puts a display in the first page is describing
        // something other than a display.
        if addr < 0x1000 {
            return Err("the surface's address is in the first page");
        }
        if !(FB_DIMENSION_MIN..=FB_DIMENSION_MAX).contains(&width)
            || !(FB_DIMENSION_MIN..=FB_DIMENSION_MAX).contains(&height)
        {
            return Err("the geometry is outside anything a display could be");
        }
        if bits_per_pixel == 0 || bits_per_pixel > 32 {
            return Err("a pixel is zero bits wide, or wider than four bytes");
        }

        // A pitch below one row of pixels is the dangerous one, because it is
        // the field a writer multiplies by the row number: believing it would
        // put the last row's last pixel short of where the loader said the
        // surface ends, and every row after the first inside the previous one.
        let bytes_per_pixel = u32::from(bits_per_pixel).div_ceil(8);
        let row =
            width.checked_mul(bytes_per_pixel).ok_or("a row of pixels does not fit in 32 bits")?;
        if pitch < row {
            return Err("the pitch is narrower than one row");
        }

        // The extent is what a later step reserves, so an extent that does not
        // fit is refused here rather than saturating into a reservation that
        // covers less than the surface.
        let extent =
            u64::from(pitch).checked_mul(u64::from(height)).ok_or("the extent overflows")?;
        addr.checked_add(extent).ok_or("the surface ends past the end of the address space")?;

        let kind = match raw_kind {
            0 => FramebufferKind::Indexed,
            1 => FramebufferKind::Direct(Channels {
                red_at: ((packed_low >> 16) & 0xFF) as u8,
                red_bits: ((packed_low >> 24) & 0xFF) as u8,
                green_at: (packed_high & 0xFF) as u8,
                green_bits: ((packed_high >> 8) & 0xFF) as u8,
                blue_at: ((packed_high >> 16) & 0xFF) as u8,
                blue_bits: ((packed_high >> 24) & 0xFF) as u8,
            }),
            2 => FramebufferKind::EgaText,
            other => FramebufferKind::Unknown(other),
        };

        Ok(Self { addr, pitch, width, height, bits_per_pixel, kind })
    }

    /// Read a structure this kernel wrote, and say whether it read it back.
    ///
    /// # Why a fixture is the only way this can be checked here
    ///
    /// [`Self::parse`] is seven reads at seven word offsets into a structure
    /// somebody else's code wrote, and every one of those offsets is a number
    /// taken from a specification. An offset that is wrong by one word compiles,
    /// passes every lint, and produces a *plausible* framebuffer — a pitch read
    /// as a width, a height read as a depth — which is the kind of wrong that is
    /// found on the machine rather than in the tree.
    ///
    /// It cannot be exercised by any boot `xtask` can start, because QEMU's
    /// `-kernel` loader never sets the flag that reaches it. So the structure is
    /// built here instead, from the specification's byte offsets written out as
    /// arithmetic rather than as the word indices the parser uses — the two
    /// spellings are what make this a check and not a restatement — and handed
    /// to the real function.
    ///
    /// The obvious alternative, feeding a fixture to [`BootInfo::new`], is
    /// foreclosed by a detail worth recording: that function takes the pointer
    /// as a `u32` because a multiboot loader leaves one in `ebx`, and nothing
    /// this kernel can allocate after the address-space switch has an address
    /// that fits in thirty-two bits.
    ///
    /// Returns the parsed surface, or the reason it was refused.
    pub fn self_check() -> Result<Self, &'static str> {
        /// Byte offset to word index, which is the conversion the parser does
        /// in its head and this does on the page.
        const fn word(byte_offset: usize) -> usize {
            byte_offset / 4
        }

        // Thirty-two words is past the last field this parser reads, and the
        // structure a loader writes is longer still.
        let mut fixture = [0u32; 32];

        // The framebuffer block, at the byte offsets the specification gives.
        fixture[word(88)] = 0xFD00_0000; // address, low half
        fixture[word(92)] = 0; // address, high half
        fixture[word(96)] = 4096; // pitch
        fixture[word(100)] = 1024; // width
        fixture[word(104)] = 768; // height
        // The packed bytes, written as bytes in the order the specification
        // lists them rather than as a shifted-and-ored word: one of these
        // fields is at bit zero, and a `0 << 0` term is both noise and a lint.
        //
        // Byte 108 is bits per pixel, 109 the type, 110 and 111 the first
        // channel: thirty-two bits, direct colour, red at 16 for 8 bits — which
        // is the layout a display usually reports and the one the driver pins.
        fixture[word(108)] = u32::from_le_bytes([32, 1, 16, 8]);
        // Bytes 112 to 115: green at 8 for 8 bits, blue at 0 for 8.
        fixture[word(112)] = u32::from_le_bytes([8, 8, 0, 8]);

        // SAFETY: `fixture` is a live array this function owns, longer than the
        // last word `parse` reads, and correctly aligned for `u32` by its type.
        let parsed = unsafe { Self::parse(fixture.as_ptr()) }?;

        // Each of these is a field that would still be *plausible* if the offset
        // it came from were wrong, which is why every one is checked rather than
        // the geometry alone.
        if parsed.addr != 0xFD00_0000 {
            return Err("the address did not survive the round trip");
        }
        if parsed.pitch != 4096 {
            return Err("the pitch did not survive the round trip");
        }
        if parsed.width != 1024 || parsed.height != 768 {
            return Err("the geometry did not survive the round trip");
        }
        if parsed.bits_per_pixel != 32 {
            return Err("the depth did not survive the round trip");
        }
        // Written out rather than compared against the named constant in
        // `crate::screen`: that constant is what the drawing code *believes*
        // this layout is, and a check that asked one belief whether it matched
        // itself would pass however wrong both were.
        let expected = Channels {
            red_at: 16,
            red_bits: 8,
            green_at: 8,
            green_bits: 8,
            blue_at: 0,
            blue_bits: 8,
        };
        if parsed.kind != FramebufferKind::Direct(expected) {
            return Err("the channel layout did not survive the round trip");
        }
        if parsed.extent() != 4096 * 768 {
            return Err("the extent is not the pitch times the height");
        }

        Ok(parsed)
    }
}

/// What the loader did with the video request in the header.
///
/// Three answers rather than two, because *there was no answer* and *the answer
/// was not a surface* are different facts about a machine and the second one is
/// a defect somewhere. R04: an unreadable structure is absent and never
/// assumed-good, and which of the two it was is printed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Video {
    /// The loader set no framebuffer flag. Either it does not implement the
    /// video request — QEMU's `-kernel` loader says so on stderr and continues
    /// — or it could not satisfy it.
    Absent,
    /// The loader claimed the fields and they do not describe a surface.
    Refused(&'static str),
    /// A surface, validated.
    Present(Framebuffer),
}

/// The loader's handoff structure, validated.
#[derive(Clone, Copy)]
pub struct BootInfo {
    mmap_addr: u32,
    mmap_len: u32,
    mem_lower_kib: u32,
    mem_upper_kib: u32,
    /// Copied rather than pointed at. The loader put the string in low memory,
    /// which the kernel's own address space stops mapping — so a pointer kept
    /// here would be a fault waiting for the first read after the switch, in
    /// exactly the way the memory map was.
    cmdline: [u8; CMDLINE_MAX],
    cmdline_len: usize,
    /// Copied, for the same reason the command line is: the list lives in low
    /// memory the kernel's own address space stops mapping, and the extents are
    /// needed after the switch.
    modules: [Module; MAX_MODULES],
    module_count: usize,
    modules_dropped: usize,
    /// What the loader did with the header's video request. Copied by value
    /// like everything else here: it is four small integers, and a pointer into
    /// the loader's structure would be a fault waiting for the address-space
    /// switch in exactly the way the memory map was.
    video: Video,
}

/// Why a handoff was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BootError {
    /// `eax` did not hold the multiboot magic: whatever loaded this kernel does
    /// not speak the protocol, so nothing else it left is meaningful.
    NotMultiboot,
    /// The info pointer is null, or points below the first megabyte where no
    /// loader would place it.
    ImplausiblePointer,
    /// The loader did not provide a memory map. Without one there is nothing to
    /// build a frame allocator from, which is the entire reason for this
    /// handoff.
    NoMemoryMap,
}

impl BootError {
    /// A sentence for the serial log, since there is nowhere else to report.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NotMultiboot => "not entered by a multiboot loader",
            Self::ImplausiblePointer => "loader info pointer is implausible",
            Self::NoMemoryMap => "loader provided no memory map",
        }
    }
}

impl BootInfo {
    /// Validate a handoff.
    ///
    /// # Safety
    ///
    /// `info` must be what a multiboot loader left in `ebx`: either garbage
    /// this function will reject, or a pointer to a mapped, loader-owned
    /// structure. The boot stub identity-maps the first gigabyte before this
    /// runs, and multiboot places its structure well below that.
    pub unsafe fn new(magic: u32, info: u32) -> Result<Self, BootError> {
        if magic != super::boot::MULTIBOOT_MAGIC {
            return Err(BootError::NotMultiboot);
        }
        // Below 1 MiB is firmware and legacy device space. A loader that claims
        // to have put its structure there is not one to trust with the rest.
        if info < 0x1000 {
            return Err(BootError::ImplausiblePointer);
        }

        let base = info as usize as *const u32;

        // SAFETY: `base` is the loader's structure, which the boot stub's
        // identity mapping covers, and the fields read here are the fixed
        // prefix that multiboot 1 guarantees is present whenever the magic
        // matched.
        let flags = unsafe { word_at(base, 0) };

        if flags & FLAG_MMAP == 0 {
            return Err(BootError::NoMemoryMap);
        }

        // SAFETY: as above. Offsets are in 32-bit words: 11 and 12 are
        // `mmap_length` and `mmap_addr`, 1 and 2 are `mem_lower`/`mem_upper`.
        let mmap_len = unsafe { word_at(base, 11) };
        // SAFETY: as above.
        let mmap_addr = unsafe { word_at(base, 12) };

        let (mem_lower_kib, mem_upper_kib) = if flags & FLAG_MEM != 0 {
            // SAFETY: as above.
            let lower = unsafe { word_at(base, 1) };
            // SAFETY: as above.
            let upper = unsafe { word_at(base, 2) };
            (lower, upper)
        } else {
            (0, 0)
        };

        if mmap_len == 0 || mmap_addr < 0x1000 {
            return Err(BootError::NoMemoryMap);
        }

        let mut cmdline = [0u8; CMDLINE_MAX];
        let mut cmdline_len = 0;
        if flags & FLAG_CMDLINE != 0 {
            // SAFETY: as above — word 4 is `cmdline`, a pointer the loader owns
            // and the identity window still covers.
            let ptr = unsafe { word_at(base, 4) };
            if ptr >= 0x1000 {
                let mut at = ptr as usize as *const u8;
                while cmdline_len < CMDLINE_MAX {
                    // SAFETY: the loader's string is NUL-terminated and lives in
                    // memory the identity window covers. The bound stops the walk
                    // whether or not the terminator is where it should be, which
                    // is the rule for every other loader-owned structure here.
                    let byte = unsafe { at.read_volatile() };
                    if byte == 0 {
                        break;
                    }
                    cmdline[cmdline_len] = byte;
                    cmdline_len += 1;
                    at = at.wrapping_add(1);
                }
            }
        }

        let mut modules = [Module { start: 0, end: 0 }; MAX_MODULES];
        let mut module_count = 0;
        let mut modules_dropped = 0;
        if flags & FLAG_MODS != 0 {
            // SAFETY: as above — words 5 and 6 are `mods_count` and
            // `mods_addr`, inside the fixed prefix the magic vouched for.
            let count = unsafe { word_at(base, 5) };
            // SAFETY: as above.
            let addr = unsafe { word_at(base, 6) };

            if addr >= 0x1000 {
                for index in 0..count {
                    // Each entry is four words: start, end, string, reserved.
                    let entry = (addr as usize as *const u32).wrapping_add(index as usize * 4);
                    // SAFETY: the loader declared `count` entries at `addr`, and
                    // the walk is bounded by that count — this is the same trust
                    // the memory map is read under, with the same bound.
                    let start = unsafe { word_at(entry, 0) };
                    // SAFETY: as above.
                    let end = unsafe { word_at(entry, 1) };

                    // A module that ends before it starts, or sits in the first
                    // page, is a structure to disbelieve rather than to reserve
                    // — reserving a wild range would remove real memory from
                    // the allocator on the loader's say-so.
                    if end <= start || start < 0x1000 {
                        continue;
                    }
                    if module_count == MAX_MODULES {
                        modules_dropped += 1;
                        continue;
                    }
                    modules[module_count] = Module { start: u64::from(start), end: u64::from(end) };
                    module_count += 1;
                }
            }
        }

        let video = if flags & FLAG_FRAMEBUFFER == 0 {
            Video::Absent
        } else {
            // SAFETY: `base` is the structure validated above, and the flag the
            // branch tested is the loader's own statement that the framebuffer
            // words are populated — which is the whole of what `parse` needs
            // and none of what it trusts.
            match unsafe { Framebuffer::parse(base) } {
                Ok(fb) => Video::Present(fb),
                Err(why) => Video::Refused(why),
            }
        };

        Ok(Self {
            mmap_addr,
            mmap_len,
            mem_lower_kib,
            mem_upper_kib,
            cmdline,
            cmdline_len,
            modules,
            module_count,
            modules_dropped,
            video,
        })
    }

    /// What the loader did with the header's video request.
    ///
    /// Nothing in the frame acts on this yet, which is `intent/0011` step 1
    /// being exactly one step: the boot reports what it was given, and the
    /// reservation, the mapping and the component that draws are each their own
    /// change with their own evidence. A build that starts consuming this
    /// without reserving the extent first has handed a display's memory to the
    /// frame allocator.
    #[must_use]
    pub const fn video(&self) -> Video {
        self.video
    }

    /// The modules the loader placed in memory, as extents.
    ///
    /// Every one of these must be in the reserved list before a region
    /// containing it is offered to the frame allocator.
    #[must_use]
    pub fn modules(&self) -> &[Module] {
        &self.modules[..self.module_count]
    }

    /// Modules the loader reported and this kernel did not keep.
    ///
    /// Non-zero means [`MAX_MODULES`] was too small on this machine, and the
    /// memory those modules occupy is *not* reserved. It is counted here so
    /// that the decision has a number to be made from, and the decision is made
    /// where a module's contents are first depended on — `main::component`,
    /// which refuses to boot rather than reading a component out of memory
    /// something else may already have been handed.
    #[must_use]
    pub const fn modules_dropped(&self) -> usize {
        self.modules_dropped
    }

    /// Memory below 1 MiB, in kibibytes, as the loader counted it.
    #[must_use]
    pub fn mem_lower_kib(&self) -> u32 {
        self.mem_lower_kib
    }

    /// Memory above 1 MiB, in kibibytes, as the loader counted it.
    ///
    /// This is a summary and the memory map is the truth; they can disagree,
    /// and where they do the map wins. It is reported because a disagreement is
    /// itself worth seeing.
    #[must_use]
    pub fn mem_upper_kib(&self) -> u32 {
        self.mem_upper_kib
    }

    /// The command line the loader was given, as far as it was read.
    ///
    /// Empty when there was none. Not valid UTF-8 necessarily — a loader will
    /// pass whatever it was handed — so the bytes are what is offered, and a
    /// caller that wants text says so.
    #[must_use]
    pub fn cmdline(&self) -> &[u8] {
        &self.cmdline[..self.cmdline_len]
    }

    /// Does the command line contain this parameter?
    ///
    /// A substring test rather than a parser. Phase 00 has two parameters and
    /// no need for a grammar; when it has a seed to select and a policy to
    /// name, this grows into one and the callers do not change.
    #[must_use]
    pub fn has_parameter(&self, needle: &[u8]) -> bool {
        if needle.is_empty() || needle.len() > self.cmdline_len {
            return false;
        }
        self.cmdline().windows(needle.len()).any(|window| window == needle)
    }

    /// The decimal number following this parameter, if there is one.
    ///
    /// `parameter_u32(b"timer=")` on a command line of `timer=60` gives 60.
    /// `None` if the parameter is absent, if nothing follows it, or if what
    /// follows would not fit — an out-of-range value is refused rather than
    /// wrapped, because a boot parameter that silently becomes a different
    /// number is worse than one that is ignored.
    ///
    /// Still not a parser, for the same reason [`Self::has_parameter`] is not:
    /// the needle includes its own `=`, so a caller gets exactly the grammar it
    /// wrote and nothing has to agree about separators.
    #[must_use]
    pub fn parameter_u32(&self, needle: &[u8]) -> Option<u32> {
        let line = self.cmdline();
        if needle.is_empty() || needle.len() > line.len() {
            return None;
        }

        let at = line.windows(needle.len()).position(|window| window == needle)?;
        let digits = line[at + needle.len()..].iter().copied().take_while(u8::is_ascii_digit);

        let mut value: u32 = 0;
        let mut any = false;
        for digit in digits {
            value = value.checked_mul(10)?.checked_add(u32::from(digit - b'0'))?;
            any = true;
        }

        if any { Some(value) } else { None }
    }

    /// Where the loader's own memory map lives, as a base and a length.
    ///
    /// The frame allocator has to be told: the map is read lazily by
    /// [`Self::regions`], so handing out the frames it sits in would corrupt
    /// the structure being walked to decide which frames to hand out.
    #[must_use]
    pub fn mmap_extent(&self) -> (u64, u64) {
        (u64::from(self.mmap_addr), u64::from(self.mmap_len))
    }

    /// Walk the memory map.
    #[must_use]
    pub fn regions(&self) -> Regions {
        Regions {
            cursor: self.mmap_addr,
            end: self.mmap_addr.saturating_add(self.mmap_len),
            remaining: MAX_REGIONS,
        }
    }
}

/// An iterator over the loader's memory map.
///
/// Bounded twice over: by the declared length, and by [`MAX_REGIONS`]. A map
/// whose entry sizes are corrupt terminates the walk rather than the machine.
pub struct Regions {
    cursor: u32,
    end: u32,
    remaining: usize,
}

impl Iterator for Regions {
    type Item = Region;

    fn next(&mut self) -> Option<Region> {
        loop {
            // Each entry is a u32 size followed by `size` bytes, so the
            // smallest entry that could describe anything is 20 bytes.
            if self.remaining == 0 || self.cursor.saturating_add(20) > self.end {
                return None;
            }

            let entry = self.cursor as usize as *const u32;

            // SAFETY: the entry lies inside the map the loader declared —
            // the bounds check above enforces it, and the boot stub's identity
            // mapping covers the address.
            let size = unsafe { word_at(entry, 0) };
            // SAFETY: as above.
            let base_lo = unsafe { word_at(entry, 1) };
            // SAFETY: as above.
            let base_hi = unsafe { word_at(entry, 2) };
            // SAFETY: as above.
            let len_lo = unsafe { word_at(entry, 3) };
            // SAFETY: as above.
            let len_hi = unsafe { word_at(entry, 4) };
            // SAFETY: as above.
            let kind = unsafe { word_at(entry, 5) };

            // The size field excludes itself. A zero or absurd size is the
            // corruption case: stop walking rather than spin or run off.
            let step = size.saturating_add(4);
            if step < 24 {
                return None;
            }
            self.cursor = self.cursor.saturating_add(step);
            self.remaining -= 1;

            let len = (u64::from(len_hi) << 32) | u64::from(len_lo);
            if len == 0 {
                continue;
            }

            return Some(Region {
                base: (u64::from(base_hi) << 32) | u64::from(base_lo),
                len,
                kind: RegionKind::from_raw(kind),
            });
        }
    }
}
