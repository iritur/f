// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The firmware's tables, read as untrusted input.
//!
//! # Why the kernel goes looking rather than being told
//!
//! Multiboot 1 hands over a memory map, a command line and a module list, and
//! nothing else. There is no field in it for the root system description
//! pointer, so a kernel that wants ACPI has to find it the way the
//! specification says it is findable: a sixteen-byte-aligned signature in one
//! of two windows of low memory, with a checksum over the structure that
//! carries it. That scan is the entire reason this module exists, and it is
//! also the reason every function in it is written as a parser of hostile bytes
//! rather than as a reader of a struct.
//!
//! The memory map does show both windows this kernel cares about — the PCIe
//! configuration space and the remapping unit's registers appear on it as
//! reserved ranges. Reading their addresses off the map would be guessing from
//! a coincidence: the map says *something is here*, and only ACPI says *what*.
//! A kernel that mapped 0xB000_0000 as configuration space because a reserved
//! range started there would be right on this emulator and wrong on the first
//! machine that laid its memory out differently, with no error at any point.
//!
//! # Everything here crossed a trust boundary
//!
//! Firmware writes these tables. The kernel does not control their lengths,
//! their signatures or their checksums, and a table that claims to be four
//! gibibytes long is a table this kernel must refuse rather than walk. So:
//!
//! - every length is checked against [`MAX_TABLE`] *and* against what the
//!   direct map can address, before a single byte past the header is read;
//! - every table's checksum is summed over its own claimed length, and a
//!   non-zero sum is a refusal;
//! - every entry count is derived from a length that has already been checked,
//!   so a malformed length produces a short walk rather than a wild one;
//! - a signature is compared in full. Three of the four bytes matching is not
//!   a match.
//!
//! Fail closed, R04: an unreadable table is *absent*, never assumed-good.
//!
//! # What is deliberately not here
//!
//! No AML, no namespace, no interpreter. This kernel reads two tables by
//! signature — `MCFG` for where configuration space is, `DMAR` for where the
//! remapping units are — and stops. Everything else a general ACPI
//! implementation is for is policy about power and devices that this system has
//! no opinion about yet, and an interpreter for a bytecode written by firmware
//! is a large attack surface to acquire before there is anything to spend it
//! on.
//!
//! *Reversal:* a machine whose remapping units cannot be described without
//! `_DSM`, or an interrupt routing problem the static tables cannot answer.
//! Both are E5's, when there is a machine rather than an emulator.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use crate::mem::{Frame, FrameAllocator};

/// The signature that marks the root pointer.
const RSDP_SIGNATURE: [u8; 8] = *b"RSD PTR ";

/// How long the first revision of the root pointer is, and how much of every
/// later revision the first checksum covers.
const RSDP_V1_LEN: u32 = 20;

/// The full length of a revision-2 root pointer.
const RSDP_V2_LEN: u32 = 36;

/// Bytes in every table header: signature, length, revision, checksum, and the
/// six identification fields nothing here reads.
const HEADER_LEN: u32 = 36;

/// The largest table this kernel will walk.
///
/// A mebibyte. Real tables are hundreds of bytes and the largest thing a PC
/// puts in one is a differentiated description table of a few tens of
/// kibibytes; this is three orders of magnitude past that. It exists so that a
/// length field of `0xFFFF_FFFF` — which is what a torn table or a hostile one
/// looks like — is refused by a bound rather than absorbed by a loop somebody
/// hoped would terminate.
const MAX_TABLE: u32 = 1 << 20;

/// Where the firmware records the extended BIOS data area, as a paragraph.
const EBDA_POINTER: u64 = 0x040E;

/// How much of the extended BIOS data area the specification says to scan.
const EBDA_SCAN: u64 = 1024;

/// The lower bound of the second window the specification names.
const BIOS_SCAN_START: u64 = 0x000E_0000;

/// One past the upper end of that window.
const BIOS_SCAN_END: u64 = 0x0010_0000;

/// The alignment the root pointer is guaranteed to sit on.
const RSDP_ALIGN: u64 = 16;

