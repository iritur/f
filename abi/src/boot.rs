// SPDX-License-Identifier: Apache-2.0 OR MIT
//! How a booting machine is told which generation to be.
//!
//! # One token on a command line, and why that is the whole mechanism
//!
//! The frame already reads its options from the multiboot command line —
//! `docs/booting-on-hardware.md` passes `timer=60` exactly this way — so
//! selection needs no new boot-time code, no new device and, above all, no
//! second reader of the on-disk format. `f.root=<64 hex>` names a module by its
//! root hash; the default is the newest module the loader offered. On hardware
//! the menu is the loader's own, written by `cargo xtask generation --install`
//! into the GRUB fragment that document already describes.
//!
//! # Why the grammar is here rather than beside the parser that will use it
//!
//! Nothing parses this yet: the frame does at `E2-P07`. It is written now, in
//! this crate and in this commit, because the boot module `cargo xtask
//! generation` packs is named by the same hash this token names — and a grammar
//! written later by a different hand is the second reader of a format that the
//! spec spends a page refusing to have. One definition, in the crate whose
//! layout is already load-bearing against code we do not control.
//!
//! # What this is not
//!
//! It is not authentication. A root hash on a command line is a *selection*, and
//! anyone who can set the command line can select anything the loader offered.
//! RFC 0012 says what the attestation does and does not prove; this module only
//! says how the sixty-four characters are spelled.

use crate::error;

/// The token's key, including its `=`.
///
/// Namespaced with `f.` because a multiboot command line is shared with whatever
/// else a loader puts on it, and an unprefixed `root=` is a word half the world
/// already uses for something else.
/// Unit: none — a literal.
pub const KEY: &str = "f.root=";

/// How many hexadecimal characters a root is written as.
/// Unit: characters.
pub const DIGITS: usize = 64;

/// How many bytes those characters decode to.
/// Unit: bytes.
pub const ROOT_BYTES: usize = DIGITS / 2;

/// How wide the rendered token is: the key and its sixty-four digits.
/// Unit: bytes.
pub const TOKEN_BYTES: usize = KEY.len() + DIGITS;

/// A parsed selection token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The generation root the boot was asked for.
    /// Unit: bytes, exactly 32 of them — the SHA-256 the fold produced, in the
    /// order FIPS 180-4 produces it, which is the order the hexadecimal reads
    /// left to right.
    pub root: [u8; ROOT_BYTES],
}

impl Selection {
    /// Render the token a loader would be given.
    ///
    /// Lower case, and only lower case, because a hash that is printed one way
    /// and parsed another is a hash two people compare by eye and disagree
    /// about. [`Selection::parse`] refuses upper case for the same reason rather
    /// than accepting both — R04, and a canonical form is only canonical if
    /// there is one of it.
    ///
    /// It returns the token rather than filling a buffer, because this crate has
    /// no allocator and no formatter: a fixed-width array is the only shape a
    /// `no_std` caller can hold, and returning it keeps the caller from having to
    /// know the width to declare the buffer.
    #[must_use]
    pub fn render(&self) -> [u8; TOKEN_BYTES] {
        let mut out = [0u8; TOKEN_BYTES];
        out[..KEY.len()].copy_from_slice(KEY.as_bytes());
        for (n, byte) in self.root.iter().enumerate() {
            out[KEY.len() + 2 * n] = HEX[(byte >> 4) as usize];
            out[KEY.len() + 2 * n + 1] = HEX[(byte & 0xF) as usize];
        }
        out
    }

    /// Parse one command-line word.
    ///
    /// # Errors
    ///
    /// A packed [`error::ARGUMENT`]. `UNKNOWN_FLAG` when the word is not this
    /// key at all, so a caller scanning a command line can tell *not mine* from
    /// *mine and wrong*; `MALFORMED_HEADER` when it is this key with something
    /// other than sixty-four lower-case hexadecimal characters after it.
    ///
    /// A truncated hash is refused rather than left-padded and a long one rather
    /// than truncated, because either accommodation would let two different
    /// roots select one generation.
    pub fn parse(word: &[u8]) -> Result<Self, i32> {
        let Some(digits) = word.strip_prefix(KEY.as_bytes()) else {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        };
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        if digits.len() != DIGITS {
            return Err(bad);
        }
        let mut root = [0u8; ROOT_BYTES];
        let mut n = 0;
        while n < ROOT_BYTES {
            let high = nibble(digits[2 * n]).ok_or(bad)?;
            let low = nibble(digits[2 * n + 1]).ok_or(bad)?;
            root[n] = (high << 4) | low;
            n += 1;
        }
        Ok(Self { root })
    }
}

/// The lower-case alphabet a root is printed in.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// One lower-case hexadecimal character, or nothing.
const fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: [u8; ROOT_BYTES] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF, 0x0F, 0x1E, 0x2D, 0x3C, 0x4B, 0x5A, 0x69, 0x78, 0x87, 0x96, 0xA5, 0xB4, 0xC3, 0xD2,
        0xE1, 0xF0,
    ];

    #[test]
    fn a_token_round_trips_through_its_own_grammar() {
        let out = Selection { root: ROOT }.render();
        assert_eq!(&out[..KEY.len()], b"f.root=");
        assert_eq!(&out[KEY.len()..KEY.len() + 8], b"00112233");
        assert_eq!(Selection::parse(&out), Ok(Selection { root: ROOT }));
    }

    #[test]
    fn a_word_that_is_not_this_key_is_told_apart_from_one_that_is_wrong() {
        let unknown = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

        assert_eq!(Selection::parse(b"timer=60"), Err(unknown));
        assert_eq!(Selection::parse(b"root=00"), Err(unknown));
        assert_eq!(Selection::parse(b"f.root="), Err(bad));
    }

    #[test]
    fn a_root_that_is_not_sixty_four_lower_case_hex_digits_is_refused() {
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let good = Selection { root: ROOT }.render();

        // Upper case. Accepted by a lenient parser, and then two spellings
        // select one generation and a person comparing by eye is wrong.
        let mut shouted = good;
        shouted[KEY.len() + 10] = b'A';
        assert_eq!(Selection::parse(&shouted), Err(bad));

        // Short and long, neither padded nor truncated.
        assert_eq!(Selection::parse(&good[..good.len() - 1]), Err(bad));
        let mut long = [b'0'; TOKEN_BYTES + 1];
        long[..KEY.len()].copy_from_slice(KEY.as_bytes());
        assert_eq!(Selection::parse(&long), Err(bad));

        // A character outside the alphabet entirely.
        let mut junk = good;
        junk[KEY.len()] = b'g';
        assert_eq!(Selection::parse(&junk), Err(bad));
    }
}
