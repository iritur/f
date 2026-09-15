---
id: 0012
status: draft
spec: ./spec.md
---

# Plan: land the decomposition where the graph can read it

This change builds nothing. Its whole diff is three Markdown files, and the work
it describes is somebody else's to start — which makes the plan short and makes
the last section the important one, because the only thing this entry can get
*wrong* in a way that costs an afternoon is telling the originator to paste
something the graph then misreads. The first draft did exactly that, in three
places at once, and the *Handoff* below is written to be executed rather than
interpreted for that reason: every line the originator must add or replace is
given in full, in the order it must be made, with the check that says the step
landed.

## Files

```
intent/0012-the-interface/intent.md    NEW: what E3-00 asks for, and how little of E3 can honestly start
intent/0012-the-interface/spec.md      NEW: the seventy-two lines, in TODO.md's format, with the availability buckets
intent/0012-the-interface/plan.md      NEW: this file, and the Handoff
```

Three files, no code. `E3-00`'s deliverable is an `intent/` entry and
`docs/sdlc.md` puts an intent before a spec before a diff, so a decomposition
that arrived as a diff would have skipped two stages to save one commit. Where
`E3-00`'s exit stands is stated once, in `intent.md`, and is not restated here.

**Files this change deliberately does not touch, and who owns each.**

```
TODO.md                    the originator's. The Handoff, and nothing else
xtask/src/main.rs          the orchestrator's. Step 10's lint, if step 10 is taken
docs/rfc/README.md         the orchestrator's. Three RFC rows are owed: see below
claims/README.md           the orchestrator's. One claim row is owed, down from three
interface/src/lib.rs       the orchestrator's. Nothing here adds a module; three subtasks would
Cargo.toml                 the orchestrator's. Nothing here adds a crate
```

The rows this work implies, stated so that whoever owns those files does not
have to re-derive them from the spec:

- `docs/rfc/README.md` — **three** rows, one more than the first draft counted.
  The vocabulary version-negotiation decision (`E3-B06a`, which RFC 0077's fourth
  reversal condition dates to the day the semantic tree gets an entry format);
  the shaper decision (`E3-B03a`, the first crossing of the licence boundary this
  epoch would make); and the narrowing that resolves `E3-B03`'s exit against
  `E3-P05`'s, which is *Handoff* step 8 and is a reversal of something already
  written down. Every number is the registry's to allocate and none is allocated
  here, for the reason `docs/postmortem/0001` records: two worktrees that each
  take a number from a registry they cannot both see will take the same number.
- `claims/README.md` — **one** row, down from three, and the change is not a
  scope cut. `claims/0034-canvas-escape-rate.toml` and
  `claims/0035-theme-refusals.toml` were registered while this entry was in
  review and already name the tasks that move them. What is still owed is the
  boundary-crossing count on `E3-B01j`, which is a **count** and may therefore
  gate on the machines this project already has, in the way `claims/0005`
  established.
## Order

1. **`intent.md` first, and it goes back to the originator before anything
   else.** It is the only one of the three a non-engineer would read, and the
   thing it is asking for approval of is not the task list — it is the sentence
   that six of seventy-two tasks can start and that fifteen need a purchase
   order. If that sentence is rejected, nothing below it survives.
2. **`spec.md` second**, because the seventy-two lines are only worth writing
   once the availability argument holds. `intent/README.md` rule 2: the spec goes
   back to the originator before a plan starts, and this is the review that is
   still cheap.
3. **`plan.md` third and shortest**, because the plan for a document is mostly
   the list of things the document must not do.
4. **Then, and only by a person: the *Handoff*.** It is ten steps and the first
   seven are one edit. Steps 1 to 7 must land together: a file corrected in a
   second pass is a file where somebody read the first pass in between, and in
   this particular case a file where `cargo xtask todo E3` reported nine
   available tasks to whoever looked in the gap.

The one sequencing rule worth stating because it is easy to get backwards:
**allocate the letter suffixes once, in `TODO.md`, before any worktree branches
off them.** Task ids are a registry, `docs/postmortem/0001` is what happens when
two parallel trees each take one entry from a registry neither can see, and
`E3-B01a` is exactly as collidable as an RFC number. Three ids in this
decomposition are already referenced from outside `TODO.md` — `E3-B02` from
`ROUTES`, `E3-B06l` from `claims/0034`, `E3-B01` from `claims/0035` — so a
renumbering is not free.

