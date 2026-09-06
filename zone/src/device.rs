// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The four zone operations, and a device that answers them the way a
//! sequential-write-required device does — by refusing.
//!
//! # Why the model refuses rather than tolerates
//!
//! A zoned device is only worth modelling for the operations it *will not*
//! perform. A model that accepted a positioned write into a sequential zone, or
//! answered a read of a block past the write pointer, would let every mapping
//! bug above it pass: the mapping would compute an address, the model would
//! serve it, and the first disagreement would be on real hardware. So
//! [`ZonedMemory`] refuses a positioned write into a sequential zone, refuses a
//! read past a zone's write pointer, refuses an append to a full zone, and
//! zeroes a zone on reset. Each refusal is one of the constraints the mapping
//! exists to satisfy, and each is what makes the corresponding invariant a
//! predicate somebody can fail rather than a sentence somebody wrote.
//!
//! *The reset zeroes as well as moving the pointer, and both are load-bearing.*
//! The pointer is the device's own rule — RFC 0060's `ZONE_RESET` = 6, under
//! which "every byte previously in it becomes unreadable" — and the zeroing is
//! the second, independent tooth: a mapping that somehow resolved into a reset
//! zone past the pointer check would read zeros, and a zeroed block is refused
//! by every constructor in `f_abi::store`. RFC 0059's I2 is about beliefs as
//! well as bytes, and one mechanism guarding it is one mechanism to get wrong.
//!
//! # What this is not
//!
//! It is not QEMU's zoned virtio-blk, and this crate does not pretend it is.
//! The exit `E2-B02` closes on says *a zoned device or its emulation*; this is
//! the emulation, in the host, and `zone/tests/cycle.rs` says in its own first
//! paragraph which half of the evidence that leaves owed. The trait below is
//! shaped so that the owed half is a second implementation of it and not a
//! rewrite of anything above it: five methods, each one of RFC 0060's opcodes,
//! with the block half inherited from [`f_blob::device::Device`] rather than
//! restated.

use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::refusal;
use f_blob::device::Device;

/// What a zone permits, which is the only distinction that changes an address.
///
/// Two values and not the three a real report carries: `sequential write
/// preferred` exists in the standard and is refused here by omission, because a
/// zone that *permits* a positioned write but would rather not have one is a
/// zone whose contract a caller cannot check. A mapping written against a
/// preference would be a mapping whose correctness depends on a device's mood.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Writable at any address in it, and rewritable. The superblock's zone
    /// must be one of these, because a superblock that could never be
    /// rewritten could never be re-pointed — `f_blob::store::Store::format`
    /// records that obligation and this is where it is checkable.
    Conventional,
    /// Writable only at the write pointer, by append, and returned to the start
    /// only by a reset.
    SequentialWriteRequired,
}

/// Where a zone's write pointer is, in the three positions that change what an
/// operation means.
///
/// This is the *device's* state and is not RFC 0059's `state(z) ∈ {free, open,
/// sealed, condemned}`. The two are deliberately different types with different
/// names: the device knows whether bytes have been written, and the collector
/// knows whether it is allowed to take them. `condemned` has no device
/// counterpart at all, which is the clearest evidence that collapsing them
/// would be wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    /// Nothing written since the last reset.
    Empty,
    /// Some blocks written, more will fit.
    Open,
    /// No more will fit, or [`Zoned::finish`] said so.
    Full,
}

/// One zone, as `ZONE_REPORT` describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Which zone this describes.
    /// Unit: zone index, zero-based.
    pub zone: u32,
    /// What the zone permits.
    /// Unit: none — a classification, not a quantity.
    pub kind: Kind,
    /// The zone's first block.
    /// Unit: block index, zero-based, device-absolute.
    pub start: u64,
    /// How many blocks the zone holds.
    /// Unit: count of blocks.
    pub blocks: u64,
    /// The next block an append would land at, device-absolute. Equal to
    /// `start` on an empty zone and to `start + blocks` on a full one.
    /// Unit: block index, zero-based, device-absolute.
    pub write_pointer: u64,
    /// Where the pointer stands, in the terms an operation cares about.
    /// Unit: none — a state, not a quantity.
    pub state: Pointer,
}

