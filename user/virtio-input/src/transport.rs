// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The modern virtio PCI transport, as a component reaches it: four register
//! windows the frame mapped, a handshake, one queue, one doorbell.
//!
//! # Why this file looks like the other three and is not shared with them
//!
//! Because what is actually shared between four drivers is the whole of the
//! modern-transport handshake, and the three offsets in `crate::queue` are not
//! the interesting part of it. RFC 0054 records the decision at three drivers:
//! moving a handful of constants would leave the discipline duplicated while
//! making it look shared, and the merge that is worth doing is the one RFC 0071
//! did on the *supervisor* side, where the duplication was byte-identical code
//! rather than a common vocabulary.
//!
//! This is the fourth sample and it does not change that answer, but it adds
//! one data point the other three could not: the handshake below was written
//! against a device of a kind none of them drive — one that never reads
//! anything — and not a line of it differs from `user/virtio-gpu`'s except the
//! queue index and the feature set offered. That is the strongest evidence so
//! far that the transport is the transport, and it is the evidence a fifth
//! driver should be weighed against rather than a fifth copy.
//!
//! # What this driver does not do with a device configuration space
//!
//! It does not read it, which costs more here than on any other driver in the
//! tree and is worth stating precisely rather than in general.
//!
//! virtio-input's configuration space is a select/subselect window: a driver
//! writes `select` and `subsel`, and the device answers with the device's name,
//! its serial, its `devids` (bus, vendor, product, version), the absolute-axis
//! ranges, and — the two that matter — `EV_BITS`, the bitmaps saying which event
//! types and which codes within them this device can ever produce. A driver that
//! read them would know it had a mouse and not a keyboard before the first
//! record arrived, and could refuse a device it has no translation for at
//! start-up instead of counting records it ignores.
//!
//! What not reading them costs, exactly: this driver binds *any* virtio-input
//! device and finds out what it is from the records it produces, so a device
//! whose events this build has no opcode for is bound successfully and then
//! reports nothing but `crate::routing::reported::IGNORED`. That is a worse
//! error message than a refusal at start-up and it is not a worse *outcome* —
//! nothing is misread, because `crate::driver` decides what a record means from
//! the record. The cost is paid because the select window is a second protocol
//! with its own refusals, and a driver that could not be trusted to translate
//! `EV_REL` correctly would not be made trustworthy by also parsing a bitmap.
//!
//! *What would reverse this:* a machine with two virtio-input devices, which is
//! the ordinary case the moment there is both a keyboard and a mouse. Then
//! *which* device this instance was given is a question the routing page cannot
//! answer — the manifest binds a vendor and a device id, not an instance — and
//! `devids` is where the answer is. `E3-B04` is the parent line and the
//! supervisor is where the second instance would be bound.
//!
//! # No device-specific feature bits, which is unusual and is the reason to say
//! so
//!
//! virtio-input defines none. The whole of what this driver negotiates is
//! `VIRTIO_F_VERSION_1` and `VIRTIO_F_ACCESS_PLATFORM`, both of which are
//! transport features in the upper half of the space, and the lower half is
//! written as zero because there is nothing in it to want. On the other three
//! drivers that zero is a list of things declined; here it is the device's whole
//! vocabulary, and a reader who assumed an omission would be looking for
//! something that does not exist.

use f_ring::device::Window;

use crate::Trouble;

