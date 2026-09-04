// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The modern virtio PCI transport, driven through four granted register
//! windows and no `unsafe` — for a device with one queue this driver uses and
//! one it does not.
//!
//! # Why the driver does not walk configuration space
//!
//! Because `user/virtio-gpu/manifest.toml` says it does not. The manifest
//! declares four register frames, not a bus, so finding the device, reading its
//! capability list and mapping those four structures is the supervisor's work,
//! and what arrives here is four [`Window`]s and a notification multiplier.
//!
//! That is a boundary and not a convenience, and both other drivers say why at
//! length: a component that could read configuration space could enumerate every
//! function on the machine, and one that could write it could move another
//! device's base-address register. It is not repeated here.
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
//! What the bit is worth differs by device and this is the third answer. On a
//! block device an unrefused escape is the device *reading* memory it was not
//! granted, bounded by a request. On a network device it is the device *writing*
//! into memory it was not granted, at a moment nothing in this system chose. On
//! a display controller it is a read again — and the reading is the smaller half
//! of it. What a display does with what it reads is **put it on a screen**,
//! which is outside the machine: an unrefused escape here is not a corruption to
//! be found later in somebody's buffer, it is a page of another component's
//! memory rendered to whoever is looking at the display. That is the first
//! exfiltration path in this tree that does not go through a ring, a channel or
//! a capability, and the remapping unit is the only thing standing in it.
//!
//! The legacy transport cannot express bit 33 at all — its feature word is
//! thirty-two bits — so there is no legacy path here and there will not be one.
//! There is also no legacy virtio-gpu device to have one: the display controller
//! was defined after the modern transport and has no transitional PCI device id,
//! which is a fact `kernel/src/arch/x86_64/virtio.rs` had to be changed to
//! express and is the largest single thing the frame owed a third driver. RFC
//! 0054.
//!
//! # What this driver deliberately does not negotiate, and what that costs
//!
//! Nothing from the lower feature word: not `VIRTIO_GPU_F_VIRGL`, not
//! `VIRTIO_GPU_F_EDID`, not `VIRTIO_GPU_F_RESOURCE_UUID`, not
//! `VIRTIO_GPU_F_RESOURCE_BLOB`, not `VIRTIO_GPU_F_CONTEXT_INIT`. The same
//! choice both other drivers made and for the same reason — every feature down
//! there is an optimisation or a second layout, and a driver that negotiated one
//! would be a driver whose behaviour depends on what the emulator was built
//! with, which is a thing a fixture must not depend on.
//!
//! The costs, stated rather than discovered:
//!
//! - **No `VIRGL` and no `CONTEXT_INIT`.** No three-dimensional rendering and no
//!   per-context command streams. This driver moves a rectangle of pixels a
//!   client already has; it cannot ask the display to compute one. That is the
//!   whole of what *minimal* means in this task's title and it is the reason
//!   this crate is a tenth the size of a real display driver.
//! - **No `EDID`.** The display's real modes are not read, so this driver does
//!   not know what the screen is. It does not need to: the geometry of a frame
//!   is the client's, carried in the entry, and refused against the buffer the
//!   entry names. A driver that read `EDID` would be a driver with an opinion
//!   about what its clients may draw.
//! - **No `RESOURCE_BLOB`.** A resource is made of guest pages this driver
//!   attaches and the device copies out of. With blob resources the guest and
//!   the host can share the pixels with no copy at the *host* boundary at all,
//!   which is the next zero-copy claim after this one and is not this task's.
//!   What it costs today is one host-side copy per frame, inside the emulator,
//!   which no counter in this tree can see and which this comment is therefore
//!   the only record of.
//! - **No `RESOURCE_UUID`.** No way to hand a resource to another device. A
//!   system with a camera and a display would want it.
//!
//! It also does not read the device's own configuration structure, which for a
//! display controller carries `num_scanouts` and a pending-events word. The cost
//! is exact: this driver assumes scanout zero exists, and a machine whose
//! display has none would have its `SET_SCANOUT` refused by the device rather
//! than by this driver — a refusal that arrives one round trip later and names
//! the display rather than the manifest. `crate::routing::at::CONFIG_OFFSET` is
//! routed anyway, because the manifest declares four register frames and a page
//! the frame stopped filling in would be a manifest describing a component
//! nobody had checked against it.
//!
//! # Why there is no `stop`
//!
//! **Both other drivers have one and this one must not**, and it is the sharpest
//! difference the third device made. `Transport::stop` on those two writes zero
//! to `DEVICE_STATUS`, which is a reset, and a reset is what a driver owes a
//! device before its queue memory stops being its queue memory. On a display
//! controller a reset also destroys every resource and replaces the scanout with
//! nothing — it **blanks the screen** — so a display driver that reset itself on
//! an ordinary ending would be a driver whose last act is to throw away the one
//! thing it was asked to produce.
//!
//! Two facts make leaving the device running safe, and both are properties of
//! the *kind* of device rather than of this code:
//!
//! - **A display controller does nothing until it is told.** A network card
//!   writes into posted buffers when a packet arrives, which is why
//!   `user/virtio-net` resets before it gives a buffer back. Nothing arrives at
//!   a display. Every transfer it performs is one this driver asked for on a
//!   doorbell it rang.
//! - **The frame takes the access away, not the driver.** `kernel/src/gpu.rs`
//!   clears the bus-master bit and detaches the function from its domain before
//!   it frees anything, exactly as it does for the other two. That is the
//!   backstop, and it is the frame's because the frame is what owns the
//!   allocator.
//!
//! What is *not* skipped is the reset a driver owes at the **start**:
//! [`Transport::open`] writes zero to `DEVICE_STATUS` before anything else and
//! refuses a device that does not come back. So a restarted display driver does
//! reset — and blanks the screen — which is what
//! `user/virtio-gpu/manifest.toml`'s restart comment says is the honest first act
//! of one.

