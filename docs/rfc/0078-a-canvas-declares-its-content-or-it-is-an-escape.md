# RFC 0078: A canvas declares its content, or it is an escape

- Status: accepted
- Date: 2026-09-14
- Revised: 2026-09-14, after the first adversarial review, and again the same
  day after the second. What moved and what did not is in *What the review
  changed, and what it did not* and in *What the second review changed*; the
  decision is the same decision, with clause 4's second half and then its scope
  added, and with the claims the module was making without evidence either made
  good or withdrawn.
- Affects: `interface/src/canvas.rs`; `interface/src/node.rs`'s `Role::Canvas`,
  `Track`, `Clip` and `Marker` and its `Defect::EmptyCanvas`, which this builds
  on and does not relax; RFC 0077, whose refusal of a time range on `Node` this
  is the other half of; `docs/design/ring-scene-boot.html` section 13;
  `TODO.md` `E3-D02`, and through it the exits of `E3-B06` and `E3-P05`

## Decision

A canvas is admitted as a participant when it declares four things, and it is an
**escape** when it declares fewer. The four:

1. **Which surface** — the `Role::Canvas` node the declaration is about.
2. **In what dimension** — the ordered dimension its content is placed in, and
   the scale of that dimension. For a timeline that is a `TimeBase`, in ticks a
   second, declared by the application.
3. **What is selected in it** — a `Selection`, which may select nothing, because
   *nothing is selected* is a statement and saying nothing is not.
4. **Where every occupant it declares sits, at least one occupant, and nothing
   under it that cannot be placed at all** — every `Clip` and every `Marker` the
   tree declares under that canvas has a placement in the dimension; a canvas
   that places none at all is refused outright; and the canvas's subtree holds
   nothing but lanes, clips and markers.

The first three are the arguments of `Arrangement::declaring`; there is no
constructor that omits one and no `Default`. The fourth is checked against the
tree by `Arrangement::admit`, which is the only thing in the module that can
produce a `Participating` — and every query lives on `Participating`. So a
canvas that has not met the floor is not asked the questions and answered
badly; it is not asked, because the value that answers them cannot be built.
The error type is called `Escape`, which is section 13's reading of the
situation made into a word a compiler prints.

### Why clause 4 has a second half

It did not have one, and the first adversarial review of this decision found the
hole in a sentence: a canvas declaring one empty `Track` and placing nothing
satisfied every clause and was admitted as a full participant. That is the
cheapest escape anybody will ever find — draw the whole sequence in pixels,
declare one lane, place nothing — it costs one node, and no test tried it.

The argument against it is this RFC's own, made two sections below about
`node.rs` and then not applied here: *the cheapest possible compliance with a
rule about children is to declare children, and a rule whose cheapest compliance
is worthless is a rule that teaches applications to comply worthlessly.* A
clause met by declaring nothing is met vacuously, so `Escape::NothingPlaced`
refuses it, and the refusal names the canvas rather than a node, because the
canvas is the thing that said nothing.

The price is real and is not hidden. **An application whose canvas is genuinely
empty — a new project, three lanes, nothing on them — is refused, and has to
declare that node as the `Group` it currently is until it has content.** That is
the trade RFC 0077 made when it refused a childless canvas, taken one step
further for the same reason: a canvas with nothing in it is strictly less
expressive than the group it should have been, and from every projection's side
it is indistinguishable from a full one that declared nothing. The node's role
changes the moment the first clip lands; its `NodeId` does not, which is what
makes the change survivable rather than free. The reversal condition is below,
and it is the one that arrives second.

### Why clause 4 is also about what is not a clip

The second adversarial review found the same hole one level lower, and it is the
same hole. The clause counts clips and markers; the vocabulary does not make a
canvas's children be clips and markers. `Role::Group`, `Role::List`, `Role::Item`,
`Role::Text`, `Role::Image` and `Role::Command` all satisfy
`accepts_parent(Some(Canvas))`, so an application places one token clip, meets
every clause, and declares the content it actually draws as a list hanging off
the canvas — placed nowhere, asked nothing, counted by no clause. The floor
becomes a form filled in with a clip.

