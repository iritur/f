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
| 0012 | `E2-D01` | An update is a generation swap, and the root is the attestation | accepted |
| 0013 | `E0-B14` | Every component publishes a state tree | accepted; reversal condition read against `E2-P01`'s sweep on 2026-09-07 and not met |
| 0058 | `E2-D02` | A mutable extent is a second object kind, and not a unification | accepted |
| 0059 | `E2-D03` | A collector is three invariants and a batch-class consumer | accepted |
| 0060 | `E2-D05` | A publish is a barrier sequence, and atomicity is not free | accepted |
| 0061 | `E2-P02` | A boundary is a predicate over a window of content, and not a distance from the last cut | accepted; superseded in part by 0062 |
| 0062 | `E2-P02` | A period below the minimum starves every rule, so the starved clause carries periodic content and is measured | accepted; superseded in part by 0064 |
| 0063 | `E2-D04` | A transfer is declared in the manifest, and quiescence is asserted by its occupant | accepted |
| 0064 | `E2-B09` | A starved run is crossed and not inhabited, so the flat clause is scoped by a window and not by a point | accepted |
| 0065 | `E1-B15` | A component declares its state tree, and a spawn refuses one that does not | accepted |
| 0066 | `E2-B05` | The assembler lands above the frame with no caller, rather than inside it with one | accepted |
| 0067 | `E2-B05` | A driver declares the part it binds, and the topology never inherits a scan order | accepted; written as 0065 in its own worktree and renumbered here at the merge |

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
number, because each computed its own without the other's fields in it. The
lesson is filed here rather than in either entry: **two parallel worktrees that
both touch one numbered registry will both take the same number**, and the only
defences are to allocate the number before the branch or to expect the merge to
resolve it. This tree does the second, and this row is what that looks like.