/// The four zone opcodes RFC 0060 numbered, over the block device the store
/// already has.
///
/// # Why this extends `Device` rather than replacing it
///
/// Because the store above it writes blocks and must keep writing blocks: a
/// trait that replaced [`Device`] would make `f_blob::store::Store` generic
/// over two things that do the same job, and the first bug would be a caller
/// that picked the wrong one. What a zoned device *adds* is four operations and
/// a rule about where a write may land; the rule is enforced by this trait's
/// implementations refusing [`Device::write`] into a sequential zone, so a
/// caller cannot get at the flat address space by accident.
pub trait Zoned: Device {
    /// How many zones the device has.
    /// Unit: count of zones.
    fn zone_count(&self) -> u32;

    /// How many blocks are in one zone. Every zone is the same size, which is
    /// what lets a block index be divided rather than searched for.
    /// Unit: count of blocks.
    fn zone_blocks(&self) -> u64;

    /// What a zone permits, without the round trip [`Zoned::report`] is.
    ///
    /// On the trait rather than read off a report because a mapping asks it on
    /// every write, and because it is the one thing about a zone that never
    /// changes: a report's pointer is a moment's answer and a zone's kind is
    /// the device's geometry.
    fn kind(&self, zone: u32) -> Kind;

    /// `ZONE_REPORT` = 7. The device's own pointer, which nothing above it
    /// maintains a copy of — RFC 0060 refused the copy, because after a cut
    /// there would be two pointers to reconcile with no record of which was
    /// ahead.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone the device does not have.
    fn report(&mut self, zone: u32) -> Result<Report, i32>;

    /// `ZONE_APPEND` = 4. Write `from` at the zone's write pointer and answer
    /// the block it landed at.
    ///
    /// The zone names the *start* and never the destination: the device assigns
    /// the position. That is the whole reason this is not a positioned write.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone the device does not have, a conventional
    /// zone, or a buffer that is not one block long; [`refusal::FULL`] for a
    /// zone with no room left.
    fn append(&mut self, zone: u32, from: &[u8]) -> Result<u64, i32>;

    /// `ZONE_FINISH` = 5. Seal the zone: it accepts no further append, whatever
    /// room is left in it.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone the device does not have or a
    /// conventional zone.
    fn finish(&mut self, zone: u32) -> Result<(), i32>;

    /// `ZONE_RESET` = 6. Return the write pointer to the zone's start; every
    /// byte previously in it becomes unreadable.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a zone the device does not have or a
    /// conventional zone.
    fn reset(&mut self, zone: u32) -> Result<(), i32>;
}

/// What one device has been asked to do, in the counts a write-amplification
/// number is a ratio of.
///
/// Counted here and not one level up, because the spec defines a device byte as
/// *a byte crossing the blk ring in a `WRITE` or `ZONE_APPEND` entry* — at the
/// boundary, not at the caller. A counter above the boundary counts what the
/// caller meant to do; this counts what the device was asked for, which is the
/// only place both sides of a ratio can be counted the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counted {
    /// Bytes in `ZONE_APPEND` entries.
    /// Unit: bytes.
    pub appended_bytes: u64,
    /// Bytes in positioned `WRITE` entries — the superblock, and nothing else
    /// on a device whose data zones are all sequential.
    /// Unit: bytes.
    pub written_bytes: u64,
    /// Bytes in `READ` entries.
    /// Unit: bytes.
    pub read_bytes: u64,
    /// `ZONE_FINISH` entries.
    /// Unit: count of entries.
    pub finishes: u64,
    /// `ZONE_RESET` entries.
    /// Unit: count of entries.
    pub resets: u64,
    /// `FLUSH` entries.
    /// Unit: count of entries.
    pub flushes: u64,
}

impl Counted {
    /// Every byte this device was asked to put on media: the numerator of a
    /// write-amplification ratio, by the spec's own definition of a device
    /// byte.
    ///
    /// Unit: bytes.
    #[must_use]
    pub const fn device_bytes(&self) -> u64 {
        self.appended_bytes + self.written_bytes
    }
}

/// The modelled zoned device: blocks in memory, a write pointer per zone, and
/// four refusals.
///
/// Zone 0 is conventional and every other zone is sequential-write-required,
/// which is the geometry the spec requires of a device this format will mount:
/// the superblock is rewritable and nothing else is.
pub struct ZonedMemory {
    /// Unit: bytes.
    block_bytes: usize,
    /// Unit: count of blocks.
    zone_blocks: u64,
    /// Unit: count of zones.
    zone_count: u32,
    /// The device's contents.
    bytes: Vec<u8>,
    /// Each zone's write pointer, relative to that zone's start.
    /// Unit: count of blocks written since the last reset.
    written: Vec<u64>,
    /// Whether `ZONE_FINISH` has sealed each zone short of its capacity.
    sealed: Vec<bool>,
    /// What the device has been asked to do.
    counted: Counted,
}

