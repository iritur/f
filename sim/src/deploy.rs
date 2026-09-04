// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The seam: the component set the loader is handed, read here as the frame
//! reads it.
//!
//! *The loader is handed*, and not *the boot spawns*, because the two are not
//! the same set today and this module can only see the first. Which modules a
//! boot instantiates is the boot's own answer — one, at present — and
//! `cargo xtask sim --join` is what compares the two and declares the
//! difference. RFC 0036.
//!
//! # What this module is for
//!
//! RFC 0032 decided that the simulator models the system *above* the frame, and
//! stated the cost in one sentence: the word **boot** in E1-P01's exit is
//! answered by `cargo xtask trace --hash` and the word **workload** by
//! `cargo xtask sim`. It also named where the two are supposed to join — the
//! component manifest set — and recorded, in bold, that the join was **not
//! built**: a scenario was a table of integers and read no manifest, so the
//! seam was a stated location rather than a shared object.
//!
//! This module is that object. It reads the same component files the loader
//! hands the machine as boot modules — a [`Record`] and an image in one blob,
//! `cargo xtask component`, RFC 0030 — validates each with the frame's own
//! reader, and answers a [`Deployment`]: the ordered set of components, each
//! with the content hash a spawn names it by. [`crate::scenario`] turns that
//! into actors, and the trace writes the hashes down, so the two halves of
//! `boot-to-workload` now quote the same numbers rather than the same intention.
//!
//! # Why this is not a second reader
//!
//! Because a second reader is a second belief about what a component is.
//! `Record::read` is the frame's, in `f-abi`, and it is called here unchanged:
//! the magic, the schema, the record length, the image length, every field
//! against a closed set, every reserved byte required to be zero. A simulator
//! with a lenient parser would run a component set the frame would refuse, and
//! report having run the system.
//!
//! # The alignment, which is not a detail
//!
//! `Record::read` refuses a module that does not begin on an eight-byte
//! boundary, and it is right to: it hands back a reference *into* the module's
//! bytes, so reading one that is misaligned would be an unaligned load. A
//! `Vec<u8>` is aligned to one byte. Whether `fs::read` happens to hand back
//! something eight-aligned is a property of the allocator on the day, which is
//! exactly the kind of thing this crate exists not to depend on — a run that
//! refused its own component set once in a hundred boots would be the worst
//! failure available here. So a file is copied into [`Module`], whose alignment
//! is part of its type, and the bound that makes that possible is stated and
//! checked rather than assumed.

use std::fs;
use std::path::Path;

use f_abi::manifest::{ContentId, Record, Refusal as Malformed, restart};

use crate::scenario::Peer;

/// The extension a component file carries. `cargo xtask component` writes
/// `target/component/<name>.fc`.
pub const EXTENSION: &str = "fc";

/// The largest component file this reader will hold. Unit: bytes.
///
/// A record is 2 216 bytes and the frame reserves
/// `kernel::process::TEXT_PAGES` — sixteen — for a component's text, so a
/// component file is at most 67 752 and this is the next power of two above it.
/// It is a bound on a *buffer whose alignment is part of its type*, which is the
/// whole reason it is a constant rather than a `Vec`: see the module
/// documentation.
///
/// **It was eight kibibytes, derived from an image bound of one page, and the
/// day that stopped being true is exactly the day this refused a build** — which
/// is what the sentence that used to be here said would happen: *the day a
/// component's image grows past one page is the day the frame's loader changes
/// too, and a refusal here is a better place to find that out than a silent
/// truncation.* RFC 0047 is that day, `user/virtio-blk` is thirteen kibibytes of
/// image, and this number is derived from the frame's reservation rather than
/// from what today's build happens to produce — because a bound fitted to the
/// current artefact is a bound that refuses the next commit.
pub const MODULE_MAX: usize = 131_072;

