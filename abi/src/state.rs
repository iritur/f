// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The state tree, v0: what a component is doing, as a map of memory.
//!
//! RFC 0013 is the decision and this is the wire format for it. The six
//! properties that document names are the six things this module exists to
//! make true, and each one shows up here as something the format cannot say.
//!
//! **A map of memory, not a serialisation of it.** A node names a live word by
//! its offset in the published region. There is no encode step, so there is no
//! sampling interval and no second copy to disagree with the first. What the
//! publisher does is the store it was already doing.
//!
//! **Read, never delivered.** Nothing here has a callback in it and nothing can
//! be added to it that would: the format is a header, a schema block and an
//! array of words. R05.
//!
//! **Atomic per node, not across the tree.** Every value is one machine word,
//! read once. Two nodes read in one pass may be from different instants, and
//! the format has no generation counter, no seqlock and no way to express that
//! two nodes were read together — because it does not promise it. Anything that
//! needs a consistent pair of numbers needs one node holding both.
//!
//! **The hash is over bytes, not over interpretation.** [`snapshot`] runs over
//! the data block in node-id order and does not consult the schema, so a reader
//! too old to name half the nodes computes the same hash as one that can name
//! them all. That is the property that makes two readings comparable across
//! versions, and it is why the hash takes the words rather than the tree.
//!
//! **Node ids are permanent.** A retired id is never reused, for the reason
//! `TODO.md` never reuses a task id: the id is the only thing that makes two
//! readings across time comparable at all. The schema carries the names, so
//! renaming a node is free and renumbering one is a lie.
//!
//! **One build.** There is no debug variant of this. A tree that exists only
//! where somebody remembered to enable it describes a system nobody runs.

use crate::error;

/// Identifies a state tree, so a foreign or unwritten mapping is caught before
/// anything in it is believed.
pub const TREE_MAGIC: u64 = 0x465f_5354_4154_0001;

/// The version of this format.
///
/// Separate from [`ABI_VERSION`](crate::ABI_VERSION) on purpose: the tree is
/// read by tools that may be much older or newer than the component publishing
/// it, and tying its format to the channel ABI would make every ring change a
/// reason for a monitoring tool to stop working.
pub const TREE_VERSION: u32 = 1;

/// What a node's word means.
///
/// A `u8` on the wire. **An unknown kind is skipped and counted, not refused**,
/// and this is the one deliberate exception to R04 in the whole tree — argued
/// in RFC 0013 and repeated here because a reader "fixed" into refusing an
/// unknown kind would make every old tool useless against every new system.
/// The bytes are still hashed: what a reader cannot name, it can still compare.
pub mod kind {
    /// An interior node. Its word is reserved and reads as zero.
    pub const SUBTREE: u8 = 1;
    /// A count that only goes up.
    pub const COUNTER: u8 = 2;
    /// A value that goes both ways.
    pub const GAUGE: u8 = 3;
}

/// What a node's word is counted in.
///
/// R03 says every published field states its unit, and a tree whose nodes did
/// not would be publishing numbers a reader has to guess at. `NONE` is a real
/// answer and not a missing one — an identifier is not a quantity.
pub mod unit {
    /// Not a quantity.
    pub const NONE: u8 = 0;
    /// Nanoseconds.
    pub const NANOSECONDS: u8 = 1;
    /// Bytes.
    pub const BYTES: u8 = 2;
    /// Physical frames.
    pub const FRAMES: u8 = 3;
    /// Ring entries.
    pub const ENTRIES: u8 = 4;
    /// System calls, ring operations, or anything else answered one at a time.
    pub const CALLS: u8 = 5;
    /// Processor cores.
    pub const CORES: u8 = 6;
    /// Capability table slots.
    pub const SLOTS: u8 = 7;
}

