// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The store: blobs and objects on a device, named by what they contain.
//!
//! # No byte from the device is trusted until a constructor returned `Ok`
//!
//! That is the rule this file is arranged around, and it is not a style. The
//! device is a peer in the sense `ring/src/mapping.rs` means — somebody else
//! wrote these bytes, possibly a long time ago, possibly across a power cut —
//! so every read decodes through `f_abi::store`'s validating constructors,
//! which refuse on the magic, the declared length and the kind before they
//! return any field. Above that, [`Store::get`] hashes the content it read and
//! refuses when the digest is not the name it was asked for. There is no path
//! through this file where a length field a device wrote is used before it has
//! been believed.
//!
//! # What a publish is, and which half is here
//!
//! RFC 0060: **write the blobs, `FLUSH`, append the root record, `FLUSH`**. The
//! first half is here — [`Store::put`], [`Store::put_object`] and then
//! [`Store::barrier`]. The second half is not, because a root record is
//! appended to a root *zone* and this crate knows nothing about zones. `E2-B02`
//! puts `f-zone` between this and the device; a store that knew about zones
//! would be a second crate that could disagree with it about an offset.
//!
//! # Allocation, sequentially, from the first free block
//!
//! A record occupies whole blocks and the next one starts after it. Nothing is
//! ever overwritten and nothing is ever freed here: reclamation is the
//! collector's under RFC 0059, and it works on zones rather than on records.
//! What that costs a reader of this file is that [`Store`] holds a
//! `BTreeMap` from hash to block — an index in memory, rebuilt by nothing,
//! surviving no restart. `E2-B03` is the durable index, and the map here is
//! deliberately the smallest thing that makes `put` and `get` a pair rather
//! than a rehearsal for one.
//!
//! # Determinism
//!
//! Nothing in this file reads a clock, draws a value or asks an `Env` for
//! anything. A `BTreeMap` and not a `HashMap`, RFC 0004, and the ordering
//! matters here for a second reason beyond the lint: the map is walked when a
//! test reports what it stored, and a walk in seeded-hash order would make two
//! runs of the same seed print different things.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::{CHUNKS_MAX, Header, ObjectHead, SCHEMA, Superblock, kind, refusal};
use f_env::split::label;
use f_hash::sha256;

use crate::chunk::{CHUNK_MAX_BYTES, CHUNK_MIN_BYTES, CHUNK_TARGET_BYTES, Chunker, MASK_BITS};
use crate::device::Device;
use crate::gear::{GEAR_LABEL, MASK_LABEL};

/// The identity `gear.rs` derives the gear table from.
///
/// A `const` and not a call, which is the crate doc's claim made mechanical:
/// `f_env::split::label` is a `const fn`, so this is evaluated by the compiler
/// and this crate names the determinism substrate at compile time and never
/// while it runs. If `f_env` ever drops `const` from `label`, this line stops
/// compiling rather than quietly moving the work to run time.
/// Unit: none — an identity, not a quantity.
const GEAR_IDENTITY: u64 = label(GEAR_LABEL);

/// The identity `gear.rs` derives the mask from.
/// Unit: none — an identity, not a quantity.
const MASK_IDENTITY: u64 = label(MASK_LABEL);

/// The smallest block this store will format a device with.
///
/// Unit: bytes. Every record in `f_abi::store` fits inside one block and is
/// asserted at compile time to — a record spanning two blocks could not have
/// its magic believed until both had been read, which is a reader that reads
/// twice before it disbelieves anything. Five hundred and twelve is also the
/// smallest logical block any device this project will meet reports.
pub const MIN_BLOCK_BYTES: u32 = 512;

/// The superblock this build would write, given a device's geometry.
///
/// The chunker's parameters come from this build and are not a caller's to
/// choose: a superblock whose parameters were an argument would let a caller
/// format a device this build cannot chunk for, which is precisely the
/// situation the fields exist to make impossible. The geometry *is* the
/// caller's, because this crate cannot see a zone — `f-zone` at `E2-B02` is
/// what will know how many zones a device has and how large they are, and until
/// then the numbers are recorded as they were handed over rather than checked.
#[must_use]
pub fn superblock_for_this_build(
    block_bytes: u32,
    zones: u32,
    zone_bytes: u64,
    root_zone_a: u32,
    root_zone_b: u32,
) -> Superblock {
    Superblock {
        schema: SCHEMA,
        block_bytes,
        zones,
        root_zone_a,
        root_zone_b,
        chunk_min_bytes: CHUNK_MIN_BYTES as u32,
        chunk_target_bytes: CHUNK_TARGET_BYTES as u32,
        chunk_max_bytes: CHUNK_MAX_BYTES as u32,
        mask_bits: MASK_BITS,
        zone_bytes,
        gear_label: GEAR_IDENTITY,
        mask_label: MASK_IDENTITY,
    }
}

