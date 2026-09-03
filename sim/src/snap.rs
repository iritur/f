// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A running simulation, written out at a decision point and re-entered from
//! there.
//!
//! # What this is for, in one sentence
//!
//! `E1-P08`: *a long scenario bisects in seconds rather than hours*. A run whose
//! failure sits near the end costs a full replay every time somebody wants to
//! look at it again, and every narrowing pass a minimiser makes
//! ([`crate::sweep::MINIMISE_BUDGET`] of them, at up to a whole run each) pays
//! the same bill. A snapshot is that bill paid once.
//!
//! # The only property that makes a snapshot worth having
//!
//! **A restored run and a run that reached the same point by replaying are
//! indistinguishable.** Same decisions from there on, same records, same digest,
//! same verdict. That is the whole specification, because the failure mode of a
//! snapshot is not a crash — it is a field that quietly did not travel,
//! producing a plausible run that diverges from the real one. A plausible
//! divergent run is **worse than no snapshot**: it sends somebody looking for a
//! bug at a point the system never reached.
//!
//! So the property is tested rather than argued.
//! [`tests::a_restored_run_is_the_run_that_replayed`] takes every shipped
//! scenario at several seeds, cuts each run at points spread along its length,
//! and requires the restored tail to equal the replayed tail record for record —
//! plus the digest, the decision log, the fault count and the finishing instant.
//! A field that matters at any of those cuts moves one of those five.
//!
//! What that does **not** prove is stated in RFC 0043: a field that is saved,
//! restored, and influences nothing after any tested cut is a field this cannot
//! distinguish from a field that is absent. That is the residual, and it is the
//! residual every differential test has.
//!
//! # What has to travel, and what does not
//!
//! The interesting half of this task was finding out which sources of
//! nondeterminism are *state* and which are *derivations*, and RFC 0026 settled
//! that a task before anybody asked the question.
//!
//! - **The ordering stream and the fault streams are derivations.** A decision's
//!   value is `draw(seed, domain, site, occurrence)` and nothing else, so what
//!   travels is an occurrence count per site — a handful of `(label, u64)`
//!   pairs. There is no generator state, no parent and no tree. RFC 0026's
//!   split-by-identity bought that without being asked to, and it is why the
//!   answer to *what is in a snapshot of a seeded simulator* is not *the whole
//!   random tree*.
//! - **The value stream is a chain.** [`crate::World::draw`] steps a
//!   `f_env::split::Stream`, which folds its own output back into its state, so
//!   state `n` is reachable only by taking `n` steps. Those five words travel,
//!   through `Stream::state` — a method `E1-P08` is the reason for, and whose
//!   documentation says why the two halves of RFC 0026 needed different answers.
//!
//! The rest is ordinary: virtual time and everything due on it, the entries in
//! flight on every wire, the injector's per-class occurrence counts and strike
//! total, the artefact so far, and every actor's own state — which for a device
//! is its virtqueue, its control region, its registration table and its jobs,
//! and for a client is its buffer set, which buffers are out, and under which
//! tokens.
//!
//! # A snapshot is data on disk, so it is a wire format
//!
//! Fixed-width, little-endian, versioned, checksummed and **refused when it does
//! not match this build** (R04). Three separate refusals, because they are three
//! different mistakes:
//!
//! - [`FORMAT`] changes when the layout changes. An old file is refused by
//!   number rather than misread.
//! - [`build`] fingerprints everything a snapshot's *meaning* depends on that is
//!   not in the snapshot: the label table, the scenario tables field by field,
//!   the fault classes, the registration and buffer geometry, and which
//!   deliberate defects this binary was compiled with. A snapshot taken with
//!   `mutate-crossed-completion` on is not a snapshot of the binary without it,
//!   and the fingerprint says so before a byte is interpreted.
//! - The **commit** travels when the caller names one, and a caller who names a
//!   different one is refused. `(seed, commit)` is this tree's whole
//!   reproduction contract and a snapshot is a point inside one such pair.
//!
//! The fingerprint is the load-bearing one and the commit is the courteous one.
//! A fingerprint catches what a commit cannot — two builds of one commit with
//! different features — and a commit catches what a fingerprint cannot, which is
//! a change to a model's *behaviour* that moved no table. Neither alone is
//! enough, and RFC 0043 says so rather than leaving a reader to notice.
//!
//! # Where the cut is, and why it can be anywhere
//!
//! Between two steps. [`crate::Simulation::run_to`] takes one message from the
//! timeline, hands it to one actor and returns; between two of those, nothing in
//! the model holds a borrow, nothing is half-written, and the world is exactly
//! the sum of its parts. That is R05's shape — *nothing is delivered
//! asynchronously, every event is drained at a polling point* — collected as a
//! second dividend: a system whose events all arrive at one place has a
//! well-defined *between*, and a system with callbacks does not.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use f_abi::{Cqe, Sqe};

use crate::deploy::Deployment;
use crate::dev::Protocol;
use crate::fault::{Class, Injection};
use crate::scenario::{self, Scenario};
use crate::sweep::Trial;
use crate::{ActorId, Cut, Halt, Message, Outcome, Simulation, Trouble, World};

/// What a snapshot begins with, so that a file which is not one is refused as
/// *not a snapshot* rather than as a bad version.
pub const MAGIC: &[u8; 12] = b"F-SIM-SNAP\0\0";

/// The layout version.
///
/// Bumped whenever a field below moves, appears or disappears. It is not a
/// compatibility promise in either direction: this tree keeps a snapshot for
/// minutes rather than for years, and a reader that could interpret two layouts
/// would be a second definition of what a run is. RFC 0043.
pub const FORMAT: u32 = 1;

/// The largest snapshot this reader will consider. Unit: bytes.
///
/// Two gibibytes, far above anything the scenario tables can produce and far
/// below anything that would exhaust a machine. It bounds the outermost length
/// so that a corrupt one is refused rather than allocated against; every inner
/// count is checked against the bytes actually remaining.
pub const LIMIT: usize = 1 << 31;

/// One simulated minute. Unit: nanoseconds.
///
/// A constant because the exit criterion this module answers is written in
/// minutes — *a failure at simulated minute 40 is re-entered at minute 39* — and
/// a unit that appears in a criterion should appear in the code that meets it.
pub const MINUTE_NS: u64 = 60_000_000_000;

/// Why a snapshot was refused.
///
/// Every variant names what did not match and what was expected, because the
/// person reading it is holding a file and has to decide between regenerating it
/// and checking out a different commit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Broken {
    /// The file does not begin with [`MAGIC`].
    NotASnapshot,
    /// Written by a different layout of this format.
    Format {
        /// What the file says. Unit: none.
        found: u32,
        /// What this build reads. Unit: none.
        want: u32,
    },
    /// Written by a build whose tables, geometry or features differ from this
    /// one's.
    Build {
        /// The file's fingerprint. Unit: none.
        found: u64,
        /// This build's. Unit: none.
        want: u64,
    },
    /// Written at a different commit from the one the caller named.
    Commit {
        /// What the file says.
        found: String,
        /// What the caller asked for.
        want: String,
    },
    /// The bytes ran out.
    Truncated {
        /// How many were wanted. Unit: bytes.
        want: usize,
        /// How many were left. Unit: bytes.
        left: usize,
    },
    /// The body was read and bytes remained. Unit: bytes.
    Trailing(usize),
    /// The checksum over everything before it does not match.
    Checksum {
        /// What the file says. Unit: none.
        found: u64,
        /// What the bytes hash to. Unit: none.
        want: u64,
    },
    /// A label id no label in this build answers to. Unit: none — the id.
    Label(u32),
    /// A label this build produced that [`LABELS`] does not hold.
    ///
    /// A *save*-side refusal, and the reason there is one: a label written as
    /// some other label's id would restore into a run nobody had. Fail closed at
    /// the moment the loss would happen rather than at the moment it is noticed.
    Unlabelled(&'static str),
    /// An actor that has not been taught to save itself. Unit: none — its name.
    Unsaveable(&'static str),
    /// An actor tag no kind in this build answers to. Unit: none — the tag.
    Kind(u32),
    /// A scenario name this build's tables do not hold.
    NoSuchScenario(String),
    /// A value outside what its field can mean, naming the field.
    Bounds(&'static str),
    /// A part rebuilt from the file did not agree with what the file recorded
    /// beside it, naming the part.
    ///
    /// A registration table is rebuilt by replaying the registrations that made
    /// it rather than by copying its slots — `service.rs` argues why — and this
    /// is the check that the replay landed where the save said it did.
    Diverged(&'static str),
    /// The file could not be read or written. Unit: none — the reason.
    Io(String),
}

impl Broken {
    /// A sentence for a report.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NotASnapshot => {
                "this file does not begin with a snapshot's magic, so it is not one".to_string()
            }
            Self::Format { found, want } => format!(
                "snapshot format {found}, and this build reads {want}. A snapshot is kept for \
                 minutes and not for years: take a new one."
            ),
            Self::Build { found, want } => format!(
                "snapshot build {found:#018x}, and this build is {want:#018x}. The tables, the \
                 geometry or the compiled-in defects differ, so this file describes a run this \
                 binary cannot continue. Take a new one with this binary."
            ),
            Self::Commit { found, want } => format!(
                "snapshot taken at commit {found}, and {want} was asked for. A snapshot is a \
                 point inside one (seed, commit) pair and means nothing outside it."
            ),
            Self::Truncated { want, left } => {
                format!("the snapshot ends early: {want} byte(s) wanted, {left} left")
            }
            Self::Trailing(over) => format!("{over} byte(s) after the end of the snapshot"),
            Self::Checksum { found, want } => {
                format!("snapshot checksum {found:#018x}, and the bytes hash to {want:#018x}")
            }
            Self::Label(id) => format!("label {id} is not one this build knows"),
            Self::Unlabelled(label) => format!(
                "`{label}` is not in `snap::LABELS`, so it cannot be written into a snapshot. \
                 Every label a model can put in a record, a message or a decision belongs in \
                 that table — see its documentation for why the refusal is at this end."
            ),
            Self::Unsaveable(name) => format!(
                "the actor `{name}` has no snapshot, so this run cannot be written out. \
                 `Actor::save` refuses by default, which is why this is a message rather than \
                 a silently short file."
            ),
            Self::Kind(tag) => format!("actor tag {tag} is not one this build knows"),
            Self::NoSuchScenario(name) => {
                format!("the snapshot names the scenario `{name}`, which this build does not have")
            }
            Self::Bounds(field) => format!("`{field}` holds a value it cannot mean"),
            Self::Diverged(part) => format!(
                "`{part}` was rebuilt from the snapshot and did not match what the snapshot \
                 recorded beside it. The file is internally inconsistent; a run continued from \
                 it would be a run nobody had."
            ),
            Self::Io(why) => why.clone(),
        }
    }
}

