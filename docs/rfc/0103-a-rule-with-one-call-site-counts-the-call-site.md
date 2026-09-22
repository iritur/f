# RFC 0103: A rule with one call site counts the call site

- Status: accepted
- Date: 2026-09-23
- Affects: `xtask/src/main.rs` (`lint_stamp`, `INPUT_PATH`, `THE_ONE_CALLER`),
  `input/src/stamp.rs`, and RFC 0099's narrowing of `E3-B04a`

## Decision

`lint-stamp` checks **two counts rather than one**: exactly one clock *reading*,
in `input/src/stamp.rs`, and exactly one *call* to it, in a named file —
`THE_ONE_CALLER`, today `user/virtio-input/src/clock.rs`.

RFC 0099 narrowed `E3-B04a`'s exit from *one time source in the whole path* to
*one time source in the crates that can hold one*, on a measurement: three of
the four stages could not reach a clock, and `f_input::stamp::at_interrupt` had
no caller outside its own `#[cfg(test)]` module. It named `E3-B04d` as what
would reverse it. `E3-B04d` has landed the driver, so this is that reversal
arriving — the narrowing widening again, on a second measurement rather than on
the same argument run backwards.

**The zero case is what earns the entry.** Before this, a path with no caller at
all and a path with exactly one were indistinguishable to the check: both
reported one reading and said nothing about whether anything used it. That is
the vacuity RFC 0099 described, and the second count is what makes it visible.

## Context

**Why a named file and not *anywhere on the path*.** *Anywhere* cannot express
*exactly one*. A whole-path count stays satisfied when a second crate takes a
stamp and the driver's call is deleted — two changes that cancel in the total
and mean the opposite of each other. Naming the file makes the substitution a
red build and the reader of the failure sees which file moved.

The cost is stated rather than discovered: the constant is a path, so moving the
driver's clock module is an edit here. That is the same cost `INPUT_PATH` already
carries for the stages, and the same answer — a rule about which file holds a
thing is a rule that names the file.

## Consequences

**What this makes easy.** `stage_reach` already prints, per stage, whether
anything under it can reach a clock at all. With the caller counted beside it,
a green `lint-stamp` now says three things a reader can check independently: one
reading, one call, and which rows are held open because nothing under them could
hold a second reading if it tried.

**What it does not claim, and `E3-B04d` is `[>]` for exactly this.** A call site
in the source is not a call at a boot. There is no `kernel/src/input.rs`, so
nothing stands the driver up and nothing drains its ring; the compositor
consumes no events from it. The stamp travels between two crates in the
*workspace* and in no *run*. `E3-B04a`'s closing condition — *`E3-B04d` gives
`at_interrupt` a caller that is not a test* — is true in the source and not yet
true in a boot, and both lines stay `[>]` on that.

## What would reverse this

**A second legitimate caller.** A second driver that timestamps input — a
serial-line keyboard, a second virtio device — makes *exactly one call* false of
a correct tree. The repair then is not to widen the count but to ask what the
rule is actually about: one reading per *event*, which is a property of a path
and not of a file, and which needs the stamp to be carried rather than counted.
`E5-B06`'s hardware-timestamped input is where that question arrives, and it
arrives with a device whose own clock makes the reading somebody else's.

**The narrowing this un-narrows returning.** If `user/virtio-input` is deleted
before `kernel/src/input.rs` exists, the caller count goes to zero and the rule
is a rule with no subject again. `reach_findings` already refuses a path no
stage of which can reach a clock; the caller count should be refused the same
way rather than reported as a green zero, and it is.
