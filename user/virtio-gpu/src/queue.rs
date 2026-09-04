// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One split virtqueue, written into a region the component was granted.
//!
//! # What is the same as the other two drivers' queues, and why that is the
//! result
//!
//! Nearly all of it. Descriptors, an available ring and a used ring, over a
//! [`Region`] — memory this component holds a capability for and the frame has
//! given the device a translation for. Every address that reaches a descriptor
//! comes from [`Region::device_at`] or from a
//! [`Reach`](f_ring::registry::Reach) a registration answered, so the only
//! addresses this driver can put in front of a device are addresses inside
//! grants it was given. There is no type in this crate that turns either into a
//! slice, which is the mechanism behind *zero copies on the data path*: the
//! operation that would copy has nothing to copy from.
//!
//! `user/virtio-blk/src/queue.rs` says that paragraph and
//! `user/virtio-net/src/queue.rs` says it again. That it can be said a third
//! time, for a device of a different kind, without editing `ring/src/device.rs`
//! is the finding E1-B04 exists to produce. What follows is the part that is
//! *not* the same.
//!
//! # One difference, and it is the device's kind rather than its direction
//!
//! E1-B03's three differences were all the receive direction: a queue that knows
//! its index, a completion that says which chain it was, and a head chosen by
//! arithmetic rather than by an allocator. **None of them applies here**, and
//! that is worth stating because it is the first evidence that the second
//! driver's differences were about *receiving* rather than about *being a second
//! driver*. This driver has one queue, one chain outstanding at a time, and a
//! head that is a constant — which is `user/virtio-blk`'s shape and not
//! `user/virtio-net`'s.
//!
//! What is new is what the descriptors *carry*. A block chain is a header, a
//! payload and a status byte; a network chain is a header and a frame. A display
//! chain is a **command and its answer**, and both of them live in this
//! component's own control page: the pixels are not in the chain at all. They
//! reach the device through an address inside one of the commands, which the
//! device keeps and reads later. So a reader looking for the client's bytes in a
//! descriptor will not find them, and that is the mechanism rather than an
//! accident — [`crate::driver`] is where the address goes in and where it is
//! taken back out again.
//!
//! # Why the layout is a constant rather than a computation
//!
//! The three rings have separate address registers in the modern transport, so
//! where they sit inside one region is this file's choice and not the
//! specification's. Round numbers mean a reader can check the arithmetic without
//! a calculator, and the three assertions below are what keep the choice honest.
//! The numbers are the other two drivers' numbers and they are still not shared,
//! for the reason `crate::transport`'s `common` module states about a larger
//! duplication: what is actually shared between three drivers is the whole of
//! the split-virtqueue discipline, and moving three offsets would leave the
//! discipline duplicated while making it look shared. RFC 0054.

use f_ring::device::Region;

use crate::Trouble;

/// This descriptor is not the last of its chain.
pub const DESC_NEXT: u16 = 1;

/// The *device* writes this descriptor's buffer.
pub const DESC_WRITE: u16 = 2;

/// Bytes in one descriptor: address, length, flags, link.
/// Unit: bytes.
const DESC_BYTES: u32 = 16;

/// Where the available ring sits inside the queue's region. Unit: bytes.
const AVAIL_AT: u32 = 2048;

/// Where the used ring sits inside the queue's region. Unit: bytes.
const USED_AT: u32 = 4096;

/// How large a region the layout needs. Unit: bytes.
pub const QUEUE_BYTES: u32 = 8192;

/// How many descriptors that layout holds. Unit: descriptors.
pub const QUEUE_SIZE: u16 = 64;

const _: () = assert!(DESC_BYTES * (QUEUE_SIZE as u32) <= AVAIL_AT);
const _: () = assert!(AVAIL_AT + 6 + 2 * (QUEUE_SIZE as u32) <= USED_AT);
const _: () = assert!(USED_AT + 6 + 8 * (QUEUE_SIZE as u32) <= QUEUE_BYTES);

/// Which virtqueue of a virtio-gpu device this is.
pub mod index {
    /// The queue every display command travels on.
    /// Unit: none — a queue index.
    pub const CONTROL: u16 = 0;

    /// The cursor queue, which this driver **does not use** and does not
    /// enable.
    ///
    /// Named rather than omitted so that a reader looking for it finds the
    /// decision instead of an absence. What it costs is exact and is the one
    /// omission in this crate a user would notice immediately: there is no
    /// hardware cursor. A system that wanted a pointer would have to composite
    /// it into the scanout, which means a transfer and a flush of the whole
    /// frame every time the pointer moves — the cursor queue exists precisely so
    /// that a pointer costs one small command instead. It is not implemented
    /// because nothing in this tree draws a pointer, and the day something does
    /// the cost is a second queue in `crate::driver` and a second doorbell in
    /// `crate::transport`, both of which the shape already affords.
    /// Unit: none — a queue index.
    pub const CURSOR: u16 = 1;
}

