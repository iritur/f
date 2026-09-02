# The component manifest

What a component *is*, before it runs: its image, its speculation domain, the
capabilities routed to it and the ones it may ask for, the protocols it speaks
and on how many rings, what its supervisor does when it ends, and what admission
may refuse it. One file per component, called `manifest.toml`, in the
permissive tree. `user/virtio-blk/manifest.toml` is the worked example and
`cargo xtask lint-manifests` is the schema as code — `xtask/src/manifest.rs`,
run on every `cargo xtask lint`, with a fixture that breaks each rule below.

This document is the schema as prose. Where the two disagree the lint is wrong
and this document is what it is wrong against; fix the lint and add the fixture
that would have caught it.

## Why a manifest, and why it is closed

Three decisions this tree has already made need a place to be stated per
component, and none of them can be stated at run time.

RFC 0008 says a component is spawned from a manifest named by content hash,
holds exactly the capabilities the manifest declared and its supervisor
supplied, and is restarted under a policy the manifest declares. RFC 0005 says a
component's speculation-domain kind is declared in its manifest and never
inferred. RFC 0007 says admission is arithmetic and refuses, which needs a
declared demand to refuse. The manifest is where all three are written, and a
spawn is where all three are checked — so the file is the topology's unit, and
its hash is what measured boot extends into.

It is closed because it is a parser's input. A field the reader does not know
is refused, not skipped; a value outside a table is refused, not mapped to the
nearest one; a field that means nothing under the declared policy is refused,
not ignored. R04. The cost is that every schema change is a `schema` bump and a
visible diff; the benefit is that no two readers can hold different beliefs
about one file, which is how a format acquires two incompatible interpretations
and no error.

## The syntax, which is a subset of TOML

Every valid manifest is valid TOML, so any TOML reader accepts it. The lint
accepts less: comments, `[table]` and `[[array]]` headers, and `key = value`
where the value is a `"string"` with no escapes and no inner quote, an unsigned
integer (underscores between digits allowed), `true`/`false`, or a one-line list
of strings. Multi-line strings, inline tables, dotted keys, signed numbers and
floats are refused with a line number. The reason is in `xtask/src/manifest.rs`:
the tree parses its own formats and buys no dependency for one, and the
supervisor does not read TOML at all — it reads a fixed-layout record that
E1-B05 defines in `abi/` and that this file compiles to. Every bound below
exists so that record can exist.

The first line is `# SPDX-License-Identifier: Apache-2.0 OR MIT`. A manifest is
authored source that becomes part of a component's identity, and its licence is
part of what is hashed.

## Where a manifest lives

Beside the crate that builds the image it names: `user/<name>/manifest.toml`,
with `image = "user/<name>"`. `user/` is where the component above the frame
already lives (`user/init`) and where `ring-scene-boot` section 14 puts user
components; a path and the thing it points at then move together; and a second
top-level directory for the same kind of thing — `components/` was the
alternative — would make "where do components live" a question with two
answers. Discovery is by file name, not by directory, so the choice is not
load-bearing for the lint; it is load-bearing for the reader.

Never under `third_party/`. An imported tree is verbatim and may contain a file
called anything; and a manifest is *policy* — which domain, which capabilities,
which restart — reviewed here, which a re-import must not be able to change. An
imported driver's manifest lives in `user/` and its `image` points into
`third_party/`, which is exactly the shape RFC 0005 rule 4 checks.

## Top level

| field | type | required | what it is |
| --- | --- | --- | --- |
| `schema` | integer | yes | The schema this file is written to. Must be `1`. A later value is refused: a reader that guesses at fields it was not written for is two readers. |
| `name` | string | yes | The component's name in the topology: `[a-z0-9-]`, at most 32 bytes, no edge hyphen. Unique across the tree — `lint-manifests` refuses two manifests with one name, because `sibling:` references and the topology name a component by it. |
| `image` | string | yes | Where the image comes from. Either a tree-relative path to the crate that builds it — forward slashes, no `.`/`..`/empty segment, not under `target/` — or `sha256:` and sixty-four lower-case hex digits for bytes the tree does not build. |
| `domain` | string | yes | RFC 0005's kind: `shared`, `private` or `hostile`. No default, and none of the working names other documents used (`trusted`, `confined`) is accepted — the RFC's spelling is the only spelling. |

