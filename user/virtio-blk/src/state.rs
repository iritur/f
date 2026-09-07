// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What this driver hands to the build that replaces it: thirty-two bytes per
//! record, sixteen records, and a check word so that a transfer which did not
//! cross intact is refused by its reader.
//!
//! `user/virtio-blk/manifest.toml` declares `schema = 1`, `record_bytes = 32`
//! and `records_max = 16`, and says in as many words that the byte layout is
//! `E2-B06`'s — *because a layout with no reader is a format nobody has
//! disagreed with yet*. This module is that layout and this crate is both of its
//! readers.
//!
//! # Why the registration table, and nothing else
//!
//! RFC 0063 states the cost of `restart_only` and it is one sentence: a `SetId`
//! names a slot in *this instance's* table, a fresh instance's table has never
//! been filled, so every id a client still holds is answered `NO_SUCH_CAP` and
//! every buffer set has to be registered again before a single read can be
//! submitted. Nothing is lost and nothing is corrupted — a refusal is an answer
//! — but the client does work again at a moment nobody asked it to.
//!
//! **That cost is the whole of what `in_place` buys here**, so it is the whole
//! of what crosses. The driver's other state is either the client's (the buffers
//! themselves, which the client owns and the frame's translations reach), the
//! device's (the queue, which the incoming instance re-negotiates through
//! `Driver::start`), or a counter (`Counters`, published through
//! `routing::REPORT`, whose loss costs a reader a discontinuity and costs a
//! client nothing). A record carrying a counter would make the swap look
//! seamless in a log while changing nothing a client can observe, which is the
//! wrong thing to spend a fixed-width record on.
//!
//! # Why a table crosses as the operations that made it
//!
//! `f_ring::registry::Table`'s slots are private and it cannot be copied, for
//! the reason that type exists at all: *a registration table that could be
//! copied would be two tables issuing one set of identifiers*. Widening `ring/`
//! so a transfer could fabricate one would put a hole in exactly the type RFC
//! 0028 built to stop a peer naming a set it was not issued.
//!
//! So a record is one **deed** — a registration, an unregistration, or every set
//! retired — and the incoming instance rebuilds its table by replaying them
//! through the same `Table::execute` the live path uses. The identifiers the
//! replay issues are therefore the identifiers the client already holds, which
//! is the property that makes the swap invisible; and they are issued by the
//! real table rather than asserted by this module, which is what makes it a
//! reconstruction rather than a claim. `sim/src/service.rs` makes the same
//! argument about a snapshot and arrived at it first.
//!
//! # What the check word is for, and what it is not
//!
//! It is for a record that did not cross intact: a window written short, a
//! record read at the wrong offset, a byte the incoming build interpreted
//! differently from the outgoing one. It is **not** a defence against an
//! adversary — the window is memory the incoming instance bought out of its own
//! `Untyped` and granted to the outgoing one, so both ends are the same
//! component and a component that wanted to lie to itself could compute a
//! matching check. Saying which of the two it is here is the point: RFC 0060's
//! barriers are what make a *publish* atomic and this is the much smaller claim
//! that a transfer is verified rather than assumed.
//!
//! *Reversal:* a transfer that crosses a boundary two builds do not both own —
//! a window in shared memory a third party can reach, say. Then a fold is not
//! enough and the answer is `f-hash` over the window with the digest carried
//! beside `records`, which is a `Declaration` change and therefore RFC 0063's
//! reversal rather than this module's.

use f_abi::Sqe;
use f_abi::buf::SetId;
use f_ring::registry::{registration, unregistration};

/// The state-record schema this build writes and reads.
///
/// The **component's** ordinal and not the ABI's: it is compared only against
/// another build of this same driver, and a component elsewhere in the tree that
/// also declares `1` has said nothing to this one. It matches
/// `user/virtio-blk/manifest.toml`'s `[transfer] schema`, and
/// [`tests::the_declaration_matches_the_manifest`] is what says so out loud.
/// Unit: none — a state-record schema ordinal.
pub const SCHEMA: u32 = 1;

