---
id: 0007
status: draft
reviewed_by: "pending: Dmitri Chudinov"
skills: spec-from-intent, claims-registry, determinism-review, rfc-author, frame-and-unsafe, licence-boundary
---

# Spec: twenty-seven bounds, and the twenty-five a checkout can check

Four claims publish a `[threshold]` table and reproduce with a command that runs
their workload and reads that table with nothing. `cargo xtask claim
chunk-size-distribution`, `… blob-verification-refusals`, `… write-amplification`
and `… index-blocks-per-query` dispatch through `ROUTES` to arms that call `sh`
and return — `xtask/src/main.rs:14828`, `:14841`, `:14846`, `:14847` — while
eleven other claims dispatch to `claim_compare` (`xtask/src/main.rs:15030`),
which reads the claim's own table, reads the workload's printed rows, and fails
naming every row that is red *and* every row nothing printed. Three of the four
are `status = "gating"`. This spec is the extension of that mechanism to the
four, the retirement of the two places where a bound is typed into a workload by
hand, and a decision about the two rows that no checkout can measure.

## Behaviour

### The three shapes, and which bound gets which

The tree already has two shapes for a declared quantity, and a third for a
number held in two files. Nothing here invents one.

**Shape A — the comparison.** `claim_compare` (`xtask/src/main.rs:15030`). The
bound lives only in `claims/*.toml`; the workload prints `name value` rows under
the claim's registered metric names (`measured_rows`, `xtask/src/main.rs:14941`);
the claim's command compares every row and fails with all of them, not the first.
A row no workload printed is a finding (`:15055`). **This is the shape for every
row a checkout can produce a number for**, which is twenty-five of the four
claims' twenty-seven.

**Shape B — the gap.** `gap_holds` (`xtask/src/main.rs:465`), used by
`OWED_REVERSALS` (`:405`), `CHAOS_GAP` (`:3103`), `SWAP_GAP` (`:3446`) and
`HEAP_GAP` (`:3040`). A quantity that is true today *for a stated structural
reason*, asserted on an ordinary run, with a needle in the source whose
disappearance turns the run red and prints the documents to update. `HEAP_GAP` is
the closest precedent and the one to read: `run` requires `peak == 0` rather than
tolerating it, because "a zero that is *asserted* is evidence; a zero that is
merely printed is the `claims/0017` shape this tree has already been bitten by
twice" (`xtask/src/main.rs:3036`, assertion at `:2470`). **This is the shape for
rows nothing in a checkout can measure** — see *The third state*.

