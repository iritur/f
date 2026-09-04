# RFC 0053: A proof compiles the shipped file, and states its bound

- Status: accepted
- Date: 2026-09-04
- Affects: `kernel/proofs/` (new), `Cargo.toml`, `docker/Dockerfile`,
  `docker/README.md`, `xtask/src/main.rs`, `.github/workflows/nightly.yml`,
  `docs/test-taxonomy.md`'s capability paragraph, `docs/TESTING-STATUS.md`'s L3
  row, `E1-P07`, `E1-P12`, and `claims/`, which this deliberately does not
  trigger

## Decision

The five capability properties get a **bounded proof over the file the kernel
ships**, not over a copy of it and not over a model of it. `kernel/proofs`
compiles `kernel/src/cap.rs` a second time — through `#[path]`, so there is one
file — against three stand-ins for the parts of the kernel a model checker
cannot follow, and states the five properties over *arbitrary* handles and
*arbitrary* rights within a table whose size is bounded and whose contents are
built by running real operations.

Three things follow from that sentence and each is a decision somebody could
disagree with:

**The crate is outside the workspace.** Kani installs a rustc of its own. RFC
0022 already decided that a checker's toolchain requirement is the checker's
business; this is that decision applied to a tool that takes it further than
RustMC did. A workspace member would be built by `cargo xtask test` under the
pin, which buys nothing and breaks on the first day the two compilers disagree.

**The page a table buys is smaller in the proof than in the kernel.** The
stand-in `mem` sets `FRAME_SIZE` to 256 bytes, so a page of slots is eight
rather than a hundred and twenty-eight. That is the bound; it is stated at the
constant in `kernel/proofs/src/mem.rs`; and it binds the one harness that grows
a table rather than all ten — which is not asserted, because the other nine are
run a second time at the kernel's own 4096 and have to verify there too.

**A proof that cannot fail does not count.** `cargo xtask prove` runs the
harnesses twice: once clean, where all ten must verify, and once with
`mutate-unchecked-index` armed, where `total_lookup` must fail *and a check
that failed must be located in `cap.rs`*. The second half of that is stated
carefully because the first draft of it was not: Kani prints a `Location:` for
every check it emits, so a report over this crate names `cap.rs` on a run where
nothing failed, and a guard reading the whole log was one every possible armed
run satisfied. It reads the failing checks. That is RFC 0017's argument, made for the third time and on the
one property that has never had a fixture.

## Context

`E0-P08` left the five properties as a negative suite that runs at every boot:
a real table, five tables broken on purpose, and — for the fifth property, which
cannot have a fixture because a table that panics takes the machine down — a
kernel built with the bounds check removed and required to die. That suite is
good evidence and it is the wrong *shape* of evidence for one sentence in
`docs/design/proving-ground.html`: **capability soundness is bet 04's entire
content, and if it is wrong nothing else in the system matters.**

The gap is the quantifier. `properties::forged` sweeps every slot index against
eight generations and reports that none of the unissued ones resolved. What it
has established is *no handle we tried resolved*. What bet 04 needs is *no
handle resolves*. Those differ by a solver.

Three arrangements were live.

**Run the checker on `f-kernel` itself.** Rejected, and it is worth saying why
rather than only that it is impractical. The kernel is a `no_std`, `no_main`
binary with x86-64 assembly in it, twenty-four thousand lines of which the
capability table is twenty-six hundred. A bounded model checker's unit of work
is a crate, so this would hand a solver the ACPI parser and the buddy allocator
in order to ask a question about a lookup. The answer would not be a stronger
proof; it would be a proof that never finished.

**Copy `cap.rs` into a crate the checker can build.** Rejected, and this is the
option that costs nothing today and is therefore the one to argue with. A copy
proves something true of a file nobody runs. It drifts, and the *first* thing
it stops containing is the thing this whole arrangement exists to detect: the
`#[cfg(feature = "mutate-unchecked-index")]` in `Table::resolve`. A proof over a
copy would go on verifying after the shipped lookup had been broken, which is
precisely the failure `cargo xtask mutate` was written to make impossible one
layer down.

