// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Two split virtqueues, written into a region the component was granted.
//!
//! # What is the same as the block driver's queue, and why that is the result
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
//! `user/virtio-blk/src/queue.rs` says the same paragraph, and the fact that it
//! can be said twice without editing `ring/src/device.rs` is the finding E1-B03
//! exists to produce. What follows is the part that is *not* the same, because
//! that is the part worth a reader's attention.
//!
//! # Three differences, each forced by the receive direction
//!
//! **A queue knows its index.** A block device has one virtqueue and the block
//! driver's transport writes `QUEUE_SELECT` as a literal zero. A network device
//! has two per pair, and which doorbell to ring is a per-queue number the device
//! publishes — so [`Queue`] carries the index it was laid out for and the
//! transport is asked for that queue's doorbell rather than for *the* doorbell.
//!
//! **A completion says which chain it was.** [`Queue::harvest`] answers the
//! used element's `id` as well as its length. The block driver has one request
//! outstanding and therefore never has to ask; a receive queue holds every
//! posted buffer at once and the whole question is *which of them just filled
//! up*. Reading `id` costs one more four-byte load and is the difference
//! between a driver that can receive and one that cannot.
//!
//! **A chain's head is chosen by the caller and never by this file.** There is
//! no free-descriptor list here and there will not be one: [`crate::driver`]
//! assigns descriptor pairs to receive slots by arithmetic — slot `i` is
//! descriptors `2i` and `2i+1` — so which descriptors a chain occupies is a
//! constant of the layout rather than the outcome of an allocation. An
//! allocator would be a place a seeded run stopped reproducing, and RFC 0004
//! says the only source of that is `f_env::Env`, which a component does not
//! hold.
//!
//! # Why the layout is a constant rather than a computation
//!
//! The three rings have separate address registers in the modern transport, so
//! where they sit inside one region is this file's choice and not the
//! specification's. Round numbers mean a reader can check the arithmetic
//! without a calculator, and the three assertions below are what keep the choice
//! honest. The numbers are `user/virtio-blk/src/queue.rs`'s numbers, and that
//! file explains why it did not share a constant with
//! `kernel/src/arch/x86_64/dma.rs` — that file is E1-B01's frozen adversary. The
//! reason does not extend to *this* file, and the constants are still not
//! shared, for a different reason worth stating: a shared queue layout would be
//! a fourth crate or a wider `abi`, and what is actually shared between the two
//! drivers is much larger than three offsets. RFC 0051 says what a third driver
//! should do about that, and deliberately does not do it here on the strength of
//! two examples.

use f_ring::device::Region;

use crate::Trouble;

/// This descriptor is not the last of its chain.
pub const DESC_NEXT: u16 = 1;

/// The *device* writes this descriptor's buffer.
pub const DESC_WRITE: u16 = 2;

/// Bytes in one descriptor: address, length, flags, link.
/// Unit: bytes.
const DESC_BYTES: u32 = 16;

/// Where the available ring sits inside one queue's region. Unit: bytes.
const AVAIL_AT: u32 = 2048;

/// Where the used ring sits inside one queue's region. Unit: bytes.
const USED_AT: u32 = 4096;

/// How large a region one queue's layout needs. Unit: bytes.
pub const QUEUE_BYTES: u32 = 8192;

/// How many descriptors that layout holds. Unit: descriptors.
pub const QUEUE_SIZE: u16 = 64;

const _: () = assert!(DESC_BYTES * (QUEUE_SIZE as u32) <= AVAIL_AT);
const _: () = assert!(AVAIL_AT + 6 + 2 * (QUEUE_SIZE as u32) <= USED_AT);
const _: () = assert!(USED_AT + 6 + 8 * (QUEUE_SIZE as u32) <= QUEUE_BYTES);

