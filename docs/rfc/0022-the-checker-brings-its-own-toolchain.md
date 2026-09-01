# RFC 0022: The checker brings its own toolchain

- Status: accepted
- Date: 2026-09-01
- Affects: `rust-toolchain.toml`, `docker/`, `E0-P16`, `docs/TESTING-STATUS.md`, `ring/tests/litmus.rs`, and `claims/README.md`, which this deliberately does not trigger

## Decision

A verification tool's toolchain requirement is **the tool's business, not the
tree's**. RustMC pins its own Rust nightly and its own LLVM major; it gets both,
in an image target only the checking job uses. `rust-toolchain.toml` does not
move for it, now or when RustMC's pin next moves.

The checker produces a verdict, not a number. Nothing it reports enters
`claims/`, so this triggers no claims re-run and no re-measurement — which is
the property that makes a second toolchain affordable at all.

## Context

`E0-P16`'s exit named one load-bearing unknown: whether GenMC can be built
against an LLVM major that the pinned rustc agrees with. It has been measured,
and the answer is no, in both directions at once.

| | Rust | LLVM |
|---|---|---|
| This tree | `nightly-2026-08-01` (rustc 1.99.0-nightly) | **22.1.8** |
| RustMC | `nightly-2025-08-20` | **21** |
| GenMC upstream | — | 15 – 20 |

GenMC's README states the constraint that makes this binding: *"GenMC must be
compiled against the same LLVM major version used by the Rust installation."*
There is no configuration of the two that meets in the middle. RustMC is already
one major ahead of upstream GenMC and still one behind us.

Three options were live.

**Move the pin back to meet the checker.** Rejected, and not only because
`CLAUDE.md` forbids moving it as a side effect of another change. The pin is
what every claim in `claims/` was measured under; moving it to satisfy a test
tool inverts the relationship between the thing being measured and the thing
measuring it.

**Wait for RustMC to reach LLVM 22.** Rejected, and this is the option worth
arguing with, because it is the one that costs nothing today. It has no trigger.
RustMC pins a *specific* nightly, deliberately — it is a research tool tracking
its own LLVM, not a crate tracking rustc. Meanwhile this tree bumps its pin as a
reviewable commit whenever there is reason to. **Both ends move, and they move
apart.** A plan whose completion condition is "someone else's tool catches up to
a pin we keep moving" is a preference wearing a decision's clothes, which is the
thing the last section of this template exists to catch.

**Give the checker its own toolchain.** Accepted. It is the only one of the
three whose cost is bounded and nameable: one more image target, built by one
job, pinned by RustMC's requirements rather than ours.

### What the measurement changed about the task

`cargo rustmc test` compiles a **crate's test targets** to LLVM IR and explores
their interleavings exhaustively. That is not a bigger version of what the
litmus job does — it is a different shape of test, and the existing suite cannot
be handed to it. `ring/tests/litmus.rs` runs 500 000 rounds across two threads;
exhaustive exploration of that is not a long run, it is a non-terminating one.

So `E0-P16` does not mean *run the litmus tests under a checker*. It means
**write the small tests a checker can exhaust** — two threads, one or two
entries, a handful of operations — and keep the stress suite beside them. The
two answer different questions and neither replaces the other: one explores what
RC11 permits, the other what a real machine does with a real store buffer at a
volume no exhaustive search will reach.

## Consequences

**Easy.** The checking job is isolated. A RustMC upgrade, or its abandonment,
touches one image target and no source file. The primary pin stays reviewable on
its own terms, and `claims/` stays measured under one compiler.

**Hard.** Two Rust toolchains in the repository is two things to explain to a
newcomer, and the second one is a year older than the first. Whoever adds it
owes `docker/README.md` a paragraph saying which is which and why the old one is
not a mistake.

**Foreclosed, and stated rather than discovered:** the checker verifies the
protocol **as written**, not the IR this tree ships. Different rustc, different
IR. What survives that difference is exactly the part being checked — atomic
operations and their orderings are specified by the language, not chosen by the
backend — but "survives" is an argument, not a measurement, and any citation of
a RustMC result owes the reader this sentence. The gap is real. It is also much
smaller than the one that exists today, which is no model check at all.

## What would reverse this

**RustMC or GenMC supporting the LLVM major that the pinned rustc bundles.**
Then delete the second toolchain and check with one. This is the outcome to
prefer and the reason the checking job is kept isolated enough to make deleting
it cheap.

**`f-ring` failing to compile under `nightly-2025-08-20`.** This is not yet
known, and it is the one fact that would make this decision unworkable rather
than merely awkward. The evidence available says it should: `ring` and `abi` are
edition-2024 crates with no `#![feature]` gates, and edition 2024 has been stable
since 1.85. But the evidence is an inference and the measurement is one command:

```
cargo +nightly-2025-08-20 build -p f-ring
```

If it fails, this RFC is superseded and "wait" comes back — with a trigger it
does not currently have, which is that the checker's own pin must reach a rustc
this tree's source compiles under.
