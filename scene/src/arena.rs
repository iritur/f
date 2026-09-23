// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The retained graph itself: one fixed region of slots, and the edits that
//! move nodes around inside it.
//!
//! Section 07 of `docs/design/ring-scene-boot.html` puts the scene on the
//! system's side of the boundary and sends the client's differences across it.
//! `abi/src/scene.rs` is what a difference looks like on the wire and
//! [`crate::kind`] is what one means once it has been believed. This is the
//! thing they are about: the tree that is still there between frames, the tree
//! a delta edits, and the tree a commit publishes.
//!
//! # Why the bound comes first
//!
//! Everything below is arranged around one decision, which is that the graph
//! has a maximum and cannot grow past it. [`NODES_MAX`] is that maximum and
//! [`Refusal::Capacity`] is what exceeding it costs.
//!
//! The argument is not that allocation is slow. It is that **a renderer which
//! can allocate under load has a worst case nobody measures**. A compositor is
//! the one component in this system whose failure is the whole machine going
//! dark, and every one of its inputs is chosen by a client. Give it an
//! allocator and the client chooses how much memory it uses; give it a growing
//! `Vec` and the client chooses when it reallocates, which is to say when a
//! frame misses. Neither of those shows up in a test, because a test submits a
//! scene somebody thought of. The bound makes the worst case a number written
//! down in this file, and makes exceeding it an event with a name that reaches
//! the client that caused it.
//!
//! The price is real and is not hidden: **an application whose scene genuinely
//! needs more nodes than [`NODES_MAX`] cannot draw it, and has to send the part
//! of it that is on screen.** That is the same trade `interface/src/canvas.rs`
//! makes with `PLACED_MAX` one layer up, for the same reason and with the same
//! reversal condition — the first measured scene that needs more, at which
//! point the number moves and the structure does not.
//!
//! # One region, and what makes it one
//!
//! *One fixed allocation* is a claim that has to survive somebody editing this
//! file, so it is not left to prose:
//!
//! - **There is no allocator to reach for.** The crate is `#![no_std]` and
//!   pulls in no `alloc`, so `Vec`, `Box` and `BTreeMap` are not merely
//!   discouraged here, they are not nameable.
//! - **The free list lives in the slots it tracks.** A released slot is pushed
//!   onto the arena's free list through its own `next` field — the same word
//!   that holds its next sibling while it is occupied. A slot is either in the
//!   tree or in the free list, never both, so the two structures cannot want
//!   the same word at the same time and no second array is needed to say which
//!   slots are spare.
//! - **Tearing down a subtree needs no stack.** [`Arena::remove`] threads its
//!   work list through the `next` field of the nodes it is about to release, so
//!   an arbitrarily deep subtree is torn down without recursion and without a
//!   scratch array proportional to its depth. This is the clause of the exit
//!   that is easiest to fail by accident: a recursive teardown is one fixed
//!   allocation plus however much stack the client's tree depth asks for, and
//!   the client chooses the depth.
//! - **A second region would change the type's size**, and the const assertion
//!   at the bottom of this file fails when anything but the slots grows. It is
//!   a statement the compiler checks rather than a habit a reader is asked to
//!   keep.
//!
//! [`Arena`] is neither `Clone` nor `Copy`, and that is structural rather than
//! an omission: it holds [`Created`] tokens, which [`crate::kind`] deliberately
//! refuses to make copyable, so there is no expression anywhere that turns one
//! arena into two. A caller holds it by reference or it holds it in place.
//!
//! # Where the nodes come from
//!
//! Every occupied slot holds a [`Created`], and the only function in this
//! workspace that produces one is [`Change::of`], which wants a whole [`Delta`]
//! — so *this node exists because a delta said so* is a property of the types
//! rather than of this file's discipline. [`crate::kind`] hands two obligations
//! down to exactly here, and both are discharged below:
//!
//! - *"a `Created` dropped on the floor is a node this module has forgotten …
//!   it is the arena's to catch, where the live slots are counted against the
//!   creations that filled them."* The census is a `ByKind<u16>` maintained by
//!   the same two operations that fill and empty slots, and
//!   `the_census_and_the_slots_agree` walks the array to check the two
//!   independently-maintained answers against each other.
//! - *"What the caller supplies is ancestry … The arena does [hold the tree],
//!   and that is where the check belongs."* [`Arena::remove`] never asks a
//!   caller for ancestry: it walks the tree it holds, and each node it reaches
//!   is retired through `removed_by` if the delta named it and `removed_under`
//!   if it was underneath.
//!
//! `abi/src/scene.rs` hands one down too — *"a cycle spread over two entries is
//! the graph's to catch, and `E3-B01c` is where that lives"* — and
//! [`Refusal::Cycle`] is it.
//!
//! # A create of a node that already exists is a move
//!
//! There is no `MoveNode` opcode, and `reconcile`'s *the one sentence the six
//! opcodes can form that means move* is the argument for the reading this
//! module implements: re-issuing `CreateNode` for a live node re-hangs it,
//! keeping its identity, its kind, its properties and its subtree, and changing
//! only where it sits among its siblings. The two files have to agree about
//! this or a reorder emitted by one is a destruction performed by the other, so
//! the agreement is asserted here against `reconcile`'s own reading rather than
//! restated as a second paragraph.
//!
//! One consequence belongs to this file alone. [`Change::of`] answers
//! `Created` for every `CreateNode`, because it does not hold the tree and
//! cannot know whether the node is already in it. **The arena is the only thing
//! that knows**, so it is where the second token dies: a move drops the
//! [`Created`] it was handed and leaves the census alone, because nothing
//! began. That is the one place in this workspace where dropping a `Created` is
//! correct, and it is correct precisely because no node came into being.
//!
//! # Nothing half-applies
//!
//! Every operation validates before it moves a link. A refusal therefore leaves
//! the graph exactly as it found it, which is what makes [`Refusal::Capacity`]
//! usable at all: a client told its scene is full can carry on submitting a
//! smaller one, and a compositor that had already spliced half an edit in could
//! not offer that. The exit's teardown clause depends on it too — an arena that
//! kept a slot from a refused create could not be filled to the maximum twice.
//!
//! # No panic, and what that costs
//!
//! `Cargo.toml` sets `panic = "abort"` in every profile, so a panic here is not
//! an exception, it is the compositor gone and the screen with it — caused by
//! whatever a client happened to submit. So there is no `unwrap`, no `expect`,
//! no slice indexing that can be out of bounds, and no `debug_assert` standing
//! in for a decision. `SlotIx::at` is how the indexing is disposed of: the
//! slot array is a power of two long and an index is masked into it, so the
//! expression has no failure path rather than a failure path a reader has to
//! convince themselves is unreachable.
//!
//! The arithmetic on the census is saturating for the same reason, and that one
//! is a trade worth naming: saturating addition can hide a bug that a panic
//! would have shown. It is chosen because the bug it could hide is a census off
//! by one, and the alternative it replaces is a machine that stops. *What would
//! reverse this:* a debug build of the compositor that is allowed to abort, at
//! which point the arithmetic should be checked there and saturating in
//! release.
//!
//! # No clock, no randomness, no float
//!
//! Every function here is a pure function of the arena it is given and the
//! record it is handed. This crate takes no `f_env::Env` and would have nothing
//! to ask one: there is no clock to read, nothing to draw, and no iteration
//! order that is not the paint order the client stated. A function whose answer
//! depends only on its arguments is reproducible from its arguments, which is a
//! stronger determinism statement than a seeded one. No `HashMap` — there is no
//! map at all — and no binary floating point: the only quantities here are the
//! fixed-point records `f_abi::scene` defines, carried through unexamined with
//! the scale in each field's name. RFC 0004.

// *A named refusal rather than a panic* is the half of the exit a later edit is
// most likely to walk past, and prose does not stop an edit. These four lints
// are the stop: inside this module, outside its tests, reaching for `unwrap`,
// `expect`, `panic!` or `unreachable!` does not compile. `indexing_slicing` is
// the fifth and the interesting one — it makes every `[...]` on an array here an
// expression somebody had to write an allow for, with the argument for why that
// one cannot be out of bounds attached to it. There are exactly two, and they
// are `Arena::slot` and `Arena::slot_mut`.
//
// Not `arithmetic_side_effects`: it would want an allow on every `+= 1` in the
// file, and an allow written eleven times is an allow nobody reads. The three
// additions and the two subtractions are argued individually instead — at
// `Arena::take_free`, `Arena::release` and `Arena::remaining` — which is fewer
// annotations and more argument.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing
    )
)]

