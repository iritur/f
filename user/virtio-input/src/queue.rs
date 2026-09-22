// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One split virtqueue, written into a region the component was granted — and
//! the first one in this tree that only ever runs the other way.
//!
//! # What is the same as the other three drivers' queues
//!
//! Nearly all of it. Descriptors, an available ring and a used ring, over a
//! [`Region`] — memory this component holds a capability for and the frame has
//! given the device a translation for. Every address that reaches a descriptor
//! comes from [`Region::device_at`], so the only addresses this driver can put in
//! front of a device are addresses inside grants it was given. There is no type
//! in this crate that turns a `Region` into a slice, which is what keeps *this
//! component cannot address memory outside its grant* a property of the types
//! rather than of the arithmetic.
//!
//! That paragraph is `user/virtio-blk`'s, `user/virtio-net`'s and
//! `user/virtio-gpu`'s, and it survives a fourth time unedited. What follows is
//! the part that does not.
//!
//! # A queue that is refilled rather than drained
//!
//! The three drivers before this one post a chain **because they want
//! something**: a sector, a frame transmitted, a display command answered. The
//! chain is outstanding until the device answers it, and an empty queue is a
//! driver with nothing outstanding — the ordinary resting state.
//!
//! virtio-input's event queue is the opposite at every point. Every descriptor
//! this driver posts is device-**writable** and describes eight bytes of this
//! component's own memory; the device fills one whenever the user does
//! something, which may be never; and the resting state is the queue **full** of
//! posted buffers. So [`Queue::post`] is called at start-up for every slot and
//! then once per record harvested, and a queue that is not full is a queue that
//! has fallen behind the user.
//!
//! [`Queue::posted`] is what makes that visible rather than assumed. The network
//! driver's receive half is the nearest thing in the tree and it is still not the
//! same: there, a receive buffer belongs to a *client* and RFC 0024's reclaim
//! decides what happens to it. Here every buffer is this component's own, which
//! is why there is no registration in this file and no `Reach` anywhere in this
//! crate.
//!
//! # Why the buffers are in the same region as the rings
//!
//! Because the alternative is a second grant for 512 bytes. The region the
//! manifest routes is untyped memory this component splits as it likes, the
//! records are eight bytes each and there are [`QUEUE_SIZE`] of them, and putting
//! them past the used ring in the same region costs one constant and no manifest
//! field. What it costs a *reader* is that the region now has four things in it
//! rather than three, which is why [`BUFFERS_AT`] is asserted against the ring
//! above it rather than described in prose.
//!
//! *What would reverse this:* a record width the device chooses. virtio-input's
//! is fixed by the specification at eight bytes, so the buffer area's size is a
//! constant here; a device family whose record width came out of its
//! configuration space would need the split decided at run time, and then the
//! buffers want their own region so that a wrong answer is a refusal rather than
//! a ring overwritten from below.

use f_ring::device::Region;

use crate::Trouble;

/// This descriptor is not the last of its chain.
///
/// Declared and unused: every chain this driver posts is one descriptor long,
/// because a `virtio_input_event` is eight bytes and there is nothing to split.
/// It is here so that a reader comparing this file with the other three finds
/// the same two flags with the same names, and so that the day a status-queue
/// chain needs two descriptors the constant is not invented again.
pub const DESC_NEXT: u16 = 1;

/// The *device* writes this descriptor's buffer.
///
/// Set on every descriptor this driver posts, which is the sentence that makes
/// this queue the one that runs the other way.
pub const DESC_WRITE: u16 = 2;

/// Bytes in one descriptor: address, length, flags, link.
/// Unit: bytes.
const DESC_BYTES: u32 = 16;

/// Where the available ring sits inside the queue's region. Unit: bytes.
const AVAIL_AT: u32 = 2048;

/// Where the used ring sits inside the queue's region. Unit: bytes.
const USED_AT: u32 = 4096;

/// Where the event buffers sit inside the queue's region.
///
/// Past the used ring, with the assertions below rather than a comment saying it
/// fits. Unit: bytes.
pub const BUFFERS_AT: u32 = 6144;

/// Bytes in one `virtio_input_event`: a type, a code and a value.
///
/// Fixed by the specification and not negotiated, which is what lets the buffer
/// area be a constant. Unit: bytes.
pub const RECORD_BYTES: u32 = 8;

/// How large a region the layout needs. Unit: bytes.
pub const QUEUE_BYTES: u32 = 8192;

