// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What the store writes to, and a model of one that runs on the host.
//!
//! # Why the barrier is on the trait from the first commit
//!
//! Because a publish's correctness is an *ordering* — write the blobs, `FLUSH`,
//! append the root record, `FLUSH` — and a trait that cannot express one pushes
//! the ordering into every caller, where each caller gets to have its own
//! opinion about it. RFC 0060 decided what a barrier is before this file
//! existed, so [`Device::flush`] lands with the trait rather than being added
//! on the day `f-zone` needs it.
//!
//! What [`Device::flush`] promises is exactly what RFC 0060's `FLUSH` = 3
//! promises and not a word more: when it returns, every write this device has
//! already reported complete is on stable media. It says nothing about writes
//! still in flight, nothing about writes submitted after it, and nothing about
//! whether the device told the truth. The format does not rest on the last of
//! those — verify before accept is what makes a lying device cost a rollback
//! rather than a corruption — and the barrier buys the *rate* at which a
//! publish survives a power cut, not its correctness.
//!
//! # What is deliberately not here
//!
//! **The four zone operations.** `ZONE_APPEND`, `ZONE_FINISH`, `ZONE_RESET` and
//! `ZONE_REPORT` are RFC 0060's other four opcodes, and a store that knew about
//! zones would be a second crate that could disagree with `f-zone` about an
//! offset. `E2-B02` puts `f-zone` between this trait and the real device; what
//! this trait is, is the *block* device the store needs, which is the half that
//! has no zones in it at all.
//!
//! **The honest and lying modes.** A device that acknowledges a barrier it did
//! not honour is `E2-P01`'s wrapper around [`Memory`], not a second
//! implementation of it — RFC 0034's rule that a device model is a peer on the
//! real types, applied one level down. Writing the modes here would make the
//! cut model's device and the store's device two types that have to be kept
//! agreeing.

use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::refusal;

/// A block device, in the five operations the store needs of one.
///
/// Addresses are block indices and never byte offsets, because every record the
/// store writes begins at a block boundary and is padded to a whole number of
/// them: an interface in bytes would admit an address no record can start at,
/// and would then have to refuse it.
pub trait Device {
    /// The device's logical block. Every `read` and `write` moves exactly this
    /// many bytes.
    ///
    /// Unit: bytes.
    fn block_bytes(&self) -> usize;

    /// How many blocks the device has.
    ///
    /// Unit: count of blocks.
    fn blocks(&self) -> u64;

    /// Read one block into `into`, which must be exactly [`Device::block_bytes`]
    /// long.
    ///
    /// `&mut self` and not `&self`, which is the honest signature even for a
    /// model that could answer from a slice: a read on the device this store
    /// will actually have is a submission to a ring, and a ring is mutated by
    /// being submitted to. A trait whose read took `&self` would have to be
    /// widened on the day the real device arrives, and every caller written
    /// against the narrow one rewritten with it.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a block outside the device or a buffer that is
    /// not one block long. A device with hardware to report may add its own
    /// [`f_abi::error::DEVICE`] refusals.
    fn read(&mut self, block: u64, into: &mut [u8]) -> Result<(), i32>;

    /// Write one block from `from`, which must be exactly
    /// [`Device::block_bytes`] long.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a block outside the device or a buffer that is
    /// not one block long.
    fn write(&mut self, block: u64, from: &[u8]) -> Result<(), i32>;

    /// The barrier. When this returns, every write already reported complete is
    /// on stable media.
    ///
    /// # Errors
    ///
    /// Whatever the device says. There is no partial barrier: it completed or
    /// it did not.
    fn flush(&mut self) -> Result<(), i32>;
}

/// The modelled device: blocks in memory, a bounds check, and a barrier that
/// counts.
///
/// # Why the barrier counts rather than doing nothing
///
/// It does nothing — memory is already as durable as this model gets — and a
/// `flush` that did nothing *and said nothing* would leave "the store issues
/// the barrier before it claims a publish" as a sentence in a comment. The
/// counter makes it a number a test can assert, which is the same discipline
/// `cargo xtask cut` applies one level up when it counts device operations by
/// opcode. RFC 0060's second reversal condition is a ratio of counts, and this
/// is where the denominator starts.
pub struct Memory {
    /// Unit: bytes.
    block_bytes: usize,
    /// The device's contents, `blocks * block_bytes` of them.
    bytes: Vec<u8>,
    /// Unit: count of barriers issued.
    flushes: u64,
    /// Unit: count of blocks written.
    writes: u64,
}

impl Memory {
    /// A zeroed device of `blocks` blocks.
    ///
    /// Zeroed and not filled with a pattern, because a zeroed block is what an
    /// unformatted device offers and every record in `f_abi::store` refuses one
    /// — so a model that started from noise would be a model where a decoder
    /// bug is caught by luck instead of by the case it was written for.
    ///
    /// # Panics
    ///
    /// If the device would be larger than this machine can allocate, which is
    /// the allocator's panic and not a refusal of this crate's: a test that
    /// asks for a device it cannot have has a wrong number in it, and a
    /// `Result` here would let it carry on with a smaller one.
    #[must_use]
    pub fn new(block_bytes: usize, blocks: u64) -> Self {
        let len = block_bytes * usize::try_from(blocks).expect("a device this host can address");
        Self { block_bytes, bytes: vec![0u8; len], flushes: 0, writes: 0 }
    }

