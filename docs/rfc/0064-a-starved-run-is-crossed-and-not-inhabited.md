# RFC 0064: A starved run is crossed and not inhabited, so the flat clause is scoped by a window and not by a point

- Status: accepted
- Date: 2026-09-07
- Affects: `claims/0017-bytes-rechunked-per-byte.toml` (the scope of the four
  chunked-kind rows, four new rows beside them, the extent-kind rows `E2-B09`
  owed it, and `[workload] path`, which still named a file that measures the
  numerator only); `bench/src/bin/rechunk.rs` (the classifier and what it
  prints); `blob/tests/chunker.rs` (eight rows printed under the names the claim
  registers them under — **no assertion in that file is changed, weakened or
  removed**); `xtask/src/main.rs` (`Route::Rechunk`, and
  `bytes-rechunked-per-byte` leaving `Route::Chunker`); `docs/rfc/README.md`'s
  status row for RFC 0062; `E2-B09` and `E2-P02`.
- Supersedes, in RFC 0062 and nowhere else, **one assertion in its universal
  form**: *Consequences*' third replacement assertion, *"a pair whose edit is
  **not** inside a starved run must be carried by the flat clause. The mixture
  name decides nothing; the candidate gap the edit sits in decides everything."*
  The first sentence of that is false as stated and is measured false below; the
  second is true and is what this entry sharpens — the candidate gap the edit
  sits in decides less than everything, because a gap the edit does *not* sit in
  and is about to walk into decides the rest. **RFC 0062 is not edited by this
  entry and is not superseded as a whole**: its theorem, its `P(empty)`
  arithmetic, its account of the periodic class, its four thresholds and its
  reading of `E2-P02`'s exit all stand, and the run below re-measures three of
  them unchanged. The marking is `docs/rfc/README.md`'s *Reserved numbers*
  table, where RFC 0062's row becomes
  `accepted; superseded in part by 0064` with a row for this entry beside it.
  RFC 0061 is untouched: this entry is an argument that its published bound was
  read too narrowly, not that it was wrong.

## Decision

The published bound does not move, and neither does the number in it. What
moves is where the second clause is *evaluated*, and it moves from a point to
an interval, which is where RFC 0061 put it in the first place.

RFC 0061's sentence is *"they agree ... in no case later than
`X + L + RESYNC_BOUND_BYTES`, **except across a starved run**"*. Both this
workload and `blob/tests/chunker.rs` implemented *across* as *inside*: they
asked `starved_end(candidates, at)` — is the edit's own candidate gap a starved
one — and applied the flat clause to every edit for which the answer was no. An
edit sitting in perfectly cuttable content a hundred kilobytes ahead of a 645 KiB
candidate gap is outside every starved run by that test, and its two boundary
streams walk into that run, force a cut in it at two different offsets, and are
separated by exactly the mechanism RFC 0062 proves. The clause covers it. The
point predicate does not see it.

So the flat clause is scoped by the window it allows itself:

> An edit of `L` bytes at `X` re-chunks at most
> `CHUNK_MAX_BYTES + RESYNC_BOUND_BYTES` = **786 432 bytes**, independent of the
> object's size, **provided no starved run meets `[X + L, X + L +
> RESYNC_BOUND_BYTES]`** — a starved run being RFC 0061's, a maximal run in which
> no two consecutive candidates are closer than
> `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` = 245 760 bytes. Where a starved run does
> meet that window, the bound is its second clause: agreement by the end of that
> run plus one chunk, `max(X + L + RESYNC_BOUND_BYTES, run_end +
> CHUNK_MAX_BYTES)`, which scales with the object and is what RFC 0058's second
> object kind exists for.

Three properties of that statement are what make it a bound rather than an
excuse, and each is a row in `claims/0017`:

1. **The predicate is prior.** It is a function of the content and the edit —
   the candidate sequence is a pure function of the last sixty-four bytes at each
   position, so both halves are computable before the chunker runs. It is not
   *the edits that passed*, which is the fitted number this registry's own
   `[diagnosis]` tells the next reader not to write.
2. **The window is the clause's own.** `RESYNC_BOUND_BYTES` and nothing else: the
   flat clause promises agreement inside that interval, so the content it can
   promise it over is content with no forced cut inside it. A shorter window
   would enlarge the sample, and enlarging the sample is exactly the move that
   would make this fitting.
3. **The excused class is asserted, not waved through.** The second clause is
   checked on every edit the first excuses — `rechunk_edits_exceeding_the_second_clause`,
   `max = 0` — because a class excused from one clause and asserted under none is
   a class that cannot go red. Measured 0 over the 1049 excused edits of 1280.

And the price is stated rather than buried: on the drawn workload the flat clause
now covers **231 of 1280 edits** across five mixtures — 117 of the 640 at 8 MiB
and 114 of the 640 at 128 MiB — where three of those five mixtures are content
RFC 0062's arguments already condemn and contribute nothing. Of the 128 edits
per size on the *uniform* mixture, 99 and 102 are clear, against the 94 the
arithmetic below predicts. That is the number a reader should hold against this
design: roughly one edit in four, on content this chunker is *good* at, is an
edit whose cost the flat clause does not bound.

## Context

**The measurement, and it was taken twice.** `cargo run --release -p f-bench
--bin rechunk` at claim 0017's own geometry — 8 MiB and 128 MiB objects, 4096-byte
writes, 64 writes per object, seeds 1 and 2, five mixtures:

```
bytes_rechunked_per_edit_chunked_small       1138541
  threshold                                   786432
  edits the bound was exercised on            158 small, 160 large
  of the unstarved edits, violating that bound  5 small,   0 large
```

Every one of the five is on `concatenated` content at 8 MiB, and the workload
prints each as it happens:

```
! concatenated seed 1 at 8388608 edit at 365496 re-chunked 1138541 (257984 -> 1396525);
    starved_end 356560, widest candidate gap crossed 645661
! concatenated seed 2 at 8388608 edit at 5581675 re-chunked 1036267 (5545559 -> 6581826);
    starved_end 5571549, widest candidate gap crossed 576301
