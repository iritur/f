# RFC 0005: Speculation is a boundary the language does not draw

- Status: accepted
- Date: 2026-09-02
- Affects: `docs/design/fast-path.html` bet 01 and section 14,
  `docs/what-must-be-stated.html` sections 14, 16, 18 and 19,
  `docs/the-long-plan.html`'s uncovered speculation row, `CONTRIBUTING.md`
  R02, the component manifest of E1-D04, the supervisor of E1-B05, `ring/`,
  `kernel/`, `LICENSING.md`, RFC 0003, RFC 0007, RFC 0008 and RFC 0016

## Decision

Language-enforced isolation is an integrity boundary and not a confidentiality
one. The type system decides what a component may *write*; the hardware's
speculative machinery decides what it can *read*, and the two do not agree.
Confidentiality in this system is therefore drawn along a different line. A
**speculation domain** is a set of components permitted to share a core's
microarchitectural state — its predictors, its buffers, its caches — and a
component's domain **kind** is declared in its manifest, in the topology, and
never inferred at run time.

Three kinds. The identifiers are the values the manifest's required `domain`
field carries, and they are the only values it may carry.

| `domain` | who belongs | what it buys | what it costs |
| --- | --- | --- | --- |
| `shared` | components built from the permissive tree, with no `unsafe`, reviewed here | the hot path: siblings co-scheduled, switches with no flush, in-domain ring calls at the price claim 0001 measures | no confidentiality claim between members — declared, not discovered |
| `private` | every imported driver; any native component that holds a secret | at every instant, no core or sibling running it runs another domain; a predictor barrier and the buffer flushes the part documents, on every switch in and out | the flush on every crossing, and a sibling that idles or runs the same domain |
| `hostile` | code nobody in this tree vouches for: a guest, an agent's program, anything downloaded | a whole physical core for its lifetime, held as RFC 0007 holds one; a last-level cache partition, or exclusion where there is none; on hardware that leaks the frame's mappings, the frame unmapped from its address space beyond the entry | a core the machine no longer has for as long as the component lives, and a full address-space switch on every call it makes to the frame |

Five rules travel with the table.

1. **The field is required and closed.** A manifest without `domain`, or with
   any value not in the table, is refused by the supervisor in the `ARGUMENT`
   domain of RFC 0010 before a page of the image is mapped. There is no default
   kind. R04.
2. **A kind is delivered in full or the spawn is refused.** A machine that
   cannot supply an idle sibling for a `private` component, or a whole core and
   a cache partition for a `hostile` one, does not host it, and does not host it
   "with a note". The refusal is `ADMISSION`, naming the component that could
   not be satisfied. A part that reports no thread-level sibling in the
   extended topology leaf satisfies the sibling clause by construction — there
   is no sibling to exclude and nothing to refuse — and the supervisor records
   the mechanism as *unexercised* rather than as satisfied, because those are
   the same admission and very different evidence. That is the QEMU case, and
   the Consequences section says what it costs the tests rather than letting a
   green boot mean more than it does. This is RFC 0007's argument applied to
   confidentiality instead of latency: exclusion costs capacity, which is
   visible, and waiving costs a leak, which is not.
3. **Nothing inherits a kind.** The kind is a property of the image, carried by
   the manifest whose hash a spawn names under RFC 0008, and a child's kind
   comes from the child's manifest and nowhere else. R06. A spawner may create
   a child at its own kind or a more isolated one and never at a less isolated
   one: `hostile` spawns only `hostile`, `private` spawns `private` or
   `hostile`, and only `shared` may spawn into `shared`. Otherwise the
   supervisor is the route by which attacker-authored code joins the domain
   every native component lives in. That ordering makes a spawner's kind a
   ceiling on every descendant's, so the root decides what the whole tree can
   contain and is named here rather than discovered: **the supervisor declares
   `shared`**, which is the table's first row read literally — native, built
   from the permissive tree, no `unsafe`, reviewed here. It is not an exception
   made for convenience. What a supervisor holds is capabilities, and a handle
   is an index into the holder's own table, so an index observed from another
   domain names nothing there; revocation, not confidentiality, is what guards
   what a supervisor has. A supervisor judged to hold a secret and moved to
   `private` would make the `shared` row unreachable for every component in the
   system and buy nothing for it, and if anyone ever makes that judgement it
   amends this rule rather than editing a manifest. The one component with no
   spawner is outside rule 1 by construction: the frame *starts* the first
   component (`user/init` today) rather than spawning it from a manifest, so
   there is no field to read and nothing to refuse. E1-B05 gives it a manifest like every
   other component and it declares `shared`; until then it is not an
   undeclared kind, it is the `shared` domain, and an image the frame starts
   that no manifest describes is a state that ends when the supervisor lands.