/// The fixed width of one record, in bytes.
///
/// Thirty-two, which is the manifest's `record_bytes` and a multiple of
/// `f_abi::transfer::RECORD_ALIGN`. Fixed because the window is records laid end
/// to end and read in place, so the frame's arithmetic over it is a
/// multiplication rather than a parse.
/// Unit: bytes.
pub const RECORD_BYTES: u32 = 32;

/// The most records this build will hand over.
///
/// Sixteen, the manifest's `records_max`, and it is a bound on *deeds* rather
/// than on live registrations — an unregistration is a record too. The driver's
/// own table holds far fewer sets than that; what this bounds is how much
/// history a long-lived instance may accumulate before a swap has to be
/// abandoned as `WindowTooSmall`, which is why [`Journal::full`] exists and is
/// asked before a deed is recorded rather than at the swap.
/// Unit: records.
pub const RECORDS_MAX: u32 = 16;

/// What one record says happened.
///
/// Three, because [`Journal`] replays through `Table::execute` and `retire_all`,
/// and those are the only two calls in this crate that move a slot's generation
/// or its liveness. A fourth kind would be a fourth entry here rather than a
/// special case at replay. Zero is not a kind, for
/// `f_abi::transfer::mode`'s reason: a zeroed window declares nothing rather
/// than declaring the first thing.
pub mod deed {
    /// A client registered a buffer set. The record carries what it asked for.
    pub const REGISTER: u8 = 1;
    /// A client retired one. The record carries the set id it named.
    pub const UNREGISTER: u8 = 2;
    /// Every set this instance held was retired at once.
    pub const RETIRE_ALL: u8 = 3;

    /// Whether a wire value is a kind this build knows. Fail closed, R04.
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, REGISTER | UNREGISTER | RETIRE_ALL)
    }
}

/// One state record, thirty-two bytes, as it sits in the transfer window.
///
/// `repr(C)` and asserted below, for `f_abi::manifest::Record`'s reason: the
/// layout is the format, and a field reordered or widened is a build failure
/// with a number in it rather than two builds disagreeing about one window.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record {
    /// The token the client's own submission carried, replayed unchanged.
    ///
    /// Carried rather than reissued because the completion the replay produces
    /// is matched by it, and a token this module invented would be a completion
    /// no client is waiting for.
    /// Unit: none — an opaque token minted by the client.
    pub token: u64,
    /// What the deed named. A set id's bits under [`deed::UNREGISTER`], and the
    /// length in bytes the registration asked for under [`deed::REGISTER`].
    ///
    /// One field for two meanings, and it is the one compression in this layout:
    /// the two are never both present, thirty-two bytes is the declared width,
    /// and a record that carried both would spend eight bytes on a hole in every
    /// deed. [`Record::kind`] is what says which, and `read` refuses a kind it
    /// does not know rather than reading this field anyway.
    /// Unit: per-kind — bytes under `REGISTER`, none under `UNREGISTER`.
    pub named: u64,
    /// Buffers in the registered set, under [`deed::REGISTER`]. Zero otherwise.
    /// Unit: buffers.
    pub buffers: u32,
    /// The capability slot the registration named, under [`deed::REGISTER`].
    /// Unit: none — a capability-table slot.
    pub cap: u32,
    /// Which deed. Unit: none — a [`deed`] constant. Zero is not a kind.
    pub kind: u8,
    /// Reserved. Must be zero — a non-zero value is refused rather than
    /// ignored, per R04.
    /// Unit: none; this is not a quantity and does not become one without a
    /// [`SCHEMA`] bump.
    pub _reserved: [u8; 3],
    /// A fold over the twenty-eight bytes before it.
    ///
    /// What makes the transfer verified rather than assumed on the *reader's*
    /// side, which is the side that matters: a harness comparing the two tables
    /// afterwards is checking a swap that already happened, and this refuses one
    /// that should not. See the module documentation for what it is not.
    /// Unit: none — a check word.
    pub check: u32,
}

