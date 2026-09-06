// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The gear table and the mask, derived rather than transcribed.
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

use crate::chunk::MASK_BITS;

/// The label the gear table is derived from.
///
/// Unit: none — a stable text identity, not a quantity. It is a superblock
/// field: see this module's header for what changing it costs.
pub const GEAR_LABEL: &str = "f-blob gear v1";

/// The label the mask is derived from.
///
/// Unit: none — a stable text identity, not a quantity. Separate from
/// [`GEAR_LABEL`] so that a future mask width can be chosen without disturbing
/// a table that every object hash on every device depends on.
///
/// **`v2` and not `v1`, and the version is the point.** RFC 0061 replaced two
/// masks selected by distance from the previous boundary with one, so the
/// derivation off this label changed: a different number of bits, drawn from a
/// different position in the stream. A derivation that moved under an unchanged
/// label is precisely the silent hash change this field is a superblock field
/// to prevent, so the label moved with it. `GEAR_LABEL` did not move, because
/// the table did not.
pub const MASK_LABEL: &str = "f-blob mask v2";

/// One 64-bit word per byte value, added into the rolling register.
///
/// Unit: none — an entry is a mixing constant, not a quantity.
pub const GEAR: [u64; 256] = gear_table();

/// The one mask a candidate is tested against.
///
/// Unit: none — a bit set. [`MASK_BITS`] bits, the highest at bit 63.
///
/// One and not two: RFC 0061 retired normalised chunking, because which of the
/// two masks applied was decided by the distance since the previous boundary,
/// and that dependence is the phase-lock `E2-P02` measured.
pub const MASK: u64 = mask();

/// The register's fixed point on a run of zero bytes.
///
/// Unit: none — a register value, not a quantity. `h = 2h + gear[0]` reaches
/// this after sixty-four bytes and stays there, so it is the value every long
/// zero run in every object presents to the mask.
///
/// It is a constant here because the mask's draw has an obligation towards it
/// — RFC 0061's constraint that the fixed point must not hit the mask — and an
/// obligation stated in prose beside a value nothing names is an obligation
/// nobody can check. The test at the foot of this module discharges it.
pub const ZERO_RUN_FIXED_POINT: u64 = 0x127e_e44b_ae55_2daa;

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

/// The mask, from the first draw off [`MASK_LABEL`].
///
/// # The constraint the draw had to satisfy, stated before it was drawn
///
/// RFC 0061 states it: [`ZERO_RUN_FIXED_POINT`] must **not** hit the mask, and
/// if the first draw under `"f-blob mask v2"` had hit it, the label would have
/// incremented until one did not. That makes it a derivation constraint and not
/// a constant fitted to a test, which is why it is written here and in the RFC
/// rather than discovered by a red assertion. The reason it matters: a mask the
/// fixed point hits would cut every zero region in every object in the system
/// at the same arithmetic offset — boundaries decided by the register's
/// arithmetic rather than by content, which is a worse property than a zero run
/// having no candidates at all.
///
/// The first draw does not hit it, so the label is `v2` and stays there.
const fn mask() -> u64 {
    let mut stream = Stream::from_seed(label(MASK_LABEL));
    spread(&mut stream, MASK_BITS)
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
    use super::{GEAR, MASK, MASK_BITS, ZERO_RUN_FIXED_POINT};
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
    fn the_highest_bit_of_the_mask_is_bit_sixty_three() {
        assert_eq!(MASK.leading_zeros(), 0, "the mask does not reach bit 63");
        assert_eq!(MASK.count_ones(), MASK_BITS);
    }

    /// The register's fixed point on a zero run is where the mask is not.
    ///
    /// RFC 0061 states this as a constraint on the draw rather than as a hope
    /// about it: if `"f-blob mask v2"`'s first draw had hit the fixed point,
    /// the label would have incremented. It did not, and this test is what says
    /// so to anyone who moves `MASK_BITS`, the mask label or the gear table —
    /// all three of which move the two sides of this comparison independently.
    ///
    /// What goes wrong if it ever goes red: every zero region in every object
    /// in the system acquires boundaries at an offset decided by the register's
    /// arithmetic, identical across all of them and unrelated to content. RFC
    /// 0061 lists that as a reversal condition and says the label increments
    /// before anything else is concluded.
    #[test]
    fn the_mask_does_not_hit_the_zero_run_fixed_point() {
        let mut register: u64 = 0;
        for _ in 0..128 {
            register = (register << 1).wrapping_add(GEAR[0]);
        }
        assert_eq!(
            register, ZERO_RUN_FIXED_POINT,
            "the gear table moved, so the fixed point RFC 0061 drew the mask against moved too"
        );
        assert_ne!(
            register & MASK,
            0,
            "the zero-run fixed point {register:#018x} hits the mask {MASK:#018x}, so zero-filled \
             content would cut at an arithmetic offset rather than not at all"
        );
    }
}