## Proof

There is no build here. The command that would say this worked is the graph's:

```
cargo xtask todo E3
```

run after steps 1 to 7, and the state of the tree before it says so is the state
this entry leaves: three new Markdown files, no Rust, no `Cargo.toml`, so
`cargo xtask verify` is unchanged by this change and running it proves nothing
about it.

What the graph must say afterwards, with the arithmetic in `spec.md`'s
*Behaviour* and the five checks in its *Evidence*:

- **`ready to start — 6 task(s)`**, and they are `E3-B01a`, `E3-B02a`, `E3-B03a`,
  `E3-B04a`, `E3-B06a`, `E3-P01a`;
- **`waiting — 81 task(s)`**, every one naming a blocker from the closed list of
  fourteen external ids in `spec.md`'s check 2;
- **`done 5 · standing 0`** — `E3-00` and the four decisions.

A seventh available task is the proof failing, not the tool being generous: it
means a `needs:` is missing, which is the same reading the epoch's standing
paragraph had to correct by hand on 2026-09-09 and the one thing a decomposition
can make four times worse. The exception is step 10: if `E3-B08` is added, ready
is 7 and the seventh is `E3-B08`, and none of the six moves.

**What this command does not check:** `E3-00`'s exit. Nothing in the tree
notices an `XL` without a decomposition, so after the paste a reader could delete
all seventy-two lines and every command would still exit zero. Step 10 is the
line that would change that.

## Risks

**A pasted `needs:` that names a task id which does not exist.** The graph
refuses outright — `TODO.md references tasks that are not in it` — so this one is
loud rather than silent, and it is the reason the *Handoff* gives every parent's
replacement `needs:` in full rather than describing it.

**A parent pasted without its replacement `needs:` line.** Silent, and the
failure the first draft would have shipped: `E3-B03` and `E3-P01` have no
`needs:` at all today, so they rank as ready with every one of their children
undone. Step 7's check is written to catch exactly this.

**Two worktrees allocating the same letters.** Covered above and it is the
likeliest mechanical failure, because this decomposition creates seventy-two
registry entries in one edit and `docs/postmortem/0001` is a record of what one
duplicated entry cost.

**A parent ticked `[x]` with its children `[ ]`.** Adding the edges made this
visible and did not make it impossible, because ticking a box is a human act.
Step 10 is the only thing in this plan that would make a machine notice.

---

# Handoff: what the originator must do to `TODO.md`

Ten steps. None of them is this entry's to make. Steps 1 to 7 are one edit and
land together; steps 8 to 10 are separable and each says what it is waiting on.
Every replacement line is given in full so that the edit is mechanical, and each
step ends with the observation that says it landed.

**Where each edit lands.** Line numbers as `TODO.md` stands on 2026-09-15, read
from the file rather than remembered, and every quoted string below was checked
against it. They are anchors for finding the line, not for applying the edit:
step 1 inserts seventy-two lines and everything after the first insertion point
moves, so **each step is applied by its quoted string, not by its number.**

```
  1209        ## E3 — The interface
  1216-1251   the standing paragraph                     step 4
  1253-1254   E3-00                                      step 2
  1258-1265   E3-D01 … E3-D04                            step 3
  1269-1271   E3-B01   exit 1270, needs 1271             steps 1, 5
  1272-1274   E3-B02   exit 1273, needs 1274             steps 1, 6
  1275-1276   E3-B03   exit 1276, no needs line today    steps 1, 7, 8
  1277-1279   E3-B04   exit 1278, needs 1279             steps 1, 7
  1280-1282   E3-B05   exit 1281, needs 1282             steps 1, 7
  1283-1285   E3-B06   exit 1284, needs 1285             steps 1, 7
  1286-1288   E3-B07   exit 1287, needs 1288             steps 1, 7
  1292-1293   E3-P01   exit 1293, no needs line today    steps 1, 7
  1315-1317   E3-R01   exit 1316, needs 1317             step 9
  1325        ## E4 — The platform
```

