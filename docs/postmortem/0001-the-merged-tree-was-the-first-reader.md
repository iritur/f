# Post-mortem 0001: the merged tree was the first reader

- Date of the incident: 2026-09-07
- Date of this document: 2026-09-07
- Detected by: `cargo xtask verify` on the merged tree, run before the merge was
  called done. Nothing in any branch reported any of it, and nothing could have.
- Commits: caused by `d188c62`, `1c84725`, `50d6a6c` and `6d90401` landing in
  parallel; surfaced and fixed by `5cfcb6f`, `baa6de5` and `d13c686`. The
  directory-walker half was caused by `5817a20` paying only the first of five
  and finished in `d13c686`.

## What happened

Four builds of epoch E2's fifth wave were written in four git worktrees, none of
which could see the others: `E2-B06`/`E2-P08` (the place swap), `E2-B07` (the
measured boot), `E2-P07` (the rollback) and `E2-P05` (the whole-system
comparison). Each was green in its own tree, on the checks that tree's own
change needed — the comparison branch says so in its commit message, in as many
words: *`lint`, `test` and `compare` are green; `run` was not needed — nothing
here is in the frame or on the boot path.* That sentence was true of the branch
and false of the tree it was about to be part of.

They were merged one at a time. Every textual conflict was a parallel-feature
conflict and every one was resolved by keeping both sides: a `Cargo.toml` taking
two dependencies, a usage banner listing two flags, `xtask/src/main.rs`
declaring two verbs. Nothing in the conflict resolution was interesting, which
is the part worth remembering.

Then the merged tree was run, and five things were wrong that no branch had
been able to see:

1. **Every boot in the tree refused itself.** `E2-B07` put `f.root=` and
   `f.frame=` on every command line, because *what are you running* is a
   question a machine should answer on a Tuesday. `E2-P07` made a root that no
   offered module folds to into a refusal that ends the boot, because a machine
   that quietly booted a different generation is the failure a rollback test
   exists to catch. Both are right. Neither branch offered a boot module on an
   ordinary boot, so `cargo xtask run` — the cheapest command in the tree —
   asked for a generation and then refused to be it.
2. **`cargo xtask rollback` composed half a statement.** Its six boots named a
   root and no frame hash, which the merged frame refuses as `HalfADeclaration`
   before it reaches the selection those boots exist to test. Every one of them
   would have died on the measurement, and the error would have named the wrong
   thing.
3. **`cargo xtask generation --install` wrote a boot menu that boots nothing.**
   Each `menuentry` carried `f.root=` from the generation it was describing and
   `f.frame=` from whichever build last sat in `target/`. On QEMU nobody would
   have noticed; on hardware every entry in the menu would refuse, and the first
   reader of the defect would be somebody rebooting a machine they cared about.
4. **Two lines in one boot log began `  generation    `.** `E2-B07` prints the
   selected root in the identity window near the top; `E2-P07` prints the
   frame's selection block from `kernel::generation::report` five hundred lines
   further down `kernel/src/main.rs`. `cargo xtask attest` took the first line
   under that label and read its first word as a digest — so which of the two
   answered *what did this machine measure* was a fact about where two unrelated
   `kprintln!`s sit, and on the merged tree it happened to be the right one.
5. **Four directory walkers were reading other checkouts of this repository.**
   The agent harness puts git worktrees under `.claude/worktrees/`, and
   `lint-manifests` reported twenty-eight problems against four trees that were
   not this one.

## What made it possible

For 1 through 4: **two halves of one contract were written in two places that
could not see each other.** The contract is the boot command line —
`f.root=<64 hex> f.frame=<64 hex>`, one statement, refused in halves. `E2-B07`
wrote the reader and made every boot carry the tokens; `E2-P07` wrote the
composer and composed only half of them, in three commands and in every
`menuentry`. Each branch's tests passed because each branch contained one half
and tested it against itself.

This is the same shape as the wave-4 collision recorded in `docs/rfc/README.md`:
two worktrees both wrote an RFC and both called it 0065, and — far more
dangerous — both stamped a new manifest count byte at the record's byte 101.
There the shared thing was a numbered registry; here it is a grammar. In both
cases the merge was the only reader that had both sides.

For 5: **a skip list copied into five walkers.** `5817a20` found the first one
and fixed it where it was, with the reversal condition written beside the skip.
Four others had their own copies of the same `matches!(name, "target" | ".git" |
…)` and none of them learned.

## What made it invisible

For 1 through 3, nothing was invisible for long — but only because the merged
tree was *run*. The three defects are all of the shape *a check that fires
correctly on an input the branch that wrote it never produces*, and no amount of
reading either diff would have surfaced them: both diffs are correct.
Defect 3 is the one that would have stayed invisible, and it is the one worth
being frightened of. Nothing in `verify` boots a `menuentry`; the fragment is
written under the build directory and read by a loader this repository does not
contain. Its first reader would have been a stranger's hardware.

