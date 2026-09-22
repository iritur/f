// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The vocabulary `f_abi::semantic` admits against, read out of
//! `f_interface::node` rather than copied from it.
//!
//! # Why this exists, and who said it was owed
//!
//! RFC 0083 part two states the admission rule in one line — an ordinal is
//! admitted when `from_index(i)` answers **and** `since(role) <= agreed` — and
//! records that `abi` has no implementor of [`Vocabulary`] outside its own test
//! fixtures, so until one is written the agreed version constrains no admission
//! anywhere and an older receiver is protected by its own list length, which is
//! what it would have had with no handshake at all. `E3-B06c` is named there as
//! the task that writes one. This is it.
//!
//! # What makes it not a second copy of the vocabulary
//!
//! [`Closed::Role`] is answered by calling `Role::from_index` and `Role::since`,
//! which are emitted from the `vocabulary!` invocation that declares the roles —
//! so a role appended there is admitted here on the day it is appended, and a
//! role removed stops being admitted, without anybody editing this file.
//!
//! The other two closed fields are not so lucky, and it is worth saying exactly
//! why rather than leaving a reader to find it. `f_interface::node::Relation`
//! and `Content` are enums whose variants **carry data** — a relation is an edge
//! to a `NodeId`, a content is the text or the reading itself — so neither has
//! an ordinal map to read: there is no `Relation::from_index`, because there is
//! no `Relation` without its target. The map is therefore here, as an exhaustive
//! `match`, and exhaustiveness is what holds it: a fifth variant added to either
//! enum stops this build with a non-exhaustive match and has to be given an
//! ordinal deliberately.
//!
//! What exhaustiveness does *not* hold is [`Interface::RELATIONS_NAMED`] and
//! [`Interface::CONTENTS_NAMED`], which are counted from the specimen arrays
//! below. A variant given a match arm and left out of its array is a variant
//! whose ordinal this vocabulary then **refuses** — the conservative direction,
//! and the reason the arrays sit beside the matches rather than in another file.
//! *What would reverse this:* a kind enum in `interface`, emitted from one list
//! the way `Role` is, at which point these two matches go the way `Role::ALL`
//! went. `E3-B06f` is the task that first needs a content body to have somewhere
//! to go, and it is the one that should pay for it.

use f_abi::semantic::{Agreed, Closed, VOCABULARY_VERSION_MIN, Vocabulary};
use f_interface::node::{
    CapRef, Content, NodeId, Quantity, Reading, Relation, Role, StateSet, Text, Unit,
};

/// RFC 0077's vocabulary, as something `f_abi::semantic` can admit against.
///
/// A unit struct with no state, because a vocabulary is a fact about a build and
/// not about a channel: what varies per channel is the *agreed version*, and
/// that arrives as an argument. A type holding a version would be a second place
/// the agreement lives, and `f_abi::semantic::Session` is the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interface;

impl Interface {
    /// One specimen of every relation kind this build names.
    ///
    /// The targets are arbitrary and never read: what is read is the variant,
    /// through [`Self::relation_ordinal`]. Written as specimens rather than as a
    /// count, so that the count below is derived from a list a reader can check
    /// against the enum by eye — which is the weakest of the three mechanisms
    /// here and is named as such in the module comment.
    const RELATIONS: [Relation; 4] = [
        Relation::LabelledBy(NodeId::UNNAMED),
        Relation::Controls(NodeId::UNNAMED),
        Relation::Owns(NodeId::UNNAMED),
        Relation::FlowsTo(NodeId::UNNAMED),
    ];

    /// One specimen of every content kind this build names, on
    /// [`Self::RELATIONS`]' terms exactly.
    const CONTENTS: [Content; 4] = [
        Content::None,
        Content::Text(Text::EMPTY),
        Content::Value(Reading::plain(Quantity::whole(0, Unit::Count))),
        Content::Media(CapRef::new(0)),
    ];

    /// How many relation ordinals cross. Unit: relation kinds.
    pub const RELATIONS_NAMED: u16 = Self::RELATIONS.len() as u16;

    /// How many content ordinals cross. Unit: content kinds.
    pub const CONTENTS_NAMED: u16 = Self::CONTENTS.len() as u16;

    /// The ordinal a relation kind crosses as.
    ///
    /// Exhaustive by construction. A fifth variant in
    /// `f_interface::node::Relation` fails this match, which is the whole of why
    /// the map is written in this direction: the other direction — ordinal to
    /// variant — is a function the compiler cannot check for completeness, and
    /// `abi`'s `admitted!` comment records what that cost the last time somebody
    /// wrote one.
    /// Unit: none — an ordinal in this map's own order.
    #[must_use]
    pub const fn relation_ordinal(relation: Relation) -> u16 {
        match relation {
            Relation::LabelledBy(_) => 0,
            Relation::Controls(_) => 1,
            Relation::Owns(_) => 2,
            Relation::FlowsTo(_) => 3,
        }
    }

    /// The ordinal a content kind crosses as, on
    /// [`Self::relation_ordinal`]'s terms exactly.
    /// Unit: none — an ordinal in this map's own order.
    #[must_use]
    pub const fn content_ordinal(content: &Content) -> u16 {
        match content {
            Content::None => 0,
            Content::Text(_) => 1,
            Content::Value(_) => 2,
            Content::Media(_) => 3,
        }
    }

    /// Is `ordinal` one this build names, with no version filter applied?
    ///
    /// Split out from [`Vocabulary::names`] so that the version half below is
    /// visibly a separate question. A vocabulary whose two halves were one
    /// expression is a vocabulary in which dropping the version costs nobody a
    /// diff, and RFC 0083's finding was exactly that the version half had gone
    /// missing without one.
    fn named(field: Closed, ordinal: u16) -> bool {
        match field {
            Closed::Role => Role::from_index(ordinal as usize).is_some(),
            Closed::Relation => ordinal < Self::RELATIONS_NAMED,
            Closed::Content => ordinal < Self::CONTENTS_NAMED,
        }
    }

