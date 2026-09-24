// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The compositor boot: a component that holds the machine's scene graph at
//! ring 3, this frame as its client, and a state tree read back out of the page
//! the frame published for it.
//!
//! # What this file is and what it is not
//!
//! It is the **client's half** of `E3-B01f`. `user/compositor` is the component:
//! the graph, the frame under construction, the serve loop and the tally, in a
//! crate that forbids `unsafe`. What is here is everything a client does around
//! one — allocate a ring, stand the component up on a core of its own, submit
//! two frames' worth of scene deltas, reap the completions, and then read what
//! the component published and judge it against what was asked for.
//!
//! It is **not** a second scene graph. There is one arena, in `f-scene`, and
//! nothing here holds a node: what this file knows about the scene is what its
//! own script says it built — four nodes created, a subtree of two removed —
//! and that is deliberately a different kind of knowledge from the component's
//! census. Two counts, neither derived from the other, which is the only
//! arrangement in which a comparison means anything.
//!
//! # Why the frame is the client, and what that costs the claim
//!
//! Because there is no other client. `E1-B05`'s ring-3 supervisor does not yet
//! hand a place's occupant a core *and* a peer, so the only thing in this boot
//! that can hold the far end of a channel is the frame — the arrangement every
//! datapath boot in this tree has, recorded under the same reversal. `CHAOS_GAP`
//! in xtask carries what is still owed, and a fourth component in the same
//! position widens nothing.
//!
//! What it costs is worth stating plainly. *Client* here means **the far end of
//! a real channel across a real privilege boundary**: the deltas are real
//! `f_abi::scene` records, encoded by this file and decoded by
//! `f_scene::commit::Batch` over there, and the payloads travel in the channel's
//! own arena rather than being handed across as arguments. What it does not mean
//! is an unprivileged application, and `E3-B01g` is the task where one end of
//! this ring stops being the frame.
//!
//! # The two halves, and why neither means anything alone
//!
//! `compositor=serve` builds a scene, commits it, tears part of it down in a
//! second frame, and requires the component's published numbers to be the ones
//! the script implies. It is the positive control, and without it the refusal
//! below would be a refusal that might refuse everything.
//!
//! `compositor=mute` takes the same component's own record, empties its state
//! declaration and changes nothing else, and requires the admission a spawn
//! performs to refuse it `ADMISSION/NO_STATE_TREE` — **and requires the
//! unmodified record to be admitted**, in the same boot, through the same
//! function. RFC 0065 says every component publishes a tree; this is the half
//! that says so about a component rather than about the mechanism.
//!
//! `compositor=floorless` describes a machine below the bottom of RFC 0080's
//! ladder — a CPU and no way at all to put an image on a screen — and requires
//! the frame to refuse it a compositor `ADMISSION/NO_RUNG` before a page is
//! spent, having admitted this boot's own machine through the same function in
//! the same run. `E3-B02b`'s second clause, and it is `mute`'s shape for
//! `mute`'s reason.
//!
//! # Where `E3-B02b`'s three clauses are
//!
//! One sentence each, because they are in three different places and a reader
//! looking for the third will not otherwise find it.
//!
//! *A backend with compute shaders and no usable scan starts at rung 2 and says
//! so in the compositor's tree* is [`BACKEND_CAPABILITIES`] and the serve
//! half's `tree[4]` clause: the frame describes the hybrid's machine, and the
//! number it is checked against is [`HYBRID_RUNG`], read off RFC 0080's table
//! by hand rather than recomputed.
//!
//! *A backend satisfying no rung is refused a compositor rather than handed the
//! floor* is the floorless half.
//!
//! *An overloaded compositor holds its rung* is the serve half, and it is a
//! conjunction of clauses rather than one: the second frame is submitted a
//! nanosecond before its deadline, so `late` is one of two, the estimate is
//! more than [`OVERLOAD_TIMES`] the room that frame had — that is the overload,
//! and it is a property of the script rather than of the host this ran on — and
//! the degradation register is [`SERVE_REGISTER`]. In the same run the frame
//! writes a **better** report onto the routing page once the first frame has
//! closed, so the rung had somewhere to go. A component that recomputed its rung
//! from that word would publish [`PROMOTED_RUNG`] and go red. The negative has a
//! run behind it, which is the only form in which a negative is worth asserting.
//!
//! *Every frame carries the reduction it chose* is `E3-B07d`, and it is read off
//! the same two halves rather than out of a new one. The word the component
//! publishes is a **register** — one field per frame, newest first — so the serve
//! half requires fitted-then-short and the waking half, which closes three
//! frames with the short one in the middle, requires them to alternate. Before
//! RFC 0118 that node carried the last frame answer alone, and a build deciding
//! its degradation once published a word byte-identical to a build deciding per
//! frame; what caught it then was `late` being one of two, which is a count and
//! not a record. Both are kept: the count says how many, the register says
//! which.

use f_abi::manifest::Record;
use f_abi::scene::{
    Commit, CreateNode, Delta, Entry, NO_NODE, PAYLOAD_BYTES, RemoveNode, SetPaint, kind,
};
use f_abi::{ABI_VERSION, Cqe, control, error, feature, state};
// `f_compositor::pacing::Record` under another name, because `f_abi::manifest::
// Record` is already in scope above and the two are unrelated. The alias is the
// noun this file uses about it anyway: what the component chose, per frame.
use f_compositor::pacing::{Record as Chosen, degraded};
use f_compositor::routing::{self, at, bell, life, node, reported, stopped};
use f_compositor::tree::reported_capabilities;
use f_env::{Env, SeededEnv};
use f_interface::backend::{Capability, select};
// `E3-B06d`. The frame resolves the same theme the component was given and
// compares what it published against what the resolver answered, rather than
// against numbers written into this file. It holds no colour and computes no
// contrast: the `Resolved` is a local, the boundary table is read rather than
// measured, and `cargo xtask lint-token-pair` holds both of those of this file.
use f_interface::token::{Resolved, Theme, Token, resolve};
use f_ring::{Arena, Bell, Collector, Hardware, Mapping, Path, Poster, Producer, Window};

use crate::mem::{FRAME_SIZE, FrameAllocator};
use crate::paging;
use crate::process::{self, ServerPlan};

/// The frame's address for the board, and the component's, required to agree by
/// the machine rather than by two comments.
const _: () = assert!(crate::process::BOARD == routing::AT);

/// Entries in either ring. Unit: entries.
///
/// Sixteen, which is what every other channel in this boot is and what
/// `user/compositor/manifest.toml` declares. A frame of eight deltas therefore
/// does not fit twice over, which is the point: the client submits and reaps
/// rather than filling a ring sized to hold the whole run.
const ENTRIES: u32 = 16;

/// How long the client waits for the component to end after being told to, and
/// for any one completion. Unit: microseconds.
const EXIT_MICROS: u64 = 500_000;

/// How far apart this boot tells the component its display scans out.
/// Unit: nanoseconds.
///
/// Sixteen milliseconds and two thirds is a sixty-hertz frame. There is no
/// display in this boot and the number is therefore a *statement* rather than a
/// measurement — which is exactly what it would be on a real machine too, until
/// `E3-B02f` reaches a scanout and a driver reports one. What it has to be here
/// is a period the wake time is computed against and a period neither frame's
/// cost comes close to, so that a build which confused the two produces a
/// visibly wrong number.
const SCANOUT_PERIOD_NANOS: u64 = 16_666_667;

/// What this boot tells the component to hold back against its estimate being
/// wrong. Unit: nanoseconds.
///
/// A millisecond, and a policy rather than a measurement — `E3-B01h`'s exit
/// names the margin as a term and does not say who decides it. What it must be
/// for this boot is non-zero and not a multiple of anything else here, so that a
/// component which dropped the term from its subtraction publishes a different
/// wake time rather than the same one.
const PACING_MARGIN_NANOS: u64 = 1_000_000;

/// How much time the first frame's commit leaves between itself and its
/// deadline. Unit: nanoseconds.
///
/// A whole scanout period, which is far more than the frame cost this script
/// produces — so the first frame **fits**, the degradation policy is never
/// asked, and `f_compositor::pacing::degraded::FITTED` is what the component
/// holds after it.
const FRAME_ONE_SLACK_NANOS: u64 = SCANOUT_PERIOD_NANOS;

/// And how much the second one leaves. Unit: nanoseconds.
///
/// One nanosecond, which no frame fits inside — so the second frame is **late**,
/// the policy is asked, and it answers `SHORT` because nothing on this wire can
/// declare an effect for it to degrade. One and not zero because
/// `f_abi::scene::Commit` refuses a deadline of `NO_DEADLINE`, and the refusal is
/// right: a frame nobody scheduled is a frame the pacing loop cannot account for.
///
/// **The two slacks together are what make the degradation word evidence of
/// anything.** A component that answered the same way for every frame publishes
/// the same word as one that decided per frame; what separates them is
/// `Board::late`, which this script requires to be one out of two.
const FRAME_TWO_SLACK_NANOS: u64 = 1;

/// How far past its remaining budget this script drives the frame it overloads.
///
/// `E3-B07d`'s exit says *under 2x overload*, and on this boot that is a
/// property of the **script** rather than a measurement on a named machine:
/// [`FRAME_TWO_SLACK_NANOS`] is one nanosecond and every step of this boot's
/// seeded clock is at least one, so the estimate the component publishes is
/// thousands of times the room its second frame had. This constant is the floor
/// the verdict checks against, not the ratio the script achieves — what it
/// forbids is a later script softening the slack until the word *overload* stops
/// being true, which would leave every clause below green over a run that was
/// never overloaded. A claim gating on a frame *rate* is `E3-B07` and `E3-B07f`,
/// which need runner-class-A hardware; nothing here borrows their sentence.
/// Unit: none — a multiple of the remaining budget.
const OVERLOAD_TIMES: u64 = 2;

/// The degradation register the serving half's two frames must leave behind.
///
/// **The whole of `E3-B07d` on this half.** The first frame has a whole scanout
/// period of room and the second has a nanosecond, so the two answers differ —
/// and a register is the only published shape in which *they differed* is
/// visible at all. A component that decided its degradation once and repeated it
/// publishes `FITTED, FITTED` or `SHORT, SHORT`, both of which are a word this
/// constant is not; the snapshot this node used to carry was **byte-identical**
/// between those builds and the one that decides per frame, which is the
/// mutation that made RFC 0118 worth its diff.
///
/// Oldest pushed first, which is the order the frames closed in.
const SERVE_REGISTER: Chosen = Chosen::EMPTY.pushing(degraded::FITTED).pushing(degraded::SHORT);

/// And the register the waking half must leave behind.
///
/// [`SERVE_REGISTER`] with the batched third frame on the end, which has a whole
/// scanout period of room and therefore fits. **Fitted, short, fitted**, and the
/// alternation is what this half is worth: a compositor that latched its worst
/// answer — degraded once, degraded for ever — passes the serving half two-frame
/// clause and cannot produce this word. Derived from the serving half constant
/// rather than written out, because the two halves send the same script and a
/// register that disagreed about its first two frames would be a boot checking
/// two different runs.
const WAKE_REGISTER: Chosen = SERVE_REGISTER.pushing(degraded::FITTED);

/// The seed this boot's clock and its frame costs are drawn from.
///
/// **A seeded environment of this boot's own, rather than the one `kmain`
/// holds.** Two reasons, and the second is the one that matters. A component at
/// ring 3 has no clock — RFC 0004 — so the frame reads one and writes it into
/// the routing page, and if that reading came from this machine's timestamp
/// counter then every number this boot prints would differ between a fast host
/// and a slow one, which is the objection the deadline constant used to carry
/// and the reason nothing here was paced before. A seeded environment answers
/// it: the readings are a function of this constant alone.
///
/// Its own rather than `kmain`'s because `kmain`'s has been drawn from by every
/// stage before this one, so its state depends on what else the boot did — and a
/// number printed here would then move when an unrelated stage started drawing.
/// One constant in one file is what makes these numbers reproducible from
/// reading the file.
const PACING_SEED: u64 = 0x_C0_11_05_17_0B_01_A0_00;

/// How far this boot's clock moves between two entries, at most.
/// Unit: nanoseconds.
///
/// Drawn per entry rather than fixed, so that the two frames in this script cost
/// different amounts and a percentile is a percentile of something. One is added
/// to every draw, so a step is never zero — which is what makes *the second
/// frame does not fit in one nanosecond* an arithmetic fact rather than a
/// property of this seed.
const STEP_SPREAD_NANOS: u64 = 4096;

/// What this boot tells the component the backend under it reports.
///
/// Compute shaders and storage buffers, a CPU and a way to present an image —
/// and **no usable subgroup scan**, which is the one capability RFC 0080 keeps
/// the hybrid rung for. Built out of `Capability::index()` rather than written
/// as a bitmask, for the reason `f_compositor::tree::reported_capabilities`
/// gives from the other side: the bit positions are the vocabulary's and neither
/// end of this page may invent them.
///
/// There is no backend. This is the frame *saying* what one would report, which
/// is the same thing `E3-B02a`'s four synthetic backends are and is all this
/// task needs: what is under test is that the component selects a rung from a
/// report and publishes which, not that a GPU driver exists.
/// Unit: none — a bitmask of capability indices.
const BACKEND_CAPABILITIES: u64 = (1 << Capability::ComputeShaders.index())
    | (1 << Capability::StorageBuffers.index())
    | (1 << Capability::Cpu.index())
    | (1 << Capability::ImagePresent.index());

/// Which rung that report selects, as the component publishes it.
///
/// Two: `Rung::Hybrid` is index 1 and the published word is the index plus one,
/// so that zero can mean *no rung at all*. **Read off RFC 0080's table by hand
/// rather than recomputed**, which is the whole of why it is worth asserting: a
/// boot that called `backend::select` itself would be comparing the component's
/// answer against the same function that produced it, and would stay green
/// through any change to the ladder. This is the independent reading, and it
/// goes red the day the ladder's order or the hybrid's predicate moves.
///
/// The derivation, so a later reader can check it: rung 1 requires a prefix scan
/// across cooperating lanes and the report above does not carry one; rung 2
/// requires compute shaders, which it does, and explicitly tolerates the missing
/// scan. `E3-B02b`'s exit is the same sentence — *a backend with compute shaders
/// and no usable scan starts at rung 2 and says so in the compositor's tree*.
/// Unit: none — a rung ordinal.
const HYBRID_RUNG: u64 = 2;

/// What the floorless half tells the component the backend reports.
///
/// A CPU and nothing else: a machine that can run the code but has no way at
/// all to put a finished image on a screen, no compute shaders and no triangle
/// pipeline. Every rung of RFC 0080's ladder asks for something this set does
/// not carry — rungs 1 and 2 want compute shaders, rung 3 wants a CPU *and* a
/// present, rung 4 wants a fixed-function pipeline — so it satisfies none, and
/// that is the whole of what this half is about.
///
/// **A CPU rather than nothing, and the distinction is the one
/// [`STARVED_HEAP_BYTES`] makes by being two pages rather than zero.**
/// `Capabilities::NONE` is what a device the frame could not open reports,
/// which is a different finding wearing this one's name; what RFC 0080 refuses
/// a compositor is a machine that answered, whose answer is sound, and which is
/// still below the floor.
/// Unit: none — a bitmask of capability indices.
const FLOORLESS_CAPABILITIES: u64 = 1 << Capability::Cpu.index();

/// What the serving half tells the component the backend reports **after** its
/// first frame has closed.
///
/// [`BACKEND_CAPABILITIES`] and a usable subgroup scan: the machine gained the
/// one capability that separates the hybrid from the top rung, half way through
/// a run. The frame writes it into the same word it wrote the first report in,
/// so a component that read the page again would find it.
///
/// This is what makes *never promoted* a clause with a run behind it rather
/// than a sentence. A negative needs a moment at which the thing could have
/// happened, and a boot that reported one set for the whole run has no such
/// moment: it shows a compositor that held its rung and a compositor that
/// recomputed its rung from an unchanged report as the same green log.
/// Unit: none — a bitmask of capability indices.
const PROMOTED_CAPABILITIES: u64 = BACKEND_CAPABILITIES | (1 << Capability::SubgroupScan.index());

