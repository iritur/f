# RFC 0082: The shaper is imported, and there is no crate for the permissive tree to link

- Status: accepted
- Date: 2026-09-15
- Affects: `third_party/` — its first entry, owed rather than present;
  `LICENSING.md` rules 1 and 3 and RFC 0003's import inventory, applied to a
  class they did not enumerate; `text/src/lib.rs`, whose module documentation
  names this decision as open; `docs/manifest.md`'s `image`/`domain` pair;
  `xtask/src/main.rs`'s `lint_licensing`, which needs one strengthening named
  below; `TODO.md` E3-B03a and a line in front of E3-B03c that does not exist
  yet

## Decision

The shaper is **imported**. Its source arrives under `third_party/<name>/`
carrying whatever licence it arrives under, it is built into a component image
of its own, and it is reached from the permissive tree over a ring.

The second half of that sentence is the part written to survive an edit. It is
reached over a ring **and by no other route, because there is no other route to
take**: nothing under `third_party/` is a workspace member, nothing there has a
row in `[workspace.dependencies]`, and what the permissive tree names is an
`image` path in a `manifest.toml` rather than a crate. A Rust file cannot `use`
an image. There is consequently no dependency row for anybody to write, no
crate name for anybody to spell, and nothing for a later edit to reach for in a
hurry — which is a different and much stronger state of affairs than a lint that
notices afterwards. The lint is still owed, and *why* it is still owed is in
*Consequences*; it is a second net rather than the guard.

What is imported is narrow and it is named here rather than left to the import:
**the interpretation of a font's own tables for one run of text** — glyph
selection and positioning, which is `GSUB` and `GPOS`, for one script in one
direction in one face at one size under one feature set. Everything around that
stays in the permissive tree. The boundary is drawn by a rule rather than by
whatever turns out to be convenient on the day:

> What is imported is specified by somebody else's file format and measured by
> no claim in this tree. What is written is specified by a document this tree
> can run a conformance corpus against, or sits on a path a claim rests on.

| the work | side | why that side |
| --- | --- | --- |
| glyphs and positions for one run | imported | somebody else's tables, and getting it wrong is visible as wrong text and invisible to every claim |
| bidirectional reordering, UAX #9 | written, `text/` | `E3-B03e`; a specification with a conformance corpus this tree runs |
| line breaking and cluster boundaries, UAX #14 and #29 | written, `text/` | `E3-B03f`; the same, and breaks fall on the incremental-layout path |
| the shaping cache, keyed by string, face, size and features | written, `text/` | `E3-B03c`; the determinism property is the cache's, and a cache behind the boundary is a cache this tree cannot test |
| metrics, accumulation and the rounding rule | written, `text/` | `E3-B03d` and RFC 0004; fixed point with the scale in the name |
| the coverage atlas, GPU path rendering, and the rule that chooses | written | `E3-B03g` and `E3-B03h`; section 09's latency chain |

Three properties the import must satisfy. They are the import task's acceptance
conditions and not this document's preferences, and an upstream that cannot
meet them is not importable here whatever else is true of it:

1. **It is a pure function of `(text, face, size, features)`.** Its manifest
   routes it no `irq`, no device `[[capability]]`, and no `powerbox` ask, so it
   holds no handle through which a clock, a locale database or an environment
   could reach it. RFC 0004's rule is satisfied structurally at this boundary
   rather than by reading somebody else's source for `time()`.
2. **It brings its own allocator, inside its own address space, sized in its own
   manifest.** A `[[capability]]` of type `untyped` with a `bytes` count, and
   admission refuses a component that asks for more than there is (RFC 0007).
3. **Its output crosses the ring as fixed-point integers with the scale in the
   name.** Whatever it does internally stops at the shim. No `f32` and no `f64`
   appears in the `shape` protocol's types, which live in `abi/` like every
   other protocol's and are owed to the import task.

What this RFC deliberately does **not** decide is *which* upstream. E3-B03a asks
where the shaper comes from, and the side of the boundary is the answer to that
question; the name of the project is provenance, with its own evidence —
licence, maintenance, whether it builds freestanding, and property 3 above —
recorded in `PROVENANCE.md` where `LICENSING.md` rule 3 already requires it. A
decision that named an upstream today would be naming it from memory rather than
from a build, which is the failure mode of every dependency choice made in a
design document.

