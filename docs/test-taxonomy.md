# Test taxonomy

For each class of bug this system can have: which layer catches it, what the
concrete check is *today*, how often that check runs, and — where nothing
catches it — which task closes the gap or why the gap is accepted.

`docs/design/proving-ground.html` specifies the layers and
`docs/TESTING-STATUS.md` says where each one actually stands. Neither answers
the question a person asks when they are about to change something: *if I break
this, what notices, and when?* This page is that direction of the same
information, and it is written to be re-read rather than admired — a row whose
"today" column is out of date is a row telling somebody a check is happening
that is not, which `CONTRIBUTING.md` already names as worse than an honest
absence.

Rows are bug classes rather than components on purpose. A component-shaped table
gets a tick beside everything that has any test at all; a bug-shaped one has to
say what would have to go wrong and what would see it, and that is where the
holes are.

## What the three statuses mean

- **catches** — a named check runs on a stated cadence and would fail on this
  bug. Not *proves*: sampling counts, and where a row is sampled rather than
  proved it says so.
- **partially** — something runs and would catch some instances. The cell says
  which half is covered, because "partially" with no boundary is a tick with
  extra syllables.
- **GAP** — nothing today. Every GAP is answered below in *Every gap, and who
  owns it*, with either a `TODO.md` task id or an explicit acceptance carrying a
  reversal condition.

## The layers

L0 to L7 are `proving-ground.html`'s, with their real status from
`TESTING-STATUS.md`. **P** and **X** are this page's additions, and both add a
*name* rather than a plan: the checks were already there and had no row. They
are lettered rather than numbered on purpose — the ladder's numbering belongs to
`proving-ground.html`, and a row called L8 here would collide with whatever that
document calls L8 the day it grows one.

| | Layer | Status today |
|---|---|---|
| **L0** | Determinism substrate | Built — `env/`, `lint-determinism`, `xtask trace` |
| **L1** | Deterministic simulation | Hook only — `env/src/sim.rs`, consumed by `ring/tests/faults.rs`. No device models, no seed sweeps |
| **L2** | Concurrency and memory model | Stress tests only — `ring/tests/litmus.rs` on two architectures. Not a model check |
| **L3** | Proof | Absent |
| **L4** | Fuzzing | Instrumentation only — `xtask coverage`. No generator, no corpus, no hostile-peer harness |
| **L5** | Performance regression | Harness only — `bench/`, `claims/history.jsonl`. No change-point detection |
| **L6** | Hardware in the loop | Absent — one boot outside QEMU, by hand, on a virtual machine |
| **L7** | Claims registry | Built, three entries, none gating |
| **P** | Provoke the real kernel | Built — `xtask fault`, `user`, `cap`, `mutate`, `panic`, `trace` |
| **X** | Policy made executable | Built — twelve lints, six hooks, `REVIEW.md`, `evals/` |

**P** is the row `TESTING-STATUS.md` already says the seven-layer taxonomy
lacks: the real kernel, in QEMU, being asked to do something that must not work.
It is neither simulation nor proof nor fuzzing, and it is where most of this
project's evidence currently comes from — so a taxonomy without a name for it
would have to file its best-covered rows under "other".

**X** is the second absence, and this table is what found it. Nothing in the
seven layers has a home for *a decision being broken* as distinct from a
behaviour being wrong: `unsafe` outside the frame, an import across the licence
boundary, a number in prose with no claim, a mutable `static` outside `PerCpu`.
Those are caught by a lint wall, a set of hooks and four review passes, none of
which is a test in the sense L1 to L6 mean. Forty of the eighty-five rows below
carry X, and before this page they had no layer at all.

P and X sit beside the ladder rather than on top of it. `proving-ground.html`
orders L0 to L7 by dependency, each rung resting on the one below; P and X rest
on almost nothing and are the cheapest checks in the tree, which is why the
lints run first and the boots run before the tests that need a model checker.
Numbering them would have said the opposite, which is the second reason they are
letters.

## The cadences

| Cadence | What it is | Where it is configured |
|---|---|---|
| **every edit** | a `PreToolUse` hook, on each `Write`/`Edit`/`Bash` an agent makes | `.claude/settings.json`, `.claude/hooks/` |
| **every build** | a `const _: () = assert!(…)`, so the compiler is the check | `abi/src/lib.rs`, `abi/src/state.rs` |
| **every verify** | `cargo xtask verify` — lint, test, run, panic, trace, mutate | `xtask/src/main.rs`, `fn verify` |
| **every PR** | a job in `ci.yml`, which triggers on `pull_request` and on `push` to `main` | `.github/workflows/ci.yml` |
| **daily** | `maintain.yml`, 05:23 UTC — `ops/detect.sh` and the bands | `.github/workflows/maintain.yml` |
| **weekly** | `agent-evals.yml` Mon 04:17 UTC; `security-scan.yml` Tue 06:41 UTC | those two files |
| **per release** | `cargo xtask release`, behind `F_RELEASE_AUTHORIZATION` | `RELEASING.md`, `.claude/hooks/release-gate.sh` |
| **on demand** | the command exists and nothing schedules it | — |
| **never** | no check exists | — |

