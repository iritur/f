# RFC 0141: The shim links the import by one row, and the shaper is declared before it can run

- Status: accepted
- Date: 2026-09-26
- Affects: `user/shaper/` (new: the shim, its manifest, its host test);
  `abi/src/shape.rs` (new: the `shape` protocol); the root `Cargo.toml`'s
  `exclude`; `xtask/src/main.rs` — `IMPORT_LINKERS`, `LINKERS_MAX`,
  `IMPORT_HOST_CODE`, `UNSPAWNABLE`, `IMPORT_COMPONENT_CAPS`, a fourth
  `IMPORT_READERS` row, `IMPORT_CHECKSUMS`, `linker_row_findings`,
  `cargo_view`, `patch_table_findings`, `shim_lock_findings`,
  `linker_lock_findings`, `checksum_anchor_findings`, `host_code_findings`,
  `import_component_findings`, `flat_checks` (split out of `flat_image_with`),
  `walker_skips`, `FORBIDDEN_TYPES`' `f16` and `f128`, `config_row_redirects`'
  `[patch]` and `[replace]`, the cargo configuration file named `config`, and
  the shim's steps in `test-host`, `cross`, `test`, `lint-style` and
  `lint-boundary`; `xtask/src/imported.rs` — the `Archive` and `Crate` tables,
  the font rule, the source-import check, `IMPORT_LICENCES`,
  `data_file_names`, `host_code`, `vendored_crate_dirs`;
  `xtask/src/shaper.rs` (new); `text/src/face.rs`, whose format gives way;
  `user/objects/manifest.toml`'s `[[face]]` digest; RFC 0082's *What cannot be
  observed* section and the test that holds it; `LICENSING.md`; `TODO.md`
  `E3-B03b0`. RFC 0003, RFC 0005, RFC 0082, RFC 0092, RFC 0114, RFC 0138.

## Decision

**The shaper's shim is a permissive crate outside the workspace, `user/shaper`,
and it is the one manifest in the tree allowed to link an import — by name, at
one exact version, resolved from the import's `vendor/` and from nowhere else.**
The exception is a checked row, `IMPORT_LINKERS`, and everything that would widen
it is red. The shaper is declared in `user/shaper/manifest.toml`, RFC 0082's three
properties are checks on that file, and the component is **not spawned** because
its image is nine times what a spawn shape maps — a row, `UNSPAWNABLE`, whose
reason `cargo xtask test` measures on every run. `E3-B03b0` settled eight things,
and each is written where its code is:

1. **Where the shim lives** — `user/shaper`, its own workspace, in the root's
   `exclude`. `third_party/` is imported verbatim, a re-import replaces it whole,
   and a hook refuses an agent's write there; the shim is this tree's code, so it
   is written here, under this tree's SPDX line and `unsafe_code = "forbid"`. It
   is not a workspace member because a member that linked the import would put
   fourteen imported crates into the workspace's lockfile, its `cargo metadata`
   and every `cargo test --workspace`, none of which can resolve them without a
   source replacement. `IMPORT_LINKERS` (in `xtask/src/main.rs`) says the rest.
2. **The root `exclude` gains `"third_party"`** — the row RFC 0082 owed — and
   `"user/shaper"`. `linker_row_findings` requires both.
3. **The import's procedural macro and its three build scripts are accepted as
   the import's code**, named one by one in `IMPORT_HOST_CODE` with what each
   does, and compared with the vendored crates in both directions on every lint.
4. **A face is admitted into a data import by one narrow rule**: the extension
   `ttf` *and* the TrueType `sfntVersion` `00 01 00 00` in the first four bytes
   (`DATA_FONT` in `xtask/src/imported.rs`). The data record's `Archive` table is
   read, checked for shape, and never looked for on disk — and it is **required**:
   an archive `Retrieved with` names has a row, and a row names an archive
   `Retrieved with` names. Its hash is a record of what was fetched, and no check
   can confirm it without the archive, which is not kept.
5. **The `shape` protocol is `abi/src/shape.rs`**: a fixed-width request (the
   face by content address, the script, the direction, up to eight features, up
   to 128 bytes of UTF-8) and a reply of a head (the grid) and glyph records
   whose four positions are `_design_units` integers. No size crosses, because a
   design-unit position does not depend on one.