impl ZonedMemory {
    /// A zeroed device of `zone_count` zones of `zone_blocks` blocks each.
    ///
    /// # Panics
    ///
    /// If the device would be larger than this machine can allocate, which is
    /// [`f_blob::device::Memory::new`]'s reasoning one level down: a test that
    /// asks for a device it cannot have has a wrong number in it, and a
    /// `Result` here would let it carry on with a smaller one.
    #[must_use]
    pub fn new(block_bytes: usize, zone_blocks: u64, zone_count: u32) -> Self {
        let blocks = zone_blocks * u64::from(zone_count);
        let len = block_bytes * usize::try_from(blocks).expect("a device this host can address");
        Self {
            block_bytes,
            zone_blocks,
            zone_count,
            bytes: vec![0u8; len],
            written: vec![0u64; zone_count as usize],
            sealed: vec![false; zone_count as usize],
            counted: Counted::default(),
        }
    }

    /// What this device has been asked to do.
    #[must_use]
    pub const fn counted(&self) -> &Counted {
        &self.counted
    }

    /// Which zone a device-absolute block lies in, or `None` for a block
    /// outside the device.
    ///
    /// Unit: zone index, zero-based.
    #[must_use]
    pub fn zone_of(&self, block: u64) -> Option<u32> {
        let zone = block / self.zone_blocks;
        u32::try_from(zone).ok().filter(|zone| *zone < self.zone_count)
    }

    /// What a zone permits. Zone 0 is the conventional one and there is exactly
    /// one, because the superblock is the only thing this format rewrites.
    #[must_use]
    pub const fn kind_of(zone: u32) -> Kind {
        if zone == 0 { Kind::Conventional } else { Kind::SequentialWriteRequired }
    }

    /// Flip the lowest bit of one byte, wherever it lies.
    ///
    /// Here for [`f_blob::device::Memory::flip`]'s reason and no other: a
    /// verifier that has never failed is indistinguishable from one that cannot
    /// fail, and a run that only ever reads back what it wrote never asks the
    /// verifier a question it could answer wrongly. It writes *underneath* the
    /// zone rules deliberately — no device offers this, and a corruption that
    /// had to be expressible as a device operation could not model the class of
    /// fault this exists for.
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

    /// The zone's first block, device-absolute.
    fn start_of(&self, zone: u32) -> u64 {
        u64::from(zone) * self.zone_blocks
    }

    /// Refuse a zone this device does not have, or one that is conventional
    /// when the operation is a zone operation.
    fn sequential(&self, zone: u32) -> Result<usize, i32> {
        if zone >= self.zone_count || Self::kind_of(zone) == Kind::Conventional {
            return Err(refusal::ADDRESS);
        }
        Ok(zone as usize)
    }
}

impl Device for ZonedMemory {
    fn block_bytes(&self) -> usize {
        self.block_bytes
    }

    fn blocks(&self) -> u64 {
        self.zone_blocks * u64::from(self.zone_count)
    }

    fn read(&mut self, block: u64, into: &mut [u8]) -> Result<(), i32> {
        if into.len() != self.block_bytes {
            return Err(refusal::ADDRESS);
        }
        let zone = self.zone_of(block).ok_or(refusal::ADDRESS)?;
        // The refusal that gives RFC 0059's I2 its teeth. A block past a
        // sequential zone's write pointer has never been written since the last
        // reset, and a device that answered it would be inventing bytes — so a
        // mapping that still resolves a hash into a zone the collector reset is
        // refused here, at the device, rather than caught upstairs by a hash
        // that happens not to match.
        if Self::kind_of(zone) == Kind::SequentialWriteRequired {
            let offset = block - self.start_of(zone);
            if offset >= self.written[zone as usize] {
                return Err(refusal::ADDRESS);
            }
        }
        let (at, end) = self.span(block).ok_or(refusal::ADDRESS)?;
        into.copy_from_slice(&self.bytes[at..end]);
        self.counted.read_bytes += self.block_bytes as u64;
        Ok(())
    }