**Compile the shipped file against stand-ins.** Accepted. `#[path]` means there
is one `cap.rs` and the checker reads it. The cost is three stand-ins, and it is
paid where it can be seen: `mem` (a page size, an address the table never
dereferences, an allocator that refuses), `percpu` (a shard that holds nothing,
because no proof goes near the per-core static) and `pages` (a `Backing` whose
memory the harness owns). Each is a file with a comment saying what it is not.

### What was measured before this was accepted

Two things, because both could have made this unworkable rather than merely
awkward.

**Kani installs, and it brings a compiler.** `cargo install --locked
kani-verifier` compiles in 15 s under the pinned nightly, and `cargo kani setup`
downloads a 483 MB bundle and installs `nightly-2025-11-21` — rustc
1.93.0-nightly, 573 MB — as a rustup toolchain beside the pin. `rust-toolchain.toml`
does not move, exactly as RFC 0022 said it would not. This is the second tool to
require that arrangement, which is the first evidence that RFC 0022 was a
decision rather than a special case.

**The shipped file compiles under both.** `kernel/proofs` builds under the
pinned nightly as well as under Kani's — clean, in all three feature
configurations — which is what makes "the stand-ins still match `mem`" a thing
an ordinary `cargo build` can find out rather than something the schedule
discovers. That sentence was a plan when it was first written and is now
`cargo xtask lint-proofs`, in `lint` and therefore in `verify`: three
`cargo check`s and a `cargo fmt --check`, under fifteen seconds, no checker
involved. A mitigation stated as a mechanism has to be one (R01), and this is
the whole of what it took.

**And the proofs terminate — nine of the ten quickly, and the tenth not at
all.** Measured on the four-core development container, one harness per run:
`unnamed` 8 s, `forged` 8 s, `forged_across_a_process` 15 s, `stale` 31 s,
`narrowing` 144 s, `total_lookup` 14 s, `total_derive` 132 s, `total_revoke`
43 s, `total_frame_side` 28 s, `total_bought` 252 s. The armed run of
`total_lookup` fails in 3 to 11 s. `cargo xtask prove` end to end — ten
harnesses, nine of them a second time at 4096, and the armed run — is about
twenty minutes in the `full` image (1156 s and 1178 s on two runs). Every figure
here is a container wall clock and therefore context rather than a claim:
`bench/src/lib.rs` refuses to record one, `docker/README.md` says why, and none
of these reaches `claims/`. They are here so that a harness which has become ten
times slower is recognisable as such. The tenth harness is the subject of *The harness that was written, run,
and taken out*, below.

**And one measurement that changed the design rather than confirming it.**
Totality started as a single harness listing all nine operations and did not
finish in twenty-five minutes; split by operation it is the four above, in under
four minutes together. The reason is a fact about bounded model checking rather
than about this table — an assertion does not cut a path, so nine lookups over
one symbolic handle multiply rather than add — and it is written at the
harnesses because the next person to add an operation will otherwise add it to
whichever harness is nearest.

### The bound, stated rather than left to be inferred

Every loop in `cap.rs` runs to `Table::capacity`, which is
`TABLE_SLOTS + grown × SLOTS_PER_PAGE`, and `SLOTS_PER_PAGE` is
`FRAME_SIZE / size_of::<Slot>()`. At the kernel's page size that is 128, a
table that has bought one page is 160 slots, and the revocation walk is
`160 × 161` iterations. A bounded model checker unrolls a loop rather than
summarising it, so that is twenty-six thousand copies of the loop body with
symbolic slot contents in each. The proofs would not be slow; they would not
terminate in any budget worth having.

So the stand-in sets `FRAME_SIZE` to 256 and a page is eight slots.

