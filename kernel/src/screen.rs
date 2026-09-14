// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The boot log, on a screen.
//!
//! # Why this is in the frame, when `intent/0011` says a component should draw
//!
//! Because on the machine this was asked for there is no component yet, and
//! there is no serial cable either. `docs/booting-on-hardware.md` states the
//! situation plainly: with no serial port you see a black screen and have no
//! way to tell a clean halt from a crash. Every boot outside the emulator so
//! far has been read down a wire into `E:\@vmw\Arch\`, and the three that
//! mattered most — the one that stalled inside core bring-up among them —
//! produced their evidence *before* anything above the frame was running.
//!
//! So this is the fallback `intent/0011` names in advance and RFC 0081
//! declares: a fixed-grid character console, inside the frame, with its cost to
//! the unsafe count stated rather than discovered. It is **not** the
//! presentation path. It has no scene, no compositor, no client and no ring; it
//! renders one stream of bytes the frame was already printing, and the day a
//! component can draw, this stops being the thing on the screen and becomes the
//! thing that was on the screen until it could.
//!
//! # What it does not do, deliberately
//!
//! It never reads the surface. Scrolling a framebuffer by reading it back is
//! the single worst habit of the console this replaces: the mapping is
//! uncacheable, and a read from it is a bus transaction that stalls the core.
//! The text is held in [`Console::want`] — ordinary cached memory — and the
//! screen is brought level with it by writing only the cells that differ from
//! [`Console::have`]. A scroll is therefore a shift of two small arrays and a
//! redraw of the cells that actually changed, which on a log screen is far
//! fewer than all of them because most of a log line is trailing blank.
//!
//! It does no colour, no cursor, no control sequences and no wrapping policy
//! beyond the obvious one. A console that grew those would be a terminal
//! emulator in ring 0, which is a thing to be avoided rather than achieved.
//!
//! # The one place two cores meet, which is the fifth
//!
//! RFC 0016 says every mutable `static` under `kernel/` is a `PerCpu<T>` and
//! that a fifth cross-core word needs an argument. [`CONSOLE`] is that fifth
//! place and the argument is in RFC 0081; the short form is that a screen is
//! one device the way COM1 is one device, every core already writes COM1
//! unsynchronised, and a *per-core* console would be wrong by construction —
//! two cursors over one surface — rather than wrong under a race. Every index
//! here is bounds-checked, so the worst a race produces is a garbled cell.

use core::cell::UnsafeCell;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::multiboot::{Channels, Framebuffer, FramebufferKind};

/// Glyph box width, before scaling. Unit: pixels.
const GLYPH_W: usize = 6;

/// Glyph box height, before scaling. Unit: pixels.
const GLYPH_H: usize = 8;

/// How many screen pixels one glyph pixel becomes, on each axis.
///
/// Two rather than one, and the reason is a person rather than a number: at one
/// this font is six pixels wide, which on a 1080p panel is a character about a
/// millimetre across and unreadable at the distance somebody stands from a
/// machine they are bringing up. At two the grid is 160 by 67 on that panel and
/// 85 by 48 on the 1024 by 768 a virtual machine usually starts in, which is a
/// terminal-shaped screen in both.
const SCALE: usize = 2;

/// The widest grid this console will keep, in characters.
///
/// A surface wider than this is *clipped*, not refused: the console is a
/// fallback for reading a boot, and half a line on the screen beats a refusal
/// on a machine whose only other output is the serial port that is missing.
const MAX_COLS: usize = 160;

/// The tallest grid this console will keep, in characters.
const MAX_ROWS: usize = 64;

/// Cells in the grid.
const CELLS: usize = MAX_COLS * MAX_ROWS;

/// First and last character the font has a glyph for.
const FIRST: u8 = 0x20;
const LAST: u8 = 0x7E;

/// What is drawn for a byte the font has no glyph for.
///
/// A visible box rather than a space, because a log line containing a byte this
/// console cannot draw is a fact worth seeing — silently dropping it would make
/// the screen disagree with the serial log, and the two disagreeing is the one
/// failure this whole file would have no way to report.
const REPLACEMENT: u8 = b'?';

