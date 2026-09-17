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

use crate::arch::x86_64::multiboot::{Framebuffer, FramebufferKind};

/// Glyph box width. Unit: pixels.
const GLYPH_W: usize = 8;

/// Glyph box height. Unit: pixels.
///
/// Eight by sixteen, which is the size a text console has been on this
/// architecture since the VGA adapter and is what a Linux console still uses
/// today. That is not nostalgia: it is the cell that has had the most eyes on
/// it, and at 1024 by 768 it gives 128 columns by 48 rows — the shape a
/// terminal is expected to be, rather than the shape a font happened to make.
///
/// **This replaced a five-by-seven drawn at double size**, which was legible
/// and looked like a calculator. Sixteen rows is what buys real ascenders,
/// descenders that clear the baseline, and a `1` that cannot be read as an `l`.
const GLYPH_H: usize = 16;

/// How many screen pixels one glyph pixel becomes, on each axis.
///
/// One, on anything a machine of this era actually has. The font is now a
/// full-size cell rather than a small one that needed doubling, so a 1080p
/// panel gets 240 by 67 characters — which is exactly what the same panel shows
/// under Linux, because it is the same cell.
///
/// The exception is a display dense enough that a sixteen-pixel cell stops
/// being readable at arm's length. Past 2560 pixels across, one cell becomes
/// two, which on a 4K panel gives 240 by 67 again — the same screen, on a
/// display with four times the pixels.
fn scale_for(width: u32) -> usize {
    if width >= 2560 { 2 } else { 1 }
}

/// The widest grid this console will keep, in characters.
///
/// Two hundred and fifty-six covers 1920 across at one-to-one, with room past
/// it. A surface wider than this is *clipped*, not refused: the console is a
/// fallback for reading a boot, and half a line on the screen beats a refusal
/// on a machine whose only other output is the serial port that is missing.
const MAX_COLS: usize = 256;

/// The tallest grid this console will keep, in characters.
///
/// Ninety-six is 1536 pixels of sixteen-row cells, which covers every mode a
/// firmware is likely to hand over at one-to-one and every 4K mode at two.
const MAX_ROWS: usize = 96;

/// Cells in the grid.
const CELLS: usize = MAX_COLS * MAX_ROWS;

/// How many rows the screen moves when it has to move at all.
///
/// # Why a block and not a line
///
/// Because a fixed framebuffer cannot pan. A display driver scrolls by moving
/// the address the scanout reads from, which costs nothing; this console has no
/// such address to move, so *scrolling* means every line's pixels are drawn
/// again one row higher. Once the screen is full, a one-row scroll makes every
/// new line cost a full redraw — and a boot prints two hundred lines onto a
/// screen that holds sixty-seven.
///
/// Scrolling eight rows at once pays that redraw one time in eight. The seven
/// lines in between land on rows that were left blank by the jump, and cost one
/// row of drawing each.
///
/// What it costs is that the log advances in steps rather than smoothly, and
/// that up to seven rows at the bottom are blank while a block fills. Neither
/// costs a reader anything that matters: **the newest line is always the last
/// one drawn**, so a machine that stops has its final line on the screen, which
/// is the whole reason somebody is looking at it.
///
/// This is jump scrolling, which terminals did when they had the same problem
/// for the same reason. *Reversal:* a display whose scanout address this system
/// can move makes scrolling free and this constant meaningless — that is the
/// component, and RFC 0085 names it as where this stops being the algorithm.
const SCROLL_BLOCK: usize = 8;

/// How many unchanged cells a run of damage will cross rather than break at.
///
/// A run is redrawn as one sweep of consecutive stores, and stopping it costs
/// the sweep: the next run starts a new burst, and the processor's fill buffer
/// takes the gap as a reason to drain early. Redrawing a cell that did not
/// change costs the stores for one character and nothing else — it is
/// idempotent, since what is drawn is what the grid already said.
///
/// So a short gap is cheaper to paint over than to stop for. Four cells is
/// thirty-two pixels, which is half of one fill buffer; a gap wider than that
/// is worth the break. RFC 0027's rule, applied where the tree already applies
/// it: coalescing is a pass over the damage, not a lookup per cell.
const RUN_GAP: usize = 4;

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

