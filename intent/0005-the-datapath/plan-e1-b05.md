---
id: 0005
task: E1-B05
status: in-progress
spec: ./spec.md
---

# Plan: a supervisor is a component, and the frame answers its ring

`E1-B05` is one task line and seven increments. This plan fixes the order and
the shape of each, because the increments are not independent — increment 2
cannot be written before increment 1 decides where the opcodes are answered,
and increments 5 to 7 each turn a declared quantity red on purpose and must
carry the documents that quantity names.

## What changed before this plan was written

**The wall the task line states is gone, and the line has not been corrected.**
`TODO.md` E1-B05 says a supervisor cannot drive a control ring because
`f_ring::Mapping::adopt` is `unsafe` and a `user/` crate may not write `unsafe`.
RFC 0037 ended that. `ring/src/adopt.rs` exports `Adopted`, `Client` and
`Server`; `Adopted::at` is safe at `ring/src/adopt.rs:150`; and four components
already use it — `user/store/src/runtime.rs:13`, `user/store/src/component.rs:24`
and the three virtio drivers. `kernel/src/component.rs:39-41` says so in the
frame's own words.

What is actually missing is what `kernel/src/component.rs:44-56` names: **there
is no supervisor component.** Three things, and they are one thing.

- Nothing submits `f_abi::control::op::SPAWN` (0x14) or `op::STOP` (0x16). The
  opcodes are specified in `abi/src/control.rs:82` and `:99` down to what the
  completion carries, and the ring that would carry them works.
- Nothing reads `notice::PEER_GONE`. `tear_down` posts it with a cause on every
  teardown; the notice is there and the reader is not.
- A supervisor is a component, so it needs a place, an account and a manifest,
  and the `Untyped` behind that account is what replaces `PLACES_MAX` = 4 and
  `SUPERVISOR_ORDER` = 2 (`kernel/src/component.rs:201`, `:128`). RFC 0044 names
  all three as one deviation.

## The decision this plan makes before any code

**`op::SPAWN` and `op::STOP` are not arms on `Supervising::execute`.**

That is the frame's only control-ring server today
(`kernel/src/supervisor.rs:376`), and it is the obvious place to put them and
the wrong one. `Supervising` is device-shaped by construction: it borrows a
`Unit`, a `vtd::Domain` and the *client's* `Table`, and its two opcodes both go
through one `iommu::Grant`. Its own module comment says there is no version of
it that lives above the frame, which is a statement about what a driver
supervisor is.

A spawn needs none of that and needs four things `Supervising` does not hold:
the places array, the account the frames are charged to, the **supervisor's**
table — not the client's, which is the table `Supervising` deliberately carries
— and the `Reservations`. `component::spawn`'s signature
(`kernel/src/component.rs:1769`) is the list.

So the frame gets a second server, in `kernel/src/component.rs` beside the
machinery it drives, and the two servers stay apart. Writing it as a fifth and
sixth arm on `Supervising` would mean widening that struct to carry places and
an account for two opcodes that never touch a device, and would put the
capability-granting path and the device-translation path behind one `match` —
which is the merge RFC 0071 refused one file over, for the same reason.

This is a decision a later contributor would otherwise re-litigate, so it is
RFC 0073 (increment 1) and not a comment.

## Order of work

Each increment ends at a gate. An increment that cannot state its gate is not
ready to start.

### 1 — RFC 0073, the decision above

**Files:** `docs/rfc/0073-a-supervisor-is-a-component-and-the-frame-answers-its-ring.md` `NEW`.

Carries: why a second server rather than a fifth arm; that `op::SPAWN`'s
completion hands back an `Endpoint` handle, which is the one opcode that
*creates* a capability from the frame; and the reversal condition — a third
control-ring server would mean the split is by accident rather than by kind, and
the two should then merge behind a router.

**Gate:** the RFC is accepted and `cargo xtask lint` is green. The number is
allocated here, before any branch, because two worktrees both took RFC 0065 and
both stamped a count at byte 101.

