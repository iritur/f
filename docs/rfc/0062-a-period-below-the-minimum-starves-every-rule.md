# RFC 0062: A period below the minimum starves every rule, so the starved clause carries periodic content and is measured

- Status: accepted
- Date: 2026-09-06
- Affects: `intent/0006-state/spec.md` (the chunker paragraph's account of what
  the starved clause covers, and the first entry in *Risks and reversal*);
  `blob/src/chunk.rs` and `blob/src/lib.rs`'s header prose;
  `blob/tests/chunker.rs`'s property 4;
  `claims/0017-bytes-rechunked-per-byte.toml`'s
  `resync_pairs_carried_by_the_flat_clause` row and its `[diagnosis]`;
  `docs/rfc/README.md`'s status row for RFC 0061; `E2-B09` and `E2-P02`.
- Supersedes, in RFC 0061 and nowhere else, **two sentences and one
  requirement**: the *Consequences* sentence *"Periodic content whose period is
  below the target size leaves it, which is the measured repair"*; the sentence
  in the same entry's RFC 0058 paragraph, *"Half of that stops being true:
  periodic content below the target size is served by the chunked kind under this
  rule"*; and the first bullet of its confirming run, *"with the flat clause —
  not the starved clause — carrying every periodic and concatenated pair"*,
  together with the remedy its first reversal condition names for that bullet.
  **RFC 0061 is not edited by this entry and is not superseded as a whole**: its
  acceptance rule, its mask width and labels, its fixed-point constraint, its
  bound, its foreclosure of rules that read the previous boundary, and
  `RESYNC_BOUND_BYTES` all stand, and the run below confirms them. The marking is
  the one `docs/rfc/README.md` uses and the only one it uses — a status in its
  *Reserved numbers* table — and the row RFC 0061 needs is
  `accepted; superseded in part by 0062`, with a row for this entry beside it
  under `E2-P02`. Neither file is touched here, because both belong to the diff
  that implements this.

## Decision

The window predicate stays exactly as RFC 0061 decided it, and the *claim RFC
0061 made about what it repairs* is withdrawn. Periodic content whose period is
below `CHUNK_TARGET_BYTES` is **not** served by the chunked kind's flat clause;
it is carried by the starved clause, measurably, and — for the periods that
matter — provably. The published bound is unchanged and holds 32 of 32 pairs.
What changes is the sentence describing which content lands in which of its two
clauses, and the test stops asserting the sentence that was wrong and starts
asserting the structure that is right.

The reason this is not a mask width, not a rule change and not a lookahead
buffer is one line of arithmetic RFC 0061 did not do:

> **Any boundary rule that reads only a bounded window of content and guarantees
> a minimum chunk size of `m` produces no content boundary at all on exactly
> `p`-periodic content with `p < m`.**
>
> On content of period `p ≥ 64` the register at position *i* is a function of the
> last sixty-four bytes and therefore of `i mod p`, so any predicate over a
> bounded window of content has a boundary set `B` with `B + p = B`, away from
> the object's ends. If `B` contains any position `b` it contains `b + p`, and
> `(b + p) − b = p < m` contradicts the minimum. So `B` is empty, every cut is
> forced at `CHUNK_MAX_BYTES`, and the object is starved. ∎

`CHUNK_MIN_BYTES` = 16 KiB, so **every exactly-periodic object with a period
below 16 KiB is starved under every rule this design is willing to consider** —
one mask or two, local maxima, nested masks, anything. The workloads RFC 0058
names have periods of 4096 and 8192 bytes. They are inside that class by
arithmetic rather than by luck, and no chunker reaches them.

Three things follow, and they are the whole entry. The starved clause is
load-bearing and the published bound already says so. `E2-P02` asserts the
*structure* that produces the split — the candidate set on periodic content is
all-or-nothing — rather than asserting a distribution over mixtures that the
design cannot promise. And RFC 0058's second object kind stops being a hedge
against a class somebody hopes is small and becomes the answer to a class
somebody can compute.

## Context

