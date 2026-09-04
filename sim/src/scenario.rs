// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A scenario is data, and a run of one is an artefact.
//!
//! # Why the set is a table and not a directory of files
//!
//! A scenario is a handful of integers. Putting them in TOML would buy a parser,
//! a schema, a lint for the schema and a failure mode where a scenario is
//! misspelled rather than missing — and it would buy nothing else, because
//! nothing outside this repository writes one yet. `xtask` already keeps its
//! provocation sets this way (`PROVOCATIONS`, `ESCAPES`, `DMA_PROVOCATIONS`),
//! and the reason is the same in all four places: a table in the language is
//! data a compiler checks.
//!
//! *Reversal:* `E1-R01` publishes the simulator as a tool a third party runs
//! against their own checkout, and the moment somebody outside this tree wants a
//! scenario of their own, this becomes a file format. RFC 0030 is the argument
//! for what that file format should then be — compiled, not parsed at the point
//! of use.
//!
//! # Every field is read
//!
//! A scenario with a knob nothing turns is a scenario that lies about what it
//! varies. [`tests::every_field_changes_the_run`] holds each field against the
//! run and requires the digest to move, which is what stops this table growing
//! settings that were true once.

use crate::actors::{Client, Service};
use crate::client::App;
use crate::deploy::Deployment;
use crate::dev::{Config, Device};
use crate::fault::{Class, Injection};
use crate::native::Native;
use crate::{ActorId, Message, Outcome, Simulation, Trouble};

/// How many messages a scenario may deliver before it is called stuck.
///
/// A bound on messages rather than on simulated time, because a scenario that
/// fails to terminate usually does so by exchanging messages at one instant.
/// Generous against what the scenarios below need — the largest of them is three
/// orders of magnitude under it — because the number is here to catch a model
/// that loops, not to bound a model that works.
pub const BUDGET: u32 = 1_000_000;

/// What every artefact says about itself, before a single actor is installed.
///
/// RFC 0032 decided that this simulator runs the system above the frame, and an
/// exit criterion answered by two commands owes every artefact the sentence
/// saying which of the two it is. So the coverage is in the hashed bytes rather
/// than in a document beside them: a trace quoted in a year says what it
/// covered, and a reader who wants the other half is told the command that
/// produces it.
const COVERS: &[&str] = &[
    "f-sim artefact 1 — a run of the system above the frame",
    "covers      components, the rings between them, and the devices at the far end",
    "not covered the frame's own instructions. `cargo xtask trace --hash` boots the real",
    "            kernel and hashes that; it is the other half of (seed, commit). RFC 0032",
];

/// What the client is pointed at.
///
/// **This is where component substitution is visible.** One client
/// ([`crate::client::App`]) and four things it can talk to, chosen here and
/// nowhere else: the client's source does not mention any of them and does not
/// branch on which it has. `docs/design/proving-ground.html` is where the
/// property comes from — *hardware already sits behind a component boundary, so
/// a simulated device is a component substitution rather than a kernel patch* —
/// and putting the choice in the scenario table rather than in the client is
/// what makes that sentence checkable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Peer {
    /// The bounded queue stage one shipped, with [`crate::actors::Client`]
    /// above it. Not part of the substitution claim: it speaks a protocol of
    /// opaque operations rather than the ring's, which is what makes it the
    /// machinery demonstration it was written as.
    Queue,
    /// A block device behind a virtqueue.
    Blk,
    /// A network interface behind a virtqueue.
    Net,
    /// A display controller behind a virtqueue.
    Gpu,
    /// A component with no device under it at all: `f_ring::registry` and a
    /// service time.
    Native,
    /// **The seam.** Not one peer but a set of them, read from the compiled
    /// manifest records the loader is handed as boot modules — one actor per
    /// component, each modelled according to the protocol its own record
    /// declares.
    ///
    /// This is the variant that makes `boot-to-workload` a pair of runs over one
    /// component set rather than two commands that happen to be in one
    /// paragraph. RFC 0035 argues it; [`crate::deploy`] reads the records;
    /// `cargo xtask sim --join` is what requires the set this scenario ran to be
    /// the set the boot spawned.
    Deployment,
}

impl Peer {
    /// A word for the artefact's header. Unit: none — a stable label.
    ///
    /// The same word each model writes into the trace, so that a header line
    /// saying *modelled as blk* and a record line written by the block device
    /// use one name.
    /// [`tests::a_peers_label_is_the_name_its_model_writes_into_the_trace`] is
    /// what keeps the two equal; a literal is used here rather than the
    /// associated constant so that this file does not have to import the
    /// protocol trait to name a peer.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Blk => "blk",
            Self::Net => "net",
            Self::Gpu => "gpu",
            Self::Native => "native",
            Self::Deployment => "set",
        }
    }
}

/// How many translations one peer's domain holds. Unit: translations.
///
/// Two, which is one more than any scenario here needs — each client registers
/// exactly one set with its peer. The spare is so that a domain running out is a
/// scenario away rather than a code change; the refusal itself is exercised in
/// `service.rs`, which is where the arithmetic lives.
const DOMAIN: u32 = 2;

/// One run's worth of configuration.
///
/// Small and flat on purpose: everything here is a number a person reading a
/// failing seed's reproduction command has to be able to hold in their head.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scenario {
    /// What it is called on the command line. Unit: none.
    pub name: &'static str,
    /// One line about what it is for. Unit: none.
    pub what: &'static str,
    /// What the client is pointed at. Unit: none.
    pub peer: Peer,
    /// How many clients submit. Unit: clients.
    pub clients: u32,
    /// How many operations each client keeps outstanding. Unit: operations.
    pub window: u32,
    /// How many operations the service will hold at once. Unit: operations.
    /// Below `clients * window` and the refusal path runs.
    pub depth: u32,
    /// How many operations each client issues. Unit: operations.
    pub operations: u32,
    /// The shortest the service takes over one operation. Unit: nanoseconds.
    pub service_ns: u64,
    /// How much longer than that it may take. Unit: nanoseconds; zero is a
    /// constant service time, which is a legitimate scenario and the one that
    /// isolates ordering from timing.
    pub spread_ns: u64,
    /// How long a refused client waits before submitting again. Unit:
    /// nanoseconds.
    pub retry_ns: u64,
    /// Bytes in each buffer of the client's registered set. Unit: bytes.
    /// Ignored by [`Peer::Queue`], which has no buffers.
    pub buffer_bytes: u32,
    /// How large the device is. Unit: per-peer — sectors for [`Peer::Blk`],
    /// bytes of frame for [`Peer::Net`] where zero is a link that is down,
    /// resources for [`Peer::Gpu`] — and the peer that reads it is what states
    /// which. Ignored by [`Peer::Queue`] and [`Peer::Native`].
    pub extent: u64,
    /// One completion in this many may be lost, chosen by the seed. Unit:
    /// completions; zero is never.
    ///
    /// A device that loses one resets, because RFC 0024 gives a client no way to
    /// take a buffer back except on evidence that its peer is gone. Ignored by
    /// [`Peer::Queue`] and [`Peer::Native`].
    pub lose_one_in: u32,
    /// What this scenario breaks, and how often. Unit: see
    /// [`Injection`](crate::fault::Injection). Empty is a scenario that breaks
    /// nothing, which is every scenario that shipped before `E1-P02`.
    ///
    /// A field rather than a flag on the command line, so that a reproduction
    /// command stays a scenario's name and a seed. A fault plan reachable only
    /// from `argv` would make a failing seed an incomplete bug report, which is
    /// the one thing this apparatus is built not to produce — `fault.rs` argues
    /// it, RFC 0039 records it, and `E1-P03` is the task that would have paid
    /// for getting it wrong.
    pub injects: &'static [Injection],
    /// Whether this scenario's first client carries the hard class with a
    /// deadline, and its peer orders its queue by what
    /// `f_abi::deadline::inherit` returned rather than by arrival.
    ///
    /// `E1-B06` and RFC 0049. One field and not two, and that is a choice with
    /// a cost: a control run turns *both* halves off, so `deadline` against
    /// itself-with-this-false compares a run where one client is urgent and the
    /// queue orders against a run where nobody is and it does not. That is
    /// still a control — under it every client is symmetric, so the first
    /// client's mean position is the mean position and any advantage it shows
    /// is the ordering — and the sharp version, where only the *queue* changes,
    /// is the boot's: `cargo xtask deadline` runs the identical burst against
    /// both orders. What the model is for is that a sweep can reach the rule at
    /// all.
    pub deadlines: bool,
}

