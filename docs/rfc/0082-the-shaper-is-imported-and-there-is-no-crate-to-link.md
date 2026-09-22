# RFC 0082: The shaper is imported, and there is no crate for the permissive tree to link

- Status: accepted
- Date: 2026-09-15
- Affects: `third_party/` — its first entry, owed rather than present;
  `LICENSING.md` rules 1 and 3 and RFC 0003's import inventory, applied to a
  class they did not enumerate; `text/src/lib.rs`, whose module documentation
  names this decision as open, and `text/Cargo.toml`'s dependency comment
  beside it; `LICENSING.md`'s table row for `third_party/<name>/`, which says
  driver source; `docs/manifest.md`'s `image`/`domain` pair;
  `xtask/src/main.rs`'s `lint_licensing`, which carries the strengthening named
  below as of this revision; the root `Cargo.toml`'s `exclude`, which is owed
  `"third_party"`; `TODO.md` E3-B03a and a line in front of E3-B03c that does
  not exist yet

## Decision

The shaper is **imported**. Its source arrives under `third_party/<name>/`
carrying whatever licence it arrives under, it is built into a component image
of its own, and it is reached from the permissive tree over a ring.

The second half of that sentence is the part that has to survive an edit, and
the first draft of this section claimed more for it than was true. **It claimed
that the shaper is reached over a ring and by no other route *because there is
no other route to take*, and that is false.** The claim is corrected here rather
than quietly softened, because it was the whole basis on which the decision was
accepted.

