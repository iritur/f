// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame's own adversary, at the other end of the bus.
//!
//! # Why this exists and why it is written like this
//!
//! `probe.rs` is sixty instructions of assembly that provoke the processor's
//! protections from ring 3, and its opening paragraph says why: a milestone
//! needed something to provoke before there was a loader, and a protection
//! nothing tries to violate is a protection nobody has checked. This is the
//! same argument for the bus. E1-B01's exit is that *a driver component
//! provably cannot address memory outside its grant, and the attempt is a fault
//! rather than a corruption* — and the drivers are E1-B02, E1-B03 and E1-B04,
//! which cannot be built until this one is finished. So the frame builds the
//! smallest possible thing that performs real DMA and points it at memory it
//! was not given.
//!
//! It is a *provocation* and not a driver. There is no interrupt, no queue of
//! outstanding requests, no error recovery, and no attempt to be correct for
//! any request but the one it makes. Every one of those absences is deliberate:
//! this has to be small enough that a reader can check it says what it does,
//! because it is the evidence for the whole task.
//!
//! # Both halves, or neither means anything
//!
//! One transfer with a descriptor pointing *outside* the domain's grant, which
//! must be refused and recorded as a fault. One with a descriptor *inside* it,
//! which must complete and must land bytes in the buffer. `mutate` already
//! makes this argument about defects and `panic_path` about endings, and it is
//! the same one: a refusal proves nothing if the same setup refuses when it
//! should not, because then what was measured is a device that was never
//! started.
//!
//! The transfer is a block *read*, so the device is the writer. That is the
//! direction that matters: a device reading memory it was not given is a leak
//! this provocation could not see from inside the machine, and a device writing
//! memory it was not given is the corruption the exit criterion names. Reading
//! the destination buffer afterwards is what separates *the transfer was
//! refused* from *the transfer happened and wrote nothing*.
//!
//! # The modern register layout, and why the legacy one could not be used
//!
//! virtio has two. The legacy one is sixteen I/O ports and a page-frame number,
//! and it was written first here because it is a quarter of the code: the queue
//! address register takes a physical page number directly, so what the
//! remapping unit translates is visible in a single register write.
//!
//! It does not work, and the reason is worth recording because it is not
//! obvious and it cost a rewrite. **A virtio device issues its transfers
//! through the platform's address translation only if the driver negotiates
//! `VIRTIO_F_ACCESS_PLATFORM`** — feature bit 33. Without it the device is
//! defined to address physical memory directly and the emulator obliges,
//! bypassing the remapping unit entirely; and the legacy interface has a
//! thirty-two-bit feature word, so it cannot name bit 33 at all. A legacy
//! virtio device is therefore *architecturally* outside the protection this
//! module demonstrates, and a provocation built on one measures nothing while
//! looking exactly like a pass — which is the failure this whole task is about,
//! arrived at from the other side.
//!
//! It is also a fact E1-B02 needs, which is why it is written here rather than
//! in a commit message: **a virtio driver in this system must negotiate that
//! bit or it has no isolation**, and a driver that fails to negotiate it should
//! be refused rather than run.
//!
//! *Reversal:* none for the feature bit, which is in the specification. The
//! *layout* choice reverses if a device this system must drive offers only the
//! legacy one — and such a device cannot be isolated, so what reverses with it
//! is whether that device is used at all.
//!
//! # The grant is made the way a component's would be
//!
//! [`grant`] does not call the remapping unit. It builds a real
//! [`Table`](crate::cap::Table), grants it a `Frame` capability for each page
//! the device is allowed, and goes through [`iommu::Grant`] — which is
//! `f_ring::registry::Domains`, the interface E1-B02, E1-B03 and E1-B04 are
//! told to build on.
//!
//! That is a deliberate cost: reaching `vtd::Unit` directly is four lines
//! shorter and the check would pass either way. It is paid because the
//! alternative leaves the interface three tasks depend on with no execution
//! path at all, and an interface no boot has ever called is a design document
//! with a type signature. What the detour buys, every boot, is the handle
//! resolution, the rights check, the capability's extent as the bound on the
//! grant, and the unwind when a later page refuses.
//!
//! It also buys the refusal. Before the device is attached, this asks for a
//! translation for a page it holds *without* `rights::GRANT` — memory a
//! component may use and may not pass on — and requires the frame to refuse it.
//! Without that, the capability check in `iommu::Grant::map` would be a branch
//! nothing in this repository ever takes.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use core::sync::atomic::{Ordering, fence};

use f_abi::cap::{CapType, rights};
use f_abi::error;
use f_ring::registry::{Domains, PageWalk};

use super::paging::{self, AddressSpace, Features};
use super::pci::{self, Bdf, Survey};
use super::vtd::{Fault, Unit};
use crate::cap::Table;
use crate::iommu;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};

/// Who makes virtio devices.
const VIRTIO_VENDOR: u16 = 0x1AF4;

/// The block device, as a modern-only one identifies itself.
const VIRTIO_BLK_MODERN: u16 = 0x1042;

/// The block device, as a transitional one does.
///
/// Recognised so that a transitional device earns the refusal *this build
/// cannot isolate it* rather than *no device found*, which is a much less
/// useful thing to read in a log. The module comment says why a device that
/// cannot negotiate feature bit 33 is not a device this provocation can use.
const VIRTIO_BLK_TRANSITIONAL: u16 = 0x1001;

