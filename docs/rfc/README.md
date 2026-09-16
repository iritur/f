# Decision record

Four documents of reasoning exist in `docs/design/`. They are rewritten as the
design moves, which means the *reasoning* survives and the *reversals* do not.
This directory holds the reversals.

The rule: any decision that changes what is already written down, or that a
future contributor would otherwise re-litigate, gets an entry. Entries are
append-only. A superseded RFC is marked superseded, never edited away.

The first four are backfilled from the design conversation that produced the
documents, because those reversals were real and the reasoning behind them was
otherwise only implicit in the current text.

## Reserved numbers

Not an index, and deliberately not. Sixty-two entries have file names that are
already their titles, and a table of all of them would be a change to sixty-two
closed decisions' evidence with nothing gained — the same answer E1's plan gave
about a fourth crate. What a table *is* worth is the rows a reader trips over:
a number that was a gap for two epochs, and a decision whose task line is not
where anybody would look for it. Those are what this section holds. It grows one
row per entry that needs one, and an entry that needs none does not get a row.

| RFC | Task | Title | Status |
|---|---|---|---|
| 0012 | `E2-D01` | An update is a generation swap, and the root is the attestation | accepted; the open question below, from `E2-P07`'s refusal boot, was answered on 2026-09-07 by reordering the boot |
| 0013 | `E0-B14` | Every component publishes a state tree | accepted; reversal condition read twice on 2026-09-07 — against `E2-P01`'s sweep and against `E2-P05`'s comparison — and not met by either |
| 0058 | `E2-D02` | A mutable extent is a second object kind, and not a unification | accepted |
| 0059 | `E2-D03` | A collector is three invariants and a batch-class consumer | accepted |
| 0060 | `E2-D05` | A publish is a barrier sequence, and atomicity is not free | accepted |
| 0061 | `E2-P02` | A boundary is a predicate over a window of content, and not a distance from the last cut | accepted; superseded in part by 0062 |
| 0062 | `E2-P02` | A period below the minimum starves every rule, so the starved clause carries periodic content and is measured | accepted; superseded in part by 0064 |
| 0063 | `E2-D04` | A transfer is declared in the manifest, and quiescence is asserted by its occupant | accepted; reversal condition read against `E2-P08`'s sweep on 2026-09-07 and not met |
| 0064 | `E2-B09` | A starved run is crossed and not inhabited, so the flat clause is scoped by a window and not by a point | accepted |
| 0065 | `E1-B15` | A component declares its state tree, and a spawn refuses one that does not | accepted |
| 0066 | `E2-B05` | The assembler lands above the frame with no caller, rather than inside it with one | accepted |
| 0067 | `E2-B05` | A driver declares the part it binds, and the topology never inherits a scan order | accepted; written as 0065 in its own worktree and renumbered here at the merge |
| 0068 | `E0-P18` | A core that does not answer is held, and not fatal | accepted |
| 0069 | — | A claim states the kind of evidence behind it, and a kind may not stand in for another | draft; no task line owns it, which is the reason for the row — written from a reading of another project's validation rule on 2026-09-10, ahead of the lint that would make it mechanical |
| 0070 | `E0-P18` | An arriving core is given the bits it needs, not the register | accepted |
| 0071 | `E1-B16` | The shared half of three supervisors is one module, and `Reported` is not in it | accepted; the number was allocated on a commit of its own before the diff, because six separate plans for separate work each computed 0071 as next free — `docs/postmortem/0001`'s first failure waiting to happen again. The entry keeps the two sentences the implementation proved wrong |
| 0072 | `E1-B15` | A runtime is a component, so it publishes a tree, so it needs the pages to do it in | accepted; the entry carries the ceiling — the init and runtime chain is its text reservation plus nine pages and has to stay below `SPAWN_GUARD`, so four is what it is and seven is the most it could be |
| 0073 | `E1-B05` | A supervisor is a component, and the frame answers its ring from a second server | accepted; the number was allocated on a commit of its own before the diff, for 0071's reason. The entry records that the wall `E1-B05` states — `Mapping::adopt` is `unsafe`, so a `user/` crate cannot drive a control ring — was demolished by RFC 0037, and that the real blocker is three things that are one thing: there is no supervisor component |
| 0074 | `E0-P18` | The ring-3 field of `IA32_STAR` carries its privilege level | accepted; closes the second defect RFC 0070 left open — the instruments it landed named it on the next boot, and the fix is two bits the emulator's `sysret` had been supplying for free |
| 0075 | `E1-B05` | Scheduling is a job, and ownership is not scheduling | accepted; refuses the obvious join — an `Instance` producing a `Prepared` — because `reap` returns frames to the allocator and an occupant's pages are owed to its account, so the convenience method is a double free the type system would not name. Two ownership records, one scheduling interface |
| 0076 | `E1-B05` | A supervisor is told when it is started, not while it runs | accepted; the frame cannot write pending state into a running component's table, because that table is its core's — so a supervisor is told by notices already on its ring when it is scheduled. Refuses two alternatives on record: moving where a place-death pends, which trades RFC 0008's structurally-guaranteed *granted then peer gone* for a documented one, and a fifth shared word, which is affordable only when something cannot be done without it. This is what lets `policy::decide` leave the frame |
| 0077 | `E3-D01` | The semantic vocabulary is closed, and a canvas that declares nothing is a defect | accepted; twenty-two roles with no `Other`, no string role and no `#[non_exhaustive]`, because the web's annotation vocabulary failed on ambiguity rather than on size. Closedness is structural rather than tested: a `vocabulary!` macro emits the enum, `ALL`, `COUNT` and every table from one list, so there is no second place a variant can be written — the two tests that tried to guard a hand-written array were both defeated, the second by `Role::Other => {}`, and deleting them was the repair. The canvas loophole is closed by `Defect::EmptyCanvas`. Departs from section 11 by one field, `parent`, and says what would make that wrong |
| 0078 | `E3-D02` | A canvas declares its content, or it is an escape | accepted; the floor a self-rendering surface must clear to be admitted at all, with the timeline built to section 13's own falsification — an agent selecting the clip between 4.2 s and 6.8 s on track 3, and a screen reader describing it, neither seeing a pixel. Times are integers with the scale in the name because this tree has no floats, which makes a timeline's time base a decision rather than a formatting detail. Honest that the incentive question underneath — whether developers will declare structure they do not declare today — is a usability finding this RFC does not settle |
| 0079 | `E3-D03` | A theme may not make an interface unreadable, and it is clamped rather than refused | accepted; three categories — themeable, range-checked, and out of a theme's reach — with the contrast arithmetic in fixed point because the two architectures do not agree on floats. `every_ground_admits_a_readable_ink` sweeps every integer luminance rather than a stride over the colour cube, which is what caught the true minimum of 4 582 at `#008909` lying between two swept greys. The stroke's floor is derived rather than chosen and no longer shared with spacing: a theme with no space is dense, a theme with no stroke has deleted a rule the compositor was told to draw |
| 0080 | `E3-D04` | The fallback ladder is decided before it is needed, and a rung that declares nothing is not a rung | accepted; four rungs named in order with each rung's cost registered as `claims/0033` rather than asserted in prose, and the ladder bound to the registry by `include_str!` rather than by a copied array. Both reversal conditions compare a rung to a rung on a machine that started at one of them — the earlier threshold-to-threshold form deleted the hybrid while it was winning, and refuted the floor on speed it was never offering. A ladder decided after it is needed is decided under the pressure of a machine that cannot run the top rung |
| 0081 | `intent/0011` | The frame draws the log when nothing else can | accepted; a fixed-grid character console inside the frame, because the machine F boots on outside the emulator has no output but a serial port and the boots that went wrong produced their evidence before any component existed. The surface is write-only — the text lives twice in cached memory and only differing cells are drawn, so a scroll touches no pixel. Answers RFC 0076's refusal of a fifth shared place rather than dodging it: the policy-conforming `PerCpu<Console>` is eight grids for one screen, and eight cursors over one surface is wrong by construction rather than under a race. Nothing reads the console back, which is what makes it unlike 0076's case. The font is typed rather than imported because `LICENSING.md` has no category for permissively-licensed imported data |
| 0082 | `E3-B03a` | The shaper is imported, and there is no crate for the permissive tree to link | accepted; the first decision the licence boundary has actually been asked to hold. Imported source arrives under `third_party/`, is built into a component image of its own, and is reached from the permissive tree over a ring. **The basis the first draft gave for that was retracted in review rather than quietly softened**: it claimed the shaper was reached over a ring *and by no other route because there is no other route to take*, and a reviewer built the thing that paragraph called impossible. What survives is narrower and is stated as such — what the permissive tree *names* is an `image` path in a `manifest.toml`, and a Rust file cannot `use` an image — and the rest is now held by a lint rather than by the absence of a route: `lint_licensing` reads the dependency *graph* and not only the spelling, because a permissive crate taking the import as a path dependency under a name of its own mentions `third_party` nowhere in its Rust source and passed the old check. The exit is **not** closed and the RFC says which clause and why: `third_party/` carries no entry, so the two files that would make the route exist have not landed. What is imported is drawn by a rule rather than by convenience: what is specified by somebody else's file format and measured by no claim here is imported; what a conformance corpus can be run against, or what a claim rests on, is written |
| 0083 | `E3-B06a` | A vocabulary version is negotiated before a tree flows, and an unknown role is not a value a projection can hold | accepted; RFC 0077's fourth reversal condition dated itself to the day the semantic tree got an entry format, and this is that day. Negotiation is in band, by a `DeclareVocabulary` entry that must come first — not a field in `ChannelHeader` and not a feature bit, which is part of the decision rather than an accident of scope. It carries a **return leg on its own completion**, so both sides hold an agreement each of them computed rather than one peer announcing and the other believing; that is what `ChannelHeader::negotiate` does one level up, and a handshake that suppressed its own completion would be a declaration, which is not RFC 0011's shape. Indices are append-only, and version 1's list is frozen by a digest over its role names in order — asserted **in a test** in `abi/src/semantic.rs` that reads `interface/src/node.rs` through `include_str!`, *not* at compile time; the `const` that would make a reorder a build failure is owed, and the RFC says where. A version ordinal is worth negotiating only if it names the same list on both sides: without the freeze, negotiation certifies that two builds agree while index 13 means `Command` on one and `Toggle` on the other. An unrecognised index is refused at the boundary rather than mapped, which is what keeps the vocabulary closed across a wire as well as within a build |
| 0084 | `E3-B01b`, `E3-B01d` | An exit narrowed by measurement is a reversal, and two of E3's are narrowed | accepted; the first RFC in this tree about the *spec* rather than the code. An exit is the sentence a task is accepted on, so narrowing one is a reversal and belongs here — a spec's description may be edited freely, its `exit:` may not, and the narrowing is written into the spec with this number beside it because `intent/0012`'s plan is a paste-ready handoff and a module comment that disagrees with a spec line is a false line waiting for a paste. `E3-B01b` loses *the way `Role` already is*: a seventh kind added end to end broke exactly one build, and `Role` does not achieve the wider sentence either — it has consumers that happen to match exhaustively, which is a fact about who has written code and not a property of the type. `E3-B01d` gains a clause about the ring: a slot whose write is not visible reads as whatever occupied it last, a previous frame's entry decodes, and the commit seals over a frame that is part this one and part the last. Closing that is a per-entry submission sequence in `abi/`, not a change to the commit path, so the narrowed sweep requires a lying ring to produce third scenes rather than asserting none exist |

