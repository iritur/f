# RFC 0040: A sweep needs an oracle that does not name the bug

- Status: accepted, amended by RFC 0042
- Date: 2026-09-03
- Affects: `sim/` (new: `check.rs` — the oracle; `sweep.rs` — the grid, the
  minimiser and the corpus's shape; `dev.rs` — `crossed`, one deliberate defect
  behind a feature; `main.rs` — `--sweep`, `--check`, `--corpus`, `--record` and
  the replay flags), `xtask` (`sweep`, `sweep_mutate`, `sweep_corpus`,
  `sweep_record`, `DEFECTS`, and the release manifest's seed-corpus row),
  `sim/corpus.txt` (new), `.github/workflows/nightly.yml` (new), `RELEASING.md`.
  Extends RFC 0017 to the layer above the frame, closes the deferral RFC 0021
  opened, and reverses nothing.

## Decision

**A seed sweep is checked by properties that name no scenario, no fault class
and no defect**, and a failure's identity is the first of those properties to
fail. Four things follow, and each is the part somebody would otherwise do
differently.

- **The oracle is separate from the fault classes.** RFC 0039 built seven
  classes, each with a *stated response* asserted by a test written for it. That
  is right for a class and wrong for a sweep: a sweep's premise is that the bug
  is not known in advance, so an injected bug that had to be named by an
  assertion written for it would be found by the assertion rather than by the
  sweep. `sim/src/check.rs` holds five properties expressed over the trace's own
  vocabulary, and they were written before the defect below existed.
- **A finding's signature is the first check that fails, and minimisation
  preserves the signature rather than merely preserving a failure.** `fault.rs`
  states that neither knob of an `Injection` is a subset operation and names the
  exact accident that follows — a minimiser that still saw *a* failure and
  concluded the earlier strikes were not required. Requiring the same check is
  the guard, and it is a guard rather than a proof.
- **The sweep decides what is smallest, and says in what terms.** `Size` is a
  lexicographic tuple — armed classes, strikes actually made, operations,
  clients, window, how late injection begins — and the report leads with the
  smallest reproduction across the whole sweep. That is *no human triage* made
  mechanical: read finding 1.
- **A deliberate defect lives in `sim/src/dev.rs` behind a feature that is off by
  default**, `cargo xtask lint-mutations` refuses to let it become a default, and
  `cargo xtask sweep --mutate` requires the sweep *and the corpus* to go red with
  it and green without. RFC 0017 made this argument for the kernel; this is the
  same argument one layer up.

The seed corpus is `sim/corpus.txt`, and **a corpus line is an argv**. There is
no format beyond that: `f-sim`'s own command-line parser reads an entry, so an
entry the binary cannot run is an entry that fails to load, and a stranger
reproduces one by pasting it after `cargo run -q -p f-sim -- --trace`.

## Context

E1-P03's exit is *an injected bug is found by the sweep and reported as `(seed,
commit)` plus a one-line repro, with no human triage*. Everything needed to run
a million trials already existed — virtual time, a seeded derivation with
independent streams (RFC 0026), device models on the real ring types (RFC 0034),
a hashed artefact, seven fault classes (RFC 0039). What did not exist was an
answer to *how does a run know it went wrong*, and that turns out to be the whole
of the task.

Four alternatives were live for the oracle, and each looks better than it is.

**Compare against a golden trace.** Cheap, and it is what a digest already does.
It answers *did this run change* and never *is this run correct*, so it goes red
on every intentional change and says nothing about a defect that has been there
since the model was written. It is `cargo xtask sim`'s job and it is already
done.

**Assert per scenario.** More precise, and it is what `fault.rs` does. It does
not scale to a sweep in the only direction that matters: eighteen scenarios times
one assertion each is eighteen bugs that can be found, and the bug a sweep is for
is the nineteenth.

**Reuse the fault-class assertions as the oracle.** Tempting, because they exist
and they are strong. They are also written *for* the class they assert about —
`peergone` asserts that buffers come home — so a sweep built on them would find
exactly the seven things somebody already thought of. The seven stay where they
are; the oracle is new and deliberately weaker per property and broader in reach.

**A crash oracle: call anything that panics a bug.** This is what most fuzzers
do and it is nearly free. It also finds nothing here, because the models are safe
Rust in a crate that forbids `unsafe`, and the failure mode a component system
has is not a crash — it is a buffer nobody can take back and a client that never
finishes. `check::bound` is the property that catches that, and a crash oracle
would have watched it happen in silence.

Two smaller decisions were live and are recorded because they cost something.

**Where the parallelism and the clock go.** A sweep is the one place in this tree
where wall-clock time and threads are legitimately wanted, and the risk is that
either reaches a verdict. The split is: `f-sim` lays out the grid, runs it at
whatever worker count it is given, and assembles the report in grid order — its
output is a function of its arguments, and a test runs one sweep at one worker
and at five and requires one report. `xtask` supplies the commit and the
component directory, and times the whole thing. The clock could not have been put
in `sim/` even if somebody wanted to: `cargo xtask lint-determinism` scans that
tree with no allow-list entry, so the policy made the decision before the author
did.

**Whether the sweep is a process per trial.** `sim/src/main.rs` argues at length
that the reproduction check must be two *processes*, because two calls in one
process share an allocator and can agree with themselves for reasons that have
nothing to do with the seed. A sweep is not a reproduction check — its job is to
find a failing pair and hand out a command a stranger runs as its own process —
and a process per trial would cost tens of milliseconds against a run that takes
a fifth of a millisecond. Measured on the four-core development container: 1.18
million trials in 268 s in process; the same grid at one `cargo run` each would
take on the order of a day.

## Consequences

**Easy: a sweep is falsifiable.** `cargo xtask sweep --mutate` arms
`mutate-crossed-completion` — the block, network and display devices answer a
coalesced pair's third completion with the first entry's token — and requires the
sweep to find it, minimise it and print a reproduction; then requires the corpus
to go red on it; then requires both to go green without it. Seventeen seconds on
the development container. Without that harness, every green sweep this tree ever
prints would be indistinguishable from a sweep that cannot fail.

**Easy: a failing seed arrives as a command.** The report prints, per finding,
the line that reproduces the whole scenario and the line that reproduces the
minimised trial, each carrying the commit. Nothing has to be reconstructed from a
log.

**Easy: the release contract is whole again.** The seed corpus was the last
content carrying RFC 0021's conditional requirement, and that RFC's *what would
reverse this* named E1-P03 as the owner of deleting the mechanism. The
`Requirement` enum is gone, the eight contents are unconditional, and
`RELEASING.md` is true without a footnote.

**Hard, and stated: there is now a second deliberate defect in shipped source.**
A reader of `Device::reap` sees two versions of one line. RFC 0017 made that
trade once for `kernel/src/cap.rs` and said the pattern would extend; this is the
extension, and it is worth being explicit that the count is now two.

**Hard: two distinct bugs that trip one check in one scenario are reported as
one finding.** That is the price of a signature stable enough to survive
shrinking. What limits it is that findings are kept per `(scenario, check)` and
the corpus keeps a trial per finding, so two bugs that minimise to two different
trials still leave two entries behind.

**Foreclosed: the minimiser cannot prove that what it removed was irrelevant.**
It promises what delta debugging promises — *a smaller trial that fails the same
check* — and the report says so in those words rather than using the word
*minimal* unqualified. A candidate that trips the same check for a different
reason would be accepted, and this module would call it the same bug.

**Stated because the harness's green result would otherwise imply more than it
should: `mutate-crossed-completion` does not need a sweep to be found.** At the
tree's own default seed it fires in twelve of the eighteen scenarios, so a single
run of most of them would catch it. Where the sweep's N earns its keep is the
other end of the distribution — `blkloss` fires at five seeds in sixty-four and
`peergone` at four in thirty-two — and where it earns it unambiguously is the
*minimum*: the smallest reproduction the sweep reports comes from seed 1 rather
than seed 0, and finding it is what sixty-three extra trials bought. The honest
claim is therefore *the sweep finds it, minimises it and reports it as a
command*, and not *only a sweep could have found it*. A defect that needed a
thousand seeds would demonstrate more and would also make the harness a minute
long inside `verify`; the one chosen is the compromise, and the number that would
change the answer is written into `MUTATE_SEEDS` where the harness reads it.

**What the oracle cannot see, and it is a third of the apparatus.** Five
properties read off `app` records catch what reaches a client. A device that
completed in a different order, refused for a different reason, or wrote a
different length — while still answering every token once, intact, and leaving no
client hanging — passes all five. That class belongs to the digest, which
`cargo xtask sim` requires to reproduce and to move, and to the seven per-class
assertions of RFC 0039. Three layers, none subsuming another, and a reader who
takes a clean sweep for a clean simulator has read one of the three.

**A cost worth naming rather than burying: every entry in `sim/corpus.txt` today
was found under a deliberate defect.** No sweep of this tree has yet found a bug
that was not put there on purpose, which is the honest state of a simulator whose
models were written three tasks ago and whose oracle is five properties old. Each
entry carries a `# under` line saying which defect found it, because a corpus of
seeds that are all green and do not say why would read as a corpus that never
found anything. The day a sweep finds one with nothing armed, it lands in the
same file by the same command and its `# under` line says so.

