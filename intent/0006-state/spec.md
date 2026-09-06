---
id: 0006
status: draft
reviewed_by: "pending: Dmitri Chudinov"
skills: determinism-review, frame-and-unsafe, licence-boundary, claims-registry, rfc-author, memory-ordering
---

# Spec: state — one hash for the machine, and a way to take it back

The system acquires a memory and an undo. A content-addressed store lands as
`no_std` libraries under one component, reaching the zoned device the only way
anything does — over the blk driver's ring — and every blob, object and
generation is named by the SHA-256 that `xtask/src/pack.rs` already computes
for a release, moved to one crate so that there is one identity rather than
two. A publish is not one write and its atomicity is not free: it is a barrier
sequence over an opcode space the blk driver does not yet have, and the format
is written so that a device which lies about the barrier costs a rollback
rather than a corruption. A generation is a closed tree of fixed-width records,
compiled on the host the way RFC 0030 compiles a manifest and folded to one
root; the root is packed into one boot module, so the loader stays ignorant of
the on-disk format and boot is a pure function of one hash. An update stops
being a reboot: a component is instantiated alongside, drains to the quiescent
point the ring's cursors already define, hands over fixed-width state, and the
place swaps its routing. The simulator gains a power-cut model that cuts inside
a write and may reorder anything a barrier has not covered; the registry gains
four counts that gate in the container — device bytes per application byte and
its ratio against a tuned Linux filesystem on the identical emulated device,
bytes re-chunked per application byte on both object kinds, copies per read,
and resident bytes per unit of work — and the two timings beside them stay
`pending` on the machine E0 still owes. Twenty-five task ids, four movements,
gate G2, release 0.3.

## Behaviour

**Decide.** Four decisions the task lines name, and a fifth RFC this spec
discovers is owed. `E2-D01` is **RFC 0012**, the number `TODO.md` has reserved
for it since the line was written: an update is a generation swap, the root
hash is the attestation, and the rollback metric changes from *one reboot* to
*one generation swap, and a reboot only when the frame changed* — which is why
the root record carries the frame's hash as a separate field, so that a swap
can tell whether the frame moved without unpacking anything. `E2-D02` is **RFC
0058**, mutable extents as a second object kind rather than a unification:
copy-on-write at a granularity of `EXTENT_BYTES` = 1 MiB, content-addressed
only at a snapshot boundary, and the random-write workload it is bad at named
in the RFC so that `E2-B09` measures the workload the design chose rather than
one that happened to be convenient. RFC 0058 also has to say why the obvious
alternative was refused — a per-piece dirty log absorbed until snapshot, which
would cut the small-write number by two orders of magnitude and would put
mutable state that is not content-addressed in front of the store — because
that is the first thing a reader who dislikes 256× will propose.

`E2-D03` is **RFC 0059**, the collector's policy written as three invariants,
because `E2-P03` asserts invariants and cannot assert behaviour. The first
ranges over pinned roots **and open publishes**: a publish registers as a
transient root at its first write and is released only when its root record is
durable. Without that clause a publish spanning more than one zone seals a zone
whose live fraction against pinned roots is zero, the sweep resets it before
the root lands, and the root then names a reset zone — `E2-P01`'s forbidden
third state produced by the collector rather than by the cut, with all three
invariants holding while it happens. The second is that a reset zone holds no
live blob. The third is the one that needs a number before it is an invariant
at all: *collection never starves a deadline-class read* is not assertable
without a bound, and a bound expressed in time is a clock. So RFC 0059 states
it as a count — **a hard-class read submitted while a sweep is in progress is
handed to the device with at most `collector_operations_ahead_of_hard_read` = 1
collector operation ahead of it** (`Unit: count of device operations`) — and
the 1 is not invented: it is `in_flight` from claim 0012, the one chain
`Driver::execute` keeps inside the device, because everything else the
collector has queued is batch-class work a hard-class entry overtakes under RFC
0025. The row moves when `E1-B09` raises `in_flight`, and it is re-derived in
the same diff, exactly as claim 0012's own `in_flight` row says. Sweeping below
`SWEEP_LIVE_FRACTION` = 0.25 is the fourth number RFC 0059 fixes, and the
write-amplification threshold below is derived from it. K reaching
`docs/design/` needs a claim of its own; inside `E2-P03` it is an assertion
under the simulator's virtual clock and never a duration.

`E2-D04` is a schema in `abi/` and not an RFC: `f_abi::transfer`, a fixed-width
record a component declares saying whether it can be updated in place, what its
state records look like, and what it declares when it cannot. Its fields, with
units, because `lint-units` runs on `abi/` and two peers read this the moment
`E2-P08` swaps one: `schema` (`Unit: none`, a version), `mode` (`Unit: none`;
`restart_only` | `in_place`, and `restart_only` is the honest declaration and
the default), `record_bytes` (`Unit: bytes`, the fixed width of one state
record), `records_max` (`Unit: count of records`, the most the component will
hand over). It lands before `user/virtio-blk` declares one, under ordering rule
1.

**A fifth RFC is owed and this spec names it: RFC 0060, a publish is a barrier
sequence and atomicity is not free.**
`docs/design/deadline-all-the-way-down.html` section 04 says *write the blobs,
then write the root*, and *atomicity is free because a root is a single write*.
That sentence is being reversed here — the root record is one write, and the
ordering between it and the blobs it names is not free, it is bought — and the
house rule is that a reversal of something already written down is an RFC. RFC
0060 carries the barrier sequence, the extension of the blk service's opcode
space that makes it expressible, and the verify-before-accept rule that keeps
the format correct on a device that lies about the barrier. It lands with
`E2-B01`'s diff, before `f-zone` exists to depend on it. No task line in
`TODO.md` names it; that is reported to the originator rather than fixed by
editing the file.

**Build.** The hash first, because everything else is named by it. `hash/`
(package `f-hash`) is SHA-256 as FIPS 180-4 states it, `no_std`, no allocator,
with a streaming state so a chunk is hashed as it is scanned rather than
buffered; `xtask/src/pack.rs` loses its transcription and calls this one, and
the two vectors the standard publishes move with it. That is `E2-B01`'s first
sentence and it is one implementation, which is what RFC 0012 needs before it
can say "one identity".

*The ordering, which is the load-bearing part and the part that did not exist.*
The only path to the device is the blk driver's ring, and `user/virtio-blk`
answers exactly two opcodes today — `op::READ` = 1 and `op::WRITE` = 2. There
is no flush, so nothing in this design can ask a writeback cache to persist the
blobs before the root record that names them, and there is no reset, so `zone/`
cannot reset a zone over a ring that cannot express one. Under RFC 0060 the
service's opcode space gains five, in the driver's own `op` module because that
module is already the shared definition both peers read — `kernel/src/blk.rs`
imports `f_virtio_blk::driver` — and each maps to the virtio request type
beside it: `FLUSH` = 3 (`VIRTIO_BLK_T_FLUSH`), `ZONE_APPEND` = 4
(`VIRTIO_BLK_T_ZONE_APPEND`), `ZONE_FINISH` = 5, `ZONE_RESET` = 6 and
`ZONE_REPORT` = 7. They are new wire under ordering rule 1 and land before
`f-zone`.

**A publish is: write the blobs, `FLUSH`, append the root record, `FLUSH`.**
The first barrier is what makes the root record's meaning true; the second is
what makes it durable before the next publish is allowed to start. Zone appends
use `ZONE_APPEND` rather than a positioned write, so the device rather than the
writer assigns the position and two appenders cannot race a write pointer — the
completion carries the position back, which is what the index records.