/// A window onto physical memory that has been checked against what is mapped.
///
/// Every read in this module goes through one of these rather than through a
/// bare pointer, and the reason is the one `paging` already gives for the
/// direct map having a *limit* that is reported rather than assumed: a firmware
/// table at an address the direct map does not reach is a page fault at ring 0,
/// inside a module whose whole job is to survive bad input. It is not
/// hypothetical — the remapping unit's own registers sit at 0xFED9_0000 on this
/// emulator, two orders of magnitude above where a 128 MiB machine's direct map
/// ends, which is why they are mapped through the device window instead. That
/// address appears in no memory-map entry at all, incidentally: the reserved
/// range at 0xFED1_C000 that a reader might take for it is the chipset's own
/// root-complex block. Which is the module's thesis in one address.
#[derive(Clone, Copy)]
pub struct Phys {
    offset: u64,
    limit: u64,
}

impl Phys {
    /// A window over the allocator's direct map.
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the address space
    /// currently in `CR3`, and `limit` must be that map's own limit — which is
    /// [`AddressSpace::direct_limit`](super::paging::AddressSpace::direct_limit).
    #[must_use]
    pub unsafe fn new(frames: &FrameAllocator, limit: u64) -> Self {
        // A frame at physical zero reaches the base of the window, which is the
        // one piece of arithmetic this needs from the allocator and the one
        // place it is asked for.
        Self { offset: frames.virt(Frame::from_addr(0)) as u64, limit }
    }

    /// Is every byte of `len` at `addr` inside the window?
    ///
    /// Checked rather than wrapping, because `addr` and `len` both come from
    /// firmware and a sum that wrapped would answer *yes* for a range starting
    /// near the top of the address space.
    fn covers(&self, addr: u64, len: u64) -> bool {
        addr.checked_add(len).is_some_and(|end| end <= self.limit)
    }

    /// Read one byte, or `None` where the window does not reach.
    fn byte(&self, addr: u64) -> Option<u8> {
        if !self.covers(addr, 1) {
            return None;
        }
        let at = self.offset.wrapping_add(addr) as *const u8;
        // SAFETY: `covers` has established the byte is inside the direct map,
        // which maps every usable physical address as ordinary readable memory.
        // Volatile because these are bytes this kernel does not own and the
        // compiler may not assume it is the only reader of them.
        Some(unsafe { at.read_volatile() })
    }

    /// Read one little-endian `u16`.
    fn u16_at(&self, addr: u64) -> Option<u16> {
        let low = u16::from(self.byte(addr)?);
        let high = u16::from(self.byte(addr.wrapping_add(1))?);
        Some(low | (high << 8))
    }

    /// Read one little-endian `u32`.
    fn u32_at(&self, addr: u64) -> Option<u32> {
        let low = u32::from(self.u16_at(addr)?);
        let high = u32::from(self.u16_at(addr.wrapping_add(2))?);
        Some(low | (high << 16))
    }

    /// Read one little-endian `u64`.
    ///
    /// Built out of narrower reads rather than one eight-byte load, because
    /// nothing in a firmware table is guaranteed to be eight-byte aligned — the
    /// extended root table's entry array starts thirty-six bytes into a table
    /// whose own alignment nobody promised — and an unaligned volatile load is
    /// undefined rather than merely slow.
    fn u64_at(&self, addr: u64) -> Option<u64> {
        let low = u64::from(self.u32_at(addr)?);
        let high = u64::from(self.u32_at(addr.wrapping_add(4))?);
        Some(low | (high << 32))
    }

    /// Sum `len` bytes at `addr`, modulo 256, the way every ACPI checksum is
    /// defined.
    fn checksum(&self, addr: u64, len: u64) -> Option<u8> {
        if !self.covers(addr, len) {
            return None;
        }
        let mut sum: u8 = 0;
        let mut at = addr;
        let end = addr.wrapping_add(len);
        while at < end {
            sum = sum.wrapping_add(self.byte(at)?);
            at = at.wrapping_add(1);
        }
        Some(sum)
    }
}

/// How many usable regions this build will remember.
///
/// Sixteen. A PC-class memory map has between three and ten; sixteen is past
/// every machine this kernel has booted on and small enough to sit in a
/// `discover` frame. What matters is not the number but the direction it fails
/// in — see [`Ram::add`].
const MAX_SPANS: usize = 16;