**What the confirming run confirmed.** RFC 0061 was accepted on a measured
failure, a structural argument, and a size distribution it admitted was
arithmetic rather than observation. `cargo test -p f-blob --test chunker` over
the same eight recorded seeds and four mixtures reports:

- **Property 4 on 32 of 32 pairs** against the published bound — zero
  violations, where the rule RFC 0061 replaced failed 7 of 32 and one of those
  failures preserved nothing after the edit at all.
- **Property 5 on 32 of 32 pairs.**
- **The tighter flat clause applies on 13 pairs, holds on all 13, and is met
  exactly on 12.** That is the number RFC 0061's whole argument produces —
  agreement at the first accepted boundary at or after
  `X + L + CHUNK_MIN_BYTES + 64` — and it is met at that boundary rather than
  merely inside the 512 KiB outer allowance.
- **Property 2's aggregate mean 108 063 bytes over 859 interior chunks**, inside
  the unmoved `[32 768, 131 072]` band, against the 84 152 the entry predicted:
  the arithmetic was 7.3% low because the maximum truncates the tail, which is
  the direction an unmodelled truncation moves a mean, and the entry had already
  said this was the number in it most likely to be wrong.
- **The forced-cut fraction, recorded for the first time**: 166 per ten thousand
  interior chunks on uniform content, 2148 periodic, 2222 concatenated, 10 000
  zero-filled. RFC 0061's third reversal condition was one interior chunk in ten
  on uniform content. It did not fire and it is not close: retiring normalised
  chunking cost 1.66%, which is the price the bound was bought for, and it was
  affordable. `claims/0018` holds those numbers under that ceiling, and this
  entry does not tighten it — a ceiling drawn tightly around a single
  measurement is the description RFC 0061 refused to invent, and one run is
  still one run.
- **Zero-filled content exactly `CHUNK_MAX_BYTES` on 87 of 87 interior chunks**,
  which is what pins the gear table and the new mask together.

**What it falsified, using its own numbers.** RFC 0061 required the flat clause,
not the starved clause, to carry every periodic and concatenated pair, and said
in as many words that a pair passing only because its starved run swallowed the
object is a pass it does not claim. The run carries 8 of those 16. The other
eight — periodic on seeds 2, 3, 4, 6 and 8, concatenated on seeds 4, 5 and 7 —
pass only on the starved allowance, and the test prints the candidate count of
the whole edited object beside each: **0, 1, 1, 1 and 3** on the periodic ones
across two to four megabytes, and 48, 14 and 15 on the concatenated ones, whose
few candidates sit in their uniform sub-runs and not near the edit. Those objects
are not marginal cases. They are candidate-free.

**The cause is arithmetic, and RFC 0061 had the first half of it and stopped.**
The entry knew the register is a function of the last sixty-four bytes; it did
not carry that through to periodic content. On content of period *p* the register
therefore takes at most *p* distinct values and recurs every *p* bytes, so the
expected number of mask hits over a *whole object* is `p / 2^MASK_BITS ≤ 1` and
the candidate set is all-or-nothing: density `1/p` if some residue hits the mask,
**empty otherwise**, with `P(empty) = e^(−p / 2^16)`. Averaged over the drawn
periods that is `1 − e^(−1)` = 0.632, and 5 of 8 seeds measured it. So the mask
width RFC 0061 chose made periodic content *more* starved than the 14-bit loose
mask it retired, where the same arithmetic gives 0.245 — which is why the entry's
own pre-repair measurements had seen periodic content with candidates in it, and
why it drew the wrong conclusion from that.

**Then the width is the suspect, and it is not, and this is where the two-line
theorem in *Decision* arrives.** A narrower mask moves `P(empty)` and can do
nothing else, because the acceptance rule then finishes the job: a
`+p`-invariant candidate set with `p < CHUNK_MIN_BYTES` has a candidate within
`p` bytes behind every candidate, so **no candidate is ever accepted, however
dense they are**. Density is not the axis. The axis is `p` against
`CHUNK_MIN_BYTES`, and on the test's drawn distribution — uniform over
`[64, 65536)` — that puts **one drawn period in four (24.9%) beyond the reach of
any rule at all**. Dropping `CHUNK_MIN_BYTES` is the only thing that moves that
line, and RFC 0061 measured what dropping it costs: 233 of 233 interior chunks
below the minimum, a mean of 10 186 against a 65 536 target, two red properties
bought for one repaired.