## What would reverse this

**A sweep that is red for a week on something nobody can act on.** The whole
design rests on a finding being a command rather than a symptom; if reports start
arriving that reproduce and cannot be shrunk to anything a person can read, the
answer is a better `Size` ordering or more moves, and if neither helps then the
oracle is checking something at the wrong altitude and the properties should move
closer to the models.

**A check that fires on a correct run.** `check::tests::every_shipped_scenario_is_clean_at_a_spread_of_seeds`
and `sweep::tests::a_clean_tree_sweeps_clean` are the guards, and a false finding
that gets past both means a property here is stronger than the system's actual
contract. The answer is to weaken *that property* and say what it lost, not to
add an exception for the scenario that tripped it — an oracle with a list of
exceptions is a golden trace with extra steps.

**Minimisation that stops being deterministic.** `minimising_twice_answers_the_same_trial`
is the check, and it is not a formality: if a candidate order ever depends on
iteration order, a hash, or an allocation, the reproduction command a nightly job
prints becomes one of several it might have printed, and the exit criterion is
not met however green the job looks.

**The defect ceasing to be reachable.** `mutate-crossed-completion` needs two
consecutive coalescing decisions with work still behind them. A change to the
scenario table that stopped any scenario coalescing that deeply would make the
harness pass by not looking — which is why it fails loudly with that sentence in
the message rather than reporting a quiet green.

**A host harness for the frame.** RFC 0032 says the simulator will never catch a
bug inside the frame's own algorithms. If the frame ever becomes host-testable —
the reversal RFC 0017 already names — the sweep's grid grows a dimension and the
oracle grows properties about the frame, and the split between what `sim --join`
compares would have to be redrawn with it.