/// Which rung that second report would select, as the component spells a rung.
///
/// One: `Rung::ComputePath` is index 0 and the published word is the index plus
/// one. Read off RFC 0080's table by hand for [`HYBRID_RUNG`]'s reason — a boot
/// that asked `backend::select` what to expect would agree with the component
/// through any change to the ladder, including a change that broke it.
///
/// Nothing in this file requires the component to publish it. It is written
/// down because it is the number a promoted compositor *would* publish, and a
/// reader of the serve verdict needs to know that the report the frame left on
/// the page names a better rung rather than the same one.
/// Unit: none — a rung ordinal.
const PROMOTED_RUNG: u64 = 1;

// The promotion is a promotion: a different report, naming a better rung.
//
// Both halves are worth asserting because both can rot independently. A
// promoted set equal to the first one would leave the serve half writing the
// same word twice and calling it an opportunity; a promoted set naming the same
// rung would leave it writing a different word that changes nothing, which
// passes the clause below and exercises nothing.
const _: () = {
    assert!(
        PROMOTED_CAPABILITIES != BACKEND_CAPABILITIES,
        "the report the frame promotes to is the one it started with"
    );
    assert!(
        PROMOTED_RUNG < HYBRID_RUNG,
        "the report the frame promotes to does not name a better rung, so holding the first \
         one costs the component nothing"
    );
};

/// The heap the starved half describes, which is two pages.
///
/// Small enough that the graph cannot fit and large enough to be a heap: the
/// allocator's own prologue is written into it and `Heap::valid` answers true,
/// so what the component refuses is the *size* rather than a region that was
/// never described. A zero would have exercised a different refusal and called
/// it this one.
/// Unit: bytes.
const STARVED_HEAP_BYTES: u64 = 8192;

/// How many turns the component's loop spends with nothing arriving before it
/// gives up. Unit: turns.
///
/// Large, because this is a backstop rather than a mechanism: the frame's stop
/// notice is what ends the run, and a boot that reaches this number has a client
/// that stopped submitting without saying so. RFC 0046 — a hang is a count.
const IDLE_SPINS: u64 = 1_000_000;

/// Which half of the check this boot is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    /// Stand the component up and serve it two frames.
    Serve,
    /// Stand the identical component up over a heap too small to hold a graph,
    /// and require it to refuse before it serves anybody.
    ///
    /// **The control for the half above, and the one clause of this component
    /// nothing else can reach.** A compositor's graph is 131 096 bytes in a
    /// region the frame describes, and the component checks the region against
    /// what it is about to ask of it — because `Box::new` has no fallible form
    /// here and an allocation that cannot be answered ends at
    /// `handle_alloc_error`, which under `panic=immediate-abort` stops the core
    /// with nothing written anywhere. This is that check being taken, and
    /// without it the check would be a guard nothing in this tree kills.
    Starved,
    /// Put its own record past the admission a spawn performs, twice.
    Mute,
    /// Describe a machine below the bottom of RFC 0080's ladder and require the
    /// frame to refuse it a compositor, having admitted this boot's own machine
    /// through the same function first.
    ///
    /// **The half `E3-B02b`'s second clause is, and the reason it is a half
    /// rather than a test.** *Refused a compositor rather than handed the
    /// floor* is a sentence about something that does not happen, and the only
    /// way to show it is a run in which the component would otherwise have been
    /// stood up: same record, same image, same core, same rings, one word on
    /// the routing page different. What the verdict then requires is that
    /// nothing was spent — no address space, no tree, no board — which is what
    /// separates a refusal from a component that started and gave up.
    Floorless,
    /// Stand the component up on a core of its own, let it **stop that core**,
    /// and wake it from this one.
    ///
    /// **`E3-B01g`, and the half that observes cross-core delivery for the first
    /// time.** Everything [`Half::Serve`] does, plus the one difference the task
    /// is about: the routing page says the frame will ring, so the component's
    /// idle turn arms the ring's wakeup flag, looks once more, and asks the
    /// frame to halt its core. The client waits for that to happen — it reads
    /// the component's own park count off the board — and only then submits, so
    /// the doorbell it sends lands on a core that is stopped.
    ///
    /// The last frame arrives as **one batch**, which is the second clause: four
    /// entries, one publish, one operation charged to the doorbell and at most
    /// one ring. A client that charged per entry would report four here and
    /// would be measuring batching while calling it suppression.
    Wake,
}

impl Half {
    /// A word for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Starved => "starved",
            Self::Mute => "mute",
            Self::Floorless => "floorless",
            Self::Wake => "wake",
        }
    }

    /// What this half tells the component the backend under it reports.
    ///
    /// One function rather than a literal at each use, because the frame writes
    /// this word onto the routing page and also decides admission from it, and
    /// two spellings of *what this machine reports* is the arrangement in which
    /// a boot admits one machine and describes another.
    /// Unit: none — a bitmask of capability indices.
    const fn reported(self) -> u64 {
        match self {
            Self::Floorless => FLOORLESS_CAPABILITIES,
            Self::Serve | Self::Starved | Self::Mute | Self::Wake => BACKEND_CAPABILITIES,
        }
    }
}

/// May a machine reporting `bits` be given a compositor at all?
///
/// RFC 0080's refusal, at the only place in this tree that stands a compositor
/// up. A machine satisfying no rung is refused here, before a page is spent,
/// rather than handed a component that would select the floor — which is the
/// alternative that RFC names and declines, on the grounds that a refusal is
/// answerable and a compositor that misses every frame is not.
///
/// # What this function deliberately does not return
///
/// A [`f_interface::ladder::Rung`]. It answers *is there one*, and the rung
/// itself never crosses back into this file: RFC 0080 says the rung is chosen
/// **by the compositor**, and a frame that computed one and wrote it onto the
/// routing page would have taken that decision away while leaving the sentence
/// in the RFC. So the component reads the same word this function read and
/// calls the same `select` on it, and the two cannot disagree because there is
/// one ladder and one predicate — but only one of them is choosing.
///
/// The bits are turned into a capability set by
/// `f_compositor::tree::reported_capabilities`, which is the component's own
/// reader. A second reader here would be the second place a bit position is
/// decided, and the failure it produces is a frame refusing a machine over a
/// capability the component would have seen at a different index.
///
/// # Errors
///
/// `ADMISSION/NO_RUNG` where the report satisfies no rung of the ladder.
fn admit_backend(bits: u64) -> Result<(), i32> {
    match select(reported_capabilities(bits)) {
        Ok(_) => Ok(()),
        Err(_) => Err(error::pack(error::ADMISSION, error::admission::NO_RUNG)),
    }
}

/// Why the boot could not make its judgement.
///
/// Every variant is the frame's own arithmetic or a component that could not be
/// built, and every one of them fails the boot. A component that ran and did the
/// wrong thing is **not** here: that is a verdict, and [`Report::verdict`] is
/// where it is made.
#[derive(Clone, Copy, Debug)]
pub enum Trouble {
    /// No boot module carried a component file named `compositor`.
    NoComponent,
    /// Its record declares no heap, so there is nowhere for the graph to live.
    NoHeap,
    /// Its manifest and `f_compositor::routing::HEAP_BYTES` disagree about how
    /// much heap the graph needs.
    HeapDisagrees,
    /// A channel could not be described or adopted.
    Channel(i32),
    /// The component could not be built.
    Process(process::Error),
    /// Its state tree could not be published or read back.
    StateTree(i32),
    /// The core would not take the job.
    Scheduled(usize),
    /// It did not exit inside [`EXIT_MICROS`] after being told to.
    Overdue,
    /// It published a board this build cannot read.
    BadReport,
    /// A submission was refused by the ring, or a completion never arrived.
    Refused,
    /// The admission probe could not stake the account it needs.
    Admission,
    /// The component never parked inside the bound, so there was nothing asleep
    /// to ring.
    ///
    /// Its own variant rather than [`Trouble::Refused`], because it is the one
    /// failure of this boot that is about the *harness* and not about the
    /// component: a client that gave up waiting for a park has not observed a
    /// compositor that refused to sleep, it has observed itself being impatient.
    /// A verdict that folded the two would report a missing wakeup as a broken
    /// compositor.
    NotParked,
}

impl Trouble {
    /// A line for the log.
    #[must_use]
    pub const fn why(self) -> &'static str {
        match self {
            Self::NoComponent => "no boot module carries a component file named compositor",
            Self::NoHeap => "the compositor's manifest declares no heap for its graph",
            Self::HeapDisagrees => {
                "the manifest's heap and the crate's own constant are different numbers"
            }
            Self::Channel(_) => "a channel could not be described or adopted",
            Self::Process(_) => "the component could not be built",
            Self::StateTree(_) => "the component's state tree could not be published or read",
            Self::Scheduled(_) => "the core would not take the job",
            Self::Overdue => "the component did not exit after being told to",
            Self::BadReport => "the component published a board this build cannot read",
            Self::Refused => "the ring refused a submission the client had room for",
            Self::Admission => "the admission probe could not stake an account",
            Self::NotParked => "the component never stopped its core, so nothing was asleep",
        }
    }
}

/// Where and for how long the component runs.
///
/// A struct because the five travel together and always will: they are one
/// decision — *which core, on what clock, for how long* — and threading them as
/// five arguments is what made `objects::demonstrate`'s signature wider than
/// clippy's bound and a reader's patience.
pub struct Scheduling {
    /// The physical address of the frame the **frame's** state tree is published
    /// in, which every shape maps read-only. The component's own tree is a
    /// different page and this file publishes it. Unit: bytes, physical.
    pub tree: u64,
    /// Which core the component is allocated. Unit: none — a core index.
    pub cpu: usize,
    /// The rate that core arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks it asks for. Unit: timer ticks.
    pub target: u64,
    /// The timestamp counter's rate, for bounding the waits. Unit: kilohertz.
    pub tsc_khz: u64,
}

/// What the boot saw, from both sides.
pub struct Report {
    /// Which half ran.
    pub half: Half,
    /// Entries the client put on the ring. Unit: entries.
    pub submitted: u64,
    /// Completions it reaped. Unit: entries.
    pub completed: u64,
    /// Completions that carried a refusal, or answered an entry the client was
    /// not waiting for. Unit: entries.
    pub refused: u64,
    /// What the component published on its board, or zeroes for a half that
    /// stood none up.
    pub board: Board,
    /// The nodes its declared schema carries, as the frame wrote them.
    /// Unit: nodes.
    pub tree_nodes: u32,
    /// The snapshot taken before the component ran a line. Unit: none — a fold.
    ///
    /// **Not zero, and expecting it to be was this boot's own first red.**
    /// `f_abi::state::Reader::snapshot` folds the schema as well as the words,
    /// so a tree nothing has written into still has a fold. What *blank* means
    /// is that every word is zero, which is [`Report::tree_before`], and what
    /// this number is for is saying that the fold moved.
    pub tree_blank: u64,
    /// The fifteen words this component publishes, read before it ran a line.
    ///
    /// Required to be zero, which is what makes the fifteen read afterwards
    /// evidence of anything: a component that published nothing into a tree
    /// somebody else had already filled in would be indistinguishable from one
    /// that published. Unit: as [`Report::tree`].
    pub tree_before: [u64; WORDS],
    /// The snapshot taken after it ended. Unit: none — a fold.
    pub tree_after: u64,
    /// The fifteen words this component's manifest says it publishes, read back
    /// out of the tree by id, in `node::WRITTEN`'s order: frames, edits, nodes,
    /// refused, rung, frame, deadline, pacing, degraded, resolves, notes, rules,
    /// waits, signalled, timeouts.
    ///
    /// Fifteen is `f_abi::manifest::STATE_NODES_MAX` less the subtree, so this
    /// array cannot grow again without an RFC widening that bound.
    pub tree: [u64; WORDS],
    /// The deadline the client put on the last commit it submitted.
    ///
    /// Held by the *client*, because the client computed it — a deadline in this
    /// boot is a clock reading plus a slack, and the component is required to
    /// publish the number it was sent rather than one of its own. A tree that
    /// agreed with the component's board and with nothing outside it would be
    /// two copies of one opinion.
    /// Unit: nanoseconds, in this boot's seeded epoch.
    pub submitted_deadline: u64,
    /// The last reading the client wrote into the routing page.
    ///
    /// Unit: nanoseconds, in this boot's seeded epoch.
    pub last_tick: u64,
    /// Whether the record as its manifest declares it was admitted.
    pub admitted: bool,
    /// What the admission said about the same record with its state declaration
    /// emptied. Unit: none — a packed refusal, or zero for one that was
    /// admitted.
    pub muted: i32,
    /// Whether the report this boot's own machine makes was admitted a
    /// compositor by [`admit_backend`].
    ///
    /// **The positive control for [`Report::floorless`], and a separate field
    /// from [`Report::admitted`] because it is a separate subject.** That one
    /// is about a record and this one is about a machine; a boot that folded
    /// the two would report *something was admitted* and leave a reader to
    /// guess which.
    pub backend_admitted: bool,
    /// What [`admit_backend`] said about a machine below the bottom of the
    /// ladder. Unit: none — a packed refusal, or zero for one that was
    /// admitted.
    pub floorless: i32,
    /// What the routing page carried at `at::BACKEND_CAPABILITIES` when the run
    /// ended.
    ///
    /// Read back off the page rather than remembered, which is what makes the
    /// serving half's *never promoted* clause a clause about a run: the frame
    /// writes a better report there part way through, and this is the frame
    /// checking that the write landed before it requires the component to have
    /// ignored it. Zero for a half that stood no component up, because there
    /// was no page.
    /// Unit: none — a bitmask of capability indices.
    pub reported: u64,
    /// How long the image is. Unit: bytes.
    pub image: u64,
    /// How much heap the frame described for this run.
    ///
    /// Reported rather than taken from the manifest, because the two differ on
    /// exactly one half and that half is about the difference.
    /// Unit: bytes.
    pub heap: u64,
    /// What the doorbell did, on the half that has one.
    pub bells: Bells,
}

/// What the doorbell did, from both ends of it.
///
/// **Two ends and neither derived from the other, which is the whole design of
/// this half.** The client's three counts are what *this* core decided: how many
/// operations it accounted, how many of those it rang for, and what the one
/// batch cost. The core's four are what the *other* core observed: interrupts
/// delivered to it, halts it took, halts a doorbell ended, and waits a latched
/// doorbell spared it. A build in which the ringing and the delivery were the
/// same counter could not fail this half.
#[derive(Clone, Copy, Default)]
pub struct Bells {
    /// Which path `f_ring::doorbell::Path::select` chose for this channel.
    ///
    /// Carried rather than inferred from the counts, and the difference matters:
    /// a `KernelIpi` channel whose consumer never slept rings nothing, so
    /// *rings is zero* and *the path is polling* are different statements and a
    /// log that computed one from the other would say the wrong one on a
    /// perfectly suppressed run.
    /// Unit: none — the name of a path.
    pub path: &'static str,
    /// Which core the component ran on. Unit: none — a core index.
    pub worker: usize,
    /// Which core the client ran on. Unit: none — a core index.
    ///
    /// Published because *cross-core* is the exit's word and a boot that printed
    /// only the target would leave a reader to assume the two differed.
    pub client: usize,
    /// Operations the client's doorbell accounted, where a published batch is
    /// one. Unit: operations.
    pub operations: u64,
    /// Doorbells the client sent. Unit: doorbells.
    pub rings: u64,
    /// Entries in the one batch this half publishes. Unit: entries.
    pub batch_entries: u64,
    /// Operations that batch was charged. Unit: operations.
    pub batch_operations: u64,
    /// Doorbells it cost. Unit: doorbells.
    pub batch_rings: u64,
    /// Doorbells the worker core was delivered, read off that core after it
    /// reported finished. Unit: doorbells.
    pub delivered: u64,
    /// Times the worker core was really stopped. Unit: halts.
    pub parks: u64,
    /// Halts of the worker core that a doorbell ended, as distinct from the
    /// timer. Unit: halts.
    pub woken: u64,
    /// Waits on the worker core that a latched doorbell spared. Unit: waits.
    pub spared: u64,
}

