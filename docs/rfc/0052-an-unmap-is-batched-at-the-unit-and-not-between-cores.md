# RFC 0052: An unmap is batched at the unit, and not between cores

- Status: accepted
- Date: 2026-09-04
- Affects: `kernel/src/arch/x86_64/vtd.rs`, `kernel/src/iommu.rs`,
  `kernel/src/smp.rs`, `kernel/src/churn.rs`, `kernel/src/main.rs`,
  `bench/src/bin/unmap_churn.rs`, `xtask/src/main.rs`,
  `claims/0014-unmap-churn.toml`, `claims/0015-unmap-churn-cost.toml`,
  `docs/test-taxonomy.toml` and `docs/test-taxonomy.md`.
  It does not affect RFC 0016 and the reason is the substance of this RFC.

## Decision

**The datapath's unmap churn is batched at the remapping unit and nowhere
else.** `iommu::Grant::unmap` issues one global invalidation for the whole
request rather than one per page, and the shootdown protocol between cores is
left exactly as it is — one page, one interrupt per other running core, one spin
on an acknowledgement — because the measurement that `E1-B14` demanded found
that the churn performs *no shootdowns at all*.

Both halves of that sentence are numbers rather than judgements, taken by
`cargo xtask churn` and registered as `claims/0014`:

| | control | batched |
| --- | --- | --- |
| unmap requests | 40 | 40 |
| pages cleared | 320 | 320 |
| global invalidations | 320 | 40 |
| register round trips | 640 | 80 |
| shootdowns | 0 | 0 |
| interrupts | 0 | 0 |

Seven of every eight invalidations gone, at the eight-page set the datapath
registers; fourteen serialising register round trips saved per set. And zero on
the axis the task named, against a boot whose running shootdown total is one —
so the zero is a property of the datapath rather than of a counter nobody wired
up.

## Context

`TODO.md`'s `E1-B14` is written unusually and the shape of it is the decision it
was asking for. Its ordering rule 3 forbids designing a batch before the
measurement exists, so the task permitted two outcomes and demanded a number
either way: *an unmap-under-churn workload exists beside the E1-P10 claims and
records shootdowns, IPIs and p99 unmap cost; then either batching lands with the
improvement measured on the same workload, or this task closes with the number
that says one-page-one-IPI was already under the bound.*

What was true when it was written: every revoke-unmap in this kernel is
`paging::unmap_user_live` followed by `smp::shootdown`, which is one page, one
interrupt per other core, one acknowledgement spin. That is correct and it is
priced for a kernel that unmaps rarely. The datapath was expected to change the
rate, because registered buffer sets cycle and a driver restart retires a
component's whole grant.

The rate did change. The *path* did not, and that is what the workload found.
The churn's unmaps are device translations, and a device translation is not a
processor translation:

- retiring a registered buffer set edits the remapping unit's second-level
  tables through `iommu::Grant::unmap`, and no core's translation buffer holds
  anything about them;
- a driver restart's teardown makes no shootdown either, and
  `component::tear_down` had already written down why in its own words: an
  instance's address space has never been in `CR3` on any core, so the shootdown
  is the empty case;
- the one path that does shoot down is `process::withdraw`, reached by revoking
  a capability a *running* process had mapped, which is what `cargo xtask cap
  unmap` boots and is not a datapath event at all.

So the axis the task named turned out to be quiet, and the axis beside it turned
out to be loud. `vtd`'s module comment had already predicted where and had
already named this task as the arbiter: *this build uses the registers, does a
global invalidation after every change to a table the unit walks, and pays a
serialising round trip for it. That is the wrong trade for a datapath and the
right one for a first implementation, and the number that decides when to change
it is `E1-B14`'s unmap-under-churn workload.*

The live alternatives were three. **Batch the shootdown**, which is what the
task's title contemplated: rejected by the number, because there is nothing to
batch — and it would have needed a queue of pending invalidations per core,
which RFC 0016 names as the thing that would reverse the rule `smp.rs` bends,
and which would have been a fifth word two cores reach in exchange for no
measured saving. **Batch the unit's invalidation**, which is what landed.
**Move to the queued invalidation interface**, which is a second code path and a
larger change, and which `claims/0015` is the number that would justify.

