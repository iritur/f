# RFC 0133: A UI frame is the representative scene, warm

- Status: accepted
- Date: 2026-09-25
- Affects: `TODO.md` task `E3-B01` (its exit is judged against this frame),
  `claims/0039-ring-crossings-per-representative-frame.toml` (new),
  `claims/0038-ring-crossings-per-ui-frame.toml` (unchanged, and now not the
  only reading of the instrument), `user/compositor/src/timeline.rs` (new: the
  frame's client application and the reconciler re-exported for the frame),
  `user/compositor/src/lib.rs` (*no client library* narrowed to the component),
  `user/compositor/src/routing.rs` and `user/compositor/src/component.rs`
  (`reported::CUT_FRAMES`, `CUT_DRAINED`, `CUT_ANSWERED`, written at every
  commit before its completion is posted), `kernel/src/compositor.rs`
  (`Half::Timeline`, `drive_timeline`, the per-commit verdict and the rows),
  `kernel/src/main.rs` (the parameter), `xtask/src/main.rs` (the half, its
  liveness, `Route::RepresentativeCrossings`).

## Decision

**The UI frame `E3-B01`'s exit is counted over is `claims/0033-scene/scene.toml`
at the moment its playhead moves, with the scene already in the compositor — a
warm frame — rebuilt whole by the client and reconciled.** Its crossings are the
entries between the commit that closed the previous frame and the commit that
closes it, counted by `E3-B01j`'s instrument on both sides and required equal at
every commit. On that frame the count is **four**: the playhead's transform and
the commit out, their two completions back. Under ten, so the parent's exit is
met, and `claims/0039` gates on the worst of eight such frames being at most
nine.

The frames that bring the scene into the compositor — forty of them, 1 709
deltas under a cap of fifty — are **not** UI frames in this sense. They are
published (the worst is 102) and bounded exactly, and they bound nothing the
design says about a frame.

## Context

`claims/0038` counted eight per frame and declined, in its own threshold
comment, to gate on ten: its two frames were a script written for the test —
five creations and a paint, then a removal — driven by a client with no
reconciler, so the number described the script rather than a frame of anything.
It named what would earn the sentence: a client with a reconciler in front of
it, on a frame nobody chose for its count.

Three candidates for *the* UI frame were live.

1. **`claims/0033`'s scene, warm.** Chosen by `E3-B02j` for representativeness
   rather than reachability, argued from design section 13, and chosen by a task
   pricing a *renderer* — not trying to make a crossing count small. Its own
   README says a run reports the frame in which `playhead` moved and nothing
   else did, and that *the first frame of this scene is a different number and
   is not what the claim's rows bound*.
2. **The same scene, cold** — every node created in one frame. Not deliverable
   as one frame in this tree at all: 1 709 deltas against `f_scene::commit::
   DELTAS_MAX` of sixty-four and the manifest's cap of fifty (RFC 0128). A cold
   frame is the scene loading, and the design's own arithmetic (section 07's
   five to fifty deltas) is drawn for warm frames.
3. **A frame a skeptic would pick: the same timeline while it plays.** A playing
   audio timeline animates more than its playhead — the eight level meters in
   the track headers move every frame, and the status line's time readout
   changes. That frame is not the one scene.toml declares (`dirty_part` is the
   playhead alone), but it is the obvious objection, and it is measured below
   rather than argued away.

The choice is (1), because it is the only definition in this tree made by
somebody with no stake in the number, and because (3) is a different scene —
scene.toml would need a second `dirty_part` and a new hash, which is `E3-B02j`'s
file to change and not this task's.

### Why the parts that cannot reach the compositor do not change the count

scene.toml's `[unreachable]` table is still true: 645 paths (35 729 segments)
have no geometry encoding and would not fit a one-page channel's 2 176-byte
arena if they had one; 155 glyph runs have no opcode and no shaper; the two
effect declarations travel on `SetEffect`, which the reconciler does not emit.
So the client builds all 995 nodes, all transforms and all paints, and no path,
run or effect declaration. **None of those moves in the warm frame**, and the
reconciler emits a property delta only where a property differs — so the
unreachable content would cost the warm frame nothing even if it were reachable.
That is the narrow claim this RFC rests on: *the warm frame's deltas are the
deltas the whole scene would cost*. The build frames' are not (every path would
add a `SetPath`), and they are not what `claims/0039` bounds.

### Where a frame's crossings end

