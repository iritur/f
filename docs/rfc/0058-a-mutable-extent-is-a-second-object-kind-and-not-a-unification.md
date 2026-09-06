# RFC 0058: A mutable extent is a second object kind, and not a unification

- Status: accepted
- Date: 2026-09-06
- Affects: `docs/design/deadline-all-the-way-down.html` section 04 (the
  *Where this design is genuinely bad* paragraph) and section 09's line that
  mutable extents are "a patch rather than a resolution"; `abi/src/store.rs`
  (the `extent` blob kind), `blob/src/extent.rs`, `bench/src/bin/rechunk.rs`,
  `claims/0017-bytes-rechunked-per-byte.toml`; `E2-B09`, and RFC 0059's first
  collector invariant

## Decision

The store gains a second object kind. An **extent** is an object whose children
are fixed-offset pieces of `EXTENT_BYTES` = 1 MiB rather than content-defined
chunks, the copy-on-write unit is **one whole piece**, and a write of any size
produces a new piece blob for every piece it touches — a 4 KiB write copies,
edits and re-hashes a megabyte. The extent's own record, the list of piece
hashes that gives the extent one name, is written **only at an explicit
snapshot** taken by the component that owns the extent; between snapshots there
is no hash that names the current bytes, and a reader resolving the extent by
hash sees the last snapshot, whole. This is a second kind and not a
generalisation of the first: the same bytes stored as an object and as an extent
have two different hashes, a consumer picks a kind when it creates the object,
and changing its mind costs a full rewrite. We are choosing to carry two shapes
in the format rather than one shape that covers both badly, and the price is
paid in the number `E2-B09` registers — on the workload this kind exists for,
**every application byte written costs up to 256 bytes re-copied and re-hashed,
and up to 512 when the write straddles a piece boundary.**

## Context

`docs/design/deadline-all-the-way-down.html` section 04 has said since it was
written that content addressing "punishes small random writes to large mutable
objects — a database file, a virtual machine image", that the mitigation is a
mutable extent, and that this "is a real complication rather than an elegant
unification". Section 09 is blunter: mutable extents are "a patch rather than a
resolution", measure that case early and honestly. This RFC is the design that
sentence promised. It does not resolve the complication. It puts a granularity,
a boundary and a number on it so that the complication is arguable.

What was true when this was decided. The chunker exists as `E2-B01` specifies
it: gear-hash content-defined chunking, `CHUNK_MIN_BYTES` = 16 KiB,
`CHUNK_TARGET_BYTES` = 64 KiB, `CHUNK_MAX_BYTES` = 256 KiB, and a
resynchronisation bound of `RESYNC_BOUND_BYTES` = 512 KiB **or the end of the
enclosing candidate-free run plus one chunk, whichever is later**. That second
clause is the whole reason a second kind is needed, and it is worth stating in
the negative: on content with no chunk candidates — long zero runs, or periodic
content whose period is below the target size, which is precisely what a
freshly-provisioned virtual machine image and a preallocated database file are
made of — every boundary is forced at the maximum, a forced cut is relative to
the previous boundary, and two streams offset by an edit do not resynchronise
until the run ends. On that content the chunked kind's cost per edit is bounded
by the object's size, not by 786 432 bytes. Doing nothing was therefore not
"slow"; it was unbounded on the one workload a skeptic reaches for first.

Four alternatives were live.

- **One kind, and publish the bad number.** The honest minimum, and the reason
  it loses is the paragraph above: the number it would publish is not a number,
  it is a function of the object's size. A bound that grows with the file is not
  something a database can plan against.
- **Fixed-size chunking for everything — the unification.** Replace
  content-defined boundaries with fixed 1 MiB pieces for all objects and delete
  the second kind. Rejected: content-defined chunking is what makes an insertion
  in the middle of a component image or a package tree cost a bounded region
  instead of the tail, and that is the workload the store is *good* at and the
  one `lineage-and-debts` compares against git and Unison on. Unifying downward
  would trade a win for a tie.
