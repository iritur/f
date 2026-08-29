# RFC 0004: The determinism substrate is mandatory from M0

- Status: accepted
- Date: 2026-08-28
- Affects: `env/`, every crate in the workspace, `xtask lint-determinism`

## Decision

Nothing in the system observes nondeterminism except through an injectable
`Env`. Time is not an instruction, it is a capability. Randomness is a
capability. Interrupt arrival, ring consumer ordering, core assignment and
allocation addresses are decided by a policy that is real in production and
seeded under test.

The contract: `(seed, commit_hash)` reproduces a run byte for byte.

## Context

Deterministic simulation is the strongest known technique in systems
reliability — FoundationDB on the order of a trillion simulated CPU-hours,
TigerBeetle reproducing any bug exactly from a seed. It is almost never applied
to operating systems because kernels leak nondeterminism at thousands of
unmarked places.

F's architecture removes nearly all of them, and not deliberately: one ring
interface gives a single interception point, no ambient authority means no
hidden channels, user-space drivers make a modelled device a component
substitution, and content-addressed state makes whole-system comparison a hash
comparison. Those decisions were made for speed and safety and hand simulation
its preconditions for free.

## Consequences

This is the one property on the list that cannot be retrofitted. Every other
testing layer can be added at ordinary cost; this one means re-plumbing every
nondeterminism source in a system that has grown around their absence. That is
why it is in M0 and why `cargo xtask lint-determinism` fails the build rather
than warning.

The allow-list is deliberately tiny and each entry carries a reason. There is
exactly one `rdtsc` in the tree, in `kernel/src/arch/x86_64/mod.rs`, and it is
the implementation of the hardware `Env`.

The cost is real: hash maps with seeded iteration order are banned in favour of
ordered maps, and every subsystem must thread an `Env` rather than reaching for
a global clock.

## What would reverse this

Nothing short of abandoning simulation as the primary correctness technique. If
that happened, this RFC would be superseded rather than relaxed — a partially
deterministic system has the costs of both approaches and the benefits of
neither.