/// One span of memory the loader called usable. Unit: bytes.
#[derive(Clone, Copy)]
struct Span {
    base: u64,
    len: u64,
}

/// What the loader said is ordinary memory.
///
/// # Why a firmware table's addresses are checked against this at all
///
/// Because `MCFG` and `DMAR` do not describe memory: they describe *device
/// registers*, and the kernel's response to being told where some are is
/// [`super::paging::map_device`], whose safety obligation says in as many
/// words that pointing it at ordinary memory aliases that memory with
/// different caching and corrupts it without any hardware reporting anything.
/// The checksum on the table does not discharge that obligation — it
/// establishes that firmware wrote what it meant to, not that what it meant
/// was outside RAM — and the version-register sanity check happens after the
/// mapping already exists. So the loader's own memory map is the second
/// opinion, and it is the only one available at this point in the boot.
///
/// # When this has to be filled
///
/// Before the kernel's own address space is activated, and this is not advice.
/// `BootInfo::regions` walks the loader's map through the boot stub's identity
/// mapping; after the switch that mapping is gone and the walk is a page fault
/// at ring 0, inside the discovery stage whose entire purpose is to survive bad
/// input. So `kmain` copies the map into one of these while it is still
/// readable and hands it down, and the copy is why this type owns an array
/// rather than borrowing an iterator.
///
/// *Reversal:* a machine whose remapping unit genuinely sits inside a range
/// the loader reports as usable, which would be firmware describing one region
/// two ways. The answer there is to believe ACPI over multiboot and say so,
/// which is a change to this one function.
pub struct Ram {
    spans: [Span; MAX_SPANS],
    count: usize,
    /// Whether a region was dropped for want of room.
    ///
    /// The whole reason this field exists: a build that quietly forgot the
    /// seventeenth region would accept a device base inside it, which is the
    /// failure this type exists to refuse arrived at by running out of an
    /// array. When it is set, [`Ram::overlaps`] answers *yes* to everything and
    /// the machine loses its IOMMU rather than gaining a corruption. R04.
    overflowed: bool,
}

impl Default for Ram {
    fn default() -> Self {
        Self::new()
    }
}

impl Ram {
    /// A map with nothing in it, which refuses nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self { spans: [Span { base: 0, len: 0 }; MAX_SPANS], count: 0, overflowed: false }
    }

    /// Note one usable region.
    pub fn add(&mut self, base: u64, len: u64) {
        if len == 0 {
            return;
        }
        match self.spans.get_mut(self.count) {
            Some(slot) => {
                *slot = Span { base, len };
                self.count = self.count.saturating_add(1);
            }
            None => self.overflowed = true,
        }
    }

    /// Does `len` bytes at `base` touch any of it?
    #[must_use]
    pub fn overlaps(&self, base: u64, len: u64) -> bool {
        if self.overflowed {
            return true;
        }
        let Some(end) = base.checked_add(len) else { return true };
        self.spans.get(..self.count).unwrap_or(&[]).iter().any(|span| {
            let Some(span_end) = span.base.checked_add(span.len) else { return true };
            base < span_end && span.base < end
        })
    }

    /// Is this an address a device window may start at — page-aligned, and
    /// outside everything the loader called usable?
    fn allows(&self, base: u64, len: u64) -> Result<(), Absent> {
        if !base.is_multiple_of(4096) {
            return Err(Absent::Misaligned);
        }
        if self.overlaps(base, len) {
            return Err(Absent::NotDeviceMemory);
        }
        Ok(())
    }
}

