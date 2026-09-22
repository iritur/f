# No software timestamp in the measurement path

`E3-P01d`. Its exit: *the path from injection to capture is written out and
every time in it is the instrument's; one software timestamp found in it fails
this task, and the review that looked is recorded rather than assumed.*

Two halves, and they fail differently. Writing the path out is a design act and
is done once. Looking is an act somebody performs on a particular day against a
particular set of files, and a review nobody recorded is indistinguishable six
months later from a review nobody did. So this file is both: the path, then the
review of it, with its date, its commands, its output and what it could not
reach.

**This is a review of the specified path.** The rig does not exist; nothing in
this tree today fetches a trace from a digitiser. Calling that a completed review
of the rig would be exactly the failure `docs/TECHNICAL-DEBT.md` was written to
prevent — a clause quietly reinterpreted into one that could be met from here. So
the review below states, per step, whether it was *checked* or is *owed to the
assembled rig*, and the owed half is printed on every green run of `verify.sh`
rather than mentioned once here.

## The path, written out

Eleven steps from arming the instrument to a number. The column that matters is
the last one: whose clock names this instant.

| # | What happens | In the measured interval | Whose clock names it |
|---|---|---|---|
| 1 | The operator arms the digitiser: both channels, one record, `capture_window_nanos` long, triggered on channel A's edge | no | none — arming takes no time in the number |
| 2 | The injector controller decides to fire | no | none, and deliberately: see *Why the controller's clock is not in it* |
| 3 | The optocoupled relay closes across key K's contacts | yes, at its leading edge | — |
| 4 | **Channel A crosses its threshold.** The sense line, pulled to a known level across K's contacts, falls | this is `index_a` | **the digitiser's** |
| 5 | The keyboard scans the matrix, debounces, forms a report | yes | no clock is read |
| 6 | The report crosses USB to the machine under test; F's driver takes it at interrupt | yes | `input/src/stamp.rs` reads `env.now()` here — **and that value is not in this number**; see below |
| 7 | F's input path, compositor and late-latch produce a frame | yes | no clock in this number |
| 8 | The frame is scanned out | yes | none |
| 9 | The panel's pixels change state | yes | none |
| 10 | **Channel B crosses its threshold.** The detector's output rises past the fixed level | this is `index_b` | **the digitiser's**, the same clock and the same record as step 4 |
| 11 | The host fetches the record over LAN and writes it to disk | no — after the fact | the host has a clock and **must not be allowed to put it in the number**; this is the step the review reaches into |

The number is

```
input_to_photon_nanos = (index_b - index_a) * digitiser_sample_interval_nanos
```

Two sample indices out of one record, times an interval the instrument reports
for that record. There are exactly two times in the whole path and both are step
4's and step 10's, both are the digitiser's, and neither is ever expressed as an
absolute instant. Nothing subtracts a wall-clock time from a wall-clock time
anywhere, so there is no instant for anything to disagree about.

The interval so formed carries `README.md`'s error bar, ±2 µs, and no other
uncertainty — which is a statement this file is entitled to make only because
every step above either contributes no time or contributes one of those two.
A software timestamp anywhere in the path would be a term the budget does not
contain, and the two documents would then be wrong together rather than one of
them being wrong alone. `verify.sh` compares the two copies of that number for
exactly that reason.

### Why the controller's clock is not in it

Step 2 is where a rig usually goes wrong. The obvious design takes channel A
from a controller output — a GPIO raised in the same instruction that commands
the relay — and then the controller's own scheduling, its interrupt latency and
its output driver are all inside the measured interval, silently, and the rig
reports them as the machine's latency.

Channel A is taken **across the key's own contacts** instead. Everything inside
the controller is then before `index_a` and outside the number by construction,
which is a property of where the probe is clipped and not of anybody's care.

What is left inside the interval is steps 5 and 6 — contact closure, matrix scan,
debounce, and the device's own reporting over USB. That is real input latency and
belongs in the number, and characterising it as a distribution is `E3-P01b`'s
exit. **It is a stated hole here**, not a finding: this review establishes that
nothing in steps 5 and 6 contributes a *timestamp*, not that their delay is
known.

### Why F's own stamp at step 6 is not in it

