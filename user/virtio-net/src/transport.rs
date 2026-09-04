// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The modern virtio PCI transport, driven through four granted register
//! windows and no `unsafe` — for a device with two queues rather than one.
//!
//! # Why the driver does not walk configuration space
//!
//! Because `user/virtio-net/manifest.toml` says it does not. The manifest
//! declares four register frames, not a bus, so finding the device, reading its
//! capability list and mapping those four structures is the supervisor's work,
//! and what arrives here is four [`Window`]s and a notification multiplier.
//!
//! That is a boundary and not a convenience. Configuration space is the whole
//! bus: a component that could read it could enumerate every function on the
//! machine, and one that could *write* it could move another device's
//! base-address register. Handing a driver four windows rather than a bus is
//! what makes *this driver drives this device* a statement about capabilities.
//!
//! # The feature bit the whole isolation story rests on
//!
//! `VIRTIO_F_ACCESS_PLATFORM`, bit 33. Without it a virtio device is defined to
//! address physical memory directly and the emulator obliges, bypassing the
//! remapping unit entirely — so every isolation test passes for the wrong
//! reason, which is what E1-B01 found after building its first provocation on
//! the legacy interface. [`Transport::open`] **refuses** a device that does not
//! offer the bit: a driver in this system that fails to negotiate it has no
//! isolation, and R04 says the answer to that is a refusal.
//!
//! It matters more here than it did for the block driver, and the reason is the
//! direction this task is about. A block driver's transfers are almost all
//! *reads of memory the device was pointed at*; a network device's receive path
//! is the device **writing** into memory on its own schedule, with no request
//! outstanding and nothing this side timed. A device outside the remapping unit
//! doing that is not a test that passes for the wrong reason, it is a device
//! writing wherever a driver's arithmetic said, whenever a peer sends a packet.
//!
//! The legacy transport cannot express bit 33 at all — its feature word is
//! thirty-two bits — so there is no legacy path here and there will not be one.
//!
//! # What this driver deliberately does not negotiate, and what that costs
//!
//! Nothing from the lower feature word: not `VIRTIO_NET_F_MAC`, not
//! `VIRTIO_NET_F_MRG_RXBUF`, not `VIRTIO_NET_F_CTRL_VQ`, and none of the
//! checksum or segmentation offloads. The same choice `user/virtio-blk` made and
//! for the same reason — every feature down there is an optimisation or a second
//! layout, and a driver that negotiated one would be a driver whose behaviour
//! depends on what the emulator was built with, which is a thing a fixture must
//! not depend on.
//!
//! The costs, stated rather than discovered:
//!
//! - **No `MAC`.** The device's own address is not read out of the
//!   configuration structure, so this driver has no interface address of its
//!   own and whoever forms a frame chooses one. That is not a workaround: on
//!   this boot the *client* forms the frame, so the address is the client's,
//!   which is where an address belongs in a system with no network stack.
//! - **No `MRG_RXBUF`.** One received frame occupies one buffer. A frame larger
//!   than a posted buffer is truncated by the device rather than continued into
//!   a second, so the buffer has to be at least the link's maximum frame — which
//!   [`crate::driver::FRAME_MAX`] states and refuses against.
//! - **No `CTRL_VQ`.** No unicast filter, no promiscuous switch, no VLAN table,
//!   no multiqueue steering, no link-change announcement. The device therefore
//!   filters nothing, and every frame on the link reaches this driver's receive
//!   queue. That is a *widening* rather than a narrowing and it is the honest
//!   reading of `domain = "private"` in the manifest.
//! - **No checksum or segmentation offload.** Every frame is sent as it is,
//!   which is what `sim/src/net.rs` models — `GSO_NONE`, all six header fields
//!   zero, `num_buffers` one — so the model and this file agree by construction
//!   rather than by inspection.
//!
//! Even with `VIRTIO_NET_F_MRG_RXBUF` absent, the header is the twelve-byte
//! layout with `num_buffers` in it, because `VIRTIO_F_VERSION_1` is negotiated
//! and the specification fixes the modern header at twelve bytes regardless.
//! That is the same fact `sim/src/net.rs` records from the model's side, and
//! getting it wrong in either direction shifts every frame by two bytes.

use f_ring::device::Window;

use crate::Trouble;

