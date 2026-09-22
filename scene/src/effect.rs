// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What an effect node costs, and what is done instead when the frame cannot
//! afford it — one declaration, and never two fields that can arrive apart.
//!
//! `abi::scene::kind::EFFECT` is *blur, drop shadow or a material*, and
//! [`kind::Kind::Effect`](crate::kind::Kind::Effect) is the same thing on the
//! near side of the decoder. This module adds the two numbers section 10's
//! degradation policy needs from such a node before a frame is late: what the
//! declarer expects the effect to cost, and what choosing something cheaper
//! gives back. Neither is any use without the other, so neither is a field of
//! its own here.
//!
//! # The half declaration is the bug
//!
//! An effect that declared a cost and no fallback is worse than an effect that
//! declared nothing. The policy walking a late frame asks each node what
//! degrading it would buy; a node with a cost and no fallback answers *nothing*,
//! and the policy's remaining moves are to render it and miss the deadline, or
//! to drop it anyway — which is a change to the picture that nobody declared and
//! that `E3-B07d` would have to record as a decision the compositor made on its
//! own. Both of those happen at frame time, milliseconds from scanout, in the
//! one place in this system where there is no time to be told anything.
//!
//! So the half declaration is refused where it is cheap to refuse: at the door
//! between the words a producer wrote and the value this crate will act on.
//! Two things are delivered there, and it is worth being exact about which,
//! because the obvious summary of them is larger than either.
//!
//! - **A declaration that names one word and not the other is refused**, by
//!   [`Effect::declared`], which is the only constructor of an [`Effect`].
//! - **No [`Effect`] holds half a declaration**, because the type has no shape
//!   for one: both fields are `NonZeroU32`, neither is an `Option`, and there
//!   is no other constructor.
//!
//! **What is not delivered, stated here so that nobody has to find it.** An
//! effect node that declares *nothing at all* is not refused by anything, in
//! this module or anywhere else in this workspace. Nothing requires a
//! `Kind::Effect` node to produce an [`Effect`]: `crate::arena` stores the node,
//! `crate::commit` applies the frame, and neither has any notion of a
//! declaration. Such a node reaches the degradation policy as a node absent
//! from the policy's table — which is the frame-time surprise this file opened
//! by describing, arriving by the one route this file does not close.
//!
//! Closing it is two changes and neither is here. It needs a record on the wire
//! for a declaration to arrive in — see *where the boundary is* below — and it
//! needs a census on the side that holds the nodes: a count of `Kind::Effect`
//! creations against declarations, refused at the commit, which is
//! `crate::arena`'s and `crate::commit`'s to keep and not this type's.
//! [`kind::ByKind`](crate::kind::ByKind) is already the table for the first
//! half of it.
//!
//! # Two numbers, and why the second is a saving
//!
//! The obvious encoding is a cost and a second cost — what the effect takes and
//! what the fallback takes — and it has a hole a producer falls straight into.
//! An unwritten field is zero. A fallback cost of zero is also entirely
//! meaningful: the cheapest thing that can be done instead of an effect is not
//! applying it, and that costs nothing. So *estimate 900, fallback 0* is both
//! *this effect skips when it must* and *somebody forgot the second word*, and
//! no decoder can tell those apart. Every repair for that ambiguity is a third
//! word whose only job is to say whether the second one was written — and two
//! words that must agree about one fact is the exact shape of the defect this
//! file exists to remove.
//!
//! A saving has no such hole, because zero is already an answer with a reason:
//! a fallback that saves nothing is not a fallback, and is refused under its own
//! name for its own cause. The free fallback — the effect not applied at all —
//! is `saving == estimate`, which is a value in the middle of the range rather
//! than at the sentinel end of it, and the arithmetic that recovers the
//! fallback's own cost is a subtraction the boundary has already made safe.
//!
//! It is also the number the policy actually wants. A late frame does not ask
//! what a fallback costs; it asks what choosing one buys, and sorts by the
//! answer. [`Effect::saving_us_x100`] hands that back as a `NonZeroU32`, and it
//! can only be that type because the refusal below exists.
//!
//! # Cheaper is a claim. This is the half of it that is checked
//!
//! Checked, at the door: that the declaration is not self-contradictory. The
//! saving is at least one unit, so the fallback is strictly cheaper than the
//! effect; and at most the estimate, so it is not cheaper than free. Past those
//! two refusals the fallback's cost cannot underflow and needs no `Option`
//! anywhere, which is the practical dividend of checking at a boundary rather
//! than at each use.
//!
//! Not checked, here or anywhere else in this workspace today: that either
//! number resembles what the renderer will actually take. Both come from the
//! same declarer in the same breath, and a producer that halves both of them is
//! believed. Nothing in this module measures anything — it holds no clock and
//! is not going to grow one. A declaration that is coherent and false produces a
//! frame-time surprise of precisely the kind the exit names, and the instrument
//! that catches it is the running estimate against the remaining budget, which
//! is `E3-B07c` and reads the compositor's own rolling p99 through `Env`.
//! Writing *cheaper is enforced* here would be a lie in the shape of a check.
//! What is enforced is that a declarer cannot name a fallback while saying it
//! saves nothing, and cannot name one that gives back more than the effect ever
//! cost.
//!
//! # What makes an estimate honest
//!
//! It is an estimate the declarer supplies, not a measurement anybody took, so
//! the standard has to be stated rather than assumed:
//!
//! - **It is an upper bound the declarer would be held to**, for this node's
//!   subtree at the size it is being submitted at. A median is the wrong
//!   statistic for a deadline, for `claims/`' reason — a frame is late when the
//!   slow case happens, not when the average does.
//! - **It does not move with the machine's load.** It is a property of the work
//!   and not of the moment. An estimate that already contained the overload
//!   would be double-counted by a policy whose whole job is to subtract the
//!   overload, and the compositor would degrade twice for one late frame.
//! - **It is marginal and per node**: what this effect adds over rendering the
//!   same subtree without it. That is exactly the quantity skipping it gives
//!   back, and any other reading makes the saving below mean something else.
//!
//! When it is wrong it is wrong in one of two directions and neither is visible
//! from here. Too low: the policy under-degrades, the frame misses anyway, and
//! the node looks innocent in whatever record is kept of the choice. Too high:
//! the policy degrades a node that would have fitted, and the picture is worse
//! than the machine required — the failure nobody reports, because nothing
//! missed. Both are frame-time consequences, both belong to `E3-B07c` and
//! `E3-B07d`, and this module's contribution to them is that the numbers they
//! compare against exist at all and arrived together.
//!
//! # One step, and where a ladder would go
//!
//! One fallback per node. A chain of them would need every rung to declare its
//! own fallback, and nothing terminates that recursion except a rule that the
//! last rung must be free — which `saving == estimate` already says in a single
//! step. *What would reverse this:* a measured corpus in which one step is too
//! coarse, an effect whose only cheaper form is still unaffordable, at which
//! point the honest change is a rung list with a terminator rather than a second
//! optional field beside these two.
//!
//! # What this deliberately does not say
//!
//! What the effect *is*. No blur, no shadow, no material, and no parameters. The
//! degradation policy needs two numbers and the node they belong to; anything
//! else it thinks it knows about how a blur is made cheaper is a second copy of
//! the renderer's knowledge living in a crate that cannot render. [`Instead`] is
//! two variants for that reason — *not applied* and *applied at whatever lesser
//! fidelity the renderer has* — and the second one carries no recipe.
//!
//! *What would reverse this:* `E3-B07b`'s fixed order turning out to need a rank
//! per effect shape rather than per declared saving. Then the shape is a field
//! the wire record carries and this type grows one, which is a diff to
//! `abi/src/scene.rs` reviewed as an ABI change — not a convention a renderer
//! and a policy are asked to share.
//!
//! # Where the boundary is, and what the wire does not carry yet
//!
//! *An `Effect` delta* means a `CreateNode` whose kind is `kind::EFFECT`,
//! together with the words that declare it. The first half is already a type:
//! [`kind::Created`](crate::kind::Created) has no public constructor and
//! [`kind::Change::of`](crate::kind::Change::of) is the only function that
//! returns one, so a reference to a [`Created`] *is* the evidence that a delta
//! arrived, and [`Effect::declared`] asks for one rather than for a node number
//! a caller found somewhere.
//!
//! The second half has no record to arrive in today: `abi/src/scene.rs` has six
//! opcodes and none of them carries an effect's parameters. **So there is no
//! delta in this workspace that can carry an estimate without a saving, and
//! nothing is refused at the wire boundary — because nothing can knock on it.**
//! [`Declared`] is a plain in-process struct that a caller inside this crate
//! hands over, and [`Effect::declared`] is the boundary that exists: the one
//! between two integers somebody wrote and the value this crate will act on.
//! Every sentence in this file is about that boundary and none of them is about
//! a decoder.
//!
//! That is a narrower claim than *a delta carrying one and not the other is
//! refused*, and the difference is not a formality: a peer could not produce a
//! half declaration if it tried, and the day it can, the refusal will be a
//! decoder's rather than this function's. It is also no longer a narrowing that
//! lives only here. RFC 0084 rules that an exit is the sentence a task is
//! accepted on, so narrowing one is a reversal and belongs in the record:
//! `intent/0012-the-interface/spec.md`'s `E3-B07a` line now carries the
//! narrower sentence, both of the measurements it rests on, and the RFC number
//! beside it. A module paragraph that disagreed with the spec would be a false
//! line waiting for a paste into `TODO.md`, which is exactly what that RFC was
//! written about. What landing the record would take is
//! a seventh opcode in `abi/src/scene.rs` — `SET_EFFECT`, carrying a node, an
//! estimate and a saving — which is an ABI change with an RFC behind it, and it
//! stops the build of every consumer that decides per opcode: `section_of` and
//! `admit` in `crate::commit`, `REACH` in `crate::dirty`, `Change::of` in
//! `crate::kind`, and `Arena::apply`. That list is the cost of the change and
//! the reason it is one diff and not this one.
//!
//! Two limits remain either way, stated in the same place `kind.rs` states its
//! own. Nothing in this workspace can force a future decoder to call this
//! function. And nothing requires an effect node to be declared at all — the
//! module's *what is not delivered* above is the whole of that one. What this
//! file does deliver is that there is no *other* route from two words to an
//! [`Effect`], and no way to hold half of one afterwards.
//!
//! Both of those are claims about what this file **does not contain**, which no
//! call can observe: a second constructor is invisible to a test that calls the
//! first one, and a cost field narrowed to `u32` behind an accessor that still
//! returns `NonZeroU32` changes no behaviour `Effect::declared` can produce.
//! They were true by reading and defended by nothing for three rounds. What
//! defends them now is
//! `tests::the_only_route_from_two_words_to_an_effect_is_still_declared`, which
//! reads this file as text and asks it those two questions, and the `const _`
//! size assertion beside the struct, which asks the compiler the cheaper half
//! of the second one. Both are named here because a reader who deletes one
//! should know which sentence above goes with it.
//!
//! # No clock, no randomness, no binary floating point, no allocator
//!
//! Every function here is a pure function of its arguments. This crate takes no
//! `f_env::Env` and would have nothing to do with one: an answer that depends
//! only on the argument is more deterministic than one that draws, and is
//! reproducible from the argument alone. Costs are integers with their scale in
//! their names, per RFC 0004, because the two architectures do not agree on
//! binary fractions and a frame that degraded differently on one of them would
//! be this whole thesis failing quietly. No allocation and no collection: a
//! declaration is two words wide.

