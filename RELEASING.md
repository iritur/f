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

## Release 0.2, and the four numbers that are not in it

0.2 is G1's release, and G1 is the datapath: *a driver is killed under
sustained load and no client observes anything but latency*, and the apparatus
half — *a bug injected anywhere is found by an overnight seed sweep and arrives
as a reproduction command*. The apparatus half is what 0.2 is. **The four
datapath claims are not in it**, and this section is why, in the place a reader
will look rather than in a task list they will not.

The four are `E1-P10`: ring submit under load, doorbells per operation, copies
per operation, kernel entries per operation. Two independent things stop them,
and either one alone would be enough.

**There is no workload.** `E1-P10` needs `E1-B09`, the user-interrupt doorbell.
`E1-B09` needs `E0-B15`, which built that path as far as *refusing to
construct*: `Bell::new` declines on a machine that does not report the hardware,
and that is every machine this project can reach. QEMU's TCG backend implements
no part of Intel's UINTR and no `-cpu` model advertises the bit. So the boot
reports a doorbell count over the two operations a self-test performs, which is
deliberately not registered as a claim, because a count over two operations is
not *doorbells per operation under load*.

**There is no machine.** All four are times. `bench/src/lib.rs` refuses to
record a timing where `F_ENVIRONMENT=container`, and that refusal is the harness
working rather than failing — a number with no environment attached is how a
benchmark becomes marketing. `E0-D10` owns obtaining the class-A machine that is
allowed to record one; it has not been obtained. Every timing claim in the
registry is `pending` for that single reason, not for four different ones.

So **the four datapath claims cannot be produced on any machine this project
currently has**, and 0.2 goes out without them or does not go out. The contract
at the top of this file already decides which: *a number that cannot be
reproduced from the package is removed from the package*. Four numbers that
cannot be produced at all were never in it. What this section adds is that the
absence is **named** — a manifest lists what is present, and absence is the one
thing it cannot state.

What would have to become true. All three, and the first two in either order:

1. A machine that advertises the doorbell hardware, so `E1-B09` has a path that
   can execute rather than one that correctly refuses to construct.
2. A class-A machine as `claims/runner-class-A.md` specifies it, so `E0-D10`
   closes and `f_bench::Environment` permits a recording at all.
3. `E1-P10` then registers four entries with baselines and thresholds, and this
   section is deleted rather than amended — an absence that has stopped being
   one is not a paragraph worth keeping.

Neither of the first two substitutes for the other. Hardware without a class-A
machine produces a number nothing may record; a class-A machine without the
doorbell produces a measurement of the fallback path wearing the fast path's
name, which is worse than no number because it would look like one.

**This section does not rely on being re-read.** Both reasons are declared in
`xtask` as `DATAPATH_GAP`, one row each: the refusal in `Bell::new` that keeps
the doorbell path from executing, and `WHY_CONTAINER` in `bench/src/lib.rs` that
keeps a timing from being recorded. `cargo xtask lint` requires both to still be
there, and the day either goes the build goes red and names this section as one
of the documents that has stopped being true. An absence written only in prose
is an absence nobody checks — and the failure is not that it goes unfixed, it is
that it *is* fixed while four documents go on describing it. Same mechanism as
`OWED_REVERSALS` and `CHAOS_GAP`, RFC 0036's precedent, applied to a number that
does not exist rather than to a deviation that does.

**What 0.2 does contain** is the eight contents above, and the claims it carries
are stated by the package rather than by this paragraph. `MANIFEST` opens with a
block derived from `claims/` at the moment the package was built: a tally, then
one line per claim naming its status. `claims/snapshot.json` is the same
statement machine-readably, `cargo xtask claims` prints it from a checkout, and
`claims/*.toml` inside `source.tar` carries each one's baseline, workload and
one-command reproduction. Counts are not repeated here, because a count in prose
beside a generated file is the copy that goes stale. RFC 0056.


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

The seed corpus is used the same way, and it is one command rather than three
because there is nothing to fetch — the corpus and the scenario set are in the
tree the clone already produced:

```bash
docker compose -f docker/compose.yaml run --rm dev cargo xtask sweep
docker compose -f docker/compose.yaml run --rm dev cargo xtask sweep --help
```

`README.md`, *Sweeping your own checkout*, is the whole route and the cost.
What a scenario is, what a seed is and what to do with a finding are printed
by `cargo xtask sweep --help` and `cargo run -q -p f-sim -- --help` rather than
written here, because a document drifts from a program and a `--help` cannot.
RFC 0055.

Two things about the first sequence are load-bearing.

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

### From the package alone

The route above starts at a `git clone`. A release package has no repository in
it — `git archive` writes files and no `.git`, which is exactly the property
that lets the source be content-addressed — so the two things it carries have to
be unpacked into **one** directory:

```bash
mkdir f-0.2 && tar -xf f-<version>.tar -C f-0.2
cd f-0.2 && tar -xf source.tar          # MANIFEST now sits at the tree root
docker compose -f docker/compose.yaml run --rm dev cargo xtask sweep
docker compose -f docker/compose.yaml run --rm dev cargo xtask claim <name>
```

One directory rather than two, and that is the whole of the mechanism. A sweep
prints `(seed, commit)` pairs and refuses to run without the commit, because a
seed without one reproduces nothing; in a checkout git answers, and in an
unpacked package `MANIFEST` does — it is a member of the package tar and not of
`source.tar`, so unpacking them apart leaves the sweep with nothing to read and
it says so.