/// Offsets in the common configuration structure.
///
/// The specification's, not this file's: every one of them is fixed by the
/// virtio standard, which is why they are constants rather than fields read out
/// of anything. They are the same constants `user/virtio-blk/src/transport.rs`
/// holds, and they are written again rather than shared for the reason
/// `crate::queue`'s module comment gives: what the two drivers share is much
/// larger than a table of offsets, and RFC 0051 says what to do about that when
/// there are three.
mod common {
    /// Which half of the feature space the register below reads.
    pub const DEVICE_FEATURE_SELECT: u32 = 0x00;
    /// What the device offers, in the selected half.
    pub const DEVICE_FEATURE: u32 = 0x04;
    /// Which half of the feature space the register below writes.
    pub const DRIVER_FEATURE_SELECT: u32 = 0x08;
    /// What the driver accepts, in the selected half.
    pub const DRIVER_FEATURE: u32 = 0x0C;
    /// The handshake.
    pub const DEVICE_STATUS: u32 = 0x14;
    /// How many queues the device has. Read and never written: a driver that
    /// asked for queue one on a device with one queue would be programming
    /// registers the device does not define.
    pub const NUM_QUEUES: u32 = 0x12;
    /// Which queue the registers below refer to.
    pub const QUEUE_SELECT: u32 = 0x16;
    /// How many descriptors it has. Readable *and* writable in this layout: a
    /// driver may shrink a queue to what it means to use.
    pub const QUEUE_SIZE: u32 = 0x18;
    /// Whether the device may take work from this queue.
    pub const QUEUE_ENABLE: u32 = 0x1C;
    /// Where in the notification window this queue's doorbell is.
    pub const QUEUE_NOTIFY_OFF: u32 = 0x1E;
    /// The descriptor table's address, in the device's address space.
    pub const QUEUE_DESC: u32 = 0x20;
    /// The available ring's address.
    pub const QUEUE_DRIVER: u32 = 0x28;
    /// The used ring's address.
    pub const QUEUE_DEVICE: u32 = 0x30;
    /// How much of the structure this driver reads.
    /// Unit: bytes.
    pub const EXTENT: u32 = 0x38;
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

/// The device addresses memory the way the platform says to, which on a machine
/// with a remapping unit means *through it*. Feature bit 33.
///
/// The module comment is why this is not optional.
const FEATURE_ACCESS_PLATFORM: u32 = 1 << 1;

/// Bytes in the header every frame carries, on a device that negotiated
/// `VIRTIO_F_VERSION_1`.
///
/// Twelve: flags, segmentation type, header length, segment size, two checksum
/// offsets, and `num_buffers`. Twelve whether or not `VIRTIO_NET_F_MRG_RXBUF`
/// was negotiated, which is the fact a legacy driver gets wrong by two bytes and
/// which `sim/src/net.rs` records from the model's side.
/// Unit: bytes.
pub const HEADER_BYTES: u32 = 12;

/// The queues this driver requires the device to have. Unit: queues.
///
/// Two: one receive and one transmit. A device with fewer is not a virtio-net
/// device this driver can drive, and a device with more has queue pairs and a
/// control queue this driver does not negotiate — which is not a reason to
/// refuse it, only a reason not to touch them.
const QUEUES_NEEDED: u16 = 2;

/// The four windows the supervisor routes, and the one number that is not a
/// window.
///
/// A struct rather than four arguments because three of the four are the same
/// type: a call site that passed `isr` where `notify` goes would compile, and
/// the failure would be a doorbell written into an interrupt-status register.
#[derive(Clone, Copy, Debug)]
pub struct Windows {
    /// The common configuration structure.
    pub common: Window,
    /// The notification structure — every queue's doorbell.
    pub notify: Window,
    /// The interrupt-status register.
    pub isr: Window,
    /// The device's own configuration structure. Read by nothing in this
    /// driver, because `VIRTIO_NET_F_MAC` is not negotiated and there is
    /// nothing else in a virtio-net configuration structure this driver acts
    /// on — routed anyway, because the manifest declares four register frames
    /// and a driver that quietly used three would be a manifest describing a
    /// component nobody had checked against it.
    pub config: Window,
    /// How far apart two queues' doorbells are in the notification window.
    /// Unit: bytes per queue index, from the device's own notification
    /// capability. Zero is legal and means every queue shares one doorbell.
    pub notify_multiplier: u32,
}

/// One virtio device, after the handshake.
///
/// Holds no queue: [`crate::queue::Queue`] is separate, because the queue lives
/// in memory the component was granted and the transport lives in registers it
/// was granted, and the two are different kinds of authority. Keeping them apart
/// is what lets the queue be tested on a host with no device.
///
/// What it *does* hold that a one-queue driver's does not is a doorbell per
/// queue. The notification offset is a per-queue number the device publishes and
/// the multiplier is a second one; a driver with two queues that rang one
/// doorbell for both would transmit and never receive, or receive and never
/// transmit, depending on which offset it kept.
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    common: Window,
    notify: Window,
    isr: Window,
    /// Where each queue's doorbell is, inside [`Transport::notify`], indexed by
    /// queue. Unit: bytes.
    doorbells: [u32; QUEUES_NEEDED as usize],
    /// How many descriptors each queue has, after this driver shrank it.
    /// Unit: descriptors.
    sizes: [u16; QUEUES_NEEDED as usize],
}

