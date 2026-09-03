# RFC 0045: The architecture list is derived from the workspace

- Status: accepted
- Date: 2026-09-03
- Affects: `xtask/src/main.rs` (`PORTABILITY`, `lint-arch-tests`),
  `.github/workflows/ci.yml`, `docs/test-taxonomy.toml`,
  `docs/test-taxonomy.md`, `E1-P11`

## Decision

**Which crates are checked on AArch64 is read from `Cargo.toml`'s `members`, not
written beside it.** Every workspace member is tested on both architectures and
compiled for `aarch64-unknown-none` by default; a member that is not needs a row
in `PORTABILITY` in `xtask/src/main.rs` carrying a reason and a reversal
condition, and a member with no row at all is a hard failure rather than a skip.
The check runs in both directions: a row naming a crate the workspace no longer
has fails too.

And one level below that: **no test may be compiled on one architecture and not
the other without a reason and a reversal recorded beside it.** `PORTABILITY` is
about crates and the exit criterion is about tests, and those are not the same
unit — a crate can be on both runners with a `#[cfg(target_arch)]` test inside
it. `cargo xtask lint-arch-tests` reads the gate in the source and refuses that,
with `ARCH_TEST_ALLOW` as the recorded exception. It is empty.

The consequence that is the point: **adding a crate to this workspace cannot
silently skip an architecture.** It joins both checks, or the build stops and
names it. Nor can adding a test.

## Context

`CLAUDE.md` records the same scar twice — *testing the ring only on x86-64*, and
*writing code above the frame that only compiles on x86-64, and finding out from
the AArch64 job*. Both were answered with lists of crate names, and both lists
then went stale exactly the way a list beside a thing goes stale.

What was true when this was written:

| | named | workspace |
|---|---|---|
| `cargo xtask test`, host half | `--workspace --exclude f-kernel` | derived already |
| `cargo xtask test`, AArch64 half | six crates, by hand | ten members |
| `ci.yml` `test-x86` | four crates, by hand | ten members |
| `ci.yml` `test-aarch64` | four crates, by hand | ten members |

Three consequences followed, none of them visible from a green run.

**`f-store` and `f-virtio-blk` were compiled for AArch64 by nothing in CI.** The
arm job built four crates and neither was among them; the cross-compile that did
cover them lived in `cargo xtask test`, which no CI job runs. `docs/test-taxonomy`'s
`aarch64-compile` row claimed the cadence *every verify, every PR*, and the second
half of that was not true. A cadence a row claims and nobody keeps is worse than a
row that admits it runs on no cadence, because the first one is believed.

**`f-sim`, `f-bench` and `xtask` had tests that ran on a laptop and on no
runner.** The taxonomy already had a row admitting this — *a test that exists and
runs on no runner*, status `partially` — and it named `xtask` and `bench` from
before `f-sim` existed, which is the staleness this RFC is about, appearing in
the document that tracks it.

**The hand-written AArch64 list happened to be correct.** That is the part worth
stating plainly, because it is why this is a structural change and not a bug fix:
the six crates named were exactly the six `no_std` non-kernel members. Nothing was
broken on the day it was read. The defect is that nothing would have said so on
the day it stopped being true, and `xtask`'s own comment records that exact thing
having already happened once, with `f-bench` and `f-init`.

Two alternatives were live.

**Derive the answer entirely — check `#![no_std]` and infer.** Rejected. It would
have produced the right six crates today with no table to maintain, and it
produces a *rule* where what is wanted is a *record*: the exit criterion for
E1-P11 is "no test is skipped on AArch64 without a recorded reason", and an
inference records nothing. It also fails in the direction that matters — a
`no_std` crate that is nonetheless x86-64's, which is precisely what `f-kernel`
is.

**Leave the lists and add a lint comparing them.** Rejected as the same thing
with more moving parts: two lists and a third check that they agree is three
places to edit, and the exclusion reasons would live in none of them.

## Consequences

**Easy.** A crate added to the workspace is checked on both architectures with no
edit anywhere. Leaving one out costs a sentence, and the sentence is printed on
every run of `cargo xtask cross` and `cargo xtask test-host` — green runs
included, because an exclusion nobody reads is an exclusion nobody argues with.

**Hard.** The exception table is prose in a Rust source file, and prose rots. The
mitigation is mechanical and small: a test requires every reason to contain the
word *Reversal*, which is the same thing `docs/rfc/0000-template.md`'s last
section exists for. It does not check that the reversal is *true*, and nothing
can.

