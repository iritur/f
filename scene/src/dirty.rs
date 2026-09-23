// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a frame marked, and the only route from that record to the nodes an
//! encoder is allowed to see.
//!
//! Section 07 of `docs/design/ring-scene-boot.html` puts the scene on the
//! system's side of the boundary so a client sends differences instead of
//! pixels. That argument buys nothing if the compositor then re-encodes the
//! whole graph every frame: the client would have sent one delta and the encode
//! stage would have walked a thousand nodes, and the crossing the design counts
//! would have been traded for a traversal nobody counts. This module is where
//! the saving is actually taken. A frame's deltas mark subtrees, the commit
//! seals what they marked, and the encode stage walks that — and is handed
//! nothing else.
//!
//! [`crate::arena`] deliberately has no part in this. Its size assertion names
//! *a dirty set* as one of the things that would make its one fixed allocation
//! two, so the marks live here, in a structure a caller places where it likes
//! and which names nodes by the client's own identifiers rather than by the
//! arena's private slots.
//!
//! # What one delta changes
//!
//! Two rules and a silence, and [`REACH`] is the table that states them per
//! opcode rather than a match somewhere deciding case by case.
//!
//! - **A property delta marks the subtree rooted at the node it names.** Not
//!   the node alone. A transform, a clip and a layer are context for everything
//!   beneath them, so a changed transform changes where every descendant lands;
//!   the four `Family::Context` kinds are most of the six for that reason. A
//!   paint change on a leaf marks a subtree of one, which costs nothing, and
//!   the uniform rule means the encode stage never depends on a per-kind damage
//!   table that a seventh kind would silently fall out of.
//! - **A structural delta marks the subtree enclosing the change** — the parent
//!   the node is hung under, or was hung under. Paint order is sibling order,
//!   so inserting, moving or removing a node changes what occludes what among
//!   every sibling after it, and re-encoding only the moved node would leave the
//!   picture with the old occlusion in it. A move marks both ends: the parent it
//!   left and the parent it arrives under, because the hole and the fill are two
//!   different regions. When the parent is [`NO_NODE`] the sibling list is the
//!   forest of roots, and the whole scene is marked — there is no node to name.
//! - **A commit marks nothing.** It is not an empty arm: closing the frame is
//!   what [`Dirty::at_commit`] does with the whole set, and a commit that also
//!   marked something would be a commit that dirtied the scene it is publishing.
//!
//! `SET_EFFECT` is a property delta by the first rule and it is worth saying
//! why, because the declaration it carries is two costs and not a mark on the
//! screen. What the declaration decides is whether `E3-B07b`'s policy degrades
//! this node when the frame is late, and degrading it changes the picture over
//! its whole subtree — so a declaration that changed and was not re-encoded is
//! a subtree drawn against last frame's answer to *what may be dropped*. The
//! cheaper reading, that a cost is metadata and dirties nothing, is only true
//! in the frames where nothing is late, which are the frames this module is
//! not for. *What would reverse this:* the declaration ceasing to be read at
//! encode time — a policy that consulted it after the encoder had run would
//! make this row `Reach::Nothing`, and the commit would stop being the one
//! opcode that marks nothing.
//!
//! The cost of the structural rule is real and stated: inserting one node under
//! a parent with a thousand children marks a thousand and one. *What would
//! reverse this:* bounds per node, at which point the mark can be a rectangle
//! and only the siblings that actually overlap it are dirty. That is a
//! different structure — damage regions rather than dirty subtrees — and it is
//! an RFC, not an edit to this rule.
//!
//! # Marks are an antichain, and that is what makes the count an equality
//!
//! The number this module is accepted on is an equality: the encoder visits
//! *exactly* the nodes of the marked subtrees. An upper bound would be satisfied
//! by a walker that visits everything, which is precisely the walker this
//! module exists to replace, so the interesting half of the property is that no
//! node is visited twice.
//!
//! That holds because the set of marked roots is kept an antichain — no marked
//! root is an ancestor of another — and disjoint subtrees of a tree share no
//! node. It is enforced twice, and the second time is the one that matters:
//!
//! - **At mark time**, a node already inside a marked subtree is subsumed and
//!   adds nothing, and a node that encloses existing marks sheds them. This
//!   keeps the list short, which is the reason it is done at all.
//! - **At walk time**, every marked root that any other marked root now
//!   encloses is skipped. Mark-time subsumption is a statement about the tree as
//!   it was when the mark was taken, and a later delta in the same frame can
//!   move one marked subtree inside another — at which point a set that was
//!   disjoint when it was written down is not disjoint when it is walked. The
//!   walk asks about the tree it is actually walking, so the equality survives
//!   an edit order nobody thought of.
//!
//! # What *allowed to walk* means here, exactly
//!
//! Stated plainly, because the word does more work in some designs than it can
//! do in this one.
//!
//! **What is structural.** [`Visit`] is the only thing this module hands out,
//! it has no public constructor, and there is exactly one expression in the
//! crate that builds one — inside `walk_subtree`, in the same statement that
//! increments the tally. So the count [`Sealed::walk`] answers with is not a
//! measurement taken beside the walk that could drift from it: it is the number
//! of times a `Visit` came into existence, and a `Visit` for a clean node cannot
//! be made. An encode function whose parameter is a `Visit` has no arena, no
//! node identifier it was not given, and no children to descend into; it is
//! structurally unable to reach a clean node, because reaching one requires an
//! arena and it has none.
//!
//! **What is not.** [`Sealed::walk`] takes `&Arena`, so its *caller* holds the
//! graph and Rust has no way to make one borrow forbid another. Nothing here
//! stops a consumer that also holds the arena from walking it. The guarantee is
//! therefore exactly as wide as the encoder's signature: `E3-B02c` should take
//! [`Visit`] and nothing else, and then *only what was marked is encoded* is a
//! type. If it takes `&Arena` as well, what remains of this paragraph is a
//! convention, and a convention is what the next edit walks past. Saying so is
//! better than implying a guarantee the borrow checker does not make.
//!
//! # Two states, and why the transition costs a move
//!
//! [`Dirty`] accepts marks and cannot be walked. [`Sealed`] can be walked and
//! cannot be marked. The only route between them is [`Dirty::at_commit`], which
//! takes `self` by value, and [`Sealed::reopen`], which does the same going
//! back. *A commit marks, and the encode stage then walks* is the ordering the
//! task names, and here it is the shape of the types rather than a rule in a
//! comment: there is no `walk` to call on an unsealed set and no `mark` to call
//! on a sealed one, so neither mistake has an expression that compiles.
//!
//! The price is that the transition moves the mark array. It is
//! [`MARKS_MAX`] words — a quarter of a kilobyte — once per frame, against a
//! state that cannot be in both phases at once. *What would reverse this:* a
//! measured frame in which that move shows against section 12's budget, at
//! which point the two states become one type with a flag and this paragraph
//! becomes the argument that was traded away.
//!
//! # Coarsening, not refusing
//!
//! The mark list holds [`MARKS_MAX`] disjoint subtrees. A frame that scatters
//! more changes than that does not get a refusal: the whole scene is marked
//! instead. That direction is chosen because over-drawing is a slow frame and
//! under-drawing is a wrong picture, and because a refusal here would have to be
//! handled by a client that did nothing wrong — it sent legal deltas. The same
//! answer covers every other case this module cannot reason about: a parent
//! chain longer than the arena can hold means the graph is already wrong, and
//! marking everything is the one answer that cannot be a stale region.
//!
//! [`MARKS_MAX`] is sixty-four because the alternative is a dirty set the size
//! of the scene, which is a second copy of the graph held to avoid walking the
//! first. *What would reverse this:* a measured frame that coarsens often enough
//! to matter, at which point the number moves and nothing else here does.
//!
//! # What this costs
//!
//! Every lookup in this module goes through `Arena`'s public accessors, and
//! each of those is a scan of every slot — `Arena::find`'s *the cost is the
//! search*. So a mark costs `marks × depth` of those scans and a visit costs a
//! handful. That is stated rather than hidden: this module gets cheaper by
//! exactly the factor a key-to-slot index would give the arena, and it does not
//! get its own, because a second index is a second structure that can disagree
//! with the tree. *What would reverse this:* the index the arena's own note
//! already names the condition for.
//!
//! # No clock, no randomness, no float, no allocator
//!
//! Every function here is a pure function of the set it is given and the graph
//! it is handed, which is a stronger determinism statement than a seeded one:
//! there is nothing to draw and nothing to observe. The walk's order is paint
//! order for the subtree and mark order for the list, both of which are the
//! client's own, so there is no iteration order for a seed to fix — and no
//! `HashMap`, because there is no map. No binary floating point: everything
//! here is a node identifier or a count. No allocation: the mark list is an
//! array whose length is a compile-time constant, and the walk carries no stack
//! at all — it climbs the arena's own parent links, which is what keeps a tree
//! whose depth the client chose from being a stack the compositor has to have.
//! RFC 0004.