/// How many descriptors that layout holds, and therefore how many records the
/// device can write before this driver has looked.
///
/// Sixty-four, which is the other three drivers' number and is doing different
/// work here: on them it is a bound on concurrency, and on this queue it is the
/// **depth of the buffer between the user and the loop**. A user producing
/// events faster than [`Queue::post`] refills is a user whose events the device
/// has nowhere to put, and the device drops them — outside this component, where
/// nothing here can count them. That is the one loss on this path this driver
/// cannot report, and it is named here rather than left to be discovered.
///
/// *What would reverse this:* a measurement. A mouse at one thousand reports a
/// second against a loop that turns far faster than that is not close to the
/// bound; a device that batches, or a frame that stops scheduling this component
/// for a whole display refresh, is. `E3-P01`'s rig is what would say which.
/// Unit: descriptors.
pub const QUEUE_SIZE: u16 = 64;

const _: () = assert!(DESC_BYTES * (QUEUE_SIZE as u32) <= AVAIL_AT);
const _: () = assert!(AVAIL_AT + 6 + 2 * (QUEUE_SIZE as u32) <= USED_AT);
const _: () = assert!(USED_AT + 6 + 8 * (QUEUE_SIZE as u32) <= BUFFERS_AT);
const _: () = assert!(BUFFERS_AT + RECORD_BYTES * (QUEUE_SIZE as u32) <= QUEUE_BYTES);

/// Which virtqueue of a virtio-input device this is.
pub mod index {
    /// The queue the device writes events into. The only one this driver uses.
    /// Unit: none — a queue index.
    pub const EVENT: u16 = 0;

    /// The queue a driver writes *to* the device on, which this driver **does
    /// not use** and does not enable.
    ///
    /// Named rather than omitted so that a reader looking for it finds the
    /// decision instead of an absence. What it carries is the same
    /// `virtio_input_event` record in the other direction, and what that is for
    /// is state the device holds on the driver's behalf: keyboard LEDs
    /// (`EV_LED`), the bell (`EV_SND`), and force feedback (`EV_FF`). So the cost
    /// is exact and small: caps lock does not light up, and a keyboard's
    /// indicator lamps say whatever they said when the machine booted.
    ///
    /// It is not implemented because nothing in this tree decides what a lamp
    /// should say — that is a state a compositor or a keymap owns, and neither
    /// exists yet — and a driver that could write a queue nobody had a reason to
    /// write would be a second doorbell with no caller. The day something does,
    /// the cost is one more queue in `crate::driver` and one more doorbell in
    /// `crate::transport`, both of which the shape already affords.
    /// Unit: none — a queue index.
    pub const STATUS: u16 = 1;
}

/// One split virtqueue over granted memory.
#[derive(Clone, Copy, Debug)]
pub struct Queue {
    region: Region,
    /// Descriptors in the ring. Unit: descriptors; a power of two, so the two
    /// cursors below are masked rather than divided.
    size: u16,
    /// How many buffers this driver has offered. Unit: buffers, wrapping, which
    /// is what the available ring's index is.
    published: u16,
    /// How many the device has filled and this driver has taken.
    /// Unit: buffers, wrapping.
    seen: u16,
}

/// One element of a used ring: which buffer, and how many bytes the device says
/// it wrote into it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Filled {
    /// The descriptor the device is giving back.
    /// Unit: none — a descriptor index. **A device's word**, and the caller
    /// checks it.
    pub head: u16,
    /// How many bytes the device says it wrote. Unit: bytes.
    ///
    /// Acted on rather than merely checked, which is the difference between this
    /// and the display driver's identical-looking field: a device that wrote
    /// fewer bytes than one record wrote *part* of a record, and the fields this
    /// driver would read out of the rest are whatever was in the buffer from the
    /// last time round. [`Trouble::ShortUsed`] is what that earns.
    pub written: u32,
}

impl Queue {
    /// Lay the event virtqueue, of `size` descriptors, out in `region`.
    ///
    /// The region must be zeroed, and that is the caller's obligation rather
    /// than this function's work: a component's memory arrives zeroed from the
    /// frame, and zeroing it again here would be a driver that cannot tell the
    /// difference between memory it was given and memory it found.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a size that is not a power of two, is zero, or
    /// does not fit [`QUEUE_BYTES`]; and for a region shorter than the layout.
    pub const fn over(region: Region, size: u16) -> Result<Self, Trouble> {
        if size == 0 || !size.is_power_of_two() || size > QUEUE_SIZE {
            return Err(Trouble::Layout);
        }
        if region.len() < QUEUE_BYTES {
            return Err(Trouble::Layout);
        }
        Ok(Self { region, size, published: 0, seen: 0 })
    }

