# RFC 0015: Capability operations are bridging calls, and each names its opcode

- Status: accepted
- Date: 2026-08-30
- Affects: `kernel/`, `abi/`, `docs/rfc/0014-the-syscall-door.md`

## Decision

Four calls are added behind the `syscall` entry at M4: `CAP_INSPECT`,
`CAP_DERIVE`, `CAP_REVOKE` and `CAP_MAP`. They exist because a ring is named by
a `Channel` capability, so the capability table has to work before there is any
ring to work it through. All four are **bridging calls** and are retired at M5.

RFC 0014 rule 1 says a call may exist only if it cannot be an opcode on a ring.
Read alone, that forbids all four: every one of them could be an opcode, and at
M5 every one of them will be. Rule 2 is what actually governs them — *every call
added before the ring exists names the thing that replaces it* — and this RFC
records the reading that rule 1 is about **permanent** calls and rule 2 about
**temporary** ones. A call that satisfies rule 2 does not have to satisfy rule 1;
a call that satisfies neither may not exist.

The four, and what retires each:

| call | what it does | retired by |
| --- | --- | --- |
| `CAP_INSPECT` | what a handle names | an opcode on the component's control ring |
| `CAP_DERIVE` | mint a weaker capability, or a copy | the same |
| `CAP_REVOKE` | withdraw everything below a capability | the same |
| `CAP_MAP` | map a frame into an address space | the same, with `Sqe::cap` as the frame handle |

`CAP_MAP` is the one worth arguing about, and it is included deliberately.
Without it the table authorises nothing: a process could mint and withdraw
capabilities all day and never use one, and the negative suite would be checking
a bookkeeping structure rather than a boundary. Milestone M6 says a
capability-restricted process *touches nothing it was not handed*, and a frame
capability that cannot be turned into a mapping is not the thing that sentence is
about.

Two consequences follow that are not obvious and are load-bearing:

**A copy is a child in the derivation tree, not a sibling.** There is no `COPY`
call; a copy is `CAP_DERIVE` with the rights the capability already carries. seL4
places a copy beside its source, where revoking the source does not reach it.
That is the wrong default here: `docs/what-must-be-stated.html` files *nothing
can be revoked* as a structural drawback of the interface F replaces and answers
it with "revoke recursively through a derivation tree", and a revoke a copy
escapes does not answer it. The cost — two holders of equal authority are not
equal, because whoever derived first can revoke the other — is stated in
`kernel/src/cap.rs` rather than hidden.

**A process may not enlarge its own address space.** `CAP_MAP` maps only where a
page table already exists and refuses otherwise, because allocating a table on a
process's behalf is spending memory nobody has accounted for. The account is what
`Untyped` is for and E1 is where it arrives.

## Context

M4 is *capabilities*: a per-process table of typed slots, with derive, copy and
recursive revoke, and a negative suite as the exit criterion. M5 is the first
ring. The order is not negotiable in the other direction — `Sqe::cap` is an
index into the caller's capability table, so a ring whose entries name
capabilities cannot be the mechanism by which capabilities are managed. The
bootstrap has to happen somewhere and the door is the only somewhere there is.

What made this need writing down rather than assuming is that RFC 0014 exists
precisely to make adding a call expensive, and this change adds four at once —
more than doubling the interface that document was written to prevent growing.
Waving that through on the grounds that the milestone says so is exactly the
move rule 1 was written to stop.

Live alternatives at the time:

- **Grant everything statically and add no calls at all.** A process is handed
  its capabilities at build time and can neither derive nor revoke. It would
  pass three of the five properties and could not exercise the other two, and
  the derivation tree — which is the substance of M4 — would ship untested. The
  suite would be checking a table nobody could change.
- **One multiplexed `CAP_OP` call with an operation selector.** One call by the
  count, four by the content, and the count is not what rule 1 is about. It
  would also invent an opcode space at the door, which is the thing rule 3 says
  not to build here and RFC 0011 says not to guess at.
- **Bring the ring forward to M4 and put these on it.** Rejected on the same
  ground as `Sqe`-shaped calls in RFC 0014: it means designing the ring's
  entries against a capability system that does not exist yet, and both would
  then be provisional at once.
- **Ship the table with no way to use a capability — no `CAP_MAP`.** Three
  calls instead of four, and a smaller diff. Rejected because the resulting
  negative suite proves nothing about authority: every refusal would be a
  refusal to update a data structure, and "cannot exceed granted rights" would
  have no rights that reach anything.

## Consequences

Easy: the negative suite is real. A process presents handles, and every refusal
it earns is an authority decision about an object it can otherwise reach — the
grant frame is unmapped until a capability maps it, so `cap=rights` failing to
map it writable is a page that stays read-only rather than a return code.

Hard, on purpose: there are now seven calls at the door and RFC 0014's reversal
condition — *the three calls are still here after M5* — has become a check on
seven. The rule that document asked for, a lint that fails the build on a call
no document names, is still owed. It is E0-D07's, and it is more owed now than it
was: this RFC doubled the surface it would police.

Foreclosed: the door cannot acquire a capability call that is not on the table
above without amending this document, and it cannot keep any of the four past
M5 without amending it again.

## What would reverse this

**A capability operation turns out to be needed on a hot path.** The same answer
RFC 0014 gives: that is evidence the ring is not carrying something it should,
not a reason to make the door faster. The one plausible candidate is mapping —
a component that maps and unmaps per request — and the answer there is a ring
opcode, which is what M5 makes it anyway.

**`CAP_MAP` cannot be expressed as an `Sqe`.** It packs four values into two
registers today, which an `Sqe` has room for several times over. If it turns out
the entry cannot carry two capability handles — one field is `cap`, and the
second would have to live in `ext` — then either `Sqe` is wrong or mapping into
somebody else's address space belongs somewhere other than a ring, and both are
worth knowing before E1 builds on the assumption.

**A copy needs to survive its source being revoked.** The broker case: a
component holding capabilities on behalf of others, where the holder's ability to
withdraw is the bug rather than the feature. That would make a copy a sibling
after all, and the change is one line in `Table::derive` plus a `Property::Stale`
that no longer expects it.

**The four are still here after M5.** The same reversal RFC 0014 sets for its
three, and the same remedy: the rule stops being a document and becomes a lint.