// The same five as `crate::arena`, for the same reason: this module is
// reachable from whatever a client submits, `panic = "abort"` is set in every
// profile, and a panic here is the screen going dark. Outside the tests,
// `unwrap`, `expect`, `panic!`, `unreachable!` and a subscript that could be out
// of bounds do not compile, so *no panicking path* is a property of the build
// rather than a habit of whoever edits next.
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

use f_abi::scene::{Delta, Entry, NO_NODE, SetPaint, SetPath, SetTransform, op};

use crate::arena::{Applied, Arena, NODES_MAX, Refusal};
use crate::kind::Kind;

/// The most disjoint subtrees one frame records before the whole scene is
/// marked instead.
///
/// Sixty-four: a frame of scattered edits, and the point past which the set is
/// no longer cheaper than the thing it saves. The module's *coarsening, not
/// refusing* is the argument and the reversal condition.
/// Unit: subtree roots.
pub const MARKS_MAX: usize = 64;

/// How far one opcode's change reaches into the graph.
///
/// The policy, as a value, so that it is stated once and read rather than
/// decided at each call site. [`REACH`] is where each opcode's answer lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The node the record names, and everything under it.
    ///
    /// The property deltas. A node's transform, geometry and paint are context
    /// for its descendants, so changing one changes the subtree.
    Subtree,
    /// The subtree enclosing the node the record names: its parent, and
    /// everything under that.
    ///
    /// A removal. The node itself is about to be gone, so marking it would mark
    /// nothing by the time the encoder walks; what has changed is the sibling
    /// list it was in, and that is the parent's subtree.
    Enclosing,
    /// Both ends of a placement: the parent the record states, and the parent
    /// the node is under now.
    ///
    /// A create is one of these with only the first end, because the node is
    /// not in the tree yet. A create of a node that already exists is a move —
    /// `Arena::hang`'s reading — and a move dirties the hole as well as the
    /// fill.
    Placement,
    /// Nothing in the graph.
    ///
    /// The commit, and a real answer rather than an empty arm: closing a frame
    /// is [`Dirty::at_commit`]'s to do with the whole set, and a commit that
    /// marked something would dirty the frame it is publishing.
    Nothing,
}

/// What each opcode's change reaches, and the whole of this module's policy.
///
/// A table and not a match, on `crate::kind`'s argument for [`ByKind`]: an
/// exhaustive match demands an arm and not an arm that *says* anything, and
/// `Entry::Commit(_) => {}` compiles. A table has no empty arm to write — it is
/// `[_; op::COUNT]`, so a seventh opcode in `f_abi::scene` makes this literal
/// the wrong length and stops the build here, in the same afternoon, asking
/// what the new opcode dirties.
///
/// Each row names its opcode rather than relying on position, and the assertion
/// below checks the names against `op::ALL` in order — so reordering the wire's
/// list is a build error too, rather than a table that silently shifted by one.
///
/// [`ByKind`]: crate::kind::ByKind
/// Unit: none — one row per opcode, in declaration order.
pub const REACH: [(u8, Reach); op::COUNT] = [
    (op::CREATE_NODE, Reach::Placement),
    (op::SET_TRANSFORM, Reach::Subtree),
    (op::SET_PATH, Reach::Subtree),
    (op::SET_PAINT, Reach::Subtree),
    (op::REMOVE_NODE, Reach::Enclosing),
    (op::COMMIT, Reach::Nothing),
    (op::SET_EFFECT, Reach::Subtree),
];

// The table and the wire's opcode list are one list, checked where a
// disagreement is a build and not a behaviour. The length is the type's to
// enforce; this is the order, which a type cannot.
//
// The subscripts are the one place in this file that indexes rather than asks:
// both arrays are `op::COUNT` long and the cursor is below `op::COUNT`, so
// there is no out-of-bounds case for these expressions to have — and in a const
// block a mistake about that is a compile error rather than a running
// compositor's.
#[cfg_attr(not(test), allow(clippy::indexing_slicing))]
const _: () = {
    let mut at = 0;
    while at < op::COUNT {
        assert!(
            REACH[at].0 == op::ALL[at],
            "f_scene::dirty::REACH and f_abi::scene::op::ALL name different opcodes"
        );
        at += 1;
    }
};

