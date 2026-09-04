# RFC 0041: A place survives its occupant, and the state behind it survives both

- Status: accepted
- Date: 2026-09-03
- Affects: `sim/src/chaos.rs`, `sim/src/deploy.rs`, `sim/src/main.rs`,
  `xtask` (`chaos`, `CHAOS_GAP`, `lint-components`, `ROUTES`, `verify`),
  `claims/0005-driver-restart-blast-radius.toml`,
  `claims/0006-driver-restart-latency.toml`, RFC 0008, RFC 0024, RFC 0032,
  RFC 0033, RFC 0036, E1-P06, E1-B05, E1-B08

## Decision

Gate G1's first sentence — *a driver is killed under sustained load and the
system does not notice* — is decomposed into four checkable claims, and each is
made to fail separately.

**One.** *No client observes anything except added latency* is three sentences
and not one, and a harness asserting the coat would be asserting nothing. They
are: **no operation is lost** — every submission is eventually answered; **no
operation is answered twice** — and no completion arrives for a token its client
no longer holds; and **no operation is answered wrongly** — a value written at a
position through one instance of a driver is read back at that position through
a later one. `sim/src/chaos.rs` records each as its own label in the artefact and
`chaos::verdict` refuses on each separately, so a failure names which of the
three it was.

**Two.** *Only latency* is completed by a bound, because a wait with an
unbounded tail is a hang with better manners. The bound is not a threshold
somebody tuned: it is the control run's own worst operation plus the backoff
ladder the component's **manifest** declares, added up over the kills the run
took. Both terms are declared elsewhere — one by `user/virtio-blk/manifest.toml`,
one by a run of the same workload with nothing killed — so there is no number in
the harness to move when a result is inconvenient.

**Three.** *Under sustained load* is a number rather than a hope. A place refuses
to kill an occupant with nothing outstanding and reschedules instead, and it
writes the outstanding count into the artefact beside every kill it does take.
`operations_in_flight_at_kill_min` has a threshold of one in claim 0005, and it
is what stops the five zeros above it being free: a suite of kills landing
between operations would report every one of them while testing a quiescent
system.

**Four.** **The third claim is an ordering the code performs, not a placement
the harness chose.** The medium lives behind the place — a disk's sectors do not
die when its driver does — but *where a map lives* is not a claim, and the first
draft of this RFC had one doing the work of the other. A store the kill path
never touches produces a read-back of zero whatever the code around it does,
which is an alarm wired to a wire no fault can reach: the same shape as an IOMMU
proof that was green because nothing was being remapped. Review caught it, and
this is the correction.

What the claim rests on instead is a rule with a fault path through it: **a write
reaches the medium before its completion is handed to the client, and never
after.** The occupant holds the work it has accepted and not answered, keyed by
the token and carrying the position off the client's own `Sqe::offset` rather
than one this file re-derives; a kill discards that set, so every kill throws
away writes that were in the middle of being committed.
`writes_interrupted_at_kill` counts them, its threshold in claim 0005 is a
**minimum**, and `chaos::verdict` refuses a run that reaches zero — because a
read-back over values nothing ever threatened is a check on the harness rather
than on the system. At the gating seed the two components interrupt seven and
nine writes respectively.

Two negative controls make the alarm one somebody has watched fail.
`Chaos::lazy` gives the occupant the write-back cache a real driver would have —
it answers out of memory and commits afterwards, which is the actual
restart-durability bug — and a kill in that window loses a write the client was
told had succeeded. `Chaos::volatile` is the coarser one: a machine whose disk is
erased by a segfault. Each has a test that requires the read-back to go wrong at
three seeds, requires the *control* run carrying the same defect with no kill in
it to stay clean, and requires the verdict to name that failure and no other.

The buffer-stamp check is counted under its own name rather than inside the
read-back, because a gating threshold should not have a number that cannot move
folded into it: no device model in this crate can reach a client's bytes. It is
kept for the milestone that adds one, and claim 0005 says so beside the metric
rather than leaving a reader to find out here.

Beside all four, and not optional: **a control run of the same workload with
nothing killed**. A survival with no control beside it establishes that nothing
went wrong, not that anything was under test — the argument `blk`, `runtime` and
`mutate` each make one subsystem over.

### The gap, declared as a set rather than left as a silence

