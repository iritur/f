# RFC 0092: A boundary is checked where the build answers, and the routes it still cannot see are named

- Status: accepted
- Date: 2026-09-18
- Affects: `TODO.md`, whose `E3-B03a` exit line is replaced below and annotated
  with this number beside its existing RFC 0088 citation;
  `intent/0012-the-interface/spec.md`, whose `E3-B03a` line carries the same
  sentence because RFC 0088's *Affects* calls that file a paste-ready handoff;
  `xtask/src/main.rs`, which gains three finding functions and a subcommand and
  keeps its textual net with a narrower job; `LICENSING.md`, whose rule 4 says
  what the lint enforces and will be enforcing five things rather than two
- Does not affect RFC 0003, which is the policy. This entry is about the
  *mechanism* that observes it and about the sentence a task closes on

## Decision

Two parts. The second is the one that costs something.

**One. The licence boundary stops being checked by reading spellings and starts
being checked by reading what the build did.** Four nets, of which three are new:

- `cargo metadata --no-deps --offline --locked` is parsed by Cargo rather than
  by us, so every manifest spelling — multi-line strings, dotted keys, split
  keys, member globs, `build =`, `[lib] path`, `[patch]`, workspace inheritance
  — closes at once, and a `cargo metadata` failure is a finding rather than an
  `Ok`.
- Every `*.d` rustc emits is walked, each prerequisite resolved and
  canonicalised, and any path with a `third_party` component is refused. This is
  the compiler's own answer to *what did you read*, so `#[path]` in every
  spelling, raw strings, escaped literals, attributes split across lines, a
  comment between the key and its `=`, `include!`, `include_str!`,
  `include_bytes!`, macro expansion and symlinks stop being nine patterns and
  become one finding.
- The build-script, symlink and configuration surfaces are **prohibited rather
  than inspected**. The permissive tree may carry no build script — no target of
  kind `custom-build` in cargo's resolved view, whatever the row that declared it
  was called and whatever the file is named — no symlink, and no
  `.cargo/config.toml` row that can redirect a build. Each is an allow-list.
  This is the half that closes `OUT_DIR` laundering and the static-link family
  at the mechanism instead of chasing their traces.

The existing textual net stays, with its job rewritten to the one thing the
other three cannot do: see code the build never compiled.

**Two. `E3-B03a`'s exit loses the word *every*, because no mechanism in this
tree can honestly carry it.** The sentence becomes:

> `cargo xtask lint` refuses every route from the permissive tree into
> `third_party/` that reaches code the lint compiles, and prohibits the build
> script, symlink and configuration surfaces outright rather than inspecting
> them; with a fixture driving each route red, and a fixture recording each of
> the three routes it cannot see.

The three are named rather than left as an unspecified remainder, which is the
whole difference between a scoped claim and a broken one:

1. **A route behind a configuration the lint never compiles.** `lint_style`
   clippies the host world and `x86_64-unknown-none`; the AArch64 world is
   compiled by `cargo xtask test`, not by `lint`. Anything behind
   `cfg(target_arch = "aarch64")` or an off-by-default feature is seen only by
   the textual net, and only if its path is spelled rather than composed by a
   macro.
2. **A proc macro that reads an imported file at expansion time.** The bytes
   reach the crate without the file becoming a prerequisite of it.
3. **A dependency taken by registry name on a vendored copy outside
   `third_party/`.** That is `deny.toml`'s ground and outside this exit's own
   words, and is recorded here so the next reader does not mistake the
   boundary's silence for the licence policy's.

The three are carried in code as `BOUNDARY_BLIND` in `xtask/src/main.rs`,
printed in `lint-boundary`'s success line the way `RING_PROOF_BLIND` is printed
in `lint-proofs`', and each has a fixture that exercises its mechanism and
asserts the nets are **silent**. That direction is what makes the list
self-removing: a residue somebody closes turns its fixture red and has to be
deleted deliberately. Without it the clause *a fixture recording each of the
three routes it cannot see* is discharged by this paragraph, and a paragraph is
what can be deleted, closed or joined by a fourth with nothing going red.

A stale target directory is a fourth way the second net reads yesterday's
answer, and it is a property of this environment rather than of a route; it is
recorded under *Consequences* rather than in the sentence.

### A sentence this entry got wrong, corrected by its own first run

The draft accepted above said, of the second net: *a prerequisite that will not
canonicalise is judged lexically rather than skipped, because skipping is how a
check reads a failure as an absence*. **That is wrong, and the first run of the
net refuted it within the hour.**

