# RFC 0131: The latch's patch leaves with its frame

- Status: accepted
- Date: 2026-09-25
- Affects: `user/compositor/src/latch.rs` (`LateLatch::restore`, the
  `owed` transform and the `restores` count; the module's *the patch is the
  submitted frame's*), `user/compositor/src/tree.rs` (`Held::close` restores
  after `Waits::frame` returns), and `TODO.md` task `E3-B01i`, whose exit this
  does not change and whose mechanism it corrects.

## Decision

**The late latch writes its patch into the retained graph for the length of one
submission, and writes the client's committed transform back once the chain
driver returns.** The patch is the submitted frame's and never the graph's:
between frames the retained graph holds exactly what the client's deltas made
it, and the next frame's latch reads *committed* out of a graph nothing but a
client has written. The restore goes through the same door as the patch —
`f_scene::arena::Arena::set_transform`, one node and six numbers — and is the
client's own record, taken whole at the latch, not a transform re-derived.

## Context

Until this RFC nothing took the patch back out. `LateLatch::patch` read the
transform it called *committed* out of the graph, patched the graph, and left
it; on the next frame without a new client `SetTransform` on that node,
*committed* was the compositor's own previous latch. The comment above that
read says why that is the one comparison it may not make — *a compositor
comparing its own memory of the commit against its own patch would agree with
itself however wrong the graph was* — and the code made it on every frame after
the first. An adversarial audit found it by reading; every test closed one
frame, and the one host test that latched four times over one graph never
asked what it read as committed. The boot closes one frame.

Two placements were live.

1. **In the graph, restored after submit** — this decision.
2. **Only in the outgoing frame, never in the graph.** Needs an outgoing frame
   to put the patch in, and this component has none: nothing draws, the graph
   is what a submission would be read from, and there is no heap for a second
   arena — `routing::HEAP_BYTES` leaves 6 528 bytes beside the graph and the
   batch. A patch held beside the graph as an overlay would oblige every future
   reader of *the submitted frame* to remember to apply it, and would leave the
   boot's *and by nothing else* — `latch::unmoved`, the fold of the graph either
   side of the patch — with nothing patched to fold.

**Why this answers `E3-B06f` rather than only `E3-B01i`.** That line's property
is that the compositor cannot tell an authored delta from a projected one: the
retained graph is a function of what clients sent and of nothing else. A graph
the latch had written into and left was neither authored nor projected — a
third source, and one no client could see or undo. Restored after submit, the
retained graph between frames is the client's again.

The restore is **after the submission and not before the next latch**. A
restore deferred to the next frame's latch passes the *committed* check and
fails the other half: the graph between frames still holds a transform no
client sent, and a client `SetTransform` arriving in the meantime would be
overwritten by the compositor's memory of the previous commit.

## Consequences

**Easy.** `a_second_frame_is_latched_against_the_clients_transform_and_not_its_own`
closes two frames, the second with no client transform, and requires the
second latch's *committed* to be the client's and the graph between and after
to be the client's. Deleting the restore reddens it on the graph clause and
`the_components_own_window_latches_the_frame_it_closes` on `restores`; moving
the restore to the start of the next latch passes the *committed* clause and
reddens the between-frames one. `LateLatch::restores` beside `latches` says,
on any run, whether every patch left.

**Hard.** Nothing on the board publishes `restores`, so a boot cannot yet see
a restore the graph refused; the host tests can. The graph refusing a node it
accepted one statement earlier is the arena contradicting itself, and it is
returned as `false` and left out of the count rather than invented a word for.

**Forecloses** a retained graph that carries anything a client did not send,
for as long as this component has no buffer of its own to submit from.

## What would reverse this

- **A renderer that encodes the submitted frame into a buffer of its own**
  (`E3-B02`). Then the patch belongs in that buffer, is never written into the
  graph at all, and the restore is deleted rather than kept as a second guard.
- **A heap with room for a second arena**, at which point the submitted frame
  can be a copy and the same move applies without a renderer.
- **A client that wants the latched position kept** — a cursor the compositor
  is asked to own rather than to borrow. That is a delta the compositor would
  author, and it needs a wire opcode saying so rather than a latch that forgets
  to give a node back.
