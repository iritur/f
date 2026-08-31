// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where everything in a channel mapping is.
//!
//! One channel is one contiguous shared mapping. This module is the arithmetic
//! that says which byte holds what, and it is wire: two peers that disagree
//! about it read each other's cursors as each other's entries.
//!
//! # Why the offsets are computed and also carried
//!
//! [`ChannelHeader::sqe_offset`] and [`ChannelHeader::cqe_offset`] are written
//! by the peer that created the mapping, and this module can compute both from
//! `ring_size` alone. That looks redundant, and it is the opposite: the carried
//! values are what a reader *checks the computation against*. A peer whose
//! layout arithmetic differs from ours — a different version, a different
//! toolchain's padding, a deliberately hostile mapping — is caught at setup by
//! the two numbers disagreeing, rather than at the first read by a cursor being
//! interpreted as an entry.
//!
//! So [`Layout::describe`] writes them and [`Layout::adopt`] refuses a header
//! whose values are not the ones this build would have produced. Neither side
//! trusts the other's arithmetic; both sides state it.
//!
//! # Why the arena's length is not in the header
//!
//! Because the header cannot know it. The inline arena runs from the end of the
//! completion ring to the end of the mapping, and how long the mapping is, is
//! known to whoever mapped it — not to whoever wrote the first cache line of
//! it. A length in the header would be a peer-supplied claim about a region
//! this side already knows the true extent of, which is a strictly worse
//! arrangement than asking the mapper.
//!
//! See `docs/design/ring-scene-boot.html` section 02.

use crate::{ChannelHeader, error};

/// One cache line, and the grain every region in a channel is aligned to.
///
/// Not a tuning constant. The cursors are on separate lines because sharing one
/// costs 100–150 cycles per operation through false sharing, and the entry
/// array is line-aligned because an [`Sqe`](crate::Sqe) is exactly one line.
pub const LINE: u32 = 64;

/// Byte offset of the [`ChannelHeader`]. First line of the mapping.
pub const HEADER: u32 = 0;

/// Byte offset of the producer cursor. Its own line, deliberately.
pub const HEAD: u32 = LINE;

/// Byte offset of the consumer cursor and the consumer's flags word. Its own
/// line, for the same reason.
pub const TAIL: u32 = 2 * LINE;

/// Byte offset of the consumer's flags word.
///
/// On the consumer's own line and not on its own, deliberately. The consumer
/// writes both the tail and this word, so they share a line by the same
/// argument that keeps the two *cursors* apart: what costs is two peers writing
/// one line, not one peer writing two words.
pub const FLAGS: u32 = TAIL + 4;

/// Byte offset of the completion ring's producer cursor, advanced by the
/// service. Its own line — RFC 0018.
pub const CQ_HEAD: u32 = 3 * LINE;

/// Byte offset of the completion ring's consumer cursor, advanced by the
/// client. Its own line.
pub const CQ_TAIL: u32 = 4 * LINE;

/// Byte offset of the submission index ring: `ring_size` `u32` slots.
///
/// Section 02 puts this at `0x00C0`. It is at `0x0140`, because the table it
/// came from gave the completion ring no cursors and a ring without cursors
/// cannot report its own occupancy. RFC 0018 is the argument, including why the
/// two new lines went here rather than somewhere that would have left this
/// offset alone.
pub const SQ_INDEX: u32 = 5 * LINE;

/// The largest ring this arithmetic will describe.
///
/// Chosen so that every offset below stays inside a `u32` with room to spare:
/// the entry array is the widest region at 64 bytes an entry, so 2^24 entries
/// is a gibibyte and 2^25 would be the first size where the total could not be
/// stated. A ring this large is already far past the point where the working
/// set stops fitting in cache, so the ceiling costs nothing real.
pub const MAX_ENTRIES: u32 = 1 << 24;

/// Where each region of one channel mapping begins.
///
/// Construct with [`Layout::new`] for a mapping this side is creating, or with
/// [`Layout::adopt`] for one a peer created. The two differ in exactly one way,
/// and it is the important one: `adopt` believes nothing it is told.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    entries: u32,
    sqe: u32,
    cqe: u32,
    arena: u32,
    arena_len: u32,
}

/// Round `value` up to the next multiple of [`LINE`], or `None` on overflow.
const fn line_align(value: u32) -> Option<u32> {
    match value.checked_add(LINE - 1) {
        Some(sum) => Some(sum & !(LINE - 1)),
        None => None,
    }
}