Two cadences are weaker than they look, and it is worth saying so once here
rather than in nine cells. **every edit** covers an agent session and nothing
else: a person editing in an editor gets *every PR* and no earlier. And **every
verify** is a local loop somebody has to run — `CONTRIBUTING.md` asks for it
before review, and the gate is what makes it more than a request.

Which matters most where a row says *every verify* and nothing after it. Three
do: `lint-manifests`, `lint-reproduce`, and the part of `cargo xtask test` the
gate's four-crate list does not name. Those three verbs appear in no workflow
file, so for them the local loop is not a convenience that saves a round trip —
it is the only place the check happens, and a contributor who skips it skips the
check. `verify` and the gate are deliberately different and always will be, but
these three are different in the direction nobody intended.

The rows for `cargo xtask cap` and `cargo xtask user` say *every PR* and not
*every verify* for the same reason, read the other way: `fn verify` is lint,
test, run, panic, trace and mutate, and it invokes neither. The eleven capability
escapes and the seven ring-3 boots run in the CI `kernel` job. A local green
`verify` is not evidence that a change to the capability table is safe.

## A — the ring and the memory model

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Submission publishing store weakened to `Relaxed` | L2 | `cargo test -p f-ring --test litmus --release`, both runners; `mutate-relaxed-submission` exists as a fixture and gates nothing | every PR | **partially** |
| Completion publishing store weakened to `Relaxed` | L2 | the same job; `mutate-relaxed-completion` | every PR | **partially** |
| Suppression fence removed (Store-Load, doorbell) | L2 | the litmus job with `--features f-ring/mutate-no-doorbell-fence`, **x86-64 only** | every PR | **catches** |
| An interleaving the stress tests never produce | L2, L3 | nothing | never | **GAP** |
| A new `Release`/`Acquire` pair lands with no litmus test | X | `REVIEW.md` pass 2, the `CLAUDE.md` convention | every PR | **partially** |
| Cursor arithmetic wrong at the wrap | L2 | `ring/src/lib.rs::cursors_may_wrap`, plus the outstanding-across-the-wrap assertion in `litmus.rs` | every verify, every PR | **catches** |

The first two rows are the sharpest "partially" on this page and the reason to
read it. Both mutation fixtures were put in front of the stress suite as a CI
gate on the AArch64 runner — the machine where weakening `Release` is a real
defect — and **the suite passed with them on**. Both steps were removed as
gates, because a gate asserting that a probabilistic test catches a specific
reordering goes red on a Tuesday for reasons nobody can reproduce. So the
standing position is not "the litmus tests might miss something": they were
given the exact defect, on the exact hardware, and did not see it. `E0-P16` owns
what does.

The third row is the inverse, and is the lesson of the pair. `Release` and
`Acquire` compile to plain `mov` on x86-64, so the store buffer performs the
Store-Load reordering the doorbell fence exists to stop; on AArch64 they are
`stlr` and `ldar`, which are RCsc, so removing the fence changes nothing
observable there. Which machine can see a defect is a property of the reordering
it depends on, not of how serious it is — 58 971 lost wakeups in 500 000 rounds
on x86-64, a clean pass on arm.

## B — a hostile peer

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| A hostile header or cursor panics the consumer | L4, P, X | `ring/tests/headers.rs`; `litmus.rs::a_hostile_client_cursor_never_panics`; the `unwrap_used`/`panic`/`unreachable` clippy wall over `ring` | every verify, every PR | **partially** |
| A hostile header is accepted rather than refused | L4 | `headers.rs` — every invalid header refused with a structured error, and the region survives the refusal | every verify, every PR | **partially** |
| An entry mutated between validation and use | L1, L4 | nothing | never | **GAP** |
| Unknown opcode, flag or reserved bit accepted (R04) | L4, X | `abi` unit tests over the reading of `Sqe`; `REVIEW.md`; no lint | every verify, every PR | **partially** |
| A peer that dies mid-claim, or lies about its epoch | L1, L4 | `ring/tests/faults.rs` injects at `ring.publish`, `ring.consume` and `chan.bind` | every PR, both runners | **partially** |
| ABI layout drift — a field reordered inside a fixed-size struct | X | `const _: () = assert!(size_of::<Sqe>() == 64)` and its four siblings catch a size change; **no offset assertion, no golden bytes** | every build | **partially** |
| A peer at a different ABI version | X | `ChannelHeader::negotiate`, RFC 0011, exercised against a real mapping by `headers.rs` | every verify, every PR | **catches** |