    /// The vocabulary version `ordinal` was introduced in, if this build names
    /// it.
    ///
    /// `Role` answers from the second column of its own declaring line.
    /// `Relation` and `Content` answer [`VOCABULARY_VERSION_MIN`] because every
    /// kind of either has been there since version one, and the filter is
    /// written out anyway rather than skipped: the day a fifth relation is
    /// appended, this is where its version goes, and a filter that was absent is
    /// a filter somebody has to remember to add. RFC 0083's third finding was a
    /// missing filter, not a wrong one.
    /// Unit: none — a vocabulary version ordinal.
    fn since(field: Closed, ordinal: u16) -> Option<u16> {
        match field {
            Closed::Role => Role::from_index(ordinal as usize).map(Role::since),
            Closed::Relation | Closed::Content => {
                Self::named(field, ordinal).then_some(VOCABULARY_VERSION_MIN)
            }
        }
    }
}

impl Vocabulary for Interface {
    /// RFC 0083 part two's rule, both halves, in the order it states them.
    ///
    /// A role this build has and the peer's agreed version predates is **not**
    /// named, and that is the point rather than an edge case: two peers that
    /// agreed version one must produce the same tree from the same entries, so a
    /// newer receiver admitting a role only its own build knows would make the
    /// agreement a statement about the sender alone.
    fn names(&self, field: Closed, ordinal: u16, agreed: Agreed) -> bool {
        match Self::since(field, ordinal) {
            Some(since) => since <= agreed.version(),
            None => false,
        }
    }

    /// Every state bit RFC 0077 names, which is
    /// `f_interface::node::StateSet::KNOWN`.
    ///
    /// Read from that constant rather than written as a mask, for the reason the
    /// module comment gives about roles: a sixth state added there is admitted
    /// here without anybody editing this file, and a state removed stops being
    /// admitted. The agreed version is taken and not read, for the reason
    /// [`Interface::since`] gives — the five states are version one's, and this
    /// is where a sixth one's version would go.
    fn state_bits(&self, _agreed: Agreed) -> u8 {
        StateSet::KNOWN.bits()
    }
}

#[cfg(test)]
mod tests {
    use f_abi::semantic::{Handshake, VOCABULARY_VERSION};

    use super::*;

    /// The agreement an identical peer reaches, which is what every test below
    /// admits against unless it is about the version.
    fn agreed() -> Agreed {
        Handshake::HERE.negotiate().expect("this build agrees with itself")
    }

    #[test]
    fn every_role_the_vocabulary_declares_is_named_at_its_own_version() {
        let agreed = agreed();
        for index in 0..Role::COUNT {
            let role = Role::from_index(index).expect("the index came from the count");
            assert!(
                Interface.names(Closed::Role, index as u16, agreed),
                "{} is declared and not admitted",
                role.name()
            );
        }
        // And nothing past the end. The one ordinal a receiver is most likely to
        // be handed by a newer peer.
        assert!(!Interface.names(Closed::Role, Role::COUNT as u16, agreed));
        assert!(!Interface.names(Closed::Role, u16::MAX, agreed));
    }

    #[test]
    fn a_role_newer_than_the_agreed_version_is_not_named() {
        // The half RFC 0083's third finding says had no implementation. It is
        // written against a build that has no role newer than version one, so it
        // asserts the *arithmetic* rather than a role: admission is `since` at
        // or below the agreed version, at whatever version that is, and a
        // `names` that had dropped the filter would answer `true` for a role
        // whose `since` was above it.
        let agreed = agreed();
        for index in 0..Role::COUNT {
            let role = Role::from_index(index).expect("the index came from the count");
            assert_eq!(
                Interface.names(Closed::Role, index as u16, agreed),
                role.since() <= agreed.version(),
                "{} is admitted on a rule that is not RFC 0083's",
                role.name()
            );
        }
        assert_eq!(agreed.version(), VOCABULARY_VERSION);
    }

    #[test]
    fn the_two_maps_that_are_not_derived_agree_with_their_counts() {
        let agreed = agreed();
        for (position, relation) in Interface::RELATIONS.into_iter().enumerate() {
            let ordinal = Interface::relation_ordinal(relation);
            assert_eq!(
                ordinal, position as u16,
                "a relation's ordinal is not its position in the list that counts them",
            );
            assert!(Interface.names(Closed::Relation, ordinal, agreed));
        }
        for (position, content) in Interface::CONTENTS.iter().enumerate() {
            let ordinal = Interface::content_ordinal(content);
            assert_eq!(
                ordinal, position as u16,
                "a content kind's ordinal is not its position in the list that counts them",
            );
            assert!(Interface.names(Closed::Content, ordinal, agreed));
        }
        assert!(!Interface.names(Closed::Relation, Interface::RELATIONS_NAMED, agreed));
        assert!(!Interface.names(Closed::Content, Interface::CONTENTS_NAMED, agreed));
    }

    #[test]
    fn the_admitted_state_bits_are_the_vocabularys_own() {
        assert_eq!(Interface.state_bits(agreed()), StateSet::KNOWN.bits());
        // A bit outside the five is what a peer with a sixth state sends, and
        // the admission is `abi`'s. Asserted here because the mask is this
        // file's answer, and a mask of `0xFF` would pass every assertion above.
        assert_eq!(StateSet::KNOWN.bits() & 0b1110_0000, 0);
    }
}
