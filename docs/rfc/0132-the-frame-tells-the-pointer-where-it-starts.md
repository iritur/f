# RFC 0132: The frame tells the pointer where it starts

- Status: accepted
- Date: 2026-09-25
- Affects: `user/virtio-input/src/routing.rs` (`at::ORIGIN_X_X65536`,
  `at::ORIGIN_Y_X65536`), `user/virtio-input/src/component.rs` (the two words
  read and refused unless each is an `i32`), `user/virtio-input/src/driver.rs`
  (`Decoder::starting_at`; `Driver::start` takes the origin; the module's *origin
  of the pointer*), `kernel/src/input.rs` (`COMMITTED_TX_X65536` and
  `COMMITTED_TY_X65536` are `(640, 360)` device pixels and are what the driver
  is told; the `input pointer` line carries the start), `xtask/src/main.rs`
  (`input_pointer` reads the start; `cargo xtask input` subtracts it and refuses
  a start of zero), and `TODO.md` task `E3-B01i`.

## Decision

**The virtio-input driver's accumulator starts where the frame tells it, on
two new routing-page words, and the input boot tells it `(640, 360)` device
pixels and commits the client's pointer transform there.** The driver still
knows nothing of where a pointer *is*; it is told where one *starts*, which is
the frame's choice to make and the client's to commit. `Decoder::new` keeps
starting at `(0, 0)` for host tests; the component uses
`Decoder::starting_at` with the routing page's pair.

## Context

`E3-B01i`'s boot committed the pointer's translation at `(0, 0)` because that
was where the driver's accumulator began and nothing could tell it otherwise —
`kernel/src/input.rs`'s `commit` doc argued the origin was the one honest place
a client that has never been told the position can put it. The argument was
right about honesty and blind to one defect. Over a committed translation of
zero, a latch that computes `tx = committed.tx + predicted` submits exactly the
frame a latch that computes `tx = predicted` does: the kernel's *latched is
where the driver's accumulator ended* and the harness's *latched minus committed
is the motion injected* both pass it. The host test commits at `(640, 360)` and
catches it, so the latch was held; the boot was not. An audit found it by
reading — the shape wave 16 fixed on the matrix, where an identity commit could
not tell a latch that copied the client's scale from one that wrote its own.

Two alternatives were live. A constant start compiled into the driver would
have closed the boot's blindness, but it is a driver choosing a screen position
it has no surface for, which `crate::driver`'s module comment refuses for
clamping on the same grounds. And writing down that *the boot is blind to it,
and the host test holds it* would have been honest and cheaper; it was not
chosen because telling the start costs two words and a constructor, and the
boot is the only place the latch runs on a real device path.

## Consequences

**What it makes easy.** The boot catches an additive latch in three places,
none derived from another: the kernel's latch-against-accumulator clause
(`2·start + motion` against `start + motion`), the harness's subtraction, and —
for a driver that ignored the start — the harness's *moved by exactly the
motion* over the accumulator's own end minus its start. The harness also
refuses a start of zero on either axis, beside the kernel's `const` assertion,
so the blindness cannot come back as a tidy-up.

**What it makes hard.** The `input pointer` line is a format of eleven leading
words now, not five, and the harness checks the word before each number.

**What it forecloses.** Nothing: zero is still an ordinary start the component
accepts, and a frame that does know where a pointer was left can now say so.

## What would reverse this

A pointer whose position is absolute at the device — a tablet, or a
`virtio-tablet` in place of `virtio-mouse` — at which point there is no start to
tell, the device's own coordinate is the position, and the boot commits wherever
that device reports its first sample. Or a latch rewritten to carry the
committed translation and the motion as separate fields, at which point the two
cases are distinguishable without the scene having to make them so.
