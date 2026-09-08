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
//! Nothing parsed the token when it was written: the frame does at `E2-P07`. It
//! was written anyway, in this crate, because the boot module `cargo xtask
//! generation` packs is named by the same hash this token names — and a grammar
//! written later by a different hand is the second reader of a format that the
//! spec spends a page refusing to have. One definition, in the crate whose
//! layout is already load-bearing against code we do not control.
//!
//! [`Module`] arrived here on that argument being cashed rather than restated.
//! Its layout lived in `xtask/src/generation.rs` while `xtask` was the only
//! thing that knew it, with a comment naming the day it should move: the day
//! something read it. `E2-B05`'s assembler is that reader, so the writer in
//! `xtask` and the reader in `f-assembler` now share this one definition
//! instead of two that agree until they do not.
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

    /// Find this token on a whole command line, if it is there.
    ///
    /// `None` when the line carries no word beginning with [`KEY`]; otherwise
    /// whatever [`Selection::parse`] made of that word, so that *absent* and
    /// *present and wrong* stay two different answers all the way up to the
    /// caller. A boot with no `f.root=` is an ordinary boot; a boot with a
    /// malformed one is a boot that was asked for a generation and cannot say
    /// which, and those must not both come back as `None`.
    ///
    /// Words are separated by ASCII whitespace, which is what a multiboot
    /// command line is: `timer=60 f.root=<64 hex>` is two words and the frame
    /// reads each with the grammar that owns it. The **first** matching word
    /// wins and the rest are not examined — a line naming two roots is a line
    /// whose author has two beliefs about one machine, and quietly taking the
    /// last would make which one runs a property of how the loader concatenates
    /// its arguments.
    ///
    /// It lives here rather than beside the parser in `kernel/` for the reason
    /// the module header gives about the grammar: separating a word from a line
    /// *is* part of how the sixty-four characters are spelled, and a second hand
    /// writing that later is the second reader this arrangement refuses to have.
    /// The scan is [`token`], shared with [`Declaration::find`] so that the two
    /// halves of one statement cannot come to disagree about where a word ends
    /// or which of two wins.
    #[must_use]
    pub fn find(cmdline: &[u8]) -> Option<Result<Self, i32>> {
        token(cmdline, KEY).map(Self::parse)
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
        Ok(Self { root: digest(digits)? })
    }
}

/// The first word of a command line that begins with `key`, or nothing.
///
/// Shared by both tokens for [`digest`]'s reason one level up: separating a word
/// from a line is part of how a token is spelled, and two scans are two chances
/// to disagree about it. There *were* two. `kernel/src/measure.rs` carried its
/// own — it split on `b' '` alone and assigned unconditionally, so the last
/// match won — while `kernel/src/generation.rs` called [`Selection::find`],
/// which splits on whitespace and takes the first. On a line carrying a tab or a
/// second `f.root=` the frame therefore attested to one root and validated
/// another. The loop was deleted rather than corrected beside this one: a
/// grammar with two implementations is the arrangement this module exists not to
/// have, and correcting the copy would have left the second reader in place.
///
/// Words are separated by ASCII whitespace, which is what a multiboot command
/// line is. The **first** matching word wins and the rest are not examined — a
/// line naming two roots is a line whose author has two beliefs about one
/// machine, and quietly taking the last would make which one runs a property of
/// how the loader concatenated its arguments.
fn token<'a>(cmdline: &'a [u8], key: &str) -> Option<&'a [u8]> {
    cmdline.split(|byte| byte.is_ascii_whitespace()).find(|word| word.starts_with(key.as_bytes()))
}

