# RFC 0085: A book is a third kind of document, and it states what it is ahead of

- Status: accepted
- Date: 2026-09-16
- Affects: `docs/book/` (new), `README.md` (the reading order and the layout
  block), `CLAUDE.md` (the Architecture paragraph, which currently names only
  two kinds of document), `xtask/src/main.rs` (`documents()`, which is widened
  here and is the reason this RFC is not purely editorial),
  `intent/0013-three-books`. It does not affect `docs/design/`, `docs/rfc/` or
  `claims/`, and saying so is half the decision.

## Decision

This repository gains a third kind of document, and the three kinds are
distinguished by **what permission each one carries about being ahead of the
code**.

- A **design document** in `docs/design/` is an argument, aimed at a reader who
  wants to disagree. It is permitted to be ahead of the code, and it is, on
  purpose. This is unchanged.
- An **RFC** in `docs/rfc/` is a decision and the conditions that would reverse
  it. It is about a moment. This is unchanged.
- A **book** in `docs/book/` explains the system to a particular reader.
  **It carries no permission to be ahead of the code**, because a book reads as
  a manual, and a manual describing behaviour the system does not have is not
  ahead of the code — it is wrong about it. A book therefore states, in its
  first screen and again at the head of every chapter, what fraction of it
  currently runs.

There are three books, and they are divided on the axis Genode divides its own
three on: **what the reader takes as given.**

| Book | The reader takes as given | Covers |
| --- | --- | --- |
| F Foundations | the hardware | the system itself, for somebody who may change it |
| F Applications | the system | writing a component, for somebody who takes F as given |
| F Evidence | nothing, including the claims | the apparatus that decides whether any of it is true |

Four rules govern all three:

1. **A book cites; it never restates.** An RFC is referenced by number, never
   summarised into a second account of the decision. A claim is referenced by
   name, never copied as a value. A book that restates becomes a second place a
   decision is recorded, and two places eventually record it differently.
2. **No measured number appears in a book.** A measurement — a latency, a
   throughput, a ratio against a baseline, anything that is or ought to be an
   entry in `claims/` — is *named and not quoted*: the book gives the claim and
   the command that re-derives it. This is stricter than the rule on design
   documents, which may cite a claim value through a `data-claim` span, and it
   is stricter deliberately: a book is the document a newcomer reads, so it is
   the document a stray number does the most damage in.

   **Structural counts of the tree are not measurements and may be stated** —
   how many crates are in the frame, how many words cross a core boundary, how
   many fault classes there are. The distinction is whether the number came from
   *running* the system or from *looking at* it. A reader can check the second
   kind by reading the tree, and in most cases a lint goes red when one moves;
   the first kind is exactly what `claims/` exists to stop a document owning a
   second copy of. This paragraph is here because the first draft of this rule
   said "no number with a value" and the book it was written for stated four
   shards and six fault classes on its second page, which meant the rule was
   wrong rather than the book.
3. **Format follows `docs/design/`.** Self-contained HTML, the same stylesheet,
   the same dark-mode handling. No generator and no build step, because a
   document that needs a toolchain is a document that stops being rebuilt.
4. **A book is revised at a gate**, not continuously, and its status block names
   the gate it was last true at. F has no releases, so a release cadence is not
   available to borrow.

And one change to code, made in the same diff rather than after it:
`documents()` in `xtask/src/main.rs` now recurses from `docs/` instead of naming
`docs/` and `docs/design/` non-recursively.

## Context

The ask was for "the same sort of books as Genode has". Genode ships three —
Foundations, Applications, Platforms — revised every release. Reading them, the
useful thing is not the subject division but the axis: each book is defined by
what its reader is willing to take as given, and a chapter belongs to the book
whose reader needs it. Device drivers are an architectural position in one book
and a thing you write in another, and that is not duplication.

This repository has a great deal of writing and no book. Five design documents,
eighty-odd RFCs, thirty-odd claims, a manifest schema in prose, a test
taxonomy, three boot narratives, an SDLC, and two long-form pages sitting loose
in `docs/`. All of it is organised by *what kind of statement it is* — a
reasoning, a decision, a number, a procedure. That is the right axis for a
project defending itself and the wrong one for somebody arriving: the answer to
"how do I write a component" is currently spread across `docs/manifest.md`, RFCs
0008, 0015, 0065 and 0077, and a worked example in `user/virtio-blk/`, with
nothing anywhere saying so.

Two alternatives were live, and both were rejected for stated reasons.