**The reduction binds one of the ten harnesses, and that is checked rather
than argued.** `FRAME_SIZE` reaches `cap.rs` in exactly two places — the
arithmetic `Table::retype` performs on an untyped watermark and the page
`Table::grow` buys — so a harness that never grows its table cannot depend on
it. Nine of the ten never grow. Leaving that as an argument would be the thing
this tree does not do, so `cargo xtask prove` has a second pass: those nine are
run again with `wide-page`, which is the kernel's own 4096, and must verify both
ways. If the independence is real they pass twice; if it stops being real, the
second pass is where that is found.

What the ten buy at the reduced size:

- **The paging arithmetic is proved.** Which page an index falls in, that
  `Table::at` answers `None` past what was paid for rather than reading memory,
  that a bought slot's first generation comes from the table's floor and not
  from one, that `capacity` and not `TABLE_SLOTS` is what every bound is
  against. All of that is structure, and it is the part growth actually changed.
- **The handle is not reduced at all.** Every harness answers
  `Handle::from_bits(kani::any())` — all thirty-two bits, so all four billion
  handles, including every index above the table and every generation nobody
  issued. This is the quantifier the negative suite does not have, and it is
  intact at both page sizes.
- **The rights lattice is not reduced at all.** `narrowing` quantifies over all
  256 held bitmaps against all 256 asked ones, undefined bits included, and it
  is one of the nine that run at 4096 as well. It takes both from `kani::any()`
  rather than from the crate's `some_rights`, whose `assume` would exclude the
  undefined bits and quietly leave this at 64 x 256 while three files said
  65 536 — which is what it did until review caught it. The reduction was also
  buying nothing: the unconstrained harness verifies in 144 s against the 150 s
  the constrained one was recorded at, so there was never a trade to make.

What it does not buy, said plainly: **`total_bought` holds for a page of eight
slots and not for a page of a hundred and twenty-eight.** A defect that only
appears at the larger page — an index computation that eight cannot overflow —
is outside it. It is inside `properties::self_test`, which runs the same code at
the real page size on every boot. The two instruments cover different halves and
neither is the other's substitute, which is the sentence RFC 0022 wrote about
litmus tests and model checking.

### The harness that was written, run, and taken out

One property has no proof here and the absence is deliberate rather than
overlooked, so it is recorded where somebody looking for it will find it.

*A slot in a bought page, used and given back, does not answer the handle it
answered to last time* — the half of `forged` that growth added, and the reason
`Slot::fresh` starts a bought page at the table's generation floor. The harness
was written and run. Reaching a bought slot through the public interface means
filling the free part first, because `place` fills the lowest vacancy: that is
thirty-two grants each scanning a forty-slot `vacancy`, then a `clear_all`, a
second account, a second `grow`, and a symbolic handle resolved against a page
reached through a raw pointer. **It did not terminate in forty-five minutes.**
Every other harness here is seconds or a few minutes, and a harness that needs
three quarters of an hour is one whose failure a person learns to wait out.

So it is out, and `kernel/proofs/src/proofs.rs` carries the statement of the gap
where the harness was. What covers it instead is `properties::forged`, which
does exactly this at every boot at the real page size on a table grown out of a
real frame. The property is *checked*; it is not *proved*. That is a smaller gap
than it sounds — what a proof adds over that check is quantification over the
handle, and `total_bought` already quantifies over every handle against a grown
table — but what is missing is the two together, and that is the honest shape of
it.

*What would close it:* a cheaper way to reach a bought slot, a smaller
`TABLE_SLOTS`, or a checker that summarises a loop instead of unrolling it.

There is a second reduction and it is smaller: the *depth* of the derivation
tree `stale` builds is three, and the number of operations a harness performs
before quantifying is a handful. Table contents are symbolic but the *sequence*
that produced them is not. A property that only fails after a fourth derive is
outside this too. Depth three is where a revocation that stops at the children
first becomes visible, which is why the negative suite chose it and why this
does.

### What the proofs are not about

