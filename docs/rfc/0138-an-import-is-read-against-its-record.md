# RFC 0138: An import is read against its record, and a generated table against its generator

- Status: accepted
- Date: 2026-09-25
- Affects: `xtask/src/main.rs` — `lint_licensing`, `TOOLING`, and three new
  tables, `DERIVED_DATA`, `IMPORT_READERS` and the two licence lines;
  `xtask/src/imported.rs`, new; `text/src/bidi_class.rs` and
  `text/src/bidi_brackets.rs`, generated; `text/src/property.rs`, new;
  `LICENSING.md`'s table, rules 1–3 and a section on binaries;
  `.gitattributes`; RFC 0114, whose four checks this is the diff for and whose
  words it reads narrowly in three places named below; `TODO.md` `E3-B03e0`

## Decision

RFC 0114 specified four checks and deliberately did not write them. This entry
is the diff, and it records the choices the specification left open, because
each is a place a later reader could reasonably have gone the other way:

1. **The licence line is compared to the byte, and a second tag anywhere is a
   second licence.** Every Rust source opens with
   `// SPDX-License-Identifier: Apache-2.0 OR MIT` exactly — a trailing `\r`, a
   missing space or an `OR GPL-2.0-only` is a finding — or is a row of
   `DERIVED_DATA` and opens with the dual line. Outside `xtask/`, the tag may
   appear on no other line in any form; inside it, on no comment line, because
   the tooling writes headers in string literals. A generated file's header is
   the `//` block directly under line 1, and `Upstream-File`, `Upstream-SHA-256`,
   `Unicode-Version` and `Regenerate` must each occur **once in the whole file**
   and inside that block — so a header in a `/* */` block, or a correct header
   with a contradicting decoy lower down, is red.
2. **`IMPORT_READERS` reads code and string literals, decoded, and not
   comments.** RFC 0114 said *any other permissive file containing the literal
   `third_party`*. Taken as text, that is three findings today — `kernel/src/
   screen.rs`, `text/src/corpus.rs` and `text/src/face.rs` name the import in
   prose — and a comment opens nothing. So the reader strips comments (nesting,
   as the compiler's do) and decodes every literal (`\x5f`, `\u{74}`, a
   continued line, raw and raw-byte strings), and the needles are the
   directory's name **and the name of every data file an import holds**, so a
   `concat!` that splits the one and spells the other is still caught.
3. **The provenance comparison is part of the third check, not a fifth.**
   RFC 0114's item 3 names the per-file SHA-256 as a field the record needs; a
   field with no comparison behind it is a hash-shaped string in prose. So *every
   imported tree carries `LICENSE` and `PROVENANCE.md`* is read, for a data
   import, as: every file on disk has a row, every row names a file on disk, and
   each byte count, hash and self-stated version is the one recorded. The
   category's own definition — *files no compiler here reads* — is held by an
   allow-list of extensions (`.txt`, plus `LICENSE` and `PROVENANCE.md`).
4. **There is a fifth check, and it lives inside `lint-licensing`.** A committed
   table must be byte-identical to what `cargo xtask unicode` writes from the
   file its row names, regenerated in memory on every lint. It is inside the
   existing verb rather than a new one because it answers the same row's
   question — *is this file what `DERIVED_DATA` says it is* — and because a new
   verb is a new row in CI that `lint-gate` would, correctly, refuse to go
   without.
5. **The generator decides nothing, and says so where it would have to.**
   Defaults for unlisted scalars are the file's own `@missing` lines; the first
   must cover the code space and the rest may not overlap each other, so the
   order they are applied in decides nothing, and an overlap is a refusal. Where
   the file states `# Total code points` under a value's heading, the resolved
   table must reproduce it: all twenty-three do for `DerivedBidiClass.txt`, which
   is the upstream file checking this tree's reading of its defaults. Names are
   the file's short values, which UAX #9's rules use.
6. **The generated files hold values and no code.** The enum, its `ALL` list,
   the version and the tables are generated; `bidi_class` and `paired_bracket`
   are in `text/src/property.rs` under this tree's terms, with the tables' shape
   asserted at compile time there. A lookup emitted by the generator would be
   code under the data's licence, and the number of files in this tree that are
   not purely liftable — two — would stop being a count of tables.
