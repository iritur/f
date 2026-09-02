// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Capabilities: what a component may name, and what it may do with it.
//!
//! This is the wire half. The table itself is the frame's — `kernel/src/cap.rs`
//! — and nothing here knows how it is stored. What is here is the three things
//! that cross the boundary: the [`Handle`] a component puts in a register or in
//! [`Sqe::cap`](crate::Sqe::cap), the [`CapType`] the frame reports back, and
//! the [`rights`] bitmap that says which operations are permitted to it.
//!
//! # A handle is an index, and that is the whole security argument
//!
//! There is no global capability space, so there is no name to guess. A handle
//! is meaningless except as an offset into the table of the component that
//! presents it, and the frame is the only writer of that table. That is what
//! "a process cannot name a capability it was not given" means here, and it is
//! structural rather than checked: the check — bounds, occupancy, generation —
//! only decides *which* refusal a bad handle earns.
//!
//! Contrast the descriptor it replaces. A file descriptor is a small integer in
//! a per-process table too, and the reason it fails is not the numbering: it is
//! that the table is populated ambiently, by inheritance across `fork` and by
//! opening a path, so a process holds authority nobody deliberately gave it.
//! `docs/what-must-be-stated.html` files that under *descriptors as small
//! integers*, and the answer is grant-only population, not a wider integer.
//!
//! # What the generation is, and what it is not
//!
//! The generation makes a **stale** handle detectable. A slot that is cleared
//! and refilled would otherwise transfer authority silently — the same integer,
//! a different object, and no event anywhere — which is the failure mode every
//! descriptor table has and `close`-then-`open` reproduces on purpose.
//!
//! It is not a secret and it is not a defence against guessing. Sixteen bits
//! guessed by a hostile component is not a hard problem; the reason guessing
//! buys nothing is that the index has to name a slot the frame filled *for that
//! component*, and no generation makes an unfilled slot resolve. Stating this
//! the other way round would be the mistake: a capability system whose safety
//! rests on an unguessable token has quietly become a password system.
//!
//! Generations count from one, so a zeroed word — [`Sqe::ZERO`](crate::Sqe::ZERO)
//! included — names nothing. See [`Handle::NULL`].
//!
//! See `docs/design/ring-scene-boot.html` section 15, milestone M4, and
//! `docs/rfc/0015-capabilities-at-the-door.md`.

/// What kind of object a capability names.
///
/// Six from the milestone and a seventh from E1-D03. The discriminants are wire
/// values: a component is told them by [`Handle`] lookups, and a later ABI
/// version may add to the list but may not renumber it.
///
/// Three of the six have no object behind them at M4 and the variant says which
/// milestone gives them one. That is deliberate: a type added when its object
/// arrives is a type the table's shape was not designed for, and the shape is
/// the part that is expensive to change once two peers exist. The seventh is
/// added on the same argument, ahead of the task that gives it an object.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapType {
    /// Memory that has no type yet. Deriving from it retypes a sub-range into
    /// something that does — a [`CapType::Frame`], today — and the derived
    /// capability is a child, so revoking the untyped region reaches every
    /// object minted out of it.
    Untyped = 1,
    /// One page of physical memory. Its rights are the permissions a mapping of
    /// it may carry, which is what makes [`rights::WRITE`] on a frame the
    /// difference between a writable mapping and a refusal.
    Frame = 2,
    /// An address space pages may be mapped into. A process holds exactly one
    /// at M4 and it is its own; mapping into somebody else's is E1, and is what
    /// turns this from an operand nobody varies into one that matters.
    AddressSpace = 3,
    /// One end of a ring. No object until M5 — `E0-B12` creates the first one —
    /// and this is what [`Sqe::cap`](crate::Sqe::cap) is validated against.
    Channel = 4,
    /// The right to send to a component that is not listening on a ring yet.
    /// No object until the component lifecycle exists: `E1-D01`, RFC 0008.
    Endpoint = 5,
    /// One interrupt vector, delivered to a component rather than to the frame.
    /// No object until a driver lives outside the kernel, which is E1.
    Irq = 6,
    /// A registered buffer set: memory a component handed to a service so a
    /// device may reach it. A child of the memory capability it was registered
    /// from, so revoking the parent — or the component dying, RFC 0008 — tears
    /// the registration down with it. No object until `E1-B10`; the decision
    /// is `docs/rfc/0024-a-buffer-is-owned-by-one-side.md`.
    BufferSet = 7,
}