- **A per-piece dirty log, absorbed at snapshot.** Append each small write to a
  log and fold the log into pieces only when a snapshot is taken. This would cut
  the number below by roughly two orders of magnitude — a 4 KiB write becomes a
  4 KiB append — and it is the first thing a reader who dislikes 256× will
  propose, so it is answered here rather than in a review comment. It is refused
  because it puts durable bytes in front of the store that no hash names. Three
  other designs then stop working as written: RFC 0059's collector marks from
  hashes, and would have to mark from a log it cannot verify; `E2-P01`'s exit —
  every cut leaves the old root or the new one, never a third thing — becomes
  *the old root plus a replay*, and a replay is the third thing, with a
  correctness argument that nothing checks; and RFC 0060's verify-before-accept
  has nothing to verify a log entry against. The dirty log buys one number and
  spends three properties. If the number turns out to matter more than the
  properties, that is a reversal condition and it is stated below, not a
  possibility we are pretending not to see.
- **Sub-piece copy-on-write: a Merkle tree of small leaves inside each piece.**
  Keeps everything content-addressed and cuts the re-hashing to a leaf plus a
  path. Rejected on the read side. A 4 KiB copy-on-write unit means a piece stops
  being contiguous on the device, so a sequential read of a megabyte becomes up
  to 256 scattered reads on a zoned device that rewards large sequential appends
  and punishes nothing else — and the read path is where this design's claims
  actually are (`copies_per_read` = 0, `resident_bytes_per_read_byte` ≤ 1.0). We
  would be trading a claim we intend to defend for a claim we do not make.

## Consequences

**The granularity, and why 1 MiB rather than the neighbours.** The piece is the
unit of contiguity, not only the unit of copying, so the argument is two-sided.
Going *down* to 256 KiB would cut the per-write copy from 256× to 64×, and loses
on three counts: it is exactly `CHUNK_MAX_BYTES`, so the second kind would have
the same rewrite granularity as the first and would exist for no structural gain
in the case that motivated it; the piece list quadruples, and at the sizes
`E2-B09` measures the list is the thing rewritten at every snapshot; and a
256 KiB append is small enough that per-operation overhead starts to show on the
device. Going *up* to 4 MiB makes the per-write factor 1024× and shrinks a list
that is already small enough. 1 MiB is the smallest power of two strictly above
`CHUNK_MAX_BYTES`, and it is the size at which a 128 MiB extent — the larger of
the two object sizes claim 0017 measures — has 128 pieces, whose 32-byte hashes
are 4096 bytes, exactly one 4 KiB block. That coincidence is the actual
argument: below it the extent record costs more than one block for objects this
size, above it we pay four times the copy to save bytes in a record that already
fits in one. It is an argument from arithmetic, not from measurement, and the
measurement that would move it is named below.

**Reads are unaffected by the granularity**, which is what makes a coarse write
unit affordable. A read resolves to a zone and an offset and takes a
block-granular range inside a piece; nothing forces a reader to fetch a megabyte
because the writer had to write one. The granularity is a write-side constant
only.

**The copy-on-write unit is the whole piece, and a write touches at most two.**
A write of `W ≤ EXTENT_BYTES` bytes at offset `O` rewrites
`ceil((O mod EXTENT_BYTES + W) / EXTENT_BYTES)` pieces, which is 1 or 2. The
per-edit cost is therefore bounded by `2 × EXTENT_BYTES` and not by
`EXTENT_BYTES`, and the straddling case is about 0.39% of 4 KiB writes at
uniformly drawn offsets, so a bench that runs long enough will hit it. We state
it here because the alternative — a workload that draws aligned offsets — would
be fitting the measurement to the threshold. The exit's words still hold: the
bound is a stated multiple of the copy-on-write granularity and is independent
of the object's size.

