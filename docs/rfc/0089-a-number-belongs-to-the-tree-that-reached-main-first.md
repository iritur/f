# RFC 0089: A registry number belongs to the tree that reached main first

- Status: accepted
- Date: 2026-09-17
- Affects: `docs/rfc/` (two files renamed, two headings changed, and nothing
  else inside either), `docs/rfc/README.md` (the *Reserved numbers* section
  gains three rows and the paragraph that pre-committed to this), `CLAUDE.md`
  (*Common mistakes* gains the line that section's own rule had earned),
  `xtask/src/main.rs` (`lint_registries`, new), and every file that cited
  `RFC 0085` — `README.md`, `docs/book/evidence.html`,
  `docs/rfc/0082`, `docs/rfc/0083`, `intent/0012-the-interface/spec.md`
- Renumbers, without editing, RFC 0085 (*A book is a third kind of document*) to
  **0087** and RFC 0085 (*A decide task's exit may not require its
  implementation*) to **0088**

## Decision

When two trees take the same registry number and both reach `main`, **the number
belongs to the one whose merge into `main`'s first-parent history is earlier**,
and every later claimant is renumbered to the next free number in merge order.
The renumbering changes the file's name and its title line and nothing else: the
body, the date, the status and the reasoning stay exactly as they were accepted,
because what was decided did not change and only its address did.

The rule is *merge order* and not authorship date, not the order of the numbers
themselves, and not which entry has a row in `docs/rfc/README.md`. Merge order
is the only ordering every tree in the project agrees on without consulting any
other tree — an agent in a worktree can compute it with
`git log --first-parent origin/main --diff-filter=A -- <path>` and get the same
answer as anybody else, on any machine, at any time. Authorship dates are drawn
from three different clocks; the README table is explicitly not an index and
covers thirty-one of eighty-nine entries; and "lowest number wins" is not a
tie-break at all when the number is what collided.

And `cargo xtask lint-registries` refuses a fourth. It reads `docs/rfc/`,
`claims/` and `intent/`, and fails when two files in one of them share a number
or when a file's own title line disagrees with its name.

## Context

Three files in `docs/rfc/` were named `0085-*.md`, all `Status: accepted`, all
dated 2026-09-16, each reached through a different pull request:

| merged | PR | title |
|---|---|---|
| first | #54 | The first real display, and the three things it said |
| second | #56 | A book is a third kind of document |
| third | #59 | A decide task's exit may not require its implementation |

Every one of the three was cited elsewhere in the tree, and the citations meant
three different documents: `kernel/src/screen.rs` meant the display, `CLAUDE.md`
meant the book, `intent/0012-the-interface/spec.md` meant the decide-task rule.
The number had stopped identifying anything. Nothing was red — no lint in this
tree reads a registry for uniqueness — so the collision was found by a human
reading a directory listing, which is the detection method this RFC exists to
replace.

This is the *second* occurrence of the failure `docs/postmortem/0001` records.
The first was `E2-P07` and `E1-B15` taking one RFC number and one reserved byte
between four worktrees that were each green. `docs/rfc/README.md` closed that
incident with a paragraph that declined to add a line to `CLAUDE.md`'s *Common
mistakes* — on that section's own rule, *added when the same mistake happens
twice* — and set a condition in as many words: *If it happens a second time,
that is the twice the rule asks for and the line is earned.* This is that second
time, the line is earned, and this RFC is the paying of a debt somebody else
wrote down rather than a decision taken fresh.

Three alternatives were live.

**Leave them.** Three files can coexist in a directory and every one of them is
findable by its title. Refused because a citation is the point of a number: the
tree contains eight surviving `RFC 0085` references and contained fifteen, and a
reader chasing one had no way to know which document was meant short of reading
all three. A registry whose entries are not addressable is a filing convention.

**Renumber by content rather than by order** — give `0085` to whichever entry is
most cited, or most central. Refused because it is a judgement, and a judgement
is exactly what two agents in two worktrees cannot make identically. The value
of a tie-break is that it is mechanical.

**Reserve numbers up front**, so an agent claims `0090` before writing the body.
This is what `docs/rfc/README.md`'s *Reserved numbers* heading suggests, and it
does not work for the reason the first incident already established: a
reservation lives in a file, a file lives in a tree, and a tree is exactly what
a peer cannot see. It is the mechanism that failed, not a mechanism that was
missing. What makes it fail *loudly* is the lint, which is why the lint is here
and a stronger reservation protocol is not.

## Consequences

**Easy.** A citation identifies one document again. A collision is now found by
`cargo xtask lint` on the first commit that creates one, in the tree that
created it, rather than by somebody noticing a directory listing weeks later —
and the lint covers `claims/` and `intent/` too, which have the same shape and
have never been checked either. Resolving the next one is mechanical: run the
merge-order command, rename the later claimant, fix its title line, repoint its
citations.

**Hard.** Two accepted RFCs changed their names, so every external reference to
them — a commit message, a pull request comment, a branch that has not merged —
now points at a file that is not there. That cost is real and it is paid once;
it grows with every week the collision is left, which is the argument for doing
it now rather than after the next one. `git log --follow` traverses the rename,
and the three rows added to `docs/rfc/README.md` are there so that a reader
arriving from an old citation lands somewhere that explains itself.

**Foreclosed.** A number can no longer be quietly reused for a second meaning,
which was available until now and cost nothing to do by accident. Also foreclosed
is the *merge* strategy for a collision — two RFCs on one number combined into
one entry — which was never proposed and would have destroyed one of two
independently accepted decisions.

What this does **not** do is prevent the collision. Two agents will still take
`0090` next week, and both trees will still be green in isolation, because the
lint runs in a tree and a tree cannot see its peers. What changes is that the
merge goes red instead of the citation going ambiguous, and the merged tree is
where `docs/postmortem/0001` says the first execution of a program nobody has
run takes place.

## What would reverse this

**A collision the rule resolves wrongly.** Merge order is a proxy for *who had
the number first*, and it is a good proxy while pull requests merge in roughly
the order they are opened. If a long-lived branch that took a number early is
merged after a short branch that took the same number later, this rule renumbers
the wrong one — the entry that had been cited for a month keeps changing its
address while the newcomer keeps the number. If that happens, the tie-break
should move to the earlier *authored* commit and accept the clock-skew cost,
which is smaller than repeatedly renumbering the more-cited entry.

**A third registry-shaped directory the lint does not read.** `lint_registries`
names three, and a fourth added without a row would reproduce this incident
exactly. The check for that is the lint's own: it fails when a named directory
holds no numbered files, so a directory that is renamed or emptied goes red
rather than silently passing.

**The lint firing more than about twice a year.** That would mean numbers are
being taken concurrently often enough that a reservation protocol is worth its
cost after all — a `docs/rfc/NEXT` file taken by the first pull request to touch
it, with the merge conflict doing the work. It is not worth it at two incidents
in two epochs, and it would be at two a month.
