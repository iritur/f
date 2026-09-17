// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The six scene node kinds as types, and the one route by which a node of a
//! kind begins or ends.
//!
//! `abi/src/scene.rs` already has the kinds: six `u16` constants and a `known`
//! that refuses everything else. It has them as numbers because that is what a
//! value crossing a trust boundary must be — fixed width, and refusable when it
//! is not one of the six. This module is the other half of the same decision,
//! on the near side of the decoder, where a kind has been believed and there is
//! nothing left to refuse: here it is a type, the set of them is closed, and
//! the closedness is the compiler's to enforce rather than a reader's to
//! remember.
//!
//! Section 07 of `docs/design/ring-scene-boot.html` is where the six come from
//! and where the argument for *six* lives. Nothing here re-argues it; a seventh
//! is that section's to move, and what this module owes is that the move cannot
//! happen quietly.
//!
//! # Created and removed by a delta, and by no other route
//!
//! [`Created`] is a node that exists, and the kind it was given. It has no
//! public constructor, no `Default`, no `From`, no `Clone` and no `Copy`, and
//! its fields are private. The only function in this workspace that returns one
//! is [`Change::of`], which takes a [`Delta`] — so a node comes into being
//! because a delta said so, and there is no expression anywhere that produces a
//! [`Created`] without one in hand.
//!
//! Ending one is the same property run backwards. [`Created::removed_by`] and
//! [`Created::removed_under`] are the only methods that take `self` by value,
//! and both demand a [`Removal`], which is itself constructible only out of a
//! `RemoveNode` delta. A consumer holding a [`Created`] cannot take it apart,
//! cannot duplicate it, and cannot retire it by any route that does not start
//! at a delta.
//!
//! What Rust will not give is the last step: a value can always be dropped, and
//! a [`Created`] dropped on the floor is a node this module has forgotten. That
//! is a leak rather than a second route — nothing is created and nothing is
//! ended behind the delta's back — and it is the arena's to catch, where the
//! live slots are counted against the creations that filled them. Saying so is
//! better than a `Drop` impl that would pretend to.
//!
//! **A [`Kind`] variant is a name, not a node.** `Kind::Draw` is writable
//! anywhere, and has to be: the next section is that every consumer must be
//! able to *match* on a kind, which is impossible if the variants are not
//! public. Naming a kind creates nothing. What the two clauses together forbid
//! is a *node* that no delta introduced, and an integer that becomes a kind
//! without passing a decoder — which is why `Kind::from_wire` is private and
//! there is no `TryFrom<u16>` here. The only public door from a number to a
//! kind is [`Change::of`], and it is a delta-shaped door.
//!
//! # A seventh kind
//!
//! Three things break, and none of them is a test.
//!
//! - **This file stops compiling if `abi` gains a kind and this list does not.**
//!   The const block below walks every `u16` there is and requires
//!   `abi::scene::kind::known` and this module's `from_wire` to answer the same
//!   thing about all of them. It is a const assertion and not a test because
//!   the two lists disagreeing is not a behaviour to observe, it is a build
//!   that should not link. A seventh constant added over there is a compile
//!   error here on the same afternoon.
//! - **Every consumer that matches exhaustively stops compiling.** [`Kind`] is
//!   an ordinary enum and deliberately not `#[non_exhaustive]`, for the reason
//!   `interface/src/node.rs` gives about its `Role`: that attribute forces
//!   every consumer to carry a wildcard arm, and a wildcard arm is where a kind
//!   nobody understood goes to be rendered as nothing.
//! - **Every consumer holding a per-kind table *as a literal* stops
//!   compiling.** [`ByKind`] holds `[T; Kind::COUNT]`, so a table written out
//!   with six entries is the wrong length the day there are seven. This is the
//!   arm of the argument a match cannot carry: an exhaustive match demands an
//!   arm and not an arm that says anything, and `Kind::Volume => {}` compiles.
//!   A written-out table has no empty row. A consumer that wants per-kind
//!   behaviour should reach for [`ByKind`] first and a match second, and this
//!   module offers no third way. The one in the tree today is
//!   `crate::effect`'s `MAY_DECLARE`, which decides which kinds may carry a
//!   cost and a fallback, and which is the reason [`Effect`](Kind::Effect) is
//!   not read out of a `matches!`.
//!
//! # How far that goes, checked rather than asserted
//!
//! Stated exactly, because the summary of it is larger than any of it and this
//! module's whole worth is that a reader can trust the summary.
//!
//! A seventh kind, added to the `kinds!` list below and to `abi::scene::kind`,
//! stops **this file's** build once, at the family census const block, and
//! stops **`crate::effect`'s** build once, at `MAY_DECLARE`'s six rows. Two
//! errors, measured: `error[E0308]` at `scene/src/effect.rs:230` and
//! `error[E0080]` at the census below.
//!
//! **Not the wire cross-check**, which an earlier draft of this paragraph
//! counted and which a reviewer measured out of it. That const block asks
//! whether `abi::scene::kind::known` and `from_wire` agree about every `u16`,
//! and a kind added to *both* lists satisfies it. It fires on a **partial**
//! addition — one list edited and not the other — which is a different and
//! narrower event than a seventh kind, and the bullet above says so correctly.
//! The count is written here rather than left implicit because a reader
//! auditing this module will audit the number, and a number in a section
//! titled *checked* has to have been.
//!
//! It does not stop three other things, and none of the three is hypothetical:
//!
//! - **A consumer that writes `_ =>`.** Rust has no way to refuse a wildcard
//!   over somebody else's enum, here or in `interface/src/node.rs`.
//! - **A [`ByKind`] that is not written as a literal.** `ByKind::new([0;
//!   Kind::COUNT])` is a repeat expression and `Kind::ALL.map(..)` is a map
//!   over the list itself; both grow to seven silently. `crate::arena` holds
//!   one of the first and two of the second, so the guard this section argues
//!   for has exactly one instance in the workspace and it is the one named
//!   above. A per-kind table that must fire is written out row by row.
//! - **A consumer that carries a kind as a `u16`.** `crate::reconcile` and
//!   `crate::commit` both do, validated by `abi::scene::kind::known` rather
//!   than by this type, and a seventh kind reaches them with no edit at all.
//!   Whether the decoder belongs below those modules is their argument and not
//!   this one's; what is this one's is not to claim [`Kind`] is the only
//!   near-side representation of *what kind a node is*, because it is not.
//!
//! The exit this file is accepted on says *a seventh kind is a compile error in
//! every consumer, the way `Role` already is*. **That is more than is true of
//! `Role` and more than is true here**, and the sentence this file does stand
//! behind is the narrower one: a seventh kind is a compile error in every
//! consumer that decides per kind without a wildcard, and the one consumer in
//! this workspace that decides per kind is such a consumer.
//!
//! # Why a macro, in a tree that mostly refuses them
//!
//! Because the alternative is four sequences that have to agree — the variants,
//! `ALL`, the wire mapping and the family — and this project has twice watched
//! a variant be added to an enum and left out of an array that every loop
//! iterates. The repair both times was not a better test; it was the removal of
//! the second place to write the thing. One line below carries everything this
//! module answers about a kind, and `ALL`, `COUNT`, `index`, `name`, `wire`,
//! `family` and `from_wire` are all emitted from it.
//!
//! The cost is one indirection between a reader and the enum, which is why
//! nothing else in this crate is written this way. It is paid here because here
//! the second copy was the defect.
//!
//! # No clock, no randomness, no float, no allocator
//!
//! Everything here is a pure function of the delta it is handed. Nothing reads
//! a clock or draws a number, so this crate takes no `f_env::Env` and would
//! have nothing to do with one: a function whose answer depends only on its
//! argument is more deterministic than one that draws, and is reproducible from
//! the argument alone. No binary floating point, per RFC 0004 — the kinds are
//! identifiers and carry no quantity at all. No `Vec` and no allocation:
//! [`ByKind`] is an array whose length is a compile-time constant, which is
//! also what makes it the guard described above.