/// The scenario set.
///
/// Each member exists to make a different part of the machinery decide
/// something, and no count is written here because a count in a comment beside
/// a list is a number that stops matching the list. A set where every member
/// exercises the same path is a set of one wearing several names — which is why
/// `net` and `netdrop` are two entries: one link that carries and one that does
/// not, because for a while the delivery path had no scenario at all.
pub const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "handshake",
        what: "two clients, a service that never refuses and never varies — ordering alone",
        peer: Peer::Queue,
        clients: 2,
        window: 1,
        depth: 4,
        operations: 6,
        service_ns: 1_000,
        // Zero: with no variation in service time, every difference between two
        // seeds is a difference in *order*. If this scenario stopped responding
        // to its seed, the ordering machinery would have stopped working and
        // nothing else would say so.
        spread_ns: 0,
        retry_ns: 2_000,
        buffer_bytes: 0,
        extent: 0,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "contention",
        what: "four clients against a queue too shallow for them — the refusal path",
        peer: Peer::Queue,
        clients: 4,
        window: 2,
        // Two against eight outstanding: the queue is full often, so `full` and
        // the back-off are on the ordinary path rather than in a corner.
        depth: 2,
        operations: 8,
        service_ns: 500,
        spread_ns: 1_500,
        retry_ns: 3_000,
        buffer_bytes: 0,
        extent: 0,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "pipeline",
        what: "three clients keeping a deep queue busy — completion order under load",
        peer: Peer::Queue,
        clients: 3,
        window: 8,
        depth: 32,
        operations: 24,
        service_ns: 200,
        spread_ns: 800,
        retry_ns: 1_000,
        buffer_bytes: 0,
        extent: 0,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "blk",
        what: "a disk behind a virtqueue — reordered completions and coalesced notifications",
        peer: Peer::Blk,
        clients: 2,
        // Four outstanding against one disk, which is what gives the device
        // something to reorder: with a window of one there is never more than
        // one candidate and `blk.complete` records nothing at all.
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        // Sectors. Larger than any request this scenario makes, so the disk's
        // own refusal stays out of the way and what varies is the ordering.
        extent: 4_096,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "blkfull",
        what: "a disk too shallow for its client — both refusals and the back-off between them",
        peer: Peer::Blk,
        clients: 1,
        window: 8,
        // One, against a window of eight: the ring fills, the device fills, and
        // the client meets both. The two are distinguishable at the client —
        // the ring's arrives as `RingError::Full` before anything leaves, the
        // device's arrives as a completion carrying `RESOURCE/DEVICE_FULL`.
        depth: 1,
        operations: 10,
        service_ns: 900,
        spread_ns: 200,
        retry_ns: 1_500,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "blkloss",
        what: "a disk that loses a completion, resets, and leaves its client buffers to reclaim",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 16,
        service_ns: 300,
        spread_ns: 300,
        retry_ns: 1_000,
        buffer_bytes: 512,
        extent: 4_096,
        // One in three. Frequent enough that the reset happens early in every
        // seed rather than in a corner of some of them — a fault path that runs
        // in one seed of fifty is a fault path a sweep pays for and rarely
        // gets.
        lose_one_in: 3,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "net",
        what: "a link that carries every frame — the delivery path, which is not the quiet one",
        peer: Peer::Net,
        clients: 2,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 200,
        spread_ns: 400,
        retry_ns: 2_000,
        buffer_bytes: 1_024,
        // Bytes of frame, header included, and comfortably above the 1 036 a
        // full buffer needs. This scenario exists because the shipped one did
        // not: `net` used to be the dropping scenario, so `Net::serve`'s
        // delivery path ran in a unit test and in no scenario at all, and a
        // regression that broke frame delivery would have moved no digest.
        // `netdrop` below is the case that was here, kept and named.
        extent: 4_096,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "netdrop",
        what: "a link every frame is too long for, dropping them where nothing can say so",
        peer: Peer::Net,
        clients: 2,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 200,
        spread_ns: 400,
        retry_ns: 2_000,
        buffer_bytes: 1_024,
        // Bytes of frame, header included. Below the 1 036 a full buffer needs,
        // so every frame is too long for the link and every one is dropped —
        // and the client is told nothing, which is the whole point of this
        // device. `net.rs` is where that is argued.
        extent: 512,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "gpu",
        what: "a display with two resources, fenced creations, and transfers that may overtake",
        peer: Peer::Gpu,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 250,
        spread_ns: 500,
        retry_ns: 2_000,
        buffer_bytes: 256,
        // Resources. Two, against six creations: the display fills, the
        // creations after it are refused, and the transfers that depended on
        // them are refused for a reason the first refusal caused — which is the
        // shape of failure a device model is for.
        extent: 2,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "deployment",
        what: "every compiled component the build produced, one client each — the seam's half",
        peer: Peer::Deployment,
        // One client per component, and the *count* comes from the deployment
        // rather than from here: a virtqueue has one driver, and how many
        // components there are is the manifest set's answer and not a
        // scenario's. The field is read — `install_deployment` clamps nothing
        // by it — and it is one because that is what a component gets.
        clients: 1,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        // Sectors, for whichever components declare a disk. Larger than
        // anything this scenario asks for, so what the run exercises is the
        // component set and the ordering rather than a device's own refusal —
        // the refusals have scenarios of their own above.
        extent: 4_096,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    Scenario {
        name: "native",
        what: "the same client against a component with no device — the substitution's other half",
        peer: Peer::Native,
        clients: 2,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 0,
        lose_one_in: 0,
        injects: &[],
        deadlines: false,
    },
    // ---- E1-P02: one scenario per fault class ------------------------------
    //
    // Seven entries, one per class in [`crate::fault::Class`], and the pairing
    // is checked rather than trusted — `fault::tests::every_class_has_a_scenario`
    // holds the set against this table. Each one names its class in `what`, so a
    // reader of `--list` sees the seven classes rather than seven scenario names
    // they have to look up.
    //
    // All seven point at [`Peer::Blk`] and none of them is arbitrary about it.
    // The block device is the detailed model — a three-descriptor chain, a
    // status byte the driver writes `0xFF` into first, and an address decode —
    // and four of the seven classes are only observable through one of those.
    // A class whose response is asserted against a queue with a delay would be a
    // class asserted against the harness.
    Scenario {
        name: "alloc",
        what: "allocation failure — the frame refuses a component the memory it registered for",
        peer: Peer::Blk,
        // Two, and this is the field the assertion turns on: an allocation
        // failure is *one component's*, so the run has to contain a component it
        // did not happen to. One client would make "contained" unfalsifiable.
        clients: 2,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        // The second registration, whichever client's that turns out to be —
        // the seed chooses which of the two channels goes first, so the class is
        // aimed at an occurrence and not at a component. `after: 0` would refuse
        // whoever happened to be first, which is a scenario about the timeline.
        injects: &[Injection { class: Class::Alloc, after: 1, one_in: 1 }],
        deadlines: false,
    },
    Scenario {
        name: "mapfault",
        what: "translation fault — a descriptor the device's domain declines to translate",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        // Larger than anything asked for, so the only refusal in this run is the
        // injected one. A disk that also refused on its own terms would make the
        // count the assertion rests on ambiguous.
        extent: 4_096,
        lose_one_in: 0,
        // One in two rather than every one, so the run contains both answers:
        // requests whose translation held completed, and the rest were refused.
        // A scenario where everything faults cannot show that the refusal was
        // confined to the requests it struck.
        injects: &[Injection { class: Class::MapFault, after: 1, one_in: 2 }],
        deadlines: false,
    },
    Scenario {
        name: "faultin",
        what: "device page-fault latency — a translation that was there and took far longer",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 10,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        // Every transfer, because the assertion is that the run finished later
        // and nothing else moved — and *later* has to be a consequence of the
        // class rather than of which requests a seed happened to strike.
        injects: &[Injection { class: Class::FaultIn, after: 0, one_in: 1 }],
        deadlines: false,
    },
    Scenario {
        name: "peergone",
        what: "peer death mid-operation — a device that stops with work still outstanding",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        // The fourth completion the device was about to publish, so that the
        // word *mid-operation* means something: a peer that died on its first is
        // a peer that died with nothing out, and the buffers-come-home assertion
        // would hold over an empty set.
        injects: &[Injection { class: Class::PeerGone, after: 3, one_in: 1 }],
        deadlines: false,
    },
    Scenario {
        name: "doorbell",
        what: "torn doorbell — a bell rung with no entry behind it, half the pair of stores",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        // Deep enough that nothing is ever refused: the assertion is exactly-once
        // over the tokens, and a retry re-submits a token, so a run with
        // back-pressure in it would be a run where a repeat is legitimate.
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        injects: &[Injection { class: Class::Doorbell, after: 0, one_in: 2 }],
        deadlines: false,
    },
    Scenario {
        name: "partial",
        what: "partial write — the payload landed and the device's status byte did not",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 12,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        // One in two, for `mapfault`'s reason: the run has to hold transfers
        // that completed beside the ones that were torn, or the assertion cannot
        // tell a class that refuses what it struck from a device that is off.
        injects: &[Injection { class: Class::Partial, after: 1, one_in: 2 }],
        deadlines: false,
    },
    Scenario {
        name: "latecqe",
        what: "delayed completion — the device finished and the driver was told late",
        peer: Peer::Blk,
        clients: 1,
        window: 4,
        depth: 8,
        operations: 10,
        service_ns: 400,
        spread_ns: 600,
        retry_ns: 2_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        // Every completion, for `faultin`'s reason. The two together are the
        // pair `E1-P06`'s exit will quote — *no client observes anything except
        // added latency* — approached from before the work and from after it.
        injects: &[Injection { class: Class::LateCqe, after: 0, one_in: 1 }],
        deadlines: false,
    },
    Scenario {
        name: "deadline",
        what: "batch reads queued behind a hard-class one — a device queue ordered by deadline",
        peer: Peer::Blk,
        // One, and it is this model's shape rather than a preference: a
        // virtqueue has exactly one driver, so `install_peers` gives every
        // client a device of its own and two clients never share a queue. The
        // contention an ordering needs therefore comes from one client's own mix
        // of urgent and batch work, which is also what RFC 0025's inversion is
        // made of.
        clients: 1,
        // The whole buffer set, so the client keeps eight requests out at once
        // and there is something for an urgent one to overtake.
        window: 8,
        // One. The device holds a single chain at a time, so seven of the eight
        // outstanding requests wait in the *driver's* queue whenever the device
        // is busy — and the driver's queue is the only place an order can exist,
        // because a virtqueue is consumed in the order the driver posts. It is
        // also `f_virtio_blk::pending::IN_FLIGHT`, which is the real driver's
        // depth and the granularity of every overtake it performs.
        depth: 1,
        operations: 24,
        service_ns: 400,
        // Non-zero, so the seed moves the timing as well as the interleaving:
        // an ordering that only holds when every request takes the same time is
        // an ordering a sweep should be able to break.
        spread_ns: 200,
        retry_ns: 1_000,
        buffer_bytes: 512,
        extent: 4_096,
        lose_one_in: 0,
        injects: &[],
        deadlines: true,
    },
];