/// Sixty-four lower-case hexadecimal characters, or a refusal.
///
/// Shared by both tokens because there is one spelling of a digest on a command
/// line and two parsers would be two chances to disagree about it — the same
/// argument this module's own header makes about the grammar living beside the
/// format rather than beside whichever parser was written first. It is the
/// *spelling* that is shared and not the meaning: the two callers wrap it in two
/// types, so nothing downstream can hold one where the other belongs.
fn digest(digits: &[u8]) -> Result<[u8; ROOT_BYTES], i32> {
    let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
    if digits.len() != DIGITS {
        return Err(bad);
    }
    let mut out = [0u8; ROOT_BYTES];
    let mut n = 0;
    while n < ROOT_BYTES {
        let high = nibble(digits[2 * n]).ok_or(bad)?;
        let low = nibble(digits[2 * n + 1]).ok_or(bad)?;
        out[n] = (high << 4) | low;
        n += 1;
    }
    Ok(out)
}

/// The second token's key, including its `=`.
///
/// Namespaced for [`KEY`]'s reason, and separate from it for a reason of its
/// own: a selection and a declaration are two different statements. `f.root=`
/// says *be this generation*; `f.frame=` says *and this is the frame hash that
/// generation was compiled against*. RFC 0012 puts the second in the root
/// record's `frame` field so that a swap can decide whether it needs a reboot by
/// comparing thirty-two bytes without resolving a tree; on the boot path there
/// is no record to read it out of yet, so the loader carries it beside the
/// selection and the frame compares it against what it measured of itself.
///
/// *Reversal:* a frame that mounts a store on its boot path. Then the field is
/// read out of the root record the way RFC 0012 describes, this token becomes
/// the way a machine with no store is told what it is, and the day nothing needs
/// that is the day this key goes.
/// Unit: none — a literal.
pub const FRAME_KEY: &str = "f.frame=";

/// How wide the rendered declaration is: the key and its sixty-four digits.
/// Unit: bytes.
pub const FRAME_TOKEN_BYTES: usize = FRAME_KEY.len() + DIGITS;

/// The frame hash a generation declares for the image it was compiled against.
///
/// # Why this is not a `Selection` with a different key
///
/// Because the two would then be one type whose meaning depended on a string,
/// and the one mistake worth designing against here is a caller that compares a
/// root against a frame hash and finds them equal at a length rather than at a
/// meaning. Two types cost twenty lines and make that a compile error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The frame hash the generation carries for its frame leaf.
    /// Unit: bytes, exactly 32 of them — the SHA-256 RFC 0012 defines over
    /// `__text_start .. __text_end` then `__rodata_start .. __rodata_end` of the
    /// linked kernel image, in the order FIPS 180-4 produces it.
    pub frame: [u8; ROOT_BYTES],
}

impl Declaration {
    /// Render the token a loader would be given.
    ///
    /// Lower case only, for [`Selection::render`]'s reason and by the same
    /// arithmetic.
    #[must_use]
    pub fn render(&self) -> [u8; FRAME_TOKEN_BYTES] {
        let mut out = [0u8; FRAME_TOKEN_BYTES];
        out[..FRAME_KEY.len()].copy_from_slice(FRAME_KEY.as_bytes());
        for (n, byte) in self.frame.iter().enumerate() {
            out[FRAME_KEY.len() + 2 * n] = HEX[(byte >> 4) as usize];
            out[FRAME_KEY.len() + 2 * n + 1] = HEX[(byte & 0xF) as usize];
        }
        out
    }

    /// Find this token on a whole command line, if it is there.
    ///
    /// [`Selection::find`]'s answers and [`Selection::find`]'s rules, over
    /// [`FRAME_KEY`]: absent and present-and-wrong stay two different answers,
    /// words are separated by ASCII whitespace, and the first match wins.
    ///
    /// It exists because the frame reads *both* halves of one statement — RFC
    /// 0012 puts `f.frame=` beside `f.root=` and `kernel/src/measure.rs`
    /// refuses half of it — and a frame holding the shared scan for one half and
    /// a loop of its own for the other would tie-break the two halves
    /// differently. That is what it did: it split on `b' '` alone and let the
    /// *last* match win, so one tab, or one line naming two roots, made the
    /// frame attest to one root and validate another.
    ///
    /// *Reversal:* a command line that is no longer whitespace-separated words.
    #[must_use]
    pub fn find(cmdline: &[u8]) -> Option<Result<Self, i32>> {
        token(cmdline, FRAME_KEY).map(Self::parse)
    }

