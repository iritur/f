# RFC 0057: A proof links the shipped crate, and says what it reached

- Status: accepted
- Date: 2026-09-04
- Affects: `ring/proofs/` (new), `ring/Cargo.toml` (five of its nine deliberate
  defects acquire a second instrument and four are declared out of reach),
  `Cargo.toml`'s `exclude`,
  `xtask/src/main.rs` (`prove`, `lint-proofs`, `unsafe`),
  `.github/workflows/nightly.yml`'s `prove` job;
  `docs/rfc/0053-a-proof-compiles-the-shipped-file-and-states-its-bound.md`,
  whose `#[path]` arrangement this deliberately does *not* copy;
  `docs/rfc/0017-a-kernel-that-can-be-built-wrong.md`,
  `docs/rfc/0046-a-hostile-peer-is-generated-and-a-hang-is-a-count.md` and
  `docs/rfc/0048-an-entry-is-generated-by-its-structure-and-kept-by-its-coverage.md`,
  whose defects this gives a second instrument to

## Decision

`f_ring`'s promise — *never panic on anything a peer wrote* — is proved rather
than sampled, in `ring/proofs`, by a bounded model checker, over a mapping every
byte of which a solver chose. Six things about how, and a seventh about what it
does not reach.

**The proofs link the shipped crate; they do not recompile a file out of it.**
RFC 0053 compiled `kernel/src/cap.rs` a second time through `#[path]`, and was
right to: that file lives inside a bare-metal binary crate that cannot be built
for the host, so there was no way to reach it except to compile it again against
stand-ins. `f-ring` has no such problem. It is an ordinary `no_std` library
whose own tests already run on the host, so `ring/proofs` takes it as a **path
dependency** and the question RFC 0053 spends a section on — *does the proof
still reach the file the kernel ships* — does not arise, because there is no
second copy and no second compilation for one to drift from. Same guarantee, one
fewer mechanism. The armed half below is therefore doing a different job here
than it does there, and this RFC says which.

**The fixture is a region of bytes, not a struct of fields.** `ring/src/mapping.rs`
opens by naming the trap: everything else in `f_ring` takes borrowed Rust
references and is correct partly because the borrow checker already proved the
regions do not overlap, are the length they say and are aligned — so a `Channel`
assembled out of fields a harness owns *can only ever be laid out correctly*.
A proof over that shape would be a proof about a channel the hostile case never
produces. So `peer::Region` is one aligned run of `REGION` bytes, every one of
them symbolic, handed to the real `Mapping::adopt`; the header, both pairs of
cursors, the flags word, the index ring, both entry arrays and the arena are
then the *same* symbolic bytes, standing in the relationship they stand in when
a peer holds the far end. The cursors are all 2³² values. The slot numbers in
the index ring are all 2³². The entries and the arena are arbitrary.

There is exactly one exception, and it is named rather than left to be
discovered: `draining_an_arbitrary_channel` builds its channel out of fields.
The Consequences section says what that costs and what pays it back.

**The bound is the mapping, and it is one number.** `peer::REGION` is 640 bytes,
which `f_abi::layout` turns into a ring of **one or two** entries — the arena
lands at 512 for a ring of one and at 576 for a ring of two, and every larger
`ring_size` a header could claim is refused by `Layout::adopt` for not fitting.
Nothing else in the fixture is bounded. As in RFC 0053, the reduction is not
left as an argument: `wide-ring` grows the region to hold a ring of eight and
`cargo xtask prove`'s second pass runs every harness that does not pay for the
size again under it.

**A cover that cannot be satisfied is a failed proof.** This is the part that is
new relative to RFC 0053 and it is the answer to the question this repository
asks of every green result — *what input would make it green while the property
was false*. For a proof over arbitrary bytes the answer is precise and
embarrassing: bytes that never get past the first check. A harness whose
`Mapping::adopt` always refuses proves nothing about `pop`, costs nothing to
write, cannot fail, and is indistinguishable in a report from one that proves
everything. `kani::cover!` is the instrument: an unsatisfiable cover is a
**failed** verification, so a fixture that has stopped reaching the code it is
about goes red rather than quiet. Every harness carries covers for each answer
it can produce, and `cargo xtask prove` prints the satisfied count beside each
one.

**And the enforcement is `xtask`'s, because the checker's does not exist.** This
is the correction that matters most in this RFC and it was found by a reviewer
building the counterexample rather than by anybody arguing about it: Kani 0.67.0
— the version in the `full` image this job names — treats an unreachable cover
as information. Given a harness with one satisfiable cover and one an `assume`
makes unreachable, it prints