/// Which ring protocols this simulator has a model for.
///
/// **Fail closed, R04.** A component whose data ring names a protocol that is
/// not in this table is refused, and the refusal names the protocol. The
/// alternative — treating an unknown protocol as a component with no device —
/// is a silent claim that the component has no device, which for the next
/// driver somebody writes would be false while the run reported having
/// simulated it.
///
/// The cost is real and worth stating: adding a component to `user/` puts a red
/// scenario in front of whoever adds it until they say what it is. That is the
/// seam being load-bearing rather than decorative, and it is the property this
/// whole module exists to buy.
///
/// *Reversal:* if this becomes a tax rather than a check — several components
/// whose protocols genuinely have no device below them — the answer is a field
/// in the manifest schema saying so, not a default here. RFC 0030 is where a
/// new field is argued, and a default in this table would be the simulator
/// deciding something a manifest should declare.
const MODELS: &[(&str, Peer)] = &[
    ("blk", Peer::Blk),
    ("net", Peer::Net),
    ("gpu", Peer::Gpu),
    // `store` has no device under it at all: it is the lifecycle fixture
    // `E1-B05` spawns, and `user/store/manifest.toml` declares an `inline`
    // payload precisely because it moves no bytes. `native` is the peer that
    // models a component with a registration table and a service time and
    // nothing below, which is what that component is.
    ("store", Peer::Native),
];

/// One component file, held where a [`Record`] may be read out of it.
///
/// The alignment is the type's, not the allocator's. See the module
/// documentation for why that matters more than it looks like it should.
///
/// `Debug` prints the length and not the bytes: eight kibibytes of mostly zero
/// in a failure message is a failure message nobody reads to the end.
pub struct Module {
    bytes: Box<Aligned>,
    len: usize,
}

/// Eight-aligned storage for a component file.
#[repr(align(8))]
struct Aligned([u8; MODULE_MAX]);

impl core::fmt::Debug for Module {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(out, "Module({} bytes)", self.len)
    }
}

impl Module {
    /// Copy a component file into a buffer a record can be read out of.
    ///
    /// # Errors
    ///
    /// [`Refusal::TooLarge`] for a file past [`MODULE_MAX`].
    pub fn hold(raw: &[u8]) -> Result<Self, Refusal> {
        if raw.len() > MODULE_MAX {
            return Err(Refusal::TooLarge { bytes: raw.len() });
        }
        let mut bytes = Box::new(Aligned([0u8; MODULE_MAX]));
        if let Some(head) = bytes.0.get_mut(..raw.len()) {
            head.copy_from_slice(raw);
        }
        Ok(Self { bytes, len: raw.len() })
    }

    /// The file's bytes, beginning on an eight-byte boundary.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.0.get(..self.len).unwrap_or(&[])
    }
}

/// Why a component set was refused.
///
/// One variant per thing that can be wrong, because a refusal that says only
/// *bad deployment* is one somebody debugs by deleting files. Every one of them
/// names what to look at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The directory of component files is not there.
    NoDirectory {
        /// Where it was looked for.
        at: String,
    },
    /// It is there and holds none.
    Empty {
        /// Where it was looked in.
        at: String,
    },
    /// A file could not be read.
    Unreadable {
        /// Which file.
        file: String,
        /// What the filesystem said.
        why: String,
    },
    /// A file is larger than [`MODULE_MAX`].
    TooLarge {
        /// How long it was. Unit: bytes.
        bytes: usize,
    },
    /// The frame's own reader refused the record.
    Malformed {
        /// Which file.
        file: String,
        /// Which disbelief, in `f-abi`'s own words.
        why: Malformed,
    },
    /// A name or a protocol that is not UTF-8. `Record::read` already refuses
    /// anything outside `[a-z0-9-]`, so this cannot happen from a validated
    /// record — and it is checked rather than unwrapped, because *cannot
    /// happen* is how a panic gets into a tool that runs in CI.
    NotText {
        /// Which file.
        file: String,
    },
    /// A component declaring no data ring, in a scenario that submits work to
    /// one. Named rather than skipped: a run that quietly drove one fewer
    /// component than the boot spawned is a run whose artefact means less than
    /// it says.
    NoRing {
        /// Which component, by the name its record declares.
        component: String,
    },
    /// A protocol [`MODELS`] has no model for.
    NoModel {
        /// Which component, by the name its record declares.
        component: String,
        /// The protocol its ring declares.
        protocol: String,
    },
    /// Two component files declaring one name. Which of them the boot spawns
    /// is a question about the loader, and a deployment that cannot answer it
    /// is not one.
    Twice {
        /// The name both of them declare.
        component: String,
    },
}