/// Every label this crate can put in a trace record, a message kind or a
/// decision site.
///
/// # Why a table and not the string
///
/// A record carries two `&'static str`s and a decision carries one, and a
/// snapshot has to give back the *same* statics: the trace's fixed-width format
/// and every `match` on a message kind depend on it. Writing the bytes and
/// interning them on the way back would make a restored run's labels heap
/// strings that compare equal — which works until something matches on a
/// `&'static str` pattern or the column width moves. So the file carries an
/// index into this table, and this table is the definition.
///
/// # Why one table rather than each module's own list
///
/// Because the other arrangement fails silently. A label missing from here is
/// refused at *save* time by name ([`Broken::Unlabelled`]), which is a message
/// naming the file to edit; a label written as another label's id would be a
/// snapshot that restores into a different run. The table is also folded into
/// [`build`], so a snapshot cannot be read by a binary whose table differs —
/// including one where a label was merely reordered.
///
/// The order is therefore part of the format. Append; never insert, never
/// reorder, and bump [`FORMAT`] if you must.
pub const LABELS: &[&str] = &[
    // Actor names.
    crate::client::App::NAME,
    crate::actors::Client::NAME,
    // `actors::Service::NAME` is deliberately absent: it is the word "service",
    // which `proto::kind::SERVICE` below already carries. One string is one id,
    // and a table holding it twice would answer to two — harmless today,
    // because both ids read back as the same static, and wrong the day one of
    // the two constants is renamed and the other is not.
    // `tests::no_label_is_in_the_table_twice` is what keeps this true.
    crate::blk::Blk::NAME,
    crate::net::Net::NAME,
    crate::gpu::Gpu::NAME,
    crate::native::Native::NAME,
    crate::fault::ACTOR,
    // Message kinds the ring protocol uses.
    crate::proto::kind::START,
    crate::proto::kind::SUBMIT,
    crate::proto::kind::CQE,
    crate::proto::kind::RETRY,
    crate::proto::kind::GONE,
    crate::proto::kind::POLL,
    crate::proto::kind::SERVICE,
    crate::proto::kind::REAP,
    // What the ring protocol's actors write down.
    crate::proto::wrote::REGISTER,
    crate::proto::wrote::BOUND,
    crate::proto::wrote::ISSUE,
    crate::proto::wrote::DONE,
    crate::proto::wrote::REFUSED,
    crate::proto::wrote::RECLAIM,
    crate::proto::wrote::FINISHED,
    crate::proto::wrote::FULL,
    crate::proto::wrote::QUEUED,
    crate::proto::wrote::DENIED,
    crate::proto::wrote::TAKEN,
    crate::proto::wrote::SERVED,
    crate::proto::wrote::DROPPED,
    crate::proto::wrote::HELD,
    crate::proto::wrote::RESET,
    crate::proto::wrote::UNSUPP,
    crate::proto::wrote::IOERR,
    crate::proto::wrote::NOREACH,
    crate::proto::wrote::FENCED,
    crate::proto::wrote::LINKDOWN,
    // Stage one's pair, whose vocabulary is partly its own. The words it shares
    // with the ring protocol are the ones above: `actors::kind::START` and
    // `proto::kind::START` are one string, and a table that listed both would
    // have two ids for one label and a reader that could not tell which.
    crate::actors::kind::FINISH,
    crate::actors::kind::COMPLETE,
    crate::actors::wrote::QUEUE,
    // Decision sites.
    crate::time::CHANNEL,
    crate::actors::NEXT,
    crate::ENV_CHOOSE,
    crate::blk::Blk::COMPLETE,
    crate::blk::Blk::DROP,
    crate::blk::Blk::COALESCE,
    crate::net::Net::COMPLETE,
    crate::net::Net::DROP,
    crate::net::Net::COALESCE,
    crate::gpu::Gpu::COMPLETE,
    crate::gpu::Gpu::DROP,
    crate::gpu::Gpu::COALESCE,
    crate::native::Native::COMPLETE,
    // Fault classes, which a strike writes into the trace as its kind.
    "alloc",
    "mapfault",
    "faultin",
    "peergone",
    "doorbell",
    "partial",
    "latecqe",
];

/// The label table, indexed the other way.
///
/// A `BTreeMap` because RFC 0004 forbids the hash map that would otherwise be
/// reached for, and built once because a linear scan per record over a run of a
/// million records is the difference between a snapshot that costs nothing and
/// one nobody takes.
fn index() -> &'static BTreeMap<&'static str, u32> {
    static INDEX: OnceLock<BTreeMap<&'static str, u32>> = OnceLock::new();
    INDEX.get_or_init(|| {
        LABELS
            .iter()
            .enumerate()
            .map(|(at, label)| (*label, u32::try_from(at).unwrap_or(u32::MAX)))
            .collect()
    })
}

/// Which kind of actor a saved blob describes.
///
/// Numbers rather than names, and stable: the tag is the first field of every
/// actor's blob and the loader dispatches on it. Appending is free; changing one
/// is a [`FORMAT`] bump.
pub mod tag {
    /// [`crate::client::App`].
    pub const APP: u32 = 1;
    /// [`crate::actors::Client`].
    pub const CLIENT: u32 = 2;
    /// [`crate::actors::Service`].
    pub const SERVICE: u32 = 3;
    /// A [`crate::dev::Device`] over [`crate::blk::Blk`].
    pub const BLK: u32 = 4;
    /// A [`crate::dev::Device`] over [`crate::net::Net`].
    pub const NET: u32 = 5;
    /// A [`crate::dev::Device`] over [`crate::gpu::Gpu`].
    pub const GPU: u32 = 6;
    /// [`crate::native::Native`].
    pub const NATIVE: u32 = 7;
}