use core::num::NonZeroU32;
use core::ops::{Index, IndexMut};

use f_abi::scene::{Delta, Entry, Refusal, kind as wire};

/// What a renderer does with a node of a kind.
///
/// Three answers, and the census is load-bearing rather than descriptive: a
/// kind belonging to none of them would be a kind with no answer to *what does
/// a traversal do when it reaches this*, and cannot be written, because a
/// `kinds!` line without a family does not parse. The count per family is
/// asserted at compile time further down, so a seventh kind moves a number
/// somebody has to look at rather than slipping in behind one.
///
/// This is the device `interface/src/node.rs` uses for roles, one layer down
/// and with a different question: that one asks what a *projection* may do with
/// a node, this one asks what a *renderer* must do with it. Nothing maps
/// between the two, and nothing should — a scene kind is a rendering
/// instruction and a semantic role is a meaning, and the whole point of
/// [`Kind::Semantic`] is that the second travels in the first's tree without
/// becoming it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    /// Changes the context its subtree renders in and puts down no marks of its
    /// own. Four kinds.
    Context,
    /// Puts marks on the target. One kind.
    Mark,
    /// Says what the subtree is and contributes nothing to the picture. One
    /// kind.
    Meaning,
}

/// Names a wire constant where a pattern is expected.
///
/// `macro_rules!` will not take `wire::$constant` as a pattern directly — a
/// path followed by a metavariable is ambiguous there — and this is the
/// indirection that resolves it. `abi/src/scene.rs` needed the same one for the
/// same reason and calls it `opcode_pattern!`; it exists for no other purpose,
/// and is declared before `kinds!` because a macro must be defined before it is
/// used.
macro_rules! wire_pattern {
    ($constant:ident) => {
        wire::$constant
    };
}

