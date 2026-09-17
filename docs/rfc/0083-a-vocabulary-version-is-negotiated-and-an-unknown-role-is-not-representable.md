# RFC 0083: A vocabulary version is negotiated before a tree flows, and an unknown role is not a value a projection can hold

- Status: accepted
- Date: 2026-09-15
- Affects: RFC 0077, whose fourth reversal condition names this day and asks for
  this companion *in the shape of RFC 0011*; RFC 0011, whose negotiation this
  repeats one level down and whose `PEER`/`VERSION_UNSUPPORTED` it reuses rather
  than duplicating; `abi/src/semantic.rs` (`E3-B06b`), which is the
  implementation of part one and part three and is in the tree — this document
  is the argument for the choices that file makes, not a specification of a file
  that does not exist; `interface/src/node.rs`'s `vocabulary!` invocation, which
  owes three emitted items listed under *What this decision owes*;
  `interface/src/node.rs`'s `StateSet::from_bits`, whose documentation defers the
  question answered here by name; `docs/design/ring-scene-boot.html` section 11,
  which says the tree crosses on part I's envelope and says nothing about two
  envelopes disagreeing; `intent/0012-the-interface/spec.md`'s `E3-B06a` and
  `E3-B06b`, which is where those subtasks live — `TODO.md` carries only the
  coarse `E3-B06`

## Decision

Three parts, and the third is the one written to survive an edit.

**One.** A channel that carries a semantic tree agrees a **vocabulary version**
before it carries anything else, in RFC 0011's shape: each side states the
highest version it speaks and the oldest it still speaks, the agreement is the
highest version both reach, and no overlap is a refusal that names what was
missing rather than a channel that opens anyway and degrades. The agreement is
made **in band**, by a `DeclareVocabulary` entry that must be the first semantic
entry on the channel and may appear once per channel epoch — not by a field in
`ChannelHeader`, and not by a feature bit. Nothing in `abi`'s shared header
changes, and *that* is part of the decision rather than an accident of scope:
see the refusals.

*Shape* means **both legs**, and the leg that is easy to leave out is the one
coming back. The writer's range is the entry's payload. The receiver's answer is
the entry's **completion**: `Cqe::ext` carries the agreed version, and because
that completion is the only channel the answer has, `DeclareVocabulary` may not
carry `NO_CQE` — a handshake that suppressed its own completion would be a
declaration, and a declaration is not 0011's shape. The writer then checks the
answer against the range it actually stated rather than believing it, so both
sides hold an agreement each of them computed, which is exactly what
`ChannelHeader::negotiate` does one level up and the reason a peer's statement
never has to be trusted. A reply *entry* was refused in its place: it would need
an eighth opcode, a rule about who may send it, and an answer to what happens
when it never arrives — three new questions to move a number that already has a
place to sit.

The return leg is not decoration. Without it the **sender rule** below is
unreachable, and the sender rule is where this whole argument lands: a sender
that is never told what was agreed can neither refuse to start nor substitute
older roles by hand, so the only feedback left would be a refused frame, which
the *Consequences* section says explicitly is not the notification.

And the refusal names what was missing rather than only that something was. No
overlap is `PEER`/`VERSION_UNSUPPORTED`, and its **detail word carries the
highest vocabulary version the refusing side speaks** — its ceiling rather than
its floor, because the refused peer's question is *what would I have had to say*.
This is stated here because it was previously asserted by pointing at a doc
comment: see *What this decision owes*, where the conflicting statements in
`abi/src/lib.rs` about what a `PEER` detail word carries are recorded as an
outstanding edit rather than quoted selectively.