/// How many words this component publishes into its own tree.
///
/// `f_compositor::routing::node::WRITTEN`'s length, read from the component's
/// own crate rather than written here: the two sides index the same array, and a
/// boot that carried its own count would be the second place the number is
/// written and the one that goes stale.
/// Unit: nodes.
const WORDS: usize = node::WRITTEN.len();

/// How many waits one UI frame enters, on section 08's chain.
///
/// Two, and it is derived from the sentence rather than from the record's bound:
/// the chain has three stages and the **application is the first**, so it waits
/// for nothing. `f_abi::trace::STAGES_PER_FRAME` is three and is the right number
/// for sizing the record — a stage that waits for nothing still has an entry's
/// worth of room reserved against the day it does — and it is the wrong number
/// here, because this is a count of waits entered and not of room.
///
/// It breaks on exactly what `abi/src/trace.rs` says breaks its own bound: a
/// fourth waiting stage, or one stage submitting twice. Both are addition and
/// neither touches this file, so the clause this constant is in is what says so.
/// Unit: waits per UI frame.
const WAITS_PER_FRAME: u64 = 2;

/// What the component wrote into the half of its routing page that is its own.
#[derive(Clone, Copy, Default)]
pub struct Board {
    /// Entries it took off the data ring. Unit: entries.
    pub drained: u64,
    /// Deltas staged into a frame that never closed. Unit: deltas.
    pub staged: u64,
    /// Frames that closed. Unit: frames.
    pub frames: u64,
    /// Deltas that reached the graph. Unit: deltas.
    pub edits: u64,
    /// Nodes that began. Unit: nodes.
    pub created: u64,
    /// Nodes that left, subtrees included. Unit: nodes.
    pub removed: u64,
    /// Nodes the graph holds now. Unit: nodes.
    pub live: u64,
    /// Entries it refused. Unit: entries.
    pub refused: u64,
    /// The frame identifier of the last frame that closed. Unit: none.
    pub named: u64,
    /// Why its loop ended. Unit: none — a `stopped` ordinal.
    pub outcome: u64,
    /// How many of its declared nodes it wrote a word into. Unit: nodes.
    pub published: u64,
    /// The scanout the last closed frame was paced against. Unit: nanoseconds.
    pub scanout: u64,
    /// Its rolling p99 estimate at that frame. Unit: nanoseconds.
    pub estimate: u64,
    /// What it held back against the estimate being wrong. Unit: nanoseconds.
    pub margin: u64,
    /// The wake time the three of them make. Unit: nanoseconds.
    pub wake: u64,
    /// How many frames the estimate was taken over. Unit: samples.
    pub samples: u64,
    /// What it gave up to fit the last frame, as a
    /// `f_compositor::pacing::degraded` ordinal. Unit: none.
    pub degraded: u64,
    /// Which rung it is holding. Unit: none — a rung ordinal.
    pub rung: u64,
    /// The deadline the last closed frame carried. Unit: nanoseconds.
    pub deadline: u64,
    /// How many frames did not fit before their own deadline. Unit: frames.
    pub late: u64,
    /// How many times it asked the frame to stop its core. Unit: waits.
    pub parked: u64,
    /// How many of those really stopped it, as the frame answered. Unit: waits.
    ///
    /// The component's own count, and deliberately not the frame's: the frame
    /// keeps its own in `crate::doorbell`, read off the worker core after the
    /// join, and the verdict requires the two to agree. Two counters neither of
    /// which is derived from the other is the only arrangement in which their
    /// agreement says anything.
    pub halted: u64,

    // --- the resolved theme, `E3-B06d` --------------------------------------
    //
    // Five words about one call the component made before it served anybody, and
    // the reason the frame reads all five rather than the three that have a node
    // is that two of them are about the *report* rather than about the machine.
    // A component that had let its report truncate, or whose count of notes
    // disagreed with the resolver's own verdict on the same report, would publish
    // three plausible tree words and be caught by these two.
    /// How many times it resolved a theme. One. Unit: none — resolutions.
    pub resolves: u64,
    /// How many decisions the resolver made that the theme's author did not.
    ///
    /// `f_interface::token::Report::len` as the component read it. Unit: none —
    /// notes.
    pub notes: u64,
    /// How many notes did not fit in the report and were counted instead.
    ///
    /// `f_interface::token::Report::dropped`. Read beside [`Board::notes`] rather
    /// than folded into it, because a truncated report is the one place in that
    /// module where something happens and nothing says so, and a frame that read
    /// only the length would put the silence back. Unit: none — notes.
    pub dropped: u64,
    /// One where the resolver judged the theme untouched, zero otherwise.
    ///
    /// `f_interface::token::Report::is_clean`, which is *no notes and none
    /// dropped*. The frame reads it as well as the two counts and requires the
    /// three to be consistent — a component reporting nought notes, nought
    /// dropped and a dirty report has one of the three wrong, and no one of them
    /// alone says which. Unit: none — a flag.
    pub clean: u64,
    /// How many ordered pairs of distinct grounds owe a rule between them.
    ///
    /// RFC 0079's one obligation on a compositor, counted. Unit: none — ordered
    /// pairs of grounds.
    pub rules: u64,

    // --- the boundary crossings, `E3-B01j` ----------------------------------
    //
    // Three words, and the reason the frame reads all three rather than the one
    // it needs is that the one it needs is a sum and a division. A component that
    // published a plausible total with nothing behind it, or a rate that was not
    // the total over the frames, would satisfy a single comparison and be caught
    // by these. The frame's own two counts are in [`Report::submitted`] and
    // [`Report::completed`], taken on this side of the boundary and derived from
    // nothing here — which is the whole of what `E3-B01j` asks for.
    /// Completions the component put on the data ring. Unit: entries.
    pub answered: u64,
    /// Entries that crossed the boundary in either direction, as the component
    /// summed them. Unit: entries.
    pub crossings: u64,
    /// [`Board::crossings`] per UI frame, times a thousand, as the component
    /// divided. Unit: entries per UI frame, times one thousand.
    pub crossings_per_frame_x1000: u64,

    // --- the synchronisation state, `E3-B05f` -------------------------------
    //
    // Seven words about the chain, of which three have a node in the component's
    // tree and four do not. `f_compositor::routing::reported` draws the division
    // and this file relies on it: the three are the machine's answer to *is
    // anything still waiting*, and the four are whether the record they come out
    // of can be believed at all — which is evidence about the component and
    // belongs where the frame checks the component.
    /// Waits the last frame entered and did not get out of. Unit: waits.
    pub waits: u64,
    /// The highest value that landed on the compositor's own timeline.
    /// Unit: none — a timeline value, which is an ordinal.
    pub signalled: u64,
    /// Frames that closed with a wait outstanding and no room left.
    /// Unit: frames — UI frames.
    pub timeouts: u64,
    /// Waits every frame's trace named, summed. Two per frame in this build.
    /// Unit: waits.
    pub traced: u64,
    /// Waits the traces could not hold, summed. RFC 0101. Unit: waits.
    pub trace_dropped: u64,
    /// One while every trace named every wait its frame entered.
    /// Unit: none — a flag.
    pub trace_complete: u64,
    /// Refusals the component's own chain produced, which is the component
    /// contradicting itself. Unit: refusals.
    pub chain_refusals: u64,
}

impl Board {
    /// Read one out of the page, or `None` where the component never finished
    /// writing it.
    ///
    /// Visible to the rest of the frame rather than to this file alone, for
    /// [`found`]'s reason: `kernel/src/input.rs` stands the same component up and
    /// reads the same page, and a second reader there would be a second opinion
    /// about what a compositor published — the failure this type exists to make
    /// impossible between the component and the frame, repeated inside it.
    pub(crate) fn of(board: &Window) -> Option<Self> {
        if board.read64(reported::MAGIC).ok()? != routing::MAGIC {
            return None;
        }
        Some(Self {
            drained: board.read64(reported::DRAINED).ok()?,
            staged: board.read64(reported::STAGED).ok()?,
            frames: board.read64(reported::FRAMES).ok()?,
            edits: board.read64(reported::EDITS).ok()?,
            created: board.read64(reported::CREATED).ok()?,
            removed: board.read64(reported::REMOVED).ok()?,
            live: board.read64(reported::LIVE).ok()?,
            refused: board.read64(reported::REFUSED).ok()?,
            named: board.read64(reported::TOKEN).ok()?,
            outcome: board.read64(reported::OUTCOME).ok()?,
            published: board.read64(reported::PUBLISHED).ok()?,
            scanout: board.read64(reported::SCANOUT).ok()?,
            estimate: board.read64(reported::ESTIMATE).ok()?,
            margin: board.read64(reported::MARGIN).ok()?,
            wake: board.read64(reported::WAKE).ok()?,
            samples: board.read64(reported::SAMPLES).ok()?,
            degraded: board.read64(reported::DEGRADED).ok()?,
            rung: board.read64(reported::RUNG).ok()?,
            deadline: board.read64(reported::DEADLINE).ok()?,
            late: board.read64(reported::LATE).ok()?,
            parked: board.read64(reported::PARKED).ok()?,
            halted: board.read64(reported::HALTED).ok()?,
            resolves: board.read64(reported::RESOLVES).ok()?,
            notes: board.read64(reported::NOTES).ok()?,
            dropped: board.read64(reported::DROPPED).ok()?,
            clean: board.read64(reported::CLEAN).ok()?,
            rules: board.read64(reported::RULES).ok()?,
            answered: board.read64(reported::ANSWERED).ok()?,
            crossings: board.read64(reported::CROSSINGS).ok()?,
            crossings_per_frame_x1000: board.read64(reported::CROSSINGS_PER_FRAME_X1000).ok()?,
            waits: board.read64(reported::WAITS).ok()?,
            signalled: board.read64(reported::SIGNALLED).ok()?,
            timeouts: board.read64(reported::TIMEOUTS).ok()?,
            traced: board.read64(reported::TRACED).ok()?,
            trace_dropped: board.read64(reported::TRACE_DROPPED).ok()?,
            trace_complete: board.read64(reported::TRACE_COMPLETE).ok()?,
            chain_refusals: board.read64(reported::CHAIN_REFUSALS).ok()?,
        })
    }
}

/// What the first frame is called.
///
/// One and two rather than two memorable words: a frame identifier is the
/// submitter's and is opaque to the compositor, so the only thing this client
/// needs of them is that the two differ and neither is zero — `f_abi::scene`
/// refuses a zero, so a zeroed payload is not a commit of frame zero.
/// Unit: none — a frame identifier, not a quantity.
const FRAME_ONE: u64 = 1;

/// What the second frame is called. See [`FRAME_ONE`].
const FRAME_TWO: u64 = 2;

/// What the third frame is called, and the one only the wake half sends.
/// See [`FRAME_ONE`]. Unit: none — a frame identifier, not a quantity.
const FRAME_THREE: u64 = 3;

/// How much room the third frame's commit leaves. Unit: nanoseconds.
///
/// A whole scanout period, as the first frame's: this frame is not about pacing
/// and a late one here would move the degradation counters the serving half's
/// clauses are written against.
const FRAME_THREE_SLACK_NANOS: u64 = SCANOUT_PERIOD_NANOS;

/// How many deltas the wake half sends as one batch. Unit: deltas.
///
/// Four, and it has to be more than one for the clause to say anything and
/// small enough that four payloads fit in the channel's arena beside each
/// other — which they do with room to spare: `PAYLOAD_BYTES` is 56 and the
/// arena of a one-page channel with sixteen entries is over two kibibytes. The
/// const assertion below is what keeps that true rather than remembered.
const BATCH_DELTAS: usize = 4;

/// The node whose subtree the second frame removes.
///
/// Node 2 holds node 3 under it, so removing it is **two** nodes and not one —
/// which is the arithmetic this client does for itself and the number the
/// component is required to agree with. A client that asked for one removal and
/// checked that one node left would be checking that a removal happened rather
/// than that a subtree went with it.
const REMOVED_ROOT: u32 = 2;

/// How many nodes that removal takes. Unit: nodes.
const REMOVED_NODES: u64 = 2;

/// The two frames this client submits, in the order they go on the ring.
///
/// Written out as records rather than built in a loop, because every one of them
/// is a decision a reader should be able to check against `abi/src/scene.rs`:
/// the sections arrive in the order a frame's entries must — removals, then
/// creations, then property sets — the commit carries a deadline and every other
/// delta carries none, and the second frame's removal names a node with a child.
///
/// `user/compositor/src/tree.rs` builds the same scene in its own tests, and the
/// two are deliberately not shared: a fixture both sides imported would make the
/// host test and the boot one observation rather than two.
fn script() -> [Delta; 8] {
    let create = |node: u32, parent: u32, kind: u16| Delta {
        user_data: u64::from(node),
        class: 0,
        deadline: 0,
        payload_offset: 0,
        flags: 0,
        body: Entry::CreateNode(CreateNode { node, parent, before: NO_NODE, kind }),
    };
    // **The deadline a commit carries here is a *slack* and not an instant**,
    // and [`drive`] turns it into one by adding the clock reading it wrote for
    // that entry. It has to be that way round: a deadline is a point on the
    // frame's own clock, this boot's clock is seeded and its origin is wherever
    // the draws have reached by the time the commit goes on the ring, and a
    // literal instant written here would be a deadline that had already passed
    // or one a whole run away depending on nothing anybody chose.
    //
    // What it buys is that *how much room the frame had* is exact rather than
    // approximate: the component reads the same reading the client added, so the
    // remaining time at the commit is the slack to the nanosecond, and the two
    // halves of this script — one frame that fits and one that does not — are
    // arithmetic rather than a race.
    let commit = |user_data: u64, named: u64, slack_nanos: u64| Delta {
        user_data,
        class: 0,
        deadline: slack_nanos,
        payload_offset: 0,
        flags: 0,
        body: Entry::Commit(Commit { frame_token: named }),
    };
    [
        create(1, NO_NODE, kind::LAYER),
        create(2, 1, kind::TRANSFORM),
        create(3, 2, kind::DRAW),
        create(4, 1, kind::SEMANTIC),
        Delta {
            user_data: 5,
            class: 0,
            deadline: 0,
            payload_offset: 0,
            flags: 0,
            body: Entry::SetPaint(SetPaint {
                node: 3,
                red_x65535: 0x4000,
                green_x65535: 0x8000,
                blue_x65535: 0xC000,
                alpha_x65535: 0xFFFF,
                stroke_width_x65536: 0x0002_0000,
            }),
        },
        commit(6, FRAME_ONE, FRAME_ONE_SLACK_NANOS),
        Delta {
            user_data: 7,
            class: 0,
            deadline: 0,
            payload_offset: 0,
            flags: 0,
            body: Entry::RemoveNode(RemoveNode { node: REMOVED_ROOT }),
        },
        commit(8, FRAME_TWO, FRAME_TWO_SLACK_NANOS),
    ]
}

/// The third frame, published as **one batch**.
///
/// **This is the second half of `E3-B01g`'s exit and it is deliberately a
/// separate array from [`script`].** *A batch still rings at most one doorbell*
/// is a claim about an accounting rule, and the only way to put teeth in it is
/// to have a batch whose entry count and operation count are different numbers
/// that a boot prints side by side. Four entries, one publish, one operation.
///
/// **What this is not is `E3-B01j`'s evidence.** That task counts boundary
/// crossings per UI frame on both sides and `E3-B01`'s exit is the number under
/// ten; nothing here publishes such a figure and the verdict below asks for no
/// bound on one. What this batch is for is the doorbell, and the distinction
/// matters because the two mechanisms move the same counter in opposite
/// directions — `f_ring::doorbell::Bell::submitted` says so, and a boot that
/// blurred them would report batching working and call it suppression.
///
/// Three creations under node 1, which the second frame left standing, and a
/// commit. No removal, so [`Expected`]'s seed is not disturbed and the wake
/// half's census is the serving half's plus three.
fn batch_script() -> [Delta; BATCH_DELTAS] {
    let create = |user_data: u64, node: u32, parent: u32, kind: u16| Delta {
        user_data,
        class: 0,
        deadline: 0,
        payload_offset: 0,
        flags: 0,
        body: Entry::CreateNode(CreateNode { node, parent, before: NO_NODE, kind }),
    };
    [
        create(9, 5, 1, kind::LAYER),
        create(10, 6, 5, kind::TRANSFORM),
        create(11, 7, 5, kind::DRAW),
        Delta {
            user_data: 12,
            class: 0,
            deadline: FRAME_THREE_SLACK_NANOS,
            payload_offset: 0,
            flags: 0,
            body: Entry::Commit(Commit { frame_token: FRAME_THREE }),
        },
    ]
}