6. **RFC 0082's three properties are checks in `lint-manifests`**, on the
   manifest whose `image` is the linker's directory: no `[[device]]`, only
   `untyped` and `endpoint` capabilities, every one `from = "supervisor"` — no
   `powerbox` ask and no `sibling:` handle, which is another component's endpoint
   or memory and so a route a clock could reach the shaper by; exactly one
   `heap`, `untyped`, from the supervisor, with `bytes`; every ring a `shape`
   server, and `abi/src/shape.rs` naming no binary floating-point type — `f16`
   and `f128` as well as `f32` and `f64`, since a feature gate is one line. RFC
   0005 rule 4 is carried across to that image: it may not be `shared`.
7. **Three runs of text in Inter 4.001 Regular are shaped through the protocol
   by a host test**, `user/shaper/tests/run.rs`, against expectations derived
   from the font's own tables before HarfRust ran. `Tokyo -> Vienna` left to
   right: glyph ids from `cmap`, advances from `hmtx`, the arrow from `GSUB`
   lookup 47 under `calt`, and −160 and −40 from `GPOS` lookup 1's class pairs,
   15,632 design units. The same run right to left: visual order, clusters
   descending, `>` mirrored so that `calt` lookup 48 makes the left arrow, and
   four other class pairs, 15,596. And `h` with U+0301: under `Latn` the accent's
   offset is (−1210, 372), `GPOS` lookup 8's two anchors meeting; under `Cyrl`,
   whose `GPOS` script lists no `mark` feature, it is (0, 0). The shim hands the
   import only a face its manifest declares, and a panic in its image is a
   fault (below). The tests run in `cargo xtask test-host`, which is what both
   CI test jobs run.
8. **`text/src/face.rs`'s format gives way**, as that module said it would the
   day a real face arrived: `Face::read` reads the OpenType table directory and
   `head`, `hhea`, `maxp` and `hmtx`; `GLYPHS_MAX` is the format's own 65,535;
   the old format is deleted rather than kept beside it; `compose` writes a
   minimal OpenType file, so the declared face's digest moved.

## Context

`E3-B03b0`'s exit is three observations: the import's `LICENSE` and
`PROVENANCE.md`, `cargo xtask lint` green with the entry present, and one run
through the `shape` protocol producing fixed-point advances on both
architectures. The two imports landed a commit before this one (`b978a76`,
`48811d4`) and `lint-licensing` was red with them in the tree, for three reasons
that were each a reader meeting a shape it had not been written for:

- **Three hundred false readers.** `IMPORT_READERS`' second needle is *every
  file name a data import holds*, on the ground that a path can be assembled
  without spelling `third_party` and still spell the file. `data_file_names` read
  every import, and HarfRust's `vendor/` holds `lib.rs`, `mod.rs`, `README.md`
  and `Cargo.toml`. Source imports are now skipped: nothing opens source at run
  time, it is linked by the one row or not reached, and the directory needle
  still guards it.
- **Tables the record reader could not read.** HarfRust's record has a
  five-column `Crate` table and Inter's a three-column `Archive` table beside its
  `File` table. The reader now tells four tables apart by header and width, and a
  row under an unknown header is still a finding.
- **A font in a data import.** The data category is *files no compiler in this
  workspace reads*, held by `txt`. A TrueType file carries hinting programs, which
  an interpreter runs, so it is not inert in general; it is data *here* because
  nothing in this tree runs them — HarfRust reads `cmap`, `GSUB`, `GPOS` and
  `hmtx`, and `f_text::face` reads four tables of integers.

### Why the link is by name and not by path

The first version of this change linked HarfRust by path and admitted exactly one
path row from exactly one manifest. It was withdrawn for two reasons, both
measured: cargo verifies the `.cargo-checksum.json` of a crate it takes from a
vendored *directory source* and verifies nothing of a crate it takes by path, so
`Changed: nothing` would have been true of thirteen crates and assumed of the one
that matters; and cargo prints a path crate's own lints and manifest warnings as
this tree's, into a `verify` whose healthy output has none. So the shim takes
`harfrust = { version = "=0.13.3" }`, and `xtask/src/shaper.rs` passes cargo's
source replacement on the command line with `--offline --locked`. On the command
line and not in a `.cargo/config.toml`, because `lint-boundary` prohibits a
configuration row that redirects a build and a source replacement is exactly
one; the one function that spells it is under `TOOLING`.