*Once per epoch* is the rule, and where that rule is **held** is worth writing
down rather than rounding to *enforced*, because today it is held over
agreements and not over attempts. `Session` in `abi/src/semantic.rs` remembers
an agreement and refuses a second one with `Refusal::Renegotiated`; it remembers
nothing whatever about a handshake it *refused*. So a peer whose range does not
overlap is told this side's ceiling — the detail word above — and may then
re-offer inside the same epoch without limit. That is measured rather than
feared: on one `Session::opening(1)`, a million `{highest: 10, floor: 5}`
handshakes are each refused `VersionUnsupported { offered: 1 }` and each leave
`agreed` and `poisoned` at `None`, and the million-and-first entry, offering
`{highest: 1, floor: 1}`, is agreed. The same missing memory is why *first*
means first **accepted** and not first offered: a `DeclareNode` sent ahead of
the handshake is refused `NotNegotiated` and poisons the frame, the `Commit`
that ends the frame clears the poison, and the handshake is then agreed as the
third entry the channel carried. Nothing is admitted before the handshake, which
is the half the decision needs and the half that holds; the number of entries
refused in front of it is bounded by nothing.

RFC 0011 gets that bound for free and this RFC does not, which is a price of
moving the handshake in band and is charged here rather than in a footnote.
There the range is a header field, so a peer cannot re-state it without
re-opening the channel, and re-opening moves the epoch; here the range is an
entry, and an entry can simply be sent again. The bound therefore has to be a
rule the receiver keeps, and the receiver keeps no such rule today. What would
close it is one `Session` field and no new opcode — a refused negotiation made
**terminal for the epoch**, recorded where a `Commit` does not clear it, so that
every later entry of that epoch, a second `DeclareVocabulary` included, is
refused with the standing `VersionUnsupported` until `follow_epoch` moves. That
is the rule the frame poison already follows one level down — *the first is the
cause and the rest are consequences* — applied to the channel rather than to the
frame. It is owed below, with the test that has to go red without it. Until it
lands, this clause binds a receiver only if the receiver keeps it itself, and
the second reversal condition below — *a component that has to lie about its
version to work* — is not merely available but free: the refusal names the
version to claim, and nothing charges for claiming it.

**Two.** A version ordinal is worth negotiating only if it identifies the **same
list** on both sides, so the vocabulary's indices are **append-only**. A new role
is appended and carries the version it was introduced in; no role is reordered;
no index is reused; removing a role raises the floor, which is RFC 0011's own
word for it and is a decision about which peers get dropped. Version 1's list is
frozen by a digest over its roles' names in order, so that reordering the list
is a failure somebody is shown rather than a review note. *Where* it is asserted
is worth stating rather than rounding to *at compile time*: today it is a test in
`abi/src/semantic.rs`, which reads `interface/src/node.rs` with `include_str!`
and so rebuilds when the list changes, but fails when the tests run. The `const`
assertion in `interface/src/node.rs` that would make a reorder a **build**
failure is owed below and is not there yet. Without
this, negotiation is worse than absent: it certifies that two builds agree while
the number 13 means `Command` on one and `Toggle` on the other, and neither side
ever finds out.

The freeze is asserted for the **role list only**, and that is narrower than the
argument above deserves. `Relation` and `Content` already cross under the same
admission rule, and `StateSet`'s bit order is answered by the same vocabulary;
the re-meant-ordinal failure applies to all three identically, and nothing
freezes them. So for those three the negotiated number is, today, precisely the
*label* this section says a label is not. Extending the digest and the `since`
column to them is listed under *What this decision owes* rather than assumed
here, because writing *uniform* over a freeze that covers one list of four is
the kind of sentence this RFC exists to not contain.

**Three.** An ordinal outside the agreed vocabulary is **refused, and it is
refused by not being representable**. The claim is exact, and it is worth
stating exactly, because the layer it is true of is not the layer a reader
assumes.

Decoding a semantic entry yields an ordinal the agreed vocabulary **named**, or
it yields an error; there is no third outcome. `abi` has no fallback role, no
clamp and no skip, and it never supplies an ordinal it was not given: the
admitted value's field is private and the admission is its only constructor, so
an ordinal the agreement did not name is a value that does not exist anywhere in
the system rather than a value every projection has to carry an arm for. The one
function that turns an admitted ordinal into a `Role` is `interface`'s
`Role::from_index`, derived from `ALL` the way `from_name` already is, and past
that function no decoded type has a role field wider than `Role`. The value a
wildcard arm would have to match cannot be constructed, so no projection can be
handed one and there is nothing for an arm to be written about.

