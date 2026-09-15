// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The semantic entry format: six edits, one handshake, and the ordinal that is
//! refused rather than mapped.
//!
//! # What crosses, and what carries it
//!
//! `docs/design/ring-scene-boot.html` section 11 says an application declares
//! what its interface *is* and the system decides what that looks like, and
//! that the declaration crosses *on an ordinary ring, using the ordinary
//! envelope from part I*. So there is no second transport here and no second
//! framing: a semantic edit is an [`Sqe`] whose opcode names the edit and a
//! fixed-width record in the channel's inline arena that the entry points at.
//! [`layout`](crate::layout) is where the arena is; this module is what is in
//! it.
//!
//! [`scene`](crate::scene) is the sibling this format is deliberately shaped
//! against — same envelope, same stride discipline, same two halves of *an
//! unread field is refused*. Two entry formats in one crate that disagreed
//! about how a non-zero unread field is refused would be one rule with two
//! implementations, which is the defect R04 exists to prevent rather than an
//! instance of following it. Where this format diverges it says so: it carries
//! no deadline at all, and it opens with a handshake.
//!
//! # The problem this module exists for
//!
//! RFC 0077 closed the semantic vocabulary: twenty-two roles, no `Other`, no
//! `Custom`, no reserved range, no `#[non_exhaustive]`. What buys that
//! closedness is the compiler — a twenty-third role is a compile error in every
//! projection, so the person who must decide what a new role looks like in a
//! screen reader is told by their build rather than by a user. RFC 0077 then
//! wrote down, as its own fourth reversal condition, the place where that
//! mechanism has no purchase: **the compiler is not in the loop across a trust
//! boundary.** Two builds of this system will be six months apart, both in the
//! field, both conforming, and one of them will hand the other an ordinal the
//! other has never heard of.
//!
//! RFC 0083 is the decision about what happens then, and this module is its
//! `abi` half. Three parts, of which the third is the one written to survive an
//! edit:
//!
//! 1. A channel that carries a semantic tree agrees a **vocabulary version**
//!    before it carries anything else, in RFC 0011's shape and **in band** —
//!    [`op::DECLARE_VOCABULARY`] must be the first entry on the channel and may
//!    appear once per channel epoch. Nothing in [`ChannelHeader`](crate::ChannelHeader) changes, and
//!    that is part of the decision rather than an accident of scope: a field
//!    that is meaningless on a block-device channel and must be zero there is
//!    the shape R04 exists to refuse.
//! 2. The vocabulary's indices are **append-only**, which is what makes a
//!    version ordinal identify a list rather than merely label one. The failure
//!    that ends systems is not the unknown ordinal — that one is loud — it is
//!    the *re-meant* ordinal: both sides say version 1, both are telling the
//!    truth, and 14 is `Toggle` on one and `Command` on the other. Nothing is
//!    unknown, every entry decodes, every projection is total, and a settings
//!    panel is announced wrongly forever.
//! 3. An ordinal outside the agreed vocabulary is **refused, and it is refused
//!    by not being representable**. There is no fallback role, no clamp, no
//!    skip, and no [`Refusal`] a caller can turn into a value.
//!
//! # Where the ordinal is allowed to exist, and why it is not here
//!
//! `interface/` takes no dependencies on purpose, so that the semantic layer
//! can be abandoned without taking anything else down; this crate sits beneath
//! the frame and cannot depend on a layer above it. Neither crate can see the
//! other. So the decode lives in neither: this module carries the entry format
//! and the protocol constants, `interface/src/node.rs` carries the list and the
//! functions derived from it, and the join is made by the component that owns
//! the tree.
//!
//! The reviewer's question about a vocabulary written in two places is RFC
//! 0077's recorded defect — a second place a role can be written — arriving
//! with a version number on it. It is not, and the reason is structural rather
//! than asserted: **this module never enumerates roles.** No count, no name, no
//! table, no `match`. It does not know that there are twenty-two, and there is
//! no edit to this file that could make it disagree with the list, because
//! there is nothing here to disagree with.
//!
//! What it holds instead is one predicate it asks somebody else:
//! [`Vocabulary::names`]. An ordinal becomes a [`RoleOrdinal`] only by passing
//! it, and [`RoleOrdinal`] has no other constructor outside this module — so a
//! role ordinal the agreed vocabulary did not name is a value that does not
//! exist anywhere in the system, rather than a value every projection has to
//! carry an arm for. That is the difference between refusing something and
//! guarding against it.
//!
//! The same rule covers every closed enum that crosses, not just roles:
//! [`Closed`] is the list of them, and one method answers for all. A per-enum
//! policy is a table of judgements the next enum's author has to guess at.
//!
//! # What a wildcard arm would have cost, stated once
//!
//! The obvious answer — decode an unknown ordinal onto a neutral role, or hand
//! it to the projections as a number and let each decide — is the one every
//! predecessor shipped, and three separate things are wrong with it. It
//! **succeeds**, so the sender is told nothing and its author believes a
//! control is present that no user can operate. It is **total over the
//! unknown**, so the vocabulary is open in practice the moment it exists:
//! nobody needs an RFC to ship a role, they need a peer that ships one. And it
//! **decides** — four projections each guess at what a role nobody defined
//! should look like, and the screen reader's guess and the display's guess
//! disagree. Whether the grey box is produced by a `_` arm in a projection or
//! by an `unwrap_or` in a decoder is a question about which file it lives in,
//! not about what it does.
//!
//! # The granularity of a refusal is the frame
//!
//! Section 11 makes the semantic protocol atomic per commit, so the unit that
//! already exists is the unit that is refused. [`Session`] is where that lives:
//! the offending entry is reported by its position and its ordinal, the pending
//! edit is **poisoned**, every later entry of that frame is refused with the
//! same [`Fault`] so that a caller cannot apply one, the [`op::COMMIT`] that
//! would have closed the frame fails with it, and the tree the receiver is
//! already presenting stands unchanged.
//!
//! Not the entry alone. The entries are not independent: a refused
//! [`op::DECLARE_NODE`] takes its children with it, so *refuse one entry* is
//! never local — it is a cascade whose extent the sender cannot predict, and
//! what the receiver would present is an interface with a hole in it. A hole is
//! worse than a grey box, because a grey box is visibly wrong and a settings
//! panel silently missing its third toggle is a screenshot nobody questions.
//!
//! Not the channel either. After a successful negotiation an out-of-range
//! ordinal is a peer that has broken its word, which sounds like a
//! [`error::PEER`] condition — but a receiver cannot tell a lying peer from a
//! buggy one from a proxy that mangled a field, and the cost of guessing wrong
//! is destroying a working component's interface. [`error::ARGUMENT`], per
//! frame, keeps the refusal proportionate and puts the evidence in the sender's
//! own completions, where its author is the one who sees it.
//!
//! # No deadline, and no field to put one in
//!
//! [`scene`](crate::scene)'s commit reads [`Sqe::deadline`] because a frame is
//! paced against a display. A semantic commit is not: the same tree is
//! presented at once to a display, a screen reader and an agent, and those
//! three present it at three different moments by construction. A deadline here
//! would be a second answer to *when is this shown* in a system where no single
//! answer is even meaningful.
//!
//! So [`Delta`] has no deadline field. Not a field that is checked against
//! zero — no field. [`Delta::envelope`] writes [`NO_DEADLINE`], the
//! whole-envelope comparison in [`Delta::decode`] refuses any entry that
//! carries one, and the refusal is [`Refusal::Reserved`] because that is
//! exactly what it is: a field this opcode does not read. An eighth opcode that
//! genuinely schedules something has to add the field to [`Delta`] and to
//! [`Delta::envelope`], which is a diff a reviewer reads rather than a column
//! of `false` nobody does.
//!
//! # An unread field is refused, and it is refused structurally
//!
//! [`scene`](crate::scene)'s rule, unchanged and for its reason, so that the
//! two formats cannot drift:
//!
//! - **The payload.** The `Reader` a record decodes through counts what that
//!   decoder consumed, and `Reader::finish` requires every byte past it to be
//!   zero. A field a record does not read is a field in the tail, and the tail
//!   is refused. [`SetRelations`] is the record that shows what this buys: it
//!   reads exactly as many edge slots as its count declares, so the unused
//!   slots *are* the tail and a producer that left stale bytes in one is
//!   refused by the rule that was already there rather than by a check somebody
//!   remembered to write.
//! - **The envelope.** [`Delta::decode`] rebuilds the [`Sqe`] this build would
//!   have written from the fields it actually read, and compares all
//!   sixty-four bytes. Any field of [`Sqe`] this format does not set — `cap`,
//!   `deadline`, `buf_set`, `buf_index`, `_reserved`, `ext`, and anything a
//!   later ABI adds — must arrive zero, because the canonical entry has it
//!   zero.
//!
//! # No clock, no floating point, no allocator
//!
//! Nothing here reads a clock, draws randomness or observes an ordering: every
//! function in this module is a pure function of the bytes and the vocabulary
//! it is handed, and the vocabulary is a parameter rather than a global for
//! that reason — a pure function of what it is given is more deterministic than
//! one that consults something. RFC 0004. Nothing here is binary floating point
//! either; the one quantity the semantic layer carries is a scaled integer with
//! its scale beside it, and it travels in the arena rather than in a payload.
//!
//! # An eighth opcode
//!
//! It is an RFC, by the rule that makes every change to this crate an ABI
//! change, and it is also a diff to one list: the `entries!` invocation below
//! emits the opcode constants, the [`Entry`] variants, the dispatch, the answer
//! to *is this the handshake*, and the specimens the round-trip test runs over.
//! There is no second place an opcode could be written and no way to add one
//! the round-trip test does not pick up.
//!
//! The opcode space is the semantic service's own. Section 05 makes an opcode
//! space per-service, so these numbers are not [`crate::op`]'s, not
//! [`control::op`](crate::control::op)'s and not
//! [`scene::op`](crate::scene::op)'s; nothing should compare an opcode across
//! two of them.

use crate::{NO_DEADLINE, Sqe, error, flags};

/// Bytes of inline arena one semantic entry's payload occupies, whatever its
/// opcode.
///
/// The widest record is [`SetRelations`], and this is the next multiple of
/// eight above it. Stated as its own constant rather than borrowed from
/// [`scene::PAYLOAD_BYTES`](crate::scene::PAYLOAD_BYTES), which happens to hold
/// the same number today: a format that inherited another format's stride would
/// change silently the day that one grew a field, and the two are not one
/// decision.
/// Unit: bytes.
pub const PAYLOAD_BYTES: usize = 56;

/// The width of a submission entry, which [`Delta::decode`] compares in full.
///
/// Stated here as well as in `lib.rs` because the comparison depends on it and
/// on there being no padding inside an [`Sqe`]; the assertions further down
/// this file are what make both facts rather than hopes.
/// Unit: bytes.
const SQE_BYTES: usize = 64;

/// No node. Zero, so that a zeroed payload names nothing.
///
/// `interface`'s `NodeId::UNNAMED` is zero for the same reason and this is the
/// same value: RFC 0077 says no relation may carry it, and RFC 0077's third
/// refusal says no node may hold it as its own identity. Here it is also the
/// legal value of [`DeclareNode::parent`] for a root and of
/// [`DeclareNode::before`] for a node appended last, which is why it is a named
/// constant rather than a sentinel a reader has to recognise.
/// Unit: none — a node identifier, not a quantity.
pub const NO_NODE: u64 = 0;

/// A node that is not operable.
///
/// RFC 0077 spells *read-only* as the absence of an intent rather than as a
/// second field, because a fact stated twice is a fact that can disagree with
/// itself. `interface`'s `CapRef` is a `u32` so that the day the tree crosses a
/// ring the translation is a rename rather than a re-encoding; this is that
/// day, and this is the rename.
/// Unit: none — a capability reference in the submitter's own space.
pub const NO_INTENT: u32 = 0;

/// How many edges one [`SetRelations`] entry carries.
///
/// A second statement of `interface`'s `RELATIONS_MAX`, and a deliberate one:
/// the wire has to state its own capacity, because the stride is arithmetic a
/// consumer does before it believes any field in the arena. The two are held
/// together by `the_wire_carries_as_many_relations_as_a_node_has`, which reads
/// `interface/src/node.rs` at compile time — so the copy cannot drift quietly,
/// only loudly.
///
/// *What would reverse this:* a node that may hold more relations than fit one
/// payload. At that point a node's relation set stops being one entry, and the
/// atomicity a single entry gives it — a set is replaced or it is not — has to
/// be rebuilt out of the commit instead. That is an ABI change, and this is the
/// right place to notice it.
/// Unit: edges.
pub const RELATIONS_MAX: usize = 4;

/// The submission flags a semantic entry may carry.
///
/// [`flags::NO_CQE`] and nothing else, for
/// [`scene::FLAGS_ACCEPTED`](crate::scene::FLAGS_ACCEPTED)'s reasons. A client
/// declaring a hundred nodes does not want a hundred completions it will never
/// read, and RFC 0028's asymmetry still holds: a *refused* entry completes
/// whatever this flag says, so suppressing the success completion costs the
/// client nothing it needs — which matters more here than it does for a scene,
/// because here the refusal is the whole protocol.
///
/// [`flags::LINK`] and [`flags::DRAIN`] are refused rather than honoured,
/// because ordering inside a frame is already decided: the ring's order is the
/// order, and the commit is the barrier. [`flags::FIXED_BUF`] is refused
/// because the payload is in the arena at [`Delta::payload_offset`].
/// Unit: none — a bitmask of the [`flags`] constants.
pub const FLAGS_ACCEPTED: u8 = flags::NO_CQE;

/// The highest vocabulary version this build speaks.
///
/// One. RFC 0077 froze version 1's list; RFC 0083 made the indices
/// append-only, so a later version is this list with roles added to the end and
/// never a list with roles moved within it.
///
/// This number can drift from the list it names, in either direction, and both
/// directions are safe — which is why nothing here depends on a lint keeping
/// them level. Ahead of the list: version *n* and version *n − 1* have
/// identical role sets, everything that decoded before decodes now, and nothing
/// new is admitted. Behind the list: a role appended with a `since` above this
/// is a role the agreed version does not name, and a node carrying it is
/// refused at the boundary — which is the correct treatment of a role that has
/// not been released.
/// Unit: none — a vocabulary version ordinal. Zero is not a version.
pub const VOCABULARY_VERSION: u16 = 1;

