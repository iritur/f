// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The store's on-disk records, and the generation node kinds folded over them.
//!
//! # Why this is in the wire crate and not in the store
//!
//! Because the only use anybody has for `#[repr(C)]` over these types is to
//! view device bytes as a struct, that view is a pointer cast, and RFC 0001
//! puts a pointer cast inside the frame. A library above the frame that wanted
//! one would have to be granted `unsafe`, and the grant is the thing worth
//! refusing — so the types live here, where `unsafe` is already permitted, and
//! nothing casts anyway: every record below is encoded by hand and decoded
//! field by field.
//!
//! **There is no `#[repr(C)]` in this file, and that is deliberate.** The layout
//! is the encoding, not the compiler's opinion of the struct, so each type
//! states its own byte width and a `const` assertion below it requires that
//! width to equal the sum of its fields. Padding is what that assertion exists
//! to forbid: an unhashed byte inside a hashed structure is a place two
//! generations can differ while naming the same thing.
//!
//! The second reason they are here is `cargo xtask lint-units`, which holds
//! `abi/` to a stated unit on every public field. These records carry hashes,
//! counts and indices — three kinds of number that look alike — and a field
//! whose unit is only obvious to the person who wrote it is exactly the defect
//! R03 exists to catch.
//!
//! # The four node kinds, and why a fifth is an RFC
//!
//! [`node::BYTES`], [`node::COMPONENT`], [`node::TOPOLOGY`] and
//! [`node::GENERATION`] are the whole vocabulary of a generation expression, and
//! they are enumerated *here* rather than in `f-generation` so that adding one
//! is a diff to the wire crate. Every change to this crate is an ABI change and
//! is reviewed as one, which makes "a fifth node kind is an RFC" a rule the
//! process enforces rather than a convention a reader is asked to remember. The
//! spec's own reversal condition is a topology `E2-B05` cannot express as a tree
//! of these four.
//!
//! Nothing here is decoded from bytes this side wrote. The device wrote them,
//! and a device is a peer in the sense `ring/src/mapping.rs` means — so every
//! `from_bytes` refuses on magic, on kind and on length before it returns any
//! field, and a caller holding a decoded value may read it without checking it
//! again.
//!
//! # No timestamp
//!
//! There is no clock in any record in this file. Order between generations is a
//! counter the writer chooses; RFC 0004 is why, and the practical half is that a
//! generation whose identity moved because a clock did would not be the same
//! generation on two machines — which is exactly what `E2-P06` compares.

use crate::error;
use crate::manifest::NAME_MAX;

/// The first four bytes of every node. `F_ND`, little-endian.
///
/// A magic per *node* and not only per file, because a generation tree is
/// walked from offsets a parent computed, and a wrong offset lands on plausible
/// bytes far more often than it lands on nothing.
/// Unit: none — a fixed byte pattern.
pub const MAGIC: u32 = 0x444e_5f46;

/// How many bytes of header every node begins with: [`MAGIC`], the kind, and
/// the node's own encoded length.
/// Unit: bytes.
pub const HEADER_BYTES: usize = 8;

/// The node kinds a generation expression is built from.
///
/// Zero is not a kind, on purpose and for the reason `manifest::domain` reserves
/// it: a zeroed block must never decode as anything. A value this build does not
/// know is refused and never skipped — R04.
pub mod node {
    /// A leaf naming content this tree did not compile: a blob or an object
    /// hash. The frame image is the one this slice uses.
    pub const BYTES: u16 = 1;
    /// A leaf naming a component file — RFC 0030's record and image together,
    /// which is already one hash.
    pub const COMPONENT: u16 = 2;
    /// An ordered list of named components and the routes between them.
    pub const TOPOLOGY: u16 = 3;
    /// The root: the frame hash and the topology hash and nothing else.
    pub const GENERATION: u16 = 4;

    /// Is this a kind this build knows?
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, BYTES | COMPONENT | TOPOLOGY | GENERATION)
    }

    /// Is this one of the two leaf kinds, which share an encoding and differ in
    /// what a resolver should expect to find at the address?
    #[must_use]
    pub const fn leaf(value: u16) -> bool {
        matches!(value, BYTES | COMPONENT)
    }

    /// A word for a log or a rendering.
    #[must_use]
    pub const fn label(value: u16) -> &'static str {
        match value {
            BYTES => "bytes",
            COMPONENT => "component",
            TOPOLOGY => "topology",
            GENERATION => "generation",
            _ => "unknown",
        }
    }
}

/// How many components one topology may name.
///
/// A bound rather than a growable list, because the whole tree is decoded
/// without an allocator and a decoder with no bound is a decoder that trusts a
/// length field a peer wrote. Sixty-four is far above the four this tree has and
/// far below anything that would make a mount-time walk a cost worth measuring,
/// which is the spec's stated reversal for that walk.
/// Unit: components.
pub const MEMBERS_MAX: usize = 64;

/// How many routes one topology may declare.
/// Unit: routes.
pub const ROUTES_MAX: usize = 256;

/// A leaf: one content address and the name it is known by.
///
/// # Why the two leaf kinds share one encoding
///
/// They carry the same two fields, and duplicating the codec would create two
/// places for a decoder to disagree with an encoder about one layout. What
/// separates them is [`Leaf::kind`], which says what a resolver should expect to
/// find at the address: [`node::BYTES`] is content that arrived from outside
/// this tree, [`node::COMPONENT`] is a component file this tree compiled from a
/// manifest. Keeping the distinction in a field rather than in a type is also
/// what puts the *set* of leaf kinds in one place a reader can count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Leaf {
    /// Which leaf this is.
    /// Unit: none — a [`node`] constant, either [`node::BYTES`] or
    /// [`node::COMPONENT`]. Zero is not a kind.
    pub kind: u16,
    /// The SHA-256 of the content this leaf names.
    /// Unit: bytes, exactly 32 of them — a content address, in the order FIPS
    /// 180-4 produces it and printed as 64 hexadecimal characters.
    pub hash: [u8; 32],
    /// The name this leaf is known by in the topology, NUL-padded.
    /// Unit: bytes of ASCII from `[a-z0-9-]`, at most [`NAME_MAX`], no edge
    /// hyphen, never empty. The padding is not part of the name; it is hashed
    /// because it is a field of fixed width, not because it means anything.
    pub name: [u8; NAME_MAX],
}

impl Leaf {
    /// The encoded width of a leaf.
    /// Unit: bytes.
    pub const BYTES: usize = HEADER_BYTES + 32 + NAME_MAX;

    /// Encode. Little-endian, field by field, nothing skipped.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Leaf::BYTES] {
        let mut out = [0u8; Leaf::BYTES];
        put_header(&mut out, self.kind, Leaf::BYTES);
        out[8..40].copy_from_slice(&self.hash);
        out[40..72].copy_from_slice(&self.name);
        out
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// # Errors
    ///
    /// A packed [`error::ARGUMENT`] naming the disbelief: the magic, a declared
    /// length that is not this one, or a kind this build does not know or that
    /// is not a leaf.
    pub fn from_bytes(raw: &[u8; Leaf::BYTES]) -> Result<Self, i32> {
        let kind = header(raw, Leaf::BYTES)?;
        if !node::leaf(kind) {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&raw[8..40]);
        let mut name = [0u8; NAME_MAX];
        name.copy_from_slice(&raw[40..72]);
        Ok(Self { kind, hash, name })
    }
}

/// The head of a topology node: how many members and how many routes follow.
///
/// A topology is the one node whose encoded length is not a constant, and the
/// length is a function of these two counts rather than of anything a reader has
/// to search for. Both are bounded before either is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Topology {
    /// How many components this topology names.
    /// Unit: components, at most [`MEMBERS_MAX`]. Zero is allowed and means a
    /// topology with nothing in it, which folds to a hash like any other.
    pub members: u16,
    /// How many routes this topology declares.
    /// Unit: routes, at most [`ROUTES_MAX`].
    pub routes: u16,
}

impl Topology {
    /// The encoded width of a topology's head, before the routes that follow.
    /// Unit: bytes.
    pub const HEAD_BYTES: usize = HEADER_BYTES + 2 + 2;