/// Which virtqueue of a virtio-net device this is.
///
/// The specification's numbering for a single queue pair, and it is not
/// arbitrary: receive is even and transmit is odd, so a device with `N` pairs
/// has receive queues at `2n` and transmit queues at `2n + 1`. This driver has
/// one pair and says so by naming two constants rather than by writing `0` and
/// `1` at four call sites.
pub mod index {
    /// The queue the device writes frames into. Unit: none — a queue index.
    pub const RECEIVE: u16 = 0;
    /// The queue the device reads frames out of. Unit: none — a queue index.
    pub const TRANSMIT: u16 = 1;
    /// The control queue, which this driver **does not use** and does not
    /// negotiate `VIRTIO_NET_F_CTRL_VQ` for.
    ///
    /// Named rather than omitted so that a reader looking for it finds the
    /// decision instead of an absence. Everything the control queue offers —
    /// unicast filtering, promiscuous mode, VLAN tables, multiqueue steering,
    /// announcing a link change — is a policy this system has nowhere to put
    /// yet, and a driver that negotiated the feature would have to answer what
    /// its filter table contains. Unit: none — a queue index.
    pub const CONTROL: u16 = 2;
}

/// One split virtqueue over granted memory.
#[derive(Clone, Copy, Debug)]
pub struct Queue {
    region: Region,
    /// Which virtqueue of the device this is. Unit: none — a queue index.
    which: u16,
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
    /// checks it: a device that answered with an index it was never given would
    /// otherwise steer this driver's own bookkeeping.
    pub head: u16,
    /// How many bytes the device says it wrote. Unit: bytes.
    ///
    /// Meaningful on the receive queue, where it is the header plus the frame,
    /// and **zero on the transmit queue**, where virtio-net defines no answer at
    /// all — `sim/src/net.rs` is a model of exactly that silence and says why it
    /// is the protocol rather than a hole. A driver that reported a transmit as
    /// *delivered* would be inventing information.
    pub written: u32,
}

impl Queue {
    /// Lay virtqueue `which`, of `size` descriptors, out in `region`.
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
    pub const fn over(region: Region, which: u16, size: u16) -> Result<Self, Trouble> {
        if size == 0 || !size.is_power_of_two() || size > QUEUE_SIZE {
            return Err(Trouble::Layout);
        }
        if region.len() < QUEUE_BYTES {
            return Err(Trouble::Layout);
        }
        Ok(Self { region, which, size, published: 0, seen: 0 })
    }

