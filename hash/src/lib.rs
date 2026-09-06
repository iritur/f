// SPDX-License-Identifier: Apache-2.0 OR MIT
//! SHA-256, and it is the only one in the tree.
//!
//! # Why a crate for four hundred lines
//!
//! A release address, a blob's name, an object's name and a generation's root
//! are all *the same statement*: these bytes and no others. That statement is
//! only worth anything if there is one function behind it. Until this crate
//! existed there was one transcription in `xtask/src/pack.rs` and the store was
//! about to acquire a second — and two transcriptions of the same standard are
//! the worst shape this can take, because they agree on every input anybody
//! tries and the day they disagree is the day a machine refuses a package it
//! built itself. RFC 0012's *one identity* is a sentence about hashes; this
//! crate is what makes it true about code.
//!
//! *Reversal:* a second implementation is justified when it is a different
//! *function* — a hardware SHA-256 instruction, or a different digest
//! altogether behind a format version. Neither is a second transcription of
//! this one, and neither may be added without the published vectors below
//! moving with it.
//!
//! # Why the state is streaming and not a one-shot over a slice
//!
//! [`sha256`] is the convenient half and [`Sha256`] is the load-bearing half.
//! The chunker in `f-blob` hashes bytes as it scans them and never holds a
//! whole chunk, and the store hashes an object's content while writing it out a
//! block at a time; a one-shot over `&[u8]` would force both to buffer, which
//! for a 256 KiB chunk means a quarter megabyte of heap this system does not
//! have to spend. That is also why the padding is done inside the block buffer
//! rather than by growing a message: this crate has no allocator, is `no_std`,
//! and is expected to be linked into a component image where the heap arrives
//! later and belongs to `ring/`.
//!
//! # What it is not
//!
//! Not optimised. The largest single thing this hashes today is a kernel image,
//! once, in a command that also runs a compiler; the busiest is the chunker,
//! and the constant this crate would be tuned against does not exist until
//! claim 0017 measures one. Clever here would be a cost with no reader.
//!
//! Not a MAC, not a KDF, not a password hash. It names content. Anything that
//! needs a key needs a different construction and an RFC saying which.

#![no_std]

/// The block SHA-256 compresses, in bytes. FIPS 180-4 section 1.
const BLOCK_BYTES: usize = 64;

/// Where the length field starts in the final block, in bytes. The padding rule
/// is stated here once so that neither [`Sha256::update`] nor [`Sha256::finish`]
/// carries a bare `56`.
const LENGTH_OFFSET: usize = BLOCK_BYTES - 8;

/// The round constants: the first thirty-two bits of the fractional parts of
/// the cube roots of the first sixty-four primes. FIPS 180-4 section 4.2.2.
const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// The initial state: the first thirty-two bits of the fractional parts of the
/// square roots of the first eight primes. FIPS 180-4 section 5.3.3.
const H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// The SHA-256 of some bytes, in one call.
///
/// The same digest [`Sha256`] produces for the same bytes fed in any number of
/// pieces — which is a test in this crate rather than a promise here, because
/// the two paths are the same code only as long as nobody makes them differ.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut state = Sha256::new();
    state.update(bytes);
    state.finish()
}

/// A SHA-256 in progress: the chained state, the bytes not yet a whole block,
/// and how long the message is so far.
///
/// Fed by [`update`](Self::update) as many times as the caller likes and closed
/// once by [`finish`](Self::finish), which consumes it — a hasher that could be
/// finished twice would invite the reading that the second answer continues the
/// first, and it does not.
#[derive(Clone)]
pub struct Sha256 {
    /// The eight chaining words, `H0` until the first block is compressed.
    state: [u32; 8],
    /// Bytes seen since the last compression. Only `buffered` of them are live.
    block: [u8; BLOCK_BYTES],
    /// How many bytes of `block` are live, always below `BLOCK_BYTES`: a full
    /// block is compressed the moment it fills rather than held, so that
    /// `finish` never has two blocks to deal with.
    buffered: usize,
    /// The message length so far, in bytes. The standard's field is in *bits*
    /// and is 64 bits wide, so a message above 2^61 bytes is outside the format
    /// rather than a defect here; the shift below is wrapping to say so without
    /// a panic on an input no device in this system can present.
    length_bytes: u64,
}