**0012 is why this directory runs 0011, 0013.** `TODO.md` reserved the number
for the generation swap when the E2 line was written, and then E0 and E1 wrote
forty-six entries around the hole. A reader arriving at the gap has no way to
tell a reserved number from a withdrawn one, and the difference matters: a
withdrawn entry is a decision somebody unmade, and this was a decision nobody
had made yet. RFC 0012 says so in its own *Context*; this row is where it is
visible without opening the file.

**0019 was never allocated, and this sentence is the whole of that fact.** The
directory runs 0018, 0020, and unlike the hole one row up there is nothing
behind this one: no commit in this repository's history has ever touched a path
matching `docs/rfc/0019-*`, no entry's *Affects* or *Supersedes* names an RFC
0019, and no task line reserves it. The number was skipped once and nobody
noticed. It is written down because the only thing that distinguishes this hole
from 0012's is a search a reader should not have to run, and because one
coincidence makes the search likely: `claims/0019` *is* reserved —
`resident-bytes-per-unit-of-work`, by `intent/0006-state/spec.md` against
`E2-B08`, and RFC 0059 cites it by that number — and this tree runs two
registries that number their entries the same way. The number stays empty rather
than being reissued: entries here are read by number, and a 0019 written after
0063 would be a decision filed two epochs before it was taken.