impl Refusal {
    /// A sentence for a person, naming what to do about it.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoDirectory { at } => format!(
                "no component files at {at}\n\n\
                 The deployment scenario runs the component set the loader is handed, read from \
                 the same compiled manifest records (RFC 0030). Build \
                 them with `cargo xtask component`, or run `cargo xtask sim`, which builds them \
                 first."
            ),
            Self::Empty { at } => {
                format!("{at} holds no `.{EXTENSION}` files, so there is no component set to run")
            }
            Self::Unreadable { file, why } => format!("reading {file}: {why}"),
            Self::TooLarge { bytes } => format!(
                "a component file of {bytes} bytes, against a bound of {MODULE_MAX}\n\n\
                 The bound is on a buffer whose alignment is part of its type — `sim/src/deploy.rs` \
                 says why — and an image past one page is a change to the frame's loader as well \
                 as to this number."
            ),
            Self::Malformed { file, why } => format!(
                "{file} is not a component file this build can read: {}\n\n\
                 This is `f_abi::manifest::Record::read` refusing, which is the frame's own \
                 reader and not a second one. The same bytes would be refused at a spawn.",
                why.message()
            ),
            Self::NotText { file } => {
                format!("{file} carries a name or a protocol that is not text")
            }
            Self::NoRing { component } => format!(
                "`{component}` declares no data ring, so there is nothing for a client to submit \
                 to.\n\n\
                 Every component in a deployment scenario is driven by one client over one \
                 declared ring. A component that legitimately has none needs a scenario that \
                 says what it is doing instead."
            ),
            Self::NoModel { component, protocol } => format!(
                "`{component}` serves the `{protocol}` protocol and this simulator has no model \
                 for it.\n\n\
                 Refused rather than defaulted: treating an unknown protocol as a component with \
                 no device would be the simulator claiming to have run something it did not. \
                 Either add a model, or add the mapping to `MODELS` in `sim/src/deploy.rs` and \
                 say in the diff which of the two it is."
            ),
            Self::Twice { component } => format!(
                "two component files declare `{component}`, and which one the boot spawns is a \
                 question about the loader"
            ),
        }
    }
}

/// One component, as its compiled record declares it.
///
/// Everything here is read out of the record. Nothing is a default and nothing
/// is a guess, which is what lets the trace say *this run covered these
/// components* and have it mean something.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Component {
    /// The component's name in the topology. Unit: none.
    pub name: String,
    /// The content hash over the record and the image together — the identity a
    /// spawn names, and the number the boot log prints beside `manifest`.
    /// Unit: none.
    pub id: u64,
    /// The data ring's name. Unit: none.
    pub ring: String,
    /// The protocol that ring speaks. Unit: none.
    pub protocol: String,
    /// Entries in the declared ring. Unit: entries, a power of two.
    pub entries: u32,
    /// How many clients the ring admits. Unit: clients.
    pub clients: u32,
    /// RFC 0005's speculation-domain kind. Unit: none — an
    /// `f_abi::manifest::domain` constant.
    pub domain: u8,
    /// What the supervisor does when it ends. Unit: none — an
    /// `f_abi::manifest::restart` constant.
    pub restart: u8,
    /// The pause before the first respawn. Unit: timer ticks, at the frame's
    /// own rate.
    ///
    /// The four numbers below are the *policy*, and they are here rather than in
    /// a scenario for the reason every other field on this structure is: a
    /// manifest is the reviewable statement of what a component is, and a
    /// harness that waited a pause of its own choosing would leave
    /// `user/virtio-blk/manifest.toml`'s backoff ladder as decoration.
    /// `crate::chaos` is what reads them; nothing else does yet, and saying so
    /// is cheaper than a reader wondering.
    pub backoff_first_ticks: u32,
    /// The cap the pause doubles up to. Unit: timer ticks.
    pub backoff_max_ticks: u32,
    /// Respawns inside the window before the place is retired.
    /// Unit: restarts.
    pub max_restarts: u32,
    /// The window that count is taken over. Unit: timer ticks.
    pub budget_window_ticks: u32,
    /// Bytes of image after the record. Unit: bytes.
    pub image_bytes: u32,
    /// What the simulator puts under it, from [`MODELS`].
    pub peer: Peer,
}

