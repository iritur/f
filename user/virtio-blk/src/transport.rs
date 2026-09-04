// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The modern virtio PCI transport, driven through four granted register
//! windows and no `unsafe`.
//!
//! # Why the driver does not walk configuration space
//!
//! Because `user/virtio-blk/manifest.toml` says it does not. The manifest
//! declares four register frames routed from the supervisor — *common
//! configuration, notification, ISR and device configuration, as the modern PCI
//! transport lays them out in one BAR* — so finding the device, reading its
//! capability list and mapping those four structures is the supervisor's work,
//! and what arrives here is four [`Window`]s and a notification multiplier.
//!
//! That is a boundary and not a convenience. Configuration space is the whole
//! bus: a component that could read it could enumerate every function on the
//! machine, and a component that could *write* it could move another device's
//! base-address register. Handing a driver four windows rather than a bus is
//! what makes *this driver drives this device* a statement about capabilities.
//!
//! # The feature bit the whole isolation story rests on
//!
//! `VIRTIO_F_ACCESS_PLATFORM`, bit 33. Without it a virtio device is defined to
//! address physical memory directly and the emulator obliges, bypassing the
//! remapping unit entirely — so every isolation test passes for the wrong
//! reason, which is exactly what E1-B01 found after building its first
//! provocation on the legacy interface. [`Transport::open`] **refuses** a device
//! that does not offer the bit, rather than proceeding without it: a driver in
//! this system that fails to negotiate it has no isolation, and R04 says the
//! answer to that is a refusal.
//!
//! The legacy transport cannot express bit 33 at all — its feature word is
//! thirty-two bits — so there is no legacy path here and there will not be one.
//! *Reversal:* a device this system must drive that offers only the legacy
//! interface. Such a device cannot be isolated, so what reverses with it is
//! whether the device is used, not whether this file grows a second path.

use f_ring::device::Window;

use crate::Trouble;

/// Offsets in the common configuration structure.
///
/// The specification's, not this file's: every one of them is fixed by the
/// virtio standard, which is why they are constants rather than fields read out
/// of anything.
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

/// Where the block device publishes its capacity, in its own configuration
/// structure.
const CONFIG_CAPACITY: u32 = 0x00;

/// Bytes in a sector, which is the grain every offset and length in this
/// protocol is a multiple of.
/// Unit: bytes.
pub const SECTOR_BYTES: u32 = 512;

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
    /// The device's own configuration structure, which for a block device
    /// carries its capacity.
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
/// was granted, and the two are different kinds of authority. Keeping them
/// apart is what lets the queue be tested on a host with no device.
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    common: Window,
    notify: Window,
    isr: Window,
    config: Window,
    /// Where queue zero's doorbell is, inside [`Transport::notify`].
    /// Unit: bytes.
    doorbell: u32,
    /// How many descriptors queue zero has, after this driver shrank it.
    /// Unit: descriptors.
    size: u16,
}

impl Transport {
    /// Reset the device, negotiate, and shrink queue zero to `wanted`.
    ///
    /// Answers a transport whose queue is *selected and sized* and not yet
    /// enabled: the caller has to give the device the three ring addresses
    /// first — [`Transport::queue_at`] — and the ordering is the whole reason
    /// this is two calls. A device told to enable a queue whose address
    /// registers still hold their reset values is a device pointed at physical
    /// address zero.
    ///
    /// # Errors
    ///
    /// [`Trouble::NotResponding`] for a device that does not come out of reset,
    /// [`Trouble::NoPlatformAddressing`] for one that does not offer feature bit
    /// 33 — see the module comment for why that is fatal rather than a
    /// degradation — [`Trouble::FeaturesRefused`] for one that vetoes the set
    /// offered, [`Trouble::NoQueue`] for a queue too small for a
    /// three-descriptor chain, and [`Trouble::Register`] carrying an
    /// `ARGUMENT/BAD_ADDRESS` for a window too short to hold the structure it
    /// was routed as.
    pub fn open(windows: Windows, wanted: u16) -> Result<Self, Trouble> {
        let Windows { common, notify, isr, config, notify_multiplier } = windows;
        if common.len() < common::EXTENT {
            return Err(Trouble::Register(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::BAD_ADDRESS,
            )));
        }

        // Reset first and unconditionally: firmware may have left the device
        // part-way through somebody else's initialisation, and a status
        // register written on top of that is a device in a state nothing
        // describes.
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

