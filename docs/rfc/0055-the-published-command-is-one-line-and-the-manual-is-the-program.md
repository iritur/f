# RFC 0055: The published command is one line, and the manual is the program

- Status: accepted
- Date: 2026-09-04
- Affects: `README.md` (a new section, *Sweeping your own checkout*),
  `RELEASING.md` (the stranger's route gains the sweep), `docker/README.md` (one
  verb in the command list, and why it is only a pointer), `xtask`
  (`sweep --help`, the `unknown option` arm that now names it, and `sweep_dirty`
  beside `sweep_commit`), `sim/src/main.rs` (`main` answers `--help` on stdout;
  `usage`; `corpus` checks the header; `record` writes when the header changed;
  `Tree`, `--tree` and what a reproduction line is allowed to say),
  `sim/corpus.txt` (the header, one scenario short),
  `.github/workflows/nightly.yml` (one comment that had counted the scenarios by
  hand). Pays `E1-R01`. Extends RFC 0040 and RFC 0042 to the question they left
  open — *who reads this* — and reverses neither.

## Decision

**A stranger with Docker and a checkout runs one line and gets a seed sweep over
their own tree, and everything they then need to know is printed by the
programs rather than written in a document beside them.**

```bash
docker compose -f docker/compose.yaml run --rm dev cargo xtask sweep
```

Four things follow, and each is a place where the obvious alternative is worse.

- **The command lives in `README.md`, not in `docker/README.md`.** They are two
  different questions. `docker/README.md` answers *what is this environment and
  what is it not for* — it is where `F_ENVIRONMENT=container` is argued and
  where the timing refusal is explained — and it is read by somebody already
  working in the tree. `README.md` is read by somebody who has just cloned it,
  and it already carries the other half of this apparatus: *Reproducing a
  published number*, which invites a stranger to disbelieve a claim. A seed
  sweep is the same invitation pointed forwards rather than backwards, so it
  belongs in the same file and immediately after it. `docker/README.md` gets the
  verb in its command list and one paragraph saying why the argument is
  elsewhere, because a verb absent from the environment's own list reads as a
  verb the environment does not support.
- **The scenario set, the meaning of a seed, the anatomy of a finding and what
  to do with one are help text.** `cargo xtask sweep --help` and
  `cargo run -q -p f-sim -- --help` carry them, and the scenario table and the
  five properties are rendered from `SCENARIOS` and `CHECKS` rather than
  restated. A document is a second account of a table, and the second account is
  the one that goes stale — this tree already has the scar in
  `sim/corpus.txt`, whose header is regenerated from the shipped table on every
  write for exactly that reason.
- **`cargo xtask sweep --help` answers rather than complains.** It used to reach
  the `unknown option for sweep` arm, which is the one moment a stranger asks
  the tool what it is and the tool answers with a complaint about an argument.
- **A report states the tree it was produced in, and its reproduction lines are
  shaped by it.** `--commit <sha>` was the whole of what a report said about
  where it came from, and a commit is not a tree: `git rev-parse HEAD` names
  what is committed and says nothing about what has been changed since. So on
  any modified checkout the printed `repro` line — `git switch --detach <sha> &&
  cargo run …` — told a reader to discard the changes that produced the finding
  and then run a different program. `cargo xtask sweep` now asks git both
  questions and passes the second as `--tree clean|dirty`; `f-sim` prints it on
  a `tree` line in every report and emits the `git switch` half only when it was
  told `clean`. See *The tree is not the commit*, below.

## Context

`E1-R01`'s exit is *a third party runs a seed sweep against their own checkout
using the published command*. Nothing was missing from the machinery: RFC 0040
built the sweep, the oracle and the corpus; RFC 0042 bounded it, sharded it and
made a reproduction line judge itself; `RELEASING.md` already ships the corpus
and the scenario set as one of eight contents and `cargo xtask release
--dry-run` already reports eight of eight. What was missing was the sentence
that says *type this*, and a way for a stranger to find out what came back.

Three shapes were live.

**A `docs/simulator.md`.** The most obvious, and it is what most projects would
do. It would be the fourth place the scenario table appears — after
`SCENARIOS`, after `f-sim --help` and after `sim/corpus.txt`'s regenerated
header — and the only one of the four with nothing keeping it in line. RFC 0040
already refused a corpus format that a program does not parse, on the same
grounds: *an entry this binary cannot run is an entry that fails to load*. A
page a program does not render is a page that can be wrong without failing.

**A wrapper script — `./sweep.sh`.** One line for the stranger, and it hides
which command actually ran, so a bug report names the wrapper instead of the
verb. `docker/README.md`'s own rule is the argument against it: *a workflow
somebody has to remember to dispatch is institutional knowledge with a YAML file
in front of it*. `docker/dev.ps1 x sweep` already exists for Windows and passes
its arguments through, which is the wrapper this tree is willing to have.

**Nothing, on the grounds that `cargo xtask sweep` is already in `CLAUDE.md`.**
It is not: `CLAUDE.md` is one page for an agent working in the tree, it is not
what a stranger reads, and it is explicitly forbidden from growing into a
document. A command nobody outside the project can find is not published.

