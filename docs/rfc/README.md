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

Not an index, and deliberately not. Sixty-one entries have file names that are
already their titles, and a table of all of them would be a change to sixty-one
closed decisions' evidence with nothing gained — the same answer E1's plan gave
about a fourth crate. What a table *is* worth is the rows a reader trips over:
a number that was a gap for two epochs, and a decision whose task line is not
where anybody would look for it. Those are what this section holds. It grows one
row per entry that needs one, and an entry that needs none does not get a row.

| RFC | Task | Title | Status |
|---|---|---|---|
| 0012 | `E2-D01` | An update is a generation swap, and the root is the attestation | accepted |
| 0058 | `E2-D02` | A mutable extent is a second object kind, and not a unification | accepted |
| 0059 | `E2-D03` | A collector is three invariants and a batch-class consumer | accepted |
| 0060 | `E2-D05` | A publish is a barrier sequence, and atomicity is not free | accepted |
| 0061 | `E2-P02` | A boundary is a predicate over a window of content, and not a distance from the last cut | accepted; superseded in part by 0062 |
| 0062 | `E2-P02` | A period below the minimum starves every rule, so the starved clause carries periodic content and is measured | accepted |

**0012 is why this directory runs 0011, 0013.** `TODO.md` reserved the number
for the generation swap when the E2 line was written, and then E0 and E1 wrote
forty-six entries around the hole. A reader arriving at the gap has no way to
tell a reserved number from a withdrawn one, and the difference matters: a
withdrawn entry is a decision somebody unmade, and this was a decision nobody
had made yet. RFC 0012 says so in its own *Context*; this row is where it is
visible without opening the file.

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