RFC 0036 is the precedent. The simulator kills a component that is *serving a
client's sustained load*; a boot kills a place's occupant that serves nobody,
because nothing schedules a component's polling loop on the datapath yet. That
difference is `CHAOS_GAP` in `xtask/src/main.rs`, and it is a set of things whose
**presence** keeps it open — today, one: `driver.execute(` in
`kernel/src/blk.rs`, which is RFC 0033's own reversal condition stated as a grep.
`cargo xtask chaos` requires every entry to still be there, so the day one goes
the build is red and whoever closed it is told to move the kill into the boot and
delete the declaration. A gap that quietly stops being true while the document
keeps describing it is the failure this shape exists to refuse.

## Context

E1-P06 arrived with two halves of a system and no join between them. `E1-B05`
built the lifecycle — spawn from a manifest, one control ring per component,
notices as pending state, endpoint-as-a-place, uniform teardown — and could not
run its occupant, because nothing schedules a component. `E1-B08` built the
runtime — a component holding a core and driving its own rings in safe code — and
does not spawn into a place. `kernel/src/blk.rs` still calls `Driver::execute`.
Its own note says the two remaining moves are work rather than a wall, and they
are not this task's.

So the choice was between a green result over a smaller thing and an honest
result with a declared gap. This RFC takes the second, which is the shape
RFC 0036 established: the workload half runs in the simulator, where sustained
load, seeded kills and byte-identical replay are all real; the frame half is
`cargo xtask component`, which kills a place's occupant against real memory on
the boot core; and the difference between them is a quantity the build checks.

Three alternatives were live.

**Kill the scheduled runtime instead.** `cargo xtask runtime` puts a component on
a core with its own executor and 16 384 work items. Killing that is a real kill
of a real scheduled component. It was rejected because the runtime serves *its
own* queue and has no client: there is nobody on the far end of a ring to observe
anything, so the exit criterion's subject would not exist. A kill nobody could
observe is not the experiment.

**Make the client library retry and call the restart invisible.** Rejected
because it moves the claim rather than making it. A client that reconnects and
re-submits is exactly what a client of a place does — RFC 0008 makes the endpoint
survive the occupant precisely so it can — but the claim has to be about the
*application* above that library, which is why the ledger is keyed by logical
operation and not by submission. A re-submission is one operation submitted
twice; an answer to both is a failure, and that is the distinction the ledger
exists to hold.

**Put the store in the device model.** Rejected on the first draft, and the
reason is worth keeping: `sim/src/blk.rs`'s device is what the *occupant* is, so
a store inside it would be a disk erased by a driver bug. It would also have made
the read-back check pass for the wrong reason — a store that dies and a store
that never existed answer a read the same way.

The fourth alternative — assert the buffer's own stamp and call that the content
check — was rejected because `sim/src/fault.rs` has already recorded why it would
be worthless: no device model in this crate can touch a client's buffer, because
`Reach` is an address and a length and there is no type that turns one into
bytes. A check that cannot fail is indistinguishable from one nobody wrote. The
stamp is still checked, and it is not what the third claim rests on.

## Consequences

**Easy.** A component added to `user/` is a component this harness kills, and
whoever adds one that cannot survive being killed finds out from a red build with
a reproduction command rather than from a bug report.

That sentence was written before it was true, and it is worth recording why. The
sweep's set is the deployment's — the component files the build produced — and
the coverage check compared it against the deployment directory, which is the
same set read twice. It could not fail: a component dropped from `COMPONENTS` in
`xtask/src/main.rs` would have vanished from *both* sides at once and printed a
green `coverage 1 of 1` over half the tree. That is a join comparing a directory
with itself, which is the third time this epoch, and a better error message would
not have fixed it.

So the other side of the comparison is now derived independently, from the one
property of a component that cannot be hand-maintained: a directory with a
`manifest.toml` is a component. `manifest::files` walks for them, `cargo xtask
chaos` requires the number its sweep killed to equal that count, and a new
`lint-components` requires the build list to equal that set in both directions. A
component crate carrying a manifest and no build entry now reddens the lint; one
dropped from the build output reddens the sweep.

And the blast-radius claim gates *here*: every metric is a count produced by a
virtual clock, so it is the same number in a container and on bare metal, and
`cargo xtask verify` runs it.