### 2 — the server, and `op::STOP` *(done: `77467f3`)*

**Files:** `kernel/src/component.rs`.

**Amended after reading the code, and the original text is worth keeping because
it was wrong in two ways a plan written from memory usually is.**

It said the gate was *a unit test in the frame's own test module*. `kernel/` is
`test = false` and carries **zero** `#[cfg(test)]` blocks; the idiom here is a
`self_test` run at boot and asserted from the log, which is what
`cargo xtask run` reads. There is no test module to put a test in.

And it put `op::SPAWN` before `op::STOP`. `STOP` needs only the place and a
deadline; `SPAWN` needs the arena walked for a capability supply. `STOP` is
therefore what makes the server real at the smallest size, and a server that
answered *nothing* — the original increment 2 — could not be written at all
without dead fields under `-D warnings`. So increments 2 and 4 are one.

A `Serving` struct holding `&mut Place`, `&mut Table`, and what a spawn needs
besides, with `execute(&Sqe) -> Cqe` answering `STOP` and refusing everything
else with `UNKNOWN_OPCODE`.

**Gate, met:** the boot's stop goes through the server, and two refusals are
asserted rather than printed — a boot line per refusal would move the trace hash
for a check. The teardown stays outside the arm so the boot log is byte-identical
and an unmoved trace hash is the evidence that the path moved and the behaviour
did not.

### 3 — `op::SPAWN` *(done)*

**Files:** `kernel/src/component.rs`.

The arm resolves `entry.cap` to the paying `Untyped`, reads `entry.ext[0]` as the
manifest content hash, and calls the existing `spawn`. Every refusal is already
implemented and tested — `check_needs` and `admit` are what the boot's six
deliberate refusals exercise — so this arm adds no policy, only a route to it,
plus the two checks that are about the *entry* rather than the manifest.

**What the original text got wrong here too:** it said the arm *walks the arena*
for the supplied handles. There is no arena to walk — the boot builds these
entries directly, and `offer` still runs on the frame's side. That is the frame
holding ground RFC 0008 says is the supervisor's, exactly as `policy` is, and it
moves in increment 7 with `policy` rather than here.

**Gate, met:** the boot spawns through the ring and the existing
`refusals 6 spawn(s) refused on purpose` line still reads 6; the `spawn place
store epoch 0` and `supervisor ok` lines are byte-identical.

### 4 — *(folded into increment 2)*

`abi/src/control.rs:99` already fixes the semantics: a stop with `NO_DEADLINE`
is an `ARGUMENT` refusal, and a stop whose deadline has passed is a kill spelled
the same way as a polite stop.

**Gate:** the boot's `stop place store epoch 1 stopped against a deadline
already behind it` line is produced through the ring rather than by a direct
call, and a `NO_DEADLINE` stop is refused.

### 5 — `user/supervisor/`, the component

**Files:** `user/supervisor/Cargo.toml` `NEW`, `user/supervisor/manifest.toml`
`NEW`, `user/supervisor/src/lib.rs` `NEW`, `user/supervisor/src/component.rs`
`NEW`, `xtask/src/main.rs` (`COMPONENTS`), `user/generation.toml`.

Modelled on `user/store`, which is this tree's worked example of a component
with a lifecycle: `Heap::COMPONENT` as its `#[global_allocator]`, its control
ring adopted with `f_ring::adopt::Server`, a `[[state]]` block so its tree is
readable, and a `heap` need so it has memory to decide with.

**Gate:** `cargo xtask lint-components` and `lint-manifests` green with five
components, `lint-generations` round-trips to a fixpoint, and the boot spawns it
into a place. It does nothing yet — it is spawned and not scheduled, like every
other component, which is what increment 6 changes.

### 6 — the join: a scheduled occupant

**Files:** `kernel/src/component.rs`, `kernel/src/runtime.rs`, `kernel/src/main.rs`.

