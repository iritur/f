// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One split virtqueue, modelled at the level virtio actually works: a
//! descriptor table, an available ring, a used ring, and two cursors that never
//! cross.
//!
//! # Why this file is a copy of two others, on purpose
//!
//! `kernel/src/arch/x86_64/dma.rs` builds a virtqueue by hand against a real
//! QEMU device, and `user/virtio-blk/src/queue.rs` builds one inside a
//! component. Both chose the same three offsets and both said why: the modern
//! transport gives the three rings separate address registers, so where they sit
//! inside one region is the driver's choice, and round numbers let a reader
//! check the arithmetic without a calculator. This is the third, and **a model
//! that disagreed with the device would be a source of bugs rather than a way of
//! finding them** — so the numbers below are those numbers, the flags are those
//! flags, and the accessor names are [`f_ring::device::Region`]'s names, so that
//! a reader holding the two files side by side sees the same call at the same
//! offset.
//!
//! They are *copied* rather than shared, and `queue.rs` already gives the
//! reason for the pair it is half of: `dma.rs` is E1-B01's frozen adversary, and
//! a constant shared with it would make a change here a change to closed
//! evidence. [`tests::the_layout_is_the_one_the_real_queues_use`] is what keeps
//! the three honest instead — one test, three literals, and a failure that names
//! the two files to go and read.
//!
//! # One region, two ends, and what that does not model
//!
//! A real virtqueue is memory a driver writes and a device reads, concurrently,
//! across a bus. Here the driver's accesses and the device's accesses are
//! *separate events at separate instants* on one timeline, which is what a
//! single-threaded discrete-event simulator can express and is exactly R05's
//! shape: every access happens at a polling point, and there is no second path
//! in.
//!
//! What that models faithfully is the protocol — who may write which cursor, in
//! what order, and what the other end is allowed to conclude from it. The two
//! cursors each end owns are private state on both real ends too
//! (`last_avail_idx` in a device, `last_used_idx` in a driver), and
//! [`tests::neither_end_reads_past_what_the_other_published`] is the invariant
//! that says the model keeps them apart.
//!
//! What it does **not** model is the memory ordering. [`Queue::offer`] writes
//! the ring slot and then the index, in that order, because that order is the
//! protocol; on the real path there is a `Release` fence between them and here
//! there is nothing to fence. `docs/design/proving-ground.html` is blunt about
//! this being outside layer 1's reach — *simulation will not catch it; only a
//! memory-model tool will* — and layer 2 is where that gap has an owner. A
//! simulator that appeared to check the fence would be worse than one that says
//! it cannot.
//!
//! # A descriptor carries an address the model never dereferences
//!
//! [`Part::at`] is an address in the *device's* address space, which is what a
//! [`Reach`](f_ring::registry::Reach) answers and what the frame's IOMMU
//! translated. Nothing here turns one into bytes, and that absence is the same
//! one `registry::Reach` is built around: a type that could be dereferenced
//! would be a type inviting the copy `E1-B02` counts. The device model checks
//! that an address falls inside something it was given and refuses otherwise,
//! which is the model's version of the fault `dma.rs` provokes on real silicon.

use f_abi::error;

/// Bytes in one descriptor: address, length, flags, link. Unit: bytes.
const DESC_BYTES: u32 = 16;

/// Where the available ring sits inside the region. Unit: bytes.
const AVAIL_AT: u32 = 2048;

/// Where the used ring sits inside the region. Unit: bytes.
const USED_AT: u32 = 4096;

/// How large a region this layout needs. Unit: bytes.
pub const QUEUE_BYTES: u32 = 8192;

/// How many descriptors the layout above holds. Unit: descriptors.
pub const QUEUE_SIZE: u16 = 64;

const _: () = assert!(DESC_BYTES * (QUEUE_SIZE as u32) <= AVAIL_AT);
const _: () = assert!(AVAIL_AT + 6 + 2 * (QUEUE_SIZE as u32) <= USED_AT);
const _: () = assert!(USED_AT + 6 + 8 * (QUEUE_SIZE as u32) <= QUEUE_BYTES);

/// This descriptor is not the last of its chain.
pub const DESC_NEXT: u16 = 1;

/// The *device* writes this descriptor's buffer.
pub const DESC_WRITE: u16 = 2;

