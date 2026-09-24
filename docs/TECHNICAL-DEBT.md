# Technical debt: what this tree cannot answer from here

Every row on this page is a sentence that was once an exit criterion in
`TODO.md` and is now owed. RFC 0093 is the decision that put them here: epochs
E0 through E2 close against what a virtual machine can honestly establish, and
a clause naming something a virtual machine may not answer for is **moved here
rather than deleted**.

Moved rather than deleted is the whole of it. A criterion quietly dropped is
indistinguishable, six months later, from a criterion that was met — which is
what `A-07` exists to forbid. So the clause is stored verbatim, beside the
machine or the person that would settle it, and the task that lost it says in
its own body that it was narrowed and cites RFC 0093.

## How to read this

A row is **not** a to-do. Nothing on this page can be worked on with a commit;
if it could, it would be a task in `TODO.md` instead, and putting it here would
be the scope cut this page exists to prevent. What each row is for is a reader
asking *what does this project not know about itself*, and getting a straight
answer with the reason attached.

The four things a virtual machine may not answer for, from RFC 0093:

1. **A time.** `bench/src/lib.rs` refuses to record a measurement outside a
   recording environment, and that refusal is the harness working.
2. **A reservation obtained by partition.** All four of RFC 0007's components
   by partition needs Intel RDT's CAT and MBA; no guest exposes a resctrl
   partition.
3. **An instruction the emulator does not implement.** QEMU's TCG backend
   implements no part of Intel UINTR and no `-cpu` model advertises the bit.
4. **A second machine, or a second person.** One machine twice is not two
   machines, and a stranger reproducing a claim is an observation about
   somebody else's afternoon.

A clause that is none of those four does not belong on this page, and a
reviewer can say so without knowing the subsystem. That is the property the
list is for.

## What is deliberately *not* here

**`E2-B02` and `E2-P10`**, and this entry is kept on the page it does not
belong to because **it was wrong and the correction is worth more than the
row**.

It used to read: *zoned virtio-blk emulation needs QEMU 8, and the tuned-Linux
zoned baseline needs a directory that does not exist yet. Both are real blockers
and neither is a machine: a development image is a change to
`docker/Dockerfile`. Work, not debt.*

Both halves have been settled by looking rather than by reasoning, and they went
opposite ways.

The baseline directory **exists** — `claims/baselines/linux-6.x-tuned-zoned/`,
nine files, with the workload dials cross-checked against `zone/tests/cycle.rs`
by its own `verify.sh`. That sentence was stale, and `E2-P10`'s body said the
same thing for as long.

The image change **was made** — `docker/Dockerfile`'s base is trixie and QEMU is
10.0 — and it is not what the exit was waiting for. QEMU has no zoned emulation
for virtio-blk in any version: `-device virtio-blk-pci,help` lists eighty
properties and not one is zone-related, and `zoned=on` on a file-backed blockdev
is refused as unexpected. What QEMU's zoned virtio-blk support *is* is
passthrough of a **host zoned block device**. What it synthesises from a plain
file is zoned NVMe, which this tree has no driver for and which would be an
epoch of work to acquire one for.

So the blocker is a host that can present a zoned block device. That is not
special hardware — `modprobe null_blk zoned=1 zone_size=64 nr_zones=48
zone_nr_conv=4 blocksize=4096` matches `device.conf` exactly — and it is
therefore still **work rather than debt**, which is why there is no row for it
below. It is work that cannot be done from *this* host: the kernel under Docker
Desktop here is WSL2's, and it reports `CONFIG_BLK_DEV_ZONED=y` with
`CONFIG_BLK_DEV_NULL_BLK` and `CONFIG_BLK_DEV_ZONED_LOOP` both unset, so there
is no module to load and the container is not privileged either way.

**Why that distinction is worth a paragraph rather than a row.** RFC 0093's four
categories are what a *virtual machine* may not answer for, and a zoned device is
not one of them: any ordinary Linux host or CI runner settles it, with no
hardware anybody has to buy. A row here would say this project cannot know
something about itself, and that would be the scope cut this page exists to
prevent. What is true is narrower and belongs in `TODO.md`: `E2-B02` closes on a
machine that is not this one, and the list of what to run there is written down
rather than left to be rediscovered.

