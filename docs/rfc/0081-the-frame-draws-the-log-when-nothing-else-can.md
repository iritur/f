# RFC 0081: The frame draws the log when nothing else can, and that is the fifth shared place

- Status: accepted
- Date: 2026-09-14
- Affects: `kernel/src/screen.rs` (new), `kernel/src/main.rs` (`open_screen`, one
  reservation, two provocations, `MAX_RESERVED` 12 to 13),
  `kernel/src/arch/x86_64/serial.rs` (the `kprint!` macro, which now tees),
  `xtask/src/main.rs` (`cargo xtask screen`), `intent/0011`'s constraint that no
  pixel path lives in the frame, RFC 0016's count of what crosses a core, and
  RFC 0076's refusal of a fifth shared word

## Decision

**The frame draws its own boot log on a firmware framebuffer, as a fixed-grid
character console, and the console's state is one shared object rather than a
`PerCpu<T>`.**

Two things are being decided and they are separable. The first is that a pixel
path exists inside the frame at all, which `intent/0011` wrote down as the thing
to avoid and named the condition under which it would be paid. The second is
that this console is the fifth place two cores meet, which RFC 0016 says needs
an argument and RFC 0076 refused for its own case four days ago. Both are
argued below and the second is the harder one.

What is *not* decided: nothing here is the presentation path. There is no
scene, no commit, no damage protocol on a ring, no client and no deadline. The
console renders one stream of bytes the frame was already printing to a serial
port. When a component can draw, this stops being the thing on the screen.

## Context

### Why the frame, when a component was the plan

`intent/0011` argues at length that a framebuffer console in ring 0 is the
wrong shape, and that argument is unchanged and still correct. It also names
the one case that would force the opposite, in advance rather than in
retrospect:

> It does not cover a hang inside core bring-up on a machine with no serial,
> which is precisely where the three VMware boots stalled. The fallback is a
> frame-side, fixed-grid, no-scroll writer, declared in advance with its unsafe
> cost. Expect to pay it at E5 and say so now.

The bill arrived earlier than E5 because the machine arrived earlier. The
machine F actually boots on outside the emulator is a VMware guest whose entire
output is a serial port written into a file on the host, and
`docs/booting-on-hardware.md` states what that means for anybody without one:

> with no serial port you will see a black screen and have no way to tell a
> clean halt from a crash.

Every boot outside QEMU so far has been read down that wire. The boots that
mattered most — the one that stalled inside core bring-up among them — produced
their evidence before anything above the frame was running, which is precisely
the window a component cannot cover. A console that only exists once components
exist is a console that is absent for every failure that has actually happened
on this project.

The alternatives were live and are recorded:

- **Wait for a component.** Correct in shape, and it answers none of the boots
  that have gone wrong. A component console cannot show a fault in
  `paging::build`, because there is no address space yet to spawn into.
- **Keep using serial.** This is what the request was to stop doing. It is also
  not available on a machine whose firmware offers no serial port at all, which
  is most machines built in the last decade.
- **A frame-side writer with no scrolling**, which `intent/0011` describes. It
  is smaller, and it shows the first forty-eight lines of a boot that prints
  two hundred. The end of a log is where a fault prints its registers, so a
  console that cannot reach the end is a console that misses the thing it was
  built for. Scrolling is two array shifts and costs nothing on the surface,
  because of the next paragraph.

### Why nothing is ever read back from the surface

The console this replaces — every framebuffer console, everywhere — scrolls by
reading the screen and writing it back one line higher. The mapping is
uncacheable, so every one of those reads is a bus transaction that stalls the
core, and it is the single reason a framebuffer console feels slow.

Here the text lives twice in ordinary cached memory: `want`, which is what the
log says, and `have`, which is what has actually been drawn. A scroll shifts
`want` and touches no pixel. The surface is brought level afterwards by writing
only the cells where the two differ, which on a log screen is far fewer than
all of them, because most of a log line is trailing blank and a scrolled blank
is still blank. **The surface is write-only**, and the rule is worth stating
plainly because the moment somebody reads it back for a scroll, the design is
gone.

### The font is not imported, and that is a licence decision rather than a taste one

`LICENSING.md` allows two things and no third: the permissive tree, every file
of which carries `Apache-2.0 OR MIT`, and `third_party/<name>/`, which carries
whatever its source requires and **may not be `use`d from the permissive tree at
all** — the only permitted coupling is a ring. There is no category for
*imported data under a permissive licence*, and a font is exactly that.

So an imported font is foreclosed here twice over. Terminus, the obvious
candidate and the one asked for, is SIL OFL — a licence that permits embedding
and would be perfectly comfortable in a component. It cannot go in
`kernel/src/` without a file in the permissive tree carrying a licence that is
not the permissive tree's, and it cannot go in `third_party/` and be reached,
because the frame cannot import from there. GNU Unifont, the other obvious
candidate, is GPL and would take the frame with it.

