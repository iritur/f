// SPDX-License-Identifier: Apache-2.0 OR MIT
//! How a submission names a buffer, on both paths.
//!
//! This is the wire half of buffer ownership. The ownership itself — which side
//! may touch the bytes at any moment — is the ring's, in `f_ring::buffers`,
//! and nothing here knows about it. What is here is the twelve bytes at offset
//! 32 of an [`Sqe`](crate::Sqe) and the two readings they have:
//!
//! - **Registered.** [`flags::FIXED_BUF`](crate::flags::FIXED_BUF) is set, and
//!   `buf_set`/`buf_index` name one buffer of a set the submitter registered
//!   earlier on this channel. No address crosses the boundary. The service
//!   resolves the pair against the registration it holds, which is memory it
//!   already knows the extent of.
//! - **Virtual.** `FIXED_BUF` is clear, and the same eight bytes are a virtual
//!   address in the submitter's own address space, low half first. Legal only
//!   on a channel that negotiated
//!   [`feature::SHARED_VIRTUAL_MEMORY`](crate::feature::SHARED_VIRTUAL_MEMORY):
//!   the device walks the submitter's page tables through the IOMMU, so there
//!   is nothing to register and the triple collapses to the address, which is
//!   what `ring-scene-boot` section 05 says it does. The address-space
//!   identifier is the channel's, never the entry's.
//!
//! # A set identifier is an index, and the argument is the capability one
//!
//! A [`SetId`] is sixteen bits of slot and sixteen of generation, packed the
//! way [`cap::Handle`](crate::cap::Handle) is and for the same reasons. There
//! is no global registration space: the id means nothing except as a slot in
//! the registrations *this channel* holds, so a forged one can only ever name a
//! slot the service filled for this peer or fail the check. The generation
//! makes a stale id — a set de-registered and its slot reused — detectable
//! rather than silently transferred. Generations count from one, so a zeroed
//! entry names no set; and on the other path an address of zero is refused
//! too, so [`Sqe::ZERO`](crate::Sqe::ZERO) names no buffer whatever its flags
//! say.
//!
//! # What reading a name decides, and what it does not
//!
//! [`Name::read`] is structural: it says which path the entry took, refuses a
//! path the channel did not negotiate, and refuses a name that nobody could
//! have issued. Whether the set exists, whether the index is inside it, and
//! whether that buffer is already lent to the device are questions only the
//! service's own registration can answer — and each earns a refusal named in
//! `docs/rfc/0024-a-buffer-is-owned-by-one-side.md`.

use crate::{Sqe, error, feature, flags};

/// A channel's name for one registered buffer set.
///
/// Issued by the service in the completion of a registration, carried back in
/// [`Sqe::buf_set`](crate::Sqe::buf_set) on every entry that uses the set. The
/// packing is here rather than in either peer because both have to agree on it.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SetId(u32);

impl SetId {
    /// The id that names nothing. Zero, never issued: generations start at one.
    pub const NULL: Self = Self(0);

    /// The generation a slot is issued at first.
    pub const FIRST_GENERATION: u16 = 1;

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

    /// Which registration slot.
    #[must_use]
    pub const fn index(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    /// Which occupant of that slot.
    #[must_use]
    pub const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// Could this id have been issued by anybody?
    ///
    /// Structural, like [`cap::Handle::is_issuable`](crate::cap::Handle::is_issuable):
    /// it rules out the zero generation and nothing else. Only the service's
    /// registration table can say whether an id names one of *its* sets.
    #[must_use]
    pub const fn is_issuable(self) -> bool {
        self.generation() != 0
    }
}

/// One reading of an entry's buffer fields.
///
/// Not itself a wire type. It is what the twelve bytes *mean* on this channel,
/// and the same bytes mean different things on channels that negotiated
/// different features — which is why it is produced by [`Name::read`] with the
/// negotiated set in hand and never by looking at the entry alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Name {
    /// One buffer of a registered set.
    Registered {
        /// The set, as this channel names it.
        set: SetId,
        /// Which buffer of the set. Unit: buffers, zero-based.
        index: u32,
    },
    /// A virtual address in the submitter's own address space.
    Virtual {
        /// The address. Unit: bytes, in the submitter's address space. Zero is
        /// refused, so this is never zero.
        address: u64,
    },
}

impl Name {
    /// Write this name into an entry, setting or clearing
    /// [`flags::FIXED_BUF`] to say which path it took.
    ///
    /// Everything else in the entry is left alone: the opcode, the class, the
    /// deadline, `user_data`, `offset`, `len` and `ext` are the caller's.
    pub const fn write(self, entry: &mut Sqe) {
        match self {
            Self::Registered { set, index } => {
                entry.flags |= flags::FIXED_BUF;
                entry.buf_set = set.bits();
                entry.buf_index = index;
            }
            Self::Virtual { address } => {
                entry.flags &= !flags::FIXED_BUF;
                // Low half first, high half second. Stated rather than left to
                // the byte order of whichever machine writes it, because the
                // two peers may not share one.
                entry.buf_set = address as u32;
                entry.buf_index = (address >> 32) as u32;
            }
        }
    }

