# RFC 0084: An exit narrowed by measurement is a reversal, and four of E3's are narrowed

- Status: accepted
- Date: 2026-09-16
- Affects: `intent/0012-the-interface/spec.md`, whose `E3-B01b`, `E3-B01d`,
  `E3-B04c` and `E3-B07a` exit lines are replaced below, and which is a
  paste-ready handoff, so a false line there becomes a false line in `TODO.md`; `scene/src/kind.rs` and
  `scene/src/commit.rs`, which already state the narrower sentences and now have
  somewhere to point; `CLAUDE.md`'s *Reversals need RFCs*, applied to a kind of
  sentence nobody had asked whether it covered; `abi/src/scene.rs` and
  `f_abi::Sqe`, named below as co-owners — with `scene/src/commit.rs`, which
  holds the consumer's half — of the one change that would restore `E3-B01d`'s
  wider claim

## Decision

Two parts, and the second is the one that outlives this epoch.

**One. Four exits are narrowed to what measurement shows**, in the words their
own modules already use. Two were narrowed when this RFC was written; two were
added on 2026-09-16 in the circumstance described under *A failure this RFC
caused*, which is recorded rather than tidied away.

`E3-B01b` said *a seventh kind is a compile error in every consumer, the way
`Role` already is*. It now says:

> a seventh kind is a compile error in every consumer that decides per kind
> without a wildcard, and the one consumer in this workspace that decides per
> kind is such a consumer.

`E3-B01d` said *the graph read back is the old scene or the new one, never a
third*. It now says:

> over a ring whose `Release`/`Acquire` pair holds, the graph read back after a
> cut is the old scene or the new one, never a third; over a ring whose pair
> does not hold, third scenes are produced, and the sweep counts them and
> requires them to be there.

`E3-B04c` said *over a recorded motion corpus the error is bounded by an integer
stated in the test*. The integer was measured to be that one corpus's own
maximum rounded up to a pixel rather than a property of the predictor: a
reviewer reimplemented the predictor from scratch, reproduced the constants
exactly, then violated them on 16% of fresh draws over-predicting and 75%
under-predicting, from the same generator. It now names the set it is a maximum
over:

> over the named corpus the test sweeps — 4096 seeded recordings at two report
> rates — the error is bounded by an integer stated in the test.

The word *bounded* is doing weaker work than it was, and that is the point: a
maximum over a named set is a different claim from a bound over a domain, and
only one of them was ever delivered.

**Amended on 2026-09-21: that is no longer the sentence in the spec, and this
RFC is where it is recorded.** Two later rounds added requirements to
`E3-B04c`'s exit and cited this number for them, which is the failure recorded
below happening a second time in the file that names it — a reviewer who checked
the citation found a one-clause sentence where the spec carries a paragraph. The
sentence the spec carries, and which this RFC now records, is:

> deterministic from a seed; over the named corpus the test sweeps — 4096
> seeded recordings at two report rates, and the test asserts that 4096 were
> folded rather than naming the count in prose — the error is bounded by an
> integer stated in the test, and over-prediction is bounded separately from
> under-prediction because a user notices them differently. *Separately* is a
> claim about the instrument and is now met by it: an error is lag only when the
> cursor moved the way the pointer moved and fell short, so a cursor drawn the
> wrong way along a motion that reversed — the textbook snap-back — is
> over-prediction rather than lag. The test also draws ten times that corpus and
> is required to observe the over-prediction bound failing there. Ten times the
> corpus runs the worst over-prediction from 13.44 px to 15.70 px at the
> recorded rate and from 15.76 px to 18.41 px at half of it — the second past
> the stated 16, on eight of 81 920 measurements and every one of them at the
> halved rate — while the two under-prediction maxima move by 4.6 and 5.1 per
> cent and are exceeded five times. That pair is (16 px, 32 px) and was
> (8 px, 32 px) until the classifier above was repaired. `input/src/predict.rs`
> pins each number to the set's own maximum in a `const` so that widening it is
> a build error, and carries the one universal over-prediction bound beside it
> as a theorem over the speed and lead ceilings: 1152 px for the two-axis sum a
> `Deviation` reports, seventy-three times looser, and *attained* by a
> prediction the test drives into that corner.