**Shape C — one number, read rather than copied.** `hostile_thresholds`
(`xtask/src/main.rs:17257`) reads `claims/0008`'s table so that `cargo xtask
hostile` enforces exactly what the registry publishes;
`admission_thresholds_match` (`:3902`) does it for `claims/0010`;
`zone/tests/cycle.rs:766` reads `pub const IN_FLIGHT` out of the driver's source
so that RFC 0059's bound is "a fact rather than a transcription". **This is the
shape for a row that must also be asserted per commit**, which is every row of
`claims/0018` and two of `claims/0024`.

### What each of the four commands does afterwards

`cargo xtask claim chunk-size-distribution` runs `blob/tests/chunker.rs`'s
`the_mean_chunk_is_within_a_factor_of_two_of_the_target` — the existing filter
stays; all eleven rows come out of that one test — and compares all eleven
against `claims/0018`'s table. The four hand-copied bounds at
`blob/tests/chunker.rs:381`, `:391`, `:404` and `:414`
(`INTERIOR_CHUNKS_MIN`, `ZERO_FILLED_INTERIOR_CHUNKS_MIN`,
`FORCED_PER_TEN_THOUSAND_UNIFORM_MAX`, `FORCED_PER_TEN_THOUSAND_STARVING_MAX`)
stop being literals and are read out of `claims/0018-chunk-size-distribution.toml`
in shape C, so the per-commit assertions `E2-P02` paid for keep firing inside
`cargo xtask test` and there is one copy of each number.

`cargo xtask claim index-blocks-per-query` runs `index/tests/query.rs` and
compares all four rows against `claims/0024`. `TOTAL_ADVANTAGE_MIN` and
`BREAK_EVEN_MAX` (`index/tests/query.rs:119`, `:127`) are read from the claim in
shape C. The run gains a row block printing `blocks_read_per_query`,
`blocks_read_per_tree_walk`, `break_even_queries` and `total_blocks_advantage` by
their registered names; today it prints only prose (`:404`–`:434`) and not one
line `measured_rows` can parse. `total_blocks_advantage` is printed as a whole
number, because the bound is `min = 8` and the run's 24.0x printed in tenths
would be 240 against a bound meaning 8.

`cargo xtask claim write-amplification` runs `zone/tests/cycle.rs` and compares
three of five rows: `zones_reset`, `collector_operations_ahead_of_hard_read`, and
`modelled_device_bytes_per_app_byte` renamed with its scale (below). The
remaining two are owed and named.

`cargo xtask claim blob-verification-refusals` runs `blob/tests/million.rs` at
`--blobs 1000000` and compares all seven rows. The report at
`blob/tests/million.rs:461` prints human names with spaces, and prints `verified`
and `refused` twice with different values across the two phases — under
`measured_rows` that is a conflict, not a measurement — so the run gains a row
block under the seven registered names (`blobs_verified_before_the_flip`,
`reads_refused_after_one_flipped_bit`, and so on). The existing human report
stays; a reader needs it to disagree.

### Fractions

`Bound` is `{ min: Option<u64>, max: Option<u64> }` (`xtask/src/main.rs:17241`)
and `thresholds_in`'s parser is `parse::<u64>().ok()` (`:14896`–`:14906`). So
`device_bytes_per_app_byte = { max = 1.5 }` parses to `Bound { min: None, max:
None }`, and `claim_compare` prints it as `no bound` and calls it green **even
where the route compares**. Five rows in the registry are in that state today:
`claims/0001:43`, `claims/0004:61`, `claims/0016:187`, `:188`, `:194`. The mirror
image is on the workload side: `zone/tests/cycle.rs:324` already prints its row
under the exactly correct registered name, and prints `1.1336`, which
`measured_rows` drops at `:14953` without a word.

Every `[threshold]` bound becomes an integer with its scale in its name — the
answer `FORBIDDEN_TYPES`' own doc comment gives for Rust
(`xtask/src/main.rs:109`), applied to the registry. `claims/0016` gets
`modelled_device_bytes_per_app_byte_ten_thousandths = { max = 15000 }` and the
cycle prints `11336`; the arithmetic for it is already in the workload, which
computes four fractional digits. **No bound moves.** 1.5 and 15000
ten-thousandths are one number spelled twice; if a re-spelling makes a row red,
the red is the finding and the row stays where it is.

The determinism policy could not have caught this: `FORBIDDEN_TYPES` is matched
against Rust identifiers (`names_type`, `xtask/src/main.rs:128`) and a TOML file
is not scanned. That is how three claims came to publish a bound in a type the
tree forbids everywhere it looks.

### The third state — the intent's first and second open questions, answered

Two rows of `claims/0016` name a boundary that does not exist in this tree:
`device_bytes_per_app_byte` is defined as a `query-blockstats` count inside a
guest on QEMU 8 against a tuned Linux on the identical device, and
`ratio_vs_baseline` is that comparison's other half. `TODO.md:1187` (`E2-P10`)
owns them. They must not be deleted, must not be satisfied by the modelled number
under a different name — that substitution is what the claim's own header spends
a page refusing — and must not be silently uncompared.

So the rule is **every row is either compared or explicitly owed**, and owed is
written in the claim rather than inferred:

```toml
[threshold]
device_bytes_per_app_byte_ten_thousandths = { max = 15000, owed = "E2-P10" }
```

`claim_compare` prints such a row as `owed  <name>  (E2-P10)`, does not require a
workload to have printed it, and does not treat its absence as a finding. A row
that is neither compared nor owed is a finding, exactly as today. The task id is
required and must be a line that exists in `TODO.md`, so an `owed` is a debt with
an owner rather than a way to make a comparison total.

This is a **reversal**: `claim_compare` today states, in its own error text, that
a row nothing printed is a finding and "do not [retire the threshold] to make
this green" (`xtask/src/main.rs:15060`). `owed` is a third answer to that
sentence and needs an RFC before it is written. See *Policy applied*.

### Where the comparisons run

The rule is already decided twice and written down in neither place a reader
would look for it: `xtask/src/main.rs:11182` — *a gating claim that nothing in
the local loop runs is a claim that gates nothing* — and
`.github/workflows/nightly.yml:653`, which runs `cargo xtask claim
rollback-comparisons` rather than `cargo xtask rollback` precisely because only
the claim's command compares the table. This spec writes that pair down once, as
a rule in `claims/README.md`, and applies it:

- `chunk-size-distribution`, `index-blocks-per-query` and `write-amplification`
  run their workloads inside `cargo xtask test` already, so their shape-C
  assertions fire per commit. The comparison is the claim's command and costs
  seconds; nothing new enters `verify`.
- `blob-verification-refusals` is a million blobs, half a gigabyte and about a
  minute of hashing in release. Its per-commit gate stays at ten thousand with
  its assertions derived from the blob count it was given, and **the comparison
  at the published scale goes on the nightly beside the rollback job**, keeping
  its report as an artifact. That is the rollback precedent applied rather than
  re-argued.

## Policy applied

**Determinism (RFC 0004).** Two edges. First, the registry publishes floats and
the lint cannot see them; the fix is the fixed point with its scale in the name
that `FORBIDDEN_TYPES`' own reason names (`xtask/src/main.rs:109`). Second, the
new code in `xtask` is inside the crate the determinism lint checks, and
`CLAUDE.md` records reaching for `HashMap` there as a scar: `thresholds_in` and
`measured_rows` already use `BTreeMap` and the extension keeps them. Nothing here
reads a clock, draws randomness or observes ordering, so nothing reaches
`f_env::Env` and nothing needs a `DETERMINISM_ALLOW` entry — the shape-C file
reads are `std::fs::read_to_string` on a path derived from `CARGO_MANIFEST_DIR`,
which is the shape `zone/tests/cycle.rs:766` already uses.

**The frame (RFC 0001).** Nothing here is `unsafe` and nothing here is under
`abi/`, `ring/` or `kernel/`. The edits are in `xtask/`, three test targets and
four registry files.

**The licence boundary (RFC 0003).** Untouched. No workload here imports
`third_party/` and none gains a ring.

**Evidence.** See *Evidence*. The one thing worth saying under policy is that the
obvious implementation is forbidden by the intent's own first constraint: a row
that goes red when something first compares it is not repaired by moving the
bound. RFC 0064 is the precedent — three RFCs looked at `claims/0017`'s 786 432
and every one of them left it where it was.

**Decisions (RFC owed).** One RFC, carrying two decisions that are both reversals
of text already in the tree:

1. *A published bound is an integer with its scale in its name.* This adds a rule
   to `claims/README.md` and re-spells five rows across `claims/0001`, `0004` and
   `0016` without moving one of them.
2. *A row no checkout can measure is `owed` and names the task that owes it.*
   This reverses `claim_compare`'s stated rule that a row nothing prints is
   always a finding (`xtask/src/main.rs:15055`–`:15066`).

Its *What would reverse this* section is the one that matters: an `owed` row that
outlives the task it names, or a second claim acquiring an `owed` row for a
boundary that could have been measured, is the escape hatch becoming the norm —
and the answer then is that `owed` expires, so a row owed against a closed task
is a red build.

`docs/rfc/0069` (`draft`, untracked at the time of writing) adds an `evidence`
key and a sixth rule to `claims/README.md`. It is a sibling, not a conflict: it
says what kind of observation a green result is, and this says whether anything
compared. Whichever lands second writes its rule as the seventh. The RFC number
is *the next free one at landing* — `docs/rfc/` runs to 0072 today, and this
tree's rule is that a number taken in a spec is a number a concurrent landing
takes first.

## Not in scope

- **Moving any bound.** If a comparison turns a claim red, the finding is
  reported and the number stays. That is a separate RFC with a measurement in it,
  owned by whoever measures.
- **Measuring `device_bytes_per_app_byte` at its own boundary.** `E2-P10`
  (`TODO.md:1187`) owns the QEMU 8 image change, the guest boot of
  `user/objects` and the tuned-Linux comparison. This spec makes the debt visible
  on every run of the claim's command; it does not pay it.
- **Replacing thresholds with change-point detection.** `claims/README.md` rule 5
  says thresholds "either miss real regressions or fire until everyone mutes
  them", and `E2-P09` (`TODO.md:1183`) is that task. A bound nobody compares is
  worse than a threshold, and this is the cheap repair that makes the expensive
  one worth doing.
- **The other twenty-six claims' routes.** Two float rows outside `claims/0016`
  (`claims/0001:43`, `claims/0004:61`) are re-spelled because the rule is
  registry-wide, but no other claim's route changes here.
- **`docs/design/`.** No number is published or moved, and `claim_references`
  finds no document rendering any of these keys, so no page re-renders.

## Evidence

- **A named test of the mechanism.** `gap_holds_under` has a fixture because "a
  check that has never failed is indistinguishable from a check that cannot"
  (`xtask/src/main.rs:493`). The `owed` reading and the integer-bound parser get
  the same: a fixture claim whose rows are one green, one red, one owed, one owed
  against a task `TODO.md` does not have, and one bound spelled `1.5` — with the
  last two required to be findings.
- **Mutation, not argument.** `E2-P02` checked each new gate on `claims/0018` by
  moving the bound, watching the run go red, and reverting (`TODO.md:1144`). The
  same for all twenty-five rows here: moving any published bound by one, in the
  claim file, makes that claim's command go red naming that row, and breaking the
  measurement instead does the same. That is the intent's stated observable, and
  it is a check somebody performs once and records rather than a claim.
- **A lint that makes it unforgettable.** `lint_reproduce`
  (`xtask/src/main.rs:16690`) already refuses a claim with no `ROUTES` entry
  (`:16707`) — "its command runs nothing". It gains the next sentence: a claim
  whose route compares no table, and a `[threshold]` bound no parser can read,
  are both findings. Without this the four are fixed and the fifth is one new
  claim away.
- **No new claim.** Nothing here measures anything. The evidence that this worked
  is that four existing claims stop being able to be green while wrong.

## Risks and reversal

**Most likely to be wrong: a row goes red the first time anything compares it.**
That is the outcome the intent predicts — "if wiring a claim up turns it red, the
red is the finding" — and it has happened twice on this branch already. The
comfortable candidate is `claims/0016`'s `modelled_device_bytes_per_app_byte` at
1.1336 against `max = 1.5`; the uncomfortable one is `claims/0018`'s
`forced_cuts_per_ten_thousand_uniform = { max = 1000 }` against a measured 166,
which is comfortable now and is the row RFC 0061 named as a reversal condition.
The observation that says this went wrong is a plan that arrives with a bound
moved in it.

**Second: shape C makes three test crates read files.** `index/tests/query.rs`,
`blob/tests/chunker.rs` and `zone/tests/cycle.rs` would each read a
`claims/*.toml` at run time through `CARGO_MANIFEST_DIR`. The precedent states
its own limit and this inherits it: it is textual, so a bound moved to another
file or spelled differently goes past it (`zone/tests/cycle.rs:762`). The failure
mode is a test that cannot find the claim and passes; the answer is that a
missing file or a missing row is a failure in the test and never a skip, which is
what `the_bound_is_the_drivers_chain_length` does at `:766`.

**Third: `owed` becomes the way to make a run green.** This is the honest risk,
and it is why the rule requires a task id that exists and why the RFC's reversal
condition is an owed row outliving its task. The weaker design — a row simply
absent from the table — is what `claims/0016` does today and is the state this
whole spec exists to end.

**Fourth: a claim gets a comparing route and keeps its silent gate.** `cargo
xtask test` running `blob/tests/chunker.rs` is not `cargo xtask claim
chunk-size-distribution`, and a contributor who runs only `verify` sees the
shape-C assertions and not the comparison. That is deliberate and is the cost
side of the local-loop rule. It becomes wrong if any of these four ever gains a
row only the claim's command can produce and a contributor can break.

## Decisions taken on the originator's behalf

1. **Twenty-five compared, two owed**, rather than twenty-seven compared. The
   split falls exactly where `claims/0016`'s own header puts it.
2. **`owed` is a key on the threshold row**, not a per-claim status and not a
   separate table. A per-claim status already exists and means something else.
3. **The four hand-copied bounds in `blob/` and the two in `index/` are read from
   the claim (shape C) rather than deleted in favour of the comparison.**
   Deleting them would retire per-commit assertions `E2-P02` paid for on purpose.
4. **The integer-bound rule is registry-wide**, so `claims/0001` and
   `claims/0004` are re-spelled too, although neither is one of the four.
5. **One RFC, not two.** The unit rule and the `owed` state are one argument —
   what a `[threshold]` row is allowed to be — and splitting them would make two
   entries that cite each other.
6. **`blob-verification-refusals`' comparison goes nightly**, following the
   rollback precedent, rather than into `verify`.

## Open questions carried forward

- **Who notices a red night.** The rollback job keeps a report as an artifact and
  opens no issue, on the argument that a deterministic failure reproduces from
  the claim's command (`.github/workflows/nightly.yml:644`). `TODO.md` `A-06`
  says a muted job is a deleted job with extra steps. This spec follows the
  precedent and does not re-open it; the originator should say whether one more
  scheduled claim is the point at which that argument stops holding.
- **Whether `owed` should carry a date as well as a task.** A task id says who
  owes it; a date would say for how long, and would make *the escape hatch
  becoming the norm* observable rather than arguable. Not specified here because
  `TODO.md` ids are permanent and dates in this tree have so far lived in the RFC
  that set them.
- **`claims/0004`'s `ratio_resolve_virtual_vs_fixed` and `claims/0001`'s
  `ratio_vs_baseline`** are re-spelled by rule 1, but neither claim's route
  compares anything either — both are `pending` latency claims waiting on a
  machine. Whether `owed` is also the right answer for *a whole claim* that waits
  on a runner class, rather than for a row, is the same question one level up and
  is not answered here.
