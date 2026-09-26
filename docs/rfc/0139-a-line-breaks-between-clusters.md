# RFC 0139: A line breaks between clusters, and a table is read the way its file writes it

- Status: accepted
- Date: 2026-09-25
- Affects: `text/src/line.rs` and `text/src/grapheme.rs`, new; six generated
  tables in `text/src/` — `line_break.rs`, `grapheme_break.rs`,
  `east_asian_width.rs`, `general_category.rs`, `indic_conjunct_break.rs`,
  `extended_pictographic.rs`; `text/src/property.rs`;
  `text/tests/break_conformance.rs`, new; `xtask/src/imported.rs`, whose
  generator gains two shapes and three readings; `xtask/src/main.rs`'s
  `DERIVED_DATA` and `IMPORT_READERS`; `LICENSING.md` rule 2; RFC 0138, whose
  point 5 and whose sentence *the generator does not read `PROVENANCE.md`* this
  reads more widely in the places named below; `TODO.md` `E3-B03f`

## Decision

`E3-B03f` writes UAX #29's extended grapheme clusters and UAX #14's default
line breaking into `f-text`, holds both to the specification's own files, and
takes five decisions a later reader could reasonably have taken the other way.

1. **The text path breaks a line only between clusters.** The untailored
   algorithm is `f_text::line::decisions`, and it is what `LineBreakTest.txt`
   is run against. What the text path is offered is
   `f_text::line::opportunities`: each UAX #14 opportunity that is also an
   extended grapheme cluster boundary, and no other. The exit's *no break
   falls inside a cluster* is read as exactly that, and asserted over every
   case of both files and the argued eight — every offered break is a cluster
   boundary, every one is a UAX #14 opportunity, every UAX #14 opportunity
   at a cluster boundary is offered, and the offered breaks called mandatory
   are exactly LB3 to LB5's, derived from the `Line_Break` property rather
   than from the module, since the file writes `!` as `÷`. The filter is not
   hypothetical: 1,388 of `LineBreakTest.txt`'s own opportunities fall inside
   a cluster — 1,064 allowed by LB18 (a mark after a space, which LB10 makes
   `AL` and GB9 keeps with the space), 311 by LB31 (spacing marks such as
   U+1BF2 after a non-base), 7 by LB8 and 6 by LB20. The count is pinned in
   the harness and printed on every run, and it is also the control that the
   check can see what it looks for: the untailored breaks are sent through
   the same assertion, and what it reports for them is counted. Mandatory
   breaks are never filtered and need not be: every scalar LB4 or LB5 breaks
   after is a `Control`, `CR` or `LF` to UAX #29, which a unit test holds over
   the whole code space.
2. **Both levels are the files' own *default*, with nothing outside them.**
   `GraphemeBreakTest.txt` and `LineBreakTest.txt` call themselves the
   *Default* tests and, unlike UAX #9's two files, name no rule out of scope.
   So `OUTSIDE_THE_LEVEL` is empty and an entry in `EXCLUDED` has nothing it
   may cite: a failing case is a defect or a renamed level. All 766 and all
   19,338 cases pass.
3. **Thai has no line break inside it, and that is stated rather than hidden.**
   LB1's default resolves `SA` to `AL` or `CM`, so LB28 and LB9 hold every
   position in a Thai sentence. That is `f_text::corpus`'s Thai hazard exactly;
   the default algorithm does not solve it, and a dictionary tailoring is not
   in this line.
4. **`General_Category` is generated whole**, although the rules read five of
   its thirty values — `Mn` and `Mc` for LB1, `Pi` and `Pf` for LB15a, LB15b
   and LB19, `Cn` for LB30b. A table of only those would be a grouping the
   generator chose, which RFC 0138's point 5 forbids it, and the whole table
   reproduces all thirty totals `DerivedGeneralCategory.txt` states, which a
   subset could not be checked by.
5. **The generator reads three file shapes RFC 0138 did not meet.**
   (a) `DerivedGeneralCategory.txt` has no `@missing` line because it lists
   every code point, `Cn` included; a file with no `@missing` line is therefore
   read only if every code point is listed, and one unlisted scalar is a
   refusal. (b) `DerivedCoreProperties.txt` and `emoji-data.txt` each hold
   many properties; the generator reads the lines of the one a row names —
   `InCB` in the middle field, `Extended_Pictographic` in the last — and no
   others, and a binary property's default is the file's own sentence, `All
   omitted code points have Extended_Pictographic=No`, without which it
   refuses. `GraphemeBreakProperty.txt` states its totals under no heading, and
   a total after lines of one value is held to that value. (c) `emoji-data.txt`
   states `Version: 17.0`, which does not name its URL, so the third component
   is taken from the import's `PROVENANCE.md` — **only** if the record is a
   release of what the file said. That is the one place the generator now
   reads the record, which RFC 0138 said it did not: the record is itself
   checked against the bytes by check 3, and the alternative, a version typed
   into `SHAPES`, is a value nobody checks.

## Context

The import already held `LineBreak.txt`, `GraphemeBreakProperty.txt`,
`EastAsianWidth.txt`, `emoji-data.txt` and both corpus files; the user added
`DerivedCoreProperties.txt` (for GB9c's `Indic_Conjunct_Break`) and
`extracted/DerivedGeneralCategory.txt` (for LB1) for this line. The first
builder of the line lost its connection after generating the tables; the
second read its work, kept the generator changes and the grapheme module with
tests added for each new reading, and wrote the rest.

The LB25 this corpus tests is fifteen parts rather than the older list of
pairs, and `HH` is a class the older rule lists do not name; the corpus, not
memory, is what settled both.
The first run of `LineBreakTest.txt` failed sixteen cases, all `IS × NU`
outside a number, which is LB25's fourteenth part; nothing else failed.

## Consequences

**Easy.** A caret, a selection and a line all move by the same unit, so a
line never ends between a base and a mark the reader sees as one character.
The next table is a row, in any of four shapes. `E3-B03g` and the shaper get
opportunities with a rule and a mandatory flag.

**Hard.** Two algorithms run per scalar on the text path instead of one. The
filter is a tailoring, so a reader comparing this path to another UAX #14
implementation sees 1,388 fewer opportunities in the corpus and has to find
this entry to learn why. Eight files in the permissive tree now carry a second
licence, all tables.

**Forecloses.** A break inside a cluster on the text path, without an entry
here that says for which script and why.

## What would reverse this

**A script whose readers expect a break between two scalars UAX #29 joins.**
Point 1 rests on there being none. The evidence would be a corpus entry whose
argued breaks the filter removes; the repair is a tailoring of UAX #29 argued
for that script, not a route around the filter.

**The first surface that sets Thai narrower than its sentence.** Point 3
becomes a defect then, and the repair is a dictionary imported with a record
and a tailoring of LB1's `SA` resolution named in the level.

**A header that puts a rule out of scope.** Point 2's empty list is a reading
of two headers; a Unicode version whose file says a rule is untested or
tailored makes that rule, and only it, a row of `OUTSIDE_THE_LEVEL`.

**`emoji-data.txt` stating its version in full**, or moving into the UCD's
first-line convention. Point 5(c) then has no reason to exist and the
generator stops reading the record.

**The count of 1,388 moving without the Unicode version moving.** Either the
filter or the check changed; the harness is red either way, and the answer is
in the diff, not the constant.