**0060 had no task line at all, and that is why this section exists rather than
an index.** It landed inside `E2-B01`'s diff — the five blk opcodes a barrier
sequence needs, ordering rule 1 putting the wire first — and between the day it
landed and the day somebody read this file, nothing in the tree recorded that a
reversal had been paid: `lint-owed` reports reversal conditions, not owed RFCs.
The plan for `intent/0006-state` predicted the column would read *none*
permanently and treated this row as the whole mitigation. It reads `E2-D05`
instead, because the same bookkeeping that wrote this row gave the decision the
task line it was missing. A row that had to say *none* would still have been
worth more than an index.

**0061 is filed against a prove task, which is unusual and correct.** It is the
first entry here produced by a measurement rather than by a decision meeting:
`E2-P02` ran, the published re-chunking bound failed on 7 of 32 (seed, mixture)
pairs, and the RFC is the reversal that failure forced. Filing it under the
build task that will implement it would hide where the evidence came from.

**0062 is the second entry produced by a measurement, and the first produced by
another entry's own confirming run.** 0061 was accepted with a run owed against
it; that run confirmed the rule — the published bound on 32 of 32 pairs, the
tight clause on all 13 it applies to, the forced-cut fraction at 1.66% against a
reversal condition of one in ten — and falsified a sentence 0061 had written
about which of the bound's two clauses carries periodic content. So 0061 is
marked *superseded in part* rather than superseded: two sentences and one
requirement go, and the acceptance rule, the mask, the bound and
`RESYNC_BOUND_BYTES` all stand and are confirmed. A row is the only place that
distinction is visible without reading both files, which is the same argument
0012 and 0060 make one row up. An audit of that slice has since put the
effective sample beside the first of the three numbers 0062 confirms: the
published bound could have failed on 18 of the 32 pairs, the other 14 being
pairs whose starved allowance reaches past the edited object's own end, so
`agreed <= allowed` cannot be false there. Neither entry is edited for it —
entries here are append-only and what they reported is what they reported — and
the count lives in `claims/0017` as a thresholded row instead.