Two rules join `image` and `domain`, and they are the two lines of RFC 0005 a
lint can enforce:

- An `image` under `third_party/` may not declare `shared` (rule 4: the licence
  boundary is the speculation boundary).
- An `image` named by hash must declare `hostile`. A hash names bytes with no
  source in this tree, so nobody here vouches for them, and RFC 0005's table
  puts code nobody vouches for in `hostile`. What would reverse this is a
  signed-image mechanism under which a hash *carries* a vouching; that is an
  RFC, and this rule names it so it is not relaxed by accident.

An `image` path that does not exist yet is not refused. The manifest is written
before the driver on purpose — this schema precedes E1-B02 as a claim precedes
its number — and the moment existence matters is assembly, which is E1-B05's and
refuses there. The lint reports every such manifest on its ok line, so "not yet"
stays visible and does not become "never". A directory with no `Cargo.toml` in
it is the same state — the manifest lives in the directory its crate will be
built from, so the directory exists the moment the manifest does. A path that is
a *file* is refused; that is a typo, not a plan.

## `[[capability]]` — needs and asks

Zero to sixteen entries, in the order the supervisor's spawn entry supplies them
and the order the `granted` notices arrive on the control ring (RFC 0008).
Sixteen is half of `kernel::cap::TABLE_SLOTS`; the other half is what the
component mints and is granted while running — a channel per client that
connects, a buffer set per transfer — and a manifest that filled the table at
spawn would describe a component that cannot accept a client. E1-B13 makes the
table an object paid for from `Untyped`, after which the bound is the wire
record's rather than the table's.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `name` | string | yes | The slot's name, `[a-z0-9-]`, unique within the manifest. A ring's `to` refers to it. |
| `type` | string | yes | `abi::cap::CapType`, one snake_case word per variant: `untyped`, `frame`, `address_space`, `channel`, `endpoint`, `irq`, `buffer_set`. The variant is spelled, not the short label `CapType::label` prints; `space` and `bufset` are refused. The lint's table is checked against `abi/src/cap.rs` by a test, so a variant added there fails here until this document and the table say so. |
| `rights` | list | yes | The minimum rights the supplied handle must carry, from `abi::cap::rights`: `read`, `write`, `execute`, `derive`, `revoke`, `grant`. Each at most once; an unknown word is refused. An empty list is legal — `rights::NONE` names an object and authorises nothing. `execute` on an `endpoint` is refused: RFC 0008 says it is undefined there and a derivation asking for it is refused, and a manifest asking for it would be refused later at greater cost. |
| `from` | string | yes | Where the handle is routed from. `supervisor`: supplied in the spawn entry from the supervisor's own table. `sibling:<name>`: supplied by the supervisor from an endpoint it holds to the named component under the same supervisor — a *need*, checked for shape here and for existence by the topology, which is not in this file. `powerbox`: not supplied at spawn; an *ask*, resolved while running through the broker of RFC 0008. A component is not its own sibling. |
| `optional` | boolean | no | Absent means `false`. A need not supplied and not optional refuses the spawn; an optional one arrives as an empty slot. The default is the one that gives less. |
| `frames` | integer | iff `type = "frame"` | How many pages, each 4096 bytes, at least one. Refused on any other type: a count belongs to the thing it counts. |
| `bytes` | integer | iff `type = "untyped"` | How much, a positive multiple of 4096, because untyped memory is retyped a page at a time. Refused on any other type. |

Nothing here names a vector, a device address or a peer's identity. Which
interrupt a device raises and which physical pages its registers occupy are the
machine's to know and the topology's to bind; a manifest that named them would be
a manifest bound to one machine, and two spawns of one hash would no longer be
the same component.

## `[[ring]]` — data rings

