# RFC 0003: No Linux runs anywhere; driver source is imported

- Status: accepted
- Date: 2026-08-27
- Supersedes: the static-partition model
- Affects: architecture document section 14, `third_party/`, `LICENSING.md`

## Decision

Nothing boots Linux. Nothing runs beside it. There is no compatibility
partition. What is imported is *driver source*, compiled against a
kernel-API shim into F's own isolated components, running under F's memory
model, scheduler and capability rules.

Import what is not being researched; write what is. Graphics and wireless are
imported. Storage, network, audio, accelerators and the input path are written,
because those are the code paths the claims live on — importing Linux's block
layer to make a storage-efficiency argument would mean importing the thing
being measured.

## Context

An earlier design ran F as a real-time partition beside Linux on one machine,
with a static-partitioning hypervisor. That resolved the driver problem
elegantly and was rejected on direction: a research vehicle whose thesis is
that a clean-slate architecture is worth a constant factor cannot rest its
hardware support on the system it is arguing against.

Fuchsia's driver framework was evaluated as a replacement inventory. Its
*architecture* is the right container and is adopted. Its *drivers* are not
usable: coverage is narrow, its graphics stack supports only unified-memory
devices with discrete-memory GPUs an unported aspiration, and the project is in
maintenance.

## Consequences

Imported drivers run as isolated, IOMMU-confined, restartable components with
no ambient authority, so imported C sits outside the trusted computing base and
the unsafe-code metric survives.

The licence boundary lands on the same boundary. Imported source is GPL and a
component built from it is a derivative work; F's separation is separate
address spaces with no shared symbols, communicating only over a ring, which is
far stronger than the linking argument FreeBSD relies on. See `LICENSING.md`.

The honest risk: graphics drivers assume kernel context. Hosting them in an
isolated user-space component may not work, and the fallback is a documented
in-frame exception with its cost measured rather than hidden.

## What would reverse this

Demonstrating that the shim cannot host a modern graphics driver out of frame
at acceptable cost. The fallback is in-frame with an exception, not a return to
running Linux.