What is here is therefore original: five pixels by seven, ninety-five glyphs,
typed. That is a real cost — it is a plainer font than Terminus and nobody
would choose it on looks — and it is the cost of the boundary being where it
is. **The reversal is cheap and is named in the last section**: the day this
console moves into a component, that component may carry any font whose licence
allows it, and the licence boundary does not move an inch to allow it.

A table somebody typed is a table somebody mistyped, so it is checked by being
read back as shapes rather than by being trusted: `cargo xtask screen font`
prints all ninety-five glyphs to the serial port as characters.

### The fifth place, and RFC 0076's bar

RFC 0016 counts four words under `kernel/` that two cores reach — a mailbox and
three shootdown words — and says a fifth needs an argument. RFC 0076 was asked
for one four days ago, for a supervisor's pending state, and **refused**, on a
standard this entry has to clear rather than dodge: a fifth shared word is
*affordable only when something cannot be done without it*.

**First, which of the two rules this breaks, because they are different sizes.**
CLAUDE.md bundles them: every mutable `static` under `kernel/` is a `PerCpu<T>`,
and two cores reach one slot in exactly four places. If only the boot processor
ever wrote the console, this would break the first and not the second, and RFC
0076's bar would not apply at all.

It breaks both. Any core can print — a panic on core 3, a fault path, an
assertion in an arriving core's bring-up — and those are precisely the lines
this console exists to show. So it is a genuine fifth place and the whole bar
applies.

The console does not clear that bar by being important. It clears it by what
the alternatives actually are, and there are two rather than one.

**The naive conforming spelling is expensive.** `PerCpu<T>` costs
`MAX_CPUS * size_of::<T>()` whether the cores exist or not, and `MAX_CPUS` is
eight: a `PerCpu<Console>` is eight grids, about three hundred and twenty
kibibytes of `.bss`, seven of which must never be drawn from, because there is
one screen.

**The coherent conforming spelling is the one worth refusing properly, and it
is not incoherent at all.** Band the surface — one horizontal region per core,
each core writing only its own band. Nothing is shared, every index is one
core's, the lint passes, and for *per-core diagnostics* it is the right design
and somebody will eventually build it.

It is refused because of what a boot log is. A boot log is **one chronological
stream**: its whole value is that the line before a fault is the line that
preceded it in time, on whatever core. Banding replaces that with eight
independent streams a reader has to interleave by eye, and the case this
console exists for — a machine that stalls inside core bring-up with no serial
port — is exactly the case where the interesting lines are on two different
cores and their order is the finding. Banding costs the thing the feature is
for, which is a stronger reason to refuse it than cost, and the reason had
better be on this page rather than in somebody's head: a reader who thinks of
banding unaided and does not find it here concludes it was not considered.

So the conforming answers are, respectively, more expensive and destructive of
the ordering that makes the artefact worth having. That is the condition RFC
0076 names.

The two entries are also about different kinds of state, and this is the part
that keeps them consistent rather than merely compatible. RFC 0076's fifth word
would have been **read back, by the frame, to make a decision** — that is what
made a race there a correctness problem and what made the alternative worth its
cost. **Nothing reads this console.** No decision anywhere in the system
depends on its contents; no counter is derived from it; no test asserts on it.
It is written, and a human looks at it. A race produces a wrong character in a
cell, next to a serial log that is unaffected and remains the record.

And the precedent is already set by the device this sits beside. COM1 is one
device, written by whatever core is printing, with no lock in the path, since
M0. Every interleaved boot log this project has ever collected is that race
happening and being harmless. A screen is the same device with a different
shape, and holding it to a stricter rule than the port it duplicates would be a
rule about novelty rather than about sharing.

What the `unsafe impl Sync` rests on is narrower than any of that and is stated
at the code: every index into both arrays is bounds-checked or masked, so two
cores racing produce a wrong character and never a write outside the arrays.

### The two sinks are separate, and `cargo xtask trace` only hashes one

`trace` hashes the whole boot log and requires two runs of one commit to agree,
so anything that makes the *order in which cores print* observable in that
stream turns a real property into intermittent flakiness. A tee is exactly the
shape of change that could do it.

It does not, and the reason is structural rather than lucky. `Tee::write_str`
writes the serial port **first and unconditionally**, and the screen second.
Nothing in the console path can reorder, suppress or delay a serial byte: the
`DRAWING` flag gates only the second half, so a core that loses it has already
written its line to the wire. `trace` captures the serial stream and nothing
hashes the console — there is no way to read the console back to hash it, which
is the same property the rest of this entry rests on.

What does change is the log's *contents*: there are new lines in it, and they
are deterministic. Under the emulator the framebuffer is always absent, so the
line is always `framebuffer none` and `open_screen` always returns before it
can print a geometry. Confirmed rather than reasoned: `cargo xtask trace` on
this change reports `ok — 0x935fa6c7bf9befb9`, two boots agreeing.