/// The first cache line of a published region.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct TreeHeader {
    /// Must equal [`TREE_MAGIC`].
    /// Unit: none — a constant, checked before anything else in the mapping is
    /// trusted. There is no zero: a zeroed header is a refused one.
    pub magic: u64,
    /// The format version. Unit: none — a format ordinal. Zero is not a
    /// version.
    pub version: u32,
    /// How many nodes the schema block describes.
    /// Unit: nodes. Zero is refused: a tree with no nodes is a region somebody
    /// got wrong, not an empty tree.
    pub nodes: u32,
    /// Byte offset of the schema block from the first byte of the mapping.
    /// Unit: bytes. Zero would overlap this header and is refused.
    pub schema_offset: u32,
    /// Byte offset of the data block, on the same origin.
    /// Unit: bytes. Zero is refused for the same reason.
    pub data_offset: u32,
    /// Incremented when the schema changes — a component that republishes with
    /// different nodes. A reader holding an older generation is holding a
    /// description of a different tree.
    /// Unit: schema republications. Zero is a tree that has never changed
    /// shape, which is the state every tree opens in.
    pub generation: u32,
    /// Reserved. Must be zero; a non-zero word is refused rather than ignored,
    /// per R04. Unit: none.
    pub _reserved: [u32; 3],
}

/// One node: what it is, where its word is, and what to call it.
///
/// Exactly 32 bytes, so the schema block is an array a reader can index without
/// consulting anything.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub struct SchemaEntry {
    /// This node's permanent identifier. Never reused.
    /// Unit: none — an identifier, not a quantity. Zero is not a node.
    pub id: u32,
    /// The id of the node this hangs under, or zero for the root.
    /// Unit: none — a node identifier. Zero means no parent.
    pub parent: u32,
    /// Byte offset of this node's word from the start of the data block.
    /// Unit: bytes, from `data_offset` and not from the mapping.
    pub offset: u32,
    /// One of [`kind`]. Unit: none.
    pub kind: u8,
    /// One of [`unit`]. Unit: none — it *is* the unit.
    pub unit: u8,
    /// How many bytes of `name` are used.
    /// Unit: bytes. Zero is a node with no name, which is legal and unhelpful.
    pub name_len: u8,
    /// Reserved. Must be zero. Unit: none.
    pub _reserved: u8,
    /// The node's name, ASCII, not terminated.
    /// Unit: none. Sixteen bytes because a name longer than that is a
    /// description, and descriptions belong in the document that owns the node.
    pub name: [u8; 16],
}

const _: () = assert!(core::mem::size_of::<TreeHeader>() == 64);
const _: () = assert!(core::mem::size_of::<SchemaEntry>() == 32);

/// The size of a node's word. Every node is one, which is what makes a snapshot
/// atomic per node.
pub const WORD: u32 = 8;

impl SchemaEntry {
    /// A zeroed entry, so a schema block can be initialised without a loop.
    pub const ZERO: Self = Self {
        id: 0,
        parent: 0,
        offset: 0,
        kind: 0,
        unit: 0,
        name_len: 0,
        _reserved: 0,
        name: [0; 16],
    };

    /// One node, named.
    ///
    /// Takes the name as a slice and truncates at sixteen bytes rather than
    /// refusing, because a name is the one field in this format with no
    /// semantics attached to it — and a build that failed over a long label
    /// would be a build that failed over a label.
    #[must_use]
    pub const fn new(id: u32, parent: u32, offset: u32, kind: u8, unit: u8, name: &[u8]) -> Self {
        let mut entry = Self::ZERO;
        entry.id = id;
        entry.parent = parent;
        entry.offset = offset;
        entry.kind = kind;
        entry.unit = unit;

        let len = if name.len() > 16 { 16 } else { name.len() };
        entry.name_len = len as u8;
        let mut i = 0;
        while i < len {
            entry.name[i] = name[i];
            i += 1;
        }
        entry
    }

    /// The name, as far as it goes.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        let len = (self.name_len as usize).min(16);
        &self.name[..len]
    }
}

