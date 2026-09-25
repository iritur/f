---
id: 0012
status: draft
reviewed_by:
skills: spec-from-intent, claims-registry, rfc-author, determinism-review, licence-boundary, frame-and-unsafe, memory-ordering
---

# Spec: seventy-two tasks behind the eight coarsest lines in E3

`E3-00` asks that E3 contain no `XL` task without a decomposition, and its own
line says the deliverable is an entry here rather than an edit to `TODO.md`.
This spec is the decomposition: four `XL` tasks and four `L` tasks become
seventy-two, each with an exit somebody other than its author could observe and
a `needs:` that names what actually stands in front of it. It is written in
`TODO.md`'s line format so that adopting it is a paste rather than a
transcription, and it is written the day RFCs 0077 to 0080 were accepted so that
the subtasks are about the vocabulary, the canvas floor, the token layer and the
ladder that now exist rather than about four things somebody was going to
decide.

The first draft of this file was refused by an adversarial review on the same
day, and most of the refusal was right. What changed is recorded in *What the
review changed* at the end, one finding at a time, including the one it was wrong
about and the one that is owed to a task rather than to this file. The two
changes that matter to a reader who reads nothing else: **every `XL` now names
its own subtasks in `needs:`**, without which the decomposition is invisible to
the one command nominated as its evidence; and **the count is seventy-two rather
than seventy-four**, because two proposed lines carried an observation that
already belonged to their parent and the honest fix was to withdraw them rather
than to write the sentence twice.

## What this entry can and cannot close

`E3-00`'s exit is a property of `TODO.md`'s E3 section and `intent.md` states
once where it stands. The consequence for reading this file: the `Behaviour`
section describes the graph **after** the *Handoff* is executed, not the graph
today.

What is owed here instead is that the *Handoff* be mechanical — every edit
written out verbatim, in the order it must be made, with line numbers and quoted
strings checked against `TODO.md` rather than remembered — so that no judgement
is left stranded in a draft for the person making the paste.

## Behaviour

The observable is the graph, **after the paste**. `cargo xtask todo E3` then
reports, and the arithmetic is written out so the next reader can check it
rather than trust it:

```
  20   E3 lines today          E3-00, four D, seven B, seven P, one R
+ 72   subtasks pasted         12 B01 + 12 B02 + 11 B03 + 5 B04
                               + 6 B05 + 13 B06 + 7 B07 + 6 P01
-----
  92   E3 lines after
-  5   done                    E3-00, and E3-D01..D04 ticked by Handoff step 3
-----
  87   open, of which
   6   ready                   E3-B01a E3-B02a E3-B03a E3-B04a E3-B06a E3-P01a
  81   waiting
```

Six, and no seventh, and that is true **only because each `XL` now names its
children**. Before that edit the answer is nine: `E3-B03` and `E3-P01` carry no
`needs:` line at all and are ready today; `E3-B02` needs only `E3-D04` and
`E1-B04`, and `E1-B04` is `[x]` while step 3 ticks `E3-D04` in the same edit. The
first draft of this spec predicted six and would have produced nine, which is
the one arithmetic a decomposition is not allowed to get wrong, because the
number it gets wrong is the number it exists to produce.

Every task that reports as blocked names a blocker that is a subtask in this
decomposition, or one of exactly fourteen tasks outside it, listed in full under
*Evidence*. No task reports as available because its line forgot to say what it
needs — which is the reading the epoch's standing paragraph corrected by hand on
2026-09-09 and which a decomposition can make four times worse.

## Six rules

Six rules were applied to every line below, and a proposal that broke one was
rewritten or withdrawn rather than kept.

**An `XL` names its decomposition in `needs:`, and keeps its own line.** This is
the rule the first draft did not have, and without it the seventy-two lines are
peers that happen to share a prefix: `xtask`'s `Task` struct carries `id`,
`status`, `size`, `title`, `needs`, `epoch` and `standing` and has no notion of a
parent, so a relation that is not an edge is not in the graph at all. Three
options were open and two are rejected in *What the review changed*, finding 4.
The parent keeps `XL` because `TODO.md`'s own size rule presumes it does — *an
`XL` that is not decomposed **by the time it starts** is a planning failure* is a
sentence about an `XL` that persists and acquires a decomposition, not about one
that is deleted — and because three claim files and one `ROUTES` entry already
point at parent ids: `claims/0033`'s route is `Route::Unbuilt("E3-B02")`,
`claims/0035` names `E3-B01`, and `claims/0034` names `E3-B06l`. A parent's
`needs:` is exactly its children, plus any external task its own exit needs that
no child carries; the two exceptions are `E3-B04` and `E3-B07` and both say so on
the line.

**An exit is an observation, and it belongs to one task.** Not *implemented*,
not *working*, not *the renderer is correct*. A test that fails today and passes
then; a number in a file; a boot that reaches a stage it did not reach. Where
the honest exit is another task's observation, the task was merged into that one
or the observation was moved — because `E0-B12`, `E1-B01` and `E1-B05` each
recorded what happens otherwise, which is that one of the two tasks is
permanently lying about its state. The first draft broke this rule six times
over in its own new lines, by writing *the parent's exit* on a subtask while the
parent kept it; those six are resolved one at a time in *What the review
changed*, finding 6, and **no line below carries an exit that any other line in
`TODO.md` also carries — with one exception, named on its own line rather than
left to be found.** `E3-P01f` carries `E3-R01`'s exit word for word, because that
sentence is an observation about the rig's artefact and not about release 0.4;
`E3-R01` is not this entry's line to edit, so the duplicate is made obvious and
*Handoff* step 9 is where `E3-R01` gets an exit of its own. Until that step
lands, this is the fourth instance of the defect `E0-B12`, `E1-B01` and `E1-B05`
each recorded, and it is E3's rather than this decomposition's.

**A `needs:` names what is missing, not what is related.** Several parents name
`E0-B12` and `E0-B15`, and both of those are `[>]` for a number and for Intel
UINTR respectively — the ring's code, its layout, its cursor protocol, its
suppression and the kernel doorbell path are all in the tree and green.
`E0-B12`'s own line says the missing half *belongs to `E0-P05`*. So no line below
needs either, and the parent's replacement `needs:` drops them: nothing in this
decomposition wants the submit-latency number or the user-interrupt path. That
is the single clearest thing the decomposition has to say about the parent lines,
and it is the one *Handoff* step that changes the graph rather than the prose.