`Direct` — the one `Backing` a running process uses. The harnesses drive the
table through `pages::Pages`, whose promise is the same and whose
implementation is three lines of harness rather than three lines of direct map.
So what is proved is *the table's behaviour given a backing that keeps its
promise*. Whether `Direct` keeps it is a different question and the boot suite
is where it is answered. This is worth stating because it is the one place a
reader could take these proofs to say more than they do.

## Consequences

**Easy.** `E1-P12` is now a file. The crate, the verb, the image target and the
schedule all exist; putting `f-ring`'s validation paths under the same checker
is a second module and a second entry in `PROOF_HARNESSES`.

**Easy, and it is the thing worth having.** The properties are now stated twice
in two shapes that fail differently. A regression that a fixture would miss
because nobody thought of the input is a counterexample here, printed as an
assignment that `--concrete-playback` turns into a test case.

**Hard.** Two toolchains became three: the pin, RustMC's, and Kani's. A
newcomer now has to be told which is which. `docker/README.md` carries that
paragraph, and the reason each exists is one sentence long in each case.

**Hard, and this is the cost that will actually be felt.** `kernel/src/cap.rs`
has acquired a second compiler and a second set of module dependencies it must
keep satisfying. Adding a `use crate::` naming a fourth kernel module breaks a
build the workspace does not contain. That is a real standing constraint on one
file and it is invisible at the moment of violating it, so it is not left to
the schedule: `cargo xtask lint-proofs` runs the ordinary build in the gate, and
the person who writes the line finds out in the same session rather than from a
nightly the next morning. The fix when it fires is small — the stand-ins are
three short files and a fourth dependency needs a fourth one — which is the
reason this is a constraint worth carrying at all.

**Hard, in the image.** The `full` image grows from 2.70 GB to 4.18 GB. It is
in `full` and not in `dev` precisely so that the pull-request gate does not pay
it.

**Foreclosed, and stated rather than discovered.** The proof holds for the *IR
Kani's rustc produces*, not the IR the kernel ships, and for a page of eight
slots rather than a hundred and twenty-eight. Both gaps are real. Both are far
smaller than the one that exists otherwise, which is no proof at all — and RFC
0022 wrote that sentence first, about a different tool, which is some evidence
that it is the right sentence.

**Not foreclosed, and worth saying:** nothing here enters `claims/`. A checker
produces a verdict, not a number, so this triggers no claims re-run. That is the
property that makes a third toolchain affordable at all.

## What would reverse this

**A defect that the reduced page size hides.** If `properties::self_test` at the
real page size ever catches something these proofs pass over, the reduction
stops being a bound and becomes a hole. The answer then is not a bigger unwind —
the walk is quadratic and no budget survives it — but making the revocation walk
stop being quadratic, which `MAX_PAGES` already names as its own reversal
condition. The two reversals are the same reversal, which is worth knowing.

**`cap.rs` acquiring a dependency the stand-ins cannot cheaply carry.** A
capability table that needs the page tables, the IOMMU or another core is a
table that cannot be compiled in isolation, and at that point this arrangement
is a fourth stand-in that is really a second implementation. Then the answer is
to reverse this and go back to the boot suite alone, rather than to keep a
proof whose stand-ins have quietly become a model.

**Kani ceasing to be maintained, or its rustc drifting far enough that
`cap.rs`'s edition stops compiling under it.** The same live condition RFC 0022
named for `ring` and `abi`, now attached to one more file. The checking job
going red with a parse error is what noticing looks like. If that becomes an
obstruction rather than an annoyance, delete `kernel/proofs` — the workspace
`exclude` line, `lint_proofs` in xtask, and the two jobs in `nightly.yml`
(`prove` and the `image_full` build nothing else consumes) are the whole of the
rest. `lint_proofs` refuses to be the expensive part of that: it skips when the
directory is gone rather than failing.

**Verus arriving on the frame at phase 02.** If the frame's invariants are being
discharged deductively, a bounded check of a subset of them is a weaker
statement kept for its speed rather than for its content. Keep it while it is
the faster instrument; delete it when it stops being the only one.