/// The oldest vocabulary version this build still speaks.
///
/// [`ChannelHeader::abi_version_min`](crate::ChannelHeader::abi_version_min)'s argument one level down: setup meets in
/// the middle rather than demanding equality, which is what makes a component
/// updatable independently of the projections that present it. Raising this is
/// a decision about which deployed peers get dropped, and RFC 0083 calls that
/// raising the floor.
/// Unit: none — a vocabulary version ordinal, and never above
/// [`VOCABULARY_VERSION`].
pub const VOCABULARY_VERSION_MIN: u16 = 1;

/// Why an entry was not believed.
///
/// This module's own enum, in the shape
/// [`scene::Refusal`](crate::scene::Refusal) and
/// [`manifest::Refusal`](crate::manifest::Refusal) already use, packing into
/// the domains RFC 0010 fixes without adding a code to [`error::argument`].
/// Adding one was the obvious move and is the wrong one: sibling entry formats
/// are being written against the same model, and a new `argument` code invented
/// in each of four places is four meanings wearing one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The opcode is not one of this service's seven.
    UnknownOpcode,
    /// The entry carries a submission flag outside [`FLAGS_ACCEPTED`].
    UnknownFlag,
    /// A field this opcode does not read is not zero: anywhere in the payload
    /// past what the record consumed, or anywhere in the envelope this format
    /// never looks at — including a deadline, which no semantic opcode reads.
    Reserved,
    /// The entry does not frame a payload: a length that is not
    /// [`PAYLOAD_BYTES`], an arena offset that is not an arena offset, or a
    /// handshake whose floor is above its ceiling.
    Malformed,
    /// A closed field carries a value outside its set for a reason the
    /// vocabulary has no say in — a node named as its own parent, an edge that
    /// leaves and arrives at one node, or a count past [`RELATIONS_MAX`].
    Value,
    /// A field that must name a node holds [`NO_NODE`]. Separate from
    /// [`Refusal::Value`] because a zeroed payload produces exactly this, and a
    /// caller chasing a producer that forgot to fill a record wants to be told
    /// that rather than *some field is wrong*.
    NoNode,
    /// The agreed vocabulary does not name this ordinal.
    ///
    /// The refusal this module exists for, and the one that is carried rather
    /// than mapped. It reports the ordinal because RFC 0083 asks that the
    /// offending entry be reported *by its position and its ordinal*: the
    /// position is [`Fault::at`], and this is the other half. A sender that
    /// agreed a version below the one it was built against is meant to learn
    /// from this, in its own completions, rather than at a user's desk.
    Unnamed {
        /// Which closed enum the ordinal was read as.
        field: Closed,
        /// The value the agreed vocabulary does not name.
        /// Unit: none — an ordinal in `field`'s declaration order.
        ordinal: u16,
    },
    /// A state bit outside the set the agreed version names.
    ///
    /// `interface`'s `StateSet::from_bits` keeps every bit it is given rather
    /// than masking against `KNOWN`, deliberately, because the bits are the
    /// evidence that the two sides disagree. This is the layer holding that
    /// evidence deciding what to do with it, and the answer is the one
    /// [`Refusal::Unnamed`] gives: refuse and report, never tidy away.
    UnknownState {
        /// Every bit of the arriving set, including the ones this build does
        /// understand — the evidence, not the residue.
        /// Unit: none — a bitmask in the vocabulary's own bit order.
        bits: u8,
    },
    /// A semantic entry arrived before [`op::DECLARE_VOCABULARY`].
    ///
    /// Not a courtesy. Until a version is agreed, an ordinal is a number nobody
    /// has said the meaning of, and a receiver that read one anyway would be
    /// choosing a meaning — which is the wildcard arm arriving before the tree
    /// does.
    NotNegotiated,
    /// A second [`op::DECLARE_VOCABULARY`] within one channel epoch.
    ///
    /// Re-negotiating mid-stream would change what an index means underneath a
    /// tree that was already built out of the old meaning, and there is no
    /// entry in this format that could say which nodes were built under which
    /// agreement. The epoch is the unit that may agree again, because moving it
    /// already discards every outstanding token.
    Renegotiated,
    /// The two sides' vocabulary version ranges do not overlap.
    ///
    /// The one refusal here that is not [`error::ARGUMENT`]: the peer is
    /// present and the channel is healthy, but there is no version of the
    /// vocabulary both speak, which is what
    /// [`error::peer::VERSION_UNSUPPORTED`] already means one level up. RFC
    /// 0083 reuses it rather than minting a second code for one sentence.
    VersionUnsupported,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownOpcode => "the opcode is not one of this service's seven",
            Self::UnknownFlag => "the entry carries a submission flag a semantic entry may not",
            Self::Reserved => "a field this opcode does not read is not zero",
            Self::Malformed => "the entry does not frame a payload of the one width there is",
            Self::Value => "a closed field carries a value outside its set",
            Self::NoNode => "a field that must name a node names none",
            Self::Unnamed { .. } => "the agreed vocabulary does not name this ordinal",
            Self::UnknownState { .. } => "the agreed vocabulary does not name one of these states",
            Self::NotNegotiated => "a semantic entry arrived before the vocabulary was agreed",
            Self::Renegotiated => "the vocabulary was agreed twice in one channel epoch",
            Self::VersionUnsupported => "no vocabulary version is spoken by both sides",
        }
    }

    /// The refusal as a packed [`error`], for a completion.
    ///
    /// Every one but [`Refusal::VersionUnsupported`] is [`error::ARGUMENT`],
    /// which is RFC 0083's granularity decision written as a code: the channel
    /// is healthy and the peer is present, and what is wrong is one frame.
    /// [`error::argument::FEATURE_NOT_NEGOTIATED`] is the worked precedent the
    /// RFC names — it sits in `ARGUMENT` rather than `PEER` for exactly this
    /// reason — so the two handshake-order refusals reuse it.
    ///
    /// No code here is new. [`Refusal::Value`], [`Refusal::NoNode`],
    /// [`Refusal::Unnamed`] and [`Refusal::UnknownState`] share
    /// [`error::argument::UNKNOWN_FLAG`], which
    /// [`scene::Refusal`](crate::scene::Refusal),
    /// [`manifest::Refusal`](crate::manifest::Refusal) and
    /// [`store::refusal`](crate::store::refusal) already use for *a closed
    /// field carries a value this build does not accept*. They are
    /// distinguishable as values, which is where a caller distinguishes them;
    /// they are one code on the wire, which is where RFC 0010 says the domain
    /// is the stable part.
    #[must_use]
    pub const fn packed(self) -> i32 {
        match self {
            Self::VersionUnsupported => error::pack(error::PEER, error::peer::VERSION_UNSUPPORTED),
            Self::UnknownOpcode => error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
            Self::Reserved => error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
            Self::Malformed => error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
            Self::NotNegotiated | Self::Renegotiated => {
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED)
            }
            Self::UnknownFlag
            | Self::Value
            | Self::NoNode
            | Self::Unnamed { .. }
            | Self::UnknownState { .. } => {
                error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG)
            }
        }
    }
}

/// What this build and its peer agreed the vocabulary's ordinals mean.
///
/// There is one way to obtain one — [`Handshake::negotiate`] — because the
/// field is private and this module mints none anywhere else. So a function
/// that takes an `Agreed` cannot be called on a channel where no version was
/// agreed, and *the handshake happened* is a fact the type system carries
/// rather than a comment asking a caller to remember.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Agreed(u16);

impl Agreed {
    /// The version both sides speak.
    ///
    /// Handed out because the vocabulary needs it — a role's `since` is
    /// compared against it — and for no other purpose. Nothing in this module
    /// branches on it.
    /// Unit: none — a vocabulary version ordinal.
    #[must_use]
    pub const fn version(self) -> u16 {
        self.0
    }