use f_ring::device::Window;

use crate::Trouble;

/// Offsets in the common configuration structure.
///
/// The specification's, not this file's: every one of them is fixed by the
/// virtio standard, which is why they are constants rather than fields read out
/// of anything. They are the same constants `user/virtio-blk/src/transport.rs`
/// and `user/virtio-net/src/transport.rs` hold, and they are written again
/// rather than shared.
///
/// **Three copies is where this stops being a defensible trade**, and saying so
/// is the point of writing it a third time. RFC 0051 argued that what the two
/// drivers shared was much larger than a table of offsets and that a third
/// driver should decide; RFC 0054 is that decision, and it is *not* taken here —
/// what belongs in `abi/` is the whole of the modern transport handshake and not
/// nine numbers, and moving nine numbers would leave the handshake duplicated
/// while making it look shared.
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
    /// How many queues the device has. Read and never written.
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

/// The queues this driver requires the device to have. Unit: queues.
///
/// One: the control queue, which is queue zero and carries every command in the
/// display protocol. A virtio-gpu device also defines a cursor queue at index
/// one — `crate::queue::index::CURSOR` says what not using it costs — and this
/// driver neither enables it nor requires it to exist, because a driver that
/// refused a device for a queue it never touches would be refusing on its own
/// behalf rather than on the specification's.
const QUEUES_NEEDED: u16 = 1;

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
    /// driver — the module comment says what that costs — and routed anyway,
    /// because the manifest declares four register frames.
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
    /// Where the control queue's doorbell is, inside [`Transport::notify`].
    /// Unit: bytes.
    doorbell: u32,
    /// How many descriptors the control queue has, after this driver shrank it.
    /// Unit: descriptors.
    size: u16,
}