The cost was measured before anything was written about it, and it is recorded
here rather than in `README.md`. From a clone with no `target/`,
no build cache and the image already present, the published command took
**3 min 16 s** of wall clock at `7f3c3c0`, and **4 min 44 s** on a second run
from a second empty volume set with more of the machine busy; the sweep itself
was **1.8 s** and **0.9 s** respectively, and everything else was the cold build
of the workspace and the three component images. A warm run of the same command
took **16.6 s**. Building the `dev` image from nothing, `--no-cache`, on the
same laptop, took **20 min 38 s**.

Those are costs and not claims — a container refuses to record a timing, which
is `bench/src/lib.rs`'s job and correct — and the two cold runs differing by a
third of themselves is the argument for `README.md` stating a shape rather than
a figure. A command whose first run costs an unknown amount is one a stranger
abandons half-way; a command whose cost is published to the second invites an
argument about somebody else's laptop, and has nothing that can go red when it
stops being true. R12: a concession is written as a cost. These four are dated,
attached to `7f3c3c0`, and in an RFC, which is where a number nobody can
re-derive belongs.

## The tree is not the commit

This was found in review of this change, and it is the more interesting of the
three defects here because nothing was broken: every part worked, and together
they published a sentence that was false on most of the machines it was
published for.

`E1-P03` built the sweep for somebody hunting a bug in work they are doing.
That person's checkout is, by definition, modified — that is why they are
sweeping it. The report they get says *paste this line*; the line says *check
out this commit first*; and following it deletes the work under test. The
person most likely to find something is the person the line is most wrong for.

Three shapes were considered and two rejected.

**Refuse a sweep on a modified tree.** It is the strictest reading of R04 and it
is the wrong end of it: it would break the published command on exactly the
trees it was published for, and this RFC's own subject is that the command
works from a clone somebody has since edited. A refusal that stops the useful
case to protect the reporting case protects nothing, because nobody would run
it.

**Say nothing and keep the line.** What was already there. The report is
correct on a clean checkout and silently misleading everywhere else, which is
the shape RFC 0017 and `--mutate` exist to make impossible elsewhere in this
tree: a green thing that cannot go red.

**State it, and let it decide the line.** Taken. Three states rather than a
`bool`, because *nobody said* is not *it was clean* and a two-valued flag has
to pick one of those to be its default; `f-sim` runs no subprocess and reads no
repository, so it cannot find out and must be told. A caller that says nothing
gets the state that claims least. `release` already names its package `-dirty`
and `reproduce` already prints *(dirty — not a quotable tree)*; both read the
same two words out of `git status --porcelain`, so this is the third caller of a
mechanism the tree already had and not a new idea.

The consequence worth naming is that the mutation harness could no longer
assert a constant. `cargo xtask sweep --mutate` required the string
`repro      git switch --detach` in the report, which on a developer's tree is
now the wrong shape — so it asks `sweep_dirty` the same question and requires
the shape it asked for. That is not a weakened assertion: it is the same
assertion made about a report whose form is now a function of something, and a
harness that had kept the constant would have gone red on every tree anybody
works in.

## Consequences

**Easy.** A bug arrives as an argument list. That is the whole shape `E1-P03`
was built for and it now has an audience: the help text says *report the
argument list, not the symptom*, and the line a report prints already carries
its own commit and exits non-zero on its own.

**Easy.** The three entry points cannot drift from the tree, because two of them
are rendered from the tables the sweep runs and the third is a command line that
`cargo xtask sweep --help` and `README.md` state identically.

**Hard.** `README.md` grew a long section, and this file's own argument is that
documents rot. The mitigation is that almost nothing in it is a fact about the
simulator — it is one command, the shape of a cost, and pointers at three
`--help` outputs. The facts stayed in the programs.

**And the section states no number.** It did, in the first draft: four
wall-clock figures, marked as costs and not claims and with the container's
timing refusal named beside them. Review was right that this is not enough. A
number on the front page with nothing that can go red behind it is exactly what
`claims/` exists to refuse, `lint_claims` cannot see a figure no document cites
as a claim, and the two cold runs behind *three to five minutes* differed by a
third of themselves. So the paragraph says the shape instead — the image
dominates and is paid once, the first run is minutes, a warm run is seconds —
which is what a reader needs in order not to abandon it half way, and which
cannot be quietly wrong on somebody else's laptop next year. The measured
figures stay here, in *Context*, where they are dated and attached to a commit.

**Foreclosed.** A future `docs/simulator.md`. If one is wanted, this decision has
to be reversed rather than worked around, because two accounts is exactly the
state this refuses.

**Three defects fell out of publishing this, and each was in the half nobody
reads.**

`cargo run -q -p f-sim -- --help` **exited 1 and printed to stderr behind an
`f-sim:` prefix**, because `parse` returns the usage screen as an error. That is
invisible while the only reader is somebody already working in the tree, and
wrong the moment the text is the published answer to *what is a scenario*: a
stranger pipes it into a pager and gets nothing. `main` answers help before
`parse` now, on stdout, exit zero. `parse`'s arm stays, because `parse` also
reads corpus entries and there a help request really is an entry the binary
cannot run.

