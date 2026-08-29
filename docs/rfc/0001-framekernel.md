# RFC 0001: The kernel is a framekernel

- Status: accepted
- Date: 2026-08-27
- Affects: `kernel/`, architecture document bet 01

## Decision

Run everything in one address space for monolithic speed, and use the language
rather than the memory management unit to draw the safety boundary. A small
privileged *frame* is the only code permitted `unsafe`; services above it are
unprivileged by construction.

## Context

Three shapes were live: monolithic, microkernel, and framekernel. The
microkernel option was rejected not on isolation grounds — it is stronger there
— but because the hot path in this design is supposed to contain no boundary
crossing at all, which makes fast inter-process communication a solution to a
problem the architecture is trying to delete rather than optimise.

Asterinas is the existence proof: Linux-comparable performance with the unsafe
trusted base held near 14% of the codebase.

## Consequences

Isolation reasoning becomes a property of the type system, so it must be
enforced mechanically or it is not enforced at all. Hence
`unsafe_code = "forbid"` at the workspace root, three crates overriding it, and
`cargo xtask lint-unsafe` failing the build on any fourth.

The cost is honest and stated in the architecture document: frame soundness is
a proof obligation, not a compiler guarantee. Discharging it is the priority
target for verification work.

## What would reverse this

Evidence that the frame's unsafe surface cannot be held small — say, if it
exceeds 10% of the codebase by phase 02 — would mean the partition is not real
and a microkernel's hardware-enforced boundary is worth its cost after all.
