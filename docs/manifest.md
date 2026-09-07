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
integer — decimal, or `0x` and lower-case hexadecimal, with underscores between
digits allowed in either — `true`/`false`, or a one-line list of strings.
Multi-line strings, inline tables, dotted keys, signed numbers and floats are
refused with a line number. Upper-case hexadecimal is refused rather than
accepted, for the reason a root hash is printed one way: two spellings of one
identifier are two things a reader compares by eye and gets wrong. Hexadecimal
is in the subset for `[[device]]`, whose two fields are bit patterns a bus
reports and which every datasheet and every constant in
`kernel::arch::x86_64::virtio` writes in hex — `vendor = 6900` would be a number
a reviewer has to convert before they can check it. The reason is in `xtask/src/manifest.rs`:
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
| `schema` | integer | yes | The schema this file is written to. Must be `3`. A later value is refused: a reader that guesses at fields it was not written for is two readers. |
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
| `from` | string | yes | Where the handle is routed from. `supervisor`: supplied in the spawn entry from the supervisor's own table. `sibling:<name>`: supplied by the supervisor from an endpoint it holds to the named component under the same supervisor — a *need*, checked for shape here and for existence by the topology, which is not in this file. Because it arrives *through* an endpoint, only an `endpoint` or a `channel` may say it: a page of memory, an interrupt or an address space does not travel on one and comes from the supervisor's own table, so `sibling:` on any other `type` is refused. `powerbox`: not supplied at spawn; an *ask*, resolved while running through the broker of RFC 0008. A component is not its own sibling. |
| `optional` | boolean | no | Absent means `false`. A need not supplied and not optional refuses the spawn; an optional one arrives as an empty slot. The default is the one that gives less. Refused on an ask: `powerbox` supplies nothing at spawn, so there is nothing there for `optional` to make optional, and a field that means nothing under the declared route is refused like every other one. |
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
| `max_restarts` | integer | iff not `never` | How many respawns the supervisor performs within `budget_window_ms` before it stops trying, at least 1. Unit: restarts. Zero is `never` under another name and is refused; say `never`. |
| `budget_window_ms` | integer | iff not `never` | The window that count is taken over, at least 1 and never below `backoff_max_ms`. Unit: milliseconds. |

Under `never` the four quantities are refused rather than ignored, because a
reader who sees a backoff will believe there is one.

The count and the window are one field between them, and RFC 0008 is why there
are two. That RFC fixes the budget as *how many restarts in what window*, and
says the window is read from `Env` — so a restart storm is a seeded scenario
under the simulator rather than a wall-clock accident, which is RFC 0004's
substrate keeping a call site it would otherwise have lost. An earlier draft of
this schema had `max_restarts` alone and counted over the supervisor's lifetime,
on the argument that a budget which resets is a budget a slow fault loop defeats
— one fault a day, forever, restarted forever. The window is what the tree says
instead, because a lifetime count is a number no scenario can reach and
therefore a rule nothing tests. The slow loop is real and the answer to it is a
lifetime cap *beside* the window, in schema 2, when a workload shows one: E1-P06
kills drivers at random under load and is that workload. Adding it amends RFC
0008's restart section, which is where the argument would have to be had.

A window below `backoff_max_ms` is refused. Once the backoff reaches its cap,
consecutive restarts are further apart than the window, so the count never
reaches its maximum and the budget can never be exhausted — a policy that says
`on_fault` with a budget and means `always`.

What exhaustion does is RFC 0008's, not this schema's: the place is **retired**
— its endpoint is revoked in every holder's table, pending connects complete
`PEER/GONE`, and the supervisor's own supervisor is told by the ordinary route.
A manifest declares the numbers; what happens when they run out is written once,
there.

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

`class` is also the ceiling RFC 0025 refuses against: an entry whose class is
more urgent than its submitter's ceiling earns `ADMISSION`/`NOT_HELD`, and this
field is where that ceiling is declared. `abi::class` has four ordinals and this
table has two spellings, which is not a gap — the table says what admission may
*refuse*, and only two of the four are refusable. `hard` is refused by RFC
0007's arithmetic; `soft` is refused its memory and nothing else; `batch` and
`idle` reserve nothing, so a manifest declaring one would state a demand no
admission test can fail. A component that wants to submit at batch or idle
already may: a ceiling is a maximum, and RFC 0025's first bound demotes and
never promotes. What this schema does not offer is a ceiling *below* `soft` — a
component that may not write `SOFT` at all — which is what RFC 0025 means by
"batch for a component that declares nothing". Schema 1 does not produce that
component, because `[reservation]` is required; the day one is wanted it is a
third value here and a `schema` bump, and not a default read into a missing
table, because a default is how a component acquires a ceiling nobody chose.

