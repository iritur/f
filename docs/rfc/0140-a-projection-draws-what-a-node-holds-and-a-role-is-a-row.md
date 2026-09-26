# RFC 0140: A projection draws what a node holds, and a role is a row

- Status: accepted
- Date: 2026-09-26
- Affects: `semantic/src/draw.rs` (new), `text/src/paragraph.rs` (new),
  `interface/src/reader.rs` and `semantic/src/emit.rs` (the argument, at the
  two tables), `interface/src/token.rs` (its two per-role tables become rows),
  `text/src/bidi.rs` (CR LF is one paragraph separator),
  `text/tests/break_conformance.rs` (the paragraph over both corpora),
  `xtask/src/main.rs` (`lint-projections`), `.github/workflows/ci.yml`,
  `docs/test-taxonomy.toml`, `TODO.md` `E3-B03k`

## Decision

**A text-specific branch is a path through a projection that text takes and no
other role does. A row of a projection's per-role table is not one, on two
conditions, and `cargo xtask lint-projections` holds both.**

`E3-B03k`'s exit forbids *a text-specific branch in either projection*, and
RFC 0104 requires each projection to hold an exhaustive `match` on `Role` — so
each already has a `Role::Text` arm and a `Role::Label` arm. Read literally the
two sentences contradict each other, and one of them has to be given a meaning
before either can be checked. The meaning this takes:

1. **A row is a value, not code.** Its right-hand side is a literal — a struct,
   a variant, a constructor applied to one — with no call, no condition and no
   block. Every role then leaves the table by the same edge and meets the same
   code after it.
2. **The value is not text's alone.** With string literals set aside, the
   `Role::Text` row equals the row of some role that is not text, and so does
   the `Role::Label` row. A row shaped like no other role's is a decision only
   text reaches, which is a branch in a table's clothes. String literals are set
   aside because RFC 0104 confines the language to one field — the noun is the
   one thing a row is meant to own.

A grouped arm — `Role::Label | Role::Text | Role::Image => Self::Text` — that
holds a role which is not text is a shared row by construction: one value,
several roles, text among them. A grouped arm of the two text roles alone is
not, and must equal some other arm like a single row. A file may hold several
tables; `interface/src/token.rs` holds two, the ink a role takes and the floor
it owes, and the display projection paints through both, so both are rows.

Outside the tables neither role may be named in a projection's shipped code at
all, and a glob or grouped import of `Role`'s variants is refused because it
lets a bare `Text` pattern past every needle. A role is compared only with a
named role or with another role, never by its index or name — `role.index() !=
21` is `Role::Text` spelled as a number — and the only methods called on it are
the ones the vocabulary emits from its one list.

**Outside the projections, a text role is a value and never a predicate.** A
projection can take a path only text takes without naming text in its own file,
by calling something that does: `const PROSE: Role = Role::Text` in the
vocabulary, `impl Role { fn is_prose(self) … Self::Text }`, a free function
comparing a role with `Role::Label`. Two audits of this RFC's first lint wrote
each of those and each passed. So every shipped file is read too: an `impl` of
`Role` may not name either text role; a `const` or `static` of roles may not
hold one; `Role` may not be renamed nor asked for by a literal; and anywhere
else a text role appears only as a value handed to a call or a tuple — which is
how a declaration names one, `Node::new(id, parent, Role::Label)` — and never in
a comparison, a pattern or a binding.

**The display projection draws text because a node holds text.** `draw.rs`
decides what is set by an exhaustive `match` on `Content` and whether anything
is drawn by the family of the kind `emit::kind_of` chose — the rule that already
decides where a paint goes. It names no role, and its row in the lint says so,
which is stricter than the rule for a file that holds a table: it may read a
role only by handing it to `kind_of`. A button whose content is *Play* is set
through the same lines as a paragraph of prose.

**A line ends only where the text path offers, over the whole text.** RFC
0139's *a line breaks between clusters* reaches a set paragraph only if every
line end is a position `f_text::line::opportunities` offers over the *whole*
text, and the first version broke that twice at the paragraph split. P1 read a
scalar at a time made CR and LF two paragraphs and a line ended between them,
inside the one cluster GB3 makes of them; and asking for opportunities one
paragraph at a time let LB3 offer each paragraph's end whether or not the text
has a break there. So `bidi::paragraph_len` counts a CR LF as one separator, as
the Unicode Standard's newline guidelines and ICU do, and `bidi::resolve`
accepts one closing a paragraph; the setter asks for opportunities once, over
the whole text, and each paragraph draws its line ends from that one iterator,
so every line end is an item of it by construction. Both conformance corpora of
UAX #9 stay whole: no case holds two paragraphs. Every case text of UAX #14's
and UAX #29's corpora is now also set as a paragraph, in a measure nothing fits
— where the line ends must be exactly the opportunities — and in one everything
fits.

That leaves the one place the two specifications disagree. U+001C, U+001D and
U+001E are paragraph separators to UAX #9 and combining marks to UAX #14, which
offers no break after them before a letter. A line may not cross a paragraph,
and may not end where nothing is offered, so no setting satisfies both, and
`Setting::set` refuses such a text (`Unset::SeparatorMidLine`) rather than
overrule either specification silently. Where UAX #14 does allow a break after
one — before an ideograph — the paragraph's end is offered and ends the line.

**The shaper's seat is a parameter.** `f_text::paragraph` composes UAX #9, the
text path's UAX #14 opportunities and the fixed-point pen, and asks the caller
how many design units each segment between two opportunities advances. RFC
0082's shaper has not landed, and a width invented here would be this tree
pretending it had.

## Context

