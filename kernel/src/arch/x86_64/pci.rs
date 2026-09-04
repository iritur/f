// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Configuration space, over the window `MCFG` names.
//!
//! # Why this is not a PCI subsystem
//!
//! A remapping unit's context entry is keyed by *requester id* — a bus, a
//! device and a function, sixteen bits of it — so the smallest thing that can
//! program an IOMMU is something that can say which function it is programming
//! it for. That is the whole of what this is for. There is no resource
//! allocator here, no capability walker, no interrupt routing, no driver model,
//! and adding any of them before a driver asks for them would be building the
//! part of a PC-class kernel that is easiest to write and hardest to justify.
//!
//! What this does have is enumeration, because a requester id has to come from
//! somewhere and the alternative is a constant in a source file that is right
//! on one emulator.
//!
//! # Why the memory-mapped path rather than the port pair
//!
//! `0xCF8`/`0xCFC` needs no mapping at all and reaches every function this
//! kernel will ever address. It was rejected for one reason: it reaches the
//! first 256 bytes of a function's configuration space and nothing above, which
//! is where every PCI Express capability lives — including the ones a later
//! driver needs and including address-translation services, which is the
//! feature RFC 0028's shared-virtual path would rest on. A kernel that starts
//! with the narrow window acquires code that assumes it.
//!
//! The port pair also cannot be told apart from a second agent using it: it is
//! two registers shared by the whole machine, so a correct implementation is
//! two registers plus a lock, and this kernel has no locks (RFC 0016). The
//! memory-mapped window has no such state — an address is a function, and two
//! cores reading two functions touch nothing in common.
//!
//! # Everything read here is a device's answer, not the kernel's
//!
//! A vendor id of `0xFFFF` is how the *bus* says nothing answered, and it is
//! also what a broken window reads as. Both are treated the same way — absent —
//! which is the fail-closed reading (R04): a function that cannot be identified
//! is not a function this kernel will hand a translation to.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use super::acpi::Ecam;
use super::paging::{self, AddressSpace, BuildError, Features};
use crate::mem::FrameAllocator;

/// Bytes of configuration space per function, in the memory-mapped window.
const FUNCTION_SPAN: u64 = 4096;

/// Devices on a bus.
const DEVICES: u8 = 32;

/// Functions on a device.
const FUNCTIONS: u8 = 8;

/// How many buses this kernel will walk.
///
/// Eight, and the number is a cost rather than a capability. Every bus scanned
/// is 256 pages of the device window mapped, because a function has to be
/// readable before it can be found absent — so a kernel that scanned all 256
/// buses a segment can hold would map a gibibyte of window at boot to find, on
/// this emulator, five functions.
///
/// Bus zero is always scanned; the rest come from bridges found on the buses
/// already walked. *Reversal:* a machine whose remapping unit has to translate
/// for a device more than eight bus hops from the root, at which point this
/// becomes a scan that maps a page, reads it, and unmaps it again.
const MAX_BUSES: usize = 8;

/// How many functions this kernel will remember.
///
/// Thirty-two. The emulator answers with five, six with the block device the
/// `iommu` boots add, and a large server with a few dozen;
/// a machine with more is walked in full and the ones past this are counted
/// rather than kept, which the boot log says. A silently truncated list would
/// be a device the IOMMU never hears about, and a device the IOMMU never hears
/// about is a device with no translation — which under an enabled unit is a
/// device that faults, so the failure is loud even where the list is not.
pub const MAX_FUNCTIONS: usize = 32;

/// Which function, as a remapping unit names one.
///
/// The field order is the wire order of a requester id, and
/// [`Self::source_id`] is the sixteen bits a context entry is indexed by. That
/// is the entire reason this type exists rather than three loop variables.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Bdf {
    /// Which segment group. Zero on every machine this kernel currently boots.
    pub segment: u16,
    /// Bus number.
    pub bus: u8,
    /// Device number, 0..32.
    pub device: u8,
    /// Function number, 0..8.
    pub function: u8,
}

impl Bdf {
    /// The requester id: bus, device and function packed as the hardware packs
    /// them.
    ///
    /// Unit: none — a device identifier, not a quantity. The segment is *not*
    /// in it: a requester id is unique within a segment, and which segment is a
    /// property of the remapping unit rather than of the request.
    #[must_use]
    pub const fn source_id(&self) -> u16 {
        ((self.bus as u16) << 8) | ((self.device as u16 & 0x1F) << 3) | (self.function as u16 & 0x7)
    }
}

