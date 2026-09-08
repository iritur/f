// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a component declares about being updated in place, and what it declares
//! when it cannot.
//!
//! RFC 0012 made an update a generation swap rather than a boot, and RFC 0041
//! made a place outlive its occupant. Between those two there is a hole: a place
//! can be refilled from a *newer manifest*, and nothing said whether the
//! incoming occupant is allowed to inherit anything from the outgoing one. This
//! module is that declaration, and RFC 0063 is the reasoning.
//!
//! # Why this is a declaration and not a protocol
//!
//! Because the expensive decision is the one that can be taken from two
//! manifests, before anything drains. A place whose incoming and outgoing
//! [`Declaration`]s disagree — a different [`Declaration::schema`], a different
//! [`Declaration::record_bytes`], either side saying [`mode::RESTART_ONLY`] —
//! is a place that will restart, and the assembler knows that from the
//! generation tree without stopping a single client. A swap that discovered its
//! own impossibility halfway would have already held a client's submissions to
//! find out.
//!
//! # What is opaque here, and deliberately
//!
//! The frame never reads a state record. It knows how wide one is
//! ([`Declaration::record_bytes`]) and how many there can be
//! ([`Declaration::records_max`]), because those two are what size the window
//! somebody has to pay for; it does not know what a record *means*, because the
//! only reader of a state record is another build of the same component.
//! [`Declaration::schema`] is what makes the two builds agree, and it is the
//! component's ordinal rather than this crate's: two components that both
//! declare `schema = 1` are not saying anything about each other.
//!
//! # Why quiescence is not a field here
//!
//! RFC 0018's cursors say the rings are empty. They cannot say the *occupant*
//! is empty: RFC 0041 names the set a driver holds — work accepted and not yet
//! answered, keyed by token — and a request inside a device is behind both
//! cursors and behind the driver. So quiescence is asserted by the occupant over
//! a ring-empty precondition the frame checks, always, and there is no mode in
//! which it is not. A third value meaning "only from a named point" would be
//! describing what [`mode::IN_PLACE`] already means, and the second value it
//! implies — a cursors-only transfer — is one no component in this tree can
//! correctly use.

/// Whether a component can be updated in place.
///
/// Zero is not a mode, so a zeroed record declares none rather than declaring
/// the first — the same rule [`crate::manifest::domain`] and
/// [`crate::manifest::restart`] follow, and for the same reason: a default is
/// how a component acquires a property nobody chose.
pub mod mode {
    /// The occupant hands nothing over. A generation swap at this place is a
    /// teardown and a spawn from the incoming manifest, and it is counted as a
    /// restart rather than as a swap — RFC 0012's `places_restarted`, which is
    /// deliberately not summed with `places_swapped`.
    ///
    /// This is the honest declaration. A component that has not built a state
    /// record, or that holds nothing worth carrying, says this and is believed.
    pub const RESTART_ONLY: u8 = 1;

    /// The occupant drains to a quiescent point it asserts, writes its state
    /// records into a window the incoming occupant paid for, and the routing
    /// word swaps. RFC 0063.
    pub const IN_PLACE: u8 = 2;

    /// Whether a wire value is a mode this build knows. Fail closed, R04.
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, RESTART_ONLY | IN_PLACE)
    }

    /// The mode as `docs/manifest.md` spells it, or `"?"`.
    #[must_use]
    pub const fn label(value: u8) -> &'static str {
        match value {
            RESTART_ONLY => "restart_only",
            IN_PLACE => "in_place",
            _ => "?",
        }
    }
}

/// The alignment one state record is required to have, in bytes.
///
/// Eight, because the window is `records_max` records laid end to end and read
/// in place: a width that is not a multiple of this puts record *n* on an odd
/// boundary for every odd *n*, and an unaligned load is what
/// [`crate::manifest::Refusal::Unaligned`] already refuses one record up. It is
/// the alignment of the widest scalar the wire types use and not a number
/// somebody chose, which is why there is no claim behind it.
/// Unit: bytes.
pub const RECORD_ALIGN: u32 = 8;