/// FNV-1a over bytes, for the file's own checksum.
///
/// The trace's [`crate::trace::digest`] skips carriage returns, which is right
/// for a log and wrong for a binary: a checksum blind to one byte value is a
/// checksum that cannot see a class of corruption. So this is the same
/// polynomial without that clause, and the two are deliberately not one
/// function — `trace.rs` says why its copy exists, and this says why it is not
/// that copy.
#[must_use]
pub fn checksum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The fingerprint of everything a snapshot's meaning depends on that is not in
/// the snapshot.
///
/// Computed rather than written down, so it cannot be forgotten: it folds the
/// label table in order, both scenario tables field by field, the fault class
/// labels, the registration and buffer geometry, and the deliberate defects this
/// binary was built with. Any of those changing changes this, and a snapshot
/// from before the change is refused by number rather than restored into a model
/// that means something else.
///
/// Unit: none — an opaque 64-bit value.
#[must_use]
pub fn build() -> u64 {
    static BUILD: OnceLock<u64> = OnceLock::new();
    *BUILD.get_or_init(|| {
        let mut text = format!("f-sim snapshot {FORMAT}\n");
        for label in LABELS {
            text.push_str(label);
            text.push('\n');
        }
        for class in Class::ALL {
            text.push_str(class.label());
            text.push('\n');
        }
        for scenario in scenario::SCENARIOS.iter().chain(scenario::LONG) {
            text.push_str(&fingerprint(scenario));
        }
        text.push_str(&format!(
            "geometry {} {} {} {:#x} {:#x}\n",
            crate::service::SLOTS,
            crate::client::BUFFERS,
            crate::virtq::QUEUE_SIZE,
            crate::service::GRANT_BASE,
            crate::service::GRANT_STRIDE,
        ));
        // The two deliberate defects. A snapshot taken with one on describes a
        // run the binary without it does not have, and this is what refuses the
        // pair rather than letting a restore diverge from its own replay.
        text.push_str(&format!(
            "defects {} {}\n",
            cfg!(feature = "mutate-crossed-completion"),
            cfg!(feature = "mutate-silent-reset"),
        ));
        checksum(text.as_bytes())
    })
}

/// One scenario, as the fingerprint sees it: every field, in order.
fn fingerprint(scenario: &Scenario) -> String {
    let mut out = format!(
        "{} {:?} {} {} {} {} {} {} {} {} {} {}",
        scenario.name,
        scenario.peer,
        scenario.clients,
        scenario.window,
        scenario.depth,
        scenario.operations,
        scenario.service_ns,
        scenario.spread_ns,
        scenario.retry_ns,
        scenario.buffer_bytes,
        scenario.extent,
        scenario.lose_one_in,
    );
    for injection in scenario.injects {
        out.push_str(&format!(
            " {}:{}:{}",
            injection.class.label(),
            injection.after,
            injection.one_in
        ));
    }
    out.push('\n');
    out
}

/// Bytes going out, and the one thing that can go wrong while they do.
///
/// Little-endian throughout, because a snapshot crosses a disk rather than a
/// machine boundary and one order stated is worth more than an argument about
/// which. Primitive writes cannot fail, so they answer nothing and the call
/// sites stay readable; the fallible write is [`Writer::label`], which records
/// the fault and carries on so that the refusal names the *first* missing label
/// rather than an arbitrary one.
pub struct Writer {
    bytes: Vec<u8>,
    fault: Option<Broken>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    /// An empty writer.
    #[must_use]
    pub const fn new() -> Self {
        Self { bytes: Vec::new(), fault: None }
    }

    /// One byte.
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// A flag, as one byte that is zero or one.
    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// Two bytes.
    pub fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Four bytes.
    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Four bytes, signed.
    pub fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Eight bytes.
    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// A count, as four bytes.
    ///
    /// A separate method from [`Writer::u32`] so a reader of a save function can
    /// see which numbers are lengths, and so a length past what four bytes hold
    /// is refused here rather than truncated.
    pub fn count(&mut self, value: usize) {
        match u32::try_from(value) {
            Ok(count) => self.u32(count),
            Err(_) => {
                self.fault.get_or_insert(Broken::Bounds("a count past four bytes"));
                self.u32(u32::MAX);
            }
        }
    }

    /// Raw bytes, length first.
    pub fn blob(&mut self, value: &[u8]) {
        self.count(value.len());
        self.bytes.extend_from_slice(value);
    }

    /// A string, length first.
    pub fn str(&mut self, value: &str) {
        self.blob(value.as_bytes());
    }

    /// One of the labels in [`LABELS`], as its index.
    ///
    /// Refuses a label the table does not hold, by name. See [`LABELS`] for why
    /// the refusal is at this end.
    pub fn label(&mut self, value: &'static str) {
        match index().get(value) {
            Some(id) => self.u32(*id),
            None => {
                self.fault.get_or_insert(Broken::Unlabelled(value));
                self.u32(u32::MAX);
            }
        }
    }

    /// A submission entry, field by field.
    ///
    /// Field by field and not as sixty-four bytes of memory: a `repr(C)` struct
    /// has padding, padding is not written, and a file whose bytes depended on
    /// what was left in a hole would differ between two runs of one seed.
    pub fn sqe(&mut self, entry: &Sqe) {
        self.u8(entry.opcode);
        self.u8(entry.flags);
        self.u16(entry.class);
        self.u32(entry.cap);
        self.u64(entry.user_data);
        self.u64(entry.deadline);
        self.u64(entry.offset);
        self.u32(entry.buf_set);
        self.u32(entry.buf_index);
        self.u32(entry.len);
        // The reserved word travels too. It is zero on every entry this crate
        // builds and R04 says a peer must send zero, but *a field that quietly
        // did not travel* is the exact failure this module is written against,
        // and a snapshot that normalised a field would hide the day one did not
        // hold it.
        self.u32(entry._reserved);
        self.u64(entry.ext[0]);
        self.u64(entry.ext[1]);
    }

    /// A completion entry, field by field.
    pub fn cqe(&mut self, entry: &Cqe) {
        self.u64(entry.user_data);
        self.i32(entry.result);
        self.u32(entry.flags);
        self.u64(entry.timestamp);
        self.u64(entry.ext);
    }

    /// The bytes, or the first thing that went wrong producing them.
    ///
    /// # Errors
    ///
    /// Whatever a write refused with, most often [`Broken::Unlabelled`].
    pub fn finish(self) -> Result<Vec<u8>, Broken> {
        match self.fault {
            Some(fault) => Err(fault),
            None => Ok(self.bytes),
        }
    }

    /// How many bytes have been written. Unit: bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Has nothing been written?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Bytes coming in, from a file nothing in this process wrote.
///
/// Every read is bounds-checked and the first failure is remembered; later reads
/// answer zero and the whole load is refused at [`Reader::finish`]. That shape
/// rather than a `Result` per read for the same reason [`Writer`] has it — a
/// loader is a list of fields and reads best as one — and it is sound because
/// nothing a faulted read answered is ever committed: the caller sees the
/// `Result` before the value reaches a simulation.
pub struct Reader<'b> {
    bytes: &'b [u8],
    at: usize,
    fault: Option<Broken>,
}