/// The font: five pixels wide, seven tall, in a six-by-eight box.
///
/// One row per glyph, in ASCII order from [`FIRST`]. Bit 4 is the leftmost
/// pixel of a row and bit 0 the rightmost, so a literal written in binary reads
/// left to right as the pixels appear. The eighth row is blank for every glyph
/// that has no descender, which is what separates one line from the next
/// without the grid needing a gap of its own.
///
/// **This table is original work and is deliberately not an imported font.**
/// `LICENSING.md` draws the licence boundary at `third_party/`, and a font is
/// exactly the kind of asset that arrives with a licence nobody reads — the
/// obvious candidate, GNU Unifont, is GPL and would have taken the whole frame
/// with it. What is here is plain enough to have been typed, and
/// [`dump_font`] is how it was checked: the glyphs were read back as characters
/// on a serial line before anything was asked to draw them on a screen.
static GLYPHS: [[u8; GLYPH_H]; 95] = [
    [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000], // space
    [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100, 0b00000], // !
    [0b01010, 0b01010, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000], // "
    [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010, 0b00000], // #
    [0b00100, 0b01111, 0b10100, 0b01110, 0b00101, 0b11110, 0b00100, 0b00000], // $
    [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011, 0b00000], // %
    [0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101, 0b00000], // &
    [0b00100, 0b00100, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000], // '
    [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010, 0b00000], // (
    [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000, 0b00000], // )
    [0b00000, 0b00100, 0b10101, 0b01110, 0b10101, 0b00100, 0b00000, 0b00000], // *
    [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000, 0b00000], // +
    [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000], // ,
    [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000, 0b00000], // -
    [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100, 0b00000], // .
    [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000, 0b00000], // /
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110, 0b00000], // 0
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110, 0b00000], // 1
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111, 0b00000], // 2
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110, 0b00000], // 3
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010, 0b00000], // 4
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110, 0b00000], // 5
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110, 0b00000], // 6
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00000], // 7
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110, 0b00000], // 8
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100, 0b00000], // 9
    [0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000, 0b00000], // :
    [0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b00100, 0b01000, 0b00000], // ;
    [0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010, 0b00000], // <
    [0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000], // =
    [0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000, 0b00000], // >
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100, 0b00000], // ?
    [0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01110, 0b00000], // @
    [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0b00000], // A
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110, 0b00000], // B
    [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110, 0b00000], // C
    [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100, 0b00000], // D
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111, 0b00000], // E
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b00000], // F
    [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111, 0b00000], // G
    [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0b00000], // H
    [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110, 0b00000], // I
    [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100, 0b00000], // J
    [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001, 0b00000], // K
    [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111, 0b00000], // L
    [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001, 0b00000], // M
    [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b00000], // N
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0b00000], // O
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000, 0b00000], // P
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101, 0b00000], // Q
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001, 0b00000], // R
    [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110, 0b00000], // S
    [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000], // T
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0b00000], // U
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00000], // V
    [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001, 0b00000], // W
    [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001, 0b00000], // X
    [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000], // Y
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111, 0b00000], // Z
    [0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110, 0b00000], // [
    [0b10000, 0b01000, 0b01000, 0b00100, 0b00010, 0b00010, 0b00001, 0b00000], // \
    [0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110, 0b00000], // ]
    [0b00100, 0b01010, 0b10001, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000], // ^
    [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111], // _
    [0b01000, 0b00100, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000], // `
    [0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111, 0b00000], // a
    [0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110, 0b00000], // b
    [0b00000, 0b00000, 0b01111, 0b10000, 0b10000, 0b10000, 0b01111, 0b00000], // c
    [0b00001, 0b00001, 0b01111, 0b10001, 0b10001, 0b10001, 0b01111, 0b00000], // d
    [0b00000, 0b00000, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110, 0b00000], // e
    [0b00110, 0b01001, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000, 0b00000], // f
    [0b00000, 0b00000, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110], // g
    [0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b00000], // h
    [0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110, 0b00000], // i
    [0b00010, 0b00000, 0b00110, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100], // j
    [0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b00000], // k
    [0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110, 0b00000], // l
    [0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10101, 0b10101, 0b00000], // m
    [0b00000, 0b00000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b00000], // n
    [0b00000, 0b00000, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110, 0b00000], // o
    [0b00000, 0b00000, 0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000], // p
    [0b00000, 0b00000, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001], // q
    [0b00000, 0b00000, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000, 0b00000], // r
    [0b00000, 0b00000, 0b01111, 0b10000, 0b01110, 0b00001, 0b11110, 0b00000], // s
    [0b01000, 0b01000, 0b11100, 0b01000, 0b01000, 0b01001, 0b00110, 0b00000], // t
    [0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b10011, 0b01101, 0b00000], // u
    [0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00000], // v
    [0b00000, 0b00000, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010, 0b00000], // w
    [0b00000, 0b00000, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b00000], // x
    [0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110], // y
    [0b00000, 0b00000, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111, 0b00000], // z
    [0b00010, 0b00100, 0b00100, 0b01000, 0b00100, 0b00100, 0b00010, 0b00000], // {
    [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000], // |
    [0b01000, 0b00100, 0b00100, 0b00010, 0b00100, 0b00100, 0b01000, 0b00000], // }
    [0b00000, 0b00000, 0b01000, 0b10101, 0b00010, 0b00000, 0b00000, 0b00000], // ~
];

