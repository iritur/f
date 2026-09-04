// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The supervisor's half of a virtio device: find it, read its capability list,
//! and map the four register structures a driver component is routed.
//!
//! # Why this is the frame's work and not the driver's
//!
//! Because `user/virtio-blk/manifest.toml` says so, and the manifest is right:
//! it declares *four register frames*, not a bus. Configuration space is the
//! whole machine — a component that could read it could enumerate every
//! function, and one that could write it could move another device's
//! base-address register out from under its driver. Handing a driver four
//! windows rather than a bus is what makes *this driver drives this device* a
//! statement about capabilities instead of about intent.
//!
//! So the walk lives here, above the licence boundary and inside the frame, and
//! what crosses to a component is four addresses and two lengths each.
//!
//! # Why `dma.rs` walks the same list and is left alone
//!
//! It does, and this module does not call it or replace it. `dma.rs` is
//! E1-B01's provocation: sixty descriptors of hand-written virtqueue whose
//! whole value is that a reader can check it says what it does, and it is the
//! evidence a closed task's exit rests on. Refactoring it to share a walk with
//! a later task would change that evidence for the convenience of the later
//! task, which is the wrong trade in exactly the direction `claims/README.md`
//! warns about.
//!
//! The duplication is therefore deliberate and bounded: two walks of one
//! capability list, one of which is frozen. *What would merge them* is a third
//! caller — E1-B03's network driver is the obvious one — at which point `dma.rs`
//! keeps its copy and the two live callers share this one, because the reason
//! to leave `dma.rs` alone does not extend to a file nobody has finished
//! writing yet.
//!
//! # What this module refuses
//!
//! A transitional device, before anything else. A device that cannot negotiate
//! `VIRTIO_F_ACCESS_PLATFORM` addresses physical memory by specification and is
//! architecturally outside the remapping unit, so routing its registers to a
//! component would be handing out a driver with no isolation — and every test
//! against it would pass for the wrong reason. `dma.rs` records what that cost
//! to find out; this refuses it at the door.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use super::paging::{self, AddressSpace, Features};
use super::pci::{self, Bdf, Survey};
use crate::mem::{FRAME_SIZE, FrameAllocator};

/// Who makes virtio devices.
pub const VIRTIO_VENDOR: u16 = 0x1AF4;

/// The block device, as a modern-only one identifies itself.
pub const VIRTIO_BLK_MODERN: u16 = 0x1042;

/// The block device, as a transitional one does.
///
/// Recognised so that a transitional device earns the refusal *this build
/// cannot isolate it* rather than *no device found*, which is a much less
/// useful thing to read in a log.
pub const VIRTIO_BLK_TRANSITIONAL: u16 = 0x1001;

/// The network device, as a modern-only one identifies itself.
///
/// The second caller this module's comment predicted, and it arrived needing
/// two constants and no code. That is worth stating rather than assuming: the
/// walk above was written parameterised by device id on the argument that
/// E1-B03 would be the third reader of a virtio capability list, and it was —
/// so `dma.rs` keeps its frozen copy and the two live callers share this one,
/// exactly as this file said they would.
pub const VIRTIO_NET_MODERN: u16 = 0x1041;

/// The network device, as a transitional one does.
///
/// Recognised for [`VIRTIO_BLK_TRANSITIONAL`]'s reason, and the refusal matters
/// more here: a transitional network device is a bus master that writes into
/// memory whenever a packet arrives, with no request outstanding and nothing
/// timing it, and it addresses physical memory by specification. *No device
/// found* would be a much worse thing to read in a log than *this build cannot
/// isolate it*.
pub const VIRTIO_NET_TRANSITIONAL: u16 = 0x1000;

