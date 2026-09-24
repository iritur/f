# RFC 0121: A remote presents the declaration, and decides what the link does not carry

- Status: accepted
- Date: 2026-09-24
- Affects: `semantic/src/remote.rs`, `semantic/src/tree.rs`,
  `interface/src/example.rs`, `interface/src/node.rs`; RFC 0104's *a projection
  is the layer allowed to choose words*; RFC 0083's *a second decoder*;
  `E3-B06h`, and `E3-B06g` by the same argument

## Decision

`f_abi::semantic` carries identity, place, order, role, intent and state, and it
carries no layout, no style and no content. A receiver therefore cannot
reconstruct the declaration its peer wrote, and it has two honest options:
present nothing, or **decide the rest itself**. This RFC takes the second and
names what is decided.

The far side derives one thing from each role — *how are this node's children
arranged* — in a single exhaustive match with no wildcard arm,
`f_semantic::remote::presentable`. Everything else about the geometry comes from
the far side's own theme at the far side's own density, through
`f_interface::token::Resolved::span` and `f_interface::solve::Demand::of`, which
are the same two functions the local path uses. There is no remote-only solver
and no remote-only projection stage: `restate` produces
`f_interface::node::Node` values and `f_semantic::emit::project` draws them.

Two roles are **refused by name** rather than presented, and the refusal fails
the frame: a `Role::Canvas`, whose pixels are drawn by application code on the
application's machine and which no opcode in this format carries, and a
`Role::Image`, whose content is a `CapRef` — a slot and a generation in the
*declaring* peer's capability table, which the far side does not hold. The
refusal names the node, the role and the reason.

A refresh rate reaches the presentation and nothing else. `Pacer` decides when
what is held is shown; the link is not paced, because a frame that is folded has
already been applied and the next presentation shows it.

And one consequence for the application: the settings-panel declaration moves out
of `interface/src/node.rs`'s test module into `f_interface::example`, public, so
that *one unmodified application* is a property of a call rather than a claim in
a report.

## Context

`docs/design/ring-scene-boot.html` part III lists a remote client among the
projections in one line — *the tree itself, projected on the far side at its
density and refresh rate; not an encoded pixel stream* — and `E3-B06h` turns that
into an exit with four clauses. Three of them were about arithmetic and one was
about a gap nobody had named.

`abi/src/semantic.rs` had already named it, in the comment on `Closed`: *`Unit`
and `Flow` are closed enums of the same vocabulary and are absent, because no
entry in this format carries one … a node's flow belongs to a layout entry this
format does not yet have.* That comment was written about which enums need
version negotiation. Read from the receiver's side it says something larger: a
peer's constraints do not cross, so the layout the far side solves is **not the
author's layout**. `E3-B06f` did not meet this, because its projection is handed
the author's own `&[Node]` on one machine.

The alternatives were live and are recorded rather than implied:

- **Refuse to present without constraints.** Honest, and it makes the remote
  client useless until a layout entry exists — which is an opcode, a version
  negotiation and a receiver bound, none of which `E3-B06h` is scoped to add.
- **Send the pixels.** The thing the design document names and rejects, in the
  sentence this task exists to make true.
- **Put a default constraint on the vocabulary**, so that every node carries a
  flow whether or not its author wrote one. This is the tempting one and it is
  worse than it looks: it moves a *projection's* guess into the *declaration*,
  where every other projection then reads it as something the author said.
- **Decide per role at the receiver, in one place, with the decision written
  down.** What was done. RFC 0104 already established that a projection is the
  layer allowed to choose words; the words a screen reader says are not in the
  declaration either, and this is that permission applied to arrangement.

The refusals had a third option too — present the role as something else, a
placeholder rectangle where a canvas was — and it is refused for the reason the
exit gives: a placeholder is a frame whose sender believes it contains a canvas.

## Consequences

**Two remotes agree only if they share this table.** That is the cost, stated
plainly. The same declaration presented by two different builds can be arranged
differently, and nothing in the protocol says it should not be. What the protocol
does guarantee is what it guaranteed before: the same *structure*, the same
identities, the same roles, the same intents.

**The author's constraints are not honoured remotely.** A node that declared
twelve ems is twelve ems locally and a theme floor remotely. This is a real loss
and it is the argument for the layout entry the reversal below names.

**Style and content do not cross either**, and the consequences differ. Content
has nowhere to go in any projection yet — `E3-B06f` says so and this changes
nothing. Style does have somewhere to go, so an entry declaring `field.danger` is
painted in the remote's ordinary ink for an entry. The information is not lost:
`StateSet::INVALID` crosses. What is missing is a rule turning a state into a
paint, and inventing one here would have been a second place a colour is chosen.

**The comparison the exit asked for lands where it should.** One declaration of
the settings panel is 24 entries and 2 880 bytes — the entries were counted at
the encoder and the same pairs were decoded by the receiver — against 4 096 000
bytes for one uncompressed 1 280 by 800 frame and 10 108 800 for one 1 080 by
2 340 frame: 1 422 and 3 510 times smaller, and 85 333 times for one still second
at sixty hertz. **The pixel side is uncompressed**, which is the upper bound and
not a fair fight; this tree has measured no compressed remote protocol, so the
honest sentence is *three orders of magnitude smaller than the raw framebuffer,
and an unmeasured amount smaller than a compressed one*. No `claims/` row is
taken, and the judgement is recorded here rather than left to be re-taken: a
claim in this registry has a baseline, a workload and a machine, and these
numbers have none of the three — they are arithmetic over constants that a unit
test holds exactly, which `CLAUDE.md`'s own scar says belongs in `verify` as a
file read rather than in a registry as a measurement. The day a compressed
protocol is measured against this on a real link, that number has a baseline and
a machine and is owed a row.

**A shipped declaration is a new kind of artefact in `interface/`.** The crate
that says it is *deliberately the smallest thing that can* express the thesis now
also ships an application. The rule that keeps it honest is written at the top of
the module: nothing may be added to that declaration for a projection's benefit,
and a projection that cannot present it says so. `E3-B06g` inherits both the
fixture and the rule.

**The receiver's bound is the frame's bound.** Fifteen nodes and seven states
plus a handshake and a commit is twenty-four entries, of which twenty-three are
staged — one under `f_semantic::tree::FRAME_ENTRIES_MAX`. The settings panel very
nearly does not fit in one frame, and a sender that split it across two would owe
an answer to what the receiver presents in between.

## What would reverse this

**A layout entry in the semantic format.** The reversal `abi` already anticipates:
an opcode carrying a `Flow` and a constraint, at which point `Flow` joins
`Closed`, the far side reads the author's arrangement, and `presentable` answers
only *can this be presented* — the half that is about a machine boundary rather
than about a missing field. Watch for it arriving as a hint on `DeclareNode`,
which is how it will be spelled the first time and which would give it no version
of its own.

**A measurement in which the ratio stops mattering.** Against a compressed stream
on a real link the pixel side falls by one to two orders of magnitude. If the
remaining margin does not pay for the far side's arrangement being its own, the
argument for this projection is weaker than this file implies, and the honest
answer is a hybrid: the declaration for structure, a compressed surface for the
`Role::Canvas` this build refuses.

**A transfer that moves what a `CapRef` names across the boundary.** Then
`Role::Image` stops being a refusal and becomes a presentation, and the variant
`Unpresentable::Media` is what has to be argued away.

**Two remotes that must agree pixel for pixel.** A test harness comparing two
machines' screenshots, or a design review conducted over a remote link, makes
*the far side arranges* a defect rather than a feature. That is the day the
layout entry is owed, and this RFC is what it reverses.