    /// Admit a state bitmask, or refuse it.
    ///
    /// A mask rather than an ordinal, because a state is a bit in a word and a
    /// role is a position in a list. Asking [`Vocabulary`] to answer for one
    /// bit at a time would make the caller do arithmetic to answer a question
    /// its own `KNOWN` constant already answers exactly.
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownState`], carrying every arriving bit rather than only
    /// the offending ones, because the whole set is the evidence that the two
    /// builds hold different vocabularies.
    pub fn admit_state(self, bits: u8, vocabulary: &dyn Vocabulary) -> Result<StateBits, Refusal> {
        if bits & !vocabulary.state_bits(self) == 0 {
            Ok(StateBits(bits))
        } else {
            Err(Refusal::UnknownState { bits })
        }
    }
}

/// A set of states the agreed vocabulary names, every bit of it.
///
/// [`StateBits`] has no constructor outside this module either, for
/// [`RoleOrdinal`]'s reason: a set carrying a bit nobody agreed the meaning of
/// is a value this system does not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateBits(u8);

impl StateBits {
    /// The bits, for the one function that turns them into a state set.
    /// Unit: none — a bitmask in the vocabulary's own bit order.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The list this build's peer holds, answered one question at a time.
///
/// # Why a predicate and not a table
///
/// Because a table here would be the second place a role is written, which is
/// the defect RFC 0077 spent two adversarial reviews closing inside
/// `interface/`. This crate cannot see `interface/`, and the answer is not to
/// copy the list across the gap but to hold none: the implementor is the
/// component that owns the tree, its answer is derived from the single
/// `vocabulary!` list — `ALL.iter().filter(|r| r.since() <= agreed)` — and
/// nothing in this module could disagree with a list it does not have.
///
/// # What an implementor owes
///
/// - **Derive the answer, never write it.** An implementation that matches on a
///   hand-written range of ordinals is the copied table, wearing a method.
/// - **Answer `false` and nothing else.** There is no return path here for *I
///   do not know this one, use that one instead*, and that absence is the whole
///   design: see the module's *what a wildcard arm would have cost*.
/// - **Be a pure function of `(field, ordinal, agreed)`.** No clock, no
///   randomness, no state a second call could change. RFC 0004, and also the
///   reason this is a parameter rather than a global: a decoder that consulted
///   something could be made to decode one payload two ways.
///
/// # Why an implementor cannot be policed here
///
/// Stated rather than left for a reviewer to find: an implementation that
/// answers `true` for every ordinal reopens the vocabulary, and nothing in this
/// module can detect that. What this module guarantees is narrower and is what
/// the exit asks for — *this* crate never maps an ordinal onto anything, never
/// supplies one it was not given, and has no fallback to fall back to. The
/// list's own closedness is `interface/`'s to keep, and RFC 0077 is where it is
/// kept.
pub trait Vocabulary {
    /// Does version `agreed` of the vocabulary name `ordinal` as a value of
    /// `field`?
    fn names(&self, field: Closed, ordinal: u16, agreed: Agreed) -> bool;

    /// Every state bit version `agreed` of the vocabulary names.
    /// Unit: none — a bitmask in the vocabulary's own bit order.
    fn state_bits(&self, agreed: Agreed) -> u8;
}

/// Emits the closed fields, their admitted ordinals, and the one admission
/// rule, from one list.
///
/// # Why a macro, in a crate that has almost none
///
/// Because the alternative is four sequences that have to agree: the [`Closed`]
/// variants, the newtype per field, the [`Agreed`] method that mints it, and
/// the refusal that names it. `interface/src/node.rs`'s `vocabulary!` was
/// written after two adversarial reviews found a role that was in an enum and
/// not in an array, and the second review found the hole still open under the
/// test that was supposed to have closed it — because a loop over a list cannot
/// see what the list omits, and an exhaustive match demands an arm rather than
/// an arm that says anything.
///
/// A closed field added here without its newtype would be a field with no
/// admission, which is a field whose ordinal reaches a consumer unchecked. That
/// is precisely the hole this module exists to have none of, so it is made
/// unwritable rather than tested for.
macro_rules! admitted {
    ( $( $variant:ident / $newtype:ident / $method:ident, $about:literal; )* ) => {
        /// A closed enum of the semantic vocabulary, as it crosses: an ordinal
        /// in the declaring list's own order.
        ///
        /// Not the values — this module holds none of those and could not name
        /// one. This is the list of *kinds of field* whose values are ordinals,
        /// so that one admission rule can cover all of them. RFC 0083 asks for
        /// exactly that uniformity, because a per-enum policy is a table of
        /// judgements the next enum's author has to guess at.
        ///
        /// `Unit` and `Flow` are closed enums of the same vocabulary and are
        /// absent, because no entry in this format carries one: a quantity's
        /// unit travels in the arena with the quantity, and a node's flow
        /// belongs to a layout entry this format does not yet have. They join
        /// this list on the day an entry carries one, which is the day the line
        /// declaring that entry is written.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Closed {
            $(
                #[doc = $about]
                $variant,
            )*
        }

        impl Closed {
            /// How many closed fields cross in this format.
            ///
            /// Counted from the list rather than written down, so it cannot
            /// disagree with what it counts.
            /// Unit: closed fields.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every closed field, in declaration order.
            ///
            /// Emitted from the same list as the variants, so it holds every
            /// one there is — not because a test checks it, but because there
            /// is no way to write one this array does not get.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// A word for a log or a completion trace.
            #[must_use]
            pub const fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)*
                }
            }
        }

        $(
            #[doc = $about]
            ///
            /// An ordinal the agreed vocabulary **named**. The tuple field is
            /// private and this module mints one in exactly one place, so a
            /// value of this type the vocabulary did not name cannot be
            /// constructed — not by a decoder here, not by a producer
            /// elsewhere, and not by a projection that wanted a neutral answer.
            /// That is the whole of *refused rather than mapped to anything*:
            /// there is no fallback because there is nothing to fall back to.
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct $newtype(u16);

            impl $newtype {
                /// The ordinal, for the one function that turns it into a value
                /// of the closed enum.
                ///
                /// That function is `interface`'s, it is derived from the
                /// single list the way `from_name` already is, and it is the
                /// only place an ordinal becomes a value. RFC 0083's *a second
                /// decoder* is the reversal condition on that, and it applies
                /// to a snapshot read back and to a simulator replay as much as
                /// to this wire.
                /// Unit: none — an ordinal in the declaring list's own order.
                #[must_use]
                pub const fn get(self) -> u16 {
                    self.0
                }

                /// Which closed field this ordinal belongs to.
                ///
                /// A role ordinal and a relation ordinal are both small
                /// integers meaning entirely different things; separate types
                /// are what stop one being read as the other, and this is how a
                /// log line says which it was holding.
                #[must_use]
                pub const fn field(self) -> Closed {
                    Closed::$variant
                }
            }
        )*

        impl Agreed {
            $(
                #[doc = $about]
                ///
                /// The admission, and the only one. `vocabulary` answers from
                /// the single declaring list; a `false` is a refusal and never
                /// a substitution.
                ///
                /// # Errors
                ///
                /// [`Refusal::Unnamed`], carrying the field and the ordinal, so
                /// that the sender's completion says which value of which
                /// closed enum its build has and this one does not.
                pub fn $method(
                    self,
                    ordinal: u16,
                    vocabulary: &dyn Vocabulary,
                ) -> Result<$newtype, Refusal> {
                    if vocabulary.names(Closed::$variant, ordinal, self) {
                        Ok($newtype(ordinal))
                    } else {
                        Err(Refusal::Unnamed { field: Closed::$variant, ordinal })
                    }
                }
            )*
        }
    };
}

admitted! {
    Role / RoleOrdinal / admit_role,
        "What a node *is*: an index into the closed vocabulary RFC 0077 froze.";
    Relation / RelationOrdinal / admit_relation,
        "An edge the tree's shape does not already say: labelled-by, controls, described-by.";
    Content / ContentOrdinal / admit_content,
        "Which kind of thing a node holds: text, a quantity, media, or nothing.";
}

/// The first entry on a semantic channel, and the only one that may be.
///
/// RFC 0011's shape one level down: each side states the highest version it
/// speaks and the oldest it still speaks, and the agreement is the highest
/// version both reach. It is in band — an entry rather than a [`ChannelHeader`](crate::ChannelHeader)
/// field — and that is a decision rather than an accident of scope. The header
/// is sixty-four bytes with four reserved words for the entire system, and a
/// field that is meaningless on a block-device channel and must be zero there
/// is the shape R04 exists to refuse; worse, the frame would have to know the
/// semantic vocabulary's version in order to open a channel it does not
/// otherwise participate in, which wires `interface/`'s number into the frame's
/// core type and contradicts that crate's deliberate independence. In band, it
/// costs one opcode in one service's own opcode space, which is the cheapest
/// change to shared state available, which is none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handshake {
    /// The highest vocabulary version the writer speaks.
    /// Unit: none — a vocabulary version ordinal. Zero is not a version and is
    /// refused rather than read as *any*.
    pub highest: u16,
    /// The oldest vocabulary version the writer still speaks.
    /// Unit: none — a vocabulary version ordinal, and never above `highest`.
    pub floor: u16,
}

impl Handshake {
    /// What this build offers.
    ///
    /// Derived from the two constants rather than written a third time, so a
    /// floor raised in one place is raised on the wire.
    pub const HERE: Self = Self { highest: VOCABULARY_VERSION, floor: VOCABULARY_VERSION_MIN };

    /// Agree a version with the peer that wrote this handshake.
    ///
    /// The intersection, at the highest version both sides can speak. Nothing
    /// here is permissive: [`ChannelHeader::negotiate`](crate::ChannelHeader::negotiate)'s own sentence is that
    /// *negotiation makes compatibility explicit; it does not make the wire
    /// permissive*, and the boundary refusals below a successful agreement are
    /// what hold against a peer that negotiated and then broke its word. A
    /// hostile peer writes 200 into the role field regardless of what it said
    /// at setup, and this handshake is not what stops it.
    ///
    /// # Errors
    ///
    /// [`Refusal::Malformed`] for a floor above the ceiling, which is a
    /// statement no honest build makes; [`Refusal::VersionUnsupported`] when
    /// the ranges do not overlap.
    pub const fn negotiate(self) -> Result<Agreed, Refusal> {
        if self.floor > self.highest {
            return Err(Refusal::Malformed);
        }
        let version =
            if self.highest < VOCABULARY_VERSION { self.highest } else { VOCABULARY_VERSION };
        if version < self.floor || version < VOCABULARY_VERSION_MIN {
            return Err(Refusal::VersionUnsupported);
        }
        Ok(Agreed(version))
    }
}

/// Writes a record's fields into a payload, counting as it goes.
///
/// Little-endian and by hand, [`store`](crate::store)'s discipline and
/// [`scene`](crate::scene)'s: the layout is the encoding rather than the
/// compiler's opinion of a struct, so nothing here depends on the host's word
/// size, its alignment rules or its byte order. Every slot the writer does not
/// reach stays zero, which is what makes a record's tail the same bytes on both
/// sides of the wire.
struct Writer<'a> {
    /// The payload being filled.
    out: &'a mut [u8; PAYLOAD_BYTES],
    /// How much of it has been written.
    at: usize,
}

impl Writer<'_> {
    /// Append one byte.
    fn u8(&mut self, value: u8) {
        self.put(&[value]);
    }

    /// Append two bytes.
    fn u16(&mut self, value: u16) {
        self.put(&value.to_le_bytes());
    }

    /// Append four bytes.
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes.
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    /// The one place bytes land.
    ///
    /// Indexing rather than a checked write, and the bound is a compile-time
    /// fact rather than a runtime hope: every record asserts
    /// `WIDTH <= PAYLOAD_BYTES` further down this file, and
    /// `no_record_writes_past_its_declared_width` requires every record's
    /// writer to stop at its own `WIDTH`. A record wide enough to overflow this
    /// cannot reach a build.
    fn put(&mut self, bytes: &[u8]) {
        self.out[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }
}

/// Reads a record's fields out of a payload, counting as it goes.
///
/// The counting is the point. [`Reader::finish`] refuses every byte the
/// record's decoder did not consume, so *unread* is a property of what the
/// decoder actually did rather than of a width somebody wrote down beside it. A
/// field dropped from a decoder does not become a field that is ignored; it
/// becomes a field in the tail, and the tail is refused.
struct Reader<'a> {
    /// The payload being read.
    raw: &'a [u8; PAYLOAD_BYTES],
    /// How much of it has been consumed.
    at: usize,
}

impl Reader<'_> {
    /// Take one byte.
    fn u8(&mut self) -> u8 {
        let at = self.take(1);
        self.raw[at]
    }

    /// Take two bytes.
    fn u16(&mut self) -> u16 {
        let at = self.take(2);
        u16::from_le_bytes([self.raw[at], self.raw[at + 1]])
    }

    /// Take four bytes.
    fn u32(&mut self) -> u32 {
        let at = self.take(4);
        u32::from_le_bytes([self.raw[at], self.raw[at + 1], self.raw[at + 2], self.raw[at + 3]])
    }

    /// Take eight bytes.
    fn u64(&mut self) -> u64 {
        let at = self.take(8);
        let mut word = [0u8; 8];
        let mut i = 0;
        while i < 8 {
            word[i] = self.raw[at + i];
            i += 1;
        }
        u64::from_le_bytes(word)
    }

    /// Advance, and answer where the caller's bytes start. Bounded by the same
    /// compile-time assertion [`Writer::put`] is.
    fn take(&mut self, bytes: usize) -> usize {
        let at = self.at;
        self.at += bytes;
        at
    }

    /// Every byte this record did not read must be zero.
    ///
    /// The whole of *an entry with a non-zero unread field is refused rather
    /// than ignored*, for the payload half, written once so that no record can
    /// be written without it.
    fn finish(self) -> Result<(), Refusal> {
        let mut at = self.at;
        while at < PAYLOAD_BYTES {
            if self.raw[at] != 0 {
                return Err(Refusal::Reserved);
            }
            at += 1;
        }
        Ok(())
    }
}

/// What a record may consult while it decodes: the agreement, if there is one,
/// and the list it was agreed against.
///
/// Carried as one value rather than two parameters so that a record cannot hold
/// a vocabulary without the version it was agreed at — which is the pair the
/// admission rule needs, and separating them would let a decoder ask the
/// vocabulary a question about a version nobody negotiated.
///
/// `agreed` is an [`Option`] because exactly one record decodes before there is
/// an agreement, and that record consults nothing. Every other opcode is
/// refused before its record is reached: see [`Entry::read`].
#[derive(Clone, Copy)]
struct Admission<'a> {
    /// The version both sides agreed, or `None` before the handshake.
    agreed: Option<Agreed>,
    /// The list that version names.
    vocabulary: &'a dyn Vocabulary,
}

impl Admission<'_> {
    /// The agreement a record's ordinals are admitted against.
    ///
    /// Unreachable as a refusal for every record this dispatch actually
    /// reaches, because [`Entry::read`] refuses a non-handshake opcode with no
    /// agreement before any record is decoded. It returns a [`Result`] anyway
    /// rather than unwrapping: an eighth opcode that reached here another way
    /// would get the refusal rather than a panic, in a component whose whole
    /// job is decoding bytes it does not trust.
    fn agreed(self) -> Result<Agreed, Refusal> {
        self.agreed.ok_or(Refusal::NotNegotiated)
    }
}

/// What every opcode's record can do.
///
/// Private, because nothing outside this module should encode a payload without
/// the envelope that frames it — [`Delta`] is the door. Its value is what it
/// makes obligatory: a record cannot exist without a width, a specimen, a
/// writer and a reader, so an eighth opcode cannot be added without all four,
/// and [`Entry::specimens`] picks the specimen up without anybody remembering
/// to add it to a test.
trait Record: Copy + Sized {
    /// The bytes this record's fields occupy, before the padding to
    /// [`PAYLOAD_BYTES`].
    /// Unit: bytes.
    const WIDTH: usize;

    /// One value of this record that is legal on the wire.
    ///
    /// Not a default and not an empty value: every closed field holds something
    /// inside its set, so that decoding it succeeds and the round-trip test has
    /// something to round-trip. A field added to a record without a value here
    /// does not compile.
    ///
    /// Every ordinal in a specimen is zero — the first entry of its list —
    /// because the lists are append-only, so ordinal zero is named by every
    /// version there has been and by every version there will be short of a
    /// floor raise.
    const SPECIMEN: Self;

    /// Write the fields, in order.
    fn write(&self, out: &mut Writer);

    /// Read the fields, in the same order, refusing before any of them is
    /// handed back.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming the disbelief. The tail is not this method's
    /// business: [`Record::from_payload`] holds that rule for every record.
    fn read(raw: &mut Reader, admission: Admission) -> Result<Self, Refusal>;

    /// Encode into a whole payload slot. Everything past the fields is zero.
    fn to_payload(&self) -> [u8; PAYLOAD_BYTES] {
        let mut out = [0u8; PAYLOAD_BYTES];
        let mut writer = Writer { out: &mut out, at: 0 };
        self.write(&mut writer);
        out
    }

    /// Decode a whole payload slot, refusing whatever this record did not read.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] from the record's own fields, or [`Refusal::Reserved`] for
    /// a byte past them.
    fn from_payload(raw: &[u8; PAYLOAD_BYTES], admission: Admission) -> Result<Self, Refusal> {
        let mut reader = Reader { raw, at: 0 };
        let value = Self::read(&mut reader, admission)?;
        reader.finish()?;
        Ok(value)
    }
}

/// Names an opcode constant where a pattern is expected.
///
/// `macro_rules!` will not take `op::$constant` as a pattern directly — a path
/// followed by a metavariable is ambiguous there — and this is the indirection
/// that resolves it. It exists for no other reason and is declared before
/// `entries!` because a macro must be defined before it is used.
macro_rules! opcode_pattern {
    ($constant:ident) => {
        op::$constant
    };
}

/// The seven opcodes, written once.
///
/// # Why a macro, in a crate that mostly refuses them
///
/// Because the alternative is five sequences that have to agree: the opcode
/// constants, the list `known` matches against, the [`Entry`] variants, the
/// answer to *is this the handshake*, and the specimens a round-trip test
/// iterates. The exit this file is accepted on says an ordinal the vocabulary
/// does not name is refused at the boundary; a hand-written specimen table is
/// precisely how a sentence like that stops being true while every test stays
/// green — an eighth opcode is added, it is not in the table, and the loop over
/// the table keeps passing.
///
/// The `handshake` column is the one that is not decoration. *Which entry
/// agrees the vocabulary* is the question the whole protocol order rests on,
/// and it is answered on the line that declares the opcode rather than by a
/// comparison against a constant somewhere inside a decoder — so an eighth
/// opcode must answer it, and two opcodes claiming it fails the assertion below
/// the invocation rather than producing a channel that can be re-negotiated.
///
/// The cost is the indirection between a reader and the enum, which is why
/// nothing else in this file is written this way. It is paid here because here
/// the second copy was the defect.
macro_rules! entries {
    (
        $(
            $(#[$about:meta])*
            $variant:ident / $record:ident / $constant:ident = $opcode:literal,
            handshake: $handshake:literal;
        )*
    ) => {
        /// The semantic service's opcode space.
        ///
        /// Per-service and not global: section 05. These numbers mean nothing
        /// on the frame's ring, the control ring or the compositor's, and
        /// comparing an opcode across two spaces is the mistake a single global
        /// enumeration would invite.
        ///
        /// They start at one, so that a zeroed entry — which is what an
        /// untouched slot of a fresh mapping holds — names no opcode and is
        /// refused rather than read as the first one. RFC 0028 reserves `0xFE`
        /// and `0xFF` at the top of every service's space for buffer
        /// registration, and nothing here approaches them.
        pub mod op {
            $(
                $(#[$about])*
                ///
                /// Unit: none — an opcode is an identifier, not a quantity.
                pub const $constant: u8 = $opcode;
            )*

            /// How many opcodes this service has.
            ///
            /// Counted from the list rather than written down, so it cannot
            /// disagree with what it counts.
            /// Unit: opcodes.
            pub const COUNT: usize = [$(stringify!($constant)),*].len();

            /// Every opcode, in declaration order.
            ///
            /// Emitted from the same list as the constants, so it holds every
            /// opcode there is — not because a test checks it, but because
            /// there is no way to write one this array does not get.
            /// Unit: none — opcodes.
            pub const ALL: [u8; COUNT] = [$($constant),*];

            /// Is this an opcode this build implements?
            ///
            /// R04: the negative answer is what turns an unknown opcode into a
            /// refusal instead of a silently skipped entry.
            #[must_use]
            pub const fn known(opcode: u8) -> bool {
                matches!(opcode, $($constant)|*)
            }

            /// A word for a log or a trace.
            #[must_use]
            pub const fn label(opcode: u8) -> &'static str {
                match opcode {
                    $($constant => stringify!($variant),)*
                    _ => "unknown",
                }
            }

            /// Is this the entry that agrees the vocabulary?
            ///
            /// `None` for an opcode this build does not know, which is a
            /// different answer from *no* and is kept different on purpose: a
            /// caller that collapsed the two would treat an unknown opcode as
            /// an ordinary edit and go on to ask whether the handshake had
            /// happened, which is the wrong question about an opcode that does
            /// not exist.
            #[must_use]
            pub const fn is_handshake(opcode: u8) -> Option<bool> {
                match opcode {
                    $($constant => Some($handshake),)*
                    _ => None,
                }
            }
        }

        /// What a semantic entry says, once its payload has been believed.
        ///
        /// One variant per opcode, emitted from the same list, so the set of
        /// things an entry can be is the set of opcodes there are.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Entry {
            $(
                $(#[$about])*
                $variant($record),
            )*
        }

        impl Entry {
            /// One legal value of every opcode's record.
            ///
            /// The round-trip test's corpus, and it is derived rather than
            /// written: an eighth opcode is round-tripped by the tests that
            /// already exist, on the day it is declared, without anybody
            /// remembering.
            /// Unit: none — one entry per opcode, in declaration order.
            #[must_use]
            pub const fn specimens() -> [Self; op::COUNT] {
                [$(Self::$variant($record::SPECIMEN)),*]
            }

            /// The opcode that names this entry.
            #[must_use]
            pub const fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant(_) => op::$constant,)*
                }
            }

            /// The payload as it crosses: the record's fields, then zero to
            /// [`PAYLOAD_BYTES`].
            #[must_use]
            pub fn payload(&self) -> [u8; PAYLOAD_BYTES] {
                match self {
                    $(Self::$variant(record) => record.to_payload(),)*
                }
            }

            /// The bytes a record's fields occupy, for the caller that needs to
            /// know where its tail starts.
            /// Unit: bytes.
            #[must_use]
            pub const fn width(&self) -> usize {
                match self {
                    $(Self::$variant(_) => $record::WIDTH,)*
                }
            }

            /// The entry an opcode and a payload name.
            ///
            /// The handshake's order is settled here, before any record is
            /// decoded, and it is settled from the list rather than against a
            /// constant: an opcode that declares itself the handshake may only
            /// arrive with no agreement standing, and an opcode that does not
            /// may only arrive with one. Doing it here rather than inside the
            /// records is what makes it cover [`Remove`] and [`Commit`], which
            /// carry no ordinal and would otherwise have sailed through a check
            /// that lived where the ordinals are read.
            fn read(
                opcode: u8,
                raw: &[u8; PAYLOAD_BYTES],
                admission: Admission,
            ) -> Result<Self, Refusal> {
                match (op::is_handshake(opcode), admission.agreed.is_some()) {
                    (None, _) => return Err(Refusal::UnknownOpcode),
                    (Some(true), true) => return Err(Refusal::Renegotiated),
                    (Some(false), false) => return Err(Refusal::NotNegotiated),
                    (Some(_), _) => {}
                }
                match opcode {
                    $(
                        opcode_pattern!($constant) =>
                            Ok(Self::$variant($record::from_payload(raw, admission)?)),
                    )*
                    _ => Err(Refusal::UnknownOpcode),
                }
            }
        }
    };
}

entries! {
    /// Agree what an ordinal means, before any ordinal crosses.
    ///
    /// Must be the first entry on the channel and may appear once per channel
    /// epoch. RFC 0083, and the module's *the problem this module exists for*.
    DeclareVocabulary / Handshake / DECLARE_VOCABULARY = 0x01, handshake: true;

    /// Introduce a node: its identity, where it sits, what it is, and what
    /// operating it does.
    ///
    /// The one opcode that introduces an identifier. Every other record here
    /// names a node this one already declared, which is what makes *a node
    /// exists because an entry declared it* a property of the format rather
    /// than of the receiver's discipline.
    DeclareNode / DeclareNode / DECLARE_NODE = 0x02, handshake: false;

    /// Replace how a node stands.
    SetState / SetState / SET_STATE = 0x03, handshake: false;

    /// Replace what a node holds.
    SetContent / SetContent / SET_CONTENT = 0x04, handshake: false;

    /// Replace the edges the tree's shape does not already say.
    SetRelations / SetRelations / SET_RELATIONS = 0x05, handshake: false;

    /// Remove a node and everything under it.
    ///
    /// The subtree and not the node alone, because the alternative is an entry
    /// that can leave a child with no parent — a tree state no interface means,
    /// and one every projection would have to have an opinion about.
    Remove / Remove / REMOVE = 0x06, handshake: false;

    /// Close the frame: apply every entry since the last commit, or none.
    ///
    /// Section 11 makes the semantic protocol atomic per commit, and this is
    /// the boundary RFC 0083 makes the unit of refusal. It carries no deadline;
    /// the module's *no deadline* is why.
    Commit / Commit / COMMIT = 0x07, handshake: false;
}

impl Record for Handshake {
    const WIDTH: usize = 2 + 2;
    const SPECIMEN: Self = Self::HERE;

    fn write(&self, out: &mut Writer) {
        out.u16(self.highest);
        out.u16(self.floor);
    }

    fn read(raw: &mut Reader, _admission: Admission) -> Result<Self, Refusal> {
        let highest = raw.u16();
        let floor = raw.u16();
        // Not `negotiate` — that is the receiver's decision and it happens in
        // `Session`, where the agreement is kept. What is refused here is the
        // statement itself: a peer that offers no version at all has not made
        // one, and a zeroed payload is exactly that, so it is refused for the
        // reason every zeroed payload in this crate is rather than agreed at
        // version zero.
        if highest == 0 || floor > highest {
            return Err(Refusal::Malformed);
        }
        Ok(Self { highest, floor })
    }
}

/// Introduce a node: its identity, where it sits, what it is, and what
/// operating it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclareNode {
    /// The identifier this node answers to from now on.
    ///
    /// Chosen by the submitter, in the submitter's own space, and never by the
    /// receiver. RFC 0077's fourth rule — *identity is stable and
    /// author-assigned* — is what lets a redesign keep a test passing and what
    /// lets an arrangement be keyed by identity rather than by widening the
    /// node; an identifier the receiver allocated would have to come back in a
    /// completion, and a client that waited for one could not declare a tree
    /// without a round trip per node.
    /// Unit: none — a node identifier, not a quantity. [`NO_NODE`] is refused,
    /// because RFC 0077 makes a node holding it a `Defect::Unnamed`.
    pub node: u64,
    /// The node this one sits under.
    /// Unit: none — a node identifier. [`NO_NODE`] is a root, which is the one
    /// place the value means *nobody* rather than being refused.
    pub parent: u64,
    /// The sibling this node is inserted in front of.
    ///
    /// Child order is meaningful in this vocabulary — a list's items, a table's
    /// rows, a row's cells — so a format that could only append would make
    /// *insert one item in the middle* into a rebuild of every sibling after
    /// it, on the edit a reconciler makes most often.
    /// Unit: none — a node identifier, and a sibling of this node under
    /// `parent`. [`NO_NODE`] appends after the last sibling.
    pub before: u64,
    /// What this node is.
    ///
    /// The field this whole module is arranged around. It is a [`RoleOrdinal`]
    /// rather than a `u16`, so that a decoded entry cannot carry an ordinal the
    /// agreed vocabulary did not name, and so that a projection has nothing to
    /// write a wildcard arm about.
    /// Unit: none — an ordinal into the vocabulary this channel agreed under
    /// RFC 0083, not a quantity. It is meaningless without that agreement, which
    /// is why it is a [`RoleOrdinal`] and not a `u16`.
    pub role: RoleOrdinal,
    /// What operating this node does — a capability, never a callback.
    ///
    /// On the declaration rather than in an entry of its own, because an intent
    /// is not a state: it is what the node is *for*, and a node whose intent
    /// could be replaced under a projection that has already announced it is a
    /// node that lied. Replacing it means removing the node and declaring
    /// another, which is exactly what it is.
    /// Unit: none — a capability reference in the submitter's own space.
    /// [`NO_INTENT`] means the node is not operable, which is how RFC 0077
    /// spells read-only.
    pub intent: u32,
}

impl Record for DeclareNode {
    const WIDTH: usize = 8 + 8 + 8 + 2 + 4;
    const SPECIMEN: Self =
        Self { node: 7, parent: NO_NODE, before: NO_NODE, role: RoleOrdinal(0), intent: NO_INTENT };

    fn write(&self, out: &mut Writer) {
        out.u64(self.node);
        out.u64(self.parent);
        out.u64(self.before);
        out.u16(self.role.0);
        out.u32(self.intent);
    }

    fn read(raw: &mut Reader, admission: Admission) -> Result<Self, Refusal> {
        let node = raw.u64();
        let parent = raw.u64();
        let before = raw.u64();
        let ordinal = raw.u16();
        let intent = raw.u32();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        // A node under itself, or in front of itself. The only cycle one entry
        // can state on its own, so it is the only one this format can refuse; a
        // cycle spread over two entries is the tree's to catch, where `check`
        // already lives.
        if parent == node || before == node {
            return Err(Refusal::Value);
        }
        // The ordinal is not biased by one, unlike `scene::kind`, and the
        // difference is the exit this file is accepted on: the value on the
        // wire is the role's index in the declaring list, with nothing
        // arithmetic standing between them, so a reader comparing a capture
        // against `vocabulary!` is comparing the same number. The work a bias
        // would have done — making a zeroed payload name nothing — is done by
        // `node` above, which a zeroed payload fails first.
        let role = admission.agreed()?.admit_role(ordinal, admission.vocabulary)?;
        Ok(Self { node, parent, before, role, intent })
    }
}

/// Replace how a node stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetState {
    /// The node whose states these are.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused.
    pub node: u64,
    /// The states it is now in.
    ///
    /// Replaces the set rather than merging into it, for the reason
    /// [`scene::SetTransform`](crate::scene::SetTransform) replaces rather than
    /// composes: a merging entry would make the tree a function of how many
    /// times it had been sent, so a frame cut in the middle would produce a
    /// third tree neither side ever asked for. Clearing a state is sending the
    /// set without it, which needs no second opcode.
    /// Unit: none — a bit set, not a quantity. A population count over it would
    /// be a number, and nothing on this wire takes one.
    pub state: StateBits,
}

impl Record for SetState {
    const WIDTH: usize = 8 + 1;
    const SPECIMEN: Self = Self { node: 7, state: StateBits(0) };

    fn write(&self, out: &mut Writer) {
        out.u64(self.node);
        out.u8(self.state.0);
    }

    fn read(raw: &mut Reader, admission: Admission) -> Result<Self, Refusal> {
        let node = raw.u64();
        let bits = raw.u8();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        let state = admission.agreed()?.admit_state(bits, admission.vocabulary)?;
        Ok(Self { node, state })
    }
}

/// Replace what a node holds.
///
/// # Why the content is not in the payload
///
/// `interface`'s `TEXT_MAX` is 192 bytes and a `Reading` is four optional
/// quantities; neither fits [`PAYLOAD_BYTES`], and widening the stride to fit
/// the widest content would make every [`Remove`] carry two hundred bytes of
/// nothing, on a format whose whole point is that a batch is an array with a
/// stride. So this record says *which kind, and where the body is*, the way
/// [`scene::SetPath`](crate::scene::SetPath) points at geometry, and the body
/// lives in the arena.
///
/// The kind stays here rather than travelling with the body, because the kind
/// is the closed field: it is admitted against the agreed vocabulary by the
/// same rule as a role, and a discriminant that arrived inside an opaque blob
/// would be a closed field this format never saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetContent {
    /// The node whose content this is.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused.
    pub node: u64,
    /// Which kind of thing it holds.
    /// Unit: none — an ordinal into the content vocabulary, not a quantity.
    pub kind: ContentOrdinal,
    /// Where the body is.
    /// Unit: bytes from the first byte of the channel's inline arena, not from
    /// the start of the mapping — [`crate::op::WRITE_SERIAL`]'s origin, and for
    /// its reason. Zero is the first byte of the arena.
    pub body_offset: u32,
    /// How long the body is.
    /// Unit: bytes. Zero is a content kind whose body is empty, which is what
    /// *holds nothing* looks like and is why it is not refused.
    pub body_len: u32,
}

impl Record for SetContent {
    const WIDTH: usize = 8 + 2 + 4 + 4;
    const SPECIMEN: Self = Self { node: 7, kind: ContentOrdinal(0), body_offset: 0, body_len: 0 };

    fn write(&self, out: &mut Writer) {
        out.u64(self.node);
        out.u16(self.kind.0);
        out.u32(self.body_offset);
        out.u32(self.body_len);
    }

    fn read(raw: &mut Reader, admission: Admission) -> Result<Self, Refusal> {
        let node = raw.u64();
        let ordinal = raw.u16();
        let body_offset = raw.u32();
        let body_len = raw.u32();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        let kind = admission.agreed()?.admit_content(ordinal, admission.vocabulary)?;
        Ok(Self { node, kind, body_offset, body_len })
    }
}

/// One edge the tree's shape does not already say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// What kind of edge it is.
    /// Unit: none — an ordinal into the relation vocabulary, not a quantity.
    pub kind: RelationOrdinal,
    /// The node it points at.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused, because RFC
    /// 0077's third refusal is that an edge to zero used to resolve and must
    /// not.
    pub target: u64,
}

/// Replace the edges the tree's shape does not already say.
///
/// The whole set in one entry, which is what makes replacing it atomic without
/// any help from the commit: a node's relations are read together — a label and
/// the thing it labels are one fact — and a set assembled out of several
/// entries would have intermediate states in which the node claims a label it
/// no longer has.
///
/// That atomicity is bought with [`RELATIONS_MAX`], and the bound is the wire's
/// rather than a copy of `interface`'s: a set that does not fit one payload is
/// a set this record cannot make atomic, and the day `interface` allows one is
/// the day this format has to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetRelations {
    /// The node these edges leave.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused.
    pub node: u64,
    /// How many of `edges` are this node's.
    /// Unit: edges, and never above [`RELATIONS_MAX`].
    pub count: u16,
    /// The edges, `count` of them from the front.
    ///
    /// The slots past `count` are not read, which is deliberate rather than
    /// lax: a field that is not read is a field in the tail, so
    /// [`Reader::finish`] requires them to be zero and a producer that left
    /// stale bytes in one is refused by the rule that was already there. There
    /// is no hand-written *and the rest must be empty* check here, because a
    /// hand-written check is what a fifth slot gets left out of.
    /// Unit: none — edges, not a quantity. How many are read is `count`; the
    /// array length is the fixed maximum and never a measurement.
    pub edges: [Edge; RELATIONS_MAX],
}

impl SetRelations {
    /// One edge slot: a relation ordinal and the node it points at.
    /// Unit: bytes.
    const SLOT: usize = 2 + 8;

    /// An edge that is not there.
    ///
    /// Zero in both fields, so that an unused slot encodes as the zero bytes
    /// [`Reader::finish`] demands of it.
    const NO_EDGE: Edge = Edge { kind: RelationOrdinal(0), target: NO_NODE };
}

impl Record for SetRelations {
    const WIDTH: usize = 8 + 2 + RELATIONS_MAX * Self::SLOT;
    const SPECIMEN: Self = Self {
        node: 7,
        count: 1,
        edges: [
            Edge { kind: RelationOrdinal(0), target: 3 },
            Self::NO_EDGE,
            Self::NO_EDGE,
            Self::NO_EDGE,
        ],
    };

    fn write(&self, out: &mut Writer) {
        out.u64(self.node);
        out.u16(self.count);
        // Only the declared edges are written. The rest of the payload stays
        // the zero it was initialised to, which is the same bytes the reader
        // requires — the two halves of the tail rule meeting in the middle.
        let mut i = 0;
        while i < self.count as usize && i < RELATIONS_MAX {
            out.u16(self.edges[i].kind.0);
            out.u64(self.edges[i].target);
            i += 1;
        }
    }

    fn read(raw: &mut Reader, admission: Admission) -> Result<Self, Refusal> {
        let node = raw.u64();
        let count = raw.u16();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        if count as usize > RELATIONS_MAX {
            return Err(Refusal::Value);
        }
        let agreed = admission.agreed()?;
        let mut edges = [Self::NO_EDGE; RELATIONS_MAX];
        let mut i = 0;
        while i < count as usize {
            let ordinal = raw.u16();
            let target = raw.u64();
            if target == NO_NODE {
                return Err(Refusal::NoNode);
            }
            // An edge from a node to itself. Every relation in this vocabulary
            // is between two nodes — a label and the thing it labels, a control
            // and the thing it controls — and a self-edge is the one a
            // reconciler emits when it has lost track of which node it is on.
            if target == node {
                return Err(Refusal::Value);
            }
            edges[i] = Edge { kind: agreed.admit_relation(ordinal, admission.vocabulary)?, target };
            i += 1;
        }
        Ok(Self { node, count, edges })
    }
}

/// Remove a node and everything under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remove {
    /// The node to remove, with its subtree.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused, which is what
    /// stops a zeroed payload from meaning *remove the root*.
    pub node: u64,
}

impl Record for Remove {
    const WIDTH: usize = 8;
    const SPECIMEN: Self = Self { node: 7 };

    fn write(&self, out: &mut Writer) {
        out.u64(self.node);
    }

    fn read(raw: &mut Reader, _admission: Admission) -> Result<Self, Refusal> {
        let node = raw.u64();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        Ok(Self { node })
    }
}

/// Close the frame: apply every entry since the last commit, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Commit {
    /// The submitter's name for this frame, quoted back in whatever the
    /// receiver says about it.
    ///
    /// Distinct from [`Delta::user_data`], which names the *entry*: a frame is
    /// many entries and a refusal names the frame, so a caller matching a
    /// refusal to the tree it was building needs the frame's name rather than
    /// the commit entry's.
    /// Unit: none — chosen by the submitter and never interpreted. Zero is
    /// refused, because a frame nobody named is a frame a refusal cannot be
    /// reported against.
    pub frame_token: u64,
}

impl Record for Commit {
    const WIDTH: usize = 8;
    const SPECIMEN: Self = Self { frame_token: 9 };

    fn write(&self, out: &mut Writer) {
        out.u64(self.frame_token);
    }

    fn read(raw: &mut Reader, _admission: Admission) -> Result<Self, Refusal> {
        let frame_token = raw.u64();
        if frame_token == 0 {
            return Err(Refusal::Value);
        }
        Ok(Self { frame_token })
    }
}

/// One semantic entry as it crosses: part I's envelope, and the body its opcode
/// names.
///
/// Every field here is either read by this format or refused, and there is no
/// third category — which is the sentence the whole module is arranged to make
/// true. See *an unread field is refused* for how each half is enforced, and
/// *no deadline* for the field that is absent rather than checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delta {
    /// Returned verbatim in the completion, and opaque here.
    /// Unit: none — chosen by the submitter and never interpreted. Zero is a
    /// legal token; a client that wants to match completions chooses otherwise.
    pub user_data: u64,
    /// The scheduling class, and the depth its urgency has already crossed.
    ///
    /// Read on every opcode because it is the ring's field and not this
    /// format's: the service that drains the ring orders by it, and
    /// [`deadline::inherit`](crate::deadline::inherit) is what decides what it
    /// means. Nothing here re-decides it, and nothing here may discard it.
    /// Unit: none — a `class` field: an ordinal in the low byte, a depth in the
    /// high one. Zero is `class::HARD` at depth zero.
    pub class: u16,
    /// Where this entry's payload is.
    /// Unit: bytes from the first byte of the channel's inline arena, not from
    /// the start of the mapping. Zero is the first byte of the arena.
    pub payload_offset: u32,
    /// Submission flags.
    /// Unit: none — a bitmask, and a subset of [`FLAGS_ACCEPTED`]. Zero is no
    /// flags, which is always legal.
    pub flags: u8,
    /// What the entry says.
    /// Unit: none — one opcode's record.
    pub body: Entry,
}

impl Delta {
    /// The opcode this entry submits under.
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        self.body.opcode()
    }

    /// Is this an entry a peer would accept, as far as the envelope goes?
    ///
    /// The half of [`Delta::decode`]'s rules that does not need the payload,
    /// exposed so that a producer can refuse its own entry before submitting it
    /// rather than learning about it in a completion. It is the same code path,
    /// not a second statement of the same rules.
    ///
    /// The value half needs no exposing: every ordinal in a [`Delta`] was
    /// admitted when it was constructed, because there is no other way to
    /// construct one.
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownFlag`] for a flag outside [`FLAGS_ACCEPTED`].
    pub const fn check(&self) -> Result<(), Refusal> {
        envelope_rules(self.opcode(), self.flags)
    }

    /// The frame this entry closes, or `None` for an entry that closes none.
    ///
    /// The arms are written out rather than closed with a wildcard, so that an
    /// eighth opcode stops this build and asks whether it closes a frame. A
    /// wildcard would answer `None` on its behalf, which is the safe answer and
    /// exactly the kind of safe answer nobody ever revisits.
    #[must_use]
    pub const fn frame(&self) -> Option<u64> {
        match self.body {
            Entry::Commit(commit) => Some(commit.frame_token),
            Entry::DeclareVocabulary(_)
            | Entry::DeclareNode(_)
            | Entry::SetState(_)
            | Entry::SetContent(_)
            | Entry::SetRelations(_)
            | Entry::Remove(_) => None,
        }
    }

    /// The submission entry this delta crosses in.
    ///
    /// Written field by field, with no `..Sqe::ZERO`, so that a field added to
    /// [`Sqe`] stops this build here and asks whether a semantic entry reads
    /// it. [`Delta::decode`] then compares an arriving entry against exactly
    /// this, which turns whatever is answered into something a peer cannot get
    /// wrong.
    #[must_use]
    pub const fn envelope(&self) -> Sqe {
        Sqe {
            opcode: self.opcode(),
            flags: self.flags,
            class: self.class,
            // A semantic entry names no capability. Authority here is the
            // ring's — the client holds a channel to the component that owns
            // the tree, and what it may declare is that tree — so a capability
            // index on an entry would be an authority nobody granted and a
            // field nobody reads. The one capability the tree does carry is
            // `DeclareNode::intent`, which belongs to the node rather than to
            // the submission.
            cap: 0,
            user_data: self.user_data,
            // No semantic opcode is scheduled work: the module's *no deadline*
            // argues it, and this is where a peer that disagreed is refused,
            // because a non-zero deadline fails the byte comparison below.
            deadline: NO_DEADLINE,
            offset: self.payload_offset as u64,
            // The payload is in the arena, at `offset`. `FIXED_BUF` is outside
            // `FLAGS_ACCEPTED`, so these two never name a registered set.
            buf_set: 0,
            buf_index: 0,
            len: PAYLOAD_BYTES as u32,
            _reserved: 0,
            // Not a spare pair of words. A field that belongs to an opcode
            // belongs in that opcode's record, where the record's own width
            // states it and `Reader::finish` polices it; a field in the
            // envelope would be a field every opcode shares whether it reads it
            // or not.
            ext: [0, 0],
        }
    }

    /// Encode: the entry, and the payload it points at.
    ///
    /// Total, on purpose. It writes what it was given, including a value
    /// [`Delta::decode`] would refuse — which is what lets a test build a
    /// malformed entry without a second encoder, and lets a hostile-peer
    /// harness produce the entries it exists to produce. A producer that wants
    /// the check calls [`Delta::check`] first.
    ///
    /// What it cannot write is an ordinal nobody agreed, because it cannot be
    /// handed one: that refusal is at the type and not at this function.
    #[must_use]
    pub fn encode(&self) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        (self.envelope(), self.body.payload())
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The order is the order a refusal is distinguishable in, which is
    /// [`store`](crate::store)'s rule for the same job: the opcode and the
    /// flags first, because a caller acts on those differently; the framing;
    /// then the payload, which is where the handshake order and the vocabulary
    /// are settled; then the whole-envelope comparison, which is the catch-all
    /// nothing can be added past.
    ///
    /// `agreement` is `None` before [`op::DECLARE_VOCABULARY`] has been
    /// believed and `Some` after. It is a parameter rather than state because
    /// this function is a pure function of what it is handed — a decoder that
    /// consulted something could be made to decode one payload two ways — and
    /// [`Session`] is where the state that answers it lives.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]. Three are worth naming: [`Refusal::Reserved`] is what an
    /// entry gets for any byte, anywhere, that this build does not read,
    /// including a deadline; [`Refusal::NotNegotiated`] is what every opcode
    /// but the handshake gets before one; and [`Refusal::Unnamed`] is the one
    /// this module exists for.
    pub fn decode(
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        agreement: Option<Agreed>,
        vocabulary: &dyn Vocabulary,
    ) -> Result<Self, Refusal> {
        envelope_rules(entry.opcode, entry.flags)?;
        if entry.len as usize != PAYLOAD_BYTES {
            return Err(Refusal::Malformed);
        }
        // A channel mapping is a `u32` of bytes — `layout::MAX_ENTRIES` is
        // chosen so that every offset in one fits with room to spare — so an
        // offset past that is not an offset into any arena. Refused here rather
        // than truncated into the field below, because a truncation would make
        // the comparison that follows fail as *a reserved field is not zero*,
        // which is a true sentence about the wrong field.
        if entry.offset > u64::from(u32::MAX) {
            return Err(Refusal::Malformed);
        }

        let body = Entry::read(entry.opcode, payload, Admission { agreed: agreement, vocabulary })?;
        let delta = Self {
            user_data: entry.user_data,
            class: entry.class,
            payload_offset: entry.offset as u32,
            flags: entry.flags,
            body,
        };

        // The check this module shares with `scene`, on the envelope half. Not
        // a list of the fields a semantic entry ignores — such a list is
        // correct on the day it is written and silent afterwards — but the
        // entry this build would have produced from the fields it just read,
        // compared in full. Any field `envelope` does not set, including one a
        // later ABI adds, must arrive zero.
        if sqe_bytes(&delta.envelope()) != sqe_bytes(entry) {
            return Err(Refusal::Reserved);
        }
        Ok(delta)
    }
}

