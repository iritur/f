# RFC 0035: A boot-to-workload run is a pair, joined at the component set

- Status: accepted; amended by RFC 0036
- Date: 2026-09-03
- Affects: `sim/`, `xtask`, `.github/workflows/ci.yml`,
  `docs/TESTING-STATUS.md` L1, `docs/test-taxonomy.md` and its TOML twin,
  RFC 0030, RFC 0032, E1-P01, E1-P02, E1-P03, E1-P08

**Amendment, RFC 0036.** Step 3 below overstated what was built. The join it
describes was one-directional — it required the boot's components to be among
the simulator's and nothing the other way — and the tree it shipped into has a
boot that instantiates one module while the simulator runs every compiled
record. So the pair was *not* about one set, and the check could not see it.
RFC 0036 makes the comparison symmetric and requires the difference to equal a
declared list. Read step 3, and the sentence in the paragraph after it about the
log printing "each spawned component's `ContentId`", with that correction: a boot
prints one such hash per place it spawns, which today is one.

## Decision

E1-P01's exit says *a whole boot-to-workload run executes under simulation and
reproduces byte-identically from `(seed, commit)`*. RFC 0032 decided that the
simulator models the system **above** the frame, which makes that sentence a
claim about a **pair of runs over one component set** rather than about one
process. This RFC fixes what the pair is, and builds the join RFC 0032 named and
left unbuilt.

**A boot-to-workload run, defined.** At one commit:

1. `cargo xtask trace --hash` boots the real kernel in QEMU. The boot ends by
   spawning components from the compiled manifest records the loader hands it as
   modules (RFC 0030), and its log prints each spawned component's
   `ContentId` — one hash over the record and the image together. That log is
   hashed. This is the **boot** half.
2. `cargo xtask sim --hash deployment` runs the `deployment` scenario, whose
   component set is *those same component files*, read with `f-abi`'s own
   `Record::read` — one actor per record, each modelled by what its declared ring
   protocol says, driven by one client, against a workload the scenario states.
   Its artefact is hashed. This is the **workload** half.
3. `cargo xtask sim --join` requires the two to be about one set: it boots, reads
   the content hashes out of the log, asks the simulator which components it
   would run, and refuses if the first are not among the second. *(RFC 0036: it
   also refuses if the second are not among the first, except for a declared
   list of components the boot does not yet spawn — which is where the tree
   actually stands.)*

