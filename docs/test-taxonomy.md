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
  proved it says so — **(sampled)** in the cell. Two different things wear that
  word and the difference is worth one sentence. A check that is total over its
  input is *catches* outright: give it the bug and it fails, every time. A check
  that is total over each input it *sees*, drawn from a generator, is
  **catches (sampled)**: the oracle has no holes, the input set does, and the
  row names the task that owns quantifying over it.
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
| **L1** | Deterministic simulation | Built — `sim/`, `cargo xtask sim`: virtual time, seeded ordering, device models and component substitution (`E1-P01`), seven fault classes each with a scenario and an asserted response (`E1-P02`), and seed sweeps with automatic minimisation (`E1-P03`) — `cargo xtask sweep`, five scenario-independent properties in `sim/src/check.rs`, `sim/corpus.txt` as the regression corpus, and `cargo xtask sweep --mutate` as the evidence the sweep can fail. No snapshot and restore — `E1-P08` |
| **L2** | Concurrency and memory model | Stress tests only — `ring/tests/litmus.rs` on two architectures. Not a model check |
| **L3** | Proof | Absent |
| **L4** | Fuzzing | Two fuzzers and the instrumentation. `E1-P04`'s hostile *peer*: `ring/tests/hostile.rs` draws a peer's behaviour from a seed through `f_env`, `cargo xtask hostile` runs a hundred million operations in `verify` and the exit's billion in CI, `--miri` carries the one property no in-process assertion can make, and `ring/corpus.txt` is its regression corpus. `E1-P05`'s structure-aware *entry*: `ring/tests/entries.rs` generates from fifteen named families with an eighth of the budget on bytes, keeps an input that lights a region nothing had lit — per case, out of the profile runtime's own counters — and `cargo xtask entries --coverage` publishes the share of a named list of thirty-seven functions that `ring/entries-corpus.txt` covers, which `claims/0009` gates at 95%. `--mutate` is the evidence each can fail: three defects, one per oracle |
| **L5** | Performance regression | Harness only — `bench/`, `claims/history.jsonl`. No change-point detection |
| **L6** | Hardware in the loop | Absent — one boot outside QEMU, by hand, on a virtual machine |
| **L7** | Claims registry | Built, eight entries, two of them gating — `claims/0005` (a count of components a driver restart disturbs) and `claims/0008` (hostile operations, and its twenty-six reach minimums). The row said *three entries, none gating* until `E1-P04`, and both halves of that had stopped being true |
| **P** | Provoke the real kernel | Built — `xtask fault`, `user`, `cap`, `mutate`, `panic`, `trace` |
| **X** | Policy made executable | Built — sixteen lints, six hooks, `REVIEW.md`, `evals/`. `cargo xtask lint` prints one `lint-` line per check, so the number is `cargo xtask lint \| grep -c '^lint-'` rather than something to remember. It read *twelve* while four more lints landed, which is what a remembered count does |

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
which is a test in the sense L1 to L6 mean. Forty of the eighty-six rows below
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

Which matters most where a row says *every verify* and nothing after it. Five
lints do: `lint-manifests`, `lint-components`, `lint-datapath`, `lint-owed` and
`lint-reproduce`. Those verbs appear in no workflow file — the set is
`cargo xtask lint | grep -o '^lint-[a-z-]*'` minus what `grep -o 'lint-[a-z-]*'
.github/workflows/*.yml` finds — so for them the local loop is not a convenience
that saves a round trip: it is the only place the check happens, and a
contributor who skips it skips the check. `verify` and the gate are deliberately
different and always will be, but these are different in the direction nobody
intended.