**Foreclosed, and stated rather than discovered.** The AArch64 target is
`aarch64-unknown-none`, so this covers the crates that reach the machine and
says nothing about the three host tools. Their tests run on the arm runner —
which is the half of the question that is about them — but they are not compiled
for a bare-metal AArch64 target and there is no configuration in which they
would be.

### The level below the table, and why a table could not reach it

`PORTABILITY` answers *which crates run on both runners*. It cannot answer *which
tests do*, and the gap between those two questions is not academic: a crate on
both runners with an architecture-gated test inside it leaves `test-host` green
on both machines while one of them collects fewer tests. Nothing fails. A test
count is not an assertion and nobody reads one.

That is the same shape of defect this RFC's first half removes — a check that is
green while the property it stands for does not hold — so it gets the same
treatment rather than a note saying it is unlikely. `cargo xtask lint-arch-tests`
reads the gate where the gate is written:

- an architecture `cfg` on a test function, or on any block a test is written
  inside; and
- an architecture `cfg` on a `mod NAME;` declaration, followed into the file it
  names and everything that file declares in turn — because the gate is on the
  declaration and the file it gates carries no trace of it. `user/init/src/lib.rs`
  is the shape: `#[cfg(target_arch = "x86_64")] pub mod component;`, and
  `component.rs` says nothing about an architecture anywhere in it; and
- a file-scope `#![cfg(target_arch = …)]`, held apart from the item gate and
  never cleared by the line of code below it. This third one was missing from
  the first version of the check and review found it green over exactly that
  input — an inner attribute, a `use`, then two tests, all of them compiled on
  one machine and none of them reported. It is the shape that matters most
  rather than least: an integration test file under `tests/` is named by no
  `mod` declaration anywhere, so the second bullet can never reach it, and a
  file-scope attribute is the only gate it can carry. `ring/tests/litmus.rs`,
  `faults.rs`, `headers.rs` and `hostile.rs` are those files, and the ring's
  tests are what the scar is about.

Twenty-four files in this tree are behind such a gate today and not one of them
contains a test, which is why `ARCH_TEST_ALLOW` is empty. That is the finding
rather than the default: the empty list is a measurement, and the check is what
keeps it one.

**What would make this green while tests were being skipped.** One shape used to
and no longer does, and saying which is the point of this paragraph: a
file-scope `#![cfg(target_arch = …)]` was read as a gate on the *next item*, so
the first `use` below it discarded it and every test in the file read as
ungated. That was not one of the limits the first version declared — it was an
undeclared hole, in the one place an integration test can be gated — and it is
now a refused input with two cases behind it. The lesson is the one this
epoch keeps teaching: the declared-limits paragraph is only worth what someone
has tried to falsify, so each of the three refused shapes has a test that goes
red without the fix.

What remains is read from source text, so a test a macro generates, a module
reached through `#[path]`, and — the one worth naming — a test gated on a
*feature* that only one architecture ever enables are all invisible to it. The
last is live rather than theoretical:
`user/store` and `user/virtio-blk` both write
`all(target_arch = "x86_64", feature = "image")`, and a crate that wrote the
feature half alone would be architecture-gated in a way no reader of the source
could see. The limit is declared in the function rather than left implied, the
way `JOIN_GAP` and `CHAOS_GAP` are declared, because a check that claims more
than it reads is worse than one that says where it stops.

The alternative was to compare the tests the two runners collected. It was
rejected for a reason worth writing down: it is a check that lives in neither
runner's job. It needs both logs, so it fails in a third place, it cannot run
before a push, and the development container cannot run it at all. That last
claim is measured rather than assumed, and it is worth being precise about
because a reader who checks will find something that looks like a
counter-example. The container is x86-64 and it *does* carry
`qemu-system-aarch64`. What it does not carry is any way to run an AArch64
**host** binary: no `qemu-user` — every `qemu-*` in the image is a full-system
emulator — nothing registered in `binfmt_misc`, and `rustup target list
--installed` is `aarch64-unknown-none`, `x86_64-unknown-none` and
`x86_64-unknown-linux-gnu`, with no `aarch64-unknown-linux-gnu` to build a test
harness for in the first place. The system emulator is no help either, because
what it would boot is a frame, and the frame is x86-64 — which is the row
`f-kernel` already holds in the table above. So the container compiles for that
architecture and cannot execute for it, and E1-P11's *under emulation* resolves
to the arm runner rather than to a local command. A source check, by contrast,
runs on the laptop where the gate is being written, which is the moment it is
cheap to argue with.