    /// Which virtqueue of the device this is. Unit: none — a queue index.
    #[must_use]
    pub const fn which(&self) -> u16 {
        self.which
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
    /// came out of a [`Region::device_at`] or a
    /// [`Reach`](f_ring::registry::Reach) the service resolved. Nothing here
    /// checks that, and nothing here could: a descriptor is sixteen bytes and a
    /// driver is entitled to write them. What refuses an address outside the
    /// component's grants is the device's own domain, one bus away.
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
    /// rests on and is load-bearing for the same reason one reader further
    /// away: a device has a weaker relationship to this core's store buffer than
    /// another core does.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a head past the queue.
    pub fn offer(&mut self, head: u16) -> Result<(), Trouble> {
        if head >= self.size {
            return Err(Trouble::Layout);
        }
        // Masked rather than divided, which is what the power-of-two size in
        // `over` buys — the same reason `f_ring::registry::Table` requires one.
        let slot = self.published & (self.size - 1);
        let at = AVAIL_AT + 4 + u32::from(slot).saturating_mul(2);
        self.region.put16(at, head)?;
        self.published = self.published.wrapping_add(1);
        self.region.publish16(AVAIL_AT + 2, self.published)?;
        Ok(())
    }

    /// Take one completion, if the device has published one.
    ///
    /// Answers the chain's head **and** what the device says it wrote, which is
    /// the one line where this file differs from the block driver's in a way a
    /// reader has to notice. That driver has one chain outstanding, so the head
    /// is a constant it never reads back; a receive queue holds every posted
    /// buffer at once and the head is the only thing that says which one just
    /// filled.
    ///
    /// The head is a **device's word** and is not trusted here: this returns it,
    /// and [`crate::driver`] refuses one that does not name a slot it posted. A
    /// driver that indexed its own bookkeeping with it directly would be a
    /// driver a device can steer.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a used ring whose element falls outside the
    /// region, which [`Queue::over`] has already made unreachable.
    pub fn harvest(&mut self) -> Result<Option<Finished>, Trouble> {
        let published = self.region.consume16(USED_AT + 2)?;
        if published == self.seen {
            return Ok(None);
        }
        let slot = self.seen & (self.size - 1);
        let at = USED_AT + 4 + u32::from(slot).saturating_mul(8);
        // A used element is `{ u32 id; u32 len; }`. The id is a descriptor index
        // and is therefore sixteen bits of meaning in a thirty-two bit field;
        // a value that does not fit is a device describing a chain that cannot
        // exist, and it is refused by the caller rather than truncated here.
        let head = self.region.get32(at)?;
        let written = self.region.get32(at + 4)?;
        // Refused **before** the cursor moves, which is the same order this file
        // argues for the head everywhere else: a driver that consumed an element
        // it then refused would have taken the one piece of evidence about what
        // the device did out of reach of whoever comes to read it. Nothing today
        // re-reads a refused element — the only caller stops — but *believe it,
        // then advance* is the order somebody will change this to, and this way
        // round is the one that survives it.
        let Ok(head) = u16::try_from(head) else { return Err(Trouble::Layout) };
        self.seen = self.seen.wrapping_add(1);
        Ok(Some(Finished { head, written }))
    }

    /// Chains offered and not yet taken back. Unit: chains.
    ///
    /// Published as a counter rather than asserted to be zero, because *the
    /// device is still holding this many buffers* is exactly what a reader of a
    /// stuck driver wants to know — and on a receive queue it is not an error
    /// at all, it is the resting state.
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

        assert_eq!(
            Queue::over(region, index::RECEIVE, 0).map(|_| ()),
            Err(Trouble::Layout),
            "no descriptors"
        );
        assert_eq!(
            Queue::over(region, index::RECEIVE, 3).map(|_| ()),
            Err(Trouble::Layout),
            "not a power of two"
        );
        assert_eq!(
            Queue::over(region, index::RECEIVE, QUEUE_SIZE * 2).map(|_| ()),
            Err(Trouble::Layout),
            "past what the layout holds"
        );

        let short = Region::at(base, 0x4000_0000, QUEUE_BYTES - 16).expect("an aligned region");
        assert_eq!(
            Queue::over(short, index::RECEIVE, QUEUE_SIZE).map(|_| ()),
            Err(Trouble::Layout),
            "a region the three rings do not fit in"
        );
    }