**A decision sits immediately before the work it protects.** Ordering rule 3.
Two decisions are owed inside this epoch and both are the first subtask of the
build task that needs them: what happens when two vocabulary versions meet across
a ring (`E3-B06a`, which RFC 0077 names as its own fourth reversal condition and
dates to the day the tree gets an entry format), and where the text shaper comes
from (`E3-B03a`, which is the licence boundary's first real test). Promoting them
to `E3-D05` and `E3-D06` changes nothing about the argument and is the
originator's call.

**A `needs:` line says nothing in prose that it does not mean as an edge.**
`xtask`'s `ids_in` splits the whole `*needs:*` line on anything that is not
alphanumeric or a dash and keeps every token shaped like a task id — so a task
named inside the explanatory parenthesis becomes a blocker, silently and with no
diagnostic. The parentheses on the lines below therefore explain *why* without
naming another task; where the argument needs a second id, it is in the `*exit:*`
line, which nothing parses. This is check 5 under *Evidence*, and it is the one
rule here that came from reading `parse_todo` rather than from reading
`TODO.md` — the first revision of this spec added three unintended edges in
parentheses it thought were comments.

**Nothing invents a number.** Every threshold in E3 is a target and says so.
Four of the seventy-two exist to move a claim off `pending` or to register one,
and each names which, because `intent/0008` found that four `pending` rows with
four good reasons and no owner add up to a story that is argued rather than
measured.

---

## E3-B01 — the retained scene graph

Twelve subtasks. The parent's exit — *boundary crossings per UI frame under 10,
counted rather than estimated* — is a **count**, and `claims/0005` established
that a count can gate on the machines this project has. So the parent is not
gated on hardware at all, which one line could not say; what it is gated on is a
compositor existing, and the compositor is a component, and the supervisor that
spawns one paid RFC 0008 on 2026-09-13.

`E3-B01j` is the instrument and the parent is the threshold, and they are two
observations rather than one: `E3-B01j` closes when the two sides count and
agree, whatever the number is, and the parent closes when that number is under
ten. The edit that separates them: make the frame's counter and the component's
counter disagree and `E3-B01j` goes red with the parent untouched; leave them
agreeing at eleven and the parent goes red with `E3-B01j` green.

The order below is the order the risk falls in. The wire format is first because
ordering rule 1 puts it there and because the moment there is a client there are
two peers; atomicity is third because a torn frame has no recovery the user will
not see, and because `E2-P01`'s cut model already exists and can be pointed at a
commit instead of a publish — which is this epoch's first inheritance from the
last one.

```
- [ ] **E3-B01a** `M` The scene-delta entry format in `abi/`: `CreateNode`, `SetTransform`, `SetPath`, `SetPaint`, `RemoveNode`, `Commit`, over part I's envelope.
  *exit:* every opcode round-trips through fixed-width bytes on x86-64 and AArch64; an entry with a non-zero unread field is refused rather than ignored; the commit carries the frame token and the deadline it was scheduled against.
  *needs:* E3-00 (ordering rule 1: two peers read this the moment a client exists, so it lands before the compositor does)
- [ ] **E3-B01b** `S` The scene node kinds as types — `Transform`, `Clip`, `Layer`, `Draw`, `Effect`, `Semantic` — and nothing else.
  *exit:* each kind is created and removed by a delta and by no other route, and a seventh kind is a compile error in every consumer that decides per kind without a wildcard — and the one consumer in this workspace that decides per kind is such a consumer. Narrowed from *a compile error in every consumer, the way `Role` already is* by RFC 0084, because a seventh kind was added end to end and broke exactly one build, and `Role` does not achieve the wider sentence either.
  *needs:* E3-B01a
- [ ] **E3-B01c** `M` The retained graph itself: an arena with a named maximum, no allocator, no `Vec`.
  *exit:* a scene at the maximum is built, mutated and torn down inside one fixed allocation, and exceeding it is a named refusal rather than a panic.
  *needs:* E3-B01b
- [ ] **E3-B01d** `M` The commit is atomic: every delta in it, or none of them.
  *exit:* `E2-P01`'s cut model, pointed at a commit instead of a publish, cuts at every entry boundary across a seed sweep, and over a **model of** a ring whose `Release`/`Acquire` pair holds the graph read back is the old scene or the new one, never a third; over a model of a ring whose pair does not hold, third scenes are produced and the sweep is required to count them. Narrowed from the unqualified sentence by RFC 0084: a slot whose write is not visible reads as the previous frame's entry, which decodes, and closing that is a per-entry submission sequence declared in `abi/` **and checked in the commit path** — the earlier wording said `abi/` alone, on a premise about `f_abi::Sqe` that was false and is corrected in RFC 0084. Narrowed a **second** time to *a model of a ring*, and RFC 0084 does not yet record that one — **the amendment is owed and is named here rather than cited falsely**: `f-scene` depends on `f-abi` alone, no run in this workspace drains an `f_ring` into `commit::Batch`, so `Mode::Honest` is this file's statement of what the ordering pair means and not a reading of it. The run that would make it a reading belongs with `E3-B01g`, the first task with a ring between two components.
  *needs:* E3-B01c
- [ ] **E3-B01e** `M` Dirty-subtree tracking: what a commit marks, and what the encode stage is then allowed to walk.
  *exit:* one changed node in a scene of a thousand marks one subtree, and the number of nodes the encoder visits equals the number in that subtree — counted in a test rather than sampled in a profile.
  *needs:* E3-B01c
- [ ] **E3-B01f** `M` The compositor is a component: a manifest, a declared state tree, spawned by the supervisor that now lives above the frame.
  *exit:* a boot brings up a compositor at ring 3 that adopts its control ring and a client's data ring and publishes its tree; a build whose manifest declares no tree is refused at spawn, which `ADMISSION/NO_STATE_TREE` already does.
  *needs:* E3-B01c, E1-B05
- [ ] **E3-B01g** `M` The delta ring between two components, and the doorbell the compositor sleeps on.
  *exit:* cross-core delivery is observed for the first time — `E0-B15` left it unproven and said it belonged with the component that would actually sleep on a doorbell; one boot shows a client's commit waking a parked compositor, and a batch still rings at most one doorbell.
  *needs:* E3-B01f, E0-B13
- [ ] **E3-B01h** `M` Frame pacing: the application's deadline computed backwards from the next scanout, from the compositor's own rolling p99.
  *exit:* the wake time is scanout minus the p99 estimate minus the margin, every term an integer with its scale in its name, and two runs from one seed compute the same wake time to the tick on both architectures.
  *needs:* E3-B01f
- [ ] **E3-B01i** `M` Late-latch: pointer and stylus re-read immediately before submit and patched into the transform node.
  *exit:* a frame trace shows the latched transform differing from the committed one by exactly the motion injected between commit and submit, and by nothing else.
  *needs:* E3-B01h, E3-B04a
- [ ] **E3-B01j** `M` The boundary-crossing counter, on both sides, and its claim row.
  *exit:* the frame and the component each count the crossings of one UI frame, the two counts are required to agree and a deliberately miscounted side goes red, the number is printed at boot, and `claims/` gains a row carrying it — **whatever the number is**. The threshold is `E3-B01`'s exit and not this one: this task closes honestly with the count at forty-seven, and the claim is a count, so it may gate on the machines this project has in the way `claims/0005` established.
  *needs:* E3-B01g, E3-B01h
- [ ] **E3-B01k** `S` The compositor publishes the frame's story into its own state tree: rung, frame token, deadline, pacing estimate, and what was degraded.
  *exit:* a boot reads all five back out of the component's subtree rather than out of the serial log.
  *needs:* E3-B01f, E3-D04, E1-B15
- [ ] **E3-B01l** `L` Reconciliation in the client library: immediate-mode in, deltas out — section 11's concession, and the one that decides adoption.
  *exit:* a client that rebuilds its whole tree every frame emits exactly the differences, asserted against a hand-written diff over a corpus of edits that includes a reorder, which is the edit this kind of reconciler gets wrong.
  *needs:* E3-B01a
```

## E3-B02 — GPU compute rasterisation, climbing a named ladder

Twelve subtasks, and they are different tasks than they would have been on
2026-09-13. RFC 0080 named four rungs, fixed their order by *fidelity* rather
than by cost, registered each rung's cost as a row of `claims/0033` before
anything could produce one, and left exactly one of a rung's three obligations
in prose: *what the machine must supply*, because expressing it needs a
vocabulary for what a backend reports about itself and there is no backend. That
RFC says `E3-B02` owes the type. It is the first subtask, and until it exists a
machine's rung is something a human matches against a paragraph.
`interface/src/ladder.rs` as it stands carries `CLAIM`, `RUNGS`, `Fidelity`,
`CpuCoverage`, `Rung`, and on `Rung` the `ALL`, `name`, `cost_metric`,
`cpu_coverage`, `fidelity` and `below` accessors. Every one of those is a
statement about a *rung*; none is a statement about a *machine*, and the
difference is the gap — read from the file at the end of this work rather than
from memory, because that module is being edited in parallel with this one.

Six of the twelve name a machine in their own `needs:` and three more reach one
through a sibling. `E3-B02h` and `E3-B02l` are two of that second three, and they
are the reason the parent's exit had
to be split: *the fallback ladder is exercised on hardware that cannot run the
top rung* is a sentence about a machine with no GPU, which is every machine this
project has, and it was sitting in an exit whose other half needs a purchase
order. It moves to `E3-B02l`, where it can be observed the day rung 3 exists.

```
- [ ] **E3-B02a** `M` What a backend says about itself — the vocabulary RFC 0080 deliberately left in prose.
  *exit:* each rung's *what the machine must supply* column becomes a predicate over a reported capability set, four synthetic backends select rungs one to four in a test, and no clause of that table is left in a paragraph. The test matches exhaustively on `Rung`, so a fifth rung is a compile error here rather than a rung nothing selects.
  *needs:* E3-00, E3-D04
- [ ] **E3-B02b** `M` The rung is chosen once, at start, by the compositor, and never promoted.
  *exit:* a backend with compute shaders and no usable scan starts at rung 2 and says so in the compositor's tree; a backend satisfying no rung is refused a compositor rather than handed the floor; an overloaded compositor holds its rung.
  *needs:* E3-B02a, E3-B01k
- [ ] **E3-B02c** `L` Rung 1, stage 1 — encode: the dirty subtrees flattened into a linear encoding of paths, transforms and paints. The only stage that touches the CPU, and the only stage of rung 1 observable without a GPU.
  *exit:* the encoding is byte-identical for one scene on both architectures, and the encoder visits only what `E3-B01e` marked.
  *needs:* E3-B01e, E3-B02b
- [ ] **E3-B02d** `L` Rung 1, stage 2 — coarse raster: segments binned into 16x16 tiles, a prefix scan computing per-tile counts and offsets.
  *exit:* counts and offsets are bit-identical to a serial reference over the scene corpus, and the test names the scan as the one stage that requires cooperating lanes — which is what rung 2 exists for.
  *needs:* E3-B02c, E5-D01
- [ ] **E3-B02e** `L` Rung 1, stage 3 — fine raster: exact coverage per pixel, composited in linear space, effects as stages of this pass rather than as separate targets.
  *exit:* one scene renders to a reference within an integer tolerance, and the number of intermediate targets a scene forces is counted and is zero for scenes that do not genuinely need one.
  *needs:* E3-B02d, E5-D01
- [ ] **E3-B02f** `M` Rung 1, stage 4 — present: the swapchain write, or the surface handed straight to the display controller when one surface covers the screen.
  *exit:* a trace shows no composition at all in the single-surface case, and the present path costs one MMIO write rather than a system call.
  *needs:* E3-B02e, E5-D01, E1-B04 (the backend this rides, named here rather than on the parent, because this is the line that touches it)
- [ ] **E3-B02g** `L` Rung 2, the hybrid: the scan moves to the CPU and everything else stays on the GPU.
  *exit:* on a backend whose scan is refused, the hybrid's image is *identical* to rung 1's on the same scene — identical, because `Fidelity::Exact` is a claim this test either keeps or breaks.
  *needs:* E3-B02f, E5-D01
- [ ] **E3-B02h** `L` Rung 3, the all-CPU raster.
  *exit:* identical again, with the backend absent rather than degraded; the control is that the test runs on a machine with no GPU at all, which is every machine this project currently has.
  *needs:* E3-B02e (the image it must match is rung 1's, which is why this line waits on a machine its own test does not use)
- [ ] **E3-B02i** `L` Rung 4, the tessellating floor.
  *exit:* an image that is *not* identical, whose difference from rung 1 is bounded and characterised, and which no consumer treats as exact — `Fidelity::Approximate` is read by something rather than merely declared.
  *needs:* E3-B02h, E5-D01 (RFC 0080's fourth column for this rung is *a fixed-function triangle pipeline*, which is a machine and not a fallback from one)
- [ ] **E3-B02j** `M` The scene `claims/0033` deliberately did not fix.
  *exit:* one scene chosen and named in the claim's `[workload]` row with its content hash, and the argument recorded is representativeness rather than reachability — the claim says a scene chosen this early is chosen to be reachable, and this is the task that must not.
  *needs:* E3-B02a (**corrected on 2026-09-24.** This line named `E3-B02i` when it was written — rung 4, on a GPU — and its own exit is a choice, an argument and a content hash. RFC 0088 settled that a decide task's exit may not require its own implementation, and waiting on every rung would have made this *a scene chosen for reachability*, which the exit above forbids. Found by a reader comparing this file against `TODO.md`, which is the comparison that catches a spec and a tracker drifting apart)
- [ ] **E3-B02k** `M` Four rungs, one scene, one machine, one invocation: `claims/0033` moves off `pending`.
  *exit:* `cargo xtask claim raster-cost-per-rung` produces four numbers from one build instead of the `Route::Unbuilt("E3-B02")` refusal it produces today, or refuses again and names which rung would not build; a run that rebuilt between rungs is refused by the harness rather than averaged.
  *needs:* E3-B02j, E0-D10, E0-P18, E5-D01
- [ ] **E3-B02l** `S` The ladder exercised on hardware that cannot run the top rung — moved here from the parent's exit, because it is an observation about a machine with no GPU.
  *exit:* a machine reporting no compute shaders starts at rung 3 and renders, recorded with the machine named; the descent is one step, and no total order over hardware is computed anywhere, because none exists.
  *needs:* E3-B02b, E3-B02h
```

## E3-B03 — text

Eleven subtasks, and the first of them is a decision because the licence
boundary has never been tested and this is where it gets tested. Every shaper
worth importing is somebody else's source under somebody else's licence, and
`LICENSING.md` and RFC 0003 say the permissive tree never imports
`third_party/` and reaches it only over a ring. Deciding that after the shaping
cache is written is deciding it in front of code that already assumes an answer.

The rest is section 08's *text, honestly* paragraph turned into tasks: shaping
on the CPU cached by string, face, size and features; a coverage atlas for small
static text; GPU path rendering where the atlas would thrash. Two constraints
bite harder here than anywhere else in the epoch. Text metrics are the single
most natural place in this system to reach for a float, and RFC 0004 forbids
both types on both architectures. And a shaping cache is the single most natural
place to reach for a `HashMap`, whose iteration order is seeded per process —
which would make cache statistics differ between two runs of one seed and quietly
destroy the only property that makes the cache testable.

This parent's exit is the one line in the epoch this decomposition does **not**
propose replacement text for, because it contradicts `E3-P05`'s and resolving a
contradiction between two written exits is a reversal that needs an RFC. That is
*Handoff* step 8 and it is the originator's.

```
- [ ] **E3-B03a** `S` **Decide where the shaper comes from**: imported behind the licence boundary, or written in the permissive tree.
  *exit:* an RFC, accepted, naming which and what would reverse it, and — because it answers *imported* — naming the task that lands the entry and the two files that make the ring observable; and `cargo xtask lint` refuses every route from the permissive tree into `third_party/` **that reaches code the lint compiles**, and prohibits the build-script, symlink and configuration surfaces outright rather than inspecting them, with a fixture driving each route red and a fixture recording each of the three routes it cannot see. Narrowed from *every route … other than a ring* by RFC 0092, before the mechanism was built rather than after it: a line-oriented text matcher cannot make a universal claim over a grammar it does not parse, three review rounds defeated the old check by about one character each, and the three routes that survive the new one — a configuration `lint` never compiles, a proc macro reading an imported file at expansion, and a vendored copy taken by registry name outside `third_party/` — are named there rather than left as a remainder. Narrowed earlier from *if imported, `third_party/` carries the entry and `cargo xtask lint` shows the permissive tree reaching it over a ring and by no other route* by RFC 0088, because this is an `S`-sized decide task whose own decision fired a conditional asking for an import, a manifest and a `shape` protocol that no amount of deciding produces; the entry, the ring it is reached over and `"third_party"` in the root `exclude` move to the import task RFC 0082 writes out.
  *needs:* E3-00 (ordering rule 3: this sits immediately in front of every line below it)
- [ ] **E3-B03b** `M` A face is addressed by content hash and declared before it is used.
  *exit:* a boot loads a face out of the blob store by hash and refuses one the manifest did not declare — the first consumer of E2's addressing that is not E2.
  *needs:* E3-B03a, E2-B01
- [ ] **E3-B03c** `M` The shaping cache, keyed by string, face, size and features.
  *exit:* no `HashMap` and no `HashSet`; from one seed the hit and miss counts are identical across runs and across architectures, which is the property a cache keyed by a hashed string loses silently.
  *needs:* E3-B03b
- [ ] **E3-B03d** `S` Every metric in the text path is a fixed-point integer with its scale in its name.
  *exit:* no `f32` or `f64` reaches the tree, `DETERMINISM_ALLOW` gains no entry, and the rounding rule is stated once where advances accumulate rather than at each call site.
  *needs:* E3-B03a
- [ ] **E3-B03e0** `M` The Unicode data arrives: the tables generated, the corpus committed, and the boundary taught the difference.
  **Added on 2026-09-24, and this decomposition not containing it is the finding.** `E3-B03e` was refused twice for want of two decisions that were not its own to take — a licence decision and a corpus decision — and RFC 0114 and RFC 0115 take them. What is left over is an import with no owner, which is what this line is.
  *exit:* `third_party/unicode/` carries `LICENSE` and a `PROVENANCE.md` naming upstream, the Unicode version, every file and its SHA-256; `text/src/` carries the generated `Bidi_Class` and bracket tables, each opening with the dual SPDX line and a header naming the upstream file, its hash and the command that rebuilt it; `LICENSING.md` carries the third row and the sentence a redistributor of a *binary* needs, since a binary carries no file headers; and `cargo xtask lint` shows the four checks RFC 0114 specifies, each demonstrated red.
  *needs:* E3-B03d
- [ ] **E3-B03e** `L` Bidirectional reordering, UAX #9.
  *exit:* the conformance corpus passes at a level named in the task rather than implied by the code, and the cases that level excludes are listed where a reader will find them.
  *needs:* E3-B03c, E3-B03d, E3-B03e0
- [ ] **E3-B03f** `L` Line breaking and cluster boundaries, UAX #14 and UAX #29.
  *exit:* the conformance cases pass, and no break falls inside a cluster — asserted over the corpus rather than over an example.
  *needs:* E3-B03e
- [ ] **E3-B03g** `M` The coverage atlas for small static text.
  *exit:* residency is bounded by a named maximum, and a scene whose text did not change regenerates no atlas entry, counted per frame rather than asserted.
  *needs:* E3-B03f, E3-B02e
- [ ] **E3-B03h** `M` GPU path rendering for large, animated and transformed text, and the rule that chooses between it and the atlas.
  *exit:* the rule is data a test reads rather than a constant in a branch, and one glyph at two sizes an order apart takes both paths and is correct on both.
  *needs:* E3-B03g
- [ ] **E3-B03i** `M` The script and direction corpus: chosen, named, and argued one entry at a time.
  *exit:* the list is in the tree with a sentence per entry saying what its absence would hide, and adding a script is a diff to that list rather than to a test.
  *needs:* E3-B03a
- [ ] **E3-B03j** `M` How a rendering is compared to a reference, and where in this tree that is allowed at all.
  *exit:* the comparison is an integer over pixels with a stated tolerance and a deliberate one-pixel regression goes red; and the carve-out from `E3-P05` is RFC 0091, which narrows that line to interface assertions and confines an image-to-image comparison to this task's corpus — cited by number rather than assumed, because as the two parent lines stood they contradicted each other. The lint that entry's fourth point owes lands here too: in `lint_all`, refusing an image-to-image comparison outside this corpus's directory, in the shape `lint_licensing` already has, because a carve-out enforced by memory grows one exception at a time.
  *needs:* E3-B03i, E3-B02e
- [ ] **E3-B03k** `M` Text joins the vocabulary: `Role::Text` and `Role::Label` reach the rasteriser and the screen reader from one declaration.
  *exit:* one tree produces a rendered paragraph and a spoken phrase with no second declaration and no text-specific branch in either projection.
  *needs:* E3-B03f, E3-B06i
```

## E3-B06 — the semantic layer and its projections

Thirteen subtasks, and this is the task the four decisions changed most. It is no
longer *design a semantic layer*: the vocabulary is twenty-two closed roles with
indices, families and a parent rule; a canvas is admitted by `Arrangement::admit`
or it is an `Escape`; a theme has already been resolved once per ground by
`token::resolve` into a `Resolved` and a `Report` that a projection paints from
without checking anything. What is left is the stages of section 12 — resolve,
solve, emit — and four projections over a closed enum, each of which is total by
construction and none of which may carry a wildcard arm.

The parent's exit cannot stay as *`E3-P04` passes*: `E3-P04` needs `E3-B06`, so
as the two lines are written neither can be observed before the other. *Handoff*
step 7 replaces it with the one property no single subtask can observe: a
twenty-third role added to the `vocabulary!` invocation in `node.rs` must break
**all four** projections at once, which is a fixture over the set rather than an
assertion inside any one of them. The per-projection halves of that property are
on `E3-B06g` to `E3-B06j`, one sentence each, because a parent exit demanding
something no child produces is a parent exit nobody can meet; the
authored-against-projected equivalence stays `E3-B06f`'s, where it is the emit
stage's own observation; and the four-projection demonstration stays
`E3-P04`'s.

Two things in here pay debts the RFCs recorded rather than inventing work. RFC
0077's fourth reversal condition is *a tree crossing a ring between different
vocabulary versions*, and it dates itself: the day the semantic tree gets an
entry format in `f_abi`. That day is `E3-B06b`, so the decision is `E3-B06a` and
comes first. And RFC 0079 names the way its own decision is most likely to be
quietly undone — a projection that stops asking `Resolved::on` for a pair and
starts remembering an ink's colour, which will look like a cache and will pass
every test in `token.rs` while the interface goes unreadable on the grounds
nobody checked. That is a lint, and `E3-B06d` is where it is owed.

```
- [ ] **E3-B06a** `M` **Decide what happens when two vocabulary versions meet across a ring.** RFC 0077's fourth reversal names this day: the day the semantic tree gets an entry format.
  *exit:* an RFC in RFC 0011's shape, accepted, naming what a receiver does with a role it has never heard of and why that is not the wildcard arm this vocabulary was closed to prevent, arriving by a route the compiler does not cover.
  *needs:* E3-00, E3-D01
- [ ] **E3-B06b** `M` The semantic entry format in `abi/`: `DeclareNode`, `SetState`, `SetContent`, `SetRelations`, `Remove`, `Commit`, over part I's envelope.
  *exit:* the twenty-two role indices on the wire are `Role::index`'s, by a test that fails if either moves; an index the vocabulary does not name is refused at the boundary rather than mapped to anything.
  *needs:* E3-B06a
- [ ] **E3-B06c** `M` The system owns the tree and the application holds a handle — section 11's inversion, built.
  *exit:* a component that dies leaves its declared tree readable and addressable, and the handle does not survive it; one test shows both halves, because either one alone is the ordinary behaviour of something else.
  *needs:* E3-B06b, E3-B01f
- [ ] **E3-B06d** `M` Resolve: a `Theme` becomes a `Resolved` against the display's real characteristics, once per ground, with the `Report` carried to the caller rather than dropped.
  *exit:* the compositor holds a `Resolved` and contains no readability arithmetic at all; and a lint refuses any type outside `interface/src/token.rs` that holds an `Rgb` without the `Token` pair it was checked against, with a fixture that makes the lint fail, which RFC 0079 names as the thing to watch first.
  *needs:* E3-B06c, E3-D03
- [ ] **E3-B06e** `L` Solve: incremental constraint layout. The stage the pillar dies at, if it dies.
  *exit:* work proportional to the change — a delta touching five nodes of ten thousand re-solves five subtrees, counted; a cascade past a stated scope fails the test rather than slowing the frame.
  *needs:* E3-B06d
- [ ] **E3-B06f** `M` Emit: scene deltas into part II's retained graph, projected from types that exist.
  *exit:* the compositor cannot tell an authored delta from a projected one — one scene fed both ways produces byte-identical graphs, which is the single property part III asks of part II so that this layer can be abandoned without taking the renderer down.
  *needs:* E3-B06e, E3-B01e
- [ ] **E3-B06g** `M` The display projection, end to end.
  *exit:* the settings-panel tree `interface/src/node.rs` already builds reaches pixels with no application code beyond its declaration, and this projection matches `Role` exhaustively with no wildcard arm — a role it has nothing to draw is a named refusal rather than a silent skip.
  *needs:* E3-B06f, E3-B02e
- [ ] **E3-B06h** `M` The remote projection: the tree at the far side's density and refresh rate, not an encoded pixel stream.
  *exit:* one declaration presented at two densities and two refresh rates from one unmodified application; the bytes crossing the link are counted and compared against what the same frame would have cost as pixels; and the far side's `Role` match is exhaustive with no wildcard arm, which is what stops a role the remote cannot present from crossing as nothing.
  *needs:* E3-B06f
- [ ] **E3-B06i** `M` The screen-reader projection: `canvas.rs`'s `linearise` generalised from one arrangement to a whole tree.
  *exit:* every one of the twenty-two roles has a phrase and a twenty-third would be a compile error — an exhaustive `match` on `Role` in the projection rather than a table a new variant can silently miss; RFC 0078's timeline is described without a pixel, including the clip between 4.2 s and 6.8 s on track 3.
  *needs:* E3-B06c
- [ ] **E3-B06j** `M` The agent projection: the tree read directly, intents invoked as capability calls.
  *exit:* an agent selects that clip and invokes its intent; the invocation is an authorised capability call that is logged, and the same call after the capability is revoked is refused — `cargo xtask cap`'s discipline in a new place; and what an agent may do with each role is an exhaustive `match` with no wildcard arm, so a new role is a decision here rather than a default.
  *needs:* E3-B06i, E3-B06k
- [ ] **E3-B06k** `M` Canvas participation across the ring: `Arrangement` and `Placement` keyed by `NodeId`, and the floor enforced on the far side too.
  *exit:* a canvas whose placements did not arrive is refused rather than rendered — `Escape` is a wire refusal and not only a constructor's, which is what stops the floor being local to the declaring process.
  *needs:* E3-B06b
- [ ] **E3-B06l** `M` The corpus behind `claims/0034`, and the run that moves it off `pending`.
  *exit:* `cargo xtask claim canvas-escape-rate` computes `canvas_escape_rate_x1000` over a corpus of applications ported by somebody who did not write the vocabulary, records what the corpus was beside the number, and refuses when the corpus is empty rather than reporting a clean zero — which `node.rs`'s `canvas_census` already does by returning `None` for a tree with no nodes, so the two halves of the refusal cannot come to disagree.
  *needs:* E3-B06j
- [ ] **E3-B06m** `M` The corpus behind `claims/0035`, and the run that moves it off `pending`.
  *exit:* `cargo xtask claim theme-refusals` produces both of the numbers RFC 0079 named — themes resolving clean per thousand, and decisions per theme — over a corpus whose every theme was written by somebody not working on `token.rs`, with its content hash recorded beside the number; the five themes in `token.rs` are a demonstration and the run refuses them as a corpus. `claims/0035` names the parent as what moves it; the line that first brings up a compositor to hold a `Resolved` is what it actually waits for, and that is this line's second blocker.
  *needs:* E3-B06d, E3-B01f
```

## E3-B04 — the input path

Five subtasks. The parent's exit — *a full frame of latency removed, shown as
before-and-after on the rig* — is a measurement the parent takes itself, so the
parent needs `E3-P01e`, the line the rig's calibration is on. The first draft
proposed a sixth subtask carrying that same sentence; it is withdrawn, because a
subtask whose needs and whose observation are both identical to its parent's is
the parent with a letter after it.

The seam with `E3-B01i` is deliberate and is asserted from both sides, because
late-latch consuming a prediction is the one place these two tasks meet and a
seam nobody tests is a seam that drifts.

```
- [ ] **E3-B04a** `M` Timestamped at the driver, at interrupt time, and nowhere else.
  *exit:* one time source in the crates that can hold one; a lint finds any second reading of a clock in them, any crate that names the stamp and has no row, and any file compiled in from off the path, each with a fixture that makes the lint fail; under the simulator the stamp is `Env`'s virtual time and one seed gives one sequence of stamps on both architectures. **Narrowed from *one time source in the whole path* by RFC 0099**, on a measurement rather than on a judgement: `abi/`, `interface/` and `scene/` depend on neither `f-env` nor `f-input`, so a second clock reading cannot be *written* there and their rows are held open in advance rather than checked — `stage_reach` computes that and `lint-stamp` prints it on every green run. The interrupt-time call site and the stages downstream of the driver arrive with `E3-B04d`, and the reversal is mechanical: a second row reaching a clock.
  *needs:* E3-00
- [ ] **E3-B04b** `S` The input entry format in `abi/`.
  *exit:* fixed-width on both architectures, unread fields refused when non-zero, and the timestamp's scale in its name rather than in a comment.
  *needs:* E3-B04a
- [ ] **E3-B04c** `M` Prediction forward to the next scanout.
  *exit:* deterministic from a seed; over the named corpus the test sweeps — 4096 seeded recordings at two report rates, and the test asserts that 4096 were folded rather than naming the count in prose — the error is bounded by an integer stated in the test, and over-prediction is bounded separately from under-prediction because a user notices them differently. *Separately* is a claim about the instrument and is now met by it: an error is lag only when the cursor moved the way the pointer moved and fell short, so a cursor drawn the wrong way along a motion that reversed — the textbook snap-back — is over-prediction rather than lag. The test also draws ten times that corpus and is required to observe the over-prediction bound failing there. Narrowed from *over a recorded motion corpus the error is bounded by an integer* by RFC 0084, on a measurement: the integer is that corpus's own maximum rounded up to a pixel and not a property of the generator. Ten times the corpus runs the worst over-prediction from 13.44 px to 15.70 px at the recorded rate and from 15.76 px to 18.41 px at half of it — the second past the stated 16, on eight of 81 920 measurements and every one of them at the halved rate — while the two under-prediction maxima move by 4.6 and 5.1 per cent and are exceeded five times. So the tighter half of the pair, the half the module argues hardest for, is the less stable half. That pair is (16 px, 32 px) and was (8 px, 32 px) until the classifier above was repaired: the errors were always there and were being counted in the other column, so the earlier pair was a kinder ruler and not a tighter predictor — no prediction this module makes changed. `input/src/predict.rs` states the pair over the named set, pins each number to the set's own maximum in a `const` so that widening it is a build error, and carries the one universal over-prediction bound beside it as a theorem over the speed and lead ceilings: 1152 px for the two-axis sum a `Deviation` reports, seventy-three times looser, and *attained* by a prediction the test drives into that corner rather than asserted over three constants. The unqualified sentence comes back only with a predictor whose over-prediction has a closed form over the input domain, which a first-order extrapolator of a measured velocity does not.
  *needs:* E3-B04b
- [ ] **E3-B04d** `M` A real device rather than a model: virtio-input in user space, in the shape the three E1 drivers already share.
  *exit:* a driver component delivers events a compositor consumes, using `kernel/src/supervisor.rs`'s shared half rather than a fourth copy of it; hardware-timestamped native input is `E5-B06` and this line says so rather than implying it is here.
  *needs:* E3-B04b, E1-B16
- [ ] **E3-B04e** `M` Late-latch consumes the prediction.
  *exit:* the latched value is the predicted one, asserted from both sides of the seam — the input path's trace and the compositor's frame trace agree on the value and on how far it moved.
  *needs:* E3-B04c, E3-B01i
- [ ] **E3-B04f** `M` What arrived is what was sent: the crossing attested on both sides of the input ring.
  **Added on 2026-09-25, a day after `TODO.md` gained it.** Wave 13 put this line and the next into the tracker and not into this decomposition, which is the drift that comparing the two files exists to catch. RFC 0124.
  *exit:* a boot shows the entries that arrived are the entries that were sent, as two words neither side computed from the other; a reading minted between the two stages turns it red with the counts agreeing; and the frame holds no reading and names none.
  *needs:* E3-B04d
- [ ] **E3-B04g** `M` The driver's entries reach a client: the compositor drains the input ring itself.
  **Added on 2026-09-25 with `E3-B04f`.** RFC 0125.
  *exit:* a boot in which `user/compositor` holds the other end of the driver's data channel, decodes every entry with `f_abi::input::Event::decode`, rebuilds the reading with `from_wire_nanos` passed a field read, and folds what it drained into a word required to equal the driver's — with the frame relaying no input entry at all.
  *needs:* E3-B04f, E3-B01i
```

## E3-B05 — explicit synchronisation

Six subtasks. This task's exit is a negative — *no implicit wait appears in a
frame trace* — and a negative needs the trace to exist first and to be
reproducible, which is why the trace is its own subtask rather than a
by-product. The failure mode of the whole mechanism is a hang, and a hang
produces no log line, so two of the six are about refusing rather than waiting.

The parent's second clause — *frame time is bounded rather than typical* — moves
to `E3-B05d` in *Handoff* step 7, and this is the move worth reading twice: the
two clauses fail differently. The negative is observable on the machines this
project has today; the bound is a time on `runner-class-A`. Leaving them on one
line means a task that cannot close until somebody buys a machine, hiding work
that can start behind work that cannot.

```
- [ ] **E3-B05a** `M` Timeline semaphores in `abi/`: the application signals N, the compositor waits N and signals M, the present engine waits M.
  *exit:* fixed-width on both architectures, and a wait on a value nothing can ever signal is refused at submission rather than waited on — the one failure of this mechanism that leaves no evidence behind it.
  *needs:* E3-00, E3-B01a
- [ ] **E3-B05b** `M` What a frame trace is and what it records.
  *exit:* one frame's trace names every wait, its value and who signalled it, and is byte-identical for one seed — a trace that is not reproducible cannot support this task's negative.
  *needs:* E3-B05a, E3-B01f
- [ ] **E3-B05c** `M` The submission cannot express an implicit wait.
  *exit:* a submission is built by a constructor that takes its waits explicitly and refuses one that would let the driver insert its own, checked where the submission is built rather than where the trace is read — the static half of the parent's negative, and the half that still holds on a driver whose trace this tree cannot read.
  *needs:* E3-B05b
- [ ] **E3-B05d** `M` Frame time bounded rather than typical — moved here from the parent's exit, because it is a time and the parent's negative is not.
  *exit:* the maximum and the p99 are both recorded and the claim gates on the maximum — a bound whose evidence is a central tendency is not a bound.
  *needs:* E3-B05c, E0-D10, E0-P18
- [ ] **E3-B05e** `M` What happens when a wait does not arrive.
  *exit:* a timeout is a named fate, the supervisor decides it, and a boot shows a compositor restarted for one — RFC 0008 is paid, so the policy that decides is above the frame rather than inside it.
  *needs:* E3-B05b, E1-B05
- [ ] **E3-B05f** `S` The synchronisation state is in the compositor's own tree.
  *exit:* waits outstanding, last value signalled and timeouts are read back at boot out of the component's subtree.
  *needs:* E3-B05b, E1-B15
```

## E3-B07 — the degradation policy

Seven subtasks. The parent's exit has two halves that fail differently — *under
2x overload the frame rate holds* is a measurement on a machine, and *the
quality reduction is visible in the state tree, per frame* is a boot this
project can run today, because `E1-B15` closed and every component publishes a
tree. Splitting them is most of the value here, and the split is *Handoff* step
7: the second half moves to `E3-B07d` and the parent keeps the first, with
`E0-D10` and `E0-P18` on the parent's own line because that is what a frame rate
holding is measured on.

The last subtask exists because RFC 0080 forecloses a compositor that promotes
itself, and because the two mechanisms that share the word *fallback* are one
frame apart: the ladder chooses a rasteriser when a compositor starts, and this
policy downgrades an effect inside a frame that is already late. A system that
confused them would answer a missed frame by changing renderers, which is the
one response guaranteed to miss the next frame too.

```
- [ ] **E3-B07a** `M` Every effect node declares a cost estimate and a cheaper fallback.
  *exit:* a declaration naming one word and not the other is refused by `Effect::declared`, which is the only constructor of an `Effect` and whose two cost fields are `NonZeroU32`, so no value of the type can hold half a declaration and the refusal names which half was missing. Narrowed from *an `Effect` delta carrying one and not the other is refused at the boundary, so an effect with no fallback is a declaration error rather than a frame-time surprise* by RFC 0084, on two measurements. `abi/src/scene.rs` has six opcodes and none of them carries an effect's parameters, so no delta in this workspace can carry one word without the other and the exit's antecedent never reaches a decoder — the boundary that exists is the one between two integers a caller wrote and the value the crate will act on. And nothing requires a `Kind::Effect` node to declare anything at all: a node created with that kind and no declaration is accepted by `crate::arena` and `crate::commit`, which is a census neither of them keeps rather than a refusal this function could make. Both halves come back together with a seventh opcode `SET_EFFECT` carrying a node, an estimate and a saving, which is the reversal and is an ABI change.
  *needs:* E3-B01b
- [ ] **E3-B07b** `M` The downgrade priority order is data, and it is fixed.
  *exit:* the order is a table a test reads rather than the order branches happen to be written in; one overload from one seed produces the same downgrades in the same sequence on both architectures.
  *needs:* E3-B07a
- [ ] **E3-B07c** `M` The running estimate against the remaining budget.
  *exit:* integers with their scale in their names, taken from the compositor's own rolling p99 through `Env` and from no other clock; the estimate's staleness is bounded by a stated number of frames rather than assumed fresh.
  *needs:* E3-B07b, E3-B01h
- [ ] **E3-B07d** `M` The choice recorded per frame in the component's own tree — moved here from the parent's exit, because it is a boot and the parent's half is a measurement.
  *exit:* under 2x overload every frame carries the reduction it chose, read out of the component's subtree rather than the serial log; `E1-B15` closed, so the tree this is written into already exists.
  *needs:* E3-B07c, E1-B15
- [ ] **E3-B07e** `M` The compositor's submissions are bandwidth-capped.
  *exit:* an explicit cap, declared in the manifest and enforced rather than advisory, and a frame that tries to exceed it and is stopped.
  *needs:* E3-B07c, E1-B07
- [ ] **E3-B07i** `S` The compositor is the lowest-ranked deadline task on the machine.
  *exit:* the compositor's manifest declares the lowest-ranked class that carries a deadline, argued in this tree's admission vocabulary; admission honours it at the compositor's spawn and refuses the hard-class variant of the same record, and a class that could displace a deadline workload is refused wherever a manifest is read.
  *needs:* E3-B07c, E1-B07
- [ ] **E3-B07f** `M` Section 10's measurable form: a deadline workload's p99.9 must not move.
  *exit:* published either way, with the workload named and its baseline distribution established before the compositor is overloaded; a move says the isolation is decorative and is reported as that rather than re-scoped.
  *needs:* E3-B07e, E3-B07i, E0-D10, E0-P18
- [ ] **E3-B07g** `S` A missed frame never changes the rung.
  *exit:* an overloaded compositor degrades effects and holds the rung it started at, asserted in a test — the two mechanisms that share the word *fallback*, kept apart by something other than a paragraph.
  *needs:* E3-B07d, E3-B02b
- [ ] **E3-B07h** `M` The wire carries an effect's declaration.
  **Added on 2026-09-25, four days after `TODO.md` gained it, and `cargo xtask lint-exits` is what found it missing.** `E3-B07a`'s fourth adversarial round filed this line on 2026-09-21: RFC 0084 narrowed `E3-B07a` and named a seventh opcode as the reversal, and nothing owned that condition. It reached the tracker and not this decomposition, which is the drift `E3-B04f` and `E3-B04g` had three days later. The count in *Behaviour* is left as it stands, as it was for those two, because it is the arithmetic of the day the decomposition was pasted. The exit below is the one the line was filed with, from `80eb15c`, word for word. RFC 0127.
  *exit:* a `SET_EFFECT` delta carrying an estimate without a saving is refused by the decoder in `abi/src/scene.rs` — `E3-B07a`'s pre-narrowing sentence, restored where it was always meant to live, because the boundary it names is a decoder's and not a constructor's. The opcode carries a node, an estimate and a saving; it has a byte image in the per-opcode table; `the_vocabulary_is_the_one_the_design_names` moves from six opcodes to seven; and each of the five consumers that decide per opcode carries an arm — `commit::section_of`, `commit::admit`, `dirty::REACH`, `kind::Change::of` and `Arena::apply`. It widens a wire vocabulary RFC 0010 makes stable, so it is an ABI change and wants its own RFC.
  *needs:* E3-B07a
```

## E3-P01 — the photodiode rig

Six subtasks, and this is the only task in the epoch whose every line is in
bucket A or B: nothing in the graph blocks the rig. What blocks it is money, and
money is not a task in this file — a gap this decomposition names rather than
closes, and which `intent.md`'s fourth open question is about.

The parent's exit changes, and this is the one place the change is a swap rather
than a move. *The rig reproduces a known measurement on a conventional machine
within its own error bar* is calibration word for word, and `E3-P01e` is the line
calibration is on — so leaving it on the parent as well would be the defect this
spec's second rule names, one observation closing two tasks. What the parent
takes instead is the integration none of the six observes alone: the rig
**assembled**, one capture run end to end through `E3-P01d`'s path at
`E3-P01c`'s error bar. That asks for nothing this line did not already ask, which
matters — a rewrite that *adds* a requirement is as much a change to a written
exit as one that drops it, and a rig that reproduces a measurement is by
construction a rig that has been built and run. An earlier draft wrote the exit
as the rig *in service* through `cargo xtask claim`, which was new work no
subtask produced, and it is withdrawn.

`E3-P01f` is where `E3-R01`'s exit belongs and is named rather than quietly
fixed: *the rig's method is documented well enough for a third party to build
one* is a statement about this task's artefact and not about release 0.4. That is
*Handoff* step 9 and it is the originator's.

```
- [ ] **E3-P01a** `M` The bill of materials, in the shape `claims/runner-class-A.md` already uses for a machine.
  *exit:* every part named with a source and a price and the total stated; a rig nobody can order is a design for a rig.
  *needs:* E3-00
- [ ] **E3-P01b** `M` Injection: what presses the key, and how its own delay is known.
  *exit:* the injector is not the machine under test, and its delay is characterised as a distribution rather than quoted as a constant.
  *needs:* E3-P01a
- [ ] **E3-P01c** `M` Capture: photodiode, amplifier, digitiser, and the sampling rate that sets the error bar.
  *exit:* the instrument's own error bar is stated as an integer before any measurement is taken with it.
  *needs:* E3-P01a
- [ ] **E3-P01d** `S` No software timestamp anywhere in the measurement path.
  *exit:* the path from injection to capture is written out and every time in it is the instrument's; one software timestamp found in it fails this task, and the review that looked is recorded rather than assumed.
  *needs:* E3-P01b, E3-P01c
- [ ] **E3-P01e** `M` Calibration.
  *exit:* the rig reproduces a known measurement on a conventional machine within its own error bar, and the known measurement is named before the rig is built rather than chosen afterwards from what it happened to produce.
  *needs:* E3-P01d
- [ ] **E3-P01f** `M` The method, documented well enough for a third party to build one.
  *exit:* a document carrying the bill of materials, the wiring, the procedure and the calibration result, well enough that a third party can build one. That sentence was `E3-R01`'s exit as written; Handoff step 9 moved it here, because it is an observation about this task's artefact and not about a release.
  *needs:* E3-P01e
```

---

## What is available, precisely

Four buckets, every one of the seventy-two in exactly one of them, and the rule
that puts a line in a bucket is mechanical so that a reader can disagree with the
answer rather than with the author. Follow each line's `needs:` backwards until
every path ends at a `[x]` task:

- **A** — no unmet blocker at all.
- **D** — some path passes through `E0-D10`, `E0-P18` or `E5-D01`. Those three
  are what this file has instead of a purchase order: a machine named, a machine
  booted on, a workstation specified.
- **C** — not D, and some path passes through `E1-B05` or `E2-B01`, the two
  `[>]` tasks whose code is already in the tree.
- **B** — everything else: every path ends inside this decomposition or at a
  task that is `[x]` today. Not *no blocker outside the seventy-two* — five of
  these lines name `E0-B13`, `E1-B04`, `E1-B07`, `E1-B15` or `E1-B16`, and all
  of them reach `E3-00`. The property is that nothing on the path is still open
  outside this decomposition.

D is checked before C, because a purchase dominates a `[>]`. The buckets are
transitive on purpose: the question they answer is *can this be finished*, and a
line whose only blocker is a sibling that needs a GPU needs a GPU. The first
draft's buckets were asserted; these are derivable from the pasted lines and from
nothing else, which is the difference between a list a reader can check and a
list a reader has to believe.

**A — ready the morning the paste lands** (6). One wire format (`E3-B01a`, scene
deltas), one vocabulary RFC 0080 owes (`E3-B02a`), two decisions (`E3-B03a`, the
shaper; `E3-B06a`, version negotiation), one timestamping rule (`E3-B04a`) and
one shopping list (`E3-P01a`). The input *entry format* is `E3-B04b` and is in
bucket B behind `E3-B04a`, which is why this bucket holds one wire format and not
two. None of the six is a renderer.

**B — finishable without any task outside this decomposition** (20).
`E3-B01b`, `c`, `d`, `e`, `l`; `E3-B03d`, `i`; `E3-B04b`, `c`, `d`; `E3-B05a`;
`E3-B06b`, `k`; `E3-B07a`, `b`; `E3-P01b`, `c`, `d`, `e`, `f`.

Five of those twenty are the rig, and the first draft said of this bucket *no
hardware and no purchase*, which was false in the same paragraph that listed a
photodiode, an amplifier, a digitiser and an injector. The correction is not to
move the rig: it is to say what the bucket means. **Bucket B is a statement about
the graph, and the rig's blocker is not in the graph.** `TODO.md` has no line
that says *buy a photodiode*, so no `needs:` can name one, and the rig is
simultaneously the epoch's only finishable task and a task nobody can start
without spending money. Both halves are true and the file can express only one of
them. That is worth one sentence here rather than a contradiction across two
pages.

**C — waiting on one of two tasks whose code is already in the tree** (31).
Behind `E1-B05`, the supervisor, whose restart policy left the frame on
2026-09-13 and which is `[>]` because `E1-P06` has not run: `E3-B01f`–`k`,
`E3-B02b`, `c`, `E3-B04e`, `E3-B05b`, `c`, `e`, `f`, `E3-B06c`–`f`, `h`–`j`, `l`,
`m`, `E3-B07c`–`e`, `g`. Behind `E2-B01`, the blob store, because a face is
addressed by hash: `E3-B03b`, `c`, `e`, `f`, `k`. This is the bucket worth
staring at: thirty-one of seventy-two sit behind two tasks that are neither of
them a research problem.

**D — a machine somebody has to buy** (15). `E3-B02d`–`l`; `E3-B03g`, `h`, `j`;
`E3-B05d`; `E3-B06g`; `E3-B07f`. Eight name `E5-D01`, `E0-D10` or `E0-P18`
directly and seven reach one through a sibling. Two of the fifteen deserve their
own sentence, because the first draft put them here while their own exits said
the opposite: `E3-B02h` and `E3-B02l` are the CPU rung and the descent onto it,
and **both are observed on a machine with no GPU**, which is every machine this
project has. They are in D because the image they must match is rung 1's and rung
1 needs the workstation — not because their own test does. On the day a machine
is bought they are the two cheapest lines in the bucket, and on the day somebody
argues for reordering the ladder they are the two that could be brought forward.

The reading to take from the four buckets is not that a twelfth of the epoch is
available. It is that **the available twelfth contains none of the epoch's
risk**. Wire formats and decisions are the cheap half of every task here; the
expensive halves are incremental layout, a prefix scan, bidirectional text and
four rasterisers, and all four sit in C or D. An epoch that spends its available
twelfth and then stops has spent the part that teaches it least, which is worth
knowing before it is spent rather than after.

## Policy applied

**Determinism (RFC 0004)** constrains three of the eight tasks in a way that
would otherwise be discovered in review. The shaping cache cannot be a
`HashMap`, because iteration order is seeded per process and the cache's only
testable property is that one seed gives one sequence of hits. Every text
metric, every frame cost and every contrast ratio is a fixed-point integer with
its scale in its name; the ladder's claim and the token layer already do this and
the text path is where somebody will want a float. And the pacing estimate and
the degradation budget read time through `Env` and through nothing else, which
is what makes `E3-B01h` and `E3-B07b` assertable at all — a pacing loop that read
a clock directly would be untestable rather than merely non-deterministic.

**The frame (RFC 0001).** Nothing in E3 is in the frame. The compositor, the
renderer, the text stack and every projection are components at ring 3 and may
not write `unsafe`; where they must touch a device they do it the way the three
E1 drivers do, over `ring/src/device.rs`'s safe accessors, which exist for
exactly this. A proposal that needed `unsafe` above the frame would be a finding
to bring back here rather than a line to write.

**The licence boundary (RFC 0003).** `E3-B03a` is the first time this epoch's
work would cross it, and the decision is proposed before the code rather than
after. An imported shaper is reachable only over a ring; a written one is a cost
paid on purpose.

**Memory ordering.** The delta ring is an ordinary ring and rests on part I's
one `Release` store and one `Acquire` load. `E3-B01g` is the first time anything
in this tree parks waiting for a peer's commit, which makes it the first real
consumer of the suppression protocol RFC 0020 corrected — so the litmus job, not
the x86 boot, is where that subtask's evidence lives.

**Claims (`claims/README.md`).** The registry moved under this spec while it was
in review, and the first draft's *three rows are owed* is now one.
`claims/0034-canvas-escape-rate.toml` and `claims/0035-theme-refusals.toml` were
registered on 2026-09-14 by `E3-D01` and RFC 0079, and they already name the
tasks that move them — `E3-B06l` by id, and `E3-B01` for `0035`, which inside the
group is `E3-B01f`. So the row this decomposition still owes is **one**: the
boundary-crossing count, on `E3-B01j`, which is a count and may therefore gate on
the machines this project has in the way `claims/0005` established. `claims/0033`
is moved off `pending` by `E3-B02k` and by nothing else, and today its
reproduction resolves to `Route::Unbuilt("E3-B02")` and refuses, which is the
registry working rather than failing.

All three rows have a `ROUTES` entry as of this writing —
`Route::Unbuilt("E3-B02")`, `Route::Unbuilt("E3-B06l")` and
`Route::Unbuilt("E3-B01")` — so each command refuses by naming the task that owes
it a workload. An earlier draft of this file reported the last two as missing and
said so in the present tense; they were added while it was in review, which is
the same failure as finding 9 and is why nothing here states a working-tree fact
without having re-read the file it is about.

**RFCs.** Two are owed inside the epoch and both are subtasks: `E3-B06a` on
vocabulary version negotiation and `E3-B03a` on the shaper. A third is owed by
*Handoff* step 8, the `E3-B03`/`E3-P05` contradiction, and that one is the
originator's to trigger because it is a reversal of something already written
down. The parent-exit rewrites in steps 6 and 7 are **not** reversals and do not
need one, by a test worth stating because it will be asked again: *a rewrite that
changes which line carries an observation is bookkeeping; a rewrite that changes
whether the observation is required at all is a reversal.* Nothing in steps 6 and
7 stops being required by E3 before gate G3; step 8 is where one of two written
sentences genuinely has to lose.

## Not in scope

This entry decomposes eight lines and no others. `E3-D01` through `E3-D04` are
done and are not re-argued. `E3-P02` through `E3-P07` and `E3-R01` are left
whole: each is `M` or `S` and each is a measurement whose shape is decided by
the task that produces the thing measured, so decomposing them now would be
writing a test plan for code nobody has designed.

Nothing here changes `TODO.md`, `interface/src/lib.rs`, `Cargo.toml`,
`xtask/src/main.rs`, `docs/rfc/README.md` or `claims/README.md`. Every line those
files are owed is in `plan.md`, addressed to whoever owns them.

And this entry does not re-cost the epoch. `TODO.md` says 4–10 person-years at
high risk, and seventy-two tasks with sizes attached is the input to a re-costing
rather than the result of one. Adding up the sizes would produce a number that
looks measured and is not, which is the failure `claims/0033`'s own preamble is
written against.

## Evidence

Five checks, and for each one the edit that makes it go red, because a check
whose failing edit nobody can name is a check nobody has tested.

**1. The count and the ranking.** `cargo xtask todo E3`, run after *Handoff*
steps 1 to 7, prints `ready to start — 6 task(s)`, lists exactly `E3-B01a`,
`E3-B02a`, `E3-B03a`, `E3-B04a`, `E3-B06a` and `E3-P01a`, then `waiting — 81
task(s)` and `done 5 · standing 0`. If step 10 is taken as well, `ready` is 7 and
the seventh is `E3-B08`; the decomposition's own six do not move.
*Red when:* a `needs:` line is dropped from any pasted line, or an `XL`'s
children are pasted without the parent's replacement `needs:` — the task appears
in `ready` and the count is 7 where it should be 6, or 9 if all three parent
lines are missed. This is the failure the epoch's standing paragraph corrected by
hand on 2026-09-09, and it is the one a decomposition can multiply.

**2. Every blocker is named, and the set of names is closed.** The complete list
of ids that may appear in a `needs:` on any of the seventy-two lines, and nothing
else may:

```
  siblings and cousins inside this decomposition   E3-B01a … E3-P01f
  the entry point                                  E3-00
  three of the four decisions                      E3-D01, E3-D03, E3-D04
  already [x], named so the line says what it rode E0-B13, E1-B04, E1-B07,
                                                   E1-B15, E1-B16
  [>], and the epoch's real front door             E1-B05, E2-B01
  the purchase                                     E0-D10, E0-P18, E5-D01
```

Fourteen ids outside the decomposition, and `E3-D02` is deliberately not among
them: the canvas floor is carried by `E3-B06b`'s and `E3-B06k`'s types rather
than waited on. The first draft's version of this check named six ids while the
spec's own lines used thirteen, so it passed by not looking — which is why it is
written as a closed set rather than as an example.
*Red when:* any pasted line names an id outside this list. The check is
`grep -o 'E[0-9]-[A-Z0-9]*' ` over the pasted block, sorted unique, diffed
against the list above; an id that does not exist at all is already fatal inside
`cargo xtask todo`, which refuses with *TODO.md references tasks that are not in
it*.

**3. No exit belongs to two tasks.** Every `*exit:*` line in E3 is distinct, and
the two that are not — `E3-B06`'s, which is `E3-P04`'s by reference, and
`E3-R01`'s, which is `E3-P01f`'s word for word — are *Handoff* steps 7 and 9.
*Red when:* a subtask is pasted carrying the sentence its parent's `*exit:*` also
carries. The first draft failed this check six times and the check did not exist;
it exists now, and the pass is the difference between seventy-four proposed lines
and seventy-two.

**4. Two blockers that were inherited rather than needed are gone.** No line in
E3 — parent or child — names `E0-B12` or `E0-B15` in a `needs:` after step 5.
*Red when:* either id appears. Both are `[>]` for a number and for Intel UINTR;
`E0-B12`'s own line assigns the number to `E0-P05`, and the ring's layout, cursor
protocol, suppression and kernel doorbell path are in the tree and green.
Inheriting those two markers wholesale is how a task that needs an afternoon comes
to look like a task that needs a purchase order.

**5. No `needs:` line names a task it does not mean.** For every pasted line and
every replaced parent line, the ids `xtask` extracts are exactly the ids
intended: `grep '^  \*needs:\*' | grep -oE 'E[0-9]-[A-Z0-9]+[a-z]?'` over the
block returns, outside the decomposition, exactly the fourteen of check 2 and
nothing else — in particular not `E3-B01`, `E3-P01`, `E3-B02` or any other parent,
none of which any subtask waits on.
*Red when:* an explanatory parenthesis names a task. That is not hypothetical:
the first revision of this spec named `E3-B01` inside `E3-B06m`'s parenthesis and
`E3-P01` inside `E3-B04`'s, and `ids_in` would have read both as blockers — a
wrong edge that no diagnostic anywhere reports, because the id exists and the
sentence reads as a comment.

**None of these five checks observes `E3-00`'s own exit.** Nothing in this tree
does: `grep -n 'XL' xtask/src/main.rs` returns nothing, so after the paste a
reader could delete all seventy-two lines and every command would still exit
zero. The mechanism that would change that is `plan.md` step 10, and it is a
proposal because `xtask/src/main.rs` is not this entry's file.

## What the review changed, and what it did not

An adversarial review refused the first draft of these three files on 2026-09-14
with eleven findings. Nine are fixed, one is refused and one is deferred; the
dispositions are here rather than only in a reply, because a reviewer who was
wrong about this tree deserves a paragraph the next reader can find, and a
reviewer who was right deserves the fix recorded beside the error.

**1. `TODO.md` is unchanged and unlinked — fixed, by conceding it.** The exit is
E3's property and this entry cannot close it. *What this entry can and cannot
close* says so at the top, `Behaviour` is now explicitly about the graph after
the paste, and `plan.md`'s *Handoff* replaces nine prose corrections with ten
numbered edits carrying verbatim replacement lines.

**2. The predicted count was impossible — fixed.** Seventy-four subtasks plus
twenty parents is ninety-four, not seventy-four. The arithmetic is now in
`Behaviour` as arithmetic: 20 + 72 = 92, less 5 done, leaves 87, of which 6 are
ready and 81 wait.

**3. *Six available and no seventh* was already nine — fixed.** It is six after
the paste, and only because each `XL` names its children; the three lines that
made it nine — `E3-B03` and `E3-P01` with no `needs:` at all, `E3-B02` needing
only two met tasks — are named in `Behaviour`, and the six were re-derived from
`TODO.md`'s own status markers rather than taken from the review.

**4. No parent-to-child edges existed — fixed, by adding them as `needs:`.**
Three options were open. *The tool grows a notion of a parent* is rejected for
this round: `xtask/src/main.rs` is not this entry's file, and a decomposition
that requires a tool change before it can be read is a decomposition that cannot
be pasted. *The parent is retired in favour of its children* is rejected on
evidence: `TODO.md`'s own rule is that ids are permanent and a dropped task
becomes `[~]` in place, `xtask`'s `done()` counts `~` as done, and a retired
parent therefore **unblocks every dependent whose `needs:` nobody remembered to
repoint** — silently, with no red anywhere in the tree. It would also break three
pointers this entry cannot reach: `ROUTES` carries `Route::Unbuilt("E3-B02")`,
and `claims/0034` and `claims/0035` name `E3-B06l` and `E3-B01`. So the parent
stays and names its children. What that does **not** fix is stated rather than
glossed: a parent can still be ticked `[x]` with every child `[ ]`, because
ticking a box is a human act and no `needs:` prevents it. That residue is finding
5's, and it goes to `E3-B08`.

**5. Nothing in the tree can go red on this exit — deferred, to `E3-B08`.** See
the last paragraph of *Evidence*. The deferral names a line, an exit, a mechanism
that needs no new field on `Task`, and the edit that would make it fail; what it
does not do is build it here.

**6. Six new instances of the defect the spec's own first rule names — fixed, one
at a time.** `E3-B01j` became the instrument and the parent kept the threshold,
which are two observations that fail on different edits. `E3-B02l` took the
ladder clause and the parent gave it up. `E3-B04f` and `E3-P01g` were
**withdrawn** — each was its parent with a letter after it — which is where
seventy-four becomes seventy-two. `E3-B05c` was re-aimed at the static half of
the negative, and `E3-B05d` took the parent's second clause. `E3-B07d` took the
parent's second half. The sentence the review quoted back — *this decomposition
does not add a fourth instance* — is now a check with a failing edit rather than
an assertion, and it is check 3 under *Evidence*. The seventh instance the review
found, `E3-P01f` against `E3-R01`, is pre-existing, is not this entry's to edit,
and is *Handoff* step 9.

**7. Bucket B contradicted the rest of the document — fixed.** The clause *no
hardware and no purchase* is gone. The buckets are defined as a backward walk
over `needs:`, which is a statement about the graph, and the paragraph after
bucket B says in as many words that the rig's blocker is money and that money is
not in the graph.

**8. Bucket D was not derivable from the pasted lines — fixed.** `E5-D01` now
appears in the `needs:` of the four GPU stages that cannot be observed without a
workstation; `E1-B04` moved off the parent onto `E3-B02f`, which is the line that
touches it; and D is defined as a closure, so the six lines that reach a purchase
through a sibling are in it by a rule rather than by assertion. `E3-B02h`'s
contradiction is resolved in the direction the review implied but stated
explicitly: it is in D because rung 1 is, and its own test runs on a machine with
no GPU — as does `E3-B02l`'s, which the review did not notice and which is the
same shape.

**9. A stale number re-asserted as present fact — fixed in `intent.md`**, and the
review's own replacement was stale within a day, which is the finding
generalised. The registry held thirty-one rows when the review counted and
thirty-three when this was written, because two of this epoch's own claims landed
in between. The sentence now carries its date and the command that recomputes it.

**10. The Evidence criterion was incomplete — fixed.** It named six ids while the
spec's lines used thirteen, so the check could pass by not looking. It is now a
closed list of every id that may appear, with the diff that makes it red.

**11. The substrate is uncommitted — refused, on the record.** The review is
right that RFCs 0077–0080, `interface/` and `claims/0033` were untracked when it
looked, and wrong that this makes a statement about them unreliable. Every intent
in this repository is written against a working tree; `intent/0006` was written
against E2's, and an intent that could only describe committed work would always
describe the epoch before its own. What *would* make the statement unreliable is
describing those files from memory, and the fix for that is to read them, which
this revision did: the four RFCs are `Status: accepted` and dated 2026-09-14;
`interface/src/` holds `canvas.rs`, `ladder.rs`, `lib.rs`, `node.rs` and
`token.rs`; `Role` has twenty-two variants, now emitted from a `vocabulary!`
invocation that also derives `COUNT`, `ALL`, `index`, `name` and the family, so
a twenty-third role is one line in one place — which is what makes the parent's
compile-fail fixture a single edit; `ladder.rs` carries `CLAIM`, `RUNGS`,
`Fidelity`, `CpuCoverage` and `Rung` with six accessors, all of them about a rung
and none about a machine, which is why `E3-B02a` exists; `canvas.rs` carries
`admit`, `Escape`, `Participating` and `linearise`; `token.rs` carries `resolve`,
`Resolved`, `Resolved::on`, `Report` and `Report::is_clean`; and `node.rs`'s
`canvas_census` returns `None` for an empty tree. Those four modules are being
edited in parallel with this entry, so every sentence here about them was
re-read from the files at the end of this round rather than carried forward. The one place the review's
point bites is ordering, and it is now a constraint on the *Handoff*: step 3
ticks `E3-D01`–`D04` and **must not be made before those files are committed**,
because a `[x]` whose evidence is untracked is a claim about a tree nobody else
has.

## What the second review changed

A second adversarial review refused the revision, with two blocking findings and
thirteen others. The dispositions, one line each, with the edit that would now
make each go red in brackets:

1. **The exit is unmet and `TODO.md` is unchanged — stands, and is not answered
   again.** Agents here may not edit `TODO.md`; the exit closes on the paste.
   `intent.md` says it once and no file argues it. [no edit: this is not a claim
   about code]
2. **Handoff step 2 reversed the exit it was closing — removed.** The step no
   longer touches the `*exit:*` line; it ticks the box and adds the intent
   linkage, and the standing property stays standing. If the standing form is
   thought wrong, that is an RFC proposed to the originator. [restoring the
   rewritten exit reinstates the reversal]
3. **`E3-B08` was specified in a command `verify` never calls, and was red on
   E4, E5 and E6 the day it landed — respecified.** It is a lint in `lint_all`,
   conditional on the epoch's `E<n>-00` being `[x]`. [moving the rule back into
   `todo`, or dropping the condition, makes it unreachable or red]
