// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Content addressing and archiving, with no dependency outside this file.
//!
//! # Why both of these are written here
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
//! Neither is novel and neither should be clever. SHA-256 is FIPS 180-4 and is
//! checked here against the two vectors that document publishes; the archive is
//! POSIX ustar with every variable field nailed to a constant.

/// The SHA-256 of some bytes.
///
/// The reference implementation, transcribed. There is nothing to optimise
/// here: the largest thing this hashes is a kernel image, once, in a command
/// that also runs a compiler.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
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

    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    // The padding, as the standard states it: a one bit, then zeroes, then the
    // length in bits as a big-endian u64, to a multiple of 64 bytes.
    let mut message = bytes.to_vec();
    let bits = (bytes.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());

    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
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

        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

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
    fn sha256_matches_the_vectors_the_standard_publishes() {
        // FIPS 180-4's own two examples. A hash implementation checked only
        // against itself is a hash function, just not that one.
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_spans_the_block_boundary() {
        // 55, 56 and 64 bytes are where the padding changes shape: 56 is the
        // length that no longer leaves room for the length field, so it grows a
        // whole extra block. An implementation that is wrong anywhere is
        // usually wrong exactly here.
        let long = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha256(&long)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        for len in [55usize, 56, 57, 63, 64, 65] {
            // Not a known vector, but it must not panic and must depend on the
            // length: two different inputs hashing alike here would be a
            // padding bug rather than a collision.
            let a = sha256(&vec![0u8; len]);
            let b = sha256(&vec![0u8; len + 1]);
            assert_ne!(a, b, "inputs of {len} and {} bytes hashed alike", len + 1);
        }
    }

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