And the cost of this decision, stated in it rather than underneath it: **on the
day this RFC is accepted, the second clause of `E3-B03a`'s exit cannot be
observed.** `third_party/` is empty. There is no manifest, no image, and no
dependency graph for a lint to walk. That clause is an obligation with an owner,
named in *Consequences*, and it is not satisfied by this document.

## Context

`third_party/` has been empty since M0 and this is the first thing proposed to
go in it. RFC 0003 settled that imported source exists at all and gave the rule
that decides each case — *import what is not being researched; write what is* —
and then enumerated: graphics and wireless imported; storage, network, audio,
accelerators and the input path written, because those are the code paths the
claims live on. Text is in neither list. So this RFC does not reverse RFC 0003;
it applies RFC 0003's rule to a class that RFC 0003 did not enumerate, and the
reason it needs an RFC at all is that it is the first crossing, that `text/` has
been written with the question open in its module documentation, and that ten
subtasks stand behind it.

The facts that bear on it were true before anybody argued: there is no allocator
above the frame; `unsafe` is forbidden above the frame; and every shaper worth
importing is a large body of C or C++ written against neither constraint.

### The last time this tree asked this question it answered the other way

RFC 0081 needed a font for the frame's console and decided to **type one** — five
pixels by seven, ninety-five glyphs, plainer than any candidate and chosen over
all of them. The reasoning there is worth restating because it looks like a
precedent against this decision and is not: `LICENSING.md` allows the permissive
tree and `third_party/`, with no third category for permissively-licensed
imported *data*, so a font could go in neither place — not in `kernel/src/`
without putting somebody else's licence in the permissive tree, and not in
`third_party/` and be reached, **because the frame cannot reach `third_party/`
at all**.

That last clause is the whole difference. RFC 0081's consumer was the frame, for
which the ring-only coupling is not a constraint but an impossibility. This
RFC's consumer is a component above the frame, for which a ring is the ordinary
way to reach anything. Same boundary, opposite answer, and what moved is which
side of the frame the consumer sits on. RFC 0081 says this itself in its own
reversal section — *the day this console moves into a component, that component
may carry any font whose licence allows it, and the licence boundary does not
move an inch to allow it.*

### The case for writing it in the permissive tree, which is better than it looks

It deserves a real hearing, because four of its arguments are correct.

**Determinism.** An imported shaper is a black box to RFC 0004's lint, which
reads Rust source and never walks into `third_party/`. A written shaper would be
integers the whole way down and its cross-architecture behaviour would be a
property of construction rather than the result of a test. Since text metrics
are, as `intent/0012`'s decomposition says, *the single most natural place in
this system to reach for a float*, importing a body of code that certainly
contains them is importing the exact hazard the epoch was warned about.

**No crossing in the frame path.** A ring round trip on every shaping call is a
cost a written shaper simply does not have.

**No licence question at all.** RFC 0002 makes this a research vehicle whose
results are meant to be liftable by anyone, which is why the permissive tree is
permissive. Anything behind the boundary is not liftable on those terms.

**And there is a permissively licensed pure-Rust shaper in the world**, a port
of the standard one. Taken as an ordinary dependency it would sit in the
permissive tree with a licence `deny.toml`'s allow-list already accepts, and
there would be nothing to isolate.

### Why it loses, and the argument is not the one about size

The obvious refutation is scale — that a shaper is Arabic and the Indic scripts
and Khmer and Myanmar, that per-script complexity is the entire content of the
problem, and that something which shapes Latin correctly is a demonstration and
not a shaper. That is true and it is not the argument, because a research
project is allowed to spend years on a thing if the thing is what it is
researching. The reason it is not: shaping is not what this project claims.
`docs/design/`'s claims rest on the latency chain, the delta ring, incremental
layout and the caches. Not one of them rests on `GSUB`. RFC 0003's rule was
written for exactly this and answers it in one line.