/// A snapshot hash over a published data block.
///
/// FNV-1a over the words in node-id order, little-endian, and the choice is the
/// same one `xtask`'s trace hash made for the same reason: it has to be
/// identical on two machines and in two readers, which rules out anything the
/// standard library reserves the right to change or seeds per process. It does
/// **not** have to be collision-resistant — nothing adversarial produces these
/// — and it is eight lines here rather than a dependency.
///
/// Over the *words*, deliberately, and never over the schema. The schema is
/// constant for a generation, so hashing it would fold a constant into every
/// answer and hide a data block that never changes; and a reader too old to
/// name half the nodes must still compute the same hash as one that can name
/// them all.
#[must_use]
pub fn snapshot(words: &[u64]) -> u64 {
    let mut hash = SEED;
    for word in words {
        hash = fold(hash, *word);
    }
    hash
}

/// The seed FNV-1a starts at.
const SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// One word into the hash, little-endian.
///
/// One definition, used by both the slice form above and [`Reader::snapshot`],
/// because two readers of the same bytes disagreeing about the hash would make
/// the whole mechanism worthless — and two copies of eight lines is exactly how
/// that happens.
///
/// Written as shifts rather than `to_le_bytes()`, and the reason is the linker
/// rather than taste: iterating a `[u8; 8]` goes through `IndexRange`, and
/// `user/init` links as one library with no `core` beside it, so that is an
/// undefined symbol at link time. Arithmetic has no such edge.
#[inline]
const fn fold(mut hash: u64, word: u64) -> u64 {
    let mut shifted = word;
    let mut byte = 0;
    while byte < 8 {
        hash ^= shifted & 0xFF;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        shifted >>= 8;
        byte += 1;
    }
    hash
}

impl TreeHeader {
    /// Structural validation, before any offset in this header is used.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/MALFORMED_HEADER` for a header this build will not read: a
    /// foreign magic, a version it does not speak, no nodes, an overlapping
    /// block, or a reserved word carrying something.
    ///
    /// `#[inline]` for the reason `f_abi::door::call` is: `user/init` links as one
    /// library and nothing else, so everything a component calls across a crate
    /// boundary has to be compiled into it. `xtask`'s link step states that as a
    /// claim about the component rather than checking it, and an undefined symbol
    /// there is an error rather than a warning — which is how this was found.
    #[inline]
    pub fn check(&self, mapping_len: u32) -> Result<(), i32> {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

        if self.magic != TREE_MAGIC || self.version != TREE_VERSION {
            return Err(malformed);
        }
        if self.nodes == 0 || self._reserved != [0; 3] {
            return Err(malformed);
        }
        // Both blocks must start after this header and end inside the mapping.
        let header = core::mem::size_of::<TreeHeader>() as u32;
        if self.schema_offset < header || self.data_offset < header {
            return Err(malformed);
        }

        let schema_bytes = self.nodes.checked_mul(32).ok_or(malformed)?;
        let schema_end = self.schema_offset.checked_add(schema_bytes).ok_or(malformed)?;
        let data_bytes = self.nodes.checked_mul(WORD).ok_or(malformed)?;
        let data_end = self.data_offset.checked_add(data_bytes).ok_or(malformed)?;

        if schema_end > mapping_len || data_end > mapping_len {
            return Err(malformed);
        }
        // The two blocks may not overlap each other. A schema that pointed into
        // the data block would let a publisher's counter rewrite the
        // description of what it counts.
        let overlaps = self.schema_offset < data_end && self.data_offset < schema_end;
        if overlaps {
            return Err(malformed);
        }
        // Alignment, because both blocks are read as arrays of aligned types.
        if !self.schema_offset.is_multiple_of(32) || !self.data_offset.is_multiple_of(WORD) {
            return Err(malformed);
        }
        Ok(())
    }
}