/// Where a function's capability list starts, in configuration space.
const CAP_POINTER: u64 = 0x34;

/// A vendor-specific capability, which is how virtio describes its windows.
const CAP_VENDOR: u8 = 0x09;

/// The common configuration structure.
const CFG_COMMON: u8 = 1;

/// The notification structure.
const CFG_NOTIFY: u8 = 2;

/// Offsets in the common configuration structure.
mod common {
    /// Which half of the feature space the register below reads.
    pub const DEVICE_FEATURE_SELECT: u64 = 0x00;
    /// What the device offers, in the selected half.
    pub const DEVICE_FEATURE: u64 = 0x04;
    /// Which half of the feature space the register below writes.
    pub const DRIVER_FEATURE_SELECT: u64 = 0x08;
    /// What the driver accepts, in the selected half.
    pub const DRIVER_FEATURE: u64 = 0x0C;
    /// The handshake.
    pub const DEVICE_STATUS: u64 = 0x14;
    /// Which queue the registers below refer to.
    pub const QUEUE_SELECT: u64 = 0x16;
    /// How many descriptors it has. Readable *and* writable, unlike the legacy
    /// layout: a driver may shrink a queue to what it means to use.
    pub const QUEUE_SIZE: u64 = 0x18;
    /// Whether the device may take work from this queue.
    pub const QUEUE_ENABLE: u64 = 0x1C;
    /// Where in the notification window this queue's doorbell is.
    pub const QUEUE_NOTIFY_OFF: u64 = 0x1E;
    /// The descriptor table's address, in the device's address space.
    pub const QUEUE_DESC: u64 = 0x20;
    /// The available ring's address.
    pub const QUEUE_DRIVER: u64 = 0x28;
    /// The used ring's address.
    pub const QUEUE_DEVICE: u64 = 0x30;
}

/// The driver has noticed the device.
const STATUS_ACKNOWLEDGE: u8 = 1;

/// The driver knows how to drive it.
const STATUS_DRIVER: u8 = 2;

/// The driver is ready and the device may start.
const STATUS_DRIVER_OK: u8 = 4;

/// The driver has finished negotiating features.
const STATUS_FEATURES_OK: u8 = 8;

/// The device speaks the non-legacy specification. Feature bit 32, which is bit
/// zero of the upper feature word.
const FEATURE_VERSION_1: u32 = 1 << 0;

/// The device addresses memory the way the platform says to, which on this
/// machine means *through the remapping unit*. Feature bit 33.
///
/// The whole provocation rests on this bit. See the module comment: a device
/// whose driver does not negotiate it bypasses translation by specification
/// rather than by accident.
const FEATURE_ACCESS_PLATFORM: u32 = 1 << 1;

/// This descriptor is not the last of its chain.
const DESC_NEXT: u16 = 1;

/// The *device* writes this descriptor's buffer.
const DESC_WRITE: u16 = 2;

/// A block read.
const BLK_IN: u32 = 0;

/// Bytes in the block request header: type, priority, sector.
const HEADER_BYTES: u64 = 16;

/// Bytes in a sector, which is the size of the one transfer this makes.
const SECTOR_BYTES: u64 = 512;

/// How many descriptors this provocation uses.
///
/// Sixty-four, written into the device's own queue-size register rather than
/// taken from it. A modern device lets the driver shrink a queue, and shrinking
/// it is what keeps the three rings inside one eight-kibibyte block whose
/// layout is fixed below — so the queue's size stops being a property of the
/// emulator and becomes a property of this file, which is what a fixture needs.
const QUEUE_SIZE: u16 = 64;

/// Where the available ring sits inside the queue block. Unit: bytes.
///
/// Past `16 * QUEUE_SIZE`, which is 1024. The three rings have separate address
/// registers in the modern layout, so this is a layout this file chose rather
/// than one the specification imposes — and choosing round numbers means a
/// reader can check the arithmetic without a calculator. The three assertions
/// below are what keep the choice honest.
const AVAIL_AT: u64 = 2048;

/// Where the used ring sits inside the queue block. Unit: bytes.
const USED_AT: u64 = 4096;

/// How large the queue block is. Unit: bytes.
const QUEUE_BYTES: u64 = 8192;

const _: () = assert!(16 * (QUEUE_SIZE as u64) <= AVAIL_AT);
const _: () = assert!(AVAIL_AT + 6 + 2 * (QUEUE_SIZE as u64) <= USED_AT);
const _: () = assert!(USED_AT + 6 + 8 * (QUEUE_SIZE as u64) <= QUEUE_BYTES);

/// How many times the used ring is read before the transfer is called lost.
///
/// A count and not a duration, for the reason `vtd` gives at its own spin
/// bound: what is being waited for is a device, and a duration would need a
/// clock. Each turn reads a device register, which under emulation is an exit
/// to the emulator and therefore a point at which its own work can run — so
/// this is a bound on *rounds of asking*, and a spin that never asked would be
/// a spin that could not be answered.
const POLL_LIMIT: u32 = 2_000_000;