The list used to include a sixth entry that was not a lint: the part of
`cargo xtask test` the gate's hand-written four-crate list did not name. That
one is gone rather than fixed, because there is no four-crate list any more —
both test jobs run `cargo xtask test-host` over the whole workspace, `cross` is
a step in the `policy` job, and `lint-arch-tests` is a step beside it. `E1-P11`,
RFC 0045.

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
| A hostile header or cursor panics the consumer | L4, P, X | `ring/tests/headers.rs`; `litmus.rs::a_hostile_client_cursor_never_panics`; the `unwrap_used`/`panic`/`unreachable` clippy wall over `ring`; and `cargo xtask hostile` — `ring/tests/hostile.rs` draws headers and cursors from a seed, catches a panic per episode so a failure is a seed rather than a backtrace, and `--mutate` arms `mutate-believed-header` and requires it to be found | every verify (10⁸ operations), every PR (10⁹) | **partially** — a billion draws is still sampling |
| A hostile header is accepted rather than refused | L4 | `headers.rs` — every invalid header refused with a structured error, and the region survives the refusal. `hostile.rs` counts each refusal domain and `claims/0008` puts a **minimum** on all four, so a build that stopped producing one of them goes red | every verify, every PR | **partially** — the fuzzer asserts *no panic*, not that a particular drawn header was refused: the cases that assert refusal are still `headers.rs`'s fifteen |
| An entry mutated between validation and use | L1, L4 | `hostile.rs` overwrites whole entry slots and forges index-ring slot numbers against a live channel, and `Consumer::pop` copies an entry out before any field is read | every verify, every PR | **partially** — the write lands *between* operations. A peer writing during one call needs a second thread, which this harness does not have and no task owns |
| Unknown opcode, flag or reserved bit accepted (R04) | L4, X | `abi` unit tests over the reading of `Sqe`; and `E1-P05`'s **envelope oracle** — `entries.rs` computes the refusal R04 owes from the entry's own bytes *before* the code under test sees them and requires the domain and the code to match, over 262 144 generated cases per verify, on `f_ring::execute` and on `f_abi::buf::Request::read` alike | every verify, every PR | **catches** (sampled) — the oracle is total on every entry it sees; the *set* of entries is drawn, and `E1-P12` owns the quantifier |
| A peer that dies mid-claim, or lies about its epoch | L1, L4 | `ring/tests/faults.rs` injects at `ring.publish`, `ring.consume` and `chan.bind`. `cargo xtask sim` runs `peergone`, which kills a device model with work outstanding and requires every buffer to come home and no completion to arrive after the reset. `hostile.rs` restarts the peer between operations and again between a batch being staged and published, and re-adopts at the moved epoch; `claims/0008` puts a minimum on both counts | every PR, both runners; every verify | **partially** — both halves are now driven, and what is still not asserted is a *code*: an operation against a stale epoch is refused, and the fuzzer folds `PEER/EPOCH_CHANGED` in with `Corrupt` rather than requiring that one by name |
| ABI layout drift — a field reordered inside a fixed-size struct | X | `const _: () = assert!(size_of::<Sqe>() == 64)` and its four siblings catch a size change; and `E1-P05`'s `LAYOUT`, twelve `offset_of!` assertions over every field of an `Sqe`, checked before any case is drawn. **`Cqe` and `ChannelHeader` still have only their size checks** | every build, every verify | **partially** |
| A peer at a different ABI version | X | `ChannelHeader::negotiate`, RFC 0011, exercised against a real mapping by `headers.rs` | every verify, every PR | **catches** |

Most rows here are "partially" for one reason: what exists samples, and what is
owed proves. `E1-P04` has landed — a billion generated hostile operations with no
panic and no hang, and Miri's verdict on memory unsafety at four thousand — and
not one of *its* rows reached **catches**. That is the honest outcome rather than
a disappointing one: a billion draws is a large sample and a sample is what it
stays.

The R04 row is the one exception and it is a narrow one, so the cell says
**catches (sampled)** rather than **catches**. What changed there is not the
sample size but the *oracle*: `E1-P05` computes the refusal R04 owes from an
entry's own bytes before the code sees them, so every entry it draws is checked
rather than counted, and `mutate-ignored-flag` is the evidence it can fail. The
input set is still drawn. `E1-P12` asks for panic-freedom over arbitrary header
bytes, cursors and entries as a bounded proof, and that is what takes any row
here to **catches** with nothing in brackets. The distance between them is the
distance between *no fuzzer found one* and *there is none*.

