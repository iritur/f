// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The tree itself: what one frame of entries does to it, and what it refuses.
//!
//! # Whole or not at all, and why this one may copy
//!
//! [`Tree::apply`] copies the tree, applies the frame into the copy, and swaps
//! only when every entry succeeded. `f_scene::commit::Batch` cannot do that —
//! its arena is 131 096 bytes — so it walks the frame twice and carries a
//! `Refused::Diverged` for the case where the second walk disagrees with the
//! first. This tree is a few hundred bytes, so the copy is available, and taking
//! it removes that whole variant: there is no second walk to disagree with, and
//! a refusal here is a tree that was never touched rather than a tree that was
//! touched and put back.
//!
//! The reason to say so rather than to quietly differ: a reader who knows one of
//! these two files will assume the other is arranged the same way, and the
//! assumption is wrong in the direction that matters.
//!
//! *What would reverse this:* a tree that outgrows a copy — [`NODES_MAX`] in the
//! thousands, or a node that carries a content body — at which point this file
//! owes the two-walk argument and the variant that goes with it.
//!
//! # The key, and what it is doing in the signature
//!
//! [`Tree::apply`] takes an `f_abi::semantic::Sealed` **by value**. That type
//! has no public constructor and is minted in exactly one place —
//! `Session::accept`, on a commit it did not refuse — so a caller cannot reach
//! this function with a frame whose commit never arrived. RFC 0083 records that
//! `abi` mints the key and cannot force its use, because a `#![no_std]` crate
//! with no allocator cannot accumulate a frame, and names *the apply path that
//! demands the key* as `E3-B06c`'s. This signature is that sentence discharged.
//!
//! *What would reverse this:* an apply path here that takes a
//! [`Delta`](f_abi::semantic::Delta) and no `Sealed`. RFC 0083 says so in those
//! words, and it means a path, not this path: a second entry point that applied
//! one delta without a key would undo the argument whatever this one still says.

use f_abi::semantic::{DeclareNode, Delta, Entry, NO_NODE, RoleOrdinal, Sealed, StateBits};

/// How many nodes one tree holds. Unit: nodes.
///
/// Sixteen, and it is a *receiver's* bound rather than the vocabulary's: RFC
/// 0077 puts no ceiling on a tree and `f_interface::node::DEPTH_MAX` bounds only
/// the depth. What picks this number is where the tree lives — one frame the
/// system allocated, holding a [`crate::Registry`] of them — and a tree that
/// wanted more would be asking for more of the system's memory than a component
/// gets to ask for by declaring nodes.
///
/// *What would reverse this:* a registry sized from a capability rather than
/// from a constant, which is the day a tree's size becomes something an
/// application is charged for. `E3-B06e`'s layout solver is the first thing that
/// will care.
pub const NODES_MAX: usize = 16;

/// How many edits one frame may carry before this receiver refuses it.
/// Unit: entries.
///
/// The frame is staged before any of it reaches the tree — that is what
/// *whole or not at all* costs — so this is memory the receiver holds on the
/// sender's say-so, and it is bounded for that reason rather than for a
/// vocabulary one. A sender with more to say sends two frames, which is what a
/// reconciler does anyway.
pub const FRAME_ENTRIES_MAX: usize = 24;

/// One node, as the system holds it.
///
/// What is here is identity, place, order, role, state and intent — section
/// 11's struct minus the three fields this build has nowhere to put. `content`
/// and `relations` are refused rather than stored ([`Rejected::NotStored`]), and
/// `layout` and `style` are `E3-B06d` and `E3-B06e`'s.
///
/// The role is kept as a [`RoleOrdinal`] and never as a `u16`. That type's field
/// is private and `abi` mints one in exactly one place, so a node in this tree
/// carries a role the agreed vocabulary **named** — and a reader that wants the
/// enum asks `f_interface::node::Role::from_index`, which is the one function
/// that turns an ordinal into a role. Storing the bare number would be RFC
/// 0083's *a second decoder*, one layer further in than the two it already
/// names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    /// The identifier its author chose, stable across every redesign.
    /// Unit: none — a node identifier, never [`NO_NODE`].
    id: u64,
    /// The node it sits under, or [`NO_NODE`] for a root.
    /// Unit: none — a node identifier.
    parent: u64,
    /// Where it sits among its siblings, smallest first.
    ///
    /// A number rather than a link, because `SetRelations` aside this tree is
    /// read far more often than it is edited and a rank is what a projection
    /// wants: *the third cell of this row* is a comparison, not a walk. Ranks
    /// are dense on insertion and are allowed to become sparse on removal, which
    /// costs nothing — only their order is ever read.
    /// Unit: none — an order among siblings, not a position in the array.
    rank: u32,
    /// What it is.
    /// Unit: none — an ordinal the agreed vocabulary named.
    role: RoleOrdinal,
    /// What operating it does, or [`f_abi::semantic::NO_INTENT`].
    /// Unit: none — a capability reference in the declaring peer's own space.
    intent: u32,
    /// How it stands, once something has said.
    ///
    /// **`Option`, and the absence is a finding rather than a convenience.**
    /// `f_abi::semantic::StateBits` has no constructor outside `abi` — the same
    /// rule [`RoleOrdinal`] keeps, for the same reason — so there is no zero
    /// here to start a node at. A receiver that wrote `StateBits(0)` would be
    /// minting an admitted value it never admitted, which is the one thing that
    /// type exists to make impossible. So a node that has never been told how it
    /// stands says so, and `f_interface::node::StateSet::NONE` is what a
    /// projection reads it as.
    /// Unit: none — a bit set the agreed vocabulary admitted.
    state: Option<StateBits>,
}