/// Why the provocation could not be set up.
///
/// None of these is the result being looked for. A provocation that could not
/// be arranged is not a provocation that was survived, and the boot path says
/// so rather than reporting a pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// No virtio block device on this machine.
    NoDevice,
    /// The device is transitional, so it cannot negotiate the feature bit that
    /// puts its transfers through the remapping unit. Not a bug here: a device
    /// that addresses physical memory by specification is a device this build
    /// cannot isolate, and saying so is more useful than failing later.
    Legacy,
    /// The device does not describe the register windows this build needs, or
    /// describes them in a base-address register firmware did not assign.
    NoWindows,
    /// The device does not offer the feature bit that routes its transfers
    /// through the platform's translation.
    NoPlatformAddressing,
    /// The device refused the features this build offered.
    FeaturesRefused,
    /// The device reported no queue zero, or one too small for a
    /// three-descriptor chain.
    NoQueue,
    /// The allocator had no block for the queue or the buffers.
    NoFrames,
    /// A device window could not be mapped.
    NoMapping,
    /// The remapping unit refused something. Its own reason, unchanged.
    Unit(super::vtd::Refuse),
    /// A capability table this provocation builds for itself would not hold the
    /// handles the grant is made of. A bug here rather than a machine property.
    Authority,
    /// The frame refused a translation for memory the provocation does hold a
    /// grantable capability for.
    Refused,
    /// `PageWalk::reaches` disagrees with the translations just made through
    /// `Domains::map`, which is the two readings of one set of tables having
    /// come apart.
    WalkDisagrees,
    /// The frame *gave* a translation for a capability carrying no right to
    /// hand memory to a device.
    ///
    /// The one variant in this list that is a failure of the thing being
    /// tested rather than of the test: a component that could put memory in a
    /// device's domain without `rights::GRANT` has an authority the capability
    /// system never issued.
    NotRefused,
    /// The device did not come out of reset.
    NotResponding,
}

impl Trouble {
    /// A sentence for the boot log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NoDevice => "no virtio block device on this machine",
            Self::Legacy => {
                "the device is transitional, so it addresses memory the unit cannot see"
            }
            Self::NoWindows => "the device does not describe the register windows this build needs",
            Self::NoPlatformAddressing => "the device does not offer platform addressing",
            Self::FeaturesRefused => "the device refused the features this build offered",
            Self::NoQueue => "the device reports no usable queue zero",
            Self::NoFrames => "no frames for the queue and its buffers",
            Self::NoMapping => "a device window could not be mapped",
            Self::Unit(e) => e.message(),
            Self::Authority => {
                "the provocation could not mint the capabilities its grant is made of"
            }
            Self::Refused => "the frame refused a translation for memory the provocation holds",
            Self::WalkDisagrees => "the domain's page walk disagrees with the grant just made",
            Self::NotRefused => {
                "the frame gave a device translation for a capability carrying no right to grant"
            }
            Self::NotResponding => "the device did not come out of reset",
        }
    }
}

/// What one provocation did.
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    /// Whether the data buffer was translated in the device's domain.
    pub inside: bool,
    /// Where the descriptor pointed. Unit: bytes, in the device's address space
    /// — which this build makes identical to the physical one.
    pub target: u64,
    /// Whether the device published a completion.
    pub completed: bool,
    /// The status byte the device wrote, or `0xFF` for a transfer that never
    /// wrote one. Zero is the device's own code for success.
    pub status: u8,
    /// Whether the sector the device was asked to read actually landed in the
    /// buffer. Read from the buffer rather than inferred from the completion,
    /// because *the transfer was refused* and *the transfer happened and wrote
    /// nothing* are different claims and only one is the exit criterion.
    pub landed: bool,
    /// The first fault the unit recorded, if it recorded one.
    pub fault: Option<Fault>,
    /// How many faults it recorded.
    pub faults: u32,
    /// Which function was provoked.
    pub bdf: Bdf,
    /// What the component-facing interface answered on the way in.
    pub checks: Checks,
}

/// The two windows a modern virtio device is driven through.
#[derive(Clone, Copy)]
struct Windows {
    /// Where the common configuration structure is mapped.
    common: u64,
    /// Where queue zero's doorbell is.
    notify: u64,
}

