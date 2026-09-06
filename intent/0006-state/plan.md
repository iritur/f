---
id: 0006
status: in-progress
spec: ./spec.md
---

# Plan: twenty-five tasks, and a first slice built to be wrong early

Not one pull request, for the reason intent 0005's plan gave: E2 is an epoch
and its tasks close individually against their own exits. What this plan fixes
is the shape of the diff for the first slice and the order the slice is built
in, so that a task landing on its own does not find a crate it needed built
differently by the one before it.

The first slice is the part of E2 nothing else can start without: the three
RFCs the task lines already number (`E2-D01`, `E2-D02`, `E2-D03`) and a fourth
no line names (RFC 0060, which the revised spec found owed), the hash and the
blob store (`E2-B01`, with `E2-P02` as its specification, and with the five new
blk opcodes RFC 0060 carries), and the evaluator (`E2-B04`). Everything else
in the epoch is named by the hash or waits on E1, and each of those tasks gets
one paragraph at the end saying what it will touch and what it waits for — not a row, because a row for a file that
will be built after `E1-B05` moves is a row written about a design that may
not survive `E1-B05` moving.

Marked `NEW` means the path did not exist when E2 started.

## Files

### Decide — four RFCs, three at the numbers they were owed and one nobody owed

```
docs/rfc/0012-an-update-is-a-generation-swap-and-the-root-is-the-attestation.md
                                            NEW: E2-D01. The number TODO.md
                                            reserved when the line was written,
                                            which is why 0011 is followed by
                                            0013 in this directory. An update is
                                            a generation swap, the root hash is
                                            the attestation, and the rollback
                                            metric becomes "one generation swap,
                                            and a reboot only when the frame
                                            changed" — which is why the root
                                            record carries the frame's hash as
                                            its own field. It also has to decide
                                            the thing this plan found while
                                            reading: a component file's spawn
                                            identity is FNV-1a over 64 bits
                                            (`abi::manifest::ContentId`,
                                            `xtask::manifest::content_id`), so
                                            "one identity" has a third identity
                                            to name. See Risks.
docs/rfc/0058-a-mutable-extent-is-a-second-object-kind-and-not-a-unification.md
                                            NEW: E2-D02. Copy-on-write at
                                            `EXTENT_BYTES` = 1 MiB,
                                            content-addressed at a snapshot
                                            boundary only, and the random-write
                                            workload it is bad at — VM images
                                            and sparse database files, named with
                                            its write size — so E2-B09 measures
                                            the workload the design chose. It also
                                            has to say why the obvious alternative
                                            was refused: a per-piece dirty log
                                            absorbed until snapshot, which would
                                            cut the small-write number by two
                                            orders of magnitude and would put
                                            mutable state that is not
                                            content-addressed in front of the
                                            store. That is the first thing a
                                            reader who dislikes 256× proposes, so
                                            it is answered before it is asked.
                                            Written after the chunker exists, so
                                            its comparison paragraph quotes
                                            `CHUNK_MIN_BYTES`,
                                            `CHUNK_TARGET_BYTES` and
                                            `CHUNK_MAX_BYTES` in blob/src/chunk.rs
                                            rather than the ones the design page
                                            guessed at
docs/rfc/0059-a-collector-is-three-invariants-and-a-batch-class-consumer.md
                                            NEW: E2-D03. Nothing reachable is
                                            swept; a reset zone holds no live
                                            blob; collection never starves a
                                            deadline-class read. Invariants, not
                                            behaviour, because E2-P03 asserts
                                            them. The first ranges over pinned
                                            roots *and open publishes* — a publish
                                            registers as a transient root at its
                                            first write and is released only when
                                            its root record is durable — without
                                            which a multi-zone publish has its
                                            first zone swept out from under it
                                            while all three invariants hold. The
                                            third is a count or it is not
                                            assertable at all:
                                            `collector_operations_ahead_of_hard_
                                            read` = 1, which is claim 0012's
                                            `in_flight` rather than an invented
                                            number, and it moves when E1-B09
                                            raises that one, re-derived in the
                                            same diff. `SWEEP_LIVE_FRACTION` =
                                            0.25 is the fourth number it fixes,
                                            and claim 0016's 1.5 is derived from
                                            it. Roots are pinned explicitly, and
                                            the RFC says what pins one
docs/rfc/0060-a-publish-is-a-barrier-sequence-and-atomicity-is-not-free.md
                                            NEW, and owed by the spec rather than
                                            by TODO.md: no task line names it,
                                            which is reported to the originator
                                            and not fixed by editing that file. It
                                            reverses `deadline-all-the-way-down`
                                            section 04's *atomicity is free
                                            because a root is a single write*, and
                                            a reversal is an RFC. It carries three
                                            things: the sequence — blobs, FLUSH,
                                            root record, FLUSH — the five opcodes
                                            that make it expressible, and
                                            verify-before-accept, which is what
                                            keeps the format correct on a device
                                            that acknowledges a barrier it did not
                                            honour. The cost of a lying device is
                                            stated as a rollback rather than a
                                            corruption, and the reversal is
                                            generation trees large enough that
                                            mount-time resolution is itself worth
                                            measuring. It lands inside E2-B01's
                                            diff, before f-zone exists to depend
                                            on it
docs/rfc/README.md                          a "Reserved numbers" section with one
                                            row per RFC above: number, task,
                                            title, status. Four rows now, and
                                            0060's task column says *none* — which
                                            is the only place in the tree an RFC
                                            with no task line is visible, and the
                                            reason the section is worth more than
                                            an index. E1's plan promised "the
                                            index row per RFC" and landed none,
                                            because the file has no index —
                                            thirteen lines of rule. What it lacks
                                            and a reader trips on is why 0012 was
                                            a gap for two epochs; the rows are
                                            where that is said, and a full index
                                            of sixty entries is somebody else's
                                            change
docs/what-must-be-stated.html               the three sentences that quote
                                            "full system rollback: one reboot"
                                            (as the concession, the table row
                                            and the payoff) point at RFC 0012.
                                            A sentence, not a number, so no
                                            claim moves
docs/design/deadline-all-the-way-down.html  section 04: the "real complication"
                                            paragraph gains the granularity RFC
                                            0058 fixes, the collector's three
                                            sentences become the prose form of RFC
                                            0059's invariants, and *atomicity is
                                            free because a root is a single write*
                                            is struck and replaced by a sentence
                                            citing RFC 0060. Each cites its RFC.
                                            No number in any of the three
```

### Build — the wire, the hash, the chunker, the fold, then the format

```
user/virtio-blk/src/driver.rs               the `op` module gains five constants:
                                            FLUSH = 3 (VIRTIO_BLK_T_FLUSH),
                                            ZONE_APPEND = 4, ZONE_FINISH = 5,
                                            ZONE_RESET = 6, ZONE_REPORT = 7, each
                                            with the virtio request type named
                                            beside it. Here and not in abi/
                                            because this module is already the
                                            shared definition both peers read —
                                            kernel/src/blk.rs imports
                                            f_virtio_blk::driver — and a second
                                            place to look up an opcode is how two
                                            peers come to disagree about one. New
                                            wire under ordering rule 1, so the
                                            numbers are fixed in this slice and
                                            the behaviour is not: E2-B02 answers
                                            them, and until it does the driver's
                                            existing unknown-opcode refusal covers
                                            all five — which is what `cargo xtask
                                            blk` must still show green. RFC 0060
Cargo.toml                                  three members — "hash", "blob",
                                            "generation" — and three
                                            [workspace.dependencies] rows,
                                            f-hash, f-blob, f-generation, plain
                                            path dependencies with no
                                            default-features line: none of the
                                            three is a component with an image
                                            feature, so RFC 0054's shape does
                                            not apply and saying so beside the
                                            rows is the whole comment. `f-env`
                                            already has a row and gains nothing;
                                            what changed is who takes it, and that
                                            is blob/Cargo.toml's line. Nothing
                                            in `exclude` changes
hash/Cargo.toml                             NEW: package `f-hash`, no
                                            dependencies, `[lints] workspace =
                                            true`, so it inherits the forbid.
                                            env/Cargo.toml is the pattern: a
                                            no_std library that tests on the
                                            host with std's test harness
hash/src/lib.rs                             NEW: `#![no_std]`. SHA-256 as FIPS
                                            180-4 states it: `Sha256` with
                                            `new`, `update(&[u8])`,
                                            `finish() -> [u8; 32]`, and
                                            `sha256(&[u8]) -> [u8; 32]` over it.
                                            The streaming state is the point —
                                            the chunker hashes bytes as it
                                            scans them and never holds a chunk.
                                            No allocator, no feature flags. The
                                            K table and the compression function
                                            move from xtask/src/pack.rs with the
                                            two vectors the standard publishes
                                            and the block-boundary lengths; one
                                            new test feeds a 200-byte input
                                            split at every position and asserts
                                            the stream equals the one-shot,
                                            because a streaming state that is
                                            wrong is wrong at a block boundary
                                            it never sees in a one-shot test
