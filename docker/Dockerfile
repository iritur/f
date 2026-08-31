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

ARG BASE=debian:bookworm-slim

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

ENV F_UID=1000 F_GID=1000

COPY docker/entrypoint.sh /usr/local/bin/entrypoint
RUN chmod +x /usr/local/bin/entrypoint \
 && mkdir -p /work/target \
 && chown -R 1000:1000 /work

WORKDIR /work
ENTRYPOINT ["/usr/local/bin/entrypoint"]
CMD ["/bin/bash"]