/// Why the tables could not be read.
///
/// Every one of these describes the *machine* rather than a bug here, which is
/// why the boot path prints them and carries on rather than stopping. A machine
/// with no ACPI is a machine this kernel runs on with one subsystem fewer, and
/// `-machine pc` is exactly that machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Absent {
    /// Neither window held a checksummed root pointer.
    NoRsdp,
    /// The root pointer named a description table that does not validate —
    /// wrong signature, impossible length, or a bad checksum.
    BadRoot,
    /// The tables validate and none of them is the one asked for.
    NoTable,
    /// A table validated its header and then described something this build
    /// cannot use: an entry array running past the table's own length, or a
    /// remapping structure whose stated length is shorter than its own header.
    Malformed,
    /// The table sits at a physical address the direct map does not reach. Not
    /// a bad table — a machine with more memory than this kernel maps — and
    /// worth telling apart from corruption.
    Unreachable,
    /// A table validated and then named, as device registers, an address that
    /// overlaps memory the loader called usable.
    ///
    /// A checksum says firmware wrote the number it meant to write. It says
    /// nothing about the number being outside RAM, and the next thing that
    /// happens to a register base is an uncacheable writable mapping — which
    /// aliases whatever else owns that memory with different caching, and is a
    /// corruption the hardware does not report. So the two claims are checked
    /// separately: the checksum, and then this.
    NotDeviceMemory,
    /// A register base or a configuration-space base that is not page-aligned.
    ///
    /// Both are mapped a page at a time, so an unaligned base would silently
    /// become the page below it — a window offset from where firmware said it
    /// was, read as though it were not.
    Misaligned,
}

impl Absent {
    /// A sentence for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoRsdp => "no checksummed root pointer in either window",
            Self::BadRoot => "the root pointer names a table that does not validate",
            Self::NoTable => "the tables validate and do not include this one",
            Self::Malformed => "the table's own lengths do not describe it",
            Self::Unreachable => "the table is above what the direct map reaches",
            Self::NotDeviceMemory => "the table calls memory the loader gave us device registers",
            Self::Misaligned => "the window firmware described does not start on a page",
        }
    }
}

/// The root pointer, once it has checksummed.
#[derive(Clone, Copy, Debug)]
pub struct Root {
    /// Physical address of the description table this kernel chose to walk.
    table: u64,
    /// Whether that table's entries are eight bytes rather than four.
    extended: bool,
    /// What the pointer said about itself, so the boot log can say which of the
    /// two shapes the firmware handed over.
    pub revision: u8,
}

/// Find the root pointer by the scan the specification defines.
///
/// Two windows, in the order ACPI names them: the first kibibyte of the
/// extended BIOS data area, whose paragraph address the BIOS leaves in the BIOS
/// data area, and then the fixed range from 0xE0000 to 0xFFFFF. Sixteen bytes
/// apart, because the structure is guaranteed to be aligned that way and
/// scanning every byte would eventually find this signature inside a string.
///
/// # Errors
///
/// [`Absent::NoRsdp`] when neither window holds one that checksums, and
/// [`Absent::BadRoot`] when one does and names a table that does not.
pub fn root(phys: &Phys) -> Result<Root, Absent> {
    let ebda = phys.u16_at(EBDA_POINTER).unwrap_or(0);
    // A paragraph is sixteen bytes. Zero means the BIOS left no pointer, and
    // scanning from address zero would be scanning the real-mode interrupt
    // vector table for a coincidence.
    let ebda_base = u64::from(ebda).wrapping_mul(16);

    let windows: [(u64, u64); 2] = [
        (ebda_base, if ebda_base == 0 { 0 } else { EBDA_SCAN }),
        (BIOS_SCAN_START, BIOS_SCAN_END.wrapping_sub(BIOS_SCAN_START)),
    ];

    for (base, len) in windows {
        let mut at = base;
        let end = base.wrapping_add(len);
        while at < end {
            if let Some(found) = candidate(phys, at) {
                return found;
            }
            at = at.wrapping_add(RSDP_ALIGN);
        }
    }

    Err(Absent::NoRsdp)
}