/// One split virtqueue over granted memory.
#[derive(Clone, Copy, Debug)]
pub struct Queue {
    region: Region,
    /// Descriptors in the ring. Unit: descriptors; a power of two, so the two
    /// cursors below are masked rather than divided.
    size: u16,
    /// How many chains this driver has offered. Unit: chains, wrapping, which
    /// is what the available ring's index is.
    published: u16,
    /// How many completions it has taken. Unit: chains, wrapping.
    seen: u16,
}

/// One element of a used ring: which chain, and how many bytes the device says
/// it wrote into it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Finished {
    /// The head descriptor of the chain the device is giving back.
    /// Unit: none — a descriptor index. **A device's word**, and the caller
    /// checks it.
    pub head: u16,
    /// How many bytes the device says it wrote. Unit: bytes.
    ///
    /// On this protocol it is the response header the device filled in, which is
    /// always the same size — so unlike the network driver's receive queue, this
    /// number carries no information the driver acts on. It is answered anyway,
    /// and checked: a device that reported a used length shorter than the
    /// response header it was required to write has not answered the command,
    /// and reading a response out of the slot it left alone would be reading
    /// whatever this driver put there.
    pub written: u32,
}

impl Queue {
    /// Lay the control virtqueue, of `size` descriptors, out in `region`.
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

    /// Write one descriptor.
    ///
    /// `at` is an address in the *device's* space, which in this driver always
    /// came out of a [`Region::device_at`]. Nothing here checks that, and
    /// nothing here could: a descriptor is sixteen bytes and a driver is
    /// entitled to write them. What refuses an address outside the component's
    /// grants is the device's own domain, one bus away.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for an index past the queue.
    pub fn describe(
        &self,
        index: u16,
        at: u64,
        len: u32,
        flags: u16,
        next: u16,
    ) -> Result<(), Trouble> {
        if index >= self.size {
            return Err(Trouble::Layout);
        }
        let base = u32::from(index).saturating_mul(DESC_BYTES);
        self.region.put64(base, at)?;
        self.region.put32(base + 8, len)?;
        self.region.put16(base + 12, flags)?;
        self.region.put16(base + 14, next)?;
        Ok(())
    }

    /// Offer a chain to the device: put its head in the available ring and
    /// publish the index that makes it visible.
    ///
    /// The publish is a `Release` fence and then the store —
    /// [`Region::publish16`] — which is the same discipline the ring itself
    /// rests on and is load-bearing for the same reason one reader further away:
    /// a device has a weaker relationship to this core's store buffer than
    /// another core does.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a head past the queue.
    pub fn offer(&mut self, head: u16) -> Result<(), Trouble> {
        if head >= self.size {
            return Err(Trouble::Layout);
        }
        let slot = self.published & (self.size - 1);
        let at = AVAIL_AT + 4 + u32::from(slot).saturating_mul(2);
        self.region.put16(at, head)?;
        self.published = self.published.wrapping_add(1);
        self.region.publish16(AVAIL_AT + 2, self.published)?;
        Ok(())
    }

    /// Take one completion, if the device has published one.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a used element whose id cannot be a descriptor
    /// index, which is a device describing a chain that cannot exist — refused
    /// rather than truncated into a plausible slot, and refused **before** the
    /// cursor moves so that a second reader can still see what the device wrote.
    pub fn harvest(&mut self) -> Result<Option<Finished>, Trouble> {
        let published = self.region.consume16(USED_AT + 2)?;
        if published == self.seen {
            return Ok(None);
        }
        let slot = self.seen & (self.size - 1);
        let at = USED_AT + 4 + u32::from(slot).saturating_mul(8);
        let head = self.region.get32(at)?;
        let written = self.region.get32(at + 4)?;
        let Ok(head) = u16::try_from(head) else { return Err(Trouble::Layout) };
        self.seen = self.seen.wrapping_add(1);
        Ok(Some(Finished { head, written }))
    }

