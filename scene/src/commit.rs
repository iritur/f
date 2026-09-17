// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The commit: every delta a frame carries reaches the graph, or none of them
//! does, and there is no third scene in between for anything to draw.
//!
//! `abi/src/scene.rs` puts one opcode at the end of a frame — `COMMIT`, the
//! only one that reads a deadline — and says what it means: *apply every delta
//! since the last commit, or none*. [`crate::arena`] deliberately does not
//! implement that sentence. Its `Applied::Closes` arm hands the frame token
//! back untouched and says why: *the effect is `commit`'s rather than this
//! module's*. This is that module.
//!
//! # The shape of the problem, stated before the design
//!
//! A frame arrives as entries in a ring. A consumer walks them in index order
//! and stops at the tail the producer published. So the ways a frame can arrive
//! incompletely are not exotic: the client died mid-frame, the deadline passed,
//! the channel was revoked, the ring was drained one entry short. Every one of
//! them is a **cut** at an entry boundary, and the question this file is
//! accepted on is what a reader of the graph sees after one.
//!
//! There are exactly two acceptable answers — the scene as it was before the
//! frame, and the scene the frame describes — and one unacceptable one, which
//! is a scene holding some of the frame's deltas and not the rest. That third
//! scene is not a degraded picture, it is a picture **no client ever asked
//! for**: a node created but never painted, a subtree removed and its
//! replacement never hung, a transform replaced on one node of a pair that only
//! means anything together. A compositor that can draw one has a failure mode
//! whose blast radius is *whatever the client happened to be halfway through*,
//! which is the worst available answer to *what does it look like when it goes
//! wrong* — nobody can say.
//!
//! # The one decision: nothing reaches the graph before the commit does
//!
//! Everything below follows from it. A [`Batch`] holds deltas and **has no
//! `Arena` anywhere in its type**: not a field, not a parameter of
//! [`Batch::offer`], not a borrow it could be handed. There is therefore no
//! expression in this workspace that puts a delta into the graph as it arrives,
//! which is what makes *a cut before the commit leaves the old scene* a
//! property of the types rather than of anybody's discipline.
//!
//! The alternative is the obvious one and is the defect: drain the ring, call
//! `Arena::apply` per entry, treat the commit as the moment to draw. It passes
//! every test that submits a whole frame. It produces a third scene at every
//! cut but two. The sweep at the bottom of this file runs exactly that — see
//! `tests::Applier::Eager` — and **requires it to produce a third scene**,
//! because a sweep that can only ever observe two states has proved nothing
//! about the third.
//!
//! # The second decision: [`Sealed`] is the only key
//!
//! A batch that has accumulated deltas cannot apply itself. [`Batch::commit`]
//! demands a [`Sealed`], which has no public constructor, is neither `Clone`
//! nor `Copy`, and is produced in exactly one place: [`Batch::offer`], when the
//! bytes it was handed decode to a `COMMIT`. Three things follow without a test
//! having to observe them.
//!
//! - **A frame with no commit cannot be applied.** There is no key.
//! - **A frame cannot be applied twice.** The key is moved into
//!   [`Batch::commit`] and the batch's serial moves with it, so a second
//!   attempt has nothing to present. Re-applying a frame is not harmless: a
//!   `RemoveNode` is not idempotent, and a re-hang of a node whose subtree has
//!   changed under it is a different scene.
//! - **A batch that refused anything can never produce a key.** This is the
//!   load-bearing one, and it is why [`Batch::offer`] takes *bytes* rather than
//!   a decoded [`Delta`]. A slot the producer has never reached reads as the
//!   zeros of a fresh mapping; `abi::scene` refuses those; this module records
//!   the refusal; and the `COMMIT` that arrives afterwards is refused with
//!   [`Refusal::Poisoned`] instead of sealing. **A frame that lost an entry is
//!   not a shorter frame. It is not a frame.**
//!
//! # The limit of that last one, which is the ring's and not this file's
//!
//! *A slot that did not land reads as zeros* is true of a slot nobody has
//! written yet and false of every slot after that. The ring initialises its
//! entries once and zeroes none of them on completion — `ring/src/lib.rs` — so
//! a slot whose write is not **visible** holds the entry that occupied it last,
//! which is a previous frame's delta and decodes perfectly. The batch has no
//! way to tell it from one of its own: there is no sequence number on an entry,
//! and every field of [`Sqe`] a scene delta does not read is already required
//! to be zero.
//!
//! What stands between that and a third scene is the ring's `Release` store and
//! the consumer's `Acquire` load, which together promise that every slot below
//! the published tail holds what the producer wrote. **A commit's atomicity
//! therefore does rest on that pair**, and the sweep at the bottom of this file
//! measures it rather than assuming it away: `tests::Mode::Lying` is a ring that
//! promises nothing, it is swept at every cut alongside the honest one, and the
//! run *requires* it to produce third scenes and to produce frames that only
//! [`admit`] stopped. No count of them is written down here, because a count
//! written into prose is a number nothing re-derives; the assertions are floors
//! and the failure message prints what the run actually saw.
//!
//! *What would reverse this:* a per-entry submission sequence in the ABI, which
//! the consumer checks against the slot's own index, at which point a stale slot
//! is detectable and the sentence above loses its clause. That is a diff to
//! `abi/src/scene.rs` with an RFC behind it and not a diff to this file.
//!
//! # Why the bytes are the door, and the only door
//!
//! [`Batch::offer`] takes an [`Sqe`] and a payload, not a `Delta`. A second
//! door taking an already-decoded delta would be a second place the decision
//! *is this entry part of this frame* is made, and the two would part company
//! the first time one of them learned something the other did not — which is
//! `docs/postmortem/0001`'s finding, and the reason `abi::scene` has one
//! `entries!` list. The cost is that an in-process caller re-encodes; the
//! encoder is total and the arithmetic is 56 bytes of memcpy, against a frame
//! whose cheapest alternative is a redraw.
//!
//! # Or none of them: the other half, and how it is bought
//!
//! Truncation is not the only way a frame can fail. The graph can refuse an
//! entry — a property set on a node that does not exist, a placement in front
//! of something that is not a sibling, a scene that has run out of slots. Apply
//! six deltas, have the seventh refused, and the graph holds six: the same
//! third scene by another route.
//!
//! Undo is not available, and the reason is worth stating because it is the
//! first thing a reader will reach for. Two of the graph's operations have no
//! inverse. `Arena::remove` destroys a subtree and nothing in this crate can
//! hold one; `Arena::set_paint` on a node that had no paint cannot be put back,
//! because the graph has no *clear*. A journal would therefore cover four of
//! the six opcodes, which is a journal that does not work.
//!
//! What is done instead is **admission**: [`admit`] walks the batch against the
//! graph, touching nothing, and answers whether the graph will accept every
//! entry. It can do that cheaply because of the third decision below, and
//! because it delegates every hard question back to the graph — presence to
//! `Arena::holds`, ancestry to `Arena::parent_of`, kind to `Arena::kind_of`,
//! room to `Arena::remaining`.
//!
//! # The third decision: a frame's entries arrive in sections
//!
//! [`Batch::offer`] requires a frame to be *removals, then creations, then
//! property sets*. Out of order is [`Refusal::OutOfPhase`] and the batch is
//! poisoned.
//!
//! This is not arbitrary and it is not new: it is exactly what
//! [`crate::reconcile`] emits, in the order its own three phases are written,
//! and `reconcile` is the only producer of frames in this workspace. Requiring
//! it here turns a habit into a rule, and the rule is what makes admission
//! exact without a second copy of the tree:
//!
//! - **every removal precedes every re-hang**, so *which nodes this frame
//!   destroys* is a question about the graph as it stands, answerable with
//!   `Arena::parent_of` and nothing else. Interleave them and the answer
//!   depends on where each node had been moved to by then — which needs a
//!   shadow tree, a second statement of what `Arena` already is, and the exact
//!   shape of mistake `docs/postmortem/0001` is about.
//! - **every creation precedes every property set**, so a set names a node that
//!   is either already in the graph or was created earlier in the same frame,
//!   and `Arena::holds` plus a list of at most `LIMIT` identifiers decides it.
//!
//! *The price is real and is not hidden:* a client that needs to remove a
//! subtree, and then remove another one it first had to build, is refused and
//! must send two frames. *What would reverse this:* a measured client whose
//! scene cannot be expressed in three sections, at which point admission needs
//! the removal cone recomputed per entry and this file grows the walk to do it.
//!
//! # What is left open, and where it is closed
//!
//! [`Refused::Diverged`] is the one door to a third scene this file does not
//! shut. It is what a commit answers when admission said yes and the graph then
//! said no — this build's two halves disagreeing, which is not something a peer
//! can cause, and is the same concession `arena::Refusal::Mismatched` already
//! makes for the same reason. The graph may hold part of the frame when it
//! happens; the sweep counts it and requires zero across every seed.
//!
//! Closing it properly is a change to `crate::arena` and not to this file: a
//! graph that could answer *would you accept this* from the same code that
//! performs it, or take a checkpoint a refusal restores, would delete [`admit`]
//! entirely and with it the only place in this module where a rule about
//! placement is written a second time. That is said here rather than left
//! implicit, because a reader who finds this paragraph should know the repair
//! is a deletion and not an addition.
//!
//! # No clock, no randomness, no float, no allocator
//!
//! Every function here is a pure function of the batch it holds and the graph
//! it is given. This crate takes no `f_env::Env` and would have nothing to ask
//! one, which is [`crate::arena`]'s argument and the same one: a function whose
//! answer depends only on its arguments is reproducible from its arguments,
//! which is a stronger determinism statement than a seeded one. The sweep below
//! takes its seed as a parameter for exactly that reason. No `HashMap` — there
//! is no map, only a fixed ledger walked in order — and no binary floating
//! point: nothing here looks inside a property record, it carries
//! `f_abi::scene`'s own fixed-point values through unexamined. RFC 0004.

// The four lints `crate::arena` denies for the sentence it is accepted on, and
// this module is accepted on the same one: a compositor that aborts is a screen
// that goes dark, so every failure here has a name instead of a panic.
// `indexing_slicing` is not among them, and that is the choice worth arguing —
// every index below is bounded by a `len` only this file writes, and an allow
// attached to a dozen of them is an allow nobody reads. `arena.rs` makes the
// same trade for `arithmetic_side_effects` and argues the arithmetic
// individually; the bounds here are argued once, at `Ledger`, which is the only
// place one could be wrong.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable)
)]

use f_abi::scene::{
    Delta, Entry, Frame, NO_NODE, PAYLOAD_BYTES, Refusal as WireRefusal, RemoveNode,
};
use f_abi::{NO_DEADLINE, Sqe};

use crate::arena::{Applied, Arena, Hung, NODES_MAX, Refusal as GraphRefusal};

/// The largest frame this module is sized for by default.
///
/// Sixty-four, and the number is section 07's rather than this file's: the
/// arithmetic there is drawn for five to fifty deltas a frame, and this is that
/// with headroom to the next power of two. It is a default and not a law —
/// [`Batch`] is generic over its bound so that a component states its own — and
/// the only thing it must be is *a number*, because a batch that grew would put
/// an allocator on the path a frame is drawn on, which is the one place in this
/// system nobody measures. `crate::reconcile::Deltas` carries the same bound
/// from the producing side of the wire.
///
/// *What would reverse this:* a measured client whose ordinary frame exceeds
/// it. The number moves; nothing else here does.
/// Unit: deltas.
pub const DELTAS_MAX: usize = 64;

