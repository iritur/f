// SPDX-License-Identifier: Apache-2.0 OR MIT
//! An agent's invocation, as a capability call: authorised, logged, and refused
//! once the capability is revoked.
//!
//! `f_interface::agent` is the projection — it locates the clip and answers
//! what may be done with it. This is what the doing *is*. The split is not
//! tidiness: `interface/Cargo.toml` declines `f-abi` so that the vocabulary
//! stays a leaf that can be deleted, and a handle is `f-abi`'s. This crate is
//! the one place that may take both, and `semantic/Cargo.toml` argues at length
//! why it exists at all; the agent's two halves land either side of that line
//! for exactly the reason the line is there.
//!
//! # `f_interface::node::CapRef`'s own comment names this task
//!
//! > a handle is an index into *one component's* capability table and means
//! > nothing outside that component. […] what a node carries has to be
//! > something the *owner of the tree* can resolve. Today nothing resolves it.
//!
//! [`Authority`] is a thing that resolves it. A node carries a [`CapRef`] — an
//! opaque token the declaring application chose — and a would-be invoker
//! carries a [`Handle`], which is the same slot-and-generation pair the
//! capability table has had since RFC 0002. [`Authority::invoke`] demands both
//! and checks that they are about the same intent. Neither buys the other:
//! there is no call here that turns a [`CapRef`] read out of a tree into a
//! [`Handle`], which is `registry.rs`'s rule between [`TreeId`](crate::TreeId)
//! and [`Handle`] applied one layer down and for the same reason. An agent that
//! can *see* an intent has bought nothing towards invoking it.
//!
//! # What this is not
//!
//! **It is not the frame's capability table.** That is `kernel/src/cap.rs`, a
//! boot is what exercises it, and `cargo xtask cap` is the eight boots that do.
//! What is here is that command's *discipline* in the new place `E3-B06j` asks
//! for: a handle that names no slot is [`Refused::NoSuchCapability`]
//! (`cap=unowned`, `cap=beyond`); a handle from before a revoke is
//! [`Refused::Revoked`] (`cap=stale`, `cap=unmap`); a handle over an object
//! this operation is not about is [`Refused::WrongIntent`] (`cap=type`); and a
//! handle held without the right the act needs is
//! [`Refused::RightNotHeld`] (`cap=rights`). Each of the four answers with the
//! code `f_abi::error::authority` already defines for it, through
//! [`Refused::authority`], so the refusal an agent is told is the refusal the
//! frame would tell it and not a second vocabulary for the same four facts.
//!
//! The *order* is the frame's too, and it had to be looked up rather than
//! guessed: `kernel/src/cap.rs`'s `Table::invoke` resolves the handle, then
//! refuses the wrong kind, then the missing right, and its `resolve_at` answers
//! `revoked()` only for a generation **below** the slot's — above it is
//! `no_such()`, because a handle from the future was issued to nobody and is a
//! caller guessing rather than one that has lost something. `Authority::decide`
//! draws both lines in the same places. *What would reverse this:* the frame
//! reordering its own, at which point this module follows rather than argues.
//!
//! *What would reverse this:* the day the tree crosses a ring. Then the frame's
//! table is the only one, this module resolves a [`CapRef`] into a [`Handle`]
//! rather than holding a table of its own, and the tests below become a boot.
//! `f_interface::node::CapRef`'s comment already says that day needs its own
//! RFC, and this module does not spend it.
//!
//! # Why the right an invocation needs is [`rights::WRITE`]
//!
//! Because invoking is what `f_interface::node::Family::Control` calls
//! *operating it changes the world*, and `f_abi::cap::rights` already has the
//! bit that means *may change this*. The alternative was a seventh bit, and
//! that module says what one costs: *adding a seventh is an ABI change and
//! should be argued as one — a rights bitmap that grows without argument is how
//! a permission model becomes a list of special cases.* An agent handed
//! [`rights::READ`] over an intent may therefore read the tree, see the intent,
//! and not invoke it, which is a grant somebody would actually write.
//!
//! *What would reverse this:* an operation that is an invocation and is not a
//! change — a query intent, which the vocabulary has no word for today.
//!
//! # The log, and the one thing a flood can and cannot erase
//!
//! [`Log`] keeps the most recent [`Record`]s in a ring and counts everything
//! that ever happened. That is a deliberate asymmetry: an agent that floods the
//! table with refusals can push the detail of the first one out of the ring, and
//! cannot move [`Log::refused`] back down, so *how many* and *how many of those
//! were refused* survive any flood and the ring is where a reader goes for
//! *which*. [`Log::lost`] says how many records the ring dropped, so a reader is
//! never quietly told a partial history is the whole of it.
//!
//! The alternative — refuse the invocation when the log is full — was rejected
//! because it hands any caller a denial of service over every other caller. The
//! alternative in the other direction — keep the first N — was rejected because
//! the interesting record in a live system is the last one. *What would reverse
//! this:* a consumer that must have every record, at which point the log is a
//! stream and not a buffer and this type is the wrong shape.
//!
//! **A record carries no time**, which is RFC 0004 rather than an omission:
//! nothing observes a clock outside `f_env::Env` and this crate is `no_std`
//! with no environment in reach. [`Record::sequence`] counts from one and is
//! what orders a log, which is the only property anything reading one needs.

use f_abi::cap::{Handle, rights};
use f_abi::error;
use f_interface::agent::{Invocation, Selected};
use f_interface::canvas::Operable;
use f_interface::node::{CapRef, NodeId, Role};

