// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The page the frame fills in before this component's first instruction, and
//! the half of it this component fills in before its last.
//!
//! # Why a page and not arguments
//!
//! Because a component is entered with one register's worth of argument —
//! `f_abi::door::Entry`, a selector and a capability index — and everything else
//! it needs to know is a *fact the frame holds*. `user/virtio-blk/src/routing.rs`
//! made that argument first and it is unchanged here: where the rings are, how
//! long they are, how much memory the client granted and what geometry to use
//! are all answers only the frame has, and a component that assumed any of them
//! would be a component that works until the frame's layout moves.
//!
//! [`AT`] is the one address this component holds as a constant, and it is
//! `kernel::process::BOARD` — the same address `f_virtio_blk::routing::AT` is,
//! because `SPAWN_HEAP`'s arrangement applies here too: one address per
//! component shape, so there is one assertion per shape rather than one per
//! field. `kernel/src/objects.rs` is what asserts it.
//!
//! # The two halves, and why the magic goes last
//!
//! The frame writes [`at`] and this component reads it; this component writes
//! [`reported`] and the frame reads it. Each half writes its own magic **last**,
//! which is the whole of the discipline: a reader that arrives before the writer
//! finished finds a zero rather than a plausible tally. RFC 0013's *read, never
//! delivered* — the frame takes these numbers out of memory it granted, and this
//! component is never asked for them.
//!
//! # What this component is told that a driver is not
//!
//! A driver is told where its device is. This component has no device: it is
//! told where the **client's memory** is, because that is what it registers and
//! what a read has to land in. `at::BUFFERS_AT` and `at::BUFFERS_LEN` are that
//! region, mapped at `kernel::process::BLK_QUEUES` — a driver's queue memory
//! address, reused rather than duplicated, because both are one contiguous
//! granted region a component addresses from a constant and no instance is ever
//! both shapes.

/// Where the frame maps this page. Must equal `kernel::process::BOARD`.
///
/// Asserted in `kernel/src/objects.rs` rather than trusted, so a build where the
/// two disagree fails to link rather than reading a page of somebody else's
/// memory as a routing table.
/// Unit: bytes, in this component's address space.
pub const AT: u64 = 0x0041_8000;

/// How much of it this component reads and writes. Unit: bytes.
pub const BYTES: u32 = 4096;

/// What a filled-in half looks like.
///
/// Two different values, so that a component reading the frame's half and a
/// frame reading the component's cannot accept each other's — which is what a
/// single magic would let happen on a page that was written by the wrong side.
/// Unit: none — a sentinel.
pub const MAGIC: u64 = 0x0B_1E_C7_50_F_0000_01;

/// The same, for the half this component writes. See [`MAGIC`].
/// Unit: none — a sentinel.
pub const REPORTED_MAGIC: u64 = 0x0B_1E_C7_50_F_0000_02;

/// What the frame tells this component.
///
/// Offsets in bytes from [`AT`], each naming one 64-bit word. Written out rather
/// than derived from an index, because these are a wire between two crates and a
/// formula would be a second place the layout is stated.
pub mod at {
    /// [`super::MAGIC`], written by the frame **last**. Unit: none.
    pub const MAGIC: u32 = 0x00;
    /// Where the control ring is. Unit: bytes, this component's address space.
    pub const CONTROL_AT: u32 = 0x08;
    /// How long it is. Unit: bytes.
    pub const CONTROL_LEN: u32 = 0x10;
    /// Where the data ring is — the objects ring this component serves.
    /// Unit: bytes, this component's address space.
    pub const DATA_AT: u32 = 0x18;
    /// How long it is. Unit: bytes.
    pub const DATA_LEN: u32 = 0x20;
    /// Where the client's memory is, which this component registers and a read
    /// lands in. Unit: bytes, this component's address space.
    pub const BUFFERS_AT: u32 = 0x28;
    /// How much of it. Unit: bytes.
    pub const BUFFERS_LEN: u32 = 0x30;
    /// One buffer's width, which is also the store's block.
    ///
    /// The two are one number because `ReadPath::read` refuses a set whose
    /// stride is not the block: the device moves whole blocks, so a shorter
    /// buffer would have to be filled by something other than the device and a
    /// longer one would be a buffer the device only partly wrote.
    /// Unit: bytes.
    pub const BLOCK_BYTES: u32 = 0x38;
    /// How many bytes of content to put in the blob this component writes and
    /// then serves. Unit: bytes.
    pub const BLOB_BYTES: u32 = 0x40;
    /// What to fill that content with. Unit: none — a seed.
    pub const SEED: u32 = 0x48;
    /// The ABI version the frame agreed. Unit: none — a version.
    pub const NEGOTIATED_VERSION: u32 = 0x50;
    /// The features it agreed. Unit: none — a bitmask.
    pub const NEGOTIATED_FEATURES: u32 = 0x58;
}