What this does **not** claim, because it is not true and an RFC that claimed it
would be the guard a later edit walks past: it does not claim the type system
closes this on its own. Two seams are held by argument rather than by the
compiler, and both are named here so that neither is discovered later.

- **The predicate belongs to somebody else.** `abi` asks a `Vocabulary`
  implementor whether the agreed version names an ordinal. An implementor that
  answers `true` for everything reopens the vocabulary, and nothing in `abi` can
  detect that — the module says so in its own words. What the implementor owes
  is a *derivation*, `ALL.iter().filter(|r| r.since() <= agreed)`, and an
  implementation that matches on a hand-written range of ordinals is the copied
  table wearing a method. This is the one place *closed* rests on a review
  rather than on a build, and `interface/`'s list — RFC 0077's — is what is
  closed; `abi`'s contribution is that it never maps and never mints.
- **The ordinal is readable.** The admitted ordinal's accessor is public,
  because the crate that turns it into a `Role` is a crate `abi` cannot see, so
  the number has to cross that boundary as a number. What stops the accessor
  from being the escape hatch is that there is exactly one thing to do with it,
  and *a second decoder*, below, is the reversal condition on that — it applies
  to a snapshot read back and a simulator replay as much as to this wire.

The refusal is uniform across every closed enum that crosses — `Role`,
`StateSet`'s bits, `Relation`, `Unit`, `Flow`, `Content`'s discriminant —
because a per-enum policy is a table of judgements that the next enum's author
has to guess at. Uniform means the *refusal*: one admission rule, one refusal
type, no per-enum policy. It does not mean the freeze, which part two says
reaches the role list only.

The **granularity** of the refusal is the frame. Section 11 says the semantic
protocol is *atomic per commit*, so the unit that already exists is the unit that
is refused: the offending entry is reported by its position and its ordinal, the
pending edit is poisoned, the `Commit` that would have closed the frame fails
with the same error, and the tree the receiver is already presenting **stands
unchanged**. Not the entry alone, because a tree missing one node is a tree whose
author believes the node is there. Not the channel, because one bad frame from a
peer that may simply be newer is not grounds to destroy a working interface.

*Stands unchanged* is a guarantee about a **working** interface, and on the first
frame of a channel there is no interface yet: refusing that frame refuses
everything there was, and what the receiver presents is what it was already
presenting, which is nothing. The sentence is not a promise that a first frame
can fail usefully, and a receiver whose peer's opening frame is refused has a
channel it cannot use — which is the correct outcome and is why the handshake,
not the frame, is where a version disagreement is supposed to surface.

What a **sender** does when it agrees a version below the one it was built
against is named here, because it is where the whole argument lands. It refuses
to start and says so, or it declares a tree using only the agreed version's roles
— choosing the substitution *in application code, by the author who knows what
the control means*. What it may not do is send the newer ordinal anyway and let
the receiver sort it out. A wildcard arm is exactly that: it makes the
substitution decision in the one place where nobody knows what was substituted.

## Context

RFC 0077 bought closedness with the compiler. A twenty-third role is a compile
error in every projection, the person who must decide what a new role looks like
in a screen reader is told by their build rather than by a user, and that is the
whole mechanism. It then wrote down, as its own fourth reversal condition, the
place where the mechanism has no purchase: *closedness is bought by the compiler,
and the compiler is not in the loop across a trust boundary.* The day it dated
itself to is the day the semantic tree gets an entry format in `f_abi`, which is
`E3-B06b`, which is the task immediately after this one.

The question is therefore not whether the vocabulary is closed. It is closed, and
nothing below reopens it. The question is what a receiver built against version 1
does when a producer built against version 2 hands it the ordinal 22 — and the
honest starting position is that this is a case the compiler genuinely cannot
cover, because the two sides were compiled at different times by different people
against different lists.

Two things already in the tree constrain the answer.

