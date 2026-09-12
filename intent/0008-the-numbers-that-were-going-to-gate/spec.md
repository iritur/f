---
id: 0008
status: draft
reviewed_by: "pending: Dmitri Chudinov"
skills: claims-registry, determinism-review, rfc-author, frame-and-unsafe, licence-boundary
---

# Spec: the four are one set, and three of them can gate this week

The four claims `intent/0006-state/spec.md` promised would gate — `claims/0016
write-amplification`, `claims/0017 bytes-rechunked-per-byte`, `claims/0019
resident-bytes-per-unit-of-work`, `claims/0022 copies-per-read` — are all
`status = "pending"`, and the audit's premise survives contact with the source:
nobody has looked at them together and no line in the tree makes anybody. What
does not survive contact is the assumed reason. **None of the four is blocked by
`F_ENVIRONMENT`**, and each says so in its own `[hardware] notes`
(`claims/0016:215`, `claims/0022:143`); the environment refusal blocks the
*other* eight pending entries, every one of which has a nanosecond or a
millisecond as its primary metric. All four of these are counts, and all four
are blocked by software this project has not written. Further: **three of the
four already hold a measured, thresholded, machine-independent primary metric** —
`bytes_rechunked_per_edit_chunked_large` = 262 144 against 786 432
(`claims/0017:201`, `:286`), `resident_bytes_per_read_byte_micro` = 1 000 000
against 1 000 000 (`claims/0019:106`, `:121`), `copies_per_read` = 0 against 0
(`claims/0022:100`, `:116`) — and every one of those numbers gates nothing,
because `status` is one field per file and each file also holds a row that has
no number. This spec separates the two and makes the separation data rather than
four paragraphs.

## Behaviour

### 1. What each of the four is actually waiting for, checked rather than repeated

This is the table the intent asks for, verified against source on 2026-09-11.
Several sentences in the tree that name a blocker are stale — true when written,
false now. The audit exists because prose drifts, so correcting them is part of
this spec's work rather than an aside.

| claim | primary metric | measured? | what it waits for | verdict |
| --- | --- | --- | --- | --- |
| 0016 | `device_bytes_per_app_byte` | **no** | QEMU 8 in the dev image; the boot of `user/objects` over the blk ring; a run of `claims/baselines/linux-6.x-tuned-zoned/apply.sh` | stays `pending`; its modelled rows leave |
| 0017 | `bytes_rechunked_per_edit_chunked_large` | **yes** | nothing, for the primary. The file is `pending` against a denominator no row of it uses | **gates**, statement corrected |
| 0019 | `resident_bytes_per_read_byte_micro` | **yes** | nothing, for the primary. Two of its three named blockers are stale | **gates**, statement corrected |
| 0022 | `copies_per_read` | **yes** | nothing, for the primary. Its one named blocker is stale in part | **gates**, statement corrected |

The reasons in full, because they differ and the differences are the finding:

- **0016 is the only one whose subject cannot be counted today.** Its primary is
  `device_bytes_per_app_byte` counted by QEMU `query-blockstats` in a guest
  (`claims/0016:9-11`), and `ratio_vs_baseline` needs that run and a peer run of
  the tuned f2fs configuration. `emulated = true` (`claims/0016:214`) is
  permanent by the file's own words — the device is a model whichever way this
  goes, and no work in this tree makes it real hardware. What it is *not* blocked
  on is the baseline: `claims/baselines/linux-6.x-tuned-zoned/` exists with nine
  files including `apply.sh` and `verify.sh`. Written down, never run.
- **0017 is `pending` against a denominator its own promise never named.**
  `intent/0006-state/spec.md:1022-1024` registers it as `bytes_rechunked_per_edit`,
  *counts*, *gating*, `max = 786 432` on the chunked kind and `2 097 152` on the
  extent kind. Every one of those rows is measured, and every one is compared by
  `cargo xtask claim bytes-rechunked-per-byte` (`Route::Rechunk`, paid at
  `E2-B09`). The file is `pending` because its *name and statement* adopted *per
  application byte*, and an application byte is defined once as a byte submitted
  on the objects ring (`claims/0017:17-22`), which does not exist. The ratio the
  name promises has no row, no threshold and no number; the bound the spec
  promised has all three.