## Consequences

**What it makes easy.** A component's whole grant comes back for the price of
one invalidation instead of one per page. The saving grows with the set: it is
`2 × (pages − 1)` register round trips per request, so a datapath that registers
larger regions pays proportionally less to give them back. Nothing about the
capability model, the ring, or the component lifecycle moves.

**What it makes hard, stated as a cost rather than hidden in the metric (R12).**
The window between an entry being cleared and the unit being told widens from
one page's walk to the whole request's. A device holding a cached translation
for the first page of a set keeps it until the last page's walk is done. That
window is bounded by the request either way, and the contract `Grant::unmap`
owes its caller is about the moment it *returns* — RFC 0024's soundness rests on
a transfer faulting rather than landing once the unmap has run, and it has run
when the call returns. But the interior of the loop is longer than it was, and a
reader who wants to disagree with this change should disagree here.

**What it forecloses.** Nothing about the shootdown. `smp.rs` still has four
words that two cores reach and this RFC adds none — the two counters it adds are
`PerCpu<u64>` slots written and read only by their owning core, which is
narrower than the rule RFC 0016 already permits, and `smp.rs` says so where they
are declared. The day something in the datapath does reach `process::withdraw`,
the question `E1-B14` was written to ask becomes live again, and the workload
that answers it is already in the tree.

**What it did not fix, declared with its number.** The mapping half of the same
cycle costs exactly what the unmap half used to: `Grant::map` walks the pages one
at a time and `Unit::map` invalidates after each, so registering an eight-page
set is eight global invalidations and sixteen round trips. `CHURN_GAP` in
`xtask` is that declaration, `claims/0014`'s `map_invalidations_per_page` is its
number bounded in both directions, and the churn boot's own verdict goes red if
it moves. It was left because a map that fails part way must undo what it made —
otherwise a device holds a translation for the first half of a buffer its driver
was told it does not have at all — and batching an invalidation across that undo
is a different argument on the mapping path of two live datapaths. Doing it on
the strength of a measurement taken for something else is how a task acquires a
second one.

**What the measurement observes, and what it only counts.** Counting what the
frame did is not the same as observing that it worked, and the first draft of
this measurement did only the first: it counted cleared entries and published
invalidations and never asked whether anything was gone. It asks now. Every
retirement is followed by a walk of the unit's own second-level tables — the
same walk `registry::PageWalk` answers a registration with — requiring the set's
pages not to be translated, and every registration by the same walk requiring
that they are, because a walk that answered *no* to everything would report a
perfect revocation over a domain that never mapped anything. `claims/0014`
carries both as thresholds. The frames are counted the same way and for the same
reason: forty register-and-retire cycles per half is the rate at which a leak of
one table frame per cycle stops being invisible, and the allocator's free count
either side of the churn is required to be equal — which closes
`docs/test-taxonomy`'s *frame leak under churn* row for the registration path.

What no boot in this tree observes is a **device** faulting after a batched
multi-page unmap. Nothing is attached to the churn's domain, and the boot that
does watch a device fault after a withdrawal — `cargo xtask blk`'s `outside`
half — registers one page, where `PerRequest` and `PerPage` are the same run.
The residual is small, because the invalidation is global and one at the end
throws away every entry the loop cleared exactly as well as one after each, and
*small* is why it is `REVOKE_GAP` in `xtask` rather than a paragraph: it names
the SAFETY comment in `kernel/src/churn.rs` whose presence keeps it open, and it
goes red on the day a device is attached there or the blk boot registers more
than a page.