    fn write(&mut self, block: u64, from: &[u8]) -> Result<(), i32> {
        if from.len() != self.block_bytes {
            return Err(refusal::ADDRESS);
        }
        let zone = self.zone_of(block).ok_or(refusal::ADDRESS)?;
        // A positioned write into a sequential zone is the operation the whole
        // mapping exists to avoid, and a model that tolerated it would let
        // every mapping bug above it through. It is refused rather than
        // translated into an append: translating would hide which caller had
        // the wrong idea about where its bytes go.
        if Self::kind_of(zone) == Kind::SequentialWriteRequired {
            return Err(refusal::ADDRESS);
        }
        let (at, end) = self.span(block).ok_or(refusal::ADDRESS)?;
        self.bytes[at..end].copy_from_slice(from);
        self.counted.written_bytes += self.block_bytes as u64;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), i32> {
        // Nothing to do: this device's stable media is the same memory the
        // writes landed in. What the barrier does here is admit it happened, so
        // that "the publish issued two barriers" is a number a test asserts
        // rather than a sentence in a comment.
        self.counted.flushes += 1;
        Ok(())
    }
}

impl Zoned for ZonedMemory {
    fn zone_count(&self) -> u32 {
        self.zone_count
    }

    fn zone_blocks(&self) -> u64 {
        self.zone_blocks
    }

    fn kind(&self, zone: u32) -> Kind {
        Self::kind_of(zone)
    }

    fn report(&mut self, zone: u32) -> Result<Report, i32> {
        if zone >= self.zone_count {
            return Err(refusal::ADDRESS);
        }
        let index = zone as usize;
        let start = self.start_of(zone);
        let written = self.written[index];
        let full = self.sealed[index] || written == self.zone_blocks;
        let state = if full {
            Pointer::Full
        } else if written == 0 {
            Pointer::Empty
        } else {
            Pointer::Open
        };
        Ok(Report {
            zone,
            kind: Self::kind_of(zone),
            start,
            blocks: self.zone_blocks,
            // A sealed zone's pointer is at its end, whatever room is left:
            // `ZONE_FINISH` is not a hint, and a mount that read the pointer to
            // find the last record must see the same number a reset would undo.
            write_pointer: if full { start + self.zone_blocks } else { start + written },
            state,
        })
    }

    fn append(&mut self, zone: u32, from: &[u8]) -> Result<u64, i32> {
        if from.len() != self.block_bytes {
            return Err(refusal::ADDRESS);
        }
        let index = self.sequential(zone)?;
        if self.sealed[index] || self.written[index] >= self.zone_blocks {
            return Err(refusal::FULL);
        }
        let at = self.start_of(zone) + self.written[index];
        let (from_byte, to_byte) = self.span(at).ok_or(refusal::ADDRESS)?;
        self.bytes[from_byte..to_byte].copy_from_slice(from);
        self.written[index] += 1;
        self.counted.appended_bytes += self.block_bytes as u64;
        Ok(at)
    }

    fn finish(&mut self, zone: u32) -> Result<(), i32> {
        let index = self.sequential(zone)?;
        self.sealed[index] = true;
        self.counted.finishes += 1;
        Ok(())
    }

    fn reset(&mut self, zone: u32) -> Result<(), i32> {
        let index = self.sequential(zone)?;
        let start = self.start_of(zone);
        let (from_byte, _) = self.span(start).ok_or(refusal::ADDRESS)?;
        let len =
            usize::try_from(self.zone_blocks).map_err(|_| refusal::ADDRESS)? * self.block_bytes;
        // Zeroed as well as re-pointed. The pointer is the device's rule and the
        // zeroing is the second, independent tooth: a mapping that somehow got
        // past the pointer check would read zeros, and a zeroed block is
        // refused by every constructor in `f_abi::store`. Two mechanisms, so
        // that I2 does not rest on one of them being right.
        self.bytes[from_byte..from_byte + len].fill(0);
        self.written[index] = 0;
        self.sealed[index] = false;
        self.counted.resets += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Pointer, Zoned, ZonedMemory};
    use alloc::vec;
    use f_abi::store::refusal;
    use f_blob::device::Device;

    const BLOCK: usize = 512;

    fn device() -> ZonedMemory {
        ZonedMemory::new(BLOCK, 4, 3)
    }

