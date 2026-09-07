// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The build side of the frame's measurement: the same two ranges, from the ELF.
//!
//! RFC 0012 fixes what "the frame hash" is in one sentence — *SHA-256 over two
//! named ranges of the linked kernel image, in this order:
//! `__text_start .. __text_end`, then `__rodata_start .. __rodata_end`* — and
//! this file is one of the two implementations of that sentence.
//! `kernel/src/measure.rs` is the other, and the whole value of the arrangement
//! is that the two are computed by different code over different views of the
//! same bytes: this one reads a file on a host, that one reads its own mapped
//! image on a machine. One implementation shared between them would agree with
//! itself and say nothing.
//!
//! # Why the ranges are found twice, by section and by symbol
//!
//! Because the two could come apart and nothing would say so. The linker script
//! puts `__text_start` at the head of the `.text` output section and
//! `__text_end` at its foot, so the section's `[sh_addr, sh_addr + sh_size)` and
//! the symbol pair are the same interval *today*. Reading only the section would
//! keep working if somebody moved a symbol; reading only the symbols would mean
//! mapping addresses back to file offsets by hand. So this reads the section for
//! its bytes, reads the symbols for their addresses, and **requires them to
//! agree** — which turns a linker-script edit that decoupled them into a refusal
//! naming both intervals rather than into a hash that quietly covers a different
//! one than the frame recomputes.
//!
//! *Reversal:* a linker script in which text or rodata is deliberately more than
//! one output section. Then the interval is a list, this file walks it, and RFC
//! 0012's sentence grows a plural.
//!
//! # What is deliberately not read
//!
//! Everything else in the image. `.boot`, `.data`, `.got`, `.bss` and `.stacks`
//! are outside the measurement by RFC 0012's decision and not by this file's
//! convenience, and that RFC states the residual: a frame change confined to a
//! writable initial value is invisible here.

use std::path::Path;

/// The two ranges, in the order they are hashed: the section the bytes come
/// from, and the symbol pair the interval is defined by.
///
/// The order is the wire — swapping the two would produce a different digest
/// over the same bytes — so it is written down once, here, and
/// `kernel/src/measure.rs` states it again in the same order beside the same RFC
/// citation.
const RANGES: [(&str, &str, &str); 2] =
    [(".text", "__text_start", "__text_end"), (".rodata", "__rodata_start", "__rodata_end")];

/// One measured range: where it is, and how much of it was hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    /// The section name the bytes came from.
    /// Unit: none — a name.
    pub section: &'static str,
    /// The virtual address the range starts at, from the section header and the
    /// boundary symbol, which had to agree to get here.
    /// Unit: bytes, virtual.
    pub start: u64,
    /// How many bytes were hashed.
    /// Unit: bytes.
    pub len: u64,
}

/// The frame hash of a linked kernel image: the two ranges, in order.
///
/// # Errors
///
/// A file that cannot be read, an ELF64 this reader does not believe, a missing
/// section or symbol, or a section and a symbol pair that disagree — the last
/// being the one worth having, because it is the failure a linker-script change
/// would otherwise cause silently.
pub fn frame(path: &Path) -> Result<([u8; 32], [Range; 2]), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("reading {} to measure the frame: {e}", crate::relative(path)))?;
    let elf = Elf::read(&bytes, &crate::relative(path))?;

    let mut state = f_hash::Sha256::new();
    let mut ranges = [Range { section: "", start: 0, len: 0 }; 2];
    for (slot, (section, start_symbol, end_symbol)) in RANGES.iter().enumerate() {
        let head = elf.section(section)?;
        let start = elf.symbol(start_symbol)?;
        let end = elf.symbol(end_symbol)?;

        if head.addr != start || head.addr.saturating_add(head.size) != end {
            return Err(format!(
                "in {}, the `{section}` section covers [{:#x}, {:#x}) and `{start_symbol} .. \
                 {end_symbol}` covers [{start:#x}, {end:#x}).\n\n\
                 RFC 0012 defines the frame hash over the *symbols*, and this reader takes the \
                 bytes from the *section*, so the two agreeing is what makes them one \
                 measurement. They no longer do. Either kernel/linker.ld moved a boundary out \
                 of the section it bounds — in which case this reader has to follow it — or the \
                 range is now more than one output section, which is the plural RFC 0012's \
                 sentence does not have and is the reversal this file names.",
                elf.name,
                head.addr,
                head.addr.saturating_add(head.size),
            ));
        }

        let from = usize::try_from(head.offset).map_err(|_| "a vast section offset".to_string())?;
        let len = usize::try_from(head.size).map_err(|_| "a vast section".to_string())?;
        let to = from.checked_add(len).ok_or("a section that runs off the end")?;
        let body = bytes.get(from..to).ok_or_else(|| {
            format!(
                "in {}, the `{section}` section claims [{from}, {to}) of a {} byte file. A \
                 section header naming bytes the file does not have is not an image this reader \
                 will measure.",
                elf.name,
                bytes.len()
            )
        })?;
        state.update(body);
        ranges[slot] = Range { section, start, len: head.size };
    }

    Ok((state.finish(), ranges))
}