Every row here is "partially" for one reason: what exists samples, and what is
owed proves. `E1-P04` asks for a billion hostile operations with no panic, no
memory unsafety and no hang; `E1-P12` asks for panic-freedom over arbitrary
header bytes, cursors and entries as a bounded proof. The distance between those
and what is in the tree today is the distance between *no fuzzer found one* and
*there is none*.

The layout row is the quiet one. A reorder inside `Sqe` keeps `size_of` at 64
and changes what every byte means to a peer built at another commit, and nothing
in the tree would notice. It costs nothing while both ends of every channel come
from one tree, and stops costing nothing at `E1-B05`.

## C — capabilities and isolation

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Naming a slot the frame never filled | P | `cargo xtask cap unowned` | every PR | **catches** (sampled) |
| Forging a handle | P | `cargo xtask cap forge` — the handle space swept, in range and past it | every PR | **catches** (sampled) |
| Use after revoke | P | `cargo xtask cap stale` | every PR | **catches** (sampled) |
| Derivation escape — rights that were never granted | P | `cargo xtask cap rights`, `cap type` | every PR | **catches** (sampled) |
| A process makes the kernel panic by trying | P | all eleven `cap` boots must end at 33; `cargo xtask mutate` is the build that breaks the property and must go red | every PR (`cap`); every verify, every PR (`mutate`) | **catches** |
| Mapping left after revoke (TLB shootdown) | P | `cargo xtask cap unmap` — one page, one process, one boot | every PR | **partially** |
| The capability table outgrows the slots its holder has paid for | P | `cargo xtask cap flood` buys a page and stops where the untyped region runs out; `cap quota` spends the region first and stops at the free size; `cap beyond` names slots past what was bought | every PR | **catches** |
| Authority arriving by inheritance (R06) | P, X | the negative suite covers the part that runs; no lint | every PR | **partially** |
| A ring-3 process touching what it was not handed | P | `cargo xtask user` — seven boots, six must fault and one must not | every PR | **catches** |
| Writing to a read-only grant | P | `cargo xtask cap state` | every PR | **catches** |
| Speculation across a domain boundary (R02) | X | `cargo xtask lint-manifests` requires the domain field RFC 0005 rule 4 names; the supervisor refusal is not built | every verify | **partially** |
| A driver addressing memory outside its grant (IOMMU) | L1, P | `cargo xtask iommu` — two boots on a machine with a VT-d unit and a real virtio-blk device, with the frame's own adversary as the requester: one descriptor inside the grant, which must land bytes, and one outside it, which must be refused and recorded in the unit's own fault registers. `cargo xtask blk` — three boots with the requester's descriptor written by a driver **component**, `user/virtio-blk`, out of a `Reach` the frame answered its client's registration with: one carries a sector out and back through a ring byte for byte, one withdraws the client's page from the driver's domain between the write and the read (RFC 0024's reclaim, the frame's property), and one has the driver add a frame to the address it was answered before writing it into a descriptor, requiring the unit to fault at the address the driver invented — the only one of the five that is a driver *reaching* outside its grant. All five halves also require the frame to refuse a device translation for a capability carrying no `GRANT` | every PR | **partially** — the descriptor and the arithmetic behind it are a driver component's own, and the component's *code* is still called by the frame because nothing schedules one; RFC 0033 dates that to E1-B08. Both commands are jobs in `ci.yml`, so all five halves keep the cadence claimed here |

The five properties hold and each has something that breaks it, which is
`E0-P08` met. What none of them is, is a proof: eleven boots sample a space
`E1-P07` is meant to exhaust with Kani, and the honest reading of a green `cap`
run is that the escapes somebody thought of were refused.

The `unmap` row changes character in this epoch. Today a revoke is one page, one
IPI, one spin on an acknowledgement, and one boot exercises it once. `E1-B14`
writes the workload where that stops being enough — registered buffers cycling,
a driver restart unmapping a whole grant page by page — and that workload is
also the first thing that could observe a mapping surviving a revoke under
churn.