A probe had reached the import by a raw-string `#[path]`, been reverted, and
left its dep-info behind. The imported file no longer existed, so `canonicalize`
failed, so the lexical fallback fired, and the net reported a live boundary
violation against a route that had been deleted. The fixture that asserts the
real tree is clean is what caught it.

Working the cases shows the fallback was never earning anything. A file that
exists canonicalises, so a real route is caught by the resolving path. The only
entry the lexical path adds is one naming a file that is **gone** — which is a
route that cannot be compiled today and that cargo rewrites on the next build.
So the fallback caught no route the resolving path missed and produced a false
positive on every stale tree.

The rule is now: **a prerequisite that no longer exists is skipped and counted,
and the count is printed in the success line.** That keeps the original concern
honest — a skip nobody can see is how a check reads a failure as an absence — by
making the skip visible rather than by turning it into a finding. The reasoning
that produced the wrong sentence is worth naming, because it will recur: *fail
closed* is a good instinct that becomes a false positive the moment the failing
condition is more common than the thing being guarded against.

### Two sentences this entry got wrong about its own mechanism

**"Each is an allow-list that is empty today" was true of two of the three.**
There was no configuration allow-list at all. The configuration rule shipped as
an *inspection* — a row was refused when its value contained the string
`third_party` — which is the thing this entry argues against, in the paragraph
that argues against it. A `[source]` replacement, an `[env]` row, a linker
wrapper and a relative `-L` into a copy taken elsewhere each pass that check
while reaching an imported tree. The rule is now the key rather than the value:
`rustflags`, `rustdocflags`, `rustc`, the three wrapper keys, `linker`, `runner`
and `paths`, plus any row under `[env]` or `[source]`, are refused unless
`CONFIG_ALLOW` carries them with a reason. **That list is not empty**, and it
cannot be: this repository's own `.cargo/config.toml` carries `build.rustflags`
and `target.x86_64-unknown-none.rustflags`, without which reproduction claims are
false and the kernel image does not link. Two rows with their reasons written
down is what this decision costs, and it is a smaller cost than a surface that
reads *prohibited* and behaves like a grep.

**"No `build.rs`, no `build =` or `links =` row" was two spellings, not a
mechanism.** TOML admits `"build" = "make.rs"` and `package.build = "make.rs"`,
which are the same manifest to cargo and neither of which begins with `build`
once trimmed, and the file need not be called `build.rs` at all. The
prohibition now reads cargo's resolved view for a target of kind `custom-build`,
which is indifferent to all of that; the filename check stays as the belt for a
crate cargo cannot resolve. Both errors have the same shape and it is the shape
this entry was written to remove: the argument moved to mechanisms and two of
the three implementations stayed at spellings.

## Context

Three adversarial review rounds have now defeated this check, each by about one
character. A single quote beat one round. A raw string, a newline, an `include!`
and a `build =` row beat the next. The fourth round was being prepared as four
more patterns when the question was asked properly.

**The enumeration does not terminate, and that is a fact about Rust rather than
about the checker.** Arbitrary whitespace and comments are admitted between
every token of `#[path = "…"]`; `reaches_import` compares raw text, so
`third\u{5f}party` is eleven characters that are not the token; `path_attr_findings`
requires `#[` on the line it is scanning and discards any line whose trim starts
with `*`; and the symlink and plain-`mod` routes spell `third_party` in no file
at all. A line-oriented text matcher cannot make a universal claim over a grammar
it does not parse, and adding a fifth pattern buys exactly one more round.

A sweep enumerated **twenty-nine distinct routes**, twenty-six of which the four
nets close, and the two mechanisms considered and rejected are worth recording
because both are tempting. A hand-written Rust parser in `xtask` is the same
mistake one level down: a parser is a model of the grammar, and the compiler
already holds the answer and writes it to disk on every build. And banning the
token `third_party` across a file set — the right instinct — dies to
`"third\u{5f}party"` and to `["third","party"].join("_")`, both of which were
compiled during the sweep. The inversion survives; its object changes from
*strings* to *mechanisms*, which no escape sequence evades.

**Why the sentence is narrowed before the code is written, rather than after.**
RFC 0084 records that a narrowing written at the end of a build is a sentence
written under the pressure of what got built. The residues above were known from
the sweep, not discovered by failing to catch them, and writing them down first
means the fixtures are built to a claim rather than the claim fitted to the
fixtures. This ordering is the correction RFC 0084's postscript asks for.

