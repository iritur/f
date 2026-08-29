# CLAUDE.md

One page, on purpose. If it grows past what fits on a screen, something in it
has stopped being a convention and become a document — move it to `docs/`.

## Commands

```
cargo xtask verify     # the whole local loop: lint, test, boot. Run this before asking for review.
cargo xtask lint       # four policy checks, then fmt and clippy
cargo xtask test       # workspace tests, x86-64 and AArch64
cargo xtask run        # boot the kernel in QEMU; expects exit code 33
cargo xtask fault pf   # boot into a deliberate fault; pf, ud, df, nx, wx or stack
cargo xtask user       # seven boots: a process violates one rule each, and is killed
cargo xtask timer 60   # run the 1 kHz timer and print a jitter histogram
cargo xtask claims     # the claims registry and what gates
cargo xtask todo E0    # what is available to start now, ranked by what it unblocks
cargo xtask coverage   # host tests with instrumentation
```

Healthy output for `verify` ends with `verify: all green` and exit code 0.
Anything else is a failure, including a `warning:` line — clippy runs with
`-D warnings`.

Requires the pinned toolchain in `rust-toolchain.toml` and
`qemu-system-x86_64`. Do not bump the toolchain as a side effect of another
change: it invalidates every claim (`claims/README.md`).

## Architecture

`abi` wire types, `env` the determinism substrate, `ring` the one system
interface, `kernel` the frame, `user/init` the first component, `xtask` policy
made executable, `third_party` imported drivers behind a licence boundary.
`docs/design/*.html` is the reasoning and is ahead of the code by design;
`docs/rfc/` is where reversals live.

## Conventions

- **Determinism.** Nothing observes time, randomness or ordering except through
  `f_env::Env`. No `Instant::now`, no `rdtsc`, no `thread_rng`, no `HashMap` or
  `HashSet` — iteration order is seeded per process, so use `BTreeMap` and
  `BTreeSet`. New call sites need an allow-list entry with a reason in
  `xtask/src/main.rs`. RFC 0004.
- **The frame.** `unsafe` is permitted in `abi/`, `ring/`, `kernel/` and nowhere
  else; the workspace forbids it everywhere else at compile time. Every
  `unsafe` block carries a `// SAFETY:` comment discharging the obligations the
  `# Safety` section states. RFC 0001.
- **The licence boundary.** The permissive tree never imports `third_party/`.
  Reachable only over a ring. `LICENSING.md`, RFC 0003.
- **Kernel state is per-CPU.** Every mutable `static` under `kernel/` is a
  `PerCpu<T>`, so two cores never reach the same slot and nothing there locks.
  `kernel/src/percpu.rs`, `ring-scene-boot` section 14.
- **Every file starts with** `// SPDX-License-Identifier: Apache-2.0 OR MIT`.
- **Numbers need claims.** Any number that reaches `docs/design/` has an entry
  in `claims/` with a baseline, a workload and a one-command reproduction.
- **Reversals need RFCs.** Changing something already written down means an
  entry in `docs/rfc/`, copied from `0000-template.md`. The section that matters
  is *What would reverse this*.
- **Memory ordering.** The ring rests on one `Release` store and one `Acquire`
  load. Changing either needs a litmus test that fails under the weaker
  ordering. `Relaxed` there passes on x86 and corrupts data on AArch64.
- Comments say *why*. This tree is written for a reader who wants to disagree
  with it, so state the reasoning and the reversal condition, not the mechanics.

## Common mistakes

Added when the same mistake happens twice. Each line is a scar.

- Running `cargo fmt` without `rustfmt.toml`'s `use_small_heuristics = "Max"` in
  effect, then reformatting lines nobody touched.
- Testing the ring only on x86-64. Total store order hides the entire class of
  bug the ring is exposed to; the AArch64 job is where those tests mean
  anything.
- Editing `docs/design/*.html` to change a published number instead of changing
  the claim it renders from.
- Adding a `TODO.md` task with no `exit:`. A task with no exit is a wish.
- Reaching for `HashMap` out of habit in `xtask`, which is checked by the same
  determinism lint it implements.

## How work reaches this repo

`intent/` → spec → plan → diff → review → claim. `docs/sdlc.md` is the whole
route, including which stage an agent is allowed to close on its own.