/// Refuse a device whose chunker is not this build's.
///
/// # Why this is a refusal and not a warning
///
/// Because the alternative is silent. A chunker that changed under a mount
/// still works: it cuts, it hashes, it stores, and every object it writes is
/// deduplicated against nothing that came before because the boundaries moved.
/// The device would fill with two populations of chunks covering the same
/// bytes, and the first symptom would be a capacity number nobody could
/// explain. So the six parameters that decide a boundary are on the device, and
/// this compares them.
///
/// It is here rather than in `abi/` because `abi/` does not know what any
/// build's chunker is, and it is public because `E2-B02`'s mount is its real
/// caller: this crate has no mount of its own, for the reason [`Store`]'s own
/// documentation gives.
///
/// # Errors
///
/// [`refusal::UNKNOWN`] naming a device this build cannot read: the parameters
/// are well-formed and are not ones this build could reproduce.
pub fn refuse_a_chunker_that_moved(found: &Superblock) -> Result<(), i32> {
    let mine = superblock_for_this_build(
        found.block_bytes,
        found.zones,
        found.zone_bytes,
        found.root_zone_a,
        found.root_zone_b,
    );
    if found.chunk_min_bytes != mine.chunk_min_bytes
        || found.chunk_target_bytes != mine.chunk_target_bytes
        || found.chunk_max_bytes != mine.chunk_max_bytes
        || found.mask_bits != mine.mask_bits
        || found.gear_label != mine.gear_label
        || found.mask_label != mine.mask_label
    {
        return Err(refusal::UNKNOWN);
    }
    Ok(())
}

/// Blobs and objects on one device.
///
/// # Why there is no `mount` in this crate
///
/// A mount reads both root zones' write pointers with `ZONE_REPORT`, scans
/// backwards from each, and takes the highest generation whose check verifies
/// and whose generation tree resolves. Every clause of that sentence is about a
/// zone, and this crate has none: the format does not maintain a write pointer
/// — that is the whole reason mount holds no mutable state on the device — so a
/// mount written here would have to invent one and then disagree with the
/// device about it. `E2-B02` is where mount goes, and
/// [`refuse_a_chunker_that_moved`] is the part of it that lives here because it
/// is about this build's chunker rather than about the device's zones.
pub struct Store<D: Device> {
    device: D,
    superblock: Superblock,
    /// The first block no record occupies. Unit: block index, zero-based.
    free: u64,
    /// Where each blob starts. Unit: block index, zero-based, per content
    /// address.
    located: BTreeMap<[u8; 32], u64>,
}

impl<D: Device> Store<D> {
    /// Write the superblock and return a store over the device.
    ///
    /// The superblock goes in block zero, which the spec requires to be a
    /// *conventional* zone: a superblock in a sequential-write-required zone
    /// could never be rewritten and so could never be re-pointed. Nothing here
    /// can check that — a zone type is `ZONE_REPORT`'s answer and this crate
    /// does not speak it — so `E2-B02`'s mount refuses a device whose zone 0 is
    /// sequential, and this comment is where the obligation is recorded.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for a block size the format cannot use or a
    /// superblock that does not describe the device it is being written to;
    /// [`refusal::UNKNOWN`] for a chunker that is not this build's;
    /// [`refusal::FULL`] for a device with no room for a superblock and a
    /// record after it.
    pub fn format(device: D, layout: &Superblock) -> Result<Self, i32> {
        if layout.block_bytes < MIN_BLOCK_BYTES
            || layout.block_bytes as usize != device.block_bytes()
        {
            return Err(refusal::MALFORMED);
        }
        if device.blocks() < 2 {
            return Err(refusal::FULL);
        }
        refuse_a_chunker_that_moved(layout)?;

        let mut store = Self { device, superblock: *layout, free: 1, located: BTreeMap::new() };
        let mut block = vec![0u8; store.superblock.block_bytes as usize];
        block[..Superblock::BYTES].copy_from_slice(&layout.to_bytes());
        store.device.write(0, &block)?;
        // The superblock is durable before anything names it, which is the same
        // ordering a publish has and the reason it is written here rather than
        // left to the first barrier a caller happens to issue.
        store.device.flush()?;
        Ok(store)
    }