use core::num::NonZeroU32;

use f_abi::scene::Refusal;

use crate::kind::{ByKind, Created, Kind};

/// Which kinds may carry a declaration, one row per kind.
///
/// A [`ByKind`] written as a literal rather than a `match` on
/// [`Created::kind`], and the choice is `crate::kind`'s own argument rather
/// than a taste: an exhaustive match demands an arm and not an arm that *says*
/// anything, so `Kind::Volume => false` compiles and is exactly how a seventh
/// kind comes to be classified by whoever was in a hurry. A table has no empty
/// row to write — this literal is six entries long, [`ByKind`] holds
/// `[T; Kind::COUNT]`, and the day there are seven kinds this line is the wrong
/// length and **this crate stops compiling**, in a consumer of `Kind` rather
/// than only in the file that declares them.
///
/// The rows are positional, which is the one thing a table cannot state about
/// itself: the length catches a kind *added*, and nothing in the length catches
/// the six being **reordered** in the `kinds!` list, which would slide the
/// permission onto a neighbour with every row still present and the array still
/// six long. So the assertion below pins the `true` to [`Kind::Effect`] by name
/// and requires it to be the only one — `crate::dirty`'s `REACH` pins its rows
/// to opcodes for the same reason and in the same shape.
///
/// *What would reverse this:* a second kind that has something cheaper to do
/// instead — a `Layer` whose intermediate target could be skipped is the
/// candidate — at which point this row becomes `true` and every sentence in
/// this module about *an effect node* becomes a sentence about two kinds.
/// Unit: none — one permission per kind, in `Kind::ALL`'s order.
const MAY_DECLARE: ByKind<bool> = ByKind::new([
    false, // Transform: a matrix has no cheaper form; it is applied or it is not.
    false, // Clip: the same, and skipping one paints outside the clip.
    false, // Layer: an intermediate target the renderer may already elide.
    false, // Draw: the marks themselves. Degrading these is not this policy's.
    true,  // Effect: blur, drop shadow, material — the parameterised kinds.
    false, // Semantic: contributes nothing to the picture, so costs nothing to skip.
]);