```

Three edits at seed 1 — at 269 513, 365 496 and 429 909 — produce the identical
1 138 541, which is the first thing worth noticing: they are three different
edits with one answer because they are three edits *in the same chunk region*
walking into the same 645 661-byte candidate gap, and what the number measures is
the gap and not the edit. At seed 2 the same shape at 576 301. `starved_end`
evaluated at each edit returns a position at or behind the edit — 356 560 against
an edit at 365 496 — so every one of the five is *outside* a starved run by the
point test, and the flat clause was applied to all five.

**The cause is the sentence, not the chunker.** Re-run with the second clause
evaluated over the window instead of at the point, same command and same seeds:

```
bytes_rechunked_per_edit_chunked_small        345390   (threshold 786432)
bytes_rechunked_per_edit_chunked_large        262144
rechunk_edits_clear_of_a_starved_run_small       117
rechunk_edits_clear_of_a_starved_run_large       114
rechunk_edits_near_a_starved_run_small            41
rechunk_edits_near_a_starved_run_large            46
bytes_rechunked_per_edit_near_starved_small  1138541
bytes_rechunked_per_edit_near_starved_large   292161
rechunk_edits_exceeding_the_second_clause          0
```

The five edits move into the class the second clause covers, and the second
clause holds on them with room: at seed 1 the run they walk into ends at
2 066 122, so the clause allows agreement by 2 328 266 and they agree at
1 396 525. It holds on all 1049 excused edits, the 962 starved ones included,
which is the first time that clause has been asserted anywhere rather than
assumed — `E2-P02` asserts it as part of an allowance and never as a row.

**Why this is not a bound moved to fit a measurement, which is the accusation it
has to answer, and it is the third entry in a row that has to answer it.**
Nothing about the bound moves. `RESYNC_BOUND_BYTES` is 512 KiB, claim 0017's
threshold is 786 432, `CHUNK_MIN_BYTES`, `CHUNK_MAX_BYTES` and the acceptance
rule are untouched, and no constant in `blob/src/chunk.rs` is edited by this
entry. What changes is a classifier in a workload, in the direction of the words
RFC 0061 already published, and it is paid for on the spot in three ways: the
class it moves edits *into* acquires a threshold it did not have, the class it
leaves behind acquires a floor (`rechunk_edits_clear_of_a_starved_run`,
`min = 64`) so that it cannot quietly empty, and the falsified number stays
printed beside the corrected one — `edits outside a starved run in total` and
`their re-chunk maximum` are still in the report, still reading 158 and
1 138 541, where the next reader finds them.

**Two answers were genuinely live.**

- **Call the published bound false and restate it as a number that scales.** The
  honest version of this is *the flat bound does not hold on content with starved
  runs in it, and the chunked kind therefore has no size-independent bound at
  all*. It is refused on the measurement rather than on preference: the bound
  does hold, on 231 of 231 edits whose content admits no forced cut in the
  region, at 345 390 bytes against an allowance of 786 432, and equally at both
  object sizes — 345 390 at 8 MiB and 262 144 at 128 MiB, sixteen times the size
  and a *smaller* number. A bound that holds on a class somebody can compute is
  worth more than no bound, and the class is computable from the file's own
  content.
- **Keep the point predicate and widen the threshold to 1 138 541 or above.**
  This is the move the registry exists to prevent, and it is worth naming because
  it is the cheap one: two rows, one commit, everything green. It fails on its
  own terms — 1 138 541 is a fact about one 645 661-byte gap at one seed, the
  next seed's gap is a different width, and a threshold set from it would be red
  again the first time a wider gap was drawn. A number produced by a mechanism the
  design admits is unbounded cannot be bounded by widening; it can only be scoped
  out or measured as what it is.

**Why `E2-P02` did not find this and was not going to.** That test is 32
(seed, mixture) pairs with one edit each; this workload is 1280 edits. The event
is rare — 5 in 1280 exceed the flat bound, 87 in 1280 are in the class at all —
and 32 draws of a 5-in-1280 event miss it 88% of the time. This is the argument
for a claim having a workload with a denominator rather than only a property
test, made by the thing happening: `claims/0017` said from the day it was
registered that `blob/tests/chunker.rs` measures the numerator against a `Vec`
and that `bench/src/bin/rechunk.rs` was where the real measurement would arrive.
It arrived, and it contradicted a published sentence in its first full run.

**And the reason nobody saw the first run.** `bench/src/bin/rechunk.rs` was not
reachable from `cargo xtask claim`: the claim routed to the chunker test alone,
so a threshold breach inside the bench went unobserved inside a green `verify`,
and `[workload] path` still named `blob/tests/chunker.rs`. That is the same
defect an audit raised against `claims/0018` one wave earlier. It is fixed in
this diff rather than filed — `Route::Rechunk` runs both workloads and compares
every row of the claim's `[threshold]` table against what they printed, a row
neither prints is a failure rather than a silence, and the fix was verified by
breaking a threshold on purpose and watching the command go red.

## Consequences

**What `claims/0017` says now.** The four chunked-kind rows keep their names,
their 786 432 and their meaning, and gain the scope sentence above; the class
the flat clause does not cover gains three rows —
`bytes_rechunked_per_edit_near_starved_small` and `_large`, recorded with no
flat threshold, and `rechunk_edits_exceeding_the_second_clause` at `max = 0` —
and the class it does cover gains a floor at 64 per object size. Sixty-four is
derived and not measured: the uniform mixture alone contributes 128 edits per
size, candidate gaps on uniform content are geometric with mean
`2^MASK_BITS` = 65 536 bytes, so an edit is inside a starved run with the
length-biased probability `(1 + r)e^(−r)` = 0.112 at `r` = 3.75 and a starved gap
begins inside the 512 KiB window with probability `1 − e^(−8e^(−r))` = 0.172,
leaving 0.736 of them clear — about 94 of 128, with a standard deviation near
five. Sixty-four is six standard deviations below that and is reachable by a
broken generator and by nothing else. Measured 117 and 114 in total, of which
the uniform mixture's own share is 99 and 102.

The extent-kind rows `E2-B09` owed the claim land in the same diff, and they are
RFC 0058's two predictions confirmed: the extent path costs
`2 × EXTENT_BYTES` = 2 097 152 bytes at its maximum, **the same number at 8 MiB
and at 128 MiB**, so it is bounded by the copy-on-write granularity and not by
the object; `pieces_touched_max` is 2; and straddling writes are 5 of 1280 drawn
writes, 0.391%, against the `4095 / 1048576` = 0.3906% RFC 0058 predicted from
the geometry.

**What `E2-P02` keeps, and this is the part that must not be misread.** Nothing
in `blob/tests/chunker.rs` is weakened. Its `resync_allowance` still evaluates
the starved clause at the edit, which is *stricter* than the bound this entry
states, and it still holds 32 of 32 pairs under that stricter reading. Widening
it to the window predicate would make the test weaker in exchange for making it
agree with this entry, and a test loosened to agree with an RFC is a test that
has stopped being evidence. What the test gains is eight printed rows under the
names `claims/0017` registers, so that the registry compares them instead of
trusting that an assertion somewhere in the file did. **The day a pair does fail
there, the first question is whether that pair's flat window meets a starved run**
— if it does, this entry says the failure is the second clause's business and the
tight reading has simply found the class early; if it does not, the acceptance
rule is reading something outside its window and the prefix half of the bound is
in question.

**What this makes easy.** Deciding, for a given file and without running
anything, which clause of the bound applies to an edit: scan for a 240 KiB run
with no candidate in it within 512 KiB after the write. That is the same shape of
question RFC 0062 left the reader — *does my content repeat with a period shorter
than 16 KiB* — asked about a neighbourhood instead of about the whole object, and
the two together are now the whole of what a reader needs.

**What this makes hard.** Any future claim that the chunked kind's cost is
bounded *in general*. It is bounded on content with no forced cut near the edit,
and roughly a quarter of uniformly drawn edits on uniform content fail that test.
The comfortable summary — *an edit re-chunks at most 786 432 bytes* — is now
false without its clause, and the clause is not a footnote.

**What this forecloses.** The reading in which the starved class is a property of
*pathological content*. Every one of the five falsifying edits is in the
`concatenated` mixture, which is content with candidates in it — the class is
reached from cuttable content, by proximity, and no mixture label predicts it.
RFC 0062 already said the label decides nothing; this says the label decides
nothing *even about the edit's own position*.

## What would reverse this

- **A clear edit above 786 432 bytes.** One edit with no starved run in its flat
  window that re-chunks more than the bound allows falsifies the scoped statement
  outright, and there is no clause left to move it into. `claims/0017`'s
  `bytes_rechunked_per_edit_chunked_*` rows are that assertion and
  `cargo xtask claim bytes-rechunked-per-byte` is where it fires. This is the
  observation this entry most wants somebody to go looking for, and the way to
  look is more seeds: `--seeds 8` is four times the sample for four times the
  wall clock.
- **`rechunk_edits_exceeding_the_second_clause` going non-zero.** Then the
  *second* clause is wrong — the sequences do not agree by the run's end plus one
  chunk — and the bound has no clause that holds. That is a larger finding than
  this entry's and it lands in the same command; it would put RFC 0061's second
  clause, not its acceptance rule, on the table.
- **`rechunk_edits_clear_of_a_starved_run` falling below 64 at either size.**
  Then the scope has eaten the sample and the flat threshold is green over
  nothing. The first suspect is the generator, as it is one level up in
  `resync_pairs_unstarved`; the second is `MASK_BITS`, which moves candidate
  density and therefore how often a 240 KiB gap appears.
- **The measured clear fraction moving materially away from 0.736 on uniform
  content while `MASK_BITS` and the gear table stand.** The floor above is
  derived from an exponential model of candidate gaps, and the model is what
  makes 64 a threshold rather than a guess. The uniform mixture measured 99 and
  102 of its 128 against a predicted 94, and the totals of 117 and 114 carry the
  concatenated mixture's contribution besides — so the comparison a future run
  should make is per mixture. A uniform-only count far
  from 94 says the gaps are not geometric with mean 65 536, and the derivation of
  every threshold in this entry rests on that.
- **A workload in which the near class is empty.** Then this entry has scoped out
  a class that does not occur, the correction was unnecessary, and the point
  predicate was adequate after all. Measured 87 of 1280, so this is not the
  present state; a run that drew only uniform content at 128 MiB would come close
  to it, which is why the mixture is iterated and not drawn.
