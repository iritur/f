# RFC 0044: A place per component file, and an account the manifest sizes

- Status: accepted
- Date: 2026-09-03
- Affects: `kernel/src/component.rs`, `kernel/src/main.rs`, `xtask`
  (`JOIN_GAP`, `sim_join`, its tests), `docs/rfc/0036`'s declared gap,
  `docs/rfc/0029`'s open question about a grant into a full table, E1-B05,
  E1-P01

## Decision

**The frame fills a place for every component file the loader carries, and each
place's account is sized from that component's own manifest rather than from a
constant.** Three consequences follow and each is a decision somebody could
disagree with, so each is stated:

1. An account holds the manifest's declared footprint **plus** what its declared
   needs are carved out of, rounded up to a buddy order. `memory_bytes` is the
   footprint and says so; the routed needs come out of the same region in this
   build because the supervisor is the frame and there is nowhere else for them
   to come from.
2. **A grant into a full capability table is refused, and the holder buys the
   page itself out of its own account.** `kernel/src/cap.rs` named this as the
   choice E1-B05 had to make and declined to make it, because there was no
   second component to make it for. There is one now.
3. A need whose type this build has no object for is satisfied with a capability
   of the declared type carrying the declared rights and naming **nothing**, and
   the count of those reaches the boot log as `Report::unbound`. Today that is
   exactly the `irq` need in `user/virtio-blk/manifest.toml`.

`JOIN_GAP` in `xtask` becomes empty, and its emptiness is the evidence RFC 0036
said it would be.

## Context

RFC 0036 found that the two halves of `boot-to-workload` were about two
different component sets — `{store}` and `{store, virtio-blk}` — while the
artefact header claimed coverage of both, and made the difference a *declared
set* required to match exactly. The declaration was one name, and the task that
would remove it was named: a boot that spawns the whole module set.

The reason the boot did not was recorded as one thing and turned out to be two.
The recorded reason was that `kernel/src/component.rs` built a place from
`*modules.first()`. The unrecorded one is the more interesting: **the account was
a constant.** `ACCOUNT_ORDER` was five — a hundred and twenty-eight kibibytes —
and `user/virtio-blk/manifest.toml` declares two mebibytes, so a second place
could not have been admitted even if a second place had been built. E1-B02's own
note records exactly that refusal, `ADMISSION/MEMORY`, and reads as a fact about
the driver; it was a fact about a number in the frame.

Two smaller things were load-bearing and neither was visible until an account was
sized exactly rather than generously:

- **`admit` runs twice, and the second call was passing on slack.** The
  supervisor tests whether it can afford to ask; the frame tests what it was
  asked, inside `spawn`, from a table it does not trust. The second runs *after*
  the offer has carved the needs out of the account, so what it compares is the
  remaining extent against the declared footprint. Under a constant with a
  hundred kibibytes of headroom that comparison was true the way a stopped
  clock is. With an exact account it fails, which is how the missing term in
  the sum was found rather than argued about.
- **`CHARGED_MAX` was `1 << ACCOUNT_ORDER`**, which said something true while
  every account was one size. Sized accounts made it say that a `Supply` and an
  `Instance` must each be five hundred and twelve handles wide, on a boot
  processor's stack, once per place, to hold the twenty-three the largest
  component in this tree actually charges.

The alternatives that were live:

- **Leave the constant and enlarge it.** One order for the largest manifest in
  the tree, which is a number that has to be edited every time a manifest grows
  and whose failure mode is a refusal that reads as being about the component.
  It also keeps the second `admit` decorative.
- **Charge the needs to something other than the place's account.** Correct for
  a real topology and premature here: `from` in a manifest is what says where a
  need is routed from, and until there is a second supervisor to route from,
  inventing a second source would be inventing topology in the frame.
- **Have the frame pay for a full table out of the account it is spending on
  the spawn.** This is `cap.rs`'s first option, and it is the kernel reserve RFC
  0008 refuses to have, reached from the other side: the frame spending
  somebody else's account on the frame's own say-so, at a moment the component
  being granted to may not exist yet.

## Consequences

**Easy.** Adding a component file is now adding a component file. The loader
carries it, the frame gives it a place, the account comes from its manifest, and
`cargo xtask sim --join` compares the set the boot spawned against the set the
simulator runs with nothing in between. A component whose manifest declares more
memory is admitted or refused on the strength of what it declared, and the
admission line prints the footprint, the needs and the account side by side so
the slack a buddy order introduces is a number a reader can see rather than one
only `account_order` knew.

**Hard.** A place is still the frame's, and every place after the first is
*filled* rather than *scripted*: it is spawned, published to, held and torn
down, and it is not killed, restarted or retired. That is deliberate — running
the same three branches against a second occupant exercises the same code — but
it means the boot's evidence about places two and up is *instantiation* and not
*lifecycle*, and no artefact here should say otherwise.

**Foreclosed, and it is the honest cost.** The `irq` need is met with a
capability naming no vector, because nothing in this build routes a device
interrupt to a component. The spawn is real — real record, real account, real
address space, real table, real control ring, and the type, rights and quantity
checks are the same five `probe_refusals` provokes on every boot — and the
object behind that one handle is not. A reader who took *the needs were checked*
for *the needs were met* would be taking the fourth false pass of this epoch,
so it is counted rather than described: `Report::unbound`, printed on the
supervisor's summary line, one today. E1-B09 is what makes it zero.

**Not claimed, and this is the one to read.** That `virtio-blk` *runs*. It is
spawned into a place and it is not scheduled: `kernel/src/blk.rs` still calls
`Driver::execute`, which is RFC 0033's reversal grep, and `CHAOS_GAP` in `xtask`
still requires that call to be there. So `JOIN_GAP` closing and `CHAOS_GAP`
staying open are not in tension — they are two different sentences about the same
component, and the first is about which components a boot *instantiates* while
the second is about which component *serves a client's load when it is killed*.
Closing the second needs the driver's own polling loop at ring 3, which needs a
component image larger than the one text page `spawn` maps and a route by which a
component asks the frame for a device translation. Neither is this RFC's.

## What would reverse this

**A need routed from somewhere that is not the spawning supervisor's account.**
The moment a manifest's `from` field names a sibling or a region the frame owns,
that need stops belonging in `account_bytes`'s sum and the account shrinks by
exactly what moved. The sum is in one function for that reason.

**A component whose declared parts do not fit `CHARGED_MAX`.** The list a
teardown gives frames back through is an array on a stack, and it is bounded
because a refund can only take back the top of a watermark. A manifest that
legitimately needs more is the point at which that stops being an array and the
account stops being a watermark — and until then the refusal is
`ADMISSION/MEMORY` before anything is spent, which is a domain rather than a
bound.

**A supervisor that is a component.** `PLACES_MAX`, `SUPERVISOR_ORDER` and the
frame minting an `Untyped` and an `Endpoint` out of nothing are all the same
deviation: the frame standing in for a component that does not exist yet. When
one does, its `Untyped` is the bound on how many places it holds, its own
account is what it was routed, and the two constants go. RFC 0008 says this and
E1-B05 owes the rest of it — the restart policy still runs in the frame, and
`component::policy::decide` is still written to take a record and a tally and no
kernel state so that moving it stays a move.

**A second reader of the boot log's spawn line.** The manifest's content hash is
printed once per place, on the spawn line and nowhere else, because
`xtask`'s `spawned_from` reads it and RFC 0036's join is a comparison of that set
against the simulator's. A second line carrying a hash would make the boot claim
to have spawned twice what it did — which is a check going green while the
property behind it stopped holding, and is the failure this epoch has now
recorded four times.