**Keep Genode's set exactly, with Platforms as the third book.** Rejected
because F's Platforms book would be thin, and thin for a reason that is a
deliberate design decision rather than a gap: F pins itself to one machine class
with an IOMMU (RFC 0031), has two architectures rather than many kernels and
many boards, and has booted twice in a virtual machine and never on metal
(`E0-P18`). Genode needs that book because platform variety is Genode's problem.
It is not F's. Meanwhile the thing F has that Genode does not is the evidence
apparatus — the claims registry, the simulator, the sweeps, the litmus tests,
the Kani proofs, `mutate`, `attest`, `compare` — which is both a book's worth of
material and the part of this project most worth explaining to an outsider.
Swapping Platforms for Evidence keeps the axis and drops the parity. Platform
bring-up stays where it already is: a Foundations chapter, plus
`docs/booting-on-hardware.md` and the three boot narratives.

**Write all three now.** Rejected. Genode wrote Applications after fifteen years
and a shipping desktop. F is at M0 with one first component and no compositor, so
an Applications book written today would have no worked example that was not
invented, and an invented worked example in a manual is exactly the failure this
RFC's third rule exists to prevent. Applications is gated on E3. If it is wanted
sooner, it is written as a *design document* — which may be ahead of the code —
and promoted to a book when it has an example, rather than written as a book
with the examples left blank.

The `documents()` widening is not incidental. That function walked exactly two
directories, neither recursively, and both `lint-claims` and `claims --render`
run off it. A book at `docs/book/foundations.html` would have been invisible to
both: a number could have been published there with nothing in `claims/` behind
it and the build would have stayed green. That is `CLAUDE.md`'s own scar about
directory walkers carrying their own skip lists, arriving from the other
direction — not a walker that skipped too much, but one that reached too little.
Widening it in the same diff that creates the directory is the whole of what
keeps this RFC from quietly weakening the claims discipline it claims not to
touch.

## Consequences

**What this makes easy.** A newcomer with one of three questions is pointed at
one file rather than at a reading order across five. The design documents are
freed from a job they were doing badly — `fast-path.html` is the case for five
bets and was never a chapter, and flattening it into one would have lost the
only document that says why any of this is worth doing. And the evidence
apparatus, which is currently the hardest part of this project to explain and
the easiest to mistake for bureaucracy, gets a reader of its own.

**What this makes hard.** There are now three more documents that can stop being
true, in a tree that already has five design documents, eighty-odd RFCs and a
`lint-owed` whose whole job is that documents go stale when a reversal is paid.
Rule 4 — revised at a gate, with the gate named in the status block — is the
mitigation, and it is a weaker mitigation than a lint. A book that claims to be
true at G0 when G0 has not been passed is caught by nothing mechanical.

**What this forecloses.** A fourth book. Three is already more than this tree can
keep true, and the next subject that wants book-length treatment is a chapter in
one of these or a design document, not a fourth volume. It also forecloses the
easy version of publishing a number: a book may not carry one, so a number that
wants to be in front of a newcomer has to become a claim with a baseline and a
reproduction first, which is the longer route and the intended one.

**What it does not change.** `docs/design/` keeps its permission to be ahead of
the code, and no design document is replaced, moved or renamed.
`lineage-and-debts.html` in particular has no book home and should not acquire
one. No claim's `document` field moves, so `lint-claim-owners` is untouched.

## What would reverse this

Four observations, each of which should change the decision rather than prompt a
patch.

- **A book is found to be materially wrong about what runs**, in a way a reader
  acted on. That is rule 4 failing, and the response is to make the status block
  mechanical — a lint that reads the gate a book claims and the gate `TODO.md`
  says has been passed — rather than to write the status block more carefully.
- **The Applications book is still ungated two epochs after E3 closes.** That
  would mean the component-facing story is not explainable rather than not yet
  built, which is a finding about the architecture and not about the
  documentation.
- **F acquires a second machine class or a second kernel base.** The argument
  against Platforms above rests entirely on F pinning to one. If that pin moves —
  and RFC 0031 names its own reversal conditions — Platforms becomes the right
  third book and Evidence becomes a chapter of Foundations, or there are four
  books and this RFC's foreclosure was wrong.
- **Somebody publishes a number in a book anyway and the lint does not catch
  it.** Rule 2 is currently a convention; `lint-claims` only checks citations
  that are *declared* as claim spans, so a bare figure in prose passes. If that
  happens once, the response is a check that a book contains no digit sequence
  outside a code block without a claim reference beside it — ugly, and cheaper
  than the alternative.