Zero to eight entries. **The control ring is not one of them.** Every component
has exactly one, created with it, with the frame at the other end (RFC 0008); a
ring named `control` is refused, and so is a data ring that offers the
`control_events` feature, because that is a second control ring under another
name.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `name` | string | yes | `[a-z0-9-]`, unique within the manifest, not `control`. |
| `role` | string | yes | `server`: clients connect to this component's endpoint and each receives one ring of this shape. `client`: this component connects through an endpoint it holds. |
| `protocol` | string | yes | The typed protocol spoken on it, by name: `[a-z0-9.-]`, at most 32 bytes. The name is the opcode space; `abi` says opcode spaces are per service, and this is where a service says which one it speaks. |
| `version_min`, `version` | integer | yes | The range of that protocol this component speaks, both at least 1, floor not above ceiling. Negotiated under RFC 0011 — intersection, highest common, refused with `PEER` naming what was missing — and never demanded equal. |
| `entries` | integer | yes | Slots per ring: a power of two from 2 to 65 536, as `ChannelHeader::ring_size` requires. Unit: entries. |
| `payload` | string | yes | How the bytes of an operation reach the peer. `inline`: in the entry itself. `registered`: through a registered buffer set the submitter owns (`ring-scene-boot` section 04) — the device transfers into it directly and nothing is copied; this is the zero-copy path E1-B02's exit counts. `shared_virtual`: the device walks the submitter's page tables, and `features` must offer `shared_virtual_memory` — the payload path *is* the feature bit, and naming one without the other is an intention without a mechanism (R01). |
| `features` | list | no | Feature bits offered, from `abi::feature`, each at most once. Absent means none: the base protocol, which every conforming peer speaks. |
| `features_required` | list | no | The subset of `features` this component cannot proceed without. A bit required and not offered is refused here for the same reason `ChannelHeader::negotiate` refuses it at setup. |
| `clients` | integer | iff `role = "server"` | The most simultaneous clients: 1 to 64. One SPSC ring per client, always, bounded at creation — `ring-scene-boot` section 06 says why a shared producer slot is not acceptable across a trust boundary. Refused on a client ring, which has one peer. |
| `to` | string | iff `role = "client"` | The `name` of a `[[capability]]` in this manifest of type `endpoint` carrying `write`, because `write` on an endpoint is the right to connect (RFC 0008). Refused on a server ring, which names nobody: its clients hold *its* endpoint. |

Where the protocol version travels on the wire is not this document's to say.
`ChannelHeader` versions the ABI, not the vocabulary; the connect handshake of
E1-B05 is the natural carrier and E1-B02 is the first component that needs it.
What this document fixes is that the range is declared here, per ring, and
negotiated rather than matched.

## `[restart]` — what the supervisor does when the component ends

Required. RFC 0008: a restart is a new spawn and not a resurrection — new table,
new memory, new channels, a higher epoch — and the one thing that survives is the
endpoint its clients hold, so a client that lost its peer reconnects through the
handle it already has. The manifest declares the policy; the supervisor of
E1-B05 applies it; the frame provides only the mechanism.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `policy` | string | yes | `never`: the place is left empty however the component ended. `on_fault`: respawn after a fault — an exception at ring 3, or a corrupted control ring — and not after an exit or a stop. `always`: respawn after a fault or an exit, and not after a stop, which is the supervisor's own decision. |
| `backoff_first_ms` | integer | iff not `never` | The pause before the first respawn, at least 1. Unit: milliseconds. Zero is a restart loop with no pause in it. |
| `backoff_max_ms` | integer | iff not `never` | The pause doubles from `backoff_first_ms` on each respawn and is capped here; not below the first. Unit: milliseconds. |
| `max_restarts` | integer | iff not `never` | How many respawns the supervisor will perform over its own lifetime, at least 1. Unit: restarts. After the last, the place stays empty: connects to it pend until their deadlines pass, which is what its clients see. Zero is `never` under another name and is refused; say `never`. |

Under `never` the three quantities are refused rather than ignored, because a
reader who sees a backoff will believe there is one.

The count does not reset. A budget that resets is a budget a slow fault loop
defeats — one fault an hour, forever, restarted forever — and a component that
has exhausted a lifetime budget is one somebody should look at. E1-P06 kills
drivers at random under load and is the workload that says whether a lifetime
budget is too tight; if it is, the reversal is a `reset_after_ms` field in schema
2, with the reason written beside it.

Milliseconds here and nanoseconds on the wire, deliberately: `Sqe::deadline` is
nanoseconds because a deadline is compared against a clock, and a backoff is a
number a person chooses. The record E1-B05 compiles this to may carry whatever
unit it likes as long as the field name says which.

## `[reservation]` — what admission may refuse

