# syntax=docker/dockerfile:1.7
#
# The F development environment.
#
# One image, four properties:
#
#   1. The toolchain comes from `rust-toolchain.toml`, not from this file. The
#      pin is stated once, in the repository, and a bump is a reviewable commit
#      there rather than a drift between somebody's laptop and CI.
#   2. It contains everything `cargo xtask lint | test | run | coverage` needs,
#      including QEMU, so a clean machine reaches a green build with Docker as
#      its only prerequisite.
#   3. It is the same environment CI can run in. "No step exists that is not in
#      the tree" is a release-contract rule, and a developer's laptop is a step.
#   4. It is deliberately *not* a measurement environment. See docker/README.md.
#
# Build context is one file: see .dockerignore.

# Trixie, and the storage epoch is what moved it. `E2-B02` needs a QEMU that
# knows about zoned block devices at all, and bookworm's is 7.2, which does not.
# Trixie's is 10.0, which does — with a limit this file is not the place to
# state and `docs/TECHNICAL-DEBT.md` is.
#
# It brings a second thing with it, and that one was already written down as a
# reversal condition: see the Kani block in the `full` stage, whose own
# *what would reverse this* is "this image's base moving past 2.39". Trixie
# carries glibc 2.41, so it has.
ARG BASE=debian:trixie-slim

# ---------------------------------------------------------------------------
# base — system packages only. Nothing here changes when the toolchain moves.
# ---------------------------------------------------------------------------
FROM ${BASE} AS base

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8

