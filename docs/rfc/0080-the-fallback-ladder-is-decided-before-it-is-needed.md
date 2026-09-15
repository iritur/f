# RFC 0080: The fallback ladder is decided before it is needed, and a rung that declares nothing is not a rung

- Status: accepted
- Date: 2026-09-14
- Affects: `interface/src/ladder.rs`, `claims/0033-raster-cost-per-rung.toml`,
  `docs/design/ring-scene-boot.html` sections 08 and 10 (extended, with one
  narrowing recorded below), `xtask/src/main.rs`'s `ROUTES`, `TODO.md` E3-D04
  and E3-B02

## Decision

The renderer has **four rungs, named now, in this order**: the compute path of
section 08, a hybrid, an all-CPU raster, and a tessellating floor. They descend
in *fidelity* and not in hardware requirement, and the first three produce the
same image while the fourth does not — which is why the floor is fourth despite
asking least of the CPU.

| # | rung | what the machine must supply | cost | picture |
| --- | --- | --- | --- | --- |
| 1 | compute path | compute shaders, storage buffers, and a prefix scan across cooperating lanes | `compute_path_us_x100` | exact |
| 2 | hybrid | compute shaders; the scan of rung 1 may be unavailable or unusable | `hybrid_us_x100` | exact |
| 3 | CPU raster | a CPU, and any way at all to put a finished image on the screen | `cpu_raster_us_x100` | exact |
| 4 | tessellating floor | a fixed-function triangle pipeline | `tessellated_us_x100` | approximate |

A rung is a rung only if it declares all three of those columns: **what the
machine must supply** for it to start, **what it costs**, as the name of a row in
`claims/0033-raster-cost-per-rung.toml` rather than as a number, and **whether
it changes the picture**. A renderer that cannot state all three is not an
additional rung; it is an implementation detail of one of these four.

The rung is chosen **once, when a compositor starts**, by the compositor, from
what the backend says it can do, and is published in the component's own state
tree. It never promotes itself while running. A machine that satisfies no rung's
requirement is **refused a compositor** rather than given the lowest one.

The cost, stated in the decision rather than underneath it: **there is no rung
every machine can reach**, and this RFC declines to invent one.

## Context

`E3-B02`'s exit clause is *the fallback ladder is exercised on hardware that
cannot run the top rung*. That sentence cannot be written as a test until the
rungs have names, which is why `E3-D04` sits in front of it. But the sequencing
is not the argument; the argument is what happens if the order is reversed.

A fallback ladder decided *after* it is needed is decided in front of a machine
that has already failed. At that moment the question is no longer *what should
this system do when it cannot render properly* but *what can this box do*, and
those have different answers. The rung that gets written is whatever the
hardware in the room supports, the justification is supplied afterwards, and
nobody ever revisits it because it works on the machine it was written for. That
is how every "software fallback" in every compositor came to be the thing it is,
and it is a failure of sequencing rather than of engineering.

Writing the ladder now costs something real and it is worth naming: **nothing
here has been measured and nothing can be**, because there is no compositor
(`E3-B01`), no renderer (`E3-B02`), and no machine in this project's reach with
a GPU on it. So the four costs are registered as a `pending` claim with
thresholds and no numbers, in the manner `claims/0002` established, and the
targets predate any number anybody is later tempted by.

### Why these four, and not three

The obvious three are *the good one, a slow correct one, and something that
always works*. The hybrid is the one somebody would cut, and it is kept for a
specific reason that is falsifiable rather than decorative: section 08 prices
the coarse raster at 0.5 ms and says plainly what makes it possible — a
prefix-scan that *removes the inherently sequential part of rasterization*. That
scan is the one stage that depends on lanes cooperating, and it is therefore the
one stage a GPU can fail at while succeeding at everything else. Hardware with
compute shaders and no usable subgroup scan is not hypothetical; it is most of
the installed base older than about five years. A ladder without the hybrid
sends that machine from the top rung to an all-CPU raster in one step, which is
most of a factor of three by this ladder's own thresholds, for a defect in one
stage.

