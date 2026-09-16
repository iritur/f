# RFC 0084: An exit narrowed by measurement is a reversal, and two of E3's are narrowed

- Status: accepted
- Date: 2026-09-16
- Affects: `intent/0012-the-interface/spec.md`, whose `E3-B01b` and `E3-B01d`
  exit lines are replaced below and which is a paste-ready handoff, so a false
  line there becomes a false line in `TODO.md`; `scene/src/kind.rs` and
  `scene/src/commit.rs`, which already state the narrower sentences and now have
  somewhere to point; `CLAUDE.md`'s *Reversals need RFCs*, applied to a kind of
  sentence nobody had asked whether it covered; `abi/src/scene.rs` and
  `f_abi::Sqe`, named below as the owners of the one change that would restore
  `E3-B01d`'s wider claim

## Decision

Two parts, and the second is the one that outlives this epoch.

**One. The two exits are narrowed to what measurement shows**, in the words
their own modules already use.

`E3-B01b` said *a seventh kind is a compile error in every consumer, the way
`Role` already is*. It now says:

> a seventh kind is a compile error in every consumer that decides per kind
> without a wildcard, and the one consumer in this workspace that decides per
> kind is such a consumer.

`E3-B01d` said *the graph read back is the old scene or the new one, never a
third*. It now says:

> over a ring whose `Release`/`Acquire` pair holds, the graph read back after a
> cut is the old scene or the new one, never a third; over a ring whose pair
> does not hold, third scenes are produced, and the sweep counts them and
> requires them to be there.

**Two. An exit sentence is something already written down**, so narrowing one is
a reversal in `CLAUDE.md`'s sense and needs an entry here. This was not obviously
true before it was asked. `docs/sdlc.md` routes work from an intent through a
spec to a diff, and the exits live in the spec, which is a working document an
epoch edits as it learns — so a reader could reasonably have concluded that
editing one is ordinary spec maintenance.

The argument for treating them as written-down anyway is that **an exit is the
sentence a task is accepted on**. It is the one line a later reader uses to
decide whether a task is done, and a task marked done against a sentence quietly
rewritten to fit what was built is precisely the failure this project has
recorded four times in two weeks. So: a spec's *description* may be edited
freely; its `exit:` may not.

The mechanical consequence, stated so it cannot be read as advice: **a narrowed
exit is edited in the spec, with the RFC number beside it.** A narrowing that
lives only in a module comment leaves the spec asserting something the tree does
not do, and `intent/0012-the-interface/plan.md` is a handoff whose whole shape is
*paste these lines into `TODO.md`*. A module comment and a spec that disagree is
not a documentation problem; it is a false line waiting for a paste.

## Context

Nineteen E3 subtasks were built against their exits, then read by adversarial
reviewers whose instruction was to defeat the exit sentence. Eleven were refused
on twenty-eight findings. The repair round allowed three outcomes per finding —
make the sentence true, delete a guard that cannot guard, or narrow a false claim
— and these two are the narrowings.

**`E3-B01b`, measured.** The exit calibrates itself against `Role`: *the way
`Role` already is*. A reviewer took the calibration literally. Adding a seventh
`Kind` end to end — the `kinds!` list, `abi::scene::kind`, its `known` and
`label` arms — produced **zero errors** across `cargo check --workspace
--all-targets`. Adding a twenty-third `Role` to `interface/src/node.rs` produced
`error[E0004]`. The yardstick refuted the file that had invoked it.

The repair made the parity real where it could. `scene/src/effect.rs`'s
`matches!(created.kind(), Kind::Effect)` — the only per-kind decision in the
workspace, and a wildcard — became a written-out six-row `ByKind<bool>` literal,
after which the same experiment yields `error[E0308]: expected an array with a
size of 7, found one with a size of 6`, in a module other than the one declaring
the kinds. That is the guard the exit wanted, and it exists.

What could not be made true is the quantifier. Three things still take a seventh
kind silently, and the module names all three rather than counting them:

- a consumer that writes `_ =>`, which Rust gives no way to refuse over somebody
  else's enum, here or in `interface/`;
- a `ByKind` built as `[x; Kind::COUNT]` or as `Kind::ALL.map(..)`, both of which
  grow to seven on their own — `crate::arena` holds one of the first and two of
  the second;
- a consumer carrying a kind as a `u16` validated by `abi::scene::kind::known`,
  which `crate::reconcile` and `crate::commit` both do.

