# `photodiode-rig` — the instrument that measures input to photon

`docs/design/proving-ground.html` layer 6 says the headline interface number
cannot be measured in software: the software timestamp ends at GPU submission,
and scanout, panel response and the display's own processing are a substantial
fraction of what a person perceives. The instrument it names instead is a
photodiode on the panel and an instrumented input device sharing one timebase.
This directory is that instrument, specified the way `claims/runner-class-A.md`
specifies a machine.

It exists for the same reason that file does. `E3-P02` publishes *input to
photon, p99 under 14 ms*, and a claim whose instrument is a phrase in prose is a
claim nobody can check and nobody can build. `E3-P01a` asks for the bill of
materials; `E3-P01c` asks for the error bar **before** any measurement is taken,
which is the clause that stops an instrument from being specified backwards out
of the first number it happened to produce.

## What this file is, and what it is not

It is a specification complete enough that a stranger could order the parts and
defend a number taken on the result.

It is **not a rig**. Nothing in this repository can buy hardware, and `E3-P01`'s
exit — *the rig is assembled and one capture runs end to end on it* — is not met
by a file. Three of the six subtasks are nevertheless on the near side of the
purchase, and this directory is those three:

- **`E3-P01a`**, the bill of materials, because a rig nobody can order is a
  design for a rig.
- **`E3-P01c`**, the error bar, because it follows from a sample interval and a
  rise time, and both are datasheet figures that exist before the part does.
- **`E3-P01d`**, the software-timestamp review, in `timestamp-review.md` beside
  this file.

What a file cannot do is stop somebody assembling a *different* rig and
reporting numbers as though it were this one. That is the same failure mode
`runner-class-A.md` names about `F_ENVIRONMENT`, and it has the same answer: the
error bar below is computed at **the worst front end and the coarsest sample
interval this specification permits**, so a rig built to it cannot be worse than
the number stated here, and a rig that *is* worse is out of specification in a
way item 2 of the assembly checklist names rather than in a way a reader has to
notice.

## What the rig must be able to do

One requirement, and everything below is a consequence of it.

**Two instants, one clock, and the clock is not the machine under test.** The
electrical moment of input and the optical moment of light must land in one
record of one digitiser, sampled by one timebase, so that the number is a
difference of two sample indices. The machine under test contributes no time to
the measurement at all — not a stamp it takes, not a stamp a driver takes, not a
stamp the capture host takes on arrival.

That is stronger than *a shared timebase*, which is how the design document puts
it, and the difference is worth stating because it is the whole of `E3-P01d`. A
timebase shared *with* the machine under test would put that machine's clock
inside the measurement, and every question about whether its clock is honest
would become a question about the number. A timebase shared only between the
injector's edge and the photodiode's edge does not, and there is then nothing to
ask.

`input/src/stamp.rs` takes exactly one clock reading in F's input path, at the
driver, at interrupt time (`E3-B04a`). That reading is real, it is useful, and
**it is not this number**. `presented_at - stamped_at` is what F can see of its
own latency; input to photon is what a person sees. Publishing the first under
the second's name is the defect `timestamp-review.md` exists to catch.

## The instrument, as numbers

Every requirement this file imposes, as integers with the scale in the name, in
one place so that the prose and the budget below cannot drift apart. RFC 0004:
no floats, including in a document this tree will later compute from.

<!-- spec:begin -->
```
digitiser_sample_interval_nanos        100
digitiser_record_length_samples        500000
capture_window_nanos                   50000000
timebase_accuracy_ppm                  25
inter_channel_skew_nanos               1
front_end_bandwidth_hertz              100000
rise_time_bandwidth_product_nanohertz  350000000
amplitude_hold_denominator             10
optical_step_millivolts                1000
front_end_noise_millivolts             2
error_bar_half_width_nanos             2000
bom_total_usd                          3205
prices_read_on                         2026-09-21
prices_stale_after_days                180
```
<!-- spec:end -->

