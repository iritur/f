# RFC 0077: The semantic vocabulary is closed, and a canvas that declares nothing is a defect

- Status: accepted
- Date: 2026-09-14
- Affects: `interface/src/node.rs`, and through it `interface/src/canvas.rs`
  (`E3-D02`) and `interface/src/token.rs` (`E3-D03`);
  `claims/0034-canvas-escape-rate.toml`, which is this decision's own reversal
  condition registered as a target;
  `docs/design/ring-scene-boot.html` sections 11 and 13, one of which this
  departs from by one field; `TODO.md` `E3-D01`, and the exits of `E3-B06` and
  `E3-P05` which are written against these types

## Decision

The semantic vocabulary is a **closed** Rust enum of twenty-two roles. There is
no `Role::Other`, no `Role::Custom`, no variant carrying a string, no reserved
extension range, and no `#[non_exhaustive]`. A role that does not exist cannot
be invented by an application: it is added by an RFC or it is not added.

The loophole that would make that meaningless is `Role::Canvas`, and it is
closed by the same move rather than by a convention. **A canvas that declares no
children is a defect, and the tree does not pass `check`.** A canvas may keep
its pixels; it may not keep its content. Section 13 says every escape into
canvas should be read as a bug report about the vocabulary rather than as an
application's choice, and `Defect::EmptyCanvas` is that sentence made
mechanical.

Two smaller decisions follow the same principle and are part of this one. The
rule *style is semantic tokens, never values* has exactly one hole — the token
name — so `TokenName`'s grammar refuses a value spelled as a name at
construction. It takes **two** clauses to do that, and the first draft of this
paragraph claimed one was enough: *begins with a letter* refuses `12px` and
`50%` and lets `ff0000` straight through, because `f` is a letter. The second
clause refuses a name spelled entirely from the hexadecimal alphabet, which is
the one that actually keeps colour literals out, and it costs `facade` and
`decade` — stated in the type's own documentation rather than discovered by
whoever first wants one. And the vocabulary is joined to its neighbours **by
`NodeId`**, never by widening `Node`: `canvas.rs` holds a clip's time range in
its own structure keyed by identity, because a vocabulary that is extended by
growing its node is not closed in any sense that matters.

A third refusal was added during review and belongs in the decision rather than
in a comment: **a node may not hold `NodeId::UNNAMED` as its own identity**, and
`check` reports `Defect::Unnamed` when one does. `NodeId::UNNAMED` is zero, it
is the parent of a root, and it is the value `NodeId`'s documentation says no
relation may carry. It was not enforced anywhere. A tree whose node was
identified by zero passed `check` clean, and then `find` answered that node for
every edge that meant *nobody* — so a `LabelledBy` pointing at nothing resolved
and the tree was accepted. One refusal closes both: with no node holding zero,
an edge to zero is a `Defect::UnknownRelation`. This is the same move as
`EmptyCanvas` — a sentence that was true only as long as nobody wrote the
counterexample, made mechanical.

No number in this decision has been measured. Nothing in `E3` has been
measured. The canvas-escape rate named below is a target on record, not a
result, and `claims/0034-canvas-escape-rate.toml` is where it is on record —
registered with no workload and a reproduction command that **refuses** rather
than exiting zero, because a green command that measured nothing is the failure
the registry exists to prevent.

## Context

Section 11 of `ring-scene-boot` gives the `Node` struct and four rules that are
load-bearing rather than stylistic: intent is a capability reference and not a
callback, layout is constraints and never coordinates, style is semantic tokens
and never values, identity is stable and author-assigned. Section 13 gives the
three hard cases and says which of them could void the pillar — and the one it
says sank every predecessor is the canvas.

What neither section gives is a size. Section 13 states the two failure
directions instead: too small and applications escape into canvas, which voids
the thesis; too large and the vocabulary becomes another sprawling markup
vocabulary whose meanings are ambiguous in practice, which is the failure that
overtook accessibility annotation on the web. Both failures end in the same
place — a projection that cannot rely on what a node says — and they arrive from
opposite directions, so a decision has to pick a point and say what would show
the pick was wrong.