The hybrid is also the rung this RFC is least sure of, and the reversal section
says exactly what would delete it.

### Why not five

Two fifth rungs were considered and both are refused.

**A remote rung** — project the tree to another machine and let it render. This
is a real capability and section 12 already lists it as a *projection*, with its
own row in that table. It is not a rung: every rung on this ladder answers the
question *how does this machine turn a scene into pixels*, and the remote answer
is *it does not*. Putting it here would make the ladder two things, and the first
machine that lost its network would descend into a rung that had stopped
existing.

**Splitting the CPU rung** into a vectorised and a scalar variant. Refused
because it fails the three-part test above: a scalar CPU raster supplies nothing
different, changes the picture not at all, and differs only in a number. That is
an implementation detail of the CPU rung, and its cost belongs in the
distribution behind `cpu_raster_us_x100` rather than in a fifth row. A rung is a
decision the system makes; this would be a decision the compiler makes.

### Why the order is fidelity and not cost

This is the part most likely to be read as a mistake, so it is stated as a
refusal. Ordering by cost puts the tessellating floor second: triangles through
fixed-function hardware are fast, and on a great many machines faster than an
all-CPU raster. Ordering by hardware requirement puts the CPU rung last, because
it is the one that needs no GPU at all.

Both were refused, and for one reason: **the first three rungs are three ways to
compute the same image, and the fourth is a different image.** Conflation
artifacts where two shapes meet, sampled coverage instead of computed coverage —
these are visible, and they are the kind of visible that a user reports as *the
text looks wrong on this machine* rather than as *this machine is slow*. A
system willing to spend most of the latency chain to keep the picture exact,
and willing to change the picture only when there is nothing else left, is
making a defensible trade. A system that reorders those two because triangles
are cheap has quietly decided that correctness of output ranks below cost, which
is the opposite of the trade section 10 makes one layer down.

The consequence is that the ladder is **not monotone in what the machine must
have**, and that is not hidden: `Rung::below` is one step rather than a search,
and whoever holds the backend tries a rung, fails to start it, and asks for the
one beneath. No total order over hardware is ever computed, because none exists.

### What is *not* on this ladder

Section 10's per-frame effect degradation. That mechanism downgrades a blur
inside a frame that is already late and recovers on the next one; this one
chooses a rasteriser at a component's start. They share the word *fallback* and
nothing else, and a system that confused them would answer a missed frame by
changing renderers — the one response guaranteed to miss the next frame too.
Both must exist and neither substitutes for the other.

## Consequences

**What it makes easy.** `E3-B02`'s exit becomes writable before the renderer is:
a test that says *start on hardware that cannot run rung 1 and require rung 2*
needs only the names. `interface/src/ladder.rs` holds them with nothing else in
it, so it compiles for both architectures today and costs a compositor nothing
when one exists.

**What it makes true that was not.** Every rung owes a registered cost. This is
the half of `E3-D04` that is normally skipped — *it will be slower* is not a
cost, and four rungs with four adjectives is a ladder nobody can ever be shown
to have got wrong. `claims/0033` is four thresholds with no numbers behind them,
which is a weak claim and an honest one; what it forecloses is the strong and
dishonest version where the numbers arrive later and the targets are set to
whatever arrived.

The registry says the same thing in its own voice rather than taking this RFC's
word for it: `raster-cost-per-rung`'s entry in `ROUTES` is `Route::Unbuilt`,
naming `E3-B02`, and that route **refuses**. It was tempting to let the command
exit zero with an explanation, and that would have been the worse of the two —
a green run that measured nothing is precisely what a registry exists to
prevent, and the first person to quote it would be quoting a pass. The variant
is dead code the day `E3-B02` gives this claim a real route, and should be
deleted then rather than kept for a second occasion.