The layout row is the quiet one. A reorder inside `Sqe` keeps `size_of` at 64
and changes what every byte means to a peer built at another commit. `E1-P05`'s
`LAYOUT` closed that for `Sqe` — twelve `offset_of!` assertions, checked before
any case is drawn, and there because the fuzzer writes an entry's wire image at
fixed offsets and would otherwise have gone on flipping bits in a mislabelled
field while every run stayed green. `Cqe` and `ChannelHeader` still have only
their size checks, and nothing in the tree would notice a reorder in either. It
costs nothing while both ends of every channel come from one tree, and stops
costing nothing at `E1-B05`.

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
| A driver addressing memory outside its grant (IOMMU) | L1, P | `cargo xtask iommu` — two boots on a machine with a VT-d unit and a real virtio-blk device, with the frame's own adversary as the requester: one descriptor inside the grant, which must land bytes, and one outside it, which must be refused and recorded in the unit's own fault registers. `cargo xtask blk` — three boots with the requester's descriptor written by a driver **component**, `user/virtio-blk`, out of a `Reach` the frame answered its client's registration with: one carries a sector out and back through a ring byte for byte, one withdraws the client's page from the driver's domain between the write and the read (RFC 0024's reclaim, the frame's property), and one has the driver add a frame to the address it was answered before writing it into a descriptor, requiring the unit to fault at the address the driver invented — the only one of the five that is a driver *reaching* outside its grant. All five halves also require the frame to refuse a device translation for a capability carrying no `GRANT` | every PR | **partially** — the descriptor and the arithmetic behind it are a driver component's own, and the component's *code* is still called by the frame because nothing routes a device into a spawned component's address space — E1-B08 landed the scheduling half, so a component now runs at ring 3 with its own polling loop, and RFC 0038 names the routing half as what is left: four register windows and a DMA region mapped into a spawned component with its IOMMU domain programmed. Both commands are jobs in `ci.yml`, so all five halves keep the cadence claimed here |
| A device writing outside the driver's grant, on the direction the device chooses when | L1, P | `cargo xtask net` — three boots on a machine with a VT-d unit and a real virtio-net device, driven by `user/virtio-net` at ring 3 on a core the frame allocated it: `inside` forms an ARP request in a registered buffer, posts a receive, and requires the reply to land **in that buffer** carrying the MAC this boot invented; `silent` is the identical client with the transmit removed and requires the buffer back untouched as a cancellation, which is what makes `inside` a reply rather than a link; `escape` displaces the receive *data* descriptor by one page and requires the unit to fault at the address the driver invented, on a **write**, with the client's buffer still holding all 42 poison bytes. All three also require `copies = 0` beside a non-zero `provoked`, and a registration of a capability with no `GRANT` to be refused | every PR | **catches** — with three bounds. The component is *scheduled* and not spawned into a place (`CHAOS_GAP`). Every boot posts exactly one receive buffer, so the multi-slot machinery is covered by `f_virtio_net::driver`'s unit tests against a memory-backed device and by no boot. And `RECEIVE_MICROS` is wall-clock, so a red carrying `Trouble::NoFrame` is a wedged component or a slow runner, and says so on its own line |
| A device putting bytes somewhere no code in this system can read them back from | L1, P | `cargo xtask gpu` — three boots on a machine with a VT-d unit and a real virtio-gpu device, driven by `user/virtio-gpu` at ring 3, with the harness capturing the emulator's framebuffer over its monitor socket while the boot holds still: `inside` fills one buffer of a registered set with a 16x16 gradient, submits one entry, and requires the capture to be 16x16 and to hash to the number the kernel printed for the client's own pixels; `blank` is the identical client with the submission removed and requires the capture **not** to hold them; `escape` displaces the resource's backing entry by one page and requires the unit to fault at the address the driver invented, on a **read**, with the capture still not holding the client's pixels. All three also require `copies = 0` beside a non-zero `provoked`, the client's buffer back unwritten, and a registration of a capability with no `GRANT` to be refused | every PR | **catches** — and it is the one row where the kernel does not reach its own verdict, because a scanout cannot be read back from inside the machine. The escape half's evidence is deliberately not the device's word: this emulator answers `OK` for a backing the unit refused and flushes bytes that are not the client's. RFC 0054 |