RFC 0007's other two components — memory bandwidth and a cache partition — are
not declared. They are the machine's to supply, by partition or by exclusion,
and admission records which; a manifest that stated a bandwidth demand would be
stating it in units no two machines share. When a workload arrives that needs to
declare one, that is schema 2 and the field name will carry its unit.

## `[transfer]` — what a component declares about being updated in place

Required. RFC 0063, and it is required for the reason `[restart]` is: a manifest
that says nothing has not chosen `restart_only`, it has left the choice to
whoever reads it next, and a place refilled from a newer manifest is exactly
where two readers disagreeing costs a client.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `mode` | string | yes | `restart_only` or `in_place`. |
| `schema` | integer | iff `in_place` | The state-record schema this build writes and reads, at least 1. The *component's* ordinal, not this file's: it is compared only against another build of the same component, and two components that both write `1` have said nothing to each other. Unit: none — a state-record schema ordinal. |
| `record_bytes` | integer | iff `in_place` | The fixed width of one state record, a positive multiple of 8 — the window is records laid end to end and read in place, so a width that is not puts every other record on an odd boundary. Unit: bytes. |
| `records_max` | integer | iff `in_place` | The most records this component will hand over. A bound and not a count: what crosses is however many the outgoing instance writes, up to this. Unit: count of records. |

Under `restart_only` the three quantities are refused rather than ignored, for
the same reason a backoff under `policy = "never"` is: a reader who sees a record
width will believe there is one.

`record_bytes * records_max` is the transfer window, and it may not exceed
`[reservation] memory_bytes`. RFC 0063 buys the window out of the **incoming**
instance's own `Untyped` account — so that a transfer is paid for by something
revocable and never by the frame, which is what RFC 0008 rests a component's
whole footprint on — and a window larger than the account is a swap admission
could never grant. Refused here rather than at the swap, where a client's
submissions are already being held.

What the frame does **not** know is what a record means. It knows how wide one
is and how many there can be, because those two size the window somebody has to
pay for; the only reader of a state record is another build of the same
component, and `schema` is what makes those two builds agree.

There is no third mode meaning *transferable only from a named quiescent point*,
and its absence is a decision. RFC 0018's cursors say the rings are empty; they
cannot say the occupant is empty, because a driver holds work it has accepted
and not answered and a request inside a device is behind both cursors. So every
in-place transfer waits for a point the occupant asserts, which is what
`in_place` already means — and the mode a third value would leave behind, a
cursors-only transfer, is one no component in this tree can correctly use.

## `[[device]]` — what part a driver binds

Zero to four entries. RFC 0067. Most components have none, and none is the
honest answer for anything that is not a driver: the array is absent, the
compiled record carries `devices = 0`, and nothing was left undecided by the
absence. That is the one place this table differs from `[transfer]`, which is
required because silence there would pick one of two answers; a device list has
a natural empty, and an empty list is the list rather than a default.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `vendor` | integer | yes | The PCI vendor identifier the bus reports, 1 to 0xFFFE. Zero is not a vendor and `0xFFFF` is how a bus says nothing answered — a manifest declaring it would match every empty slot on the machine. Unit: none — a device identifier, not a quantity. |
| `device` | integer | yes | The PCI device identifier the bus reports, 1 to 0xFFFF. Unit: none — a device identifier, not a quantity. |

**A property, and never an address.** The paragraph under `[[capability]]` still
holds without amendment: nothing here names a vector, a device address or a
peer's identity, because *which slot the card is in* is the machine's business
and a manifest that named one would be bound to one machine. What is declared
here is *what the part is*; where it is, is discovered. The assembler matches
the first against the second, which is the whole of `E2-B05`'s **bind drivers by
declared properties** — and it is what lets two spawns of one hash still be the
same component on two machines whose buses are wired differently.

**The entries are canonical and are checked, never sorted.** Sorted ascending on
`(vendor, device)`, and a file out of order is refused with the offending entry
named, for `f-generation`'s reason one level up: two entries swapped would be
two component files with two content hashes naming one driver, and a content
address that names two things names nothing. One part declared twice is refused
for the neighbouring reason — that is an author with two beliefs about one part,
not a list to be de-duplicated.

