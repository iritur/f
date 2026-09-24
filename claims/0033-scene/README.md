# The scene behind `claims/0033`

This directory holds the one scene `claims/0033-raster-cost-per-rung.toml` prices
four renderers over. `scene.toml` is the scene; this file is the argument for it,
and the claim's `[workload]` names the file by hash:

    $ sha256sum claims/0033-scene/scene.toml

The value is **not repeated here.** `claims/0033`'s `[workload] scene_sha256` is
the one place it is written down, and a hash written in two files is a hash that
can disagree with itself — which is the whole of what `claims/README.md` is for,
applied to a number this directory would otherwise own a second copy of.

`.gitattributes` pins every file in this repository to LF in the repository and
in a checkout, so that command answers the same on every machine. If a working
tree has drifted anyway, `git show HEAD:claims/0033-scene/scene.toml | sha256sum`
is the same number read from what git stores, and the disagreement between the
two is the drift rather than the scene.

## Why this is named for the claim and not for its content

`claims/canvas-corpus/` and `claims/theme-corpus/` are its two neighbours and
both are named for what they hold, because both are corpora: directories that
*grow*, assembled by somebody who is not us, where a new entry is the point. This
is the opposite object. It is one file that must not change, cited by one claim,
and the day it changes every number that claim holds is about a different
picture. So it is named for the claim, and a reader who finds this directory
knows immediately that its contents are not theirs to extend.

## The exit this closes, and the trap in it

`E3-B02j`'s exit: *one scene chosen and named in the claim's workload row with
its content hash, and the argument recorded is representativeness rather than
reachability.* The second clause is the whole task, and `claims/0033`'s own
`[workload]` note is what sets it:

> The scene itself is deliberately not fixed here. Fixing it now would be
> choosing a benchmark before there is anything to run it on, and a scene chosen
> that early is chosen to be reachable rather than to be representative.

That sentence was written to hold the choice open until `E3-B02` could make it.
It was also the sentence that would have been quietly falsified by waiting: this
line used to wait on rung 4 of a GPU, and RFC 0088's reading — a *decide* task's
exit may not require its own implementation — freed it to start today. Waiting
for a rung to exist would have meant choosing in front of a renderer, and a scene
chosen in front of a renderer is a scene the renderer can draw. **So the choice
is made now, deliberately before anything can render it, and `scene.toml`'s
`[unreachable]` table is the receipt.** Seven rows, each naming what no rung here
can do and the task that would change it, every one of them expected to stop
being true.

There is a cost and it is stated rather than discovered: nothing checks that this
scene is *renderable*, because nothing renders. What can be checked today is
internal, and these are the relations, written out here rather than left in a
script so that whoever implements the check does not have to reconstruct them:

1. Every part's `parent` is another part, and exactly one part — `surface` — has
   none.
2. Every `kinds` entry is one of the six kinds of `abi/src/scene.rs`.
3. `[census]`'s five count rows each equal the sum of the parts' rows, and its six
   per-kind rows sum to its `nodes`.
4. `[scene] dirty_part` names exactly one part, and `[census] dirty_nodes` is that
   part's node count.
5. Every `[[run]]` sits on a part that lists its script; a run of any script but
   `latin` states exactly `runs` times that entry's sample length in
   `text/src/corpus.rs`, and a `latin` run no more than `runs` times the control
   sample's; and each part's two `glyph_` rows are the sums of its runs.
6. Every `[[effect]]` sits on a part that declares the `effect` kind, states both
   words as non-zero, and states a saving no larger than its estimate — which is
   `scene/src/effect.rs`'s own arithmetic, since `Instead` is derived from the
   pair rather than written beside it.
7. `[workload] scene_sha256` in `claims/0033` is the hash of this file.

That set was run by hand in the wave that chose the scene and **broken six ways**
to show it could fail — including one mutation that changes nothing but a
sentence, which only relation 7 catches, and two that the first version of the
check could not see at all because it held a copy of this file's composition
instead of reading `[[run]]`. **The check itself is owed to `E3-B02`**, where an
encoder that builds this scene can compare the graph it built against the census,
and a scene that does not build is a red run rather than a reader's discovery.

## What is in it, and what real interfaces do that it does

An audio timeline, one frame, at the moment the playhead moves. Twelve parts, 995
nodes, 645 paths over 35 729 segments, 155 glyph runs across all eight corpus
scripts, two declared effects, and **three dirty nodes** — every one of those
quoted from `scene.toml`'s `[census]`, which is the file that has to be right,
and none of them stated here for the first time.