    /// The whole stored node's encoded width: the head and the routes after it.
    /// Unit: bytes.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        Self::HEAD_BYTES + self.routes as usize * Route::BYTES
    }

    /// Encode the head. The routes are appended by the caller, which is what
    /// lets a folder stream a topology without ever holding one.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Topology::HEAD_BYTES] {
        let mut out = [0u8; Topology::HEAD_BYTES];
        put_header(&mut out, node::TOPOLOGY, self.bytes());
        out[8..10].copy_from_slice(&self.members.to_le_bytes());
        out[10..12].copy_from_slice(&self.routes.to_le_bytes());
        out
    }

    /// Decode the head, refusing before either count is handed back.
    ///
    /// The declared length is checked against the counts, and that is the check
    /// that matters: a head whose length field and route count disagree is a
    /// head that would walk a reader off the end of the node.
    ///
    /// # Errors
    ///
    /// A packed [`error::ARGUMENT`]: the magic, the kind, a count past its
    /// bound, or a declared length the counts do not produce.
    pub fn from_bytes(raw: &[u8; Topology::HEAD_BYTES]) -> Result<Self, i32> {
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        if u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) != MAGIC {
            return Err(bad);
        }
        if u16::from_le_bytes([raw[4], raw[5]]) != node::TOPOLOGY {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }
        let members = u16::from_le_bytes([raw[8], raw[9]]);
        let routes = u16::from_le_bytes([raw[10], raw[11]]);
        if members as usize > MEMBERS_MAX || routes as usize > ROUTES_MAX {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }
        let head = Self { members, routes };
        if u16::from_le_bytes([raw[6], raw[7]]) as usize != head.bytes() {
            return Err(bad);
        }
        Ok(head)
    }
}

/// One route: a component, the capability it needs, and where it comes from.
///
/// # Why indices and not names
///
/// A name inside a route would be a second place a component's name is written,
/// and two places can differ. An index cannot: it either names a member of this
/// topology or it is dangling, and dangling is refused. That is also what keeps
/// the encoding free of strings that are not names — the spec's phrase — since
/// the only string here is the capability's own.
///
/// A route carries no magic and no kind of its own, because it is never found on
/// its own: it lies inside a topology node whose head has already been believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    /// The component that needs the capability.
    /// Unit: index into the topology's component list, zero-based.
    pub component: u16,
    /// The component that provides it.
    /// Unit: index into the topology's component list, zero-based.
    pub source: u16,
    /// The capability's name, NUL-padded.
    /// Unit: bytes of ASCII from `[a-z0-9-]`, at most [`NAME_MAX`], no edge
    /// hyphen, never empty.
    pub capability: [u8; NAME_MAX],
}

impl Route {
    /// The encoded width of one route.
    /// Unit: bytes.
    pub const BYTES: usize = 2 + 2 + NAME_MAX;

    /// Encode.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Route::BYTES] {
        let mut out = [0u8; Route::BYTES];
        out[0..2].copy_from_slice(&self.component.to_le_bytes());
        out[2..4].copy_from_slice(&self.source.to_le_bytes());
        out[4..36].copy_from_slice(&self.capability);
        out
    }

    /// Decode.
    ///
    /// Total rather than fallible, and that is the honest signature: there is no
    /// magic to disbelieve, and every bit pattern of two integers and thirty-two
    /// bytes is a value. What a route can be *wrong* about — a dangling index, a
    /// capability outside its alphabet, its place in the order — is a property of
    /// the topology around it, so `f-generation`'s checker is where it is refused.
    #[must_use]
    pub fn from_bytes(raw: &[u8; Route::BYTES]) -> Self {
        let mut capability = [0u8; NAME_MAX];
        capability.copy_from_slice(&raw[4..36]);
        Self {
            component: u16::from_le_bytes([raw[0], raw[1]]),
            source: u16::from_le_bytes([raw[2], raw[3]]),
            capability,
        }
    }
}

/// The root node.
///
/// # Why it stores nothing of its own
///
/// The fold's rule is that a node's hash is the SHA-256 of its encoding *with
/// each child replaced by that child's hash*. Applied here, the hashed
/// generation node is exactly the frame hash and the topology hash and nothing
/// else — which is the sentence the spec insists on. Storing those two hashes in
/// the tree *as well* would be storing a value that is already derivable, in a
/// structure whose entire claim is that it has one representation; two
/// representations of one fact is how two encoders come to disagree while both
/// pass their own tests.
///
/// What a mount needs — *every child of the generation node resolves* — it gets
/// by walking the tree it was handed, which is the same walk the fold makes.
///
/// *What would reverse this:* a reader that must believe a root without holding
/// the tree under it, which is a different problem from the one `E2-P07` has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation;

impl Generation {
    /// The encoded width of the stored root node: a header and nothing else.
    /// Unit: bytes.
    pub const BYTES: usize = HEADER_BYTES;

    /// The width the *folded* root node has — the header and its two children's
    /// hashes — which is what a folder streams and never what a tree stores.
    /// Unit: bytes.
    pub const FOLDED_BYTES: usize = HEADER_BYTES + 32 + 32;

    /// Encode.
    #[must_use]
    pub fn to_bytes() -> [u8; Generation::BYTES] {
        let mut out = [0u8; Generation::BYTES];
        put_header(&mut out, node::GENERATION, Generation::FOLDED_BYTES);
        out
    }

    /// Decode, refusing on the magic, the kind and the declared length.
    ///
    /// The length in the header is the *folded* width, because the header is
    /// hashed and what is hashed is the folded node. A reader walking the tree
    /// therefore does not use it as a stride — [`Generation::BYTES`] is what it
    /// steps by — and the field is checked anyway, because a header field nobody
    /// checks is a header field two writers can disagree about.
    ///
    /// # Errors
    ///
    /// A packed [`error::ARGUMENT`].
    pub fn from_bytes(raw: &[u8; Generation::BYTES]) -> Result<Self, i32> {
        let kind = header(raw, Generation::FOLDED_BYTES)?;
        if kind != node::GENERATION {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }
        Ok(Self)
    }
}

/// Write the eight-byte header every node begins with.
fn put_header(out: &mut [u8], kind: u16, bytes: usize) {
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..6].copy_from_slice(&kind.to_le_bytes());
    // Every node this crate can construct fits a `u16`, which the `const`
    // assertion at the bottom of this file makes a fact rather than a hope.
    out[6..8].copy_from_slice(&(bytes as u16).to_le_bytes());
}

/// Believe a node's header, and return the kind it declares.
///
/// The magic first, the length second, the kind last, because the kind is the
/// only one of the three whose refusal a caller distinguishes.
fn header(raw: &[u8], bytes: usize) -> Result<u16, i32> {
    let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
    if raw.len() < HEADER_BYTES {
        return Err(bad);
    }
    if u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) != MAGIC {
        return Err(bad);
    }
    if u16::from_le_bytes([raw[6], raw[7]]) as usize != bytes {
        return Err(bad);
    }
    let kind = u16::from_le_bytes([raw[4], raw[5]]);
    if !node::known(kind) {
        return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
    }
    Ok(kind)
}

// Every node's encoded width is the sum of its field widths, asserted at compile
// time. This is the whole of "no padding is ever hashed": if a field is added
// and the width is not, the build stops here rather than producing a generation
// whose root covers bytes nobody named.
const _: () = assert!(Leaf::BYTES == HEADER_BYTES + 32 + NAME_MAX);
const _: () = assert!(Topology::HEAD_BYTES == HEADER_BYTES + 2 + 2);
const _: () = assert!(Route::BYTES == 2 + 2 + NAME_MAX);
const _: () = assert!(Generation::BYTES == HEADER_BYTES);
const _: () = assert!(Generation::FOLDED_BYTES == HEADER_BYTES + 32 + 32);
// And every node fits the `u16` its header states its length in, which is what
// makes the cast in `put_header` a fact rather than a hope.
const _: () = assert!(Topology::HEAD_BYTES + ROUTES_MAX * Route::BYTES <= u16::MAX as usize);

// ---------------------------------------------------------------------------
// The on-disk records: the superblock, the blob header and the root record.
//
// The generation nodes above are what a *tree* is made of; these three are what
// a *device* is made of. They are in one file because they share the rule that
// puts both here — a peer wrote these bytes, the peer is the device, and a
// reader that trusts them before it has refused them is the defect this whole
// file is shaped around.
// ---------------------------------------------------------------------------

/// The superblock's magic. `F_SB`, little-endian.
///
/// A magic per record kind rather than one for the store, because these three
/// records are found at addresses a *caller* computed — block zero, an index
/// entry, a zone's write pointer — and a wrong address lands on a plausible
/// record far more often than it lands on nothing. Three magics make that
/// arithmetic error a refusal instead of a misreading.
/// Unit: none — a fixed byte pattern.
pub const SUPERBLOCK_MAGIC: u32 = 0x4253_5f46;

/// The blob header's magic. `F_BH`, little-endian.
/// Unit: none — a fixed byte pattern.
pub const BLOB_MAGIC: u32 = 0x4842_5f46;

/// The root record's magic. `F_RR`, little-endian.
/// Unit: none — a fixed byte pattern.
pub const ROOT_MAGIC: u32 = 0x5252_5f46;