/// What the script implies, counted from the script rather than written down
/// beside it.
///
/// **This is the half that makes the comparison worth making.** Every number the
/// component publishes is checked against one of these, and every one of these
/// is derived by walking the deltas this client actually submitted — so a script
/// that changes moves both sides of the comparison and a component that changes
/// moves one.
struct Expected {
    /// Frames the script closes. Unit: frames.
    frames: u64,
    /// Deltas that should reach the graph. Unit: deltas.
    edits: u64,
    /// Nodes the script creates. Unit: nodes.
    created: u64,
    /// Nodes it removes, subtrees included. Unit: nodes.
    removed: u64,
}

impl Expected {
    /// Walk the script and count.
    ///
    /// The `match` has one arm per opcode and no wildcard, so a seventh scene
    /// opcode stops this build and asks what a boot should expect of it.
    fn of(script: &[Delta]) -> Self {
        let mut expected = Self { frames: 0, edits: 0, created: 0, removed: REMOVED_NODES };
        expected.count(script);
        expected
    }

    /// Add one run of deltas to what is expected.
    ///
    /// Split out of [`Expected::of`] for the wake half, which submits two runs
    /// — the script one entry at a time and the third frame as a batch — and
    /// must not seed [`REMOVED_NODES`] twice. Calling `of` on each and adding
    /// the two would do exactly that, and the boot would expect four removals
    /// where the script asks for two.
    ///
    /// The `match` has one arm per opcode and no wildcard, so a seventh scene
    /// opcode stops this build and asks what a boot should expect of it.
    fn count(&mut self, script: &[Delta]) {
        for delta in script {
            match delta.body {
                Entry::Commit(_) => self.frames += 1,
                Entry::CreateNode(_) => {
                    self.created += 1;
                    self.edits += 1;
                }
                Entry::SetTransform(_)
                | Entry::SetPath(_)
                | Entry::SetPaint(_)
                | Entry::SetEffect(_)
                | Entry::RemoveNode(_) => self.edits += 1,
            }
        }
    }

    /// How many nodes the graph should hold at the end. Unit: nodes.
    const fn live(&self) -> u64 {
        self.created.saturating_sub(self.removed)
    }
}