impl Layout {
    /// Lay out a channel with `entries` slots in each ring and `arena_len`
    /// bytes of inline arena.
    ///
    /// Returns `None` for a ring size that is not a power of two, is zero, or
    /// is past [`MAX_ENTRIES`] — and for any arena that would push the total
    /// past a `u32`. Refusal rather than saturation: a mapping whose length was
    /// silently clamped is a mapping two sides would disagree about.
    ///
    /// # Why the completion ring has as many slots as the submission ring
    ///
    /// The layout table in section 02 writes them as `N` and `M`, leaving the
    /// two free to differ. They do not differ here, and the reason is that a
    /// completion ring smaller than its submission ring can fill while
    /// operations are still outstanding — at which point a service either
    /// blocks, drops a completion, or grows a queue of its own. All three are
    /// worse than the memory: at `M == N` the ring cannot fill, because an
    /// operation must have left the submission ring to produce a completion.
    /// A service that wants fewer completions than submissions has
    /// [`flags::NO_CQE`](crate::flags::NO_CQE), which says so on the entry
    /// rather than in the geometry.
    #[must_use]
    pub const fn new(entries: u32, arena_len: u32) -> Option<Self> {
        if entries == 0 || !entries.is_power_of_two() || entries > MAX_ENTRIES {
            return None;
        }

        // The index ring, then the entry array on a line boundary.
        let Some(index_bytes) = entries.checked_mul(4) else { return None };
        let Some(after_index) = SQ_INDEX.checked_add(index_bytes) else { return None };
        let Some(sqe) = line_align(after_index) else { return None };

        let Some(sqe_bytes) = entries.checked_mul(64) else { return None };
        let Some(cqe) = sqe.checked_add(sqe_bytes) else { return None };

        let Some(cqe_bytes) = entries.checked_mul(32) else { return None };
        let Some(after_cqe) = cqe.checked_add(cqe_bytes) else { return None };
        // A one-entry ring leaves the arena on a 32-byte boundary; every larger
        // one is already aligned. Stated as arithmetic rather than as a special
        // case, so the small ring the tests use is not a different shape.
        let Some(arena) = line_align(after_cqe) else { return None };

        if arena.checked_add(arena_len).is_none() {
            return None;
        }

        Some(Self { entries, sqe, cqe, arena, arena_len })
    }

    /// Adopt the layout a peer's header describes, having checked it is the one
    /// this build would have produced.
    ///
    /// `mapping_len` is the true length of the mapping, from whoever mapped it
    /// — never from the header, which is the peer's claim and not a fact.
    ///
    /// # Errors
    ///
    /// A packed [`error`] result so a refusal can be written straight into a
    /// completion. [`error::argument::MALFORMED_HEADER`] for a header that
    /// fails [`ChannelHeader::is_valid`], names a ring size this build will not
    /// lay out, states an offset that is not the computed one, or describes
    /// more channel than the mapping actually holds.
    pub fn adopt(header: &ChannelHeader, mapping_len: u32) -> Result<Self, i32> {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

        if !header.is_valid() {
            return Err(malformed);
        }

        // Arena length is not yet known — it is whatever the mapping has left
        // over — so lay out with none and fill it in below.
        let Some(computed) = Self::new(header.ring_size, 0) else { return Err(malformed) };

        // The check this whole module exists for. Not `>=`, not `within a
        // page`: equal to what this build computes, or refused.
        if header.sqe_offset != computed.sqe || header.cqe_offset != computed.cqe {
            return Err(malformed);
        }

        // Every region must be inside the mapping, arena or no arena.
        let Some(arena_len) = mapping_len.checked_sub(computed.arena) else {
            return Err(malformed);
        };

        Ok(Self { arena_len, ..computed })
    }

    /// The header describing this layout, for a peer to adopt.
    ///
    /// `epoch` counts this peer's restarts; `offers` and `requires` are what
    /// this side implements and what it cannot proceed without.
    #[must_use]
    pub const fn describe(&self, epoch: u32, offers: u64, requires: u64) -> ChannelHeader {
        ChannelHeader {
            magic: crate::CHANNEL_MAGIC,
            features: offers,
            features_required: requires,
            abi_version: crate::ABI_VERSION,
            abi_version_min: crate::ABI_VERSION_MIN,
            ring_size: self.entries,
            sqe_offset: self.sqe,
            cqe_offset: self.cqe,
            epoch,
            _reserved: [0; 4],
        }
    }

    /// Slots in each ring. Unit: entries. A power of two, never zero.
    #[must_use]
    pub const fn entries(&self) -> u32 {
        self.entries
    }

    /// The mask that turns a free-running cursor into a slot.
    ///
    /// Unit: none — a bitmask, always `entries - 1`.
    #[must_use]
    pub const fn mask(&self) -> u32 {
        self.entries - 1
    }

    /// Byte offset of the submission index ring. Unit: bytes from the first
    /// byte of the mapping.
    #[must_use]
    pub const fn sq_index_offset(&self) -> u32 {
        SQ_INDEX
    }