**Two manifests may not declare one part.** This is the only rule in this
document that no single file can satisfy on its own, and it is a
`lint-manifests` refusal across the whole set rather than a run-time choice: two
drivers matching one device at boot would be resolved by *something*, and the
only things available at that moment are the order a bus scan reported and the
order the topology happens to list — a topology decided by either is not a
function of the generation root, which is exactly what `E2-B05` claims it is. So
the ambiguity is refused where an author can see it.

There is no wildcard and no class-code match, deliberately. A driver defined by
its class rather than by a part number — AHCI, xHCI — is RFC 0067's stated
reversal: a wider record, a wider overlap test, and a schema bump. It is not a
sentinel added to these two fields, because a sentinel would make two property
sets overlap without being equal and the refusal above would quietly become a
subsumption test nobody wrote.

## `[[state]]` — what the component publishes about itself

Required, and required non-empty. RFC 0013 puts a hierarchical, typed state tree
in **every** component and RFC 0065 is what makes *every* mean every: this array
is the declaration, the frame writes the schema block out of it into the
component's own page before the component's first instruction, and a manifest
that declares nothing is refused `ADMISSION/NO_STATE_TREE` at the spawn.

| field | type | required | what it is |
| --- | --- | --- | --- |
| `id` | integer | yes | This node's permanent identifier inside this component's tree, at least 1, strictly ascending down the array. **Never reused**, for the reason `TODO.md` never reuses a task id: the id is the only thing that makes two readings of this component across time comparable at all. A retired node takes its id with it. Unit: none — an identifier, not a quantity. |
| `name` | string | yes | `[a-z0-9-]`, at most **16** bytes, no edge hyphen. Sixteen and not 32, because this goes on the wire as `abi::state::SchemaEntry::name`: RFC 0013 says a longer label is a description, and a description belongs in the document that owns the node. Unit: none. |
| `kind` | string | yes | `subtree`, `counter`, `gauge` or `mount`. A subtree's word is reserved and reads zero; a counter only goes up; a gauge goes both ways; a mount names another published region by physical address. Unit: none. |
| `unit` | string | yes | What the word is counted in: `none`, `nanoseconds`, `bytes`, `frames`, `entries`, `calls`, `cores`, `slots`, `events`, `address`, `trees` or `nodes`. `none` is a real answer and not a missing one — an identifier is not a quantity. Unit: none — it *is* the unit. |
| `parent` | string | iff not the first | The `name` of a node declared **above** this one. Refused on the first node and required on every other. Unit: none. |

At most 16 nodes. The bound is the frame's page rather than a taste: a
component's published region is one frame, and header, schema block and data
block share it. There is room for four times as many; what there is no room for
is a component that publishes its heap one node at a time.

**One root, and it is the first node.** The wire format does not require it and
a component's tree does: the frame reaches a component's tree at one mount
address, so a declaration with two parentless nodes would be two trees at that
address with the second reachable only by whoever went looking. Because ids
ascend and a parent must be named before its child, no node except the first
*can* be the root.

**The frame writes the description; the component writes the numbers.** The word
a node names lives at its index in the data block — offsets tile it in
declaration order, so a declared offset would be a second opinion the reader
could contradict. That split is what buys three things a constant inside the
component's image could not:

- A supervisor can refuse a component that publishes nothing *before it spends a
  frame on it*. A declaration inside the image is one the frame would have to
  run the component to find out about, and a component that has already run has
  already escaped the refusal.
- The schema block exists before the component's first instruction, so a
  component that is spawned and never scheduled still has a readable tree with
  zeros in it. That is the difference between *this component has done nothing*
  and *this component cannot be read*.
- The declaration is inside the content hash a spawn names, so a component whose
  account of itself changed is a different component — the same rule that
  already applies to its code.

What the frame does **not** know is what a node means. It knows the id, the
shape and the unit, because those are what a reader needs to not do the wrong
arithmetic to a number; what the word counts is the component's business, and
the only thing that keeps a node's name honest is the same thing that keeps a
claim's honest — somebody reading both.

## What is refused, collected

For a reviewer, in one place:

- A first line that is not the SPDX header.
- Any syntax outside the subset: escapes, multi-line strings, inline tables,
  dotted or quoted keys, signed numbers, a list that does not close on its line.
