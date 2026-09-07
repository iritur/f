// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Verify before accept, both halves: the highest generation whose `check`
//! verifies **and** whose generation tree resolves.
//!
//! # Why the second half is the one that matters
//!
//! RFC 0060's answer to a device that acknowledges a barrier it did not honour
//! is not to detect the lie — nothing above the device can — it is to refuse a
//! root whose blobs are not there. A record's `check` says *these bytes are a
//! whole record*; it says nothing about whether the tree the record names
//! exists. A root that verifies and does not resolve is exactly the third thing
//! `E2-P01`'s exit forbids: a machine naming a state it cannot produce. So a
//! mount that stopped at the `check` would answer a hash and hand the caller a
//! *not found* on the first read after it — and *not found* is
//! indistinguishable from *collected*, which is the failure the spec spends a
//! paragraph refusing.
//!
//! So this walks the tree, and a root that does not resolve is counted in
//! [`Mounted::roots_refused_unresolved`] and the mount falls to the next-highest
//! generation. That is the rollback RFC 0060 says a lying device costs.
//!
//! # What resolution costs, and what bounds it
//!
//! The spec's sentence is *resolution is bounded by the size of the generation
//! tree, which is tens of nodes, not by the blob count*. That is true of the
//! walk below — it visits the root, its children and their children, and stops —
//! and it is **not** true of the scan underneath it: [`f_blob::store::Store::mount`]
//! reads every block of the address space to rebuild a hash-to-block map,
//! because after a cut nothing else on the device says where a blob is. The
//! expensive half is therefore the map and not the walk, and it is expensive
//! until `E2-B03`'s index is on the device. Saying which half costs what is
//! worth more here than a number: the number moves the day the index lands.
//!
//! # The shape of a generation blob, and where it is owed
//!
//! A [`kind::GENERATION`] blob's content is an [`ObjectHead`] and that many
//! child hashes — the same encoding an object's content has, and deliberately
//! so. `f_abi::store::Generation` is the *folded* node and stores nothing of its
//! own: its hash is the frame hash and the topology hash, and there is no child
//! list in it to walk, which is correct for a fold and useless to a mount that
//! has to check that the children are on the device. Until `E2-B07` writes a
//! real folded tree into the store, the child list a mount needs is the one the
//! format already has, and reusing it means one codec rather than two.
//! *What would reverse this:* `E2-B07`, at which point this walk decodes
//! `f_generation`'s nodes and this paragraph becomes the record of what it used
//! to do.
//!
//! # Determinism
//!
//! A `BTreeSet` for what has been visited and an explicit stack for the walk,
//! rather than a `HashSet` and recursion: RFC 0004 for the first, and for the
//! second, a tree whose depth a device gets to choose is a stack a device gets
//! to choose the depth of. Nothing here reads a clock or draws a value.

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::{ExtentHead, ObjectHead, RootRecord, kind};
use f_blob::store::Store;

use crate::device::Zoned;
use crate::map::ZoneMap;
use crate::publish::record_at;

/// What one mount found.
///
/// The three counts are here rather than in a log because `cargo xtask cut`'s
/// artefact requires two of them to be non-zero somewhere in a sweep: a sweep in
/// which no root was ever refused for either reason is a sweep of the model
/// rather than of the format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mounted {
    /// The generation this mount accepted, or `None` for a device with no root
    /// that both verifies and resolves — which is what a device looks like
    /// before its first publish has landed.
    /// Unit: none — a record, not a quantity.
    pub root: Option<RootRecord>,
    /// Root records whose bytes were on the device and whose `check` did not
    /// verify: torn records, and nothing else can produce one.
    /// Unit: count of root records.
    pub roots_refused_check: u64,
    /// Root records that verified and whose generation tree did not resolve.
    /// Unit: count of root records.
    pub roots_refused_unresolved: u64,
    /// Blobs the scan found and rebuilt a map from.
    /// Unit: count of blobs.
    pub blobs: u64,
}