/// Why a frame did not happen.
///
/// Local to this module rather than a widening of `arena::Refusal`, on that
/// enum's own terms: those are about an edit and a graph that already exists,
/// and these are about a *frame* — an aggregate the graph has no opinion about.
/// The two that are not this module's own are carried through rather than
/// restated, so the wire's refusals and the graph's refusals keep their names
/// and their node identifiers all the way out to the completion a client reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The batch already holds every delta it was sized for.
    ///
    /// The frame is refused whole rather than truncated, which is the only
    /// answer consistent with the rest of this module: a truncated frame is a
    /// third scene the client would have no way of hearing about.
    Full,
    /// The bytes in the slot are not a delta this build accepts.
    ///
    /// Carried from `f_abi::scene` unchanged. The common case is not a hostile
    /// peer: it is a slot whose payload had not landed when the entry became
    /// visible. Where the producer has never reached that slot, it reads as the
    /// zeros of a fresh mapping and is refused as an unknown opcode, and that
    /// refusal is what stops a frame with a hole in it from ever sealing. Where
    /// it has, the slot holds a previous frame's entry instead and this refusal
    /// is not made — the module's *the limit of that last one* is what carries
    /// the frame then, and it is the ring's ordering pair rather than anything
    /// here.
    Wire(WireRefusal),
    /// An edit arrived after an edit belonging to a later section.
    ///
    /// *Removals, then creations, then property sets*: the module's *a frame's
    /// entries arrive in sections* is the argument, and `crate::reconcile`'s
    /// three phases are where the order comes from.
    OutOfPhase,
    /// A second commit in one frame.
    ///
    /// The first one ended the frame and produced the only [`Sealed`] there
    /// will be. A second is a peer that has lost track of its own frame
    /// boundary, and accepting it would mean deciding on its behalf which of
    /// the two frames it meant.
    AlreadySealed,
    /// An earlier offer to this batch was refused, so it can never seal.
    ///
    /// The message carried is the *first* refusal's, not the most recent: that
    /// is the one describing what actually went wrong, and every refusal after
    /// it is a consequence.
    Poisoned(&'static str),
    /// The graph would refuse this edit, so the frame is refused instead.
    ///
    /// Answered by [`admit`] before anything is applied. The node the refusal
    /// names is the graph's own answer and reaches the client unchanged.
    Graph(GraphRefusal),
    /// The seal was cut for a frame this batch has since thrown away.
    ///
    /// A [`Batch::reset`] — including the one every [`Batch::commit`] performs
    /// — ends the frame the outstanding seal belonged to, and this is what that
    /// seal is worth afterwards. It is the half of *a frame is applied at most
    /// once* that a moved value cannot state on its own: `Sealed` being neither
    /// `Clone` nor `Copy` stops a second key existing, and this stops a key
    /// outliving the frame it named.
    ///
    /// **What it does not catch**, said here rather than left to be found: two
    /// `Batch`es that have seen the same number of frames carry the same
    /// serial, so a key cut by one would open the other. That is not reachable
    /// across a trust boundary — a seal is produced and consumed inside one
    /// component, and a compositor holds one batch per channel — and closing it
    /// structurally means a generative lifetime brand on both types, which is a
    /// large piece of machinery for a hazard that lives entirely inside one
    /// component's own code. *What would reverse this:* a compositor that
    /// drains two channels' frames through one code path, where the two batches
    /// are chosen by a value rather than by which variable is in scope.
    ///
    /// The one route to it that was *not* a trust boundary is closed:
    /// [`Batch`] is no longer `Copy`, so `let twin = batch;` does not compile
    /// and a second batch carrying one serial cannot be made by assignment.
    /// That was a keystroke, and this paragraph excluded exactly the case it
    /// made easy.
    StaleSeal,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Full => "the frame carries more deltas than this batch was sized for",
            Self::Wire(refusal) => refusal.message(),
            Self::OutOfPhase => {
                "a frame's entries are removals, then creations, then property sets"
            }
            Self::AlreadySealed => "the frame was already closed by a commit",
            Self::Poisoned(first) => first,
            Self::Graph(refusal) => refusal.message(),
            Self::StaleSeal => "the seal was cut for a frame this batch has thrown away",
        }
    }
}

/// What offering one slot's bytes did.
///
/// Two answers and no third, because there are two kinds of entry in a frame:
/// the edits, which accumulate, and the one commit, which ends it. A caller
/// draining a ring matches on this and stops when it has a [`Sealed`].
#[derive(Debug)]
pub enum Offered {
    /// The delta joined the frame. Nothing has reached the graph.
    Staged,
    /// The frame is closed. This is the key, and there is no other.
    Sealed(Sealed),
}

/// The right to apply one frame to one graph, exactly once.
///
/// No public constructor, no `Clone`, no `Copy`, and one producer:
/// [`Batch::offer`] on a `COMMIT`. The module's *[`Sealed`] is the only key* is
/// the whole argument, and the absences above are what make it a statement
/// about the types rather than about anybody's care.
///
/// It carries the [`Frame`] because that is what a commit is *for* — the token
/// and the deadline travel together, which is `f_abi::scene::Frame`'s own
/// reason for existing — and because a caller that has just closed a frame
/// needs both in order to publish it.
#[derive(Debug)]
pub struct Sealed {
    /// The frame this commit closes, as the entry stated it.
    frame: Frame,
    /// Which batch cut this key.
    /// Unit: none — a counter, bumped every time a batch is reset.
    serial: u32,
}

impl Sealed {
    /// The frame this seal closes.
    #[must_use]
    pub const fn frame(&self) -> Frame {
        self.frame
    }
}

/// What a commit did, once it had happened.
///
/// Counts rather than a bare `Ok(())`, because the caller that has just closed
/// a frame is the one that has to complete every entry on the ring and
/// invalidate what changed — and *how many nodes began*, *how many moved* and
/// *how many left* are three different questions for it. Every number here is
/// summed from what `Arena::apply` answered, so none of them is a second
/// opinion about anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed {
    /// The frame that was closed.
    pub frame: Frame,
    /// How many deltas were applied.
    /// Unit: deltas.
    pub edits: usize,
    /// How many nodes began.
    /// Unit: nodes.
    pub created: usize,
    /// How many nodes were re-hung without beginning.
    /// Unit: nodes.
    pub moved: usize,
    /// How many nodes left, subtrees included.
    /// Unit: nodes.
    pub removed: usize,
    /// How many property records were replaced.
    /// Unit: deltas.
    pub set: usize,
}

/// Why a frame was not applied, and what the graph holds because of it.
///
/// Two variants, and they are not degrees of the same thing. One is the
/// ordinary answer to a frame the graph will not have, and it promises the
/// scene is untouched. The other is this build disagreeing with itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The frame was refused and **nothing reached the graph**.
    ///
    /// The scene is exactly what it was before the first delta of this frame
    /// was offered. This is the answer the module's task line is about — *every
    /// delta in it, or none of them* — and this is the *none*.
    Whole {
        /// Which delta of the frame was refused.
        ///
        /// Two refusals have no such delta and say so here rather than
        /// inventing one, because a caller that indexes `edits()` with this
        /// deserves to know which: [`Refusal::Graph`] carrying
        /// `GraphRefusal::Capacity` is a statement about the whole frame and
        /// answers `edits().len()`, one past the last delta; and
        /// [`Refusal::StaleSeal`] is a statement about the key rather than
        /// about any entry and answers zero.
        /// Unit: none — an index into the frame's edits, and `edits().len()`
        /// for a refusal that is about the frame rather than a delta of it.
        at: usize,
        /// Why.
        refusal: Refusal,
    },
    /// [`admit`] said the graph would accept this frame and the graph did not.
    ///
    /// Not reachable from anything a peer can send: admission and application
    /// read the same graph, in the same order, and ask it the same questions.
    /// If it happens, the two halves of this build have come apart — the
    /// concession `arena::Refusal::Mismatched` already makes, made once more
    /// here because the alternative is a compositor that aborts.
    ///
    /// **The graph may hold part of the frame.** That is the one third scene
    /// this file does not remove, and the module's *what is left open* says
    /// where it is closed: in `crate::arena`, by a graph that can rehearse
    /// itself, at which point [`admit`] is deleted rather than repaired.
    Diverged {
        /// Which delta the graph refused.
        /// Unit: none — an index into the frame's edits.
        at: usize,
        /// The graph's own answer.
        refusal: GraphRefusal,
        /// How many deltas had already reached the graph.
        /// Unit: deltas.
        applied: usize,
    },
}

/// Which of a frame's three sections an entry belongs to.
///
/// Ordered, and the ordering is the rule: an entry may open a later section and
/// never an earlier one. The variants are written in the order
/// `crate::reconcile` emits them, so a reader comparing the two files is
/// comparing two statements of one decision rather than two decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    /// `RemoveNode`. What this frame destroys, before anything is moved.
    Removing,
    /// `CreateNode`. Nodes beginning, and nodes re-hung.
    Creating,
    /// `SetTransform`, `SetPath`, `SetPaint`.
    Setting,
}

/// Which section an entry belongs to, or `None` for the one that ends a frame.
///
/// Six arms and no wildcard, each of which decides something: a seventh opcode
/// in `f_abi::scene` stops this build and asks where in a frame it goes. A
/// wildcard — or an arm that fell through to [`Section::Setting`] — is exactly
/// how a seventh opcode comes to be admitted into a frame nobody decided it
/// belonged in.
const fn section_of(entry: &Entry) -> Option<Section> {
    match entry {
        Entry::RemoveNode(_) => Some(Section::Removing),
        Entry::CreateNode(_) => Some(Section::Creating),
        Entry::SetTransform(_) | Entry::SetPath(_) | Entry::SetPaint(_) => Some(Section::Setting),
        Entry::Commit(_) => None,
    }
}

/// A frame under construction: the deltas offered so far, and nothing else.
///
/// **There is no `Arena` in this type and none in [`Batch::offer`]'s
/// signature.** That absence is the module's first decision made structural: a
/// delta cannot reach the graph while a frame is being assembled, because there
/// is nothing here for it to reach.
///
/// `LIMIT` is the component's own statement of the largest frame it will
/// accept, on `crate::reconcile::Deltas`' terms from the producing side.
/// [`DELTAS_MAX`] is the number section 07 argues for.
///
/// **Neither `Clone` nor `Copy`, and that absence is load-bearing.** The
/// module's *a frame cannot be applied twice* rests on the serial that ties a
/// [`Sealed`] to the batch that cut it; a batch that could be duplicated by
/// assignment duplicates that serial, and `let twin = batch;` then produces a
/// second key for one frame and applies it twice — with nothing but
/// [`admit`]'s runtime opinion of the second application standing in the way.
/// [`Refusal::StaleSeal`]'s doc argues that two batches carrying one serial is
/// unreachable across a trust boundary; a derived `Copy` made it one keystroke
/// *inside* a component, which is precisely the case that argument excludes.
#[derive(Debug)]
pub struct Batch<const LIMIT: usize> {
    /// The frame's edits, in arrival order. Only the first `len` mean anything,
    /// and the commit that ends a frame is not among them — it is a [`Sealed`].
    edits: [Delta; LIMIT],
    /// How many of them there are.
    len: usize,
    /// The furthest section any entry so far has opened.
    section: Section,
    /// How many `RemoveNode` entries this frame carries.
    ///
    /// Kept because *this frame destroys something* is the case a cut is most
    /// expensive in, and the one the sweep below is required to have reached —
    /// the role `zone/tests/cut.rs` gives a publish that crosses a root-zone
    /// wrap. Unit: deltas.
    destroys: usize,
    /// The frame has been closed by a commit.
    sealed: bool,
    /// The first refusal this batch suffered, or `None`.
    ///
    /// A batch holding one of these can never seal, which is what makes a frame
    /// that lost an entry not a frame.
    poison: Option<&'static str>,
    /// Which batch this is. Unit: none — a counter.
    serial: u32,
}

impl<const LIMIT: usize> Batch<LIMIT> {
    /// What an unwritten slot holds.
    ///
    /// A removal of [`NO_NODE`], which is `crate::reconcile::Deltas`' filler
    /// and its argument: `f_abi::scene`'s decoder refuses it, so if one ever
    /// escaped past `len` it would be rejected as a malformed entry rather than
    /// applied as an edit. Nothing should read one — `len` bounds every slice
    /// this type hands out — and the filler is chosen so that the failure of
    /// that sentence is loud.
    const FILLER: Delta = Delta {
        user_data: 0,
        class: 0,
        deadline: NO_DEADLINE,
        payload_offset: 0,
        flags: 0,
        body: Entry::RemoveNode(RemoveNode { node: NO_NODE }),
    };