**Hard.** Three knobs now exist in shipped source whose only callers are tests:
`lazy`, `volatile` and `leaky`. That is a real cost and it is the same one
`runtime::Half::Provoke` and `dev.rs`'s deliberate defect pay: an alarm nothing
can trip is not evidence, and the alternative — a patch applied by hand when
somebody remembers — is how a negative control stops being run. Each is one
branch, named after itself, and refused as a default.

**Also hard.** A component's declared restart policy is now a question the
verdict asks rather than an assumption it makes. `restart = never` is a value
`docs/manifest.md` permits and RFC 0008 gives meaning to, and the first version
of this harness applied one verdict to every component — so the first component
to declare it would have turned `cargo xtask verify` red for a legitimate
statement about itself, and the fix under pressure would have been to widen the
verdict, which is the wrong direction. R04 read correctly is refusing what the
build does not expect, not refusing what the schema allows. A component that
declares `never` is therefore judged by a different question: the place is
*expected* to stay empty, *nothing is lost* is not asked because the client was
told its peer is gone and no peer is coming, and what still holds is that nothing
was answered twice, nothing wrongly, and nothing refused in a way the client
could not retry. A refill against such a policy is itself a failure, and the
verdict says so in those words.

**Foreclosed.** The latency in gate G1's sentence cannot be published from this
tree. Claim 0006 is `pending` and says why: the figure `cargo xtask chaos` prints
is *virtual* nanoseconds, a property of the model's parameters rather than of any
machine, and a claim that recorded it would be a claim about a simulator wearing
a hardware number's clothes. Splitting was the alternative to weakening — the
counts gate today and the time waits for `E0-D10`'s machine and for the gap above
to close.

**Not claimed.** That the killed code ran at ring 3. RFC 0032 put the frame's own
instructions in QEMU and this crate above them, so what is killed here is a
model's occupant. What the boot establishes instead is that the *mechanism* is
real against real memory: real records, real address spaces, real capability
tables paying a real account, a real channel carrying a real epoch. Neither half
claims the other's, and `CHAOS_GAP` is what keeps that honest.

## What would reverse this

**The gap closing, and the build says so.** When `driver.execute(` leaves
`kernel/src/blk.rs` — RFC 0033's reversal grep, and the two moves `E1-B08` names
as remaining — `cargo xtask chaos` goes red on its own declaration. At that point
the kill belongs in the boot: a scheduled driver component, a client submitting
through a ring at ring 3, and the same four claims asserted against a boot log
rather than against a trace. `CHAOS_GAP` becomes empty and its emptiness is the
evidence, exactly as `JOIN_GAP`'s will be.

**A component whose device state is its own.** The read-back is asked of
components whose state is behind them — a disk, an object store — and not of a
link, which holds none, or a display, whose resources are established by its
driver and die with it. The day a component of the third kind is deployed, this
decomposition is incomplete: *no client observes anything except added latency*
is false for a client whose operation depends on state the restart erased, and
the honest answer is a fifth claim about what a driver must re-establish on
restart rather than a wider threshold here. `sim/src/chaos.rs`'s `Chaos::of` is
where that choice is made and where the reader is sent.

**The interruption count going to zero.** `writes_interrupted_at_kill` is a
minimum and it is the row that keeps the third claim honest. If a workload change
moves it to zero — a wider window, a different phase order, a service time that
empties the occupant before a kill is due — then every zero beside it is a
statement about a run in which no write was ever at risk, and the answer is the
workload rather than the threshold. The reverse direction is the reversal that
matters more: the day a device model can reach a client's bytes,
`buffers_returned_torn` stops being a check that cannot fire, and the paragraph
above calling it one has to go with it.

**A bound that is not the ladder.** If the worst client wait stops being
explained by the declared backoff — if it sits well under it, or past it — then
either the kills are not landing on work in flight or something is waiting that
the policy does not account for. Both are findings and neither is a reason to
widen the bound: the bound is two declared quantities added together, and a
result outside it means one of the declarations is wrong.

**Three kills stopping being enough.** The workload takes three per component
because `user/store/manifest.toml` declares a budget of three. If a longer sweep
finds a failure the three-kill run does not, the answer is `E1-P03`'s sweep
across seeds rather than more kills at one seed — and the seam is already there,
because a chaos run is a function of `(seed, commit)` like every other scenario
in this crate.