7. **The corpus travels in `source.tar` and in nothing compiled.** RFC 0114 says
   the corpus *never enters an artefact*; this reads *artefact* as a build
   product. The release package's `source.tar` is `git archive` of the commit —
   the tree at a tag — and a stranger re-running conformance needs the corpus
   exactly as much as the harness does. The packager therefore needs no change,
   and the image is kept clear of the corpus by `lint-boundary`'s `include_str!`
   net rather than by a packaging rule. The same `source.tar` is what carries
   the Unicode notice beside the image, which is what `LICENSING.md`'s new
   sentence about binaries asks for.
8. **An import is never line-ending normalised.** `.gitattributes` marks
   `third_party/**` `-text`. The files are LF and `* text=auto eol=lf` would
   leave them alone today; the rule is for the day a file arrives that
   normalisation would change, which is the day the hash check would otherwise
   go red on some machines and not others.

## Context

`E3-B03e0` was owed by RFC 0114 and waited on an import. The user fetched twelve
files from `unicode.org` on 2026-09-25 — Unicode 17.0.0, the Unicode License V3,
19,016,770 bytes, LF throughout — and the coordinator measured each; this
builder measured them again, independently, and the two readings agreed.

The decisions above are the ones the specification could not make without the
files. Two were found only by having them: `DerivedBidiClass.txt` spells its
defaults with long names (`Left_To_Right`) and its data with short ones (`L`),
so the alias has to come from somewhere, and the file's own `# Bidi_Class=…`
headings are the only table of it that is not a second file; and those same
headings carry totals, which turned *the generator must not choose a default*
from a sentence into a check. And the generator does not read `PROVENANCE.md`:
its inputs are the upstream file and the `DERIVED_DATA` row, and the three
copies of each hash — row, record, header — are held equal to the bytes by
checks 1, 3 and 5 rather than by one reading another.

A project hook refuses any write under `third_party/` from an agent session, so
`PROVENANCE.md` is written by the person who imported the files, from the text
this line's builder measured. That is the hook doing its job — an import is its
own commit — and it means that until the record lands, check 3 is red on this
tree, naming the missing file.

## Consequences

**Easy.** `E3-B03e` has `bidi_class` and `paired_bracket` and a corpus with a
record; `E3-B03f`'s three properties are a `SHAPES` row and a `DERIVED_DATA` row
each, and `every_break_property_this_import_holds_is_a_row` already reads
`LineBreak.txt`, `GraphemeBreakProperty.txt` and `EastAsianWidth.txt` through
the parser `Bidi_Class` uses. A Unicode version bump is red at every step until
the files, the record, the rows and the tables agree.

**Hard.** `cargo xtask lint` now hashes nineteen megabytes on every run. And
`xtask` depends on `f-text`, so a generated table that does not compile cannot
be regenerated by the tool that generated it; the repair is to restore the file
from git and regenerate, which is one command more than it would otherwise be.

**Forecloses.** A lookup, a normalisation or any other code in a generated file,
without an RFC that says why the count of non-liftable files should grow.

## What would reverse this

**A reader the comment-stripping hides.** Point 2 rests on a comment being
unable to open a file. A doc comment is an attribute to the compiler, and a
`#[doc = include_str!(…)]` would be code and caught; if a route is ever found
that turns comment text into a run-time read, point 2 is wrong and the check
goes back to RFC 0114's literal reading, with the three prose mentions reworded.

**A second reader under `xtask/`.** `TOOLING`'s exemption means one would not
be seen. If `xtask` grows a second file that opens the import, the answer is to
move it into `imported.rs` or to narrow the exemption to exclude the needle, and
the observation is a `grep -l third_party/unicode xtask/src` returning two files.

**An `@missing` overlap, or a total that does not reproduce.** Either makes the
generator refuse, which is right; it also means Unicode has published a file
this reading does not understand, and the repair is a reading of UAX #44 written
down here, not a precedence rule added quietly to the parser.

**The corpus becoming the reason a release is large.** Point 7 makes every
package nineteen megabytes larger. If a release is ever refused or split on
size, a `Content` row that excludes the corpus from `source.tar` and ships it
beside, named, is the replacement — never a package that silently lacks it.