abi/src/store.rs                            NEW: the on-disk record types, their
                                            codecs, and the four generation node
                                            kinds. Here rather than in blob/ for
                                            the reason the spec gives: the only
                                            use for #[repr(C)] is to view device
                                            bytes as a struct, that view is a
                                            pointer cast, and a library above the
                                            frame may not write one. So there is
                                            no repr(C) anywhere in this file —
                                            `Superblock`, `Header`, `Kind`,
                                            `ObjectHead`, `RootRecord` and the
                                            `bytes`/`component`/`topology`/
                                            `generation` node kinds are
                                            fixed-width and little-endian, encoded
                                            by hand and decoded field by field by
                                            a validating `from_bytes(&[u8; N]) ->
                                            Result<_, i32>` that refuses on magic,
                                            kind and length bound before it
                                            returns any field, because a peer
                                            wrote these bytes in the sense
                                            ring/src/mapping.rs means — the device
                                            did. Each record's encoded length is
                                            asserted equal to the sum of its field
                                            widths at compile time, so no padding
                                            is ever hashed. Every public field's
                                            doc comment carries `Unit:`, which is
                                            what lint-units already holds abi/ to,
                                            and that is the second reason the
                                            types are here. The superblock carries
                                            the geometry *and* the chunker's
                                            parameters — chunk_min_bytes,
                                            chunk_target_bytes, chunk_max_bytes,
                                            mask_strict_bits, mask_loose_bits,
                                            gear_label — plus root_zone_a and
                                            root_zone_b; the root record carries
                                            generation (zero reserved, so a zeroed
                                            block is never one), root, frame,
                                            module, previous and `check`, the
                                            SHA-256 over every preceding field.
                                            Golden-bytes and refusal tests are an
                                            inline `#[cfg(test)] mod tests`,
                                            because that is abi/'s pattern and
                                            abi/ has no tests/ directory. No
                                            timestamp anywhere; the generation
                                            counter is the order and RFC 0004 is
                                            why. It lands in two commits — the
                                            node kinds with the fold, the store
                                            records with the format — and Order
                                            says why
abi/src/boot.rs                             NEW: the selection token's grammar,
                                            `f.root=<64 hex>`, with a `Unit:` on
                                            each field. In this slice and under
                                            ordering rule 1 rather than with
                                            E2-P07, because the boot module `cargo
                                            xtask generation` packs is named by
                                            the same hash the token names, and a
                                            grammar written later by a different
                                            hand is the second reader of the
                                            format the spec forbids. Nothing
                                            parses it yet; the frame does at
                                            E2-P07
abi/src/lib.rs                              `pub mod store;` and `pub mod boot;`,
                                            each in the commit its module lands in
blob/Cargo.toml                             NEW: package `f-blob`.
                                            [dependencies] f-hash, f-abi for the
                                            record types, and f-env — a real
                                            dependency and not a dev one, because
                                            gear.rs derives its two tables from
                                            `f_env::split`'s `const fn`s at
                                            compile time. The store still draws
                                            nothing from an `Env` on any path at
                                            run time and the crate doc says so;
                                            the tests draw objects and edits from
                                            one. `[[test]] name = "million",
                                            harness = false`, for the reason
                                            ring's `hostile` gives: the count is a
                                            command-line argument so the gate and
                                            the exit can differ by a number rather
                                            than by a build
blob/src/lib.rs                             NEW: `#![no_std]`,
                                            `extern crate alloc`. The crate doc
                                            states what a blob is, and states the
                                            bound in the strength the spec gives
                                            it and not a word more: hard before
                                            the last boundary preceding X − 64;
                                            resynchronising within
                                            `RESYNC_BOUND_BYTES` of X + L **or at
                                            the end of the enclosing candidate-
                                            free run plus one chunk, whichever is
                                            later**. The second clause is the
                                            honest half and it is written here
                                            rather than found by a user: in
                                            content with no candidates at all
                                            every boundary is forced at the
                                            maximum, a forced cut is by definition
                                            relative to the previous boundary, and
                                            two streams offset by L do not
                                            resynchronise until the run ends. The
                                            cause is maximum-size forcing and not
                                            the minimum, and the reversal an
                                            earlier draft wrote — acceptance
                                            independent of the previous boundary —
                                            does not fix a forced cut; the
                                            paragraph says so, so that nobody
                                            reaches for it. It also states that
                                            this crate names `f_env::split` at
                                            compile time and never at run time.
                                            `alloc` and not a fixed table, because
                                            an object's chunk list is the first
                                            variable-length thing in the
                                            workspace; spec decision 4, and see
                                            Risks for what it does and does not
                                            cost here
blob/src/gear.rs                            NEW: `pub const GEAR: [u64; 256]` and
                                            the two masks, all derived by
                                            `const fn` from `f_env::split::Stream
                                            ::from_seed(f_env::split::label("f-
                                            blob gear v1"))` and the same stream
                                            under "f-blob mask v1", with bit 63
                                            forced set. One derivation, which is
                                            RFC 0026's whole argument: a local
                                            SplitMix64 here would be the third
                                            transcription in the tree
                                            (kernel/src/env.rs is the second) and
                                            a second generator for a reviewer to
                                            check against the paper. Changing
                                            either label changes every object
                                            hash, which is why `gear_label` is a
                                            superblock field and the sentence
                                            saying so lives beside the constant.
                                            Three tests, each a property a two-
                                            kilobyte literal could not be checked
                                            for: the table has no zero entry — a
                                            zero makes a byte invisible to the
                                            register — no two equal entries, and
                                            **the highest set bit of each mask is
                                            bit 63**, which is the assertion that
                                            keeps the 64-byte window from silently
                                            becoming a 16-byte one
blob/src/chunk.rs                           NEW: the chunker, FastCDC's shape on a
                                            gear register, normalised.
                                            `CHUNK_MIN_BYTES` = 16 KiB,
                                            `CHUNK_TARGET_BYTES` = 64 KiB,
                                            `CHUNK_MAX_BYTES` = 256 KiB,
                                            `MASK_STRICT_BITS` = 18 below the
                                            target and `MASK_LOOSE_BITS` = 14
                                            above it, `RESYNC_BOUND_BYTES` =
                                            512 KiB, each a `pub const` with a doc
                                            comment stating its unit. Normalised
                                            chunking is adopted because it is what
                                            reduces cuts forced at the maximum,
                                            and the crate says that is the reason
                                            — the forced cut is exactly what the
                                            bound's second clause is about. With
                                            `h = (h << 1) + gear[b]`, bit k of the
                                            register depends only on the last k+1
                                            bytes, so the window is the last
                                            sixty-four bytes *because the highest
                                            mask bit is 63* and for no other
                                            reason; that sentence is in the doc
                                            comment, because an earlier draft
                                            claimed 64 with a mask over the low 16
                                            and was wrong by a factor of four.
                                            `Chunker` carries the register and the
                                            bytes since the last cut;
                                            `feed(&mut self, &[u8]) -> Cuts<'_>`
                                            is an iterator of cut offsets into the
                                            slice fed, `finish(self) -> usize` is
                                            the tail. An iterator and not a
                                            callback, because lint-callbacks says
                                            no interface registers one and this is
                                            an interface. Candidates under the
                                            minimum are ignored, the maximum
                                            forces, and the register is not reset
                                            at a cut