**`Role` does not achieve *every consumer* either.** It has consumers that happen
to match exhaustively, which is a fact about who has written code so far and not
a property of the type.

**`E3-B01d`, measured.** The first implementation modelled a ring slot whose
write did not land as zeros. `E2-P01` does not: a medium that did not take a
write holds its **prior** content, and `zone/tests/cut.rs` is built on that.
Ported faithfully — `Recorded` carrying the previous commit's slots, index for
index — the same sweep produced third scenes.

The reason is worth stating, because it is not a defect in the commit path. A
ring slot whose write is not visible reads as whatever occupied it last:
`ring/src/lib.rs` initialises its slots once and nothing zeroes one on
completion. A previous frame's entry **decodes**. The batch takes it for one of
this frame's, and the commit that follows either seals over a frame that is part
this one and part the last, or is refused whole by `admit`. The first is a third
scene.

Nothing in the delta format distinguishes *this slot's bytes belong to this
frame* from *they belong to the last one*. There is no sequence number on an
entry, and `f_abi::Sqe` has no field left that a scene delta does not already
read or require to be zero.

## Consequences

**What this makes easy.** `E3-B01d`'s narrowed sentence turns a hidden dependency
into a stated one: a commit's atomicity rests on the ring's `Release`/`Acquire`
pair, which is what this repository already says that pair is for, and the sweep
is now the measurement of it rather than an argument against it. `Mode::Lying`'s
third scenes are *required to be there*, on the same logic as `zone/tests/cut.rs`'s
controls and the eager applier beside them: a run in which they were zero would
be a run whose model had stopped being faithful. That is a stronger test than the
one that was refused, not a weaker one.

**What it makes hard, deliberately.** Nobody can now write *scene commits are
atomic* without the clause. That sentence will look like an omission to a reader
who wants a headline, and it is meant to.

**What it forecloses.** Closing `E3-B01d` by improving `scene/src/commit.rs`. The
stale-slot case is an ABI change and is named as one: a per-entry submission
sequence the consumer checks against the slot's own index, which makes a stale
slot **detectable** rather than merely unlikely. That belongs to
`abi/src/scene.rs` and to whoever next opens `f_abi::Sqe`, and until it lands the
clause stays.

**What it costs `E3-B01b`.** The claim that `Kind` is closed the way `Role` is may
not be made — and, the half worth saying out loud, **neither may the claim about
`Role`**. `interface/src/node.rs`'s own wording is the origin of the overclaim,
and a reader who takes `E3-D01` at face value will believe something about
`Role`'s consumers that no mechanism provides.

**A shape, not yet a rule.** Two of eleven repairs were narrowings. That is a
rate worth watching rather than a finding. If a third of an epoch's exits end up
narrowed, the thing to examine is not the repairs but how exits are written: an
exit is meant to be a sentence somebody other than its author could observe, and
a sentence that cannot survive its own first measurement was never that.

## What would reverse this

**For `E3-B01d`, precisely one thing, and it is buildable.** A per-entry
submission sequence in the delta format, checked by the consumer against the
slot's own index. With it a stale slot is refused rather than decoded, the lying
ring stops producing third scenes, and the original unqualified exit becomes
true — at which point this RFC is superseded and `Mode::Lying`'s
required-nonzero assertion inverts to a required zero. Anything less does not do
it: a per-*frame* sequence still leaves the first slot of a torn frame
indistinguishable from the last slot of the frame before.

**For `E3-B01b`, two things, either of which would do.** A way to refuse a
wildcard over a foreign enum — which Rust does not have, and which would be a
language change rather than a tree change. Or the weaker and reachable version:
every per-kind table in the workspace written out as a literal, a lint that
refuses `[x; Kind::COUNT]` and `Kind::ALL.map(..)` in a table that must fire, and
no consumer carrying a kind as a bare `u16`. That is three edits in
`crate::arena` and one decision in `crate::reconcile` and `crate::commit` about
where the decoder belongs. If somebody does that work, the quantifier comes back
and this half is superseded.

**For the general half, the reversal is a measurement rather than a mechanism.**
If exits stop being narrowed — if a run of epochs closes its tasks against the
sentences they were written with — then routing a narrowing through here is
buying nothing, and the honest response is to say so rather than keep the
ceremony. The opposite observation reverses it the other way: if a narrowing is
ever found to have been made **silently**, in a module comment with no spec edit,
then this RFC failed at the only thing it was for, and the answer is a lint over
`exit:` lines rather than a stronger sentence here.
