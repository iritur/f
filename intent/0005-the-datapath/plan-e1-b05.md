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

**Gate, and it is three declared quantities going red at once — all on purpose:**

- `HEAP_GAP` (`xtask/src/main.rs`). Its needle is the sentence at
  `kernel/src/runtime.rs:63`. When an occupant is scheduled, `user/store`'s
  64-byte box executes for the first time and the boot's `peak 0 byte(s)`
  becomes non-zero. `cargo xtask run` is written to refuse exactly that and to
  say it is the good ending. **This is the acceptance test for increment 6**, and
  it is why this branch is based on the one that declared it.
- `CHAOS_GAP`'s needle `prepare_driver(` in `kernel/src/blk.rs`, once a driver
  is scheduled inside the place its manifest is spawned into rather than beside
  it. That closes `E1-P06`'s remaining half.
- `E2-B10`'s exit, which asks for a boot in which a component allocates.

Each of those names documents in its fourth field, and they are updated in this
increment's diff rather than the one after.

### 7 — the policy moves, and three owed reversals are paid

**Files:** `kernel/src/component.rs`, `user/supervisor/src/component.rs`,
`abi/src/door.rs`, `xtask/src/main.rs` (`OWED_REVERSALS`).

`policy::decide` (`kernel/src/component.rs:369`) was written over a `&Record`, a
`&mut Budget` and a tick with no kernel state at all, precisely so this is a
move. The work is in the callers — `:1212`, `:1331`, `:1378-1379` — each of
which decides inline inside teardown today and must become *post the death
outward, wait for a `SPAWN`*. The supervisor gains the `notice::PEER_GONE` arm
that reads the cause `abi/src/control.rs` packs.

Then `ANNOUNCE` and `PROGRESS` retire off the door (RFC 0014) because a
component is now started by something other than the frame writing a job into a
per-core slot, and the four capability calls retire onto `INSPECT`/`DERIVE`/
`REVOKE`/`MAP` (RFC 0015).

**Gate:** `cargo xtask lint-owed` drops from four rows to one — RFC 0051's
`Reported` merge, which is unrelated and stays.

## What this plan does not do

- **It does not touch `TODO.md`.** This tree's agents may not, and E1-B05's line
  needs correcting on two counts — the dead `adopt` wall, and an exit written as
  another task's observation (`E1-P06 passes`), which is the third such exit on
  that list. Both are in the corrections document for the originator.
- **It does not close `E1-B05`.** The line's exit is `E1-P06 passes`, and
  `E1-P06` is a separate task that kills drivers under sustained load. Increment
  6 makes that possible; it does not perform it.
- **It registers no number.** Nothing here is a measurement, and the one figure
  this makes true — a non-zero heap peak — is a count read out of a prologue,
  not a time. `claims/0006-driver-restart-latency.toml` is where a restart
  latency would go, and it is `pending` on a machine nobody has.