/// How far a change to this opcode reaches, or `None` for an opcode this build
/// does not know.
///
/// `None` is kept distinct from [`Reach::Nothing`] for the reason
/// `op::carries_deadline` keeps its own: *this opcode dirties nothing* and *this
/// is not an opcode* are different facts, and a caller that collapsed them would
/// treat an unknown opcode as a clean one and publish a frame missing whatever
/// it did.
#[must_use]
pub fn reach_of(opcode: u8) -> Option<Reach> {
    REACH.iter().find(|(code, _)| *code == opcode).map(|(_, reach)| *reach)
}

/// What recording a change did to the set.
///
/// Ordered by how much of the scene the answer says is dirty, ascending, so
/// that two answers about one delta compose with `max` rather than with a rule
/// about which of them wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Marked {
    /// Nothing was marked.
    ///
    /// A commit, or a delta naming a node the scene does not hold. The second
    /// is not an error: a delta the arena will refuse changes nothing, so there
    /// is nothing to re-encode, and refusing it here as well would be this
    /// module restating a rule `crate::arena` already states — which is how the
    /// two halves of a system come to disagree.
    Nothing,
    /// The node is already inside a marked subtree, so the set is unchanged.
    Subsumed,
    /// A new subtree root went into the set.
    Added,
    /// The whole scene is marked, and the individual roots are gone.
    ///
    /// The forest was named, the list was full, or the graph gave an answer
    /// this module will not reason about. The module's *coarsening, not
    /// refusing* is why this is an answer and not a refusal.
    Whole,
}

/// The mark list itself, shared by the two states so that neither can hold a
/// second copy of it.
#[derive(Clone, Copy, Debug)]
struct Marks {
    /// The marked subtree roots, in the order they were marked.
    ///
    /// Only the first `len` are meaningful. Client identifiers, not slots:
    /// nothing here may depend on the arena's private indexing, and a mark
    /// outliving a node is handled by the walk finding nothing there.
    roots: [u32; MARKS_MAX],
    /// How many of `roots` are meaningful.
    /// Unit: subtree roots.
    len: usize,
    /// Is the whole scene marked?
    ///
    /// When it is, `len` is zero: the individual roots are not kept beside a
    /// flag that supersedes them, because two answers to *what is dirty* is one
    /// answer too many.
    whole: bool,
}

impl Marks {
    /// Nothing marked.
    const CLEAN: Self = Self { roots: [NO_NODE; MARKS_MAX], len: 0, whole: false };

    /// The marked roots, in the order they were marked.
    fn each(&self) -> impl Iterator<Item = u32> + '_ {
        self.roots.iter().take(self.len).copied()
    }

    /// Put a root in the list, or answer `false` when there is no room.
    ///
    /// `get_mut` and not a subscript, and the `None` arm is a real case rather
    /// than one a reader has to argue is unreachable: it is exactly *the list is
    /// full*, and the caller turns it into [`Marked::Whole`].
    fn push(&mut self, node: u32) -> bool {
        match self.roots.get_mut(self.len) {
            Some(slot) => {
                *slot = node;
                self.len += 1;
                true
            }
            None => false,
        }
    }

    /// Is this node already inside one of the marked subtrees?
    fn covered(&self, arena: &Arena, node: u32) -> Ancestry {
        let mut answer = Ancestry::Separate;
        for root in self.each() {
            match ancestry(arena, root, node) {
                Ancestry::Covers => return Ancestry::Covers,
                Ancestry::Broken => answer = Ancestry::Broken,
                Ancestry::Separate => {}
            }
        }
        answer
    }

    /// Drop every mark that this node encloses, and say whether the graph gave
    /// an answer this module will not reason about.
    ///
    /// The list is compacted in place rather than leaving holes, because a hole
    /// would be a second thing `len` has to mean.
    fn shed(&mut self, arena: &Arena, ancestor: u32) -> bool {
        let mut kept = 0;
        let mut broken = false;
        for at in 0..self.len {
            let Some(&one) = self.roots.get(at) else { continue };
            match ancestry(arena, ancestor, one) {
                // Covered by the incoming mark: its subtree is inside that one,
                // so keeping it would be the double visit the module's
                // *antichain* section is about.
                Ancestry::Covers => {}
                Ancestry::Separate => {
                    if let Some(slot) = self.roots.get_mut(kept) {
                        *slot = one;
                        kept += 1;
                    }
                }
                Ancestry::Broken => broken = true,
            }
        }
        self.len = kept;
        broken
    }

    /// Does another mark enclose the one at this position, as the tree stands
    /// now?
    ///
    /// The walk-time half of the antichain, and the half that survives a frame
    /// that moved one marked subtree inside another after both were marked. It
    /// climbs this mark's own parent chain once and asks, at each step, whether
    /// any other mark is standing there — so the cost is one ascent per mark
    /// rather than a comparison of every pair.
    ///
    /// At the mark itself the question is only about an earlier duplicate:
    /// a root does not shadow itself, and of two marks naming one node exactly
    /// one must walk.
    fn shadowed(&self, arena: &Arena, at: usize) -> bool {
        let Some(&node) = self.roots.get(at) else { return true };
        let mut cursor = node;
        let mut steps = 0;
        loop {
            for (other, root) in self.each().enumerate() {
                if root == cursor && other != at && (steps > 0 || other < at) {
                    return true;
                }
            }
            if steps >= NODES_MAX {
                // More steps than the arena has slots: the parent chain is a
                // cycle the arena promises it refuses. Answering *shadowed*
                // declines to walk this mark, and the walk's own budget is what
                // keeps the other marks from running forever.
                return true;
            }
            steps += 1;
            match arena.parent_of(cursor) {
                Ok(NO_NODE) | Err(_) => return false,
                Ok(parent) => cursor = parent,
            }
        }
    }
}

/// What this frame has changed so far: the open set, which accepts marks and
/// cannot be walked.
///
/// The module's *two states* is the argument for why that second clause is a
/// missing method rather than a rule. Neither `Clone` nor `Copy`: a frame's
/// record of what changed is a place, and two of them are two answers.
#[derive(Debug)]
pub struct Dirty {
    /// The marks taken so far this frame.
    marks: Marks,
}

impl Dirty {
    /// A frame that has changed nothing.
    ///
    /// A `const` rather than a `new()`, on `Arena::EMPTY`'s terms: it can
    /// initialise a `static`, which is where a compositor's per-client record
    /// actually lives.
    pub const CLEAN: Self = Self { marks: Marks::CLEAN };