/// Set up one virtqueue by hand, point a descriptor at memory, and kick it.
///
/// `inside` decides the whole experiment: with it, the data buffer is
/// translated in the device's domain and the transfer must complete; without
/// it, the buffer is a page the domain does not translate and the transfer must
/// be refused and recorded.
///
/// # Errors
///
/// [`Trouble`], every variant of which means the provocation did not happen.
///
/// # Safety
///
/// `space` must be the address space in `CR3`, `frames` must be rebound onto
/// its direct map, `unit` must have translation enabled, and nothing else in
/// this kernel may be driving the device this finds. On this machine nothing
/// is: the frame has no block driver, which is why E1-B02 is a later task.
pub unsafe fn provoke(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    unit: &mut Unit,
    window: &pci::Space,
    survey: &Survey,
    inside: bool,
) -> Result<Outcome, Trouble> {
    if survey.find(VIRTIO_VENDOR, VIRTIO_BLK_TRANSITIONAL).is_some() {
        return Err(Trouble::Legacy);
    }
    let found = survey.find(VIRTIO_VENDOR, VIRTIO_BLK_MODERN).ok_or(Trouble::NoDevice)?;
    let bdf = found.bdf;

    // SAFETY: the caller's guarantee, and this function was already mapped once
    // by the survey — mapping it again writes the same leaf entry.
    let config = unsafe { pci::reopen(frames, space, features, window, bdf) }
        .map_err(|_| Trouble::NoMapping)?;

    // Memory space, so the register windows below answer. Bus mastering stays
    // off until the queue is built and the domain is programmed: a device that
    // could issue transactions while its descriptors were half-written would be
    // a race this provocation has no way to lose safely.
    // SAFETY: `config` is the function's configuration space, just mapped.
    unsafe { pci::command_set(config, pci::COMMAND_MEMORY) };
    // SAFETY: as above.
    unsafe { pci::command_clear(config, pci::COMMAND_BUS_MASTER) };

    // SAFETY: the caller's guarantee for the mapping, and `config` is the
    // function's configuration space.
    let windows = unsafe { windows(frames, space, features, config) }?;

    let order = Order::new(1).ok_or(Trouble::NoFrames)?;
    let queue = frames.alloc_zeroed(order).ok_or(Trouble::NoFrames)?;
    // The request header and the status byte share one page, and the data
    // buffer has one of its own — because the data buffer is the thing whose
    // translation is the experiment, and a page it shared with the header would
    // make the header untranslatable too. The header must be readable by the
    // device in *both* halves of the experiment, or the outside run would fail
    // at the wrong step and prove nothing about the buffer.
    let control = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;
    let data = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;

    // SAFETY: the caller's guarantee, and every frame here was just allocated
    // and is held by nobody else.
    let result = unsafe { run(frames, unit, &windows, config, bdf, queue, control, data, inside) };

    // Whatever happened, the device goes back into reset before its memory is
    // given back. This ordering is what makes the free safe rather than merely
    // tidy: a device left with a queue address pointing at a frame the
    // allocator has handed to somebody else is exactly the corruption this task
    // is about, arrived at through the teardown.
    // SAFETY: `config` is the function's configuration space.
    unsafe { pci::command_clear(config, pci::COMMAND_BUS_MASTER) };
    // SAFETY: `windows.common` is the mapped common configuration structure and
    // zero is the architectural reset.
    unsafe { write8(windows.common, common::DEVICE_STATUS, 0) };

    // SAFETY: every one of these was allocated above, at the order it is freed
    // at, and the device has been reset and stripped of bus mastering.
    unsafe { frames.free(queue) };
    // SAFETY: as above.
    unsafe { frames.free(control) };
    // SAFETY: as above.
    unsafe { frames.free(data) };

    result
}

/// Walk the function's capability list and map the two windows it names.
///
/// # Safety
///
/// `config` must be the function's mapped configuration space, and the caller's
/// guarantees for [`paging::map_device`] must hold.
unsafe fn windows(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    config: u64,
) -> Result<Windows, Trouble> {
    let mut common_at: Option<u64> = None;
    let mut notify_at: Option<(u64, u32)> = None;

    // SAFETY: the caller's guarantee; the capability pointer is a defined byte
    // of a type-0 header.
    let mut at = u64::from(unsafe { read8(config, CAP_POINTER) });
    // A bound on the walk rather than a trust in the list terminating. The list
    // lives in memory a device controls, and a device that pointed a capability
    // at itself would otherwise be a kernel that never boots — a denial of
    // service a device should not be able to arrange.
    let mut left = 48;
    while (0x40..0x100).contains(&at) && left > 0 {
        left -= 1;
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
            // SAFETY: as above.
            let offset = unsafe { read32(config, at.wrapping_add(8)) };
            // SAFETY: as above.
            let length = unsafe { read32(config, at.wrapping_add(12)) };

            if kind == CFG_COMMON || kind == CFG_NOTIFY {
                // SAFETY: the caller's guarantee, and `bar` came from the
                // device's own description of where it put the window.
                let base =
                    unsafe { window_at(frames, space, features, config, bar, offset, length) }?;
                if kind == CFG_COMMON {
                    common_at = Some(base);
                } else {
                    // SAFETY: as above; the notification capability is four
                    // bytes longer than the others and carries its multiplier
                    // in them.
                    let multiplier = unsafe { read32(config, at.wrapping_add(16)) };
                    notify_at = Some((base, multiplier));
                }
            }
        }
        if next == 0 || next == at {
            break;
        }
        at = next;
    }

    let common = common_at.ok_or(Trouble::NoWindows)?;
    let (notify_base, multiplier) = notify_at.ok_or(Trouble::NoWindows)?;

    // Which doorbell queue zero's kick goes to. Read here rather than after the
    // queue is enabled, because a doorbell read after the fact is a doorbell
    // that may have moved.
    // SAFETY: `common` is the mapped common configuration structure.
    unsafe { write16(common, common::QUEUE_SELECT, 0) };
    // SAFETY: as above.
    let queue_notify_off = unsafe { read16(common, common::QUEUE_NOTIFY_OFF) };
    let notify =
        notify_base.wrapping_add(u64::from(queue_notify_off).wrapping_mul(u64::from(multiplier)));

    Ok(Windows { common, notify })
}