/// Why an access or a chain was refused.
///
/// The same three shapes `user/virtio-blk/src/queue.rs` has, because the model
/// refuses in the same places the driver does. Every one is a refusal rather
/// than a panic: a model that panicked where the system refuses would report a
/// crash for a case the system handles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// The geometry does not hold: a size that is not a power of two, an index
    /// past the queue, a region too short for the layout.
    Layout,
    /// An access fell outside the region or was misaligned. Carries the packed
    /// [`f_abi::error`] result the real accessor answers with.
    Region(i32),
    /// The driver has no free descriptor for a chain of this length. The
    /// driver's own back-pressure, and distinct from a device that is busy.
    NoDescriptors,
}

/// Memory one end writes and the other reads, with the address the *device*
/// sees it at.
///
/// Deliberately shaped like [`f_ring::device::Region`], down to the method
/// names: `get*` reads, `put*` writes, [`Region::publish16`] is the store that
/// makes earlier writes visible and [`Region::consume16`] is the load that
/// makes later reads valid. On the real path those two carry the fence; here
/// they carry the *discipline*, which is the half a model can hold.
#[derive(Clone, Debug)]
pub struct Region {
    bytes: Vec<u8>,
    /// Where the device addresses byte zero. Unit: bytes, in the device's
    /// address space — the answer [`Domains::map`](f_ring::registry::Domains::map)
    /// gives, carried here for the same reason `f_ring::device::Region` carries
    /// it: so a driver cannot assume the device's address space is its own.
    device: u64,
}

/// The refusal every out-of-range access answers with.
fn bad_address() -> i32 {
    error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS)
}

impl Region {
    /// `len` zeroed bytes, which the device addresses from `device`.
    #[must_use]
    pub fn new(len: u32, device: u64) -> Self {
        Self { bytes: vec![0; len as usize], device }
    }

    /// Bytes in the region. Unit: bytes.
    #[must_use]
    pub fn len(&self) -> u32 {
        u32::try_from(self.bytes.len()).unwrap_or(u32::MAX)
    }

    /// Never, as every constructor takes a length — the pair is conventional
    /// and the lint asks for it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Where the device addresses `offset`. Unit: bytes, device space.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an offset outside the region.
    pub fn device_at(&self, offset: u32) -> Result<u64, i32> {
        if offset >= self.len() {
            return Err(bad_address());
        }
        Ok(self.device.wrapping_add(u64::from(offset)))
    }

    /// Does `len` bytes at the device address `at` fall inside this region, and
    /// if so, at what offset?
    ///
    /// The model's whole address decode. A device model asks it of every
    /// descriptor before reading anything, and a `None` is the refusal that
    /// stands in for the fault a real remapping unit raises.
    #[must_use]
    pub fn holds(&self, at: u64, len: u32) -> Option<u32> {
        let offset = at.checked_sub(self.device)?;
        let offset = u32::try_from(offset).ok()?;
        let end = offset.checked_add(len)?;
        (end <= self.len()).then_some(offset)
    }

    /// Read one byte.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an access outside the region.
    pub fn get8(&self, offset: u32) -> Result<u8, i32> {
        self.bytes.get(offset as usize).copied().ok_or_else(bad_address)
    }

    /// Read two bytes, little-endian.
    ///
    /// # Errors
    ///
    /// As [`Region::get8`], and for a misaligned offset.
    pub fn get16(&self, offset: u32) -> Result<u16, i32> {
        Ok(u16::from_le_bytes(self.window::<2>(offset)?))
    }

    /// Read four bytes, little-endian.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`], at four bytes.
    pub fn get32(&self, offset: u32) -> Result<u32, i32> {
        Ok(u32::from_le_bytes(self.window::<4>(offset)?))
    }

    /// Read eight bytes, little-endian.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`], at eight bytes.
    pub fn get64(&self, offset: u32) -> Result<u64, i32> {
        Ok(u64::from_le_bytes(self.window::<8>(offset)?))
    }

    /// Write one byte.
    ///
    /// # Errors
    ///
    /// As [`Region::get8`].
    pub fn put8(&mut self, offset: u32, value: u8) -> Result<(), i32> {
        let slot = self.bytes.get_mut(offset as usize).ok_or_else(bad_address)?;
        *slot = value;
        Ok(())
    }