**`claims/0022 copies-per-read`.** It is `pending`, and its `[hardware] runner`
says `runner-class-A` like every other pending claim — but its own note says
the machine it needs *is not a runner class*, it is a boot of `user/objects`
over the blk driver's ring. A task in this tree. It stays off this page, and
the `[hardware]` field that says otherwise is a defect in the registry rather
than a reason to record a debt nobody owes.

## The blockers

Six things stand behind every row below, and two more stand behind E3 and
behind no row at all. They are here because a blocker a reader cannot see is a
blocker nobody has costed. Why they carry no row is RFC 0109 and the section
after the register: a row is the receipt for a clause a closing task gave up,
and every E3 line these two block is `[ ]`.

### `runner-class-A` — a machine nobody has bought

`claims/runner-class-A.md` is the specification: required capabilities, the
firmware settings that cannot be undone from an operating system, a kernel
command line, a recipe per reservation component, and a seven-item checklist a
stranger runs before setting `F_ENVIRONMENT`.

What makes it class-A is **all four reservation components by partition**. RFC
0007 permits partitioning by exclusion where the hardware cannot partition, and
that stays a legitimate reservation — it is just not this one. That single
requirement rules out every desktop part: Intel dropped CAT from client silicon
after Skylake-X.

Fifteen claims are `pending` on it. `E0-D10` owns it and is explicit that the
machine half *"is a purchase order and not a commit"*.

### UINTR silicon — an instruction that has never run

`ring/src/doorbell.rs` builds `Path::UserInterrupt` to the point of refusing to
construct, and no further. `Bell::new` returns `Err(Path::UserInterrupt)`
rather than downgrading, because a channel negotiated into an instruction that
faults is worse than a channel that refused. The refusal is the whole of what
can be exercised here.

### Bare metal — six attempts, three of them `M0 ok`, none of them bare metal

`docs/first-`, `second-` and `third-boot-outside-qemu.md` record three boots to
`M0 ok` outside the emulator, carrying one user process, then four components
and a supervisor, then E2’s state trees. **All three were VMware guests**, so
`E0-P18`’s *“a machine that is not an emulator”* is met on the reading the task
can guarantee, and its stronger reading is not.

`E0-P18` is also the one item here that is **not** waiting on a purchase, and
its own body insists on the distinction: `E0-P05` and `E0-P06` need class-A
silicon because they need RDT partitioning, while this needs an x86-64 machine
with a serial port and Secure Boot off — a machine somebody may already own. It
is the cheapest unblocked item in the epoch and the only one whose result
nobody can predict.

**The three attempts that failed are the argument that these boots are not a
formality.** Attempts four, five and six ran on a VMware guest over a
Threadripper host and none reached `M0 ok`. Between them they produced two real
defects that QEMU had been hiding for the life of the project:

- The trampoline handed an arriving core the boot processor’s whole
  `IA32_EFER`, `LMA` included — a read-only status bit written as 1 into a core
  in thirty-two-bit protected mode. Some processors ignore it; some raise
  `#GP`, and a `#GP` in the trampoline is a triple fault. RFC 0070.
- `IA32_STAR`’s ring-3 field was written as `0x28`, and `sysret` on AMD loads
  that field plus eight exactly as written — `0x30`, a stack selector
  requesting ring 0 — where Intel, and QEMU under every `-cpu`, force the
  privilege bits and had been supplying them for free since the field was first
  written. RFC 0074.

Both are fixed in the tree. **Neither has been demonstrated on the machine that
found them**, and that is the sharpest single row on this page: a defect
diagnosed from a log, repaired against an emulator that could never have shown
it, and not yet re-run where it failed.

### A second machine, and a second date

`cargo xtask generation --elsewhere` separates the checkout path out of the
bundle a second machine differs by, and says nothing about the rest of it —
core count, host load, filesystem, kernel, uid. `lint-remap` prints exactly
that limitation on every green run. A second date is a second run of one commit
a week later, which on a moving branch is the exception rather than the rule.