    /// What this store was formatted with.
    #[must_use]
    pub const fn superblock(&self) -> &Superblock {
        &self.superblock
    }

    /// The first block no record occupies.
    ///
    /// Unit: block index, zero-based.
    #[must_use]
    pub const fn free(&self) -> u64 {
        self.free
    }

    /// How many distinct blobs this store holds.
    ///
    /// Unit: count of blobs. Distinct is the load-bearing word: two `put`s of
    /// the same content are one record, which is deduplication and is also what
    /// makes this number worth asserting in a test that draws its content.
    #[must_use]
    pub fn records(&self) -> usize {
        self.located.len()
    }

    /// The block a hash's record starts at, or `None` for a hash this store
    /// holds nothing under.
    ///
    /// Unit: block index, zero-based. This is what an index records — `E2-B03`
    /// makes it durable and `E2-B08` reads a blob by it — and it is also what
    /// `blob/tests/million.rs`'s control needs, because a byte flipped into a
    /// record's padding would refuse nothing and the control would pass while
    /// proving nothing.
    #[must_use]
    pub fn address(&self, hash: &[u8; 32]) -> Option<u64> {
        self.located.get(hash).copied()
    }

    /// The device, for a caller that has business with it — a cut model, a
    /// counter, a control that corrupts a byte.
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    /// The device.
    #[must_use]
    pub const fn device(&self) -> &D {
        &self.device
    }

    /// The barrier: the first half of RFC 0060's publish sequence ends here.
    ///
    /// Every blob written before this call is on stable media when it returns,
    /// so a root record appended after it names hashes that exist. The second
    /// barrier — the one that makes the root record itself durable — is
    /// `f-zone`'s, because the record it covers is appended to a zone.
    ///
    /// # Errors
    ///
    /// Whatever the device says. There is no partial barrier.
    pub fn barrier(&mut self) -> Result<(), i32> {
        self.device.flush()
    }

    /// Store `content` under its own hash, and answer that hash.
    ///
    /// Content already present is not written twice: two objects sharing a run
    /// of bytes share the chunks covering it, and that falls out of the naming
    /// rather than being arranged. The second `put` still costs a hash, because
    /// the name *is* the hash and there is no cheaper way to know.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] for a kind this build does not know;
    /// [`refusal::FULL`] when the record would not fit on the device; whatever
    /// the device says about a write.
    pub fn put(&mut self, kind: u16, content: &[u8]) -> Result<[u8; 32], i32> {
        if !kind::known(kind) {
            return Err(refusal::UNKNOWN);
        }
        let hash = sha256(content);
        if self.located.contains_key(&hash) {
            return Ok(hash);
        }

        let block_bytes = self.superblock.block_bytes as usize;
        let record_bytes = Header::BYTES + content.len();
        let blocks = record_bytes.div_ceil(block_bytes) as u64;
        if self.free.checked_add(blocks).is_none_or(|end| end > self.device.blocks()) {
            return Err(refusal::FULL);
        }

        let header = Header { kind, flags: 0, content_bytes: content.len() as u64, hash };
        // One buffer for the whole record, zeroed, so the padding after the
        // content is zeros rather than whatever the allocator last held. The
        // padding is not hashed and nothing reads it; it is zeroed because a
        // device image with allocator leavings in it is a device image nobody
        // can diff against another.
        let mut record = vec![0u8; blocks as usize * block_bytes];
        record[..Header::BYTES].copy_from_slice(&header.to_bytes());
        record[Header::BYTES..record_bytes].copy_from_slice(content);
        for (index, one) in record.chunks(block_bytes).enumerate() {
            self.device.write(self.free + index as u64, one)?;
        }

        self.located.insert(hash, self.free);
        self.free += blocks;
        Ok(hash)
    }