    /// How many subtree roots are marked.
    ///
    /// Zero when the whole scene is marked, which is [`Dirty::is_whole`]'s
    /// answer and not this one's: the roots are gone, not summarised.
    /// Unit: subtree roots.
    #[must_use]
    pub const fn marked(&self) -> usize {
        self.marks.len
    }

    /// Is the whole scene marked?
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.marks.whole
    }

    /// Is there nothing to encode?
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.marks.len == 0 && !self.marks.whole
    }

    /// Mark the subtree rooted at this node.
    ///
    /// The primitive. A node the scene does not hold roots no subtree, so it
    /// marks nothing — see [`Marked::Nothing`] for why that is not a refusal.
    /// [`NO_NODE`] names the forest, which is `f_abi::scene::CreateNode::parent`'s
    /// own reading of that value, and marks the whole scene.
    ///
    /// **A mark names a root, not a set of nodes.** What is under that root is
    /// decided when the encode stage walks, not now, which is what makes it
    /// correct to mark before the arena has applied the delta that caused the
    /// change.
    pub fn mark(&mut self, arena: &Arena, node: u32) -> Marked {
        if node == NO_NODE {
            return self.everything();
        }
        if self.marks.whole {
            return Marked::Whole;
        }
        if !arena.holds(node) {
            return Marked::Nothing;
        }
        match self.marks.covered(arena, node) {
            Ancestry::Covers => return Marked::Subsumed,
            Ancestry::Broken => return self.everything(),
            Ancestry::Separate => {}
        }
        if self.marks.shed(arena, node) {
            return self.everything();
        }
        if self.marks.push(node) { Marked::Added } else { self.everything() }
    }

    /// Mark the whole scene, and forget the individual roots.
    fn everything(&mut self) -> Marked {
        self.marks.len = 0;
        self.marks.whole = true;
        Marked::Whole
    }

    /// Mark what this delta changes, **before** the arena applies it.
    ///
    /// [`REACH`] is the policy and this is its application: the table says how
    /// far the opcode's change reaches, and the match below says only which
    /// node the record names, so there is one statement of the rule and one of
    /// the operand.
    ///
    /// # The order, and what the two orders cost
    ///
    /// Before, for every opcode, and [`Dirty::apply`] exists so that a caller
    /// need not remember it. Called after the arena has applied the delta, a
    /// removal marks nothing — its parent is no longer reachable from a node
    /// that is gone — and a move marks the parent it arrived under twice
    /// instead of the parent it left once, which is a hole left with last
    /// frame's picture in it. This module cannot make that mistake a compile
    /// error: it would have to be the thing that drives the arena, and `E3-B01d`
    /// owns what a commit does to the graph. So it is said here plainly, and
    /// [`Dirty::apply`] is where the ordering is written down once.
    pub fn record(&mut self, arena: &Arena, delta: &Delta) -> Marked {
        let Some(reach) = reach_of(delta.opcode()) else {
            // Unreachable while `REACH` covers `op::ALL`, and the const
            // assertion under the table is what keeps that true — a seventh
            // opcode stops the build rather than arriving here. If it is
            // reached anyway, the opcode is one whose damage nobody has
            // decided, and the whole scene is the only answer that cannot leave
            // a stale region on a screen.
            return self.everything();
        };
        // Which node the record names, and where a placement says to put it.
        // The arms are per opcode with no wildcard, so a seventh stops the
        // build here as well as at the table: one of the two would be enough,
        // and both is what it costs to have neither be the only one.
        let (node, placed_under) = match delta.body {
            Entry::CreateNode(at) => (at.node, Some(at.parent)),
            Entry::SetTransform(set) => (set.node, None),
            Entry::SetPath(set) => (set.node, None),
            Entry::SetPaint(set) => (set.node, None),
            Entry::RemoveNode(gone) => (gone.node, None),
            Entry::Commit(_) => (NO_NODE, None),
            Entry::SetEffect(set) => (set.node, None),
        };

        match reach {
            Reach::Nothing => Marked::Nothing,
            Reach::Subtree => self.mark(arena, node),
            Reach::Enclosing => match arena.parent_of(node) {
                Ok(parent) => self.mark(arena, parent),
                // The scene does not hold it, so the arena will refuse the
                // delta and the picture does not change.
                Err(_) => Marked::Nothing,
            },
            Reach::Placement => {
                let fill = match placed_under {
                    Some(parent) => self.mark(arena, parent),
                    // A reach of `Placement` on an opcode whose record states no
                    // parent: the whole scene, which is the answer that cannot
                    // be wrong about a placement nobody can locate.
                    None => self.everything(),
                };
                // The other end, and only a move has one: a node already in the
                // tree leaves a hole where it was.
                let hole = match arena.parent_of(node) {
                    Ok(parent) => self.mark(arena, parent),
                    Err(_) => Marked::Nothing,
                };
                fill.max(hole)
            }
        }
    }

    /// Mark what this delta changes and then apply it to the graph.
    ///
    /// The ordering [`Dirty::record`] describes, written once so that a caller
    /// driving the arena delta by delta cannot invert it. A delta the arena then
    /// refuses has cost a mark, which is a subtree re-encoded and nothing else —
    /// the safe direction, chosen deliberately, because the other order loses
    /// the parent of a node that is about to stop having one.
    ///
    /// A caller whose commit stage stages deltas rather than applying them —
    /// which is what `f_abi::scene`'s *apply every delta since the last commit,
    /// or none* asks for — cannot use this, and should call [`Dirty::record`]
    /// before it stages and honour the order itself.
    ///
    /// # Errors
    ///
    /// Whatever `Arena::apply` refuses, unchanged. Nothing is re-decided here.
    pub fn apply(&mut self, arena: &mut Arena, delta: &Delta) -> Result<Applied, Refusal> {
        self.record(arena, delta);
        arena.apply(delta)
    }

    /// Close the frame: hand back the set the encode stage may walk.
    ///
    /// By value, and that is the whole mechanism. The marks stop being markable
    /// at exactly the moment they become walkable, because the type that could
    /// do the first is consumed producing the type that can do the second.
    ///
    /// The token is `f_abi::scene::Commit::frame_token`, carried so that a
    /// consumer encoding this plan can say which frame it encoded without being
    /// handed the commit a second time.
    #[must_use]
    pub const fn at_commit(self, frame_token: u64) -> Sealed {
        Sealed { marks: self.marks, frame_token }
    }
}