What a path row would have made visible, `IMPORT_LINKERS` makes visible instead,
and more: **no permissive manifest may name any crate a source import vendors**,
in any spelling cargo accepts — a key, a `[dependencies.<name>]` header with or
without spaces round its dots, a top-level dotted key, a target table,
`[workspace.dependencies]`, or `package =`. The dependencies are not scanned
line by line; they are cargo's own reading, `cargo metadata --no-deps` run in
every workspace the tree holds (`cargo_view`), and a workspace cargo cannot read
is a finding rather than a skip. A scanner was what the first version had, and
three spellings passed it with cargo taking the dependency: a top-level
`dependencies.harfrust = …`, a header `[dependencies . harfrust]`, and a crate
under `docs/` that a member took by path — the walkers skipped `docs` and
`target` at any depth, and now skip them only at the root, and a nested
`target` only where cargo's `CACHEDIR.TAG` marks it beside an excluded crate
(`walker_skips`). That reads a route
`BOUNDARY_BLIND`'s third entry declared invisible — *a copy taken by registry
name* — for the crates this tree has vendored. It does not close that entry: a
registry crate this tree has *not* vendored is still invisible, and the entry
stands. `lint-licensing`'s path check is untouched and still refuses every path
into `third_party/`. And because the shim's crate would link the import one hop
away, **no manifest may name the shim's directory** either.