*What the format does if the device lies.* A device that acknowledges `FLUSH`
without persisting is not detectable from above, so the format does not rest on
detecting it. It rests on **verify before accept**: a root record is believed
only after its check field verifies and after every child of its generation
node resolves — the frame hash, the topology hash, and each component leaf
under it — and resolution is bounded by the size of the generation tree, which
is tens of nodes, not by the blob count. A root whose blobs did not land is
therefore refused at mount and the mount falls to the next-highest verifying
record, which is the previous generation. So a lying device costs a rollback,
not a corruption, and the barrier is what makes that cost rare rather than what
makes the design correct. *What would reverse this:* generation trees large
enough that mount-time resolution is itself a cost worth measuring, at which
point the choice is to trust `FLUSH` and to register a claim for it.

*What the mount does with a half-published generation.* Exactly the above: it
is not a third state, it is the previous root plus unreferenced blobs, and the
next collection reclaims them because nothing pins them.
`roots_refused_unresolved` is a count in `cargo xtask cut`'s artefact so that
this path is exercised rather than argued.

`blob/` (package `f-blob`) is the store as a library: the chunker, the record
kinds and the on-disk format. **The record types and their codecs live in
`abi/src/store.rs`**, not in `blob/`, and they are not `repr(C)`. The only
reason to be `repr(C)` is to view device bytes as a struct, and that view is a
pointer cast — what `abi/src/state.rs` does under `unsafe` today — which a
library above the frame may not write. So a record is *decoded*, field by
field, in safe code: `Record::from_bytes(&[u8; N]) -> Result<Record, i32>`
reads each field with `u64::from_le_bytes` and refuses on magic, kind and
length bound before it returns, and **no byte from the device is trusted until
that constructor returned `Ok`** — a peer wrote it, in the sense
`ring/src/mapping.rs` means, because the device did. The layout is fixed by the
encoder/decoder pair and a golden-bytes test rather than by a Rust attribute,
each record's encoded length is asserted equal to the sum of its field widths
at compile time so that there is no padding anywhere in a hash, and `blob/`
consumes the types. `abi/` is where `lint-units` already runs, which is the
second reason to put them there — and it is what makes `E2-B08` honest, because
the blob header that lands by DMA in the caller's registered buffer is decoded
from those bytes there and never viewed.

The format is fixed-width and little-endian, and every public field states its
unit in its doc comment under R03.

The **superblock** — written once, and its zone must be a *conventional* zone:
a superblock in a sequential-write-required zone could never be rewritten and
so could never be re-pointed, and the mount refuses a device whose zone 0 is
sequential rather than discovering that later. Fields: `magic` (`Unit: none`),
`schema` (`Unit: none`, a version), `block_bytes` (`Unit: bytes`, the device's
logical block), `zone_bytes` (`Unit: bytes`), `zones` (`Unit: count of zones`),
`root_zone_a` and `root_zone_b` (`Unit: zone index, zero-based`), and the
chunker's parameters, because a chunker that changed under a mount decides
every object hash and nothing on disk would say so: `chunk_min_bytes`,
`chunk_target_bytes`, `chunk_max_bytes` (`Unit: bytes`), `mask_strict_bits`,
`mask_loose_bits` (`Unit: bits`), `gear_label` (`Unit: none`, the label the
gear table is derived from). A mount whose compiled-in parameters disagree with
the superblock's refuses rather than re-chunking.

The **blob header** — at every block-aligned position a blob starts: `magic`,
`kind` (`Unit: none`; `chunk`, `object`, `extent`, `generation`), `flags`
(`Unit: none`, zero until something is named), `content_bytes` (`Unit: bytes`,
the content that follows, before padding to `block_bytes`), `hash` (`Unit:
none`, the SHA-256 of the content). An **object** is a blob whose content is
`object_bytes` (`Unit: bytes`, the object's logical size), `chunks` (`Unit:
count of chunk hashes`) and that many hashes in order, which is what makes an
object one hash. An **extent** blob (`E2-B09`) is an object whose children are
`EXTENT_BYTES` = 1 MiB pieces — `extent_bytes` (`Unit: bytes`), `pieces`
(`Unit: count of pieces`) — and whose write path replaces one piece rather than
re-chunking a region.

The **root record**, appended to a root zone: `magic`, `generation` (`Unit:
count of publishes since the superblock; the first publish is 1, and 0 is
reserved so that a zeroed block is never a generation`), `root` (`Unit: none`,
the generation hash), `frame` (`Unit: none`, the frame image's hash), `module`
(`Unit: none`, the hash of the boot module this root was booted from),
`previous` (`Unit: none`, the previous root, or zero at the first publish), and
**`check` (`Unit: none`, the SHA-256 over every preceding field of this
record)**. The check field is not decoration: without it a torn record whose
magic landed and whose `root` field is half-written names a hash that does not
exist, and *not found* is indistinguishable from *collected*. No field is
believed before `check` verifies. There is no timestamp in any of these; the
generation counter is the order, and RFC 0004 is why.

*The root zone, redesigned, because the first version could not exist.* A
sequential-write-required zone fills, and the only operation on a full one is
reset, which destroys every record in it including the current one — a window
with no root at all, reached deterministically after `zone_bytes / block_bytes`
publishes, and the write-once superblock could never be re-pointed at a fresh
zone. So there are **two root zones, A and B, both named in the superblock**.
Records append to the active one. When the active zone's remaining capacity
falls below `ROOT_CARRY` = 16 records (`Unit: count of root records`), the last
`ROOT_CARRY` records are appended to the other zone, `FLUSH`, and only then is
the full zone reset. There is never a moment with no durable root, and the
guaranteed rollback depth immediately after a wrap is sixteen generations
rather than one — chosen so that `E2-P07`'s demonstration always has somewhere
to go and a stranger's machine does not depend on where in the cycle it was
cut. **Mount takes no mutable pointer.** It reads both zones' write pointers
with `ZONE_REPORT` — the device's own pointer, which the format does not
maintain — scans backwards from each, and takes the highest `generation` whose
`check` verifies and whose generation tree resolves. Rollback reaches back as
far as the records surviving in the two zones, which is between 16 and
`2 · zone_bytes / block_bytes` generations, and the design says that plainly
rather than promising unbounded history: keeping a generation *beyond* that is
pinning its root, which is what snapshot retention already means.

The **chunker** is gear-hash content-defined chunking in FastCDC's shape, and
the window is stated as the arithmetic actually gives it. With
`h = (h << 1) + gear[b]`, bit *k* of the register depends only on the last
*k+1* bytes, so a mask over the low 16 bits would give a 16-byte effective
window and not the 64 an earlier draft of this spec claimed — a number that
would have been false by a factor of four in the crate doc, in `E2-P02`'s
specification and in the claim's workload description, and a 16-byte window is
exactly what phase-locks boundaries on data with short periods. So the masks
are FastCDC's shape: set bits spread through the register with **the highest at
bit 63**, so a candidate depends on the last 64 bytes and every byte of the
window contributes to the decision. `MASK_STRICT_BITS` = 18 below the target
size and `MASK_LOOSE_BITS` = 14 above it — FastCDC's normalised chunking,
adopted because it is what reduces cuts forced at the maximum, and the crate
says that is the reason. Target `CHUNK_TARGET_BYTES` = 64 KiB,
`CHUNK_MIN_BYTES` = 16 KiB below which candidates are ignored,
`CHUNK_MAX_BYTES` = 256 KiB at which a boundary is forced. The gear table is
`const GEAR: [u64; 256]`, filled by
`f_env::split::Stream::from_seed(f_env::split::label("f-blob gear v1"))`
iterated 256 times: `env/src/split.rs` already carries a `const fn` SplitMix64
and RFC 0026's whole argument is one derivation, so a third transcription
(`kernel/src/env.rs` is the second) would be a second generator for a reviewer
to check against the paper. The mask positions come from the same stream under
the label `"f-blob mask v1"`, with bit 63 forced set. Changing either label
changes every object hash, which is why `gear_label` is a superblock field.

The bound the design promises is two-sided and stated in the strength it has.
Nothing before the last boundary preceding `X − 64` bytes changes, by
construction. After the edit, the two boundary sequences resynchronise **within
`RESYNC_BOUND_BYTES` = 512 KiB of `X + L`, or at the end of the enclosing
candidate-free run plus one chunk, whichever is later** — and the second clause
is not a hedge, it is the honest half. In content with no candidates at all —
zero runs, periods below the target size, which is exactly `E2-D02`'s named bad
workload of VM images and sparse database files — every boundary is forced at
the maximum, a forced cut is by definition relative to the previous boundary,
and two streams offset by `L` do not resynchronise until the run ends. An
earlier draft blamed the minimum size for this; the minimum is not the cause,
the maximum-size forcing is, and the reversal that draft stated — acceptance
independent of the previous boundary — does not fix a forced cut. That is
written in the crate rather than found by a user, and it is measured: the
zero-filled workload is its own row in the re-chunking claim, so the number the
design is bad at is recorded rather than averaged away.

`zone/` (package `f-zone`) is `E2-B02`: sequential fill with `ZONE_APPEND`,
seal with `ZONE_FINISH`, copy-forward, reset with `ZONE_RESET`, and the
collector RFC 0059 specifies — mark from pinned roots and open publishes, sweep
below `SWEEP_LIVE_FRACTION`, run as a batch-class consumer. `index/` (package
`f-index`) is `E2-B03`: paths, metadata and declared attributes to hashes, a
log-structured store on the device with a `BTreeMap` in memory, a local call
and not a service. **`E2-B03`'s exit is *measured against a tree walk over the
same data*, and the measurement has to be a count that can differ.** Ring
crossings cannot be: inside one component a tree walk crosses zero boundaries
too, so zero against zero would make the exit vacuous. The count is **device
blocks read per query against device blocks read per tree walk over the same
data**, recorded in the host test against the modelled device and stated as a
bound. And the exit's *without crossing a component boundary* is read here as
*the index is not a service*: the query crosses the client's boundary into
`user/objects` and none between the index and the store. That reading is
weaker than the words and it is flagged for the originator rather than buried
in *Decisions*.

`generation/` (package `f-generation`) is `E2-B04`, the evaluator, and the
answer to the intent's second open question is that **the expression is not a
language**. A generation expression is a closed tree of four fixed-width node
kinds, enumerated in `abi/src/store.rs` so that a fifth is a diff to `abi/` and
an RFC by rule rather than by convention: `bytes` (a leaf naming a blob or
object hash), `component` (a leaf naming a component file — RFC 0030's record
plus image, already one hash), `topology` (an ordered list of named components
and the routes between them, each route a `capability` name and a `source`
(`Unit: index into the topology's component list, zero-based`), names
thirty-two bytes of `[a-z0-9-]` as `docs/manifest.md` already bounds them), and
`generation` (the root). **The generation node is the frame hash and the
topology hash and nothing else.** The store parameters are not in it: a root
that embedded `block_bytes` and `zone_bytes` would be a different root on two
runners with different emulated disks, so `E2-P06` would fail on its first run
and name a leaf that is not a leaf, a disk replacement would change *what are
you running* without anything running having changed, and a rollback across a
re-provisioned device would name a generation that no longer exists. Geometry
lives in the superblock. What a generation may state about a device is a
*requirement* — a minimum checked at mount — and a requirement is a bound, not
a value the hash depends on.