    /// Parse one command-line word.
    ///
    /// # Errors
    ///
    /// The same two as [`Selection::parse`], for the same reasons: `UNKNOWN_FLAG`
    /// for *not mine*, `MALFORMED_HEADER` for *mine and wrong*.
    pub fn parse(word: &[u8]) -> Result<Self, i32> {
        let Some(digits) = word.strip_prefix(FRAME_KEY.as_bytes()) else {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        };
        Ok(Self { frame: digest(digits)? })
    }
}

/// The magic a boot module begins with. `F_MOD`, and a schema in the low half.
/// Unit: none — a fixed byte pattern.
pub const MODULE_MAGIC: u64 = 0x465f_4d4f_4400_0001;

/// How many bytes of head a module has before its per-file lengths.
///
/// The magic, the record tree's length and the file count.
/// Unit: bytes.
pub const MODULE_HEAD_BYTES: usize = 8 + 4 + 4;

/// The most component files one boot module may carry.
///
/// [`crate::manifest::CAPABILITIES_MAX`]'s reason, one level up: the whole
/// module is decoded without an allocator, and a decoder with no bound is a
/// decoder that trusts a length field somebody else wrote. Sixty-four is
/// `f_abi::store::MEMBERS_MAX`, because the files are the tree's members and a
/// module with more files than the tree has members is a module the fold cannot
/// cover.
/// Unit: files.
pub const MODULE_FILES_MAX: usize = crate::store::MEMBERS_MAX;

/// The boot module a generation travels in.
///
/// # Why a module at all, and why the layout is here
///
/// At boot the store is not running, so an assembler holding a root hash has
/// nothing to read. The answer is not a loader that understands the on-disk
/// format — that is a second implementation of the format, outside the tree and
/// outside the claim, and on hardware it would be GRUB, which is GPLv3 and
/// would cross the licence boundary this epoch does not cross. So the record
/// tree and the component files it names travel together as one multiboot
/// module, RFC 0030's component-file shape one level up, and the assembler
/// recomputes the fold over what it was handed.
///
/// The writer of this layout is `xtask/src/generation.rs` and its first reader
/// is `f-assembler`. That is exactly the moment its own doc comment named for
/// moving it here: a format with a writer in `xtask` and a reader above the
/// frame, described in two places, is the two-readers problem this whole
/// arrangement exists to not have.
///
/// # The layout, and what it deliberately is not
///
/// ```text
/// 0   u64  MODULE_MAGIC
/// 8   u32  the record tree's length in bytes
/// 12  u32  how many component files follow
/// 16  u32 x files   each file's length in bytes
///     the record tree
///     each component file, in the tree's canonical member order
/// ```
///
/// Little-endian, fixed-width, no offsets and no names: the names are in the
/// tree and a second copy of a name is a second thing to disagree about. The
/// order is the tree's own canonical order, which is why the module needs no
/// index — the *n*th file is the *n*th member.
#[derive(Clone, Copy, Debug)]
pub struct Module<'a> {
    /// The whole module, as the loader placed it.
    bytes: &'a [u8],
    /// The record tree's length. Unit: bytes.
    tree_bytes: usize,
    /// How many component files follow. Unit: files.
    files: usize,
}

