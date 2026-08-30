---
id: 0003
status: agreed
reviewed_by: Dmitri Chudinov
skills: frame-and-unsafe, determinism-review, rfc-author
---

# Spec: a capability table, and the suite that says it holds

A process holds a table of typed slots. It may look at one, mint a weaker one or
a copy, withdraw everything below one, and map a frame it holds into an address
space it holds. Everything else it tries is refused, and the refusal names which
of four things went wrong. The frame counts what it answered, and a boot where
the count is not exactly right fails.

## Behaviour

**The handle.** Sixteen bits of slot index and sixteen of generation, in the
`u32` that `Sqe::cap` already is. Generations count from one, so a zeroed word
names nothing — which matters because `Sqe::ZERO` has `cap: 0` and a submission
that was memset must not carry authority over slot zero.

The generation is what makes a *stale* handle detectable, and it is not what
makes a forged one fail. A forged handle fails because the index has to name a
slot the frame filled for that process, and there is no global capability space
in which to guess a name. Stating it the other way round would make this a
password system wearing a capability system's vocabulary, and `abi/src/cap.rs`
says so where somebody would otherwise assume it.

A slot that has been through 65 535 capabilities is **retired** rather than
wrapped. A generation that wraps is a handle held since before the wrap becoming
valid again; retiring converts that into a table one slot smaller, which is an
error a component can be told about.

**The types.** Six: `Untyped`, `Frame`, `AddressSpace`, `Channel`, `Endpoint`,
`Irq`. Three of them have no object behind them at M4, and each says which
milestone gives it one — `Channel` at M5, `Endpoint` at E1-D01, `Irq` when a
driver lives outside the kernel. They exist now because the table's *shape* is
the expensive thing to change once two peers exist, and all six are exercised in
the boot suite even where only three are ever held by a process.

**The rights.** Six bits — read, write, execute, derive, revoke, grant — and the
only legal operation on them is narrowing. There is no call that adds a right
and none that asks the frame for one. `execute` is separate from `read` because
write-exclusive-or-execute is a rule about mappings and a bitmap that cannot
express it cannot enforce it.

**The derivation tree.** A slot records the handle it was derived from. A
handle, not a pointer and not a child list: it carries a generation, so a parent
link into a slot that has since been refilled reads as broken rather than
silently naming the new occupant. Revocation walks the table marking descendants
in a bitmask until a pass marks nothing — bounded, quadratic in a table of
thirty-two, and with no recursion in it, because a recursive revoke is a stack
depth chosen by whoever built the tree.

**A copy is a child.** There is no copy operation: a copy is a derivation with
the rights the capability already carries, which puts it *below* its source in
the tree. seL4 puts it beside. The corpus lists "nothing can be revoked" as a
structural drawback and answers it with recursive revocation, and a revoke a copy
escapes does not answer it — the authority is still out there and the log says it
was withdrawn. The cost is that two holders of equal authority are not equal, and
it is stated rather than hidden.

**Retyping.** Deriving from an `Untyped` mints a `Frame` naming the next
unclaimed frame of the region and advances a watermark. There is no way to copy
an untyped capability at M4; the operation that separates the two takes a target
type and a sub-range as operands and belongs on a ring.

**The four calls.** `CAP_INSPECT`, `CAP_DERIVE`, `CAP_REVOKE`, `CAP_MAP`, and
RFC 0015 is the argument for why four calls exist behind a door RFC 0014 says
does not accumulate an interface. In short: a ring is named by a `Channel`
capability, so the table has to work before there is a ring to work it through;
rule 2 of RFC 0014 governs calls added before the ring, and each of these four
names the opcode that retires it at M5.

**Mapping.** `CAP_MAP` takes a frame capability, an address space capability, an
address and the rights the mapping is to carry. The checks are in a fixed order —
authority, then argument, then page tables — because a frame that checked the
address first would refuse an overlapping mapping whether or not the caller was
entitled to make it, and the negative suite would pass while proving nothing.

It allocates nothing. Every page table on the path has to exist already, and an
absent one is a refusal rather than a frame spent on a process's say-so. Two
things follow and both are the point: the free count still has to come back
exactly, and a process cannot enlarge its own address space by asking. The
account that would let it is `Untyped`'s, and E1 is where it arrives.

**What a process starts with.** Three capabilities: its address space, one frame,
and one untyped region. The frame capability deliberately carries no write right,
which is the whole of the rights half of the suite — a process that could map it
writable would have exceeded what it was granted. There is no capability for the
page the process is executing out of, because a process that could remap its own
text is one for which write-exclusive-or-execute is advisory.

**The preamble.** Every process, whatever it was told to provoke, first uses its
capabilities correctly: inspect one, derive a copy, map the copy, touch the page.
Three answered calls on every boot. A suite made only of refusals passes against
a frame that refuses everything, and this is what stops that.

**The seven escapes**, chosen by `cap=` on the command line, one of which must
succeed and six of which must be refused:

| `cap=` | what the process does | what must happen |
| --- | --- | --- |
| `grant` | derives weaker, reads it back | nothing refused |
| `unowned` | names a slot in range the frame never filled | twice: no such capability |
| `forge` | every slot × four generations, then four impossible words | 132 attempts, 4 resolve |
| `stale` | derives twice, revokes the root, uses both leaves | twice: revoked |
| `rights` | derives wider, then maps writable without the right | twice: right not held |
| `type` | a space where a frame belongs, and the reverse | twice: wrong type |
| `flood` | derives until the table is full | 28 mints, then a resource error |