`E3-B03` and `E3-P01` having no `*needs:*` line at all is the reason step 7's
check is written the way it is: for those two the edit is an insertion and a
missed insertion leaves a line that ranks as ready with eleven and six undone
children behind it.

### Step 0 — the precondition, and it is not a `TODO.md` edit

Commit `docs/rfc/0077` through `0080`, `interface/`, and `claims/0033` through
`0035`. Step 3 marks four tasks `[x]` on the evidence in those files, and a `[x]`
whose evidence is untracked is a claim about a tree nobody else has. This is the
one point the adversarial review's eleventh finding lands, and it lands here
rather than on the spec.

### Step 1 — paste the seventy-two lines

From `spec.md`, each fenced block immediately beneath its parent line, in the
order the spec gives them, which is the order the risk falls in rather than
alphabetical. Eight blocks: `E3-B01` twelve, `E3-B02` twelve, `E3-B03` eleven,
`E3-B04` five, `E3-B05` six, `E3-B06` thirteen, `E3-B07` seven, `E3-P01` six.

*Landed when:* E3 holds 92 task lines and `cargo xtask todo E3` does not refuse.

### Step 2 — tick `E3-00` against the exit as written, and pay the linkage

**The `*exit:*` line does not change.** *E3 contains no `XL` task without a
decomposition* is a standing property of the section: step 1 makes it true, and
it is what a fifth undecomposed `XL` pasted into E3 next year would break.
Rewriting it into a statement about one afternoon would not satisfy it, it would
reverse it — and a reversal is an RFC proposed to the originator, not a line in a
handoff. An earlier draft of this step did exactly that and it is withdrawn.

What the step does change is the box and the linkage. `intent/README.md` and
`A-12`: an intent that becomes work names its task ids and the task names the
intent, and either half alone rots; today the intent names the ids and E3 names
no intent. Replace `E3-00` with:

```
- [x] **E3-00** `M` **Decompose this epoch before starting it.** Everything below is coarse on purpose; each task becomes five to fifteen tasks with exits when the epoch opens. `intent/0012-the-interface/` is the decomposition and the argument for it.
  *exit:* E3 contains no `XL` task without a decomposition.
```

*Landed when:* `cargo xtask todo E3` reports `done 1` before step 3, and the
**four** lines whose only blocker was `E3-00` — `E3-B01a`, `E3-B03a`, `E3-B04a`
and `E3-P01a` — leave the waiting list. Seven lines name `E3-00`; the other three
do not move here, and that is not a dropped `needs:`: `E3-B02a` also needs
`E3-D04` and `E3-B06a` also needs `E3-D01`, both of which step 3 ticks, and
`E3-B05a` also needs `E3-B01a`, which is work rather than a tick.

### Step 3 — tick the four decisions

`E3-D01` through `E3-D04` read `[ ]` and their exits are met. RFC 0077 closed the
vocabulary at twenty-two roles with no `Other`; RFC 0078 made the timeline
expressible with clips, tracks, times and a selection and put section 13's own
falsification — the clip between 4.2 s and 6.8 s on track 3 — into a test; RFC
0079 clamps a deliberately hostile theme and demonstrates it; RFC 0080 names four
rungs and registers `claims/0033`. The four modules are `interface/src/node.rs`,
`canvas.rs`, `token.rs` and `ladder.rs`. Mark each `[x]` and name the RFC and the
claim each produced.

This cannot wait for a second pass: a decomposition that reports six available
tasks while the decisions they depend on read `[ ]` reports zero.

*Landed when:* `done 5`.

### Step 4 — the E3 preamble, three stale sentences

The paragraph is dated *checked 2026-09-09* and three of its statements are no
longer true. The date should move with the corrections, or the paragraph should
say which day each clause was checked.

1. *"`cargo xtask lint-owed` reports five reversals fallen due and unpaid, four
   of them in the frame."* `OWED_REVERSALS` now holds three rows — RFC 0014's and
   RFC 0015's door retirements and RFC 0051's `Reported` — and the one that was
   paid is the frame's. Both the count and *four of them in the frame* are stale.
   The comment where RFC 0008's row used to be is worth reading before rewriting
   the sentence, because it says what is *not* claimed by the row's absence.