/// The version of the on-disk format this build writes and believes.
///
/// One, and a mount refuses anything else rather than guessing which fields
/// moved. Nothing has ever formatted a device with this format, so there is
/// exactly one version in the world and the first migration is still free.
/// Unit: none — a version.
pub const SCHEMA: u32 = 1;

/// The codes the store adds to the domains RFC 0010 fixes.
///
/// # Why they are here and not in `error`'s own modules
///
/// RFC 0010 says a service may add codes freely and may not add domains, and
/// the store is a service — `E2-B08`'s `user/objects` is the component that
/// will answer over a ring. `error::argument`'s eight codes are all about a
/// *ring entry*: an opcode, a flag bit, a reserved word, a class. A store's
/// codes are about a record and a device, so they are defined beside the
/// records rather than in the middle of the ring's vocabulary.
///
/// What that costs, said out loud because it is the objection: there are now
/// two files where an [`error::ARGUMENT`] code is defined, and two files can
/// come to disagree about a number. The mitigation is that this module
/// *continues* `error::argument`'s numbering rather than starting again at one,
/// so a collision would be a duplicate integer a reader finds by searching for
/// it, rather than two meanings wearing one number.
pub mod code {
    /// The caller's buffer cannot hold what it asked for. Continues
    /// [`crate::error::argument`], whose last code is 8.
    /// Unit: none — a code within [`crate::error::ARGUMENT`].
    pub const SHORT_BUFFER: u16 = 9;

    /// The content read back does not hash to the name it was stored under.
    ///
    /// The first code in [`crate::error::DEVICE`], which had none: every
    /// failure that domain described so far was reported by hardware, and this
    /// one is a disagreement the store detects about what the hardware
    /// returned.
    /// Unit: none — a code within [`crate::error::DEVICE`].
    pub const CONTENT_MISMATCH: u16 = 1;
}

/// The refusals the store's records return, named once.
///
/// # Why these are constants and not `error::pack` calls at each site
///
/// Because a caller compares against them. `blob/tests/million.rs` requires
/// that a flipped byte is refused as [`refusal::CONTENT`] and not as something
/// else that also happens to be an error, and a test that spelled the packed
/// integer out itself would pass on a store that had started returning the
/// right refusal for the wrong reason.
pub mod refusal {
    use crate::error;

    /// The magic, the declared length, or a record torn across a power cut: the
    /// bytes are not this record. Also a root record whose `check` does not
    /// verify, which is the same statement — the bytes are not a record anybody
    /// finished writing.
    /// Unit: none — a packed [`error`] value.
    pub const MALFORMED: i32 = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

    /// A kind, a schema, a flag or a count this build does not know. Refused
    /// and never skipped, R04: a fifth blob kind arriving without its RFC reads
    /// as a record this reader cannot describe, and describing it anyway is how
    /// two readers come to disagree about one device.
    /// Unit: none — a packed [`error`] value.
    pub const UNKNOWN: i32 = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);

    /// The address is not one this operation can act on — which is
    /// [`error::argument::BAD_ADDRESS`]'s own words, and it covers both places
    /// the store means it: a hash this store holds nothing under, and a block
    /// outside the device. One name rather than two, because the two are the
    /// same statement at two granularities and a caller does the same thing
    /// with either.
    /// Unit: none — a packed [`error`] value.
    pub const ADDRESS: i32 = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);

    /// The device has no room left for the record being written.
    /// Unit: none — a packed [`error`] value.
    pub const FULL: i32 = error::pack(error::RESOURCE, error::resource::DEVICE_FULL);

    /// The caller's buffer is shorter than the content it asked for.
    /// Unit: none — a packed [`error`] value.
    pub const SHORT_BUFFER: i32 = error::pack(error::ARGUMENT, super::code::SHORT_BUFFER);

    /// The bytes read back do not hash to the name they were stored under.
    ///
    /// The one refusal in this list that is not about a field: every other
    /// entry says the record is not what it claims to be, and this one says the
    /// record is exactly what it claims and the device did not return what was
    /// written. `blob/src/store.rs` raises it and `blob/tests/million.rs` is
    /// what makes it reachable, because a verifier nothing can fail is
    /// indistinguishable from one that cannot fail.
    /// Unit: none — a packed [`error`] value.
    pub const CONTENT: i32 = error::pack(error::DEVICE, super::code::CONTENT_MISMATCH);
}

/// What a blob holds. The four kinds, and a fifth is an RFC.
///
/// A module of constants rather than an `enum`, which is this crate's shape
/// wherever a value arrives from a peer: an `enum` makes the *known* values a
/// type and leaves each decoder to invent what an unknown one is, and every
/// decoder that invents something invents something slightly different.
/// [`kind::known`] is the one answer, and it is `false` for zero — a zeroed
/// block must never decode as a blob.
pub mod kind {
    /// A run of bytes named by SHA-256 over exactly those bytes and by nothing
    /// else, which is what makes two objects sharing a run share its storage.
    ///
    /// **Two producers cut these, and that is deliberate.** The chunker cuts
    /// them at boundaries the content chose; `f_blob::extent` cuts them at
    /// fixed `EXTENT_BYTES` offsets, and RFC 0058 forecloses a fifth kind for
    /// the second — *adding a fifth blob kind is a diff to `abi/` and an RFC by
    /// rule*. Nothing downstream needs to tell them apart: a chunk is its bytes,
    /// the parent record says how the bytes are arranged, and a piece that
    /// happens to be byte-identical to a chunk is one record rather than two,
    /// which is deduplication rather than a collision. What a *piece* has that
    /// a chunk does not is its position in [`super::ExtentHead`]'s list, and
    /// that is the parent's fact, not this one's.
    pub const CHUNK: u16 = 1;
    /// An [`super::ObjectHead`] and its ordered chunk hashes: what makes an
    /// object one hash rather than a list somebody has to keep together.
    pub const OBJECT: u16 = 2;
    /// The second object kind — a [`super::ExtentHead`] and its ordered piece
    /// hashes, where a piece is a fixed `EXTENT_BYTES` offset rather than a
    /// boundary the content chose, and a write replaces whole pieces rather
    /// than re-chunking a region. The number was fixed before anything wrote
    /// one, so that the kind space was decided in one commit rather than
    /// extended by whichever commit first needed it; `f_blob::extent` is the
    /// writer, and `E2-B09` is where it arrived. RFC 0058.
    pub const EXTENT: u16 = 3;
    /// A stored generation tree: the bytes `f-generation` folded, kept as a
    /// blob so that a root's children resolve through the same read path as
    /// everything else.
    pub const GENERATION: u16 = 4;

    /// Is this a kind this build knows?
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, CHUNK | OBJECT | EXTENT | GENERATION)
    }

    /// A word for a log or a rendering.
    #[must_use]
    pub const fn label(value: u16) -> &'static str {
        match value {
            CHUNK => "chunk",
            OBJECT => "object",
            EXTENT => "extent",
            GENERATION => "generation",
            _ => "unknown",
        }
    }
}

/// The device's own description of itself, written once into a conventional
/// zone.
///
/// # Why the chunker's parameters are on the device
///
/// Because a chunker that changed under a mount decides every object hash and
/// nothing else on the device would say so. The failure is not a crash: it is a
/// store that keeps working, writes chunks the old boundaries would never have
/// produced, and silently stops deduplicating against everything written before
/// it. So the parameters are here, and a mount whose compiled-in values
/// disagree with these refuses. `blob::store::refuse_a_chunker_that_moved` is
/// that check, and it is in `blob/` because `abi/` does not know what any
/// build's chunker is — which is the right way round: this crate states the
/// question and the crate that has an answer answers it.
///
/// # Why there is no current-root field
///
/// There is nothing mutable on the device at all. A pointer to the current root
/// would be a field that can be stale, torn, or written in the wrong order
/// against the thing it points at; mount instead reads both root zones' write
/// pointers from the device with `ZONE_REPORT` and takes the highest
/// `generation` that verifies. RFC 0060.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Superblock {
    /// The format version these fields are laid out in.
    /// Unit: none — a version. [`SCHEMA`] is what this build writes, and a
    /// different one is refused rather than guessed at.
    pub schema: u32,
    /// The device's logical block. Every record starts at a block boundary and
    /// is padded to a whole number of them.
    /// Unit: bytes.
    pub block_bytes: u32,
    /// How many zones the device has.
    /// Unit: count of zones.
    pub zones: u32,
    /// The first root zone.
    /// Unit: zone index, zero-based.
    pub root_zone_a: u32,
    /// The second root zone. Two of them, with `ROOT_CARRY` records carried
    /// between them before a reset, so that there is never a moment with no
    /// durable root — a single root zone could only be emptied by destroying
    /// the record that was current. `f-zone` owns the carry; the superblock
    /// owns these two numbers, because they are geometry.
    /// Unit: zone index, zero-based.
    pub root_zone_b: u32,
    /// The smallest chunk the chunker will cut.
    /// Unit: bytes.
    pub chunk_min_bytes: u32,
    /// The size the mask width was chosen around.
    /// Unit: bytes.
    pub chunk_target_bytes: u32,
    /// The size at which a boundary is forced whatever the content says.
    /// Unit: bytes.
    pub chunk_max_bytes: u32,
    /// How many bits of the mask a candidate is tested against. One width and
    /// not two: RFC 0061 retired normalised chunking.
    /// Unit: bits.
    pub mask_bits: u32,
    /// A zone's capacity.
    /// Unit: bytes.
    pub zone_bytes: u64,
    /// The identity the gear table was derived from.
    ///
    /// The number `f_env::split::label` gives the label text, and not the text
    /// itself: the number is what the derivation consumes, so a label that was
    /// edited and a derivation that moved under an unchanged label are the same
    /// refusal. What it costs is that a hex dump of the superblock shows a word
    /// rather than `f-blob gear v1`, and the text is in `blob/src/gear.rs`
    /// beside the derivation that spends it.
    /// Unit: none — an identity, not a quantity.
    pub gear_label: u64,
    /// The identity the mask was derived from, separately from the gear table
    /// so that a width can move without disturbing a table every object hash
    /// depends on.
    /// Unit: none — an identity, not a quantity.
    pub mask_label: u64,
}