It answers only when help is the **whole** command line, which the first draft
got wrong: it scanned the whole argument list, so `f-sim --check --seed 0x1 blk
-h` printed a usage screen and exited zero, having run no trial and reported
the status a clean run reports. That is a real fail-open and R04 covers it —
the argument list is not a help request, it is a real invocation carrying an
argument this binary does not accept. It has a second victim: `xtask`'s
`f_sim` tells a refusal from a verdict by whether anything reached standard
output, so a usage screen printed there reads to it as a sweep that found
nothing. Every other argument list falls through to `parse`, which refuses.

**`sim/corpus.txt`'s header had quietly stopped matching the table**, which is
the exact failure the header exists to make impossible. Its own comment says the
scenario set is *regenerated from the shipped table on every write so that it
cannot quietly stop matching* — and `record` only wrote when a sweep had **found
something**. So a scenario added to a tree with nothing wrong with it drifts the
header and nothing says so, for as long as nobody finds a bug. `deadline` had
been missing from the shipped set since it landed. Two changes: `record` writes
whenever the bytes differ, and `corpus` refuses when the header on disk is not
the header this binary generates — so the property is checked on the command the
nightly already runs, rather than believed. A regenerated header is one line of
diff, which is what it should have been all along.

That is the general shape of what this RFC is for. A guarantee that holds *on
write* is not a guarantee, it is a habit, and the place it fails is always the
artefact nobody reads until a stranger does.

**Found while checking, and it belongs to `E1-R02` rather than here.** The
release package is not sufficient on its own for this. `cargo xtask release`
builds it, `source.tar` inside it carries the whole tree — `docker/`, `xtask/`,
`sim/`, `sim/corpus.txt` — and unpacking it and running the published command
gets as far as building everything and then refuses:

```
xtask: cannot read the commit from git: git rev-parse HEAD failed
```

`sweep_commit` reads the commit from git and fails closed when it cannot,
which is right — *a seed without a commit reproduces nothing at all* — and
`git archive` does not produce a repository. The commit is in the package: the
`MANIFEST` states it on its second line. So the gap is a small one and it is
exactly `E1-R02`'s exit, *a third party runs a seed sweep and the four datapath
claims from the package alone*, which is why the fix is not taken here: the
choices are whether `sweep` learns `--commit`, whether the packager writes
something a tree can identify itself from, or whether the package ships a
repository — and picking one of those is that task's decision, not this one's.

**Not closed.** The exit says *a third party*, and no third party has run it.
What is checkable from here is that a clean clone with nothing but what git
carries runs the published command and gets a sweep, and that is what was
checked. `E0-R02` has the identical honest gap for the reproduction route and
records it the same way; this is not a different kind of claim and should not
be written as one.

## What would reverse this

- **Somebody clones this, runs the published command and cannot tell what came
  back.** That is the observation this is built against, and the help text is
  what would be wrong.
- **The first-run cost stops being minutes.** The section states a shape so
  that it can become false. If a from-nothing first run reaches an hour — a
  heavier image, a larger sysroot, a scenario table that must be built before a
  sweep — then one command is the wrong shape and the right one is a published
  image somebody pulls, which is a different decision with a different failure
  mode (a tag nobody rebuilds).
- **Somebody sends a report from a dirty tree and it is acted on as if it were
  a bug report.** The `tree` line and the missing `git switch` are the whole of
  what stops that, and both are text a reader can skip. If it happens, the next
  step is `--record` refusing to write a corpus entry from a tree it was told is
  dirty, and after that `sweep` refusing outright with a `--anyway` — which is
  the refusal rejected here, earned rather than assumed.
- **`--tree` starts being passed wrongly, or a caller learns to lie to it.** It
  is a claim a caller makes and nothing verifies: `f-sim` cannot check it
  without reading a repository, which would end its being a function of its
  arguments. `cargo xtask sweep` is the only caller that states it, and if a
  second one appears that states it from something other than
  `git status --porcelain`, the flag has become decoration and belongs inside
  `f-sim` behind an explicit *this binary may read the tree* — a much larger
  decision, and RFC 0004's.
- **The header check starts refusing for a reason that is not drift.** A corpus
  file somebody is legitimately holding at an older header — a bisect, a
  bug report from a release back — is now refused by `--corpus` before a single
  entry is replayed. If that turns out to be a thing people actually do, the
  check belongs in a lint over the tree rather than in the replay path, and the
  replay should warn instead of refusing.
- **A third account of the scenario set appears and is useful.** If the tables
  turn out to need prose that a `--help` cannot carry — a tutorial, worked
  findings, pictures — then a page in `docs/` earns its place, and the rule
  becomes *the page may not restate a table* rather than *there is no page*.
- **`cargo xtask sweep` stops being the thing a third party should run.** If the
  released artefact becomes the package rather than the checkout — `E1-R02`'s
  exit is *from the package alone* — then the published command is one that
  unpacks a `.tar` and this section names the wrong entry point.