/// The table covers exactly the printable range and no reader has to count.
const _: () = assert!(GLYPHS.len() == (LAST - FIRST) as usize + 1);

/// The rows of one character's glyph.
fn glyph(ch: u8) -> &'static [u8; GLYPH_H] {
    let ch = if (FIRST..=LAST).contains(&ch) { ch } else { REPLACEMENT };
    &GLYPHS[(ch - FIRST) as usize]
}

/// Somewhere pixels can be written, and nothing else.
///
/// Deliberately not a *framebuffer*: [`selftest`] builds one of these over an
/// ordinary array on the stack, which is what lets the whole path above the
/// last volatile write be exercised on a machine with no display at all. The
/// type knows an address, a shape and how to pack a colour, and knows nothing
/// about where the memory came from.
#[derive(Clone, Copy)]
pub struct Surface {
    /// Virtual address of the first pixel.
    at: u64,
    /// Bytes from one row to the next. Unit: bytes.
    pitch: u32,
    /// Unit: pixels.
    width: u32,
    /// Unit: pixels.
    height: u32,
    /// Unit: bytes.
    bytes_per_pixel: u32,
    /// The packed value of a lit pixel.
    ink: u32,
    /// The packed value of an unlit one.
    paper: u32,
}

impl Surface {
    /// Describe a surface the loader handed over, mapped at `at`.
    ///
    /// The colours are computed once, here, rather than per pixel: packing is
    /// the only part of drawing that depends on the channel layout, and a
    /// per-pixel `match` on a format that cannot change during a boot is the
    /// kind of cost that does not look like one.
    #[must_use]
    pub fn over(fb: &Framebuffer, at: u64) -> Self {
        let bytes_per_pixel = u32::from(fb.bits_per_pixel).div_ceil(8);
        let (ink, paper) = match fb.kind {
            // A light grey on near-black. Not white on black: a boot log is
            // read for minutes at a time, and full-intensity white on a panel
            // at close range is the one choice a reader notices.
            FramebufferKind::Direct(channels) => {
                (pack(channels, 0xC8, 0xCF, 0xD8), pack(channels, 0x0A, 0x0C, 0x12))
            }
            // Indexed, text and unknown kinds get the two values every palette
            // agrees about at its ends. This console does not load a palette —
            // on a mode it did not ask for, the honest thing is to draw in
            // whatever *is* black and white rather than to guess an index.
            _ => (u32::MAX, 0),
        };

        Self {
            at,
            pitch: fb.pitch,
            width: fb.width,
            height: fb.height,
            bytes_per_pixel,
            ink,
            paper,
        }
    }