use f_abi::scene::{
    CreateNode, Delta, Entry, NO_NODE, Refusal as WireRefusal, SetEffect, SetPaint, SetPath,
    SetTransform,
};

use crate::kind::{ByKind, Change, Created, Kind, Removal};

/// The most nodes one scene may hold.
///
/// A thousand and twenty-four: a screen of a user interface, not a document.
/// The number is argued from what is on a display rather than from what an
/// application owns — a dense desktop window is a few hundred nodes, a file
/// listing of two hundred rows at three nodes a row is six hundred, and a
/// thousand is that with headroom. An application drawing more than this is
/// drawing more than a person is looking at, and the thing it should send is
/// the window of its content it is actually showing. That windowing decision is
/// `interface/src/canvas.rs`'s `PLACED_MAX` argument one layer down, and it is
/// the same decision: the structure that refuses to hold everything is what
/// forces the question *which part of this is on screen*, which an application
/// has to answer anyway and would otherwise answer late.
///
/// Sized so that the whole retained graph is a small number of pages — the
/// assertion at the bottom of this file is what keeps that true — against a
/// single 4K frame's tens of megabytes. Section 07's arithmetic turns on the
/// scene being orders of magnitude smaller than the pixels it describes, and a
/// graph that cost a frame buffer would have spent the argument.
///
/// A power of two, which is not cosmetic: `SlotIx::at` masks with
/// `NODES_MAX - 1`, and that is what removes the last panicking expression from
/// this file. The assertion below is what stops somebody moving the number to
/// 1500 and quietly reintroducing one.
///
/// *What would reverse this:* the first measured scene that needs more nodes
/// than this and cannot sensibly window itself. The number moves; nothing else
/// here does.
/// Unit: nodes.
pub const NODES_MAX: usize = 1024;

/// The mask that turns any index into one of the slots.
///
/// `NODES_MAX - 1`, which is every low bit set because [`NODES_MAX`] is a power
/// of two. Private, because it is an implementation of [`SlotIx::at`] and not a
/// fact about the arena.
/// Unit: none — a bitmask over slot indices.
const SLOT_MASK: usize = NODES_MAX - 1;

// The two facts `SlotIx::at` rests on, stated where a violation is a build
// rather than a behaviour. A `NODES_MAX` that is not a power of two makes the
// mask wrong; one that does not fit a `u16` makes the links wrong. Neither is
// something to discover from a running compositor.
const _: () = assert!(
    NODES_MAX.is_power_of_two(),
    "NODES_MAX must be a power of two: SlotIx::at masks rather than checks"
);
const _: () = assert!(
    NODES_MAX <= u16::MAX as usize,
    "NODES_MAX must fit the u16 that a link between slots is stored in"
);

/// Which slot, and nothing else.
///
/// A `u16` rather than a `u32` because three of these sit in every slot and
/// [`NODES_MAX`] is asserted to fit one. It is a newtype rather than a bare
/// integer so that a node identifier — which is the client's number, in the
/// client's space — and a slot index — which is this arena's, and means nothing
/// outside it — cannot be passed to each other's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlotIx(u16);

impl SlotIx {
    /// Where in [`Arena::slots`] this is.
    ///
    /// Masked rather than checked, and the difference is the point. `& SLOT_MASK`
    /// yields a value below [`NODES_MAX`] for every `u16` there is, and the array
    /// is exactly [`NODES_MAX`] long, so this expression has no out-of-bounds case
    /// to have an opinion about — not one that is unreachable, none. Every index
    /// this arena stores came out of its own free list, so the mask is the
    /// identity on every value that ever reaches it; it is written anyway,
    /// because *the possibility is removed* and *the possibility is guarded* are
    /// different states and only one of them survives an edit by somebody who
    /// has not read this paragraph.
    const fn at(self) -> usize {
        self.0 as usize & SLOT_MASK
    }
}

/// Why an edit did not reach the graph.
///
/// Local to this crate rather than a reuse of `f_abi::scene::Refusal`, on
/// `reconcile::Refusal`'s terms: that one is about an entry arriving from a
/// peer and packs into RFC 0010's domains because it has to reach a completion,
/// and these are about a graph that already exists and the edit that did not
/// happen to it. [`Refusal::Wire`] carries the other kind through rather than
/// restating it, so a caller draining a ring has one thing to match on and the
/// wire's own refusals keep their own names.
///
/// Every variant that is about a node names it. A caller that has just had an
/// edit refused wants to know which node, and reconstructing that from the
/// delta it submitted is a second lookup it should not have to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The arena already holds [`NODES_MAX`] nodes and the edit asks for
    /// another.
    ///
    /// The refusal this module exists to be able to make. Named rather than
    /// fatal and named rather than silent: a client whose scene outgrew the
    /// compositor's is told so, with nothing half-applied, and may submit a
    /// smaller scene on the next frame. A truncating arena would have drawn a
    /// picture missing whatever did not fit, and nobody would have found out
    /// until a user did.
    Capacity,
    /// An edit names a node this arena does not hold.
    ///
    /// A property set on a node that was never created, or a removal rooted at
    /// one. Refused rather than ignored, because a client that thinks a node
    /// exists and a compositor that does not are about to disagree about every
    /// frame after this one.
    NoSuchNode(u32),
    /// A node is hung under a parent this arena does not hold.
    ///
    /// Separate from [`Refusal::NoSuchNode`] because the node in the message is
    /// a different one — the parent, not the subject — and a caller chasing an
    /// ordering mistake in its own submission wants to be told which.
    NoSuchParent(u32),
    /// `CreateNode::before` names a node that is not a child of the stated
    /// parent.
    ///
    /// Paint order is sibling order, so *insert in front of this* is only
    /// meaningful among siblings. A `before` from somewhere else in the tree is
    /// a client that has lost track of its own structure, and placing the node
    /// somewhere plausible instead would hide it.
    NotASibling(u32),
    /// A node would be hung under itself or under one of its own descendants.
    ///
    /// The cycle `f_abi::scene::CreateNode` cannot refuse because one entry
    /// cannot state it — its own check catches `parent == node` and its comment
    /// hands the rest here. A cycle in a retained graph is not a strange scene,
    /// it is a traversal that does not terminate, which is the compositor
    /// hanging on input a client chose.
    Cycle(u32),
    /// A node that already exists came back with a different kind.
    ///
    /// Refused rather than quietly turned into a removal and a creation, which
    /// is what most retained systems do. `reconcile::Refusal::KindChanged`
    /// refuses the same thing on the client's side of the wire and gives the
    /// argument: identity is stable and author-assigned, so a key whose kind
    /// changed is a reused identifier, and the silent repair costs that node's
    /// whole subtree on a frame nobody was watching.
    KindChanged(u32),
    /// The token and the record handed to [`Arena::hang`] do not describe the
    /// same node.
    ///
    /// [`Arena::hang`] takes a [`Created`] — which is where the node's identity
    /// and kind have already been believed — *and* the `CreateNode` record that
    /// says where to put it, and the two carry the node twice. Checking rather
    /// than trusting is what lets the door take both without one of them being
    /// decoration.
    Mismatched,
    /// The delta is one a peer would have refused.
    ///
    /// Carried through from `f_abi::scene` rather than re-decided here.
    /// [`Change::of`] calls `Delta::check`, so the envelope rules reach the
    /// graph without this file restating any of them — and two statements of
    /// one rule is how the two halves of a system come to disagree.
    Wire(WireRefusal),
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Capacity => "the scene already holds every node the arena has room for",
            Self::NoSuchNode(_) => "an edit names a node this scene does not hold",
            Self::NoSuchParent(_) => "a node is hung under a parent this scene does not hold",
            Self::NotASibling(_) => {
                "a node is placed in front of something that is not its sibling"
            }
            Self::Cycle(_) => "a node would be hung under itself or under its own descendant",
            Self::KindChanged(_) => "a node that already exists came back with a different kind",
            Self::Mismatched => "the creation token and the record name different nodes",
            Self::Wire(refusal) => refusal.message(),
        }
    }

    /// The node the refusal is about, or [`NO_NODE`] when it is about none.
    ///
    /// [`Refusal::Capacity`], [`Refusal::Mismatched`] and [`Refusal::Wire`] are
    /// about the arena, the call and the entry respectively, and answering
    /// [`NO_NODE`] for them is the same *names nothing* the wire format already
    /// spells that way.
    /// Unit: none — a node identifier, not a quantity.
    #[must_use]
    pub const fn node(self) -> u32 {
        match self {
            Self::NoSuchNode(node)
            | Self::NoSuchParent(node)
            | Self::NotASibling(node)
            | Self::Cycle(node)
            | Self::KindChanged(node) => node,
            Self::Capacity | Self::Mismatched | Self::Wire(_) => NO_NODE,
        }
    }
}