The five properties hold and each has something that breaks it, which is
`E0-P08` met. What none of *them* is, is a proof: eleven boots sample a space,
and the honest reading of a green `cap` run is that the escapes somebody thought
of were refused. `E1-P07` added the other shape beside them rather than in place
of them — `kernel/proofs` compiles `kernel/src/cap.rs` a second time under Kani
and `cargo xtask prove` states the five over arbitrary handles and the whole
rights lattice, with `mutate-unchecked-index` required to fail one of them, on
the nightly schedule. Two halves stay checked and unproved and are named where
they are: a bought slot across a process boundary, and the page size
`total_bought` holds at. RFC 0053.

The `unmap` row changed character in this epoch, and not in the direction it was
expected to. `E1-B14` wrote the workload where one page, one IPI would stop
being enough — registered buffers cycling, a driver restart retiring a whole
grant — and the workload found that the churn issues *no* shootdowns at all: a
registered buffer set is a device translation, so retiring one edits the
remapping unit's tables and no processor's. So the `cap unmap` row above is
still the only boot that shoots down, and it is still one page. What the churn
did buy is one global invalidation per request instead of one per page (RFC
0052, `claims/0014`), and what it now observes is in the *memory* section
below: the unit's own tables read back after each retirement, and the free count
either side of the whole thing. The half it still does not observe is a device
faulting after a batched multi-page unmap, which `REVOKE_GAP` declares.

## D — memory

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Frame leak on process death | P | `process::reap` fails the boot if the count does not return; the boot line reports `user frames N given back, free count unchanged` | every verify, every PR | **catches** (one death) |
| Frame leak under churn | L1, L5, P | `cargo xtask churn` — the allocator's free count before the churn and after every set is retired, the domain released and the workload's own memory handed back; the boot fails if they differ. Forty register-and-retire cycles per half, each building and clearing second-level tables in the remapping unit | every verify, every PR | **partially** — `E1-B14` closed the registration path and `claims/0014` bounds it at zero leaked frames; the churn `E1-P06` and `E1-B10` name is components dying and being refilled under load, where the frames are a process's rather than a domain's |
| An unmap that invalidates the remapping unit once per page under datapath churn | L1, P | `cargo xtask churn` — one boot runs the churn under both invalidation policies over the identical geometry and requires each to mean what it says: the control once per page, the candidate once per request. `claims/0014` bounds both, plus the round trips saved per set and a minimum of two pages per set — at one page the two halves are the same run | every verify, every PR | **catches** — the *mapping* half of the same cycle is still one invalidation per page, declared as `CHURN_GAP` with its number in `claims/0014` |
| Mapping left after revoke, under churn | L1, P | `cargo xtask churn` — every retirement is followed by a walk of the unit's own second-level tables requiring the set's eight pages to be gone, and every registration by the same walk requiring that they are there, plus a pass that makes a set with a page taken out from under it and requires one batched request to leave nothing translated beyond the hole; `claims/0014` bounds all of it | every verify, every PR | **partially** — the tables are read back, a **device** is not. Nothing is attached to the churn's domain, and the boot that does watch a device fault after a withdrawal (`cargo xtask blk`'s `outside` half) registers one page, where the batched and per-page policies are the same run. `REVOKE_GAP` in `xtask` is that residual, declared |
| Allocator split/coalesce corruption | L1, P | `cargo xtask run` — `mem::self_test` phases two to four: an `Env`-driven mix of orders, a probe upward from order 9, and a 2 MiB block handed back as 512 shuffled frames that must reappear as a 2 MiB block. `cargo xtask orders` boots a machine with a gibibyte in it and requires order 18 | every verify, every PR | **catches** |
| Kernel-global mutable state outside `PerCpu` | X | `cargo xtask lint-percpu` | every verify, every PR | **catches** |
| A fifth word crossing a core with no named ordering | X | `REVIEW.md` pass 2, RFC 0016; `lint-percpu` does not count atomics | every PR | **partially** |
| W^X or NX broken on the kernel mapping | P | `cargo xtask fault nx`, `cargo xtask fault wx` | on demand | **GAP** |
| Page tables leaked by the higher-half on-ramp | P | the free count is asserted across the switch — `arch/x86_64/paging.rs` | every verify, every PR | **catches** |