impl CapType {
    /// The type a wire value names, or `None` if this build does not know it.
    ///
    /// Unknown is refused rather than ignored: `docs/what-must-be-stated.html`
    /// R04, and the same rule
    /// [`ChannelHeader::is_valid`](crate::ChannelHeader::is_valid) applies to a
    /// reserved field.
    #[must_use]
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Untyped),
            2 => Some(Self::Frame),
            3 => Some(Self::AddressSpace),
            4 => Some(Self::Channel),
            5 => Some(Self::Endpoint),
            6 => Some(Self::Irq),
            7 => Some(Self::BufferSet),
            _ => None,
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_wire(self) -> u8 {
        self as u8
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Untyped => "untyped",
            Self::Frame => "frame",
            Self::AddressSpace => "space",
            Self::Channel => "channel",
            Self::Endpoint => "endpoint",
            Self::Irq => "irq",
            Self::BufferSet => "bufset",
        }
    }
}

/// What a holder may do with the object a capability names.
///
/// A bitmap, and the only operation on it that is ever legal is *narrowing*.
/// There is no call that adds a right to a capability and none that asks the
/// frame to grant one: a component's authority is a monotonically shrinking
/// function of what it was handed, which is the property the derivation tree
/// exists to keep true.
///
/// Six bits in one `u8`, with two spare. Adding a seventh is an ABI change and
/// should be argued as one — a rights bitmap that grows without argument is how
/// a permission model becomes a list of special cases.
pub mod rights {
    /// May be read, and may be mapped readable.
    pub const READ: u8 = 1 << 0;
    /// May be written, and may be mapped writable.
    pub const WRITE: u8 = 1 << 1;
    /// May be mapped executable. Separate from [`READ`] because
    /// write-exclusive-or-execute is a rule about mappings, and a rights bitmap
    /// that cannot express it cannot enforce it.
    pub const EXECUTE: u8 = 1 << 2;
    /// May have weaker capabilities derived from it. Withholding this is how a
    /// component is handed authority it cannot pass on in any form.
    pub const DERIVE: u8 = 1 << 3;
    /// May have its descendants revoked. Held by whoever is entitled to take
    /// the authority back, which is not always whoever holds the object.
    pub const REVOKE: u8 = 1 << 4;
    /// May be transferred to another component. There is no second component to
    /// transfer to until E1-D01; the bit exists now because the alternative is
    /// discovering at E1 that every capability ever minted was transferable by
    /// default.
    pub const GRANT: u8 = 1 << 5;

    /// Every right this build defines.
    pub const ALL: u8 = READ | WRITE | EXECUTE | DERIVE | REVOKE | GRANT;

    /// No rights at all. A legal capability: it names an object and authorises
    /// nothing, which is what the weakest derivation produces.
    pub const NONE: u8 = 0;

    /// Are *all* of `asked` present in `held`?
    #[must_use]
    pub const fn holds(held: u8, asked: u8) -> bool {
        held & asked == asked
    }

    /// Is `child` a narrowing of `parent` — that is, does it add nothing?
    ///
    /// Equality narrows: a copy is the identity case, and it is a derivation
    /// like any other so that revoking the parent reaches it. `kernel/src/cap.rs`
    /// says why a copy is a child here and a sibling in seL4.
    #[must_use]
    pub const fn narrows(parent: u8, child: u8) -> bool {
        child & !parent == 0
    }

    /// A bit this build does not define.
    #[must_use]
    pub const fn unknown(bits: u8) -> bool {
        bits & !ALL != 0
    }
}

/// A component's name for one of its capabilities.
///
/// Sixteen bits of slot index and sixteen of generation, packed into the `u32`
/// that [`Sqe::cap`](crate::Sqe::cap) already is. The packing is here rather
/// than in the frame because both sides have to agree on it: a component
/// computes no handles it was not given, but it does have to store, compare and
/// pass the ones it holds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Handle(u32);

impl Handle {
    /// The handle that names nothing.
    ///
    /// Zero, and never valid, for the reason the module comment gives:
    /// generations count from one, so no slot is ever issued at generation zero
    /// and a zeroed field cannot become an authority by accident.
    /// [`Sqe::ZERO`](crate::Sqe::ZERO) depends on this.
    pub const NULL: Self = Self(0);

    /// The generation a slot is issued at first.
    pub const FIRST_GENERATION: u16 = 1;

