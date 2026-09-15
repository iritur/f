// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Reconciliation: the client rebuilds its whole tree every frame, and the wire
//! carries the differences.
//!
//! # The concession this is
//!
//! `docs/design/ring-scene-boot.html` section 11 calls it *the concession that
//! decides adoption*, and the sentence under that heading is the whole of this
//! module's brief: developers want to rebuild the tree every frame because it
//! is dramatically easier to reason about than manual delta bookkeeping, the
//! system wants retained for section 07's reasons, and the two are resolved in
//! the client library rather than in the protocol. Section 04's ring ergonomics
//! problem has the same shape and the same answer — if using the model
//! correctly requires understanding the model, it will not be used correctly.
//!
//! So the protocol keeps its retained form and nothing above it has to. A
//! client writes [`Node`]s into a slice in the order it wants them painted,
//! hands the slice to [`Reconciler::frame`], and gets back the deltas that turn
//! the tree the compositor is holding into the tree it just described.
//!
//! # Exactly the differences, and why *exactly* is the hard word
//!
//! A reconciler that emits a superset is not a reconciler that is slightly
//! slow. It is a reconciler that has given up the argument section 07 rests on:
//! fifty deltas against a 33 MB frame is four orders of magnitude, and one that
//! re-sends a subtree because a sibling moved has spent the margin the whole
//! pipeline was designed around. Worse, it is invisible — the tree that arrives
//! is right, the frame looks right, and the only symptom is a number nobody is
//! watching.
//!
//! Three things here are arranged around that word.
//!
//! - **A reorder emits moves, not removals.** The edit an index-keyed
//!   reconciler gets wrong: comparing sibling *positions* rather than sibling
//!   *keys*, `[a, b, c] → [c, a, b]` looks like three changed slots, and what
//!   comes out is three removals and three creations — plus every property of
//!   every recreated node, and the destruction of three subtrees. What comes
//!   out of this module is one entry. Keys are matched first and positions are
//!   derived from them, which is section 11's fourth rule — *identity is stable
//!   and author-assigned* — spent on the thing it was saved for.
//! - **The set of moved siblings is the smallest one there is.** Not merely
//!   small: the nodes left alone are a longest increasing subsequence of the
//!   previous sibling order, and the complement of a longest increasing
//!   subsequence is a minimum-size set of elements whose re-placement leaves
//!   the rest in order. A greedy left-to-right scan — the obvious
//!   implementation, and the one most keyed reconcilers ship — emits three
//!   moves for `[a, b, c, d] → [b, c, d, a]` where one suffices. That case is
//!   in the corpus below for exactly that reason.
//! - **A removal names the root of what vanished and nothing under it.**
//!   [`RemoveNode`] takes the subtree, so one entry per vanished *node* is a
//!   superset that also names nodes that stopped existing three entries ago.
//!
//! The corpus at the bottom of this file asserts the emitted sequence against a
//! hand-written one element by element, which is what makes *exactly* checkable
//! rather than claimed; it then applies that sequence to a model of the
//! compositor's side and requires the result to equal the client's tree, which
//! is what stops the hand-written sequence from being wrong in the same
//! direction as the code that produced it.
//!
//! # The one sentence the six opcodes can form that means *move*
//!
//! There is no `MoveNode`. Section 07's protocol is `CreateNode`,
//! `SetTransform`, `SetPath`, `SetPaint`, `RemoveNode`, `Commit`, and
//! [`CreateNode::before`] — the field that states sibling order, which is paint
//! order — appears on `CreateNode` and nowhere else. A reorder is therefore
//! expressible in exactly one way: **re-issue `CreateNode` for a node that
//! already exists, and read it as *hang this node here***. The node keeps its
//! identity, its kind, its properties and its subtree; only its place among its
//! siblings changes.
//!
//! That reading is not this module's convenience, it is the only reading under
//! which `before` is usable at all. The alternative — `CreateNode` on an
//! existing node replaces it — makes every reorder a destruction, which is the
//! failure this task exists to avoid, and leaves the format with no way to say
//! the most common edit a user interface performs. `Model` in the tests below
//! is that contract written as code rather than as a paragraph: it is the least
//! compositor that can accept what this module emits, and every corpus frame
//! runs through it.
//!
//! *What would reverse this:* a seventh opcode. If a compositor ever needs to
//! distinguish *create* from *move* — for a transition, an animation, or a cost
//! model that charges differently for the two — then `MoveNode` is an RFC and
//! an ABI change, and this module emits it instead. The reversal is cheap
//! precisely because the decision is written here once.
//!
//! # No clock, no seed, no allocator
//!
//! The output is a pure function of two trees. There is no clock to read and no
//! seed to take: this crate depends only on `f-abi`, and a function that draws
//! nothing is a stronger determinism statement than one that draws from a
//! seeded `Env`, because there is no draw to reproduce. Two architectures run
//! the same comparisons in the same order and emit the same entries, and the
//! encoding they cross in is `f_abi::scene`'s, which is little-endian by hand.
//!
//! No allocator and no `Vec`, this crate's rule: [`Reconciler`] holds the
//! previous tree and every scratch array it needs in one fixed allocation whose
//! size is its `NODES` parameter, and a tree that does not fit is
//! [`Refusal::Capacity`] rather than a panic or a growth.
//!
//! The cost is the search. Matching a key against the previous tree is a scan,
//! so a frame is quadratic in the node count rather than linear —
//! deliberately, because the alternative today is an index this crate would
//! have to build and maintain, and `E3-B01c`'s arena is where a key-to-slot
//! index is going to live anyway. *What would reverse this:* that arena landing
//! with such an index, or a measured tree large enough for the quadratic term
//! to show against section 12's budget, whichever comes first. No number is
//! claimed here either way — this module publishes none.

use f_abi::scene::{
    CreateNode, Entry, NO_NODE, RemoveNode, SetPaint, SetPath, SetTransform, fill, kind,
};

/// An index that names nothing: no previous node, no rank, no predecessor.
///
/// `u32::MAX` rather than an `Option<u32>`, because these live in arrays as
/// long as the tree and an `Option` would widen several of them for a niche the
/// type cannot find. A tree with this many nodes has been refused by
/// [`Refusal::Capacity`] long before, since `NODES` is what bounds it.
const NOWHERE: u32 = u32::MAX;

/// One, in the 16.16 fixed point [`SetTransform`] is written in.
///
/// Spelled here rather than inline so that the identity matrix below reads as a
/// matrix. RFC 0004 is why this is an integer and not a `1.0`.
/// Unit: none — a ratio, scaled by 65 536.
const ONE_X65536: i64 = 65_536;

