# RFC 0116: A bound that loses an entry in silence is a defect in a subsystem that did not move

- Filed under a shortened slug on 2026-09-24. The file was `0116-a-bound-that-loses-an-entry-in-silence-is-a-defect-in-a-subsystem-that-did-not-move.md`, which is over ustar's 100-byte name field, so `cargo xtask release` could not pack the tree and two CI jobs refused it. The title is unchanged; the slug is not a summary and the number is what anything cites
- Status: accepted
- Date: 2026-09-24
- Affects: `kernel/src/main.rs` (`MAX_RESERVED`, `FIXED_RESERVED`,
  `reserved_ranges`), `xtask/src/main.rs` (`lint-bounds`),
  `.github/workflows/ci.yml`, `docs/booting-on-hardware.md`,
  `claims/0030-rollback-comparisons.toml`,
  `docs/postmortem/0002-the-only-runner-was-a-schedule.md`

## Decision

Two things, and they answer different halves of one incident.

**`reserved_ranges` no longer drops a module reservation in silence.** It counts
what did not fit and returns the count, and the boot refuses on it by name.
RFC 0101 recorded the behaviour as *`reserved_ranges` stops at the bound and says
nothing*; that sentence is now false on purpose. A reservation that did not fit
is memory corruption with a delay fuse — the frame allocator will offer a live
module's frames to whoever asks next — so there is no degraded mode to continue
into, and the operator who sees the consequence is three subsystems away from the
cause.

**`cargo xtask lint-bounds` evaluates the arithmetic that four boot-path bounds
were resting on.** Each of the four is larger than a count kept in another file,
every relation was written in a comment, and nothing compared one side with the
other. This is the check RFC 0101 named in its *What would reverse this* and
declined to write, on the grounds that a rule deserves its own task. It is four
file reads and no build, so it runs in `cargo xtask lint` — which means in
`cargo xtask verify` and in the pull-request gate.

The four relations, written in `lint_bounds` and nowhere else:

| bound | at least | because |
|---|---|---|
| `MAX_RESERVED` | `FIXED_RESERVED + MAX_MODULES` | a module whose reservation did not fit is one the allocator hands out |
| `MAX_MODULES` | `1 + PLACES_MAX + 1 + 2` | init, a module per place, a successor for a swap, and two generations on a menu |
| `RESERVATIONS_MAX` | `2 * PLACES_MAX` | `component::fill` grants after the spawn, so a refilled place holds two entries |
| `PLACES_MAX` | `COMPONENTS.len()` | a component file with no place is one no downstream count looks for |

## Context

The nightly job *a generation broken, rolled back, and compared three ways* went
red on 2026-09-23 at `b74807ca`, the merge of pull request #76 — E3's first four
waves — and printed:

```
xtask: the boot selecting f.root=a7912d2f… f.frame=e824b3ac… exited Some(35)
  rather than 33.
FAIL: the generation: no module this machine was offered folds to the root it
  was asked for
    offered     1 boot module(s), 0 of them refused
```

Two generation modules were offered and the frame found one. In the same boot
log, eleven lines above the failure:

```
  address space 0x0000000000386000 root, direct map at 0xffff800000000000
  module        0x0000000000386000..0x00000000003b45e0  185 KiB
```

The address space's own root is the first page of the eleventh module. At
`b74807ca` `MAX_RESERVED` was the literal `13` — three fixed ranges, one
exclusion, one per module — chosen against the modules a boot *had*.
`reserved_ranges` filled three and then ten, stopped at the bound, and returned a
list that was indistinguishable from a list nothing had been dropped from. The
allocator was handed a region with a live module inside it and gave the module's
first frame to the first caller, which was the page-table builder.

**So the defect was in what the claim checks and not in the check.** The claim's
route is right, its thresholds are right, and `f_generation::select` did exactly
what it is written to do: it looked for `MODULE_MAGIC`, found it in one of the two
blobs it was offered, and reported one. Reading it the other way round — treating
`offered 1` as a selection bug — would have put a repair into the one subsystem
that was behaving.

**The repair for the cause was already written, and that is the second half of
the incident.** Commit `84b10b7` on 2026-09-22 derived the bound as
`5 + multiboot::MAX_MODULES` and RFC 0101 recorded the diagnosis in the same
words as above. It landed on a branch; `b74807ca` was the merge of an earlier
state of that branch; the nightly then reported, on a schedule, a day later, a
defect whose fix was already in a peer's working tree. Nobody read the cross for
a day. `docs/postmortem/0002` is that half.

What was live and is not taken: raising `MAX_RESERVED` again and leaving the
silence, which is what buys the next day; and a `const` assertion instead of the
lint, which is tautological now that the bound is derived from the count and does
nothing about the other three relations.

## Consequences

**What this makes easy.** Being one short. Every relation above is now evaluated
on a laptop in the same second as `lint-percpu`, and the failure text names the
bound, the value it holds, the value it needs and the sentence that says what one
short costs. A boot that drops a reservation says so on the line where it
happened, and `cargo xtask run` — which `verify` runs — is now a boot that would
catch today's module count against yesterday's bound.

**What this makes hard, and it is the residue.** `FIXED_RESERVED` is a count of
assignments in a function body, so the lint cannot check it: it reads the
constant, not the three unconditional ranges plus the exclusion plus the
framebuffer that the constant is supposed to be counting. A sixth fixed
reservation added and not declared is the literal thirteen again, one slot
further along — and what stands where a check would is the boot's refusal, which
is a diagnosis and not a guard. The honest statement is that one of the two
sides of that relation is now checked and the other is reported.

The lint's resolver is also deliberately small: a sum of decimal integers and
constants it has already read, taken by the last path segment. A bound written as
anything else is a finding rather than a pass, which is the right way round and
will be irritating exactly once.

## What would reverse this

**A fifth relation of the same shape appearing somewhere `lint-bounds` does not
look.** The check holds four pairs by name; it is not a rule about bounds, it is
four facts about four of them. The shape that would reverse the *approach* is a
tree where this list is long enough that maintaining it is the work — at which
point the answer is a declaration beside each constant rather than a table in
`xtask`, and this file is what that change argues with.

**A boot that legitimately offers more modules than it reserves.** Nothing does
today, and if something ever must — a loader placing blobs the frame is meant to
ignore entirely — then the refusal above is wrong and the right repair is a
reservation list that grows, not a fatal line. The reversal is visible: it
arrives as somebody wanting `MAX_RESERVED` raised without a module to point at.

**`FIXED_RESERVED` going wrong in the direction the lint cannot see.** That is
the residue named above and it is the one to watch first, because it will not
look like a reversal. It will look like a sixth range being reserved, correctly,
in a diff about something else.