    /// Describe a surface over memory the caller owns.
    ///
    /// For [`selftest`], and the two colours are the ones its dump reads back.
    fn over_memory(at: u64, width: u32, height: u32) -> Self {
        Self { at, pitch: width * 4, width, height, bytes_per_pixel: 4, ink: u32::MAX, paper: 0 }
    }

    /// Can this console write a pixel of this depth at all?
    ///
    /// Two, three and four bytes. One byte is an indexed mode and is refused by
    /// [`begin`] rather than drawn badly; anything wider than four is not a
    /// packed pixel format this console has ever seen and is refused for the
    /// same reason — the honest answer to a mode nobody has met is a line
    /// saying so.
    fn drawable(self) -> bool {
        matches!(self.bytes_per_pixel, 2..=4)
    }

    /// Write one pixel, if it is on the surface.
    ///
    /// Clipped rather than checked by the caller, because the caller is a glyph
    /// blit whose last column and row fall off the edge on any screen whose
    /// size is not a multiple of the cell. Silently dropping those is what
    /// *clipping* means and is the whole reason the bound is here.
    fn put(self, x: u32, y: u32, value: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let at = self.at
            + u64::from(y) * u64::from(self.pitch)
            + u64::from(x) * u64::from(self.bytes_per_pixel);

        match self.bytes_per_pixel {
            4 => {
                // SAFETY: `at` is inside the surface — the bound above covers
                // the pixel, and the surface's extent was validated by
                // `Framebuffer::parse` or, under `selftest`, is an array this
                // crate owns. Volatile because a framebuffer is a device: the
                // compiler may not drop, reorder or merge a write to it.
                unsafe { (at as *mut u32).write_volatile(value) };
            }
            3 => {
                let bytes = value.to_le_bytes();
                for (index, byte) in bytes[..3].iter().enumerate() {
                    // The offset is computed here rather than inside the block,
                    // which is `multiboot::word_at`'s rule and the same reason:
                    // computing an address is not the dangerous act,
                    // dereferencing one is, and a block holding both has a
                    // SAFETY comment that covers whichever the reader thinks of
                    // first. It is also what keeps this to one operation.
                    let byte_at = (at as *mut u8).wrapping_add(index);
                    // SAFETY: as above, and three bytes from a pixel's start is
                    // inside a three-byte pixel.
                    unsafe { byte_at.write_volatile(*byte) };
                }
            }
            2 => {
                // SAFETY: as above, at two bytes wide.
                unsafe { (at as *mut u16).write_volatile(value as u16) };
            }
            // A depth this console cannot write is left alone rather than
            // approximated. One byte per pixel is an indexed mode, and writing
            // a packed colour into a palette index would paint in whatever
            // colour that index happens to hold.
            _ => {}
        }
    }

    /// Draw one character in cell `(col, row)`.
    fn cell(self, col: usize, row: usize, ch: u8) {
        let rows = glyph(ch);
        let origin_x = (col * GLYPH_W * SCALE) as u32;
        let origin_y = (row * GLYPH_H * SCALE) as u32;

        for (dy, bits) in rows.iter().enumerate() {
            for dx in 0..GLYPH_W {
                // Bit 4 is the leftmost of the five the font uses; the sixth
                // column of the box is always blank and is what separates one
                // character from the next.
                let lit = dx < 5 && bits & (1 << (4 - dx)) != 0;
                let value = if lit { self.ink } else { self.paper };

                // The scale is a nested loop rather than a wider write because
                // a wider write would have to know the pixel format again, and
                // `put` is the one place in this file that does.
                for sy in 0..SCALE {
                    for sx in 0..SCALE {
                        let x = origin_x + (dx * SCALE + sx) as u32;
                        let y = origin_y + (dy * SCALE + sy) as u32;
                        self.put(x, y, value);
                    }
                }
            }
        }
    }
}