What is true: what the permissive tree *names* is an `image` path in a
`manifest.toml` rather than a crate, and a Rust file cannot `use` an image. What
is not true is that this leaves nothing for a later edit to reach for. A
reviewer built the thing the paragraph called impossible, in this workspace's
shape: a permissive crate takes `f-shape-sys = { path =
"../third_party/shaper" }`, calls `f_shape_sys::shape()`, spells `third_party`
in no Rust file, and builds. Worse, the imported crate **joins the workspace**
while it is at it — cargo adds a path dependency under the workspace root to
`workspace_members` automatically unless the root `Cargo.toml`'s `exclude` names
it, and this workspace's `exclude` is `["kernel/proofs", "ring/proofs",
"abi/proofs"]`. So the import would be compiled under the permissive tree's lint
table and its licence field, by a row that fits on one line, with `cargo xtask
lint` green throughout.

So the narrowed claim, which is what this RFC now asserts: **the shaper is
reached over a ring, and every other route is closed by a check rather than by
impossibility.** The check is `lint_licensing`'s dependency-graph half, which
exists as of this revision — it reads every permissive `Cargo.toml`, including
the root's `[workspace.dependencies]` and its `members` list, and refuses any
`path` that resolves under `third_party/` whatever the dependency is called. It
has fixtures that drive each spelling of that row to red; `no_route_into_the_
import` in `xtask/src/main.rs` is the module. The textual check stays, because a
`use third_party` and a path row are different mistakes.

What is *not* claimed any more: that the absence of a dependency row is a
structural property of the workspace. It is a property of `third_party/` being
empty, which stops being true on the day the import lands, and that is exactly
the day it was supposed to hold. One edit would make it structural again and it
is owed to whoever owns the root manifest: add `"third_party"` to `exclude`, so
that cargo itself refuses the auto-membership and the lint is left guarding only
the link. Until that lands, the guard is the lint, and a lint is a thing a
reviewer reads rather than a thing a compiler enforces.

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

1. **It is a pure function of `(text, face, size, features)` as far as its
   handles go.** Its manifest routes it no `irq`, no device `[[capability]]`,
   and no `powerbox` ask, so it holds no *handle* through which a clock, a
   locale database or an environment could reach it. That is a narrower
   statement than the first draft's "RFC 0004's rule is satisfied structurally
   at this boundary", and the narrowing is owed to the syscall door: `PROGRESS`
   (`abi/src/door.rs`) needs no handle, is available to every component, and is
   answered out of a per-core tick count — so a component holding nothing at all
   can poll it and observe a signal that varies with how long the frame let it
   run. What property 3 below actually removes is the ability for that signal to
   change the *output*: whatever the import observes, what crosses the ring is
   fixed-point integers, and `E3-B03i`'s corpus is what would catch an import
   whose positions moved with anything but its inputs. Structurally denied a
   clock handle, checked rather than denied for the door.
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
over a ring **and by no other route**. That is three separate observations. This
tree now makes two of them and cannot make the third, and which is which matters:
the two it makes are the *no other route* half, and the one it cannot make is
*the route exists*, because the route's two files are the entry that has not
landed.

1. **The route exists and it is a ring.** `user/shaper/manifest.toml` in the
   permissive tree — never under `third_party/`, which `docs/manifest.md` already
   refuses, because a manifest is policy and a re-import must not be able to
   change it — with `image = "third_party/<name>"`, a `domain` that is not
   `shared`, and a `[[ring]]` declaring the `shape` protocol with `role =
   "server"`. The protocol it names is the second file: **`abi/src/shape.rs`**,
   beside `abi/src/objects.rs`, `abi/src/store.rs` and `abi/src/input.rs`, which
   is where every other protocol in this tree is declared and what makes the
   name `shape` resolve to something rather than to a directory. Those two are
   the files the clause means, and a reader discharges it with one `test -f`
   each.

   The consuming component declares the matching `role = "client"` ring whose
   `to` names an `endpoint` capability carrying `write`. That is a **row added
   to a manifest that already exists**, not a third file, and *which* component
   adds it is the import task's to choose: `user/` holds nine of them and the
   first caller of a shaper is not decided here. Naming a consumer now would be
   naming it in the document that cannot check it — and the reason the clause
   says *two files* rather than three is exactly that.

   `lint-manifests` checks every one of those fields today against a schema it
   already has. **This observation needs no new code; it needs the two files.**
2. **No other route in source.** `lint_licensing` reads every Rust file in the
   permissive tree and refuses `use third_party` and `third_party::`. This exists
   and runs on every `cargo xtask lint`.
3. **No other route in the dependency graph.** *This now exists, and it exists
   because the first draft of this RFC said it did not need to.* Check 2 matches
   two string literals. A permissive crate that took an imported crate as a path
   dependency under any name of its own — `f-shape-sys`, say — spells
   `third_party` nowhere in its Rust source and passed. `manifests()`, the
   walker that collects every crate manifest, excludes `third_party/` from its
   walk, and nothing read a *permissive* manifest looking for a path row
   pointing into it. So the licensing lint checked a spelling and not a graph.

   It reads both now. `licensing_graph_findings` in `xtask/src/main.rs` walks
   every permissive `Cargo.toml` — the set `manifests()` already returns, which
   includes the workspace root and so its `[workspace.dependencies]` — and
   refuses any `path` value that resolves under `third_party/` by any spelling:
   the inline table, the dotted key, the `[dependencies.x]` section, and a
   `members` entry naming a directory under it. Comments are stripped first, and
   `exclude = ["third_party"]` is deliberately not a finding, because that row is
   the repair rather than the defect. Every one of those spellings has a fixture
   that drives it red in `no_route_into_the_import`, and one more fixture runs
   the lint against this workspace's own manifests, so a walker that stopped
   returning anything fails as loudly as a row that appeared. Check 2 stays: a
   `use third_party` and a path row are different mistakes.

   Two corrections to this paragraph, both found by re-reading it against the
   code rather than against itself. *By any spelling* was false by one
   character: the scan read TOML's basic strings and not its literal ones, so
   `path = '../third_party/shaper'` produced nothing, in the path check and in
   the `members` walk both. It reads both kinds now and has a fixture for each
   half. And *one more fixture runs the lint against this workspace's own
   manifests* was true and pinned nothing: it asserted that the walk returned
   more than one manifest, so adding one directory name to the walker's skip
   list — `"user"`, which would stop seven crates being read — left it green.
   It now reads the root manifest's own `members` list and requires a manifest
   and a source from every crate in it, which is red against exactly that edit.

4. **No other route through a `#[path]` attribute.** *This is the third route,
   it was missed by both reviews of check 3, and it is the worst of them.*
   `#[path = "../../third_party/shaper/src/lib.rs"] mod shaper;` in a permissive
   crate writes no manifest row, so check 3 reads nothing, and no `use`, so
   check 2 matches nothing — `grep -c` for both of check 2's literals over the
   consuming file returns 0, and the crate's `Cargo.toml` contains no occurrence
   of `third_party` at all. What it links is not a crate but a *file*, compiled
   into the permissive crate under that crate's `unsafe_code = "forbid"` and its
   `license = "Apache-2.0 OR MIT"` field, with no boundary left to inspect.
   LICENSING.md rule 1 forbids `use`ing a file under `third_party/` from the
   permissive tree; this is that, without the `use`. It is also the local idiom
   rather than an exotic one — `kernel/proofs` and `ring/proofs` both compile a
   shipped file through `#[path]` — which is what makes it the likeliest of the
   three to be written by somebody who is not trying to evade anything.
   `path_attr_findings` refuses it as of this revision, sharing check 3's parser
   because it is the same grammar at a different layer, and the fixture that
   drives it red sits beside the others.