What `E3-B03k` needed was in the tree as four pieces nobody had composed:
`bidi.rs` (`E3-B03e`), `line.rs` and `grapheme.rs` (`E3-B03f`, RFC 0139),
`metric.rs` (`E3-B03d`), and the reader (`E3-B06i`, RFC 0104). What it needed
and did not find is the part of its own exit that names pixels:

- **No rasteriser draws any node of a declared tree.** `E3-B02e` (fine raster),
  `E3-B02h` (the all-CPU rung), `E3-B03g` (the atlas) and `E3-B06g` (the display
  projection, whose exit is literally *reaches pixels*) are all open. `emit.rs`
  says in its header that it emits no geometry. `kernel/src/screen.rs` draws the
  boot log in a bitmap font inside the frame, and RFC 0081 says it is not the
  presentation path.
- **No shaper exists.** RFC 0082 decided it is imported and named a task to land
  it; `TODO.md` does not carry that task, and `third_party/` holds Unicode's
  data and nothing that maps a scalar to a glyph. `text/src/face.rs` has an
  advance per glyph index and no `cmap`.
- **A reference image is confined to `E3-B03j`'s corpus** by RFC 0091, and
  `E3-B03j` is open. A pixel comparison written here to evidence a render would
  be the carve-out that RFC exists to keep in one place.

So `E3-B03k`'s exit is **not met**, and this RFC does not narrow it. What was
built is the part of it that does not need a pixel: one declaration read by both
projections, checked by address; the paragraph set through the text path's own
stages at a measure that forces breaks, with every line ending where
`f_text::line::opportunities` offered and each line shown in UAX #9's order; the
words set equal to the words said; and the clause about branches made a lint.
The line's `needs:` names `E3-B03f` and `E3-B06i` only, and that is the
decomposition's defect this found: it also needs `E3-B06g` and the shaper import
RFC 0082 wrote out.

Alternatives that were live:

- **Declare the table a branch and restructure it away.** The only structure
  that has no `Role::Text` arm is a table indexed by ordinal, which RFC 0104
  exists to refuse, or a wildcard, which `E3-B06i`'s exit refuses. Both reopen
  the defect the match closed.
- **Check only for `if … Role::Text` outside the table.** Weaker than it looks:
  a row that calls `text_phrasing(role)` or carries a field no other role sets
  is a text path with the table's permission, and a lint that read only outside
  the table would pass it.
- **Write a CPU rasteriser here to meet the exit.** It would be `E3-B02h` built
  inside `E3-B03k` with no rung-1 image to match, over glyphs no shaper
  produced, evidenced by an image comparison RFC 0091 confines elsewhere — three
  lines' work done under a fourth's name, and the claim would be the weakest
  thing in it.

## Consequences

**Easy.** The rasteriser, when it lands, takes a `Setting` — which scalars share
a line, how wide each line is on the device grid, the order each is shown in —
and has no reason to learn a role. The moment it names `f_text::paragraph` it
must be a row of `PROJECTIONS`, because the lint's other direction finds any
file that sets text and has none.

**Hard.** A projection that genuinely needs text treated differently must say so
as a field of its table's row that another role could share. That is the
intended cost: the difference is then visible in the one place a reader looks,
and a second role that wants it gets it by value.

**Foreclosed.** Nothing. A row may still differ from every other in its noun.

**What the lint cannot see**, stated because it is textual. It is why
`docs/test-taxonomy.toml` says the lint **partially** catches the class:

- A text role handed as a value into a local binding, an array or an `Option`
  outside a `const` or `static` of roles — `let prose = [Role::Surface,
  Role::Text];` — and compared later through what holds it, or used as a
  pattern.
- A function outside the projection files that decides by a role's index or
  name — `r.index() == 21` in a file with no row — since only projection files
  are read for that.
- A branch on a noun a table returned (`phrasing(role).noun == "text"`), because
  string literals are not read.
- A screen reader written in a file that neither has a row nor sets text, or a
  crate that reaches the paragraph through `use f_text as t`.

Each is a construct a reviewer sees and nobody writes by habit. `use Role as R`,
which the first version named here, is now refused.

**Where the second condition cannot fire.** The screen reader's `Phrasing` has
two booleans beside its noun, and all four of their combinations are already
some role's row — a group's, an item's, a track's, a label's. So changing the
text row to `entered: true` passes the lint; so does giving the ink table's text
row the status ink, which is a shared value too. By this RFC's definition both
are right rather than holes — text would then take a path another role takes —
but each changes what a listener hears or a reader sees. The first version left
the reader's change in front of nothing but the reader's own tests.
`one_declaration_is_said_and_set_and_the_two_agree` in `semantic/src/draw.rs`
now holds the tree's current answer for both — the label and the paragraph are
each said once and whole, and painted as the button holding text is — so either
change is red there, as a visible decision about how text is heard or looks,
while the lint stays with the definition.

## What would reverse this

- **A listener or a display that needs text handled unlike any other flat
  content** — read by sentence, say, or drawn with a caret. The observation is a
  field on a row that only text sets; the answer is to argue it as a field some
  other role could share, and if none could, this RFC's second condition is
  wrong and the definition is what changes.
- **A shaper whose width for a line is not the sum of its segments'** — kerning
  across a space. Then the seat is a per-line question and `paragraph.rs`
  measures differently; the rule about roles does not move.
- **The rasteriser landing**, at which point `E3-B03k` is re-run with its
  pixels and this RFC's *not met* is what gets reversed, not its definition.
- **A tailoring for U+001C to U+001E** — UAX #14 treating them as mandatory
  breaks, or a higher-level protocol treating them as segment separators —
  argued because some declared text needs them. Then the refusal becomes a
  line end, and the conformance file whose result it moves says so.
- **A revision of P1 that makes CR LF two paragraphs**, which would put UAX #9
  at odds with UAX #14 and UAX #29 over one position.