Evaluation is a post-order Merkle fold — a node's hash is the SHA-256 of its
encoding with each child replaced by that child's hash — and the encoding has
no offsets, no pointers and no strings that are not names. **The canonical form
is defined here, because *the same expression yields the same root* is only
true if the compiler is canonical.** Components are sorted bytewise on the
zero-padded 32-byte name; routes are sorted on (component index, capability
name); duplicates are refused. `cargo xtask generation` **refuses**
non-canonical source and prints the canonical form as a diff rather than
silently reordering it, so that two authors with the same set cannot produce
two roots and no author learns their file was rewritten by reading a hash. And
the compiler is invertible: `cargo xtask generation --decompile` renders a
record tree back to canonical source and a lint checks the round trip is a
fixpoint on every `generation.toml` in the tree. That is the tripwire that
actually holds, because the refusal that matters is not on the artefact — no
record kind changes when `import`, interpolation and conditionals return to the
*source* under the name of convenience, which is how Nix's language would
re-enter through the compiler while a tripwire watched the records. A source
feature not representable in a record fails the round trip.

What the expression deliberately is not: lazy, recursive, functional, or
extensible at run time. There is no `import`, no interpolation, no fetch, no
conditional, no environment, no path and no clock, and nothing a person writes
is read by the machine: `generation.toml` is source, `cargo xtask generation`
compiles it through the checker `lint-manifests` already is, and the record
tree is the artefact. The machine never evaluates anything. It receives a root,
and the assembler recomputes the fold over the records it is handed as a check,
which is the same crate doing the same arithmetic.

*How a root reaches a booting machine, which an earlier draft left out and
`E2-P07` hits first.* The root's records live in a store served by
`user/objects`, and at boot that component is not running, so an assembler
holding a hash has nothing to read. The answer is not a loader that understands
the on-disk format — that would be a second implementation of the format,
outside the tree and outside the claim, and on hardware it would be GRUB, which
is GPLv3 and would cross the licence boundary this epoch does not cross.
Instead: **the generation record tree and its component files are packed into
one boot module named by the root hash**, RFC 0030's component-file shape one
level up, handed over as a multiboot module. The assembler recomputes the fold
over the module and refuses a mismatch. The store holds the same bytes, which
is what `E2-P07`'s bit-identity compares and what E4's delta transfer will
move. *What would reverse this:* a second reader of the format appearing
anywhere — in a loader, an installer or a rescue tool.

Selection is a **multiboot command-line token**, since the kernel already reads
its options from the multiboot command line (`docs/booting-on-hardware.md`
passes `timer=60` exactly this way): `f.root=<64 hex>` names a module by root
hash, the default is the newest module the loader offered, and the grammar is
defined in `abi/src/boot.rs` with a `Unit:` on each field. `cargo xtask
rollback` passes it with `-append`; on hardware, `cargo xtask generation
--install` writes one `menuentry` per installed generation into the GRUB
fragment `docs/booting-on-hardware.md` already documents. **That is what the
exit's "boot menu" means** — the loader's menu, configuration this repo already
writes, nothing imported and no new boot-time code that reads zones.

Above the libraries, one component: `user/objects` (package `f-objects`), the
object store as a place — `f-blob`, `f-zone` and `f-index` linked, a client
ring on one side and the blk driver's ring on the other, with the collector
inside it. `user/store` keeps its name and its job; the store is not called
`store`, and *Decisions* says why. `user/assembler` (package `f-assembler`) is
`E2-B05`: it instantiates a topology from a root, routes capabilities as the
topology declares, binds drivers by declared properties, and leaves a subtree
unstarted when its driver fails rather than failing the boot. **Binding is not
allowed to inherit a hardware ordering.** A PCI scan is a discovery order and
two drivers can declare the same property set, so the tie-break is stated
rather than discovered: binding iterates the topology's component list in
canonical order and, within it, matches devices sorted by their declared
identity (bus/device/function as a `BTreeMap` key), never by the order
discovery reported them; two drivers matching one device is a `lint-manifests`
refusal at compile time and not a run-time choice. Without that, `E2-B05`'s
byte-identical topology from one root is a property of the scan.