**0063 arrived with its task line already written, and needs a row for the
other half of what this section is for.** `E2-D04` named the decision when the
E2 line was written, so the column reads a task rather than the *none* 0060 was
about. What a reader trips over is the state of that task: the exit is three
clauses, the entry pays two — the schema is merged, `user/virtio-blk` declares
`in_place` — and the third, *and `E2-P08` swaps it*, waits on `E2-B06`, which
waits on `E2-B05`, which waits on `E1-B05`. So `E2-D04` stays open for two
builds after the decision inside it is closed, and a reader who follows this row
into `TODO.md` finds `[>]` with no way to tell a question nobody has answered
from one answered and waiting for a swap. `E2-D06` is the line that says the
decision itself is closed, and it exists for the *second* reversal in this entry
rather than the first: the manifest schema goes 1 to 2, which RFC 0030 priced as
a rebuild of every component file and `docs/manifest.md` refused outright, and
no task line in that file named it.

**That row now reads `[x]`, and the reversal condition behind it has been read.**
`E2-P08` swapped it on 2026-09-07: `cargo xtask swap` replaces `user/virtio-blk`
in place twice under sustained load, and `claims/0029` is the artefact — 0
operations redone in place beside 24 redone by the restart route, through one
client at one seed, which is 0063's central sentence measured rather than
argued. Its stated reversal is that same task *observing a dropped, doubled or
wrongly-answered operation across a swap*, and the run observed none of the
three with three negative controls in shipped source proving each counter can
move. So the decision closes and what stays open is its implementation in the
frame: no boot can put two generations of one component in front of the frame,
`SWAP_GAP` in `xtask/src/main.rs` is the checked line, and `E2-B06` is where a
reader meets it.

**0064 is the third entry on one bound and the first produced by a workload the
registry could not reach.** 0061 was falsified by a property test, 0062 by 0061's
own confirming run, and 0064 by `bench/src/bin/rechunk.rs` — which measured
1 138 541 bytes against a published 786 432 and reported it inside a green
`verify`, because the claim's reproduction command routed to the property test
and not to the bench. So the row that matters here is not only the decision but
what it says about this directory's neighbour: a threshold in `claims/` that no
command compares against is a number in a file, and the same defect had been
raised against `claims/0018` one wave earlier. 0062 is marked *superseded in
part* for one assertion in its universal form — an edit outside a starved run is
not always carried by the flat clause, because the run can be in front of the
edit rather than under it — and its theorem, its arithmetic and its four
thresholds stand. The column reads `E2-B09` because that is the build whose
measurement produced it; `E2-P02` is where the strict reading still lives, and
0064 says in as many words that it must stay strict.