/// Map the pages of one base-address register that a device window falls in.
///
/// # Safety
///
/// As [`windows`].
unsafe fn window_at(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    config: u64,
    bar: u8,
    offset: u32,
    length: u32,
) -> Result<u64, Trouble> {
    if bar >= 6 {
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
    // Bounded, so that a device claiming a huge structure cannot make this map
    // the whole device window. Sixteen pages is four times the largest
    // structure any virtio device describes.
    let mut left = 16;
    while page < end && left > 0 {
        left -= 1;
        // SAFETY: the caller's guarantee, and `page` is inside a window the
        // device published as its own registers.
        unsafe { paging::map_device(frames, space, page, features) }
            .map_err(|_| Trouble::NoMapping)?;
        page = page.wrapping_add(FRAME_SIZE);
    }
    if page < end {
        return Err(Trouble::NoWindows);
    }

    Ok(paging::DEVICE_OFFSET.wrapping_add(start))
}

/// The provocation proper, with every allocation already made.
///
/// Split out so that the teardown in [`provoke`] runs on every path, including
/// the ones that refuse. A refusal that leaked the queue would leak it on the
/// boot where the interesting thing went wrong.
///
/// # Safety
///
/// As [`provoke`], and every frame must be one the caller allocated for this.
#[expect(
    clippy::too_many_arguments,
    reason = "every one is a thing the caller allocated or read; bundling them into a struct \
              would be a type that exists so that a lint passes"
)]
unsafe fn run(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    windows: &Windows,
    config: u64,
    bdf: Bdf,
    queue: Frame,
    control: Frame,
    data: Frame,
    inside: bool,
) -> Result<Outcome, Trouble> {
    let common = windows.common;

    // Reset first and unconditionally: firmware may have left the device
    // part-way through somebody else's initialisation, and a status register
    // written on top of that is a device in a state nothing describes.
    // SAFETY: `common` is the device's mapped common configuration structure,
    // and zero is the architectural reset.
    unsafe { write8(common, common::DEVICE_STATUS, 0) };
    // SAFETY: as above; reading the status register has no side effect.
    if unsafe { read8(common, common::DEVICE_STATUS) } != 0 {
        return Err(Trouble::NotResponding);
    }
    // SAFETY: as above.
    unsafe { write8(common, common::DEVICE_STATUS, STATUS_ACKNOWLEDGE) };
    // SAFETY: as above.
    unsafe { write8(common, common::DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER) };

    // The upper half of the feature space, which is where the two bits this
    // provocation cannot do without live.
    // SAFETY: as above.
    unsafe { write32(common, common::DEVICE_FEATURE_SELECT, 1) };
    // SAFETY: as above.
    let offered = unsafe { read32(common, common::DEVICE_FEATURE) };
    let wanted = FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM;
    if offered & wanted != wanted {
        return Err(Trouble::NoPlatformAddressing);
    }

    // Nothing from the lower half. Every feature down there is an optimisation
    // or a second layout, and a provocation that negotiated one would be a
    // provocation whose behaviour depends on what the emulator was built with.
    // SAFETY: as above.
    unsafe { write32(common, common::DRIVER_FEATURE_SELECT, 0) };
    // SAFETY: as above.
    unsafe { write32(common, common::DRIVER_FEATURE, 0) };
    // SAFETY: as above.
    unsafe { write32(common, common::DRIVER_FEATURE_SELECT, 1) };
    // SAFETY: as above.
    unsafe { write32(common, common::DRIVER_FEATURE, wanted) };
    // SAFETY: as above.
    unsafe {
        write8(
            common,
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
    }
    // Read back, because this is the one point in the handshake where the
    // device has a veto: a device that clears this bit has refused the set
    // offered, and a driver that carried on would be driving it under an
    // agreement only one side made.
    // SAFETY: as above.
    if unsafe { read8(common, common::DEVICE_STATUS) } & STATUS_FEATURES_OK == 0 {
        return Err(Trouble::FeaturesRefused);
    }

    // SAFETY: as above.
    unsafe { write16(common, common::QUEUE_SELECT, 0) };
    // SAFETY: as above.
    let offered_size = unsafe { read16(common, common::QUEUE_SIZE) };
    if offered_size < 3 {
        return Err(Trouble::NoQueue);
    }
    let size = offered_size.min(QUEUE_SIZE);
    // SAFETY: as above; the size written is never larger than the size read,
    // which is the only constraint the specification places on it.
    unsafe { write16(common, common::QUEUE_SIZE, size) };

    // A domain of the component's own. One per component, whatever else it is:
    // `vtd`'s module comment argues why that is not the same question RFC 0005
    // answers.
    // SAFETY: the caller's guarantee that frames are addressable.
    let mut domain = unsafe { unit.domain(frames) }.map_err(Trouble::Unit)?;

    // Everything between here and the kick that can refuse is behind one call,
    // so that a refusal gives the domain's tables back rather than leaking
    // them. A leak on the path where something else went wrong is a leak
    // nobody ever sees, because the boot that took that path fails for the
    // other reason and the free count is never compared.
    // SAFETY: the caller's guarantee, passed down.
    let asked = unsafe { grant(frames, unit, &mut domain, bdf, queue, control, data, inside) };
    let checks = match asked {
        Ok(checks) => checks,
        Err(why) => {
            // Detach before release, and unconditionally: `grant` attaches
            // last, so on most paths there is nothing to detach and this
            // refuses harmlessly — but on the one where the attach itself
            // refused, a context entry may already be present over tables about
            // to be freed.
            // SAFETY: as above.
            let _ = unsafe { unit.detach(frames, bdf) };
            // SAFETY: nothing is attached and no device is walking these
            // tables.
            unsafe { unit.release(frames, domain) };
            return Err(why);
        }
    };

    // --- the queue, written by hand ----------------------------------------

    let base = frames.virt(queue);
    let header = frames.virt(control);
    let status_at = header.wrapping_add(HEADER_BYTES as usize);
    let payload = frames.virt(data);

    // The request: read sector zero. Nothing about the sector matters; what
    // matters is that a real device really performs a transfer. One statement
    // per field rather than one block for the three, because the frame's rule
    // is one unsafe operation per block and three writes into one frame is
    // three operations however closely they are related.
    // SAFETY: `header` is the start of a zeroed frame this provocation owns and
    // the request type is its first four bytes.
    unsafe { header.cast::<u32>().write_volatile(BLK_IN) };
    // SAFETY: as above; the priority is the next four.
    unsafe { header.wrapping_add(4).cast::<u32>().write_volatile(0) };
    // SAFETY: as above; the sector number is the eight after that, and the
    // sixteen together are the whole of a block request header.
    unsafe { header.wrapping_add(8).cast::<u64>().write_volatile(0) };
    // A byte the device is expected to overwrite. `0xFF` is not a status any
    // device defines, so a status of `0xFF` afterwards means *nothing was
    // written here* rather than *the device reported something*.
    // SAFETY: as above, inside the same frame.
    unsafe { status_at.write_volatile(0xFF) };
    // The same trick on the data page, and it is the measurement the whole
    // provocation turns on: the block behind this device answers zeroes, so a
    // buffer still holding this pattern is a buffer nothing wrote.
    // SAFETY: `payload` is the start of a zeroed frame this provocation owns.
    unsafe { core::ptr::write_bytes(payload, 0xA5, SECTOR_BYTES as usize) };

    // Three descriptors: the header, which the device reads; the sector, which
    // it writes; and the status byte, which it also writes.
    // SAFETY: `base` is the start of a zeroed eight-kibibyte block and the index
    // is below `size`, which was checked to be at least three.
    unsafe { descriptor(base, 0, control.addr(), HEADER_BYTES as u32, DESC_NEXT, 1) };
    // SAFETY: as above.
    unsafe { descriptor(base, 1, data.addr(), SECTOR_BYTES as u32, DESC_NEXT | DESC_WRITE, 2) };
    // SAFETY: as above.
    unsafe { descriptor(base, 2, control.addr().saturating_add(HEADER_BYTES), 1, DESC_WRITE, 0) };

    let avail = base.wrapping_add(AVAIL_AT as usize);
    let used = base.wrapping_add(USED_AT as usize);

    // The head of the chain into the available ring, then the index that
    // publishes it. Exactly the discipline the ring rests on: the entry is
    // written first, and the cursor that makes it visible is written after a
    // release fence. A device has a weaker relationship to this core's store
    // buffer than another core does, and the fence is what makes the
    // descriptors it reads the ones that were written.
    // SAFETY: `avail` is inside the queue block; slot zero of its ring is at
    // offset four, after the flags and the index.
    unsafe { avail.wrapping_add(4).cast::<u16>().write_volatile(0) };
    fence(Ordering::Release);
    // SAFETY: as above; the index is at offset two.
    unsafe { avail.wrapping_add(2).cast::<u16>().write_volatile(1) };
    fence(Ordering::Release);

    // The three ring addresses, in the device's address space — which this
    // build makes the identity of the physical one. `crate::iommu` is where
    // that decision is argued, and these are the three registers where it is
    // visible.
    // SAFETY: `common` is the mapped common configuration structure.
    unsafe { write64(common, common::QUEUE_DESC, queue.addr()) };
    // SAFETY: as above.
    unsafe { write64(common, common::QUEUE_DRIVER, queue.addr().wrapping_add(AVAIL_AT)) };
    // SAFETY: as above.
    unsafe { write64(common, common::QUEUE_DEVICE, queue.addr().wrapping_add(USED_AT)) };

    // Now, and not before: the device may issue transactions. Everything it
    // could reach is either translated or deliberately not.
    // SAFETY: `config` is the function's configuration space.
    unsafe { pci::command_set(config, pci::COMMAND_BUS_MASTER) };

    // SAFETY: `common` is the mapped common configuration structure.
    unsafe { write16(common, common::QUEUE_ENABLE, 1) };
    // SAFETY: as above.
    unsafe {
        write8(
            common,
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );
    }

    // The kick.
    // SAFETY: `windows.notify` is queue zero's doorbell, computed from the
    // device's own notification capability and its queue-notify offset.
    unsafe { (windows.notify as *mut u16).write_volatile(0) };

    // Wait for a completion, and give up rather than hang. Each turn reads a
    // device register, which under emulation is an exit to the emulator and
    // therefore a point at which the device's own work can make progress — a
    // loop that only read memory would be a loop the emulator never got a
    // chance to answer.
    let mut left = POLL_LIMIT;
    let mut completed = false;
    while left > 0 {
        // SAFETY: `used` is inside the queue block and the index is at offset
        // two, after the flags.
        let index = unsafe { used.wrapping_add(2).cast::<u16>().read_volatile() };
        if index != 0 {
            completed = true;
            break;
        }
        // SAFETY: `common` is the mapped common configuration structure and
        // reading the status register has no side effect.
        let _ = unsafe { read8(common, common::DEVICE_STATUS) };
        left = left.saturating_sub(1);
        core::hint::spin_loop();
    }
    fence(Ordering::Acquire);

    // SAFETY: the byte the third descriptor named, inside a frame this
    // provocation owns.
    let status = unsafe { status_at.read_volatile() };
    // Did anything actually arrive? The block behind this device answers
    // zeroes, and the buffer was filled with a pattern it cannot produce — so a
    // byte still holding the pattern is a byte nothing wrote.
    // SAFETY: as above, inside the data frame.
    let landed = unsafe { payload.read_volatile() } != 0xA5;

    let faults = unit.faults();

    // Bus mastering is cleared here rather than only in the caller, because the
    // domain's tables are freed a few lines below and a device still able to
    // issue transactions would be walking them.
    // SAFETY: `config` is the function's configuration space.
    unsafe { pci::command_clear(config, pci::COMMAND_BUS_MASTER) };
    // And the context entry goes too, before the tables under it. Clearing the
    // bus master bit is the device agreeing not to address memory; detaching is
    // the unit no longer having anywhere to send it if it did.
    // SAFETY: the caller's guarantee, and `bdf` is the function `attach` was
    // called for a few dozen lines above.
    let _ = unsafe { unit.detach(frames, bdf) };
    // SAFETY: the device is detached and stripped of bus mastering, and nothing
    // else was ever attached to this domain.
    unsafe { unit.release(frames, domain) };

    Ok(Outcome {
        inside,
        target: data.addr(),
        completed,
        status,
        landed,
        fault: faults.first,
        faults: faults.records,
        bdf,
        checks,
    })
}

/// What the component-facing interface answered while the grant was made.
///
/// Carried out of [`grant`] and printed, because both of these are checks and
/// neither is visible in the transfer itself.
#[derive(Clone, Copy, Debug)]
pub struct Checks {
    /// Whether a capability carrying no [`rights::GRANT`] was refused a device
    /// translation.
    pub refused_without_grant: bool,
    /// Whether [`iommu::Grant`]'s page walk says the domain reaches the data
    /// page. Must agree with which half of the experiment is running, and the
    /// caller requires it to.
    pub reaches_data: bool,
}

/// The rights a component holds over memory it means to hand to a device.
///
/// `GRANT` is the load-bearing one and [`iommu::Grant::map`] argues why:
/// putting a page in a device's domain is a transfer to something the
/// capability system does not mediate. `WRITE` is here because the transfer is
/// a block read and the device is the writer; a set without it would produce a
/// read-only translation and the *inside* half would fault, which is a way this
/// provocation could pass for the wrong reason in the other direction.
const GRANTABLE: u8 = rights::READ | rights::WRITE | rights::GRANT;

/// Put the grant into the domain and give the device to it.
///
/// Everything that can refuse, in one place, so that the one caller has one
/// error path to undo rather than four. The attach is last on purpose: until it
/// runs the device is in no domain at all, so a refusal before it leaves
/// nothing to detach.
///
/// The translations are asked for through [`iommu::Grant`] rather than of the
/// unit directly — see the module comment for what that costs and what it buys.
///
/// # Errors
///
/// [`Trouble::Unit`] carrying the remapping unit's own reason,
/// [`Trouble::Authority`] for a table this function could not fill,
/// [`Trouble::Refused`] where the frame refused memory the provocation holds
/// grantably, and [`Trouble::NotRefused`] where it did not refuse memory the
/// provocation holds without the right to grant it.
///
/// # Safety
///
/// As [`provoke`], and every frame must be one the caller allocated for this.
#[expect(
    clippy::too_many_arguments,
    reason = "the same list `run` holds; splitting it into a struct would be a type that \
              exists so that a lint passes"
)]
unsafe fn grant(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    domain: &mut super::vtd::Domain,
    bdf: Bdf,
    queue: Frame,
    control: Frame,
    data: Frame,
    inside: bool,
) -> Result<Checks, Trouble> {
    // The capability table a component would hold. Built here rather than taken
    // from `cap::mine()` because the running process's table is not this
    // provocation's, and borrowing it would make the check depend on what some
    // other stage of the boot happened to leave in it.
    let mut table = Table::EMPTY;
    let mut hand = |frame: Frame, held: u8| -> Result<u32, Trouble> {
        table
            .grant(CapType::Frame, held, frame.addr(), frame.bytes())
            .map(|handle| handle.bits())
            .map_err(|_| Trouble::Authority)
    };

    // Everything the device legitimately holds: its queue, and the page
    // carrying the request header and the status byte it must write.
    let queue_cap = hand(queue, GRANTABLE)?;
    let control_cap = hand(control, GRANTABLE)?;
    // The data page twice, and the pair is the point. One handle carries the
    // right to hand it to a device and one does not; they name the same bytes,
    // so what separates them is authority alone.
    let data_cap = hand(data, GRANTABLE)?;
    let ungrantable = hand(data, rights::READ | rights::WRITE)?;

    let checks = {
        // Reborrowed rather than moved: `unit`, `domain` and `frames` are
        // needed again below, and a `Grant` that owned them for the rest of
        // this function would be the long-lived object `iommu.rs` argues
        // against — one more owner of the single frame allocator.
        let mut asking = iommu::Grant {
            unit: &mut *unit,
            domain: &mut *domain,
            frames: &mut *frames,
            table: &table,
        };

        for (cap, bytes) in [(queue_cap, queue.bytes()), (control_cap, control.bytes())] {
            let len = u32::try_from(bytes).map_err(|_| Trouble::Authority)?;
            asking.map(cap, len).map_err(|_| Trouble::Refused)?;
        }

        // The refusal, in both halves of the experiment, before the device can
        // do anything at all. A component holding this capability may read and
        // write the page itself and may not pass it on, and a device
        // translation is passing it on.
        let page = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Authority)?;
        match asking.map(ungrantable, page) {
            Ok(_) => return Err(Trouble::NotRefused),
            // The domain *and* the code the refusal names are checked, not
            // only that there was one: a `RESOURCE` refusal here would mean the
            // table ran out of room and the rights were never consulted, which
            // would pass this check while testing nothing.
            Err((packed, _))
                if error::unpack(packed)
                    == Some((error::AUTHORITY, error::authority::RIGHT_NOT_HELD)) => {}
            Err(_) => return Err(Trouble::NotRefused),
        }

        // The whole experiment, in one branch. With it the descriptor points at
        // a page the domain translates; without it, at a page it does not.
        if inside {
            asking.map(data_cap, page).map_err(|_| Trouble::Refused)?;
        }

        Checks {
            refused_without_grant: true,
            // Asked of the domain's own second-level tables, which is
            // `PageWalk` answering the question a service asks on the
            // shared-virtual path. It must agree with `inside`, and the caller
            // fails the boot if it does not — a walk that said *yes* to an
            // unmapped page would be the one bug in `iommu.rs` that this
            // provocation could otherwise not see, because the device's fault
            // and the walk's answer come from the same tables read two ways.
            reaches_data: asking.reaches(data.addr(), page),
        }
    };

    if checks.reaches_data != inside {
        return Err(Trouble::WalkDisagrees);
    }

    // SAFETY: the caller's guarantee, and `bdf` is the function whose registers
    // the caller is programming.
    unsafe { unit.attach(frames, bdf, domain) }.map_err(Trouble::Unit)?;
    Ok(checks)
}