/// Why an invocation did not happen.
///
/// Seven refusals in two families, and the families are the point. The first
/// three are the *projection's*: what the application declared, read back at
/// the agent. The last four are the capability table's, and each is one of
/// `f_abi::error::authority`'s four codes — [`Refused::authority`] is where the
/// mapping lives, and it answers `None` for the first three, because *this
/// application is not offering that* is not a statement about anybody's
/// authority and telling an agent otherwise would send it looking for a grant
/// that would not have helped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// An agent does not invoke a node of that role at all —
    /// [`Invocation::Never`]. The role is carried because that is the whole of
    /// the answer.
    NeverInvoked(Role),
    /// The node declares no intent. Read-only is what `node.rs` spells as the
    /// absence of one, so this is an honest declaration rather than a fault,
    /// and it will never become invocable without the tree changing.
    NothingDeclared,
    /// The node declares an intent and the application does not currently
    /// declare it operable. Not now, which is not the same as not ever — so an
    /// agent may wait, and the intent is carried so it knows what it is waiting
    /// for.
    NotNow(CapRef),
    /// The handle names no capability: a slot past the end of the table, a slot
    /// nothing was ever issued at, or the zero word, which names nothing by
    /// construction.
    NoSuchCapability,
    /// **The clause.** The handle was issued over this slot and the slot has
    /// moved on — revoked, or revoked and reissued to somebody else. Both
    /// generations are carried so that a report can say *minted at one, the
    /// slot is at two* rather than only *refused*, which is the shape
    /// `Registry::generation` is published in for.
    Revoked {
        /// The generation the handle carries.
        /// Unit: none — a generation ordinal.
        held: u16,
        /// The generation the slot is at now.
        /// Unit: none — a generation ordinal.
        now: u16,
    },
    /// The handle is live and names a different intent from the one the node
    /// declares.
    ///
    /// Checked before the rights, deliberately: a caller told *right not held*
    /// would go and widen a grant it already holds over the wrong object, and
    /// come back with the same refusal. `f_abi::error::authority::WRONG_TYPE`
    /// makes the same argument about the same confusion one layer down.
    WrongIntent {
        /// What the capability is about.
        names: CapRef,
        /// What the node declares.
        asked: CapRef,
    },
    /// The handle names the right intent and does not carry [`rights::WRITE`].
    RightNotHeld {
        /// The rights the capability carries.
        /// Unit: none — a bitmap of `f_abi::cap::rights`.
        held: u8,
        /// The rights the act needs.
        /// Unit: none — a bitmap of `f_abi::cap::rights`.
        asked: u8,
    },
}

impl Refused {
    /// This refusal as the packed result the frame would answer with, or `None`
    /// for the three that are not the capability table's to refuse.
    ///
    /// Packed rather than the bare code, because the domain is half the answer:
    /// a caller that sees `AUTHORITY` knows to look at its handles and one that
    /// sees nothing at all knows to look at the tree.
    #[must_use]
    pub const fn authority(self) -> Option<i32> {
        let code = match self {
            Self::NeverInvoked(_) | Self::NothingDeclared | Self::NotNow(_) => return None,
            Self::NoSuchCapability => error::authority::NO_SUCH_CAP,
            Self::Revoked { .. } => error::authority::REVOKED,
            Self::WrongIntent { .. } => error::authority::WRONG_TYPE,
            Self::RightNotHeld { .. } => error::authority::RIGHT_NOT_HELD,
        };
        Some(error::pack(error::AUTHORITY, code))
    }
}

/// What happened to one invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It was authorised and it happened.
    Invoked,
    /// It did not.
    Refused(Refused),
}

/// One line of the log.
///
/// It names the node, the intent, the handle presented and what came of it —
/// the four things a reader auditing *who invoked what, with whose authority*
/// asks for. A refused call is a record like any other: a log that recorded
/// only what succeeded would answer *nothing happened* for a component that
/// spent a boot sweeping the handle space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// Which invocation this was, counting from one.
    ///
    /// Zero is [`Record::NOTHING`] and never a written record, which is what
    /// lets a filler be told from a line. Unit: none — an ordinal over every
    /// invocation this log has seen.
    pub sequence: u64,
    /// Which node was selected.
    pub node: NodeId,
    /// What the node declares operating it does, if it declares anything. Read
    /// from the selection rather than from the handle, so that a record of a
    /// refusal says what was *asked for* and not what the table happened to
    /// hold.
    pub intent: Option<CapRef>,
    /// The handle presented. [`Handle::NULL`] is a caller that presented the
    /// zero word, which is a record worth keeping.
    pub handle: Handle,
    /// What came of it.
    pub outcome: Outcome,
}

impl Record {
    /// A line nobody wrote: what a fresh [`Log`] is filled with.
    ///
    /// [`Outcome::Refused`] with [`Refused::NoSuchCapability`] for
    /// `Utterance::UNSAID`'s reason — there is no *nothing happened* outcome and
    /// this module will not invent one for a filler — and the sequence is zero,
    /// which is the field that says it is filler. [`Log::records`] never yields
    /// one.
    pub const NOTHING: Self = Self {
        sequence: 0,
        node: NodeId::UNNAMED,
        intent: None,
        handle: Handle::NULL,
        outcome: Outcome::Refused(Refused::NoSuchCapability),
    };
}