// The table's `true` belongs to the kind this module is about, and to one kind.
// Checked here rather than in a test because a row that had shifted is not a
// behaviour to observe, it is a build that should not link — `crate::kind`'s
// own two const blocks are written for the same reason.
const _: () = {
    assert!(*MAY_DECLARE.at(Kind::Effect), "the effect kind may not declare an effect");
    let rows = MAY_DECLARE.as_array();
    let mut at = 0;
    let mut allowed = 0;
    while at < Kind::COUNT {
        if rows[at] {
            allowed += 1;
        }
        at += 1;
    }
    assert!(allowed == 1, "a kind other than Effect may declare an effect");
};

/// An effect's two words, exactly as a producer wrote them.
///
/// The unbelieved form, and its fields are public because this is what arrives
/// rather than what this crate acts on: a half-written declaration has to be
/// *expressible* in order to be refused, which is `abi::scene::Delta::encode`'s
/// argument for being total — a malformed entry that no type can hold is a
/// malformed entry no test can build.
///
/// Everything past [`Effect::declared`] is the other kind of value. There is no
/// constructor of an [`Effect`] taking one of these numbers without the other,
/// no `Default` that would supply the missing one, and no accessor handing back
/// an `Option`, so *one and not the other* is a thing this module can be handed
/// and never a thing it can be holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declared {
    /// What the declarer expects this effect to add to the frame.
    ///
    /// Marginal over rendering the same subtree without the effect, and an
    /// upper bound rather than a typical case — the module's *what makes an
    /// estimate honest* is the whole of what this number promises.
    /// Unit: microseconds, scaled by 100, so one unit is ten nanoseconds — below
    /// anything a declarer can honestly distinguish, which is the point: a
    /// hundred cheap effects must not each round to zero. Zero is not an
    /// estimate. It is the value an unwritten word has, and it is refused.
    pub estimate_us_x100: u32,
    /// What choosing the fallback instead is expected to give back.
    ///
    /// A saving and not a second cost, for the reason the module argues at
    /// length: zero has to mean *nobody declared one* here, and it cannot mean
    /// that if the field is a cost, because the commonest fallback there is
    /// costs nothing.
    /// Unit: microseconds, scaled by 100 — the same scale as
    /// [`Declared::estimate_us_x100`], because the two are subtracted. Zero is a
    /// fallback that saves nothing, which is not a fallback and is refused.
    /// Equal to the estimate is the fallback that is free, which is the effect
    /// not being applied at all.
    pub saving_us_x100: u32,
}