impl Superblock {
    /// The encoded width of a superblock.
    /// Unit: bytes.
    pub const BYTES: usize = 68;

    /// Encode. Little-endian, field by field, nothing skipped.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Superblock::BYTES] {
        let mut out = [0u8; Superblock::BYTES];
        out[0..4].copy_from_slice(&SUPERBLOCK_MAGIC.to_le_bytes());
        out[4..8].copy_from_slice(&(Superblock::BYTES as u32).to_le_bytes());
        out[8..12].copy_from_slice(&self.schema.to_le_bytes());
        out[12..16].copy_from_slice(&self.block_bytes.to_le_bytes());
        out[16..20].copy_from_slice(&self.zones.to_le_bytes());
        out[20..24].copy_from_slice(&self.root_zone_a.to_le_bytes());
        out[24..28].copy_from_slice(&self.root_zone_b.to_le_bytes());
        out[28..32].copy_from_slice(&self.chunk_min_bytes.to_le_bytes());
        out[32..36].copy_from_slice(&self.chunk_target_bytes.to_le_bytes());
        out[36..40].copy_from_slice(&self.chunk_max_bytes.to_le_bytes());
        out[40..44].copy_from_slice(&self.mask_bits.to_le_bytes());
        out[44..52].copy_from_slice(&self.zone_bytes.to_le_bytes());
        out[52..60].copy_from_slice(&self.gear_label.to_le_bytes());
        out[60..68].copy_from_slice(&self.mask_label.to_le_bytes());
        out
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The magic, then the declared length, then the schema — the same order
    /// every record in this file refuses in, and the schema is where *kind*
    /// sits for a record there is only one of. A schema this build does not
    /// know is refused rather than read with the fields it recognises, because
    /// a field that moved between versions reads perfectly and means something
    /// else.
    ///
    /// A zero `block_bytes` is refused too, and it is the one bound checked
    /// here rather than by a caller: every address in the store is a multiple
    /// of it, so a zero would surface as a division by zero several files away
    /// from the record that caused it.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for the magic, the length or a zero
    /// `block_bytes`; [`refusal::UNKNOWN`] for a schema this build does not
    /// write.
    pub fn from_bytes(raw: &[u8; Superblock::BYTES]) -> Result<Self, i32> {
        prologue(raw, SUPERBLOCK_MAGIC, Superblock::BYTES)?;
        let schema = word(raw, 8);
        if schema != SCHEMA {
            return Err(refusal::UNKNOWN);
        }
        let block_bytes = word(raw, 12);
        if block_bytes == 0 {
            return Err(refusal::MALFORMED);
        }
        Ok(Self {
            schema,
            block_bytes,
            zones: word(raw, 16),
            root_zone_a: word(raw, 20),
            root_zone_b: word(raw, 24),
            chunk_min_bytes: word(raw, 28),
            chunk_target_bytes: word(raw, 32),
            chunk_max_bytes: word(raw, 36),
            mask_bits: word(raw, 40),
            zone_bytes: long(raw, 44),
            gear_label: long(raw, 52),
            mask_label: long(raw, 60),
        })
    }
}

/// The header at every block-aligned position a blob starts.
///
/// The content follows it immediately and is padded with zeros to a whole
/// number of blocks. The padding is not covered by [`Header::hash`] and is not
/// covered by anything else either: it is addressing rather than content, and
/// the record means `content_bytes` bytes however many blocks it occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// What the content is.
    /// Unit: none — a [`kind`] constant. Zero is not a kind.
    pub kind: u16,
    /// Zero until something is named.
    ///
    /// Reserved here rather than added later, because adding a field to a
    /// record that has been written to a device is a migration and reserving
    /// two bytes now is not. A non-zero value is refused rather than ignored —
    /// R04, and the rule `error::argument::RESERVED_NOT_ZERO` already states
    /// for a ring entry: a bit that is silently dropped is two peers with
    /// different beliefs about what just happened.
    /// Unit: none — a bit set, currently empty.
    pub flags: u16,
    /// The content that follows, before the padding to the device's block.
    /// Unit: bytes.
    pub content_bytes: u64,
    /// The SHA-256 of that content.
    ///
    /// It is *also* the name the blob is stored under, and this is the one
    /// place the format writes a fact twice. The second copy is what makes a
    /// read checkable: `get` hashes the content it read and compares, so a
    /// device that returned the wrong block — or the right block with a byte
    /// flipped in it — is caught by the record rather than by whoever asked
    /// for it.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub hash: [u8; 32],
}

impl Header {
    /// The encoded width of a blob header.
    /// Unit: bytes.
    pub const BYTES: usize = 52;

    /// Encode.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; Header::BYTES] {
        let mut out = [0u8; Header::BYTES];
        out[0..4].copy_from_slice(&BLOB_MAGIC.to_le_bytes());
        out[4..8].copy_from_slice(&(Header::BYTES as u32).to_le_bytes());
        out[8..10].copy_from_slice(&self.kind.to_le_bytes());
        out[10..12].copy_from_slice(&self.flags.to_le_bytes());
        out[12..20].copy_from_slice(&self.content_bytes.to_le_bytes());
        out[20..52].copy_from_slice(&self.hash);
        out
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The declared length is the *header's* own width and never the record's.
    /// The record's width is `content_bytes` rounded up to the device's block,
    /// which is derivable from two numbers that are already here — and a second
    /// statement of a derivable length is a second place two writers can
    /// disagree while both pass their own tests.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for the magic or the length; [`refusal::UNKNOWN`]
    /// for a kind this build does not know or a flag bit it does not define.
    pub fn from_bytes(raw: &[u8; Header::BYTES]) -> Result<Self, i32> {
        prologue(raw, BLOB_MAGIC, Header::BYTES)?;
        let kind = short(raw, 8);
        if !kind::known(kind) {
            return Err(refusal::UNKNOWN);
        }
        let flags = short(raw, 10);
        if flags != 0 {
            return Err(refusal::UNKNOWN);
        }
        Ok(Self { kind, flags, content_bytes: long(raw, 12), hash: digest(raw, 20) })
    }
}

/// How many chunk hashes one object may name.
///
/// A bound, because the count is a length field a peer wrote and a decoder with
/// no bound is a decoder that allocates whatever the device asks it to. A
/// million chunks at `CHUNK_TARGET_BYTES` is an object of about 64 GiB, which
/// is far past anything this system stores in one object and far below what
/// would make the list itself the cost. An object larger than that wants a
/// second level of indirection rather than a wider bound, and that is a design
/// change with an RFC rather than a constant edited here.
/// Unit: count of chunk hashes.
pub const CHUNKS_MAX: usize = 1 << 20;

/// The head of an object blob: how long the object is, and how many chunks
/// follow.
///
/// # Why an object stores its own length
///
/// Because the sum of the chunk lengths is not available without reading every
/// chunk, and a reader that has to fetch a hundred blobs to answer *how big is
/// this* is a reader nobody will use. It is also what catches a truncated chunk
/// list: the bytes the chunks deliver must add up to this number, and
/// `blob/src/store.rs` refuses when they do not.
///
/// It carries no magic of its own, for [`Route`]'s reason: it is never found on
/// its own. It lies inside a blob whose content hash has already been verified
/// against the name the caller asked for, so by the time these twelve bytes are
/// read, they are known to be the bytes that were written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectHead {
    /// The object's logical size — the bytes its chunks reconstruct.
    /// Unit: bytes.
    pub object_bytes: u64,
    /// How many chunk hashes follow this head, in order.
    /// Unit: count of chunk hashes, at most [`CHUNKS_MAX`].
    pub chunks: u32,
}