/// What hanging a node did.
///
/// Two answers, because *a node began* and *a node moved* are different events
/// for anything counting nodes or invalidating regions, and the wire spells
/// both with `CreateNode`. A caller that does not care may ignore it; a caller
/// keeping a census must not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hung {
    /// The node did not exist and now does. A slot was taken.
    Created,
    /// The node already existed and is now somewhere else among its siblings.
    /// No slot was taken, its subtree came with it, and its properties are
    /// untouched.
    Moved,
}

/// What applying a delta did to the graph.
///
/// Returned rather than swallowed because a compositor draining a ring has to
/// complete each entry, and *what happened* is what a completion says. Every
/// variant carries the number a caller would otherwise go looking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// A node was hung, newly or by moving.
    Hung(Hung),
    /// A subtree left, and this many nodes went with it — the root the delta
    /// named and everything that was under it.
    /// Unit: nodes.
    Removed(usize),
    /// A property of the node this names was replaced, or — for a declaration,
    /// which this graph deliberately does not hold — checked against a node
    /// that is there. [`Arena::set_effect`] is where the difference is argued.
    /// Unit: none — a node identifier, not a quantity.
    Set(u32),
    /// The delta closes a frame and says nothing about the graph.
    ///
    /// Not an empty arm. A commit is a real entry with a real effect, and the
    /// effect is `commit`'s rather than this module's: what the arena owes is
    /// to hand the token on, unexamined, so that whoever publishes the frame
    /// has it. An arena that had quietly done nothing here would be an arena
    /// that looked identical whether commits were being delivered or dropped.
    /// Unit: none — a frame identifier, not a quantity.
    Closes(u64),
}

/// One node's place in the tree, and everything a delta has said about it.
///
/// Fixed width, because [`NODES_MAX`] of them are laid out at once and a slot
/// whose size depended on its contents would be an allocator wearing a struct.
struct Slot {
    /// The node occupying this slot, or `None` for a slot that is free.
    ///
    /// The one field that says whether this slot is part of the tree. It holds
    /// a [`Created`], so the answer to *what is this node and where did it come
    /// from* is a token only a delta can produce, and the slot cannot be filled
    /// by any other route.
    held: Option<Created>,
    /// The slot holding this node's parent, or `None` for a root.
    parent: Option<SlotIx>,
    /// The slot holding this node's first child in paint order.
    first_child: Option<SlotIx>,
    /// The next sibling in paint order — or, while this slot is free, the next
    /// free slot.
    ///
    /// **The double duty is the point.** A slot is in the tree or in the free
    /// list and never both, so the two uses cannot collide, and the free list
    /// therefore costs no storage of its own. This is what *no second region*
    /// means concretely: releasing a slot does not need anywhere to put it.
    next: Option<SlotIx>,
    /// The affine transform a delta set on this node, or `None` for a node no
    /// transform delta has named.
    ///
    /// `f_abi::scene`'s own record, stored as it arrived. `None` rather than an
    /// identity matrix on purpose: `reconcile` states what a node with no
    /// transform of its own means on the client's side, and a second statement
    /// of that default here is a second thing that can disagree with it. The
    /// arena reports absence and lets the renderer read `reconcile`'s answer.
    /// Unit: none — 16.16 fixed point, the scale in each field's name.
    transform: Option<SetTransform>,
    /// The geometry a delta pointed this node at, on
    /// [`Slot::transform`]'s terms.
    /// Unit: bytes, from the first byte of the channel's inline arena.
    path: Option<SetPath>,
    /// What a delta said to paint this node with, on [`Slot::transform`]'s
    /// terms.
    /// Unit: none — intensities scaled so that 65 535 is full.
    paint: Option<SetPaint>,
}

impl Slot {
    /// A slot holding nothing and linked to nothing.
    ///
    /// A `const` so that [`Arena::EMPTY`] can be written without a loop and
    /// without a constructor that returns the whole arena by value.
    const EMPTY: Self = Self {
        held: None,
        parent: None,
        first_child: None,
        next: None,
        transform: None,
        path: None,
        paint: None,
    };
}

/// The retained scene graph: [`NODES_MAX`] slots, and nothing else.
///
/// The module documentation is the argument. What the type adds to it is the
/// absence of `Clone`, `Copy` and `Default`: an arena is a place rather than a
/// value, obtained once from [`Arena::EMPTY`] and thereafter held by reference,
/// and there is no expression that produces a second one from a first.
pub struct Arena {
    /// Every slot there is. The whole of the arena's storage, and the const
    /// assertion at the bottom of this file is what keeps *whole* true.
    slots: [Slot; NODES_MAX],
    /// The first root, in paint order. Roots are a sibling list exactly like
    /// any other, so a client's own root enters the tree by the same code that
    /// hangs everything else — `f_abi::scene::CreateNode::parent` calls
    /// [`NO_NODE`] *a node with no parent*, and this is where that lands.
    roots: Option<SlotIx>,
    /// The first free slot, chained through [`Slot::next`].
    free: Option<SlotIx>,
    /// How many slots have ever been handed out.
    ///
    /// A bump pointer beside the free list, so that [`Arena::EMPTY`] is an
    /// array of empty slots rather than a thousand-link chain a `const fn`
    /// would have to build. It only rises; a released slot goes on [`free`],
    /// and `a_torn_down_scene_is_rebuilt_out_of_the_same_slots` is what
    /// observes that the second scene is served from returned slots rather than
    /// from fresh ones.
    /// Unit: slots.
    bumped: u16,
    /// How many live nodes there are of each kind.
    ///
    /// `crate::kind`'s obligation discharged: the live slots counted against
    /// the creations that filled them. `ByKind` rather than an array of six,
    /// which is the difference between a table that is the wrong length the day
    /// there is a seventh kind — in this crate's own build — and a table that
    /// silently has a hole in it.
    /// Unit: nodes.
    census: ByKind<u16>,
}

impl Arena {
    /// An empty scene.
    ///
    /// A `const` and not a `new()`, because the arena is a hundred-odd
    /// kilobytes and a constructor returning one by value is a copy of all of
    /// it at every call. Written this way it can initialise a `static`, which is
    /// where a component's scene actually lives, and a caller that wants one on
    /// a stack still writes `let mut scene = Arena::EMPTY;`.
    pub const EMPTY: Self = Self {
        slots: [const { Slot::EMPTY }; NODES_MAX],
        roots: None,
        free: None,
        bumped: 0,
        census: ByKind::new([0; Kind::COUNT]),
    };