impl Transport {
    /// Reset the device, negotiate, and shrink both queues to `wanted`.
    ///
    /// Answers a transport whose queues are *sized* and not yet enabled: the
    /// caller has to give the device each queue's three ring addresses first —
    /// [`Transport::queue_at`] — and the ordering is the whole reason this is
    /// two calls. A device told to enable a queue whose address registers still
    /// hold their reset values is a device pointed at physical address zero, and
    /// on a *receive* queue that is a device that will write there.
    ///
    /// # Errors
    ///
    /// [`Trouble::NotResponding`] for a device that does not come out of reset,
    /// [`Trouble::NoPlatformAddressing`] for one that does not offer feature bit
    /// 33 — see the module comment for why that is fatal rather than a
    /// degradation — [`Trouble::FeaturesRefused`] for one that vetoes the set
    /// offered, [`Trouble::NoQueue`] for a device with fewer than two queues or
    /// a queue too small for a two-descriptor chain, and [`Trouble::Register`]
    /// carrying an `ARGUMENT/BAD_ADDRESS` for a window too short to hold the
    /// structure it was routed as.
    pub fn open(windows: Windows, wanted: u16) -> Result<Self, Trouble> {
        let Windows { common, notify, isr, config: _, notify_multiplier } = windows;
        if common.len() < common::EXTENT {
            return Err(Trouble::Register(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::BAD_ADDRESS,
            )));
        }

