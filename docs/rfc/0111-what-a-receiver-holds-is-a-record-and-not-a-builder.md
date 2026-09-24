# RFC 0111: What a receiver holds is a record and not a builder

- Status: accepted
- Date: 2026-09-24
- Affects: `abi/src/sync.rs` (`Submission::decode`, the new `Arrived`), the new
  `abi/src/trace.rs`, `TODO.md` `E3-B05b` and `E3-B05c`, `intent/0012-the-interface/spec.md`'s
  same two lines

## Decision

`Submission::decode` returns `Arrived` and no longer returns `Submission`.
`Arrived` carries the same record — the same waits, the same signal, the same
bytes — and carries **no constructor that adds a wait to it**: no `waiting`, no
`signalling`, and no conversion back to the builder in either direction. A driver
is a receiver, so the value a driver holds is now a record.

`E3-B05a` closed on the claim that an unsatisfiable wait is *refused where a
submission is built, not detected where it is waited on*, and that claim was and
is true: `Wait` and `Signal` have private fields in a private module with one
constructor each, reached only through the `Timeline` that would satisfy them,
and `Submission::waiting` takes a `Wait` rather than two numbers. So the first
half of `E3-B05c` — *a constructor that takes its waits explicitly* — was already
discharged, and this entry does not re-do it.

The second half was not discharged, and what was missing was not a check. It was
that **the decoded value was still a builder.** `Submission::waiting` is public;
`Submission` is `Copy`; so a receiver holding a decoded submission held, with it,
the ability to put another wait into the client's record and submit that. Nothing
refused it because nothing was wrong with it: every function involved was being
used for what it is. That is implicit synchronisation exactly — a wait inside the
submission the client wrote, with the client's own bytes still around it — and it
was reachable through the public API with no `unsafe`, no cast and no second
copy of any rule.

## Context

`E3-B05`'s exit is a negative: *no implicit wait appears in a frame trace.* A
negative needs two things, and the epoch's spec splits them onto two lines
deliberately. `E3-B05b` is the trace, so that the negative can be *observed*.
`E3-B05c` is the static half, so that the negative still holds against a driver
whose trace this tree cannot read — an imported one, behind the licence boundary
of RFC 0003, which this tree links over a ring and does not compile.

Three repairs were live for the second half.

**A check at the build site.** `Submission::waiting` could refuse a wait added
after a decode, given a flag saying the submission arrived. Declined: it is a
line somebody can stop calling, it needs a moment to run at, and the module's own
header already argues at length that this is the weaker form. The file's standing
claim is *a check is a line somebody can stop calling* — adding one here would
have been that sentence contradicted in its own file.

**A declaration in the record: `Depends::OnWhatItNames` against
`Depends::OnNothing`, a byte at offset 2, zero meaning nothing declared and
therefore refused.** This was attractive and is worth recording as refused,
because the reason is a rule this tree already holds. The declaration is
*derivable from the wait count*: a submission with zero waits depends on nothing,
because `wait_count` is counted from the array and there is no field a receiver
could read as permission to add one. `Submission`'s own documentation declines to
store a count beside the array for exactly that reason — *a second statement of a
derivable fact is a thing two writers disagree about while both pass their own
tests* — and a byte carrying a fact the record already carries would have been
that mistake committed in the file that names it. It would also have spent a
reserved byte and an ABI change to buy nothing.

**Splitting the type.** Taken. It costs one wrapper, one `pub(super)` sealing
constructor, four forwarding methods and one asymmetric `PartialEq`, and it has
no moment at which it runs.

The trace is the other half of the same wave and the two are tied at one point
worth naming: a trace entry is built from a `Wait`, so an implicit wait cannot
appear in a trace either — a receiver that invented a dependency would have to
invent a `Wait` first, and `sync`'s `proof` module is what makes that
unavailable. The static half and the observable half rest on one property.

## Consequences

**What this makes easy.** Handing a decoded submission to something that must not
extend it — a driver, a supervisor persisting it for `E3-B05f`, a recorder. There
is no way to get it wrong, so there is nothing to review. `Arrived::encode`
forwards the bytes unchanged, so forwarding a record does not route it back
through a builder, and `what arrived re-encodes to what arrived` is a test rather
than a convention.

**What this makes hard, and it is intended.** A receiver that genuinely needs one
more wait has to build a new submission, from admitted waits, which is a new
record with its own bytes and its own entry in a frame trace. That is more typing
and it is the point: the new record is *visible*, which is what the parent task's
negative asks for. Nothing was added to make it convenient, because a convenience
here is the door reopened.

**What it forecloses.** `From<Arrived> for Submission` and any inverse of
`Arrived::sealed`. Either would hand a receiver the builder back, and a door with
one caller is still a door. The comparison a caller actually wants — *the bytes
decoded to the submission I built* — is available as `PartialEq<Submission> for
Arrived`, asymmetric on purpose so that it is a comparison and not a conversion.

**What it does not buy, and this is the residue.** Nothing here reaches an
imported driver's internals. A driver behind the licence boundary can program the
hardware as it likes and this tree cannot read its trace; `E3-B05c`'s exit says
as much and calls this the half that still holds against such a driver. What
holds is narrower and is the most that can hold: the *record* it was handed says
what the client asked for and nothing else, and a wait the driver added is not
expressible as the client having asked for it. The wider claim — that no wait
exists that the client did not ask for — is not made and cannot be made from
inside this tree.

**Two smaller reversals ride with it.** `Chain::compositor_waits_and_signals`,
`Chain::present_waits` and `Chain::landed` each take a `&mut Trace`, so there is
no path through the chain's own doors that enters a wait and does not record it.
`present_waits` keeps `&self`, which matters: the file argues that the chain's
third stage takes `&self` *because a stage that signals nothing changes no
timeline*, and had the trace been a field of `Chain` that function would have
become `&mut self` and the argument would have been withdrawn for a reason that
has nothing to do with what a present engine is. The trace is an argument, so the
sentence survives the recording. And `Refusal` gains an eleventh variant,
`NoStage`, which belongs to `trace.rs` and is declared in `sync.rs` because a
trace entry *is* a wait and two lists of reasons a wait is not believed would be
two answers to one question; it cost one declaration and no arm, no row and no
line in a test, which is the evidence the `refusals!` macro was owed.

`abi` also gains its first dependency row of any kind, `f-env` under
`[dev-dependencies]`, because *byte-identical for one seed* means a seed and a
seed in this tree means `f_env::SeededEnv`. `ring/Cargo.toml` states the same
arrangement for the same reason; nothing under `[dependencies]` changed and no
image links it.

## What would reverse this

**A second door to a `Wait`.** Everything above rests on `Timeline::wait` being
the only one. The day a second exists, the recorder belongs at the admission
rather than at the chain, and `Arrived` stops being the boundary that matters
because a receiver can admit its own waits without going anywhere near a decoded
record. That is the observation to watch, and it will not look like a reversal: it
will look like a convenience constructor.

**A receiver that must extend a submission on a hot path.** If the cost of
building a new record is ever *measured* to matter — not supposed, measured, on
`E3-B05d`'s named machine — then the honest change is a builder that consumes an
`Arrived` and states in its own name that it is making a new record, not a
`waiting` method on `Arrived`. The distinction is the whole of this entry.

**A driver whose trace this tree can read.** If an imported driver ever exports a
submission log this tree can decode, the residue above becomes testable: the
negative stops being a static claim about a record and becomes an observation
about a driver. `E3-B05` would then be closable on evidence rather than on
construction, and this entry's *what it does not buy* paragraph is the one to
delete.