    /// How many nodes the arena can hold at once.
    ///
    /// [`NODES_MAX`], as a method, for a caller that has the arena and not the
    /// constant. There is no way for the two to disagree.
    /// Unit: nodes.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        NODES_MAX
    }

    /// How many nodes the scene holds.
    ///
    /// Summed from the census rather than kept as a separate counter. One
    /// number maintained in one place cannot drift from itself, and the census
    /// has to be maintained anyway — a second total beside it would be exactly
    /// the kind of derivable fact this tree refuses to store twice.
    /// Unit: nodes.
    #[must_use]
    pub fn live(&self) -> usize {
        self.census.as_array().iter().map(|count| *count as usize).sum()
    }

    /// Is the scene empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live() == 0
    }

    /// How many more nodes will fit.
    ///
    /// Saturating, on the census's terms: [`Arena::live`] cannot exceed
    /// [`NODES_MAX`] — there are only that many slots to fill — but a
    /// subtraction that *could* underflow is a subtraction that aborts a
    /// compositor on a bad day, and this file has none of those.
    /// Unit: nodes.
    #[must_use]
    pub fn remaining(&self) -> usize {
        NODES_MAX.saturating_sub(self.live())
    }

    /// How many live nodes there are of one kind.
    /// Unit: nodes.
    #[must_use]
    pub const fn count_of(&self, kind: Kind) -> usize {
        *self.census.at(kind) as usize
    }

    /// Does the scene hold this node?
    #[must_use]
    pub fn holds(&self, node: u32) -> bool {
        self.find(node).is_some()
    }

    /// What kind of node this is, or `None` for a node the scene does not hold.
    #[must_use]
    pub fn kind_of(&self, node: u32) -> Option<Kind> {
        let ix = self.find(node)?;
        Some(self.slot(ix).held.as_ref()?.kind())
    }

    /// This node's parent, or [`NO_NODE`] for a root.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold — which is a
    /// different answer from *it is a root*, and is kept different because a
    /// caller walking upwards would otherwise stop at the same value for both.
    pub fn parent_of(&self, node: u32) -> Result<u32, Refusal> {
        let ix = self.find(node).ok_or(Refusal::NoSuchNode(node))?;
        Ok(match self.slot(ix).parent {
            Some(parent) => self.node_at(parent),
            None => NO_NODE,
        })
    }

    /// This node's children, in paint order.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn children(&self, node: u32) -> Result<Children<'_>, Refusal> {
        let ix = self.find(node).ok_or(Refusal::NoSuchNode(node))?;
        Ok(Children { arena: self, at: self.slot(ix).first_child })
    }

    /// The roots, in paint order.
    ///
    /// A forest and not a tree, because `f_abi::scene::CreateNode::parent`
    /// permits [`NO_NODE`] and nothing in the format says there is one of them.
    /// A compositor that wants a single root should say so in its own terms;
    /// the graph does not invent one.
    #[must_use]
    pub const fn roots(&self) -> Children<'_> {
        Children { arena: self, at: self.roots }
    }

    /// The transform a delta set on this node, or `None` if none has.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn transform_of(&self, node: u32) -> Result<Option<SetTransform>, Refusal> {
        let ix = self.find(node).ok_or(Refusal::NoSuchNode(node))?;
        Ok(self.slot(ix).transform)
    }

    /// The geometry a delta pointed this node at, or `None` if none has.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn path_of(&self, node: u32) -> Result<Option<SetPath>, Refusal> {
        let ix = self.find(node).ok_or(Refusal::NoSuchNode(node))?;
        Ok(self.slot(ix).path)
    }

    /// What a delta said to paint this node with, or `None` if none has.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn paint_of(&self, node: u32) -> Result<Option<SetPaint>, Refusal> {
        let ix = self.find(node).ok_or(Refusal::NoSuchNode(node))?;
        Ok(self.slot(ix).paint)
    }

    /// Apply one delta to the graph.
    ///
    /// The whole route from an entry a peer submitted to a scene that has
    /// changed. [`Change::of`] is asked first, which calls `Delta::check`, so
    /// every envelope rule `f_abi::scene` states is enforced here without one
    /// of them being written down a second time.
    ///
    /// The arms are per opcode with no wildcard, so a seventh opcode in
    /// `f_abi::scene` stops this build and asks what it does to the graph.
    ///
    /// # Errors
    ///
    /// [`Refusal::Wire`] for a delta a peer would refuse, and any of the
    /// graph's own refusals past it. Nothing is half-applied: see the module's
    /// *nothing half-applies*.
    pub fn apply(&mut self, delta: &Delta) -> Result<Applied, Refusal> {
        let change = Change::of(delta).map_err(Refusal::Wire)?;
        match delta.body {
            Entry::CreateNode(at) => match change {
                Change::Created(created) => self.hang(created, at).map(Applied::Hung),
                // `Change::of` answers `Created` for `CreateNode` and for no
                // other opcode, so this is `f_scene::kind` and `f_abi::scene`
                // disagreeing about which opcode introduces a node — not
                // anything a peer can cause. It is a refusal and not a panic
                // because a compositor that aborts is a screen that goes dark:
                // if the two halves of this build have come apart, saying so to
                // the caller is the last useful thing left to do.
                Change::Removed(_) | Change::Untouched => Err(Refusal::Mismatched),
            },
            Entry::RemoveNode(_) => match change {
                Change::Removed(removal) => self.remove(&removal).map(Applied::Removed),
                Change::Created(_) | Change::Untouched => Err(Refusal::Mismatched),
            },
            Entry::SetTransform(record) => {
                self.set_transform(record).map(|()| Applied::Set(record.node))
            }
            Entry::SetPath(record) => self.set_path(record).map(|()| Applied::Set(record.node)),
            Entry::SetPaint(record) => self.set_paint(record).map(|()| Applied::Set(record.node)),
            Entry::SetEffect(record) => self.set_effect(record).map(|()| Applied::Set(record.node)),
            Entry::Commit(commit) => Ok(Applied::Closes(commit.frame_token)),
        }
    }

    /// Hang a node: create it, or move one that already exists.
    ///
    /// The door a node enters the graph by. It wants a [`Created`] — which only
    /// [`Change::of`] produces, and only out of a delta — and the `CreateNode`
    /// record that says where to put it. The two carry the node's identity and
    /// kind twice and are checked against each other, which is what stops
    /// either of them being decoration.
    ///
    /// Everything is validated before any link moves, so a refusal leaves the
    /// graph untouched.
    ///
    /// # Errors
    ///
    /// [`Refusal::Capacity`] when the node is new and every slot is taken;
    /// [`Refusal::Mismatched`] when the token and the record disagree;
    /// [`Refusal::NoSuchParent`] and [`Refusal::NotASibling`] for a placement
    /// that names nodes this scene does not have in the relationship claimed;
    /// [`Refusal::KindChanged`] for an identifier reused for a different kind of
    /// node; [`Refusal::Cycle`] for a move that would put a node under itself.
    pub fn hang(&mut self, created: Created, at: CreateNode) -> Result<Hung, Refusal> {
        if created.node() != at.node || created.kind().wire() != at.kind {
            return Err(Refusal::Mismatched);
        }
        let node = created.node();
        // A node in front of itself. `f_abi::scene::CreateNode` refuses this at
        // the decoder, and it is refused again here because `hang` is a door in
        // its own right: a caller holding a token and a record has not
        // necessarily come through one.
        if at.before == node {
            return Err(Refusal::NotASibling(at.before));
        }

        let parent = match at.parent {
            NO_NODE => None,
            named => Some(self.find(named).ok_or(Refusal::NoSuchParent(named))?),
        };
        let before = match at.before {
            NO_NODE => None,
            named => {
                let sibling = self.find(named).ok_or(Refusal::NotASibling(named))?;
                if self.slot(sibling).parent != parent {
                    return Err(Refusal::NotASibling(named));
                }
                Some(sibling)
            }
        };

        match self.find(node) {
            // Already here: this is a move. `reconcile`'s *the one sentence the
            // six opcodes can form that means move* is the argument, and the
            // node keeps its identity, its kind, its properties and its
            // subtree.
            Some(ix) => {
                if self.slot(ix).held.as_ref().map(Created::kind) != Some(created.kind()) {
                    return Err(Refusal::KindChanged(node));
                }
                if let Some(under) = parent
                    && self.descends_from(under, ix)
                {
                    return Err(Refusal::Cycle(node));
                }
                self.unlink(ix);
                self.link(ix, parent, before);
                // `created` is dropped here, and this is the one place in the
                // workspace where that is right: nothing began, so the census
                // is unchanged and there is no node for the token to have been
                // the evidence of. See the module's *a create of a node that
                // already exists is a move*.
                Ok(Hung::Moved)
            }
            None => {
                let Some(ix) = self.take_free() else {
                    return Err(Refusal::Capacity);
                };
                let kind = created.kind();
                self.slot_mut(ix).held = Some(created);
                self.census[kind] = self.census[kind].saturating_add(1);
                self.link(ix, parent, before);
                Ok(Hung::Created)
            }
        }
    }

    /// Replace this node's transform.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn set_transform(&mut self, record: SetTransform) -> Result<(), Refusal> {
        let ix = self.find(record.node).ok_or(Refusal::NoSuchNode(record.node))?;
        self.slot_mut(ix).transform = Some(record);
        Ok(())
    }

    /// Replace the geometry this node draws.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn set_path(&mut self, record: SetPath) -> Result<(), Refusal> {
        let ix = self.find(record.node).ok_or(Refusal::NoSuchNode(record.node))?;
        self.slot_mut(ix).path = Some(record);
        Ok(())
    }

    /// Replace what this node is painted with.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn set_paint(&mut self, record: SetPaint) -> Result<(), Refusal> {
        let ix = self.find(record.node).ok_or(Refusal::NoSuchNode(record.node))?;
        self.slot_mut(ix).paint = Some(record);
        Ok(())
    }

    /// Check a declaration against the graph, and hold none of it.
    ///
    /// The one setter here that stores nothing, and the absence is the
    /// decision rather than a gap somebody will fill in later.
    ///
    /// # Why the graph is not where a declaration lives
    ///
    /// Because the thing that reads one does not read the graph.
    /// [`crate::degrade::downgrade`] takes a `&mut [Effect]` — a table of
    /// declarations, ranked and spent against one frame's overrun — so
    /// whichever component runs that policy holds that table, and a
    /// declaration hung on a node would have to be gathered out of the tree
    /// into exactly such a table before it was any use. Two homes for one
    /// fact, and the graph's copy read by nobody.
    ///
    /// It is also not free, and the number is measured rather than supposed. A
    /// declaration per [`Slot`] is sixteen bytes times [`NODES_MAX`], which is
    /// 16 KiB, and `f_compositor`'s heap is declared in a manifest: the field
    /// existed for one build and that build failed at
    /// `user/compositor/src/lib.rs`'s *the heap this manifest declares is
    /// smaller than the graph this component holds*. RFC 0100's bound is the
    /// same argument one level up — a structure a component holds is paid for
    /// where it is held, and this one would have been paid for in a component
    /// that does not read it.
    ///
    /// What the call still does is the part that is this file's: the node has
    /// to exist. A declaration naming a node the scene does not hold is a
    /// producer that has lost track of its own frame, and it is refused here
    /// on the same terms as a paint delta naming one.
    ///
    /// *What would reverse this:* `E3-B07c` finding that the policy wants the
    /// declaration *at* the node — a rank that depends on where in the tree
    /// the effect sits, say — at which point the field comes back, with the
    /// manifest number it costs moving in the same diff.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] for a node the scene does not hold.
    pub fn set_effect(&mut self, record: SetEffect) -> Result<(), Refusal> {
        self.find(record.node).ok_or(Refusal::NoSuchNode(record.node))?;
        Ok(())
    }

    /// Remove a subtree, and answer how many nodes left with it.
    ///
    /// The delta names the root; everything under it goes, which is
    /// `f_abi::scene::RemoveNode`'s rule and not this module's choice. The
    /// ancestry `crate::kind::Created::removed_under` declines to check is
    /// checked here by construction: the walk starts at the root the delta
    /// named and only ever follows this arena's own child links, so a node it
    /// reaches *is* under that root and there is nothing for a caller to
    /// assert.
    ///
    /// # How it walks
    ///
    /// The work list is threaded through the `next` field of the very nodes
    /// about to be released. That is what makes teardown fit in the one
    /// allocation: a recursive walk would spend stack proportional to a depth
    /// the client chose, and an explicit stack would be a second array sized for
    /// the same thing. Each turn of the loop releases exactly one node and no
    /// node is reachable from two parents, so the walk is linear in the subtree
    /// and terminates — and [`Refusal::Cycle`] is what keeps the second half of
    /// that true.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchNode`] when the scene does not hold the root.
    pub fn remove(&mut self, removal: &Removal) -> Result<usize, Refusal> {
        let root = removal.root();
        let at_root = self.find(root).ok_or(Refusal::NoSuchNode(root))?;

        // Detach the subtree from whatever held it first, so that nothing still
        // in the tree points into what is being freed.
        self.unlink(at_root);

        let mut gone = 0;
        let mut pending = Some(at_root);
        while let Some(at) = pending {
            pending = self.slot(at).next;
            // Splice this node's children onto the front of the work list.
            // Their `next` words are being overwritten, which is safe exactly
            // because they are on their way to the free list and will be
            // rewritten again when they get there.
            let mut child = self.slot(at).first_child;
            while let Some(one) = child {
                let after = self.slot(one).next;
                self.slot_mut(one).next = pending;
                pending = Some(one);
                child = after;
            }
            if self.release(at, removal).is_some() {
                gone += 1;
            }
        }
        Ok(gone)
    }

    /// The slot holding a node, or `None`.
    ///
    /// A scan of every slot. The cost is stated rather than hidden: one lookup
    /// is [`NODES_MAX`] slot reads, so a frame of fifty deltas is bounded but
    /// not cheap. `reconcile`'s *the cost is the search* points at this file and
    /// says a key-to-slot index is *going to live anyway* — it does not, yet,
    /// and this is the note saying so. An index is a second array, which is the
    /// one thing the exit sentence spends its whole argument on, so it is not
    /// added before there is a measurement asking for it. *What would reverse
    /// this:* a measured frame in which this scan shows against section 12's
    /// budget, at which point the index goes inside this same struct and the
    /// size assertion at the bottom of the file moves with it, deliberately and
    /// in a diff somebody reads.
    ///
    /// [`NO_NODE`] finds nothing, because a [`Created`] never holds it.
    fn find(&self, node: u32) -> Option<SlotIx> {
        let mut raw: u16 = 0;
        while (raw as usize) < NODES_MAX {
            let ix = SlotIx(raw);
            if let Some(held) = self.slot(ix).held.as_ref()
                && held.node() == node
            {
                return Some(ix);
            }
            raw += 1;
        }
        None
    }

    /// The slot at an index.
    ///
    /// One of the two array accesses in this module, and the allow is the
    /// argument rather than a silencing: [`SlotIx::at`] masks with
    /// `NODES_MAX - 1`, `NODES_MAX` is asserted to be a power of two, and
    /// `slots` is exactly `NODES_MAX` long — so the subscript is below the
    /// length for every `SlotIx` that exists and there is no out-of-bounds case
    /// for this expression to have. `get` would be the same access with an
    /// `Option` the caller has to invent a meaning for, and the meaning would
    /// be *this cannot happen*.
    #[cfg_attr(not(test), allow(clippy::indexing_slicing))]
    const fn slot(&self, ix: SlotIx) -> &Slot {
        &self.slots[ix.at()]
    }

    /// The slot at an index, to write. [`Arena::slot`]'s argument, exactly.
    #[cfg_attr(not(test), allow(clippy::indexing_slicing))]
    fn slot_mut(&mut self, ix: SlotIx) -> &mut Slot {
        &mut self.slots[ix.at()]
    }

    /// The node a slot holds, or [`NO_NODE`] for one that holds none.
    fn node_at(&self, ix: SlotIx) -> u32 {
        match self.slot(ix).held.as_ref() {
            Some(held) => held.node(),
            None => NO_NODE,
        }
    }

    /// A slot to put a new node in, or `None` when there are none left.
    ///
    /// The free list first, then the bump pointer. `None` here is the whole of
    /// [`Refusal::Capacity`]: there is no second place to look, and the
    /// alternative — asking somebody for more memory — is the thing this module
    /// is arranged not to be able to do.
    fn take_free(&mut self) -> Option<SlotIx> {
        if let Some(ix) = self.free {
            self.free = self.slot(ix).next;
            *self.slot_mut(ix) = Slot::EMPTY;
            return Some(ix);
        }
        if (self.bumped as usize) < NODES_MAX {
            let ix = SlotIx(self.bumped);
            self.bumped += 1;
            return Some(ix);
        }
        None
    }

    /// Retire the node in a slot and put the slot back on the free list.
    ///
    /// The token is consumed, which is the only way `crate::kind` allows a node
    /// to end: `removed_by` for the root the delta named, and `removed_under`
    /// for a node that was beneath it. Trying the first and falling back to the
    /// second is total — `removed_by` hands the token back rather than
    /// destroying it when the removal names somebody else — so there is no
    /// branch here that has to be argued unreachable.
    ///
    /// Answers the kind that left, or `None` for a slot that held nothing.
    fn release(&mut self, ix: SlotIx, removal: &Removal) -> Option<Kind> {
        let free = self.free;
        let slot = self.slot_mut(ix);
        let held = slot.held.take();
        *slot = Slot::EMPTY;
        slot.next = free;
        self.free = Some(ix);

        let token = held?;
        let kind = match token.removed_by(removal) {
            Ok(kind) => kind,
            Err(beneath) => beneath.removed_under(removal),
        };
        // Saturating, and the module's *no panic* says why. The subtraction is
        // paired with the addition in `hang` — a slot is released only when it
        // held something, and it held something only because one `hang` put it
        // there — so the floor is never reached.
        self.census[kind] = self.census[kind].saturating_sub(1);
        Some(kind)
    }

    /// The head of the sibling list a parent owns, or of the roots.
    const fn head(&self, parent: Option<SlotIx>) -> Option<SlotIx> {
        match parent {
            Some(one) => self.slot(one).first_child,
            None => self.roots,
        }
    }

    /// Point the head of that list somewhere else.
    fn set_head(&mut self, parent: Option<SlotIx>, to: Option<SlotIx>) {
        match parent {
            Some(one) => self.slot_mut(one).first_child = to,
            None => self.roots = to,
        }
    }

    /// Take a node out of the sibling list it is in, leaving its subtree alone.
    fn unlink(&mut self, ix: SlotIx) {
        let parent = self.slot(ix).parent;
        let after = self.slot(ix).next;
        if self.head(parent) == Some(ix) {
            self.set_head(parent, after);
        } else {
            let mut cursor = self.head(parent);
            while let Some(one) = cursor {
                let next = self.slot(one).next;
                if next == Some(ix) {
                    self.slot_mut(one).next = after;
                    break;
                }
                cursor = next;
            }
        }
        let slot = self.slot_mut(ix);
        slot.parent = None;
        slot.next = None;
    }

    /// Put a node into a sibling list, in front of `before` or at the end.
    ///
    /// Written as one walk with no failure arm. `before` is `None` — append —
    /// or a slot [`Arena::hang`] has already checked to be a child of `parent`,
    /// and both cases end at the same test: stop at the node whose successor is
    /// `before`, which for `None` is the node with no successor at all. There
    /// is therefore no *`before` was not found* case to have to decide about,
    /// and no way for this to leave a node holding a parent it is not listed
    /// under.
    fn link(&mut self, ix: SlotIx, parent: Option<SlotIx>, before: Option<SlotIx>) {
        let slot = self.slot_mut(ix);
        slot.parent = parent;
        slot.next = None;

        let head = self.head(parent);
        if head.is_none() || head == before {
            self.slot_mut(ix).next = head;
            self.set_head(parent, Some(ix));
            return;
        }
        let mut cursor = head;
        while let Some(one) = cursor {
            let next = self.slot(one).next;
            if next == before || next.is_none() {
                self.slot_mut(ix).next = next;
                self.slot_mut(one).next = Some(ix);
                return;
            }
            cursor = next;
        }
    }

    /// Is `candidate` the node at `ancestor`, or somewhere beneath it?
    ///
    /// The cycle check `f_abi::scene::CreateNode` hands down. It walks upwards
    /// from the candidate parent, which is bounded by the depth of the tree and
    /// so by [`NODES_MAX`]; the step counter is what makes that a fact rather
    /// than an inference, because this is the one walk in the file whose whole
    /// purpose is to run over a graph that might already be wrong.
    fn descends_from(&self, candidate: SlotIx, ancestor: SlotIx) -> bool {
        let mut cursor = Some(candidate);
        let mut steps = 0;
        while let Some(one) = cursor {
            if one == ancestor {
                return true;
            }
            if steps >= NODES_MAX {
                // More steps than there are slots means the parent chain is
                // already a cycle. Answering *yes* refuses the edit, which is
                // the safe direction: the alternative is walking forever.
                return true;
            }
            steps += 1;
            cursor = self.slot(one).parent;
        }
        false
    }
}