`input/src/stamp.rs` reads a clock exactly once, at the driver, at interrupt
time. That is `E3-B04a`'s rule and `cargo xtask lint-stamp` enforces it across
four crates. The reading is real and the number derived from it —
`presented_at - stamped_at` — is a useful number.

It is a **different** number. F's stamp begins at step 6 and ends at step 7; this
measurement begins at step 4 and ends at step 10. F cannot see steps 3 to 5 at
all, and it cannot see steps 8 to 10, and those are between a third and a half of
what a person waits for.

So the review's finding about step 6 is not *no clock is read there* — one is,
and it should be. The finding is that **no value F computes reaches this
number**, because the number is computed entirely from one digitiser record that
F never touches. That is the strongest form the statement can take: not a rule
somebody follows, but a data flow with no edge in it.

The failure mode this leaves open is not a timestamp in the path. It is
**publishing F's number under the rig's name** — reporting `presented_at -
stamped_at` as *input to photon* because both are latencies of an input. That is
a labelling defect, it is invisible in any grep, and the only defence is that
`E3-P02` names this file and this file says which of the two it is. Recorded here
because a review that lists only what it can grep for teaches the next reader
that the greppable set is the whole set.

## The review

**Performed:** 2026-09-21, against commit `fba7b46`, on the whole of this
checkout except `.claude/`, by the session that wrote this file. Recorded rather
than asserted: every command below is reproducible and `verify.sh` re-runs the
two that can be re-run from a checkout.

### What was looked for

A software timestamp is any value read from a clock that is not the digitiser's
and that could reach `input_to_photon_nanos`. The needle set:

```
env\.now\(|Instant::now|SystemTime::now|rdtsc|clock_gettime|gettimeofday|
QueryPerformanceCounter|time\.time\(|time\.monotonic|perf_counter|
datetime\.now|Date\.now
```

Three families, on purpose. The Rust ones are what this tree could read. The
POSIX and Windows ones are what a vendor's capture library reads underneath. The
Python ones are what a capture script will be written in, because every
oscilloscope vendor's example program is Python and every one of them prints the
time it saved the file.

### Finding 1 — the needle set fires. Control, not decoration

A review whose search finds nothing proves nothing until the search is shown to
find something. `input/src/stamp.rs` is the one file in this tree that is
*supposed* to read a clock, so it is the control:

```
$ grep -nE '<the needle set>' input/src/stamp.rs
46://! `from_wire_nanos(env.now().as_nanos())` is `cargo xtask lint-stamp`, which
190:    StampNanos { nanos: env.now().as_nanos() }
231:            assert_eq!(at_interrupt(&env).nanos(), env.now().as_nanos());
```

Three hits, one of them the reading itself at line 190. The set is live.
`verify.sh` runs this control on every invocation and **fails if it returns
nothing**, because the day somebody edits the needle list into uselessness is the
day this whole review becomes a green check over an empty search.

### Finding 2 — there is no capture toolchain in this tree to review

```
$ grep -rlnE 'SCPI|WAVeform|pyvisa|usbtmc' --exclude-dir=.claude .
$
```

Nothing. No file in this checkout talks to a digitiser, which is what *the rig
does not exist* looks like when it is checked rather than assumed. This is the
half of `E3-P01d` that is **owed to the assembled rig**, and it is the half the
failure the task exists to catch actually lives in.

### Finding 3 — the failure to catch is host-side stamping, and it is designed out rather than looked for

The digitiser choice in `README.md` row 3 is where this review actually bites,
and it is the reason that row says *raw samples exportable with the instrument's
own sample interval and origin* rather than just naming a bandwidth.

There are two classes of digitiser and they fail differently.

A **PC-hosted digitiser** — a USB DAQ or PC oscilloscope — streams samples into
host memory and the driver knows when each buffer arrived. Every such product's
API offers that arrival time, usually in the same structure as the samples, and a
capture script that reaches for the obvious field gets the host's clock. The
timestamp is not *added* by anybody; it is *there*, one field away from the data,
and nothing warns you. That class is **excluded by row 3**, and this paragraph is
the reason it is excluded — recorded as a design decision so that a later reader
choosing a cheaper part knows what they are giving up.

