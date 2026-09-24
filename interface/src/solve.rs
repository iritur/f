// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Solve: layout from constraints, with the work bounded by the change rather
//! than by the tree.
//!
//! Section 12 of `docs/design/ring-scene-boot.html` is four stages and one
//! warning. [`crate::token`] is stage one; this is stage two, the one the note
//! calls *the entire engineering risk in this part*: **if a delta touching five
//! semantic nodes triggers a full layout solve, the section 05 budget is gone
//! and the simpler design wins outright.** `E3-B06e`'s own title says the same
//! thing less kindly — the stage the pillar dies at, if it dies.
//!
//! So this module is not written to lay out a tree well. It is written so that a
//! test can tell the difference between a solver that is incremental and one
//! that says it is, and that is a different design problem with a different
//! failure mode. A solver that re-solves ten thousand nodes and reports *five
//! subtrees* would satisfy every sentence of the exit while doing none of what
//! the exit is about.
//!
//! # The counter cannot be bypassed, which is the whole of the instrument
//!
//! `E3-B06e`'s exit asks for the number of re-solved subtrees to be *counted*.
//! A count is easy to produce and easy to produce falsely, so the count here is
//! not maintained beside the work — it is maintained **by** the work, and there
//! is no other door:
//!
//! - Every layout input a node carries — its [`Demand`], its intrinsic size, its
//!   solved extent and its solved offset — lives in a private field of a type in
//!   a private module. Nothing in this file can name it.
//! - The only function that returns any of them is that type's `visit`, and
//!   `visit` increments the slot's read count and stamps the pass that read it.
//!   There is no `demand()` accessor to reach for, because one would be the
//!   second door and the second door is what makes a counter a claim.
//! - So *nodes this pass read* is not a statistic about the solver, it is the
//!   solver's own trace. A pass that read ten thousand slots cannot report five,
//!   and [`Layout::nodes_read_in`] walks the array afterwards to say which slots
//!   were stamped — an answer maintained by nobody, checked against
//!   [`Work::nodes`], which is maintained by the pass. The two disagreeing is a
//!   red test, on `scene::arena`'s `the_census_and_the_slots_agree` terms.
//!
//! That is why the tests below assert on *which* slots were touched and not only
//! on a total. A whole-tree re-solve is caught by the other 9 945 slots
//! carrying this pass's stamp, which no arithmetic in the reporting path can
//! hide.
//!
//! # A solve boundary is declared, and a tree that declares none is refused
//!
//! The incremental part rests on one property, and it is worth stating plainly
//! because it is an obligation this module puts on applications that RFC 0077
//! did not: **a node whose declared extent is definite — `min_em_x100` equal to
//! `max_em_x100` — is a boundary, and a change underneath it cannot escape it.**
//! Nothing above a definite node has to be re-solved, because a definite node's
//! extent is the same number whatever its children do and whatever its parent
//! offers it.
//!
//! A change is therefore re-solved from the nearest definite ancestor of the
//! node that changed, and [`Scope`] is the ceiling on how many nodes those
//! subtrees may hold between them. A tree whose ten thousand nodes hang under one
//! root with no definite node anywhere between is **refused** — [`Cascade`] —
//! rather than laid out slowly. That is the exit's second clause taken literally:
//! *a cascade past a stated scope fails the test rather than slowing the frame.*
//! A frame that merely slows is the defect this clause exists to refuse, because
//! a slow frame is discovered by a user and a refusal is discovered by whoever
//! wrote the declaration.
//!
//! The obvious cheaper rule was considered and is not what this does, and the
//! reason is the same reason: stop the upward walk when a recomputed intrinsic
//! turns out equal to the stored one. That is strictly better in the common case
//! and it is not a guarantee — whether it fires depends on the *values* in the
//! tree, so the same declaration is incremental under one theme and quadratic
//! under the next, and a test that passes today goes red when a designer changes
//! a number. An optimisation nobody can state is not a bound. RFC 0113 is the
//! argument and names what would reverse it: a cascade bound that holds without a
//! declared boundary, at which point the refusal below is deleted and the
//! declaration stops carrying an obligation.
//!
//! # One axis, because the vocabulary declares one
//!
//! [`crate::node::Constraints`] has `min_em_x100`, `max_em_x100`, `grow` and
//! `flow`, and its own documentation says the sizes are *measured along the
//! parent's flow*. There is no cross-axis field, so there is no cross-axis
//! declaration to solve, and this module solves the axis the vocabulary has.
//! [`Flow::Inline`], [`Flow::Block`] and [`Flow::Wrap`] are therefore the same
//! arithmetic pointed in different directions — which direction is the emit
//! stage's to read. [`Flow::Own`] is the one that changes the arithmetic, and it
//! changes it by stopping: a node that arranges its own children is not descended
//! into, which is what `canvas.rs` means seen from this side and is also,
//! incidentally, the cheapest subtree in the tree.
//!
//! *What would reverse this:* a cross-axis field in `Constraints`, which is an
//! RFC against RFC 0077, at which point `Wrap` stops being `Inline` with
//! permission and this module grows a second measure pass.
//!
//! # No allocator, no growable anything, no floats
//!
//! [`NODES_MAX`] slots and one index over them, both fixed, on `scene::arena`'s
//! argument: *a renderer which can allocate under load has a worst case nobody
//! measures*, and every input here is chosen by a client. Arithmetic is `pt_x10`
//! — tenths of a point, the unit [`crate::token::Resolved`] already answers in —
//! widened to `i64` wherever a sum of ten thousand of them could not fit, and
//! divided in exactly one place, where a leftover is shared out.
//!
//! The index is a **second array**, which `scene::arena`'s size assertion
//! deliberately refuses to grow. That is not a contradiction and it is not a
//! reuse: `arena::find` is a scan of every slot and says so, naming *a measured
//! frame in which this scan shows against section 12's budget* as what would
//! reverse it. This stage is that budget. A scan would mean a delta touching five
//! nodes read ten thousand slots in order to find them, which is the exit's
//! failure condition reached before any layout arithmetic happened at all — so
//! the index is here, and [`Layout::probes`] is what makes its absence a red test
//! rather than a slow one.

use core::cell::Cell as Counter;

use crate::node::{DEPTH_MAX, Flow, Node, NodeId};
use crate::token::{Resolved, Span};

/// How many nodes a [`Layout`] holds.
///
/// Ten thousand is `E3-B06e`'s own figure — the exit says *five nodes of ten
/// thousand* — so the bound is the smallest round number above it that leaves the
/// index below its load ceiling. It is a maximum with no allocator behind it, for
/// `scene::arena::NODES_MAX`'s reason, and the price is the one stated there: an
/// interface that genuinely needs more nodes than this cannot be laid out and has
/// to declare the part of itself that is on screen.
///
/// *What would reverse this:* the first real interface measured past it. The
/// number moves and nothing else here does.
/// Unit: nodes.
pub const NODES_MAX: usize = 10_240;

/// How many slots the array holds: [`NODES_MAX`] of them, and one more at index
/// zero that is never a node.
///
/// Slot zero is the sentinel, and that is the whole reason this constant exists.
/// Every *none* in this module — no parent, no child, no sibling, no free slot,
/// an empty index bucket — is the integer zero, so a [`Layout`] that has never
/// held anything is a region of zero bytes. RFC 0100 is the rule that makes that
/// worth the one wasted slot: a component holds only what it can zero-initialise,
/// and a seven-hundred-kilobyte structure whose *empty* state is full of
/// `u16::MAX` is seven hundred kilobytes of image rather than seven hundred
/// kilobytes of nothing.
///
/// **Nothing in this diff observes that property**, because nothing in a
/// component holds a [`Layout`] yet. What observes it is whoever wires this stage
/// into `user/compositor`, by the image not growing — `E3-B06f` and `E3-B06g`.
/// The sentinels are arranged for it here rather than discovered there, because
/// discovering it there means changing every *none* in this file at the moment
/// somebody is trying to make a boot draw something.
/// Unit: slots.
const SLOTS: usize = NODES_MAX + 1;

/// How many buckets the identifier index spreads over.
///
/// A power of two, so [`bucket`] masks rather than divides, and far enough above
/// [`NODES_MAX`] that a full [`Layout`] probes short chains. Private: it is an
/// implementation of the lookup and not a fact about a layout.
/// Unit: index buckets.
const INDEX_SLOTS: usize = 16_384;

/// The mask that turns a mixed identifier into a bucket.
/// Unit: none — a bitmask over index buckets.
const INDEX_MASK: usize = INDEX_SLOTS - 1;

/// A bucket nothing has ever occupied. A probe that reaches one has found
/// nothing, because insertion never skips a free bucket.
///
/// **Zero**, which is not arbitrary: slot zero is never a node, so zero is free
/// to mean *nothing here*, and an index of zeroes is an index that costs a
/// component nothing to start. [`SLOTS`] is the argument.
/// Unit: none — a sentinel.
const INDEX_EMPTY: u16 = 0;

/// A bucket a removal emptied. A probe must continue past it, because the key it
/// is looking for may have been placed beyond it while this bucket was occupied.
/// Unit: none — a sentinel.
const INDEX_GONE: u16 = u16::MAX;

// The three facts the lookup rests on, stated where a violation is a build
// rather than a behaviour.
const _: () = assert!(
    INDEX_SLOTS.is_power_of_two(),
    "INDEX_SLOTS must be a power of two: bucket masks rather than divides"
);
const _: () = assert!(
    NODES_MAX * 4 <= INDEX_SLOTS * 3,
    "the index must stay under three quarters full with every slot live"
);
const _: () = assert!(
    SLOTS < INDEX_GONE as usize,
    "a slot index must be distinguishable from the index's two sentinels"
);
const _: () = assert!(
    INDEX_EMPTY == 0 && Ix::NONE.0 == 0,
    "every `nothing` in this module is the integer zero, or a Layout is not zero-initialisable"
);

/// How many distinct changed nodes one pass records before it stops
/// distinguishing them.
///
/// `scene::dirty::MARKS_MAX`'s number and its reason: past this the pass marks
/// the whole forest instead, which is correct rather than approximate — a boot's
/// first frame declares a whole tree and there is nothing incremental about it. A
/// caller who wants the incremental path back commits more often, which is the
/// behaviour the bound is trying to produce.
/// Unit: changed nodes.
pub const MARKS_MAX: usize = 64;

/// How many distinct subtrees one pass may re-solve.
///
/// [`MARKS_MAX`] changed nodes cannot produce more boundaries than there are
/// changed nodes, so this is the same number for the same reason rather than a
/// second decision.
/// Unit: subtrees.
pub const SUBTREES_MAX: usize = MARKS_MAX;

/// How deep the walks below reach: [`DEPTH_MAX`] parent steps, the root they
/// start from, and one more so the overflow arm is unreachable rather than
/// untested.
/// Unit: levels.
const LEVELS: usize = DEPTH_MAX + 2;

/// What a node needs along its parent's flow, after a theme has been resolved
/// against a display and before anything has been placed.
///
/// This is deliberately not [`Node`]. The solve stage runs after
/// [`crate::token::resolve`], so the numbers it works in are the resolved ones;
/// taking a `&[Node]` would mean either re-resolving per pass — which is the
/// thing `E3-B06d` built a lint to prevent — or holding a second tree of nodes
/// beside the one the system already owns. What a solver needs from a declaration
/// is four numbers, and [`Demand::of`] is the only place they are derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Demand {
    /// The smallest extent this node is useful at, in tenths of a point.
    /// Unit: pt_x10.
    pub min_pt_x10: i32,
    /// The largest extent worth giving it, or [`Span::UNBOUNDED`].
    /// Unit: pt_x10.
    pub max_pt_x10: i32,
    /// Share of the leftover, relative to siblings.
    /// Unit: shares.
    pub grow: u16,
    /// Whether this node arranges its own children, and which direction the emit
    /// stage reads the result in.
    pub flow: Flow,
}

