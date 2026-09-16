# RFC 0086: The header is what the loader compiled, and the table is what the reader implements from

- Status: accepted
- Date: 2026-09-16
- Affects: `kernel/src/arch/x86_64/multiboot.rs` (`Framebuffer::parse`'s colour
  reads, `Channels`' note, and `self_check`'s fixture); RFC 0085's first
  section, whose open question this closes, and its last reversal condition,
  which is the one that fired

## Decision

The multiboot information structure's colour fields are read at **byte 112**,
not at byte 110 where the specification's table draws them. The fixture that
checks the parser is written at 112 too, and says why.

## Context

RFC 0085 left a question open and said where the answer would go:

> The boot prints the six numbers on its `framebuffer` line; if they turn out to
> be wrong, that is a parsing defect with its own entry, and this RFC has made
> the screen legible while it is found.

A VMware guest printed them:

```
  framebuffer   1024 x 768 x 32 other at 0x00000000f0000000
    channels    r 0+0, g 16+8, b 8+8
```

**A channel cannot be zero bits wide.** That is not a display reporting an
unusual layout, it is a reader looking in the wrong place — and the rest of the
line says exactly how wrong. Ordinary `b8g8r8x8` is `r 16+8, g 8+8, b 0+8`.
What was read is two zero bytes followed by the first four real ones, every
field shifted one position later. The parser was two bytes early.

### Where the two bytes come from

The specification draws a table, and the table puts `color_info` at byte 110,
immediately after the one-byte depth at 108 and the one-byte type at 109. Read
that table and 110 is the obvious answer; it is the answer this parser had, and
it is wrong on every loader anyone will meet.

Loaders do not fill the structure from the table. They fill it through the
reference `multiboot.h`, where those six bytes are the second arm of a union
whose first arm begins with a thirty-two-bit palette address. A union takes the
alignment of its widest member, so the compiler places it at the next multiple
of four — **112** — and leaves 110 and 111 as padding. The header is what the
loader compiled. The table is what the reader implemented from. They disagree,
and memory holds the header's answer.

The machine's own numbers confirm it rather than merely fitting it: bytes 110
and 111 read as zero, which is padding, and the four bytes after them are
`16, 8, 8, 8` — red at 16 for 8 bits, green at 8 for 8 — with blue at 0 for 8 in
the word after. That is `b8g8r8x8`, which is what a firmware surface on this
class of machine reports and what `f_virtio_gpu::driver::FORMAT` already pins.

### Why the check agreed with the defect, which is the part worth keeping

`Framebuffer::self_check` exists because no boot this harness can start has a
framebuffer, so the parser is otherwise never executed. It builds a structure at
known offsets and requires the fields to survive a round trip. It passed
throughout.

It passed because **the fixture wrote the colour bytes where the parser read
them.** Both came from the same table and the same belief, so the round trip
agreed however wrong the belief was. A fixture built from the same
understanding as the code it checks is a mirror, and a mirror always agrees.

This is a sharper version of a rule the tree already has. `self_check`'s own
comment says the expected layout is written out rather than compared against
the drawing code's constant, *because a check that asked one belief whether it
matched itself would pass however wrong both were* — and the check then did
exactly that one level down, at the offsets rather than at the values. The
guard was in the right place and one step too shallow.

What broke the tie was not a check but a machine. That is the honest reading of
the whole screen effort: every defect in it so far — the colour, the em dash,
the font, and now this — was found by a display and none by the harness.

## Consequences

- `r 0+0`, or any zero-width channel, is now the signature of this mistake
  being made again, and `Channels`' documentation says so at the place somebody
  would edit.
- The fixture writes 110 and 111 as zero padding deliberately, so a future
  parser that drifts back to the table's offsets reads zeros and fails the
  round trip instead of agreeing with itself.
- `cargo xtask screen parse` reports `b8g8r8x8` where it used to report a
  layout assembled from the wrong bytes.
- **The screen was legible the whole time this was wrong**, which was not luck:
  RFC 0085 replaced a packed colour with `Channels::saturated`, which sets every
  bit of every reported field. A field it cannot use contributes nothing, so a
  dropped red channel is a tint rather than a black screen. The robust value
  bought the time to find the real defect, which is the argument that RFC made
  in the abstract and this entry is the instance of.

## What would reverse this

- **A loader that packs the structure.** One that wrote the colour fields at
  110 as the table says would now be misread by exactly the two bytes this
  entry moved. Nothing in the wild is known to do it, and the repair if one
  appears is not to move the offset back but to decide from the *type* byte and
  the padding: a direct-colour surface whose bytes at 110 and 111 are a
  plausible position and width, and whose bytes at 112 onward are not, is that
  loader.
- **Multiboot 2, or Limine.** Both carry this block in a tagged structure with
  its own layout, and `E5-D03` may choose either. Then this offset is gone
  rather than corrected.