/// The scenarios that are too long to sweep.
///
/// # Why a second table rather than a flag on the first
///
/// Because [`SCENARIOS`] is not just a list — it *is* the sweep's grid, the
/// determinism check's list, and the header `sim/corpus.txt` is regenerated
/// from. A scenario that runs for forty simulated minutes belongs in none of
/// those: `cargo xtask sweep` would multiply its cost by sixty-four seeds and
/// `cargo xtask sim` would run it three times per commit, and the thing it is
/// for — proving that a run of a hundred thousand steps can be re-entered near
/// its end — is answered once by `cargo xtask snapshot` rather than repeatedly
/// by every command that iterates the table.
///
/// So the split is by *cost*, which is the only property that distinguishes
/// them, and it is stated here rather than left as a name somebody has to
/// recognise. Both tables are found by [`find`], both are fingerprinted into
/// `snap::build`, and a member of either is a legitimate `--seed` and
/// `--trace` argument. What only [`SCENARIOS`] is, is *swept*.
///
/// *Reversal:* the day the sweep can afford a long scenario — because a machine
/// got faster or because sharding got finer — this table folds back into the
/// one above and `sweep::Sweep::span` stops taking a count.
pub const LONG: &[Scenario] = &[Scenario {
    name: "soak",
    what: "an hour of disk under a steady client — the scenario a bisect is for",
    peer: Peer::Blk,
    clients: 1,
    window: 4,
    depth: 8,
    // Long enough that a full replay is seconds of wall clock and the run has a
    // minute forty. Both halves matter: a scenario that reached minute forty in
    // a hundred steps would meet the exit's words and none of its point, because
    // *bisects in seconds rather than hours* is a claim about work avoided.
    operations: 120_000,
    // Sixty milliseconds a request, which is a slow disk and a deliberate one:
    // the run's simulated length is about `operations * service_ns / window`, so
    // this is what puts minute forty inside a run whose step count a laptop can
    // still finish. `claims/0007` is where the two numbers this produces are
    // registered.
    service_ns: 60_000_000,
    spread_ns: 30_000_000,
    retry_ns: 100_000_000,
    buffer_bytes: 512,
    extent: 4_096,
    lose_one_in: 0,
    injects: &[],
    // The long run is about re-entering a snapshot near its end, and adding an
    // ordering to it would change what is being re-entered without changing
    // what is being asked. `deadline` in `SCENARIOS` is where the ordering is
    // swept.
    deadlines: false,
}];

/// Find a scenario by name, in either table.
///
/// [`SCENARIOS`] first, so that a name in both resolves to the swept one — which
/// cannot happen, because [`tests::no_scenario_is_in_both_tables`] refuses it,
/// and the ordering is here so that the day somebody defeats that test the
/// answer is the conservative one.
#[must_use]
pub fn find(name: &str) -> Option<&'static Scenario> {
    SCENARIOS.iter().chain(LONG).find(|scenario| scenario.name == name)
}

impl Scenario {
    /// Does this scenario's component set come from compiled manifest records?
    ///
    /// A property of the scenario rather than of its name, so that a second
    /// deployment scenario is handled the day somebody writes one, and so that
    /// the binary decides whether to read files by asking rather than by
    /// matching a string.
    #[must_use]
    pub const fn needs_components(&self) -> bool {
        matches!(self.peer, Peer::Deployment)
    }

    /// Run this scenario at `seed`.
    ///
    /// # Errors
    ///
    /// [`Trouble`], if the run does not finish or a message names an actor that
    /// does not exist. Both are refusals rather than partial results: a
    /// truncated trace hashes to something, and something is worse than nothing
    /// here.
    pub fn run(&self, seed: u64) -> Result<Outcome, Trouble> {
        self.run_on(seed, &Deployment::default())
    }

    /// Run this scenario at `seed`, over a component set read from the compiled
    /// manifest records.
    ///
    /// The deployment is a parameter rather than a field of the scenario table
    /// because a table is data a compiler checks and a deployment is an artefact
    /// a build produced. Keeping the file reading in `main.rs` and out of here
    /// is what lets every other scenario stay a pure function of `(seed,
    /// commit)` with nothing under it.
    ///
    /// # Errors
    ///
    /// [`Trouble::NeedsDeployment`] if this is the deployment scenario and the
    /// set is empty — fail closed, because a run over no components would
    /// produce a short trace, a stable digest, and no evidence at all.
    /// Otherwise as [`Scenario::run`].
    pub fn run_on(&self, seed: u64, deployment: &Deployment) -> Result<Outcome, Trouble> {
        self.start(seed, deployment)?.run()
    }

    /// The same, set up and not yet stepped.
    ///
    /// Split out of [`Scenario::run_on`] for `E1-P08`, which has to put cuts
    /// along a run and therefore needs the simulation rather than its outcome.
    /// Nothing about the setup moved: the two functions are one function with
    /// the last line taken off, which is what keeps a snapshotted run the same
    /// run as a plain one.
    ///
    /// # Errors
    ///
    /// As [`Scenario::run_on`].
    pub fn start(&self, seed: u64, deployment: &Deployment) -> Result<Simulation, Trouble> {
        let mut sim = Simulation::new(seed, BUDGET);
        if self.peer == Peer::Deployment && deployment.is_empty() {
            return Err(Trouble::NeedsDeployment);
        }
        // Armed before anything is installed, so a class is armed for the whole
        // run or for none of it. `fault.rs` says why there is no way to arm one
        // part-way through.
        sim.world().arm(self.injects);
        self.cover(&mut sim, deployment);
        let clients = match self.peer {
            Peer::Queue => self.install_queue(&mut sim),
            Peer::Deployment => self.install_deployment(&mut sim, deployment),
            _ => self.install_peers(&mut sim),
        };

        // Every client starts at the same instant, on its own channel, which is
        // the first interleaving decision of the run and the one that decides
        // who gets into a shallow queue first.
        let start = if self.peer == Peer::Queue {
            crate::actors::kind::START
        } else {
            crate::proto::kind::START
        };
        for id in clients {
            sim.world().send(0, id, Message { from: id, kind: start, token: 0, detail: 0 });
        }

        Ok(sim)
    }