    /// Byte offset of the submission entry array. Unit: bytes from the first
    /// byte of the mapping.
    #[must_use]
    pub const fn sqe_offset(&self) -> u32 {
        self.sqe
    }

    /// Byte offset of the completion ring. Unit: bytes from the first byte of
    /// the mapping.
    #[must_use]
    pub const fn cqe_offset(&self) -> u32 {
        self.cqe
    }

    /// Byte offset of the inline arena. Unit: bytes from the first byte of the
    /// mapping.
    #[must_use]
    pub const fn arena_offset(&self) -> u32 {
        self.arena
    }

    /// Bytes of inline arena. Unit: bytes. Zero is a channel whose opcodes
    /// carry everything in the entry, which is legal.
    #[must_use]
    pub const fn arena_len(&self) -> u32 {
        self.arena_len
    }

    /// Bytes the whole mapping needs. Unit: bytes.
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.arena + self.arena_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixed_region_has_its_own_line_except_the_flags() {
        // The one property the fixed offsets exist to have. A regression here
        // is the 100–150 cycles a shared cursor line costs, and it is invisible
        // in every functional test.
        let lines = [HEADER, HEAD, TAIL, CQ_HEAD, CQ_TAIL, SQ_INDEX];
        for (i, offset) in lines.iter().enumerate() {
            assert_eq!(offset % LINE, 0, "region {i} at {offset} is not line-aligned");
            if i > 0 {
                assert_eq!(*offset, lines[i - 1] + LINE, "region {i} does not follow the last");
            }
        }
        // And the exception, which is on the consumer's line on purpose.
        assert_eq!(FLAGS / LINE, TAIL / LINE);
    }

    #[test]
    fn regions_are_ordered_aligned_and_disjoint() {
        for shift in 0..12 {
            let n = 1u32 << shift;
            let l = Layout::new(n, 4096).expect("a power of two under the ceiling");

            assert_eq!(l.sq_index_offset(), SQ_INDEX);
            assert!(
                l.sqe_offset() >= SQ_INDEX + 4 * n,
                "the index ring must fit before the entries"
            );
            assert_eq!(l.cqe_offset(), l.sqe_offset() + 64 * n);
            assert!(l.arena_offset() >= l.cqe_offset() + 32 * n);
            assert_eq!(l.total(), l.arena_offset() + 4096);

            // Every region begins on a cache line. The cursors depend on it and
            // an `Sqe` is a line, so a misaligned array is a torn entry.
            for offset in [l.sqe_offset(), l.cqe_offset(), l.arena_offset()] {
                assert_eq!(offset % LINE, 0, "region at {offset} is not line-aligned for n={n}");
            }
        }
    }

    #[test]
    fn a_ring_size_that_is_not_a_power_of_two_is_refused() {
        assert!(Layout::new(0, 0).is_none());
        assert!(Layout::new(3, 0).is_none());
        assert!(Layout::new(MAX_ENTRIES * 2, 0).is_none());
        assert!(Layout::new(MAX_ENTRIES, 0).is_some());
    }

    #[test]
    fn an_arena_that_would_overflow_the_total_is_refused() {
        assert!(Layout::new(1024, u32::MAX).is_none());
    }

    #[test]
    fn a_described_layout_adopts_back_to_itself() {
        let l = Layout::new(64, 1024).unwrap();
        let header = l.describe(0, 0, 0);
        let adopted = Layout::adopt(&header, l.total()).expect("our own header");
        assert_eq!(adopted, l);
    }

    #[test]
    fn an_offset_the_peer_invented_is_refused() {
        let l = Layout::new(64, 1024).unwrap();

        // The shape of the attack this check exists for: a header that passes
        // structural validation and points the entry array at the cursors.
        let mut header = l.describe(0, 0, 0);
        header.sqe_offset = HEAD;
        assert!(Layout::adopt(&header, l.total()).is_err());

        let mut header = l.describe(0, 0, 0);
        header.cqe_offset = header.sqe_offset;
        assert!(Layout::adopt(&header, l.total()).is_err());
    }

    #[test]
    fn a_mapping_too_short_for_the_header_is_refused() {
        let l = Layout::new(64, 0).unwrap();
        let header = l.describe(0, 0, 0);
        assert!(Layout::adopt(&header, l.total() - 1).is_err());
        assert!(Layout::adopt(&header, l.total()).is_ok());
    }

    #[test]
    fn a_malformed_header_is_refused_before_the_arithmetic() {
        let l = Layout::new(64, 0).unwrap();
        let mut header = l.describe(0, 0, 0);
        header.magic = 0;
        let refusal = Layout::adopt(&header, l.total()).unwrap_err();
        assert_eq!(
            error::unpack(refusal),
            Some((error::ARGUMENT, error::argument::MALFORMED_HEADER))
        );
    }
}