impl Report {
    /// Did the machine do what this half asked of it?
    ///
    /// # Errors
    ///
    /// A sentence for the boot log, naming the clause that did not hold.
    pub fn verdict(&self) -> Result<(), &'static str> {
        match self.half {
            Half::Mute => self.mute_verdict(),
            Half::Starved => self.starved_verdict(),
            Half::Serve => self.serve_verdict(),
            Half::Floorless => self.floorless_verdict(),
            Half::Wake => self.wake_verdict(),
        }
    }

    /// What both serving halves owe about crossings and about the chain.
    ///
    /// # Why this is one function and not two blocks
    ///
    /// Because every clause here is a *relation between numbers* rather than a
    /// statement about a script, and a relation that held on one half and was
    /// forgotten on the other would be a clause with half the coverage it reads
    /// as having. The numbers each script implies are pinned in each half's own
    /// verdict, where the script is; what is here is what has to hold whatever the
    /// script says.
    ///
    /// # `E3-B01j`: both sides count, and the two counts are required to agree
    ///
    /// That is the exit's own sentence and the first two clauses are it. The
    /// frame's count is `submitted + completed` — entries it put on the ring and
    /// completions it reaped, both counted on this side of the boundary and
    /// derived from nothing the component published. The component's is
    /// `drained + answered`, summed **by the component** into
    /// `reported::CROSSINGS`; the third clause requires that sum to be the sum of
    /// its own two terms, so a component that published a plausible total with
    /// nothing behind it is caught rather than believed.
    ///
    /// One side counting and the other trusting it is the failure this line
    /// exists to prevent, and the arrangement that avoids it is the one
    /// `claims/0037` established between a runtime and the frame: two counters,
    /// neither derived from the other, required equal.
    ///
    /// # `E3-B05f`: the trace is checked before anything is read out of it
    ///
    /// Three clauses, and their order is the argument. A trace that dropped a wait
    /// is a record whose other numbers are over an unknown set — `unreleased` on a
    /// truncated trace is *of the waits it kept* — so completeness is asked first.
    /// Then the entry count: two waits per frame is section 08's shape with the
    /// application removed, because the application is the first stage and waits
    /// for nothing. A component whose present stage waited and recorded nothing
    /// would publish a plausible outstanding count and half of this.
    ///
    /// # Errors
    ///
    /// A sentence for the boot log, naming the clause that did not hold.
    fn crossings_and_chain_held(&self) -> Result<(), &'static str> {
        if self.board.crossings != self.submitted.saturating_add(self.completed) {
            return Err("the component and the frame disagree about how many entries crossed the \
                 boundary. The two counts are taken on opposite sides of it and neither is \
                 derived from the other, so one of them is counting something correlated with \
                 a crossing rather than a crossing");
        }
        if self.board.crossings != self.board.drained.saturating_add(self.board.answered) {
            return Err(
                "the component's crossing total is not its own two directions added together, \
                 so the total is a number with nothing behind it rather than a sum a reader \
                 can check",
            );
        }
        if self.board.answered != self.completed {
            return Err(
                "the completions the component says it put on the ring are not the ones the \
                 client reaped, so the return leg of the crossing count is being reported by \
                 one side only",
            );
        }
        // The rate, re-derived rather than trusted, which is `reported::WAKE`'s
        // rule: a published quotient whose terms are beside it is checkable, and
        // this is the check. Integer division both sides, so the two truncate
        // identically or the component divided something else.
        // `checked_div` rather than a guarded division, because clippy is right
        // about the shape and the reason is worth the line: *no frame closed* and
        // *the quotient is zero* are the same published word here, and a reader of
        // the component's own `crossings_per_frame_x1000` is told to read
        // `frames` beside it to tell them apart. Spelling the absence as `None`
        // and then as a zero is that arrangement said once rather than twice.
        let expected_rate_x1000 =
            self.board.crossings.saturating_mul(1000).checked_div(self.board.frames).unwrap_or(0);
        if self.board.crossings_per_frame_x1000 != expected_rate_x1000 {
            return Err("the crossings-per-frame figure the component published is not its own \
                 crossing total over its own frame count, so the number a claim would carry \
                 is not the number its two terms make");
        }
        // --- the frame trace, `E3-B05f` --------------------------------------
        if self.board.trace_complete != 1 || self.board.trace_dropped != 0 {
            return Err(
                "a frame's trace did not name every wait its frame entered, so every other \
                 number read out of it is over the waits the record happened to keep. RFC 0101 \
                 is why the component says so rather than the bound being asserted somewhere",
            );
        }
        if self.board.traced != self.board.frames.saturating_mul(WAITS_PER_FRAME) {
            return Err(
                "the traces do not name two waits per frame: section 08 has three stages and \
                 two of them wait — the application is the first and waits for nothing. Below \
                 this is a stage that waited and recorded nothing, which is the one thing \
                 E3-B05's negative cannot survive; above it is a trace that was not emptied at \
                 the frame boundary, so one frame's record carries the frame before it and \
                 still reads as complete. Both directions are named because both were reached \
                 by mutation, and a message naming only the first sent a reader looking for \
                 the wrong defect",
            );
        }
        if self.board.chain_refusals != 0 {
            return Err(
                "the component's own timeline chain refused one of its own frame ordinals, \
                 which is the component contradicting itself rather than a client's mistake. \
                 `f_compositor::waits` counts it instead of ending the run, so the run looks \
                 ordinary and this is the only place it is visible",
            );
        }
        // Two independent readings of one fact, required to agree. `late` is the
        // pacing policy's answer — a degradation word that was not *fitted* — and
        // `timeouts` is the trace's: a wait still open on a frame with no room
        // left. A build that decided lateness once and reported it twice could not
        // fail this; a build where the chain stopped landing signals altogether
        // moves one and not the other.
        if self.board.timeouts != self.board.late {
            return Err(
                "the frames the component abandoned are not the frames it found late, though \
                 the two are readings of one fact taken through different numbers — the \
                 degradation word and the frame trace",
            );
        }
        // And the last frame's own state, against the last frame's own word. The
        // trace is one frame's, so the outstanding count is about the frame the
        // degradation word describes and the two have to say the same thing.
        if (self.board.waits != 0) != (Chosen::of(self.board.degraded).latest() != degraded::FITTED)
        {
            return Err(
                "the last frame has a wait outstanding and the component says it fitted, or it \
                 fitted and something is still waiting on it — one of the two words is about a \
                 different frame from the other",
            );
        }
        if self.board.signalled > self.board.frames {
            return Err(
                "the compositor's timeline has reached a value for a frame it never closed, \
                 which is a signal recorded at submission rather than at the landing — the one \
                 distinction `f_abi::sync::Timeline::signalled` exists to keep",
            );
        }
        Ok(())
    }

    /// The half that sleeps, `E3-B01g`.
    ///
    /// **Two exits' worth of clauses and the order is the argument.** The scene
    /// clauses come first, because a doorbell that woke a component which then
    /// applied the wrong frame is a doorbell that proved nothing; then the two
    /// halves of the exit, each with the control that stops it passing for the
    /// wrong reason.
    fn wake_verdict(&self) -> Result<(), &'static str> {
        let mut expected = Expected::of(&script());
        expected.count(&batch_script());

        if self.board.outcome != stopped::TOLD {
            return Err(
                "the component did not end on the frame's stop notice: its outcome word says it \
                 fell out of its loop for a reason of its own — which on this half includes \
                 having stopped its core and never been rung",
            );
        }
        if self.completed != self.submitted || self.refused != 0 {
            return Err(
                "the client did not get one clean completion per entry it submitted, so what \
                 the component published is about a different run from the one that was asked \
                 for",
            );
        }
        if self.board.drained != self.submitted {
            return Err(
                "the component took a different number of entries off the ring than the client \
                 put on it",
            );
        }
        if self.board.refused != 0 || self.board.staged != 0 {
            return Err(
                "the component refused an entry, or ended with a frame still open: every delta \
                 this client sent belongs to a frame that closed",
            );
        }
        if self.board.frames != expected.frames
            || self.board.edits != expected.edits
            || self.board.created != expected.created
            || self.board.removed != expected.removed
            || self.board.live != expected.live()
        {
            return Err(
                "the component's account of what it applied is not what the client's script and \
                 its batch asked for",
            );
        }
        if self.board.named != FRAME_THREE {
            return Err(
                "the last frame the component closed is not the batched one, so the batch \
                 either did not arrive or did not close",
            );
        }

        // --- cross-core delivery, `E3-B01g`'s first clause -------------------
        //
        // Four clauses and every one of them is a control for the next. Two
        // cores, or the word *cross-core* means nothing. A doorbell delivered to
        // the other one, or nothing was sent. A halt on that core, or there was
        // nothing asleep to wake. And a halt that a **doorbell** ended — which
        // is the one a timer would otherwise pass, because a halted core is
        // restarted by any unmasked interrupt and every core running a process
        // has a timer armed.
        if self.bells.worker == self.bells.client {
            return Err(
                "the component ran on the core that rang it, so nothing here is cross-core: \
                 this boot needs a second core and did not get one",
            );
        }
        if self.bells.delivered == 0 {
            return Err(
                "no doorbell was delivered to the core the component ran on, so the client's \
                 commits reached it by polling and the interrupt path was never taken",
            );
        }
        if self.bells.parks == 0 {
            return Err(
                "the core the component ran on never halted, so whatever the doorbell reached \
                 was not a parked compositor",
            );
        }
        if self.bells.woken == 0 {
            return Err(
                "every halt of the component's core ended on something other than a doorbell \
                 — the timer is armed on that core and will end a halt on its own, which is \
                 the reason this count is separate from the halts",
            );
        }
        // The two ends of the same event, counted independently: the component
        // counts what the frame *answered* it, and the frame counts what it
        // *did*. A build where one number was computed from the other could not
        // fail this.
        if self.board.halted != self.bells.parks {
            return Err(
                "the component and the frame disagree about how many times its core was really \
                 stopped, which is one of the two counting the other's answer",
            );
        }
        if self.board.parked != self.bells.parks.saturating_add(self.bells.spared) {
            return Err(
                "the waits the component asked for are not the halts plus the waits a latched \
                 doorbell spared, so a wait went somewhere this boot cannot account for",
            );
        }
        if self.bells.rings == 0 || self.bells.rings > self.bells.operations {
            return Err(
                "the client rang no doorbell at all, or rang more often than it accounted an \
                 operation — either way the suppression figure beside it is not a ratio",
            );
        }

        // --- a batch is one operation and at most one doorbell ---------------
        //
        // `E3-B01g`'s second clause, and the whole of what it has teeth against
        // is a client that charged the doorbell per entry. `BATCH_DELTAS` is
        // four; a build that accounted per entry would report four operations
        // and four rings for one publish, and would then report a *falling*
        // doorbells-per-operation as batch size rose and call that suppression.
        if self.bells.batch_entries != BATCH_DELTAS as u64 {
            return Err("the batch this half publishes is not the size the clause is about");
        }
        if self.bells.batch_operations != 1 {
            return Err("a published batch was charged more than one operation, so \
                 doorbells-per-operation on this channel counts entries and would fall as batch \
                 size rose — which is batching being reported as suppression");
        }
        if self.bells.batch_rings > 1 {
            return Err("one published batch rang more than one doorbell");
        }

        // --- the crossings and the chain, on the half that batches ----------
        //
        // The same relations the serving half is held to, and running them here
        // is the point rather than the tidiness: this half submits twelve entries
        // as **nine publishes**, so it is the only place in this boot where the
        // two units can be told apart at all. On the serving half the client
        // submits one at a time on purpose and the two numbers are equal.
        //
        // **Which clause forbids which, because a mutation found the answer is
        // not the obvious one.** Charging the crossing per publish on the
        // *frame's* side was tried — `seen.submitted += 1` for the batch — and it
        // reddens the completion clause two screens up rather than anything here:
        // a client that counts nine submissions still reaps twelve completions,
        // so `completed != submitted` catches it first. That is a real property
        // and not an accident, and it means what the relations below uniquely
        // forbid is the **component** counting something other than entries —
        // which nothing else in this boot would notice, because the component's
        // total is the only number here that no other clause reads.
        //
        // --- and what this script implies about the chain --------------------
        //
        // The mirror image of the serving half's three, which is why they are
        // worth pinning here as well rather than left to the relations. This half
        // closes **three** frames and the one with no room is the *second*, so the
        // last frame reaches its own value: nothing is outstanding at the end, the
        // timeline stands at the frame count rather than one below it, and the
        // abandoned frame is still counted. A build that left every wait open, or
        // one that landed every signal, moves one of these three and not the
        // others — and neither could be told apart on the serving half, where the
        // abandoned frame is the last one.
        if self.board.waits != 0 {
            return Err(
                "a wait is outstanding at the end of a run whose last frame had a whole scanout \
                 period of room. The batched frame fitted, so the value promised for it was \
                 reached and nothing should still be waiting on it",
            );
        }
        if self.board.signalled != expected.frames {
            return Err(
                "the compositor's timeline did not reach the value it promised for the last \
                 frame, though that frame fitted — or it reached one it never promised. The \
                 abandoned frame is the second of three here, so a timeline that stopped at it \
                 is a build that never lands a signal again after one is skipped",
            );
        }
        if self.board.timeouts != 1 {
            return Err(
                "this half did not find exactly one frame abandoned: the script's second frame \
                 has a nanosecond of room and the other two have a whole scanout period each, \
                 so a component answering the same way for all three is not reading the \
                 deadline",
            );
        }
        // --- `E3-B07d`, on the half that closes three frames -----------------
        //
        // **This is the reading the serving half cannot give.** Three frames,
        // and the one with no room is the *middle* one, so the register
        // alternates: fitted, short, fitted. Two builds survive the serving
        // half two-frame version of this clause and die here — one that decides
        // once, which leaves one answer three times, and one that *latches* its
        // worst answer, which is the shape a reader would write if they thought
        // a degradation was a mode rather than a per-frame choice. Neither can
        // produce a word whose middle field differs from both its neighbours.
        //
        // Out of the **tree** and not the board, which is `E3-B07d` exit
        // sentence: *read out of the component subtree rather than the serial
        // log*. The equality above already requires the two to agree, so reading
        // the tree here costs nothing and says what the exit asks for.
        if Chosen::of(self.tree[8]) != WAKE_REGISTER {
            return Err(
                "the degradation register on this half is not fitted, short, fitted: three \
                 frames closed, the middle one had a nanosecond of room and the other two had \
                 a whole scanout period each, so a compositor choosing per frame leaves an \
                 alternating register. A build that decided once leaves one answer three \
                 times, and one that latched its worst answer never comes back up",
            );
        }
        self.crossings_and_chain_held()
    }

    /// The half that is refused a compositor.
    ///
    /// Five clauses, and the order is the argument. The control first, because
    /// a refusal of every machine says nothing about this one; then the refusal
    /// and the name it carries; then the three that say **nothing was spent** —
    /// which is the difference between *refused a compositor* and *given one
    /// that stopped*, and is the whole of what RFC 0080 asks for here.
    fn floorless_verdict(&self) -> Result<(), &'static str> {
        if !self.backend_admitted {
            return Err(
                "the machine this boot's other halves describe was itself refused a compositor, \
                 so the refusal beside it is a refusal of everything and says nothing about \
                 rungs",
            );
        }
        if self.floorless == 0 {
            return Err(
                "a machine satisfying no rung of RFC 0080's ladder was admitted a compositor: \
                 the floor was handed out as a default, which is the outcome that RFC refuses \
                 in favour of an answerable no",
            );
        }
        if self.floorless != error::pack(error::ADMISSION, error::admission::NO_RUNG) {
            return Err("a machine satisfying no rung was refused a compositor, and not with \
                 ADMISSION/NO_RUNG");
        }
        // Nothing was spent, read three ways: no component was prepared, so no
        // tree was published out of its manifest; no routing page was written,
        // so the word this half's machine would have been described in is zero;
        // and the component never ran, so its board is the default rather than
        // a tally. A build whose refusal came after the address space would
        // pass the three clauses above and fail these.
        if self.tree_nodes != 0 || self.tree_after != 0 || self.tree != [0; WORDS] {
            return Err(
                "a machine refused a compositor had a state tree published for it anyway, so \
                 the refusal came after the component was built rather than before",
            );
        }
        if self.reported != 0 || self.heap != 0 {
            return Err(
                "a machine refused a compositor was described a routing page and a heap, so \
                 the refusal came after the frame had spent them",
            );
        }
        if self.board.drained != 0 || self.board.outcome != 0 || self.submitted != 0 {
            return Err(
                "a machine refused a compositor served a client: something ran, which is a \
                 component that stopped rather than one that was never handed the floor",
            );
        }
        // And the component file was there to be built, which is what keeps
        // this half from passing on a boot carrying no compositor at all.
        if self.image == 0 {
            return Err(
                "this half refused a machine on a boot that carries no compositor image, so \
                 what was refused is not a compositor",
            );
        }
        Ok(())
    }

    /// The refusal half.
    fn mute_verdict(&self) -> Result<(), &'static str> {
        if !self.admitted {
            return Err(
                "the compositor's own manifest was refused admission, so the refusal beside it \
                 says nothing about state trees",
            );
        }
        if self.muted == 0 {
            return Err(
                "a record declaring no state tree was admitted: RFC 0065's refusal did not \
                 happen, and a component that publishes nothing would have been spawned",
            );
        }
        if self.muted != error::pack(error::ADMISSION, error::admission::NO_STATE_TREE) {
            return Err("a record declaring no state tree was refused, and not with \
                 ADMISSION/NO_STATE_TREE");
        }
        Ok(())
    }

    /// The starved half.
    ///
    /// Four clauses, and the last two are the ones worth having: the component
    /// must refuse **before** it takes anything off a ring, and its tree must
    /// still be readable and empty — a component that could not hold a graph is
    /// not a component nobody can read, which is RFC 0065's distinction from the
    /// other side.
    fn starved_verdict(&self) -> Result<(), &'static str> {
        if self.board.outcome != stopped::NO_GRAPH {
            return Err(
                "a component whose heap cannot hold its graph did not end with the outcome \
                 that says so: either it allocated anyway, or it stopped for another reason",
            );
        }
        if self.board.drained != 0 || self.board.frames != 0 {
            return Err(
                "a component that refused its own heap served a client anyway, which is a \
                 compositor answering frames it has nowhere to put",
            );
        }
        if self.tree_nodes as usize != WORDS + 1 || self.tree_before != [0; WORDS] {
            return Err("the tree the frame published for a starved component is not the one its \
                 manifest declares");
        }
        if self.tree != [0; WORDS] {
            return Err("a component that never held a graph published words about one");
        }
        // Including the rung, which is the one word a starved component could
        // plausibly have published: it is chosen in the constructor, and the
        // constructor is reached only after the heap check. A build that decided
        // the rung before asking whether it had a graph would publish a two here
        // and fail the clause above, which is the point of naming it.
        Ok(())
    }

    /// The serving half.
    fn serve_verdict(&self) -> Result<(), &'static str> {
        let expected = Expected::of(&script());
        if self.board.outcome != stopped::TOLD {
            return Err(
                "the component did not end on the frame's stop notice: its outcome word says it \
                 fell out of its loop for a reason of its own",
            );
        }
        if self.completed != self.submitted || self.refused != 0 {
            return Err(
                "the client did not get one clean completion per entry it submitted, so what \
                 the component published is about a different run from the one that was asked \
                 for",
            );
        }
        if self.board.drained != self.submitted {
            return Err(
                "the component took a different number of entries off the ring than the client \
                 put on it",
            );
        }
        if self.board.refused != 0 || self.board.staged != 0 {
            return Err(
                "the component refused an entry, or ended with a frame still open: every delta \
                 this client sent belongs to a frame that closed",
            );
        }
        if self.board.frames != expected.frames
            || self.board.edits != expected.edits
            || self.board.created != expected.created
            || self.board.removed != expected.removed
        {
            return Err(
                "the component's account of what it applied is not what the client's script \
                 asked for",
            );
        }
        // The census, and it is the clause a component that reported its own
        // arithmetic instead of asking the graph would fail: `created` and
        // `removed` are sums of what each commit answered, and this is the
        // arena's own count of what is in it.
        if self.board.live != expected.live() {
            return Err(
                "the graph does not hold the nodes the script left in it, though the applied \
                 counts agree — which is a removal that did not reach the arena",
            );
        }
        if self.board.named != FRAME_TWO {
            return Err("the last frame the component closed is not the last one submitted");
        }

        // --- the frame's story, `E3-B01k` and `E3-B01h` ----------------------
        //
        // Every clause below is checked against something this file knows
        // independently: the rung against RFC 0080's table read by hand, the
        // deadline against the number the client put on the wire, the wake time
        // against the subtraction the exit spells out, and the degradation
        // against a script that was written to have one frame of each kind.
        // --- `E3-B02b`: chosen once, and never promoted ----------------------
        //
        // The frame promoted the report after the first frame closed, so this
        // run had a moment at which a compositor that recomputed its rung would
        // have moved it. That the moment existed is checked first: a write that
        // did not land would leave the clause below green over a run in which
        // nothing was ever offered.
        if self.reported != PROMOTED_CAPABILITIES {
            return Err(
                "the frame's promoted report is not on the routing page at the end of the run, \
                 so the backend never gained a capability and `never promoted` is a clause \
                 nothing in this boot exercised",
            );
        }
        if self.board.rung != HYBRID_RUNG {
            return Err(
                "the component is not holding the rung RFC 0080's table gives the capabilities \
                 it was started with: either a hybrid backend was described and something else \
                 was selected, or the compositor promoted itself when the backend gained a \
                 subgroup scan part way through the run — which is the one thing RFC 0080 \
                 forecloses, because a rung change moves the cost distribution the pacing \
                 estimator is built on",
            );
        }
        if self.board.samples != expected.frames {
            return Err(
                "the pacing window holds a different number of samples than the client closed \
                 frames, so the estimate is over something other than this run's frames",
            );
        }
        if self.board.estimate == 0 {
            return Err(
                "the rolling estimate is zero after two frames, so the component measured \
                 nothing — every step of this boot's clock is at least one nanosecond",
            );
        }
        if self.board.margin != PACING_MARGIN_NANOS {
            return Err("the component did not hold back the margin the frame gave it");
        }
        // The scanout is a boundary of the period the frame declared, and it is
        // the first one *after* the last reading the client wrote. A component
        // that aimed at the boundary it had already reached would produce a wake
        // time in the past for every frame that lands on time.
        if !self.board.scanout.is_multiple_of(SCANOUT_PERIOD_NANOS)
            || self.board.scanout <= self.last_tick
        {
            return Err(
                "the scanout the last frame was paced against is not the first boundary of the \
                 declared period after the clock reading the component was given",
            );
        }
        // `E3-B01h`'s first clause, spelled as the exit spells it. A component
        // whose subtraction dropped a term agrees with itself and fails this.
        if self.board.wake
            != self
                .board
                .scanout
                .saturating_sub(self.board.estimate)
                .saturating_sub(self.board.margin)
        {
            return Err(
                "the wake time is not the scanout less the p99 estimate less the margin, which \
                 is the whole of what E3-B01h computes",
            );
        }
        // The script submits one frame with a whole scanout of room and one with
        // a nanosecond. **One of two, and not two of two**: a component that
        // answered the same way whatever the deadline said would pass every
        // clause above and fail this one, which is the reason the count is
        // published beside the word.
        if self.board.late != 1 {
            return Err(
                "the component did not find exactly one of the two frames late: one was given a \
                 whole scanout period of room and the other a single nanosecond, so a build \
                 that answers the same way for both is not reading the deadline",
            );
        }
        // --- `E3-B07d`: the choice, per frame, and the load it was made under
        //
        // The load first, because a per-frame record of a run that was never
        // overloaded is evidence of nothing. The estimate is what the component
        // measured and the slack is what this script left, so their ratio is the
        // overload — a script rather than a wall clock, which is what makes this
        // clause reproducible from `PACING_SEED` alone.
        if self.board.estimate < FRAME_TWO_SLACK_NANOS.saturating_mul(OVERLOAD_TIMES) {
            return Err("the frame this script meant to overload was not overloaded: what the \
                 component estimates a frame costs here is less than twice the room the second \
                 commit left it, so every clause below is about a compositor that was \
                 comfortable");
        }
        let register = Chosen::of(self.board.degraded);
        if register != SERVE_REGISTER {
            return Err(
                "the degradation register does not carry one answer per frame: the first frame \
                 had a whole scanout period of room and the second a single nanosecond, so a \
                 component deciding per frame leaves two different answers in it. A build that \
                 decided once leaves one answer twice — and before this register existed, such \
                 a build published a word byte-identical to the right one",
            );
        }
        if register.latest() != degraded::SHORT {
            return Err("the last frame was submitted a nanosecond before its deadline and the \
                 component did not answer that it was short: nothing on this wire can declare \
                 an effect, so there is nothing for the policy to give back");
        }
        // And the frame *before* it, which is the half a snapshot cannot carry.
        if register.nth_back(1) != degraded::FITTED {
            return Err(
                "the frame before the last one is not recorded as having fitted, though it was \
                 given a whole scanout period of room — so the register is holding one answer \
                 for every frame rather than each frame its own",
            );
        }
        // Nothing before those two, because nothing before those two closed. A
        // field carrying an answer there would be a component reporting a frame
        // this client never sent, which is what `degraded::NONE` exists to make
        // legible: zero is the absence of a frame and not a frame that fitted.
        if register.nth_back(2) != degraded::NONE {
            return Err(
                "the degradation register carries an answer for a frame that never closed: \
                 this run closed two, and every older field of the register should say that \
                 nothing was recorded there",
            );
        }
        if self.board.deadline != self.submitted_deadline {
            return Err(
                "the deadline the component published is not the one the client put on the \
                 last commit it submitted",
            );
        }

        // --- the published tree ---------------------------------------------
        if self.tree_nodes as usize != WORDS + 1 {
            return Err(
                "the schema the frame published out of the manifest does not carry the nodes \
                 this component writes, plus the subtree they hang under",
            );
        }
        if self.tree_before != [0; WORDS] {
            return Err(
                "the tree already carried a word before the component ran, so a word in it                  afterwards would be evidence of nothing",
            );
        }
        if self.board.published as usize != WORDS {
            return Err(
                "the component wrote fewer words into its tree than it declares nodes: an id it \
                 published is not one its manifest carries",
            );
        }
        // In `node::WRITTEN`'s order, which is the order the tree was read back
        // in. Fifteen words and not four: a board that agreed with a tree about
        // the four old ones and diverged on the rest would be a component with
        // two sets of numbers, which is the failure this comparison exists to
        // catch and the reason every word on the board has a node beside it.
        //
        // Fifteen is also this component's manifest full —
        // `f_abi::manifest::STATE_NODES_MAX` is sixteen and the subtree is one of
        // them — which is why `E3-B01j`'s crossing figure is checked off the board
        // by `Report::crossings_and_chain_held` and has no row here.
        let published = [
            self.board.frames,
            self.board.edits,
            self.board.live,
            self.board.refused,
            self.board.rung,
            self.board.named,
            self.board.deadline,
            self.board.estimate,
            self.board.degraded,
            self.board.resolves,
            self.board.notes,
            self.board.rules,
            self.board.waits,
            self.board.signalled,
            self.board.timeouts,
        ];
        if self.tree != published {
            return Err(
                "the numbers in the component's published tree are not the numbers on its \
                 board, though both come out of one set of counters",
            );
        }
        if self.tree[0] != expected.frames
            || self.tree[1] != expected.edits
            || self.tree[2] != expected.live()
        {
            return Err("the tree a reader would find does not say what this client asked the \
                 compositor to do");
        }
        // And the five the story added, read out of the subtree rather than out
        // of the board — which is `E3-B01k`'s exit sentence, *a boot reads all
        // five back out of the component's subtree rather than out of the serial
        // log*. The equality above already requires the two to agree; these are
        // the clauses that say what the five have to **be**, so a component whose
        // board and tree agreed on five wrong numbers is caught here.
        if self.tree[4] != HYBRID_RUNG
            || self.tree[5] != FRAME_TWO
            || self.tree[6] != self.submitted_deadline
            || self.tree[7] != self.board.estimate
            || Chosen::of(self.tree[8]) != SERVE_REGISTER
        {
            return Err(
                "the five words E3-B01k publishes are not in the component's subtree: a reader \
                 of the tree alone cannot see the rung, the frame, its deadline, what a frame \
                 costs here, or what was given up to fit",
            );
        }
        // --- the resolved theme, `E3-B06d` ----------------------------------
        //
        // **The frame resolves the same theme and compares.** That is the shape
        // of this clause and the reason it is not three equalities against
        // numbers written here. The component holds a `Resolved` produced by
        // `f_interface::token::resolve`; so does this function, from the same
        // constant, after a second process on a second core has finished with
        // it; and the two are required to agree. Three constants written into
        // this file by hand would be a frame checking a component against what
        // the author of the line believed on the afternoon they wrote it, which
        // is what `submitted_deadline` exists to avoid one clause up: *a tree
        // that agreed with the component's board and with nothing outside it
        // would be two copies of one opinion.*
        let (resolved, report) = resolve(&Theme::DEFAULT);
        if self.tree[9] != 1 {
            return Err(
                "the component did not resolve its theme exactly once. RFC 0079 resolves an \
                 ink once per ground, and a build that resolved per frame would answer every \
                 colour question with the same colours and differ only in the sixty-four-step \
                 clamp it spent per ink per ground per frame",
            );
        }
        if self.tree[10] != report.len() as u64 {
            return Err(
                "the notes in the component's tree are not the notes resolving this theme \
                 produces. The count is what makes RFC 0079's clamp auditable rather than \
                 promised, so a component whose report disagrees with the resolver's has \
                 either dropped a decision somebody was owed or invented one",
            );
        }
        if self.board.dropped != 0 {
            return Err(
                "the component's report truncated: notes were counted instead of carried, \
                 which is the one place in the resolver where something happens and nothing \
                 says so",
            );
        }
        if (self.board.clean != 0) != report.is_clean() || (self.tree[10] == 0) != report.is_clean()
        {
            return Err(
                "the resolver's own verdict on the report and the count of notes in it do not \
                 agree, so one of the two is being published without having been read",
            );
        }
        if self.tree[11] != rules_owed(&resolved) {
            return Err(
                "the rules the component says it owes are not the ones this theme's grounds \
                 owe. RFC 0079 hands a compositor exactly one obligation — draw the rule \
                 between two grounds that do not part on their own — and a count that \
                 disagrees with the resolver's own boundary table is that obligation read \
                 wrongly or not at all",
            );
        }
        // --- the synchronisation state, `E3-B05f` ---------------------------
        //
        // The relations are in [`Report::crossings_and_chain_held`] and are not
        // repeated; what is here is what **this script** implies, which is the
        // half a relation cannot supply. The script closes two frames and gives
        // the second a single nanosecond of room, so the compositor never reaches
        // the value it promised for it: the present engine's wait on that value is
        // open at the end of the run, the timeline stands at the first frame's
        // value, and one frame was abandoned.
        //
        // **Read out of the subtree and not off the board**, which is this line's
        // exit sentence and `E3-B01k`'s before it. The equality above already
        // requires the tree and the board to agree; these three are what the three
        // words have to *be*, so a component whose board and tree agreed on three
        // wrong numbers is caught here and nowhere else.
        if self.tree[12] != 1 {
            return Err(
                "the compositor's subtree does not say that a wait is outstanding. The second \
                 frame of this script was submitted a nanosecond before its deadline, so the \
                 value promised for it is never reached and the present engine's wait on it is \
                 the hang `abi/src/trace.rs` exists to make readable — a zero here is a build \
                 that landed a signal for a frame it did not finish",
            );
        }
        // **Against the frame *ordinal* and not against `FRAME_TWO`**, and the
        // difference is worth the sentence because in this script the two are the
        // same number. A timeline value is the count of frames the compositor has
        // finished; a frame token is the client's own opaque word, and this client
        // names its frames one, two and three after their order. A clause written
        // against the token would pass for the wrong reason the day a client names
        // a frame 0x12, which is exactly what `node::FRAME`'s own comment says a
        // token is allowed to be.
        if self.tree[13] != expected.frames.saturating_sub(1) {
            return Err(
                "the compositor's timeline has not stopped one frame short of the frames it \
                 closed. The last of them was abandoned, so a value equal to the frame count is \
                 a build that moves the timeline when it submits a signal rather than when the \
                 signal lands — which would report a frame composited while it was still \
                 queued, and which every other word in this tree would agree with",
            );
        }
        if self.tree[14] != 1 {
            return Err(
                "the compositor's subtree does not count the abandoned frame. One of the two \
                 frames this script sends has no room before its own deadline, so a zero here \
                 is a timeout nothing in this build can reach and a published field that \
                 cannot move — which is the failure E3-B01k was written against",
            );
        }
        if self.tree_after == self.tree_blank {
            return Err(
                "the tree is byte-identical to the one published before the component ran, so \
                 nothing was published at all",
            );
        }
        self.crossings_and_chain_held()
    }
}

