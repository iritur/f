# RFC 0123: A timeout is a fate a supervisor names, and it restarts where a fault does

- Status: accepted
- Date: 2026-09-24
- Affects: `abi/` (`control.rs` — `cause` gains a fifth word, `TIMEDOUT`, so
  `cause::known` and `cause::label` gain an arm and the test pinning the first
  *unnamed* cause moves from five to six; `manifest.rs` — `Record` gains
  `restarts_after_cause`, which becomes the policy table, and
  `Record::restarts_after` stops being one and becomes a translation into it),
  `user/supervisor/` (`policy.rs` — `decide` takes a cause word rather than two
  booleans, and gains `Liveness` and `fate`; `component.rs` — the drain reads
  the cause off `Cqe::ext` instead of recording a boolean), RFC 0008 (whose
  three ways a component ends become four, and whose *restart is the
  supervisor's act* gains the case it was weakest on), RFC 0013 (the judgement
  is taken over what an occupant published about itself), RFC 0076 (the
  supervisor's memory of what it last read is one more number the frame stores
  and does not interpret), and `TODO.md` task `E3-B05e`, which this pays part
  of and which this RFC says plainly it does not close.

## Decision

**A component that has stopped making progress ends by a fate of its own, that
fate is named above the frame, and a manifest that restarts after a fault
restarts after it.** Three parts, each disagreeable on its own:

- **`f_abi::control::cause::TIMEDOUT` is a fifth cause**, carried on a peer-gone
  notice like the four before it, with the occupant's own count of abandoned
  frames in the detail half. It is not a `FAULT` with a reserved vector and not
  a `STOPPED` with a flag.
- **The frame does not compute it.** Every other cause is something the frame
  observed — an exception it took, a door call it answered, a deadline it
  compared. This one is a judgement over two numbers an occupant published about
  itself, and `f_supervisor::policy::fate` is the only function in this tree
  that makes it. The frame's part is to carry the reading across and the word
  back, the way it already carries a restart budget it never reads.
- **It restarts where a fault does.** `restart = "on_fault"` now means *restart
  this when it fails*, and a timeout is a failure of the occupant to do what it
  was admitted to do. `restart = "never"` still leaves the place; a stop still
  never restarts, because a stop is the supervisor's own decision.

The rule the supervisor applies is the weakest one that uses both readings: the
occupant's abandoned-frame count has risen since this supervisor last looked
**and** a wait is outstanding now. There is no tolerance and no duration.

## Context

`E3-B05e`'s exit is *a timeout is a named fate, the supervisor decides it, and a
boot shows a compositor restarted for one — RFC 0008 is paid, so the policy that
decides is above the frame rather than inside it.* Three things were true when
that was picked up.

**The vocabulary could not spell it.** `f_abi::control::cause` had four words and
a restart policy is written in terms of exactly that distinction — RFC 0008 says
so in the module: *a client that cannot tell them apart cannot tell a crash from
a planned shutdown.* A stuck component arriving as `FAULT` would be a supervisor
restarting a wedged compositor with a log line saying it had crashed, and a
reader of that log would go looking for an exception that never happened.

**The supervisor could not read the one it already had.** `policy::decide` took
`faulted: bool, exited: bool`, and the only caller in the tree passed
`true, false` for every death it was told about, under a comment saying that an
occupant the frame tore down is a death this supervisor treats as a fault. That
comment was the whole classification. The cause has been in `Cqe::ext` since RFC
0008 and nothing had ever read it — so *the supervisor decides from the
manifest's restart policy* was true of a policy being fed a constant.

**The reading existed and nothing acted on it.** `user/compositor/src/waits.rs`
(`E3-B05f`, 2026-09-24) publishes *waits outstanding*, *the value last reached*
and *frames abandoned* into the component's own subtree, and says in its own
header that deciding what to do about them belongs to this line. It also settles
the hardest question this decision would otherwise have had to answer: a
component has no clock (RFC 0004), so *how long has this wait been outstanding*
is not a question anything at ring 3 can ask. The compositor already does the
timing in the only currency it has — its own frame deadline against its own p99
estimate — and what a supervisor needs is that answer, not a stopwatch of its
own.

Three alternatives were live.

