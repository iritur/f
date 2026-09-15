# RFC 0079: A theme may not make an interface unreadable, and it is clamped rather than refused

- Status: accepted
- Date: 2026-09-14
- Affects: `interface/src/token.rs`; `interface/src/node.rs`'s `TokenSet` and
  `TokenName`, which this decision resolves and whose grammar it undertakes not
  to widen; `docs/design/ring-scene-boot.html` sections 11 and 12, the second of
  which this makes mechanical; `TODO.md` `E3-D03`, and `E3-B01`'s compositor,
  which will be the first thing to hold a `Resolved` and which this RFC gives one
  obligation to; `claims/0035-theme-refusals.toml`, registered by this decision

## Decision

Design tokens fall into exactly three categories, and every token in the
vocabulary is in one of them by name rather than by convention.

**Themeable, with no bound at all: the three ground colours.** `surface-1`,
`surface-2` and `field` may be any of the sixteen million sRGB values, and
`resolve` never moves one. A background cannot be unreadable on its own —
readability is a property of a *pair* — so the free half of every pair is the
ground and the checked half is the ink.

**Two grounds are a pair, and the answer is a boundary rather than a clamp.**
This is the clause the first draft of this RFC did not have, and its absence was
a hole big enough to drive a theme through: with all three grounds free and never
evaluated against each other, a theme setting `surface-1`, `surface-2` and
`field` to one colour cleared every floor in the module, produced no note, and
left every grouped region and every text field with no boundary a reader could
see. WCAG 1.4.11, which the non-text floor is cited from, is about exactly that.

The fix is *not* to hold `surface-2` to three to one against `surface-1`. That
would refuse `#F2F2F2` on `#FFFFFF` — this module's own default, and the ordinary
idiom for a raised surface everywhere else — and a layer that refused it would be
the decorative theme layer this RFC spends four alternatives arguing against.
1.4.11 does not require two backgrounds to differ; it requires the region to be
*identifiable*, and a rule around it identifies it. So `Resolved::boundary` says,
for any two grounds, whether they part on their own, and carries the colour of
the rule that must be drawn when they do not — `edge` resolved on the raised
ground, which has already cleared the non-text floor against it. **That is the
one obligation this decision places on the compositor, and it is stated here
rather than left implied.**

The promise is one-sided, and the arithmetic is why. There are pairs of grounds
against which no colour in the space reaches three to one on *both* sides at
once: `#515151` and `#9F9F9F` contrast 2.998 to 1 with each other — under the
floor, so a rule is owed — and the best any rule between them does against the
worse side is 2.646 to 1. The rule therefore clears the floor against the ground
it is drawn on and reports what it achieves against the other.
`no_colour_separates_two_grounds_on_both_sides` exhibits the pair by sweeping
every luminance, so that a reader proposing a two-sided promise can see what it
would cost.

**The floor a pair is held to belongs to the node, not to the token it wears.**
This is the second clause a draft of this RFC did not have, and its absence was
the same hole in a different wall. `node.rs` puts no restriction on which tokens
a node wears, so a `Role::Text` node wearing `edge` is a tree an author declares
— and `edge` is held to three to one, so that node was held to three to one. The
standard therefore travelled with the token the theme had just moved: the same
node reads 4 542 under `Theme::DEFAULT` and 3 032 under a theme that sets
`surface-1` to `#767676`, and the demonstration could not see the difference,
because it asked the ink what floor it owed. **A floor an attacker can move is
not a floor.**

So the floor comes from `Duty`, which comes from the node's `Role`: a role a
caller can read glyphs on owes 4.5 whatever token it wears, and the one role
whose whole content is its own edge owes 3. An ink whose own floor is below what
the node owes is **refused** — the clamp-or-refuse rule applied to a nominal
thing, since `edge` raised to 4.5 is `text` and choosing that on an author's
behalf is the guessing this RFC refuses everywhere else — and `Paint::refused`
names the declined token beside `Paint::unknown`, which names a token that was
never in the vocabulary at all. The node paints in its role's own ink, and that
the role's own ink always clears the role's own floor is asserted rather than
assumed, because it is what makes the fallback safe.

**Range-checked: the six ink colours and the four metrics.** Each ink is
resolved **once per ground**, and if the pair does not clear its floor the ink
is moved along the line towards black or white — whichever helps — by the
smallest sixty-fourth that works. Each metric takes its bound if it is outside
it. A fifth metric, the resolved em, is *derived* from two of the four and is
range-checked separately, because a legal text size multiplied by a legal
density can be an illegal em and a layer that checked only the factors would
ship it.