/// The display controller, which identifies itself one way and only one way.
///
/// **There is no transitional constant beside this one, and its absence is the
/// only thing the frame's device discovery owed a third driver.** The
/// transitional device ids are the sixteen numbers from `0x1000` that the
/// original specification assigned, and every device defined after the modern
/// transport arrived — the display controller among them — has a modern id and
/// nothing else. There is no legacy virtio-gpu to refuse.
///
/// [`route`] was written taking a transitional id as an ordinary argument, on
/// the assumption every virtio device has a twin to be refused. It takes an
/// [`Option`] now, and the change is one line in each of three callers. Two
/// things are worth stating about it rather than leaving it as a diff:
///
/// - It is a **widening of a refusal into a choice**, which is the direction R04
///   says to be careful in. The care is that `None` means *this device has no
///   transitional form*, which is a fact about the specification, and never
///   *do not check* — a caller that passed `None` for a device that does have
///   one would be a caller turning the legacy refusal off, and the constant
///   beside each modern id is what makes that visible in the call.
/// - It is the first change this module has needed for a second or third
///   caller. `dma.rs` keeps its frozen copy, the walk itself is untouched, and
///   what moved is a parameter. RFC 0054.
pub const VIRTIO_GPU_MODERN: u16 = 0x1050;

/// Where a function's capability list starts, in configuration space.
const CAP_POINTER: u64 = 0x34;

/// A vendor-specific capability, which is how virtio describes its windows.
const CAP_VENDOR: u8 = 0x09;

/// The common configuration structure.
const CFG_COMMON: u8 = 1;

/// The notification structure.
const CFG_NOTIFY: u8 = 2;

/// The interrupt-status register.
const CFG_ISR: u8 = 3;

/// The device's own configuration structure.
const CFG_DEVICE: u8 = 4;

/// How many entries of a capability list are walked before the list is called a
/// loop.
///
/// The list lives in memory a device controls, so a device that pointed a
/// capability at itself would otherwise be a kernel that never boots — a denial
/// of service a device should not be able to arrange. `dma.rs` bounds its own
/// walk for the same reason and at the same number.
const CAPABILITY_LIMIT: u32 = 48;

/// How many pages of one window this will map.
///
/// Sixteen, which is four times the largest structure any virtio device
/// describes. Bounded so that a device claiming a huge structure cannot make
/// this map the whole device window.
const WINDOW_PAGES: u32 = 16;

/// Why the device could not be found or could not be routed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// No such virtio device on this machine.
    NoDevice,
    /// The device is transitional, so it addresses physical memory by
    /// specification and the remapping unit cannot see it. See the module
    /// comment: this is a refusal rather than a fallback.
    Legacy,
    /// The device does not describe all four register structures, or describes
    /// one in a base-address register firmware did not assign.
    NoWindows,
    /// A device window could not be mapped.
    NoMapping,
    /// The device's capability list is not DWORD-aligned, which the
    /// specification requires and every read of it assumes.
    Unaligned,
}

impl Trouble {
    /// A sentence for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoDevice => "no virtio device of that kind on this machine",
            Self::Legacy => {
                "the device is transitional, so it addresses memory the unit cannot see"
            }
            Self::NoWindows => "the device does not describe the four register windows",
            Self::NoMapping => "a device window could not be mapped",
            Self::Unaligned => "the device's capability list is not aligned to four bytes",
        }
    }
}

/// One register structure, as a component is routed it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Structure {
    /// Where this kernel reads and writes it.
    /// Unit: bytes, a kernel virtual address in the device window.
    pub at: u64,
    /// How long the device says it is.
    /// Unit: bytes.
    pub len: u32,
}

/// Everything a driver component needs to be routed for one virtio device.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    /// Which function, which is also what a context entry is indexed by.
    pub bdf: Bdf,
    /// Its configuration space, for the bus-master bit and nothing else — a
    /// component never sees this address.
    pub config: u64,
    /// The common configuration structure.
    pub common: Structure,
    /// The notification structure.
    pub notify: Structure,
    /// The interrupt-status register.
    pub isr: Structure,
    /// The device's own configuration structure.
    pub device: Structure,
    /// How far apart two queues' doorbells are in the notification window.
    /// Unit: bytes per queue index.
    pub notify_multiplier: u32,
    /// How many pages of the device window this mapped.
    /// Unit: pages.
    pub pages: u32,
}