    #[test]
    fn a_positioned_write_into_a_sequential_zone_is_refused_rather_than_translated() {
        let mut zoned = device();
        let block = vec![7u8; BLOCK];
        // Zone 0 is conventional and takes it.
        assert_eq!(zoned.write(0, &block), Ok(()));
        // Zone 1 begins at block 4 and does not.
        assert_eq!(zoned.write(4, &block), Err(refusal::ADDRESS));
        assert_eq!(zoned.append(1, &block), Ok(4));
    }

    #[test]
    fn an_append_answers_the_position_the_device_chose() {
        let mut zoned = device();
        let block = vec![1u8; BLOCK];
        assert_eq!(zoned.append(2, &block), Ok(8));
        assert_eq!(zoned.append(2, &block), Ok(9));
        assert_eq!(zoned.report(2).map(|r| r.write_pointer), Ok(10));
        assert_eq!(zoned.report(2).map(|r| r.state), Ok(Pointer::Open));
    }

    #[test]
    fn a_full_zone_refuses_an_append_and_a_finished_one_refuses_it_early() {
        let mut zoned = device();
        let block = vec![1u8; BLOCK];
        for _ in 0..4 {
            zoned.append(1, &block).expect("four blocks fit in a four-block zone");
        }
        assert_eq!(zoned.append(1, &block), Err(refusal::FULL));
        assert_eq!(zoned.report(1).map(|r| r.state), Ok(Pointer::Full));

        zoned.append(2, &block).expect("one block into an empty zone");
        zoned.finish(2).expect("a sequential zone can be sealed");
        assert_eq!(zoned.append(2, &block), Err(refusal::FULL), "sealed short of capacity");
        assert_eq!(zoned.report(2).map(|r| r.write_pointer), Ok(12), "the pointer is at the end");
    }

    /// The refusal RFC 0059's I2 rests on, tested from both sides of the reset.
    #[test]
    fn a_read_past_the_write_pointer_is_refused_and_a_reset_moves_the_pointer_back() {
        let mut zoned = device();
        let written = vec![9u8; BLOCK];
        let mut read = vec![0u8; BLOCK];
        zoned.append(1, &written).expect("block 4");

        assert_eq!(zoned.read(4, &mut read), Ok(()));
        assert_eq!(read, written);
        assert_eq!(zoned.read(5, &mut read), Err(refusal::ADDRESS), "never written");

        zoned.reset(1).expect("a sequential zone can be reset");
        assert_eq!(zoned.read(4, &mut read), Err(refusal::ADDRESS), "unreadable after the reset");
        assert_eq!(zoned.report(1).map(|r| r.state), Ok(Pointer::Empty));
        // And the second tooth: the bytes are gone as well as unaddressable, so
        // a mapping that got past the pointer check would read a zeroed block
        // rather than a record it could still believe.
        zoned.append(1, &vec![0u8; BLOCK]).expect("block 4 again");
        zoned.read(4, &mut read).expect("block 4 is inside the pointer again");
        assert!(read.iter().all(|byte| *byte == 0), "the reset zeroed what was there");
    }

    #[test]
    fn a_conventional_zone_answers_no_zone_operation() {
        let mut zoned = device();
        let block = vec![1u8; BLOCK];
        assert_eq!(zoned.append(0, &block), Err(refusal::ADDRESS));
        assert_eq!(zoned.finish(0), Err(refusal::ADDRESS));
        assert_eq!(zoned.reset(0), Err(refusal::ADDRESS));
        assert_eq!(zoned.report(0).map(|r| r.kind), Ok(Kind::Conventional));
        assert_eq!(zoned.report(3), Err(refusal::ADDRESS), "a zone the device does not have");
    }

    #[test]
    fn what_the_device_was_asked_for_is_counted_at_the_device() {
        let mut zoned = device();
        let block = vec![1u8; BLOCK];
        zoned.write(0, &block).expect("the conventional zone");
        zoned.append(1, &block).expect("a sequential one");
        zoned.flush().expect("the model's barrier cannot fail");
        let counted = *zoned.counted();
        assert_eq!(counted.written_bytes, BLOCK as u64);
        assert_eq!(counted.appended_bytes, BLOCK as u64);
        assert_eq!(counted.device_bytes(), 2 * BLOCK as u64);
        assert_eq!(counted.flushes, 1);
    }
}