impl Sha256 {
    /// A hasher over the empty message.
    #[must_use]
    pub const fn new() -> Self {
        Self { state: H0, block: [0; BLOCK_BYTES], buffered: 0, length_bytes: 0 }
    }

    /// Feed some more of the message.
    ///
    /// Chunk boundaries are not part of the message: `update(a); update(b)` is
    /// `update(a ++ b)`, which is the property the store depends on when it
    /// hashes an object it is writing out one block at a time.
    pub fn update(&mut self, bytes: &[u8]) {
        self.length_bytes = self.length_bytes.wrapping_add(bytes.len() as u64);

        let mut rest = bytes;

        // Finish the partial block first, if there is one and the caller
        // brought enough to fill it. Anything else would leave two places that
        // know how a block is assembled.
        if self.buffered > 0 {
            let want = BLOCK_BYTES - self.buffered;
            let take = want.min(rest.len());
            self.block[self.buffered..self.buffered + take].copy_from_slice(&rest[..take]);
            self.buffered += take;
            rest = &rest[take..];
            if self.buffered == BLOCK_BYTES {
                compress(&mut self.state, &self.block);
                self.buffered = 0;
            } else {
                // Everything the caller brought fits inside the partial block,
                // so there is nothing to compress and nothing to re-buffer.
                // Returning here rather than falling through is load-bearing:
                // the tail assignment below overwrites `buffered`, which for a
                // short update on a non-empty buffer would silently throw the
                // buffered bytes away and hash a shorter message than it
                // counted.
                return;
            }
        }

        // Whole blocks straight out of the caller's slice: no copy, which is
        // what makes hashing a large object cost one pass and not two.
        let (blocks, tail) = rest.as_chunks::<BLOCK_BYTES>();
        for block in blocks {
            compress(&mut self.state, block);
        }

        self.block[..tail.len()].copy_from_slice(tail);
        self.buffered = tail.len();
    }

    /// Close the message and return its digest.
    #[must_use]
    pub fn finish(mut self) -> [u8; 32] {
        // The padding, as the standard states it: a one bit, then zeroes, then
        // the length in bits as a big-endian u64, to a multiple of 64 bytes.
        // Done in place because there is no allocator to grow a message in.
        let bits = self.length_bytes.wrapping_mul(8);
        self.block[self.buffered..].fill(0);
        self.block[self.buffered] = 0x80;
        self.buffered += 1;

        // The one case worth naming: with the marker past the length field
        // there is no room left, so this block is compressed as padding alone
        // and the length goes in the next one. An implementation that is wrong
        // anywhere is usually wrong exactly here, which is why the tests walk
        // every length across the boundary rather than sampling.
        if self.buffered > LENGTH_OFFSET {
            compress(&mut self.state, &self.block);
            self.block.fill(0);
        }

        self.block[LENGTH_OFFSET..].copy_from_slice(&bits.to_be_bytes());
        compress(&mut self.state, &self.block);

        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// One block, compressed into the chaining state. FIPS 180-4 section 6.2.2.
///
/// The reference computation, transcribed and nothing more: the message
/// schedule expanded in place, sixty-four rounds, and the result added to the
/// state it started from.
fn compress(state: &mut [u32; 8], block: &[u8; BLOCK_BYTES]) {
    let mut w = [0u32; 64];
    for (word, bytes) in w.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*bytes);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);

        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *slot = slot.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A digest as the sixty-four characters everything else in the world
    /// prints, without a `String`: this crate has no allocator and its tests
    /// are not the place to acquire one. `xtask/src/pack.rs` keeps the `String`
    /// version, because a manifest is text.
    fn hex(digest: &[u8; 32]) -> [u8; 64] {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = [0u8; 64];
        for (pair, byte) in out.as_chunks_mut::<2>().0.iter_mut().zip(digest) {
            pair[0] = DIGITS[usize::from(byte >> 4)];
            pair[1] = DIGITS[usize::from(byte & 0xF)];
        }
        out
    }