**Every artefact says what it covers.** A trace opens with a header — hashed
with the records, not printed beside them — naming what the run modelled
(components, the rings between them, the devices at the far end), what it did
not (the frame's own instructions), and the command that covers the other half.
The `deployment` scenario's header additionally names every component and its
content hash. A hash quoted in a year therefore carries its own boundary, and
cannot be quoted as covering the system.

**The seed is never in the artefact.** It is the other half of `(seed, commit)`
and it stays out of the hashed bytes. Putting it in would move the digest
whenever the seed moved, whether or not the run did — and the negative control
that requires a different seed to give a different answer would then pass with
the simulator having taken no different decision at all. A seed's evidence is
the run it produced.

**A protocol with no model is refused.** `deploy::MODELS` maps a declared ring
protocol to what the simulator puts under it. A component whose protocol is not
in that table is a refusal naming the protocol, not a component quietly modelled
as having no device. That is what makes the seam load-bearing rather than
decorative, and its cost is stated below.

## Context

RFC 0032 wrote the seam's location in bold and its absence beside it: *that join
is not built. Today a scenario is a table of integers and reads no manifest, so
the seam is a stated location rather than a shared artefact.* It also said why
that mattered — **an unbuilt seam cannot be shown to be in the wrong place** —
which is an argument for building it rather than for describing it better.

Four shapes were live.

**Leave it a sentence.** Two commands, one paragraph in three documents, and a
directory both of them happen to use. Rejected: a shared directory name is not
evidence. Nothing would notice if the simulator read a stale build, ran one
component fewer than the boot spawned, or ran a set the frame would have
refused — and each of those failures produces a green run with a stable hash,
which is the one result a reproduction check must never report as a pass.

**Have the simulator parse `user/*/manifest.toml`.** Rejected by RFC 0030, whose
whole content is that a manifest is compiled and not parsed at the point of use.
A second parser is a second belief about what a component is, and the belief that
matters is the frame's. The simulator therefore calls `Record::read` — the same
function, in the same crate, that a spawn calls — and inherits every refusal it
makes.

**Have the boot emit a deployment artefact both halves consume.** A third
format, a third writer and a third thing to keep in step, to answer a question
the boot log already answers: it prints the content hash of what it spawned, and
the trace job already hashes that log. The join reads the log for exactly that
reason — it is a check on *what the kernel did*, and a check that read the
component directory twice would agree with itself whatever the kernel had done.

**Merge the two halves into one command with one hash.** Rejected on failure
legibility, which is the same argument RFC 0032 made for keeping `trace` and
`sim` separate: `trace` red means the frame, `sim` red means the model, and
`sim --join` red means the two are no longer about one component set. A single
merged hash would say only *something moved*. It would also put QEMU inside the
host-only workload check, which every seed sweep at E1-P03 would then pay for.

## Consequences

**The `commit` half of `(seed, commit)` is now mechanical for one scenario.** A
component's identity covers its record *and* its image, so a changed manifest
field or a changed line of driver source is a different `deployment` digest —
whether or not the run behaves differently. That is the property the exit asks
for, stated as a mechanism rather than as a convention about tagging.

**And it costs nothing in reproducibility that was not already being paid.** The
`deployment` digest is now sensitive to the exact bytes of the component images,
so two runners that built different images would disagree. They already had to
agree: the boot log prints those same hashes, and the two-runner `trace` job has
been hashing that log since E0-P02. This RFC did not add that dependency, it
made it visible in a second place.

**Adding a component to `user/` now puts a red scenario in front of whoever adds
it.** Their options are to add a device model, or to add one line to
`deploy::MODELS` saying the component has no device below it. That is the seam
being load-bearing, and it is deliberate: the alternative is a simulator that
silently covers less of the system every time the system grows, while its
artefact goes on saying it ran the deployment.

*If that becomes a tax rather than a check* — several components whose protocols
genuinely have no device — the answer is a field in the manifest schema saying
so, argued in an RFC as RFC 0030 requires, and not a default in the table. A
default there would be the simulator deciding something a manifest should
declare.

**Every existing digest moved once.** The coverage header is part of the
artefact, so the nine scenarios stage one and stage two recorded have new hashes
as of this commit. Nothing recorded is broken by that: a seed binds to a commit,
and this is a commit. It is worth naming because it is the shape of the cost a
seed corpus will pay in future — E1-P03's corpus is priced in the commit it was
drawn from, exactly as E1-B11 priced it in the generator.

**`cargo xtask verify` boots once more.** The join is in the local gate rather
than only in CI, on the same argument that put `trace_check` there: the failure
it catches — the two halves quietly ceasing to be about one component set — is
invisible to every other check in the loop, and a check somebody has to remember
to run before pushing is not a check.

**What the pair still does not claim, stated so nobody has to infer it.** That
one process executed both halves: it did not, and no artefact says it did. That
the frame's own algorithms are covered: RFC 0032 lists what covers those. That
anything here is a statement about timing: the simulator's clock is fictional,
`proving-ground.html` is blunt about it, and L5 and L6 exist for that reason.

**E1-P02, E1-P03 and E1-P08 inherit a scenario with real components in it.** A
fault aimed at the `deployment` scenario is a fault aimed at the component set
this tree actually deploys, which is a better default target than a synthetic
one — and a minimised report from E1-P03 can name the component by the name its
manifest declares rather than by an actor index.

## What would reverse this

**The seam turning out to be in the wrong place** — which is now an observation
somebody can make, and was not before. The shape: a failure that reproduces on
both sides of the seam, with both halves green at one commit, and the system
still behaving differently on a machine. That would say the thing that
distinguishes one deployment from another is not the component set, and the join
would have to move to whatever it is instead.

**The boot ceasing to spawn from a fixed set of modules.** E1-B08 lands a
supervisor that can drive a control ring, and RFC 0008 is explicit that spawning
is then the supervisor's act rather than the frame's. If a supervisor begins
choosing a component set at run time — from a store, from a policy, from
something a user picked — then the compiled records the loader hands over stop
being the whole answer, and the join has to read whatever the supervisor read.
The migration is stated so it is a move rather than a rewrite: `deploy` takes a
set of modules and everything above it takes a `Deployment`, so a second source
of one is a new constructor and not a new shape.

**The `deployment` scenario becoming the only one that is ever run.** It is the
scenario that binds to the build, which makes it the slowest and the most
coupled; the other nine exist because they isolate one mechanism each and cost
nothing to run. If a sweep at E1-P03 finds that only this one ever finds
anything, that is evidence the others are not exploring what they claim to, and
the answer is to fix them rather than to delete them.
