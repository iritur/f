# RFC 0046: A hostile peer is generated, and a hang is a count

- Status: accepted
- Date: 2026-09-03
- Affects: `ring/tests/hostile.rs`, `ring/corpus.txt`,
  `claims/0008-hostile-peer-operations.toml`, `xtask` (`hostile`),
  `.github/workflows/ci.yml`, `.github/workflows/nightly.yml`,
  `docker/Dockerfile`, `docs/test-taxonomy.md` and `.toml` (the L4 layer row
  and group B's four hostile-peer rows)

## Decision

`E1-P04`'s exit — *one billion hostile operations with no kernel panic, no
memory unsafety, and no hang* — is met by **three separate runs asserting three
separate properties, at three different counts, with all three counts written
down**.

1. **No panic**, and **no hang**, over the ordinary build, at a billion
   operations. Measured: **44.3 s to 60.3 s** for 1 000 000 512 operations over
   five runs, single threaded, on the four-core development container. The
   range rather than the best of them, because the machine is shared and a cost
   quoted at its minimum is one somebody later cannot reproduce.
2. **No memory unsafety**, over the same generator under Miri, at **4 096**
   operations per commit and **65 536** nightly. Miri costs about six orders of
   magnitude, so it cannot be the same run and pretending otherwise would be a
   claim about the property it checked and the count it never reached.

   **This conjunct of the exit is not met at a billion, and no tool available
   meets it.** Miri interprets about 29 operations a second here, so a billion is
   of the order of a year. What is done instead is what this tree does with every
   other gap it cannot close: the shortfall is a **number in the registry**.
   `claims/0008`'s `unsafety_gap` is `exit_operations / miri_operations`, `xtask`
   recomputes it from the constants it actually runs and requires the claim to
   agree, and raising the exit's count without raising Miri's turns the run red.
   A sentence beside a green result would have been the alternative, and
   `JOIN_GAP` and `CHAOS_GAP` are the precedent for preferring a quantity.
3. **Reach**, which is neither of the three and is what makes them mean
   anything: the run counts every path it touched and
   `claims/0008-hostile-peer-operations.toml` puts a **minimum** on the ones
   that say the interesting paths were reached. The claim's rows and `xtask`'s
   list are checked against each other on every run — they are one list written
   twice, and a copy nobody reads is a threshold that quietly stops existing.
   The minimums are **stated per 100 000 000 operations and scaled to the run**,
   and they are about a hundredth of what a healthy run produces rather than
   `min = 1`; *The reach minimums have to be able to bind* says why.

And **a hang is a count, not a duration**. There are exactly three loops on the
peer-facing path of `f_ring`, each has a ceiling that is a function of the
channel's geometry and never of anything the peer wrote, and the fuzzer counts
the work each operation actually does and refuses it against that ceiling. No
wall clock is read anywhere in the fuzzer.

### What *kernel* means in the exit, and why this runs on the host

The exit says *no kernel panic* and this suite is a host test against `f_ring`.
That is not a substitution and it is worth one paragraph rather than a footnote.

`f_ring` **is** the peer-facing path. The same crate, the same
`Mapping::adopt`, the same `Consumer::pop`, the same `execute` run inside the
frame and inside this test; the frame is where they are called from, not a
second implementation of them. Running them on the host is what makes a billion
operations cost a minute instead of a billion boots, and it is what makes Miri
possible at all — there is no way to run an aliasing checker over code executing
in QEMU under a kernel.

What the host cannot show is the frame calling them: a hostile peer against the
*machine*, with a real component, a real entry and a real fault path behind it.
That is `cargo xtask runtime`'s fourth boot, which scribbles the control ring's
header before a component is entered and requires the adoption to refuse rather
than believe it — E1-B08, RFC 0038. One boot against the machine and a billion
operations against the code are complementary, and neither is the other's
sample. A defect this suite finds is a defect the kernel has, because it is the
kernel's code; a defect only the boot can find is one in how the frame calls it.

## Context

`E0-B13` left `ring/tests/headers.rs`: fifteen hand-written hostile headers
driven into a real 4 KiB region, each required to be refused with the domain and
code RFC 0010 names. It is a good test and it has a ceiling — a hand-written
case is a case somebody thought of, and the bugs that survive review are the
ones nobody thought of.

Three things were live when this was decided.

**How to say *hang* without a timeout.** The obvious harness is a watchdog: run
the operation, kill it after *n* seconds, call that a hang. It is also a flake
generator. A timeout fires on a loaded CI runner for reasons that have nothing
to do with the code, and once it has done so twice somebody raises it until it
never fires — at which point the property is not being checked at all. The
alternative that was live and rejected was *no hang assertion at all, with the
job timeout as the backstop*, which is what most fuzzers do.

**Whether Miri could carry the whole exit.** It cannot, and the first
measurement said so bluntly: one episode of 1 024 operations took **three
minutes** — and most of that turned out to be the *test's* fault rather than
Miri's, which is the second half of this RFC's content.

**Whether a billion is affordable per commit.** Measured rather than assumed:
44 to 60 s in the development container, so of the order of two minutes on a two-core GitHub
runner. Affordable in a job; not affordable in `cargo test --workspace`, which
builds unoptimised and runs at about 0.9 M operations per second against
release's 22.6 M.

### Why not an existing engine

`docs/design/proving-ground.html` says it twice — *do not write a fuzzer*, write
the F-specific parts and drive them with an existing engine — and this file is a
generator and a loop with no engine under it. The advice is honoured where it
applies and refused where it does not, and both halves are worth stating.

What the advice is about is the **mutation engine**: coverage feedback, corpus
scheduling, minimisation. That is a mature field, it is not F-specific, and
`E1-P05` is the task that buys it. This is the other half — the peer's behaviour,
which nobody else's engine knows how to draw.

What made an engine the wrong host for *this* run is the determinism contract.
RFC 0004 says nondeterminism reaches this system only through `f_env::Env`, and
`cargo xtask lint-determinism` scans `ring/` with no allow-list entry, so a
harness under `libFuzzer` would have its input decided by a component that reads
its own clock and its own entropy and cannot be asked what it did. The property
this run needs is not *more inputs* but **a finding that is a seed**: an episode
derived from `(seed, index)` reproduces in a millisecond, stands alone in
`ring/corpus.txt` as an argv, and replays identically under Miri — which an
engine's corpus file, produced by a build that does not exist any more, does not.

The trade is real and it is the reversal condition: no coverage feedback means
this generator finds what its families of hostile values reach and nothing else.
If `E1-P05` finds a class of entry this one reaches at a rate near zero, its
corpus is fed in here rather than the families being widened by guesswork.

## Consequences

### A hang is a count

`f_ring`'s peer-facing path has three loops and no others:

| loop | ceiling | asserted by |
|---|---|---|
| `Service::drain` | `budget` iterations | `Drained::executed <= BUDGET` |
| `write_serial` | `arena_len` bytes, in at most `arena_len` calls, because a piece is at least one byte and the range was already refused unless wholly inside the arena | the counting sink |
| `Arena::copy_out` | the slice it was handed, which is one of those pieces | the counting sink, transitively |

So *stuck* is defined as **an operation that did more work than the geometry
permits**, and that is the same judgement on a fast machine and a slow one. The
refusal is load-bearing rather than decorative: the sink answers zero once the
bound is passed, `write_serial` treats a short answer as a partial completion
and stops, so a hypothetical unbounded loop **terminates and reports itself**
instead of becoming a job somebody kills at the timeout.

### The counts, and why they differ

| run | count | measured | where |
|---|---|---|---|
| `cargo test --workspace` | 2 097 152 | 2.4 s, unoptimised | every local `verify`, every CI test job — free, because the target is already in the workspace |
| `cargo xtask hostile` | 100 000 000 | 4.4–7.3 s, release, over four runs | `verify`, and the CI workload job |
| the exit's own number | 1 000 000 512 | 44.3–60.3 s, release, over five runs | the CI workload job, and nightly at a walking base |
| Miri, per commit | 4 096 | 119–137 s end to end over three runs, about a minute of it sysroot | its own CI job |
| Miri, nightly | 65 536 | about 11 minutes | nightly |

Writing all five down is the point. A single number would let the small one be
gated quietly while the large one appears in prose.

**And three of them are thresholds rather than prose.** `claims/0008` carries
`operations` (the gate), `exit_operations` (the exit's billion) and
`miri_operations`, and `hostile_thresholds_match` requires each to equal the
constant `xtask` runs. `cargo xtask hostile --exit` refuses to call itself the
exit if it performed fewer than `exit_operations`. The claim's published
reproduction command is still the gate — the registry allows one command per
claim and `lint-reproduce` enforces its exact form — and `[reproduce]` names the
other two beside it, so a stranger is told which number the published command
reaches rather than left to infer it from a workflow file. Before that, the
registry gated a tenth of the exit and published the whole of it.

### The reach minimums have to be able to bind

Every reach row was `min = 1` when this landed, and a minimum of one catches only
a path that has **stopped**. The observed values at the gate's count span 60 068
to 298 507 655, so a regression that collapsed a path by five orders of magnitude
— a generator drawing an unknown opcode once a run instead of sixty-seven
thousand times — would have passed unchanged while the claim's own prose said
that exactly this fails. Reach is the only thing standing between a clean billion
and a vacuous one; a bound on it that cannot bind is a plan.

So each row is stated **per `operations`** — per a hundred million — and `xtask`
scales it to the run that happened, floored at one. The gate enforces the number
in the file, the exit's billion enforces ten times it, and a short diagnostic run
still enforces *reached at all*. The values are about a hundredth of the observed
ones: far enough below to survive ordinary drift in the generator, near enough to
fire on a collapse.

### The reproduction is an episode, not a prefix

A run is `episodes * 1024` operations, and an episode is a freshly zeroed region
with a sound header. Each episode's stream is derived from `(seed, episode
index)` **by identity** — RFC 0026 — so episode 700 000 is the same episode
whatever ran before it, and a finding at operation 999 999 999 reproduces in one
millisecond rather than in forty-four seconds. That is split-by-identity paying
for itself a third time, after `sim.rs`'s sites and `E1-P08`'s snapshots.

The cost is stated rather than hidden: **a peer cannot carry a corruption across
an episode boundary**, so a bug needing more than 1 024 operations of
accumulated damage is not reachable. `STEPS` is the knob and raising it trades
reproduction cost for reach.

### What Miri actually costs, and what the test owed it

The first Miri measurement was three minutes for one episode, and the cause was
this test rather than the tool. The region was a `#[repr(align(4096))]` value
and every access derived a fresh raw pointer from a `&Page`; under Stacked
Borrows each derivation retags the **whole four-kibibyte allocation**, so one
byte poke cost four thousand bookkeeping operations and zeroing the page cost
sixteen million. Taking the pointer once — from `alloc_zeroed`, freed in `Drop`
— moved one episode from three minutes to about ten seconds.

The general lesson is worth more than the fix: **under an aliasing checker, what
a test costs is the number of aliasing events it generates and not the number of
operations it performs.** A harness that wants to be run under Miri is written
against that budget from the start.

Two further shapes were forced by the same tool and are recorded because each
looks like a stylistic choice and is not:

- The region is an **allocation and not a `Box`**. A pointer derived from a
  box's unique tag is invalidated by any later move of that box, and a run holds
  the region across every episode.
- It is **freed and not leaked**. Miri's leak check is on by default, and a
  suite that has to be told to ignore leaks cannot notice one in the code under
  test.

### The declared gap: `MIRI_GAP`

RFC 0037 carries a channel's base address as a `u64`, because that is what it is
— an address in the component's own space. So `Adopted::at` and `Adopted::bind`
perform integer-to-pointer casts, and Miri says so:

> integer-to-pointer cast … Miri might miss pointer bugs in this program

That is not a defect to fix here. It is RFC 0037's design, and the alternative —
`Adopted` holding a pointer — would reintroduce exactly the borrow that RFC made
a component free of. What it means is stated instead:

**`MIRI_GAP`: on the `Adopted`/`Client`/`Server` path, Miri checks
out-of-bounds accesses, uninitialised reads and invalid values as it does
everywhere; its *aliasing* discipline on pointers reconstructed from that `u64`
is weakened to permissive provenance. The `Mapping::adopt` path — which the
`batch`, `drain` and `execute` operations use — is checked in full.**

Both paths are driven by the same generator on every run, so the strong half is
never the only half; and the strong half is the one that reaches `Consumer::pop`
and `execute`, which is where the reads a hostile peer aims at actually happen.
The gap closes if `f_ring::adopt` ever grows a pointer-carrying form, and it is
checked the way `JOIN_GAP` and `CHAOS_GAP` are: by being written here and named
in the claim rather than discovered next to a result.

`MIRI_GAP` is about *what* Miri checks on one path. Its other half is about **how
much** it checks at all, and that one is a threshold rather than a paragraph:
`claims/0008`'s `unsafety_gap = { max = 244140 }` is `exit_operations /
miri_operations`, recomputed by `xtask` from the constants it runs. The two are
separate because they fail separately — a pointer-carrying `Adopted` would close
the first and leave the second exactly where it is.

### The declared gap: `PEER_GAP`

The peer and the honest end are **one thread**, and every hostile write lands at
an operation boundary rather than inside a call. There is one exception and it
is deliberate: `Peer::batch` stages a batch, lets the peer restart and rewrite
the header while the entries are staged and unpublished, and then publishes —
which is the exit's *restarts mid-operation*, and it is expressible only because
both ends hold the region by shared reference rather than by `&mut [u8]`.

So:

**`PEER_GAP`: a peer that writes while a single call is executing — between
`Consumer::pop`'s bounds check and its read, or between `execute`'s range check
and `write_serial`'s copy — is not generated. What is generated is a peer that
writes between calls, and one that writes inside the one multi-call operation a
batch is.**

That is not a gap in the *ring*: `Consumer::pop` copies the entry out before any
field is examined, and `execute` validates the copy rather than the shared bytes,
which is what makes the window empty by construction rather than by timing. It
is a gap in the **evidence**, and the distinction matters because the mechanism
is a design property somebody could remove without any test here noticing.

One smaller assumption is written down beside `Region::address` rather than here,
because it is about the reproduction rather than about the peer: the region's base
is an **allocator address**, so it differs between processes and no seed
determines it. Nothing observable depends on its numeric value today — what
`Adopted::at` reads out of it is alignment and length, both properties of the page
rather than of where the page landed — and the reversal condition is a finding
that does not reproduce in a second process at the same seed and episode.

Closing `PEER_GAP` means a second thread, and that is a different instrument: the
property would stop being *no panic* and become *no data race*, which is Miri's
verdict under `-Zmiri-preemption-rate` rather than a counter's. It is written
here rather than discovered beside a green run, the way `JOIN_GAP` and
`CHAOS_GAP` are, and `docs/test-taxonomy.md`'s *entry mutated between validation
and use* row carries the same sentence in its own words.

### The harness's own arguments are answered, not refused

`harness = false` makes the binary the test, and `parse` failed closed on every
argument it did not know — R04 applied one level too far out. Cargo hands **every**
test target in a package the same arguments, so `cargo test -q --release -p f-ring
litmus` ran the six litmus tests, passed them, and then failed the package because
`hostile` was handed a filter meant for another target. `--list`, which is how
cargo and nextest enumerate tests, failed the same way.

So the two kinds of argument are separated. The fuzzer's own options — `--seed`,
`--ops`, `--episode` — still fail closed, because a misspelling that quietly ran a
different run than the one somebody asked for is what a gate binary cannot afford.
Cargo's protocol is answered the way libtest answers it: a filter that does not
match the target's name selects nothing and exits zero having run nothing and said
so, `--list` names the one test, and libtest's own flags are accepted and ignored.
**R04 is about fields a peer writes.** The harness's argv is not one of them, and
reading it as one broke every narrowed test run in the tree.

The same pass moved the episode arithmetic out of `run` and into `parse`, with
`checked_mul` and `checked_add` and a refusal naming the bound. `--ops` within
1024 of `u64::MAX` rounds up past the end of the range, and in the debug build
`cargo test --workspace` uses that is an overflow panic in the harness — reported,
by a fuzzer whose whole subject is arithmetic on numbers somebody else supplied, as
a finding about the ring.

### The residual on the hang property

A counter in the caller cannot catch a loop that never calls out and never
returns. Every loop that exists today does call out or is structurally bounded,
which is why the table above is a table rather than a hope — but a loop added
later might not be. The residual is the job timeout, and it is a backstop rather
than an assertion. Saying so is the whole of R01 applied to this file: a
mechanism is named where there is one, and a plan is called a plan where there
is not.

### The harness can fail

Three deliberate defects in `ring/`, one per property, on RFC 0017's argument
extended the way RFC 0040 extended it to the simulator:

- `mutate-believed-header` — `Mapping::adopt` unwraps the layout the peer
  described instead of refusing it. **Panics.**
- `mutate-trusted-slot` — `Consumer::pop` drops the bounds check on the slot
  number in the index ring and reads through a raw pointer. **Memory unsafety**,
  and deliberately not a panic: an unchecked *index* would be a bounds-check
  panic, which is the other property's fixture. This one reads past the entry
  array into the completion ring and returns a plausible entry, which is
  precisely what Miri exists to notice and what the ordinary build cannot.
- `mutate-unbounded-drain` — `Service::drain` ignores its budget. **Stuck**, and
  it is the one that shows why the hang property is a count: the run still
  terminates, in this single-threaded harness, and the defect is a peer choosing
  how long a call takes.

One defect proves one signature. Three properties with one defect between them
would be one property under test and two decorations — RFC 0042's finding,
applied here before it could be rediscovered.

### The corpus

`ring/corpus.txt`, in `sim/corpus.txt`'s shape: a header, then a comment block
per entry naming what was found, at which commit, under which defect and with
what evidence, then a line that **is an argv** for the fuzzer. Following the
existing shape rather than inventing a second one is deliberate; there is no
format beyond *a line is an argv*, plus one rule about the comments — a blank
line ends a block, so the comments immediately above a line belong to that entry
and the file's header belongs to none.

Three things about it are decisions rather than mechanics, and the first two were
got wrong once each.

**A recorder reads the blocks, not only the argv.** The first version of
`hostile_record` parsed entries by stripping every comment line and re-emitted
them bare, so each `--record` run silently deleted the provenance of everything
an earlier run had written — and the file that shipped was seven bare lines under
a header claiming each said what it was found under. The header was describing a
format the writer destroyed. Provenance is now carried through verbatim.

**The corpus is shown to be able to go red.** `sweep_mutate` spends its `[3/5]`
on exactly this and its comment is the argument: *a regression suite whose
entries have never been seen to fail is a file of command lines nobody has
tested*. That step was missing here, which is this epoch's fourth instance of a
check that is green while the thing it stands for is absent. `--mutate` now arms
each of the two visible defects in turn and requires **every entry whose `#
under` line names it** to fail, then disarms and requires every entry to pass.
The requirement is exact rather than statistical: an entry recorded under one
defect going red under another says nothing either way, so only the entries a
defect owns are required, and the rest are counted.

**What an entry is worth is measured, not assumed.** An entry says *this run
found something once*. What it cannot say is whether a run that did **not** find
it exists — and if none does, the entry carries exactly the information an
arbitrary episode carries, which is none. So `--record` replays five *control*
episodes, chosen not to be corpus entries, with the same defect armed, and writes
the count into the entry's `# also` line. `HOSTILE_SELECTIVITY` in `xtask` holds
the expected count per defect and `--mutate` checks each for equality, both
directions, the way `JOIN_GAP` is a set that must match exactly.

The two answers differ, and that is the useful part:

| defect | control episodes reproducing it | what an entry recorded under it is |
|---|---|---|
| `mutate-believed-header` | 5 of 5 | provenance — a seed, a commit and an evidence line that outlive the run, and nothing an arbitrary episode lacks |
| `mutate-unbounded-drain` | 1 of 5 | a regression suite in the full sense: the entry reaches something four episodes in five do not |

Half the file earns its keep and half of it is provenance, and saying which half
is which is worth more than an average would be. A defect that becomes easier or
harder to reach moves a number in `HOSTILE_SELECTIVITY`, rewrites the `# also`
lines through `--record`, and changes what this section says.

### One row this RFC does not move, and says so

`docs/test-taxonomy.md`'s group B moved with this change: the L4 layer row names
the fuzzer, the *entry mutated between validation and use* row stops reading
`nothing / never`, and the *lies about its epoch* row stops saying that nothing
writes a hostile cursor. Not one of them reached **catches**, which is the
honest outcome — a billion draws is a large sample and a sample is what it
stays.

`docs/TESTING-STATUS.md`'s own L4 line still reads *no hostile-peer fuzzer*, and
it is wrong as of this commit. It was left alone deliberately: that file is a
status page maintained as one document rather than row by row, and editing one
line of it from inside a task is how a status page acquires two voices. The
correction belongs in its next pass, and naming it here is what stops it being
discovered instead.

## What would reverse this

- **A loop appears on the peer-facing path whose ceiling is not a function of
  the geometry.** Then the table above stops being complete, the counting sink
  stops being sufficient, and the hang property needs a different mechanism —
  most likely a step budget threaded through the ring itself, which is a change
  to the shipped code rather than to a test.
- **A billion operations stops being affordable.** It is 44 to 60 s today. If the
  channel grows so that one operation costs ten times more, the per-commit run
  drops to the hundred million and the billion becomes nightly-only. The numbers
  are in the claim so that this is a diff rather than a habit.
- **Miri gains a way to check the `Adopted` path's aliasing.** Then `MIRI_GAP`
  is deleted rather than narrowed. Strict provenance would do it, and it would
  require `f_ring::adopt` to carry a pointer — which RFC 0037 rejected for
  reasons that have nothing to do with this file, so the trade would have to be
  re-argued there.
- **A structure-aware generator finds what this one cannot.** `E1-P05` is the
  coverage-fed submission fuzzer and it is a different instrument: this one
  draws from families of hostile values with no feedback at all. If `E1-P05`
  finds a class of entry this generator reaches at a rate near zero, the right
  answer is to feed its corpus in here rather than to widen the families by
  guesswork.
- **A defect stops being reached by every control episode, or starts being.**
  `HOSTILE_SELECTIVITY` goes red in either direction, which is the point of
  checking it for equality rather than for a floor. The response is to move the
  number, re-record the `# also` lines, and change the table above — never to
  widen the check.
- **`E1-P12` lands.** That is the task that owns the shortfall this RFC declares.
  Kani over `Layout::adopt`, `Consumer::pop`, `take` and `execute` proves the
  panic-freedom and memory-safety properties over *arbitrary* header bytes,
  cursors and entries rather than over a sample, at which point `unsafety_gap` is
  **deleted** rather than narrowed and this suite becomes the thing that says the
  proof's assumptions still describe the code. Until then the gap is a number in
  the registry and `E1-P12` is its owner, which is the honest form of *this
  conjunct of the exit is not met*.
- **Miri gets fast enough, or something cheaper gets strong enough.** Then
  `miri_operations` rises and `unsafety_gap` falls. AddressSanitizer is the
  obvious candidate and does **not** qualify: `mutate-trusted-slot` reads past
  the entry array and stays inside the same 4 KiB allocation, which is precisely
  what a redzone allocator cannot see and what Stacked Borrows can — so a cheap
  instrument here would have to be an aliasing one, and there is not one.
- **The episode boundary is shown to hide a bug.** A finding that needed a
  longer episode to reach — noticed because it appears at a larger `STEPS` and
  not at 1 024 — retires the reproduction-cost argument, and the fuzzer would
  then need a real minimiser rather than a structural one.