**One thing found while confirming it, which is not this change's and is worth
somebody's time.** The first `trace` after a kernel edit failed with *the frame
this image measures is not the frame the generation declares*, and the second
passed unchanged. The image had been rebuilt and the generation record it is
measured against had not, so it would fire for any edit to the frame.

The reason it is worth writing down is that **the refusal is correct**. RFC 0012
makes that disagreement a refusal to publish a root rather than a warning line,
and `claims/0032` requires the refusal to happen exactly once across five boots
— so the check firing is the system working. What is wrong is only that two
artefacts were built in the wrong order, and a reader who trusts the message
goes looking for a measurement bug that is not there. Read the build before the
measurement.

### The drawing path is entered once, and that is about the panic handler

There is a second flag, `DRAWING`, and it is worth separating from the sharing
argument because it is not there for the second core. It is there because the
panic handler prints. A panic raised inside this file would call `kprintln!`,
which tees, which draws, which panics — unbounded recursion into the guard
page, destroying the message that would have said what went wrong. A core that
finds the flag set writes to the serial port and returns.

**It is not a lock and must not become one.** Nothing waits on it and nothing
retries; a core that loses skips its line. `CLAUDE.md` says nothing under
`kernel/` locks, and that rule is about waiting — a test-and-skip with no waiter
cannot deadlock, cannot invert a priority and cannot delay anybody. A `while`
around it would be a lock and would be a violation.

Its effect on the second core is a consequence rather than a purpose, and it is
an improvement: a core whose line arrives mid-draw is dropped from the screen
instead of interleaved into it, and its line is in the serial log either way.

## Consequences

- The `kprint!` macro tees. Every `kprintln!` already in the tree reaches the
  screen with no call site changed, which is the only way the memory map — all
  of it printed before there is a framebuffer — could ever appear on a display.
  A boot with no display pays one acquire load and a branch per string.
- **The screen starts late and does not pretend otherwise.** It begins at the
  line after the address-space switch, because that is the first moment there
  is somewhere to map a surface into. Everything before it is on the wire only.
  A console that replayed the earlier lines would be holding a copy of the log
  in order to lie about when it started.
- The surface is reserved from the frame allocator, and the boot reports
  whether it was inside a region the loader called usable. On every machine
  seen so far it is not — a display's memory is a device's — and the
  reservation changes nothing.
- **A machine where it *is* inside usable memory has a second problem this
  does not solve, and the boot says so rather than hiding it.** The reservation
  keeps the allocator off the range, but the direct map already covers usable
  memory write-back, so the surface would be mapped twice with different cache
  attributes — the alias `map_device`'s safety comment warns about, and one the
  hardware does not report. Nothing in this tree has met such a machine; the
  `inside a usable region` note exists so that the first one to meet it finds a
  line rather than a mystery. The repair, if it ever happens, is to punch the
  range out of the direct map, which is a change to `paging::build` and not to
  this file.
- The mapping is strongly uncacheable, one page at a time, because that is what
  the device window is. Seven hundred and sixty-eight pages for a 1024 by 768
  surface, paid once, plus two or three frames of page table.
- `MAX_RESERVED` goes from twelve to thirteen.
- A failure anywhere in `open_screen` prints a line and the boot continues. A
  machine that boots without a picture is strictly better than one that refuses
  to boot because it could not draw, and the serial port is unaffected by
  anything that can go wrong in there.
- `cargo xtask screen` is a new verb with two checks, and it exists because the
  real path **cannot be executed under this emulator at all**: QEMU's `-kernel`
  loader implements no part of the multiboot video request, so `open_screen`
  returns on its first line for every boot the harness can start. What the verb
  covers is the font and the blit. What it does not cover, and this is stated
  rather than left to be assumed, is the handoff itself — that a loader fills
  the fields in, that the extent maps, and that a display shows what was
  written. That needs GRUB, which needs a machine that is not this emulator.

## What would reverse this

- **A component that can draw before the frame needs to.** That is the whole
  design and this is the stand-in for it. The day a display component can be
  running at the address-space switch, this file's reason for existing is gone,
  and with it the fifth shared place and the hand-typed font — a component may
  carry Terminus, or any font whose licence permits embedding, without the
  licence boundary moving.
- **A second reader of the console's state.** The argument above rests entirely
  on nothing reading it back. The first piece of code that makes a decision
  from `want`, `have` or the cursor invalidates that argument and re-opens RFC
  0076's refusal against this entry.
- **A measurement showing the uncacheable mapping dominating boot time.** The
  repair is write-combining, which means programming `IA32_PAT` on every core —
  a change to a global processor register for the benefit of a fallback
  console, which should be bought with a number rather than assumed. It is the
  named next change and deliberately not made here.
- **A machine where the console is wrong rather than plain.** The font is five
  by seven because it was typed rather than imported; the grid clips at 160 by
  64. Either becoming a real limit on a real display argues for the component,
  not for a bigger table in the frame.
