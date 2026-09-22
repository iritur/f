# RFC 0102: A seventh scene opcode, and the exit it un-narrows

- Status: accepted
- Date: 2026-09-22
- Affects: `abi/src/scene.rs`, `scene/src/{arena,commit,dirty,effect,kind}.rs`,
  `kernel/src/compositor.rs`, and RFC 0084's narrowing of `E3-B07a`

## Decision

`f_abi::scene`'s opcode vocabulary widens from six to seven. `SET_EFFECT = 0x07`
carries a node, an estimate and a saving — three `u32`s, the two costs with
their scale in their names — and **the decoder refuses a declaration naming one
word and not the other.** RFC 0010 makes that vocabulary stable, so this is an
ABI change and takes an entry. Existing opcode numbers are untouched and the new
one takes the next free value.

`E3-B07a`'s pre-narrowing sentence is restored where it was always meant to
live. The narrowed form put the refusal on `Effect::declared`, a constructor,
and RFC 0084 said plainly why: *`abi/src/scene.rs` has six opcodes and none of
them carries an effect's parameters, so no delta in this workspace can carry one
word without the other and the exit's antecedent never reaches a decoder.*
**That measurement is now false**, by construction, and this is the entry that
makes it false on purpose rather than by drift.

## Context

RFC 0084 narrowed `E3-B07a` on two measurements and named this opcode under
*What would reverse this*, charged to no line. `grep -rn SET_EFFECT` returned
three hits and every one of them was the sentence itself. A reversal condition
charged to nobody reads like a plan and is not one, so `E3-B07h` was filed on
2026-09-21 and this is its decision.

**Two things the build refused along the way, both worth keeping.**

The first draft stored the declaration on the node — `Option<SetEffect>` on
`Slot` — and the build went red at `user/compositor/src/lib.rs`: *the heap this
manifest declares is smaller than the graph this component holds*. Sixteen bytes
by `NODES_MAX` is 16 KiB. On re-reading it was wrong independently of the
arithmetic: `degrade::downgrade` takes `&mut [Effect]`, so the table belongs to
whoever runs the policy, and a declaration hung on a node would have to be
gathered into exactly such a table before it was any use. **`Arena::set_effect`
therefore checks the node exists and stores nothing**, with the measurement and
the reversal in its own doc comment so `E3-B07c` starts from a number.

The second is smaller and is the better story. The record was written with three
guards — estimate zero, saving zero, saving above estimate — and **the first one
could not be killed**: mutating it to `== u32::MAX` left the suite at 20 passed,
0 failed, because a saving past the zero check is non-zero and a non-zero saving
against a zero estimate is already *above* it. The guard was deleted rather than
kept. Two conditions, both killable, both killed under test.

## Consequences

**What this makes easy.** A refusal at the boundary that exists rather than at a
constructor a peer does not have to call. The decoder is the only thing both
sides of a ring share, and an effect declared across one now cannot arrive half
formed.

**What it costs, and every item was a build error on the day the opcode was
declared rather than a reviewer's catch:** one row in `entries!`, one record, one
byte image in the per-opcode table, and one arm in each of the five consumers
that decide per opcode — `commit::section_of`, `commit::admit`, `dirty::REACH`,
`kind::Change::of` and `Arena::apply`. That is the cost `E3-B01b`'s narrowed
exit predicted for a seventh *kind*, observed for a seventh *opcode*, and it
came out the same way.

**A hazard closed on the way past.** The specimen corpus required every field of
a record to be non-zero and distinct — enforced by a comment three records
carried and four did not. It is now a test that reads each record's field list
out of the `Writer` that wrote it, so an eighth record inherits the rule the way
it inherits its width bound. Both sides are `#[cfg(test)]`; a shipped encoder
writes no span list.

**What is not closed, stated so nobody reads it as done.** Nothing refuses a
`SET_EFFECT` naming a node that is not `Kind::Effect`. That refusal is
`Effect::declared`'s, and routing a decoded record through it needs a refusal
variant `arena::Refusal` and `commit::Refusal` do not have. It was open before
this entry and it is open after; `admit`'s new arm says so where a reader meets
it.

## What would reverse this

- **`E3-B07b`'s downgrade order needing a rank per effect *shape* rather than
  per declared saving.** Then the shape is a fourth field and this is an ABI
  change again.
- **The declaration turning out to belong at the node after all** — `E3-B07c` —
  which puts a field back on `Slot` and moves `f_compositor`'s declared heap in
  the same diff. The measurement above is what that decision should start from.
- **A refusal code of RFC 0010's own for *a closed field is outside its set*.**
  `Refusal::Value` is reused here rather than a new refusal minted, following
  the module's *why the refusals are local*; a distinct code would move the
  three rows this record shares with `Undeclared::REFUSAL`, and
  `every_refusal_is_an_argument_error_a_completion_can_carry` is where that
  split would have to be made deliberately.