const _: () = assert!(core::mem::size_of::<Record>() == RECORD_BYTES as usize);
const _: () = assert!(core::mem::align_of::<Record>() == 8);

/// The offset of [`Record::check`] within a record. Unit: bytes.
///
/// Named rather than written as `28` at both ends, because the fold and the
/// layout have to move together and a literal in two functions is where they
/// stop doing that.
const CHECKED_BYTES: usize = 28;

impl Record {
    /// A record with nothing in it. Refused by [`Record::read`], because
    /// [`Record::kind`] is zero and zero is not a kind.
    pub const EMPTY: Self =
        Self { token: 0, named: 0, buffers: 0, cap: 0, kind: 0, _reserved: [0; 3], check: 0 };

    /// The record for a registration this instance answered.
    #[must_use]
    pub const fn registered(token: u64, cap: u32, len_bytes: u32, buffers: u32) -> Self {
        Self::sealed(Self {
            token,
            named: len_bytes as u64,
            buffers,
            cap,
            kind: deed::REGISTER,
            ..Self::EMPTY
        })
    }

    /// The record for an unregistration this instance answered.
    #[must_use]
    pub const fn unregistered(token: u64, set: u32) -> Self {
        Self::sealed(Self { token, named: set as u64, kind: deed::UNREGISTER, ..Self::EMPTY })
    }

    /// The record for every set being retired at once.
    #[must_use]
    pub const fn retired_all() -> Self {
        Self::sealed(Self { kind: deed::RETIRE_ALL, ..Self::EMPTY })
    }

    /// The same record with its check word computed.
    const fn sealed(mut record: Self) -> Self {
        record.check = record.fold();
        record
    }

    /// The fold over everything but the check word itself.
    ///
    /// FNV-1a over the record's own bytes in declaration order, written out
    /// field by field rather than over a `repr(C)` view of `self`: reading the
    /// struct as bytes needs `unsafe`, this crate forbids `unsafe` at compile
    /// time (RFC 0001, RFC 0033), and a driver that needed one exception to
    /// compute a checksum would have needed it for the wrong reason.
    const fn fold(&self) -> u32 {
        let mut bytes = [0u8; CHECKED_BYTES];
        let token = self.token.to_le_bytes();
        let named = self.named.to_le_bytes();
        let buffers = self.buffers.to_le_bytes();
        let cap = self.cap.to_le_bytes();
        // A `const fn` cannot iterate a slice of slices, so the copy is written
        // out. Eight, eight, four, four, one, three — twenty-eight, which is
        // every byte of the record except the check word itself.
        let mut nth = 0;
        while nth < 8 {
            bytes[nth] = token[nth];
            bytes[8 + nth] = named[nth];
            nth += 1;
        }
        let mut nth = 0;
        while nth < 4 {
            bytes[16 + nth] = buffers[nth];
            bytes[20 + nth] = cap[nth];
            nth += 1;
        }
        bytes[24] = self.kind;
        let mut nth = 0;
        while nth < 3 {
            bytes[25 + nth] = self._reserved[nth];
            nth += 1;
        }

        let mut hash = 0x811c_9dc5u32;
        let mut nth = 0;
        while nth < CHECKED_BYTES {
            hash ^= bytes[nth] as u32;
            hash = hash.wrapping_mul(0x0100_0193);
            nth += 1;
        }
        hash
    }

    /// Whether this record is one this build will act on.
    ///
    /// Fail closed, R04: an unknown kind, a non-zero reserved byte, or a check
    /// word that does not match. Each is refused rather than ignored, and the
    /// caller's answer to any of them is the same — the swap is abandoned with
    /// the outgoing occupant still serving, which is the whole reason phase A is
    /// reversible.
    #[must_use]
    pub const fn intact(&self) -> bool {
        deed::known(self.kind)
            && self._reserved[0] == 0
            && self._reserved[1] == 0
            && self._reserved[2] == 0
            && self.check == self.fold()
    }