So the canvas's subtree is closed to the three roles its dimension has a place
for — a lane, a clip, a marker — and anything else under it, at any depth, is
`Escape::Unplaceable`. This is the second time the same argument has been applied
to this decision and the reason is worth stating once: a clause about *how much*
content must be placed is worth nothing while there is a place to put content
that the clause does not look at.

The price, named rather than hidden, and larger than the last one. **A canvas may
not carry a caption, a legend, an overlay button, or a group of them, among its
descendants.** Those are real things applications draw over timelines. What they
must do instead is declare them as siblings of the canvas rather than as
children, which is where a projection laying out a surface would rather find them
anyway: a node inside a canvas is a node the application is claiming to draw
itself. A name for the canvas is a `Role::Label` beside it pointing with
`Relation::LabelledBy`, which is how the vocabulary already spells naming. The
reversal condition is below.

Five smaller decisions are part of this one. **The time base is the
application's**, not this crate's, because a base that cannot name a sample or a
frame boundary makes every edit an approximation and the approximation is
invisible. **The selection is a stretch of the dimension**, not a set of node
identities, because a selection may cover no clip at all and `StateSet::SELECTED`
already holds the other fact. **An ambiguous query is
refused rather than guessed, and the refusal is a different word from the empty
answer**: where two clips cover the stretch asked
about — a crossfade — the answer is `Sole::Several` and a set-valued query is
offered, because an agent told *the clip* when there are two has been given a
wrong answer confidently, and one told *nothing is there* when two things are has
been given a different wrong answer. An `Option` spells those two identically,
which is why the answer is a three-valued type and not one.

**The canvas declares its rendering, and this crate never resolves it.** Section
13's sentence has two halves, and a module building the first will forget the
second. The vocabulary already has the word: `Content::Media` is *pixels or
samples the application holds elsewhere, named by capability*, so a canvas
carrying one is saying *these pixels are mine, here is the handle*, and
`Participating::rendering` is how a projection asks for it. Nothing here decodes
it or derives an answer from it, and that independence is the property rather
than the limitation — every query answers the same with the handle absent, so a
consumer that cannot render is not a consumer that gets less. The two halves
touch at exactly one point, which is that a rendering with nothing placed under
it is the opaque rectangle wearing a capability, and clause 4's second half is
what refuses it.

**Addressable and scriptable are asked separately, and scriptable is asked of a
canvas.** Locating a clip and being able to do something to it are two facts, and
`Participating::operable` is the second: for a node this canvas declares, it
answers what an agent may invoke — the capability the node carries, whether the
application currently declares it operable, or *inert* for one carrying nothing.
The same answer rides on every `Phrase`, so a consumer that linearised the canvas
can act on what it heard without walking the tree again, which is the difference
between a description and a script.

It is asked of a canvas and of nothing else, and that took two goes. The first
version was a fixture assertion with no module behind it. The second added the
method and left the unscoped constructor public beside it — `Operable::of(node)`,
a two-field read applicable to any node of any tree — with the refusal documented
over it. `Operable` has no public constructor now: every route to one runs through
a canvas that declared the node, and the refusal for one that did not is
`Escape::NotOfThisCanvas`. The same rule reaches the rest of the questions: a
query about a lane refuses a lane this canvas does not declare, rather than
answering `Sole::Nothing` — which is the empty answer `Sole` exists to keep apart
from a refusal, and was being handed back for questions the canvas had no standing
over.

Nothing here has been measured. The canvas-escape rate RFC 0077 registers as a
target is still a target, and this RFC adds a second thing to watch that nobody
has watched yet: see the last section.

## Context