/// Why a frame was not reconciled.
///
/// Local to this crate rather than a reuse of `f_abi::scene::Refusal`, and the
/// difference is the subject: that one is about an entry that arrived from a
/// peer, and it packs into RFC 0010's domains because it has to reach a
/// completion. Every refusal here is about the *client's own tree*, caught
/// before anything crosses anything, and handed to a caller that has not
/// submitted yet. Giving it a wire code would be inventing an error nobody can
/// receive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The tree has more nodes than this reconciler was sized for.
    ///
    /// Named rather than fatal, and named rather than silently truncated: a
    /// client that outgrew its reconciler should be told about the number it
    /// chose, not handed a frame missing whatever did not fit.
    Capacity,
    /// The frame needs more deltas than the buffer it was given holds.
    ///
    /// The frame is abandoned, because a commit is atomic: half a frame's
    /// deltas is not a smaller frame, it is a torn one. Nothing already written
    /// into the buffer may be submitted.
    Overflow,
    /// A node calls itself [`NO_NODE`], which is not a name.
    NoKey,
    /// Two nodes in one tree carry the same key.
    ///
    /// Identity is the whole basis of the diff below — a key matched against
    /// the previous tree is what turns a reorder into a move — so two nodes
    /// answering to one key is not an edit with a defined difference.
    DuplicateKey,
    /// A node's parent is not a node that appeared before it.
    ///
    /// The tree arrives in pre-order, and this is the rule that says so. It is
    /// one refusal covering three mistakes — a parent that does not exist, a
    /// node that is its own parent, and a child written above its parent — and
    /// that is the point: *a parent appears before its child* is not merely a
    /// convenience for the walk below, it makes a cycle unrepresentable rather
    /// than something a checker has to go looking for.
    OutOfOrder,
    /// A node's kind is not one of `f_abi::scene::kind`'s six.
    UnknownKind,
    /// A key that already exists came back with a different kind.
    ///
    /// Refused rather than quietly turned into a removal and a creation, which
    /// is what most reconcilers do. Section 11's fourth rule is that identity
    /// is stable and author-assigned; a key whose kind changed is an
    /// application that has reused an identifier, and the silent repair costs
    /// that node's whole subtree on a frame nobody was watching. The client is
    /// told instead, and the fix — a different key — is one it can make.
    KindChanged,
    /// A property record in a client's tree names a node.
    ///
    /// The records this module diffs are `f_abi::scene`'s own, so that a field
    /// added to the wire format is a field this module compares and carries
    /// without being edited. Each of them holds a `node`, which inside a
    /// [`Node`] is redundant: the node a property belongs to is the one holding
    /// it. Refused rather than ignored or overwritten, because a property with
    /// two possible node fields is a property two readers can disagree about —
    /// the emitter fills it in on the way out, and in a tree it is
    /// [`NO_NODE`].
    PropertyNamed,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Capacity => "the tree has more nodes than this reconciler holds",
            Self::Overflow => "the frame needs more deltas than the buffer given for it",
            Self::NoKey => "a node carries no key",
            Self::DuplicateKey => "two nodes in one tree carry the same key",
            Self::OutOfOrder => "a node's parent is not a node that appeared before it",
            Self::UnknownKind => "a node's kind is not one of the six",
            Self::KindChanged => "a key that already exists came back with a different kind",
            Self::PropertyNamed => "a property record in a client's tree names a node",
        }
    }
}

