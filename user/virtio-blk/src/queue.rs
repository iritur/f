// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One split virtqueue, written into a region the component was granted.
//!
//! # What is in here and what deliberately is not
//!
//! Descriptors, an available ring and a used ring, over a [`Region`] — which is
//! to say over memory this component holds a capability for and the frame has
//! given the device a translation for. Every address that reaches a descriptor
//! comes from [`Region::device_at`], so the only addresses this driver can put
//! in front of a device are addresses inside grants it was given.
//!
//! What is not in here is any way to reach a *client's* bytes. A descriptor
//! carries an address and a length; a [`Reach`](f_ring::registry::Reach) is an
//! address and a length; and there is no type in this crate that turns either
//! into a slice. That is the mechanism behind *zero copies on the data path* —
//! not a discipline, not a review note: the operation that would copy has
//! nothing to copy from.
//!
//! # Why the layout is a constant rather than a computation
//!
//! The three rings have separate address registers in the modern transport, so
//! where they sit inside one region is this file's choice and not the
//! specification's. Round numbers mean a reader can check the arithmetic
//! without a calculator, and the three assertions below are what keep the
//! choice honest. `kernel/src/arch/x86_64/dma.rs` chose the same three numbers
//! for the same reason, and the two agreeing is a coincidence worth stating
//! rather than a shared constant: that file is E1-B01's frozen adversary and a
//! constant shared with it would make a change here a change to closed
//! evidence.
//!
//! # One request at a time
//!
//! [`Queue::offer`] publishes one chain and [`Queue::harvest`] takes one
//! completion, and the driver above serves one request before it takes the
//! next. The queue is sixty-four descriptors because that is what fits the
//! layout below, not because sixty-four requests can be outstanding.
//!
//! *Reversal, and it has an owner:* E1-B06 orders a device queue by deadline,
//! which is only a statement about anything if more than one request is
//! outstanding. The state that would change is `published` and `seen` becoming
//! a ring of tokens rather than two counters, and nothing above this file
//! assumes otherwise — `Driver::execute` already answers one entry at a time.

use f_ring::device::Region;

use crate::Trouble;

/// This descriptor is not the last of its chain.
pub const DESC_NEXT: u16 = 1;

/// The *device* writes this descriptor's buffer.
pub const DESC_WRITE: u16 = 2;

/// Bytes in one descriptor: address, length, flags, link.
/// Unit: bytes.
const DESC_BYTES: u32 = 16;

/// Where the available ring sits inside the region. Unit: bytes.
const AVAIL_AT: u32 = 2048;

/// Where the used ring sits inside the region. Unit: bytes.
const USED_AT: u32 = 4096;

/// How large a region this layout needs. Unit: bytes.
pub const QUEUE_BYTES: u32 = 8192;

/// How many descriptors the layout above holds.
/// Unit: descriptors.
pub const QUEUE_SIZE: u16 = 64;

const _: () = assert!(DESC_BYTES * (QUEUE_SIZE as u32) <= AVAIL_AT);
const _: () = assert!(AVAIL_AT + 6 + 2 * (QUEUE_SIZE as u32) <= USED_AT);
const _: () = assert!(USED_AT + 6 + 8 * (QUEUE_SIZE as u32) <= QUEUE_BYTES);

/// A split virtqueue over granted memory.
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

impl Queue {
    /// Lay a queue of `size` descriptors out in `region`.
    ///
    /// The region must be zeroed, and that is the caller's obligation rather
    /// than this function's work: a component's memory arrives zeroed from the
    /// frame — `component::charge` states it as an obligation — and zeroing it
    /// again here would be a driver that cannot tell the difference between
    /// memory it was given and memory it found.
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
    /// component's grants is the device's own domain, one bus away, and that
    /// refusal is the second half of this task's exit.
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
    /// rests on and is load-bearing for the same reason, one reader further
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
    /// Answers how many bytes the device says it wrote. Unit: bytes — and the
    /// driver above does not believe it: `dma.rs` records that this emulator's
    /// block device reports a *successful* completion for a transfer the
    /// remapping unit refused, so a completion is evidence that the device
    /// finished and never evidence that bytes moved.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a used ring whose element falls outside the
    /// region, which [`Queue::over`] has already made unreachable.
    pub fn harvest(&mut self) -> Result<Option<u32>, Trouble> {
        let published = self.region.consume16(USED_AT + 2)?;
        if published == self.seen {
            return Ok(None);
        }
        let slot = self.seen & (self.size - 1);
        let at = USED_AT + 4 + u32::from(slot).saturating_mul(8);
        let written = self.region.get32(at + 4)?;
        self.seen = self.seen.wrapping_add(1);
        Ok(Some(written))
    }