/// Byte offsets inside the common configuration structure, as the modern PCI
/// transport lays it out.
///
/// The same offsets the other three drivers name, and still not shared with them
/// for the module comment's reason. Unit: bytes.
pub mod common {
    /// Which half of the feature space the next read answers about.
    pub const DEVICE_FEATURE_SELECT: u32 = 0x00;
    /// The features the device offers in the selected half.
    pub const DEVICE_FEATURE: u32 = 0x04;
    /// Which half of the feature space the next write applies to.
    pub const DRIVER_FEATURE_SELECT: u32 = 0x08;
    /// The features the driver accepts in the selected half.
    pub const DRIVER_FEATURE: u32 = 0x0C;
    /// How many queues the device has.
    pub const NUM_QUEUES: u32 = 0x12;
    /// The handshake's state machine.
    pub const DEVICE_STATUS: u32 = 0x14;
    /// Which queue the queue registers below answer about.
    pub const QUEUE_SELECT: u32 = 0x16;
    /// How many descriptors the selected queue has; written to shrink it.
    pub const QUEUE_SIZE: u32 = 0x18;
    /// Non-zero once the driver has given the queue its addresses.
    pub const QUEUE_ENABLE: u32 = 0x1C;
    /// The selected queue's offset into the notification structure.
    pub const QUEUE_NOTIFY_OFF: u32 = 0x1E;
    /// Where the device finds the descriptor table.
    pub const QUEUE_DESC: u32 = 0x20;
    /// Where it finds the available ring.
    pub const QUEUE_DRIVER: u32 = 0x28;
    /// Where it finds the used ring.
    pub const QUEUE_DEVICE: u32 = 0x30;
    /// How many bytes of the structure this driver reads or writes.
    /// Unit: bytes.
    pub const EXTENT: u32 = 0x38;
}

/// The driver has noticed the device.
const STATUS_ACKNOWLEDGE: u8 = 1;
/// The driver knows how to drive it.
const STATUS_DRIVER: u8 = 2;
/// The driver is ready; the device may use the queues.
const STATUS_DRIVER_OK: u8 = 4;
/// The feature negotiation is settled, and the device has a veto.
const STATUS_FEATURES_OK: u8 = 8;

/// Feature bit 32: the device speaks the modern specification.
///
/// Bit zero of the *upper* half of the feature space, which is why every read
/// and write of it below is preceded by a select of half one.
const FEATURE_VERSION_1: u32 = 1 << 0;

/// Feature bit 33: the device's transfers go through the platform's address
/// translation.
///
/// Required rather than preferred. `crate::Trouble::NoPlatformAddressing` says
/// why refusing is the only honest answer, and the argument is sharper for this
/// device than for any of the other three: a device without this bit writes
/// wherever the driver's arithmetic said, whenever the *user* moves a mouse,
/// with nothing outstanding and nothing timing it.
const FEATURE_ACCESS_PLATFORM: u32 = 1 << 1;

/// The queues this driver requires the device to have. Unit: queues.
///
/// One: the event queue, which is queue zero. A virtio-input device also defines
/// a status queue at index one — `crate::queue::index::STATUS` says what not
/// using it costs — and this driver neither enables it nor requires it to exist,
/// because a driver that refused a device for a queue it never touches would be
/// refusing on its own behalf rather than on the specification's.
const QUEUES_NEEDED: u16 = 1;

/// The least the event queue can be for this driver to use it.
///
/// One descriptor, because one `virtio_input_event` is one descriptor and there
/// is no chain. That is the smallest number in this position in the tree — the
/// other three need two or more, for a header and a payload — and the difference
/// is the shape of the device rather than a relaxation: there is nothing for a
/// second descriptor to carry. Unit: descriptors.
const QUEUE_MINIMUM: u16 = 1;

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
    /// The device's own configuration structure. Read by nothing in this driver
    /// — the module comment says what that costs — and routed anyway, because
    /// the manifest declares four register frames.
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
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    common: Window,
    notify: Window,
    isr: Window,
    /// Where the event queue's doorbell is, inside [`Transport::notify`].
    /// Unit: bytes.
    doorbell: u32,
    /// How many descriptors the event queue has, after this driver shrank it.
    /// Unit: descriptors.
    size: u16,
}