**The argument that decides it is the heap, and it is a fact about this tree
rather than a judgement.** A shaper needs dynamic memory; that is not a defect of
any particular implementation, it is what buffer-of-glyphs-becomes-a-different-
buffer-of-glyphs means. In the permissive tree an image that wants a heap needs
`#[global_allocator]`, that is an `unsafe impl GlobalAlloc`, and RFC 0001 forbids
`unsafe` above the frame. This is not hypothetical and not new: `user/objects`
records the debt in its own module documentation and is not a component because
of it, and RFC 0066 gives the same reason for `f-assembler` landing as a library
with its caller injected rather than as a component. Both are waiting on one
heap in `ring/` that does not exist. **A shaper written in the permissive tree
today would join that queue, behind an allocator nobody has written, and would
be the third entry on a list of things blocked by it.** An imported shaper does
not join the queue: it carries its own allocator inside its own address space,
in the one part of this repository where the `unsafe` that an allocator is may
be written.

That is the whole shape of the decision. The isolation the shaper needs for
technical reasons — its own address space, its own heap, its own `unsafe` — is
the isolation the licence needs. They are the same isolation, which is
`LICENSING.md`'s thesis arriving one layer up from where it was written.

The pure-Rust candidate does not escape this. It is a port, so it allocates the
way the thing it is a port of allocates, and it is a port of code that uses
floating point for variable-font instancing. Both of those are assertions about
a crate this RFC has not built, and they are written here as **the thing the
import task must check first**, not as findings. If they are wrong — if a
pure-Rust shaper can be shown to be `no_std`, allocator-free and float-free —
then this decision is wrong, and that is the reversal condition this document
would most like to lose to. It is first in the last section for that reason.

### Why the cache is on this side of the ring, which is what makes the import affordable

`E3-B03c` puts the shaping cache in the permissive tree keyed by string, face,
size and features, and that placement was decided for a determinism reason — a
cache keyed through a `HashMap` has per-process iteration order, so hit and miss
counts differ between two runs of one seed and the only testable property of the
cache is gone. The placement has a second consequence this RFC depends on and
`E3-B03c` did not have to argue: **the cache is in front of the boundary, so a
scene whose text did not change crosses it zero times.** The crossing cost lands
on cold text and on nothing else, which is the same shape as `E3-B03g`'s exit
for the atlas.

Had the cache gone behind the ring — which is where it would have gone if the
shaper were imported as a whole subsystem instead of as one run's worth of work
— every shaping call would be a crossing and the steady-state cost of the
boundary would be proportional to the text on the screen rather than to the text
that changed. That is the version of this decision that fails, and the boundary
drawn in *Decision* is drawn where it is to avoid it.

### Why the shaper is the right place for this boundary to be tested first

RFC 0003 planned for the boundary's first real load to be a graphics driver, and
named the honest risk: graphics drivers assume kernel context, hosting them out
of frame may not work, and the fallback is an in-frame exception with its cost
measured. A shaper is a much better first subject and the reasons are all
structural. It holds no device and takes no interrupt, so nothing about it
depends on a driver model. It is a pure function, so restarting it loses nothing
but a cache entry the permissive side still holds. It is off the frame path in
steady state, per the section above. And it is the only one of the two whose
failure mode is *text renders wrong* rather than *the machine has no display*.

If the licence boundary is going to turn out to be in the wrong place, this is
the cheapest possible way to find out.

## Consequences

**What it makes easy.** `text/` keeps its zero dependencies, which its own module
documentation already promised against this decision going the other way. Ten
subtasks — `E3-B03b` through `E3-B03k` — are unblocked and every one of them
stays in the permissive tree, so the epoch's text work is permissively licensed
work with one imported component underneath it rather than a permissive shell
around an imported core. The shaper is a component like any other, so it carries
a `[restart]` policy and a shaper that faults takes text down and not the
compositor: RFC 0008's *restart is a new spawn and not a resurrection*, and what
survives is the endpoint the client already holds.

**What it makes true that was not.** This is the first Rust in this repository
that may contain `unsafe` without being in the frame, and the only thing standing
between that and *`unsafe` anywhere* is a directory name — `rust_sources()` skips
`third_party/`, so `lint-unsafe` never reads it. That is intended and it is RFC
0003's design, where imported code sits outside the trusted computing base and
the unsafe-code metric survives because it measures `abi/`, `ring/` and
`kernel/` and nothing else. It is written down here because the first time a
policy's exemption is used is the moment to say out loud that it is an
exemption. What keeps it honest is not the lint: it is that an
image under `third_party/` may not declare `domain = "shared"`, which
`lint-manifests` refuses today under RFC 0005 rule 4, so the code that is outside
the metric is also outside the speculation boundary.

### What `cargo xtask lint` has to show, and what it shows today