blob/src/device.rs                          NEW: `Device` — `block_bytes()`,
                                            `blocks()`, `read(block, &mut [u8])`,
                                            `write(block, &[u8])` and `flush()`,
                                            each returning a `Result` — and
                                            `Memory`, the modelled device: a
                                            `Vec<u8>` of blocks with a bounds
                                            check and a `flush` that is a no-op.
                                            The barrier is on the trait from the
                                            first commit rather than added when
                                            `f-zone` needs it, because a publish's
                                            correctness is an ordering and a trait
                                            that cannot express one pushes the
                                            ordering into every caller. The honest
                                            and lying modes are not here: they are
                                            E2-P01's wrapper, for the same reason
                                            the cut model wraps this type rather
                                            than writing a second one — RFC 0034's
                                            rule that a device model is a peer on
                                            the real types, applied one level down
blob/src/store.rs                           NEW: `Store<D: Device>` over
                                            `f_abi::store`'s record types.
                                            `put(kind, content) -> Hash` writes
                                            header, content and padding to
                                            `block_bytes`; `get(hash, &mut [u8])`
                                            reads, decodes the header with
                                            `from_bytes`, hashes the content and
                                            refuses a mismatch — **no byte from
                                            the device is trusted until that
                                            constructor returned Ok**;
                                            `put_object(bytes)` runs the chunker
                                            and one `Sha256` over the same bytes,
                                            writes each chunk and then the object
                                            blob — the `object_bytes` total and
                                            the ordered chunk hashes, which is
                                            what makes an object one hash — and
                                            `get_object` walks the list. The
                                            publish sequence's first half is here:
                                            the blobs, then `Device::flush`. Its
                                            second half is not, because a root
                                            record is appended to a root *zone*
                                            and this crate knows nothing about
                                            zones: E2-B02 puts `f-zone` between
                                            this and the device, and a store that
                                            knew about zones would be two crates
                                            that could disagree about an offset.
                                            Sequential allocation from the first
                                            free block
blob/tests/chunker.rs                       NEW: E2-P02, written before chunk.rs
                                            exists and committed failing to
                                            compile in the same branch, so it
                                            tests the bound and not the code.
                                            **Its generator is a four-way mixture
                                            and not uniform bytes**: uniform,
                                            zero-filled, periodic with a period
                                            below the target size, and
                                            concatenations of those. With uniform
                                            bytes alone a candidate appears every
                                            64 KiB on average, so the bound passes
                                            on the data the design is good at
                                            while the workload it is bad at fails
                                            unobserved. Objects, edits, lengths
                                            and the mixture choice are drawn from
                                            `f_env::split::Stream` children at the
                                            identities "e2-p02/object",
                                            "e2-p02/edit", "e2-p02/length" and
                                            "e2-p02/mixture" under RFC 0026, so a
                                            fifth draw added later moves nothing.
                                            Five properties: every chunk but the
                                            last is in [CHUNK_MIN_BYTES,
                                            CHUNK_MAX_BYTES]; the mean over a
                                            large draw is within a factor of two
                                            of CHUNK_TARGET_BYTES; nothing before
                                            the last boundary preceding X − 64
                                            changes, exactly; the two boundary
                                            sequences agree again within
                                            RESYNC_BOUND_BYTES of X + L **or at
                                            the end of the enclosing candidate-
                                            free run plus one chunk, whichever is
                                            later**; and identical content in two
                                            objects yields identical chunk hashes,
                                            which is the deduplication half of the
                                            task's title. Each is one `#[test]`
                                            with its seed and its mixture in the
                                            failure message
blob/tests/million.rs                       NEW: E2-B01's first exit. Writes N
                                            blobs of drawn content into a
                                            `Memory` device with 512-byte blocks,
                                            reads every one back by hash and
                                            verifies it; then flips one byte in
                                            the device and asserts that exactly
                                            one read now refuses on the content
                                            hash, because a verifier nothing can
                                            fail is the same as no verifier. The
                                            refusal is `f_abi::store`'s, the same
                                            one the golden-bytes tests name.
                                            N defaults to 10 000 under
                                            `cargo test`, which is the per-commit
                                            gate; `--blobs 1000000` is the exit
                                            and it runs in release
generation/Cargo.toml                       NEW: package `f-generation`.
                                            [dependencies] f-hash and f-abi, the
                                            second now for the four node kinds and
                                            their codecs as well as for
                                            `abi::manifest::NAME_MAX`, so a name's
                                            bound is stated once in the tree and
                                            not twice. No alloc: see fold.rs
generation/src/lib.rs                       NEW: `#![no_std]`. The crate doc says
                                            what the expression is not — not lazy,
                                            not recursive, not extensible at run
                                            time; no import, no interpolation, no
                                            conditional, no path, no clock — and
                                            that the machine never evaluates
                                            anything, it recomputes a fold. It
                                            also states the canonical form, since
                                            *the same expression yields the same
                                            root* is only true if the compiler is
                                            canonical, and this crate is where the
                                            ordering is enforced rather than where
                                            it is described
generation/src/record.rs                    NEW: the checker over the node kinds
                                            abi/src/store.rs defines. This crate
                                            holds no codec, which is what makes a
                                            fifth kind a diff to abi/ and an RFC
                                            by rule rather than by convention.
                                            `Tree::check(&[u8]) -> Result<Tree,
                                            Refusal>` refuses an unknown kind, a
                                            name outside `[a-z0-9-]` at NAME_MAX,
                                            a duplicate name, a dangling index —
                                            routes carry indices into the member
                                            list, never offsets — and
                                            **non-canonical order**: components
                                            sorted bytewise on the zero-padded
                                            32-byte name, routes on (component
                                            index, capability name). The
                                            `generation` node is the frame hash
                                            and the topology hash **and nothing
                                            else**, and that is the line this
                                            reconciliation changed: a root
                                            embedding block_bytes and zone_bytes
                                            would be a different root on two
                                            runners with different emulated disks,
                                            so E2-P06 would fail on its first run
                                            and name a leaf that is not a leaf.
                                            Geometry lives in the superblock, and
                                            what a generation may say about a
                                            device is a requirement checked at
                                            mount rather than a value the hash
                                            depends on
generation/src/fold.rs                      NEW: `root(&Tree) -> [u8; 32]`, the
                                            post-order Merkle fold: a node's hash
                                            is the SHA-256 of its encoding with
                                            each child replaced by that child's
                                            hash, streamed into one `Sha256`
                                            state. No padding is ever hashed,
                                            which abi/'s compile-time length
                                            assertion makes true rather than a
                                            comment here. No allocator and no
                                            explicit stack, because the tree's
                                            depth is fixed by its four kinds —
                                            generation, topology, leaf — and a
                                            fold over a tree of known depth is
                                            three nested loops. That is the
                                            argument for why this crate needs no
                                            heap, and it is also the argument for
                                            why a fifth node kind that nests is an
                                            RFC and not a patch