Section 13 of `ring-scene-boot` names three hard cases, says the canvas is the
one that sank every predecessor, and states the test so that it can fail: *can
an agent select the clip between 4.2 s and 6.8 s on track 3, and can a screen
reader describe it, without either one seeing a single pixel?* That sentence is
now a test in `interface/src/canvas.rs`, with those numbers, and it is the exit
of `E3-D02` rather than a paraphrase of it. A test that reproduces the design
document's own falsification is worth more than five that do not, because when
it goes red the thing that has failed is the pillar.

RFC 0077 closed the vocabulary and refused the obvious escape hatch by making a
childless canvas a `Defect::EmptyCanvas`. That refusal is necessary and it is
not sufficient, and the gap is exactly what this RFC is for. A canvas can
satisfy `check` — three tracks, five clips, everything named and in its right
place — and still answer nothing, because *that a clip exists* is not *when it
is*. The cheapest possible compliance with a rule about children is to declare
children, and a rule whose cheapest compliance is worthless is a rule that
teaches applications to comply worthlessly.

So there are two floors and they compose. The vocabulary's floor says a canvas
may not keep its content. This one says what *keeping* means: content nobody can
locate in the canvas's own dimension is content the canvas kept.

### The incentive problem, which this RFC does not settle

Section 13 states it plainly and it should be restated here rather than filed
under consequences: **this asks developers to declare structure they do not
currently declare.** The pitch is that they get automation, accessibility,
testing, remote projection and agent support in exchange, where today each is a
separate integration they write and maintain — and the trade is only real if
declaring the model is *less* work than the five integrations it replaces.

That is a usability question and not a technical one. Nothing in this RFC
answers it, nothing in this crate can answer it, and it should be tested against
real developers early, because the answer decides whether the pillar is
adoptable at all. What this decision does is make the question askable in one
direction only: a developer who declares nothing gets nothing, rather than
getting a rectangle that renders and fails silently everywhere else. If the
answer comes back that the floor is too much work, the reversal below says what
that looks like when it happens.

### What the review changed, and what it did not

This decision was refused once, by a reviewer whose job was to break it, and the
findings are recorded here rather than in a changelog because the next reader
will otherwise raise them again.

**Three findings were about the exit's own words and they were right.** The exit
says the timeline case is *addressable and scriptable while the pixels stay
custom*, and two of those three clauses were being carried by the fixture rather
than by the artifact. *Scriptable* rested on one assertion — `node.intent`
is `Some` — that was true because the fixture had written an intent 250 lines
earlier, and `canvas.rs` read no intent anywhere; deleting the whole module left
that assertion passing. *While the pixels stay custom* was not represented at
all: the fixture's canvas carried `Content::None`, so every assertion would have
passed in a vocabulary where a canvas could not carry a rendering. Both are now
the module's work rather than the fixture's — `Participating::operable` and
`Participating::rendering` — and the floor's hole is clause 4's second half
above.

**Four findings were correctness and all four were fixed.** Markers were dropped
from every selection by a `let … else` that read the wrong half of a `Span`, so
the playhead at 5.0 s was outside a selection of 4.2 s to 6.8 s. The
linearisation converted `Content` down to text and mapped the other three kinds
to the empty string, which silently unnamed the *canonical* clip — the one whose
content is `Media`; it now carries the content whole, so there is no conversion
left to forget to extend. `TimeBase::names` took a single `u32`, which made the
headline rate this base exists for unaskable. And `Escape::Unsayable` carried a
number wrong by nine orders of magnitude, guarding a case no test constructed.

**What the review did not move is recorded with the refusals below**, as four
entries: an intent required of every occupant, a chronologically sorted
linearisation, markers in `occupants_over`, and deleting `Escape::Unsayable` as
unreachable.

### What the second review changed