impl Component {
    /// Read one component file.
    ///
    /// # Errors
    ///
    /// [`Refusal`], naming the file and what about it was refused.
    pub fn read(file: &str, module: &Module) -> Result<Self, Refusal> {
        let bytes = module.as_slice();
        let record = Record::read(bytes)
            .map_err(|why| Refusal::Malformed { file: file.to_string(), why })?;
        let name =
            text(record.label()).ok_or_else(|| Refusal::NotText { file: file.to_string() })?;

        // The first declared ring. A record may carry up to eight and both
        // components in this tree declare one; taking the first rather than
        // searching by name is what `Record::rings` already orders, and a
        // component that declares several is a scenario question rather than a
        // reader question — see `Refusal::NoRing`'s message.
        let ring =
            record.rings().first().ok_or_else(|| Refusal::NoRing { component: name.clone() })?;
        let protocol = text(trim(&ring.protocol))
            .ok_or_else(|| Refusal::NotText { file: file.to_string() })?;
        let peer =
            MODELS.iter().find(|(named, _)| *named == protocol).map(|(_, peer)| *peer).ok_or_else(
                || Refusal::NoModel { component: name.clone(), protocol: protocol.clone() },
            )?;

        Ok(Self {
            name,
            id: ContentId::of(bytes).bits(),
            ring: text(trim(&ring.name))
                .ok_or_else(|| Refusal::NotText { file: file.to_string() })?,
            protocol,
            entries: ring.entries,
            clients: ring.clients,
            domain: record.domain,
            restart: record.restart,
            backoff_first_ticks: record.backoff_first_ticks,
            backoff_max_ticks: record.backoff_max_ticks,
            max_restarts: record.max_restarts,
            budget_window_ticks: record.budget_window_ticks,
            image_bytes: record.image_bytes,
            peer,
        })
    }

    /// The line this component contributes to the artefact's header.
    ///
    /// Without the `# ` a header line carries: `crate::trace::Trace::cover` adds
    /// that, so one place decides what a header line looks like.
    ///
    /// The name is padded to the schema's own bound and the hash is eighteen
    /// characters; the protocol is padded to twelve, and a protocol longer than
    /// that shifts the columns after it. That is legibility rather than
    /// reproduction, and the distinction is worth stating once: two runs at one
    /// commit read one record, so a column that moves moves in both. What would
    /// break reproduction is a field whose *value* differed between two
    /// machines, which is the next paragraph.
    ///
    /// The **path the file was read from is deliberately absent**. It differs
    /// between two machines that agree about everything that matters, and a
    /// reproduction check whose artefact carried one would fail for a reason
    /// with nothing to do with the run.
    #[must_use]
    pub fn cover(&self) -> String {
        format!(
            "component   {:<32} {:#018x}  {:<12} {:>6} entries {:>3} clients  {:<7} {:<8} as {}",
            self.name,
            self.id,
            self.protocol,
            self.entries,
            self.clients,
            f_abi::manifest::domain::label(self.domain),
            restart::label(self.restart),
            self.peer.label(),
        )
    }
}