/// How many ordered pairs of two *different* grounds do not part on their own.
///
/// # Why the frame counts this rather than asking
///
/// Because what is being checked is a count the *component* took, and a check
/// that called the component's own function would be the component agreeing with
/// itself. `f_compositor::tree` has a function of this shape and this file
/// deliberately does not call it: two loops over one table are two opinions, and
/// their agreeing is the whole of the evidence. It is the argument this file
/// already makes about a `Board` against a published tree, one layer down.
///
/// Six and not nine: a ground over itself is one region rather than a boundary,
/// and counting the diagonal would report that every theme in the tree owes
/// three rules nobody can draw.
///
/// It reads the table `resolve` already filled and evaluates no contrast of its
/// own. `cargo xtask lint-token-pair` is what holds that, and it holds it of this
/// file for the same reason it holds it of the compositor: a colour remembered
/// without the pair it was checked against is defensible against nothing.
/// Unit: none — ordered pairs of grounds.
fn rules_owed(resolved: &Resolved) -> u64 {
    let mut owed = 0;
    for over in Token::ALL {
        for under in Token::ALL {
            if !over.is_ground() || !under.is_ground() || over == under {
                continue;
            }
            if !resolved.boundary(over, under).self_evident {
                owed += 1;
            }
        }
    }
    owed
}

/// Print what happened, one subject per line.
pub fn report_lines(report: &Report) {
    crate::kprintln!(
        "  compositor    a component holding the scene graph at ring 3, and the {} half: {}",
        report.half.name(),
        match report.half {
            Half::Serve =>
                "the client commits two frames of deltas and the component publishes what it \
                 holds",
            Half::Starved =>
                "the same component over a heap two pages long, which must refuse before it \
                 serves anybody",
            Half::Mute =>
                "the same record twice, as declared and with its state declaration emptied",
            Half::Floorless =>
                "a machine below the bottom of RFC 0080's ladder, which must be refused a \
                 compositor before a page is spent",
            Half::Wake =>
                "the component stops its own core between frames and the client on another \
                 core rings it awake",
        }
    );
    match report.half {
        Half::Floorless => {
            crate::kprintln!(
                "  compositor    this machine: {}; a machine satisfying no rung: {} — \
                 ADMISSION/NO_RUNG is {}",
                if report.backend_admitted { "admitted" } else { "REFUSED" },
                report.floorless,
                error::pack(error::ADMISSION, error::admission::NO_RUNG),
            );
            crate::kprintln!(
                "  compositor    nothing spent: {} tree node(s), {} B heap, {} entr(y/ies) \
                 submitted, image_bytes {}",
                report.tree_nodes,
                report.heap,
                report.submitted,
                report.image,
            );
        }
        Half::Mute => {
            crate::kprintln!(
                "  compositor    as declared: {}; declaring no tree: {} — \
                 ADMISSION/NO_STATE_TREE is {}",
                if report.admitted { "admitted" } else { "REFUSED" },
                report.muted,
                error::pack(error::ADMISSION, error::admission::NO_STATE_TREE),
            );
        }
        Half::Serve | Half::Starved | Half::Wake => {
            crate::kprintln!(
                "  compositor    {} entr(y/ies) submitted, {} answered, {} refused, {} drained \
                 by the component",
                report.submitted,
                report.completed,
                report.refused,
                report.board.drained,
            );
            crate::kprintln!(
                "  compositor    {} frame(s) closed, {} delta(s) applied, {} node(s) created, {} \
                 removed, {} live, last frame {}",
                report.board.frames,
                report.board.edits,
                report.board.created,
                report.board.removed,
                report.board.live,
                report.board.named,
            );
            crate::kprintln!(
                "  compositor    state tree {} node(s) from the manifest, {} written by the \
                 component: frames {}, edits {}, nodes {}, refused {}",
                report.tree_nodes,
                report.board.published,
                report.tree[0],
                report.tree[1],
                report.tree[2],
                report.tree[3],
            );
            crate::kprintln!(
                "  compositor    state tree rung {}, frame {}, deadline {} ns, pacing {} ns, \
                 degraded 0x{:x}",
                report.tree[4],
                report.tree[5],
                report.tree[6],
                report.tree[7],
                report.tree[8],
            );
            // The register, newest frame first, one field per frame that closed.
            // Printed beside the packed word rather than instead of it: the word
            // is what the tree carries and what a mutation moves, and the fields
            // are what a reader of this log is actually asking about — *did this
            // compositor decide once, or once per frame*.
            let register = Chosen::of(report.tree[8]);
            crate::kprintln!(
                "  compositor    degraded per frame, newest first: {} {} {} {}",
                register.nth_back(0),
                register.nth_back(1),
                register.nth_back(2),
                register.nth_back(3),
            );
            crate::kprintln!(
                "  compositor    state tree resolves {}, notes {}, rules owed {}; the report \
                 dropped {} and the resolver calls it {}",
                report.tree[9],
                report.tree[10],
                report.tree[11],
                report.board.dropped,
                // Three answers and not two. A component that resolved nothing
                // publishes a zero here, and *dirty* would be a sentence about a
                // report that does not exist — which is exactly what the starved
                // half prints, and what a reader of that half would have had to
                // work out for themselves.
                match (report.tree[9], report.board.clean) {
                    (0, _) => "nothing, because no theme was resolved",
                    (_, 0) => "dirty",
                    _ => "clean",
                },
            );
            crate::kprintln!(
                "  compositor    wake {} ns = scanout {} - p99 {} over {} frame(s) - margin {}; \
                 {} frame(s) late",
                report.board.wake,
                report.board.scanout,
                report.board.estimate,
                report.board.samples,
                report.board.margin,
                report.board.late,
            );
            crate::kprintln!(
                "  compositor    backend reported 0x{:x} at the start, 0x{:x} at the end; rung \
                 {} throughout, and a promoted compositor would say {}",
                BACKEND_CAPABILITIES,
                report.reported,
                report.board.rung,
                PROMOTED_RUNG,
            );
            crate::kprintln!(
                "  compositor    outcome {}, image_bytes {}, heap_bytes {}",
                report.board.outcome,
                report.image,
                report.heap,
            );
            if report.half == Half::Wake {
                crate::kprintln!(
                    "  compositor    doorbell {}: client core {} rang {} of {} operation(s); \
                     core {} was delivered {}",
                    report.bells.path,
                    report.bells.client,
                    report.bells.rings,
                    report.bells.operations,
                    report.bells.worker,
                    report.bells.delivered,
                );
                crate::kprintln!(
                    "  compositor    core {} halted {} time(s), {} of them ended by a doorbell, \
                     {} wait(s) spared by a latched one; the component asked to stop {} time(s) \
                     and was told it halted {}",
                    report.bells.worker,
                    report.bells.parks,
                    report.bells.woken,
                    report.bells.spared,
                    report.board.parked,
                    report.board.halted,
                );
                crate::kprintln!(
                    "  compositor    one batch: {} entr(y/ies), {} operation(s), {} doorbell(s) \
                     — a client charging per entry would say {} and {}",
                    report.bells.batch_entries,
                    report.bells.batch_operations,
                    report.bells.batch_rings,
                    BATCH_DELTAS,
                    BATCH_DELTAS,
                );
            }
            // --- the synchronisation state, `E3-B05f` -----------------------
            //
            // Out of the **subtree** for the three words that have a node and out
            // of the board for the four that do not, printed in that order so a
            // reader sees which is which. The exit's own sentence is *read back at
            // boot out of the component's subtree*, so the log prints where each
            // number came from rather than leaving a reader to assume.
            crate::kprintln!(
                "  compositor    state tree waits outstanding {}, timeline reached {}, \
                 timeout(s) {}; the traces named {} wait(s), dropped {}, and are {}",
                report.tree[12],
                report.tree[13],
                report.tree[14],
                report.board.traced,
                report.board.trace_dropped,
                match (report.board.frames, report.board.trace_complete) {
                    // Three answers and not two, on the shape the theme line one
                    // screen up already uses: a component that closed no frame
                    // traced nothing, and *incomplete* would be a sentence about a
                    // record that does not exist — which is exactly what the
                    // starved half prints.
                    (0, _) => "empty, because no frame closed",
                    (_, 0) => "INCOMPLETE",
                    _ => "complete",
                },
            );
            // --- the boundary crossings, `E3-B01j` --------------------------
            //
            // **Both counts, side by side, and the conversion beside them.** The
            // number is meaningless without saying what a crossing is, so the line
            // prints the two directions that make it and what a client charging
            // per *publish* would have said instead — which on the batching half
            // is a different number and on the serving half is the same one, and a
            // reader who cannot see both cannot tell which unit the figure is in.
            // `E3-B01g` established that printing the conversion is what keeps a
            // count honest, and this is that rule applied to the unit it settled.
            crate::kprintln!(
                "  compositor    {} crossing(s) by the frame's count, {} by the component's: \
                 {} out, {} back, over {} UI frame(s) — {} per frame x1000; per publish a \
                 client would say {}",
                report.submitted + report.completed,
                report.board.crossings,
                report.board.drained,
                report.board.answered,
                report.board.frames,
                report.board.crossings_per_frame_x1000,
                report.bells.operations,
            );
            // And the rows `claims/0038` reads, on the serving half alone.
            //
            // **One half owns them**, for `claims/0037`'s reason stated in
            // `kernel/src/main.rs`: the wake half closes a third frame and batches
            // it, so its rate is a different number, and one row name carrying
            // both values reaches `xtask`'s `measured_rows` as a row printed twice
            // with different numbers — which it refuses rather than averages. The
            // serving half is the one whose workload the claim publishes: a client
            // that submits one delta at a time, which `drive` does deliberately
            // and says so.
            if report.half == Half::Serve {
                crate::kprintln!(
                    "    ring_crossings_per_ui_frame_x1000    {}",
                    report.board.crossings_per_frame_x1000,
                );
                crate::kprintln!(
                    "    ring_crossings_counted_by_the_frame    {}",
                    report.submitted + report.completed,
                );
                crate::kprintln!(
                    "    ring_crossings_counted_by_the_component    {}",
                    report.board.crossings,
                );
                crate::kprintln!(
                    "    ring_entries_client_to_component    {}",
                    report.board.drained
                );
                crate::kprintln!(
                    "    ring_entries_component_to_client    {}",
                    report.board.answered,
                );
                crate::kprintln!("    ui_frames_closed    {}", report.board.frames);
            }
        }
    }
}

/// Find the component file this boot is about, by the name its manifest
/// declares.
///
/// By name and not by position, for `objects::demonstrate`'s reason: the
/// loader's order is the loader's, `user/generation.toml` sorts one way and
/// `xtask`'s `COMPONENTS` another, and a boot that took a module by index would
/// be reading whichever component happened to be built first.
///
/// Visible to the rest of the frame rather than to this file alone, because
/// `kernel/src/input.rs` stands the same component up from the same boot modules
/// and a second finder there would be a second answer to *which module is the
/// compositor* — the exact defect the paragraph above is about, one layer up.
/// It stays private to the crate: nothing outside the frame may name a boot
/// module at all.
///
/// # Safety
///
/// As [`demonstrate`]: the direct map must be live and cover every boot module.
pub(crate) unsafe fn found(boot: &crate::BootInfo) -> Option<(&'static [u8], Record)> {
    // SAFETY: the caller's guarantee.
    let (modules, count) = unsafe { crate::component::modules(boot) };
    for module in modules.iter().take(count) {
        let Ok(record) = Record::read(module) else { continue };
        if record.name.starts_with(b"compositor") && record.name.get(10) == Some(&0) {
            let image = record.image(module).ok()?;
            return Some((image, *record));
        }
    }
    None
}

/// How much heap this record declares, out of the need the frame maps.
///
/// `None` for a record with no such need, which is a compositor with nowhere to
/// put its graph — refused before anything is spent rather than discovered as an
/// allocation that comes back null at ring 3.
///
/// Visible to the rest of the frame for [`found`]'s reason: `kernel/src/input.rs`
/// maps the same heap for the same component and must check it against the same
/// constant.
/// Unit: bytes.
pub(crate) fn heap_declared(record: &Record) -> Option<u64> {
    for need in record.needs() {
        if need.name.starts_with(b"heap") && need.name.get(4) == Some(&0) {
            return Some(need.bytes);
        }
    }
    None
}