```
 ** 1 of 2 cover properties satisfied (1 unreachable)
VERIFICATION:- SUCCESSFUL
```

and exits 0. `cargo kani --help` in that image offers no
`--fail-uncoverable`; there is no flag to buy the behaviour. So for a while the
rule above was stated in five places — two module comments, `xtask`, this RFC
and the nightly's own job comment — and mechanised in nowhere, which is exactly
the shape CONTRIBUTING R01 calls worse than a rule honestly listed as review:
a check somebody believes is happening.

The regression it exists to catch was then run rather than described. Setting
`peer::REGION` to 64 — which is what a change to `f_abi::layout`'s offsets would
do to it indirectly, leave it too short for any ring — makes `Mapping::adopt`
refuse every region, so `popping_an_arbitrary_entry` returns before it ever
reaches `pop`. Kani calls that run `VERIFICATION:- SUCCESSFUL`. With
`cover_check` in it, `cargo xtask prove popping_an_arbitrary_entry` instead
prints

```
xtask: `popping_an_arbitrary_entry` verified, and that verdict says nothing.
5 cover properties, so 5 of them cannot be reached at all:
`0 of 5 cover properties satisfied (5 unreachable)`
```

and goes red. That is the whole difference, and it is the answer this task owes
the repository's standing question.

`xtask`'s `cover_check` now reads the count out of every report and refuses when
`satisfied != total`, when the summary line is missing from a crate that owes
one, and when the line is in a shape it cannot parse — because a count that
cannot be read must not be read as a count that is fine. `ProofCrate::covered`
is which crates owe the line: true for `ring/proofs`, false for `kernel/proofs`,
whose harnesses quantify over handles with no fixture between them and the
table. Two unit tests hold it, over a report captured from the checker in the
image itself rather than written from memory, and a third walks
`RING_PROOF_HARNESSES` and refuses a harness that carries no cover at all — the
same vacuum arriving a second before a twenty-minute run instead of after one.

**The harnesses are compiled by the pinned toolchain too, not only by Kani.**
`mod proofs` is not behind `#[cfg(kani)]`; only the `kani::proof` and
`kani::unwind` attributes are, and `kani::any`, `kani::assume` and
`kani::cover!` have a shim in `lib.rs` that answers the least interesting value
and evaluates nothing. The reason is what `lint-proofs` is *for*: it exists so
that `f-ring`'s API moving under this crate is a fifteen-second failure in
`cargo xtask lint` rather than a discovery twenty minutes into a nightly, and
with the harness module compiled out that build saw the three trait
implementations in `peer` and none of the calls — `Consumer::pop`,
`Mapping::adopt`, `Table::register`, `execute`, `BufferSet::carve` — which are
the whole of what could move. The mitigation was aimed at the half that could
not rot. It found a dead import the moment it stopped being.

The cost is a shim that has to keep up with the checker's surface: a harness
drawing a type the shim cannot answer for is a compile error here, which is
deliberate — see `kani::Zeroed`'s own comment — and a *new* `kani::` facility
used in a harness needs a line in the shim before the ordinary build passes.
That is the right price, because the alternative is the build not covering the
file that does the work.

**An oracle does not re-derive its expectation by calling the code under test.**
`adopting_an_arbitrary_layout` states an equivalence — `Layout::adopt` succeeds
*exactly* when the header is one this build would have written — and its first
version wrote the right-hand side as `header.is_valid() && ...`, which is
`adopt`'s own body. That equivalence cannot fail for the class of defect it is
presented as catching: weaken `ChannelHeader::is_valid` to admit a non-zero
reserved word, which is R04's fail-closed half, and both sides move together
while the harness stays green and its docstring stops being true. So
`proofs::this_builds_header` writes the five clauses out from
`abi/src/lib.rs`'s *field documentation*, and spells one of them differently on
purpose — `count_ones() == 1` where `is_valid` says `is_power_of_two()`. Two
spellings of one property is what makes a comparison a comparison.
`negotiating_with_an_arbitrary_peer` uses the same oracle for the same reason.
The standing cost is stated at the function: a sixth clause added to `is_valid`
and not added there fails the harness on the *clean* build, saying a header this
build wrote was refused, which is the direction this should rot in.

## Context

What was true when this was written.