- **0019's blockers are two-thirds stale.** `claims/0019:76` says the figure is
  read from the component's own accounting "because `E1-B15` has not landed and
  no component publishes a subtree yet". `E1-B15` is `[x]` (`TODO.md:845`),
  schema 3 carries `[[state]]` and `Record::state_nodes`
  (`abi/src/manifest.rs:758`), and five state trees were mounted and read back
  through their roots on the third boot outside QEMU (`TODO.md:511`). The heap is
  likewise landed — see below. What is genuinely unmet is the *unit*: the spec
  defines this as resident **pages** read from the frame's state tree
  (`intent/0006-state/spec.md:1047-1050`), `user/objects` is not a component so
  it publishes no subtree of its own, and payload bytes are what the run can
  count.
- **0022's one blocker is half stale.** `claims/0022:64-67` says an image linking
  `f-blob` needs a `#[global_allocator]`, "the spec's decision 4 puts that heap
  in `ring/` and it does not exist". `ring/src/heap.rs` exists — `unsafe impl
  GlobalAlloc for Heap` at line 397, eight tests in `ring/tests/heap.rs`, landed
  at `f77057f` under `E2-B10`, and `user/store` already allocates over it
  (`user/store/src/component.rs:98`). The same stale sentence is in
  `user/objects/src/lib.rs:53` and in `E2-B08`'s `TODO.md` prose. What remains is
  the ring: `user/virtio-blk` answers two of RFC 0060's seven opcodes, and
  `user/objects` is a library with no `manifest.toml` and no entry in
  `COMPONENTS` (`xtask/src/main.rs:1308`).

None of this is a timing. `WHY_CONTAINER` (`bench/src/lib.rs:306`) blocks
`claims/0001`, `0002`, `0004`, `0006`, `0007`, `0011`, `0013` and `0015`, and
nothing else. Twelve entries are `pending`; eight wait for a machine and four
wait for a component, and the registry says nothing about which is which.

### 2. Three claims gate, at the boundary they name

`claims/0017`, `claims/0019` and `claims/0022` go `status = "gating"`. **No
number moves and no threshold moves.** What changes is the statement, rewritten
to name the boundary the rows were taken at, so that a claim's subject and its
evidence are the same thing:

- 0017's name and statement become *bytes re-chunked per edit*, which is what
  every row of it measures and what `intent/0006-state/spec.md:1022` promised.
  The per-application-byte ratio becomes a named row with no number and a
  `[blocked]` entry pointing at `E2-B08`.
- 0019's statement names the component's own accounting in payload bytes as the
  boundary, and states the direction of the error: pages are what an operating
  system charges, payload bytes are a floor, and the published figure moves
  **upward** when the state-tree reading arrives. The pages reading becomes a
  named row with no number and a `[blocked]` entry.
- 0022's statement names the modelled `ZonedDevice` boundary. The ring-boundary
  count becomes a named row with no number and a `[blocked]` entry.

The rule that decides this, stated once so it is not re-derived per claim: **a
claim gates when its `[metrics] primary` has a number taken at the boundary its
statement names.** A file may hold rows at two boundaries — `claims/0016` already
does, `device_*` beside `modelled_*` — and the rows with no number do not stop
the rows with one from gating. This keeps the intent's constraint rather than
routing around it: nothing is promoted, and a count taken at a different boundary
is still a different count however similar it looks.

### 3. `claims/0016` splits, on `claims/0005`'s precedent

0016's primary cannot become the measured row without promoting a model into a
published device count, which the file refuses in its own words
(`claims/0016:106-109`). So it splits the way `0005`/`0006`, `0012`/`0013` and
`0014`/`0015` split: the half that can be counted on the machines this project
has becomes its own `gating` entry, and the half that waits keeps the number and
the promise.

A new claim — `zoned-cycle-decomposition`, at the next free number — carries
`modelled_device_bytes_per_app_byte` = 1.1336 (`max = 1.5`), the fill and
copy-forward terms, `zones_reset`, `zone_finish_entries`, `zone_reset_entries`,
`flush_entries`, and `collector_operations_ahead_of_hard_read` (`min = 1, max = 1`,
RFC 0059's third invariant). Its statement says in its first sentence that
`ZonedMemory` is a model in the host and that the number is a property of the
mapping's arithmetic rather than of a device. `claims/0016` keeps
`device_bytes_per_app_byte` and `ratio_vs_baseline`, stays `pending`, and cites
the sibling the way `0005` cites `0006`.

### 4. `pending` says what it waits for, as data

Every `pending` claim gains a `[blocked]` table:

```toml
[blocked]
on    = "component"          # "machine" | "component"
owner = "E2-B08"             # a task ID in TODO.md
why   = "one sentence a reader can check"
```

`on = "machine"` is *waiting for a machine this project does not own* — the eight
timings, all of them `E0-D10`. `on = "component"` is *waiting for something this
project has not written* — every row this spec leaves unmeasured. Those two fail
differently and are answered by different people, which is the intent's first
open question, and this is the answer: **one status, two kinds, and the kind is a
field rather than a paragraph.** A fourth status word was the alternative and is
refused — `claim_run`'s match at `xtask/src/main.rs:14864-14885` is where the
cost of a new status shows up, and `evidence` in RFC 0069 (draft, in this working
tree) makes the same move on a different axis. The two keys should look alike.

`cargo xtask lint-claims` (`xtask/src/main.rs:14196`) then refuses:

- a `pending` claim with no `[blocked]` table;
- an `owner` that is not a task ID present in `TODO.md`. The lint **reads**
  `TODO.md` and never writes it (`xtask/src/main.rs:485-487`), and its failure
  message names the originator as the person who adds a missing line — for the
  reason the `Gap` type's fourth field exists: an instruction that assumes the
  reader already knows the answer is not an instruction.

### 5. One command answers "promised, and measured?"

`cargo xtask claims` (`xtask/src/main.rs:14235`) prints name and status and
nothing else. It gains a `milestone` column and a `blocked on` column, and
`--milestone E2` filters. That is the intent's third outcome and it is two
columns: `milestone` is already read by `claim_run` (`xtask/src/main.rs:14779`)
and `[blocked] on` is the field above. The snapshot `write_snapshot` rewrites on
every listing carries both, so the answer is also in a file a release packages.

### 6. Release 0.3 names the numbers it will not have

`STORAGE_GAP`, beside `DATAPATH_GAP` (`xtask/src/main.rs:3143-3160`): one row per
number release 0.3 ships without, each with the source condition `gap_holds`
checks (`xtask/src/main.rs:500`) and the documents that say so — `TODO.md`,
`RELEASING.md`'s new section, this spec, and the claim. When a gap closes the
build goes red and prints the list, which is the whole reason the fourth field is
there. `RELEASING.md` gains *Release 0.3, and the numbers that are not in it*, on
RFC 0056's terms and on release 0.2's precedent (`TODO.md:969`). The rule and the
mechanism both already exist; what does not exist is the E2 rows.

### 7. The stale sentences, corrected in this diff

`claims/0019:76`, `claims/0022:64-67`, `user/objects/src/lib.rs:49-57`, and the
`[hardware] notes` of 0019 and 0022 where they repeat the same blocker. Each
names something that has been paid. They are corrected here rather than left for
whoever next opens the file, because a spec that found them and did not fix them
has made the audit's own complaint worse. `E2-B08`'s `TODO.md` prose carries the
same sentence and is **reported to the originator, not edited** — proposed task
lines accompany this spec.

## Policy applied

**Determinism (RFC 0004).** Nothing here observes time, randomness or ordering;
every number it touches is a count from a seeded run and no new number is taken.
Two things still constrain it. The new `lint-claims` reading walks `claims/` and
reads `TODO.md`: `BTreeMap`/`BTreeSet` only, because `xtask` is checked by the
determinism lint it implements, and if it grows a directory walk it takes the
shared skip list rather than a fifth copy — `CLAUDE.md`'s scar about the four
walkers that read four *other* checkouts. And the new columns keep the file-name
ordering the existing `BTreeMap<String, String>` imposes rather than sorting by
status, so a reader diffing two runs sees a stable list.

**The frame (RFC 0001).** Nothing needs `unsafe`; all of it is in `claims/`,
`xtask/`, `docs/` and `RELEASING.md`. Worth saying because the blocker this spec
keeps declining to pay is the one that *did* need the frame — a component image
linking `f-blob` needs a `#[global_allocator]`, that is an `unsafe impl
GlobalAlloc`, and it is in `ring/src/heap.rs` where RFC 0001 puts it. That debt
is paid. What remains is a component image and five blk opcodes, and neither is a
frame question.

**The licence boundary (RFC 0003).** Untouched. Nothing here imports
`third_party/` or reads across it.

**Evidence (`claims/README.md`).** Rule 2 is what this spec is about: `gating`
fails the build on regression, `tracked` records without gating, and muting is a
status change in a reviewable diff. Three files move from recording nothing to
failing the build on numbers they already print. The rule this spec must not
break is the intent's own — no threshold is fitted to a first measurement — and
none is: every threshold named here was written before its run and is carried
unchanged.

**Decisions (RFC).** One RFC is owed, and it covers three reversals at once
because they are consequences of one decision: *a pending claim names what it
waits for, and a claim gates at the boundary it names*. It reverses
`intent/0006-state/spec.md:1047-1050` (0019 was to be read as resident pages from
the frame's state tree and will gate as payload bytes), restates `claims/0017`'s
subject from per-byte to per-edit, and adds a required table to `claims/*.toml`
plus a sixth rule to `claims/README.md`. Its *What would reverse this* section is
the one that matters: if a claim gating at a modelled boundary is ever quoted
without its boundary clause, the split was too subtle and the right answer is two
files rather than two row-sets in one.

**The collision this spec is most likely to cause, and it is the one `E1-B15`
already recorded.** RFC 0069 is a draft in this working tree that adds an
`evidence` key to every entry in `claims/`, a rule to `claims/README.md`, and
changes to `lint-claims`, `claim`, `claims --render` and the snapshot. This spec
adds a `[blocked]` table to every pending entry and changes the same four places.
Two parallel worktrees touching one numbered registry will both take the same
number and both stamp a field at the same byte — `TODO.md:851`,
`docs/postmortem/0001`. So: **allocate this RFC's number before branching**, and
land this after RFC 0069 or in the same diff. If they land in parallel, the merge
is the first execution of a program nobody has run. (RFC 0069 also contains a
sentence this audit falsifies — *every pending entry is a time* — which is these
four; whoever owns that draft should hear it from this spec rather than from a
reviewer.)

## Not in scope

- **Taking any of the four promised numbers.** No image base change to trixie, no
  boot of `user/objects` over the blk ring, no run of
  `claims/baselines/linux-6.x-tuned-zoned/apply.sh`. Those belong to `E2-B02`,
  `E2-B08` and `E2-P10`, and this spec's whole point is that they are not free.
- **Making `user/objects` a component image**, and the five blk opcodes of RFC
  0060's seven a real ring under it needs. `E2-B08`, `E2-B10`'s boot half,
  `E2-B02`.
- **The eight pending timings and `E0-D10`'s machine.** The intent rules them out
  in as many words, and the separation this spec draws is the reason they stay
  out: they fail differently and are answered by a different person.
- **RFC 0069's `evidence` key.** Coordinate, do not absorb. A spec that swallowed
  a draft RFC from another worktree would be the merge problem it is warning
  about.
- **Change-point detection replacing thresholds** (`E2-P09`). Every threshold
  here is carried unchanged and this spec takes no position on whether a
  threshold is the right instrument.
- **Editing `TODO.md`.** Proposed lines go to the originator.

## Evidence

- `cargo xtask claims --milestone E2` prints, in one screen, every claim this
  epoch promised, its status, and for each `pending` one the kind of thing it
  waits for and the task that owns it. The intent's third outcome is this
  command's output.
- `cargo xtask lint-claims` fails on a `pending` claim with no `[blocked]` table,
  and fails on a `[blocked] owner` naming a task `TODO.md` does not have. Both
  failures are demonstrated by breaking them on purpose before the diff lands —
  `E2-B09`'s method, after `claims/0017` spent a wave unable to fire inside a
  green `verify`.
- `cargo xtask claim bytes-rechunked-per-byte`, `... resident-bytes-per-unit-of-work`
  and `... copies-per-read` print `this claim gates the build` where they printed
  `status is pending ... Not evidence. Not gating.`, against the same numbers and
  the same thresholds as the day before.
- `cargo xtask claim zoned-cycle-decomposition` reproduces 1.1336 at the modelled
  boundary and gates on `max = 1.5`.
- `gap_holds("STORAGE_GAP", STORAGE_GAP)` passes inside `lint-claims`, and its
  fixture proves a row can fail: a check that has never failed is
  indistinguishable from one that cannot (`xtask/src/main.rs:490-495`).
- `RELEASING.md` can say something it could not: which of release 0.3's numbers
  will be absent on the day it ships, and for which of two reasons each.

## Risks and reversal

**Most likely to be wrong: three claims gating at a modelled boundary read, to a
skimmer, as the epoch's promise met.** That is the intent's second-order cost
arriving by a new route. The observation that would say so is a sentence quoting
`copies_per_read = 0` or `resident_bytes_per_read_byte` = 1.0 without its
boundary clause. `lint-claims`' citation check catches that in `docs/design/` and
nowhere else — and there is already a live example it cannot see:
`docs/design/deadline-all-the-way-down.html:208` and `:285` sell "the
write-amplification claim" in prose with **no rendered number**, so the citation
check is structurally blind to them. Those two sentences are exactly where a
threshold that has stood unmeasured for an epoch starts to look like a result.
Reversal condition: if a second such sentence is found, the citation check's
scope is wrong and it should read prose references to a claim *by name*, not only
rendered values.

**Second: `[blocked] owner` points into a file this tree's agents may not edit.**
A lint requiring a field that points at a line only the originator can add is a
lint that can deadlock a diff. Mitigated by refusing an *unknown* ID rather than
requiring a particular one, and by the failure message naming who adds the line.
If it deadlocks anyway, `owner` becomes advisory and the check drops to a warning
— a real loss, and the reason it is written as a refusal first.

**Third: renaming `claims/0017` breaks references the citation check does not
scan.** It reads `docs/`. `intent/0006-state/spec.md`,
`intent/0006-state/plan.md`, `TODO.md` and RFC 0061, 0062 and 0064 all name the
claim. The rename is mechanical and the grep is cheap; the risk is that it is
done once and not re-checked. If a second claim ever has to be renamed for this
reason, that is the signal the citation check's scope is too narrow.

**Fourth, and the one that makes this whole spec optional: the split may be one
mechanism too many.** If a reader must hold *status*, *evidence*, *`[blocked]
on`* and *which boundary each row was taken at* in their head to read one claim
file, the registry has become the thing it exists to prevent. The observation
that would say so is a reviewer asking what a green row means and getting four
answers. The cheaper alternative — refused here, worth re-opening if that happens
— is per-row status, which is a bigger schema change with no precedent in this
tree, against a split that has three.

## Decisions taken on the originator's behalf

1. **Three of the four gate now, at the boundary their rows were taken at.** The
   intent offered three verdicts per claim and this takes the first for 0017,
   0019 and 0022. The alternative was the second verdict for all four — a task
   and a date — which is the status quo with a table on top, and the intent
   forbids a fifth honest paragraph in as many words.
2. **`claims/0016` splits rather than waits whole.** Its modelled rows are
   checkable today and its device rows are not; `0005`/`0006` is the precedent
   and the reasoning is one this project has already accepted twice more.
3. **Nothing is withdrawn and no design page loses a sentence.** Checked: no page
   under `docs/design/` renders a number from any of the four. The intent's third
   verdict is therefore not needed, and the two prose sentences that *sell* the
   write-amplification claim gain the boundary and the status rendered from the
   registry rather than being deleted.
4. **One RFC, not three.** The three reversals are consequences of one decision.
   A reviewer who disagrees should say so before the plan: splitting it into
   three afterwards is three times the citation work.
5. **`[blocked]` rather than a fourth status word.** Argued above; the cost of a
   new status is visible at `xtask/src/main.rs:14864`.

### Open questions carried forward, unanswered

- **Does release 0.3 wait for any of the four, or ship and say what is missing?**
  This spec answers the *mechanism* — it ships and names them, on RFC 0056 and
  release 0.2's precedent — and deliberately does **not** answer the schedule.
  `E2-R01`'s `needs:` line lists `E2-P10`, which is the four's owner of record,
  and whether that dependency stays is the originator's call, not a spec's.
- **How is a promise allowed to age?** The intent asks whether a design page
  selling an unmeasured number loses the sentence or gains a date. It is not live
  for these four (decision 3), so this spec has no evidence with which to write a
  rule about all documents, and inventing one from a case that does not arise is
  how a rule gets written that nobody can apply. Carried.
- **Is the `[blocked] on` vocabulary two values or three?** `machine` and
  `component` cover all twelve pending entries today. A claim waiting on a *third
  party* — `E1-R03`'s reproduction by somebody who is not this project — is
  neither, and gate G4 is the long plan's one deliberately out-of-reach outcome.
  Left at two, because a third value with no member is a value nobody gets right
  the day the first one arrives.