/// Every invocation that was attempted, and the most recent `ENTRIES` of them
/// in detail.
///
/// Generic over the count for [`crate::Registry`]'s reason: a frame states its
/// own and a host test states two, and a constant here would be the frame's
/// number in a crate the frame links.
#[derive(Clone, Copy, Debug)]
pub struct Log<const ENTRIES: usize> {
    /// The ring.
    kept: [Record; ENTRIES],
    /// Where the next record goes.
    next: usize,
    /// How many were authorised. Unit: invocations.
    invoked: u64,
    /// How many were not. Unit: invocations.
    refused: u64,
    /// How many records the ring has dropped. Unit: records.
    lost: u64,
}

impl<const ENTRIES: usize> Log<ENTRIES> {
    /// A log of nothing.
    pub const EMPTY: Self =
        Self { kept: [Record::NOTHING; ENTRIES], next: 0, invoked: 0, refused: 0, lost: 0 };

    /// How many invocations were authorised. Unit: invocations.
    #[must_use]
    pub const fn invoked(&self) -> u64 {
        self.invoked
    }

    /// How many were refused. Unit: invocations.
    #[must_use]
    pub const fn refused(&self) -> u64 {
        self.refused
    }

    /// How many were attempted at all. Unit: invocations.
    #[must_use]
    pub const fn written(&self) -> u64 {
        self.invoked + self.refused
    }

    /// How many records the ring dropped to make room. Unit: records.
    ///
    /// The number that keeps [`Log::records`] honest: a reader seeing a nonzero
    /// value knows it is holding a tail and not a history.
    #[must_use]
    pub const fn lost(&self) -> u64 {
        self.lost
    }

    /// The records the ring still holds, oldest first.
    pub fn records(&self) -> impl Iterator<Item = &Record> + '_ {
        let held = if ENTRIES == 0 { 0 } else { (self.written() as usize).min(ENTRIES) };
        (0..held).map(move |step| &self.kept[(self.next + ENTRIES - held + step) % ENTRIES])
    }

    /// The most recent record, if the ring holds one.
    #[must_use]
    pub fn last(&self) -> Option<&Record> {
        self.records().last()
    }

    /// Write one record and answer its sequence.
    ///
    /// The counters move whether or not there is room for the detail, which is
    /// the module comment's asymmetry made mechanical: the two lines a flood
    /// cannot touch are updated before the ring is.
    fn write(
        &mut self,
        node: NodeId,
        intent: Option<CapRef>,
        handle: Handle,
        outcome: Outcome,
    ) -> u64 {
        match outcome {
            Outcome::Invoked => self.invoked += 1,
            Outcome::Refused(_) => self.refused += 1,
        }
        let sequence = self.written();
        if ENTRIES == 0 {
            self.lost += 1;
            return sequence;
        }
        if sequence > ENTRIES as u64 {
            self.lost += 1;
        }
        self.kept[self.next] = Record { sequence, node, intent, handle, outcome };
        self.next = (self.next + 1) % ENTRIES;
        sequence
    }
}

impl<const ENTRIES: usize> Default for Log<ENTRIES> {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// An invocation that was authorised and happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Invoked {
    /// Which node.
    pub node: NodeId,
    /// What was invoked, as the application named it.
    pub intent: CapRef,
    /// The handle that authorised it.
    pub handle: Handle,
    /// Which record of the log this is. Unit: none — an invocation ordinal.
    pub sequence: u64,
}

/// What a slot is doing.
///
/// Three and not two, on [`crate::Registry`]'s argument exactly: *there was
/// never a capability here* and *the one that was here is gone* are different
/// facts about the world, and `f_abi::error::authority::REVOKED`'s own comment
/// says they need different handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// Nothing was ever issued here.
    Vacant,
    /// A live capability.
    Held,
    /// One that was revoked and has not been reissued.
    Revoked,
}

/// One capability over one intent.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// The generation a handle over this slot must carry. Zero while vacant,
    /// which is what makes [`Handle::NULL`] name nothing.
    /// Unit: none — a generation ordinal.
    generation: u16,
    /// What the slot is doing.
    standing: Standing,
    /// What the capability is about.
    intent: CapRef,
    /// What its holder may do with it.
    /// Unit: none — a bitmap of `f_abi::cap::rights`.
    rights: u8,
}

impl Slot {
    /// A slot holding nothing.
    const VACANT: Self = Self {
        generation: 0,
        standing: Standing::Vacant,
        intent: CapRef::new(0),
        rights: rights::NONE,
    };
}

/// The capabilities an agent may be handed over a tree's intents.
///
/// Read to invoke and written only to issue or revoke, which is why
/// [`Authority::invoke`] takes `&self`: an invocation is not a change to who
/// may invoke. Saying that with a borrow rather than in a comment is what stops
/// the check and the grant from drifting into one call.
#[derive(Clone, Copy, Debug)]
pub struct Authority<const SLOTS: usize> {
    slots: [Slot; SLOTS],
}

impl<const SLOTS: usize> Authority<SLOTS> {
    /// A table holding no capabilities.
    pub const EMPTY: Self = Self { slots: [Slot::VACANT; SLOTS] };

    /// Hand out a capability over `intent`.
    ///
    /// `None` when every slot is taken. A revoked slot is **not** taken —
    /// [`reissue`](Self::reissue) is how it comes back, and it comes back at
    /// the generation the revoke moved to, so nothing that was handed out
    /// before it starts working again.
    pub fn issue(&mut self, intent: CapRef, rights: u8) -> Option<Handle> {
        let (index, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.standing == Standing::Vacant)?;
        slot.generation = Handle::FIRST_GENERATION;
        slot.standing = Standing::Held;
        slot.intent = intent;
        slot.rights = rights;
        let index = u16::try_from(index).ok()?;
        Some(Handle::new(index, slot.generation))
    }