generation/tests/root.rs                    NEW: the same tree folded twice is the
                                            same root; a one-byte change in any
                                            leaf, any name or any route changes
                                            it; two members swapped change it,
                                            because the list is ordered;
                                            non-canonical source is refused rather
                                            than reordered; the refusals in
                                            record.rs each fire on a fixture. No
                                            parameter case any more, because the
                                            generation node no longer carries one.
                                            A worked tree of four components with
                                            two routes is the fixture, and it is
                                            the same tree user/generation.toml
                                            describes
user/generation.toml                        NEW: the source, in canonical form.
                                            The tree's four components by name,
                                            the routes between them by capability
                                            name and source, and the frame. **Not
                                            the store parameters** — those are the
                                            superblock's, and a generation states
                                            requirements rather than values.
                                            Beside the manifests it composes and
                                            not at the root, because it is a
                                            description of the tree under user/
                                            and nothing else
xtask/Cargo.toml                            f-hash, f-abi and f-generation as
                                            dependencies. xtask is std and these
                                            are no_std libraries that build for
                                            the host; this is the arrangement
                                            ring/proofs uses in the other
                                            direction. f-abi is what lets the
                                            generation verb encode a record tree
                                            with the same codec the machine
                                            decodes it with, which is the whole of
                                            "one implementation"
xtask/src/pack.rs                           the transcription goes and
                                            `f_hash::sha256` takes its place;
                                            the module doc's "checked here
                                            against the two vectors" becomes
                                            "checked in hash/, once". Its two
                                            SHA-256 tests move to hash/src/
                                            lib.rs; the archive test stays.
                                            After this commit there is one
                                            SHA-256 in the tree, which is the
                                            sentence RFC 0012 needs to be true
xtask/src/generation.rs                     NEW: `cargo xtask generation`. Reads
                                            user/generation.toml through the
                                            reader manifest.rs already has — one
                                            parser, not two — builds the component
                                            files the way `components()` does,
                                            hashes each with f-hash for its
                                            `component` leaf, encodes the record
                                            tree with `f_abi::store`, checks it
                                            with `f_generation::record`, folds it,
                                            and prints the root and every leaf
                                            hash beside its name. **Source that is
                                            not canonical is refused and the
                                            canonical form printed as a diff**,
                                            never silently reordered, so that two
                                            authors with the same set cannot
                                            produce two roots and no author learns
                                            their file was rewritten by reading a
                                            hash. It then **packs the record tree
                                            and the component files into one boot
                                            module named by the root hash** — RFC
                                            0030's component-file shape one level
                                            up — which is what keeps the loader
                                            ignorant of the on-disk format.
                                            `--decompile` renders a record tree
                                            back to canonical source, and that is
                                            the half that matters: the tripwire is
                                            on the *source*, because no record
                                            kind changes when import and
                                            interpolation return to
                                            generation.toml under the name of
                                            convenience. `--install`, which writes
                                            a `menuentry` per generation into the
                                            GRUB fragment docs/booting-on-
                                            hardware.md documents, is E2-P07's and
                                            is not built here; it is named so the
                                            flag is not invented twice. The leaves
                                            are printed so that E2-P06's job diffs
                                            leaves before roots and names the
                                            input that moved rather than reporting
                                            that two roots differed
xtask/src/main.rs                           seven edits, each one line or a row.
                                            PORTABILITY gains `f-hash`, `f-blob`,
                                            `f-generation`, all `host: None, bare:
                                            None`, in the same commit as the
                                            member lines because `classify` fails
                                            hard on a member with no row.
                                            `lint_units`'s path test widens from
                                            `abi/` to `abi/`, `blob/`,
                                            `generation/` and its message stops
                                            saying "in abi/" — abi/ was already in
                                            the set, which is why the record types
                                            moving there costs nothing and is the
                                            second reason they moved. The
                                            `generation` verb and its `--decompile`
                                            flag join the match and `help()`.
                                            `lint_all` gains the round-trip
                                            fixpoint check over every
                                            generation.toml, beside
                                            `lint_manifests` because it is the
                                            same question one level up. ROUTES
                                            gains `("bytes-rechunked-per-byte",
                                            Route::Bench("rechunk"))`. And four
                                            things deliberately do not change,
                                            each stated in the commit: no
                                            DETERMINISM_ALLOW entry, because
                                            naming `f_env::split` inside a `const`
                                            is not a call site FORBIDDEN matches
                                            and nothing in the three crates
                                            reaches an `Env` at run time; no
                                            DATAPATH or NOT_THE_FRAME row, because
                                            none of the three is a component; no
                                            COMPONENTS change, because none has a
                                            manifest; no UNSAFE_ALLOW change,
                                            because none of the three needs one
                                            and abi/ is already on the list
bench/Cargo.toml                            f-blob as a dependency
bench/src/bin/rechunk.rs                    NEW: claim 0017's workload. Two object
                                            sizes, 8 MiB and 128 MiB, drawn from a
                                            `Stream` child at "rechunk/object";
                                            **the same four content mixtures
                                            E2-P02 draws** — uniform, zero-filled,
                                            periodic below the target size, and
                                            concatenations — at "rechunk/mixture",
                                            so the candidate-free case is its own
                                            recorded row rather than a number
                                            averaged away; 4 KiB writes at offsets
                                            drawn at "rechunk/offset" with bytes
                                            drawn at "rechunk/bytes"; per write,
                                            the object is re-chunked from the last
                                            boundary before the edit and the bytes
                                            scanned and the bytes hashed are
                                            counted per edit. Counts only, no
                                            clock, so `Sample` has nothing to
                                            refuse and the claim may gate in the
                                            container the way 0005 and 0014 do.
                                            The extent-kind half of the run is
                                            E2-B09's and this file gains it then
```

### Prove — the claim, and what is registered now

```
claims/0017-bytes-rechunked-per-byte.toml   NEW: E2-B09's claim, registered at
                                            E2-B01 because its chunked-kind rows
                                            are E2-B01's second exit — "re-chunks
                                            only a bounded region" is a number or
                                            it is prose. 0017 and not 0016,
                                            because the spec's registry reserves
                                            0016 for `write-amplification`, which
                                            lands with E2-B02; the caveat below
                                            about numbers still applies. `gating`,
                                            milestone E2. [metrics] primary
                                            `bytes_rechunked_per_edit_chunked_
                                            large`; secondary the small-object
                                            twin, the two re-hashed rows, the
                                            zero-filled mixture as its own row,
                                            `resync_bytes_max`, and the geometry
                                            (`object_bytes_small`, `object_bytes_
                                            large`, `write_bytes`,
                                            `writes_per_object`) so that a run
                                            which quietly shrank the object cannot
                                            report a bound it did not test.
                                            [threshold] copied from the spec's
                                            derivation rather than invented here:
                                            both chunked rows `max = 786432`,
                                            which is CHUNK_MAX_BYTES 256 KiB +
                                            RESYNC_BOUND_BYTES 512 KiB, the same
                                            number at both sizes, so a number that
                                            scales with the object fails;
                                            `resync_bytes_max = { max = 524288 }`;
                                            the geometry rows as minimums.
                                            [baseline] `none` with the reason: an
                                            absolute statement about what this
                                            chunker was made to do, and the
                                            systems lineage-and-debts compares
                                            against do not expose this count.
                                            [owner] docs/design/deadline-all-the-
                                            way-down.html section 04. [hardware]
                                            runner-class-A with 0014's note that
                                            the claim does not need it.
                                            [reproduce] `cargo xtask claim
                                            bytes-rechunked-per-byte`.
                                            [diagnosis] one entry per threshold.
                                            E2-B09 adds the extent rows —
                                            `bytes_rechunked_per_edit_extent_* =
                                            { max = 1048576 }`, which is exactly
                                            EXTENT_BYTES and is the exit's
                                            *bounded by the copy-on-write
                                            granularity and not by the object's
                                            size*, and is also the 256× a 4 KiB
                                            write costs — in its own diff; adding
                                            rows with their thresholds before
                                            their number exists is R11, moving one
                                            is not