`ChannelHeader::negotiate` is RFC 0011 built: four version fields, the
intersection, `PEER`/`VERSION_UNSUPPORTED` when the ranges do not overlap, and
one sentence that decides most of what follows — *negotiation makes compatibility
explicit; it does not make the wire permissive*. An unknown flag, an unknown
opcode and a non-zero reserved word are all still refused after a successful
negotiation, because a negotiation is a statement made by an untrusted peer.
`FEATURE_NOT_NEGOTIATED` is the worked precedent for the granularity question: it
sits in `ARGUMENT` rather than `PEER` because the channel is healthy and the peer
is present, and what is wrong is one entry.

And `StateSet::KNOWN` in `node.rs` says, of a bit outside it, that it *came from a
version this build does not speak* — written before any such version existed,
next to a `from_bits` whose documentation says in as many words that what to do
about an unknown bit *is deliberately not decided here: that is vocabulary
version negotiation, which RFC 0077 names as a reversal condition and `E3-B06a`
owns*. This RFC is that. `from_bits` keeps every bit it is given rather than
masking, deliberately, because the bits are the evidence that the two sides
disagree; what is decided below is what the layer holding that evidence then
does, which is refuse and report rather than tidy away.

### Two directions of drift, and only one of them looks like the problem

The case everybody pictures is the **unknown ordinal**: the receiver is older, an
index arrives past the end of its list, and something has to happen. It is the
loud failure, and it is the easy one, because the receiver can see that it does
not know.

The case that ends systems is the **re-meant ordinal**. Both sides say version 1,
both are telling the truth as they understand it, and one of them holds a list
whose order is not the other's. Nothing is unknown. Every entry decodes. Every
projection is total. A settings panel's `Toggle` is presented as a `Command`, a
screen reader announces it as one, an agent invokes it as one, and no error is
raised anywhere in the system because no rule was broken — the rule was never
written. Negotiation on its own makes this *worse* rather than better: it
produces a signed statement that the two vocabularies agree.

Part two of the decision exists entirely for the second case, and it is why this
RFC does not stop at *negotiate a number*. A version ordinal that merely names a
list is a label. A version ordinal that identifies a list is a promise, and the
append-only rule with its frozen digest is the difference.

### Where the ordinal is allowed to exist

`interface/` takes no dependencies, on purpose, so that the semantic layer can be
abandoned without taking anything else down; `abi/` sits beneath the frame and
cannot depend on a layer above it. Neither crate can see the other, so the decode
lives in neither: `abi/src/semantic.rs` carries the entry format and the protocol
constants, `interface/src/node.rs` carries the list and the two functions derived
from it, and the join is made by the component that owns the tree (`E3-B06c`).

That is three files, and the reviewer's question about three files is whether
this is RFC 0077's recorded defect — a second place a role can be written —
arriving with a version number on it. It is not, and the reason is worth stating
precisely rather than asserting: **neither number can make the other wrong.**

- `abi`'s `VOCABULARY_VERSION` ahead of the list — a constant bumped with no role
  appended — means version *n* and version *n − 1* have identical role sets.
  Everything that decoded before decodes now, and nothing new is admitted.
- The list ahead of `abi`'s constant — a role appended with `since: 2` while the
  constant is still 1 — means the agreed version is 1, the new role's `since` is
  above it, and a node carrying it is refused at the boundary. Which is the
  correct treatment of a role that has not been released.

Drift is safe in both directions because the admission rule is `from_index(i)`
**and** `since(role) <= agreed`, and both halves are derived from the one list.
`abi` never enumerates roles: no count, no name, no table. How many roles version
*v* has is `ALL.iter().filter(|r| r.since() <= v).count()`, which is a derivation
and not a copy. An `xtask` lint asserting that the highest `since` in the list
equals `abi`'s constant would be a tidiness check and is named under *owes*;
nothing here depends on it, because the failure it would catch is already safe.

## What was refused, and why each loses

**Nothing: the vocabulary is closed, so this cannot happen.** The position the
first draft of this section was tempted by, since RFC 0077's exit is that a role
is added by an RFC or not at all. It loses to the calendar. Two builds of this
system will be six months apart, both will be in the field, and both will be
conforming — the vocabulary being closed says a role cannot be *invented*, not
that two builds hold the same list. A decision that depends on every peer being
compiled on the same day has assumed away the component model, which is RFC
0011's own context paragraph and the reason that RFC exists.

