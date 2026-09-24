# RFC 0114: A property table is generated into the permissive tree, and the corpus it is tested against is not

- Filed under a shortened slug on 2026-09-24. The file was `0114-a-property-table-is-generated-into-the-permissive-tree-and-the-corpus-it-is-tested-against-is-not.md`, which is over ustar's 100-byte name field, so `cargo xtask release` could not pack the tree and two CI jobs refused it. The title is unchanged; the slug is not a summary and the number is what anything cites
- Status: accepted
- Date: 2026-09-24
- Affects: `LICENSING.md` — its table, which has two rows and needs three, and its
  rule 1, which names one permitted coupling and needs a second for data;
  `third_party/README.md`, which says *imported source* and will hold data;
  `xtask/src/main.rs`'s `lint_licensing`, whose SPDX check reads a prefix where
  this decision needs a value, and whose `TOOLING` row exempts the generator for
  a reason that is not this one; `deny.toml`, which already allows `Unicode-3.0`
  for a crate and cannot see a file; RFC 0082's *What this does not solve*, which
  named this as `E3-B03e`'s and named the two options; `TODO.md` E3-B03e and
  E3-B03f, neither of which can start without this

## Decision

Unicode's data files are **imported as data, under `third_party/unicode/`, and
they are reached by two routes and no third**:

1. **A property table consulted per character is generated into the permissive
   tree.** `Bidi_Class` for UAX #9, the bracket pairs N0 needs, and later the
   break properties of UAX #14 and #29 are committed as Rust tables under
   `text/src/`, each generated from a named upstream file by a command somebody
   runs deliberately. The generated file carries `// SPDX-License-Identifier:
   (Apache-2.0 OR MIT) AND Unicode-3.0` and a header naming the upstream file,
   the Unicode version, the SHA-256 of the input it was generated from, and the
   command that regenerates it.
2. **A conformance corpus is read at run time by a named test and never enters an
   artefact.** `BidiTest.txt` and `BidiCharacterTest.txt` stay under
   `third_party/unicode/` and are opened by path from `text/tests/`, on the host,
   by a harness that is named in a table in `xtask` the way the one clock reading
   is named. They are not generated, not `include_str!`-ed, not compiled and not
   crossed by a ring, because none of those is what a file read by a test needs.

So `LICENSING.md` gains a third category, which is the second of the two options
RFC 0082 left open, and it is deliberately narrow: **the category holds files no
compiler in this workspace reads.** No `.rs`, no `.c`, no `.h`, no build script,
nothing an interpreter runs. A directory under `third_party/` that holds code is
governed by rule 1 and reached over a ring exactly as before; one that holds data
is governed by this, and an import that is both is two imports.

**What is not decided here is which Unicode version.** That is provenance, it
belongs in `PROVENANCE.md` with a hash beside it, and a decision naming a version
today would be naming it from memory rather than from a file — RFC 0082's answer
about which upstream shaper, applied to a data file for the same reason.

## Context

RFC 0082 named this and did not decide it, in as many words: the written half of
the text path needs Unicode character property tables, *that is imported data
under a permissive licence, which is precisely the category `LICENSING.md` does
not have*, and the options are generating the tables into the permissive tree
from a build step or `LICENSING.md` gaining a third category, *which is an RFC of
its own*. This is that RFC. It exists because a citation is cheaper to write than
a decision — RFC 0084's own hazard — and `E3-B03e` cannot start on a citation.

### The rule that already sorts this, and the collision that dissolves

The objection to be answered first is that this contradicts RFC 0082: the shaper
is imported and reached over a ring, so the boundary for one half of `text/` is a
privilege boundary and for the other half is a table in a source file, which
reads like the boundary moving to wherever it is convenient.

It is not, and RFC 0082's own rule is what says so:

> What is imported is specified by somebody else's file format and measured by no
> claim in this tree. What is written is specified by a document this tree can
> run a conformance corpus against, or sits on a path a claim rests on.