## E — time and scheduling

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| Deadline inversion in a device queue | L1, L5 | `cargo xtask deadline` — three boots of one client script against one ring-3 `virtio-blk` component, and one of them is a control: the ordered half must return the hard-class read at position 0 of 7 having overtaken the 6 batch reads it was submitted behind, the arrival half must return the identical burst with the read last having overtaken 0, and the unadmitted half must refuse a class the client does not hold rather than serve it. The overtake is read twice — the frame's completion order and the component's own queue counter — and required to agree. `claims/0012` gates on the counts; the depth of six is a fixture and the *time* it saved is `claims/0013`, `pending` | every verify, every PR | **catches** |
| A component claiming urgency forever | L1, L3 | `abi/src/deadline.rs`'s `the_depth_bound_is_enforced`, and that is a unit test. `E1-B06` landed the ordering and did not close this: there is no chain here for RFC 0025's bound 4 to decay along. The absence is checked rather than assumed — `DEADLINE_DEPTH_GAP` in `xtask` goes red the day anything outside `abi/` forwards an inherited class | every verify, every PR | **GAP** |
| A granted reservation misses its deadline under load | L1, L5, L6 | nothing — there is no admission control | never | **GAP** |
| An over-subscribed reservation is admitted | L3, P | nothing | never | **GAP** |
| Timer jitter regression | L5, L7 | `cargo xtask timer 60`; claim 0002 is `pending` and gates nothing | on demand | **GAP** |
| Ring submit latency regression | L5, L7 | `cargo xtask claim ring-submit-latency`; claim 0001 is `pending`; `bench` refuses to record under `F_ENVIRONMENT=container` | on demand | **GAP** |
| Boot-time regression | L5, L7 | claim 0003, `tracked`, `cargo xtask claim boot-to-m0`, threshold 50 ms | on demand | **partially** |
| A kernel entry on the hot path that nobody counted | L5, P | `cargo xtask runtime` — four boots. `load` requires zero door calls and zero ring-3 faults across sixteen thousand work items a component scheduled inside its own allocation; `provoke` runs the same load with one crossing on purpose and requires the frame's count and the component's own to be non-zero and equal, taken on opposite sides of the boundary; `reclaim` posts RFC 0008's notice from the timer handler under load and requires parking at an allocation boundary within one quantum, ringing a doorbell as it goes so the non-timer interrupt bucket is a number a boot can move; `hostile` scribbles the control ring header and requires a structured refusal. All five buckets are published as `state::node::RUNTIME_*` | every PR | **catches** |
| A regression too small for a threshold | L5 | nothing; `claims/history.jsonl` is accumulating for it | never | **GAP** |

R08 is the rule this group exists to keep honest: a hard class with no admission
test is a hint with a better name. Four of these nine rows are empty because
the thing they would test does not exist yet, and that is the correct state at
the start of E1. What would not be correct is for them to be absent from the
table, because that is how a promised layer acquires no owner.

The ninth row is the newest, and it is in this group rather than in group I
because what it catches is a property of the system and not of the apparatus:
*nothing the code at ring 3 does reaches the frame*. It arrived with four
buckets and three interrupt vectors counted in none of them — entries that were
neither on the hot path nor excluded from it — which is the failure the row now
exists to make visible, and RFC 0038 records it as a scar rather than a
correction.

## F — determinism

