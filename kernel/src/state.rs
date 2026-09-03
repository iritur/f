// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame's own state tree: counters it already keeps, in a region a reader
//! can map.
//!
//! `f_abi::state` is the wire format and RFC 0013 is the decision. This is the
//! frame publishing itself into one, and the interesting part is what it does
//! *not* do: there is no collect step, no sampling interval and no second copy.
//! A node names a word, and publishing is the store the subsystem was already
//! going to make.
//!
//! # Why the region is not a static
//!
//! Because a `static` here would be kernel-global mutable state, which
//! `cargo xtask lint-percpu` refuses and RFC 0016 argues against. The tree's
//! address is threaded through call sites the way the frame allocator already
//! is. That is more typing and it is the same rule everything else in this
//! kernel follows.
//!
//! # Why the frame is never given back
//!
//! It is machine-wide and it outlives every process that reads it. A tree that
//! could be freed would be a mapping a reader still holds, which is the one
//! shape of use-after-free a capability system is supposed to make impossible —
//! so it simply is not freed, and `Tree` has no way to.
//!
//! # What is published, and what deliberately is not
//!
//! Frame counts, cores, ring tallies, capability slots. Nothing that varies
//! with time: the boot log is a fixture that `cargo xtask trace` hashes, and a
//! tick count or a timestamp in it would make two runs of one commit disagree
//! for a reason that has nothing to do with the kernel. That exclusion is a
//! decision rather than an oversight, and its reversal condition is a boot log
//! that is no longer the reproduction artefact.

use f_abi::state::{SchemaEntry, TreeHeader, WORD, kind, snapshot, unit, validate};

use crate::mem::{FrameAllocator, Order};

/// Node ids, permanent and never reused.
///
/// Written down here rather than derived from the order of the array below,
/// because the array's order is a detail and the ids are the wire. A node that
/// moves in the array keeps its id; a node that is retired takes its id with
/// it, for the reason `TODO.md` never reuses a task id.
pub mod node {
    /// The frame itself, and the root everything hangs under.
    pub const ROOT: u32 = 1;
    /// Physical memory.
    pub const MEMORY: u32 = 2;
    /// Frames the allocator knows about.
    pub const MEMORY_TOTAL: u32 = 3;
    /// Frames it has not handed out.
    pub const MEMORY_FREE: u32 = 4;
    /// The machine's cores.
    pub const TOPOLOGY: u32 = 5;
    /// Cores that answered and are running.
    pub const TOPOLOGY_STARTED: u32 = 6;
    /// The frame's own channel.
    pub const RING: u32 = 7;
    /// Entries the service has executed.
    pub const RING_EXECUTED: u32 = 8;
    /// Entries it has refused.
    pub const RING_REFUSED: u32 = 9;
    /// The capability table.
    pub const CAPS: u32 = 10;
    /// Slots in it.
    pub const CAPS_SLOTS: u32 = 11;
    /// Allocations the calling core's own free lists answered.
    pub const MEMORY_SERVED: u32 = 12;
    /// Allocations that had to reach the machine-wide frontier.
    pub const MEMORY_REFILL: u32 = 13;
    /// Allocations that had to reach another core's free lists.
    pub const MEMORY_REMOTE: u32 = 14;
    /// How much of `MEMORY_REMOTE` the boot's own self-test provoked.
    ///
    /// Published beside the total rather than subtracted from it, because a
    /// reader who maps this tree under load is asking a different question
    /// from the boot log's, and the boot log's answer is a difference it has
    /// already taken. `mem::provoke_remote` withholds the frontier and asks an
    /// empty shard for a frame on every boot, so that a zero on the hot path
    /// is a zero a working counter produced rather than one nothing could ever
    /// move; this node is what lets a reader take that provocation back out.
    pub const MEMORY_FORCED: u32 = 15;
    /// The remapping unit.
    pub const IOMMU: u32 = 16;
    /// Domain ids the unit has.
    pub const IOMMU_DOMAINS: u32 = 17;
    /// Domain ids handed out.
    pub const IOMMU_USED: u32 = 18;
    /// Transactions the unit refused and recorded.
    ///
    /// A counter and not a gauge, and that is the whole point of publishing it:
    /// a reader watching this rise is watching a device try to address memory
    /// nobody gave it. Zero on a healthy machine, and a number nothing else in
    /// the tree can produce.
    pub const IOMMU_FAULTS: u32 = 19;
    /// A node of a kind this build does not name, published on purpose.
    ///
    /// RFC 0013's one deliberate exception to R04 is that a reader skips and
    /// counts an unknown kind rather than refusing the tree. A reader whose
    /// skip path is never taken is a reader whose skip path is untested, and
    /// this is the cheapest possible way to take it: one node, in every tree,
    /// forever.
    pub const RESERVED_KIND: u32 = 63;
}

