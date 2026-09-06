// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Content-defined chunking: FastCDC's shape on a gear register, with
//! acceptance stated as a predicate over a window of content.
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
//! # Why acceptance is a window and not a recurrence — RFC 0061
//!
//! A *candidate* is `register & MASK == 0` and nothing else: one mask, no
//! second one selected by how far the scan has run. A candidate is *accepted*
//! when **no candidate occurred in the preceding [`CHUNK_MIN_BYTES`]**, which is
//! not the same rule as *at least [`CHUNK_MIN_BYTES`] since the last cut* even
//! though it delivers the same minimum chunk size. The difference is the whole
//! of RFC 0061 and it is one sentence: **a predicate over a window of `w` bytes
//! forgets everything more than `w` bytes back, and a recurrence over the
//! previous boundary need never forget anything.**
//!
//! The minimum still holds, in two lines. If `c` and `c'` are consecutive
//! accepted positions then `c'` has no candidate in `[c' − CHUNK_MIN_BYTES,
//! c')`, and `c` is a candidate at or before `c'`, so `c' − c` is at least
//! `CHUNK_MIN_BYTES`. And the whole acceptance decision at `p` reads the content
//! in `[p − CHUNK_MIN_BYTES − 64, p]` and nothing else, so two streams offset
//! by an insertion take the same decision at every position 16 448 bytes past
//! the edit.
//!
//! **What this replaced, and why it is not coming back.** Normalised chunking:
//! two masks, a strict one below [`CHUNK_TARGET_BYTES`] and a loose one at and
//! above it, adopted to make cuts forced at [`CHUNK_MAX_BYTES`] rarer. Which
//! mask applied was decided by *the distance since the previous boundary*, so
//! it was the same dependence RFC 0061 removes from acceptance; keeping it
//! would have left the phase-lock in the mask choice after removing it from the
//! acceptance test. `E2-P02` measured that phase-lock: on periodic content four
//! of eight seeds never resynchronised at all. If the forced-cut fraction turns
//! out to be the price, RFC 0061 names the repair and it is *not* this — it is
//! nested masks with the loose hit accepted only when no loose hit occurred in
//! the preceding [`CHUNK_TARGET_BYTES`], which is a window too.
//!
//! # What still reads the previous boundary, and why exactly one thing does
//!
//! The forced cut at [`CHUNK_MAX_BYTES`], and its companion: an accepted
//! candidate within [`CHUNK_MIN_BYTES`] of a boundary is suppressed, so the
//! minimum survives beside a forced cut. That suppression can only ever bite
//! after a *forced* boundary — after a content boundary the window rule has
//! already guaranteed the gap — so it reaches no further than the region where
//! every cut is forced anyway. It is granted because the alternative is an
//! unbounded chunk, and RFC 0061 forecloses every other rule that would like to
//! know where the last cut was.
//!
//! # What the parameters are, and where they are also written down
//!
//! Every constant below is a superblock field. A chunker that changed under a
//! mount decides every object hash and nothing on disk would say so, so a mount
//! whose compiled-in parameters disagree with the device's refuses rather than
//! re-chunking.

use crate::gear::{GEAR, MASK};

/// The smallest chunk the content may choose.
///
/// Unit: bytes. Also the width of the window acceptance is a predicate over: a
/// candidate is accepted when no candidate occurred in the preceding
/// `CHUNK_MIN_BYTES`. Candidates inside that window are ignored — not deferred,
/// ignored: the register keeps running and the position is simply not a cut.
pub const CHUNK_MIN_BYTES: usize = 16 * 1024;

/// The size the mask width is chosen around.
///
/// Unit: bytes. [`MASK_BITS`] is the width at which a candidate appears once
/// per this many bytes on uniform content, so this is the raw candidate
/// spacing rather than a size any particular chunk has — the minimum thins the
/// candidates and pushes the accepted spacing above it.
pub const CHUNK_TARGET_BYTES: usize = 64 * 1024;

