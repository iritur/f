# RFC 0047: A driver is scheduled, and asks the frame for a translation

- Status: accepted
- Date: 2026-09-03
- Affects: `kernel/src/blk.rs`, `kernel/src/process.rs`, `kernel/src/smp.rs`,
  `kernel/src/component.rs`, `kernel/linker.ld`,
  `kernel/src/arch/x86_64/paging.rs`, `abi/src/control.rs`, `ring/src/device.rs`,
  `user/virtio-blk/`, `xtask` (`MINTS`, `NOT_THE_FRAME`, `IMAGE_MAX`,
  `CHAOS_GAP`), RFC 0033's reversal condition, RFC 0044's *not claimed*
  paragraph, E1-B02, E1-B08, E1-P06

## Decision

**`user/virtio-blk` runs at ring 3, on a core the frame allocated it, in its own
polling loop — and the one thing it cannot do for itself, it asks the frame for
on its control ring.** `kernel/src/blk.rs` keeps the supervisor's half and calls
no part of the driver. RFC 0033 wrote its own reversal as a grep anybody can
run — *see which crate calls `Driver::execute`* — and the answer is now
`user/virtio-blk` and nowhere else, which `cargo xtask lint-datapath` checks
rather than leaving to a reader.

Four things had to be true and none of them was:

1. **A component's text may be more than one page.** Every process this kernel
   had ever built was one page and the build refused an image that was not. A
   driver with its own transport, queue and registration table is thirteen
   kibibytes. `process::TEXT_PAGES` is the reservation, `component::spawn`
   charges the account for the pages an image actually occupies, and `xtask`'s
   `IMAGE_MAX` is the same bound stated where the build can refuse it.
2. **A device's registers are mapped into the component, uncached.**
   `paging::UserPage::Device` is `Data` plus the two cache bits the frame's own
   device mappings already use. `Data` would have been wrong in the way that is
   invisible under an emulator and fatal on a machine.
3. **A driver asks the frame for a device translation.**
   `control::op::DEVICE_MAP` and `DEVICE_UNMAP`, on the control ring the
   component already holds. The frame answers from a polling loop on the boot
   processor — `smp::join_serviced` — which is where the remapping unit, the
   allocator and the *client's* capability table already are.
4. **The frame tells the component where everything landed.** One page,
   `f_virtio_blk::routing`, whose address is the single constant both sides
   hold; the kernel asserts the two definitions agree at compile time. The
   component writes its own counters into the far half of the same page, which
   is RFC 0013's *read, never delivered*.

## Context

RFC 0044 built a place per component file and stated plainly what it had not
done: *that `virtio-blk` runs. It is spawned into a place and it is not
scheduled.* It named the two blockers — a component image larger than one text
page, and a route by which a component asks the frame for a device translation —
and said neither was its own. This is both of them.

The wall was never the driver's code. `user/virtio-blk` has driven a real device
through real registers since E1-B02; what it could not do was *be the thing
running*. E1-B08 landed the mechanism for that (`kernel/src/runtime.rs`: a
component holds a core, schedules inside it, and crosses no boundary), RFC 0037
landed safe channel adoption, and RFC 0033 landed safe device accessors. Three
pieces, each built for this, and nothing had put them together.

The alternatives that were live:

- **Pre-compute the translations and hand the driver a table.** The frame knows
  which capability the client will register and what the answer will be, so it
  could publish the answers on the board and let the driver look them up. The
  authority check would even be real — the same `iommu::Grant`, the same
  client's table, the same refusal. It was rejected because the *timing* would
  be a fiction: the driver would not be asking, and a reader taking *the driver
  asked for a translation and was refused* from a run where the frame had
  decided both in advance would be taking the fourth false pass of this epoch.
  A component that cannot ask is a component the topology cannot change under.
- **Answer the driver from the timer handler on its own core.** The frame
  reaches a core it has given away in exactly one way and `runtime::on_ring3_tick`
  already uses it. It would work, and it would mean handing the allocator, the
  remapping unit and a client's capability table across a core boundary through
  raw pointers, to be used inside an interrupt gate. The boot processor is
  already spinning; making it spin *usefully* costs one function and moves no
  authority anywhere.