The instrument counts totals at the end of a run, and a run that builds for
forty frames and plays for eight has one total over two kinds of frame. The
component now writes the same three counters — frames, drained, answered — at
every commit, **before** that commit's completion is posted, so the ring's
`Release`/`Acquire` pair orders them for the client on both architectures. The
client takes its own count at the same moment (completions reaped before the
commit's own). A frame's crossings are the difference between two consecutive
cuts: a partition of the run, every crossing in exactly one frame. This is not
a second counter — the counters are `E3-B01j`'s, read at a different moment —
and the boot requires both sides equal at each of the forty-eight cuts, plus
the last cut equal to the end-of-run totals less the one completion it is taken
before.

### What the design counts, beside what this counts

Section 09's walk says *two hardware writes and no system calls per frame*. That
counts doorbells and MMIO writes; `claims/0038` settled that a crossing here is
an entry in either direction, which is stricter. The two are different units
and neither is converted into the other: the four here is entries, and this
client asks for a completion on every entry, so a client setting `NO_CQE` on its
deltas would read three.

### The frames that were measured

- **The chosen frame:** 4 on every one of eight warm frames — 2 out, 2 back, by
  the component's count and by the client's.
- **The skeptic's frame**, as a mutation of the client's application — the
  eight level meters' paint changes with the playhead, every warm frame: nine
  deltas from the reconciler, **20 per frame, 10 out and 10 back** on both
  sides, and `cargo xtask claim ring-crossings-per-representative-frame` went
  `RED  ring_crossings_per_representative_frame_worst = 20  (min 2, max 9)`. A
  client setting `NO_CQE` on its deltas would read eleven — arithmetic, not a
  run. So *under ten* holds for the frame the scene declares and does **not**
  hold for a playing timeline whose meters move, under this definition of a
  crossing. That is the finding, and it is published rather than folded into a
  choice of frame.
- **The same frame on two of eight** (meters moving only as the playhead
  reaches frame five, so frames five and six each cost 20): the mean over eight
  is 8 000 per thousand — under ten — and the claim still went
  `RED  ring_crossings_per_representative_frame_worst = 20  (min 2, max 9)`.
  That run is why the gate is on the worst frame and not on the mean.

## Consequences

`E3-B01`'s exit is met on a frame nobody chose for its count, through the
reconciler and the real ring, counted on both sides at every frame boundary.
`claims/0038` stays exactly as it is: the scripted serving half, gating on
eight, and still not the parent's figure.

The compositor crate now carries a client application and re-exports the
reconciler for the frame to call. The component never calls either. This is a
cost of the frame being the client and having no host tests, and it reverses
when either stops being true.

The kernel image carries an empty reconciler as a constant (`EMPTY_RECONCILER`,
the size of 995 nodes and their scratch), because the kernel is built
unoptimised and writing `Reconciler::new()` built it on the boot processor's
stack — the first boot of this half took a double fault there. That is a real
cost to the frame's constants, measured rather than estimated: the boot's
`frame` line read 1 773 568 bytes of text and rodata before this change and
1 904 640 after it, 131 072 bytes with the timeline client's code, and it is
stated in the source beside the static.

What this forecloses: gating the parent on a mean. `claims/0039` bounds the
**worst** of its eight frames, because the exit is per frame and a mean over
eight is satisfied by seven cheap frames and one that is not.

## What would reverse this

- **A second representative scene**, or `claims/0033`'s scene declaring more
  than the playhead dirty. The frame this RFC counts is whatever scene.toml's
  `dirty_part` says moves; if `E3-B02j`'s successor decides a playing timeline
  dirties its meters too, the count on that frame is over ten under this
  definition, and `E3-B01`'s exit is **not** met — the parent reopens rather
  than the definition moving.
- **A crossing redefined as a publish or a doorbell.** `claims/0038` refused
  that at length; if it is ever accepted, every number here converts and this
  RFC is superseded with it.
- **A client that is not the frame** (`E3-B01g`'s remaining sentence): the
  application moves out of `f-compositor`, and the per-frame cut is re-argued
  if that client pipelines frames, because then a commit's completion is no
  longer the boundary between one frame's crossings and the next.
- **Geometry reaching the arena.** Once a path can travel, the build frames grow
  by 645 `SetPath` deltas and the warm frame should not move; if it does, the
  claim that unreachable content costs the warm frame nothing was wrong.