4. **The licence boundary is the speculation boundary.** An image built from
   `third_party/` may declare `private` or `hostile` and may not declare
   `shared`. RFC 0003 put imported code in its own address space, reachable
   only over a ring; that was the integrity half. This is the other half, and it
   lands on the same line for the same reason `LICENSING.md` gives: nobody can
   grep for "is this driver leaking", but everybody can grep for a module path
   beside a manifest value.
5. **The frame is not a kind and cannot be declared.** It is present in every
   domain's address space, and its obligation is unconditional: every index the
   frame reads from memory a peer can write — a capability handle, a buffer
   index, a cursor, a slot — is bounds-checked *and masked*, so that the
   mispredicted path loads in range too. The gadget the frame presents to every
   kind is a bounds check followed by a dependent load, and a bounds check on
   its own is what that gadget is made of. The mask is derived from the check's
   outcome and applied after it; it never replaces it, and the Context below
   says why that distinction is worth a sentence. It also goes under the same
   `cfg` as the check it depends on: RFC 0017's `mutate-unchecked-index`
   removes that check to prove property five is load-bearing, and a mask left
   standing would hand the mutated build an in-range index, turn the boot green
   and retire a falsifiable property without anyone deciding to.

**What is mechanised and what stays review.** Rules 1 and 2 are the supervisor's
refusals and land with E1-B05; they fail closed, so an absent check is a
component that does not start rather than one that starts unprotected. Rule 4,
and rule 1's schema again, are `cargo xtask lint-manifests` over every manifest
in the tree — the *topology check* the R02 row of `CONTRIBUTING.md` had been
promising, and the half of that row that is true in the present tense — so that
a boot is not the first place a missing field or a `third_party` image in
`shared` is found. Rule 3's ordering is the supervisor's. Rule 5 stays review,
and so does the question of which native component *holds a secret* and therefore
belongs in `private` rather than `shared`: that is a judgement no grep makes.
The table says what each kind means; it cannot say what a component contains.
Saying which rows are review is R01 applied to this RFC.

**Two boundaries that share a name and are not the same thing.** E1-B01 gives
every component that holds a device an IOMMU domain: the set of frames its
device may reach by direct memory access, enforced by a translation the device
cannot speculate through, and per component regardless of kind — a `shared`
virtio-blk gets one exactly as a `private` imported GPU does. The speculation
domain bounds what a *core* running the component can observe transiently; the
IOMMU domain bounds what its *device* can reach. Neither implies the other. A
perfect IOMMU domain does nothing about the driver's own code sampling a
sibling's buffers, and an exclusive core does nothing about its network card
writing anywhere it likes. An imported driver needs both, which is why RFC 0003
committed to the first and this RFC adds the second. And RFC 0010 uses *domain*
for the kind of thing that refused an operation; that is a field of
`Cqe.result`, this is a field of a manifest, the two never meet in one
structure, and the collision is named here so it is not mistaken for a
relationship.

**Why the topology and not the scheduler.** Assigning the kind in the manifest
rather than deciding it at a switch is not a preference for declarations. A
run-time decision to flush, or to hold a sibling idle, is a decision about what
*another* core is about to run, which is a cross-core protocol — and RFC 0016
says a fifth cross-core word needs an argument, and a protocol that cannot fit
in a word needs a structure. Static assignment dissolves the protocol: a
`private` component is placed on a physical core, both siblings of it, at
admission, so at run time its sibling is idle or running the same domain *by
construction*, and no core ever asks another what it is doing. The flush on a
switch is then an action one core takes on itself. Nothing in this RFC adds to
the four words in `kernel/src/smp.rs`.

## Context

`docs/what-must-be-stated.html` files this as the one gap that differs in kind
from the other eight: not something F has not designed yet, but a claim F is
already making and has not defended. The architecture document's case for the
ring counts the transient-execution mitigations as part of the syscall cost it
avoids — and those mitigations are bought against a *hardware* boundary. A
boundary drawn by a type system is the one this class of attack defeats most
reliably, as browser and WebAssembly sandboxes have shown repeatedly, and no
document in the corpus distinguished integrity from confidentiality. The
absence of a mitigation cost had been read as the absence of a threat. The gap
register asked for this RFC before any untrusted component is hosted; E1 is
where the first imported code arrives, so it is being written at the last
moment it can be written before the thing it gates, and later than
`what-must-be-stated.html` section 14 asked. That is recorded rather than
smoothed over.