/// One function that answered.
#[derive(Clone, Copy, Debug)]
pub struct Function {
    /// Where it is.
    pub bdf: Bdf,
    /// Who made it.
    pub vendor: u16,
    /// Which part.
    pub device: u16,
    /// Base class code.
    pub class: u8,
    /// Sub-class code.
    pub subclass: u8,
    /// Header type, with the multi-function bit already taken out.
    pub header: u8,
}

/// An open configuration-space window.
///
/// Holds no mapping of its own. The device window is linear — a physical
/// address `p` is read at [`paging::DEVICE_OFFSET`] `+ p` — so a page that has
/// been mapped once is reachable by arithmetic forever after, and this type
/// carries the arithmetic rather than a table of addresses.
#[derive(Clone, Copy, Debug)]
pub struct Space {
    ecam: Ecam,
}

/// What an enumeration found.
#[derive(Clone, Copy)]
pub struct Survey {
    /// The functions that answered, in bus order.
    functions: [Function; MAX_FUNCTIONS],
    /// How many of them are filled.
    count: usize,
    /// How many answered in total, including any past [`MAX_FUNCTIONS`].
    pub seen: u32,
    /// How many buses were walked.
    pub buses: u32,
    /// How many pages of the device window the walk had to map.
    pub pages: u32,
}

impl Survey {
    /// The functions that were kept.
    #[must_use]
    pub fn functions(&self) -> &[Function] {
        self.functions.get(..self.count).unwrap_or(&[])
    }

    /// The first function matching a vendor and device id.
    #[must_use]
    pub fn find(&self, vendor: u16, device: u16) -> Option<Function> {
        self.functions().iter().find(|f| f.vendor == vendor && f.device == device).copied()
    }
}

impl Space {
    /// Open the window `MCFG` described.
    #[must_use]
    pub const fn new(ecam: Ecam) -> Self {
        Self { ecam }
    }

    /// Which segment this window describes.
    #[must_use]
    pub const fn segment(&self) -> u16 {
        self.ecam.segment
    }

    /// Physical address of one function's configuration space.
    ///
    /// `None` for a bus this window does not describe, which is the check that
    /// keeps a bus number out of range from addressing somebody else's memory.
    fn phys(&self, bdf: Bdf) -> Option<u64> {
        if bdf.bus < self.ecam.start_bus
            || bdf.bus > self.ecam.end_bus
            || bdf.device >= DEVICES
            || bdf.function >= FUNCTIONS
        {
            return None;
        }
        // The layout the specification fixes: bus at bit 20, device at 15,
        // function at 12. Written as a sum of shifts rather than as one packed
        // expression because each term is a different field with a different
        // bound, and the bounds are checked above one at a time.
        let index = (u64::from(bdf.bus) << 20)
            | (u64::from(bdf.device) << 15)
            | (u64::from(bdf.function) << 12);
        self.ecam.base.checked_add(index)
    }

    /// Make one function's configuration space readable, and say where.
    ///
    /// # Errors
    ///
    /// [`BuildError`] from the mapping, or [`BuildError::DeviceOutOfWindow`]
    /// for a function this window does not describe — which is the same refusal
    /// the mapping would give and is given here so the caller need not
    /// distinguish two ways of being out of range.
    ///
    /// # Safety
    ///
    /// As [`paging::map_device`]: `space` must be the address space in `CR3`
    /// and `frames` must be rebound onto its direct map.
    unsafe fn open(
        &self,
        frames: &mut FrameAllocator,
        space: &AddressSpace,
        features: Features,
        bdf: Bdf,
    ) -> Result<u64, BuildError> {
        let phys = self.phys(bdf).ok_or(BuildError::DeviceOutOfWindow)?;
        // Idempotent: mapping a page that is already mapped writes the same
        // leaf entry again. That is what lets an enumeration walk a bus without
        // keeping a record of which of its pages it has already reached.
        // SAFETY: the caller's guarantee, and `phys` is inside the window
        // firmware described as configuration space — device registers rather
        // than memory, which is the other half of what `map_device` asks for.
        unsafe { paging::map_device(frames, space, phys, features) }
    }
}

/// Read one 32-bit word of a function's configuration space.
///
/// # Safety
///
/// `virt` must be the address [`Space::open`] returned for a function, and
/// `offset` must be a four-byte-aligned offset below [`FUNCTION_SPAN`].
unsafe fn read32(virt: u64, offset: u64) -> u32 {
    let at = virt.wrapping_add(offset) as *const u32;
    // SAFETY: the caller's guarantee. Volatile because this is a device: the
    // value is not the compiler's to cache, and configuration space has
    // registers that change without anything here writing them.
    unsafe { at.read_volatile() }
}