These are **floors and ceilings, not the parts**. A digitiser with a finer
sample interval satisfies `digitiser_sample_interval_nanos`; a front end with
more bandwidth satisfies `front_end_bandwidth_hertz`. The worked examples below
all exceed several of them, and the error bar is computed from this block rather
than from them, because the error bar has to be a property of the specification
and not of whichever unit was in stock.

## Hardware

Stated as required capabilities with one worked example, for the reason
`runner-class-A.md` gives: a single part number ages out and a capability list
does not. Anything satisfying the requirement column is an equivalent rig.

Prices are **list, in United States dollars, as at `prices_read_on`, before
shipping, duty and local tax**. See *What would change this file* — they are the
part of this document with the shortest half-life, and `verify.sh` goes red on
its own once they are `prices_stale_after_days` old rather than waiting for a
reader to wonder.

**What they are, exactly, because the difference matters when the order is
placed.** They are budget figures to two significant figures, compiled from
catalogue list prices for the worked examples. **They were not re-read from each
vendor's live catalogue on `prices_read_on`**, and this file says so rather than
implying a currency it does not have — the session that wrote it had no way to
reach a catalogue. What the total is good for is deciding whether to buy the rig:
it is right to within a few hundred dollars and it is right about the order of
magnitude, which is the question `E3-P01a` exists to make answerable. What it is
not good for is a purchase order. Whoever places one re-reads every row against
its source, edits the table and `prices_read_on` together, and lets `verify.sh`
go green — and that edit is the first time this table will have been read from
the vendors rather than compiled.

<!-- bom:begin -->
| # | Requirement | Worked example | Source | Qty | Unit USD | Line USD |
|---|---|---|---|---|---|---|
| 1 | Amplified silicon photodetector, 400–1100 nm, DC-coupled, switchable gain, bandwidth at or above `front_end_bandwidth_hertz` at the gain used, BNC output | Thorlabs PDA36A2 | Thorlabs | 1 | 529 | 529 |
| 2 | Fixture holding the detector against one fixed region of the panel, repeatable to the millimetre, optical axis normal to the glass | Thorlabs SM1L10 tube, SM1RC clamp, TR3 post, PH3 holder, BA2 base | Thorlabs | 1 | 145 | 145 |
| 3 | Digitiser: two or more channels on one timebase, 12-bit or better, `digitiser_sample_interval_nanos` or finer sustained over `digitiser_record_length_samples` per channel, timebase accuracy at or under `timebase_accuracy_ppm`, inter-channel skew at or under `inter_channel_skew_nanos`, raw samples exportable with the instrument's own sample interval and origin | Rigol DHO924S, 12-bit, 4 channel | TEquipment | 1 | 1399 | 1399 |
| 4 | Panel under test: pixel response specified by its manufacturer and small beside the number being measured, refresh rate fixed, all panel-side processing and variable refresh disabled and recorded | LG 27GR95QE-B, 27-inch OLED, 240 Hz | B&H Photo | 1 | 799 | 799 |
| 5 | Keyboard whose switch contacts are electrically reachable without destroying it, and which is otherwise an ordinary input device | Keychron V4, hot-swap sockets | Keychron | 1 | 84 | 84 |
| 6 | Bounce-free contact closure across one key: solid state, no mechanical relay, isolated from the injector's supply | CPC1017N optocoupled MOSFET relay | Digi-Key | 5 | 3 | 15 |
| 7 | Injector controller, which is **not** the machine under test | Raspberry Pi Pico 2 | Digi-Key | 2 | 5 | 10 |
| 8 | Injector board: pull-up to a known level on the sensed contact, series protection, headers, wire | perfboard, 1% resistors, headers, hookup wire | Digi-Key | 1 | 35 | 35 |
| 9 | Coaxial leads, detector and sense line to digitiser, equal length | Pomona 2249-C-36, BNC–BNC, 1 m | Digi-Key | 3 | 21 | 63 |
| 10 | Test lead from the keyboard's switch contacts to a BNC channel | Pomona 3782-24-0, BNC to minigrabber | Digi-Key | 2 | 28 | 56 |
| 11 | Shroud excluding room light from the detector without touching the panel's surface | Thorlabs BK5 blackout fabric | Thorlabs | 1 | 45 | 45 |
| 12 | Regulated 5 V supply for the injector, separate from the machine under test | bench USB supply, 5 V 2 A | Digi-Key | 1 | 25 | 25 |
<!-- bom:end -->

