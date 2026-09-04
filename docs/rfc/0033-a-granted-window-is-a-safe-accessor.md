# RFC 0033: A granted window is a safe accessor, and a channel is not

- Status: accepted
- Date: 2026-09-03
- Affects: `ring/` (new: `device.rs` — `Window` and `Region`, and the two
  `pub use`s beside `Mapping`), `user/virtio-blk/` (new: the driver crate the
  accessor exists for), `kernel/src/arch/x86_64/virtio.rs` (new: the
  supervisor's half — find the device, walk its capability list, map its four
  register structures), `kernel/src/blk.rs` (new: the datapath demonstration and
  its verdict), `kernel/src/state.rs` (four nodes and a subtree),
  `kernel/Cargo.toml` and the root `Cargo.toml` (the frame depends on a `user/`
  crate, with a feature turned off and a date on it), `xtask/src/main.rs`
  (`cargo xtask blk`, `virtio-blk` in `COMPONENTS`, the AArch64 check list),
  RFC 0001 (which this does *not* widen, and the first section says why),
  RFC 0024 and RFC 0028 (whose seams this fills from the driver's side),
  RFC 0030 (whose named deferral this narrows by exactly one half);
  `TODO.md` tasks E1-B02, which this is, E1-B01, whose remaining exit clause
  this closes, and E1-B08, which this hands two named deferrals

## Decision

**A component may touch a device's registers and its own DMA region through a
safe accessor supplied by the frame, and may not touch a channel that way.**

`f_ring::device::Window` is a register window and `f_ring::device::Region` is
granted memory a device also reads and writes. Both are constructed by safe
functions taking a raw address and a length; every access through them is
bounds-checked, alignment-checked, volatile, and refuses with
`ARGUMENT/BAD_ADDRESS` rather than panicking. A driver above the frame uses
them and contains no `unsafe`, which is a property the workspace enforces with
`unsafe_code = "forbid"` and `cargo xtask lint-unsafe` backstops.

Nothing about the frame's `unsafe` boundary moves. Every `unsafe` block behind
these two types is in `ring/`, which is one of the three crates RFC 0001 already
names, and there are thirteen of them — one per width per direction — for every
driver this system will ever have.

### Why this is safe to call, stated once so it is not re-argued per driver

This is not a new argument. The tree already makes it twice and both are named
in the code:

- **`f_abi::door::call`** is one instruction on the frame's side of the
  boundary, because *the calling convention is the platform's, not the
  application's* — and a component that hand-rolled it could get it wrong in a
  way only the kernel could detect.
- **`f_abi::state::Reader::at`** is a *safe* function over an address the frame
  mapped, and its own comment already writes out the reasoning this RFC
  generalises: the obligation is discharged against a contract the frame keeps,
  and *a component that invents an address gets a page fault, which is the
  defined machine outcome the entire isolation suite rests on — `cargo xtask
  user` is seven boots of exactly that.*

`Window::at` and `Region::at` are the third and fourth, and the contract is the
same sentence: `base` names `len` bytes the frame mapped for this component in
answer to a capability it holds. It is **not sound by Rust's rules** and saying
so is the point rather than a caveat. What makes it acceptable is that the
failure mode is the one the hardware exists to produce, and that the alternative
— every driver containing the same `unsafe` block written slightly differently —
is worse in every direction: more `unsafe`, in crates nobody reviews as the
frame, with thirteen chances per driver to get a bound wrong.

### What is checked, and what is contract

Per access, always: the offset plus the width falls inside the length, and the
offset is aligned to the width. Both refuse rather than round. That is R04 at
the one place being helpful is most tempting — a length clipped to fit is a
driver told its transfer succeeded over fewer bytes than it named, and an offset
rounded down is a register write somewhere else. It is also a soundness
obligation and not only a policy: an unaligned volatile load through a raw
pointer is undefined behaviour, so the alignment check is what stands between a
driver's arithmetic slip and a hole in the frame.

Once, at the constructor: the address is the frame's. There is no check for this
and there cannot be one — a component that could tell whether an address was
granted would be a component with a page walk.

### Two types, because the readers differ

A `Window` is registers: reads can have side effects, two reads of one address
can differ, and the device is watching the order. A `Region` is memory a device
also reads and writes, and it carries **two** addresses — where this component
sees the bytes and where the *device* does. The second is
`f_ring::registry::Domains::map`'s answer, which is the frame's, and carrying it
in the type is what stops a driver assuming the device's address space is its
own. `kernel/src/iommu.rs` already writes the reversal condition for that
assumption — a device that cannot address the whole of physical memory — and a
driver written against `Region` needs no change on the day it falls due.

Neither type hands out a slice, and that is the same decision
`f_ring::Mapping` makes: a slice asserts exclusive access to memory whose whole
purpose is that something else writes it.

### And why a channel is not one of these

**A channel is shared with a hostile peer; a device window is not.**

`f_ring::Mapping::adopt` is `unsafe` for a reason that has nothing to do with
whether the address was granted. Its region contains a header a peer wrote,
cursors a peer advances, and an arena a peer may be scribbling while an
operation reads it — `ring-scene-boot` section 06 assumes a peer that is
compromised while holding a live channel. The obligation `adopt` carries is
*the only references into this range are ones this type hands out*, and that is
an obligation about aliasing between two live writers, which no bounds check
discharges.

A device window has one writer on this side, and the device on the other is
bounded by its IOMMU domain rather than by this component's discipline. So the
two are different problems and this RFC solves one of them. Extending `Window`'s
argument to channels would be the wrong argument used twice, and it would be
used at exactly the place where being wrong is most expensive.

**The consequence is a deferral with a name and a date, not a gap.** A component
still cannot drive a ring, so the driver this RFC exists for is called by the
frame — `kernel/src/blk.rs` — where a scheduled component would call it from its
own polling loop. That is the same wall E1-B05 hit and recorded for the restart
policy, and the same one RFC 0030 booked against E1-B08. This RFC narrows it by
exactly one half: the device half is solved, the channel half is not, and
E1-B08 owns the rest.

## Context

What was true when this was decided.

E1-B01 finished with `kernel/src/iommu.rs::Grant` implemented and *constructed
by nothing but the frame's own adversary*, and with an exit criterion whose last
clause it could not observe: *a driver component provably cannot address memory
outside its grant*. Its own notes say so — **the word component in it belongs to
E1-B02** — and `TODO.md` records the defect that makes that worth naming: one
criterion belonging to two tasks means one of them is always lying about its
state.

E1-B05 had already hit the wall from the other side, twice: a supervisor cannot
adopt its control ring, so RFC 0008's restart policy runs in the frame, and
RFC 0014's and RFC 0015's reversal conditions fell due and were not paid. Its
own summary is the sentence this RFC starts from: *the wall it hit is worth more
than what it built*.

So the question this task could not avoid was the one E1-B05 wrote down and
deferred: **how does a component touch anything the frame gave it?** The
alternatives were live and each loses for a different reason.

- **Widen the frame to include driver crates.** The obvious answer and the
  worst: RFC 0001's partition is enforced by the compiler, and a `user/` crate
  with `unsafe_code = "allow"` is one line in a `Cargo.toml` after which the
  property stops being a property. RFC 0001 sets its own reversal condition at
  ten percent of the tree; this would move the number in the wrong direction for
  every driver, forever, to save thirteen accessor bodies written once.
- **A syscall per register access.** Sound, and unusable: a virtio handshake is
  about thirty register accesses and a queue kick is one on the hot path. It
  would also be the frame acquiring an interface — RFC 0014 says the door does
  not accumulate one, and RFC 0030 has it shrinking rather than growing.
- **A macro in `abi/` that expands to `unsafe` in the component.** Rejected for
  the reason `cargo xtask lint-unsafe` exists: the lint is textual, a macro is
  how a property enforced by grep stops being enforced, and the resulting
  `unsafe` would live in a crate nobody reviews as the frame.
- **Nothing: the driver stays in the kernel.** This is what E1-B01 did with
  `dma.rs`, deliberately and for one task. Keeping it would mean the first three
  device drivers are frame code, which is the arrangement this whole project
  exists to argue against, and it would mean E1-B03 and E1-B04 inherit the
  decision without anybody having made it.

The other thing that was true, and that decided the *shape* of the answer:
`f_ring::registry` already had `Domains` and `PageWalk` as seams, and
`f_ring::buffers` already had the ownership typestate. So a driver written
against those two plus a safe accessor is a driver with no remaining excuse to
touch a client's bytes — which is what made *zero copies on the data path*
expressible as a structural property rather than a discipline.

## Consequences

**Easy.** A driver is an ordinary safe Rust crate. `user/virtio-blk` is a
transport, a virtqueue and a service loop, all of it testable on the host
against ordinary memory — thirteen host tests run under `cargo xtask test`,
including one that reads a descriptor back at fixed offsets rather than through
the writer that wrote it. E1-B03 and E1-B04 write no `unsafe` either, and the
accessor they share is reviewed once.

The manifest also stops being decoration. `user/virtio-blk/manifest.toml` was
written before the driver *and* before the supervisor that routes for it, so
`kernel/src/blk.rs` reads the compiled record on every run and sizes what it
routes from the declaration: the untyped region is the `queues` need's `bytes`,
and a device describing more register pages than the `mmio` need declares is
**refused** rather than served — which is the direction the manifest itself
argues for, *a device whose BAR is larger is a different device and a different
manifest, not a bigger number*.

*Zero copies on the data path* becomes structural rather than careful: a
`Reach` is an address and a length and deliberately not a slice, a `Region` is
the driver's own memory, and there is no type in the driver crate that turns a
client's buffer into bytes. The counter that says so is published — and beside
it is a second counter the boot moves through the *same* copying function on
purpose, because a counter nothing can move is not a counter. That is the
argument `state::node::MEMORY_FORCED` already makes beside `MEMORY_REMOTE`.

**And the structure is checked rather than described.** Review found the hole in
the paragraph above, and it is worth stating plainly because it is the shape of
hole this whole decision creates. *There is no type in the driver crate that
turns a client's buffer into bytes* is a claim about that crate's source, not
about a boot; the published zero would read exactly the same from a crate that
had grown a second way to move bytes, so on its own it was an assertion in a
comment with a `u64` around it. Worse, this RFC's own accessor was the easiest
way to grow one: `Region::at` is a **safe** `const fn` over a bare address, so a
component that forbids `unsafe` can name the direct map, build a region over a
client's buffer and read it through the accessor made safe here — with the
crate's copying function untouched and the counter still at zero.

`cargo xtask lint-datapath` closes both. For each crate that publishes a
zero-copy counter it requires exactly one function that moves bytes, exactly one
call to it, that call inside the boot's own self-check, and **no shipped line of
the component that mints a `Region` or a `Window` out of an address it
invented** — a component receives a granted window, which is this RFC's decision
restated as a check instead of as a sentence. Test fixtures are exempt at the
first `#[cfg(test)]`, because a host test has no frame to be handed a window by.
Five tests in `xtask` are the fixture that breaks it, including the two above.

That the lint has to exist is a consequence of the safe constructor and belongs
here rather than in the driver: the alternative — making `Region::at` and
`Window::at` `unsafe fn`, so a component could not call them at all — is a real
option and it loses on one point, which is that the driver crates' own host
tests build regions over ordinary arrays and cannot write `unsafe` either. A
`#[cfg(test)]`-gated safe constructor would work and would put a build-mode
distinction in the middle of a soundness argument, which is worse than a lint
that says the same thing in one place and can be read.

**And the driver can now be the adversary.** `E1-B01` proved *a driver cannot
address memory outside its grant* with the frame's own hand-built descriptor,
and wrote down that the word *component* in that sentence belonged here. Review
found that the first attempt at it proved something adjacent instead: `blk
outside` withdraws the client's translation under a descriptor the driver built
correctly, which is RFC 0024's reclaim and is the *frame's* property, not the
driver's. So there is a third half. `blk escape` takes nothing away —
`Driver::provoke_escape` resolves the registration and then adds a frame to the
address **itself** before writing it into a descriptor, which is arithmetic no
type in that crate can prevent, because a `Reach` is an address and a length and
an address is an integer. That is precisely the point: the answer to a driver
doing arithmetic on an address has to be the remapping unit and not a type. The
unit faults it at the address the driver invented — `0x…257000`, one page past
the `0x…256000` the registration answered — and `Report::verdict` requires both
that the fault is at that address and that the provocation ran, so a build where
nothing escaped cannot report the protection holding.

**Hard.** There is now a safe function in the tree whose soundness rests on a
contract rather than on a type, and it is reachable from a crate that forbids
`unsafe`. A component that computes an address and hands it to `Window::at` is
not refused; it takes a page fault, and the frame kills it. That is the right
outcome and it is a *different* kind of guarantee from the one the rest of the
workspace's safe code offers, so it is written on the type, in the module, and
here.

The frame also depends on a `user/` crate for one milestone, which reads
strangely on purpose. `kernel/Cargo.toml` says why in the dependency itself and
turns off the feature carrying the component's `#[panic_handler]`, because a
lang item may be defined once per linked artefact.

**Not done, and named rather than implied.** The component file is built,
hashed and carried as a boot module, and its record is what the datapath routes
against — but it is **not spawned into a place**. `component::demonstrate` fills
one place from the first component file the loader carried, and this manifest's
`memory_bytes` is two mebibytes against an account the frame stakes at a hundred
and twenty-eight kibibytes, so a spawn would be refused `ADMISSION/MEMORY`
before it began. That is E1-B05's machinery and E1-B05's constant, and enlarging
either from here would be this task editing another's demonstration to make its
own claim larger. *Reversal:* a supervisor that sizes an account from what it
was routed rather than from a constant, which is E1-B08's, and at which point
the driver is spawned into a place like anything else.

**Forecloses.** A driver that maps its clients' buffers. A driver that reaches
configuration space — the supervisor routes four windows, not a bus, and
`user/virtio-blk/manifest.toml` declared exactly that before the driver existed.
And a virtio driver that runs without `VIRTIO_F_ACCESS_PLATFORM`: `Transport::open`
refuses a device that does not offer it, because a device that addresses
physical memory by specification is a device with no isolation and no way to
know — which is the failure E1-B01 found the hard way and the one thing about
this datapath that must not be discovered twice.

## What would reverse this

**A component that needs to write `unsafe` anyway.** If E1-B03 or E1-B04 turns
out to need something these two types cannot express — a scatter-gather engine
whose descriptors are not a fixed layout, a device whose registers must be
accessed as a burst — then the accessor is the wrong shape and the answer is to
widen the accessor, in `ring/`, rather than to widen the frame. The observation
is a driver crate's `Cargo.toml` acquiring a `[lints]` override, and that diff
is the thing to catch.

**A page fault from a component that had a bug rather than an exploit.** The
contract's cost is that a bad address is a dead component instead of a refused
call. If that turns out to be a routine debugging experience rather than a
theoretical one, the answer is a frame-side validation — the component asks the
frame *is this mine* once, at bind, rather than never — which is one call and
not a per-access cost. The observation is a driver that dies at `Window::at`
more than once in development.

**The channel half outliving E1-B08.** This RFC's first section argues that a
channel is a different problem; that argument is only honest if the difference
is eventually paid for rather than used as a reason to stop. If E1-B08 lands and
`kernel/src/blk.rs` still calls the driver directly, then *a channel is not a
window* was a reason to defer rather than a distinction, and R01 applies to it.
The measurement is trivial and is exactly the one to make: `grep` for
`Driver::execute` and see which crate calls it.

**A safe constructor that the lint is the only thing holding.** `lint-datapath`
is a source check: it reads names, and a component that reached a client's bytes
by some shape it does not know about would pass it. If a second way is ever
found — a helper in `ring/` that yields a slice, a `Region` handed across a
call the lint cannot see — then the answer is not a third clause in the lint. It
is to make `Region::at` and `Window::at` `unsafe fn` and give the driver crates
a constructor that cannot name an arbitrary address, paying the host-test cost
argued above. The observation is a finding this lint did not catch, which is a
thing to write down when it happens rather than to argue about now.

**A device the supervisor cannot enumerate for.** The four windows arrive
because `kernel/src/arch/x86_64/virtio.rs` walks the capability list on the
component's behalf. A device whose register layout can only be discovered by the
driver — a firmware blob, a class of device with a vendor-specific probe — would
break that split, and the answer is not to hand a component configuration space.
It is a routed capability that names *one function's* configuration space and
nothing else, which is a narrower object than this build has and a change to the
manifest schema rather than to this decision.