/// The children of one node, or the roots, in paint order.
///
/// Borrows the arena, so a caller cannot edit the graph while walking it — the
/// iterator-invalidation question is answered by the borrow checker rather than
/// by a rule in this paragraph.
pub struct Children<'a> {
    /// The graph being walked.
    arena: &'a Arena,
    /// The slot this iterator will report next.
    at: Option<SlotIx>,
}

impl Iterator for Children<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        let at = self.at?;
        self.at = self.arena.slot(at).next;
        self.arena.slot(at).held.as_ref().map(Created::node)
    }
}

// The slots are the whole of the arena. Anything else this struct grows — a
// key-to-slot index, a dirty set, a second generation of the tree — shows up
// here as a size that is no longer the slot array plus a handful of words, and
// the build stops. The exit this file is accepted on says *one fixed
// allocation*, and a second region inside one struct is still a second region.
//
// It is a const assertion and not a test because the thing it is about is the
// shape of a type: there is no behaviour to observe, and the honest place for a
// statement nobody may violate is the one that refuses to produce an artefact.
const _: () = assert!(
    size_of::<Arena>() - size_of::<[Slot; NODES_MAX]>() <= 32,
    "the arena has grown something beside its slots: one fixed allocation is no longer one"
);

// And the region is small enough to be somewhere a component can actually put
// it: sixty-four pages of four kilobytes, which is the ceiling `NODES_MAX`'s
// documentation argues against a frame buffer's tens of megabytes. A retained
// graph that cost what the pixels cost would have spent section 07's whole
// argument. Widening a slot — a seventh property record, a wider link — is what
// this catches, and it catches it in the same diff that widened it.
const _: () = assert!(
    size_of::<Arena>() <= 64 * 4096,
    "the arena no longer fits the page budget NODES_MAX was chosen against"
);