**An open question against 0012, and its answer.** The question is left standing
below exactly as it was filed, because a question deleted once it is answered is
a question the next reader has to ask again from nothing.
`cargo xtask rollback`'s `refuses()` boots a command line carrying
`f.root=0000…0` — sixty-four zeros, which is a well-formed token that no module
can fold to, and the boot is expected to end in the frame's refusal. It does.
But the refusal is `kernel::generation::report`'s, five hundred lines into
`kernel/src/main.rs`, and the frame has already published its identity by then:
`Identity::generation` answers **1** for any `Some(root)`, so the state tree
carries counter 1 beside four zeroed root words. That is read out of the boot
rather than inferred from the source: the log of that boot says
`generation    0000…0000 selected as publish 1`, then
`state 42 counter = 1` with `root0` through `root3` at zero, and only then
`FAIL: the generation: no module this machine was offered folds to the root it
was asked for`. A reader following the protocol 0012 fixes — load the counter,
and if it is non-zero load the four root words — reads a machine attesting to a
root of zeros on a boot that is about to refuse.

The reserved value in that RFC is the **counter**, and the sentence it is
reserved by is *the value the format already reserves so that a zeroed block is
never a generation*. This is the other half of that sentence and 0012 does not
say which way it goes: whether an all-zero root is a token the grammar should
refuse in `abi/src/boot.rs`, whether `publish_identity` should happen after
selection rather than before it, or whether a published root the frame has not
yet found a module for should carry the counter zero it carries when nobody
named a generation at all. Each of the three costs something different — the
first is a wire refusal, the second reorders a boot, the third makes *told
nothing* and *told something impossible* indistinguishable in the tree — and the
decision belongs to whoever owns 0012 rather than to the run that noticed.
Recorded on 2026-09-07, with `docs/postmortem/0001` carrying the same paragraph
from the merge's side; nothing in wave 5 depends on the answer, and
`claims/0030`'s `unknown_roots_refused` row measures the refusal and not this.

**Answered on 2026-09-07: the second of the three, and the boot was reordered.**
`kernel::generation::report` now runs *before* `state::Tree::publish`, so a
machine that cannot be the generation it was named ends the boot having
published nothing — no tree and no identity — and the log of the refusal boot no
longer carries `counter = 1` at all. The other two were refused for what they
would have cost. Refusing an all-zero root in `abi/src/boot.rs` puts a *value*
into a grammar that until now only spelled a width, and it answers a narrower
question than the one asked: `f.root=` with any sixty-four digits no module
carries has the same defect, and zeros are only the case somebody happened to
write. Publishing counter zero for a root the frame has not resolved makes *told
nothing* and *told something impossible* the same state in the tree, which is
the one distinction the refusal exists to draw.

This is an answer to a question 0012 left open and not a reversal of anything it
decided, so it carries no RFC of its own: the reserved-counter sentence already
says a zeroed block is never a generation, and this is that sentence applied to
the instant at which the counter stops being zero. *What would reverse it:* a
frame that must publish before it can select — a store mounted on the boot path,
which is the same condition `FRAME_KEY`'s own reversal names — at which point the
publish comes first and the counter has to carry the distinction instead.

**0013 gets a row because its reversal condition fell due and somebody had to
say what reading it.** That entry's *what would reverse this* names E1's seeded
fault sweeps and **E2's crash-consistency work** as the evidence that could
reverse per-node atomicity, and `intent/0006-state/spec.md` says in as many
words that the section is read again when `E2-P01`'s sweep has run. It has run —
108 280 cuts, `claims/0025` — and the condition is **not** met. The evidence 0013
asks for is a class of bug whose signature was present in the tree but only in
the relationship between two nodes read at the same instant, and this sweep
produces the opposite: it cuts a machine, rebuilds the device, and mounts a
*quiesced* one, which is precisely the case 0013 says per-node atomicity covers.
Nothing in 108 280 cuts was missed for want of a consistent cut of a live
machine, because no cut was taken of a live machine at all. So the entry stands
unedited — entries here are append-only — and what the row records is that the
question was asked rather than left to lapse. What could still fire it is
`E2-P05`, which compares two whole-system trees under load and is the task that
would read two nodes at one instant; that reading is owed there and not here,
and `E2-P05` waits on `E1-B15`.

**That second reading has now been taken, and it is also not met.** `E2-P05`
landed on 2026-09-07 — `claims/0031`, 64 of 64 injected divergences localised to
the exact node — and the reason it does not fire 0013's condition is the same
shape as the sweep's: `sim/src/whole.rs` folds each component's tree **quiesced**
and one component at a time, so no node in any of those 64 injections, and none
in the disarmed control run, was read at the same instant as another. The task
that *would* read two nodes at one instant is a comparison taken of a machine
under load, and this tree cannot take one yet for a reason that is not the
comparison's: the frame lays out a child's header and schema and writes no word
after that (RFC 0065), so a boot's component subtrees are constants.
`compare::WHOLE_SYSTEM_GAP` is the checked declaration of that, printed on every
green run, and it is where the third reading of this condition will come from.