RUN apt-get update && apt-get install -y --no-install-recommends \
      # build essentials: proc macros, build scripts and `cc` for the host
      build-essential \
      pkg-config \
      # the emulator the kernel target runs under, both architectures, because
      # the AArch64 job is not a portability nicety — x86-64's total store
      # order hides the entire class of ordering bug the ring is exposed to
      qemu-system-x86 \
      qemu-system-arm \
      qemu-utils \
      # image assembly, needed from E0-B02 when a bootloader handoff arrives
      xorriso \
      mtools \
      dosfstools \
      # debugging a kernel that has no debugger of its own yet
      gdb \
      lld \
      # the ordinary tools every task in TODO.md assumes exist
      git \
      curl \
      ca-certificates \
      python3 \
      jq \
      file \
      less \
      procps \
    && rm -rf /var/lib/apt/lists/*

# A non-root user whose id matches the default WSL2 user, so files created in a
# bind mount are owned by the person who created them rather than by root.
ARG UID=1000
ARG GID=1000
RUN groupadd --gid ${GID} f 2>/dev/null || true \
 && useradd --uid ${UID} --gid ${GID} --create-home --shell /bin/bash f 2>/dev/null || true

# Git trusts the tree it is handed, because in this image it has no way to earn
# that trust and no reason to withhold it.
#
# Git refuses a repository whose worktree is owned by another uid — a defence
# against a shared machine where somebody else's checkout can run hooks as you.
# Neither half of that holds here. The image is given exactly one tree, by
# whoever ran it, and every uid inside it is an artefact of how the tree got
# mounted: a bind mount carries the host's ownership, and a CI runner checks out
# as one uid and then runs the container's steps as another. The check is
# answering a question the invocation has already settled.
#
# It is set here, once, rather than in each caller, because the failure it
# causes is unreadable. `git diff` is one of the few commands that tolerates
# running outside a repository, so it renders the refusal as *warning: Not a git
# repository* — which is how a CI job came to report a byte-identical file as
# stale, and how `xtask release` came within one step of printing a manifest for
# a tree it could not name. A misdiagnosis this cheap to prevent should not be
# rediscovered per job.
#
# What would reverse this: an image that is handed a tree it did not ask for —
# building somebody else's pull request in a runner shared with other work. Then
# the ownership check is load-bearing again and belongs back on, with the safe
# directory named explicitly instead of `*`.
RUN git config --system --add safe.directory '*'

# ---------------------------------------------------------------------------
# toolchain — Rust, installed from the repository's own pin.
# ---------------------------------------------------------------------------
FROM base AS toolchain

ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:$PATH

# `--default-toolchain none` so that nothing is chosen here. The next step
# reads rust-toolchain.toml and installs exactly what it names: the channel,
# the components, and both bare-metal targets.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --default-toolchain none --profile minimal

COPY rust-toolchain.toml /tmp/pin/rust-toolchain.toml
WORKDIR /tmp/pin
RUN rustup toolchain install && rustup show && rustup component list --installed
WORKDIR /

# The toolchain lives outside any home directory so it survives a bind mount
# over /work and is shared by every user of the image.
RUN chmod -R a+rX /opt/rustup /opt/cargo

# `ENV PATH` covers a command run directly. A *login* shell re-reads
# /etc/profile and rebuilds PATH from scratch, which silently loses the
# toolchain — so state it in both places. `bash -lc` is what CI runners and
# habit both produce, and a missing rustc there is a confusing failure.
RUN printf '%s\n' \
      'export RUSTUP_HOME=/opt/rustup' \
      'export CARGO_HOME=/opt/cargo' \
      'export PATH=/opt/cargo/bin:$PATH' \
      > /etc/profile.d/10-f-toolchain.sh \
 && chmod 0644 /etc/profile.d/10-f-toolchain.sh

# Make the pinned toolchain the rustup default as well.
#
# Without this, `cargo` is a proxy with nothing to proxy to unless the working
# directory contains rust-toolchain.toml — so it works in /work and fails
# everywhere else, including in this Dockerfile's own later stages. The pin is
# not weakened by having a default: a rust-toolchain.toml override always wins,
# and the default is read *from* the pin rather than stated a second time.
RUN cd /tmp/pin \
 && rustup default "$(rustup show active-toolchain | cut -d' ' -f1)" \
 && cd / && cargo --version && rustc --version

# ---------------------------------------------------------------------------
# dev — the default image. Everything the E0 CI matrix runs, except cargo-deny.
# ---------------------------------------------------------------------------
FROM toolchain AS dev

# Miri, and it is in `dev` rather than in `full` because one of `E1-P04`'s three
# properties cannot be checked without it. *No memory unsafety* is not
# observable from inside the program: the ordinary build dies on a signal with
# no seed attached, or does not die at all and reads whatever was next in the
# page. `cargo xtask hostile --miri` is the check, RFC 0046 is why it is a
# separate command at a much smaller count, and a tool that lived only in the
# opt-in image would be a tool the gate could not run.
#
# It is not in `rust-toolchain.toml`'s `components`, deliberately: that file is
# the pin every contributor's laptop reads, and bumping it invalidates every
# claim (`claims/README.md`). Miri is an *environment* dependency the way QEMU
# is — needed to run one command, not to build the system — and the environment
# is what this image is. It costs about 40 MB and one download at image build.
#
# The sysroot is **not** built here, and that is a decision rather than an
# omission. Miri keys its sysroot on the flags it is run with, so a sysroot
# built in this image would either bind the image to one particular `MIRIFLAGS`
# — a second copy of a constant that lives in `xtask` — or be rebuilt on first
# use anyway, and the failure when the two disagree is the unreadable
# *detected a concurrent sysroot build with different settings*. It was built
# here for one commit and produced exactly that. So the minute it costs is paid
# per job, and `claims/0008`'s timings say so.
#
# The `chmod` restores what the toolchain stage established: a component added
# after it would otherwise be readable only by root.
RUN rustup component add miri \
 && chmod -R a+rX /opt/rustup /opt/cargo \
 && cargo miri --version

# F_ENVIRONMENT is announced so that a build can tell it is in the container,
# and so that a timing measurement taken here can be refused rather than
# quietly published as a claim.
ENV CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1 \
    F_UID=1000 \
    F_GID=1000 \
    F_ENVIRONMENT=container

# The image starts as root only long enough for the entrypoint to make the
# mounted volumes writable. Every command lands as the development user.
COPY docker/entrypoint.sh /usr/local/bin/entrypoint
RUN chmod +x /usr/local/bin/entrypoint \
 && mkdir -p /work/target \
 && chown -R 1000:1000 /work

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/entrypoint"]
CMD ["/bin/bash"]

# ---------------------------------------------------------------------------
# full — adds the tools the later epochs need. Slower to build, opt-in.
# ---------------------------------------------------------------------------
FROM toolchain AS full

ENV CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1 \
    F_ENVIRONMENT=container

# Prebuilt where possible; compiled from source only as a fallback, because a
# ten-minute image build that always works beats a three-minute one that does
# not. Versions are pinned: an unpinned tool is an ambient dependency, which is
# the thing this image exists to remove.
#
# A pin here is not free maintenance: cargo-deny reads the live RustSec
# database, so a version old enough to predate a scoring format in it fails to
# parse an advisory it should be reporting. Bump these deliberately, and treat
# a parse failure as the pin being stale rather than as a bad advisory.
ARG CARGO_DENY_VERSION=0.20.2
ARG CARGO_LLVM_COV_VERSION=0.9.0
ARG CARGO_NEXTEST_VERSION=0.9.143

RUN --mount=type=cache,target=/opt/cargo/registry,sharing=locked \
    set -eux; \
    curl -sSfL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash; \
    for pkg in \
        "cargo-deny@${CARGO_DENY_VERSION}" \
        "cargo-llvm-cov@${CARGO_LLVM_COV_VERSION}" \
        "cargo-nextest@${CARGO_NEXTEST_VERSION}" ; do \
      cargo binstall --no-confirm --disable-telemetry "${pkg}" \
        || cargo install --locked "${pkg%@*}" --version "${pkg#*@}" ; \
    done; \
    chmod -R a+rX /opt/cargo

# Kani, the bounded model checker `cargo xtask prove` runs against the
# capability table. E1-P07, RFC 0053.
#
# **It brings its own toolchain, and that is the reason it is here rather than
# in `dev`.** RFC 0022 decided that a verification tool's toolchain requirement
# is the tool's business and gets an image target rather than a move of
# `rust-toolchain.toml`; Kani goes further than RustMC did — `cargo kani setup`
# downloads a release bundle and installs a *rustc of its own*, currently
# nightly-2025-11-21, beside the pin. Nothing this tree builds is compiled by
# it: `kernel/proofs` is outside the workspace and only `cargo xtask prove`
# reaches it.
#
# What it costs, measured on this image rather than estimated: 483 MB of
# release bundle under /opt/kani, 573 MB of extra rustup toolchain, and 15 s of
# compilation for the launcher. `docker images` puts the result at 4.18 GB
# against 2.70 GB without it, so 1.48 GB. That is the reason this is in `full`
# — the `dev` image is pulled by every job in the pull-request gate, and paying
# a gigabyte and a half there for a verb that runs on a schedule would be a cost
# in the wrong place.
#
# `KANI_HOME` rather than the default `$HOME/.kani`: setup runs as root here
# and the entrypoint drops to uid 1000, so a bundle under /root would be one
# the user running the checker cannot read. /opt is where this image already
# keeps the things that outlive a home directory, for the same reason.
#
# The version is pinned, like the three tools above it and for the same reason:
# an unpinned tool is an ambient dependency, and this is the one image whose
# purpose is to not have any. A bump is a reviewable commit here, and it
# invalidates nothing in `claims/` — the checker produces a verdict, not a
# number, which is RFC 0022's own argument for why a second toolchain is
# affordable at all.
# **Installed on both architectures, and the exception that used to be here is
# gone because its own reversal condition arrived.**
#
# What stood here said the `aarch64-unknown-linux-gnu` Kani bundle is linked
# against GLIBC_2.39 while this image's base was bookworm, which carries 2.36 —
# so setup *succeeded* and `kani --version` then failed at the end of a
# two-minute layer, reading as a broken image rather than as an unavailable
# tool. That is how it arrived: the arm64 half of the pull-request gate went red
# on a change that had nothing to do with it.
#
# The paragraph ended with *what would reverse this: Kani publishing an aarch64
# bundle against an older glibc, or this image's base moving past 2.39*. The base
# moved to trixie for `E2-B02`, trixie carries 2.41, and the second clause is
# met — so the `case` on `TARGETARCH` goes and the `RUN` is unconditional again,
# which is what that sentence said should happen.
#
# Kept from the old paragraph because it is still true and still the reason this
# is in `full` rather than `dev`: `cargo xtask prove` runs on amd64 in
# `.github/workflows/nightly.yml`, which builds its own `full-...-amd64` tag, and
# the one pull-request job that needs this image — dependency policy — is amd64
# too. What the arm64 `full` image is for is `cargo-deny`, `cargo-llvm-cov` and
# `cargo-nextest`. The difference now is that the tag's two halves no longer
# quietly differ, which is the failure the manifest job exists to prevent.
#
# `kani --version` is still asserted, so a bundle that stops working is still a
# red layer — on both architectures now rather than on one.
#
# *What would reverse this:* an aarch64 bundle that needs a glibc newer than the
# base carries. The guard would come back, and it would come back with a number
# in it rather than an architecture.
ARG KANI_VERSION=0.67.0
ARG TARGETARCH
ENV KANI_HOME=/opt/kani

RUN set -eux; \
    cargo install --locked kani-verifier --version "${KANI_VERSION}"; \
    cargo kani setup; \
    chmod -R a+rX /opt/kani; \
    kani --version; \
    chmod -R a+rX /opt/cargo /opt/rustup

# --- Verus, the second checker, and it is second on purpose ----------------
#
# RFC 0022 decided a checker's toolchain is the checker's business and RFC 0053
# put a bounded proof over *the file the kernel ships*. RFC 0096 is what happens
# when those two meet Verus: its `#[path]` mechanism does not transfer, because
# Kani verifies ordinary Rust while Verus verifies only what is written inside
# `verus!{ … }`. So the shipped file carries the annotations, and `f-kernel`
# takes `verus_builtin` and `verus_builtin_macros` as ordinary dependencies —
# both of which were checked to compile for `x86_64-unknown-none` under this
# image's pinned nightly before the decision was taken.
#
# The binary release rather than a build from source, for the same reason the
# Kani layer takes a bundle: Verus pins a rustc of its own, and building it here
# would make this image's pin and the checker's pin one decision when they are
# deliberately two.
#
# `verus --version` is asserted, so a release that stops working is a red layer
# rather than a checking job that silently verifies nothing — which is the
# failure mode a verifier has and a compiler does not.
#
# **x86-64 only, and the layer skips rather than fails.** Verus publishes
# `arm64-macos`, `x86-linux`, `x86-macos` and `x86-win` — there is no
# arm64-linux bundle — and the first cut of this made that absence fatal, which
# took the `environment image (arm64)` job red with `no Verus bundle for
# aarch64`. The image is built for both architectures and one checker not
# existing for one of them is not a broken image, so the layer stands down and
# says so.
#
# What it costs is stated rather than hidden: an arm64 `full` image has Kani and
# not Verus, so a deductive proof cannot be run there. That is the same shape as
# the Kani aarch64 guard this file used to carry, arrived at from the other
# direction — and it is why `verus --version` below is asserted only where the
# bundle was installed, because an assertion that cannot run is not an
# assertion.
#
# *What would reverse this:* an arm64-linux release, at which point the `if`
# becomes a `case` with two arms and the arm64 proof job stops being a gap. A
# Verus needing a newer glibc than the base carries would reverse it the other
# way, and would come back as a number rather than as an architecture — which
# is what took the Kani aarch64 case out on trixie.
ARG VERUS_VERSION=0.2026.09.13.671956e
# The rustc Verus pins, installed beside this tree's and never instead of it.
#
# RFC 0022 decided a checker's toolchain is the checker's business, and this
# is that decision arriving as a number: the bundle drives rustc through the
# rustup shim and asks for **stable 1.98.1**, so an image without it fails at
# `verus --version` with `toolchain is not installed` rather than at anything
# resembling a proof. `rust-toolchain.toml`'s nightly is untouched — RFC 0096
# names "two rustc versions must coexist" as a cost of annotating the shipped
# file, and this line is what that cost looks like.
#
# Pinned rather than read out of the bundle so that a Verus release which
# moved its compiler is a red layer with a version in the message, instead of
# an image that quietly fetched a different one.
ARG VERUS_TOOLCHAIN=1.98.1
ENV VERUS_HOME=/opt/verus
ENV PATH=/opt/verus:$PATH

RUN set -eux; \
    # `unzip` in this stage and not the base: it is the checker's business,
    # and a package added to the base invalidates every layer under it —
    # including the Kani bundle, a 483 MB download this image would then take
    # again for the sake of one unarchiver. The first attempt at this layer
    # put it in the base and rebuilt from `apt-get update`.
    apt-get update; \
    apt-get install -y --no-install-recommends unzip; \
    rm -rf /var/lib/apt/lists/*; \
    if [ "$(uname -m)" != "x86_64" ]; then \
      echo "no Verus bundle for $(uname -m); this image has Kani and not Verus" >&2; \
      exit 0; \
    fi; \
    asset="verus-${VERUS_VERSION#release/}-x86-linux.zip"; \
    url="https://github.com/verus-lang/verus/releases/download/release%2F${VERUS_VERSION}/${asset}"; \
    tmp="$(mktemp -d)"; \
    curl -fsSL -o "${tmp}/verus.zip" "${url}"; \
    mkdir -p /opt/verus; \
    unzip -q "${tmp}/verus.zip" -d "${tmp}"; \
    cp -r "${tmp}"/verus-*/. /opt/verus/; \
    rm -rf "${tmp}"; \
    chmod -R a+rX /opt/verus; \
    rustup toolchain install "${VERUS_TOOLCHAIN}" --profile minimal; \
    chmod -R a+rX /opt/rustup; \
    verus --version

ENV F_UID=1000 F_GID=1000

COPY docker/entrypoint.sh /usr/local/bin/entrypoint
RUN chmod +x /usr/local/bin/entrypoint \
 && mkdir -p /work/target \
 && chown -R 1000:1000 /work

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/entrypoint"]
CMD ["/bin/bash"]
