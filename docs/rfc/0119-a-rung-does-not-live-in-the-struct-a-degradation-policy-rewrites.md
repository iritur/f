# RFC 0119: A rung does not live in the struct a degradation policy rewrites

- Status: accepted
- Date: 2026-09-24
- Affects: `user/compositor/src/tree.rs`, `user/compositor/src/component.rs`;
  RFC 0080's *never promoted* clause; `E3-B07g` and `E3-B02b` in `TODO.md`

## Decision

The rung the compositor started on leaves `f_compositor::tree::Story` and becomes
a field of `Held` with a type of its own, `StartingRung`, whose constructor is
private to the module and whose only method answers a word. `Story` — the struct
every closing frame rewrites — no longer has a member a demotion could be written
into, and the function that chooses a reduction, `pacing::chose`, takes a budget,
a frame ordinal and a duration and answers an ordinal, so not one of its types
can name a rung. The two mechanisms this system spells *fallback* are kept apart
by what can be typed rather than by a paragraph saying they are different.

## Context

`tree.rs` carried this paragraph, and it is quoted rather than summarised because
it was right about everything except what to do:

> **Nothing in this crate enforces that, and a reader should not believe it
> does.** The type permits a second assignment — `Story` is a plain struct whose
> other fields are written every frame — and what catches one is the boot [...]
> A type that made the second assignment impossible would be better and is not
> written here, because the rung is published in the same struct as four numbers
> that must move and splitting them would cost a reader the one place the frame's
> story is.

`E3-B02b` had established the first direction — chosen once at start, never
promoted — and the guard it left was a boot one privilege boundary away:
`kernel/src/compositor.rs` writes a better backend report onto the routing page
after the first frame closes, and a component that recomputed its rung publishes
a different word and goes red. That is a real guard and it stays.

`E3-B07g` is the other direction: an overloaded compositor must not *demote*
either. Its exit names the failure shape in its own words — *the two mechanisms
that share the word fallback, kept apart by something other than a paragraph* —
and the paragraph above was the thing that was there. Falling back a rung and
falling back an effect are both *fallback* in this tree's prose; they were
written into one struct; and one of the two is rewritten every frame by a policy
whose entire job is to give something up. The trade the paragraph defended — a
reader gets the frame's story in one place — is real and is smaller than it
sounds: the story is assembled at the publish anyway, in
`component::report` and `component::publish`, where the rung arrives from `Held`
and the four per-frame numbers from `Story`, one line apart.

An alternative was live and is recorded: leave the field where it is and add a
test. A test over one or two degradation answers is a sample, and a sample is
what a paragraph would be — a build that demoted on one particular ordinal
survives any single case anybody thought to write. What is written instead is
both: the type, and a cross product over every rung of the ladder against every
ordinal the policy can produce, including the criteria words that no wire in this
build can yet reach.

## Consequences

What it makes easy: a reviewer looking for a demotion has one place to look and
finds the absence structural. `Story` has no rung; `StartingRung` has no
arithmetic, no ordering and no `index`, so *fall back one rung* has no spelling;
and `Held::rung` hands out a shared reference, never an exclusive one.

What it does not claim, because the claim would be false: Rust privacy is
module-wide, so code inside `tree.rs` can still assign the field, and the test
module does exactly that to drive the cross product. What the type buys is that
the assignment cannot be written *by the degradation path*, which is where it
would have been written, and cannot be written at all from another crate.

What it costs: two accessors where there was one, and a reader of the published
tree now finds the rung and the frame's four numbers coming from two places in
`component::publish` rather than one. The boot's clause is unchanged, which is
the point — three checks now hold the sentence and none of them is it alone.

## What would reverse this

A rung that legitimately moves while a component runs. RFC 0080 forecloses
promotion on the grounds that a rung change moves the cost distribution
`pacing` estimates from underneath the estimator; if that argument is ever
answered — a compositor that re-primes its window across a rung change, say, with
a claim behind it — then the rung is no longer a value fixed at start and this
type is in the way. The honest repair then is not to delete the type but to give
it the one transition the new argument permits, named, so that the transitions it
does *not* permit stay unspellable.

The weaker reversal to watch is the accessor count becoming a pattern: if a
second and a third value fixed at start each grow a type of their own, what is
wanted is one `Started` record holding all of them, and this becomes a field of
that rather than a shape repeated three times.