/// Mount a device: rebuild the map, then take the highest generation that
/// verifies and resolves.
///
/// The store comes back with the mount, because a caller that has just been told
/// which root it has is a caller about to read the tree under it — and handing
/// back only the record would mean scanning the device twice.
///
/// # Errors
///
/// Whatever [`f_blob::store::Store::mount`] refuses the device for — a block
/// zero that is not a superblock this build reads, a chunker that moved — and
/// whatever the device says about a report.
pub fn mount<Z: Zoned>(device: Z, data_from: u32) -> Result<(Mounted, Store<ZoneMap<Z>>), i32> {
    let map = ZoneMap::remount(device, data_from)?;
    let mut store = Store::mount(map)?;
    let mut found = Mounted { blobs: store.records() as u64, ..Mounted::default() };

    // Every candidate, highest generation first, so that the first one that
    // resolves is the answer and the ones below it are never read. The rollback
    // is therefore as short as the damage: a device with one torn record falls
    // back one generation and stops.
    // `candidates` answers in ascending generation, which is the order the
    // dedup needs; a mount wants the other one.
    for record in candidates(&mut store)?.into_iter().rev() {
        if resolves(&mut store, &record.root) {
            found.root = Some(record);
            break;
        }
        found.roots_refused_unresolved += 1;
    }

    found.roots_refused_check = torn_records(&mut store)?;
    Ok((found, store))
}

/// Every root record on the device whose `check` verifies.
///
/// [`crate::publish::latest_root`] answers the highest of these and this needs
/// all of them, because the highest may not resolve. Two functions over one scan
/// rather than one function with a flag: a caller that wants the highest
/// verifying record — the publish path, choosing a zone — is not doing the same
/// thing as a mount.
fn candidates<Z: Zoned>(store: &mut Store<ZoneMap<Z>>) -> Result<Vec<RootRecord>, i32> {
    let (a, b) = (store.superblock().root_zone_a, store.superblock().root_zone_b);
    let mut out = Vec::new();
    for zone in [a, b] {
        let report = store.device_mut().device_mut().report(zone)?;
        for at in report.start..report.write_pointer {
            // Through the publish path's own reader, so that what a mount
            // believes and what a publish believes cannot come apart: there is
            // one function in this crate that turns a block into a record.
            if let Some(record) = record_at(store, at)? {
                out.push(record);
            }
        }
    }
    // A record carried across a wrap exists in both zones. Two copies of one
    // generation is not two candidates, and leaving the duplicate in would make
    // `roots_refused_unresolved` count the same refusal twice.
    out.sort_unstable_by_key(|record| record.generation);
    out.dedup_by_key(|record| record.generation);
    Ok(out)
}

/// Blocks in the root zones that carry this format's root magic and do not
/// decode.
///
/// This is the torn-record count, and it is deliberately not *every* block that
/// failed to decode: a sealed zone's write pointer sits at its end whatever room
/// is left, so a scan reaches blocks nothing ever wrote, and counting those
/// would make the number a function of the zone size. A block whose first four
/// bytes are [`f_abi::store::ROOT_MAGIC`] was written by this format and is not
/// a whole record, which is the one thing a `check` refuses that nothing else
/// explains.
fn torn_records<Z: Zoned>(store: &mut Store<ZoneMap<Z>>) -> Result<u64, i32> {
    let (a, b) = (store.superblock().root_zone_a, store.superblock().root_zone_b);
    let block_bytes = store.superblock().block_bytes as usize;
    let mut torn = 0u64;
    let mut block = vec![0u8; block_bytes];
    for zone in [a, b] {
        let report = store.device_mut().device_mut().report(zone)?;
        for at in report.start..report.write_pointer {
            if store.device_mut().device_mut().read(at, &mut block).is_err() {
                continue;
            }
            if block.len() < RootRecord::BYTES {
                continue;
            }
            let magic = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
            if magic != f_abi::store::ROOT_MAGIC {
                continue;
            }
            if record_at(store, at)?.is_none() {
                torn += 1;
            }
        }
    }
    Ok(torn)
}