/// The five refusals, written once.
///
/// # Why a macro, in a file that would rather not have one
///
/// Because the alternative is two sequences that have to agree — the variants,
/// and the list something walks to check that their sentences are distinct —
/// and this repository has twice watched a variant be added to an enum and left
/// out of an array every loop iterated. `docs/postmortem/0001` is one, and
/// `interface/src/node.rs`'s `vocabulary!` was written after the other.
///
/// This file had exactly that array, five entries long, hand-written inside a
/// test, one file away from the [`Fallback`] type it congratulates itself for
/// removing a second copy from. A sixth variant is forced by the exhaustive
/// `message` match below to have *a* sentence; it was not forced to have a
/// *distinct* one, because the list that checks distinctness did not know it
/// existed, so a copy-pasted sentence would have shipped green. One list closes
/// it: [`Undeclared::ALL`] is emitted from the same lines as the variants, and
/// there is no way to write a variant it does not get.
macro_rules! refusals {
    (
        $(
            $(#[$about:meta])*
            $variant:ident, $sentence:literal, $half:literal;
        )*
    ) => {
        /// Why a declaration was not believed.
        ///
        /// A local enum rather than `abi::scene::Refusal`, and the difference is
        /// worth a sentence: `kind.rs` returns that one because every refusal it
        /// makes is a refusal the wire decoder would also have made, and two
        /// statements of one rule is how two halves of a system come to
        /// disagree. None of these is such a rule. `abi::scene` has no effect
        /// record and therefore no opinion about these two words, so returning
        /// its type would claim an agreement that does not exist yet.
        ///
        /// What does not vary is what a peer is told: [`Undeclared::REFUSAL`] is
        /// one code for all of them, which is `abi::scene::Refusal::packed`'s own
        /// argument — the distinctions below are for the producer reading its own
        /// log, and the domain is the part that is stable on the wire.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Undeclared {
            $($(#[$about])* $variant,)*
        }

        impl Undeclared {
            /// How many ways a declaration can be refused.
            ///
            /// Counted from the list rather than written down, so it cannot
            /// disagree with what it counts.
            /// Unit: refusals.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every refusal, in declaration order.
            ///
            /// Emitted from the same lines as the variants, so it holds every
            /// refusal there is — not because a test checks it, but because
            /// there is no way to write one this array does not get. That is
            /// what makes `a_refusal_says_which_half_was_missing` a statement
            /// about the enum rather than about whatever somebody remembered to
            /// type into it.
            /// Unit: none — refusals.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// A line for a log.
            ///
            /// Written per variant rather than derived from the name, because
            /// the name says which case this is and the sentence says what the
            /// producer did wrong, and a producer reading a boot log has the
            /// second question.
            #[must_use]
            pub const fn message(self) -> &'static str {
                match self {
                    $(Self::$variant => $sentence,)*
                }
            }

            /// The words in this variant's sentence that say which case it is.
            ///
            /// Written per variant, on the same line as the sentence, because
            /// the clause the exit carries — *the refusal names which half was
            /// missing* — is a claim about the English a producer reads, and
            /// nothing about an exhaustive `match` forces a sentence to be
            /// about its own arm. That gap was real and it was open: swapping
            /// [`Undeclared::NoEstimate`]'s sentence with
            /// [`Undeclared::NoFallback`]'s left this crate green, because
            /// *non-empty* and *pairwise distinct* — all the suite asked of a
            /// sentence — are both true of a pair of lies about each other.
            ///
            /// This fragment is what
            /// `a_refusal_says_which_half_was_missing` requires to appear in
            /// this variant's own sentence **and in no other's**, which is the
            /// correspondence the variant name has and the sentence did not.
            /// A sixth refusal cannot be written without one, for the same
            /// reason it cannot be written without a sentence: the macro asks
            /// for it on the line that declares the variant.
            ///
            /// *The reversal condition:* a refusal whose whole sentence is the
            /// identification, with no fragment unique to it. Then this is a
            /// duplicate of `message` and the test should compare whole
            /// sentences instead.
            /// Unit: none — a fragment of the sentence beside it.
            #[must_use]
            pub const fn names(self) -> &'static str {
                match self {
                    $(Self::$variant => $half,)*
                }
            }
        }
    };
}

refusals! {
    /// The node is not an effect node.
    ///
    /// A cost and a fallback attached to a draw or a clip is a declaration about
    /// something that has no cheaper form to fall back to, and a policy that
    /// accepted it would be degrading nodes whose kind never said it could be
    /// degraded.
    NotAnEffect, "a declaration was offered for a node that is not an effect", "not an effect";

    /// Neither word was written.
    ///
    /// Kept distinct from the two halves below because a producer that wrote
    /// nothing is a different defect from one that wrote half: the first forgot
    /// the record, the second forgot a field in it, and they are fixed in
    /// different places.
    Neither, "an effect node declared neither a cost nor a fallback",
        "neither a cost nor a fallback";

    /// A saving, and nothing for it to be a saving from.
    ///
    /// The mirror of [`Undeclared::NoFallback`], and refused just as hard. A
    /// fallback whose effect has no declared cost cannot be ranked against
    /// anything, so accepting it would put a node in the policy's table that the
    /// policy can never choose.
    NoEstimate, "an effect node declared a saving and no cost to save from", "no cost to save from";

    /// An estimate, and nothing cheaper to do instead.
    ///
    /// The case the exit is written about: an effect that has told the
    /// compositor what it will cost and not what to do when that is too much.
    NoFallback, "an effect node declared a cost and nothing cheaper to do instead",
        "nothing cheaper to do instead";

    /// A fallback that gives back more than the effect ever cost.
    ///
    /// Nothing is cheaper than free, so a saving above the estimate is not a
    /// generous declaration, it is an incoherent one — and a policy that
    /// subtracted it from a frame's remaining budget would believe it had
    /// recovered time that never existed.
    CheaperThanFree, "a fallback saves more than the effect it replaces costs", "saves more";
}

impl Undeclared {
    /// What a peer is told, for any of these.
    ///
    /// One code and five values, which is the shape `abi::scene::Refusal::packed`
    /// already argues for: the caller distinguishes these where it can act on
    /// the difference, and the wire carries the domain, which is the part RFC
    /// 0010 makes stable. `Refusal::Value` is the code, because every one of
    /// these is a closed field carrying something outside its set — a node kind
    /// that may not declare, or a word that may not be zero.
    /// Unit: none — a refusal, not a quantity.
    pub const REFUSAL: Refusal = Refusal::Value;
}

/// What a renderer does with an effect the frame cannot afford.
///
/// Two answers, and the second one carries no recipe. *How* a blur is made
/// cheaper — fewer taps, a smaller kernel, a downsampled target — is the
/// renderer's knowledge, and a vocabulary of it here would be a second copy of
/// that knowledge in a crate with no renderer in it. What the policy needs to
/// distinguish is whether the picture still contains the effect, because those
/// are different things to have done to a frame and `E3-B07d` records which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instead {
    /// The effect is not applied. The subtree renders as though the node were
    /// not there, which is the cheapest thing that can happen to it and the
    /// most visible.
    Skip,
    /// The effect is applied at whatever lesser fidelity the renderer has for
    /// it, costing what the declaration said the reduced form costs.
    Reduced,
}

/// What is done instead, and what that is expected to cost.
///
/// Derived rather than declared, and that is the point: [`Instead`] is read off
/// the cost rather than carried beside it, so a *skip* that claims to cost
/// something and a *reduced* that claims to be free are not values this type has
/// — they are not refused, they are absent. Two fields that must agree about one
/// fact is the defect the module documentation opens with, and it would be a
/// poor file that removed it from the wire and reintroduced it here.
///
/// There is no public constructor. The only source of one is
/// [`Effect::fallback`], so a fallback is always a fallback *of* something, with
/// the arithmetic between them already checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fallback {
    /// What the cheaper rendering is expected to cost.
    ///
    /// Unit: microseconds, scaled by 100 — [`Declared::estimate_us_x100`]'s
    /// scale. Zero is the effect not being applied at all, and is the one place
    /// in this module where zero is a quantity rather than an omission: it got
    /// here by subtraction from two words that were both written, never by a
    /// word that was not.
    cost_us_x100: u32,
}

