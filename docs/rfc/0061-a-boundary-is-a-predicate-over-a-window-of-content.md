# RFC 0061: A boundary is a predicate over a window of content, and not a distance from the last cut

- Status: accepted
- Date: 2026-09-06
- Affects: `intent/0006-state/spec.md` (the chunker paragraph, the superblock's
  mask fields, the two-clause re-chunking bound and the first entry in *Risks
  and reversal*); `blob/src/chunk.rs`, `blob/src/gear.rs`,
  `blob/tests/chunker.rs`; `abi/src/store.rs`'s superblock record when it is
  written; `claims/0017-bytes-rechunked-per-byte.toml`'s chunked-kind rows;
  `E2-B01` and `E2-P02`. It revises the second clause of the bound RFC 0058
  quotes in its *Context* — RFC 0058 is **not** superseded and its decision
  stands, but the class of content its argument rests on gets smaller, and that
  is said below rather than left for a reader to notice. It also revises
  `docs/design/deadline-all-the-way-down.html` line 204, *an edit near the start
  of a large file does not re-chunk everything after it*, which `E2-P02`
  measured false on periodic content and which this entry makes true again for
  content that produces candidates.

## Decision

The chunker's acceptance rule stops being a recurrence over the previous
boundary and becomes a predicate over a bounded window of content. Three
changes, and no change to `CHUNK_MIN_BYTES`, `CHUNK_TARGET_BYTES`,
`CHUNK_MAX_BYTES` or `RESYNC_BOUND_BYTES`:

1. **A candidate is a pure function of the last sixty-four bytes.** One mask,
   `MASK_BITS` = 16, its highest bit at 63: position *p* is a candidate when
   `register(p) & MASK == 0` and for no other reason. Normalised chunking is
   retired, because its two masks are selected by *distance since the previous
   boundary*, which is precisely the dependence being removed; keeping it would
   leave the phase-lock in the mask choice after removing it from the acceptance
   test.
2. **The minimum chunk size is restated as a content predicate.** A candidate is
   *accepted* when **no candidate occurs in the preceding `CHUNK_MIN_BYTES`**,
   rather than when it is at least `CHUNK_MIN_BYTES` past the previous cut. This
   still guarantees the minimum, in two lines: if *c* and *c'* are consecutive
   accepted positions, *c'* has no candidate in
   `[c' − CHUNK_MIN_BYTES, c')`, and *c* is a candidate at or before *c'*, so
   `c' − c ≥ CHUNK_MIN_BYTES`. The whole acceptance decision at *p* is now a
   function of the content in `[p − CHUNK_MIN_BYTES − 64, p]` and of nothing
   else.
3. **The forced cut is unchanged and is the only surviving reference to the
   previous boundary.** A boundary is still forced at `CHUNK_MAX_BYTES` since
   the previous boundary, and an accepted candidate within `CHUNK_MIN_BYTES` of
   a forced boundary is suppressed so that the minimum survives beside it. That
   suppression can only be reached inside a run with no candidate for
   `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` bytes, which is the region the bound's
   second clause is about and now says so exactly.

The published bound becomes, for an insertion of `L` bytes at `X`: nothing
before the last boundary preceding `X − 64` changes (unchanged, and hard); the
two boundary sequences agree **from the first accepted boundary at or after
`X + L + CHUNK_MIN_BYTES + 64`, and in no case later than
`X + L + RESYNC_BOUND_BYTES`**, *except* across a **starved** run — a maximal
run of the edited object in which no two consecutive candidates are closer than
`CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` — where they agree only at the end of that
run plus one chunk. The word that changed is the one that matters: the second
clause covered *candidate-free* content and now covers *candidate-starved*
content, which is the strictly smaller class in which a cut can be forced at
all.

**This entry rests on a measurement not yet taken.** The failure it repairs is
measured and so is the mechanism; the size distribution the repair produces is
arithmetic, not observation. The confirming run is named in *Consequences* and
the numbers it must report are stated there with the thresholds they must meet.

## Context

`E2-P02` is five properties over eight recorded seeds and four content mixtures.
Three hold. Two failed, and this entry exists because the second clause of the
bound was written about the wrong thing.