/// What this component tells the frame.
///
/// The frame reads these after the run. Every one of them is a count this
/// component took, which is the point: `E2-B08`'s exit is *counted rather than
/// asserted*, and a number the frame computed for itself would be the frame
/// grading its own homework.
pub mod reported {
    /// Why the run ended — one of [`super::stopped`]. Unit: none — an ordinal.
    pub const OUTCOME: u32 = 0x100;
    /// The registered set this component issued for the client's memory, packed
    /// as `f_abi::buf::SetId`.
    ///
    /// The frame cannot invent this: a set id is a slot and a generation issued
    /// by `f_ring::registry::Table`, and a client that guessed one would be
    /// naming a registration that was never made. So the client is *told* what
    /// it registered, which is RFC 0024's shape seen from the service side.
    /// Unit: none — a packed set identifier.
    pub const SET: u32 = 0x108;
    /// How many buffers that set holds. Unit: count of buffers.
    pub const BUFFERS: u32 = 0x110;
    /// One buffer's width. Unit: bytes.
    pub const STRIDE: u32 = 0x118;
    /// How many content bytes the blob holds. Unit: bytes.
    pub const CONTENT_BYTES: u32 = 0x120;
    /// The blob's content address, four words, little-endian in the order
    /// FIPS 180-4 produces the bytes.
    ///
    /// Published rather than agreed, because a hash is a *function of the
    /// content* and the frame has the content: it fills the same bytes from the
    /// same seed and can check this against its own arithmetic. That check is
    /// what makes the read a read of something the client asked for rather than
    /// of whatever the component felt like returning.
    /// Unit: bytes, thirty-two of them across four words.
    pub const HASH: u32 = 0x128;
    /// Submissions answered, refusals included. Unit: count of entries.
    pub const ENTRIES: u32 = 0x148;
    /// Reads completed and content verified. Unit: count of reads.
    pub const READS: u32 = 0x150;
    /// Content bytes that landed in the client's registered buffer because an
    /// entry asked for them.
    ///
    /// **The number this whole shape exists to produce.**
    /// `intent/0006-state/spec.md` defines an application byte as one a client
    /// submitted on the objects ring, and this is that count, taken by the
    /// component that answered the entry and read by the client across a
    /// privilege boundary. Unit: bytes.
    pub const DELIVERED: u32 = 0x158;
    /// Bytes that went through any buffer that was **not** the client's
    /// registered one, while answering an entry.
    ///
    /// `E2-B08`'s exit as a number, and it must be zero. Unit: bytes.
    pub const STAGED: u32 = 0x160;
    /// Entries answered with a refusal rather than a result.
    /// Unit: count of entries.
    pub const REFUSED: u32 = 0x168;
    /// Notices drained off the control ring at a polling point.
    /// Unit: count of events.
    pub const NOTICES: u32 = 0x170;
    /// [`super::REPORTED_MAGIC`], written **last**. Unit: none.
    pub const MAGIC: u32 = 0x1F8;
}

/// Why a run ended.
///
/// Distinct values and not a boolean, for `f_virtio_blk::routing::stopped`'s
/// reason: a component that stopped because its board was blank and one that
/// was told to stop look identical from outside, and only one of them is the
/// run the boot asked for.
pub mod stopped {
    /// Told to stop, on the control ring, having served. Unit: none.
    pub const TOLD: u64 = 0;
    /// The board was not filled in. Unit: none.
    pub const NO_ROUTING: u64 = 1;
    /// The board was filled in with something this build cannot use — a length
    /// that is not a length, a geometry no store can be built at. Unit: none.
    pub const BAD_ROUTING: u64 = 2;
    /// A ring could not be adopted. Unit: none.
    pub const NO_RING: u64 = 3;
    /// The client's memory could not be registered as a buffer set. Unit: none.
    pub const NO_REGISTRATION: u64 = 4;
    /// The store could not be built, or the blob could not be written into it.
    /// Unit: none.
    pub const NO_STORE: u64 = 5;
    /// The control ring carried something this build cannot name, which means
    /// the peer has stopped speaking. R04. Unit: none.
    pub const BAD_NOTICE: u64 = 6;
    /// The self-check that makes the published zero worth reading did not move
    /// the counter it is there to move. Unit: none.
    pub const NO_SELF_CHECK: u64 = 7;
}

/// The content the frame and this component both put in the blob.
///
/// **A function of the seed and the offset, and not a draw.** RFC 0004 forbids
/// this crate an `Env`, and it would be the wrong tool anyway: what is needed is
/// that two sides compute the *same* bytes without exchanging them, which is a
/// pure function and not a random source. The frame fills the region it expects
/// with this, the component fills the blob with this, and the comparison after
/// the read is then a comparison of two independent computations rather than of
/// a buffer against itself.
///
/// The mixing is deliberately not uniform-looking and does not need to be: what
/// it has to do is differ at every offset, so that a read landing one byte out,
/// or landing the wrong block, fails the comparison. A constant fill would pass
/// every such defect.
#[must_use]
pub const fn content_byte(seed: u64, offset: usize) -> u8 {
    let at = offset as u64;
    let mixed = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(at.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    ((mixed >> 33) ^ mixed) as u8
}