impl Fallback {
    /// What the cheaper rendering is expected to cost.
    /// Unit: microseconds, scaled by 100. Always strictly below the estimate it
    /// is a fallback for, which is a fact about how this value is built rather
    /// than a range a caller has to re-check.
    #[must_use]
    pub const fn cost_us_x100(self) -> u32 {
        self.cost_us_x100
    }

    /// What the renderer does.
    ///
    /// A cost of nothing is the effect not being applied; anything else is the
    /// effect applied at less fidelity. Nobody declares this separately, so
    /// nobody can declare it wrongly.
    #[must_use]
    pub const fn instead(self) -> Instead {
        if self.cost_us_x100 == 0 { Instead::Skip } else { Instead::Reduced }
    }
}

/// An effect node, its declared cost, and the cheaper thing to do instead.
///
/// The type the exit rests on. Its fields are private, it has no `Default`, no
/// `From` and no public constructor; [`Effect::declared`] is the only function
/// in this workspace that returns one, and it refuses every input that carries
/// a cost without a fallback or a fallback without a cost. So an [`Effect`] in
/// hand is a whole declaration, and *an effect with no fallback* is not a state
/// a consumer has to test for, cope with, or remember — it is a state that does
/// not exist above this line.
///
/// It is `Copy`, unlike [`kind::Created`](crate::kind::Created): a creation
/// token is evidence and must not be duplicated, but a declaration is a value,
/// and a policy that reads it into a table is doing exactly what it should.
///
/// **No `Ord`, deliberately.** A derived order here would order by node
/// identifier, a table sorted by it would look like a downgrade order, and
/// `E3-B07b`'s whole exit is that the downgrade order is chosen, written down
/// and read from data rather than inherited from whatever a `derive` happened to
/// produce. The numbers that order is allowed to use are all public below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    /// The node this declaration is about.
    node: u32,
    /// What the effect is expected to cost, as the producer declared it.
    estimate_us_x100: NonZeroU32,
    /// What choosing the fallback gives back. Never zero and never above the
    /// estimate, which is what makes both the subtraction below and
    /// [`Effect::saving_us_x100`]'s return type honest.
    saving_us_x100: NonZeroU32,
}

// An `Option<Effect>` costs what an `Effect` costs, and that is the two cost
// fields being non-zero rather than a convenience worth having.
//
// Why here and not in the tests: a size is a fact about the shipped build, and
// this assertion is the compiler's own arithmetic rather than a run of
// anything. Why it is not enough on its own, which is the part a later reader
// needs: it catches *both* cost fields being widened at once and nothing less.
// Widen one and the other still donates the niche, `Option<Effect>` does not
// move, and this line stays green over a type that can now hold half a
// declaration. `the_only_route_from_two_words_to_an_effect_is_still_declared`
// is the half that catches that one, and the two are written next to each other
// so that neither is mistaken for the whole guard.
//
// The reversal condition: a field is added whose type carries a niche of its
// own. Then this assertion is about that field rather than about the costs,
// which is worse than not having it, and it should be deleted rather than kept
// as a true sentence about something else.
const _: () = assert!(core::mem::size_of::<Option<Effect>>() == core::mem::size_of::<Effect>());

impl Effect {
    /// Believe a declaration, or refuse it.
    ///
    /// The one door. It takes the creation token rather than a node number,
    /// because [`Created`] has no public constructor and
    /// [`Change::of`](crate::kind::Change::of) is the only function that returns
    /// one — so *an `Effect` delta* is not a description of how this ought to be
    /// called, it is what holding the first argument means. The token is
    /// borrowed and not consumed: the arena owns it, and a node does not stop
    /// existing because something declared what it costs.
    ///
    /// # Errors
    ///
    /// An [`Undeclared`], and four of the five are about the two words rather
    /// than the node: neither written, one written without the other in either
    /// direction, or a saving larger than the cost it claims to save. The fifth
    /// is a declaration offered for a node that is not [`Kind::Effect`]. Every
    /// one of them completes as [`Undeclared::REFUSAL`].
    pub fn declared(created: &Created, words: Declared) -> Result<Self, Undeclared> {
        // Read out of the table rather than matched, and [`MAY_DECLARE`] is
        // where the argument is: this is the one sentence in the workspace that
        // decides something per kind, so it is the one that has to stop
        // compiling when there is a seventh — which `matches!(.., Kind::Effect)`
        // did not, because a wildcard answers *not an effect* on a new kind's
        // behalf and nothing ever revisits a safe answer.
        if !*MAY_DECLARE.at(created.kind()) {
            return Err(Undeclared::NotAnEffect);
        }
        // The four combinations, written as four arms. `NonZeroU32::new` is the
        // test for *was this word written*, and it is also the type the answer
        // is carried in afterwards, so the check and the invariant are the same
        // expression rather than a check followed by a value that has forgotten
        // it.
        match (NonZeroU32::new(words.estimate_us_x100), NonZeroU32::new(words.saving_us_x100)) {
            (None, None) => Err(Undeclared::Neither),
            (None, Some(_)) => Err(Undeclared::NoEstimate),
            (Some(_), None) => Err(Undeclared::NoFallback),
            (Some(estimate), Some(saving)) => {
                if saving.get() > estimate.get() {
                    return Err(Undeclared::CheaperThanFree);
                }
                Ok(Self {
                    node: created.node(),
                    estimate_us_x100: estimate,
                    saving_us_x100: saving,
                })
            }
        }
    }