**Two answers were genuinely live, and the second one loses on its own
arithmetic.**

- **Adopt local-maximum chunking now (the MAXP family), paying the `w`-byte
  lookahead RFC 0061 refused.** This is what RFC 0061's first reversal condition
  names, and its stated reason is that MAXP's independence is unconditional where
  the window rule's is conditional on the forced cut. That is true, and it is not
  the property in question. The property in question is whether MAXP's candidate
  set can be empty on a periodic sequence, and the theorem answers it: MAXP's
  guaranteed minimum *is* its window `w`, its expected chunk is `2w + 1`, so a
  64 KiB mean forces `w` = 32 KiB — and MAXP is therefore starved on **every**
  period at or below 32 KiB, deterministically, where the mask is starved on the
  same content only with probability `e^(−p / 2^16)`. Priced over the drawn
  distribution: the window predicate carries 33.9% of drawn periods in the flat
  clause (measured 3 of 8, predicted 0.339), MAXP would carry 50.0%. That is a
  real improvement, and it is a *fraction* rather than a repair — RFC 0061's
  requirement was every pair, and half is not every pair. What the fraction costs
  is a 32 KiB lookahead buffer in a `no_std` library above the frame, the end of
  *it holds no bytes and copies none*, `CHUNK_MIN_BYTES` ceasing to be a choice
  and doubling to 32 KiB, a superblock field set that changes shape, every object
  hash in the system, and claims 0017 and 0018 re-measured from nothing. And it
  moves the starved boundary from `p < 16 KiB` to `p < 32 KiB`, which is the
  **wrong direction** for the workloads RFC 0058 names: a 4 KiB database page and
  an 8 KiB one are starved before and after.
- **Keep the window predicate, and say in the published bound that the starved
  clause carries periodic content.** Chosen. The bound already holds 32 of 32
  with that clause in it; what was wrong was a sentence about which content the
  clause covers, and the honest repair for a wrong sentence is the right sentence
  plus a test that asserts the mechanism rather than the hope.

**Why this is not a bound moved to fit a measurement, which is the accusation it
has to answer.** Nothing about the bound moves: not `RESYNC_BOUND_BYTES`, not
claim 0017's 786 432-byte threshold, not the two clauses, not the starved run's
240 KiB definition, not a constant in `blob/src/chunk.rs`. The published bound
was measured *before* this entry and holds 32 of 32. What is withdrawn is a
prediction about the split between two clauses that were both already published,
and it is withdrawn because the arithmetic that would have refuted it before the
run was available before the run and nobody did it. The test loses no assertion
in the trade — it gains one that is harder to satisfy than the one it replaces
was on the content it applies to, and it gains it in the place where the
falsified sentence used to be.

## Consequences

**What `E2-P02` asserts instead, and every clause of it can fail.** The
requirement being retired is *every periodic and concatenated pair is carried by
the flat clause*, which was a statement keyed to a mixture *label*. Four
assertions replace it, keyed to the object's own candidate sequence:

1. **The published bound, untouched**: `agreed ≤ allowed` on 32 of 32 pairs.
2. **The tight clause where it applies, untouched**: a pair whose edit is not
   inside a starved run must agree by the first accepted boundary at or after
   `X + L + CHUNK_MIN_BYTES + 64`. It applies on 13 pairs and is violated on
   none. This is RFC 0061's actual content, and it stays exactly as strong.
3. **Carriage, restated over content instead of over labels**: a pair whose edit
   is *not* inside a starved run must be carried by the flat clause. The mixture
   name decides nothing; the candidate gap the edit sits in decides everything.
   This is the replacement for the withdrawn sentence, and it sits where that
   sentence sat, so that a reader looking for it finds what is true rather than
   nothing.