/// Pack a colour the way this surface's channels are laid out.
fn pack(channels: Channels, red: u32, green: u32, blue: u32) -> u32 {
    /// Narrow an eight-bit component to the width a channel actually has.
    fn fit(component: u32, bits: u8, at: u8) -> u32 {
        if bits == 0 || bits > 8 || at > 31 {
            return 0;
        }
        (component >> (8 - bits)) << at
    }

    fit(red, channels.red_bits, channels.red_at)
        | fit(green, channels.green_bits, channels.green_at)
        | fit(blue, channels.blue_bits, channels.blue_at)
}

/// The text on the screen, and the text that should be.
struct Console {
    surface: Option<Surface>,
    cols: usize,
    rows: usize,
    col: usize,
    row: usize,
    /// What the log says.
    want: [u8; CELLS],
    /// What has actually been drawn. The difference between the two is the
    /// damage, and it is the only thing that is ever written to the surface.
    have: [u8; CELLS],
}

impl Console {
    const NEW: Self = Self {
        surface: None,
        cols: 0,
        rows: 0,
        col: 0,
        row: 0,
        want: [b' '; CELLS],
        have: [b' '; CELLS],
    };

    /// Take a surface and work out the grid that fits on it.
    fn attach(&mut self, surface: Surface) {
        self.cols = (surface.width as usize / (GLYPH_W * SCALE)).min(MAX_COLS);
        self.rows = (surface.height as usize / (GLYPH_H * SCALE)).min(MAX_ROWS);
        self.col = 0;
        self.row = 0;
        self.want = [b' '; CELLS];
        // Not `[b' '; CELLS]`: nothing has been drawn, and the surface holds
        // whatever the firmware left in it. Marking every cell as *already a
        // space* would make the first flush skip the clearing pass and leave
        // the firmware's logo behind the text.
        self.have = [0; CELLS];
        self.surface = Some(surface);
    }

    /// Is there anywhere to draw?
    fn live(&self) -> bool {
        self.surface.is_some() && self.cols > 0 && self.rows > 0
    }

    fn put(&mut self, ch: u8) {
        if !self.live() {
            return;
        }

        match ch {
            b'\n' => {
                self.newline();
                return;
            }
            // The serial writer turns a newline into a carriage return and a
            // newline; the screen wants only the newline, and a carriage return
            // that moved the cursor would overprint the line just written.
            b'\r' => return,
            b'\t' => {
                // Four, because the boot report indents in twos and a tab that
                // landed between its columns would make the screen disagree
                // with the serial log about where a line starts.
                let next = (self.col + 4) & !3;
                while self.col < next && self.col < self.cols {
                    self.put(b' ');
                }
                return;
            }
            _ => {}
        }

        if self.col >= self.cols {
            self.newline();
        }
        let at = self.row * MAX_COLS + self.col;
        if let Some(slot) = self.want.get_mut(at) {
            *slot = ch;
        }
        self.col += 1;
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 < self.rows {
            self.row += 1;
        } else {
            self.scroll();
        }
    }

    /// Shift the text up one line. The surface is not touched.
    fn scroll(&mut self) {
        for row in 1..self.rows {
            let (above, here) = ((row - 1) * MAX_COLS, row * MAX_COLS);
            for col in 0..self.cols {
                self.want[above + col] = self.want[here + col];
            }
        }
        let last = (self.rows - 1) * MAX_COLS;
        for col in 0..self.cols {
            self.want[last + col] = b' ';
        }
    }

    /// Bring the surface level with the text, writing only what differs.
    fn flush(&mut self) {
        let Some(surface) = self.surface else { return };
        for row in 0..self.rows {
            for col in 0..self.cols {
                let at = row * MAX_COLS + col;
                if self.want[at] == self.have[at] {
                    continue;
                }
                surface.cell(col, row, self.want[at]);
                self.have[at] = self.want[at];
            }
        }
    }
}