    /// The node this declaration is about.
    ///
    /// A `u32` and not a `NonZeroU32`, which is a narrowing this module declines
    /// to redo: the number came from [`Created::node`](crate::kind::Created::node),
    /// which is a `NonZeroU32` behind that type's own accessor, and re-checking
    /// it here would cost a branch that cannot be taken and an answer for a case
    /// that cannot happen.
    /// Unit: none — a node identifier, not a quantity. Never `NO_NODE`.
    #[must_use]
    pub const fn node(self) -> u32 {
        self.node
    }

    /// What the effect is expected to cost.
    ///
    /// Handed back as the non-zero type it was checked into rather than as a
    /// bare `u32`, so that a consumer dividing by it — a cost per saved
    /// microsecond is the obvious ranking for `E3-B07b` — does not have to
    /// re-establish a fact this module already refused the alternative to.
    /// Unit: microseconds, scaled by 100.
    #[must_use]
    pub const fn estimate_us_x100(self) -> NonZeroU32 {
        self.estimate_us_x100
    }

    /// What degrading this node is expected to buy.
    ///
    /// The number the degradation policy sorts by, and the reason this type can
    /// promise it is not zero: a declaration whose fallback saved nothing was
    /// refused at the door rather than carried in with a zero that every
    /// consumer would have to notice.
    /// Unit: microseconds, scaled by 100.
    #[must_use]
    pub const fn saving_us_x100(self) -> NonZeroU32 {
        self.saving_us_x100
    }

