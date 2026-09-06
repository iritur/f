// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The gear table and the two masks, derived rather than transcribed.
//!
//! # Why there is no SplitMix64 in this file
//!
//! A 256-entry table of random-looking words is the kind of thing that gets
//! pasted into a source file, and once it is pasted nobody can check it against
//! anything. `env/src/split.rs` already carries a `const fn` SplitMix64 and
//! RFC 0026's whole argument is that there is **one** derivation in this tree.
//! `kernel/src/env.rs` is the second place that argument had to be made; a
//! third transcription here would be a second generator for a reviewer to check
//! against the paper, and the reviewer would have to check the table too.
//!
//! So the table is a `const fn` over `f_env::split::Stream`, evaluated at
//! compile time. What lands in the binary is the same 2 KiB either way; what
//! differs is that this version can be re-derived from two labels and eleven
//! lines.
//!
//! # Why the labels are on disk
//!
//! Changing either label changes every boundary, therefore every chunk, and
//! therefore every object hash in the system — silently, because a hash that
//! changed still looks like a hash. `gear_label` is a superblock field for that
//! reason, and a mount whose compiled-in label disagrees with the one on the
//! device refuses rather than re-chunking.

use f_env::split::{Stream, label};

use crate::chunk::{MASK_LOOSE_BITS, MASK_STRICT_BITS};

/// The label the gear table is derived from.
///
/// Unit: none — a stable text identity, not a quantity. It is a superblock
/// field: see this module's header for what changing it costs.
pub const GEAR_LABEL: &str = "f-blob gear v1";

/// The label the two masks are derived from.
///
/// Unit: none — a stable text identity, not a quantity. Separate from
/// [`GEAR_LABEL`] so that a future mask width can be chosen without disturbing
/// a table that every object hash on every device depends on.
pub const MASK_LABEL: &str = "f-blob mask v1";

/// One 64-bit word per byte value, added into the rolling register.
///
/// Unit: none — an entry is a mixing constant, not a quantity.
pub const GEAR: [u64; 256] = gear_table();

/// The mask consulted below [`crate::chunk::CHUNK_TARGET_BYTES`].
///
/// Unit: none — a bit set. [`MASK_STRICT_BITS`] bits, the highest at bit 63.
pub const MASK_STRICT: u64 = MASKS.0;

/// The mask consulted at and above [`crate::chunk::CHUNK_TARGET_BYTES`].
///
/// Unit: none — a bit set. [`MASK_LOOSE_BITS`] bits, the highest at bit 63.
pub const MASK_LOOSE: u64 = MASKS.1;

/// Both masks from one stream, in this order, so that the strict one does not
/// move when the loose one's width changes.
const MASKS: (u64, u64) = masks();

const fn gear_table() -> [u64; 256] {
    let mut stream = Stream::from_seed(label(GEAR_LABEL));
    let mut table = [0u64; 256];
    let mut index = 0;
    while index < 256 {
        table[index] = stream.next_u64();
        index += 1;
    }
    table
}

const fn masks() -> (u64, u64) {
    let mut stream = Stream::from_seed(label(MASK_LABEL));
    let strict = spread(&mut stream, MASK_STRICT_BITS);
    let loose = spread(&mut stream, MASK_LOOSE_BITS);
    (strict, loose)
}

/// `bits` set bits scattered through a word, the highest of them bit 63.
///
/// Bit 63 is forced rather than drawn, and it is the whole reason this function
/// is not `stream.next_u64() & something`. With `h = (h << 1) + gear[b]`, bit
/// *k* of the register is a function of the last *k+1* bytes; the window a
/// boundary decision sees is therefore set by the **highest** bit the mask
/// tests and by nothing else. A mask over the low sixteen bits gives a
/// sixteen-byte window, which is what phase-locks boundaries on data with short
/// periods, and an earlier draft of the spec claimed sixty-four for exactly
/// that mask.
///
/// The rest are drawn rather than placed at regular intervals: a regular
/// spacing is a structure in the mask, and a structure in the mask meets the
/// structure in the content.
const fn spread(stream: &mut Stream, bits: u32) -> u64 {
    let mut mask: u64 = 1 << 63;
    let mut set = 1;
    while set < bits {
        // 63 and not 64: bit 63 is already set, and drawing it again would
        // spin. The remainder's bias over a 63-way choice is not a property
        // this mask needs; reproducibility is.
        let position = (stream.next_u64() % 63) as u32;
        if mask & (1 << position) == 0 {
            mask |= 1 << position;
            set += 1;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::{GEAR, MASK_LOOSE, MASK_LOOSE_BITS, MASK_STRICT, MASK_STRICT_BITS};
    use alloc::collections::BTreeSet;

    /// A zero entry makes a byte value invisible to the register.
    ///
    /// `h = (h << 1) + 0` is `h << 1`: the byte contributes nothing, so a run
    /// of that value behaves like a shift register running down to a fixed
    /// point, and every object containing enough of it chunks the same way.
    /// Three properties are checked here rather than a 2 KiB literal being
    /// eyeballed, which is the argument for deriving the table at all.
    #[test]
    fn no_gear_entry_is_zero() {
        for (byte, entry) in GEAR.iter().enumerate() {
            assert_ne!(*entry, 0, "gear[{byte}] is zero, so byte {byte} is invisible");
        }
    }

    /// Two equal entries make two byte values interchangeable.
    ///
    /// Not fatal — the register would still be content-defined — but it is a
    /// collision the derivation is not supposed to produce, and finding one
    /// would mean the stream is not what this module thinks it is.
    #[test]
    fn no_two_gear_entries_are_equal() {
        let distinct: BTreeSet<u64> = GEAR.iter().copied().collect();
        assert_eq!(distinct.len(), GEAR.len(), "the gear table has a repeated entry");
    }

    /// The assertion that keeps the sixty-four-byte window from becoming a
    /// sixteen-byte one.
    ///
    /// This is the property the whole prefix half of the bound rests on, and it
    /// is one bit. An earlier draft of the spec stated a sixty-four-byte window
    /// over a mask on the low sixteen bits — wrong by a factor of four, in the
    /// crate doc, in E2-P02's specification and in a claim's workload
    /// description. A test is cheaper than a fourth reader.
    #[test]
    fn the_highest_bit_of_each_mask_is_bit_sixty_three() {
        assert_eq!(MASK_STRICT.leading_zeros(), 0, "the strict mask does not reach bit 63");
        assert_eq!(MASK_LOOSE.leading_zeros(), 0, "the loose mask does not reach bit 63");
        assert_eq!(MASK_STRICT.count_ones(), MASK_STRICT_BITS);
        assert_eq!(MASK_LOOSE.count_ones(), MASK_LOOSE_BITS);
    }
}