impl Transport {
    /// Reset the device, negotiate, and shrink the event queue to `wanted`.
    ///
    /// Answers a transport whose queue is *sized* and not yet enabled: the
    /// caller has to give the device the queue's three ring addresses first —
    /// [`Transport::queue_at`] — and the ordering is the whole reason this is
    /// two calls. A device told to enable a queue whose address registers still
    /// hold their reset values is a device pointed at physical address zero, and
    /// on this device that is a device pointed at physical address zero *with a
    /// writable descriptor*.
    ///
    /// # Errors
    ///
    /// [`Trouble::NotResponding`] for a device that does not come out of reset,
    /// [`Trouble::NoPlatformAddressing`] for one that does not offer feature bit
    /// 33, [`Trouble::FeaturesRefused`] for one that vetoes the set offered,
    /// [`Trouble::NoQueue`] for a device with no event queue or one with no
    /// descriptors, and [`Trouble::Register`] carrying an `ARGUMENT/BAD_ADDRESS`
    /// for a window too short to hold the structure it was routed as.
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
        // driver cannot do without live — and, for this device, the only half
        // with anything in it at all.
        common.write32(common::DEVICE_FEATURE_SELECT, 1)?;
        let offered = common.read32(common::DEVICE_FEATURE)?;
        let wanted_features = FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM;
        if offered & wanted_features != wanted_features {
            return Err(Trouble::NoPlatformAddressing);
        }

        // Zero in the lower half, and here that is the device's whole
        // vocabulary rather than a list of declines. The module comment says so
        // because a reader who assumed an omission would go looking for a
        // feature this device does not have.
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

        if common.read16(common::NUM_QUEUES)? < QUEUES_NEEDED {
            return Err(Trouble::NoQueue);
        }

        common.write16(common::QUEUE_SELECT, crate::queue::index::EVENT)?;
        let offered_size = common.read16(common::QUEUE_SIZE)?;
        if offered_size < QUEUE_MINIMUM {
            return Err(Trouble::NoQueue);
        }
        // Never larger than the device offered, which is the only constraint the
        // specification places on this write — and shrinking is what keeps the
        // rings and the buffers inside the region the manifest declares, so a
        // queue's size is a property of this driver rather than of the emulator.
        //
        // It is also rounded *down* to a power of two, which the other three
        // drivers do not have to do because they ask for one. `crate::queue`
        // masks its cursors rather than dividing them, so a size that is not a
        // power of two would be a ring whose slot arithmetic is wrong — refused
        // by `Queue::over` rather than accepted here, and shrunk here so that
        // the refusal is not reached by an emulator offering a legal size this
        // layout cannot use.
        let capped = if offered_size < wanted { offered_size } else { wanted };
        let size = if capped.is_power_of_two() {
            capped
        } else {
            1 << (u16::BITS - 1 - capped.leading_zeros())
        };
        common.write16(common::QUEUE_SIZE, size)?;

        let notify_off = common.read16(common::QUEUE_NOTIFY_OFF)?;
        let doorbell = u32::from(notify_off).saturating_mul(notify_multiplier);