    /// The header of the blob stored under `hash`, decoded and believed.
    ///
    /// A caller needs this before [`Store::get`] to know how large a buffer to
    /// offer, and `get_object` needs it to know what kind of thing it is
    /// holding. It costs one block read.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a hash this store holds nothing under;
    /// [`refusal::MALFORMED`] or [`refusal::UNKNOWN`] for bytes that are not a
    /// header this build can read; [`refusal::CONTENT`] when the header found
    /// there names a different blob, which is a device that returned the wrong
    /// block.
    pub fn head(&mut self, hash: &[u8; 32]) -> Result<Header, i32> {
        let block = self.located.get(hash).copied().ok_or(refusal::ADDRESS)?;
        let mut first = vec![0u8; self.superblock.block_bytes as usize];
        self.device.read(block, &mut first)?;
        Self::header_in(&first, hash)
    }

    /// Decode the header at the start of a block that has been read, and refuse
    /// a header that names a different blob.
    ///
    /// The second check is not redundant beside the content hash `get` takes:
    /// it catches a device that returned the wrong block *before* the length
    /// field in that block is used to decide how much more to read.
    fn header_in(block: &[u8], hash: &[u8; 32]) -> Result<Header, i32> {
        if block.len() < Header::BYTES {
            return Err(refusal::MALFORMED);
        }
        let mut raw = [0u8; Header::BYTES];
        raw.copy_from_slice(&block[..Header::BYTES]);
        let header = Header::from_bytes(&raw)?;
        if header.hash != *hash {
            return Err(refusal::CONTENT);
        }
        Ok(header)
    }