    /// Take the capability at `index` back.
    ///
    /// **The other half of the clause.** The generation moves on, so every
    /// handle minted over this slot is stale from here — `Registry::close`'s
    /// one call and its saturating step, for the reason RFC 0002 gives: a
    /// generation that wraps is a stale handle that becomes valid again, so a
    /// slot that reaches [`Handle::RETIRED_GENERATION`] is retired rather than
    /// reissued.
    ///
    /// Answers whether there was a live capability to take. `false` for a slot
    /// that is vacant or already revoked, so that revoking twice is a `false`
    /// rather than a second generation — which would otherwise let a caller
    /// walk a slot to retirement by asking repeatedly.
    pub fn revoke(&mut self, index: u16) -> bool {
        let Some(slot) = self.slots.get_mut(index as usize) else { return false };
        if slot.standing != Standing::Held {
            return false;
        }
        slot.standing = Standing::Revoked;
        slot.generation = slot.generation.saturating_add(1);
        true
    }

    /// Hand a revoked slot out again, over whatever intent the new holder is
    /// being given.
    ///
    /// `Registry::adopt`'s shape and its purpose: after this the slot is
    /// **live**, so the only thing standing between the old holder's handle and
    /// the object is the generation it was minted at. A suite that only ever
    /// tested against a shut slot would be testing that shut slots refuse.
    ///
    /// `None` for a slot that is not revoked, and for one whose generation has
    /// reached [`Handle::RETIRED_GENERATION`].
    pub fn reissue(&mut self, index: u16, intent: CapRef, rights: u8) -> Option<Handle> {
        let slot = self.slots.get_mut(index as usize)?;
        if slot.standing != Standing::Revoked || slot.generation == Handle::RETIRED_GENERATION {
            return None;
        }
        slot.standing = Standing::Held;
        slot.intent = intent;
        slot.rights = rights;
        Some(Handle::new(index, slot.generation))
    }

    /// The generation a handle over that slot must now carry, or zero.
    ///
    /// Published so a report can name both numbers. Unit: none — a generation
    /// ordinal.
    #[must_use]
    pub fn generation(&self, index: u16) -> u16 {
        self.slots.get(index as usize).map_or(0, |slot| slot.generation)
    }

    /// Invoke what `selected` declares, on the authority of `handle`, and write
    /// what happened to `log`.
    ///
    /// **The exit.** The three things it demands are a selection this
    /// projection minted, a handle somebody was issued, and a log; there is no
    /// call here that takes a bare [`CapRef`], so an agent that has read an
    /// intent out of a tree has no route to this function with it.
    ///
    /// Every outcome is logged, authorised or not, before it is answered.
    ///
    /// # Errors
    ///
    /// [`Refused`]'s seven, in the order the type documents: what the
    /// application declared first, the capability table second. A refusal from
    /// the first family means the handle was never consulted, which is why
    /// [`Refused::authority`] answers `None` for those three.
    pub fn invoke<const ENTRIES: usize>(
        &self,
        selected: &Selected,
        handle: Handle,
        log: &mut Log<ENTRIES>,
    ) -> Result<Invoked, Refused> {
        let decided = self.decide(selected, handle);
        let outcome = match decided {
            Ok(_) => Outcome::Invoked,
            Err(refused) => Outcome::Refused(refused),
        };
        let declared = match selected.operable() {
            Operable::Invocable(intent) | Operable::Disabled(intent) => Some(intent),
            Operable::Inert => None,
        };
        let sequence = log.write(selected.node(), declared, handle, outcome);
        match decided {
            Ok(intent) => Ok(Invoked { node: selected.node(), intent, handle, sequence }),
            Err(refused) => Err(refused),
        }
    }

    /// Would this invocation be authorised, and over what intent?
    ///
    /// Split from [`invoke`](Self::invoke) so that the decision is one
    /// expression and the logging cannot be forgotten on a path: every `return`
    /// below lands in the same `match` above.
    fn decide(&self, selected: &Selected, handle: Handle) -> Result<CapRef, Refused> {
        // --- what the application declared ----------------------------------
        if selected.affordance().invocation == Invocation::Never {
            return Err(Refused::NeverInvoked(selected.role()));
        }
        let intent = match selected.operable() {
            Operable::Invocable(intent) => intent,
            Operable::Disabled(intent) => return Err(Refused::NotNow(intent)),
            Operable::Inert => return Err(Refused::NothingDeclared),
        };

        // --- and only now, the capability table -----------------------------
        // The zero word first, and on its own, because a handle at generation
        // zero over slot zero would otherwise be answered `Revoked` — which is
        // a wrong answer with a plausible shape, since nothing was ever issued
        // there. RFC 0002's rule: generations count from one, so a zeroed field
        // is never an authority.
        if !handle.is_issuable() {
            return Err(Refused::NoSuchCapability);
        }
        let slot = self.slots.get(handle.index() as usize).ok_or(Refused::NoSuchCapability)?;
        if slot.standing == Standing::Vacant {
            return Err(Refused::NoSuchCapability);
        }
        // **Older than the slot, and only older.** A generation below the
        // slot's named a capability that has been taken back; one at or above
        // it was never issued to anybody, and a caller presenting one is
        // guessing rather than holding something it has lost.
        // `kernel/src/cap.rs`'s `resolve_at` draws the line in exactly this
        // place — `revoked()` below the slot, `no_such()` above it — and a
        // module whose whole claim is that it answers what the frame would
        // answer does not get to draw it anywhere else.
        if handle.generation() < slot.generation {
            return Err(Refused::Revoked { held: handle.generation(), now: slot.generation });
        }
        // Asked after that and separately from it, for the reason
        // `Registry::apply` gives: a *reissued* slot is live, so the only thing
        // between the old holder's handle and the object is the generation —
        // and a revoked slot at the generation nothing has yet been issued at
        // is refused by this line rather than by its standing. One condition
        // doing both would pass both tests while holding one rule.
        if handle.generation() != slot.generation || slot.standing != Standing::Held {
            return Err(Refused::NoSuchCapability);
        }
        if slot.intent != intent {
            return Err(Refused::WrongIntent { names: slot.intent, asked: intent });
        }
        if !rights::holds(slot.rights, rights::WRITE) {
            return Err(Refused::RightNotHeld { held: slot.rights, asked: rights::WRITE });
        }
        Ok(intent)
    }
}