**Not themeable at all: four things.** The two contrast floors; the operable
minimum, which is **the larger of the absolute `CONTROL_MIN_PT_X10` and the
resolved em with the two rules that bound it** — absolute at the bottom so that
shrinking the em cannot lower it, and tied to what the control has to contain at
the top so that inflating a metric cannot leave an eighteen-point control holding
seventy-two-point text, or a seventy-two-point control that is two of its own
thirty-six-point rules and no interior, both of which two legal metrics
multiplied together will otherwise produce; the application's own declared
`min_em_x100`, which is honoured in the unit it was written in; and the
last-resort font family, which is appended structurally so there is no state in
which the stack is empty.

The stroke is inside that expression because it was inside no other. It was the
one metric bounded only against itself — half an em, related to nothing it had to
fit within — which is the same shape of defect as the derived em one level down
and takes the same answer. Counting the rules makes a thicker rule produce a
*taller* control rather than a smaller one, and the same arithmetic gives a
separator the floor it did not have: a node whose whole content is a rule is at
least as thick as that rule. `Resolved::floor_pt_x10` is where both live; before
it, the tree's separator reported a minimum of nought beside a rule the same
`Resolved` said was thirty-six points thick.

That answered the stroke's inflation and left its collapse alone, and the two
are not the same question. `Metric::Stroke` shared `Metric::Space`'s arm in
`Metric::low`, and therefore its floor of zero — under a comment about padding
that says nothing about rules. A theme asking for `stroke_em_x100 = 0` was
inside every bound, produced no `Note`, and left `Report::is_clean` true, while
taking to nothing both the floor above and the one obligation this decision
hands anybody else: `Boundary` carries a colour and no thickness, so the rule
owed wherever `self_evident` is false has exactly one source for how thick it
is. A theme with no space is dense. A theme with no stroke has deleted a rule
the compositor was told to draw and been told nothing was wrong.

So the two metrics no longer share a floor. The stroke's is **two hundredths of
an em**, and it is derived rather than chosen: the thickness is this metric
times the em over a hundred, the em's own floor is ninety, and one hundredth of
ninety truncates to zero where two does not. It is the smallest value whose
product cannot vanish at any em this layer will resolve, which is the only
property being claimed for it — this decision does not say how thick a rule
should be, only that there is one. What would reverse it: a display on which a
boundary is drawn by something that takes no thickness from here — a hairline
the device defines, or a boundary expressed as a shadow — at which point the
guarantee belongs wherever the substitute is decided.

The bound went unread for the ordinary reason, and it is worth recording because
it is a pattern rather than an accident: every hostile theme in the module
attacked by making things enormous, so nothing ever pushed on a low end.
`HOSTILE_ERASED` is the theme that asks for nothing, and it exists because a
bound nothing pushes down on is a bound nobody has read.

**The floor for text is 4.5 to 1 and for non-text is 3 to 1.** Both come from
WCAG 2.1, success criteria 1.4.3 and 1.4.11. They are cited rather than derived:
nothing in this project has measured legibility, and a floor invented here would
be a number chosen because it was available.

**A hostile theme is clamped, not refused, and every clamp is reported.** The
rule that decides which: *what has a metric is clamped, what is nominal is
refused.* A size outside its bounds has a nearest legal size. A colour that
fails contrast has a nearest legal colour along its own hue. A token name
outside the vocabulary has no nearest anything, so it is dropped and named in
the `Paint` that would have carried it.

The arithmetic is fixed point throughout — `LINEAR_X100000`,
`luminance_x100000`, `contrast_x1000` — because RFC 0004 forbids the
floating-point types. sRGB-to-linear is a 256-entry table, the error bound is
under 0.008 in ratio terms, and it is pinned by three tests rather than trusted.

No number here except the two contrast floors has a source outside this
repository. The four metric bounds and the operable minimum are **targets**;
nothing in `E3` has been measured.

## Context

Section 11 says *style is semantic tokens, never values*, and calls it the
mechanism behind the architecture document's claim that a theme reaches
applications its author never saw. Section 12 says where a token stops being a
name: *a token becomes a value only here, and never in the application.* RFC
0077 built the half that holds names and deliberately could not resolve one.
This is the other half, and the question it had to answer is the one those two
sentences leave open — if a theme reaches applications its author never saw,
what stops a theme from breaking them?

