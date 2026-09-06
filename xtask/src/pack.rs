// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Archiving, and the hexadecimal a manifest prints a digest as.
//!
//! # Why the archive is written here
//!
//! `RELEASING.md`'s rule is that no step exists that is not in the tree, and a
//! release package is the one artefact where that rule is load-bearing rather
//! than tidy. Until this module existed, `release --dry-run` computed its hashes
//! by shelling out to `sha256sum` — so the *content address of a release*
//! depended on which coreutils the machine had, and on a machine without it the
//! command printed a manifest with the hash column simply absent. A content
//! address that is sometimes present is not a content address.
//!
//! The archive is here for the same reason and one more: every archiver in
//! reach writes a timestamp, a user name and a directory order into the bytes,
//! and the exit criterion this serves is that two machines produce *identical*
//! bytes. That is not something to configure out of a general-purpose tool; it
//! is the whole specification, and it is forty lines.
//!
//! # Why the SHA-256 is not written here any more
//!
//! It was, and the transcription is gone: [`f_hash`] is the one in the tree and
//! this module calls it like everything else does. The reason is RFC 0012's
//! *one identity* — a release address, a blob's name and a generation's root
//! are the same statement about bytes, and a second transcription of FIPS 180-4
//! is a statement that agrees on every input anybody tries until the day it does
//! not. The two published vectors moved with the code, so they are checked in
//! `hash/`, once. What stayed is [`hex`], which is text rather than hashing:
//! `f-hash` has no allocator and a `MANIFEST` line is a `String`.
//!
//! Neither half is novel and neither should be clever. The archive is POSIX
//! ustar with every variable field nailed to a constant.

/// A digest, as the sixty-four characters everything else in the world prints.
#[must_use]
pub fn hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('?'));
        out.push(char::from_digit(u32::from(byte & 0xF), 16).unwrap_or('?'));
    }
    out
}

/// A POSIX ustar archive with every variable field nailed to a constant.
///
/// # What is pinned, and why each one had to be
///
/// Each of these is a way two machines produce different bytes for the same
/// tree, and every general-purpose archiver writes at least four of them:
///
/// - **Modification time: zero.** The obvious one, and the only one people
///   remember.
/// - **Owner and group: zero, with empty names.** A package built by `root` in
///   a container and by a user on a laptop otherwise differ by a name nobody
///   chose.
/// - **Mode: 0644, or 0755 for the one executable.** Whatever the checkout's
///   umask was is not part of the release.
/// - **Entry order: the caller's, and the caller sorts.** `read_dir` order is
///   a filesystem's business and differs between two machines with the same
///   files — the one difference a build-it-twice check on a *single* machine
///   cannot see.
/// - **No compression.** Not an omission: a deflate stream carries the encoder
///   version and level in its output, so compressing here would add a
///   dependency whose *version* reaches the content address. The package is a
///   tar, and whoever ships it may compress it afterwards — that is an envelope
///   and not the content.
pub struct Tar {
    out: Vec<u8>,
}

impl Tar {
    #[must_use]
    pub fn new() -> Self {
        Self { out: Vec::new() }
    }

    /// Add one regular file.
    ///
    /// # Errors
    ///
    /// A path that does not fit ustar's 100-byte name field. Refused rather
    /// than silently written as a PAX extension, because a PAX header carries
    /// its own set of variable fields and this type's whole claim is that it
    /// has none.
    pub fn file(&mut self, name: &str, executable: bool, data: &[u8]) -> Result<(), String> {
        if name.len() >= 100 {
            return Err(format!(
                "{name} is {} bytes and ustar's name field is 100. Shorten the path,\n\
                 or teach this to write a prefix field — but not a PAX header.",
                name.len()
            ));
        }

        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        write_octal(&mut header[100..108], if executable { 0o755 } else { 0o644 });
        write_octal(&mut header[108..116], 0); // uid
        write_octal(&mut header[116..124], 0); // gid
        write_octal(&mut header[124..136], data.len() as u64);
        write_octal(&mut header[136..148], 0); // mtime
        header[156] = b'0'; // a regular file
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");

        // The checksum is computed with its own field read as eight spaces,
        // which is the one piece of this format that is not obvious.
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
        write_octal(&mut header[148..154], u64::from(sum));
        header[154] = 0;
        header[155] = b' ';

        self.out.extend_from_slice(&header);
        self.out.extend_from_slice(data);
        let padding = (512 - data.len() % 512) % 512;
        self.out.resize(self.out.len() + padding, 0);
        Ok(())
    }

    /// The archive: two zero blocks and done.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        self.out.resize(self.out.len() + 1024, 0);
        self.out
    }
}

impl Default for Tar {
    fn default() -> Self {
        Self::new()
    }
}

/// A ustar numeric field: zero-padded octal, then a NUL.
fn write_octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    let text = format!("{value:0digits$o}");
    field[..digits].copy_from_slice(&text.as_bytes()[text.len() - digits..]);
    field[digits] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_is_a_function_of_its_contents_and_nothing_else() {
        // The exit criterion, stated where it can fail cheaply. Two archives
        // built from the same names and bytes must be the same archive — there
        // is no clock, no user and no filesystem in this type.
        let build = || {
            let mut tar = Tar::new();
            tar.file("MANIFEST", false, b"one\ntwo\n").expect("a short name");
            tar.file("kernel.elf32", true, &[0xCC; 700]).expect("a short name");
            tar.finish()
        };
        assert_eq!(build(), build());

        let mut other = Tar::new();
        other.file("MANIFEST", false, b"one\nTWO\n").expect("a short name");
        other.file("kernel.elf32", true, &[0xCC; 700]).expect("a short name");
        assert_ne!(build(), other.finish(), "one changed byte did not change the archive");
    }

    #[test]
    fn an_archive_is_blocked_and_terminated_the_way_tar_expects() {
        let mut tar = Tar::new();
        tar.file("a", false, b"x").expect("a short name");
        let bytes = tar.finish();

        // Header, one padded data block, two zero blocks.
        assert_eq!(bytes.len(), 512 * 4);
        assert_eq!(&bytes[257..263], b"ustar\0");
        assert_eq!(bytes[156], b'0', "not marked as a regular file");
        assert!(bytes[512 * 2..].iter().all(|b| *b == 0), "the trailer is not zeroed");
    }

    #[test]
    fn a_name_ustar_cannot_hold_is_refused_rather_than_truncated() {
        let mut tar = Tar::new();
        let long = "d/".repeat(60);
        assert!(tar.file(&long, false, b"").is_err(), "a 120-byte name was accepted");
    }
}