**And the removal would have mattered more than the check, which is why the
first draft leaned on it and why the lean was wrong.** The paragraph that stood
here said the property is held by there being nothing to write — no workspace
membership, no `[workspace.dependencies]` row, and an image built from source
that is not a library this workspace resolves. The first two clauses are false:
cargo adds a path dependency under the workspace root to `workspace_members`
automatically, so writing the row *creates* both the membership and the
resolvable library, and no step in between is anybody's decision. What the
absence really rests on is `third_party/` being empty, which is a property that
expires on the day this decision is carried out. So the honest ordering is the
opposite of the one written first: **the lint is the guard, and the structural
removal is what is owed** — `"third_party"` in the root `Cargo.toml`'s
`exclude`, which is an edit to a file this document's author may not touch and
which is therefore named here with an owner rather than assumed.

### What cannot be observed on the day this is accepted, and what closes it

Plainly, because the alternative is a clause everybody assumes somebody checked:
`third_party/` is empty — `find third_party -type f` returns `README.md` and
nothing else — `user/shaper/manifest.toml` does not exist, and
`abi/src/shape.rs` does not exist. Those are the two files, by path, because a
clause that names a directory and a protocol is a clause a reader cannot run a
command against; the first draft of this section named the second one as
"`abi/`'s `shape` protocol" and a fourth review round was right that it
identified nothing. Observation 3 is written now and observations 1 and 2
are not, because 1 needs the two files and 2 has nothing to read.

**So the second clause of `E3-B03a`'s exit is not met, and this RFC's own first
sentence is what makes it live.** The exit reads *if imported, `third_party/`
carries the entry and `cargo xtask lint` shows the permissive tree reaching it
over a ring and by no other route*; this RFC answers *imported*, and the
conditional then asks for an entry that is not there. A conditional clause the
document itself fires is not discharged by the document noting that it cannot be
observed. **`E3-B03a` does not close on this artifact.** What it can close on is
its first clause — an RFC, accepted, naming which of the two options and what
would reverse it — and the *by no other route* half of the second, which is now
a check with fixtures rather than an assertion. The entry is the part that is
missing, and the task below is the one that lands it.

**RFC 0088 is where that was settled, and it went the way this section implies.**
Two refusals later the conclusion was that the exit was malformed rather than
the work skipped: an `S`-sized *decide* task whose exit demanded an
implementation was two tasks' exits joined by *if*, and the second task was not
in the decomposition. So `E3-B03a`'s exit line in
`intent/0012-the-interface/spec.md` was narrowed by RFC 0088 to the decision and
the checks, with the entry, the ring and `"third_party"` in the root `exclude`
moved to the task written out below. That is the route RFC 0084 opened for a
narrowed exit, and the narrowing is in the spec with the number beside it rather
than only here — which is the whole of what RFC 0084 asked for.

A note on the registry row, because the honest version of it is not the one a
reviewer reported. `docs/rfc/README.md`'s row for this document was refused for
still asserting the retracted sentence while the body retracted it; the
observation behind that was `git diff --stat docs/rfc/README.md` being empty
across the repair commit, which was true. The row was in fact rewritten one
commit later, and reading it now shows it quoting *"and by no other route because
there is no other route to take"* as the claim that **was** retracted, saying a
reviewer built what it called impossible, and saying in as many words that the
exit is not closed and which clause. Body and index agree. What the row does not
yet carry is route 4 above — it names the dependency graph and not the `#[path]`
attribute — which is a row that is incomplete rather than false, and is owed to
whoever owns that file, as no agent working in this tree may edit it and no check
compares a registry row to the RFC it indexes.

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
date; `cargo xtask lint` is green with the entry present — observation 3 is
implemented and has been shown to go red against a crate that takes a path
dependency into `third_party/` under an unrelated name, so what that task adds
is the subject rather than the check; and one run of text through the `shape`
protocol produces fixed-point advances on both architectures.

Two smaller obligations, named so they are not inferred from an absence.
`"third_party"` belongs in the root `Cargo.toml`'s `exclude`, for the reason
*Decision* gives. And two comments are now stale: `text/src/lib.rs` and
`text/Cargo.toml` both describe the shaper question as open — "if the answer is
*imported*", "may land behind the licence boundary" — and this document closes
it. `LICENSING.md`'s table row is the third: it defines `third_party/<name>/` as
"Imported driver source and its shim", and what will sit there is a shaper. None
of the three is a file this document's author owns.

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
