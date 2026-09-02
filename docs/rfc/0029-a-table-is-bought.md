# RFC 0029: A capability table is bought, a page at a time, by whoever holds it

- Status: accepted
- Date: 2026-09-03
- Affects: `kernel/src/cap.rs` (the storage, the revocation walk, the property
  suite), `kernel/src/process.rs` (the derive that can spend, the revoke that
  is now three steps, two more provocations), `kernel/src/arch/x86_64/probe.rs`
  (`cap=quota` and `cap=beyond`), `kernel/src/main.rs` (the self-test needs the
  allocator), `xtask/src/main.rs` (`ESCAPES`, now eleven); `abi/` **not at
  all**, which is the part of this RFC most worth reading; RFC 0008, whose
  E1-B13 paragraph this implements and whose cross-table parent link it
  deliberately does not; `TODO.md` E1-B13, and `E1-B05`, which this hands two
  named decisions

## Decision

The capability table stops being a fixed array and becomes storage the
component pays for out of its own `Untyped`, exactly as RFC 0008 said everything
a component is made of would be. Five things are decided here that RFC 0008 left
to whoever built it:

1. **The request is a derive with nowhere to go.** There is no grow call. When
   `Table::place` finds no free slot, the table looks for the lowest-indexed
   `Untyped` it holds that carries `rights::DERIVE` and has a frame left,
   advances that region's watermark by one frame, writes a page of empty slots
   into it, and places the child. A component that cannot pay is refused
   `RESOURCE/QUOTA_EXHAUSTED`. RFC 0008 shrinks the door to `EXIT` and the
   doorbell; an explicit grow would have been a fifth capability call at the
   door on the day the other four are being retired, which is the wrong
   direction by a whole RFC. An explicit grow opcode on the control ring stays
   available to E1-B05 and this decision does not foreclose it — it makes it
   optional rather than load-bearing.

2. **Only a derive grows a table; a grant never does.** A grant is authority
   arriving from outside, at a moment when there may be no component yet to
   charge, and a grant that could buy a page on the frame's say-so is the kernel
   reserve this whole change exists to not have. `Table::grant` still refuses
   with `RESOURCE/QUOTA_EXHAUSTED` when the table is full.

3. **The generation floor crosses the boundary that ends a process.** A bought
   page is dropped when the component that paid for it ends, and the next
   component buys *different memory* for the same slot indices. A page whose
   slots started at the first generation would therefore hand the next occupant
   of a core a handle the last one is still holding — which is precisely the
   failure E0-B10 found at the free part's boundary and `Table::clear_all`
   already refuses to have. So a table carries a `floor`: the generation a newly
   bought slot's first occupant is issued at, raised on every `clear_all` to the
   highest generation any bought slot reached, and saturating for the same
   reason a slot's own generation saturates.

4. **The revocation walk stays iterative and stays bounded; the marks move to
   the caller's stack.** They were a `u32`, one bit per slot, because a table was
   thirty-two slots. They are now a `Condemned` bitmap sized by `MAX_SLOTS`,
   passed between the three steps of a revocation and owned by neither the
   table nor anything beside it. And a revocation is three steps —
   `condemn`, `next_mapping`, `sweep` — rather than one, because the addresses
   a revocation withdraws are no longer bounded by anything that fits in a
   return value.

5. **The ceiling is `MAX_PAGES` and it is stated as a cost.** A table may buy
   four pages. The number comes from the walk being quadratic in what a
   component chose to spend, not from memory. A component that reaches it is
   refused with the same code as a component that cannot pay, which is a
   conflation and is written down as one in `cap.rs`.

The wire format did not change and did not need to. `Handle` carries a
sixteen-bit index, which addresses 65 536 slots; `MAX_SLOTS` in this build is
544, and a compile-time assertion in `cap.rs` fails the build the day those two
cross. Nothing in `abi/` moved, no peer's assumption about handle packing
changed, and `error::resource::QUOTA_EXHAUSTED` already existed in the domain
RFC 0010 puts it in — it has been the refusal for a full table since M4. The
task that would have widened the index is a different and much more expensive
task, and it is worth saying plainly that this was not it.

## Context

`kernel/src/cap.rs` has carried its own reversal condition since M4: *a
component that legitimately holds more than `TABLE_SLOTS`, which is E1's first
real supervisor*. RFC 0008 made that concrete — a supervisor holds an endpoint
per place, a control-ring channel per child and an `Untyped` per child, and
thirty-two runs out at about eight children — and named E1-B13 as the task that
would build it. What RFC 0008 fixed was the *shape*: the table is retyped from a
supplied `Untyped`, revoking a supervisor's `Untyped` ends every component it
paid for, and a component that cannot pay is refused rather than served.

Three alternatives were live.

**A larger array.** Two hundred and fifty-six slots would have carried a
supervisor and cost nothing to write. It is the wrong answer for a reason that
has nothing to do with memory: a fixed array is a bound the *kernel* chose, so
running out is a fact about the build rather than about the component, and two
components with different appetites share one number. `Untyped` accounting makes
the bound a property of what a component was handed, which is what makes the
refusal local — and locality is the precondition
`docs/design/deadline-all-the-way-down.html` section 03 names for a simulation
that reproduces. A shared bound, however large, is a shared fate.

**One contiguous region, relocated on growth.** Copy the free part into a
larger region bought from the `Untyped` and keep one flat array. It keeps
indexing trivial and the walk unchanged, and it needs contiguous physical memory
of a size that doubles — which is an allocator promise this kernel does not
make today, and which E1-B12 is separately rewriting. The chain of frame-sized
pages needs no contiguity, no copy, and no allocator call at all: the memory is
the untyped region's own, reached through the direct map.