impl Demand {
    /// What a node asks for, once a theme has been resolved.
    ///
    /// The bridge to `E3-B06d`, and the only one: the minimum here has already
    /// been through [`Resolved::span`], so a control too small to operate was
    /// raised before this module saw it and this module has no opinion about
    /// readability at all.
    #[must_use]
    pub fn of(resolved: &Resolved, node: &Node) -> Self {
        let span = resolved.span(node);
        Self {
            min_pt_x10: span.min_pt_x10,
            max_pt_x10: span.max_pt_x10,
            grow: node.layout.grow,
            flow: node.layout.flow,
        }
    }

    /// An extent that is exactly this and nothing else — a solve boundary.
    ///
    /// The declaration an application makes when it wants a change beneath this
    /// node to stay beneath it. `flow` is the caller's, because being a boundary
    /// and arranging children are independent.
    #[must_use]
    pub const fn definite(pt_x10: i32, flow: Flow) -> Self {
        Self { min_pt_x10: pt_x10, max_pt_x10: pt_x10, grow: 0, flow }
    }

    /// A node needing at least `min_pt_x10` and taking whatever it is given.
    #[must_use]
    pub const fn flexible(min_pt_x10: i32, grow: u16, flow: Flow) -> Self {
        Self { min_pt_x10, max_pt_x10: Span::UNBOUNDED, grow, flow }
    }

    /// Is this node a solve boundary?
    ///
    /// One comparison, and it is the whole of the rule: an extent that cannot
    /// move is an extent nothing below it and nothing above it can move.
    #[must_use]
    pub const fn is_definite(&self) -> bool {
        self.min_pt_x10 == self.max_pt_x10
    }

    /// Does a solver arrange this node's children?
    ///
    /// [`Flow::Own`] says no, and that is `canvas.rs`'s escape hatch seen from
    /// this side: the subtree under a canvas costs this module nothing, because
    /// the application has taken arrangement back.
    #[must_use]
    pub const fn arranges(&self) -> bool {
        !matches!(self.flow, Flow::Own)
    }

    /// The largest extent this demand can be read as wanting.
    ///
    /// [`Resolved::span`] never answers with a maximum below its minimum, but a
    /// [`Demand`] can also be written by hand, and an inverted pair would make
    /// the clamp below hand back an extent under the node's own floor. Taking the
    /// larger of the two is one expression rather than a refusal, because there
    /// is nothing a caller could usefully do with the refusal.
    /// Unit: pt_x10.
    const fn ceiling_pt_x10(&self) -> i32 {
        if self.max_pt_x10 > self.min_pt_x10 { self.max_pt_x10 } else { self.min_pt_x10 }
    }
}

/// How many nodes one pass is allowed to re-solve.
///
/// The exit's *stated scope*, stated by the caller so that a compositor with a
/// frame budget and a test with an assertion can disagree about it. It is checked
/// against the **plan** — the sum of the subtree sizes the pass is about to walk
/// — and not against the work as it happens, because a refusal that had to walk
/// the subtree in order to refuse it would have spent the frame it was
/// protecting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    /// The most nodes one pass may re-solve.
    /// Unit: nodes.
    pub nodes_max: u32,
}

impl Scope {
    /// The scope this module states, for a caller with no number of its own.
    ///
    /// Five hundred and twelve nodes, and the reason it is a round number rather
    /// than a measured one is that **nothing in E3 has been measured** —
    /// `claims/0033` says so about itself in its first paragraph, and a number
    /// chosen here against section 12's 0.2 ms would be evidence invented at the
    /// cheapest possible moment. What this number is, then, is a bound on what a
    /// *declaration* may ask for, in the same spirit as
    /// `scene::arena::NODES_MAX`: exceeding it is a fact about a tree rather than
    /// a fact about a machine, which is why it can be stated before any machine
    /// exists.
    ///
    /// *What would reverse this:* the first measured solve pass, which turns it
    /// into a number with a claim behind it and probably a different one.
    pub const DEFAULT: Self = Self { nodes_max: 512 };

    /// No ceiling at all: what a first pass over a whole declared tree wants,
    /// because a first pass is a full solve and there is nothing incremental
    /// about it to be honest about.
    pub const UNBOUNDED: Self = Self { nodes_max: u32::MAX };
}

/// What a pass did, counted by the pass itself.
///
/// Every field here is incremented by the code that does the thing it counts —
/// there is no summary step — and [`Layout::nodes_read_in`] is the independent
/// answer to the one that matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    /// Distinct subtrees re-solved. The exit's number.
    /// Unit: subtrees.
    pub subtrees: u32,
    /// Distinct nodes this pass read at all.
    /// Unit: nodes.
    pub nodes: u32,
    /// Individual reads, which is larger: measure and place each read a node, and
    /// a parent reads every child twice to share a leftover out.
    /// Unit: reads.
    pub reads: u64,
    /// Changed nodes the pass started from, or zero when the whole forest was
    /// marked.
    /// Unit: changed nodes.
    pub marks: u32,
    /// Nodes whose children asked for more than the node could give. Reported
    /// rather than refused: an overflowing declaration is the author's to fix and
    /// a projection still has to draw something.
    /// Unit: nodes.
    pub overflows: u32,
    /// Nodes whose maximum stopped them taking their share of the leftover, so
    /// that part of the leftover went unspent.
    /// Unit: nodes.
    pub capped: u32,
}

impl Work {
    /// A pass that found nothing to do.
    pub const NOTHING: Self =
        Self { subtrees: 0, nodes: 0, reads: 0, marks: 0, overflows: 0, capped: 0 };
}

/// A pass refused because the plan was larger than the scope.
///
/// Not a slow frame and not a partial layout: nothing is written when this is
/// returned and the marks stay where they were, so a caller can widen the scope
/// and ask again, or fix the declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cascade {
    /// The largest refused subtree's root — the node whose declaration has no
    /// definite ancestor between it and the change.
    pub at: NodeId,
    /// How many nodes the plan came to.
    /// Unit: nodes.
    pub nodes: u32,
    /// How many it was allowed.
    /// Unit: nodes.
    pub scope_nodes: u32,
    /// How many subtrees the plan held.
    /// Unit: subtrees.
    pub subtrees: u32,
}

/// Why a declaration did not reach the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// [`NodeId::UNNAMED`] names nothing, so nothing can be laid out under it or
    /// addressed by it. `node::check` refuses the same identity for the same
    /// reason.
    Unnamed,
    /// [`NODES_MAX`] slots are all live.
    Full,
    /// The parent has not been declared. A delta stream declares parents before
    /// children; one that does not has a hole in its tree, and inventing an
    /// ancestor is how a layout acquires a node nobody declared.
    NoParent(NodeId),
    /// The node would sit deeper than [`DEPTH_MAX`], which `node::check` already
    /// refuses in a declared tree.
    TooDeep(NodeId),
    /// A node already here was re-declared under a different parent.
    ///
    /// Refused rather than performed, and the reason is this module's own bound: a
    /// move changes two subtrees, every descendant's depth and two ancestor
    /// chains' counts, so it is [`Layout::remove`] followed by [`Layout::declare`]
    /// — two operations whose costs are the two things that actually happened.
    /// *What would reverse this:* a semantic delta stream that carries a move, at
    /// which point the cost is one operation's and belongs here.
    Reparented(NodeId),
    /// Nothing here holds that identity.
    Unknown(NodeId),
}

impl Refused {
    /// One line, for a report with no room for a match.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unnamed => "a node may not be declared under the identity that names nothing",
            Self::Full => "the layout already holds NODES_MAX nodes",
            Self::NoParent(_) => "the parent has not been declared",
            Self::TooDeep(_) => "deeper than DEPTH_MAX",
            Self::Reparented(_) => "a move is a removal and a declaration",
            Self::Unknown(_) => "no node here holds that identity",
        }
    }
}

/// What a declaration did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declared {
    /// Was this a node the layout did not hold?
    pub inserted: bool,
    /// Did anything change? A re-declaration that changes no number marks
    /// nothing, because there is nothing to re-solve.
    pub changed: bool,
    /// How deep it sits, a root being zero.
    /// Unit: levels.
    pub depth: u32,
}

/// Which slot, and nothing else.
///
/// `scene::arena::SlotIx`'s reason for being a newtype: an author's identifier
/// and this module's slot index cannot be passed to each other's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ix(u16);

impl Ix {
    /// No slot.
    ///
    /// Zero, because slot zero is never allocated — see [`SLOTS`]. A sentinel of
    /// `u16::MAX` would read identically everywhere in this file and would make a
    /// fresh [`Layout`] a region of set bits rather than a region of nothing.
    const NONE: Self = Self(0);

    /// Is this a slot at all?
    const fn is_some(self) -> bool {
        self.0 != Self::NONE.0
    }
}

/// Spread an author's identifier over the index.
///
/// The finalising mix of SplitMix64, with its published constants. Two things
/// about it are load-bearing here and neither is cryptographic. It is a **pure
/// function of the key** — no seed, no state, no process-dependent anything — so
/// two runs of the same declarations probe the same buckets in the same order,
/// which is what RFC 0004 is about and is the reason `HashMap`'s per-process seed
/// is refused in this tree rather than this arithmetic. And it moves the low bits:
/// identifiers an author assigns are usually consecutive small integers, and
/// masking those without mixing would pile a thousand siblings into a thousand
/// adjacent buckets, which is correct and slow.
///
/// It is written here rather than taken from `f-hash` because
/// `interface/Cargo.toml` takes no dependencies, so that this layer can be
/// deleted without taking anything else down. Four lines is the price.
const fn mix(key: u64) -> u64 {
    let mut x = key;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x
}

/// Where an identifier's probe starts.
const fn bucket(id: NodeId) -> usize {
    mix(id.value()) as usize & INDEX_MASK
}

/// What a node comes to want, given what its children came to want.
///
/// A free function rather than a method on [`Demand`], because it is the one
/// place the three cases are written and a reader looking for *how does a size
/// propagate upward* should find one expression rather than three arms scattered
/// through a walk.
///
/// - A **definite** node wants what it declared, whatever is underneath it. That
///   is the boundary rule, and it is the same sentence as the incrementality
///   argument seen from the arithmetic's side.
/// - A node that **arranges children** wants the larger of its own minimum and
///   what its children came to, held under its own maximum.
/// - Anything else — a leaf, or a [`Flow::Own`] node whose children are the
///   application's business — wants its minimum.
///
/// Widened to `i64` because ten thousand children of a thousand tenths of a point
/// overflows an `i32` and the sum is not the answer, only an input to it.
/// Unit: pt_x10.
fn intrinsic_pt_x10(demand: Demand, children_pt_x10: i64) -> i32 {
    if demand.is_definite() {
        return demand.min_pt_x10;
    }
    if !demand.arranges() {
        return demand.min_pt_x10;
    }
    let floor = i64::from(demand.min_pt_x10);
    let want = if children_pt_x10 > floor { children_pt_x10 } else { floor };
    let ceiling = i64::from(demand.ceiling_pt_x10());
    let held = if want > ceiling { ceiling } else { want };
    i32::try_from(held).unwrap_or(i32::MAX)
}

/// Everything about a node that the solver reads, handed out in one piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    demand: Demand,
    parent: Ix,
    first_child: Ix,
    next_sibling: Ix,
    subtree_nodes: u32,
    depth: u32,
    intrinsic_pt_x10: i32,
    extent_pt_x10: i32,
    offset_pt_x10: i32,
}

/// The slot, and the one door into it.
///
/// A private module rather than a private field, because the two are not the same
/// guarantee. A private field is invisible outside the module it is declared in,
/// and this module is where the solver lives — so a field private to it would be
/// a field the solve passes can read without counting, which is exactly the
/// bypass the exit is about. Put the type one level down and the solver is
/// *outside* its privacy: there is no expression in the solve passes that reaches
/// a demand except through [`Slot::visit`], and [`Slot::visit`] counts.
///
/// *What would reverse this:* a second accessor returning any of the four layout
/// numbers. It will not arrive as an argument about counting; it will arrive as a
/// convenience.
mod slot {
    use super::{Demand, Flow, Ix, NodeId, View};