impl<'a> Module<'a> {
    /// Believe a module, or say why not.
    ///
    /// Every offset below is a function of a count that has already been
    /// believed, and the total length is checked **exactly**: a trailing byte is
    /// a byte no fold covers, and a byte no fold covers is a place two modules
    /// differ while naming one generation.
    ///
    /// # Errors
    ///
    /// A packed [`error::ARGUMENT`]: `MALFORMED_HEADER` for a magic or a length
    /// this build does not believe, `UNKNOWN_FLAG` for a file count past
    /// [`MODULE_FILES_MAX`].
    pub fn read(bytes: &'a [u8]) -> Result<Self, i32> {
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let head = bytes.get(..MODULE_HEAD_BYTES).ok_or(bad)?;
        let magic = u64::from_le_bytes([
            head[0], head[1], head[2], head[3], head[4], head[5], head[6], head[7],
        ]);
        if magic != MODULE_MAGIC {
            return Err(bad);
        }
        let tree_bytes = u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize;
        let files = u32::from_le_bytes([head[12], head[13], head[14], head[15]]) as usize;
        if files > MODULE_FILES_MAX {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }

        let lengths_at = MODULE_HEAD_BYTES;
        let tree_at = lengths_at.checked_add(files.checked_mul(4).ok_or(bad)?).ok_or(bad)?;
        let files_at = tree_at.checked_add(tree_bytes).ok_or(bad)?;
        let module = Self { bytes, tree_bytes, files };

        let mut total = files_at;
        let mut index = 0;
        while index < files {
            total = total.checked_add(module.file_bytes(index).ok_or(bad)?).ok_or(bad)?;
            index += 1;
        }
        if bytes.len() != total {
            return Err(bad);
        }
        Ok(module)
    }

    /// How many component files this module carries.
    /// Unit: files.
    #[must_use]
    pub const fn files(&self) -> usize {
        self.files
    }