This is the increment the whole task is about, and `kernel/src/runtime.rs:63`
states it: *there, a component is spawned into a place and never scheduled;
here, a component is scheduled and never spawned into one.* A place's occupant
is handed a core.

**Amended after reading `process.rs`, and RFC 0075 is the decision that came out
of it.** This section used to say the work was making an `Instance` produce a
`Prepared`, since the scheduled path already knows how to run one. That is a
double free: `reap` returns `Prepared::pages` to the `FrameAllocator` and checks
the free count in `Prepared::before`, while an occupant's pages were derived
from an account and are owed back to it by `tear_down`. The convenience method
would be a value whose whole purpose is to be handed to a function that must
never see it.

What the code actually needs is much smaller. `smp::run_on` does not take a
process — it publishes a `process::Job` into a per-CPU slot and the target core
reads it: `{ root, entry, stack, argument, hz, target }`, six fields, every one
of which an `Instance` has or trivially knows. So the join is a `Job` built from
the occupant, and `Prepared` is not involved at any point.

**Files, corrected:** `kernel/src/component.rs` and `kernel/src/process.rs`
(a second `Job` construction site). `kernel/src/runtime.rs` is **not** touched —
its half already works, and the sentence in its module comment is `HEAP_GAP`'s
needle, so editing it would turn that constant red without the boot having
changed, which is a red build for a false reason.

**Gate — one declared quantity going red, on purpose:**

- `HEAP_GAP` (`xtask/src/main.rs`). Its needle is the sentence at
  `kernel/src/runtime.rs:63`. When an occupant is scheduled, `user/store`'s
  64-byte box executes for the first time and the boot's `peak 0 byte(s)`
  becomes non-zero. `cargo xtask run` is written to refuse exactly that and to
  say it is the good ending. **This is the acceptance test for increment 6**, and
  it is why this work is based on the branch that declared it. The constant's
  fourth field names the documents to update in the same diff.

**Outcome, and the gate is not met.** The mechanism landed in `9ddde09`: the
supervisor's occupant enters ring 3 under its own address space on every boot,
which is the first half of RFC 0033's sentence becoming false. It then dies on
its first instruction — `exception 14 at 0x410ff8, error 0x6`, a stack probe
walking `0x4008` past `SPAWN_STACK_TOP` into the guard page. So `peak` is still
zero, `HEAP_GAP`'s needle is still in the tree, and this increment is open.

What closes it is `SPAWN_STACK_PAGES`, and it is a diff of its own rather than a
line here: four is pinned by three compile-time assertions to constants owned by
`f_ring` and the two virtio drivers, so it moves in lockstep with them or the
frame it is too small for shrinks instead.

Two things this increment found that the RFC did not anticipate, both now in it:
writing `Job` alone is not scheduling — a core needs five per-core shards — and
the capability table is per-core rather than per-instance, so scheduling swaps it
in and restores the core's previous one afterwards. That restore also explains a
non-determinism a sibling session reported: two boots of one unchanged binary
disagreeing, which `cargo xtask trace` no longer reproduces.

`CHAOS_GAP` is **not** in this gate, and the earlier text was wrong to put it
there. Its needle is `prepare_driver(` in `kernel/src/blk.rs`, and it closes when
a *driver* is scheduled inside the place its manifest is spawned into — which
this increment makes possible and does not do. `E1-P06`'s remaining half stays
open here.

`E2-B10`'s exit — a boot in which a component allocates — is met by this
increment, and the line is the originator's to tick.

### 7 — the supervisor spawns *(done)*

**Files:** `kernel/src/component.rs`, `kernel/src/process.rs`,
`user/supervisor/{manifest.toml,src/component.rs,src/lib.rs,src/routing.rs}`,
`Cargo.toml`, `kernel/Cargo.toml`, `xtask/src/main.rs`, the three driver
`routing.rs` files.