    /// Write the header that says what this run covers.
    ///
    /// Before anything is installed, so the artefact opens with a statement of
    /// what was set up rather than closing with a summary of what happened. The
    /// wording is RFC 0032's decision in the artefact's own bytes: an exit
    /// criterion answered by two commands owes every artefact the sentence
    /// saying which of the two it is.
    ///
    /// The deployment header names the components **the build produced**, which
    /// is what this half of the pair covers, and says so in those words. It
    /// used to say *the component set the boot spawns*, which was a claim about
    /// the other half that nothing here can see: a boot instantiates whatever
    /// the frame instantiates, and today that is the first module only.
    /// `cargo xtask sim --join` is where the two sets are compared and where
    /// the difference is declared. RFC 0036.
    fn cover(&self, sim: &mut Simulation, deployment: &Deployment) {
        let world = sim.world();
        for line in COVERS {
            world.cover(line);
        }
        world.cover(&format!("scenario    {}", self.name));
        // What was broken, in the hashed bytes. An artefact that did not say it
        // was produced under injection would be quoted later as a clean run —
        // the same failure the coverage header above exists to prevent, one
        // level down. The plan and not the strikes: the header is written before
        // the run and states what was set up, and what actually struck is in the
        // `fault` records the run wrote.
        for injection in self.injects {
            world.cover(&format!(
                "injects     {} — one consultation in {}, after the first {}",
                injection.class.label(),
                injection.one_in,
                injection.after
            ));
        }
        if self.peer != Peer::Deployment {
            return;
        }
        world.cover(&format!(
            "from        {} compiled manifest record(s), read as the frame reads them",
            deployment.len()
        ));
        world.cover(
            "which boot  whichever of them a boot instantiates is the boot's half and is not",
        );
        world.cover(
            "            asserted here: `cargo xtask sim --join` compares the two. RFC 0036",
        );
        for component in deployment.components() {
            world.cover(&component.cover());
        }
    }

    /// One actor per component the loader is handed, each modelled as its own
    /// record says.
    ///
    /// **This is the join.** Everything about the shape of the run below comes
    /// out of a compiled record: how many components there are, what each one is
    /// called, what protocol it serves and therefore what is modelled under it,
    /// and how deep the ring it declares is. What comes from the scenario is the
    /// *workload* — how many operations, how long a service takes, how long a
    /// refused client waits — because a manifest declares a component's shape
    /// and has no opinion about what anybody asks it to do.
    ///
    /// One client per component, for the reason [`Scenario::install_peers`]
    /// gives: a virtqueue has exactly one driver. The `clients` a record
    /// declares is how many the *server* admits, which is a bound this model
    /// does not exercise and does not pretend to — the header writes the
    /// declared number down so a reader can see the difference.
    fn install_deployment(&self, sim: &mut Simulation, deployment: &Deployment) -> Vec<ActorId> {
        let mut clients = Vec::new();
        for (who, component) in deployment.components().iter().enumerate() {
            let cfg = Config {
                depth: self.depth,
                service_ns: self.service_ns,
                spread_ns: self.spread_ns,
                lose_one_in: self.lose_one_in,
                extent: self.extent,
                queue_size: crate::virtq::QUEUE_SIZE,
                domain: DOMAIN,
                ordered: self.deadlines,
            };
            let actor: Box<dyn crate::Actor> = match component.peer {
                Peer::Blk => Box::new(
                    Device::new(crate::blk::Blk, cfg).expect("the layout's own queue size"),
                ),
                Peer::Net => Box::new(
                    Device::new(crate::net::Net, cfg).expect("the layout's own queue size"),
                ),
                Peer::Gpu => Box::new(
                    Device::new(crate::gpu::Gpu::default(), cfg)
                        .expect("the layout's own queue size"),
                ),
                // Unreachable by construction: `deploy::MODELS` maps a protocol
                // to one of the four peers a component can be, and neither
                // `Queue` nor `Deployment` is in it. A branch rather than a
                // panic, because the mapping table is the place that decision is
                // made and a second refusal here would be a second place to
                // read.
                Peer::Queue | Peer::Deployment | Peer::Native => {
                    Box::new(Native::new(self.depth, self.service_ns, self.spread_ns, DOMAIN))
                }
            };
            let peer = sim.install(actor);
            let app = App::new(
                u32::try_from(who).unwrap_or(u32::MAX),
                peer,
                self.window,
                self.operations,
                self.buffer_bytes,
                self.retry_ns,
                // The declared ring, in entries. The one number in this loop
                // that is the *record's* rather than the scenario's, and it is
                // the one a client can feel: a ring of sixteen refuses where a
                // ring of two hundred and fifty-six does not.
                component.entries,
            )
            .urgent(self.deadlines && who == 0);
            clients.push(sim.install(Box::new(app)));
        }
        clients
    }

    /// Stage one's pair: one bounded queue and the clients that hammer it.
    fn install_queue(&self, sim: &mut Simulation) -> Vec<ActorId> {
        // The service first, so its id is stable at zero whatever the client
        // count is. A trace is read by people as well as hashed.
        let service =
            sim.install(Box::new(Service::new(self.depth, self.service_ns, self.spread_ns)));

        (0..self.clients)
            .map(|who| {
                sim.install(Box::new(Client::new(
                    who,
                    service,
                    self.window,
                    self.operations,
                    self.retry_ns,
                )))
            })
            .collect()
    }