For 4, nothing was wrong yet and that is the point: reading a labelled block by
position gives you a line, and a line that parses is indistinguishable from the
right one until the two disagree. The order that made it right is not a rule
anybody wrote down, and the next line printed under that label would have moved
it. This is the one of the five that was found by reading rather than by
running, and it was found only because the merge made somebody look at both
print sites at once.

For 5, the findings named paths in this tree and were about a different one,
which `5817a20` already recorded as the worst shape a lint finding can have: not
wrong in a way that makes you doubt the lint, wrong in a way that makes you
doubt the tree.

## What was true that we believed was not

**That a branch which is green is a branch that composes.** Four green trees
merged cleanly, and the merged tree could not boot. Green means *this tree's
checks pass on this tree's inputs*, and where two branches split a contract,
neither tree contains the input that breaks it.

The corollary, which is the sentence to keep: **the merge is not a text
operation, it is the first execution of a program nobody has run.** Three of the
five were found by running the merged tree: one directly, and two by following
that one's cause into the other two places that composed the same line. None of
the three was found by reading the merge, and every conflict in it was resolved
correctly.

## What changed

Each line names something that runs.

- **`cargo xtask run` is now the check for defect 1.** `emulator` puts the
  packed generation on the loader's module list beside the tokens on the command
  line — six modules of `MAX_MODULES`'s eight — so *every* boot in this tree is
  now a demonstration that selection works, rather than only the six in
  `cargo xtask rollback`. A future half-written boot contract fails the cheapest
  command in the repository instead of the most expensive.
- **The token pair is spelled once**, in `generation::tokens`, and `rollback`
  declares the clean frame on every boot it composes. Defect 2 has one place to
  be wrong in rather than four.
- **`fragment`'s test requires both halves of every `menuentry`**, and each
  entry now carries the frame hash read out of *that* generation's own record
  tree rather than whichever build last sat in `target/`. Defect 3 is the one
  with no runtime reader in this repository, so the test is the only thing that
  can hold it.
- **`attest` picks the generation line by content** — the first line under the
  label whose first word is sixty-four characters — and not by order, because
  order there is a fact about where two unrelated `kprintln!`s sit and nothing
  holds it still.
- **Four more walkers skip `.claude`.** One lesson and not four fixes:
  `CLAUDE.md`'s *Common mistakes* gained the line, because this is the second
  occurrence and that section's rule is *twice*.
- **`CLAUDE.md` gained the parallel-worktree line**, which `docs/rfc/README.md`
  judged unearned after wave 4 on the explicit condition that a second
  occurrence would earn it. This is that occurrence, and that file's paragraph
  now says so rather than being left to contradict the rule it cited.
- The claims that came out of the wave are `claims/0029` through `claims/0032`;
  the RFCs are 0012 (whose open question below is recorded against it), 0013,
  0063, 0065 and 0066. No eval came out of this, and the next section says why.

## What we chose not to change

- **A lint that refuses a second walker with its own skip list.** Five walkers
  now carry the same `matches!`; the obvious fix is one `fn skips(name: &str)`
  and the reason it was not taken here is scope — this document's diff is
  bookkeeping, and rewriting five call sites in `xtask` belongs to whoever next
  touches one of them. What is not deferred is the record: the line in
  `CLAUDE.md` is what a sixth walker's author reads, and this paragraph is what
  makes the sixth occurrence a decision rather than an accident.
- **An eval.** `evals/README.md` treats an incident as skipping the queue
  straight to a task, and the temptation here is an eval for *did the agent
  check the other worktrees*. It would fail for the wrong reason: neither agent
  could read the others, and both checked everything they could see. The defence
  that works is the one taken above — make the contract fail in the cheapest
  command — and the defence that does not is a rule telling agents to be more
  careful about trees they cannot open.
- **A pre-merge lint on the boot command line's grammar.** Considered and
  rejected: `abi/src/boot.rs` already owns the grammar and the frame already
  refuses half a declaration by name. The defect was not that the rule was
  unwritten, it was that one composer had not read it, and a lint for *every
  composer of this line calls `generation::tokens`* is a lint with one call site
  and no second implementation to disagree with it.

## Recorded rather than answered, and since answered

`cargo xtask rollback`'s `refuses()` boots `f.root=` with an all-zero root, which
is a well-formed token and selects nothing. The frame parses it, publishes
generation counter **1** with four zeroed root words into its state tree, and
only then — five hundred lines later in the boot — refuses the boot because no
offered module folds to it. So a reader following RFC 0012's protocol sees a
machine attesting to a root of zeros with a counter that says a root describes
it. Zero is reserved on the *counter*, and this is a well-formed counter over a
root value nothing can produce. It is written up against RFC 0012 in
`docs/rfc/README.md`, where that decision's owner will see it; it is not
answered here, and nothing in this wave depends on the answer.

**Answered on 2026-09-07 and the paragraph above is kept as it was written.**
The selection now runs before the state tree is published, so the refusal boot
publishes no identity at all and there is no counter for a reader to believe.
The full answer, including the two alternatives that were refused and what
would reverse this one, is against RFC 0012 in `docs/rfc/README.md` — one
decision, written where its owner filed the question, and named from here.