/// The font: an eight-by-sixteen cell, ninety-five glyphs, ASCII order from
/// [`FIRST`].
///
/// Sixteen bytes per glyph, one per row, top row first. Bit 7 is the leftmost
/// pixel and bit 0 the rightmost — the storage order every eight-wide bitmap
/// font on this architecture has used, which is why a row reads as a hexadecimal
/// byte rather than as a binary literal somebody has to squint at.
///
/// The shapes follow the proportions of the console font this architecture has
/// had since the VGA adapter, because that is what a reader's eye expects a
/// machine to boot in and because those proportions have had more hours of
/// reading than any other bitmap: caps ten rows tall from row 2, x-height six
/// rows from row 5, descenders through row 14, and the rightmost column left
/// clear so adjacent characters do not touch.
///
/// **This table is original work and is deliberately not an imported font.**
/// `LICENSING.md` allows the permissive tree and `third_party/`, and permits
/// nothing in the second to be reached from the first except over a ring. There
/// is no category for imported *data* under a permissive licence, which is
/// exactly what a font is: Terminus is SIL OFL and would be welcome in a
/// component, and cannot go in `kernel/src/` without putting a licence that is
/// not this tree's into this tree. GNU Unifont is GPL and would take the frame
/// with it. RFC 0081 records that gap as the thing to close before a component
/// draws, rather than as a thing to route around here.
///
/// [`dump_font`] is how it is checked, and the check is not optional: a table
/// somebody typed is a table somebody mistyped, and a wrong row is a letter
/// that is subtly not that letter — which no compiler and no test of the
/// drawing path would notice. `cargo xtask screen font` reads all ninety-five
/// back as shapes on a serial line.
/// One line per glyph, which `rustfmt` would turn into six. The attribute is
/// the same one `abi/src/scene.rs` uses for its fixtures and for the same
/// reason: this is a picture stored as numbers, and a formatter that wrapped it
/// would be wrapping the picture.
#[rustfmt::skip]
static GLYPHS: [[u8; GLYPH_H]; 95] = [
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // space
    [0x00,0x00,0x18,0x3C,0x3C,0x3C,0x18,0x18,0x18,0x00,0x18,0x18,0x00,0x00,0x00,0x00], // !
    [0x00,0x00,0x66,0x66,0x66,0x24,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // "
    [0x00,0x00,0x00,0x6C,0x6C,0xFE,0x6C,0x6C,0x6C,0xFE,0x6C,0x6C,0x00,0x00,0x00,0x00], // #
    [0x00,0x18,0x18,0x7C,0xC6,0xC2,0xC0,0x7C,0x06,0x86,0xC6,0x7C,0x18,0x18,0x00,0x00], // $
    [0x00,0x00,0x00,0x00,0xC2,0xC6,0x0C,0x18,0x30,0x60,0xC6,0x86,0x00,0x00,0x00,0x00], // %
    [0x00,0x00,0x38,0x6C,0x6C,0x38,0x76,0xDC,0xCC,0xCC,0xCC,0x76,0x00,0x00,0x00,0x00], // &
    [0x00,0x00,0x30,0x30,0x30,0x60,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // '
    [0x00,0x00,0x0C,0x18,0x30,0x30,0x30,0x30,0x30,0x18,0x0C,0x00,0x00,0x00,0x00,0x00], // (
    [0x00,0x00,0x30,0x18,0x0C,0x0C,0x0C,0x0C,0x0C,0x18,0x30,0x00,0x00,0x00,0x00,0x00], // )
    [0x00,0x00,0x00,0x00,0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00,0x00,0x00,0x00,0x00], // *
    [0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x7E,0x18,0x18,0x00,0x00,0x00,0x00,0x00,0x00], // +
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x18,0x30,0x00,0x00], // ,
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFE,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // -
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x00,0x00,0x00,0x00], // .
    [0x00,0x00,0x02,0x06,0x0C,0x18,0x30,0x60,0xC0,0x80,0x00,0x00,0x00,0x00,0x00,0x00], // /
    [0x00,0x00,0x38,0x6C,0xC6,0xC6,0xD6,0xD6,0xC6,0xC6,0x6C,0x38,0x00,0x00,0x00,0x00], // 0
    [0x00,0x00,0x18,0x38,0x78,0x18,0x18,0x18,0x18,0x18,0x18,0x7E,0x00,0x00,0x00,0x00], // 1
    [0x00,0x00,0x7C,0xC6,0x06,0x0C,0x18,0x30,0x60,0xC0,0xC6,0xFE,0x00,0x00,0x00,0x00], // 2
    [0x00,0x00,0x7C,0xC6,0x06,0x06,0x3C,0x06,0x06,0x06,0xC6,0x7C,0x00,0x00,0x00,0x00], // 3
    [0x00,0x00,0x0C,0x1C,0x3C,0x6C,0xCC,0xFE,0x0C,0x0C,0x0C,0x1E,0x00,0x00,0x00,0x00], // 4
    [0x00,0x00,0xFE,0xC0,0xC0,0xC0,0xFC,0x06,0x06,0x06,0xC6,0x7C,0x00,0x00,0x00,0x00], // 5
    [0x00,0x00,0x38,0x60,0xC0,0xC0,0xFC,0xC6,0xC6,0xC6,0xC6,0x7C,0x00,0x00,0x00,0x00], // 6
    [0x00,0x00,0xFE,0xC6,0x06,0x06,0x0C,0x18,0x30,0x30,0x30,0x30,0x00,0x00,0x00,0x00], // 7
    [0x00,0x00,0x7C,0xC6,0xC6,0xC6,0x7C,0xC6,0xC6,0xC6,0xC6,0x7C,0x00,0x00,0x00,0x00], // 8
    [0x00,0x00,0x7C,0xC6,0xC6,0xC6,0x7E,0x06,0x06,0x06,0x0C,0x78,0x00,0x00,0x00,0x00], // 9
    [0x00,0x00,0x00,0x00,0x18,0x18,0x00,0x00,0x00,0x18,0x18,0x00,0x00,0x00,0x00,0x00], // :
    [0x00,0x00,0x00,0x00,0x18,0x18,0x00,0x00,0x00,0x18,0x18,0x30,0x00,0x00,0x00,0x00], // ;
    [0x00,0x00,0x06,0x0C,0x18,0x30,0x60,0x30,0x18,0x0C,0x06,0x00,0x00,0x00,0x00,0x00], // <
    [0x00,0x00,0x00,0x00,0x00,0x7E,0x00,0x00,0x7E,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // =
    [0x00,0x00,0x60,0x30,0x18,0x0C,0x06,0x0C,0x18,0x30,0x60,0x00,0x00,0x00,0x00,0x00], // >
    [0x00,0x00,0x7C,0xC6,0xC6,0x0C,0x18,0x18,0x18,0x00,0x18,0x18,0x00,0x00,0x00,0x00], // ?
    [0x00,0x00,0x7C,0xC6,0xC6,0xDE,0xDE,0xDE,0xDC,0xC0,0x7C,0x00,0x00,0x00,0x00,0x00], // @
    [0x00,0x00,0x10,0x38,0x6C,0xC6,0xC6,0xFE,0xC6,0xC6,0xC6,0xC6,0x00,0x00,0x00,0x00], // A
    [0x00,0x00,0xFC,0x66,0x66,0x66,0x7C,0x66,0x66,0x66,0x66,0xFC,0x00,0x00,0x00,0x00], // B
    [0x00,0x00,0x3C,0x66,0xC2,0xC0,0xC0,0xC0,0xC0,0xC2,0x66,0x3C,0x00,0x00,0x00,0x00], // C
    [0x00,0x00,0xF8,0x6C,0x66,0x66,0x66,0x66,0x66,0x66,0x6C,0xF8,0x00,0x00,0x00,0x00], // D
    [0x00,0x00,0xFE,0x66,0x62,0x68,0x78,0x68,0x60,0x62,0x66,0xFE,0x00,0x00,0x00,0x00], // E
    [0x00,0x00,0xFE,0x66,0x62,0x68,0x78,0x68,0x60,0x60,0x60,0xF0,0x00,0x00,0x00,0x00], // F
    [0x00,0x00,0x3C,0x66,0xC2,0xC0,0xC0,0xDE,0xC6,0xC6,0x66,0x3A,0x00,0x00,0x00,0x00], // G
    [0x00,0x00,0xC6,0xC6,0xC6,0xC6,0xFE,0xC6,0xC6,0xC6,0xC6,0xC6,0x00,0x00,0x00,0x00], // H
    [0x00,0x00,0x3C,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x3C,0x00,0x00,0x00,0x00], // I
    [0x00,0x00,0x1E,0x0C,0x0C,0x0C,0x0C,0x0C,0xCC,0xCC,0xCC,0x78,0x00,0x00,0x00,0x00], // J
    [0x00,0x00,0xE6,0x66,0x66,0x6C,0x78,0x78,0x6C,0x66,0x66,0xE6,0x00,0x00,0x00,0x00], // K
    [0x00,0x00,0xF0,0x60,0x60,0x60,0x60,0x60,0x60,0x62,0x66,0xFE,0x00,0x00,0x00,0x00], // L
    [0x00,0x00,0xC6,0xEE,0xFE,0xFE,0xD6,0xC6,0xC6,0xC6,0xC6,0xC6,0x00,0x00,0x00,0x00], // M
    [0x00,0x00,0xC6,0xE6,0xF6,0xFE,0xDE,0xCE,0xC6,0xC6,0xC6,0xC6,0x00,0x00,0x00,0x00], // N
    [0x00,0x00,0x38,0x6C,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0x6C,0x38,0x00,0x00,0x00,0x00], // O
    [0x00,0x00,0xFC,0x66,0x66,0x66,0x7C,0x60,0x60,0x60,0x60,0xF0,0x00,0x00,0x00,0x00], // P
    [0x00,0x00,0x38,0x6C,0xC6,0xC6,0xC6,0xC6,0xC6,0xD6,0xDE,0x7C,0x0C,0x0E,0x00,0x00], // Q
    [0x00,0x00,0xFC,0x66,0x66,0x66,0x7C,0x6C,0x66,0x66,0x66,0xE6,0x00,0x00,0x00,0x00], // R
    [0x00,0x00,0x7C,0xC6,0xC6,0x60,0x38,0x0C,0x06,0xC6,0xC6,0x7C,0x00,0x00,0x00,0x00], // S
    [0x00,0x00,0x7E,0x7E,0x5A,0x18,0x18,0x18,0x18,0x18,0x18,0x3C,0x00,0x00,0x00,0x00], // T
    [0x00,0x00,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0x7C,0x00,0x00,0x00,0x00], // U
    [0x00,0x00,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6,0x6C,0x38,0x10,0x00,0x00,0x00,0x00], // V
    [0x00,0x00,0xC6,0xC6,0xC6,0xC6,0xD6,0xD6,0xD6,0xFE,0xEE,0x6C,0x00,0x00,0x00,0x00], // W
    [0x00,0x00,0xC6,0xC6,0x6C,0x7C,0x38,0x38,0x7C,0x6C,0xC6,0xC6,0x00,0x00,0x00,0x00], // X
    [0x00,0x00,0x66,0x66,0x66,0x66,0x3C,0x18,0x18,0x18,0x18,0x3C,0x00,0x00,0x00,0x00], // Y
    [0x00,0x00,0xFE,0xC6,0x86,0x0C,0x18,0x30,0x60,0xC2,0xC6,0xFE,0x00,0x00,0x00,0x00], // Z
    [0x00,0x00,0x3C,0x30,0x30,0x30,0x30,0x30,0x30,0x30,0x30,0x3C,0x00,0x00,0x00,0x00], // [
    [0x00,0x00,0x80,0xC0,0x60,0x30,0x18,0x0C,0x06,0x02,0x00,0x00,0x00,0x00,0x00,0x00], // backslash
    [0x00,0x00,0x3C,0x0C,0x0C,0x0C,0x0C,0x0C,0x0C,0x0C,0x0C,0x3C,0x00,0x00,0x00,0x00], // ]
    [0x10,0x38,0x6C,0xC6,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // ^
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF,0x00], // _
    [0x18,0x18,0x0C,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // `
    [0x00,0x00,0x00,0x00,0x00,0x78,0x0C,0x7C,0xCC,0xCC,0xCC,0x76,0x00,0x00,0x00,0x00], // a
    [0x00,0x00,0xE0,0x60,0x60,0x78,0x6C,0x66,0x66,0x66,0x66,0x7C,0x00,0x00,0x00,0x00], // b
    [0x00,0x00,0x00,0x00,0x00,0x7C,0xC6,0xC0,0xC0,0xC0,0xC6,0x7C,0x00,0x00,0x00,0x00], // c
    [0x00,0x00,0x1C,0x0C,0x0C,0x3C,0x6C,0xCC,0xCC,0xCC,0xCC,0x76,0x00,0x00,0x00,0x00], // d
    [0x00,0x00,0x00,0x00,0x00,0x7C,0xC6,0xFE,0xC0,0xC0,0xC6,0x7C,0x00,0x00,0x00,0x00], // e
    [0x00,0x00,0x38,0x6C,0x64,0x60,0xF0,0x60,0x60,0x60,0x60,0xF0,0x00,0x00,0x00,0x00], // f
    [0x00,0x00,0x00,0x00,0x00,0x76,0xCC,0xCC,0xCC,0xCC,0xCC,0x7C,0x0C,0xCC,0x78,0x00], // g
    [0x00,0x00,0xE0,0x60,0x60,0x6C,0x76,0x66,0x66,0x66,0x66,0xE6,0x00,0x00,0x00,0x00], // h
    [0x00,0x00,0x18,0x18,0x00,0x38,0x18,0x18,0x18,0x18,0x18,0x3C,0x00,0x00,0x00,0x00], // i
    [0x00,0x00,0x06,0x06,0x00,0x0E,0x06,0x06,0x06,0x06,0x06,0x66,0x66,0x3C,0x00,0x00], // j
    [0x00,0x00,0xE0,0x60,0x60,0x66,0x6C,0x78,0x78,0x6C,0x66,0xE6,0x00,0x00,0x00,0x00], // k
    [0x00,0x00,0x38,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x3C,0x00,0x00,0x00,0x00], // l
    [0x00,0x00,0x00,0x00,0x00,0xEC,0xFE,0xD6,0xD6,0xD6,0xD6,0xC6,0x00,0x00,0x00,0x00], // m
    [0x00,0x00,0x00,0x00,0x00,0xDC,0x66,0x66,0x66,0x66,0x66,0x66,0x00,0x00,0x00,0x00], // n
    [0x00,0x00,0x00,0x00,0x00,0x7C,0xC6,0xC6,0xC6,0xC6,0xC6,0x7C,0x00,0x00,0x00,0x00], // o
    [0x00,0x00,0x00,0x00,0x00,0xDC,0x66,0x66,0x66,0x66,0x66,0x7C,0x60,0x60,0xF0,0x00], // p
    [0x00,0x00,0x00,0x00,0x00,0x76,0xCC,0xCC,0xCC,0xCC,0xCC,0x7C,0x0C,0x0C,0x1E,0x00], // q
    [0x00,0x00,0x00,0x00,0x00,0xDC,0x76,0x66,0x60,0x60,0x60,0xF0,0x00,0x00,0x00,0x00], // r
    [0x00,0x00,0x00,0x00,0x00,0x7C,0xC6,0x60,0x38,0x0C,0xC6,0x7C,0x00,0x00,0x00,0x00], // s
    [0x00,0x00,0x10,0x30,0x30,0xFC,0x30,0x30,0x30,0x30,0x36,0x1C,0x00,0x00,0x00,0x00], // t
    [0x00,0x00,0x00,0x00,0x00,0xCC,0xCC,0xCC,0xCC,0xCC,0xCC,0x76,0x00,0x00,0x00,0x00], // u
    [0x00,0x00,0x00,0x00,0x00,0xC6,0xC6,0xC6,0xC6,0x6C,0x38,0x10,0x00,0x00,0x00,0x00], // v
    [0x00,0x00,0x00,0x00,0x00,0xC6,0xC6,0xD6,0xD6,0xD6,0xFE,0x6C,0x00,0x00,0x00,0x00], // w
    [0x00,0x00,0x00,0x00,0x00,0xC6,0x6C,0x38,0x38,0x38,0x6C,0xC6,0x00,0x00,0x00,0x00], // x
    [0x00,0x00,0x00,0x00,0x00,0xC6,0xC6,0xC6,0xC6,0xC6,0x7E,0x06,0x0C,0xF8,0x00,0x00], // y
    [0x00,0x00,0x00,0x00,0x00,0xFE,0xCC,0x18,0x30,0x60,0xC6,0xFE,0x00,0x00,0x00,0x00], // z
    [0x00,0x00,0x0E,0x18,0x18,0x18,0x70,0x18,0x18,0x18,0x18,0x0E,0x00,0x00,0x00,0x00], // {
    [0x00,0x00,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x00,0x00,0x00,0x00], // |
    [0x00,0x00,0x70,0x18,0x18,0x18,0x0E,0x18,0x18,0x18,0x18,0x70,0x00,0x00,0x00,0x00], // }
    [0x00,0x00,0x76,0xDC,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // ~
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
    /// How many screen pixels one glyph pixel becomes, on each axis. Carried on
    /// the surface rather than being a constant, because it is a property of
    /// the display that was handed over. [`scale_for`] decides it.
    scale: usize,
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
        let ink = match fb.kind {
            // **Every bit of every channel, rather than a chosen colour**, and
            // the reason is a screen that came back the wrong hue. The first
            // version of this picked a light grey — a considered `(0xC8, 0xCF,
            // 0xD8)` — packed through the layout the loader reported, and the
            // VMware guest drew the log in yellow-green. Yellow-green is red
            // plus green with no blue, which is what a packed colour looks like
            // when one channel's position or width is not what the code
            // believed. The colour was carrying an assumption about a structure
            // the loader wrote, and it was the only part of this file that
            // could be wrong *quietly*: a misplaced glyph is visible, and a
            // misplaced channel is just a colour somebody might have chosen.
            //
            // Saturating every channel removes the assumption rather than
            // fixing it. All bits set in each reported field is white under any
            // layout, any width and any order — there is no arrangement of
            // three masks that makes it something else — so the worst a
            // misreported channel can now do is leave that channel dark, which
            // is a tint rather than a lie. `intent/0011` still asks for the
            // channel line to be read back off the machine; this makes the
            // screen legible whatever it says.
            //
            // White on black is also what the console this is compared against
            // does, which is the other half of the argument.
            FramebufferKind::Direct(channels) => channels.saturated(),
            // Indexed, text and unknown kinds get the two values every palette
            // agrees about at its ends. This console does not load a palette —
            // on a mode it did not ask for, the honest thing is to draw in
            // whatever *is* black and white rather than to guess an index.
            _ => u32::MAX,
        };

        Self {
            at,
            pitch: fb.pitch,
            width: fb.width,
            height: fb.height,
            bytes_per_pixel,
            scale: scale_for(fb.width),
            ink,
            // Black is zero in every direct-colour layout and at one end of
            // every palette, so it needs none of the care the ink needed.
            paper: 0,
        }
    }

    /// Describe a surface over memory the caller owns.
    ///
    /// For [`selftest`], and the two colours are the ones its dump reads back.
    /// Scale one, because the check reads pixels back as characters and
    /// doubling every one of them would only make the dump twice as wide.
    fn over_memory(at: u64, width: u32, height: u32) -> Self {
        Self {
            at,
            pitch: width * 4,
            width,
            height,
            bytes_per_pixel: 4,
            scale: 1,
            ink: u32::MAX,
            paper: 0,
        }
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
        let origin_x = col * GLYPH_W * self.scale;
        let origin_y = row * GLYPH_H * self.scale;

        // The fast path, and it is the one every cell of a real boot takes: a
        // cell wholly on the surface, four bytes to a pixel, drawn at
        // one-to-one. `put` is correct for all of that and pays for it per
        // pixel — a bounds test, two multiplies to rebuild an address the
        // previous pixel was next to, and a match on a depth that cannot change
        // during a boot. A hundred and twenty-eight times a character, on a
        // screen that redraws sixteen thousand of them when it scrolls.
        //
        // What makes it worth separating rather than optimising in place is the
        // *store pattern*. Hoisting the row address turns a glyph row into
        // eight stores to consecutive addresses, which is exactly the shape
        // write-combining exists to gather: the processor fills one buffer and
        // puts it out as a burst. Scattered stores to the same page do not
        // combine, so the mapping change and this loop are one improvement
        // rather than two.
        let fits = self.scale == 1
            && self.bytes_per_pixel == 4
            && origin_x + GLYPH_W <= self.width as usize
            && origin_y + GLYPH_H <= self.height as usize;

        if fits {
            for (dy, bits) in rows.iter().enumerate() {
                let base =
                    self.at + (origin_y + dy) as u64 * u64::from(self.pitch) + origin_x as u64 * 4;
                for dx in 0..GLYPH_W {
                    // Bit 7 is the leftmost pixel of the row: the byte is the
                    // row, most significant bit first, which is how every
                    // eight-wide bitmap font on this architecture is stored.
                    let value = if bits & (1 << (7 - dx)) != 0 { self.ink } else { self.paper };
                    // SAFETY: the surface is four bytes to a pixel and this
                    // whole cell was tested against its width and height above,
                    // so this address is inside it. Four-byte aligned because
                    // the surface's first pixel is page-aligned and every term
                    // added to it is a multiple of four. Volatile because a
                    // display is a device: the compiler may not drop, reorder
                    // or merge a write to it.
                    unsafe { ((base + dx as u64 * 4) as *mut u32).write_volatile(value) };
                }
            }
            return;
        }

        // Everything else: a clipped cell at the edge of a screen whose size is
        // not a multiple of the glyph, a depth that is not four bytes, or a
        // scale of two. `put` clips and packs, one pixel at a time.
        for (dy, bits) in rows.iter().enumerate() {
            for dx in 0..GLYPH_W {
                let value = if bits & (1 << (7 - dx)) != 0 { self.ink } else { self.paper };
                for sy in 0..self.scale {
                    for sx in 0..self.scale {
                        let x = (origin_x + dx * self.scale + sx) as u32;
                        let y = (origin_y + dy * self.scale + sy) as u32;
                        self.put(x, y, value);
                    }
                }
            }
        }
    }

    /// Draw a row of adjacent characters as one sweep.
    ///
    /// # Why this is not a loop over [`Surface::cell`]
    ///
    /// Because of where the stores land. Drawing one character writes eight
    /// pixels, jumps a whole scanline to the next glyph row, and writes eight
    /// more — sixteen bursts of thirty-two bytes, each a long way from the
    /// last. The mapping is write-combining, and write-combining gathers
    /// *consecutive* stores: a burst that ends after thirty-two bytes leaves
    /// half a fill buffer, and the jump to the next row is the processor's
    /// reason to flush it.
    ///
    /// Drawing sixty characters as a run inverts the loops. Each glyph row
    /// becomes one sweep of four hundred and eighty pixels — nineteen hundred
    /// contiguous bytes, which is thirty full buffers in a row — and there are
    /// sixteen sweeps rather than nine hundred and sixty bursts. The pixels
    /// written are identical; only their order changes.
    ///
    /// The glyph lookup moves inside the inner loop, which is one indexed read
    /// per character per row out of a table that is never out of cache, against
    /// stores to memory on the far side of the bus. It is not the expensive
    /// part and it is not close.
    fn run(self, col: usize, row: usize, chars: &[u8]) {
        let origin_x = col * GLYPH_W * self.scale;
        let origin_y = row * GLYPH_H * self.scale;
        let span = chars.len() * GLYPH_W * self.scale;

        let fits = self.scale == 1
            && self.bytes_per_pixel == 4
            && origin_x + span <= self.width as usize
            && origin_y + GLYPH_H <= self.height as usize;

        if !fits {
            // A clipped run at the edge of a screen, a depth this cannot write
            // wide, or a doubled scale. `cell` is correct for all of them and
            // the sweep is an optimisation rather than the meaning.
            for (index, &ch) in chars.iter().enumerate() {
                self.cell(col + index, row, ch);
            }
            return;
        }

        for dy in 0..GLYPH_H {
            let mut at =
                self.at + (origin_y + dy) as u64 * u64::from(self.pitch) + origin_x as u64 * 4;
            for &ch in chars {
                let bits = glyph(ch)[dy];
                for dx in 0..GLYPH_W {
                    let value = if bits & (1 << (7 - dx)) != 0 { self.ink } else { self.paper };
                    // SAFETY: four bytes to a pixel, and the whole run was
                    // tested against the surface's width and height above, so
                    // every address this walks is inside it. Four-byte aligned
                    // because the first pixel is page-aligned and every term
                    // added to it is a multiple of four. Volatile because a
                    // display is a device.
                    unsafe { (at as *mut u32).write_volatile(value) };
                    at += 4;
                }
            }
        }
    }

    /// Paint the whole surface in [`Surface::paper`].
    ///
    /// # Why this is not sixteen thousand blank glyphs
    ///
    /// Because that is what it replaced. The console starts by believing
    /// nothing has been drawn, so its first flush used to draw *every* cell —
    /// including the eleven-twelfths of a boot screen that are blank — as a
    /// glyph, eight pixels at a time with a jump to the next row between them.
    /// Two million stores in sixteen thousand strided bursts.
    ///
    /// This is the same two million stores in one sweep from the first byte of
    /// the surface to the last, which is the access pattern write-combining is
    /// best at and the one a display's memory is happiest with. Afterwards the
    /// console records the screen as blank rather than as unknown, so the first
    /// flush draws the handful of cells that actually have text in them.
    fn clear(self) {
        if self.bytes_per_pixel != 4 {
            // The slow path is correct and this is a start-up cost paid once,
            // so a depth the fast sweep cannot write is left to `cell`: the
            // console will paint blanks over it on the first flush.
            return;
        }
        for y in 0..self.height {
            let base = self.at + u64::from(y) * u64::from(self.pitch);
            for x in 0..self.width {
                // SAFETY: inside the surface by construction — `y` and `x` are
                // bounded by its own height and width, and its extent was
                // validated before it was mapped. Aligned and volatile for the
                // reasons `cell`'s fast path gives.
                unsafe { ((base + u64::from(x) * 4) as *mut u32).write_volatile(self.paper) };
            }
        }
    }
}

/// The nearest character this font has to `ch`.
///
/// # Why a fold and not a wider font
///
/// The alternative is to carry glyphs for the punctuation this tree writes,
/// which means carrying a lookup from code point to glyph, which means the font
/// stops being an array indexed by arithmetic. For a fallback console that
/// exists to be read during a boot, the dash somebody typed and the dash this
/// draws being *different dashes* costs the reader nothing; the character being
/// absent costs them the line.
///
/// So the folds are the ones this repository's own prose actually uses, and
/// everything else outside ASCII becomes [`REPLACEMENT`] — one mark for one
/// character, which is the part that matters. The list is short on purpose: a
/// fold nobody needs is a row somebody has to check.
fn fold(ch: char) -> u8 {
    match ch {
        // ASCII passes through untouched, which is every byte of a boot log
        // that is not punctuation somebody reached for.
        '\n' | '\r' | '\t' => ch as u8,
        ' '..='~' => ch as u8,
        // The dashes. An em dash separates clauses all through this tree's
        // output and an en dash turns up in ranges; both read as a hyphen.
        '\u{2014}' | '\u{2013}' | '\u{2212}' => b'-',
        // Quotation marks, which arrive whenever a sentence quotes a term.
        '\u{2018}' | '\u{2019}' => b'\'',
        '\u{201C}' | '\u{201D}' => b'"',
        // The ellipsis, which is one character and three dots. Only the first
        // is drawn: a fold that grew a cell would put the grid out of step with
        // the column the caller counted on.
        '\u{2026}' => b'.',
        // Units and arithmetic the boot report prints: microseconds, a
        // multiplication sign between dimensions, a middle dot between fields.
        '\u{00B5}' | '\u{03BC}' => b'u',
        '\u{00D7}' => b'x',
        '\u{00B7}' => b'.',
        // A non-breaking space is a space. Nothing about this grid breaks
        // lines, so the distinction has nowhere to land.
        '\u{00A0}' => b' ',
        _ => REPLACEMENT,
    }
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
        self.cols = (surface.width as usize / (GLYPH_W * surface.scale)).min(MAX_COLS);
        self.rows = (surface.height as usize / (GLYPH_H * surface.scale)).min(MAX_ROWS);
        self.col = 0;
        self.row = 0;
        self.want = [b' '; CELLS];
        // The surface holds whatever the firmware left in it, so something has
        // to paint over it. `clear` is that something — one sweep — and because
        // it has run, the screen really *is* blank and the console may record
        // it as such. Before `clear` existed this said `[0; CELLS]`, which
        // meant every cell differed from what had been drawn and the first
        // flush painted sixteen thousand blank glyphs to reach the same state.
        surface.clear();
        self.have = [b' '; CELLS];
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
            return;
        }

        // The screen is full, so it moves — by a block rather than a line, for
        // the reason [`SCROLL_BLOCK`] gives. Clamped, because a grid shorter
        // than the block still has to advance by something and one row is what
        // is left when there is no room for eight.
        let by = SCROLL_BLOCK.clamp(1, self.rows.saturating_sub(1).max(1));
        self.scroll(by);
        self.row = self.rows - by;
    }

    /// Shift the text up `by` rows. The surface is not touched.
    ///
    /// This is the half of scrolling that is free: two array walks in ordinary
    /// cached memory. What costs is the redraw it implies, which is why the
    /// caller does this rarely rather than often.
    fn scroll(&mut self, by: usize) {
        for row in by..self.rows {
            let (above, here) = ((row - by) * MAX_COLS, row * MAX_COLS);
            for col in 0..self.cols {
                self.want[above + col] = self.want[here + col];
            }
        }
        for row in self.rows.saturating_sub(by)..self.rows {
            let at = row * MAX_COLS;
            for col in 0..self.cols {
                self.want[at + col] = b' ';
            }
        }
    }

    /// Bring the surface level with the text, writing only what differs.
    fn flush(&mut self) {
        let Some(surface) = self.surface else { return };
        for row in 0..self.rows {
            let base = row * MAX_COLS;
            let mut col = 0;
            while col < self.cols {
                if self.want[base + col] == self.have[base + col] {
                    col += 1;
                    continue;
                }

                // One past the last cell known to differ, and a scan that runs
                // ahead of it. The run keeps going across a gap of unchanged
                // cells shorter than `RUN_GAP` — painting over them, which is
                // idempotent — and ends at the last difference rather than
                // wherever the scan stopped, so no run has a tail of cells that
                // did not need drawing.
                let start = col;
                let mut end = col + 1;
                let mut scan = end;
                while scan < self.cols {
                    if self.want[base + scan] != self.have[base + scan] {
                        scan += 1;
                        end = scan;
                    } else if scan - end < RUN_GAP {
                        scan += 1;
                    } else {
                        break;
                    }
                }

                surface.run(start, row, &self.want[base + start..base + end]);
                for index in start..end {
                    self.have[base + index] = self.want[base + index];
                }
                col = end;
            }
        }

        // The display is write-combining now, which means a store to it may sit
        // in a fill buffer until the processor has a reason to drain one — and
        // *the machine stopped* is not among its reasons. Without this, the
        // last line before a hang is the line that never reaches the screen,
        // which is precisely the line somebody is looking at the screen to
        // find. One instruction, once per line of output.
        crate::arch::x86_64::paging::fence_stores();
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

    // Characters, not bytes, and it is the difference between a readable screen
    // and a littered one. This tree's log is written with typographic
    // punctuation — the em dash in `frame — 4 capabilit(ies) revoked` is in
    // almost every line the supervisor prints — and an em dash is three bytes
    // of UTF-8. Walking bytes put three replacement marks on the screen for one
    // character the reader can see is a dash, so the first real display this
    // console met showed `???` down the middle of half its lines. `&str` is
    // valid UTF-8 by construction, so `chars` is both the fix and the cheaper
    // spelling: no decoder, no state, no way to be mid-character at a flush.
    let mut ended_a_line = false;
    for ch in text.chars() {
        console.put(fold(ch));
        ended_a_line |= ch == '\n';
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

/// Redraw the whole screen, timed, and say what it cost.
///
/// # Why this is behind a boot parameter and not a line of the boot report
///
/// Because the boot log is a fixture: two runs of one `(seed, commit)` produce
/// byte-identical output, and `cargo xtask trace` hashes it. A number taken
/// from a real clock is different every run, so printing one unconditionally
/// would turn a check that catches real divergence into one that fails always.
/// Behind `screen=cost` it is a measurement somebody asked for.
///
/// # What it measures, and what it does not
///
/// It marks every cell as undrawn and flushes, so what is timed is a full
/// screen of glyphs — the worst thing this console ever does, and exactly what
/// a scroll costs when the text is dense. It does not measure the clear sweep
/// at attach, and it does not measure a typical line, which touches a few
/// dozen cells rather than every one of them.
///
/// The number is the point of comparison for the memory type: the same screen
/// drawn through an uncacheable mapping and through a write-combining one is
/// the whole argument for RFC 0085's change, and this is how a machine settles
/// it rather than a paragraph.
pub fn cost() {
    if !ENABLED.load(Ordering::Acquire) {
        crate::kprintln!("  screen        no surface, so nothing to measure");
        return;
    }
    if DRAWING.swap(true, Ordering::Acquire) {
        return;
    }

    // SAFETY: as `write_str` — the swap above establishes that nothing else is
    // inside the drawing path, and the reference is dropped before the store.
    let console = unsafe { console() };
    let (cols, rows) = (console.cols, console.rows);
    let cells = cols * rows;

    // Every cell differs from what is on the screen, which is what makes the
    // flush below a full redraw rather than a no-op.
    console.have = [0; CELLS];

    let before = crate::arch::x86_64::read_tsc();
    console.flush();
    let after = crate::arch::x86_64::read_tsc();

    DRAWING.store(false, Ordering::Release);

    let cycles = after.saturating_sub(before);
    let per_cell = if cells > 0 { cycles / cells as u64 } else { 0 };
    crate::kprintln!(
        "  screen        {cells} cell(s) redrawn in {cycles} cycle(s), {per_cell} per cell"
    );

    // Milliseconds when the timer has been calibrated, because cycles are a
    // ratio and a person wants a duration. Zero means this core has no
    // calibration yet, and saying so is better than dividing by it.
    let khz = crate::arch::x86_64::apic::tsc_khz();
    if let Some(us) = cycles.saturating_mul(1000).checked_div(khz) {
        crate::kprintln!("                {us} us at {khz} kHz");
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
            let mut line = [b'.'; GLYPH_W];
            for (index, cell) in line.iter_mut().enumerate() {
                if row & (1 << (7 - index)) != 0 {
                    *cell = b'#';
                }
            }
            crate::kprintln!("           {}", core::str::from_utf8(&line).unwrap_or("????????"));
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
    const W: u32 = 160;
    /// One row of cells. Unit: pixels.
    ///
    /// `over_memory` draws at scale one, so this is the cell height exactly.
    const H: u32 = GLYPH_H as u32;

    let mut pixels = [0u32; (W * H) as usize];
    let surface = Surface::over_memory(pixels.as_mut_ptr() as u64, W, H);

    for (col, ch) in text.chars().enumerate() {
        surface.cell(col, 0, fold(ch));
    }

    crate::kprintln!("  screen        self-test, {W} x {H} pixels, {GLYPH_W} x {GLYPH_H} cell");
    for y in 0..H {
        let mut line = [b'.'; W as usize];
        for x in 0..W {
            if pixels[(y * W + x) as usize] != 0 {
                line[x as usize] = b'#';
            }
        }
        crate::kprintln!("    {}", core::str::from_utf8(&line).unwrap_or("?"));
    }

    // The same characters again, through `run` this time, into a second buffer
    // — and the two must be identical byte for byte.
    //
    // This is the whole check on the sweep, and it is the right shape for it:
    // `run` exists only to write the pixels `cell` would have written, in a
    // different order, so *it agrees with `cell`* is not a proxy for
    // correctness, it is the definition. A faster loop that drew anything else
    // — off by a row, short by a column, packing the wrong value into the gap
    // between characters — shows up here as a byte that differs, on a check
    // that needs no display and no eye.
    let mut swept = [0u32; (W * H) as usize];
    let by_run = Surface::over_memory(swept.as_mut_ptr() as u64, W, H);
    let mut folded = [b' '; (W / GLYPH_W as u32) as usize];
    for (index, ch) in text.chars().enumerate() {
        if let Some(slot) = folded.get_mut(index) {
            *slot = fold(ch);
        }
    }
    by_run.run(0, 0, &folded);

    let agree = pixels.iter().zip(swept.iter()).all(|(a, b)| a == b);
    // `cell` left the cells past the string untouched and `run` painted them as
    // spaces, so the comparison is over the characters both were given. The
    // fold array is the full width of the surface for exactly that reason: both
    // paths see the same trailing blanks.
    crate::kprintln!(
        "  screen        the run sweep and the per-cell blit {}",
        if agree { "agree" } else { "DIFFER — the sweep is drawing something else" }
    );
}
