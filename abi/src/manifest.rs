// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The component manifest, as the frame reads it: a fixed-width record, and the
//! image it names, in one blob with one hash.
//!
//! # Why there is no TOML here
//!
//! `docs/manifest.md` is the schema a person writes and `xtask/src/manifest.rs`
//! is the checker that refuses what does not fit it. Neither of them is what a
//! supervisor or a frame reads. `cargo xtask component <name>` runs that same
//! checker over `user/<name>/manifest.toml` and emits a **component file**: a
//! [`Record`] followed immediately by the image bytes, handed to the machine as
//! one boot module and named by one [`ContentId`] over the whole of it.
//!
//! The argument is RFC 0030's and the short version is three sentences. A
//! kernel with no allocator has no business running a text parser, and every
//! bound in the schema — thirty-two-byte names, sixteen capabilities, eight
//! rings — exists so that this record can have a size. A manifest that stops
//! fitting the schema is refused by `cargo xtask lint-manifests`, at lint time,
//! naming the field; what *this* module refuses is a record that arrived, which
//! is a different question with a different answer. And because the hash covers
//! the record and the image together, what a component *is* — its code and its
//! declared shape — is one name, which is the sentence RFC 0008 rests its
//! spawn on.
//!
//! # What reading one costs
//!
//! A length check, an alignment check, a magic, a schema, and then every field
//! judged against a closed set. There is no state machine and nothing is
//! allocated: [`Record::read`] hands back a *reference into the module's own
//! bytes*, so a component file is validated where the loader left it and copied
//! nowhere. That is what makes it affordable to re-validate on every spawn
//! rather than trusting whoever handed it over — which is the property that
//! survives a hostile supervisor, and the reason the frame does not accept a
//! pre-checked structure from one.
//!
//! # Milliseconds there, ticks here
//!
//! `docs/manifest.md` writes a backoff and a budget window in milliseconds,
//! because a person chooses those. This record carries them in **timer ticks**,
//! because a supervisor compares them against a count the frame keeps and RFC
//! 0004 does not let it read a clock. The conversion happens once, in `xtask`,
//! where it is a build step somebody can read. R03 is why every field below
//! says which of the two it is.

use crate::cap::{CapType, rights};
use crate::error;
use crate::transfer::{Declaration, mode};

/// The first eight bytes of a component file.
///
/// Chosen so that a module which is not one — `user/init`'s flat image, a
/// firmware blob, a file the loader placed for its own reasons — is skipped
/// rather than interpreted. The frame walks the boot modules by magic and not
/// by position, which is what makes adding a component a change to a list and
/// not to the kernel.
pub const MAGIC: u64 = 0x465f_4d41_4e00_0001;

/// The schema this build knows, and the only value [`Record::schema`] may
/// carry.
///
/// The same number `docs/manifest.md` and `xtask::manifest::SCHEMA` carry, and
/// a test in `xtask` requires the three to agree. A later schema is refused
/// rather than read approximately: a reader that guesses at fields it was not
/// written for is two readers with different beliefs about one component.
///
/// **Two since RFC 0063.** Schema 1 had no [`Record::transfer`], so a schema-1
/// record says nothing about whether its component can be updated in place —
/// and reading silence as `restart_only` would be a component acquiring a
/// property nobody chose, which is the one thing this format refuses. So a
/// schema-1 component file is refused rather than read approximately, and every
/// component file in the tree is rebuilt. RFC 0030 priced that cost when it
/// made a manifest compiled rather than parsed; this is the first change to
/// pay it.
///
/// **Three, and two decisions arrived at it together.** Schema 2 had no
/// [`Record::state`], so a schema-2 component publishes no state tree — and a
/// supervisor that read that silence as *this component has nothing to say
/// about itself* would be reinventing exactly the tolerance RFC 0013 was
/// written against. The refusal is `ADMISSION/NO_STATE_TREE` and it happens at
/// the spawn; the schema bump is what makes a record that could not carry the
/// declaration unreadable rather than quietly mute. RFC 0065.
///
/// Schema 2 also had no [`Record::binding`], so a driver said nothing about
/// which device it drives and an assembler had nothing to bind it by except the
/// order a bus scan reported — which is the one thing `E2-B05`'s exit cannot
/// survive, because a topology bound by a scan order is not a function of a
/// root. Silence is not readable as *binds nothing* for [`Record::transfer`]'s
/// reason: a driver that acquired *binds nothing* by omission would be a
/// component with a property nobody chose. RFC 0067.
///
/// The two were written in parallel and land in one schema, which is why one
/// bump pays for both: schema 3 carries both declarations, a schema-2 component
/// file is refused rather than read approximately, and every component file in
/// the tree is rebuilt once at the cost RFC 0030 priced.
pub const SCHEMA: u32 = 3;

/// The longest name, in bytes.
///
/// Names are `[a-z0-9-]`, so bytes are characters. Thirty-two is what
/// `docs/manifest.md` bounds them at, and this is the record those bounds were
/// chosen for.
pub const NAME_MAX: usize = 32;

/// The most `[[capability]]` entries a manifest may declare.
pub const CAPABILITIES_MAX: usize = 16;

/// The most `[[ring]]` entries a manifest may declare. The control ring is not
/// one of them: every component has exactly one, created with it, and RFC 0008
/// is why it is never declared.
pub const RINGS_MAX: usize = 8;

/// The most `[[device]]` entries a manifest may declare.
///
/// Four, and the number is a bound rather than a guess: a driver declares one
/// entry per device identity it will drive, and the widest case in this tree is
/// a virtio driver that accepts a modern id and the transitional id beside it —
/// two. Four leaves room for a third and a fourth part number without leaving
/// room for a manifest that binds a bus.
/// Unit: entries.
pub const DEVICES_MAX: usize = 4;
/// The most `[[state]]` nodes a manifest may declare, its root included.
///
/// Sixteen, and the bound is the frame's page rather than a taste: a
/// component's published region is one frame, [`crate::state::TreeHeader`] and
/// the schema block share it with the data block, and sixteen nodes is
/// `64 + 16 * 32 + 16 * 8` — six hundred and forty bytes of four thousand and
/// ninety-six. There is room for four times as many; what there is not room for
/// is a component that publishes its whole heap one node at a time, which is
/// the failure mode a tree with no bound has. RFC 0013 puts names in the schema
/// and numbers in the data block precisely so that a subtree is cheap and a
/// *description* is not.
///
/// *Reversal:* a component whose honest account of itself does not fit. At that
/// point the region stops being one frame and the declaration grows a page
/// count, which is a change to this constant and to `component::spawn`'s fixed
/// parts and to nothing else.
pub const STATE_NODES_MAX: usize = 16;

/// One page, as the record counts memory. Unit: bytes.
pub const FRAME_BYTES: u64 = 4096;

/// The grain a hard-class reservation's memory is stated in. Unit: bytes.
pub const HUGE_BYTES: u64 = 2 * 1024 * 1024;

/// RFC 0005's speculation-domain kinds, as wire values.
///
/// Zero is not a kind, so a zeroed record names none rather than naming the
/// first — the same rule [`CapType`] follows and for the same reason.
pub mod domain {
    /// Shares a core's speculative state with its siblings.
    pub const SHARED: u8 = 1;
    /// Holds something, and is not co-resident with anything that does not.
    pub const PRIVATE: u8 = 2;
    /// Assumed to be trying. Never `shared`, and never built from the
    /// permissive tree.
    pub const HOSTILE: u8 = 3;

    /// Is this a kind this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SHARED | PRIVATE | HOSTILE)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(value: u8) -> &'static str {
        match value {
            SHARED => "shared",
            PRIVATE => "private",
            HOSTILE => "hostile",
            _ => "unknown",
        }
    }
}

/// What a supervisor does when a component ends.
///
/// RFC 0008 fixes the semantics and `docs/manifest.md` fixes the spelling;
/// these are the wire values of that spelling.
pub mod restart {
    /// The place is left empty however the component ended.
    pub const NEVER: u8 = 1;
    /// Respawn after a fault — an exception at ring 3, or a control ring the
    /// component corrupted — and not after an exit or a stop.
    pub const ON_FAULT: u8 = 2;
    /// Respawn after a fault or an exit, and not after a stop, which is the
    /// supervisor's own decision.
    pub const ALWAYS: u8 = 3;

    /// Is this a policy this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, NEVER | ON_FAULT | ALWAYS)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(value: u8) -> &'static str {
        match value {
            NEVER => "never",
            ON_FAULT => "on_fault",
            ALWAYS => "always",
            _ => "unknown",
        }
    }
}

/// The reservation classes admission may refuse.
///
/// Two of [`crate::class`]'s four, and `docs/manifest.md` says why the other
/// two are absent: `batch` and `idle` reserve nothing, so a manifest declaring
/// one would state a demand no admission test can fail.
pub mod class {
    /// Scheduled around the hard class; refused its memory and nothing else.
    pub const SOFT: u8 = 1;
    /// Holds whole cores for its life, and is admitted by arithmetic that can
    /// say no. RFC 0007.
    pub const HARD: u8 = 2;

    /// Is this a class this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SOFT | HARD)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(value: u8) -> &'static str {
        match value {
            SOFT => "soft",
            HARD => "hard",
            _ => "unknown",
        }
    }

    /// The ceiling a component declaring this class is admitted for, as
    /// [`crate::deadline::Admitted`] reads it.
    ///
    /// Two vocabularies meet here and neither is wrong: this module's ordinals
    /// are *what a manifest may declare* and start at one so that a zeroed
    /// record declares no class, and [`crate::class`]'s are *how urgent* and
    /// start at zero because smaller is more urgent. A supervisor reading a
    /// record and handing the result to `deadline::inherit` needs the map, and
    /// it belongs here rather than at each supervisor: two spellings of it is
    /// one too many, and the second would be discovered on the day they
    /// disagree.
    ///
    /// `None` for a value this build does not know, which is R04 rather than a
    /// convenience — a record whose class byte is a value no schema produced
    /// must not be read as the nearest class. A component declaring nothing is
    /// admitted for [`crate::class::BATCH`] by RFC 0025, and that is the
    /// *caller's* substitution to make: it is a policy about missing
    /// declarations, and this function is a translation between two ordinal
    /// spaces.
    #[must_use]
    pub const fn admitted(value: u8) -> Option<u16> {
        match value {
            SOFT => Some(crate::class::SOFT),
            HARD => Some(crate::class::HARD),
            _ => None,
        }
    }
}

/// Where a declared capability is routed from.
pub mod route {
    /// Supplied in the spawn entry, from the supervisor's own table. A *need*.
    pub const SUPERVISOR: u8 = 1;
    /// Supplied by the supervisor from an endpoint it holds to a named
    /// component under the same supervisor. Also a need; [`Need::sibling`]
    /// carries the name.
    pub const SIBLING: u8 = 2;
    /// Not supplied at spawn. An *ask*, resolved while running through the
    /// broker of RFC 0008.
    pub const POWERBOX: u8 = 3;

    /// Is this a route this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SUPERVISOR | SIBLING | POWERBOX)
    }
}

/// Which end of a data ring this component is.
pub mod role {
    /// Clients connect to this component's endpoint and each receives one ring.
    pub const SERVER: u8 = 1;
    /// This component connects through an endpoint it holds.
    pub const CLIENT: u8 = 2;