    /// An empty frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            edits: [Self::FILLER; LIMIT],
            len: 0,
            section: Section::Removing,
            destroys: 0,
            sealed: false,
            poison: None,
            serial: 0,
        }
    }

    /// How many deltas this frame carries.
    /// Unit: deltas.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Is the frame still empty?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many removals this frame carries.
    /// Unit: deltas.
    #[must_use]
    pub const fn destroys(&self) -> usize {
        self.destroys
    }

    /// Has this batch suffered a refusal it cannot recover from?
    ///
    /// A poisoned batch will never produce a [`Sealed`]. A caller draining a
    /// ring may stop as soon as this is true; it does not have to, because
    /// every further offer is refused anyway.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poison.is_some()
    }

    /// The deltas offered so far.
    #[must_use]
    pub fn edits(&self) -> &[Delta] {
        &self.edits[..self.len]
    }

    /// Throw the frame away and start another.
    ///
    /// Bumps the serial, so any [`Sealed`] still in flight from the discarded
    /// frame is refused by [`Batch::commit`] rather than opening the next one.
    /// [`Batch::commit`] calls this on both of its paths, which is why a frame
    /// cannot be applied twice and why a refused frame does not linger into the
    /// next one.
    pub fn reset(&mut self) {
        self.len = 0;
        self.section = Section::Removing;
        self.destroys = 0;
        self.sealed = false;
        self.poison = None;
        self.serial = self.serial.wrapping_add(1);
    }

    /// Offer one ring slot's bytes to the frame.
    ///
    /// The only door, and it takes bytes: the module's *why the bytes are the
    /// door* is the argument, and the consequence is that a slot whose payload
    /// had not landed when its entry became visible is refused here rather than
    /// silently dropped from the frame.
    ///
    /// # Errors
    ///
    /// A [`Refusal`], and the batch is poisoned by every one of them —
    /// including [`Refusal::Full`] and [`Refusal::OutOfPhase`]. That is
    /// deliberate and is the whole of *or none of them*: a frame that had one
    /// entry refused is not a frame with a hole in it that could still be
    /// closed, it is a frame that will not happen, and the batch is made unable
    /// to close it rather than trusted not to.
    pub fn offer(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
    ) -> Result<Offered, Refusal> {
        if let Some(first) = self.poison {
            return Err(Refusal::Poisoned(first));
        }
        if self.sealed {
            return Err(self.poisoned(Refusal::AlreadySealed));
        }

        let delta = match Delta::decode(entry, payload) {
            Ok(delta) => delta,
            Err(refusal) => return Err(self.poisoned(Refusal::Wire(refusal))),
        };

        let Some(section) = section_of(&delta.body) else {
            // A commit. `Delta::frame` answers `Some` for exactly this opcode
            // and `section_of` answers `None` for exactly this opcode, so the
            // two agree by construction rather than by a check written here.
            let Some(frame) = delta.frame() else {
                return Err(self.poisoned(Refusal::Wire(WireRefusal::UnknownOpcode)));
            };
            self.sealed = true;
            return Ok(Offered::Sealed(Sealed { frame, serial: self.serial }));
        };

        if section < self.section {
            return Err(self.poisoned(Refusal::OutOfPhase));
        }
        if self.len >= LIMIT {
            return Err(self.poisoned(Refusal::Full));
        }
        self.section = section;
        if section == Section::Removing {
            self.destroys += 1;
        }
        // `self.len < LIMIT` was decided one statement ago, and `edits` is
        // `LIMIT` long: the index below is the only one in this type and this
        // is its whole argument.
        self.edits[self.len] = delta;
        self.len += 1;
        Ok(Offered::Staged)
    }

    /// Record a refusal and hand it back.
    ///
    /// One function, so that *every refusal poisons* is one line rather than a
    /// habit repeated at six call sites — which is the form of that rule an
    /// edit cannot walk past by forgetting a statement.
    fn poisoned(&mut self, refusal: Refusal) -> Refusal {
        if self.poison.is_none() {
            self.poison = Some(refusal.message());
        }
        refusal
    }

    /// Apply the whole frame to the graph, or none of it.
    ///
    /// The [`Sealed`] is consumed and the batch is reset on both paths, so a
    /// frame is applied at most once and a refused frame does not survive into
    /// the next one.
    ///
    /// # Errors
    ///
    /// [`Refused::Whole`] when the graph would refuse an entry, with the scene
    /// exactly as it was. [`Refused::Diverged`] when admission and the graph
    /// disagreed, which is this build broken rather than this frame bad; see
    /// that variant.
    pub fn commit(&mut self, arena: &mut Arena, sealed: Sealed) -> Result<Closed, Refused> {
        if sealed.serial != self.serial {
            // The key names a frame this batch no longer has, so there is no
            // frame here to throw away either: the batch is left exactly as it
            // was found, which is what lets the frame it *is* assembling
            // survive a stale key arriving late.
            return Err(Refused::Whole { at: 0, refusal: Refusal::StaleSeal });
        }
        let frame = sealed.frame;

        if let Err((at, refusal)) = admit::<LIMIT>(arena, self.edits()) {
            self.reset();
            return Err(Refused::Whole { at, refusal });
        }

        let mut closed =
            Closed { frame, edits: self.len, created: 0, moved: 0, removed: 0, set: 0 };
        for (at, delta) in self.edits[..self.len].iter().enumerate() {
            match arena.apply(delta) {
                Ok(Applied::Hung(Hung::Created)) => closed.created += 1,
                Ok(Applied::Hung(Hung::Moved)) => closed.moved += 1,
                Ok(Applied::Removed(gone)) => closed.removed += gone,
                Ok(Applied::Set(_)) => closed.set += 1,
                // A commit never enters `edits` — `offer` turns it into a
                // `Sealed` instead — so a commit reaching here is this module
                // disagreeing with itself, and it is reported rather than
                // stepped over for the reason `arena::Refusal::Mismatched` is.
                Ok(Applied::Closes(_)) => {
                    self.reset();
                    return Err(Refused::Diverged {
                        at,
                        refusal: GraphRefusal::Mismatched,
                        applied: at,
                    });
                }
                Err(refusal) => {
                    self.reset();
                    return Err(Refused::Diverged { at, refusal, applied: at });
                }
            }
        }
        self.reset();
        Ok(closed)
    }
}

impl<const LIMIT: usize> Default for Batch<LIMIT> {
    fn default() -> Self {
        Self::new()
    }
}

/// What the batch has done to one node, so far, without having done it.
///
/// Three facts and no fourth, because three is what `Arena::hang` asks about a
/// node it is handed: *is it there*, *what is it*, and *whose child is it*. The
/// tree itself is never copied — everything about a node this frame has not
/// touched is asked of the graph.
#[derive(Clone, Copy)]
struct Touched {
    /// Unit: none — a node identifier.
    node: u32,
    /// Unit: none — an `f_abi::scene::kind` constant.
    kind: u16,
    /// Unit: none — a node identifier, or `NO_NODE` for a root.
    parent: u32,
}

/// The batch's own effects, as admission accumulates them.
///
/// Bounded by `LIMIT` in both arrays, and the bound is exact rather than
/// generous: every entry of a frame notes at most one node and every removal
/// notes at most one root, so a frame of `LIMIT` entries cannot fill either
/// array past `LIMIT`. That sentence is the argument for every index in this
/// struct, and it is the reason the two `push` sites below can be written
/// without a failure path.
struct Ledger<const LIMIT: usize> {
    /// Nodes this frame has created or re-hung, in the order it did.
    touched: [Touched; LIMIT],
    /// Unit: nodes.
    touched_len: usize,
    /// The roots this frame removes.
    removed: [u32; LIMIT],
    /// Unit: nodes.
    removed_len: usize,
}

impl<const LIMIT: usize> Ledger<LIMIT> {
    /// Nothing noted yet.
    const fn new() -> Self {
        Self {
            touched: [Touched { node: NO_NODE, kind: 0, parent: NO_NODE }; LIMIT],
            touched_len: 0,
            removed: [NO_NODE; LIMIT],
            removed_len: 0,
        }
    }

    /// What this frame last said about a node, or `None` if it has said
    /// nothing.
    fn touched(&self, node: u32) -> Option<Touched> {
        self.touched[..self.touched_len].iter().rev().find(|seen| seen.node == node).copied()
    }

    /// Note that this frame has hung a node here.
    ///
    /// Appends rather than overwriting, and [`Ledger::touched`] reads backwards,
    /// so a node hung twice in one frame answers with the second placement
    /// without this function having to search for the first.
    fn hung(&mut self, node: u32, kind: u16, parent: u32) {
        if self.touched_len < LIMIT {
            self.touched[self.touched_len] = Touched { node, kind, parent };
            self.touched_len += 1;
        }
    }

    /// Note that this frame removes the subtree at this root.
    fn removes(&mut self, root: u32) {
        if self.removed_len < LIMIT {
            self.removed[self.removed_len] = root;
            self.removed_len += 1;
        }
    }

    /// Is this node at or under something this frame removes?
    ///
    /// Asked of the graph as it stands, which is exact because every removal in
    /// a frame precedes every re-hang in it — the module's *a frame's entries
    /// arrive in sections*. Nothing has moved yet, so ancestry now is ancestry
    /// at the moment the removals run.
    ///
    /// The walk is bounded by [`NODES_MAX`] because a graph with a cycle in it
    /// would otherwise not terminate. There is no such graph — `Arena` refuses
    /// the edit that would make one — and the bound is written anyway, for
    /// `arena::SlotIx::at`'s reason: *the possibility is removed* and *the
    /// possibility is guarded* are different states, and only one of them
    /// survives an edit by somebody who has not read this paragraph.
    fn destroys(&self, arena: &Arena, node: u32) -> bool {
        let mut at = node;
        let mut steps = 0;
        while steps < NODES_MAX {
            if self.removed[..self.removed_len].contains(&at) {
                return true;
            }
            match arena.parent_of(at) {
                Ok(NO_NODE) | Err(_) => return false,
                Ok(parent) => at = parent,
            }
            steps += 1;
        }
        // A graph deep enough to exhaust the bound is one this module cannot
        // reason about, and refusing the frame is the conservative half.
        true
    }

    /// Will the graph hold this node when this frame's edits reach it?
    fn holds(&self, arena: &Arena, node: u32) -> bool {
        if self.touched(node).is_some() {
            return true;
        }
        arena.holds(node) && !self.destroys(arena, node)
    }

    /// Whose child this node will be, or [`NO_NODE`] for a root.
    ///
    /// Only meaningful for a node [`Ledger::holds`] answers `true` for, and
    /// every caller below asks that first.
    fn parent_of(&self, arena: &Arena, node: u32) -> u32 {
        match self.touched(node) {
            Some(seen) => seen.parent,
            None => arena.parent_of(node).unwrap_or(NO_NODE),
        }
    }

    /// What kind of node this will be, or `None` for one that will not be
    /// there.
    fn kind_of(&self, arena: &Arena, node: u32) -> Option<u16> {
        match self.touched(node) {
            Some(seen) => Some(seen.kind),
            None => arena.kind_of(node).map(|kind| kind.wire()),
        }
    }

    /// Walking upwards from `from`, is `target` an ancestor of it — or `from`
    /// itself?
    ///
    /// `Arena::hang`'s cycle check, asked of the tree this frame will have
    /// produced by the time the edit runs. Bounded for [`Ledger::destroys`]'s
    /// reason, and exhaustion is refusal for the same one.
    fn reaches(&self, arena: &Arena, from: u32, target: u32) -> bool {
        let mut at = from;
        let mut steps = 0;
        while steps < NODES_MAX {
            if at == target {
                return true;
            }
            if at == NO_NODE {
                return false;
            }
            at = self.parent_of(arena, at);
            steps += 1;
        }
        true
    }
}

/// The first child of a node, or the first root of the forest.
fn first_child(arena: &Arena, node: u32) -> Option<u32> {
    if node == NO_NODE {
        return arena.roots().next();
    }
    arena.children(node).ok().and_then(|mut walk| walk.next())
}

/// The sibling standing behind this one in paint order, or `None` for the last.
fn next_sibling(arena: &Arena, parent: u32, node: u32) -> Option<u32> {
    let mut walk = if parent == NO_NODE {
        arena.roots()
    } else {
        match arena.children(parent) {
            Ok(children) => children,
            Err(_) => return None,
        }
    };
    while let Some(sibling) = walk.next() {
        if sibling == node {
            return walk.next();
        }
    }
    None
}

/// How many slots removing this subtree gives back.
///
/// A pre-order walk that carries **no work list at all**: down to the first
/// child, then sideways, then up — three questions the graph already answers,
/// and one node's worth of state. That is the discipline `Arena::remove` keeps
/// for the same reason, and it is what lets a frame's capacity be decided
/// exactly rather than bounded below by the number of removals, which would
/// refuse every frame that replaces a full scene with another full one.
///
/// The cost is stated rather than hidden: *sideways* is a scan of the sibling
/// list, so this is quadratic in a wide subtree, and it is walked once per
/// removal in a frame. *What would reverse this:* the measured frame in which
/// it shows against section 12's budget, at which point `Arena` grows a subtree
/// count and this function is deleted rather than optimised.
/// Unit: nodes.
fn freed_by(arena: &Arena, root: u32) -> usize {
    if !arena.holds(root) {
        return 0;
    }
    let mut count = 1usize;
    let mut at = root;
    let mut descend = true;
    // Every node is descended into once and climbed out of once, so twice the
    // arena is a bound no acyclic graph reaches and no cyclic one exceeds.
    let mut budget = NODES_MAX * 2;
    while budget > 0 {
        budget -= 1;
        if descend && let Some(child) = first_child(arena, at) {
            at = child;
            count += 1;
            continue;
        }
        if at == root {
            break;
        }
        let parent = arena.parent_of(at).unwrap_or(NO_NODE);
        if let Some(sibling) = next_sibling(arena, parent, at) {
            at = sibling;
            count += 1;
            descend = true;
        } else if parent == NO_NODE {
            break;
        } else {
            at = parent;
            descend = false;
        }
    }
    count
}