/// The properties a node has, written once.
///
/// # Why a macro here and nowhere else in this crate
///
/// Because a property is otherwise four agreeing places: a field on [`Node`],
/// the value a freshly created node starts from, the comparison that decides
/// whether it changed, and the delta that states the new value. A fifth
/// property added to three of the four is a property the reconciler never
/// emits — the frame is simply wrong, on a path no test names, in the direction
/// that is hardest to see, because the tree that arrives is *nearly* right.
///
/// `interface/src/node.rs`'s `vocabulary!` is the precedent and the argument is
/// the same one: a hand-written second list cannot be checked by a loop over
/// itself, and an exhaustive match only demands an arm and not an arm that says
/// anything. What replaced those is not a better test; it is the absence of a
/// second place to write the thing.
///
/// The identity fields are emitted here too, so the struct in full comes off
/// this one invocation, and `emit_properties` destructures a node with no `..`
/// — which is what stops a field being added to `Node` outside the list. It
/// would not be compared, it would not be emitted, and it does not compile.
macro_rules! properties {
    (
        $(
            $(#[$about:meta])*
            $field:ident : $record:ident / $variant:ident = $initial:expr;
        )*
    ) => {
        /// One node of the tree a client rebuilds every frame.
        ///
        /// A flat slice of these in **pre-order** is a tree: a node's parent is
        /// named by key and must appear earlier in the slice, and sibling order
        /// — which is paint order, section 07 — is the order they appear in.
        /// There is no child list and no pointer, which is what lets a client
        /// build a frame into a fixed array, and what makes a subtree a
        /// contiguous range for anything that has to move one.
        ///
        /// Every field is diffed. The three identity fields are diffed
        /// structurally — a key that is new is a creation, a key that vanished
        /// is a removal, a parent or a sibling position that changed is a
        /// re-hang, and a kind that changed is [`Refusal::KindChanged`] — and
        /// the rest are the properties below, each of which is one delta when
        /// it differs and nothing when it does not.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Node {
            /// What this node answers to, stably, across every frame it appears
            /// in.
            ///
            /// The application's own identifier, in the application's own
            /// space, and the same number that crosses as [`CreateNode::node`].
            /// Section 11's fourth rule: a visual redesign must not change it.
            /// Everything this module can do for a reorder rests on this number
            /// being the same number next frame.
            /// Unit: none — a node identifier, not a quantity. [`NO_NODE`] is
            /// [`Refusal::NoKey`].
            pub key: u32,
            /// The node this one hangs under.
            /// Unit: none — the [`Node::key`] of a node earlier in the slice.
            /// [`NO_NODE`] is a root, which is how a client's own top-level
            /// node enters the compositor's tree.
            pub parent: u32,
            /// What kind of node this is.
            /// Unit: none — an `f_abi::scene::kind` constant. Zero is not a
            /// kind, and neither is anything outside the six.
            pub kind: u16,
            $(
                $(#[$about])*
                pub $field: $record,
            )*
        }

        impl Node {
            /// How many settable properties a node has.
            ///
            /// Counted from the list rather than written down, so it cannot
            /// disagree with what it counts.
            /// Unit: properties.
            pub const PROPERTY_COUNT: usize = [$(stringify!($field)),*].len();

            /// A node with every property at the value a node holds the instant
            /// a `CreateNode` brings it into existence.
            ///
            /// The starting point for an immediate-mode client: build one of
            /// these and overwrite what this frame means. It matters that this
            /// is the same constant the diff compares a new node against — a
            /// client that leaves a property alone emits no delta for it, which
            /// is what *exactly the differences* means on the frame a node is
            /// born.
            #[must_use]
            pub const fn new(key: u32, parent: u32, kind: u16) -> Self {
                Self { key, parent, kind, $( $field: $initial, )* }
            }

            /// The value of an array slot that holds no node.
            ///
            /// Never read: `held` and every scratch array are bounded by the
            /// length beside them. It carries a kind of zero anyway, so that if
            /// one ever did escape into a tree it would be refused by
            /// [`Refusal::UnknownKind`] rather than accepted as a node — the
            /// filler is subject to the same check everything else is.
            pub const UNUSED: Self = Self::new(NO_NODE, NO_NODE, 0);
        }

        /// Every property that differs, in declaration order, and nothing else.
        ///
        /// `held` is the node as the compositor holds it, or `None` for a node
        /// that did not exist a moment ago — which is a node whose properties
        /// are compared against the value [`Node::new`] starts from, because
        /// that is what the `CreateNode` in front of these deltas just gave it.
        fn emit_properties(
            node: &Node,
            held: Option<&Node>,
            out: &mut Emit,
        ) -> Result<(), Refusal> {
            // No `..` in this pattern, and that is the guard rather than
            // decoration: a field added to `Node` outside the `properties!`
            // list stops compiling *here*, in the one function that decides
            // what counts as a difference. The three identity fields are bound
            // and discarded because they are diffed structurally instead.
            let Node { key, parent: _, kind: _, $($field,)* } = *node;
            $(
                {
                    let held_value = match held { Some(held) => held.$field, None => $initial };
                    if $field != held_value {
                        let mut set = $field;
                        // The one place a property record learns whose it is.
                        // In a tree the field is `NO_NODE` — `check_properties`
                        // refuses anything else — so there is never a second
                        // answer being overwritten here.
                        set.node = key;
                        out.push(Entry::$variant(set))?;
                    }
                }
            )*
            Ok(())
        }

        /// No property record in a client's tree names a node.
        ///
        /// Emitted from the same list as the fields, so a fourth property is
        /// checked on the day it is declared.
        const fn check_properties(node: &Node) -> Result<(), Refusal> {
            $(
                if node.$field.node != NO_NODE {
                    return Err(Refusal::PropertyNamed);
                }
            )*
            Ok(())
        }
    };
}

properties! {
    /// The affine matrix applied to this node's subtree.
    ///
    /// `f_abi::scene`'s own record, minus the node it names — see
    /// [`Refusal::PropertyNamed`] — and reusing the wire record is deliberate
    /// rather than lazy: a field added to it is a field this module's derived
    /// `PartialEq` compares and this module's emitter carries, without a line
    /// of this file changing. A transcription of the six numbers into a local
    /// struct would be a copy that goes stale in silence, which is the defect
    /// the list above exists to prevent one level up.
    /// Unit: none — 16.16 fixed point, the scale in each field's name. The
    /// initial value is the identity, which is what a node with no transform of
    /// its own means.
    transform: SetTransform / SetTransform = SetTransform {
        node: NO_NODE,
        a_x65536: ONE_X65536,
        b_x65536: 0,
        c_x65536: 0,
        d_x65536: ONE_X65536,
        tx_x65536: 0,
        ty_x65536: 0,
    };

    /// The geometry this node draws, as a range of the channel's inline arena.
    ///
    /// A new node encloses nothing, so the range is empty. The fill rule of an
    /// empty path is unobservable and is still written down, because the wire
    /// refuses a rule of zero: `NON_ZERO` is what almost every vector format
    /// means by default, and a client that means the other one says so and gets
    /// a delta for it.
    /// Unit: bytes, from the first byte of the arena.
    path: SetPath / SetPath = SetPath {
        node: NO_NODE,
        geometry_offset: 0,
        geometry_bytes: 0,
        fill_rule: fill::NON_ZERO,
    };

    /// What this node is painted with.
    ///
    /// A new node is fully transparent — alpha zero, no stroke — so a node that
    /// is created and never painted costs one delta and not two. Straight alpha
    /// and linear light, both the wire record's terms and neither of them this
    /// module's to reinterpret.
    /// Unit: none — intensities scaled so that 65 535 is full.
    paint: SetPaint / SetPaint = SetPaint {
        node: NO_NODE,
        red_x65535: 0,
        green_x65535: 0,
        blue_x65535: 0,
        alpha_x65535: 0,
        stroke_width_x65536: 0,
    };
}

/// A frame's deltas, in the order they are submitted.
///
/// A fixed buffer with a named maximum rather than a growing one, this crate's
/// rule. `LIMIT` is the client's own statement of the largest frame it intends
/// to produce — section 07's arithmetic is drawn for five to fifty deltas — and
/// a client that exceeds its own number is told so by [`Refusal::Overflow`]
/// rather than by an allocator.
#[derive(Clone, Copy, Debug)]
pub struct Deltas<const LIMIT: usize> {
    /// The entries. Only the first `len` of them mean anything.
    entries: [Entry; LIMIT],
    /// How many of them a frame produced.
    len: usize,
}

impl<const LIMIT: usize> Deltas<LIMIT> {
    /// What an unwritten slot holds.
    ///
    /// A removal of [`NO_NODE`], which `f_abi::scene`'s decoder refuses: if one
    /// ever reached a peer it would be rejected as a malformed entry rather
    /// than applied as an edit. Nothing should read one — `len` bounds every
    /// slice this type hands out — and the filler is chosen so that the failure
    /// of that sentence is loud.
    const FILLER: Entry = Entry::RemoveNode(RemoveNode { node: NO_NODE });

    /// An empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self { entries: [Self::FILLER; LIMIT], len: 0 }
    }

    /// The frame's deltas.
    ///
    /// Submit them in this order, and then a `Commit`. The commit is the
    /// caller's and not this module's: it carries a frame token and the
    /// deadline the frame was scheduled against, and that deadline comes from
    /// the pacing loop of `E3-B01h` — a clock this crate cannot read and should
    /// not pretend to.
    #[must_use]
    pub fn as_slice(&self) -> &[Entry] {
        &self.entries[..self.len]
    }
}

impl<const LIMIT: usize> Default for Deltas<LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the deltas go, and the one place the buffer's limit is enforced.
struct Emit<'a> {
    /// The caller's buffer.
    out: &'a mut [Entry],
    /// How much of it has been written.
    at: usize,
}