impl<const SLOTS: usize> Default for Authority<SLOTS> {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use f_interface::agent::{Acting, Unactable};
    use f_interface::canvas::{Arrangement, Escape, Extent, Placement, Selection, Ticks, TimeBase};
    use f_interface::node::{Content, Node, StateSet, Text};

    use super::*;

    // RFC 0078's timeline again, at the identities `interface/src/reader.rs`
    // and `interface/src/agent.rs` give it. **A third copy, and the reason is
    // the crate boundary rather than carelessness**: a test module cannot
    // import another crate's test fixtures, and the alternative — publishing
    // one — would put a timeline about door slams into the vocabulary every
    // application is asked to adopt. What must not drift is the declaration,
    // and what would notice is that this file selects the clip by *time and
    // lane* rather than by identity, so a fixture whose door slam moved would
    // stop selecting a clip at all.
    const SURFACE: NodeId = NodeId::new(400);
    const TRANSPORT: NodeId = NodeId::new(410);
    const PLAY: NodeId = NodeId::new(411);
    const SEQUENCE: NodeId = NodeId::new(420);
    const DIALOGUE: NodeId = NodeId::new(421);
    const AMBIENCE: NodeId = NodeId::new(422);
    const MUSIC: NodeId = NodeId::new(423);
    const BED: NodeId = NodeId::new(424);
    const FOLEY: NodeId = NodeId::new(425);
    const STEPS: NodeId = NodeId::new(426);
    const DOOR: NodeId = NodeId::new(427);
    const RAIN: NodeId = NodeId::new(428);
    const PLAYHEAD: NodeId = NodeId::new(429);

    /// The canvas's own rendering, never resolved anywhere here.
    const RENDERING: CapRef = CapRef::new(0x0400);
    /// What operating the door slam does. **The intent the exit is about.**
    const DOOR_INTENT: CapRef = CapRef::new(0x0409);
    /// What operating the rain would do, if the application were letting it.
    const RAIN_INTENT: CapRef = CapRef::new(0x040a);
    /// What operating the music bed does.
    const BED_INTENT: CapRef = CapRef::new(0x040b);
    /// What pressing play does.
    const PLAY_INTENT: CapRef = CapRef::new(0x0401);

    fn text(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits within TEXT_MAX"))
    }

    /// A time in tenths of a second. The scale is in the name for RFC 0004's
    /// reason: `42` is 4.2 s, and this tree has no floating-point value to
    /// write it any other way.
    fn seconds_x10(value: i64) -> Ticks {
        TimeBase::FLICKS.ticks_from_seconds(value, 1).expect("a time this base can name")
    }

    fn span_x10(from: i64, to: i64) -> Extent {
        Extent::new(seconds_x10(from), seconds_x10(to)).expect("a span that advances")
    }

