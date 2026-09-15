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
//! *An effect with no fallback is a declaration error rather than a frame-time
//! surprise* is the whole of the exit, and both halves of it are here — the
//! refusal below, and the fact that past the refusal there is no [`Effect`]
//! holding one number and not the other, because the type has no shape for one.
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
//! opcodes and none of them carries an effect's parameters, and adding one is an
//! ABI change with an RFC behind it. That costs this file nothing — [`Declared`]
//! is those words as they will arrive and [`Effect::declared`] is the door they
//! pass, and a door refuses whether or not anything has knocked yet. What it
//! costs the reader is one stated limit, in the same place `kind.rs` states its
//! own: nothing in this workspace can force a future decoder to call this
//! function. What it can do, and does, is make sure there is no *other* route
//! from two words to an [`Effect`], and no way to hold half of one afterwards.
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

use crate::kind::{Created, Kind};

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

/// Why a declaration was not believed.
///
/// A local enum rather than `abi::scene::Refusal`, and the difference is worth
/// a sentence: `kind.rs` returns that one because every refusal it makes is a
/// refusal the wire decoder would also have made, and two statements of one rule
/// is how two halves of a system come to disagree. None of these is such a rule.
/// `abi::scene` has no effect record and therefore no opinion about these two
/// words, so returning its type would claim an agreement that does not exist
/// yet.
///
/// What does not vary is what a peer is told: [`Undeclared::REFUSAL`] is one
/// code for all of them, which is `abi::scene::Refusal::packed`'s own argument
/// — the distinctions below are for the producer reading its own log, and the
/// domain is the part that is stable on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undeclared {
    /// The node is not an effect node.
    ///
    /// A cost and a fallback attached to a draw or a clip is a declaration about
    /// something that has no cheaper form to fall back to, and a policy that
    /// accepted it would be degrading nodes whose kind never said it could be
    /// degraded.
    NotAnEffect,
    /// Neither word was written.
    ///
    /// Kept distinct from the two halves below because a producer that wrote
    /// nothing is a different defect from one that wrote half: the first forgot
    /// the record, the second forgot a field in it, and they are fixed in
    /// different places.
    Neither,
    /// A saving, and nothing for it to be a saving from.
    ///
    /// The mirror of [`Undeclared::NoFallback`], and refused just as hard. A
    /// fallback whose effect has no declared cost cannot be ranked against
    /// anything, so accepting it would put a node in the policy's table that the
    /// policy can never choose.
    NoEstimate,
    /// An estimate, and nothing cheaper to do instead.
    ///
    /// The case the exit is written about: an effect that has told the
    /// compositor what it will cost and not what to do when that is too much.
    NoFallback,
    /// A fallback that gives back more than the effect ever cost.
    ///
    /// Nothing is cheaper than free, so a saving above the estimate is not a
    /// generous declaration, it is an incoherent one — and a policy that
    /// subtracted it from a frame's remaining budget would believe it had
    /// recovered time that never existed.
    CheaperThanFree,
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

    /// A line for a log.
    ///
    /// Written per variant rather than derived from the name, because the name
    /// says which case this is and the sentence says what the producer did
    /// wrong, and a producer reading a boot log has the second question.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotAnEffect => "a declaration was offered for a node that is not an effect",
            Self::Neither => "an effect node declared neither a cost nor a fallback",
            Self::NoEstimate => "an effect node declared a saving and no cost to save from",
            Self::NoFallback => "an effect node declared a cost and nothing cheaper to do instead",
            Self::CheaperThanFree => "a fallback saves more than the effect it replaces costs",
        }
    }
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
        // Matched rather than compared, because `Kind` has no const equality and
        // because a match is what a seventh kind would have to be re-read
        // against — the kinds are closed in `kind.rs` and this arm is the one
        // sentence here that depends on which of them is which.
        if !matches!(created.kind(), Kind::Effect) {
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

    /// A deadline far enough from zero to be unmistakably a deadline.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

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
    /// boundary below leans on. The envelope comes from `op::carries_deadline`
    /// rather than from a literal here, so a change to which opcodes are
    /// scheduled reaches this file from the list that states it.
    fn created(kind: Kind) -> Created {
        let body = Entry::CreateNode(CreateNode {
            node: NODE,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind.wire(),
        });
        let scheduled = op::carries_deadline(body.opcode()) == Some(true);
        let delta = Delta {
            user_data: 0,
            class: 0,
            deadline: if scheduled { SCHEDULED_AT } else { NO_DEADLINE },
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
        // The five refusals are five sentences, because a producer that forgot a
        // word reads a log rather than a discriminant. Distinctness is the whole
        // property: a message shared by two causes is a message that identifies
        // neither.
        let messages = [
            Undeclared::NotAnEffect.message(),
            Undeclared::Neither.message(),
            Undeclared::NoEstimate.message(),
            Undeclared::NoFallback.message(),
            Undeclared::CheaperThanFree.message(),
        ];
        for (at, one) in messages.iter().enumerate() {
            assert!(!one.is_empty());
            for other in &messages[at + 1..] {
                assert_ne!(one, other);
            }
        }
        // One code on the wire, five values here — the module documentation's
        // argument, asserted so that a later edit that split the code has to
        // come through this line.
        assert_eq!(Undeclared::REFUSAL, Refusal::Value);
    }
}
