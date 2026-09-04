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
pub const SCHEMA: u32 = 1;

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
    /// Reserved. Must be zero — a non-zero value is refused rather than
    /// ignored, per R04.
    /// Unit: none; this is not a quantity and is not expected to become one
    /// without a schema bump.
    pub _reserved: [u8; 3],
    /// The declared capabilities, in the order the supervisor supplies them and
    /// the order the `granted` notices arrive.
    /// Unit: entries; the first [`Record::capabilities`] are real and the rest
    /// are all zero.
    pub capability: [Need; CAPABILITIES_MAX],
    /// The declared data rings.
    /// Unit: entries; the first [`Record::rings`] are real and the rest are all
    /// zero.
    pub ring: [Ring; RINGS_MAX],
}

// The layout is the ABI. Pinned here so that a field reordered, widened or
// inserted is a build failure with a number in it rather than a component file
// two builds of this tree disagree about. A change to any of these three is a
// `schema` bump and a rebuild of every component file — RFC 0030 states that
// cost rather than hiding it.
const _: () = assert!(core::mem::size_of::<Need>() == 80);
const _: () = assert!(core::mem::size_of::<Ring>() == 104);
const _: () = assert!(core::mem::size_of::<Record>() == 2216);
// No padding anywhere: the sum of the parts is the whole. A padded record has
// bytes the reader never judges, and an unjudged byte inside a hashed structure
// is a place two files can differ while claiming to name one component.
const _: () = assert!(
    core::mem::size_of::<Record>()
        == 104
            + CAPABILITIES_MAX * core::mem::size_of::<Need>()
            + RINGS_MAX * core::mem::size_of::<Ring>()
);

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
        _reserved: [0; 3],
        capability: [Need::EMPTY; CAPABILITIES_MAX],
        ring: [Ring::EMPTY; RINGS_MAX],
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

        if record.magic != MAGIC {
            return Err(Refusal::NotAManifest);
        }
        if record.schema != SCHEMA {
            return Err(Refusal::Schema);
        }
        if record.record_bytes as usize != size {
            return Err(Refusal::RecordSize);
        }
        if record._reserved != [0; 3] {
            return Err(Refusal::Reserved);
        }
        if record.image_bytes == 0 {
            return Err(Refusal::Quantity);
        }
        // The module is exactly the record and the image, and nothing after it.
        // Trailing bytes are refused rather than ignored because the content
        // hash covers the whole module: bytes nobody reads are bytes two files
        // can differ in while naming one component.
        if module.len() != size + record.image_bytes as usize {
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

        record.check_restart()?;
        record.check_reservation()?;
        record.check_capabilities()?;
        record.check_rings()?;
        Ok(record)
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
        record
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
}