    /// One peer and one client each, because a ring has two ends.
    ///
    /// A virtqueue has exactly one driver, so two clients sharing one device
    /// would be two drivers writing one descriptor table — which the device
    /// model refuses, correctly, and which no scenario should be asking for.
    /// The contention a scenario with several clients produces is therefore in
    /// the *timeline* rather than in one queue: several pairs due at one
    /// instant, and the seed choosing between their channels.
    fn install_peers(&self, sim: &mut Simulation) -> Vec<ActorId> {
        let cfg = Config {
            depth: self.depth,
            service_ns: self.service_ns,
            spread_ns: self.spread_ns,
            lose_one_in: self.lose_one_in,
            extent: self.extent,
            // The whole ring, so that a scenario short of descriptors is short
            // because its device is busy rather than because the model was
            // built small. `virtq::QUEUE_SIZE` is the layout's own bound and
            // `dma.rs` writes the same number into a real device's register.
            queue_size: crate::virtq::QUEUE_SIZE,
            domain: DOMAIN,
            ordered: self.deadlines,
        };

        // Peers first, for the same legibility reason the queue path installs
        // its service first: a reader of a trace should be able to tell what an
        // actor index is without counting.
        let peers: Vec<ActorId> = (0..self.clients)
            .map(|_| -> Box<dyn crate::Actor> {
                match self.peer {
                    Peer::Blk => Box::new(
                        Device::new(crate::blk::Blk, cfg).expect("the layout's own queue size"),
                    ),
                    Peer::Net => Box::new(
                        Device::new(crate::net::Net, cfg).expect("the layout's own queue size"),
                    ),
                    Peer::Gpu => Box::new(
                        Device::new(crate::gpu::Gpu::default(), cfg)
                            .expect("the layout's own queue size"),
                    ),
                    // Unreachable: `run` sends `Peer::Queue` elsewhere. An
                    // expectation rather than a panic-free branch would be a
                    // fifth peer nobody wrote.
                    Peer::Queue | Peer::Deployment | Peer::Native => {
                        Box::new(Native::new(self.depth, self.service_ns, self.spread_ns, DOMAIN))
                    }
                }
            })
            .map(|actor| sim.install(actor))
            .collect();

        peers
            .into_iter()
            .enumerate()
            .map(|(who, peer)| {
                let who = u32::try_from(who).unwrap_or(u32::MAX);
                let app = App::new(
                    who,
                    peer,
                    self.window,
                    self.operations,
                    self.buffer_bytes,
                    self.retry_ns,
                    // The ring's depth and the device's outstanding limit are
                    // one number here. Two would be more faithful and would need
                    // a knob nothing yet turns; the two refusals stay
                    // distinguishable either way, because the ring's arrives as
                    // `RingError::Full` before anything leaves this side and
                    // the device's arrives as a completion.
                    self.depth,
                )
                // The first client, and only the first: a run where everybody
                // is urgent is a run where nobody is, and what the scenario
                // measures is this client's position among the rest.
                .urgent(self.deadlines && who == 0);
                sim.install(Box::new(app))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::Protocol;
    use crate::{DEFAULT_SEED, World};

    fn digest(scenario: &Scenario, seed: u64) -> u64 {
        scenario.run(seed).expect("a shipped scenario terminates").digest()
    }

    /// Every scenario a test can run without a build behind it.
    ///
    /// All of them but `deployment`, whose component set is read from the
    /// component files `cargo xtask component` writes. Excluded by *what it is*
    /// rather than by name, so a second deployment scenario is covered the day
    /// somebody adds one. Its own reproduction is asserted below over records
    /// built in memory, and over the real files by `cargo xtask sim`.
    fn self_contained() -> impl Iterator<Item = &'static Scenario> {
        SCENARIOS.iter().filter(|scenario| scenario.peer != Peer::Deployment)
    }

    /// The component set the deployment tests run over: one of each kind of
    /// peer a manifest can name, which is more than this tree deploys today and
    /// is the point — the join has to hold for the component set that exists
    /// next year as well.
    fn deployment() -> Deployment {
        use crate::deploy::fixture::component;
        Deployment::of(vec![component("virtio-blk", "blk", 256), component("store", "store", 16)])
            .expect("two names")
    }

    #[test]
    fn no_scenario_is_in_both_tables() {
        // Two entries under one name would make `find` answer whichever came
        // first and every other command about the scenario ambiguous — a `--seed
        // soak` that swept and one that did not, told apart by nothing a reader
        // can see. It is also what `find`'s documentation promises, and a
        // promise in a doc comment that no test holds is a preference.
        for long in LONG {
            assert!(
                !SCENARIOS.iter().any(|scenario| scenario.name == long.name),
                "`{}` is in both scenario tables",
                long.name
            );
        }
        let mut names: Vec<&str> =
            SCENARIOS.iter().chain(LONG).map(|scenario| scenario.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two scenarios share a name");
    }

    #[test]
    fn a_long_scenario_reproduces_from_its_seed_like_every_other_one() {
        // `LONG` is excluded from the sweep and from `cargo xtask sim` by cost,
        // and cost is the only thing it is excluded by: a scenario nothing
        // reproduces would be a scenario whose determinism nobody has checked,
        // and `E1-P08` re-enters one of these at minute thirty-nine. Narrowed to
        // something a unit test can afford — which is the same narrowing
        // `sweep::Trial` applies, so what is checked is the scenario and not a
        // second copy of it.
        for long in LONG {
            let mut short = *long;
            short.operations = 60;
            let first = short.run(DEFAULT_SEED).expect("a long scenario terminates when narrowed");
            let second = short.run(DEFAULT_SEED).expect("terminates");
            assert_eq!(
                first.trace.text(),
                second.trace.text(),
                "`{}` did not reproduce",
                long.name
            );
            assert_eq!(first.log, second.log, "`{}` took two different interleavings", long.name);

            let other = short.run(DEFAULT_SEED ^ 0x5EED).expect("terminates");
            assert_ne!(
                first.digest(),
                other.digest(),
                "`{}` produced one artefact at two seeds, so its digest cannot fail",
                long.name
            );
        }
    }

    #[test]
    fn the_long_table_is_long_enough_to_have_the_minute_its_task_names() {
        // `E1-P08`'s exit is written in simulated minutes — *a failure at minute
        // 40* — and a scenario that finished in minute three would meet every
        // other assertion here while making that sentence meaningless. Checked
        // as the whole scenario rather than narrowed, because the length is the
        // property.
        for long in LONG {
            let outcome = long.run(DEFAULT_SEED).expect("a long scenario terminates");
            let minutes = outcome.finished_ns / 60_000_000_000;
            assert!(
                minutes >= 40,
                "`{}` runs for {minutes} simulated minute(s) and E1-P08 is about minute 40",
                long.name
            );
        }
    }

    #[test]
    fn one_seed_reproduces_its_run_byte_for_byte() {
        // Half of the exit criterion, asserted rather than observed. The other
        // half is below, and the other *shape* of it — two processes rather than
        // two calls — is `cargo xtask sim`.
        for scenario in self_contained() {
            let first = scenario.run(DEFAULT_SEED).expect("terminates");
            let second = scenario.run(DEFAULT_SEED).expect("terminates");
            assert_eq!(
                first.trace.text(),
                second.trace.text(),
                "{} produced two different traces from one seed",
                scenario.name
            );
            assert_eq!(first.digest(), second.digest());
            assert_eq!(first.log, second.log, "{} took two different interleavings", scenario.name);
        }
    }

    #[test]
    fn a_different_seed_is_a_different_run() {
        // The other half, and the one that stops the first being worthless. A
        // digest over something that does not vary agrees with itself forever;
        // this is what says the digest is over the run.
        for scenario in self_contained() {
            let base = digest(scenario, DEFAULT_SEED);
            let mut differs = 0;
            for step in 1..=8u64 {
                if digest(scenario, DEFAULT_SEED ^ step) != base {
                    differs += 1;
                }
            }
            assert_eq!(
                differs,
                8,
                "{} ignored {} of eight seed changes, so a sweep of it explores nothing",
                scenario.name,
                8 - differs
            );
        }
    }

    /// The scenario a field is exercised against, by name.
    fn base(name: &str) -> Scenario {
        *find(name).unwrap_or_else(|| panic!("`{name}` is not a scenario"))
    }

    #[test]
    fn every_field_changes_the_run() {
        // A knob nothing turns is a scenario lying about what it varies. Each
        // field is moved on its own and the digest has to move with it — against
        // a base that *reads* it, which is the part that grew when the device
        // models arrived. `extent` means nothing to a bounded queue and
        // `retry_ns` means nothing where nothing is ever refused, so a single
        // base would have made half this table pass for the wrong reason.
        /// One row: what is being moved, the scenario that reads it, and the
        /// move itself.
        type Case = (&'static str, &'static str, fn(&mut Scenario));

        let cases: &[Case] = &[
            ("clients", "contention", |s| s.clients += 1),
            ("window", "contention", |s| s.window += 1),
            ("depth", "contention", |s| s.depth += 4),
            ("operations", "contention", |s| s.operations += 1),
            ("service_ns", "contention", |s| s.service_ns += 1),
            ("spread_ns", "contention", |s| s.spread_ns += 1),
            // Read only where something is refused, which is what these two
            // bases are for.
            ("retry_ns", "contention", |s| s.retry_ns += 1),
            ("retry_ns (device)", "blkfull", |s| s.retry_ns += 1),
            ("buffer_bytes", "blk", |s| s.buffer_bytes += 512),
            // A disk's extent is only visible when a request runs past it, so it
            // is moved to a value that refuses rather than by one.
            ("extent (blk)", "blk", |s| s.extent = 1),
            // Down from a link that carries every frame to one nothing fits
            // through, which is the difference `net` and `netdrop` ship as two
            // scenarios: the label the device writes changes, and the digest
            // with it.
            ("extent (net)", "net", |s| s.extent = 512),
            ("extent (gpu)", "gpu", |s| s.extent += 1),
            // Turned off rather than nudged: a rate of three and a rate of four
            // can lose the same completion, because both are one draw reduced
            // to a different modulus and the reductions coincide often enough
            // that a nudge is not evidence. Off against on is.
            ("lose_one_in", "blkloss", |s| s.lose_one_in = 0),
            ("peer", "blk", |s| s.peer = Peer::Native),
            // Armed rather than nudged, for `lose_one_in`'s reason and one more:
            // a scenario with an empty plan consults every class and strikes at
            // none, so the only move that says the field is read is arming one.
            ("injects", "blk", |s| {
                s.injects = &[Injection { class: Class::LateCqe, after: 0, one_in: 1 }];
            }),
            // Off against on, for `lose_one_in`'s reason: the field turns two
            // things on together — a client that claims urgency and a queue
            // that reads it — and a run with neither is the control.
            ("deadlines", "deadline", |s| s.deadlines = false),
        ];

        for (what, from, moved) in cases {
            let start = base(from);
            let mut changed = start;
            moved(&mut changed);
            assert_ne!(
                digest(&changed, DEFAULT_SEED),
                digest(&start, DEFAULT_SEED),
                "{what} changed nothing against `{from}`"
            );
        }
    }

    /// Where each of a client's operations sat in the order its peer put them
    /// into the virtqueue, as a mean scaled by a thousand.
    ///
    /// The device writes `wrote::QUEUED` at the moment a chain is offered, which
    /// is exactly the moment the driver's choice is made — before it, a request
    /// is in the driver's queue; after it, it belongs to the device and nothing
    /// can move it. So this is the ordering and not a completion time, which
    /// would also carry the device's own reordering and the seed's service
    /// spread. Unit: positions per thousand operations.
    fn mean_position(scenario: &Scenario, seed: u64, urgent: bool) -> u64 {
        let outcome = scenario.run(seed).expect("a shipped scenario terminates");
        let queued: Vec<u64> = outcome
            .trace
            .records()
            .iter()
            .filter(|record| record.kind == crate::proto::wrote::QUEUED)
            .map(|record| record.token)
            .collect();
        // Which operations carry the hard class is asked of the client that
        // marks them rather than recomputed here: two derivations of one rule is
        // one too many, and this copy would go stale silently.
        let mine: Vec<usize> = queued
            .iter()
            .enumerate()
            .filter(|(_, token)| crate::client::App::is_urgent(true, **token) == urgent)
            .map(|(at, _)| at)
            .collect();
        assert!(!mine.is_empty(), "no operation of that kind was queued");
        let sum: usize = mine.iter().sum();
        (sum as u64).saturating_mul(1_000) / mine.len() as u64
    }

    #[test]
    fn a_hard_class_read_reaches_the_device_ahead_of_queued_batch_work() {
        // `E1-B06` in the model, and the reason the model has it at all: a rule
        // the simulator does not share is a rule `E1-P03`'s sweep cannot
        // explore. The boot proves the ordering against a control that differs
        // in one ordinal; this proves the *model* does the same thing, so that a
        // seed which breaks it arrives as a reproduction command.
        let ordered = base("deadline");
        let mut flat = ordered;
        flat.deadlines = false;

        // Several seeds, because one is an anecdote and this is an ordering
        // under a service time the seed varies. Every one of them and not a
        // majority: the rule is arithmetic, so a seed that breaks it is a bug
        // rather than noise.
        for seed in [DEFAULT_SEED, 1, 2, 3, 0xfeed_face_dead_beef] {
            let urgent = mean_position(&ordered, seed, true);
            let control = mean_position(&flat, seed, true);
            assert!(
                urgent < control,
                "seed {seed:#x}: hard-class operations sat at {urgent} per thousand with the \
                 queue ordering and at {control} without it, so the ordering bought them nothing"
            );
        }
    }

    #[test]
    fn the_batch_work_is_what_moves_back() {
        // The other half of the same run, and the half that says this is an
        // *ordering* rather than a queue that got faster: what the hard-class
        // operations gained, the batch operations behind them paid for. A run
        // where both moved forward would be a run where something other than the
        // order changed.
        let ordered = base("deadline");
        let mut flat = ordered;
        flat.deadlines = false;
        let batch = mean_position(&ordered, DEFAULT_SEED, false);
        let control = mean_position(&flat, DEFAULT_SEED, false);
        assert!(
            batch >= control,
            "batch work moved forward when the queue started ordering, so the ordering is not \
             what moved the hard-class work"
        );
    }

    /// Only what the client itself wrote, which is the whole of what a client
    /// can observe about its peer.
    fn as_the_client_saw_it(outcome: &Outcome) -> Vec<(&'static str, u64, u64)> {
        outcome
            .trace
            .records()
            .iter()
            .filter(|record| record.actor == crate::client::App::NAME)
            .map(|record| (record.kind, record.token, record.detail))
            .collect()
    }

    /// How many records an actor wrote under one label.
    fn count(outcome: &Outcome, actor: &str, kind: &str) -> usize {
        outcome
            .trace
            .records()
            .iter()
            .filter(|record| record.actor == actor && record.kind == kind)
            .count()
    }

    #[test]
    fn one_client_sees_the_same_thing_from_a_model_and_from_a_component() {
        // **The substitution claim, collected.**
        //
        // One client implementation, two peers with nothing in common below the
        // ring: a modelled block device with a virtqueue, descriptors, a status
        // byte and an address decode; and a component that is
        // `f_ring::registry` and a service time. Point the client at either and
        // it writes down the same sequence — the same issues, the same
        // completions, the same lengths, in the same order.
        //
        // What that establishes is not that the two peers are alike. It is that
        // **every difference between them is one the scenario asked for**: a
        // latency, an interleaving, a lost completion. A client whose control
        // flow moved when the peer below it changed would be a client the
        // user-space-driver argument does not hold for, and this is where that
        // would show.
        //
        // The window is one, deliberately. With more outstanding, each peer
        // draws its completion order at its own decision site — `blk.complete`
        // and `native.complete` — and two sites are two streams by design, so
        // the orders differ *because the seed says so*. That is the machinery
        // working rather than the claim failing, and pinning the window at one
        // is how the two are told apart.
        let mut modelled = base("blk");
        modelled.clients = 1;
        modelled.window = 1;
        modelled.spread_ns = 0;
        modelled.operations = 8;

        let mut component = modelled;
        component.peer = Peer::Native;

        for seed in [DEFAULT_SEED, 1, 0xA5A5_A5A5_A5A5_A5A5] {
            let a = modelled.run(seed).expect("terminates");
            let b = component.run(seed).expect("terminates");
            assert_eq!(
                as_the_client_saw_it(&a),
                as_the_client_saw_it(&b),
                "at seed {seed:#018x} the client could tell which peer it had"
            );
            assert_ne!(
                a.digest(),
                b.digest(),
                "the two peers produced one trace, so the comparison above is over nothing"
            );
        }
    }

    #[test]
    fn how_often_a_device_loses_work_is_visible_in_the_run() {
        // `lose_one_in` off against on is in the table above. The *rate* needs a
        // test of its own and a sweep of seeds, and the reason is a property of
        // the model rather than a weakness of it: a device resets on its first
        // loss, because RFC 0024 gives its client no other way to take a buffer
        // back — so the rate decides only *when* that happens, and two rates
        // whose first draw reduces the same way produce one run. Over enough
        // seeds they must not all coincide, or the rate would be a number
        // nothing reads.
        let often = base("blkloss");
        let mut rarely = often;
        rarely.lose_one_in = 9;
        assert!(
            (0..64u64).any(|seed| digest(&often, seed) != digest(&rarely, seed)),
            "one loss in three and one in nine gave the same run at all of sixty-four seeds"
        );
    }

    #[test]
    fn a_device_that_loses_a_completion_gives_every_buffer_back() {
        // The finding driving the real ownership types produced, asserted rather
        // than described. RFC 0024 gives a client no way to take a buffer back
        // except on evidence that its peer is gone, so a device that lost a
        // completion and carried on would leave its client holding memory
        // forever — a hang with a quiet trace. The model resets instead, and
        // this is what says the reset reaches the client and the buffers come
        // home.
        let scenario = base("blkloss");
        for seed in [DEFAULT_SEED, 7, 0x5EED_5EED_5EED_5EED] {
            let outcome = scenario.run(seed).expect("terminates");
            assert!(
                count(&outcome, crate::blk::Blk::NAME, crate::proto::wrote::DROPPED) > 0,
                "seed {seed:#018x} lost nothing"
            );
            assert_eq!(
                count(&outcome, crate::blk::Blk::NAME, crate::proto::wrote::RESET),
                1,
                "a device lost work and carried on"
            );
            assert!(
                count(&outcome, crate::client::App::NAME, crate::proto::wrote::RECLAIM) > 0,
                "the client was never told, so its buffers are still out"
            );
            assert_eq!(
                count(&outcome, crate::client::App::NAME, crate::proto::wrote::FINISHED),
                1,
                "the client did not finish, which is a hang wearing a trace"
            );
        }
    }

    #[test]
    fn a_disk_completes_in_an_order_its_driver_did_not_choose() {
        // If the used ring came back in available-ring order the model would
        // never find the client that assumes it does — and nothing in virtio
        // promises that order. Two seeds must be able to produce two orders over
        // the same submissions.
        let order = |seed| {
            base("blk")
                .run(seed)
                .expect("terminates")
                .trace
                .records()
                .iter()
                .filter(|record| record.actor == crate::blk::Blk::NAME)
                .filter(|record| record.kind == crate::proto::wrote::SERVED)
                .map(|record| record.token)
                .collect::<Vec<_>>()
        };
        let first = order(DEFAULT_SEED);
        assert!(first.len() > 4, "the disk served almost nothing, so this proves little");
        assert_eq!(first, order(DEFAULT_SEED), "one seed produced two completion orders");
        assert!(
            (1..40u64).any(|seed| order(seed) != first),
            "no seed in thirty-nine changed the order a disk completed in"
        );
    }

    #[test]
    fn a_fenced_request_is_never_overtaken_by_another_fenced_one() {
        // The display's own rule, and the reason it is a third device rather
        // than a second disk. Its unfenced transfers may come back in any order;
        // its fenced creations may not, because a driver that could not tell
        // which creation the display ran out on could not tell which resource it
        // holds.
        for seed in [DEFAULT_SEED, 3, 11, 0xDEAD_BEEF] {
            let outcome = base("gpu").run(seed).expect("terminates");
            let fenced: Vec<u64> = outcome
                .trace
                .records()
                .iter()
                .filter(|record| record.actor == crate::gpu::Gpu::NAME)
                .filter(|record| record.kind == crate::proto::wrote::FENCED)
                .map(|record| record.token)
                .collect();
            assert!(fenced.len() >= 2, "seed {seed:#018x}: too few fences to say anything");
            let mut sorted = fenced.clone();
            sorted.sort_unstable();
            assert_eq!(fenced, sorted, "seed {seed:#018x}: a fenced request was overtaken");
        }
    }

    #[test]
    fn the_lowest_address_on_a_disk_is_reachable_and_a_retry_does_not_move() {
        // Two properties of one number, and both were false until the client
        // stopped deriving an operation's position from its issue counter.
        //
        // A disk one sector long serves exactly the request at sector zero and
        // refuses every other, so the count below is the whole test: it was
        // zero when the first request landed at sector one, which meant the
        // lowest address on every modelled disk in this crate was unreachable
        // and no scenario could have said so.
        //
        // The second property is the one a device would notice. A token refused
        // for `RESOURCE` is submitted again, and a request that moved between
        // attempts is a request no driver makes — so no token may appear both
        // served and refused by the disk. That cannot be arranged by a
        // scenario: it is a consequence of the position being the token's
        // rather than the clock's.
        let mut scenario = base("blk");
        scenario.clients = 1;
        scenario.window = 4;
        // One sector, and a depth of one so that the refusal-and-retry path
        // runs against it: both properties are read out of the same run.
        scenario.extent = 1;
        scenario.depth = 1;
        scenario.operations = 6;

        for seed in [DEFAULT_SEED, 4, 0xC0FFEE] {
            let outcome = scenario.run(seed).expect("terminates");
            let of = |kind: &'static str| {
                outcome
                    .trace
                    .records()
                    .iter()
                    .filter(|record| record.actor == crate::blk::Blk::NAME)
                    .filter(move |record| record.kind == kind)
                    .map(|record| record.token)
                    .collect::<Vec<_>>()
            };
            let served = of(crate::proto::wrote::SERVED);
            let refused = of(crate::proto::wrote::IOERR);
            assert!(
                !served.is_empty(),
                "seed {seed:#018x}: a disk of one sector served nothing, so sector zero is \
                 still out of reach"
            );
            assert!(
                !refused.is_empty(),
                "seed {seed:#018x}: nothing ran past the end of a one-sector disk, so the \
                 comparison below is over one outcome"
            );
            assert!(
                served.iter().all(|token| !refused.contains(token)),
                "seed {seed:#018x}: a token was served on one attempt and refused on \
                 another, so its sector moved between them"
            );
        }
    }

    #[test]
    fn a_link_carries_its_frames_and_the_scenario_set_shows_it() {
        // The gap this pair of scenarios closed. `net` used to be the dropping
        // scenario, so `Net::serve`'s *delivery* path had a unit test in
        // `net.rs` and no scenario at all — nothing in any digest reached it,
        // and a regression that stopped frames being delivered would have moved
        // no hash in the suite. This is the delivery path with a scenario
        // behind it.
        let scenario = base("net");
        let outcome = scenario.run(DEFAULT_SEED).expect("terminates");
        assert!(
            count(&outcome, crate::net::Net::NAME, crate::proto::wrote::SERVED) > 0,
            "the link carried nothing, so the delivery path is unexercised again"
        );
        assert_eq!(
            count(&outcome, crate::net::Net::NAME, crate::proto::wrote::LINKDOWN),
            0,
            "a frame that fits was dropped"
        );
        // And the client cannot tell the difference from the dropping scenario,
        // which is the whole point of the device: same completion count, same
        // silence.
        assert_eq!(
            count(&outcome, crate::client::App::NAME, crate::proto::wrote::DONE),
            (scenario.clients * scenario.operations) as usize,
            "a delivered frame reached the client as something other than a completion"
        );
    }

    #[test]
    fn a_link_that_drops_every_frame_tells_its_client_nothing() {
        // A transmit queue has no status byte, no response header and a used
        // length of zero, so a dropped frame and a delivered one are the same
        // completion. It reads like a bug in the model and it is the protocol —
        // and a client that expects to hear about a drop has a bug only silence
        // can find. `net.rs` argues it; this is it happening end to end.
        let scenario = base("netdrop");
        let outcome = scenario.run(DEFAULT_SEED).expect("terminates");
        assert!(
            count(&outcome, crate::net::Net::NAME, crate::proto::wrote::LINKDOWN) > 0,
            "the link carried everything, so this scenario checks nothing"
        );
        assert_eq!(
            count(&outcome, crate::net::Net::NAME, crate::proto::wrote::SERVED),
            0,
            "the link delivered a frame it was too small for"
        );
        assert_eq!(
            count(&outcome, crate::client::App::NAME, crate::proto::wrote::DONE),
            (scenario.clients * scenario.operations) as usize,
            "a client was told about a drop"
        );
    }

    #[test]
    fn a_client_that_meets_both_refusals_still_finishes_its_work() {
        // Back-pressure end to end, over the real types: the ring hands the
        // buffer straight back (`RingError::Full`, and `Idle::submit` returns
        // the buffer with it) and the device answers a completion carrying
        // `RESOURCE/DEVICE_FULL`. Both are retries, neither is a loss, and the
        // count at the end is what says so.
        let scenario = base("blkfull");
        for seed in [DEFAULT_SEED, 2, 0xFEED] {
            let outcome = scenario.run(seed).expect("terminates");
            assert!(
                count(&outcome, crate::client::App::NAME, crate::proto::wrote::FULL) > 0
                    || count(&outcome, crate::blk::Blk::NAME, crate::proto::wrote::DENIED) > 0,
                "seed {seed:#018x}: neither refusal ran, so the back-off never did either"
            );
            assert_eq!(
                count(&outcome, crate::client::App::NAME, crate::proto::wrote::DONE),
                scenario.operations as usize,
                "seed {seed:#018x}: a refusal lost work instead of deferring it"
            );
        }
    }

    #[test]
    fn no_buffer_ever_comes_back_altered() {
        // The ownership property read out of the trace across every scenario
        // that has buffers. The client stamps a byte derived from the token
        // before it lends a buffer and checks it on the way back; a `done`
        // carrying `u64::MAX` is what a mismatch looks like. It cannot happen —
        // `InFlight` has no method that reaches the bytes — which is precisely
        // why it is worth checking that nothing in the model found a way.
        for scenario in self_contained() {
            for seed in [DEFAULT_SEED, 5] {
                let outcome = scenario.run(seed).expect("terminates");
                assert!(
                    outcome.trace.records().iter().all(|record| {
                        record.actor != crate::client::App::NAME
                            || record.kind != crate::proto::wrote::DONE
                            || record.detail != u64::MAX
                    }),
                    "{} at {seed:#018x}: a buffer came back with bytes nobody can write",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn a_scenario_is_found_by_its_name_and_by_nothing_else() {
        for scenario in SCENARIOS {
            assert_eq!(find(scenario.name), Some(scenario));
        }
        assert_eq!(find("handshak"), None);
        assert_eq!(find(""), None);
    }

    #[test]
    fn the_shipped_scenarios_finish_far_inside_the_budget() {
        // The budget is here to catch a model that loops. If a shipped scenario
        // were anywhere near it, the budget would be a limit on the work rather
        // than a guard against a bug, and a failure would be ambiguous.
        for scenario in self_contained() {
            let outcome = scenario.run(DEFAULT_SEED).expect("terminates");
            assert!(
                outcome.steps < BUDGET / 100,
                "{} took {} steps against a budget of {BUDGET}",
                scenario.name,
                outcome.steps
            );
        }
    }

    #[test]
    fn a_deployment_run_reproduces_from_its_seed_and_moves_when_the_seed_does() {
        // The exit criterion, for the scenario the exit is about, asserted in
        // process. The cross-process form — two runs of the binary over the real
        // component files — is `cargo xtask sim`, and it is the one that is
        // comparable evidence to two QEMU boots.
        let set = deployment();
        let scenario = base("deployment");
        let first = scenario.run_on(DEFAULT_SEED, &set).expect("terminates");
        let second = scenario.run_on(DEFAULT_SEED, &set).expect("terminates");
        assert_eq!(first.trace.text(), second.trace.text(), "one seed, two artefacts");
        assert_eq!(first.log, second.log, "one seed, two interleavings");

        let moved = (1..=8u64)
            .filter(|step| {
                scenario.run_on(DEFAULT_SEED ^ step, &set).expect("terminates").digest()
                    != first.digest()
            })
            .count();
        assert_eq!(moved, 8, "a seed change the deployment run did not feel");
    }

    #[test]
    fn the_artefact_names_the_components_it_covered_and_what_it_did_not() {
        // **The definition of boot-to-workload, in the bytes rather than in a
        // document.** A trace quoted in a year has to say what it covered, or
        // it will be quoted as covering the system.
        let set = deployment();
        let outcome = base("deployment").run_on(DEFAULT_SEED, &set).expect("terminates");
        let text = outcome.trace.text();

        assert!(text.contains("not covered the frame"), "the artefact hides its own boundary");
        assert!(text.contains("cargo xtask trace"), "the artefact does not name its other half");
        for component in set.components() {
            assert!(text.contains(&component.name), "{} is not in the artefact", component.name);
            assert!(
                text.contains(&format!("{:#018x}", component.id)),
                "{} ran without its identity in the artefact",
                component.name
            );
        }
        // Never the seed: a digest that moved with the seed on its own would
        // make the negative control pass without a single decision changing.
        assert!(
            !text.contains(&format!("{DEFAULT_SEED:#018x}")),
            "the seed reached the artefact, so its digest moves for free"
        );
    }

    #[test]
    fn a_different_component_set_is_a_different_artefact() {
        // The `commit` half of `(seed, commit)`, made mechanical. A component's
        // identity is the hash over its record *and* its image, so a change to
        // either — a manifest field, a line of driver source — is a different
        // artefact here whether or not the run behaves differently. That is
        // what lets a hash be quoted against a commit rather than against a
        // description of one.
        use crate::deploy::fixture::{component, module, record};

        let set = deployment();
        let scenario = base("deployment");
        let before = scenario.run_on(DEFAULT_SEED, &set).expect("terminates").digest();

        // The same components, one of them built from an image that differs by
        // one byte. Nothing about the run changes; the identity does.
        let mut other = record("store", "store", 16);
        other.image_bytes = 4;
        let held = module(&other);
        let mut bytes = held.as_slice().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let held = crate::deploy::Module::hold(&bytes).expect("the same length");
        let rebuilt = crate::deploy::Component::read("store.fc", &held).expect("well formed");
        let moved =
            Deployment::of(vec![component("virtio-blk", "blk", 256), rebuilt]).expect("two names");
        assert_ne!(
            before,
            scenario.run_on(DEFAULT_SEED, &moved).expect("terminates").digest(),
            "one byte of a component image left the artefact unchanged"
        );

        // And a set with a component missing is a different artefact too, which
        // is the failure that matters most: a run that silently drove fewer
        // components than the boot spawned.
        let fewer = Deployment::of(vec![component("store", "store", 16)]).expect("one name");
        assert_ne!(
            before,
            scenario.run_on(DEFAULT_SEED, &fewer).expect("terminates").digest(),
            "dropping a whole component left the artefact unchanged"
        );
    }

    #[test]
    fn the_declared_ring_is_what_the_client_submits_over() {
        // What makes this a *join* rather than a scenario with a longer header:
        // a number the manifest declares reaches the run. A ring of two entries
        // fills where a ring of two hundred and fifty-six does not, and the
        // client writes `full` when it does.
        use crate::deploy::fixture::component;
        let scenario = base("deployment");
        let roomy = Deployment::of(vec![component("virtio-blk", "blk", 256)]).expect("one");
        let cramped = Deployment::of(vec![component("virtio-blk", "blk", 2)]).expect("one");
        let full = |set: &Deployment| {
            count(
                &scenario.run_on(DEFAULT_SEED, set).expect("terminates"),
                crate::client::App::NAME,
                crate::proto::wrote::FULL,
            )
        };
        assert_eq!(full(&roomy), 0, "a ring of 256 refused a window of four");
        assert!(full(&cramped) > 0, "a ring of two never filled, so the number is not read");
    }

    #[test]
    fn the_deployment_scenario_refuses_to_run_over_nothing() {
        // Fail closed, R04. An empty run produces a short trace and a perfectly
        // stable digest, which is the one result a reproduction check must never
        // report as a pass.
        assert_eq!(base("deployment").run(DEFAULT_SEED), Err(Trouble::NeedsDeployment));
        assert!(Trouble::NeedsDeployment.message().contains("cargo xtask sim"));
    }

    #[test]
    fn a_peers_label_is_the_name_its_model_writes_into_the_trace() {
        // The header says `as blk` and the block device writes `blk` beside
        // every record it makes. Two spellings of one thing is two things a
        // reader of an artefact cannot connect.
        assert_eq!(Peer::Blk.label(), crate::blk::Blk::NAME);
        assert_eq!(Peer::Net.label(), crate::net::Net::NAME);
        assert_eq!(Peer::Gpu.label(), crate::gpu::Gpu::NAME);
        assert_eq!(Peer::Native.label(), crate::native::Native::NAME);
    }

    #[test]
    fn the_env_contract_assumes_a_clock_this_one_is_not() {
        // A finding recorded as a test rather than as a comment, because a
        // comment about a gap is a comment somebody deletes.
        //
        // `f_env::contract::check` requires the clock to advance while it draws
        // values, and its own documentation says why it can: "a virtual clock
        // advances by being used and a hardware clock advances on its own". A
        // discrete-event clock is a third kind and is neither. It cannot advance
        // under draws, because the timeline sets it to each message's instant and
        // a clock that had run ahead would be moved *backwards* by the next
        // dispatch — which is the one thing the contract exists to forbid.
        //
        // So this environment fails the contract's first property and satisfies
        // the rest, and the honest thing is to say which. When somebody teaches
        // `contract::check` about a clock the caller advances, this test is what
        // will tell them the simulator was waiting for it.
        let mut world = World::new(DEFAULT_SEED);
        assert_eq!(
            f_env::contract::check(&mut world),
            Err(f_env::contract::Violation::ClockStopped)
        );

        // The properties it does hold, checked directly rather than assumed from
        // a check that stopped early.
        let mut scheduler = World::new(DEFAULT_SEED);
        {
            use f_env::Env;
            let sched = scheduler.scheduler();
            assert_eq!(sched.choose(0), 0);
            assert_eq!(sched.choose(1), 0);
            for n in [2u32, 3, 8, 64, 1000, u32::MAX] {
                for _ in 0..64 {
                    assert!(sched.choose(n) < n);
                }
            }
            sched.yield_point();
        }

        let mut generator = World::new(DEFAULT_SEED);
        {
            use f_env::Env;
            let first = generator.next_u64();
            assert!(
                (1..64).any(|_| generator.next_u64() != first),
                "the generator returned one value forever"
            );
        }
    }
}