impl Emit<'_> {
    /// Append one delta, or refuse the frame.
    fn push(&mut self, entry: Entry) -> Result<(), Refusal> {
        if self.at == self.out.len() {
            return Err(Refusal::Overflow);
        }
        self.out[self.at] = entry;
        self.at += 1;
        Ok(())
    }
}

/// The working memory one frame needs, sized once.
///
/// Every array is indexed by a position in one of the two trees, and every one
/// of them exists because there is no allocator to put it anywhere else. They
/// are fields of the reconciler rather than locals of a function so that the
/// cost is paid at construction, where a client can see it, rather than on a
/// stack during a frame.
struct Scratch<const NODES: usize> {
    /// For each node of the new tree, where the compositor's copy of it is, or
    /// [`NOWHERE`] for a node that does not exist yet — including one whose key
    /// survives but whose previous ancestor is being removed out from under it.
    baseline: [u32; NODES],
    /// For each node of the held tree, whether this frame destroys it: its key
    /// is absent from the new tree, or an ancestor's is.
    gone: [bool; NODES],
    /// For each node of the held tree, its position among the surviving
    /// children of the group being reconciled, or [`NOWHERE`].
    rank_of_held: [u32; NODES],
    /// The group's members: positions in the new tree, in the new tree's order.
    member: [u32; NODES],
    /// Each member's position among its previous siblings, or [`NOWHERE`] for a
    /// member that has no previous position under this parent at all — a new
    /// node, or one that came from another parent. Those can never be left
    /// alone, which is why they are never in the chain below.
    rank: [u32; NODES],
    /// The patience-sort tails: for each length, the member ending the
    /// increasing subsequence of that length whose rank is smallest.
    tails: [u32; NODES],
    /// Each member's predecessor in the increasing subsequence ending at it.
    link: [u32; NODES],
    /// Whether a member is left where it is. The complement is what moves.
    keep: [bool; NODES],
}

/// Immediate-mode in, deltas out.
///
/// It holds the tree the compositor holds — that is the entire state — and
/// every frame it is handed a new tree and answers with the difference. Size it
/// for the largest tree the client will build: `NODES` bounds both trees and
/// every scratch array, and anything larger is [`Refusal::Capacity`].
///
/// A refused frame changes nothing. The held tree is adopted only when a frame
/// is emitted in full, so a client that overflows its buffer, or names a kind
/// that does not exist, can repair the frame and submit it against the same
/// compositor state.
pub struct Reconciler<const NODES: usize> {
    /// The tree the compositor holds, in pre-order.
    held: [Node; NODES],
    /// How much of it is a tree.
    held_len: usize,
    /// One frame's working memory.
    scratch: Scratch<NODES>,
}

impl<const NODES: usize> Reconciler<NODES> {
    /// A reconciler holding nothing, which is what a compositor holds before a
    /// client's first frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: [Node::UNUSED; NODES],
            held_len: 0,
            scratch: Scratch {
                baseline: [NOWHERE; NODES],
                gone: [false; NODES],
                rank_of_held: [NOWHERE; NODES],
                member: [NOWHERE; NODES],
                rank: [NOWHERE; NODES],
                tails: [NOWHERE; NODES],
                link: [NOWHERE; NODES],
                keep: [false; NODES],
            },
        }
    }

    /// The tree this reconciler believes the compositor is holding.
    ///
    /// Exposed because a client whose frame was refused on the *wire* needs to
    /// be able to say so — the two sides have diverged, and the repair starts
    /// with comparing them. Nothing here can detect that divergence on its own:
    /// this crate does not read completions.
    #[must_use]
    pub fn held(&self) -> &[Node] {
        &self.held[..self.held_len]
    }

    /// The deltas that turn the held tree into `next`.
    ///
    /// Three phases, and the order between them is required rather than chosen.
    /// Removals first, so that nothing later names a node on its way out.
    /// Structure second, parents before children, so that a node exists before
    /// anything is hung under it. Properties last, so that every node a
    /// property names has been created. Inside a sibling group the structural
    /// entries come out in reverse order, which is the one ordering under which
    /// [`CreateNode::before`] always names a sibling already standing in its
    /// final place.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]. Every one but [`Refusal::Overflow`] is decided before a
    /// single delta is written, so a malformed tree costs nothing. On any
    /// refusal the held tree is unchanged and `out` must not be submitted: a
    /// commit is all of a frame or none of it.
    pub fn frame<const LIMIT: usize>(
        &mut self,
        next: &[Node],
        out: &mut Deltas<LIMIT>,
    ) -> Result<(), Refusal> {
        out.len = 0;
        if next.len() > NODES {
            return Err(Refusal::Capacity);
        }
        validate(next)?;

        let Self { held, held_len, scratch } = self;
        let previous: &[Node] = &held[..*held_len];

        // What this frame destroys, in one forward sweep. The tree is in
        // pre-order, so a node's parent has already been answered for by the
        // time the node is reached, and *destroyed because an ancestor was*
        // needs no second walk.
        for (i, node) in previous.iter().enumerate() {
            let vanished = find(next, node.key).is_none();
            let orphaned = match find(&previous[..i], node.parent) {
                Some(parent) => scratch.gone[parent],
                None => false,
            };
            scratch.gone[i] = vanished || orphaned;
        }

        // Where each new node's previous self is, if it has one that survives.
        // A node whose key is in both trees but whose previous ancestor is
        // being removed is *not* a survivor: the removal takes the subtree with
        // it, so the node is born again and everything it holds is new. A
        // reconciler that read *the key is in both trees* as *the node
        // survived* would emit a move for a node that no longer exists.
        for (j, node) in next.iter().enumerate() {
            scratch.baseline[j] = match find(previous, node.key) {
                Some(i) if !scratch.gone[i] => {
                    if previous[i].kind != node.kind {
                        return Err(Refusal::KindChanged);
                    }
                    u32::try_from(i).unwrap_or(NOWHERE)
                }
                _ => NOWHERE,
            };
        }

        let emitted = {
            let mut emit = Emit { out: &mut out.entries, at: 0 };

            // Phase one. The root of each vanished subtree, and nothing under
            // it: `RemoveNode` takes the subtree, so a second entry for a child
            // would name a node that stopped existing one entry earlier.
            for (i, node) in previous.iter().enumerate() {
                if find(next, node.key).is_some() {
                    continue;
                }
                let under_a_removal =
                    matches!(find(&previous[..i], node.parent), Some(p) if scratch.gone[p]);
                if !under_a_removal {
                    emit.push(Entry::RemoveNode(RemoveNode { node: node.key }))?;
                }
            }

            // Phase two, one sibling group at a time: the roots first, then the
            // children of each node in the new tree's pre-order. That reaches a
            // parent's group before any of its children's groups, so a node is
            // always created before anything is hung under it.
            group(NO_NODE, next, previous, scratch, &mut emit)?;
            for node in next {
                group(node.key, next, previous, scratch, &mut emit)?;
            }

            // Phase three. Every property that differs, against the node's own
            // previous self or against what the `CreateNode` in phase two just
            // gave it.
            for (j, node) in next.iter().enumerate() {
                let baseline = scratch.baseline[j];
                let held_node =
                    if baseline == NOWHERE { None } else { Some(&previous[baseline as usize]) };
                emit_properties(node, held_node, &mut emit)?;
            }

            emit.at
        };

        out.len = emitted;
        held[..next.len()].copy_from_slice(next);
        *held_len = next.len();
        Ok(())
    }
}