2. *"the tree a component owns that the degradation policy would write into
   (`E1-B15`) [is] `[>]`."* `E1-B15` is `[x]`. This matters to the decomposition
   rather than only to the prose: it is what moves `E3-B07`'s second exit clause
   out of the hardware bucket and into work this project can do today, which is
   step 7's `E3-B07d`.
3. *"Every timing claim in the registry — fifteen of thirty — is `pending`."* The
   registry holds **thirty-three** rows, of which fifteen are `pending`,
   seventeen `gating` and one `tracked` — re-counted on 2026-09-15 with the
   command below rather than carried over from the last draft — and the denominator moved
   because this epoch's own three claims landed in between, so a fourth
   re-assertion of the old figure would have been wrong for a reason this entry
   caused. Two of the fifteen are not times at all: `claims/0034` is a ratio of
   integers and `claims/0035` is a share, and both say in their own first
   paragraphs that this is why they could gate where a timing claim could not.
   The sentence should carry its date and the command that recomputes it:
   `grep -h '^status' claims/*.toml | sort | uniq -c`.

*Landed when:* nothing in the preamble contradicts `cargo xtask lint-owed`,
`TODO.md` line 846, or the registry.

### Step 5 — `E3-B01`'s `needs:`, which is the one graph change

This is the only step that removes a blocker rather than moving a sentence, so
the argument is given rather than asserted. `E3-B01` names `E0-B12` and `E0-B15`.
`E0-B12` is `[>]` for a number its own line assigns to `E0-P05` — *Claim 0001
stays `pending` until `E0-P05` runs it on `runner-class-A`* — and its ring,
layout, cursor protocol and suppression are in the tree and green. `E0-B15` is
`[>]` for a user-interrupt path behind a negotiated feature bit, which QEMU's TCG
backend implements no part of; its kernel IPI doorbell is in the tree. A scene
graph waits for neither. It also names the supervisor as still owing RFC 0008's
restart policy to the frame, which stopped being true at commit `9df5cba` on
2026-09-13.

Every external blocker this line used to name is now carried by the subtask that
actually waits on it: the supervisor by `E3-B01f`, the bound ring by `E3-B01g`,
and the other two by nothing at all. Replace the whole `*needs:*` line with:

```
  *needs:* E3-B01a, E3-B01b, E3-B01c, E3-B01d, E3-B01e, E3-B01f, E3-B01g, E3-B01h, E3-B01i, E3-B01j, E3-B01k, E3-B01l (its decomposition, and nothing else — every external blocker this line used to carry is now on the subtask that waits on it, and the two that no subtask names are named nowhere because nothing in a scene graph waits for a submit-latency number or for a user-interrupt path)
```

**Nothing may be added to that parenthesis that looks like a task id.**
`xtask`'s `ids_in` splits the whole `*needs:*` line and keeps every id-shaped
token, so a task named in the explanation becomes a blocker — silently, with no
diagnostic, because the id exists and the sentence reads as a comment. Writing
`E0-B12` inside the parenthesis to say it is *not* needed would put it back in the
graph. Every parenthesis below obeys the same rule, and `spec.md`'s check 5 is how
to tell.

If that argument is not accepted, leaving `E0-B12` and `E0-B15` in place costs
nothing measurable — the parent is blocked by twelve children regardless — and
the decomposition still stands. What it costs is the sentence: the spec's fourth
evidence check is that neither id appears anywhere in E3, and that check would
have to go.

*Landed when:* no `*needs:*` line in E3 names either id —
`sed -n '/^## E3/,/^## E4/p' TODO.md | grep '^  \*needs:\*' | grep -c 'E0-B12\|E0-B15'`
prints `0`. The grep has to be scoped to `*needs:*` lines or it goes red on a
correct execution: `E3-B01g`'s *exit:* names `E0-B15` deliberately, to record
where that task's unproven clause is finally observed, and an exit line is not
parsed as an edge by anything.

### Step 6 — `E3-B02`'s exit and `needs:`

