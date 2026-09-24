# RFC 0115: An expectation belongs to the pass that holds it, and the corpus stays the input half

- Status: accepted
- Date: 2026-09-24
- Affects: `text/src/corpus.rs` — its *What is deliberately not here* section,
  whose stated reason is narrower than it reads and is corrected by this entry
  rather than by a change to the rule; `TODO.md` E3-B03e and E3-B03f, whose exits
  say *the conformance corpus passes* and could not say where the expectations
  live; RFC 0114, which decides where the corpus file itself sits; RFC 0082's
  table, which put UAX #9 on the written side

## Decision

**`text/src/corpus.rs` stays the input half, and its rule does not bend.** No
glyph, no advance, no break position, and no embedding level goes into it. What
changes is that the rule now has a reason that survives the case it is about to
meet, and the expectations have two homes chosen by kind rather than by
convenience:

1. **The exhaustive expectations are the specification's own files, read by a
   host harness.** `BidiTest.txt` and `BidiCharacterTest.txt` sit under
   `third_party/unicode/` per RFC 0114 and are opened by a test under
   `text/tests/`. Hundreds of thousands of cases with levels and a reorder beside
   each one: they are an expectation this tree neither writes nor edits, and the
   harness is the only thing in `text/` that ever sees them.
2. **The argued expectations belong to the pass that holds them.** The eight
   entries of the corpus are argued one at a time, and what each one is *expected
   to do* differs per consumer: `E3-B03e` expects levels, `E3-B03f` expects break
   positions and cluster boundaries, `E3-B03g` expects an atlas residency, and
   `E3-B03j` expects pixels. Each of those lives with its own pass, in that
   module's own tests, derived from the numbered rules with the derivation
   written beside it.

This is `corpus.rs`'s own sentence made binding — *`E3-B03e`, `E3-B03f`,
`E3-B03g` and `E3-B03j` each iterate `Script::ALL` and bring their own
expectations* — and what this entry adds is the argument for why that is right
rather than merely what was done first.

## Context

Last wave's builder refused to start `E3-B03e` and named this as the second of
two blockers, correctly: a reordering test needs expected levels, and the module
that holds the corpus refuses to hold an expected output on purpose. Either the
rule bends or the expectations live somewhere that is not `text/src/corpus.rs`,
and nobody had decided which.

### The file's stated reason does not cover this case, and that is the trap

`corpus.rs` says, in *What is deliberately not here*:

> An expected output needs a shaper, and RFC 0082 put the shaper behind the
> licence boundary where this crate cannot link it.

For a glyph and an advance that is exactly right. **For an embedding level it is
false**, and the falseness is the dangerous part rather than the harmless part.
A level is a function of the scalars and their `Bidi_Class` — the paragraph
direction, the explicit formatting characters, the bracket pairs. No face, no
size, no feature set, nothing behind the licence boundary is consulted to know
it. So a reader arriving at `E3-B03e` and reasoning from the file's stated reason
finds that the reason does not apply, concludes the refusal does not apply
either, and writes levels into the corpus. The rule would then have been bent by
a premise rather than by a decision, which is the worst way for a rule to go.

This entry corrects the premise and keeps the rule, and `corpus.rs` gains the
narrower reason in its own words so that the next reader starts from a true one.

### The file's second reason holds, and is satisfied rather than bent

> A corpus that guessed at outputs today would be a corpus asserting what this
> tree's first shaper happens to do, which is the fixture that makes every later
> shaper wrong by definition.

This is the load-bearing half and it survives intact — and neither home above
violates it. `BidiCharacterTest.txt` is not a guess and is not this tree's: it is
the specification's expectation, which is what the word conformance means, and a
fixture taken from it makes a *later implementation* wrong only if the later
implementation is wrong. The argued eight are not guesses either, and the
discipline that keeps them from becoming ones is that each expectation is written
with the rules it was derived from named — X1 through X8, W1 through W7, N0
through N2, I1, I2, L1, L2 — so a reader can check the derivation instead of
trusting it. An expectation with no derivation beside it is the guess the file is
warning about, whichever file it is written in.

### The reason the file does not state, which is the one that decides the shape

`corpus.rs` is a `const` table in a `no_std` crate with no allocator, eight
entries long, where every entry is checked at compile time to have a sentence
saying what its absence would hide, a sample inside its declared ranges, ranges
its sample reaches, ranges narrow enough to mean something, and a signature no
other entry has. That machinery is what makes the corpus *argued* rather than
collected.

A conformance corpus is the opposite kind of object. It is chosen by
exhaustiveness, its cases have no sentence and cannot have one, no case is
distinguishable from its neighbour by a hazard, and there are hundreds of
thousands of them. Merging the two destroys precisely the property the const
assertions hold: `entries_exercise_different_things` is a statement nobody can
make about a case machine-generated from a class sequence, and a file where most
entries cannot satisfy the checks is a file whose checks get an exemption. The
corpus file says of itself that it is chosen *by argument rather than by
coverage*; conformance is coverage. They are two objects and the decision is to
keep them two.

### And the argument that keeps even the small, non-guessed expectations out