impl Node {
    /// The identifier its author chose.
    /// Unit: none — a node identifier.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The node it sits under, or [`NO_NODE`].
    /// Unit: none — a node identifier.
    #[must_use]
    pub const fn parent(&self) -> u64 {
        self.parent
    }

    /// Where it sits among its siblings.
    /// Unit: none — an order among siblings.
    #[must_use]
    pub const fn rank(&self) -> u32 {
        self.rank
    }

    /// What it is, as the ordinal the vocabulary admitted.
    #[must_use]
    pub const fn role(&self) -> RoleOrdinal {
        self.role
    }

    /// What operating it does.
    /// Unit: none — a capability reference.
    #[must_use]
    pub const fn intent(&self) -> u32 {
        self.intent
    }

    /// How it stands, or `None` if no `SetState` has named this node.
    #[must_use]
    pub const fn state(&self) -> Option<StateBits> {
        self.state
    }
}

/// What one applied frame did.
///
/// Counted rather than inferred, for `f_compositor::tree::Counters`' reason: a
/// receiver that reported *the frame was applied* and nothing else would be
/// publishing its own opinion of its success. Every field here is a number the
/// sender also knows, because the sender wrote the entries that produced it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Nodes that began. Unit: nodes.
    pub declared: u64,
    /// Entries that changed how a node stands. Unit: entries.
    pub stated: u64,
    /// Nodes that left, subtrees included. Unit: nodes.
    pub removed: u64,
    /// Handshakes carried inside the frame. Unit: entries.
    ///
    /// Counted and not applied. An agreement is a fact about the channel and the
    /// tree holds none of it; it is counted rather than skipped silently so that
    /// a frame whose entries do not add up is visibly a frame with a handshake
    /// in it rather than a frame that lost one.
    pub agreements: u64,
}

impl Applied {
    /// A frame that did nothing, which is a legal frame: a commit with no edits
    /// before it is a peer saying *nothing changed*, and refusing it would make
    /// an idle frame an error.
    pub const ZERO: Self = Self { declared: 0, stated: 0, removed: 0, agreements: 0 };
}

/// Why a frame did not reach the tree, or a handle did not reach a tree.
///
/// One enum for both, because the caller that has to act on either is the same
/// caller and a second enum would be a second thing to translate. Which of the
/// two a variant belongs to is said on the variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The handle names a generation this slot has moved past.
    ///
    /// **The registry's, and the one this task exists to produce.** A component
    /// that has ended leaves [`crate::Registry::close`] behind it, and every
    /// handle minted before that call answers this from then on.
    Stale,
    /// The handle names a slot that holds no tree.
    Vacant,
    /// The frame carried more entries than [`FRAME_ENTRIES_MAX`].
    Full,
    /// The frame carried a commit as one of its edits.
    ///
    /// A commit is the frame's boundary rather than an entry in it: it is what
    /// produced the [`Sealed`] this apply was called with, and staging it too
    /// would be counting the boundary as a thing inside it.
    NotAnEdit,
    /// A node was declared whose identifier the tree already holds.
    ///
    /// Not a merge and not a replacement. RFC 0077 says a node whose intent
    /// changes is a different node, so redeclaring one is a peer that has lost
    /// track of what it sent.
    /// Unit: none — the node identifier.
    Declared(u64),
    /// An entry named a node the tree does not hold.
    /// Unit: none — the node identifier.
    NoSuchNode(u64),
    /// A declaration named a parent the tree does not hold.
    /// Unit: none — the parent's node identifier.
    NoSuchParent(u64),
    /// A declaration asked to sit before a node that is not a sibling of it.
    /// Unit: none — the node identifier that was named as `before`.
    NotASibling(u64),
    /// The tree holds [`NODES_MAX`] nodes and the frame declared another.
    Capacity,
    /// The entry is one this build stores nothing of.
    ///
    /// `SetContent` and `SetRelations`, and [`crate`]'s own comment argues the
    /// refusal: a receiver that answered an entry and stored nothing of it would
    /// be a tree that disagrees with its writer about what it holds. Refusing is
    /// what a writer can act on.
    /// Unit: none — the node identifier the entry named.
    NotStored(u64),
}