The exit as written — *the CPU raster segment of the budget is zero, measured;
the fallback ladder is exercised on hardware that cannot run the top rung* — is
two observations, and RFC 0080 narrowed the first of them on 2026-09-14 in as
many words: section 08's claim holds for rung 1, which is the rung the section 05
budget is written for, and the lower three rungs are explicitly outside it and
carry `claims/0033`'s own thresholds. The CPU rung's is **not a multiple of the
top rung's** — the claim sets `cpu_raster_us_x100` at 840 000 against
`compute_path_us_x100` at 315 000 and derives it from section 09's latency chain
rather than from the rung above, and RFC 0080's narrowing paragraph says so in
those words, having corrected *about two and a half times* out of itself. Any
ratio quoted here is a number both of those files have already refused once, so
this step quotes the two thresholds and no ratio at all.

The second observation is about a machine with no GPU, which is every machine
this project has, and it has no business in an exit whose other half needs a
purchase order. Replace both lines with:

```
  *exit:* the CPU raster segment of the budget is zero on rung 1, measured — the rung section 05's budget is written for, with the lower three outside it and carrying `claims/0033`'s own thresholds, which is the narrowing RFC 0080 recorded on 2026-09-14. The ladder's descent onto hardware that cannot run the top rung is `E3-B02l`, because that is an observation about a machine with no GPU and this one is not.
  *needs:* E3-B02a, E3-B02b, E3-B02c, E3-B02d, E3-B02e, E3-B02f, E3-B02g, E3-B02h, E3-B02i, E3-B02j, E3-B02k, E3-B02l (its decomposition; the ladder decision and the backend driver this line used to name are now on the two subtasks that read them)
```

The narrowing needs no RFC: RFC 0080 already carries it, and this line is being
brought into agreement with a decision rather than reversing one.

*Landed when:* the sentence has moved rather than been copied — `E3-B02`'s
`*exit:*` no longer carries *exercised on hardware that cannot run the top rung*
and `E3-B02l`'s line does. The check fails on the parent being edited without the
child being pasted, and on the child being pasted without the parent being
edited, which are the two ways this step is half-done.

### Step 7 — the remaining six parents

Six `needs:` lines and four exits. The exits move an observation from a parent
onto the child that can observe it; **nothing stops being required and nothing
starts being required**, which is why none of these needs an RFC. The test,
stated once and applying to all four, in both directions because an earlier draft
stated only one of them and then added a requirement under it: a rewrite that
changes which line carries an observation is bookkeeping; a rewrite that changes
*whether* an observation is required — dropping one, or introducing one E3 did
not ask for — is a reversal and is an RFC. Step 8 is the only reversal in this
Handoff, and it is the originator's to file.

`E3-B03` — `needs:` only; the exit is step 8's:

```
  *needs:* E3-B03a, E3-B03b, E3-B03c, E3-B03d, E3-B03e, E3-B03f, E3-B03g, E3-B03h, E3-B03i, E3-B03j, E3-B03k
```

`E3-B04` — `needs:` only; the exit stays, because the parent takes the
measurement:

```
  *needs:* E3-B04a, E3-B04b, E3-B04c, E3-B04d, E3-B04e, E3-P01e (a *calibrated* rig, which is the line calibration is on; the blanket parent also waited on the rig's documentation, and a document is not a prerequisite for a measurement)
```

`E3-B05` — the second clause moves to `E3-B05d`, because a negative observable on
today's machines and a bound that needs `runner-class-A` fail differently and
were sharing one line:

```
  *exit:* no implicit wait appears in a frame trace. *Frame time is bounded rather than typical* is `E3-B05d`, which needs a named machine and this line does not.
  *needs:* E3-B05a, E3-B05b, E3-B05c, E3-B05d, E3-B05e, E3-B05f
```

`E3-B06` — the exit is `E3-P04`'s by reference and `E3-P04` needs `E3-B06`, so as
the two lines stand neither can be observed before the other, and a replacement
is forced rather than chosen. What replaces it is not new work: a closed
vocabulary whose consumers may hold a wildcard arm is not closed, and that is
RFC 0077's decision — accepted as `E3-D01` — rather than this Handoff's
addition. The line below observes it once over the four projections; each
projection's own half is on `E3-B06g` to `E3-B06j`.

