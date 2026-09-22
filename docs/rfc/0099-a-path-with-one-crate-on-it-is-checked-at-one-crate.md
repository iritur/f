# RFC 0099: A path with one crate on it is checked at one crate, and `E3-B04a` says so

- Status: accepted
- Date: 2026-09-21
- Affects: `TODO.md` and `intent/0012-the-interface/spec.md` (`E3-B04a`'s exit),
  `xtask/src/main.rs` (`lint_stamp`, `INPUT_PATH`), `input/src/stamp.rs`

## Decision

`E3-B04a`'s exit says *one time source in the whole path; a lint finds any
second reading of a clock in it*. That reads as a statement about four stages
and is today a statement about one. **It is narrowed to the crates that can hold
a clock reading at all**, and the lint computes which those are and prints the
answer on every green run rather than implying there are four.

What is no longer claimed, both halves stated so a reader can check them:

- **That `abi/`, `interface/` and `scene/` are stages being checked.** None of
  the three depends on `f-env` or on `f-input`, so in those crates there is no
  expression that both reads a clock and compiles — a second time source cannot
  be written there, and no single-file diff can make `lint-stamp` fire. Their
  rows are places held open in advance. `stage_reach` is the function that says
  so and `reach_findings` is what refuses a path *no* stage of which can reach a
  clock, because a rule with no subject is the failure mode this narrowing
  exists to make visible rather than to hide.
- **That a stamp travels between stages at all.** `f_input::stamp::at_interrupt`
  has no caller outside its own `#[cfg(test)]` module, no crate in this
  workspace depends on `f-input`, and `sim/Cargo.toml` does not list it either.
  So the exit's *under the simulator the stamp is `Env`'s virtual time* clause is
  carried by unit tests over `f_env::SeededEnv` in `input/src/stamp.rs`, and not
  by `cargo xtask sim`.

What is still claimed, and each of these now has a fixture that has been watched
go red: exactly one clock reading in the one crate that can hold one; no crate
that names the stamp and carries no `INPUT_PATH` row; no file compiled into an
on-path crate from off the path; no `StampNanos` minted from a number the stage
computed rather than decoded; and a refusal if the whole path ever goes
clock-less.

## Context

`E3-B04a` was built in `0fda44f` and refused by three successive adversarial
rounds. Round four found four defects. Three were repairable and are repaired:
the lint's own *What it cannot see* section closed the mint route with a
sentence that was false — it said `StampNanos` has no constructor taking a
`u64`, and `from_wire_nanos` is exactly one — `INPUT_PATH` was checked only for
rows matching no source and never for sources having no row, so `abi/` was
silently off a path whose wire form it declares, and membership was decided by
where a file sits on disk, so `#[path]` and `include!` compiled code in from
outside every needle's reach. That last one is RFC 0092's route, met a second
time in a different lint, one epoch away from where `E3-B03a` meets it.

The fourth is not a defect and is not repairable by a lint. **The lint guards a
route before there is traffic on it.** That is the right order to build in —
`E3-B04d` lands the driver and `E3-B01i` the first consumer — and it is the
wrong thing to call met, because *one time source in the whole path* is
trivially true of a path that carries nothing.

Two alternatives were live and both were worse. **Leave the exit and hold the
line `[>]` until `E3-B04d` lands** keeps a true sentence at the cost of a task
that is finished waiting on a task it does not block; `E3-B04d`'s own `needs:`
does not name this line, so the wait would have been for company. **Widen the
lint until the three empty rows are checked** is the one that had to be refused
explicitly: there is nothing there to check, and a check over an empty set is
green for the same reason it is meaningless. Making it *look* checked is the
decoration four rounds of this epoch have been spent removing.

## Consequences

**What this makes easy.** The lint now says what it knows. A reader of a green
`lint-stamp` run gets one line per stage — `input/ reaches a clock, so a second
reading here compiles`, and for the other three `reaches no clock: nothing under
it depends on f-env or f-input, so this row is held open rather than checked` —
which is a stronger artefact than the unqualified sentence was, because it names
its own coverage instead of asserting it.

**What this makes hard, and it is deliberate.** The row for a stage cannot be
quietly deleted to make the lint pass, and a path that stops reaching a clock
anywhere is refused rather than reported green. Those are the two ways a
narrowed rule usually rots.

**What it forecloses.** Nothing. Each of the three held-open rows closes by
itself, without an edit here, the day something under it depends on `f-env` or
`f-input`: `stage_reach` recomputes and the row starts being checked.

## What would reverse this

`E3-B04d` — the virtio-input driver — giving `at_interrupt` a caller that is not
a test, and `E3-B01i`'s late-latch giving the stamp a consumer. At that point a
stamp really does travel between crates, at least one more stage reaches a
clock, and the unqualified sentence becomes a statement about a path rather than
about a crate. The reversal is mechanical and needs no edit to the lint: it is
`stage_reach` returning `true` for a second row.

The narrower reversal, which would say this RFC was wrong rather than early: a
demonstration that a second clock reading **can** be written into `abi/`,
`interface/` or `scene/` and compile. That would mean `A_CLOCK_IN_REACH` is the
wrong vocabulary — that a crate can read a clock without depending on `f-env` or
`f-input` — and the narrowing rests on it.