impl Rejected {
    /// A word for a log line or a boot report.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Stale => "the handle names a generation this tree has moved past",
            Self::Vacant => "the handle names a slot that holds no tree",
            Self::Full => "the frame carries more entries than this receiver stages",
            Self::NotAnEdit => "a commit was staged as one of the frame's edits",
            Self::Declared(_) => "a node was declared whose identifier the tree already holds",
            Self::NoSuchNode(_) => "an entry named a node the tree does not hold",
            Self::NoSuchParent(_) => "a declaration named a parent the tree does not hold",
            Self::NotASibling(_) => "a declaration named a `before` that is not a sibling",
            Self::Capacity => "the tree is full",
            Self::NotStored(_) => "this build stores nothing of that entry",
        }
    }

    /// The node this refusal names, or [`NO_NODE`] where it names none.
    ///
    /// [`NO_NODE`] rather than an `Option`, because the wire spells the same
    /// absence the same way and `f_compositor::tree` already reports a refusal's
    /// node in a completion's `ext` on those terms.
    /// Unit: none — a node identifier.
    #[must_use]
    pub const fn node(self) -> u64 {
        match self {
            Self::Declared(node)
            | Self::NoSuchNode(node)
            | Self::NoSuchParent(node)
            | Self::NotASibling(node)
            | Self::NotStored(node) => node,
            Self::Stale | Self::Vacant | Self::Full | Self::NotAnEdit | Self::Capacity => NO_NODE,
        }
    }
}

/// The frame under construction: the entries offered since the last commit.
///
/// The same shape `f_scene::commit::Batch` has one layer down, and deliberately:
/// a receiver that has learnt one of these loops has learnt the other. What it
/// does **not** have is a `Sealed` of its own — `abi`'s is the one that matters
/// here, and a second seal minted by this type would be exactly the *second
/// decoder* shape RFC 0083 refuses, applied to a key.
#[derive(Clone, Copy, Debug)]
pub struct Staged {
    /// The entries, oldest first. Slots past `len` are never read.
    entries: [Option<Delta>; FRAME_ENTRIES_MAX],
    /// How many of `entries` belong to the frame being assembled.
    /// Unit: entries.
    len: usize,
}

impl Staged {
    /// A frame with nothing in it.
    pub const EMPTY: Self = Self { entries: [None; FRAME_ENTRIES_MAX], len: 0 };

    /// Add one entry to the frame being assembled.
    ///
    /// # Errors
    ///
    /// [`Rejected::Full`] past [`FRAME_ENTRIES_MAX`], and [`Rejected::NotAnEdit`]
    /// for a commit — which is refused *here*, on the way in, rather than in
    /// [`Tree::apply`], so that a caller that staged one learns before it has a
    /// key in its hand.
    pub fn offer(&mut self, delta: Delta) -> Result<(), Rejected> {
        if matches!(delta.body, Entry::Commit(_)) {
            return Err(Rejected::NotAnEdit);
        }
        let slot = self.entries.get_mut(self.len).ok_or(Rejected::Full)?;
        *slot = Some(delta);
        self.len += 1;
        Ok(())
    }

    /// Forget everything staged.
    ///
    /// Called on both paths out of [`Tree::apply`] by the caller, which is
    /// `f_scene::commit::Batch::commit`'s rule restated where a reader will look
    /// for it: a refused frame does not linger into the next one. The length is
    /// what is reset and the slots are left alone, because a slot past `len` is
    /// never read and zeroing it would be work nobody can observe.
    pub const fn clear(&mut self) {
        self.len = 0;
    }