    /// Is this a role this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, SERVER | CLIENT)
    }
}

/// How the bytes of an operation reach the peer.
pub mod payload {
    /// In the entry itself.
    pub const INLINE: u8 = 1;
    /// Through a registered buffer set the submitter owns. RFC 0024, RFC 0028.
    pub const REGISTERED: u8 = 2;
    /// The device walks the submitter's page tables, behind the negotiated
    /// feature bit of the same name.
    pub const SHARED_VIRTUAL: u8 = 3;

    /// Is this a path this build knows?
    #[must_use]
    pub const fn known(value: u8) -> bool {
        matches!(value, INLINE | REGISTERED | SHARED_VIRTUAL)
    }
}

/// What names a component: one hash over its record and its image together.
///
/// # Why this is a type rather than a `u64`
///
/// Because it is the field most likely to widen, and widening it should be a
/// compile error at every use rather than a search. FNV-1a over the whole blob
/// identifies a component against *accident* — a truncated module, a
/// mismatched pair, a place refilled from the wrong manifest — and it is
/// honest about being nothing more: a component file arrives from the boot
/// loader on the same trust path as the kernel image, so there is no adversary
/// between them to be collision-resistant against.
///
/// *Reversal, and it is the one this type exists for:* the day a component file
/// arrives from anywhere the boot loader did not put it — a network, a store, a
/// second stage — this becomes a cryptographic digest, sixty-four bits stops
/// being enough, and the change is to this struct and to `schema`. RFC 0030
/// records it as a condition rather than as an intention.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ContentId(u64);

impl ContentId {
    /// The identity of a component file, over every byte of it.
    #[must_use]
    pub const fn of(bytes: &[u8]) -> Self {
        // FNV-1a, the same construction `state::snapshot` uses and for the same
        // reason: it has to be identical in two readers and in two toolchains,
        // which rules out anything the standard library reserves the right to
        // change. Written as an index loop rather than an iterator because this
        // is `const` and because `user/init` links as one library with no
        // `core` beside it.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut at = 0;
        while at < bytes.len() {
            hash ^= bytes[at] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            at += 1;
        }
        Self(hash)
    }

    /// The identity as a number, for a log line or a state-tree node.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// From a number that was one.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }
}

/// One declared capability: a need the supervisor supplies at spawn, or an ask
/// the powerbox answers later.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Need {
    /// The slot's name, `[a-z0-9-]`, NUL-padded to the full width.
    /// Unit: bytes of ASCII, at most [`NAME_MAX`]; the padding is not part of
    /// the name and a non-zero byte after the first zero is refused.
    pub name: [u8; NAME_MAX],
    /// How much untyped memory, for a need of type [`CapType::Untyped`].
    /// Unit: bytes, a positive multiple of [`FRAME_BYTES`]. Zero on every other
    /// type, and a non-zero value there is refused rather than ignored.
    pub bytes: u64,
    /// How many pages, for a need of type [`CapType::Frame`].
    /// Unit: pages of [`FRAME_BYTES`] bytes, at least one. Zero on every other
    /// type, and refused there.
    pub frames: u32,
    /// What kind of object.
    /// Unit: none — a [`CapType`] wire value. Zero is not a type.
    pub kind: u8,
    /// The least the supplied handle must carry.
    /// Unit: none — a bitmask of [`rights`] constants. Empty is legal: a
    /// capability that names an object and authorises nothing.
    pub rights: u8,
    /// Where the handle comes from.
    /// Unit: none — a [`route`] constant. Zero is not a route.
    pub route: u8,
    /// Whether a need not supplied still permits the spawn.
    /// Unit: none — zero or one, and any other value is refused. Refused
    /// entirely on an ask, which supplies nothing at spawn for it to make
    /// optional.
    pub optional: u8,
    /// The sibling this is routed through, for [`route::SIBLING`].
    /// Unit: bytes of ASCII, as [`Need::name`]. All zero on every other route,
    /// and refused there.
    pub sibling: [u8; NAME_MAX],
}

/// One declared data ring. The control ring is never one of these.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    /// The ring's name within this manifest, NUL-padded.
    /// Unit: bytes of ASCII, as [`Need::name`]. Never `control`.
    pub name: [u8; NAME_MAX],
    /// The typed protocol spoken on it, by name, NUL-padded.
    /// Unit: bytes of ASCII from `[a-z0-9.-]`, at most [`NAME_MAX`].
    pub protocol: [u8; NAME_MAX],
    /// Feature bits offered.
    /// Unit: none — a bitmask of [`crate::feature`] constants. Never carries
    /// `CONTROL_EVENTS`: that is the control ring's, and a data ring offering
    /// it is a second control ring under another name.
    pub features: u64,
    /// The subset of [`Ring::features`] this component cannot proceed without.
    /// Unit: none — a bitmask of [`crate::feature`] constants, and a subset of
    /// the field above; a bit required and not offered is refused here for the
    /// same reason `ChannelHeader::negotiate` refuses it at setup.
    pub features_required: u64,
    /// The oldest protocol version this component speaks.
    /// Unit: none — a protocol version ordinal, at least 1. Zero is not a
    /// version.
    pub version_min: u32,
    /// The newest protocol version this component speaks.
    /// Unit: none — a protocol version ordinal, never below
    /// [`Ring::version_min`].
    pub version: u32,
    /// Slots per ring.
    /// Unit: entries — a power of two from 2 to 65 536, as
    /// `ChannelHeader::ring_size` requires.
    pub entries: u32,
    /// The most simultaneous clients, for a server.
    /// Unit: clients, from 1 to 64. Zero on a client ring, which has one peer,
    /// and a non-zero value there is refused.
    pub clients: u32,
    /// Which end of the ring this component is.
    /// Unit: none — a [`role`] constant. Zero is not a role.
    pub role: u8,
    /// How the bytes of an operation reach the peer.
    /// Unit: none — a [`payload`] constant. Zero is not a path.
    pub payload: u8,
    /// Which [`Record::capability`] this ring connects through, for a client.
    ///
    /// An index rather than a name: the checker has already resolved the `to`
    /// field against the capability list, and resolving a name twice is how two
    /// readers come to disagree about which slot was meant.
    /// Unit: none — an index into [`Record::capability`], below
    /// [`Record::capabilities`]. [`NO_CAPABILITY`] on a server ring, which
    /// names nobody.
    pub connects_through: u8,
    /// Reserved. Must be zero — a non-zero value is refused rather than
    /// ignored, per R04.
    /// Unit: none; this is not a quantity and is not expected to become one
    /// without a schema bump.
    pub _reserved: [u8; 5],
}

/// What [`Ring::connects_through`] says when a ring connects through nothing.
///
/// Not zero, because zero is a capability index. A sentinel that is also a legal
/// value is the bug this constant exists to not have.
pub const NO_CAPABILITY: u8 = u8::MAX;

/// One device identity a driver declares it will bind.
///
/// # Why a property and not an address
///
/// `docs/manifest.md` has said since schema 1 that nothing in a manifest names a
/// device address: *which* slot a card is in is the machine's business, and a
/// manifest that named one would be a manifest bound to one machine, so two
/// spawns of one hash would stop being the same component. That sentence still
/// holds and this record does not weaken it. What is declared here is *what the
/// part is* — the vendor and the part number a bus reports — and what is
/// discovered is *where it is*. The assembler matches the first against the
/// second, which is the whole of `E2-B05`'s *bind drivers by declared
/// properties*.
///
/// # Why the pair and not a class code
///
/// Because the pair is what this tree already matches on:
/// `kernel::arch::x86_64::pci::Survey::find` takes a vendor and a device and
/// nothing else, and a field here that no matcher reads would be a field two
/// builds could differ in while naming one component.
///
/// *What would reverse this:* a driver that binds a class rather than a part —
/// an AHCI or an xHCI driver, which are defined by their class code and not by
/// anybody's vendor id. That is a wider `Binding`, a wider overlap test in
/// `cargo xtask lint-manifests`, and a schema bump. It is deliberately not a
/// wildcard added to these two fields: a wildcard would make two property sets
/// overlap without being equal, and the compile-time refusal below would
/// silently become a subsumption test nobody wrote.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Binding {
    /// Who made the part.
    /// Unit: none — a PCI vendor identifier as the bus reports it. Zero is not
    /// a vendor and is refused; so is [`Binding::NO_VENDOR`], which is how a
    /// bus says nothing answered and would therefore match every empty slot on
    /// the machine.
    pub vendor: u16,
    /// Which part.
    /// Unit: none — a PCI device identifier as the bus reports it. Zero is not
    /// a device identifier and is refused. There is no wildcard, deliberately;
    /// the type's own comment says why.
    pub device: u16,
}

impl Binding {
    /// A binding with nothing in it, which is what every slot past
    /// [`Record::devices`] must be.
    pub const EMPTY: Self = Self { vendor: 0, device: 0 };

    /// The value a bus returns when nothing answered at an address.
    ///
    /// Refused as a declared vendor for `kernel::arch::x86_64::pci`'s own
    /// reason: a manifest declaring it would match every empty slot there is.
    /// Unit: none — a PCI vendor identifier.
    pub const NO_VENDOR: u16 = 0xFFFF;

    /// Does this declaration match a part a bus reported?
    ///
    /// Equality on both fields, which is the whole of the match: there is no
    /// wildcard in this record, and this is the function that would have to
    /// grow one.
    #[must_use]
    pub const fn matches(&self, vendor: u16, device: u16) -> bool {
        self.vendor == vendor && self.device == device
    }
}