claims/snapshot.json                        the sixteenth entry, written by
                                            `cargo xtask claims`, which
                                            lint-snapshot then holds the tree to
```

The other five claims the spec names are registered when the capability they
measure is built, which is the rule R11 states and this slice builds none of
them: `write-amplification` (0016) with `E2-B02`; `copies-per-read` (0018),
`resident-bytes-per-unit-of-work` (0019) and `read-path-latency` (0020) with
`E2-B08`; `generation-swap-pause` (0021) with `E2-B06`. Two of those moved
since this plan was first written — `resident-bytes-under-load` became
`resident-bytes-per-unit-of-work` and was reclassified from a timing that
waits to a count that gates, because a byte count is the same number on a
fast host and a slow one — and `generation-swap-pause` is new. Their names
are fixed here so two branches cannot register one number under two names;
their numbers are not, because E1 already learned that a number taken in a
plan is a number a concurrent landing takes first (claim 0008 became 0014).
Next free at landing, and the plan row is updated the way E1's was.

### Release, and the list

```
intent/0006-state/plan.md                   NEW: this
TODO.md                                     six lines, each edited at its landing
                                            by the session that lands it and
                                            naming intent/0006-state/: E2-D01,
                                            E2-D02, E2-D03 to [x] on merge, their
                                            exits being "merged"; E2-B01 to [x]
                                            when both commands under Proof are
                                            green, with both quoted on the line;
                                            E2-P02 to [x] in the same change as
                                            E2-B01, because its exit is E2-B01's
                                            second clause over random edits and
                                            sizes and the test closes when the
                                            chunker exists to fail it; E2-B04 to
                                            [>] and not [x], because its exit is
                                            "checked by E2-P06" and one container
                                            running the fold twice is not two
                                            machines on two dates. Six and not
                                            seven: **RFC 0060 has no task line and
                                            this plan does not add one.** It lands
                                            inside E2-B01's diff and the gap is
                                            reported to the originator, which is
                                            what the spec asks for and not an
                                            oversight
```

## Order

Whatever can fail first. The two things most likely to be wrong are the
chunker's bound and the two-machine root, and the on-disk format — the part
that looks like the work — is last, because a format is cheap to write and
expensive to have written around a chunker that then changed. What moved when
this plan was reconciled against the revised spec is that the *wire* came to
the front: a publish is a barrier sequence now, and a barrier is an opcode two
peers read.

1. **RFC 0012**, before any code, for two reasons that are both ordering rule
   1. It is the decision `f-hash` serves: "one identity" is the sentence that
   justifies moving a page of SHA-256 out of xtask rather than leaving it. And
   it has to answer the `ContentId` question before `abi/src/store.rs` fixes
   the width of a `component` leaf — whether the spawn's 64-bit FNV identity
   becomes the low half of the file's SHA-256 (an `abi/` change, cheap now
   while one peer reads it) or stays a separate short name the generation does
   not carry. Either is defensible; writing the leaf before deciding is not.
2. **RFC 0060 and the five opcodes** in `user/virtio-blk/src/driver.rs`.
   Second rather than somewhere near the format, which is the step this
   reconciliation added: `FLUSH`, `ZONE_APPEND`, `ZONE_FINISH`, `ZONE_RESET`
   and `ZONE_REPORT` are new wire two peers read, ordering rule 1 puts wire
   first, and the numbers have to be fixed before anything is written against
   them. The RFC also fixes verify-before-accept, which is what the root
   record's `check` field and the mount's *take the highest verifying
   generation* are written from — so it precedes the format rather than
   explaining it afterwards. No behaviour lands here: the driver still answers
   two opcodes, and `cargo xtask blk` proves that by staying green.
3. **`hash/`, and `pack.rs` calling it.** Small, and everything after it is
   named by it. It can fail on the published vectors and on the streaming test,
   and if it fails there, nothing else has been built on it yet.
4. **`blob/tests/chunker.rs`, then `gear.rs` and `chunk.rs`.** The test is
   committed first and does not compile, which is the discipline
   `docs/sdlc.md` gives bug fixes, applied to a bound: a property written after
   the chunker passes tests the chunker. Then the chunker, iterated until the
   five properties hold across the seed set **on all four content mixtures** —
   or until the bound is found not to hold on an object `Env` drew, which is
   the spec's first reversal and comes back here as a finding rather than as a
   wider threshold. This step is where the slice is most likely to stop and
   argue, and the mixture is why: uniform bytes alone would have let a chunker
   that is bad at zero-filled content pass without anybody seeing it.
5. **The node kinds in `abi/src/store.rs`, `abi/src/boot.rs`, `generation/`,
   `user/generation.toml`, `cargo xtask generation`.** The second risky thing.
   `abi/src/store.rs` lands in two commits and this is the first: the fold
   needs the four node kinds, and the format does not need the fold. Then run
   the verb in two fresh containers from the same commit and diff the leaves.
   The two containers share a checkout path, so a leaf that differs here is not
   the path — `--remap-path-prefix` is `E2-P06`'s first deliverable and this
   rehearsal does not stand in for it. If a leaf differs, the input that moved
   is named by the output, and it is the finding `E2-P06`'s job exists to
   produce, found before the job exists. If a component file's hash differs
   between the two runs, the finding is `E0-R01`'s territory — a build that
   does not reproduce — and it is why step 1 had to decide what the leaf
   contains.
6. **The store records in `abi/src/store.rs`, `device.rs`, `store.rs`,
   `tests/million.rs`.** The format, last, for the reason it was last before.
   By now the chunker's constants and the leaf's width are settled, so the
   superblock — which carries those constants, so that a mount can refuse a
   chunker that changed under it — and the object record are written once.
   `Device::flush` lands with the trait rather than after it, because step 2
   already decided what a barrier is. The million-blob run is the exit and it
   runs in release; the ten-thousand default is what `cargo xtask test` sees.
7. **RFC 0058 and RFC 0059**, each immediately before the task that would be
   expensive without it — rule 3 — and after step 4, so that RFC 0058's
   comparison paragraph quotes `CHUNK_MIN_BYTES`, `CHUNK_TARGET_BYTES` and
   `CHUNK_MAX_BYTES` as they are and names its bad workload at the write size
   claim 0017 measures. RFC 0059 sits directly before `E2-B02`, which is not in
   this slice, and lands here anyway because the three invariants are cheaper
   to state before `f-zone` exists than to reverse-engineer from it — and
   because `SWEEP_LIVE_FRACTION` is where claim 0016's threshold is derived
   from, so the number is owed before the claim that spends it is registered.
8. **`bench/src/bin/rechunk.rs`, claim 0017, the ROUTES row, the snapshot.**
   The number that closes `E2-B01`'s second clause, registered with thresholds
   copied from the spec's derivations rather than invented here. If step 4
   held, this is a green run; if this is red and step 4 was green, the two
   disagree about what "bounded" means and the claim is the one that is right,
   because it is the one a stranger reproduces.
9. **The TODO lines**, at each landing, by the session that lands it.

Each step goes to a green `cargo xtask verify` before the next one starts.

## Proof

Every command runs in the development container, from the repository root, and
from a worktree the project name is `f-<name>` so the target volumes do not
collide:

```
docker compose -f docker/compose.yaml run --rm -T dev <command>
```

The floor, before anything is offered for review, and expected to stay green at
every step of *Order* rather than at the end:

```
cargo xtask verify
```

Per movement, the commands that observe the exits. `cargo xtask test` is two
things — `cargo test --workspace` under the host, then `cargo check -p f-hash
-p f-blob -p f-generation … --target aarch64-unknown-none` for every
PORTABILITY row with `bare: None` — and the second half is what says the three
new crates compile off x86-64. It does not run them there: the arm runner's
`tests (AArch64, weak memory)` job does, and nothing local substitutes for it.

```
cargo xtask lint                            lint-units over blob/ and generation/
                                            as well as abi/, where the record
                                            types now live; lint-determinism
                                            finding nothing under hash/, blob/,
                                            generation/; lint-unsafe reporting
                                            none of the three and no new block in
                                            abi/; lint-arch-tests with no gate in
                                            any of them; the generation round-trip
                                            fixpoint over every generation.toml;
                                            lint-reproduce and lint-snapshot over
                                            the sixteenth claim; lint-style with
                                            -D warnings