The eight argued samples' levels are few, cheap and correct. The temptation to put
them in `corpus.rs` anyway is the strongest thing on this side and it loses for a
reason that has nothing to do with shapers: **two passes have different
expectations about one sample.** A reorderer's expectation about `mixed` is three
runs at levels 0, 1 and 2; a breaker's is a set of positions; the atlas's is a
residency count; `E3-B03j`'s is an image. A file holding all four becomes the
union of every pass's beliefs about eight strings, and then *adding a script is a
diff to one list and to nothing else* — the property that module is built around,
and the property it says out loud — becomes a diff to one list and four
expectations, three of which the author of the new entry is not qualified to
write.

Worse, there is no check available for an expectation the way there is for a
sentence. `is_a_sentence` can refuse an empty `hides` field; nothing can refuse a
wrong level. So an expectation in the corpus would be the one field in that file
with no compile-time guard behind it, sitting in a file whose entire argument is
that its guards are compile-time.

## Consequences

### What the harness owes, and none of it is the algorithm

`E3-B03e` writes the reordering and this entry writes none of it — RFC 0088's
standing rule, that a *decide* task's exit may not require its own
implementation. What the decision does fix is four things about the harness,
because each one is a way for a conformance run to report nothing and read as a
pass:

1. **It prints how many cases it ran, and a run of zero is red.** A parser that
   silently matched nothing is the commonest failure of a text-fixture harness,
   and `claims/canvas-corpus/` already holds this tree's answer to the same shape
   of question: the emptiness is the finding. A corpus file that is absent fails
   by name with the command that restores it (RFC 0114) and does not skip.
2. **The representative character per class comes from the generated table.**
   `BidiTest.txt`'s cases are sequences of *classes*, not of characters, so
   running them requires choosing a scalar for each class. That choice must be
   read out of the table RFC 0114 generates — the first scalar whose class is that
   class — and not written by hand, because a hand-written list would make the
   harness a test of the list against the table the list was copied from, which is
   the *second copy* failure this repository has recorded in four different files.
   This is the one seam where the two RFCs meet, and it is the place where a
   conformance run could quietly be testing its own opinion.
3. **The cases the chosen level excludes are data, not a comment.** `E3-B03e`'s
   exit asks for a conformance level *named in the task rather than implied by the
   code*, and for the excluded cases to be listed *where a reader will find them*.
   A comment above a `continue` is not that. A table of excluded case kinds, each
   with a reason, read by the harness and printed in its summary, is — and the
   count of excluded cases is then a number that moves when somebody widens the
   exclusion.
4. **It runs on both architectures and the expectation is integers.** `f-text`
   carries `None` in both columns of `xtask`'s portability table, so the AArch64
   job compiles and tests it. A level is a small integer and a reorder is a
   permutation, so there is no rounding anywhere in this comparison — which means
   a divergence between the two architectures here would be a real defect and not
   a float, and that is worth having a place to observe.

### What `text/src/corpus.rs` gains, and it is not a field

One paragraph. The stated reason becomes the narrower true one, with this entry's
number beside it, so the next reader of that section starts from a premise that
holds for levels as well as for advances. No field, no expectation, no second
list, and the eight entries are untouched.

### What this makes easy, hard, and what it forecloses

**Easy.** `E3-B03e` and `E3-B03f` can be written: the input corpus is already
there and unchanged, the expectations have an address, and the harness has four
obligations stated before it is written rather than discovered in review.

**Hard.** An expectation about one sample is now in two places at once when two
passes both have one — the reorderer's levels for `mixed` in one module, the
breaker's positions for `mixed` in another — and nothing checks that the two are
about the same string, beyond both naming the same corpus entry. That is the
price of the decision and it is the right way round: a wrong expectation in one
pass's tests fails that pass, where a wrong shared expectation would fail
whichever pass was written second and look like that pass's defect.

**Forecloses.** A single answer file that says what the whole text path does to
the eight samples — which would be the obvious way to hold a rendering
comparison, and is `E3-B03j`'s problem rather than this one's. If that file ever
becomes the right object, it is a new artefact with its own argument and not a
field added to `corpus.rs`.

## What would reverse this

**An expectation that is genuinely common to every pass.** If two or more passes
turn out to need the *same* expected value about a sample — not two values about
one sample, one value both consult — then the argument in *Context* about a union
of beliefs stops applying to that value, and it belongs with the input because it
is as much a property of the sample as the sample's direction is. The scalar count
per cluster is the candidate: a reorderer, a breaker and an atlas all care about
it. The reversal is that value being written down twice and the two copies
disagreeing, which is observable and which nobody should wait for.

**The conformance corpus turning out not to be an expectation this tree can hold
without editing it.** If the harness has to *modify* cases to run them —
normalising line endings is fine, changing a case's expected levels is not — then
what is being run is not the specification's corpus, the *not a guess* argument
above evaporates, and the expectations are this tree's after all. At that point
they are ordinary fixtures and belong wherever fixtures belong, which is beside
the pass, and this entry's first home disappears rather than moves.

**`corpus.rs` growing a sentence it cannot check.** The rule kept here is held by
the file's five compile-time assertions, and the moment an entry carries a field
no assertion can refuse, the argument that the corpus is different in kind from a
conformance file has lost its mechanism. If that happens, the honest reading is
that the corpus has become a collection, and the decision to keep two objects
should be re-taken rather than inherited.
