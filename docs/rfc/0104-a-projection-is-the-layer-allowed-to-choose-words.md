# RFC 0104: A projection is the layer allowed to choose words

- Status: accepted
- Date: 2026-09-23
- Affects: `interface/src/canvas.rs` (the sentence this reverses and one
  visibility), `interface/src/reader.rs`, `interface/src/node.rs`

## Decision

**A projection may emit words in one language. Nothing below a projection may.**

`interface/src/canvas.rs` says a module that emits English is one every
non-English consumer has to work around, and refuses to assemble a sentence.
That argument is right about a *linearisation* — an ordering of nodes is
language-free and should stay so — and it does not survive a screen reader:
somebody has to own the nouns, and a projection whose output is a phrase
sequence is the layer whose whole job that is.

So the language is confined to **one field**. `Phrasing::noun` carries the word;
everything else a consumer needs — role, state, position, extent, times — is
language-free data beside it. A consumer that wants a different language
replaces the noun and keeps the structure.

**The exhaustive `match` is the thing that must survive, and it is the reason
this is a decision rather than a detail.** `E3-B06i`'s exit asks for a `match` on
`Role` with no wildcard, twice, and the reason is that a `[&str; Role::COUNT]`
indexed by `Role::index` compiles when a twenty-third role is added and yields
the wrong phrase in silence. That is RFC 0077's closed vocabulary being closed
for nothing.

## Context

Two smaller reversals ride with this and are recorded rather than left to be
found:

- **`Operable::of` widened from private to `pub(crate)`.** `canvas.rs` said in
  as many words that *the privacy is the scoping*: a public version would answer
  *what may be invoked on this* for a node no canvas admitted. The reader asks it
  of nodes in a tree rather than nodes in a canvas, which is a different set with
  the same question, so the scoping moves from the keyword to the crate and the
  sentence in `canvas.rs` is corrected rather than left standing.
- **The timeline is described without a pixel, which is RFC 0078's own
  falsification run as a test.** The clip between 4.2 s and 6.8 s on track 3 is
  selected and described, and *4.2 s* is a rendering of an integer. No float
  reaches the tree — RFC 0004 leaves no other option — and the time base is
  `canvas.rs`'s, taken rather than re-invented.

## Consequences

**What this makes easy.** A second projection — remote, agent — takes the same
tree and the same `Role` match and emits something else. The screen reader is
the first of four and the one that forces the question, because it is the only
one whose output is words.

**What this makes hard, deliberately.** Adding a role is now a compile error in
this projection as well as in the vocabulary. That is the cost the exit asked
for and it is paid where a reader meets it.

**What it forecloses.** Nothing about locales. A key table resolved by a locale
is a perfectly good later design; what it must not do is become a table indexed
by ordinal, which is the silently-wrong table wearing an internationalisation
argument.

## What would reverse this

**A second locale.** At that point the noun becomes a key a locale resolves, and
the property that must survive the change is the `match` — not the words. An
implementation that replaced the `match` with a lookup while adding the locale
would close this RFC and reopen the defect it exists to prevent, and the test to
keep is the compile-fail one.

**A consumer that needs a phrase this projection does not produce.** If a screen
reader's output turns out to need more than a noun and language-free structure —
agreement, inflection, ordering that depends on the word — then the language is
not confined to one field and this decision is wrong about where the boundary
is. The observation is a projection that starts formatting; the answer then is a
phrasing layer of its own, not a wider field here.
