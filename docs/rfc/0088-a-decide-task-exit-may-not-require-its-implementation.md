# RFC 0088: A decide task's exit may not require its implementation, and `E3-B03a`'s did

- Status: accepted
- Date: 2026-09-16
- Affects: `intent/0012-the-interface/spec.md`, whose `E3-B03a` exit line is
  replaced below and which is a paste-ready handoff, so a false line there
  becomes a false line in `TODO.md`; RFC 0082, which decided *imported* and then
  had to write out that the exit its own first sentence fired could not be
  observed; `xtask/src/main.rs`'s `lint_licensing`, which carries the third
  route named below as of this revision; `docs/rfc/README.md`'s row for RFC
  0082, which names two of the three routes and which no agent working in this
  tree may edit; the root `Cargo.toml`'s `exclude`, still owed `"third_party"`,
  and now owed by a named task rather than by nobody

## Decision

**`E3-B03a`'s exit is narrowed, and the narrowing is that a decide task is
accepted on its decision.** The line said:

> an RFC, accepted, naming which and what would reverse it; if imported,
> `third_party/` carries the entry and `cargo xtask lint` shows the permissive
> tree reaching it over a ring and by no other route.

It now says:

> an RFC, accepted, naming which and what would reverse it, and — because it
> answers *imported* — naming the task that lands the entry and the two files
> that make the ring observable; and `cargo xtask lint` refuses every route from
> the permissive tree into `third_party/` other than a ring, with a fixture
> driving each route red.

What moved out of the exit and into the task that imports a shaper: the entry
itself, the `user/shaper/manifest.toml` and `abi/` `shape` protocol that make
*reached over a ring* an observation rather than a plan, and `"third_party"` in
the root `Cargo.toml`'s `exclude`. What stayed: the decision, its reversal
condition, and the half of *by no other route* that is a check this tree runs.

**The general half, and it is the one worth keeping.** `E3-B03a` is an `S`-sized
task whose verb is *decide*. Its exit contained a conditional clause that its own
decision fired, and the consequent of that clause was an implementation — source
under `third_party/`, a manifest, a protocol in `abi/`, a component image. No
amount of deciding produces those. So the exit was not a sentence the task could
be observed to have satisfied; it was two tasks' exits joined by *if*, and the
second task did not exist in the decomposition at all. The rule this settles:
**an exit may require the artefact the task produces and the checks that make it
observable, and may not require an artefact a later task produces.** A
conditional whose consequent belongs to another task is that other task's exit,
written early.

This is the route RFC 0084 opened, taken for the first time by somebody other
than its author, and it is worth saying that the route held: the narrowing is in
the spec with a number beside it rather than in a module comment, and this
document is what a reader following the number finds.

## Context

`E3-B03a` was refused twice. Both refusals were correct and neither was about
the decision.

**The first clause was met on day one.** RFC 0082 exists, is accepted, decides
*imported* over *written in the permissive tree*, argues the losing side at
length — four correct arguments for writing it, one of which is this epoch's own
warning about floats — and states what would reverse it. Nobody has disputed
that clause in either round.

**The second clause fired on the first clause and then asked for a build.**
`find third_party -type f` returns exactly one path, `third_party/README.md`.
There is no `user/shaper/`, no `abi/src/shape.rs`, no image and no ring. RFC 0082
says this itself, in the open, in a section written for the purpose: *"on the day
this RFC is accepted, the second clause of `E3-B03a`'s exit cannot be
observed"*, and then, flatly, **"`E3-B03a` does not close on this artifact."**
The second reviewer re-ran the observation rather than taking the prose for it —
one file under `third_party/`, no `shaper` under `user/`, no `shape.rs` under
`abi/src/` — and found the document and the tree in agreement that the clause is
unmet.

**So the honest reading is that the sentence was malformed, not that the work was
skipped.** The alternative live at the point of refusal was to close the clause
by importing a shaper: choosing an upstream, vendoring it with `LICENSE` and
`PROVENANCE.md`, writing the shim, the manifest and the `shape` protocol, and
building a component image. That is an `L`-sized task with a provenance decision
of its own inside it, and RFC 0082 already declines to name an upstream *"from
memory rather than from a build"*. Doing it under an `S` decide task's letter
would have put the import in the tree with nobody's name against the choice,
which is the failure the licence boundary exists to prevent.

**And the *by no other route* half turned out to be the part with real work in
it.** Three routes from the permissive tree into `third_party/` have now been
built and compiled by reviewers rather than imagined, in this workspace's shape:

1. `use third_party` / `third_party::` in Rust source. Refused since M0.
2. A path dependency under a name of its own — `f-shape-sys = { path =
   "../third_party/shaper" }`. It spells `third_party` in no Rust file, and
   cargo adds it to `workspace_members` besides, because the root `exclude` does
   not name `third_party`. Refused as of RFC 0082's revision, by
   `licensing_graph_findings`.