/// Is there a root pointer at exactly this address, and if so what does it say?
///
/// `None` means *no signature here, keep scanning*. `Some(Err(..))` means a
/// signature that checksummed and then described something unusable, which
/// stops the scan: two structures carrying this signature in one machine's low
/// memory is not a case to paper over by quietly taking the second.
fn candidate(phys: &Phys, at: u64) -> Option<Result<Root, Absent>> {
    for (index, want) in RSDP_SIGNATURE.iter().enumerate() {
        let offset = u64::try_from(index).ok()?;
        if phys.byte(at.wrapping_add(offset))? != *want {
            return None;
        }
    }

    // The first checksum covers the original twenty bytes and no more, on every
    // revision. A revision-2 pointer that failed this is not a revision-2
    // pointer worth reading the rest of.
    if phys.checksum(at, u64::from(RSDP_V1_LEN))? != 0 {
        return None;
    }

    let revision = phys.byte(at.wrapping_add(15))?;
    let rsdt = u64::from(phys.u32_at(at.wrapping_add(16))?);

    if revision < 2 {
        return Some(validate_root(phys, rsdt, false, revision));
    }

    // The extended pointer's own length, which the firmware states rather than
    // this kernel assuming. Anything shorter than the structure it claims to be
    // is refused: a length below the fields about to be read is the one case
    // where trusting it reads somebody else's memory.
    let length = phys.u32_at(at.wrapping_add(20))?;
    if !(RSDP_V2_LEN..=MAX_TABLE).contains(&length) {
        return Some(Err(Absent::BadRoot));
    }
    if phys.checksum(at, u64::from(length))? != 0 {
        return Some(Err(Absent::BadRoot));
    }

    let xsdt = phys.u64_at(at.wrapping_add(24))?;
    // A revision-2 pointer with a null extended table is legal and means *use
    // the short one*. Preferring the extended table where there is one is not a
    // preference for the newer thing: the short table's entries are four bytes,
    // so a table above four gibibytes is unnameable in it, and a machine that
    // has one is a machine whose short table is silently incomplete.
    if xsdt == 0 {
        return Some(validate_root(phys, rsdt, false, revision));
    }
    Some(validate_root(phys, xsdt, true, revision))
}

/// Check that the table the root pointer names is a description table.
fn validate_root(phys: &Phys, table: u64, extended: bool, revision: u8) -> Result<Root, Absent> {
    let want = if extended { *b"XSDT" } else { *b"RSDT" };
    header(phys, table, &want)?;
    Ok(Root { table, extended, revision })
}

/// A validated table: where it is and how long it said it was.
#[derive(Clone, Copy, Debug)]
pub struct Table {
    /// Physical address of the first byte of the header. Unit: bytes.
    pub base: u64,
    /// The whole table's length, header included. Unit: bytes.
    pub length: u32,
}

impl Table {
    /// Bytes after the header, saturating rather than underflowing.
    const fn body(&self) -> u32 {
        self.length.saturating_sub(HEADER_LEN)
    }
}

/// Read and check one table header.
///
/// The three checks are made in the order that keeps each one meaningful: the
/// signature first, so a table nobody asked for is rejected before its length is
/// believed; the length next, against both the specification's floor and this
/// kernel's ceiling, so the checksum has a bounded range to sum over; and the
/// checksum last, over exactly the length that has just been bounded.
fn header(phys: &Phys, base: u64, want: &[u8; 4]) -> Result<Table, Absent> {
    if !phys.covers(base, u64::from(HEADER_LEN)) {
        return Err(Absent::Unreachable);
    }
    for (index, byte) in want.iter().enumerate() {
        let offset = u64::try_from(index).map_err(|_| Absent::Malformed)?;
        if phys.byte(base.wrapping_add(offset)).ok_or(Absent::Unreachable)? != *byte {
            return Err(Absent::NoTable);
        }
    }

    let length = phys.u32_at(base.wrapping_add(4)).ok_or(Absent::Unreachable)?;
    if !(HEADER_LEN..=MAX_TABLE).contains(&length) {
        return Err(Absent::Malformed);
    }
    if !phys.covers(base, u64::from(length)) {
        return Err(Absent::Unreachable);
    }
    if phys.checksum(base, u64::from(length)).ok_or(Absent::Unreachable)? != 0 {
        return Err(Absent::BadRoot);
    }

    Ok(Table { base, length })
}