    /// This record as the thirty-two bytes it occupies in the window.
    ///
    /// Written field by field in declaration order rather than by casting
    /// `self` to a byte slice, and the reason is the same one [`Record::fold`]
    /// gives: that cast is `unsafe`, this crate forbids `unsafe` at compile time
    /// (RFC 0001, RFC 0033), and a driver that needed one exception to copy a
    /// struct out would have needed it for the wrong reason. It also makes the
    /// byte order **little-endian and stated** rather than the host's, which is
    /// what a format has to be even when both readers are the same build: the
    /// two ends of a swap are two builds by definition, and *this build's
    /// endianness* is not something a format may assume.
    #[must_use]
    pub const fn to_bytes(&self) -> [u8; RECORD_BYTES as usize] {
        let mut out = [0u8; RECORD_BYTES as usize];
        let token = self.token.to_le_bytes();
        let named = self.named.to_le_bytes();
        let buffers = self.buffers.to_le_bytes();
        let cap = self.cap.to_le_bytes();
        let check = self.check.to_le_bytes();
        let mut nth = 0;
        while nth < 8 {
            out[nth] = token[nth];
            out[8 + nth] = named[nth];
            nth += 1;
        }
        let mut nth = 0;
        while nth < 4 {
            out[16 + nth] = buffers[nth];
            out[20 + nth] = cap[nth];
            out[28 + nth] = check[nth];
            nth += 1;
        }
        out[24] = self.kind;
        let mut nth = 0;
        while nth < 3 {
            out[25 + nth] = self._reserved[nth];
            nth += 1;
        }
        out
    }

    /// One record read out of the window, exactly as it was written.
    ///
    /// It does **not** judge: [`Record::intact`] is what does, and keeping the
    /// two apart is deliberate. A reader that refused inside the decoder could
    /// not say *which* record of the sixteen was wrong, and a swap that is
    /// abandoned should name the record it abandoned on.
    #[must_use]
    pub const fn from_bytes(bytes: &[u8; RECORD_BYTES as usize]) -> Self {
        let mut token = [0u8; 8];
        let mut named = [0u8; 8];
        let mut buffers = [0u8; 4];
        let mut cap = [0u8; 4];
        let mut check = [0u8; 4];
        let mut reserved = [0u8; 3];
        let mut nth = 0;
        while nth < 8 {
            token[nth] = bytes[nth];
            named[nth] = bytes[8 + nth];
            nth += 1;
        }
        let mut nth = 0;
        while nth < 4 {
            buffers[nth] = bytes[16 + nth];
            cap[nth] = bytes[20 + nth];
            check[nth] = bytes[28 + nth];
            nth += 1;
        }
        let mut nth = 0;
        while nth < 3 {
            reserved[nth] = bytes[25 + nth];
            nth += 1;
        }
        Self {
            token: u64::from_le_bytes(token),
            named: u64::from_le_bytes(named),
            buffers: u32::from_le_bytes(buffers),
            cap: u32::from_le_bytes(cap),
            kind: bytes[24],
            _reserved: reserved,
            check: u32::from_le_bytes(check),
        }
    }

    /// The submission that replaying this record hands to `Table::execute`, or
    /// `None` for [`deed::RETIRE_ALL`], which is not an entry.
    ///
    /// The real `f_ring::registry` constructors and not an `Sqe` this module
    /// filled in: an entry built here would be a second opinion about what a
    /// registration looks like, and the first opinion is the one the live path
    /// already uses.
    #[must_use]
    pub fn replay(&self) -> Option<Sqe> {
        match self.kind {
            deed::REGISTER => Some(registration(
                self.token,
                self.cap,
                u32::try_from(self.named).unwrap_or(u32::MAX),
                self.buffers,
            )),
            deed::UNREGISTER => Some(unregistration(
                self.token,
                SetId::from_bits(u32::try_from(self.named).unwrap_or(u32::MAX)),
            )),
            // Not an entry. `Table::retire_all` takes no submission, so a caller
            // that turned this into one would be inventing a request no client
            // made.
            _ => None,
        }
    }
}