It refused the decision again, on one finding, and the finding was that the first
repair had been written rather than made: the method that answers *what may be
invoked* was scoped, and the unscoped field read it claimed to replace was public
thirty lines above the sentence claiming it. Nothing exercised it. That is fixed
above and in the module — `Operable::of` is private, the scope check has a test
that asks a canvas about a node of *another canvas* carrying an enabled intent,
and the refusal is therefore about standing rather than about the node having
nothing to say.

Three other findings were about claims the artifact did not support, and all
three are now supported by the artifact rather than by a sentence. `declares`
said it was *the scope of every question here* while scoping exactly one: the
three lane queries and `lane_of` refuse out-of-scope questions now, and
`lane_of` returns a `Result` so that *it sits on the canvas itself* and *not mine
to answer* stopped being spelled the same way. The floor bound two roles and is
described above. And the enum's doc miscounted its own question-shaped variants,
which mattered because the argument for one error type rested on the count.

Four were about tests that could not fail: `PLACED_MAX` was exercised against the
builder and never against a tree, the signed `Ticks` argument had no negative
number anywhere in it, two of the four `Content` kinds never travelled through a
phrase, and the closed-enum test was driven by a hand-written array that a new
variant did not force anybody to extend. Each now has the thing it was missing.
One finding — that this RFC has no row in `docs/rfc/README.md` — was not acted on:
that table is not an index and says so, and holds one row per entry that needs
one. This entry's task line is `E3-D02`, where a reader would look.

### What was refused, and why each loses

**Children as the whole floor.** Keep `Defect::EmptyCanvas` and stop — say that
a canvas must declare *what* is in it and leave *where* to convention. Refused,
and this is the central refusal. A timeline that declares a track and a clip and
no times is strictly less useful than the list it could have been, and it passes
every check in `node.rs`. Every question section 13 asks is a question about
where: which clip is between 4.2 s and 6.8 s, what is under the playhead, what
does this selection touch. A floor that does not reach *where* is a floor above
nothing.

**A time range on `Node`.** The direct answer, refused by RFC 0077 on the
grounds that a vocabulary extended by growing its node is not closed. This RFC
is what pays for that refusal rather than merely repeating it: the arrangement
is a side structure keyed by `NodeId`, the join works, and the timeline case is
expressible with the vocabulary untouched. Had it not worked, 0077's refusal
would have been the thing to reverse.

**Milliseconds, or any base fixed in this crate.** The lazy answer, and wrong
for the one case the module exists for. Audio at 48 kHz has a sample every
20.83 µs and NTSC video a frame every 33.3667 ms; a base that cannot name those
boundaries puts every cut between two samples, and nothing downstream can tell
that it happened. The cost of declaring it per canvas is real and is named: two
canvases in one tree may use different bases, so a consumer comparing across
them must convert. That is accepted because a conversion is visible in the code
that does it, while a rounding is visible nowhere.

**A rational time base — a numerator and a denominator, as the editing tools
use.** Genuinely better for content whose rate divides no integer tick rate, and
refused for version 1 because there is not any: `TimeBase::FLICKS`, at 705 600 000
ticks a second, names every common frame rate exactly, NTSC's 30000/1001
included, and every common audio rate. A rational base would cost every consumer
a fraction comparison and would make two declarations that mean the same instant
unequal as values. The reversal condition is below and it is concrete.

The *predicate* takes a fraction even though the base does not, and it has to.
`TimeBase::names` asks *does this base name every boundary of content running at
this rate*, and half the rates worth asking about are not integers. Its first
version took a single `u32`, which made the headline case unaskable: a caller
following this RFC's own NTSC sentence rounds to 29 970, 705 600 000 does not
divide by that, and the predicate answered *false* about a rate flicks names
exactly — the wrong answer in the expensive direction, telling an application to
invent a base it does not need. The reversal condition below is armed on that
predicate, so a predicate that misfires there misfires a reversal.