/// One node of the state tree a component declares it will publish.
///
/// # Why the declaration is in the manifest and not in the component
///
/// RFC 0013 says the schema block is *published once per generation* and that
/// the data block has to be **generated from the same declaration the schema
/// is** — a build-time obligation it creates and names as the only defence
/// against the two drifting. This is that declaration, and putting it here
/// rather than in the component's own image buys three things a component-side
/// constant could not:
///
/// - A supervisor can **refuse a component that publishes nothing** before it
///   spends a frame on it, which is `ADMISSION/NO_STATE_TREE` and is the clause
///   that keeps RFC 0013's *every* honest. A declaration inside the image is
///   one the frame would have to run the component to find out about, and a
///   component that has already run has already escaped the refusal.
/// - The schema block exists **before the component's first instruction**, so a
///   component that is spawned and never scheduled still has a readable tree
///   with zeros in it. That is the difference between *this component has done
///   nothing* and *this component cannot be read*, and a boot that could not
///   tell them apart is the boot this task exists to end.
/// - The declaration is inside the content hash a spawn names, so a component
///   whose *account of itself* changed is a different component — the same rule
///   `ContentId` already applies to its code.
///
/// What it costs is that a node's name and hierarchy are chosen where the
/// manifest is written rather than where the counter is kept, and the two can
/// drift. That is a real cost and it is the one RFC 0013 already accepted for
/// the schema block; what makes it survivable is that the ids are permanent and
/// the *component* is what fills the words.
///
/// Exactly twenty-eight bytes: [`crate::state::SchemaEntry`] without its
/// `offset`, because an offset is derived — the words tile the data block in
/// declaration order — and a declared offset would be a second opinion about
/// where a node lives.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Node {
    /// This node's permanent identifier, unique within this component's tree
    /// and never reused across its lifetime.
    /// Unit: none — an identifier, not a quantity. Zero is not a node.
    pub id: u32,
    /// The id of the node this hangs under, or zero for this tree's root.
    /// Unit: none — a node identifier. Exactly one node in a declaration may
    /// carry zero, and it is the first: that is what makes a component's tree
    /// *one* tree rather than a forest wearing one name.
    pub parent: u32,
    /// One of [`crate::state::kind`]. Unit: none. Zero is not a kind.
    pub kind: u8,
    /// One of [`crate::state::unit`]. Unit: none — it *is* the unit.
    ///
    /// Not closed against this build's set, deliberately: RFC 0013's reader
    /// skips and counts a node it cannot name, so a manifest declaring a unit
    /// a future build defines is a manifest an older frame can still publish.
    /// What is closed is the *kind*, because the kind is what says whether the
    /// word is a count, a level or an address, and a publisher that got that
    /// wrong would be publishing a number under the wrong arithmetic.
    pub unit: u8,
    /// How many bytes of `name` are used.
    /// Unit: bytes, at most sixteen. Zero is a node with no name, which the
    /// wire format allows and this declaration refuses — a node nobody can
    /// name in a file a person wrote is a typing mistake.
    pub name_len: u8,
    /// Reserved. Must be zero; a non-zero byte is refused rather than ignored,
    /// per R04. Unit: none.
    pub _reserved: u8,
    /// The node's name, ASCII `[a-z0-9-]`, not terminated.
    /// Unit: none. Sixteen bytes, which is [`crate::state::SchemaEntry`]'s
    /// field and not [`NAME_MAX`]: this name goes on the wire as that one.
    pub name: [u8; 16],
}

const _: () = assert!(core::mem::size_of::<Node>() == 28);

impl Node {
    /// A node that is not one.
    pub const EMPTY: Self =
        Self { id: 0, parent: 0, kind: 0, unit: 0, name_len: 0, _reserved: 0, name: [0; 16] };

    /// The name, as far as it goes.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        let len = (self.name_len as usize).min(16);
        self.name.get(..len).unwrap_or(&[])
    }

    /// The schema entry this declaration becomes, for a node at `index` in the
    /// data block.
    ///
    /// The offset is `index * WORD` and comes from here rather than from the
    /// declaration, because `crate::state::validate` requires exactly that
    /// tiling and a declared offset would be a second opinion it could
    /// contradict. One arithmetic, in one place, so that the schema the frame
    /// writes and the schema a reader validates cannot disagree.
    #[must_use]
    pub const fn entry(&self, index: u32) -> crate::state::SchemaEntry {
        let mut out = crate::state::SchemaEntry::ZERO;
        out.id = self.id;
        out.parent = self.parent;
        out.offset = index * crate::state::WORD;
        out.kind = self.kind;
        out.unit = self.unit;
        out.name_len = self.name_len;
        let mut at = 0;
        while at < 16 {
            out.name[at] = self.name[at];
            at += 1;
        }
        out
    }
}

/// A whole manifest, as the frame reads it.
///
/// Field order is by alignment and not by the order `docs/manifest.md` lists
/// them, because a record with padding in it is a record with bytes nobody
/// checks — and an unchecked byte in a hashed structure is a place two
/// component files can differ while naming the same component.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Record {
    /// [`MAGIC`], and the first thing a reader looks at.
    /// Unit: none — a fixed byte pattern.
    pub magic: u64,
    /// The least the `Untyped` account supplied at spawn must hold: the
    /// component's whole footprint, address space and capability table
    /// included.
    /// Unit: bytes, a positive multiple of [`FRAME_BYTES`] in the soft class
    /// and of [`HUGE_BYTES`] in the hard class.
    pub memory_bytes: u64,
    /// The period the schedulability test admits against, for a hard-class
    /// reservation.
    /// Unit: nanoseconds, at least 1. Zero in the soft class, and refused
    /// there.
    pub cpu_period_ns: u64,
    /// Execution time per period, for a hard-class reservation.
    /// Unit: nanoseconds, from 1 to [`Record::cpu_period_ns`]. Zero in the soft
    /// class, and refused there.
    pub cpu_budget_ns: u64,
    /// The schema this record is written to.
    /// Unit: none — a schema ordinal. Must be [`SCHEMA`].
    pub schema: u32,
    /// How long this record is, so that a reader can tell a record it is too
    /// old for from one that was truncated.
    /// Unit: bytes. Must equal this build's `size_of::<Record>()`.
    pub record_bytes: u32,
    /// How many bytes of image follow the record in the same module.
    /// Unit: bytes, at least one. The module is exactly
    /// `record_bytes + image_bytes` long, checked rather than assumed.
    pub image_bytes: u32,
    /// The pause before the first respawn.
    /// Unit: timer ticks, at the frame's own tick rate. At least 1 under a
    /// policy that restarts, and zero under [`restart::NEVER`]. Milliseconds in
    /// `docs/manifest.md`; `xtask` converts, and the module comment says why.
    pub backoff_first_ticks: u32,
    /// The cap the pause doubles up to.
    /// Unit: timer ticks, never below [`Record::backoff_first_ticks`]. Zero
    /// under [`restart::NEVER`].
    pub backoff_max_ticks: u32,
    /// How many respawns the supervisor performs within the window below before
    /// it stops trying and retires the place.
    /// Unit: restarts, at least 1. Zero under [`restart::NEVER`].
    pub max_restarts: u32,
    /// The window that count is taken over.
    /// Unit: timer ticks, at least 1 and never below
    /// [`Record::backoff_max_ticks`]. Zero under [`restart::NEVER`].
    pub budget_window_ticks: u32,
    /// Whole physical cores, both SMT siblings held, for a hard-class
    /// reservation.
    /// Unit: physical cores, at least 1. Zero in the soft class, and refused
    /// there.
    pub cores: u32,
    /// The component's name in the topology, NUL-padded.
    /// Unit: bytes of ASCII from `[a-z0-9-]`, at most [`NAME_MAX`], no edge
    /// hyphen, never empty.
    pub name: [u8; NAME_MAX],
    /// RFC 0005's speculation-domain kind.
    /// Unit: none — a [`domain`] constant. Zero is not a kind.
    pub domain: u8,
    /// What the supervisor does when this component ends.
    /// Unit: none — a [`restart`] constant. Zero is not a policy.
    pub restart: u8,
    /// The reservation class admission may refuse.
    /// Unit: none — a [`class`] constant. Zero is not a class.
    pub class: u8,
    /// How many of [`Record::capability`] are real.
    /// Unit: entries, at most [`CAPABILITIES_MAX`]. Every entry past this must
    /// be all zero, so that two records declaring the same component cannot
    /// differ in bytes nobody reads.
    pub capabilities: u8,
    /// How many of [`Record::ring`] are real.
    /// Unit: entries, at most [`RINGS_MAX`]. The control ring is not counted:
    /// every component has exactly one and never declares it.
    pub rings: u8,
    /// How many of [`Record::binding`] are real.
    ///
    /// **Zero is a declaration and not a silence.** A component that binds no
    /// device declares no `[[device]]` entry, and `cargo xtask lint-manifests`
    /// requires the choice to have been made in the source rather than left to
    /// whoever reads the record next — which is the same argument RFC 0063 made
    /// for [`Record::transfer`], one field over.
    /// Unit: entries, at most [`DEVICES_MAX`]. Every entry past this must be
    /// all zero.
    pub devices: u8,
    /// How many of [`Record::state`] are real.
    ///
    /// **Zero refuses the spawn** — `ADMISSION/NO_STATE_TREE`, RFC 0065 — and
    /// that is the field's whole point: it is not a count with an empty case,
    /// it is the declaration RFC 0013 requires of every component, and the
    /// count is how it is made. It is checked at the spawn and not at
    /// [`Record::read`] on purpose: a record with no nodes is well formed, and
    /// what is wrong with it is a decision the *supervisor* takes about what it
    /// will host, which is where every other admission refusal is taken.
    ///
    /// Unit: entries, at most [`STATE_NODES_MAX`]. Every entry past this must
    /// be all zero, so that two records declaring the same component cannot
    /// differ in bytes nobody reads.
    ///
    /// It took a reserved byte, as [`Record::devices`] did one field over —
    /// which is what a reserved byte is for and is why taking one is a schema
    /// bump. Two of the three are spent and one is left.
    pub state_nodes: u8,
    /// Reserved. Must be zero — a non-zero value is refused rather than
    /// ignored, per R04.
    /// Unit: none; this is not a quantity and is not expected to become one
    /// without a schema bump.
    pub _reserved: [u8; 1],
    /// What this component declares about being updated in place, and what it
    /// declares when it cannot.
    ///
    /// Here rather than in a second record, because a swap decides whether it
    /// is possible by comparing two *manifests* and a manifest that carried
    /// this somewhere else would be two files naming one component. RFC 0063.
    /// Unit: none — a [`crate::transfer::Declaration`], every field of which
    /// states its own.
    pub transfer: Declaration,
    /// The device identities this component declares it will bind.
    ///
    /// A *set* written as an array, and the order in the array is the source's
    /// own: two entries swapped are two component files with different content
    /// hashes naming one driver, which is why `cargo xtask lint-manifests`
    /// refuses a manifest whose entries are not sorted on (vendor, device).
    /// Canonical here for the reason `f_generation::record` is canonical one
    /// level up — the same declaration has to produce the same bytes or the
    /// root is not a function of the source.
    /// Unit: entries; the first [`Record::devices`] are real and the rest are
    /// all zero.
    pub binding: [Binding; DEVICES_MAX],
    /// The declared capabilities, in the order the supervisor supplies them and
    /// the order the `granted` notices arrive.
    /// Unit: entries; the first [`Record::capabilities`] are real and the rest
    /// are all zero.
    pub capability: [Need; CAPABILITIES_MAX],
    /// The declared data rings.
    /// Unit: entries; the first [`Record::rings`] are real and the rest are all
    /// zero.
    pub ring: [Ring; RINGS_MAX],
    /// The state tree this component publishes, in data-block order.
    ///
    /// Last in the record, because a field inserted before an array moves every
    /// slot in it and a component file the writer and the reader disagree about
    /// is exactly what the offset assertions below exist to prevent. Schema 3
    /// paid that cost once already: [`Record::binding`] landed *before* the
    /// arrays and moved [`Record::capability`] and [`Record::ring`] sixteen
    /// bytes, which is why every component file is rebuilt. Appending here is
    /// what kept it to one such move rather than two.
    /// Unit: entries; the first [`Record::state_nodes`] are real and the rest
    /// are all zero.
    pub state: [Node; STATE_NODES_MAX],
}