## D — memory

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Frame leak on process death | P | `process::reap` fails the boot if the count does not return; the boot line reports `user frames N given back, free count unchanged` | every verify, every PR | **catches** (one death) |
| Frame leak under churn | L1, L5 | nothing | never | **GAP** |
| Allocator split/coalesce corruption | L1, P | `cargo xtask run` — `mem::self_test` phases two to four: an `Env`-driven mix of orders, a probe upward from order 9, and a 2 MiB block handed back as 512 shuffled frames that must reappear as a 2 MiB block. `cargo xtask orders` boots a machine with a gibibyte in it and requires order 18 | every verify, every PR | **catches** |
| Kernel-global mutable state outside `PerCpu` | X | `cargo xtask lint-percpu` | every verify, every PR | **catches** |
| A fifth word crossing a core with no named ordering | X | `REVIEW.md` pass 2, RFC 0016; `lint-percpu` does not count atomics | every PR | **partially** |
| W^X or NX broken on the kernel mapping | P | `cargo xtask fault nx`, `cargo xtask fault wx` | on demand | **GAP** |
| Page tables leaked by the higher-half on-ramp | P | the free count is asserted across the switch — `arch/x86_64/paging.rs` | every verify, every PR | **catches** |

## E — time and scheduling

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Deadline inversion in a device queue | L1, L5 | nothing — RFC 0025 decides the rule and nothing orders a device queue yet | never | **GAP** |
| A component claiming urgency forever | L1, L3 | RFC 0025 decides the decay; nothing enforces it | never | **GAP** |
| A granted reservation misses its deadline under load | L1, L5, L6 | nothing — there is no admission control | never | **GAP** |
| An over-subscribed reservation is admitted | L3, P | nothing | never | **GAP** |
| Timer jitter regression | L5, L7 | `cargo xtask timer 60`; claim 0002 is `pending` and gates nothing | on demand | **GAP** |
| Ring submit latency regression | L5, L7 | `cargo xtask claim ring-submit-latency`; claim 0001 is `pending`; `bench` refuses to record under `F_ENVIRONMENT=container` | on demand | **GAP** |
| Boot-time regression | L5, L7 | claim 0003, `tracked`, `cargo xtask claim boot-to-m0`, threshold 50 ms | on demand | **partially** |
| A regression too small for a threshold | L5 | nothing; `claims/history.jsonl` is accumulating for it | never | **GAP** |

R08 is the rule this group exists to keep honest: a hard class with no admission
test is a hint with a better name. Four of these eight rows are empty because
the thing they would test does not exist yet, and that is the correct state at
the start of E1. What would not be correct is for them to be absent from the
table, because that is how a promised layer acquires no owner.

## F — determinism

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| A clock, `HashMap` or `rdtsc` outside `Env` | L0, X | `.claude/hooks/determinism-guard.sh`, then `cargo xtask lint-determinism` | every edit, every verify, every PR | **catches** |
| A source of nondeterminism no pattern names | L0, P | `cargo xtask trace` locally; the two-runner `trace` and `reproduction` jobs in CI | every verify, every PR | **catches** |
| A determinism leak that never reaches the boot log | L0, L1 | nothing — the trace hashes the boot log, and the state tree deliberately publishes nothing that varies with time | never | **GAP** |
| Correlated streams in a seed sweep | L1 | per-site streams in `env/src/sim.rs`, plus the independence test | every verify, every PR | **partially** |
| A fault-injection site that is never exercised | L1 | the site table is fixed at sixteen and overflow is counted and reported rather than dropped | every verify, every PR | **partially** |
| An allow-list entry added in the same diff as the code needing it | X | `REVIEW.md` pass 1 names it as the commonest way the policy erodes | every PR | **partially** |

The second row justifies the whole of L0, and it is worth stating what it
catches that nothing else does. `mutate-unseeded-time` puts one unseeded read of
the timestamp counter on the boot path. The kernel still boots, every assertion
still holds, it still prints `M0 ok` and exits 33 — every other check in this
tree is green on it. The only thing wrong is that two runs no longer agree.