Section 13 of `docs/design/ring-scene-boot.html` is why it is a timeline rather
than anything else. That section names the case that *sank every predecessor* —
an audio timeline or a 3D viewport, which renders its own pixels and cannot be
expressed in a node vocabulary — and states the test in its own words: can an
agent select the clip between 4.2 s and 6.8 s on track 3, and can a screen reader
describe it, without either one seeing a single pixel? `scene.toml` carries that
selection as integers, on track 3, from 4 200 ms to 6 800 ms. A scene that could
not express the question would measure the rendering pillar while dodging the one
the same document calls least settled, and `interface/src/node.rs` already has
`canvas`, `track`, `clip` and `marker` in its vocabulary *because* of that
section — the roles this scene needs were chosen before this scene was.

The rest of the argument is per part, in `scene.toml`, because a claim's workload
row is not where twelve arguments fit. The five that decide it:

- **Chrome around one content view.** Nearly every window somebody works in all
  day is a small amount of widget chrome around one large surface the application
  draws itself. A scene of widgets alone measures the cheapest layer a real frame
  contains, and is what a benchmark chosen from what a toolkit can already draw
  looks like.
- **Three nodes dirty of 995.** The encode stage is the only CPU stage and it
  touches changed nodes only, so the ratio between what moved and what did not
  *is* the pipeline this claim prices. A scene that dirties everything measures a
  cold frame every frame and bounds a renderer on a state real interfaces are in
  once.
- **Many tiny paths, one enormous one, and a translucent layer over both.** The
  ruler's 240 two-segment ticks are what the binning stage costs when nothing
  amortises it; the waveforms are 32 768 segments of a path nobody authored by
  hand, which is what an audio timeline actually contains; and the selection
  overlay is the blend depth without which a frame measures coverage and reports
  a number that reads as a whole frame.
- **Body text at 14 px, in quantity, in eight scripts.** Section 08's *Text,
  honestly* splits a coverage atlas for small static text from GPU path rendering
  for large or transformed text at exactly that size, so a scene without small
  text exercises neither side of the split. Eight scripts because
  `text/src/corpus.rs` argues one at a time what each one's absence would hide,
  and a scene that drew only Latin would be the scene that module exists to
  refuse.
- **Semantics in the same tree, costing the walk.** 117 semantic nodes
  contributing nothing to the picture and not free. Leaving them out would price
  a pipeline this system is not going to run.

## What it does not carry, and why that is not an omission

**No string of text.** Every glyph run names a corpus entry and a scalar count.
The corpus is the one place a sample is written, so this scene cannot drift from
it, and a script removed from the corpus is a scene that cannot be built — which
is the failure this arrangement wants rather than one it tolerates.

**No node dump.** 995 rows is a file nobody reads and a diff nobody reviews. What
is written is every count a renderer can be held to, which is why the census has
two relations over it rather than one: the parts sum to it, and the per-kind rows
sum to its node count.

**No timing.** Not one number in `scene.toml` is a cost. The costs are
`claims/0033`'s four rows and its twelve recorded metrics, and a scene file that
carried a second opinion about them would be the thing `claims/README.md` exists
to prevent.

**No second scene.** `claims/0033`'s discipline is one scene, one machine, one
invocation, four renderers, and a rung timed on an easier frame than the rung
above it is not a ladder. A second entry in this directory would be that, so
there is no entry format here and no instruction for adding one.

## What a run has to record about it

Two things beyond the numbers, both of which this scene forces:

1. **Which frame.** The scene declares its dirty subtree, so a run reports the
   frame in which `playhead` moved and nothing else did. The first frame of this
   scene is a different number and is not what the claim's rows bound.
2. **What each rung did with the two effect declarations.** A material is a
   stage of the fine raster on rung 1 and is not expressible as triangles at all
   on the floor, so a rung that skipped or reduced it rendered a cheaper picture
   of a different scene. `claims/0033` already says three of its four rungs
   produce the same image and the fourth does not; this is the part of that
   sentence a number can be made to carry.

*What would reverse the choice:* a measurement showing that this scene's cost is
dominated by one part to the point where the other eleven are noise — the
waveforms are the candidate, at 92 per cent of the segment count. That would make
it a polyline benchmark wearing an interface's clothes, and the repair is fewer
waveform segments rather than a different scene, because the part of the argument
that would have failed is *proportion* and not *composition*. The choice itself
reverses if a corpus of real ported interfaces — `claims/canvas-corpus/`, when
somebody who is not us fills it — shows that timelines are unrepresentative of
what applications on this system actually are. That is the one observation that
could not be had from here, and it is the reason this file names the section it
took its hard case from rather than claiming the case itself.
