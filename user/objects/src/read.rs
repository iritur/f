// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Hash to zone and offset, and the device landing the bytes where the caller
//! already had them.
//!
//! # The shape of one read, in the order it happens
//!
//! 1. A name resolves to a hash through [`Index`], which is a `&self` call on a
//!    map this component already holds. No block is read and no ring is
//!    crossed — `index/src/lib.rs` argues that at length and this is its one
//!    consumer.
//! 2. The hash resolves to a *logical* block through `Store::address`, and that
//!    block resolves to a **zone and a device-absolute offset** through
//!    `f_zone::map::ZoneMap`. [`Placement`] is that answer, and it is the
//!    task's own sentence — *hash to zone and offset* — as a value a test can
//!    assert on rather than a step buried in a transfer.
//! 3. The record's blocks are read one submission each, every one of them
//!    naming a buffer of the set the **caller** registered. The device writes
//!    into the caller's memory; this component never sees the bytes.
//! 4. The blob header is decoded **in place**, out of the first bytes the
//!    device just wrote, by borrowing them as a `&[u8; Header::BYTES]` and
//!    handing that to `f_abi::store::Header::from_bytes`. Not a cast — RFC 0001
//!    puts a pointer cast inside the frame and this crate is not in the frame —
//!    and not a copy either, which is the half a reader should check: nothing
//!    is moved, the constructor reads fields out of memory it borrows, and no
//!    byte the device wrote is believed before it returns `Ok`.
//! 5. The content is hashed **in the caller's buffer** and compared against the
//!    name it was asked for. Verify before accept, at the granularity a blob
//!    has one, and it is what makes a zero-copy read worth as much as a copied
//!    one — a design that had to copy in order to check would have bought its
//!    safety with the number this task is about.
//!
//! # Why the geometry is one buffer per block
//!
//! Because that is what a submission names. RFC 0024's registered path names
//! `(set, index)` and the service resolves the pair; a driver reading four
//! blocks issues four entries with four indices, and the table refuses a second
//! submission naming a buffer the device already holds. A set of block-sized
//! buffers is therefore the honest geometry, and it has a second property this
//! file depends on: buffers of one set are contiguous in the client's region,
//! so a record spread across `n` of them is one run of bytes the caller reads
//! without reassembling anything.
//!
//! # The count, and the reading this file owns
//!
//! [`Counters::staged_bytes`] is the **driver's** reading of *bytes moved
//! through any buffer that is not the caller's registered one*, and it is taken
//! at submission: this side knows, per entry, whether it named a registered
//! buffer or handed over memory of its own. `f_objects::dma::Landed::unregistered_bytes`
//! is the **device model's**, taken where the bytes actually move.
//! [`ReadPath::readings_agree`] requires the two to be equal, which is claim
//! 0012's discipline of counting one event on both sides of a boundary —
//! because a zero one component published is a zero that component can produce
//! by writing it.
//!
//! **Where the count's boundary is, because a reviewer will ask.** What is
//! tallied is *bytes that came to rest somewhere that is not the caller's
//! registered buffer* — a staging buffer, a cache page, an inline payload.
//! `f_hash::sha256` reads the content out of that buffer sixty-four bytes at a
//! time into its own compression state, and that is deliberately **not** a
//! copy by this definition: nothing holds a second copy of the record
//! afterwards, the state is 32 bytes whatever the record's length, and a
//! definition that counted it would count every consumer of the bytes and
//! could never be zero for any design at all. The definition that can be zero,
//! and the one `intent/0006-state/spec.md` states, is about *buffers*.
//!
//! [`ReadPath::provoke_second_copy`] is the page cache, modelled, and it exists
//! so that a zero is evidence: it reads a block into memory this component owns
//! and then moves it on into the caller's buffer, which is exactly the second
//! copy the exit denies. Both readings move, by the same bytes.

use alloc::vec;
use alloc::vec::Vec;

use f_abi::buf::SetId;
use f_abi::store::{Header, refusal};
use f_blob::device::Device;
use f_blob::store::Store;
use f_hash::sha256;
use f_index::{Index, ns};
use f_zone::device::Zoned;
use f_zone::map::ZoneMap;

use crate::dma::Landing;
use crate::resident::Resident;

/// Where a hash lives, in the two coordinates the task names.
///
/// The device-absolute figures describe the record's **first** block. A record
/// occupies consecutive *logical* blocks, and a mapping is free to place those
/// in different zones — a collector that copied one of them forward would do
/// exactly that — so a placement that claimed to describe the whole record
/// would be claiming something the mapping does not promise. What the transfer
/// names is logical blocks, which `ZoneMap` resolves one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The block in the store's flat address space.
    /// Unit: block index, zero-based.
    pub logical: u64,
    /// The zone the first block lives in.
    /// Unit: zone index, zero-based.
    pub zone: u32,
    /// The first block, device-absolute.
    /// Unit: block index, zero-based.
    pub block: u64,
    /// Where the record starts on the device.
    /// Unit: bytes, from the start of the device.
    pub offset_bytes: u64,
}