impl ObjectHead {
    /// The encoded width of an object's head, before the hashes that follow.
    /// Unit: bytes.
    pub const HEAD_BYTES: usize = 12;

    /// The encoded width of the whole object blob's content: the head and the
    /// hashes after it.
    /// Unit: bytes.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        Self::HEAD_BYTES + self.chunks as usize * 32
    }

    /// Encode the head. The hashes are appended by the caller, which is what
    /// lets a writer stream a chunk list it never holds twice.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; ObjectHead::HEAD_BYTES] {
        let mut out = [0u8; ObjectHead::HEAD_BYTES];
        out[0..8].copy_from_slice(&self.object_bytes.to_le_bytes());
        out[8..12].copy_from_slice(&self.chunks.to_le_bytes());
        out
    }

    /// Decode the head, refusing before either number is handed back.
    ///
    /// Two refusals, and the second is the interesting one: an object of no
    /// bytes made of some chunks, or an object of some bytes made of no chunks,
    /// is a contradiction readable without knowing anything about the chunker.
    /// Refusing it here is what keeps the arithmetic that walks a chunk list
    /// free of a case that cannot arise.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] for a count past [`CHUNKS_MAX`];
    /// [`refusal::MALFORMED`] for a head whose two numbers contradict.
    pub fn from_bytes(raw: &[u8; ObjectHead::HEAD_BYTES]) -> Result<Self, i32> {
        let object_bytes = long(raw, 0);
        let chunks = word(raw, 8);
        if chunks as usize > CHUNKS_MAX {
            return Err(refusal::UNKNOWN);
        }
        if (object_bytes == 0) != (chunks == 0) {
            return Err(refusal::MALFORMED);
        }
        Ok(Self { object_bytes, chunks })
    }
}

/// How many piece hashes one extent may name.
///
/// A bound for [`CHUNKS_MAX`]'s reason, with different arithmetic behind it:
/// the count is a length field a peer wrote, and a decoder with no bound
/// allocates whatever the device asks it to. A million pieces at RFC 0058's
/// `EXTENT_BYTES` = 1 MiB is an extent of one tebibyte, far past anything this
/// system keeps in one mutable object, and the piece list at that size is
/// 32 MiB — well past the one block RFC 0058's granularity argument was drawn
/// around. An extent larger than that wants a coarser piece or a second level
/// of indirection, and RFC 0058's third reversal condition is where that is
/// decided; it is not a constant edited here.
/// Unit: count of piece hashes.
pub const PIECES_MAX: usize = 1 << 20;

/// The head of an extent blob: how long the extent is, and how many
/// fixed-offset pieces follow.
///
/// # Why this is a second head and not [`ObjectHead`] under a different kind
///
/// Because the two counts mean different things and a reader has to be able to
/// tell which one it is holding. An object's children are boundaries the
/// *content* chose, so `chunks` is a fact about the bytes; an extent's children
/// are fixed-offset pieces, so `pieces` is `ceil(extent_bytes / EXTENT_BYTES)`
/// and is a fact about the writer's granularity. One type would make those
/// indistinguishable at the point a decoder has the least else to go on, and
/// RFC 0058's decision is precisely that these are two kinds rather than one
/// shape covering both badly. The two encodings are the same width on purpose:
/// nothing is bought by making them differ, and a reader comparing them should
/// find that what differs is the meaning.
///
/// # Why `EXTENT_BYTES` is not a field here
///
/// RFC 0058: the piece size is recoverable from the data. Every piece but the
/// last is exactly `EXTENT_BYTES`, and a piece is a blob whose [`Header`]
/// carries `content_bytes`, so a reader recovers the granularity from piece 0
/// rather than from a record field or a compiled-in constant. That is what lets
/// extents written at two granularities coexist and stay readable, which is
/// what makes RFC 0058's granularity reversal cheap. The constant is a
/// *writer's* constant and lives in `f_blob::extent`: not in this crate, and
/// not on the device beside the chunker's parameters, which are there because
/// they decide every object hash and a mount must refuse a device that
/// disagrees.
///
/// It carries no magic of its own, for [`ObjectHead`]'s reason: it is never
/// found on its own, and by the time these twelve bytes are read the blob's
/// content hash has been verified against the name the caller asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentHead {
    /// The extent's logical size — the bytes its pieces reconstruct.
    /// Unit: bytes.
    pub extent_bytes: u64,
    /// How many piece hashes follow this head, in order.
    /// Unit: count of piece hashes, at most [`PIECES_MAX`].
    pub pieces: u32,
}

impl ExtentHead {
    /// The encoded width of an extent's head, before the hashes that follow.
    /// Unit: bytes.
    pub const HEAD_BYTES: usize = 12;

    /// The encoded width of the whole extent blob's content: the head and the
    /// hashes after it.
    ///
    /// This is the number RFC 0058's granularity argument turns on — 128 pieces
    /// of 1 MiB is a 128 MiB extent whose list is 4096 bytes, exactly one 4 KiB
    /// block — so it is a method rather than a paragraph, and the arithmetic is
    /// checkable against the encoder that produces it.
    /// Unit: bytes.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        Self::HEAD_BYTES + self.pieces as usize * 32
    }

    /// Encode the head. The hashes are appended by the caller.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; ExtentHead::HEAD_BYTES] {
        let mut out = [0u8; ExtentHead::HEAD_BYTES];
        out[0..8].copy_from_slice(&self.extent_bytes.to_le_bytes());
        out[8..12].copy_from_slice(&self.pieces.to_le_bytes());
        out
    }

    /// Decode the head, refusing before either number is handed back.
    ///
    /// The contradiction refused is [`ObjectHead`]'s: an extent of no bytes
    /// made of some pieces, or an extent of some bytes made of none. What is
    /// deliberately *not* refused is a count that disagrees with a granularity,
    /// because this crate does not know the granularity and RFC 0058 says it
    /// must not — a decoder with `EXTENT_BYTES` compiled into it would refuse
    /// every extent written at a different one, which is exactly the
    /// coexistence that decision bought.
    ///
    /// # Errors
    ///
    /// [`refusal::UNKNOWN`] for a count past [`PIECES_MAX`];
    /// [`refusal::MALFORMED`] for a head whose two numbers contradict.
    pub fn from_bytes(raw: &[u8; ExtentHead::HEAD_BYTES]) -> Result<Self, i32> {
        let extent_bytes = long(raw, 0);
        let pieces = word(raw, 8);
        if pieces as usize > PIECES_MAX {
            return Err(refusal::UNKNOWN);
        }
        if (extent_bytes == 0) != (pieces == 0) {
            return Err(refusal::MALFORMED);
        }
        Ok(Self { extent_bytes, pieces })
    }
}

/// One publish, appended to a root zone.
///
/// # Why the check field is not decoration
///
/// Without it a record torn across a power cut — the magic landed, `root` is
/// half written — names a hash that never existed, and *not found* is
/// indistinguishable from *collected*. With it, a torn record is refused and
/// the mount falls to the previous generation, which is a rollback rather than
/// a machine naming a state it cannot produce. Verify before accept, RFC 0060;
/// the second half of that rule — every child of the generation node resolves —
/// is the mount's rather than this record's.
///
/// # Why there is no timestamp
///
/// [`RootRecord::generation`] is the order. A clock in a record would make the
/// identity of a publish a function of when it happened, which is exactly what
/// `E2-P06` compares two machines on. RFC 0004.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootRecord {
    /// Which publish this is, counting from the superblock.
    /// Unit: count of publishes since the superblock; the first publish is 1,
    /// and 0 is reserved so that a zeroed block is never a generation.
    pub generation: u64,
    /// The generation hash — what the fold produced, and what identifies this
    /// state of the machine.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub root: [u8; 32],
    /// The frame image's hash, carried separately so that a swap can tell
    /// whether the frame moved without unpacking anything. RFC 0012: a rollback
    /// is one generation swap, and a reboot only when this field changed.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub frame: [u8; 32],
    /// The hash of the boot module packed for this root when its generation was
    /// compiled — not the module this root was booted from. A root that arrives
    /// by swap is never booted, and a root with no module is a root no reboot
    /// can reach, so `f.root=<hex>` would have nothing to select. RFC 0012.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub module: [u8; 32],
    /// The previous root, or zero at the first publish.
    /// Unit: bytes, exactly 32 of them — a content address.
    pub previous: [u8; 32],
    /// The SHA-256 over every preceding field of this record.
    /// Unit: bytes, exactly 32 of them — a digest over this record's first
    /// [`RootRecord::CHECKED_BYTES`] bytes.
    pub check: [u8; 32],
}

