# RFC 0066: The assembler lands above the frame with no caller, rather than inside it with one

- Status: accepted
- Date: 2026-09-07
- Affects: `user/assembler/` (its home, and the `Start` trait at its edge); `kernel/src/component.rs`, which is unchanged and says why; `intent/0006-state/spec.md`'s *Risks* entry naming this decision as owed; `TODO.md` E2-B05's exit, which this bounds; `E2-B07` and `E2-P07`, which inherit the residual

## Decision

**`f-assembler` is a library above the frame at `user/assembler/`, and nothing
at boot calls it yet.** It decides the whole of what `E2-B05` is about — refold
the module, check the routes against the manifests, bind drivers by declared
property, compute the start order, and record what a failure cost — and hands
the act of starting to whoever implements `start::Start`. The frame's own boot
path in `kernel/src/component.rs` is untouched: it still walks boot modules by
magic and fills its four places itself, exactly as it did before this task.

The alternative — landing the assembler *in* the frame so that a real boot goes
through it today — is refused. It would work, and it would be the second owed
reversal of RFC 0008 in the same file, on top of the restart policy that is
already there and already reported by `cargo xtask lint-owed`.

## Context

`intent/0006-state/spec.md` named this decision before it was needed: *a
decision about the assembler's home if `E1-B05`'s policy has not left the frame
when `E2-B05` is otherwise ready*. `E1-B05` has not moved. `cargo xtask
lint-owed` still reports `kernel/src/component.rs — RFC 0008: the restart policy
runs in the frame, where that RFC says it does not belong`, and E1-B05's own
`TODO.md` row records the wall it hit: a supervisor at ring 3 has to drive a
control ring, driving one meant adopting a mapped channel, and adoption was
`unsafe`. RFC 0037 removed that wall for a *driver*; what has not happened is
the move of the policy itself.

So `E2-B05` arrived ready with nowhere to be called from, and the three live
options were:

- **In the frame.** The assembler runs in `kernel/`, the boot really is a
  function of one hash today, and E2-B07 and E2-P07 unblock immediately. The
  cost is a second reversal of RFC 0008 in the same file — and a much larger
  one than the first, because a topology instantiator is not one function taking
  a record and a tally the way `component::policy::decide` was deliberately
  written to be. Moving it out later would be a rewrite rather than a move,
  which is precisely the mistake E1-B05 took care not to make.
- **Not at all — wait for E1-B05.** Honest, and it blocks `E2-B06`, `E2-B07`
  and `E2-P07` behind a task in another epoch's movement. It also leaves the
  hardest question in this task — *what does byte-identical mean, and over
  what* — unanswered and therefore un-reviewable.
- **Above the frame, with the caller injected.** What was built.

There is a second reason the third option is not merely the least bad. The crate
needs a heap: a `BTreeMap` keyed by device address is what stops a bus scan's
order from reaching a topology, and RFC 0004's *iteration order* rule is the
whole argument for the map. An image linking it therefore needs a
`#[global_allocator]`, that is an `unsafe impl GlobalAlloc`, and RFC 0001
forbids `unsafe` above the frame — so `f-assembler` could not be a *component*
today even if a supervisor existed to be its parent. `user/objects/` recorded
the same debt one directory over two commits earlier, waiting on the same heap
in `ring/` that `intent/0006-state/spec.md`'s decision 4 puts there.

## Consequences

**What is closed.** Both halves of `E2-B05`'s exit are demonstrated rather than
argued, and neither is a hash comparison: `render::topology` states in bytes
what *byte-identical* is taken over — every decision the assembler made, and
deliberately not the module, which is the input — and `cargo xtask generation`
instantiates its own output twice and prints the digest, so the claim holds for
this tree's real generation rather than only for a fixture. A driver failed on
purpose leaves its subtree unstarted, two routes deep, with the boot alive and a
report coming back. Discovery order cannot reach a topology, which is RFC 0065.
`user/generation.toml`'s standing promise that its routes are *checked against
the manifests by `E2-B05`* is paid.

**What is not.** No booting machine goes through this crate. `cargo xtask run`
boots the frame's own path, and *boot is a pure function of one hash* is
therefore a property of the assembly and not yet of the frame's boot. That
sentence is the residual and it is written here rather than left for a reader of
a green test to infer.

**What it makes easy.** `E2-B06` and `E2-B07` can be written against a topology
type that exists, with a `Start` implementation as the only new thing between
them and a real boot. `E2-P07`'s rollback compares a root against a module that
something already refolds.

**What it makes hard.** Nothing demonstrates the assembler under the frame's
real spawn refusals — a need not supplied, an account too small — because
nothing has run it there. The `Start` trait's implementations are a test's
today, and a trait with one test implementation is a trait whose shape has not
met a real caller.

**Why the trait is not a callback.** R05 forbids an interface that lets a *peer*
register code. This is a trait the system implements and this crate calls into,
which is the distinction `cargo xtask lint-callbacks` already draws for
`f_env::Env`, and that lint is green over `abi/` and `ring/` where the rule
lives.

## What would reverse this

- **`E1-B05`'s policy leaving the frame.** The condition this RFC exists
  under. On that day the supervisor is a component, `start::Start` is
  implemented by it as a `SPAWN` on a control ring, and this decision is spent:
  what changes is one impl, not this crate. `cargo xtask lint-owed`'s first row
  going green is the observation.
- **`ring/src/heap.rs` landing.** The other half. With a `#[global_allocator]`
  above the frame, `f-assembler` can be a component with a `manifest.toml`, a
  `COMPONENTS` entry and a place of its own — which is what the plan's *with a
  manifest, a `COMPONENTS` entry and a PORTABILITY row* asked for and what this
  task delivered only the third of.
- **A downstream task needing a real boot through the assembler sooner.** If
  `E2-B07`'s measured boot or `E2-P07`'s rollback cannot be demonstrated without
  one, the trade above is re-opened with a concrete cost on the other side — and
  the answer is still not to move the assembler into the frame, but to
  implement `Start` against the frame's existing spawn from the smallest
  possible ring-3 caller.
- **The assembler being wrong in a way only a boot would find.** The evidence
  would be a topology this crate accepts that the frame then refuses to spawn.
  The routing check added here exists to make that class smaller; if one turns
  up anyway, the check is missing a condition the frame applies, and the fix is
  to state it here rather than to stop checking.