**What it narrows in `docs/design/ring-scene-boot.html`, recorded because a
reversal needs one.** Section 08 says the CPU raster segment *disappears
entirely rather than merely getting faster*, and `E3-B02`'s exit asks for it to
be zero, measured. This RFC puts a CPU raster back into the system. The two are
reconciled and the reconciliation is a narrowing rather than an agreement: that
claim is now a claim about **the top rung**, which is the rung the section 05
budget is written for, and it holds there unchanged. The lower three rungs are
explicitly outside that budget and say so in their own thresholds — the CPU
rung's is section 09's entire latency chain spent, and so is the floor's. Neither
is a multiple of the top rung, and this sentence used to call one *about two and
a half times* it, which was a multiple the claim states it is not and was wrong
in the third digit besides. So *the CPU raster segment is zero* stops being a property of the system
and becomes a property of the system **on hardware that runs rung 1**. Anybody
quoting the stronger form after this RFC is quoting something that was narrowed
on this date.

That last sentence was, when this RFC was first written, only in this RFC — a
narrowing recorded in the one file the people quoting the narrowed sentence do
not open. So section 08's paragraph now carries the citation inline, which is
this tree's convention (`deadline-all-the-way-down.html` cites RFCs 0058 to
0062 the same way): the sentence itself is unchanged, and what follows it is a
reference and a clause saying what was narrowed. No number in that document
moved, and none could have — changing a published number there instead of
changing the claim it renders from is on `CLAUDE.md`'s list of scars.

`TODO.md` E3-B02's exit still reads *the CPU raster segment of the budget is
zero, measured*, which is the stronger form and now needs the same qualifier.
Agents working in this tree may not edit `TODO.md`, so that edit is owed by
whoever owns the task list and is named here rather than left for the day
somebody runs E3-B02 against a sentence this RFC already narrowed.

**What it leaves in prose, on the record.** Two of the three things a rung must
declare are types in `interface/src/ladder.rs` — the cost metric and the
fidelity. The third, *what the machine must supply*, is the table in this
document and nothing else, because expressing it needs a vocabulary for what a
backend reports about itself and there is no backend. Inventing that vocabulary
now would be guessing at what a GPU driver will say, and a wrong guess frozen
into the crate everything else depends on is worse than an absent one: the
compositor would then have to satisfy it. `E3-B02` owes the type, and the day it
lands, a machine's rung becomes something a test can derive rather than
something a human matches against a paragraph.

**What it makes hard.** Four renderers is four things to keep correct, and three
of them are exercised only on hardware nobody has. The mitigation is the one
`claims/0033` already imposes — all four timed over one scene in one run, in one
build, once per machine, with the rung the compositor started at recorded —
which means the lower rungs are at least *built* everywhere even where they are
not chosen. It is not a substitute for running them, and this RFC does
not pretend otherwise.

**What it forecloses.** A compositor that promotes itself. The rung is chosen at
start, and a backend that comes back after a driver restart is answered by the
supervisor restarting the compositor — RFC 0008's *restart is the supervisor's
act*, reaching this layer through RFC 0073 and RFC 0076. The reason is frame
pacing: section 09's whole mechanism is the compositor computing backwards from
scanout using its own rolling p99 cost, and a rung change moves that
distribution underneath the estimator. One decision point, at a start, with a
restart as the only way to take it again.

## What would reverse this

**The hybrid is the rung with an expiry condition, and the condition compares
the two rungs to each other.** On each of the first three machines this ladder is
exercised on, record which rung the compositor started at, and time all four
rungs on one scene in one run — both of which `claims/0033`'s `[workload]`
requires of every run, and one run is one machine. The second half of that was
always there; the first was added when this condition was read back against that
file and found to be citing a requirement which was not in it, which is the
failure this section is otherwise about. The hybrid is deleted
if **no machine starts at it**, or if, on a machine that does start there,
`hybrid_us_x100` is not below that machine's own `cpu_raster_us_x100`. Then the
middle rung is a seam being maintained for its own sake — three rungs, this RFC
superseded rather than amended, and `claims/0033` loses a row.