The frame is the judge, not the process — a compromised process would lie. It
counts answers by refusal code and compares against an exact expectation, so a
run refused the right number of times for the wrong reasons is a failed boot.

**The five properties, and five tables broken on purpose.** The suite is written
once as a function over a trait, run at boot against a real table and against
five flawed ones — a lookup that masks the index instead of checking it, one that
skips the generation, one that answers for empty slots, a revoke that stops at
the children, and a derive that lets rights widen. Each must be caught by the
property it breaks and by that property alone, and the boot prints how many were
caught. A checker nobody has watched fail is a checker nobody has tested.

The flawed tables ship in the kernel image, because the kernel has no host test
harness and a fixture under `cfg(test)` would never run. They are constructed in
one function and nothing in ring 3 can reach one. That is the status the hostile
ring-3 program and the `fault=` provocations already have: this tree ships its
adversaries.

## Policy applied

**The frame (RFC 0001).** All of the table is `kernel/`, and none of it needs
`unsafe` — it is an array and some arithmetic. The `unsafe` this change does add
is three obligations: a `&mut` to this core's table taken where a system call
cannot be interleaved, a mapping written into an address space that is live, and
a shared reborrow of the caller's frame allocator held for exactly as long as
the borrow it came from is dormant. Each is discharged where it is taken.

**Determinism (RFC 0004).** Everything the boot log gains is a fixed number: how
many capabilities were granted, how many calls were answered, how many refused,
how many flawed tables were caught. The free slot chosen by a grant is the lowest
one rather than the next one, so the same sequence of operations produces the
same handles on every run. `DETERMINISM_ALLOW` does not grow. Two runs of one
commit are still byte-identical, which was checked rather than assumed.

**Per-CPU state.** The table is one `PerCpu<Table>`, for the same reason the
process's state is: a process runs on one core and its calls arrive on that one.
The allocator's address is carried in the process's existing state as a `usize`
rather than a pointer, because `PerCpu` is `Sync` only for a `Send` payload and
that bound is the right one — a pointer there would be asking for an exception to
it rather than needing one.

**Reversals need RFCs.** One is owed and written: RFC 0015, on why four calls
appear behind a door that exists to stop calls appearing. It records the reading
that RFC 0014's rule 1 governs permanent calls and rule 2 governs bridging ones,
and names the opcode that retires each of the four.

**Numbers need claims.** None are published. Nothing here is a measurement.

## Not in scope

- **A second process, or a second table.** One `PerCpu<Table>` per core and one
  process at a time. The isolation property between two tables is checked in the
  boot suite with two tables in memory at once; it is not checked between two
  *running* processes, because there is still only one. E0-B10.
- **Transferring a capability to another component.** The `GRANT` right exists
  and nothing exercises it, because there is nobody to grant to. E1-D01.
- **A quota.** `Untyped` retypes into frames and nothing charges anything for a
  page table. `CAP_MAP` refusing to allocate is the honest form of that gap: the
  process cannot spend what nobody is accounting for.
- **Unmapping, and revocation reaching a mapping.** Revoking a frame capability
  that has already been mapped withdraws the capability and leaves the mapping.
  That is wrong in the long run and is named as a risk below rather than fixed
  here: undoing it needs shootdown, which needs the second core.
- **Capabilities for channels, endpoints or interrupts.** The types exist and are
  exercised; the objects do not.
- **Mutation fixtures for property five.** Four of the five properties have a
  flawed table that breaks them at runtime. The fifth cannot: a fixture that
  panics takes the machine down rather than being caught. What it has instead is
  a compile-time half — the module denies indexing, `unwrap` and `panic`
  outright, so the constructs that turn a hostile handle into a fault cannot be
  written — and a runtime half that catches the masked-index form. E0-P08 is
  where the remaining gap is recorded.

## Evidence

- `cargo xtask run` — the boot prints `capabilities 32 slots, 5 properties hold,
  5 flawed tables caught`, and the process that follows grants three
  capabilities, answers three calls, refuses none, and gives every frame back.
- `cargo xtask cap` — seven boots, one per escape. Six must be refused, with the
  exact refusal codes the frame expects; the seventh must not be, which is what
  stops the other six passing for the wrong reason.
- `cargo xtask user` — the seven M3 provocations still hold, now with the
  capability preamble running inside each of them.
- `cargo test -p f-abi` — the handle packing, the rights arithmetic and the four
  authority codes, on the host.
- Two runs of one commit are byte-identical.

## Risks and reversal

**A revoked frame capability does not unmap the frame.** The capability is gone
and the page is still there. It is the largest gap in this change and it is
stated rather than papered over: revocation withdraws a name, and undoing the
mapping that name authorised needs an unmap, which needs a shootdown, which needs
the second core. *What would reverse this:* the second process, at which point a
revoke that leaves a mapping is a revoke that leaves one process reading
another's memory.

**The table is a fixed thirty-two slots in a per-core static.** Fine for one
process; wrong for a component that legitimately holds more. *What would reverse
this:* E1's first supervisor, at which point the table becomes an object the
`Untyped` capability pays for — which is the same change as giving a process a
quota.

**A process may only map where its address space already has a page table.**
Today that is one two-mebibyte region and it is enough. *What would reverse
this:* a component with a real address space, which is E0-B10 loading `init` from
a boot module.

**The flawed tables are in the shipped image.** They are unreachable from ring 3
and constructed in one function, and the alternative is fixtures that never run.
*What would reverse this:* a host test harness for kernel logic, which would mean
the table moving out of the kernel crate — and that is a bigger decision than
this change, because it splits the frame across crates.
