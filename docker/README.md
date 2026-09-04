# The development environment

One container. Docker is the only thing that has to be installed on the host —
no Rust, no QEMU, nothing on `PATH`.

```powershell
.\docker\dev.ps1 doctor     # is this machine ready, and what will be slow
.\docker\dev.ps1 build      # once, and again after a toolchain pin bump
.\docker\dev.ps1 lint       # cargo xtask lint
.\docker\dev.ps1 test       # cargo xtask test
.\docker\dev.ps1 run        # cargo xtask run — the kernel, in QEMU
.\docker\dev.ps1 x sweep    # cargo xtask sweep — the seed sweep
.\docker\dev.ps1 shell      # everything else
```

On Linux or macOS, or inside WSL2, skip the wrapper:

```bash
docker compose -f docker/compose.yaml build dev
docker compose -f docker/compose.yaml run --rm dev cargo xtask lint
```

The sweep is listed here and documented in `README.md` rather than the other
way round, and the split is deliberate: this file is *what the environment is*
and that one is *what somebody who has just cloned this does with it*. The seed
sweep is the second — it is a released tool with a corpus and a scenario set
behind it, not an everyday chore of the container. It appears in the list above
anyway, because a verb absent from the environment's own command list reads as
a verb the environment does not support. `E1-R01`, RFC 0055.

## Why this exists

Three reasons, in the order they matter.

**`BOOTSTRAP.md` says this scaffold has never been compiled**, because it was
written on a machine with no Rust toolchain and no QEMU. That is task `E0-B01`
and it is the first thing on the list. A container removes the excuse: the
toolchain and the emulator arrive with the image.

**The environment is part of the tree.** The release contract in
`docs/the-long-plan.html` states that no step may exist which is not in the
repository — and a developer's laptop is a step. A `Dockerfile` beside the
source is that step, written down and versioned like everything else.

**It is the same environment CI runs.** Today the CI workflow installs its
tools per job. Once the image exists, the jobs can run *in* it, and the class
of failure where a build passes locally and fails in CI stops being possible.
That change is proposed in "Using this in CI" below rather than applied.

## What is inside

| | | Why |
|---|---|---|
| Rust | from `rust-toolchain.toml` | The pin is stated once, in the repository. The image reads it at build time, so the container and CI cannot drift from the tree. Bumping the pin is a commit there, and a rebuild here. |
| Targets | `x86_64-unknown-none`, `aarch64-unknown-none` | Both, because the AArch64 job is not a portability nicety — x86-64's total store order hides the ordering bugs the ring is exposed to. |
| Components | `rust-src`, `llvm-tools`, `clippy`, `rustfmt` | `rust-src` for `build-std`, `llvm-tools` for the coverage instrumentation wired at M0. |
| QEMU | `qemu-system-x86`, `qemu-system-arm` | `cargo xtask run` drives `qemu-system-x86_64` with `isa-debug-exit`, and asserts on exit code 33. |
| Image tools | `xorriso`, `mtools`, `dosfstools` | Not needed yet. Needed at `E0-B02`, when a bootloader handoff arrives and the kernel has to be put in something bootable. |
| Debugging | `gdb`, `lld`, `file`, `jq`, `python3` | The ordinary tools every task in `TODO.md` quietly assumes. |
| **`full` image only** | `cargo-deny`, `cargo-llvm-cov`, `cargo-nextest` | The dependency-policy job, coverage summarising, and a faster test runner. Pinned by version; an unpinned tool is an ambient dependency, which is the thing this image exists to remove. |
| **`full` image only** | `kani` | The bounded model checker `cargo xtask prove` runs against the capability table and against the ring's validation paths. It brings a compiler with it, which is why it is here and not in `dev` — see "Two toolchains, and why the second one is not a mistake" below. |

Two images, one Dockerfile: `dev` is the default and covers every CI job except
dependency policy and proof. `full` adds the four tools above and takes longer
to build.

```powershell
.\docker\dev.ps1 build full
.\docker\dev.ps1 full cargo deny check
```

## Two toolchains, and why the second one is not a mistake

The `full` image contains two Rust compilers. That is deliberate, it is the
decision in RFC 0022, and it is worth a paragraph because the second one is
older than the first and looks like a mistake until you know what it is for.

| | Which | Who chose it | What it builds |
|---|---|---|---|
| The pin | `rust-toolchain.toml` | this repository | everything in the workspace, and every number in `claims/` |
| The checker's | `nightly-2025-11-21`, installed by `cargo kani setup` into `/opt/kani` and `/opt/rustup` | Kani | `kernel/proofs` and `ring/proofs` only, and only when `cargo xtask prove` runs |

**A verification tool's toolchain requirement is the tool's business.** Kani
pins a specific nightly because its compiler plugin is built against a specific
rustc internal API; that pin moves when Kani moves, on Kani's schedule. Moving
`rust-toolchain.toml` to meet it would invert the relationship between the thing
being measured and the thing measuring it — the pin is what every claim in
`claims/` was measured under — and `CLAUDE.md` forbids moving it as a side
effect of another change for exactly that reason. So the checker gets its own,
in the image target only the checking job uses.