/// The one console, and the one place under `kernel/` that is neither a
/// `PerCpu<T>` nor one of RFC 0016's four handshake words. RFC 0081.
struct Shared(UnsafeCell<Console>);

// SAFETY: a screen is one device, written the way COM1 already is — by whatever
// core is printing, with no lock anywhere in the path. RFC 0081 argues that at
// length; what makes it *sound enough to compile* is narrower and is here: every
// index into `want` and `have` is bounds-checked or masked to the array, so two
// cores racing produce a wrong character in a cell and never a write outside the
// arrays. Nothing in the system reads this state back, so no decision depends on
// it being consistent.
unsafe impl Sync for Shared {}

static CONSOLE: Shared = Shared(UnsafeCell::new(Console::NEW));

/// Is there a screen to draw on? Read on the print path before anything else.
///
/// Separate from the `Option` inside the console so that the overwhelmingly
/// common case — every boot under the emulator, and every line printed before
/// the surface is mapped — is one relaxed load and a branch, rather than a
/// reference into a structure that is forty kibibytes of `.bss`.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// The console, for the core that is printing.
///
/// # Safety
///
/// The caller must not hold the returned reference across anything that could
/// print, which in practice means: use it and drop it inside one function. Two
/// cores may hold one at once, which is what the `Sync` implementation above
/// discharges and RFC 0081 argues.
unsafe fn console() -> &'static mut Console {
    // SAFETY: the caller's guarantee, and the argument on `unsafe impl Sync`.
    unsafe { &mut *CONSOLE.0.get() }
}

/// Start drawing the boot log on this surface.
///
/// Everything printed from here on goes to the screen as well as to the serial
/// port. Nothing printed *before* here is replayed: the frame prints its first
/// lines before there is an address space, let alone a mapping, and a console
/// that pretended otherwise would be holding a copy of the log in order to lie
/// about when it started. The serial port remains the whole log; the screen is
/// the part of it that happened after there was a screen.
/// # Errors
///
/// The reason this surface cannot be drawn on, when it cannot. Refusing here
/// rather than drawing nothing is the difference between a boot that reports a
/// mode it cannot use and one that reports a grid and then shows a blank
/// screen — and a blank screen is exactly what a reader would take for a crash.
pub fn begin(fb: &Framebuffer, at: u64) -> Result<(), &'static str> {
    let surface = Surface::over(fb, at);
    if !surface.drawable() {
        // An indexed mode is the realistic one: at one byte per pixel a written
        // colour is a palette index, and painting in whatever colour that index
        // happens to hold is worse than not painting. The header asks for direct
        // colour, so reaching this means a loader answered with something else.
        return Err("this console draws 16, 24 and 32 bits per pixel, and nothing else");
    }

    // SAFETY: used and dropped inside this function, which prints nothing.
    let console = unsafe { console() };
    console.attach(surface);
    if !console.live() {
        return Err("the surface is smaller than one character cell");
    }

    console.flush();
    ENABLED.store(true, Ordering::Release);
    Ok(())
}

/// How many characters fit, for the line that reports it.
#[must_use]
pub fn grid() -> (usize, usize) {
    // SAFETY: as `begin`.
    let console = unsafe { console() };
    (console.cols, console.rows)
}

/// Set while a core is inside the drawing path, and the reason is the panic
/// handler rather than the second core.
///
/// A panic inside this file would print, and printing comes back here: the
/// handler calls `kprintln!`, which tees, which draws, which panics. That is
/// not a slow path or a garbled cell, it is unbounded recursion into the guard
/// page, and the message that would have said what went wrong is the thing it
/// destroys.
///
/// So the drawing path is entered once. A caller that finds this set writes to
/// the serial port and returns, which means: a panic in here leaves the last
/// good screen standing and reports itself down the wire, and a panic anywhere
/// *else* still draws, because the flag is clear.
///
/// **This is not a lock and must not become one.** Nothing waits on it, nothing
/// retries, and a core that loses skips its line rather than blocking — which
/// is also the honest answer for the second core, whose line reaches the serial
/// log either way. `CLAUDE.md` says nothing under `kernel/` locks, and a
/// test-and-skip with no waiter is not a lock; a `while` around it would be.
static DRAWING: AtomicBool = AtomicBool::new(false);