The answer cannot be *nothing*, because the failure is not hypothetical. Every
system that has shipped user themes has shipped an unreadable one, and the
reason is structural rather than careless: a theme is written against the two or
three screens its author looked at, and then applied to every screen on the
machine. A palette that is merely tasteless on those screens is illegible on the
rest. That is what makes this layer different from a stylesheet — it is
underneath every application at once, and a preference with that reach is no
longer only a preference.

What made this decidable rather than a matter of taste is an arithmetic fact
that the design document does not contain and that the exit depends on. For a
ground of relative luminance *L*, the best contrast any foreground can reach is
the larger of `(L + 0.05) / 0.05` and `1.05 / (L + 0.05)` — black or white,
nothing between them does better. The two are equal where `L + 0.05` is the
square root of 0.0525, and there both come to the square root of 21: 4.5825757,
which **truncates to 4.582 and must be truncated rather than rounded**, because
it is a ceiling and a ceiling rounded up is a promise nothing keeps. `#008909` is
a colour that stands on it: against that ground, black and white both come to
exactly 4.582 and nothing in the space does better.

That single number decides most of this RFC. A floor of 4.5 is reachable by
clamping against every possible ground, with eighty-two parts in a thousand to
spare; a floor of 7 — the same standard's enhanced level — is not reachable
against a wide band of grounds at all. Clamping is a coherent policy under the
first number and an impossible one under the second, which is why the choice of
floor and the choice of clamp-or-refuse are one decision and not two.

`every_ground_admits_a_readable_ink` asserts the fact by exhaustion, and *what it
exhausts* is the part worth stating, because the first version of it got this
wrong. It swept 256 greys and a stride over the colour cube, and argued that any
colour's relative luminance is a convex combination of channel luminances and
therefore lands in the interval the greys already cover. Landing in the interval
two swept values straddle is not landing on a swept value: the minimum lies
strictly between grey `#757575` and grey `#767676`, the sweep stepped over it,
and the ceiling it confirmed was one part in a thousand too high. The test now
sweeps **every integer luminance** from zero to white's instead of sampling
colours — contrast depends on a ground only through its luminance, and a superset
with no gaps in it cannot step over a minimum — and asserts equality rather than
an inequality, so a ceiling written too low is as red as one written too high.

### The alternatives that were live

**Refuse the theme and keep the previous one.** The obvious design, and the one
most systems that check anything at all use. Refused, on where this layer sits:
a theme is applied machine-wide, so refusing one is a system-wide interface
outage whose cause is a settings screen. The user who chose the bad theme gets
no interface rather than a corrected one, and the failure arrives as *nothing
happened* — which is indistinguishable from the setting not having been saved.
Worse, refusal is all-or-nothing over a structure with eighteen independent
pairs: a theme whose seventeen good pairs are thrown away because of one bad one
has been punished for a typo.

The honest cost of clamping is stated in Consequences rather than hidden here,
and it is real: a clamp can collapse two tokens into one colour.

**Refuse the hostile ground instead of moving the ink.** Symmetric, and it
looks tidier — the ink is what the designer cared about, so preserve it and
reject the background. Refused because it inverts which half of the pair is
load-bearing. A ground is a large area and the thing a theme is *for*; an ink is
a small area whose job is to be read on it. Moving the large area is a visible
change to the theme's character, moving the small one is a change to a colour
nobody can name. And there is a mechanical reason on top of the aesthetic one:
grounds are shared between inks, so moving one ground to fix one pair silently
changes the other five pairs on it, and a fix that propagates is a fix nobody
can reason about.

**Hold a raised ground to three to one against the ground it sits on.** The
symmetric answer to the ground-against-ground gap, and the one a reviewer reaches
for first: if `surface-2` and `surface-1` are a pair, check the pair. Always
achievable, too — the ceiling above is 4.582, so a raised ground can always be
moved far enough. Refused on what it costs, which is every subtle raised surface
in existence. `Theme::DEFAULT`'s `#F2F2F2` on `#FFFFFF` is 1.119 to 1 and would
be clamped most of the way to a mid grey; so would the equivalent in every theme
anybody has written, because a card a shade off its background is the idiom, not
an attack. A layer that corrects the default theme it ships has a floor that is
wrong, which is the test `a_theme_this_layer_agrees_with_is_not_touched` exists
to apply — and what settles it is what 1.4.11 actually asks for, since the
criterion is about the region being identifiable and not about two backgrounds
differing. The rule is the cheaper answer and the correct one.