// The layout is the ABI. Pinned here so that a field reordered, widened or
// inserted is a build failure with a number in it rather than a component file
// two builds of this tree disagree about. A change to any of these is a
// `schema` bump and a rebuild of every component file — RFC 0030 states that
// cost rather than hiding it.
const _: () = assert!(core::mem::size_of::<Need>() == 80);
const _: () = assert!(core::mem::size_of::<Ring>() == 104);
const _: () = assert!(core::mem::size_of::<Binding>() == 4);
const _: () = assert!(core::mem::size_of::<Node>() == 28);
// 2696 = 104 + 16 + 4 * 4 + 16 * 80 + 8 * 104 + 16 * 28. The number is pinned
// because `xtask::manifest`'s mirror reads it out of this line — it cannot
// depend on this crate — but it is not *asserted* here on its own: the sum
// below derives it from the parts, so a number typed wrong is a build failure
// and not a component file two builds disagree about.
const _: () = assert!(core::mem::size_of::<Record>() == 2696);
// No padding anywhere: the sum of the parts is the whole. A padded record has
// bytes the reader never judges, and an unjudged byte inside a hashed structure
// is a place two files can differ while claiming to name one component.
const _: () = assert!(
    core::mem::size_of::<Record>()
        == 104
            + core::mem::size_of::<Declaration>()
            + DEVICES_MAX * core::mem::size_of::<Binding>()
            + CAPABILITIES_MAX * core::mem::size_of::<Need>()
            + RINGS_MAX * core::mem::size_of::<Ring>()
            + STATE_NODES_MAX * core::mem::size_of::<Node>()
);
// And the five offsets `xtask::manifest::record` mirrors, pinned here rather
// than derived there. The writer stamps fields at literal offsets because
// `xtask` deliberately does not depend on `f-abi`; a size assertion alone would
// let a field inserted *before* the arrays keep the total and move every slot,
// which is a component file the writer and the reader disagree about in a way
// no test that only sums sizes can see.
const _: () = assert!(core::mem::offset_of!(Record, transfer) == 104);
const _: () = assert!(core::mem::offset_of!(Record, binding) == 120);
const _: () = assert!(core::mem::offset_of!(Record, capability) == 136);
const _: () = assert!(core::mem::offset_of!(Record, ring) == 1416);
const _: () = assert!(core::mem::offset_of!(Record, state) == 2248);

/// Why a component file was refused.
///
/// One variant per thing a reader can disbelieve, because a refusal that says
/// only *malformed* is a refusal somebody debugs by bisecting the file. Every
/// one of them packs into RFC 0010's [`error::ARGUMENT`] domain: the record is
/// an argument to a spawn, and a spawn refusing it is not a failure of
/// authority.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// Shorter than a record, or shorter than the record says it is.
    Truncated,
    /// The record does not begin on an eight-byte boundary, so reading it in
    /// place would be an unaligned load. Refused rather than copied: a loader
    /// that puts a module at an odd address has done something this kernel
    /// should notice.
    Unaligned,
    /// The first eight bytes are not [`MAGIC`]. Not a component file.
    NotAManifest,
    /// A schema this build does not know.
    Schema,
    /// The record's own length is not this build's.
    RecordSize,
    /// A reserved field is not zero. R04.
    Reserved,
    /// A count is past its bound, or an entry past the count is not zero.
    Count,
    /// A name is empty, too long, or carries a byte outside its alphabet.
    Name,
    /// A closed field carries a value outside its set: a domain, a type, a
    /// route, a role, a payload, a policy or a class.
    Value,
    /// A rights bitmap carries a bit this build does not define, or asks for
    /// `EXECUTE` on an endpoint, which RFC 0008 leaves undefined there.
    Rights,
    /// A quantity is out of range, or two quantities disagree: a backoff cap
    /// below its floor, a budget above its period, a window below the longest
    /// backoff, a ring size that is not a power of two.
    Quantity,
    /// A field means nothing under the declared policy, route, role or class,
    /// and is not zero. Refused rather than ignored, because a reader who sees
    /// a backoff under `never` will believe there is one.
    NotUnderThisPolicy,
    /// A list that has to be canonical is not: two `[[device]]` entries name
    /// one part, or they are not sorted on (vendor, device). Refused rather
    /// than sorted, because a reader that reordered would let two different
    /// component files carry one meaning and the content address would stop
    /// naming what was written.
    Order,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Truncated => "the component file is shorter than the record it declares",
            Self::Unaligned => "the component file does not begin on an eight-byte boundary",
            Self::NotAManifest => "the module does not begin with the manifest magic",
            Self::Schema => "the record is written to a schema this build does not know",
            Self::RecordSize => "the record declares a length this build does not have",
            Self::Reserved => "a reserved field is not zero",
            Self::Count => "a count is past its bound, or an entry past a count is not zero",
            Self::Name => "a name is empty, too long, or outside its alphabet",
            Self::Value => "a closed field carries a value outside its set",
            Self::Rights => "a rights bitmap is undefined here",
            Self::Quantity => "a quantity is out of range, or two of them disagree",
            Self::NotUnderThisPolicy => "a field means nothing under what was declared",
            Self::Order => "a canonical list repeats an entry or is out of order",
        }
    }

    /// The refusal as a packed error, for a completion.
    ///
    /// Every one of them is [`error::ARGUMENT`]; which code depends on what
    /// kind of disbelief it is, and the three are the three RFC 0010 already
    /// distinguishes.
    #[must_use]
    pub const fn packed(self) -> i32 {
        let code = match self {
            Self::Truncated | Self::Unaligned | Self::NotAManifest | Self::RecordSize => {
                error::argument::MALFORMED_HEADER
            }
            Self::Reserved => error::argument::RESERVED_NOT_ZERO,
            Self::Rights => error::argument::RIGHTS_CONFLICT,
            _ => error::argument::UNKNOWN_FLAG,
        };
        error::pack(error::ARGUMENT, code)
    }
}

impl Record {
    /// A record with nothing in it, for a builder to fill.
    ///
    /// Every field zero except the three a reader looks at first, so that a
    /// builder which forgets a field produces something [`Record::read`]
    /// refuses rather than something it accepts with a default. There are no
    /// defaults in this format and that is the point: `docs/manifest.md` is
    /// closed because a default is how a component acquires a property nobody
    /// chose.
    pub const EMPTY: Self = Self {
        magic: MAGIC,
        memory_bytes: 0,
        cpu_period_ns: 0,
        cpu_budget_ns: 0,
        schema: SCHEMA,
        record_bytes: core::mem::size_of::<Self>() as u32,
        image_bytes: 0,
        backoff_first_ticks: 0,
        backoff_max_ticks: 0,
        max_restarts: 0,
        budget_window_ticks: 0,
        cores: 0,
        name: [0; NAME_MAX],
        domain: 0,
        restart: 0,
        class: 0,
        capabilities: 0,
        rings: 0,
        devices: 0,
        state_nodes: 0,
        _reserved: [0; 1],
        transfer: Declaration::EMPTY,
        binding: [Binding::EMPTY; DEVICES_MAX],
        capability: [Need::EMPTY; CAPABILITIES_MAX],
        ring: [Ring::EMPTY; RINGS_MAX],
        state: [Node::EMPTY; STATE_NODES_MAX],
    };

    /// Read a component file where the loader left it.
    ///
    /// The reference points into `module`, so nothing is copied and the record
    /// can be validated as many times as it is spawned from. Every field is
    /// judged before the reference is handed back; a caller that got one may
    /// read any field without checking it again, which is the only reading of
    /// "validated" worth having.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming which disbelief. Fail closed, R04: a field this
    /// build does not know is refused and never skipped.
    pub fn read(module: &[u8]) -> Result<&Self, Refusal> {
        let size = core::mem::size_of::<Self>();
        if module.len() < size {
            return Err(Refusal::Truncated);
        }
        if !module.as_ptr().cast::<Self>().is_aligned() {
            return Err(Refusal::Unaligned);
        }
        // SAFETY: `module` is at least `size_of::<Record>()` bytes long,
        // checked above, and correctly aligned for a `Record`, checked above.
        // `Record` is `#[repr(C)]` and every one of its fields is an integer or
        // an array of integers, so every bit pattern is a valid value — there
        // is no niche here for arbitrary bytes to violate. The reference
        // borrows `module`, so it cannot outlive the bytes it names.
        let record = unsafe { &*module.as_ptr().cast::<Self>() };
        record.judge(module.len())?;
        Ok(record)
    }

    /// Read a component file out of bytes that are not aligned for a record.
    ///
    /// # Why this exists beside [`Record::read`], and does not replace it
    ///
    /// [`Refusal::Unaligned`] is a real refusal and stays one: a *loader* that
    /// puts a module at an odd address has done something this kernel should
    /// disbelieve rather than work around, and the frame's path keeps that
    /// refusal untouched.
    ///
    /// What changed is that a component file is no longer always something a
    /// loader placed. `E2-B05`'s assembler is handed **one** boot module —
    /// [`crate::boot::Module`] — with the component files packed inside it at
    /// offsets the *format* chose, and no format that packs variable-length
    /// files end to end can promise every one of them an eight-byte boundary
    /// without padding the reader would then have to judge. So the caller that
    /// holds bytes it did not place gets a way in that copies, and the record it
    /// gets back is owned rather than borrowed.
    ///
    /// **Every judgement is the same one**, not a second set: both entry points
    /// call one private function, so a field that becomes refusable becomes
    /// refusable in both by construction. Two validators over one layout is
    /// exactly the defect this crate exists to not have.
    ///
    /// The cost is one copy of the record — not of the image, which stays where
    /// it is — per component file per instantiation.
    ///
    /// # Errors
    ///
    /// Every [`Refusal`] [`Record::read`] produces except [`Refusal::Unaligned`],
    /// which cannot arise here.
    pub fn read_unaligned(module: &[u8]) -> Result<Self, Refusal> {
        let size = core::mem::size_of::<Self>();
        if module.len() < size {
            return Err(Refusal::Truncated);
        }
        // SAFETY: `module` is at least `size_of::<Record>()` bytes long, checked
        // above. `read_unaligned` requires the pointer to be valid for a read of
        // that many bytes and imposes no alignment requirement, which is the
        // whole reason it is the call here. `Record` is `#[repr(C)]` and every
        // one of its fields is an integer or an array of integers, so every bit
        // pattern is a valid value and there is no niche for arbitrary bytes to
        // violate; the result is an owned copy that borrows nothing.
        let record = unsafe { module.as_ptr().cast::<Self>().read_unaligned() };
        record.judge(module.len())?;
        Ok(record)
    }

    /// Every judgement a component file has to pass, over a record that is
    /// already in memory.
    ///
    /// `module_bytes` is the whole file's length, because two of the checks are
    /// about the file and not about the record: the image is not empty, and the
    /// file is exactly the record and the image with nothing after it.
    fn judge(&self, module_bytes: usize) -> Result<(), Refusal> {
        let size = core::mem::size_of::<Self>();
        let record = self;

        if record.magic != MAGIC {
            return Err(Refusal::NotAManifest);
        }
        if record.schema != SCHEMA {
            return Err(Refusal::Schema);
        }
        if record.record_bytes as usize != size {
            return Err(Refusal::RecordSize);
        }
        if record._reserved != [0; 1] {
            return Err(Refusal::Reserved);
        }
        if record.image_bytes == 0 {
            return Err(Refusal::Quantity);
        }
        // The module is exactly the record and the image, and nothing after it.
        // Trailing bytes are refused rather than ignored because the content
        // hash covers the whole module: bytes nobody reads are bytes two files
        // can differ in while naming one component.
        if module_bytes != size + record.image_bytes as usize {
            return Err(Refusal::Truncated);
        }

        if !is_name(&record.name) {
            return Err(Refusal::Name);
        }
        if !domain::known(record.domain)
            || !restart::known(record.restart)
            || !class::known(record.class)
        {
            return Err(Refusal::Value);
        }
        if record.capabilities as usize > CAPABILITIES_MAX || record.rings as usize > RINGS_MAX {
            return Err(Refusal::Count);
        }
        if record.devices as usize > DEVICES_MAX
            || record.state_nodes as usize > STATE_NODES_MAX
        {
            return Err(Refusal::Count);
        }

        record.check_restart()?;
        record.check_reservation()?;
        record.check_transfer()?;
        record.check_bindings()?;
        record.check_capabilities()?;
        record.check_rings()?;
        record.check_state()?;
        Ok(())
    }

