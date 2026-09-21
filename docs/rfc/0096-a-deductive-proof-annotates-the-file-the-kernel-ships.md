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

## What the checker did when it was finally run, on 2026-09-20

The two questions this RFC left open were whether `verus!{ … }` around a
`#![no_std]` module verifies rather than merely parses, and whether any of the
properties Kani already proves are expressible without restructuring the
functions they are about. The first is answered, and the answer came with
something better than a yes.

The subject was `kernel/src/cap.rs`'s `refund`, reduced to its arithmetic and
given two postconditions — one that should hold and one that should not:

```text
verification results:: 1 verified, 1 errors

error: postcondition not satisfied
  --> probe.rs:46:9
46 |         out.0 + out.1 == object + extent,
   |         failed this postcondition
```

**The floor property verified. The conservation property was refuted.** `refund`
does `slot.extent = slot.extent.saturating_add(bytes)`, so `object + extent` is
*not* preserved: a refund whose `extent + bytes` would pass `u64::MAX` loses the
difference silently. Verus found it over all `u64`.

**Kani cannot reach it**, and that is the whole justification for a second
checker rather than a faster one. `kernel/proofs/src/mem.rs` sets `FRAME_SIZE`
to 256 and the harnesses run an eight-slot table; `u64` overflow is not in that
space and never will be. RFC 0053's own reversal condition says a bounded check
is *a weaker statement kept for its speed rather than for its content*, and this
is the first thing in this tree that shows the difference as a result rather
than as an argument.

Whether the overflow is *reachable* in a running frame is a separate question
and this note does not claim it is: `extent` is bytes remaining in an untyped
region, and a region that large does not exist on any machine here. What the
refutation establishes is that the precondition was never written down, and the
day an account's arithmetic changes there is now something that will say so.

## What the layer cost, which is two facts rather than an estimate

**Verus pins its own rustc and demands it through the rustup shim.** The first
build of the layer failed at `verus --version` with `toolchain '1.98.1-x86_64-
unknown-linux-gnu' is not installed`. This RFC listed *two rustc versions must
coexist* as the sharpest technical risk; it is now a pinned `ARG` in
`docker/Dockerfile` installing stable 1.98.1 beside this tree's nightly.
Pinned rather than read out of the bundle, so a Verus release that moved its
compiler is a red layer with a version in the message.

**`unzip` belongs in the `full` stage and not the base.** Putting it in the base
invalidated every layer under it — including the Kani bundle, a 483 MB download
the image then took again for the sake of one unarchiver.

## What building it established, on 2026-09-21

**The file is `kernel/src/watermark.rs` and it is smaller than this entry
expected.** The decision above is *the shipped file carries the annotations*,
and the cost it priced was the whole of `cap.rs` becoming a file two compilers
read. It is not: Verus runs a compiler front end over its input, so a file that
says `use crate::` needs the crate around it — which is the stand-in machinery
`kernel/proofs` carries for Kani — and a file that names nothing above itself
does not. The three functions an untyped account's watermark is moved by are
arithmetic over four `u64`s and nothing else, so they are their own module,
`verus` reads that module as a crate root exactly as it sits on disk, and
`cap.rs` calls into it. No second crate, no `#[path]`, no stand-ins.

**Three properties, and two of them corrected the code they were written for.**

- `carved` — carving a frame moves the base up and the remainder down by the
  same number, so `object + extent` does not move. `Table::retype` did that with
  two `wrapping_` calls under the comment *"Checked immediately above, so
  neither of these can wrap"*. What was checked immediately above is
  `extent >= FRAME_SIZE`, which bounds the subtraction and says nothing about
  the addition: an account whose `object` was within a frame of the end of the
  address space wrapped to zero and handed out a frame at address zero.
- `refunded` — the conservation this entry's own probe refuted on 2026-09-20,
  now stated and held. The guard that makes it hold is what
  `mutate-saturating-refund` takes away.
- `carvable` — the predicate `Table::grow` selects an account by, so that a
  selection which says yes cannot be followed by a charge that says no. It used
  to be one of its two clauses.

**Neither correction is claimed to be reachable.** `extent` is bytes left in an
untyped region and `object` is a physical address; no machine in this tree has
either within a frame of `u64::MAX`. What the refutations establish is that the
preconditions were never written down, which is a different and checkable
claim — and it is the reason `mutate-saturating-refund` is in `DEFECTS` and
deliberately **not** in `MUTATIONS`. There is no boot provocation that finds it,
which is the whole argument for a second checker rather than a broader first
one.

**The two costs this entry forecast were both paid and both were smaller than
priced.** `f-kernel` takes `verus_builtin` and `verus_builtin_macros`, pinned
with `=` because both are published by date; they pull seven proc-macro crates
behind them and none of those is in the image, because the frame leaf
`cargo xtask generation` measures is the linked artefact and a macro that ran at
compile time is not in it. The frame compiles for `x86_64-unknown-none` under
`nightly-2026-08-01` with `-D warnings`, which is the sharpest risk this entry
named, tested rather than assumed. One line of cost was not forecast:
`use verus_builtin::*` is load-bearing under Verus and has no user under rustc,
so it carries an `allow(unused_imports)` with the reason written beside it.

**The route is `cargo xtask verus`, and it is a step of the nightly `prove` job
rather than a job of its own.** `image_full` is the only image either checker
exists in, and `xtask`'s `proof_schedule` refuses more than one job depending on
it — every job waiting on that image is a job the checkers' toolchains can take
down. Two checkers behind one image is one blast radius; two jobs would be two.
The verb has three phases: the proof, the armed run that must be refuted at the
named property and in the named file, and a `cargo check` of the armed build —
because nothing else in this tree ever compiles that feature, so a defect that
had stopped compiling would keep passing phase two for ever.

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