impl<'b> Reader<'b> {
    /// A reader over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'b [u8]) -> Self {
        Self { bytes, at: 0, fault: None }
    }

    /// How many bytes are left. Unit: bytes.
    #[must_use]
    pub const fn left(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    fn take(&mut self, want: usize) -> Option<&'b [u8]> {
        if self.fault.is_some() {
            return None;
        }
        if self.left() < want {
            self.fault = Some(Broken::Truncated { want, left: self.left() });
            return None;
        }
        let out = &self.bytes[self.at..self.at + want];
        self.at += want;
        Some(out)
    }

    /// One byte.
    pub fn u8(&mut self) -> u8 {
        self.take(1).map_or(0, |b| b[0])
    }

    /// A flag. Anything but zero and one is refused (R04): a byte nobody wrote
    /// is a file nobody produced.
    pub fn bool(&mut self) -> bool {
        match self.u8() {
            0 => false,
            1 => true,
            _ => {
                self.fault.get_or_insert(Broken::Bounds("a flag that is neither zero nor one"));
                false
            }
        }
    }

    /// Two bytes.
    pub fn u16(&mut self) -> u16 {
        self.take(2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
    }

    /// Four bytes.
    pub fn u32(&mut self) -> u32 {
        self.take(4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Four bytes, signed.
    pub fn i32(&mut self) -> i32 {
        self.u32() as i32
    }

    /// Eight bytes.
    pub fn u64(&mut self) -> u64 {
        self.take(8)
            .map_or(0, |b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// A count, checked against the bytes actually left.
    ///
    /// `each` is the smallest number of bytes one element can occupy. A count
    /// claiming more elements than the rest of the file could hold is refused
    /// *before* anything reserves memory for it, which is the difference between
    /// a corrupt file and an allocation nobody survives. R04.
    pub fn count(&mut self, each: usize, what: &'static str) -> usize {
        let count = self.u32() as usize;
        if self.fault.is_some() {
            return 0;
        }
        if count.saturating_mul(each.max(1)) > self.left() {
            self.fault = Some(Broken::Bounds(what));
            return 0;
        }
        count
    }

    /// Raw bytes, length first, borrowed rather than copied.
    pub fn chunk(&mut self) -> &'b [u8] {
        let len = self.count(1, "a byte string longer than the file");
        self.take(len).unwrap_or(&[])
    }

    /// Raw bytes, length first.
    pub fn blob(&mut self) -> Vec<u8> {
        self.chunk().to_vec()
    }

    /// A string, length first. Refused if it is not UTF-8.
    pub fn str(&mut self) -> String {
        let bytes = self.blob();
        match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                self.fault.get_or_insert(Broken::Bounds("a string that is not UTF-8"));
                String::new()
            }
        }
    }

    /// One of the labels in [`LABELS`], by its index.
    pub fn label(&mut self) -> &'static str {
        let id = self.u32();
        match LABELS.get(id as usize) {
            Some(label) => label,
            None => {
                self.fault.get_or_insert(Broken::Label(id));
                ""
            }
        }
    }

    /// A submission entry, in the order [`Writer::sqe`] wrote it.
    pub fn sqe(&mut self) -> Sqe {
        Sqe {
            opcode: self.u8(),
            flags: self.u8(),
            class: self.u16(),
            cap: self.u32(),
            user_data: self.u64(),
            deadline: self.u64(),
            offset: self.u64(),
            buf_set: self.u32(),
            buf_index: self.u32(),
            len: self.u32(),
            _reserved: self.u32(),
            ext: [self.u64(), self.u64()],
        }
    }

    /// A completion entry, in the order [`Writer::cqe`] wrote it.
    pub fn cqe(&mut self) -> Cqe {
        Cqe {
            user_data: self.u64(),
            result: self.i32(),
            flags: self.u32(),
            timestamp: self.u64(),
            ext: self.u64(),
        }
    }

    /// Note a refusal of the caller's own, so a loader that finds an
    /// inconsistency reports it the way a truncation is reported.
    pub fn refuse(&mut self, why: Broken) {
        self.fault.get_or_insert(why);
    }

    /// The first refusal, if there was one.
    #[must_use]
    pub fn fault(&self) -> Option<Broken> {
        self.fault.clone()
    }

    /// Has anything already gone wrong?
    ///
    /// Read by a loader that is about to do something expensive or fallible with
    /// a value a faulted read answered — rebuilding a registration table, most
    /// of all — so that a corrupt file is not replayed into a model.
    #[must_use]
    pub const fn faulted(&self) -> bool {
        self.fault.is_some()
    }

    /// Nothing went wrong and nothing is left over.
    ///
    /// # Errors
    ///
    /// The first refusal, or [`Broken::Trailing`] when bytes remain — a file
    /// this build read less of than it holds is a file it misunderstood. R04:
    /// refused rather than ignored.
    pub fn finish(self) -> Result<(), Broken> {
        match self.fault {
            Some(fault) => Err(fault),
            None if self.left() > 0 => Err(Broken::Trailing(self.left())),
            None => Ok(()),
        }
    }
}

/// What a snapshot says about itself before any of it is interpreted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Head {
    /// The trial this is a point inside.
    pub trial: Trial,
    /// The commit it was taken at; empty when the caller named none.
    pub commit: String,
    /// Messages delivered before the cut. Unit: steps.
    pub steps: u32,
    /// Interleaving decisions taken before the cut. Unit: decisions.
    pub decisions: u32,
    /// Where the clock stood. Unit: nanoseconds on the simulator's clock, whose
    /// zero is the start of the run.
    pub at_ns: u64,
    /// Records this file holds. Unit: records.
    ///
    /// Zero for a terse snapshot, which carries the artefact's running hash
    /// instead — see [`Head::whole`].
    pub records: u32,
    /// Does this snapshot carry the artefact, or only its running hash?
    ///
    /// A whole mark restores a run that is indistinguishable from the replay in
    /// every respect the oracle included; a terse one restores a run with the
    /// same digest and the same tail, and `check::examine` refuses to judge it.
    /// RFC 0043 measures what each costs.
    pub whole: bool,
    /// The bound the run was started under. Unit: steps.
    ///
    /// In the file rather than taken from `scenario::BUDGET` at restore, because
    /// a restore that silently raised a budget would turn a run that did not
    /// terminate into one that did — which is the one verdict a re-entry must
    /// not be able to invent.
    pub budget: u32,
    /// How large the whole file is. Unit: bytes.
    pub bytes: usize,
}

impl Head {
    /// Where the cut sits, in simulated minutes. Unit: minutes.
    #[must_use]
    pub const fn minutes(&self) -> u64 {
        self.at_ns / MINUTE_NS
    }

    /// One line, for a report.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{} seed {:#018x} — minute {}, step {}, decision {}, {} record(s) in the file, {} \
             bytes, {}",
            self.trial.scenario,
            self.trial.seed,
            self.minutes(),
            self.steps,
            self.decisions,
            self.records,
            self.bytes,
            if self.whole { "whole" } else { "terse" },
        )
    }
}

/// Write a paused simulation out.
///
/// # Errors
///
/// [`Broken::Unsaveable`] for an actor that has no snapshot — the refusal,
/// rather than a short file, that keeps a partly-taught crate honest — and
/// [`Broken::Unlabelled`] for a label outside [`LABELS`].
pub fn save(sim: &Simulation, trial: &Trial, commit: &str, terse: bool) -> Result<Vec<u8>, Broken> {
    let mut body = Writer::new();
    body.str(trial.scenario);
    body.u64(trial.seed);
    body.u32(trial.clients);
    body.u32(trial.window);
    body.u32(trial.operations);
    body.count(trial.injects.len());
    for injection in trial.injects {
        body.label(injection.class.label());
        body.u32(injection.after);
        body.u32(injection.one_in);
    }
    // Two facts that must agree: the plan the world is armed with, and the plan
    // the trial names. Only the trial's travels — `fault.rs` says why — so this
    // is where the assumption behind that choice is checked rather than trusted.
    if sim.world_ref().plan() != trial.injects {
        return Err(Broken::Diverged("the world's fault plan and the trial's"));
    }
    body.u32(sim.steps());
    body.u32(sim.budget());
    // The cut, stated in the header rather than only implied by the world below
    // it. Two reasons: `head` answers it without rebuilding anything, and the
    // world's own copy is then something to check the load against rather than
    // the only witness. R04 likes a second opinion.
    let world = sim.world_ref();
    body.u64(world.clock());
    body.u32(world.decided());
    // The records the file will actually hold, which is none in a terse one.
    // The count a *reader* wants — records so far in the run — is the trace's
    // own `carried`, and it is in the trace rather than duplicated here.
    let records = if terse { 0 } else { world.trace().len() };
    body.u32(records.try_into().unwrap_or(u32::MAX));
    body.bool(terse);
    world.save(&mut body, terse);
    body.count(sim.actors().len());
    for actor in sim.actors() {
        actor.save(&mut body)?;
    }
    let body = body.finish()?;

    let mut out = Writer::new();
    let mut bytes = Vec::from(*MAGIC);
    out.u32(FORMAT);
    out.u64(build());
    out.str(commit);
    out.blob(&body);
    bytes.extend_from_slice(&out.finish()?);
    let sum = checksum(&bytes);
    bytes.extend_from_slice(&sum.to_le_bytes());
    Ok(bytes)
}

/// Read a snapshot's header without rebuilding anything.
///
/// # Errors
///
/// Every refusal [`restore`] can give about the file itself: the magic, the
/// format, the build fingerprint, the commit and the checksum.
pub fn head(bytes: &[u8], commit: &str) -> Result<Head, Broken> {
    let (mut body, taken_at) = open(bytes, commit)?;
    header(&mut body, &taken_at, bytes.len())
}

/// Check the envelope and answer a reader over the body.
fn open<'b>(bytes: &'b [u8], commit: &str) -> Result<(Reader<'b>, String), Broken> {
    if bytes.len() > LIMIT {
        return Err(Broken::Bounds("a snapshot larger than this reader will consider"));
    }
    if bytes.len() < MAGIC.len() + 8 || &bytes[..MAGIC.len()] != MAGIC.as_slice() {
        return Err(Broken::NotASnapshot);
    }
    let (payload, tail) = bytes.split_at(bytes.len() - 8);
    let found = u64::from_le_bytes([
        tail[0], tail[1], tail[2], tail[3], tail[4], tail[5], tail[6], tail[7],
    ]);
    let want = checksum(payload);
    if found != want {
        return Err(Broken::Checksum { found, want });
    }

    let mut outer = Reader::new(&payload[MAGIC.len()..]);
    let format = outer.u32();
    if format != FORMAT {
        return Err(Broken::Format { found: format, want: FORMAT });
    }
    let stamp = outer.u64();
    if stamp != build() {
        return Err(Broken::Build { found: stamp, want: build() });
    }
    let taken_at = outer.str();
    if !commit.is_empty() && taken_at != commit {
        return Err(Broken::Commit { found: taken_at, want: commit.to_string() });
    }
    let body = outer.chunk();
    outer.finish()?;
    Ok((Reader::new(body), taken_at))
}