What is true in the tree as this is decided. Every process has had its own
address space since M3 (`kernel/src/process.rs`), with the frame mapped into
each. `ring::Consumer::pop` masks the cursor into the index ring, reads a slot
number from memory the peer owns, bounds-checks it against the entry array and
then loads through it; `cap::Table::resolve` checks a handle's index against
`TABLE_SLOTS` and then indexes. Both are correct integrity checks and neither is
a defect — they are the two constructs `cap.rs` reduces panic-freedom to, and
RFC 0017's `mutate-unchecked-index` exists to prove one of them is load-bearing.
`cap.rs` says of its check, in the tree today, "Checked, never masked. A mask is
the bug this returns an error for", and that sentence is right about the thing
it is about: an index masked *instead of* being checked names the wrong slot
silently, which is one of the two constructs property five exists to exclude,
and the comment stays. Rule 5 asks for the other thing — a mask *after* the
check, derived from its outcome, on the path the check already allowed — and
one word carrying both readings is why this is written out here rather than
left to whoever reads that comment next. Whoever lands the masking writes both
halves into that comment in the same diff. The point of rule 5 is that a
correct integrity check is exactly what the speculative gadget is built from.
Each of those two sites owes one more instruction, the check's outcome folded
into a mask on the index, and that instruction is the whole of this RFC's cost
inside the frame. The second core
came up at E0-B10, and `kernel/src/smp.rs` already reads the extended topology
leaf, at the core level, to count logical processors; the thread level of the
same leaf is one subleaf away, so whether two APIC ids are siblings is a
question the boot can ask and has not yet needed to.

The corpus proposed three kinds under different names: the frame, a shared
domain, and private domains. Two things moved. The frame is not a kind a
manifest can declare, so it became rule 5, an obligation rather than a row. And
"private" was split, because the two things the corpus put in it — an imported
graphics driver and an agent's downloaded program — differ by the most
expensive resource on the machine. Whether that split earns its place is the
last reversal condition below.

Alternatives that were live:

- **Mitigate everywhere.** Every switch flushes, every component is effectively
  `private`. Rejected because it re-imports the multiplier the ring exists to
  avoid and pays it between components that hold nothing from each other; the
  latency claims would then either be false or the mitigations would be turned
  off "for now", which is how the present gap was made.
- **Mitigate nowhere and say so.** Declare confidentiality within a machine a
  non-goal. Rejected on two grounds. RFC 0003 hosts imported C, and
  `fast-path.html` section 12 names a confidential-computing guest and an agent
  execution host as the workloads this architecture is for. And "not a goal"
  decays to "not a problem" within a document or two — the gap register shows
  the decay already complete.
- **Decide at run time.** Flush when the scheduler switches between components
  that look different: a different owner, a different licence, a device
  capability held. Rejected for the cross-core reason above, and for a second:
  a decision made from behaviour changes when behaviour does, and the component
  most worth protecting against is the one that arranges to look harmless.
- **Two kinds.** Trusted and untrusted. Rejected because it forces the imported
  driver into the same box as arbitrary downloaded code, and that box costs a
  whole core for life — or, collapsing the other way, puts imported drivers in
  `shared`, which is the thing R02 forbids. `private` is the middle rung of a
  cost ladder: the switch, not the core.
- **One domain per address space.** The model page-table isolation gives Linux.
  Rejected as the wrong axis. Address spaces are already per process here and
  do nothing about a sibling or a shared predictor; they are integrity
  boundaries, which is the whole point.

## Consequences

**Easy.** The confidentiality claim becomes a sentence that can be checked from
two manifests: "A cannot read B by speculation" is a statement about A's kind
and B's kind. `docs/the-long-plan.html` has carried an uncovered row — *a
speculative read across a domain* — with nothing catching it because domains did
not exist; the negative suite now has a definition of "another domain" to test
against, and that test is a research contribution rather than a checkbox. The
licence boundary acquires a third coincident property for one lint. RFC 0007's
physical-core mechanism is used twice — one mechanism, two claims, which that RFC
said was the shape to look for. And the supervisor's refusals are a few lines
that land with the schema.

**Hard.** Cores, and it is written here as a cost beside the claim it buys
(R12). A `hostile` component takes a physical core for its lifetime, on top of
what RFC 0007 already takes for the hard class; on a two-core development
machine it takes half the machine and a second one cannot be hosted at all. A
`private` component pays a flush on every crossing, and the imported graphics
stack is the crossing-heavy component in the system — so the risk RFC 0003
already names, that graphics may end up in-frame with a documented exception,
acquires a second reason to bite. Rule 5 costs an instruction on the ring's hot
path, and claim 0001 is where it will show. The list of what a part must flush
is the vendor's and is per part; it belongs beside the machine description in
`claims/runner-class-A.md` and its successors, not in this RFC, and it is a
list that goes stale. And a QEMU virtual CPU has no sibling: under `cargo
xtask run` the sibling rule is vacuous and a passing test says nothing about
the mechanism. That is the same shape as the AArch64 scar in `CLAUDE.md` — the
class of bug this RFC is about is invisible on the machine the tests run on.