    /// How many barriers this device has been asked for.
    ///
    /// Unit: count of barriers.
    #[must_use]
    pub const fn flushes(&self) -> u64 {
        self.flushes
    }

    /// How many blocks have been written to this device.
    ///
    /// Unit: count of blocks.
    #[must_use]
    pub const fn writes(&self) -> u64 {
        self.writes
    }

    /// Flip the lowest bit of one byte, wherever it lies.
    ///
    /// # Why a device model has a mutator no store will ever call
    ///
    /// Because a verifier that has never failed is indistinguishable from one
    /// that cannot fail. `blob/tests/million.rs` writes a million blobs and
    /// reads every one of them back, and that run is green whether the content
    /// hash is checked or ignored; the control that separates the two is this
    /// function, and the assertion that exactly one read afterwards is refused
    /// with `refusal::CONTENT`. It is `pub` rather than `#[cfg(test)]` because
    /// a test in `blob/tests/` is a separate crate and cannot see a test-only
    /// item — and because `E2-P01`'s cut model is a second caller for it.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for an offset outside the device.
    pub fn flip(&mut self, at: usize) -> Result<(), i32> {
        let byte = self.bytes.get_mut(at).ok_or(refusal::ADDRESS)?;
        *byte ^= 1;
        Ok(())
    }

    /// The byte range one block occupies, or `None` for a block outside the
    /// device.
    fn span(&self, block: u64) -> Option<(usize, usize)> {
        let at = usize::try_from(block).ok()?.checked_mul(self.block_bytes)?;
        let end = at.checked_add(self.block_bytes)?;
        if end > self.bytes.len() { None } else { Some((at, end)) }
    }
}

impl Device for Memory {
    fn block_bytes(&self) -> usize {
        self.block_bytes
    }

    fn blocks(&self) -> u64 {
        (self.bytes.len() / self.block_bytes) as u64
    }

    fn read(&mut self, block: u64, into: &mut [u8]) -> Result<(), i32> {
        if into.len() != self.block_bytes {
            return Err(refusal::ADDRESS);
        }
        let (at, end) = self.span(block).ok_or(refusal::ADDRESS)?;
        into.copy_from_slice(&self.bytes[at..end]);
        Ok(())
    }

    fn write(&mut self, block: u64, from: &[u8]) -> Result<(), i32> {
        if from.len() != self.block_bytes {
            return Err(refusal::ADDRESS);
        }
        let (at, end) = self.span(block).ok_or(refusal::ADDRESS)?;
        self.bytes[at..end].copy_from_slice(from);
        self.writes += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), i32> {
        // Nothing to do: this device's stable media is the same memory the
        // writes landed in. What the barrier does here is admit it happened.
        self.flushes += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Device, Memory};
    use alloc::vec;
    use f_abi::store::refusal;

    #[test]
    fn a_block_outside_the_device_is_refused_rather_than_wrapped() {
        let mut device = Memory::new(512, 4);
        let mut block = vec![0u8; 512];

        assert_eq!(device.read(3, &mut block), Ok(()));
        assert_eq!(device.read(4, &mut block), Err(refusal::ADDRESS));
        assert_eq!(device.write(4, &block), Err(refusal::ADDRESS));
        // The arithmetic that would overflow rather than the index that is
        // merely too large, because those are two different mistakes and only
        // one of them is caught by comparing against `blocks()`.
        assert_eq!(device.read(u64::MAX, &mut block), Err(refusal::ADDRESS));
    }

    #[test]
    fn a_buffer_that_is_not_one_block_is_refused() {
        let mut device = Memory::new(512, 2);
        let mut short = vec![0u8; 511];
        assert_eq!(device.read(0, &mut short), Err(refusal::ADDRESS));
        assert_eq!(device.write(0, &short), Err(refusal::ADDRESS));
    }

    #[test]
    fn what_was_written_is_what_is_read_and_a_barrier_is_counted() {
        let mut device = Memory::new(512, 2);
        let mut written = vec![0u8; 512];
        written[0] = 0xAB;
        written[511] = 0xCD;
        device.write(1, &written).expect("block 1 is inside a two-block device");
        device.flush().expect("the model's barrier cannot fail");

        let mut read = vec![0u8; 512];
        device.read(1, &mut read).expect("block 1 again");
        assert_eq!(read, written);
        // Block 0 was never written and is still what an unformatted device
        // offers.
        device.read(0, &mut read).expect("block 0");
        assert!(read.iter().all(|byte| *byte == 0));
        assert_eq!(device.flushes(), 1);
        assert_eq!(device.writes(), 1);
    }

    #[test]
    fn a_flip_changes_one_byte_and_refuses_an_offset_the_device_does_not_have() {
        let mut device = Memory::new(512, 1);
        device.flip(0).expect("the first byte is inside the device");
        let mut read = vec![0u8; 512];
        device.read(0, &mut read).expect("block 0");
        assert_eq!(read[0], 1, "the lowest bit and nothing else");
        assert!(read[1..].iter().all(|byte| *byte == 0));
        assert_eq!(device.flip(512), Err(refusal::ADDRESS));
    }
}