### `photodiode-rig` — an instrument nobody has bought

`claims/photodiode-rig/` is the specification, in the shape
`claims/runner-class-A.md` uses for a machine: twelve parts with a source and a
price each and a stated total, an error bar of **±2 µs** derived from a sample
interval and a front end's rise time *before any part exists*, an eleven-step
measurement path with a *whose clock* column and no software timestamp in it,
and a `verify.sh` that re-adds every number and goes red on its own when the
prices are 180 days old.

Gate G3 is a time — input to photon at p99, taken by a photodiode, under load —
so this instrument stands behind `E3-P02` and, through it, behind `E3-R01` and
release 0.4.

**Three of `E3-P01`'s six subtasks close on the near side of it**, and that is
the interesting thing about this blocker rather than an aside. `E3-P01a` is a
bill of materials, `E3-P01c` asks for the instrument's own error bar *stated as
an integer before any measurement is taken with it*, and `E3-P01d` asks for the
path written out and the review that looked recorded. None of the three needs a
part. `E3-P01b`, `E3-P01e` and `E3-P01f` do: a distribution is a set of
measurements of a real actuator, a calibration is a reproduction of a known
number, and a document containing a calibration result contains one.

It is **not** one of RFC 0093's four categories. It is a purchase, like
`E0-D10`, and `E0-D10` is already on this page — which is the precedent for
treating a purchase as a blocker and the reason this section says *six* rather
than *five and a half*.

**One thing a reader should not have to discover.** The prices are budget
figures compiled from catalogue list prices for the worked examples, and not one
of them was read from a live vendor catalogue — the session that wrote the file
had no way to reach one. `claims/photodiode-rig/README.md` says so in its own
words under *Hardware*, says what the total is good for (deciding whether to buy
the rig) and what it is not good for (a purchase order), and `verify.sh` carries
a staleness check so the figures cannot quietly become a quotation.

### A stranger

Three exits name a person who has not read this tree doing something and
timing it. No commit can be that person.

### A GPU — a machine `E5-D01` names and nobody owns

`E3-B02`'s ladder has four rungs and this tree can observe one stage of the top
one. `E3-B02c` closed on 2026-09-23 saying so in its own title — *the only stage
of rung 1 observable without a GPU* — and the three stages after it, the two
lower rungs that must match rung 1's image, and `claims/0033`'s four numbers all
want compute shaders that no machine here reports.

**What stands where `claims/runner-class-A.md` stands for the other machine is
`E5-D01`, and it is not the same kind of thing.** That line is *name the
machine*, and its exit is *a bill of materials anyone can buy* — a document. It
has no second clause saying the machine exists, so it has nothing to narrow and
no row, and six E3 lines name it in `needs:` as though closing it would unblock
them. `E0-D10` is the same shape handled correctly: the specification half is met
and the **machine half** is the row below. Until `E5-D01` grows that half, the
honest statement is that six E3 lines wait on a purchase nobody has specified and
the graph says they wait on a file.

It is **not** one of RFC 0093's four categories. It is a purchase, like `E0-D10`
and like the rig, and the precedent for treating a purchase as a blocker is
already on this page.

### An energy meter — an instrument with no line in the file

`E3-P07` is *energy per frame, external meter rather than a model*, and **nothing
in `TODO.md` acquires a meter**. Its `needs:` names `E3-B01`, `E0-D10` and
`E1-D06`; not one of the three is an instrument. There is no bill of materials,
no `claims/` directory in the shape `claims/photodiode-rig/` uses, no error bar
stated before a part exists, and no task whose exit is *a meter*.

Four exits depend on it — `E3-P07`, `E5-P03`, `E5-P04` and E7's watts-per-token
line — and `bench/src/lib.rs` carries `joules_per_op` as `Metric::Unavailable`
with RFC 0006 naming `E5-B07` as the day it stops being. `E0-D10` and `E3-P01a`
both exist because a purchase with no specification is a blocker nobody has
costed. This is that, un-prevented, and naming it is the only thing this page can
do about it.