    /// Write two bytes, little-endian.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`].
    pub fn put16(&mut self, offset: u32, value: u16) -> Result<(), i32> {
        self.write(offset, &value.to_le_bytes())
    }

    /// Write four bytes, little-endian.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`], at four bytes.
    pub fn put32(&mut self, offset: u32, value: u32) -> Result<(), i32> {
        self.write(offset, &value.to_le_bytes())
    }

    /// Write eight bytes, little-endian.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`], at eight bytes.
    pub fn put64(&mut self, offset: u32, value: u64) -> Result<(), i32> {
        self.write(offset, &value.to_le_bytes())
    }

    /// The store that makes every earlier write visible to the other end.
    ///
    /// A plain [`Region::put16`] here, and the name is what carries the
    /// meaning: on the real path this is a `Release` and the module
    /// documentation says why the model has nothing to put in its place.
    /// Keeping the name means the two files still read alike at the one call
    /// site where the ordering is the protocol.
    ///
    /// # Errors
    ///
    /// As [`Region::put16`].
    pub fn publish16(&mut self, offset: u32, value: u16) -> Result<(), i32> {
        self.put16(offset, value)
    }

    /// The load that makes every later read of what the other end wrote valid.
    ///
    /// The `Acquire` half of the pair above, and absent here for the same
    /// reason.
    ///
    /// # Errors
    ///
    /// As [`Region::get16`].
    pub fn consume16(&self, offset: u32) -> Result<u16, i32> {
        self.get16(offset)
    }

    /// `N` bytes at `offset`, refusing an access that leaves the region or is
    /// misaligned to its own width.
    fn window<const N: usize>(&self, offset: u32) -> Result<[u8; N], i32> {
        // Fits: `N` is 2, 4 or 8 at every call site in this file.
        let width = u32::try_from(N).map_err(|_| bad_address())?;
        if !offset.is_multiple_of(width) {
            return Err(bad_address());
        }
        let end = offset.checked_add(width).ok_or_else(bad_address)?;
        let slice = self.bytes.get(offset as usize..end as usize).ok_or_else(bad_address)?;
        slice.try_into().map_err(|_| bad_address())
    }

    /// Write `value` at `offset`, with the same checks [`Region::window`] makes.
    fn write(&mut self, offset: u32, value: &[u8]) -> Result<(), i32> {
        let width = u32::try_from(value.len()).map_err(|_| bad_address())?;
        if !offset.is_multiple_of(width) {
            return Err(bad_address());
        }
        let end = offset.checked_add(width).ok_or_else(bad_address)?;
        let slot = self.bytes.get_mut(offset as usize..end as usize).ok_or_else(bad_address)?;
        slot.copy_from_slice(value);
        Ok(())
    }
}

/// One descriptor, as the end that reads it sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Part {
    /// Where the buffer is. Unit: bytes, in the device's address space.
    pub at: u64,
    /// How much of it. Unit: bytes.
    pub len: u32,
    /// Whether the *device* writes it. The [`DESC_WRITE`] flag, read out.
    pub write: bool,
}

/// A chain the device took off the available ring.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Chain {
    /// The descriptor the chain starts at, which is what the used ring names.
    /// Unit: none — a descriptor index.
    pub head: u16,
    /// Its descriptors, in link order.
    pub parts: Vec<Part>,
}

/// A split virtqueue over one region, with both ends' private state.
#[derive(Clone, Debug)]
pub struct Queue {
    region: Region,
    /// Descriptors in the ring. Unit: descriptors; a power of two, so the two
    /// cursors are masked rather than divided — the same reason
    /// `f_ring::registry::Table` requires one.
    size: u16,

    // --- the driver's private state ----------------------------------------
    /// Chains offered. Unit: chains, wrapping, which is what the available
    /// ring's index is.
    published: u16,
    /// Completions taken. Unit: chains, wrapping. A device's `last_used_idx`
    /// seen from the other side.
    seen: u16,
    /// One bit per descriptor, set while the driver has it in a chain.
    ///
    /// A bitmap and not a free list, and the lowest free descriptor always —
    /// for the reason `f_ring::registry::Table::register` gives about its own
    /// slots: an allocation order that depended on anything else would be a
    /// place a seeded run stopped reproducing.
    held: u64,

    // --- the device's private state -----------------------------------------
    /// Chains the device has taken. Unit: chains, wrapping. This is
    /// `last_avail_idx`, and it is private to the device on real silicon too.
    taken: u16,
    /// Completions the device has published. Unit: chains, wrapping.
    used: u16,
}