/// The envelope rules that do not need the payload.
///
/// One function, called by [`Delta::check`] before submitting and by
/// [`Delta::decode`] after receiving, so that a producer and a consumer cannot
/// hold different opinions about which entries are legal.
///
/// Shorter than [`scene`](crate::scene)'s by exactly the deadline, which is not
/// an omission: no semantic opcode reads one, so there is no per-opcode question
/// to ask, and the byte comparison refuses a deadline as the unread field it is.
const fn envelope_rules(opcode: u8, flags: u8) -> Result<(), Refusal> {
    if op::is_handshake(opcode).is_none() {
        return Err(Refusal::UnknownOpcode);
    }
    if flags & !FLAGS_ACCEPTED != 0 {
        return Err(Refusal::UnknownFlag);
    }
    Ok(())
}

/// The sixty-four bytes of a submission entry.
///
/// A byte view rather than a field-by-field comparison, and the difference is
/// the whole of [`Delta::decode`]'s guarantee: a comparison written out by hand
/// covers the fields somebody listed, and this one covers the fields there are.
///
/// The same function exists privately in [`scene`](crate::scene). It is
/// duplicated rather than shared because the shared home would be `lib.rs`,
/// beside [`Sqe`], and moving it there is a change to a file this one does not
/// own; the duplication is recorded here rather than left for a reader to
/// notice.
fn sqe_bytes(entry: &Sqe) -> &[u8; SQE_BYTES] {
    // SAFETY: `Sqe` is `#[repr(C, align(64))]`, so its fields are laid out in
    // declaration order at fixed offsets; `size_of::<Sqe>()` is `SQE_BYTES` and
    // the assertion below this function shows that width to be the exact sum of
    // its field widths, so the type carries no padding and every one of its
    // bytes is an initialised byte of an integer. `[u8; SQE_BYTES]` has
    // alignment one, which `Sqe`'s sixty-four satisfies. The reference produced
    // borrows `entry` for its own lifetime and is shared, so nothing is mutated
    // through it and nothing outlives it.
    unsafe { &*core::ptr::from_ref(entry).cast::<[u8; SQE_BYTES]>() }
}