/// How many nodes this build publishes.
pub const NODES: usize = 20;

/// The schema, written once and never again for a generation.
///
/// Offsets tile the data block in order and `f_abi::state::validate` requires
/// exactly that — no gap, no overlap. A gap would be a word the snapshot hashes
/// that nothing describes, so the hash would move for a reason no reader could
/// name.
const SCHEMA: [SchemaEntry; NODES] = [
    SchemaEntry::new(node::ROOT, 0, 0, kind::SUBTREE, unit::NONE, b"frame"),
    SchemaEntry::new(node::MEMORY, node::ROOT, WORD, kind::SUBTREE, unit::NONE, b"memory"),
    SchemaEntry::new(
        node::MEMORY_TOTAL,
        node::MEMORY,
        2 * WORD,
        kind::GAUGE,
        unit::FRAMES,
        b"total",
    ),
    SchemaEntry::new(node::MEMORY_FREE, node::MEMORY, 3 * WORD, kind::GAUGE, unit::FRAMES, b"free"),
    SchemaEntry::new(node::TOPOLOGY, node::ROOT, 4 * WORD, kind::SUBTREE, unit::NONE, b"topology"),
    SchemaEntry::new(
        node::TOPOLOGY_STARTED,
        node::TOPOLOGY,
        5 * WORD,
        kind::GAUGE,
        unit::CORES,
        b"started",
    ),
    SchemaEntry::new(node::RING, node::ROOT, 6 * WORD, kind::SUBTREE, unit::NONE, b"ring"),
    SchemaEntry::new(
        node::RING_EXECUTED,
        node::RING,
        7 * WORD,
        kind::COUNTER,
        unit::ENTRIES,
        b"executed",
    ),
    SchemaEntry::new(
        node::RING_REFUSED,
        node::RING,
        8 * WORD,
        kind::COUNTER,
        unit::ENTRIES,
        b"refused",
    ),
    SchemaEntry::new(node::CAPS, node::ROOT, 9 * WORD, kind::SUBTREE, unit::NONE, b"caps"),
    SchemaEntry::new(node::CAPS_SLOTS, node::CAPS, 10 * WORD, kind::GAUGE, unit::SLOTS, b"slots"),
    // Memory's three allocation paths, and they sit here rather than beside
    // `free` for a reason worth stating: `validate` requires ids to ascend in
    // schema order, ids are permanent, and these were minted after `topology`
    // and `caps` already held 5 through 11. The tree's *shape* is the parent
    // field, which still puts them under `memory`; the array's order is a
    // detail, exactly as `node` says.
    SchemaEntry::new(
        node::MEMORY_SERVED,
        node::MEMORY,
        11 * WORD,
        kind::COUNTER,
        unit::EVENTS,
        b"served",
    ),
    SchemaEntry::new(
        node::MEMORY_REFILL,
        node::MEMORY,
        12 * WORD,
        kind::COUNTER,
        unit::EVENTS,
        b"refill",
    ),
    SchemaEntry::new(
        node::MEMORY_REMOTE,
        node::MEMORY,
        13 * WORD,
        kind::COUNTER,
        unit::EVENTS,
        b"remote",
    ),
    SchemaEntry::new(
        node::MEMORY_FORCED,
        node::MEMORY,
        14 * WORD,
        kind::COUNTER,
        unit::EVENTS,
        b"forced",
    ),
    SchemaEntry::new(node::IOMMU, node::ROOT, 15 * WORD, kind::SUBTREE, unit::NONE, b"iommu"),
    SchemaEntry::new(
        node::IOMMU_DOMAINS,
        node::IOMMU,
        16 * WORD,
        kind::GAUGE,
        unit::SLOTS,
        b"domains",
    ),
    SchemaEntry::new(node::IOMMU_USED, node::IOMMU, 17 * WORD, kind::GAUGE, unit::SLOTS, b"used"),
    SchemaEntry::new(
        node::IOMMU_FAULTS,
        node::IOMMU,
        18 * WORD,
        kind::COUNTER,
        unit::EVENTS,
        b"faults",
    ),
    // Deliberately a kind nothing names. See `node::RESERVED_KIND`.
    SchemaEntry::new(node::RESERVED_KIND, node::ROOT, 19 * WORD, 0xEE, unit::NONE, b"reserved"),
];

