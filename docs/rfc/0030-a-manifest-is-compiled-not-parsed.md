# RFC 0030: A manifest is compiled, not parsed — and the frame spawns the first supervisor

- Status: accepted
- Date: 2026-09-03
- Affects: `abi/` (`manifest.rs`, the fixed-layout record and its reader;
  `control.rs`, the four control opcodes, the seven notice kinds and the
  pending-notice state machine; `lib.rs` — `cflags::NOTICE`, the second reading
  of `Cqe::user_data` RFC 0008 owed, and `error::peer::EMPTY`),
  `kernel/src/cap.rs` (the pending-notice field in a slot, the stop word and the
  two grade words in a table), `kernel/src/component.rs` (new: places, spawn,
  connect, stop, uniform teardown, the restart policy),
  `kernel/src/arch/x86_64/multiboot.rs` (nothing — it already carries eight
  modules, and saying so is half of this RFC's second decision),
  `xtask/src/main.rs` (`cargo xtask component`, and `-initrd` as a list),
  `kernel/src/main.rs` (one line: a boot with no component file says so and
  carries on), `tools/f-on-metal.sh` and `docs/booting-on-hardware.md` (an
  optional third module),
  `docs/manifest.md` (the two things it deferred to E1-B05 by name), RFC 0008
  (which this implements and does not amend) and RFC 0005 (whose `domain` field
  the record carries); `TODO.md` tasks E1-B05, which this is, and E1-B08, which
  this hands one named deferral

## Decision

Two questions had to be answered before a supervisor could exist, and both are
the kind a future contributor re-litigates in the third month rather than the
first. They are answered here so that the answer is a document rather than a
shape somebody inferred from the code.

### One: the manifest a supervisor reads is not the manifest a person writes

**`manifest.toml` is source. `abi::manifest::Record` is the artefact. Nothing
above the compiler ever sees TOML.**

`cargo xtask component <name>` reads `user/<name>/manifest.toml` through the
same checker `cargo xtask lint-manifests` runs — one parser, not two — and emits
a **component file**: a fixed-width, `#[repr(C)]` [`Record`] followed by the
image bytes, in one blob the boot loader hands over as one module. The content
hash a spawn names is over that blob whole, so *what a component is* — its code
and its declared shape together — is one name, which is what RFC 0008 asked for
and what a measured boot can extend into.

Three consequences, each of which is the reason rather than a side effect:

- **The frame has no parser.** A kernel with no allocator parsing TOML is a
  kernel whose attack surface includes a text format; every bound in
  `docs/manifest.md` — thirty-two-byte names, sixteen capabilities, eight rings
  — exists because a record has to have a size, and this is the record they were
  sized for. Reading one is a length check, a magic check, a schema check and a
  field-by-field refusal, and there is no state machine in it.
- **The refusal moves left.** A manifest that stops fitting the schema is
  refused by `cargo xtask lint-manifests` at lint time, which is the check
  `docs/manifest.md` names as the schema-as-code and which this RFC does not
  move. What the frame refuses is a *record*: a magic that is not the magic, a
  schema it does not know, a reserved field that is not zero, a count past the
  bound, a string that is not `[a-z0-9-]`. Both refusals exist because they
  refuse different things — one refuses what a person wrote, the other refuses
  what arrived.
- **Two spawns of one hash are the same component.** There is no argument
  vector, no environment block and no configuration read at run time, so the
  hash is the whole of a component's identity. That is RFC 0008's sentence and
  this is the form that makes it true.

The alternative that was live is a TOML reader in the frame, and it loses on
the argument this tree has already made twice: `xtask` parses its own formats
and buys no dependency for one, and the frame is the place where that argument
is strongest rather than weakest. The other alternative — the supervisor parses
TOML in user space and hands the frame a checked structure — loses for a
different reason: the supervisor would then be trusted to have parsed
correctly, and the frame would be validating a structure whose provenance it
cannot check. The record is validated by whoever reads it, every time, which is
the only arrangement that survives a hostile supervisor.

**What the record carries and in what unit** is `abi/src/manifest.rs`, and every
field states one, because `cargo xtask lint-units` reads the doc comment and
R03 makes it normative. Two are worth naming here because the schema states them
in a different unit on purpose. `docs/manifest.md` writes a backoff and a budget
window in **milliseconds**, because a person chooses those; the record carries
them in **timer ticks** at the frame's own tick rate, because a supervisor
compares them against a count the frame keeps and RFC 0004 forbids it a clock.
The conversion happens once, in `xtask`, where it is a build step somebody can
read — not in the frame, where it would be arithmetic on a number whose unit had
to be inferred.

### Two: the frame spawns the first supervisor, and it does so from a boot module

RFC 0008 says the supervisor is a component and that the frame is the only
producer on a control ring, which together leave one question: who spawns the
first one. **The frame does, from a boot module, and the supervisor spawns
everything else.** There is no second mechanism and no bootstrap special case in
the spawn path: the first spawn differs from every later one only in who
submitted it, which is nobody — the frame performs it at boot from a topology
compiled into the boot modules themselves.

The multiboot half of that has a shorter answer than expected, and the answer is
worth recording because the obvious assumption is wrong. **`multiboot.rs`
already carries eight modules.** `MAX_MODULES` has been eight since M1, every
one of them is validated and reserved before the frame allocator is populated,
and a ninth is *counted as dropped* rather than ignored — `main::component`
already refuses to boot when the count is non-zero, because a module the kernel
did not keep is a module whose memory was not reserved. So carrying more than
one module costs the kernel nothing at all.

What changes is `xtask`. QEMU's `-initrd` takes a **comma-separated list** for
multiboot, and the kernel's `machine_with` passed exactly one path. It now
passes a list, and the order is the contract: module 1 is `user/init`'s flat
image, unchanged, because it is not a component with a manifest and every
existing boot depends on it being first; modules 2 and beyond are component
files, each a record and an image. `main::component` keeps naming module 1 by
index, and `component::modules` reads the rest by magic rather than by position
— a module whose first eight bytes are not the record magic is skipped and
counted, so a loader that reorders them produces a smaller topology rather than
a component built out of the wrong bytes.

### What this task builds, and what it honestly does not

RFC 0008 fixes the mechanism; this section fixes what of it runs at E1-B05, so
that E1-P06 and E1-B08 are not surprised by a gap they discover.

**Built here, and exercised at boot on every run.** Stated as a list of what
runs rather than of what exists, because the difference between those two is
what this section is for.

- **A place**: a manifest hash, an endpoint carrying the five rights defined on
  one, and at most one occupant. A spawn naming a different hash is refused — a
  different manifest is a different place — and the hash covers the image, so a
  component whose *code* changed is refused as firmly as one whose declaration
  did.
- **Spawn from a record**, with the account charged a page at a time through the
  same derive a component's own growth uses, every part of the instance zeroed
  before it is handed over, and the admission refusal taken before anything is
  spent: `ADMISSION/MEMORY` for an account smaller than the manifest declares,
  and `ADMISSION/NOT_SCHEDULABLE` for a hard class this build cannot promise.
- **The supply, checked rather than believed.** RFC 0008's spawn entry supplies
  one handle from the supervisor's own table per need, in the manifest's order,
  and the frame checks each: the declared type, at least the declared rights,
  `GRANT` beside them, and at least the declared *quantity* — `bytes` for an
  untyped region and `frames` for a frame, which are fields a reader would
  otherwise have to take on trust that somebody read. A need not supplied and
  not optional refuses; a handle for a need the manifest does not declare
  refuses. Five refusals with five codes, and all five are **provoked on every
  boot** by `component::probe_refusals`, because a check nobody has watched fail
  is indistinguishable from one that cannot fail. The supervisor's side of the
  same entry — carving the declared quantity out of its account — is
  `component::offer`, and it moves above the frame with the policy.
- **One control ring per component**, laid out by `f_abi::layout` in a frame the
  account paid for, mapped into the instance, opened at the occupant's epoch,
  and requiring `feature::CONTROL_EVENTS` rather than offering it — a control
  ring whose peer cannot speak notices is not a control ring and the spawn does
  not proceed.
- **The pending-notice state machine**, in `abi/src/control.rs`, with RFC 0008's
  five states and all three collision rules tested at every collision on the
  host; the field itself in a capability slot, the stop word that only moves
  earlier, and the two latest-wins grades in the table. The *storage* half — the
  packing into a slot's type byte, the watermark that moves back, the name given
  up with its descendants, and the rule that a slot owing a notice is not
  refilled — is checked on every boot by `cap::properties::storage`, which is
  five checks reported beside the five authority properties.
- **Notices delivered, and not merely owed.** Every notice both tables owe is
  posted onto a control ring as a completion entry carrying `cflags::NOTICE` and
  read back off it at a polling point, in the order `ORDER` fixes, in as many
  post-then-drain rounds as it takes — because a control ring is sixteen entries
  and a table owes more than that, which is the case the pending-state design
  exists for. R05 is satisfied by the entry being in the ring, not by the state
  being pending. Six of the seven kinds arrive on every boot and the boot log
  says *how many arrived* rather than how many are defined; the seventh is
  *reclaim*, which is per core in a component's allocation, and nothing here
  holds an allocation because nothing here is scheduled.
- **Connect against a place**, and **all three of its outcomes, each driven at
  boot**: a refill completes it with a channel whose header carries the new
  occupant's epoch, a deadline that passes with the place still empty completes
  it `PEER/EMPTY` — the code this task adds beside `GONE`, and which is
  deliberately not `GONE` because the place may yet be refilled — and a *retired*
  place completes it `PEER/GONE`, both for a connect arriving and for one
  already waiting. Retirement is reached the way a supervisor reaches it, by
  spending the budget: the demonstration decides until the policy says
  `Retire`, tears the place down with `cause::RETIRED`, and then submits against
  it.
- **The two counters kept apart.** `Report::lost` is the number gate G1's
  sentence is about — a client that observed anything except added latency — and
  `Report::probed` is the frame taking an outcome on purpose. They are the same
  branch with one flag choosing which one increments, so the zero in `0
  client(s) lost` is a claim about the client rather than about unreachable
  code.
- **Uniform teardown** in the order RFC 0008 fixes, with the frames refunded to
  the account they were charged to, the names the supervisor minted given up,
  and the peer-gone notice posted to the endpoint's holders — and the free count
  required to come back exactly where it started or the boot fails. Three of the
  four causes are driven at boot: a fault, a stop whose deadline had passed, and
  a retirement. The fourth, an *exit*, is the same call with a different cause
  word and is not driven, because a component that exits has to have run and
  nothing here is scheduled. There is no shootdown, and that is correct rather
  than deferred: an instance's address space is never in `CR3` on any core in
  this build, so the shootdown is the empty case — `process::withdraw` is where
  the non-empty case runs, on every `cap=unmap` boot.
- **The restart policy**, applied by one function over a record and a tally,
  with a backoff that doubles and caps, a budget counted over a window, and a
  retirement when it runs out. The tick count the window is measured against is
  read from `Env` once, in `main::kmain`, and converted from nanoseconds at the
  frame's own `TIMER_HZ`; the demonstration then advances it by the backoff the
  policy itself returned, which is what a supervisor does and is why nothing in
  the boot log moves between a fast host and a slow one. Both edges of the
  window run: a count exhausted inside it retires the place, and the same count
  once the window has elapsed does not.

The four **opcodes** are defined in `abi::control::op` with their meanings and
their closed `known` test, and the frame implements the operations behind three
of them — spawn, connect and stop. What does not exist yet is the *submission
path*: no component submits on its control ring, because no component can adopt
one. `grant`, the powerbox's one operation, is defined and not implemented for
the same reason — there is no second component to broker between. Both wait on
the same thing, which is the first deferral below.

**Not required, and said so rather than enforced.** A boot that carries no
component file prints one line and carries on. The milestone this kernel already
passes on metal installs exactly one module — `docs/booting-on-hardware.md` and
`tools/f-on-metal.sh` both — and a demonstration that turned that machine into
one that halts would be R04 read the wrong way round: failing closed on
something the milestone does not require is failing. A component file is now an
optional third module in both, and the kernel says which of the two it got.

**Deferred, and named so it is not mistaken for done:**

- **The supervisor's policy does not yet run at ring 3.** The place table, the
  policy and the budget live in the frame in this task, in the core-local shard
  a running component's core owns. RFC 0008 is explicit that restart is the
  supervisor's act and not the frame's, and this is a deviation with a reason
  and a date: a supervisor at ring 3 has to drive a control ring, driving one
  means adopting a mapped channel, and adopting one is `unsafe` — which a `user/`
  crate may not write. The safe adoption a component needs is **E1-B08's**, and
  it is the one thing that stands between the code here and the division RFC
  0008 asks for. The frame-side policy is written as one function over a record
  and a tally, taking no kernel state, so that moving it is a move rather than a
  rewrite. *Reversal, and it is a date rather than a measurement:* E1-B08 lands
  a safe channel adoption for components, and this policy moves above the frame
  in the same change.
- **A restarted instance is built and admitted, and is not scheduled.** There is
  no scheduler until E1-B08, so an instance runs when the frame hands it a core
  and not before. The demonstration runs each occupant in turn, which is enough
  to show a client's connect pending across a death and resuming at a higher
  epoch; it is not enough to show a driver killed *under load*, which is E1-P06's
  and needs the scheduler.
- **A spawn grants into the child's table; it does not derive across tables.**
  RFC 0008 wants the child's capability to be a descendant of the supervisor's,
  so that revoking the supervisor's `Untyped` reaches it and so that the
  derivation tree is the route for revocation across components. The cross-table
  parent link is E1-B13's, and RFC 0029 says in its own *Affects* line that it
  deliberately did not land it. Until it does, what ends a component is the
  frame ending it — `component::tear_down` — and not the revocation walk
  crossing. The account is still real, and what it bounds is still real: an
  instance's frames are retyped out of it and refunded to it. What is not yet
  true is the sentence *revoking a supervisor's `Untyped` ends every component
  it paid for*, and that sentence is worth more than the convenience of writing
  it down early. *Reversal:* a cross-table parent link in `cap.rs`, at which
  point the grants become derives and this paragraph goes.
- **The page tables under an instance are not charged to its account.** Text,
  stack and the control ring are retyped from the supplied `Untyped`, which is
  the account RFC 0008 names. The page tables under them come from
  `paging::user_space`, which allocates. They are returned exactly — the free
  count comes back to where it started on every boot, which is asserted — so
  nothing leaks; what is not yet true is that a supervisor's quota bounds them.
  *Reversal:* `paging::user_space` taking frames rather than an allocator, which
  is a change to that module and not to the lifecycle.
- **An instance is built and admitted, and is not scheduled.** Stated twice on
  purpose, because it is the sentence separating this task from gate G1: the
  demonstration provokes a fault, a teardown, a restart and a resumed connect,
  and every one of those is the real mechanism against real memory — but nothing
  is *under load*, because nothing is scheduled. E1-P06 is where load arrives and
  E1-B08 is what makes it possible.
- **The door has not shrunk.** RFC 0014 and RFC 0015 wrote *the calls are still
  here after M5* as their reversal condition and RFC 0008 made it fall due here.
  It falls due and is not paid, for the reason above: a component with no safe
  way to adopt its control ring has no second path in, and retiring the door
  before there is one would leave the first component in this system unable to
  say anything at all. `ANNOUNCE`, `PROGRESS` and the four capability calls stay
  until E1-B08, and this paragraph is the record that they were not simply
  forgotten.

## Context

What was true when this was decided.

`docs/manifest.md` had already named both halves of the first decision and left
them to this task by name: *the supervisor does not read TOML at all — it reads
a fixed-layout record that E1-B05 defines in `abi/`*, and, under *what this
schema does not decide*, *E1-B05 defines the `#[repr(C)]` form in `abi/`, with
`Unit:` on every field, and the hash a spawn names is over it and the image
together*. This RFC is therefore less a choice than a discharge — but the
alternative was live enough that the schema document wrote its own reversal
condition for it (*a second reader*), and a decision recorded only as a
deferral in a schema document is a decision the next contributor gets to make
again.

`kernel/src/cap.rs` did **not** carry the pending-notice field RFC 0008 assigns
to E1-B13. That was checked rather than assumed: the slot at the end of E1-B13
holds a kind, rights, a generation, a parent and a mapping, and the table holds
its pages, its growth count and its generation floor — and no notice state
anywhere. RFC 0029, which is E1-B13's record, says so in its own *Affects* line:
it touches `abi/` **not at all**, and the notice field is an ABI-shaped thing.
So the field arrives here, with the opcodes that read it, which is the better
seam anyway: a field with no reader is a field whose five states nobody has had
to get right.

The five states cost nothing, and that is the second thing that was checked
rather than assumed. A slot is exactly thirty-two bytes with no padding, so a
sixth field would have made it forty and cut a bought page from a hundred and
twenty-eight slots to a hundred and two — a fifth of every table a component
buys, spent on three bits. The type byte carries a `CapType` whose wire values
run to seven, so its top five bits were already unused and are where the notice
state goes. `Slot::occupied` masks, and that mask is the whole cost.

The alternatives that were live for the first decision, and why each lost:

- **A TOML reader in the frame.** Rejected on the argument above, and on a
  second one: the subset `xtask/src/manifest.rs` accepts is chosen to be the
  smallest thing the schema needs, and a second implementation of even a subset
  is a second set of beliefs about one file. `docs/manifest.md`'s reversal
  condition for its own subset is *a second reader*, and putting one in the
  frame would have triggered it on the day it was written.
- **The image carries its manifest in a header the linker writes.** Attractive:
  one file, no `xtask` step. Rejected because the linker script would then be
  the schema, and a schema expressed as a linker script cannot be checked by
  `lint-manifests` at all — the refusal would move from lint time to link time
  and stop naming which field was wrong.
- **The record and the image as two modules, hashed separately.** Rejected
  because two hashes is two identities, and RFC 0008 rests on there being one:
  *the manifest is reached by the same hash, so what a component is — its code
  and its declared shape together — is one name*. Two modules also makes a
  loader that drops one produce a component with a manifest and no code, which
  is a state nothing else in this design has.

And for the second decision:

- **`init` spawns the first supervisor.** Rejected: `init` would then need a
  control ring, an `Untyped` and a topology, which is to say it would be the
  supervisor under a different name. The one-line version is that there is no
  work for a component between the frame and the first supervisor to do.
- **The frame *is* the supervisor, permanently.** Rejected by RFC 0008 —
  *policy in the frame is policy nobody can replace* — and this RFC's deferral
  above is a schedule against that decision, not a reversal of it.

## Consequences

**Easy.** A component's identity is one hash over one blob, so measured boot,
the simulator's component substitution and `cargo xtask component` all name the
same thing. Adding a component is a crate, a manifest and a line in the module
list — no kernel change, because the frame reads records by magic rather than by
position. The frame's spawn path has no branch for the first component, so the
bootstrap is not a special case anybody has to keep working. And every bound in
`docs/manifest.md` is now load-bearing rather than aspirational: a schema change
that outgrows the record fails to compile, because the record's size is
asserted.

**Hard.** There are two spellings of every manifest quantity — milliseconds in
the source, ticks in the record — and a conversion between them that lives in
one place and must stay there. A record is versioned by `schema`, so a schema
bump is an ABI-shaped change with a rebuild of every component file, which is
the cost of the frame not guessing. And the deferrals above are real: a reader
of `kernel/src/component.rs` will find policy in the frame, and only this
document says why and until when.

**Forecloses.** A component configured at run time; a manifest read from
anywhere but the module it arrived in; and a supervisor that hands the frame a
structure the frame does not re-validate. The last one is the one worth naming:
there is no path by which a supervisor's belief about a manifest becomes the
frame's belief about it.

## What would reverse this

**A component whose manifest cannot be fixed-width.** A driver that legitimately
needs a variable-length field — a device path, a firmware blob, a policy table —
and cannot express it as a capability it is routed instead. The record would then
grow a trailing variable section with its own length prefix, hashed with the
rest, and the fixed part would keep every bound it has; that is an extension
rather than a reversal of this decision, and it should be written as one so the
bound does not simply disappear.

**A second reader of `manifest.toml` above the frame.** `docs/manifest.md`
already wrote this condition for its own subset and this RFC inherits it: if a
tool or a supervisor ends up parsing the source rather than the record, the
answer is to make the record the only thing that is read, not to grow the
subset. The observation is a `manifest.toml` opened by anything but
`xtask/src/manifest.rs`.

**The policy deferral outliving E1-B08.** If a safe channel adoption lands for
components and the restart policy is still in the frame afterwards, then this
document's *reversal is a date* was a plan rather than a mechanism, and R01
applies to it. The measurement is trivial and it is exactly the one to make:
`grep` for the policy function's name and see which crate it is in.

**A module list a loader reorders.** The frame reads component files by magic
and skips what it does not recognise, which is the fail-closed reading. If a
real loader at E5 turns out to hand modules over in an order that makes
`user/init` no longer first, then module 1's position stops being the contract
and `init` acquires a record of its own — which is a smaller change than it
sounds, and is the direction this design already points.
