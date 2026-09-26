# Licensing

The licence boundary and the isolation boundary are the same boundary.
This is a structural decision, not a paperwork one. See `docs/design/fast-path.html`
section 14.

## The rule

| Tree | Licence | Why |
|---|---|---|
| `abi/`, `env/`, `ring/`, `kernel/`, `user/`, `xtask/` | Apache-2.0 OR MIT | The research must be reusable. Everything written for this project is permissively licensed so results can be lifted by anyone. |
| `third_party/<name>/` holding source | Whatever that source requires: `third_party/harfrust/` is MIT, its crates MIT, Apache-2.0, Zlib or Unicode-3.0 | Imported source, verbatim. Delimited, never mixed into the permissive tree. Its **shim** is written here and lives beside it, not inside it — `user/shaper`, outside the workspace, the one crate `IMPORT_LINKERS` admits to link an import, and only into the import's own component image. RFC 0082, RFC 0141. |
| `third_party/<name>/` holding data | The data's own terms: `third_party/unicode/` is the Unicode License V3, `third_party/inter/` the SIL Open Font License 1.1 | Files **no compiler in this workspace reads** — no `.rs`, `.c`, `.h`, build script or anything an interpreter runs, held by an allow-list of extensions rather than a list of languages, and one font rule: a `.ttf` whose first four bytes are the TrueType version, because nothing here runs its hinting programs (RFC 0141). Reached by two routes and no third: a table generated into the permissive tree by a command, and a file opened by path from a named host test. An import that is both source and data is two imports. RFC 0114. |

## Why this is clean here and messy elsewhere

Imported Linux driver source is GPL-2.0, and a component built from it is a
derivative work. FreeBSD manages the same problem in-kernel and rests the
separation on a linking argument.

F does not have to make that argument. Imported drivers run as isolated
components: **separate address space, no shared symbols, communication only over
a ring**. That is a far stronger separation than linking, and it lands exactly
where the licence boundary needs to be.

Consequence: an imported driver is never linked into the frame, even where the
confinement carries a measurable cost. Safety was already one reason. This is a
second, independent one.

## What a binary owes, since it carries no file headers

Every mechanism above puts the notice in a file: a `LICENSE` beside the import,
a header on each generated table, a licence line a tool can read. **A binary
carries none of them.** So, for whoever redistributes one:

**A binary built from this tree that links `f-text` — the frame's image does —
can contain tables generated from the Unicode Character Database, which are
under the Unicode License V3 as well as this tree's terms; redistributing such a
binary means carrying the copyright and permission notice in
`third_party/unicode/LICENSE` with it, or in the documentation that accompanies
it, as that licence requires.**

A release package already does: `source.tar` carries `third_party/unicode/`
whole, notice included, beside the image.

**A binary built from `user/shaper` — the shaper's component image — links
HarfRust and the crates it vendors**, under MIT, Apache-2.0, Zlib and, for
`unicode-ident`, Unicode-3.0 as well; redistributing it means carrying the
notices in `third_party/harfrust/LICENSE` and in each crate's directory under
`third_party/harfrust/vendor/`. No such binary is distributed today: the image is
built and measured and no frame spawns it (`UNSPAWNABLE`, RFC 0141). No binary
carries `third_party/inter/`; a face is data a component is handed at run time,
and the OFL's notice travels with the file in `source.tar`. The conformance corpus in that
directory never enters a compiled artefact — it is not generated, not embedded
and not compiled, which `cargo xtask lint-boundary`'s `include_str!` net holds —
and it does travel in `source.tar`, because that is the tree at a tag and the
corpus is what a stranger reruns conformance against. RFC 0138.

## Rules that follow

