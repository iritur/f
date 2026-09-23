# RFC 0106: The shared driver supervisor gains an optional client end

- Status: accepted
- Date: 2026-09-23
- Affects: `kernel/src/supervisor.rs` (`Supervising::reaper`), `kernel/src/blk.rs`,
  `kernel/src/net.rs`, `kernel/src/gpu.rs`, `kernel/src/input.rs`, and RFC 0071

## Decision

`Supervising::reaper` is `Option<&Collector>` and was `&Collector`.

RFC 0071 moved into `kernel/src/supervisor.rs` exactly what was byte-identical
across three driver supervisors, and recorded that nothing there takes anything
from a caller but the driver's manifest name. **`E3-B04d`'s driver is the first
that produces rather than answers.** It holds the *client's* end of its own data
ring and the frame holds the server's, so there is no completion for the frame
to reap — and a field that must be supplied and is never read is worse than an
absent one, because the next reader cannot tell which it is.

Three supervisors changed by seven lines between them — `reaper: Some(&reaper)`.

## Context

The two alternatives are written out at the field rather than here, and both are
worse in the same direction:

- **A collector over a completion ring neither side posts on.** A field that is
  supplied, never observed, and cannot be observed — the shape this tree keeps
  finding and calling decoration. `cargo xtask net` is the datapath that
  exercises `Supervising::within`, and a collector nothing collects from would
  have passed it.
- **A fourth copy of the supervisor for the producing case.** That is the thing
  RFC 0071 exists to prevent, and the exit of `E3-B04d` names it in as many
  words: *using `kernel/src/supervisor.rs`'s shared half rather than a fourth
  copy of it*.

The widening was mutation-tested from inside the shared half rather than from
the new caller: changing `NEED_QUEUES` in `kernel/src/supervisor.rs` from
`b"queues"` to `b"queuez"` turns the input boot red, because that boot reads its
manifest through `supervisor::declared` and has no copy to fall back on. A
fourth copy would have survived that mutation, which is what makes it evidence
about the sharing rather than about the driver.

## Consequences

**What this makes easy.** A driver whose direction is inbound. Every other part
of the shared half — the manifest read, the register window, the allocator
order, the serve point, RFC 0008's stop notice — applied unchanged to a device
of a kind none of the first three were.

**What it costs.** One `Option` at four call sites, and the honest statement that
the shared half now has a shape with two cases rather than one. RFC 0071's
sentence — *nothing there takes anything from a caller but the driver's manifest
name* — is narrowed by exactly this field, and that narrowing is what this entry
records.

## What would reverse this

**A second producing driver**, which would make the `Option` a two-case shape
rather than a one-case shape with an exception. At two the question is whether
the producing and answering supervisors are one function with a branch or two
functions with a shared middle, and the answer should come from reading both
rather than from this entry.

**A completion ring the frame does post on for an input device.** `E5-B06`'s
hardware-timestamped input is where that becomes plausible: a device whose own
clock stamps the event may want an acknowledgement path, and if it does, the
`None` arm stops being the accurate one and the field goes back to being
required.