**0067 was 0065 too, and the collision is what these three rows are for.**
`E2-B05` and `E1-B15` were built in parallel worktrees, neither able to see the
other, and both needed the manifest. Both wrote an RFC and both called it 0065;
both bumped `schema` 2 → 3. Nothing warned them, because the only thing that
would have is a number allocated in a place both could read, and a worktree is
by construction not that place. The merge is where it surfaced.

It was fixed rather than preserved, and the distinction is worth stating because
this directory is append-only. Append-only protects *decisions*: an entry that
was accepted stays readable at the number it was accepted under, so that a
reader following a citation from 2026 lands on what the citation meant. A number
assigned twice is not a decision — it is two files that cannot both answer to
the name they claim, and a citation to "RFC 0065" that resolves to either one is
worth nothing. So one of them moved. **0065** kept *a component declares its
state tree*, because it had twenty-seven references in the tree against the
other's sixteen and the cheaper edit is the one that leaves more citations
alone; the driver entry became **0067**. 0066 sat between them, is `E2-B05`'s
other entry, and did not move — its one citation of "RFC 0065" now reads 0067,
which is the only forward reference in this directory and is a consequence of
renumbering the later half of a pair rather than the earlier.

What the two decisions did *not* do is collide in substance. Both wanted a
manifest array and neither supersedes the other: schema 3 carries `[[device]]`
and `[[state]]` together, each count byte took one of the record's three
reserved bytes, and `abi::manifest::Record` is 2696 bytes — neither worktree's
number, because each computed its own without the other's fields in it.

The number in this directory is what surfaced, and it is not what the collision
cost. **Both writers had stamped their count at byte 101.** Each had taken *the
first* of the record's three reserved bytes, which is the obvious choice and the
only one either could see, so two manifests compiled by two branches would have
answered one offset with two different meanings — a component's device count
read as its state-node count and back again — with every test on both branches
green, because neither branch contained the other's field to disagree with. That
is a silent corruption in the one place this project has decided a corruption
must never be silent, and no lint could have found it: `lint-manifests` checks a
manifest against the record it was built from, and both were self-consistent.
The merge is the only reader that had both. `devices` keeps 101, `state_nodes`
takes 102, two of the three reserved bytes are spent and one is left, and
`every_structural_lie_is_refused` now breaks byte 0 because that is the only
reserved index there is.

The lesson is filed here rather than in either entry: **two parallel worktrees
that both touch one numbered registry will both take the same number**, and the
only defences are to allocate the number before the branch or to expect the
merge to resolve it. That holds for the reserved *bytes* of a wire record
exactly as it holds for the numbers of the entries in this directory, and the
byte is the more dangerous of the two because a doubled RFC number is loud and a
doubled offset is not. This tree does the second defence, and this row is what
that looks like.

**It happened again one wave later, so the line was earned and taken.** The
paragraph below said that if it happened a second time the rule's *twice* was
satisfied; wave 5 is that second time, in a form the first did not have. Four
worktrees split one *grammar* rather than one registry: `E2-B07` wrote the
reader of `f.root=`/`f.frame=` and made every boot carry both, `E2-P07` wrote
the composer and composed only the first — in three commands and in every
`menuentry` `cargo xtask generation --install` writes — and the merged frame
refuses half a declaration by name. Every tree was green, every conflict was a
keep-both, and the merged tree could not boot. `CLAUDE.md` now carries the line;
`docs/postmortem/0001` is the incident, including the one defect of the five
whose first reader would have been somebody's hardware. The paragraph below
stands unedited, because what it decided was right for what it could see and the
condition it set is exactly the one that fired.

`CLAUDE.md`'s *Common mistakes* did **not** gain a line, and the judgement is
recorded rather than left to be re-taken. That section's rule is *added when the
same mistake happens twice*, and two agents making one mistake simultaneously —
neither able to learn from the other — is one occurrence with two symptoms
rather than two occurrences. A line saying *do not take a registry number in a
worktree* would also not have prevented it: both agents checked the tree they
could see, and the tree they could see was complete. If it happens a second
time, that is the twice the rule asks for and the line is earned. `TODO.md`'s
`E1-B15` line carries the same paragraph from the manifest's side, and the two
are meant to agree.