        // Reset first and unconditionally: firmware may have left the device
        // part-way through somebody else's initialisation, and a status register
        // written on top of that is a device in a state nothing describes.
        common.write8(common::DEVICE_STATUS, 0)?;
        if common.read8(common::DEVICE_STATUS)? != 0 {
            return Err(Trouble::NotResponding);
        }
        common.write8(common::DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
        common.write8(common::DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;

        // The upper half of the feature space, which is where the two bits this
        // driver cannot do without live.
        common.write32(common::DEVICE_FEATURE_SELECT, 1)?;
        let offered = common.read32(common::DEVICE_FEATURE)?;
        let wanted_features = FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM;
        if offered & wanted_features != wanted_features {
            return Err(Trouble::NoPlatformAddressing);
        }

        // Nothing from the lower half, which for this device is every
        // virtio-net feature there is. The module comment lists what each
        // absence costs.
        common.write32(common::DRIVER_FEATURE_SELECT, 0)?;
        common.write32(common::DRIVER_FEATURE, 0)?;
        common.write32(common::DRIVER_FEATURE_SELECT, 1)?;
        common.write32(common::DRIVER_FEATURE, wanted_features)?;
        common.write8(
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        )?;
        // Read back, because this is the one point in the handshake where the
        // device has a veto: a device that clears this bit has refused the set
        // offered, and a driver that carried on would be driving it under an
        // agreement only one side made. RFC 0011's rule, in the one place in
        // this system where the peer is silicon.
        if common.read8(common::DEVICE_STATUS)? & STATUS_FEATURES_OK == 0 {
            return Err(Trouble::FeaturesRefused);
        }

        // Asked rather than assumed, which is the check a one-queue driver never
        // had to make. A virtio-net device has at least one queue pair; a device
        // answering fewer than two is not one, and selecting queue one on it
        // would be writing registers it does not define.
        if common.read16(common::NUM_QUEUES)? < QUEUES_NEEDED {
            return Err(Trouble::NoQueue);
        }

        let mut doorbells = [0u32; QUEUES_NEEDED as usize];
        let mut sizes = [0u16; QUEUES_NEEDED as usize];
        let mut which = 0u16;
        while which < QUEUES_NEEDED {
            common.write16(common::QUEUE_SELECT, which)?;
            let offered_size = common.read16(common::QUEUE_SIZE)?;
            // Two descriptors is the least a chain on either queue can be: a
            // header the device reads or writes, and one buffer.
            if offered_size < 2 {
                return Err(Trouble::NoQueue);
            }
            // Never larger than the device offered, which is the only constraint
            // the specification places on this write — and shrinking is what
            // keeps the three rings inside the region the manifest declares, so
            // a queue's size is a property of this driver rather than of the
            // emulator.
            let size = if offered_size < wanted { offered_size } else { wanted };
            common.write16(common::QUEUE_SIZE, size)?;

            let notify_off = common.read16(common::QUEUE_NOTIFY_OFF)?;
            let Some(slot) = doorbells.get_mut(which as usize) else {
                return Err(Trouble::NoQueue);
            };
            *slot = u32::from(notify_off).saturating_mul(notify_multiplier);
            let Some(slot) = sizes.get_mut(which as usize) else { return Err(Trouble::NoQueue) };
            *slot = size;
            which += 1;
        }

        Ok(Self { common, notify, isr, doorbells, sizes })
    }

    /// How many descriptors queue `which` has. Unit: descriptors.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoQueue`] for a queue this driver did not size, which is
    /// every queue past the pair it negotiated for.
    pub fn size(&self, which: u16) -> Result<u16, Trouble> {
        self.sizes.get(which as usize).copied().ok_or(Trouble::NoQueue)
    }

    /// Give the device one queue's three ring addresses, in its own address
    /// space.
    ///
    /// Every one of them is a [`Region::device_at`](f_ring::device::Region::device_at)
    /// answer — which is [`Domains::map`](f_ring::registry::Domains::map)'s
    /// answer, which is the frame's — so a driver cannot point a device at an
    /// address of its own devising without the frame having translated it
    /// first.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a window that cannot hold the register, and
    /// [`Trouble::NoQueue`] for a queue this driver did not size.
    pub fn queue_at(&self, which: u16, desc: u64, driver: u64, device: u64) -> Result<(), Trouble> {
        if which >= QUEUES_NEEDED {
            return Err(Trouble::NoQueue);
        }
        self.common.write16(common::QUEUE_SELECT, which)?;
        self.common.write64(common::QUEUE_DESC, desc)?;
        self.common.write64(common::QUEUE_DRIVER, driver)?;
        self.common.write64(common::QUEUE_DEVICE, device)?;
        Ok(())
    }

    /// Enable both queues and tell the device the driver is ready.
    ///
    /// Both, in one call, and deliberately: the specification requires every
    /// queue a driver means to use to be enabled before `DRIVER_OK`, and a
    /// method that enabled one would be a method every caller has to remember to
    /// call twice.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn run(&self) -> Result<(), Trouble> {
        let mut which = 0u16;
        while which < QUEUES_NEEDED {
            self.common.write16(common::QUEUE_SELECT, which)?;
            self.common.write16(common::QUEUE_ENABLE, 1)?;
            which += 1;
        }
        self.common.write8(
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        Ok(())
    }

    /// Ring queue `which`'s doorbell.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoQueue`] for a queue this driver did not size, and
    /// [`Trouble::Register`] for a notification window too short for the
    /// doorbell the device's own capability placed there — which is a device
    /// describing itself inconsistently, and is refused rather than written
    /// somewhere nearby.
    pub fn kick(&self, which: u16) -> Result<(), Trouble> {
        let at = self.doorbells.get(which as usize).copied().ok_or(Trouble::NoQueue)?;
        self.notify.write16(at, which)?;
        Ok(())
    }

    /// Read the interrupt-status register.
    ///
    /// Called once per turn of the receive poll, and the reason is not the
    /// value: reading a device register is an exit to the emulator, which is a
    /// point at which the device's own work — and, on this path, the emulator's
    /// whole network backend — can make progress. A poll that only read memory
    /// would be a poll the emulator never got a chance to answer.
    ///
    /// It is also the register this driver will wait on once it has its
    /// interrupt. The manifest declares an `irq` need and says why the reversal
    /// is nearer for a receive queue than for anything the block driver does:
    /// there, a spin waits for an answer the device owes; here it waits for a
    /// packet nobody promised. E1-B09 owns it.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn poke(&self) -> Result<u8, Trouble> {
        Ok(self.isr.read8(0)?)
    }

    /// Put the device back in reset.
    ///
    /// What a driver does before its queue memory stops being its queue memory.
    /// A device left with a queue address pointing at a frame somebody else now
    /// owns is the corruption this whole subsystem is about, arrived at through
    /// the teardown — and for a **receive** queue it is not even a race that
    /// needs a request in flight: a device holding posted receive buffers writes
    /// into them the next time anything arrives on the link, which is a thing
    /// no code in this system decides.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn stop(&self) -> Result<(), Trouble> {
        self.common.write8(common::DEVICE_STATUS, 0)?;
        Ok(())
    }
}
