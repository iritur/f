# RFC 0085: The first real display, and the three things it said

- Status: accepted
- Date: 2026-09-16
- Affects: `kernel/src/screen.rs` (the font, the fold, the scale, the grid
  bounds, the blit and the clear), `kernel/src/arch/x86_64/multiboot.rs`
  (`Channels::saturated`), `kernel/src/arch/x86_64/paging.rs` (the
  page-attribute table, `map_device_wc`, `fence_stores`),
  `kernel/src/arch/x86_64/boot.rs` (the video request's three numbers),
  `kernel/src/smp.rs` (one call on the arrival path), `kernel/src/main.rs`,
  `xtask/src/main.rs` (`cargo xtask screen cost`),
  `docs/booting-on-hardware.md`'s *The screen* section, and RFC 0081's
  write-combining reversal condition, which is paid here
- Supersedes, in RFC 0081 and nowhere else, **three statements about the
  implementation and one reversal condition**: that the font is *five pixels by
  seven, ninety-five glyphs, typed*; that the console draws *a light grey on
  near-black*; that *the grid clips at 160 by 64*; and the reversal bullet
  beginning *A machine where the console is wrong rather than plain*, whose
  prediction is answered below rather than followed. **RFC 0081 is not
  superseded as a whole and its decision is untouched**: the console still lives
  in the frame, is still the fifth shared place, and still rests on nothing
  reading it back.

## Decision

The console keeps every decision RFC 0081 made and changes everything RFC 0081
described. An eight-by-sixteen cell rather than a five-by-seven drawn at double
size; white built from the channel masks rather than a colour packed through
them; characters rather than bytes; a video request that names 1920 by 1080
rather than asking the firmware what it already has; and **a write-combining
mapping rather than an uncacheable one**, which is RFC 0081's own named next
change, paid.

## Context

RFC 0081 shipped a console that had never met a display. Every check it carried
ran against an array in memory, because QEMU's `-kernel` loader implements no
part of the multiboot video request and no boot the harness can start has a
framebuffer at all. That was stated honestly at the time and it was still the
whole of the problem: **the first real display disagreed with the code in three
ways, and two of them were invisible to every check that existed.**

### One. The colour was carrying an assumption, and it was the only part that could be wrong quietly

The log came up **yellow-green**. Yellow-green is red plus green with the blue
missing, which is what a packed colour looks like when one channel's position or
width is not what the code believed it was.

The console had picked a considered light grey — `(0xC8, 0xCF, 0xD8)`, chosen
because full white on a panel at close range is the one choice a reader notices
— and packed it through the six numbers the loader reported. Packing needs all
six to mean what the caller believes. Every other thing this file does is
checkable by looking at it: a glyph in the wrong place is visibly in the wrong
place, a misread pitch skews the whole screen. **A wrong colour is just a
colour**, and somebody might have chosen it.

The repair is not to find the bad field. It is to stop depending on the answer:
`Channels::saturated` sets every bit of every reported field, which is white
under any order, any width and any arrangement, because there is no way to
arrange three saturated masks into something that is not white. A field this
code misreads now leaves that channel dark — a tint — rather than a different
colour entirely.

**This is the general shape and it is worth naming.** The value that survives a
misread structure is better than the value that is correct when the structure is
read right. White on black is also what the console being compared against does,
so the robust choice and the idiomatic one are the same choice.

What is *not* done is diagnosing the field. The boot prints the six numbers on
its `framebuffer` line; if they turn out to be wrong, that is a parsing defect
with its own entry, and this RFC has made the screen legible while it is found.

### Two. Three question marks where a dash should be

This repository's own prose is written with typographic punctuation. The em dash
in `place virtio-gpu — 4 capabilit(ies) revoked` is in almost every line the
supervisor prints, and an em dash is three bytes of UTF-8.

The console walked bytes. Each of those three bytes was outside the font's
range, so each drew a replacement mark: `???` down the middle of half the
screen, on a display whose entire purpose is to be read.

`&str` is valid UTF-8 by construction, so the fix is `chars` rather than
`bytes`, and it is smaller and cheaper than what it replaces — no decoder, no
state, no way to be halfway through a character when a flush happens.
`screen::fold` then maps the punctuation this tree actually writes onto the
nearest thing the font has: dashes to a hyphen, curly quotes to straight ones, a
micro sign to `u`. The list is short deliberately, because a fold nobody needs
is a row somebody has to check.

**The alternative was a wider font**, and it was refused: carrying glyphs for
punctuation means a lookup from code point to glyph, which means the font stops
being an array indexed by arithmetic. For a console read during a boot, a dash
that is not quite the author's dash costs the reader nothing. A dash that is
absent costs them the line.

### Three. The font was legible and looked like a calculator

Five by seven at double size is 95 glyphs of real information and no more. It
has no room for a descender that clears the baseline, no room to tell `1` from
`l`, and at double size its strokes are two pixels wide — which is why it read
as a calculator display rather than as a console.

Eight by sixteen is the cell this architecture has used since the VGA adapter
and the one a Linux console still uses. That is not nostalgia and it is not
compatibility: it is the bitmap cell that has had the most hours of reading, and
its proportions — caps ten rows, x-height six, descenders through row fourteen —
are what a reader's eye expects a booting machine to look like. The table is
still typed, still original, and still checked by being read back as shapes.

**RFC 0081 predicted this moment and predicted the wrong remedy.** Its last
reversal bullet says that a console *wrong rather than plain* on a real display
argues for the component rather than for a bigger table in the frame. A real
display arrived and the console was wrong rather than plain, and moving to a
component was not available: the component is E3, the E3 decisions landed the
same week, and none of its build work has started. What the bullet got right is
that the fix is not *more font*; what it got wrong is that the choice was
between a component and nothing. Sixteen hundred bytes of table is not a step
toward a font engine in ring 0, and the licence gap it was really about —
`LICENSING.md` has no category for imported data under a permissive licence, so
Terminus cannot be reached from the frame — is untouched and still owed.

### And the mode, which is the one place this takes something away

The request named no size, so GRUB handed back the mode the firmware was already
in: 1024 by 768. At eight by sixteen that is 128 columns, and this kernel prints
lines longer than 128 columns.

The header now asks for 1920 by 1080 at thirty-two bits. This is safe for a
reason worth stating rather than trusting: GRUB turns the request into the mode
list `1920x1080x32,1920x1080,auto` — its own construction — and tries them in
order, so a firmware that cannot set it falls back to exactly what the empty
request would have produced. **No arrangement of this header ends with no
framebuffer where the empty one would have got one.**

The cost is control. GRUB writes the header's request into `gfxpayload` itself,
so `GRUB_GFXMODE` no longer decides what the payload gets — the kernel's request
wins over the machine's configuration, which is the wrong way round for anything
except a fallback console whose whole job is to be readable before there is
anything else. Zeroing the three numbers hands the choice back.

### And it was too slow to watch, which the mapping's memory type explains

A boot that draws is a boot that redraws. A scroll moves every line, so every
cell whose character changed is drawn again — on a 1080p screen that is up to
sixteen thousand cells of a hundred and twenty-eight pixels each, two million
stores, for one new line of output. Under the uncacheable mapping every one of
those stores is its own bus transaction, and the processor waits for it.
Uncacheable is what a device *register* needs and needs for correctness, since a
cached read of a status register returns whatever it said the first time. A
framebuffer has no registers, and this console never reads it back — that is the
property RFC 0081 rests on — so uncacheable was buying nothing there but the
cost.

Three changes, and they are one improvement rather than three:

- **Entry 4 of the page-attribute table becomes write-combining**, and the
  display is mapped through it. The processor may then gather stores into fill
  buffers and put them out as bursts. It needs none of the cache-flushing
  ceremony the manual attaches to that register, for the reason `PAT_VALUE`
  states: nothing has ever been mapped through entry 4, so there is no old
  meaning to be wrong about, and entries 0 to 3 are written back as found.
- **Every core programs it**, on the arrival path, because any core can print
  and the memory type is the writing core's. A core that had missed it would
  put the display in its own cache with nothing to flush it — not slow, wrong.
- **The blit hoists its per-pixel work.** A glyph row is now eight stores to
  consecutive addresses with the row address computed once, rather than eight
  calls that each rebuild an address, re-test a bound and re-match a depth that
  cannot change during a boot. That matters *because of* the mapping: scattered
  stores do not combine, so the gathering the first change buys is only
  available to a loop shaped like this one.

Two smaller things fall out. The first paint was sixteen thousand blank glyphs,
drawn to cover whatever the firmware left on the screen; it is one sequential
sweep now, which is the access pattern write-combining is best at, and the
console may then record the screen as blank rather than as unknown — so the
first flush draws only the cells that have text in them.

And **a flush ends with a store fence**, which is the cost of the memory type
rather than an optimisation. A write-combining store may sit in a fill buffer
until the processor has a reason to drain it, and *the machine stopped* is not
one of its reasons. Without the fence, the last line before a hang is the line
that never reaches the screen — which is the line somebody is reading the screen
to find.

**What is not claimed is a number.** `cargo xtask screen cost` times a full
redraw and prints cycles, per-cell cycles and microseconds, and it reports *no
surface* on every machine this harness can start, because QEMU's loader provides
none. The measurement belongs to a machine with a display, and the verb exists
so that the question is settled by one rather than by this paragraph. What can
be said from here is the shape of the change and not its size.

## Consequences

- 240 columns by 67 rows at 1080p, against 85 by 48 before. The grid bounds go
  to 256 by 96, which is 48 KiB of `.bss` for the two arrays rather than 20.
- The scale is a property of the surface rather than a constant, and is two only
  past 2560 pixels across, where a sixteen-row cell stops being readable at
  arm's length.
- A 1080p surface is 8.3 MiB, so `open_screen` maps 2025 pages rather than 768.
  Still paid once, still uncacheable, and the write-combining question RFC 0081
  left open is unchanged and still wants a measurement.
- The self-test string is `F — screen 1l`, which is not neutral: it carries the
  em dash that produced `???` and the two characters the old font could not tell
  apart. A check that would not have caught the defect is a check that has not
  been updated for it.

## What would reverse this

- **The licence gap closing.** The moment there is a category for imported data
  under a permissive licence, or the console moves above the frame, the typed
  table stops being the best available font and Terminus or Spleen replaces it.
  That is the same reversal RFC 0081 named and this entry does not weaken it —
  it only denies that a crude font in the meantime was the only alternative.
- **A machine where 1920 by 1080 is the wrong request.** A display that cannot
  reach it falls back and loses nothing; a display much larger than it gets less
  than it could. If naming a mode turns out to cost more machines than it
  serves, the three numbers go back to zero and the choice returns to
  `GRUB_GFXMODE`, where a machine's own configuration is a better place for it.
- **`cargo xtask screen cost` on a real display showing the redraw is still too
  slow.** Then the memory type was not the binding constraint and the next thing
  to attack is the redraw itself: a scroll that rewrites every changed cell is
  the algorithm a dumb framebuffer forces, and escaping it needs a scanout whose
  start address can move — which is a display driver, which is the component.
- **The `framebuffer` line reporting channel fields that are plainly wrong.**
  Then the yellow-green screen was a parsing defect rather than a fragile
  colour, `saturated` has been hiding it, and the fix belongs in
  `Framebuffer::parse` with this entry's first section as the symptom that found
  it.