/// What a commit sealed: the plan the encode stage is allowed to walk.
///
/// No `mark` and no `record`. The module's *what allowed to walk means* is how
/// far that goes and where it stops.
#[derive(Debug)]
pub struct Sealed {
    /// The marks the frame took.
    marks: Marks,
    /// The frame this plan belongs to.
    /// Unit: none — a frame identifier, not a quantity.
    frame_token: u64,
}

impl Sealed {
    /// The frame this plan belongs to.
    /// Unit: none — a frame identifier, not a quantity.
    #[must_use]
    pub const fn frame_token(&self) -> u64 {
        self.frame_token
    }

    /// How many subtree roots this plan holds. Zero when the whole scene is
    /// marked.
    /// Unit: subtree roots.
    #[must_use]
    pub const fn marked(&self) -> usize {
        self.marks.len
    }

    /// Is the whole scene marked?
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.marks.whole
    }

    /// Is there nothing to encode?
    ///
    /// The answer a compositor acts on before anything else: a frame that
    /// dirtied nothing is a frame with no encode stage at all, which is the
    /// cheapest thing this module can do for a client that committed without
    /// changing anything.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.marks.len == 0 && !self.marks.whole
    }

    /// Hand the encode stage every node it may see, and answer how many that
    /// was.
    ///
    /// Preorder within each marked subtree — paint order, because that is what
    /// `Arena::children` is in — and mark order between them. Preorder plus
    /// [`Visit::depth`] is a tree, so an encoder can rebuild the structure from
    /// the flat sequence without asking the graph anything.
    ///
    /// The tally is not an observation of the walk. [`Visited::nodes`] is
    /// incremented in the one statement that constructs a [`Visit`], so it is
    /// the number of nodes that were handed over, and there is no second count
    /// for it to drift from.
    ///
    /// A marked root whose node a later delta removed contributes nothing: the
    /// mark named a root, and there is no longer a subtree there.
    pub fn walk<F>(&self, arena: &Arena, mut each: F) -> Visited
    where
        F: FnMut(Visit<'_>),
    {
        let mut tally = Visited { nodes: 0, subtrees: 0, halted: false };
        if self.marks.whole {
            // Every root of the forest. The roots are disjoint by construction
            // — a node has one parent — so this needs no shadow check.
            for root in arena.roots() {
                walk_subtree(arena, root, &mut each, &mut tally);
            }
            return tally;
        }
        for (at, root) in self.marks.each().enumerate() {
            if self.marks.shadowed(arena, at) {
                continue;
            }
            walk_subtree(arena, root, &mut each, &mut tally);
        }
        tally
    }

    /// Open a fresh set for the next frame.
    ///
    /// By value, so the plan cannot be walked after the frame that owned it has
    /// been let go. The marks are dropped rather than carried: a node that is
    /// still dirty after its frame was encoded is a node whose delta arrived in
    /// the next frame, and it will mark itself then.
    #[must_use]
    pub const fn reopen(self) -> Dirty {
        Dirty::CLEAN
    }
}

/// One node the encode stage is allowed to encode.
///
/// The only thing this module hands out, and it has no public constructor:
/// `walk_subtree` holds the single expression in the crate that builds one.
/// An encoder whose input is this type has no arena, so the nodes it can reach
/// are the nodes it was given — which is as far as *allowed* goes here, and the
/// module says where that stops.
///
/// The property records come back on `Arena`'s own terms — `None` for a node no
/// delta has set one on, rather than an identity this module would have had to
/// invent, since `crate::reconcile` states what absence means and a second
/// statement of it is a second thing that can disagree.
pub struct Visit<'a> {
    /// The graph the node is in. Private, and that is the point of the type.
    arena: &'a Arena,
    /// The node.
    node: u32,
    /// Its kind, read once when the visit was minted.
    kind: Kind,
    /// How far below the marked root it sits.
    depth: u32,
}

impl Visit<'_> {
    /// The node this visit is about.
    /// Unit: none — a node identifier, not a quantity.
    #[must_use]
    pub const fn node(&self) -> u32 {
        self.node
    }

    /// What kind of node it is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// How far below the marked root this node sits — zero for the root itself.
    ///
    /// With preorder, this is the whole of the structure: a visit at depth `d`
    /// is a child of the last visit at depth `d - 1`.
    /// Unit: edges.
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.depth
    }

    /// The transform a delta set on this node, or `None` if none has.
    /// Unit: none — 16.16 fixed point, the scale in each field's name.
    #[must_use]
    pub fn transform(&self) -> Option<SetTransform> {
        self.arena.transform_of(self.node).ok().flatten()
    }

    /// The geometry a delta pointed this node at, or `None` if none has.
    /// Unit: bytes, from the first byte of the channel's inline arena.
    #[must_use]
    pub fn path(&self) -> Option<SetPath> {
        self.arena.path_of(self.node).ok().flatten()
    }

    /// What a delta said to paint this node with, or `None` if none has.
    /// Unit: none — intensities scaled so that 65 535 is full.
    #[must_use]
    pub fn paint(&self) -> Option<SetPaint> {
        self.arena.paint_of(self.node).ok().flatten()
    }
}

/// What one walk did.
///
/// A count and not a sample. The module's exit is an equality against the size
/// of the marked subtrees, and an equality is only worth stating about a number
/// that is the walk itself rather than a measurement beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visited {
    /// How many nodes were handed to the encoder.
    /// Unit: nodes.
    pub nodes: usize,
    /// How many marked subtrees were actually walked.
    ///
    /// Fewer than [`Sealed::marked`] when a mark was shadowed by another or its
    /// node had been removed by the time the frame closed.
    /// Unit: subtrees.
    pub subtrees: usize,
    /// Did the walk stop early?
    ///
    /// True only when it has already handed over more nodes than the arena can
    /// hold, which means the graph contains a cycle `crate::arena` promises to
    /// refuse. Stopping is the safe direction: an incomplete frame is a picture
    /// with something stale in it, and the alternative is a compositor that
    /// never returns.
    pub halted: bool,
}

/// Where one node stands relative to another.
enum Ancestry {
    /// The ancestor is the node, or is somewhere above it.
    Covers,
    /// Neither is above the other.
    Separate,
    /// The parent chain is longer than the arena has slots, so the graph is
    /// already wrong and this module will not reason about it.
    Broken,
}