4. **The structure that produces the split, asserted rather than described.** On
   the *drawn* periodic object — not the edited one, whose inserted uniform bytes
   contribute their own candidates and are what the measured counts of 1, 1, 1 and
   3 actually are — count the candidates at positions at or after 64, where the
   register is a pure function of `i mod p`, and assert that the count is
   **either zero or at least `len / p − 1`**. The eight *edited* periodic objects
   measured 41, 1, 1, 0, 77, 3, 83 and 1, and the reason those middle counts are
   the insertion's and not the content's is the assertion itself: a period below
   65 536 in an object above two megabytes puts `len / p ≥ 32` candidates in it
   the moment one residue hits, so a count of 1 or 3 cannot be a residue hit and
   must be the 33 to 94 KiB of uniform bytes the edit spliced in. The next agent
   confirms that reading by running the count on the drawn object, where the
   assertion predicts 41, 0, 0, 0, 77, 0, 83 and 0. That is the all-or-nothing
   property;
   it is what makes the split intelligible, and it is a real assertion with a real
   failure mode: it goes red the day the register stops being a function of the
   last sixty-four bytes, which is the day the prefix half of the bound stops
   being true. The test threads the drawn period out of the generator and prints
   it beside the count, so that `e^(−p / 2^16)` can be checked by hand against
   the run.

The split itself becomes recorded data with three thresholds, none of them the
measured value:

- **At most seven of the eight periodic pairs may be starved.** Under
  `P(empty) = e^(−p / 2^16)` a correct mask starves all eight with probability
  `0.632^8` = 2.6%, so this is the largest threshold eight seeds can carry
  without becoming a description; measured 5. Its failure mode is the one that
  matters: a mask widened without anybody noticing what it does to short periods
  — exactly the regression RFC 0061 introduced when 14 bits became 16 and
  `P(empty)` went from 0.245 to 0.632. Raising this threshold needs more seeds,
  not a bigger number.
- **All eight zero-filled pairs must be starved.** Not a statistic: zero-filled
  content has no candidates at all by the fixed-point constraint on the mask's
  draw, so this is exact, and it is a second guard on the property `gear`'s test
  guards from the other side.
- **At least four pairs must be unstarved**, as the positive control that keeps
  assertions 2 and 3 from passing vacuously. A uniform edit sits in a 240 KiB
  candidate gap with probability `e^(−3.75)` = 2.4%, so the eight uniform seeds
  alone make four a threshold that nothing but a broken generator reaches;
  measured 13.

**What `claims/0017` must say.** The row
`resync_pairs_carried_by_the_flat_clause = { min = 24 }` is RFC 0061's number,
and it is red because RFC 0061 was wrong rather than because the chunker is. It
is **replaced, not widened** — widening it to 16 would be exactly the fitted
threshold its own `[diagnosis]` tells the reader not to write — by the rows the
assertions above produce:
`resync_pairs_unstarved_missing_the_tight_clause = { max = 0 }`,
`resync_pairs_unstarved = { min = 4 }`,
`resync_pairs_starved_periodic = { max = 7 }` and
`resync_pairs_starved_zero_filled = { min = 8, max = 8 }`. Three of the four sit
at zero or at an exact count, which is what a threshold looks like when it is
derived rather than observed. The `[diagnosis]` entry that currently says *the
entry names the next design — local-maximum chunking* is superseded by this one
and must say what the theorem says instead: a period below `CHUNK_MIN_BYTES` is
starved for every rule, the repair is the second object kind, and the only knob
that moves the class is `CHUNK_MIN_BYTES` itself.

**RFC 0058's argument gets larger, and that is the honest reading of the
measurement.** RFC 0058 justified a second object kind on the sentence that the
chunked kind is unbounded on "long zero runs, or periodic content whose period is
below the target size, which is precisely what a freshly-provisioned virtual
machine image and a preallocated database file are made of". RFC 0061 wrote that
half of that had stopped being true. It had not. Both halves stand, and the
periodic half is now the *stronger* of the two rather than the weaker: a long zero
run is starved because a particular register fixed point misses a particular drawn
mask — a fact about two constants, which a different draw would change — while
periodic content below `CHUNK_MIN_BYTES` is starved because of a theorem about
minimum chunk sizes that no draw and no rule changes. RFC 0058's decision was
already correct; what it now has is a reason that does not depend on the chunker's
parameters at all. A reader who wants to attack the second object kind has to
attack `CHUNK_MIN_BYTES` = 16 KiB, and `E2-P02` has already measured what happens
when that goes: two red properties and a mean of 10 186 bytes.