**A selection of node identities, or reusing `StateSet::SELECTED`.** Refused
because it cannot express what a timeline's selection is. Selecting 4.2 s to
6.8 s on a lane with nothing on it is a real operation — it is where the next
paste goes, and what a ripple delete closes — and a set of selected clips is
empty in that case and in the case where nothing is selected, which are
different facts. `SELECTED` on a node stays what it is, and the two meet at
`Participating::selected`, which is a derivation rather than a second copy.

**A multi-range selection.** Real: a ripple delete across three lanes and two
stretches. Refused for version 1 because a bounded array of stretches is a
capacity decision with no corpus to size it against, and the exit needs exactly
one stretch. Reversal below.

**A lane named on the placement.** It would make every query a single lookup.
Refused because the tree already says which lane a clip is on — it is the node's
parent — and a second copy of that fact is a second thing that can disagree with
the first. This is RFC 0077's refusal of a time range on `Node`, running in the
other direction, and taking it in only one direction would have been a
convenience rather than a principle.

**Coordinates.** A rectangle per clip, in pixels, which is what every canvas API
that has ever shipped offers. Refused by section 11's rule and worth restating
because the canvas is where that rule is most tempting to break: what a canvas
declares is where its content sits in *the content's own dimension*, which is a
fact about the work, and not where it sits on a display, which is the
projection's business and is different on every one of them.

**Answering an ambiguous query with the first match.** Clips on one lane may
overlap, because a crossfade is exactly that, so the query that asks for *the*
clip over a stretch can have two answers. Returning the first would be a total
function and a wrong one. Refused: an agent acts on what it is told, and a
confident wrong identity is worse than a refusal it can handle.

**A trait over every kind of canvas, and building the viewport now.** A trait
would have to name the type of *where an occupant sits*, which is precisely what
differs between a timeline and a viewport — so it is either an associated type,
which makes the dimension open and hands every projection an extent it may not
understand, or it is erased behind a box this crate has no allocator for. An
open dimension is the open vocabulary again, arriving through the type system.
Building the viewport was refused for `ladder.rs`'s reason: a three-dimensional
extent invented now would be a guess frozen into a crate everything downstream
must satisfy. And the camera is harder than it looks — **a camera is a
projection**, and part III's claim is that the system owns projection, so a
declared camera is either a coordinate, which section 11 forbids, or a semantic
statement whose vocabulary nobody has. The floor's four clauses hold for a
viewport unchanged, which is the argument for having stated them abstractly.

**An intent on every occupant, as a fifth clause of the floor.** The tempting
way to make *scriptable* structural rather than merely answerable, and the first
thing a reader who has just been shown the floor's second half will reach for.
Refused: a read-only timeline is real — a review copy, a locked lane, a render
preview — and `node.rs` spells read-only as the absence of an intent. A floor
demanding one would refuse an honest declaration in order to catch a dishonest
one, which is the trade this decision refuses everywhere else, and it would put
this crate in the business of deciding which content an application is allowed
to publish read-only. What is done instead is to make the difference sayable:
an inert occupant answers *inert*, not silence, so a projection tells *nothing
to invoke here* apart from *nobody asked*.

**A linearisation sorted into time order.** The first version of this module
described its reading order as *the order somebody would read it aloud*, which
is a claim about the clock that the code does not make — the order is the tree's,
and nothing in `admit` requires a tree to declare its clips chronologically.
Refused, and the claim corrected rather than the code: a timeline read in time
order interleaves every lane and leaves a reader announcing three lanes' worth of
clips with no structure to hang them on, while a consumer that wants chronology
has every phrase's own time and the room to sort that this crate does not have.
There is now a test that declares its clips backwards, so the claim can fail.

**Markers among what `occupants_over` returns.** Refused, and the distinction
written where both queries can be read together. *What occupies this stretch* is
about what fills it, and a playhead over a clip has not made the clip two things.
*What does this selection touch* is about everything placed inside it, cues
included, because a ripple delete over a stretch moves the cues in it. The second
question was the one getting the wrong answer, and `selected` is what was fixed.