/// The component set one run covers.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Deployment {
    components: Vec<Component>,
}

impl Deployment {
    /// Read every component file in `dir`.
    ///
    /// # Errors
    ///
    /// [`Refusal`], naming the directory or the file.
    pub fn read(dir: &Path) -> Result<Self, Refusal> {
        let at = dir.display().to_string();
        let listing = fs::read_dir(dir).map_err(|_| Refusal::NoDirectory { at: at.clone() })?;

        // Collected and sorted before anything is read. A directory hands its
        // entries back in whatever order the filesystem holds them, which is a
        // source of nondeterminism no lint can see and the exact class of thing
        // RFC 0004 is about. The sort below is by the *record's* name rather
        // than by the file's, so the order is a property of what the components
        // declare rather than of what somebody called the files.
        let mut files: Vec<String> = Vec::new();
        for entry in listing {
            let entry = entry
                .map_err(|why| Refusal::Unreadable { file: at.clone(), why: why.to_string() })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some(EXTENSION) {
                files.push(path.display().to_string());
            }
        }
        files.sort();
        if files.is_empty() {
            return Err(Refusal::Empty { at });
        }

        let mut components = Vec::new();
        for file in &files {
            let raw = fs::read(file)
                .map_err(|why| Refusal::Unreadable { file: file.clone(), why: why.to_string() })?;
            let module = Module::hold(&raw)?;
            components.push(Component::read(file, &module)?);
        }
        Self::of(components)
    }

    /// A deployment from components already read.
    ///
    /// The one constructor, so that the ordering rule and the duplicate check
    /// hold however a deployment was built — including in a test that encodes
    /// its records in memory rather than reading files.
    ///
    /// # Errors
    ///
    /// [`Refusal::Twice`] for two components of one name.
    pub fn of(mut components: Vec<Component>) -> Result<Self, Refusal> {
        components.sort_by(|a, b| a.name.cmp(&b.name));
        for pair in components.windows(2) {
            if let [first, second] = pair
                && first.name == second.name
            {
                return Err(Refusal::Twice { component: first.name.clone() });
            }
        }
        Ok(Self { components })
    }

    /// The components, in name order.
    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    /// How many. Unit: components.
    #[must_use]
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Is there nothing here?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Does this set hold a component with this content hash?
    ///
    /// What the join check asks: the boot log prints the hash of the component
    /// it spawned, and this is how the simulator answers *and I ran that one*.
    #[must_use]
    pub fn holds(&self, id: u64) -> bool {
        self.components.iter().any(|component| component.id == id)
    }
}

/// A NUL-padded field, without the padding.
fn trim(field: &[u8]) -> &[u8] {
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(field.len());
    field.get(..end).unwrap_or(&[])
}

/// A validated field as text, or nothing.
fn text(field: &[u8]) -> Option<String> {
    core::str::from_utf8(field).ok().map(str::to_string)
}

/// Component records shaped like the ones `xtask` compiles, built in memory.
///
/// Shared with `scenario.rs`'s tests, and in memory rather than read from
/// `target/component/` deliberately: a test that needs a build artefact fails in
/// a fresh checkout for a reason that is not a defect, and one that skipped
/// itself when the artefact was missing would pass in exactly the tree where it
/// had stopped checking anything. The file path is exercised by
/// `cargo xtask sim`, which builds the components first — that is where the
/// artefact-level claim belongs, because it is the claim about the real set.
#[cfg(test)]
pub(crate) mod fixture {
    use super::{Component, Module};
    use f_abi::cap::{CapType, rights};
    use f_abi::manifest::{
        FRAME_BYTES, NO_CAPABILITY, Need, Record, Ring, class, domain, encode, name_bytes, payload,
        protocol_bytes, restart, role, route,
    };