`E2-B06` is the swap, and the quiescent point is the ring's own: the place
stops delivering to the old instance, so a client's submissions pend exactly as
RFC 0008's connect pends against an empty place; the old instance drains until
the cursors of RFC 0018 say producer stopped and consumer caught up; it writes
its `f_abi::transfer` records over its control ring; the new instance
acknowledges them; the place's routing swaps; the old instance is retired.
Nothing is discarded, which is the one word that separates this from claim
0005's kill.

`E2-B07` publishes the root and the frame hash in the frame's state tree, so
*what are you running* is one read. The exit says *any modification produces a
different one*, and a frame that merely republished a hash it was handed would
meet that for everything under the root except the thing publishing it — a
weaker sentence wearing the id. So the frame half is closed without hardware:
`xtask`'s image builder computes SHA-256 over the frame image with `f-hash` at
build time, the frame recomputes it over its own loaded text and rodata at boot
and **refuses to publish a root whose `frame` field disagrees**, and `cargo
xtask mutate` gains a frame mutation whose boot must produce a different
published hash. What remains is the residual a TPM closes and E5 owns — a frame
modified to lie about its own recomputation — and that residual is named in the
attestation story rather than left for a reader to find.

`E2-B08` resolves a hash to a zone and an offset through the index and hands
the caller's registered buffer through both rings under RFC 0024's typestate,
so the DMA lands in it. The exit says *counted rather than asserted*, and a
typestate argument is asserting by construction, so the count is named: the
simulator's blk device model and the driver each tally **bytes moved through
any buffer that is not the caller's registered one**, the claim's primary is
that tally, and the two readings are required to agree — claim 0012's
discipline of counting the same event on both sides of a boundary, because a
zero one component published is a zero that component can produce by writing
it. The second half of the exit, *resident bytes per unit of work recorded*, is
a byte count and is recorded here rather than deferred: resident pages of
`user/objects` per N reads, read from the frame's state tree. `E2-B09` builds
the extent kind against the workload RFC 0058 names.

**`E1-B15` is in scope here and is written out rather than assumed.** The
intent's `todo:` carries it and `E2-P05` cannot exist without it, so its four
exit clauses are this epoch's work: every component the supervisor starts
publishes a tree of its own under RFC 0013's rules, mounted under one root; the
two reversals are paid and the words that state them are gone —
`user/store/src/report.rs`'s *a runtime that publishes a tree of its own* and
RFC 0038's *a state tree the component publishes under RFC 0013*; at least one
`E1-P02` sweep scenario asserts its system response by reading a component's
subtree rather than the serial log; and a component that publishes nothing is
refused at spawn rather than tolerated. It needs `E1-B05` and waits with the
assembler, which *Not in scope* lists.

**Prove.** `env/` gains a power-cut model, and the model is specified in
Behaviour rather than only warned about in *Risks*, because a plan written from
a warning builds the model the warning disqualifies. **A publish is a sequence
of device operations; at any cut the model lands every operation covered by a
completed `FLUSH` and any subset of the rest, in any order, with the cut
falling inside an operation at a chosen granularity.** Granularity is a
parameter — `block` and `byte` — and the sweep runs at both: a root record fits
in one block and cannot tear at block granularity, so a torn-record requirement
stated only at block granularity would be unsatisfiable and would read as the
model failing. The model has two modes: `honest`, which respects `FLUSH`, and
`lying`, which ignores it entirely, and `E2-P01`'s sentence must hold under
both. Running again from the same `(seed, cut, granularity, mode)` produces the
same device.

`E2-P01` sweeps every write boundary and every block boundary within every
write of every publish under a set of seeds and asserts that mount finds either
the previous root or the new one and never a third thing. Its artefact must
contain, as required observations rather than as hopes: at least one cut that
left a torn root record refused by its `check` field (byte granularity); at
least one cut in `lying` mode where a root record landed over blobs that did
not, refused by resolution and counted in `roots_refused_unresolved`; at least
one cut that crosses a root-zone wrap; and exactly two distinct mount outcomes
across the sweep. A sweep whose artefact shows none of the first three is a
sweep of the model.

`E2-P02` is the chunker's property test, written before the chunker exists as
its specification, and **its generator draws from a mixture rather than from
uniform bytes**: uniform, zero-filled, periodic with a period below the target
size, and concatenations of those, each a named site under RFC 0026. With
uniform bytes a candidate appears every 64 KiB on average, so the bound passes
on data the design is good at while the workload the design is bad at fails it
unobserved. The assertion is the two-sided bound as restated above, including
the candidate-free clause.

`E2-P03` runs the collector concurrently with adversarial allocation and a
hard-class reader in the simulator and asserts RFC 0059's three invariants,
with the allocation including **a multi-zone publish in flight during a sweep**
— the case the first invariant's open-publish clause exists for — and the third
asserted as `collector_operations_ahead_of_hard_read ≤ 1`.

`E2-P04` proves the frame's invariants with Verus through RFC 0022's image
shape once the frame has stopped moving, and its exit has three clauses, not
one. The candidate invariants are named so that the plan does not choose them
by accident: per-CPU ownership of every mutable `static` under `kernel/`; the
four cross-core words and no fifth; the ring's `Release`/`Acquire` pair. The
proofs **run on the nightly cadence beside `sweep`**, through the image target
only the checking job uses. And `cargo xtask mutate` gains a frame mutation the
proofs must reject, because a proof that passes on a frame it was not checking
is the failure the mutation build exists to catch everywhere else.

`E2-P05` compares two whole-system trees as two hashes and descends to the
divergent subtree. It has no dependency on the power-cut model — an earlier
draft asserted one, `TODO.md`'s graph does not, and state comparison has no use
for a cut — so the ordering is dropped. The tree is folded by `f-hash` over RFC
0013's read-only mapping, one subtree per component under one root, and the
divergence is injected through `Env` at a named site under RFC 0026, so that
the test knows the answer it is asking for.

`E2-P06` is the two-machine, two-date job over the generation root, naming the
non-reproducible leaf. Its **first deliverable is `--remap-path-prefix`** (or
`-Zremap-cwd-prefix`) in the workspace build configuration, because `E0-R01`
measured reproduction *at one path*, nothing in the tree configures a remap,
and two runners on two dates will not share a checkout path — so without it the
first leaf the job names is every component leaf, for a reason that is not the
evaluator's. `E0-R01`'s `address` job is where the remap is checked, so the
leaf is closed before the job can name it.

`E2-P07` breaks a generation on purpose, reboots with `f.root=<hex>` naming the
previous module, and verifies the restored generation bit-identical: the same
root, the same module hash, and every blob under it. `E2-P08` swaps
`user/virtio-blk` under sustained load with no dropped operation and the
transferred state verified by reading it back through the new instance.
`E2-P09` detects a 3% regression injected into synthetic history, by **binary
segmentation with a CUSUM statistic over the stored per-run distributions**,
written in `xtask/` over a `BTreeMap` keyed by commit order and adding no
dependency to `xtask/Cargo.toml` — named here because change-point detection is
the one piece of new algorithmic code in this epoch that would otherwise arrive
as a statistics crate, and *replacing thresholds* is the sentence that invites
one. Its false-positive half is a count of runs and not a p-value with a clock
in it.