**What failed.** The resynchronisation bound failed on **7 of 32
(seed, mixture) pairs** — the periodic mixture on seeds 2, 4, 5 and 6, and the
concatenated mixture on seeds 4, 5 and 7. Uniform failed 0 of 8 and zero-filled
failed 0 of 8. The first counterexample is seed 2, periodic mixture, an object of
2 170 343 bytes, an insertion of 94 537 bytes at offset 828 916: the two boundary
sequences agree again **only at 2 264 880, which is the object's end**, against
the 1 447 741 the two-clause bound allowed. Nothing after the edit was preserved.
The enclosing candidate-free run ended at 924 443 — 2570 bytes past the edit — so
the run clause the spec leaned on was inert here and was not what carried the
failure.

**The mechanism is phase, not a coding error and not density.** With a content
period below `CHUNK_TARGET_BYTES` the candidate sequence is itself periodic, and
`CHUNK_MIN_BYTES` makes a cut *the first candidate at least 16 KiB after the
previous one*. That map is a rotation on a periodic candidate sequence: it
carries a phase difference forward exactly, forever. The model of the same
chunker shows both orbits cycling the spacings **[66 825, 67 734, 69 276]**
indefinitely, offset by a constant. On uniform content the same map is
contracting in expectation, because exponentially distributed candidate gaps
eventually produce one long enough to swallow the offset, and that is the whole
of why uniform passes 8 of 8 and periodic fails 4 of 8.

**Density cannot separate the two, and that is measured.** The periodic
mixture's interior chunks average 74 643 bytes over 314 chunks; the uniform
mixture's average 70 819 over 333. Chunk mean is a proxy for candidate density
and it is a good one here, because both mixtures cut mostly on content rather
than at the maximum. Two families whose candidate densities differ by about five
per cent, one of which fails half its seeds and the other none of them, cannot be
told apart by a threshold on density. A bound stated in density would therefore
be either false — if it were tuned tightly enough to keep uniform inside it — or
vacuous over most real content, if it were loosened enough to put periodic
outside it. That is the argument that removes the second of the two options the
builder named, and it is arithmetic over the measured means rather than a
preference.

**Three options were live.**

- **Drop `CHUNK_MIN_BYTES`, keeping everything else.** Measured, not argued: with
  the minimum removed so that acceptance depends only on the last sixty-four
  bytes, the failing object resynchronises at **1 459 945** — the first candidate
  after the edit — instead of never. It is a trade and not a correction. On that
  same object **233 of 233 interior chunks fall below `CHUNK_MIN_BYTES`** and the
  mean drops to **10 186 bytes against a 65 536-byte target**, which turns
  property 1 and property 2 red, wrecks the shape claim 0017 publishes, and
  multiplies the store's per-chunk record overhead by six. Trading one red
  property for two is not a decision, and this option is refused on its own
  numbers.
- **Keep the minimum and weaken the published bound to name candidate density.**
  Refused by the paragraph above. It is also the option this tree's rules are
  most suspicious of: a bound moved to fit a measurement has to still be a bound,
  and a density threshold that admits both a passing and a failing family is not
  one.
- **Make acceptance a predicate over a bounded window of content.** Chosen. The
  structural statement is one sentence: *a predicate over a window of `w` bytes
  forgets everything more than `w` bytes back, and a recurrence over the previous
  boundary need never forget anything.* `CHUNK_MIN_BYTES` expressed as *distance
  since the last cut* is a recurrence; expressed as *no candidate in the
  preceding `CHUNK_MIN_BYTES`* it is a window, it delivers exactly the same
  minimum chunk size, and it costs one extra word of chunker state.

**A fourth was considered and refused.** Local-maximum chunking (MAXP, and its
asymmetric relatives) buys the same independence a different way — a position is
a boundary when its register value exceeds every value within `w` bytes on either
side — and it comes with a guaranteed minimum chunk size of `w` for free. It is
refused for two reasons, both about this tree rather than about the literature.
It needs `w` bytes of lookahead, so the chunker stops being able to say what
`blob/src/chunk.rs` says today — *it holds no bytes and copies none: a chunk is
hashed as it is scanned* — and would need a `w`-byte buffer in a `no_std` library
above the frame. And it couples two constants the design states separately: the
expected chunk under local maxima is `2w + 1`, so a 64 KiB mean forces a 32 KiB
minimum and `CHUNK_MIN_BYTES` stops being a choice. If the confirming run below
fails, this is the next design and not a fallback, because its independence is
unconditional where the chosen rule's is conditional on the forced cut.