    /// How many entries the frame holds. Unit: entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Is the frame empty?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &Delta> {
        self.entries.iter().take(self.len).flatten()
    }
}

impl Default for Staged {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// The tree the system owns.
///
/// `Copy`, and that is what [`Tree::apply`]'s whole-or-nothing rests on rather
/// than a rollback log. The module comment argues why this tree may afford what
/// `f_scene`'s arena may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tree {
    /// The nodes, in no particular order. Identity is the key and
    /// [`Node::rank`] is the order; the array is storage.
    nodes: [Option<Node>; NODES_MAX],
    /// Frames that closed into this tree. Unit: frames.
    frames: u64,
    /// Entries that reached it. Unit: entries.
    edits: u64,
    /// Frames this tree refused. Unit: frames.
    refused: u64,
}

impl Tree {
    /// A tree with nothing in it.
    pub const EMPTY: Self = Self { nodes: [None; NODES_MAX], frames: 0, edits: 0, refused: 0 };

    /// The node that answers to `id`, if the tree holds one.
    ///
    /// **Addressing, and the reason the exit says *addressable* rather than
    /// *readable*.** A tree read back by walking its storage is a dump; a tree
    /// whose nodes answer to the identifiers their author chose is one an
    /// automation written against the live application still works on. RFC 0077's
    /// fourth rule — identity is stable and author-assigned — is what makes that
    /// possible, and this function is where the system spends it.
    #[must_use]
    pub fn node(&self, id: u64) -> Option<&Node> {
        self.nodes.iter().flatten().find(|node| node.id == id)
    }

    /// How many nodes the tree holds. Unit: nodes.
    #[must_use]
    pub fn live(&self) -> usize {
        self.nodes.iter().flatten().count()
    }

    /// Frames that closed into this tree. Unit: frames.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// Entries that reached it. Unit: entries.
    #[must_use]
    pub const fn edits(&self) -> u64 {
        self.edits
    }

    /// Frames it refused. Unit: frames.
    #[must_use]
    pub const fn refused(&self) -> u64 {
        self.refused
    }

    /// Apply one closed frame, whole or not at all.
    ///
    /// `key` is taken by value and dropped. It is not read, and there is nothing
    /// in it to read: `f_abi::semantic::Sealed`'s own comment refuses to carry a
    /// frame token precisely so that a receiver cannot check the key instead of
    /// holding it. What demanding it buys is that this function is unreachable
    /// from a caller whose commit never arrived.
    ///
    /// # Errors
    ///
    /// A [`Rejected`], and the tree is then exactly what it was — including its
    /// counters, apart from [`Tree::refused`], which is the one number a refusal
    /// moves. A caller that wants to know which entry earned it reads
    /// [`Rejected::node`].
    pub fn apply(&mut self, staged: &Staged, key: Sealed) -> Result<Applied, Rejected> {
        // Taken by value and never read. `Sealed` is not a `Drop` type — there
        // is nothing in it to release — so this binding is what consumes it, and
        // it is named rather than pattern-ignored in the signature so that a
        // reader grepping for the key lands on a parameter with a name.
        let _consumed: Sealed = key;
        let mut trial = *self;
        let mut applied = Applied::ZERO;
        for delta in staged.entries() {
            if let Err(why) = trial.one(delta, &mut applied) {
                // The counter that moves on the failing path, on the tree the
                // caller keeps rather than on the copy that is about to be
                // dropped. A refusal that left every number where it was would
                // be a refusal a reader of the tree could not see happened.
                self.refused += 1;
                return Err(why);
            }
        }
        trial.frames += 1;
        trial.edits += staged.len() as u64;
        *self = trial;
        Ok(applied)
    }

    /// One entry, into the trial copy.
    fn one(&mut self, delta: &Delta, applied: &mut Applied) -> Result<(), Rejected> {
        match delta.body {
            Entry::DeclareVocabulary(_) => {
                applied.agreements += 1;
                Ok(())
            }
            Entry::DeclareNode(declare) => {
                self.declare(&declare)?;
                applied.declared += 1;
                Ok(())
            }
            Entry::SetState(set) => {
                let node = self.find_mut(set.node).ok_or(Rejected::NoSuchNode(set.node))?;
                node.state = Some(set.state);
                applied.stated += 1;
                Ok(())
            }
            Entry::Remove(remove) => {
                applied.removed += self.remove(remove.node)?;
                Ok(())
            }
            Entry::SetContent(set) => Err(Rejected::NotStored(set.node)),
            Entry::SetRelations(set) => Err(Rejected::NotStored(set.node)),
            // Refused on the way into [`Staged`] as well, and here too rather
            // than with a wildcard: the two refusals are one rule asked at the
            // two places an entry can arrive, and a match arm that fell through
            // would be the escape hatch this vocabulary was closed to prevent.
            Entry::Commit(_) => Err(Rejected::NotAnEdit),
        }
    }