/// The six kinds, written once so that they cannot be written twice.
///
/// The module documentation argues the shape. What the invocation below has to
/// supply for each kind is: the variant, the word a log prints, the constant in
/// `abi::scene::kind` it is the same thing as, and the family. A kind missing
/// any of the four does not parse, which is how each of those questions gets
/// asked at the moment a kind is proposed rather than at the moment something
/// downstream trips over the absence of an answer.
macro_rules! kinds {
    (
        $(
            $(#[$about:meta])*
            $variant:ident, $spelling:literal, $constant:ident, $family:ident,
        )*
    ) => {
        /// What a scene node *is*.
        ///
        /// Six variants: no `Other`, no `Custom`, no variant carrying a number,
        /// and no `#[non_exhaustive]`. Section 07 argues the six; the module
        /// documentation argues why the set being closed is worth what it
        /// costs, and what a seventh breaks.
        ///
        /// The variants, and everything this module answers about them, are
        /// emitted from the `kinds!` invocation that declares them, which is
        /// the only place a kind is written.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub enum Kind {
            $($(#[$about])* $variant,)*
        }

        impl Kind {
            /// How many kinds there are.
            ///
            /// Counted from the list rather than written as a literal, so it
            /// cannot disagree with what it counts. Every fixed-size table over
            /// kinds in this workspace is this wide — see [`ByKind`] — which is
            /// what makes a seventh kind reach a consumer's build rather than
            /// only this file's.
            /// Unit: kinds.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every kind, in declaration order, indexed by
            /// [`index`](Self::index).
            ///
            /// Emitted from the same list as the enum, so it holds every
            /// variant the enum has. Not because a test checks it — a loop over
            /// a list cannot see what the list omits — but because there is no
            /// way to write a variant this array does not get.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// This kind's position in [`ALL`](Self::ALL).
            ///
            /// The enum's own discriminant, which is the position of the line
            /// that declared the kind, which is the position of its entry in
            /// `ALL`: one list read three ways. There is no second sequence
            /// here to keep in step with the first.
            #[must_use]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The word a log or a trace prints.
            ///
            /// The same spelling `abi::scene::kind::label` uses, and
            /// `the_name_is_the_one_the_wire_crate_logs` is what keeps the two
            /// from drifting: a kind whose name differs on the two sides of the
            /// decoder makes one event look like two in a boot log, which is
            /// the only way this string can be wrong.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }

            /// The number this kind is on the wire.
            ///
            /// `abi::scene::kind`'s constant, named rather than copied: the
            /// value lives there because that is the crate whose layout is
            /// load-bearing against peers built by another toolchain, and a
            /// second numeral here would be a second thing that can disagree
            /// with it.
            /// Unit: none — a node kind, not a quantity.
            #[must_use]
            pub const fn wire(self) -> u16 {
                match self {
                    $(Self::$variant => wire::$constant,)*
                }
            }

            /// What a renderer does with a node of this kind.
            ///
            /// The fourth field of the kind's line. A kind belonging to no
            /// family cannot be written, because the line does not parse
            /// without one — the small version of the whole argument here: the
            /// question *what does a traversal do when it reaches this* is
            /// asked where the kind is declared rather than left for a renderer
            /// to answer by guessing.
            #[must_use]
            pub const fn family(self) -> Family {
                match self {
                    $(Self::$variant => Family::$family,)*
                }
            }

            /// The kind a wire value names, if this build has one for it.
            ///
            /// **Private, and that is the point.** A `u16` becomes a [`Kind`]
            /// only inside [`Change::of`], which has a whole delta around it; a
            /// public `TryFrom<u16>` would be a route from an integer a caller
            /// found somewhere to a node kind, and *found somewhere* is exactly
            /// the provenance this module exists to refuse. The negative answer
            /// is a refusal at the one door rather than an `Option` handed to
            /// every caller to mishandle.
            const fn from_wire(raw: u16) -> Option<Self> {
                match raw {
                    $(wire_pattern!($constant) => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

kinds! {
    /// An affine matrix applied to a subtree.
    ///
    /// It replaces the node's transform rather than composing with what was
    /// there — composition down the tree is the traversal's, and
    /// `abi::scene::SetTransform` says why a delta that composed would make the
    /// scene a function of how many times it had been sent.
    Transform, "transform", TRANSFORM, Context,

    /// A path restricting a subtree.
    ///
    /// A restriction and not a shape: nothing under it can paint outside the
    /// path, which is a statement about the descendants and is why this is
    /// [`Family::Context`] rather than a mark that happens to be invisible.
    Clip, "clip", CLIP, Context,

    /// Opacity and blend mode over a subtree.
    ///
    /// The kind that may cost an intermediate target, and *may* is the whole of
    /// it: whether one is needed is the renderer's arithmetic over what is
    /// actually underneath, not a property of the node. A kind that always
    /// allocated a target would be a kind an author learns to avoid, which is
    /// how grouping and compositing end up spelled two different ways.
    Layer, "layer", LAYER, Context,

    /// A filled or stroked path, a glyph run, or an image.
    ///
    /// The only kind that puts anything on the target. Everything else here
    /// either changes how this one lands or says nothing about pixels at all,
    /// which is what makes the [`Family::Mark`] census of one a fact about the
    /// design rather than an accident of the list.
    Draw, "draw", DRAW, Mark,

    /// Blur, drop shadow or a material over a subtree.
    ///
    /// Parameterised rather than open-ended — section 07's decision, and the
    /// one that keeps the shader set finite. [`Family::Context`] for the same
    /// reason [`Kind::Layer`] is: what it does, it does to what is underneath.
    Effect, "effect", EFFECT, Context,

    /// Role, label, value, relations — in the same tree, submitted in the same
    /// commit.
    ///
    /// The kind that draws nothing, which is why it is the one that had to
    /// exist. Section 07 calls accessibility arriving with the first commit the
    /// cheapest correct decision in the document: a semantic node in the scene
    /// tree is submitted by the same code that submitted the drawing, in the
    /// same frame, under the same commit — and a retrofit is a second tree
    /// nobody keeps in step. The vocabulary those roles come from is
    /// `interface/src/node.rs` and is not this crate's business; what is this
    /// crate's business is that carrying them costs a node kind and not a
    /// protocol.
    Semantic, "semantic", SEMANTIC, Meaning,
}

/// One `T` per kind, and no way to write one that is missing a kind.
///
/// The type a consumer should reach for before it reaches for a match. A
/// dispatch table, a per-kind counter, a per-kind renderer entry point: each of
/// those **written out as a six-element array literal** is a compile error the
/// day there are seven kinds — in the consumer's own crate, with the consumer's
/// own author reading the message. That is the part a match cannot do: an
/// exhaustive match demands an arm, and an arm that says nothing is still an
/// arm.
///
/// **Two ways of building one that a seventh kind does not stop**, named here
/// because both are in this workspace and both look like this type doing its
/// job: `ByKind::new([0; Kind::COUNT])` is a repeat expression and grows to
/// seven zeros, and `ByKind::new(Kind::ALL.map(..))` is a map over the list
/// that changed, so it grows too. Neither is wrong — a census that starts at
/// zero for every kind wants exactly the first — but neither is a guard, and a
/// table that has to *decide* something per kind is written row by row for that
/// reason. `crate::effect`'s `MAY_DECLARE` is the one in the tree that decides.
///
/// Indexing cannot fail and there is no `Option` anywhere on it:
/// [`Kind::index`] is the enum's discriminant and [`Kind::COUNT`] is the array's
/// length, both emitted from the one list, so the bound is a fact about the
/// types rather than a check at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByKind<T>([T; Kind::COUNT]);

impl<T> ByKind<T> {
    /// A table with a value for every kind, in [`Kind::ALL`]'s order.
    ///
    /// The only constructor, and it takes the whole array: there is no `insert`
    /// and no `Default`, because a table that could be built empty and filled
    /// in afterwards is a table that can be half filled, and the half that is
    /// missing is discovered by whatever reads it.
    pub const fn new(per_kind: [T; Kind::COUNT]) -> Self {
        Self(per_kind)
    }

    /// This kind's entry, in a `const` context.
    ///
    /// [`Index`] is the same lookup for everything else; this exists because
    /// `Index::index` is not `const`, and a per-kind table that has to be built
    /// at run time is a table that cannot be a `static`.
    #[must_use]
    pub const fn at(&self, kind: Kind) -> &T {
        &self.0[kind.index()]
    }

    /// The entries, in [`Kind::ALL`]'s order.
    ///
    /// Borrowed rather than moved, and there is no counterpart that builds one
    /// from parts beyond [`new`](Self::new): a caller that wants to iterate
    /// should zip this with [`Kind::ALL`], which is the pairing that cannot go
    /// out of step, rather than index it with a number it computed.
    #[must_use]
    pub const fn as_array(&self) -> &[T; Kind::COUNT] {
        &self.0
    }
}

impl<T> Index<Kind> for ByKind<T> {
    type Output = T;

    fn index(&self, kind: Kind) -> &T {
        &self.0[kind.index()]
    }
}

impl<T> IndexMut<Kind> for ByKind<T> {
    fn index_mut(&mut self, kind: Kind) -> &mut T {
        &mut self.0[kind.index()]
    }
}

/// A node that exists, and the kind a delta gave it.
///
/// The token this module's half of the exit rests on. Its fields are private,
/// it has no constructor a caller can reach, no `Default`, no `From` and — this
/// is the load-bearing absence — no `Clone` and no `Copy`, so one creation is
/// one of these and a consumer cannot make a second by assignment. The only
/// function that returns one is [`Change::of`]; the only methods that consume
/// one are [`removed_by`](Self::removed_by) and
/// [`removed_under`](Self::removed_under), and both demand a [`Removal`].
///
/// It is `#[must_use]` because a [`Created`] evaluated and dropped in statement
/// position is a node the arena was told about and did not store, and the
/// workspace denies `unused_must_use`.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub struct Created {
    /// The identifier the submitter chose, which is never `NO_NODE` — the type
    /// says so rather than a check somewhere saying so, which is what stops an
    /// all-zero payload from naming a node at all.
    node: NonZeroU32,
    /// What the delta said this node is.
    kind: Kind,
}

impl Created {
    /// The identifier this node answers to.
    /// Unit: none — a node identifier, not a quantity. Never zero.
    #[must_use]
    pub const fn node(&self) -> u32 {
        self.node.get()
    }

    /// What kind of node it is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// End this node, on a removal that names it.
    ///
    /// Consumes the token and hands back the kind that left, which is what a
    /// caller keeping a census per kind needs and is the only thing it gets: a
    /// node that has been removed leaves no value behind that could be mistaken
    /// for one that still exists.
    ///
    /// # Errors
    ///
    /// The node itself, unharmed, when the removal names some other root. The
    /// error is the token rather than a code because there is nothing to
    /// recover from — the caller asked the wrong question about the right node
    /// — and anything narrower would have had to throw the node away to report
    /// it.
    pub fn removed_by(self, removal: &Removal) -> Result<Kind, Self> {
        if removal.root == self.node { Ok(self.kind) } else { Err(self) }
    }

    /// End this node as a descendant of a removed subtree.
    ///
    /// `abi::scene::RemoveNode` removes the node *and everything under it*, so
    /// a consumer tearing down a subtree has descendants to retire that the
    /// delta never names. This is that route, and it is not the hole it looks
    /// like: it still costs a [`Removal`], and a [`Removal`] exists only
    /// because a `RemoveNode` delta was decoded. What the caller supplies is
    /// *ancestry* — that this node is under `removal`'s root — which is a fact
    /// about the shape of the tree and not about the delta, and no type in this
    /// module holds the tree to check it against. The arena does, and that is
    /// where the check belongs.
    ///
    /// *What would reverse this:* a node token that carried its parent, at
    /// which point ancestry is checkable here and this method should demand the
    /// chain rather than trust it. That is a wider token in every arena slot,
    /// which is a cost paid per node for a check made per removal.
    pub const fn removed_under(self, _removal: &Removal) -> Kind {
        self.kind
    }
}

/// A subtree a delta said to remove.
///
/// The root only. Which nodes are under it is the arena's knowledge and
/// deliberately not reconstructible from here, so this type cannot be mistaken
/// for a list of what actually went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Removal {
    /// The root of the subtree that goes, never `NO_NODE` — which is what stops
    /// a zeroed payload from reading as *remove everything*.
    root: NonZeroU32,
}

impl Removal {
    /// The root of the subtree that goes.
    /// Unit: none — a node identifier, not a quantity. Never zero.
    #[must_use]
    pub const fn root(&self) -> u32 {
        self.root.get()
    }
}

/// What a delta does to the population of nodes.
///
/// Three answers and not two, because *a delta that creates nothing* and *a
/// delta this build will not believe* are different events for a caller: the
/// first is four of the six opcodes doing their ordinary work, and the second
/// is a peer to stop reading. The second is a [`Refusal`] from [`Change::of`]
/// and never a variant here.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    /// A node began. The token is the only evidence of it there is.
    Created(Created),
    /// A subtree ended, rooted at the node this names.
    Removed(Removal),
    /// The delta edited a node or closed a frame. Nothing began and nothing
    /// ended.
    Untouched,
}

impl Change {
    /// What this delta does to the population.
    ///
    /// The one door. Every [`Created`] and every [`Removal`] in the workspace
    /// comes out of this function, and it takes a whole [`Delta`] — so the
    /// answer to *where did this node come from* is always *that delta*, and
    /// never *some code that knew a node identifier*.
    ///
    /// The arms are written out per opcode with no wildcard, so a seventh
    /// opcode in `abi/src/scene.rs` stops this build and asks whether it
    /// creates or ends a node. A wildcard would answer [`Change::Untouched`] on
    /// its behalf, which is the safe answer and exactly the kind of safe answer
    /// nobody ever revisits; `abi::scene::Delta::frame` declines the same
    /// wildcard for the same reason.
    ///
    /// # Errors
    ///
    /// A [`Refusal`], and every one of them is a refusal
    /// `abi::scene::Delta::decode` would also have made. `Delta::check` is
    /// called rather than restated — a delta a peer would refuse must not
    /// create a node here either, and two statements of one rule is how the two
    /// halves of a system come to disagree. Past it: [`Refusal::Value`] for a
    /// kind this build does not know, which is the refusal
    /// `abi::scene::CreateNode` makes of the same value, and
    /// [`Refusal::NoNode`] for a field that must name a node and names none.
    pub fn of(delta: &Delta) -> Result<Self, Refusal> {
        delta.check()?;
        match delta.body {
            Entry::CreateNode(create) => {
                let Some(node) = NonZeroU32::new(create.node) else {
                    return Err(Refusal::NoNode);
                };
                // The one place a number becomes a kind. A value outside the six
                // is refused here and never carried onward as a number for
                // somebody downstream to decide about: deciding later is what an
                // open vocabulary is, and the decision is made here, once, as a
                // no.
                let Some(kind) = Kind::from_wire(create.kind) else {
                    return Err(Refusal::Value);
                };
                Ok(Self::Created(Created { node, kind }))
            }
            Entry::RemoveNode(remove) => {
                let Some(root) = NonZeroU32::new(remove.node) else {
                    return Err(Refusal::NoNode);
                };
                Ok(Self::Removed(Removal { root }))
            }
            Entry::SetTransform(_) | Entry::SetPath(_) | Entry::SetPaint(_) | Entry::Commit(_) => {
                Ok(Self::Untouched)
            }
        }
    }
}

// The two lists are one list, checked where a disagreement would be a build and
// not a behaviour. A `u16` is small enough to walk in full at compile time, so
// this is not a sample and not a corpus: for every value there is, the wire
// crate and this module agree on whether it names a kind. A seventh constant in
// `abi::scene::kind` that nobody added here fails *this*, and a seventh line
// here that `abi` does not know fails it in the other direction.
//
// There is no test for this, and there should not be: a test would observe the
// same fact later, on a build that had already linked, and the honest place for
// a statement neither side may violate is the one that refuses to produce an
// artefact at all.
const _: () = {
    let mut raw: u16 = 0;
    loop {
        assert!(
            wire::known(raw) == Kind::from_wire(raw).is_some(),
            "abi::scene::kind and f_scene::kind disagree about a wire value"
        );
        if raw == u16::MAX {
            break;
        }
        raw += 1;
    }
};

// The family census, asserted rather than described. Four kinds that change the
// context their subtree renders in, one that puts marks down, one that says what
// the subtree means — and the shape of that census is the design: the renderer
// has one mark path and four ways to modify it, and accessibility rides in the
// same tree at the cost of a kind. A seventh kind moves one of these three
// numbers and has to say which, here, in a diff somebody reads.
const _: () = {
    let mut context = 0usize;
    let mut mark = 0usize;
    let mut meaning = 0usize;
    let mut i = 0;
    while i < Kind::COUNT {
        match Kind::ALL[i].family() {
            Family::Context => context += 1,
            Family::Mark => mark += 1,
            Family::Meaning => meaning += 1,
        }
        i += 1;
    }
    assert!(context == 4, "the context family is not the four kinds this module argues for");
    assert!(mark == 1, "marks are supposed to have exactly one kind");
    assert!(meaning == 1, "meaning is supposed to have exactly one kind");
};

#[cfg(test)]
mod tests {
    use super::*;
    use f_abi::NO_DEADLINE;
    use f_abi::flags;
    use f_abi::scene::{CreateNode, NO_NODE, RemoveNode, op};

    /// A deadline far enough from zero to be unmistakably a deadline.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// A delta around a body, with an envelope that is legal for it.
    ///
    /// The deadline comes from `op::carries_deadline` rather than from a branch
    /// per opcode here, so a seventh opcode gets a legal envelope from the list
    /// that declared it and every test below covers it without being edited.
    /// `abi/src/scene.rs`'s own tests build their deltas the same way.
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

    /// A create delta naming `node`, carrying a wire kind that may or may not
    /// be one this build knows.
    fn create(node: u32, raw_kind: u16) -> Delta {
        delta(Entry::CreateNode(CreateNode {
            node,
            parent: NO_NODE,
            before: NO_NODE,
            kind: raw_kind,
        }))
    }

    /// A remove delta rooted at `node`.
    fn remove(node: u32) -> Delta {
        delta(Entry::RemoveNode(RemoveNode { node }))
    }

    #[test]
    fn each_kind_begins_at_a_delta_and_ends_at_one_that_names_it() {
        // The exit's first clause, for every kind there is — and the corpus is
        // `Kind::ALL`, emitted from the list that declares them, so a seventh
        // kind is exercised here the day it is written. What a test cannot
        // observe is the *absence* of a second route: that `Created` has no
        // public constructor, no `Default` and no `Clone` is a property of the
        // type, checked by the compiler in every crate that consumes it, and a
        // test that tried to assert it would only be asserting that this file
        // still says what it says.
        for kind in Kind::ALL {
            let Change::Created(created) = Change::of(&create(7, kind.wire())).unwrap() else {
                panic!("{}: a create delta did not create", kind.name());
            };
            assert_eq!(created.kind(), kind);
            assert_eq!(created.node(), 7);

            // A removal of some other node hands the node back untouched. This
            // is the whole of *and by no other route* on the removal side: the
            // only methods that consume a `Created` take a `&Removal`, and the
            // only source of a `Removal` is the delta below.
            let Change::Removed(elsewhere) = Change::of(&remove(9)).unwrap() else {
                panic!("{}: a remove delta did not remove", kind.name());
            };
            let created = created.removed_by(&elsewhere).unwrap_err();

            let Change::Removed(its_own) = Change::of(&remove(7)).unwrap() else {
                panic!("{}: a remove delta did not remove", kind.name());
            };
            assert_eq!(created.removed_by(&its_own), Ok(kind));
        }
    }

    #[test]
    fn a_kind_this_build_does_not_know_creates_nothing() {
        // Found rather than written down, so that a seventh kind cannot turn
        // this test into one that passes for the wrong reason.
        let unknown = (1..=u16::MAX).find(|raw| !wire::known(*raw)).expect("a u16 is not six");
        assert_eq!(Change::of(&create(7, unknown)), Err(Refusal::Value));
        // Zero is not a kind, which is what stops a zeroed payload naming the
        // first one.
        assert_eq!(Change::of(&create(7, 0)), Err(Refusal::Value));
    }

    #[test]
    fn a_delta_the_wire_would_refuse_changes_no_population() {
        // `Change::of` calls `Delta::check`, so the envelope rules reach this
        // door without being restated at it. Each of these is refused by
        // `abi::scene::Delta::decode` too, and with the same refusal.
        let mut deadline_on_a_create = create(7, Kind::Draw.wire());
        deadline_on_a_create.deadline = SCHEDULED_AT;
        assert_eq!(Change::of(&deadline_on_a_create), Err(Refusal::Reserved));

        let mut linked = create(7, Kind::Draw.wire());
        linked.flags = flags::LINK;
        assert_eq!(Change::of(&linked), Err(Refusal::UnknownFlag));

        // A node named by no identifier, on both opcodes that name one.
        assert_eq!(Change::of(&create(NO_NODE, Kind::Draw.wire())), Err(Refusal::NoNode));
        assert_eq!(Change::of(&remove(NO_NODE)), Err(Refusal::NoNode));
    }

    #[test]
    fn exactly_one_opcode_creates_and_exactly_one_removes() {
        // Over `Entry::SPECIMENS`, which `abi` emits from its own opcode list —
        // so a seventh opcode arrives in this loop without anybody adding it,
        // and lands in whichever count its author decided it belongs to.
        let (mut created, mut removed, mut untouched) = (0usize, 0usize, 0usize);
        for body in Entry::SPECIMENS {
            match Change::of(&delta(body)).unwrap() {
                Change::Created(_) => created += 1,
                Change::Removed(_) => removed += 1,
                Change::Untouched => untouched += 1,
            }
        }
        assert_eq!((created, removed), (1, 1));
        assert_eq!(untouched, op::COUNT - 2);
    }

    #[test]
    fn the_name_is_the_one_the_wire_crate_logs() {
        for kind in Kind::ALL {
            assert_eq!(kind.name(), wire::label(kind.wire()));
        }
    }

    #[test]
    fn a_table_over_kinds_has_an_entry_for_every_kind() {
        // Built from `Kind::ALL` rather than from a hand-written literal,
        // because a hand-written literal here would only prove that this file
        // can count. Where the literal matters is in a consumer, and there the
        // compiler does the proving: six elements is the wrong length the day
        // `Kind::COUNT` is seven, with no test involved.
        let mut table = ByKind::new(Kind::ALL.map(Kind::name));
        for kind in Kind::ALL {
            assert_eq!(table[kind], kind.name());
            assert_eq!(table.at(kind), &kind.name());
        }
        table[Kind::Draw] = "repainted";
        assert_eq!(table[Kind::Draw], "repainted");
        assert_eq!(table.as_array().len(), Kind::COUNT);
    }

    #[test]
    fn a_subtree_removal_ends_a_descendant_the_delta_never_named() {
        // The route that exists because `RemoveNode` takes a subtree. It still
        // costs a `Removal`, which still costs a delta; what the caller brings
        // is the ancestry, which is the arena's to know.
        let Change::Created(descendant) = Change::of(&create(11, Kind::Semantic.wire())).unwrap()
        else {
            panic!("a create delta did not create");
        };
        let Change::Removed(subtree) = Change::of(&remove(4)).unwrap() else {
            panic!("a remove delta did not remove");
        };
        assert_eq!(descendant.removed_under(&subtree), Kind::Semantic);
    }
}