    #[test]
    fn a_queue_remembers_which_queue_of_the_device_it_is() {
        // The whole reason this type carries an index. A block driver writes a
        // literal zero into `QUEUE_SELECT`; a driver with two queues that lost
        // track of which one it was laying out would program the receive
        // queue's addresses into the transmit queue's registers, and every
        // symptom of that is at the device.
        let mut owned = Owned::new();
        let region = owned.region();
        let rx = Queue::over(region, index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
        let tx = Queue::over(region, index::TRANSMIT, QUEUE_SIZE).expect("a queue that fits");
        assert_eq!(rx.which(), 0);
        assert_eq!(tx.which(), 1);
        assert_ne!(rx.which(), tx.which());
    }

    #[test]
    fn the_three_rings_are_where_the_device_is_told_they_are() {
        let mut owned = Owned::new();
        let queue =
            Queue::over(owned.region(), index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
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
        let queue =
            Queue::over(owned.region(), index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
        queue.describe(1, 0xDEAD_BEEF_0000_1000, 2048, DESC_NEXT | DESC_WRITE, 2).expect("index 1");

        let at = 16;
        let bytes = &owned.0;
        let mut address = [0u8; 8];
        address.copy_from_slice(&bytes[at..at + 8]);
        assert_eq!(u64::from_le_bytes(address), 0xDEAD_BEEF_0000_1000);
        let mut len = [0u8; 4];
        len.copy_from_slice(&bytes[at + 8..at + 12]);
        assert_eq!(u32::from_le_bytes(len), 2048);
        assert_eq!(u16::from_le_bytes([bytes[at + 12], bytes[at + 13]]), DESC_NEXT | DESC_WRITE);
        assert_eq!(u16::from_le_bytes([bytes[at + 14], bytes[at + 15]]), 2);
    }

    #[test]
    fn an_index_past_the_queue_is_refused_and_not_wrapped() {
        let mut owned = Owned::new();
        let mut queue =
            Queue::over(owned.region(), index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
        assert_eq!(queue.describe(QUEUE_SIZE, 0x1000, 8, 0, 0), Err(Trouble::Layout));
        assert_eq!(queue.offer(QUEUE_SIZE), Err(Trouble::Layout));
    }

    #[test]
    fn several_chains_are_offered_and_each_lands_in_its_own_slot() {
        // The receive queue's whole shape: more than one chain outstanding, in
        // the order they were offered. A block driver never exercises this
        // because it offers one chain and waits for it.
        let mut owned = Owned::new();
        let mut queue =
            Queue::over(owned.region(), index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
        for head in [0u16, 2, 4, 6] {
            queue.offer(head).expect("a head inside the queue");
        }
        assert_eq!(queue.outstanding(), 4);

        let ring = AVAIL_AT as usize;
        let bytes = &owned.0;
        assert_eq!(u16::from_le_bytes([bytes[ring + 2], bytes[ring + 3]]), 4, "the index moved");
        for (slot, expected) in [0u16, 2, 4, 6].into_iter().enumerate() {
            let at = ring + 4 + slot * 2;
            assert_eq!(u16::from_le_bytes([bytes[at], bytes[at + 1]]), expected);
        }
    }

    #[test]
    fn a_completion_names_the_chain_it_finished_and_is_taken_once() {
        // The device's half, written by hand. What is under test is that
        // `harvest` answers the *head* as well as the length, and answers
        // exactly once per published element — a driver that took the same
        // completion twice would return a client's buffer while the device
        // still held it, and one that ignored the head would return the wrong
        // client's buffer.
        let mut owned = Owned::new();
        let mut queue =
            Queue::over(owned.region(), index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
        queue.offer(0).expect("a chain");
        queue.offer(2).expect("a second chain");
        assert_eq!(queue.harvest(), Ok(None), "nothing published yet");

        let used = USED_AT as usize;
        // The device finishes the *second* chain first, which is legal and is
        // the case a driver keyed on arrival order would get wrong.
        owned.0[used + 4..used + 8].copy_from_slice(&2u32.to_le_bytes());
        owned.0[used + 8..used + 12].copy_from_slice(&(12u32 + 42).to_le_bytes());
        owned.0[used + 2..used + 4].copy_from_slice(&1u16.to_le_bytes());

        assert_eq!(queue.harvest(), Ok(Some(Finished { head: 2, written: 54 })));
        assert_eq!(queue.harvest(), Ok(None), "and not a second time");
        assert_eq!(queue.outstanding(), 1, "the first chain is still with the device");
    }

    #[test]
    fn a_used_element_naming_a_chain_that_cannot_exist_is_refused() {
        // R04 at a field a *device* wrote. A used element's id is thirty-two
        // bits and a descriptor index is sixteen, so the extra bits are a
        // device describing something impossible — refused rather than
        // truncated into a plausible slot.
        let mut owned = Owned::new();
        let mut queue =
            Queue::over(owned.region(), index::RECEIVE, QUEUE_SIZE).expect("a queue that fits");
        queue.offer(0).expect("a chain");
        let used = USED_AT as usize;
        owned.0[used + 4..used + 8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        owned.0[used + 2..used + 4].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(queue.harvest(), Err(Trouble::Layout));
        // And the element was not consumed by being refused. A cursor that had
        // moved would put the device's own word about what it did out of reach
        // of a second reader, which is the wrong direction to be wrong in at the
        // one place a device has already misbehaved.
        assert_eq!(queue.outstanding(), 1, "the refused element did not advance the cursor");
        assert_eq!(queue.harvest(), Err(Trouble::Layout), "and it reads the same way twice");
    }
}