/// Where the offending entry was, and why.
///
/// RFC 0083 asks that a refused entry be reported *by its position and its
/// ordinal*. The ordinal is inside [`Refusal::Unnamed`], because a fault that
/// carried an ordinal field for every refusal would carry a zero for most of
/// them and a reader would have to know which ones meant it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    /// Why the entry was not believed.
    /// Unit: none — a refusal, not a quantity.
    pub refusal: Refusal,
    /// Which entry of this frame it was, counting from zero at the last commit.
    ///
    /// The position and not the arena offset, because the sender built the
    /// frame as a sequence and that is the coordinate its own code is indexed
    /// by. A producer that has to map an arena offset back to the entry it
    /// wrote is a producer that will guess.
    /// Unit: entries since the last commit.
    pub at: u32,
}

/// One semantic channel's reception: the agreement, the frame, and the poison.
///
/// # What it is for
///
/// [`Delta::decode`] is a pure function and knows nothing about what came
/// before. Three of RFC 0083's rules are about exactly that — the handshake
/// must be first, it may happen once per epoch, and a refusal poisons the frame
/// — so they need something that remembers. This is the smallest thing that
/// can.
///
/// # The contract, stated because it is what makes the refusal mean anything
///
/// **Apply nothing until [`Session::accept`] has returned a [`Commit`].** A
/// receiver that applied each entry as it arrived would have a half-built tree
/// standing when the frame was refused, and RFC 0083's *the tree the receiver
/// is already presenting stands unchanged* would be false in the one place it
/// matters.
///
/// This session helps as far as a type can: once a frame is poisoned, every
/// later entry of it is refused with the same [`Fault`], so there is no entry a
/// caller could be handed and apply. It cannot make the rule true on its own,
/// because it holds no tree — and that is why the rule is written here rather
/// than assumed.
#[derive(Clone, Copy, Debug)]
pub struct Session {
    /// The channel epoch this agreement belongs to.
    epoch: u32,
    /// The version agreed, or `None` before the handshake.
    agreed: Option<Agreed>,
    /// How many entries of the current frame have been offered.
    at: u32,
    /// The refusal that poisoned the current frame, if one did.
    poison: Option<Fault>,
}