**The half that could be had from here is the one the rig already has.**
`E3-P01a` is a bill of materials and `E3-P01c` is an error bar *stated as an
integer before any measurement is taken* — neither needs a part. Nobody has
written either for the meter and no line asks for them.

## The register

Each row: the clause as it stood, the task it came out of, the blocker, what a
virtual machine did establish in its place, and what would close it.

<!-- Rows are added as tasks are narrowed. `cargo xtask lint-debt` is what
     keeps this table and the registry from drifting apart. -->

| Task | Clause narrowed out of its exit | Blocker | What the VM established instead | What closes it |
|---|---|---|---|---|
| `E2-B04` | *“two machines”* — the generation root compared across two hosts rather than two checkout paths on one | a second machine | The path-dependence class, which is the one that bit: before `-Zremap-cwd-prefix` landed, one commit at two checkout paths gave two frame leaves and two roots. `claims/0028` is that comparison, gating. | A second host running the weekly job’s `root` matrix. |
| `E2-P06` | *“identical generation root hash, checked weekly, across two machines and two dates”* | a second machine, and a second date | A non-reproducible input fails the job and **names itself** — `generation --mutate` arms a defect, the comparison goes red, and the harness requires the `kernel` leaf by name rather than any red. | Two runners, and two weeks landing on one commit. `lint-remap` prints both limits on every green run. |
| `E0-B15` | *“both paths pass the same suppression test”* in the sense of the second having rung, and *doorbells per operation under load* | UINTR silicon, and `runner-class-A` | The kernel-IPI path delivered and counted at boot; one suppression test driven over all three paths; a batch asserted to be one operation and at most one doorbell; the user-interrupt path refusing to construct rather than downgrading. | The part for the first clause — it shares `E1-B09`’s row. For the second, a machine where two cores contend under a real workload. |
| `E2-P09` | *“ordinary run-to-run noise over the same period is not”* detected — as a claim about **this system’s** noise | `runner-class-A` | The detector, and that it does not fire on the noise model its tests state: seven tests including a step deliberately placed inside wider scatter and not reported. | A gating claim that has run repeatedly on that machine, so there is a real scatter to check the bar against. Begins at `E0-P06`. |
| `E1-P10` | *“ring submit under load”* and *“doorbells per operation”* — two of its four claims | `runner-class-A` | For the first, nothing: the workload is `E0-B12`’s and refuses to record here. For the second, the kernel-IPI path delivered and counted at boot — but *500 per 1000 operations* out of a two-operation self-test is an artefact of the sample, and `E0-B15` declined to register it for that reason. | A machine where two cores contend under a real workload. **The other two are counts and are not on this page**: `claims/0036 copies-per-operation` gates today, and kernel entries per operation is buildable here. |
| `E1-B09` | *“both paths measured; the notification-cost claim is recorded with the hardware named”* — the whole exit | UINTR silicon | The path is built to the point of refusing to construct: `Bell::new` returns `Err(Path::UserInterrupt)` rather than downgrading. The kernel-IPI path is delivered and demonstrated at boot. | A part implementing Intel UINTR, under KVM rather than TCG. Then *both paths measured* becomes a number rather than an instruction nobody has issued. |
| `E1-B10` | *“the cost, measured”* — `claims/0004` moving off `pending` | `runner-class-A` | Both paths pass the same ownership tests; the workload runs; the thresholds are written **before** any number exists, which is the half of a claim that can be made from here. | The same machine as 0001 and 0002. |
| `E1-R01` | *“a third party runs it”* | a person | The command exists, is documented where a stranger would look, and produces a seed sweep from a clean checkout. | Somebody who is not this project running it. The long plan calls this shape `G4` and puts it deliberately outside the team’s control. |
| `E0-D10` | *“the machine half”* — a machine meeting `claims/runner-class-A.md` exists and can be run on | purchase | A 209-line specification: capabilities with one worked example rather than a part number that ages out, the firmware settings that cannot be undone from an operating system, a kernel command line, a recipe per reservation component, and a seven-item checklist a stranger runs before setting `F_ENVIRONMENT`. | Somebody buying the machine and the checklist passing on it. Until then `F_ENVIRONMENT=runner-class-A` is an assertion no code in this tree can tell from a lie. |
| `E0-P18` | *“bare metal”* — a boot on a machine that is not a guest | real hardware | Three boots to `M0 ok` outside QEMU carrying one process, then four components and a supervisor, then E2’s state trees; and three failures on an AMD host that found RFC 0070 and RFC 0074, both of which QEMU could never have shown. | An x86-64 machine with a serial port and Secure Boot off — **not** a purchase, and not class-A silicon. The cheapest unblocked item in the epoch and the only one whose result nobody can predict. |
| `E0-R02` | *“a person who has never seen the repository reproduces claim 0001 from the README in under thirty minutes, including toolchain install”* | a person | `cargo xtask reproduce <claim>` dispatches over the registry rather than reimplementing per claim; `lint-reproduce` asserts the command exists, is that claim’s own, routes to something that runs, and names a runner class with a specification beside it. A README section that is the whole route. | A named person who has not read this tree running it on a machine of their own, with the wall-clock time and where it went recorded. |
| `E0-B12` | *“Under 50 ns per operation, recorded as a gating claim”* | `runner-class-A` | One million NOPs in batches of thirty-two through the batch path, and the distribution drawn; the kernel proves the layout on every boot, adopting a header out of the bytes and reporting a forged slot number as corruption. | The same number `E0-P05` owes. One machine closes both, and this row goes with that one. |
| `E0-P05` | *“`cargo xtask claims` reports it green; a deliberate 20% regression fails the build”* — the whole exit | `runner-class-A` | The workload runs to completion and the distribution is drawn; `bench` refuses to record it, naming the environment. `claims/0001` is `pending`. | A class-A machine passing the seven-item checklist, F booted on it (`E0-P18`), and `cargo xtask claim ring-submit-latency` recording a number. |
| `E2-B02` | *“on a zoned device or its emulation”* — the fill-and-collect cycle taken against a device that is really zoned rather than a model of one in the host | a host that can present a zoned block device | The whole cycle, through the same `ZonedDevice` trait a driver will implement: fill with `ZONE_APPEND`, seal with `ZONE_FINISH`, sweep below `SWEEP_LIVE_FRACTION`, copy forward, reset — 10 of 20 data zones reset, nine kept generations read back and verified, and both refusal controls exercised. `device_bytes_per_app_byte` = 1.1336, registered as `claims/0016` with `emulated = true` and its rows named `modelled_*` so the figure cannot be read as the claim's own. | A host with `null_blk zoned=1` — **not a purchase**; any ordinary Linux kernel builds the module, and this one has `CONFIG_BLK_DEV_NULL_BLK` and `CONFIG_BLK_DEV_ZONED_LOOP` unset. Then a privileged container, the `query-blockstats` reader xtask does not have, and the `--baseline` argument it does not parse. |
| `E2-B08` | *“copies per read is zero, counted rather than asserted”* — counted across a device that is really zoned, rather than one this component builds in its own heap | the same host `E2-B02` waits for | The zero, twice over and at the boundary the spec names: 0 over 256 reads with two tallies on opposite sides, each provoked non-zero in the same run, and a client `READ` crossing a real objects ring at `cargo xtask objects read` with 192 000 application bytes delivered. Resident pages taken by the frame rather than asked of the component — `state::node::OBJECTS_RESIDENT`, 58 frames — which is `claims/0019`'s own definition readable for the first time. | The same host as `E2-B02`. The ring half and the residency half are both paid; what is modelled is the device under the store. |
| `E2-P10` | the whole exit — *“bytes written to the device per byte written by the application, against a tuned Linux filesystem on the same device”* | the same host `E2-B02` waits for | `claims/0016` registered `pending` with both thresholds **stated before any measurement existed**, so they cannot be written to fit a first run; the modelled ratio 1.1336 with its decomposition; and `claims/baselines/linux-6.x-tuned-zoned/` — nine files with a `verify.sh` that cross-checks four workload dials against `zone/tests/cycle.rs`, so the baseline is configuration in the tree rather than prose that decays. | The same host, a guest booted on it, and the two pieces of plumbing `E2-B02`'s row names. |
| `E2-R01` | *“the rollback demonstration runs from the release image on a stranger’s machine”* | a person | Three of the release's six contents, each described as narrowly as it was built: `cargo xtask rollback` with `claims/0030` gating and running nightly; `cargo xtask swap` and `cargo xtask blk swapped`/`abandoned` for the live swap, with what a failed swap still costs a client written down rather than omitted; and `cargo xtask attest` with `claims/0032` and RFC 0012's five residuals named, so *attestation* cannot appear in release notes without them. | Somebody who is not this project running the command on a machine this project has never seen. The long plan calls this shape `G4` and puts it deliberately outside the team's control — the same condition `E1-R01` and `E0-R02` carry. |
| `E0-P06` | *“recorded with the reservation conditions from RFC 0007 named in the claim”* — the whole exit | `runner-class-A`, and `tsc-deadline` | 60 000 ticks at 1 kHz with a log-bucketed distribution, p50/p99/p99.9 and an exact maximum; every boot runs a hundred ticks of the same path. The measured path is the APIC one-shot, because TCG refuses `tsc-deadline` by name. | The same machine, with all four RFC 0007 components carved by F’s own frame and each recorded as obtained by partition or by exclusion. |