- **Give the driver a capability to the remapping unit.** This is the one that
  has to be refused out loud. A component that can program an IOMMU can point
  any device at any memory, which is the whole of what the device isolation in
  this system is. The asymmetry is the design: a driver holds its device and
  does not hold the unit its device is behind.

Two things were found rather than designed, and both are recorded because they
were expensive:

- **The escape provocation must bend exactly one entry.** When the component
  applied the displacement on every entry of its `escape` life, the *write*
  escaped too — so the sector was never put on the disk, and the read-back
  compared against memory nothing had written. The run went green, the fault
  was recorded at the right address, and the positive control had been
  destroyed. `blk=escape` bends the read and only the read, which is what the
  frame did when the frame was the caller.
- **The boot processor's kernel stack was sixty-four kibibytes and had been
  since M0.** A second place with a four-page image drove it into its guard
  page. The double fault says nothing about where it came from; the fix is a
  number in `kernel/linker.ld` and the reason is that `component::demonstrate`
  holds a `Supply` and an `Instance` *per place* on that stack, because a
  supervisor is not a component yet.

## Consequences

**Easy.** The claim `blk/copies = 0` stops resting on a source check. It used to
be enforced by `MINTS` in `xtask` — no shipped line of the driver may mint a
`Region` or a `Window` over a bare address — because the driver ran in the frame
and the frame's direct map covers all of physical memory, so a safe `const fn`
constructor was a way to read a client's bytes while `stage` stayed honest. The
driver runs at ring 3 now. The pages mapped for it are its text, its stack, its
two rings, its board, its device's registers and its own queue memory; an
address it invents is a page fault. `MINTS` is empty and the scan behind it is
kept working against a list its own test supplies, because a mechanism with
nothing to find is indistinguishable from one that cannot find anything.

**And the check that replaced it needed the same argument turned on itself.**
`NOT_THE_FRAME` is RFC 0033's reversal grep made executable — no line under
`kernel/` may name `Driver::` — and as first written it looked only for that
string's *absence*. A search for an absent name is satisfied by a name that
refers to nothing: rename `Driver` in `user/virtio-blk`, or add a second driver
crate whose type is called anything else, and the lint stays green over a frame
running a component's code, with the direct map back under a crate whose
`copies = 0` this tree publishes as a property. So the row carries a third
field — the prefix that *must* name it — and the rule fails when nothing does.
That also corrects what the row said about growing: a needle spelled after a
type is per-crate, so `E1-B03` and `E1-B04` each add one.

**Also easy, and it is the point of the third item above.** Every refusal the
datapath demonstrates is now demonstrated *across a privilege boundary*. The
registration of a capability with no `GRANT` is refused by the frame in answer
to a request a ring-3 component submitted; the descriptor that points past a
grant is arithmetic performed at ring 3; the withdrawal under a live
registration happens to a driver that is running. `kernel/src/blk.rs`'s module
comment used to end its `escape` paragraph with *what neither shows is that the
code doing the reaching runs at ring 3*. It does.

**Hard.** There are two polling loops now and they make progress against each
other. The client is the frame on the boot processor and the server is a
component on another core; the client's every wait serves the driver's control
ring, and the driver's every wait for a translation drains its own. Both waits
are bounded — five seconds each, the same bound `run_on` already used — and a
bound is what a wedge produces instead of a hang. It is not a check, and the boot
log says so on its own line: a bound scaled off `tsc_khz` fires for a component
that is stuck and for a machine slower than the number, and those two cannot be
told apart from inside the frame. Every other arm of `blk::Trouble` is something
the frame *observed* going wrong, so a red line on one of those means a
protection fired; `Trouble::bound` is what separates the two, `smp::NotJoined`
is where a core that answered wrongly is kept apart from a core that answered
nothing, and the reason both exist is that a wall-clock red read as a datapath
defect and a real wedge dismissed as a slow runner are the same mistake pointing
opposite ways. It also means `cargo xtask
blk` requires a second core and says so rather than falling back: a driver and
its client cannot be the same core, because the client would be inside the
driver.