impl<const NODES: usize> Default for Reconciler<NODES> {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything about a tree that can be decided without looking at the previous
/// one.
///
/// All of it before any delta is emitted, so a malformed tree costs nothing and
/// a client never has to reason about a half-written buffer.
fn validate(next: &[Node]) -> Result<(), Refusal> {
    for (j, node) in next.iter().enumerate() {
        if node.key == NO_NODE {
            return Err(Refusal::NoKey);
        }
        if !kind::known(node.kind) {
            return Err(Refusal::UnknownKind);
        }
        check_properties(node)?;
        let earlier = &next[..j];
        if find(earlier, node.key).is_some() {
            return Err(Refusal::DuplicateKey);
        }
        // A root names no parent; anything else names one that has already been
        // seen. A node cannot be earlier than itself, so this is also what
        // refuses a node that is its own parent, and by induction what leaves a
        // cycle with nowhere to be written.
        if node.parent != NO_NODE && find(earlier, node.parent).is_none() {
            return Err(Refusal::OutOfOrder);
        }
    }
    Ok(())
}

/// Where a key is in a tree.
///
/// A scan, and the module's *no clock, no seed, no allocator* says why it is
/// allowed to be one and what would change it.
fn find(tree: &[Node], key: u32) -> Option<usize> {
    tree.iter().position(|node| node.key == key)
}

/// Reconcile one sibling group: the children of `parent`, in order.
///
/// This is where a reorder becomes a move. The members are matched to their
/// previous positions by key, the largest set of them that is already in the
/// right relative order is left alone, and every other member is re-hung —
/// which is the minimum number of entries that can put a permutation right.
///
/// # Errors
///
/// [`Refusal::Overflow`] if the caller's buffer fills.
fn group<const NODES: usize>(
    parent: u32,
    next: &[Node],
    previous: &[Node],
    scratch: &mut Scratch<NODES>,
    emit: &mut Emit,
) -> Result<(), Refusal> {
    let mut members = 0usize;
    for (j, node) in next.iter().enumerate() {
        if node.parent == parent {
            scratch.member[members] = u32::try_from(j).unwrap_or(NOWHERE);
            members += 1;
        }
    }
    if members == 0 {
        return Ok(());
    }

    // The previous sibling order, counted once for the whole group rather than
    // searched for per member. A node this frame destroys is not counted: it is
    // gone by the time any of this is applied, so it occupies no position for
    // the rest to be ordered against.
    let mut rank = 0u32;
    for (i, node) in previous.iter().enumerate() {
        scratch.rank_of_held[i] = NOWHERE;
        if node.parent == parent && !scratch.gone[i] {
            scratch.rank_of_held[i] = rank;
            rank += 1;
        }
    }
    for m in 0..members {
        let j = scratch.member[m] as usize;
        let baseline = scratch.baseline[j];
        // `rank_of_held` is `NOWHERE` for a node that was a child of somebody
        // else, which is exactly right: a node that changed parent has no
        // previous position in *this* group and must be re-hung whatever the
        // rest of the group does.
        scratch.rank[m] =
            if baseline == NOWHERE { NOWHERE } else { scratch.rank_of_held[baseline as usize] };
        scratch.keep[m] = false;
    }

    // The longest increasing subsequence of the previous positions, by patience
    // sorting. The members in it are the ones left alone, and the complement of
    // a longest increasing subsequence is a minimum-size set of elements whose
    // re-placement leaves the rest in order — which is why this is here rather
    // than the greedy scan that is three lines shorter and emits three moves
    // for `[a, b, c, d] -> [b, c, d, a]` where one is enough.
    let mut chain = 0usize;
    for m in 0..members {
        let rank = scratch.rank[m];
        if rank == NOWHERE {
            continue;
        }
        let mut lo = 0usize;
        let mut hi = chain;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if scratch.rank[scratch.tails[mid] as usize] < rank {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        scratch.link[m] = if lo == 0 { NOWHERE } else { scratch.tails[lo - 1] };
        scratch.tails[lo] = u32::try_from(m).unwrap_or(NOWHERE);
        if lo == chain {
            chain += 1;
        }
    }
    let mut at = if chain == 0 { NOWHERE } else { scratch.tails[chain - 1] };
    while at != NOWHERE {
        scratch.keep[at as usize] = true;
        at = scratch.link[at as usize];
    }

    // Backwards, and that is the whole reason this loop reads oddly: a
    // `CreateNode` states its position as *in front of this sibling*, so the
    // sibling has to be standing in its final place already. Walking the group
    // from the end makes that true by construction — everything to the right of
    // a member has been placed by the time the member is — and it is true for a
    // node being created for the first time and for one being moved, which are
    // the same entry and deliberately so.
    for m in (0..members).rev() {
        if scratch.keep[m] {
            continue;
        }
        let node = &next[scratch.member[m] as usize];
        let before =
            if m + 1 < members { next[scratch.member[m + 1] as usize].key } else { NO_NODE };
        emit.push(Entry::CreateNode(CreateNode {
            node: node.key,
            parent,
            before,
            kind: node.kind,
        }))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_abi::{NO_DEADLINE, scene::Delta};

    /// The size every corpus tree is reconciled at. Small, because the corpus
    /// is about edits and not about capacity; the bound itself is observed by
    /// `the_tree_may_not_outgrow_the_reconciler`.
    const NODES: usize = 16;
    /// The delta budget every corpus frame is emitted into.
    const LIMIT: usize = 32;

    /// A root that holds the siblings the corpus rearranges.
    const ROOT: u32 = 1;
    /// A first child. Named rather than numbered at the call site, because the
    /// corpus is read as *what moved*, and `[c, a, b]` only reads that way if
    /// the letters survive into the source.
    const A: u32 = 10;
    /// A second.
    const B: u32 = 11;
    /// A third.
    const C: u32 = 12;
    /// A fourth.
    const D: u32 = 13;
    /// A child of `A`, which exists to be destroyed along with it.
    const A1: u32 = 20;

    /// The root every corpus tree starts with.
    fn root() -> Node {
        Node::new(ROOT, NO_NODE, kind::LAYER)
    }

    /// The entry that creates that root.
    fn root_created() -> Entry {
        Entry::CreateNode(CreateNode {
            node: ROOT,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind::LAYER,
        })
    }

    /// A drawable child.
    fn draw(key: u32, parent: u32) -> Node {
        Node::new(key, parent, kind::DRAW)
    }

    /// A drawable child whose paint is not the one a new node has.
    fn tinted(key: u32, parent: u32, green_x65535: u16) -> Node {
        let mut node = draw(key, parent);
        node.paint.green_x65535 = green_x65535;
        node.paint.alpha_x65535 = u16::MAX;
        node
    }

    /// The `CreateNode` a corpus expectation is written as.
    fn create(node: u32, parent: u32, before: u32) -> Entry {
        Entry::CreateNode(CreateNode { node, parent, before, kind: kind::DRAW })
    }

    /// The paint delta a [`tinted`] node is expected to produce.
    ///
    /// Built from the same constructor the tree is, so the expectation carries
    /// no second transcription of what a paint record holds: a field added to
    /// `SetPaint` moves both sides of this test at once.
    fn painted(key: u32, green_x65535: u16) -> Entry {
        let mut paint = tinted(key, NO_NODE, green_x65535).paint;
        paint.node = key;
        Entry::SetPaint(paint)
    }

    /// Every delta this module emits is one a peer accepts.
    ///
    /// The exit is about *which* deltas come out; this is the other half that
    /// nobody should have to take on trust — that each of them survives the
    /// encoder and the decoder in `f_abi::scene`, which refuses a node of zero,
    /// a kind outside the six, a fill rule outside the two, and a deadline on
    /// an opcode that does not read one.
    fn crosses_the_wire(entry: &Entry) {
        let delta = Delta {
            user_data: 0,
            class: 0,
            deadline: NO_DEADLINE,
            payload_offset: 0,
            flags: 0,
            body: *entry,
        };
        let (sqe, payload) = delta.encode();
        let back = Delta::decode(&sqe, &payload).unwrap_or_else(|refusal| {
            panic!("a peer refused an emitted delta: {}", refusal.message())
        });
        assert_eq!(back, delta);
    }

    /// The compositor's side of the contract, as the least code that can hold
    /// it.
    ///
    /// Not a stand-in for `E3-B01c`'s graph and not a second implementation of
    /// anything in this file: it is the reading of `CreateNode` that the module
    /// documentation argues for, written as code so that *a re-issued create is
    /// a re-hang, and the node keeps its identity, its kind, its properties and
    /// its subtree* is executable rather than a paragraph somebody has to agree
    /// with. Every corpus frame is applied to it, and the result is required to
    /// equal the tree the client described — which is what stops a hand-written
    /// expectation from being wrong in the same direction as the code that
    /// produced it.
    struct Model {
        /// The tree, in pre-order, exactly as [`Node`] means it.
        nodes: [Node; NODES],
        /// How much of it is a tree.
        len: usize,
    }

    impl Model {
        /// An empty compositor.
        const fn new() -> Self {
            Self { nodes: [Node::UNUSED; NODES], len: 0 }
        }

        /// The tree.
        fn as_slice(&self) -> &[Node] {
            &self.nodes[..self.len]
        }

        /// Where a key is.
        fn at(&self, key: u32) -> usize {
            find(self.as_slice(), key).expect("a delta names a node the compositor holds")
        }

        /// The half-open range one subtree occupies.
        ///
        /// Contiguous, because the array is in pre-order. That is the whole
        /// reason this model can move a subtree with two copies rather than a
        /// traversal.
        fn extent(&self, root: usize) -> (usize, usize) {
            let mut end = root + 1;
            while end < self.len && self.descends(end, root) {
                end += 1;
            }
            (root, end)
        }

        /// Is `at` under `root`?
        fn descends(&self, at: usize, root: usize) -> bool {
            let mut walk = at;
            loop {
                let parent = self.nodes[walk].parent;
                if parent == NO_NODE {
                    return false;
                }
                let parent = self.at(parent);
                if parent == root {
                    return true;
                }
                walk = parent;
            }
        }

        /// The index a child of `parent` takes when it is placed in front of
        /// `before`.
        fn insertion(&self, parent: u32, before: u32) -> usize {
            if before != NO_NODE {
                return self.at(before);
            }
            if parent == NO_NODE {
                return self.len;
            }
            let parent = self.at(parent);
            self.extent(parent).1
        }

        /// Apply one delta.
        fn apply(&mut self, entry: &Entry) {
            match *entry {
                Entry::CreateNode(create) => self.hang(create),
                Entry::RemoveNode(remove) => {
                    let (start, end) = self.extent(self.at(remove.node));
                    self.nodes.copy_within(end..self.len, start);
                    self.len -= end - start;
                }
                Entry::SetTransform(mut set) => {
                    let at = self.at(set.node);
                    set.node = NO_NODE;
                    self.nodes[at].transform = set;
                }
                Entry::SetPath(mut set) => {
                    let at = self.at(set.node);
                    set.node = NO_NODE;
                    self.nodes[at].path = set;
                }
                Entry::SetPaint(mut set) => {
                    let at = self.at(set.node);
                    set.node = NO_NODE;
                    self.nodes[at].paint = set;
                }
                Entry::Commit(_) => panic!("the reconciler does not close the frame"),
            }
        }

        /// Hang a node: create it if it is new, move it if it is not.
        fn hang(&mut self, create: CreateNode) {
            let Some(existing) = find(self.as_slice(), create.node) else {
                let at = self.insertion(create.parent, create.before);
                self.nodes.copy_within(at..self.len, at + 1);
                self.nodes[at] = Node::new(create.node, create.parent, create.kind);
                self.len += 1;
                return;
            };
            assert_eq!(
                self.nodes[existing].kind, create.kind,
                "a re-hang does not change what a node is"
            );
            let (start, end) = self.extent(existing);
            let width = end - start;
            let mut lifted = [Node::UNUSED; NODES];
            lifted[..width].copy_from_slice(&self.nodes[start..end]);
            self.nodes.copy_within(end..self.len, start);
            self.len -= width;
            let to = self.insertion(create.parent, create.before);
            self.nodes.copy_within(to..self.len, to + width);
            self.nodes[to..to + width].copy_from_slice(&lifted[..width]);
            self.len += width;
            self.nodes[to].parent = create.parent;
        }
    }

    /// One frame of the corpus: the edit, the deltas it must produce, and the
    /// four things asserted about them.
    ///
    /// The first is the exit's: the emitted sequence against a hand-written
    /// one, element by element and length included, so a superset that happens
    /// to produce the right tree fails here. The second is that every entry is
    /// one a peer accepts. The third is that the sequence is *sufficient* —
    /// applied to the model, it yields the tree the client described. The
    /// fourth is that it was complete: reconciling the same tree again emits
    /// nothing, which would not hold if a difference had been left unstated.
    fn step(
        reconciler: &mut Reconciler<NODES>,
        model: &mut Model,
        next: &[Node],
        expected: &[Entry],
    ) {
        let mut out = Deltas::<LIMIT>::new();
        reconciler.frame(next, &mut out).expect("the corpus is legal");
        assert_eq!(out.as_slice(), expected);
        for entry in out.as_slice() {
            crosses_the_wire(entry);
            model.apply(entry);
        }
        assert_eq!(model.as_slice(), next);
        assert_eq!(reconciler.held(), next);

        let mut again = Deltas::<LIMIT>::new();
        reconciler.frame(next, &mut again).expect("the corpus is legal");
        assert!(again.as_slice().is_empty(), "the frame before it did not state every difference");
    }

    #[test]
    fn a_first_frame_is_the_whole_tree_and_nothing_more() {
        // Four creations in reverse sibling order, because `before` has to name
        // a sibling that is already standing, and one paint delta for the one
        // node whose paint is not what a new node's is. The other two cost one
        // entry each and not four: a client that leaves a property alone says
        // nothing about it.
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), tinted(A, ROOT, 1_000), draw(B, ROOT), draw(C, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[
                root_created(),
                create(C, ROOT, NO_NODE),
                create(B, ROOT, C),
                create(A, ROOT, B),
                painted(A, 1_000),
            ],
        );
    }

    #[test]
    fn a_reorder_is_one_entry() {
        // The edit this kind of reconciler gets wrong, and the reason the task
        // exists. A reconciler keyed on sibling *position* sees three changed
        // slots in `[a, b, c] -> [c, a, b]` and emits `RemoveNode(a)`,
        // `RemoveNode(b)`, `RemoveNode(c)`, three creations and every property
        // of all three — destroying three subtrees to express a rotation.
        // Keyed on identity, `a` and `b` are already in the right order
        // relative to each other, and only `c` has to be told where it now
        // goes.
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT), draw(B, ROOT), draw(C, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[root_created(), create(C, ROOT, NO_NODE), create(B, ROOT, C), create(A, ROOT, B)],
        );

        let reordered = [root(), draw(C, ROOT), draw(A, ROOT), draw(B, ROOT)];
        step(&mut reconciler, &mut model, &reordered, &[create(C, ROOT, A)]);
    }

    #[test]
    fn a_rotation_is_one_entry_and_not_three() {
        // The case that separates *minimal* from *small*. A greedy
        // left-to-right scan — walk the new order with a cursor into the old
        // one and move anything that does not match — emits three entries for
        // `[a, b, c, d] -> [b, c, d, a]`: `b` in front of `a`, `c` in front of
        // `a`, `d` in front of `a`. All three are correct and two of them are
        // waste. The longest increasing subsequence here is `b, c, d`, so `a`
        // is the only node that has to be told anything.
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT), draw(B, ROOT), draw(C, ROOT), draw(D, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[
                root_created(),
                create(D, ROOT, NO_NODE),
                create(C, ROOT, D),
                create(B, ROOT, C),
                create(A, ROOT, B),
            ],
        );

        let rotated = [root(), draw(B, ROOT), draw(C, ROOT), draw(D, ROOT), draw(A, ROOT)];
        step(&mut reconciler, &mut model, &rotated, &[create(A, ROOT, NO_NODE)]);
    }