impl RootRecord {
    /// The encoded width of a root record.
    /// Unit: bytes.
    pub const BYTES: usize = 176;

    /// How much of the record [`RootRecord::check`] covers: everything before
    /// it.
    /// Unit: bytes.
    pub const CHECKED_BYTES: usize = 144;

    /// The bytes the check is taken over — the whole record except the check.
    ///
    /// A writer hashes this, puts the digest in [`RootRecord::check`], and
    /// encodes. Splitting it out rather than computing the digest here is what
    /// keeps this crate free of a hash implementation: `abi/` is the wire and
    /// has no dependencies at all, and giving it one so that one constructor
    /// could be a line shorter would be paying an architectural price for a
    /// convenience. The cost is that a caller can encode a record whose check
    /// is wrong — which [`RootRecord::from_bytes`] refuses, so the mistake
    /// cannot survive a round trip.
    #[must_use]
    pub fn checked(&self) -> [u8; RootRecord::CHECKED_BYTES] {
        let mut out = [0u8; RootRecord::CHECKED_BYTES];
        out[0..4].copy_from_slice(&ROOT_MAGIC.to_le_bytes());
        out[4..8].copy_from_slice(&(RootRecord::BYTES as u32).to_le_bytes());
        out[8..16].copy_from_slice(&self.generation.to_le_bytes());
        out[16..48].copy_from_slice(&self.root);
        out[48..80].copy_from_slice(&self.frame);
        out[80..112].copy_from_slice(&self.module);
        out[112..144].copy_from_slice(&self.previous);
        out
    }

    /// Encode.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; RootRecord::BYTES] {
        let mut out = [0u8; RootRecord::BYTES];
        out[0..RootRecord::CHECKED_BYTES].copy_from_slice(&self.checked());
        out[RootRecord::CHECKED_BYTES..RootRecord::BYTES].copy_from_slice(&self.check);
        out
    }

    /// Decode, refusing before any field is handed back — the check included.
    ///
    /// `computed` is the SHA-256 the caller took over the record's first
    /// [`RootRecord::CHECKED_BYTES`] bytes. It is an argument rather than
    /// something this function works out for itself, for the reason
    /// [`RootRecord::checked`] gives: this crate has no hash, and acquiring one
    /// would be an architectural change made for a convenience. What the
    /// signature buys is that verify-before-accept stays *inside the
    /// constructor* — a caller cannot obtain a `RootRecord` without having
    /// hashed the bytes, so there is no order of operations in which a field is
    /// believed first.
    ///
    /// # Errors
    ///
    /// [`refusal::MALFORMED`] for the magic, the declared length, a zero
    /// generation, or a check that does not verify.
    pub fn from_bytes(raw: &[u8; RootRecord::BYTES], computed: &[u8; 32]) -> Result<Self, i32> {
        prologue(raw, ROOT_MAGIC, RootRecord::BYTES)?;
        if raw[RootRecord::CHECKED_BYTES..RootRecord::BYTES] != computed[..] {
            return Err(refusal::MALFORMED);
        }
        // Zero is reserved so that a zeroed block is never a generation. A
        // freshly reset zone reads as zeros, and a reader that accepted
        // generation zero would find one in every unwritten block of it.
        let generation = long(raw, 8);
        if generation == 0 {
            return Err(refusal::MALFORMED);
        }
        Ok(Self {
            generation,
            root: digest(raw, 16),
            frame: digest(raw, 48),
            module: digest(raw, 80),
            previous: digest(raw, 112),
            check: digest(raw, 144),
        })
    }
}

/// Believe a record's prologue: the magic it must carry, then the length it
/// must declare.
///
/// The order is [`header`]'s, on a record rather than a node and for the same
/// reason: the magic says *these bytes are a record of this kind at all*, and
/// until that holds the length field is not a length, it is four bytes of
/// something else.
fn prologue(raw: &[u8], magic: u32, bytes: usize) -> Result<(), i32> {
    if raw.len() < 8 || word(raw, 0) != magic || word(raw, 4) as usize != bytes {
        return Err(refusal::MALFORMED);
    }
    Ok(())
}

/// Two little-endian bytes at `at`.
fn short(raw: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([raw[at], raw[at + 1]])
}

/// Four little-endian bytes at `at`.
fn word(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
}

/// Eight little-endian bytes at `at`.
fn long(raw: &[u8], at: usize) -> u64 {
    let mut eight = [0u8; 8];
    eight.copy_from_slice(&raw[at..at + 8]);
    u64::from_le_bytes(eight)
}

/// Thirty-two bytes at `at`.
fn digest(raw: &[u8], at: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw[at..at + 32]);
    out
}