Three things about that are worth separating, because they are three different
kinds of edit and only one of them is what this RFC is for.

**The ten-times draw is a strengthening, not a narrowing.** It adds a
requirement: the test must draw ten times the set and must observe the bound
failing there. An exit that demands more is not a reversal of anything, and on
the rule stated below — a spec's `exit:` may not be edited freely — it still
needed writing down somewhere. It is written down here. Nobody should read this
paragraph as licence to strengthen an exit by citation; what makes it acceptable
is that the requirement is met by a named test, not that a number was quoted.

**The numbers moved because the instrument was repaired, and the repair is the
substantive part.** The over-prediction bound went from eight pixels to sixteen
without a line of the predictor's arithmetic changing. `Deviation`'s classifier
decided *lag* from the sign of the error against the sign of travel alone, so a
cursor drawn forward while the pointer went backward — on the far side of the
anchor, which is the far side from where the finger is going — was on the near
side of the truth and was filed as lag. A whole class of snap-back was being
counted under the looser bound. The pair the spec used to state was therefore
not a tighter predictor but a kinder ruler, and saying so is the only honest way
to publish a bound that doubled.

**Doubling a published bound looks like widening a number to meet its
counterexample, and it is the thing this RFC exists to refuse**, so the
difference is stated rather than left to trust: the bound is not chosen, it is
`rounded_up_to_a_pixel` of the sweep's own two maxima, asserted in a `const` that
fails to build if the two disagree. A bound moved to swallow an escape would have
to move the measured maxima with it, and the measured maxima are what the
regeneration printer prints.

*What would reverse this amendment:* the exit being cut back to the one-clause
sentence above, with the ten-times draw and the measured narration filed as a
new RFC of their own. That is the other honest repair of the same finding, it
was available, and it was not taken because the requirements are met and
deleting a met requirement from an exit is a narrowing this RFC would then have
to record as well.

`E3-B07a` said *an `Effect` delta carrying one and not the other is refused at
the boundary*. The boundary named does not exist: `abi/src/scene.rs` carries six
opcodes and none of them declares an effect, so the sentence's antecedent could
not occur on any wire. The refusal moved up a layer and into the type system:

> a declaration naming one word and not the other is refused by
> `Effect::declared`, which is the only constructor of an `Effect` and whose two
> fields are `NonZeroU32`, so no value of the type can carry one and not the
> other.

That is stronger than a refusal where it stands — unrepresentable beats refused
— and weaker than the exit, because *at the boundary* was a claim about the
wire and this is a claim about a constructor. Both halves are true and the
narrowed sentence says only the second. Restoring the original needs an effect
declaration record in `abi/src/scene.rs`, which is `E3-B07`'s to add and not
this task's.

**Amended on 2026-09-21: that is not the sentence the spec carries either.** The
mechanism this RFC records below — *a citation is cheaper to write than a
decision* — has now been caught a fourth time, on the third of the four exits,
by a reviewer doing the one thing that catches it: reading the cited document to
see whether it says what the citation claims. The sentence
`intent/0012-the-interface/spec.md` carries — which `TODO.md` copied word for
word, and still does apart from the three-word correction named at the end of
this amendment — is:

> a declaration naming one word and not the other is refused by
> `Effect::declared`, which is the only constructor of an `Effect` and whose two
> cost fields are `NonZeroU32`, so no value of the type can hold half a
> declaration and the refusal names which half was missing. Narrowed from *an
> `Effect` delta carrying one and not the other is refused at the boundary, so
> an effect with no fallback is a declaration error rather than a frame-time
> surprise* by RFC 0084, on two measurements. `abi/src/scene.rs` has six opcodes
> and none of them carries an effect's parameters, so no delta in this workspace
> can carry one word without the other and the exit's antecedent never reaches a
> decoder — the boundary that exists is the one between two integers a caller
> wrote and the value the crate will act on. And nothing requires a
> `Kind::Effect` node to declare anything at all: a node created with that kind
> and no declaration is accepted by `crate::arena` and `crate::commit`, which is
> a census neither of them keeps rather than a refusal this function could make.
> Both halves come back together with a seventh opcode `SET_EFFECT` carrying a
> node, an estimate and a saving, which is the reversal and is an ABI change.