`E3-B03a`'s exit asks that the lint show the permissive tree reaching the import
over a ring **and by no other route**. That is three separate observations and
this tree makes one and a half of them.

1. **The route exists and it is a ring.** `user/shaper/manifest.toml` in the
   permissive tree — never under `third_party/`, which `docs/manifest.md` already
   refuses, because a manifest is policy and a re-import must not be able to
   change it — with `image = "third_party/<name>"`, a `domain` that is not
   `shared`, and a `[[ring]]` declaring the `shape` protocol with `role =
   "server"`. The consuming component declares the matching `role = "client"`
   ring whose `to` names an `endpoint` capability carrying `write`.
   `lint-manifests` checks every one of those fields today against a schema it
   already has. **This observation needs no new code; it needs the two files.**
2. **No other route in source.** `lint_licensing` reads every Rust file in the
   permissive tree and refuses `use third_party` and `third_party::`. This exists
   and runs on every `cargo xtask lint`.
3. **No other route in the dependency graph.** *This does not exist, and the
   gap is specific enough to be worth writing out.* Check 2 matches two string
   literals. A permissive crate that took an imported crate as a path dependency
   under any name of its own — `f-shape-sys`, say — would spell `third_party`
   nowhere in its Rust source and would pass. `manifests()`, the walker that
   collects every crate manifest, excludes `third_party/` from its walk, and
   nothing reads a *permissive* manifest looking for a path row pointing into it.
   So today's licensing lint checks a spelling and not a graph.

The strengthening is small and its shape is fixed here so that whoever writes it
does not have to re-derive it: read every permissive `Cargo.toml` — the set
`manifests()` already returns — plus the workspace root's
`[workspace.dependencies]`, and refuse any dependency whose `path` resolves under
`third_party/`, whatever the dependency is called. Keep check 2; a textual net
and a structural one catch different mistakes and neither is redundant.

**And the removal matters more than the check.** The reason to write the
strengthening is not that anybody is expected to write the row; it is that
*nobody wrote the row* is not a property a reader can verify, and the check makes
it one. The property itself is held by there being nothing to write: no
workspace membership, no `[workspace.dependencies]` row, and an image built by
its own step from source that is not a library this workspace resolves. A guard
a later edit can walk past is not a guard, and the guard here is the absence of
the thing rather than the lint about it.

### What cannot be observed on the day this is accepted, and what closes it

Plainly, because the alternative is a clause everybody assumes somebody checked:
`third_party/` is empty, `user/shaper/manifest.toml` does not exist,
`abi/`'s `shape` protocol does not exist, and observation 3 above is unwritten.
**The second clause of `E3-B03a`'s exit is therefore unobservable today, and this
RFC does not claim otherwise.**

What closes it is one task, and the decomposition in `intent/0012-the-interface`
does not contain it — the eleven `E3-B03` subtasks assume a shaper and none of
them imports one. So a line is owed to whoever owns `TODO.md`; agents working in
this tree may not edit it, which is the same position RFC 0080 was in about
`E3-B02`'s exit, and the same answer: the line is written out here so the
obligation has an owner and a shape rather than being implied by an absence.
It sits between `E3-B03b` and `E3-B03c`, because the cache is the first subtask
that needs something to have been shaped.

Its exit is three things and all three are observations: `third_party/<name>/`
carries `LICENSE` and `PROVENANCE.md` with an upstream URL, a commit hash and a
date; `cargo xtask lint` is green with observation 3 implemented, having been
shown to go red against a fixture crate that takes a path dependency into
`third_party/` under an unrelated name; and one run of text through the `shape`
protocol produces fixed-point advances on both architectures.

### What this does not solve, and it will be met by `E3-B03e`

The written half of the text path needs Unicode character property tables — bidi
classes for UAX #9, break properties for UAX #14 and #29. That is *imported data
under a permissive licence*, which is precisely the category RFC 0081 found
`LICENSING.md` does not have. Putting the tables in `text/` puts somebody else's
licence in the permissive tree; putting them in `third_party/` makes them
unreachable except over a ring, which for a table consulted per character is
absurd. This RFC does not close that, and it is not the same question — a shaper
is *code that needs isolation anyway*, and a property table is data that needs
none. `E3-B03e` is where it lands, and the options are generating the tables into
the permissive tree from a build step, which is what RFC 0081 did by hand for a
font, or `LICENSING.md` gaining a third category, which is an RFC of its own.