1. No file under `third_party/` may be `use`d from the permissive tree. The only
   permitted coupling to imported **source** is the ring protocol defined in
   `abi/`. Imported **data** has a second, and it is narrow: a table generated
   from it by `cargo xtask unicode` and committed, and a host test that opens a
   corpus file by path. Both are rows in `xtask/src/main.rs` — `DERIVED_DATA`
   for each generated file, `IMPORT_READERS` for each file that opens the import
   at run time — and a permissive source that names the import in code or a
   string literal without a row is a finding, as is a row whose file no longer
   reads it. Imported **source** has one link besides the ring, and it is on
   the far side of the ring: the shim that turns the import into a component,
   `user/shaper`, takes HarfRust by name at one exact version, resolved from the
   import's `vendor/` — the one row of `IMPORT_LINKERS`. No other permissive
   manifest may name any crate an import vendors, or the shim, and no manifest
   may carry a path into `third_party/` at all. RFC 0141.
2. Every file in the permissive tree opens with exactly
   `// SPDX-License-Identifier: Apache-2.0 OR MIT` and carries no second licence
   tag. The exception is a file generated from imported data, which opens with
   `// SPDX-License-Identifier: (Apache-2.0 OR MIT) AND Unicode-3.0`, is a row of
   `DERIVED_DATA`, and carries a header naming its upstream file, that file's
   SHA-256, the Unicode version and the command that regenerates it. There are
   eight such files, every one of them in `text/src/` and each a table of
   values with no code: `bidi_class.rs` and `bidi_brackets.rs` for UAX #9, and
   `line_break.rs`, `grapheme_break.rs`, `east_asian_width.rs`,
   `general_category.rs`, `indic_conjunct_break.rs` and
   `extended_pictographic.rs` for UAX #14 and UAX #29.
3. Every imported tree carries its own `LICENSE` and a `PROVENANCE.md` recording
   upstream URL, commit hash — for data, the version — and the date imported. A
   data import's record also names every file with its byte count and SHA-256,
   and `cargo xtask lint-licensing` reads the record against the bytes: a file
   not byte-identical to upstream is not the specification's data. A source
   import vendored by cargo names every crate with its version and checksum, and
   the check reads it crate by crate and file by file — against the import's own
   `Cargo.lock` and each crate's `.cargo-checksum.json` — and requires the
   `Commit` to carry a forty-digit hash. Code an import runs on the build machine
   — a procedural macro, a build script — is named in `IMPORT_HOST_CODE`, and a
   crate that runs one and is not named is a finding. RFC 0141.
4. Two commands enforce rules 1 and 2 in CI, and they read different things.
   `cargo xtask lint-licensing` reads the **source**: SPDX headers, a permissive
   file naming the imported tree, and a `#[path]` attribute spelling a route into
   it. `cargo xtask lint-boundary` reads the **build**: cargo's own resolved view
   of the workspace, every dep-info rustc wrote, and three surfaces that are
   prohibited rather than inspected — the permissive tree carries no build
   script, no symlink, and no `.cargo/config.toml` row that can redirect a
   build. Each is a prohibition on the *mechanism* and not on a spelling: a
   build script is refused because cargo resolved a target of kind
   `custom-build`, whatever the manifest row was called and whatever the file is
   named, and a configuration row is refused on its key rather than on whether
   its value happens to contain the word `third_party`. An ambient `RUSTFLAGS`
   naming the import is refused too, and that one is an inspection, because the
   environment a command runs in is not this repository's to prohibit.

   The split is the point rather than an accident of history. A route can be
   spelled in more ways than a matcher can enumerate — whitespace and comments
   are admitted between every token of a `#[path]` attribute, an escaped literal
   spells the underscore without writing it, and a symlink names the imported
   tree in no file at all — so the second command stops reading spellings and
   reads what the compiler says it opened. What the first command is still for is
   the one thing the second cannot do: see code the build never compiled, behind
   a `cfg` or a feature that `lint` does not enable.

   **What they do not cover is named rather than left as a remainder**, because
   a rule that claims everything is a rule nobody can check: a route behind a
   configuration this runner never compiles, a proc macro that reads an imported file
   at expansion time, and a copy taken by registry name from somewhere that is
   not `third_party/` — which is `deny.toml`'s ground rather than this file's.
   RFC 0092 holds the argument and the reversal conditions.