`E2-P10` is the write-amplification claim, and it is registered the way its
exit reads: **bytes written to the device per byte written by the application,
against a tuned Linux filesystem on the same device.** The baseline runs: a
Linux guest with f2fs in zoned mode, booted by `cargo xtask claim
write-amplification --baseline` on the *identical* QEMU zoned virtio-blk device
inside the same container, with device bytes counted on both sides by the same
counter — QEMU's own `query-blockstats` over QMP — so the two numbers are taken
at one boundary. This is a count and not a timing, so claim 0001's shape does
not carry over: 0001's ratio is `pending` because ring-submit latency is a
timing a shared host cannot defend, and *bytes written* is the same number on a
fast host and a slow one. Both sides are marked `emulated = true`. Under TCG
rather than nested KVM the guest is slow, which costs wall time and changes no
count.

**The workload is phased, so that the number is not a distribution wearing a
count's name.** Copy-forward bytes depend on the live fraction at the moment
each zone is swept, so a collector running concurrently with a client makes the
count a function of the interleaving between two components rather than of
`(seed, commit)`. So: *fill* N zones under the `Env`-drawn workload with the
collector held; *collect* to quiescence with the client idle; *count*. That is
schedule-independent and the claim's `[hardware]` notes say so the way claim
0012's do. And P10 runs on **two workloads**, not one: the sequential
fill-and-collect above, and `E2-D02`'s random-write workload on both object
kinds, so that the number the skeptic reaches for first — a 4 KiB write copying
a 1 MiB piece — is a recorded row beside the fill number rather than a number
P10's workload never sees.

**Release.** 0.3: the storage and generation claims, the rollback and live-swap
demonstrations as commands a stranger runs, the attestation story as the
sentence RFC 0012 makes true *and the residual the frame's self-hash leaves
open*, and the rollback demonstration running from the release image on a
machine outside the project. `E2-R01`'s blockers are its own contents and
nothing here shortens them.

## Policy applied

Walked in the order `spec-from-intent` gives.

**1. Determinism.** The store is the easy case and the simulator is the whole
of the hard one. A content-defined chunker's rolling hash is a function of the
bytes it has seen and nothing else — the gear table is a constant derived at
compile time from one label under RFC 0026's single derivation, the window is
the last sixty-four bytes because the highest mask bit is 63, the masks are
constants — so it is deterministic by construction and reaches for no `Env` at
run time; `lint-determinism` should find nothing in `blob/` and that absence is
the design rather than an oversight. The same holds for the fold in
`generation/`, whose canonical form is now stated rather than left to a TOML
crate's table ordering, and for the assembler's binding order, which is
declared rather than discovered from a PCI scan. The obvious implementation
worth naming is the one the format refuses: a timestamp in the root record,
which every filesystem has and which would make a publish observe a clock; the
generation counter is the order and there is no third field. `alloc` addresses
are nondeterministic and the lint cannot see them, so nothing in the five
libraries keys a map on a pointer or iterates anything but a `BTreeMap` keyed
by a hash, an offset or a name — a review obligation named here so that it is
checked rather than assumed.

The simulator side is where the policy bites. The power-cut model is a device
model, and RFC 0034 says a device model is a peer on the real ring types with
every choice drawn through `Env`: which operation the cut lands inside, which
uncovered subset lands and in what order, are all draws, the sweep's seed set
is an argument, and a cut that cannot be re-run from
`(seed, cut, granularity, mode)` is not a cut. Every workload a claim in this
epoch runs under draws its data from `f_env::Env` seeded — the
write-amplification workload's offsets and lengths, the re-chunking workload's
edits and its four content mixtures, the million blobs' bytes — through a
`Stream` split at a named identity per site under RFC 0026, so that adding a
site later cannot move a recorded number. A workload that read `/dev/urandom`
for its bytes would be an unreproducible claim wearing a seed.

**2. The frame.** The store, the index, the evaluator and the assembler are
components and libraries above the frame; none of them may write `unsafe` and
none of them needs to — which is the reason the record types are decoded field
by field in `abi/` rather than viewed through a `repr(C)` cast, and the reason
the blob header that lands by DMA in `E2-B08`'s registered buffer is decoded
from bytes there and never viewed. A blob is arithmetic over bytes and a zone
is arithmetic over offsets.

Three places the frame is touched, and one it is not. The frame gains a mount
point for every component's subtree under one root (`E1-B15`, in scope here), a
swap point at a place, and the published root — with the frame's own image hash
recomputed at boot and checked against the record before it is published, which
is the third and is a few pages of `f-hash` over its own text. The swap point
is the sentence that has to be argued rather than written: a place's routing
word read by a client on another core is a fifth cross-core word, RFC 0016 says
a fifth needs an argument, and the argument is that the word changes only at
the quiescent point when no submission is in flight — the place holds pends —
under one `Release` store and one `Acquire` load, with a litmus test under the
weaker ordering the way the ring's pair has. If `E2-B06` cannot be built that
way, the intent already says what happens: it is a finding to bring back here,
not a lock to write in `kernel/`.

The place the frame is *not* touched, and the one that was closest to needing
it: `alloc`. No `no_std` crate in this workspace uses an allocator today and
the store needs one, and a `#[global_allocator]` is an `unsafe impl GlobalAlloc`
that a component crate cannot write. So the heap is `ring/`'s — a
bump-then-free-list allocator over a granted memory region, built by a safe
constructor that checked the region the way `ring/src/device.rs` checks a
window, under RFC 0033's shape: a granted window is a safe accessor, and a heap
is a window with a policy. The component declares the static; the `unsafe` is
in the frame with a `// SAFETY:` comment; `cargo xtask unsafe` moves by a
handful of blocks, once, for every component that will ever allocate. Host
tests use `std`'s allocator and never see this. It is the decision most likely
to be re-litigated in this epoch and *Decisions* flags it.

**3. The licence boundary.** Untouched, and checked twice, because the first
walk missed the loader. No E2 task imports anything. The rollback selection is
a multiboot command-line token the frame parses and a `menuentry` this repo
writes — not a bootloader in the tree, and specifically not GRUB, which is
GPLv3 and would have crossed the boundary if the "boot menu" had meant a menu
this project ships. The boot module is RFC 0030's own shape one level up, so
the loader stays format-ignorant and no second reader of the on-disk format is
created. The tuned-Linux zoned baseline is configuration under
`claims/baselines/` in the shape `E1-D06` set — a directory of files a stranger
applies, plus the guest image that script builds, not source — and QEMU's zoned
emulation is a tool in the development image, not code in the tree. `f-hash` is
written here rather than taken from a crate for the reason `licence-boundary`
gives about dependencies added for one function, and the change-point detector
is written in `xtask/` for the same reason rather than pulling a statistics
crate in behind the word *thresholds*.

**4. Evidence.** Per task, the `exit:` line and nothing weaker. Three
constraints from `claims-registry` shape the epoch.

*Counts gate, timings wait — and the classification is by the shape of the
number, not by where it was taken.* `claims/0005` is the precedent for a count
gating on the machines this project has. Device bytes per application byte,
bytes re-chunked per application byte, copies per read **and resident bytes per
unit of work** are counts and gate; resident bytes was `pending` on the machine
in an earlier draft, which was a misclassification — a byte count is the same
number on a fast host and a slow one, `bench::Environment::classify` refuses
timings and this is not one. Read-path latency and the generation-swap pause
are timings and are `pending` on `E0-D10`'s machine.

