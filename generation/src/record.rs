// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The checker over the node kinds `f_abi::store` defines.
//!
//! # What a tree is, laid out
//!
//! One buffer, walked from the front, with every offset a function of a count
//! that has already been believed. There are no offsets *in* the encoding and
//! nothing points anywhere:
//!
//! ```text
//! 0                      the generation node, a header and nothing else
//! 8                      the frame: a `bytes` leaf
//! 80                     the topology head, then its routes
//! 80 + topology.bytes()  the members, in canonical order, one leaf each
//! ```
//!
//! The depth is three and the encoding cannot express a fourth, which is the
//! property `fold.rs` spends its heap argument on.
//!
//! # Why the checker is here and the codec is not
//!
//! Because this crate holding a codec would make a fifth node kind a patch to
//! this crate, and a fifth node kind is meant to cost an RFC. Everything below
//! decodes through `f_abi::store` and refuses on what a *tree* can be wrong
//! about that a *node* cannot: a name outside its alphabet, a duplicate, a
//! dangling index, and an order.
//!
//! # Why order is a refusal and not a normalisation
//!
//! Sorting on the way in would be the friendlier behaviour and it is refused,
//! because the whole claim of this crate is that the same expression yields the
//! same root: a compiler that quietly reorders makes *two different sources*
//! yield one root, which is the same property read backwards and is not the one
//! anybody wanted. An author who learns their file was rewritten by reading a
//! hash has learned it too late.

use f_abi::manifest::NAME_MAX;
use f_abi::store::{Generation, Leaf, MEMBERS_MAX, Route, Topology, node};

/// Where the frame's leaf begins.
/// Unit: bytes from the start of the tree.
const FRAME_AT: usize = Generation::BYTES;

/// Where the topology's head begins.
/// Unit: bytes from the start of the tree.
const TOPOLOGY_AT: usize = FRAME_AT + Leaf::BYTES;

/// Why a tree was not believed.
///
/// One variant per thing a reader can be wrong about, rather than one variant
/// for *malformed*, because the author of a `generation.toml` is the reader who
/// sees this and *the file is wrong* is not a sentence anybody can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The buffer is not the length its own counts say it is.
    Length,
    /// A node's header was refused by its codec. Carries the packed
    /// `f_abi::error` it refused with, so a caller that reports errors as
    /// completions can pass it straight on.
    Node(i32),
    /// A node of the right shape in the wrong place: a `component` where the
    /// frame belongs, or a `bytes` leaf among the members.
    Kind,
    /// A name is empty, too long, outside `[a-z0-9-]`, carries an edge hyphen,
    /// or has something other than NUL in its padding.
    Name,
    /// Two components share a name.
    DuplicateName,
    /// Two routes share a (component, capability).
    DuplicateRoute,
    /// A route names a component index this topology does not have.
    Dangling,
    /// The components are not sorted bytewise on the zero-padded name.
    Order,
    /// The routes are not sorted on (component index, capability name).
    RouteOrder,
}

impl Refusal {
    /// A line for a person holding the source this tree was compiled from.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Length => "the record tree is not the length its own counts say it is",
            Self::Node(_) => "a node's magic, kind or declared length is not one this build knows",
            Self::Kind => "a node of the right shape is in the wrong place in the tree",
            Self::Name => "a name is empty, too long, outside `[a-z0-9-]`, or badly padded",
            Self::DuplicateName => "two components have the same name",
            Self::DuplicateRoute => "two routes have the same component and capability",
            Self::Dangling => "a route names a component this topology does not have",
            Self::Order => "the components are not in canonical order",
            Self::RouteOrder => "the routes are not in canonical order",
        }
    }
}

/// A checked record tree, borrowing the bytes it was checked over.
///
/// Holding a `Tree` is the statement that every refusal below has already been
/// ruled out, which is what lets [`crate::fold::root`] walk it with no error
/// path at all — the only reading of *validated* worth having, and the same one
/// `f_abi::manifest::Record::read` uses one level down.
#[derive(Clone, Copy, Debug)]
pub struct Tree<'a> {
    bytes: &'a [u8],
    head: Topology,
}