    /// Read the blob stored under `hash` into `into`, and answer how many bytes
    /// it holds.
    ///
    /// The content is hashed and compared against the name it was asked for
    /// before this returns, so a caller holding `Ok(n)` holds `n` bytes that
    /// hash to `hash`. That check is the whole reason a store is worth having
    /// over a filesystem, and it is why `blob/tests/million.rs` flips a byte:
    /// a verifier nothing can fail is indistinguishable from one that cannot.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`], [`refusal::MALFORMED`] and [`refusal::UNKNOWN`] as
    /// [`Store::head`] gives them; [`refusal::SHORT_BUFFER`] when `into` is
    /// smaller than the content; [`refusal::CONTENT`] when the bytes read back
    /// do not hash to `hash`.
    pub fn get(&mut self, hash: &[u8; 32], into: &mut [u8]) -> Result<usize, i32> {
        let block = self.located.get(hash).copied().ok_or(refusal::ADDRESS)?;
        let block_bytes = self.superblock.block_bytes as usize;

        // The first block carries the header and the beginning of the content,
        // and it is read once: `head` would read it a second time, which on a
        // million-blob run is a million reads bought for a tidier three lines.
        let mut one = vec![0u8; block_bytes];
        self.device.read(block, &mut one)?;
        let header = Self::header_in(&one, hash)?;
        let content_bytes =
            usize::try_from(header.content_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        if content_bytes > into.len() {
            return Err(refusal::SHORT_BUFFER);
        }
        let first = (block_bytes - Header::BYTES).min(content_bytes);
        into[..first].copy_from_slice(&one[Header::BYTES..Header::BYTES + first]);

        let mut done = first;
        let mut at = block + 1;
        while done < content_bytes {
            self.device.read(at, &mut one)?;
            let take = block_bytes.min(content_bytes - done);
            into[done..done + take].copy_from_slice(&one[..take]);
            done += take;
            at += 1;
        }

        // Verify before accept, at the granularity a blob has one. Everything
        // above this line is a length a device wrote; this is what says the
        // bytes are the bytes.
        if sha256(&into[..content_bytes]) != *hash {
            return Err(refusal::CONTENT);
        }
        Ok(content_bytes)
    }

    /// Chunk `bytes`, store every chunk, then store the object that names them
    /// in order — and answer the object's hash.
    ///
    /// # Why the object is a blob like any other
    ///
    /// Because then an object has one name, and that name is a content address
    /// like every other name in the system: a generation can carry it, the
    /// index can map a path to it, and a reader resolves it through the same
    /// path as a chunk. What makes it an *object* is its kind and the shape of
    /// its content — an [`ObjectHead`] and that many hashes in order — and
    /// nothing else.
    ///
    /// Each chunk is hashed over exactly the bytes the chunker scanned, in one
    /// pass; the chunker itself holds no chunk, so what is held here is the
    /// list of names, which is the thing that has to be variable-length.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] for an object of more than `CHUNKS_MAX` chunks;
    /// otherwise whatever [`Store::put`] returns.
    pub fn put_object(&mut self, bytes: &[u8]) -> Result<[u8; 32], i32> {
        let mut chunker = Chunker::new();
        let mut cuts: Vec<usize> = Vec::new();
        // The iterator borrows the chunker and finishes the slice when it is
        // dropped, so it is scoped rather than held: `finish` below needs the
        // chunker back, and the tail is only a chunk because the object ended.
        cuts.extend(chunker.feed(bytes));
        let tail = chunker.finish();

        let mut hashes: Vec<[u8; 32]> = Vec::with_capacity(cuts.len() + 1);
        let mut at = 0;
        for cut in cuts {
            hashes.push(self.put(kind::CHUNK, &bytes[at..cut])?);
            at = cut;
        }
        if tail > 0 {
            hashes.push(self.put(kind::CHUNK, &bytes[at..])?);
        }
        if hashes.len() > CHUNKS_MAX {
            return Err(refusal::UNKNOWN);
        }

        let head = ObjectHead { object_bytes: bytes.len() as u64, chunks: hashes.len() as u32 };
        let mut content = Vec::with_capacity(head.bytes());
        content.extend_from_slice(&head.to_bytes());
        for hash in &hashes {
            content.extend_from_slice(hash);
        }
        self.put(kind::OBJECT, &content)
    }

    /// Reassemble the object stored under `hash` into `into`, and answer its
    /// logical size.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] when `hash` names a blob that is not an object;
    /// [`refusal::MALFORMED`] when the chunks do not deliver the object's own
    /// stated length, which is a truncated or padded chunk list;
    /// [`refusal::SHORT_BUFFER`] when `into` cannot hold the object; and every
    /// refusal [`Store::get`] can make, per chunk — including
    /// [`refusal::CONTENT`], so an object is verified chunk by chunk rather
    /// than as a whole.
    pub fn get_object(&mut self, hash: &[u8; 32], into: &mut [u8]) -> Result<usize, i32> {
        let header = self.head(hash)?;
        if header.kind != kind::OBJECT {
            return Err(refusal::UNKNOWN);
        }
        let content_bytes =
            usize::try_from(header.content_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        let mut content = vec![0u8; content_bytes];
        self.get(hash, &mut content)?;

        if content.len() < ObjectHead::HEAD_BYTES {
            return Err(refusal::MALFORMED);
        }
        let mut raw = [0u8; ObjectHead::HEAD_BYTES];
        raw.copy_from_slice(&content[..ObjectHead::HEAD_BYTES]);
        let head = ObjectHead::from_bytes(&raw)?;
        if head.bytes() != content.len() {
            return Err(refusal::MALFORMED);
        }
        let object_bytes = usize::try_from(head.object_bytes).map_err(|_| refusal::SHORT_BUFFER)?;
        if object_bytes > into.len() {
            return Err(refusal::SHORT_BUFFER);
        }

        let mut done = 0;
        for index in 0..head.chunks as usize {
            let at = ObjectHead::HEAD_BYTES + index * 32;
            let mut chunk = [0u8; 32];
            chunk.copy_from_slice(&content[at..at + 32]);
            let moved = self.get(&chunk, &mut into[done..object_bytes])?;
            done += moved;
        }
        if done != object_bytes {
            return Err(refusal::MALFORMED);
        }
        Ok(object_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{Store, refuse_a_chunker_that_moved, superblock_for_this_build};
    use crate::chunk::CHUNK_MAX_BYTES;
    use crate::device::{Device, Memory};
    use alloc::vec;
    use alloc::vec::Vec;
    use f_abi::store::{Header, RootRecord, Superblock, kind, refusal};
    use f_hash::sha256;

    const BLOCK: u32 = 512;

    fn store_over(blocks: u64) -> Store<Memory> {
        let device = Memory::new(BLOCK as usize, blocks);
        let layout = superblock_for_this_build(BLOCK, 8, 1 << 20, 1, 2);
        Store::format(device, &layout).expect("a device this store can format")
    }

    /// Content that is not the same twice, without drawing anything: these are
    /// unit tests of the format, and the drawn content is
    /// `blob/tests/million.rs`'s job.
    fn content(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|index| seed.wrapping_add((index % 251) as u8)).collect()
    }

    #[test]
    fn the_superblock_lands_in_block_zero_and_decodes_to_what_was_written() {
        let mut store = store_over(64);
        let mut block = vec![0u8; BLOCK as usize];
        store.device_mut().read(0, &mut block).expect("block 0");
        let mut raw = [0u8; Superblock::BYTES];
        raw.copy_from_slice(&block[..Superblock::BYTES]);

        let found = Superblock::from_bytes(&raw).expect("a superblock this build wrote");
        assert_eq!(&found, store.superblock());
        assert_eq!(refuse_a_chunker_that_moved(&found), Ok(()));
        // The barrier, before anything names the superblock.
        assert_eq!(store.device().flushes(), 1);
    }

    /// The whole reason the chunker's constants are on the device.
    #[test]
    fn a_device_chunked_by_another_build_is_refused_rather_than_re_chunked() {
        let mut moved = superblock_for_this_build(BLOCK, 8, 1 << 20, 1, 2);
        moved.chunk_min_bytes /= 2;
        assert_eq!(refuse_a_chunker_that_moved(&moved), Err(refusal::UNKNOWN));

        let mut relabelled = superblock_for_this_build(BLOCK, 8, 1 << 20, 1, 2);
        relabelled.gear_label ^= 1;
        assert_eq!(refuse_a_chunker_that_moved(&relabelled), Err(refusal::UNKNOWN));

        // And a format with one of those is refused at the point it would be
        // written, rather than at the first object that hashed differently.
        let device = Memory::new(BLOCK as usize, 8);
        assert_eq!(Store::format(device, &moved).err(), Some(refusal::UNKNOWN));
    }

    #[test]
    fn a_blob_read_back_is_the_blob_that_was_written() {
        let mut store = store_over(64);
        let written = content(3, 1000);
        let hash = store.put(kind::CHUNK, &written).expect("room for a two-block record");
        assert_eq!(hash, sha256(&written));

        let head = store.head(&hash).expect("a header this build wrote");
        assert_eq!(head.kind, kind::CHUNK);
        assert_eq!(head.content_bytes, 1000);

        let mut read = vec![0u8; 1000];
        assert_eq!(store.get(&hash, &mut read), Ok(1000));
        assert_eq!(read, written);

        // The record occupies whole blocks: 52 + 1000 bytes is three of them.
        assert_eq!(store.free(), 4);
    }

    #[test]
    fn the_same_content_twice_is_one_record() {
        let mut store = store_over(64);
        let written = content(9, 100);
        let first = store.put(kind::CHUNK, &written).expect("the first put");
        let free = store.free();
        let second = store.put(kind::CHUNK, &written).expect("the second put");
        assert_eq!(first, second);
        assert_eq!(store.free(), free, "the second put wrote nothing");
        assert_eq!(store.records(), 1);
    }

    #[test]
    fn a_hash_this_store_never_wrote_is_refused_and_a_short_buffer_is_not_truncated() {
        let mut store = store_over(64);
        let written = content(1, 600);
        let hash = store.put(kind::CHUNK, &written).expect("a two-block record");

        let mut room = vec![0u8; 600];
        assert_eq!(store.get(&[0xFF; 32], &mut room), Err(refusal::ADDRESS));

        let mut too_small = vec![0u8; 599];
        assert_eq!(store.get(&hash, &mut too_small), Err(refusal::SHORT_BUFFER));
        assert!(too_small.iter().all(|byte| *byte == 0), "a refusal wrote nothing");
    }

    /// The control, at unit scale: the same thing `blob/tests/million.rs` does
    /// a million times, here so that the refusal is pinned to the byte that was
    /// flipped rather than to a count.
    #[test]
    fn a_flipped_byte_in_the_content_is_refused_on_the_hash() {
        let mut store = store_over(64);
        let written = content(7, 300);
        let hash = store.put(kind::CHUNK, &written).expect("a one-block record");
        let block = store.address(&hash).expect("the record's address");

        let at = block as usize * BLOCK as usize + Header::BYTES + 11;
        store.device_mut().flip(at).expect("a byte inside the content");

        let mut read = vec![0u8; 300];
        assert_eq!(store.get(&hash, &mut read), Err(refusal::CONTENT));

        // And flipping it back makes the same read succeed, which is what says
        // the refusal was about that byte and not about the run of them.
        store.device_mut().flip(at).expect("the same byte");
        assert_eq!(store.get(&hash, &mut read), Ok(300));
        assert_eq!(read, written);
    }

    /// A byte flipped in a *header* is refused before the content is even read,
    /// which is a different refusal and has to stay a different one.
    #[test]
    fn a_flipped_byte_in_the_header_is_refused_on_the_record() {
        let mut store = store_over(64);
        let hash = store.put(kind::CHUNK, &content(5, 64)).expect("a one-block record");
        let block = store.address(&hash).expect("the record's address");
        let at = block as usize * BLOCK as usize;

        // The magic.
        store.device_mut().flip(at).expect("the first byte of the magic");
        let mut read = vec![0u8; 64];
        assert_eq!(store.get(&hash, &mut read), Err(refusal::MALFORMED));
        store.device_mut().flip(at).expect("back again");

        // The kind, which becomes one this build does not know.
        store.device_mut().flip(at + 9).expect("the high byte of the kind");
        assert_eq!(store.get(&hash, &mut read), Err(refusal::UNKNOWN));
    }

    #[test]
    fn an_object_is_one_hash_and_reassembles_to_the_bytes_it_was_given() {
        // Two chunks at least, so the object is not a single chunk wearing an
        // object's kind: zero-filled content is cut only by the maximum, which
        // is the one length this crate can rely on without drawing anything.
        let bytes = vec![0u8; 2 * CHUNK_MAX_BYTES + 4096];
        let mut store = store_over(4096);
        let object = store.put_object(&bytes).expect("room for three chunks and a list");

        let head = store.head(&object).expect("the object's header");
        assert_eq!(head.kind, kind::OBJECT);

        let mut read = vec![0u8; bytes.len()];
        assert_eq!(store.get_object(&object, &mut read), Ok(bytes.len()));
        assert_eq!(read, bytes);

        // The three chunks are one record between them: identical content in
        // two places is stored once, which is the deduplication half of the
        // task and is visible here as a record count.
        assert_eq!(store.records(), 3, "two identical chunks, a remainder, and the object");

        // The same bytes again are the same hash, and cost no new records.
        let records = store.records();
        assert_eq!(store.put_object(&bytes), Ok(object));
        assert_eq!(store.records(), records);
    }

    #[test]
    fn a_blob_that_is_not_an_object_is_refused_as_one() {
        let mut store = store_over(64);
        let hash = store.put(kind::CHUNK, &content(2, 40)).expect("a chunk");
        let mut read = vec![0u8; 40];
        assert_eq!(store.get_object(&hash, &mut read), Err(refusal::UNKNOWN));
    }

    /// The root record's check is a real SHA-256 here, which is the part
    /// `abi/`'s own tests cannot demonstrate: that crate has no hash in it.
    #[test]
    fn a_root_record_verifies_against_the_hash_this_crate_computes() {
        let mut record = RootRecord {
            generation: 1,
            root: [0xA1; 32],
            frame: [0xB2; 32],
            module: [0xC3; 32],
            previous: [0; 32],
            check: [0; 32],
        };
        record.check = sha256(&record.checked());

        let raw = record.to_bytes();
        let computed = sha256(&raw[..RootRecord::CHECKED_BYTES]);
        assert_eq!(RootRecord::from_bytes(&raw, &computed), Ok(record));

        // One byte of the root, moved. The check no longer covers what the
        // record says, and no field is returned.
        let mut torn = raw;
        torn[20] ^= 1;
        let computed = sha256(&torn[..RootRecord::CHECKED_BYTES]);
        assert_eq!(RootRecord::from_bytes(&torn, &computed), Err(refusal::MALFORMED));
    }

    #[test]
    fn a_device_with_no_room_refuses_rather_than_writing_past_its_end() {
        // Two blocks: one for the superblock, one for a record.
        let mut store = store_over(2);
        store.put(kind::CHUNK, &content(4, 8)).expect("the one block that is left");
        assert_eq!(store.put(kind::CHUNK, &content(6, 8)), Err(refusal::FULL));
    }

    #[test]
    fn a_device_the_format_cannot_use_is_refused() {
        let small = Memory::new(256, 8);
        let layout = superblock_for_this_build(256, 8, 1 << 20, 1, 2);
        assert_eq!(Store::format(small, &layout).err(), Some(refusal::MALFORMED));

        // A superblock describing a different device from the one it is being
        // written to.
        let device = Memory::new(BLOCK as usize, 8);
        let wrong = superblock_for_this_build(1024, 8, 1 << 20, 1, 2);
        assert_eq!(Store::format(device, &wrong).err(), Some(refusal::MALFORMED));
    }
}