Three things in it were not in this RFC, and they are three different kinds of
edit, so they are separated rather than waved through together.

**The second narrowing is a narrowing and was recorded nowhere.** *Nothing
requires a `Kind::Effect` node to declare anything at all* removes a reading the
original exit invited — *every effect node declares a cost estimate and a cheaper
fallback* is the task's own title — and it is recorded as its own measurement
under *Context* below. It is true: neither `scene/src/arena.rs` nor
`scene/src/commit.rs` contains the word `Effect` outside English prose, so
neither keeps a census of which nodes declared, and `Effect::declared` is a
function a caller either calls or does not. A guard that could refuse an
undeclared effect node would have to be a census somebody keeps, and nobody
keeps one.

**The clause *and the refusal names which half was missing* is a strengthening,
and it was decoration when it was written.** It adds a requirement about the
English a producer reads. Nothing checked it: `Undeclared::message` was pinned
only to being non-empty and pairwise distinct, so swapping `NoEstimate`'s
sentence with `NoFallback`'s — telling a producer that forgot the saving that it
forgot the estimate — left the whole crate green, including the test named after
the clause, and `message` has no other reader in the workspace. The clause is
kept rather than deleted because it is now met: `refusals!` takes a second
literal per row naming what that refusal is about, and
`a_refusal_says_which_half_was_missing` requires each fragment to appear in its
own sentence and in no other's. The swap above now reddens that test by name.
Read together with the paragraph below about `E3-B04c`, this is the same lesson
twice: **a citation attaches as cheaply to a strengthening as to a narrowing,
and a strengthening nobody measured is worse than a narrowing nobody recorded,
because it reads as a stronger claim.**

**One correction of fact, in the RFC and in the spec together.** The sentence
said *whose two fields are `NonZeroU32`*. `Effect` has three fields — a node
identifier and the two costs — so that clause was a sentence about a different
struct. It now reads *whose two cost fields*, in the block quote above and in
`intent/0012-the-interface/spec.md`. This is a correction and not a narrowing:
it claims exactly what it claimed before about the two words the exit is about.
`TODO.md` carries the same sentence and needs the same three words; that file is
not an agent's to edit, so it is named here for whoever pastes.

**Two. An exit sentence is something already written down**, so narrowing one is
a reversal in `CLAUDE.md`'s sense and needs an entry here. This was not obviously
true before it was asked. `docs/sdlc.md` routes work from an intent through a
spec to a diff, and the exits live in the spec, which is a working document an
epoch edits as it learns — so a reader could reasonably have concluded that
editing one is ordinary spec maintenance.

The argument for treating them as written-down anyway is that **an exit is the
sentence a task is accepted on**. It is the one line a later reader uses to
decide whether a task is done, and a task marked done against a sentence quietly
rewritten to fit what was built is precisely the failure this project has
recorded four times in two weeks. So: a spec's *description* may be edited
freely; its `exit:` may not.

The mechanical consequence, stated so it cannot be read as advice: **a narrowed
exit is edited in the spec, with the RFC number beside it.** A narrowing that
lives only in a module comment leaves the spec asserting something the tree does
not do, and `intent/0012-the-interface/plan.md` is a handoff whose whole shape is
*paste these lines into `TODO.md`*. A module comment and a spec that disagree is
not a documentation problem; it is a false line waiting for a paste.

## Context

Nineteen E3 subtasks were built against their exits, then read by adversarial
reviewers whose instruction was to defeat the exit sentence. Eleven were refused
on twenty-eight findings. The repair round allowed three outcomes per finding —
make the sentence true, delete a guard that cannot guard, or narrow a false claim
— and the narrowings are what this RFC records. A later round over the same
subtasks produced the third and fourth, on the same rule and by the same
measurement discipline.