impl Queue {
    /// Lay a queue of `size` descriptors out in a region the device addresses
    /// from `device`.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a size that is zero, not a power of two, or
    /// larger than [`QUEUE_SIZE`].
    pub fn new(size: u16, device: u64) -> Result<Self, Trouble> {
        if size == 0 || !size.is_power_of_two() || size > QUEUE_SIZE {
            return Err(Trouble::Layout);
        }
        Ok(Self {
            region: Region::new(QUEUE_BYTES, device),
            size,
            published: 0,
            seen: 0,
            held: 0,
            taken: 0,
            used: 0,
        })
    }

    /// The memory the two ends share, for a model that has to state where its
    /// rings are.
    #[must_use]
    pub const fn region(&self) -> &Region {
        &self.region
    }

    /// Descriptors in the ring. Unit: descriptors.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    // -----------------------------------------------------------------------
    // The driver's half.
    // -----------------------------------------------------------------------

    /// Write a chain of descriptors and answer its head.
    ///
    /// The descriptors are linked with [`DESC_NEXT`] and the last one is not,
    /// which is the whole of what a chain is. Nothing is offered to the device
    /// until [`Queue::offer`], so a half-written chain is never visible — the
    /// discipline `dma.rs` states as *the entry is written first, and the cursor
    /// that makes it visible is written after*.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoDescriptors`] when the ring has no room for a chain this
    /// long, [`Trouble::Layout`] for an empty chain, and [`Trouble::Region`]
    /// for an access the layout cannot make — which [`Queue::new`] has already
    /// made unreachable.
    pub fn chain(&mut self, parts: &[Part]) -> Result<u16, Trouble> {
        if parts.is_empty() {
            return Err(Trouble::Layout);
        }
        let mut indices: Vec<u16> = Vec::with_capacity(parts.len());
        for _ in parts {
            let Some(index) = self.claim() else {
                // Nothing half-claimed survives a refusal: a chain that could
                // not be built leaves the ring exactly as it found it, which is
                // what makes `NoDescriptors` a retry rather than a leak. A
                // model that leaked here would run a long scenario down and
                // report a device that stopped.
                for held in &indices {
                    self.held &= !(1u64 << (*held & (QUEUE_SIZE - 1)));
                }
                return Err(Trouble::NoDescriptors);
            };
            indices.push(index);
        }

        for (slot, (index, part)) in indices.iter().zip(parts.iter()).enumerate() {
            let last = slot + 1 == parts.len();
            let next = if last { 0 } else { indices.get(slot + 1).copied().unwrap_or(0) };
            let mut flags = if last { 0 } else { DESC_NEXT };
            if part.write {
                flags |= DESC_WRITE;
            }
            let at = u32::from(*index).saturating_mul(DESC_BYTES);
            self.region.put64(at, part.at).map_err(Trouble::Region)?;
            self.region.put32(at + 8, part.len).map_err(Trouble::Region)?;
            self.region.put16(at + 12, flags).map_err(Trouble::Region)?;
            self.region.put16(at + 14, next).map_err(Trouble::Region)?;
        }

        indices.first().copied().ok_or(Trouble::Layout)
    }

    /// Offer a chain: put its head in the available ring, then publish the index
    /// that makes it visible.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a head past the queue, [`Trouble::Region`] for an
    /// access the layout cannot make.
    pub fn offer(&mut self, head: u16) -> Result<(), Trouble> {
        if head >= self.size {
            return Err(Trouble::Layout);
        }
        let slot = self.published & (self.size - 1);
        let at = AVAIL_AT + 4 + u32::from(slot).saturating_mul(2);
        self.region.put16(at, head).map_err(Trouble::Region)?;
        self.published = self.published.wrapping_add(1);
        self.region.publish16(AVAIL_AT + 2, self.published).map_err(Trouble::Region)
    }