/// Write one 32-bit word of a function's configuration space.
///
/// # Safety
///
/// As [`read32`], and the value must be one the register accepts. Several
/// configuration registers are write-one-to-clear and several are read-only
/// with reserved bits that must be preserved.
unsafe fn write32(virt: u64, offset: u64, value: u32) {
    let at = virt.wrapping_add(offset) as *mut u32;
    // SAFETY: the caller's guarantee.
    unsafe { at.write_volatile(value) };
}

/// Offset of the command register, which gates whether a function may issue
/// memory reads at all.
pub const COMMAND: u64 = 0x04;

/// The function may respond to I/O-space accesses.
pub const COMMAND_IO: u16 = 1 << 0;

/// The function may respond to memory-space accesses.
pub const COMMAND_MEMORY: u16 = 1 << 1;

/// The function may initiate transactions of its own — which is to say, DMA.
///
/// The one bit in configuration space that decides whether a device is a
/// requester at all. A device with this clear cannot fault a remapping unit
/// because it cannot address memory, which is why the adversary in
/// [`super::dma`] sets it last and clears it first.
pub const COMMAND_BUS_MASTER: u16 = 1 << 2;

/// Offset of the first base-address register.
pub const BAR0: u64 = 0x10;

/// Read a function's command register.
///
/// # Safety
///
/// `virt` must be the address [`Space::open`] returned for that function.
#[must_use]
pub unsafe fn command(virt: u64) -> u16 {
    // SAFETY: the caller's guarantee; the command and status registers share
    // one aligned word and reading both is the only way to read either.
    (unsafe { read32(virt, COMMAND) } & 0xFFFF) as u16
}

/// Set bits in a function's command register, leaving the rest as they were.
///
/// Read-modify-write rather than a whole-word store, because the upper half of
/// that word is the status register: several of its bits are
/// write-one-to-clear, and storing back what was read would clear every error
/// the firmware had recorded.
///
/// # Safety
///
/// As [`command`], and `bits` must be command bits this kernel means to set.
pub unsafe fn command_set(virt: u64, bits: u16) {
    // SAFETY: the caller's guarantee.
    let word = unsafe { read32(virt, COMMAND) };
    let want = (word & 0xFFFF) as u16 | bits;
    // The status half is written back as zeroes so that no write-one-to-clear
    // bit is disturbed: a zero in those positions means *leave it alone*.
    // SAFETY: as above.
    unsafe { write32(virt, COMMAND, u32::from(want)) };
}

/// Clear bits in a function's command register.
///
/// # Safety
///
/// As [`command_set`].
pub unsafe fn command_clear(virt: u64, bits: u16) {
    // SAFETY: the caller's guarantee.
    let word = unsafe { read32(virt, COMMAND) };
    let want = (word & 0xFFFF) as u16 & !bits;
    // SAFETY: as above.
    unsafe { write32(virt, COMMAND, u32::from(want)) };
}

/// Read one base-address register, as the raw word the device answers with.
///
/// Not decoded here. A base-address register means different things in its low
/// bits depending on whether it describes memory or ports, and the decision
/// about which of those a driver wanted belongs to the driver — this kernel has
/// exactly one, in [`super::dma`], and a general decoder written for one caller
/// is a decoder with one untested branch per case it did not have.
///
/// # Safety
///
/// As [`command`], and `index` must be below six.
#[must_use]
pub unsafe fn bar(virt: u64, index: u64) -> u32 {
    // SAFETY: the caller's guarantee; `index` below six keeps the offset inside
    // the sixteen bytes of base-address registers in a type-0 header.
    unsafe { read32(virt, BAR0.wrapping_add(index.wrapping_mul(4))) }
}

/// Make one already-enumerated function's configuration space readable again.
///
/// # Errors
///
/// [`BuildError`] from the mapping.
///
/// # Safety
///
/// As [`paging::map_device`].
pub unsafe fn reopen(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    window: &Space,
    bdf: Bdf,
) -> Result<u64, BuildError> {
    // SAFETY: the caller's guarantee, passed down.
    unsafe { window.open(frames, space, features, bdf) }
}