```
  *exit:* one variant added to the `vocabulary!` invocation in `interface/src/node.rs` breaks all four projections at once — a compile-fail fixture requires an error from every one of them, so a projection that has quietly grown a wildcard arm is the one that fails to fail and is named. That is the observation no single projection can make. Each projection's own end-to-end observation is `E3-B06g` to `E3-B06j`'s, the authored-against-projected equivalence is `E3-B06f`'s, and the four-projection demonstration from one unmodified application stays `E3-P04`'s; none of the three is restated here.
  *needs:* E3-B06a, E3-B06b, E3-B06c, E3-B06d, E3-B06e, E3-B06f, E3-B06g, E3-B06h, E3-B06i, E3-B06j, E3-B06k, E3-B06l, E3-B06m (its decomposition; the two decisions and the scene graph this line used to name are now on the four subtasks that read them, which name the two lines of the graph this layer actually needs rather than all twelve)
```

`E3-B07` — the second half moves to `E3-B07d`, which is the whole value of
splitting this task: `E1-B15` closed, so the state-tree half is work this project
can do today and the frame-rate half is a measurement on a machine nobody has:

```
  *exit:* under 2x overload the frame rate holds. *The quality reduction is visible in the state tree, per frame* is `E3-B07d`, which needs no machine, because `E1-B15` closed and every component publishes a tree.
  *needs:* E3-B07a, E3-B07b, E3-B07c, E3-B07d, E3-B07e, E3-B07f, E3-B07g, E0-D10, E0-P18 (the two external ids are this line's own, not a child's: a frame rate holding is a time on a named machine)
```

`E3-P01` — the one swap rather than a move. *The rig reproduces a known
measurement on a conventional machine within its own error bar* is calibration
word for word, `E3-P01e` is the line calibration is on, and one observation
cannot close two tasks. The parent takes instead the integration none of its six
subtasks observes on its own, and it asks for **nothing this line did not already
ask**: a rig that reproduces a measurement is a rig that has been assembled and
run end to end, which is the smaller half of what was written here:

```
  *exit:* the rig is assembled and one capture runs end to end on it — the injector fires, the photodiode captures, and the digitiser records a trace at the error bar `E3-P01c` stated, over `E3-P01d`'s path with no software timestamp in it. Calibration is `E3-P01e`'s exit; it was this line's, and one observation cannot close two tasks.
  *needs:* E3-P01a, E3-P01b, E3-P01c, E3-P01d, E3-P01e, E3-P01f
```

*Landed when:* `cargo xtask todo E3` prints `ready to start — 6 task(s)`,
`waiting — 81 task(s)` and `done 5 · standing 0`. Seven means a `needs:` line was
missed; nine means all three of `E3-B02`, `E3-B03` and `E3-P01` were.

### Step 8 — the contradiction that needs an RFC

`E3-B03`'s exit asks that a corpus of scripts and directions render *correctly
against reference images, in CI*. `E3-P05`'s asks that *no test in the tree
compares images*. Both are defensible — a rasteriser's artefact is pixels, and an
interface assertion that reaches for a screenshot is exactly the brittleness
`E3-P05` exists to end — and they cannot both be exits. The narrowing is one
sentence in whichever loses, and because it reverses something already written
down it needs an RFC, from `docs/rfc/0000-template.md`, whose *What would reverse
this* section is the one that matters. `E3-B03j` is written to cite that RFC by
number, so this is the correction most worth making before anybody starts
`E3-B03`: the alternative is discovering it while writing the test.

This step is not blocked by any other and does not block steps 1 to 7.

*Landed when:* `E3-B03j`'s exit names a real RFC number.

### Step 9 — `E3-R01`'s exit is `E3-P01`'s observation

*The rig's method is documented well enough for a third party to build one* is a
statement about the rig's artefact, not about release 0.4. `TODO.md` has recorded
this same defect three times — on `E0-B12`, on `E1-B01` and on `E1-B05`, each
time as *one criterion belonging to two tasks means one of them is always lying
about its state* — and this is the fourth. `E3-P01f` is where that exit belongs
and now carries it. What `E3-R01` is owed instead is an exit about the release:
the three contents it names, published, with the parity result whichever way it
landed.