**The cost of deciding this later, which is why it is decided now.** Every one of
these constants is a superblock field, a mount whose compiled-in parameters
disagree with the device's refuses rather than re-chunking, and changing the rule
changes every object hash ever written. Nothing is on disk: `E2-B01` has not
written a superblock, `abi/src/store.rs` has no superblock record yet, and no
device in this tree has ever been formatted. Today this costs a diff to two files
and a mask label. After `E2-B01` it costs a format migration, and after `E2-R01`
it costs somebody else's data.

## Consequences

**What the mask change is, precisely.** `MASK_STRICT_BITS` = 18 and
`MASK_LOOSE_BITS` = 14 become one `MASK_BITS` = 16, the width at which the raw
candidate spacing on uniform content is `CHUNK_TARGET_BYTES` = 2^16 bytes. The
derivation stays where RFC 0026 put it — one stream, one label, bit 63 forced —
and the label moves to **`"f-blob mask v2"`**, because a derivation that changed
under an unchanged label is exactly the silent hash change `gear_label` and its
siblings are superblock fields to prevent. `GEAR_LABEL` does not move and the
gear table does not move with it.

**One constraint the drawn mask must satisfy, stated before it is drawn rather
than discovered by a red test.** The register's fixed point on a zero run —
measured at `0x127ee44bae552daa`, reached after sixty-four zero bytes — must not
hit `MASK_BITS`'s mask. If it did, zero-filled content would cut at a spacing
decided by the register's fixed point rather than by its content, which is a
worse property than having no candidates at all: every zero region in every
object in the system would cut at the same arithmetic offset. If the first draw
under `"f-blob mask v2"` hits it, the label increments until one does not, and
that rule is stated here so that it is a derivation constraint and not a constant
fitted to a test.

**The chunker grows by one word and keeps its shape.** `Chunker` becomes three
words — the register, the distance to the last candidate saturating at
`CHUNK_MIN_BYTES`, and the distance to the last boundary — and still holds no
bytes, still hashes as it scans, and `Cuts`'s `Drop` obligation is unchanged.

**What the bound is now worth, clause by clause.** The prefix clause is untouched
and still hard. The flat clause gets *stronger* where it applies: outside a
starved run the sequences agree from the first accepted boundary at or after
`X + L + CHUNK_MIN_BYTES + 64` — 16 448 bytes past the edit — rather than merely
inside `RESYNC_BOUND_BYTES` = 512 KiB, because every acceptance decision from
that point on reads only content the edit did not touch. `RESYNC_BOUND_BYTES`
stays at 512 KiB as the published outer allowance, so claim 0017's chunked-kind
threshold does **not** move: it is still `CHUNK_MAX_BYTES` + `RESYNC_BOUND_BYTES`
= **786 432 bytes**, and a bound whose repair moved its own threshold would be a
bound fitted to its measurement.

**The second clause is smaller and the class it names is nameable.** It covered
content with no candidates at all; it now covers content in which consecutive
candidates are never closer than `CHUNK_MAX_BYTES − CHUNK_MIN_BYTES` = 240 KiB,
which is exactly the condition under which a cut is forced. Long zero runs are
still in it — the register's fixed point produces no candidate, so a zero run is
still cut only by the maximum and the two streams still do not resynchronise
across one. Periodic content whose period is below the target size leaves it,
which is the measured repair.