`E1-P04` had just spent **a billion** drawn hostile operations against exactly
these paths and found nothing, and `E1-P05` a further hundred million
structure-aware entries. That is the strongest possible statement of the gap
this task closes and it is worth being precise about why: a billion samples over
a space of 2³² cursors × 2³² slot numbers × 2⁵¹² of entry bytes is not a small
fraction of the space, it is a fraction with no useful lower bound at all. The
fuzzers are not weak instruments — they found the shape of the properties, they
carry the coverage floors `claims/0008` publishes, and they run at ring sizes
and operation counts no checker will ever reach. What they cannot say is *there
is none*.

`E1-P07` had just built `kernel/proofs`, the `prove` verb, the `full` image
target and the nightly `prove` job, and its report ended with a note saying this
task was now cheap for exactly that reason. That turned out to be true of the
apparatus and false of the fixture: the capability table is a Rust structure
driven through Rust methods, and a channel is a range of shared addresses with
an obligation to disbelieve all of it. The apparatus was reused unchanged. The
fixture had to be built from the mapping down, and the `cover` rule above came
out of the first version of it, which verified `popping_an_arbitrary_entry` in
under a second because nothing it drew was ever adopted.

The alternatives that were live:

- **A second `prove` job in the nightly, beside E1-P07's.** Rejected by a check
  E1-P07 itself wrote: `proof_schedule` requires that exactly **one** job depend
  on `image_full`, because that image carries Kani's own rustc and every job
  waiting on it is a job the checker's toolchain can take down. A second job
  would have been a second dependant, which is the check firing correctly on the
  first thing that tried to do the thing it was written against. So the ring's
  proofs are a second phase of the one verb and the one job, and the count of
  crates `prove` covers is printed rather than described.
- **`#[path]`, for symmetry with `kernel/proofs`.** Rejected above. Symmetry is
  not a reason to carry a mechanism whose whole justification is a constraint
  the second case does not have — and the cost of carrying it is real: a
  `#[path]` build needs a stand-in for every module the file reaches, and
  `f_ring` reaches four of its own.
- **A struct-of-fields fixture**, which is what `ring/src/lib.rs`'s own unit
  tests use and what the first draft here used. It is faster to check and it
  cannot express the thing being proved. Kept for the harnesses where there is
  no mapping at all — `Layout::adopt`, `Request::read`, `Name::read`,
  `SetId::from_completion` — because for those a "fixture" is just the argument
  list and the input really is unbounded, mapping length included.
- **Proving the whole ring at a realistic size**, 64 or 256 entries. A checker
  unrolls a slice rather than summarising it, so this is the difference between
  seconds and not terminating. The honest version of the trade is in the
  bound above and in `peer::REGION`'s own comment.
- **Requiring all nine of `ring/Cargo.toml`'s defects to break a proof.** Five
  do. Four cannot, and for two different reasons that are worth separating.
  Three — `mutate-relaxed-submission`, `mutate-relaxed-completion`,
  `mutate-no-doorbell-fence` — are memory-ordering weakenings, and CBMC does not
  model a weak memory model at all, so a proof here is *insensitive* to them by
  construction. The fourth, `mutate-reusable-slot`, is not an ordering question:
  observing it needs a slot at `SetId::RETIRED_GENERATION`, which is sixty-five
  thousand five hundred and thirty-four retirements of one slot, and a bounded
  checker unrolls that loop rather than summarising it. Arming any of the four
  and requiring a failure would have been the exact mistake `MUTATIONS` records
  nearly making: a run that fails for the wrong reason satisfies an exit status
  and proves nothing. They are named in `RING_PROOF_BLIND` instead, which is a
  declared quantity with a test in both directions — a defect in the manifest
  and in neither list is a red build, and so is a name in the list the manifest
  no longer declares. E0-P16 is still the task that owes the first three an
  instrument, `ring/tests/entries.rs` is where the fourth is caught, and this
  RFC does not let a proof stand in for either.
- **A `site` that is always a location in the shipped file.** RFC 0053's armed
  phase requires the failing check to be located in `cap.rs`, which works
  because the defect it arms produces a *fault*. Two of the five here do —
  `mutate-trusted-slot` faults in `Consumer::pop` and `mutate-believed-header`
  panics in `Mapping::adopt` — and three do not: an ignored flag, an unbounded
  drain and a lenient index each produce a **plausible wrong answer** and
  nothing in the process faults on one. That is not a weakness of the defects,
  it is the whole reason RFC 0048 needed oracles. So `site` is a substring that
  a failing check must carry in its location *or* its description, each entry
  says which of the two it is, and the three answer-shaped defects are matched
  against the harness assertion that states the property they break. The
  assertions they are matched against are written `assert!(cond, "literal")`
  rather than `assert_eq!`, because an `assert_eq!` puts the values in the
  description and the sentence stops being a fixed string.

  One of the two fault-shaped defects turned out not to give a location either,
  and it is worth recording because the next person will hit it: a failing
  `expect` is reported by Kani as `std::result::unwrap_failed` in `core`, with
  the message replaced by *"This is a placeholder message; Kani doesn't support
  message formatted at runtime"*. `Mapping::adopt` appears in that report only
  on **passing** checks. So `mutate-believed-header` is matched against
  `unwrap_failed`, and the sentence that makes that unambiguous — *the harness
  it breaks contains no `unwrap` or `expect` of its own* — is a test rather than
  a comment, because it is a sentence a later edit can quietly falsify.