/// Find one table by signature, by walking the root table's entry array.
///
/// # Errors
///
/// [`Absent::NoTable`] when the walk completes without a match — the ordinary
/// answer on a machine with no remapping unit — and the other variants when the
/// root table itself does not describe an array.
pub fn find(phys: &Phys, root: &Root, signature: &[u8; 4]) -> Result<Table, Absent> {
    let want = if root.extended { *b"XSDT" } else { *b"RSDT" };
    let table = header(phys, root.table, &want)?;

    let stride: u32 = if root.extended { 8 } else { 4 };
    // Integer division, so a body that is not a whole number of entries walks
    // the entries it does have and ignores the tail. A table three bytes longer
    // than its last entry is a table with padding, not a table to refuse.
    let count = table.body() / stride;

    for index in 0..count {
        let offset =
            u64::from(HEADER_LEN).wrapping_add(u64::from(index).wrapping_mul(u64::from(stride)));
        let at = table.base.wrapping_add(offset);
        let entry = if root.extended {
            phys.u64_at(at).ok_or(Absent::Unreachable)?
        } else {
            u64::from(phys.u32_at(at).ok_or(Absent::Unreachable)?)
        };
        if entry == 0 {
            continue;
        }
        match header(phys, entry, signature) {
            Ok(found) => return Ok(found),
            // A neighbouring table that is not the one asked for, or one this
            // kernel cannot reach, is no reason to stop looking for the one that
            // is. A table whose own checksum is wrong is: firmware that wrote
            // one bad table wrote it into this same array.
            Err(Absent::NoTable | Absent::Unreachable) => {}
            Err(other) => return Err(other),
        }
    }

    Err(Absent::NoTable)
}

// --- MCFG: where configuration space is ------------------------------------

/// One segment group's memory-mapped configuration space.
#[derive(Clone, Copy, Debug)]
pub struct Ecam {
    /// Physical base of the segment's configuration space. Unit: bytes.
    pub base: u64,
    /// Which segment group, as `MCFG` numbers them.
    pub segment: u16,
    /// The first bus number this window describes.
    pub start_bus: u8,
    /// The last bus number it describes, inclusive.
    pub end_bus: u8,
}

/// Where the first configuration-space window in `MCFG` is.
///
/// The *first*, and that is a limit stated rather than hidden: a machine with
/// several segment groups has several windows and this kernel reads one. Every
/// PC-class machine has exactly one segment, so the limit costs nothing here and
/// would cost correctness on a large server. *Reversal:* a machine whose
/// remapping unit is described under a segment other than the first, which is
/// also the machine where [`Drhd::segment`] stops always reading zero.
///
/// # Errors
///
/// [`Absent`], with [`Absent::NoTable`] the ordinary answer on a machine with no
/// memory-mapped configuration space at all.
pub fn ecam(phys: &Phys, root: &Root, ram: &Ram) -> Result<Ecam, Absent> {
    let table = find(phys, root, b"MCFG")?;
    // Eight reserved bytes sit between the header and the first entry. A body
    // shorter than that plus one entry describes no window.
    const RESERVED: u32 = 8;
    const ENTRY: u32 = 16;
    if table.body() < RESERVED.saturating_add(ENTRY) {
        return Err(Absent::Malformed);
    }

    let at = table.base.wrapping_add(u64::from(HEADER_LEN.saturating_add(RESERVED)));
    let base = phys.u64_at(at).ok_or(Absent::Unreachable)?;
    let segment = phys.u16_at(at.wrapping_add(8)).ok_or(Absent::Unreachable)?;
    let start_bus = phys.byte(at.wrapping_add(10)).ok_or(Absent::Unreachable)?;
    let end_bus = phys.byte(at.wrapping_add(11)).ok_or(Absent::Unreachable)?;

    if base == 0 || end_bus < start_bus {
        return Err(Absent::Malformed);
    }
    // The window is a mebibyte per bus, and every byte of it is about to be
    // mapped uncacheable. `Ram::allows` is where that stops being firmware's
    // word alone — see [`Ram`].
    let buses = u64::from(end_bus.saturating_sub(start_bus)).saturating_add(1);
    ram.allows(base, buses.saturating_mul(1 << 20))?;
    Ok(Ecam { base, segment, start_bus, end_bus })
}

// --- DMAR: where the remapping units are -----------------------------------

/// One DMA-remapping hardware unit definition.
#[derive(Clone, Copy, Debug)]
pub struct Drhd {
    /// Physical base of the unit's register set. Unit: bytes.
    pub register_base: u64,
    /// Which PCI segment group the unit covers.
    pub segment: u16,
    /// The unit covers every device in its segment that no other unit claims.
    ///
    /// The include-all flag. A unit without it covers only the device scopes
    /// listed after it, which this build does not read — see [`dmar`].
    pub include_all: bool,
    /// The raw flags byte, so a log can say what was read rather than only what
    /// was concluded from it. Worth carrying because the conclusion here is a
    /// judgement — see [`super::vtd::Unit::open`], which accepts a unit without
    /// the include-all flag when it is the only one in its segment.
    pub flags: u8,
}