Twenty is what three interfaces cost, and two more are held in reserve. A
settings panel, because it is where *layout is constraints* and *style is
tokens* are most tempting to break. A file browser, because it punishes a
vocabulary with no honest notion of a column. A timeline, because it is the
canvas case — the one section 13 says sank every predecessor. Each is built in
`interface/src/node.rs`'s tests as an actual tree of nodes, each is checked, and
each asserts the exit rather than describing it. Between them they reach twenty
of the twenty-two roles; `List` and `Text` are unreached, and both are roles a
fourth interface — a menu, a document viewer — reaches for immediately.

### The departure from section 11, stated because a reversal needs one

Section 11's struct has no parent, no children and no structure of any kind,
because there the shape of the tree is stated by the transport: `DeclareNode`
says where a node goes, and the struct is only what a node *holds*. This crate
has no transport and will not have one until the compositor exists, and a
vocabulary you cannot build a tree out of cannot be shown to express three
interfaces — which is this decision's exit. So `Node` carries a `parent` and a
`&[Node]` is a tree.

This is an addition rather than a contradiction, but it is the kind of addition
that becomes a contradiction later, so the reversal condition is below.

### What was refused, and why each loses

**An open vocabulary — a role that carries a name.** The obvious design, and the
one every predecessor shipped. It loses on the evidence section 13 already
cites: the web's accessibility vocabulary did not fail because it had too few
roles, it failed because the roles it had meant different things to different
authors and a consumer could rely on none of them. A role spelled as a string is
a role with no owner and no definition, and two applications that both write
`"slider"` have agreed on nothing. The closed enum is not a restriction bolted
onto an open design; it is the only version of the design in which a projection
can be written once.

**`Role::Other`, with a mandatory label.** The humane compromise: keep the
vocabulary closed, but let an application say *this is a thing you have not got
a word for, and here is what I call it*. Refused, and this is the refusal the
exit of `E3-D01` names directly. The label is read by nothing. A screen reader
cannot linearise it, an agent cannot invoke it, a remote client cannot lay it
out, and a test cannot assert against it — so `Other` is exactly an opaque
rectangle with a caption, which is the failure the canvas case is about, arriving
through the vocabulary instead of through the renderer. And it would be used: it
is always easier than arguing for a role, which means the pressure that is
supposed to improve the vocabulary would be routed away from it permanently.

**A reserved extension range, or a vendor prefix.** The industrial answer —
leave roles 128 upwards to applications, or accept `x-` names. Refused for the
reason above with a second one on top: a namespaced escape does not stay
namespaced. The first widely-used application's private role becomes a role
every projection must handle in practice while remaining a role no projection is
required to handle in principle, which is worse than either a closed vocabulary
or an open one, because the obligation is real and unwritten.

**Start large: adopt an existing eighty-role vocabulary.** It would be faster,
and every role in it has been argued about by somebody. Refused because the
argument they were subjected to was *does a toolkit have a widget for this*,
which is not the question here. The question here is *can a projection do
something with this that it could not do without it*, and most of those eighty
fail it: a link is a command whose intent is a navigation capability, a progress
bar is a number with no intent, a toolbar is a group whose children are
commands, and each of those extra roles buys a projection nothing while giving
two authors two ways to say one thing. The bias is deliberately towards the
error that is fixable: a missing role is an RFC, a redundant role is forever.

**`#[non_exhaustive]` on the enum.** The attribute that would let the vocabulary
grow without breaking downstream builds. Refused because it *is* the escape
hatch, wearing a build-compatibility argument. It forces every projection ever
written to carry a wildcard arm, and the wildcard arm is where a node nobody
understood goes to become a grey box. Without it, a twenty-third role is a
compile error in every projection, which is the correct notification channel:
the person who must decide what a new role looks like in a screen reader is told
by their compiler rather than by a user.

**Roles for things that are appearances.** `Icon`, `Link`, `Toolbar`, `Menu`,
`Alert`, `Progress`, `Meter`, `Tab`, `Card`, `Fieldset`. Each was considered and
each reduces: an icon is an image or a style token, a link is a command, a
toolbar and a card and a fieldset are groups, a menu is a list of operable items,
an alert is a status with `INVALID` or a token, and a progress bar and a meter
are both a `Number` with no intent — which is the `intent` field earning its keep
twice, because *read-only* is already spelled there and a second spelling would
be a fact stated in two places that can disagree.

