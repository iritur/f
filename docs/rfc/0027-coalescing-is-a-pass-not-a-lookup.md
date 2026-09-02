# RFC 0027: Coalescing is a pass, and a shard keeps what it is given

- Status: accepted
- Date: 2026-09-03
- Affects: `kernel/src/mem.rs`; `kernel/src/state.rs` and `abi/src/state.rs`
  (three published nodes and one unit); the module documentation's claim that
  the free list has "no metadata"; RFC 0016's list of what crosses a core;
  RFC 0023's frontier, which survives unchanged

## Decision

The frame allocator becomes the buddy allocator
`docs/design/deadline-all-the-way-down.html` section 03 has specified since
before M1, and it does so **without the per-order bitmap a textbook buddy
allocator keeps**. Two decisions make that possible, and both are the point of
this RFC.

**Coalescing is a deferred pass, not a test on the free path.** `free` pushes
a block onto a list and does nothing else. Buddies are found later, by a pass
that takes an order's whole list, merge-sorts it by address in place — the
blocks being sorted are the sort's only storage — and pairs off adjacent
neighbours that are buddies, sweeping upward from order 0. There is no bit per
pair, no side array, and no question ever asked about a block somebody else
owns.

**A block is freed to the core that frees it.** There is no remote-free queue
and no home core. A shard reaches past itself in exactly two places, both off
the allocation hot path and both counted: the machine-wide *frontier* when it
has nothing of its own, and *another core's shard* when the frontier is spent.
The counts are published in the state tree, so "allocation took no cross-core
traffic" is a number a boot prints rather than a claim a comment makes.

## Context

The M1 allocator was one clean list, one dirty list, one frame size, one core,
and it said so in its own module comment. `Order` had been in every signature
since M1 and `alloc` refused everything above order 0. E1-B12 is where that
stops being a floor.

Four properties of the M1 allocator were load-bearing and are preserved: no
metadata outside the frames, links masked with a per-boot cookie from the
`Env`, clean and dirty lists so no block reaches a new owner carrying the old
owner's bytes, and RFC 0023's frontier — `add_region` walks and decides but
writes to nothing it accepts. The design pressure of this change was almost
entirely "how do you get buddy orders while keeping all four".

### Why not a bitmap

The obvious answer to *is my buddy free* is one bit per pair per order, about
one bit per frame in total. It was rejected twice over.

It is proportional to how much memory the machine turned out to have — half a
mebibyte on the 16 GiB machine in `docs/first-boot-outside-qemu.md` — so it
must be allocated from something, and the only thing that could allocate it is
the allocator being bootstrapped. That is precisely the bootstrap problem the
intrusive list was chosen to avoid, and RFC 0023 already rejected a side table
once for it.

And it is machine-wide while the free lists are not. A bitmap two cores both
consult is either locked or racy, and this kernel has no locks (RFC 0016).
Sharding the bitmap would mean a bit whose owner changes when a block does,
which is the metadata problem with a concurrency problem stapled to it. **The
per-CPU rule is what kills the bitmap, not the memory cost**, and that is worth
recording because it is the argument that will still hold when memory is cheap.

### Why not read the buddy's own first word

A free block's first word is its link. It is tempting to give it a second word
holding a tag — `FREE ^ cookie ^ address ^ order` — and decide a merge by
reading the buddy's tag.

Rejected, and not narrowly. An *allocated* block belongs to somebody who may
write anything into it, including bytes that look exactly like a valid tag. The
cookie is drawn from the `Env` and the seed is printed in the boot log, so it
is not a secret from anything that can read a serial line; and even a real
secret would only raise the price of forging a merge of a block a component
still holds. An allocator whose correctness rests on a guess about somebody
else's bytes has no correctness at all. The masking cookie is a defence against
*corruption*, and this RFC keeps it in that role and refuses to promote it to
an authenticator.

### Why not a remote-free queue

The usual answer to "core B freed a block that belongs to core A" is a
per-core queue of foreign frees, drained by the owner. It buys buddy locality:
a block always goes home, so its buddy is on the same shard.

It was rejected because the exit criterion is about the *hot path* and the free
path is as hot as the allocation path — the two are reached from the same
places, one after the other, in `process::reap` and in every unmap. A queue
puts a cross-core store on both. Worse, it needs a home to send a block to, and
a home is either metadata per block (see above) or an address-range partition;
and an address-range partition fine enough to give eight shards something on a
128 MiB machine is coarser than a gibibyte block, so it cannot give an order-18
allocation a single owner.

Alternatives that were live and are recorded because each is a real design:

- **Address-homed shards** (`home = (addr >> 30) % cpus`). Free of metadata and
  arithmetic-only, and it fails on any machine smaller than `cpus` gibibytes,
  where every frame homes to core 0 and every free from another core is remote.
- **Migration with no steal** — what this RFC does, minus the steal path. Two
  cores' memory drifts apart and a core that has given everything away cannot
  allocate although the machine has memory free. That is not fragmentation, it
  is a failure, so the steal path is not optional.
- **Address-ordered insertion**, so a free finds its buddy as a neighbour.
  Correct, needs no metadata, and puts an `O(n)` walk on the free path. The
  deferred pass is the same idea with the cost moved to where it can be
  scheduled.

## Consequences

- `Order::HUGE` is the grain a *shard is refilled in*, which is what "huge
  pages by default" means here. A core that wants one frame and has none takes
  two mebibytes off the frontier and splits it down, so the 511 frames beside
  it are on the same shard and adjacent — which is what lets the pass put the
  huge page back together. An allocator that took single frames off the
  frontier would produce blocks with no buddies to find. Callers still ask for
  the order they want: making `Order::DEFAULT` a default *argument* would cost
  two mebibytes per page table.
