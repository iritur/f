# Releasing

A release of this project is **an evidence package**. The software is one of
its contents, and not the most important one.

That is not a stylistic choice. The deliverable here is defensible measured
claims — `docs/the-long-plan.html` section 08 — so the thing a release has to
carry is everything a stranger needs to disbelieve a number and then check.
Software somebody installs is what a product releases; this releases the
apparatus for disagreeing with it.

## The contract

> Every published number in a release can be re-derived by a stranger, from the
> package, with one command, on hardware they can buy. If that is not true of a
> number, the number is not in the release.

This is `claims/README.md`'s rule applied to the whole artefact instead of to a
single entry. It has one enforcement mechanism and it is deliberately blunt:
**a number that cannot be reproduced from the package is removed from the
package**, not annotated, not footnoted, not shipped with a caveat. A caveat is
a thing a reader has to notice.

## What is in the package

Eight things. The first is obvious and the other seven are the point.

| | Why it is in there |
|---|---|
| **The source, at a tag** | Obvious, and insufficient on its own — which is the argument for everything below it. |
| **The claims snapshot** | Every number the release asserts, with its baseline, its workload, its distribution and its reproduction command. Machine-readable, so documents render from it rather than restating it. `claims/snapshot.json`, written by `cargo xtask claims`, and `cargo xtask lint` fails when a document disagrees with it. |
| **The baseline configuration** | The tuned Linux a comparison was made against, as configuration rather than as prose. This is the one that decays silently: without it, a tuned comparison becomes a stock comparison as the baseline ages and nobody re-checks. `claims/baselines/`, one directory per version — `apply.sh` puts a machine into it, `verify.sh` says when it has drifted out of it, and re-tuning adds a directory rather than editing one. |
| **The seed corpus and scenario set** | So a third party can run the same sweeps — and so that a reported bug arrives as a seed rather than as a description of a bug. `sim/corpus.txt`: the header is the scenario set, regenerated from the table; every other line is an argument list for `f-sim` that found something once. `cargo xtask sweep` runs N seeds across M scenarios and `cargo xtask sweep --corpus` requires every entry to be clean. RFC 0040. |
| **A content-addressed system image** | One hash naming an entire bootable generation. Reproducing it from source and getting the same hash is a *test*, and it runs in the release job. |
| **The dependency manifest and provenance** | What went in, at what version, under what licence, with the imported subtree's terms kept distinct — because the licence boundary and the isolation boundary are deliberately the same boundary. `LICENSING.md`, RFC 0003. |
| **The honest-status page** | What does not work, what was measured on emulated hardware, what is a hook rather than a system. `docs/TESTING-STATUS.md` is that page and it ships with every release. |
| **The decision record** | Every RFC, **including the superseded ones**, so a reader can see what was reversed and why rather than only what survived. |

`cargo xtask release` builds it: one `.tar`, a `MANIFEST` naming every file and
its SHA-256, and one content address over the archive. Nothing in the package
carries a clock, a user name, a directory order or a compressor version, because
the thing being asserted is that two machines at one commit produce the same
bytes — `cargo xtask release --twice` is the local half of asking, and the
`package` and `address` jobs in the pull-request gate are the other: one line
per runner, `cargo xtask release --address`, compared by a third job.

Both runners have to check out at the same absolute path, and that constraint is
measured rather than argued. The image is a debug build, so it carries the path
it was built at in DWARF and in cargo's `-Cmetadata`; the same tree packaged at
`/work` and at `/elsewhere` produces two different addresses. A container job's
workspace is `/__w/<repo>/<repo>`, fixed by the runner rather than chosen, so
this holds without anything having to arrange it — and the comparison checks it
anyway, so that a difference in paths is never reported as a difference in the
package. `CARGO_TARGET_DIR` is not one of the paths that matters.

**All eight are owed, and the mechanism that let one of them not be is gone.**
Two of them once were not: the tuned-Linux baseline and the seed corpus were
owed to `E1` tasks that argued, correctly, that they could not be written yet, so
RFC 0021 made a content's requirement a predicate over the claims registry — *a
content is required when the claim that needs it publishes a number*. That was
the honest answer to a contract listing things that did not exist, and it was
never meant to outlive them. `E1-D06` landed `claims/baselines/`; `E1-P03` landed
`sim/corpus.txt`, and with the last conditional row closed, the `Requirement`
variant in `xtask` is deleted rather than left as a shape for the next deferral
to grow into — which is exactly what RFC 0021's *what would reverse this* asked
for, naming that task as the owner.

