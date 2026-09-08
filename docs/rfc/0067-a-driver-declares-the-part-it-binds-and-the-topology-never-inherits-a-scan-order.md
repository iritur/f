# RFC 0067: A driver declares the part it binds, and the topology never inherits a scan order

- Status: accepted
- Date: 2026-09-07
- Affects: `docs/manifest.md` (`schema` 2 → 3, a new `[[device]]` array, hexadecimal in the value subset); `abi/src/manifest.rs` (`Binding`, `Record::binding`, `Record::devices`, `Record::read_unaligned`); `abi/src/boot.rs` (the boot module's layout moves here); `xtask/src/manifest.rs` and `xtask/src/main.rs` (`lint-manifests` gains the cross-manifest refusal); every `manifest.toml` in the tree; `user/assembler/` (new); `intent/0006-state/spec.md`'s assembler paragraph, which this implements

## This entry was 0065 for a day

`E2-B05` and `E1-B15` were built in parallel worktrees, neither able to see the
other, and both needed a manifest change. Both took `0065`, both took `schema`
2 → 3, and the collision surfaced at the merge. It is fixed here rather than
preserved: entries in this directory are append-only because a decision that was
taken should stay readable, and a *number assigned twice* is not a decision — it
is two files that cannot both be found by the name they claim, which is the one
defect append-only was never meant to protect. So `0065` stays with **a
component declares its state tree**, which had the more references in the tree
and therefore cost less to leave alone, and this entry moved to `0067`. `0066`
sat between them and did not move; it is `E2-B05`'s other entry and its one
reference to "RFC 0065" now reads 0067.

There is one schema 3 and it carries both arrays. Neither declaration
supersedes the other, and neither was weakened to fit: `Record` grew
`[[device]]` *and* `[[state]]`, `Record::BYTES` is 2696 rather than either
worktree's number, and each array's count byte took one of the record's three
reserved bytes.

## Decision

**A driver declares the *part* it binds as a `(vendor, device)` property in its
manifest, never an address; the assembler discovers *where* that part is and
keys devices by their identity in a `BTreeMap`; and two manifests declaring one
part is a `cargo xtask lint-manifests` refusal at compile time.** The manifest
record grows a bounded array of these declarations and the schema goes to 3, on
RFC 0030's stated terms — a schema bump and a rebuild of every component file —
because reading silence as *binds nothing* would be a driver acquiring a
property nobody chose, which is the one thing this format refuses.

The three orders that could decide a binding are ranked once, here: the
topology's canonical member order first, a device's declared identity — bus,
device, function — second, and **the order a bus scan reported, never**. A boot
that used the third would still boot; what it would stop being is a function of
one hash.

## Context

`E2-B05`'s exit is *boot is a pure function of one hash — the same root produces
a byte-identical topology*. Everything else in this epoch's Build movement waits
on it: `E2-B06`'s swap, `E2-B07`'s measured boot and `E2-P07`'s rollback all
assume that a root names one system.

A review of the spec's assembler paragraph found the hole. Nothing in the tree
said which driver drove which device: the frame's own boot path calls
`Survey::find(vendor, device)` with constants compiled into `kernel/src/blk.rs`,
which is fine for a frame that boots one driver and is not a topology at all.
An assembler that had to bind three drivers would have had exactly two things to
bind them with — the order a PCI scan produced functions in, and the order the
topology happened to list components in — and a PCI scan's order is a property
of which bridge answered first, how the firmware numbered the buses, and which
slot somebody put a card in. None of that is in the root. A topology decided by
it would be byte-identical on one machine and quietly different on the next,
and the claim would hold for as long as nobody moved a card.

`docs/manifest.md` had already refused half of the fix, in a paragraph written
at schema 1: *nothing here names a vector, a device address or a peer's
identity ... a manifest that named them would be a manifest bound to one
machine, and two spawns of one hash would no longer be the same component.*
That paragraph is right and is not weakened here. What it forbids is an
**address**; what was missing is a **property**, and the two are opposites — the
property is what makes one component file correct on two machines wired
differently.

The alternatives that were live:

- **Bind by topology order alone.** The first driver takes the first matching
  device. It removes the scan order and leaves the *other* ambiguity: two
  drivers that could both drive one card are resolved by whoever is asked
  first, which is a decision nobody wrote down.
- **Put the device list in `generation.toml` rather than in the manifest.** The
  root would then cover it, which is attractive. It fails on where the fact
  lives: what a driver can drive is a property of the driver, and a generation
  that named it would be a second place it is written — and two places can
  differ. The manifest is already hashed into the root through its leaf, so the
  root covers it either way.
- **A class-code match, or a wildcard.** Both were refused for now. A wildcard
  in particular is worse than it looks: it makes two property sets *overlap*
  without being *equal*, and the compile-time refusal below would silently stop
  being an equality test and start being a subsumption test nobody wrote.

## Consequences

**Easy.** The assembler binds by walking two canonical orders and cannot see a
discovery order, because neither structure it walks holds one — `f-assembler`'s
`Bus` is a `BTreeMap` keyed by address and has no scan order in it to return.
`user/assembler/tests/assemble.rs` shuffles a bus through eight seeded
permutations and gets one topology digest, which is the property stated as a
test rather than as a paragraph. Two drivers claiming one part is found by
`cargo xtask lint-manifests` in the repository, with both file names in the
finding, rather than by a machine that has both cards.

**Paid.** Every component file in the tree is rebuilt at schema 3, which is the
cost RFC 0030 priced when it made a manifest compiled rather than parsed and
which RFC 0063 paid once already. `abi::manifest::Record` is sixteen bytes
wider and three of its pinned offsets moved; `xtask`'s mirror of those offsets
moved with them, which is what the assertions beside them exist to force.

**Hexadecimal entered the value subset.** `docs/manifest.md`'s grammar now
accepts `0x` and lower-case hex digits. That is a widening of a closed subset
and is therefore an RFC-sized decision rather than a convenience: a PCI
identifier is a bit pattern, every datasheet and every constant in
`kernel::arch::x86_64::virtio` writes it in hex, and `vendor = 6900` is a number
a reviewer has to convert before they can check it against the thing it names.
Upper case is refused rather than accepted, which is `f_abi::boot::Selection`'s
rule for a root hash applied one level down: two spellings of one identifier are
two things a person compares by eye and gets wrong.

**A second way into a component record.** `Record::read_unaligned` exists
because a component file inside a boot module sits where the *format* put it,
not where a loader aligned it. `Refusal::Unaligned` stays exactly as it was for
the frame's own path — a loader that puts a module at an odd address has still
done something worth disbelieving — and both entry points run one private set of
judgements, so a field that becomes refusable becomes refusable in both by
construction.

**Foreclosed.** A driver that binds a *class* of device rather than a part
cannot be written against this record, and a manifest that wanted to say *any
mass-storage controller* has to wait. That is deliberate: this tree has three
drivers and all three are defined by a part number, and a field no matcher reads
is a field two builds can differ in while naming one component.

## What would reverse this

- **A driver defined by its class.** An AHCI or an xHCI driver is defined by its
  class code and by nobody's vendor id, and the day one arrives `Binding` is
  wider, `lint-manifests`'s equality test becomes a subsumption test written on
  purpose, and the schema goes to 4. The observation is a manifest somebody
  wants to write and cannot.
- **Two cards of one part.** A component binds the lowest matching address and a
  second identical card is left unbound, because two instances want two members
  and two members want two names — which the record tree refuses. The
  observation is a machine with two of anything this tree drives; the fix is a
  topology that can name an instance apart from a component, which is a fifth
  node kind and an RFC of its own.
- **A second reader of the boot module.** The spec's own reversal for the whole
  arrangement, inherited here: `f_abi::boot::Module` is the layout and
  `f-assembler` is its first reader. A loader, an installer or a rescue tool
  that decodes it separately is the thing to watch for, and the answer is to
  make it read this type rather than to grow a second one.
- **The overlap refusal firing on a manifest that is right.** If two components
  legitimately need to declare one part — a driver and a diagnostic, say — then
  the compile-time refusal is refusing something real, and the question to
  reopen is whether *claiming* a device and *declaring what you can drive* are
  the same statement. They are treated as the same statement here because in a
  tree with three drivers they are.