<!-- bom:total 3205 -->

**Total: USD 3205**, list, on `prices_read_on`, before shipping, duty and local
tax. `verify.sh` re-adds the column rather than trusting this line, because a
total nothing re-adds is a number that survives the edit that invalidates it.

Three things are deliberately **not** in that total.

**The machine under test.** It is `E0-D10`'s purchase and
`claims/runner-class-A.md`'s specification, and buying it twice in two documents
is how a budget stops being one. The rig attaches to whatever machine the claim
is taken on.

**Shipping, duty and local tax.** They depend on where the order lands and this
file does not know. What can be said without knowing is the order of magnitude:
this is a few thousand dollars and not tens of thousands, which is the checkable
version of the design document's *the photodiode setup is inexpensive*.

**The injector's characterisation.** Rows 5 to 8 are the injector's *parts* and
are priced here because `E3-P01a` asks for every part. What its delay
distribution is, and how it is obtained, is `E3-P01b`'s exit and is a stated
hole — see *What this file does not answer*.

## The capture chain, and the error bar it fixes

This is `E3-P01c`. The chain is photodiode, amplifier, digitiser, and the thing
worth saying first is that **choosing the digitiser chooses the error bar**, so
the choice is made here knowing that rather than discovered afterwards.

### The chain

Light from one fixed region of the panel falls on the photodiode. The
photocurrent is converted and amplified by the detector's own transimpedance
stage, whose gain is set once and recorded; the requirement on it is a bandwidth
floor, not a gain, because bandwidth is what enters the arithmetic below. Its
output goes over coax to digitiser channel B. The sense line across the injected
key's contacts, pulled to a known level, goes over coax of the same length to
digitiser channel A. One record holds both.

The measurement is

```
input_to_photon_nanos = (index_b - index_a) * digitiser_sample_interval_nanos
```

where `index_a` is the sample at which channel A crosses its threshold and
`index_b` is the sample at which channel B crosses its own. Sample indices, in
one record, from one clock. Nothing subtracts a time from a time; the instrument
never states an absolute instant, so there is no absolute instant for anything
to be wrong about. The digitiser's trigger position does not enter, which is why
trigger jitter is absent from the budget below rather than bounded in it.

### The error bar, before any measurement

Five terms, each derived from the specification block above. Half-widths in
nanoseconds.

<!-- budget:begin -->
| Term | nanos | Derivation |
|---|---|---|
| `sample_quantisation_nanos` | 100 | `digitiser_sample_interval_nanos`. Two edges, each located to the nearest sample, each rounded independently; their difference is uncertain by one whole interval. |
| `threshold_crossing_drift_nanos` | 350 | `front_end_rise_time_nanos / amplitude_hold_denominator`, where `front_end_rise_time_nanos` is `rise_time_bandwidth_product_nanohertz / front_end_bandwidth_hertz` = 3500. |
| `inter_channel_skew_nanos` | 1 | `inter_channel_skew_nanos`, the requirement on row 3. |
| `timebase_error_nanos` | 1250 | `capture_window_nanos * timebase_accuracy_ppm / 1000000`. |
| `amplitude_noise_nanos` | 7 | `front_end_rise_time_nanos * front_end_noise_millivolts / optical_step_millivolts`. |
<!-- budget:end -->

<!-- budget:sum 1708 -->

Sum: **1708 ns**. Stated error bar: **`error_bar_half_width_nanos` = 2000 ns**,
which is ±2 µs — 1708 rounded **up** to the next whole microsecond, because an
error bar rounded down is an error bar that is wrong in the one direction that
flatters the claim.