/// Walk the machine and record what answered.
///
/// Bus zero first, then every bus a bridge on an already-walked bus points at,
/// up to [`MAX_BUSES`]. The order is deliberate and is not a breadth-first
/// search for its own sake: the buses a machine actually has are the ones
/// reachable from the root, and walking all 256 a segment could hold would map
/// two orders of magnitude more of the device window than any machine needs.
///
/// # Errors
///
/// [`BuildError`] if a page of the window cannot be mapped, which means the
/// device window is full or the allocator is empty — both fatal, and neither a
/// property of the bus.
///
/// # Safety
///
/// As [`paging::map_device`]: `space` must be the address space in `CR3` and
/// `frames` must be rebound onto its direct map.
pub unsafe fn survey(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    window: &Space,
) -> Result<Survey, BuildError> {
    let blank = Function {
        bdf: Bdf { segment: window.segment(), bus: 0, device: 0, function: 0 },
        vendor: 0,
        device: 0,
        class: 0,
        subclass: 0,
        header: 0,
    };
    let mut found =
        Survey { functions: [blank; MAX_FUNCTIONS], count: 0, seen: 0, buses: 0, pages: 0 };

    // The buses still to walk. A fixed array rather than a queue type because
    // there is no allocator to give one, and because the bound is the point:
    // a machine that describes more buses than this walks the ones that fit.
    let mut queue = [0u8; MAX_BUSES];
    let mut queued: usize = 1;
    let mut walked: usize = 0;

    while walked < queued {
        let bus = *queue.get(walked).unwrap_or(&0);
        walked = walked.saturating_add(1);
        found.buses = found.buses.saturating_add(1);

        for device in 0..DEVICES {
            let mut functions = 1u8;
            for function in 0..FUNCTIONS {
                if function >= functions {
                    break;
                }
                let bdf = Bdf { segment: window.segment(), bus, device, function };
                // SAFETY: the caller's guarantee, passed down.
                let virt = match unsafe { window.open(frames, space, features, bdf) } {
                    Ok(virt) => virt,
                    // A bus outside the window firmware described is not an
                    // error to abort the walk on: it is a bridge pointing
                    // somewhere this segment does not cover.
                    Err(BuildError::DeviceOutOfWindow) => break,
                    Err(other) => return Err(other),
                };
                found.pages = found.pages.saturating_add(1);

                // SAFETY: `virt` is the page just mapped and zero is the
                // aligned offset of the identification word.
                let identity = unsafe { read32(virt, 0) };
                let vendor = (identity & 0xFFFF) as u16;
                // How the bus says nothing answered. A window that is mapped
                // and reads as all ones says the same thing, and both are
                // treated as absent — see the module comment.
                if vendor == 0xFFFF {
                    if function == 0 {
                        break;
                    }
                    continue;
                }

                // SAFETY: as above; offset 8 holds revision, interface and the
                // two class bytes.
                let classes = unsafe { read32(virt, 8) };
                // SAFETY: as above; offset 12 holds the header type in its
                // third byte.
                let kind = ((unsafe { read32(virt, 12) } >> 16) & 0xFF) as u8;

                if function == 0 && kind & 0x80 != 0 {
                    functions = FUNCTIONS;
                }

                let entry = Function {
                    bdf,
                    vendor,
                    device: (identity >> 16) as u16,
                    subclass: ((classes >> 16) & 0xFF) as u8,
                    class: ((classes >> 24) & 0xFF) as u8,
                    header: kind & 0x7F,
                };

                found.seen = found.seen.saturating_add(1);
                if let Some(slot) = found.functions.get_mut(found.count) {
                    *slot = entry;
                    found.count = found.count.saturating_add(1);
                }

                // A bridge names the bus behind it. Queued rather than
                // descended into, so that the bound above is a bound on the
                // whole walk rather than on its depth.
                if entry.header == 1 && queued < MAX_BUSES {
                    // SAFETY: as above; offset 24 holds the primary, secondary
                    // and subordinate bus numbers.
                    let buses = unsafe { read32(virt, 24) };
                    let secondary = ((buses >> 8) & 0xFF) as u8;
                    let known = queue.get(..queued).unwrap_or(&[]).contains(&secondary);
                    if secondary != 0
                        && !known
                        && let Some(slot) = queue.get_mut(queued)
                    {
                        *slot = secondary;
                        queued = queued.saturating_add(1);
                    }
                }
            }
        }
    }

    Ok(found)
}

/// The offset in configuration space this build reads and nothing more.
///
/// Stated as a constant so that the one assumption the reads above make —
/// everything they touch is inside the first page of a function's space — is
/// checkable rather than implied.
const _: () = assert!(FUNCTION_SPAN >= 4096);
