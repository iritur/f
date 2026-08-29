---
id: 0001
status: shipped
originator: Dmitri Chudinov
todo: E0-B07
---

# The kernel needs a clock of its own

## Problem

The kernel can be interrupted and cannot yet be interrupted *on purpose*. It
reports a fault, which is the exceptional case, and has no way to make anything
happen at a moment of its own choosing. Everything the project claims about
latency is therefore unmeasured: there is no periodic event to be late for, so
there is no lateness to record, and the number the whole architecture is
supposed to be organised around does not exist.

It is also the last thing standing between the frame and every other piece of
work in the epoch. Twenty-two tasks wait on it.

## Proposed outcome

The kernel drives a periodic interrupt at 1 kHz on the core it is running on,
and can say how late each one was. Running it for a minute produces a histogram
of that lateness — not a mean, a distribution — so that the tail is visible
rather than averaged away.

Two things have to be true of the number for it to be worth anything. It has to
come from a clock that was measured against something independent rather than
assumed, and it has to be a distribution.

## Affected users and systems

`kernel/` throughout: the interrupt table gains its first non-exception vector,
the address space gains its first device mapping, and there is a new device.
`docs/design/ring-scene-boot.html` describes M2 and does not change. The claims
registry gains its second entry. `docs/design/proving-ground.html` calls timer
jitter the first real measurement the project produces, so this is the change
that makes layer 7 mean something on a number rather than on an example.

## Constraints

The boot log is byte-identical for a given seed and commit, and that must stay
true. A measurement is not byte-identical, so a measurement cannot be part of an
ordinary boot.

The 5 µs p99 bound is the milestone's, not this change's. This change has to be
able to *say* what the number is; whether it is good enough is a separate
argument that needs a reservation policy nobody has written yet.

## Open questions

- Whether the environment the project can actually execute in is capable of
  producing a number worth recording at all. If it is not, that should be said
  out loud rather than discovered by somebody quoting the number later.
- Whether the timer wants its own abstraction now or after there is a second
  consumer of it. There is exactly one today.