**A `focused` state, and a `hidden` one.** `focused` is refused because focus
belongs to the projection and not to the declaration: one tree presented at once
to a display, a screen reader and an agent has three focuses, and a field on the
node could hold only one of them, wrongly. `hidden` is refused because a node
that should not be presented should not be declared, and because the decision is
not the application's to make — a screen reader may usefully present what a
display elides. The case that looks like hiding is a collapsed subtree, which is
the absence of `EXPANDED`.

**A time range on `Node`, for the timeline.** The direct way to answer section
13's question — *can an agent select the clip between 4.2 s and 6.8 s on track
3* — is to give `Clip` a start and an end. Refused, and this is the refusal that
makes closedness mean something: a vocabulary that grows a field whenever a new
kind of content appears is an open vocabulary with extra steps. The arrangement
is `canvas.rs`'s, keyed by `NodeId`, and identity is the join. That the join
works at all is a consequence of *identity is author-assigned and stable* — the
rule pays for itself here before it ever pays for a test.

The refusal splits section 13's question in two, and this decision answers one
half: *the clip on track 3* is this tree's — three named tracks under the
canvas, each with an occupant, every one addressable and operable — and
*between 4.2 s and 6.8 s* is `E3-D02`'s, whose own exit in `TODO.md` names
*times* among the four things it owes. The fixture had two tracks while this
RFC quoted the question, which a review caught; it has three now, and the test
is named `a_timeline_is_expressible_as_far_as_nodes_go` for the half it covers.
The exit of `E3-D01` asks that a timeline be *expressed*, and what is claimed
here is exactly that and no more: a projection can be handed this tree and name
every track, clip, marker and selection in it without a pixel. It cannot answer
*which clip is at this instant* until `canvas.rs` exists, and nothing in this
decision should be read as saying it can.

**Borrowed text, and `f_abi::cap::Handle`.** `Node` owns its text rather than
borrowing it because the system owns the tree: a node outlives the call that
declared it, and a borrow would tie the whole semantic layer's lifetime to the
frame that produced one label. The intent is a `CapRef` newtype rather than
`f_abi`'s handle because a handle is an index into one component's table and
means nothing to the projections that are not that component — and because
`interface/` takes no dependencies so that this layer can be abandoned without
taking anything else down. It is a `u32`, matching `Handle`'s width, so that the
day the tree crosses a ring the translation is a rename rather than a
re-encoding.

## Consequences

**What it makes easy.** A projection is total. `E3-B06` asks for four
projections from one declaration with no projection-specific application code;
each of those four is a function over a closed enum, and a projection that
compiles has decided what every role looks like in its medium. `E3-P05` asks
that node identity survive a deliberate visual redesign; identity is assigned by
the author and derived from nothing, so it does. Both exits are now exits
somebody can write code against rather than exits that need the vocabulary
designed first.

**What it makes true that was not.** An application cannot ship a control the
vocabulary has no word for. The honest reading of that is that some applications
will not port, and the honest response is that this is the measurement rather
than the problem: an application that cannot be expressed is the corpus telling
us the size was wrong, which is exactly what section 13 asks for.

**What actually enforces it, which is not what either of the first two drafts of
this RFC said.** Closedness across the system is bought by the compiler — a
twenty-third role is a compile error in every projection, and that claim stands.
Inside `interface/` it was not, and it took two attempts to make it so. Both
failures are worth the paragraph, because both are the ordinary way an
enforcement claim goes false.

*The first.* `Role::ALL` was a hand-written array, every test in `node.rs`
iterated it, and a loop over a list cannot see what the list omits: a
`Role::Other` added to the enum, given an index, a name and the exhaustive-match
arms the compiler demands, and left out of `ALL` and `COUNT`, passed every test
in the module — including the one named for this decision's exit.

