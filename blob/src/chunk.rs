// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Content-defined chunking: FastCDC's shape on a gear register, normalised.
//!
//! # The register, and what a boundary can see
//!
//! One word, one byte at a time: `h = (h << 1) + gear[b]`. Bit *k* of that
//! register is a function of the last *k+1* bytes and of nothing before them,
//! because the shift walks a byte's contribution one place towards the top and
//! off the end. So the window a boundary decision sees is set by the highest
//! bit the mask tests, and by nothing else.
//!
//! **The window is the last sixty-four bytes *because the highest mask bit is
//! 63*, and for no other reason.** That sentence is here because an earlier
//! draft claimed sixty-four for a mask over the low sixteen bits, which is a
//! sixteen-byte window and wrong by a factor of four — and a sixteen-byte
//! window is exactly what phase-locks boundaries on data with short periods.
//! `gear::spread` forces bit 63 and `gear`'s tests assert it.
//!
//! # Why normalised chunking
//!
//! Two masks rather than one: [`MASK_STRICT_BITS`] bits below
//! [`CHUNK_TARGET_BYTES`], so a cut before the target is unlikely, and
//! [`MASK_LOOSE_BITS`] bits at and above it, so a cut after the target is
//! likely. FastCDC's normalised chunking, and it is adopted here for one
//! reason: **it reduces the number of cuts forced at [`CHUNK_MAX_BYTES`]**.
//!
//! That is not a throughput argument, it is the bound's argument. A forced cut
//! is measured from the previous boundary rather than from the content, so two
//! streams offset by an insertion cannot resynchronise across a run of them —
//! which is the whole of the second clause of the re-chunking bound in
//! `crate`'s header. Fewer forced cuts is a smaller region where the bound is
//! weak.
//!
//! # What the parameters are, and where they are also written down
//!
//! Every constant below is a superblock field. A chunker that changed under a
//! mount decides every object hash and nothing on disk would say so, so a mount
//! whose compiled-in parameters disagree with the device's refuses rather than
//! re-chunking.

use crate::gear::{GEAR, MASK_LOOSE, MASK_STRICT};

/// The smallest chunk the content may choose.
///
/// Unit: bytes. Candidates below this are ignored — not deferred, ignored: the
/// register keeps running and the position is simply not a cut.
pub const CHUNK_MIN_BYTES: usize = 16 * 1024;

/// The size the mask widths are chosen around.
///
/// Unit: bytes. Below it the strict mask applies and above it the loose one, so
/// this is the point the distribution is normalised about rather than a size
/// any particular chunk has.
pub const CHUNK_TARGET_BYTES: usize = 64 * 1024;

/// The size at which a boundary is forced whatever the content says.
///
/// Unit: bytes. Four times the target, and the multiple is what bounds the
/// damage on content with no candidates: a chunker with no maximum would hash a
/// whole zero-filled disk image as one chunk.
pub const CHUNK_MAX_BYTES: usize = 256 * 1024;

/// Bits in the mask consulted below [`CHUNK_TARGET_BYTES`].
///
/// Unit: bits. Two above the target's own width, so a cut in the first quarter
/// of the target is four times less likely than a uniform chunker would make
/// it.
pub const MASK_STRICT_BITS: u32 = 18;

/// Bits in the mask consulted at and above [`CHUNK_TARGET_BYTES`].
///
/// Unit: bits. Two below the target's width, so a chunk that has run past the
/// target is cut four times sooner than a uniform chunker would cut it — which
/// is where the forced cuts are saved.
pub const MASK_LOOSE_BITS: u32 = 14;

/// How far past an edit the two boundary sequences are allowed to disagree.
///
/// Unit: bytes. The flat half of the two-clause bound `crate`'s header states;
/// the other half is the enclosing candidate-free run plus one chunk, and on
/// content with no candidates that half is the one that applies.
pub const RESYNC_BOUND_BYTES: usize = 512 * 1024;

/// The rolling register and the distance to the last boundary.
///
/// Two words. It holds no bytes and copies none: a chunk is hashed as it is
/// scanned, so nothing here ever has a chunk in hand.
#[derive(Clone, Debug, Default)]
pub struct Chunker {
    /// The gear register.
    register: u64,
    /// Bytes since the last boundary, in bytes.
    since_cut: usize,
}

impl Chunker {
    /// A chunker at the start of an object.
    #[must_use]
    pub const fn new() -> Self {
        Self { register: 0, since_cut: 0 }
    }