/// Find one virtio device and map the four structures its capability list
/// describes.
///
/// Answers everything the frame needs to route it to a component, and nothing
/// the component itself is told: the configuration-space address stays here.
///
/// # Errors
///
/// [`Trouble`], every variant of which means no component can be given this
/// device.
///
/// # Safety
///
/// `space` must be the address space in `CR3`, `frames` must be rebound onto
/// its direct map, and nothing else in this kernel may be driving the device
/// this finds.
pub unsafe fn route(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    window: &pci::Space,
    survey: &Survey,
    modern: u16,
    transitional: Option<u16>,
) -> Result<Found, Trouble> {
    // `None` is *this device has no transitional form*, which is a fact about
    // the specification and not a caller declining the check — see
    // [`VIRTIO_GPU_MODERN`], which is the device that has none and the reason
    // this parameter is an `Option` at all.
    if transitional.is_some_and(|id| survey.find(VIRTIO_VENDOR, id).is_some()) {
        return Err(Trouble::Legacy);
    }
    let found = survey.find(VIRTIO_VENDOR, modern).ok_or(Trouble::NoDevice)?;
    let bdf = found.bdf;

    // SAFETY: the caller's guarantee, and this function was already mapped once
    // by the survey — mapping it again writes the same leaf entry.
    let config = unsafe { pci::reopen(frames, space, features, window, bdf) }
        .map_err(|_| Trouble::NoMapping)?;

    // Memory space, so the register windows below answer. Bus mastering stays
    // off: turning it on is the caller's decision and it belongs after the
    // domain is programmed, because a device that could issue transactions
    // while its driver was still writing descriptors would be a race nobody can
    // lose safely.
    // SAFETY: `config` is the function's configuration space, just mapped.
    unsafe { pci::command_set(config, pci::COMMAND_MEMORY) };
    // SAFETY: as above.
    unsafe { pci::command_clear(config, pci::COMMAND_BUS_MASTER) };

    let mut common: Option<Structure> = None;
    let mut notify: Option<(Structure, u32)> = None;
    let mut isr: Option<Structure> = None;
    let mut device: Option<Structure> = None;
    let mut pages = 0;

    // SAFETY: the caller's guarantee; the capability pointer is a defined byte
    // of a type-0 header.
    let mut at = u64::from(unsafe { read8(config, CAP_POINTER) });
    let mut left = CAPABILITY_LIMIT;
    while (0x40..0x100).contains(&at) && left > 0 {
        left -= 1;
        // The alignment obligation `read32`'s `# Safety` states, discharged
        // where the value crosses the trust boundary rather than at each of the
        // three loads below. `at` is a byte a *device* chose, and every read32
        // in this loop is at `at` plus a multiple of four, so an odd pointer is
        // an unaligned volatile load through a raw pointer — undefined
        // behaviour, and the same one `f_ring::device::bounded` checks per
        // access one layer up. The specification requires these DWORD-aligned,
        // so a device that reports otherwise is describing itself illegally and
        // is refused rather than read anyway: R04, and the reason this file
        // checks it where `dma.rs` does not is that here the value decides which
        // four windows a *component* is handed.
        if !at.is_multiple_of(4) {
            return Err(Trouble::Unaligned);
        }
        // SAFETY: as above; `at` is inside the first 256 bytes of the header.
        let id = unsafe { read8(config, at) };
        // SAFETY: as above.
        let next = u64::from(unsafe { read8(config, at.wrapping_add(1)) });
        if id == CAP_VENDOR {
            // SAFETY: as above; a vendor capability describing a window is at
            // least sixteen bytes and the walk stays inside the first page.
            let kind = unsafe { read8(config, at.wrapping_add(3)) };
            // SAFETY: as above.
            let bar = unsafe { read8(config, at.wrapping_add(4)) };
            // SAFETY: as above, and `read32`'s second obligation is discharged
            // by the refusal at the top of the loop: `at` is a multiple of four
            // and so is `at + 8`.
            let offset = unsafe { read32(config, at.wrapping_add(8)) };
            // SAFETY: as above; `at + 12` is a multiple of four.
            let length = unsafe { read32(config, at.wrapping_add(12)) };

            if matches!(kind, CFG_COMMON | CFG_NOTIFY | CFG_ISR | CFG_DEVICE) {
                // SAFETY: the caller's guarantee, and `bar` came from the
                // device's own description of where it put the window.
                let (base, mapped) =
                    unsafe { map_window(frames, space, features, config, bar, offset, length) }?;
                pages += mapped;
                let structure = Structure { at: base, len: length };
                match kind {
                    CFG_COMMON => common = Some(structure),
                    CFG_NOTIFY => {
                        // SAFETY: as above; the notification capability is four
                        // bytes longer than the others and carries its
                        // multiplier in them, at `at + 16`, which is a multiple
                        // of four because `at` is.
                        let multiplier = unsafe { read32(config, at.wrapping_add(16)) };
                        notify = Some((structure, multiplier));
                    }
                    CFG_ISR => isr = Some(structure),
                    _ => device = Some(structure),
                }
            }
        }
        if next == 0 || next == at {
            break;
        }
        at = next;
    }

    let common = common.ok_or(Trouble::NoWindows)?;
    let (notify, notify_multiplier) = notify.ok_or(Trouble::NoWindows)?;
    let isr = isr.ok_or(Trouble::NoWindows)?;
    let device = device.ok_or(Trouble::NoWindows)?;

    Ok(Found { bdf, config, common, notify, isr, device, notify_multiplier, pages })
}

