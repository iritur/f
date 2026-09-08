// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The index: a log on the device, a map in memory, and a query that touches
//! neither a ring nor a block.
//!
//! # The shape of the map, and why it is two levels
//!
//! `BTreeMap<u8, BTreeMap<Vec<u8>, [u8; 32]>>` — namespace, then name. One flat
//! map keyed by a namespace byte prepended to the name would be the same set of
//! pairs and would cost an allocation on **every query**, because the caller
//! holds a name and the map wants a key, and building the key is a `Vec`. Two
//! levels means [`Index::get`] borrows the caller's `&[u8]` straight through
//! `Borrow<[u8]>` and allocates nothing at all. That matters here more than it
//! usually would: the measurement `E2-B03` is judged on is *what a query costs*,
//! and a query that allocated would be answering the exit with a heap
//! allocation hidden inside the zero.
//!
//! It also makes a namespace a real thing rather than a convention. RFC 0059's
//! pinned set `P` is one sub-map, walked in name order, and
//! [`Index::pinned_roots`] hands it over without filtering anything.
//!
//! # The log is append-only in the strong sense
//!
//! A block is written **once**, when it is full or when [`Index::barrier`] seals
//! it, and never again. That is not a stylistic preference: the media this
//! format is for is sequential-write-required (RFC 0060), so a partly-filled
//! block that were later topped up would be a second write at one address —
//! which the device refuses, on a path nothing local would ever exercise. The
//! price is the padding in a sealed tail block, and it is paid in the one place
//! it can be seen.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use f_abi::store::refusal;
use f_blob::device::Device;

use crate::record::{ENTRY_MAX_BYTES, Entry, HEAD_BYTES, ns, op, padding_at};

/// Names to hashes, over one region of one device.
///
/// # Why a region rather than a device
///
/// Because the index does not own a device: it shares one with the store, and
/// `f-zone` at `E2-B02` is what decides which zones each of them gets. A type
/// that assumed the whole device would have to be rewritten on the day it is
/// given a zone, and every caller written against the assumption rewritten with
/// it. So the region is `(first, blocks)` and is the caller's to choose, and
/// this crate refuses one that is not on the device.
pub struct Index<D: Device> {
    device: D,
    /// The first block of the log region.
    /// Unit: block index, zero-based.
    first: u64,
    /// How many blocks the log region holds.
    /// Unit: count of blocks.
    blocks: u64,
    /// How many blocks of the region have been sealed. The next block written is
    /// `first + used`.
    /// Unit: count of blocks.
    used: u64,
    /// The block being filled, held in memory because it may not be written
    /// twice.
    pending: Vec<u8>,
    /// Namespace, then name, to content address.
    entries: BTreeMap<u8, BTreeMap<Vec<u8>, [u8; 32]>>,
}

impl<D: Device> Index<D> {
    /// Mount the log in `[first, first + blocks)` and replay it into memory.
    ///
    /// This is the whole of the index's device cost. It reads every sealed block
    /// once and then reads nothing again, which is the trade this design is: a
    /// linear scan paid at mount buys a query that reads no block at all. The
    /// scan stops at the first block whose magic is zero, so a fresh region — a
    /// zeroed device — mounts empty in one read rather than in `blocks` of them.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for a device whose block cannot hold the widest
    /// entry, which is what makes the no-spanning rule true rather than intended,
    /// and for a torn record inside a block that had already begun;
    /// [`refusal::ADDRESS`] for a region that is not on the device;
    /// [`refusal::UNKNOWN`] for an entry from a build that is not this one.
    pub fn mount(device: D, first: u64, blocks: u64) -> Result<Self, i32> {
        if device.block_bytes() < ENTRY_MAX_BYTES {
            return Err(refusal::MALFORMED);
        }
        if blocks == 0 || first.checked_add(blocks).is_none_or(|end| end > device.blocks()) {
            return Err(refusal::ADDRESS);
        }
        let mut me =
            Self { device, first, blocks, used: 0, pending: Vec::new(), entries: BTreeMap::new() };
        me.replay()?;
        Ok(me)
    }