**The piece size is recoverable from the data.** Every piece but the last is
exactly `EXTENT_BYTES`, and a piece is itself a blob whose header carries
`content_bytes`, so a reader recovers the granularity from piece 0 rather than
from a compiled-in constant. `EXTENT_BYTES` is therefore a writer's constant and
not a superblock field — unlike the chunker's parameters, which a mount refuses
on disagreement because they decide every object hash. Extents written at
different granularities coexist and stay readable, which is what makes the
granularity reversal below cheap.

**The snapshot boundary, and what a reader sees between boundaries.** An extent
is published by `snapshot()` on the owner's handle, which writes the extent blob
— `extent_bytes`, `pieces`, and the piece hashes in order — and returns its
hash, which is then nameable by a generation node like any other object. Nothing
else publishes an extent: not a byte count, not a timer, and there are no timers
(RFC 0004). Between boundaries:

- A reader resolving the extent's published hash sees the bytes of the last
  snapshot, whole. There is never a hash that names a half-updated extent, which
  is the property the dirty log would have given up.
- The owner reading through its own handle sees its own writes. Read-your-writes
  holds inside the owning component and nowhere else, and this is the one place
  the store has state that is not a function of a root.
- A crash between boundaries loses everything written since the last snapshot
  and nothing else. Durability granularity is the snapshot interval, chosen by
  the consumer, and a consumer that wants per-commit durability puts the
  snapshot in its commit path and pays for it there.
- Pieces written since the last snapshot are durable and hashed but named by no
  root, which is exactly what RFC 0059's mark phase calls garbage. An extent
  with unsnapshotted pieces therefore registers as a **transient root** for the
  duration, under the same clause an open publish uses. The held set is bounded
  by the extent's own size: a piece rewritten twice before a snapshot leaves the
  intermediate referenced by nothing, and the collector may take it. A component
  that writes and never snapshots holds at most one extent's worth of blobs
  against the collector — a leak with a name and a bound, rather than an
  unbounded one.

**The workload it is still bad at, said with its numbers.** A database page
cache flushing 4 KiB or 8 KiB pages into a large mutable file, and a virtual
machine image taking 4 KiB guest writes, are the two named cases and neither
gets better here — they get *bounded*. A 4 KiB write costs 1 048 576 bytes
copied and re-hashed, which is 256×; straddling, 512×. A database that snapshots
per commit pays, per 4 KiB commit, one piece plus a 4096-byte piece list plus one
4096-byte root record — 1 056 768 bytes, or 258× — and that is before the index
and the collector's copy-forward, which claim 0016 counts separately. The extent
workload is deliberately **not** in claim 0016's workload set: at 256× it would
fail that claim's `device_bytes_per_app_byte` ≤ 1.5 threshold by two orders of
magnitude on the first write, and mixing it in would either destroy a threshold
that is correct for the workload it was derived for or hide this one inside an
average. Two claims, two workloads, both published.

**The number `E2-B09` registers, with its denominator defined**, because a
reviewer found "application byte" ambiguous about padding and index writes and
was right to. Claim 0017's extent rows are:

- **Denominator: application bytes.** The sum of the `bytes` fields of the write
  entries the client submitted on the objects ring, and nothing else. A 4 KiB
  write is 4096 whether it lands aligned or straddling, whether the piece was
  already dirty, and whatever the device does afterwards. This is the spec's
  definition of an application byte, applied here without exception.
- **Numerator: bytes copied and bytes hashed, as two rows.** *Copied* is bytes
  moved into the new piece, including every byte the client did not write — that
  is the quantity this design is being honest about. *Hashed* is bytes fed to
  `f-hash`'s streaming state. They are equal for a full piece and are still
  recorded separately, because a future sub-piece scheme would move them apart
  and a single row would hide it.
