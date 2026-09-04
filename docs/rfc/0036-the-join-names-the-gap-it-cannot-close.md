# RFC 0036: The join is symmetric, and the gap it cannot close is declared

- Status: accepted
- Date: 2026-09-03
- Affects: `xtask` (`sim_join`, `JOIN_GAP`), `sim/src/scenario.rs`,
  `sim/src/deploy.rs`, `.github/workflows/ci.yml`, RFC 0035, E1-P01, E1-B05,
  E1-B08

## Decision

RFC 0035 defined a boot-to-workload run as a pair of runs over **one component
set**, and built `cargo xtask sim --join` to require it. The check it built was
one-directional: it asserted that every component the boot spawned was among the
components the simulator ran, and asserted nothing in the other direction. This
RFC makes it symmetric, and — because the tree does not satisfy the symmetric
form — makes the difference a declared quantity rather than a silence.

Three things change.

1. **`sim --join` compares the two sets both ways.** `spawned ⊆ modelled` stays.
   The set `modelled \ spawned` is computed, printed, and required to equal
   `JOIN_GAP` in `xtask/src/main.rs` **exactly**. Not a bound, a set: a
   component appearing there that nobody declared is red, and a component still
   declared there after the boot starts spawning it is red too.

2. **`JOIN_GAP` is `["virtio-blk"]`, and that is the honest state of the tree.**
   `kernel/src/component.rs` builds one place from `*modules.first()`, so a boot
   instantiates `store` and nothing else, while the simulator runs every
   compiled record the build produced. The two halves are therefore about
   `{store}` and `{store, virtio-blk}`. **The exit criterion's "one component
   set" is not met today**, it is met for the intersection, and the difference is
   one line somebody has to delete rather than a sentence somebody has to
   notice.

3. **The `deployment` artefact stops claiming the boot's half.** Its `what` and
   its hashed header said *the component set the boot spawns*; they now say what
   this side can actually see — every compiled component the build produced —
   and name `sim --join` as the place the two sets are compared. A hash quoted
   in a year carries its own boundary, which is the property RFC 0035 gave the
   header for and which the wording was quietly spending.

**Who closes it: E1-B08.** Spawning belongs to a supervisor rather than to the
frame (RFC 0008), a supervisor at ring 3 has to adopt a control ring, and safe
channel adoption is E1-B08's — the wall E1-B05 hit and recorded. When a boot
spawns the whole module set, `JOIN_GAP` is empty and its emptiness is the
evidence.

## Context

The one-directional check was found by the first review of E1-P01, and what
makes it worth an RFC rather than a bug fix is that it is the third instance in
this tree of *a test passing while the property it stands for does not hold* —
after E1-B10's wrapping generation and E0-P08's mutation harness. The failure
shape is the same each time: a check written against the case somebody was
thinking about, in a tree where the other case was already true.

It was also live rather than latent. `cargo xtask sim --join` printed `join: ok`
on a tree where the simulator drove a component the kernel never instantiated,
the `deployment` scenario's own description asserted otherwise, and the artefact
header wrote that assertion into the hashed bytes. RFC 0035 named exactly this —
"nothing would notice if the simulator ... ran one component fewer than the boot
spawned" — and then shipped a check that would not have noticed the reverse.

Three shapes were live for the fix.

**Make the simulator run only what the boot spawned.** The `deployment` scenario
would take its set from the hashes in a boot log, and the symmetric check would
pass by construction. Rejected on two counts. It puts QEMU underneath the
host-only workload check, which RFC 0035 rejected for the merged-command shape
and which every seed sweep at E1-P03 would then pay for; and it makes the
simulator's coverage *shrink* to the frame's current limitation, so the day the
frame spawns two components the scenario would silently start covering one more
without anybody choosing that. Coverage that moves on its own is the thing this
seam exists to prevent.

**Widen the check to "the simulator may run more".** Cheapest, and it is the
version that fails the way the original failed: a one-line allowance under which
the workload half could drift away from the boot half one component at a time,
each drift green. R04 says refuse rather than tolerate, and a tolerance with no
number in it is the tolerance that never gets revisited.

**Declare the difference and gate on it** — this RFC. The gap is written where a
diff shows it, the check fails in both directions of change, and the cost is one
line of maintenance the day the supervisor lands. It is the same discipline
`DETERMINISM_ALLOW` uses for the determinism policy: not *no exceptions*, but
*every exception has a name, a reason and a reviewer*.

## Consequences

**The failing input is a component, and it fails today.** Drop a third `.fc`
with a modelled protocol into `target/component/`: the simulator runs three, the
boot spawns one, `modelled \ spawned` becomes `{virtio-blk, the new one}`, and
`sim --join` goes red naming it. Before this RFC that input printed `join: ok`.
That is the check being demonstrable rather than merely present — the same
standard `cargo xtask mutate` sets for the boot suite.

**`cargo xtask verify` now fails when the frame improves.** The day a supervisor
spawns both components, `JOIN_GAP` is stale and the join goes red until the entry
is deleted. That is deliberate and it is the direction of failure worth having:
a stale exception is a hole a check steps over, and the person who removes it is
the person whose change made it removable.

**The exit criterion's boundary is now written in three places rather than
inferred from none:** `JOIN_GAP`'s own documentation, the `deployment`
scenario's header, and this RFC. E1-P01 closes with the workload half covering
the whole compiled set, the boot half covering the first module, and the
difference named — which is a weaker claim than the exit's sentence and a true
one.

**Digests moved again, and it is worth saying which.** The `deployment` header
gained two lines, so that scenario's hashes from earlier in this branch are
superseded. Every scenario with buffers moved as well, for a reason recorded in
the same review and fixed in the same commit: the client was deriving an
operation's position from its issue counter rather than from its own token, so
sector zero was unreachable and a retried request moved. `handshake`,
`contention` and `pipeline` are unchanged, because a bounded queue has no
buffers and no position. RFC 0035 priced this shape of cost when the header was
introduced, and the price is the same: a seed binds to a commit, and this is a
commit.

## What would reverse this

**The gap closing.** E1-B08 lands a supervisor that spawns the module set, the
entry goes, the list is empty, and the symmetric check holds with nothing
declared. Then this RFC is history rather than policy — and the way to tell is
`JOIN_GAP`'s length, which is why it is a list in the source and not a paragraph
in a document.

**The list growing past one or two entries.** That would say the frame's
component set and the simulator's have genuinely diverged rather than being one
supervisor apart, and the answer then is not a longer list: it is that the join
is comparing the wrong objects, which is the reversal RFC 0035 already describes
for the seam itself. Two entries is a queue; four is evidence.

**A component that must be in the simulator and can never be in a boot.** A
model of a peer that exists only to be substituted — a fault injector, a hostile
peer at E1-P04 — would sit in `modelled` forever with no task that removes it.
That is a different kind of entry from this one, and it should be a different
mechanism: a component the *manifest* declares as unspawnable, argued through
RFC 0030 as its schema requires, rather than a second meaning quietly loaded
onto this list.