    /// How many descriptors it has. Unit: descriptors.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Where the device addresses the descriptor table.
    /// Unit: bytes, in the device's address space.
    ///
    /// # Errors
    ///
    /// [`Trouble::Register`] carrying `ARGUMENT/BAD_ADDRESS`, which
    /// [`Queue::over`] has already made unreachable.
    pub const fn device_desc(&self) -> Result<u64, Trouble> {
        match self.region.device_at(0) {
            Ok(at) => Ok(at),
            Err(refused) => Err(Trouble::Register(refused)),
        }
    }

    /// Where the device addresses the available ring.
    /// Unit: bytes, in the device's address space.
    ///
    /// # Errors
    ///
    /// As [`Queue::device_desc`].
    pub const fn device_avail(&self) -> Result<u64, Trouble> {
        match self.region.device_at(AVAIL_AT) {
            Ok(at) => Ok(at),
            Err(refused) => Err(Trouble::Register(refused)),
        }
    }

    /// Where the device addresses the used ring.
    /// Unit: bytes, in the device's address space.
    ///
    /// # Errors
    ///
    /// As [`Queue::device_desc`].
    pub const fn device_used(&self) -> Result<u64, Trouble> {
        match self.region.device_at(USED_AT) {
            Ok(at) => Ok(at),
            Err(refused) => Err(Trouble::Register(refused)),
        }
    }

    /// Where the record for descriptor `index` sits inside the region.
    /// Unit: bytes, from the start of the region.
    const fn record_at(index: u16) -> u32 {
        BUFFERS_AT + (index as u32) * RECORD_BYTES
    }

    /// Give the device one buffer to write the next record into.
    ///
    /// One descriptor, [`DESC_WRITE`], eight bytes of this component's own
    /// memory, and the available ring published with a `Release` store —
    /// [`Region::publish16`] — which is the same discipline the ring itself rests
    /// on and is load-bearing for the same reason one reader further away: a
    /// device has a weaker relationship to this core's store buffer than another
    /// core does.
    ///
    /// The address comes from [`Region::device_at`] and there is no other source
    /// in this crate. That is the whole of why this driver cannot point the
    /// device at memory it was not granted — and it matters more on this queue
    /// than on any other in the tree, because what an unrefused escape produces
    /// here is a device *writing* into somebody else's memory at a moment
    /// nothing in this system chose, for as long as the buffer stays posted.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for an index past the queue.
    pub fn post(&mut self, index: u16) -> Result<(), Trouble> {
        if index >= self.size {
            return Err(Trouble::Layout);
        }
        let at = self.region.device_at(Self::record_at(index))?;
        let base = u32::from(index).saturating_mul(DESC_BYTES);
        self.region.put64(base, at)?;
        self.region.put32(base + 8, RECORD_BYTES)?;
        self.region.put16(base + 12, DESC_WRITE)?;
        self.region.put16(base + 14, 0)?;

        let slot = self.published & (self.size - 1);
        let ring = AVAIL_AT + 4 + u32::from(slot).saturating_mul(2);
        self.region.put16(ring, index)?;
        self.published = self.published.wrapping_add(1);
        self.region.publish16(AVAIL_AT + 2, self.published)?;
        Ok(())
    }

    /// Take one filled buffer, if the device has published one.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a used element whose id cannot be a descriptor
    /// index, which is a device describing a buffer that cannot exist — refused
    /// rather than truncated into a plausible slot, and refused **before** the
    /// cursor moves so that a second reader can still see what the device wrote.
    pub fn harvest(&mut self) -> Result<Option<Filled>, Trouble> {
        let published = self.region.consume16(USED_AT + 2)?;
        if published == self.seen {
            return Ok(None);
        }
        let slot = self.seen & (self.size - 1);
        let at = USED_AT + 4 + u32::from(slot).saturating_mul(8);
        let head = self.region.get32(at)?;
        let written = self.region.get32(at + 4)?;
        let Ok(head) = u16::try_from(head) else { return Err(Trouble::Layout) };
        if head >= self.size {
            return Err(Trouble::Layout);
        }
        self.seen = self.seen.wrapping_add(1);
        Ok(Some(Filled { head, written }))
    }

