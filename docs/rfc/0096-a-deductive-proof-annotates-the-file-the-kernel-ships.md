# RFC 0096: A deductive proof annotates the file the kernel ships

- Status: accepted
- Date: 2026-09-20
- Affects: `kernel/Cargo.toml`, `kernel/src/cap.rs`, `docker/Dockerfile`,
  `xtask/src/main.rs` (a second proof route), `.github/workflows/nightly.yml`;
  RFC 0001, RFC 0022, RFC 0053; and `E2-P04`

## Decision

`E2-P04` puts Verus on the frame's invariants. RFC 0053 already decided what a
proof in this tree is allowed to be:

> a **bounded proof over the file the kernel ships**, not over a copy of it and
> not over a model of it

and mechanised it with `#[path]`: `kernel/proofs` compiles `kernel/src/cap.rs`
a second time, so there is one file. **That mechanism does not transfer**, and
the reason is not incidental. Kani verifies ordinary Rust, so a second compile
of an unannotated file is a complete input to it. Verus verifies only what is
written inside `verus!{ … }` carrying `requires`, `ensures`, `invariant` and
`proof` — so a second compile of an unannotated file gives Verus nothing to
check. The choice is therefore forced and it is worth naming as a choice:

1. annotate `kernel/src/cap.rs` in place, so the file the kernel ships is the
   file Verus reads; or
2. write a Verus model of it beside it, which is *a copy of it or a model of
   it* — the thing RFC 0053 refused by name.

**This RFC takes the first.** The frame's invariant-bearing modules are written
inside `verus!{ … }`, and `f-kernel` takes `verus_builtin` and
`verus_builtin_macros` as ordinary dependencies. RFC 0053's sentence survives
intact; what changes is the dialect the frame is written in.

## What it costs, which is the part to disagree with

**The frame gains two dependencies.** `kernel/` has been kept to a small,
auditable set on purpose, and this adds a proc-macro crate and its runtime
companion to the one tree where RFC 0001 concentrates `unsafe`. That is the
price of the file being one file. A reader who thinks the price is wrong should
argue for option 2 and should then say what *a copy of it and not a model of
it* is supposed to mean afterwards.

**The frame stops compiling with rustc alone.** `verus!{ … }` is a macro, so
the expansion is ordinary Rust and the *target* build is unaffected — but the
source no longer reads as plain Rust, and `cargo build -p f-kernel` acquires a
build-time dependency that must resolve under `rust-toolchain.toml`'s pin.
`CLAUDE.md` says not to bump the toolchain as a side effect of another change,
and this is exactly the change that would be tempted to.

**Two rustc versions must coexist.** Verus ships its own, as Kani does, and RFC
0022 already decided a checker's toolchain is the checker's business. What is
new is that a *runtime* crate of the checker's — `verus_builtin` — must also
compile under **this tree's** pin, because the kernel links it. Kani never
asked that. It was written here as the sharpest technical risk in the decision,
and it was then tested rather than left as one — see below; it compiles. What
survives of the risk is that it compiles *today*, against a crate published by
date.

## What was established before deciding, and what was not

Established, by probe on 2026-09-20:

- The development container reaches the network, so an image layer is not
  blocked by the sandbox: `api.github.com` answers 200 from inside it.
- Verus publishes `verus-<version>-x86-linux.zip` on its releases, so the
  checker is obtainable the same way the Kani layer obtains Kani.
- `vstd` is `no_std`-capable — `#![cfg_attr(not(feature = "std"), no_std)]` —
  and its long list of unstable features is gated on `verus_keep_ghost`, which
  is set when *Verus* compiles it and not when rustc does. So the kernel's
  ordinary build asks far less of it than the verification build does.

- **The dependency compiles under this tree's pin**, which was written here as
  the sharpest risk and was then tested rather than left as one. Both
  `verus_builtin` and `verus_builtin_macros` build clean for
  `x86_64-unknown-none` under `nightly-2026-08-01` with `-Zbuild-std`, as
  ordinary dependencies of `f-kernel`, pulling `verus_syn`,
  `verus_prettyplease`, `syn`, `quote`, `proc-macro2`, `convert_case`,
  `synstructure` and `unicode-ident` behind them. The probe was reverted: a
  dependency with no caller is not a dependency this tree keeps.

  Cargo resolved them against "latest Rust 1.99.0-nightly compatible version",
  so what holds today is a resolution and not a guarantee — a `=` pin on both
  is part of the work, because the next publish of either is free to want a
  newer compiler and the failure would arrive as a toolchain bump nobody asked
  for.

Not established, and each is a way this decision fails:

- that `verus!{ … }` around a `#![no_std]` module with `unsafe` blocks and
  raw-pointer work verifies rather than merely parses;
- that any of the five capability properties Kani already proves are
  *expressible* as `ensures` clauses without restructuring the functions they
  are about — and restructuring the frame to suit a checker is the failure mode
  RFC 0053's *the file the kernel ships* exists to prevent, arrived at from the
  inside.

## Why Kani is not deleted

RFC 0053 named its own reversal: *Verus arriving on the frame at phase 02 — if
the frame's invariants are being discharged deductively, a bounded check of a
subset of them is a weaker statement kept for its speed rather than for its
content. Keep it while it is the faster instrument; delete it when it stops
being the only one.*

That condition has **not** fired and this RFC does not fire it. Nothing is
discharged deductively yet. `cargo xtask prove` keeps its ten harnesses, its
mutation half and its place in the nightly, and the day one property is proved
in Verus the honest move is to compare the two on that one property before
retiring anything.

## What would reverse this

**A future `verus_builtin` not compiling under this tree's pin.** It compiles
today and that was tested, not assumed — but cargo resolved it against the
newest compatible version, and the crate is published dated rather than
semantically versioned. The day it wants a newer compiler, the file the kernel
ships cannot carry the annotations without a toolchain bump, the bump
invalidates every claim (`claims/README.md`), and the choice is between a
frame-wide toolchain change and option 2. If that is the state, the honest
outcome is that `E2-P04` waits rather than that the frame is re-modelled. The
defence is an exact `=` pin on both crates and noticing the day it stops
resolving.

**An invariant that needs the function rewritten to be stated.** One is a
translation; a pattern of them means the proof is shaping the frame, and a
frame shaped by its checker is a frame whose properties are about the checker.
The test is `cargo xtask mutate`: if a defect the proof is supposed to catch
stops being expressible in the mutated build, the restructuring has gone too
far.

**Verus and Kani disagreeing on a property both state.** That is the most
valuable outcome available here and it must not be resolved by preferring the
newer tool. Two checkers over one file disagreeing is evidence about the file
or about a checker, and either is worth more than a green run.