**Foreclosed, and this is the honest cost.** The instance that serves the
datapath is *scheduled* and not *spawned into a place*. `kernel/src/component.rs`
builds a place for this manifest on every boot — an account, needs checked
handle by handle, an endpoint clients hold, a restart policy — and never hands
its occupant a core; `kernel/src/blk.rs` hands a core to an instance that is in
no place, exactly as `kernel/src/runtime.rs` does. So *the datapath is served by
a scheduled component* is true and *the occupant of a place serves the datapath*
is not, and the two are one word apart. `CHAOS_GAP` in `xtask` is shrunk to
exactly that difference and to nothing wider; RFC 0041's gap section is what it
narrows.

**A declared gap now names the documents that describe it.** This change closed
half of `CHAOS_GAP` and paid RFC 0033's reversal in full; the two constants and
`sim/src/chaos.rs`'s module comment were updated and five other live documents
were not — three `TODO.md` entries, RFC 0041's gap section, and
`claims/0006`'s notes — each of which went on describing a tree that had gone.
`gap_holds`'s refusal had said *every document that describes the same
deviation ... update them* and named none of them, which is an instruction that
assumes the reader already knows the answer. Each row of `OWED_REVERSALS` and
`CHAOS_GAP` therefore carries that list, and the build prints it at the moment
the gap closes. It is a list of documents rather than paths a lint reads,
because half of them are `TODO.md` and a check that refused a stale sentence in
a file agents may not edit would be a check that has to be switched off.

**Not claimed.** That the driver was *spawned* with what it uses. Its registers,
its queue memory and its rings are mapped by the frame from what the manifest
declares — the same declaration `component::spawn` checks handle by handle for
the place — but this path checks no needs and charges no account, because there
is no supervisor to check them against. Joining the two is one act: a supervisor
that spawns and then schedules. That is E1-B05's remaining half, and it is the
same half that owes RFC 0008 its restart policy.

## What would reverse this

**A supervisor that is a component.** The moment one exists, the frame stops
answering `DEVICE_MAP` from a loop on the boot processor and starts forwarding
it — or, better, stops seeing it at all, because a supervisor holds the domain
its child's device is attached to. `Supervising` in `kernel/src/blk.rs` is one
struct for that reason: it is the list of what a supervisor holds and a driver
does not, and when a supervisor exists that list moves rather than being
rewritten.

**A device whose registers are not one window.** The four structures are
narrowed out of a single mapped span with `Window::slice`, which only ever goes
inwards, so a driver cannot widen its way out of what it declared. A device that
put its structures in two different base-address registers would need two spans,
and at that point the routing page carries two bases and the manifest declares
two windows — which is a manifest change and a review, not a bigger number here.

**A component that needs its own address for something the frame did not map.**
The board is a layout and not an allocator: the component is told where four
things are and may not ask for a fifth. The day a driver needs memory it was not
routed at spawn is the day this stops being a page of offsets and becomes a
retype operation on the control ring — which is `op::MAP`, already named, still
unimplemented.

**An image that outgrows the reservation.** `TEXT_PAGES` is sixteen and
`user/virtio-blk` is four. Seventeen is not a bigger constant: it is the point at
which a flat image copied to a fixed address stops being the right shape, and E5's
loader — one that reads a component's headers and maps what they ask for — is
what replaces every address in `kernel/src/process.rs`'s layout section, not just
that one.

**A gap row that outlives its subject.** Both mechanisms this leans on are
name-based and both have the same failure: `NOT_THE_FRAME`'s needle stops
naming anything, or a `Gap` row's needle survives a rewrite that changed what it
meant. The presence half answers the first. Nothing answers the second, and the
honest statement of it is that a row is worth what its `why` is worth: if
`prepare_driver` is ever renamed while the driver is still scheduled outside a
place, `CHAOS_GAP` goes red for the wrong reason, and the right response is to
re-point the needle rather than to widen the constant.

**`MINTS` going back.** It is empty because the address space is the
enforcement. If a component's code is ever linked into the frame again — for a
test harness, for a fallback, for anything — the direct map is under it again
and the scan is what was holding. The list is empty; the machinery is not.