    /// Introduce a node.
    fn declare(&mut self, declare: &DeclareNode) -> Result<(), Rejected> {
        if self.node(declare.node).is_some() {
            return Err(Rejected::Declared(declare.node));
        }
        if declare.parent != NO_NODE && self.node(declare.parent).is_none() {
            return Err(Rejected::NoSuchParent(declare.parent));
        }
        let rank = self.rank_for(declare.parent, declare.before)?;
        let slot = self.nodes.iter_mut().find(|slot| slot.is_none()).ok_or(Rejected::Capacity)?;
        *slot = Some(Node {
            id: declare.node,
            parent: declare.parent,
            rank,
            role: declare.role,
            intent: declare.intent,
            state: None,
        });
        Ok(())
    }

    /// Where a new child of `parent` sits, making room if it is being inserted.
    ///
    /// `before` is [`NO_NODE`] to append, which is the ordinary case and the one
    /// a reconciler produces most. Otherwise every sibling at or past the named
    /// node's rank moves up one — the arithmetic `DeclareNode::before` exists
    /// for, and the reason child order is not *the order the entries arrived*:
    /// inserting one item in the middle of a list must not be a redeclaration of
    /// every item after it.
    fn rank_for(&mut self, parent: u64, before: u64) -> Result<u32, Rejected> {
        if before == NO_NODE {
            let highest = self
                .nodes
                .iter()
                .flatten()
                .filter(|node| node.parent == parent)
                .map(|node| node.rank)
                .max();
            return Ok(highest.map_or(0, |rank| rank.saturating_add(1)));
        }
        let sibling = self.node(before).ok_or(Rejected::NoSuchNode(before))?;
        if sibling.parent != parent {
            return Err(Rejected::NotASibling(before));
        }
        let at = sibling.rank;
        for node in self.nodes.iter_mut().flatten() {
            if node.parent == parent && node.rank >= at {
                node.rank = node.rank.saturating_add(1);
            }
        }
        Ok(at)
    }

    /// Remove a node and everything under it, answering how many left.
    ///
    /// Written as a sweep that repeats until nothing more falls, rather than as
    /// a recursion: a component declares the tree and `f_interface::node`'s
    /// `DEPTH_MAX` is its bound, not this receiver's, so a recursion here would
    /// put a peer's number on the frame's stack. The sweep is bounded by
    /// [`NODES_MAX`], which is this receiver's own.
    /// Unit: nodes.
    fn remove(&mut self, id: u64) -> Result<u64, Rejected> {
        if self.node(id).is_none() {
            return Err(Rejected::NoSuchNode(id));
        }
        let mut gone = 0;
        // One pass takes the named node; each later pass takes whatever it
        // orphaned. At most one generation falls per pass, so [`NODES_MAX`]
        // passes is a chain the whole depth of the tree, and the bound is this
        // receiver's own rather than a depth a peer chose.
        for _ in 0..NODES_MAX {
            // The census the pass judges against, taken before it starts.
            // Judging against the array being edited would let a node fall
            // because its parent fell *in the same pass*, which is the same
            // answer reached by a rule that depends on slot order — and a rule
            // that depends on slot order is a tree whose shape depends on where
            // the allocator put things.
            let standing = self.nodes;
            let mut fell = false;
            for slot in &mut self.nodes {
                let falls = slot.is_some_and(|node| {
                    node.id == id || (node.parent != NO_NODE && !holds(&standing, node.parent))
                });
                if falls {
                    *slot = None;
                    gone += 1;
                    fell = true;
                }
            }
            if !fell {
                break;
            }
        }
        Ok(gone)
    }

    /// The node that answers to `id`, to be changed.
    fn find_mut(&mut self, id: u64) -> Option<&mut Node> {
        self.nodes.iter_mut().flatten().find(|node| node.id == id)
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Does this set of slots hold a node with that identifier?
///
/// A free function because [`Tree::remove`] needs the answer while it holds a
/// mutable borrow of one slot, and a method would borrow the whole tree twice.
fn holds(nodes: &[Option<Node>; NODES_MAX], id: u64) -> bool {
    nodes.iter().flatten().any(|node| node.id == id)
}