**Where the name resolves is checked, not assumed.** A `[replace]` or `[patch]`
table points `harfrust` at another copy without changing a character of the row
`IMPORT_LINKERS` reads, and `cargo metadata --no-deps` reports neither table.
Measured: a `[replace]` in the shim's manifest, and a `[patch.crates-io]` in
`user/shaper/.cargo/config.toml`, each built the shim from an edited copy under
`docs/` with every lint green. So both tables are **prohibited** — in every
permissive manifest, in any spelling (`patch_table_findings`), and in every cargo
configuration file the shim's build reads: `.cargo/config.toml` *and* the
extensionless `.cargo/config` cargo still reads, in the shim's directory and every
directory above it, and in `CARGO_HOME` (`linker_lock_findings`).
`lint-boundary`'s configuration prohibition treats `[patch]` and `[replace]` as it
treats `[source]` and `[env]`, and reads a file named `config` as it reads
`config.toml`; the extensionless name was not read for a round, and a `[patch]`
in it took `unicode-ident` from the vendored directory into the kernel's build
green. The belt under both is the shim's lockfile, which `--locked` holds its
build to: every package in `user/shaper/Cargo.lock` is either this tree's own (no
source, a crate cargo's view of the tree has at that version) or the import's
(crates.io, at the checksum the import's own `Cargo.lock` records), and nothing
else (`shim_lock_findings`).

`lint-licensing` also verifies what cargo would, without a build: every vendored
crate's directory against a `Crate` row, each row's checksum against the
import's own `Cargo.lock` and the crate's `.cargo-checksum.json`, and every file
against the SHA-256 that file lists — every file listed, nothing unlisted on
disk, nothing at the import's top level but `LICENSE`, `PROVENANCE.md` and
`Cargo.lock`. The source record's `Commit` must carry forty hex digits, which is
the exit's *a commit hash*.

Two things cargo does not verify are verified here as well. **The checksum file
is anchored outside the directory it vouches for.** A vendored file edited
together with its line in `.cargo-checksum.json` was green in `lint-licensing`
and in cargo's own build, because the list lived beside what it listed;
`IMPORT_CHECKSUMS` records the SHA-256 of each crate's checksum file, both
directions are compared (`checksum_anchor_findings`), and an edit to a vendored
crate is then an edit to `xtask/` a reviewer sees. It belongs in the `Crate`
table as a column, and is a table in `xtask/` only because the record is written
by a re-import and none has happened since. **And each crate's licence is read.**
`cargo deny` runs over the workspace and the shim is outside it, and the
`Licence` column was parsed and dropped, so a crate re-licensed GPL-3.0 in its
manifest and its checksum file was green. Each vendored crate's own `license`
must now equal its row's, and every licence either names must be in
`IMPORT_LICENCES`: `deny.toml`'s list and `Zlib`, `bytemuck`'s, with a test
holding the two lists together.

### Why the image cannot be spawned, and what was built instead

`-Zbuild-std` — how every component image is built — resolves the standard
library's own lockfile, and with `crates-io` replaced by the import's `vendor/`
the standard library's dependencies are not there. So the shim is compiled
against the prebuilt `core` and `alloc` for `x86_64-unknown-none`, as an `rlib`
(which carries `component::start`) and a `staticlib` (which carries the import and
the allocator shim rustc writes only for a final artefact), linked by
`user/init/link.ld` and held to the same checks as every component
(`flat_checks`): the entry at the first byte, no writable data. **The image is
607,696 bytes; a spawn shape maps 65,536.** The frame copies a flat image into a
fixed text reservation and reads no headers (`E5`), so no frame in this tree can
spawn it. That is not RFC 0082's third reversal condition — *the component cannot
be started … on every machine this project has* — being met; it is the frame's
spawn shape being smaller than the first component that is not this tree's own
code. It is recorded as `UNSPAWNABLE`, which `lint-components` keeps from being a
place to park any other component (the row's image must be a linker's), and
whose reason `shaper::unspawnable` measures and turns red the day it is false.

`user/shaper/src/component.rs` therefore serves no ring. It takes `serve`'s
address through an opaque barrier, so the linker keeps what a serving shaper
carries and the measurement is of that, and exits with `NOT_SERVING`. A ring loop
written today would be code no boot can run.

### What the shim refuses before the import runs, and how the image ends

**A face its manifest does not declare.** The shim checks a request's face
address against `FACES`, the addresses `user/shaper/manifest.toml`'s `[[face]]`
tables declare — a test holds the two lists equal — before it hashes the bytes,
before `f_text::face::Face::read` admits them, and before HarfRust sees them.
The hash alone would not do: it says the bytes are the bytes somebody named, not
that anybody vouched for them. Measured out of tree, over 1,500 copies of Inter
with one to eight bytes changed inside one of ten tables, each named by its own
hash, with `f_ring::heap::Heap` over a host region as the allocator: every copy
passed the hash and `Face::read`; seven, each with `GSUB` or `GPOS` changed, made
`ShaperData::new` ask for one block of 14 to 22 megabytes, sized from a lookup or
subtable count read out of the font; and sixteen more shaped with the heap's
high-water mark past the manifest's 131,072 bytes. `tests/run.rs` replays that
generator from its seed and serves those twenty-three faces, and every one is
refused as undeclared. A face the import would read and this tree does not
admit — Inter with a reserved `hhea` word set, re-hashed and declared — is
refused by `Face::read`, and with that call taken out it shapes.

**No budget on `GSUB` and `GPOS` lookup and subtable counts, and that was
decided rather than skipped.** With the address pinned, the counts the import
reads are Inter's own, so a budget would be checking constants against
themselves; and it would bound the two allocations one fuzz run found and not
the import's allocation in general, which the heap bounds instead. The reversal
below says when that stops being true.

**A panic is a fault.** A refused allocation in the image is a panic, and the
panic handler is `core::intrinsics::abort` — in the linked image
`rust_begin_unwind` is one `ud2` — so it ends in an invalid-opcode exception at
ring 3, which is what the manifest's `on_fault` respawns after
(`docs/manifest.md`: a fault is *an exception at ring 3*, and `on_fault` does
not respawn after an exit). Not the door's `EXIT`, then, which would leave the
place empty; and not a spin, which is what it was, and which would hold a core
with nobody told. `tests/run.rs` holds the handler's text to it, because no host
test can run a handler compiled for the image only.

### What "on both architectures" means in this tree

There is no AArch64 frame (`PORTABILITY`'s `f-kernel` row), so no boot on
AArch64 exists to run. What runs, and where:

| what | x86-64 | AArch64 |
| --- | --- | --- |
| the runs through the protocol, `user/shaper/tests/run.rs`, in one process with no ring and no envelope | locally, in `cargo xtask test-host`, and in CI's `tests (x86-64)` | in CI's `tests (AArch64, weak memory)` on `ubuntu-24.04-arm`, which runs `cargo xtask test-host`; **not observable from this container** (`ARCH_RUN_GAP`) |
| the shim compiled for the machine | the component image, linked and measured in `cargo xtask test` | `cargo check --target aarch64-unknown-none`, in `cargo xtask cross` |
| the component running on a frame | not possible: `UNSPAWNABLE` | not possible: no AArch64 frame |

The expectations are literals, so *the same on both* is each runner agreeing with
one argued table — `f_text::cache`'s arrangement — and a position that moved on
one architecture is red there. That is RFC 0082's second reversal condition made
executable for these runs and one face; the full condition is `E3-B03i`'s corpus
shaped on both, which needs a face per script and is later work. Each run writes
one `shaped … on <arch>:` line with every glyph id, cluster, advance and offset,
past the test harness's capture, so both jobs' logs carry it; the AArch64 line is
evidence once that job has run on a commit carrying this change, and not before.

### The face format

`f_text::face` read a format of its own whose bound was 1,024 glyphs; Inter has
2,937. The module's own reversal said *a real face format arriving whose own
header carries a units-per-em and a glyph count … `Face::read` reads that header
instead … and the format below is deleted rather than kept beside it*, and that
is what was done. One claim of the old module did not survive: *every byte is
judged*. A real face has hundreds of kilobytes an admission check never reads,
so the claim is narrowed to what still holds — the directory judged whole, every
table inside the blob, the tag order strict, and nothing after the last table but
zero padding — and the content address, which was always what named every byte.
`compose`'s own reversal (*a real face in `third_party/` … this function has no
caller outside a test, and it goes*) is **not** met: the frame composes and hashes
the declared face itself in `E3-B03b`'s boot, and the frame cannot reach
`third_party/`, so `compose` now writes a minimal OpenType file and
`user/objects/manifest.toml`'s digest moved from `1ecfbbb8…` to `924f1f78…`,
computed twice — by the Rust composition and by an independent script — before
it was written down.

## Consequences

**What it makes easy.** Every later shaping line — the cache holding real
answers, `E3-B03k`'s rendered paragraph, a corpus of faces — has a protocol, a
shim and a face to call. The import's integrity is checked on every lint without
a build. A re-import that brings a new build script, a new crate or a changed
file is red until somebody has read it.

**What it makes hard.** A second import that needs a shim is an RFC before it is
a row (`LINKERS_MAX`). The shim's builds cannot use `-Zbuild-std`, so its image
differs from every other component's in its panic strategy — `abort` rather than
`immediate-abort` — and carries the formatting code a panic message needs; the
measurement is an upper bound by that margin. `third_party/README.md` still says
*the only permitted coupling to imported source is the ring protocol*, which is
now true of the permissive tree less one named crate; that file is under
`third_party/`, which no agent may write, so the sentence is owed to whoever
owns it.

**What it forecloses.** Building the import's crates as workspace members, and a
path row into `third_party/` anywhere.

## What would reverse this

- **The shim moving into the import's tree**, if a re-import stops being a
  whole-directory replacement. Then the shim is imported source's own shim, as
  `LICENSING.md`'s table once said, and `IMPORT_LINKERS` goes.
- **`UNSPAWNABLE`'s reason going false**: a frame that reads an image's headers
  and maps what it finds (`E5`), or a shaper that fits in sixteen pages. Then the
  row goes, the component is added to `COMPONENTS` with a spawn shape, and
  `component.rs` serves the ring — and the envelope rule `abi/src/shape.rs` owes
  is written in the same diff.
- **The host measurement being wrong about the heap.** `user/shaper/manifest.toml`
  sizes `heap` at 131,072 bytes from an out-of-tree measurement of 86,056 at
  peak; the frame's `heap_peak` on the day the component runs replaces it.
- **A face admitted that nobody in this tree vouches for** — a download, a
  document's embedded font, RFC 0005's `hostile` row. The declared address then
  no longer bounds what the import reads, and a budget on `GSUB` and `GPOS`
  lookup and subtable counts, checked before the import is called, is the first
  of the checks owed.
- **The import's record carrying each checksum file's SHA-256**, as a column of
  its `Crate` table, on the next re-import: `imported::source_findings` then
  reads the anchor there and `IMPORT_CHECKSUMS` goes. Or cargo anchoring a
  vendored directory itself, which it does not today.
- **A manifest in this tree needing a `[patch]` or `[replace]` table** — a
  security fix to a registry crate ahead of its release, say. That is an RFC
  naming the table and why, and a row somewhere, not a relaxation of the
  prohibition.
- **Something in this tree running a face's hinting programs** — Skrifa's hinter,
  `docs/TECHNICAL-DEBT.md`'s planned import — which makes a `.ttf` code rather
  than data *here*, and the font rule is then the rule to revisit.
- **A build script in `IMPORT_HOST_CODE` that reads outside its own directory or
  reaches the network**, found on a re-import's review: a different version or a
  different upstream, not a wider table.
- **An AArch64 run of `user/shaper/tests/run.rs` that disagrees with the argued
  table.** That is HarfRust's floating point showing through the shim, RFC 0082's
  second reversal condition, and it refutes this upstream before it refutes the
  decision.