**`E3-B01b`, measured.** The exit calibrates itself against `Role`: *the way
`Role` already is*. A reviewer took the calibration literally. Adding a seventh
`Kind` end to end — the `kinds!` list, `abi::scene::kind`, its `known` and
`label` arms — produced **zero errors** across `cargo check --workspace
--all-targets`. Adding a twenty-third `Role` to `interface/src/node.rs` produced
`error[E0004]`. The yardstick refuted the file that had invoked it.

The repair made the parity real where it could. `scene/src/effect.rs`'s
`matches!(created.kind(), Kind::Effect)` — the only per-kind decision in the
workspace, and a wildcard — became a written-out six-row `ByKind<bool>` literal,
after which the same experiment yields `error[E0308]: expected an array with a
size of 7, found one with a size of 6`, in a module other than the one declaring
the kinds. That is the guard the exit wanted, and it exists.

What could not be made true is the quantifier. Three things still take a seventh
kind silently, and the module names all three rather than counting them:

- a consumer that writes `_ =>`, which Rust gives no way to refuse over somebody
  else's enum, here or in `interface/`;
- a `ByKind` built as `[x; Kind::COUNT]` or as `Kind::ALL.map(..)`, both of which
  grow to seven on their own — `crate::arena` holds one of the first and two of
  the second;
- a consumer carrying a kind as a `u16` validated by `abi::scene::kind::known`,
  which `crate::reconcile` and `crate::commit` both do.

**`Role` does not achieve *every consumer* either.** It has consumers that happen
to match exhaustively, which is a fact about who has written code so far and not
a property of the type.

**`E3-B01d`, measured.** The first implementation modelled a ring slot whose
write did not land as zeros. `E2-P01` does not: a medium that did not take a
write holds its **prior** content, and `zone/tests/cut.rs` is built on that.
Ported faithfully — `Recorded` carrying the previous commit's slots, index for
index — the same sweep produced third scenes.

The reason is worth stating, because it is not a defect in the commit path. A
ring slot whose write is not visible reads as whatever occupied it last:
`ring/src/lib.rs` initialises its slots once and nothing zeroes one on
completion. A previous frame's entry **decodes**. The batch takes it for one of
this frame's, and the commit that follows either seals over a frame that is part
this one and part the last, or is refused whole by `admit`. The first is a third
scene.

Nothing in the delta format distinguishes *this slot's bytes belong to this
frame* from *they belong to the last one*. There is no sequence number on an
entry.

**One sentence here was false and it was load-bearing, so it is corrected rather
than quietly dropped.** It read: *`f_abi::Sqe` has no field left that a scene
delta does not already read or require to be zero.* It has one. `user_data` is
sixty-four bits, `Delta::envelope` **echoes** it rather than zeroing it, so any
value in it passes the reserved-field comparison `Delta::decode` makes of the two
envelopes — `scene/src/commit.rs`'s
`a_delta_carries_sixty_four_bits_this_module_never_reads` is that sentence as a
run rather than as prose. Space is therefore not the obstacle.

The obstacle is a contract, and naming the right one changes what the repair
costs. `abi/src/lib.rs` states `user_data` as *returned verbatim in the
completion* — the submitter's own value, opaque to the service — so a per-entry
sequence written there is the ABI taking a field back from every client on every
ring that already uses it. That is a price to weigh against a stale slot being
detectable. It is a different argument from *there is nowhere to put it*, and it
has a different answer.

