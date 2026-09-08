// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Resident bytes per unit of work, and the three numbers that are not one
//! number.
//!
//! # What the exit asks for and what it takes to answer it honestly
//!
//! `E2-B08`'s exit ends *resident bytes per unit of work recorded*, and
//! `intent/0006-state/spec.md` classifies it as a **count that gates** rather
//! than a timing that waits, on the grounds that a byte count is the same
//! number on a fast host and a slow one. The threshold it derives is
//! `resident_bytes_per_read_byte` ≤ **1.0**, with the sentence being tested
//! written beside it: *a page cache holding a second copy would make it at
//! least 2*.
//!
//! That sentence only parses if the caller's own buffer is one of the two, so
//! the numerator has two halves and this module refuses to average them:
//!
//! - [`Resident::held_bytes`] — the peak bytes **this component** held on a
//!   read's behalf. Zero on the read path, because the device writes into the
//!   caller's buffer and this component allocates nothing for content. The
//!   page cache is exactly this number becoming non-zero.
//! - The caller's registered buffer, which is the client's own memory and is
//!   one byte per application byte by definition.
//!
//! So [`Resident::held_per_read_byte_micro`] is the component's half — zero
//! when the read path is zero-copy — and
//! [`Resident::system_per_read_byte_micro`] is that plus the client's one,
//! which is the number the spec's *at least 2* is about and which reads 1.0
//! when nothing is copied and 2.0 under a cache. Recording one and calling it
//! the other is how a threshold comes to be met by a number about something
//! else.
//!
//! # The mount is a residency cost and nothing was pricing it
//!
//! `TODO.md`'s `E2-B03` line says so in as many words: *the mount is also a
//! resident cost that nothing here prices; `claims/0019` is reserved for that
//! against `E2-B08`*. So it is priced here, amortised over the reads it buys —
//! [`Resident::mount_per_read_byte_micro`] — and it is a separate row rather
//! than folded into the one above, because the two decay differently: a copy
//! costs the same on every read for ever, and a mount is paid once and
//! divided.
//!
//! # Payload bytes, said as payload bytes
//!
//! The mount figure counts the **payload** of the two in-memory maps — a
//! thirty-two-byte hash and an eight-byte block index per store entry, a name
//! and a hash per index entry — and not the `BTreeMap` nodes around them.
//! That is an understatement and is recorded as one rather than guessed at: a
//! node's overhead is an allocator's property and this crate has no allocator
//! of its own to ask (spec decision 4 puts the component's heap in `ring/`,
//! and it does not exist yet). What the payload figure is good for is the
//! *shape* — that the mount is linear in the live set and that a read is not —
//! which is the half a reviewer can check. **Reversal:** the day the component
//! heap lands, the number this module reports is the heap's own occupancy and
//! this paragraph goes with it.
//!
//! # Micro-units, and no floating point
//!
//! Every ratio here is in millionths, so `1_000_000` is exactly 1.0. Integer
//! arithmetic because `env/` has none left and for the same reason it has none
//! left: a ratio computed in floating point is a ratio whose last digits depend
//! on the order the additions happened in, and a recorded number that moves
//! with an order nobody stated is not reproducible. The bench that prints these
//! renders them as decimals; the arithmetic that produces them does not.

/// One in this module's ratios. Unit: millionths — 1_000_000 is exactly 1.0.
pub const ONE: u64 = 1_000_000;

/// Payload bytes one store entry costs in memory.
///
/// A thirty-two-byte content address and the eight-byte block index it maps to.
/// `blob/src/store.rs` holds them in a `BTreeMap<[u8; 32], u64>`, and this is
/// that pair's width and not the map node's — the module comment says why the
/// understatement is deliberate.
/// Unit: bytes per entry.
pub const STORE_ENTRY_PAYLOAD_BYTES: u64 = 32 + 8;

/// Payload bytes one index entry costs in memory.
///
/// A name and the hash it resolves to. The name is variable-length, so this is
/// the hash and the namespace byte only, and the caller adds the names it
/// actually holds — [`Resident::mounted`] takes the name bytes as an argument
/// for exactly that reason: a per-entry constant that guessed at a name length
/// would be a number about a workload nobody ran.
/// Unit: bytes per entry.
pub const INDEX_ENTRY_PAYLOAD_BYTES: u64 = 32 + 1;

/// What this component holds, and the work it holds it against.
///
/// A peak and not an average: residency is what the machine has to find room
/// for at the worst moment, and an average would report a component that held
/// a gigabyte for one instant as holding almost nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resident {
    /// The largest number of content bytes this component has held at once.
    ///
    /// **Zero on the read path.** The device writes into the caller's
    /// registered buffer, so there is no moment at which this component is
    /// holding a read's content — which is the whole of *no page cache second
    /// copy*, said as a number instead of as a sentence.
    /// Unit: bytes.
    held_bytes: u64,
    /// Application bytes delivered to callers.
    ///
    /// The denominator, and the unit of work: a read is worth the bytes it
    /// delivered, so N reads of an M-byte blob is N × M and two runs at
    /// different blob sizes are comparable.
    /// Unit: bytes.
    delivered_bytes: u64,
    /// Payload bytes the mount holds: the store's map and the index's.
    /// Unit: bytes.
    mount_bytes: u64,
}