    /// Read the eight bytes the device wrote into `index`.
    ///
    /// `(type, code, value)`, little-endian, which is the whole of a
    /// `virtio_input_event`. Read field by field out of the region rather than
    /// cast from it, for the reason `abi/src/input.rs` gives about its own
    /// records: the bytes came from a peer, and a cast is a reader that has
    /// already believed them.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for an index past the queue, and
    /// [`Trouble::Register`] for a region that cannot hold the record — both of
    /// which [`Queue::over`]'s length check has already made unreachable, and
    /// both of which are still checked because a device's `head` is what chooses
    /// the index.
    pub fn record(&self, index: u16) -> Result<(u16, u16, u32), Trouble> {
        if index >= self.size {
            return Err(Trouble::Layout);
        }
        let at = Self::record_at(index);
        Ok((self.region.get16(at)?, self.region.get16(at + 2)?, self.region.get32(at + 4)?))
    }

    /// Buffers the device is holding and has not filled. Unit: buffers.
    ///
    /// The resting state of this queue is this number equal to [`Queue::size`],
    /// which is the opposite of every other queue in this tree. A driver reading
    /// a small number here is a driver that has fallen behind the user.
    #[must_use]
    pub const fn posted(&self) -> u16 {
        self.published.wrapping_sub(self.seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight kibibytes at a descriptor's alignment, which is what a queue needs
    /// and what a `[u8; N]` on the stack does not promise.
    #[repr(align(16))]
    struct Owned([u8; QUEUE_BYTES as usize]);

    /// Where the device addresses this test's region.
    ///
    /// Deliberately not the component's own base, so that a test that confused
    /// the two produces an address no assertion here matches.
    const DEVICE_AT: u64 = 0x4000_0000;

    impl Owned {
        const fn new() -> Self {
            Self([0; QUEUE_BYTES as usize])
        }

        fn region(&mut self) -> Region {
            Region::at(self.0.as_mut_ptr() as usize as u64, DEVICE_AT, QUEUE_BYTES)
                .expect("an aligned region")
        }

        /// The device's half: fill the buffer `head` names and publish a used
        /// element for it.
        ///
        /// Everything a real device would do and nothing this driver does, which
        /// is what makes the tests below tests of the driver rather than of
        /// themselves.
        fn device_fills(&mut self, seen: u16, head: u16, record: (u16, u16, u32), written: u32) {
            let at = (BUFFERS_AT + u32::from(head) * RECORD_BYTES) as usize;
            self.0[at..at + 2].copy_from_slice(&record.0.to_le_bytes());
            self.0[at + 2..at + 4].copy_from_slice(&record.1.to_le_bytes());
            self.0[at + 4..at + 8].copy_from_slice(&record.2.to_le_bytes());

            let slot = (seen & (QUEUE_SIZE - 1)) as u32;
            let element = (USED_AT + 4 + slot * 8) as usize;
            self.0[element..element + 4].copy_from_slice(&u32::from(head).to_le_bytes());
            self.0[element + 4..element + 8].copy_from_slice(&written.to_le_bytes());
            let published = (USED_AT + 2) as usize;
            self.0[published..published + 2].copy_from_slice(&seen.wrapping_add(1).to_le_bytes());
        }

        /// What the driver wrote into descriptor `index`: address, length,
        /// flags.
        fn descriptor(&self, index: u16) -> (u64, u32, u16) {
            let base = (u32::from(index) * DESC_BYTES) as usize;
            let mut address = [0u8; 8];
            address.copy_from_slice(&self.0[base..base + 8]);
            let mut len = [0u8; 4];
            len.copy_from_slice(&self.0[base + 8..base + 12]);
            let mut flags = [0u8; 2];
            flags.copy_from_slice(&self.0[base + 12..base + 14]);
            (u64::from_le_bytes(address), u32::from_le_bytes(len), u16::from_le_bytes(flags))
        }
    }

    #[test]
    fn a_geometry_that_is_not_a_queue_is_refused() {
        let mut owned = Owned::new();
        let base = owned.0.as_mut_ptr() as usize as u64;
        let region = owned.region();

        assert_eq!(Queue::over(region, 0).map(|_| ()), Err(Trouble::Layout), "no descriptors");
        assert_eq!(Queue::over(region, 3).map(|_| ()), Err(Trouble::Layout), "not a power of two");
        assert_eq!(
            Queue::over(region, QUEUE_SIZE * 2).map(|_| ()),
            Err(Trouble::Layout),
            "past what the layout holds"
        );

        let short = Region::at(base, DEVICE_AT, QUEUE_BYTES - 16).expect("an aligned region");
        assert_eq!(
            Queue::over(short, QUEUE_SIZE).map(|_| ()),
            Err(Trouble::Layout),
            "a region the rings and the buffers do not fit in"
        );
    }

    #[test]
    fn every_posted_descriptor_is_device_writable_and_inside_the_grant() {
        // The property this file exists for, and the one whose absence would be
        // a device writing into memory this component was never granted. Both
        // halves are checked: the flag, because a descriptor without it is one
        // the device reads instead and no event ever arrives; and the address,
        // because it must be the region's own device address and nothing else.
        let mut owned = Owned::new();
        let region = owned.region();
        let mut queue = Queue::over(region, QUEUE_SIZE).expect("a queue");
        for index in 0..QUEUE_SIZE {
            queue.post(index).expect("a slot inside the queue");
        }
        assert_eq!(queue.posted(), QUEUE_SIZE, "the resting state of this queue is full");

        for index in 0..QUEUE_SIZE {
            let (address, len, flags) = owned.descriptor(index);
            assert_eq!(len, RECORD_BYTES);
            assert_eq!(flags & DESC_WRITE, DESC_WRITE, "the device writes this buffer");
            assert_eq!(flags & DESC_NEXT, 0, "one record is one descriptor");
            let expected = DEVICE_AT + u64::from(BUFFERS_AT + u32::from(index) * RECORD_BYTES);
            assert_eq!(address, expected, "a buffer address is the region's, offset");
            assert!(
                address >= DEVICE_AT
                    && address + u64::from(len) <= DEVICE_AT + u64::from(QUEUE_BYTES),
                "no descriptor may name an address outside the grant"
            );
        }

        assert_eq!(queue.post(QUEUE_SIZE), Err(Trouble::Layout), "past the ring");
    }

    #[test]
    fn a_record_the_device_wrote_is_read_back_field_by_field() {
        let mut owned = Owned::new();
        let region = owned.region();
        let mut queue = Queue::over(region, QUEUE_SIZE).expect("a queue");
        queue.post(0).expect("a slot");
        queue.post(1).expect("a slot");

        owned.device_fills(0, 1, (0x0002, 0x0001, 0xFFFF_FFFE), RECORD_BYTES);
        let filled = queue.harvest().expect("a legal element").expect("one element");
        assert_eq!(filled, Filled { head: 1, written: RECORD_BYTES });
        assert_eq!(queue.record(1).expect("inside the queue"), (0x0002, 0x0001, 0xFFFF_FFFE));
        assert_eq!(queue.posted(), 1, "one buffer taken, one still with the device");
        assert!(queue.harvest().expect("a legal ring").is_none(), "and nothing behind it");
    }

    #[test]
    fn a_used_element_naming_a_buffer_outside_the_queue_is_refused() {
        // A device steering the driver. Refused rather than masked into a
        // plausible slot, because a driver that followed it would read eight
        // bytes it never gave the device and submit the result as something the
        // user did.
        let mut owned = Owned::new();
        let region = owned.region();
        let mut queue = Queue::over(region, 8).expect("a queue");
        queue.post(0).expect("a slot");
        owned.device_fills(0, 9, (1, 1, 1), RECORD_BYTES);
        assert_eq!(queue.harvest(), Err(Trouble::Layout));
    }

    #[test]
    fn a_buffer_is_reposted_at_the_index_the_device_gave_back() {
        // The refill, which is the whole shape of this queue: a driver that
        // harvested and did not repost would run out of buffers in
        // `QUEUE_SIZE` records and then see nothing more, with no error
        // anywhere.
        let mut owned = Owned::new();
        let region = owned.region();
        let mut queue = Queue::over(region, QUEUE_SIZE).expect("a queue");
        for index in 0..QUEUE_SIZE {
            queue.post(index).expect("a slot");
        }
        for round in 0..QUEUE_SIZE {
            owned.device_fills(round, round, (2, 0, u32::from(round)), RECORD_BYTES);
            let filled = queue.harvest().expect("a legal element").expect("one element");
            assert_eq!(filled.head, round);
            assert_eq!(queue.record(filled.head).expect("inside").2, u32::from(round));
            queue.post(filled.head).expect("reposted");
            assert_eq!(queue.posted(), QUEUE_SIZE, "full again after every refill");
        }
    }
}