| Bug class | Layers | Check today | Cadence | Status |
|---|---|---|---|---|
| A clock, `HashMap` or `rdtsc` outside `Env` | L0, X | `.claude/hooks/determinism-guard.sh`, then `cargo xtask lint-determinism` | every edit, every verify, every PR | **catches** |
| A source of nondeterminism no pattern names | L0, P | `cargo xtask trace` locally; the two-runner `trace` and `reproduction` jobs in CI | every verify, every PR | **catches** |
| A determinism leak that never reaches the boot log | L0, L1 | nothing — the trace hashes the boot log, and the state tree deliberately publishes nothing that varies with time | never | **GAP** |
| Correlated streams in a seed sweep | L1 | per-site streams in `env/src/sim.rs`, plus the independence test | every verify, every PR | **partially** |
| A fault-injection site that is never exercised | L1 | `f_sim::fault` requires every class to have a scenario, every armed scenario to strike at three seeds, and every class to be declared by a protocol that reads it — an arming a device would ignore never fires and fails the second check; `ring/tests/faults.rs` requires each of its three sites to be injected at; `env/src/sim.rs` counts and reports sites past its fixed table rather than dropping them | every verify, every PR | **catches** |
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
| A test that exists and runs on no runner | X | `cargo xtask test-host` — the whole workspace minus the crates `PORTABILITY` excludes with a reason — on the x86-64 runner and on the arm runner, and locally inside `cargo xtask test`; and `cargo xtask lint-arch-tests` for the half that one cannot see, which is a test inside an included crate compiled on one architecture only | every verify, every PR | **catches** |
| A kernel fault path that no longer reports | P | `cargo xtask fault pf\|ud\|df\|nx\|wx\|stack` | on demand | **GAP** |
| A panic reaching CI as a hang, or a hang as a pass | P | `cargo xtask panic` — three endings, three fixtures | every verify, every PR | **catches** |
| AArch64-only compile failure | L2, X | `cargo xtask cross` — every workspace member compiled for `aarch64-unknown-none`, the list derived from `Cargo.toml`'s members — in the `policy` job and inside `cargo xtask test`; the arm job builds and runs the same suite the x86-64 job runs | every verify, every PR | **catches** |
| AArch64-only ordering failure | L2 | `tests (AArch64, weak memory)` and `memory-ordering litmus (AArch64, weak memory)` | every PR | **partially** |
| Toolchain drift | X | `rust-toolchain.toml` is a protected path; the container image reads its pin from that file rather than restating it | every edit, every PR | **catches** |
| A dependency with a bad licence or a known advisory | X | `cargo deny check` in the `deps` job; `security-scan.yml` against the advisory database | every PR, weekly | **catches** |
| A credential in the diff | X | `.claude/hooks/no-credentials.sh`, `REVIEW.md` pass 3, `security-scan.yml` | every edit, every PR, weekly | **catches** |
| Release package non-reproducible across two machines | X | the `package` matrix and the `address` job; `release --twice` is the same-machine half | every PR | **catches** |
| A release that cannot name its own tree | X | `cargo xtask release --dry-run` — version and commit are fatal rather than `unknown` | every PR, per release | **catches** |
| A release crossing the boundary without authorisation | X | `.claude/hooks/release-gate.sh` on every `Bash` | every edit | **catches** |
| Coverage falling on the entry-validation path | L4 | `cargo xtask entries --coverage`: the share of a named list of thirty-seven functions that `ring/entries-corpus.txt` covers, per function, out of `llvm-cov`. `claims/0009` is gating and states the floor; `cargo xtask coverage` still publishes the per-crate figure beside it and still gates nothing | every PR | **catches** |
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
| An entry mutated between validation and use | Narrowed by `E1-P04` and then **not owned**: `hostile.rs` scribbles entry slots against a live channel between operations, and the residual is a peer writing *during* one call — a second thread this harness does not have. Accepted with a reversal condition: a finding that needs the write to land inside a call, or a `Consumer::pop` that stops copying the entry out before reading a field |
| Unknown opcode, flag or reserved bit accepted (R04) | Closed by `E1-P05`'s envelope oracle for a single entry; `E1-P12` still owns the proof, because the oracle samples entries and does not quantify over them |
| ABI layout drift — a field reordered | Closed for `Sqe` by `E1-P05`, and **not by the corpus** — a corpus line names fields rather than bytes, so it would move with a reorder instead of catching one. What closes it is `LAYOUT`, twelve `offset_of!` assertions the fuzzer needs for itself. `Cqe` and `ChannelHeader` are unowned |
| A hostile header or cursor panics the consumer | `E1-P12` (panic-freedom, proved). `E1-P04` landed the billion operations and did not close the row, because sampling does not close it |
| A hostile header accepted rather than refused | `E1-P12`. `E1-P04` landed and made every refusal domain a counter with a minimum in `claims/0008`; what it does not assert is that a particular drawn header was refused |
| A peer that dies mid-claim, or lies about its epoch | Both halves are driven — `E1-P02`'s `peergone` for the dying half, `E1-P04`'s restarts and moved epochs for the lying half. What is left is a code: `PEER/EPOCH_CHANGED` required by name rather than folded in with `Corrupt`, which is a line in `headers.rs`. `E1-P05` was named here as the other candidate and is not: an entry fuzzer's cases are self-contained by construction, so nothing in it holds a channel across an epoch change |
| Frame leak under churn | **Partially closed by `E1-B14`** for the registration path — `cargo xtask churn` requires the allocator's free count to return across forty register-and-retire cycles per half, and `claims/0014` bounds it at zero. `E1-P06` and `E1-B10` own the rest: components dying and being refilled under load |
| Mapping left after revoke, under churn | **Partially closed by `E1-B14`**, and the residual is declared rather than owned. `cargo xtask churn` walks the remapping unit's own second-level tables after every retirement and requires the set's pages to be gone, with the registration walk beside it so the zero is not a walk that answers no to everything. What no boot observes is a **device** faulting after a batched multi-page unmap: nothing is attached to the churn's domain and `cargo xtask blk`'s `outside` half registers one page. `REVOKE_GAP` in `xtask` goes red when either changes. `E1-P02`'s `peergone` still covers the *model*'s half |
| A ring-3 **component** addressing memory outside its grant | `E1-B02` — the first driver that is a component; `E1-B01` built the mechanism and stood in for the driver |
| Authority arriving by inheritance | `E1-B05` — the first lifecycle that could grant it |
| Speculation across a domain boundary | `E1-B05` — the supervisor refusal RFC 0005 names |
| A component claiming urgency forever | `E1-B05` or `E1-B07` — whichever first has a service submitting downstream on a caller's behalf. `E1-B06` was named here and landed without closing it: RFC 0025's bound 4 needs a chain, this tree is a client and a leaf, and `DEADLINE_DEPTH_GAP` now says so as a check rather than as a sentence |
| A granted reservation missing its deadline | **Partially closed by `E1-B07`**, and the residual is a number rather than a mechanism. `cargo xtask admission` runs three arms at one seed and gates on the counts: the granted arm meets every period, the unreserved arm must *miss* — without which the first proves nothing — and the over-subscribed arm must be refused. All of it is on a virtual clock, so *met its deadline* is a count of slots. The margin in nanoseconds, on a part that can deliver all four of RFC 0007's components, is `claims/0011` and is `pending` on `E0-D10`'s machine. On QEMU there is no sibling, no cache topology and no RDT, so the boot half reports that this machine hosts no hard-class reservation at all |
| An over-subscribed reservation admitted | Closed by `E1-B07`. `f_abi::reserve::Table::admit` refuses in the `ADMISSION` domain naming which of RFC 0007's four components could not be delivered, and `claims/0010` gates on two rows rather than one — a minimum on refusals *and* a maximum of zero on periods run — because a reservation admitted and then missed would satisfy the first alone |
| Timer jitter regression | `E0-P06`, itself blocked on `E0-D10` and `E0-P18` |
| Ring submit latency regression | `E0-P05`; the datapath set is `E1-P10` |
| A regression too small for a threshold | `E2-P09` — change-point detection over the stored history |
| A determinism leak that never reaches the boot log | `E1-P01`, `E2-P05` |
| Correlated streams in a seed sweep | `E1-B11` — a splittable generator, before the sweep multiplies streams |
| Driver death observed by a client | `E1-P06` — where the blast-radius claim becomes gating |
| Supervisor restart storm | `E1-B05`, `E1-P06` |
| `instructions_per_op` and `joules_per_op` absent | `E0-P05` for the PMU, `E5-P03` for the meter |
| A hardware-only bug | `E0-P18` — open; the first boot outside QEMU was a virtual machine, not metal |
| Coverage on the entry-validation path | Closed: 96.95% of thirty-seven named functions, from the committed corpus alone, gating as `claims/0009`; the fifteen missed lines are listed by name and mechanism in RFC 0048 |
| AArch64-only ordering failure | `E0-P16` — `E1-P11` landed and the row did not move: the arm runner now runs the whole host suite rather than four crates, so more code is exposed to weak memory, and the suite is still stress tests rather than a model check |

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