    /// Chains offered and not yet taken back. Unit: chains.
    ///
    /// One at most, on this driver: every command is sent and waited for. It is
    /// published rather than asserted because *the device is still holding this*
    /// is what a reader of a stuck driver wants to know.
    #[must_use]
    pub const fn outstanding(&self) -> u16 {
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

    impl Owned {
        const fn new() -> Self {
            Self([0; QUEUE_BYTES as usize])
        }

        /// The region a queue is laid out in, with a device address that is
        /// deliberately not the component's own — so a test that confused the
        /// two would produce an address no assertion here matches.
        fn region(&mut self) -> Region {
            Region::at(self.0.as_mut_ptr() as usize as u64, 0x4000_0000, QUEUE_BYTES)
                .expect("an aligned region")
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

        let short = Region::at(base, 0x4000_0000, QUEUE_BYTES - 16).expect("an aligned region");
        assert_eq!(
            Queue::over(short, QUEUE_SIZE).map(|_| ()),
            Err(Trouble::Layout),
            "a region the three rings do not fit in"
        );
    }

    #[test]
    fn the_three_rings_are_where_the_device_is_told_they_are() {
        let mut owned = Owned::new();
        let queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        assert_eq!(queue.device_desc(), Ok(0x4000_0000));
        assert_eq!(queue.device_avail(), Ok(0x4000_0000 + u64::from(AVAIL_AT)));
        assert_eq!(queue.device_used(), Ok(0x4000_0000 + u64::from(USED_AT)));
    }

    #[test]
    fn a_descriptor_lands_where_the_specification_puts_its_four_fields() {
        // Sixteen bytes in a fixed order, and a test that reads them back at
        // fixed offsets rather than through the writer that wrote them — the
        // only kind of test that can catch two fields swapped.
        let mut owned = Owned::new();
        let queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        queue.describe(1, 0xDEAD_BEEF_0000_1000, 48, DESC_NEXT, 2).expect("index 1");

        let at = 16;
        let bytes = &owned.0;
        let mut address = [0u8; 8];
        address.copy_from_slice(&bytes[at..at + 8]);
        assert_eq!(u64::from_le_bytes(address), 0xDEAD_BEEF_0000_1000);
        let mut len = [0u8; 4];
        len.copy_from_slice(&bytes[at + 8..at + 12]);
        assert_eq!(u32::from_le_bytes(len), 48);
        assert_eq!(u16::from_le_bytes([bytes[at + 12], bytes[at + 13]]), DESC_NEXT);
        assert_eq!(u16::from_le_bytes([bytes[at + 14], bytes[at + 15]]), 2);
    }

    #[test]
    fn an_index_past_the_queue_is_refused_and_not_wrapped() {
        let mut owned = Owned::new();
        let mut queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        assert_eq!(queue.describe(QUEUE_SIZE, 0x1000, 8, 0, 0), Err(Trouble::Layout));
        assert_eq!(queue.offer(QUEUE_SIZE), Err(Trouble::Layout));
    }

    #[test]
    fn one_command_goes_out_and_one_answer_comes_back() {
        // The whole of this driver's queue discipline in one test: a chain out,
        // an answer in, and nothing outstanding afterwards. A display command is
        // a request the device owes an answer to, which is `user/virtio-blk`'s
        // shape and not `user/virtio-net`'s — so what is under test here is that
        // the head is a constant and the completion is taken exactly once.
        let mut owned = Owned::new();
        let mut queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        queue.offer(0).expect("a chain");
        assert_eq!(queue.outstanding(), 1);
        assert_eq!(queue.harvest(), Ok(None), "nothing published yet");

        let used = USED_AT as usize;
        owned.0[used + 4..used + 8].copy_from_slice(&0u32.to_le_bytes());
        owned.0[used + 8..used + 12].copy_from_slice(&24u32.to_le_bytes());
        owned.0[used + 2..used + 4].copy_from_slice(&1u16.to_le_bytes());

        assert_eq!(queue.harvest(), Ok(Some(Finished { head: 0, written: 24 })));
        assert_eq!(queue.harvest(), Ok(None), "and not a second time");
        assert_eq!(queue.outstanding(), 0);
    }

    #[test]
    fn a_used_element_naming_a_chain_that_cannot_exist_is_refused() {
        // R04 at a field a *device* wrote. A used element's id is thirty-two
        // bits and a descriptor index is sixteen, so the extra bits are a device
        // describing something impossible — refused rather than truncated.
        let mut owned = Owned::new();
        let mut queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        queue.offer(0).expect("a chain");
        let used = USED_AT as usize;
        owned.0[used + 4..used + 8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        owned.0[used + 2..used + 4].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(queue.harvest(), Err(Trouble::Layout));
        assert_eq!(queue.outstanding(), 1, "the refused element did not advance the cursor");
        assert_eq!(queue.harvest(), Err(Trouble::Layout), "and it reads the same way twice");
    }
}
