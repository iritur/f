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
//!
//! # Where a set identifier comes from
//!
//! From a registration, and from nowhere else. [`Request`] is the entry that
//! asks for one and [`opcode`] the two numbers it is asked on;
//! [`SetId::from_completion`] is how the answer is read. The table that issues
//! them lives with the service, in the service's own memory — `f_ring::registry`
//! — and nothing about a registration is kept in the shared region, for the
//! reason `E0-B15` gives about the doorbell counts: evidence a peer can write
//! is not evidence, and it would have cost an ABI version for a field that
//! never needed to cross the boundary. RFC 0028.

use crate::{Cqe, Sqe, error, feature, flags};

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

    /// The generation at which a slot may not be reused.
    ///
    /// The same value and the same sentence as
    /// [`cap::Handle::RETIRED_GENERATION`](crate::cap::Handle::RETIRED_GENERATION),
    /// because a [`SetId`] is packed the way a handle is and a counter that
    /// wraps means the same thing in both: a stale name that becomes valid
    /// again, resolving into whatever took the slot, with no event anywhere. So
    /// it does not wrap — a slot that has held this many sets is retired
    /// instead, which converts a soundness hole into running out of slots, and
    /// running out of slots is a thing a peer can be told
    /// (`RESOURCE`/[`QUOTA_EXHAUSTED`](error::resource::QUOTA_EXHAUSTED)).
    ///
    /// Never issued: a set is named at generations one through
    /// `RETIRED_GENERATION - 1`, so sixty-five thousand five hundred and
    /// thirty-four registrations is what one slot is worth.
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

    /// The id a registration's completion issued.
    ///
    /// The one place a client is meant to obtain a [`SetId`] from. Before
    /// `E1-B10` there was no such place at all, which is why the module above
    /// says an invented id is a thing the service refuses rather than a thing
    /// the client cannot hold. There is a place now, and
    /// `f_ring::buffers::Fixed` has no constructor but this one — which makes
    /// the invented id an act rather than an expression, and does not make it
    /// impossible. Nothing in a wire crate could: a [`SetId`] is four bytes and
    /// this function reads four bytes out of a completion, so what it can check
    /// is that the completion is well formed and not that the sender had a
    /// table. The check that the id names one of *this service's* sets is the
    /// service's, in `f_ring::registry`, and that has not moved.
    ///
    /// # Errors
    ///
    /// The completion's own refusal when it is one, unchanged — a registration
    /// that was refused issued no id, and a caller needs the domain and the
    /// code rather than an absence it has to guess at.
    /// `AUTHORITY`/[`NO_SUCH_CAP`](error::authority::NO_SUCH_CAP) for a
    /// completion that succeeded and carries an id nobody could have issued,
    /// which is a service answering wrongly rather than a peer lying.
    /// `ARGUMENT`/[`RESERVED_NOT_ZERO`](error::argument::RESERVED_NOT_ZERO)
    /// for bits above the id: `ext` is eight bytes and an id is four, and the
    /// upper four are not yet anybody's to use. R04 — refused, not masked off.
    pub const fn from_completion(cqe: &Cqe) -> Result<Self, (u8, u16)> {
        if let Some(refused) = cqe.error() {
            return Err(refused);
        }
        if cqe.ext > u32::MAX as u64 {
            return Err((error::ARGUMENT, error::argument::RESERVED_NOT_ZERO));
        }
        let set = Self::from_bits(cqe.ext as u32);
        if !set.is_issuable() {
            return Err((error::AUTHORITY, error::authority::NO_SUCH_CAP));
        }
        Ok(set)
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
    ///
    /// What this does *not* check is the envelope — the reserved word and the
    /// undefined flag bits — because it is called by a service that has already
    /// run its own executor over the entry, in the order `f_ring::execute`
    /// states. [`Request::read`] checks it because it is called *instead of*
    /// that executor and not after it.
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

/// The two opcodes registration answers on.
///
/// # Why a cross-cutting pair, in the crate that holds no service's vocabulary
///
/// `ring-scene-boot` section 05 says the opcode space is per-service and not
/// global: a storage ring and a compositor ring share the envelope, not the
/// words. Registration is the one operation that cuts across every one of
/// those vocabularies — every service that takes a buffer needs it, none of
/// them invented it, and a client that has learned to register with one has
/// learned to register with all of them. A number agreed per service would be a
/// client that has to be told, per service, which entry to write; being told is
/// a registrar, and a registrar is the global namespace this system does not
/// have.
///
/// The **top** of the byte, because a service numbers its own opcodes upward
/// from zero — [`op::NOP`](crate::op::NOP) is zero here too. The two highest
/// values are the part of the space a service reaches last, so reserving them
/// costs the least and collides with the fewest designs that already exist.
///
/// Reserving a number is not offering the operation. A service that does not
/// register refuses these exactly as it refuses anything else it does not know,
/// with [`error::argument::UNKNOWN_OPCODE`], and
/// [`op::known`](crate::op::known) — the *frame's* vocabulary — deliberately
/// does not admit them, because the frame registers nothing at this milestone.
/// `docs/rfc/0028-registration-is-an-operation.md` is the argument and the
/// reversal condition.
pub mod opcode {
    /// Register a buffer set: the memory [`Sqe::cap`](crate::Sqe::cap) names,
    /// divided into `ext[0]` equal buffers. Answered with a
    /// [`SetId`](super::SetId) in [`Cqe::ext`](crate::Cqe::ext).
    pub const REGISTER: u8 = 0xFE;

    /// Retire the set [`Sqe::buf_set`](crate::Sqe::buf_set) names. Every id of
    /// that generation is stale afterwards, and the service refuses it with
    /// `AUTHORITY`/`REVOKED` rather than resolving it against whatever refills
    /// the slot.
    pub const UNREGISTER: u8 = 0xFF;

    /// Is this opcode one of the two?
    ///
    /// The negative answer is the one that matters, exactly as it is for
    /// [`op::known`](crate::op::known): a service dispatches on this and hands
    /// everything else to its own vocabulary, so an opcode that is nearly
    /// `REGISTER` reaches the service rather than the registration path.
    #[must_use]
    pub const fn is_registration(value: u8) -> bool {
        matches!(value, REGISTER | UNREGISTER)
    }
}

/// What a registration entry asks for.
///
/// Not itself a wire type: the wire is the [`Sqe`], and this is what its fields
/// mean when the opcode is one of [`opcode`]'s two. It exists for the same
/// reason [`Name`] does — both peers have to agree where the fields are, and
/// two accounts of that is one too many.
///
/// [`Request::read`] is structural, like [`Name::read`]. It says which of the
/// two an entry is and refuses anything that fits neither. Whether the
/// capability names memory, whether the region divides into that many buffers,
/// and whether a slot is free are questions only the service's own registration
/// table can answer, and each earns a refusal named in
/// `docs/rfc/0024-a-buffer-is-owned-by-one-side.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// Register memory as a buffer set.
    Register {
        /// The memory being registered, as the submitter's own capability. The
        /// service derives a child of it, so revoking the parent — or the
        /// component ending, RFC 0008 — reaches the registration.
        /// Unit: capability-table slots, zero-based, in the submitter's own
        /// table. Zero is a valid slot and not a null.
        cap: u32,
        /// How much of that memory the set covers.
        /// Unit: bytes. Zero is refused by the service: a set covering nothing
        /// can name no buffer.
        len: u32,
        /// How many equal buffers the region divides into.
        /// Unit: buffers. Zero is refused for the same reason a `len` of zero
        /// is.
        buffers: u32,
    },
    /// Retire a set this channel holds.
    Unregister {
        /// Which set, as this channel names it.
        /// Unit: none — a registration slot and a generation packed as
        /// [`SetId`]. Zero names no set, because generations count from one.
        set: SetId,
    },
}