Nothing else in the tree is compiled by it. `kernel/proofs` is outside the
workspace (`Cargo.toml`'s `exclude`), so `cargo xtask verify` never builds it
and `cargo xtask test` never sees it. Deleting the directory and the two jobs in
`nightly.yml` — `prove` and the `image_full` build it is the only consumer of —
is the whole of undoing this, which is what keeps the arrangement cheap to
abandon.

**What it costs.** 1.48 GB on top of the `full` image, measured rather than
estimated: `docker images` says 4.18 GB against 2.70 GB without it, which is a
483 MB release bundle under `/opt/kani` and a 573 MB rustup toolchain. The
launcher itself compiles in about fifteen seconds. `cargo kani setup` needs the
network at image-build time — it downloads both — so a build behind a proxy that
cannot reach GitHub releases fails at that layer and says so.

That layer is also the first in this file to have found a *builder* network
problem rather than a host one, and it is worth knowing before spending an hour
on it. On this Windows machine the BuildKit container resolved
`static.crates.io` and `objects.githubusercontent.com` to IPv6 addresses it had
no route to, so the layer failed with `Could not connect to server` while the
identical commands succeeded under `docker run`. The fix is one flag:

```powershell
docker build --network=host -f docker/Dockerfile --target full -t f-dev:full .
```

It is a property of the builder, not of this Dockerfile, so it is recorded here
rather than worked around in the file. GitHub's runners are not expected to need
it — they are Linux hosts with working IPv4 to both hosts — but nothing has
built this image on one yet, so that is an expectation and not an observation.
The nightly `image_full` job is where it will first be tested, and it is a job
of its own precisely so that being wrong about this costs one check rather than
seven.

**And what it does not cost.** Nothing in `claims/`. The checker produces a
verdict rather than a number, so a Kani upgrade triggers no re-measurement.
That is the property that makes a second toolchain affordable at all, and it is
the first thing to check before adding a third.

```powershell
.\docker\dev.ps1 build full
.\docker\dev.ps1 full cargo xtask prove
```

## What this environment is **not** for

**Do not collect a number here.** The container sets `F_ENVIRONMENT=container`
so that a claim harness can refuse to record one, and the reason is not
fussiness:

- QEMU runs in software emulation unless `/dev/kvm` is present, which on
  Windows needs nested virtualisation in WSL2 — available on Windows 11 with a
  configured `.wslconfig`, absent on Windows 10. Correctness is unaffected.
  Speed is roughly an order of magnitude down, and *jitter* is not merely
  slower, it is a different distribution.
- Even with acceleration, a container on a laptop shares its cores, its cache
  and its memory bandwidth with a desktop, a browser and a virus scanner. The
  reservation rules the hard class depends on are not in force.
- `claims/README.md` requires a named baseline and a reproduction command.
  "Measured in a container on somebody's laptop" is neither.

Timing claims come from a bare-metal Linux host and, from `E5`, from the
hardware lab. Everything else — building, linting, unit tests, the kernel
booting far enough to say `M0 ok`, simulation once it exists, fuzzing, coverage
— is exactly what this environment is for.

## Two things to know before the first `run`

**The first kernel build needs the network.** `cargo xtask run` builds the
kernel with `-Zbuild-std`, which compiles a sysroot and therefore resolves
crates the first time it does so. After that it is cached in the registry
volume and the build is offline. If it fails with `spurious network error`,
that is what it is; retry, and consider `CARGO_NET_RETRY=10`.

**QEMU here is Debian bookworm's 7.2.** Fine for everything through `E1`. The
storage epoch is where it stops being fine: zoned-device emulation, which
`E2-P10`'s write-amplification claim needs, is materially better in QEMU 8 and
9. When that day comes the fix is one line — build with a newer base:

```powershell
docker compose -f docker/compose.yaml build --build-arg BASE=debian:trixie-slim dev
```

Left as bookworm for now because the older, more widely deployed base is the
better default until a task actually needs the newer emulator.

## Windows specifics

**Build output lives in a volume, not in the working tree.** `target/` is a
named Docker volume. A Rust target directory is tens of thousands of small
files, and on Windows every one of them crossing the filesystem boundary costs
a syscall nobody needs; the volume is the single largest speed difference
available. The cost, stated because it surprises people: `target\` looks empty
from Windows. `.\docker\dev.ps1 export` copies it out when something needs
looking at from the host.

**Line endings.** The container runs shell scripts and `xtask` against the
bind-mounted tree. If Git ever converts this repository to CRLF on checkout,
scripts will fail inside the container with errors that name the wrong problem.
Before the first commit:

```
git config --global core.autocrlf input
```

and a `.gitattributes` with `* text=auto eol=lf` is worth adding when the
repository becomes a Git repository.

**The repository path contains non-ASCII characters.** `C:\Users\Дмитрий\...`
works with Docker Desktop's bind mounts, and it is also the first thing to rule
out if a mount ever fails in a way that makes no sense. The robust alternative
is also the fast one — see below.

**The fastest configuration, if the bind mount ever feels slow.** Clone into
the WSL2 filesystem rather than into `C:\`, and run everything from there:

```bash
wsl
cd ~ && git clone <repo> F && cd F
docker compose -f docker/compose.yaml run --rm dev cargo xtask lint
```

Files then never cross the Windows boundary at all. Editors reach them over
`\\wsl$\Ubuntu-24.04\home\<user>\F`, and VS Code's WSL extension treats it as
a local folder.

**Memory.** QEMU, `rustc` and a language server together want more than Docker
Desktop's default. If builds are being killed, raise the WSL2 limit in
`%UserProfile%\.wslconfig`:

```ini
[wsl2]
memory=8GB
processors=4
```

## Using this in CI

The current workflow installs QEMU in one job and relies on the runner's
toolchain in others. Once this image is published, each job becomes:

```yaml
jobs:
  policy:
    runs-on: ubuntu-latest
    container: ghcr.io/<owner>/f-dev:latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo xtask lint
```

which deletes the per-job tool installation, makes "works on my machine"
structurally the same statement as "works in CI", and gives the pull-request
gate its ten-minute budget back.

**Applied**, and the gate builds the image itself. The `apt-get install
qemu-system-x86` in the kernel job is gone, and so is the dependency job's
action-installed `cargo-deny` — that job runs the `full` image's copy, at the
version this `Dockerfile` pins, which matters more there than anywhere else
because it is the job whose whole subject is dependencies.

**The image is a job in `ci.yml`, not a workflow somebody dispatches.** The
first attempt at this got it wrong in an instructive way: `ci.yml` named a tag
that a separate `image.yml` published on pushes to `main`, so the gate had a
prerequisite no run of the gate produced. On a tree where that workflow had
never fired, all ten jobs failed at "Initialize containers" with `manifest
unknown` — before a single step ran. That is this file's own rule, broken by
the change quoting it: a workflow somebody has to remember to dispatch is
institutional knowledge with a YAML file in front of it. `image.yml` is gone;
the gate owns its environment.

The tag is derived from the files that define the environment —
`env-<hash of Dockerfile, entrypoint, rust-toolchain.toml>` — rather than from
the commit. A commit that does not touch the environment is a cache hit, and CI
is pinned to an immutable tag rather than to `:latest`, which closes half the
gap the section below admits to.

The AArch64 half is not one `--platform linux/arm64` away, and the note above
was optimistic about it. Building arm64 under binfmt emulation means installing
a Rust nightly and a QEMU inside an emulated userland: tens of minutes, and
failures with nothing to do with this tree. The `image` job builds each
architecture natively — `ubuntu-latest` and `ubuntu-24.04-arm` — and a
`manifest` job assembles one multi-architecture tag, so both CI jobs name the
same image. That matters for the AArch64 job in particular: its entire purpose
is to disagree with the x86-64 one, and two environments sharing one name is
the worst possible state for a job whose job is disagreement.

## Rebuilding, and when

| When | Command |
|---|---|
| `rust-toolchain.toml` changed | `.\docker\dev.ps1 build` — and per `claims/README.md`, a pin bump requires a full claims re-run |
| A system package is needed | edit `docker/Dockerfile`, then `build` |
| Something is inexplicably broken | `.\docker\dev.ps1 clean` then `build`. This drops the target and registry volumes and costs one index download |

## Git trusts the tree it is handed

`git config --system --add safe.directory '*'`, in the Dockerfile, and it is
worth knowing why rather than finding it by grep.

Git refuses a repository whose working tree is owned by another uid — a real
defence on a shared machine, where somebody else's checkout can run hooks as
you. Neither half of that holds here. The image is handed exactly one tree, by
whoever ran it, and every uid inside it is an artefact of how the tree arrived:
a bind mount carries the host's ownership, and a CI runner checks out as one
uid and then runs the container's steps as another.

It is set here rather than in each caller because the failure it causes does
not look like itself. `git diff` is one of the few commands that tolerates
running outside a repository, so it renders the refusal as *warning: Not a git
repository* and exits non-zero — which is how the claims job came to report a
byte-identical file as stale, and how `cargo xtask release` came within one
step of printing a manifest for a tree it could not name.

What would reverse it: an image handed a tree it did not ask for — building an
untrusted pull request in a runner shared with other work. Then the ownership
check is load-bearing again, and the safe directory should be named rather than
`*`.

## Reproducibility, honestly

The toolchain, the targets, the components and the three optional tools are
pinned. The Debian base image and its packages are not pinned by digest, so two
builds a month apart can differ in ways nobody chose — which is the ordinary
state of container images and is worth knowing rather than assuming away.

Pinning the base by digest is one line, and the digest belongs in the
`ARG BASE` default when the project starts caring:

```
ARG BASE=debian:bookworm-slim@sha256:<digest>
```

Until then this image is *repeatable* — everyone gets the same tools — without
being *reproducible* in the sense `E2-P06` uses the word. The generation root
hash that has to reproduce byte for byte is produced by the build, not by the
image, so this is a smaller gap than it looks. It is still a gap.