    /// What is done instead, and what that costs.
    ///
    /// The subtraction cannot underflow, and not because it is guarded here:
    /// [`Effect::declared`] refused a saving above the estimate, so the only
    /// values that reach this line are the ones the arithmetic is total over.
    #[must_use]
    pub const fn fallback(self) -> Fallback {
        Fallback { cost_us_x100: self.estimate_us_x100.get() - self.saving_us_x100.get() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Change;
    use f_abi::NO_DEADLINE;
    use f_abi::scene::{CreateNode, Delta, Entry, NO_NODE, op};

    /// The node every declaration below is about. Any non-zero value; that it is
    /// non-zero is `CreateNode`'s rule and not this file's.
    const NODE: u32 = 7;

    /// A cost a declarer might plausibly write: 90 microseconds.
    const ESTIMATE: u32 = 9_000;

    /// A saving strictly inside the estimate, so the fallback is a reduction
    /// rather than a skip.
    const SAVING: u32 = 3_500;

    /// The creation token a create delta of `kind` produces.
    ///
    /// Built through `Change::of` rather than by hand, because there is no by
    /// hand: `Created` has no public constructor, which is the property the
    /// boundary below leans on.
    ///
    /// The deadline is [`NO_DEADLINE`] and is asserted to be the only legal
    /// choice rather than selected by an `if`. What stood here was a
    /// `SCHEDULED_AT` constant behind
    /// `op::carries_deadline(body.opcode()) == Some(true)`, which for
    /// `CREATE_NODE` is `Some(false)` and always has been: the constant was
    /// never the value used and the branch was a second arm no test could
    /// reach, dressed as coverage of both. The assertion says the same thing
    /// the branch pretended to — a change to which opcodes are scheduled
    /// reaches this file from the list that states it — and unlike the branch
    /// it goes red when it happens.
    fn created(kind: Kind) -> Created {
        let body = Entry::CreateNode(CreateNode {
            node: NODE,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind.wire(),
        });
        assert_eq!(
            op::carries_deadline(body.opcode()),
            Some(false),
            "a create delta may now carry a deadline, so this helper's envelope is no longer \
             the only legal one for it"
        );
        let delta = Delta {
            user_data: 0,
            class: 0,
            deadline: NO_DEADLINE,
            payload_offset: 0,
            flags: 0,
            body,
        };
        match Change::of(&delta) {
            Ok(Change::Created(created)) => created,
            other => panic!("{}: a create delta did not create: {other:?}", kind.name()),
        }
    }

    #[test]
    fn neither_word_stands_without_the_other() {
        // The exit's clause, over every combination there is rather than over
        // the two somebody remembered: both words present, each present alone,
        // and neither. The absent value is zero because zero is what an
        // unwritten word holds, which is the only way this mistake is actually
        // made.
        for estimate_us_x100 in [0, ESTIMATE] {
            for saving_us_x100 in [0, SAVING] {
                let words = Declared { estimate_us_x100, saving_us_x100 };
                let outcome = Effect::declared(&created(Kind::Effect), words);
                let expected = match (estimate_us_x100, saving_us_x100) {
                    (0, 0) => Err(Undeclared::Neither),
                    (0, _) => Err(Undeclared::NoEstimate),
                    (_, 0) => Err(Undeclared::NoFallback),
                    (_, _) => Ok(()),
                };
                match (outcome, expected) {
                    (Ok(effect), Ok(())) => {
                        assert_eq!(effect.node(), NODE);
                        assert_eq!(effect.estimate_us_x100().get(), estimate_us_x100);
                        assert_eq!(effect.saving_us_x100().get(), saving_us_x100);
                    }
                    (Err(refused), Err(wanted)) => {
                        assert_eq!(refused, wanted, "{}", refused.message());
                    }
                    (got, wanted) => {
                        panic!(
                            "({estimate_us_x100}, {saving_us_x100}): {got:?}, wanted {wanted:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_declaration_belongs_to_an_effect_node_and_to_no_other_kind() {
        // Over `Kind::ALL`, which `kind.rs` emits from the list that declares the
        // kinds — so a seventh kind is asked this question on the day it is
        // written, and lands in whichever half its author's own edit puts it in.
        let words = Declared { estimate_us_x100: ESTIMATE, saving_us_x100: SAVING };
        for kind in Kind::ALL {
            let outcome = Effect::declared(&created(kind), words);
            if matches!(kind, Kind::Effect) {
                assert!(outcome.is_ok(), "{}: an effect node could not declare", kind.name());
            } else {
                assert_eq!(
                    outcome,
                    Err(Undeclared::NotAnEffect),
                    "{}: a node that is not an effect declared one",
                    kind.name()
                );
            }
        }
    }

    #[test]
    fn a_fallback_is_cheaper_than_the_effect_and_not_cheaper_than_free() {
        let node = created(Kind::Effect);
        let declare = |saving_us_x100| {
            Effect::declared(&node, Declared { estimate_us_x100: ESTIMATE, saving_us_x100 })
        };

        // Free is the floor and it is a legal declaration: the effect is not
        // applied at all, which is what `Instead::Skip` means.
        let free = declare(ESTIMATE).expect("a fallback that is free is still a fallback");
        assert_eq!(free.fallback().cost_us_x100(), 0);
        assert_eq!(free.fallback().instead(), Instead::Skip);

        // One unit dearer than free is a reduction, and the cheapest reduction
        // there is.
        let barely = declare(ESTIMATE - 1).expect("a reduction is a fallback");
        assert_eq!(barely.fallback().cost_us_x100(), 1);
        assert_eq!(barely.fallback().instead(), Instead::Reduced);

        // Past the floor there is nothing to declare. Checked at the boundary,
        // one unit past it and at the top of the range, because a bound that is
        // only tested far from itself is a bound nobody has tested.
        for saving in [ESTIMATE + 1, u32::MAX] {
            assert_eq!(declare(saving), Err(Undeclared::CheaperThanFree));
        }
    }

    #[test]
    fn the_saving_is_exactly_what_the_fallback_does_not_spend() {
        // The arithmetic the boundary made total. Every saving from one unit to
        // the whole estimate, so the two ends are covered by the same loop that
        // covers the middle.
        let node = created(Kind::Effect);
        for saving_us_x100 in 1..=ESTIMATE {
            let effect =
                Effect::declared(&node, Declared { estimate_us_x100: ESTIMATE, saving_us_x100 })
                    .expect("a whole declaration inside the range is believed");
            assert_eq!(
                effect.fallback().cost_us_x100() + effect.saving_us_x100().get(),
                effect.estimate_us_x100().get()
            );
            assert!(effect.fallback().cost_us_x100() < effect.estimate_us_x100().get());
        }
    }

    #[test]
    fn a_refusal_says_which_half_was_missing() {
        // Five refusals are five sentences, because a producer that forgot a
        // word reads a log rather than a discriminant. Distinctness is the whole
        // property: a message shared by two causes is a message that identifies
        // neither.
        //
        // Over `Undeclared::ALL`, which `refusals!` emits from the lines that
        // declare the variants — so a sixth refusal is asked this question on
        // the day it is written. It used to be over a five-element array typed
        // out here, which a sixth variant would never have joined: the enum's
        // exhaustive `message` match would have demanded a sentence for it and
        // this list would have gone on checking the other five, so a
        // copy-pasted sentence shipped green. That is this repository's own
        // listed mistake, and the macro is the same repair it made twice
        // before.
        // `assert_eq!(Undeclared::ALL.len(), Undeclared::COUNT)` stood here and
        // is deleted rather than kept: `ALL` is declared
        // `[Self; Self::COUNT]`, so its length *is* `COUNT` by its own type and
        // no edit to this workspace could make the comparison false. It sat at
        // the head of the one test that is a guard and read like a third one.
        //
        // What stood here after it was a pairwise `assert_ne!` over the
        // sentences, and it was not this test's name. **Distinctness is not
        // identification.** Swapping `NoEstimate`'s sentence with
        // `NoFallback`'s — telling a producer that forgot the saving that it
        // forgot the estimate — leaves five distinct non-empty sentences and
        // left this whole crate green; `message` has no other reader in the
        // workspace, so nothing else would have caught it either. The clause
        // in the exit is about what the producer is told, and the only reader
        // of that English is a human, so the test has to be the one that reads
        // it.
        //
        // The correspondence below is what a name has and a sentence did not:
        // each refusal's `names()` fragment appears in its own sentence and in
        // nobody else's. It subsumes both deleted assertions — a non-empty
        // sentence, because a non-empty fragment is inside it, and pairwise
        // distinctness, because two equal sentences would each contain the
        // other's fragment — which is why they are named here rather than kept
        // as a second statement of an implied fact.
        //
        // *The mutations that redden this:* swapping any two sentences;
        // copying one sentence over another; writing a sixth refusal whose
        // `names()` fragment is a substring of somebody else's sentence.
        for (at, one) in Undeclared::ALL.iter().enumerate() {
            assert!(!one.names().is_empty(), "{one:?} names no half");
            assert!(
                one.message().contains(one.names()),
                "{one:?} says {:?}, which does not contain {:?}: its sentence is about \
                 some other refusal than itself",
                one.message(),
                one.names()
            );
            for (also, other) in Undeclared::ALL.iter().enumerate() {
                if also == at {
                    continue;
                }
                assert!(
                    !other.message().contains(one.names()),
                    "{other:?} says {:?}, which contains {one:?}'s own {:?}: one sentence \
                     identifies two causes and therefore neither",
                    other.message(),
                    one.names()
                );
            }
        }
        // One code on the wire, five values here — the module documentation's
        // argument, asserted so that a later edit that split the code has to
        // come through this line.
        assert_eq!(Undeclared::REFUSAL, Refusal::Value);
    }

    /// This file, as text.
    ///
    /// Read rather than called, because the two clauses below are about what
    /// this file **does not** contain, and a second constructor is invisible to
    /// a test that calls the first one. Every test above could pass unchanged
    /// beside a `pub const fn from_parts` that skipped every check in
    /// [`Effect::declared`], and beside a cost field narrowed to `u32` with its
    /// accessor rewritten to hand back `NonZeroU32::MIN` on the zero it can now
    /// hold — both are safe, both compile, and neither is refused by anything
    /// that runs.
    ///
    /// `include_str!` reads at compile time and needs no filesystem at run
    /// time, which is the route `abi/src/semantic.rs` already reads
    /// `interface/src/node.rs` by, for the same reason: a fact about a source
    /// file is not otherwise observable to a run, and a comment claiming it is
    /// the thing that rots.
    ///
    /// *What would reverse this:* an `xtask` lint making the same statement
    /// over this file from outside it. That is the better home — it would fail
    /// the build rather than a test, and it could hold the rule for every type
    /// of this shape rather than for one. This sits here because the property
    /// is `E3-B07a`'s exit and the exit is closed here; a lint that subsumes it
    /// should delete this test rather than stand beside it.
    const SOURCE: &str = include_str!("effect.rs");

    /// The body of `impl Effect`, from its opening line to the `}` in column
    /// zero that closes it.
    ///
    /// Sliced rather than parsed, and the slice is narrow on purpose: an
    /// `impl Effect` block is where a second inherent constructor would be
    /// written, and scanning the whole file would find the helpers in this test
    /// module instead.
    fn impl_effect() -> &'static str {
        const OPEN: &str = "\nimpl Effect {\n";
        let at = SOURCE
            .find(OPEN)
            .expect("`impl Effect {` is no longer a line of its own, so this scan reads nothing");
        let rest = &SOURCE[at + OPEN.len()..];
        let end = rest
            .find("\n}\n")
            .expect("`impl Effect` has no closing brace in column zero, so this scan has no end");
        &rest[..end]
    }

    #[test]
    fn the_only_route_from_two_words_to_an_effect_is_still_declared() {
        // The clause of the exit that no run could observe: *`Effect::declared`
        // is the only constructor of an `Effect`, whose two cost fields are
        // `NonZeroU32`*. Both halves were true by reading and defended by
        // nothing, and they are each other's only support — *no value of the
        // type can hold half a declaration* needs the fields to be non-zero
        // **and** needs there to be no other way in, so a guard over one of
        // them is not a guard over the sentence. That is why they are one test
        // and must not be split.

        // One inherent function returns an `Effect`, and it is the one the exit
        // names. Signatures are read off one line each, which is what
        // `rustfmt.toml`'s `use_small_heuristics = "Max"` makes true of this
        // file; a signature this scan cannot read is a failure rather than a
        // skip, because a scan that silently ignores what it cannot parse is
        // the decoration this test exists not to be.
        let mut routes = 0_usize;
        for line in impl_effect().lines() {
            let line = line.trim();
            if line.starts_with("//") || !line.contains("fn ") {
                continue;
            }
            assert!(
                line.contains("->") && line.ends_with('{'),
                "this scan reads a whole signature off one line and cannot read `{line}`: a \
                 function in `impl Effect` that wraps or returns nothing needs it taught first"
            );
            let returns = line.rsplit("->").next().expect("a signature with an arrow has a tail");
            if returns.contains("Self") || returns.contains("Effect") {
                routes += 1;
                assert!(
                    line.contains("fn declared("),
                    "`{line}` is a second way to obtain an `Effect`, and the exit is that there \
                     is one; if it is a real need, the exit is the thing to change first"
                );
            }
        }
        assert_eq!(
            routes, 1,
            "`impl Effect` no longer has exactly one function that returns one, so either \
             `declared` is gone or this scan stopped seeing it"
        );

        // The family the scan above cannot see, because its methods live in an
        // `impl Trait for` block rather than in `impl Effect`: `Default`,
        // `From`, `TryFrom`, `FromStr`, a deserialiser. The rule is blanket
        // rather than a list of the ones somebody thought of, because the list
        // of traits whose method returns `Self` is not a list this file can
        // finish. A trait genuinely wanted here comes through this line and
        // says why it is not a constructor.
        // Assembled with `concat!` rather than written out, and not for style:
        // this file is its own haystack, so a needle spelled here is a needle
        // this line finds in itself. It did, on the first run. Adjacent string
        // literals do not concatenate in Rust, which is why this is a macro and
        // not two strings side by side.
        const TRAIT_IMPL: &str = concat!(" for ", "Effect ");
        assert!(
            !SOURCE.contains(TRAIT_IMPL),
            "a trait is implemented for `Effect` by hand; if its method does not return one, \
             say so here and let it past"
        );

        // The derive list, pinned whole rather than searched for `Default`.
        // Two things ride on it. `Default` in it is a constructor that answers
        // zero for both costs. `Ord` in it is the order this type's own
        // documentation refuses eight lines above the struct — a table sorted
        // by node identifier that looks like a downgrade order — and `E3-B07b`
        // is the task that would inherit it. One line refuses both, and costs
        // an edit here for any derive somebody genuinely wants.
        assert!(
            SOURCE.contains("#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub struct Effect {\n"),
            "`Effect`'s derive list changed; `Default` there is a constructor and `Ord` there is \
             `E3-B07b`'s decision made by a macro"
        );

        // And the fields, spelled. This is the clause the accessors cannot
        // carry: `saving_us_x100: u32` behind an accessor that still returns
        // `NonZeroU32` is a const-legal, `unsafe`-free edit that no assertion
        // in this module notices, because `declared` never writes a zero — and
        // after it an in-module value holding half a declaration is
        // expressible, which is exactly what the exit says cannot happen.
        //
        // The count is pinned too, because the sentence in the spec says *two*
        // and the struct has three: a fourth field would make the count wrong
        // in the other direction and nothing else here would see it.
        const FIELDS_OPEN: &str = "pub struct Effect {\n";
        let at = SOURCE.find(FIELDS_OPEN).expect("`pub struct Effect {` is no longer written");
        let rest = &SOURCE[at + FIELDS_OPEN.len()..];
        let fields = &rest[..rest.find("\n}\n").expect("`struct Effect` does not close")];
        for spelling in
            ["node: u32,", "estimate_us_x100: NonZeroU32,", "saving_us_x100: NonZeroU32,"]
        {
            assert!(
                fields.contains(spelling),
                "`Effect` no longer declares `{spelling}`; a cost field that is not `NonZeroU32` \
                 can hold the zero `declared` refuses"
            );
        }
        let declared = fields
            .lines()
            .filter(|line| {
                let line = line.trim();
                !line.starts_with("//") && line.contains(':') && line.ends_with(',')
            })
            .count();
        assert_eq!(
            declared, 3,
            "`Effect` has a field the exit does not know about; the exit counts the two that \
             are costs and this test counts all of them"
        );
    }
}