**A wildcard arm in each projection, presenting the unknown role as a neutral
box.** The obvious answer, the one every predecessor shipped, and the one RFC
0077 refused `#[non_exhaustive]` in order to prevent. Three things are wrong with
it and they are different things. It **succeeds**, so the sender is told nothing
and its author believes a control is present that no user can operate. It is
**total over the unknown**, so the vocabulary is open in practice the moment it
exists: nobody needs an RFC to ship a role, they need a peer that ships one, and
the pressure that is supposed to improve the vocabulary is routed away from it
permanently — RFC 0077's argument against `Role::Other`, verbatim, arriving by a
route that RFC's being about the enum does not close. And it **decides**: four
projections each guess at what a role nobody defined should look like, the screen
reader's guess and the display's guess disagree, and an application that works
against one is broken by the other.

**`Role::Other` at the wire only — map an unknown ordinal onto a known role
during decode.** The compromise that looks like it respects RFC 0077 because the
enum is untouched. It loses to the same three properties: it succeeds, it is
total over the unknown, and it decides. Whether the grey box is produced by a `_`
arm in a projection or by an `unwrap_or(Role::Group)` in a decoder is a question
about which file it lives in, not about what it does.

**Refuse the entry and keep the rest of the frame.** The proportionate-looking
answer, with a real precedent behind it: `FEATURE_NOT_NEGOTIATED` refuses one
entry and leaves the channel alone. It loses for a reason specific to a *tree* —
the entries are not independent. A refused `DeclareNode` takes its children with
it, because `accepts_parent` refuses every one of them and `check` reports every
relation that pointed at it, so *refuse one entry* is never local: it is a
cascade whose extent the sender cannot predict. What the receiver would then
present is an interface with a hole in it, and a hole is worse than a grey box. A
grey box is visibly wrong; a settings panel silently missing its third toggle is
a screenshot nobody questions.

**Refuse the whole tree and start again.** The strict answer, and it
over-corrects in the direction that hurts users. A component whose tree has been
presented correctly for an hour loses it because a peer sent one entry from a
newer vocabulary, and the visible failure is a blank surface rather than a stale
frame. The commit boundary already exists and is already atomic, so refusing the
frame costs exactly the frame — the smallest unit that is *complete*, and
completeness is the property the entry-granularity option lacks.

**Tear the channel down.** Available to a receiver as policy, and not made the
rule. After a successful negotiation an out-of-range ordinal is a peer that has
broken its word, which sounds like a `PEER` condition. But a receiver cannot
distinguish a lying peer from a buggy one from a proxy that mangled a field, and
the cost of guessing wrong is destroying a working component's interface.
`ARGUMENT`, per frame, keeps the refusal proportionate and puts the evidence in
the sender's own completions, where its author is the one who sees it.

**A minimum-understood-version on each entry, so a receiver may skip what it does
not need.** The most sophisticated-looking option and the worst one. It is the
wildcard arm with a version number attached, and it moves the decision about
whether a node matters from the receiver — which at least knows what it is
presenting — to the sender, which is guessing. A sender that marks a node
skippable is asserting that an interface without it is still correct, which is a
claim about a projection it has never seen.

**A `vocabulary_version` field in `ChannelHeader`, negotiated at setup.** The tidy
answer, and the one this RFC came closest to taking. Three things sink it. The
header is sixty-four bytes with four reserved words for the entire system, and
spending one on a single service's vocabulary is the scarcest possible place to
put a field that is meaningless on a block-device channel and must be zero there
— a field that is zero nearly everywhere and meaningful in one place is the shape
R04 exists to refuse. The frame would have to know the semantic vocabulary's
version in order to open a channel it does not otherwise participate in, which
wires `interface/`'s number into the frame's core type and contradicts that
crate's deliberate independence. And RFC 0011 is explicit that feature bits are
cheap to add and expensive to remove, and that the bitmap is where that design
will show its age; the in-band handshake costs one opcode in one service's own
opcode space, which is the cheapest change to shared state available, which is
none.