impl Session {
    /// A channel on which nothing has been agreed yet.
    #[must_use]
    pub const fn opening(epoch: u32) -> Self {
        Self { epoch, agreed: None, at: 0, poison: None }
    }

    /// The version both sides agreed, or `None` before the handshake.
    #[must_use]
    pub const fn agreed(&self) -> Option<Agreed> {
        self.agreed
    }

    /// Is the frame being assembled already refused?
    ///
    /// Exposed so that a producer-side test and a receiver's own logging can
    /// see the state rather than infer it from a sequence of errors.
    #[must_use]
    pub const fn poisoned(&self) -> Option<Fault> {
        self.poison
    }

    /// Follow the channel's epoch, discarding the agreement if it moved.
    ///
    /// [`ChannelHeader::epoch`](crate::ChannelHeader::epoch) moves when a peer restarts, and its own
    /// documentation says every outstanding token is then stale and must be
    /// discarded rather than matched. The agreement is such a token: it is a
    /// statement the *old* peer made about the vocabulary it holds, and the new
    /// one may be a different build entirely. Discarding it is what makes
    /// [`Refusal::Renegotiated`] a rule about a stream rather than a rule about
    /// a lifetime — an epoch is the unit that may agree again.
    ///
    /// Answers whether anything was discarded, so that a caller can drop the
    /// tree it was presenting in the same breath.
    pub fn follow_epoch(&mut self, epoch: u32) -> bool {
        if epoch == self.epoch {
            return false;
        }
        *self = Self::opening(epoch);
        true
    }

    /// Offer the next entry of this channel.
    ///
    /// # Errors
    ///
    /// A [`Fault`]: the [`Refusal`] and the position of the entry that earned
    /// it. Once a frame is poisoned, every later entry of that frame — and the
    /// [`Commit`] that would have closed it — is refused with the *first* fault
    /// rather than with one of its own, because the first is the cause and the
    /// rest are consequences of having decoded past it.
    pub fn accept(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        vocabulary: &dyn Vocabulary,
    ) -> Result<Delta, Fault> {
        // Read off the raw opcode rather than the decoded body, because a frame
        // that has already been poisoned is not decoded again: the question
        // *does this entry end the frame* has to be answerable before that. An
        // unknown opcode is not `COMMIT`, so a poisoned frame is never closed
        // by one.
        let closes = entry.opcode == op::COMMIT;
        if let Some(standing) = self.poison {
            if closes {
                self.poison = None;
                self.at = 0;
            }
            return Err(standing);
        }

        let at = self.at;
        self.at = self.at.saturating_add(1);

        let delta = match Delta::decode(entry, payload, self.agreed, vocabulary) {
            Ok(delta) => delta,
            Err(refusal) => {
                let fault = Fault { refusal, at };
                if closes {
                    self.at = 0;
                } else {
                    self.poison = Some(fault);
                }
                return Err(fault);
            }
        };

        if let Entry::DeclareVocabulary(handshake) = delta.body {
            // The negotiation, and the one refusal here that does not poison a
            // frame: no frame is open, and no overlap is a statement about the
            // channel rather than about an edit. The agreement stays `None`, so
            // every later entry is refused as `NotNegotiated` until the epoch
            // moves — which is the honest outcome for two peers that speak no
            // common vocabulary.
            match handshake.negotiate() {
                Ok(agreed) => self.agreed = Some(agreed),
                Err(refusal) => return Err(Fault { refusal, at }),
            }
        }

        if closes {
            self.at = 0;
        }
        Ok(delta)
    }
}

// The two facts `sqe_bytes` rests on. The first is also asserted in `lib.rs`,
// beside the type; it is asserted again here because this is the code that
// would be unsound without it, and an assertion in another file is a fact this
// one is trusting rather than stating.
const _: () = assert!(core::mem::size_of::<Sqe>() == SQE_BYTES);
// No padding: a `#[repr(C)]` type whose size equals the sum of its field widths
// has none anywhere. Written as the sum rather than as `64`, so that the
// arithmetic is in front of whoever changes a field.
const _: () = assert!(SQE_BYTES == 1 + 1 + 2 + 4 + 8 + 8 + 8 + 4 + 4 + 4 + 4 + 16);