    #[test]
    fn an_insertion_names_the_sibling_it_goes_in_front_of() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT), draw(B, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[root_created(), create(B, ROOT, NO_NODE), create(A, ROOT, B)],
        );

        // One node arrives in the middle. Its two siblings are untouched, which
        // is the thing `before` exists for: a format that could only append
        // would make this a rebuild of every sibling after the insertion point.
        let inserted = [root(), draw(A, ROOT), tinted(C, ROOT, 2_000), draw(B, ROOT)];
        step(&mut reconciler, &mut model, &inserted, &[create(C, ROOT, B), painted(C, 2_000)]);
    }

    #[test]
    fn a_property_that_changed_is_the_only_thing_said() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), tinted(A, ROOT, 1_000), draw(B, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[root_created(), create(B, ROOT, NO_NODE), create(A, ROOT, B), painted(A, 1_000)],
        );

        // The caret-moves case from section 07: one node's paint changes, and
        // the frame is one delta on a tree the client rebuilt in full.
        let repainted = [root(), tinted(A, ROOT, 5_000), draw(B, ROOT)];
        step(&mut reconciler, &mut model, &repainted, &[painted(A, 5_000)]);
    }

    #[test]
    fn a_vanished_subtree_is_one_removal() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT), draw(A1, A), draw(B, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[root_created(), create(B, ROOT, NO_NODE), create(A, ROOT, B), create(A1, A, NO_NODE)],
        );

        // `A1` vanishes too and is not mentioned. `RemoveNode` takes the
        // subtree, so an entry for the child would name a node that stopped
        // existing one entry earlier — which is the superset a per-node walk
        // produces, and it is not merely wasteful, it is wrong.
        let pruned = [root(), draw(B, ROOT)];
        step(&mut reconciler, &mut model, &pruned, &[Entry::RemoveNode(RemoveNode { node: A })]);
    }

    #[test]
    fn a_node_that_survives_its_parent_is_born_again() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT), tinted(A1, A, 3_000), draw(B, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[
                root_created(),
                create(B, ROOT, NO_NODE),
                create(A, ROOT, B),
                create(A1, A, NO_NODE),
                painted(A1, 3_000),
            ],
        );

        // `A` is removed and `A1` reappears under `B`. The removal takes `A1`
        // with it, so the entry for `A1` is a creation and its paint has to be
        // stated again — a reconciler that read *the key is in both trees* as
        // *the node survived* would emit a move for a node that no longer
        // exists and never send the paint at all.
        let moved = [root(), draw(B, ROOT), tinted(A1, B, 3_000)];
        step(
            &mut reconciler,
            &mut model,
            &moved,
            &[
                Entry::RemoveNode(RemoveNode { node: A }),
                create(A1, B, NO_NODE),
                painted(A1, 3_000),
            ],
        );
    }

    #[test]
    fn a_reparented_node_keeps_everything_but_its_parent() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT), tinted(A1, A, 4_000), draw(B, ROOT)];
        step(
            &mut reconciler,
            &mut model,
            &tree,
            &[
                root_created(),
                create(B, ROOT, NO_NODE),
                create(A, ROOT, B),
                create(A1, A, NO_NODE),
                painted(A1, 4_000),
            ],
        );

        // `A` stays, so `A1` is moved rather than recreated: one entry, and its
        // paint is not restated, because a re-hang keeps what the node already
        // holds. That is the half of the `CreateNode`-is-a-re-hang reading that
        // costs a delta if it is wrong in either direction.
        let reparented = [root(), draw(A, ROOT), draw(B, ROOT), tinted(A1, B, 4_000)];
        step(&mut reconciler, &mut model, &reparented, &[create(A1, B, NO_NODE)]);
    }

    #[test]
    fn an_unchanged_tree_is_no_deltas_at_all() {
        // The frame an idle application produces, which is most of them. It is
        // also what makes the counter in `E3-B01j` mean anything: a reconciler
        // that emitted something here would put a floor under the crossing
        // count that no amount of pipeline work could lift.
        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [root(), draw(A, ROOT)];
        step(&mut reconciler, &mut model, &tree, &[root_created(), create(A, ROOT, NO_NODE)]);
        step(&mut reconciler, &mut model, &tree, &[]);
    }

    #[test]
    fn a_tree_that_is_not_a_tree_is_refused_before_anything_is_emitted() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut out = Deltas::<LIMIT>::new();

        let no_key = [draw(NO_NODE, NO_NODE)];
        assert_eq!(reconciler.frame(&no_key, &mut out), Err(Refusal::NoKey));

        let twice = [draw(A, NO_NODE), draw(A, NO_NODE)];
        assert_eq!(reconciler.frame(&twice, &mut out), Err(Refusal::DuplicateKey));

        // The child written above its parent, the parent that does not exist
        // and the node that is its own parent are one refusal, because they are
        // one rule: a parent appears before its child, and a cycle therefore
        // has nowhere to be written.
        let inverted = [draw(A, B), draw(B, NO_NODE)];
        assert_eq!(reconciler.frame(&inverted, &mut out), Err(Refusal::OutOfOrder));
        let dangling = [draw(A, C)];
        assert_eq!(reconciler.frame(&dangling, &mut out), Err(Refusal::OutOfOrder));
        let itself = [draw(A, A)];
        assert_eq!(reconciler.frame(&itself, &mut out), Err(Refusal::OutOfOrder));

        let seventh = [Node::new(A, NO_NODE, 7)];
        assert_eq!(reconciler.frame(&seventh, &mut out), Err(Refusal::UnknownKind));

        let mut named = draw(A, NO_NODE);
        named.paint.node = A;
        assert_eq!(reconciler.frame(&[named], &mut out), Err(Refusal::PropertyNamed));

        // Nothing above reached the buffer, and nothing above moved the tree
        // the reconciler believes the compositor holds.
        assert!(out.as_slice().is_empty());
        assert!(reconciler.held().is_empty());
    }

    #[test]
    fn a_key_may_not_change_what_it_is() {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut out = Deltas::<LIMIT>::new();
        let tree = [draw(A, NO_NODE)];
        reconciler.frame(&tree, &mut out).expect("a legal tree");

        // The silent repair — remove it and create it again — is available and
        // is refused, because it costs the node's whole subtree on a frame
        // nobody is watching, and because the client can fix it with a
        // different key.
        let changed = [Node::new(A, NO_NODE, kind::EFFECT)];
        assert_eq!(reconciler.frame(&changed, &mut out), Err(Refusal::KindChanged));
        assert_eq!(reconciler.held(), tree.as_slice());
    }

    #[test]
    fn the_tree_may_not_outgrow_the_reconciler() {
        let mut small = Reconciler::<2>::new();
        let mut out = Deltas::<LIMIT>::new();
        let three = [draw(A, NO_NODE), draw(B, NO_NODE), draw(C, NO_NODE)];
        assert_eq!(small.frame(&three, &mut out), Err(Refusal::Capacity));
        assert!(small.held().is_empty());
    }

    #[test]
    fn a_frame_that_does_not_fit_is_not_half_submitted() {
        // A commit is every delta in it or none, so a buffer that fills is not
        // a smaller frame. The held tree must be untouched afterwards, or the
        // next frame would be computed against a scene the compositor never
        // received.
        let mut reconciler = Reconciler::<NODES>::new();
        let mut tiny = Deltas::<1>::new();
        let tree = [draw(A, NO_NODE), draw(B, NO_NODE)];
        assert_eq!(reconciler.frame(&tree, &mut tiny), Err(Refusal::Overflow));
        assert!(reconciler.held().is_empty());
        assert!(tiny.as_slice().is_empty());

        let mut enough = Deltas::<LIMIT>::new();
        reconciler.frame(&tree, &mut enough).expect("the same tree, with room for it");
        assert_eq!(enough.as_slice(), &[create(B, NO_NODE, NO_NODE), create(A, NO_NODE, B)]);
    }

    #[test]
    fn every_property_is_one_delta_when_it_changes_and_none_when_it_does_not() {
        // Not a loop over a list — a loop cannot see what a list omits, which
        // is the scar `interface/src/node.rs` carries. What closes that hole is
        // the destructuring in `emit_properties`, which has no `..` and so does
        // not compile if a field is added to `Node` outside the `properties!`
        // invocation. This observes the other end: that every property the list
        // does declare produces exactly one delta when it changes, in
        // declaration order, and that the count is the one the list counts.
        assert_eq!(Node::PROPERTY_COUNT, 3);

        let mut reconciler = Reconciler::<NODES>::new();
        let mut model = Model::new();
        let tree = [draw(A, NO_NODE)];
        step(&mut reconciler, &mut model, &tree, &[create(A, NO_NODE, NO_NODE)]);

        let mut all = draw(A, NO_NODE);
        all.transform.tx_x65536 = 3 * ONE_X65536;
        all.path.geometry_bytes = 96;
        all.paint.alpha_x65535 = u16::MAX;
        let mut transform = all.transform;
        transform.node = A;
        let mut path = all.path;
        path.node = A;
        let mut paint = all.paint;
        paint.node = A;
        step(
            &mut reconciler,
            &mut model,
            &[all],
            &[Entry::SetTransform(transform), Entry::SetPath(path), Entry::SetPaint(paint)],
        );
    }
}