*The second.* The repair was a test: `all_is_every_variant_the_enum_has`, an
exhaustive match with one `const` arm per variant, each asserting that `ALL`
held the variant its arm named. This RFC then said a twenty-third role had two
endings and neither was green. **That sentence was false, and it is the sentence
the repair was accepted on.** There was a third ending and it was green:
`Role::Other => {}`. An exhaustive match demands an *arm*, not an arm that says
anything, and an empty arm is precisely what somebody adding a role in a hurry
writes.

*What is there now.* Not a third test. `interface/src/node.rs` declares the
vocabulary in one `macro_rules!` list — a line per role carrying its name, its
family, whether it may carry an intent, and the pattern its parent must match —
and emits `Role`, `Role::ALL`, `Role::COUNT`, `index`, `name`, `family`,
`is_operable` and `accepts_parent` from it. There is no second place a role can
be written, so there is nothing for a variant to be omitted from, and
`all_is_every_variant_the_enum_has` is deleted: deleting it is the repair rather
than a casualty of it.

What a role costs now is a line in that list, which is a diff a reviewer reads.
`Role::Other` would be `Other, "other", Structure, false, Some(_any),` — and
that line fails `the_vocabulary_has_no_escape_hatch` on the spelling and
`the_family_census_is_what_the_rfc_says` on the count, both of which are now
tests about a list that is the vocabulary rather than tests about an array that
was supposed to be. The price of the construction is one macro between a reader
and the enum, which is a real cost and is why nothing else in the crate is
written this way. It is paid here because here the second copy was the defect.

This is recorded rather than quietly fixed because the mistake is the
instructive kind, and because it recurred: a test that iterates the registry of
a thing cannot be the guard on that registry, and neither can a test that has to
be kept in step with it by hand. The only guard is that there is one registry.
The same shape is available to anybody who writes a `FAMILIES` array, a `UNITS`
array or a table of defect names in this crate.

**What it makes hard.** Growing the vocabulary. Every addition is a line in a
list nobody can add to by accident, changes a family census a test asserts,
changes a count RFCs quote, and breaks every downstream projection until it is
handled.
That cost is intentional and it is the whole mechanism — but it means the
vocabulary will lag real applications, and somebody will propose `Other` again
under schedule pressure. This RFC is where the answer already is.

**What it forecloses.** Nothing structurally: a namespaced extension space, a
larger vocabulary, and `#[non_exhaustive]` all remain available, and each becomes
*cheaper* once there is a canvas-escape rate to argue from instead of a taste.
What it forecloses is doing any of them quietly.

**What it does not claim.** It does not claim twenty-two is right. It claims
twenty-two is defensible against three interfaces chosen to be hard, that the
argument is written down, and that the number is judged later by a measurement
rather than by whoever is most confident in the room.

## What this decision owes, and who owns it

Three rows this change implies live in files this change does not touch, listed
so that nobody has to re-derive them and so that their absence is visible rather
than assumed.

**`ROUTES` in `xtask/src/main.rs` — the orchestrator's, and no longer owed.**
`("canvas-escape-rate", Route::Unbuilt("E3-B06l"))` is in that table. This
paragraph said *until it is there* while the row was already in the same
uncommitted change set as this file, which is the same tense-defect the
reversal section below confesses to and repairs — caught in one place and left
standing, inverted, in the other. What the row buys is stated once, here:
`lint-reproduce` reads `claims/0034` as a claim whose published command refuses
rather than as a claim whose command runs nothing, which is what
`Route::Unbuilt` was added for and what `claims/0033` established.

**`docs/rfc/README.md` — the orchestrator's, and deferred here deliberately.**
This RFC and 0078 through 0080 have numbers and no entry in the shared index.
The review is right that `docs/postmortem/0001`'s scar is two worktrees taking
one registry number, and right that deferring the reservation is what makes that
possible rather than what prevents it. It is still not this diff's to take:
`intent/0012-the-interface/plan.md` names that file as the orchestrator's and
lists the rows owed against it, and four parallel trees each editing one index
is the postmortem's failure arriving by the other door. What would change the
answer is the index growing a mechanical check — an `xtask` verb that refuses an
RFC file with no row, the way `classify` refuses a workspace member with no row
— at which point the reservation stops depending on who remembered.