## E3 is open, so its blockers are machines and not yet rows

RFC 0109 is the decision behind this section and the argument for it being a
section rather than twenty-three rows. In one sentence: **a row is the receipt
for a clause a closing task gave up, and every line below is `[ ]`.** Four E3
exits have been narrowed — `E3-B01b`, `E3-B01d`, `E3-B04c` and `E3-B07a` — and
all four were narrowed by RFC 0084 on a *measurement*, not on a machine, which is
a fifth thing this page does not hold and RFC 0084 does.

What follows is therefore **not the register**. It is the answer to *which
machine, for which line*, which is the fact a `needs:` list hides.

### Which machine, and what waits on it

Read the third column as the interesting one. A line there is blocked by a line
that is blocked by hardware, and that is not the same as being blocked by
hardware: some of them have most of their work available and a reader flattening
the two kinds together would never find out.

| The machine | Lines that wait on it directly | Lines that wait on one of those | What closes it |
|---|---|---|---|
| A GPU — the machine `E5-D01` has not named | `E3-B02d`, `E3-B02e`, `E3-B02f`, `E3-B02g`, `E3-B02i` | `E3-B02h`, `E3-B02j`, `E3-B02k`, `E3-B02l`, `E3-B03g`, `E3-B03h`, `E3-B03j`, `E3-B06g`, and behind the last of those `E3-B06`, `E3-P04` and `E3-P05` | A part reporting compute shaders, and **`E5-P06`** — added on 2026-09-24 for this reading: *obtain the machine `E5-D01` specifies, and boot F on it*, on `E0-D10` and `E0-P18`'s precedent, with all five direct dependents naming both lines so the wait is on a boot rather than on a bill of materials. Five lines directly, eight behind them: the largest single purchase in the epoch. |
| `runner-class-A`, with F booted on it | `E3-B05d`, `E3-B07`, `E3-B07f`, `E3-P03` | `E3-P06` behind `E3-B07`; `E3-P02` needs it as well as the rig | The row for `E0-D10` above, and `E0-P18`. `bench/src/lib.rs` refuses a recording outside a recording class and that refusal is the harness working. |
| `photodiode-rig` | `E3-P01b`, `E3-P01e` | `E3-P01`, `E3-P01f`, `E3-P02`, `E3-B04`, `E3-R01` | The section above. Three of `E3-P01`'s six subtasks closed on the near side of the purchase and three did not. |
| An energy meter | `E3-P07` | none in E3 | A specification first — nothing in the file buys one, and there is no line to close. |