**What that changes for `E2-B09`, concretely.** Three things, and the first is the
one that would otherwise be missed:

- **Claim 0017's chunked-kind rows gain a periodic-below-target workload as its
  own row beside the zero-filled one**, with the drawn period recorded next to
  it. The 786 432-byte threshold does **not** apply to that row: on starved
  content the number scales with the object, which is the design admitting what it
  admits, and a threshold there would be either false or vacuous. It is a recorded
  row for the same reason the zero-filled row is one — so that the number the
  design is bad at is visible rather than averaged into a mean — and it must be
  drawn at a period below 16 KiB, because a bench that drew 48 KiB periods would
  report a bounded number and hide the whole class.
- **The extent-kind rows must be taken on a periodic workload too**, not only on
  uniformly-drawn 4 KiB writes. Periodic content below the minimum is now a
  measured member of the class the extent kind exists to serve, so a run that
  never handed the extent path that content has not measured the case the kind was
  bought for.
- **RFC 0058's own reversal conditions are unchanged**, and this entry adds none
  to it. Nothing here touches the granularity, the snapshot boundary or the 256×
  number; the class those answers cover got its width back, which makes them
  easier to justify rather than different.

**What this makes easy.** A reader can now decide, for their own data and without
running anything, whether the chunked kind will serve it: *does my content repeat
with a period shorter than 16 KiB, or go 240 KiB without a candidate*. The first
half of that question is answered by looking up the file format's page size. That
is a sharper thing to hand somebody than a probabilistic claim about masks.

**What this makes hard.** Any future argument that the chunked kind covers more
content than it does. The theorem is a floor under the starved class, and it moves
only when `CHUNK_MIN_BYTES` moves — which changes every object hash, is a
superblock field, and costs the size distribution `claims/0018` gates on. The
comfortable move of narrowing the mask to make a test greener is now visibly not
a move at all.

**What this forecloses.** The remedy RFC 0061 named for its own first reversal
condition. Local-maximum chunking is not the answer to periodic content and
cannot be, because its minimum chunk size is larger than the window predicate's
and the theorem is monotone in the minimum: MAXP at `w` = 32 KiB starves strictly
more periods than the current rule does. Anyone proposing it must now do so for
the independence it buys against *forced cuts* — its actual advantage — and must
pay for it with the measurement named below, not with the periodic mixture.

## What `E2-P02`'s exit means under this decision

The exit, quoted whole and unedited by this entry: *"an edit at offset X
re-chunks a bounded region around X and nothing after it, for randomly generated
edits and object sizes."*

- *"re-chunks a bounded region around X"* — **met**, for content that produces
  candidates the acceptance rule can accept: `CHUNK_MAX_BYTES` before the edit and
  `RESYNC_BOUND_BYTES` after it, 786 432 bytes, independent of the object's size,
  measured on 32 of 32 pairs with zero violations and met at the tighter
  16 448-byte clause on all 13 pairs where that clause applies.
- *"and nothing after it"* — **met outside a starved run and false inside one**,
  and this entry replaces RFC 0061's account of how big *inside one* is. It is not
  only wholly zero-filled objects. It is every exactly-periodic object whose
  period is below `CHUNK_MIN_BYTES` = 16 KiB, by the theorem in *Decision*, for
  every acceptance rule this design permits; and, above that period, those objects
  whose candidate set the mask happens to miss, which is `e^(−p / 2^16)` of them
  and measured 5 of 8. On that content everything after the edit is re-chunked,
  `E2-P02` asserts exactly that, and RFC 0058's extent kind exists for it.