    /// The image bytes of a module whose record this is.
    ///
    /// # Errors
    ///
    /// [`Refusal::Truncated`] if the module is not the length the record says.
    /// A caller that has been through [`Record::read`] cannot see it; the check
    /// is here anyway, because this function is also reachable from a caller
    /// that built the record itself.
    pub fn image<'a>(&self, module: &'a [u8]) -> Result<&'a [u8], Refusal> {
        let at = core::mem::size_of::<Self>();
        module.get(at..at + self.image_bytes as usize).ok_or(Refusal::Truncated)
    }

    /// The component's name, without the padding.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        let end = self.name.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
        self.name.get(..end).unwrap_or(&[])
    }

    /// The needs and asks that are real.
    #[must_use]
    pub fn needs(&self) -> &[Need] {
        self.capability.get(..self.capabilities as usize).unwrap_or(&[])
    }

    /// The data rings that are real.
    #[must_use]
    pub fn rings(&self) -> &[Ring] {
        self.ring.get(..self.rings as usize).unwrap_or(&[])
    }

    /// The device identities this component declares it will bind, in canonical
    /// order.
    ///
    /// Empty is the common answer and is a statement: this component binds no
    /// device. Nothing here distinguishes *declared none* from *said nothing*,
    /// because [`Record::read`] does not admit the second — a manifest that made
    /// no choice is refused by `cargo xtask lint-manifests` before a record
    /// exists.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        self.binding.get(..self.devices as usize).unwrap_or(&[])
    }

    /// The state-tree nodes this component declares, in data-block order.
    ///
    /// Empty is a legal *reading* and an illegal *component*: the record is
    /// well formed and the spawn is refused `ADMISSION/NO_STATE_TREE`. The
    /// split is deliberate — see [`Record::state_nodes`] — so that a tool which
    /// reads component files can say *this one declares no tree* rather than
    /// failing to read it at all.
    #[must_use]
    pub fn state_nodes(&self) -> &[Node] {
        self.state.get(..self.state_nodes as usize).unwrap_or(&[])
    }

    /// Does this policy restart after a death of this cause?
    ///
    /// The whole of RFC 0008's policy table, in one function, so that a
    /// supervisor above the frame and the frame's own demonstration cannot come
    /// to two different answers. `faulted` is true for an exception at ring 3
    /// or a control ring the component corrupted; `exited` for the one door
    /// call; and a stop is neither, which is why a stop never restarts — it is
    /// the supervisor's own decision and restarting after it would be the
    /// supervisor arguing with itself.
    #[must_use]
    pub const fn restarts_after(&self, faulted: bool, exited: bool) -> bool {
        match self.restart {
            restart::ON_FAULT => faulted,
            restart::ALWAYS => faulted || exited,
            _ => false,
        }
    }

    /// The pause before the `nth` respawn, counting from zero.
    ///
    /// Doubles from [`Record::backoff_first_ticks`] and is capped at
    /// [`Record::backoff_max_ticks`]. Saturating, and the saturation is not
    /// decoration: a shift past the width of the type is undefined in C and a
    /// panic in a debug build here, and this is the one arithmetic in the
    /// restart path a manifest's own numbers reach.
    /// Unit: timer ticks.
    #[must_use]
    pub const fn backoff_ticks(&self, nth: u32) -> u32 {
        let mut pause = self.backoff_first_ticks;
        let mut doubled = 0;
        while doubled < nth {
            pause = pause.saturating_mul(2);
            if pause >= self.backoff_max_ticks {
                return self.backoff_max_ticks;
            }
            doubled += 1;
        }
        pause
    }

    fn check_restart(&self) -> Result<(), Refusal> {
        let quantities = [
            self.backoff_first_ticks,
            self.backoff_max_ticks,
            self.max_restarts,
            self.budget_window_ticks,
        ];
        if self.restart == restart::NEVER {
            // Refused rather than ignored, and `docs/manifest.md` says why in
            // one sentence: a reader who sees a backoff will believe there is
            // one.
            return if quantities.iter().all(|q| *q == 0) {
                Ok(())
            } else {
                Err(Refusal::NotUnderThisPolicy)
            };
        }
        if quantities.contains(&0) {
            return Err(Refusal::Quantity);
        }
        if self.backoff_max_ticks < self.backoff_first_ticks
            || self.budget_window_ticks < self.backoff_max_ticks
        {
            // A window below the cap is a budget that can never be exhausted,
            // which is `on_fault` with a budget that means `always`.
            return Err(Refusal::Quantity);
        }
        Ok(())
    }

    fn check_reservation(&self) -> Result<(), Refusal> {
        let grain = if self.class == class::HARD { HUGE_BYTES } else { FRAME_BYTES };
        if self.memory_bytes == 0 || !self.memory_bytes.is_multiple_of(grain) {
            return Err(Refusal::Quantity);
        }
        if self.class == class::SOFT {
            return if self.cores == 0 && self.cpu_period_ns == 0 && self.cpu_budget_ns == 0 {
                Ok(())
            } else {
                Err(Refusal::NotUnderThisPolicy)
            };
        }
        if self.cores == 0 || self.cpu_period_ns == 0 || self.cpu_budget_ns == 0 {
            return Err(Refusal::Quantity);
        }
        if self.cpu_budget_ns > self.cpu_period_ns {
            return Err(Refusal::Quantity);
        }
        Ok(())
    }

    /// RFC 0063's declaration, judged the way every other closed field is.
    ///
    /// Three rules and one piece of arithmetic. The mode is closed and zero is
    /// not a value, so a zeroed record declares nothing rather than declaring
    /// `restart_only` by accident. Under `restart_only` the other three fields
    /// are refused when non-zero rather than ignored, because a reader who sees
    /// a record width under a component that hands nothing over will believe
    /// there is one — the same refusal [`Refusal::NotUnderThisPolicy`] makes of
    /// a backoff under `never`.
    ///
    /// The arithmetic is the one rule that ties this table to another: the
    /// window is bought out of the incoming component's own `Untyped` account
    /// (RFC 0063), so a declaration whose window is larger than
    /// [`Record::memory_bytes`] has declared a swap that can never be admitted.
    /// Refused here rather than discovered at the swap, where the client's
    /// submissions are already held.
    fn check_transfer(&self) -> Result<(), Refusal> {
        let declared = &self.transfer;
        if declared._reserved != [0; 3] {
            return Err(Refusal::Reserved);
        }
        if !mode::known(declared.mode) {
            return Err(Refusal::Value);
        }
        if !declared.in_place() {
            return if declared.schema == 0
                && declared.record_bytes == 0
                && declared.records_max == 0
            {
                Ok(())
            } else {
                Err(Refusal::NotUnderThisPolicy)
            };
        }
        if declared.schema == 0 || declared.records_max == 0 || declared.record_bytes == 0 {
            return Err(Refusal::Quantity);
        }
        if !declared.record_bytes.is_multiple_of(crate::transfer::RECORD_ALIGN) {
            return Err(Refusal::Quantity);
        }
        if declared.window_bytes() > self.memory_bytes {
            return Err(Refusal::Quantity);
        }
        Ok(())
    }

    /// Every declared binding is a part, no two of them are the same part, and
    /// they are in the order the encoder is required to write them.
    ///
    /// The order is checked and not applied, which is `f_generation::record`'s
    /// argument one level up and is the same argument here: a reader that sorted
    /// on the way in would let two different component files fold to one leaf,
    /// and the whole claim of a content address is that it does not.
    fn check_bindings(&self) -> Result<(), Refusal> {
        let mut previous: Option<Binding> = None;
        for (index, binding) in self.binding.iter().enumerate() {
            if index >= self.devices as usize {
                // Past the count, all zero — the same rule the capability and
                // ring arrays are held to, and for the same reason: a byte
                // nobody judges is a byte two files can differ in while naming
                // one component.
                if *binding != Binding::EMPTY {
                    return Err(Refusal::Count);
                }
                continue;
            }
            if binding.vendor == 0 || binding.device == 0 || binding.vendor == Binding::NO_VENDOR {
                return Err(Refusal::Value);
            }
            if let Some(before) = previous {
                match before.cmp(binding) {
                    core::cmp::Ordering::Less => {}
                    // Equal is a driver that declared one part twice, which is
                    // an author with two beliefs about one thing rather than a
                    // list to be de-duplicated.
                    core::cmp::Ordering::Equal | core::cmp::Ordering::Greater => {
                        return Err(Refusal::Order);
                    }
                }
            }
            previous = Some(*binding);
        }
        Ok(())
    }

    fn check_capabilities(&self) -> Result<(), Refusal> {
        for (index, need) in self.capability.iter().enumerate() {
            if index >= self.capabilities as usize {
                // Everything past the count is zero, so that two records
                // declaring one component cannot differ in bytes nobody reads —
                // which would give one component two content hashes.
                if !need.is_zero() {
                    return Err(Refusal::Count);
                }
                continue;
            }
            need.check()?;
        }
        Ok(())
    }

    fn check_rings(&self) -> Result<(), Refusal> {
        for (index, ring) in self.ring.iter().enumerate() {
            if index >= self.rings as usize {
                if !ring.is_zero() {
                    return Err(Refusal::Count);
                }
                continue;
            }
            ring.check(self)?;
        }
        Ok(())
    }

    /// The declared state tree: one root, ascending ids, and no node hanging
    /// from something that is not there.
    ///
    /// These are exactly the properties `crate::state::validate` will require
    /// of the schema block the frame writes out of this declaration, checked
    /// here so that the refusal names the *manifest* rather than surfacing as a
    /// frame that wrote a tree it cannot read. The two checks are deliberately
    /// duplicated and deliberately not shared: this one judges a declaration a
    /// person wrote, that one judges bytes in a mapping a peer may have
    /// scribbled, and folding them together would make the second trust the
    /// first.
    ///
    /// **One root**, which the wire format does not require and a component's
    /// tree does. RFC 0013's mount reaches a component's tree at one address,
    /// and a declaration with two parentless nodes would be two trees at that
    /// address with the second reachable only by whoever went looking. The
    /// first node is the root because the ids ascend and a parent must be named
    /// before its child, so no other node *can* be one.
    ///
    /// A zero count is not refused here. See [`Record::state_nodes`].
    fn check_state(&self) -> Result<(), Refusal> {
        for (index, node) in self.state.iter().enumerate() {
            if index >= self.state_nodes as usize {
                if node.id != 0
                    || node.parent != 0
                    || node.kind != 0
                    || node.unit != 0
                    || node.name_len != 0
                    || node._reserved != 0
                    || node.name != [0; 16]
                {
                    return Err(Refusal::Count);
                }
                continue;
            }
            if node._reserved != 0 {
                return Err(Refusal::Reserved);
            }
            if node.kind == 0 {
                return Err(Refusal::Value);
            }
            if node.name_len == 0 || node.name_len as usize > 16 || !is_node_name(node) {
                return Err(Refusal::Name);
            }
            // Ids ascend and start above zero, which is what makes two readings
            // of one component across time comparable and what makes the parent
            // check below sound in one pass.
            let previous =
                index.checked_sub(1).and_then(|at| self.state.get(at)).map_or(0, |n| n.id);
            if node.id == 0 || node.id <= previous {
                return Err(Refusal::Quantity);
            }
            if index == 0 {
                // The root, and the only node that may be parentless.
                if node.parent != 0 {
                    return Err(Refusal::Quantity);
                }
                continue;
            }
            if node.parent == 0 {
                return Err(Refusal::Quantity);
            }
            let named =
                self.state.get(..index).unwrap_or(&[]).iter().any(|prior| prior.id == node.parent);
            if !named {
                return Err(Refusal::Quantity);
            }
        }
        Ok(())
    }
}