**Report a collapsed ground as a `Note` and let the theme's author fix it.**
Tempting, because `Report` is this module's whole honesty mechanism. Refused
because a `Note` is defined here as *one decision this module made that the
theme's author did not*, and nothing is decided when two grounds are alike — so
the note would fire on `#F2F2F2` on `#FFFFFF`, which is to say on every
reasonable theme, and `Report::is_clean` would stop meaning anything. The verdict
belongs on the value, where the compositor reads it, rather than in the log,
where a human is asked to act on something that is not a fault.

**Clamp the ink to black or white outright.** Simplest possible clamp, always
correct, and it is what the last step of `raise_to_floor` does. Refused as the
*first* step because it is the version of this decision that makes the theme
layer decorative: every failing pair resolves to the same two colours, and a
theme's palette survives only where it was already fine. The sixty-four-step
scan finds the smallest move that works, which keeps the theme pointing where it
pointed. `a_clamp_is_not_a_collapse` is the assertion that this is not a claim —
the hostile theme's one saturated colour is still red on every ground.

**Solve for the exact luminance that meets the floor.** Cheaper than a scan and
arguably more precise. Refused twice over. A luminance is a plane of colours, so
solving for one means choosing a hue on a designer's behalf, which this module
has no standing to do. And the scan cannot be replaced by a binary search, which
is the form the suggestion usually arrives in: contrast along the blend is **not
monotonic** — an ink lighter than its ground, blended towards black, passes
through the ground's own luminance where the ratio is 1 — so a binary search
returns a plausible colour that does not work. That is the exact failure mode
this module exists to prevent, arriving inside the fix for it.

**Make the theme type unable to express a hostile value.** Bounded newtypes for
the metrics, a checked colour type, and the refusal moves into the type system
where it costs nothing at runtime. Refused because it would make the exit
vacuous: the demonstration would consist of showing that a value which cannot be
constructed is not constructed. `Theme`'s metrics are therefore raw `i32` and
its font preferences are raw `&str`, so a hostile theme can genuinely spell
`i32::MIN`, a negative gap, and a name made of spaces. Narrowing happens in
`resolve` and nowhere else, and what is tested is the narrowing.

**Approximate the transfer function with a polynomial.** A cubic fit is a dozen
lines and no table. Refused because its error is not something anybody could
state, and this is arithmetic that is *silently* wrong: a contrast ratio off by
a tenth still looks plausible, still orders pairs correctly, and fails only near
the floor — which is exactly where designers put text, because that is where a
palette is prettiest. The table's error is the rounding of its own entries and
nothing else, and it is pinned by `the_pair_that_straddles_the_floor`, which
asserts that two greys one value apart land on opposite sides of 4.5.

**One resolved colour per ink.** The shape every design-token system has. Refused
because it is the bug: resolving `text` to one value means the value is
defensible only against whichever ground it happened to be checked on, and a
node that declares a different ground — which `node.rs` permits, and which the
demonstration tree does twice — gets a pair nobody checked. The whole resolved
table is eighty-one bytes — nine tokens by three grounds, a ground's row holding
its own colour three times — and it removes the possibility rather than
documenting against it.

**Let a metric be a token a node can wear.** Tempting, because `space` and
`stroke` feel like style. Refused: a node able to say how much room to leave is a
node stating a coordinate under another name, which is the one route section 11's
second rule leaves open. `Token` and `Metric` are disjoint vocabularies and
`a_node_cannot_wear_a_metric` asserts it.

## Consequences

**What it makes easy.** A projection can paint without *computing* anything about
readability. Every value in a `Resolved` has already cleared its floor against the
ground it will be used on, and every span already sits above every minimum that
applies to it — so the compositor `E3-B01` builds evaluates no contrast and
compares no sizes, which is the only way the 0.05 ms resolve stage in section
12's chain is affordable.

That sentence was published before it was true. `Resolved::boundary` was a method
that recomputed three contrast ratios — six relative luminances and three
divisions — from the raw grounds on every call, and a compositor calls it once
per ground-on-ground pair per frame. There are exactly nine ordered pairs of
grounds, so `resolve` measures all nine once and the method is a lookup. It is
the eighty-one-byte ink table's own argument applied to the other half of the
module: remove the possibility rather than document against it.