/// Read the part of the body that describes the run rather than its state.
fn header(body: &mut Reader<'_>, commit: &str, bytes: usize) -> Result<Head, Broken> {
    let name = body.str();
    let Some(scenario) = scenario::find(&name) else {
        return Err(Broken::NoSuchScenario(name));
    };
    let seed = body.u64();
    let clients = body.u32();
    let window = body.u32();
    let operations = body.u32();
    let armed = body.count(12, "more injections than the file could hold");
    let mut injects = Vec::with_capacity(armed);
    for _ in 0..armed {
        let label = body.label();
        let Some(class) = crate::sweep::class(label) else {
            body.refuse(Broken::Bounds("a fault class this build does not have"));
            break;
        };
        injects.push(Injection { class, after: body.u32(), one_in: body.u32() });
    }
    let steps = body.u32();
    let budget = body.u32();
    let at_ns = body.u64();
    let decisions = body.u32();
    let records = body.u32();
    let whole = !body.bool();
    if let Some(fault) = body.fault() {
        // Nothing below this point is worth attempting on a file that has
        // already contradicted itself, and the reader carries the reason.
        return Err(fault);
    }
    let trial = Trial {
        scenario: scenario.name,
        seed,
        clients,
        window,
        operations,
        // Leaked, exactly as `sweep::plan` leaks a minimiser's candidate and for
        // the same reason: `World::arm` takes a `'static` plan because a plan is
        // granted for the life of a run, and a restored run's life is the
        // process that restored it.
        //
        // The lifetime is therefore the *process*, not the head — one plan per
        // `head` or `restore` call, a few dozen bytes each, never freed. That is
        // right for `f-sim --resume`, which restores once and exits, and it is a
        // deliberate cost in the test suite, which restores several hundred
        // times in one process. Stated here rather than left to be discovered by
        // whoever first calls `restore` in a loop: the day a caller does, this
        // is what has to become an interned plan rather than a leak.
        injects: Box::leak(injects.into_boxed_slice()),
    };
    Ok(Head {
        trial,
        commit: commit.to_string(),
        steps,
        decisions,
        at_ns,
        records,
        whole,
        budget,
        bytes,
    })
}

/// Rebuild a paused simulation from a snapshot.
///
/// Self-sufficient: the file carries every actor's state and the fault plan, so
/// nothing is read from `target/component` and nothing is passed in. A restored
/// deployment run therefore does not need the build that produced it — which is
/// the point, because the component set is already in the artefact's header
/// where `--join` reads it.
///
/// # Errors
///
/// Every refusal in [`Broken`]. A file this build cannot interpret is refused
/// whole rather than interpreted partly.
pub fn restore(bytes: &[u8], commit: &str) -> Result<(Simulation, Head), Broken> {
    let (mut body, taken_at) = open(bytes, commit)?;
    let head = header(&mut body, &taken_at, bytes.len())?;
    let mut sim =
        Simulation::resume(World::load(&mut body, head.trial.seed), head.steps, head.budget);
    sim.world().arm(head.trial.injects);
    let world = sim.world_ref();
    if world.clock() != head.at_ns
        || world.decided() != head.decisions
        || world.trace().len() != head.records as usize
    {
        return Err(Broken::Diverged("the world's own clock, decision log and record count"));
    }
    let count = body.count(8, "more actors than the file could hold");
    for _ in 0..count {
        let actor = load_actor(&mut body)?;
        let _ = sim.install(actor);
    }
    body.finish()?;
    Ok((sim, head))
}

/// One actor, by the tag its blob begins with.
fn load_actor(body: &mut Reader<'_>) -> Result<Box<dyn crate::Actor>, Broken> {
    let kind = body.u32();
    let actor: Box<dyn crate::Actor> = match kind {
        tag::APP => Box::new(crate::client::App::load(body)),
        tag::CLIENT => Box::new(crate::actors::Client::load(body)),
        tag::SERVICE => Box::new(crate::actors::Service::load(body)),
        tag::BLK => Box::new(crate::dev::Device::load(crate::blk::Blk, body)),
        tag::NET => Box::new(crate::dev::Device::load(crate::net::Net, body)),
        tag::GPU => Box::new(crate::dev::Device::load(crate::gpu::Gpu::default(), body)),
        tag::NATIVE => Box::new(crate::native::Native::load(body)),
        other => return Err(Broken::Kind(other)),
    };
    Ok(actor)
}

/// One mark, as it is handed to whoever is keeping them.
pub struct Mark<'m> {
    /// The whole file.
    pub bytes: &'m [u8],
    /// Where the clock stood when it was taken. Unit: nanoseconds.
    pub at_ns: u64,
    /// Messages delivered before it. Unit: steps.
    pub steps: u32,
}

impl Mark<'_> {
    /// Which simulated minute this mark is the last state of. Unit: minutes.
    ///
    /// A mark placed at the minute-`n` boundary is taken *before* the first step
    /// due at or after it, so its clock is still inside minute `n - 1` — which
    /// is exactly the state the exit criterion asks to re-enter: *a failure at
    /// simulated minute 40 is re-entered at minute 39*.
    #[must_use]
    pub const fn minute(&self) -> u64 {
        self.at_ns / MINUTE_NS
    }
}

/// A run, marked every `every` nanoseconds of simulated time, to the end.
///
/// The shape a bisect actually wants, and the reason the saving is real rather
/// than notional: the marks are taken **during the one pass somebody was already
/// going to run**, so the expensive replay happens once and every later
/// investigation starts from the nearest mark. A tool that produced a snapshot
/// by replaying to a point would cost the replay it exists to avoid.
///
/// `keep` is handed each mark as it is made rather than a list at the end,
/// because a mark carries the whole artefact so far and holding forty of them is
/// holding forty copies of a growing run. What a caller does with one — write
/// it, count it, throw it away — is the caller's, and `f-sim --scan` writes it
/// to a file and drops it.
///
/// # Errors
///
/// [`Scanned`], which is the run's own refusal, a save's, or whatever `keep`
/// refused with.
pub fn scan(
    trial: &Trial,
    deployment: &Deployment,
    every: u64,
    commit: &str,
    terse: bool,
    keep: &mut dyn FnMut(&Mark<'_>) -> Result<(), Broken>,
) -> Result<Outcome, Scanned> {
    let step = every.max(1);
    let mut sim = trial.narrowed().start(trial.seed, deployment).map_err(Scanned::Ran)?;
    let mut next = step;
    loop {
        match sim.run_to(Cut::Clock(next)).map_err(Scanned::Ran)? {
            Halt::Finished(outcome) => return Ok(*outcome),
            Halt::Paused(paused) => {
                let paused = *paused;
                let bytes = save(&paused, trial, commit, terse).map_err(Scanned::Wrote)?;
                let at_ns = paused.world_ref().clock();
                let mark = Mark { bytes: &bytes, at_ns, steps: paused.steps() };
                keep(&mark).map_err(Scanned::Wrote)?;
                // Past the instant this one stopped at rather than one boundary
                // on, so a scenario that jumps a minute leaves one mark for the
                // minute it landed in and not one per boundary it flew over.
                next = paused
                    .next_ns()
                    .unwrap_or(u64::MAX)
                    .saturating_div(step)
                    .saturating_add(1)
                    .saturating_mul(step);
                sim = paused;
            }
        }
    }
}

/// Why a scan stopped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Scanned {
    /// The run itself refused.
    Ran(Trouble),
    /// A snapshot could not be written.
    Wrote(Broken),
}

impl Scanned {
    /// A sentence for a report.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Ran(trouble) => trouble.message(),
            Self::Wrote(broken) => broken.message(),
        }
    }
}

/// A pending message, as the timeline holds it.
///
/// Here rather than in `time.rs` because both ends of the format want one
/// function for the pair of an address and a [`Message`].
pub fn write_message(out: &mut Writer, to: ActorId, message: &Message) {
    out.u32(to.0);
    out.u32(message.from.0);
    out.label(message.kind);
    out.u64(message.token);
    out.u64(message.detail);
}