## G — the policies

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| `unsafe` outside `abi/`, `ring/`, `kernel/` | X | `unsafe_code = "forbid"` from the workspace at compile time, and `cargo xtask lint-unsafe` over the text | every build, every verify, every PR | **catches** |
| A `// SAFETY:` comment that does not discharge its obligation | L3, X | `REVIEW.md` passes 1 and 3; no lint reads the comment | every PR | **partially** |
| The `unsafe` share crossing RFC 0001's trigger | X | `cargo xtask unsafe` reports both shares; it does not gate | on demand | **partially** |
| The permissive tree importing `third_party/` | X | `cargo xtask lint-licensing` | every verify, every PR | **catches** |
| A missing SPDX header | X | `cargo xtask lint-licensing` | every verify, every PR | **catches** |
| A number in prose with no claim | L7, X | `REVIEW.md` pass 4 | every PR | **partially** |
| A document citing a claim value the claim no longer has | L7, X | `cargo xtask lint-claims`; `cargo xtask claims --render` fixes it | every verify, every PR | **catches** |
| A claim with no owning document (R09) | X | `cargo xtask lint-claim-owners` | every verify, every PR | **catches** |
| A claim whose published reproduction runs nothing | L7, X | `cargo xtask lint-reproduce` | every verify | **catches** |
| A committed snapshot asserting what the registry does not hold | L7, X | `cargo xtask lint-snapshot`, run before anything regenerates it | every verify, every PR | **catches** |
| A quantity crossing the ABI with no unit (R03) | X | `cargo xtask lint-units` | every verify, every PR | **catches** |
| A callback registered across an interface (R05) | X | `cargo xtask lint-callbacks` | every verify, every PR | **catches** |
| A deliberate defect left on by default | X | `cargo xtask lint-mutations` | every verify, every PR | **catches** |
| A component manifest that does not fit `docs/manifest.md` | X | `cargo xtask lint-manifests` | every verify | **catches** |
| A reversal with no RFC | X | `REVIEW.md` pass 4 | every PR | **partially** |
| Buffer ownership violated — both sides hold it | X | three `compile_fail` doctests in `ring/src/buffers.rs` (E0599, E0451, E0382) | every verify, every PR | **catches** |

Twelve of these sixteen fail a build, which is a better ratio than the twelve
rules manage — `CONTRIBUTING.md` records that three of R01 to R12 are executable
and nine are review. A thirteenth has a verb that reports and does not gate, for
a reason RFC 0001 states and `E0-B21` records. The three that are left are
review for the same reason the nine are: each would need a checker that can read
intent. A lint cannot tell a number that needs a claim from a number in an
example, or a `SAFETY` comment that discharges an obligation from one that
restates it.

R01 applies to this table too. A row whose check is "`REVIEW.md` pass 4" is a
row saying a person has to apply it, which is a plan.

## H — components and drivers

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Driver death observed by a client (blast radius) | L1, P | nothing — there is no driver and no supervisor | never | **GAP** |
| Supervisor restart storm | L1 | nothing — RFC 0008 declares a restart policy and nothing runs it | never | **GAP** |
| A component that legitimately outgrows the capability table | P | `cargo xtask cap flood` holds 160 capabilities, five times the fixed count, with the growth debited from its untyped region; `cap quota` holds 32 because it spent that region first | every PR | **catches** |
| An imported driver reachable other than over a ring | X | `cargo xtask lint-licensing`; `lint-manifests` refuses an imported image in `shared` | every verify, every PR (`lint-licensing`); every verify (`lint-manifests`) | **catches** |
| A shim diverging from the API it imitates | L4 | nothing — differential fuzzing against Linux is not built | never | **GAP** |

## I — the apparatus itself

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| A weakened test — `#[ignore]`, `assert!(true)`, a shrunk repeat count | X | `.claude/hooks/tests-hold.sh`, then `REVIEW.md` pass 2 | every edit, every PR | **catches** |
| A hook that stopped firing | X | `.claude/hooks/selftest.sh`, in the `policy` job and in `agent-evals.yml` | every PR, weekly | **catches** |
| Agent configuration regression | X | `cargo xtask eval`, twenty-two tasks, floor 0.95 | every PR touching `.claude/`, weekly | **partially** |
| A test that exists and runs on no runner | X | `cargo xtask test` is `--workspace --exclude f-kernel`; the CI test jobs name four crates, so `xtask` and `bench` tests run locally and **not in the gate** | every verify | **partially** |
| A kernel fault path that no longer reports | P | `cargo xtask fault pf\|ud\|df\|nx\|wx\|stack` | on demand | **GAP** |
| A panic reaching CI as a hang, or a hang as a pass | P | `cargo xtask panic` — three endings, three fixtures | every verify, every PR | **catches** |
| AArch64-only compile failure | L2, X | `cargo check --target aarch64-unknown-none` over the four crates, inside `xtask test`; the arm job builds and runs them | every verify, every PR | **catches** |
| AArch64-only ordering failure | L2 | `tests (AArch64, weak memory)` and `memory-ordering litmus (AArch64, weak memory)` | every PR | **partially** |
| Toolchain drift | X | `rust-toolchain.toml` is a protected path; the container image reads its pin from that file rather than restating it | every edit, every PR | **catches** |
| A dependency with a bad licence or a known advisory | X | `cargo deny check` in the `deps` job; `security-scan.yml` against the advisory database | every PR, weekly | **catches** |
| A credential in the diff | X | `.claude/hooks/no-credentials.sh`, `REVIEW.md` pass 3, `security-scan.yml` | every edit, every PR, weekly | **catches** |
| Release package non-reproducible across two machines | X | the `package` matrix and the `address` job; `release --twice` is the same-machine half | every PR | **catches** |
| A release that cannot name its own tree | X | `cargo xtask release --dry-run` — version and commit are fatal rather than `unknown` | every PR, per release | **catches** |
| A release crossing the boundary without authorisation | X | `.claude/hooks/release-gate.sh` on every `Bash` | every edit | **catches** |
| Coverage falling on the entry-validation path | L4 | `cargo xtask coverage`, published and watched; nothing gates | every PR | **partially** |
| A hardware-only bug — something QEMU emulates differently | L6, P | `docs/first-boot-outside-qemu.md`: one boot, by hand, on a virtual machine rather than metal | never | **GAP** |
| `instructions_per_op` and `joules_per_op` absent, narrowing a claim | L5, L6 | the harness carries both fields and marks them `Unavailable` rather than omitting them, and the value is only as fresh as the last claim run | on demand | **partially** |
| A drift between what `verify` runs and what the gate runs | X | this page, and the note `verify` prints on its last line | every verify | **partially** |