    /// The record tree, for [`crate::store`]'s codec and `f-generation`'s
    /// checker to walk. Nothing here decodes it: this type knows where the tree
    /// is and deliberately not what is in it.
    #[must_use]
    pub fn tree(&self) -> &'a [u8] {
        let at = MODULE_HEAD_BYTES + self.files * 4;
        self.bytes.get(at..at + self.tree_bytes).unwrap_or(&[])
    }

    /// The *n*th component file, in the tree's canonical member order.
    ///
    /// `None` past the count. There is no name here and no search: the module
    /// carries no index because the tree already is one, and a second ordering
    /// would be a second thing to disagree about.
    #[must_use]
    pub fn file(&self, index: usize) -> Option<&'a [u8]> {
        if index >= self.files {
            return None;
        }
        let mut at = MODULE_HEAD_BYTES + self.files * 4 + self.tree_bytes;
        let mut n = 0;
        while n < index {
            at = at.checked_add(self.file_bytes(n)?)?;
            n += 1;
        }
        self.bytes.get(at..at.checked_add(self.file_bytes(index)?)?)
    }

    /// The declared length of one file, read out of the length array.
    fn file_bytes(&self, index: usize) -> Option<usize> {
        let at = MODULE_HEAD_BYTES + index * 4;
        let raw = self.bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize)
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

    /// A frame hash a test can tell apart from [`ROOT`] at a glance.
    const FRAME: [u8; ROOT_BYTES] = [0x5A; ROOT_BYTES];

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
    fn a_command_line_with_no_token_is_told_apart_from_one_with_a_bad_token() {
        let good = Selection { root: ROOT }.render();
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

        // Absent. An ordinary boot, and not a boot that asked for something.
        assert_eq!(Selection::find(b"timer=60"), None);
        assert_eq!(Selection::find(b""), None);

        // Present, among other words, with something on each side of it.
        let mut line = [b' '; SCRATCH];
        line[..9].copy_from_slice(b"timer=60 ");
        line[9..9 + TOKEN_BYTES].copy_from_slice(&good);
        line[9 + TOKEN_BYTES + 1..9 + TOKEN_BYTES + 9].copy_from_slice(b"boottime");
        assert_eq!(Selection::find(&line), Some(Ok(Selection { root: ROOT })));

        // Present and wrong. This is the case that must not come back as
        // `None`: the machine was asked for a generation and cannot say which.
        assert_eq!(Selection::find(b"timer=60 f.root=00 boottime"), Some(Err(bad)));

        // A word that merely contains the key is not the key: the separator is
        // whitespace, so `xf.root=...` is one word and not this one.
        let mut glued = [b'x'; TOKEN_BYTES + 1];
        glued[1..].copy_from_slice(&good);
        assert_eq!(Selection::find(&glued), None);
    }

    #[test]
    fn the_first_of_two_roots_wins_rather_than_the_last() {
        // A line naming two roots has an author with two beliefs about one
        // machine. Taking the last would make which one runs a property of how
        // the loader concatenated its arguments, which is not a property of
        // anything anybody wrote down.
        let first = Selection { root: ROOT }.render();
        let second = Selection { root: [0x5A; ROOT_BYTES] }.render();
        let mut line = [b' '; 2 * TOKEN_BYTES + 1];
        line[..TOKEN_BYTES].copy_from_slice(&first);
        line[TOKEN_BYTES + 1..].copy_from_slice(&second);
        assert_eq!(Selection::find(&line), Some(Ok(Selection { root: ROOT })));
    }

    #[test]
    fn a_declaration_is_found_by_the_same_scan_and_not_by_a_second_one() {
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let declared = Declaration { frame: FRAME }.render();

        // Absent, and absent from a line that carries the *other* token: the two
        // keys are read apart, so a line naming only a generation must not
        // answer the question about the frame it was compiled against.
        assert_eq!(Declaration::find(b"timer=60"), None);
        assert_eq!(Declaration::find(&Selection { root: ROOT }.render()), None);

        let mut line = [b' '; 9 + FRAME_TOKEN_BYTES];
        line[..9].copy_from_slice(b"timer=60 ");
        line[9..].copy_from_slice(&declared);
        assert_eq!(Declaration::find(&line), Some(Ok(Declaration { frame: FRAME })));

        // Present and wrong, which must not come back as `None` for
        // `Selection::find`'s reason: half a statement is a different statement
        // and not a weaker one.
        assert_eq!(Declaration::find(b"f.frame=00 timer=60"), Some(Err(bad)));
    }

    #[test]
    fn the_first_of_two_frame_hashes_wins_rather_than_the_last() {
        // The selection's rule, asserted of the other half of the statement:
        // one reader that took the first and one that took the last would make
        // the frame attest to one thing and validate another.
        let first = Declaration { frame: FRAME }.render();
        let second = Declaration { frame: [0x11; ROOT_BYTES] }.render();
        let mut line = [b' '; 2 * FRAME_TOKEN_BYTES + 1];
        line[..FRAME_TOKEN_BYTES].copy_from_slice(&first);
        line[FRAME_TOKEN_BYTES + 1..].copy_from_slice(&second);
        assert_eq!(Declaration::find(&line), Some(Ok(Declaration { frame: FRAME })));
    }

    #[test]
    fn a_tab_separates_two_words_the_way_a_space_does() {
        // The case the frame's own scan got wrong before it was deleted: it
        // split on `b' '` alone, so a tab glued the two tokens into one word and
        // neither was found — on a command line where `Selection::find`, one
        // file away, found both. A multiboot command line is whitespace, and a
        // tab is whitespace.
        let mut line = [b'\t'; TOKEN_BYTES + 1 + FRAME_TOKEN_BYTES];
        line[..TOKEN_BYTES].copy_from_slice(&Selection { root: ROOT }.render());
        line[TOKEN_BYTES + 1..].copy_from_slice(&Declaration { frame: FRAME }.render());
        assert_eq!(Selection::find(&line), Some(Ok(Selection { root: ROOT })));
        assert_eq!(Declaration::find(&line), Some(Ok(Declaration { frame: FRAME })));
    }

    /// How wide the scratch buffer a test module is written into is.
    /// Unit: bytes.
    const SCRATCH: usize = 128;

    /// A module carrying files of the given lengths and a tree of `tree` bytes,
    /// filled with bytes a reader can tell apart. Returns how much of `out` it
    /// used, because this crate has no allocator and a fixed buffer with a
    /// length is the shape a `no_std` test can hold.
    fn module(out: &mut [u8; SCRATCH], tree: usize, files: &[usize]) -> usize {
        let mut at = 0;
        let mut put = |bytes: &[u8]| {
            out[at..at + bytes.len()].copy_from_slice(bytes);
            at += bytes.len();
        };
        put(&MODULE_MAGIC.to_le_bytes());
        put(&(tree as u32).to_le_bytes());
        put(&(files.len() as u32).to_le_bytes());
        for length in files {
            put(&(*length as u32).to_le_bytes());
        }
        for _ in 0..tree {
            put(&[0xAA]);
        }
        for (n, length) in files.iter().enumerate() {
            for _ in 0..*length {
                put(&[n as u8]);
            }
        }
        at
    }

    #[test]
    fn a_module_hands_back_the_tree_and_each_file_in_member_order() {
        let mut scratch = [0u8; SCRATCH];
        let used = module(&mut scratch, 40, &[3, 5, 7]);
        let bytes = &scratch[..used];
        let read = Module::read(bytes).expect("a module this function wrote");

        assert_eq!(read.files(), 3);
        assert_eq!(read.tree().len(), 40);
        assert!(read.tree().iter().all(|b| *b == 0xAA));
        // The nth file is the nth member: there is no index in the module and
        // this is the property that lets there not be one.
        assert_eq!(read.file(0), Some([0u8; 3].as_slice()));
        assert_eq!(read.file(1), Some([1u8; 5].as_slice()));
        assert_eq!(read.file(2), Some([2u8; 7].as_slice()));
        assert_eq!(read.file(3), None);
    }

    #[test]
    fn a_module_with_a_trailing_byte_is_refused_rather_than_ignored() {
        // A byte no fold covers is a place two modules differ while naming one
        // generation, so the length is exact and not a minimum.
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let mut scratch = [0u8; SCRATCH];
        let used = module(&mut scratch, 8, &[4]);

        assert_eq!(Module::read(&scratch[..used + 1]).map(|m| m.files()), Err(bad));
        assert_eq!(Module::read(&scratch[..used - 1]).map(|m| m.files()), Err(bad));
        assert_eq!(Module::read(&scratch[..used]).map(|m| m.files()), Ok(1));
    }

    #[test]
    fn a_module_whose_head_is_not_this_format_is_refused() {
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let mut scratch = [0u8; SCRATCH];
        let used = module(&mut scratch, 8, &[4]);
        scratch[0] ^= 0xFF;
        assert_eq!(Module::read(&scratch[..used]).map(|m| m.files()), Err(bad));
        assert_eq!(Module::read(&[]).map(|m| m.files()), Err(bad));

        // A file count past the bound is refused before any offset is computed
        // from it, which is the whole reason the bound is in this crate.
        let unknown = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);
        let mut vast = [0u8; SCRATCH];
        let used = module(&mut vast, 0, &[]);
        vast[12..16].copy_from_slice(&((MODULE_FILES_MAX + 1) as u32).to_le_bytes());
        assert_eq!(Module::read(&vast[..used]).map(|m| m.files()), Err(unknown));
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

    #[test]
    fn a_declaration_round_trips_and_is_told_apart_from_a_selection() {
        let out = Declaration { frame: ROOT }.render();
        assert_eq!(&out[..FRAME_KEY.len()], b"f.frame=");
        assert_eq!(Declaration::parse(&out), Ok(Declaration { frame: ROOT }));

        // The two tokens are not each other, and neither parser accepts the
        // other's word. That is the property the two types exist for: a caller
        // that fed a root to the frame comparison would be comparing thirty-two
        // bytes that mean something else and finding them unequal for the right
        // answer's wrong reason.
        let unknown = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);
        assert_eq!(Declaration::parse(&Selection { root: ROOT }.render()), Err(unknown));
        assert_eq!(Selection::parse(&out), Err(unknown));
    }

    #[test]
    fn a_declaration_that_is_not_sixty_four_lower_case_hex_digits_is_refused() {
        // The same alphabet, checked again through the other token, because the
        // shared parser is an implementation detail and the *grammar* is what
        // both keys promise. A refactor that gave one of them a lenient parser
        // would pass the test above and fail this one.
        let bad = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
        let good = Declaration { frame: ROOT }.render();

        let mut shouted = good;
        shouted[FRAME_KEY.len() + 10] = b'A';
        assert_eq!(Declaration::parse(&shouted), Err(bad));
        assert_eq!(Declaration::parse(&good[..good.len() - 1]), Err(bad));
        assert_eq!(Declaration::parse(b"f.frame="), Err(bad));
    }
}