/// The same, read back.
pub fn read_message(input: &mut Reader<'_>) -> (ActorId, Message) {
    let to = ActorId(input.u32());
    let from = ActorId(input.u32());
    let kind = input.label();
    (to, Message { from, kind, token: input.u64(), detail: input.u64() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_SEED;
    use crate::client::App;
    use crate::deploy::fixture::component;

    /// The commit every test names, so that the commit check is exercised on
    /// every save rather than only by the test written for it.
    const AT: &str = "0000000000000000000000000000000000000000";

    /// The component set the deployment scenario runs over in tests.
    fn deployment() -> Deployment {
        Deployment::of(vec![component("virtio-blk", "blk", 256), component("store", "store", 16)])
            .expect("two names")
    }

    /// Every scenario a test runs, with the long one narrowed to something a
    /// test suite can afford.
    ///
    /// Narrowed by [`Trial`] rather than left out: the point of `soak` here is
    /// that it is the *same shape* as the run `cargo xtask snapshot` takes, and
    /// a table this test skipped would be a table whose actors nothing checked.
    fn trials(seed: u64) -> Vec<Trial> {
        let mut out: Vec<Trial> =
            scenario::SCENARIOS.iter().map(|scenario| Trial::of(scenario, seed)).collect();
        for scenario in scenario::LONG {
            out.push(Trial { operations: 40, ..Trial::of(scenario, seed) });
        }
        out
    }

    /// A run and everything about it two runs are compared by.
    fn whole(trial: &Trial, deployment: &Deployment) -> Outcome {
        trial.run(deployment).expect("a shipped scenario terminates")
    }

    #[test]
    fn a_restored_run_is_the_run_that_replayed() {
        // **The property.** Everything else in this module is machinery for it.
        //
        // For every scenario in both tables, at three seeds, cut at seven points
        // spread along the run: write the paused world out, read it back into a
        // fresh simulation, run that to the end, and require the result to be
        // the run that never stopped — record for record, decision for decision,
        // digest, finishing instant, step count and fault count.
        //
        // A field that did not travel and that matters at any of those cuts
        // moves one of those six. A field that did not travel and matters at
        // none of them is the residual RFC 0043 states and this cannot see.
        let deployment = deployment();
        for seed in [DEFAULT_SEED, 1, 0xFFFF_FFFF_FFFF_FFFF] {
            for trial in trials(seed) {
                let replayed = whole(&trial, &deployment);
                assert!(replayed.steps > 4, "{} is too short to cut", trial.scenario);
                for k in 1..=7u32 {
                    let at = replayed.steps.saturating_mul(k) / 8;

                    // A whole mark: the artefact travels, so *everything* is
                    // comparable, records included.
                    let restored = restore_at(&trial, &deployment, Cut::Steps(at), false);
                    let where_ = format!("{} seed {seed:#x} cut at step {at}", trial.scenario);
                    assert_eq!(
                        restored.trace.records(),
                        replayed.trace.records(),
                        "the artefact diverged — {where_}"
                    );
                    assert_eq!(restored.digest(), replayed.digest(), "the digest moved — {where_}");
                    assert_eq!(restored.log, replayed.log, "a decision moved — {where_}");
                    assert_eq!(
                        (restored.steps, restored.finished_ns, restored.injected),
                        (replayed.steps, replayed.finished_ns, replayed.injected),
                        "the run's shape moved — {where_}"
                    );

                    // A terse mark: the artefact does not travel, so what is
                    // comparable is the digest, the decisions, the shape — and
                    // the *tail*, which has to be the replayed run's own tail
                    // record for record. That last one is the property a running
                    // hash could otherwise hide: two different tails can be made
                    // to hash the same only by accident, and the records are
                    // checked so the accident is not what the test rests on.
                    let terse = restore_at(&trial, &deployment, Cut::Steps(at), true);
                    let carried = terse.trace.carried().expect("a terse restore carries a prefix");
                    let dropped = usize::try_from(carried.records).unwrap_or(usize::MAX);
                    assert_eq!(
                        terse.trace.records(),
                        &replayed.trace.records()[dropped..],
                        "the tail diverged — {where_}, terse"
                    );
                    assert_eq!(
                        terse.digest(),
                        replayed.digest(),
                        "a terse restore did not answer the whole run's digest — {where_}"
                    );
                    let skipped =
                        usize::try_from(terse.decisions).unwrap_or(usize::MAX) - terse.log.len();
                    assert_eq!(
                        terse.log,
                        replayed.log[skipped..],
                        "a decision moved — {where_}, terse"
                    );
                    assert_eq!(
                        terse.decisions, replayed.decisions,
                        "the decision count moved — {where_}, terse"
                    );
                    assert_eq!(
                        (terse.steps, terse.finished_ns, terse.injected),
                        (replayed.steps, replayed.finished_ns, replayed.injected),
                        "the run's shape moved — {where_}, terse"
                    );
                }
            }
        }
    }

    #[test]
    fn a_cut_in_simulated_time_re_enters_where_it_says() {
        // The exit criterion's own shape, at test scale: cut at an instant
        // rather than at a step, and require both that the header names the
        // instant and that the run from there is the run that replayed.
        let deployment = deployment();
        let trial = Trial { operations: 40, ..Trial::of(&scenario::LONG[0], DEFAULT_SEED) };
        let replayed = whole(&trial, &deployment);
        let half = replayed.finished_ns / 2;

        let sim = trial.narrowed().start(trial.seed, &deployment).expect("a scenario starts");
        let Halt::Paused(paused) = sim.run_to(Cut::Clock(half)).expect("it does not finish") else {
            panic!("a run half its length long did not pause");
        };
        let bytes = save(&paused, &trial, AT, false).expect("every actor saves");
        let (resumed, head) = restore(&bytes, AT).expect("this build reads its own snapshot");
        assert!(head.whole, "a snapshot taken whole did not say so");
        assert!(head.at_ns < half, "the cut was taken after the instant it was asked for");
        assert!(head.at_ns * 2 > half, "the cut was taken nowhere near the instant it was asked");
        let restored = resumed.run().expect("a restored run terminates");
        assert_eq!(restored.digest(), replayed.digest());
        assert_eq!(restored.trace.records(), replayed.trace.records());
    }

    /// Run to `cut`, write out, read back, and finish.
    ///
    /// A cut the run reaches the end before is a **panic** and not a fallback,
    /// and that is the difference between this test and a test that passes for
    /// the wrong reason: falling through to the finished outcome would compare a
    /// whole run against itself and report the comparison as green. Every caller
    /// here cuts strictly inside the run, so this cannot fire — which is exactly
    /// when a guard is worth writing, because the day somebody changes a
    /// scenario's length it can.
    fn restore_at(trial: &Trial, deployment: &Deployment, cut: Cut, terse: bool) -> Outcome {
        let sim = trial.narrowed().start(trial.seed, deployment).expect("a scenario starts");
        match sim.run_to(cut).expect("a shipped scenario terminates") {
            Halt::Finished(_) => {
                panic!("{} finished before the cut, so nothing was restored", trial.scenario)
            }
            Halt::Paused(paused) => {
                let bytes =
                    save(&paused, trial, AT, terse).expect("every actor in a scenario saves");
                let (resumed, _head) =
                    restore(&bytes, AT).expect("this build reads its own snapshot");
                resumed.run().expect("a restored run terminates")
            }
        }
    }

    #[test]
    fn a_terse_mark_is_smaller_than_a_whole_one_and_stays_the_same_size() {
        // The whole reason `--terse` exists: a whole mark grows with the run and
        // a terse one does not. Checked as *sizes at two cuts* rather than as a
        // ratio, because the ratio is a property of the scenario and the
        // direction is a property of the format.
        let deployment = deployment();
        let trial = Trial { operations: 400, ..Trial::of(&scenario::LONG[0], DEFAULT_SEED) };
        let replayed = whole(&trial, &deployment);
        let sizes = |at: u32| {
            let make = |terse: bool| {
                let sim =
                    trial.narrowed().start(trial.seed, &deployment).expect("a scenario starts");
                let Halt::Paused(paused) = sim.run_to(Cut::Steps(at)).expect("it does not finish")
                else {
                    panic!("a cut inside the run did not pause");
                };
                save(&paused, &trial, AT, terse).expect("every actor saves").len()
            };
            (make(false), make(true))
        };
        let (early_whole, early_terse) = sizes(replayed.steps / 4);
        let (late_whole, late_terse) = sizes(replayed.steps * 3 / 4);

        assert!(late_whole > early_whole * 2, "a whole mark did not grow with the run");
        // Not *equal*, and the difference is worth stating rather than rounding
        // away: a terse mark carries the live state, which breathes with how
        // much work is in flight at the cut. What it does not do is grow with
        // the *run*, and the band is what says so. The whole mark beside it
        // more than doubled over the same interval.
        assert!(
            late_terse < early_terse.saturating_mul(2),
            "a terse mark grew with the run: {early_terse} then {late_terse}"
        );
        // Eight, at four hundred operations. The factor is the run's length
        // divided by the live state, so it grows without bound as the run does:
        // `cargo xtask snapshot` measures it on the shipped `soak` and RFC 0043
        // quotes the number. Asserted small here so the test does not become a
        // claim about a machine.
        assert!(
            late_terse.saturating_mul(8) < late_whole,
            "a terse mark is not much smaller than a whole one: {late_terse} against {late_whole}"
        );
    }

    #[test]
    fn a_terse_run_is_refused_by_the_oracle_rather_than_judged() {
        // R04, at the one place a fast path could quietly answer a different
        // question. A partial artefact fails `balance` and `bound` for every
        // operation answered before the cut, and those would be findings about
        // the snapshot rather than about the system.
        let deployment = deployment();
        let trial = Trial::of(&scenario::SCENARIOS[0], DEFAULT_SEED);
        let replayed = whole(&trial, &deployment);
        assert!(!crate::check::examine(&Ok(replayed.clone())).failed(), "the whole run is clean");

        let terse = restore_at(&trial, &deployment, Cut::Steps(replayed.steps / 2), true);
        let verdict = crate::check::examine(&Ok(terse));
        assert_eq!(verdict.signature(), Some("partial"), "a terse run was judged");
    }

    /// A snapshot of `trial` taken half way through it.
    fn midway(trial: &Trial, deployment: &Deployment, terse: bool) -> Vec<u8> {
        let replayed = whole(trial, deployment);
        let sim = trial.narrowed().start(trial.seed, deployment).expect("a scenario starts");
        let Halt::Paused(paused) =
            sim.run_to(Cut::Steps(replayed.steps / 2)).expect("it does not finish")
        else {
            panic!("a run half its length long did not pause");
        };
        save(&paused, trial, AT, terse).expect("every actor saves")
    }

    #[test]
    fn a_snapshot_written_read_and_written_again_is_the_same_bytes() {
        // The other half of *no field was lost*, and the half that does not need
        // the field to matter: a load that dropped something would write it back
        // as a default, so the second file would differ from the first. Between
        // this and the trajectory test above, a field is caught either by
        // changing the run or by changing the bytes.
        let deployment = deployment();
        for trial in trials(DEFAULT_SEED) {
            for terse in [false, true] {
                let first = midway(&trial, &deployment, terse);
                let (resumed, head) =
                    restore(&first, AT).expect("this build reads its own snapshot");
                let second =
                    save(&resumed, &head.trial, AT, terse).expect("a restored world saves");
                assert_eq!(
                    first.len(),
                    second.len(),
                    "{} changed size, terse {terse}",
                    trial.scenario
                );
                assert!(first == second, "{} did not round-trip, terse {terse}", trial.scenario);
            }
        }
    }

    #[test]
    fn a_snapshot_from_another_build_is_refused_rather_than_read() {
        // R04. The fingerprint is eight bytes at a known offset; move one bit of
        // it, fix the checksum so that the checksum is not what refuses, and
        // require the refusal to name the build.
        let deployment = deployment();
        let mut bytes =
            midway(&Trial::of(&scenario::SCENARIOS[0], DEFAULT_SEED), &deployment, false);
        let at = MAGIC.len() + 4;
        bytes[at] ^= 0x01;
        reseal(&mut bytes);
        match head(&bytes, AT) {
            Err(Broken::Build { found, want }) => {
                assert_ne!(found, want);
                assert_eq!(want, build());
            }
            other => panic!("a snapshot from another build was not refused: {other:?}"),
        }
    }

    #[test]
    fn a_snapshot_from_another_commit_is_refused_rather_than_read() {
        let deployment = deployment();
        let bytes = midway(&Trial::of(&scenario::SCENARIOS[0], DEFAULT_SEED), &deployment, false);
        let elsewhere = "1111111111111111111111111111111111111111";
        match head(&bytes, elsewhere) {
            Err(Broken::Commit { found, want }) => {
                assert_eq!(found, AT);
                assert_eq!(want, elsewhere);
            }
            other => panic!("a snapshot from another commit was not refused: {other:?}"),
        }
        // And a caller who names no commit gets the file, because *no commit*
        // is a question not asked rather than a claim that any commit will do.
        // `cargo xtask snapshot` always names one; a test and a person poking at
        // a file do not have to.
        assert!(head(&bytes, "").is_ok());
    }

    #[test]
    fn a_file_that_is_not_a_snapshot_and_one_that_is_damaged_are_both_refused() {
        let deployment = deployment();
        let good = midway(&Trial::of(&scenario::SCENARIOS[0], DEFAULT_SEED), &deployment, false);
        assert_eq!(restore(b"", AT).err(), Some(Broken::NotASnapshot));
        assert_eq!(restore(b"not a snapshot at all", AT).err(), Some(Broken::NotASnapshot));

        // Truncated: the checksum is the first thing that notices, which is the
        // right answer — a short file is a damaged file.
        let short = &good[..good.len() - 16];
        assert!(matches!(restore(short, AT), Err(Broken::Checksum { .. })));

        // A byte flipped in the middle of the body.
        let mut bent = good.clone();
        let middle = bent.len() / 2;
        bent[middle] ^= 0xFF;
        assert!(matches!(restore(&bent, AT), Err(Broken::Checksum { .. })));

        // And the same flip with the checksum repaired, which is what an
        // adversary rather than a disk would produce: the reader must still
        // refuse or produce a world, and never a panic. Which of the two it is
        // depends on the byte, so this asserts the pair rather than one.
        let mut forged = bent.clone();
        reseal(&mut forged);
        let _ = restore(&forged, AT);
    }

    #[test]
    fn a_buffer_size_no_scenario_can_ask_for_is_refused_before_it_is_allocated() {
        // The failure this is written against, reproduced rather than argued: a
        // file claiming a client had four-gigabyte buffers reaches
        // `vec![0u8; buffer_bytes * BUFFERS]`, and an allocation nobody survives
        // is the process dying with a message about `alloc` rather than a
        // refusal naming a field. R04 says a value outside what its field can
        // mean is refused, and this is the field.
        //
        // Written through `App::save` rather than by patching a byte in a whole
        // snapshot, because `App::new` does not clamp a wide buffer — it takes
        // what a scenario asks for — so a save of one is exactly the file an
        // adversary would forge, produced by the same code that writes a good
        // one and therefore laid out the same way whatever fields get added
        // next.
        // Where each field sits is found rather than counted: two saves that
        // differ in nothing but one number differ in exactly one four-byte
        // window, and that window is the field. A test that hardcoded an offset
        // would keep passing while pointing at the wrong field the day somebody
        // adds one above it.
        let width_at = field_at(&saved_app(4, 512, 8), &saved_app(4, 1_024, 8));
        let window_at = field_at(&saved_app(4, 512, 8), &saved_app(5, 512, 8));
        let depth_at = field_at(&saved_app(4, 512, 8), &saved_app(4, 512, 9));

        // Every value that cannot mean anything, in the field it would arrive
        // in, and the refusal each one has to produce. Zero has to be *forged*
        // rather than saved for all three, because `App::new` clamps a zero for
        // a scenario that means *the smallest useful one* — and that a client
        // cannot hold a zero is exactly why a file claiming one is a file this
        // crate did not write.
        let forged: [(usize, &[u32], &str); 3] = [
            (
                width_at,
                &[0, crate::client::MAX_BUFFER_BYTES + 1, u32::MAX / 2, u32::MAX],
                "a buffer size no scenario can ask for",
            ),
            (
                window_at,
                &[0, crate::client::BUFFERS as u32 + 1, u32::MAX],
                "a window no client could have kept",
            ),
            (depth_at, &[0], "a client whose wire holds no submission"),
        ];
        for (at, values, why) in forged {
            for value in values {
                let mut bytes = saved_app(4, 512, 8);
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
                let mut input = Reader::new(&bytes);
                assert_eq!(input.u32(), tag::APP, "the tag `load_actor` would have eaten");
                let _ = App::load(&mut input);
                assert_eq!(
                    input.fault(),
                    Some(Broken::Bounds(why)),
                    "{value} was believed in the field at byte {at}"
                );
            }
        }

        // And the width every shipped scenario actually asks for is believed,
        // so that the bound is a bound on nonsense rather than on the models.
        for scenario in scenario::SCENARIOS.iter().chain(scenario::LONG) {
            if scenario.buffer_bytes == 0 {
                // A scenario with no client of this kind. `App::new` would clamp
                // it to one; a *file* saying zero is refused, which is the
                // asymmetry `App::load` argues for.
                continue;
            }
            let bytes = saved_app(scenario.window, scenario.buffer_bytes, scenario.depth);
            let mut input = Reader::new(&bytes);
            assert_eq!(input.u32(), tag::APP);
            let _ = App::load(&mut input);
            assert_eq!(
                input.fault(),
                None,
                "`{}` asks for {} byte buffers and a snapshot of it is refused — raise \
                 client::MAX_BUFFER_BYTES rather than narrowing the scenario",
                scenario.name,
                scenario.buffer_bytes
            );
        }
    }

    /// One client, written out the way a snapshot writes it — tag included, so
    /// the bytes are the ones `load_actor` sees.
    fn saved_app(window: u32, buffer_bytes: u32, depth: u32) -> Vec<u8> {
        let app = App::new(0, ActorId(1), window, 8, buffer_bytes, 1_000, depth);
        let mut out = Writer::new();
        app.save(&mut out);
        out.finish().expect("an app writes no labels")
    }

    /// The one four-byte window two otherwise identical saves differ in.
    ///
    /// Panics if they differ anywhere else, which is the assertion that makes
    /// this a way of *finding* a field rather than a way of guessing one.
    fn field_at(a: &[u8], b: &[u8]) -> usize {
        assert_eq!(a.len(), b.len(), "two clients wrote two different lengths");
        let at = (0..a.len()).find(|&k| a[k] != b[k]).expect("two values write two files");
        assert_eq!(&a[..at], &b[..at]);
        assert_eq!(&a[at + 4..], &b[at + 4..], "more than one field moved");
        at
    }

    #[test]
    fn a_domain_or_a_wire_of_no_size_is_refused_rather_than_repaired() {
        // Two fields that used to be clamped with `.max(1)` on the way in.
        // A clamp is a repair, and a repaired file restores into a world that is
        // *plausible* and is not the world the file described — which is the one
        // failure mode RFC 0043 says a snapshot must not have. Worse for
        // `Grants`: `Service::load` rebuilds a table and compares it against the
        // domain the file recorded beside it, and a clamp applied to both sides
        // makes that second opinion agree with itself.
        let mut out = Writer::new();
        out.u32(0); // room
        out.u64(0); // made
        out.bool(false); // starved
        out.count(0); // no translations
        let bytes = out.finish().expect("no labels");
        let mut input = Reader::new(&bytes);
        let _ = crate::service::Grants::load(&mut input);
        assert_eq!(input.fault(), Some(Broken::Bounds("a domain with no room for a translation")));

        // The same, for the two wires that used to clamp their depth. Both
        // loaders read the depth first, the tag having been eaten by
        // `load_actor`, so a single zero is the whole forgery.
        let mut out = Writer::new();
        out.u32(0);
        let bytes = out.finish().expect("no labels");
        let mut input = Reader::new(&bytes);
        let _ = crate::native::Native::load(&mut input);
        assert_eq!(input.fault(), Some(Broken::Bounds("a peer that would hold no submission")));

        let mut input = Reader::new(&bytes);
        let _ = crate::actors::Service::load(&mut input);
        assert_eq!(input.fault(), Some(Broken::Bounds("a service that would hold no operation")));
    }

    /// Put the checksum back after editing a snapshot's bytes.
    fn reseal(bytes: &mut [u8]) {
        let split = bytes.len() - 8;
        let sum = checksum(&bytes[..split]).to_le_bytes();
        bytes[split..].copy_from_slice(&sum);
    }

    #[test]
    fn an_actor_with_no_snapshot_refuses_by_name() {
        // The default on `Actor::save`, which is what stops a partly-taught
        // crate writing a file that loads into a world missing a participant.
        struct Mute;
        impl crate::Actor for Mute {
            fn name(&self) -> &'static str {
                "mute"
            }
            fn deliver(&mut self, _w: &mut World, _me: ActorId, _m: Message) {}
        }
        let mut sim = Simulation::new(1, 8);
        let _ = sim.install(Box::new(Mute));
        let trial = Trial::of(&scenario::SCENARIOS[0], 1);
        assert_eq!(save(&sim, &trial, AT, false), Err(Broken::Unsaveable("mute")));
    }

    #[test]
    fn every_label_a_run_can_write_is_in_the_table() {
        // The check that `LABELS` is the whole set rather than the set somebody
        // remembered. It runs every scenario in both tables at four seeds and
        // holds every label the run produced — in a record, in a message still
        // due, and at a decision site — against the table.
        //
        // A save already refuses an unknown label by name, so the failure this
        // prevents is a *scenario nobody snapshotted* rather than a silent
        // corruption. It is here because that scenario is the one that gets
        // added next year.
        let deployment = deployment();
        for seed in [DEFAULT_SEED, 1, 2, 3] {
            for trial in trials(seed) {
                let outcome = whole(&trial, &deployment);
                for record in outcome.trace.records() {
                    assert!(
                        index().contains_key(record.actor),
                        "actor label `{}` is not in snap::LABELS",
                        record.actor
                    );
                    assert!(
                        index().contains_key(record.kind),
                        "record kind `{}` is not in snap::LABELS",
                        record.kind
                    );
                }
                for decision in &outcome.log {
                    assert!(
                        index().contains_key(decision.site),
                        "decision site `{}` is not in snap::LABELS",
                        decision.site
                    );
                }
            }
        }
    }

    #[test]
    fn no_label_is_in_the_table_twice() {
        // Two entries for one string would mean two ids answering to it, a save
        // that picked one and a reader that could not tell them apart. Harmless
        // today because both ids read back as the same `&'static str`, and worth
        // refusing anyway: the day a label is renamed, one of the two would move
        // and the other would not.
        let mut sorted = LABELS.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "a label appears twice in snap::LABELS");
        assert_eq!(index().len(), LABELS.len());
    }

    #[test]
    fn the_build_fingerprint_moves_when_the_build_does() {
        // Not a test of `build` — it has no arguments — but of the *inputs* it
        // is built from being the ones claimed. Each of these is a change a
        // reader would expect to invalidate a snapshot.
        let stable = build();
        assert_eq!(stable, build(), "the fingerprint is not stable within a process");
        assert_ne!(stable, checksum(b""), "the fingerprint folded nothing");
        // Every scenario reaches it, which is what makes *a scenario's numbers
        // changed* refuse an old file.
        let all = {
            let mut text = String::new();
            for scenario in scenario::SCENARIOS.iter().chain(scenario::LONG) {
                text.push_str(&fingerprint(scenario));
            }
            text
        };
        for scenario in scenario::SCENARIOS.iter().chain(scenario::LONG) {
            assert!(all.contains(scenario.name), "{} is not fingerprinted", scenario.name);
        }
    }

    #[test]
    fn a_scan_marks_a_run_without_changing_it() {
        // The driver `cargo xtask snapshot` uses. Marks are taken during the
        // pass rather than by replaying to each of them, so the outcome a scan
        // answers has to be the outcome the plain run has — otherwise the
        // cheapest thing about this whole module would also be the thing that
        // changed the run.
        let deployment = deployment();
        let trial = Trial { operations: 60, ..Trial::of(&scenario::LONG[0], DEFAULT_SEED) };
        let replayed = whole(&trial, &deployment);
        let every = replayed.finished_ns / 4;
        let mut marks: Vec<Vec<u8>> = Vec::new();
        let scanned = scan(&trial, &deployment, every, AT, false, &mut |mark| {
            marks.push(mark.bytes.to_vec());
            Ok(())
        })
        .expect("a scan finishes");
        assert_eq!(scanned.digest(), replayed.digest(), "a scan changed the run it marked");
        assert!(marks.len() >= 3, "a run cut into quarters left {} mark(s)", marks.len());

        let mut last = 0;
        for mark in &marks {
            let head = head(mark, AT).expect("a mark this process wrote");
            assert!(head.at_ns >= last, "marks came out of order");
            last = head.at_ns;
            assert_eq!(head.trial.scenario, trial.scenario);
        }

        // And the last mark re-enters into the same ending.
        let (resumed, _) = restore(marks.last().expect("at least one mark"), AT).expect("readable");
        assert_eq!(resumed.run().expect("terminates").digest(), replayed.digest());
    }
}