- RFC 0023 is not merely preserved, it is cheaper. Refilling a shard with a
  two-mebibyte block writes nine link words — one per buddy left behind on the
  way down — and serves 512 frames. The old eager threading wrote 512. The
  runs-array overflow fallback improved the same way: a range is now carved
  into aligned blocks, about two writes per order rather than one per frame.
- The clean/dirty guarantee holds at every order, and the argument is about
  bytes rather than labels. **Every byte of a block on a clean list is zero
  except its first eight, which hold its masked link.** Splitting preserves it
  because it writes no byte of either half except the link words the lists
  need: the lower half inherits its parent's first word, the upper half's first
  word was zero. Coalescing preserves it because the pass zeroes the *upper*
  half's link word before merging, that being the one byte range inside the
  merged block that would otherwise not be zero. `alloc_zeroed` clears the
  first word on the way out. A merged clean block's interior link word is the
  one thing a frame-sized hygiene check can never see, so `hygiene_test` now
  builds one deliberately and reads all two mebibytes of it.
- **`free` accepts a block back in pieces.** A block may be returned whole, or
  as any set of aligned sub-blocks that exactly tile it. That is a property of
  the buddy structure rather than a concession, and it is what the coalescing
  pass exists to put back together; the self-test returns a huge page as 512
  frames in an order the `Env` chooses and requires a huge page to come back.
- The free path stays a store and an increment. The cost of deferring is
  fragmentation between passes: a machine that never coalesces answers a
  huge-page request from the frontier or refuses it. `coalesce(budget)` is the
  bounded, schedulable form for the batch-class caller that does not exist yet;
  an allocation that would otherwise fail runs the unbounded form itself,
  because it is the one caller that can afford to.
- Blocks freed on the wrong core do not merge with their buddies. That is a
  fragmentation cost, and it is bounded by how often work migrates rather than
  by how much memory the machine has.
- Four nodes join the state tree — `memory.served`, `memory.refill`,
  `memory.remote` and `memory.forced` — with one new unit, `EVENTS`. They are
  ids 12–15 and sit at the end of the schema array rather than beside
  `memory.free`, because `validate` requires ids to ascend in schema order and
  ids are permanent. The fourth exists because the third is not readable on its
  own: the self-test provokes the remote path on every boot (see below), so
  `remote` is never zero, and the figure the exit criterion is about is the
  difference. The boot log takes that difference in the line it prints; a
  reader who maps the tree under load rather than reading a serial line is
  asking the same question and must be able to take it too. The boot log grew
  two lines and the state block grew four, so the trace hash moves once;
  `cargo xtask trace` still reports two runs of one commit agreeing, which is
  the property E0-P02 claims.
- RFC 0016's sentence needs reading with this beside it. The four cross-core
  *words* in `smp.rs` are unchanged and no fifth is added: the steal path is
  not a handshake and not a shared word, it is one core mutating another core's
  shard under the invariant that exactly one core mutates this allocator at a
  time — the allocator is reached through one `&mut`, lent to a running
  process's core as a `&` that only computes addresses. **When a second mutator
  appears, this path is what needs an answer**, and the counter is what says
  how often it is taken and therefore whether the answer is a lock, a queue, or
  a different partition. Publishing the count before the second mutator exists
  is the whole reason it is published.
- The order the fixture cannot reach is checked by a second boot.
  `cargo xtask run` is pinned at 128 MiB because its log is what
  `cargo xtask trace` hashes, and 128 MiB tops out at order 13 — so
  `mem::self_test` *reports* how far up the machine it is on reached, and
  `cargo xtask orders` boots the same image on a machine with a gibibyte in it
  and *requires* 18. Without it, `Order::up`'s bound, the top of the coalescing
  sweep and `refill`'s branch above the default grain are three paths no check
  in the tree executes, and a number reproduced by hand in a report is what
  `claims/README.md` calls an anecdote. It is in `verify` and in the gate; it
  costs one boot, because nothing writes the memory it asks for (RFC 0023).
- Because the boot fixture has 128 MiB and never spends its frontier, the
  remote path would never execute — and a counter that cannot move is
  indistinguishable from a counter that works. The self-test therefore
  withholds the frontier and asks an empty shard for a frame, every boot. This
  is the shape of `docs/first-boot-outside-qemu.md`'s findings applied in
  advance: a path first executed on a large machine is a path first debugged on
  a large machine.

## What would reverse this

- **A measured cost of deferral.** A workload where the compaction pass runs
  often enough to matter, or where a huge-page request fails for fragmentation
  that eager coalescing would have prevented, argues for the bitmap — and would
  have to argue against the per-CPU rule at the same time, which is the harder
  half.
- **A second mutator of the allocator**, which E1-B08's user-level runtimes and
  E1-B05's supervisor will produce. At that point the steal path and the
  frontier are two structures two cores reach, and this RFC's invariant — one
  `&mut`, machine-wide — stops holding. The counters are the input to that
  decision: a `remote` that stays near zero under real load says a per-shard
  frontier and a refusal-instead-of-steal is enough; a `remote` that does not
  says the frame pool is the shared structure RFC 0016 predicted would have to
  argue for itself.
- **`remote` growing without a second mutator**, which would mean memory is
  drifting between shards on a machine where only one core allocates — a
  bookkeeping bug, not a policy question.
- **A machine whose translation buffer makes some other order the natural
  grain**, which moves `Order::DEFAULT` and nothing else.
- **A defect traced to the deferred pass** — a merge that produced a block
  overlapping a live allocation. One such defect, root-caused, outweighs
  everything above: it is the failure mode the bitmap does not have, and the
  self-test's exact free count plus its piecewise-return check are the two
  things standing between this design and it.