Four things about that arithmetic that a reviewer will press, answered here
rather than in a reply.

**Why the terms are added rather than combined in quadrature.** Root sum square
is the usual choice and would give roughly 1300 ns instead of 1708. It is not
taken, for two reasons. The first is that it needs a square root, and RFC 0004
forbids floats in this tree including in a document the tree computes from; an
integer square root is available but is arithmetic a reader has to trust rather
than check by adding a column. The second is that quadrature assumes the terms
are independent and zero-mean, and `threshold_crossing_drift_nanos` is neither —
it is a bound on a systematic shift with illumination, not a noise. Worst-case
addition is conservative, it is checkable by addition, and an instrument error
bar that is too wide costs the claim a margin it does not need.
*Reversal:* if the budget ever becomes comparable to the quantity measured — say
above a tenth of it — quadrature with an integer square root earns its
complexity and this paragraph is where to argue for it.

**Why the front end's rise time enters as a tenth of itself.** A rise time is not
by itself an error: an edge that rises the same way on every shot puts a constant
offset in the number, and a constant offset is what calibration (`E3-P01e`)
removes. What is left is the part that varies. A fixed-level threshold on an edge
of rise `R` moves when the edge's *amplitude* moves, and the requirement
`amplitude_hold_denominator` is that the optical step varies by under a tenth
shot to shot — held by the fixture, the shroud, one fixed panel region and one
fixed test pattern. For a first-order edge a tenth of amplitude moves a
half-amplitude crossing by about `R/18`; `R/10` is taken instead and the
difference is left in as headroom. *Reversal:* if the assembled rig cannot hold a
tenth, this term is recomputed at the fraction it can hold and the total is
restated **before** a measurement, not after one.

**Why the timebase term uses the window and not the interval.** 25 ppm of the
14 ms the claim is about is 350 ns, and 25 ppm of the full 50 ms window is
1250 ns. The window is used so that the stated bound holds for anything the rig
reports up to `capture_window_nanos`, rather than only for intervals near the
number somebody hoped for. A bound that is only valid at the answer is not a
bound.

**Why the panel's own response is not in it.** Pixel response is part of *input
to photon*. It is the system under test, not the instrument, and putting it in
the instrument's error bar would be subtracting the measurement from itself.
This is why row 4 requires that the panel's response be specified and small, and
why it requires the panel's own processing and variable refresh to be disabled
and recorded: those are things the instrument cannot separate and the claim must
therefore hold fixed.

### What the error bar buys

`E3-P02` publishes a p99 bound of 14 ms, which is 14000000 ns. The instrument
resolves that to one part in

```
14000000 / 2000 = 7000
```

and one frame at 240 Hz — 4166666 ns — to one part in 2083. So the rig can
distinguish a sub-frame change in the number, which is the property that makes it
worth building rather than estimating. It cannot distinguish two rigs from each
other; that is `E3-P01e`'s job and is not claimed here.

## Assembling and checking

A stranger should be able to answer all of these before the rig records
anything. The list is here rather than only in a script for the reason
`runner-class-A.md` gives about its own checklist: several of these are
judgements about a physical object, and a checklist that is honest is worth more
now than a script that pretends to check a fixture.

1. One edge, split and fed to both digitiser channels, reads back with a measured
   skew at or under `inter_channel_skew_nanos`.
2. The detector's gain setting is recorded, and the bandwidth its datasheet gives
   for that setting is at or above `front_end_bandwidth_hertz`. A rig run at a
   slower setting is out of specification and its error bar is not the one above.
3. With the shroud in place and the panel showing the test pattern's dark state,
   the detector's output noise is at or under `front_end_noise_millivolts` RMS.
4. The optical step between the test pattern's dark and light states, at the
   detector output, is at or above `optical_step_millivolts` peak to peak.