    /// Walk the sealed blocks and rebuild the map.
    fn replay(&mut self) -> Result<(), i32> {
        let block_bytes = self.device.block_bytes();
        let mut block = vec![0u8; block_bytes];
        while self.used < self.blocks {
            let at = self.first + self.used;
            self.device.read(at, &mut block)?;
            if padding_at(&block, 0) {
                // A block nothing was ever written to. The log ends here, and
                // the region beyond it is untouched rather than unread.
                break;
            }
            let mut cursor = 0usize;
            while cursor + HEAD_BYTES <= block_bytes && !padding_at(&block, cursor) {
                let entry = Entry::from_slice(&block[cursor..])?;
                let width = entry.encoded_bytes()?;
                apply(&mut self.entries, &entry);
                cursor += width;
            }
            self.used += 1;
        }
        Ok(())
    }

    /// **The query.** What `name` means in `namespace`, or `None`.
    ///
    /// `&self`, no device, no ring, no allocation, no boundary. This is
    /// `E2-B03`'s exit in one line: the caller links this crate, calls this
    /// method in its own address space, and gets a hash back having read zero
    /// device blocks. `index/tests/query.rs` counts that zero against a tree walk
    /// over the same names and prints both.
    #[must_use]
    pub fn get(&self, namespace: u8, name: &[u8]) -> Option<[u8; 32]> {
        self.entries.get(&namespace)?.get(name).copied()
    }

    /// Record that `name` in `namespace` means `hash`.
    ///
    /// The entry is appended first and the map is updated only if the append
    /// returned `Ok`, so a full log leaves the two agreeing rather than leaving
    /// memory ahead of the device. It is answerable by [`Index::get`] at once and
    /// durable after [`Index::barrier`] — RFC 0060's sentence, not a weakening of
    /// it.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for a name this format cannot hold;
    /// [`refusal::UNKNOWN`] for a namespace this build does not know;
    /// [`refusal::FULL`] when the region has no block left; whatever the device
    /// says about a write.
    pub fn set(&mut self, namespace: u8, name: &[u8], hash: &[u8; 32]) -> Result<(), i32> {
        let entry = Entry { namespace, op: op::SET, hash: *hash, name };
        self.append(&entry)?;
        apply(&mut self.entries, &entry);
        Ok(())
    }

    /// Record that `name` in `namespace` means nothing.
    ///
    /// A tombstone is appended even for a name the index does not hold, and that
    /// is deliberate: whether this index holds it is a fact about *this* mount's
    /// replay, and a removal that decided not to write itself would be a removal
    /// whose effect depended on what had been mounted when it was asked for.
    ///
    /// # Errors
    ///
    /// As [`Index::set`].
    pub fn remove(&mut self, namespace: u8, name: &[u8]) -> Result<(), i32> {
        let entry = Entry { namespace, op: op::REMOVE, hash: [0u8; 32], name };
        self.append(&entry)?;
        apply(&mut self.entries, &entry);
        Ok(())
    }

    /// Pin a generation root under `name`. RFC 0059.
    ///
    /// A named wrapper and not a new mechanism, because a pin *is* a name for a
    /// hash and the index is already that. What the wrapper buys is that the
    /// collector, `E2-B08`'s `PIN` opcode and this crate all spell the namespace
    /// once, here, rather than each carrying a `4` of its own.
    ///
    /// **This does not check that `root` resolves.** RFC 0059 requires that
    /// refusal and it belongs to the caller that has a resolver: `E2-B08`
    /// resolves and then calls this, in that order. An index that resolved would
    /// be a second thing able to disagree with the resolver about what resolves.
    ///
    /// # Errors
    ///
    /// As [`Index::set`].
    pub fn pin(&mut self, name: &[u8], root: &[u8; 32]) -> Result<(), i32> {
        self.set(ns::PIN, name, root)
    }

    /// Release the pin held under `name`. RFC 0059's `UNPIN`.
    ///
    /// The full re-mark that RFC requires of a removal is the collector's and is
    /// scheduled by the caller: this crate makes the pin go away durably, which
    /// is the half a collector cannot do for itself.
    ///
    /// # Errors
    ///
    /// As [`Index::set`].
    pub fn unpin(&mut self, name: &[u8]) -> Result<(), i32> {
        self.remove(ns::PIN, name)
    }

