# RFC 0118: What was given up is recorded per frame, in the word that recorded the last one

- Status: accepted
- Date: 2026-09-24
- Affects: `user/compositor/src/pacing.rs`, `user/compositor/src/routing.rs`,
  `user/compositor/src/tree.rs`, `user/compositor/manifest.toml`,
  `kernel/src/compositor.rs`; `E3-B07d` in `TODO.md` and
  `intent/0012-the-interface/`; `abi/src/manifest.rs` by not changing it

## Decision

The compositor's `degraded` state node stops carrying the last frame's
degradation choice and starts carrying a **register**: sixteen fields of four
bits in the one word it already had, newest frame in the low field, one field per
frame that closed. `f_compositor::pacing::Record` is the only code that packs or
reads it. The ordinals renumber so that zero is `NONE` — *no frame said anything
here* — rather than `FITTED`, because a register's unreached fields have to mean
something and *the frame fitted* is the worst thing they could mean. The
alternative, a seventeenth state node, is refused: `f_abi::manifest::
STATE_NODES_MAX` is sixteen, this component declares sixteen, and widening it is
a wire bound every component in this system pays for whether or not it has a
sixteenth thing to say.

## Context

`E3-B07d`'s exit is *under 2x overload every frame carries the reduction it
chose, read out of the component's subtree rather than the serial log*. The node
existed already — `E3-B01k` landed it, and a real `f_scene::degrade` call is what
produces the word — and it could not meet that sentence, for a reason worth
writing down because it is not obvious from the code: **a word holding one
snapshot is byte-identical between a compositor that decides per frame and one
that decides once at start and repeats itself.** That mutation was run against
this component's tree in an earlier wave and went red only because the boot
separately required `late` to be one out of two. A count is not a record. It says
how many frames were late and cannot say *which*, and the clause with teeth in
`E3-B07d` is the second question.

Three routes were live.

**Widen `STATE_NODES_MAX`.** It is a `u8` count and a fixed-width array in
`f_abi::manifest::Record`, so widening it is not a compatibility break — it is a
tax. Every component's manifest record grows by `size_of::<Node>()` bytes per
added slot whether the component declares the slot or not, the record is the
thing the frame reads before a page is spent, and `f_abi::state`'s tree sizing
(`four times manifest::STATE_NODES_MAX`) moves with it. RFC 0108 made exactly
this argument about a declaration most components leave empty and answered it
with a section rather than a field; the same argument arriving in a different
file gets the same answer. Nothing about *this* component's need is a reason for
`user/objects` to carry a wider record.

**Put the per-frame record somewhere that is not a state node.** The board —
`f_compositor::routing::reported` — has no bound and would have taken it in a
line. It is also not what the exit asks for: *out of the component's subtree* is
the clause, and `E3-B01k` settled the tree-versus-board question the same way
when it was asked about the frame's story. A register on the board alone would
have been the cheapest diff and the wrong one.

**Make the word that exists carry it.** A degradation answer is a small ordinal:
four criteria in `f_scene::degrade::Criterion::ORDER`, plus three answers that
are not criteria, plus the absence of a frame — eight values against the sixteen
a nibble holds. Sixteen nibbles is a 64-bit word, which is what a state node is.

## Consequences

What it makes easy: *per frame* becomes legible from the published number rather
than asserted beside it. A compositor that decides once fills the register with
one answer; one that latches its worst answer never returns to `FITTED`; one that
decides per frame leaves a register whose fields differ. The boot checks all
three — the serving half requires fitted-then-short over two frames, and the
waking half, which closes three frames with the late one in the middle, requires
them to alternate. The latching build passes the first and fails the second,
which is what the third frame is for.

What it costs, stated rather than discovered:

- **History older than sixteen frames is gone.** At sixty hertz that is a quarter
  of a second. A reader who wants a run-long figure reads `late` on the board
  beside it; the two are not redundant, because one says how many and the other
  says which.
- **The word is no longer an ordinal**, so every reader has to go through
  `Record`. `kernel/src/compositor.rs` imports it under an alias rather than
  comparing integers, and the boot log prints the fields as well as the packed
  word — a packed word alone would have made the serial log less readable than
  what it replaced, which would be this decision paying for the tree with the log.
- **The ordinals moved**, so `FITTED` is 1 and not 0. Any reader from before this
  date reading a word of zeroes as *a frame that fitted* is now reading it as
  *nothing was said*, which is the true statement about a page a component never
  reached. That is a gain and it is also a renumbering, which is why it is here
  rather than in a commit message.
- **A fifth criterion is now bounded by four bits.** It is not close — eight
  values used of sixteen — and a `const` block refuses the build that crosses it
  rather than letting one frame's answer overwrite its neighbour's.

What it forecloses: a compositor publishing *why* each frame was degraded in
anything richer than an ordinal. A criterion plus how much it saved, per frame,
does not fit a nibble and never will; the day that is wanted, the bound is the
conversation and not this word.

## What would reverse this

A second component wanting a seventeenth node. This decision is about one
component's word and it prices the widening as a tax on everybody; the moment a
*second* manifest is also full, the tax is no longer being paid for one
component's convenience and the arithmetic changes — at which point
`STATE_NODES_MAX` moves in an RFC of its own with the record's fixed width as its
cost, and this register stays anyway, because it would still be the better shape.

The other reversal is a display slower than four hertz or a reader who needs
more than a quarter of a second of decisions. Sixteen frames is a quarter of a
second at sixty hertz and four seconds at four; if the frames a reader wants to
compare are further apart than the register is long, the register is answering a
question nobody asked and the record belongs in something that is not one word.