**What it makes hard.** A cold cache is a ring round trip inside a frame that is
already being paced against scanout, and nothing in this decision measures that.
Cross-architecture reproducibility of glyph positions becomes a test to run
rather than a property of construction. A re-import is a review rather than a
version bump. And the number of crossings per frame is a thing somebody has to
count, which is a claim this RFC may not register and names as owed to the same
import task.

**What it forecloses.** A permissively licensed shaper arriving later as an
incremental addition — not because it would be forbidden, but because the ten
subtasks behind this one will have been written against a ring, and moving the
shaper into the permissive tree afterwards is a rewrite of the seam rather than a
substitution behind it. The last section says what would justify that rewrite.

It also forecloses the in-frame fallback RFC 0003 keeps for graphics, and this
asymmetry is worth stating because somebody will reach for it. RFC 0003 says that
if an imported driver cannot be hosted out of frame the fallback is a documented
in-frame exception with its cost measured. **That fallback is not available for a
shaper.** The frame cannot reach `third_party/` at all — RFC 0081 established
that for a font and the same wall stands here — so an imported shaper that cannot
live in a component has nowhere to fall back to. Its fallback is *written*, which
is this RFC reversed.

## What would reverse this

**A pure-Rust shaper that is `no_std`, allocator-free and float-free.** This is
the condition this RFC expects least and would most like to meet, and it is an
afternoon's work to test rather than a judgement: take the candidate, build it
for `aarch64-unknown-none` with no `alloc`, and grep it for `f32` and `f64` with
the same predicate `lint-determinism` uses. If it builds and the grep is empty,
the heap argument — which is the argument that decides this whole document —
evaporates, `deny.toml`'s allow-list already accepts its licence, and the shaper
should be a dependency of `text/` with no boundary anywhere near it. If it builds
only with `alloc`, the condition is not met and is *deferred to the day `ring/`'s
heap exists*, at which point it should be asked again rather than treated as
settled. Whoever asks it should record the answer here, because a condition
tested once and not written down is a condition that gets tested every year.

**Positions that differ between the two architectures, and a shim that cannot
make them stop.** Shape `E3-B03i`'s corpus on x86-64 and on AArch64 and compare
the fixed-point output byte for byte. A difference is what floating point inside
the import looks like from outside it, and property 3 of the decision is the
thing that failed. Note what this refutes and what it does not: **one upstream
diverging refutes that upstream**, and the answer is a different import or a shim
that rounds on the imported side of the ring before anything crosses. It reverses
*this decision* only if the divergence survives a second upstream, at which point
the claim is that shaping cannot be imported deterministically at all and the
written shaper becomes the cheaper of two expensive options.

**The cache stops absorbing the crossing.** On a scene whose text did not change,
count the operations submitted on the `shape` ring in a frame. If that count is
not zero, the cache is not in front of the boundary in the way *Context* claims,
and the crossing cost is proportional to text on screen rather than to text that
changed. First find out whether the cache can be fixed; a non-zero count is far
more likely to be `E3-B03c`'s defect than this RFC's. What reverses this decision
is the count staying non-zero after the cache is correct — the boundary is then
on the frame path in steady state, and the trade this document made was made
against a cost that turned out to be recurring.

**The scope of the import creeps.** The rule in *Decision* draws a line, and the
observation that the line has failed is a second thing being imported *because it
was next to the first* — property tables, a segmentation implementation, a
rasteriser — rather than because the rule selected it. Each such import is
individually defensible and the sequence is how a boundary becomes a subsystem.
The reversal is not necessarily to write a shaper; it is that the rule needs
restating or this RFC needs superseding, and either way the third import is the
moment to notice rather than the tenth.

**And the boundary itself fails, which is RFC 0003's condition arriving here.**
RFC 0003 reverses on the shim being unable to host imported source out of frame
at acceptable cost. For a shaper that means the component cannot be started, or
cannot be restarted, or its `untyped` demand is refused by admission on every
machine this project has. RFC 0003's fallback does not apply, per *Consequences*.
The shaper's fallback is the permissive tree, behind `ring/`'s heap, and the year
that costs is the price of this decision being wrong — which is the honest way to
hold a decision whose cheap outcome is the one being bet on.