// Every record fits the one payload width, asserted beside the records rather
// than in a table somewhere else. A field added to any of these that would not
// fit stops the build here, which is the whole reason `Writer::put` may index.
const _: () = assert!(Handshake::WIDTH <= PAYLOAD_BYTES);
const _: () = assert!(DeclareNode::WIDTH <= PAYLOAD_BYTES);
const _: () = assert!(SetState::WIDTH <= PAYLOAD_BYTES);
const _: () = assert!(SetContent::WIDTH <= PAYLOAD_BYTES);
const _: () = assert!(SetRelations::WIDTH <= PAYLOAD_BYTES);
const _: () = assert!(Remove::WIDTH <= PAYLOAD_BYTES);
const _: () = assert!(Commit::WIDTH <= PAYLOAD_BYTES);
// And the stride is the widest record's width rounded to eight. If a record
// ever grows past it, the assertion above it is what fails; this one is here so
// that a *narrowing* — a field deleted, leaving the stride eight bytes wider
// than anything uses — is also visible rather than free.
const _: () = assert!(PAYLOAD_BYTES == SetRelations::WIDTH.next_multiple_of(8));
// The flags this format accepts are flags the ABI defines. A bit removed from
// `flags::KNOWN` and left here would be a bit this build accepts on an entry
// and the ring's own envelope check refuses.
const _: () = assert!(FLAGS_ACCEPTED & !flags::KNOWN == 0);
// Exactly one opcode agrees the vocabulary. Two would make
// `Refusal::Renegotiated` a rule about whichever of them arrived first, and
// none would make it unreachable — a refusal nothing can earn. Counted over
// `op::ALL`, which is emitted from the same list the column is, so an eighth
// opcode is counted the day it is declared.
const _: () = {
    let mut handshakes = 0;
    let mut i = 0;
    while i < op::COUNT {
        if matches!(op::is_handshake(op::ALL[i]), Some(true)) {
            handshakes += 1;
        }
        i += 1;
    }
    assert!(handshakes == 1);
};
// This build's own offer is one `Handshake::negotiate` would not refuse as
// malformed. A floor raised past the ceiling by an edit to one constant is a
// build that cannot talk to itself.
const _: () = assert!(VOCABULARY_VERSION_MIN <= VOCABULARY_VERSION);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class;

    /// The file that declares the vocabulary, read when this file is compiled.
    ///
    /// **This is not a dependency.** `interface/` is a leaf with no
    /// dependencies and nothing depends on it, which is argued in its own
    /// manifest and is why this crate cannot import `Role`. No crate is linked
    /// here and no type is imported: `include_str!` reads a text file at
    /// compile time, which needs neither a filesystem at run time nor an
    /// allocator, and `interface/src/ladder.rs` already holds itself to
    /// `claims/0033` and RFC 0080 in exactly this way.
    ///
    /// The cost is real and is the intended one: editing `node.rs` rebuilds
    /// this crate's tests. The wire's ordinals and the list they index are one
    /// decision written in two directories, and a rebuild is the cheapest
    /// possible way for that to be a fact the toolchain knows rather than a
    /// sentence in a comment.
    const VOCABULARY: &str = include_str!("../../interface/src/node.rs");

    /// The RFC that closed the vocabulary, read when this file is compiled.
    ///
    /// Read for one thing: the census. A count in a decision document that
    /// nothing recomputes is a count that goes stale on the first edit to the
    /// list, and this file's exit quotes that count.
    const RFC_0077: &str = include_str!("../../docs/rfc/0077-the-semantic-vocabulary-is-closed.md");

    /// The RFC this module implements, read when this file is compiled.
    ///
    /// Read so that reversing the decision goes red here rather than leaving a
    /// module that implements a document nobody believes any more. RFC 0083's
    /// own *what would reverse this* names *the refusal moving into a
    /// projection* and *a second decoder*; neither is observable from this
    /// crate, so what is observable is asserted instead — that the sentences
    /// this file rests on are still in the file that decided them.
    const RFC_0083: &str = include_str!(
        "../../docs/rfc/0083-a-vocabulary-version-is-negotiated-and-an-unknown-role-is-not-representable.md"
    );

    /// How many roles this parser will hold before it gives up.
    ///
    /// Not a claim about the vocabulary — the vocabulary's size is read here,
    /// never written. It is the fixed array a `#![no_std]` test has instead of
    /// a `Vec`, set far enough above twenty-two that reaching it means the list
    /// grew by an order of magnitude, which is worth a red test of its own.
    const ROLE_LIMIT: usize = 64;

    /// Version 1's list, as a digest over its role names in declaration order.
    ///
    /// **This literal records the wire as shipped. It is never updated to match
    /// the list.** RFC 0083's part two is that a version ordinal is worth
    /// negotiating only if it identifies the same list on both sides, and the
    /// failure it is aimed at is the one nobody sees: two builds both saying
    /// version 1, both honest, holding lists whose order differs, so that 14 is
    /// `Toggle` on one and `Command` on the other. Nothing is unknown, every
    /// entry decodes, every projection is total, and a settings panel is
    /// announced wrongly forever.
    ///
    /// A reordering is therefore what this catches, and a reordering is the
    /// diff that looks like tidying: alphabetising the list, grouping the roles
    /// by family, or deleting a role nobody used. If this goes red, the
    /// question is not *what is the new digest*, it is *which version is that*.
    ///
    /// FNV-1a over the names joined by a newline. A cryptographic digest would
    /// buy nothing — nobody is attacking this number, and RFC 0083 refused
    /// putting a digest on the wire precisely because *which build is this* is
    /// attestation's question and RFC 0012 owns it.
    const VERSION_1_DIGEST: u64 = 0x87aa_8dab_7f22_46ef;

    /// The FNV-1a 64-bit offset basis.
    const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

    /// The FNV-1a 64-bit prime.
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    /// Relation ordinals this fixture's vocabulary names.
    ///
    /// The fixture's own number and not a reading of `interface`: the exit this
    /// file is accepted on is about roles, so the role count is read from the
    /// declaring list, and this is a stand-in chosen so that an ordinal above
    /// it can be shown to be refused by the same rule.
    const FIXTURE_RELATIONS: u16 = 4;

    /// Content ordinals this fixture's vocabulary names. See
    /// [`FIXTURE_RELATIONS`].
    const FIXTURE_CONTENT: u16 = 4;

    /// The state bits this fixture's vocabulary names: `interface`'s five.
    const FIXTURE_STATES: u8 = 0b0001_1111;

    /// A vocabulary that names the ordinals `interface/src/node.rs` declares.
    ///
    /// What a real implementor does is derive the answer from `Role::ALL`
    /// filtered by `since`; what this does is derive it from the same list read
    /// as text, because this crate cannot see that one. Both are derivations of
    /// the one list, which is the property that matters — neither is a table
    /// somebody typed.
    struct Declared {
        /// How many roles the list declares.
        roles: u16,
    }

    impl Vocabulary for Declared {
        fn names(&self, field: Closed, ordinal: u16, _agreed: Agreed) -> bool {
            // Exhaustive and wildcard-free on purpose: a fourth closed field
            // stops this fixture rather than being silently admitted, which is
            // the same demand the module makes of a real implementor.
            match field {
                Closed::Role => ordinal < self.roles,
                Closed::Relation => ordinal < FIXTURE_RELATIONS,
                Closed::Content => ordinal < FIXTURE_CONTENT,
            }
        }

        fn state_bits(&self, _agreed: Agreed) -> u8 {
            FIXTURE_STATES
        }
    }

    /// The role names the `vocabulary!` invocation declares, in order.
    ///
    /// A parse rather than a copy. It fails loudly — a panic is a red test —
    /// rather than skipping a line it does not understand, because a parser
    /// that silently skips is a parser that reports a shorter list and a digest
    /// over the wrong thing.
    fn declared_roles() -> ([&'static str; ROLE_LIMIT], usize) {
        let mut names = [""; ROLE_LIMIT];
        let mut found = 0;
        let mut inside = false;
        for line in VOCABULARY.lines() {
            let text = line.trim();
            if !inside {
                // The `macro_rules! vocabulary {` that *defines* the macro
                // trims to a different string, so this reaches the invocation
                // and not the definition.
                inside = text == "vocabulary! {";
                continue;
            }
            if text == "}" {
                return (names, found);
            }
            if text.is_empty() || text.starts_with("///") {
                continue;
            }
            let Some(open) = text.find('"') else {
                panic!("a `vocabulary!` line carries no spelling: `{text}`")
            };
            let rest = &text[open + 1..];
            let Some(close) = rest.find('"') else {
                panic!("a `vocabulary!` line's spelling does not close: `{text}`")
            };
            assert!(found < ROLE_LIMIT, "the vocabulary has outgrown this parser's array");
            names[found] = &rest[..close];
            found += 1;
        }
        panic!("`vocabulary! {{` never closes in interface/src/node.rs")
    }

    /// A `usize` constant `interface/src/node.rs` declares, read out of it.
    fn declared_usize(name: &str) -> usize {
        let needle = "pub const ";
        let mut from = 0;
        while let Some(at) = VOCABULARY[from..].find(needle) {
            let start = from + at + needle.len();
            let rest = &VOCABULARY[start..];
            from = start;
            let Some(eq) = rest.find('=') else { continue };
            if !rest[..eq].starts_with(name) {
                continue;
            }
            let Some(end) = rest[eq..].find(';') else { continue };
            let value = rest[eq + 1..eq + end].trim();
            return value.parse().unwrap_or_else(|_| panic!("`{name}` is not a number: `{value}`"));
        }
        panic!("interface/src/node.rs declares no `{name}`")
    }

    /// FNV-1a over the names, newline-separated.
    ///
    /// Seedless and stateless: a pure function of the list, which is what RFC
    /// 0004 asks of anything that must produce the same answer on two
    /// architectures.
    fn digest(names: &[&str]) -> u64 {
        let mut hash = FNV_BASIS;
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                hash = (hash ^ u64::from(b'\n')).wrapping_mul(FNV_PRIME);
            }
            for byte in name.as_bytes() {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
            }
        }
        hash
    }

    /// The agreement this build reaches with a peer exactly like it.
    fn agreed() -> Agreed {
        Handshake::HERE.negotiate().expect("this build speaks its own vocabulary")
    }

    /// A vocabulary naming what the declaring list names.
    fn vocabulary() -> Declared {
        let (_, roles) = declared_roles();
        Declared { roles: u16::try_from(roles).expect("the vocabulary fits a wire ordinal") }
    }

    /// One entry as it crosses.
    fn crossing(body: Entry) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        Delta { user_data: 0x0dd_ba11, class: class::SOFT, payload_offset: 8, flags: 0, body }
            .encode()
    }

    /// A `DeclareNode` carrying an ordinal, whatever the vocabulary thinks of
    /// it.
    ///
    /// Only this module can write one, because `RoleOrdinal`'s field is private
    /// — which is the property under test, read from the inside. A peer on the
    /// far side of a ring writes whatever bytes it likes, and this is how those
    /// bytes are produced without a second encoder.
    fn node_of_role(ordinal: u16) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        crossing(Entry::DeclareNode(DeclareNode {
            role: RoleOrdinal(ordinal),
            ..DeclareNode::SPECIMEN
        }))
    }

    #[test]
    fn the_role_ordinals_on_the_wire_are_the_indices_the_vocabulary_declares() {
        // The exit clause, and it has three moving parts rather than two, so
        // all three are read here.
        //
        // *The list.* `vocabulary!`'s lines, in order, parsed out of the file
        // that declares them. RFC 0077 says twenty-two, and both the count and
        // the sentence are asserted, so that a role added without the census
        // being argued goes red naming the document.
        //
        // *The index.* `Role::index` is the enum's discriminant and `Role::ALL`
        // is the declaration order, so the *i*-th line's role has index *i*.
        // Both are read as text: if `index` ever stops being `self as usize`,
        // or `ALL` stops being emitted in declaration order, the wire's
        // ordinals stop being `Role::index` values and this says so.
        //
        // *The wire.* Every ordinal the list declares crosses and comes back as
        // itself, through this format's own encoder and decoder; the first
        // ordinal past the list is refused.
        //
        // *The edits that make this go red:* appending a role to `vocabulary!`;
        // deleting one; changing `Role::index` to anything but the
        // discriminant; changing `Role::ALL` to anything but declaration order;
        // biasing the wire ordinal; widening or narrowing the role field.
        let (names, count) = declared_roles();
        assert_eq!(count, 22, "`vocabulary!` declares {count} roles");
        assert!(
            RFC_0077.contains("twenty-two roles"),
            "RFC 0077 no longer says the vocabulary has twenty-two roles"
        );

        assert!(
            VOCABULARY.contains("pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];"),
            "`Role::ALL` is no longer the declaration order of `vocabulary!`"
        );
        let Some(at) = VOCABULARY.find("pub const fn index(self) -> usize") else {
            panic!("`Role::index` is gone; the wire's ordinal has no definition to be")
        };
        let after = &VOCABULARY[at..];
        let body = after.split('}').next().unwrap_or(after);
        assert!(
            body.contains("self as usize"),
            "`Role::index` is no longer the enum's discriminant, so it is no longer the \
             position of the line that declared the role"
        );

        let vocabulary = vocabulary();
        let agreed = agreed();
        for (index, name) in names.iter().take(count).enumerate() {
            let ordinal = u16::try_from(index).expect("a role index fits a wire ordinal");
            let (entry, payload) = node_of_role(ordinal);
            let delta = Delta::decode(&entry, &payload, Some(agreed), &vocabulary)
                .unwrap_or_else(|refusal| panic!("`{name}` at {ordinal}: {}", refusal.message()));
            let Entry::DeclareNode(declared) = delta.body else {
                panic!("a `DeclareNode` decoded as something else")
            };
            assert_eq!(declared.role.get(), ordinal, "`{name}` moved on the wire");
        }

        let past = u16::try_from(count).expect("a role count fits a wire ordinal");
        let (entry, payload) = node_of_role(past);
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Unnamed { field: Closed::Role, ordinal: past }),
            "the first ordinal past the list is not refused, so the wire admits more \
             ordinals than the vocabulary declares"
        );
    }

    #[test]
    fn version_one_s_role_list_is_frozen() {
        // The other half of *if either moves*. The test above asserts that
        // ordinal *i* on the wire is the *i*-th declared role; this asserts
        // *which* role that is, which a reorder changes without changing any
        // count and without breaking a single round trip.
        //
        // The digest is over the whole `vocabulary!` list, which is version 1's
        // list exactly while `VOCABULARY_VERSION` is 1. The day a role is
        // appended for version 2, this has to start filtering by the `since`
        // column RFC 0083 owes `interface/src/node.rs`, and a second digest is
        // added for version 2 — the literal below is not edited. That is the
        // whole value of it: it records a fact that has already happened, and
        // it does not follow the list.
        assert_eq!(
            VOCABULARY_VERSION, 1,
            "the digest below is version 1's list; a version 2 needs its own, and this one \
             stays as it is"
        );
        let (names, count) = declared_roles();
        assert_eq!(
            digest(&names[..count]),
            VERSION_1_DIGEST,
            "version 1's role list is not the list that shipped. If a role was appended, \
             this needs a `since` column and a second digest, not a new literal here; if \
             the list was reordered or a role renamed, every deployed peer's ordinals now \
             mean something else."
        );
    }

    #[test]
    fn an_ordinal_the_vocabulary_does_not_name_is_refused_rather_than_mapped() {
        // *Refused* and not *mapped*: the assertion is on the shape of the
        // result, not only on its contents. A decode that produced a `Delta`
        // and a note would be the failure this module exists to not have, so
        // what is checked is that there is no `Delta` at all — the ordinal
        // reaches nothing, becomes nothing, and substitutes for nothing.
        //
        // What cannot be tested from here, and is structural instead: there is
        // no constructor for `RoleOrdinal` outside this module, so no code
        // anywhere else can manufacture the value a fallback would need. This
        // test can build one only because it is inside the module — which is
        // how a hostile peer's bytes are produced without a second encoder.
        let vocabulary = vocabulary();
        let agreed = agreed();
        let (_, count) = declared_roles();
        let past = u16::try_from(count).expect("a role count fits a wire ordinal");

        for ordinal in [past, past + 1, 200, u16::MAX] {
            let (entry, payload) = node_of_role(ordinal);
            let outcome = Delta::decode(&entry, &payload, Some(agreed), &vocabulary);
            assert_eq!(
                outcome,
                Err(Refusal::Unnamed { field: Closed::Role, ordinal }),
                "ordinal {ordinal} produced something other than a refusal naming it"
            );
        }

        // The same rule, and literally the same rule, for the other closed
        // fields that cross. A per-enum policy is what RFC 0083 refused.
        let (entry, payload) = crossing(Entry::SetRelations(SetRelations {
            edges: [
                Edge { kind: RelationOrdinal(FIXTURE_RELATIONS), target: 3 },
                SetRelations::NO_EDGE,
                SetRelations::NO_EDGE,
                SetRelations::NO_EDGE,
            ],
            ..SetRelations::SPECIMEN
        }));
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Unnamed { field: Closed::Relation, ordinal: FIXTURE_RELATIONS })
        );

        let (entry, payload) = crossing(Entry::SetContent(SetContent {
            kind: ContentOrdinal(FIXTURE_CONTENT),
            ..SetContent::SPECIMEN
        }));
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Unnamed { field: Closed::Content, ordinal: FIXTURE_CONTENT })
        );

        // And a state bit nobody agreed. `interface`'s `from_bits` keeps every
        // bit rather than masking; this is the layer that decides what the
        // evidence is for, and the answer is the same refusal.
        let unknown = FIXTURE_STATES | 1 << 5;
        let (entry, payload) =
            crossing(Entry::SetState(SetState { state: StateBits(unknown), ..SetState::SPECIMEN }));
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::UnknownState { bits: unknown })
        );
    }

    #[test]
    fn the_wire_carries_as_many_relations_as_a_node_has() {
        // `RELATIONS_MAX` is stated twice — once by the vocabulary, once by the
        // wire that has to hold a whole set in one payload — and this is what
        // stops the two drifting quietly. If `interface` raises its bound, a
        // node's relations no longer fit one entry and `SetRelations` stops
        // being atomic without the commit's help, which is an ABI change and
        // should arrive as a red build rather than as a truncated set.
        assert_eq!(
            declared_usize("RELATIONS_MAX"),
            RELATIONS_MAX,
            "a node may hold a number of relations this entry format cannot carry"
        );
    }

    #[test]
    fn every_opcode_round_trips() {
        // The corpus is derived from the list that declares the opcodes, so an
        // eighth is covered on the day it is written. A hand-written table here
        // is exactly how this test would stay green while the sentence above it
        // stopped being true.
        let vocabulary = vocabulary();
        let agreed = agreed();
        assert_eq!(Entry::specimens().len(), op::COUNT);
        for body in Entry::specimens() {
            // The handshake is the one entry that crosses before an agreement
            // and is refused after one, so it is offered the state it belongs
            // in. That asymmetry is the protocol and not a special case.
            let standing =
                if op::is_handshake(body.opcode()) == Some(true) { None } else { Some(agreed) };
            let (entry, payload) = crossing(body);
            let delta = Delta::decode(&entry, &payload, standing, &vocabulary)
                .unwrap_or_else(|r| panic!("{}: {}", op::label(body.opcode()), r.message()));
            assert_eq!(delta.body, body, "{} did not survive", op::label(body.opcode()));
            assert_eq!(delta.opcode(), body.opcode());
            assert!(op::known(body.opcode()));
        }
    }

    #[test]
    fn no_record_writes_past_its_declared_width() {
        // `Writer::put` indexes rather than checking, and the bound it rests on
        // is the const assertion beside each record. This is the other half:
        // every record's writer stops inside its own declared width, so the
        // assertion is about a number the writer actually honours.
        for body in Entry::specimens() {
            let payload = body.payload();
            let written = payload.iter().rposition(|byte| *byte != 0).map_or(0, |at| at + 1);
            assert!(
                written <= body.width(),
                "{} declares {} bytes and wrote {written}",
                op::label(body.opcode()),
                body.width()
            );
            assert!(body.width() <= PAYLOAD_BYTES);
        }
    }

    #[test]
    fn an_unread_byte_of_the_payload_is_refused() {
        // The tail rule, on the record that shows what it buys: `SetRelations`
        // reads exactly `count` slots, so a stale byte in an unused slot is a
        // field the record did not read, and the rule that was already there
        // refuses it. No hand-written *and the rest must be empty* check exists
        // here, deliberately — a hand-written check is what a fifth slot gets
        // left out of.
        let vocabulary = vocabulary();
        let agreed = agreed();
        let (entry, mut payload) = crossing(Entry::SetRelations(SetRelations::SPECIMEN));
        assert!(Delta::decode(&entry, &payload, Some(agreed), &vocabulary).is_ok());
        payload[PAYLOAD_BYTES - 1] = 1;
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Reserved)
        );

        // And the same for a byte inside an unused edge slot, which is what a
        // producer reusing a dirty arena slot actually leaves behind.
        let (entry, mut payload) = crossing(Entry::SetRelations(SetRelations::SPECIMEN));
        payload[8 + 2 + SetRelations::SLOT] = 2;
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Reserved)
        );
    }

    #[test]
    fn a_deadline_is_refused_because_no_semantic_opcode_reads_one() {
        // `Delta` has no deadline field, so this cannot be produced by any
        // encoder here — it is a peer's entry, built by hand. The
        // whole-envelope comparison is what refuses it, which means a field
        // added to `Sqe` by a later ABI is refused the same way without anybody
        // adding a line to a list.
        let vocabulary = vocabulary();
        let agreed = agreed();
        let (mut entry, payload) = crossing(Entry::Commit(Commit::SPECIMEN));
        assert!(Delta::decode(&entry, &payload, Some(agreed), &vocabulary).is_ok());
        entry.deadline = 1;
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Reserved)
        );

        // A capability index, for the same reason and by the same comparison.
        let (mut entry, payload) = crossing(Entry::Commit(Commit::SPECIMEN));
        entry.cap = 4;
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::Reserved)
        );

        // A flag outside the accepted set is not caught by the comparison —
        // `envelope` copies the flags it was given — so it has its own rule,
        // and `check` is the producer's half of that rule.
        let (mut entry, payload) = crossing(Entry::Commit(Commit::SPECIMEN));
        entry.flags = flags::LINK;
        assert_eq!(
            Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
            Err(Refusal::UnknownFlag)
        );
        let refused = Delta {
            user_data: 0,
            class: 0,
            payload_offset: 0,
            flags: flags::DRAIN,
            body: Entry::Commit(Commit::SPECIMEN),
        };
        assert_eq!(refused.check(), Err(Refusal::UnknownFlag));
    }

    #[test]
    fn nothing_crosses_before_the_vocabulary_is_agreed() {
        // RFC 0083's first refusal. Every opcode but the handshake, because a
        // check that lived where the ordinals are read would let `Remove` and
        // `Commit` through — they carry no ordinal — and a channel that can
        // remove nodes before agreeing what a node is is a channel whose
        // handshake is decoration.
        let vocabulary = vocabulary();
        for body in Entry::specimens() {
            if op::is_handshake(body.opcode()) == Some(true) {
                continue;
            }
            let (entry, payload) = crossing(body);
            assert_eq!(
                Delta::decode(&entry, &payload, None, &vocabulary),
                Err(Refusal::NotNegotiated),
                "{} crossed before the vocabulary was agreed",
                op::label(body.opcode())
            );
        }
    }

    #[test]
    fn the_vocabulary_is_agreed_once_per_epoch() {
        // RFC 0083's second refusal, and its bound. Re-negotiating mid-stream
        // would change what an index means underneath a tree already built out
        // of the old meaning; moving the epoch already discards every
        // outstanding token, so the epoch is the unit that may agree again.
        let vocabulary = vocabulary();
        let mut session = Session::opening(1);
        let (entry, payload) = crossing(Entry::DeclareVocabulary(Handshake::HERE));
        assert!(session.accept(&entry, &payload, &vocabulary).is_ok());
        assert_eq!(session.agreed().map(Agreed::version), Some(VOCABULARY_VERSION));

        assert_eq!(
            session.accept(&entry, &payload, &vocabulary),
            Err(Fault { refusal: Refusal::Renegotiated, at: 1 })
        );

        // The poison a refused handshake left is cleared by the commit that
        // ends the frame, the way any other refusal's is.
        let (commit, commit_payload) = crossing(Entry::Commit(Commit::SPECIMEN));
        assert_eq!(
            session.accept(&commit, &commit_payload, &vocabulary),
            Err(Fault { refusal: Refusal::Renegotiated, at: 1 })
        );

        assert!(session.follow_epoch(2), "a moved epoch discards the agreement");
        assert_eq!(session.agreed(), None);
        assert!(!session.follow_epoch(2), "the same epoch discards nothing");
        assert!(session.accept(&entry, &payload, &vocabulary).is_ok());
    }

    #[test]
    fn no_common_version_is_a_peer_refusal_and_not_a_degraded_channel() {
        // RFC 0083's third, and the one that reuses rather than invents: a peer
        // whose floor is above this build's ceiling is `PEER` /
        // `VERSION_UNSUPPORTED`, which already means exactly this one level up.
        // The alternative — opening the channel anyway and degrading — is the
        // failure the whole module is arranged against, arriving at setup.
        let too_new = Handshake { highest: VOCABULARY_VERSION + 9, floor: VOCABULARY_VERSION + 1 };
        assert_eq!(too_new.negotiate(), Err(Refusal::VersionUnsupported));
        assert_eq!(
            Refusal::VersionUnsupported.packed(),
            error::pack(error::PEER, error::peer::VERSION_UNSUPPORTED)
        );

        // A peer ahead of this build but still speaking version 1 meets it in
        // the middle, which is the whole point of a floor.
        let newer = Handshake { highest: VOCABULARY_VERSION + 9, floor: VOCABULARY_VERSION_MIN };
        assert_eq!(newer.negotiate().map(Agreed::version), Ok(VOCABULARY_VERSION));

        // A floor above a ceiling is a statement no honest build makes, and a
        // peer that offers no version at all has not made one.
        assert_eq!(Handshake { highest: 1, floor: 2 }.negotiate(), Err(Refusal::Malformed));
        let (entry, payload) =
            crossing(Entry::DeclareVocabulary(Handshake { highest: 0, floor: 0 }));
        assert_eq!(Delta::decode(&entry, &payload, None, &vocabulary()), Err(Refusal::Malformed));
    }

    #[test]
    fn a_refused_entry_poisons_the_frame_and_the_commit_carries_it() {
        // RFC 0083's granularity: not the entry, because the entries are not
        // independent and a tree with a hole in it is a screenshot nobody
        // questions; not the channel, because one bad frame from a peer that
        // may simply be newer is not grounds to destroy a working interface.
        // The frame, because the commit boundary already exists and is already
        // atomic.
        let vocabulary = vocabulary();
        let mut session = Session::opening(1);
        let (hello, hello_payload) = crossing(Entry::DeclareVocabulary(Handshake::HERE));
        assert!(session.accept(&hello, &hello_payload, &vocabulary).is_ok());

        let (good, good_payload) = crossing(Entry::DeclareNode(DeclareNode::SPECIMEN));
        assert!(session.accept(&good, &good_payload, &vocabulary).is_ok());

        let (_, count) = declared_roles();
        let past = u16::try_from(count).expect("a role count fits a wire ordinal");
        let (bad, bad_payload) = node_of_role(past);
        let fault =
            Fault { refusal: Refusal::Unnamed { field: Closed::Role, ordinal: past }, at: 2 };
        assert_eq!(session.accept(&bad, &bad_payload, &vocabulary), Err(fault));
        assert_eq!(session.poisoned(), Some(fault));

        // Every later entry of the frame is refused with the *first* fault, so
        // there is no entry a caller could be handed and apply — which is what
        // makes *the tree standing is unchanged* something the type helps with
        // rather than something a comment asks for.
        assert_eq!(session.accept(&good, &good_payload, &vocabulary), Err(fault));

        // And the commit fails with the same fault, naming the entry that
        // caused it rather than the one that discovered it.
        let (commit, commit_payload) = crossing(Entry::Commit(Commit::SPECIMEN));
        assert_eq!(session.accept(&commit, &commit_payload, &vocabulary), Err(fault));

        // The next frame is clean. A peer one version ahead sends one bad
        // frame, learns from its completion, and keeps its channel.
        assert_eq!(session.poisoned(), None);
        assert!(session.accept(&good, &good_payload, &vocabulary).is_ok());
        assert!(session.accept(&commit, &commit_payload, &vocabulary).is_ok());
    }

    #[test]
    fn a_zeroed_payload_names_nothing() {
        // The role ordinal is deliberately not biased by one — the wire value
        // is `Role::index` and nothing arithmetic stands between them — so the
        // work a bias would have done is done by the node identifier, and this
        // is where that substitution is checked rather than assumed.
        let vocabulary = vocabulary();
        let agreed = agreed();
        let zeroed = [0u8; PAYLOAD_BYTES];
        for body in Entry::specimens() {
            if op::is_handshake(body.opcode()) == Some(true) {
                continue;
            }
            let (entry, _) = crossing(body);
            assert!(
                Delta::decode(&entry, &zeroed, Some(agreed), &vocabulary).is_err(),
                "{} believes a zeroed payload",
                op::label(body.opcode())
            );
        }
        // Specifically: it is the identifier that refuses, not the ordinal,
        // because ordinal zero is a role — the first one the list declares.
        let (entry, _) = crossing(Entry::DeclareNode(DeclareNode::SPECIMEN));
        assert_eq!(Delta::decode(&entry, &zeroed, Some(agreed), &vocabulary), Err(Refusal::NoNode));
    }

    #[test]
    fn a_node_cannot_be_its_own_parent_sibling_or_target() {
        // The only cycles one entry can state on its own, so the only ones this
        // format can refuse. A cycle spread over two entries belongs to the
        // tree's own `check`, and claiming otherwise here would be a guard that
        // covers a third of what its name says.
        let vocabulary = vocabulary();
        let agreed = agreed();
        for body in [
            Entry::DeclareNode(DeclareNode { parent: 7, ..DeclareNode::SPECIMEN }),
            Entry::DeclareNode(DeclareNode { before: 7, ..DeclareNode::SPECIMEN }),
            Entry::SetRelations(SetRelations {
                edges: [
                    Edge { kind: RelationOrdinal(0), target: 7 },
                    SetRelations::NO_EDGE,
                    SetRelations::NO_EDGE,
                    SetRelations::NO_EDGE,
                ],
                ..SetRelations::SPECIMEN
            }),
        ] {
            let (entry, payload) = crossing(body);
            assert_eq!(
                Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
                Err(Refusal::Value)
            );
        }

        // A relation count past what the payload holds, which is the other
        // thing a producer gets wrong about this record.
        let (entry, payload) =
            crossing(Entry::SetRelations(SetRelations { count: 5, ..SetRelations::SPECIMEN }));
        assert_eq!(Delta::decode(&entry, &payload, Some(agreed), &vocabulary), Err(Refusal::Value));
    }

    #[test]
    fn an_unknown_opcode_is_refused_and_never_skipped() {
        // R04. The negative answer from the list is what makes this a refusal
        // rather than an entry a consumer walks past, and `is_handshake`
        // answering `None` rather than `false` is what stops an unknown opcode
        // being treated as an ordinary edit.
        let vocabulary = vocabulary();
        let agreed = agreed();
        let (mut entry, payload) = crossing(Entry::Commit(Commit::SPECIMEN));
        for opcode in [0u8, 0x40, 0xfe, 0xff] {
            entry.opcode = opcode;
            assert!(!op::known(opcode));
            assert_eq!(op::is_handshake(opcode), None);
            assert_eq!(op::label(opcode), "unknown");
            assert_eq!(
                Delta::decode(&entry, &payload, Some(agreed), &vocabulary),
                Err(Refusal::UnknownOpcode)
            );
        }
        for opcode in op::ALL {
            assert!(op::known(opcode));
            assert!(op::is_handshake(opcode).is_some());
        }
    }

    #[test]
    fn every_refusal_packs_into_a_domain_rfc_0010_already_fixes() {
        // No new `argument` code, which is the module's *why the refusals are
        // local*: sibling entry formats are being written against this model,
        // and a code invented in each of four places is four meanings wearing
        // one number. Exactly one refusal leaves `ARGUMENT`, and it is the one
        // about the peer rather than about a frame.
        let every = [
            Refusal::UnknownOpcode,
            Refusal::UnknownFlag,
            Refusal::Reserved,
            Refusal::Malformed,
            Refusal::Value,
            Refusal::NoNode,
            Refusal::Unnamed { field: Closed::Role, ordinal: 99 },
            Refusal::UnknownState { bits: 0x80 },
            Refusal::NotNegotiated,
            Refusal::Renegotiated,
            Refusal::VersionUnsupported,
        ];
        for refusal in every {
            let Some((domain, _)) = error::unpack(refusal.packed()) else {
                panic!("{} does not unpack", refusal.message())
            };
            let expected =
                if refusal == Refusal::VersionUnsupported { error::PEER } else { error::ARGUMENT };
            assert_eq!(domain, expected, "{}", refusal.message());
            assert!(!refusal.message().is_empty());
        }
        assert_eq!(
            Refusal::NotNegotiated.packed(),
            error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
            "the worked precedent RFC 0083 names is `FEATURE_NOT_NEGOTIATED` in `ARGUMENT`"
        );
    }

    #[test]
    fn the_closed_fields_are_one_list_and_each_names_itself() {
        // The `admitted!` list is the only place a closed field is written, so
        // this is not a check on an array — there is nothing for a field to be
        // omitted from. What it does check is the property a log line rests on:
        // an ordinal says which field it belongs to, and two fields do not
        // share a label.
        assert_eq!(Closed::ALL.len(), Closed::COUNT);
        for (i, field) in Closed::ALL.iter().enumerate() {
            for other in &Closed::ALL[i + 1..] {
                assert_ne!(field.label(), other.label());
            }
        }
        assert_eq!(RoleOrdinal(0).field(), Closed::Role);
        assert_eq!(RelationOrdinal(0).field(), Closed::Relation);
        assert_eq!(ContentOrdinal(0).field(), Closed::Content);
    }

    #[test]
    fn rfc_0083_still_decides_what_this_module_implements() {
        // A module implementing a decision nobody believes any more is worse
        // than one implementing none, because the argument for every refusal
        // above lives in that file rather than in this one. These are the
        // clauses this file rests on; if one is edited out, the code resting on
        // it should go red rather than stand there citing it.
        // Quoted in fragments that sit on one line of the source document,
        // because a clause split across a wrap would make this test fail for
        // reflowing rather than for reversing — a test that goes red for the
        // wrong reason is a test people learn to edit.
        for clause in [
            "refused by not being representable",
            "there is no third outcome",
            "reported by its position and its ordinal",
            "pending edit is poisoned",
            "entry on the channel and may appear once per channel epoch",
            "the vocabulary's indices are **append-only**",
        ] {
            assert!(RFC_0083.contains(clause), "RFC 0083 no longer says `{clause}`");
        }
    }
}