Required. RFC 0007: admission is arithmetic and refuses, and R08 says the word
*deadline* is not used for a promise nothing can refuse. A spawn is the moment of
refusal, so the demand is declared here, and E1-B07's admission control reads it
and answers `ADMISSION` naming the component that could not be satisfied.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `class` | string | yes | `soft` or `hard`. |
| `memory_bytes` | integer | yes | The least the `Untyped` account supplied at spawn must hold: the component's whole footprint — address space root and page tables, text, stack, control ring, published state tree, capability table — is retyped from it (RFC 0008). A positive multiple of 4096 in the soft class and of 2 097 152 in the hard class, which holds pre-faulted huge pages that are never reclaimed, migrated or compacted. Unit: bytes. Not the same thing as an `untyped` need: a need is memory the component retypes for its own purposes; this is what it is made of. |
| `cores` | integer | iff `hard` | Whole physical cores, both SMT siblings held, at least 1. Unit: physical cores. |
| `cpu_period_ns` | integer | iff `hard` | The period the schedulability test admits against, at least 1. Unit: nanoseconds. |
| `cpu_budget_ns` | integer | iff `hard` | Execution time per period, from 1 to the period. Unit: nanoseconds. |

In the soft class the three CPU fields are refused: the soft class is scheduled
around the hard class, holds no core, and is refused nothing at admission but
memory. Declaring a budget for it would be a number nothing reads.

RFC 0007's other two components — memory bandwidth and a cache partition — are
not declared. They are the machine's to supply, by partition or by exclusion,
and admission records which; a manifest that stated a bandwidth demand would be
stating it in units no two machines share. When a workload arrives that needs to
declare one, that is schema 2 and the field name will carry its unit.

## What is refused, collected

For a reviewer, in one place:

- A first line that is not the SPDX header.
- Any syntax outside the subset: escapes, multi-line strings, inline tables,
  dotted or quoted keys, signed numbers, a list that does not close on its line.
- A key or table appearing twice.
- A `schema` other than 1.
- A missing `name`, `image`, `domain`, `[restart]` or `[reservation]`.
- A field this document does not list, anywhere.
- A `domain`, `type`, right, feature, `from`, `role`, `payload`, `policy` or
  `class` outside its table.
- A `third_party/` image in `shared`; a hash-named image outside `hostile`; an
  image path that leaves the tree or points at build output.
- `execute` on an endpoint; a right or feature named twice; `control_events` on
  a data ring; `features_required` beyond `features`; `shared_virtual` without
  its feature bit.
- A count on the wrong type; zero frames; bytes not a multiple of a page.
- A ring named `control`; entries not a power of two in range; a version range
  with a zero or an inverted floor; a client ring naming a missing, non-endpoint
  or non-connectable capability; a server ring naming one at all; `clients`
  outside 1..=64 or on a client ring.
- Restart quantities under `never`; a zero first backoff; a max below the first;
  zero restarts.
- CPU fields in the soft class; memory not in the class's grain; a budget above
  the period; zero cores.
- Two manifests with one `name`; an image path that names a file.

Two things are stated as *not* refused, because a reader will otherwise assume
they are: an image that does not exist yet, and a `sibling:` that no manifest
declares. The first is the order of work; the second is the topology's, which is
not in this file.

## What this schema does not decide

Named so the tasks that own them are not surprised.

- **The wire record.** E1-B05 defines the `#[repr(C)]` form in `abi/`, with
  `Unit:` on every field, and the hash a spawn names is over it and the image
  together. Every bound above is a promise that the record fits.
- **The topology.** Which supervisor spawns which manifest into which place,
  and which of its endpoints satisfy whose `sibling:` needs. E1-B05.
- **What rights mean on an `irq` and a `buffer_set`.** E1-B01 and E1-D03.
- **Which native components hold a secret** and therefore belong in `private`.
  Review, per RFC 0005, and the worked example argues its own answer in a
  comment because that is where the argument is read.

## What would reverse this

- **A second reader.** If E1-B05's supervisor or any tool ends up parsing
  `manifest.toml` directly rather than the record it compiles to, the subset
  argument collapses — there would then be two parsers of one file — and the
  right fix is to make the record the only thing that is read, not to grow the
  subset.
- **A field that cannot be bounded.** If a component legitimately needs more
  than sixteen routed capabilities, or a variable-length field the record cannot
  carry, the bound moves *with* E1-B13's growable table and a stated cost, not
  quietly.
- **The unused policy.** If by gate G1 every manifest in the tree says
  `on_fault` and none says `always` or `never`, the three-way enum is a
  preference wearing a decision's clothes and should collapse to two — the same
  test RFC 0005 applies to its middle rung.