*Landed when:* `E3-R01`'s `*exit:*` no longer contains *documented well enough
for a third party to build one* and `E3-P01f`'s does — after which no two
`*exit:*` lines in E3 share a sentence, which is `spec.md`'s check 3.

### Step 10 — the line that would make `E3-00`'s exit observable

A proposal rather than an instruction, and the only step that asks for code.
Nothing in this tree observes *an `XL` without a decomposition*: `grep -n 'XL'
xtask/src/main.rs` returns nothing, and after step 7 a reader could delete all
seventy-two lines and `cargo xtask verify` would stay green. `E3-00`, `E4-00`,
`E5-00` and `E6-00` all carry that exit and nothing observes any of them. Under
`CONTRIBUTING.md`'s table it is an R01 row — *name the mechanism, not the
intention* — sitting honestly in the `review` column, which that table says is a
plan.

The mechanism needs no new field: `Task` already carries `size`, `needs` and
`epoch`. Two things about where it goes are not free, and an earlier draft of
this step got both wrong.

**It goes in `lint_all`, not in `todo`.** `verify()` (`xtask/src/main.rs:11195`)
runs `lint_all`, `test`, `run`, `orders`, `cores`, `panic_path`, `trace_check`,
`sim_check` and `sim_join`, and `lint_all` is a named list of lints —
`lint_determinism`, `lint_licensing`, `lint_unsafe`, `lint_percpu`,
`lint_mutations`, `lint_claims`, `lint_units`, `lint_callbacks`,
`lint_claim_owners`, `lint_manifests`, `generation::fixpoint`, `lint_components`,
`lint_datapath` and the reversal-gap check. A rule inside `todo` is a rule
`verify` never reaches, so it could not go red on anything.

**And it applies only to an epoch that has already been decomposed**, or it is
red on the day it lands. `E4-B01`, `E5-B01`, `E5-B02` and `E6-B01` are `XL` with
no decomposition today (`TODO.md` lines 1346, 1407, 1410 and 1474), and three of
them are protected by `E4-00`, `E5-00` and `E6-00`, which are open and whose
whole content is that those lines are not decomposed yet. The rule that is
actually `E3-00`'s exit is conditional on that box:

```
- [ ] **E3-B08** `S` `cargo xtask lint` refuses an undecomposed `XL` in an epoch whose decomposition task is done.
  *exit:* a lint in `lint_all` — which is the list `cargo xtask verify` runs — fails on any task sized `XL` whose `needs:` names fewer than five ids extending its own id, in an epoch whose `E<n>-00` is `[x]`, with a fixture in `xtask` that makes it fail. It is green on E4, E5 and E6 the day it lands, because `E4-00`, `E5-00` and `E6-00` are `[ ]` and their `XL` lines are what those tasks are for. `CONTRIBUTING.md` gains a row moving this rule out of the `review` column, which is a change to a written table and carries the RFC that says so.
  *needs:* E3-00
```

Two things about it are the originator's call and neither changes the argument.
The id: `E3-B08` puts a tooling task in an interface epoch, and the alternative
is an always-on item in `A-05`'s shape — a `*mechanism:*` line and a
`*cadence:*` — which fits the rule's nature better and never closes. And the
timing: if this lands with steps 1 to 7, `cargo xtask todo E3` reports **7**
ready rather than 6, and the seventh is `E3-B08`. The decomposition's own six do
not move either way.

*Landed when:* with `E3-00` ticked, deleting the `*needs:*` line from `E3-B01`
makes `cargo xtask verify` go red and naming that line; adding an undecomposed
`XL` to E4 leaves it green, because `E4-00` is open.

---

## And one thing that is not a step

`E0-B15` says cross-core doorbell delivery is unproven and that it belongs with
the component that will actually sleep on a doorbell. The compositor is that
component — `E3-B01g` is the first thing in this system that parks waiting for a
peer's commit — so a clause of an E0 task closes inside E3, which the graph has
no way to express and which will otherwise be rediscovered by whoever writes the
doorbell test. It is worth a sentence on `E0-B15`'s line whenever somebody is
next in it. It is not in the *Handoff* because moving a clause of a task out of
its own epoch is a decision, not a correction, and `intent.md`'s first open
question is where it is asked.