/// Put a string on the screen, if there is one.
pub fn write_str(text: &str) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    if DRAWING.swap(true, Ordering::Acquire) {
        return;
    }

    // SAFETY: the swap above establishes that no other core and no outer frame
    // of this one is inside the drawing path, so this reference is not aliased
    // for as long as it is held — which is until the store below, inside this
    // function.
    let console = unsafe { console() };

    let mut ended_a_line = false;
    for byte in text.bytes() {
        console.put(byte);
        ended_a_line |= byte == b'\n';
    }

    // Once per line rather than once per character. A flush is a scan of the
    // grid in cached memory, and doing one per byte would multiply the cost of
    // the cheap half by the length of the line.
    if ended_a_line {
        console.flush();
    }

    DRAWING.store(false, Ordering::Release);
}

/// The serial port and the screen, as one place to write.
///
/// The order is not arbitrary: the serial port is written first, so a line that
/// provokes a fault inside the drawing path has already left the machine by the
/// wire that has never needed a mapping to work.
pub struct Tee;

impl fmt::Write for Tee {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut serial = crate::arch::x86_64::serial::Serial;
        fmt::Write::write_str(&mut serial, s)?;
        write_str(s);
        Ok(())
    }
}

/// Render every glyph to the serial port, as characters.
///
/// This is how the font was checked, and it is kept because it is the only way
/// it *can* be checked on a machine with no display: the table is data somebody
/// typed, and a mistyped row is a letter that is subtly wrong in a way no
/// compiler and no test of the drawing path would notice. Reading it back as
/// shapes is the check.
pub fn dump_font() {
    for ch in FIRST..=LAST {
        crate::kprintln!("  glyph  {ch:#04x}  {}", ch as char);
        for row in glyph(ch) {
            let mut line = [b'.'; 5];
            for (index, cell) in line.iter_mut().enumerate() {
                if row & (1 << (4 - index)) != 0 {
                    *cell = b'#';
                }
            }
            crate::kprintln!("           {}", core::str::from_utf8(&line).unwrap_or("?????"));
        }
    }
}

/// Draw into an ordinary array and read the pixels back as characters.
///
/// The whole path except the last store lands on memory this function owns, so
/// it runs on any machine — including every boot under the emulator, where the
/// loader hands over no framebuffer at all and the real path is therefore never
/// taken. What it proves is what a display cannot be asked: that a string put
/// into the grid comes out of [`Surface::cell`] as the right pixels, at the
/// right scale, in the right places.
pub fn selftest(text: &str) {
    /// Wide enough for a dozen characters, which is as much as a check needs to
    /// read back. It is a stack array, and the kernel stack is sixty-four
    /// kibibytes — so this is sized by what the stack can hold rather than by
    /// what would be nice to see. Unit: pixels.
    const W: u32 = 144;
    /// One row of cells. Unit: pixels.
    const H: u32 = (GLYPH_H * SCALE) as u32;

    let mut pixels = [0u32; (W * H) as usize];
    let surface = Surface::over_memory(pixels.as_mut_ptr() as u64, W, H);

    for (col, ch) in text.bytes().enumerate() {
        surface.cell(col, 0, ch);
    }

    crate::kprintln!("  screen        self-test, {} x {} pixels, scale {SCALE}", W, H);
    for y in 0..H {
        let mut line = [b'.'; W as usize];
        for x in 0..W {
            if pixels[(y * W + x) as usize] != 0 {
                line[x as usize] = b'#';
            }
        }
        crate::kprintln!("    {}", core::str::from_utf8(&line).unwrap_or("?"));
    }
}
