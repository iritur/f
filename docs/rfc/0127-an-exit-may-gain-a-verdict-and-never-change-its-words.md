# RFC 0127: An exit may gain a verdict, and never change its words

- Status: accepted
- Date: 2026-09-25
- Affects: `xtask/src/main.rs` (`lint-exits`, `lint_all`, and two rows of
  `NOT_THE_FRAME`), `.github/workflows/ci.yml`, `intent/0012-the-interface/spec.md`
  (`E3-B07h` added), `TODO.md` (eleven E3 exits and `E1-B15`)

## Decision

`cargo xtask lint-exits` runs in `lint_all` and in the gate's `policy` job, and
holds `TODO.md` to three rules:

1. **Every task that closes has exactly one `*exit:*` line.** The standing rules
   under *Always* carry a `*cadence:*` instead and are exempt.
2. **In an epoch whose ticked `E<n>-00` names an `intent/NNNN-name/` directory,
   that directory's `spec.md` is the decomposition**, and every task line in it
   is in `TODO.md`, and every *child* line in that epoch of `TODO.md` — an id
   with something after its letter and number, `E3-B07h` and not `E3-B07` — is
   in the spec. Coarse lines are the epoch's own and are not compared.
3. **Each such tracker exit carries the spec's exit sentence, word for word.**
   A verdict may go in front of it, in bold and nothing else; evidence or a date
   may follow it. The words between may not change.

A narrowing is legal, and it is an edit to the spec with the RFC that makes it —
which is where RFC 0084 already put it. This makes the spec the only place a
narrowing *can* land.

## Context

RFC 0084 said a narrowed exit is a reversal and lands in the spec. Both files
were edited by hand and by a coordinator's record script, and nothing compared
them. On 2026-09-25 an audit found three E3 lines whose exits had drifted — one
with no exit line at all for two days, two with the sentence replaced by a
verdict — all by a script pasting a proposed block over a task; and `E3-B04f`
and `E3-B04g` in the tracker and not in the spec. `CLAUDE.md` carries *a task
with no exit is a wish*; nothing held it.

The first run of the check found twelve more:

- **Eleven E3 exits reworded as they were marked met** — `E3-B01j`, `E3-B03c`,
  `E3-B05b`, `E3-B05c`, `E3-B05f`, `E3-B06e`, `E3-B06f`, `E3-B06h`, `E3-B07c`,
  `E3-B07d`, `E3-B07g`. No RFC narrows any of them: RFC 0109 says in its own
  opening that it narrows nothing, and RFC 0111, which names `E3-B05b` and
  `E3-B05c` and lists the spec among what it affects, never edited either line
  and reads its result as meeting `E3-B05c`'s sentence rather than replacing it.
  So each is a paraphrase and the tracker is the side that is wrong. Two dropped
  a clause worth checking before restoring the words under a `**met.**`, and both
  hold: `E3-B07c`'s *through `Env`* (the component has none; the frame reads its
  own seeded one and writes the tick into the routing page) and `E3-B06h`'s *no
  wildcard arm* (twenty-two arms and two named refusals).
- **`E3-B07h`**, filed in the tracker on 2026-09-21 out of `E3-B07a`'s fourth
  review and never added to the spec. The spec is the side that is stale; the
  line is added with the exit it was filed with, from `80eb15c`.
- **`E1-B15`, with two exit lines** — a 2026-09-09 verdict and a 2026-09-11 one —
  neither of which still carries the sentence the line was filed with. E1 has no
  decomposition spec, so only rule 1 reaches it.

### What is normalised, and why each is safe

Every normalisation is a class of edit the check cannot see.

- **Whitespace, including a wrapped line**, collapses to one space. Markdown
  renders them alike, so no reader of either file can see what this erases.
- **A wrapped exit is joined** before comparison: an indented line that opens no
  paragraph of its own — no `*field:*`, no bold lead, no list item, no fence —
  continues the exit above it. Without the join a wrapped spec exit would be
  compared on its first line, and a tracker that dropped the rest would pass.
- **The sentence's final full stop is dropped**, because the tracker's shape for
  evidence is *sentence — evidence*. A word-boundary check stands in its place,
  so `seed` does not match `seedless`.

Not normalised: **emphasis**, **case**, **dashes**, **quotes**. Stripping `*`
would also strip it from `*const T` inside a code span, and an edit that only
moves emphasis costs one paste to repair, where an erased character is
permanent.

### Starts with, not contains

The brief the check was written to said *contains*, and *contains* passes `no
longer required: <the sentence>`. So what may precede the words is bold spans
and nothing else.

## Consequences

A coordinator's record script can no longer paste a verdict over an exit: the
merge is red and names the line, the spec line, and the first word that
differs. Marking a line met costs one more habit — the verdict goes in front of
the words, and the evidence after them — and that is the whole cost.

**What it cannot see.** What follows the sentence is free text, so a narrowing
written as evidence after the words — *…on both architectures — on x86-64* —
passes. Titles, sizes and `needs:` are not compared; the exit is the line's
contract and the rest is how it is scheduled. And an epoch decomposed without a
spec is held to rule 1 only; a ticked `E<n>-00` that names no spec is a finding
for that reason, since it would make the epoch's exits compared against nothing.

## The residue from `lint-datapath`, which rides here

`NOT_THE_FRAME`'s seventh row read `Event::decode(` and
`<f_abi::input::Event>::decode(` walked past it, the alias shape RFC 0126 closed
for `policy`. It is closed the same way: an eighth row whose needle is the
type's name, `Event`, with `abi/src/input.rs` as the file that must define it.
The seventh row stays, because its third field is a different property — that
the compositor decodes. A ninth row, `inbound`, closes the route neither
spelling can see: the frame links `f-compositor`, `pub mod inbound` is compiled
everywhere, and `f_compositor::inbound::Inbound::default().take(entry, payload)`
decodes an input entry inside the frame while naming neither `Event` nor
`decode`. Still unseen: a crate the frame links re-exporting the type under
another name, a function pointer to the decoder handed across, and the frame
parsing the bytes itself.

## What would reverse this

- **An emphasis-only or case-only finding that a reviewer judges was the right
  edit, twice.** Then the rule costs more than it holds and `exit_words` gains
  that normalisation, with its reason beside it.
- **A narrowing found after the words, as evidence.** That is the gap above
  happening rather than being possible, and the answer is structure: evidence
  moves out of the exit line into its own paragraph, and the exit line becomes
  exactly *verdict + words*, compared for equality rather than as a prefix.
- **A second way of saying which spec decomposes an epoch** — an `epoch:` field
  in an intent's front matter, say. Then the `E<n>-00` line stops being the only
  statement of the pairing, and this check reads the field and requires the two
  to agree rather than trusting either.