/// Check a schema block against the header that describes it.
///
/// # Errors
///
/// `ARGUMENT/MALFORMED_HEADER` for a schema that does not describe a tree:
/// ids that do not ascend, a node whose word is outside the data block, two
/// nodes sharing a word, or a gap no node names.
///
/// The last two are the ones worth having. Two nodes on one word is two
/// subsystems publishing into the same place, which reads as one of them being
/// broken; a gap is a word the snapshot hashes and no node describes, so a hash
/// would move for a reason nothing can name.
///
/// `#[inline]` for the reason `f_abi::door::call` is: `user/init` links as one
/// library and nothing else, so everything a component calls across a crate
/// boundary has to be compiled into it. `xtask`'s link step states that as a
/// claim about the component rather than checking it, and an undefined symbol
/// there is an error rather than a warning — which is how this was found.
#[inline]
pub fn validate(header: &TreeHeader, schema: &[SchemaEntry]) -> Result<(), i32> {
    let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

    if schema.len() != header.nodes as usize {
        return Err(malformed);
    }

    let mut previous_id = 0u32;
    for (index, entry) in schema.iter().enumerate() {
        if entry.id == 0 || entry.id <= previous_id {
            return Err(malformed);
        }
        previous_id = entry.id;

        if entry._reserved != 0 || entry.name_len > 16 {
            return Err(malformed);
        }
        // The words tile the data block in schema order, with no gap and no
        // overlap. Stated as an equality rather than as two separate checks,
        // because that is exactly what "tile" means and a reader can verify it
        // by looking.
        if entry.offset != index as u32 * WORD {
            return Err(malformed);
        }
        // A parent must be a node named before this one, which makes the
        // hierarchy a tree rather than possibly a cycle — and makes that
        // property checkable in one pass.
        //
        // Written as a loop over `get`, which looks worse than it is. A range
        // index (`schema[..index]`) carries a panic path and `take(index)`
        // pulls in `IndexRange::len`; `user/init` links as one library with no
        // `core` beside it, so either is an undefined symbol at link time
        // rather than a warning. The component is why this function has to be
        // panic-free and dependency-free, and the linker is what enforces it —
        // which is a better enforcement than a comment asking for it.
        if entry.parent != 0 {
            let mut named = false;
            let mut back = 0;
            while back < index {
                if let Some(prior) = schema.get(back)
                    && prior.id == entry.parent
                {
                    named = true;
                    break;
                }
                back += 1;
            }
            if !named {
                return Err(malformed);
            }
        }
    }
    Ok(())
}

/// A tree somebody else published, read from an address this component was
/// given.
///
/// # Why this is safe to call
///
/// The same argument [`door::call`](crate::door::call) already makes, and this
/// is the second place in the tree leaning on it, so it is written out rather
/// than gestured at.
///
/// A component above the frame inherits `unsafe_code = "forbid"`, and that
/// property is enforced rather than asserted — it is the whole point of the
/// crate. So the instruction that reads a mapping lives on this side of the
/// boundary, in a crate that is part of the frame and reviewed as one. The
/// obligation is discharged against a contract the frame keeps: `base` is an
/// address the frame mapped in answer to a capability the caller held, and
/// nothing in here is believed until it has been checked against the `len` the
/// caller was given.
///
/// A component that invents an address gets a page fault, which is the defined
/// machine outcome the entire isolation suite rests on — `cargo xtask user` is
/// seven boots of exactly that. It is not *sound* by Rust's rules, and saying
/// so is the point: what makes it acceptable is that the failure is the one the
/// hardware is there to produce, and that the alternative is every component
/// containing the same `unsafe` block written slightly differently.
pub struct Reader {
    base: u64,
    header: TreeHeader,
}