    /// A well-formed record: one untyped need, one server ring, soft class,
    /// restarted on a fault.
    #[must_use]
    pub fn record(name: &str, protocol: &str, entries: u32) -> Record {
        let mut record = Record::EMPTY;
        record.name = name_bytes(name).expect("a name the schema allows");
        record.domain = domain::PRIVATE;
        record.restart = restart::ON_FAULT;
        record.class = class::SOFT;
        record.memory_bytes = 16 * FRAME_BYTES;
        record.backoff_first_ticks = 8;
        record.backoff_max_ticks = 64;
        record.max_restarts = 3;
        record.budget_window_ticks = 3_000;
        record.image_bytes = 4;
        record.capability[0] = Need {
            name: name_bytes("account").expect("a name the schema allows"),
            bytes: 8 * FRAME_BYTES,
            kind: CapType::Untyped.to_wire(),
            rights: rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE,
            route: route::SUPERVISOR,
            ..Need::EMPTY
        };
        record.capabilities = 1;
        record.ring[0] = Ring {
            name: name_bytes("data").expect("a name the schema allows"),
            protocol: protocol_bytes(protocol).expect("a protocol the schema allows"),
            version_min: 1,
            version: 1,
            entries,
            clients: 4,
            role: role::SERVER,
            payload: payload::REGISTERED,
            // A server names nobody: its clients hold its endpoint, and the
            // schema refuses a server ring that names one to connect through.
            // `Ring::EMPTY` zeroes the field, and zero is a capability index.
            connects_through: NO_CAPABILITY,
            ..Ring::EMPTY
        };
        record.rings = 1;
        record
    }

    /// One component file: a record and four bytes of image.
    #[must_use]
    pub fn module(record: &Record) -> Module {
        let size = core::mem::size_of::<Record>();
        let mut bytes = vec![0u8; size + 4];
        encode(record, &mut bytes).expect("the buffer is a record long");
        Module::hold(&bytes).expect("a record and four bytes fit the bound")
    }