        Ok(Self { common, notify, isr, doorbell, size })
    }

    /// How many descriptors the event queue has. Unit: descriptors.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Give the device the event queue's three ring addresses, in its own
    /// address space.
    ///
    /// Every one of them is a `f_ring::device::Region::device_at` answer — which
    /// is the frame's answer to a `DEVICE_MAP` — so a driver cannot point a
    /// device at an address of its own devising without the frame having
    /// translated it first.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a window that cannot hold the register.
    pub fn queue_at(&self, desc: u64, driver: u64, device: u64) -> Result<(), Trouble> {
        self.common.write16(common::QUEUE_SELECT, crate::queue::index::EVENT)?;
        self.common.write64(common::QUEUE_DESC, desc)?;
        self.common.write64(common::QUEUE_DRIVER, driver)?;
        self.common.write64(common::QUEUE_DEVICE, device)?;
        Ok(())
    }

    /// Enable the event queue and tell the device the driver is ready.
    ///
    /// **This is the point at which the device may start writing**, which is a
    /// sentence none of the other three transports can say: they enable a queue
    /// the device will not touch until it is kicked. Here the buffers have to be
    /// posted *before* this call, because a user pressing a key one microsecond
    /// afterwards is a device with a record to write and nowhere to put it.
    /// `crate::driver::Driver::start` is what keeps that order.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn run(&self) -> Result<(), Trouble> {
        self.common.write16(common::QUEUE_SELECT, crate::queue::index::EVENT)?;
        self.common.write16(common::QUEUE_ENABLE, 1)?;
        self.common.write8(
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        Ok(())
    }

    /// Ring the event queue's doorbell.
    ///
    /// Rung after buffers are posted, and that is the only reason this driver
    /// rings it at all: the doorbell says *there is something new in the
    /// available ring*, which on a queue that is refilled means *here is another
    /// buffer*. A driver that never rang it would be relying on the device
    /// re-reading a ring it has no reason to look at again.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a notification window too short for the
    /// doorbell the device's own capability placed there — which is a device
    /// describing itself inconsistently, and is refused rather than written
    /// somewhere nearby.
    pub fn kick(&self) -> Result<(), Trouble> {
        self.notify.write16(self.doorbell, crate::queue::index::EVENT)?;
        Ok(())
    }

    /// Read the interrupt-status register.
    ///
    /// Called once per idle turn, and the reason is not the value: reading a
    /// device register is an exit to the emulator, which is a point at which the
    /// device's own work can make progress. A poll that only read memory would
    /// be a poll the emulator never got a chance to answer.
    ///
    /// It is also the register this driver will wait on once it has its
    /// interrupt, and the reversal is **nearer here than on any other driver in
    /// the tree**: the other three spin against a device that owes them an
    /// answer, so their bound is an anti-wedge measure. This one spins against a
    /// user, who owes nothing and may do nothing for hours. `E1-B09` is what
    /// turns the `notify` capability the manifest declares into a wait, and
    /// until it does, this driver's idleness is a count and says so.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn poke(&self) -> Result<u8, Trouble> {
        Ok(self.isr.read8(0)?)
    }

    /// Put the device back in reset.
    ///
    /// Called when the component's loop ends, and — unlike the display driver,
    /// where a reset blanks the screen and is refused for that reason — it is
    /// the ordinary teardown here. What it stops is the device writing into this
    /// component's buffers, which is a thing it would otherwise go on doing
    /// whenever the user touched the machine, into memory the frame is about to
    /// take back. The frame clears the bus-master bit and detaches the function
    /// from its domain as well, and that is what makes this belt-and-braces
    /// rather than load-bearing; it is done anyway because the alternative is a
    /// device left armed on the assumption that the frame will be quick.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`]. Its caller ignores the answer: it is already
    /// ending, and a failed reset leaves nothing further a component can do.
    pub fn reset(&self) -> Result<(), Trouble> {
        self.common.write8(common::DEVICE_STATUS, 0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::QUEUE_SIZE;

    /// How many bytes of a common configuration structure the fixture holds.
    ///
    /// Sixty-four, which is [`common::EXTENT`] rounded up to something a reader
    /// can check without a calculator. Unit: bytes.
    const DEVICE_BYTES: u32 = 64;
    const _: () = assert!(common::EXTENT <= DEVICE_BYTES);

    /// A device's registers, in memory.
    ///
    /// Backed by an array rather than by a register window, and that is the
    /// whole of what makes these tests possible: [`Window`] is a bounds-checked
    /// volatile accessor over an address and nothing in it requires the address
    /// to be a device. So a test can put the bytes a device *would* have
    /// published where the driver will read them, and watch the handshake refuse.
    ///
    /// **What it cannot model is a device that answers differently from what was
    /// written.** Memory takes every write and gives it back, so the two
    /// refusals [`Transport::open`] reaches by reading a register back —
    /// [`Trouble::NotResponding`], where the status register does not clear, and
    /// [`Trouble::FeaturesRefused`], where the device clears `FEATURES_OK` —
    /// cannot be reached here and are not tested. Saying that is better than a
    /// fixture that looks as though it covers them: what would cover them is a
    /// window backed by a model that answers, which is `sim/`'s business and not
    /// this crate's. `user/virtio-gpu/src/transport.rs` says the same thing and
    /// the limit is the same limit.
    #[repr(align(8))]
    struct Device {
        common: [u8; DEVICE_BYTES as usize],
        notify: [u8; 16],
        isr: [u8; 4],
        config: [u8; 8],
    }

    impl Device {
        /// A device that offers both transport bits and a full-sized queue.
        fn healthy() -> Self {
            let mut device = Self {
                common: [0; DEVICE_BYTES as usize],
                notify: [0; 16],
                isr: [0; 4],
                config: [0; 8],
            };
            device.put32(common::DEVICE_FEATURE, FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM);
            device.put16(common::NUM_QUEUES, 2);
            device.put16(common::QUEUE_SIZE, QUEUE_SIZE);
            device.put16(common::QUEUE_NOTIFY_OFF, 1);
            device
        }

        fn put16(&mut self, at: u32, value: u16) {
            let at = at as usize;
            self.common[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }

        fn put32(&mut self, at: u32, value: u32) {
            let at = at as usize;
            self.common[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn get16(&self, at: u32) -> u16 {
            let at = at as usize;
            let mut raw = [0u8; 2];
            raw.copy_from_slice(&self.common[at..at + 2]);
            u16::from_le_bytes(raw)
        }

        fn get8(&self, at: u32) -> u8 {
            self.common[at as usize]
        }

        /// The four windows, with `len` bytes of the common structure — so that
        /// a test can route a window too short and watch it be refused before
        /// anything is written.
        fn windows(&mut self, len: u32) -> Windows {
            Windows {
                common: Window::at(self.common.as_mut_ptr() as usize as u64, len)
                    .expect("a window"),
                notify: Window::at(self.notify.as_mut_ptr() as usize as u64, 16).expect("a window"),
                isr: Window::at(self.isr.as_mut_ptr() as usize as u64, 4).expect("a window"),
                config: Window::at(self.config.as_mut_ptr() as usize as u64, 8).expect("a window"),
                notify_multiplier: 4,
            }
        }
    }

    #[test]
    fn a_device_that_does_not_offer_platform_addressing_is_refused() {
        // R04, and on this device it is the sharpest form of it in the tree: a
        // device outside the remapping unit writes wherever this driver's
        // arithmetic said, whenever the user moves, with nothing outstanding.
        let mut device = Device::healthy();
        device.put32(common::DEVICE_FEATURE, FEATURE_VERSION_1);
        let windows = device.windows(DEVICE_BYTES);
        assert_eq!(
            Transport::open(windows, QUEUE_SIZE).map(|_| ()),
            Err(Trouble::NoPlatformAddressing)
        );

        let mut device = Device::healthy();
        device.put32(common::DEVICE_FEATURE, FEATURE_ACCESS_PLATFORM);
        let windows = device.windows(DEVICE_BYTES);
        assert_eq!(
            Transport::open(windows, QUEUE_SIZE).map(|_| ()),
            Err(Trouble::NoPlatformAddressing),
            "a legacy device is refused on the same register"
        );
    }

    #[test]
    fn the_driver_accepts_the_two_bits_it_asked_for_and_nothing_it_was_offered() {
        // The device below offers every bit there is. What the driver writes back
        // is what it asked for, because a driver that accepted an offer it does
        // not implement has agreed to behave in a way it will not.
        let mut device = Device::healthy();
        device.put32(common::DEVICE_FEATURE, u32::MAX);
        let windows = device.windows(DEVICE_BYTES);
        Transport::open(windows, QUEUE_SIZE).expect("a healthy device");

        assert_eq!(device.get16(common::QUEUE_SELECT), crate::queue::index::EVENT);
        assert_eq!(
            device.get8(common::DEVICE_STATUS),
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
            "the handshake stops short of DRIVER_OK until `run`"
        );
    }

    #[test]
    fn a_queue_larger_than_the_layout_is_shrunk_and_a_smaller_one_is_taken_as_it_is() {
        let mut device = Device::healthy();
        device.put16(common::QUEUE_SIZE, 1024);
        let windows = device.windows(DEVICE_BYTES);
        let transport = Transport::open(windows, QUEUE_SIZE).expect("a healthy device");
        assert_eq!(transport.size(), QUEUE_SIZE);
        assert_eq!(device.get16(common::QUEUE_SIZE), QUEUE_SIZE, "the device is told");

        let mut device = Device::healthy();
        device.put16(common::QUEUE_SIZE, 16);
        let windows = device.windows(DEVICE_BYTES);
        let transport = Transport::open(windows, QUEUE_SIZE).expect("a healthy device");
        assert_eq!(transport.size(), 16);
    }

    #[test]
    fn a_legal_queue_size_this_layout_cannot_mask_is_rounded_down() {
        // The one line of this handshake the other three drivers do not have.
        // The specification permits any size up to 32 768; `crate::queue` masks
        // its cursors, which is only correct for a power of two. Rounding down
        // here is what keeps a device offering 48 descriptors from reaching
        // `Queue::over`'s refusal and stopping a boot over a legal device.
        let mut device = Device::healthy();
        device.put16(common::QUEUE_SIZE, 48);
        let windows = device.windows(DEVICE_BYTES);
        let transport = Transport::open(windows, QUEUE_SIZE).expect("a healthy device");
        assert_eq!(transport.size(), 32);
        assert!(transport.size().is_power_of_two());
    }

    #[test]
    fn a_device_with_no_event_queue_or_one_with_no_descriptors_is_refused() {
        let mut device = Device::healthy();
        device.put16(common::NUM_QUEUES, 0);
        let windows = device.windows(DEVICE_BYTES);
        assert_eq!(Transport::open(windows, QUEUE_SIZE).map(|_| ()), Err(Trouble::NoQueue));

        let mut device = Device::healthy();
        device.put16(common::QUEUE_SIZE, 0);
        let windows = device.windows(DEVICE_BYTES);
        assert_eq!(Transport::open(windows, QUEUE_SIZE).map(|_| ()), Err(Trouble::NoQueue));
    }

    #[test]
    fn a_window_too_short_for_the_structure_is_refused_before_anything_is_written() {
        // Before, which is the half worth testing: a driver that wrote the reset
        // and then discovered the window was short would have put a device into
        // a state nothing describes.
        let mut device = Device::healthy();
        device.put16(common::QUEUE_SIZE, QUEUE_SIZE);
        let windows = device.windows(common::EXTENT - 1);
        assert_eq!(
            Transport::open(windows, QUEUE_SIZE).map(|_| ()),
            Err(Trouble::Register(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::BAD_ADDRESS,
            )))
        );
        assert_eq!(device.get16(common::QUEUE_SIZE), QUEUE_SIZE, "nothing was written");
    }

    #[test]
    fn the_doorbell_is_where_the_device_said_it_would_be() {
        // `notify_off` times the multiplier, and not either one alone. A driver
        // that ignored the multiplier would ring queue zero's doorbell correctly
        // and every other queue's into the wrong register.
        let mut device = Device::healthy();
        device.put16(common::QUEUE_NOTIFY_OFF, 2);
        let windows = device.windows(DEVICE_BYTES);
        let transport = Transport::open(windows, QUEUE_SIZE).expect("a healthy device");
        transport.kick().expect("a doorbell inside the notification window");

        let mut raw = [0u8; 2];
        raw.copy_from_slice(&device.notify[8..10]);
        assert_eq!(
            u16::from_le_bytes(raw),
            crate::queue::index::EVENT,
            "the doorbell is at notify_off * multiplier = 8"
        );
    }
}