**Whether a particular package can do that is stated by the package**, on the
`sweep` line of its `MANIFEST`, and that line is the thing to believe rather
than this section. `sweep   from MANIFEST` means the four commands above work;
`sweep   needs a repository` means the sweep will stop with `cannot read the
commit from git` and that the package predates the fallback. The reason it is
checked rather than asserted is that the fallback lives in `xtask/src/main.rs`
and a stranger reaches that file only through `source.tar`, which is `git
archive` **of the commit** — so a tree where the fallback is written and not yet
committed produces a package that cannot do what the tree can, and every
document describing it is right about the tree and wrong about the artefact.
That is not hypothetical; it is how this section was first published. The
packager now reads the source it is about to ship and writes the line from what
it found, and `cargo xtask release --dry-run` prints it before anything is
built.

The package's own copies of the eight contents sit at the same relative paths as
the source's and should overlay it byte for byte at a clean tag — a property
worth checking rather than assuming, because there are two
ways for it to be false and only one of them is obvious. The obvious one is
that the tree was packaged dirty, and the manifest's `version` says `-dirty`.
The other is that the checkout disobeys `.gitattributes`: the package reads the
**working tree** and `source.tar` comes from the **tree object**, so a file
saved with CRLF on a Windows checkout is in the package with CRLF and in
`source.tar` without. That second one moves the address at a fixed commit,
which is the one thing an address exists not to do — so a release is packaged
from a checkout whose line endings are what the repository says they are, and
`git ls-files --eol` naming no `w/crlf` file is how that is checked.

That command and not `git add --renormalize .`, which was the check written here
first and does not work. Renormalising converts a working-tree file through the
attributes and compares the result against the blob — and the blob is already
LF, because these files were committed correctly and *re-saved* with CRLF
afterwards. So renormalising produces no diff for exactly the files that are
wrong, and `git status` calls them unmodified for the same reason: both compare
after normalising and the package does not. Measured on this checkout, which has
28 such files, three of them under `docs/rfc/` and therefore inside the package:
`git status` is silent about all 28, renormalising names none of them, and `git
ls-files --eol` names all 28. A check that agrees with the thing it is checking
is not a check.

The sweep will call the tree **dirty** and print no `git switch` line in front
of a finding. That is not a defect: an unpacked package is that commit by
construction and nothing inside it can check that it still is, so the report
claims the least it can. RFC 0056.

A claim reproduces the same way, and what comes back depends on what the claim
is. The counts — the ones `MANIFEST` lists as `gating` — come from a
deterministic simulator or from one boot, and they were **run** from a tree with
no repository rather than assumed to survive one: every gating claim printed its
number against its own threshold and exited 0. That was worth running instead of
asserting, because the sweep standing beside them refuses without a commit and
had to be taught where to find one, and nothing except the run said the claims
did not have the same dependency. The times refuse to record and say why —
`cargo xtask claim unmap-churn` does both in one command, gating on the count
and declining the percentile, and it names the claim that owns each. That split
is the registry working, and no count is repeated here: the command prints them
and `claims/` holds them.


## Producing one

```bash
cargo xtask release --dry-run    # the manifest, and what is missing from it
cargo xtask release              # E0-R01; builds the package
```

`--dry-run` exists so that the gap between the contract and the tree is
readable at any moment rather than discovered on the day of a release. It lists
all eight contents, says which are present, and for each absent one names the
task that produces it. It read *several are absent* for the whole of E0; every
row is present now, and the command is still the thing to believe about that
rather than this sentence.

### Finishing one, which an agent does not do

Everything above can be prepared by anyone, including an agent: the package is
built, the address is printed, the absences are written down. What makes a
release *exist* is a pushed tag, and that is deliberately on the other side of a
gate an agent cannot open. `.claude/hooks/release-gate.sh` blocks `git tag -a`,
`git push --tags`, `docker push` and `cargo publish` unless
`F_RELEASE_AUTHORIZATION` names a person and a date, and it is not a flag or a
file — a person exports it in the shell where the release is made, which is the
same person who will be asked about it afterwards. A gate the gated party can
open is a log entry.

So this sequence belongs to a human, in this order, and the first two are
checks rather than ceremony:

```bash
cargo xtask verify                                  # gate 1
git ls-files --eol | grep w/crlf                    # must print nothing: see above
cargo xtask release --dry-run                       # eight of eight, `sweep from MANIFEST`, the claims block
export F_RELEASE_AUTHORIZATION="release 0.2.0, approved by <name>, <date>"
git tag -a v0.2.0 -m "release 0.2" && git push origin v0.2.0
cargo xtask release                                 # at the tag: no -dirty, and the address
```

A dry run that says `sweep needs a repository` stops this sequence before the
export. It means the source at `HEAD` cannot read a commit out of `MANIFEST`, so
the package would ship a route this document publishes and it does not have —
and the fix is a commit, not a decision.

The address is only meaningful in that order. `cargo xtask release` on a tree
with modifications names its file `-dirty` and produces an address that moves
with every file in the eight contents, which is the mechanism working rather
than a problem — but it is not the number to publish. Publish the one from a
clean tree at the tag, and `cargo xtask release --address` on a second machine
is how somebody else confirms it.

Gate 4 has no line in that block because it has no command: somebody re-reads
`docs/TESTING-STATUS.md` against the tree and says it is still true. It is the
one stopping condition a machine cannot check, which is exactly why it is the
one that goes stale — a page that ships in every package and describes a tree
three milestones ago is worse than no page, because it was written to be
believed.

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