/// Write one descriptor of a split queue.
///
/// # Safety
///
/// `base` must be the start of a queue block of at least `16 * (index + 1)`
/// bytes that this kernel owns.
unsafe fn descriptor(base: *mut u8, index: u64, addr: u64, len: u32, flags: u16, next: u16) {
    let at = base.wrapping_add((index.saturating_mul(16)) as usize);
    // The four fields are the sixteen bytes a descriptor is, in the order the
    // specification fixes, and each is written volatilely because the reader is
    // a device rather than this program.
    // SAFETY: the caller's guarantee; the address is the first eight bytes.
    unsafe { at.cast::<u64>().write_volatile(addr) };
    // SAFETY: as above; the length is the four after it.
    unsafe { at.wrapping_add(8).cast::<u32>().write_volatile(len) };
    // SAFETY: as above; the flags are the two after that.
    unsafe { at.wrapping_add(12).cast::<u16>().write_volatile(flags) };
    // SAFETY: as above; the link to the next descriptor is the last two.
    unsafe { at.wrapping_add(14).cast::<u16>().write_volatile(next) };
}

/// Read one byte of a device window.
///
/// # Safety
///
/// `base` must be a mapped device window and `offset` a defined register in it.
unsafe fn read8(base: u64, offset: u64) -> u8 {
    // SAFETY: the caller's guarantee. Volatile because this is a device.
    unsafe { (base.wrapping_add(offset) as *const u8).read_volatile() }
}