/// Will the graph accept every delta of this frame?
///
/// Reads the graph and writes nothing to it. Every question it asks is asked of
/// `Arena` — presence, ancestry, kind, room — so the only rules stated in this
/// function's own voice are the ones `Arena::hang` states, in the order it
/// states them, and a reader comparing the two is comparing two spellings of
/// one list rather than two lists.
///
/// The two refusals it does *not* have to make are worth naming, because their
/// absence is a property and not an oversight. `arena::Refusal::Wire` cannot
/// happen: `Batch::offer` decoded every delta here through `Delta::decode`, and
/// `kind::Change::of`'s remaining refusals — a zero node identifier, a kind
/// outside the six — are refusals `CreateNode::read` and `RemoveNode::read`
/// already made of the same bytes. `arena::Refusal::Mismatched` cannot happen
/// either: it is `Change::of` and this build's opcode table disagreeing, which
/// is not something a frame can arrange.
///
/// # Errors
///
/// The index of the offending delta and why. The graph is untouched either way;
/// that is the entire point of the function.
fn admit<const LIMIT: usize>(arena: &Arena, edits: &[Delta]) -> Result<(), (usize, Refusal)> {
    // The ledger is sized by the batch's own bound and not by `DELTAS_MAX`,
    // which is a default and not a law. A ledger smaller than the frame it is
    // accumulating would quietly stop recording — and a removal the ledger
    // forgot is a node admission believes is still there, which is precisely a
    // `Refused::Diverged` manufactured by an arithmetic mistake rather than by
    // the two halves genuinely disagreeing. Tying the two bounds together is
    // how that stops being something to remember.
    let mut ledger = Ledger::<LIMIT>::new();
    let mut needed = 0usize;
    let mut freed = 0usize;

    for (at, delta) in edits.iter().enumerate() {
        match delta.body {
            Entry::RemoveNode(remove) => {
                // A removal of something already inside another removal of this
                // same frame names a node that will not be there when it runs.
                // `reconcile` never emits one — it emits the root of each
                // vanished subtree and nothing under it — and the graph would
                // answer `NoSuchNode`, so that is the answer here.
                if !arena.holds(remove.node) || ledger.destroys(arena, remove.node) {
                    return Err((at, Refusal::Graph(GraphRefusal::NoSuchNode(remove.node))));
                }
                freed += freed_by(arena, remove.node);
                ledger.removes(remove.node);
            }
            Entry::CreateNode(create) => {
                // `CreateNode::read` has already refused `parent == node` and
                // `before == node` — the only cycle and the only self-sibling
                // one entry can state on its own. They are not re-checked here:
                // they are checked by the decoder every delta in this batch
                // came through.
                if create.parent != NO_NODE && !ledger.holds(arena, create.parent) {
                    return Err((at, Refusal::Graph(GraphRefusal::NoSuchParent(create.parent))));
                }
                if create.before != NO_NODE
                    && (!ledger.holds(arena, create.before)
                        || ledger.parent_of(arena, create.before) != create.parent)
                {
                    return Err((at, Refusal::Graph(GraphRefusal::NotASibling(create.before))));
                }
                if ledger.holds(arena, create.node) {
                    // A re-hang. The node keeps what it is, and may not be hung
                    // under its own descendant.
                    if ledger.kind_of(arena, create.node) != Some(create.kind) {
                        return Err((at, Refusal::Graph(GraphRefusal::KindChanged(create.node))));
                    }
                    if create.parent != NO_NODE && ledger.reaches(arena, create.parent, create.node)
                    {
                        return Err((at, Refusal::Graph(GraphRefusal::Cycle(create.node))));
                    }
                } else {
                    needed += 1;
                }
                ledger.hung(create.node, create.kind, create.parent);
            }
            Entry::SetTransform(record) => {
                if !ledger.holds(arena, record.node) {
                    return Err((at, Refusal::Graph(GraphRefusal::NoSuchNode(record.node))));
                }
            }
            Entry::SetPath(record) => {
                if !ledger.holds(arena, record.node) {
                    return Err((at, Refusal::Graph(GraphRefusal::NoSuchNode(record.node))));
                }
            }
            Entry::SetPaint(record) => {
                if !ledger.holds(arena, record.node) {
                    return Err((at, Refusal::Graph(GraphRefusal::NoSuchNode(record.node))));
                }
            }
            // `Batch::offer` turns a commit into a `Sealed` and never stores
            // one, so a commit among the edits is this module disagreeing with
            // itself. The arm decides rather than falling through, which is the
            // whole reason this match has no wildcard.
            Entry::Commit(_) => return Err((at, Refusal::AlreadySealed)),
        }
    }

    // Room, last, and over the whole frame rather than per entry: the removals
    // run before the creations, so the slots they give back are slots the
    // creations may have. `freed` is exact — the removed subtrees are disjoint,
    // which the `destroys` check above is what guarantees — so a frame that
    // replaces a full scene with another full one is admitted rather than
    // refused by an arithmetic that assumed the worst.
    if needed > arena.remaining() + freed {
        return Err((edits.len(), Refusal::Graph(GraphRefusal::Capacity)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::{Deltas, Node, Reconciler};
    use f_abi::scene::{Commit, CreateNode, SetPaint, kind};

    /// The reconciler's bound for the corpus trees. Unit: nodes.
    const NODES: usize = 16;

    /// The batch bound every corpus frame is built and swept at.
    /// Unit: deltas.
    const LIMIT: usize = 48;

    /// Ring slots one commit occupies: its edits and the commit that ends it.
    /// Unit: entries.
    const SLOTS: usize = LIMIT + 1;

    /// The key space the corpus draws from, and the one the scene is
    /// fingerprinted over.
    ///
    /// Small on purpose, and for `zone/tests/cut.rs`'s reason for a small
    /// device: keys have to *recur* across frames, or the corpus never produces
    /// a move, a re-hang, or a key reused after the subtree holding it was
    /// removed — and those are the frames where a cut is interesting. A key
    /// space larger than a frame is a corpus of creations and removals only.
    /// Unit: none — node identifiers, one through this.
    const KEYS: u32 = 10;

    /// The root every corpus tree hangs under.
    /// Unit: none — a node identifier.
    const ROOT: u32 = 1;

    /// Seeds swept. Unit: count of seeds.
    ///
    /// Sixteen, and the honest derivation is a budget rather than a floor:
    /// it is what `cargo test` pays for in a couple of seconds. It is **not**
    /// the least number that reaches the required observations — measured,
    /// two seeds already reach every one of them — and an earlier version of
    /// this comment said it was, which was a derivation written after the
    /// number and not before it. What sixteen buys over two is margin: the
    /// rarest observation here is a torn payload the decoder refuses, which
    /// needs a tear falling inside a record rather than in the padding after
    /// it, and margin on the rarest is what stops a corpus change from
    /// silently turning a required observation into a coin toss.
    ///
    /// *What would reverse this:* the sweep costing enough to be skipped. A
    /// number that makes a run slow is a number that makes the run optional,
    /// which is worse than a smaller one.
    const SEEDS: u64 = 16;

    /// Commits published per seed. Unit: count of commits.
    ///
    /// Three, which is the fewest that reaches all three of the things a target
    /// can be: the first commit, which has no scene behind it; a middle one;
    /// and one whose predecessors have already built a scene deep enough for a
    /// removal to take a subtree rather than a leaf.
    const FRAMES: u64 = 3;

    /// The seed the sweep starts from. Unit: none — a seed.
    const FIRST_SEED: u64 = 1;

    /// The deadline the corpus commits are scheduled against.
    ///
    /// A constant and not a clock: this crate reads no time, and a commit's
    /// deadline is a number some caller computed. It has to be something other
    /// than `NO_DEADLINE`, because `f_abi::scene` refuses a commit carrying
    /// none. Unit: nanoseconds, monotonic, in this channel's epoch. RFC 0009.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// FNV-1a's offset basis. Unit: none — a hash state.
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

    /// FNV-1a's prime. Unit: none — a multiplier.
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    /// At what granularity a cut falls inside an entry.
    ///
    /// `zone/tests/cut.rs`'s `Granularity`, with that file's block and byte
    /// renamed to the two things a ring slot is made of.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Granularity {
        /// An entry arrived whole or not at all.
        Whole,
        /// The cut falls inside the last entry submitted, past the barrier: a
        /// prefix of its payload is in the arena and the rest is still what the
        /// slot held before. This is the only granularity at which a torn delta
        /// — one of the required observations — can be produced at all, and a
        /// barrier that covers the slot excludes it, which is why an honest ring
        /// tears nothing: a promise of visibility is a promise about all the
        /// bytes.
        Payload,
    }

    impl Granularity {
        /// The word the reproduction line uses.
        const fn name(self) -> &'static str {
            match self {
                Self::Whole => "whole",
                Self::Payload => "payload",
            }
        }
    }

    /// What the commit entry's publication says about the entries before it.
    ///
    /// `zone/tests/cut.rs`'s `Mode`, and the analogy is exact once the barrier
    /// is named. There, a `FLUSH` promises that every write before it is on the
    /// media, and a lying device ignores it. Here, the ring's one `Release`
    /// store promises that every entry before the published tail is visible,
    /// and a build that made it `Relaxed` would promise nothing — `Relaxed`
    /// there passes on x86 and corrupts data on AArch64, which is the
    /// convention this repository already states.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mode {
        /// Every entry before the cut is in its slot. What the `Release` and
        /// `Acquire` pair buys.
        Honest,
        /// The pair promises nothing, so a drawn subset of the slots before the
        /// cut still holds whatever occupied it last — a previous frame's
        /// entry, which decodes.
        ///
        /// **A commit's atomicity does rest on this being false**, and this
        /// mode is the measurement of by how much rather than a case the sweep
        /// exists to clear: the module's *the limit of that last one* is the
        /// argument, and the run requires this mode to produce third scenes for
        /// the same reason it requires [`Applier::Eager`] to. A zero here would
        /// mean the model had stopped being faithful, not that the module had
        /// grown a defence.
        Lying,
    }

    impl Mode {
        /// The word the reproduction line uses.
        const fn name(self) -> &'static str {
            match self {
                Self::Honest => "honest",
                Self::Lying => "lying",
            }
        }
    }

    /// Which reading of *a frame is atomic* a cut is replayed against.
    ///
    /// Two, and the second is the control. `zone/tests/cut.rs` gets its control
    /// from a cargo feature that rebuilds the workspace with a barrier removed;
    /// this one is in the same binary, because the defect it models is not a
    /// missing instruction but a missing *type* — the obvious compositor,
    /// draining the ring straight into the graph. A control that cannot be
    /// forgotten is worth more than one that has to be remembered.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Applier {
        /// This module: nothing reaches the graph until a [`Sealed`] exists.
        Staged,
        /// `Arena::apply` per entry as it arrives, with the commit doing
        /// nothing — which is exactly what `arena::Applied::Closes` is, and
        /// exactly why that arm hands the token on rather than pretending the
        /// frame is finished.
        Eager,
    }

    impl Applier {
        /// The word the reproduction line uses.
        const fn name(self) -> &'static str {
            match self {
                Self::Staged => "staged",
                Self::Eager => "eager",
            }
        }
    }

    /// Which of the two intended scenes a cut left, or neither.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Outcome {
        /// The scene as it stood before the frame began.
        Old,
        /// The scene the frame describes.
        New,
        /// Anything else, which is what the exit forbids.
        Third,
    }

    /// One draw, and the whole of the randomness in this file.
    ///
    /// SplitMix64, written out rather than taken from `f_env`: `f-scene`
    /// depends only on `f-abi`, and adding a dependency to reach a generator
    /// would be a `Cargo.toml` this subtask may not write. It costs nothing,
    /// because what a sweep needs is not entropy — it is *a pure function of a
    /// stated key*, which is a stronger determinism claim than a drawn one.
    /// Every value below derives from `(seed, target, cut, granularity, mode)`
    /// and from nothing else, so a failing cut reproduces from the line the
    /// failure prints and from no other state. RFC 0004's *nothing observes
    /// randomness except through `f_env::Env`* is satisfied here by observing
    /// none.
    fn draw(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = *state;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    }

    /// One draw site, folded from the parts of the re-run key.
    ///
    /// Folded rather than chained, for `zone/tests/cut.rs`'s reason: adding a
    /// fourth part later must not move the three already there, or generation
    /// twelve stops being the same generation it was yesterday.
    fn site(parts: &[u64]) -> u64 {
        let mut state = 0x243F_6A88_85A3_08D3;
        for part in parts {
            state ^= *part;
            state = draw(&mut state);
        }
        state
    }

    /// The identity every draw for one cut derives from: the re-run key itself.
    fn key(seed: u64, target: u64, cut: usize, granularity: Granularity, mode: Mode) -> u64 {
        let flavour =
            u64::from(granularity == Granularity::Payload) << 1 | u64::from(mode == Mode::Lying);
        site(&[seed, target, cut as u64, flavour])
    }

    /// What kind of node a key is, for the life of the corpus.
    ///
    /// A function of the key alone, because `reconcile` refuses a key that
    /// changes what it is and the graph refuses it a second time. A corpus that
    /// drew the kind per frame would be a corpus of refusals.
    const fn kind_of_key(node: u32) -> u16 {
        // The six kinds are one through six. A key maps onto one of them and
        // never onto zero, which names no kind and is refused by the decoder.
        1 + (node % 6) as u16
    }

    /// One frame's tree, as a pure function of `(seed, frame)`.
    ///
    /// Answers how many nodes it wrote. Every tree is one `reconcile::validate`
    /// accepts — a root, then nodes whose parents appear earlier, no duplicate
    /// key, no property record naming a node — because a corpus tree the
    /// reconciler refused would make the sweep a test of the generator.
    fn tree(seed: u64, frame: u64, out: &mut [Node; NODES]) -> usize {
        let mut state = site(&[seed, frame, 0x7265_6500]);
        out[0] = Node::new(ROOT, NO_NODE, kind::LAYER);
        let mut len = 1usize;
        let mut used: u32 = 1 << ROOT;
        let want = 3 + (draw(&mut state) % 5) as usize;
        for _ in 0..want {
            if len >= NODES {
                break;
            }
            let mut node = 2 + (draw(&mut state) % u64::from(KEYS - 1)) as u32;
            let mut tries = 0;
            while used & (1 << node) != 0 && tries < KEYS {
                node = 2 + (node - 1) % (KEYS - 1);
                tries += 1;
            }
            if used & (1 << node) != 0 {
                continue;
            }
            used |= 1 << node;
            let parent = out[(draw(&mut state) as usize) % len].key;
            let mut built = Node::new(node, parent, kind_of_key(node));
            // A property on most nodes, so that a frame carries a third section
            // and not only the first two. The value is drawn rather than fixed,
            // so a node that survives a frame still has something to differ
            // about.
            if !draw(&mut state).is_multiple_of(3) {
                built.paint.green_x65535 = (draw(&mut state) % 65_536) as u16;
                built.paint.alpha_x65535 = u16::MAX;
            }
            out[len] = built;
            len += 1;
        }
        len
    }

    /// The frame token one commit carries.
    ///
    /// Never zero: `f_abi::scene::Commit` refuses that, so a zeroed payload is
    /// not a commit of frame zero.
    fn token(seed: u64, frame: u64) -> u64 {
        site(&[seed, frame, 0x746F_6B65_6E00]).max(1)
    }

    /// The delta one entry crosses as.
    ///
    /// The deadline is decided per opcode with no wildcard, because that is the
    /// rule `f_abi::scene` states from the other side: exactly one opcode reads
    /// a deadline, and every other must carry `NO_DEADLINE` or be refused.
    fn delta_of(body: Entry) -> Delta {
        let deadline = match body {
            Entry::Commit(_) => SCHEDULED_AT,
            Entry::CreateNode(_)
            | Entry::SetTransform(_)
            | Entry::SetPath(_)
            | Entry::SetPaint(_)
            | Entry::RemoveNode(_) => NO_DEADLINE,
        };
        Delta { user_data: 0, class: 0, deadline, payload_offset: 0, flags: 0, body }
    }

    /// The ring slots one commit occupies: its edits, then the commit.
    fn slots_of(entries: &[Entry], frame_token: u64) -> ([Delta; SLOTS], usize) {
        let mut slots = [delta_of(Entry::Commit(Commit { frame_token: 1 })); SLOTS];
        let mut len = 0usize;
        for entry in entries {
            slots[len] = delta_of(*entry);
            len += 1;
        }
        slots[len] = delta_of(Entry::Commit(Commit { frame_token }));
        len += 1;
        (slots, len)
    }

    /// Submit a whole frame the way a healthy ring delivers one.
    ///
    /// Panics on anything but success, and that is the point: every corpus
    /// frame comes out of `reconcile` against the very graph it is applied to,
    /// so a refusal here is [`admit`] refusing a frame the graph would have
    /// taken — over-refusal, which is a finding about this module and not about
    /// the corpus.
    fn deliver(arena: &mut Arena, entries: &[Entry], frame_token: u64) -> Closed {
        let (slots, len) = slots_of(entries, frame_token);
        let mut batch = Batch::<LIMIT>::new();
        for slot in &slots[..len] {
            let (sqe, payload) = slot.encode();
            match batch.offer(&sqe, &payload) {
                Ok(Offered::Staged) => {}
                Ok(Offered::Sealed(sealed)) => match batch.commit(arena, sealed) {
                    Ok(closed) => return closed,
                    Err(refused) => panic!("a corpus frame was refused: {refused:?}"),
                },
                Err(refusal) => panic!("a corpus entry was refused: {}", refusal.message()),
            }
        }
        panic!("a corpus frame carried no commit")
    }

    /// Where a node stands among its siblings, in paint order.
    ///
    /// Part of the fingerprint because sibling order *is* paint order: two
    /// scenes holding the same nodes under the same parents in a different
    /// order are two different pictures, and a fingerprint that could not tell
    /// them apart would report a reordered third scene as one of the two
    /// intended ones. Unit: none — an index among siblings.
    fn position_of(arena: &Arena, parent: u32, node: u32) -> u32 {
        let walk =
            if parent == NO_NODE { Some(arena.roots()) } else { arena.children(parent).ok() };
        let Some(walk) = walk else { return u32::MAX };
        for (at, sibling) in walk.enumerate() {
            if sibling == node {
                return at as u32;
            }
        }
        u32::MAX
    }

    /// Fold bytes into a hash.
    fn feed(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(FNV_PRIME);
        }
    }

    /// The whole scene as one number.
    ///
    /// **This is what *the graph read back* means in the exit, so what it does
    /// and does not distinguish is the reach of the whole sweep.** It walks the
    /// key space in order — so it is a canonical encoding and not a traversal
    /// whose answer depends on where it started — and for every key it folds in
    /// whether the scene holds it, what kind it is, whose child it is, where it
    /// stands among its siblings, and each of its three property records.
    ///
    /// The property records are folded as the bytes `f_abi::scene` encodes them
    /// into, rather than field by field. That is deliberate: a field added to
    /// `SetPaint` changes this fingerprint on the day it is declared, with
    /// nobody remembering — the same reason `reconcile`'s corpus builds its
    /// expectations from the same constructor its trees use.
    ///
    /// A collision would report a third scene as one of the two intended ones.
    /// Sixty-four bits of FNV-1a over a couple of hundred bytes per key is not
    /// a cryptographic claim and is not offered as one; what makes it adequate
    /// here is that the sweep also runs [`Applier::Eager`] over the same cuts
    /// and **requires it to produce a third scene**, so a fingerprint that had
    /// collapsed into a constant would fail the run rather than pass it.
    fn fingerprint(arena: &Arena) -> u64 {
        let mut hash = FNV_OFFSET;
        for node in 1..=KEYS {
            if !arena.holds(node) {
                feed(&mut hash, &[0]);
                continue;
            }
            feed(&mut hash, &[1]);
            feed(&mut hash, &node.to_le_bytes());
            feed(&mut hash, &arena.kind_of(node).map_or(0, |kind| kind.wire()).to_le_bytes());
            let parent = arena.parent_of(node).unwrap_or(NO_NODE);
            feed(&mut hash, &parent.to_le_bytes());
            feed(&mut hash, &position_of(arena, parent, node).to_le_bytes());
            match arena.transform_of(node) {
                Ok(Some(record)) => {
                    feed(&mut hash, &[1]);
                    feed(&mut hash, &Entry::SetTransform(record).payload());
                }
                Ok(None) | Err(_) => feed(&mut hash, &[0]),
            }
            match arena.path_of(node) {
                Ok(Some(record)) => {
                    feed(&mut hash, &[1]);
                    feed(&mut hash, &Entry::SetPath(record).payload());
                }
                Ok(None) | Err(_) => feed(&mut hash, &[0]),
            }
            match arena.paint_of(node) {
                Ok(Some(record)) => {
                    feed(&mut hash, &[1]);
                    feed(&mut hash, &Entry::SetPaint(record).payload());
                }
                Ok(None) | Err(_) => feed(&mut hash, &[0]),
            }
        }
        hash
    }

    /// One commit's entries, and the two scenes a cut inside it may leave.
    struct Recorded {
        /// Unit: count of commits since the empty scene.
        target: u64,
        /// The ring slots, the commit last.
        slots: [Delta; SLOTS],
        /// Unit: entries, the commit included.
        len: usize,
        /// Whether this frame destroys a subtree — the case a cut is most
        /// expensive in, and the third required observation.
        destroys: bool,
        /// The scene a cut that landed nothing leaves.
        /// Unit: none — a fingerprint.
        old: u64,
        /// The scene a cut that landed everything leaves.
        /// Unit: none — a fingerprint.
        new: u64,
        /// The token the commit entry carries when it arrives whole.
        /// Unit: none — a frame identifier.
        token: u64,
        /// What the ring slots held before this frame was written into them.
        ///
        /// **This is the model's load-bearing element**, and `zone/tests/cut.rs`
        /// is where it comes from: there, an operation that does not land is
        /// simply not performed against a device that already holds every
        /// earlier publish, so the medium retains its prior content. A ring is
        /// the same medium with a shorter memory — `ring/src/lib.rs` initialises
        /// its slots once and nothing zeroes one on completion — so a slot whose
        /// write is not visible holds the entry that occupied it last, which is
        /// a perfectly decodable delta from an earlier frame.
        ///
        /// **The last frame that reached each slot, slot by slot — not the
        /// previous frame's.** The two differ exactly where the previous frame
        /// was shorter than this one, and the difference is not cosmetic: under
        /// the previous-frame rule those slots fell back to the zeros of a fresh
        /// mapping, which is the substitution this whole model exists to remove,
        /// and a measurement found 66 of them in a single sweep. A producer
        /// writing shorter and longer frames into one ring leaves each slot
        /// holding whatever frame last reached *it*, so that is what this holds,
        /// and `prior_len` is then the ring's high-water mark rather than one
        /// frame's length.
        prior: [Delta; SLOTS],
        /// How far into the ring the producer has ever written — the high-water
        /// mark over every frame before this one, not the previous frame's
        /// length. Zero for the first commit of a run, whose slots are still the
        /// zeros of a fresh mapping, and that is the only case those zeros are
        /// the honest answer.
        /// Unit: entries.
        prior_len: usize,
    }

    /// What slot `at` held before this frame reached it.
    ///
    /// The zeros of a fresh mapping only where the producer has genuinely never
    /// reached that slot — `at` past the high-water mark of every frame before
    /// this one. Everything else is the entry of whichever earlier frame last
    /// wrote that slot, encoded the way the producer wrote it.
    ///
    /// The sentence above was false while `prior_len` was the previous frame's
    /// length: a target frame longer than the one before it fell back to zeros
    /// for the slots past that length, and those are slots an earlier frame had
    /// reached. Measured at 66 stale offers in one sweep, every one of them in
    /// the direction that makes this module's exit easier — a zeroed slot is
    /// refused by the decoder, and a decodable one is the case the exit is
    /// about.
    fn prior_bytes(recorded: &Recorded, at: usize) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        if at < recorded.prior_len {
            recorded.prior[at].encode()
        } else {
            (Sqe::ZERO, [0u8; PAYLOAD_BYTES])
        }
    }

    /// Replay the commits before `before` into a graph.
    ///
    /// Re-run rather than snapshotted, for `zone/tests/cut.rs`'s reason: a
    /// snapshot of a graph is a second representation of its state, and this
    /// file would then be asserting that the two agree.
    fn replay(seed: u64, before: u64, arena: &mut Arena) {
        let mut reconciler = Reconciler::<NODES>::new();
        let mut nodes = [Node::UNUSED; NODES];
        for frame in 1..before {
            let len = tree(seed, frame, &mut nodes);
            let mut out = Deltas::<LIMIT>::new();
            let built = reconciler.frame(&nodes[..len], &mut out);
            assert!(built.is_ok(), "the corpus tree at frame {frame} is not one to reconcile");
            deliver(arena, out.as_slice(), token(seed, frame));
        }
    }

    /// Run the corpus up to and including `target`, recording that frame's
    /// entries and the two scenes its cuts may leave.
    ///
    /// `None` for a frame that changes nothing: a target whose old and new
    /// scenes are the same number is one the sweep cannot tell two outcomes
    /// apart at, and counting it would be counting a cut that observed nothing.
    fn record(seed: u64, target: u64) -> Option<Recorded> {
        let mut arena = Arena::EMPTY;
        let mut reconciler = Reconciler::<NODES>::new();
        let mut nodes = [Node::UNUSED; NODES];
        let mut prior = [delta_of(Entry::Commit(Commit { frame_token: 1 })); SLOTS];
        let mut prior_len = 0usize;
        for frame in 1..target {
            let len = tree(seed, frame, &mut nodes);
            let mut out = Deltas::<LIMIT>::new();
            assert!(reconciler.frame(&nodes[..len], &mut out).is_ok());
            // Every frame before the target writes its own slots, so what the
            // target's slots hold when the producer starts writing into them is
            // the last frame that reached each one — which is this frame for
            // the slots it covers and an older frame for the rest.
            let (slots, len) = slots_of(out.as_slice(), token(seed, frame));
            prior[..len].copy_from_slice(&slots[..len]);
            if len > prior_len {
                prior_len = len;
            }
            deliver(&mut arena, out.as_slice(), token(seed, frame));
        }
        let old = fingerprint(&arena);

        let len = tree(seed, target, &mut nodes);
        let mut out = Deltas::<LIMIT>::new();
        assert!(reconciler.frame(&nodes[..len], &mut out).is_ok());
        let entries = out.as_slice();
        let destroys = entries.iter().any(|entry| matches!(entry, Entry::RemoveNode(_)));
        let frame_token = token(seed, target);
        let (slots, slot_len) = slots_of(entries, frame_token);
        deliver(&mut arena, entries, frame_token);
        let new = fingerprint(&arena);

        // `new` came out of `deliver`, which is `Batch::offer` and
        // `Batch::commit` — the code this sweep is about. An oracle produced by
        // the code under test cannot see a defect that is uniform across every
        // frame: a `Batch::commit` that dropped the last delta of every frame
        // would redefine `new` to match itself, and the sweep would report the
        // new scene as reached. Measured, before this check existed: mutating
        // the apply loop to `self.edits[..self.len - 1]` left the sweep green.
        //
        // So the same frame is applied a second way — `Arena::apply` per delta,
        // in the order the frame states, with no batch, no seal and no
        // admission — and the two are required to agree. That is a different
        // function reached by a different route, which is the whole of what
        // makes it an oracle. The prefix is replayed identically in both, on
        // purpose: what is being checked is this frame's application, not the
        // corpus's.
        let mut oracle = Arena::EMPTY;
        replay(seed, target, &mut oracle);
        for entry in entries {
            let delta = delta_of(*entry);
            oracle.apply(&delta).unwrap_or_else(|refusal| {
                panic!(
                    "the graph refused a corpus delta the batch applied: {} (seed \
                     {seed:#018x} target {target})",
                    refusal.message()
                )
            });
        }
        assert_eq!(
            fingerprint(&oracle),
            new,
            "the frame applied through `Batch::commit` and the same frame applied delta by \
             delta left two different scenes, so the sweep's reference for `the new one` \
             is whatever the batch does rather than what the frame says. seed \
             {seed:#018x} target {target}"
        );

        if old == new {
            return None;
        }
        Some(Recorded {
            target,
            slots,
            len: slot_len,
            destroys,
            old,
            new,
            token: frame_token,
            prior,
            prior_len,
        })
    }

    /// Everything the sweep counts.
    ///
    /// The rows `claims/0025`'s table would hold if this sweep became a claim,
    /// in the same shape, and every one of them exists to stop the primary
    /// being the zero of a run that swept nothing.
    #[derive(Default)]
    struct Counted {
        /// Unit: count of cuts.
        cuts: u64,
        /// Unit: count of cuts leaving the old scene.
        old: u64,
        /// Unit: count of cuts leaving the new scene.
        new: u64,
        /// Unit: count of cuts leaving neither.
        third: u64,
        /// Unit: count of ring entries replayed.
        landed: u64,
        /// Torn payloads the decoder refused, at payload granularity.
        /// Unit: count of entries.
        torn_refused: u64,
        /// Slots whose write was not visible, so the batch was offered whatever
        /// the slot held before this frame reached it.
        /// Unit: count of entries.
        stale_offered: u64,
        /// Of those, the ones the decoder refused — which is every one of them
        /// whose slot the producer had never reached, and none of the rest.
        /// Unit: count of entries.
        stale_refused: u64,
        /// Of those, the ones the decoder *believed*: a previous frame's entry
        /// taken into this frame as one of its own. The case the zeroed model
        /// could not produce, and the one every structural claim in this module
        /// used to rest on being impossible.
        /// Unit: count of entries.
        stale_admitted: u64,
        /// Commits that sealed over a frame at least one of whose entries was
        /// not this frame's.
        /// Unit: count of commits.
        sealed_over_a_stale_entry: u64,
        /// Cuts inside a frame that destroys a subtree.
        /// Unit: count of cuts.
        inside_a_destroying_frame: u64,
        /// Commits that sealed and applied.
        /// Unit: count of commits.
        applied: u64,
        /// Frames the graph would not have, refused whole.
        /// Unit: count of commits.
        refused_whole: u64,
        /// Commits offered a decodable commit entry after this batch had
        /// already refused something in the same frame. The reachability of the
        /// rule below: an assertion that a poisoned batch never seals is worth
        /// nothing in a run where no poisoned batch was ever offered a commit.
        /// Unit: count of entries.
        a_commit_offered_after_a_refusal: u64,
        /// Seals produced by a batch that had already refused an entry in this
        /// frame. *A batch that refused anything can never produce a key*, as a
        /// number, and the number is required to be zero.
        /// Unit: count of commits.
        sealed_after_a_refusal: u64,
    }

    // There is no `diverged` counter, and the absence is deliberate. `one_cut`
    // panics on `Refused::Diverged` with the re-run key in the message, so a
    // counter beside it could only ever be read as zero and an
    // `assert_eq!(diverged, 0)` at the bottom of the sweep was three lines no
    // edit to this workspace could redden. Deleting it is the repair: the panic
    // is the observation, and a second one that cannot fire reads as a second
    // guard.

    /// Sweep one cut point, and answer which scene it left.
    ///
    /// The model, stated before it is used: **at a cut of `n` the consumer sees
    /// the first `n` slots of the commit and no more; every slot a completed
    /// barrier covers holds what the producer wrote, whole; any subset of the
    /// rest still holds what it held before this frame reached it; and the last
    /// slot past the barrier is torn at the chosen granularity, with the bytes
    /// past the tear being the ones that were already there.** The re-run key is
    /// `(seed, target, cut, granularity, mode)` and every draw derives from it.
    ///
    /// Two differences from `zone/tests/cut.rs`, both because a ring is not a
    /// device, and both stated rather than left for a reader to notice.
    ///
    /// *No order is drawn.* That model draws one because a device completes
    /// writes in an order it chooses; a consumer walking a ring does not choose
    /// — it reads slot zero, then slot one — so **which** slots are stale is a
    /// draw here and **in what order they are read** is not. Drawing an order
    /// that changed nothing would be a parameter pretending to be evidence.
    ///
    /// *A slot that did not land is stale, not absent.* A ring has fixed slots
    /// and the consumer reads whatever is in one; there is no way to skip a
    /// slot, and a model that skipped would be modelling a queue. *What* is in
    /// one is the point, and it is where this model was wrong before: an
    /// untouched slot holds the zeros of a fresh mapping, and `abi::scene::op`
    /// starts its opcodes at one precisely so that those name no opcode — but a
    /// slot the producer has already used once holds a **previous frame's
    /// entry**, which decodes. That is `zone/tests/cut.rs`'s own model, where an
    /// operation that does not land is simply not performed against a medium
    /// that already holds every earlier publish; here the medium is the ring,
    /// and `Recorded::prior` is what it held. Substituting zeros for it makes
    /// every structural claim in this module true for free, which is why the
    /// sweep that did so proved nothing about the mode it spends half its cuts
    /// in.
    ///
    /// *What a third scene would look like here*, since a sweep that cannot say
    /// is a sweep that is not looking: a fingerprint equal to neither `old` nor
    /// `new` — a node this frame created standing in the graph with the paint
    /// the frame never got to set, a subtree removed whose replacement was
    /// never hung, a re-hang applied against a sibling that had not moved yet.
    /// [`Applier::Eager`] produces all three on nearly every cut, and the sweep
    /// requires it to.
    fn one_cut(
        seed: u64,
        recorded: &Recorded,
        cut: usize,
        granularity: Granularity,
        mode: Mode,
        applier: Applier,
        counted: &mut Counted,
    ) -> Outcome {
        let mut arena = Arena::EMPTY;
        replay(seed, recorded.target, &mut arena);

        let mut batch = Batch::<LIMIT>::new();
        let mut state = key(seed, recorded.target, cut, granularity, mode);
        // Everything the barrier covers. In lying mode it covers nothing, which
        // is the whole of what that mode is.
        let covered = match mode {
            Mode::Honest => cut,
            Mode::Lying => 0,
        };
        let torn_at = cut.checked_sub(1);
        let mut stale_in_this_frame = false;
        // Whether this batch has already refused something in this frame.
        // `Batch::offer`'s poison early-return is what makes a refusal
        // permanent, and it is the rule the module's *or none of them* rests
        // on; without it a frame that lost an entry seals and applies. Measured:
        // with that early-return deleted, every assertion in this sweep still
        // passed, because honest mode refuses nothing and every lying-mode
        // number was a floor that the extra seals only pushed further up. This
        // flag is what the sweep needed in order to be looking.
        let mut refused_in_this_frame = false;

        for at in 0..cut {
            // A slot the barrier covers arrives whole, which is `zone/tests/
            // cut.rs`'s `land(op, None)` for everything before the last
            // completed flush: a barrier that promises visibility promises all
            // of it, so a torn payload is not something an honest ring can
            // show. Past the barrier, a drawn subset lands and the last of them
            // is torn at payload granularity.
            let covered_here = at < covered;
            let landed = covered_here || draw(&mut state).is_multiple_of(2);
            let torn = !covered_here
                && granularity == Granularity::Payload
                && Some(at) == torn_at
                && landed;
            let (was_entry, was_payload) = prior_bytes(recorded, at);
            let (entry, payload) = if landed {
                let (entry, mut payload) = recorded.slots[at].encode();
                if torn {
                    let keep = (draw(&mut state) % (PAYLOAD_BYTES as u64 + 1)) as usize;
                    // The bytes past the tear are the ones that were already
                    // there, not zeros. `land` writes a prefix of an
                    // operation's bytes and leaves the medium holding its own
                    // for the rest; a ring slot's arena bytes are the same
                    // medium.
                    for (byte, was) in payload.iter_mut().zip(was_payload).skip(keep) {
                        *byte = was;
                    }
                }
                (entry, payload)
            } else {
                counted.stale_offered += 1;
                (was_entry, was_payload)
            };
            counted.landed += 1;

            match applier {
                Applier::Staged => {
                    // Asked before the offer, because after it the batch has
                    // consumed the bytes and a poisoned batch answers
                    // `Poisoned` without saying what it was handed. A commit
                    // offered to a batch that has already refused an entry is
                    // exactly the state the poison rule exists to decide.
                    if refused_in_this_frame
                        && Delta::decode(&entry, &payload).is_ok_and(|d| d.frame().is_some())
                    {
                        counted.a_commit_offered_after_a_refusal += 1;
                    }
                    match batch.offer(&entry, &payload) {
                        Ok(Offered::Staged) => {
                            if !landed {
                                // A previous frame's entry, believed and taken into
                                // this frame as one of its own. Nothing in the
                                // delta format can tell *this slot's bytes belong
                                // to this frame* from *they belong to the last
                                // one*, which is the whole of what the zeroed model
                                // was hiding.
                                counted.stale_admitted += 1;
                                stale_in_this_frame = true;
                            }
                        }
                        Ok(Offered::Sealed(sealed)) => {
                            if refused_in_this_frame {
                                counted.sealed_after_a_refusal += 1;
                            }
                            if !landed {
                                counted.stale_admitted += 1;
                                stale_in_this_frame = true;
                            }
                            if stale_in_this_frame {
                                counted.sealed_over_a_stale_entry += 1;
                            }
                            let sealed_on = sealed.frame().token;
                            match batch.commit(&mut arena, sealed) {
                                Ok(closed) => {
                                    counted.applied += 1;
                                    assert_eq!(closed.frame.token, sealed_on);
                                    // Nothing was torn at this granularity and
                                    // every entry was this frame's, so the token
                                    // that closed the frame has to be the one the
                                    // client sent. At payload granularity it need
                                    // not be, and
                                    // `a_torn_commit_token_names_a_frame_nobody_sent`
                                    // is where that is said out loud; over a stale
                                    // commit it is the *previous* frame's token,
                                    // which is the same finding one slot wider.
                                    if granularity == Granularity::Whole && !stale_in_this_frame {
                                        assert_eq!(closed.frame.token, recorded.token);
                                    }
                                }
                                Err(Refused::Whole { .. }) => counted.refused_whole += 1,
                                Err(Refused::Diverged { at, refusal, applied }) => {
                                    panic!(
                                        "admission and the graph disagreed at delta {at} of \
                                         the commit ({}), with {applied} delta(s) already \
                                         applied. This is the one third scene this module \
                                         does not remove, and the module's `what is left \
                                         open` says where it is closed. reproduce: seed \
                                         {seed:#018x} target {} cut {cut} granularity {} \
                                         mode {} applier {}",
                                        refusal.message(),
                                        recorded.target,
                                        granularity.name(),
                                        mode.name(),
                                        applier.name()
                                    );
                                }
                            }
                        }
                        Err(Refusal::Wire(_)) => {
                            refused_in_this_frame = true;
                            if torn {
                                counted.torn_refused += 1;
                            } else if !landed {
                                counted.stale_refused += 1;
                            }
                        }
                        Err(_) => refused_in_this_frame = true,
                    }
                }
                // The control. No batch, no seal, no admission: a delta that
                // decodes goes straight into the graph, and the commit is the
                // no-op `arena::Applied::Closes` says it is.
                Applier::Eager => {
                    if let Ok(delta) = Delta::decode(&entry, &payload) {
                        let _ = arena.apply(&delta);
                    }
                }
            }
        }

        counted.cuts += 1;
        if recorded.destroys {
            counted.inside_a_destroying_frame += 1;
        }
        let found = fingerprint(&arena);
        let outcome = if found == recorded.new {
            Outcome::New
        } else if found == recorded.old {
            Outcome::Old
        } else {
            Outcome::Third
        };
        match outcome {
            Outcome::Old => counted.old += 1,
            Outcome::New => counted.new += 1,
            Outcome::Third => counted.third += 1,
        }
        outcome
    }

    /// `E3-B01d`'s exit, as the run it is accepted on — and the reach of it.
    ///
    /// The `E2-P01` cut model, pointed at a commit instead of a publish: every
    /// entry boundary of every commit, at both granularities, in both modes,
    /// across a seed sweep.
    ///
    /// # The sentence this run stands behind, and the one it does not
    ///
    /// **Over a ring whose `Release`/`Acquire` pair holds, the graph read back
    /// after a cut is the old scene or the new one, never a third.** That is
    /// `Mode::Honest`, it is asserted as `honest.third == 0`, and the
    /// `Applier::Eager` control over the same cuts is required to produce third
    /// scenes so that the zero means something.
    ///
    /// **Over a ring whose pair does not hold, it is not.** `Mode::Lying` is
    /// swept and the third scenes it produces are counted and *required to be
    /// there*, in the same way and for the same reason the eager control is:
    /// the number is what the ordering pair buys, and a run in which it were
    /// zero would be a run whose model had stopped being the faithful one.
    ///
    /// The exit this subtask is accepted on said *the graph read back is the
    /// old scene or the new one, never a third*, with no clause about the
    /// ring's ordering. That was broader than what is delivered, and the
    /// difference is the entire content of the paragraph below. It is no longer
    /// a disagreement between this module and the record: RFC 0084 rules that
    /// an exit is the sentence a task is accepted on, so narrowing one is a
    /// reversal, and `intent/0012-the-interface/spec.md`'s `E3-B01d` line now
    /// carries the clause with the RFC number beside it.
    ///
    /// # Why a lying ring produces one, and why nothing here can stop it
    ///
    /// A ring slot whose write is not visible does not read as zeros. It reads
    /// as whatever occupied it last — `ring/src/lib.rs` initialises its slots
    /// once and nothing zeroes one on completion — which is a previous frame's
    /// entry, and a previous frame's entry decodes. The batch takes it for one
    /// of this frame's, and the commit that follows either seals over a frame
    /// that is part this one and part the last one, or is refused whole by
    /// [`admit`]. The first of those is a third scene.
    ///
    /// Nothing in the delta format can tell *this slot's bytes belong to this
    /// frame* from *they belong to the last one*: there is no sequence number
    /// on an entry, and `f_abi::Sqe` has no field left that a scene delta does
    /// not already read or require to be zero. **Closing it is an ABI change
    /// and not a change to this file** — a per-entry submission sequence the
    /// consumer checks against the slot's own index, which is what makes a
    /// stale slot detectable rather than merely unlikely. Until then the
    /// ordering pair is load-bearing for a commit's atomicity, which is what
    /// this repository already says that pair is for, and this sweep is the
    /// measurement of it rather than an argument against it.
    ///
    /// # What the run requires to have seen
    ///
    /// `zone/tests/cut.rs` requires four observations of its own, in the same
    /// place, for the reason that file states: *a sweep that has never failed
    /// is indistinguishable from one that cannot.* These are this file's.
    #[test]
    fn a_commit_cut_at_every_entry_boundary_leaves_the_old_scene_or_the_new_one() {
        // Counted per mode rather than together, because the two modes are now
        // two different claims and a total would be neither of them.
        let mut honest = Counted::default();
        let mut lying = Counted::default();
        let mut control = Counted::default();
        let mut targets_swept = 0u64;

        for step in 0..SEEDS {
            // Derived rather than added, so that consecutive seeds are not
            // consecutive streams.
            let seed = if step == 0 { FIRST_SEED } else { site(&[FIRST_SEED, step]) };
            for target in 1..=FRAMES {
                let Some(recorded) = record(seed, target) else {
                    continue;
                };
                targets_swept += 1;
                for granularity in [Granularity::Whole, Granularity::Payload] {
                    for mode in [Mode::Honest, Mode::Lying] {
                        // Honest mode ignores the granularity axis, so sweeping
                        // it twice counts one observation as two. `covered` is
                        // `cut` there, the loop runs `0..cut`, so `covered_here`
                        // holds for every slot: nothing is drawn, nothing is
                        // stale and `torn` — the only thing granularity decides
                        // — is false at both settings. Measured before this
                        // line existed: whole-only and payload-only honest runs
                        // produced byte-identical counters, so 584 of 1168
                        // honest cuts were exact repeats of the other 584 and
                        // the cut count read as twice the evidence it was.
                        //
                        // Skipped rather than reported in a comment, because a
                        // number that overstates a run's breadth is the thing
                        // the assertions below are written against. This is not
                        // a narrowing: the set of distinct honest observations
                        // is unchanged.
                        if mode == Mode::Honest && granularity == Granularity::Payload {
                            continue;
                        }
                        let staged = match mode {
                            Mode::Honest => &mut honest,
                            Mode::Lying => &mut lying,
                        };
                        // Zero — nothing landed — through every entry. The
                        // upper end is the whole commit, which must leave the
                        // new scene and is the control on the lower end leaving
                        // the old one.
                        for cut in 0..=recorded.len {
                            let outcome = one_cut(
                                seed,
                                &recorded,
                                cut,
                                granularity,
                                mode,
                                Applier::Staged,
                                staged,
                            );
                            if mode == Mode::Honest {
                                assert_ne!(
                                    outcome,
                                    Outcome::Third,
                                    "a cut left a third scene over a ring that kept its \
                                     ordering. the commit was frame {} of {FRAMES}, the cut fell \
                                     after {cut} of {} entry(s), at {} granularity. reproduce: \
                                     seed {seed:#018x} target {} cut {cut}",
                                    recorded.target,
                                    recorded.len,
                                    granularity.name(),
                                    recorded.target,
                                );
                                // The whole commit, delivered by an honest ring
                                // with nothing torn, has to leave the new scene
                                // — or every `Old` above is a sweep of a module
                                // that never commits anything.
                                if cut == recorded.len {
                                    assert_eq!(
                                        outcome,
                                        Outcome::New,
                                        "a whole commit over an honest ring left the old scene: \
                                         seed {seed:#018x} target {}",
                                        recorded.target
                                    );
                                }
                            }
                            let _ = one_cut(
                                seed,
                                &recorded,
                                cut,
                                granularity,
                                mode,
                                Applier::Eager,
                                &mut control,
                            );
                        }
                    }
                }
            }
        }

        assert!(targets_swept > 0, "no corpus frame changed the scene, so nothing was swept");

        // Three assertions that stood here have been deleted rather than
        // rewritten, because none of them could be driven to red and an
        // assertion that cannot fail reads as a guard.
        //
        // `honest.landed + lying.landed == control.landed` was an identity:
        // `counted.landed` is incremented before the `match applier`, and the
        // eager control is invoked for exactly the (mode, granularity, cut)
        // triples the staged applier is, so the two sides were the same
        // additions written twice.
        //
        // `honest.diverged == 0` and `lying.diverged == 0` were unreachable:
        // `one_cut` panics on the `Refused::Diverged` arm, with the re-run key,
        // so the counter could only ever be observed as zero. The property is
        // checked — by the panic — and the counter is gone with the assertions.

        // The exit's own sentence, as a number, and the clause it holds under.
        assert_eq!(honest.third, 0, "the exit's own sentence, over a ring that kept its order");
        assert!(
            honest.old > 0 && honest.new > 0,
            "the sweep reached only one of the two scenes, so `the old one or the new one` had \
             one answer and the run could not tell them apart"
        );
        // Every cut that sealed left the new scene and every cut that did not
        // left the old one. Stated as an equality rather than as two floors,
        // because a run in which those numbers merely both exceeded zero would
        // be consistent with a commit that applied and left the old scene
        // anyway. Over an honest ring only: over a lying one a commit can seal
        // over a frame that is not the recorded one, and the scene that leaves
        // is neither of the two.
        assert_eq!(
            honest.new, honest.applied,
            "a commit sealed and the scene it left was not the new one"
        );
        // `honest.old == honest.cuts - honest.applied` stood here and is
        // deleted for the reason above: one increment per cut makes
        // `old + new + third == cuts`, and with `third == 0` and
        // `new == applied` both asserted already it followed arithmetically
        // from its two neighbours. It restated them; it could not disagree
        // with them.
        assert!(
            honest.applied > 0 && honest.refused_whole == 0,
            "the honest sweep committed {} frame(s) and had {} refused whole: a corpus frame \
             refused by admission over a ring that delivered it intact is this module \
             over-refusing a frame the graph would have taken",
            honest.applied,
            honest.refused_whole
        );

        // What the model actually produced, without which none of the numbers
        // above is about the model this exit names.
        assert!(
            lying.stale_offered > 0 && lying.stale_refused > 0,
            "no slot was ever left holding what it held before this frame, so the case that \
             decides whether a commit's atomicity rests on the ring's Release/Acquire pair was \
             never reached"
        );
        assert!(
            lying.stale_admitted > 0,
            "every one of the {} stale slots offered was refused by the decoder, which is the \
             answer the zeroed model gave for free: a slot holding a previous frame's entry \
             decodes, and a sweep that never produced one is sweeping the model that made this \
             exit true without trying",
            lying.stale_offered
        );
        assert!(
            lying.sealed_over_a_stale_entry > 0,
            "no commit ever sealed over a frame carrying an entry that was not its own, so the \
             forbidden state — a commit that sealed over an incomplete frame — was unreachable \
             in this sweep rather than shown absent by it"
        );
        // *A batch that refused anything can never produce a key.* The rule
        // `Batch::offer`'s poison early-return carries, and the one this
        // module's *or none of them* rests on — stated here as a number,
        // because until it was the sweep passed with that early-return
        // deleted and only a separate hand-built test went red. The floor
        // beside it is what stops the zero being a run in which no poisoned
        // batch was ever offered a commit.
        assert!(
            lying.a_commit_offered_after_a_refusal > 0,
            "no batch was ever handed a decodable commit after it had already refused an \
             entry in the same frame, so `a batch that refused anything can never produce \
             a key` was never asked of this sweep and the zero below means nothing"
        );
        assert_eq!(
            honest.sealed_after_a_refusal + lying.sealed_after_a_refusal,
            0,
            "a batch that had already refused an entry produced a seal, so a frame that lost \
             an entry can be closed — which is the whole of what `Batch::offer`'s poison \
             is for. {} of them were honest and {} were lying",
            honest.sealed_after_a_refusal,
            lying.sealed_after_a_refusal
        );
        assert!(
            lying.refused_whole > 0,
            "admission refused nothing across {} cuts, so `admit` and everything it walks is not \
             exercised by this run at all",
            lying.cuts
        );
        assert!(
            lying.torn_refused > 0,
            "no cut ever tore a payload the decoder refused, which at payload granularity over \
             {} cuts is not luck: either that granularity stopped being swept, or every record \
             became one a truncation still decodes — a change in what a torn entry means, and \
             not a change in this number",
            lying.cuts
        );
        assert!(
            honest.inside_a_destroying_frame > 0 && lying.inside_a_destroying_frame > 0,
            "no frame in the corpus destroyed a subtree, so the entry a cut is most expensive \
             inside was never cut. Widen the corpus; do not lower this"
        );
        // The measurement this sweep exists to make, and the second control. A
        // ring whose ordering pair promises nothing is a ring over which a
        // commit is not atomic: this is how often, and a zero here would mean
        // the model had gone back to the one where a slot that did not land is
        // zeros.
        assert!(
            lying.third > 0,
            "a ring that kept none of its ordering promises still never produced a third scene \
             across {} cuts, of which {} offered the batch a slot holding a previous frame's \
             entry. Either the model stopped being the faithful one, or something in this module \
             now distinguishes a stale slot from a fresh one — and if it is the second, this \
             assertion is the one to delete and the module's `why a lying ring produces one` is \
             the paragraph to rewrite",
            lying.cuts,
            lying.stale_offered
        );
        // The first control, and the whole reason the honest zero means
        // anything. A run in which the eager reading also never produced a
        // third scene is a run whose fingerprint cannot see one.
        assert!(
            control.third > 0,
            "the eager reading — `Arena::apply` per entry, the commit a no-op — never produced a \
             third scene across {} cuts. That is not the eager reading being correct; it is this \
             sweep being unable to observe the thing it exists to forbid",
            control.cuts
        );
    }

    /// A frame that lost an entry is not a shorter frame.
    ///
    /// The structural claim `Batch::offer` rests on, exercised where a reader
    /// would want to see it: a zeroed slot in the middle, and the commit that
    /// follows refused rather than obeyed.
    #[test]
    fn a_frame_that_lost_an_entry_can_never_seal() {
        // Not `mut`, and that is the assertion: nothing in this test can reach
        // the graph, because a poisoned batch never produces the `Sealed` that
        // `Batch::commit` would need a `&mut Arena` for.
        let arena = Arena::EMPTY;
        let mut batch = Batch::<LIMIT>::new();
        let before = fingerprint(&arena);

        let create = delta_of(Entry::CreateNode(CreateNode {
            node: ROOT,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind::LAYER,
        }));
        let (sqe, payload) = create.encode();
        assert!(matches!(batch.offer(&sqe, &payload), Ok(Offered::Staged)));

        // The slot whose payload never landed.
        assert!(matches!(batch.offer(&Sqe::ZERO, &[0u8; PAYLOAD_BYTES]), Err(Refusal::Wire(_))));
        assert!(batch.is_poisoned());

        let (sqe, payload) = delta_of(Entry::Commit(Commit { frame_token: 7 })).encode();
        assert!(matches!(batch.offer(&sqe, &payload), Err(Refusal::Poisoned(_))));
        assert_eq!(fingerprint(&arena), before, "a poisoned frame reached the graph");
        assert!(arena.is_empty());
    }

    /// The model never serves zeros for a slot the producer has reached.
    ///
    /// `prior_bytes`'s doc says the zeros of a fresh mapping appear *only*
    /// where the producer has genuinely never reached that slot. That sentence
    /// was false while `Recorded::prior_len` was the previous frame's length: a
    /// target longer than the frame before it fell back to zeros for the slots
    /// past that length, and a corpus whose frames vary in length reaches those
    /// slots all the time. Measured at 8 such slots across 48 targets, which
    /// one reviewer counted as 66 stale offers once cuts, granularities and
    /// modes are multiplied out — every one of them in the direction that makes
    /// this module's exit easier, because a zeroed slot is refused by the
    /// decoder and a decodable one is the case the exit is about.
    ///
    /// The high-water mark is accumulated here a second time, from the recorded
    /// frames rather than from `prior_len`, and that duplication is the guard:
    /// a `prior_len` that went back to meaning one frame's length would agree
    /// with itself and disagree with this.
    #[test]
    fn a_slot_an_earlier_frame_reached_is_never_served_as_zeros() {
        let mut targets = 0u64;
        let mut served_as_zeros = 0u64;
        let mut reached_but_zeroed = 0u64;
        for step in 0..SEEDS {
            let seed = if step == 0 { FIRST_SEED } else { site(&[FIRST_SEED, step]) };
            // How far into the ring the corpus had written before each target.
            let mut ever = [0usize; (FRAMES + 1) as usize];
            for target in 1..=FRAMES {
                let before = ever[(target - 1) as usize];
                let Some(recorded) = record(seed, target) else {
                    continue;
                };
                targets += 1;
                for at in 0..recorded.len {
                    if at >= recorded.prior_len {
                        served_as_zeros += 1;
                        if at < before {
                            reached_but_zeroed += 1;
                        }
                    }
                }
                ever[target as usize] = before.max(recorded.len);
            }
        }
        assert!(targets > 0, "no target was recorded, so nothing was checked");
        assert!(
            served_as_zeros > 0,
            "no slot was ever served the zeros of a fresh mapping, so the branch this test is \
             about was not taken and the zero below is not evidence"
        );
        assert_eq!(
            reached_but_zeroed, 0,
            "{reached_but_zeroed} of {served_as_zeros} slot(s) served as a fresh mapping's \
             zeros had already been written by an earlier frame of the same corpus, so \
             `prior_bytes` is substituting zeros where a real ring holds a decodable entry \
             and its doc says the opposite"
        );
    }

    /// A second commit in one frame closes nothing.
    ///
    /// [`Refusal::AlreadySealed`] was a rule with no test: deleting the
    /// `if self.sealed` guard from [`Batch::offer`] left every test in this
    /// module green. It is a cut-adjacent condition — a consumer that
    /// over-read the ring, a producer that lost its frame boundary — and it is
    /// one of the routes by which a batch could close a frame that is part this
    /// one and part the next, which is the class of event this whole file is
    /// about.
    #[test]
    fn a_second_commit_in_one_frame_is_refused_and_poisons_it() {
        let mut arena = Arena::EMPTY;
        let mut batch = Batch::<LIMIT>::new();
        let before = fingerprint(&arena);

        let create = delta_of(Entry::CreateNode(CreateNode {
            node: ROOT,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind::LAYER,
        }));
        let (sqe, payload) = create.encode();
        assert!(matches!(batch.offer(&sqe, &payload), Ok(Offered::Staged)));

        let (sqe, payload) = delta_of(Entry::Commit(Commit { frame_token: 5 })).encode();
        let first = match batch.offer(&sqe, &payload) {
            Ok(Offered::Sealed(sealed)) => sealed,
            other => panic!("the first commit did not seal the frame: {other:?}"),
        };

        // The second one, with a different token: the two frames this batch
        // would otherwise have to choose between.
        let (sqe, payload) = delta_of(Entry::Commit(Commit { frame_token: 6 })).encode();
        assert_eq!(batch.offer(&sqe, &payload).unwrap_err(), Refusal::AlreadySealed);
        assert!(batch.is_poisoned(), "a second commit was refused without poisoning the frame");

        // And the poison is the permanent kind: everything after it, including
        // a third commit, answers with the first refusal's message.
        let (sqe, payload) = create.encode();
        assert_eq!(
            batch.offer(&sqe, &payload).unwrap_err(),
            Refusal::Poisoned(Refusal::AlreadySealed.message())
        );

        assert_eq!(fingerprint(&arena), before, "a refused second commit reached the graph");

        // The key the *first* commit cut still opens the frame it was cut for.
        // That is the half worth saying: refusing the second commit is not the
        // same as throwing the frame away, and a rule that quietly did both
        // would lose a frame the client completed correctly.
        let closed = batch.commit(&mut arena, first).expect("the first frame was lost");
        assert_eq!(closed.frame.token, 5);
        assert!(arena.holds(ROOT));
    }

    /// The other half of the task line, observed: a frame the graph will not
    /// have leaves the scene exactly as it found it.
    #[test]
    fn a_frame_the_graph_will_not_have_changes_nothing() {
        let mut arena = Arena::EMPTY;
        deliver(
            &mut arena,
            &[Entry::CreateNode(CreateNode {
                node: ROOT,
                parent: NO_NODE,
                before: NO_NODE,
                kind: kind::LAYER,
            })],
            1,
        );
        let before = fingerprint(&arena);

        // A frame whose first two entries are perfectly good and whose third
        // names a node nobody created. Entry at a time, the first two would
        // land; here none of them does.
        let mut batch = Batch::<LIMIT>::new();
        let frame = [
            Entry::CreateNode(CreateNode {
                node: 2,
                parent: ROOT,
                before: NO_NODE,
                kind: kind::DRAW,
            }),
            Entry::CreateNode(CreateNode {
                node: 3,
                parent: ROOT,
                before: NO_NODE,
                kind: kind::DRAW,
            }),
            Entry::SetPaint(SetPaint {
                node: 9,
                red_x65535: 1,
                green_x65535: 2,
                blue_x65535: 3,
                alpha_x65535: 4,
                stroke_width_x65536: 0,
            }),
        ];
        let (slots, len) = slots_of(&frame, 11);
        let mut refused = None;
        for slot in &slots[..len] {
            let (sqe, payload) = slot.encode();
            match batch.offer(&sqe, &payload) {
                Ok(Offered::Staged) => {}
                Ok(Offered::Sealed(sealed)) => refused = batch.commit(&mut arena, sealed).err(),
                Err(refusal) => panic!("the frame was refused at the wire: {}", refusal.message()),
            }
        }
        assert_eq!(
            refused,
            Some(Refused::Whole { at: 2, refusal: Refusal::Graph(GraphRefusal::NoSuchNode(9)) })
        );
        assert_eq!(fingerprint(&arena), before, "a refused frame left something behind");
        assert!(!arena.holds(2) && !arena.holds(3));
    }

    /// The sections are a rule and not a habit.
    #[test]
    fn an_entry_out_of_section_poisons_the_frame() {
        let mut batch = Batch::<LIMIT>::new();
        let (sqe, payload) = delta_of(Entry::SetPaint(SetPaint {
            node: ROOT,
            red_x65535: 0,
            green_x65535: 0,
            blue_x65535: 0,
            alpha_x65535: 1,
            stroke_width_x65536: 0,
        }))
        .encode();
        assert!(matches!(batch.offer(&sqe, &payload), Ok(Offered::Staged)));

        let (sqe, payload) = delta_of(Entry::RemoveNode(RemoveNode { node: ROOT })).encode();
        assert_eq!(batch.offer(&sqe, &payload).err(), Some(Refusal::OutOfPhase));
        assert!(batch.is_poisoned());
    }

    /// A key opens the frame it was cut for, and that frame only.
    ///
    /// The half of *a frame is applied at most once* a moved value cannot state
    /// on its own. `Sealed` being neither `Clone` nor `Copy` is what stops a
    /// second key existing; this is what stops a key outliving the frame it
    /// named — and the entries staged for the frame that came after it are
    /// still there, untouched, when the stale key is turned away.
    #[test]
    fn a_seal_opens_the_frame_it_was_cut_for_and_no_other() {
        let mut arena = Arena::EMPTY;
        let mut batch = Batch::<LIMIT>::new();
        let (sqe, payload) = delta_of(Entry::Commit(Commit { frame_token: 5 })).encode();

        let Ok(Offered::Sealed(stale)) = batch.offer(&sqe, &payload) else {
            panic!("a commit did not seal");
        };
        batch.reset();

        // A frame is under way again, and the key from the discarded one
        // arrives late.
        let create = delta_of(Entry::CreateNode(CreateNode {
            node: ROOT,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind::LAYER,
        }));
        let (create_sqe, create_payload) = create.encode();
        assert!(matches!(batch.offer(&create_sqe, &create_payload), Ok(Offered::Staged)));
        assert_eq!(
            batch.commit(&mut arena, stale).err(),
            Some(Refused::Whole { at: 0, refusal: Refusal::StaleSeal })
        );
        assert!(arena.is_empty(), "a stale key opened a frame");
        assert_eq!(batch.len(), 1, "a stale key threw away the frame under way");

        // And the key this frame cuts for itself is the one that works.
        let Ok(Offered::Sealed(sealed)) = batch.offer(&sqe, &payload) else {
            panic!("a commit did not seal");
        };
        assert!(batch.commit(&mut arena, sealed).is_ok());
        assert!(arena.holds(ROOT));
        assert!(batch.is_empty(), "a committed batch is a fresh frame");
    }

    /// A cut inside the commit's own payload can seal a frame nobody sent.
    ///
    /// The one thing a cut can change that the graph cannot see, written as a
    /// test rather than left in the sweep's counters, because it is a finding
    /// and not a property: the scene that comes out is the new one — the exit
    /// holds — and the *token* that closed it is a truncation of the client's.
    ///
    /// Nothing in this format can refuse it, and that is the point of saying
    /// so. A truncated `u64` is a legal `u64`; the commit record's only
    /// refusable value is zero, and `Reader::finish` has nothing to complain
    /// about because the bytes a tear leaves behind are the zeros it requires.
    /// `E3-B01j` counts one frame's boundary crossings on each side of the seam
    /// and matches them by this token, so that is the sentence which breaks
    /// first — and what fixes it is the ring's `Release` store making the
    /// payload visible before the entry, not a field added here.
    #[test]
    fn a_torn_commit_token_names_a_frame_nobody_sent() {
        let mut arena = Arena::EMPTY;
        let mut batch = Batch::<LIMIT>::new();

        let create = delta_of(Entry::CreateNode(CreateNode {
            node: ROOT,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind::LAYER,
        }));
        let (sqe, payload) = create.encode();
        assert!(matches!(batch.offer(&sqe, &payload), Ok(Offered::Staged)));

        // The commit's entry is visible and three bytes of its payload are
        // there. The rest is the zeros of an untouched mapping.
        let sent = 0x1122_3344_5566_7788u64;
        let (sqe, mut payload) = delta_of(Entry::Commit(Commit { frame_token: sent })).encode();
        for byte in payload.iter_mut().skip(3) {
            *byte = 0;
        }
        let Ok(Offered::Sealed(sealed)) = batch.offer(&sqe, &payload) else {
            panic!("a torn commit did not seal, which would make this test about nothing");
        };
        assert_eq!(sealed.frame().token, 0x0066_7788, "the low three bytes, and nothing else");
        assert_ne!(sealed.frame().token, sent);

        // And the scene is the new one. The exit is about the graph, and the
        // graph is right.
        let closed = batch.commit(&mut arena, sealed).unwrap_or_else(|refused| {
            panic!("a torn token refused a frame the graph would have taken: {refused:?}")
        });
        assert_eq!(closed.created, 1);
        assert!(arena.holds(ROOT));
        assert_ne!(closed.frame.token, sent, "the frame this commit closed is not the one sent");
    }

    /// Room is counted against what the frame itself gives back.
    ///
    /// What `freed_by` is for: without it, a frame replacing a full scene with
    /// another full one would be refused by an arithmetic that assumed the
    /// removals freed nothing.
    #[test]
    fn room_is_counted_against_what_the_frame_gives_back() {
        let mut arena = Arena::EMPTY;
        deliver(
            &mut arena,
            &[
                Entry::CreateNode(CreateNode {
                    node: ROOT,
                    parent: NO_NODE,
                    before: NO_NODE,
                    kind: kind::LAYER,
                }),
                Entry::CreateNode(CreateNode {
                    node: 2,
                    parent: ROOT,
                    before: NO_NODE,
                    kind: kind::DRAW,
                }),
                Entry::CreateNode(CreateNode {
                    node: 3,
                    parent: 2,
                    before: NO_NODE,
                    kind: kind::DRAW,
                }),
            ],
            1,
        );
        assert_eq!(arena.live(), 3);

        // Removing node 2 frees two slots — itself and the child under it —
        // which `freed_by` has to find by walking rather than by counting the
        // removals.
        assert_eq!(freed_by(&arena, 2), 2);
        let closed = deliver(
            &mut arena,
            &[
                Entry::RemoveNode(RemoveNode { node: 2 }),
                Entry::CreateNode(CreateNode {
                    node: 4,
                    parent: ROOT,
                    before: NO_NODE,
                    kind: kind::DRAW,
                }),
                Entry::CreateNode(CreateNode {
                    node: 5,
                    parent: 4,
                    before: NO_NODE,
                    kind: kind::DRAW,
                }),
            ],
            2,
        );
        assert_eq!(closed.removed, 2);
        assert_eq!(closed.created, 2);
        assert_eq!(arena.live(), 3);
    }
}