/// A section header, reduced to the three fields this file uses.
#[derive(Clone, Copy)]
struct Section {
    addr: u64,
    offset: u64,
    size: u64,
}

/// Just enough ELF64 to find a section by name and a symbol by name.
///
/// Hand-rolled rather than a crate, for `xtask`'s standing reason: this tool is
/// policy made executable, and every dependency it grows is a dependency the
/// policy checker itself has to be trusted through. A hundred lines of field
/// offsets against a format that has not changed since 1999 is the cheaper side
/// of that trade, and every offset below is named in the comment beside it.
struct Elf<'a> {
    bytes: &'a [u8],
    name: String,
    /// Where the section header table starts.
    shoff: usize,
    /// How many headers it has.
    shnum: usize,
    /// How wide one header is, as the file states it rather than as this reader
    /// assumes: the format allows a larger entry and a reader that stepped by
    /// sixty-four regardless would walk into the middle of the next one.
    shentsize: usize,
    /// Which of them is the section-name string table.
    shstrndx: usize,
}

impl<'a> Elf<'a> {
    fn read(bytes: &'a [u8], name: &str) -> Result<Self, String> {
        let bad = |why: &str| format!("{name} is not an ELF64 this reader believes: {why}");
        if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
            return Err(bad("no ELF magic"));
        }
        // `e_ident[EI_CLASS] = 2` is ELFCLASS64 and `[EI_DATA] = 1` is
        // little-endian. Both are checked rather than assumed, because every
        // multi-byte read below is a little-endian read on a 64-bit layout.
        if bytes[4] != 2 || bytes[5] != 1 {
            return Err(bad("not a little-endian 64-bit object"));
        }
        // `e_shoff` at 0x28, `e_shentsize` at 0x3a, `e_shnum` at 0x3c and
        // `e_shstrndx` at 0x3e — the ELF64 header's tail, in that order.
        let shoff = long(bytes, 0x28) as usize;
        let shentsize = short(bytes, 0x3a) as usize;
        let shnum = short(bytes, 0x3c) as usize;
        let shstrndx = short(bytes, 0x3e) as usize;
        if shentsize < 64 || shnum == 0 || shstrndx >= shnum {
            return Err(bad("a section header table this reader cannot walk"));
        }
        Ok(Self { bytes, name: name.to_string(), shoff, shnum, shentsize, shstrndx })
    }

    /// One section header, by index, as the sixty-four bytes this reader knows.
    fn header(&self, index: usize) -> Result<&'a [u8], String> {
        let at = self.shoff.saturating_add(index.saturating_mul(self.shentsize));
        self.bytes
            .get(at..at.saturating_add(64))
            .ok_or_else(|| format!("{}: section header {index} is past the end", self.name))
    }

    /// A NUL-terminated name out of a string table.
    ///
    /// Answers the empty string for anything it cannot read, which no real name
    /// is, so a malformed table produces a *not found* refusal naming what was
    /// wanted rather than a panic naming an index.
    fn string(&self, table: &Section, at: u32) -> &'a str {
        let from = (table.offset as usize).saturating_add(at as usize);
        let rest = self.bytes.get(from..).unwrap_or(&[]);
        let end = rest.iter().position(|byte| *byte == 0).unwrap_or(rest.len());
        core::str::from_utf8(&rest[..end]).unwrap_or("")
    }

    fn section_at(&self, index: usize) -> Result<Section, String> {
        let head = self.header(index)?;
        // `sh_addr` at 16, `sh_offset` at 24, `sh_size` at 32.
        Ok(Section { addr: long(head, 16), offset: long(head, 24), size: long(head, 32) })
    }

    /// The index of a section, by name.
    fn index_of(&self, want: &str) -> Result<usize, String> {
        let names = self.section_at(self.shstrndx)?;
        for index in 0..self.shnum {
            let head = self.header(index)?;
            // `sh_name` at 0, an offset into the section-name string table.
            if self.string(&names, word(head, 0)) == want {
                return Ok(index);
            }
        }
        Err(format!(
            "{} has no `{want}` section.\n\nRFC 0012 measures the frame over `.text` and \
             `.rodata`, so an image missing one of them is an image whose identity this tree \
             cannot state.",
            self.name
        ))
    }

    /// A section by name.
    fn section(&self, want: &str) -> Result<Section, String> {
        let index = self.index_of(want)?;
        self.section_at(index)
    }

    /// A symbol's value, by name, from `.symtab`.
    ///
    /// The strings come through `.symtab`'s own `sh_link` rather than from a
    /// section called `.strtab`: that name is a convention and the link is the
    /// format, and this reader would rather fail on a file that has no symbol
    /// table than succeed against the wrong string table.
    fn symbol(&self, want: &str) -> Result<u64, String> {
        let index = self.index_of(".symtab")?;
        let table = self.section_at(index)?;
        // `sh_link` at 40.
        let strings = self.section_at(word(self.header(index)?, 40) as usize)?;

        // `Elf64_Sym` is twenty-four bytes: `st_name` at 0, `st_value` at 8.
        let count = (table.size / 24) as usize;
        for entry in 0..count {
            let at = (table.offset as usize).saturating_add(entry.saturating_mul(24));
            let Some(symbol) = self.bytes.get(at..at.saturating_add(24)) else { break };
            if self.string(&strings, word(symbol, 0)) == want {
                return Ok(long(symbol, 8));
            }
        }
        Err(format!(
            "{} exports no `{want}`.\n\nkernel/linker.ld is what exports it, and RFC 0012 names \
             it as one of the four boundaries the frame hash is taken between. A build whose \
             linker script no longer exports it is a build whose frame hash would be over an \
             interval nobody wrote down.",
            self.name
        ))
    }
}