/// Read two bytes of a device window.
///
/// # Safety
///
/// As [`read8`], and the register must be two bytes wide and aligned.
unsafe fn read16(base: u64, offset: u64) -> u16 {
    // SAFETY: the caller's guarantee.
    unsafe { (base.wrapping_add(offset) as *const u16).read_volatile() }
}

/// Read four bytes of a device window.
///
/// # Safety
///
/// As [`read8`], at four bytes.
unsafe fn read32(base: u64, offset: u64) -> u32 {
    // SAFETY: the caller's guarantee.
    unsafe { (base.wrapping_add(offset) as *const u32).read_volatile() }
}

/// Write one byte of a device window.
///
/// # Safety
///
/// As [`read8`], and the value must be one the register accepts.
unsafe fn write8(base: u64, offset: u64, value: u8) {
    // SAFETY: the caller's guarantee.
    unsafe { (base.wrapping_add(offset) as *mut u8).write_volatile(value) };
}

/// Write two bytes of a device window.
///
/// # Safety
///
/// As [`write8`], at two bytes.
unsafe fn write16(base: u64, offset: u64, value: u16) {
    // SAFETY: the caller's guarantee.
    unsafe { (base.wrapping_add(offset) as *mut u16).write_volatile(value) };
}

/// Write four bytes of a device window.
///
/// # Safety
///
/// As [`write8`], at four bytes.
unsafe fn write32(base: u64, offset: u64, value: u32) {
    // SAFETY: the caller's guarantee.
    unsafe { (base.wrapping_add(offset) as *mut u32).write_volatile(value) };
}

/// Write eight bytes of a device window, as two four-byte halves, low first.
///
/// The specification permits a driver to write a sixty-four-bit field as two
/// words and fixes this order, and doing it that way rather than as one
/// eight-byte store is what keeps this correct on a device whose window is
/// implemented in thirty-two-bit registers — which is most of them.
///
/// # Safety
///
/// As [`write32`], and `offset` must name an eight-byte field.
unsafe fn write64(base: u64, offset: u64, value: u64) {
    // SAFETY: the caller's guarantee.
    unsafe { write32(base, offset, value as u32) };
    // SAFETY: as above; the upper half is the next word.
    unsafe { write32(base, offset.wrapping_add(4), (value >> 32) as u32) };
}