    /// Every pinned root, in name order: the durable half of the collector's
    /// pinned set `P`.
    ///
    /// In name order rather than in the order they were pinned, and that is the
    /// determinism argument rather than a tidiness one: a collector walks this,
    /// and a walk whose order came from a hash seeded per process would make two
    /// runs of one seed collect in two orders. RFC 0004.
    ///
    /// The *other* half of `P` — the transient entry an open publish registers —
    /// is not here and cannot be: it dies with the publish, and RFC 0059 says a
    /// pin that dies with its holder is exactly what this namespace exists to
    /// not be.
    pub fn pinned_roots(&self) -> impl Iterator<Item = (&[u8], [u8; 32])> {
        self.entries
            .get(&ns::PIN)
            .into_iter()
            .flat_map(|pins| pins.iter().map(|(name, root)| (name.as_slice(), *root)))
    }

    /// How many names this index holds in `namespace`.
    ///
    /// Unit: count of names.
    #[must_use]
    pub fn names(&self, namespace: u8) -> usize {
        self.entries.get(&namespace).map_or(0, BTreeMap::len)
    }

    /// How many blocks of the region have been sealed.
    ///
    /// Unit: count of blocks. This is the mount cost of the next mount, which is
    /// the number that says whether the design's one linear scan is still
    /// amortised — see the crate documentation's reversal.
    #[must_use]
    pub const fn blocks_used(&self) -> u64 {
        self.used
    }

    /// The device, for a caller with business with it — a counter, a cut model,
    /// a control that corrupts a byte.
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    /// The device.
    #[must_use]
    pub const fn device(&self) -> &D {
        &self.device
    }

    /// Give the device back, discarding whatever has not been sealed.
    ///
    /// What a remount is written in terms of, and the reason it is `self` rather
    /// than a `reload` method: a mount that reused this object could quietly keep
    /// a map entry the log does not carry, and then a test of durability would be
    /// testing the object it was trying to interrogate.
    pub fn into_device(self) -> D {
        self.device
    }

    /// Seal the current block, if there is one, and issue the device's barrier.
    ///
    /// When this returns, every entry written before it is on stable media. The
    /// sealed tail block is padded with zeros to the block, which is what the
    /// next mount reads as the end of that block's records.
    ///
    /// # Errors
    ///
    /// [`refusal::FULL`] when the region has no block left to seal into;
    /// whatever the device says about a write or a barrier.
    pub fn barrier(&mut self) -> Result<(), i32> {
        self.seal()?;
        self.device.flush()
    }

    /// Put an entry into the block being filled, sealing the previous one first
    /// if it will not fit.
    fn append(&mut self, entry: &Entry<'_>) -> Result<(), i32> {
        let width = entry.encoded_bytes()?;
        if !ns::known(entry.namespace) {
            return Err(refusal::UNKNOWN);
        }
        if self.pending.len() + width > self.device.block_bytes() {
            self.seal()?;
        }
        // The block this entry will land in is `first + used`, and the region has
        // to still have it. Checked after the seal above, because that seal is
        // what may have consumed the last one.
        if self.used >= self.blocks {
            return Err(refusal::FULL);
        }
        let at = self.pending.len();
        self.pending.resize(at + width, 0);
        entry.to_slice(&mut self.pending[at..at + width])
    }

    /// Write the block being filled, padded with zeros, and start a new one.
    fn seal(&mut self) -> Result<(), i32> {
        if self.pending.is_empty() {
            return Ok(());
        }
        if self.used >= self.blocks {
            return Err(refusal::FULL);
        }
        let mut block = vec![0u8; self.device.block_bytes()];
        block[..self.pending.len()].copy_from_slice(&self.pending);
        let at = self.first + self.used;
        self.device.write(at, &block)?;
        self.used += 1;
        self.pending.clear();
        Ok(())
    }
}