/// What a component declares about being updated in place.
///
/// Sixteen bytes inside [`crate::manifest::Record`], judged by
/// [`crate::manifest::Record::read`] like every other field and never
/// defaulted. The four fields are the whole of it: a swap needs to know whether
/// it may happen, whether the two builds understand each other, and how much
/// memory the window costs.
#[repr(C)]
// `PartialEq` is *identity* and not compatibility, and the distinction is the
// whole reason [`Declaration::transfers_to`] exists: that function compares
// three fields and deliberately not [`Declaration::records_max`], so two
// declarations that are `==` transfer to each other and two that transfer to
// each other need not be `==`. Reaching for `==` to decide a swap is the bug the
// named function exists to prevent, which is why this comment is here rather
// than the derive being left to look obvious.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Declaration {
    /// The state-record schema this build writes and reads.
    ///
    /// The component's ordinal and not this crate's: it is compared only
    /// against the same component's other build, and two components that
    /// declare the same number have said nothing to each other. An incoming
    /// occupant whose number differs cannot read what the outgoing one would
    /// write, so the place restarts — decided from two manifests before
    /// anything drains, which is the whole reason this is a declaration.
    /// Unit: none — a state-record schema ordinal, at least 1 under
    /// [`mode::IN_PLACE`]. Zero under [`mode::RESTART_ONLY`], and refused
    /// there.
    pub schema: u32,
    /// The fixed width of one state record.
    ///
    /// Fixed, so that the window is arithmetic and the frame never parses what
    /// it hands over. A component whose state does not fit a fixed width
    /// declares the widest record it will write and pads; a component that
    /// cannot do that declares [`mode::RESTART_ONLY`], which is what that
    /// value is for.
    /// Unit: bytes, a positive multiple of [`RECORD_ALIGN`] under
    /// [`mode::IN_PLACE`]. Zero under [`mode::RESTART_ONLY`], and refused
    /// there.
    pub record_bytes: u32,
    /// The most records this component will hand over.
    ///
    /// A bound and not a count: what actually crosses is however many the
    /// outgoing occupant writes, up to this. It is declared because the window
    /// has to be bought before the outgoing occupant is asked how much it will
    /// use, and a window sized after the answer is a window sized by the peer.
    /// Unit: count of records, at least 1 under [`mode::IN_PLACE`]. Zero under
    /// [`mode::RESTART_ONLY`], and refused there.
    pub records_max: u32,
    /// Whether this component can be updated in place.
    /// Unit: none — a [`mode`] constant. Zero is not a mode.
    pub mode: u8,
    /// Reserved. Must be zero — a non-zero value is refused rather than
    /// ignored, per R04.
    /// Unit: none; this is not a quantity and is not expected to become one
    /// without a schema bump.
    pub _reserved: [u8; 3],
}

// The layout is the ABI, for the reason `manifest.rs` states beside its own
// assertions: a field reordered, widened or inserted is a build failure with a
// number in it rather than two builds disagreeing about one component file.
// Sixteen bytes and no padding — three `u32`s, a `u8` and three reserved bytes.
const _: () = assert!(core::mem::size_of::<Declaration>() == 16);
const _: () = assert!(core::mem::align_of::<Declaration>() == 4);

impl Declaration {
    /// What a component declares when it has not decided anything.
    ///
    /// Not a default: [`crate::manifest::Record::read`] refuses `mode == 0`, so
    /// this constant produces a record that is refused rather than one that is
    /// accepted with a property nobody chose. `docs/manifest.md` requires the
    /// `[transfer]` table for the same reason it requires `[restart]`.
    pub const EMPTY: Self =
        Self { schema: 0, record_bytes: 0, records_max: 0, mode: 0, _reserved: [0; 3] };

    /// The honest declaration: this component hands nothing over.
    pub const RESTART_ONLY: Self = Self { mode: mode::RESTART_ONLY, ..Self::EMPTY };