/// Map the pages of one base-address register that a device window falls in,
/// and answer where the window starts and how many pages it took.
///
/// # Safety
///
/// As [`route`], and `config` must be the function's mapped configuration
/// space.
unsafe fn map_window(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    config: u64,
    bar: u8,
    offset: u32,
    length: u32,
) -> Result<(u64, u32), Trouble> {
    if bar >= 6 || length == 0 {
        return Err(Trouble::NoWindows);
    }
    // SAFETY: the caller's guarantee, and `bar` is below six.
    let low = unsafe { pci::bar(config, u64::from(bar)) };
    // A memory window, never an I/O one: bit zero set means ports, and the
    // modern layout puts its structures in memory.
    if low & 1 != 0 {
        return Err(Trouble::NoWindows);
    }
    // Bits 2:1 say how wide the address is. Two means this register is the low
    // half of a sixty-four-bit pair.
    let wide = (low >> 1) & 0x3 == 0x2;
    let mut base = u64::from(low & !0xF);
    if wide {
        if bar >= 5 {
            return Err(Trouble::NoWindows);
        }
        // SAFETY: as above; the pair's upper half is the next register.
        let high = unsafe { pci::bar(config, u64::from(bar).wrapping_add(1)) };
        base |= u64::from(high) << 32;
    }
    if base == 0 {
        // Firmware assigned the device no address. Nothing to map, and mapping
        // page zero of the device window would be mapping whatever is there.
        return Err(Trouble::NoWindows);
    }

    let start = base.wrapping_add(u64::from(offset));
    let end = start.wrapping_add(u64::from(length));
    let mut page = start & !(FRAME_SIZE - 1);
    let mut left = WINDOW_PAGES;
    let mut mapped = 0;
    while page < end && left > 0 {
        left -= 1;
        // SAFETY: the caller's guarantee, and `page` is inside a window the
        // device published as its own registers.
        unsafe { paging::map_device(frames, space, page, features) }
            .map_err(|_| Trouble::NoMapping)?;
        mapped += 1;
        page = page.wrapping_add(FRAME_SIZE);
    }
    if page < end {
        return Err(Trouble::NoWindows);
    }

    Ok((paging::DEVICE_OFFSET.wrapping_add(start), mapped))
}

/// Read one byte of a device window.
///
/// # Safety
///
/// `base` must be a mapped device window and `offset` a defined byte in it.
unsafe fn read8(base: u64, offset: u64) -> u8 {
    // SAFETY: the caller's guarantee. Volatile because this is a device.
    unsafe { (base.wrapping_add(offset) as *const u8).read_volatile() }
}

/// Read four bytes of a device window.
///
/// # Safety
///
/// As [`read8`], and the register must be four bytes wide and aligned.
unsafe fn read32(base: u64, offset: u64) -> u32 {
    // SAFETY: the caller's guarantee.
    unsafe { (base.wrapping_add(offset) as *const u32).read_volatile() }
}