*The earlier form of this condition was wrong, and it is worth leaving the
correction visible rather than quietly restating it.* It read: the hybrid missing
`hybrid_us_x100` while making `cpu_raster_us_x100` on the same scene. That
compares each rung to its own threshold, and the two thresholds are scaled from
different arguments on purpose — the hybrid's is twice the top rung, a budget for
the seam; the CPU rung's is section 09's latency chain. So a machine at 5.0 ms
hybrid and 7.0 ms CPU raster satisfied the old condition while the hybrid was the
faster of the two by two milliseconds, drawing the identical picture, and
deleting the rung would have moved that machine the wrong way. By this ladder's
own construction the CPU rung does everything the hybrid does and more of it on
the CPU, so it cannot be the better answer for a machine whose hybrid is slow at
the scan; a condition that says otherwise is comparing the wrong pair. A red
`hybrid_us_x100` on its own means *that machine* should descend, which is the
ladder working. Only the rung-to-rung comparison, repeated across machines, means
the rung should not exist. `claims/0033`'s `hybrid_us_x100_over` diagnosis states
the same distinction from the other end.

**The floor is refuted on the hardware it is for, and by nothing else.** The
floor is deleted if, **on a machine that cannot start rung 1**, its measured cost
is not below the cost that same machine's `cpu_raster_us_x100` records in the
same run, on the one scene. Then the floor is asking a machine for a fixed-function pipeline in order to
draw it a worse picture no faster than the rung above it already draws the right
one, and the ladder has three rungs. Repeated on every such machine this ladder
reaches, that is the condition; absence of such a machine is not the condition
being met, and the rung stands untested rather than confirmed until one exists.
`E3-B02l` is the task that takes the ladder to one.

*This condition, too, was wrong in its first form, and the correction is the same
correction one paragraph up.* It read: **the floor is refuted by its own
threshold** — `tessellated_us_x100` set at the top rung's target, so that a floor
which was not cheaper refuted itself. That is threshold-to-threshold comparison
again, the thing this section rejects by name for the hybrid, and for the floor
it is worse rather than equally bad. The floor's only reason to exist is hardware
that cannot run rung 1, so the red that deleted it could only ever be recorded on
hardware where it is never selected; and a uniformly slow machine reddens
`compute_path_us_x100` and `tessellated_us_x100` together, where the diagnosis
would have read the second as a refutation. Deleting the floor on that evidence
hands the machine it was built for the *refused a compositor* outcome this rung
exists to prevent — the reversal running backwards into the thing it was written
against. `claims/0033` now bounds that row by section 09's chain, the same bound
the CPU rung carries, because what a bound can ask of the floor on its own is
whether a machine running it still presents a frame. What it cannot ask is
whether the rung deserves to exist.

**The ordering is reversed by one measurement, not by an opinion.** Take the
three exact rungs and the floor on one machine and one scene, and ask an
observer who was not told which is which to say whether the floor's frame is the
same picture. If the answer is that it is — if analytic and sampled coverage are
indistinguishable on real content at real densities — then the floor is not a
different image, fidelity is not the axis, and the ladder should be reordered by
cost with the floor second. `E3-P05` is the machinery that would run this, since
its whole subject is assertions that replace looking at pixels.

**The refusal is reversed by a machine somebody actually has.** This RFC refuses
a compositor to a machine that satisfies no rung, on the grounds that a refusal
is answerable and a compositor missing every frame is not. The observation that
overturns it is a machine in this project's hands with a display, no compute, no
fixed-function pipeline, and not enough CPU for rung 3 — at which point *no
interface at all* is the wrong answer to give its owner, and a fifth rung below
the floor is owed along with an argument about what it is allowed to look like.
Nothing on the E3 hardware list is that machine today, which is why the refusal
is affordable now and is written down as a thing to re-examine rather than as a
principle.

**And the whole ladder goes if the pillar does.** Section 13 already names the
condition under which part III is abandoned — projection that cannot be made
incremental enough, measured by `E3-P03` as parity between a derived scene graph
and an authored one. This RFC is downstream of a compositor, not of the semantic
layer, so it survives that outcome; what it would not survive is section 08's
pipeline itself failing to hold, at which point rung 1 is not the top rung and
every threshold under it was calibrated against a number that turned out to be
wrong.