    fn timeline() -> [Node; 13] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface).with_content(text("Sequence")),
            Node::new(TRANSPORT, SURFACE, Role::Group).with_content(text("Transport")),
            Node::new(PLAY, TRANSPORT, Role::Command)
                .with_content(text("Play"))
                .with_state(StateSet::ENABLED)
                .with_intent(PLAY_INTENT),
            Node::new(SEQUENCE, SURFACE, Role::Canvas)
                .with_content(Content::Media(RENDERING))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0402)),
            Node::new(DIALOGUE, SEQUENCE, Role::Track).with_content(text("Dialogue")),
            Node::new(AMBIENCE, DIALOGUE, Role::Clip).with_content(text("ambience")),
            Node::new(MUSIC, SEQUENCE, Role::Track).with_content(text("Music")),
            Node::new(BED, MUSIC, Role::Clip)
                .with_content(text("bed"))
                .with_state(StateSet::ENABLED)
                .with_intent(BED_INTENT),
            Node::new(FOLEY, SEQUENCE, Role::Track).with_content(text("Foley")),
            Node::new(STEPS, FOLEY, Role::Clip).with_content(text("steps")),
            Node::new(DOOR, FOLEY, Role::Clip)
                .with_content(text("door-slam"))
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(DOOR_INTENT),
            Node::new(RAIN, FOLEY, Role::Clip)
                .with_content(text("rain"))
                .with_state(StateSet::BUSY)
                .with_intent(RAIN_INTENT),
            Node::new(PLAYHEAD, SEQUENCE, Role::Marker).with_content(text("Playhead")),
        ]
    }

    fn arrangement() -> Arrangement {
        let placements = [
            Placement::over(AMBIENCE, span_x10(0, 20)),
            Placement::over(BED, span_x10(42, 68)),
            Placement::over(STEPS, span_x10(0, 42)),
            Placement::over(DOOR, span_x10(42, 68)),
            Placement::over(RAIN, span_x10(68, 100)),
            Placement::at(PLAYHEAD, seconds_x10(50)),
        ];
        let mut declared = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in placements {
            declared = declared.with_placement(placement).expect("within PLACED_MAX");
        }
        declared
    }

    /// The clip between 4.2 s and 6.8 s on track 3, selected the way an agent
    /// selects it: by the lane the application declared and the stretch of the
    /// dimension the application owns.
    fn door_slam<'a>(acting: &Acting<'a>) -> Selected {
        let lane = acting.lane(SEQUENCE, 3).expect("the canvas has three lanes");
        acting.between(lane, span_x10(42, 68)).expect("one clip spans it")
    }

    /// **The exit's first clause.** An agent selects that clip, invokes its
    /// intent, and the invocation is an authorised capability call that is
    /// logged.
    #[test]
    fn an_agent_invokes_the_clip_it_selected_and_the_call_is_logged() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);
        assert_eq!(selected.node(), DOOR, "the clip the exit is about");

        let mut authority = Authority::<4>::EMPTY;
        let handle = authority.issue(DOOR_INTENT, rights::WRITE).expect("a fresh table has a slot");
        let mut log = Log::<8>::EMPTY;

        let invoked = authority.invoke(&selected, handle, &mut log).expect("the agent may");
        assert_eq!(invoked.node, DOOR);
        assert_eq!(invoked.intent, DOOR_INTENT, "what the application named, not what was guessed");
        assert_eq!(invoked.handle, handle);
        assert_eq!(invoked.sequence, 1, "the first invocation this log has seen");

        // Logged, and logged with everything a reader auditing it would ask.
        assert_eq!(log.invoked(), 1);
        assert_eq!(log.refused(), 0);
        assert_eq!(log.lost(), 0);
        let record = *log.last().expect("the ring holds it");
        assert_eq!(
            record,
            Record {
                sequence: 1,
                node: DOOR,
                intent: Some(DOOR_INTENT),
                handle,
                outcome: Outcome::Invoked,
            },
        );
        // And the table was read rather than spent: a clip repeats, and
        // invoking it twice is two records and no change of authority.
        let again = authority.invoke(&selected, handle, &mut log).expect("still authorised");
        assert_eq!(again.sequence, 2);
        assert_eq!(log.invoked(), 2);
        assert_eq!(authority.generation(handle.index()), Handle::FIRST_GENERATION);
    }

    /// **The exit's second clause, and the substance of the task.** The same
    /// call, after the capability is revoked, is refused — `cargo xtask cap`'s
    /// `stale` in a new place.
    ///
    /// Nothing about the tree changes between the two calls. The same
    /// [`Selected`], the same [`Handle`], the same [`Authority`] value; one
    /// [`Authority::revoke`] between them. That is what makes the refusal about
    /// the capability and not about the declaration.
    #[test]
    fn the_same_call_after_the_capability_is_revoked_is_refused() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);

        let mut authority = Authority::<4>::EMPTY;
        let handle = authority.issue(DOOR_INTENT, rights::WRITE).expect("a slot");
        let mut log = Log::<8>::EMPTY;
        assert!(authority.invoke(&selected, handle, &mut log).is_ok(), "it worked a moment ago");

        assert!(authority.revoke(handle.index()), "there was a live capability to take");

        let refused = authority
            .invoke(&selected, handle, &mut log)
            .expect_err("the capability is gone and the call must be too");
        assert_eq!(refused, Refused::Revoked { held: Handle::FIRST_GENERATION, now: 2 });
        assert_eq!(
            refused.authority(),
            Some(f_abi::error::pack(f_abi::error::AUTHORITY, f_abi::error::authority::REVOKED)),
            "the refusal an agent is told is the refusal the frame would tell it",
        );

        // The tree still says the clip is invocable, which is the point: what
        // changed is who may, and nothing asked the application about it.
        assert_eq!(selected.operable(), Operable::Invocable(DOOR_INTENT));

        // And the refusal is logged like any other call. A log that recorded
        // only what succeeded would answer *nothing happened* for a component
        // that spent a boot presenting handles it no longer holds.
        assert_eq!(log.invoked(), 1);
        assert_eq!(log.refused(), 1);
        let record = *log.last().expect("the ring holds it");
        assert_eq!(record.node, DOOR);
        assert_eq!(record.handle, handle);
        assert_eq!(record.intent, Some(DOOR_INTENT));
        assert_eq!(
            record.outcome,
            Outcome::Refused(Refused::Revoked { held: Handle::FIRST_GENERATION, now: 2 }),
        );

        // Revoking twice is a `false` and not a second generation, so a caller
        // cannot walk a slot to retirement by asking.
        assert!(!authority.revoke(handle.index()));
        assert_eq!(authority.generation(handle.index()), 2);
    }

    /// The generation is what refuses, and not the slot being shut.
    ///
    /// `cargo xtask semantic`'s verdict makes this distinction about a tree and
    /// it is the same distinction here: after a reissue the slot is **live**, so
    /// a suite that only ever tested against a revoked slot would be testing
    /// that revoked slots refuse.
    #[test]
    fn a_reissued_slot_is_live_and_still_refuses_the_handle_from_before_it() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);

        let mut authority = Authority::<2>::EMPTY;
        let first = authority.issue(DOOR_INTENT, rights::WRITE).expect("a slot");
        assert!(authority.revoke(first.index()));
        let second = authority
            .reissue(first.index(), DOOR_INTENT, rights::WRITE)
            .expect("a revoked slot may be handed out again");
        assert_eq!(second.index(), first.index(), "the same slot");
        assert_ne!(second.generation(), first.generation(), "and not the same capability");

        let mut log = Log::<4>::EMPTY;
        assert!(authority.invoke(&selected, second, &mut log).is_ok(), "the successor may");
        assert_eq!(
            authority.invoke(&selected, first, &mut log),
            Err(Refused::Revoked { held: first.generation(), now: second.generation() }),
            "and the predecessor may not, against a slot that is open",
        );
        assert_eq!(log.invoked(), 1);
        assert_eq!(log.refused(), 1);
    }

    /// A handle that names no capability reaches nothing, three ways.
    ///
    /// `cap=unowned`, `cap=forge` and `cap=beyond` are three boots and one
    /// answer, and they are one answer here for the reason `registry.rs` gives
    /// about `Vacant`: *there is no capability here* is a different fact from
    /// *the one that was here is gone*, and a caller told the wrong one goes
    /// looking in the wrong place.
    #[test]
    fn a_handle_that_names_no_capability_reaches_nothing() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);

        let authority = Authority::<2>::EMPTY;
        let mut log = Log::<8>::EMPTY;
        // The zero word. It must not be answered `Revoked`, which is the
        // plausible wrong answer: slot zero is at generation zero, so a check
        // that compared generations first would agree with it.
        assert_eq!(
            authority.invoke(&selected, Handle::NULL, &mut log),
            Err(Refused::NoSuchCapability),
        );
        // A slot inside the table that nothing was issued at.
        assert_eq!(
            authority.invoke(&selected, Handle::new(1, Handle::FIRST_GENERATION), &mut log),
            Err(Refused::NoSuchCapability),
        );
        // And a slot past the end of it.
        assert_eq!(
            authority.invoke(&selected, Handle::new(9, Handle::FIRST_GENERATION), &mut log),
            Err(Refused::NoSuchCapability),
        );
        assert_eq!(log.refused(), 3, "every attempt is a record, including the invented ones");
        assert_eq!(log.invoked(), 0);

        // And a handle from the *future*, over a slot that is live. It is not a
        // capability anybody lost, so it is not `Revoked` — which is the
        // boundary `kernel/src/cap.rs`'s `resolve_at` draws and the one a
        // module claiming to answer as the frame does has to draw too.
        let mut issued = Authority::<2>::EMPTY;
        let handle = issued.issue(DOOR_INTENT, rights::WRITE).expect("a slot");
        assert_eq!(handle.generation(), Handle::FIRST_GENERATION);
        assert_eq!(
            issued.invoke(&selected, Handle::new(handle.index(), 5), &mut log),
            Err(Refused::NoSuchCapability),
            "a generation above the slot's was issued to nobody",
        );
        // And one from before a revoke, over the same slot, is the other side
        // of that line.
        assert!(issued.revoke(handle.index()));
        assert_eq!(
            issued.invoke(&selected, handle, &mut log),
            Err(Refused::Revoked { held: Handle::FIRST_GENERATION, now: 2 }),
        );
    }

    /// A live capability over the wrong object, and a live capability over the
    /// right one held too weakly.
    ///
    /// `cap=type` and `cap=rights`. The order matters and is asserted: a caller
    /// told *right not held* about a handle that names another intent would
    /// widen a grant it already has over the wrong thing.
    #[test]
    fn a_capability_over_another_intent_is_refused_before_its_rights_are_read() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);

        let mut authority = Authority::<4>::EMPTY;
        // A capability over the music bed, held with every right there is.
        let wrong = authority.issue(BED_INTENT, rights::ALL).expect("a slot");
        // And one over the door slam, held read-only.
        let weak = authority.issue(DOOR_INTENT, rights::READ).expect("a second slot");
        let mut log = Log::<8>::EMPTY;

        assert_eq!(
            authority.invoke(&selected, wrong, &mut log),
            Err(Refused::WrongIntent { names: BED_INTENT, asked: DOOR_INTENT }),
            "every right in the bitmap, over the wrong clip",
        );
        assert_eq!(
            authority.invoke(&selected, weak, &mut log),
            Err(Refused::RightNotHeld { held: rights::READ, asked: rights::WRITE }),
            "an agent that may read the tree and see the intent may not invoke it",
        );
        assert_eq!(log.refused(), 2);
    }

    /// What the application is not offering is not a statement about anybody's
    /// authority, and the table is not consulted to say so.
    #[test]
    fn what_the_application_is_not_offering_is_not_an_authority_refusal() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");

        let authority = Authority::<2>::EMPTY;
        let mut log = Log::<8>::EMPTY;

        // A clip the application declares busy. The handle is the zero word, so
        // an implementation that reached the table first would answer
        // `NoSuchCapability` and send the agent to ask for a grant instead of
        // to wait.
        let rain = acting.select(RAIN).expect("declared");
        let refused = authority.invoke(&rain, Handle::NULL, &mut log).expect_err("not now");
        assert_eq!(refused, Refused::NotNow(RAIN_INTENT));
        assert_eq!(refused.authority(), None, "nobody's authority is what is wrong here");

        // A clip that declares nothing to invoke at all.
        let steps = acting.select(STEPS).expect("declared");
        assert_eq!(authority.invoke(&steps, Handle::NULL, &mut log), Err(Refused::NothingDeclared));

        // And a role an agent does not invoke, whatever any node of it carries.
        let group = acting.select(TRANSPORT).expect("declared");
        let refused = authority.invoke(&group, Handle::NULL, &mut log).expect_err("never");
        assert_eq!(refused, Refused::NeverInvoked(Role::Group));
        assert_eq!(refused.authority(), None);

        assert_eq!(log.refused(), 3, "and all three are recorded");
        assert_eq!(
            log.records().filter(|record| record.intent.is_none()).count(),
            2,
            "the two with nothing to invoke name no intent, and the busy one names its own",
        );
    }

    /// The four authority refusals answer with the four codes the frame would.
    ///
    /// Listed together so that a fifth refusal added without a code is a diff
    /// that looks wrong, which is `abi/src/cap.rs`'s own argument about the same
    /// four values.
    #[test]
    fn the_four_authority_refusals_carry_the_codes_the_frame_would() {
        let expected = [
            (Refused::NoSuchCapability, f_abi::error::authority::NO_SUCH_CAP),
            (Refused::Revoked { held: 1, now: 2 }, f_abi::error::authority::REVOKED),
            (
                Refused::WrongIntent { names: BED_INTENT, asked: DOOR_INTENT },
                f_abi::error::authority::WRONG_TYPE,
            ),
            (
                Refused::RightNotHeld { held: rights::READ, asked: rights::WRITE },
                f_abi::error::authority::RIGHT_NOT_HELD,
            ),
        ];
        for (refusal, code) in expected {
            let packed = refusal.authority().expect("an authority refusal has a code");
            assert_eq!(packed, f_abi::error::pack(f_abi::error::AUTHORITY, code));
            assert_eq!(
                f_abi::error::unpack(packed),
                Some((f_abi::error::AUTHORITY, code)),
                "{refusal:?} does not survive the round trip",
            );
        }
        for projection in [
            Refused::NeverInvoked(Role::Label),
            Refused::NothingDeclared,
            Refused::NotNow(DOOR_INTENT),
        ] {
            assert_eq!(projection.authority(), None, "{projection:?} is not about a capability");
        }
    }

    /// A flood erases the detail and cannot erase the count.
    ///
    /// The module comment's asymmetry, as the thing it exists for: an agent that
    /// pushes the first refusal out of the ring still cannot make the log say
    /// fewer refusals happened, and [`Log::lost`] says the ring is a tail.
    #[test]
    fn a_flood_erases_the_detail_and_not_the_count() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);

        let mut authority = Authority::<2>::EMPTY;
        let handle = authority.issue(DOOR_INTENT, rights::WRITE).expect("a slot");
        let mut log = Log::<2>::EMPTY;

        // The record worth keeping, first.
        authority.invoke(&selected, handle, &mut log).expect("authorised");
        // Then four invented handles, which is all it takes to push it out.
        for slot in 0..4 {
            assert!(authority.invoke(&selected, Handle::new(slot, 9), &mut log).is_err());
        }

        assert_eq!(log.written(), 5);
        assert_eq!(log.invoked(), 1, "the count of what was authorised did not move");
        assert_eq!(log.refused(), 4);
        assert_eq!(log.lost(), 3, "and the ring says how much of the history it is missing");
        assert_eq!(log.records().count(), 2);
        assert!(
            log.records().all(|record| matches!(record.outcome, Outcome::Refused(_))),
            "the authorised call is the detail the flood erased, which is what the counts are for",
        );
        assert_eq!(log.records().next().expect("two are held").sequence, 4);

        // A log with no room at all is the degenerate case of the same rule:
        // every record is lost and no count is.
        let mut none = Log::<0>::EMPTY;
        authority.invoke(&selected, handle, &mut none).expect("authorised");
        assert_eq!(none.invoked(), 1);
        assert_eq!(none.lost(), 1);
        assert_eq!(none.records().count(), 0);
        assert!(none.last().is_none());
    }

    /// Reading an intent buys nothing towards invoking it.
    ///
    /// The crate's own rule between an address and a right, one layer down: a
    /// projection hands out `Operable::Invocable(intent)` to anybody who can
    /// read the tree, and there is no call in this module that turns that value
    /// into a [`Handle`]. The evidence is a compile-time absence, so what is
    /// asserted here is the runtime half of it — the intent, presented with
    /// every handle a caller could construct without being issued one.
    #[test]
    fn seeing_an_intent_buys_nothing_towards_invoking_it() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");
        let selected = door_slam(&acting);
        let Operable::Invocable(seen) = selected.operable() else { panic!("the clip is offered") };
        assert_eq!(seen, DOOR_INTENT, "the agent can read what operating it would do");

        // A table that has issued nothing. The `CapRef` is in hand and every
        // handle over the table is a guess.
        let authority = Authority::<4>::EMPTY;
        let mut log = Log::<8>::EMPTY;
        for slot in 0..4u16 {
            for generation in [Handle::FIRST_GENERATION, 2, u16::MAX] {
                assert_eq!(
                    authority.invoke(&selected, Handle::new(slot, generation), &mut log),
                    Err(Refused::NoSuchCapability),
                );
            }
        }
        assert_eq!(log.refused(), 12);
        assert_eq!(log.invoked(), 0);
    }

    /// The questions an agent asks that have no single answer are the
    /// projection's to refuse, and they never reach a capability at all.
    #[test]
    fn a_selection_that_was_never_made_cannot_be_invoked() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");

        // There is no fourth lane, and no `Selected` comes out of asking for
        // one — which is the whole of why `invoke` takes one.
        assert_eq!(
            acting.lane(SEQUENCE, 4),
            Err(Unactable::NoSuchLane { canvas: SEQUENCE, nth: 4, of: 3 }),
        );
        assert_eq!(
            acting.between(DOOR, span_x10(42, 68)),
            Err(Unactable::Canvas(Escape::NotALane(DOOR))),
        );
    }
}