**What the measurement cannot say, and what it now can.** How long an
invalidation takes *here*. Counts are the same number in a container and on bare
metal, so `claims/0014` gates; a duration is not. What changed is where the
workload for the duration lives. `kernel/src/churn.rs` times a thousand and
twenty-four unmap requests through the shipped path on the machine's real
remapping unit and keeps the distribution; `kernel/src/main.rs` prints
percentiles only when the command line says this machine is a measurement
environment, and `cargo xtask churn` writes that word from
`f_bench::Environment::detect` — so one rule decides what may be quoted rather
than a second one in the kernel that could disagree with it. Recording is not
publishing, and the separation is what makes the instrument checkable: the
verdict fails on a short sample or a maximum of zero ticks on *every* churn
boot, which are counts, so an apparatus that does not work is found in a
container rather than on the machine the number will come from. The number
itself is still `claims/0015` and still `pending`, and the reason is unchanged
and worse than mere absence: the emulator answers an invalidation instantly and
in software, so a percentile taken here is a measurement of QEMU's dispatch loop
wearing a hardware unit's name. `claims/0015`'s `[hardware]` note says so, and
the refusal in the boot log says it on every run.

**What an unmap does with a page it cannot clear.** It carries on and reports
the first refusal at the end. This is written down because the batch got it
wrong once in the direction that matters: `unmap_range` broke out of the loop on
the first refusal for one revision, which would have left every page after a
hole translated — a device still reaching memory a request said it may not have,
arrived at by an error path being tidy. No caller can construct such a run today
(`Grant::map` undoes a partial mapping before it refuses, so a set is wholly
mapped or wholly absent), and *no caller can construct it* is the kind of
sentence that stops being true one diff after somebody reads it as permission.
So the churn boot constructs one: a set is mapped, one page in the middle is
taken out from under it with the unit's own single-page entry point, one batched
request is made over the whole set, and every page is walked afterwards —
`churn hole    8 page(s) mapped, 1 taken out from under the request, 0 still
translated after it`, with the mapped count beside the zero so that a stage
which arranged nothing cannot pass. Restoring the `break` prints `4 still
translated` and fails the boot. `claims/0014` carries both rows.

Nothing is put back: an unmap that stopped half way has taken authority away,
and restoring it would be re-granting a device access the caller asked to
remove.

## What would reverse this

**A device that observes the interior of the loop.** The concession above is
that a cleared entry stays live in the unit's cache until the request ends. It
is sound because nothing observes an unmap before it returns. A machine with
address-translation services, where a device takes a recoverable fault and
retries rather than failing, is a machine where the interior *is* observed — at
which point the invalidation belongs back inside the page loop, or the request
has to be split at a bound somebody argues for. `registry`'s
`SharedVirtual`/`PageWalk` split already names the hardware this would arrive
with.

**A queued invalidation interface.** If `claims/0015` shows the tail in the
round trip rather than in the walk, the register interface is the wrong one and
the queue is the answer — at which point `INVALIDATION_ROUND_TRIPS` stops being
two and this RFC's arithmetic stops being the arithmetic. That is a
strengthening of the same decision rather than a reversal of it, and it is named
here so the constant is found when it happens.

**A datapath event that shoots down.** The zero above is a measurement of
today's paths, not a theorem. A scheduler that puts a component instance on a
core makes `component::tear_down`'s empty case non-empty — its own comment says
this is the same call it will make then — and a supervisor that revokes a live
client's mapping reaches `process::withdraw` on the datapath. Either would make
one page, one interrupt a rate rather than a rarity, and the workload in
`kernel/src/churn.rs` would then be measuring the thing `E1-B14` was named for.
The churn boot's verdict fails on a non-zero shootdown for exactly this reason:
it is not a regression, it is a notice that this RFC's first half has expired.

**`CHURN_GAP` closing.** The day the mapping half is batched, this RFC's
Consequences describe a tree that no longer exists, and `gap_holds` prints the
list of documents to update in the diff that closes it rather than the one
after.

**`REVOKE_GAP` closing.** The day a device is attached to the churn's own domain
— or the boot that already watches a device fault registers more than one page —
this RFC's claim that the batch's soundness is argued rather than observed at
the device stops being true, and the same mechanism prints the documents to
update. That is a strengthening rather than a reversal, and it is the cheapest
one on this list.