impl Transport {
    /// Reset the device, negotiate, and shrink the control queue to `wanted`.
    ///
    /// Answers a transport whose queue is *sized* and not yet enabled: the
    /// caller has to give the device the queue's three ring addresses first —
    /// [`Transport::queue_at`] — and the ordering is the whole reason this is
    /// two calls. A device told to enable a queue whose address registers still
    /// hold their reset values is a device pointed at physical address zero.
    ///
    /// # Errors
    ///
    /// [`Trouble::NotResponding`] for a device that does not come out of reset,
    /// [`Trouble::NoPlatformAddressing`] for one that does not offer feature bit
    /// 33 — see the module comment for why that is fatal rather than a
    /// degradation — [`Trouble::FeaturesRefused`] for one that vetoes the set
    /// offered, [`Trouble::NoQueue`] for a device with no control queue or one
    /// too small for a two-descriptor chain, and [`Trouble::Register`] carrying
    /// an `ARGUMENT/BAD_ADDRESS` for a window too short to hold the structure it
    /// was routed as.
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
        // written on top of that is a device in a state nothing describes. On
        // this device it is also the *only* reset — see the module comment on
        // why there is no `stop` — so a restarted display driver blanks the
        // screen here and nowhere else.
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
        // virtio-gpu feature there is. The module comment lists what each
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

        if common.read16(common::NUM_QUEUES)? < QUEUES_NEEDED {
            return Err(Trouble::NoQueue);
        }

        common.write16(common::QUEUE_SELECT, crate::queue::index::CONTROL)?;
        let offered_size = common.read16(common::QUEUE_SIZE)?;
        // Two descriptors is the least a display command can be: the command the
        // device reads and the response it writes. Every command in this driver
        // is exactly that chain, which is why the number is two and not three —
        // a display command carries its arguments in the command structure
        // rather than in a separate buffer, so there is no payload descriptor at
        // all. `crate::driver` says what that buys.
        if offered_size < 2 {
            return Err(Trouble::NoQueue);
        }
        // Never larger than the device offered, which is the only constraint the
        // specification places on this write — and shrinking is what keeps the
        // three rings inside the region the manifest declares, so a queue's size
        // is a property of this driver rather than of the emulator.
        let size = if offered_size < wanted { offered_size } else { wanted };
        common.write16(common::QUEUE_SIZE, size)?;

        let notify_off = common.read16(common::QUEUE_NOTIFY_OFF)?;
        let doorbell = u32::from(notify_off).saturating_mul(notify_multiplier);

