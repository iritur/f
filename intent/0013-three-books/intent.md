---
id: 0013
status: draft
originator: Dmitri Chudinov
todo:
---

# Three books, the way Genode has three books, and an honest account of which of them F can fill

*Written after reading Genode's set — Foundations, Applications, Platforms — at
the 26.05 revision. The ask was "the same sort of books for F". This records
what that would mean here, what the tree already contains that would go in
them, and the one place where copying the set exactly would produce a document
that is a lie about how far the system has got.*

## Problem

This repository has a great deal of writing and no book.

What it has: five design documents in `docs/design/`, which are arguments aimed
at a reader who wants to disagree, and which are ahead of the code on purpose.
Eighty-odd RFCs, which are decisions and reversals, indexed by number and not by
subject. Thirty-odd claims, which are numbers with baselines. A manifest schema
in prose, a test taxonomy, three boot narratives, an SDLC, two long-form pages
(`the-long-plan.html`, `what-must-be-stated.html`) that sit in `docs/` beside
everything else with no stated relation to it.

None of that is organised by **who is reading it**. It is organised by *what
kind of statement it is* — a reasoning, a decision, a number, a procedure. That
is the right axis for a project defending itself, and the wrong axis for
somebody arriving. A person who wants to write a component against F has to
know that the answer is spread across `docs/manifest.md`, RFC 0008, RFC 0015,
RFC 0065, RFC 0077 and a worked example in `user/virtio-blk/`, and there is
nothing that tells them so. A person who wants to boot F on their own machine
has four documents, none of which says it is the first one.

Genode's set is organised on the other axis, and the axis is **what the reader
takes as given**. Applications takes the system as given and builds on it.
Foundations takes the hardware as given and builds the system. Platforms takes
nothing as given and brings it up on metal. Three readers, three books, and a
chapter belongs to the book whose reader needs it.

The second half of the problem is that F is at M0. It boots, it says so, and it
is deterministic. Genode wrote its Applications book after fifteen years and a
shipping desktop. Copying the set one for one today would produce two books with
material behind them and one that describes writing components for a system that
has one component, and that third book would read as a manual while being
fiction. This tree already has a name for that failure — a design document is
permitted to be ahead of the code *because it is labelled as reasoning*. A book
is not labelled as reasoning. That is the whole difference, and it is the thing
this intent exists to get decided before anybody writes 200KB of HTML.

## Proposed outcome

Three books exist, on Genode's axis, each stating in its first screen what
fraction of it currently runs:

- **F Foundations** — the system itself, for a reader who wants to understand or
  change F. The frame, determinism, capabilities, the ring protocol, components
  and manifests, the five resource subsystems, the interface, what is under the
  hood, how work reaches the repository, and the reference material.
- **F Applications** — for a reader who takes F as given and writes a component.
  What a component is here, the manifest field by field, talking on a ring,
  capabilities you ask for and cannot get back, declaring a state tree, drawing
  in the semantic vocabulary, failing and being restarted, and testing yours in
  the simulator.
- **F Platforms** — for a reader bringing F up on a machine. What F requires of
  hardware, QEMU and the fault boots, real metal, measured boot and attestation,
  drivers and the licence boundary, cores and bring-up, runner classes and which
  machines may record a number.

Observable when it exists: a newcomer with one of those three questions is
pointed at one file, not at a reading order across five. `README.md`'s "Reading
order" section becomes three lines instead of five. And every number in all
three books resolves to `claims/`, checked by the lint rather than by a reviewer.

**Staged, in this order, and for stated reasons.** Foundations first: it has the
most real material behind it and it is the book the other two cross-reference.
Platforms second: `booting-on-hardware.md` and the three `*-boot-outside-qemu.md`
narratives are most of it already, so it is largely a reorganisation of writing
that exists, which is the cheapest of the three and the most immediately useful
to anybody who wants to reproduce anything. Applications last, gated on E2 and
E3: until there is more than one component and a compositor to draw into, that
book has no worked example that is not invented, and an invented worked example
in a manual is the failure named above.

## Affected users and systems

**Documents that would have to change.** `README.md` — the "Reading order"
section and the `docs/` entries in "Layout". `CLAUDE.md` — the Architecture
paragraph currently says `docs/design/*.html` is the reasoning, and would need to
say what a book is and how it differs, which is the sentence that keeps the
distinction alive. `docs/sdlc.md` names no documentation stage and may not need
to.

**The five design documents are not replaced.** This is the expensive question
and the answer proposed is that they stay exactly as they are. A design document
argues; a book explains. `fast-path.html` is the case for five bets, aimed at
somebody who thinks the bets are wrong — that is not a chapter, and flattening
it into one would lose the only document that states why any of this is worth
doing. The books cite them. `lineage-and-debts.html` in particular has no book
home at all and should not acquire one.