    /// What the slot is.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum State {
        /// Never occupied.
        Empty,
        /// A node is here.
        Live,
        /// A node was here. The index keeps a tombstone for it, and the slot is
        /// on the free list.
        Gone,
    }

    /// One node's worth of layout.
    pub(super) struct Slot {
        /// The author's identifier.
        id: NodeId,
        state: State,
        depth: u32,
        parent: Ix,
        first_child: Ix,
        last_child: Ix,
        next_sibling: Ix,
        prev_sibling: Ix,
        subtree_nodes: u32,
        demand: Demand,
        intrinsic_pt_x10: i32,
        extent_pt_x10: i32,
        offset_pt_x10: i32,
        reads: u32,
        last_pass: u32,
    }

    impl Slot {
        /// A slot nothing has been in.
        pub(super) const EMPTY: Self = Self {
            id: NodeId::UNNAMED,
            state: State::Empty,
            depth: 0,
            parent: Ix::NONE,
            first_child: Ix::NONE,
            last_child: Ix::NONE,
            next_sibling: Ix::NONE,
            prev_sibling: Ix::NONE,
            subtree_nodes: 0,
            // Zero and not `Span::UNBOUNDED`, and `Flow::Inline` and not
            // `Flow::Own`, because both of those have a non-zero spelling and an
            // empty slot's demand is never read: `occupy` writes all four fields
            // before anything can reach the slot, and the links only ever point
            // at occupied slots. See [`SLOTS`].
            demand: Demand { min_pt_x10: 0, max_pt_x10: 0, grow: 0, flow: Flow::Inline },
            intrinsic_pt_x10: 0,
            extent_pt_x10: 0,
            offset_pt_x10: 0,
            reads: 0,
            last_pass: 0,
        };

        /// Read the node's layout, and be counted for it.
        ///
        /// The second half of the return is *this pass had not read this slot
        /// before*, which is what makes `Work::nodes` a count of nodes rather
        /// than a count of reads. `pass` is zero outside a solve, and a zero pass
        /// leaves the stamp alone so that a declaration cannot erase the record
        /// of which pass last read a slot.
        pub(super) fn visit(&mut self, pass: u32) -> (View, bool) {
            self.reads = self.reads.saturating_add(1);
            let first = pass != 0 && self.last_pass != pass;
            if pass != 0 {
                self.last_pass = pass;
            }
            let view = View {
                demand: self.demand,
                parent: self.parent,
                first_child: self.first_child,
                next_sibling: self.next_sibling,
                subtree_nodes: self.subtree_nodes,
                depth: self.depth,
                intrinsic_pt_x10: self.intrinsic_pt_x10,
                extent_pt_x10: self.extent_pt_x10,
                offset_pt_x10: self.offset_pt_x10,
            };
            (view, first)
        }

        /// The author's identifier, and be counted for reading it.
        ///
        /// `&mut self` for a getter of one word, and the reason is a defect this
        /// module had until it was mutation-tested. An identity is not a layout
        /// input, so the first draft handed it out uncounted — and a lookup that
        /// scanned every slot comparing identities would then have read ten
        /// thousand slots while every counter in the module stayed exactly where
        /// it was. *The only way to tell one node from another is through here*,
        /// so a search that is a scan shows up in [`Slot::reads`] whatever the
        /// code doing the searching chooses to report about itself.
        ///
        /// It does not stamp a pass, because a lookup is not a pass: which pass
        /// last *solved* a node is a different question from how often anything
        /// has looked at it, and one field answering both would answer neither.
        pub(super) fn key(&mut self) -> NodeId {
            self.reads = self.reads.saturating_add(1);
            self.id
        }

        /// Is a node here?
        pub(super) const fn is_live(&self) -> bool {
            matches!(self.state, State::Live)
        }

        /// How many reads this slot has answered, over its whole life.
        pub(super) const fn reads(&self) -> u32 {
            self.reads
        }

        /// The last solve pass that read it, or zero.
        pub(super) const fn last_pass(&self) -> u32 {
            self.last_pass
        }

        /// The structural links, uncounted.
        ///
        /// Shape is not a layout input: a parent link answers *where is this*,
        /// and the operations that maintain the tree read it without doing any
        /// arithmetic. The solve passes take theirs out of [`Slot::visit`]
        /// anyway, because they are already holding the view.
        pub(super) const fn parent(&self) -> Ix {
            self.parent
        }

        /// Its first child.
        pub(super) const fn first_child(&self) -> Ix {
            self.first_child
        }

        /// Its last child, so that appending a sibling is one step rather than a
        /// walk. A thousand siblings would otherwise cost half a million reads
        /// to declare, which is the exit's failure condition arriving through
        /// the door marked *construction*.
        pub(super) const fn last_child(&self) -> Ix {
            self.last_child
        }

        /// Its next sibling.
        pub(super) const fn next_sibling(&self) -> Ix {
            self.next_sibling
        }

        /// Its previous sibling.
        ///
        /// The second link, and the reason for it is this module's own exit: with
        /// one link, taking a node out of a list of a thousand siblings costs a
        /// walk of the thousand, and *work proportional to the change* is not a
        /// property a data structure can claim while its removal is a search.
        /// Two bytes a slot against a scan is the same trade the identifier index
        /// makes, made twice.
        pub(super) const fn prev_sibling(&self) -> Ix {
            self.prev_sibling
        }

        /// How deep it sits.
        pub(super) const fn depth(&self) -> u32 {
            self.depth
        }

        /// How many nodes its subtree holds, itself included. Maintained by the
        /// operations that add and remove nodes, so that a scope can be checked
        /// against a plan without walking it.
        pub(super) const fn subtree_nodes(&self) -> u32 {
            self.subtree_nodes
        }

        /// Put a node here.
        pub(super) const fn occupy(&mut self, id: NodeId, parent: Ix, depth: u32, demand: Demand) {
            self.id = id;
            self.state = State::Live;
            self.depth = depth;
            self.parent = parent;
            self.first_child = Ix::NONE;
            self.last_child = Ix::NONE;
            self.next_sibling = Ix::NONE;
            self.prev_sibling = Ix::NONE;
            self.subtree_nodes = 1;
            self.demand = demand;
            self.intrinsic_pt_x10 = 0;
            self.extent_pt_x10 = 0;
            self.offset_pt_x10 = 0;
        }

        /// Take the node out, leaving the slot on the free list through its own
        /// sibling link — `scene::arena`'s trick, and for its reason: a free list
        /// in the slots it tracks is not a second array.
        pub(super) const fn vacate(&mut self, free: Ix) {
            self.id = NodeId::UNNAMED;
            self.state = State::Gone;
            self.parent = Ix::NONE;
            self.first_child = Ix::NONE;
            self.last_child = Ix::NONE;
            self.next_sibling = free;
            self.prev_sibling = Ix::NONE;
            self.subtree_nodes = 0;
        }

        /// Say what it asks for now.
        pub(super) const fn set_demand(&mut self, demand: Demand) {
            self.demand = demand;
        }

        /// Its first child.
        pub(super) const fn set_first_child(&mut self, ix: Ix) {
            self.first_child = ix;
        }

        /// Its last child.
        pub(super) const fn set_last_child(&mut self, ix: Ix) {
            self.last_child = ix;
        }

        /// Its next sibling.
        pub(super) const fn set_next_sibling(&mut self, ix: Ix) {
            self.next_sibling = ix;
        }

        /// Its previous sibling.
        pub(super) const fn set_prev_sibling(&mut self, ix: Ix) {
            self.prev_sibling = ix;
        }

        /// How many nodes its subtree holds.
        pub(super) const fn set_subtree_nodes(&mut self, nodes: u32) {
            self.subtree_nodes = nodes;
        }

        /// What it came to want, once its children were measured.
        pub(super) const fn set_intrinsic_pt_x10(&mut self, pt_x10: i32) {
            self.intrinsic_pt_x10 = pt_x10;
        }

        /// Where it ended up.
        pub(super) const fn set_placed_pt_x10(&mut self, offset_pt_x10: i32, extent_pt_x10: i32) {
            self.offset_pt_x10 = offset_pt_x10;
            self.extent_pt_x10 = extent_pt_x10;
        }
    }
}

use slot::Slot;

/// The tree this stage holds, and the record of what it last had to re-solve.
///
/// One array of slots, one index over their identifiers, and three small fixed
/// arrays for the marks and the plan. It is neither `Clone` nor `Copy`: the whole
/// point of it is that it is the thing that is still there between frames, and a
/// copy of it is a second answer to *where is everything*.
pub struct Layout {
    slots: [Slot; SLOTS],
    /// Identifier to slot. `INDEX_EMPTY`, `INDEX_GONE`, or a slot index.
    index: [u16; INDEX_SLOTS],
    /// Slots that have been vacated, chained through their sibling links.
    free: Ix,
    /// The next never-used slot, so that a fresh layout does not have to build a
    /// free list of ten thousand entries before it can hold one node.
    fresh: usize,
    live: usize,
    /// Buckets that are not `INDEX_EMPTY`, tombstones included. The index is
    /// rebuilt when this reaches three quarters, because a table of tombstones
    /// makes every probe a scan.
    occupied: usize,
    roots: Ix,
    last_root: Ix,
    marks: [Ix; MARKS_MAX],
    marked: usize,
    whole: bool,
    pass: u32,
    probes: Counter<u64>,
    lost: u32,
}