*A threshold written before a number exists must still be a number.* Claim
0005's precedent works because every threshold there is a zero or a one, and
neither write amplification nor bytes re-chunked has a natural zero, so "a
threshold written before a number exists" with a blank in it is exactly what
`claims/README.md` rule 5 exists to make visible. The thresholds are therefore
derived here, in the spec, with their derivations, so that the plan copies
rather than invents:

- `device_bytes_per_app_byte` ≤ **1.5**. Derivation: 1.0 for the data, plus
  `f/(1−f)` = 0.33 copy-forward bytes per new byte at RFC 0059's sweep
  threshold `f` = 0.25, plus ≤ 0.06 for padding to `block_bytes` at
  `CHUNK_TARGET_BYTES` = 64 KiB and 4 KiB blocks, plus root-record overhead of
  one block per publish amortised over the publish. That totals 1.39; the
  threshold is 1.5 and the 0.11 is margin, said as margin.
- `ratio_vs_baseline` ≤ **1.0** against f2fs in zoned mode on the identical
  device. Anything above 1.0 falsifies section 04's sentence that doing the
  collection once with knowledge beats doing it twice blindly, which is the
  point of measuring it.
- `bytes_rechunked_per_edit`, chunked kind ≤ **786 432 bytes**
  (`CHUNK_MAX_BYTES` 256 KiB + `RESYNC_BOUND_BYTES` 512 KiB).
- `bytes_rechunked_per_edit`, extent kind ≤ **1 048 576 bytes** — exactly
  `EXTENT_BYTES`, which is the exit's *bounded by the copy-on-write granularity
  and not by the object's size*, and which is also 256× for a 4 KiB write: the
  number the skeptic reaches for, recorded rather than argued.
- `copies_per_read` = **0**, and `resident_bytes_per_read_byte` ≤ **1.0**: a
  page cache holding a second copy would make it at least 2, which is the
  sentence being tested.

Denominators are defined once so that two claims cannot mean different things
by them: **an application byte is a byte the client submitted on the objects
ring; a device byte is a byte crossing the blk ring in a `WRITE` or
`ZONE_APPEND` entry.** Both are counted at ring boundaries, which is the only
place both sides can be counted the same way, and block padding, index log
entries and root records are therefore device bytes and not application bytes.

*And the image digest is in every reproduction command*, which today it is in
none of the fifteen. The mechanism: the base moves to trixie for QEMU 8's zoned
device in its own commit and is pinned there by digest in `ARG BASE`, as
`docker/README.md` already sketches; `cargo xtask claim` records the image
digest beside `commit` in the artefact header and prints it in the reproduction
line; and the fifteen existing claims are re-run in the trixie image in the
same commit, which changes no count — they are simulator and boot counts — and
is the evidence that the base move is not a measurement change. `E2-P06` is the
job that names a non-reproducible input and the image must not be the first one
it names.

**5. Decisions.** Four RFCs are owed and numbered — **0012**, **0058**, **0059**
and **0060** — and each contradicts or extends `docs/design/`: RFC 0012 changes
the rollback metric `the-long-plan` states, RFC 0058 makes section 04's *real
complication* a design with a granularity in it, RFC 0059 makes the collector's
three sentences invariants a test can assert with a number in the third, and
RFC 0060 reverses section 04's *atomicity is free because a root is a single
write*. The fourth is the one no task line names, and that is reported rather
than fixed by editing `TODO.md`. Two more may become owed and are named so that
they are noticed: a decision about the quiescent point if the ring's cursors
turn out not to be enough and `f_abi::transfer` grows a state machine; and a
decision about the assembler's home if `E1-B05`'s policy has not left the frame
when `E2-B05` is otherwise ready. No number is claimed for either. RFC 0013's
*what would reverse this* names E2's crash-consistency work as the evidence
that could reverse per-node atomicity, and that section is read again when
`E2-P01`'s sweep has run.

## Not in scope

**Waits for E1, and does not start until the named task closes.** These are
E2's tasks; the graph says what they wait for, and an earlier draft of this
list stopped at the tasks that name an E1 id directly, which hid the five that
wait one hop away.

- `E2-B05` and `E2-B06` need `E1-B05`, the restart policy leaving the frame.
  The assembler is written as the parent of a supervisor at ring 3 — the
  design, not the way E1 went — and it waits for that move rather than landing
  in the frame as a second owed reversal.
- `E2-B07` needs `E2-B05`, and `E2-P07` needs `E2-B05`, so both wait on
  `E1-B05` through it.
- `E2-P08` needs `E2-B06`, and so waits on `E1-B05` too.
- `E2-R01` needs `E2-B07`, `E2-P07` and `E2-P08`, so the release chain runs
  through `E1-B05` whatever else is done first.
- `E2-D04`'s schema lands in *Decide*, but the task closes only when `E2-P08`
  swaps the driver, which is after `E1-B05`. It is a decision whose exit has a
  proof in it.
- `E1-B15` needs `E1-B05` (and `E0-B14`, which is closed). It is in scope in
  this epoch and it waits with the assembler.
- `E2-P05` needs `E1-B15`, and so waits behind it. A whole-system state is one
  only when every component publishes into it.
- `E2-B08` needs `E1-B10`. Zero copies per read is a count over a *registered*
  buffer; on an inline path the copy nobody counted is the whole number.
- `E2-P04` needs the frame to stop moving, which is `E0-B12`, `E0-B15`,
  `E1-B05`, `E1-B10` and `E1-B14`, plus the five reversals `lint-owed` reports,
  checked by hand.
- `E2-P09`'s second half needs `E0-P06`'s gating claim to have run repeatedly
  on the named machine; the first half runs on synthetic history.

`E2-P01` is deliberately absent from this list: `E1-P01` is `[x]`, so its only
open need is `E2-B01`, and the power-cut model is its own first deliverable.
`E2-P05` is here for `E1-B15` and not for the model — an earlier draft ordered
it after the cut sweep, which the graph does not say and which state comparison
has no use for.

**Waits for hardware.** A hardware root of trust — a TPM measurement of the
frame before the frame runs — is E5's, where the machine is. `E2-B07` in this
epoch closes for every modification to the generation *and* for a modification
to the frame image, because the frame recomputes its own image hash at boot and
refuses to publish a root that disagrees; what stays open is a frame modified
to lie about that recomputation, which is the case a TPM closes and which the
attestation story states. Zoned hardware for `E2-P10` is E5's task whose exit
re-measures the E2 storage claims on real hardware with the emulated numbers
retained beside them. Both timing claims stay `pending` on `E0-D10`'s machine.

**Not this epoch at all.**

- **A compositor, a shell, or anything that reads the index for a person.**
  The index answers queries over a ring; E3 puts a face on it.
- **ABI compatibility across releases.** The root record, the five new blk
  opcodes and `f_abi::transfer` are new wire under ordering rule 1; the matrix
  against N−1 peers is E4.
- **Delta transfer between machines.** Content addressing makes it possible and
  RFC 0012 will say so; building it is E4's platform work.
- **Renaming `user/store`.** Stated under *Decisions*; it is a change to closed
  tasks' evidence and it is not made here.
- **A mutable path into the state tree.** RFC 0013 declines one and nothing
  here asks again; the swap is opcodes on a control ring.
- **Unbounded generation history.** Two root zones and `ROOT_CARRY` bound how
  far back a rollback reaches without pinning; a design that keeps every
  generation forever is a retention policy and it is not this epoch's.

## Evidence

Grouped by the command that observes it.