### What the virtual machine can close, line by line

Where a half exists it is named concretely enough to start on tomorrow. Where
there is none, that is said rather than padded.

- **`E3-B02h`** — rung 3, the all-CPU raster. **The half is almost all of it.**
  `scene/src/encode.rs` produces the linear encoding today, and binning, the
  scan and coverage are integer CPU arithmetic that needs no backend. What is
  missing is the *reference*: the exit says *identical again*, and the image it
  must be identical to is rung 1's. Its own `needs:` comment states the shape —
  *this line waits on a machine its own test does not use*. A rung-3 raster
  checked bit-exactly against a serial reference of its own is buildable and is
  strictly weaker, because `Fidelity::Exact` is a claim relating two
  implementations and a test comparing rung 3 to its own model does not make it.
  **That narrowing is charged to this line and is not taken here** — RFC 0084
  puts an exit edit in `intent/0012-the-interface/spec.md`, and the clause moves
  into the table above in the diff that closes the line.
- **`E3-B02l`** — the ladder on hardware that cannot run the top rung. Its
  subject is *a machine reporting no compute shaders*, which is every machine
  here, so it is on this page **only** because it waits on `E3-B02h`, and it
  comes off the day `E3-B02h` renders at all. Nothing about it is a purchase.
- **`E3-B02j`** — the scene `claims/0033` did not fix. Its exit is a choice, an
  argument and a content hash: three things a text editor produces. It waits on
  `E3-B02i`, which is rung 4 on a GPU. RFC 0088 settled that a decide task's exit
  may not require its implementation, and this is a build line whose exit is a
  decision. Naming that is this page's business; moving the `needs:` is
  `intent/`'s.