/// The deeds one instance will hand over, in the order they happened.
///
/// A fixed array and not a growable one, for the reason the declaration is fixed
/// at all: this crate is `no_std` with no allocator, the window it writes into
/// is bounded by [`RECORDS_MAX`], and a journal that could outgrow the window
/// would discover it at the swap with a client's submissions already held.
/// [`Journal::full`] is asked *before* a deed is recorded, so an instance that
/// has done more work than one window holds knows it long before anybody asks it
/// to drain.
#[derive(Clone, Copy, Debug)]
pub struct Journal {
    deeds: [Record; RECORDS_MAX as usize],
    /// How many of them are real. Unit: records.
    used: u32,
    /// Whether a deed was ever dropped for want of room. Unit: none.
    overflowed: bool,
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

impl Journal {
    /// An instance that has answered nothing yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { deeds: [Record::EMPTY; RECORDS_MAX as usize], used: 0, overflowed: false }
    }

    /// How many records a swap would hand over. Unit: records.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.used
    }

    /// Whether this instance has anything to hand over at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Whether one more deed would not fit.
    #[must_use]
    pub const fn full(&self) -> bool {
        self.used >= RECORDS_MAX
    }

    /// Whether a deed has ever been dropped for want of room.
    ///
    /// **The honest half of a fixed bound.** An instance that overflowed cannot
    /// hand over a complete history, so it must not hand over a partial one and
    /// call it a transfer — a replay of a truncated journal produces a table
    /// missing registrations the client still holds, which is exactly the
    /// dropped operation a swap exists to avoid. The caller's answer is to
    /// declare itself not quiescent-transferable and take the restart, and
    /// `sim/src/swap.rs` is where that is a run rather than a sentence.
    #[must_use]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Write one deed down. Refused, and remembered as refused, when full.
    pub const fn record(&mut self, deed: Record) {
        if self.full() {
            self.overflowed = true;
            return;
        }
        self.deeds[self.used as usize] = deed;
        self.used += 1;
    }

    /// The deeds, in the order they happened.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        let used = self.used as usize;
        self.deeds.split_at(used.min(self.deeds.len())).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_not_a_kind_and_an_empty_window_is_refused() {
        assert!(!deed::known(0), "a zeroed window declares no deed");
        assert!(!Record::EMPTY.intact(), "and a zeroed record is not readable as one");
        assert!(!deed::known(4), "an unknown kind is refused and never guessed at");
    }

    #[test]
    fn a_sealed_record_reads_back_and_a_flipped_byte_does_not() {
        let record = Record::registered(0x1234, 2, 4096, 8);
        assert!(record.intact());

        // Every field, one at a time. A check that caught only the first would
        // pass this test with the last three unprotected.
        for broken in [
            Record { token: record.token ^ 1, ..record },
            Record { named: record.named ^ 1, ..record },
            Record { buffers: record.buffers ^ 1, ..record },
            Record { cap: record.cap ^ 1, ..record },
            Record { kind: deed::UNREGISTER, ..record },
            Record { _reserved: [1, 0, 0], ..record },
            Record { check: record.check ^ 1, ..record },
        ] {
            assert!(
                !broken.intact(),
                "a record that did not cross intact must be refused by its reader, not \
                 replayed: {broken:?}"
            );
        }
    }

    /// Thirty-two bytes out, the same record back, and a flipped byte anywhere
    /// in the window refused by the reader rather than replayed.
    #[test]
    fn a_record_crosses_the_window_as_bytes_and_a_flipped_one_is_caught() {
        for record in [
            Record::registered(0x0F0E_0D0C_0B0A_0908, 3, 0x1234_5678, 9),
            Record::unregistered(7, 0x00FF_0001),
            Record::retired_all(),
        ] {
            let bytes = record.to_bytes();
            assert_eq!(bytes.len(), RECORD_BYTES as usize);
            let back = Record::from_bytes(&bytes);
            assert_eq!(back, record, "a record must survive the window unchanged");
            assert!(back.intact());

            // Every byte of the window, one at a time. A check that covered
            // twenty-eight of thirty-two would leave four bytes a corruption
            // could hide in, and the window is the one place in a swap where
            // nothing else is watching.
            for at in 0..bytes.len() {
                let mut broken = bytes;
                broken[at] ^= 0x80;
                assert!(
                    !Record::from_bytes(&broken).intact(),
                    "byte {at} of the window could be flipped without the reader noticing"
                );
            }
        }
    }

    #[test]
    fn a_record_replays_as_the_entry_the_client_submitted() {
        let record = Record::registered(0xABCD, 3, 2048, 4);
        let entry = record.replay().expect("a registration is an entry");
        assert_eq!(entry.user_data, 0xABCD);
        // Field by field, because `Sqe` is a wire type and deliberately carries
        // no `PartialEq`: RFC 0024's ownership types are what compare entries,
        // and a derive here would be this test asking `f-abi` for a trait it
        // declined to have.
        let built = registration(0xABCD, 3, 2048, 4);
        assert_eq!(
            (entry.opcode, entry.flags, entry.len, entry.cap, entry.offset, entry.ext),
            (built.opcode, built.flags, built.len, built.cap, built.offset, built.ext),
            "the replayed entry is the one `f_ring::registry` builds, not one this module \
             filled in"
        );

        let retire = Record::retired_all();
        assert!(retire.intact());
        assert!(retire.replay().is_none(), "retiring every set is not a submission");
    }

    #[test]
    fn a_journal_that_overflows_says_so_rather_than_handing_over_a_partial_history() {
        let mut journal = Journal::new();
        assert!(journal.is_empty());
        for nth in 0..RECORDS_MAX {
            journal.record(Record::registered(u64::from(nth), 0, 512, 4));
        }
        assert_eq!(journal.len(), RECORDS_MAX);
        assert!(journal.full());
        assert!(!journal.overflowed(), "exactly the declared bound is not an overflow");

        journal.record(Record::retired_all());
        assert_eq!(journal.len(), RECORDS_MAX, "the window's bound is not exceeded");
        assert!(
            journal.overflowed(),
            "and the instance knows it cannot hand over a complete history"
        );
        assert_eq!(journal.records().len(), RECORDS_MAX as usize);
    }

    /// The three numbers in `user/virtio-blk/manifest.toml`'s `[transfer]`
    /// table, and the constants this module writes records against.
    ///
    /// Read out of the file rather than restated, because a constant that agreed
    /// with a manifest at the moment it was written and drifted afterwards is
    /// the failure this test exists for. `Record::read` in the frame refuses a
    /// declaration whose `record_bytes` is not a multiple of `RECORD_ALIGN`; it
    /// cannot know that *this* build writes thirty-two-byte records, and the
    /// day those two disagree is the day a swap replays garbage.
    #[test]
    fn the_declaration_matches_the_manifest() {
        let manifest = include_str!("../manifest.toml");
        let after = manifest.split("[transfer]").nth(1).expect("a [transfer] table");
        let table: &str = after.split("\n[").next().unwrap_or(after);
        let field = |name: &str| -> Option<u32> {
            table
                .lines()
                .find(|line| line.trim_start().starts_with(name))
                .and_then(|line| line.split('=').nth(1))
                .and_then(|value| value.trim().parse::<u32>().ok())
        };
        assert!(table.contains("in_place"), "the driver stopped declaring an in-place transfer");
        assert_eq!(field("schema"), Some(SCHEMA));
        assert_eq!(field("record_bytes"), Some(RECORD_BYTES));
        assert_eq!(field("records_max"), Some(RECORDS_MAX));
        assert_eq!(
            RECORD_BYTES % f_abi::transfer::RECORD_ALIGN,
            0,
            "a record width that is not a multiple of the alignment puts every other record \
             on an odd boundary"
        );
    }
}
