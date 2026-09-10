# RFC 0069: A claim states the kind of evidence behind it, and a kind may not stand in for another

- Status: draft
- Date: 2026-09-10
- Affects: `claims/*.toml` (one new key), `claims/README.md` (a sixth rule),
  `xtask/src/main.rs` (`lint-claims`, `claim`, `claims --render` and the
  snapshot), `docs/TESTING-STATUS.md`'s L7 row, and every document that
  renders a number from the registry
- Implements nothing yet. This entry records the decision; the lint and the
  thirty edits that make it mechanical are the diff behind it, and the RFC is
  `draft` until that diff lands, so that a reader cannot find a rule here that
  the tree does not yet enforce

## Decision

Every entry in `claims/` carries an `evidence` key with exactly one of three
values, and the value is the *kind of observation* a green result is:

| `evidence` | what a green result is | what produces it |
| --- | --- | --- |
| `proved` | a checker's verdict that no counterexample exists inside a stated bound | `cargo xtask prove <harness>` |
| `counted` | a count over a deterministic run, identical on every machine | a boot, a seed, a fuzzer, a test — anything under RFC 0004 |
| `measured` | a distribution drawn from one machine, on a named runner class | `bench/`, and only where it consents to record |

Three rules follow, and they are what the key is for:

1. **A kind may not stand in for a stronger one.** A claim whose statement
   quantifies universally — *no*, *never*, *every*, *all 2³²* — is `proved` or
   it is misworded. A `counted` result over a billion operations is a billion
   samples and is written as one. A `measured` result on one machine is a fact
   about that machine and is written as one. The lint cannot read a statement's
   quantifier, so this rule is review's; what the lint can do is refuse the
   combinations below.
2. **Only `counted` gates on an ordinary machine.** `status = "gating"` with
   `evidence = "measured"` is refused by `lint-claims` unless the entry's
   `[hardware] runner` is a class `bench/src/lib.rs` will record on. This is
   `docs/TESTING-STATUS.md`'s *a count may gate on this machine and a time may
   not*, which today is a sentence in a status table and a refusal inside the
   harness, made a rule the registry checks before anything runs.
3. **A `proved` claim names its harness and nothing else.** Its `[workload]`
   is a harness in one of `xtask`'s proof tables and its reproduction is
   `cargo xtask prove <harness>`. It has no `[baseline]`, because a proof is
   not faster or slower than Linux, and no `[hardware]`, because a solver's
   verdict does not depend on the machine it ran on. Today the registry has
   no such entry — every proof in `kernel/proofs` and `ring/proofs` is a
   property with no number — and this rule exists so that the first one is
   registered correctly rather than by copying `0001`.

The key is rendered wherever the number is: a document that quotes a claim
quotes its kind beside it, and the snapshot `cargo xtask claims` writes carries
it, so a reader of `docs/design/` can tell a solver's verdict from a count from
a measurement without opening the registry.

## Context

What was true when this was decided.

The registry distinguished a claim's *status* — `pending`, `tracked`,
`gating` — and its *machine* — `[hardware] runner` — and nothing else. Thirty
entries, seventeen gating, twelve pending, one tracked. Every gating entry is a
count and every pending entry is a time, and that split is the honest part of
the L7 row in `docs/TESTING-STATUS.md`: it follows one rule, and the rule is
stated there, in prose, and enforced by `bench/src/lib.rs` refusing to record
in a container. Nothing in `claims/` says which kind an entry is. A reader who
opens `0008-hostile-peer-operations.toml` finds a `gating` claim with a
threshold of a billion and has to know that a billion is a count of samples
rather than a bound; a reader who opens `0010-admission-refusals.toml` finds a
`gating` claim over a simulator and has to know that the simulator's clock is
fictional. The registry told them the number was green. It did not tell them
what green was evidence *of*.

The occasion was reading the AxonOS Standard's `VALIDATION.md` (AxonOS-org on
GitHub, a brain-computer-interface real-time kernel, 3 600 lines of Rust and a
conformance document). Almost nothing there transfers — it is EDF with a
utilisation ceiling on a Cortex-M4F, which RFC 0007 rejected by name — but one
discipline does. Every quantitative claim under that name carries a tag: L1,
formally proven over the whole admissible input space; L2, measured on the
reference hardware under stated conditions; L3, an L2 measurement reproduced
by a party with no stake. And one rule, which its authors call *the most
important and most commonly violated*: an L2 measurement may not be used to
make an L1 claim. A long soak is conditional evidence and cannot substitute for
a proof.