/// Does every node under this hash exist on the device and hash to its name?
///
/// The walk is the whole of *verify before accept*'s second half. It stops at
/// the first child that is missing or does not verify, because a tree with one
/// hole in it is refused whichever hole it is, and a walk that carried on would
/// pay the rest of the tree to learn nothing.
fn resolves<Z: Zoned>(store: &mut Store<ZoneMap<Z>>, root: &[u8; 32]) -> bool {
    let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
    let mut pending: Vec<[u8; 32]> = vec![*root];
    while let Some(hash) = pending.pop() {
        if !seen.insert(hash) {
            continue;
        }
        let Ok(header) = store.head(&hash) else { return false };
        let Ok(content_bytes) = usize::try_from(header.content_bytes) else { return false };
        let mut content = vec![0u8; content_bytes];
        // `get` hashes what it read and refuses when the digest is not the name
        // it was asked for, so this is where a chunk is verified and not merely
        // located. A resolution that only checked *presence* would accept a
        // device that answered the right length of the wrong bytes.
        if store.get(&hash, &mut content).is_err() {
            return false;
        }
        // A chunk is a leaf: its bytes are its whole meaning and it names
        // nothing. Everything else carries a head and a list of hashes.
        if header.kind == kind::CHUNK {
            continue;
        }
        // An extent's head has the same two fields as an object's and is
        // decoded by its own constructor anyway. Reusing `ObjectHead` for both
        // because the layouts happen to agree would be a coincidence this file
        // depended on, and RFC 0058's whole point is that the two kinds are
        // distinguishable.
        let children = if header.kind == kind::EXTENT {
            let Some(pieces) = extent_children(&content) else { return false };
            pieces
        } else {
            let Some(chunks) = object_children(&content) else { return false };
            chunks
        };
        for child in children {
            pending.push(child);
        }
    }
    true
}

/// The hashes an object's — or a generation blob's — content names, or `None`
/// for content that is not one.
fn object_children(content: &[u8]) -> Option<Vec<[u8; 32]>> {
    if content.len() < ObjectHead::HEAD_BYTES {
        return None;
    }
    let mut raw = [0u8; ObjectHead::HEAD_BYTES];
    raw.copy_from_slice(&content[..ObjectHead::HEAD_BYTES]);
    let head = ObjectHead::from_bytes(&raw).ok()?;
    if head.bytes() != content.len() {
        return None;
    }
    Some(hashes_after(content, ObjectHead::HEAD_BYTES, head.chunks as usize))
}

/// The hashes an extent's content names, or `None` for content that is not one.
fn extent_children(content: &[u8]) -> Option<Vec<[u8; 32]>> {
    if content.len() < ExtentHead::HEAD_BYTES {
        return None;
    }
    let mut raw = [0u8; ExtentHead::HEAD_BYTES];
    raw.copy_from_slice(&content[..ExtentHead::HEAD_BYTES]);
    let head = ExtentHead::from_bytes(&raw).ok()?;
    if head.bytes() != content.len() {
        return None;
    }
    Some(hashes_after(content, ExtentHead::HEAD_BYTES, head.pieces as usize))
}