    /// Scan `bytes`, answering the offsets at which a chunk ends.
    ///
    /// The offsets are into `bytes` — the slice fed, not the object — and each
    /// one is one past the last byte of a chunk, so a caller that has fed `n`
    /// bytes so far adds `n` to place them.
    ///
    /// # Why an iterator and not a callback
    ///
    /// `lint-callbacks` says no interface in this tree registers one, and this
    /// is an interface. The rule is R05's and its reason is delivery: an
    /// event this system produces is drained at a polling point by the code
    /// that wanted it, never handed to a closure at a moment the producer
    /// chose.
    pub fn feed<'a>(&'a mut self, bytes: &'a [u8]) -> Cuts<'a> {
        Cuts { chunker: self, bytes, at: 0 }
    }

    /// The bytes since the last boundary, consuming the chunker.
    ///
    /// Unit: bytes. This is the final chunk of the object, which the content
    /// did not choose — the object ended.
    #[must_use]
    pub const fn finish(self) -> usize {
        self.since_cut
    }

    /// One byte. `true` when a chunk ends here.
    fn step(&mut self, byte: u8) -> bool {
        self.register = (self.register << 1).wrapping_add(GEAR[byte as usize]);
        self.since_cut += 1;

        if self.since_cut >= CHUNK_MIN_BYTES {
            let mask = if self.since_cut < CHUNK_TARGET_BYTES { MASK_STRICT } else { MASK_LOOSE };
            if self.register & mask == 0 {
                self.since_cut = 0;
                return true;
            }
        }
        // The forced cut, and the only place a boundary is not a statement
        // about content. Everything the bound is weak about is downstream of
        // this line.
        if self.since_cut == CHUNK_MAX_BYTES {
            self.since_cut = 0;
            return true;
        }
        false
    }
}

/// The cut offsets in one fed slice.
///
/// # Why this finishes the slice when it is dropped
///
/// The register is deliberately **not** reset at a boundary — that is what
/// makes a candidate a property of the content rather than of where the
/// previous cut landed — so the chunker's state after a slice must be a
/// function of every byte in it. An iterator a caller can abandon halfway would
/// make it a function of how much of the iterator was consumed instead, and the
/// object would chunk differently depending on the shape of the loop that read
/// it. So dropping this scans the remainder.
pub struct Cuts<'a> {
    chunker: &'a mut Chunker,
    bytes: &'a [u8],
    at: usize,
}

impl Iterator for Cuts<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            self.at += 1;
            if self.chunker.step(byte) {
                return Some(self.at);
            }
        }
        None
    }
}

impl Drop for Cuts<'_> {
    fn drop(&mut self) {
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            self.at += 1;
            let _ = self.chunker.step(byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CHUNK_MAX_BYTES, CHUNK_MIN_BYTES, Chunker};
    use alloc::vec;
    use alloc::vec::Vec;

    /// Zero-filled content has no candidates, so every cut is the forced one.
    ///
    /// `h = 2h + gear[0]` reaches a fixed point after sixty-four bytes, and
    /// that fixed point either hits a mask or it does not. It does not, so a
    /// zero run cuts at exactly [`CHUNK_MAX_BYTES`] and this is the workload
    /// the second clause of the bound is about. If this test ever goes red the
    /// gear table changed, and so did every object hash ever written.
    #[test]
    fn a_zero_run_is_cut_only_by_the_maximum() {
        let mut chunker = Chunker::new();
        let zeros = vec![0u8; 3 * CHUNK_MAX_BYTES];
        let cuts: Vec<usize> = chunker.feed(&zeros).collect();
        assert_eq!(cuts, vec![CHUNK_MAX_BYTES, 2 * CHUNK_MAX_BYTES, 3 * CHUNK_MAX_BYTES]);
    }

    /// The cuts do not depend on how the bytes arrived.
    ///
    /// The register is not reset at a boundary and the state must not be a
    /// function of the feed size either, which is the same obligation
    /// [`super::Cuts`]'s `Drop` discharges from the other side.
    #[test]
    fn feeding_in_pieces_cuts_where_feeding_at_once_cuts() {
        let mut counter: u64 = 0;
        let content: Vec<u8> = (0..CHUNK_MAX_BYTES * 2)
            .map(|_| {
                counter = counter.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                (counter >> 33) as u8
            })
            .collect();

        let mut whole = Chunker::new();
        let at_once: Vec<usize> = whole.feed(&content).collect();

        let mut pieced = Chunker::new();
        let mut in_pieces = Vec::new();
        let mut base = 0;
        for piece in content.chunks(CHUNK_MIN_BYTES / 3 + 7) {
            in_pieces.extend(pieced.feed(piece).map(|at| base + at));
            base += piece.len();
        }

        assert_eq!(at_once, in_pieces);
    }
}