5. Twenty consecutive optical steps vary in amplitude by less than one part in
   `amplitude_hold_denominator`. This is the requirement
   `threshold_crossing_drift_nanos` rests on and is the one most likely to fail
   in a room with a window.
6. The digitiser sustains `digitiser_record_length_samples` per channel at
   `digitiser_sample_interval_nanos` on both channels at once — not on one.
7. The panel's own processing, overdrive and variable refresh are off, and their
   settings are written down beside the number.
8. `timestamp-review.md`'s review has been re-run against the assembled rig's own
   capture toolchain and recorded, not assumed.
9. `./verify.sh` from a checkout is green.

Item 9 checks this document against itself. Items 1 to 8 check the rig against
this document, and nothing in this tree can do them.

## What this file does not answer

Three holes, each named with the task that owns it, because a document that
quietly fills a neighbouring task's exit is the defect
`intent/0012-the-interface/spec.md` calls one observation closing two tasks.

**`E3-P01b` — the injector's delay, as a distribution.** Rows 5 to 8 buy an
injector. What its delay is, and that it is characterised as a distribution
rather than quoted as a constant, is that task's exit and is not established
here. What this file does assert about it is the part that is a *design*
constraint rather than a measurement: channel A is taken **across the key's own
contacts**, not from a controller output that says it has commanded a press. Any
delay inside the controller is therefore outside the measured interval by
construction, and what `E3-P01b` has to characterise is what is left — contact
closure, keyboard scan, debounce and the device's own reporting — which is inside
it and is part of what a person waits for.

**`E3-P01e` — the known measurement, named before the rig is built.** That task's
exit requires the reference to be named in advance precisely so it cannot be
chosen afterwards from whatever the rig produced. Naming it here would be taking
that task's exit in a document written by somebody who has not thought about
which reference is right, and the wrong reference named early is worse than none.
It is a hole.

**`E3-P01f` — the method, for a third party.** That task's artefact carries the
bill of materials, the wiring, the procedure and the calibration result. Two of
those four are here and are meant to be reused rather than restated; the wiring
diagram and the procedure are not, and the calibration result cannot be.

## What would change this file

**The prices, first and most often.** Every figure in the bill of materials is
list price in United States dollars on `prices_read_on`, before shipping, duty
and local tax. Three separate decays act on it: a part is discontinued and its
successor costs differently; a list price moves; and the currency moves against
whichever one the order is actually placed in. None of them is visible to a
reader and all of them make the total a lie of a few hundred dollars.

So this file does not ask anybody to remember. `verify.sh` compares
`prices_read_on` against today and **fails** once the gap exceeds
`prices_stale_after_days`, naming the check to re-run. That is deliberately
annoying: a red check on a document nobody is editing is the only mechanism this
tree has found that survives six months of nobody looking, and the alternative —
a sentence saying *prices may be out of date* — is a sentence every stale
document already contains.

Re-reading the prices is an edit to the table and to `prices_read_on`, and it is
not an RFC: the totals are budget figures, not claims, and nothing renders from
them. Changing a **requirement** in the specification block is a different act,
because the error bar is computed from it and `E3-P02` is published against the
error bar. That one needs an RFC, and it needs the arithmetic re-run before any
measurement rather than after one.

**A finer digitiser.** `sample_quantisation_nanos` and `timebase_error_nanos` are
together 1350 of the 1708, and both are the digitiser's. A part with a 5 ppm
timebase would take the sum to roughly 708 ns and the stated bar to 1 µs. Worth
doing when the claim needs it, and not before: the rig already resolves the
published bound to one part in 7000.

**A second panel.** The bill of materials names one, and a number taken on it is
a number about that panel. A claim that wants to say something about the system
rather than about one display needs a second, at which point row 4 gains a second
worked example and `E3-P02` states which panel each number came from.

**`E3-P01e` landing.** Calibration is the first thing that can show this budget
is wrong — a rig that misses a known measurement by more than ±2 µs has a term
missing from the table above, and finding which one is the most useful thing that
could happen to this file.