/// Where the schema block starts: immediately after the header, on the
/// thirty-two byte boundary a `SchemaEntry` array needs.
const SCHEMA_AT: u32 = 64;

/// Where the data block starts.
const DATA_AT: u32 = SCHEMA_AT + (NODES as u32) * 32;

/// A published state tree, and the only handle to it.
///
/// Holds the frame's kernel-visible address and its physical one — the second
/// because granting the frame to a process needs it and nothing else in this
/// kernel would otherwise know where the tree lives.
#[derive(Clone, Copy, Debug)]
pub struct Tree {
    base: *mut u8,
    physical: u64,
}

/// Why the tree did not come up. Every one is a bug here, not in a reader.
#[derive(Clone, Copy, Debug)]
pub enum Failure {
    /// No frame to publish into.
    NoFrame,
    /// The header this build wrote does not describe a tree it can read.
    Header(i32),
    /// The schema this build wrote is not one `validate` accepts.
    Schema(i32),
    /// Two snapshots of unchanged bytes disagreed.
    Unstable,
    /// A snapshot did not move when a word did, so the hash is over something
    /// that never varies.
    Deaf,
}

impl Failure {
    /// A line for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoFrame => "no frame for the state tree",
            Self::Header(_) => "the published header is not one this build can read",
            Self::Schema(_) => "the published schema does not describe a tree",
            Self::Unstable => "two snapshots of unchanged bytes disagreed",
            Self::Deaf => "a snapshot did not move when a published word did",
        }
    }
}

impl Tree {
    /// Publish a tree into a fresh frame.
    ///
    /// # Errors
    ///
    /// A [`Failure`], all of which mean this build wrote something it cannot
    /// read — which is exactly what a round trip through memory is for.
    pub fn publish(frames: &mut FrameAllocator) -> Result<Self, Failure> {
        let frame = frames.alloc_zeroed(Order::FRAME).ok_or(Failure::NoFrame)?;
        let tree = Self { base: frames.virt(frame), physical: frame.addr() };

        let header = TreeHeader {
            magic: f_abi::state::TREE_MAGIC,
            version: f_abi::state::TREE_VERSION,
            nodes: NODES as u32,
            schema_offset: SCHEMA_AT,
            data_offset: DATA_AT,
            generation: 0,
            _reserved: [0; 3],
        };

        // SAFETY: a fresh frame, zeroed, frame-aligned — which is stronger than
        // the 64 bytes a `TreeHeader` needs — and nothing else holds a pointer
        // into it.
        unsafe { tree.base.cast::<TreeHeader>().write(header) };

        for (index, entry) in SCHEMA.iter().enumerate() {
            // SAFETY: the schema block starts on a 32-byte boundary inside the
            // frame and is `NODES * 32` bytes, which `DATA_AT` places entirely
            // below the data block and both below a 4 KiB frame.
            unsafe { tree.schema_at(index).write(*entry) };
        }

        // Read back rather than reused. The value that matters is the one in
        // the bytes, and a round trip is what would catch a header whose Rust
        // type and whose wire image disagree.
        // SAFETY: just written, aligned, plain data.
        let written = unsafe { tree.base.cast::<TreeHeader>().read() };
        let extent = u32::try_from(Order::FRAME.bytes()).unwrap_or(u32::MAX);
        written.check(extent).map_err(Failure::Header)?;
        validate(&written, tree.schema()).map_err(Failure::Schema)?;

        Ok(tree)
    }

    /// The physical address of the frame the tree lives in.
    ///
    /// What a grant needs. Unit: bytes, physical.
    #[must_use]
    pub const fn physical(&self) -> u64 {
        self.physical
    }

    fn schema_at(&self, index: usize) -> *mut SchemaEntry {
        // SAFETY: `SCHEMA_AT` is inside the frame.
        let block = unsafe { self.base.add(SCHEMA_AT as usize) }.cast::<SchemaEntry>();
        // SAFETY: every index reaching here is below `NODES`, and the block is
        // `NODES` entries starting at a 32-byte aligned offset in the frame.
        unsafe { block.add(index) }
    }