**RFC 0058's argument narrows, and this is the honest reading of it.** RFC 0058
justifies a second object kind partly on the sentence that the chunked kind is
unbounded on "long zero runs, or periodic content whose period is below the
target size, which is precisely what a freshly-provisioned virtual machine image
and a preallocated database file are made of". Half of that stops being true:
periodic content below the target size is served by the chunked kind under this
rule. What survives is the other half — preallocation and long zero runs — and it
is still what a freshly-provisioned image is made of, so RFC 0058's decision
stands. But a reader should know the class got smaller, and if `E2-B09`'s
measurement shows the surviving class is too small to justify carrying two object
kinds, that is a reversal condition for RFC 0058 and it belongs there rather than
being discovered here.

**Deduplication, and what its mildness does and does not say.** Property 5 failed
on 1 of 32 pairs: seed 2, concatenated mixture, 2 097 152 bytes of shared content
placed at offsets 73 192 and 225 609 deduplicated to 1 558 708 bytes against the
1 572 864 the bound owed — short by **14 156 bytes, which is 0.90% of the owed
coverage**. That is the same phase-lock and not a second defect: the shared run
enters the two objects at two different phases, the cut orbits never collide, and
the chunks that would have matched never exist. **A reader must not take the small
number as evidence against the large one.** Deduplication is a byte-weighted
average over a two-megabyte run and resynchronisation is a statement about a
single position, so a defect that makes *every* boundary after an edit wrong
costs the average only in the region where the boundaries actually disagree —
here, a region that happens to be under one per cent of the run. The correct
reading is the reverse of the intuitive one: a deduplication ratio that stays
respectable while the resynchronisation bound is destroyed is what this class of
failure *looks like*, and anyone registering claim 0017 should expect
`bytes_rechunked_per_edit` on the chunked kind to scale with the object on
phase-locked content while the deduplication ratio barely moves. Neither number
is a check on the other, which is why `E2-P02` asserts both.

**The confirming run, and the numbers it must report.** This entry is accepted on
a measured failure and a structural argument, and its size distribution is
arithmetic. The measurement that confirms it is `cargo test -p f-blob --test
chunker` over the same eight seeds and four mixtures, reporting:

- **Property 4 on 32 of 32 pairs**, with the flat clause — not the starved clause
  — carrying every periodic and concatenated pair. A pair that passes only
  because its starved run swallowed the object is a pass this entry does not
  claim, so the test prints which clause carried each pair.
- **Property 5 on 32 of 32 pairs.**
- **Property 1 by construction**, which the argument above proves and the test
  still asserts.
- **Property 2's aggregate mean inside `[32 768, 131 072]`.** The arithmetic
  predicts an expected accepted spacing of `2^16 · e^(16384/65536)` = **84 152
  bytes** on uniform content before the maximum truncates the tail, against the
  70 819 measured under normalised chunking. That is 1.28× the target and inside
  the band with margin, and it is the number in this entry most likely to be
  wrong.
- **The zero-filled mixture still exactly `CHUNK_MAX_BYTES` on every interior
  chunk**, which is the assertion that pins the gear table and now also pins the
  new mask against the fixed-point constraint above.
- **The fraction of interior chunks forced at the maximum, per mixture**, which
  is the cost of retiring normalisation and which nothing currently records. The
  arithmetic puts it at a few per cent on uniform content against normalised
  chunking's under one. It has no threshold in this entry, because inventing one
  before the first measurement is how a threshold becomes a description; it gets
  one when it is measured, and it is a reversal condition below.

**What this makes easy.** A bound with a proof rather than a hope: the acceptance
decision has a stated window, and everything about resynchronisation outside a
starved run follows from that window in one step. `E2-P02` gets to assert a
tighter number than the one it was written for. And the failure class the design
admits to is now a property somebody can look for in their own data — *does my
content go 240 KiB without a candidate* — rather than a probabilistic claim.

**What this makes hard.** Size-distribution control. Normalised chunking was the
mechanism for keeping chunks near the target and forced cuts rare, and it is
gone; what replaces it is one mask width and the thinning the minimum does, both
of which are cruder. If the forced-cut fraction is high, the repair is *not* to
bring back a mask selected by distance from the last cut — that is this entry
reversed — but a content-only normalisation: nested masks, with a loose hit
accepted only when no loose hit occurred in the preceding `CHUNK_TARGET_BYTES`.
That is a bounded-lookback predicate too, it keeps both mask widths as superblock
fields, and it is a further RFC because it is a second guess at a distribution.