    /// Chains offered and not yet taken back. Unit: chains.
    ///
    /// Published as a counter rather than asserted to be zero, because *the
    /// driver has nothing outstanding* is exactly what a reader of a stuck
    /// driver wants to know and is not something a log line can claim on its
    /// own behalf.
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
    ///
    /// Built in each test rather than by a helper that hands one back, because
    /// a helper would have to return the storage and the queue together and
    /// the test would be about lifetimes instead of about a virtqueue.
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
        queue.describe(1, 0xDEAD_BEEF_0000_1000, 512, DESC_NEXT | DESC_WRITE, 2).expect("index 1");

        let at = 16;
        let bytes = &owned.0;
        let mut address = [0u8; 8];
        address.copy_from_slice(&bytes[at..at + 8]);
        assert_eq!(u64::from_le_bytes(address), 0xDEAD_BEEF_0000_1000);
        let mut len = [0u8; 4];
        len.copy_from_slice(&bytes[at + 8..at + 12]);
        assert_eq!(u32::from_le_bytes(len), 512);
        assert_eq!(u16::from_le_bytes([bytes[at + 12], bytes[at + 13]]), DESC_NEXT | DESC_WRITE);
        assert_eq!(u16::from_le_bytes([bytes[at + 14], bytes[at + 15]]), 2);
    }

    #[test]
    fn an_index_past_the_queue_is_refused_and_not_wrapped() {
        // Wrapping would put a descriptor over descriptor zero, which is the
        // head of the chain the driver is in the middle of building.
        let mut owned = Owned::new();
        let mut queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        assert_eq!(queue.describe(QUEUE_SIZE, 0x1000, 8, 0, 0), Err(Trouble::Layout));
        assert_eq!(queue.offer(QUEUE_SIZE), Err(Trouble::Layout));
    }

    #[test]
    fn an_offered_chain_advances_the_available_index_and_nothing_else() {
        let mut owned = Owned::new();
        let mut queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        assert_eq!(queue.outstanding(), 0);

        queue.offer(0).expect("the head of a chain");
        let ring = AVAIL_AT as usize;
        let bytes = &owned.0;
        assert_eq!(u16::from_le_bytes([bytes[ring], bytes[ring + 1]]), 0, "flags untouched");
        assert_eq!(u16::from_le_bytes([bytes[ring + 2], bytes[ring + 3]]), 1, "the index moved");
        assert_eq!(u16::from_le_bytes([bytes[ring + 4], bytes[ring + 5]]), 0, "slot zero");
        assert_eq!(queue.outstanding(), 1);
    }

    #[test]
    fn a_completion_is_taken_once_and_only_once() {
        // The device's half, written by hand: the used index moves and one
        // element is filled in. What is under test is that `harvest` answers
        // exactly once per published element — a driver that took the same
        // completion twice would return a client's buffer while the device
        // still held it.
        let mut owned = Owned::new();
        let mut queue = Queue::over(owned.region(), QUEUE_SIZE).expect("a queue that fits");
        queue.offer(0).expect("a chain");
        assert_eq!(queue.harvest(), Ok(None), "nothing published yet");

        let used = USED_AT as usize;
        owned.0[used + 4..used + 8].copy_from_slice(&0u32.to_le_bytes());
        owned.0[used + 8..used + 12].copy_from_slice(&512u32.to_le_bytes());
        owned.0[used + 2..used + 4].copy_from_slice(&1u16.to_le_bytes());

        assert_eq!(queue.harvest(), Ok(Some(512)));
        assert_eq!(queue.harvest(), Ok(None), "and not a second time");
        assert_eq!(queue.outstanding(), 0);
    }
}