/// Stand `user/compositor` up as a server and be its client, or put its record
/// past the admission a spawn performs.
///
/// # Errors
///
/// [`Trouble`], every variant of which fails the boot.
///
/// # Safety
///
/// As [`process::prepare_server`]: `kernel` must be the live kernel space,
/// `frames` its allocator, and `on.cpu` a started, idle core that is not this
/// one. The direct map must be live and cover every boot module.
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    half: Half,
    boot: &crate::BootInfo,
    on: Scheduling,
) -> Result<Report, Trouble> {
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let (image, record) = unsafe { found(boot) }.ok_or(Trouble::NoComponent)?;

    // **The manifest and the crate are required to be one number.** The graph's
    // size is a compile-time fact in `f-compositor` and the heap is a field in a
    // manifest; nothing links the two, so a manifest edited downwards would be a
    // component whose first allocation fails at ring 3 with nothing in the
    // failure naming the cause. Read here and compared, before a page is spent.
    let declared = heap_declared(&record).ok_or(Trouble::NoHeap)?;
    if declared != routing::HEAP_BYTES {
        return Err(Trouble::HeapDisagrees);
    }

    if half == Half::Mute {
        // SAFETY: the caller's guarantee about the boot processor and the
        // allocator, passed down; nothing is running.
        return unsafe { mute(frames, &record, image) };
    }

    // SAFETY: the caller's guarantee, passed down unchanged.
    unsafe { serve(frames, kernel, features, half, &record, image, on) }
}

/// The refusal half: one record, twice, through the admission a spawn performs.
///
/// # Safety
///
/// As [`crate::component::probe_admission`].
unsafe fn mute(
    frames: &mut FrameAllocator,
    record: &Record,
    image: &[u8],
) -> Result<Report, Trouble> {
    // As the manifest declares it. This is the control, and it runs first: a
    // refusal is only evidence about state trees if the same record without the
    // change is admitted.
    // SAFETY: the caller's guarantee.
    let declared = unsafe { crate::component::probe_admission(frames, record, image) }
        .map_err(|_| Trouble::Admission)?;

    // The same record with its declaration emptied and **nothing else changed**,
    // which is what makes this sharper than a second component file carried for
    // the purpose: the only difference between the two probes is the thing under
    // test.
    let mut muted = *record;
    muted.state_nodes = 0;
    // SAFETY: as above.
    let refused = unsafe { crate::component::probe_admission(frames, &muted, image) }
        .map_err(|_| Trouble::Admission)?;

    Ok(Report {
        half: Half::Mute,
        submitted: 0,
        completed: 0,
        refused: 0,
        board: Board::default(),
        tree_nodes: 0,
        tree_blank: 0,
        tree_before: [0; WORDS],
        tree_after: 0,
        tree: [0; WORDS],
        submitted_deadline: 0,
        last_tick: 0,
        admitted: declared.is_ok(),
        muted: refused.err().unwrap_or(0),
        // Not this half's question either way round: no machine was described
        // here and no compositor was stood up, so both words are the ones a
        // half that asked nothing is entitled to.
        backend_admitted: false,
        floorless: 0,
        reported: 0,
        image: image.len() as u64,
        heap: 0,
        // No core, no ring, no doorbell: this half probes a record.
        bells: Bells::default(),
    })
}

/// The serving half.
///
/// # Safety
///
/// As [`demonstrate`].
unsafe fn serve(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    half: Half,
    record: &Record,
    image: &'static [u8],
    on: Scheduling,
) -> Result<Report, Trouble> {
    let Scheduling { tree, cpu, hz, target, tsc_khz } = on;

    // --- RFC 0080's refusal, before a page is spent -------------------------
    //
    // What this half says the machine reports, and whether a machine reporting
    // it may be given a compositor at all. The control goes first and is the
    // same function over this boot's own machine, for `mute`'s reason: a
    // refusal that refused everything would look exactly like this one.
    //
    // **Here rather than after the rings, and the position is the clause.**
    // *Refused a compositor* means nothing was stood up, so the refusal has to
    // come before the frame the channel lives in, before the address space,
    // before the tree — and the verdict below says so by requiring every one of
    // those to be absent. A check further down would refuse a compositor that
    // already existed, which is a compositor that exited.
    let reported = half.reported();
    let backend_admitted = admit_backend(BACKEND_CAPABILITIES).is_ok();
    if let Err(code) = admit_backend(reported) {
        return Ok(Report {
            half,
            submitted: 0,
            completed: 0,
            refused: 0,
            board: Board::default(),
            tree_nodes: 0,
            tree_blank: 0,
            tree_before: [0; WORDS],
            tree_after: 0,
            tree: [0; WORDS],
            submitted_deadline: 0,
            last_tick: 0,
            // Not this half's question: no record was put past the admission a
            // spawn performs, and saying otherwise would put a true-looking
            // word in a report that had not earned it.
            admitted: false,
            muted: 0,
            backend_admitted,
            floorless: code,
            // No page was ever written, so there is nothing to report having
            // carried. The verdict requires this to be zero, which is the same
            // clause as *nothing was spent* read from the page's side.
            reported: 0,
            image: image.len() as u64,
            heap: 0,
            // Nothing was stood up, so nothing could be rung.
            bells: Bells::default(),
        });
    }

    // What the manifest declares, or — on the starved half — two pages, which is
    // the one number in this plan the component is asked to disbelieve.
    let described = match half {
        Half::Starved => STARVED_HEAP_BYTES,
        Half::Serve | Half::Mute | Half::Floorless | Half::Wake => routing::HEAP_BYTES,
    };
    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0))?;

    // The data ring. The frame writes the header and takes the *client's* end;
    // the server's end is the component's, at ring 3. Two ends of one region on
    // two sides of a privilege boundary, which is the whole shape.
    //
    // Allocated here rather than inside `prepare_server` for `ServerPlan::data`'s
    // reason: the frame holds the far end, so this page must not be handed to
    // `reap`.
    let wire = frames.alloc_zeroed(crate::mem::Order::FRAME).ok_or(Trouble::Channel(0))?;
    let at = frames.virt(wire);
    // SAFETY: `wire` was allocated zeroed just above, is frame-aligned — stronger
    // than the cache line the layout asks for — and is `FRAME_SIZE` bytes with no
    // pointer into it held anywhere else.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and every
    // accessor hands out atomics and `UnsafeCell`s rather than references.
    let client_end = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;

    // SAFETY: the caller's guarantee about `kernel`, `frames` and `cpu`, plus
    // `wire` being a frame this function allocated and holds the far end of.
    let (prepared, pages) = unsafe {
        process::prepare_server(
            frames,
            kernel,
            features,
            ServerPlan {
                image,
                selector: life::SERVE,
                tree,
                hz,
                target,
                cpu,
                data: wire.addr(),
                // No client buffer region, and the zero is the manifest's
                // declaration made real: this component's ring carries its
                // payloads `inline`, in the channel's own arena, so a registered
                // region would be a page nobody ever addresses.
                buffers: 0,
                buffer_bytes: 0,
                // What the manifest declares, or — on the starved half — two
                // pages, which is the one number in this plan the component is
                // asked to disbelieve.
                heap_bytes: described,
                // The page this task is about. RFC 0065: the schema is the
                // frame's and the words are the component's.
                own_tree: true,
            },
        )
    }
    .map_err(Trouble::Process)?;

    // The component's own tree, written before its first instruction, out of its
    // own manifest and through the same function the spawn path uses. A
    // component that is prepared and never scheduled still reads as *this
    // component has done nothing* rather than as *this component cannot be read*,
    // which is RFC 0065's distinction and the reason the blank snapshot below is
    // worth taking.
    let tree_nodes = crate::component::publish_tree(pages.own_tree as *mut u8, record)
        .map_err(|_| Trouble::StateTree(0))?;
    let blank = state::Reader::at(pages.own_tree, FRAME_SIZE as u32).map_err(Trouble::StateTree)?;
    let tree_blank = blank.snapshot();
    let mut tree_before = [0u64; WORDS];
    for (slot, id) in tree_before.iter_mut().zip(node::WRITTEN) {
        // `u64::MAX` for an id the schema does not carry, so a manifest and
        // `routing::node` that disagree fail this half's *blank* clause rather
        // than passing it with a zero nobody wrote.
        *slot = blank.value(id).unwrap_or(u64::MAX);
    }

    // SAFETY: `pages.control` is the kernel address of a frame `prepare_server`
    // allocated zeroed for this run and handed to nobody else.
    let control = unsafe {
        Mapping::describe(
            pages.control as *mut u8,
            bytes,
            ENTRIES,
            0,
            feature::CONTROL_EVENTS,
            feature::CONTROL_EVENTS,
        )
    }
    .map_err(Trouble::Channel)?;

    // --- what the component is told -----------------------------------------
    let board = Window::at(pages.board, routing::BYTES).map_err(Trouble::Channel)?;
    for (offset, value) in [
        (at::CONTROL_AT, crate::process::SPAWN_CONTROL),
        (at::CONTROL_LEN, u64::from(bytes)),
        (at::DATA_AT, crate::process::BLK_DATA),
        (at::DATA_LEN, u64::from(bytes)),
        (at::TREE_AT, crate::process::SPAWN_TREE),
        (at::NEGOTIATED_VERSION, u64::from(ABI_VERSION)),
        (at::NEGOTIATED_FEATURES, 0),
        (at::IDLE_SPINS, IDLE_SPINS),
        // What the component needs to pace a frame, and none of it is something
        // a component could have found out for itself: RFC 0004 gives it no
        // clock, nothing tells it about a display, and the margin is a policy.
        // The clock reading below is a placeholder the client overwrites before
        // every entry — it is written here so that a component whose first entry
        // somehow arrived before the first tick reads a number rather than
        // whatever the page held.
        (at::TICK_NANOS, 0),
        (at::SCANOUT_PERIOD_NANOS, SCANOUT_PERIOD_NANOS),
        (at::PACING_MARGIN_NANOS, PACING_MARGIN_NANOS),
        // What this half says the machine reports, and the same word
        // `admit_backend` was given above — one function, so a boot cannot
        // admit one machine and describe another.
        (at::BACKEND_CAPABILITIES, reported),
        // **Whether this frame will ring, which is a statement about the frame
        // and not a mode of the component.** On every half but one it says
        // *nobody rings*, which is what every boot before `E3-B01g` did and is
        // what keeps those halves' numbers comparable with the runs that
        // produced them. On the wake half it says the frame rings, and the
        // component may then stop its core.
        //
        // A component that believed this on a frame that did not ring would
        // hang, which is why it is written here rather than assumed at ring 3:
        // whether an interrupt reaches that core is a fact about a vector, an
        // interrupt controller and a second core, and all three are on this side
        // of the boundary.
        (at::DOORBELL, if half == Half::Wake { bell::RING } else { bell::POLL }),
    ] {
        board.write64(offset, value).map_err(Trouble::Channel)?;
    }
    // The magic last, which is the whole of the discipline: a component that
    // reads a page this loop never finished finds a zero rather than a plausible
    // address.
    board.write64(at::MAGIC, routing::MAGIC).map_err(Trouble::Channel)?;

    // SAFETY: the caller vouched `cpu` is started and idle, and `prepare_server`
    // has written its job. Interrupts are enabled on it, which `run_on`'s
    // contract requires so that a shootdown can be answered.
    unsafe { crate::smp::start_on(cpu) }.map_err(Trouble::Scheduled)?;

    let reaper = Collector::new(client_end.completions()).ok_or(Trouble::Channel(0))?;
    let mut producer = Producer::new(client_end.channel()).ok_or(Trouble::Channel(0))?;
    let notices = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;
    let arena = client_end.arena();

    // --- the doorbell -------------------------------------------------------
    //
    // **The path is selected the way `f_ring::doorbell::Path` says and not by
    // this half's name.** A feature bit is a statement about the protocol and
    // the hardware is a statement about the silicon; conflating them is how a
    // channel gets negotiated into an instruction that faults. What this half
    // varies is the *hardware* half — whether one core may interrupt another —
    // because that is the honest description of the difference between a boot
    // that rings and one that does not: the polling halves are the same code
    // over a machine that cannot ring, which is `Path::Polling`'s own sentence.
    //
    // The ringer carries the core, which is why `f_ring` takes an implementor
    // rather than a function: that crate has no idea what an APIC identifier is
    // and should not acquire one.
    let hardware = Hardware { user_interrupts: false, cross_core_interrupts: half == Half::Wake };
    let path = Path::select(client_end.negotiated(), hardware);
    let mut doorbell = Bell::new(path, hardware, crate::doorbell::Ipi::to(cpu))
        .map_err(|_| Trouble::Channel(0))?;

    // The client, on the half that has one. A starved component ends before it
    // adopts anything, so a client that submitted would be waiting for a
    // completion from a component that has already exited — which is this
    // harness measuring its own bound rather than the refusal it came for.
    // This boot's own clock, and the only one the component will ever see.
    // `PACING_SEED` is the whole of what decides the readings below, which is
    // what lets a pacing estimate be printed in a boot log at all.
    let mut env = SeededEnv::new(PACING_SEED, 0);
    // Named `ends` rather than `wire`, which in this function is already the
    // page the channel lives in. Two things called the same thing one scope
    // apart is how a frame gets freed instead of a struct.
    let ends = Wire { reaper: &reaper, arena: &arena, board: &board };
    let mut batch_seen = Batched::default();
    let driven = match half {
        Half::Serve | Half::Mute => drive(&producer, ends, &mut env, tsc_khz, &mut doorbell, false),
        Half::Wake => {
            // The script first, one entry at a time and each one waited for, so
            // that every submission lands on a core this client has watched stop.
            // Then the third frame, whole, as the one batch the second clause is
            // about.
            drive(&producer, ends, &mut env, tsc_khz, &mut doorbell, true).and_then(|mut seen| {
                batch_seen =
                    drive_batch(&mut producer, ends, &mut env, tsc_khz, &mut doorbell, &mut seen)?;
                Ok(seen)
            })
        }
        Half::Starved | Half::Floorless => {
            Ok(Seen { submitted: 0, completed: 0, refused: 0, deadline: 0, last_tick: 0 })
        }
    };

    // Told to stop whatever happened above, because a component left serving a
    // client that has gone is a core this boot never gets back.
    let told = notices.post(control::entry(control::notice::STOP, 0, 0, 0));
    // **And rung for, on the half where the component may be asleep.** A stop
    // notice goes on the *control* ring, which has no wakeup flag of its own and
    // needs none: a doorbell says only *stop halting*, and which ring had
    // something on it is a question the component answers by looking. A frame
    // that posted the notice and did not ring would leave a parked component
    // holding a core nobody ever gets back — which is the one hang this half can
    // produce, and it would be reported as `Trouble::Overdue` three hundred
    // lines further down with nothing naming the cause.
    //
    // Unconditionally rather than on a `wanted`: there is nothing to suppress,
    // because this is the last thing the client says and the component is
    // required to have ended before the join returns.
    if half == Half::Wake {
        doorbell.submitted(true);
    }
    // SAFETY: `start_on` was called for this core and nothing else has joined it.
    // The closure serves nothing: a compositor reaches no device, so there is
    // nothing it can ask the frame for while it runs.
    let joined = unsafe { crate::smp::join_serviced(cpu, tsc_khz, EXIT_MICROS, &mut || {}) };

    // --- what the other core saw, read once it has stopped running ----------
    //
    // **After the join and not before it, and that is what makes reading another
    // core's counters legal rather than lucky.** `crate::doorbell::delivered_at`
    // is the long form: the mailbox word the join waits on is stored with
    // `Release` by the worker and loaded with `Acquire` here, so everything that
    // core wrote before it reported finished — these four counts included — is
    // visible to this one. No new cross-core word was needed; the rendezvous
    // RFC 0016 already pays for is the rendezvous this reads behind.
    let bells = Bells {
        path: match doorbell.path() {
            Path::Polling => "Polling",
            Path::KernelIpi => "KernelIpi",
            Path::UserInterrupt => "UserInterrupt",
        },
        worker: cpu,
        client: crate::arch::x86_64::current_cpu(),
        operations: doorbell.operations(),
        rings: doorbell.rings(),
        batch_entries: batch_seen.entries,
        batch_operations: batch_seen.operations,
        batch_rings: batch_seen.rings,
        delivered: crate::doorbell::delivered_at(cpu),
        parks: crate::doorbell::parks_at(cpu),
        woken: crate::doorbell::woken_at(cpu),
        spared: crate::doorbell::spared_at(cpu),
    };

    // What the page carries now, which is not what this function wrote into it:
    // `drive` promotes the report after the first frame closes. Read before the
    // address space goes back to the allocator, for the tree's reason.
    let page_reported = board.read64(at::BACKEND_CAPABILITIES).unwrap_or(0);
    let reported_board = Board::of(&board);
    // The tree, read before `reap` gives the page back. RFC 0013's *read, never
    // delivered*, with the frame on the reading end.
    let reader =
        state::Reader::at(pages.own_tree, FRAME_SIZE as u32).map_err(Trouble::StateTree)?;
    let tree_after = reader.snapshot();
    let mut tree = [0u64; WORDS];
    for (slot, id) in tree.iter_mut().zip(node::WRITTEN) {
        *slot = reader.value(id).unwrap_or(0);
    }

    if told.is_err() {
        return Err(Trouble::Channel(0));
    }
    // **Checked before anything is torn down, and that is the order rather than a
    // preference.** A join that did not return is a core still inside this
    // component, and `reap` would give its address space back to the allocator
    // while an instruction pointer was in it. So a boot whose component did not
    // stop leaks one instance and says `Overdue`; the alternative is a frame that
    // frees memory a core is executing in, which is a worse failure and a much
    // quieter one.
    joined.map_err(|_| Trouble::Overdue)?;

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished — which is what the join above returning `Ok` means.
    let _ended = unsafe { process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    // SAFETY: allocated by this function, the component that was lent it has
    // exited — the join is what says so — and nothing else holds a pointer into
    // it.
    unsafe { frames.free(wire) };

    let seen = driven?;
    let board = reported_board.ok_or(Trouble::BadReport)?;

    Ok(Report {
        half,
        submitted: seen.submitted,
        completed: seen.completed,
        refused: seen.refused,
        board,
        tree_nodes,
        tree_blank,
        tree_before,
        tree_after,
        tree,
        submitted_deadline: seen.deadline,
        last_tick: seen.last_tick,
        // Not this half's question. Nothing here admitted anything —
        // `prepare_server` is not a spawn — and claiming otherwise would put a
        // true-looking word in a report that had not earned it.
        admitted: false,
        muted: 0,
        // The machine this half described was admitted a compositor, which is
        // what makes this the positive half of the pair the floorless half
        // completes: the same function, in the same build, over one word.
        backend_admitted,
        floorless: 0,
        reported: page_reported,
        image: image.len() as u64,
        heap: described,
        bells,
    })
}