cargo xtask test                            the host suite on this architecture,
                                            then the AArch64 cross-compile of
                                            every crate that reaches the machine
cargo xtask blk                             step 2's observation, and the reason
                                            the opcodes may land before the
                                            behaviour: five new constants and a
                                            driver that still answers two, so the
                                            datapath boot and its control are
                                            unchanged. A red run here means
                                            something other than numbers landed
cargo test -p f-hash                        the two vectors, the boundary lengths,
                                            the stream-equals-one-shot split at
                                            every position
cargo test -p f-abi                         the record codecs' golden bytes, the
                                            compile-time assertion that each
                                            encoded length equals the sum of its
                                            field widths, and `from_bytes`
                                            refusing a bad magic, an unknown kind
                                            and an out-of-bound length before it
                                            reads a field. This is what
                                            blob/tests/format.rs was, one crate
                                            over, and it runs on both runners
cargo test -p f-blob --test chunker         E2-P02, the five properties across the
                                            seed set and the four content mixtures
cargo test -p f-blob --test million         the per-commit default: 10 000 blobs
cargo test -p f-blob --release --test million -- --blobs 1000000
                                            E2-B01's first exit. Release, because
                                            a million SHA-256 blocks under the
                                            debug profile is a minute nobody
                                            should pay per commit, and the count
                                            is an argument for the reason ring's
                                            hostile test made it one
cargo test -p f-generation                  the fold, twice; every leaf, once;
                                            non-canonical source refused with its
                                            diff
cargo xtask generation                      the root, its leaves, and the boot
                                            module named by the root hash; run
                                            twice in two fresh containers and
                                            diffed before E2-B04 is marked [>]
cargo xtask generation --decompile          a record tree back to canonical
                                            source. The fixpoint over it is the
                                            lint above; this is the verb that lint
                                            calls, and the tripwire is on the
                                            source rather than on the artefact
cargo xtask claim bytes-rechunked-per-byte  E2-B01's second exit, and the number
                                            behind "bounded"
cargo xtask claims                          writes the snapshot lint-snapshot
                                            checks
cargo xtask todo E2                         after the TODO lines: E2-B02, E2-B03
                                            and E2-B09 become available and E2-P06
                                            is what E2-B04 waits for
```

One exit no command here can close: `E2-B04`'s says two machines and two
dates, and that is `E2-P06`'s weekly job. Two containers on one laptop on one
afternoon is the rehearsal, not the exit, which is why the line goes to `[>]`.

## Risks

Build risks, as distinct from design risks — those are the spec's.

**`alloc` in a `no_std` library, and the first image that links it.** `f-blob`
declares `extern crate alloc` and uses `Vec` for an object's chunk list. As a
library that compiles for `aarch64-unknown-none` and for the host without
anyone providing a `#[global_allocator]`; the obligation falls on the first
*image* that links it, which is `user/objects` and not this slice, and the heap
it will use is `ring/`'s under spec decision 4. What this slice must not do is
write the `unsafe impl GlobalAlloc` anywhere above the frame to make a test
pass — the host tests use std's allocator and need nothing. If a test is found
needing one, that is the heap's task arriving early, not a reason to widen
`UNSAFE_ALLOW`.

**Three identities where the RFC says one.** A release address is SHA-256
(`pack.rs`); a blob is SHA-256 (this slice); a component file's spawn identity
is FNV-1a over 64 bits, in `abi/` and compared by the frame at spawn. RFC 0012
has to say which of two things is true — the short id becomes the low 64 bits
of the file's SHA-256, an `abi/` change under ordering rule 1 with a test that
the two agree, or it stays a spawn-local name the generation never carries —
and `abi/src/store.rs`'s `component` leaf waits on the answer. Building the
leaf as 32 bytes and hoping is how a second wire format gets written.

**The record types now live in `abi/`, which is one of the three crates where
`unsafe` is permitted.** They moved there for two good reasons — `lint-units`
already runs over `abi/`, and a library above the frame may not view device
bytes through a cast — but the move puts them in the one place where somebody
could add `#[repr(C)]` and a pointer cast and have it compile. Nothing in the
policy set catches that: `lint-unsafe` reports a block count and `abi/` is on
the allow-list. What catches it is the golden-bytes test and the compile-time
length assertion, which is why both are written in the same commit as the
types and why the assertion is per record rather than one blanket check. A
reviewer who sees a cast appear in `abi/src/store.rs` should read it as a
reversal of spec decision 7's reasoning and ask for the RFC.

**The chunker's bound may not hold on candidate-free content, and neither of
the two obvious fixes is one.** Step 4 asserts resynchronisation within
`RESYNC_BOUND_BYTES` of `X + L` *or at the end of the enclosing candidate-free
run plus one chunk*. The first fix that will be reached for is a wider
threshold, and claim 0017's number stays where the spec derived it — a
threshold moved to make a seed pass is the failure the registry exists to make
visible. The second is the reversal an earlier draft of this plan wrote down: a
chunker whose acceptance does not depend on the previous boundary. **That one
does not work and the plan says so here so nobody spends a day on it** — the
cause is maximum-size forcing, a forced cut is relative to the previous
boundary by definition, and acceptance has nothing to do with it. If the
mixture fails, the finding is the crate doc's honest paragraph and it goes back
to the spec.