Two rows in this group were found by writing the table, and both are worth
pulling out because each is one line of YAML away from closing.

`cargo xtask fault` is in **no** automated loop. `verify` runs lint, test, run,
panic, trace and mutate; the `kernel` job runs run, user, cap, mutate and panic.
Neither runs `fault`, so the six deliberate kernel faults — the one verb whose
whole subject is code that executes only after something has already gone wrong
— run when somebody remembers.

And the gate's test jobs name four crates where `cargo xtask test` names the
workspace. That is the same drift `xtask test` was changed to fix: a hand-written
crate list stops matching the workspace the moment a crate is added, and
silently. `f-bench`'s and `xtask`'s own tests — including the manifest schema and
the archive's determinism — run locally and in no CI job.

## Every gap, and who owns it

Each row below is either scheduled by a `TODO.md` task or accepted with a
reversal condition. Nothing is left as "we should probably".

**Scheduled.**

| Gap | Owner |
|---|---|
| A publishing store weakened to `Relaxed`, either ring | `E0-P16` — the stress suite was measured not to catch it |
| An interleaving the stress tests never produce | `E0-P16` — RustMC, on small tests a checker can exhaust, under its own toolchain (RFC 0022) |
| An entry mutated between validation and use | `E1-P02`, `E1-P04` |
| Unknown opcode, flag or reserved bit accepted (R04) | `E1-P05` for the corpus, `E1-P12` for the proof |
| ABI layout drift — a field reordered | `E1-P05` — the committed corpus is the golden-bytes fixture |
| A hostile header or cursor panics the consumer | `E1-P04` (a billion operations), `E1-P12` (panic-freedom, proved) |
| A hostile header accepted rather than refused | `E1-P04` |
| A peer that dies mid-claim, or lies about its epoch | `E1-P04`, `E1-P02` |
| Frame leak under churn | `E1-B14` (the unmap-under-churn workload), `E1-P06`, `E1-B10` |
| Mapping left after revoke, under churn | `E1-B14`, `E1-P02` |
| A ring-3 **component** addressing memory outside its grant | `E1-B02` — the first driver that is a component; `E1-B01` built the mechanism and stood in for the driver |
| Authority arriving by inheritance | `E1-B05` — the first lifecycle that could grant it |
| Speculation across a domain boundary | `E1-B05` — the supervisor refusal RFC 0005 names |
| Deadline inversion in a device queue | `E1-B06` |
| A component claiming urgency forever | `E1-B06`, `E1-B07` |
| A granted reservation missing its deadline | `E1-B07` |
| An over-subscribed reservation admitted | `E1-B07` |
| Timer jitter regression | `E0-P06`, itself blocked on `E0-D10` and `E0-P18` |
| Ring submit latency regression | `E0-P05`; the datapath set is `E1-P10` |
| A regression too small for a threshold | `E2-P09` — change-point detection over the stored history |
| A determinism leak that never reaches the boot log | `E1-P01`, `E2-P05` |
| Correlated streams in a seed sweep | `E1-B11` — a splittable generator, before the sweep multiplies streams |
| A fault-injection site never exercised | `E1-P02`, `E1-P03` |
| Driver death observed by a client | `E1-P06` — where the blast-radius claim becomes gating |
| Supervisor restart storm | `E1-B05`, `E1-P06` |
| `instructions_per_op` and `joules_per_op` absent | `E0-P05` for the PMU, `E5-P03` for the meter |
| A hardware-only bug | `E0-P18` — open; the first boot outside QEMU was a virtual machine, not metal |
| Coverage on the entry-validation path | `E1-P05` — above 95%, as a claim rather than as a threshold |
| AArch64-only ordering failure | `E0-P16`, `E1-P11` |
| A test that runs on no runner | `E0-P01` — the gate's `test` half is narrower than `cargo xtask test` |