This tree already believes that. `ring/proofs/src/proofs.rs` opens with *a
billion samples is still sampling*, `docs/test-taxonomy.md` splits *catches*
from *catches (sampled)* row by row, and `f_abi::reserve::Grant::exercised` is
the predicate a claim must ask before recording a number under a reservation —
*the same admission and very different evidence*. What the tree did not have
is the tag on the entry, so the distinction lived in three documents and the
registry was the one place that did not make it.

Three alternatives were live.

**Adopt L1/L2/L3 as written.** Rejected because the levels are a hierarchy of
strength and this registry's split is a difference in *kind*. A count under
RFC 0004 is not a weaker proof; it is a different observation with a different
falsifier — a seed that reproduces a different number — and ordering it below a
proof would say that `claims/0008` should aspire to become a Kani harness,
which it should not: a hostile-peer fuzzer and a bounded proof answer different
questions and `docs/TESTING-STATUS.md`'s L2 row already says so, *two
instruments, two questions*. What is kept from the Standard is the rule that
one kind may not be dressed as another, and that is rule 1.

**Adopt L3, independent reproduction, as a fourth value.** Deferred rather than
rejected. A reproduction by somebody with no stake is a record *about* a claim,
made after the claim exists, by somebody outside the tree; it is not a kind of
observation the tree can produce, so a value for it would be one no `cargo
xtask claim` run could ever write. `cargo xtask reproduce` already prints the
refusal a third party will see and `RELEASING.md` already ships the corpus so
that one can run the sweeps. The day a reproduction arrives, the question is
what file records it and who signs it, and that is its own entry.

**Derive the kind from the entry instead of declaring it.** A time metric is
`measured`, a `bench/` workload is `measured`, a `sim/` or `tests/` workload is
`counted`, and a proof would be recognisable by its path. Rejected because
`claims/0003-boot-to-m0.toml` has a time metric, a `tracked` status and a
workload that is an `xtask` verb, and a derivation that has to special-case
the third entry is a derivation that will special-case the thirty-first
silently. Declaring the kind costs one line per entry and makes the
declaration the thing review disagrees with, which is where a disagreement
about evidence belongs.

## Consequences

Easy: a red `counted` claim is reproduced with a seed and a red `measured`
claim is reproduced with a machine, and the entry now says which before
anybody starts. The first `proved` claim is registered against a rule rather
than against `0001`'s shape, which has a baseline and a runner class it would
have had to leave blank. And *a count may gate and a time may not* is a check
that fails a pull request instead of a sentence that has to be re-read.

Hard: every existing entry gains a line, and the line has to be right. Two are
worth arguing over rather than defaulting: `0009-entry-validation-coverage` is
`counted` — a coverage share is a count of lines over a count of lines, and it
is identical on every machine because the corpus is committed — and
`0003-boot-to-m0` is `measured` even though it is `tracked` and runs on QEMU,
because a nanosecond is a nanosecond wherever it was drawn, and `tracked` is
what says it does not gate. A future entry that is a *ratio of a count to a
time* has no kind here and must be split, which `claims/0005` and `0006` have
already done once for a different reason.

Foreclosed: a claim with no kind. An entry that omits the key fails
`lint-claims`, so there is no longer a way to register a number without saying
what observing it would consist of. And a document cannot render a number
without its kind beside it, so a page cannot present a count as a bound by
leaving the word out.

Not foreclosed, and stated so that nobody reads it in: nothing here says what
evidence *level* a claim has reached. `pending` still means the machinery does
not exist, `[hardware] runner` still names the machine, and RFC 0007 still
says a number under an unexercised reservation is not a number about this
system. The kind is orthogonal to all three.

## What would reverse this

- **The key stops carrying information.** If, a year on, every gating claim
  is `counted` and every pending claim is `measured` and no `proved` entry
  has been registered, the tag is a restatement of `status` and the third
  alternative above was right. The observation is one `grep` over `claims/`;
  the response is to delete the key and record here that the split was
  already expressed.
- **A fourth kind appears that is not a reproduction.** A claim whose green
  result is neither a verdict, a count nor a distribution — an *argument*, say,
  or a hardware datasheet figure of the kind AxonOS tags *specification* —
  means the closed set was wrong. Widen it here, in a superseding entry, rather
  than by fitting the new thing under `counted`.
- **A `proved` claim goes red under a `counted` instrument.** If a harness
  verifies and a fuzzer or a boot then finds the counterexample the solver did
  not, the bound the harness was stated inside was too narrow to be worth the
  word, and rule 1 has been satisfied in letter and violated in substance. The
  response is not to relabel the claim but to widen the harness or downgrade
  the statement, and to record the pair in `docs/postmortem/`.