That rule sorts **work**, not files. Bidirectional reordering is on the *written*
side of it — that is the row RFC 0082's own table gives `E3-B03e` — and a table
of character properties is not work at all. It is the specification's own data,
consulted by written code, and nothing about it needs isolating: there is no
ambient authority to remove, no `unsafe` to contain, no allocator to confine and
no restart to survive. The isolation `LICENSING.md` draws its boundary at is
isolation of *behaviour*, and a table has none.

So there is no collision between the two decisions. What there is, is a gap in
the file that sorts by directory rather than by kind, and this closes it.

### The four routes, and three of them are refused by checks this tree already has

The decision above is less a preference than the one route left standing.

**Generating the table at build time is refused by `lint-boundary`.** A build
script is prohibited in the permissive tree — `BUILD_SCRIPT_ALLOW` is empty and
its comment says the emptiness is the check, and cargo's own resolved view is
what reports one, so the prohibition is on the mechanism and not on a spelling
(RFC 0092). *Generating the tables into the permissive tree from a build step* is
the first option RFC 0082 offered and it is unavailable; what survives of it is
generation by a command, with the output committed, which is a different thing and
is what this decision takes.

**Embedding the data file is refused by `lint-boundary`'s second net.**
`include_str!` and `include_bytes!` of a path under `third_party/` are caught by
the net that reads what the compiler says it opened — `an_include_str_is_caught_
by_the_build` is the fixture, and its own comment says why it is there: *not
compiled but embedded, which is the licence half of the rule rather than the
linkage half: the imported bytes end up in the artefact.* That is exactly this
case, already argued, already red.

**Reaching the table over a ring is refused by arithmetic, and the arithmetic has
to be stated rather than asserted.** RFC 0082 called it *absurd for a table
consulted per character*, and the word absurd is doing work a number should do.
The honest version: the cheapest form of this is not per character but per
paragraph — a run's classes fetched in one crossing, cached in front of the
boundary the way RFC 0082 puts the shaping cache in front of it — and in that
form it is not absurd at all. What refuses it is what it buys. It adds a
component, a manifest, a protocol in `abi/`, an admission demand and a restart
policy, all to answer a lookup that cannot fail, has no state, holds no device
and observes nothing. Two hundred lines of trusted path for a table. And the same
argument would move every constant in this tree behind a ring, which is the
reductio that matters: if data needs isolation then `interface/src/token.rs`'s
palettes do, and nobody believes that.

**Typing it by hand is refused by what it would mean.** RFC 0081 needed a font
for the frame's console and *typed one* — five by seven, ninety-five glyphs — and
that precedent is the one somebody will reach for here. It does not extend, and
the reason is not effort. A font is a shape anybody may draw; `Bidi_Class` is an
assignment somebody else made for every scalar in Unicode, and hand-typing it
would be **inventing the specification that the conformance corpus exists to test
against**. A tree that did that would run `BidiTest.txt` against its own opinion
of what the classes are and report the agreement as conformance.

### The licence question is already answered, and the shape question is not

