# RFC 0136: Evidence that could not fail, made able to

- Status: accepted
- Date: 2026-09-25
- Affects: `kernel/src/component.rs` (the timeout section serves the refilled
  occupant; `Wired::tree_mapped` and `mapped_page`, a walk of the occupant's own
  page tables), `kernel/src/compositor.rs` (`Refill`, `Refilled`,
  `Report::refill_held`; the served line's page is the walk; `RECAPPED`,
  `recapped`, `compositor.recapped`; `Report::declared`; four `Board` words),
  `kernel/src/input.rs` (three restore clauses on both halves),
  `user/compositor/src/` (`reported::RESTORES`, `UNRESTORED`, `LATCH_HELD_X`,
  `LATCH_HELD_Y`; `Counters::unrestored`; `Held::pointer_transform`;
  `LateLatch::restore` is `#[must_use]`; a host test at a cap of thirty guarded
  against the manifest's), `xtask`'s compositor harness (an eighth half,
  `recapped`; the refill line in `identity_held`; `recapped_held`). Narrows RFC
  0129's *why an epoch and a page*; reverses RFC 0131's *Hard* paragraph.

## Decision

Three checks an audit showed could not go red now can, and each was run red.
**`E3-B05e`'s identity** is asked of the place's *refilled* occupant as well as
its first: after the restart the lifecycle serves epoch one with the same
client, and the frame requires that component's own control ring to say one —
epoch zero against epoch zero agreed whether or not the component read anything.
**`E3-B07e`'s cap** is exercised at a second value: `compositor=capped
compositor.recapped` runs the capped half under the compositor's record with its
framed ring's cap moved to thirty and nothing else changed, derived in the frame
at the one point the cap is read, and a host test drives `Held` at thirty with a
guard that reads the manifest's cap and refuses to equal it. **RFC 0131's
restore** is published — the count beside `latches`, the refusals the restore's
`bool` now feeds instead of being dropped, and the graph's own transform for the
pointer's node when the run ended — and the input boot requires the count equal,
the refusals zero, and the graph's transform to be the one its client committed.

## Context

The audit ran its mutations and the boots stayed green: a component reporting a
constant epoch passed `compositor serve`; a compositor that ignored its routing
word and wrote fifty passed `compositor=capped`, because the manifest declares
fifty, every test used fifty, and `laid_out` is not compiled on the host; and a
compositor that never restored its latch passed `input deliver`, because
`restores` was published nowhere, `let _restored` dropped the answer, and the
boot closes one frame. All three reproduced here before anything changed.

The live alternatives were a second compiled manifest for the cap and a
two-frame input boot for the restore. A second manifest is three hundred lines
that must stay identical but for one number, and nothing checks two manifests
against each other, so the second capped run could quietly become a run of a
different component. A second input frame proves a leftover patch is read as
*committed* only when the next latch compares it, which is one step removed; the
graph's own answer for the node at the end of the run is the same fact read
directly, and it is the reading a restore that counts without writing cannot
reach.

**What the page is and is not.** RFC 0129 said an epoch and a page together
name an instance. The page on the served, liveness and timeout lines was one
field, `tree_physical`, printed three times. The served line now prints the page
the occupant's own page tables translate its tree address to, so the harness's
page comparison is two readings of *where the tree is*. It is still not an
instance identity: the refill is spawned out of the account its predecessor's
frames were refunded to and lands on the same page — the boot shows it at
`0x575000` both times. The instance identity rests on the epoch, and that is why
the refill had to be served.

## Consequences

**Easy.** Each check has its red on record, run and quoted in the work that
produced this RFC: the constant epoch reddens the kernel's verdict at the refill;
a lifecycle that skips the refill reddens `identity_held` for want of its line;
a walk of the wrong address reddens the page comparison; a component writing
fifty reddens `compositor recapped` and a `Held` counting fifty reddens the new
host test while the sixty-three others stay green; a kernel ignoring
`compositor.recapped` reddens `recapped_held`; a test moved to fifty reddens on
its own guard; a missing restore reddens the count and a restore that counts
without writing reddens the graph clause.

**Hard.** The recapped occupant was spawned from the record as compiled; only
its routing word comes from the derived one. That is honest while the cap is
read in exactly one place. The serving half now runs its script twice, so a
compositor boot costs one occupant's run more; the rows and `claims/0038` are
the first occupant's and unchanged.

**Forecloses** reading the page as an identity, and a restore whose failure is
invisible to a boot.

## What would reverse this

**An identity the frame mints per instance**, unique for the length of a boot —
RFC 0129's own reversal — replaces the refill as what makes the identity able to
fail, and the second serving run goes. **A spawn or an admission that reads the
cap** makes the derived record a lie about what was spawned; the variant must
then be a compiled record the place is filled from, produced by `xtask`
compiling the one manifest twice with the field overridden. **A latch that never
enters the retained graph** — RFC 0131's reversal — leaves nothing to restore,
and the four board words and the three input clauses go with it. **A tree
reached by a grant rather than a fixed address** leaves the walk no address to
start from, and the served page goes back to being the lifecycle's word.