**`claims/README.md` — the orchestrator's.** `intent/0012`'s plan names three
claim rows owed for this epoch, of which the canvas-escape rate is one; the
registry file now exists, and the README sentence describing it does not.

## What would reverse this

**The canvas-escape rate.** Section 13's metric, and the one this decision is
registered against. The condition, stated so it can be observed: in a corpus of
applications ported to this vocabulary, more than **one declared node in twenty**
is a `Role::Canvas`.

The denominator moved, and the move is the point. This said *one node in twenty
that carries the application's own work*, which reads well and cannot be
computed: nothing decides which nodes those are, so two people counting the same
corpus would get two rates and each could defend theirs. Every declared node in
the corpus is a denominator anybody can compute and nobody can argue with, and a
reversal condition that cannot be computed is a sentiment with a number in it.

The registry row is `claims/0034-canvas-escape-rate.toml`, at
`canvas_escape_rate_x1000 = 50`. It is `pending`, it has no workload, and its
reproduction command refuses rather than reporting a clean zero — `E3-B06l` is
the task that owes it a corpus, and `Route::Unbuilt("E3-B06l")` in `xtask`'s
`ROUTES` is what says so at the command line. This RFC previously asserted that
row in the present tense while it did not exist, which the same review caught;
the row exists now and the tense is true.

Two functions in `node.rs` relate to the rate and they are not the same
function, which this RFC also ran together. `canvas_census` is the per-tree half
of the rate — canvases over nodes, and `None` rather than zero for a tree with
nothing in it. `canvas_escapes` counts the narrower thing, a canvas that
declared *nothing*, which is the `EmptyCanvas` floor being evaded rather than the
vocabulary being too small; it is zero for every tree that passed `check`, by
construction, and its use is a corpus nobody checked.

If the rate comes in over the target, the vocabulary is too small for the work
real applications do, and the answer is roles — argued one at a time — rather
than a wider loophole.

**Somebody demoting `EmptyCanvas`.** The weaker signal and the one to watch
first, because it arrives earlier and looks like a convenience. If a real
application has to be let through `check` with the empty-canvas refusal turned
off, then the refusal is not enforcing a rule, it is obstructing one, and this
RFC is being worked around rather than followed. That is a reversal whether or
not anybody calls it one.

**A role only one application can ever use.** If an application cannot be
expressed at all without a role that no second application adopts once it
exists, then closedness is costing more than it buys and the namespaced
extension space refused above is owed.

The earlier version of this condition said *a role only one application would
ever use*, and a review was right that it could not be applied to any evidence
anybody has — including this decision's own, where thirteen of the twenty
reached roles are reached by exactly one of the three interfaces. That census is
not evidence for this condition and nothing separated the two readings. `Row` is
reached only by the file browser, and every table in every application ever
written is made of rows; *how many trees in a small corpus happen to use a role*
is a fact about the corpus.

So the observation is after the fact rather than at the proposal, which is the
only form of it that can be checked: a role is argued for, it is added, and one
release later no application but the proposer declares a node of it while the
corpus's canvas-escape rate is unchanged for everybody else. Two of those in one
epoch. One is a gap in the vocabulary; two is a shape, and the shape is that the
vocabulary is being extended privately through a public door.

**A tree crossing a ring between different vocabulary versions.** Closedness is
bought by the compiler, and the compiler is not in the loop across a trust
boundary. The first time a component built against version 1 declares a tree to a
system that speaks version 2 — or the reverse — something has to happen to a role
one side has never heard of, and *that* is a wildcard arm arriving by a route
this RFC does not close. The observation is the day the semantic tree gets an
entry format in `f_abi`. At that point this decision needs a companion about
version negotiation, in the shape of RFC 0011, and it should be written then
rather than discovered in a projection.

**The `parent` field disagreeing with a delta.** The departure from section 11
reverses on one concrete event: the first implementation of the delta protocol
has to decide whether `DeclareNode`'s parent or `Node::parent` is authoritative.
If both can be set and they can disagree, the field is duplicated state and it
comes off the node — at which point this crate grows whatever minimal arena a
test needs to build a tree, and section 11's struct stands unamended.
