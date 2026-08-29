# RFC 0006: Idle depth is computed from the reservation table

- Status: accepted
- Date: 2026-08-29
- Affects: `bench/`, `kernel/src/arch/x86_64/apic.rs`, `claims/0002-timer-jitter.toml`, RFC 0007, and the lifecycle decision when it is written

## Decision

Power is an output of admission control. It is not a subsystem with a policy of
its own, and F has no governor.

- **Idle depth is arithmetic.** Every hard-class consumer states a period in
  order to be admitted, so the frame knows the earliest deadline on each core.
  On entering idle it selects the deepest state whose worst-case exit latency
  fits inside the slack to that deadline. Nothing is predicted, because nothing
  needs to be: the set of things that will wake this core is the set of things
  that were admitted to it.
- **Frequency selection is the continuous form of the placement rule that
  already exists.** `deadline-all-the-way-down.html` section 02 places a task on
  the slowest core that still meets its deadline, because finishing just in time
  on an efficient core beats racing to idle on a fast one for most work.
  Frequency is that same computation at finer grain, on one core, once placement
  has happened.
- **Device power is a component lifecycle state.** Wake sources and resume cost
  are declared and admitted like any other reservation, and a device whose
  resume cost cannot be met is not permitted to enter that state. Admission
  refuses it, in the admission domain of RFC 0010, rather than a driver
  discovering it on the way back up.
- **Suspend is a generation-scoped transition that quiesces by rings**, not a
  per-driver callback chain. Nothing is delivered asynchronously here either:
  quiescing is draining, and a component learns the system is suspending by
  reading a ring entry at a polling point like it learns everything else.

The whole of it rests on one sentence, and it is worth stating so it can be
attacked: **Linux's governors predict because Linux cannot know, and F's
admission control means F knows.** A predictor is what is built when the
information is absent. This design's cost is making the information present, and
having paid that cost, continuing to predict would be strange.

## Context

Energy is in the first paragraph of the architecture document's thesis and was
owned by no subsystem across all five design documents. The gap register calls
that half the thesis, and it is the clearest instance of the rule that a
headline claim names the subsystem that owns it — the claim was made, nobody
owned it, and it went missing without anyone deciding to drop it. This RFC gives
it an owner, and the owner is admission control rather than a new subsystem.

Two things made now the time. The energy counters land at M2, which is where
`bench/` stops reporting `joules_per_op` as absent, and a policy designed after
the counters are wired is a policy designed against whatever the first machine
happened to do. And the arithmetic needs a reservation table to read: RFC 0007
defines one this epoch, which is why this RFC follows it rather than preceding
it.

One thing in the tree already turns on this decision. The kernel spins between
ticks and does not halt, and `apic::wait` says why: halting would put the
idle-exit path inside every jitter sample, and how deep a core may idle was a
policy that did not exist, so a halting measurement would have been measuring a
decision nobody had made. This RFC is that decision. It does not change the
kernel today — E0-D06 is design, and the implementation is E5-B07 — but it moves
the spin from *undecided* to *decided and not yet implemented*, and it fixes what
the first change to it will cost: claim 0002's `best_case_bimodal` diagnosis
already says that a kernel which halts is measuring something else.

Three alternatives were live.

**A governor with a predictor** — the menu and teo family, which observe recent
wake patterns and guess the next one. Rejected, and worth being precise about
why: it is the correct answer when the deadline set is unknown, and it is very
good engineering for that case. Here the deadline set is known by construction.
Adopting a predictor would be importing the solution to a problem this
architecture spent admission control to not have, and it would make the energy
claim a claim about a heuristic rather than about arithmetic.

**Race to idle everywhere** — run at the highest frequency, finish, sleep deep.
Rejected because it is a policy that ignores slack, and slack is the one number
this design refuses to collapse. Section 02 already states the result it gets
wrong.

**Leave it to firmware.** Rejected because firmware cannot see the deadline set
either. It is the same prediction problem one layer down, with less information
and no way to be argued with.

## Consequences

**Easy.** The deepest safe idle state becomes computable rather than guessable,
and the computation is bounded by the consumer's own slack, so a hard-class
component gets an energy story that cannot cost it latency. Energy acquires a
subsystem to name in a claim, which is what the claims registry needs before an
energy number can be published at all.

**Hard.** This inherits every cost in RFC 0007, because the arithmetic is only as
good as the table it reads. It also needs a measured exit-latency table per
platform and per state: firmware-reported exit latencies are famously
optimistic, and a computed depth resting on an optimistic number is worse than a
predictor, because it is wrong with confidence. Measuring that table is real
work and it belongs to E5-B07 rather than being assumed here.

**The honest limit.** Soft, batch and idle work states no period and is admitted
against no deadline, so it contributes nothing to the arithmetic. On a machine
running only soft work there is no earliest deadline to compute against, and
this RFC does not pretend otherwise: the frame idles on the shallowest state its
observed wake pattern justifies and *records that it is doing so*, so the
difference between a computed selection and a fallback is visible in the
measurement rather than hidden inside it. A claim collected under the fallback
is not a claim about this decision.

**Forecloses.** A tunable governor, and the tuning culture that comes with it.
Per-driver suspend and resume callbacks. And any energy claim that turns out to
be a claim about how well a predictor was tuned on the day.

## What would reverse this

A measurement showing a good predictor lands within noise of the computed
selection. On the E5 machine, under a mixed hard and soft workload, compare
joules per operation and p99 lateness under computed selection against a tuned
menu-style predictor. If the predictor matches on energy within noise and does
not lose on lateness, then the computation is buying complexity rather than
joules, and idle selection belongs below the frame as a pluggable policy like
every other scheduling policy that is not the hard class.

The other direction reverses it too. If measured exit latencies on real hardware
are so poorly characterised that the computed depth has to carry a safety margin
large enough to swallow the difference between states, the arithmetic has
degenerated into a conservative constant wearing a derivation. That should be
said, and the constant published, rather than the derivation kept for the look
of it.