`deny.toml`'s allow-list reads `["Apache-2.0", "MIT", "BSD-2-Clause",
"BSD-3-Clause", "ISC", "Unicode-3.0"]`. Somebody has already decided that this
licence is acceptable inside the workspace proper — so this RFC does not have to
weigh those terms, and it does not: what it decides is the **shape** in which
they may enter, which is the part `deny.toml` structurally cannot see. That list
is checked by `cargo deny` against the crates cargo resolved. A file is not a
crate. So the one mechanism in this repository whose job is to notice a licence
entering the build would have stayed green through every version of this decision,
including the wrong ones, and that is the reason the lint changes below are the
substance of this entry rather than an appendix to it.

### Committed rather than fetched, because the failure mode of fetching is a skip

A corpus fetched by a command is a corpus that is absent on the afternoon
somebody's network is down, and the repair under time pressure is a test that
skips when the file is missing. A skipped conformance run reports nothing and
reads as a pass, which is the one outcome `claims/README.md` exists to prevent and
which `claims/canvas-corpus/` answers the other way — *the emptiness is the
finding*. This tree's builds also do not reach the network: `crates.io` is
reachable only through compose, and `cargo xtask lint` reaches nothing at all.

So the files are committed, and the cost is paid where it can be seen: every
clone carries them. **Their sizes are not known in this container** — there is no
network here and no file to measure — and they are named as the first thing
`PROVENANCE.md` records rather than guessed at. A harness that cannot find its
corpus **fails**, naming the file and the command that restores it. It does not
skip.

## Consequences

### What `lint_licensing` has to gain, precisely, and this RFC does not write it

`xtask/src/main.rs` is a shared file and another builder is in it this wave, so
what follows is specification rather than diff. Each item is red-able and each
one closes a route that is silent today.

1. **The SPDX check must read a value and not a prefix.** It is
   `if !text.starts_with("// SPDX-License-Identifier:")` today, which accepts any
   licence expression at all. A generated table carrying `(Apache-2.0 OR MIT) AND
   Unicode-3.0` therefore passes **and looks exactly like the four hundred files
   that are purely liftable**, which is the defect: the silence is worse than a
   refusal, because the one file in the permissive tree whose terms are not the
   tree's terms is indistinguishable from every file whose terms are. The repair
   is to require the exact permissive line everywhere, and to hold the exceptions
   in a table — `DERIVED_DATA: &[(&str, &str, &str)]`, of (the permissive file,
   the upstream file it was generated from, that file's SHA-256) — whose every
   entry must open with the dual line and carry a header naming the same upstream
   file and the same hash. A table because a diff to a table is reviewable, which
   is `DETERMINISM_ALLOW`'s whole argument, and because the count of files in this
   tree that are not purely liftable is then a number somebody can read.
2. **A run-time path into the import must be named rather than unrefused.**
   Check 2 refuses `use third_party` and `third_party::`. A string literal is
   neither, so `std::fs::read_to_string("third_party/unicode/BidiTest.txt")` in a
   permissive file is silent — correct for the conformance harness, correct for
   the generator, and wrong for library code, and a lint that cannot tell those
   apart is not saying anything about any of them. The repair has a shape this
   tree already uses for *exactly one call site*: a table `IMPORT_READERS:
   &[(&str, &str)]` of (file, why), holding the harness under `text/tests/` and
   the generator, with any other permissive file containing the literal
   `third_party` a finding. `THE_ONE_READING` and `CLOCK_READS` are the
   precedent, and RFC 0103's lesson rides along — a rule with one call site has to
   count the call site, or a path with no reader and a path with one are
   indistinguishable.
3. **Rule 3 must become checked.** `LICENSING.md` says every imported tree
   carries its own `LICENSE` and a `PROVENANCE.md` recording upstream URL, commit
   hash and date imported. Nothing reads for either — `grep -n PROVENANCE
   xtask/src/main.rs` returns nothing — and the check has been costless to omit
   because `third_party/` is empty. It stops being costless on the day the first
   entry lands, which is the day nobody will be writing a lint. For a data import
   the record needs two fields more than rule 3 lists: the **SHA-256 of every
   file**, because an imported data file that is not byte-identical to upstream is
   not the specification's corpus any more, and the **Unicode version**, because
   an expectation is only an expectation of a version.
4. **`TOOLING`'s row must say this, or the generator is blessed by accident.**
   `xtask/` is exempt from the source checks with the reason *build tooling: it
   runs outside the system under test, and it contains the needles the policy
   checks search for*. That reason is true and it is not this one. The generator
   reads the import; it is permitted to, and today it is merely *unread*. A
   reader who finds the exemption will conclude the generator was considered when
   it was not.

### What a third party has to be told, and where

`LICENSING.md`'s table gains the row, and the row is not enough on its own: the
file also has to say that a **binary carries no file headers**. Every mechanism
above puts the notice in a source file, and the one artefact a third party is
most likely to receive is the one where none of them is visible. So the sentence
that has to exist somewhere a redistributor reads is that a binary built from
this tree includes tables derived from Unicode data files and carries that
licence's notice with it — and `LICENSING.md` is where it goes, because that is
the file this repository points at for the question.

Then, per artefact: `third_party/unicode/LICENSE` is the terms as they arrived;
`PROVENANCE.md` is the URL, the version, every file with its hash, the date, and
*nothing was changed*; every generated file's header is the upstream file, its
hash, the version and the regeneration command; and the dual SPDX line is the
machine-readable form of all of it, so a downstream tool gets the right answer
without reading prose.

### What this makes easy, hard, and what it forecloses

**Easy.** `E3-B03e` and `E3-B03f` can start: the classes are a table in the crate
that needs them, the corpus is a file the harness opens, and neither waits on a
ring, a component or an allocator. `text/` keeps its zero dependencies. The
reordering pass stays in the permissive tree, which is where RFC 0082's table
already put it, so the epoch's text work remains liftable except for two
generated files that name their own terms.

**Hard.** Every clone carries the corpus. A Unicode version bump is a
regeneration, a re-hash and a review rather than a version bump. And the boundary
now has two kinds of thing behind it with two kinds of coupling, which is one more
thing for a reader of `LICENSING.md` to hold — the mitigation is that the new
kind is defined by what a compiler cannot read, which is a line nobody has to
remember because a lint can hold it.

**Forecloses.** The third category is a hole if it is ever widened by one step,
and the step it would be widened by is obvious: a *segmentation implementation*
arriving beside the break properties, or a normalisation table arriving beside
the classes because it was next to it. RFC 0082's creep condition is the one to
watch and it applies here verbatim — each such import is individually defensible
and the sequence is how a boundary becomes a subsystem.

## What would reverse this

**The table stops being a re-encoding.** The whole decision rests on a generated
file being *data* — an assignment somebody else made, re-spelled as an array. The
day the generator makes decisions of its own — collapsing classes, choosing a
default for an unassigned scalar, folding two properties into one — what is
committed is code derived from somebody else's data, the argument in *Context*
about behaviour having no isolation to buy no longer applies, and the row in
`LICENSING.md` is describing the wrong thing. The observation is in the
generator's diff and not in the table's.

**A second upstream, and then a third, walks through the row.** Break properties
for UAX #14 and #29 are the second and they are legitimate: the same rule selects
them, and they arrive with the same two routes. The third is the signal. If the
third thing under `third_party/unicode/` is not a UAX property table — if it is a
locale database, a collation table, a normalisation implementation — then the rule
in *Decision* did not select it and was not what admitted it, and this RFC needs
restating or superseding rather than citing.

**The ring turns out to be affordable, which is the alternative this entry
rejected rather than dismissed.** The measurement is a comparison of bytes: the
property tables' size in every image that links `f-text`, against one component
image holding them and a per-paragraph crossing in front of the shaping cache. If
the first grows to where several components each carry a copy of the same
hundreds of kilobytes, the arithmetic that refused a component stops holding, and
the table moves behind the boundary with the shaper it is already beside. Note
what that would *not* refute: the table would still be data, and the third
category would still be needed for the corpus, which no component can hold
because no component runs a test.

**The corpus becomes the dominant cost of a clone.** If the committed conformance
files grow past the point where the repository's size is what somebody notices
first, the *committed rather than fetched* half reverses on its own terms. The
replacement is not a skip — see *Context* — it is a fetch plus a harness that
fails by name when the file is absent, which is strictly worse for a reader and
strictly better for a clone, and the trade is only worth making against a measured
size rather than an imagined one.
