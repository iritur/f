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

**`E2-B02` and `E2-P10`.** Zoned virtio-blk emulation needs QEMU 8, and the
tuned-Linux zoned baseline needs a directory that does not exist yet. Both are
real blockers and neither is a machine: a development image is a change to
`docker/Dockerfile`. Work, not debt.

**`claims/0022 copies-per-read`.** It is `pending`, and its `[hardware] runner`
says `runner-class-A` like every other pending claim — but its own note says
the machine it needs *is not a runner class*, it is a boot of `user/objects`
over the blk driver's ring. A task in this tree. It stays off this page, and
the `[hardware]` field that says otherwise is a defect in the registry rather
than a reason to record a debt nobody owes.

## The blockers

Five things stand behind every row below.

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

### A stranger

Three exits name a person who has not read this tree doing something and
timing it. No commit can be that person.

## The register

Each row: the clause as it stood, the task it came out of, the blocker, what a
virtual machine did establish in its place, and what would close it.

<!-- Rows are added as tasks are narrowed. `cargo xtask lint-debt` is what
     keeps this table and the registry from drifting apart. -->

| Task | Clause narrowed out of its exit | Blocker | What the VM established instead | What closes it |
|---|---|---|---|---|
| `E2-B04` | *“two machines”* — the generation root compared across two hosts rather than two checkout paths on one | a second machine | The path-dependence class, which is the one that bit: before `-Zremap-cwd-prefix` landed, one commit at two checkout paths gave two frame leaves and two roots. `claims/0028` is that comparison, gating. | A second host running the weekly job’s `root` matrix. |
| `E2-P06` | *“identical generation root hash, checked weekly, across two machines and two dates”* | a second machine, and a second date | A non-reproducible input fails the job and **names itself** — `generation --mutate` arms a defect, the comparison goes red, and the harness requires the `kernel` leaf by name rather than any red. | Two runners, and two weeks landing on one commit. `lint-remap` prints both limits on every green run. |
| `E1-B09` | *“both paths measured; the notification-cost claim is recorded with the hardware named”* — the whole exit | UINTR silicon | The path is built to the point of refusing to construct: `Bell::new` returns `Err(Path::UserInterrupt)` rather than downgrading. The kernel-IPI path is delivered and demonstrated at boot. | A part implementing Intel UINTR, under KVM rather than TCG. Then *both paths measured* becomes a number rather than an instruction nobody has issued. |
| `E1-B10` | *“the cost, measured”* — `claims/0004` moving off `pending` | `runner-class-A` | Both paths pass the same ownership tests; the workload runs; the thresholds are written **before** any number exists, which is the half of a claim that can be made from here. | The same machine as 0001 and 0002. |
| `E1-R01` | *“a third party runs it”* | a person | The command exists, is documented where a stranger would look, and produces a seed sweep from a clean checkout. | Somebody who is not this project running it. The long plan calls this shape `G4` and puts it deliberately outside the team’s control. |
| `E0-D10` | *“the machine half”* — a machine meeting `claims/runner-class-A.md` exists and can be run on | purchase | A 209-line specification: capabilities with one worked example rather than a part number that ages out, the firmware settings that cannot be undone from an operating system, a kernel command line, a recipe per reservation component, and a seven-item checklist a stranger runs before setting `F_ENVIRONMENT`. | Somebody buying the machine and the checklist passing on it. Until then `F_ENVIRONMENT=runner-class-A` is an assertion no code in this tree can tell from a lie. |
| `E0-P18` | *“bare metal”* — a boot on a machine that is not a guest | real hardware | Three boots to `M0 ok` outside QEMU carrying one process, then four components and a supervisor, then E2’s state trees; and three failures on an AMD host that found RFC 0070 and RFC 0074, both of which QEMU could never have shown. | An x86-64 machine with a serial port and Secure Boot off — **not** a purchase, and not class-A silicon. The cheapest unblocked item in the epoch and the only one whose result nobody can predict. |
| `E0-R02` | *“a person who has never seen the repository reproduces claim 0001 from the README in under thirty minutes, including toolchain install”* | a person | `cargo xtask reproduce <claim>` dispatches over the registry rather than reimplementing per claim; `lint-reproduce` asserts the command exists, is that claim’s own, routes to something that runs, and names a runner class with a specification beside it. A README section that is the whole route. | A named person who has not read this tree running it on a machine of their own, with the wall-clock time and where it went recorded. |
| `E0-B12` | *“Under 50 ns per operation, recorded as a gating claim”* | `runner-class-A` | One million NOPs in batches of thirty-two through the batch path, and the distribution drawn; the kernel proves the layout on every boot, adopting a header out of the bytes and reporting a forged slot number as corruption. | The same number `E0-P05` owes. One machine closes both, and this row goes with that one. |
| `E0-P05` | *“`cargo xtask claims` reports it green; a deliberate 20% regression fails the build”* — the whole exit | `runner-class-A` | The workload runs to completion and the distribution is drawn; `bench` refuses to record it, naming the environment. `claims/0001` is `pending`. | A class-A machine passing the seven-item checklist, F booted on it (`E0-P18`), and `cargo xtask claim ring-submit-latency` recording a number. |
| `E0-P06` | *“recorded with the reservation conditions from RFC 0007 named in the claim”* — the whole exit | `runner-class-A`, and `tsc-deadline` | 60 000 ticks at 1 kHz with a log-bucketed distribution, p50/p99/p99.9 and an exact maximum; every boot runs a hundred ticks of the same path. The measured path is the APIC one-shot, because TCG refuses `tsc-deadline` by name. | The same machine, with all four RFC 0007 components carved by F’s own frame and each recorded as obtained by partition or by exclusion. |

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
