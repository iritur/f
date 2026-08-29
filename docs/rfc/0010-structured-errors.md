# RFC 0010: Errors name a domain, and cancellation is not an error

- Status: accepted
- Date: 2026-08-28
- Affects: `abi/`, every service opcode space

## Decision

A negative `Cqe.result` is `-((domain << 16) | code)`, with `Cqe.ext` carrying a
per-domain detail. Six domains: the four refusals this architecture actually
distinguishes, plus the two every wire protocol needs.

- `AUTHORITY` — you do not hold the capability, or hold it without this right.
- `ADMISSION` — the reservation was refused; the deadline could not be promised.
- `RESOURCE` — a quota, a budget or a device limit was reached.
- `PEER` — the far side is gone, restarted, or speaks a version this channel did
  not negotiate.
- `ARGUMENT` — the entry is malformed. Unknown opcode, unknown flag, non-zero
  reserved field: refused, never ignored.
- `DEVICE` — the hardware reported a failure.

Two rules travel with it. **Cancellation is a completion flag, not an error
code** — it is `cflags::CANCELLED` and must never be encoded as a negative
result. **Partial completion is an explicit count**, never inferred from a short
result, so a caller never has to guess whether fewer bytes means "less than you
asked for" or "something went wrong".

## Context

`errno` is a flat integer space of about a hundred and thirty values, shared by
every subsystem in the kernel, with no room for a detail and no record of which
layer produced it. `EINVAL` is the most common answer a Linux syscall gives and
it names nothing at all; recovering what actually happened means reading the
kernel source for that version.

That distinction matters more here than it does there, because F's design turns
on refusals a caller is expected to *handle*: admission control refuses
reservations, capability checks refuse authority, quotas refuse allocations. The
resource document says a refused reservation is an error the caller must handle
exactly like a failed allocation — which requires the caller to be able to tell
it apart from a device fault, mechanically, on the hot path, without a lookup
table maintained by hand.

The ABI as shipped said only "negative values are errors", which reproduces the
drawback exactly, one milestone into the project.

## Consequences

Error paths become testable by domain: the simulator can inject "refuse every
`ADMISSION` in this run" and exercise a whole failure class at once, rather than
one code at a time.

The domain is the stable part and the code is the detailed part, so a service may
add codes freely and may not add domains. Six is a small enough number to hold in
mind, which is the property `errno` lost by growing.

The cost is that `errno`'s universality is real, and a new vocabulary is a real
tax on anyone porting code. F carries no POSIX obligation, so it pays that tax
once, at the boundary, deliberately.

## What would reverse this

Nothing about the split. If the specific domain list turns out wrong, that is a
versioned ABI addition under RFC 0011 rather than a reversal — which is why the
error space and the negotiation rules were settled in the same milestone.