The corpus is one file carrying both halves the row names. Its entries are the
seed corpus: every trial a sweep has found something with, each one an argument
list `f-sim` runs, replayed by `cargo xtask sweep --corpus` and required to be
clean now — which is what makes it a regression suite rather than a list of
numbers. Its header is the scenario set, regenerated from the shipped table on
every write so that it cannot quietly stop matching. **Every entry in it today
was found under a deliberate defect**, `mutate-crossed-completion`, and each says
so on its own line; a corpus of seeds that are all green and do not say why would
read as a corpus that never found anything. RFC 0040.

The nightly job grows it. `cargo xtask sweep --record` merges every finding into
the file before it returns its verdict, and `.github/workflows/nightly.yml`
uploads the result as an artifact — a scheduled job cannot commit to the tree and
should not try, so what it hands a person is the exact file to merge. The file is
append-only, so the diff against the tree is what that night added and nothing
else. RFC 0042.

## What a release is not

- **Not calendar-driven.** A release happens when a gate closes. Dates on a
  research vehicle produce either padded gates or quiet ones, and both are ways
  of lying to yourself about where the project is.
- **Not a marketing moment.** The honest-status page ships inside the package
  rather than beside it, and the section describing what does not work gets the
  same care as the results.
- **Not a claim that the software is finished.** Release 0.1 boots a kernel and
  is otherwise mostly apparatus. That is what it says on the page.

## Two versions, which mean different things

**The release version tracks gates.** 0.1 is G0, 0.2 is G1, and so on to 1.0.
It says which claims have been defended, not how much software exists.

**The ABI version tracks the wire.** It has a floor and feature bits, it is
negotiated rather than matched (RFC 0011), and it moves entirely independently
of the release version. A component built against release 0.3 must keep working
at 0.4, or the component model is decorative.

Bumping one never implies bumping the other.

## How a stranger reproduces a number

The whole route, from nothing:

```bash
git clone <url> && cd f
git checkout <tag>
docker compose -f docker/compose.yaml build dev     # the only prerequisite is Docker
docker compose -f docker/compose.yaml run --rm dev cargo xtask verify
cargo xtask claims                                  # what this release asserts
cargo xtask claim <name>                            # re-derive one of them
```

Two things about that sequence are load-bearing.

**The environment is in the tree.** `docker/` is the development environment,
built from the toolchain pin the repository states, so there is no step that
lives in somebody's shell history. `docs/the-long-plan.html` section 09 states
this as a negative that anyone can falsify: *no step exists that is not in the
tree*.

**A timing number will refuse to record in that container, and that is
correct.** `F_ENVIRONMENT=container` and the harness refuses — see
`bench/src/lib.rs` and `docker/README.md`. Reproducing a *timing* claim needs
the runner class the claim itself names, which is stated in the claim's
`[hardware]` section. Reproducing everything else — the boot, the negative
suites, the litmus tests, the property tests — needs only Docker.

## Producing one

```bash
cargo xtask release --dry-run    # the manifest, and what is missing from it
cargo xtask release              # E0-R01; builds the package
```

`--dry-run` exists so that the gap between the contract and the tree is
readable at any moment rather than discovered on the day of a release. It lists
all eight contents, says which are present, and for each absent one names the
task that produces it. Today several are absent, and the command says so
plainly.

## What stops a release

Any of these, and none of them is overridable by deciding the release is
important:

1. `cargo xtask verify` is not green.
2. A gating claim is red, or a claim in the snapshot has no reproduction
   command that runs from a clean checkout.
3. A number appears in a document that `cargo xtask lint` cannot trace to the
   registry.
4. `docs/TESTING-STATUS.md` has not been re-read against the tree. It is the
   page that stops the plan from being mistaken for the state of the tree, and
   it is worthless the moment it is stale.
5. An RFC that was reversed during this cycle was edited rather than
   superseded.