**`cargo xtask verify`** — the floor. Nothing in this epoch may make the seven
`user=` boots, the `cap=` boots, the six faults, the mutation build, the litmus
job or the seed sweep optional, and the intent names this as the temptation an
epoch that adds a power-cut sweep will feel.

**`cargo xtask lint`** — `lint-units` with its path set widened from `abi/` to
`blob/` and `generation/`, so the on-disk format and the expression are held to
R03 by the same check that already covers the record types in
`abi/src/store.rs`; `lint-determinism` finding nothing in the five libraries;
`lint-unsafe` reporting the heap's blocks in `ring/`, the frame's self-hash
read, and none above the frame; `lint-claims` and `lint-claim-owners` for the
six new entries, and `lint-claims` learning the `[hardware] emulated` key and
refusing a zoned-device claim without it in the same change as the claim;
`lint-manifests` refusing two drivers that match one device; the
`generation --decompile` round-trip fixpoint over every `generation.toml`;
`lint-arch-tests` and `PORTABILITY` rows for `f-hash`, `f-blob`, `f-zone`,
`f-index`, `f-generation`, `f-objects` and `f-assembler` — all seven `None,
None`, because every one of them is `no_std` and builds for
`aarch64-unknown-none` and `x86_64-unknown-none` and tests on the host, the
pattern `env/` and `ring/` set. A crate that cannot needs a row with a reason
and a reversal in `xtask/src/main.rs`, and this spec expects none.

**`cargo xtask test`** — the host tests, on both architectures' runners.
`hash/`: FIPS 180-4's vectors and the block-boundary lengths, moved from
`pack.rs`. `abi/`: the record codecs' golden bytes, the compile-time assertion
that each record's encoded length equals the sum of its fields, and
`from_bytes` refusing a bad magic, an unknown kind and an out-of-bound length
before any field is read. `blob/`: `E2-P02`'s property test over the four
content mixtures with the two-sided bound asserted, and `E2-B01`'s million
blobs written, read back and verified against a modelled device. `zone/`:
`E2-P03`'s three invariants under adversarial allocation with a hard-class
reader and a multi-zone publish in flight. `index/`: `E2-B03`'s query, **device
blocks read per query against device blocks read per tree walk over the same
data**, asserted as a bound against the modelled device. `generation/`: the
same expression folded twice and compared, a one-byte change in any leaf
changing the root, and non-canonical source refused with its diff printed.

**`cargo xtask generation`** (NEW) — compiles `generation.toml` through the
manifest checker to the record tree, prints the root, and packs the boot module
named by it; `--decompile` renders a tree back to canonical source;
`--install` writes a `menuentry` per generation. `E2-P06`'s weekly job runs it
on two runners on two dates, with `--remap-path-prefix` configured, and diffs
the leaves before the root.

**`cargo xtask cut`** (NEW) — `E2-P01`'s sweep: every write boundary and every
block boundary within a write, at `block` and `byte` granularity, in `honest`
and `lying` mode, across the seed set, each cut re-runnable from
`(seed, cut, granularity, mode)`, the mount afterwards finding one of two
roots, and the artefact carrying the four required observations — a torn record
refused by `check`, a root refused by resolution, a root-zone wrap crossed, and
exactly two distinct outcomes. This verb joins the nightly cadence beside
`sweep`, as does `E2-P04`'s Verus job.

**`cargo xtask rollback` and `cargo xtask swap`** (NEW) — the two boots gate G2
names, and **neither verb can run before `E1-B05` closes**, because both go
through `E2-B05`. `rollback` breaks a generation, reboots with
`-append f.root=<hex>` naming the previous module, and compares the restored
root, its module hash and every blob under it; `swap` replaces
`user/virtio-blk` under the chaos workload with `kills = 0` and a swap instead,
and asserts claim 0005's zeros plus one more: `operations_redone` is zero,
because a swap discards nothing.

**The zoned boot** — `E2-B02`'s evidence, and it is a boot and not a library
test because the exit says *a full fill-and-collect cycle on a zoned device or
its emulation, with write amplification recorded*, and `zone/`'s host tests are
`E2-P03`'s invariants against a modelled device. So B02 closes on a QEMU boot
of `user/objects` over the blk driver's ring against the zoned virtio-blk
device, filling every zone, sealing with `ZONE_FINISH`, copy-forwarding and
resetting at least one with `ZONE_RESET`, with device bytes written recorded in
the artefact. It is the same boot `write-amplification` measures, named here so
that the claim and the build task are not two runs of the same thing.

**`cargo xtask claim <name>`** — six entries, registered at the moment the
capability they measure is built. The numbers are next-free at registration and
the names are the stable handle.

- **0016 `write-amplification`** (`E2-P10`): `device_bytes_per_app_byte`, a
  count, `gating`, `max = 1.5`; `ratio_vs_baseline` against
  `claims/baselines/linux-6.x-tuned-zoned/` (f2fs in zoned mode, applied by a
  script inside a guest that script builds), a count, `gating`, `max = 1.0`,
  both sides counted by QEMU `query-blockstats` on the identical emulated zoned
  device, with `[hardware] emulated = true` beside `runner` and copied into the
  artefact header. Two workloads: the phased sequential fill-and-collect, and
  `E2-D02`'s random write on both object kinds.
- **0017 `bytes-rechunked-per-byte`** (`E2-B09`): `bytes_rechunked_per_edit`,
  counts, `gating`, on the chunked kind (`max = 786 432`) and the extent kind
  (`max = 1 048 576`), at two object sizes so that a number scaling with the
  object fails, and with the zero-filled workload as its own row so that the
  candidate-free case is recorded rather than averaged away.
- **0018 `copies-per-read`** (`E2-B08`): `copies_per_read`, `max = 0`, counted
  on both sides of the boundary and required to agree; `pending` until `E1-B10`
  and `gating` the day it lands.
- **0019 `resident-bytes-per-unit-of-work`** (`E2-B08`):
  `resident_bytes_per_read_byte`, a count, `gating`, `max = 1.0`, read as
  resident pages of `user/objects` per N reads from the frame's state tree.
- **0020 `read-path-latency`** and **0021 `generation-swap-pause`**: timings,
  `pending` on `E0-D10`'s machine, and `bench::Environment::classify` refuses
  to record them here, which is it working. The second is the number RFC 0012's
  metric change will eventually want.

**`cargo xtask changepoint`** (NEW) — `E2-P09`'s first half over synthetic
history by binary segmentation with a CUSUM statistic, no new dependency; the
second half is recorded as waiting on `E0-P06` in the same change.

**`cargo xtask release --dry-run`** — the zoned baseline present as files, the
image digest in every reproduction, and `E2-R01`'s package listing the two
demonstrations as commands.

**A third party** — `E2-R01`'s rollback demonstration from the release image on
a machine nobody here owns, which is the one exit in this epoch that cannot be
closed from inside it.

## Risks and reversal

**The most likely thing to be wrong is the re-chunking bound on candidate-free
content.** The two-sided bound is hard before the edit and resynchronising
after it, and the second clause — the enclosing candidate-free run — is where
the design is bad, on precisely the workload `E2-D02` names. *What would
reverse this:* `E2-P02` failing on the periodic or zero-filled mixture even
with the run clause, or claim 0017's chunked-kind row scaling with the object.
The answer is a chunker whose acceptance does not depend on the previous
boundary at all, which is a change to `f-blob` and to nothing on disk, because
the format stores boundaries as hashes and not as a rule — and because
`gear_label` and the mask widths are superblock fields, a mount can tell that
it happened.