**An explicit grow opcode.** Rejected above, and worth naming the thing that
makes it tempting: an explicit call would let a component pre-buy, which a
latency-sensitive component would want rather than discovering the cost inside
a derive. That is a real argument and it is E1-B05's to make on the control
ring, where it costs no door call.

## Consequences

**A component cannot map its own capability table**, and this is a structural
consequence rather than a check. Growth advances the untyped region's watermark
exactly as a retype does, so the frame that becomes table storage is a frame no
later retype can hand out. There is no `Frame` capability naming it, so there is
nothing to present to a map call.

**`cap=flood` and `cap=quota` differ by a number, and the number is the
evidence.** Both make the same calls; one has spent its untyped region first.
The flooding process is answered 155 times and holds 160 capabilities; the
spent one is answered 27 times and holds 32. A table that had quietly stopped
growing, or one that grew out of something nobody was charged for, would show up
as the wrong count in a boot log rather than as a subtlety somebody had to
notice.

**The property suite needs the frame allocator**, because a table that has been
paid for is a different table and the only honest way to have one is to pay. It
takes two frames, runs the five properties and the five flawed fixtures at both
sizes — twenty checks where there were ten — and gives the frames back. A
fixture with a pretend account would have been testing a second path.

**One fixture had to change, and the change is a finding.** `Flaw::MasksTheIndex`
masked an index into `TABLE_SLOTS`, which is correct only while a table is a
power of two. On a bought table that mask aliases bought slots onto free ones,
so it resolves an *in-range* forged handle and is caught by `Property::Forged`
rather than by `Property::Total` — a fixture caught by the wrong check, which
this suite treats as a failure precisely so that it gets noticed. It is now
`Flaw::ForcesIntoRange` and forces against the table's own size, which is the
general form of the same mistake and is visible only out of range.

**Revocation got more expensive in the worst case**, from about a thousand
iterations to about three hundred thousand for a table at the ceiling. Bounded,
iterative, no recursion — but quadratic in a number a component chooses by
spending, which is why `MAX_PAGES` is four and not forty.

**What this hands E1-B05, named so that neither has to re-decide.** Two things:

- *The parent link stays inside one table.* RFC 0008 says the link must be able
  to name a slot in another table so the derivation tree spans components and
  revoke-on-death crosses them. It does not do that here, and the reason is
  that there is nothing yet to name: a cross-table link needs a component
  identity, and the frame has none until E1-B05 creates one at spawn. Inventing
  one now would be inventing the thing E1-B05 replaces. What E1-B05 must add is
  a second word in the slot — `Slot::parent` is a `u32` handle and there is no
  room in it — carrying the owning component, `Handle::NULL` meaning *this
  table*; a `descendants` walk that visits other components' tables when it
  meets a foreign link; and, because a table lives in a `PerCpu` static, an
  argument for how a walk reaches a table on another core. That last one is a
  fifth word crossing a core boundary under RFC 0016 and needs its own RFC, not
  an allow-list entry.
- *A grant into a full table.* RFC 0008 has the frame placing capabilities into
  a running component's table — a powerbox grant, a spawn's needs — and decision
  2 above means those fail rather than grow. The answer is either that the
  granting supervisor pays out of the `Untyped` it is already spending, or that
  the grant is refused and the supervisor grows the child first. E1-B05 chooses,
  because E1-B05 is the first task with a second component to choose for.

**What this does not build, which RFC 0008 also assigns to E1-B13.** The
three-bit pending-notice field in each slot, the *no refill while not quiet*
rule, the stop word and the two grade words. All four are protocol state whose
only consumer is the notice delivery E1-B05 builds, and none of them is
observable — or falsifiable — before there is a control ring to drain. Building
them now would ship four fields nothing reads and no boot can break, which is
the opposite of how everything else in this tree is evidenced. They are E1-B05's
to land with the ring, and the slot has room.

## What would reverse this

**A component that reaches `MAX_PAGES` with an account still in credit.** That
is the ceiling becoming a real refusal rather than a theoretical one, and at
that point the conflation in decision 5 has to end: *your account is empty* and
*this build will not grow a table further* need separate codes in
`error::RESOURCE`, because they have different recoveries. The walk has to stop
being quadratic first — a child list, or an order that lets a revocation visit
each slot once — because raising the ceiling without that is buying iterations.

**A supervisor whose steady-state table churns.** Growth here is one-way: a
table never gives a page back until the component ends, so a supervisor that
spawns and reaps a thousand children over a day holds its high-water mark
forever. That is the right trade while a table is bounded by four pages and a
component's life is a boot; it stops being right when the high-water mark is far
above the steady state, and the answer then is releasing an empty trailing page
back to the account — which needs the account to accept a return, which
`Untyped`'s watermark does not.

**A measurement that says the derive which buys a page is a latency outlier
somebody cares about.** Growth costs a page of stores — 128 slots — inside one
derive, and a component with a deadline that lands on that derive pays it. If
`E1-P10`'s numbers show it, the answer is the explicit pre-buy on the control
ring that decision 1 left available, not moving the purchase somewhere it cannot
be refused.

**Anything that makes the sixteen-bit handle index tight.** `MAX_SLOTS` is 544
against 65 536 and the assertion in `cap.rs` is what would fail first. If a
component ever legitimately wants tens of thousands of capabilities, the index
width is an ABI change and every peer's assumption, and it is a different RFC
with a different cost.