3. `#[path = "../../third_party/shaper/src/lib.rs"] mod shaper;`. **This one was
   missed twice.** It writes no manifest row, so route 2's check reads nothing,
   and no `use`, so route 1's check matches nothing. It compiles the imported
   file *into* the permissive crate — under that crate's `unsafe_code =
   "forbid"` and its `license = "Apache-2.0 OR MIT"` field, with no crate
   boundary to inspect. It is also the spelling a contributor here already
   knows: `kernel/proofs` and `ring/proofs` both compile a shipped file this
   way. Refused as of this revision, by `path_attr_findings`.

Route 2's check had a fourth gap of its own, one character wide: it read TOML's
basic strings and not its literal ones, so `path = '../third_party/shaper'`
produced nothing. That is the shape a guard takes when its bound is the list of
counterexamples somebody has thought of so far, and it is the argument for
keeping this clause in the exit rather than moving it out with the rest.

## Consequences

**What this makes easy.** `E3-B03a` can close on something a reader can check in
a minute: an RFC that decides, and a lint that goes red on each of three routes
with a fixture apiece. Nothing in that sentence depends on a shaper existing.

**What it makes hard, deliberately.** The import now has to be a task with a
letter, an owner and an exit of its own. RFC 0082 already wrote that task's exit
out in prose because `TODO.md` is a file its author could not edit; this document
gives it a second, harder anchor, because the spec line that used to carry the
obligation no longer does. If nobody lands that task, `third_party/` stays empty
and `E3-B03b` through `E3-B03k` stay blocked on nothing — which is a visible
failure rather than a silent one, and is the trade being made.

**What it forecloses.** Closing `E3-B03a` by pointing at the lint alone and
calling the boundary proved. The lint is a guard against a route into an empty
directory. It has never run against an import, because there is nothing to
import; the day the entry lands is the first day check 3 has a subject, and RFC
0082 says so about check 1 and 2 as well. A guard with no subject that has never
been wrong is not evidence, and this document is not claiming it as any.

**What it costs, said plainly.** Two things this narrowing does not fix and does
not pretend to.

- `docs/rfc/README.md`'s row for RFC 0082 names two of the three routes. It was
  refused for a worse fault than that — for still asserting the sentence RFC
  0082's body retracts — and that fault is gone: the row was rewritten one
  commit after the repair the reviewer measured, and now quotes *"and by no
  other route because there is no other route to take"* as the retracted claim,
  says a reviewer built what it called impossible, and says the exit is not
  closed and which clause. Body and index agree. What it does not carry is the
  `#[path]` route, which is a row that is incomplete rather than false and is
  owed to whoever owns that file: no agent working in this tree may edit it, and
  nothing in `cargo xtask lint` compares a registry row to the RFC it indexes,
  so this will not be caught later either.
- `"third_party"` is still absent from the root `Cargo.toml`'s `exclude`, so a
  path dependency into the import would still join the workspace if one were
  written. Until that lands, the guard against route 2 is a lint rather than
  cargo, which is what RFC 0082 means by *"the lint is the guard, and the
  structural removal is what is owed"*.

## What would reverse this

**For `E3-B03a`'s narrowing, one thing, and it is the ordinary course of
events.** The import task lands: `third_party/<name>/` carrying `LICENSE` and
`PROVENANCE.md`, `user/shaper/manifest.toml` with `image` pointing at it and a
`[[ring]]` declaring the `shape` protocol, and `abi/`'s `shape` types. On that
day every clause of the original exit is observable, the narrowed line has
bought nothing that the wider one would not have given, and this half of this
RFC is superseded rather than merely old. The reason for narrowing now is that
the day is not today and `E3-B03a` is not the task that brings it.

**For the general half, an exit that this rule refuses and that everybody wants
anyway.** The rule says a decide task may not be accepted on a later task's
artefact. The case against it is that a decision nobody implements is worth
nothing, and that tying the decision's exit to the first use of it is exactly
what stops a tree accumulating accepted RFCs nothing acts on. If this repository
grows a run of decide tasks that closed cleanly and were never carried out, the
rule is what allowed it and the answer is to put the implementing task's letter
*in* the decide task's exit as a `needs:` in the other direction — the decision
is not done until something uses it — rather than to go back to conditionals
whose consequent belongs elsewhere.

**And for the route-counting, the measurement that should worry a reader of this
document.** Three routes were found by three separate reviewers, two of them
after a repair that claimed the class was closed. A fourth would mean the
enumeration is the check, and the enumeration is not a property — at which point
the thing to build is the structural removal rather than a fourth needle:
`"third_party"` in `exclude`, and imported source that is not a crate this
workspace can resolve at all.