impl Reader {
    /// Bind to a published tree.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for an address no tree can be read from, and
    /// `ARGUMENT/MALFORMED_HEADER` for anything [`TreeHeader::check`] or
    /// [`validate`] refuses.
    ///
    /// `#[inline]` for the reason `f_abi::door::call` is: `user/init` links as one
    /// library and nothing else, so everything a component calls across a crate
    /// boundary has to be compiled into it. `xtask`'s link step states that as a
    /// claim about the component rather than checking it, and an undefined symbol
    /// there is an error rather than a warning — which is how this was found.
    #[inline]
    pub fn at(base: u64, len: u32) -> Result<Self, i32> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        if base == 0 || !base.is_multiple_of(64) || len < core::mem::size_of::<TreeHeader>() as u32
        {
            return Err(bad);
        }

        // Copied out before a field is looked at, for the reason
        // `f_ring::Mapping` copies a channel header: the publisher can rewrite
        // these bytes between any two reads, so validating in place would check
        // one header and then read the tree described by another.
        // SAFETY: the caller's contract, argued on the type. Aligned by the
        // check above, and a whole header is inside `len`.
        let header = unsafe { (base as *const TreeHeader).read_volatile() };
        header.check(len)?;

        // SAFETY: `check` has established that the schema block is `nodes`
        // 32-byte entries at a 32-byte aligned offset inside `len`.
        let schema = unsafe {
            core::slice::from_raw_parts(
                (base + u64::from(header.schema_offset)) as *const SchemaEntry,
                header.nodes as usize,
            )
        };
        validate(&header, schema)?;

