# RFC 0120: The late latch is the compositor's, and the input path's row says so

- Status: accepted
- Date: 2026-09-24
- Affects: `xtask/src/main.rs` (`INPUT_PATH`), `user/compositor/` (a new module and
  a new dependency), `interface/`'s row in that list. `TODO.md` `E3-B01i` and
  `E3-B04e`. Nothing in `abi/` and nothing on a wire.

## Decision

The late latch — the pointer re-read after a commit closes and patched into one
transform node before the frame's submission crosses — lives in
`user/compositor/`, and `xtask`'s `INPUT_PATH` gains a row saying so. The
`interface/` row, which said that crate was "the frame loop and the late-latch",
keeps its place on the path and loses that sentence: a late latch needs an
arena, a chain and a frame, and `interface/` holds none of the three.

Two things follow and are decided here rather than discovered later. The latch
reaches the retained graph **outside a commit**, through
`f_scene::arena::Arena::set_transform`, which is the one edit in this tree that
no client sent. And the compositor **mints a `StampNanos`** —
`f_input::stamp::StampNanos::from_wire_nanos` over the scanout it aimed at —
which is a target and not an event's time, checked at the argument site by the
rule `WIRE_MINT` already carries.

## Context

`INPUT_PATH` is not an allow-list. Its own documentation says so at length: the
only way to make a clock reading legal on the input path is to argue a crate
*off* the path, which is a diff that deletes a line in front of a reviewer. What
it has is six rows, each naming a stage and why a second reading there would be
a defect, and `lint_stamp` refuses any workspace member that names the stamp's
vocabulary and has no row.

The `interface/` row was written before there was a compositor. It said the
crate held the frame loop and the late latch, and named the hazard precisely:
*at latch time the correct measurement and the wrong one differ by which of two
numbers is called the event's time, and the wrong one is the shorter
expression.* The hazard is real. The location was a forecast, and it was wrong:
`interface/` is a vocabulary, a ladder and a solver, with no dependencies of its
own, and `E3-B01k` took it into the compositor rather than the other way round.
`stage_reach` had already been printing the consequence on every green run —
`interface/ reaches no clock` — for two epochs, which is the lint saying out
loud that one of its rows guarded nothing.

Three alternatives were live.

**Put the latch in `interface/` after all.** It would have to be handed an
`&mut Arena`, an `f_abi::sync::Chain` and a frame ordinal, which is the
compositor, passed as arguments to a leaf crate that deliberately depends on
nothing. `interface/src/lib.rs`'s claim that the crate can be abandoned is the
thing that would break.

**Leave `user/compositor/` off the path and route around the needles.** It is
spellable: carry the stamp as a bare `u64` named something else and never write
`StampNanos`. It is also the exact defect the rule exists to catch, arrived at
deliberately, and the honest version of it is a row.

**Add an exemption.** There is nowhere to put one. `INPUT_PATH`'s doc explains
why it has no companion allow-list — *the first thing a second reading would do
is write itself a row* — and this RFC does not create one.

The edit outside a commit had a fourth alternative worth recording: run the
latch through `f_scene::commit::Batch` as a one-delta frame. That makes the
latch a second commit, so the frame counter moves twice for one frame, the trace
records a second frame's waits, and `crate::pacing`'s estimate starts measuring
the compositor's own patch. A late latch that closes a frame is not a late
latch.

## Consequences

`lint-stamp` now checks six stages and four of them are stages it can actually
fail on — the count of rows did not change the count of *checked* rows before
this, and `stage_reach` prints the difference. `user/compositor/` is one of the
four: it depends on `f-input`, so a second reading in it compiles, and the rule
is the only thing stopping one.

What it makes hard, deliberately: reading a clock anywhere in the compositor.
There is no clock at ring 3 to read — RFC 0004 gives a component neither a timer
nor a port — so today the constraint costs nothing, and the day somebody hands
the compositor an `Env` it costs a red build with a reason in it.

What it forecloses: nothing on a wire. No byte moved, no record grew a field,
and `abi/src/trace.rs` was not edited. That file names its own extension shape —
*a second record keyed by the entry ordinal, not a field here* — and
`crate::latch::Latched` is keyed by the trace entry ordinal the latch happened
before, which is how a frame trace shows where in the frame the difference was
made without the trace growing anything.

The mint is the part a reviewer should push on, so it is stated at its narrowest:
`f_compositor::latch::Aim::scanout_nanos` is computed by
`crate::pacing::scanout_after` from `routing::at::TICK_NANOS` — the frame's
clock, written into the component's page — and the display's declared period.
Every question about *when the pointer moved* is answered by
`crate::latch::Reading::at_nanos`, which is the driver's stamp carried across two
rings unchanged.

## What would reverse this

Three observations, each mechanical.

**A display that reports its own scanout instant.** The mint goes away rather
than moving: `Aim` holds a number that arrived off a wire, `from_wire_nanos` is
called on a field of a decoded record like every other honest caller, and the
paragraph above is deleted.

**A renderer.** `E3-B02` owes the pixels. The day a rasteriser exists, the
window between the commit and the submission is the window between the commit
and the *draw*, and whether the latch stays before the compositor's own
submission or moves after it is a question a build with pixels can answer and
this one cannot. `crate::waits::Between` is one call in one function, which is
what makes that a line to move rather than a design to redo.

**A second component that latches.** If a second compositor, or a component that
composites a subtree, wants the same window, then the latch is a shape rather
than this component's, and it belongs where a shape belongs — beside the chain
in `abi/`, with the recorder at the admission. That is the same reversal
`abi/src/trace.rs` names for itself and it would be answered once for both.

The row itself reverses on a narrower observation than any of the three: if
`user/compositor/` stops naming the stamp's vocabulary — the latch moved, or the
prediction moved to the driver — the row is deleted in the diff that moves it,
and `input_path_membership_findings` is what makes leaving it behind a finding
rather than a quiet pass.