**Putting the freeze digest on the wire and comparing it.** It would catch a
build that edited the frozen literal, which a compile-time assertion cannot.
Refused because it answers a different question: a peer that lies about its
vocabulary is already handled by the boundary refusal, and asking a peer to prove
*which build it is* is attestation's job — RFC 0012 owns that and does it
properly. A digest on the wire would also let two honest builds of the same
version refuse each other over a difference in how the digest was computed,
trading a real failure for an invented one.

## Consequences

**What it makes easy.** A newer component learns at connect rather than at a
user's desk — and *learns* is a word this decision has to pay for, which is what
part one's return leg is. The agreed version comes back in the handshake entry's
completion, so a sender built against version 2 that agrees version 1 is told so
before it declares its first node, by a value it reads rather than by an error it
has to provoke. That is the same thing RFC 0077 bought with the compile error:
*tell the person who can decide, at the moment they can decide*. Across a ring
the compiler is absent, and the setup handshake stands in its place. The boundary
refusal is not the notification; it is the floor under it, for the peer that
ignored the notification or never made one.

**What it makes hard.** Shipping a role. It was already hard — RFC 0077 made it a
line in a list, a family census, a count that RFCs quote, and a break in every
projection — and this adds the wire: a version bump, a `since` on the line, a
frozen digest for the new version, and every deployed peer below that version
unable to receive a node of it until it is updated. That is the cost of the
vocabulary meaning the same thing at both ends of a ring, and there is no version
of this where it is free.

**What it makes true that was not.** A tree can cross a trust boundary without a
wildcard arm existing anywhere in the system. Before this, the honest answer to
*what happens across a ring* was that somebody would find out while writing the
first decoder, and whatever they wrote in a hurry would become the rule.

**What it does not change, and must not.** `E3-B06`'s parent exit is that a
twenty-third role added to the `vocabulary!` invocation breaks **all four**
projections at once. Nothing here softens it: this decision adds no code path to
any projection, and the refusal lives above all four, in a decode they cannot
see. A version-2 sender is refused at the boundary rather than handled in a
projection, so the four stay total over the closed enum and a role added to the
list still breaks every one of them. If an implementation of this RFC ever puts a
version check *inside* a projection, the implementation is wrong and the parent
exit is the thing it broke.

**What it forecloses.** Nothing structurally. A header field, a feature bit, a
per-entry skippability marker and an open vocabulary all remain available, and
each becomes cheaper to argue for once there is a deployed system with two
versions in it. What it forecloses is any of them arriving quietly, inside the
first decoder somebody writes.

**What it does not claim.** It does not claim the handshake is sufficient against
a hostile peer. It is not, and it is not meant to be: a hostile peer writes 200
into the role field regardless of what it said at setup, and the *boundary
refusal* is what holds there, not the negotiation. It claims no number either:
nothing here has been measured, this RFC registers no claim, and it states no
threshold.

## What this decision owes, and who owns it

Seven items live in files this RFC does not touch, one of them in a file the
row below otherwise records as paid, and that row is also here to record
something that is **not** owed any more. They are listed so that absence is
visible rather than assumed, in the manner RFC 0077 established — and so that
a row describing a tree this is not gets corrected rather than quoted. Two of
these rows previously said the opposite of the truth: that `abi/src/semantic.rs`
did not exist, and that this RFC had no row in the index. Both were false on the
day this was accepted.

**`interface/src/node.rs` — `E3-D01`'s file, and three emitted items.** The
`vocabulary!` list gains a **`since` column**, the vocabulary version a role was
introduced in, and emits `Role::since`, `Role::from_index`, and a compile-time
assertion that `since` never decreases down the list — so a role appended out of
order fails the build rather than a review. `from_index` must be derived from
`ALL` the way `from_name` already is, for the reason `from_name`'s own
documentation gives: there must be no ordinal it accepts that is not already a
role, or the derivation becomes the escape hatch. Plus **one frozen digest per
released version**: a `const` over the names of that version's roles in order,
asserted against a literal written under a comment saying that it records the
wire as shipped and must never be updated to match the list. That literal is the
one hand-written second copy this decision asks for, and it is the correct kind —
it copies a fact that has already happened, and the whole value of it is that it
does *not* follow the list.