impl Request {
    /// Write this request into an entry.
    ///
    /// Every field the opcode reads is written and every field it does not is
    /// zeroed, because [`Request::read`] refuses a non-zero one: a client that
    /// left a stale `offset` in a reused entry would otherwise be refused for a
    /// field it never meant to fill in. The reserved word goes with them, for
    /// that same reason and not because it is this operation's — nothing may
    /// ever write it, and an entry pulled off a free list is the one place it
    /// can be non-zero without anybody having meant it. `user_data`, `class`,
    /// `deadline` and the link and drain flags are the caller's and are left
    /// alone.
    pub const fn write(self, entry: &mut Sqe) {
        entry._reserved = 0;
        entry.offset = 0;
        entry.buf_index = 0;
        entry.ext = [0; 2];
        match self {
            Self::Register { cap, len, buffers } => {
                entry.opcode = opcode::REGISTER;
                // Cleared, not set: nothing is registered yet, so this entry
                // names no set. `read` refuses it the other way round.
                entry.flags &= !flags::FIXED_BUF;
                entry.cap = cap;
                entry.len = len;
                entry.buf_set = 0;
                entry.ext[0] = buffers as u64;
            }
            Self::Unregister { set } => {
                entry.opcode = opcode::UNREGISTER;
                // Set: `buf_set` names a registered set, which is exactly what
                // the flag means. A set is not somewhere, so there is no
                // address reading of this entry on any channel.
                entry.flags |= flags::FIXED_BUF;
                entry.cap = 0;
                entry.len = 0;
                entry.buf_set = set.bits();
            }
        }
    }