**`E3-B07a`, measured, and the measurement is the absence of a thing.** The
exit's first narrowing rests on a count of opcodes, recorded above. Its second
rests on a search that finds nothing, which is the harder kind to state
honestly, so the search is written down: `abi/src/scene.rs` declares six
opcodes and none of them carries an effect's parameters, and neither
`scene/src/arena.rs` nor `scene/src/commit.rs` mentions `Effect` outside
English prose. So a node of `Kind::Effect` may be created, committed and drawn
having declared nothing, and no module is in a position to refuse it — refusing
would require one of them to hold a census of which effect nodes have declared,
and holding that census is a design decision nobody has taken. `Effect::declared`
cannot make the refusal, because a function that is never called refuses
nothing. That is why the task's title — *every effect node declares a cost
estimate and a cheaper fallback* — is broader than its exit, and the exit says
so in its own words rather than leaving the title to be read as the claim.

## A failure this RFC caused, recorded because it is the interesting part

Establishing a route for narrowing an exit created a way to abuse it that did
not exist before, and it was used within a day — twice, by two independent
agents, neither of which was trying to cheat.

`E3-B04c`'s and `E3-B07a`'s spec lines were edited to the narrowed sentences
above and annotated **"Narrowed … by RFC 0084"** at a time when this RFC narrowed
two exits and named neither of them. The citation was false when it was written.
Both were caught by the adversarial reviewers in the same round, one of which put
it exactly: *the narrowing is recorded against an RFC that does not record it.*

The mechanism is worth naming because it generalises. Before this RFC, narrowing
an exit required no ceremony and was caught by review; after it, narrowing an
exit requires a citation, and **a citation is cheaper to write than a decision**.
An agent that has honestly measured a sentence to be false now has a one-line
way to make its edit look procedurally complete. The check that caught it was not
the rule — it was a reviewer reading the cited document to see whether it said
what the citation claimed.

The two narrowings were kept, because both are right on their measurements and
both are now written above. What was wrong was the order: the edit preceded the
decision it cited. The repair is this amendment, and the general lesson is that
**a rule requiring a citation needs somebody who checks citations**, or it
converts a visible omission into an invisible falsehood.

**It happened a third time, in the same subtask, and against this amendment's own
paragraph.** The round that repaired `E3-B04c` on the findings above found that
its spec line had since grown a page of measured narration and a new requirement,
all annotated *by RFC 0084*, while this RFC still recorded a one-clause sentence.
Nobody was cheating that time either: the numbers were measured, the requirement
was met by a test, and the citation was the only ceremony available for an exit
edit that was not a narrowing. That is the mechanism sharpening rather than
recurring — the cheap citation now attaches to *strengthenings* as well, which
the rule below did not contemplate, and the check that caught it was again a
reviewer reading the cited document. The repair is the amendment above.

*What would reverse this paragraph:* a narrowing that cites an RFC which does
record it, reviewed and found accurate, in every case for a run of epochs — at
which point the ceremony is doing its job unaided and this warning is noise.

## Consequences

**What this makes easy.** `E3-B01d`'s narrowed sentence turns a hidden dependency
into a stated one: a commit's atomicity rests on the ring's `Release`/`Acquire`
pair, which is what this repository already says that pair is for, and the sweep
is now the measurement of it rather than an argument against it. `Mode::Lying`'s
third scenes are *required to be there*, on the same logic as `zone/tests/cut.rs`'s
controls and the eager applier beside them: a run in which they were zero would
be a run whose model had stopped being faithful. That is a stronger test than the
one that was refused, not a weaker one.

**What it makes hard, deliberately.** Nobody can now write *scene commits are
atomic* without the clause. That sentence will look like an omission to a reader
who wants a headline, and it is meant to.

**What it forecloses.** Closing `E3-B01d` by improving `scene/src/commit.rs`
*alone*. The stale-slot case needs an ABI change — a per-entry submission
sequence, which makes a stale slot **detectable** rather than merely unlikely —
and the ABI change is not the whole of it. The sequence is declared in
`abi/src/scene.rs`, by whoever next opens `f_abi::Sqe`; the **check of it against
the slot's own index is written in `commit::Batch::offer`**, because the consumer
is the only party that knows which index it is reading and a producer cannot
check a number it is the one writing. This paragraph used to say the repair was
somebody else's entirely. It is two diffs in two crates, and until both land the
clause stays.