    /// The generation at which a slot may not be reused.
    ///
    /// A generation counter that wraps is a stale handle that becomes valid
    /// again, so it does not wrap: a slot that has held this many capabilities
    /// is retired instead. That converts a soundness hole into running out of
    /// slots, which is an error a component can be told about.
    pub const RETIRED_GENERATION: u16 = u16::MAX;

    /// Pack a slot index and a generation.
    #[must_use]
    pub const fn new(index: u16, generation: u16) -> Self {
        Self(((generation as u32) << 16) | index as u32)
    }

    /// From the wire.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// To the wire.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Which slot.
    #[must_use]
    pub const fn index(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    /// Which occupant of that slot.
    #[must_use]
    pub const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// Could this handle have been issued by anybody?
    ///
    /// A structural test, not an authority check: it rules out the zero word
    /// and nothing else. Only a table can say whether a handle names one of
    /// *its* capabilities.
    #[must_use]
    pub const fn is_issuable(self) -> bool {
        self.generation() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sqe, error};

    #[test]
    fn a_handle_survives_the_round_trip() {
        for index in [0u16, 1, 31, 4095, u16::MAX] {
            for generation in [1u16, 2, 1000, u16::MAX] {
                let handle = Handle::new(index, generation);
                assert_eq!(handle.index(), index);
                assert_eq!(handle.generation(), generation);
                assert_eq!(Handle::from_bits(handle.bits()), handle);
            }
        }
    }

    #[test]
    fn a_zeroed_entry_names_nothing() {
        // The property the packing is arranged around: a submission that was
        // memset to zero must not carry authority over slot zero.
        assert_eq!(Handle::from_bits(Sqe::ZERO.cap), Handle::NULL);
        assert!(!Handle::NULL.is_issuable());
        assert!(Handle::new(0, Handle::FIRST_GENERATION).is_issuable());
    }

    #[test]
    fn rights_narrow_and_never_widen() {
        let parent = rights::READ | rights::DERIVE;
        assert!(rights::narrows(parent, parent), "a copy is the identity narrowing");
        assert!(rights::narrows(parent, rights::READ));
        assert!(rights::narrows(parent, rights::NONE));
        assert!(!rights::narrows(parent, parent | rights::WRITE));
        assert!(!rights::narrows(rights::NONE, rights::READ));
    }

    #[test]
    fn holding_is_not_overlapping() {
        // The mistake this function exists to prevent: `held & asked != 0` is
        // true when *any* asked right is present, which grants write access to
        // anything readable.
        let held = rights::READ | rights::EXECUTE;
        assert!(rights::holds(held, rights::READ));
        assert!(rights::holds(held, rights::READ | rights::EXECUTE));
        assert!(!rights::holds(held, rights::READ | rights::WRITE));
        assert!(rights::holds(held, rights::NONE));
    }

    #[test]
    fn a_right_this_build_does_not_define_is_refused() {
        assert!(rights::unknown(1 << 6));
        assert!(rights::unknown(1 << 7));
        assert!(!rights::unknown(rights::ALL));
        assert!(!rights::unknown(rights::NONE));
    }

    #[test]
    fn every_type_survives_the_wire_and_has_a_word() {
        let all = [
            CapType::Untyped,
            CapType::Frame,
            CapType::AddressSpace,
            CapType::Channel,
            CapType::Endpoint,
            CapType::Irq,
            CapType::BufferSet,
        ];
        for kind in all {
            assert_eq!(CapType::from_wire(kind.to_wire()), Some(kind));
            assert!(!kind.label().is_empty(), "{kind:?} has no word");
        }
        // Zero is not a type, which is what lets a zeroed slot mean empty.
        assert_eq!(CapType::from_wire(0), None);
        assert_eq!(CapType::from_wire(8), None);
        assert_eq!(CapType::from_wire(u8::MAX), None);
    }

    #[test]
    fn the_authority_domain_names_every_way_a_handle_fails() {
        // Four codes, four distinct refusals, listed together so that adding a
        // fifth without a test is a diff that looks wrong.
        let codes = [
            error::authority::NO_SUCH_CAP,
            error::authority::RIGHT_NOT_HELD,
            error::authority::REVOKED,
            error::authority::WRONG_TYPE,
        ];
        for (i, code) in codes.iter().enumerate() {
            for other in &codes[i + 1..] {
                assert_ne!(code, other, "two authority codes share a value");
            }
            let packed = error::pack(error::AUTHORITY, *code);
            assert_eq!(error::unpack(packed), Some((error::AUTHORITY, *code)));
        }
    }
}