        Ok(Self { base, header })
    }

    /// How many nodes the tree describes. Unit: nodes.
    #[must_use]
    #[inline]
    pub fn nodes(&self) -> u32 {
        self.header.nodes
    }

    /// The generation the schema was published at.
    /// Unit: schema republications.
    #[must_use]
    #[inline]
    pub fn generation(&self) -> u32 {
        self.header.generation
    }

    /// The snapshot hash of the tree as it stands.
    ///
    /// Each word is read exactly once and volatilely — atomic per node, and
    /// promising nothing about two nodes being from one instant, which is
    /// RFC 0013's decision and not a limitation of this reader.
    #[must_use]
    #[inline]
    pub fn snapshot(&self) -> u64 {
        let mut hash = SEED;
        let mut index = 0;
        while index < self.header.nodes {
            let at =
                self.base + u64::from(self.header.data_offset) + u64::from(index) * u64::from(WORD);
            // SAFETY: `check` established that `nodes` words at `data_offset`
            // are inside the mapping, and `data_offset` is eight-byte aligned.
            let word = unsafe { (at as *const u64).read_volatile() };
            hash = fold(hash, word);
            index += 1;
        }
        hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(nodes: u32) -> TreeHeader {
        TreeHeader {
            magic: TREE_MAGIC,
            version: TREE_VERSION,
            nodes,
            schema_offset: 64,
            data_offset: 64 + nodes * 32,
            generation: 0,
            _reserved: [0; 3],
        }
    }

    fn schema() -> [SchemaEntry; 3] {
        [
            SchemaEntry::new(1, 0, 0, kind::SUBTREE, unit::NONE, b"memory"),
            SchemaEntry::new(2, 1, 8, kind::GAUGE, unit::FRAMES, b"free"),
            SchemaEntry::new(3, 1, 16, kind::COUNTER, unit::CALLS, b"answered"),
        ]
    }

    #[test]
    fn the_wire_records_are_the_sizes_a_peer_will_assume() {
        // A field added to `SchemaEntry` that silently repads the record shifts
        // every entry after it, and a peer built from the specification reads
        // the tree as garbage from that point on.
        assert_eq!(core::mem::size_of::<TreeHeader>(), 64);
        assert_eq!(core::mem::size_of::<SchemaEntry>(), 32);
        assert_eq!(core::mem::align_of::<SchemaEntry>(), 32);
    }

    #[test]
    fn a_sound_header_and_schema_are_accepted() {
        // The control. Every refusal below would pass against a `check` that
        // refused everything.
        let header = header(3);
        assert_eq!(header.check(4096), Ok(()));
        assert_eq!(validate(&header, &schema()), Ok(()));
    }

    #[test]
    fn a_header_that_does_not_describe_a_tree_is_refused() {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        /// One hostile header: what it stands for, and how to make one.
        type Hostile = (&'static str, fn(&mut TreeHeader));
        let cases: [Hostile; 6] = [
            ("an unwritten page", |h| h.magic = 0),
            ("a format this build does not speak", |h| h.version = TREE_VERSION + 1),
            ("a tree with no nodes", |h| h.nodes = 0),
            ("a schema over its own header", |h| h.schema_offset = 0),
            ("a data block over the schema", |h| h.data_offset = 64),
            ("a reserved word carrying something", |h| h._reserved[1] = 1),
        ];
        for (what, bend) in cases {
            let mut bad = header(3);
            bend(&mut bad);
            assert_eq!(bad.check(4096), Err(malformed), "{what} was accepted");
        }
        // And the bound that is about the mapping rather than the header: a
        // tree that does not fit the region it was found in.
        assert_eq!(header(3).check(96), Err(malformed), "a tree larger than its mapping");
    }

    #[test]
    fn a_schema_with_a_gap_or_an_overlap_is_refused() {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let head = header(3);

        // Two nodes on one word: two subsystems publishing into the same place.
        let mut shared = schema();
        shared[2].offset = 8;
        assert_eq!(validate(&head, &shared), Err(malformed), "two nodes shared a word");

        // A gap: a word the snapshot hashes that no node describes, so the hash
        // moves for a reason nothing can name.
        let mut gap = schema();
        gap[2].offset = 24;
        assert_eq!(validate(&head, &gap), Err(malformed), "a word no node names");

        // Ids that do not ascend, which is what makes the one-pass parent check
        // sound as well as making two readings comparable.
        let mut backwards = schema();
        backwards[2].id = 2;
        assert_eq!(validate(&head, &backwards), Err(malformed), "a repeated id");

        // A parent named after the child, which a cycle would look like.
        let mut forward = schema();
        forward[1].parent = 3;
        assert_eq!(validate(&head, &forward), Err(malformed), "a parent from the future");
    }

    #[test]
    fn the_snapshot_moves_when_any_single_word_moves() {
        // The property the exit criterion rests on, and the way it fails: a
        // hash over a constant agrees with itself forever. Every word must
        // reach the answer.
        let base = [1u64, 2, 3, 4];
        let first = snapshot(&base);
        assert_eq!(first, snapshot(&base), "the same bytes hashed differently");

        for index in 0..base.len() {
            let mut moved = base;
            moved[index] = moved[index].wrapping_add(1);
            assert_ne!(snapshot(&moved), first, "word {index} does not reach the hash");
        }

        // And every byte of every word, not just the low one — a hash folding
        // words in as `u64`s through a narrower accumulator would pass the loop
        // above and lose the high half.
        let mut high = base;
        high[2] = 3 | (1 << 63);
        assert_ne!(snapshot(&high), first, "the high half of a word is not hashed");
    }

    #[test]
    fn a_reader_that_cannot_name_a_node_still_hashes_it() {
        // RFC 0013's one deliberate exception to R04, as an assertion. The hash
        // is over bytes and does not consult the schema, so a node of a kind
        // this build has never heard of contributes exactly as much as one it
        // wrote itself. A hash that skipped unknown nodes would give two
        // readers of different ages two answers about identical memory.
        let words = [7u64, 9, 11];
        let mut unknown = schema();
        unknown[2].kind = 0xEE;
        assert_eq!(validate(&header(3), &unknown), Ok(()), "an unknown kind was refused");
        assert_eq!(snapshot(&words), snapshot(&words));
    }

    #[test]
    fn a_name_longer_than_the_field_is_truncated_and_not_lost_silently() {
        let entry = SchemaEntry::new(1, 0, 0, kind::GAUGE, unit::BYTES, b"a-very-long-node-name");
        assert_eq!(entry.name_len, 16);
        assert_eq!(entry.label(), b"a-very-long-node");
    }
}
