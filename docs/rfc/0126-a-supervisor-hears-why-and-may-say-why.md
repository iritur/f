# RFC 0126: A supervisor hears why a place emptied, and may say why it emptied one

- Status: accepted
- Date: 2026-09-24
- Affects: `abi/` (`control.rs` — `op::STOP` gains a named cause in `ext[0]`,
  and `cause` gains `named_above`, which admits `TIMEDOUT` alone),
  `kernel/src/component.rs` (a peer-gone posted to a supervisor carries the
  cause in `ext`; a stop may carry a named cause onto the notice; a supervisor's
  board row carries the occupant's liveness, whether the place is occupied, and
  the place's own restart policy; the frame requires the cause the supervisor
  heard to be the cause it posted), `kernel/src/main.rs` (the compositor stage's
  reading is kept and handed to the lifecycle), `user/supervisor/`
  (`routing.rs` — three new blocks and one new row word; `policy.rs` —
  `answer`, the whole of what a run does about one row; `component.rs` — the
  drain tells *no death* from *a death with no cause*, and `declared()` is
  gone; `assemble.rs` — a row a death arrived for is withheld from the
  assembler), `xtask/src/main.rs` (`NOT_THE_FRAME` gains `policy`; `cargo xtask
  compositor` holds each half's liveness), RFC 0008, RFC 0076, RFC 0094, RFC
  0123, and `TODO.md` task `E3-B05e`.

## Decision

**The frame puts the cause of a death on the notice that tells a supervisor of
it, and a supervisor may put a cause of its own on the stop that ends an
occupant — but only a cause that is a judgement.** Three parts:

- **A peer-gone notice carries its cause.** `abi/src/control.rs` has said so
  since RFC 0008. The frame did not: a capability slot's pending state is three
  bits that say *that* a peer went, the cause lived only in `tear_down`'s
  argument, and every notice a supervisor drained carried zero. The cause is now
  held on the place (`Place::gone`) and copied onto the notice where the frame
  posts it to a supervisor's ring. A supervisor writes back the cause it heard,
  and the frame requires that word to be the word it posted — the frame's own
  word coming back, which is a transport check and not a judgement.
- **`op::STOP` may name a cause, and only `TIMEDOUT`.** `ext[0]` is zero or a
  packed cause the submitter names; the frame carries it onto the notice in
  place of `STOPPED` and reads nothing else of it. `cause::named_above` admits
  the one cause that is a supervisor's reading of what an occupant published;
  `FAULT`, `EXIT`, `STOPPED` and `RETIRED` are the frame's observations, and a
  submitter naming one is refused `ARGUMENT/RESERVED_NOT_ZERO` because it would
  be telling a place's clients a fact nobody observed.
- **A supervisor's row carries what a synchronising occupant published about
  itself, copied.** Waits outstanding, frames abandoned, and the supervisor's own
  memory of the second, in a block of their own (`at::LIVE`); whether the place
  has an occupant (`at::ROW_OCCUPANT`); and the place's own restart policy
  (`at::POLICY`). The frame compares none of them. `cargo xtask lint-datapath`
  refuses `kernel/` naming `policy` in code at all — the module's name rather
  than a path into it, because the path was beaten by an alias the first time it
  was attacked.

## Context

`E3-B05e`'s third clause was *a boot shows a compositor restarted for a
timeout*, and RFC 0123 named what was missing: the two words, copied onto a
supervisor's row, and a boot that drives them. Writing that boot found the
larger defect first.

**Every restart this tree had ever shown was decided by the wrong branch.**
`user/supervisor`'s drain stored a notice's `ext` as the cause and read zero as
*nothing died here*; the frame posted zero on every death. So the store's fault
arrived as *nothing*, the supervisor saw an untouched tally, took the branch for
*a place the frame built and never filled*, and submitted a spawn. The boot was
green, the log said `restart 0 of 3`, and `policy::decide` — whose inputs RFC
0123 repaired on the ground that a policy had been fed a constant — had never
been called on a machine at all. The mutation RFC 0123 recorded as proving *the
supervisor reads the fate off the wire* replaced that zero with `STOPPED`, which
reached `decide` for the first time and was refused; it proved the drain read
`ext`, not that anything was ever in it.

That is why this RFC carries a cause across before it carries a reading: a
timeout that travels as a named cause on a notice is worth nothing on a wire
that delivers no causes.

**The first boots of the delivery found two more, and both were restarts no
policy decided.** The first: the timeout's death was noted on the endpoint the
supervisor had been handed for the run that named the fate, and that run had
been reaped — `Table::clear_all` empties every slot — so the handle named
nothing, the refusal was discarded by a `let _`, the supervisor was `told of 0
death(s)`, and it refilled the place by the branch for a place never filled.
The frame's requirement that the heard cause be the posted one is what went red
(`the frame posted cause 0x0000000100000005 (timed out) and the supervisor heard
0x0000000000000000`). The endpoint is now minted between the run and the death,
as the store's restart already did, and the note is refused loudly.

The second is the one a reader would not have looked for. With the death
delivered, the log read `the supervisor heard timed out and said leave; restart
0 of 8` over a place that had been refilled: the compositor is a member of the
generation's topology, so RFC 0094's assembler — which answers *what does this
generation say to start* — found its row empty and started it, and the policy
was shown a row already taken. A place under `restart = "never"` would have been
refilled the same way, and a fault would have been restarted without spending
the budget. The store never showed it, and not because it is outside the
topology — `user/generation.toml` lists it — but because its routes come from
`virtio-blk` and `virtio-net`, which the assembler's empty bus leaves
`NoDevice`, and `f_assembler::start` never starts a member whose source did not
start. The compositor has no route, so it was the first member the assembler
could start into a row a death had just emptied. A row a death arrived for is now withheld from the assembler: that is
`policy::decide`'s question, and the supervisor's own module head already said
the two were different questions.

Three alternatives were live for the stop.

*The supervisor decides the restart on the run that names the fate.* One run,
no second consultation. Rejected because it is the defect above in a new place:
a verdict taken on a word the wire never carried. The fate is named on one run,
travels as a notice, and the restart is decided on the next run from what the
notice said.

*The fate travels on the board, as a retirement's verdict does.* A retirement
has no opcode behind it and a stop does, with authority already checked
(`REVOKE` on the endpoint) and a deadline already defined. The board would be a
second route to ending an occupant.

*`TIMEDOUT` as a frame-observed cause, computed when a stop's deadline passes
on an occupant whose readings look stuck.* That is RFC 0123's reversal
condition verbatim.

## Consequences

**What it makes easy.** A supervisor can now be told why, which is the
precondition for every restart policy in this tree meaning anything on a
machine; the store's restart now spends its budget (`restart 1 of 3`), and a
death whose notice carries no cause is decided on cause zero, which no policy
restarts after — a red boot rather than a plausible one. The same words that
reach `fate` are printed on both sides of the boundary and held against the
occupant's own tree outside the frame.

**The narrowing, said plainly.** No compositor in this build is served from its
place: `kernel/src/compositor.rs` stands one up outside any place, which is
`CHAOS_GAP`'s shape, and reads its tree after it ends. The reading carried onto
the compositor's row is therefore **the manifest's** — the same component file,
bound by label — and not the current occupant's, which has never run. What the
supervisor ends and refills is that occupant. The log says where the words came
from (`from the tree its component published earlier in this boot`), and
`component::Reading` says it at the type. The instance that timed out and the
instance restarted for it are the same manifest and not the same instance.

**Why a supervisor says why, and does not only hear it.** The title's second
half is not a second feature beside the line. The clause is *a compositor
restarted for a timeout*, and a restart decided on the notice's word can only be
*for* a timeout if the timeout is the word on the notice. A stop that named
nothing arrives as `STOPPED`, which no policy restarts after — correctly, since a
stop is the supervisor's own decision — so without a named cause the boot could
show a compositor stopped and never one restarted for a timeout, and the only
way to show one would be the one-run shortcut rejected above.

**The controls, and which one carries weight.** `cargo xtask compositor` holds
three answers across five halves: `serve` restarted, `starved` and `wake` left
alone, `mute` and `floorless` carrying nothing. `wake` is the control that
matters — a real reading, a frame given up, no wait outstanding — and it is what
refuses a policy that forgot the outstanding half. `starved` is weaker than it
looks: its component refuses before it serves and writes none of its nodes, so
the two words it carries are the manifest's declared zeroes. It holds a policy
that fires on every reading and not one that fires on a *written* zero, and no
half in this build writes one.

**What it costs.** The board grows three blocks and one row word, one of them
above the component's half because the gap below it was thirty-two bytes; the
layout is now held by compile-time assertions rather than by the paragraphs
beside it. `op::STOP` has a field it did not have, and a submitter that set
`ext[0]` for any other reason is now refused where it was ignored.

**What it pays on the way.** `user/supervisor`'s `declared()` returned the
store's four policy numbers for every row and said it was right by coincidence
until two supervised places declared different policies. The compositor declares
eight restarts in sixty thousand ticks, so its reversal fell due and was paid:
the row carries the place's own manifest.

## What would reverse this

**The compositor served from its place.** The day `kernel/src/compositor.rs`
drives a compositor that `component::demonstrate` placed — a `Datapath` for it,
or `E3-B01g`'s client moving out of the frame — the frame copies the two words
out of the occupant's own mounted tree, `component::Reading` goes, and the
narrowing above stops being true. Watch for the carry surviving that day: a
reading taken off one instance and put on another's row when the first is no
longer necessary.

**A second judgement.** A cause that is a supervisor's reading of something an
occupant published, other than a timeout. It is an arm in `cause::named_above`
and a paragraph here, never a flag a submitter sets.

**A slot that can hold a cause.** If `kernel/src/cap.rs`'s pending state grows
room for the cause — or a notice ring grows a side table — `Place::gone` and the
`told` list `post_only` takes are a second copy of it and go.

**An assembler that starts members on a running machine.** A generation swap
that brings a new member up beside live ones has to tell *first start* from
*refill* for itself; the withheld list is the smallest form of that and would
become a field the frame writes — *this place has had an occupant* — rather than
a list the supervisor builds from its own drain.

**Anything in `kernel/` comparing the liveness words**, including against the
supervisor's account of them. RFC 0123's reversal, unchanged: the comparison
lives in `cargo xtask compositor` because it may not live in the frame.
