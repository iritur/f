# RFC 0128: The compositor is ranked by what it cannot take, and capped in what it will

- Status: accepted
- Date: 2026-09-25
- Affects: `abi/src/manifest.rs` (`SCHEMA` 4 to 5; `Ring::_reserved` narrows
  from five bytes to one and `Ring::deltas_per_frame_max` takes the other four;
  `FRAMED_PROTOCOL`, `FRAME_DELTAS_CAP_MAX`, `Ring::framed_server`; the reader
  refuses a `scene` server with no cap, a cap past the batch, a cap on any
  other ring, and a `scene` server under the hard class), `abi/src/reserve.rs`
  (`Demand::displaces`, `Demand::hardened`), `xtask/src/manifest.rs` (the
  checker and the compiler for the same, and a round trip of every manifest in
  the tree through the frame's reader), every `user/*/manifest.toml`
  (`schema = 5`), `user/compositor/manifest.toml` (`deltas_per_frame_max = 50`,
  and its `[reservation]` argument reversed), `docs/manifest.md`, the
  compositor's manifest paragraph this RFC quotes, and `TODO.md` task
  `E3-B07e`, which this splits.

## Decision

**`docs/design/ring-scene-boot.html` section 10's sentence — *the compositor is
the lowest-ranked deadline task on the machine, and its GPU submissions are
bandwidth-capped* — is two declarations, and this tree now carries both.**

- **Lowest-ranked is the soft class, and a component serving `scene` may not
  declare the hard one.** This tree's admission vocabulary has four urgencies,
  and two of them carry a deadline: `abi::class::HARD`, *deadline must be met*,
  and `abi::class::SOFT`, *deadline honoured best-effort; missing it degrades
  quality*. The lowest-ranked deadline class is `soft`. Its own definition is
  `E3-B07`'s policy, and RFC 0025 makes a declared class a ceiling, so nothing
  the compositor submits can outrank a hard-class entry. The rank is stated as
  arithmetic in `abi::reserve::Demand::displaces`: a demand is ranked below
  every deadline workload when admitting it first never costs one of them a
  reservation it is granted alone. Both the manifest checker and
  `abi::manifest::Record::read` refuse `class = "hard"` beside a `scene`
  server ring.
- **The cap is `[[ring]] deltas_per_frame_max`, schema 5.** The most scene
  deltas one frame may carry across a `scene` server ring, required there, at
  least 1 and at most `f_scene::commit::DELTAS_MAX`, and refused on every other
  ring. The compositor declares **50**. Past the cap each further delta is
  answered `RESOURCE/QUOTA_EXHAUSTED` and not applied; the commit that closes
  the frame is never counted and never refused. What is refused is the excess
  submission, not the frame.

## Context

**The survey this was built from said the opposite about the first half, and
it was reasonable.** `E3-B07e`'s record in `TODO.md`, written on 2026-09-24,
read *the lowest-ranked deadline task* as `class = hard` with the three CPU
fields, which already exist and which `Table::admit` already refuses when it
cannot honour them. The compositor's own manifest said the same as a forecast:
*the line that earns `hard` is the first one that draws*. Two things were not
in view when either was written.

1. **`abi::reserve::Table` has no rank to give.** A hard reservation holds whole
   physical cores and, where the part cannot partition, their whole exclusion
   domain; two grants never share a core, and the only order admission knows is
   *who asked first*. So a hard-class compositor is not ranked below anything:
   it competes, first come first served, for the scarcest thing the table hands
   out. On a part with no cache or bandwidth partitioning and two four-core
   domains there is exactly one hard reservation to give, and a compositor
   admitted first takes it and the audio loop section 10 exists to protect is
   refused `NO_CORE`. `a_hard_compositor_displaces_the_workload_it_must_yield_to`
   is that, in `abi/src/reserve.rs`, and it passes.
2. **Every boot in this tree would have refused the compositor's spawn.**
   `kernel/src/admit.rs` already says QEMU cannot host a hard reservation: no
   thread level, no cache topology, no RDT, so the frame's own core poisons the
   only contention domain. `-smp 2` is what `cargo xtask` boots, and the hard
   variant of the compositor's own record is refused `NO_CORE` on a machine of
   that shape — `the_compositor_is_the_lowest_ranked_deadline_task` asks it of
   the record `user/compositor/manifest.toml` compiles to. The spawn tests that
   demand before a page is spent (`kernel/src/component.rs`), so `hard` in that
   file would have been every compositor boot red at the spawn, on a change
   that read as the design being honoured.

**The second half had no room.** `abi::manifest::Record` spent its last
reserved byte on schema 4's face count, and `abi::manifest::SCHEMA`'s own
comment says the next field takes a section or pays for the growth. The one
declaration space left was `Ring::_reserved`: five bytes at offset 99 of each
ring slot, refused non-zero. A `u32` at offset 100 is aligned and leaves one
reserved byte, so **the ring slot stays 104 bytes and the record stays 2 696**.

**RFC 0101's check was run before the width changed, and the answer is that no
width changes.** Every reader of a ring slot was found: `Record::read` and
`Record::read_unaligned` through one `judge`; `Ring::EMPTY`, `is_zero` and
`check`; `xtask::manifest::compile`, which stamps at literal offsets;
`sim/src/deploy.rs`, which builds one with `..Ring::EMPTY` and so inherits the
new field as zero; `f_assembler::topology::Instance`, which owns a `Record` by
value and whose stack temporary is why schema 4 is a section — and which sees
the same size it saw. No count sizes anything by the number of rings'
reserved bytes. The offset is pinned by a `const` assertion in `abi` and by a
test in `xtask` that compares `core::mem::offset_of!` against the writer's
offset as values, now that `xtask` depends on `f-abi`; and every manifest in
the tree is compiled and handed to `Record::read_unaligned`, which is the one
round trip that would catch a cap stamped a byte early — it lands in the
reserved byte and is refused `Reserved`.

**Fifty is section 07's number.** *A typical UI frame changes somewhere between
five and fifty nodes.* `f_scene::commit::DELTAS_MAX` is that with headroom to
sixty-four, and the batch poisons its frame on the sixty-fifth delta. A cap
must therefore be at most sixty-four to be reached before the frame is lost —
a cap above it is a cap that only ever stops frames — and fifty is where the
design stops calling a frame ordinary. Every boot in the tree sends at most
eight deltas a frame, so the cap binds nowhere the tree already runs.

**What is capped is deltas crossing the scene ring, and that is a stand-in.**
Section 10 caps the compositor's *GPU* submissions, and this compositor submits
nothing to a device. Every delta is fifty-six bytes, so a cap in deltas is a
cap in bytes accepted per frame and bounds what one frame can cost to apply.
`docs/manifest.md` refuses a bandwidth demand stated in a unit no two machines
share; a delta is not one of those.

## Consequences

**Easy.** A compositor file without a cap does not reach a boot: the checker
refuses it and so does the frame's reader, because a `scene` server with a zero
cap is `ARGUMENT` at the spawn. A hard-class compositor is refused in both
places with the reason. The rank is a function a test can call rather than a
sentence a reviewer has to believe.

**Hard, and not yet done: the enforcement.** The cap is declared, compiled,
read and bounded, and **nothing refuses a fifty-first delta yet.** Where a
delta is counted is `user/compositor/src/component.rs`, beside `E3-B01j`'s
crossing increment, and the frame hands the compositor its routing page in
`kernel/src/compositor.rs`. Neither is this RFC's file. Until those land, the
field is a declaration a reader believes and nothing keeps — R08's shape, for
the length of one handoff, and written down here so it cannot be mistaken for
the finished state. What the enforcement is, precisely, is in `E3-B07e`'s
record: the frame reads the cap out of the record by `FRAMED_PROTOCOL` and
refuses to lay out a compositor with none; the compositor counts the non-commit
deltas it takes into a frame and answers the excess
`RESOURCE/QUOTA_EXHAUSTED` with the cap as the detail, **without offering it to
the batch**, so the batch is never poisoned; the counter resets where the
commit closes the frame; and a boot sends one frame of sixty deltas and reads
fifty applied, ten refused, the frame closed and the rung held.

**Hard: a protocol renamed walks past both rules.** Both readers key on a server
ring whose protocol is exactly `scene`. A compositor manifest that said
`protocol = "scenes"` would be uncapped and unrefused, and nothing in the frame
today reads a ring's protocol at all — `kernel/src/compositor.rs` sizes the ring
from its own constant. The defence is the enforcement's first half: a frame that
finds the compositor's ring *by* `FRAMED_PROTOCOL` and refuses to start it
without one turns the rename into a red boot. Until then the rename is a gap,
and it is named rather than assumed away.

**Forecloses** a hard-class compositor, for as long as admission holds whole
cores; and a cap in any unit other than deltas on the scene ring.

## What would reverse this

- **An admission that ranks within the hard class** — deadlines sharing a core
  under a schedulability test rather than whole cores held apart. Then
  *lowest-ranked* is a position in that ranking, the compositor is `hard` with
  the longest period on the machine, and the refusal of `hard` beside a `scene`
  server is the thing that goes.
- **A ring from the compositor to a display driver.** Then section 10's cap has
  its own subject: it moves to that client ring, counts the compositor's own
  submissions, and its unit becomes what a submission there moves — bytes of
  surface, not deltas. `deltas_per_frame_max` on the scene ring stays or goes
  on its own evidence.
- **A measured client whose ordinary frame exceeds fifty deltas.** Section 07's
  number is a sentence, not a measurement; the first client that measures its
  frames and finds the ordinary one above fifty moves the cap, and above
  sixty-four moves `f_scene::commit::DELTAS_MAX` with it.
- **A second protocol with a commit**, at which point `FRAMED_PROTOCOL` is a
  list and the cap counts that protocol's entries as well.