**Deleting `Escape::Unsayable` as unreachable.** It is unreachable at
`TimeBase::FLICKS` for any tick count an `i64` holds, which is what its doc
comment should have said and did not. It is not unreachable in general, because
clause 2 hands the base to the application: at one tick a second it takes some
9.2 × 10¹⁵ seconds. A guard against an overflow in arithmetic whose inputs this
crate does not choose is not dead code, and removing it would mean a phrase with
its time missing — the opaque rectangle in miniature — for a base this crate has
no say over. The number is corrected and the variant now has a test that
constructs it.

## Consequences

**What it makes easy.** The four projections `E3-B06` asks for get canvas
content for nothing. A screen reader linearises a timeline from the declaration;
a test asserts against a clip by identity through a redesign, which is `E3-P05`;
an agent selects a stretch, learns an identity, and invokes the capability that
identity carries — an authorised call, logged and checkable, rather than a click
at a coordinate. None of those needed a canvas-specific integration, which is
the whole of the pitch in the incentive paragraph above.

**What it makes true that was not.** An application cannot ship a canvas by
declaring a rectangle. It must say what dimension its content lives in and where
each piece of it sits, and if it will not, the tree it declares is refused
before anything renders. That is the enforcement section 13 says F relies on —
*do not declare it and nothing renders* — reaching the one place the design says
the loophole would otherwise eat the thesis from inside.

**What it makes hard.** Empty canvases, on purpose. A canvas that places nothing
is refused, so an application has to declare an empty timeline as the `Group` it
currently is and swap the node's role when the first clip lands — see clause 4's
second half. Overlays and captions inside a canvas, also on purpose: a canvas's
subtree may hold only lanes, clips and markers, so a legend an application draws
over its timeline is declared beside the canvas rather than under it — see clause
4's scope. And large content. `PLACED_MAX` is 64, which is a screen of
a timeline and not a timeline, so an application with thousands of clips must
declare the window it is showing — and *which* window, and who decides when it
moves, is a question this module does not answer because it belongs with the
delta protocol that does not exist. It is not a canvas problem in the end: a
list of ten thousand rows asks the vocabulary the same question, and the answer
should be one answer.

**What it forecloses.** Nothing structurally. A rational base, a multi-range
selection, a viewport kind and a windowing rule all remain available, and each
becomes cheaper once there is an application asking for it than it would be as
a guess made today.

**What it does not claim.** It does not claim the floor is adoptable, and the
paragraph above says why that is not this RFC's to claim. It claims that the
floor is where a canvas stops being answerable if it goes any lower, that the
timeline case clears it, and that a declaration below it is refused by a type
rather than by a reviewer.

## What would reverse this

**A canvas whose content is genuinely not placed in any dimension.** The sharp
case is a paint program: strokes on a bitmap, whose only honest positions are
coordinates, which section 11 forbids as a declaration. If a real application
arrives whose canvas content cannot be placed in *any* declarable dimension,
clause 2 is refusing a legitimate canvas rather than an escape, and the answer
is either a fifth clause or an admission — on the record, in the declaration —
that some canvases are opaque and which ones. The observation is specific: an
application that ports everything except one canvas, and whose author cannot
answer *what dimension is your content in* without naming pixels.

**Compliance in letter and not in substance.** The reversal to watch first,
because it arrives earliest and looks like adoption. Two cheap versions of it are
closed by the clauses above and needed no corpus to see, because both were
available the day the module was written: a spike at *zero* occupants per canvas,
and content hung under the canvas where no clause counted it. What is left is the
next cheapest: a canvas meeting the floor with one clip covering its whole
dimension has declared nothing and passed everything.

