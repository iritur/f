# RFC 0021: A release content is owed when the claim that needs it is

- Status: accepted
- Date: 2026-08-31
- Affects: `RELEASING.md`, `xtask/src/main.rs`, `E0-R01`, `E0-R04`, and `E1-D06`, which is the task this defers to

## Decision

`RELEASING.md` lists eight things a release package contains. Two of them —
the tuned-Linux baseline configuration and the seed corpus — are read as
**required when the claim that would use them publishes a number, and not
before**. The packager reads the claims registry to decide, so what makes a
content required is a status change in `claims/`, not an edit here.

Concretely: `claims/0001-ring-submit-latency.toml` is `pending`, and its
thresholds include `ratio_vs_baseline = { min = 5.0 }`. While it publishes no
ratio, there is nothing for a baseline to be the baseline *of*. The moment it
leaves `pending`, `cargo xtask release` refuses and names `E1-D06`.

## Context

`E0-R01` had to produce a package, and two of its eight contents are owed to
`E1` tasks whose own exits argue — correctly — that they cannot be written yet.
`E1-D06` says a tuned baseline "written before there is a workload to tune it
against is a guess with a filename". The seed corpus is `E1-P01`'s and `E1-P03`'s
and needs a simulator that does not exist.

That left two bad options and this third one.

**Ship the package with the contents missing and a note.** Rejected: it makes
the contract advisory, and `RELEASING.md` exists precisely because the contents
are the thing people skip. A contract that is satisfied by a note is prose.

**Shorten the list to what E0 can produce.** Rejected, and this is the one that
would actually have happened. `A-07` is the standing item against exactly this
— *no silent scope cuts* — and `E0-D08` caught one already. A list quietly
shortened to fit the work is how a release contract becomes a description of
whatever was convenient.

The third option converts the cut into a **gate with a known trigger**. Nothing
is dropped, nothing is shipped missing, and the day the deferral stops being
defensible is the day a command goes red and says so.

What was true when this was decided: `E0-P05` is the task that makes 0001
gating, and `E0-R04` — release 0.1 — needs `E0-P05`. So the trigger is not
hypothetical and it is not far away. The sequencing consequence is worth stating
plainly rather than discovering: **release 0.1 must either pull `E1-D06`
forward or not publish `ratio_vs_baseline`.** This RFC does not decide which;
it makes the decision arrive as a red command in front of whoever is doing
`E0-P05`, instead of as a reader noticing afterwards.

## Consequences

The requirement is an enum with named variants and not a boolean or a
configuration field, because the failure mode to design against is somebody
later declaring a content not required in order to make a release go out. Every
deferral names the claim and the condition that flips it, so a reviewer reading
the table sees the trigger beside the exemption. Changing the trigger means
editing the claim's status, which is a diff a reviewer already reads carefully.

It also means `cargo xtask release` is a check on the registry and not only on
the filesystem, so a release cannot be built from a tree whose claims say more
than its package contains.

The package itself gains a `MANIFEST` naming every file and its SHA-256, and one
content address over the archive. SHA-256 and the archive writer are both in
`xtask/src/pack.rs` rather than dependencies, for the reason the release
contract states: no step exists that is not in the tree. Before this the hashes
came from shelling out to `sha256sum`, so the content address of a release
depended on which coreutils the machine had, and was silently *absent* on a
machine without it.

## What would reverse this

A second content acquiring a deferral for a reason that is not "the claim it
serves publishes nothing". At that point the predicate has become the general
mechanism for postponement it was written not to be, and the right answer is to
change `RELEASING.md`'s list in a reviewable diff — which is what should have
happened in the first place, if the contents were genuinely not part of a
release.

Also: `E1-D06` landing. The exemption then never fires again, and the variant it
uses should be deleted rather than left as a shape for the next deferral to
grow into.