The frame holds one place open — `virtio-gpu`'s, picked by `held_open` for three
stated reasons — schedules `user/supervisor` on a worker core, and answers the
`op::SPAWN` that component submits on its own control ring. The boot:

```
held          virtio-gpu — place built and admitted against a 4194304 B account,
              occupant left to the supervisor (RFC 0008)
scheduled     place supervisor on core 1 — it announced itself from ring 3
occupant      ended itself with status 0
supervised    the supervisor submitted 1 spawn(s) from ring 3 and could not submit 0;
              the frame answered 1, filled 1 place(s), last refusal 0x00000000
spawn         place virtio-gpu epoch 0 — manifest 0x218eee25ee616281, 4 need(s) supplied
```

**Four things this increment found, each of them a check working.**

*`AUTHORITY/RIGHT_NOT_HELD`.* `Serving` had one field for two jobs — where a
handle is *resolved* and where handles are *minted*. While the frame built its
own entries the submitter was the frame, so one field was right by accident. A
component submitting makes them different tables, and resolving a caller's index
in the server's own table is the confused deputy. Split into `submitter` and
`supervisor`, with the supply staying the frame's until RFC 0029's cross-table
link lands — because `tear_down` walks `Instance::supplied` in that table.

*`AUTHORITY/REVOKED`.* A core clears an occupant's table when the occupant ends,
so by the time the frame drains a ring the occupant left behind, every handle in
it is gone. The submitter's table is now a copy taken at the moment the
supervisor was started, which is the state the question *may the submitter spend
this?* is actually about.

*The powerbox grant was missing.* A supervisor must *hold* the account it names.
`write_board` grants the held-open place's `Untyped` into the supervisor's own
table with `GRANT` beside the four it already carries, and writes that handle —
the supervisor's own name for it, not the frame's — onto the board.

*`cargo xtask trace` went red.* `scheduled_line` printed a tick count, which is
time-derived, into the log `trace` hashes — breaking the rule `state.rs` wrote
down at E0-B14. It got away with it for exactly one increment: while the
occupant only announced and ended, two boots agreed by luck. Removed; `announced`
is what the line was for.

**Where the board came from.** A supervisor is *told* what it may spend and fill,
for `user/virtio-blk/src/routing.rs`'s reason and a sharper one — the frame
chooses out of the modules the loader placed, and one recorded hardware boot
placed them in a different order. It arrives as a declared `board` need, so the
account pays for it and `lint-manifests` checks it; `BLK_BOARD` is `BOARD` now,
because it is four shapes' address rather than the block driver's.

**Gate:** met. `cargo xtask verify` — `verify: all green`, exit 0.

### 8 — the policy moves *(done)*

**Files:** `docs/rfc/0076`, `kernel/src/component.rs`, `user/supervisor/src/`,
`xtask/src/main.rs`, `docs/rfc/0008`, `claims/0006`.

**RFC 0076 first, and it found that the plan's assumption was wrong.** The
blocker was never a place to put the policy — it was the *input*. A component is
told things by the frame writing pending state into its capability table, and
while a component runs, that table is its core's. The RFC's decision: **a
supervisor is told when it is started, not while it runs.** The frame posts
`PEER_GONE` into the endpoint slot, pumps it onto the ring, *then* hands over a
core. It refuses two alternatives on record — moving where a place-death pends,
which trades RFC 0008's structurally-guaranteed *granted then peer gone* for a
documented ordering; and a fifth shared word, which is affordable only when
something cannot be done without it.

`policy::decide` is `f_supervisor::policy::decide` now. The boot:

```
fault       place store epoch 0 stopped speaking: its control ring header no longer validates
supervised  told of 1 death(s); decided restart; submitted 1 spawn(s) from ring 3 …
restart     place store under on_fault — the supervisor said restart; restart 1 of 3
spawn       place store epoch 1 — nothing carried over: …
```

**Four things this increment found.**