    #[test]
    fn matches_the_vectors_the_standard_publishes() {
        // FIPS 180-4's own two examples, and the empty message. A hash
        // implementation checked only against itself is a hash function, just
        // not that one. These moved here from xtask/src/pack.rs with the code
        // they check, so that there is one place to look.
        assert_eq!(
            &hex(&sha256(b"abc")),
            b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            &hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            b"248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            &hex(&sha256(b"")),
            b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn the_million_character_vector_arrives_a_hundred_bytes_at_a_time() {
        // The standard's third example, and the only one long enough to be
        // interesting. It is fed in hundred-byte pieces rather than as one
        // slice for two reasons: this crate has no allocator to build a
        // megabyte in, and a piece length that divides neither the block nor
        // the total is exactly the case a buffered implementation gets wrong.
        let mut state = Sha256::new();
        for _ in 0..10_000 {
            state.update(&[b'a'; 100]);
        }
        assert_eq!(
            &hex(&state.finish()),
            b"cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn the_padding_changes_shape_and_the_digest_still_depends_on_the_length() {
        // 55, 56 and 64 bytes are where the padding changes shape: 56 is the
        // length that no longer leaves room for the length field, so it grows a
        // whole extra block.
        let zeros = [0u8; 130];
        for len in [0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 128] {
            // Not a known vector, but it must not panic and must depend on the
            // length: two different inputs hashing alike here would be a
            // padding bug rather than a collision.
            let a = sha256(&zeros[..len]);
            let b = sha256(&zeros[..len + 1]);
            assert_ne!(a, b, "inputs of {len} and {} zero bytes hashed alike", len + 1);
        }
    }

    #[test]
    fn a_stream_split_anywhere_is_the_message_it_spells() {
        // The property the store and the chunker rest on, and the reason this
        // crate is streaming at all: a blob store feeds a hasher whatever the
        // device handed it, which is a block, a chunk boundary or the ragged
        // end of a read — never the whole object. A state that is wrong is
        // wrong at a block boundary a one-shot test never sees, so this walks
        // *every* boundary rather than sampling three.
        //
        // 200 bytes: over three blocks, so a split can fall before, on and
        // after a compression, and inside the final block's length field.
        let mut message = [0u8; 200];
        for (i, byte) in message.iter_mut().enumerate() {
            // Not zeroes and not a period that divides 64: a message whose
            // bytes are all equal would hash the same under a state that
            // silently reordered its buffer.
            *byte = (i as u8).wrapping_mul(37).wrapping_add(11);
        }

        let want = sha256(&message);

        for split in 0..=message.len() {
            let mut state = Sha256::new();
            state.update(&message[..split]);
            state.update(&message[split..]);
            assert_eq!(state.finish(), want, "a message split at {split} hashed differently");
        }

        // And the awkward shapes a two-way split cannot express: one byte at a
        // time, and pieces that grow so no two consecutive updates share a
        // length or an alignment.
        let mut single = Sha256::new();
        for byte in &message {
            single.update(&[*byte]);
        }
        assert_eq!(single.finish(), want, "a byte-at-a-time stream hashed differently");

        let mut ragged = Sha256::new();
        let mut rest = &message[..];
        let mut piece = 1;
        while !rest.is_empty() {
            let take = piece.min(rest.len());
            ragged.update(&rest[..take]);
            rest = &rest[take..];
            piece += 1;
        }
        assert_eq!(ragged.finish(), want, "a ragged stream hashed differently");
    }

    #[test]
    fn an_empty_update_is_not_part_of_the_message() {
        // The degenerate case a caller reaches by accident: the last read
        // returned nothing, or a chunk ended exactly on a block. It must not
        // disturb the buffer, and it must not count towards the length.
        let mut state = Sha256::new();
        state.update(b"");
        state.update(b"ab");
        state.update(b"");
        state.update(b"c");
        state.update(b"");
        assert_eq!(state.finish(), sha256(b"abc"));
    }
}