**The two-root-zone wrap bounds how far back a rollback reaches, and the bound
is small immediately after a wrap.** Sixteen generations, by `ROOT_CARRY`, and
up to two zones' worth otherwise. *What would reverse this:* somebody needing
to reach a generation older than the wrap without having pinned it — at which
point the answer is pinning, or a third zone, and both are cheap; what is not
cheap is discovering it during a rollback.

**The generation expression may be too small to describe a real topology.**
Four node kinds and no functions is a deliberate refusal of Nix's language, and
the first time somebody needs the same component twice with a different route
the refusal will be argued. *What would reverse this:* a topology `E2-B05`
cannot express as a tree of records. The reversal is a fifth node kind and not
a language; the kinds are enumerated in `abi/`, so a fifth is a diff there and
an RFC by rule.

**A swap under load may need a state machine where this spec has a few words.**
The quiescent point is the ring's cursors and a place that holds pends;
`f_abi::transfer` is a record and not a protocol. *What would reverse this:*
`E2-P08` observing a dropped operation, or the driver from E1 being unable to
declare a record without being rewritten. Then quiescence is a declaration each
component makes and `E2-D04` grows into the state machine the intent asked
about, which is a decision and an RFC.

**The heap in `ring/` is the frame widening by a shape and not by a crate.**
Seven components can now allocate and nothing above the frame reviews how.
*What would reverse this:* `cargo xtask unsafe` moving by more than the
allocator's own blocks, or a component whose correctness turns out to rest on
allocation order — which is the nondeterminism the lint cannot see and the
trace diff can.

**The barrier may be a promise the device does not keep.** `FLUSH` is an
acknowledgement, not a proof, and nothing above the device can tell an honest
cache from a lying one. The format's answer is verify-before-accept and the
cost is a rollback rather than a corruption; the model's answer is that
`E2-P01` runs in `lying` mode as well as `honest`. *What would reverse this:*
mount-time resolution of a generation tree becoming expensive enough to
measure, at which point trusting `FLUSH` is the alternative and it needs a
claim of its own.

**The baseline guest may be slow enough that nobody runs it.** Nested KVM is
not available in every container and TCG is an order of magnitude slower.
*What would reverse this:* `write-amplification --baseline` taking long enough
that it drops out of the routine loop — in which case it moves to the nightly
cadence beside `cut`, and the ratio is recorded there rather than weakened.

**Nothing here produces a timing, for the third epoch in a row.** Four counts
gate and two timings queue behind `E0-D10`. *What would reverse this:* the
machine. Until then `docs/TESTING-STATUS.md` carries the count and the
`lineage-and-debts` rows for Nix, git and Unison are measured or downgraded on
the counts alone — which is enough to lose honestly on the random-write row if
the extent number says so, and claim 0017's extent row is where that loss would
be recorded.

**`E2-B09` may slip, and the release has to decide what it ships.** Carried
from the intent, not answered: if the extent kind is not built when everything
else in `E2-R01`'s list is, 0.3 either ships with the bad-workload number
measured on the chunked kind and the extent path recorded as absent, or waits.
The graph says wait. The originator may prefer the honest weaker shape, and
that is why it is in the list below.

## Decisions taken on the originator's behalf

1. **The store is not called `store`, and `user/store` keeps its name.**
   Renaming the chaos occupant re-baselines every boot-log fixture, claim
   0005's workload and four RFCs that name it, which is a change to closed
   tasks' evidence — the argument RFC 0054 already made about a fourth crate.
   The libraries are `f-hash`, `f-blob`, `f-zone`, `f-index`, `f-generation`;
   the components are `f-objects` and `f-assembler`. The intent's first open
   question, answered with a reason.
2. **Libraries at the top level, one component above them, a ring on both
   sides.** Not a library the blk driver links: a store linked into a driver
   dies with a device fault and shares its restart budget, and the collector
   would be killed by claim 0005 three times per run. `E2-B03`'s *without
   crossing a component boundary* therefore has one boundary in it — client to
   `user/objects` — and none between the index and the store, and **that is a
   reading weaker than the words, flagged rather than assumed**; the comparison
   the exit asks for is device blocks read per query against per tree walk,
   which is a count that can differ. The other half of the first open question.
3. **The expression is a tree of records, evaluated on the host, checked on the
   machine, and the compiler is invertible.** The intent's second open
   question, answered: the smallest thing that meets `E2-B04`'s exit. The
   canonical form is defined in this spec rather than left to a TOML crate's
   table ordering, and `--decompile` plus a fixpoint lint is the tripwire,
   because the refusal that would decay is on the source and not on the
   artefact.
4. **`alloc` enters the workspace, and the allocator is `ring/`'s.** The task
   asked for `no_std + alloc` libraries and this is what that costs: one heap
   in the frame under RFC 0033's shape. The alternative — fixed tables the way
   `env/src/sim.rs` does it — was rejected because a million-blob index is not
   a fixed table, and it is the decision in this spec most worth the
   originator's disagreement.
5. **The quiescent point is the ring's, plus a place that holds pends.** The
   intent's fifth open question, answered provisionally with its reversal
   stated in *Risks*.
6. **The assembler waits for `E1-B05` rather than landing in the frame.** The
   intent's fourth open question: the first of its two shapes is chosen and the
   second is refused, because two owed reversals in the frame is how E1 went
   and the point of naming it was not to go that way twice.
7. **A publish is a barrier sequence, the root zone is two zones, and the root
   record carries a check field.** The three pieces of the crash-consistency
   story that an earlier draft assumed and nothing enforced: there was no flush
   opcode, a single sequential root zone could not be re-pointed or reset
   without a window with no root, and a torn record had nothing to be refused
   by. Correctness rests on verify-before-accept rather than on the device
   keeping its word, and the barrier is what makes the fallback rare.
8. **The tuned zoned baseline is f2fs in zoned mode, and it runs in the
   container.** A named filesystem had to be chosen for the baseline directory
   to exist; btrfs's zoned mode is the live alternative and the directory's
   README says so. The baseline runs as a Linux guest on the identical emulated
   device rather than being deferred to hardware, because the number is a count
   and claim 0001's ratio is `pending` for a timing's reason, which does not
   carry over. `E2-P10` therefore closes in this epoch and `E2-R01` is not cut
   by it.
9. **`E2-B07` closes for the frame too, by the frame hashing its own image.**
   The exit says *any modification*, and publishing a handed-down hash would
   have met that for everything except the publisher. The build computes the
   frame image's hash, the frame recomputes it at boot and refuses to publish a
   disagreeing root, and `cargo xtask mutate` gains a frame mutation that must
   change it. The residual — a frame modified to lie about its own
   recomputation — is E5's TPM and is named in the attestation story.
10. **Six claims, not two, and every threshold is a number.** The two counts
    the task requires, plus `copies-per-read` registered `pending` on `E1-B10`,
    `resident-bytes-per-unit-of-work` as a count rather than as a timing, and
    two timings registered `pending` on the machine — R11, registering the
    apparatus with the capability rather than when a number happens to exist.
    Two of the thresholds have no natural zero, so they are derived in *Policy
    applied* rather than promised.
11. **A fourth RFC, 0060, is owed and no task line names it.** The publish
    ordering reverses `deadline-all-the-way-down` section 04's *atomicity is
    free*, and a reversal is an RFC. It lands with `E2-B01` and carries the
    five new blk opcodes with it. Reported rather than fixed, because `TODO.md`
    is not edited here.
12. **`E2-B09`'s slip is not decided here.** Carried forward; the graph says
    the release waits, and the originator may prefer the weaker honest shape.
