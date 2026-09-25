# Licensing

The licence boundary and the isolation boundary are the same boundary.
This is a structural decision, not a paperwork one. See `docs/design/fast-path.html`
section 14.

## The rule

| Tree | Licence | Why |
|---|---|---|
| `abi/`, `env/`, `ring/`, `kernel/`, `user/`, `xtask/` | Apache-2.0 OR MIT | The research must be reusable. Everything written for this project is permissively licensed so results can be lifted by anyone. |
| `third_party/<name>/` holding source | Whatever that source requires | Imported driver source and its shim. Delimited, never mixed into the permissive tree. |
| `third_party/<name>/` holding data | The data's own terms: `third_party/unicode/` is the Unicode License V3 | Files **no compiler in this workspace reads** — no `.rs`, `.c`, `.h`, build script or anything an interpreter runs, held by an allow-list of extensions rather than a list of languages. Reached by two routes and no third: a table generated into the permissive tree by a command, and a corpus opened by path from a named host test. An import that is both source and data is two imports. RFC 0114. |

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
whole, notice included, beside the image. The conformance corpus in that
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
   reads it.
2. Every file in the permissive tree opens with exactly
   `// SPDX-License-Identifier: Apache-2.0 OR MIT` and carries no second licence
   tag. The exception is a file generated from imported data, which opens with
   `// SPDX-License-Identifier: (Apache-2.0 OR MIT) AND Unicode-3.0`, is a row of
   `DERIVED_DATA`, and carries a header naming its upstream file, that file's
   SHA-256, the Unicode version and the command that regenerates it. There are
   two such files, `text/src/bidi_class.rs` and `text/src/bidi_brackets.rs`, and
   they hold values and no code.
3. Every imported tree carries its own `LICENSE` and a `PROVENANCE.md` recording
   upstream URL, commit hash — for data, the version — and the date imported. A
   data import's record also names every file with its byte count and SHA-256,
   and `cargo xtask lint-licensing` reads the record against the bytes: a file
   not byte-identical to upstream is not the specification's data.
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