**`xtask`, and this is a finding rather than a note.** `documents()` in
`xtask/src/main.rs` walks exactly two directories — `docs/` and `docs/design/` —
non-recursively, `.html` only. `lint-claims` and `cargo xtask claims --render`
both run off it. So a book written to `docs/book/foundations.html` is invisible
to the claims lint: a number could be published there with nothing behind it and
the build would stay green. That is `CLAUDE.md`'s own scar about a directory
walker with its own copy of the skip list, arriving from the other direction.
Either the books go flat in `docs/` where the existing walker already sees them,
or `documents()` is widened to recurse **in the same diff that creates the
directory** — never after.

`lint-claim-owners` (R09) is the second-order version: a claim names the
`document` that owns it, and every claim currently names a file in
`docs/design/`. If any owning document moves, those names move with it.

**Crates:** none. No code changes except the walker above.

**People:** whoever is on `iritur/twenty-lines-before-a-machine` and
`iritur/the-log-on-a-screen` — both are live worktrees and both touch `docs/`.
RFCs 0082 and 0083 are already taken on the first of those and are not yet on
`main`, so this change's RFC is 0085 and not 0082. `docs/postmortem/0001` is the
reason that sentence is in this file.

## Constraints

- **The claims discipline applies unchanged.** Any number that reaches a book
  needs an entry in `claims/` with a baseline, a workload and a one-command
  reproduction. The books are a new surface for numbers to escape through, and
  the lint has to cover them on the day they appear rather than the day somebody
  notices.
- **A book states what it is ahead of.** The permission `docs/design/` has to be
  ahead of the code is a permission that document carries because it is labelled
  reasoning. A book inherits no such permission, so each one carries a status
  block naming the milestone it describes and what in it does not yet run. A
  chapter describing something unbuilt says so in the chapter, not in a preface
  nobody reaches.
- **Format follows `docs/design/`.** Self-contained HTML, the same stylesheet and
  the same dark-mode handling, no new toolchain, no build step. A book that needs
  a generator is a book that stops being rebuilt.
- **No renumbering.** RFC, claim, intent and `TODO.md` identifiers are permanent.
  A book cites them; it does not restate them, and it never becomes a second
  place where a decision is recorded.
- **One page, on purpose, still applies to `CLAUDE.md`.** Whatever the books say
  about themselves, `CLAUDE.md` gains at most a sentence.

## Open questions

1. **Is Platforms the right third book for F, or is Evidence?** Genode's third
   book is platform-specific because Genode runs on many kernels and many boards
   and that variety is its problem. F pins itself to one machine class with an
   IOMMU (RFC 0031), has two architectures, and has booted twice in a VM and
   never on metal (`E0-P18`). So F's Platforms book is thin, and it is thin for
   a reason that is a deliberate design decision rather than an absence.
   Meanwhile the thing F has that Genode does not is the evidence apparatus —
   `proving-ground.html`, the claims registry, the simulator, the sweeps, the
   Kani proofs, `cargo xtask mutate` and `attest`. That is a whole book's worth
   of real, running material, and it is the part of this project most worth
   explaining to an outsider. Keeping the Genode set exactly is defensible;
   swapping Platforms for Evidence is more honest about what F is. A fourth book
   is not proposed — three is already more than this tree can fill.
2. **Does Applications get written at all before E3 closes?** The staging above
   says no. The counter-argument is that writing it early is how you find out the
   component-facing story has a hole in it, which is what a specification is for.
   If the answer is yes, it should be written as a *design document* under
   `docs/design/` and promoted to a book when it has a worked example — not
   written as a book with the examples left blank.
3. **One file per book, or a directory per book with a chapter per file?**
   Genode's are single PDFs of a few hundred pages each. F's would be far
   smaller. One file matches `docs/design/` and is what the existing claims
   walker already sees; a directory is better structure and costs the
   `documents()` widening named above. The walker should probably be widened
   regardless, since it is a latent hole whichever way this goes.
4. **Do the two loose pages — `the-long-plan.html` and `what-must-be-stated.html`
   — belong in a book, beside the design documents, or where they are?** They are
   currently in `docs/` with no stated relation to anything, which is the smaller
   version of the problem this intent is about.
5. **What is the update rule?** Genode revises all three books every release.
   F has no releases yet. The candidate rule is that a book is revised at a gate
   — G0, G1, G2 — and is otherwise allowed to lag, with the status block saying
   which gate it was last true at. An unstated rule here is how the books become
   the sixth thing that quietly stops being true.
