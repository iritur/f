# RFC 0094: the assembler gains its caller, and it is a component

- Status: accepted
- Date: 2026-09-19
- Affects: `docs/rfc/0066`, `user/supervisor`, `user/assembler`,
  `kernel/src/component.rs`, `kernel/src/generation.rs`, `kernel/src/process.rs`,
  `user/supervisor/manifest.toml`, `TODO.md` `E2-B05`

## Decision

`user/assembler` is called at boot, and the caller is **`user/supervisor`, at
ring 3**, not the frame.

The frame keeps the boot module it already selects instead of dropping it, maps
it into the supervisor's address space as a declared need, and writes the
generation root and the module's extent onto the board the supervisor already
reads. The supervisor instantiates the topology with
`f_assembler::Assembly::instantiate`, and implements `f_assembler::start::Start`
as an `f_abi::control::op::SPAWN` on the control ring it already drives. The
frame answers that spawn in `Serving::spawn`, exactly as it answers the one the
supervisor submits today.

`f-assembler` becomes an **optional** dependency of `f-supervisor`, enabled by
that crate's existing `image` feature and by nothing else.

## Context

This reverses RFC 0066's central sentence — *the assembler lands above the frame
with no caller* — and it does so by that RFC's own terms rather than against
them. Its *What would reverse this* names two conditions, and **both have
arrived**:

- *`E1-B05`'s policy leaving the frame.* It has. `decide` is
  `f_supervisor::policy::decide`, the RFC 0008 row is paid and gone from
  `OWED_REVERSALS`, and `E1-B05` is `[x]`. That entry predicted the shape of this
  change to the letter: *on that day the supervisor is a component, `start::Start`
  is implemented by it as a `SPAWN` on a control ring, and this decision is
  spent: what changes is one impl, not this crate.* One impl is what changes.
- *`ring/src/heap.rs` landing.* It has. `f_ring::heap::Heap::COMPONENT` is the
  global allocator of three components, the supervisor among them, so the crate
  that needs a `BTreeMap` has somewhere to put one above the frame.

So this RFC is not an argument that RFC 0066 was wrong. It is the record that its
condition came due, and of what the arrival cost — because that cost is four
things a reader would otherwise have to rediscover.

**The frame may not link this crate, and the reason is mechanical.** `f-assembler`
declares `extern crate alloc` unconditionally, and there is no `#[global_allocator]`
under `kernel/`. The frame already links `f-supervisor` — with
`default-features = false`, for `routing` and `policy` — so an unconditional
dependency would put an allocating crate in the kernel's graph the first time
anybody built it. Hence `optional = true` and `image = ["dep:f-assembler"]`: the
same gate `user/supervisor`'s `component` module already sits behind.

**Two content addresses name the same bytes, and they are not the same number.**
`f_assembler::topology::Instance::content` is a SHA-256 over a member's bytes,
because the generation tree is folded under SHA-256 and the assembler's whole
claim is arithmetic over that fold. `f_abi::control::op::SPAWN` carries an
`f_abi::manifest::ContentId`, which is FNV-1a, because that is what a place holds
and what `Serving::spawn` compares against. A `Start` implementation must
therefore recompute `ContentId::of(module.file(index))` rather than forward the
address the assembler already has. The two agree by construction — `pack_module`
writes the identical `.fc` bytes the loose file carries — and that agreement is
worth stating because nothing checks it.

**A member with no place is skipped, not failed.** `f_assembler::start::start`
marks the whole subtree of a failed member `Unstarted`, which is the behaviour
`E2-B05`'s second exit clause is about. A member the frame has already filled is
not a failure and must answer `Ok(())`, or the rendered topology claims a boot
failed that did not. The distinction is the load-bearing one in the impl.

**The module is copied, not aliased.** The `.fcm` arrives as a multiboot module
in memory the loader reserved; it is not the component's, and a need whose
revocation revokes nothing is not a need. So the bytes are copied into frames the
account paid for, which is also what makes `admit` able to size the thing.

## Consequences

- One new mapped need, `module`, beside `heap` and `board` — the three the frame
  maps rather than merely granting, each for the same reason: a component that
  must read something before its first instruction has no instruction with which
  to ask for it.
- Three new board fields, all in existing gaps, so no row moves.
- `kernel/src/generation.rs` keeps the selected module's bytes. It still folds,
  still prints and still hands `report` what it hands it today, so the boot log's
  existing bytes do not move.
- The supervisor's account and its manifest's `memory_bytes` grow by the module's
  size, and `cargo xtask lint-manifests` is what refuses the pair when they stop
  adding up.
- **`E2-B05`'s unmet clause is the word *boot***, and this is what closes it:
  `boot is a pure function of one hash` stops being a property of an assembly and
  becomes a property of a boot, with `f_assembler::render::digest` on the boot
  log to say so.
- The frame still walks boot modules by magic when no generation was selected.
  That path is not deleted, because a machine booted with no `f.root=` has no
  topology to instantiate and `docs/booting-on-hardware.md` makes every component
  file optional.

## What would reverse this

- **The supervisor becoming unable to hold the module.** The account pays for a
  copy of every component file in the generation, and that is linear in the
  topology. A generation large enough that the supervisor's account cannot hold
  one is a generation that needs the module mapped read-only from the loader's
  own pages instead — which needs a capability type for *memory the frame did not
  allocate and will not reclaim*, and that is a larger decision than this one.
- **A second caller.** The moment anything other than the supervisor instantiates
  a topology, the question RFC 0066 deferred comes back: `f-assembler` becomes a
  component with a `manifest.toml`, a `COMPONENTS` entry and a place of its own,
  and `Start` crosses a ring instead of a function call. This RFC is the smallest
  thing that works and says so.
- **`held_open` widening past one place.** Today the frame holds exactly one
  place open for the supervisor to fill, so a six-member topology is five members
  the supervisor can only skip. That is honest and it is also small; the day the
  frame holds a *set* open, the skipped count stops being the normal case and the
  assembler's start order starts deciding a boot rather than describing one.
- **The two content addresses diverging.** If `pack_module` ever writes bytes a
  loose `.fc` does not, the recomputation above silently names a different
  component. A lint comparing the two for every member would close it; nothing
  does today, and this is the sentence that says so.