*The supervisor was never told, twice.* First because the board minted a *second*
endpoint handle at consultation time, so the notice's handle matched no row —
the grant has to happen before the death, which is now `watch` and is a
parameter rather than a local. Then because `Table::clear_all` turns notice-owing
off on the way out — right for a teardown, wrong for an occupant that will be
given another core. Both presented identically: `told of 0 death(s)` for a place
that had demonstrably died.

*`publish` stole the supervisor's answers.* The frame drains a component's
completion ring on its behalf, and refuses anything that is not a notice. A
component that reads its own ring needs `publish_only`, and the invariant
`collected == notices` becomes `collected + handed == notices` — a second
destination, not a weaker check.

*The supervisor's place was never torn down*, because it is held out of `extras`
for the scripted lifecycle and nothing put it back. One leak, one sentence:
*a component's frames did not all come back*.

*A single-core boot has no supervisor to ask.* `cargo xtask cores` refills the
place from the frame with no policy consulted, and says so on the line. Keeping
a copy of the decision for that path would have been the reversal coming back.

**Gate:** `cargo xtask lint-owed` drops from four rows to three. RFC 0008's row
is deleted, and `docs/rfc/0008` gains a dated *what landed* section rather than
being left describing a tree that no longer exists.

**What did not move, stated because a paid row is as misread as a stale one:**
the frame still *stores* the restart tally (RFC 0076 names that seam), and a
**retirement** is still the frame's scripted act — it is the one fate with no
opcode behind it. Driving it through the supervisor needs four deaths in a row.

### 9 — the retirement, and two owed reversals are paid

**Files:** `kernel/src/component.rs`, `user/supervisor/src/`, `abi/src/door.rs`,
`xtask/src/main.rs` (`OWED_REVERSALS`).

Two things are left, and they are separable.

**The retirement.** A place's third fate is the one with no opcode behind it:
the control ring says *end this occupant*, not *end this place*. Today the
supervisor's `Retire` travels on its board and the frame performs it, and the
boot's retirement is scripted rather than reached. Reaching it means driving the
budget one death at a time — `max_restarts` restarts and the one that finds the
budget spent — each with its own teardown, notice, core schedule and refill.
That is a bigger boot, not a harder one.

**`ANNOUNCE` and `PROGRESS` (RFC 0014), whose condition is now met.** A
component is started with a channel *and told on it*, which is exactly what that
reversal waited for. The row in `OWED_REVERSALS` says `MET` in capitals because
a reader skimming for blockers would otherwise count it as one. The work is to
make an announcement a ring entry and delete two door calls.

The four capability calls (RFC 0015) retire onto
`INSPECT`/`DERIVE`/`REVOKE`/`MAP`, which are still named and unimplemented, and
that is genuinely blocked rather than owed.

**Gate:** `cargo xtask lint-owed` drops from three rows to one — RFC 0051's
`Reported` merge, which is unrelated and stays.

## What this plan does not do

- **It does not touch `TODO.md`.** This tree's agents may not, and E1-B05's line
  needs correcting on two counts — the dead `adopt` wall, and an exit written as
  another task's observation (`E1-P06 passes`), which is the third such exit on
  that list. Both are in the corrections document for the originator.
- **It does not close `E1-B05`.** The line's exit is `E1-P06 passes`, and
  `E1-P06` is a separate task that kills drivers under sustained load. Increment
  6 makes that possible; it does not perform it. Increment 7 moves one step
  further in the same direction without reaching it either: `CHAOS_GAP` says the
  driver is *scheduled outside the place its manifest is spawned into*, and the
  place a supervisor now fills is a driver's — so the occupant a boot can kill
  and the occupant that serves the datapath are one spawn closer to being the
  same component, and still are not.
- **It registers no number.** Nothing here is a measurement, and the one figure
  this makes true — a non-zero heap peak — is a count read out of a prologue,
  not a time. `claims/0006-driver-restart-latency.toml` is where a restart
  latency would go, and it is `pending` on a machine nobody has.