/// What one read delivered, said as coordinates into the caller's own buffer.
///
/// The content is **not** moved to the start of the buffer, and that is the
/// decision rather than an oversight: the record's header lands with it,
/// because the device writes whole blocks and the header is the first fifty-two
/// bytes of the first one. Sliding the content down would be a copy of every
/// byte read — the exact copy this task exists to not make — so the read
/// answers where the content is instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    /// Where the content begins in the run of buffers the read filled.
    /// Unit: bytes, from the first buffer of the run.
    pub content_at: usize,
    /// How much of it there is.
    /// Unit: bytes.
    pub content_bytes: usize,
    /// How many buffers the read filled, one per block.
    /// Unit: count of buffers.
    pub buffers: u32,
    /// Where the record was found.
    /// Unit: none — a pair of coordinates, each of which states its own.
    pub placement: Placement,
}

/// What the driver did, in counts.
///
/// Counts and never durations, for `f_virtio_blk::driver::Counters`' reason one
/// crate over: a number that moved with the host would take every fixture that
/// hashes it along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Reads completed, content verified.
    /// Unit: count of reads.
    pub reads: u64,
    /// Application bytes delivered — the denominator of every ratio here.
    /// Unit: bytes.
    pub content_bytes: u64,
    /// Blocks the driver submitted.
    /// Unit: count of blocks.
    pub blocks: u64,
    /// Bytes the driver submitted to a destination that is **not** the caller's
    /// registered buffer.
    ///
    /// **Required to be zero on the read path**, and it is a tally rather than
    /// a property proved by construction: [`ReadPath::provoke_second_copy`] can
    /// move it and nothing on the read path can, so a build in which the read
    /// path had grown a staging step reports it. It is the driver's half of the
    /// two readings the module comment names, and
    /// [`ReadPath::readings_agree`] is what requires the halves to be equal.
    /// Unit: bytes.
    pub staged_bytes: u64,
}

/// The read path.
///
/// Generic over two devices rather than one, and the reason is worth stating
/// because the obvious diff collapses them: the store's device is zoned and the
/// index's log is a flat block region, and they are the same physical device on
/// a mounted system. Modelling them as one would mean this crate deciding where
/// the index log sits on a zoned device, which is the mount's decision and is
/// `E2-B02`'s, not this file's.
pub struct ReadPath<Z: Zoned, I: Device> {
    store: Store<ZoneMap<Z>>,
    index: Index<I>,
    counters: Counters,
    resident: Resident,
}

/// Put a store and an index together into the read path over them.
///
/// A free function and not only a constructor, because *mount* is the word the
/// plan uses for this step and because it is the one place the whole stack is
/// named in one line — which is what a reader looking for where the read path
/// comes from will search for.
pub fn mount<Z: Zoned, I: Device>(store: Store<ZoneMap<Z>>, index: Index<I>) -> ReadPath<Z, I> {
    ReadPath::over(store, index)
}

impl<Z: Zoned, I: Device> ReadPath<Z, I> {
    /// The read path over a store and an index.
    pub fn over(store: Store<ZoneMap<Z>>, index: Index<I>) -> Self {
        let mut resident = Resident::default();
        resident.mounted(index.names(ns::PATH), store.records());
        Self { store, index, counters: Counters::default(), resident }
    }

    /// What the driver did.
    #[must_use]
    pub const fn counters(&self) -> &Counters {
        &self.counters
    }

    /// What this component holds resident, and against what work.
    #[must_use]
    pub const fn resident(&self) -> &Resident {
        &self.resident
    }

    /// The store, for a caller with business there — a workload writing the
    /// blobs it is about to read, a control that corrupts a byte.
    pub fn store_mut(&mut self) -> &mut Store<ZoneMap<Z>> {
        &mut self.store
    }

    /// The store.
    #[must_use]
    pub const fn store(&self) -> &Store<ZoneMap<Z>> {
        &self.store
    }

    /// The index.
    #[must_use]
    pub const fn index(&self) -> &Index<I> {
        &self.index
    }

    /// Re-read what the mount costs in memory.
    ///
    /// Called after a workload has written into the store and the index,
    /// because both maps grow and a residency figure taken before they did
    /// would be a figure about an empty component.
    pub fn remount(&mut self) {
        self.resident.mounted(self.index.names(ns::PATH), self.store.records());
    }