/// The size at which a boundary is forced whatever the content says.
///
/// Unit: bytes. Four times the target, and the multiple is what bounds the
/// damage on content with no candidates: a chunker with no maximum would hash a
/// whole zero-filled disk image as one chunk.
pub const CHUNK_MAX_BYTES: usize = 256 * 1024;

/// Bits in the mask a candidate is tested against.
///
/// Unit: bits. Sixteen, the width at which the raw candidate spacing on uniform
/// content is [`CHUNK_TARGET_BYTES`] = 2^16 bytes. One width and not two: RFC
/// 0061 retired normalised chunking, whose two widths were selected by distance
/// from the previous boundary.
pub const MASK_BITS: u32 = 16;

/// How far past an edit the two boundary sequences are allowed to disagree.
///
/// Unit: bytes. The published outer allowance of the bound `crate`'s header
/// states. Outside a starved run the sequences in fact agree far sooner — from
/// `CHUNK_MIN_BYTES + 64` past the edit, which is 16 448 bytes — but this is the
/// number the claim is registered against, and RFC 0061 left it where it was on
/// purpose: a bound whose repair moved its own threshold would be a bound
/// fitted to its measurement.
pub const RESYNC_BOUND_BYTES: usize = 512 * 1024;

/// The rolling register, the distance to the last candidate and the distance to
/// the last boundary.
///
/// Three words. It holds no bytes and copies none: a chunk is hashed as it is
/// scanned, so nothing here ever has a chunk in hand. The third word is what
/// RFC 0061 cost — the window rule needs to know how long ago the last
/// *candidate* was, which the old rule never asked.
#[derive(Clone, Debug, Default)]
pub struct Chunker {
    /// The gear register.
    register: u64,
    /// Bytes since the last candidate or the start of the object, whichever is
    /// later, in bytes — saturating at [`CHUNK_MIN_BYTES`], because the rule
    /// only ever asks whether it has reached that and a saturating counter
    /// cannot wrap on a long object.
    ///
    /// The object's start counts as a candidate for this purpose. Not for
    /// tidiness: without it the first candidate in an object would be accepted
    /// wherever it fell and the object's first chunk could be a hundred bytes,
    /// which property 1 would catch and which no reader would expect.
    since_candidate: usize,
    /// Bytes since the last boundary, in bytes.
    since_cut: usize,
}

impl Chunker {
    /// A chunker at the start of an object.
    #[must_use]
    pub const fn new() -> Self {
        Self { register: 0, since_candidate: 0, since_cut: 0 }
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
        if self.since_candidate < CHUNK_MIN_BYTES {
            self.since_candidate += 1;
        }

        if self.register & MASK == 0 {
            // A candidate, and that is the whole of what the content says. The
            // two questions below are asked of the window and of the forced
            // cut; neither of them makes this position more or less a
            // candidate, which is why the counter is reset either way.
            let far_enough_from_the_last_candidate = self.since_candidate >= CHUNK_MIN_BYTES;
            // Only ever false after a *forced* boundary: after a content
            // boundary the line above has already guaranteed the gap. This is
            // the suppression RFC 0061 grants and the only surviving reference
            // to where the last cut was.
            let far_enough_from_the_last_boundary = self.since_cut >= CHUNK_MIN_BYTES;
            self.since_candidate = 0;
            if far_enough_from_the_last_candidate && far_enough_from_the_last_boundary {
                self.since_cut = 0;
                return true;
            }
        }
        // The forced cut, and the only place a boundary is not a statement
        // about content. Everything the bound is weak about is downstream of
        // this line, and RFC 0061 named the class exactly: a run in which no
        // two consecutive candidates are closer than
        // `CHUNK_MAX_BYTES - CHUNK_MIN_BYTES`, which is the only condition
        // under which this line can be reached at all.
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
    /// that fixed point either hits the mask or it does not. It does not — RFC
    /// 0061 made that a constraint on the mask's draw and `gear`'s tests assert
    /// it — so a zero run cuts at exactly [`CHUNK_MAX_BYTES`] and this is the
    /// workload the second clause of the bound is about. If this test ever goes
    /// red the gear table or the mask changed, and so did every object hash
    /// ever written.
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