    /// Take one completion, if the device has published one.
    ///
    /// Answers the head the device is finished with and the length it says it
    /// wrote. `user/virtio-blk/src/queue.rs` answers only the length because it
    /// serves one request at a time; a model that may have several outstanding
    /// needs the head to know *which*, and the head is in the used element for
    /// exactly that reason.
    ///
    /// The length is what the device says. Nothing here believes it: `dma.rs`
    /// records that this emulator's block device reports a successful completion
    /// for a transfer the remapping unit refused, so a completion is evidence
    /// that the device finished and never evidence that bytes moved.
    ///
    /// # Errors
    ///
    /// [`Trouble::Region`] for a used element outside the region, which
    /// [`Queue::new`] has already made unreachable.
    pub fn harvest(&mut self) -> Result<Option<(u16, u32)>, Trouble> {
        let published = self.region.consume16(USED_AT + 2).map_err(Trouble::Region)?;
        if published == self.seen {
            return Ok(None);
        }
        let slot = self.seen & (self.size - 1);
        let at = USED_AT + 4 + u32::from(slot).saturating_mul(8);
        let head = self.region.get32(at).map_err(Trouble::Region)?;
        let written = self.region.get32(at + 4).map_err(Trouble::Region)?;
        self.seen = self.seen.wrapping_add(1);
        // The used element's id is four bytes wide and a descriptor index is
        // two: a device that wrote a wider value than the ring can hold is a
        // device this driver refuses rather than truncates. R04.
        let head = u16::try_from(head).map_err(|_| Trouble::Layout)?;
        Ok(Some((head, written)))
    }

    /// Give a chain's descriptors back to the ring.
    ///
    /// Walks the links, exactly as the device did, so a chain is freed as a
    /// chain. A head the driver does not hold is refused rather than ignored:
    /// freeing a descriptor twice is how a ring comes to hand one buffer to two
    /// requests, and that is the double-submission `f_ring::registry` refuses
    /// one layer up.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a head past the queue or one the driver does not
    /// hold, [`Trouble::Region`] for an access the layout cannot make.
    pub fn release(&mut self, head: u16) -> Result<(), Trouble> {
        let mut at = head;
        for _ in 0..self.size {
            if at >= self.size {
                return Err(Trouble::Layout);
            }
            let bit = 1u64 << (at & (QUEUE_SIZE - 1));
            if self.held & bit == 0 {
                return Err(Trouble::Layout);
            }
            self.held &= !bit;
            let base = u32::from(at).saturating_mul(DESC_BYTES);
            let flags = self.region.get16(base + 12).map_err(Trouble::Region)?;
            if flags & DESC_NEXT == 0 {
                return Ok(());
            }
            at = self.region.get16(base + 14).map_err(Trouble::Region)?;
        }
        // A chain longer than the ring is a cycle in the links, which only a
        // corrupt ring produces. Refused, not walked forever.
        Err(Trouble::Layout)
    }

    /// Chains offered and not yet taken back. Unit: chains.
    #[must_use]
    pub fn outstanding(&self) -> u16 {
        self.published.wrapping_sub(self.seen)
    }

    /// Descriptors the driver is holding. Unit: descriptors.
    #[must_use]
    pub const fn held(&self) -> u32 {
        self.held.count_ones()
    }