**What this forecloses.** Any future acceptance rule that reads the previous
boundary. The forced cut at `CHUNK_MAX_BYTES` is the single exception this entry
grants, and it is granted because the alternative is an unbounded chunk; anything
else that wants to know where the last cut was has to reverse this RFC, and the
reason to reverse it has to be a measurement rather than a size distribution
somebody finds prettier.

## What `E2-P02`'s exit means under this bound

The exit, quoted whole: *"an edit at offset X re-chunks a bounded region around X
and nothing after it, for randomly generated edits and object sizes."*

**Today it is unmet**, and this entry does not change that: `cargo xtask verify`
is red on properties 4 and 5, and the chunker in the tree is the one that failed
them. Nothing here is a claim that a task closed.

**When this decision is implemented and the confirming run reports the numbers
above, the exit is partly met, and it cannot be more than partly met by this
design.** The halves of the exit's sentence have different answers, and running
them together is what let the draft bound be wrong:

- *"re-chunks a bounded region around X"* — **met**, for content that produces
  candidates: the region is bounded by `CHUNK_MAX_BYTES` before the edit and
  `RESYNC_BOUND_BYTES` after it, which is the 786 432 bytes claim 0017 publishes,
  and it is independent of the object's size.
- *"and nothing after it"* — **met outside a starved run and false inside one**.
  In a run with no candidate for 240 KiB every boundary is forced, a forced cut is
  by definition relative to the previous boundary, and two streams offset by `L`
  do not resynchronise until the run ends. On a wholly zero-filled object
  *everything* after the edit is re-chunked, `E2-P02` asserts exactly that, and
  RFC 0058's extent kind exists because of it.
- *"for randomly generated edits and object sizes"* — **met**, over eight recorded
  seeds and four mixtures, and the mixture is what makes the two answers above
  separable instead of averaged.

So the honest statement is: the exit is met for content the chunker can see
boundaries in, and answered by a second object kind for content it cannot. That
is a weaker sentence than the exit's own; it is not a rewrite of the exit — the
exit's words are untouched by this entry and `TODO.md` is not edited by it — and
whoever closes `E2-P02` closes it against this paragraph or argues with it.

## What would reverse this

- **Property 4 still failing on any (seed, mixture) pair once the rule is
  implemented.** Then the recurrence is not the one this entry named, the
  remaining suspect is the forced cut, and the answer is local-maximum chunking
  with the `w`-byte lookahead buffer refused above — paid for, this time, by a
  measurement.
- **Property 2's aggregate mean outside `[32 768, 131 072]`, or the periodic and
  concatenated mixtures' own means falling below twice `CHUNK_MIN_BYTES`.** Then
  `MASK_BITS` = 16 is the wrong width. The repair is the width and not the rule,
  and the width is one line — but a width chosen after seeing the test is a fitted
  constant, so it needs its own entry saying so and its own re-derivation of the
  expected spacing.
- **The forced-cut fraction on uniform content exceeding one interior chunk in
  ten.** Then retiring normalisation cost more than the bound bought, and the
  repair is the content-only normalisation named in *Consequences* — nested masks,
  a loose hit accepted only when none occurred in the preceding
  `CHUNK_TARGET_BYTES` — as a further RFC with the measured fraction in it.
- **claim 0017's chunked-kind row measuring above 786 432 bytes on any mixture
  that is not starved.** That is the bound failing as a bound, and it is the
  observation this entry most wants somebody to go looking for.
- **The register's fixed point on a zero run hitting the drawn mask.** Then
  zero-filled content acquires boundaries decided by arithmetic rather than by
  content, `E2-P02`'s zero-filled assertion goes red, and the label increments
  before anything else is concluded.
- **A real workload that is candidate-starved for megabytes at a stretch and is
  not a mutable extent.** The surviving bad class is narrow enough that RFC 0058's
  second object kind is meant to cover all of it. One workload in it that the
  extent kind does not serve means the chunked kind needs a third answer, and the
  first thing to try is a larger `CHUNK_MAX_BYTES`, which is a claim and a
  superblock field rather than a rule change.
