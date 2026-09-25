# RFC 0134: The input path predicts too, and is asked the display's question

- Status: accepted
- Date: 2026-09-25
- Affects: `user/virtio-input/src/{forecast.rs,driver.rs,component.rs,routing.rs,lib.rs}`
  (a second predictor instance, fed where each report closes, published and never
  sent; `at::SCANOUT_AT_NANOS` and eight `reported::` words),
  `kernel/src/input.rs` (`FIRST_SCANOUT_NANOS` written to the driver; `Gesture`;
  the driver's forecast checked on every half and compared with the latch on the
  steady gesture), `xtask`'s input harness (`LEAD_MOTIONS`, a third run `lead`,
  and the harness's *how far it moved* over three witnesses),
  `user/compositor/src/latch.rs` (a host test at the boot's own numbers), and
  `TODO.md` task `E3-B04e`. Nothing in `abi/`, nothing on a wire, and
  `xtask`'s `INPUT_PATH` unchanged — no row added, none moved.

## Decision

**The virtio-input driver holds its own `f_input::predict::Predictor`, fed with
each report's position at the reading the report was stamped with, and asks it —
once, after its last report — where the pointer will be at an instant the frame
told it on its routing page: the first scanout of the display the frame
declares.** The answer, its anchor, its lead and its basis are published on the
driver's routing page and are never submitted. A third run of `cargo xtask
input`, `lead`, is the delivering half over a motion that does not turn inside
the predictor's window, and on it the frame requires the compositor's aim, the
told instant and the instant the driver says it was asked about to be one
number; both predictions to be extrapolations, the driver's having moved the
pointer off its anchor on both axes; and the two to agree on the value and the
lead. The harness then requires the compositor's `latched - committed` to be
the motion it injected plus the driver's `predicted - anchor`.

## Context

`E3-B04e`'s exit is *the latched value is the predicted one, asserted from both
sides of the seam — the input path's trace and the compositor's frame trace
agree on the value and on how far it moved*. After RFC 0125 its relay half held
on a boot and its extrapolated half did not, for two reasons the task record
named: the harness's motion turns inside the window, so the latch holds and a
held prediction agrees by copying one sample; and nothing on the input side of
the ring predicted at all, because nothing there learns the instant the
compositor aims at.

The first reason is a fixture and was repaired as one. The turning motion is
not wrong — it is what makes `E3-B01i`'s *by exactly the motion injected* an
equality — so it stays on `deliver`, and `lead` carries a second motion,
positive on both axes in every event and under the predictor's teleport ceiling
at the driver's hundred-microsecond virtual tick. It is a third run rather than
a third `Half`, because it is not a control; the kernel learns which motion was
sent from `gesture=steady` beside `input=deliver`, since only the process that
moved the pointer knows, and the verdict holds the harness to what it said.

The second reason was the question this task was allowed to answer *no* to:
whether a second predictor could be told the aim instant without widening what
the frame may touch. Four routes were weighed.

**Carry the compositor's aim to the driver.** Impossible on this boot rather
than forbidden: on one worker core the driver has been reaped before the
compositor mints its aim, so there is no driver left to tell, and a frame that
relayed the word would first have had to read it off the compositor's board —
the courier arrangement `E3-B04g` ended, with a target in place of two
coordinates. A second driver run after the compositor would need the reports
again, and the device does not replay them.

**Host the predictor in the harness.** It would name `f_input` in `xtask` and
earn `xtask/` an `INPUT_PATH` row — a host tool on the input path, holding a
reading it computes against. Refused.

**Let the driver compute the next scanout from a period.** The driver would
mint a `StampNanos` out of arithmetic, which `lint-stamp`'s mint rule refuses at
the argument site, and which would be a second implementation of
`f_compositor::pacing::scanout_after` — the aim computed twice, which is two
values from one source.

**Have the frame state the display's first scanout, and check the compositor
aimed there.** Taken. The frame is already the party that declares the display:
it writes the period onto the compositor's page. The display's scanouts are the
multiples of that period from the channel's epoch — phase zero, which is
`scanout_after`'s own assumption and carries that function's reversal — so the
first is one period in, a constant in the frame, `FIRST_SCANOUT_NANOS`. Written
onto the driver's page it is a target in RFC 0120's sense: when a display will
show a frame, not when anything happened. The compositor still mints its own aim
out of the period and its clock, and the verdict requires the two to be equal
rather than assuming it, so a compositor aiming at the wrong boundary is a red
boot with a sentence and not a silent disagreement between two predictions.

That route widens nothing `lint-stamp` or RFC 0120 and 0124 draw. The frame
already writes the driver's clock itself — `STAMP_SEED` and `STAMP_TICK_NANOS` —
and a scanout instant is a smaller thing to write than a clock. It names no
reading: `kernel/src/input.rs` contains none of the four `ON_THE_PATH` needles,
and `lint-stamp` passes unchanged with `kernel/` holding no row. It compares
and computes nothing: the words it reads from the driver are positions, a
count, a flag, the echoed target and a lead, and it tests them for equality with
the compositor's. The lead is the one of those a reader could push on — it is
`(target - newest stamp)` damped — and it has a precedent the frame already
reads, `CLOCK_AT`, the driver clock's own position; neither is subtracted from
anything here, and the day a line in `kernel/` does, it is the reversal below.

**What is shared is the implementation and not the instance**, now in a boot as
well as the host test. The driver's predictor is fed in `Driver::drain` where a
report closes, with the `StampNanos` the decoder opened the report with — the
reading itself, not a number rebuilt from an entry — and before anything is
submitted. The compositor's is fed in `LateLatch::drained` from what it decoded
off the ring. The frame requires the driver's predictor to have taken as many
positions as the driver sent as motion, and its anchor to be where the
accumulator ended: two counts and two positions taken on two paths inside the
driver.

## Consequences

**What it makes easy.** A latch that submits something other than its
predictor's answer is now a red boot. On the turning gesture it never was:
latching the anchor instead of the prediction is identical to the correct latch
when the prediction holds, which is the vacuity the host test was written
against and the boot inherited. On `lead` it is red on the value clause, and —
with that clause removed — red again on the harness's three-witness distance.
The mutation evidence is in the task's record.

**The vacuity refusal is in the boot as it is in the host test**, three times:
both bases must be extrapolations, the input path's step must be non-zero on
each axis, and the lead must be non-zero; the harness refuses a zero step on its
own numbers as well. Running `lead` with the turning motion is red on the first.

**What it makes hard.** The driver image grows by a predictor. It is not sent
and it is not on the event path's hot loop beyond one `observe` per report, but
it is code in a component RFC 0100 asks to stay small, and `forecast.rs` is
where a reader finds out why it is there.

**What it forecloses.** Nothing on a wire and nothing on `INPUT_PATH`. The
driver's prediction is never an entry; `latch.rs`'s argument that the
compositor, not the driver, aims the cursor is unchanged and is quoted in
`forecast.rs` rather than contradicted.

## What would reverse this

- **The two components running at once** (`E1-B05`, or a second worker core).
  Then the compositor's aim can reach the input side while its predictor still
  holds the window, the told instant is replaced by the minted one carried on a
  ring, and `FIRST_SCANOUT_NANOS` is deleted rather than kept as a second
  source for the same question.
- **A display that reports its own scanout instant** — RFC 0120's first
  reversal. Then both sides are told the same number off a wire, the frame
  stops stating a phase on the display's behalf, and the aim-equals-told clause
  becomes an equality of two copies of one word, which is to say it goes away.
- **A line in `kernel/` that subtracts, adds or orders the lead, the aim or
  `CLOCK_AT` against anything.** Then the frame is computing with a number
  derived from a reading, it is a stage and not a comparator, and it takes an
  `INPUT_PATH` row — red on the one `rdtsc` in this tree, exactly as RFC 0124
  left it. This RFC is not an argument against that.
- **A commit that lands after the first period.** On this boot the pacing clock
  is six seeded steps of at most 4 096 ns, so the compositor aims at the first
  boundary; a boot whose commit crossed it would aim at the second and go red on
  the instant clause. The repair is the first reversal, not a second told word.