    /// The lowest descriptor nobody holds.
    fn claim(&mut self) -> Option<u16> {
        for index in 0..self.size {
            let bit = 1u64 << (index & (QUEUE_SIZE - 1));
            if self.held & bit == 0 {
                self.held |= bit;
                return Some(index);
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // The device's half.
    // -----------------------------------------------------------------------

    /// Take the next chain off the available ring, if the driver has offered
    /// one.
    ///
    /// Walks the links and answers what the descriptors say. Every refusal here
    /// is a driver that wrote something impossible, and a device model that
    /// accepted it would be excusing a driver bug the real device would not.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a head past the queue or a chain that does not
    /// end, [`Trouble::Region`] for an access outside the region.
    pub fn take(&mut self) -> Result<Option<Chain>, Trouble> {
        let published = self.region.consume16(AVAIL_AT + 2).map_err(Trouble::Region)?;
        if published == self.taken {
            return Ok(None);
        }
        let slot = self.taken & (self.size - 1);
        let at = AVAIL_AT + 4 + u32::from(slot).saturating_mul(2);
        let head = self.region.get16(at).map_err(Trouble::Region)?;
        self.taken = self.taken.wrapping_add(1);

        let mut parts = Vec::new();
        let mut index = head;
        for _ in 0..self.size {
            if index >= self.size {
                return Err(Trouble::Layout);
            }
            let base = u32::from(index).saturating_mul(DESC_BYTES);
            let addr = self.region.get64(base).map_err(Trouble::Region)?;
            let len = self.region.get32(base + 8).map_err(Trouble::Region)?;
            let flags = self.region.get16(base + 12).map_err(Trouble::Region)?;
            parts.push(Part { at: addr, len, write: flags & DESC_WRITE != 0 });
            if flags & DESC_NEXT == 0 {
                return Ok(Some(Chain { head, parts }));
            }
            index = self.region.get16(base + 14).map_err(Trouble::Region)?;
        }
        Err(Trouble::Layout)
    }

    /// Publish one completion: the chain's head, and how many bytes the device
    /// wrote.
    ///
    /// # Errors
    ///
    /// [`Trouble::Region`] for an access the layout cannot make.
    pub fn publish(&mut self, head: u16, written: u32) -> Result<(), Trouble> {
        let slot = self.used & (self.size - 1);
        let at = USED_AT + 4 + u32::from(slot).saturating_mul(8);
        self.region.put32(at, u32::from(head)).map_err(Trouble::Region)?;
        self.region.put32(at + 4, written).map_err(Trouble::Region)?;
        self.used = self.used.wrapping_add(1);
        self.region.publish16(USED_AT + 2, self.used).map_err(Trouble::Region)
    }

    /// Chains the device has taken and not yet published. Unit: chains.
    #[must_use]
    pub fn working(&self) -> u16 {
        self.taken.wrapping_sub(self.used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x2000_0000;

    fn queue() -> Queue {
        Queue::new(8, BASE).expect("a power-of-two size inside the layout")
    }

    #[test]
    fn the_layout_is_the_one_the_real_queues_use() {
        // Three files choose these numbers and none of them shares a constant
        // with the others, on purpose: `dma.rs` is E1-B01's frozen adversary and
        // a shared constant would make a change here a change to closed
        // evidence. This is what stops the three drifting instead.
        //
        // If this fails, read `kernel/src/arch/x86_64/dma.rs` and
        // `user/virtio-blk/src/queue.rs` before changing anything here: a model
        // that disagrees with the device is a source of bugs, not a way of
        // finding them.
        assert_eq!(DESC_BYTES, 16);
        assert_eq!(AVAIL_AT, 2048);
        assert_eq!(USED_AT, 4096);
        assert_eq!(QUEUE_BYTES, 8192);
        assert_eq!(QUEUE_SIZE, 64);
        assert_eq!(DESC_NEXT, 1);
        assert_eq!(DESC_WRITE, 2);
    }

    #[test]
    fn a_size_the_layout_cannot_hold_is_refused() {
        assert_eq!(Queue::new(0, BASE).err(), Some(Trouble::Layout));
        assert_eq!(Queue::new(3, BASE).err(), Some(Trouble::Layout));
        assert_eq!(Queue::new(128, BASE).err(), Some(Trouble::Layout));
        assert!(Queue::new(64, BASE).is_ok());
    }

    #[test]
    fn a_chain_comes_out_of_the_device_end_as_it_went_in_the_driver_end() {
        // The whole protocol in one test: three descriptors, linked, offered,
        // and read back by the other end from the same bytes.
        let mut q = queue();
        let parts = [
            Part { at: 0x4000, len: 16, write: false },
            Part { at: 0x5000, len: 512, write: true },
            Part { at: 0x4010, len: 1, write: true },
        ];
        let head = q.chain(&parts).expect("three descriptors fit a ring of eight");
        assert!(
            q.take().expect("a legal ring").is_none(),
            "a chain was visible before it was offered"
        );

        q.offer(head).expect("a head inside the ring");
        let chain = q.take().expect("a legal ring").expect("the chain the driver offered");
        assert_eq!(chain.head, head);
        assert_eq!(chain.parts, parts.to_vec());
        assert!(q.take().expect("a legal ring").is_none(), "one chain was taken twice");
    }

    #[test]
    fn neither_end_reads_past_what_the_other_published() {
        // The two cursors are private state on both real ends, and this is the
        // invariant that says the model keeps them apart: the device sees
        // nothing until `offer`, and the driver sees nothing until `publish`.
        let mut q = queue();
        let head = q.chain(&[Part { at: 0x4000, len: 8, write: true }]).expect("one descriptor");
        assert!(q.harvest().expect("a legal ring").is_none());
        q.offer(head).expect("inside the ring");
        assert!(
            q.harvest().expect("a legal ring").is_none(),
            "the driver saw an unpublished completion"
        );

        let chain = q.take().expect("a legal ring").expect("the offered chain");
        assert!(q.harvest().expect("a legal ring").is_none(), "taking is not completing");
        q.publish(chain.head, 8).expect("a legal ring");
        assert_eq!(q.harvest().expect("a legal ring"), Some((head, 8)));
    }

    #[test]
    fn descriptors_are_given_back_and_the_ring_does_not_run_down() {
        // A ring of eight and a chain of three, twenty times over: without
        // `release` this stops at the seventh chain, which is exactly the bug a
        // driver has when it forgets one.
        let mut q = queue();
        let parts = [
            Part { at: 0x4000, len: 16, write: false },
            Part { at: 0x5000, len: 64, write: true },
            Part { at: 0x4010, len: 1, write: true },
        ];
        for _ in 0..20 {
            let head = q.chain(&parts).expect("the ring was emptied last time round");
            q.offer(head).expect("inside the ring");
            let chain = q.take().expect("a legal ring").expect("the offered chain");
            q.publish(chain.head, 65).expect("a legal ring");
            let (done, written) =
                q.harvest().expect("a legal ring").expect("a published completion");
            assert_eq!((done, written), (head, 65));
            q.release(done).expect("a chain the driver holds");
            assert_eq!(q.held(), 0);
        }
    }

    #[test]
    fn a_ring_with_no_room_refuses_and_keeps_nothing() {
        // `NoDescriptors` is a retry, so a refused chain must leave the ring
        // exactly as it found it. A model that leaked the descriptors it had
        // already claimed would run down over a long scenario and report a
        // device that stopped, which is the worst kind of false positive.
        let mut q = queue();
        let one = Part { at: 0x4000, len: 8, write: true };
        let mut heads = Vec::new();
        for _ in 0..8 {
            heads.push(q.chain(&[one]).expect("eight single-descriptor chains fit"));
        }
        assert_eq!(q.held(), 8);
        assert_eq!(q.chain(&[one]).err(), Some(Trouble::NoDescriptors));
        assert_eq!(q.chain(&[one, one]).err(), Some(Trouble::NoDescriptors));
        assert_eq!(q.held(), 8, "a refused chain kept descriptors");

        for head in heads {
            q.offer(head).expect("inside the ring");
            q.release(head).expect("a chain the driver holds");
        }
        assert_eq!(q.held(), 0);
    }

    #[test]
    fn a_descriptor_freed_twice_is_refused() {
        // Freeing one twice is how a ring hands one buffer to two requests,
        // which is the double submission `f_ring::registry::Table::resolve`
        // refuses one layer up. Refused here too, for the same reason and in
        // the same shape.
        let mut q = queue();
        let head = q.chain(&[Part { at: 0x4000, len: 8, write: true }]).expect("one descriptor");
        q.release(head).expect("a chain the driver holds");
        assert_eq!(q.release(head).err(), Some(Trouble::Layout));
    }

    #[test]
    fn an_access_outside_the_region_is_refused_rather_than_wrapped() {
        let region = Region::new(64, BASE);
        assert!(region.get8(63).is_ok());
        assert!(region.get8(64).is_err());
        assert!(region.get32(60).is_ok());
        assert!(region.get32(61).is_err(), "a misaligned read was allowed");
        assert!(region.get64(60).is_err(), "a read past the end was allowed");
        assert_eq!(region.device_at(0), Ok(BASE));
        assert_eq!(region.device_at(63), Ok(BASE + 63));
        assert!(region.device_at(64).is_err());
    }

    #[test]
    fn an_address_the_region_does_not_cover_is_not_decoded() {
        // The model's whole address decode, and the stand-in for the fault a
        // real remapping unit raises. Every boundary, because the interesting
        // failure is always the last byte.
        let region = Region::new(64, BASE);
        assert_eq!(region.holds(BASE, 64), Some(0));
        assert_eq!(region.holds(BASE + 32, 32), Some(32));
        assert_eq!(region.holds(BASE + 32, 33), None);
        assert_eq!(region.holds(BASE - 1, 1), None);
        assert_eq!(region.holds(BASE + 64, 1), None);
        assert_eq!(region.holds(u64::MAX, 1), None);
    }
}