**`abi/src/semantic.rs` — `E3-B06b`'s file. Paid but for one clause, and this
is what it pays.** Not owed, except for the last paragraph of this row: the file
is in the tree, and what follows describes it rather than asking for it. The
`DeclareVocabulary` entry is in `scene.rs`'s established shape — one fixed-width
payload, the private `Reader` already in that crate, and `Reader::finish`
refusing a non-zero tail so that an unread field is refused structurally rather
than against a list of field names written out by hand.
Payload: the writer's highest vocabulary version and its floor, both `u16`,
everything else zero. Protocol constants `VOCABULARY_VERSION` and
`VOCABULARY_VERSION_MIN`, both 1 today. Three refusals with names rather than
conventions — a semantic entry arriving before the handshake; a second handshake
within one channel epoch, since re-negotiating mid-stream would change what an
index means underneath a tree already built; and a role ordinal or state bit
outside the agreed version. The return leg is the entry's completion: the agreed
version is the detail word, the entry may not carry `NO_CQE`, and the writer
confirms the answer against the range it stated rather than adopting it. No
overlap of version ranges reuses `PEER`/`VERSION_UNSUPPORTED`, and the detail
word carries the highest version the refusing side speaks.

What that file does **not** carry, so the absence is visible: no `since` column
and no `Role::from_index`, because both live in `interface/src/node.rs` and are
owed above. Until they exist, the join between an admitted ordinal and a `Role`
is unwritten, and the component that owns the tree (`E3-B06c`) is where it lands.

And one rule of part one that file **states rather than holds**: the epoch has
no memory of a refused negotiation. `Session` refuses a second handshake only
once a first one has succeeded, so *may appear once per channel epoch* binds
agreements and not attempts, and a peer refused for no overlap may re-offer
within the epoch without limit — which the detail word makes cheap, since the
refusal names the version to claim. What is owed is one field on `Session`: the
refusal recorded where a `Commit` does not clear it, and every later entry of
that epoch, a second `DeclareVocabulary` included, refused with that standing
`Fault` until `follow_epoch` moves. Owed with it, because an owed mechanism with
no failing test is a wish: a refused `{highest: 10, floor: 5}` followed on the
same `Session` by `{highest: 1, floor: 1}`, which today returns `Ok` and agrees
version 1, beside the one test that covers this ground today —
`the_vocabulary_is_agreed_once_per_epoch`, which re-sends a handshake only after
a successful one and so cannot see this. It is written here rather than in that
module's comment because the comment is where it was last asserted and not
held, and a file is not the place to record what a file does not do. It is owed
by `E3-B06b` rather than by `E3-B06a`, on RFC 0088's rule that an exit may
require the artefact its own task produces and not one a later task produces:
what this task produces is the decision, and a field on `Session` is that file's.

**`abi/src/lib.rs` — two doc comments that disagree, and this RFC leans on
one.** `error::PEER`'s domain documentation says the detail word carries *the
peer's channel epoch*; `error::peer::VERSION_UNSUPPORTED`'s says it carries *the
version this side offered*. Part one above requires the second, and the
implementation follows the second, because an epoch is already in the channel
header and the version the refusing side speaks is nowhere else. Reconciling the
two — most likely by making the domain line say *per code, see each* — is an
edit to a file this decision does not own, and it is recorded here rather than
made quietly, because quoting the doc comment that suits an argument and not the
one that contradicts it is how an RFC comes to rest on nothing.

**The freeze, for the three closed enums that are not `Role`.** Part two's
digest, `since` column and monotonicity assertion cover the role list.
`Relation`, `Content` and `StateSet`'s bit order cross under the same admission
rule and are unfrozen, so a reorder in any of them is the re-meant-ordinal
failure with nothing standing in its way. Owed to whoever declares those lists.
Also owed with it: what the digest may be computed *with*. `interface/` states
in its own manifest that it has no dependencies and that `f-abi` is deliberately
not one, and `f-hash` is this tree's one SHA-256 — so a compile-time digest over
role names in `interface/src/node.rs` has to be a hand-rolled `const` fold,
which is a second hash in the tree. `abi`'s own test-side digest is an FNV-1a
fold for exactly that reason; whether that is the right answer for the
`interface` side is the question, and this RFC does not settle it.

