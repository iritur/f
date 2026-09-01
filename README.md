# F

A research operating system built to exploit the last decade of silicon that
current systems do not use.

**Thesis.** Mainstream operating systems are built on a hardware model that
stopped being accurate around 2015: uniform cores, one flat memory tier,
devices that need the kernel to mediate every access, accelerators that are
just files. A decade of capability — user-level interrupts, device-side address
translation, memory tiering, zoned storage, on-package matrix engines — is
either unused or retrofitted into abstractions that predate it. F is the
argument that designing *for* the current machine, rather than adapting *to*
it, is worth a large constant factor in both speed and energy.

Speed and efficiency are therefore not separate promises. They are the
predicted consequences of the thesis, which is a stronger position to defend.

## Status

**Milestone M0.** Boots, reports, and is already deterministic. Nothing else
works yet, and the design documents are considerably ahead of the code — which
is the intended order.

## Quick start

```
cargo xtask verify   # lint, test, boot — everything, before asking for review
cargo xtask run      # boot the kernel in QEMU, assert on its exit code
cargo xtask test     # workspace tests
cargo xtask lint     # the four policy checks
cargo xtask claims   # what is measured, and what gates
```

Requires the pinned toolchain in `rust-toolchain.toml` and
`qemu-system-x86_64`. Both are in the development container, which is the
supported route: `.\docker\dev.ps1 build` then `.\docker\dev.ps1 verify`, on a
machine whose only prerequisite is Docker.

## Reproducing a published number

Every number this project publishes can be re-derived by somebody who was not
there. That is not a courtesy; it is what makes it a number rather than an
anecdote, and `claims/README.md` is where the rule is written down.

```
cargo xtask reproduce                       # every claim, and what each needs
cargo xtask reproduce ring-submit-latency   # re-run one, here, now
```

The command prints the claim, the commit it is being run at, the class of
machine it requires, and what *this* machine is — then runs the claim's own
published reproduction and ends by saying plainly whether the number was
recorded.

**Expect it not to be, and that is the interesting part.** A timing measurement
is only recorded on a machine that can defend it, described in
`claims/runner-class-A.md`: bare metal, all four of RFC 0007's reservation
components obtained by partition, thermally stable. Anywhere else — a laptop, a
shared cloud runner, the development container — the workload still runs, the
distribution is still drawn and printed, and recording is refused with the
reason. A number with no environment attached is how a benchmark becomes
marketing, so the default is to refuse.

`cargo xtask lint-reproduce` is the standing check that every claim's published
command still resolves inside this tree, so a reproduction cannot rot into a
step somebody has to already know.

## Layout

```
abi/          Wire types crossing a trust boundary. repr(C), no generics, no Drop.
env/          The determinism substrate. Read this before anything else.
ring/         The universal system interface. One implementation, kernel and user.
kernel/       The frame. The only code permitted to be unsafe.
user/init/    The first component. Proves the protocol needs no unsafe above the frame.
xtask/        Build orchestration, and where written policy becomes an executable check.
third_party/  Imported driver source. Delimited, differently licensed, ring-only reachable.
claims/       Every number published in docs/design, with its baseline and threshold.
docs/design/  The five design documents.
docs/rfc/     Decisions, and reversals.

intent/       Where a change starts: intent, then spec, then plan.
evals/        Tasks that check the agent configuration still works.
ops/          What is watched, and what a deviation is permitted to cause.
.claude/      Skills, hooks and subagents. Reviewed like code, because it is.
```

`docs/sdlc.md` is the route a change takes through all of that, and `CLAUDE.md`
is the one page an agent reads before touching anything.

## The three things a newcomer should know

**Determinism is not optional.** Nothing observes time, randomness or ordering
except through `f_env::Env`. `(seed, commit)` reproduces a run byte for byte.
This is why whole-system simulation is possible here and not on Linux, and it
is the one property that cannot be retrofitted. RFC 0004.

**The frame is three crates.** `abi`, `ring`, `kernel` may use `unsafe`.
Everything else inherits `unsafe_code = "forbid"` from the workspace and cannot
opt out silently. RFC 0001.

**Claims gate the build.** A number without a named baseline, a published
workload and a one-command reproduction is an anecdote. `claims/README.md`.

## Reading order

1. `docs/design/fast-path.html` — the architecture and the five bets
2. `docs/design/ring-scene-boot.html` — ring, compositor, semantic layer, M0-M6
3. `docs/design/deadline-all-the-way-down.html` — the five resource subsystems
4. `docs/design/proving-ground.html` — the evidence apparatus
5. `docs/design/lineage-and-debts.html` — what this owes, and where it loses

## Licence

Apache-2.0 OR MIT for everything in this repository except `third_party/`.
See `LICENSING.md` — the licence boundary and the isolation boundary are
deliberately the same boundary.