## Consequences

Makes easy: saying *there is none* about a panic on the four paths the task
names, plus the six registration and buffer-ownership entry points RFC 0028
added afterwards, plus the two window accessors RFC 0033 did. Adding a path to
that list is one harness and one row in `RING_PROOFS`. Finding out that a
refactor broke the reach oracle costs a nightly rather than a fuzzing campaign.

Makes hard: changing the ring's geometry constants without noticing this crate.
`peer::REGION` is arithmetic against `f_abi::layout`'s offsets, so moving
`SQ_INDEX` or the header's size changes which ring sizes the proofs admit —
silently, in the direction of admitting fewer. The guard is `proofs::reached`,
called by each of the four region harnesses: two covers, *a ring of one entry is
admitted* and *a ring of two entries is admitted*, which are the two sizes the
narrow bound is documented as admitting and are both still reachable under
`wide-ring`. A region that shrank to admit only a ring of one now fails the
second of them. This RFC previously claimed that guard while no cover named a
ring size at all — every cover named an *answer* — which was a mitigation that
existed in a sentence, the same failure as the cover rule above and found in the
same review. It is still not a complete guard: a ring size the fixture can no
longer reach that nothing names here would pass.

Forecloses: nothing. Both fuzzers still run, at sizes and counts these proofs do
not reach, and `docs/TESTING-STATUS.md`'s honest position is unchanged in shape —
L3 now has an occupant for two crates rather than one, and the sentence that
matters is still that a proof and a fuzzer answer different questions.

Four bounds beyond the region are worth naming here rather than only at the
constants, because each is a narrowing somebody could otherwise read past:

- **`proofs::ARENA` is eight bytes**, so `write_serial`'s chunking loop makes at
  most one pass and the multi-pass path is checked rather than proved.
  `ring/src/lib.rs`'s own fixture sizes its arena at `CHUNK * 2 + 16` precisely
  to exercise that loop.
- **`proofs::REGISTERED` and `proofs::SETS` bound a registration's geometry**, so
  a buffer's stride is a small power of two. The index a peer presents stays all
  2^32 — which is the half that matters, because the index is the peer's and the
  defect is an index past the end. The reason for the split is that a
  thirty-two-by-thirty-two-bit multiplication of two symbolic operands is one of
  the most expensive things a SAT solver can be handed: with both free the
  harness did not finish in thirteen minutes, and with the stride bounded it is
  nine seconds.
- **`draining_an_arbitrary_channel` builds its channel out of fields rather than
  out of a region**, and it is the one harness in the crate that does. Adopting a
  region runs `ChannelHeader::is_valid`, whose four-word `_reserved` comparison
  is a sixteen-byte `memcmp` a checker unrolls seventeen times; `kani::unwind` is
  one number per harness, so seventeen then also bounds `Service::drain`'s loop,
  and each of those seventeen unrolled iterations carries an inlined
  `f_ring::execute`. That is twenty minutes, and without a header it is five.
  What the struct fixture takes on trust is the *layout* — which is precisely
  what the four region harnesses prove, over every byte, at both bounds.
- **The registration harnesses run at a depth of one.** `registering_from_an_
  arbitrary_entry`, `resolving_an_arbitrary_buffer_name`, `retiring_an_arbitrary_
  set` and `both_transports_refuse_a_name_of_the_wrong_kind` each build a fresh
  `Table<2>`, make at most one registration, and then perform one further
  operation — with the entry, the id, the index and the geometry all the
  solver's. So the quantifier is over *inputs* and not over *histories*, and at
  that depth `assert!(table.live() == 0)` on a refusal arm is a tautology about
  a table that was empty a line earlier. A defect needing two registrations — a
  leaked translation on the second, a half-registration visible only when the
  lowest free slot is not slot zero — is outside them. It is the same cost as
  the ring size, because a checker unrolls a second operation rather than
  summarising it, and it is the same argument `RING_PROOF_BLIND` makes about
  `mutate-reusable-slot` at sixty-five thousand. `ring/tests/entries.rs` keeps a
  ledger across a whole run and is the instrument that does cover histories.
  This bound was unstated in the first version of this RFC, which is how a
  reader would have read the sentence *a translation is outstanding exactly when
  a slot is live* as being about a table rather than about one operation on
  one.