*A fault with a distinguishing detail.* Cheapest: no new word, no new arm. It is
worse in the one place that matters, because `detail` is the processor's word
for a fault, and a vector this architecture does not define is a value only the
frame that wrote it can read back — R04 in reverse.

*A fourth `restart` policy value, `on_fault_or_timeout`.* Honest, and it is the
reversal named below rather than the decision, because it prices a schema bump
and a rebuild of every component file against a distinction no workload in this
tree has asked for. Every manifest here that carries `on_fault` was written to
mean *restart this when it fails*.

*A threshold — n abandoned frames within m ticks*, matching the restart budget
one field over. Rejected because neither number exists. Nothing in this tree
measures how often a healthy compositor gives a frame up, so an `n` chosen now
would be a constant with an argument behind it instead of a measurement, which
is precisely what `claims/README.md` refuses to let a number be. The rule that
went in uses both readings and no magnitudes.

## Consequences

**What it makes easy.** A supervisor can now tell four deaths apart where it
could tell none, and the fifth is available to any occupant that publishes a
liveness reading — `Liveness` names *a synchronising occupant*, not the
compositor, so the second one costs no widening. A fate carries the reading it
was taken on, so a decision that reaches a log can be checked against the
occupant's own tree rather than believed.

**What it makes hard, and it is a real cost.** A manifest can no longer express
*restart after a fault and ride out a timeout*. A component whose stuck frame is
better than its cold start — a long-running one holding client state it cannot
rebuild — is now restarted where it would previously have been left alone. That
is a policy change to every existing manifest carrying `on_fault`, applied
without those manifests being edited, and it is stated here rather than
discovered by whoever notices a component being recycled.

**What it forecloses.** Nothing on a wire: `TIMEDOUT` takes the next free cause
ordinal and the notice's shape is unchanged. `Record::restarts_after` is kept —
`sim/src/chaos.rs` models a restart storm through it — and is kept as a
*translation* rather than as a second `match`, because two tables are two
policies and the one that gets audited is never the one that ran. The test that
holds that is a cross product of every flag pair against every policy, and it is
the only thing in the tree that exercises both flags set at once.

**What this does not pay, said plainly.** `E3-B05e`'s exit has three clauses and
this RFC lands two of them. There is no delivery and no boot. `Liveness` is a
value nothing constructs on a running machine: the two numbers are in the
occupant's state tree, the frame is what can read that tree, and the frame that
would copy them onto a supervisor's board is `kernel/src/component.rs`, which
this wave did not hold. A board field written by nobody is a field that can be
wrong with nothing to notice — which `user/supervisor/src/routing.rs` refuses
one layer over — so none was added, and the crate says so in its own module
header rather than leaving a reader to find an unused type. The diff that closes
it is two words per row, copied and never interpreted, and a boot that drives
them.

## What would reverse this

**A workload that wants a timeout ridden out.** A component that holds state a
restart destroys, whose stuck frame is measurably better than its cold start.
That is a fourth `restart` value, a schema bump, and a row in
`docs/manifest.md`; it is not written now because it would be a field nothing
tests. Watch for it arriving as a special case in a supervisor first — a
component name compared against a list — which is how a missing declaration is
always spelled the first time.

**A measurement of how often a healthy pipeline gives a frame up.** `E3-B05d` is
the line that establishes what a frame costs on a named machine. The day it says
that one abandoned frame is ordinary, the rule here stops being
*abandoned-count-rose* and becomes a declared tolerance, and `fate` takes it as
an argument rather than hard-coding the weakest form.

**A renderer.** `waits.rs` names this one from the other end and it reaches here
too: the compositor abandons a frame today because nothing in this build draws,
so the value it promised for a frame that did not fit is never reached. The day
`E3-B02` rasterises, the landing comes from the rasteriser and a frame that
missed its deadline may still reach its value — at which point *a wait
outstanding on a frame that did not fit* stops being the definition of stuck and
has to be re-derived against what the renderer actually does.

**A second thing in the frame that reads these two numbers.** The whole weight
of *the policy that decides is above the frame* rests on `kernel/` copying and
not comparing. RFC 0076 already records that seam for the restart budget; this
adds a reading to it. The day anything in `kernel/` compares an occupant's
outstanding waits against anything, RFC 0008's objection has landed and this RFC
is the record of what was lost.