**Accepted, each with the condition that reverses it.**

- **A kernel fault path that no longer reports, and W^X or NX on the kernel
  mapping.** Both are the same acceptance, so they are stated once. The
  exception path *does* run in the gate: `cargo xtask user` boots six processes
  that fault and requires the kernel to report each and survive it. What is
  unscheduled is the kernel-mode half — the double-fault stack, the W^X and NX
  faults, the guard page under the fault stack — and those change only when the
  address space or the IDT changes, which is a diff a reviewer can see.
  *Reverse this* the first time a kernel-mode fault path breaks without `user`
  noticing, or when the `kernel` job stops being the longest in the gate and six
  more boots are free. The gate's measured wall clock is 2 m 56 s against a
  ten-minute budget, so the cost argument is thin and this is the weakest
  acceptance on the page.
- **A new `Release`/`Acquire` pair with no litmus test.** No mechanism. A check
  would have to tell a new ordering from a moved one, which is a question about
  a diff rather than about a file, and `xtask` reads files. *Reverse this* when a
  second ordering regression reaches `main`.
- **A fifth cross-core word with no named ordering.** RFC 0016 names four places
  where two cores reach the same slot; a fifth needs an argument, and an
  argument is a review. A lint would have to distinguish an atomic that crosses
  a core from one that does not, which is a claim about the design rather than
  about the text. *Reverse this* when a fifth lands without one.
- **Boot-time regression.** Claim 0003 is `tracked` by design: nothing in the
  thesis rests on boot time, and a tight threshold would fire the first time a
  milestone added a page-table walk the design wanted. The loose 50 ms bound
  exists to catch a tenfold change nobody intended. *Reverse this* if boot time
  becomes an argument in any document, at which point it needs an owner and a
  gate.
- **The `unsafe` share.** `cargo xtask unsafe` reports and does not gate, because
  RFC 0001's trigger is *"exceeds 10% by phase 02"* and the phase is half the
  condition. A verb that went red at phase 00 would be a gate with no path to
  green. *Reverse this* at phase 02: `lint_all` gains a line, as `E0-B21`
  records.
- **A `SAFETY` comment that does not discharge its obligation.** Review, until
  `E2-P04` puts Verus on the frame. A checker for this is a proof checker; there
  is no cheaper version, and pretending otherwise would put a rule in the
  mechanised column that is not mechanised.
- **A number in prose with no claim.** The mechanised direction is the other
  one: `lint-claims` catches a *cited* value that has moved, `lint-reproduce` a
  claim that cannot be re-derived, `lint-claim-owners` one nobody owns. A lint
  cannot tell a number that needs a claim from a number in an example.
  *Reverse this* if a published number without a claim survives a release.
- **A reversal with no RFC.** The same shape, and `REVIEW.md` pass 4 is the whole
  of it. *Reverse this* if a design document is contradicted by code for more
  than one release.
- **Agent configuration regression.** `evals/` measures whether a policy is
  *known*, not whether it is followed in hour three of a long session. That gap
  is stated in `evals/README.md`, and the hooks exist because of it.
  *Reverse this* when a policy violation passes the eval suite and no hook
  catches it.
- **An allow-list entry added in the same diff as the code that needs it.** The
  entry being a reviewable diff *is* the mechanism, and `REVIEW.md` pass 1 names
  this as the commonest way the policy erodes. Whether the stated reason survives
  reading is a judgement no lint makes. *Reverse this* when an entry reaches
  `main` whose reason does not survive being read aloud.
- **A drift between what `verify` runs and what the gate runs.** The two are
  deliberately different — `verify` cannot run the AArch64 tests, the litmus job
  or a second runner — so a check requiring them to be equal would assert
  something false. *Reverse this* when a third check exists in one and not the
  other with no stated reason, which is what `lint-taxonomy` would notice.