/// What `DMAR` says about the machine.
#[derive(Clone, Copy, Debug)]
pub struct Dmar {
    /// The first remapping unit described.
    pub unit: Drhd,
    /// How many address bits the platform's DMA can carry, as the table states
    /// it: the stored value plus one. Unit: bits.
    pub host_address_width: u8,
    /// How many remapping structures the table held, of every type. Reported so
    /// the boot log can say when a machine has more than the one this build
    /// uses.
    pub structures: u32,
    /// How many of those were remapping hardware units.
    pub units: u32,
}

/// Read `DMAR` and take the first remapping unit out of it.
///
/// # What this reads and what it skips
///
/// The table is a header followed by a chain of variable-length structures, each
/// carrying its own type and length. This walks the chain, counts what it finds,
/// and keeps the first type-0 structure — a remapping hardware unit.
/// Reserved-memory regions, root-port address-translation capabilities and
/// affinity structures are counted and skipped.
///
/// **Device scopes are not read.** A unit's scope list says which functions it
/// covers when the include-all flag is clear; this build uses the first unit and
/// [`super::vtd`] requires that flag, so a machine with several scoped units is
/// refused rather than covered by the wrong unit. The difference between *no
/// IOMMU* and *an IOMMU this build will not drive* matters to whoever reads the
/// log, so the two are separate lines rather than one.
///
/// # Errors
///
/// [`Absent`], with [`Absent::NoTable`] on a machine with no `DMAR` at all.
pub fn dmar(phys: &Phys, root: &Root, ram: &Ram) -> Result<Dmar, Absent> {
    let table = find(phys, root, b"DMAR")?;
    // Host address width, flags, and ten reserved bytes.
    const FIXED: u32 = 12;
    if table.body() < FIXED {
        return Err(Absent::Malformed);
    }

    let width =
        phys.byte(table.base.wrapping_add(u64::from(HEADER_LEN))).ok_or(Absent::Unreachable)?;

    let mut at = u64::from(HEADER_LEN.saturating_add(FIXED));
    let end = u64::from(table.length);
    let mut structures: u32 = 0;
    let mut units: u32 = 0;
    let mut first: Option<Drhd> = None;

    // Four bytes is the smallest a structure can be and still carry its own type
    // and length. The loop advances by the length the structure states, which is
    // the one number that could keep it from terminating — so a length below the
    // header's own size ends the walk rather than repeating it.
    while at.wrapping_add(4) <= end {
        let base = table.base.wrapping_add(at);
        let kind = phys.u16_at(base).ok_or(Absent::Unreachable)?;
        let length = u64::from(phys.u16_at(base.wrapping_add(2)).ok_or(Absent::Unreachable)?);
        if length < 4 || at.wrapping_add(length) > end {
            return Err(Absent::Malformed);
        }
        structures = structures.saturating_add(1);

        // Type 0 is a remapping hardware unit definition: flags, one reserved
        // byte, the segment, and then the register base.
        if kind == 0 {
            if length < 16 {
                return Err(Absent::Malformed);
            }
            units = units.saturating_add(1);
            if first.is_none() {
                let flags = phys.byte(base.wrapping_add(4)).ok_or(Absent::Unreachable)?;
                let segment = phys.u16_at(base.wrapping_add(6)).ok_or(Absent::Unreachable)?;
                let register_base = phys.u64_at(base.wrapping_add(8)).ok_or(Absent::Unreachable)?;
                if register_base == 0 {
                    return Err(Absent::Malformed);
                }
                // As `ecam`: a register base is an address about to become an
                // uncacheable writable mapping, and a checksum is not a claim
                // about where it points. `MAX_REGISTER_PAGES` pages is the most
                // `vtd` will map of it, and the whole of that is checked here
                // rather than only its first page.
                ram.allows(register_base, 4 * 4096)?;
                first = Some(Drhd { register_base, segment, include_all: flags & 1 != 0, flags });
            }
        }

        at = at.wrapping_add(length);
    }

    let unit = first.ok_or(Absent::NoTable)?;
    Ok(Dmar { unit, host_address_width: width.saturating_add(1), structures, units })
}