**A mask whose highest set bit is not 63 is a sixteen-byte window wearing a
sixty-four-byte claim.** With `h = (h << 1) + gear[b]`, bit *k* depends on the
last *k+1* bytes, so a mask over the low sixteen bits gives a sixteen-byte
window — which is exactly what phase-locks boundaries on data with short
periods, and which an earlier draft of the spec asserted was sixty-four. The
number appears in three places (the crate doc, `E2-P02`'s specification and
claim 0017's workload description) and a derived mask can drift silently, so
`gear.rs`'s third test asserts the highest set bit is 63 and the crate doc
states the arithmetic rather than the conclusion.

**`f-blob` takes `f-env` as a real dependency and must never reach it at run
time.** The two tables are `const fn` derivations under RFC 0026's single
generator, which is the right call and also the first time a library above the
frame has named the determinism substrate for a compile-time reason.
`lint-determinism` will not catch a later `Env` reaching into the store,
because an `Env` is legitimate everywhere; what says the store draws nothing is
the crate doc, and it is written as a claim so that a diff contradicting it is
visible as a contradiction.

**RFC 0060 lands inside another task's diff and nothing marks it.** No
`TODO.md` line names it, this plan does not add one, and `lint-owed` reports
reversal conditions rather than owed RFCs — so between the day the opcodes land
and the day somebody reads `docs/rfc/README.md`, the only record that a
reversal was paid is the reserved-numbers row. That row is why the section
exists rather than an index, and it is the whole mitigation. The alternative is
editing `TODO.md`, which this run may not do.

**`lint-units` widened reads every `pub` field under `blob/` and
`generation/`, tests included.** `unit_findings` walks `rust_sources()` under
the prefix, so a `pub` field on a test fixture with no `Unit:` in its doc
comment is a red lint on a struct nothing ships. The record types are in
`abi/`, which was already in the set, so the widening now bites on the two
crates' own public items and their fixtures: fixtures keep their fields
private, the chunker's constants state their units, and `Memory` in `device.rs`
has nothing public.

**PORTABILITY and the member list land together or `classify` fails.** A
member with no row is a hard error in both `test_host` and `cross_check`, in
the same commit as the `members` line or the loop is red between them. The
same commit, and nothing between.

**The million-blob device is half a gigabyte of host memory.** A million
512-byte blocks in a `Vec<u8>`. It runs in release on the container, and it is
the exit rather than the gate for exactly that reason; the default count is
what `cargo test --workspace` pays. If the container's memory limit refuses
it, the model becomes a sparse `BTreeMap<u64, [u8; 512]>` of written blocks —
`BTreeMap`, because the lint checks this crate too — and the number does not
change.

**`harness = false` means the test parses its own command line.** The
precedent is `ring/tests/hostile.rs` and the shape is copied: a `--blobs N`
flag, a default, and a usage line, so that `cargo test --workspace` runs it
with no arguments and gets the default.

**The gear and mask tables are a `const` evaluation across a crate boundary.**
`f_env::split::label`, `Stream::from_seed` and `next_u64` are all `const fn`,
so the derivation is legal; what would not be legal in spirit is the table
being *computed at run time* under the name of a constant, which the test that
recomputes it and compares would not catch. The definitions are
`pub const GEAR: [u64; 256] = gear_table();` and its two mask twins and nothing
else, so the compiler either evaluates them or refuses. If a future
`f_env::split` change drops `const` from any of the three, this crate stops
compiling rather than quietly moving the work to boot.

**`docs/rfc/README.md` grows a section and not an index.** Four rows for four
numbers, one of which was a gap for two epochs and one of which has no task
line at all. A reviewer may ask for the other fifty-six, and the answer is the
same one the E1 plan gave about a fourth crate: that is a change to fifty-six
closed decisions' evidence, and it is not this epoch's.

**The target volume is shared.** Two sessions building the workspace at once
wait on a lock; a build that appears to hang is usually that. It is a cost of
the container and not a symptom.

## The rest of E2, one paragraph each

**`E2-D04`** lands `abi/src/transfer.rs` (NEW) and a `pub mod transfer` line
in `abi/src/lib.rs`: a fixed-width record declaring whether a component can be
updated in place, what its state records look like, and `restart_only` as the
honest default, with a `Unit:` on `schema`, `mode`, `record_bytes` and
`records_max`. Under ordering rule 1 it lands before `user/virtio-blk` declares
one, and it waits for RFC 0012 and for `E1-B02`, which is closed. The *task*
does not close there: its exit is `E2-P08` swapping the driver, which is after
`E1-B05`, so it is a decision with a proof in it.

**`E2-B02`** is `zone/` (NEW, package `f-zone`): sequential fill with
`ZONE_APPEND`, seal with `ZONE_FINISH`, copy-forward, reset with `ZONE_RESET`,
and the collector RFC 0059 specifies as a batch-class consumer, between
`f-blob`'s `Store` and the `Device`. It owns the half of the publish sequence
`blob/` does not: the root record's append and the second `FLUSH`, the two root
zones the superblock names, the `ROOT_CARRY` = 16 carry before a reset, and the
mount that scans both zones from the device's own `ZONE_REPORT` write pointers
and takes the highest verifying generation. It also answers the five opcodes
step 2 only numbered. It brings the trixie base change in `docker/Dockerfile`
as its own commit with the digest pinned in `ARG BASE` in the same commit,
because the image is in every reproduction and `E2-P06` must not name it first;
the fifteen existing claims are re-run in that image in that commit. It waits
for `E2-B01`, `E2-D03` and `E1-B02`, and it registers claim 0016
`write-amplification` with `claims/baselines/linux-6.x-tuned-zoned/` (f2fs in
zoned mode, apply.sh, verify.sh, README.md) beside it — which is also where
`lint-claims` learns the `[hardware] emulated` key and starts refusing a
zoned-device claim without it.

**`E2-B03`** is `index/` (NEW, package `f-index`): paths, metadata and
declared attributes to hashes, a log-structured store on the device with a
`BTreeMap` in memory, a local call and not a service. Its exit is measured as
**device blocks read per query against device blocks read per tree walk over
the same data**, against the modelled device, because ring crossings cannot
differ — a tree walk inside one component crosses zero boundaries too, and zero
against zero is a vacuous exit. The spec reads *without crossing a component
boundary* as *the index is not a service* and flags that as weaker than the
words; this plan builds the flagged reading and does not quietly widen it. It
is the first library that cannot be written without a heap, so
`ring/src/heap.rs` (NEW) — the bump-then-free-list allocator over a granted
region, safe constructor, the `unsafe impl GlobalAlloc` with its `// SAFETY:`
in the frame — lands with it, and `cargo xtask unsafe` moves once. Waits for
`E2-B01`.

**`E2-B05`** is `user/assembler/` (NEW, package `f-assembler`, with a
manifest, a `COMPONENTS` entry and a PORTABILITY row): instantiate a topology
from a root, route capabilities as the topology declares, bind drivers by
declared properties, leave a subtree unstarted when its driver fails. It
recomputes the fold over the boot module it is handed and refuses a mismatch,
which is the same crate doing the same arithmetic and not a second reader.
**Binding order is declared and not discovered**: the topology's component list
in canonical order, and within it devices sorted by declared identity
(bus/device/function as a `BTreeMap` key), never by the order a PCI scan
reported — without which `E2-B05`'s byte-identical topology is a property of
the scan. Two drivers matching one device becomes a `lint-manifests` refusal in
`xtask/src/main.rs`, at compile time rather than as a run-time choice. Written
as the parent of a supervisor at ring 3, so it waits for `E1-B05`'s restart
policy to leave the frame and for `E2-B04`; the spec names the second RFC that
becomes owed if `E1-B05` has not moved when this is otherwise ready.

**`E2-B06`** is the swap: `kernel/src/component.rs`, where a place lives
under RFC 0041, gains the routing word that
changes only at the quiescent point, one `Release` store and one `Acquire`
load, with a litmus test in `ring/tests/litmus.rs` that fails under the weaker
ordering — the fifth cross-core word RFC 0016 says needs an argument, and the
argument is the spec's. The opcodes go on the control ring in `abi/src/
control.rs`. It registers claim 0021 `generation-swap-pause`, `pending` on
`E0-D10`'s machine, which is the number RFC 0012's metric change will
eventually want. Waits for `E2-D04` and `E2-B05`, and its exit is `E2-P08`.

**`E2-B07`** publishes the generation root and the frame's hash in the frame's
state tree, `kernel/src/state.rs`, two nodes under one subtree — and closes for
the frame too, which is what the reconciliation added. `xtask`'s image builder
computes SHA-256 over the frame image with `f-hash` at build time; the frame
recomputes it over its own loaded text and rodata at boot and **refuses to
publish a root whose `frame` field disagrees**; and `MUTATIONS` in
`xtask/src/main.rs` gains a frame mutation whose boot must produce a different
published hash. Without those three the exit's *any modification* would hold
for everything under the root except the thing publishing it. It waits for
`E2-B05`; the residual — a frame modified to lie about its own recomputation —
is E5's TPM and is named in the attestation story in 0.3.

**`E2-B08`** is the read path in `user/objects/` (NEW, package `f-objects`,
manifest, `COMPONENTS` entry, PORTABILITY row, a DATAPATH row with its one
mover and a NOT_THE_FRAME row for its type): hash to zone and offset through
the index, the caller's registered buffer handed through both rings under RFC
0024's typestate, the blob header decoded from bytes rather than viewed. The
count is named rather than argued: the simulator's blk device model and the
driver each tally **bytes moved through any buffer that is not the caller's
registered one**, and the two readings are required to agree, which is claim
0012's discipline of counting the same event on both sides. It waits for
`E2-B02` and `E1-B10`, and it registers claim 0018 `copies-per-read`
(`pending` until `E1-B10`, `gating` the day it lands), claim 0019
`resident-bytes-per-unit-of-work` — a count and `gating`, not a timing, read as
resident pages of `user/objects` per N reads from the frame's state tree — and
claim 0020 `read-path-latency` (`pending` on `runner-class-A`).

**`E2-B09`** adds `blob/src/extent.rs` (NEW) — the extent kind,
`EXTENT_BYTES` = 1 MiB pieces, a write path that replaces one piece — the
extent half of `bench/src/bin/rechunk.rs`, and the extent rows in claim 0017 at
`max = 1048576`, which is exactly `EXTENT_BYTES` and is the 256× a 4 KiB write
costs. The record type itself is `abi/src/store.rs`'s, added there beside the
object. Waits for `E2-D02` and `E2-B01`. Its slip is the release question the
spec carries forward undecided.

**`E2-P01`** is the power-cut model: `env/src/sim.rs` becomes a directory —
which E1's plan promised and the tree shows did not happen; `sim.rs` is still
one file of 565 lines — with `env/src/sim/cut.rs` (NEW) wrapping
`f_blob::device::Memory`. The model is bigger than the earlier draft's: it
lands every operation covered by a completed `FLUSH` and **any subset of the
rest in any order**, with the cut falling inside an operation at a granularity
parameter (`block` or `byte`) and in one of two modes (`honest`, which respects
`FLUSH`, and `lying`, which ignores it). Every one of those is an `Env` draw
under RFC 0034, and the re-run key is `(seed, cut, granularity, mode)`.
`xtask/src/cut.rs` (NEW) is the sweep verb and it joins nightly.yml beside
`sweep`. Waits for `E1-P01` and `E2-B01`. Its artefact must carry four required
observations or it is a sweep of the model: a torn root record refused by
`check` at byte granularity, a `lying`-mode root refused by resolution and
counted in `roots_refused_unresolved`, a cut crossing a root-zone wrap, and
exactly two distinct mount outcomes.

**`E2-P03`** is `zone/tests/invariants.rs` (NEW): the collector run
concurrently with adversarial allocation and a hard-class reader, RFC 0059's
three invariants asserted — the allocation including **a multi-zone publish in
flight during a sweep**, which is the case the first invariant's open-publish
clause exists for, and the third asserted as
`collector_operations_ahead_of_hard_read` ≤ 1. Waits for `E2-B02` and
`E2-D03`.

**`E2-P04`** is Verus over the frame's invariants through RFC 0022's image
shape, in `kernel/verus/` (NEW, excluded from the workspace for RFC 0053's
reason). Three candidate invariants, named so the builder does not pick them by
accident: per-CPU ownership of every mutable `static` under `kernel/`; the four
cross-core words and no fifth; the ring's `Release`/`Acquire` pair. The proofs
run on the **nightly cadence beside `sweep`**, not weekly, and `MUTATIONS`
gains a frame mutation the proofs must reject — a proof that passes on a frame
it was not checking is the failure the mutation build exists to catch
everywhere else. Waits for `E0-B12`, `E0-B15`, `E1-B05`, `E1-B10`, `E1-B14`
and the five reversals `lint-owed` reports, checked by hand.

**`E2-P05`** is `xtask/src/compare.rs` (NEW): two whole-system trees as two
hashes, folded by `f-hash` over RFC 0013's read-only mapping, one subtree per
component under one root, descending to the divergent subtree — with the
divergence injected through `Env` at a named site under RFC 0026, so the test
knows the answer it is asking for. **It does not wait for the power-cut
model**, which this plan previously said and the graph does not: state
comparison has no use for a cut. Waits for `E0-B14`, `E1-P01` and `E1-B15`.

**`E2-P06`** is `generation/src/diff.rs` (NEW) — `divergence(&Tree, &Tree) ->
Option<Leaf>` naming the first leaf whose hash differs — and the weekly job in
`.github/workflows/weekly.yml` (NEW) that runs `cargo xtask generation` on two
runners on two dates and diffs leaves before roots. **Its first deliverable is
neither of those**: it is `--remap-path-prefix` in `.cargo/config.toml`,
because `E0-R01` measured reproduction at one path, nothing in the tree
configures a remap, and two runners on two dates do not share a checkout — so
without it the first leaf the job names is every component leaf, for a reason
that is not the evaluator's. `E0-R01`'s `address` job is where the remap is
checked. Waits for `E2-B04`, and it is what closes `E2-B04`'s line.

**`E2-P07`** is `cargo xtask rollback` in `xtask/src/main.rs`: break a
generation, reboot with `-append f.root=<64 hex>` naming the previous boot
module through `abi/src/boot.rs`'s grammar, and compare the restored root, its
module hash and every blob under it. It also lands `--install` in
`xtask/src/generation.rs`, which writes one `menuentry` per installed
generation into the GRUB fragment `docs/booting-on-hardware.md` documents —
that is what the exit's "boot menu" means: the loader's menu, configuration
this repo already writes, nothing imported and no boot-time reader of the
on-disk format. Waits for `E2-B05`.

**`E2-P08`** is `cargo xtask swap`: `user/virtio-blk` replaced under the chaos
workload with `kills = 0`, claim 0005's zeros plus `operations_redone = 0`,
the transferred state read back through the new instance. Waits for `E2-B06`.

**`E2-P09`** is `cargo xtask changepoint` in `xtask/src/main.rs` over
synthetic history, first half only; the second half is recorded as waiting on
`E0-P06` in the same change. **Binary segmentation with a CUSUM statistic** over
the stored per-run distributions, keyed by a `BTreeMap` in commit order and
adding no dependency to `xtask/Cargo.toml` — named because change-point
detection is the one piece of new algorithmic code in this epoch that would
otherwise arrive as a statistics crate. Its false-positive half is a count of
runs and not a p-value with a clock in it. Waits for `E0-P11` and `E0-P06`.

**`E2-P10`** runs claim 0016 `write-amplification` on the emulated zoned
device with `[hardware] emulated = true` copied into the artefact header, and
**both its rows gate**: `device_bytes_per_app_byte` at `max = 1.5` and
`ratio_vs_baseline` at `max = 1.0`. The baseline is not deferred — `cargo xtask
claim write-amplification --baseline` builds and boots a Linux guest with f2fs
in zoned mode on the identical QEMU device inside the same container, and both
sides are counted by QEMU's `query-blockstats` over QMP, so the two numbers are
taken at one boundary. The workload is **phased** — fill with the collector
held, collect to quiescence with the client idle, then count — so the number is
not a function of the interleaving, and the `[hardware]` notes say so. It runs
on **two workloads**: that fill-and-collect, and `E2-D02`'s random write on
both object kinds. Waits for `E2-B02` and `E1-D06`.

**`E2-R01`** is `RELEASING.md`'s 0.3 contents, the two demonstrations as
commands, the attestation sentence *and the residual the frame's self-hash
leaves open*, `docs/TESTING-STATUS.md`'s count of pending timings, and a
stranger running the rollback from the release image. Its blockers are its own
contents and nothing here shortens them.

**`E1-B15`**, which is E1's id and this epoch's work, because the intent's
`todo:` carries it and `E2-P05` cannot exist without it. Every component the
supervisor starts publishes a tree of its own under RFC 0013's rules, mounted
under one root; the two reversals are paid and the words that state them are
deleted — `user/store/src/report.rs`'s *a runtime that publishes a tree of its
own* and RFC 0038's *a state tree the component publishes under RFC 0013*; one
`E1-P02` sweep scenario asserts its system response by reading a component's
subtree rather than the serial log; and a component that publishes nothing is
refused at spawn. It needs `E1-B05` and waits with the assembler, and it is
named here rather than left out because a task in scope with no paragraph is a
task nobody owns.
