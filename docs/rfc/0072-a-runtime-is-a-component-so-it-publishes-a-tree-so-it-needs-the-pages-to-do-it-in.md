# RFC 0072: A runtime is a component, so it publishes a tree, so it needs the pages to do it in

- Status: accepted
- Date: 2026-09-11
- Affects: `kernel/src/process.rs` (`INIT_TEXT_PAGES`, `OWN_TREE`, `self_test`),
  `kernel/src/runtime.rs`, `abi/src/state.rs` (a `Writer` beside the `Reader`),
  `user/store/src/report.rs` and `user/store/src/runtime.rs`,
  `user/init/src/component.rs`, `xtask/src/main.rs` (`INIT_MAX`), `TODO.md`
  (`E1-B15`)
- Implemented by `E1-B15`, in the commit this entry landed with

## Decision

Three things, and the second is the one that was not obvious.

**A runtime gets a page to publish its own state tree in.**
`kernel::process::OWN_TREE`, mapped by `prepare_runtime` as a fifth part, with
the frame writing the manifest's schema into it before the component's first
instruction — `component::publish_tree`, the same function the spawn path uses.
RFC 0013 says every component publishes a state tree; RFC 0065 made the schema
part of what a manifest declares; the shape that actually *runs* a runtime had
nowhere to put it, so `user/store` packed its tallies into a word and carried
them out through `door::EXIT`.

**The init and runtime shapes get four pages of text instead of one.** This is
the part `E1-B15`'s own line called "one mapping away" and was wrong about.
`user/store`'s image is 3904 bytes against a 4096-byte reservation — **192
bytes of room** — and the write path costs about 536. There is no version of
this change that fits in the shape as it was.

**The write side lives in `abi`.** `f_abi::state::Writer`, beside the `Reader`
that was already there. A component may not write `unsafe` and a mapped page
can only be reached through a raw pointer, so without this the policy in
`CLAUDE.md` and the requirement in RFC 0013 could not both hold: every
component publishes a tree, and no component is allowed to write into one.

## The ceiling, stated because the next change will spend from it

Everything from `TEXT` to `OWN_TREE` has to stay below `SPAWN_GUARD`, which is
`TEXT` plus `TEXT_PAGES` — sixteen pages. The chain is the text reservation plus
nine: guard, stack, the page above the stack, two grant pages, the frame's tree,
the control ring, the work ring, and now the component's own tree.

So `INIT_TEXT_PAGES` may be at most **seven**. It is four, which leaves three
pages spare. The two assertions that say so are in `process.rs` and fail the
build rather than a boot.

`xtask`'s `INIT_MAX` is the same number written twice, which is what its own
refusal says when an image outgrows it: *widening it means widening
`kernel::process`'s own reservation in the same diff*.

## What the frame checks now that it could not before

`user/store`'s `CONTROL_AT` and `WORK_AT` were private constants inside a module
gated on `feature = "image"`, which the kernel does not enable. So the frame
could not see them, could not compare them against its own layout, and a
disagreement was a page fault at the component's first adoption — reported as an
ordinary ring-3 fault with nothing in it naming the cause.

They are in `f_store::report` now, which is the one module of that crate the
frame links, and `kernel/src/runtime.rs` carries three compile-time assertions
against `process::RING`, `process::WORK` and `process::OWN_TREE`. That is the
arrangement the three driver crates have had since RFC 0047, where
`kernel/src/blk.rs` asserts `process::BLK_BOARD == routing::AT` and a half-done
move does not link. This shape did not have it, and the widening in this change
would have been exactly the sort of edit that needed it.

## How a boot says whether the component published

Two readings of the same page: one taken before the component's first
instruction, one after the run and before `reap` gives the page back. The pair
is the evidence and neither half is evidence alone — a snapshot that moved says
the component stored something, and only the reading taken before it ran says
the words were not already there.

The boot prints both, whether they moved, and how many of the declared nodes
carry a non-zero word. A **count** and not a named value, because the frame
reads a manifest for shape and not for meaning: which id means `work` is the
component's to declare, and the first node is the subtree the others hang under
and never carries a word at all.

On `cargo xtask runtime` the load, provoke and reclaim halves each move the
snapshot with two nodes written; the hostile half leaves it unmoved with none,
because a scribbled control-ring header is refused before the component does any
work. That is the reading the design wants: *unmoved* is a real answer and not a
missing one.

## What would reverse this

- **A component whose image does not fit four pages.** The number moves, up to
  seven, and past seven the region itself has to move — at which point
  `SPAWN_GUARD`, `TEXT_PAGES` and the two assertions are one question rather
  than three constants.
- **A loader that reads a component's headers.** E5. Both `INIT_TEXT_PAGES` and
  `TEXT_PAGES` stop existing, because a reservation is only needed by a frame
  that copies a flat image to a fixed address and jumps to its first byte.
- **`door::Entry` growing a field for the address.** Then a component is *told*
  where its world is rather than holding it as a constant, and the three
  assertions this entry added become unnecessary along with the constants they
  compare. RFC 0008 is where that was first written down as the thing that would
  make this arrangement go away.
- **A runtime that outlives the page.** The status still travels through the
  door because the frame reads it *after* the address space is gone, and a tree
  cannot answer once the memory it lived in has been given back. If a runtime
  ever publishes somewhere the frame keeps, `f_store::report` collapses to
  nothing and this paragraph is why it did not already.