/// Apply one replayed or newly written entry to the map.
///
/// A free function and not a method, so that [`Index::replay`] can call it while
/// holding a borrow of the block it decoded from — and, more usefully, so that
/// there is exactly one place where an operation becomes a change to the map.
/// Two places is how a log and its replay come to disagree about what the log
/// said.
fn apply(entries: &mut BTreeMap<u8, BTreeMap<Vec<u8>, [u8; 32]>>, entry: &Entry<'_>) {
    match entry.op {
        op::REMOVE => {
            if let Some(names) = entries.get_mut(&entry.namespace) {
                names.remove(entry.name);
            }
        }
        // `SET` and nothing else: `Entry::from_slice` and `Index::append` have
        // both already refused an operation this build does not know, so an
        // unknown value cannot reach here — and if one ever did, doing nothing
        // silently is the behaviour R04 exists to forbid, which is why this arm
        // is written as the default rather than as a wildcard that swallows.
        _ => {
            entries.entry(entry.namespace).or_default().insert(entry.name.to_vec(), entry.hash);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Index;
    use crate::record::{ENTRY_MAX_BYTES, NAME_MAX, entry_bytes, ns};
    use alloc::vec;
    use f_abi::store::refusal;
    use f_blob::device::Memory;

    const BLOCK: usize = 512;

    fn empty(blocks: u64) -> Index<Memory> {
        Index::mount(Memory::new(BLOCK, blocks), 0, blocks).expect("a region on this device")
    }

    #[test]
    fn a_name_answers_with_what_it_was_set_to_and_nothing_else() {
        let mut index = empty(16);
        index.set(ns::PATH, b"a/b", &[1u8; 32]).expect("a name this format holds");
        index.set(ns::ATTR, b"a/b", &[2u8; 32]).expect("the same name, another namespace");

        assert_eq!(index.get(ns::PATH, b"a/b"), Some([1u8; 32]));
        // The same name in another namespace is another name. This is what makes
        // the pin namespace a reservation rather than a prefix convention.
        assert_eq!(index.get(ns::ATTR, b"a/b"), Some([2u8; 32]));
        assert_eq!(index.get(ns::META, b"a/b"), None);
        assert_eq!(index.get(ns::PATH, b"a/c"), None);
        assert_eq!(index.names(ns::PATH), 1);
    }

    #[test]
    fn the_last_write_wins_and_a_removal_is_a_removal() {
        let mut index = empty(16);
        index.set(ns::PATH, b"a", &[1u8; 32]).expect("a set");
        index.set(ns::PATH, b"a", &[2u8; 32]).expect("a second set");
        assert_eq!(index.get(ns::PATH, b"a"), Some([2u8; 32]));
        index.remove(ns::PATH, b"a").expect("a removal");
        assert_eq!(index.get(ns::PATH, b"a"), None);
        // A removal of a name nothing holds is written down anyway, so that its
        // effect does not depend on what this mount happened to have replayed.
        index.remove(ns::PATH, b"never").expect("a removal of nothing");
    }

    /// The whole point of a log: what was barriered comes back, and it comes
    /// back through a mount that shares nothing with the object that wrote it.
    #[test]
    fn what_was_barriered_survives_a_remount_and_what_was_not_does_not() {
        let mut index = empty(64);
        // Enough entries to fill more than one block, so the remount exercises
        // the block-to-block step and not only the first block.
        for n in 0u16..64 {
            index.set(ns::PATH, &n.to_le_bytes(), &[n as u8; 32]).expect("a set");
        }
        index.barrier().expect("the model's barrier cannot fail");
        // Written after the barrier and never sealed: this one is in memory only.
        index.set(ns::PATH, b"tail", &[9u8; 32]).expect("a set");
        assert_eq!(index.get(ns::PATH, b"tail"), Some([9u8; 32]));

        let device = index.into_device();
        let again = Index::mount(device, 0, 64).expect("a remount");
        for n in 0u16..64 {
            assert_eq!(again.get(ns::PATH, &n.to_le_bytes()), Some([n as u8; 32]));
        }
        assert_eq!(again.names(ns::PATH), 64);
        assert_eq!(
            again.get(ns::PATH, b"tail"),
            None,
            "an entry that was never barriered is not durable, and saying so is RFC 0060"
        );
    }

    /// A removal survives a remount too, which is the half a log gets wrong if
    /// the tombstone is only applied in memory.
    #[test]
    fn a_removal_survives_a_remount() {
        let mut index = empty(16);
        index.set(ns::PIN, b"keep", &[1u8; 32]).expect("a pin");
        index.set(ns::PIN, b"drop", &[2u8; 32]).expect("a pin");
        index.unpin(b"drop").expect("an unpin");
        index.barrier().expect("a barrier");

        let again = Index::mount(index.into_device(), 0, 16).expect("a remount");
        assert_eq!(again.get(ns::PIN, b"keep"), Some([1u8; 32]));
        assert_eq!(again.get(ns::PIN, b"drop"), None);
    }

    /// RFC 0059's namespace exists, is durable, and is walked in one order.
    #[test]
    fn the_pinned_set_is_the_pin_namespace_and_is_walked_in_name_order() {
        let mut index = empty(16);
        index.pin(b"c", &[3u8; 32]).expect("a pin");
        index.pin(b"a", &[1u8; 32]).expect("a pin");
        index.pin(b"b", &[2u8; 32]).expect("a pin");
        index.set(ns::PATH, b"zzz", &[9u8; 32]).expect("a path, which is not a pin");

        let walked: vec::Vec<(&[u8], [u8; 32])> = index.pinned_roots().collect();
        assert_eq!(
            walked,
            vec![
                (b"a".as_slice(), [1u8; 32]),
                (b"b".as_slice(), [2u8; 32]),
                (b"c".as_slice(), [3u8; 32])
            ]
        );
        assert_eq!(index.get(ns::PIN, b"a"), Some([1u8; 32]));
        // A pin is not a path and a path is not a pin.
        assert_eq!(index.get(ns::PIN, b"zzz"), None);
    }

    #[test]
    fn an_empty_region_mounts_empty_and_costs_one_read() {
        let index = empty(1024);
        assert_eq!(index.names(ns::PATH), 0);
        assert_eq!(index.blocks_used(), 0);
        assert_eq!(index.pinned_roots().count(), 0);
    }

    #[test]
    fn a_region_that_is_not_on_the_device_is_refused() {
        let device = Memory::new(BLOCK, 4);
        assert_eq!(Index::mount(device, 2, 4).map(|_| ()), Err(refusal::ADDRESS));
        let device = Memory::new(BLOCK, 4);
        assert_eq!(Index::mount(device, 0, 0).map(|_| ()), Err(refusal::ADDRESS));
        // A block that cannot hold the widest entry, which is what makes "no
        // entry spans two blocks" a check rather than an intention.
        let narrow = Memory::new(ENTRY_MAX_BYTES - 1, 4);
        assert_eq!(Index::mount(narrow, 0, 4).map(|_| ()), Err(refusal::MALFORMED));
    }

    #[test]
    fn a_region_with_no_block_left_refuses_rather_than_overwriting() {
        let mut index = empty(1);
        // A block holds six of these; the seventh needs a second block, and
        // there is not one.
        let name = vec![b'x'; NAME_MAX];
        let mut written = 0u32;
        loop {
            match index.set(ns::PATH, &name[..40], &[written as u8; 32]) {
                Ok(()) => written += 1,
                Err(e) => {
                    assert_eq!(e, refusal::FULL);
                    break;
                }
            }
            assert!(written < 1000, "a one-block region accepted more than it can hold");
        }
        assert!(written > 0, "a one-block region accepted nothing at all");
    }

    #[test]
    fn a_namespace_this_build_does_not_know_is_refused() {
        let mut index = empty(4);
        assert_eq!(index.set(200, b"a", &[0u8; 32]), Err(refusal::UNKNOWN));
        let long = vec![b'x'; NAME_MAX + 1];
        assert_eq!(index.set(ns::PATH, &long, &[0u8; 32]), Err(refusal::MALFORMED));
        assert_eq!(index.set(ns::PATH, b"", &[0u8; 32]), Err(refusal::MALFORMED));
    }

    /// A torn entry inside a block that had begun is a refusal, not a shorter
    /// index. This is the case `record::padding_at` is separate from the decoder
    /// for, and without it a single flipped bit truncates an index silently.
    #[test]
    fn a_torn_entry_is_refused_by_the_mount_rather_than_read_as_the_end() {
        let mut index = empty(8);
        index.set(ns::PATH, b"a", &[1u8; 32]).expect("a set");
        index.set(ns::PATH, b"b", &[2u8; 32]).expect("a set");
        index.barrier().expect("a barrier");

        let mut device = index.into_device();
        // The magic of the *second* entry: the first one decodes, so the block
        // has begun, and this is the case a decode-as-end would swallow.
        let second = entry_bytes(1).expect("one byte is a name");
        device.flip(second).expect("an offset inside the device");
        assert_eq!(Index::mount(device, 0, 8).map(|_| ()), Err(refusal::MALFORMED));
    }
}