**Which rows move.** Of the eighty-nine rows, fifty say *catches* today,
twenty-seven *partially* and twelve *GAP* — counted from the `status` fields in
`docs/test-taxonomy.toml` rather than by hand, because this sentence is the part
of the page that goes quietly false first, and `E1-P09`'s lint is what will one
day read them from there instead of from here. Twenty-two name an E1 task as an
owner, and nineteen of those name *only* E1 tasks — so if the rest of the epoch
lands as written, eight more GAPs close and eleven more rows move from
*partially* to *catches*, which is about a quarter of everything on this page.

`E1-B14` moved two more, and the counts above include them: *frame leak under
churn* and *mapping left after revoke, under churn* went from GAP to
*partially*, both on the strength of one workload — `cargo xtask churn` reads
the allocator's free count either side of the churn and walks the remapping
unit's own tables after every retirement — and both keep a named residual rather
than a rounded-up status, `E1-P06`/`E1-B10` for the first and `REVOKE_GAP` for
the second. A third row, *an unmap that invalidates the unit once per page under
datapath churn*, is new and says *catches*.

Five rows have already moved and the counts above include them: the
allocator's split and coalesce row, which `E1-B12` took from *GAP*, the two
capability-table rows, which `E1-B13` took from *partially*, the fault-injection
row, which `E1-P02` took from *partially* by closing all three ways a site goes
unexercised — a class with no scenario, a scenario whose class never fires, and
a class armed against a device whose protocol never reads it — and the IOMMU
row,
which `E1-B01` took from *GAP* to *partially* rather than to *catches* — the
mechanism is booted twice on every PR and the requester is the frame's own
adversary rather than a driver component, so the row moves one step and names
`E1-B02` for the other. A table is bounded
by what its holder paid for now rather than by a constant, so a component
outgrowing the fixed count is an ordinary thing it does — `cap flood` holds a
hundred and sixty — and `cap quota` and `cap beyond` are the boots that say
what happens at the two edges of paying.

Two rows name an E1 task and something else, and those are the ones to watch,
because a task can land and leave its row where it was: `ring-latency-regression`
needs `E1-P10` **and** `E0-P05`, and `determinism-off-log` needs `E1-P01` **and**
`E2-P05`. A third did until this epoch and is the worked example of the risk:
`aarch64-ordering` needed `E1-P11` **and** `E0-P16` — E1-P11 has
landed and the row did not move, which is the case this paragraph was written
about: the arm runner now runs the whole host suite rather than four crates, and
the suite is still stress tests rather than a model check, so `E0-P16` is what
changes the status. Eight more
scheduled rows have no E1 owner at all. What stays open at the end of the epoch
is that set: change-point detection (`E2-P09`), the frame's proof (`E2-P04`),
hardware in the loop (`E0-P18`, then `E3-P01`), and the weak-memory model
check.

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