/// Is `ancestor` the node itself, or somewhere above it?
///
/// Climbs from the node, which is bounded by the depth of the tree and so by
/// [`NODES_MAX`]. The step counter is what makes that a fact rather than an
/// inference — it is the same device `Arena::descends_from` uses, and for the
/// same reason.
fn ancestry(arena: &Arena, ancestor: u32, node: u32) -> Ancestry {
    let mut at = node;
    let mut steps = 0;
    loop {
        if at == ancestor {
            return Ancestry::Covers;
        }
        if steps >= NODES_MAX {
            return Ancestry::Broken;
        }
        steps += 1;
        match arena.parent_of(at) {
            Ok(NO_NODE) | Err(_) => return Ancestry::Separate,
            Ok(parent) => at = parent,
        }
    }
}

/// This node's first child in paint order, or `None`.
fn first_child(arena: &Arena, node: u32) -> Option<u32> {
    arena.children(node).ok()?.next()
}

/// The sibling painted after this node, or `None` for the last of a list.
///
/// Read out of the arena rather than stored, which costs a scan of the sibling
/// list per step and keeps this module from holding a second copy of the tree's
/// links — the copy that would be wrong the first time a delta re-ordered
/// something between a mark and a walk.
fn next_sibling(arena: &Arena, node: u32) -> Option<u32> {
    let parent = arena.parent_of(node).ok()?;
    let mut siblings =
        if parent == NO_NODE { arena.roots() } else { arena.children(parent).ok()? };
    while let Some(one) = siblings.next() {
        if one == node {
            return siblings.next();
        }
    }
    None
}

