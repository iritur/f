# RFC 0090: A reference image is for what only pixels show, and an interface assertion is not that

- Status: accepted
- Date: 2026-09-17
- Affects: `TODO.md`, whose `E3-B03` and `E3-P05` exit lines are replaced below
  and each annotated with this number; `intent/0012-the-interface/spec.md`, whose
  `E3-B03j` was written to cite this entry before it existed and now cites it by
  number; `docs/test-taxonomy.md`, which gains the corpus's row on the day
  `E3-B03j` lands and does not gain one today, because a taxonomy row for a test
  nobody has written is a promise in a table of evidence
- Owed by: `intent/0012-the-interface/plan.md` *Handoff* step 8, which named this
  contradiction, declined to resolve it, and said why: a plan may not reverse a
  written exit

## Decision

Two written exits contradicted each other and this settles which sentence each
one keeps. `E3-B03` asked that a corpus of scripts and directions render
*correctly against reference images, in CI*. `E3-P05` asked that *no test in the
tree compares images*. Both are defensible and they cannot both stand.

**Neither wins whole.** The resolution is not that one of the two lines was
wrong; it is that both were stated over the wrong set.

**One. An image may be compared to an image only where the artefact under test is
pixels and no assertion over a smaller artefact can observe the defect.** Today
that is exactly one place in this project: the text conformance corpus behind
`E3-B03j`. Not *rendering* in general, and not `E3-B03` in general.

**Two. `E3-P05`'s sentence narrows from a property of the tree to a property of a
kind of test.** *No test in the tree compares images* becomes *no interface
assertion compares images*. What that task is for is unchanged, which is why the
narrowing costs it nothing: a UI test that breaks on a redesign which changed no
meaning is the brittleness `E3-P05` exists to end, and a screen full of Arabic
that shapes wrong is not that failure wearing a different hat. The original
sentence was the right rule quantified over the whole tree because, on the day it
was written, there was no rasteriser in the tree and the wider sentence cost
nothing to say.

**Three. `E3-B03`'s corpus is asserted at the glyph level first, and the image
comparison covers only the remainder.** Script and direction coverage — which is
most of what `E3-B03` is *for*, and all of `E3-B03e`'s and `E3-B03f`'s risk — is
observed as glyph ids and positions out of the shaper, which are integers, diff
cleanly, name the defect in their own output, and need no rasteriser to produce.
A reference image is then required only for what that cannot observe: coverage
per pixel, compositing in linear space, the atlas boundary, and text under a
transform. A corpus that compared images to catch a reordering bug would be using
the most expensive instrument in the tree to observe something an integer already
says.

**Four. The carve-out carries a mechanism, or it is not a carve-out.** A rule
whose only enforcement is that everyone remembers it grows one convenient
exception at a time, and this project has written that sentence down twice
already. `E3-B03j` owes a lint in `lint_all` refusing an image-to-image
comparison outside the one directory the corpus lives in, in the shape
`lint_licensing` already has for the permissive tree's imports. Until that lint
exists the carve-out is honoured by convention, and this entry says so rather
than implying otherwise.

## Context

Neither task has been started. `E3-B03` is `XL` with eleven subtasks and
`E3-P05` is `M` behind `E3-B06`, and neither has a line of code. That is the
argument for deciding now rather than later, and it is ordering rule 1's argument
rather than a new one: a contradiction between two sentences is free to resolve
while both are sentences, and expensive once one of them is a directory of images
and a CI job.

`intent/0012-the-interface/spec.md` found the contradiction while decomposing the
epoch and deliberately left `E3-B03`'s exit as the one line of the eight it
proposed no replacement text for — *because it contradicts `E3-P05`'s and
resolving a contradiction between two written exits is a reversal that needs an
RFC*. `E3-B03j` was then written to cite an RFC by a number nobody had allocated.
That is the one forward reference in the decomposition and it is now resolved.

**The alternative that was live, and why it is half-taken rather than refused.**
Glyph-level assertions — glyph ids and advances out of the shaper, compared to a
recorded list — are better than images on every axis this tree cares about:
deterministic across architectures, readable in a diff, immune to a gamma change,
and they name the cluster that moved. The case for taking them *instead* of
images is strong enough that refusing it outright would have been a decision made
to preserve `E3-B03`'s wording. What they cannot do is observe a rasteriser: a
coverage rule wrong by a subpixel, a composite done in the wrong space, a glyph
straddling an atlas tile, and a path rendering disagreeing with the atlas it
replaced are all invisible to a list of glyph ids — and `E3-B03h` is a task whose
whole content is choosing between two renderers whose outputs must agree. So the
alternative takes the larger half of the corpus and the images keep the part that
is genuinely theirs.

**What this does not do.** It does not decide the comparison itself. The
tolerance, the metric, and the platform question — whether two machines are even
required to produce the same pixels — are `E3-B03j`'s, whose exit already asks
for an integer over pixels with a stated tolerance and a deliberate one-pixel
regression going red. This entry decides where such a comparison may exist, not
what it is.

## Consequences

**What this makes easy.** `E3-B03` can be built without arguing with `E3-P05`
every time a test is added, and `E3-P05` can convert a UI suite without excepting
the text corpus by hand. Both lines now say what they mean over the set they mean
it over.

**What it makes hard, deliberately.** Adding the second carve-out. Anyone who
wants to compare images somewhere else has to come back here and reverse the word
*exactly one*, and that reversal is visible because the number is in the
sentence. That is the property the original blanket had, and the reason narrowing
it needed this document rather than an edit.

**What it forecloses.** Screenshot-based assertion of anything above the
rasteriser, permanently — including the tempting version, an end-to-end test of a
projection that "just checks the screen". `E3-B06`'s exit is a compile-fail
fixture and `E3-P05`'s is a semantic tree for exactly that reason, and this entry
does not reopen either.

**A cost this entry pays and should be read as paying.** `E3-B03`'s exit is
narrower than it was: *renders correctly against reference images* now reaches
only the part of the corpus images observe, and a reader who wanted the headline
*the whole text corpus is verified against references* may not have it. Per
RFC 0084 that is a reversal even though nothing here was measured, and it is the
third narrowing of an E3 exit in two days. The rate is the thing to watch rather
than this instance: RFC 0084's *A shape, not yet a rule* says what to examine if
it continues, and it is how exits are written rather than the repairs.

## What would reverse this

**The glyph-level half proving sufficient.** If, over a run of epochs, every
defect the image corpus catches is one the glyph assertions had already caught,
then the carve-out is buying nothing and `E3-P05`'s original unqualified sentence
should come back. That is a measurement `E3-B03j` can record for free: every red
run notes whether the glyph assertions were red in the same run.

**The corpus going red on something that is not the renderer.** A reference
comparison that fails on a toolchain bump, a host difference or a font-file
revision is measuring the harness. One such failure is a tolerance to fix; a
second, from a different cause, means images are not stable enough to be evidence
in this tree, and the carve-out should be withdrawn rather than widened.

**A second place earning the exception.** If a task outside `E3-B03` produces a
real argument that its artefact is pixels and nothing smaller can observe its
defect — `E3-B02`'s four rungs having to agree is the candidate, and it is a near
miss, because two rungs can be compared to each other rather than to a stored
image — then *exactly one place* is the wrong shape, and this becomes a category
with a test for membership. Deciding that in advance would be inventing the
category from one example.

**And the lint not landing.** If `E3-B03` closes with the corpus in the tree and
no lint refusing an image comparison outside it, then point four was a sentence
rather than a mechanism, and the honest response is to say so here rather than
keep the paragraph.