/// `count` thirty-two-byte hashes starting at `from`.
///
/// The caller has already checked that the content is exactly the head plus
/// this many hashes, which is what makes the indexing below total.
fn hashes_after(content: &[u8], from: usize, count: usize) -> Vec<[u8; 32]> {
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let at = from + index * 32;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&content[at..at + 32]);
        out.push(hash);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::mount;
    use crate::device::{Zoned, ZonedMemory};
    use crate::map::ZoneMap;
    use crate::publish::Publisher;
    use crate::roots::Roots;
    use alloc::vec;
    use alloc::vec::Vec;
    use f_abi::store::{ObjectHead, RootRecord, kind};
    use f_blob::device::Device;
    use f_blob::store::{Store, superblock_for_this_build};
    use f_hash::sha256;

    const BLOCK: usize = 512;
    const ZONE: u64 = 32;
    const DATA_FROM: u32 = 3;

    fn store() -> Store<ZoneMap<ZonedMemory>> {
        let layout = superblock_for_this_build(BLOCK as u32, 12, ZONE * BLOCK as u64, 1, 2);
        let map = ZoneMap::new(ZonedMemory::new(BLOCK, ZONE, 12), DATA_FROM).expect("data zones");
        Store::format(map, &layout).expect("a device this build can format")
    }

    /// Publish a generation node over one leaf object, which is the smallest
    /// tree the walk has anything to do with.
    fn publish(
        store: &mut Store<ZoneMap<ZonedMemory>>,
        roots: &mut Roots,
        generation: u64,
        previous: [u8; 32],
    ) -> RootRecord {
        let mut publisher = Publisher::open(roots, store);
        let leaf = store.put_object(&vec![generation as u8; 300]).expect("room for an object");
        let head = ObjectHead { object_bytes: 300, chunks: 1 };
        let mut content = Vec::with_capacity(head.bytes());
        content.extend_from_slice(&head.to_bytes());
        content.extend_from_slice(&leaf);
        let root = store.put(kind::GENERATION, &content).expect("room for a generation node");
        publisher.appended(roots, store).expect("an open write set");
        let record = RootRecord {
            generation,
            root,
            frame: [0u8; 32],
            module: root,
            previous,
            check: [0u8; 32],
        };
        publisher.commit(store, roots, &record).expect("a publish that fits")
    }

    #[test]
    fn a_mount_finds_the_highest_generation_whose_tree_resolves() {
        let mut store = store();
        let mut roots = Roots::new();
        let first = publish(&mut store, &mut roots, 1, [0u8; 32]);
        let second = publish(&mut store, &mut roots, 2, first.root);

        let device = store.into_device().into_device();
        let (found, _) = mount(device, DATA_FROM).expect("a device this build can mount");
        assert_eq!(found.root.map(|record| record.generation), Some(2));
        assert_eq!(found.root.map(|record| record.root), Some(second.root));
        assert_eq!(found.roots_refused_check, 0);
        assert_eq!(found.roots_refused_unresolved, 0);
        assert!(found.blobs >= 4, "two leaves, two chunks and two generation nodes");
    }

    #[test]
    fn a_root_that_verifies_and_does_not_resolve_is_refused_and_counted() {
        let mut store = store();
        let mut roots = Roots::new();
        let first = publish(&mut store, &mut roots, 1, [0u8; 32]);

        // A second root record naming a tree that is not on the device: exactly
        // what a lying device leaves behind when the record landed and the blobs
        // did not. It is appended by hand rather than published, because the
        // publish path cannot produce it — which is the point.
        let mut record = RootRecord {
            generation: 2,
            root: sha256(b"a generation nobody wrote"),
            frame: [0u8; 32],
            module: [0u8; 32],
            previous: first.root,
            check: [0u8; 32],
        };
        record.check = sha256(&record.checked());
        let mut block = vec![0u8; BLOCK];
        block[..RootRecord::BYTES].copy_from_slice(&record.to_bytes());
        let zone = store.superblock().root_zone_a;
        store.device_mut().append_reserved(zone, &block).expect("room in the root zone");
        store.device_mut().flush().expect("the model's barrier cannot fail");

        let device = store.into_device().into_device();
        let (found, _) = mount(device, DATA_FROM).expect("a device this build can mount");
        assert_eq!(
            found.root.map(|found| found.generation),
            Some(1),
            "the mount rolled back one generation rather than answering a root it cannot read"
        );
        assert_eq!(found.roots_refused_unresolved, 1);
        // And the `check` half is not what refused it: the record is whole.
        assert_eq!(found.roots_refused_check, 0);
    }

    #[test]
    fn a_torn_record_is_refused_by_its_check_and_counted_separately() {
        let mut store = store();
        let mut roots = Roots::new();
        let first = publish(&mut store, &mut roots, 1, [0u8; 32]);
        let second = publish(&mut store, &mut roots, 2, first.root);
        assert_eq!(second.generation, 2);

        // Damage the newest record the way a cut inside a write does: the magic
        // is still there and the bytes after it are not the ones the `check`
        // was taken over.
        let zone = store.superblock().root_zone_a;
        let report = store.device_mut().device_mut().report(zone).expect("the root zone");
        let torn = report.write_pointer - 1;
        let at = usize::try_from(torn).expect("a block index this host can address") * BLOCK;
        store.device_mut().device_mut().flip(at + 8).expect("the generation field");

        let device = store.into_device().into_device();
        let (found, _) = mount(device, DATA_FROM).expect("a device this build can mount");
        assert_eq!(
            found.root.map(|found| found.generation),
            Some(1),
            "a torn record is a rollback and not a refusal to mount"
        );
        assert_eq!(found.roots_refused_check, 1);
        assert_eq!(found.roots_refused_unresolved, 0);
    }
}