- **Crash inconsistency, torn writes, a generation root that does not
  reproduce.** No row at all rather than a row with an empty cell: there is no
  store to cut power to, so a row here would be a placeholder pretending to be
  an assessment. `E2-P01`, `E2-P06` and `E2-P07` own them, and the condition that
  brings them into this table is `E2-B01` landing.
- **A shim diverging from the API it imitates.** Differential fuzzing against
  Linux is named in `proving-ground.html` section 08 and owned by no task,
  because there is no shim. *Reverse this* when the first imported driver is
  reachable over a ring — the point at which "what does Linux do here" stops
  being rhetorical.

## What this table says about E1

Two things, and the second is the one to argue with.

**Which layers E1 builds.** L1 goes from a hook to a simulator: `E1-P01` is
virtual time, seeded ordering and device models; `E1-P02` gives it fault classes
whose responses are asserted rather than observed; `E1-P03` runs seeds nightly
and minimises a failure to a reproduction command; `E1-P08` makes a long
scenario bisectable. L4 goes from instrumentation to fuzzing: `E1-P04` is the
hostile-peer harness and `E1-P05` the structure-aware generator with a committed
corpus. L3 opens at all for the first time: `E1-P07` proves the five capability
properties the negative suite samples, and `E1-P12` proves panic-freedom over
the ring's validation paths. L7 acquires numbers that gate — `E1-P10` is four
datapath claims, against a baseline `E1-D06` has to make configuration rather
than prose. P gains the driver and supervisor rows through `E1-B05` and
`E1-P06`.

L2 is the layer E1 does **not** build, and that is worth saying out loud rather
than leaving to be noticed: `E0-P16` remains the only owner of the gap the
litmus suite was measured to have, and it is an E0 task carried into E1.

**Which rows move.** Of the eighty-five rows, forty-one say *catches* today,
twenty-seven *partially* and seventeen *GAP*. Twenty-three name an E1 task as an
owner, and twenty of those name *only* E1 tasks — so if the rest of the epoch
lands as written, eight more GAPs close and twelve more rows move from
*partially* to *catches*, which is about a quarter of everything on this page.

Four rows have already moved and the counts above include them: the
allocator's split and coalesce row, which `E1-B12` took from *GAP*, the two
capability-table rows, which `E1-B13` took from *partially*, and the IOMMU row,
which `E1-B01` took from *GAP* to *partially* rather than to *catches* — the
mechanism is booted twice on every PR and the requester is the frame's own
adversary rather than a driver component, so the row moves one step and names
`E1-B02` for the other. A table is bounded
by what its holder paid for now rather than by a constant, so a component
outgrowing the fixed count is an ordinary thing it does — `cap flood` holds a
hundred and sixty — and `cap quota` and `cap beyond` are the boots that say
what happens at the two edges of paying.

Three rows name an E1 task and something else, and those are the ones to watch,
because a task can land and leave its row where it was: `ring-latency-regression`
needs `E1-P10` **and** `E0-P05`, `determinism-off-log` needs `E1-P01` **and**
`E2-P05`, and `aarch64-ordering` needs `E1-P11` **and** `E0-P16`. Eight more
scheduled rows have no E1 owner at all. What stays open at the end of the epoch
is that set: change-point detection (`E2-P09`), the frame's proof (`E2-P04`),
hardware in the loop (`E0-P18`, then `E3-P01`), the gate's crate list
(`E0-P01`), and the weak-memory model check.

There is a shape in that list worth naming. Every row this epoch closes is one
where the check is *specific to F* — the simulator, the hostile-peer harness,
the claims. Every row that stays open is one `proving-ground.html` section 13
told us to buy or borrow rather than build: a model checker, a change-point
detector, a photodiode. That is the document's advice arriving as a prediction
rather than as a preference, and it means E1's testing risk is schedule rather
than invention.

## The companion file

`docs/test-taxonomy.toml` carries the same rows in machine-readable form — one
`[[row]]` per bug class, with `layers`, `check`, `cadence`, `status`, and either
`scheduled_by` or `accepted_because` with its `reversal`.

It exists so this page cannot rot silently. A table of what catches what is
exactly the kind of document that is true on the day it is written and quietly
false a quarter later, and the failure mode is invisible: nothing goes red when
a row stops being true, which is the property this repository refuses to accept
anywhere else.

Nothing reads the file yet. A `cargo xtask lint-taxonomy` could — every `check`
naming an `xtask` verb that exists, every `scheduled_by` naming a `TODO.md` task
id that exists and is still open, every `accepted_because` carrying a `reversal`,
and every row in this page present in the `.toml`. That is a follow-up rather
than part of this task, and it is named here so the absence is a decision rather
than an oversight: a lint written the same day as the table it checks is a lint
written against one example.