    /// How large the transfer window is, in bytes.
    ///
    /// Two `u32`s multiplied into a `u64`, which cannot overflow, so this
    /// answers rather than refusing. Zero under [`mode::RESTART_ONLY`], which
    /// is the arithmetic saying the same thing the mode does.
    /// Unit: bytes.
    #[must_use]
    pub const fn window_bytes(&self) -> u64 {
        (self.record_bytes as u64) * (self.records_max as u64)
    }

    /// Whether this component may be updated in place at all.
    #[must_use]
    pub const fn in_place(&self) -> bool {
        self.mode == mode::IN_PLACE
    }

    /// Whether an occupant declaring `self` can hand its state to one declaring
    /// `incoming`.
    ///
    /// The static half of a swap, and the reason this type exists: three field
    /// comparisons over two manifests, taken before a client's submissions are
    /// held. Both sides must say [`mode::IN_PLACE`], agree on the state-record
    /// schema, and agree on the record width — a wider record in the incoming
    /// build is not a compatible superset, it is a different reading of the
    /// same bytes, and `schema` is the field that exists to be bumped when the
    /// shape changes.
    ///
    /// It does **not** compare [`Declaration::records_max`]: the outgoing side
    /// writes at most its own bound and the incoming side bought a window for
    /// its own, so a swap into an occupant that will hold fewer records is a
    /// swap that can run out of window. That is the one incompatibility this
    /// function cannot see, because it is arithmetic over a count the outgoing
    /// occupant has not reported yet; RFC 0063 says what the swap does when it
    /// happens, and it is an abandonment and not a loss.
    #[must_use]
    pub const fn transfers_to(&self, incoming: &Self) -> bool {
        self.in_place()
            && incoming.in_place()
            && self.schema == incoming.schema
            && self.record_bytes == incoming.record_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_not_a_mode() {
        assert!(!mode::known(0), "a zeroed record must declare no mode");
        assert!(mode::known(mode::RESTART_ONLY));
        assert!(mode::known(mode::IN_PLACE));
        assert!(!mode::known(3), "an unknown mode is refused and never guessed at");
    }

    #[test]
    fn a_restart_only_declaration_buys_no_window() {
        assert_eq!(Declaration::RESTART_ONLY.window_bytes(), 0);
        assert!(!Declaration::RESTART_ONLY.in_place());
    }

    #[test]
    fn the_window_is_the_product() {
        let declared =
            Declaration { schema: 1, record_bytes: 32, records_max: 16, ..Declaration::EMPTY };
        assert_eq!(declared.window_bytes(), 512);
    }

    #[test]
    fn the_widest_window_does_not_overflow() {
        let declared =
            Declaration { record_bytes: u32::MAX, records_max: u32::MAX, ..Declaration::EMPTY };
        assert_eq!(declared.window_bytes(), u64::from(u32::MAX) * u64::from(u32::MAX));
    }

    /// The three comparisons `transfers_to` makes, and the one it does not.
    #[test]
    fn compatibility_is_three_fields() {
        let base = Declaration {
            schema: 1,
            record_bytes: 32,
            records_max: 16,
            mode: mode::IN_PLACE,
            _reserved: [0; 3],
        };
        assert!(base.transfers_to(&base));
        assert!(
            !base.transfers_to(&Declaration { schema: 2, ..base }),
            "a state-record schema the incoming build does not know is a restart"
        );
        assert!(
            !base.transfers_to(&Declaration { record_bytes: 64, ..base }),
            "a different record width is a different reading of the same bytes"
        );
        assert!(
            !base.transfers_to(&Declaration::RESTART_ONLY),
            "an incoming occupant that hands nothing over accepts nothing either"
        );
        assert!(
            !Declaration::RESTART_ONLY.transfers_to(&base),
            "an outgoing occupant that declared restart_only is restarted"
        );
        assert!(
            base.transfers_to(&Declaration { records_max: 8, ..base }),
            "a smaller incoming bound is not visible here; it is an abandonment at run time"
        );
    }
}