That last conclusion is a declaration about a machine, and a declaration about a
machine is the kind that rots in one direction: the day the container gains a
way to run AArch64 code, the sentence saying it cannot is still in the file,
still read as true, and the local loop goes on not running a suite it could now
run. So it is `ARCH_RUN_GAP` — a list of the properties the arm runner
establishes and this machine cannot, printed on every `cargo xtask test`, with
`cargo xtask test` **refusing** if a run path turns up after all: an interpreter
registered in `binfmt_misc`, or a `qemu-aarch64` user-mode emulator on `PATH`.
Those two are the run paths that exist; a *system* emulator is not one, because
what it boots is a frame and the frame is x86-64. `JOIN_GAP` is the precedent
and states the reason this is worth a check rather than a comment: the failure
that matters is not that a gap is never closed but that it closes and the
documents go on describing it.

### What still runs on one architecture, and why

E1-P11's exit is about tests, and the table above is about crates, so the
remainder belongs here rather than nowhere. Every one of these is a deliberate
single-architecture check with a reason:

| What | Runner | Why |
|---|---|---|
| The QEMU boot suite — `run`, `orders`, `user`, `cap`, `iommu`, `blk`, `runtime`, `mutate`, `panic` | x86-64 | The frame is x86-64. `KERNEL_TARGET` is `x86_64-unknown-none` and `kernel/src/arch/` is one architecture. *Reversal:* an AArch64 frame, at which point the suite acquires a second runner. |
| `litmus --features mutate-no-doorbell-fence` | x86-64 | Store-load is the one reordering total store order performs and the one AArch64 forbids: `stlr`/`ldar` are RCsc. Measured, not inferred — 58 971 lost wakeups on x86-64, a clean pass on the arm runner. RFC 0020, and `ci.yml` argues it at the step. |
| `trace`, `simulation`, `workload`, `sweep --corpus`, `chaos`, `snapshot` | x86-64 | These compare two runs for equality, and the pairs are what the claim is about: two *machines*, both of them the runner this repository's claims were measured on. A third architecture would be a different claim, not more of this one. |
| `coverage`, `claims`, `package`, `deps` | x86-64 | Reports and registry checks over the tree. Nothing they read is architecture-dependent, and `package`'s own comment names the one confounder it does check — the build path. |

None of these is *skipped* on AArch64: each is a check about something that is
x86-64, or about something with no architecture in it at all. The distinction
matters, because the row this RFC closes was about tests that were skipped and
looked like tests that had run.

### Handed on rather than done here

`CLAUDE.md`'s *Common mistakes* carries the two scars this RFC is an answer to —
*testing the ring only on x86-64* and *writing code above the frame that only
compiles on x86-64*. Both now have a command behind them, `cargo xtask cross`
and `cargo xtask lint-arch-tests`, and both lines would be worth annotating with
it. That file is not edited here: it is protected, and a scar is the maintainer's
sentence to write. `docs/TESTING-STATUS.md` is in the same position and is
further out of date — it still describes the local loop as *an AArch64
cross-compile of the four crates the arm job tests*, and neither half of that is
true after this change.

## What would reverse this

**An AArch64 frame.** Then `f-kernel` loses both of its reasons at once, the boot
suite becomes a matrix rather than a job, and the interesting half of this table
— which crates are x86-64's — collapses to nothing. This is the outcome to
prefer, and `user/init/src/lib.rs` has named it as the reversal for
`f_abi::door::call` since M0.

**The table growing past the workspace being readable at a glance.** Ten members
with four exclusions is a page. Forty members with twenty exclusions is a policy
document with a `struct` around it, and at that point the right answer is a
manifest key each crate carries about itself, checked here rather than listed
here — the same move `lint-components` made when the component list stopped
being something one file could hold. The trigger is a number: more exclusions
than inclusions.

**`ARCH_TEST_ALLOW` acquiring entries faster than it acquires arguments.** One
entry is a decision; a handful is a pattern, and the pattern would mean the
workspace has grown architecture-specific code above the frame that wants
testing on the architecture it is for. At that point the answer is a second
bare-metal runner rather than a longer allow-list, and this RFC is the thing to
reverse.

**The reasons turning out to be unread.** The test that every exclusion states a
reversal is a proxy, and a weak one. If a reason is ever found to have survived
the condition it named as its reversal, the table is not doing its job and the
answer is to make the reversal a check rather than a sentence — a `cfg` that
fails to compile once the frame it waits for exists.