**What it costs `E3-B01b`.** The claim that `Kind` is closed the way `Role` is may
not be made — and, the half worth saying out loud, **neither may the claim about
`Role`**. `interface/src/node.rs`'s own wording is the origin of the overclaim,
and a reader who takes `E3-D01` at face value will believe something about
`Role`'s consumers that no mechanism provides.

**A shape, not yet a rule.** Two of eleven repairs were narrowings. That is a
rate worth watching rather than a finding. If a third of an epoch's exits end up
narrowed, the thing to examine is not the repairs but how exits are written: an
exit is meant to be a sentence somebody other than its author could observe, and
a sentence that cannot survive its own first measurement was never that.

## What would reverse this

**For `E3-B01d`, precisely one thing, and it is buildable.** A per-entry
submission sequence in the delta format, checked by the consumer against the
slot's own index. With it a stale slot is refused rather than decoded, the lying
ring stops producing third scenes, and the original unqualified exit becomes
true — at which point this RFC is superseded and `Mode::Lying`'s
required-nonzero assertion inverts to a required zero. Anything less does not do
it: a per-*frame* sequence still leaves the first slot of a torn frame
indistinguishable from the last slot of the frame before.

The price is nameable and is not a missing field. `Sqe::user_data` is the only
sixty-four bits an entry carries that a scene delta neither reads nor requires to
be zero, and the ABI contracts it as the submitter's own value returned verbatim
in the completion. Somebody has to decide whether a scene channel may take it, or
whether the sequence costs a wider `Sqe` instead. Whoever does that also writes
the consumer's half in `commit::Batch::offer`, and both halves land together or
neither is worth anything.

**For `E3-B01b`, two things, either of which would do.** A way to refuse a
wildcard over a foreign enum — which Rust does not have, and which would be a
language change rather than a tree change. Or the weaker and reachable version:
every per-kind table in the workspace written out as a literal, a lint that
refuses `[x; Kind::COUNT]` and `Kind::ALL.map(..)` in a table that must fire, and
no consumer carrying a kind as a bare `u16`. That is three edits in
`crate::arena` and one decision in `crate::reconcile` and `crate::commit` about
where the decoder belongs. If somebody does that work, the quantifier comes back
and this half is superseded.

**For `E3-B07a`, one opcode, and it restores both halves at once.** A seventh
opcode in `abi/src/scene.rs` — `SET_EFFECT`, carrying a node, an estimate and a
saving. With it there is a wire record that can arrive carrying one word and not
the other, so *refused at the boundary* has a boundary again and the refusal
becomes a decoder's; and a delta that declares is a delta an applier can count,
so a census of undeclared effect nodes becomes something `crate::arena` or
`crate::commit` could keep rather than a decision nobody has taken. It is an ABI
change and stops the build of every consumer that decides per opcode —
`commit::section_of`, `commit::admit`, `dirty::REACH`, `kind::Change::of` and
`Arena::apply` — which is the cost and the reason it is its own diff. **It is
charged to no line in `TODO.md`.** `E3-B07`'s own exit is about frame rate under
overload and none of `E3-B07a`–`g` names an effect declaration record, so this
paragraph currently states a reversal condition that nobody owns. Filing it —
the shape a repair round proposed is a sibling `E3-B07h`, `M`, *the wire carries
an effect's declaration*, carrying the pre-narrowing sentence as its exit — is a
decomposition change for whoever owns `E3-B07`, and it wants an RFC of its own
for the ABI change. Until it is filed, this entry is the only place the work is
written down, which is exactly the condition this RFC was written to stop
tolerating.

**For the general half, the reversal is a measurement rather than a mechanism.**
If exits stop being narrowed — if a run of epochs closes its tasks against the
sentences they were written with — then routing a narrowing through here is
buying nothing, and the honest response is to say so rather than keep the
ceremony. The opposite observation reverses it the other way: if a narrowing is
ever found to have been made **silently**, in a module comment with no spec edit,
then this RFC failed at the only thing it was for, and the answer is a lint over
`exit:` lines rather than a stronger sentence here.