#[cfg(test)]
mod tests {
    use super::*;
    use f_abi::NO_DEADLINE;
    use f_abi::scene::{Commit, RemoveNode, SetEffect, fill, op};

    /// A deadline far enough from zero to be unmistakably a deadline.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// A delta around a body, with an envelope that is legal for it.
    ///
    /// The deadline comes from `op::carries_deadline` rather than a branch per
    /// opcode, so a seventh opcode gets a legal envelope from the list that
    /// declared it. `abi/src/scene.rs` and `crate::kind`'s tests build theirs
    /// the same way.
    fn delta(body: Entry) -> Delta {
        let scheduled = op::carries_deadline(body.opcode()) == Some(true);
        Delta {
            user_data: 0,
            class: 0,
            deadline: if scheduled { SCHEDULED_AT } else { NO_DEADLINE },
            payload_offset: 0,
            flags: 0,
            body,
        }
    }

    /// A `CreateNode` record placing `node` of `kind` under `parent`, in front
    /// of `before`.
    const fn placing(node: u32, parent: u32, before: u32, kind: Kind) -> CreateNode {
        CreateNode { node, parent, before, kind: kind.wire() }
    }

    /// The delta that hangs a node.
    fn hang_of(node: u32, parent: u32, before: u32, kind: Kind) -> Delta {
        delta(Entry::CreateNode(placing(node, parent, before, kind)))
    }

    /// The delta that removes a subtree.
    fn remove_of(node: u32) -> Delta {
        delta(Entry::RemoveNode(RemoveNode { node }))
    }

    /// The delta that declares what a node's effect costs.
    fn declare_of(node: u32) -> Delta {
        delta(Entry::SetEffect(SetEffect { node, estimate_us_x100: 9_000, saving_us_x100: 3_500 }))
    }