/// What the client itself saw.
struct Seen {
    /// Entries it put on the ring. Unit: entries.
    submitted: u64,
    /// Completions it reaped. Unit: entries.
    completed: u64,
    /// Completions carrying a refusal, or answering an entry this client was not
    /// waiting for. Unit: entries.
    refused: u64,
    /// The deadline this client put on the last commit it submitted.
    ///
    /// Kept because the component is required to publish it back, and a boot
    /// that took the component's word for what it was sent would be comparing a
    /// number against itself. Unit: nanoseconds, in this boot's seeded epoch.
    deadline: u64,
    /// The last clock reading it wrote into the routing page.
    /// Unit: nanoseconds, in this boot's seeded epoch.
    last_tick: u64,
}

/// Submit the script, one delta at a time, and reap every completion.
///
/// # Why one at a time
///
/// Because the payload travels in the channel's arena and this client writes
/// every payload at the same offset. A second entry submitted before the first
/// was answered would overwrite bytes the component had not read yet, and the
/// frame it belonged to would lose an entry — which the component would refuse
/// correctly, and the boot would then be measuring this client's own mistake.
///
/// It is also what this task is allowed to do. Batching deltas into one crossing
/// is `E3-B01`'s exit — boundary crossings per UI frame under ten — and
/// `E3-B01j` is the task that counts them on both sides. A client that batched
/// here would be building that task's evidence without its counter.
///
/// # Errors
///
/// [`Trouble::Refused`] where the ring will not take an entry, or where a
/// completion does not arrive inside the bound.
fn drive(
    producer: &Producer<'_>,
    wire: Wire<'_, '_>,
    env: &mut SeededEnv,
    tsc_khz: u64,
    doorbell: &mut Bell<crate::doorbell::Ipi>,
    waited: bool,
) -> Result<Seen, Trouble> {
    let Wire { reaper, arena, board } = wire;
    let mut seen = Seen { submitted: 0, completed: 0, refused: 0, deadline: 0, last_tick: 0 };
    let mut parked = 0;
    for mut delta in script() {
        // **On the half that sleeps, wait for it to be asleep.** Without this
        // the boot would still be correct — the ring's arm-look-sleep and the
        // frame's latch between them mean nothing is ever lost — and it would
        // be evidence of nothing in particular, because *a client's commit woke
        // a parked compositor* would be a likely interleaving rather than a
        // thing that happened. [`waited_for_park`] says what it costs and what
        // it does not buy.
        if waited {
            parked = waited_for_park(board, tsc_khz, parked)?;
        }

        // --- the clock, written before the entry it belongs to --------------
        //
        // **The order is the whole of why the component's measurements mean
        // anything, and it is an ordering argument rather than a convenience.**
        // The reading goes into the page, then the entry goes on the ring with a
        // `Release` publish, and the component takes it with an `Acquire` — so a
        // reading written here is visible to the component before the entry that
        // was written after it, and the component reads the page only once it
        // has an entry in hand. Neither side spins on the other and neither
        // needs to: the ring's own pair is what orders them, which is the same
        // pair `ring/src/lib.rs` rests every payload byte on.
        //
        // The step is drawn rather than fixed, so the two frames of this script
        // cost different amounts and a percentile is a percentile of something.
        // One is added to every draw, so a step is never zero — which is what
        // makes *the second frame does not fit in one nanosecond* an arithmetic
        // fact about this script rather than a property of this seed.
        let step_nanos = 1 + env.next_u64() % STEP_SPREAD_NANOS;
        env.advance(step_nanos);
        let now = env.now().as_nanos();
        if board.write64(at::TICK_NANOS, now).is_err() {
            return Err(Trouble::Refused);
        }
        seen.last_tick = now;

        // A commit's deadline is a slack in `script` and an instant on the wire,
        // and this is where it becomes one. Adding *this* reading rather than a
        // later one is what makes the room the frame had exact: the component
        // reads the same number, so what it has left at the commit is the slack
        // to the nanosecond.
        if matches!(delta.body, Entry::Commit(_)) {
            delta.deadline = now.saturating_add(delta.deadline);
            seen.deadline = delta.deadline;
        }

        let (entry, payload) = delta.encode();
        if !arena.copy_in(0, &payload) {
            return Err(Trouble::Refused);
        }
        let wanted = match producer.submit(entry) {
            Ok(wanted) => wanted,
            Err(_) => return Err(Trouble::Refused),
        };
        seen.submitted += 1;
        // One entry is one operation, and it rings only if the component said
        // it was about to sleep — which on the polling halves it never does,
        // because it was told nobody rings and never arms the flag. So the same
        // call on every half accounts the same operations and sends no
        // interrupt where none was asked for.
        doorbell.submitted(wanted);

        // Waited for, and bounded rather than spun on: a run that reaches the
        // bound has a component that is not making progress, which is a
        // different failure from one that is slow.
        let deadline = crate::smp::deadline_after(tsc_khz, EXIT_MICROS);
        loop {
            match reaper.take() {
                Ok(Some(answer)) => {
                    seen.completed += 1;
                    if answered_badly(&answer, entry.user_data) {
                        seen.refused += 1;
                    }
                    break;
                }
                Ok(None) => {
                    if crate::smp::past(deadline) {
                        return Err(Trouble::Refused);
                    }
                    core::hint::spin_loop();
                }
                Err(_) => return Err(Trouble::Refused),
            }
        }

        // --- the backend gains a capability, mid-run ------------------------
        //
        // `E3-B02b`: *the rung is chosen once, at start, and never promoted*.
        // The first frame has closed and been answered, so the component has
        // long since read its report and chosen; now the frame writes a
        // **better** one into the same word — the hybrid's set plus a usable
        // subgroup scan, which RFC 0080's table gives the top rung — and the
        // verdict requires the rung the component publishes at the end to be
        // the one it started with.
        //
        // **Why after the completion rather than before the frame.** A report
        // promoted before the component had read the first one would be a boot
        // that described one machine and asked about another; what this task
        // needs is a component that has already chosen, offered a better answer
        // afterwards. The completion is what says the component has been
        // through `Held` at least once.
        //
        // What this is *not* is a mechanism. Nothing in `user/compositor` reads
        // this word twice, and that is the property under test rather than an
        // omission: a build that added the second read would publish
        // `PROMOTED_RUNG` here and go red on the serve verdict, which is the
        // only way a negative clause gets a run behind it.
        if let Entry::Commit(commit) = delta.body
            && commit.frame_token == FRAME_ONE
            && board.write64(at::BACKEND_CAPABILITIES, PROMOTED_CAPABILITIES).is_err()
        {
            return Err(Trouble::Refused);
        }
    }
    Ok(seen)
}

/// The client's end of the channel, and the page beside it.
///
/// Three references that travel together and always will — a completion ring,
/// the arena a payload is staged in, and the routing page the component reads
/// its clock off. `Scheduling` one file up made the same move for the same
/// reason: threading them separately is what put two functions here past
/// clippy's argument bound, and a bound that is routed around with an `allow`
/// is a bound nobody is keeping.
///
/// The producer is **not** in it, and that is not tidiness: [`drive`] holds it
/// shared and [`drive_batch`] holds it exclusively, because `Producer::batch`
/// takes `&mut self` so that a submission cannot interleave with a batch that is
/// still being filled. Putting it here would make that a runtime rule again.
#[derive(Clone, Copy)]
struct Wire<'a, 'm> {
    /// Where completions are reaped.
    reaper: &'a Collector<'m>,
    /// Where a payload is staged before its entry is published.
    arena: &'a Arena<'m>,
    /// The page the client writes its clock into and reads the component's
    /// park count out of.
    board: &'a Window,
}

/// What one published batch cost.
///
/// Three numbers because the clause is a comparison between them: entries is
/// what went on the ring, operations is what the doorbell was charged, and rings
/// is what it sent. A build that charged per entry moves the first two together
/// and that is the failure the clause exists to catch.
#[derive(Clone, Copy, Default)]
struct Batched {
    /// Entries staged and published. Unit: entries.
    entries: u64,
    /// Operations the doorbell accounted for them. Unit: operations.
    operations: u64,
    /// Doorbells it sent. Unit: doorbells.
    rings: u64,
}

/// Wait until the component's own park count has moved past `was`.
///
/// **A timing observation, and nothing rests on it.** The word is written
/// volatilely by the component on one core and read volatilely here on another,
/// exactly as `at::TICK_NANOS` already is in the other direction; a reading that
/// was stale would make this client ring early, and the ring's arm-look-sleep
/// and the frame's wakeup latch would absorb that without losing an entry. What
/// it buys is that the boot's own sentence is about a run: the submission that
/// follows lands on a core this client has watched stop.
///
/// # Errors
///
/// [`Trouble::NotParked`] where the count did not move inside the bound, which
/// is its own variant because it is a failure of this harness's patience and not
/// of the component — the two would otherwise be one red line.
fn waited_for_park(board: &Window, tsc_khz: u64, was: u64) -> Result<u64, Trouble> {
    let deadline = crate::smp::deadline_after(tsc_khz, EXIT_MICROS);
    loop {
        let now = board.read64(reported::PARKED).unwrap_or(was);
        if now > was {
            return Ok(now);
        }
        if crate::smp::past(deadline) {
            return Err(Trouble::NotParked);
        }
        core::hint::spin_loop();
    }
}

/// Publish the third frame as one batch, and account it as one operation.
///
/// **`E3-B01g`'s second clause, and the reason the payloads go to different
/// offsets.** [`drive`] writes every payload at offset zero because it waits for
/// a completion before the next entry, so the bytes are read before they are
/// overwritten. A batch cannot do that: four entries become visible with one
/// store and the component may read any of them in any order, so each needs its
/// own bytes. The stride is `PAYLOAD_BYTES`, which is the stride
/// `abi/src/scene.rs` fixes for exactly this reason.
///
/// # Errors
///
/// [`Trouble::Refused`] where the ring will not take the batch or a completion
/// does not arrive inside the bound, and [`Trouble::NotParked`] where the
/// component never stopped its core.
fn drive_batch(
    producer: &mut Producer<'_>,
    wire: Wire<'_, '_>,
    env: &mut SeededEnv,
    tsc_khz: u64,
    doorbell: &mut Bell<crate::doorbell::Ipi>,
    seen: &mut Seen,
) -> Result<Batched, Trouble> {
    let Wire { reaper, arena, board } = wire;
    // Asleep first, as [`drive`]'s own submissions are, and for the same reason.
    // The count is read fresh rather than carried in, because every entry above
    // moved it and what this needs is one more park after the last of them.
    let was = board.read64(reported::PARKED).unwrap_or(0);
    waited_for_park(board, tsc_khz, was)?;

    // One reading for the whole batch, written before any of it goes on the
    // ring. Four entries published by one store are one moment, and giving them
    // four readings would be this client inventing an ordering the ring does not
    // have.
    let step_nanos = 1 + env.next_u64() % STEP_SPREAD_NANOS;
    env.advance(step_nanos);
    let now = env.now().as_nanos();
    if board.write64(at::TICK_NANOS, now).is_err() {
        return Err(Trouble::Refused);
    }
    seen.last_tick = now;

    let mut expected = [0u64; BATCH_DELTAS];
    let mut batch = producer.batch();
    for (ix, mut delta) in batch_script().into_iter().enumerate() {
        // Its own bytes, at its own offset, and the entry is told where they
        // are. A batch whose entries all named offset zero would decode four
        // times into whichever payload was written last.
        let at_offset = ix * PAYLOAD_BYTES;
        delta.payload_offset = at_offset as u32;
        if matches!(delta.body, Entry::Commit(_)) {
            delta.deadline = now.saturating_add(delta.deadline);
            seen.deadline = delta.deadline;
        }
        let (entry, payload) = delta.encode();
        if !arena.copy_in(at_offset, &payload) {
            return Err(Trouble::Refused);
        }
        expected[ix] = entry.user_data;
        if batch.push(entry).is_err() {
            return Err(Trouble::Refused);
        }
    }

    // **One store, one operation, at most one doorbell.** Everything above this
    // line is invisible to the component; everything after it has already
    // happened as far as the component is concerned. That is what makes the
    // accounting below a rule rather than a convention.
    let before = doorbell.operations();
    let rang = doorbell.rings();
    let wanted = batch.publish().map_err(|_| Trouble::Refused)?;
    doorbell.submitted(wanted);
    let batched = Batched {
        entries: BATCH_DELTAS as u64,
        operations: doorbell.operations() - before,
        rings: doorbell.rings() - rang,
    };
    seen.submitted += BATCH_DELTAS as u64;

    // Reaped in whatever order they come back, matched by the word each entry
    // carried. A client that assumed the order would be asserting something the
    // ring does not promise, and would pass on a component that answered the
    // batch backwards.
    let deadline = crate::smp::deadline_after(tsc_khz, EXIT_MICROS);
    let mut taken = 0;
    while taken < BATCH_DELTAS {
        match reaper.take() {
            Ok(Some(answer)) => {
                seen.completed += 1;
                taken += 1;
                if answer.result < 0 || !expected.contains(&answer.user_data) {
                    seen.refused += 1;
                }
            }
            Ok(None) => {
                if crate::smp::past(deadline) {
                    return Err(Trouble::Refused);
                }
                core::hint::spin_loop();
            }
            Err(_) => return Err(Trouble::Refused),
        }
    }
    Ok(batched)
}

/// Is this completion anything other than *the entry it names was accepted*?
///
/// Two things at once on purpose: a refusal, and an answer to a different entry.
/// A client that checked only the result would count a completion for entry
/// three as the answer to entry four and never notice.
fn answered_badly(answer: &Cqe, expected: u64) -> bool {
    answer.result < 0 || answer.user_data != expected
}