/// Is a declared node's name `[a-z0-9-]`, with no edge hyphen and no byte past
/// its length?
///
/// The alphabet is the one every other name in this file uses, and the bytes
/// past `name_len` have to be zero for the reason the arrays past their counts
/// do: the record is hashed whole, and a byte nobody judges is a byte two
/// files can differ in while naming one component.
fn is_node_name(node: &Node) -> bool {
    let len = node.name_len as usize;
    if len == 0 || len > 16 {
        return false;
    }
    if node.name.iter().skip(len).any(|byte| *byte != 0) {
        return false;
    }
    let Some(name) = node.name.get(..len) else { return false };
    if name.first() == Some(&b'-') || name.last() == Some(&b'-') {
        return false;
    }
    name.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

impl Need {
    /// A slot that declares nothing.
    pub const EMPTY: Self = Self {
        name: [0; NAME_MAX],
        bytes: 0,
        frames: 0,
        kind: 0,
        rights: 0,
        route: 0,
        optional: 0,
        sibling: [0; NAME_MAX],
    };

    /// The slot's name, without the padding.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        let end = self.name.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
        self.name.get(..end).unwrap_or(&[])
    }

    /// What kind of object this names, or `None` for a value this build does
    /// not define.
    #[must_use]
    pub const fn cap_type(&self) -> Option<CapType> {
        CapType::from_wire(self.kind)
    }

    /// Is every byte of this slot zero?
    fn is_zero(&self) -> bool {
        self.name == [0; NAME_MAX]
            && self.sibling == [0; NAME_MAX]
            && self.bytes == 0
            && self.frames == 0
            && self.kind == 0
            && self.rights == 0
            && self.route == 0
            && self.optional == 0
    }

    fn check(&self) -> Result<(), Refusal> {
        if !is_name(&self.name) {
            return Err(Refusal::Name);
        }
        let Some(kind) = self.cap_type() else { return Err(Refusal::Value) };
        if !route::known(self.route) {
            return Err(Refusal::Value);
        }
        if self.optional > 1 {
            return Err(Refusal::Value);
        }
        if rights::unknown(self.rights) {
            return Err(Refusal::Rights);
        }
        // RFC 0008 leaves `EXECUTE` undefined on an endpoint and refuses a
        // derivation asking for it. A manifest asking for it would be refused
        // later at greater cost, so it is refused here.
        if kind == CapType::Endpoint && self.rights & rights::EXECUTE != 0 {
            return Err(Refusal::Rights);
        }
        // A count belongs to the thing it counts.
        match kind {
            CapType::Frame if self.frames == 0 => return Err(Refusal::Quantity),
            CapType::Untyped if self.bytes == 0 || !self.bytes.is_multiple_of(FRAME_BYTES) => {
                return Err(Refusal::Quantity);
            }
            CapType::Frame => {
                if self.bytes != 0 {
                    return Err(Refusal::NotUnderThisPolicy);
                }
            }
            CapType::Untyped => {
                if self.frames != 0 {
                    return Err(Refusal::NotUnderThisPolicy);
                }
            }
            _ => {
                if self.frames != 0 || self.bytes != 0 {
                    return Err(Refusal::NotUnderThisPolicy);
                }
            }
        }
        // A handle routed *through* an endpoint has to be something that
        // travels on one. A page of memory, an interrupt or an address space
        // does not.
        if self.route == route::SIBLING {
            if !matches!(kind, CapType::Endpoint | CapType::Channel) {
                return Err(Refusal::NotUnderThisPolicy);
            }
            if !is_name(&self.sibling) {
                return Err(Refusal::Name);
            }
        } else if self.sibling != [0; NAME_MAX] {
            return Err(Refusal::NotUnderThisPolicy);
        }
        // An ask supplies nothing at spawn, so there is nothing there for
        // `optional` to make optional.
        if self.route == route::POWERBOX && self.optional != 0 {
            return Err(Refusal::NotUnderThisPolicy);
        }
        Ok(())
    }
}

impl Ring {
    /// A ring slot that declares nothing.
    pub const EMPTY: Self = Self {
        name: [0; NAME_MAX],
        protocol: [0; NAME_MAX],
        features: 0,
        features_required: 0,
        version_min: 0,
        version: 0,
        entries: 0,
        clients: 0,
        role: 0,
        payload: 0,
        connects_through: 0,
        _reserved: [0; 5],
    };

    /// The ring's name, without the padding.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        let end = self.name.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
        self.name.get(..end).unwrap_or(&[])
    }

    fn is_zero(&self) -> bool {
        self.name == [0; NAME_MAX]
            && self.protocol == [0; NAME_MAX]
            && self.features == 0
            && self.features_required == 0
            && self.version_min == 0
            && self.version == 0
            && self.entries == 0
            && self.clients == 0
            && self.role == 0
            && self.payload == 0
            && self.connects_through == 0
            && self._reserved == [0; 5]
    }

    fn check(&self, record: &Record) -> Result<(), Refusal> {
        if self._reserved != [0; 5] {
            return Err(Refusal::Reserved);
        }
        if !is_name(&self.name) || !is_protocol(&self.protocol) {
            return Err(Refusal::Name);
        }
        if !role::known(self.role) || !payload::known(self.payload) {
            return Err(Refusal::Value);
        }
        if self.version_min == 0 || self.version < self.version_min {
            return Err(Refusal::Quantity);
        }
        if self.entries < 2 || self.entries > 65_536 || !self.entries.is_power_of_two() {
            return Err(Refusal::Quantity);
        }
        // A data ring offering the control ring's feature bit is a second
        // control ring under another name, and RFC 0008 permits exactly one.
        if self.features & crate::feature::CONTROL_EVENTS != 0 {
            return Err(Refusal::NotUnderThisPolicy);
        }
        if self.features_required & !self.features != 0 {
            return Err(Refusal::Quantity);
        }
        // The payload path *is* the feature bit. Naming one without the other
        // is an intention without a mechanism, R01.
        if self.payload == payload::SHARED_VIRTUAL
            && self.features & crate::feature::SHARED_VIRTUAL_MEMORY == 0
        {
            return Err(Refusal::NotUnderThisPolicy);
        }
        if self.role == role::SERVER {
            if self.clients == 0 || self.clients > 64 {
                return Err(Refusal::Quantity);
            }
            // A server names nobody: its clients hold *its* endpoint.
            if self.connects_through != NO_CAPABILITY {
                return Err(Refusal::NotUnderThisPolicy);
            }
            return Ok(());
        }
        // A client ring has one peer, so a client count on it is a field that
        // means nothing.
        if self.clients != 0 {
            return Err(Refusal::NotUnderThisPolicy);
        }
        let Some(through) = record.needs().get(self.connects_through as usize) else {
            return Err(Refusal::Quantity);
        };
        // `write` on an endpoint is the right to connect. RFC 0008's table.
        if through.cap_type() != Some(CapType::Endpoint)
            || !rights::holds(through.rights, rights::WRITE)
        {
            return Err(Refusal::NotUnderThisPolicy);
        }
        Ok(())
    }
}

/// Is this a name the schema admits: `[a-z0-9-]`, non-empty, no edge hyphen,
/// NUL-padded with nothing after the first NUL?
///
/// The padding rule is the one worth stating. A name with a byte after its
/// terminator would hash differently from the same name without one, so two
/// component files could name one component and carry two [`ContentId`]s —
/// which is the failure a place refilled by hash would show as *a different
/// manifest is a different place*.
fn is_name(bytes: &[u8; NAME_MAX]) -> bool {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
    if end == 0 {
        return false;
    }
    if bytes.iter().skip(end).any(|b| *b != 0) {
        return false;
    }
    let Some(name) = bytes.get(..end) else { return false };
    if name.first() == Some(&b'-') || name.last() == Some(&b'-') {
        return false;
    }
    name.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// As [`is_name`], with `.` admitted: a protocol name is `[a-z0-9.-]`.
fn is_protocol(bytes: &[u8; NAME_MAX]) -> bool {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
    if end == 0 {
        return false;
    }
    if bytes.iter().skip(end).any(|b| *b != 0) {
        return false;
    }
    let Some(name) = bytes.get(..end) else { return false };
    if name.first() == Some(&b'-') || name.last() == Some(&b'-') {
        return false;
    }
    name.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'.')
}

/// Put a name into a padded field, or refuse it.
///
/// The one place a name becomes bytes, so that a builder cannot produce a
/// record [`Record::read`] would refuse for a reason the builder could have
/// seen.
///
/// # Errors
///
/// [`Refusal::Name`] for anything [`is_name`] would refuse.
pub fn name_bytes(name: &str) -> Result<[u8; NAME_MAX], Refusal> {
    let mut out = [0u8; NAME_MAX];
    if name.len() > NAME_MAX {
        return Err(Refusal::Name);
    }
    for (slot, byte) in out.iter_mut().zip(name.as_bytes()) {
        *slot = *byte;
    }
    if is_name(&out) { Ok(out) } else { Err(Refusal::Name) }
}

/// As [`name_bytes`], for a protocol name.
///
/// # Errors
///
/// [`Refusal::Name`] for anything [`is_protocol`] would refuse.
pub fn protocol_bytes(name: &str) -> Result<[u8; NAME_MAX], Refusal> {
    let mut out = [0u8; NAME_MAX];
    if name.len() > NAME_MAX {
        return Err(Refusal::Name);
    }
    for (slot, byte) in out.iter_mut().zip(name.as_bytes()) {
        *slot = *byte;
    }
    if is_protocol(&out) { Ok(out) } else { Err(Refusal::Name) }
}