- A key or table appearing twice.
- A `schema` other than 3.
- A missing `name`, `image`, `domain`, `[restart]`, `[reservation]`,
  `[transfer]` or `[[state]]`.
- A field this document does not list, anywhere.
- A `domain`, `type`, right, feature, `from`, `role`, `payload`, `policy` or
  `class` outside its table.
- A `third_party/` image in `shared`; a hash-named image outside `hostile`; an
  image path that leaves the tree or points at build output.
- No `[[state]]` node at all; more than 16; an `id` of zero, repeated, or not
  ascending; a `name` outside `[a-z0-9-]` or longer than 16 bytes; a `kind` or
  `unit` outside its table; a `parent` on the first node, or one naming a node
  not declared above.
- `execute` on an endpoint; a right or feature named twice; `control_events` on
  a data ring; `features_required` beyond `features`; `shared_virtual` without
  its feature bit.
- A count on the wrong type; zero frames; bytes not a multiple of a page.
- `sibling:` on anything but an `endpoint` or a `channel`; `optional` on an ask.
- A ring named `control`; entries not a power of two in range; a version range
  with a zero or an inverted floor; a client ring naming a missing, non-endpoint
  or non-connectable capability; a server ring naming one at all; `clients`
  outside 1..=64 or on a client ring.
- Restart quantities under `never`; a zero first backoff; a max below the first;
  zero restarts; a zero window, or one below the longest backoff.
- CPU fields in the soft class; memory not in the class's grain; a budget above
  the period; zero cores.
- Transfer quantities under `restart_only`; a zero state-record schema; a
  `record_bytes` that is not a positive multiple of 8; zero `records_max`; a
  window larger than the account that buys it.
- Two manifests with one `name`; an image path that names a file.

Two things are stated as *not* refused, because a reader will otherwise assume
they are: an image that does not exist yet, and a `sibling:` that no manifest
declares. The first is the order of work; the second is the topology's, which is
not in this file.

## What this schema does not decide

Named so the tasks that own them are not surprised.

- ~~**The wire record.**~~ **Decided, at E1-B05.** `abi::manifest::Record` is
  the `#[repr(C)]` form, with `Unit:` on every field; `cargo xtask component`
  compiles a manifest into one and appends the image; and the hash a spawn names
  is over the record and the image together. Every bound above turned out to be
  a promise the record keeps: a name is thirty-two bytes because that is what a
  slot affords, sixteen capabilities and eight rings because the arrays are
  fixed. RFC 0030 is the argument, and the one thing it changes here is a unit:
  the backoff and the window are milliseconds in this document because a person
  chooses them, and **timer ticks** in the record because a supervisor compares
  them against a count the frame keeps and RFC 0004 does not let it read a
  clock. `xtask` converts, once, and `the_tick_rate_is_the_frames` reads the
  frame's own rate and fails when it moves.
- **The topology.** Which supervisor spawns which manifest into which place,
  and which of its endpoints satisfy whose `sibling:` needs. E1-B05.
- **What rights mean on an `irq` and a `buffer_set`.** E1-B01 and E1-D03.
- **Which native components hold a secret** and therefore belong in `private`.
  Review, per RFC 0005, and the worked example argues its own answer in a
  comment because that is where the argument is read.

## What would reverse this

- **A second reader.** If a supervisor or any tool ends up parsing
  `manifest.toml` directly rather than the record it compiles to, the subset
  argument collapses — there would then be two parsers of one file — and the
  right fix is to make the record the only thing that is read, not to grow the
  subset. RFC 0030 inherits this condition and states the observation that would
  trigger it: a `manifest.toml` opened by anything but `xtask/src/manifest.rs`.
- **A field that cannot be bounded.** If a component legitimately needs more
  than sixteen routed capabilities, or a variable-length field the record cannot
  carry, the bound moves *with* E1-B13's growable table and a stated cost, not
  quietly.
- **The unused transfer mode.** The mirror of the row below. If by the end of
  E2 every manifest says `restart_only` except the one `E2-P08` swaps, the
  two-way enum is a field with one user, and the question to ask is whether
  `in_place` is a property of components or a property of one driver. RFC 0051's
  argument — a second driver is what says the shape is a shape — is the one that
  answers it, and `user/virtio-gpu` is the second driver.
- **The unused policy.** If by gate G1 every manifest in the tree says
  `on_fault` and none says `always` or `never`, the three-way enum is a
  preference wearing a decision's clothes and should collapse to two — the same
  test RFC 0005 applies to its middle rung.