- **`E3-B02k`** — four rungs, one scene, one machine, one invocation. **No half.**
  A GPU and a recording environment, both, and `cargo xtask claim
  raster-cost-per-rung` refuses with `Route::Unbuilt("E3-B02")` until there is
  one.
- **`E3-B02d`, `E3-B02e`, `E3-B02f`, `E3-B02g`, `E3-B02i`** — the halves are the
  serial sides and the plumbing. `E3-B02d` is *bit-identical to a serial
  reference*, and the serial reference is half of that test. `E3-B02f`'s *one MMIO
  write rather than a system call* is a property of a submission path
  `abi/src/sync.rs` already carries the shape of. What none of them has is an
  image or a device, which is the other half of each.
- **`E3-B03g`, `E3-B03h`** — the atlas and the size rule. *Residency bounded by a
  named maximum* and *no atlas entry regenerated for unchanged text, counted per
  frame* are counts over a CPU-side cache; *the rule is data a test reads rather
  than a constant in a branch* is a table. Both end in pixels that come from
  `E3-B02e`.
- **`E3-B03j`** — how a rendering is compared to a reference. *An integer over
  pixels with a stated tolerance, and a deliberate one-pixel regression goes red*
  needs two images and no GPU. It waits on `E3-B02e` because the rendering is
  rung 1's; the day rung 3 renders, the comparison has a producer. This is the
  strongest half on this page.
- **`E3-B06g`** — the display projection. The exhaustive `Role` match with no
  wildcard arm is a compile-time property available today. *Reaches pixels* is
  not, and `E3-B06`, `E3-P04` and `E3-P05` are behind it.
- **`E3-B05d`, `E3-B07`, `E3-B07f`, `E3-P03`** — the halves are what `E0-P05`'s
  row above already records for a time: a workload that runs, thresholds written
  before any number exists, and a distribution drawn. `E3-B05d`'s own sentence is
  the one to keep — *a bound whose evidence is a central tendency is not a bound*
  — and a maximum is the hardest thing to take in an emulator.
- **`E3-P01b`, `E3-P01e`, `E3-P01f`, `E3-P01`, `E3-P02`** — the photodiode
  section above is the full account and nothing here changes it. A distribution
  is a set of measurements of a real actuator, a calibration is a reproduction of
  a known number, and a document containing a calibration result contains one.