    /// A path to the hash of the object at it.
    ///
    /// The whole of what the index costs a read, and it is a `&self` call: no
    /// block, no entry, no ring.
    #[must_use]
    pub fn resolve(&self, name: &[u8]) -> Option<[u8; 32]> {
        self.index.get(ns::PATH, name)
    }

    /// Hash to zone and offset.
    ///
    /// `None` for a hash this store holds nothing under, and for a logical
    /// block the mapping has not placed — which are two different absences and
    /// are deliberately not told apart here: a caller that could distinguish
    /// them would be able to ask whether a hash it does not hold exists.
    #[must_use]
    pub fn locate(&self, hash: &[u8; 32]) -> Option<Placement> {
        let logical = self.store.address(hash)?;
        let map = self.store.device();
        let block = map.physical(logical)?;
        let zone = map.zone_of(logical)?;
        let block_bytes = u64::from(self.store.superblock().block_bytes);
        Some(Placement { logical, zone, block, offset_bytes: block * block_bytes })
    }

    /// Read the blob stored under `hash` into the caller's registered buffers,
    /// starting at buffer `first`.
    ///
    /// The device writes the bytes; this component moves none of them. What it
    /// does is decide which blocks, name which buffers, decode the header out
    /// of the memory the device just wrote, and hash the content there before
    /// saying `Ok`.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a hash this store holds nothing under or a
    /// block the mapping has not placed; [`refusal::MALFORMED`] or
    /// [`refusal::UNKNOWN`] for bytes that are not a header this build can
    /// read; [`refusal::CONTENT`] when the header found there names a different
    /// blob, or when the content read back does not hash to `hash`;
    /// [`refusal::SHORT_BUFFER`] when the run of buffers from `first` cannot
    /// hold the whole record, which is refused **before** a block is read
    /// rather than after; whatever the registration refuses a name with; and
    /// whatever the device says about a read.
    pub fn read(
        &mut self,
        hash: &[u8; 32],
        landing: &mut Landing<'_>,
        set: SetId,
        first: u32,
    ) -> Result<Read, i32> {
        let placement = self.locate(hash).ok_or(refusal::ADDRESS)?;
        let block_bytes = self.store.superblock().block_bytes as usize;
        if landing.stride() != block_bytes {
            // A buffer that is not one block is a geometry this path cannot
            // submit: the device moves whole blocks, so a shorter buffer would
            // have to be filled by something other than the device and a longer
            // one would be a buffer the device only partly wrote.
            return Err(refusal::SHORT_BUFFER);
        }

        // The first block, into the first buffer. Its header is what says how
        // many more there are, so it is read before the size of the transfer is
        // known — which is the one thing a read of a content-addressed record
        // cannot avoid, and it is one block rather than a probe of its own.
        let window = landing.lend(set, first, block_bytes)?;
        self.store.device_mut().read(placement.logical, window)?;
        landing.release(set, first)?;
        self.counters.blocks = self.counters.blocks.saturating_add(1);

        let head = landing.buffer(first).ok_or(refusal::ADDRESS)?;
        let header = header_in(head, hash)?;
        let content_bytes =
            usize::try_from(header.content_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        let record_bytes = Header::BYTES.checked_add(content_bytes).ok_or(refusal::SHORT_BUFFER)?;
        let blocks = record_bytes.div_ceil(block_bytes);
        let blocks = u32::try_from(blocks).map_err(|_| refusal::SHORT_BUFFER)?;
        // Refused before the second block is read, not after: a run that
        // discovered halfway through that it had nowhere to put the rest would
        // have already written into somebody's buffer.
        if landing.contents(first, blocks).is_none() {
            return Err(refusal::SHORT_BUFFER);
        }

        for step in 1..blocks {
            let window = landing.lend(set, first + step, block_bytes)?;
            self.store.device_mut().read(placement.logical + u64::from(step), window)?;
            landing.release(set, first + step)?;
            self.counters.blocks = self.counters.blocks.saturating_add(1);
        }

        // Verify before accept, in the caller's own memory. Everything above
        // this line is a length a device wrote; this is what says the bytes are
        // the bytes — and it costs no copy, which is the half worth noticing:
        // a design that had to stage the record in order to check it would have
        // paid for its safety with the number this task records.
        let filled = landing.contents(first, blocks).ok_or(refusal::SHORT_BUFFER)?;
        let content = filled
            .get(Header::BYTES..Header::BYTES + content_bytes)
            .ok_or(refusal::SHORT_BUFFER)?;
        if sha256(content) != *hash {
            return Err(refusal::CONTENT);
        }

        self.counters.reads = self.counters.reads.saturating_add(1);
        self.counters.content_bytes =
            self.counters.content_bytes.saturating_add(content_bytes as u64);
        self.resident.delivered(content_bytes as u64);
        Ok(Read { content_at: Header::BYTES, content_bytes, buffers: blocks, placement })
    }

    /// Do the read the page cache would have done: land a block in memory this
    /// component owns, and move it on into the caller's buffer.
    ///
    /// # Why the read path has a provocation at all
    ///
    /// Because `copies_per_read = 0` published by a build whose tally had been
    /// deleted reads exactly like `copies_per_read = 0` published by a build
    /// that is zero-copy, and the exit says *counted rather than asserted*. So
    /// this moves both readings on purpose, and a run that publishes a zero
    /// beside this non-zero is publishing evidence. It is the same argument
    /// `kernel/src/mem.rs` makes with `provoke_remote` and
    /// `f_virtio_blk::driver::provoke_copy` makes one crate over, with one
    /// difference: there the provocation moves a *second* counter beside the
    /// one that must stay zero, and here it moves the very counter the claim is
    /// about — because `intent/0006-state/spec.md` asks for a count that can go
    /// non-zero, not for a second number beside one that cannot.
    ///
    /// The bytes it moves are a real block of the record, so what this models
    /// is a cache that is *working* rather than a scribble: the caller ends up
    /// holding the same bytes it would have held, and the only difference is
    /// that they were moved twice and both readings say so.
    ///
    /// # Errors
    ///
    /// Every refusal [`ReadPath::read`] can make about a placement, plus
    /// [`refusal::ADDRESS`] for a buffer index the set does not have.
    pub fn provoke_second_copy(
        &mut self,
        hash: &[u8; 32],
        landing: &mut Landing<'_>,
        into: u32,
    ) -> Result<u64, i32> {
        let placement = self.locate(hash).ok_or(refusal::ADDRESS)?;
        let block_bytes = self.store.superblock().block_bytes as usize;
        // Memory this component owns, allocated here and dropped here. The read
        // path holds no such buffer between calls, which is why
        // `Resident::held` is zero for it and non-zero for the duration of this
        // one — a residency figure is a peak and not an average.
        let mut cache: Vec<u8> = vec![0u8; block_bytes];
        self.store.device_mut().read(placement.logical, &mut cache)?;
        // The driver's reading, taken at the submission: this entry named no
        // registered buffer at all.
        self.counters.staged_bytes = self.counters.staged_bytes.saturating_add(block_bytes as u64);
        self.resident.held(block_bytes as u64);
        // And the device model's, taken where the bytes move. Nothing is passed
        // across: the two tallies are moved by two statements in two crates'
        // worth of separate bookkeeping, which is what makes their agreement a
        // check rather than a restatement.
        landing.provoke_copy(&cache, into)?;
        Ok(block_bytes as u64)
    }

    /// Do the two readings of the same event agree?
    ///
    /// Claim 0012's discipline: one event counted on both sides of a boundary,
    /// and a disagreement is a failure rather than a note. A driver that named
    /// a registered buffer and staged the bytes anyway moves one and not the
    /// other; so does a build in which either tally stopped working.
    #[must_use]
    pub fn readings_agree(&self, landing: &Landing<'_>) -> bool {
        self.counters.staged_bytes == landing.landed().unregistered_bytes
    }
}

/// Decode the header out of the bytes the device wrote, and refuse a header
/// that names a different blob.
///
/// **Borrowed, not copied.** `TryFrom<&[u8]> for &[u8; N]` hands back a
/// reference into the caller's own registered buffer, so
/// `Header::from_bytes` reads its fields out of the memory the device just
/// wrote and nothing is moved anywhere. That is what
/// `intent/0006-state/spec.md` means by *the blob header that lands by DMA in
/// the caller's registered buffer is decoded from those bytes there and never
/// viewed*: not a `repr(C)` cast, which RFC 0001 puts inside the frame, and not
/// a fifty-two-byte copy either, which would be a copy nobody counted.
///
/// The second check is not redundant beside the content hash: it catches a
/// device that returned the wrong block *before* the length field in that block
/// is used to decide how much more to read.
fn header_in(block: &[u8], hash: &[u8; 32]) -> Result<Header, i32> {
    let raw: &[u8; Header::BYTES] = block
        .get(..Header::BYTES)
        .ok_or(refusal::MALFORMED)?
        .try_into()
        .map_err(|_| refusal::MALFORMED)?;
    let header = Header::from_bytes(raw)?;
    if header.hash != *hash {
        return Err(refusal::CONTENT);
    }
    Ok(header)
}