**This one is not armed, and saying where it would be armed is the honest form of
it.** The quantity is the distribution of `Arrangement::len()` over admitted
canvases, and a spike at one is the floor being satisfied rather than used.
Nothing computes it: there is no corpus, nothing in `interface/` walks more than
one declaration, and `claims/0034-canvas-escape-rate.toml` records a different
ratio with a different denominator — canvases per thousand declared nodes, which
is RFC 0077's condition and not this one. What arms it is one claim row recording
that distribution, fed by whatever first walks a corpus of declarations; until
that exists this condition can only fire if somebody happens to look, and a
condition that needs somebody to happen to look is a hope. It is written down here
so that the first corpus tool has a row waiting for it. If that is what the
corpus shows, the floor is a form somebody fills in, and the next thing to decide
is whether a floor can be written that is not gameable — or whether the incentive
problem above has already answered the question. Note what closing the zero case
cost, because the same bill arrives again: the clause that closed it also refuses
a legitimately empty canvas, and each further tightening will refuse something
honest in order to catch something dishonest.

**A canvas that is legitimately empty.** The other side of that clause. The
observation is an application whose canvas is empty often enough, or long enough,
that swapping the node's role between `Group` and `Canvas` is a real cost — a
projection that cannot follow the change, an author who reliably gets the swap
wrong, or a tree where the canvas has lanes the user has named and is about to
fill. The answer then is not to relax the clause back to vacuity, because that
restores the one-node escape exactly. It is to give the vocabulary a way to say
*a canvas with nothing in it yet*, which is a declaration rather than a silence —
and that is a change to `node.rs` and needs its own RFC.

**Somebody turning the floor off.** RFC 0077's second reversal, in this
module's terms. If a real application has to be let through with its placements
omitted — an `admit` that skips clause 4, a flag, a second constructor — then
the refusal is obstructing rather than enforcing, and this RFC is being worked
around rather than followed. That is a reversal whether or not anybody calls it
one.

**Content that flicks does not divide.** The rational base becomes owed the day
an application's own rate is not a divisor of 705 600 000 and rounding into it
loses a boundary the application can observe. Two candidates exist already and
neither has appeared here: hardware clocked at a rate nobody standardised, and
content resampled to a ratio rather than a rate.

The observation is mechanical and is about a declaration this crate can already
see: **a canvas that declares a base other than `TimeBase::FLICKS`.** Clause 2
hands the base to the application precisely so that it may, and an application
only does it for one reason — flicks does not name its boundaries. Nothing in the
vocabulary declares the *content's* rate, so a predicate over that rate has no
declaration to run against; what there is, is the base the application chose, and
choosing a different one is the application saying so. `TimeBase::names` is the
test the application applies before choosing, taking the numerator and the
denominator the content's rate actually has rather than a rounded whole number —
which is a different question and was, until the first review, the only one the
predicate could be asked.

**An overlay that belongs inside the canvas.** The other side of clause 4's
scope. The observation is an application whose canvas genuinely contains
something that is not a lane, a clip or a marker, and whose author cannot move it
to a sibling of the canvas without lying about the structure — a legend whose
position is *inside this canvas* in a way a projection would have to know about,
or a nested canvas. The answer then is not to reopen the subtree, because that
restores the smuggling exactly. It is either a fourth role the dimension can hold
— declared, placed, and refused if it is not — or, for the nested case, a canvas
under a canvas with its own arrangement, which is a change to this decision and
needs its own RFC.

**A selection that cannot be declared.** The first application whose selection
is two stretches, or three lanes, and which therefore has to declare a selection
it does not mean. One is a gap; the second one in the same epoch is the shape
0077 names, and the answer is a bounded set of stretches with the bound argued
from what the corpus actually holds.

**A canvas that does not fit.** The first declaration that exceeds
`PLACED_MAX`. That is not a bound to raise quietly — it is the moment the
windowing question has to be answered, and answered for lists and trees at the
same time, because a vocabulary with two different answers to *how do I declare
more content than fits* has two mechanisms where it needs one.