A **standalone instrument** samples on its own timebase and answers a query.
Its export carries the sample interval and the record's origin as instrument
quantities; the host supplies none of them. The host's clock is then genuinely
absent from the data rather than merely unused.

Two traps remain even there, and both are owed to the assembled rig:

- **The vendor's own export writes a header.** A CSV saved by an instrument's PC
  application typically carries a capture-time or save-time field taken from the
  PC, beside fields that are the instrument's. It must be **named and discarded
  by name** in whatever reads the file, not merely left unread — a later script
  that does `columns[0]` picks it up.
- **The vendor's application may re-time on fetch.** Some bench software
  recomputes a time axis on the host. The capture toolchain must fetch raw
  samples and the instrument's own sample interval, and derive nothing from the
  host.

### Finding 4 — one clock reading in F's input path, and it is outside this number

Established in *Why F's own stamp at step 6 is not in it* above. The mechanism
that keeps it true as the tree changes is `cargo xtask lint-stamp`, which refuses
a second clock reading anywhere on the input path and has a fixture that makes it
fail. This review did not re-run it; it belongs to `cargo xtask verify` and to
the coordinating session, and citing it is honest where re-running it here would
be a second copy of somebody else's check.

### Finding 5 — the scan was watched catching the failure it is for

Findings 1 and 2 are a search that fired on a control and a search that found
nothing. Neither of those shows that the scan would catch **the** failure — a
digitiser whose host software stamps the trace on arrival — because no such file
was in front of it. So one was written and put in front of it, in a scratch
directory outside this tree:

```python
import time

def fetch(scope):
    raw = scope.query_binary_values(":WAVeform:DATA?")
    return {"samples": raw, "captured_at": time.time()}
```

That is the whole defect in five lines and it is what a real capture script
looks like on the day somebody adds one field. `F_RIG_CAPTURE` pointed at its
directory:

```
BAD   no software clock in the capture toolchain  .../fake-capture
      .../fake-capture/capture.py:5:    return {"samples": raw, "captured_at": time.time()}
      E3-P01d fails on one. Either the reading is outside the number and the
      path in timestamp-review.md must say where, or the rig is measuring the
      host's clock and calling it the instrument's.
```

Exit 1, the file and the line named. Recorded because the standard this epoch
has settled on is that a guard nobody has watched kill something is decoration,
and four review rounds have been spent removing decoration. This one has been
watched.

It bounds what the scan is, too. It reads text for a needle. It cannot see a
timestamp arriving through a compiled vendor library that spells none of the
needles, and it cannot tell a clock reading in the number from a clock reading
in a log line. Those are read by a person, against the path above, which is why
step 11 has a sentence rather than a check.

### Verdict

For the path as specified: **no software timestamp**, on all eleven steps, with
the two instrument times at steps 4 and 10 and nothing else.

For the path as built: **not reviewed, because it is not built.** Finding 2 is
what that looks like when it is checked.

## When this is re-run

On the assembled rig, before the first measurement is recorded, over the capture
toolchain as it then exists — and the result appended to this file with its own
date, not substituted for this one. Item 8 of `README.md`'s assembly checklist is
that requirement stated where somebody assembling the rig will read it.

Point `verify.sh` at the toolchain with

```
F_RIG_CAPTURE=/path/to/capture ./verify.sh
```

and it applies the needle set to it. Until that variable names a directory, the
script prints the owed line and counts it as owed rather than as passed. A check
that reports *nothing found* over a directory that does not exist is the same
green as a check over a clean one, and telling them apart is the only thing
standing between this file and a review of a drawing.

## What would reverse this

**A device that stamps its own events.** `input/src/stamp.rs` names `E5-B06`,
hardware-timestamped native input, as the day its one clock reading moves out of
this tree and into the device. If that device's stamp is on the *same* timebase
as the digitiser — a shared reference clock, not a shared notion of time — then
step 6 could supply an instrument time and the rig would have a third honest
instant. That is a real improvement and it is also the single most dangerous
change to this path, because it makes a software-supplied number look like an
instrument one. It needs an RFC and it needs this review re-run from step 1.

**A claim that needs an absolute instant.** Everything above rests on the number
being a difference of two indices in one record. A claim correlating the rig
against something outside the record — a second instrument, a network capture —
needs a common time and reopens every question this file closes.
