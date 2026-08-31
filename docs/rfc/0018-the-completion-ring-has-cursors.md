# RFC 0018: The completion ring has cursors, and the layout table did not give it any

- Status: accepted
- Date: 2026-08-31
- Affects: `abi/`, `ring/`, `kernel/`, `docs/design/ring-scene-boot.html` section 02

## Decision

The channel layout in `ring-scene-boot` section 02 places seven regions: a
header, a producer cursor, a consumer cursor, an index ring, an entry array, a
completion ring, and an inline arena. The completion ring is `32*M` bytes of
completions and nothing else. It has no cursors.

A ring with no cursors is not a ring. There is no way to say how many
completions have been posted, none to say how many have been reaped, and
therefore no way to tell an empty completion ring from a full one — which is the
same free-running head-and-tail pair the submission ring already needs and for
exactly the same reasons.

So the layout gains two more cache lines. The completion cursors sit
immediately after the submission cursors, at `0x00C0` and `0x0100`, and the
index ring moves from `0x00C0` to `0x0140`. The published offsets for the
header and the two submission cursors are unchanged; every offset from the
index ring onward moves by 128 bytes, and the ones past it were already
computed from `ring_size` rather than fixed.

## Context

E0-B12 is the first task to build the layout rather than describe it, and
building it is what surfaced this. The submission half had been implemented at
M0 against the same table and did not notice, because the submission half is
the half the table gets right: `f_ring::Channel` borrowed a head, a tail, a
flags word and an entry array, and every one of those has an offset in section
02.

Three alternatives were live.

**Put the completion cursors in the header.** The header has four reserved
words and two would fit. Rejected: the header is read once at setup and the
cursors are written on every operation, so this would put a line that both
peers hammer inside the line that carries the magic and the negotiated version
— false sharing between the hot path and the one structure that must stay
readable when everything else has gone wrong.

**Reuse the submission cursors.** A single `head`/`tail` pair cannot serve two
rings whose occupancies differ, which they do the moment one entry carries
`NO_CQE`.

**Put them after the completion ring, so the published offsets do not move.**
This is the only alternative that preserves the table as written, and it was
rejected on the grounds that it optimises for a document over a format. The
region ordering in section 02 is *header, cursors, rings, arena* — metadata
first, then the arrays, then the payload — and that ordering is the reason the
cursors are adjacent to each other rather than scattered. Preserving a stale
offset by breaking the principle the offset came from is the wrong trade, and
the offsets in question have never been read by anything outside this tree.

The moment to take this is now: no peer has ever been built against the
published table, because until this task there was nothing to build a peer
against.

## Consequences

Makes easy: the completion ring works, and it works with the same code shape,
the same `Release`/`Acquire` argument and the same corrupt-cursor validation as
the submission ring. `f_ring::Poster` and `f_ring::Collector` are the mirror of
`Producer` and `Consumer` and can be read as such.

Makes hard: nothing that was easy before. The offsets were computed from
`ring_size` by `f_abi::layout` from the first line of code that used them, so
no caller spells a literal.

Forecloses: reading a channel mapping written by anything built against the
section 02 table as published. That is an empty set today and will not be after
`E0-B13`, which is why this lands before it rather than after.

Costs 128 bytes per channel. Against a 4 KiB minimum region that is three per
cent, and against the alternative — a completion ring that cannot report its
own occupancy — it is not a trade worth thinking about twice.

## What would reverse this

Evidence that the two completion cursors do not need separate cache lines. The
argument for separating them is inherited wholesale from the submission
cursors, where the measurement exists and is decisive: sharing a line costs
100–150 cycles per operation through false sharing. That measurement is about a
pair where *both sides write on every operation*.

The completion ring is not obviously that pair. A service posts completions and
a client reaps them, and a client that batches its reaping — takes eight
completions and advances the tail once — writes its cursor an eighth as often
as the service writes its own. If the completion cursors turn out to be cold
enough that sharing a line is free, this reclaims 64 bytes per channel and the
layout loses a region.

The measurement that would show it: `claims/0001-ring-submit-latency.toml`'s
workload extended to drive completions, run with the two cursors on one line
and on two, on a machine where the two ends are on different physical cores. A
difference inside the noise is the reversal.