fn short(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn long(bytes: &[u8], at: usize) -> u64 {
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the fixture puts its two ranges. Arbitrary, page-aligned, and
    /// higher-half so that a reader confusing a virtual address with a file
    /// offset produces something obviously wrong rather than something plausible.
    const TEXT_AT: u64 = 0xFFFF_FFFF_8010_0000;
    const RODATA_AT: u64 = 0xFFFF_FFFF_8020_0000;

    /// A minimal ELF64 with the four boundary symbols and the two sections.
    ///
    /// Written by hand rather than by building a kernel, because what is under
    /// test is the *reader* and a fixture that had to be compiled could not
    /// express the case worth testing: a section and a symbol pair that disagree.
    /// The real image is exercised on every boot — `cargo xtask attest` requires
    /// the frame to arrive at the same digest this reader does, which is the
    /// other half of the evidence and the half no unit test can produce.
    struct Fixture {
        text: Vec<u8>,
        rodata: Vec<u8>,
        /// What `__text_end` says, which is normally `TEXT_AT + text.len()`.
        text_end: u64,
    }

    impl Fixture {
        fn new(text: &[u8], rodata: &[u8]) -> Self {
            Self {
                text: text.to_vec(),
                rodata: rodata.to_vec(),
                text_end: TEXT_AT + text.len() as u64,
            }
        }

        fn build(&self) -> Vec<u8> {
            const NAMES: &[&str] = &["", ".text", ".rodata", ".symtab", ".strtab", ".shstrtab"];
            let mut shstrtab = Vec::new();
            let mut name_at = Vec::new();
            for name in NAMES {
                name_at.push(shstrtab.len() as u32);
                shstrtab.extend_from_slice(name.as_bytes());
                shstrtab.push(0);
            }

            let symbols = ["", "__text_start", "__text_end", "__rodata_start", "__rodata_end"];
            let values =
                [0, TEXT_AT, self.text_end, RODATA_AT, RODATA_AT + self.rodata.len() as u64];
            let mut strtab = Vec::new();
            let mut symtab = Vec::new();
            for (name, value) in symbols.iter().zip(values) {
                let at = strtab.len() as u32;
                strtab.extend_from_slice(name.as_bytes());
                strtab.push(0);
                let mut entry = [0u8; 24];
                entry[0..4].copy_from_slice(&at.to_le_bytes());
                entry[8..16].copy_from_slice(&value.to_le_bytes());
                symtab.extend_from_slice(&entry);
            }

            // Section bodies laid out after the header, in section order, so
            // that a file offset and a virtual address are never the same
            // number by accident.
            let mut body = Vec::new();
            let mut offsets = Vec::new();
            for bytes in [&self.text, &self.rodata, &symtab, &strtab, &shstrtab] {
                offsets.push(64 + body.len() as u64);
                body.extend_from_slice(bytes);
            }
            let shoff = 64 + body.len() as u64;

            let mut out = vec![0u8; 64];
            out[0..4].copy_from_slice(b"\x7fELF");
            out[4] = 2;
            out[5] = 1;
            out[0x28..0x30].copy_from_slice(&shoff.to_le_bytes());
            out[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
            out[0x3c..0x3e].copy_from_slice(&6u16.to_le_bytes());
            out[0x3e..0x40].copy_from_slice(&5u16.to_le_bytes());
            out.extend_from_slice(&body);

            // Index 0 is the null header the format reserves.
            out.extend_from_slice(&[0u8; 64]);
            let sizes = [
                self.text.len() as u64,
                self.rodata.len() as u64,
                symtab.len() as u64,
                strtab.len() as u64,
                shstrtab.len() as u64,
            ];
            let addrs = [TEXT_AT, RODATA_AT, 0, 0, 0];
            for index in 0..5 {
                let mut head = [0u8; 64];
                head[0..4].copy_from_slice(&name_at[index + 1].to_le_bytes());
                head[16..24].copy_from_slice(&addrs[index].to_le_bytes());
                head[24..32].copy_from_slice(&offsets[index].to_le_bytes());
                head[32..40].copy_from_slice(&sizes[index].to_le_bytes());
                // `sh_link` on `.symtab` names `.strtab`, which is index 4.
                if index == 2 {
                    head[40..44].copy_from_slice(&4u32.to_le_bytes());
                }
                out.extend_from_slice(&head);
            }
            out
        }

        fn measure(&self) -> Result<([u8; 32], [Range; 2]), String> {
            let dir = std::env::temp_dir().join(format!(
                "f-measure-{}-{}",
                std::process::id(),
                self.text.len() * 31 + self.rodata.len() * 7 + self.text_end as usize
            ));
            std::fs::write(&dir, self.build()).expect("a fixture this test wrote");
            let out = frame(&dir);
            let _ = std::fs::remove_file(&dir);
            out
        }
    }

    #[test]
    fn the_digest_is_text_then_rodata_and_the_order_is_load_bearing() {
        let (digest, ranges) = Fixture::new(b"TTTT", b"RRRRRRRR").measure().expect("a fixture");

        // Not *a* hash of those bytes — the hash of them concatenated in that
        // order, computed independently of the reader. RFC 0012 fixes the order,
        // and a reader that hashed rodata first would agree with itself forever
        // and disagree with every frame that ever booted.
        assert_eq!(digest, f_hash::sha256(b"TTTTRRRRRRRR"));
        assert_ne!(digest, f_hash::sha256(b"RRRRRRRRTTTT"));

        assert_eq!(ranges[0], Range { section: ".text", start: TEXT_AT, len: 4 });
        assert_eq!(ranges[1], Range { section: ".rodata", start: RODATA_AT, len: 8 });
    }

    #[test]
    fn a_byte_of_rodata_moves_the_digest() {
        // The property `cargo xtask attest` demonstrates against a real image,
        // asserted here against the reader alone so that a failure can be told
        // apart from a boot that did not happen.
        let before = Fixture::new(b"TTTT", b"RRRRRRRR").measure().expect("a fixture").0;
        let after = Fixture::new(b"TTTT", b"RRRRRRRS").measure().expect("a fixture").0;
        assert_ne!(before, after);
    }

    #[test]
    fn a_symbol_that_does_not_bound_its_section_is_refused() {
        // The case this reader exists to catch and the reason it reads the
        // ranges twice. A linker script that moved `__text_end` out of `.text`
        // would otherwise leave the build side hashing one interval and the
        // frame recomputing another, and the first thing to notice would be
        // every boot refusing to publish a root with no line saying why.
        let mut drifted = Fixture::new(b"TTTT", b"RRRRRRRR");
        drifted.text_end = TEXT_AT + 3;
        let why = drifted.measure().expect_err("a symbol that does not bound its section");
        assert!(why.contains("__text_start .. __text_end"), "{why}");
        assert!(why.contains("RFC 0012"), "{why}");
    }

    #[test]
    fn a_file_that_is_not_an_elf64_is_refused_before_anything_is_read() {
        let dir = std::env::temp_dir().join(format!("f-measure-junk-{}", std::process::id()));
        std::fs::write(&dir, b"not an elf at all, not even close").expect("a fixture");
        let why = frame(&dir).expect_err("junk");
        let _ = std::fs::remove_file(&dir);
        assert!(why.contains("not an ELF64 this reader believes"), "{why}");
    }
}