/// Hand every node of one subtree to the encoder, in paint order.
///
/// **The one place a [`Visit`] is made.** The tally rises in the same statement,
/// so the number a caller reads is the number of nodes that were handed over and
/// not a count kept beside the walk.
///
/// No recursion and no stack. It descends by the arena's child links and climbs
/// back by its parent links, so a subtree whose depth the client chose costs
/// this function nothing but steps — `Arena::remove` refuses a scratch array
/// sized by a client's depth for the same reason, and a walk that recursed
/// would have put the same number on the compositor's stack instead.
fn walk_subtree<F>(arena: &Arena, root: u32, each: &mut F, tally: &mut Visited)
where
    F: FnMut(Visit<'_>),
{
    // A mark whose node a later delta removed: the mark named a root, and there
    // is no longer a subtree there to encode.
    if arena.kind_of(root).is_none() {
        return;
    }
    tally.subtrees += 1;

    let mut at = root;
    let mut depth = 0;
    loop {
        if tally.nodes >= NODES_MAX {
            tally.halted = true;
            return;
        }
        let Some(kind) = arena.kind_of(at) else { return };
        tally.nodes += 1;
        each(Visit { arena, node: at, kind, depth });

        if let Some(child) = first_child(arena, at) {
            at = child;
            depth += 1;
            continue;
        }
        // No children: climb until there is a sibling to take, and stop at the
        // root — which is what keeps the walk inside the subtree that was
        // marked instead of carrying on into the one beside it.
        loop {
            if at == root {
                return;
            }
            if let Some(sibling) = next_sibling(arena, at) {
                at = sibling;
                break;
            }
            match arena.parent_of(at) {
                Ok(NO_NODE) | Err(_) => return,
                Ok(parent) => {
                    at = parent;
                    depth = depth.saturating_sub(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Hung;
    use f_abi::NO_DEADLINE;
    use f_abi::scene::{CreateNode, RemoveNode};

    /// A deadline far enough from zero to be unmistakably a deadline.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// The frame the sealed plans below belong to.
    const FRAME: u64 = 0x0BAD_F00D_0000_0001;

    /// How many groups the scene of a thousand has.
    const GROUPS: u32 = 10;

    /// How many leaves hang under each group.
    const PER_GROUP: u32 = 100;

    /// The scene of a thousand, actually built: a root, ten groups, a hundred
    /// leaves under each.
    const SCENE_NODES: usize = 1 + GROUPS as usize + (GROUPS * PER_GROUP) as usize;

    /// The one root.
    const ROOT: u32 = 1;

    /// One past the largest identifier the scene uses, so that a per-node
    /// tally can be an array rather than a map this crate could not allocate.
    const IDS: usize = 2101;

    /// The identifier of a group.
    const fn group(nth: u32) -> u32 {
        10 + nth
    }

    /// The identifier of a leaf under a group.
    const fn leaf(under: u32, nth: u32) -> u32 {
        1000 + under * 100 + nth
    }

    /// Every identifier the scene of a thousand uses.
    fn every_node() -> impl Iterator<Item = u32> {
        core::iter::once(ROOT)
            .chain((1..=GROUPS).map(group))
            .chain((1..=GROUPS).flat_map(|under| (1..=PER_GROUP).map(move |nth| leaf(under, nth))))
    }

    /// A delta around a body, with an envelope that is legal for it.
    ///
    /// The deadline comes from `op::carries_deadline` rather than a branch per
    /// opcode, so a seventh opcode gets a legal envelope from the list that
    /// declared it. `crate::arena`'s tests build theirs the same way.
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

    /// The delta that hangs a node at the end of its parent's children.
    fn hang(node: u32, parent: u32, kind: Kind) -> Delta {
        delta(Entry::CreateNode(CreateNode { node, parent, before: NO_NODE, kind: kind.wire() }))
    }

    /// The delta that repaints a node, which is the smallest change there is.
    fn repaint(node: u32) -> Delta {
        delta(Entry::SetPaint(SetPaint {
            node,
            red_x65535: 0,
            green_x65535: 32_768,
            blue_x65535: 65_535,
            alpha_x65535: 65_535,
            stroke_width_x65536: 0,
        }))
    }

    /// Build the scene of a thousand, every node of it through `apply`.
    fn build_a_thousand(arena: &mut Arena) {
        let created = Ok(Applied::Hung(Hung::Created));
        assert_eq!(arena.apply(&hang(ROOT, NO_NODE, Kind::Layer)), created);
        for under in 1..=GROUPS {
            assert_eq!(arena.apply(&hang(group(under), ROOT, Kind::Transform)), created);
            for nth in 1..=PER_GROUP {
                assert_eq!(arena.apply(&hang(leaf(under, nth), group(under), Kind::Draw)), created);
            }
        }
        assert_eq!(arena.live(), SCENE_NODES);
    }

    /// Is `node` the root, or under it?
    ///
    /// The second opinion, and deliberately the other algorithm: the walk
    /// descends by child links, this ascends by parent links. Two ways of
    /// asking agreeing is worth more than either of them alone.
    fn under(arena: &Arena, root: u32, node: u32) -> bool {
        let mut at = node;
        loop {
            if at == root {
                return true;
            }
            match arena.parent_of(at) {
                Ok(NO_NODE) | Err(_) => return false,
                Ok(parent) => at = parent,
            }
        }
    }

    /// How many live nodes of the scene of a thousand are in this subtree.
    fn size_of_subtree(arena: &Arena, root: u32) -> usize {
        every_node().filter(|node| arena.holds(*node) && under(arena, root, *node)).count()
    }

    /// Walk a sealed plan, counting how many times each node was handed over.
    ///
    /// Answers the tally the walk reports, the number of times the closure was
    /// actually called, and the per-node counts — three numbers from three
    /// places, so a walk that reported a count it did not hand over fails here.
    fn tally(sealed: &Sealed, arena: &Arena) -> (Visited, usize, [u8; IDS]) {
        let mut seen = [0u8; IDS];
        let mut calls = 0;
        let visited = sealed.walk(arena, |visit| {
            calls += 1;
            if let Some(count) = seen.get_mut(visit.node() as usize) {
                *count = count.saturating_add(1);
            }
        });
        (visited, calls, seen)
    }

    #[test]
    fn one_changed_node_in_a_scene_of_a_thousand_marks_one_subtree() {
        // The exit sentence, in order and at the stated size.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        // One changed node: a group in the middle of the scene.
        let changed = group(4);
        let mut dirty = Dirty::CLEAN;
        assert!(dirty.is_clean());
        assert_eq!(dirty.record(&arena, &repaint(changed)), Marked::Added);
        assert_eq!(dirty.marked(), 1, "one changed node marked more than one subtree");
        assert!(!dirty.is_whole());

        let sealed = dirty.at_commit(FRAME);
        assert_eq!(sealed.frame_token(), FRAME);
        let (visited, calls, seen) = tally(&sealed, &arena);

        // The equality. The expected number is counted by ascending parent
        // links over every node of the scene, which is not how the walk found
        // them.
        let expected = size_of_subtree(&arena, changed);
        assert_eq!(expected, 1 + PER_GROUP as usize, "the subtree is not the size the scene says");
        assert_eq!(visited.nodes, expected, "the encoder visited more or fewer than the subtree");
        assert_eq!(calls, visited.nodes, "the tally is not the number of nodes handed over");
        assert_eq!(visited.subtrees, 1);
        assert!(!visited.halted);
        assert!(visited.nodes * 9 < SCENE_NODES, "the walk is not a saving over the whole scene");

        // And not a count that happens to match: exactly those nodes, each
        // exactly once, and no node outside the subtree at all.
        for node in every_node() {
            let want = u8::from(under(&arena, changed, node));
            assert_eq!(
                seen.get(node as usize).copied(),
                Some(want),
                "node {node} was handed over {want} time(s) too few or too many"
            );
        }
    }

    #[test]
    fn a_change_to_a_leaf_visits_one_node_of_the_thousand() {
        // The other end of the same sentence: the smallest change there is
        // costs the encoder one node, not a thousand and not a hundred.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let changed = leaf(7, 50);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.record(&arena, &repaint(changed)), Marked::Added);

        let sealed = dirty.at_commit(FRAME);
        let (visited, calls, seen) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, 1);
        assert_eq!(calls, 1);
        assert_eq!(visited.subtrees, 1);
        assert_eq!(seen.get(changed as usize).copied(), Some(1));
        assert_eq!(seen.iter().map(|count| *count as usize).sum::<usize>(), 1);
    }

    #[test]
    fn a_visit_carries_the_node_its_kind_and_its_depth_and_nothing_to_walk_with() {
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let changed = group(2);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.apply(&mut arena, &repaint(changed)), Ok(Applied::Set(changed)));
        let sealed = dirty.at_commit(FRAME);

        // Preorder plus depth is the tree: the root at zero, its children at
        // one, and the paint the delta set arriving with the node it was set on.
        let mut first = None;
        let mut deepest = 0;
        let mut painted = 0;
        let visited = sealed.walk(&arena, |visit| {
            if first.is_none() {
                first = Some((visit.node(), visit.kind(), visit.depth()));
            }
            deepest = deepest.max(visit.depth());
            if visit.paint().is_some() {
                painted += 1;
            }
        });
        assert_eq!(first, Some((changed, Kind::Transform, 0)));
        assert_eq!(deepest, 1, "the group's leaves are one edge below it");
        assert_eq!(painted, 1, "only the node the delta named carries paint");
        assert_eq!(visited.nodes, 1 + PER_GROUP as usize);
    }

    #[test]
    fn a_mark_a_later_move_puts_inside_another_is_not_walked_twice() {
        // The attack on the equality. Two disjoint subtrees are marked, and
        // then — with the dirty set not told, which is the worst case — one is
        // moved inside the other. A set that checked disjointness only when the
        // marks were taken would visit the moved subtree twice and report a
        // count larger than the tree it walked.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let host = group(1);
        let guest = group(2);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, host), Marked::Added);
        assert_eq!(dirty.mark(&arena, guest), Marked::Added);
        assert_eq!(dirty.marked(), 2);

        // The move, straight at the arena.
        assert_eq!(
            arena.apply(&hang(guest, host, Kind::Transform)),
            Ok(Applied::Hung(Hung::Moved))
        );
        assert_eq!(arena.parent_of(guest), Ok(host));

        let sealed = dirty.at_commit(FRAME);
        assert_eq!(sealed.marked(), 2, "the plan still holds both marks");
        let (visited, calls, seen) = tally(&sealed, &arena);

        let expected = size_of_subtree(&arena, host);
        assert_eq!(expected, 2 * (1 + PER_GROUP as usize), "the move did not land");
        assert_eq!(visited.nodes, expected, "a node was visited twice, or not at all");
        assert_eq!(calls, visited.nodes);
        assert_eq!(visited.subtrees, 1, "the shadowed mark was walked as a subtree of its own");
        for node in every_node() {
            let want = u8::from(under(&arena, host, node));
            assert_eq!(seen.get(node as usize).copied(), Some(want), "node {node}");
        }
    }

    #[test]
    fn a_mark_inside_a_marked_subtree_is_subsumed_and_one_enclosing_it_sheds_it() {
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, group(3)), Marked::Added);
        assert_eq!(dirty.mark(&arena, leaf(3, 1)), Marked::Subsumed, "a descendant added a mark");
        assert_eq!(dirty.mark(&arena, group(3)), Marked::Subsumed, "the same node marked twice");
        assert_eq!(dirty.marked(), 1);

        // The root encloses it, so the group's mark goes rather than sitting
        // inside the root's and being walked a second time.
        assert_eq!(dirty.mark(&arena, ROOT), Marked::Added);
        assert_eq!(dirty.marked(), 1, "the enclosed mark survived");

        let sealed = dirty.at_commit(FRAME);
        let (visited, calls, seen) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, SCENE_NODES);
        assert_eq!(calls, SCENE_NODES);
        assert_eq!(visited.subtrees, 1);
        assert!(every_node().all(|node| seen.get(node as usize).copied() == Some(1)));
    }

    #[test]
    fn the_forest_is_what_no_node_names_and_it_visits_every_node_once() {
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, NO_NODE), Marked::Whole);
        assert!(dirty.is_whole());
        assert_eq!(dirty.marked(), 0, "the whole scene is not a list of roots as well");

        let sealed = dirty.at_commit(FRAME);
        let (visited, calls, seen) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, arena.live());
        assert_eq!(visited.nodes, SCENE_NODES);
        assert_eq!(calls, visited.nodes);
        assert!(!visited.halted);
        assert!(every_node().all(|node| seen.get(node as usize).copied() == Some(1)));
    }

    #[test]
    fn more_scattered_changes_than_the_list_holds_coarsen_to_the_whole_scene() {
        // Not a refusal, and not silently dropped marks: the frame that
        // scatters more changes than the list can hold gets the whole scene,
        // which over-draws and cannot be a stale region.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut dirty = Dirty::CLEAN;
        for nth in 0..MARKS_MAX {
            let node = leaf(1 + (nth as u32 % GROUPS), 1 + (nth as u32 / GROUPS));
            assert_eq!(dirty.mark(&arena, node), Marked::Added, "mark {nth} did not land");
        }
        assert_eq!(dirty.marked(), MARKS_MAX);
        assert!(!dirty.is_whole());

        let one_more = leaf(GROUPS, PER_GROUP);
        assert_eq!(dirty.mark(&arena, one_more), Marked::Whole);
        assert!(dirty.is_whole());

        let sealed = dirty.at_commit(FRAME);
        let (visited, calls, seen) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, SCENE_NODES, "coarsening lost or doubled a node");
        assert_eq!(calls, visited.nodes);
        assert!(every_node().all(|node| seen.get(node as usize).copied() == Some(1)));
    }

    #[test]
    fn a_removal_marks_the_hole_it_leaves_and_the_walk_is_what_survives() {
        // `Dirty::apply` is where the ordering lives: the parent is read before
        // the arena takes the subtree away, because afterwards there is no node
        // left to ask.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let gone = group(4);
        let mut dirty = Dirty::CLEAN;
        let removed = dirty.apply(&mut arena, &delta(Entry::RemoveNode(RemoveNode { node: gone })));
        assert_eq!(removed, Ok(Applied::Removed(1 + PER_GROUP as usize)));
        assert_eq!(dirty.marked(), 1);
        assert!(!arena.holds(gone));

        let sealed = dirty.at_commit(FRAME);
        let (visited, calls, seen) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, arena.live(), "the hole's subtree is what is left of the root's");
        assert_eq!(visited.nodes, SCENE_NODES - (1 + PER_GROUP as usize));
        assert_eq!(calls, visited.nodes);
        assert_eq!(seen.get(gone as usize).copied(), Some(0), "a removed node was encoded");
    }

    #[test]
    fn a_mark_whose_node_is_gone_by_the_commit_walks_nothing() {
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let doomed = leaf(5, 5);
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.mark(&arena, doomed), Marked::Added);
        assert_eq!(
            arena.apply(&delta(Entry::RemoveNode(RemoveNode { node: doomed }))),
            Ok(Applied::Removed(1))
        );

        let sealed = dirty.at_commit(FRAME);
        assert_eq!(sealed.marked(), 1);
        let (visited, calls, _) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, 0);
        assert_eq!(visited.subtrees, 0, "a subtree that is not there was counted as walked");
        assert_eq!(calls, 0);
    }

    #[test]
    fn a_delta_the_arena_will_refuse_marks_nothing() {
        // Refusing it here as well would be two statements of one rule. A
        // delta that changes nothing dirties nothing, and the arena is still
        // the one that says no.
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let absent = 9_999;
        let mut dirty = Dirty::CLEAN;
        assert_eq!(dirty.record(&arena, &repaint(absent)), Marked::Nothing);
        assert!(dirty.is_clean());
        assert_eq!(arena.apply(&repaint(absent)), Err(Refusal::NoSuchNode(absent)));

        let sealed = dirty.at_commit(FRAME);
        assert!(sealed.is_clean(), "a frame that changed nothing has an encode stage");
        let (visited, calls, _) = tally(&sealed, &arena);
        assert_eq!(visited.nodes, 0);
        assert_eq!(calls, 0);
    }

    #[test]
    fn every_opcode_but_the_commit_dirties_something_in_a_scene_that_holds_its_nodes() {
        // The corpus is `Entry::SPECIMENS`, so a seventh opcode is recorded
        // here on the day it is declared rather than when somebody remembers.
        let mut arena = Arena::EMPTY;
        let created = Ok(Applied::Hung(Hung::Created));
        // The nodes the specimens name.
        assert_eq!(arena.apply(&hang(3, NO_NODE, Kind::Layer)), created);
        assert_eq!(arena.apply(&hang(7, 3, Kind::Draw)), created);
        assert_eq!(arena.apply(&hang(0x0A0B_0C0D, 3, Kind::Draw)), created);

        for body in Entry::SPECIMENS {
            let mut dirty = Dirty::CLEAN;
            let marked = dirty.record(&arena, &delta(body));
            assert_eq!(
                marked == Marked::Nothing,
                body.opcode() == op::COMMIT,
                "{} dirties the wrong amount of the scene",
                op::label(body.opcode())
            );
            assert!(reach_of(body.opcode()).is_some(), "an opcode with no reach");
        }
    }

    #[test]
    fn a_commit_seals_the_frame_and_reopening_it_starts_the_next_one_clean() {
        let mut arena = Arena::EMPTY;
        build_a_thousand(&mut arena);

        let mut dirty = Dirty::CLEAN;
        dirty.record(&arena, &repaint(group(6)));
        let sealed = dirty.at_commit(FRAME);
        assert_eq!(sealed.frame_token(), FRAME);
        assert_eq!(sealed.marked(), 1);

        let next = sealed.reopen();
        assert!(next.is_clean(), "the next frame started with the last one's marks");
        assert_eq!(next.marked(), 0);
        assert!(!next.is_whole());
    }
}