It is not, however, handed nothing to do, and an earlier draft of this line said
it was: *no readability logic in it at all* is what made the ground-against-ground
gap this RFC's to own rather than the compositor's to notice. The compositor has
exactly one obligation and it is a painting instruction rather than a
calculation: **when it puts one ground on another and `Boundary::self_evident` is
false, it draws `Boundary::edge_rgb` between them.** One branch, one colour it was
handed, no arithmetic — the flag and the colour are both read out of the
nine-entry table `resolve` fills, so the branch is the whole of the compositor's
work. Anything beyond that is this module failing to do its job.

**What it makes true that was not.** An ink's resolved colour depends on its
ground. Anything downstream that caches a colour must cache the pair, and a
projection that carried `Rgb` around without remembering what it was checked
against has reintroduced the defect. This is the single most likely way for this
decision to be quietly undone.

**What it makes hard, and this is the real cost.** A clamp can collapse two
tokens into one colour. Under the hostile theme, `text` and `text.muted` on
`surface-1` both resolve to `#040404`, at 4 513 to 1: both are legible, and the
distinction the theme wanted is gone. (This paragraph said `#030303` for two
rounds. That is the colour the blend reaches if its division rounds towards
negative infinity; `toward` divides in Rust, which truncates towards zero, so the
sixty-second step of both inks is `#040404`. The collapse was real and the hex was
asserted rather than computed — which is the failure this RFC warns about two
sections above, under the heading of an approximation nobody can state the error
of.) The layer cannot avoid this — the theme asked for two
colours that are both illegible on the same ground, and there is no answer that
is both readable and distinct — but it must not hide it, which is why both moves
appear in the `Report` with their before and after ratios. A theme's author
reading two `ContrastRaised` notes on the same ground can see the collapse; one
reading nothing cannot.

**What it forecloses.** An alpha channel on a token colour. `Rgb` has three
fields, because an alpha channel would make the pair that is checked different
from the pair that is painted, and a theme could then defeat the floor without
changing a colour value. Compositing belongs to the scene, downstream of the
point at which a token has become a value. A future that wants translucent
surfaces has to composite first and check the result, which is a different
decision and should be argued as one.

**What it does not claim.** It says nothing about whether an interface is
*good*. Contrast is a necessary condition for legibility and not a sufficient
one, and there is no floor here for colour-blind confusability, for text on an
image, or for anything about motion. It also says nothing about the sizes being
right: the four metric bounds and the operable minimum are targets that no
measurement supports, and the claims registry is asked to carry them as such
rather than this file pretending otherwise.

Two more, both of them boundaries of the ground-against-ground clause rather than
omissions from it. **A rule between two grounds is promised against the region it
encloses and not against the region outside it** — for the arithmetic reason in
the Decision, and because the alternative is bounding the grounds, which this RFC
refuses twice over. And **whether a compositor actually draws the rule is not
something this layer can assert.** It can hand over `self_evident` and a colour;
it cannot make the branch exist. That is the second most likely way for this
decision to be quietly undone, after the cached `Rgb` below, and it fails the
same way: silently, on the grounds nobody looked at.

**The registry row this owes, and now has.**
`claims/0035-theme-refusals.toml`, milestone `E3`, status `pending`: the share of
a corpus of real themes that resolves clean, with the mean number of decisions
per theme beside it as the thing a red share is debugged with. The share carries
a floor *and* a ceiling, because both ends are failures and they are different
ones — a layer that clamps something in every theme has bounds that are wrong,
and a layer that clamps nothing in any theme is not doing anything. Its
reproduction is `cargo xtask claim theme-refusals`, and it cannot run until there
is a compositor and a corpus, which means `E3-B01`; until then its route refuses
rather than exiting zero, because a green command that measured nothing is what
the registry exists to prevent.

## What would reverse this

**A ground a theme needs and this floor cannot serve.** Concretely: a theme
whose ground sits in the band near `#767676`, whose designer wants a coloured
ink on it, and whose every coloured ink is clamped to within a few values of
black — so that the theme's palette survives only on its other two grounds. The
observation is a `Report` in which the same ground appears in four or more
`ContrastRaised` notes with `now_x1000` within twenty of 4 500. That is the
clamp working exactly as specified and producing a theme nobody would ship, and
at that point the correct answer is probably to refuse that *ground* — which is
this RFC's second refused alternative returning with evidence behind it.