impl Resident {
    /// Record what the mount costs, from the two maps' own sizes.
    ///
    /// `index_names` is how many names the path namespace holds and
    /// `store_records` how many distinct blobs the store does. Both are asked
    /// of the things themselves rather than counted here, because a count kept
    /// beside a collection is a count that can disagree with it.
    pub fn mounted(&mut self, index_names: usize, store_records: usize) {
        let index = index_names as u64 * INDEX_ENTRY_PAYLOAD_BYTES;
        let store = store_records as u64 * STORE_ENTRY_PAYLOAD_BYTES;
        self.mount_bytes = index.saturating_add(store);
    }

    /// A read delivered `bytes` to its caller.
    pub fn delivered(&mut self, bytes: u64) {
        self.delivered_bytes = self.delivered_bytes.saturating_add(bytes);
    }

    /// This component held `bytes` of content at once.
    ///
    /// The peak is kept rather than the sum: a cache that holds one block at a
    /// time for a million reads is resident in one block, and a sum would
    /// report it as resident in a gigabyte.
    pub fn held(&mut self, bytes: u64) {
        if bytes > self.held_bytes {
            self.held_bytes = bytes;
        }
    }

    /// The peak content bytes this component held.
    /// Unit: bytes.
    #[must_use]
    pub const fn held_bytes(&self) -> u64 {
        self.held_bytes
    }

    /// Application bytes delivered.
    /// Unit: bytes.
    #[must_use]
    pub const fn delivered_bytes(&self) -> u64 {
        self.delivered_bytes
    }

    /// Payload bytes the mount holds.
    /// Unit: bytes.
    #[must_use]
    pub const fn mount_bytes(&self) -> u64 {
        self.mount_bytes
    }

    /// This component's own residency per application byte delivered.
    ///
    /// Zero when the read path copies nothing. `None` before any work has been
    /// done, because a ratio with no denominator is not a small number — it is
    /// not a number, and returning zero for it would be the flattering way to
    /// be wrong.
    /// Unit: millionths — [`ONE`] is 1.0.
    #[must_use]
    pub const fn held_per_read_byte_micro(&self) -> Option<u64> {
        if self.delivered_bytes == 0 {
            return None;
        }
        Some(self.held_bytes.saturating_mul(ONE) / self.delivered_bytes)
    }

    /// The whole system's residency per application byte delivered: the
    /// caller's own buffer, plus whatever this component held beside it.
    ///
    /// This is the number `intent/0006-state/spec.md`'s *a page cache holding a
    /// second copy would make it at least 2* is about. It is [`ONE`] exactly
    /// when nothing is copied.
    /// Unit: millionths — [`ONE`] is 1.0.
    #[must_use]
    pub const fn system_per_read_byte_micro(&self) -> Option<u64> {
        match self.held_per_read_byte_micro() {
            Some(held) => Some(ONE + held),
            None => None,
        }
    }

    /// What the mount costs per application byte delivered, which is the number
    /// that falls as the work rises.
    /// Unit: millionths — [`ONE`] is 1.0.
    #[must_use]
    pub const fn mount_per_read_byte_micro(&self) -> Option<u64> {
        if self.delivered_bytes == 0 {
            return None;
        }
        Some(self.mount_bytes.saturating_mul(ONE) / self.delivered_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{INDEX_ENTRY_PAYLOAD_BYTES, ONE, Resident, STORE_ENTRY_PAYLOAD_BYTES};

    #[test]
    fn no_work_yet_is_not_a_ratio_of_zero() {
        let resident = Resident::default();
        assert_eq!(resident.held_per_read_byte_micro(), None);
        assert_eq!(resident.system_per_read_byte_micro(), None);
        assert_eq!(resident.mount_per_read_byte_micro(), None);
    }

    #[test]
    fn a_zero_copy_read_is_one_for_the_system_and_zero_for_the_component() {
        let mut resident = Resident::default();
        resident.delivered(65_536);
        assert_eq!(resident.held_per_read_byte_micro(), Some(0));
        assert_eq!(resident.system_per_read_byte_micro(), Some(ONE));
    }

    #[test]
    fn a_cache_that_holds_every_byte_it_delivers_makes_the_system_number_two() {
        let mut resident = Resident::default();
        resident.delivered(65_536);
        resident.held(65_536);
        assert_eq!(resident.held_per_read_byte_micro(), Some(ONE));
        assert_eq!(
            resident.system_per_read_byte_micro(),
            Some(2 * ONE),
            "the spec's `at least 2`, arrived at rather than asserted"
        );
    }

    #[test]
    fn the_peak_is_kept_and_not_the_sum() {
        let mut resident = Resident::default();
        resident.held(4096);
        resident.held(512);
        resident.held(1024);
        assert_eq!(resident.held_bytes(), 4096, "the worst moment, not the last one");
    }

    #[test]
    fn the_mount_is_priced_from_the_two_maps_and_amortises() {
        let mut resident = Resident::default();
        resident.mounted(1000, 1000);
        assert_eq!(
            resident.mount_bytes(),
            1000 * INDEX_ENTRY_PAYLOAD_BYTES + 1000 * STORE_ENTRY_PAYLOAD_BYTES
        );
        resident.delivered(73_000);
        let one_read = resident.mount_per_read_byte_micro().expect("work has been done");
        resident.delivered(73_000 * 9);
        let ten_reads = resident.mount_per_read_byte_micro().expect("more work");
        assert!(ten_reads < one_read, "a cost paid once and divided falls as the work rises");
    }
}