        // Nothing from the lower half. Every feature down there is an
        // optimisation or a second layout, and a driver that negotiated one
        // would be a driver whose behaviour depends on what the emulator was
        // built with — which is a thing a fixture must not depend on.
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

        common.write16(common::QUEUE_SELECT, 0)?;
        let offered_size = common.read16(common::QUEUE_SIZE)?;
        if offered_size < 3 {
            return Err(Trouble::NoQueue);
        }
        // Never larger than the device offered, which is the only constraint the
        // specification places on this write — and shrinking is what keeps the
        // three rings inside the region the manifest declares, so the queue's
        // size is a property of this driver rather than of the emulator.
        let size = if offered_size < wanted { offered_size } else { wanted };
        common.write16(common::QUEUE_SIZE, size)?;

        let notify_off = common.read16(common::QUEUE_NOTIFY_OFF)?;
        let doorbell = u32::from(notify_off).saturating_mul(notify_multiplier);

        Ok(Self { common, notify, isr, config, doorbell, size })
    }

    /// How many descriptors queue zero has. Unit: descriptors.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Give the device the three ring addresses, in its own address space.
    ///
    /// Every one of them is a [`Region::device_at`](f_ring::device::Region::device_at)
    /// answer — which is [`Domains::map`](f_ring::registry::Domains::map)'s
    /// answer, which is the frame's — so a driver cannot point a device at an
    /// address of its own devising without the frame having translated it
    /// first. That is the sentence the second half of this task makes the
    /// hardware enforce rather than merely stating.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a window that cannot hold the register.
    pub fn queue_at(&self, desc: u64, driver: u64, device: u64) -> Result<(), Trouble> {
        self.common.write16(common::QUEUE_SELECT, 0)?;
        self.common.write64(common::QUEUE_DESC, desc)?;
        self.common.write64(common::QUEUE_DRIVER, driver)?;
        self.common.write64(common::QUEUE_DEVICE, device)?;
        Ok(())
    }

    /// Enable the queue and tell the device the driver is ready.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn run(&self) -> Result<(), Trouble> {
        self.common.write16(common::QUEUE_SELECT, 0)?;
        self.common.write16(common::QUEUE_ENABLE, 1)?;
        self.common.write8(
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        Ok(())
    }

    /// Ring queue zero's doorbell.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a notification window too short for the
    /// doorbell the device's own capability placed there — which is a device
    /// describing itself inconsistently, and is refused rather than written
    /// somewhere nearby.
    pub fn kick(&self) -> Result<(), Trouble> {
        self.notify.write16(self.doorbell, 0)?;
        Ok(())
    }

    /// Read the interrupt-status register.
    ///
    /// Called once per turn of the completion poll, and the reason is not the
    /// value: reading a device register is an exit to the emulator, which is a
    /// point at which the device's own work can make progress. A poll that only
    /// read memory would be a poll the emulator never got a chance to answer —
    /// `dma.rs` records the same thing about its own spin.
    ///
    /// It is also the register this driver will wait on once it has its
    /// interrupt: the manifest declares an `irq` need, and waiting on it is
    /// E1-B09's. Until then the status is read and discarded, which is a poll
    /// wearing an interrupt's clothes and is written down as such.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn poke(&self) -> Result<u8, Trouble> {
        Ok(self.isr.read8(0)?)
    }

    /// How many sectors the device says it holds.
    ///
    /// Unit: sectors of [`SECTOR_BYTES`] bytes. Read from the device's own
    /// configuration structure, which is the fourth routed window and the only
    /// thing this driver uses it for — a bound to refuse a request past the end
    /// of the disk with, rather than discovering it as a device error.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a configuration window too short to hold it.
    pub fn capacity(&self) -> Result<u64, Trouble> {
        let low = self.config.read32(CONFIG_CAPACITY)?;
        let high = self.config.read32(CONFIG_CAPACITY + 4)?;
        Ok(u64::from(low) | (u64::from(high) << 32))
    }

    /// Put the device back in reset.
    ///
    /// What a driver does before its queue memory stops being its queue memory.
    /// A device left with a queue address pointing at a frame somebody else now
    /// owns is the corruption this whole task is about, arrived at through the
    /// teardown — and a restart is exactly when it would happen.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn stop(&self) -> Result<(), Trouble> {
        self.common.write8(common::DEVICE_STATUS, 0)?;
        Ok(())
    }
}