- **`E3-P07`** — **no half at all, and no line that would make one.** See the
  meter's section above: what could be had from here is a bill of materials and a
  stated error bar, which is what `E3-P01a` and `E3-P01c` are for the rig, and
  nobody has written either.
- **`E3-B04`** — a full frame of latency removed, *shown as before-and-after on
  the rig*. No half; the measurement is the exit. Its `needs:` names `E3-P01e`
  rather than `E3-P01`, on the argument that a document is not a prerequisite for
  a measurement — which is right, and `E3-P02`'s blanket `needs: E3-P01` does not
  carry it.
- **`E3-R01`** — release 0.4, carrying `E3-P02`'s claim, `E3-P04`'s demonstration
  and `E3-P03`'s parity result. Two of the three are on this page, so there is no
  half until they have one.

### What is deliberately not on this list

**The fourteen open E3 lines that are reachable here.** `E3-B05b`, `E3-B05c`,
`E3-B05f`, `E3-B06d`, `E3-B06e`, `E3-B06f`, `E3-B06h`, `E3-B06m`, `E3-B07c`,
`E3-B07d`, `E3-B07g`, `E3-B01i`, `E3-B01j` and `E3-B03c` name no machine. They
are written out because `cargo xtask lint-debt` compares two sets of task ids and
cannot know whether a clause was reachable — only a reader can, and a reader
needs the list to refuse a row with.

**`E3-B06l`.** Its exit wants *a corpus of applications ported by somebody who
did not write the vocabulary*, which reads like a second person and is not one.
RFC 0110 settled the same requirement for `claims/0035` with no purchase: a
corpus is a directory with recorded provenance, and the module's own
demonstration is refused **by value** so a flattering share is not expressible.
What `E3-B06l` lacks is applications, of which this tree has one declaration.
Work, not debt — with a residue: RFC 0110's by-value refusal has no analogue for
an application, so independence there is provenance a reader trusts rather than
arithmetic a run performs.

**`E3-P04`, `E3-P05`, `E3-P06`.** Derived only, through `E3-B06` and `E3-B07`, and
listed in the table above so a reader does not count them twice. `E3-P05`'s own
exit is a property of the tree's tests, already narrowed by RFC 0091, and what it
waits for is a UI suite rather than a machine. Unstarted work behind unstarted
work is not debt at either depth.

**The AArch64 gap, which closed.** `E3-B01h` held at `[>]` for two days on *on
both architectures* and `E3-B04a` on *one time source in the whole path*; both
closed, and neither closed on hardware — the first on the
`tests (AArch64, weak memory)` job at commit `cbd58e8`, the second on
`cargo xtask input deliver`. A gap paid by a runner that already existed is the
opposite of a row on this page, and the two should not be written in one voice.

**The four exits RFC 0084 narrowed.** `E3-B01b`'s quantifier, `E3-B01d`'s *model
of* a ring, `E3-B04c`'s maximum over a named corpus, and `E3-B07a`'s constructor
— since superseded in part by RFC 0102's seventh opcode. Each lost a clause and
none of them lost it to a machine: they lost sentences that could not survive
their own first measurement. RFC 0093's four categories do not include that, and
RFC 0084 is where they are recorded.

## How this page is kept honest

`docs/TESTING-STATUS.md` published *"Built, fifteen entries, six gating"*
against a registry holding thirty-three and seventeen, in runs whose own output
counted the claims correctly four lines above. A register written as prose
somebody updates when they remember arrives in the same place.

So `cargo xtask lint-debt` compares two sets: the tasks whose `TODO.md` entry
cites RFC 0093, and the rows below. A narrowing with no row is a clause deleted
rather than moved, which is the one thing the RFC rests on; a row naming no
narrowing is a debt nobody owes, or a clause already answered whose row should
have gone in the diff that answered it. It is the discipline `gap_holds` applies
to `OWED_REVERSALS`, `CHAOS_GAP` and five others, pointed at this page.

What it does not check is the prose above the table. The blocker sections are
read by people and kept by people, and saying so is better than implying a
coverage that is not there.