**What was checked and left alone.** Deleting the `xtask/` exemption was
proposed and refused: `xtask` is compiled, so the dep-info net covers every route
inside it with no exemption at all, while deleting the textual exemption turns
the shipped-tree fixture red on the checker's own fixture strings. And
`#[cfg_attr(all(), path = "…")]` on one line is documented in
`xtask/src/main.rs` as a gap and is **not** one — the byte before `path` is a
space, so the key test passes. That doc sentence is wrong in the safe direction,
which is still wrong, and it is corrected with a fixture beside it.

## Consequences

**What this makes easy.** A route stops being a pattern to recognise. The day
somebody invents a thirtieth spelling of `#[path]`, no lint changes, because the
lint is not reading the spelling.

**What it makes hard, deliberately.** A build script in the permissive tree. One
`BUILD_SCRIPT_ALLOW` row reopens the entire `OUT_DIR`-laundering and static-link
family, which is why that allow-list is named in *What would reverse this*
rather than described in a comment.

**What it costs.** `cargo xtask test` grows by roughly twenty to forty seconds —
a two-crate fixture compiles in 0.23 s in the container and `cargo metadata` runs
in about 1 s. That is defensible for one of three non-negotiable policies and
indefensible if a reviewer discovers it rather than reading it here.

**Two ways it goes red on something legitimate, both closed in the design rather
than left to be discovered.** The imported crate compiles its own sources into
the same target tree, so the dep-info net must attribute each `.d` to a package
and check only permissive ones, or `lint` goes red on the import task's first
build and the repair under time pressure is to weaken the net. And the fixtures
write `.d` files full of imported prerequisites, so the walk must skip
`target/licence-fixtures/` — the `.claude` worktree scar, one directory over.

**A second build directory per proof crate, which the first draft did not
have.** `lint_all` runs `lint_proofs` before this check, and that command
`cargo check`s `kernel/proofs`, `ring/proofs` and `abi/proofs` from their own
directories with no `--target-dir`. All three are in the root manifest's
`exclude` and are therefore their own workspace roots, so their dep-info lands
in `<crate>/target/` — sixty-three files that a net walking only `<root>/target`
never opened, in the three crates in this tree that already reach outside their
own directories by `#[path]` out of habit. Net one cannot recover them either:
`cargo metadata --no-deps` lists members, and these are excluded precisely so
they are not members. `boundary_roots` now yields one `(tree, build directory)`
pair per compiled tree, each carrying its own base because a dep-info's relative
prerequisites are relative to the workspace root of the command that wrote it.

**One way it goes green while broken that no fixture can close.** A stale target
directory: the second net reads the dep-info cargo last wrote, so a route added
to a crate cargo considers fresh produces no new `.d` and no finding. This tree
already carries that scar from the other side — *docker cargo misses host edits*
— and the honest mitigation is that `lint_boundary` runs immediately after
`lint_style` has just compiled, and refuses when the target directory carries no
`.d` at all rather than printing ok over an empty walk.

**What it forecloses.** Closing `E3-B03a` with an unqualified universal. Nobody
can now write *every route* about this boundary without the clause, and a reader
who wants the headline will read it as an omission. It is meant to.

## What would reverse this

**An allow-list entry, and this is the reversal most likely to be taken.** The
first `BUILD_SCRIPT_ALLOW` or `SYMLINK_ALLOW` row reopens a family this entry
closes structurally, and the check's strength drops to whatever the textual net
sees. A row added without an argument here is this RFC failing quietly, which is
the failure mode it is least able to detect about itself.

**The residues being closed, which would restore the word *every*.** Residue 1
goes when `lint` compiles every configuration the tree ships — most cheaply by
having the boundary net read the dep-info `cargo xtask test` produces for the
AArch64 world as well as the host's. Residue 2 goes when a proc macro in the
permissive tree is prohibited the way build scripts now are, which costs nothing
today because there are none. Residue 3 is not this exit's and would move to
`deny.toml`. If all three land, this sentence is superseded and the unqualified
one comes back — and that is a real prospect rather than a courtesy, because two
of the three are prohibitions over sets that are empty right now.

**A fourth round defeating the mechanism rather than a pattern.** If a reviewer
breaks this check, the thing to read is *which net* they went through. A break in
the dep-info net means rustc's answer is not the answer, which would be the most
interesting result in this file and would reverse the whole first half. A break
in the residue sentence means the residues are wrong, which is a repair to this
entry. A break by a thirtieth `#[path]` spelling means the mechanism change did
not happen and somebody kept adding patterns.

**And the null result worth recording.** If a year passes with no allow-list row
and no route found, the three prohibitions are carrying the boundary and the
textual net is dead weight — at which point deleting it is the honest move
rather than maintaining two mechanisms for one rule.