impl<'a> Tree<'a> {
    /// Check a record tree, or say what is wrong with it.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]. Fail closed: nothing here is skipped, clamped or
    /// reordered.
    pub fn check(bytes: &'a [u8]) -> Result<Self, Refusal> {
        if bytes.len() < TOPOLOGY_AT + Topology::HEAD_BYTES {
            return Err(Refusal::Length);
        }

        // The root, then the frame, then the topology's head — each believed
        // before the next one's offset is computed from it.
        let mut root = [0u8; Generation::BYTES];
        root.copy_from_slice(&bytes[..Generation::BYTES]);
        Generation::from_bytes(&root).map_err(Refusal::Node)?;

        let frame = leaf_at(bytes, FRAME_AT)?;
        if frame.kind != node::BYTES {
            return Err(Refusal::Kind);
        }
        if !named(&frame.name) {
            return Err(Refusal::Name);
        }

        let mut raw = [0u8; Topology::HEAD_BYTES];
        raw.copy_from_slice(&bytes[TOPOLOGY_AT..TOPOLOGY_AT + Topology::HEAD_BYTES]);
        let head = Topology::from_bytes(&raw).map_err(Refusal::Node)?;

        let members_at = TOPOLOGY_AT + head.bytes();
        let end =
            members_at.checked_add(head.members as usize * Leaf::BYTES).ok_or(Refusal::Length)?;
        // Exactly, not at least. A trailing byte is a byte the fold does not
        // cover, and a byte the fold does not cover is a place two trees differ
        // while naming one generation.
        if bytes.len() != end {
            return Err(Refusal::Length);
        }

        let tree = Self { bytes, head };

        // The members: each a `component` leaf, each named, and strictly
        // increasing on the padded name — which is one comparison that refuses
        // both a duplicate and a misorder, and tells them apart.
        let mut previous: Option<[u8; NAME_MAX]> = None;
        for index in 0..head.members {
            let leaf = leaf_at(bytes, members_at + index as usize * Leaf::BYTES)?;
            if leaf.kind != node::COMPONENT {
                return Err(Refusal::Kind);
            }
            if !named(&leaf.name) {
                return Err(Refusal::Name);
            }
            if let Some(before) = previous {
                match before.cmp(&leaf.name) {
                    core::cmp::Ordering::Less => {}
                    core::cmp::Ordering::Equal => return Err(Refusal::DuplicateName),
                    core::cmp::Ordering::Greater => return Err(Refusal::Order),
                }
            }
            previous = Some(leaf.name);
        }

        // The routes: no dangling index, named capabilities, and strictly
        // increasing on (component, capability). `source` is checked as a bound
        // and not as an order, because a route is identified by who needs the
        // capability and by which capability — a second route to a second source
        // under one name is the duplicate, not a distinct route.
        let mut before: Option<(u16, [u8; NAME_MAX])> = None;
        for index in 0..head.routes {
            let route = tree.route(index).ok_or(Refusal::Length)?;
            if route.component >= head.members || route.source >= head.members {
                return Err(Refusal::Dangling);
            }
            if !named(&route.capability) {
                return Err(Refusal::Name);
            }
            let key = (route.component, route.capability);
            if let Some(previous) = before {
                match previous.cmp(&key) {
                    core::cmp::Ordering::Less => {}
                    core::cmp::Ordering::Equal => return Err(Refusal::DuplicateRoute),
                    core::cmp::Ordering::Greater => return Err(Refusal::RouteOrder),
                }
            }
            before = Some(key);
        }

        Ok(tree)
    }

    /// The bytes this tree was checked over.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The frame's leaf: a `bytes` leaf naming the frame image's content
    /// address, and the first of the generation node's two children.
    #[must_use]
    pub fn frame(&self) -> Leaf {
        self.leaf(FRAME_AT)
    }

    /// The topology's counts.
    #[must_use]
    pub const fn head(&self) -> Topology {
        self.head
    }

    /// How many components this topology names.
    /// Unit: components.
    #[must_use]
    pub const fn members(&self) -> u16 {
        self.head.members
    }

    /// How many routes it declares.
    /// Unit: routes.
    #[must_use]
    pub const fn routes(&self) -> u16 {
        self.head.routes
    }

    /// One member, in canonical order.
    #[must_use]
    pub fn member(&self, index: u16) -> Option<Leaf> {
        if index >= self.head.members {
            return None;
        }
        let at = TOPOLOGY_AT + self.head.bytes() + index as usize * Leaf::BYTES;
        Some(self.leaf(at))
    }

    /// One route, in canonical order.
    #[must_use]
    pub fn route(&self, index: u16) -> Option<Route> {
        if index >= self.head.routes {
            return None;
        }
        let at = TOPOLOGY_AT + Topology::HEAD_BYTES + index as usize * Route::BYTES;
        let mut raw = [0u8; Route::BYTES];
        raw.copy_from_slice(self.bytes.get(at..at + Route::BYTES)?);
        Some(Route::from_bytes(&raw))
    }

    /// A leaf at an offset a checked count produced.
    ///
    /// Infallible because the only caller is a `Tree`, and a `Tree` exists only
    /// where [`Tree::check`] already decoded every leaf once. The `unwrap_or`
    /// below is what keeps that fact from needing `unsafe` or a panic, and it
    /// fails towards a leaf that no fold will match rather than towards a crash.
    fn leaf(&self, at: usize) -> Leaf {
        let mut raw = [0u8; Leaf::BYTES];
        if let Some(slice) = self.bytes.get(at..at + Leaf::BYTES) {
            raw.copy_from_slice(slice);
        }
        Leaf::from_bytes(&raw).unwrap_or(Leaf { kind: 0, hash: [0; 32], name: [0; NAME_MAX] })
    }
}

/// Decode one leaf, or say why not.
fn leaf_at(bytes: &[u8], at: usize) -> Result<Leaf, Refusal> {
    let mut raw = [0u8; Leaf::BYTES];
    let slice = bytes.get(at..at + Leaf::BYTES).ok_or(Refusal::Length)?;
    raw.copy_from_slice(slice);
    Leaf::from_bytes(&raw).map_err(Refusal::Node)
}

/// Is this NUL-padded field a name?
///
/// `[a-z0-9-]`, non-empty, no edge hyphen, and **nothing but NUL after the
/// name**. The last clause is the one worth stating: the padding is hashed
/// because the field is fixed-width, so a byte hidden in it is a byte that
/// changes a root without changing anything a person can read. `docs/manifest.md`
/// bounds a name this way already, and `f_abi::manifest::NAME_MAX` is where the
/// bound is written down.
#[must_use]
pub fn named(padded: &[u8; NAME_MAX]) -> bool {
    let end = padded.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
    let Some(name) = padded.get(..end) else { return false };
    if name.is_empty() {
        return false;
    }
    if !padded[end..].iter().all(|b| *b == 0) {
        return false;
    }
    if name.first() == Some(&b'-') || name.last() == Some(&b'-') {
        return false;
    }
    name.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

// The bound the fold's *no explicit stack* argument rests on: a topology's
// member count fits the loop `fold.rs` writes, and the tree's depth is three
// because the encoding has no node that can hold a node.
const _: () = assert!(MEMBERS_MAX <= u16::MAX as usize);
