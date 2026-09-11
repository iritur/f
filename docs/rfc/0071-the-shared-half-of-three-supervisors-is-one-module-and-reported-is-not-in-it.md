# RFC 0071: The shared half of three supervisors is one module, and `Reported` is not in it

- Status: draft
- Date: 2026-09-11
- Affects: `kernel/src/blk.rs`, `kernel/src/net.rs`, `kernel/src/gpu.rs`, a new
  `kernel/src/supervisor.rs`, `xtask/src/main.rs` (`OWED_REVERSALS` and
  `CHAOS_GAP`), `docs/rfc/0051`, `docs/rfc/0054`, `TODO.md` (`E1-B16`)
- Implements nothing yet. This entry records the decision and the measurement
  behind it; the diff is `E1-B16` and this stays `draft` until that diff lands

## Decision

The part of the three driver supervisors that is the same code moves into one
`kernel/src/supervisor.rs`. The part that is not the same code stays where it
is, and this entry names which is which, measured rather than asserted.

**Measured, with comments stripped, on 2026-09-11:**

| item | `blk.rs` | `net.rs` | `gpu.rs` | verdict |
| --- | --- | --- | --- | --- |
| `Registers`, `Registers::of`, `Registers::physical` | 33 | 33 | 33 | byte-identical in all three |
| `struct Supervising` | 11 | 11 | 11 | byte-identical in all three |
| `declared` | 28 | 28 | 28 | byte-identical in all three |
| `order_for` | 11 | 11 | 11 | byte-identical in all three |
| `impl Supervising` | 80 | 74 | 74 | `net` and `gpu` byte-identical; `blk` differs in one method |
| `struct Reported` | 10 | 5 | 5 | **not the same type** |

The first four move verbatim. `impl Supervising` moves with one repair, below.
`Reported` does not move, and the rest of this entry is mostly about why.

## `Reported` is three types that share a name

`OWED_REVERSALS` says the three supervisors "hold one `Registers`,
`Supervising`, `Reported`, `declared` and `order_for` between them". Four of
those five are one thing. `Reported` is not: `blk`'s carries `capacity`,
`overtaken`, `queued_max` and `in_flight`, which the other two have no
counterpart for, because a block device answers a request that has a depth and
an order and a display command does not.

Its *mechanism* is shared — a magic word, an `of` that checks it, fifty lines
that differ by one — and that is exactly the trap. A merged `Reported` with a
shared prefix and per-driver payloads would move `blk`'s `DRAINED`, `OUTCOME`,
`SHORTFALL` and `UNADMITTED` by eight bytes. Every writer uses the symbolic
name, so a full rebuild is safe; a partial one is not, and the magic check would
still pass on a stale component image reading shifted fields. A silent
relayout that survives the one check written to catch it is not a refactor worth
having.

So the reversal is paid **in part and the part is stated**. Its
`OWED_REVERSALS` row is rewritten rather than deleted, and what it names after
this is `Reported` alone.

## `blk`'s `awaited` becomes `within`

The one difference inside `impl Supervising` is that `blk` has

```rust
fn awaited(&mut self, tsc_khz: u64) -> Result<f_abi::Cqe, Trouble>
```

where `net` and `gpu` have

```rust
fn within(&mut self, tsc_khz: u64, micros: u64) -> Result<Option<f_abi::Cqe>, Trouble>
```

`awaited` is `within` at a fixed `ANSWER_MICROS` with `None` turned into
`Trouble::NoAnswer`. The merged module carries `within`, which is the more
general of the two and the one two of the three drivers already use, and `blk`
keeps `awaited` as four lines over it. Nothing about the timeout changes.

## What this is not allowed to break, and the check that says so

**`cargo xtask verify` does not boot `blk`, `net` or `gpu` at all.** It calls
`deadline` and `churn` and never the three datapath boots. A green `verify` is
therefore not evidence for this change, and treating it as evidence is how this
merge would go wrong quietly.

The acceptance test is the twelve boots those three commands run — six halves
for `blk`, three for `net`, three for `gpu` — plus `cargo xtask deadline`, each
compared against a log recorded on the unmerged tree, with every
machine-independent line required to be **byte-identical**. Not "green": green
is what a merge that lost a control would also be. `docs/postmortem/0001` is
the entry that earned this rule — the merged tree is the first execution of a
program nobody has run.

## `CHAOS_GAP` keys on a needle this change moves

`xtask`'s `CHAOS_GAP` has one row, and its needle is `prepare_driver(` in
`kernel/src/blk.rs`. Moving the call into a shared module deletes that needle
and turns the check red **without closing the gap it declares**, which is a red
build for a false reason and worse than no check.

The call therefore stays in `kernel/src/blk.rs`. If a later change does move it,
that change re-points the row in the same diff and says so, because the gap is
about a driver being scheduled outside the place its manifest is spawned into
and has nothing to do with which file the call sits in.

## Why now, when RFC 0054 argued against it

RFC 0054 refused this merge, and its refusal is worth quoting rather than
overruled: the merge rewrites `kernel/src/blk.rs`, which is the evidence a
closed task's exit rests on, *inside a task whose own evidence is a picture on a
screen*, so a defect introduced by the merge would be found by the wrong check
or by none.

Every clause of that is about E1-B04. As a unit of its own the clause is false:
`blk`, `net` and `gpu` each have a control that must fail, all three are CI
jobs, and `deadline` carries a gating claim. The merge's own evidence is exactly
the evidence it disturbs, which is the condition RFC 0054 was asking for.

What RFC 0054 does not license is doing it silently. It lists the supervisor
merge as its own reversal condition, so this entry and `E1-B16` exist.

## What would reverse this

- **A fourth driver that cannot use the shared `Supervising`.** The merged
  module is three drivers' agreement, and three is a small number to generalise
  from. A device that needs a different wait, a second live domain, or a
  register window that is not four pages turns one of the moved items back into
  a per-driver one — and the honest response is to move that item back out, not
  to add a parameter to it for one caller.
- **`Reported` becoming one type after all.** If a later driver's counters turn
  out to be `blk`'s counters, the argument above evaporates and the remaining
  `OWED_REVERSALS` row is paid by a merge with the offset assertion this entry
  refused to do without: a compile-time check, written before the offsets move,
  comparing each crate's field offsets against the `abi` constants.
- **`verify` learning to boot the three datapaths.** The acceptance test above
  exists because it does not. If it does, the baseline-log comparison becomes
  the cheap thing rather than the careful thing, and this section is where to
  record that the careful version was once necessary.