**`docs/rfc/README.md` — the orchestrator's, and the row is there.** RFC 0077
argues at length that `intent/0012-the-interface/plan.md` names that file as the
orchestrator's, and that four parallel worktrees each editing one index is
`docs/postmortem/0001`'s failure arriving by the other door. That argument is
about who writes the row, not about whether one exists: the row for 0083 is in
the file. What is still owed is the thing 0077 named — an `xtask` verb that
refuses an RFC file with no row, the way `classify` refuses a workspace member
with no row — so that the next RFC's row is not a thing somebody remembered.

**An `xtask` lint, owed and deliberately not depended on.** That the highest
`since` in the `vocabulary!` list equals `abi`'s `VOCABULARY_VERSION`. It is
tidiness rather than safety: *Where the ordinal is allowed to exist* shows that
drift in either direction is already safe, and a guard described as load-bearing
when it is not is how a reader comes to trust the wrong thing.

## What would reverse this

**An index moving.** The condition everything above rests on. If a role is ever
reordered or removed in place rather than appended — if the frozen digest is
edited in a diff that is not a deliberate floor raise — then the negotiated
version certifies an agreement that does not hold, and every argument here is
void. This is the reversal to watch for hardest, because the diff that does it
looks like tidying: alphabetising the list, grouping the roles by family, or
deleting a role nobody used.

**A component that has to lie about its version to work.** If the ordinary way to
ship against a frame one version behind becomes *claim version 1 and avoid the
new roles by hand*, then the handshake is theatre and the boundary refusal is
doing all the work. The right response would be to make deliberate degradation
cheap — a way for an author to declare, per node, which older role a newer one
falls back to — rather than to let the lying continue under a rule that forbids
it.

**The once-per-epoch clause staying unheld.** Part one says the handshake may
appear once per channel epoch; today that binds agreements and not attempts, and
the field that would bind attempts is owed above. The reversal is not the gap —
the gap is written down — it is the gap outliving the argument for it. If the
owed `Session` field is not written, the honest edit is to **delete the clause**
and say in its place that a receiver bounds retries itself, because a rule
stated and unkept is read by a peer as permission and by a reader as protection,
and it is the second of those that costs. Watch for it arriving as a comment
claiming the bound rather than a field holding it: that is how it read before
this paragraph, and a reviewer had to run a million handshakes to find out
otherwise.

**Boundary refusals being routine rather than exceptional.** After a handshake, a
refused frame means a peer sent an ordinal it had just agreed not to send. One is
a bug. A steady rate of them, in a system whose peers all negotiated, means
negotiation is not reaching the code that builds trees and the handshake is in
the wrong place — most likely too far from the producer, agreed by a transport
layer whose result the application never sees. The count belongs in a claim owed
by whoever first runs two vocabulary versions against each other; this RFC
registers none and states no target, because nothing has been measured.

**A second version ordinal appearing.** If somebody needs to version `Relation`
separately from `Role`, because the two really do change on different clocks,
then one ordinal was the wrong granularity and the decision to cover every closed
enum in the semantic layer with one number was wrong with it. Watch for it
arriving as a feature bit, which is how it will be spelled the first time.

**A second decoder.** The *not representable* claim holds because there is one
place an ordinal becomes a `Role`. A tree arriving by another route the compiler
does not cover — a snapshot read back (RFC 0043), a simulator replay of a seed
recorded under an older vocabulary, a remote projection at a machine boundary
(`E3-B06h`) — must pass through that same function. A second decoder written for
one of those routes is this decision reversed, whatever it says about itself,
because the route it covers is the route the next grey box arrives by.

**The refusal moving into a projection.** If a version check ever appears inside
the display, remote, screen-reader or agent projection, then *not representable*
has already failed somewhere upstream — something is handing a projection a raw
ordinal — and the repair is upstream rather than in the projection that noticed.
A projection that can ask about a version is a projection that can have a
wildcard arm.