4. **The parent `E3-B06` exit repeated `E3-B06f`'s sentence — rewritten.** The
   parent now carries the all-four-at-once fixture; `f` keeps the
   authored-against-projected equivalence. [copying either sentence onto the
   other line fails check 3]
5. **Three quarters of that exit had no task behind it — fixed in the three
   tasks.** `E3-B06g`, `h` and `j` each carry their own projection's exhaustive
   `Role` match with no wildcard arm; `i` already did. [a `_ =>` arm added to any
   projection: that projection stops erroring and the parent's fixture goes red]
6. **Step 5's landing grep hit `E3-B01g`'s exit line — scoped to `*needs:*`.**
   [unscoping it makes a correct execution report red]
7. **Step 2's landing arithmetic said six — it is four.** The other three lines
   naming `E3-00` wait on `E3-D04`, `E3-D01` and `E3-B01a`. [ticking `E3-00`
   alone and seeing six would now mean an edge was dropped]
8. **A ratio nothing supported — deleted.** `claims/0033` sets
   `cpu_raster_us_x100` at 840 000 against 315 000 and says it is *not a
   multiple*; RFC 0080 says the same, having already corrected *two and a half*
   out of itself. No ratio is quoted now. [re-deriving 840 000 from the top rung
   contradicts both files]
9. **An obligation attributed to RFC 0079 — corrected in `intent.md`.** That RFC
   names a signal; the lint on `E3-B06d` is this decomposition's proposal.
   [`grep -c lint` on that RFC is 0]
10. **A stale working-tree fact — deleted.** `claims/0034` and `claims/0035` have
    `ROUTES` entries; the prescribed `Route::Unbuilt("E3-B01f")` named an id that
    does not exist. [both rows are at `xtask/src/main.rs:14839-14840`]
11. **Step 6's landing check could not fail — replaced** with one that fails on
    either half of the step being done alone.
12. **`E3-P01`'s exit swap added a requirement no subtask produced — narrowed**
    to the assembled rig and one end-to-end capture. [a software timestamp in the
    capture path, or a missing stage, fails it; nothing new is asked]
13. **Bucket B's definition was false of its own members — restated** as *no
    open blocker outside this decomposition*, since five of them name `[x]`
    tasks and all reach `E3-00`.
14. **Bucket A's summary miscounted — restated** item by item: one wire format,
    one vocabulary, two decisions, one timestamping rule, one shopping list.
    `E3-B04b` is the input entry format and is in B.
15. **`claims/0033`'s own 9 ms/8.4 ms gap — not this entry's file**, and it is
    now argued in that file: 9 ms less three unpriced terms, split at 8.4, stated
    as the file's choice.

## Risks and reversal

**The decomposition is wrong about where the work is.** The most likely error,
and it has a shape: `E3-B06e` — incremental constraint layout — is one `L` line
here and is named by section 12 as *the entire engineering risk in this part*.
If that subtask alone turns out to be an epoch, the decomposition has done the
thing it was written to prevent, one level down. The observation is concrete: a
first attempt at `E3-B06e` that does not have a working incremental solve inside
a month means the line is `XL` and needs this treatment again — and if `E3-B08`
exists by then, the tool says so rather than a reviewer.

**The buckets rot the moment a blocker lifts.** Bucket C exists because two
tasks are `[>]`, and both could close this month. That is fine and expected —
what is not fine is the counts in this file being quoted in three months as
though they were still true. They are a photograph taken on 2026-09-14, and the
thing that stays true is the `needs:` graph in the lines themselves, which is why
the lines and not the counts are the deliverable. Finding 9 is what this looks
like when it happens to somebody else's number, and finding 9 happened twice in
two days.

**Seventy-two lines is a document nobody reads.** `TODO.md` is meant to be
read by somebody who wants to pick up work without asking permission, and
quadrupling one epoch's line count works against that. The defence is that
`cargo xtask todo E3` is the interface and the file is the data — but if the
epoch's section becomes unreadable, the answer is a `docs/` page and not a
smaller decomposition, because the coarseness is what this task exists to remove.

**The two proposed decisions get skipped.** `E3-B03a` and `E3-B06a` are each one
subtask in front of ten, and both are the kind of decision that is cheaper to
skip than to make. If either is skipped, the evidence is visible late and
expensively: an imported shaper reached from the permissive tree without a ring,
or a projection with a wildcard arm in it. The first is caught by a lint that
exists; the second is caught by nothing until `E3-B06i`'s exhaustive `match` on
`Role` exists, which is why `E3-B06a` is the one to watch.

**The parent is ticked with its children undone.** Adding `needs:` made the
relation visible and did not make it enforceable. This is the residue of finding
4, it is real, and the only thing standing in front of it today is that a parent
carrying twelve `[ ]` children in its `needs:` looks wrong in `cargo xtask todo
E3`'s waiting list. `E3-B08` is what would make it look wrong to a machine.
