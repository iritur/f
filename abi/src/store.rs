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
}