The honest costs, stated where the numbers will be rather than in a rebuttal:

- **`cargo xtask prove` roughly doubles.** It was about twenty minutes for the
  capability properties; the ring's seventeen harnesses are the same order, and
  two of them — `executing_an_arbitrary_entry` and
  `draining_an_arbitrary_channel` — are most of it, at about six and five
  minutes each on the development container. That is a nightly's budget and not
  a pull request's, which is why the job was already on a schedule. It is
  context and not a claim: `bench/src/lib.rs` refuses to record a container wall
  clock and that refusal is right, so these are numbers to size a job by and not
  numbers to publish.
- **The ring is one or two entries**, or one to eight under `wide-ring`. A
  defect that needs nine queued entries to appear is outside these proofs.
  `ring/tests/hostile.rs` runs the same code at larger sizes for a billion
  operations, and the two instruments are not substitutes for each other.
- **Nothing here says anything about ordering.** CBMC is a sequential checker.
  Every `Release`/`Acquire` pair in this crate is outside these proofs entirely,
  and the litmus suite plus the AArch64 job remain the only instrument on them.
  This is the largest gap in the file and it is the first thing a reader should
  be told, which is why `RING_PROOF_BLIND` is printed by `lint-proofs` on every
  local run rather than filed in a document.

## What would reverse this

- **A defect the small ring hides.** If a bug is found in `Consumer::pop`,
  `Collector::take` or `Service::drain` that needs a ring of four or more, the
  bound stops being a cost and becomes a hole, and the answer is a checker that
  summarises a loop rather than a larger `REGION`.
- **A cover that becomes unsatisfiable for a legitimate reason.** The rule above
  assumes that every branch a harness names is reachable in the shipped code.
  A deliberate narrowing — an opcode withdrawn, a refusal made unconditional —
  would make a cover fail on a correct build, and the fix is to delete the cover
  in the same diff rather than to weaken the rule.
- **`ChannelHeader::is_valid` gaining a clause.** `proofs::this_builds_header`
  is a hand-written second statement of it, deliberately not a call, so the two
  must be changed together. The day they are not, the clean build fails saying a
  header this build wrote was refused — which is the failure being asked for,
  and the reversal is only that somebody must go and add the clause rather than
  wonder why the proof went red.
- **Kani learning to fail on an unreachable cover.** If a later release grows a
  `--fail-uncoverable` or makes it the default, `cover_check` becomes a second
  opinion rather than the only one, and the honest move is to keep it — it also
  catches a *missing* summary line and a report format that moved — while
  deleting the paragraph above that says the checker cannot.
- **`f-ring` acquiring a dependency Kani cannot build.** It has one today,
  `f-abi`, and both are `no_std` and dependency-free. The day either takes a
  crate with a build script or platform intrinsics, this arrangement pays RFC
  0053's cost after all and the `#[path]` question reopens.
- **E0-P16 landing.** A model checker for the ordering would take over the three
  defects named in `RING_PROOF_BLIND`, and the sentence *nothing here says
  anything about ordering* would stop being a permanent gap and become a
  division of labour between two checkers. That is the outcome this RFC expects
  and does not assume.
- **`f_ring::execute` growing a third opcode.** The two it has carry no
  capability and read no table, which is why `executing_an_arbitrary_entry` can
  be one harness. An opcode that resolves a handle would put the capability
  table inside this harness's state space, and the split `kernel/proofs` had to
  make — one harness per operation, because an assertion does not cut a path —
  would arrive here too.
- **The prove job outgrowing its schedule.** Two harnesses are most of the ring
  half and both are `execute` inlined into a loop. If a nightly stops fitting,
  the knob is to move the wide passes to a weekly rather than to delete a
  harness — the wide pass is the thing that turns a bound from an argument into
  a check, and a bound nobody re-checks is the shape this whole file exists to
  avoid.
- **Verus at phase 02.** RFC 0053's last reversal condition, and it applies
  identically: a proof system that can state a *specification* rather than a
  bound would replace the whole of this with something stronger, and the
  harnesses here would become the regression suite for it.