    /// One component, read the way a deployment reads it.
    #[must_use]
    pub fn component(name: &str, protocol: &str, entries: u32) -> Component {
        Component::read(name, &module(&record(name, protocol, entries)))
            .expect("a well-formed record")
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{component, module, record};
    use super::*;
    use f_abi::manifest::{Ring, domain};

    #[test]
    fn a_component_is_read_by_the_frames_own_reader() {
        let read = component("virtio-blk", "blk", 256);
        assert_eq!(read.name, "virtio-blk");
        assert_eq!(read.protocol, "blk");
        assert_eq!(read.ring, "data");
        assert_eq!(read.entries, 256);
        assert_eq!(read.clients, 4);
        assert_eq!(read.peer, Peer::Blk);
        assert_eq!(read.domain, domain::PRIVATE);
    }

    #[test]
    fn the_identity_is_the_one_a_spawn_names() {
        // `ContentId::of` over the record *and* the image, which is what
        // `kernel/src/component.rs` prints beside `manifest` and what the join
        // check compares against. Two components differing only in their image
        // are two identities.
        let one = module(&record("store", "store", 16));
        let mut other_bytes = one.as_slice().to_vec();
        let last = other_bytes.len() - 1;
        other_bytes[last] ^= 0xFF;
        let other = Module::hold(&other_bytes).expect("the same length");

        let a = Component::read("a", &one).expect("well formed");
        let b = Component::read("b", &other).expect("well formed");
        assert_eq!(a.id, ContentId::of(one.as_slice()).bits());
        assert_ne!(a.id, b.id, "a component's image is not in its identity");
    }

    #[test]
    fn a_protocol_with_no_model_is_refused_rather_than_defaulted() {
        // Fail closed, R04, and the whole reason the seam is load-bearing. The
        // day somebody adds a component this simulator cannot model, they are
        // told so rather than shown a green run that covered one component
        // fewer than it claimed.
        let refusal = Component::read("x", &module(&record("frobnicator", "frob", 16)))
            .expect_err("an unknown protocol is refused");
        assert_eq!(
            refusal,
            Refusal::NoModel { component: "frobnicator".to_string(), protocol: "frob".to_string() }
        );
        assert!(refusal.message().contains("MODELS"), "the refusal must say what to do");
    }

    #[test]
    fn a_component_with_no_data_ring_is_refused() {
        let mut bare = record("store", "store", 16);
        bare.ring[0] = Ring::EMPTY;
        bare.rings = 0;
        assert_eq!(
            Component::read("x", &module(&bare)),
            Err(Refusal::NoRing { component: "store".to_string() })
        );
    }

    #[test]
    fn bytes_that_are_not_a_component_file_are_refused_by_the_frames_reader() {
        let module = Module::hold(&[0u8; 64]).expect("inside the bound");
        assert_eq!(
            Component::read("junk.fc", &module),
            Err(Refusal::Malformed {
                file: "junk.fc".to_string(),
                why: f_abi::manifest::Refusal::Truncated
            })
        );
    }

    #[test]
    fn a_file_past_the_bound_is_refused_naming_both_numbers() {
        let refusal = Module::hold(&vec![0u8; MODULE_MAX + 1]).expect_err("past the bound");
        assert_eq!(refusal, Refusal::TooLarge { bytes: MODULE_MAX + 1 });
        assert!(refusal.message().contains(&MODULE_MAX.to_string()));
    }

    #[test]
    fn a_record_is_read_from_an_eight_byte_boundary() {
        // The property `Module` exists for. A `Vec<u8>` is aligned to one byte
        // and `Record::read` refuses an unaligned module, correctly — so a
        // reader that passed the vector straight through would work or not
        // depending on what the allocator returned that morning.
        let held = module(&record("store", "store", 16));
        assert_eq!(held.as_slice().as_ptr() as usize % 8, 0);
    }

    #[test]
    fn a_deployment_is_ordered_by_what_the_records_declare() {
        // Not by the order a directory listed its files, which is the
        // filesystem's business and varies between two machines holding the
        // same bytes.
        let forwards = Deployment::of(vec![
            component("virtio-blk", "blk", 256),
            component("store", "store", 16),
        ])
        .expect("two names");
        let backwards = Deployment::of(vec![
            component("store", "store", 16),
            component("virtio-blk", "blk", 256),
        ])
        .expect("two names");
        assert_eq!(forwards, backwards);
        assert_eq!(
            forwards.components().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["store", "virtio-blk"]
        );
    }

    #[test]
    fn two_components_of_one_name_are_refused() {
        assert_eq!(
            Deployment::of(vec![component("store", "store", 16), component("store", "store", 16)]),
            Err(Refusal::Twice { component: "store".to_string() })
        );
    }

    #[test]
    fn a_deployment_answers_whether_it_ran_the_component_the_boot_spawned() {
        let one = component("store", "store", 16);
        let id = one.id;
        let deployment = Deployment::of(vec![one]).expect("one component");
        assert!(deployment.holds(id));
        assert!(!deployment.holds(id ^ 1), "any hash at all was accepted");
    }

    #[test]
    fn a_missing_directory_says_what_to_run() {
        let refusal = Deployment::read(Path::new("target/there-is-no-such-directory-here"))
            .expect_err("nothing is there");
        assert!(refusal.message().contains("cargo xtask component"));
    }

    #[test]
    fn a_cover_line_has_the_same_width_whatever_it_holds() {
        // A header line is hashed with the rest of the artefact, so a column
        // that could move is a column that makes two identical runs disagree.
        let short = component("a", "blk", 2).cover();
        let long = component("a-very-long-component-name-here", "blk", 65536).cover();
        assert_eq!(short.len(), long.len(), "a column moved");
        assert!(!short.contains('/') && !short.contains('\\'), "a path reached the artefact");
    }
}