- *"for randomly generated edits and object sizes"* — **met**, over eight recorded
  seeds and four mixtures, with the mixture iterated rather than drawn so that the
  two answers above are separable instead of averaged.

So the exit is **partly met, and this design cannot meet it more than partly** —
the same honest statement RFC 0061 reached, arrived at again with the class in the
second bullet larger than that entry believed, and with a proof under it instead
of a prediction. The difference that matters to whoever closes the line: under RFC
0061 the shortfall was a defect awaiting a repair that the entry named, so closing
`E2-P02` meant conceding an open reversal condition. Under this entry the
shortfall is a property of every chunker with a minimum chunk size, the repair is
a second object kind that already exists as a decision, and `E2-P02` closes
against this section or argues with it. The exit's words are not edited here and
`TODO.md` is not edited here.

## What would reverse this

- **The period spectrum of real content.** Take a freshly-provisioned virtual
  machine image and a preallocated database file — the two artefacts RFC 0058
  names — and measure two numbers per file: the smallest period at which long runs
  repeat, and the candidate count per mebibyte under the shipped gear table and
  mask. This entry asserts those periods are below `CHUNK_MIN_BYTES` = 16 KiB,
  because page sizes are 4096 and 8192 bytes, and that no rule change reaches
  them. If the dominant periods come back **above 16 KiB**, the theorem stops
  covering the workloads the argument rests on, the starvation there is the mask's
  fault after all, and local-maximum chunking or a narrower mask is buying
  something real. This is the measurement this entry most wants somebody to take,
  it needs no code in `blob/`, and it decides the whole question.
- **Local-maximum chunking, modelled and counted on the same 32 pairs.**
  `blob/tests/chunker.rs` holds whole objects in memory, so MAXP can be modelled
  there at `w` = 32 KiB with no lookahead buffer, no crate change and no format
  change: implement it beside `candidates`, and report flat-clause carriage, the
  chunk-size distribution and the forced-cut fraction over the same eight seeds
  and four mixtures. This entry predicts 50.0% of drawn periods carried against
  the window predicate's 33.9%, and that both starve every period below their own
  minimum. **A measured carriage materially above 50%, or any periodic pair with
  `p` below 32 KiB carried by the flat clause, falsifies the theorem's application
  here**, and `E2-P02`'s repair becomes MAXP with the lookahead paid for.
- **A smaller `CHUNK_MIN_BYTES`, measured — the knob the theorem identifies, and
  the one nothing has measured.** RFC 0061 measured `CHUNK_MIN_BYTES` removed
  entirely (233 of 233 chunks below the minimum, mean 10 186, two red properties).
  It did not measure 4 KiB or 8 KiB, and the theorem says the starved class is
  exactly `p < CHUNK_MIN_BYTES`, so that is where the class is priced. Run
  `claims/0018`'s distribution and the store's per-chunk record overhead at
  `CHUNK_MIN_BYTES` = 4096. If property 2's band survives and the record overhead
  costs less than the deduplication a 4 KiB minimum buys on 4 KiB-periodic
  content, the class shrinks by a constant somebody measured, and this entry's
  *nothing reaches them* becomes *nothing reached them at 16 KiB*.
- **Property 4's new structural assertion going red** — a periodic object whose
  candidate count is neither zero nor at least one per period. Then the register
  is not a function of the last sixty-four bytes, the `+p`-invariance argument is
  unsound, and the prefix half of the bound is in question before anything about
  the starved clause is.
- **claim 0017's chunked-kind periodic row measuring bounded** once `E2-B09`
  exists: a re-chunk cost on periodic content below 16 KiB that does *not* scale
  with the object. Then something accepts a candidate the theorem says cannot be
  accepted, and the acceptance rule in `blob/src/chunk.rs` is not the rule written
  down here.
- **The starved fraction on the periodic mixture reaching 8 of 8.** The threshold
  is set at seven for a reason; at eight, `MASK_BITS` has drifted or the gear
  table has, and the chunked kind has quietly stopped serving periodic content
  altogether rather than serving a third of it.
