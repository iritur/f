# RFC 0011: Peers negotiate a version, they do not match one

- Status: accepted
- Date: 2026-08-28
- Affects: `abi/`, `ring/`, the component model

## Decision

A `ChannelHeader` carries four version fields rather than one: the version the
writer speaks, the oldest version it still speaks, the feature bits it offers,
and the subset of those it cannot do without. Setup computes the intersection
and opens at the highest common version with the agreed feature set, or refuses
with a `PEER` error naming what was missing.

Additions go in reserved space and are gated by a feature bit. Removals raise
the floor, which is a visible, reviewable diff in `abi/`.

Negotiation makes compatibility explicit; it does not make the wire permissive.
Unknown flags, unknown opcodes and non-zero reserved fields are still refused
with `ARGUMENT`. Fail closed.

## Context

Linux's single most valuable property is that it does not break userspace. Its
worst internal property is the exact opposite: an unstable in-kernel API that
makes an out-of-tree driver a permanent maintenance burden. F wants the first
and must avoid the second, because the driver-import strategy rests entirely on
the boundary being a *protocol* rather than a kernel API.

The ABI as shipped required `abi_version` to be identical on both sides. That is
stricter than either half of Linux's position, and it quietly falsifies three
arguments the corpus makes elsewhere: that the compositor can be replaced without
recompiling the system, that imported drivers sit behind a stable protocol, and
that a component can be updated in place. Lockstep versioning makes all three
untrue.

Fuchsia's availability annotations and compatibility testing are the worked
answer, and the architecture document already says to take the discipline rather
than the interface language.

## Consequences

Two versions of a protocol are live at once, so the compatibility matrix becomes
a test axis. That is affordable here because a peer is a component substitution:
an "old peer" scenario is a different component, not a different machine.

The frame must implement at least one version behind itself, which is a real
carrying cost and the reason the floor exists — a version below the floor is
refused rather than half-supported. `ABI_VERSION_MIN` is a promise about how far
back the implementation actually goes, so raising it is a decision about who gets
dropped, taken deliberately.

Feature bits are cheap to add and expensive to remove. Expect the bitmap to be
where this design eventually shows its age, and prefer a version bump over a
fifth compatibility bit for the same subsystem.

## What would reverse this

Evidence that no component is ever updated independently of the frame — which
would mean the component model is not real, and the right response would be to
fix that rather than to simplify the header.