        Ok(Self { common, notify, isr, doorbell, size })
    }

    /// How many descriptors the control queue has. Unit: descriptors.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Give the device the control queue's three ring addresses, in its own
    /// address space.
    ///
    /// Every one of them is a [`Region::device_at`](f_ring::device::Region::device_at)
    /// answer — which is [`Domains::map`](f_ring::registry::Domains::map)'s
    /// answer, which is the frame's — so a driver cannot point a device at an
    /// address of its own devising without the frame having translated it first.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a window that cannot hold the register.
    pub fn queue_at(&self, desc: u64, driver: u64, device: u64) -> Result<(), Trouble> {
        self.common.write16(common::QUEUE_SELECT, crate::queue::index::CONTROL)?;
        self.common.write64(common::QUEUE_DESC, desc)?;
        self.common.write64(common::QUEUE_DRIVER, driver)?;
        self.common.write64(common::QUEUE_DEVICE, device)?;
        Ok(())
    }

    /// Enable the control queue and tell the device the driver is ready.
    ///
    /// One queue and not two: the cursor queue is left disabled, which is legal
    /// — the specification requires a driver to enable every queue it means to
    /// use and says nothing about the rest — and is what
    /// `crate::queue::index::CURSOR` costs.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn run(&self) -> Result<(), Trouble> {
        self.common.write16(common::QUEUE_SELECT, crate::queue::index::CONTROL)?;
        self.common.write16(common::QUEUE_ENABLE, 1)?;
        self.common.write8(
            common::DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        Ok(())
    }

    /// Ring the control queue's doorbell.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] for a notification window too short for the
    /// doorbell the device's own capability placed there — which is a device
    /// describing itself inconsistently, and is refused rather than written
    /// somewhere nearby.
    pub fn kick(&self) -> Result<(), Trouble> {
        self.notify.write16(self.doorbell, crate::queue::index::CONTROL)?;
        Ok(())
    }

    /// Read the interrupt-status register.
    ///
    /// Called once per turn of the wait for a command's answer, and the reason
    /// is not the value: reading a device register is an exit to the emulator,
    /// which is a point at which the device's own work can make progress. A poll
    /// that only read memory would be a poll the emulator never got a chance to
    /// answer.
    ///
    /// It is also the register this driver will wait on once it has its
    /// interrupt. The manifest declares an `irq` need and says why the reversal
    /// is *further* away here than on either other driver: what an interrupt
    /// buys a display driver is idleness between frames rather than liveness,
    /// because every command it sends is owed an answer.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`].
    pub fn poke(&self) -> Result<u8, Trouble> {
        Ok(self.isr.read8(0)?)
    }

    /// Put the device back in reset, destroying every resource it holds and
    /// blanking the screen.
    ///
    /// **Not a teardown.** The module comment says at length why this driver has
    /// no `stop` and why leaving a display controller running is both safe and
    /// necessary; this method exists for the one case where the alternative is
    /// worse than a blank screen, and `crate::driver::Driver::sequence` is its
    /// only caller. A command failed between the attach and the detach and the
    /// detach failed too, so the display is still holding a client's buffer and
    /// nothing else will make it let go. A client told its memory is its own
    /// again in that state has been told something false, and this is what makes
    /// it true.
    ///
    /// It is spelled `reset` rather than `stop` deliberately: `stop` is a word
    /// somebody reaches for in a teardown, and this must never be reached for
    /// there.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`]. Its one caller ignores the answer, because it is
    /// already on a path where something has failed and a failed reset leaves
    /// nothing further a component can do — `crate::driver::Counters::halted` is
    /// what a boot reads instead.
    pub fn reset(&self) -> Result<(), Trouble> {
        self.common.write8(common::DEVICE_STATUS, 0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::QUEUE_SIZE;

    /// A device's common configuration structure, in memory.
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
    /// this crate's.
    /// How many bytes of it the fixture holds.
    ///
    /// Sixty-four, which is [`common::EXTENT`] rounded up to something a reader
    /// can check without a calculator. Unit: bytes.
    const DEVICE_BYTES: u32 = 64;

    const _: () = assert!(common::EXTENT <= DEVICE_BYTES);

    #[repr(align(64))]
    struct Device([u8; DEVICE_BYTES as usize]);

    impl Device {
        /// A device that would pass the handshake, before a test spoils one
        /// field of it.
        ///
        /// Built as the *working* case and then broken per test, rather than
        /// built per test from nothing, because a fixture assembled differently
        /// for each refusal is a fixture where the refusal may be coming from
        /// the assembly.
        fn healthy() -> Self {
            let mut device = Self([0; DEVICE_BYTES as usize]);
            device.put32(common::DEVICE_FEATURE, FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM);
            device.put16(common::NUM_QUEUES, 1);
            device.put16(common::QUEUE_SIZE, QUEUE_SIZE);
            device.put16(common::QUEUE_NOTIFY_OFF, 3);
            // Not a value any device would publish, and that is the point: the
            // driver writes this register before it reads the queue's size, so a
            // handshake that had skipped the write would read a size out of the
            // register bank of a queue it never selected. Left as something no
            // queue index is, so the assertion that it became zero is an
            // assertion about a write rather than about an array that started
            // zeroed.
            device.put16(common::QUEUE_SELECT, u16::MAX);
            device
        }

        fn put8(&mut self, at: u32, value: u8) {
            self.0[at as usize] = value;
        }

        fn put16(&mut self, at: u32, value: u16) {
            let at = at as usize;
            self.0[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }

        fn put32(&mut self, at: u32, value: u32) {
            let at = at as usize;
            self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn get16(&self, at: u32) -> u16 {
            let at = at as usize;
            u16::from_le_bytes([self.0[at], self.0[at + 1]])
        }

        fn get32(&self, at: u32) -> u32 {
            let at = at as usize;
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&self.0[at..at + 4]);
            u32::from_le_bytes(bytes)
        }

        fn get8(&self, at: u32) -> u8 {
            self.0[at as usize]
        }

        /// The four windows, all over the same bytes.
        ///
        /// One window and not four, because `open` touches only the common
        /// configuration structure — which is itself worth asserting, and is
        /// what this shape asserts: a handshake that had reached for the
        /// notification structure or the interrupt-status register would be
        /// reading and writing the common structure's own fields here, and every
        /// assertion below would move.
        fn windows(&mut self, len: u32) -> Windows {
            let base = self.0.as_mut_ptr() as usize as u64;
            let window = Window::at(base, len).expect("a window over an aligned array");
            Windows {
                common: window,
                notify: window,
                isr: window,
                config: window,
                notify_multiplier: 4,
            }
        }
    }

    #[test]
    fn a_device_that_does_not_offer_platform_addressing_is_refused() {
        // **The refusal this whole driver rests on, observed rather than
        // claimed.** `kernel/src/arch/x86_64/dma.rs` records what it cost to
        // find out that a device without feature bit 33 addresses physical
        // memory by specification: every isolation test passes, for the wrong
        // reason. Neither `user/virtio-blk` nor `user/virtio-net` has a test
        // that watches the refusal happen — the emulator always offers the bit,
        // so their boots cannot reach it — and this is the third driver making
        // the refusal a thing that is checked rather than a thing that is
        // written down.
        let mut device = Device::healthy();
        device.put32(common::DEVICE_FEATURE, FEATURE_VERSION_1);
        assert_eq!(
            Transport::open(device.windows(DEVICE_BYTES), QUEUE_SIZE).map(|_| ()),
            Err(Trouble::NoPlatformAddressing),
        );

        // And a device offering neither bit, which is the legacy case: the same
        // refusal, because the driver asks for both and refuses on the pair
        // rather than on either.
        let mut legacy = Device::healthy();
        legacy.put32(common::DEVICE_FEATURE, 0);
        assert_eq!(
            Transport::open(legacy.windows(DEVICE_BYTES), QUEUE_SIZE).map(|_| ()),
            Err(Trouble::NoPlatformAddressing),
        );
    }

    #[test]
    fn the_driver_accepts_the_two_bits_it_asked_for_and_nothing_it_was_offered() {
        // The other half of the same property. A device that offers everything
        // is the case where a driver that wrote back what it was *offered*
        // rather than what it *asked for* would look identical on a boot — it
        // would negotiate features nothing in this crate implements and behave
        // differently depending on what the emulator was built with, which is
        // the thing `crate::transport`'s module comment refuses.
        let mut device = Device::healthy();
        device.put32(common::DEVICE_FEATURE, u32::MAX);
        let transport =
            Transport::open(device.windows(DEVICE_BYTES), QUEUE_SIZE).expect("a healthy device");
        assert_eq!(transport.size(), QUEUE_SIZE);

        assert_eq!(
            device.get32(common::DRIVER_FEATURE),
            FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM,
            "the driver accepted something it was offered rather than what it asked for",
        );
        assert_eq!(device.get32(common::DRIVER_FEATURE_SELECT), 1, "in the upper feature word");
        // And the device was left ready to be given queue addresses and not yet
        // running: `open` and `run` are two calls for that reason, and a status
        // register carrying `DRIVER_OK` here would be a device already able to
        // take work from a queue whose address registers still hold zero.
        assert_eq!(
            device.get8(common::DEVICE_STATUS),
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
        assert_eq!(device.get16(common::QUEUE_SELECT), crate::queue::index::CONTROL);
    }

    #[test]
    fn a_queue_larger_than_the_layout_is_shrunk_and_a_smaller_one_is_taken_as_it_is() {
        // The specification's one constraint on this write is *never larger than
        // the device offered*, and the layout's is *never larger than the region
        // holds*. A driver that honoured only the first would lay three rings
        // out past the end of its own grant.
        let mut roomy = Device::healthy();
        roomy.put16(common::QUEUE_SIZE, 256);
        let transport =
            Transport::open(roomy.windows(DEVICE_BYTES), QUEUE_SIZE).expect("a healthy device");
        assert_eq!(transport.size(), QUEUE_SIZE);
        assert_eq!(roomy.get16(common::QUEUE_SIZE), QUEUE_SIZE, "the device was told the smaller");

        let mut cramped = Device::healthy();
        cramped.put16(common::QUEUE_SIZE, 8);
        let transport =
            Transport::open(cramped.windows(DEVICE_BYTES), QUEUE_SIZE).expect("a healthy device");
        assert_eq!(transport.size(), 8, "a device that offers less is believed");
    }

    #[test]
    fn a_device_with_no_control_queue_or_one_too_small_is_refused() {
        // R04 at two numbers a device chose. A display controller defines a
        // control queue and a cursor queue; a device answering zero is not one,
        // and a queue that cannot hold a command and its answer is not a queue
        // this driver can send anything on.
        let mut none = Device::healthy();
        none.put16(common::NUM_QUEUES, 0);
        assert_eq!(
            Transport::open(none.windows(DEVICE_BYTES), QUEUE_SIZE).map(|_| ()),
            Err(Trouble::NoQueue),
        );

        let mut tiny = Device::healthy();
        tiny.put16(common::QUEUE_SIZE, 1);
        assert_eq!(
            Transport::open(tiny.windows(DEVICE_BYTES), QUEUE_SIZE).map(|_| ()),
            Err(Trouble::NoQueue),
        );
    }

    #[test]
    fn a_window_too_short_for_the_structure_is_refused_before_anything_is_written() {
        // The frame routes a window and the device describes a structure inside
        // it; a window shorter than the structure is those two disagreeing, and
        // it is refused rather than written into. Before anything is written,
        // which is what the second assertion checks: a handshake that had reset
        // the device and *then* refused would leave a device somebody else may
        // be driving in a state nothing describes.
        let mut device = Device::healthy();
        device.put8(common::DEVICE_STATUS, 0xAB);
        let short = common::EXTENT - 1;
        assert_eq!(
            Transport::open(device.windows(short), QUEUE_SIZE).map(|_| ()),
            Err(Trouble::Register(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::BAD_ADDRESS,
            ))),
        );
        assert_eq!(device.get8(common::DEVICE_STATUS), 0xAB, "the device was touched anyway");
    }

    #[test]
    fn the_doorbell_is_where_the_device_said_it_would_be() {
        // Two words the *device* published, multiplied. This driver rings one
        // doorbell and would work with the multiplier read as zero — queue zero
        // is at offset zero whatever it is — which is exactly why the arithmetic
        // is checked here rather than left to a boot: the first thing that
        // touches the cursor queue would find the field had never been read.
        let mut device = Device::healthy();
        let transport =
            Transport::open(device.windows(DEVICE_BYTES), QUEUE_SIZE).expect("a healthy device");
        // `healthy` publishes a notification offset of three and the windows
        // carry a multiplier of four, so the doorbell is twelve bytes in — and
        // the notification window here is the same array, so ringing it writes
        // the queue index there.
        transport.kick().expect("a doorbell inside the window");
        assert_eq!(device.get16(12), crate::queue::index::CONTROL);
    }
}