- **Excluded from both, and where each is counted instead.** Padding to
  `block_bytes` is a device byte, is never hashed (the hash is over
  `content_bytes`), and belongs to claim 0016. Index log entries and root records
  are device bytes and belong to claim 0016. The extent record's own rewrite at a
  snapshot is a *snapshot* cost, not a per-write cost: folding it into the
  per-write number would make that number a function of how often the bench
  snapshots, which is a knob and not a property, so it is its own row with the
  snapshot interval recorded beside it. No byte is counted by two claims.
- **Thresholds.** `bytes_rechunked_per_edit_extent_*` at `max = 2097152`
  (`2 × EXTENT_BYTES`), with `pieces_touched_max = { max = 2 }` beside it as the
  row that says *why* the primary is two megabytes and not one. The straddling
  edit is recorded as its own row, the way the zero-filled mixture already is, so
  the 0.39% case is visible rather than averaged away. Geometry rows
  (`write_bytes`, `object_bytes_large`) stay required, so a run that quietly
  shrank the write cannot report a bound it did not test.

**What this forecloses.**

- *Cross-object deduplication for this kind.* Piece boundaries are at fixed
  offsets, so an extent deduplicates only against byte-identical, 1 MiB-aligned
  content. Clones of the same lineage still share — a virtual machine image
  copied from a snapshot shares every piece until it is written, which is the
  case that pays — but two images built independently from the same base share
  nothing, and that is the case content addressing was supposed to win. The
  extent kind gives it up.
- *Sub-piece durability.* There is no way to make a byte durable without making a
  megabyte durable. A write-ahead log must not be an extent; it is an append-only
  chunked object, and a consumer that wants both puts its log in one kind and its
  pages in the other.
- *One name per byte sequence.* An object and an extent over identical bytes have
  different hashes, because their contents are a chunk list and a piece list.
  "One hash for the machine" now means one hash per (bytes, kind) pair. Nothing
  collides — `kind` is in the blob header and the hashed content differs — but
  conversion between kinds is a copy, and a consumer that chose wrong rewrites.
- *The unification itself.* Adding a fifth blob kind is a diff to `abi/` and an
  RFC by rule; so is collapsing these two into one. A later design that finds the
  single shape has to reverse this entry rather than quietly widen a kind.

## What would reverse this

- **claim 0017's extent primary measuring above `2 × EXTENT_BYTES`, or
  `pieces_touched_max` above 2.** Then the copy-on-write unit is not what this
  entry says it is, and the write path — not the threshold — is wrong.
- **A real consumer whose write distribution makes the 256× continuous rather
  than a bench artefact.** Concretely: an E3 or E4 component whose own
  submissions on the objects ring show more than half its writes below 64 KiB
  against extents larger than 1 GiB, sustained over a workload somebody actually
  runs. At that point the per-piece dirty log is buying more than it spends, and
  its three costs — an unverifiable mark root, a replay in the crash story, and
  nothing for verify-before-accept to check — become the design work rather than
  the reason to refuse.
- **The extent record becoming the dominant term.** If extents above 128 MiB are
  the common case, the piece list exceeds one block and is rewritten in full at
  every snapshot; a claim 0017 run whose snapshot row exceeds its per-write row
  says so directly, and the answer is to raise `EXTENT_BYTES` to 4 MiB and re-run
  the arithmetic in *Consequences*, not to reopen the kind.
- **Deduplication failing to survive cloning.** Once two virtual machine images
  descend from one base in a real test, measure shared pieces. Fewer than half
  shared means fixed-offset pieces do not survive the cloning that was the whole
  remaining deduplication argument, and the granularity or the boundary rule has
  to change.
- **A crash-consistency defect on the extent path.** `E2-P01` producing a mount
  outcome that is neither the last snapshot nor the previous one, root-caused to
  an extent. Keeping every durable byte inside a hashed blob was chosen precisely
  to make that class of bug impossible; one such defect means the rule bought
  nothing, and the cheap thing we refused should be reconsidered on its own
  merits.