    /// Read the name an entry carries, on a channel whose negotiated feature
    /// set is `features`.
    ///
    /// # Errors
    ///
    /// A packed [`error`] result, so the refusal can go straight into a
    /// completion. `ARGUMENT`/[`FEATURE_NOT_NEGOTIATED`](error::argument::FEATURE_NOT_NEGOTIATED)
    /// for an address on a channel without shared virtual memory — the entry
    /// used a feature outside the agreed set, which
    /// [`Negotiated`](crate::Negotiated) says a peer must not do.
    /// `AUTHORITY`/[`NO_SUCH_CAP`](error::authority::NO_SUCH_CAP) for a set id
    /// nobody could have issued. `ARGUMENT`/[`BAD_ADDRESS`](error::argument::BAD_ADDRESS)
    /// for a null address. Each refusal's detail is the offending field.
    ///
    /// R04 throughout: an entry that does not fit either reading is refused,
    /// never read as the nearest one.
    pub const fn read(entry: &Sqe, features: u64) -> Result<Self, (i32, u64)> {
        if entry.flags & flags::FIXED_BUF != 0 {
            let set = SetId::from_bits(entry.buf_set);
            if !set.is_issuable() {
                return Err((
                    error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                    entry.buf_set as u64,
                ));
            }
            return Ok(Self::Registered { set, index: entry.buf_index });
        }

        if features & feature::SHARED_VIRTUAL_MEMORY == 0 {
            return Err((
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
                feature::SHARED_VIRTUAL_MEMORY,
            ));
        }
        let address = (entry.buf_set as u64) | ((entry.buf_index as u64) << 32);
        if address == 0 {
            return Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), 0));
        }
        Ok(Self::Virtual { address })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_id_survives_the_round_trip() {
        for index in [0u16, 1, 255, u16::MAX] {
            for generation in [1u16, 2, u16::MAX] {
                let id = SetId::new(index, generation);
                assert_eq!(id.index(), index);
                assert_eq!(id.generation(), generation);
                assert_eq!(SetId::from_bits(id.bits()), id);
                assert!(id.is_issuable());
            }
        }
        assert!(!SetId::NULL.is_issuable());
        assert_eq!(SetId::from_bits(Sqe::ZERO.buf_set), SetId::NULL);
    }

    #[test]
    fn a_registered_name_round_trips_and_sets_the_flag() {
        let name = Name::Registered { set: SetId::new(3, 2), index: 7 };
        let mut entry = Sqe::ZERO;
        name.write(&mut entry);
        assert_ne!(entry.flags & flags::FIXED_BUF, 0);
        assert_eq!(Name::read(&entry, 0), Ok(name));
        // And on a channel that also has shared virtual memory, the flag still
        // decides: the path is per entry, the permission is per channel.
        assert_eq!(Name::read(&entry, feature::SHARED_VIRTUAL_MEMORY), Ok(name));
    }

    #[test]
    fn a_virtual_name_round_trips_only_where_it_was_negotiated() {
        let name = Name::Virtual { address: 0x0000_7F12_3456_7000 };
        let mut entry = Sqe::ZERO;
        entry.flags = flags::FIXED_BUF | flags::LINK;
        name.write(&mut entry);
        assert_eq!(
            entry.flags,
            flags::LINK,
            "writing an address clears FIXED_BUF and nothing else"
        );
        assert_eq!(entry.buf_set, 0x3456_7000, "low half first");
        assert_eq!(entry.buf_index, 0x7F12, "high half second");

        assert_eq!(Name::read(&entry, feature::SHARED_VIRTUAL_MEMORY), Ok(name));
        assert_eq!(
            Name::read(&entry, 0),
            Err((
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
                feature::SHARED_VIRTUAL_MEMORY
            ))
        );
    }

    #[test]
    fn a_zeroed_entry_names_no_buffer_on_either_path() {
        // The property the two zero rules exist for: a submission that was
        // memset to zero carries no buffer, whichever way its flag falls.
        let mut entry = Sqe::ZERO;
        entry.flags = flags::FIXED_BUF;
        assert_eq!(
            Name::read(&entry, feature::SHARED_VIRTUAL_MEMORY),
            Err((error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP), 0))
        );

        entry.flags = 0;
        assert_eq!(
            Name::read(&entry, feature::SHARED_VIRTUAL_MEMORY),
            Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), 0))
        );
    }

    #[test]
    fn a_refusal_names_the_offending_field() {
        let mut entry = Sqe::ZERO;
        entry.flags = flags::FIXED_BUF;
        entry.buf_set = SetId::new(9, 0).bits();
        let (_, detail) = Name::read(&entry, 0).unwrap_err();
        assert_eq!(detail, 9, "the detail is the set id as written, not a translation of it");
    }
}