    #[test]
    fn a_declaration_is_checked_against_the_graph_and_held_nowhere() {
        // `E3-B07h`'s arm in `Arena::apply`, and the one thing that arm
        // decides: a declaration is about a node, so the node has to be there.
        // Without this test the arm could be `Ok(Applied::Set(record.node))`
        // with no lookup at all and the whole suite would stay green, because
        // no corpus frame anywhere in this crate declares an effect cost.
        let mut arena = Arena::EMPTY;
        let root = hang_of(1, NO_NODE, NO_NODE, Kind::Effect);
        assert_eq!(arena.apply(&root), Ok(Applied::Hung(Hung::Created)));

        assert_eq!(arena.apply(&declare_of(1)), Ok(Applied::Set(1)));
        assert_eq!(
            arena.apply(&declare_of(2)),
            Err(Refusal::NoSuchNode(2)),
            "a declaration naming a node the scene does not hold was applied"
        );

        // *Held nowhere* is the other half of the sentence and it is not a
        // behaviour this test can observe — there is no accessor to ask,
        // because there is nothing stored to ask about. What guards it is the
        // pair of `const _` assertions above this module, which are what the
        // field cost when it briefly existed: a declaration per slot is 16 KiB
        // and `user/compositor`'s manifest is where that lands.
        // `Arena::set_effect` carries the argument.
        //
        // What this half does assert is the visible consequence: applying a
        // declaration leaves the graph exactly as it was, so the node it named
        // is neither created by it nor changed in any way this file reports.
        assert!(arena.holds(1) && !arena.holds(2));
        assert_eq!(arena.live(), 1, "a declaration created a node");
        assert_eq!(arena.kind_of(1), Some(Kind::Effect));
        assert_eq!(arena.paint_of(1), Ok(None));
        assert_eq!(arena.transform_of(1), Ok(None));
        assert_eq!(arena.path_of(1), Ok(None));
    }

    /// Node `nth`'s kind, cycling through every kind there is.
    ///
    /// Derived from `Kind::ALL` rather than written out, so a seventh kind ends
    /// up in the scenes below on the day it is declared.
    fn kind_of_nth(nth: usize) -> Kind {
        match Kind::ALL.get(nth % Kind::COUNT) {
            Some(kind) => *kind,
            None => Kind::Draw,
        }
    }

    /// Node identifiers are one-based: `NO_NODE` is zero and names nothing.
    ///
    /// Node 1 is the only root; every later node hangs under an earlier one, so
    /// the scene is a sixteen-way tree about three levels deep — depth and
    /// siblings both, rather than a chain or a fan, since the two exercise
    /// different halves of `link`.
    const fn parent_of_nth(node: u32) -> u32 {
        if node <= 1 { NO_NODE } else { (node - 2) / 16 + 1 }
    }

    /// Fill the arena to exactly `NODES_MAX` nodes, appending each in turn.
    ///
    /// Every node arrives as a delta, through `apply`, so the scene below is
    /// built by the same route a peer's entries take and not by a back door the
    /// tests have to themselves.
    fn fill_to_the_maximum(arena: &mut Arena) {
        for nth in 1..=NODES_MAX {
            let node = nth as u32;
            let hung =
                arena.apply(&hang_of(node, parent_of_nth(node), NO_NODE, kind_of_nth(nth - 1)));
            assert_eq!(hung, Ok(Applied::Hung(Hung::Created)), "node {node} did not go in");
        }
    }

    /// How many slots actually hold something, counted by walking the array.
    ///
    /// The second opinion. `Arena::live` sums the census, which is maintained
    /// by `hang` and `release`; this walks the storage. Two numbers maintained
    /// by different code agreeing is worth more than either of them alone,
    /// which is the whole reason `crate::kind` asked for a census in the first
    /// place.
    fn occupied(arena: &Arena) -> usize {
        arena.slots.iter().filter(|slot| slot.held.is_some()).count()
    }

    #[test]
    fn a_scene_at_the_maximum_is_built_mutated_and_torn_down_in_one_allocation() {
        // The exit sentence, in order.
        let mut arena = Arena::EMPTY;
        assert!(arena.is_empty());
        assert_eq!(arena.capacity(), NODES_MAX);

        // Built, to the maximum.
        fill_to_the_maximum(&mut arena);
        assert_eq!(arena.live(), NODES_MAX);
        assert_eq!(arena.remaining(), 0);
        assert_eq!(occupied(&arena), NODES_MAX);
        // Every slot the arena has is in use, and it never reached for a
        // second region: `bumped` is how many slots it has ever handed out.
        assert_eq!(arena.bumped as usize, NODES_MAX);

        // Mutated. Properties first, on nodes scattered through the tree.
        let painted = SetPaint {
            node: 500,
            red_x65535: 0,
            green_x65535: 32_768,
            blue_x65535: 65_535,
            alpha_x65535: 65_535,
            stroke_width_x65536: 0,
        };
        assert_eq!(arena.apply(&delta(Entry::SetPaint(painted))), Ok(Applied::Set(500)));
        assert_eq!(arena.paint_of(500), Ok(Some(painted)));
        let shaped = SetPath {
            node: 501,
            geometry_offset: 0x2000,
            geometry_bytes: 96,
            fill_rule: fill::EVEN_ODD,
        };
        assert_eq!(arena.apply(&delta(Entry::SetPath(shaped))), Ok(Applied::Set(501)));
        assert_eq!(arena.path_of(501), Ok(Some(shaped)));
        // A node no transform delta has named reports absence rather than an
        // identity matrix this module would have had to invent.
        assert_eq!(arena.transform_of(501), Ok(None));

        // Structure second: a move. Node 1024 leaves wherever it was and is
        // hung in front of node 2's first child, and the arena is no fuller
        // for it.
        let elsewhere = arena.parent_of(1024).unwrap();
        let displaced = arena.children(2).unwrap().next().unwrap();
        assert_eq!(
            arena.apply(&hang_of(1024, 2, displaced, kind_of_nth(1023))),
            Ok(Applied::Hung(Hung::Moved))
        );
        assert_eq!(arena.live(), NODES_MAX, "a move is not a creation");
        assert_eq!(arena.parent_of(1024), Ok(2));
        assert_ne!(elsewhere, 2);
        let order: [u32; 2] = [
            arena.children(2).unwrap().next().unwrap(),
            arena.children(2).unwrap().nth(1).unwrap(),
        ];
        assert_eq!(order, [1024, displaced], "the move did not land in front of its sibling");

        // A commit passes through the graph without editing it.
        let closed = arena.apply(&delta(Entry::Commit(Commit { frame_token: 0x1234 })));
        assert_eq!(closed, Ok(Applied::Closes(0x1234)));
        assert_eq!(arena.live(), NODES_MAX);

        // Torn down, by a delta, in one removal of the one root.
        let removed = arena.apply(&remove_of(1));
        assert_eq!(removed, Ok(Applied::Removed(NODES_MAX)));
        assert!(arena.is_empty());
        assert_eq!(occupied(&arena), 0, "a slot survived the teardown holding something");
        assert_eq!(arena.roots().count(), 0);
        for kind in Kind::ALL {
            assert_eq!(arena.count_of(kind), 0, "{} survived the teardown", kind.name());
        }
    }

    #[test]
    fn a_torn_down_scene_is_rebuilt_out_of_the_same_slots() {
        // The other half of *inside one fixed allocation*: the second scene at
        // the maximum is served entirely from slots the first one gave back.
        // `bumped` only ever rises, so a second region would show here as it
        // rising again.
        let mut arena = Arena::EMPTY;
        fill_to_the_maximum(&mut arena);
        let handed_out = arena.bumped;
        assert_eq!(handed_out as usize, NODES_MAX);

        assert_eq!(arena.apply(&remove_of(1)), Ok(Applied::Removed(NODES_MAX)));
        assert!(arena.is_empty());

        fill_to_the_maximum(&mut arena);
        assert_eq!(arena.live(), NODES_MAX);
        assert_eq!(
            arena.bumped, handed_out,
            "the second scene took fresh slots instead of the returned ones"
        );
    }