/// Write a record into the first bytes of a component file.
///
/// The counterpart of [`Record::read`], and it is here rather than in `xtask`
/// for the reason the whole crate exists: the layout is load-bearing against a
/// reader that was not built from this source, so there is one place that knows
/// it. `xtask` fills a [`Record`] by name — every field is public — and calls
/// this; a field added to the record without being filled is a compile error
/// there rather than a zero byte the frame refuses much later.
///
/// # Errors
///
/// [`Refusal::Truncated`] when `out` is shorter than a record.
pub fn encode(record: &Record, out: &mut [u8]) -> Result<(), Refusal> {
    let size = core::mem::size_of::<Record>();
    let Some(head) = out.get_mut(..size) else { return Err(Refusal::Truncated) };
    // SAFETY: `Record` is `#[repr(C)]` with no padding — the assertion above
    // this function's module says so and would fail the build otherwise — and
    // every field is an integer or an array of integers, so every byte of it is
    // initialised and there is no provenance to lose. `size` bytes are read
    // from a live `&Record` and written to a slice of exactly that length,
    // which cannot overlap it: `out` is a `&mut` and `record` a `&`.
    let bytes =
        unsafe { core::slice::from_raw_parts(core::ptr::from_ref(record).cast::<u8>(), size) };
    head.copy_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A soft-class manifest that reads: one untyped need, one endpoint need,
    /// one client ring through it, `always` with a budget.
    fn well_formed() -> Record {
        let mut record = Record::EMPTY;
        record.name = name_bytes("store").unwrap();
        record.domain = domain::PRIVATE;
        record.restart = restart::ALWAYS;
        record.class = class::SOFT;
        record.memory_bytes = 16 * FRAME_BYTES;
        record.backoff_first_ticks = 8;
        record.backoff_max_ticks = 64;
        record.max_restarts = 3;
        record.budget_window_ticks = 3_000;
        record.image_bytes = 4;
        // The honest declaration, which is what a fixture that is not testing
        // this field should carry: RFC 0063 and `crate::transfer`.
        record.transfer = Declaration::RESTART_ONLY;

        record.capability[0] = Need {
            name: name_bytes("account").unwrap(),
            bytes: 8 * FRAME_BYTES,
            kind: CapType::Untyped.to_wire(),
            rights: rights::READ | rights::DERIVE | rights::REVOKE | rights::GRANT,
            route: route::SUPERVISOR,
            ..Need::EMPTY
        };
        record.capability[1] = Need {
            name: name_bytes("peer").unwrap(),
            kind: CapType::Endpoint.to_wire(),
            rights: rights::WRITE | rights::GRANT,
            route: route::SUPERVISOR,
            ..Need::EMPTY
        };
        record.capabilities = 2;

        record.ring[0] = Ring {
            name: name_bytes("data").unwrap(),
            protocol: protocol_bytes("f.store.v1").unwrap(),
            version_min: 1,
            version: 1,
            entries: 16,
            role: role::CLIENT,
            payload: payload::INLINE,
            connects_through: 1,
            ..Ring::EMPTY
        };
        record.rings = 1;

        // The tree, because RFC 0065 makes one part of what a well-formed
        // manifest declares: a root and one counter under it. A fixture with no
        // tree would be a fixture of a component the supervisor refuses, which
        // is what `a_declaration_with_no_root_or_a_broken_one_is_refused`
        // constructs on purpose rather than inheriting by accident.
        record.state[0] =
            node(1, 0, crate::state::kind::SUBTREE, crate::state::unit::NONE, b"store");
        record.state[1] =
            node(2, 1, crate::state::kind::COUNTER, crate::state::unit::EVENTS, b"served");
        record.state_nodes = 2;
        record
    }

    /// One declared node, written the way a manifest compiler writes one.
    fn node(id: u32, parent: u32, kind: u8, unit: u8, name: &[u8]) -> Node {
        let mut out = Node::EMPTY;
        out.id = id;
        out.parent = parent;
        out.kind = kind;
        out.unit = unit;
        out.name_len = name.len() as u8;
        out.name.get_mut(..name.len()).unwrap().copy_from_slice(name);
        out
    }

    /// How much image every fixture below carries. Four bytes, because the
    /// image's *content* is not what any of these tests are about and its
    /// length is.
    const IMAGE: usize = 4;

    /// A module buffer, aligned the way the loader's page-aligned module is.
    ///
    /// `#[repr(align(8))]` rather than a heap allocation, because this crate is
    /// `no_std` and the tests are the same code the frame runs. It is also the
    /// only way to exercise [`Refusal::Unaligned`]'s *absence* honestly: a
    /// buffer that happened to be aligned would pass whatever the check did.
    #[repr(C, align(8))]
    struct Module([u8; core::mem::size_of::<Record>() + IMAGE]);

    fn module(record: &Record) -> Module {
        let mut bytes = Module([0u8; core::mem::size_of::<Record>() + IMAGE]);
        encode(record, &mut bytes.0).unwrap();
        bytes
    }

    fn read(bytes: &[u8]) -> Result<&Record, Refusal> {
        Record::read(bytes)
    }

    /// One thing wrong with a record, and what reading it should earn.
    ///
    /// A named type rather than a tuple in the test body, because a `&[(&str,
    /// fn(&mut Record), Refusal)]` is a signature a reader has to parse before
    /// they can read the cases — which is the whole of what clippy's complaint
    /// about it is worth.
    type Lie = (&'static str, fn(&mut Record), Refusal);

    /// One field that means nothing under what the record declares. Every one of
    /// these earns the same refusal, which is why it needs no third element.
    type Meaningless = (&'static str, fn(&mut Record));

    #[test]
    fn a_well_formed_record_survives_the_round_trip() {
        let record = well_formed();
        let bytes = module(&record);
        let back = read(&bytes.0).expect("a well-formed record was refused");
        assert_eq!(back.label(), b"store");
        assert_eq!(back.needs().len(), 2);
        assert_eq!(back.rings().len(), 1);
        assert_eq!(back.image(&bytes.0).unwrap().len(), IMAGE);
    }

    #[test]
    fn every_structural_lie_is_refused() {
        // One field wrong at a time, because a fixture that breaks two things
        // is caught by whichever check notices first and the check it was
        // written for stays unexercised.
        let cases: &[Lie] = &[
            ("magic", |r| r.magic = 0, Refusal::NotAManifest),
            ("schema", |r| r.schema = SCHEMA + 1, Refusal::Schema),
            ("record length", |r| r.record_bytes += 8, Refusal::RecordSize),
            ("reserved", |r| r._reserved[1] = 1, Refusal::Reserved),
            ("no image", |r| r.image_bytes = 0, Refusal::Quantity),
            ("domain", |r| r.domain = 9, Refusal::Value),
            ("policy", |r| r.restart = 0, Refusal::Value),
            ("class", |r| r.class = 4, Refusal::Value),
            ("capability count", |r| r.capabilities = 17, Refusal::Count),
            ("ring count", |r| r.rings = 9, Refusal::Count),
            ("name", |r| r.name[0] = b'-', Refusal::Name),
            ("padding after a name", |r| r.name[NAME_MAX - 1] = b'x', Refusal::Name),
        ];
        for (what, break_it, expect) in cases {
            let mut record = well_formed();
            break_it(&mut record);
            let bytes = module(&record);
            assert_eq!(read(&bytes.0).err(), Some(*expect), "{what} was not refused as expected");
        }
    }

    #[test]
    fn a_slot_past_the_count_must_be_zero() {
        // The rule that keeps a content hash honest: bytes nobody reads are
        // bytes two files can differ in while naming one component.
        let mut record = well_formed();
        record.capability[5].kind = CapType::Frame.to_wire();
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Count));

        let mut record = well_formed();
        record.ring[3].entries = 4;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Count));

        let mut record = well_formed();
        record.binding[2] = Binding { vendor: 0x1AF4, device: 0x1042 };
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Count));
    }

    #[test]
    fn a_declared_device_is_a_part_and_the_list_is_canonical() {
        // A driver that declares two parts is the shape this tree actually
        // has — a virtio device answers to a modern id and a transitional one —
        // so the accepting case is the two-entry one.
        let mut record = well_formed();
        record.binding[0] = Binding { vendor: 0x1AF4, device: 0x1001 };
        record.binding[1] = Binding { vendor: 0x1AF4, device: 0x1042 };
        record.devices = 2;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).map(|r| r.bindings().len()), Ok(2));

        // Out of order, and refused rather than sorted: a reader that reordered
        // would let two component files carry one meaning, and a content
        // address that names two things names nothing.
        let mut swapped = record;
        swapped.binding.swap(0, 1);
        let bytes = module(&swapped);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Order));

        // One part declared twice is an author with two beliefs about one
        // thing, not a list to be de-duplicated.
        let mut twice = record;
        twice.binding[1] = twice.binding[0];
        let bytes = module(&twice);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Order));

        // Neither zero nor the value a bus returns when nothing answered: the
        // second would match every empty slot on the machine.
        for bad in [
            Binding { vendor: 0, device: 0x1042 },
            Binding { vendor: 0x1AF4, device: 0 },
            Binding { vendor: Binding::NO_VENDOR, device: 0x1042 },
        ] {
            let mut record = well_formed();
            record.binding[0] = bad;
            record.devices = 1;
            let bytes = module(&record);
            assert_eq!(read(&bytes.0).err(), Some(Refusal::Value), "{bad:?}");
        }
    }

    #[test]
    fn a_field_that_means_nothing_is_refused_and_not_ignored() {
        let cases: &[Meaningless] = &[
            ("a backoff under never", |r| {
                r.restart = restart::NEVER;
            }),
            ("cpu fields in the soft class", |r| r.cores = 1),
            ("frames on an untyped need", |r| r.capability[0].frames = 1),
            ("a sibling on a supervisor route", |r| {
                r.capability[1].sibling = name_bytes("other").unwrap();
            }),
            ("clients on a client ring", |r| r.ring[0].clients = 2),
        ];
        for (what, break_it) in cases {
            let mut record = well_formed();
            break_it(&mut record);
            let bytes = module(&record);
            assert_eq!(
                read(&bytes.0).err(),
                Some(Refusal::NotUnderThisPolicy),
                "{what} was not refused"
            );
        }
    }

    #[test]
    fn a_budget_that_can_never_be_exhausted_is_refused() {
        // A window below the backoff cap means consecutive restarts are further
        // apart than the window, so the count never reaches its maximum: a
        // policy that says `on_fault` with a budget and means `always`.
        let mut record = well_formed();
        record.budget_window_ticks = record.backoff_max_ticks - 1;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Quantity));
    }

    #[test]
    fn execute_on_an_endpoint_is_refused_here_rather_than_at_the_spawn() {
        let mut record = well_formed();
        record.capability[1].rights |= rights::EXECUTE;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Rights));
    }

    #[test]
    fn a_client_ring_must_name_an_endpoint_it_may_connect_through() {
        // Naming the untyped need instead of the endpoint.
        let mut record = well_formed();
        record.ring[0].connects_through = 0;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::NotUnderThisPolicy));

        // Naming a slot that is not there at all.
        let mut record = well_formed();
        record.ring[0].connects_through = 7;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Quantity));

        // The endpoint without `write`, which is the right to connect.
        let mut record = well_formed();
        record.capability[1].rights = rights::GRANT;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::NotUnderThisPolicy));
    }

    #[test]
    fn a_truncated_module_is_refused_rather_than_read() {
        let record = well_formed();
        let bytes = module(&record);
        for len in [0usize, 8, core::mem::size_of::<Record>() - 1] {
            assert_eq!(read(&bytes.0[..len]).err(), Some(Refusal::Truncated), "at {len} bytes");
        }
        // And a byte nobody reads on the end: the module is exactly the record
        // and the image, because the content hash covers the whole of it.
        let mut longer = module(&record);
        longer.0[core::mem::size_of::<Record>()..].fill(0);
        let mut short = record;
        short.image_bytes = (IMAGE - 1) as u32;
        let bytes = module(&short);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Truncated));
    }

    #[test]
    fn the_backoff_doubles_and_is_capped() {
        let record = well_formed();
        assert_eq!(record.backoff_ticks(0), 8);
        assert_eq!(record.backoff_ticks(1), 16);
        assert_eq!(record.backoff_ticks(2), 32);
        assert_eq!(record.backoff_ticks(3), 64);
        // Capped, and it stays capped however far it is asked. The saturation
        // is what stops a manifest's own numbers reaching an overflow.
        assert_eq!(record.backoff_ticks(4), 64);
        assert_eq!(record.backoff_ticks(u32::MAX), 64);
    }

    #[test]
    fn the_policy_table_is_rfc_0008s() {
        let mut record = well_formed();
        for (policy, faulted, exited, expect) in [
            (restart::NEVER, true, false, false),
            (restart::NEVER, false, true, false),
            (restart::ON_FAULT, true, false, true),
            (restart::ON_FAULT, false, true, false),
            (restart::ALWAYS, true, false, true),
            (restart::ALWAYS, false, true, true),
            // A stop is neither a fault nor an exit, and no policy restarts
            // after one: it is the supervisor's own decision.
            (restart::ALWAYS, false, false, false),
        ] {
            record.restart = policy;
            assert_eq!(
                record.restarts_after(faulted, exited),
                expect,
                "policy {} with faulted={faulted} exited={exited}",
                restart::label(policy)
            );
        }
    }

    #[test]
    fn the_identity_is_over_the_record_and_the_image_together() {
        let record = well_formed();
        let bytes = module(&record);
        let first = ContentId::of(&bytes.0);
        assert_eq!(first, ContentId::of(&bytes.0), "the same bytes identified differently");

        // A byte of the image moves it, which is the half a record-only hash
        // would miss — and the half that decides whether a place refilled by
        // hash gets the same code back.
        let mut other = module(&record);
        let last = other.0.len() - 1;
        other.0[last] ^= 1;
        assert_ne!(ContentId::of(&other.0), first, "the image does not reach the identity");

        // And a byte of the record moves it too.
        let mut third = well_formed();
        third.max_restarts = 4;
        assert_ne!(
            ContentId::of(&module(&third).0),
            first,
            "the record does not reach the identity"
        );
    }

    #[test]
    fn a_declared_class_maps_onto_the_ceiling_a_service_is_admitted_for() {
        // The two ordinal spaces, checked against each other rather than
        // assumed equal. They are not equal and never were: a manifest's
        // `soft` is 1 and so is `class::SOFT`, which is a coincidence at one
        // value and a trap at the other — a manifest's `hard` is 2 and
        // `class::HARD` is 0, so a supervisor that cast one to the other would
        // admit a hard-class driver at the *batch* ceiling and every request it
        // served would silently be batch work.
        assert_eq!(class::admitted(class::SOFT), Some(crate::class::SOFT));
        assert_eq!(class::admitted(class::HARD), Some(crate::class::HARD));
        assert_ne!(u16::from(class::HARD), crate::class::HARD, "the trap this map exists for");

        // Everything else is refused rather than approximated, including the
        // zero a record that declares no reservation carries. What a component
        // with no declaration is admitted for is RFC 0025's answer and the
        // caller's to apply.
        for value in [0u8, 3, 4, 0xFF] {
            assert_eq!(class::admitted(value), None, "{value} was read as a class");
            assert!(!class::known(value));
        }

        // And every ordinal this map answers is one `Admitted` will accept, so
        // a supervisor never holds a ceiling the ABI would refuse to build.
        for value in [class::SOFT, class::HARD] {
            let ordinal = class::admitted(value).expect("a known class");
            assert!(crate::deadline::Admitted::new(ordinal).is_some());
        }
    }

    /// A record that declares nothing about being updated in place is refused,
    /// which is what makes `restart_only` a declaration rather than a silence.
    /// RFC 0063.
    #[test]
    fn a_record_with_no_transfer_mode_is_refused() {
        let mut record = well_formed();
        record.transfer = Declaration::EMPTY;
        let bytes = module(&record);
        assert_eq!(read(&bytes.0).err(), Some(Refusal::Value));
    }

    /// The three quantities mean nothing under `restart_only`, so a non-zero
    /// one is refused rather than ignored — the same rule a backoff under
    /// `never` is held to, and for the same reason: a reader who sees a record
    /// width will believe there is one.
    #[test]
    fn a_quantity_under_restart_only_is_refused() {
        for mutate in [
            (|d: &mut Declaration| d.schema = 1) as fn(&mut Declaration),
            |d: &mut Declaration| d.record_bytes = 8,
            |d: &mut Declaration| d.records_max = 1,
        ] {
            let mut record = well_formed();
            mutate(&mut record.transfer);
            let bytes = module(&record);
            assert_eq!(read(&bytes.0).err(), Some(Refusal::NotUnderThisPolicy));
        }
    }

    /// `in_place` is the mode with arithmetic behind it: a schema, a width that
    /// records can be laid end to end at, a bound, and a window the component's
    /// own account can hold.
    #[test]
    fn an_in_place_declaration_is_judged_field_by_field() {
        let sound = Declaration {
            schema: 1,
            record_bytes: 32,
            records_max: 16,
            mode: mode::IN_PLACE,
            _reserved: [0; 3],
        };
        let mut record = well_formed();
        record.transfer = sound;
        assert!(read(&module(&record).0).is_ok(), "a sound declaration was refused");

        for (broken, why) in [
            (Declaration { schema: 0, ..sound }, "a state-record schema of zero"),
            (Declaration { record_bytes: 0, ..sound }, "a record of no width"),
            (Declaration { records_max: 0, ..sound }, "a bound of no records"),
            (Declaration { record_bytes: 12, ..sound }, "a width records cannot be aligned at"),
            // The window is bought out of this component's own account, and
            // `well_formed` declares sixteen pages of it.
            (Declaration { records_max: 4096, ..sound }, "a window larger than the account"),
        ] {
            let mut record = well_formed();
            record.transfer = broken;
            assert_eq!(read(&module(&record).0).err(), Some(Refusal::Quantity), "{why}");
        }
    }

    /// R04 reaches inside the declaration too.
    #[test]
    fn a_reserved_byte_in_the_declaration_is_refused() {
        let mut record = well_formed();
        record.transfer._reserved = [0, 1, 0];
        assert_eq!(read(&module(&record).0).err(), Some(Refusal::Reserved));
    }

    /// Every way a declared state tree can fail to be one, and each earns its
    /// own refusal.
    ///
    /// The properties are `crate::state::validate`'s, checked one layer earlier
    /// so that the finding names the manifest. The one that is *not* the wire
    /// format's is **one root**: RFC 0065 argues it, and it is the property
    /// that makes a mount reach a whole tree rather than whichever half a
    /// reader started walking from.
    #[test]
    fn a_declaration_with_no_root_or_a_broken_one_is_refused() {
        assert!(read(&module(&well_formed()).0).is_ok(), "the control was refused");

        let cases: [Lie; 8] = [
            ("a second root", |r| r.state[1].parent = 0, Refusal::Quantity),
            ("a root with a parent", |r| r.state[0].parent = 9, Refusal::Quantity),
            ("a parent nothing names", |r| r.state[1].parent = 7, Refusal::Quantity),
            ("an id that does not ascend", |r| r.state[1].id = 1, Refusal::Quantity),
            ("a node with no id", |r| r.state[0].id = 0, Refusal::Quantity),
            ("a kind that is not one", |r| r.state[1].kind = 0, Refusal::Value),
            ("a node with no name", |r| r.state[1].name_len = 0, Refusal::Name),
            ("a reserved byte carrying something", |r| r.state[1]._reserved = 1, Refusal::Reserved),
        ];
        for (what, bend, want) in cases {
            let mut record = well_formed();
            bend(&mut record);
            assert_eq!(read(&module(&record).0).err(), Some(want), "{what} was accepted");
        }

        // A byte past a declared name, and an entry past the count: two places
        // a hashed record could differ while naming one component.
        let mut trailing = well_formed();
        trailing.state[1].name[15] = b'x';
        assert_eq!(read(&module(&trailing).0).err(), Some(Refusal::Name), "a byte past a name");

        let mut past = well_formed();
        past.state[2] = node(9, 1, crate::state::kind::GAUGE, crate::state::unit::NONE, b"ghost");
        assert_eq!(read(&module(&past).0).err(), Some(Refusal::Count), "an entry past the count");

        let mut many = well_formed();
        many.state_nodes = STATE_NODES_MAX as u8 + 1;
        assert_eq!(read(&module(&many).0).err(), Some(Refusal::Count), "a count past its bound");
    }

    /// A record declaring no tree is *readable*, and that is the split RFC 0065
    /// makes on purpose: refusing it here would leave a tool that reads
    /// component files unable to say which of them publishes nothing, and the
    /// refusal belongs where every other admission refusal is taken.
    #[test]
    fn a_record_that_declares_no_tree_reads_and_says_so() {
        let mut record = well_formed();
        record.state = [Node::EMPTY; STATE_NODES_MAX];
        record.state_nodes = 0;
        let bytes = module(&record);
        let read = read(&bytes.0).expect("a record with no tree is well formed");
        assert!(read.state_nodes().is_empty(), "a mute record claimed to declare something");
    }

    /// A declaration becomes the schema block the frame writes, and that block
    /// is one `crate::state::validate` accepts.
    ///
    /// The property the whole mechanism rests on: the schema and the data block
    /// come from *one* declaration, which is the build-time obligation RFC 0013
    /// creates and names as the only defence against the two drifting. A
    /// conversion that produced a schema the reader refused would be that drift
    /// on its first day.
    #[test]
    fn a_declaration_becomes_a_schema_block_the_reader_accepts() {
        let record = well_formed();
        let mut schema = [crate::state::SchemaEntry::ZERO; STATE_NODES_MAX];
        for (index, node) in record.state_nodes().iter().enumerate() {
            schema[index] = node.entry(index as u32);
        }
        let nodes = record.state_nodes().len();
        let header = crate::state::TreeHeader {
            magic: crate::state::TREE_MAGIC,
            version: crate::state::TREE_VERSION,
            nodes: nodes as u32,
            schema_offset: 64,
            data_offset: 64 + nodes as u32 * 32,
            generation: 0,
            _reserved: [0; 3],
        };
        assert_eq!(header.check(4096), Ok(()), "the header the frame would write is unreadable");
        assert_eq!(
            crate::state::validate(&header, &schema[..nodes]),
            Ok(()),
            "the schema the frame would write is not one a reader accepts"
        );
        assert_eq!(schema[1].label(), b"served");
        assert_eq!(schema[1].offset, crate::state::WORD);
    }
}