    /// Read the request an entry carries.
    ///
    /// Takes no negotiated feature set, and that is the difference between this
    /// and [`Name::read`]: registration is the base protocol. Which path a
    /// *submission* takes is negotiated, and a channel that agreed shared
    /// virtual memory registers nothing — but the entry that would register is
    /// the same entry either way, and gating it on a feature would be two
    /// readings of one opcode.
    ///
    /// # The order of the checks
    ///
    /// The envelope before the operation: reserved word, then flags, then
    /// opcode — `f_ring::execute`'s order, for `f_ring::execute`'s reason. An
    /// entry with a non-zero reserved word is malformed whatever it claims to
    /// be, and reporting the opcode first would tell a caller its opcode was
    /// wrong when it was not.
    ///
    /// A service dispatching on [`opcode::is_registration`] reaches this
    /// function *instead of* its own executor, not after it, so an envelope
    /// checked only there would be an envelope not checked at all on the one
    /// entry that hands out an authority.
    ///
    /// # Errors
    ///
    /// A packed [`error`] result, so a refusal goes straight into a completion.
    /// `ARGUMENT`/[`UNKNOWN_OPCODE`](error::argument::UNKNOWN_OPCODE) for an
    /// opcode that is neither, detail the opcode.
    /// `ARGUMENT`/[`UNKNOWN_FLAG`](error::argument::UNKNOWN_FLAG) for a bit
    /// outside [`flags::KNOWN`], detail the offending bits.
    /// `ARGUMENT`/[`RESERVED_NOT_ZERO`](error::argument::RESERVED_NOT_ZERO)
    /// for [`Sqe::_reserved`](crate::Sqe) or for a field this opcode does not
    /// read, detail the offending bits — R04, because a field a peer filled in
    /// and this side skipped is two peers with different beliefs about what was
    /// asked.
    /// `ARGUMENT`/[`BAD_ADDRESS`](error::argument::BAD_ADDRESS) for
    /// [`flags::FIXED_BUF`] falling the wrong way for the opcode, detail the
    /// flags as written, and for a buffer count that does not fit its field.
    /// `AUTHORITY`/[`NO_SUCH_CAP`](error::authority::NO_SUCH_CAP) for an
    /// unregistration naming an id nobody could have issued.
    pub const fn read(entry: &Sqe) -> Result<Self, (i32, u64)> {
        if entry._reserved != 0 {
            return Err((
                error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
                entry._reserved as u64,
            ));
        }
        let unknown = entry.flags & !flags::KNOWN;
        if unknown != 0 {
            return Err((
                error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
                unknown as u64,
            ));
        }

        let bad_flags =
            Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), entry.flags as u64));
        match entry.opcode {
            opcode::REGISTER => {
                if entry.flags & flags::FIXED_BUF != 0 {
                    return bad_flags;
                }
                let unread =
                    entry.offset | entry.buf_set as u64 | entry.buf_index as u64 | entry.ext[1];
                if unread != 0 {
                    return Err((
                        error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
                        unread,
                    ));
                }
                // The count is eight bytes on the wire and four in the reading.
                // Refused rather than truncated: truncation turns a peer's
                // absurd number into a plausible one, which is the shape of
                // every length bug this crate exists not to have.
                if entry.ext[0] > u32::MAX as u64 {
                    return Err((
                        error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS),
                        entry.ext[0],
                    ));
                }
                Ok(Self::Register { cap: entry.cap, len: entry.len, buffers: entry.ext[0] as u32 })
            }
            opcode::UNREGISTER => {
                if entry.flags & flags::FIXED_BUF == 0 {
                    return bad_flags;
                }
                let unread = entry.offset
                    | entry.cap as u64
                    | entry.len as u64
                    | entry.buf_index as u64
                    | entry.ext[0]
                    | entry.ext[1];
                if unread != 0 {
                    return Err((
                        error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
                        unread,
                    ));
                }
                let set = SetId::from_bits(entry.buf_set);
                if !set.is_issuable() {
                    return Err((
                        error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                        entry.buf_set as u64,
                    ));
                }
                Ok(Self::Unregister { set })
            }
            other => {
                Err((error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE), other as u64))
            }
        }
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
    fn a_set_id_retires_a_slot_where_a_capability_handle_does() {
        // The packing is the same and the argument is the same, so the number
        // is the same. Two different retirement points would mean one of the
        // two had been reasoned about and the other copied.
        assert_eq!(SetId::RETIRED_GENERATION, crate::cap::Handle::RETIRED_GENERATION);
        assert_eq!(SetId::FIRST_GENERATION, crate::cap::Handle::FIRST_GENERATION);
        // Structurally issuable — nothing in the wire format rules it out. What
        // rules it out is the table that issues ids, in `f_ring::registry`,
        // which stops before it and retires the slot instead.
        assert!(SetId::new(0, SetId::RETIRED_GENERATION).is_issuable());
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

    /// Fill every field of an entry with something, so that a `write` which
    /// forgets to clear one is caught by the `read` beside it rather than by a
    /// service months later.
    fn dirty() -> Sqe {
        let mut entry = Sqe::ZERO;
        entry.flags = flags::LINK | flags::FIXED_BUF;
        entry.cap = 0xDEAD;
        entry.user_data = 0x1234;
        entry.deadline = 99;
        entry.offset = 0x5678;
        entry.buf_set = 0xAAAA;
        entry.buf_index = 0xBBBB;
        entry.len = 0xCCCC;
        entry.ext = [7, 8];
        entry
    }

    #[test]
    fn a_registration_round_trips_through_a_dirty_entry() {
        let asked = Request::Register { cap: 3, len: 4096, buffers: 8 };
        let mut entry = dirty();
        asked.write(&mut entry);

        assert_eq!(entry.opcode, opcode::REGISTER);
        assert_eq!(entry.flags, flags::LINK, "FIXED_BUF is cleared and LINK is the caller's");
        assert_eq!(entry.user_data, 0x1234, "the token is never the protocol's to touch");
        assert_eq!(entry.deadline, 99);
        assert_eq!(Request::read(&entry), Ok(asked));
    }

    #[test]
    fn an_unregistration_round_trips_through_a_dirty_entry() {
        let set = SetId::new(2, 5);
        let asked = Request::Unregister { set };
        let mut entry = dirty();
        entry.flags = flags::DRAIN;
        asked.write(&mut entry);

        assert_eq!(entry.opcode, opcode::UNREGISTER);
        assert_eq!(entry.flags, flags::DRAIN | flags::FIXED_BUF, "this entry names a set");
        assert_eq!(entry.buf_set, set.bits());
        assert_eq!(Request::read(&entry), Ok(asked));
    }

    #[test]
    fn an_opcode_that_is_neither_is_refused_rather_than_read_as_the_nearer_one() {
        // R04, at the one place it is most tempting to be helpful: 0xFD is one
        // below REGISTER, and a dispatcher that rounded would put a service's
        // own opcode into the registration path.
        assert!(!opcode::is_registration(0xFD));
        assert!(opcode::is_registration(opcode::REGISTER));
        assert!(opcode::is_registration(opcode::UNREGISTER));

        let mut entry = Sqe::ZERO;
        entry.opcode = 0xFD;
        assert_eq!(
            Request::read(&entry),
            Err((error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE), 0xFD))
        );
    }

    #[test]
    fn a_field_this_opcode_does_not_read_is_refused_and_not_skipped() {
        let reserved = error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO);

        let mut entry = Sqe::ZERO;
        Request::Register { cap: 1, len: 64, buffers: 2 }.write(&mut entry);
        entry.offset = 8;
        assert_eq!(Request::read(&entry), Err((reserved, 8)));

        let mut entry = Sqe::ZERO;
        Request::Unregister { set: SetId::new(1, 1) }.write(&mut entry);
        entry.len = 16;
        assert_eq!(Request::read(&entry), Err((reserved, 16)));
    }

    #[test]
    fn the_envelope_is_refused_before_the_opcode_is_believed() {
        // R04's other two cases, on the one entry that hands out an authority.
        // A service dispatches registration *instead of* running its own
        // executor over the entry, so an envelope checked only there would not
        // be checked here at all — and a peer that set a reserved word and an
        // undefined flag would be told its registration worked.
        let reserved = error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO);
        let unknown_flag = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);

        let mut entry = Sqe::ZERO;
        Request::Register { cap: 0, len: 256, buffers: 4 }.write(&mut entry);
        let well_formed = entry;
        entry._reserved = 0xDEAD_BEEF;
        assert_eq!(Request::read(&entry), Err((reserved, 0xDEAD_BEEF)));

        let mut entry = well_formed;
        entry.flags |= 1 << 7;
        assert_eq!(
            Request::read(&entry),
            Err((unknown_flag, 1 << 7)),
            "detail is the bit, not all"
        );

        // Both at once: the reserved word first, because an entry with one is
        // malformed whatever else it says, and naming the flags would send a
        // caller looking at the field it did fill in correctly.
        let mut entry = well_formed;
        entry._reserved = 1;
        entry.flags |= 1 << 6;
        assert_eq!(Request::read(&entry), Err((reserved, 1)));

        // And the same on an unregistration, which reaches the same envelope
        // through a different arm.
        let mut entry = Sqe::ZERO;
        Request::Unregister { set: SetId::new(1, 1) }.write(&mut entry);
        entry._reserved = 4;
        assert_eq!(Request::read(&entry), Err((reserved, 4)));

        // A write clears the reserved word rather than leaving whatever the
        // entry it reused had, so a client cannot refuse itself with a field
        // that is nobody's.
        let mut recycled = Sqe::ZERO;
        recycled._reserved = 0x99;
        Request::Register { cap: 1, len: 64, buffers: 1 }.write(&mut recycled);
        assert_eq!(recycled._reserved, 0);
        assert!(Request::read(&recycled).is_ok());
    }

    #[test]
    fn every_flag_this_build_defines_is_in_the_known_list() {
        // The list is what R04's refusal is measured against, and a flag added
        // without being added to it would be a bit silently accepted by one
        // reader of the envelope and refused by the other.
        for defined in [flags::LINK, flags::DRAIN, flags::FIXED_BUF, flags::NO_CQE] {
            assert_ne!(flags::KNOWN & defined, 0, "a defined flag outside the known list");
        }
        assert_eq!(flags::KNOWN, 0b1111, "four flags, and the top four bits still nobody's");
    }

    #[test]
    fn the_flag_decides_which_request_an_entry_is_and_is_refused_the_wrong_way_round() {
        // The two opcodes disagree about FIXED_BUF on purpose: a registration
        // names no set yet and an unregistration names nothing else. An entry
        // with the flag falling the other way is refused rather than read past.
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);

        let mut entry = Sqe::ZERO;
        Request::Register { cap: 0, len: 64, buffers: 1 }.write(&mut entry);
        entry.flags |= flags::FIXED_BUF;
        assert_eq!(Request::read(&entry), Err((bad, u64::from(flags::FIXED_BUF))));

        let mut entry = Sqe::ZERO;
        Request::Unregister { set: SetId::new(1, 1) }.write(&mut entry);
        entry.flags &= !flags::FIXED_BUF;
        assert_eq!(Request::read(&entry), Err((bad, 0)));
    }

    #[test]
    fn an_unregistration_of_an_id_nobody_could_have_issued_is_refused() {
        let mut entry = Sqe::ZERO;
        entry.opcode = opcode::UNREGISTER;
        entry.flags = flags::FIXED_BUF;
        entry.buf_set = SetId::new(4, 0).bits();
        assert_eq!(
            Request::read(&entry),
            Err((error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP), 4))
        );
    }

    #[test]
    fn a_buffer_count_that_does_not_fit_its_field_is_refused_and_not_truncated() {
        // Truncation would turn 2^32 into zero, and zero is a value the service
        // has its own refusal for — so the peer would be told the wrong thing
        // about the wrong field.
        let mut entry = Sqe::ZERO;
        entry.opcode = opcode::REGISTER;
        entry.ext[0] = 1 << 32;
        assert_eq!(
            Request::read(&entry),
            Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), 1 << 32))
        );
    }

    #[test]
    fn a_set_id_is_read_out_of_the_completion_that_issued_it() {
        let set = SetId::new(6, 3);
        let issued =
            Cqe { user_data: 1, result: 0, flags: 0, timestamp: 0, ext: set.bits() as u64 };
        assert_eq!(SetId::from_completion(&issued), Ok(set));

        // A refusal issued no id, and the caller gets the refusal rather than
        // an absence it would have to interpret.
        let refused = Cqe {
            user_data: 1,
            result: error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
            flags: 0,
            timestamp: 0,
            ext: 16,
        };
        assert_eq!(
            SetId::from_completion(&refused),
            Err((error::RESOURCE, error::resource::QUOTA_EXHAUSTED))
        );

        // A success carrying nothing, and one carrying bits above the id.
        let empty = Cqe { user_data: 1, result: 0, flags: 0, timestamp: 0, ext: 0 };
        assert_eq!(
            SetId::from_completion(&empty),
            Err((error::AUTHORITY, error::authority::NO_SUCH_CAP))
        );
        let wide = Cqe { user_data: 1, result: 0, flags: 0, timestamp: 0, ext: 1 << 40 };
        assert_eq!(
            SetId::from_completion(&wide),
            Err((error::ARGUMENT, error::argument::RESERVED_NOT_ZERO))
        );
    }
}