    /// The schema block, as the reader sees it.
    #[must_use]
    pub fn schema(&self) -> &[SchemaEntry] {
        // SAFETY: `NODES` entries at a 32-byte aligned offset inside the frame,
        // written by `publish` and never written again.
        unsafe { core::slice::from_raw_parts(self.schema_at(0).cast_const(), NODES) }
    }

    /// The data block.
    fn words(&self) -> *mut u64 {
        // SAFETY: `DATA_AT` is eight-byte aligned by construction and inside
        // the frame, which the header check confirmed against the frame's own
        // length rather than against this constant.
        unsafe { self.base.add(DATA_AT as usize) }.cast::<u64>()
    }

    /// Publish a value into the node `id` names.
    ///
    /// Does nothing for an id this build does not publish, which is deliberate:
    /// a publisher naming a node that does not exist is a bug in the publisher,
    /// and the alternative — a panic — would let a mistaken counter take the
    /// machine down. The self-test below is what catches the schema being wrong.
    pub fn set(&self, id: u32, value: u64) {
        let Some(index) = SCHEMA.iter().position(|entry| entry.id == id) else { return };
        // SAFETY: `index` is a position in `SCHEMA`, so it is below `NODES`.
        let slot = unsafe { self.words().add(index) };
        // SAFETY: as above. Volatile because a reader in another address space
        // is loading the same word, and this store may not be folded away or
        // merged with a neighbouring node's.
        unsafe { slot.write_volatile(value) };
    }

    /// Read the whole data block, once, in node-id order.
    ///
    /// The order is the schema's, which `validate` has already required to be
    /// ascending by id — so this is *the* order the hash is defined over and
    /// not merely a convenient one.
    fn read(&self) -> [u64; NODES] {
        let mut out = [0u64; NODES];
        for (index, slot) in out.iter_mut().enumerate() {
            // SAFETY: `index` is below `NODES`.
            let word = unsafe { self.words().add(index) };
            // SAFETY: as above. Volatile and read exactly once, which is what
            // makes a snapshot atomic per node — and the format promises
            // nothing about two nodes being from one instant.
            *slot = unsafe { word.read_volatile() };
        }
        out
    }

    /// The snapshot hash of the tree as it stands.
    #[must_use]
    pub fn snapshot(&self) -> u64 {
        snapshot(&self.read())
    }

    /// Two readings with nothing in between must agree, and a reading after a
    /// change must not.
    ///
    /// The second half is the one that matters. A hash over a block nothing
    /// writes agrees with itself forever, which is indistinguishable from a
    /// hash that works — the same defect `cargo xtask trace` exists to catch
    /// one layer down, and the reason that command builds a kernel that is
    /// *meant* to disagree.
    ///
    /// # Errors
    ///
    /// [`Failure::Unstable`] or [`Failure::Deaf`].
    pub fn self_test(&self) -> Result<u64, Failure> {
        let first = self.snapshot();
        if self.snapshot() != first {
            return Err(Failure::Unstable);
        }

        // Bumped and then put back, on a node whose whole purpose is to be
        // published. `RESERVED_KIND` is the right one to disturb: nothing reads
        // it for its value, so a build that left it bumped would still be
        // publishing the truth about every node anybody uses.
        let before = self.read()[NODES - 1];
        self.set(node::RESERVED_KIND, before.wrapping_add(1));
        let moved = self.snapshot();
        self.set(node::RESERVED_KIND, before);

        if moved == first {
            return Err(Failure::Deaf);
        }
        if self.snapshot() != first {
            return Err(Failure::Unstable);
        }
        Ok(first)
    }

    /// Render the tree to the boot log, one node a line.
    ///
    /// The frame printing its own tree is not the same as a component reading
    /// it — that is what the grant is for — but it is what makes the boot log
    /// carry the evidence, and it is the only renderer in the system that can
    /// afford `kprintln!`.
    pub fn render(&self) {
        let words = self.read();
        for (entry, value) in self.schema().iter().zip(words) {
            let known = matches!(entry.kind, kind::SUBTREE | kind::COUNTER | kind::GAUGE);
            crate::kprint!("  state         {:>3}  ", entry.id);
            for byte in entry.label() {
                crate::kprint!("{}", *byte as char);
            }
            if known {
                crate::kprintln!(" = {value}");
            } else {
                // The skip-and-count path, taken every boot. A reader that
                // cannot name a kind still hashed the word; what it must not do
                // is pretend to interpret it.
                crate::kprintln!(" = <kind {} not named by this build>", entry.kind);
            }
        }
    }
}