Two claims are owed and no number is given here for either; both are in
`what-must-be-stated` section 18. *Cross-domain call and setup cost*, against
an in-domain ring call, a tuned Linux syscall and seL4-class IPC as the floor,
decides whether `private` is affordable at all; it is owed by the task that
hosts the first `private` component. *Frame index-masking overhead*, against
the same build with the masking removed — RFC 0017's mechanism is the right
one — decides whether rule 5 survives contact with claim 0001; it is owed by
the change that adds the masking. Masking that costs measurably is a rule that
will be quietly disabled later, which is why the number is being asked for
rather than assumed.

**Foreclosed.** A default kind. A kind chosen at run time from anything a
component does. An imported driver in `shared` "until the shim settles". A
spawn that inherits its parent's kind. A `hostile` component on a hot path — by
construction, since every call it makes to the frame is a full switch, and
RFC 0014 rule 3 says nothing at the door is made fast. And one that reads
oddly until it does not: a report that component A read component B's memory by
speculation, both in `shared`, is a manifest error and not a kernel bug. The
table said so.

## What this RFC does not promise

Restated here because `what-must-be-stated` section 19 predicts that this RFC
will tempt someone to promise a speculation proof.

- **No proof.** Verification in this project stops at the frame and the
  protocols (`docs/design/proving-ground.html`); seL4's cost is the reference,
  and it does not extend upward. The negative-suite test is a demonstration
  that a *known* attack fails against a *declared* mechanism. It is not
  evidence that no attack exists, and nothing written above it may cite it as
  if it were.
- **The flush list is relayed, not made.** What a part must have flushed to
  separate two domains is the vendor's statement about the vendor's silicon.
  This RFC says *that* the list is applied on every `private` switch and says
  nothing about whether the list is complete.
- **Nothing within a kind.** Two `shared` components may read each other;
  that is what the word means. Two `hostile` components on their own cores
  still share memory bandwidth, which this RFC does not partition — RFC 0007
  partitions it for latency, in the hard class, and `hostile` does not take
  that allocation — so a colluding pair can signal through contention.
  Covert channels between components that *want* to communicate are not
  addressed here.
- **Nothing physical.** Power, electromagnetic and thermal channels, and any
  attack with the machine open.
- **Nothing about hardware not yet shipped.** The mechanisms are named by
  role — predictor barrier, buffer flush, address-space split, sibling
  exclusion, cache partition — because each has an instruction or a register on
  both architectures this tree builds for and the instruction differs. A part
  that offers a new mechanism, or withdraws one, changes the per-part list and
  not this table.

In the comparison document's vocabulary: between kinds, confidentiality is
*mitigated by a declared mechanism*; within a kind it is *not held*, and the
system says so in a field a reviewer can read.

## What would reverse this

- **The cross-domain cost claim.** If hosting a `private` component costs a
  microkernel crossing per ring call *and* the datapath workloads of E1-P10
  cross into it often, then bet 01's premise does not hold at the
  confidentiality level. The remedy is to say so in `fast-path.html` and
  supersede this RFC, not to narrow the definition of `private` until the
  number fits.
- **The masking claim.** If rule 5 costs measurably against claim 0001, the
  unconditional rule will not survive and this RFC should be superseded by one
  that makes masking per kind — a worse system, and the number is what would
  justify it.
- **A hardware primitive.** A shipping, verified mechanism for speculative
  isolation *within* an address space — a domain tag the predictors and caches
  honour — on the runner class that takes claims. Then `private` loses the
  sibling and the switch, the kinds survive and the cost column is rewritten.
- **A leak between kinds.** The negative-suite test demonstrating a read across
  kinds with every mechanism in this RFC in place and the per-part list
  current. That reverses the *definition* of the kind that leaked, not the
  split; the table is amended and the test stays.
- **An unused rung.** If, by gate G2, every manifest in the tree declares
  `shared` or `hostile` and none declares `private`, the middle rung is a
  preference wearing a kind's clothes and the split collapses to two. The
  opposite observation is also a reversal: if untrusted hosts are routinely
  declared `private` because `hostile` is unaffordable, rule 2 has been made
  decorative by the people it constrains, and the answer is a cheaper
  `hostile` — the address-space split alone, perhaps — recorded as a new table
  rather than a quietly relaxed old one.