// Every record's encoded width is the sum of its field widths, asserted at
// compile time, for the reason the node assertions above exist: an unhashed
// byte inside a hashed structure is a place two writers can differ while naming
// the same thing, and a field added without its width is how one appears.
const _: () = assert!(Superblock::BYTES == 4 + 4 + 4 * 9 + 8 * 3);
const _: () = assert!(Header::BYTES == 4 + 4 + 2 + 2 + 8 + 32);
const _: () = assert!(ObjectHead::HEAD_BYTES == 8 + 4);
const _: () = assert!(ExtentHead::HEAD_BYTES == 8 + 4);
// RFC 0058's granularity argument, asserted against the encoder rather than
// left in the entry's prose: the *hash list* of a 128 MiB extent at
// `EXTENT_BYTES` = 1 MiB is 128 x 32 = 4096 bytes, exactly one 4 KiB block, and
// that coincidence is the whole reason the granularity is 1 MiB and not 256 KiB
// or 4 MiB. The record is the head on top of that, so 4108 — the entry's number
// is the list and not the record, and the twelve bytes push the record over a
// block boundary. That is stated here rather than rounded away: it costs one
// extra block per snapshot at that size and it does not move the argument,
// which is about how the list scales against the copy. These lines go red if
// the head widens or a hash stops being 32 bytes, which are the two ways the
// argument could quietly stop holding.
const _: () = assert!(ExtentHead { extent_bytes: 128 << 20, pieces: 128 }.bytes() == 12 + 4096);
const _: () = assert!(RootRecord::CHECKED_BYTES == 4 + 4 + 8 + 32 * 4);
const _: () = assert!(RootRecord::BYTES == RootRecord::CHECKED_BYTES + 32);
// And no record is wider than the smallest block a device is likely to offer,
// because a record spanning two blocks could not have its magic believed until
// both had been read. Five hundred and twelve is the floor this store formats
// against, and a record that outgrew it would need a reader that reads twice.
const _: () = assert!(Superblock::BYTES <= 512);
const _: () = assert!(Header::BYTES <= 512);
const _: () = assert!(RootRecord::BYTES <= 512);
#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> [u8; NAME_MAX] {
        let mut out = [0u8; NAME_MAX];
        out[..text.len()].copy_from_slice(text.as_bytes());
        out
    }

    /// The golden bytes, written out rather than compared against another
    /// encoder: the point of a golden test is that it fails when the layout
    /// moves, and a test that encodes and decodes with the same code survives
    /// any layout change at all.
    #[test]
    fn a_leaf_encodes_to_the_bytes_the_format_says() {
        let leaf = Leaf { kind: node::COMPONENT, hash: [0xAB; 32], name: name("store") };
        let raw = leaf.to_bytes();

        assert_eq!(&raw[0..4], &[0x46, 0x5F, 0x4E, 0x44], "magic, little-endian");
        assert_eq!(&raw[4..6], &[2, 0], "kind");
        assert_eq!(&raw[6..8], &[72, 0], "length");
        assert_eq!(&raw[8..40], &[0xAB; 32]);
        assert_eq!(&raw[40..45], b"store");
        assert!(raw[45..72].iter().all(|b| *b == 0), "the name is NUL-padded");
        assert_eq!(Leaf::from_bytes(&raw), Ok(leaf));
    }

    #[test]
    fn a_topology_head_encodes_its_own_length() {
        let head = Topology { members: 4, routes: 2 };
        let raw = head.to_bytes();
        assert_eq!(&raw[4..6], &[3, 0], "kind");
        assert_eq!(u16::from_le_bytes([raw[6], raw[7]]) as usize, 12 + 2 * 36);
        assert_eq!(Topology::from_bytes(&raw), Ok(head));
    }

    #[test]
    fn a_generation_states_the_width_of_the_node_that_is_hashed() {
        let raw = Generation::to_bytes();
        assert_eq!(&raw[4..6], &[4, 0]);
        assert_eq!(u16::from_le_bytes([raw[6], raw[7]]) as usize, 72);
        assert_eq!(Generation::from_bytes(&raw), Ok(Generation));
    }

    #[test]
    fn a_route_round_trips_through_its_own_encoding() {
        let route = Route { component: 0, source: 1, capability: name("block") };
        assert_eq!(Route::from_bytes(&route.to_bytes()), route);
    }

    /// Fail closed, R04. Every one of these is a byte a device could have
    /// written.
    #[test]
    fn a_node_this_build_does_not_know_is_refused_and_never_skipped() {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let unknown = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);

        let good = Leaf { kind: node::BYTES, hash: [0; 32], name: name("kernel") }.to_bytes();

        let mut bad_magic = good;
        bad_magic[0] ^= 0xFF;
        assert_eq!(Leaf::from_bytes(&bad_magic), Err(malformed));

        let mut bad_length = good;
        bad_length[6] = 71;
        assert_eq!(Leaf::from_bytes(&bad_length), Err(malformed));

        // A zeroed block, which is the whole reason zero is not a kind.
        assert_eq!(Leaf::from_bytes(&[0u8; Leaf::BYTES]), Err(malformed));

        // A fifth kind, arriving without the RFC.
        let mut fifth = good;
        fifth[4] = 5;
        assert_eq!(Leaf::from_bytes(&fifth), Err(unknown));

        // A known kind that is not a leaf, at a leaf's offset.
        let mut wrong_kind = good;
        wrong_kind[4] = node::TOPOLOGY as u8;
        assert_eq!(Leaf::from_bytes(&wrong_kind), Err(unknown));
    }

    #[test]
    fn a_topology_whose_length_and_counts_disagree_is_refused() {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let unknown = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);

        let mut raw = Topology { members: 4, routes: 2 }.to_bytes();
        raw[10] = 3; // three routes, under a length that says two
        assert_eq!(Topology::from_bytes(&raw), Err(malformed));

        let mut past_bound = Topology { members: 4, routes: 2 }.to_bytes();
        past_bound[8..10].copy_from_slice(&(MEMBERS_MAX as u16 + 1).to_le_bytes());
        assert_eq!(Topology::from_bytes(&past_bound), Err(unknown));
    }

    /// A superblock this build wrote, byte by byte.
    ///
    /// Written out rather than compared against the encoder, for the reason the
    /// leaf's golden test gives: a test that encodes and decodes with the same
    /// code survives any layout change at all, and this layout is what every
    /// device ever formatted will be read with.
    fn a_superblock() -> Superblock {
        Superblock {
            schema: SCHEMA,
            block_bytes: 512,
            zones: 64,
            root_zone_a: 1,
            root_zone_b: 2,
            chunk_min_bytes: 16 * 1024,
            chunk_target_bytes: 64 * 1024,
            chunk_max_bytes: 256 * 1024,
            mask_bits: 16,
            zone_bytes: 1 << 20,
            gear_label: 0x0102_0304_0506_0708,
            mask_label: 0x1112_1314_1516_1718,
        }
    }

    #[test]
    fn a_superblock_encodes_to_the_bytes_the_format_says() {
        let raw = a_superblock().to_bytes();

        assert_eq!(&raw[0..4], b"F_SB", "magic, little-endian");
        assert_eq!(&raw[4..8], &[68, 0, 0, 0], "the record's own declared length");
        assert_eq!(&raw[8..12], &[1, 0, 0, 0], "schema");
        assert_eq!(&raw[12..16], &[0, 2, 0, 0], "block_bytes = 512");
        assert_eq!(&raw[16..20], &[64, 0, 0, 0], "zones");
        assert_eq!(&raw[20..24], &[1, 0, 0, 0], "root_zone_a");
        assert_eq!(&raw[24..28], &[2, 0, 0, 0], "root_zone_b");
        assert_eq!(&raw[28..32], &[0, 0x40, 0, 0], "chunk_min_bytes = 16 KiB");
        assert_eq!(&raw[32..36], &[0, 0, 1, 0], "chunk_target_bytes = 64 KiB");
        assert_eq!(&raw[36..40], &[0, 0, 4, 0], "chunk_max_bytes = 256 KiB");
        assert_eq!(&raw[40..44], &[16, 0, 0, 0], "mask_bits");
        assert_eq!(&raw[44..52], &[0, 0, 0x10, 0, 0, 0, 0, 0], "zone_bytes = 1 MiB");
        assert_eq!(&raw[52..60], &[8, 7, 6, 5, 4, 3, 2, 1], "gear_label");
        assert_eq!(&raw[60..68], &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11], "mask_label");
        assert_eq!(Superblock::from_bytes(&raw), Ok(a_superblock()));
    }

    /// Fail closed, R04, on the record a mount reads first and trusts most.
    #[test]
    fn a_superblock_this_build_cannot_read_is_refused() {
        let good = a_superblock().to_bytes();

        let mut bad_magic = good;
        bad_magic[1] ^= 0xFF;
        assert_eq!(Superblock::from_bytes(&bad_magic), Err(refusal::MALFORMED));

        let mut bad_length = good;
        bad_length[4] = 67;
        assert_eq!(Superblock::from_bytes(&bad_length), Err(refusal::MALFORMED));

        // A zeroed block, which is what an unformatted device offers.
        assert_eq!(Superblock::from_bytes(&[0u8; Superblock::BYTES]), Err(refusal::MALFORMED));

        // A format from the future, read with this build's field offsets: the
        // fields would decode perfectly and mean something else.
        let mut next_schema = good;
        next_schema[8] = 2;
        assert_eq!(Superblock::from_bytes(&next_schema), Err(refusal::UNKNOWN));

        // Every address in the store is a multiple of this number.
        let mut no_blocks = a_superblock();
        no_blocks.block_bytes = 0;
        assert_eq!(Superblock::from_bytes(&no_blocks.to_bytes()), Err(refusal::MALFORMED));
    }

    #[test]
    fn a_blob_header_encodes_to_the_bytes_the_format_says() {
        let header = Header { kind: kind::CHUNK, flags: 0, content_bytes: 4096, hash: [0xCD; 32] };
        let raw = header.to_bytes();

        assert_eq!(&raw[0..4], b"F_BH", "magic, little-endian");
        assert_eq!(&raw[4..8], &[52, 0, 0, 0], "the header's own width, never the record's");
        assert_eq!(&raw[8..10], &[1, 0], "kind");
        assert_eq!(&raw[10..12], &[0, 0], "flags, and there are none yet");
        assert_eq!(&raw[12..20], &[0, 0x10, 0, 0, 0, 0, 0, 0], "content_bytes = 4096");
        assert_eq!(&raw[20..52], &[0xCD; 32], "hash");
        assert_eq!(Header::from_bytes(&raw), Ok(header));
    }

    /// Fail closed, R04. Every one of these is a block a device could return.
    #[test]
    fn a_blob_header_this_build_does_not_understand_is_refused() {
        let good =
            Header { kind: kind::OBJECT, flags: 0, content_bytes: 1, hash: [0; 32] }.to_bytes();

        let mut bad_magic = good;
        bad_magic[0] ^= 0xFF;
        assert_eq!(Header::from_bytes(&bad_magic), Err(refusal::MALFORMED));

        let mut bad_length = good;
        bad_length[4] = 51;
        assert_eq!(Header::from_bytes(&bad_length), Err(refusal::MALFORMED));

        // A zeroed block. The whole reason zero is not a kind and not a magic.
        assert_eq!(Header::from_bytes(&[0u8; Header::BYTES]), Err(refusal::MALFORMED));

        // A fifth kind, arriving without the RFC that would have added it.
        let mut fifth = good;
        fifth[8] = 5;
        assert_eq!(Header::from_bytes(&fifth), Err(refusal::UNKNOWN));

        // A flag bit this build does not define: refused, never dropped.
        let mut flagged = good;
        flagged[10] = 1;
        assert_eq!(Header::from_bytes(&flagged), Err(refusal::UNKNOWN));

        // A magic from the record one file over, at a blob's offset. The whole
        // argument for three magics rather than one.
        let mut wrong_record = good;
        wrong_record[0..4].copy_from_slice(&ROOT_MAGIC.to_le_bytes());
        assert_eq!(Header::from_bytes(&wrong_record), Err(refusal::MALFORMED));
    }

    #[test]
    fn an_object_head_round_trips_and_refuses_a_contradiction() {
        let head = ObjectHead { object_bytes: 1_048_576, chunks: 17 };
        let raw = head.to_bytes();
        assert_eq!(&raw[0..8], &[0, 0, 0x10, 0, 0, 0, 0, 0], "object_bytes = 1 MiB");
        assert_eq!(&raw[8..12], &[17, 0, 0, 0], "chunks");
        assert_eq!(ObjectHead::from_bytes(&raw), Ok(head));
        assert_eq!(head.bytes(), ObjectHead::HEAD_BYTES + 17 * 32);

        // The empty object: no bytes and no chunks, which is not a
        // contradiction and folds to a hash like anything else.
        let empty = ObjectHead { object_bytes: 0, chunks: 0 };
        assert_eq!(ObjectHead::from_bytes(&empty.to_bytes()), Ok(empty));

        let bytes_without_chunks = ObjectHead { object_bytes: 4096, chunks: 0 }.to_bytes();
        assert_eq!(ObjectHead::from_bytes(&bytes_without_chunks), Err(refusal::MALFORMED));

        let chunks_without_bytes = ObjectHead { object_bytes: 0, chunks: 3 }.to_bytes();
        assert_eq!(ObjectHead::from_bytes(&chunks_without_bytes), Err(refusal::MALFORMED));

        let past_bound = ObjectHead { object_bytes: 1, chunks: CHUNKS_MAX as u32 + 1 }.to_bytes();
        assert_eq!(ObjectHead::from_bytes(&past_bound), Err(refusal::UNKNOWN));
    }

    /// The extent head's twin of the test above, and the two are kept side by
    /// side rather than folded into one parameterised test for the reason the
    /// types are two types: what a reader has to be able to check is that a
    /// twelve-byte head decoded as the wrong one of these would be a different
    /// *meaning* with the same bytes, and a shared test would be the place that
    /// stopped being visible.
    #[test]
    fn an_extent_head_round_trips_and_refuses_a_contradiction() {
        let head = ExtentHead { extent_bytes: 8 * 1_048_576, pieces: 8 };
        let raw = head.to_bytes();
        assert_eq!(&raw[0..8], &[0, 0, 0x80, 0, 0, 0, 0, 0], "extent_bytes = 8 MiB");
        assert_eq!(&raw[8..12], &[8, 0, 0, 0], "pieces");
        assert_eq!(ExtentHead::from_bytes(&raw), Ok(head));
        assert_eq!(head.bytes(), ExtentHead::HEAD_BYTES + 8 * 32);

        let empty = ExtentHead { extent_bytes: 0, pieces: 0 };
        assert_eq!(ExtentHead::from_bytes(&empty.to_bytes()), Ok(empty));

        let bytes_without_pieces = ExtentHead { extent_bytes: 4096, pieces: 0 }.to_bytes();
        assert_eq!(ExtentHead::from_bytes(&bytes_without_pieces), Err(refusal::MALFORMED));

        let pieces_without_bytes = ExtentHead { extent_bytes: 0, pieces: 3 }.to_bytes();
        assert_eq!(ExtentHead::from_bytes(&pieces_without_bytes), Err(refusal::MALFORMED));

        let past_bound = ExtentHead { extent_bytes: 1, pieces: PIECES_MAX as u32 + 1 }.to_bytes();
        assert_eq!(ExtentHead::from_bytes(&past_bound), Err(refusal::UNKNOWN));

        // A count that disagrees with *a* granularity is accepted, and this is
        // the assertion that says so rather than a comment claiming it: one
        // piece for eight megabytes is what an extent written at
        // `EXTENT_BYTES` = 8 MiB looks like, and RFC 0058 requires this crate to
        // read it. A decoder that refused here would refuse every extent
        // written at a granularity other than the one it was compiled with,
        // which is exactly the coexistence that entry bought.
        let coarser = ExtentHead { extent_bytes: 8 * 1_048_576, pieces: 1 };
        assert_eq!(ExtentHead::from_bytes(&coarser.to_bytes()), Ok(coarser));
    }

    /// The check is a digest this crate cannot compute, so the tests here
    /// stand a fixed array in for one.
    ///
    /// That is not a weaker test than hashing would be: what this file owns is
    /// the *rule* — the record is not believed unless the bytes at the check's
    /// offset equal the digest the caller took — and the rule is exercised in
    /// both directions below. That the digest is a real SHA-256 over
    /// [`RootRecord::CHECKED_BYTES`] bytes is `blob/src/store.rs`'s to
    /// demonstrate, because that is the crate with a hash in it.
    fn a_root(check: [u8; 32]) -> RootRecord {
        RootRecord {
            generation: 7,
            root: [0x11; 32],
            frame: [0x22; 32],
            module: [0x33; 32],
            previous: [0x44; 32],
            check,
        }
    }

    #[test]
    fn a_root_record_encodes_to_the_bytes_the_format_says() {
        let record = a_root([0x55; 32]);
        let raw = record.to_bytes();

        assert_eq!(&raw[0..4], b"F_RR", "magic, little-endian");
        assert_eq!(&raw[4..8], &[176, 0, 0, 0], "the record's own declared length");
        assert_eq!(&raw[8..16], &[7, 0, 0, 0, 0, 0, 0, 0], "generation");
        assert_eq!(&raw[16..48], &[0x11; 32], "root");
        assert_eq!(&raw[48..80], &[0x22; 32], "frame");
        assert_eq!(&raw[80..112], &[0x33; 32], "module");
        assert_eq!(&raw[112..144], &[0x44; 32], "previous");
        assert_eq!(&raw[144..176], &[0x55; 32], "check");

        // What the check covers is every preceding field and nothing else.
        assert_eq!(&record.checked()[..], &raw[..RootRecord::CHECKED_BYTES]);
        assert_eq!(RootRecord::from_bytes(&raw, &[0x55; 32]), Ok(record));
    }

    /// Verify before accept, in the smallest form the rule has: no field of
    /// this record is returned until the check the caller computed is the check
    /// the record carries.
    #[test]
    fn a_root_record_is_not_believed_before_its_check_verifies() {
        let raw = a_root([0x55; 32]).to_bytes();

        // The digest the caller computed is not the one the record carries:
        // either the record is torn or the bytes it covers moved. Both are the
        // same refusal, because both mean nobody finished writing this.
        assert_eq!(RootRecord::from_bytes(&raw, &[0x56; 32]), Err(refusal::MALFORMED));

        // A record torn mid-`root`: the magic landed, the rest did not follow,
        // so the check field is whatever the zone held — zeros here — while the
        // digest a caller takes over the surviving prefix is not that. This is
        // the case the field exists for, and *not found* is what a reader
        // without it would report for the hash this record half names.
        let mut torn = raw;
        torn[20..176].fill(0);
        assert_eq!(RootRecord::from_bytes(&torn, &[0xEE; 32]), Err(refusal::MALFORMED));

        // A zeroed block in a freshly reset zone is not generation zero.
        let zeroed = a_root([0x55; 32]);
        let mut ungenerated = zeroed;
        ungenerated.generation = 0;
        let raw = ungenerated.to_bytes();
        assert_eq!(RootRecord::from_bytes(&raw, &[0x55; 32]), Err(refusal::MALFORMED));

        let mut bad_magic = zeroed.to_bytes();
        bad_magic[2] ^= 0xFF;
        assert_eq!(RootRecord::from_bytes(&bad_magic, &[0x55; 32]), Err(refusal::MALFORMED));
    }

    /// The four blob kinds are the vocabulary, and zero is not one of them.
    #[test]
    fn zero_is_not_a_blob_kind_and_a_fifth_is_not_either() {
        assert!(kind::known(kind::CHUNK));
        assert!(kind::known(kind::OBJECT));
        assert!(kind::known(kind::EXTENT));
        assert!(kind::known(kind::GENERATION));
        assert!(!kind::known(0), "a zeroed block must not decode as a blob");
        assert!(!kind::known(5), "a fifth kind arrives with an RFC, not with a device");
        assert_eq!(kind::label(5), "unknown");
    }

    /// The refusals are distinct values, which is the whole of naming them.
    ///
    /// A caller distinguishes *this device returned the wrong bytes* from *this
    /// record is not a record*, and if two of these packed to one integer the
    /// distinction would exist only in the doc comments.
    #[test]
    fn the_store_refusals_are_six_distinct_values() {
        let all = [
            refusal::MALFORMED,
            refusal::UNKNOWN,
            refusal::ADDRESS,
            refusal::FULL,
            refusal::SHORT_BUFFER,
            refusal::CONTENT,
        ];
        for (index, one) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(one, other, "two store refusals pack to one integer");
            }
            assert!(*one < 0, "a refusal is a negative result, RFC 0010");
        }
        assert_eq!(error::unpack(refusal::CONTENT), Some((error::DEVICE, 1)));
    }
}