**The floor moving up.** If accessibility review requires the enhanced 7 to 1
level for any part of this interface, clamping stops being possible for a large
band of grounds and `Note::ContrastUnreachable` — today a branch no *theme* can
enter, because both shipped floors are under the ceiling — starts firing in
ordinary use. That is why `resolve_with` takes the floors as an argument rather
than reading them off two constants: the day is rehearsed by a test that raises
the text floor to 7 000 and watches fifteen of the eighteen pairs come back
unreachable at 4 582, through the producer that would fire in earnest. Until that
argument existed the note was witnessed by a `Note` a test constructed by hand,
which is a reversal condition nothing can exercise, which is one nobody will
recognise. This RFC is then superseded rather
than amended, because the whole of it rests on the floor being under 4.582, and
the successor has to pick between refusing grounds and abandoning the promise.
`a_floor_above_the_ceiling_is_refused_rather_than_approximated` exists so that
the day this happens the code says so instead of returning colours that do not
work.

**A compositor that draws no rule, or a design language in which a rule is the
wrong answer.** Two reversals wearing one shape. The first is the boundary
obligation going unimplemented: `E3-B01` paints a raised ground with no rule
around it, `Boundary::self_evident` is read by nothing, and every theme with a
subtle raised surface — which is most of them — loses its regions. The signal is
`E3-B01` existing and `Resolved::boundary` having no caller *in it* — not
`boundary` having no caller at all, which is true on the day this RFC is accepted
and every day until that compositor is written, because nothing in the tree
depends on this crate yet. The second is the interesting one: a
design language that separates regions by *elevation* rather than by a line —
shadow, inset, a gap — has a legitimate answer to 1.4.11 that this decision does
not model at all, because `Boundary` carries a colour and nothing else. At that
point the type owes a second kind of answer and this clause is amended rather
than superseded; what would not be acceptable is a compositor quietly choosing
elevation and reporting that it drew the rule.

**Two grounds a theme needs and a rule cannot part.** The one-sidedness above
made concrete: a theme whose `surface-2` sits in the band where its own contrast
against `surface-1` is just under three to one, so a rule is owed, and where no
rule clears the floor on the outer side. The observation is a `Boundary` with
`self_evident` false and `edge_under_x1000` under 3 000, which is reachable today
and is reported rather than refused. If it turns out to be common in real themes
rather than a corner, the answer is to bound the *grounds* after all — this RFC's
second refused alternative, returning with evidence, and it would cost
`Theme::DEFAULT`'s own raised surface. `claims/0035` is where the frequency would
be seen.

**The metric bounds being wrong in a direction that matters.** The observation
is a real display on which nine points is illegible, or a real theme for which
seventy-two points is the reasonable setting rather than the absurd one. Either
one moves a number that is currently a target with nothing behind it, and the
registry row above is where it would be seen first. Note that this reverses the
*bounds* and not the decision: that there are bounds, and that exceeding them is
reported, is what this RFC decides.

The tie between the em and the operable floor reverses differently, and is worth
separating. The observation would be a control that is legitimately shorter than
one em along its flow — an inline toggle whose label sits beside it rather than
inside it is the obvious candidate — for which
`max(CONTROL_MIN_PT_X10, em + 2 * stroke)` is too large and the layout is refused
something reasonable. That would mean the
floor belongs to the *node* rather than to the theme, and `Role` is where it
would then live. What would not reverse it is a theme finding the floor
inconvenient: two legal metrics multiplied into an unusable control is the case
this clause exists for, and it is the same argument the derived em rests on one
level down.

**The contrast table disagreeing with a real renderer.** `E3-B02` will put actual
pixels on an actual display, and the first thing that could refute the
arithmetic here is a pair this module admits at 4.51 which is visibly worse on
that display than a pair it clamped at 4.49. That would mean the error bound is
wrong, or that the display's transfer function is not the one the table assumes —
and the second is the likelier of the two, because this table is sRGB and an HDR
or wide-gamut path is not. A display whose gamut is not sRGB needs the
linearisation to be a property of the display rather than a constant of this
module, and that is a change to what `resolve` takes as an argument.

**The last one, and the one to watch first.** A projection that stops asking
`Resolved::on` for a pair and starts remembering an ink's colour. It will not
look like a reversal — it will look like a cache — and every test in
`token.rs` will still pass while the interface goes quietly unreadable on the
grounds nobody checked. The signal is any type outside this module that holds an
`Rgb` without the `Token` pair it came from.