    #[test]
    fn exceeding_the_maximum_is_a_named_refusal_and_changes_nothing() {
        // The clause that gets faked. The arena is actually driven past its
        // maximum, the refusal actually comes back by name, and the scene is
        // then shown to be exactly what it was — because a refusal that had
        // consumed a slot would be a refusal that makes the next frame smaller.
        let mut arena = Arena::EMPTY;
        fill_to_the_maximum(&mut arena);

        let one_more = (NODES_MAX + 1) as u32;
        let refused = arena.apply(&hang_of(one_more, 1, NO_NODE, Kind::Draw));
        assert_eq!(refused, Err(Refusal::Capacity));
        assert_eq!(Refusal::Capacity.node(), NO_NODE);

        assert!(!arena.holds(one_more), "the refused node took a slot anyway");
        assert_eq!(arena.live(), NODES_MAX);
        assert_eq!(occupied(&arena), NODES_MAX);
        assert_eq!(arena.children(1).unwrap().count(), 16, "the refusal disturbed the tree");

        // And the refusal is about room rather than about that node: free one
        // and the identical delta is accepted.
        assert_eq!(arena.apply(&remove_of(NODES_MAX as u32)), Ok(Applied::Removed(1)));
        assert_eq!(arena.remaining(), 1);
        assert_eq!(
            arena.apply(&hang_of(one_more, 1, NO_NODE, Kind::Draw)),
            Ok(Applied::Hung(Hung::Created))
        );
        assert_eq!(arena.live(), NODES_MAX);
    }

    #[test]
    fn the_census_and_the_slots_agree() {
        // Two numbers maintained by different code. `crate::kind` hands this
        // check down in as many words: a `Created` dropped on the floor is a
        // node the arena believes in and does not hold, and the only way to see
        // that is to count both.
        let mut arena = Arena::EMPTY;
        fill_to_the_maximum(&mut arena);
        for kind in Kind::ALL {
            let walked = arena
                .slots
                .iter()
                .filter(|slot| slot.held.as_ref().map(Created::kind) == Some(kind))
                .count();
            assert_eq!(arena.count_of(kind), walked, "the census of {} is wrong", kind.name());
        }
        assert_eq!(arena.live(), occupied(&arena));

        // A move drops a `Created` and must not move a single count.
        let before: [usize; Kind::COUNT] = Kind::ALL.map(|kind| arena.count_of(kind));
        let moved = hang_of(3, 1, NO_NODE, arena.kind_of(3).unwrap());
        assert_eq!(arena.apply(&moved), Ok(Applied::Hung(Hung::Moved)));
        let after: [usize; Kind::COUNT] = Kind::ALL.map(|kind| arena.count_of(kind));
        assert_eq!(before, after, "a move changed the population");
        assert_eq!(arena.live(), occupied(&arena));
    }

    #[test]
    fn a_subtree_removal_takes_everything_under_it_and_nothing_beside_it() {
        let mut arena = Arena::EMPTY;
        fill_to_the_maximum(&mut arena);
        // Node 2's subtree: itself, its sixteen children, and theirs.
        let mut expected = 0;
        for node in 1..=NODES_MAX as u32 {
            let mut walk = node;
            while walk != NO_NODE {
                if walk == 2 {
                    expected += 1;
                    break;
                }
                walk = parent_of_nth(walk);
            }
        }
        assert!(expected > 16, "the shape under test is not a subtree worth removing");

        assert_eq!(arena.apply(&remove_of(2)), Ok(Applied::Removed(expected)));
        assert!(!arena.holds(2));
        assert!(arena.holds(1), "the root went with a subtree that was not it");
        assert_eq!(arena.live(), NODES_MAX - expected);
        assert_eq!(arena.live(), occupied(&arena));
        // The parent's list closed over the hole rather than keeping a link
        // into freed storage.
        assert!(arena.children(1).unwrap().all(|child| child != 2));
    }

    #[test]
    fn a_cycle_spread_over_two_entries_is_refused() {
        // `f_abi::scene::CreateNode` refuses `parent == node` and says the rest
        // is the graph's. This is the rest: two entries, each legal on its own,
        // that together would make a traversal that does not terminate.
        let mut arena = Arena::EMPTY;
        assert!(arena.apply(&hang_of(1, NO_NODE, NO_NODE, Kind::Layer)).is_ok());
        assert!(arena.apply(&hang_of(2, 1, NO_NODE, Kind::Layer)).is_ok());
        assert!(arena.apply(&hang_of(3, 2, NO_NODE, Kind::Layer)).is_ok());

        // 1 under 3, where 3 is 1's grandchild.
        assert_eq!(arena.apply(&hang_of(1, 3, NO_NODE, Kind::Layer)), Err(Refusal::Cycle(1)));
        // And the near case: a node under itself. `f_abi::scene::CreateNode`
        // refuses it at the decoder, and it is refused again here, because
        // `Change::of` does not look at `parent` and a caller that built its
        // own delta never met that decoder.
        assert_eq!(arena.apply(&hang_of(1, 1, NO_NODE, Kind::Layer)), Err(Refusal::Cycle(1)));
        // Nothing moved.
        assert_eq!(arena.parent_of(1), Ok(NO_NODE));
        assert_eq!(arena.parent_of(3), Ok(2));
        assert_eq!(arena.live(), 3);
    }

    #[test]
    fn an_edit_naming_a_node_the_scene_does_not_hold_is_refused() {
        let mut arena = Arena::EMPTY;
        assert!(arena.apply(&hang_of(1, NO_NODE, NO_NODE, Kind::Draw)).is_ok());

        let orphan = SetPaint {
            node: 9,
            red_x65535: 0,
            green_x65535: 0,
            blue_x65535: 0,
            alpha_x65535: 0,
            stroke_width_x65536: 0,
        };
        assert_eq!(arena.apply(&delta(Entry::SetPaint(orphan))), Err(Refusal::NoSuchNode(9)));
        assert_eq!(arena.apply(&remove_of(9)), Err(Refusal::NoSuchNode(9)));
        assert_eq!(arena.apply(&hang_of(2, 9, NO_NODE, Kind::Draw)), Err(Refusal::NoSuchParent(9)));
        // `before` has to be a sibling, not merely a node.
        assert!(arena.apply(&hang_of(2, 1, NO_NODE, Kind::Draw)).is_ok());
        assert_eq!(arena.apply(&hang_of(3, NO_NODE, 2, Kind::Draw)), Err(Refusal::NotASibling(2)));
        assert_eq!(arena.live(), 2, "a refused edit put something in the scene");
    }

    #[test]
    fn an_identifier_reused_for_another_kind_is_refused() {
        let mut arena = Arena::EMPTY;
        assert!(arena.apply(&hang_of(1, NO_NODE, NO_NODE, Kind::Draw)).is_ok());
        assert_eq!(
            arena.apply(&hang_of(1, NO_NODE, NO_NODE, Kind::Semantic)),
            Err(Refusal::KindChanged(1))
        );
        assert_eq!(arena.kind_of(1), Some(Kind::Draw));
        assert_eq!(arena.count_of(Kind::Semantic), 0);
    }

    #[test]
    fn a_delta_the_wire_would_refuse_never_reaches_the_graph() {
        // `apply` asks `Change::of`, which asks `Delta::check`, so the envelope
        // rules arrive here without this file restating one of them.
        let mut arena = Arena::EMPTY;
        let mut scheduled = hang_of(1, NO_NODE, NO_NODE, Kind::Draw);
        scheduled.deadline = SCHEDULED_AT;
        assert_eq!(
            arena.apply(&scheduled),
            Err(Refusal::Wire(WireRefusal::Reserved)),
            "a deadline on an opcode that does not read one reached the graph"
        );
        assert!(arena.is_empty());

        let no_node = delta(Entry::CreateNode(CreateNode {
            node: NO_NODE,
            parent: NO_NODE,
            before: NO_NODE,
            kind: Kind::Draw.wire(),
        }));
        assert_eq!(arena.apply(&no_node), Err(Refusal::Wire(WireRefusal::NoNode)));
        assert!(arena.is_empty());
    }

    #[test]
    fn every_refusal_says_something_and_names_what_it_is_about() {
        // Written out rather than looped, because the corpus is the enum and a
        // loop would need an array of it — which is the hand-written list this
        // tree keeps finding holes in. An arm missing here is a variant with no
        // `message`, which the match in `message` would already have refused to
        // compile.
        for refusal in [
            Refusal::Capacity,
            Refusal::NoSuchNode(7),
            Refusal::NoSuchParent(7),
            Refusal::NotASibling(7),
            Refusal::Cycle(7),
            Refusal::KindChanged(7),
            Refusal::Mismatched,
            Refusal::Wire(WireRefusal::Malformed),
        ] {
            assert!(!refusal.message().is_empty());
        }
        assert_eq!(Refusal::Cycle(7).node(), 7);
        assert_eq!(Refusal::Wire(WireRefusal::Malformed).node(), NO_NODE);
    }
}