impl Default for Layout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout {
    /// A layout holding nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [const { Slot::EMPTY }; SLOTS],
            index: [INDEX_EMPTY; INDEX_SLOTS],
            free: Ix::NONE,
            // One, because slot zero is the sentinel and is never handed out.
            fresh: 1,
            live: 0,
            occupied: 0,
            roots: Ix::NONE,
            last_root: Ix::NONE,
            marks: [Ix::NONE; MARKS_MAX],
            marked: 0,
            whole: false,
            pass: 0,
            probes: Counter::new(0),
            lost: 0,
        }
    }

    /// How many nodes it holds.
    #[must_use]
    pub const fn live(&self) -> usize {
        self.live
    }

    /// How many more it could hold.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        NODES_MAX - self.live
    }

    /// How many solve passes have run.
    #[must_use]
    pub const fn pass(&self) -> u32 {
        self.pass
    }

    /// How many changed nodes are waiting for the next pass.
    #[must_use]
    pub const fn marked(&self) -> usize {
        self.marked
    }

    /// Is the next pass a full solve?
    ///
    /// True when more than [`MARKS_MAX`] distinct nodes changed, which a first
    /// declaration of a whole tree always does.
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.whole
    }

    /// How many slots the identifier lookup has read, over the life of this
    /// layout.
    ///
    /// What the index itself costs, which is a different question from what the
    /// slots cost: a probe that lands on an empty bucket reads no slot at all.
    /// [`Layout::slot_reads`] is the one a lookup cannot lie to, and this is the
    /// one that says whether the chains are short. Both are asserted where five
    /// nodes are found among ten thousand, because a scan and a degenerate table
    /// are two different ways to arrive at the same bad number.
    /// Unit: index buckets.
    #[must_use]
    pub fn probes(&self) -> u64 {
        self.probes.get()
    }

    /// Links that led to a slot holding no node.
    ///
    /// Zero, always, and a test asserts it. It exists because every walk below
    /// answers a broken link by stopping rather than by panicking, and a walk
    /// that silently stops is a walk that silently reports less work than it did.
    /// `semantic::agent::Log::lost`'s reason: a counter nobody expects to move is
    /// the only kind whose zero means anything.
    /// Unit: links.
    #[must_use]
    pub const fn lost(&self) -> u32 {
        self.lost
    }

    /// Does it hold this identity?
    #[must_use]
    pub fn holds(&mut self, id: NodeId) -> bool {
        self.find(id).is_some()
    }

    /// Everything the solver knows about one node, on the same terms the solver
    /// gets it.
    ///
    /// `&mut self` for a getter, deliberately: the only door to a layout number
    /// is the counted one, and adding an uncounted door for the convenience of
    /// queries would be adding exactly the door the doc comment on the slot
    /// module says will arrive as a convenience. A query is work too, it is
    /// counted as work, and the pass stamp is left alone because no pass read it.
    fn look(&mut self, id: NodeId) -> Option<View> {
        let ix = self.find(id)?;
        let (view, _) = self.slot_mut(ix)?.visit(0);
        Some(view)
    }

    /// Where a node was placed along its parent's flow, and how much of the axis
    /// it got, in tenths of a point.
    ///
    /// The output of the stage, in the order *offset, extent*.
    #[must_use]
    pub fn placed_pt_x10(&mut self, id: NodeId) -> Option<(i32, i32)> {
        let view = self.look(id)?;
        Some((view.offset_pt_x10, view.extent_pt_x10))
    }

    /// What a node came to want once its children were measured.
    /// Unit: pt_x10.
    #[must_use]
    pub fn intrinsic_pt_x10(&mut self, id: NodeId) -> Option<i32> {
        Some(self.look(id)?.intrinsic_pt_x10)
    }

    /// What a node asks for.
    #[must_use]
    pub fn demand(&mut self, id: NodeId) -> Option<Demand> {
        Some(self.look(id)?.demand)
    }

    /// Is a node a solve boundary — the thing a cascade stops at?
    #[must_use]
    pub fn is_boundary(&mut self, id: NodeId) -> Option<bool> {
        Some(self.look(id)?.demand.is_definite())
    }

    /// How many nodes hang under a node, itself included.
    /// Unit: nodes.
    #[must_use]
    pub fn subtree_nodes(&mut self, id: NodeId) -> Option<u32> {
        let ix = self.find(id)?;
        Some(self.slot(ix)?.subtree_nodes())
    }

    /// How deep a node sits, a root being zero.
    /// Unit: levels.
    #[must_use]
    pub fn depth(&mut self, id: NodeId) -> Option<u32> {
        let ix = self.find(id)?;
        Some(self.slot(ix)?.depth())
    }

    /// The last solve pass that read a node, or zero for one no pass has.
    #[must_use]
    pub fn last_pass(&mut self, id: NodeId) -> Option<u32> {
        let ix = self.find(id)?;
        Some(self.slot(ix)?.last_pass())
    }

    /// How many reads a node has answered over its whole life.
    /// Unit: reads.
    #[must_use]
    pub fn reads(&mut self, id: NodeId) -> Option<u32> {
        let ix = self.find(id)?;
        Some(self.slot(ix)?.reads())
    }

    /// The slot at an index, or `None` for [`Ix::NONE`] and anything past the
    /// array.
    ///
    /// `get` rather than a subscript, and the `None` arm is not *this cannot
    /// happen*: it is the one answer a walk can give to a link that led nowhere
    /// without panicking, and [`Layout::lost`] is where those answers are counted
    /// so that a silent stop is not a silently smaller report.
    fn slot(&self, ix: Ix) -> Option<&Slot> {
        if !ix.is_some() {
            return None;
        }
        self.slots.get(ix.0 as usize)
    }

    /// The same, to write.
    fn slot_mut(&mut self, ix: Ix) -> Option<&mut Slot> {
        if !ix.is_some() {
            return None;
        }
        self.slots.get_mut(ix.0 as usize)
    }

    /// Which slot holds an identity.
    ///
    /// Linear probing from [`bucket`], stopping at the first bucket nothing has
    /// ever occupied, because insertion never skips one. Every bucket the probe
    /// looks at is counted, which is what makes [`Layout::probes`] an instrument
    /// rather than a statistic.
    fn find(&mut self, id: NodeId) -> Option<Ix> {
        if !id.is_named() {
            return None;
        }
        let mut at = bucket(id);
        for _ in 0..INDEX_SLOTS {
            self.probes.set(self.probes.get().saturating_add(1));
            let entry = *self.index.get(at)?;
            if entry == INDEX_EMPTY {
                return None;
            }
            if entry != INDEX_GONE {
                let ix = Ix(entry);
                if self.slot(ix).is_some_and(Slot::is_live)
                    && self.slot_mut(ix).map(Slot::key) == Some(id)
                {
                    return Some(ix);
                }
            }
            at = (at + 1) & INDEX_MASK;
        }
        None
    }

    /// Point the index at a slot. The caller has already established that the
    /// identity is absent, which is why this does not compare keys.
    fn index_insert(&mut self, id: NodeId, ix: Ix) {
        let mut at = bucket(id);
        let mut reusable: Option<usize> = None;
        for _ in 0..INDEX_SLOTS {
            self.probes.set(self.probes.get().saturating_add(1));
            let Some(&entry) = self.index.get(at) else { break };
            if entry == INDEX_EMPTY {
                let target = reusable.unwrap_or(at);
                if reusable.is_none() {
                    self.occupied += 1;
                }
                if let Some(cell) = self.index.get_mut(target) {
                    *cell = ix.0;
                    return;
                }
                break;
            }
            if entry == INDEX_GONE && reusable.is_none() {
                reusable = Some(at);
            }
            at = (at + 1) & INDEX_MASK;
        }
        // Unreachable while `occupied` is held under three quarters, which is what
        // `compact` keeps true. Counted rather than asserted, on
        // [`Layout::lost`]'s terms.
        self.lost = self.lost.saturating_add(1);
    }

    /// Leave a tombstone where an identity was.
    fn index_remove(&mut self, id: NodeId) {
        let mut at = bucket(id);
        for _ in 0..INDEX_SLOTS {
            self.probes.set(self.probes.get().saturating_add(1));
            let Some(&entry) = self.index.get(at) else { break };
            if entry == INDEX_EMPTY {
                return;
            }
            if entry != INDEX_GONE && self.slot_mut(Ix(entry)).map(Slot::key) == Some(id) {
                if let Some(cell) = self.index.get_mut(at) {
                    *cell = INDEX_GONE;
                }
                return;
            }
            at = (at + 1) & INDEX_MASK;
        }
    }

    /// Rebuild the index from the slots, dropping every tombstone.
    ///
    /// A table of tombstones turns every probe into a scan, which is the defect
    /// the index exists to avoid, so churn is paid for at the moment it would
    /// start costing rather than gradually and invisibly. It is the one operation
    /// here whose cost is the tree rather than the change, and it is deliberately
    /// outside a solve pass: it reads identities, never demands, so it cannot move
    /// a pass stamp and cannot flatter a count.
    fn compact(&mut self) {
        self.index = [INDEX_EMPTY; INDEX_SLOTS];
        self.occupied = 0;
        for raw in 1..self.fresh {
            let ix = Ix(u16::try_from(raw).unwrap_or(u16::MAX));
            if !self.slot(ix).is_some_and(Slot::is_live) {
                continue;
            }
            let Some(id) = self.slot_mut(ix).map(Slot::key) else { continue };
            self.index_insert(id, ix);
        }
    }

    /// Take a free slot, or say there is none.
    fn take_slot(&mut self) -> Option<Ix> {
        if self.free.is_some() {
            let ix = self.free;
            self.free = self.slot(ix).map_or(Ix::NONE, Slot::next_sibling);
            return Some(ix);
        }
        if self.fresh < SLOTS {
            let ix = Ix(u16::try_from(self.fresh).unwrap_or(u16::MAX));
            self.fresh += 1;
            return Some(ix);
        }
        None
    }

    /// Record that a node's contribution to its parent changed.
    ///
    /// The mark is the node itself and not its boundary, because which node is a
    /// boundary is a question about demands, and answering it here would mean
    /// reading demands outside a pass — where the reads would not be attributed to
    /// the pass that caused them. `scene::dirty` takes the same view: a delta
    /// records what it named, and the walk happens at the commit.
    fn mark(&mut self, ix: Ix) {
        if self.whole {
            return;
        }
        for existing in self.marks.iter().take(self.marked) {
            if *existing == ix {
                return;
            }
        }
        match self.marks.get_mut(self.marked) {
            Some(cell) => {
                *cell = ix;
                self.marked += 1;
            }
            None => {
                self.whole = true;
                self.marked = 0;
            }
        }
    }

    /// Add `delta` to the subtree count of every ancestor from `from` upward.
    ///
    /// Bounded by [`DEPTH_MAX`], which is what makes maintaining `subtree_nodes`
    /// cheap enough to do on every declaration — and it is worth doing, because it
    /// is what lets a scope be checked against a plan without walking the thing
    /// the plan is about.
    fn adjust_ancestors(&mut self, from: Ix, delta: i64) {
        let mut here = from;
        for _ in 0..=DEPTH_MAX {
            if !here.is_some() {
                return;
            }
            let Some(slot) = self.slot_mut(here) else {
                self.lost = self.lost.saturating_add(1);
                return;
            };
            let now = i64::from(slot.subtree_nodes()).saturating_add(delta);
            let now = if now < 0 { 0 } else { now };
            slot.set_subtree_nodes(u32::try_from(now).unwrap_or(u32::MAX));
            here = slot.parent();
        }
    }

    /// Put a node into a sibling list: last child of `parent`, or last root.
    fn link_last(&mut self, ix: Ix, parent: Ix) {
        let tail = if parent.is_some() {
            self.slot(parent).map_or(Ix::NONE, Slot::last_child)
        } else {
            self.last_root
        };
        if let Some(slot) = self.slot_mut(ix) {
            slot.set_prev_sibling(tail);
            slot.set_next_sibling(Ix::NONE);
        }
        if tail.is_some() {
            if let Some(slot) = self.slot_mut(tail) {
                slot.set_next_sibling(ix);
            }
        } else if parent.is_some() {
            if let Some(slot) = self.slot_mut(parent) {
                slot.set_first_child(ix);
            }
        } else {
            self.roots = ix;
        }
        if parent.is_some() {
            if let Some(slot) = self.slot_mut(parent) {
                slot.set_last_child(ix);
            }
        } else {
            self.last_root = ix;
        }
    }

    /// Take a node out of its sibling list. Two link writes, because there are
    /// two links.
    fn unlink(&mut self, ix: Ix, parent: Ix) {
        let (prev, next) = match self.slot(ix) {
            Some(slot) => (slot.prev_sibling(), slot.next_sibling()),
            None => {
                self.lost = self.lost.saturating_add(1);
                return;
            }
        };
        if prev.is_some() {
            if let Some(slot) = self.slot_mut(prev) {
                slot.set_next_sibling(next);
            }
        } else if parent.is_some() {
            if let Some(slot) = self.slot_mut(parent) {
                slot.set_first_child(next);
            }
        } else {
            self.roots = next;
        }
        if next.is_some() {
            if let Some(slot) = self.slot_mut(next) {
                slot.set_prev_sibling(prev);
            }
        } else if parent.is_some() {
            if let Some(slot) = self.slot_mut(parent) {
                slot.set_last_child(prev);
            }
        } else {
            self.last_root = prev;
        }
        if let Some(slot) = self.slot_mut(ix) {
            slot.set_prev_sibling(Ix::NONE);
            slot.set_next_sibling(Ix::NONE);
        }
    }

    /// Declare a node, or say again what an already-declared node needs.
    ///
    /// The one way a tree gets into this module. A parent must already be here — a
    /// delta stream declares parents before children — and a node already here may
    /// change what it needs but not where it sits, for [`Refused::Reparented`]'s
    /// reason.
    ///
    /// # Errors
    ///
    /// Every variant of [`Refused`]. None of them is recoverable by trying
    /// something slightly different: each says the declaration was wrong.
    pub fn declare(
        &mut self,
        id: NodeId,
        parent: NodeId,
        demand: Demand,
    ) -> Result<Declared, Refused> {
        if !id.is_named() {
            return Err(Refused::Unnamed);
        }
        let parent_ix = if parent.is_named() {
            match self.find(parent) {
                Some(ix) => ix,
                None => return Err(Refused::NoParent(parent)),
            }
        } else {
            Ix::NONE
        };

        if let Some(ix) = self.find(id) {
            if self.slot(ix).map_or(Ix::NONE, Slot::parent) != parent_ix {
                return Err(Refused::Reparented(id));
            }
            let seen = self.slot_mut(ix).map(|slot| slot.visit(0));
            let Some((view, _)) = seen else {
                self.lost = self.lost.saturating_add(1);
                return Err(Refused::Unknown(id));
            };
            if view.demand == demand {
                return Ok(Declared { inserted: false, changed: false, depth: view.depth });
            }
            if let Some(slot) = self.slot_mut(ix) {
                slot.set_demand(demand);
            }
            self.mark(ix);
            return Ok(Declared { inserted: false, changed: true, depth: view.depth });
        }

        let depth = if parent_ix.is_some() {
            self.slot(parent_ix).map_or(0, Slot::depth).saturating_add(1)
        } else {
            0
        };
        if depth as usize > DEPTH_MAX {
            return Err(Refused::TooDeep(id));
        }
        if self.live >= NODES_MAX {
            return Err(Refused::Full);
        }
        if self.occupied * 4 >= INDEX_SLOTS * 3 {
            self.compact();
        }
        let Some(ix) = self.take_slot() else { return Err(Refused::Full) };
        if let Some(slot) = self.slot_mut(ix) {
            slot.occupy(id, parent_ix, depth, demand);
        } else {
            self.lost = self.lost.saturating_add(1);
            return Err(Refused::Full);
        }
        self.live += 1;
        self.index_insert(id, ix);
        self.link_last(ix, parent_ix);
        self.adjust_ancestors(parent_ix, 1);
        // A new child changes what its parent asks for; a new root changes nothing
        // above it, so it marks itself.
        self.mark(if parent_ix.is_some() { parent_ix } else { ix });
        Ok(Declared { inserted: true, changed: true, depth })
    }

    /// Take a node and everything under it out.
    ///
    /// # Errors
    ///
    /// [`Refused::Unknown`] when nothing here holds the identity.
    pub fn remove(&mut self, id: NodeId) -> Result<usize, Refused> {
        let Some(ix) = self.find(id) else { return Err(Refused::Unknown(id)) };
        let (parent, count) = match self.slot(ix) {
            Some(slot) => (slot.parent(), slot.subtree_nodes()),
            None => return Err(Refused::Unknown(id)),
        };
        self.unlink(ix, parent);
        self.adjust_ancestors(parent, -i64::from(count));
        if parent.is_some() {
            self.mark(parent);
        }

        // The work list is threaded through the sibling links of the nodes about to
        // be released — `scene::arena::remove`'s trick, for its reason: a recursive
        // teardown is one fixed allocation plus however much stack the client's
        // depth asks for, and the client chooses the depth.
        let mut head = ix;
        if let Some(slot) = self.slot_mut(ix) {
            slot.set_next_sibling(Ix::NONE);
        }
        let mut gone = 0;
        while head.is_some() {
            let here = head;
            let (next, first) = match self.slot(here) {
                Some(slot) => (slot.next_sibling(), slot.first_child()),
                None => {
                    self.lost = self.lost.saturating_add(1);
                    break;
                }
            };
            let Some(key) = self.slot_mut(here).map(Slot::key) else {
                self.lost = self.lost.saturating_add(1);
                break;
            };
            head = next;
            let mut child = first;
            while child.is_some() {
                let after = self.slot(child).map_or(Ix::NONE, Slot::next_sibling);
                if let Some(slot) = self.slot_mut(child) {
                    slot.set_next_sibling(head);
                }
                head = child;
                child = after;
            }
            self.index_remove(key);
            let free = self.free;
            if let Some(slot) = self.slot_mut(here) {
                slot.vacate(free);
            }
            self.free = here;
            self.live = self.live.saturating_sub(1);
            gone += 1;
        }
        Ok(gone)
    }

    /// Read a slot inside a pass, and be counted for it.
    fn visit_at(&mut self, ix: Ix, pass: u32, work: &mut Work) -> Option<View> {
        let seen = self.slot_mut(ix).map(|slot| slot.visit(pass));
        let Some((view, first)) = seen else {
            self.lost = self.lost.saturating_add(1);
            return None;
        };
        if first {
            work.nodes = work.nodes.saturating_add(1);
        }
        work.reads = work.reads.saturating_add(1);
        Some(view)
    }

    /// The subtree a change has to be re-solved from.
    ///
    /// Up from the changed node's parent to the first definite ancestor, or to a
    /// root. Every step reads a demand, which is honest work and is counted as
    /// such: deciding where a cascade stops is part of the cost of the change.
    /// Bounded by [`DEPTH_MAX`], so a plan can never cost more than [`MARKS_MAX`]
    /// times seventeen reads however large the tree is — which is why the scope
    /// does not have to bound the planning as well as the work.
    fn boundary_of(&mut self, mark: Ix, pass: u32, work: &mut Work) -> Ix {
        let Some(view) = self.visit_at(mark, pass, work) else { return Ix::NONE };
        let mut here = view.parent;
        if !here.is_some() {
            return mark;
        }
        for _ in 0..=DEPTH_MAX {
            let Some(view) = self.visit_at(here, pass, work) else { return Ix::NONE };
            if view.demand.is_definite() || !view.parent.is_some() {
                return here;
            }
            here = view.parent;
        }
        here
    }

    /// Is `a` an ancestor of `b`, or `b` itself?
    fn encloses(&self, a: Ix, b: Ix) -> bool {
        if a == b {
            return true;
        }
        let mut here = b;
        for _ in 0..=DEPTH_MAX {
            let Some(slot) = self.slot(here) else { return false };
            let parent = slot.parent();
            if !parent.is_some() {
                return false;
            }
            if parent == a {
                return true;
            }
            here = parent;
        }
        false
    }

    /// Solve, and say what it cost.
    ///
    /// `viewport_pt_x10` is the extent a root is offered along its own flow. The
    /// pass re-solves the subtree of every changed node's nearest definite
    /// ancestor, and nothing else.
    ///
    /// # Errors
    ///
    /// [`Cascade`] when the plan is larger than `scope`. Nothing is written and the
    /// marks are left where they were, so a caller can widen the scope and ask
    /// again — or read the node the refusal names and put a definite extent on
    /// something above it.
    pub fn solve(&mut self, viewport_pt_x10: i32, scope: Scope) -> Result<Work, Cascade> {
        if !self.whole && self.marked == 0 {
            return Ok(Work::NOTHING);
        }
        self.pass = self.pass.saturating_add(1);
        let pass = self.pass;
        let mut work = Work::NOTHING;

        let mut plan = [Ix::NONE; SUBTREES_MAX];
        let mut planned = 0usize;
        let mut nodes = 0i64;
        let mut largest = Ix::NONE;
        let mut largest_nodes = 0u32;

        if self.whole {
            let mut root = self.roots;
            while root.is_some() {
                let (next, count) = match self.slot(root) {
                    Some(slot) => (slot.next_sibling(), slot.subtree_nodes()),
                    None => {
                        self.lost = self.lost.saturating_add(1);
                        break;
                    }
                };
                nodes = nodes.saturating_add(i64::from(count));
                if count >= largest_nodes {
                    largest_nodes = count;
                    largest = root;
                }
                planned += 1;
                root = next;
            }
        } else {
            work.marks = u32::try_from(self.marked).unwrap_or(u32::MAX);
            for at in 0..self.marked {
                let mark = self.marks.get(at).copied().unwrap_or(Ix::NONE);
                if !mark.is_some() {
                    continue;
                }
                let root = self.boundary_of(mark, pass, &mut work);
                if root.is_some() {
                    planned = self.enrol(&mut plan, planned, root);
                }
            }
            for entry in plan.iter().take(planned) {
                let count = self.slot(*entry).map_or(0, Slot::subtree_nodes);
                nodes = nodes.saturating_add(i64::from(count));
                if count >= largest_nodes {
                    largest_nodes = count;
                    largest = *entry;
                }
            }
        }

        let planned_nodes = u32::try_from(nodes).unwrap_or(u32::MAX);
        if planned_nodes > scope.nodes_max {
            return Err(Cascade {
                at: self.slot_mut(largest).map(Slot::key).unwrap_or(NodeId::UNNAMED),
                nodes: planned_nodes,
                scope_nodes: scope.nodes_max,
                subtrees: u32::try_from(planned).unwrap_or(u32::MAX),
            });
        }

        if self.whole {
            let mut root = self.roots;
            while root.is_some() {
                let next = self.slot(root).map_or(Ix::NONE, Slot::next_sibling);
                self.solve_subtree(root, viewport_pt_x10, pass, &mut work);
                root = next;
            }
        } else {
            for at in 0..planned {
                let root = plan.get(at).copied().unwrap_or(Ix::NONE);
                if root.is_some() {
                    self.solve_subtree(root, viewport_pt_x10, pass, &mut work);
                }
            }
        }

        work.subtrees = u32::try_from(planned).unwrap_or(u32::MAX);
        self.marked = 0;
        self.whole = false;
        Ok(work)
    }

    /// Put one boundary root into the plan, keeping the plan's entries disjoint.
    ///
    /// Two changed nodes under one definite ancestor are one subtree, and two
    /// changed nodes whose boundaries nest are also one subtree — the outer one.
    /// Counting the inner one as a second is exactly how a report says five and
    /// means four, so the nesting is resolved here rather than reported.
    fn enrol(&self, plan: &mut [Ix; SUBTREES_MAX], planned: usize, root: Ix) -> usize {
        let mut planned = planned;
        let mut at = 0;
        while at < planned {
            let held = plan.get(at).copied().unwrap_or(Ix::NONE);
            if self.encloses(held, root) {
                return planned;
            }
            if self.encloses(root, held) {
                planned -= 1;
                let last = plan.get(planned).copied().unwrap_or(Ix::NONE);
                if let Some(cell) = plan.get_mut(at) {
                    *cell = last;
                }
                continue;
            }
            at += 1;
        }
        if let Some(cell) = plan.get_mut(planned) {
            *cell = root;
            return planned + 1;
        }
        planned
    }

    /// Measure and place one boundary subtree.
    ///
    /// The root of one is either definite — in which case its extent is the one
    /// number it declared, whatever anything else does — or a root of the forest,
    /// in which case it is offered the viewport. There is no third case, because
    /// those are the two things [`Layout::boundary_of`] can answer.
    fn solve_subtree(&mut self, root: Ix, viewport_pt_x10: i32, pass: u32, work: &mut Work) {
        let Some(view) = self.visit_at(root, pass, work) else { return };
        self.measure(root, pass, work);
        let (offset, extent) = if view.parent.is_some() {
            (view.offset_pt_x10, view.demand.min_pt_x10)
        } else {
            let want = i64::from(viewport_pt_x10);
            let floor = i64::from(view.demand.min_pt_x10);
            let ceiling = i64::from(view.demand.ceiling_pt_x10());
            let held = if want > ceiling { ceiling } else { want };
            if held < floor {
                work.overflows = work.overflows.saturating_add(1);
            }
            let held = if held < floor { floor } else { held };
            (0, i32::try_from(held).unwrap_or(i32::MAX))
        };
        if let Some(slot) = self.slot_mut(root) {
            slot.set_placed_pt_x10(offset, extent);
        }
        self.place(root, pass, work);
    }

    /// Bottom-up: what every node in the subtree comes to want.
    ///
    /// Iterative, with one cursor per level and no recursion, because the
    /// alternative is a stack whose depth a declaration chooses. [`DEPTH_MAX`]
    /// bounds the arrays and `node::check` bounds the trees.
    fn measure(&mut self, root: Ix, pass: u32, work: &mut Work) {
        let mut stack = [Ix::NONE; LEVELS];
        let mut cursor = [Ix::NONE; LEVELS];
        let mut sum = [0i64; LEVELS];
        let Some(view) = self.visit_at(root, pass, work) else { return };
        let mut depth = 0usize;
        stack[0] = root;
        cursor[0] = if view.demand.arranges() { view.first_child } else { Ix::NONE };
        loop {
            let here = stack.get(depth).copied().unwrap_or(Ix::NONE);
            let child = cursor.get(depth).copied().unwrap_or(Ix::NONE);
            if child.is_some() {
                let Some(cv) = self.visit_at(child, pass, work) else {
                    if let Some(cell) = cursor.get_mut(depth) {
                        *cell = Ix::NONE;
                    }
                    continue;
                };
                if let Some(cell) = cursor.get_mut(depth) {
                    *cell = cv.next_sibling;
                }
                if depth + 1 >= LEVELS {
                    self.lost = self.lost.saturating_add(1);
                    continue;
                }
                depth += 1;
                stack[depth] = child;
                cursor[depth] = if cv.demand.arranges() { cv.first_child } else { Ix::NONE };
                sum[depth] = 0;
            } else {
                let Some(hv) = self.visit_at(here, pass, work) else {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    continue;
                };
                let children = sum.get(depth).copied().unwrap_or(0);
                let intrinsic = intrinsic_pt_x10(hv.demand, children);
                if children > i64::from(intrinsic) {
                    work.overflows = work.overflows.saturating_add(1);
                }
                if let Some(slot) = self.slot_mut(here) {
                    slot.set_intrinsic_pt_x10(intrinsic);
                }
                if depth == 0 {
                    return;
                }
                depth -= 1;
                if let Some(cell) = sum.get_mut(depth) {
                    *cell = cell.saturating_add(i64::from(intrinsic));
                }
            }
        }
    }

    /// Top-down: hand every node its extent and its offset.
    fn place(&mut self, root: Ix, pass: u32, work: &mut Work) {
        let mut cursor = [Ix::NONE; LEVELS];
        self.share(root, pass, work);
        let Some(view) = self.visit_at(root, pass, work) else { return };
        let mut depth = 0usize;
        cursor[0] = if view.demand.arranges() { view.first_child } else { Ix::NONE };
        loop {
            let child = cursor.get(depth).copied().unwrap_or(Ix::NONE);
            if !child.is_some() {
                if depth == 0 {
                    return;
                }
                depth -= 1;
                continue;
            }
            let Some(cv) = self.visit_at(child, pass, work) else {
                if let Some(cell) = cursor.get_mut(depth) {
                    *cell = Ix::NONE;
                }
                continue;
            };
            if let Some(cell) = cursor.get_mut(depth) {
                *cell = cv.next_sibling;
            }
            if cv.demand.arranges() && cv.first_child.is_some() {
                self.share(child, pass, work);
                if depth + 1 < LEVELS {
                    depth += 1;
                    cursor[depth] = cv.first_child;
                } else {
                    self.lost = self.lost.saturating_add(1);
                }
            }
        }
    }

    /// Give one node's children their extents and their offsets.
    ///
    /// Each child starts at what it came to want, and whatever is left over is
    /// shared by `grow`. The share is computed against a **running total** rather
    /// than per child, so the integer division loses nothing: every unit of the
    /// leftover is handed to exactly one child, the last child gets the remainder,
    /// and the answer does not depend on the order the divisions happen to round
    /// in. This is the only division in the module.
    ///
    /// A child whose maximum stops it taking its share does not have that share
    /// redistributed, and that is a decision rather than an oversight:
    /// redistribution is a loop over the siblings, run again each time a child
    /// caps, and an unbounded inner loop over siblings is the cascade this module
    /// refuses one level up. The unspent leftover is reported as [`Work::capped`].
    /// *What would reverse this:* a declaration in which the residue is visible, at
    /// which point the loop is bounded by how many children can cap — which is all
    /// of them — and the cost is stated rather than discovered.
    fn share(&mut self, parent: Ix, pass: u32, work: &mut Work) {
        let Some(pv) = self.visit_at(parent, pass, work) else { return };
        if !pv.demand.arranges() {
            return;
        }
        let mut base_total: i64 = 0;
        let mut grow_total: i64 = 0;
        let mut child = pv.first_child;
        while child.is_some() {
            let Some(cv) = self.visit_at(child, pass, work) else { break };
            base_total = base_total.saturating_add(i64::from(cv.intrinsic_pt_x10));
            grow_total = grow_total.saturating_add(i64::from(cv.demand.grow));
            child = cv.next_sibling;
        }
        let leftover = i64::from(pv.extent_pt_x10).saturating_sub(base_total);
        let leftover = if leftover > 0 { leftover } else { 0 };

        let mut at = pv.offset_pt_x10;
        let mut consumed: i64 = 0;
        let mut handed: i64 = 0;
        let mut child = pv.first_child;
        while child.is_some() {
            let Some(cv) = self.visit_at(child, pass, work) else { break };
            let share = if grow_total == 0 {
                0
            } else {
                consumed = consumed.saturating_add(i64::from(cv.demand.grow));
                let target = leftover.saturating_mul(consumed) / grow_total;
                let this = target - handed;
                handed = target;
                this
            };
            let want = i64::from(cv.intrinsic_pt_x10).saturating_add(share);
            let ceiling = i64::from(cv.demand.ceiling_pt_x10());
            let given = if want > ceiling {
                work.capped = work.capped.saturating_add(1);
                ceiling
            } else {
                want
            };
            let given = i32::try_from(given).unwrap_or(i32::MAX);
            if let Some(slot) = self.slot_mut(child) {
                slot.set_placed_pt_x10(at, given);
            }
            at = at.saturating_add(given);
            child = cv.next_sibling;
        }
    }

    /// Every read every slot has answered, counted by walking them.
    ///
    /// The independent total, and the instrument for the half of the exit that is
    /// not about layout arithmetic at all: *touching five nodes must not read the
    /// other nine thousand nine hundred and forty-five*, which includes not
    /// reading them in order to find the five. A lookup that was a scan would
    /// show up here even if it reported nothing about itself, because telling one
    /// slot from another goes through a counted door.
    /// Unit: reads.
    #[must_use]
    pub fn slot_reads(&self) -> u64 {
        let mut total = 0;
        for slot in &self.slots {
            total += u64::from(slot.reads());
        }
        total
    }

    /// How many nodes a given pass read, counted by walking every slot.
    ///
    /// The independent answer. [`Work::nodes`] is maintained by the pass as it
    /// runs; this is maintained by nobody and derived afterwards, and the solve
    /// path never calls it. Two independently produced answers to one question is
    /// `scene::arena`'s `the_census_and_the_slots_agree`, and the reason it is
    /// worth the walk is that an incremental solver's own report is exactly the
    /// thing under suspicion.
    /// Unit: nodes.
    #[must_use]
    pub fn nodes_read_in(&self, pass: u32) -> u32 {
        let mut count = 0;
        for slot in &self.slots {
            if slot.is_live() && slot.last_pass() == pass {
                count += 1;
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{CapRef, Constraints, Role};
    use crate::token::{Theme, resolve};

    /// Nine hundred and nine sections of ten leaves, under one surface, is ten
    /// thousand nodes exactly — which is the number `E3-B06e`'s exit is written
    /// about, so the fixture is that number and not a round approximation of it.
    const SECTIONS: u64 = 909;
    /// Leaves per section.
    const PER_SECTION: u64 = 10;
    /// What the fixture holds: the surface, plus a section and its ten leaves,
    /// nine hundred and nine times.
    const TREE_NODES: u32 = 1 + (SECTIONS as u32) * (1 + PER_SECTION as u32);
    /// One section and its ten leaves.
    const SUBTREE_NODES: u32 = 1 + PER_SECTION as u32;

    const SURFACE: NodeId = NodeId::new(1);

    /// A section's identity. Disjoint from the leaves' range so that a mistyped
    /// fixture collides rather than aliasing quietly.
    fn section(at: u64) -> NodeId {
        NodeId::new(1_000 + at)
    }

    /// A leaf's identity.
    fn leaf(at: u64, which: u64) -> NodeId {
        NodeId::new(1_000_000 + at * 100 + which)
    }

    /// A section's extent when it is a boundary: definite, so a change inside it
    /// cannot escape it.
    const SECTION_PT_X10: i32 = 1_000;
    /// What a leaf asks for.
    const LEAF_PT_X10: i32 = 50;
    /// The axis a surface is offered.
    const VIEWPORT_PT_X10: i32 = 1_000_000;

    /// The fixture, with the sections declaring whatever the caller wants them
    /// to — which is the difference between the incremental tests and the
    /// cascade test, and the only difference.
    fn build(layout: &mut Layout, sections: Demand) {
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 1, Flow::Block))
            .expect("a surface with nothing in the way of it");
        for at in 0..SECTIONS {
            layout.declare(section(at), SURFACE, sections).expect("a section under the surface");
            for which in 0..PER_SECTION {
                layout
                    .declare(
                        leaf(at, which),
                        section(at),
                        Demand::flexible(LEAF_PT_X10, 0, Flow::Own),
                    )
                    .expect("a leaf under its section");
            }
        }
    }

    /// Sections that are boundaries.
    fn bounded() -> Demand {
        Demand::definite(SECTION_PT_X10, Flow::Block)
    }

    /// Sections that are not, so that a change under one has nowhere to stop
    /// short of the surface.
    fn unbounded_sections() -> Demand {
        Demand::flexible(0, 1, Flow::Block)
    }

    #[test]
    fn ten_thousand_nodes_fit_and_the_first_pass_is_a_full_solve() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        assert_eq!(layout.live(), TREE_NODES as usize);
        assert_eq!(layout.subtree_nodes(SURFACE), Some(TREE_NODES));
        assert_eq!(layout.subtree_nodes(section(0)), Some(SUBTREE_NODES));
        // Ten thousand declarations are far more than MARKS_MAX distinct changed
        // nodes, so the first pass is honestly a whole one rather than a small
        // number the record cannot support.
        assert!(layout.is_whole());

        let before = layout.slot_reads();
        let work = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");
        assert_eq!(work.subtrees, 1);
        assert_eq!(work.nodes, TREE_NODES);
        assert_eq!(work.overflows, 0);
        assert_eq!(layout.nodes_read_in(layout.pass()), TREE_NODES);
        assert_eq!(layout.lost(), 0);
        // What the pass says it read, against what the slots say they answered.
        // Nothing else reads a slot during a pass, so these are the same number
        // arrived at from opposite ends, and `Work::reads` not being maintained at
        // all is a mutation this is the only assertion that catches.
        assert_eq!(layout.slot_reads() - before, work.reads);
        assert!(work.reads > u64::from(work.nodes), "measure and place each read a node");
    }

    #[test]
    fn a_delta_touching_five_nodes_of_ten_thousand_re_solves_five_subtrees() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");
        let full = layout.pass();

        // Five leaves, in five different sections, each asking for ten tenths of
        // a point more than it did.
        let touched: [u64; 5] = [0, 7, 108, 500, SECTIONS - 1];
        let before = layout.probes();
        let before_reads = layout.slot_reads();
        for at in touched {
            let changed = layout
                .declare(leaf(at, 0), section(at), Demand::flexible(LEAF_PT_X10 + 10, 0, Flow::Own))
                .expect("a leaf saying what it needs now");
            assert!(changed.changed);
            assert!(!changed.inserted);
        }
        let probes = layout.probes() - before;
        let slot_reads = layout.slot_reads() - before_reads;
        assert!(!layout.is_whole());
        assert_eq!(layout.marked(), 5);

        let reads_before = layout.slot_reads();
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("five subtrees, in scope");
        assert_eq!(layout.slot_reads() - reads_before, work.reads);
        let pass = layout.pass();
        assert_eq!(pass, full + 1);

        // The exit's number, and then the same number arrived at three other
        // ways so that the count cannot be the only witness to itself.
        assert_eq!(work.subtrees, 5);
        assert_eq!(work.marks, 5);
        assert_eq!(work.nodes, 5 * SUBTREE_NODES);
        assert_eq!(layout.nodes_read_in(pass), 5 * SUBTREE_NODES);
        assert_eq!(layout.nodes_read_in(full), TREE_NODES - 5 * SUBTREE_NODES);
        assert_eq!(layout.lost(), 0);

        // Which slots, and not only how many. Every node of the five subtrees
        // carries this pass's stamp.
        for at in touched {
            assert_eq!(layout.last_pass(section(at)), Some(pass), "section {at} was not re-solved");
            for which in 0..PER_SECTION {
                assert_eq!(
                    layout.last_pass(leaf(at, which)),
                    Some(pass),
                    "leaf {which} of section {at} was not re-solved"
                );
            }
        }
        // And the surface is not one of them, which is the whole claim: a change
        // under a definite ancestor does not reach the tree above it.
        assert_eq!(layout.last_pass(SURFACE), Some(full));
        for at in 0..SECTIONS {
            if touched.contains(&at) {
                continue;
            }
            assert_eq!(layout.last_pass(section(at)), Some(full), "section {at} was re-solved");
        }

        // Finding the five did not read the ten thousand either. A scan of the
        // slots would be five lookups of ten thousand probes; the index makes it
        // this.
        assert!(probes < 64, "finding five nodes cost {probes} index probes");
        assert!(slot_reads < 64, "finding and re-declaring five nodes read {slot_reads} slots");
    }

    #[test]
    fn two_changes_under_one_boundary_are_one_subtree() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");

        for which in [0, 4] {
            layout
                .declare(leaf(3, which), section(3), Demand::flexible(60, 0, Flow::Own))
                .expect("a leaf saying what it needs now");
        }
        assert_eq!(layout.marked(), 2);
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("one subtree, in scope");
        assert_eq!(work.marks, 2);
        assert_eq!(work.subtrees, 1);
        assert_eq!(work.nodes, SUBTREE_NODES);
        assert_eq!(layout.nodes_read_in(layout.pass()), SUBTREE_NODES);
    }

    #[test]
    fn a_boundary_inside_another_plan_entry_is_not_a_second_subtree() {
        // surface → group → panel → field. The group is not a boundary and the
        // panel is, so a change to the group is re-solved from the surface and a
        // change to the field from the panel — and the panel's subtree is inside
        // the surface's.
        const GROUP: NodeId = NodeId::new(10);
        const PANEL: NodeId = NodeId::new(11);
        const FIELD: NodeId = NodeId::new(12);
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 1, Flow::Block))
            .expect("a surface");
        layout.declare(GROUP, SURFACE, Demand::flexible(0, 1, Flow::Block)).expect("a group");
        layout.declare(PANEL, GROUP, Demand::definite(400, Flow::Block)).expect("a panel");
        layout.declare(FIELD, PANEL, Demand::flexible(100, 0, Flow::Own)).expect("a field");
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");

        layout.declare(GROUP, SURFACE, Demand::flexible(50, 1, Flow::Block)).expect("a group");
        layout.declare(FIELD, PANEL, Demand::flexible(120, 0, Flow::Own)).expect("a field");
        assert_eq!(layout.marked(), 2);
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("in scope");
        assert_eq!(work.marks, 2);
        assert_eq!(work.subtrees, 1, "the panel's subtree sits inside the surface's");
        assert_eq!(work.nodes, 4);
    }

    #[test]
    fn a_cascade_past_the_stated_scope_is_a_refusal_and_not_a_slow_frame() {
        let mut layout = Layout::new();
        build(&mut layout, unbounded_sections());
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");
        let full = layout.pass();
        let placed = layout.placed_pt_x10(leaf(4, 4)).expect("a leaf that was placed");

        layout
            .declare(leaf(4, 4), section(4), Demand::flexible(LEAF_PT_X10 + 10, 0, Flow::Own))
            .expect("a leaf saying what it needs now");
        let refused =
            layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect_err("no boundary to stop at");

        assert_eq!(refused.at, SURFACE);
        assert_eq!(refused.nodes, TREE_NODES);
        assert_eq!(refused.scope_nodes, Scope::DEFAULT.nodes_max);
        assert_eq!(refused.subtrees, 1);
        // Nothing was laid out, and the mark is still owed: a refusal is not a
        // half-solved tree.
        assert_eq!(layout.placed_pt_x10(leaf(4, 4)), Some(placed));
        assert_eq!(layout.marked(), 1);
        // The plan cost what a plan costs — the changed node and its ancestors —
        // and not what the refused work would have.
        assert!(layout.nodes_read_in(layout.pass()) <= 1 + DEPTH_MAX as u32);
        assert_eq!(layout.nodes_read_in(full), TREE_NODES - layout.nodes_read_in(layout.pass()));

        // Widening the scope is the caller's other option, and it works, which is
        // what makes the refusal a policy rather than a defect.
        let work = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a full solve");
        assert_eq!(work.subtrees, 1);
        assert_eq!(work.nodes, TREE_NODES);
    }

    #[test]
    fn a_definite_node_is_the_boundary_and_a_flexible_one_is_not() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        assert_eq!(layout.is_boundary(section(0)), Some(true));
        assert_eq!(layout.is_boundary(leaf(0, 0)), Some(false));
        assert_eq!(layout.is_boundary(SURFACE), Some(false));
        assert_eq!(layout.is_boundary(NodeId::new(7)), None);
    }

    #[test]
    fn the_leftover_is_shared_by_grow_and_nothing_is_lost_to_the_division() {
        const A: NodeId = NodeId::new(21);
        const B: NodeId = NodeId::new(22);
        const C: NodeId = NodeId::new(23);
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Inline))
            .expect("a surface");
        for id in [A, B, C] {
            layout.declare(id, SURFACE, Demand::flexible(0, 1, Flow::Own)).expect("a child");
        }
        let work = layout.solve(100, Scope::DEFAULT).expect("in scope");
        assert_eq!(work.capped, 0);

        // A hundred tenths of a point over three equal shares. Two children get
        // thirty-three and the third gets thirty-four, because the shares are
        // taken against a running total rather than one at a time — so the
        // remainder lands somewhere instead of evaporating.
        assert_eq!(layout.placed_pt_x10(A), Some((0, 33)));
        assert_eq!(layout.placed_pt_x10(B), Some((33, 33)));
        assert_eq!(layout.placed_pt_x10(C), Some((66, 34)));
        let total: i32 = [A, B, C]
            .iter()
            .filter_map(|id| layout.placed_pt_x10(*id))
            .map(|(_, extent)| extent)
            .sum();
        assert_eq!(total, 100, "every tenth of the leftover went to exactly one child");
    }

    #[test]
    fn a_maximum_stops_a_child_taking_its_share_and_the_residue_is_reported() {
        const NARROW: NodeId = NodeId::new(31);
        const WIDE: NodeId = NodeId::new(32);
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Inline))
            .expect("a surface");
        layout
            .declare(
                NARROW,
                SURFACE,
                Demand { min_pt_x10: 0, max_pt_x10: 10, grow: 1, flow: Flow::Own },
            )
            .expect("a child that will not grow past ten");
        layout
            .declare(WIDE, SURFACE, Demand::flexible(0, 1, Flow::Own))
            .expect("a child that will");
        let work = layout.solve(100, Scope::DEFAULT).expect("in scope");

        assert_eq!(work.capped, 1);
        assert_eq!(layout.placed_pt_x10(NARROW), Some((0, 10)));
        // Fifty went to the wide child and forty went nowhere, which is the
        // single distribution pass stated rather than hidden.
        assert_eq!(layout.placed_pt_x10(WIDE), Some((10, 50)));
    }

    #[test]
    fn a_definite_node_absorbs_children_that_ask_for_more_and_says_so() {
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Block))
            .expect("a surface");
        layout
            .declare(section(0), SURFACE, Demand::definite(100, Flow::Block))
            .expect("a section too small for what it holds");
        for which in 0..PER_SECTION {
            layout
                .declare(leaf(0, which), section(0), Demand::flexible(LEAF_PT_X10, 0, Flow::Own))
                .expect("a leaf");
        }
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("in scope");
        assert!(work.overflows >= 1);
        // The section is still exactly what it declared: the overflow is reported
        // and the boundary holds, because a boundary that moved under pressure
        // from below would not be one.
        assert_eq!(layout.placed_pt_x10(section(0)), Some((0, 100)));
        assert_eq!(layout.intrinsic_pt_x10(section(0)), Some(100));
    }

    #[test]
    fn a_node_that_arranges_its_own_children_is_not_descended_into() {
        const CANVAS: NodeId = NodeId::new(41);
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Block))
            .expect("a surface");
        layout
            .declare(CANVAS, SURFACE, Demand::flexible(200, 0, Flow::Own))
            .expect("a canvas, arranging itself");
        for which in 0..3 {
            layout
                .declare(leaf(0, which), CANVAS, Demand::flexible(10, 0, Flow::Own))
                .expect("an occupant the application places");
        }
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("in scope");

        // Two nodes read out of a plan of five. The plan is a bound and the work
        // is what happened; a canvas is where the two differ, which is stated
        // rather than left for a reader to notice in an assertion.
        assert_eq!(layout.subtree_nodes(SURFACE), Some(5));
        assert_eq!(work.nodes, 2);
        assert_eq!(layout.nodes_read_in(layout.pass()), 2);
        assert_eq!(layout.last_pass(leaf(0, 0)), Some(0));
    }

    #[test]
    fn what_a_layout_costs_and_what_every_nothing_in_it_is() {
        // Stated rather than discovered, because the number decides whether a
        // component can hold one: 672 KiB is ten thousand slots of sixty-four
        // bytes and an index of sixteen thousand `u16`s. A slot that grew by one
        // word would cost another eighty kilobytes, which is why this is an
        // assertion and not a comment.
        assert_eq!(core::mem::size_of::<Slot>(), 64);
        assert_eq!(core::mem::size_of::<Layout>(), 688_376);

        // And every `nothing` in it is zero, which is what makes a fresh one a
        // region of zero bytes rather than a region of set bits. RFC 0100's rule
        // is the reason; `SLOTS` is the argument.
        assert_eq!(Ix::NONE.0, 0);
        assert_eq!(INDEX_EMPTY, 0);
        let mut empty = Slot::EMPTY;
        let (view, _) = empty.visit(0);
        assert_eq!(
            view.demand,
            Demand { min_pt_x10: 0, max_pt_x10: 0, grow: 0, flow: Flow::Inline }
        );
        assert_eq!(view.parent, Ix::NONE);
        assert_eq!(view.subtree_nodes, 0);
        assert_eq!(view.intrinsic_pt_x10, 0);
        assert_eq!(view.extent_pt_x10, 0);
        assert_eq!(view.offset_pt_x10, 0);
    }

    #[test]
    fn nothing_marked_is_no_pass_at_all() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");
        let pass = layout.pass();
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("nothing to do");
        assert_eq!(work, Work::NOTHING);
        assert_eq!(layout.pass(), pass, "a pass that did nothing did not happen");
    }

    #[test]
    fn a_re_declaration_that_changes_nothing_marks_nothing() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");
        let again = layout
            .declare(leaf(2, 2), section(2), Demand::flexible(LEAF_PT_X10, 0, Flow::Own))
            .expect("the same declaration twice");
        assert!(!again.changed);
        assert_eq!(layout.marked(), 0);
    }

    #[test]
    fn more_changes_than_marks_max_is_a_whole_forest_and_not_an_approximation() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a first full solve");
        for at in 0..(MARKS_MAX as u64 + 1) {
            layout
                .declare(leaf(at, 0), section(at), Demand::flexible(60, 0, Flow::Own))
                .expect("a leaf saying what it needs now");
        }
        assert!(layout.is_whole());
        assert_eq!(layout.marked(), 0);
        let work = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a whole solve");
        assert_eq!(work.subtrees, 1);
        assert_eq!(work.nodes, TREE_NODES);
    }

    #[test]
    fn a_removal_takes_the_subtree_and_the_counts_with_it() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        let gone = layout.remove(section(5)).expect("a section that is here");
        assert_eq!(gone, SUBTREE_NODES as usize);
        assert_eq!(layout.live(), (TREE_NODES - SUBTREE_NODES) as usize);
        assert_eq!(layout.subtree_nodes(SURFACE), Some(TREE_NODES - SUBTREE_NODES));
        assert!(!layout.holds(section(5)));
        assert!(!layout.holds(leaf(5, 9)));
        assert!(layout.holds(leaf(6, 9)));
        assert_eq!(layout.remove(section(5)), Err(Refused::Unknown(section(5))));
        assert_eq!(layout.lost(), 0);
    }

    #[test]
    fn a_slot_a_removal_freed_is_declared_into_again() {
        let mut layout = Layout::new();
        build(&mut layout, bounded());
        let live = layout.live();
        layout.remove(section(5)).expect("a section that is here");
        for which in 0..PER_SECTION {
            layout
                .declare(leaf(5, which), SURFACE, Demand::flexible(LEAF_PT_X10, 0, Flow::Own))
                .expect("a leaf where the section was");
        }
        layout.declare(section(5), SURFACE, bounded()).expect("the section again, empty");
        assert_eq!(layout.live(), live);
        assert_eq!(layout.lost(), 0);
        let work = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a whole solve");
        assert_eq!(work.nodes, TREE_NODES);
    }

    #[test]
    fn the_declaration_refusals_each_have_a_case() {
        let mut layout = Layout::new();
        assert_eq!(
            layout.declare(NodeId::UNNAMED, NodeId::UNNAMED, Demand::definite(1, Flow::Own)),
            Err(Refused::Unnamed)
        );
        assert_eq!(
            layout.declare(SURFACE, NodeId::new(9), Demand::definite(1, Flow::Own)),
            Err(Refused::NoParent(NodeId::new(9)))
        );
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Block))
            .expect("a surface");
        layout.declare(section(0), SURFACE, bounded()).expect("a section");
        layout.declare(section(1), SURFACE, bounded()).expect("another section");
        assert_eq!(
            layout.declare(section(1), section(0), bounded()),
            Err(Refused::Reparented(section(1)))
        );

        // Seventeen levels: DEPTH_MAX parent steps is the last one admitted.
        let mut parent = SURFACE;
        for step in 1..=DEPTH_MAX as u64 {
            let here = NodeId::new(5_000 + step);
            let declared = layout
                .declare(here, parent, Demand::flexible(0, 0, Flow::Block))
                .expect("in depth");
            assert_eq!(declared.depth, step as u32);
            parent = here;
        }
        let too_deep = NodeId::new(9_999);
        assert_eq!(
            layout.declare(too_deep, parent, Demand::flexible(0, 0, Flow::Block)),
            Err(Refused::TooDeep(too_deep))
        );
        for refusal in [
            Refused::Unnamed,
            Refused::Full,
            Refused::NoParent(SURFACE),
            Refused::TooDeep(SURFACE),
            Refused::Reparented(SURFACE),
            Refused::Unknown(SURFACE),
        ] {
            assert!(!refusal.message().is_empty());
        }
    }

    #[test]
    fn a_layout_full_of_nodes_refuses_the_next_one() {
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Block))
            .expect("a surface");
        let mut at = 0u64;
        while layout.live() < NODES_MAX {
            at += 1;
            layout
                .declare(NodeId::new(100_000 + at), SURFACE, Demand::flexible(1, 0, Flow::Own))
                .expect("room for one more");
        }
        assert_eq!(layout.remaining(), 0);
        let over = NodeId::new(100_000 + at + 1);
        assert_eq!(
            layout.declare(over, SURFACE, Demand::flexible(1, 0, Flow::Own)),
            Err(Refused::Full)
        );
        assert_eq!(layout.lost(), 0);
    }

    #[test]
    fn what_a_theme_resolved_is_what_the_solver_is_given() {
        let (resolved, _) = resolve(&Theme::DEFAULT);
        let node = Node::new(NodeId::new(3), NodeId::UNNAMED, Role::Group)
            .with_layout(Constraints::flowing(Flow::Block).at_least(400).growing(2));
        let demand = Demand::of(&resolved, &node);
        let span = resolved.span(&node);

        assert_eq!(demand.min_pt_x10, span.min_pt_x10);
        assert_eq!(demand.max_pt_x10, span.max_pt_x10);
        assert_eq!(demand.grow, 2);
        assert_eq!(demand.flow, Flow::Block);
        // Four ems of a ten-and-a-half-point em at the default density. The
        // number is the token layer's, arrived at through its own arithmetic, and
        // this module neither reproduces it nor rounds it a second time.
        assert_eq!(demand.min_pt_x10, resolved.em_pt_x10() * 4);
        assert!(!demand.is_definite());
        assert!(demand.arranges());
    }

    #[test]
    fn an_operable_node_reaches_the_solver_already_raised_to_its_floor() {
        let (resolved, _) = resolve(&Theme::DEFAULT);
        // With an intent, because that is what `Resolved::floor_pt_x10` reads: a
        // node is held to the operable floor when it can be operated, and a
        // command with no capability behind it cannot be. The floor is the token
        // layer's rule and this test asserts that this module inherited it rather
        // than reproducing it.
        let tiny = Node::new(NodeId::new(4), NodeId::UNNAMED, Role::Command)
            .with_layout(Constraints::LEAF.at_least(1))
            .with_intent(CapRef::new(0x0601));
        let demand = Demand::of(&resolved, &tiny);
        // `E3-B06d` owns the floor and this module inherits it: the solver never
        // sees the one tenth of an em the author wrote, so there is no place here
        // for a second opinion about how small a control may be.
        assert_eq!(demand.min_pt_x10, resolved.operable_min_pt_x10());
        assert!(demand.min_pt_x10 > resolved.em_pt_x10() / 100);
    }

    #[test]
    fn an_inverted_demand_is_read_as_its_own_minimum_rather_than_refused() {
        let demand = Demand { min_pt_x10: 200, max_pt_x10: 100, grow: 0, flow: Flow::Own };
        assert_eq!(demand.ceiling_pt_x10(), 200);
        assert_eq!(intrinsic_pt_x10(demand, 0), 200);
    }

    #[test]
    fn the_intrinsic_of_the_three_kinds_of_node() {
        let definite = Demand::definite(500, Flow::Block);
        assert_eq!(intrinsic_pt_x10(definite, 9_000), 500, "a boundary absorbs its children");

        let arranging = Demand::flexible(100, 0, Flow::Block);
        assert_eq!(intrinsic_pt_x10(arranging, 0), 100, "its own minimum with no children");
        assert_eq!(intrinsic_pt_x10(arranging, 900), 900, "its children when they ask for more");

        let own = Demand::flexible(100, 0, Flow::Own);
        assert_eq!(intrinsic_pt_x10(own, 900), 100, "a canvas does not grow to fit occupants");

        let capped = Demand { min_pt_x10: 100, max_pt_x10: 400, grow: 0, flow: Flow::Block };
        assert_eq!(intrinsic_pt_x10(capped, 900), 400, "and a maximum still holds");
    }

    #[test]
    fn the_index_spreads_consecutive_identifiers_and_survives_churn() {
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 0, Flow::Block))
            .expect("a surface");
        // Enough churn to fill the index with tombstones several times over, so
        // that `compact` runs and the lookups still answer. Without it the probe
        // chains grow until every lookup is the scan the index exists to avoid.
        for round in 0..8u64 {
            for at in 0..2_000u64 {
                let id = NodeId::new(1_000_000 + round * 10_000 + at);
                layout.declare(id, SURFACE, Demand::flexible(1, 0, Flow::Own)).expect("a child");
            }
            for at in 0..2_000u64 {
                let id = NodeId::new(1_000_000 + round * 10_000 + at);
                assert!(layout.holds(id));
                layout.remove(id).expect("a child that is here");
            }
        }
        assert_eq!(layout.live(), 1);
        assert_eq!(layout.lost(), 0);

        let before = layout.probes();
        for at in 0..1_000u64 {
            let id = NodeId::new(4_000_000 + at);
            layout.declare(id, SURFACE, Demand::flexible(1, 0, Flow::Own)).expect("a child");
        }
        for at in 0..1_000u64 {
            assert!(layout.holds(NodeId::new(4_000_000 + at)));
        }
        let probes = layout.probes() - before;
        // Three thousand lookups over a thousand live nodes. A table whose
        // tombstones were never cleared would be reading thousands of buckets per
        // lookup by now.
        assert!(probes < 12_000, "a thousand declarations and a thousand lookups cost {probes}");
    }

    #[test]
    fn a_solve_over_a_forest_takes_every_root_and_a_change_under_one_takes_one() {
        const SECOND: NodeId = NodeId::new(2);
        let mut layout = Layout::new();
        for root in [SURFACE, SECOND] {
            layout
                .declare(root, NodeId::UNNAMED, Demand::flexible(0, 1, Flow::Block))
                .expect("a surface");
            layout.declare(section(root.value()), root, bounded()).expect("a section");
        }
        let work = layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a whole solve");
        assert_eq!(work.subtrees, 2);
        assert_eq!(work.nodes, 4);

        layout
            .declare(section(SECOND.value()), SECOND, Demand::definite(700, Flow::Block))
            .expect("a section saying what it needs now");
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("in scope");
        assert_eq!(work.subtrees, 1);
        assert_eq!(work.nodes, 2, "the second surface and its section, and not the first");
        assert_eq!(layout.placed_pt_x10(section(SECOND.value())), Some((0, 700)));
    }

    #[test]
    fn a_new_root_marks_itself_and_nothing_else() {
        const SECOND: NodeId = NodeId::new(2);
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(0, 1, Flow::Block))
            .expect("a surface");
        layout.solve(VIEWPORT_PT_X10, Scope::UNBOUNDED).expect("a whole solve");
        let first = layout.pass();
        layout
            .declare(SECOND, NodeId::UNNAMED, Demand::flexible(0, 1, Flow::Block))
            .expect("a second surface");
        let work = layout.solve(VIEWPORT_PT_X10, Scope::DEFAULT).expect("in scope");
        assert_eq!(work.subtrees, 1);
        assert_eq!(work.nodes, 1);
        assert_eq!(layout.last_pass(SURFACE), Some(first));
    }

    #[test]
    fn a_root_offered_less_than_it_needs_overflows_rather_than_shrinking() {
        let mut layout = Layout::new();
        layout
            .declare(SURFACE, NodeId::UNNAMED, Demand::flexible(800, 0, Flow::Block))
            .expect("a surface needing eighty points");
        let work = layout.solve(500, Scope::DEFAULT).expect("in scope");
        assert_eq!(work.overflows, 1);
        assert_eq!(layout.placed_pt_x10(SURFACE), Some((0, 800)));
    }

    #[test]
    fn the_mixing_is_a_pure_function_of_the_key() {
        // RFC 0004's concern is iteration order that varies per process, and this
        // is the assertion that says which side of that line the index falls on:
        // the same identifier answers the same bucket, always, and two adjacent
        // identifiers do not answer adjacent buckets.
        assert_eq!(bucket(NodeId::new(42)), bucket(NodeId::new(42)));
        let mut adjacent = 0;
        for at in 0..64u64 {
            if bucket(NodeId::new(1_000 + at)).abs_diff(bucket(NodeId::new(1_001 + at))) <= 1 {
                adjacent += 1;
            }
        }
        assert!(adjacent < 8, "{adjacent} of 64 consecutive identifiers landed side by side");
        // A published constant's published fixed point, so that an edit to the
        // arithmetic is a red test rather than a different tree with the same
        // behaviour.
        assert_eq!(mix(0), 0);
        assert_ne!(mix(1), mix(2));
        assert_ne!(mix(1) & INDEX_MASK as u64, mix(2) & INDEX_MASK as u64);
    }
}
