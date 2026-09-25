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
//! one — tell it where its rings are, submit two frames' worth of scene deltas,
//! reap the completions, and then read what the component published and judge
//! it against what was asked for.
//!
//! # Where the component runs, since RFC 0129
//!
//! **In its place.** The three halves that stand a compositor up are served by
//! `component::demonstrate` against the occupant of the compositor's own place,
//! with [`Placed`] as the client, so the instance a supervisor stops for a
//! timeout is the instance whose tree the reading came from. Until RFC 0129 this
//! file stood a compositor up *beside* the place with `process::prepare_server`,
//! and the reading on the supervisor's row was that instance's while the
//! occupant stopped and restarted was the place's own, which had never run — RFC
//! 0126's narrowing. The reason it ran beside was not the one the next section
//! gave: the manifest declared no `board` and no `data` need, so a spawned
//! occupant had nowhere to be told where its rings were. [`Placed`] says it at
//! length; a manifest without them is refused by name in the lifecycle, and
//! nothing falls back to a stand-up.
//!
//! What moved is who built the instance. The script, the board, the clock, the
//! doorbell, the stop and every verdict below are unchanged; the place's account
//! paid for every page, the frame's root mounts the tree, and two words say
//! which instance ran — [`Report::served`].
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
//! Because there is no other client. Nothing in this build holds the far end of
//! a scene ring but the frame — the arrangement every datapath boot in this tree
//! has. The sentence that stood here said the supervisor did not hand a place's
//! occupant a core *and* a peer; `component::Datapath` has done that for the
//! block place since `E1-B05`, and it is how this file's client now reaches the
//! compositor's place. What is left of the sentence is the client: the peer is
//! still the frame.
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
use crate::process;

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
///
/// **Described, and since RFC 0129 no longer mapped at this size.** A place maps
/// the heap its manifest declares out of its own account, so the starved
/// occupant has the whole of it mapped and two pages *described* — the prologue
/// is the frame's to write and is what the component reads. That is a narrowing
/// and it is named: this half now proves the component refuses on the size it is
/// told, not on the size it could touch. *What would reverse this:* a place that
/// maps a heap smaller than its manifest declares, which is a spawn taking a
/// quantity from somewhere other than the record — or a second component file
/// whose manifest declares two pages.
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
    /// Send one frame past the cap the compositor's manifest declares, and one
    /// frame under it.
    ///
    /// **`E3-B07e`'s boot, and RFC 0128's *Consequences* is what it shows.** The
    /// frame found the cap on the compositor's `scene` server ring and wrote it
    /// onto the routing page; the client sends [`CAPPED_DELTAS`] creations and a
    /// commit, and requires the cap's worth applied, the rest answered
    /// `RESOURCE/QUOTA_EXHAUSTED` with the cap as the detail, the commit accepted
    /// and the frame closed on the rung it started on. Then [`UNCAPPED_DELTAS`]
    /// creations and a commit, every one applied — which is the clause that says
    /// the count is the frame's and not the run's.
    ///
    /// **Its own half rather than frames added to the serving one**, because the
    /// serving half's script is `claims/0038`'s workload and its bounds are
    /// exact: two frames and sixteen crossings. Seventy more entries there would
    /// be a claim re-declared to make room for a different subject.
    Capped,
    /// Build the representative scene through the reconciler, then play it:
    /// `E3-B01`'s own count.
    ///
    /// **The parent's exit, and the one half whose frames were not written for
    /// the test they are in.** `claims/0033-scene/scene.toml` — an audio
    /// timeline, 995 nodes, three of them dirty when the playhead moves — is
    /// rebuilt whole every frame by `f_compositor::timeline`, `f_scene`'s
    /// reconciler turns each rebuild into the deltas that differ, and [`drive`]
    /// puts every one of them on the ring. Forty frames build the scene under
    /// the manifest's cap; eight more move the playhead, and those eight are
    /// the UI frames RFC 0133 says `E3-B01`'s sentence is about.
    ///
    /// **Its own half rather than frames added to the serving one**, for the
    /// capped half's reason: the serving half's script is `claims/0038`'s
    /// workload and its bounds are exact.
    Timeline,
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
            Self::Capped => "capped",
            Self::Timeline => "timeline",
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
            Self::Serve
            | Self::Starved
            | Self::Mute
            | Self::Wake
            | Self::Capped
            | Self::Timeline => BACKEND_CAPABILITIES,
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
    /// A half that stands a compositor up was asked of the path that stands none
    /// up, or the other way round.
    ///
    /// **Its own variant because there is no second path, and a fallback is the
    /// defect.** Since RFC 0129 the serving halves are served from the
    /// compositor's place by the component lifecycle, and nothing in this file
    /// stands one up beside the place — so a boot that reached here for one had
    /// its halves routed wrong, and a stand-up here would put RFC 0126's gap
    /// back green.
    Placed,
    /// The lifecycle never served the place this client was built for, so there
    /// is nothing to judge.
    ///
    /// A red line and not an empty report: a boot that asked for a compositor
    /// half and never ran it would otherwise print a verdict over zeroes.
    NotServed,
    /// The compositor's record serves no ring speaking
    /// `f_abi::manifest::FRAMED_PROTOCOL`, so there is no cap to hand it.
    ///
    /// **RFC 0128's *a protocol renamed walks past both rules*, closed here.**
    /// The manifest checker and the frame's reader both key on a server ring
    /// whose protocol is exactly `scene`, so a manifest saying `scenes` is
    /// uncapped and refused by neither. The frame is where the cap is read, so
    /// the frame is where its absence is refused — before a page is spent,
    /// beside the heap check — and the rename is a red boot rather than a
    /// compositor serving without a quota.
    Unframed,
    /// The timeline half's client could not have the frames its reconciler and
    /// its tree live in. `E3-B01`.
    NoClientMemory,
    /// The timeline half's reconciler refused one of its own frames, or the cap
    /// the manifest declares is too small to build the scene in the frames this
    /// client records. The client's own tree being wrong, and red rather than
    /// skipped: a frame the reconciler refused is a frame whose crossings were
    /// never counted. `E3-B01`.
    Reconciled,
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
            Self::Placed => {
                "a serving half is served from the compositor's place and there is no second \
                 path that stands one up beside it"
            }
            Self::NotServed => {
                "the compositor's place was never served, so there is no run to judge — the \
                 component lifecycle did not reach it"
            }
            Self::Unframed => {
                "the compositor's record serves no `scene` ring, so the frame has no \
                 deltas_per_frame_max to hand it and will not start it uncapped (RFC 0128)"
            }
            Self::NoClientMemory => {
                "the timeline client could not have the frames its reconciler and its tree \
                 live in"
            }
            Self::Reconciled => {
                "the timeline client's reconciler refused one of its own frames, or the cap is \
                 too small to build the scene in the frames this client records"
            }
        }
    }
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
    /// Which occupant of the compositor's place was served, and the physical
    /// page its tree is — `None` on the two halves that stand nothing up.
    ///
    /// `E3-B05e`'s identity, from the client's side. The component reports the
    /// epoch it read off its own control ring (`Board::epoch`) and the verdict
    /// requires the two to agree; the page is printed so `cargo xtask
    /// compositor` can hold it equal to the page the lifecycle copied the
    /// supervisor's reading from and the page it unmounted when it stopped the
    /// occupant. RFC 0129.
    ///
    /// **The page is walked, not remembered** — `Wired::tree_mapped`, the
    /// translation the occupant's own page tables give its tree address — while
    /// the lifecycle's two lines print its bookkeeping and its root's mount word.
    /// So the harness's page comparison is two readings of where the tree is.
    /// What it is not is an instance identity: a refill is spawned out of the
    /// account its predecessor's frames were refunded to and can land on the
    /// same page, which is why [`Report::refilled`] rests on the epoch.
    /// Unit: instances, and bytes, physical.
    pub served: Option<(u32, u64)>,
    /// The place's next occupant, served after the supervisor restarted it —
    /// on the serving half, which is the one half whose place is restarted, and
    /// `None` on every other.
    ///
    /// **What makes [`Report::served`]'s identity able to fail.** That one is
    /// epoch zero against epoch zero, which agree whether or not the component
    /// reads its ring; this one is epoch one, so a component reporting a
    /// constant says the wrong occupant here. [`Refill`] is the argument.
    pub refilled: Option<Refilled>,
    /// The cap the frame read off the compositor's `scene` server ring and wrote
    /// onto its routing page, `E3-B07e`. Zero on the two halves that stand
    /// nothing up. Unit: deltas per frame.
    pub cap: u64,
    /// The cap the compositor's record declares as compiled: [`Report::cap`]
    /// on every run but the recapped one, where the frame wrote [`RECAPPED`]
    /// and this says what it was moved from. Zero on the two halves that stand
    /// nothing up. Unit: deltas per frame.
    pub declared: u64,
    /// Completions the client reaped that said `RESOURCE/QUOTA_EXHAUSTED` with
    /// [`Report::cap`] as the detail, by the frame they were sent in — the
    /// first commit this client submitted closes index zero, and anything past
    /// the last index is counted in it.
    ///
    /// **Per frame, because the clause is per frame.** A total of ten would be
    /// satisfied by a compositor that refused five of the sixty and five of the
    /// eight, and only the split says the count went back to zero at the commit.
    /// Unit: completions.
    pub capped: [u64; CAPPED_FRAMES],
    /// What the timeline half recorded at every commit, `E3-B01`; nothing on
    /// every other half.
    pub timeline: Timeline,
}

impl Report {
    /// A report of nothing having been stood up: every count zero, no board, no
    /// tree, no bells, nothing admitted — what a half that asked nothing is
    /// entitled to, which each half then overrides for what it did ask.
    fn nothing(half: Half, image: u64) -> Self {
        Self {
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
            admitted: false,
            muted: 0,
            backend_admitted: false,
            floorless: 0,
            reported: 0,
            image,
            heap: 0,
            bells: Bells::default(),
            served: None,
            refilled: None,
            cap: 0,
            declared: 0,
            capped: [0; CAPPED_FRAMES],
            timeline: Timeline::NOTHING,
        }
    }
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

    // --- the late latch, `E3-B01i` ------------------------------------------
    //
    // Both ends of the transform and neither difference, which is
    // `f_compositor::latch::Latched`'s own rule: a component that published its
    // own subtraction could not be checked against itself, and the frame that
    // does the subtracting here is the one that injected the motion. The entry
    // ordinal is the word that says *where in the frame* — zero is after the
    // commit closed and before the compositor's own submission entered its wait.
    /// Positions the device reported that the component took. Unit: reports.
    pub pointer_reports: u64,
    /// Positions its predictor refused as not newer than the newest it held.
    /// Unit: reports.
    pub pointer_stale: u64,
    /// Readings it refused because they carried no stamp, which is what a page
    /// the frame has not written reads as. Unit: readings.
    pub pointer_unstamped: u64,
    /// Frames that carried a latch. Unit: frames.
    pub latches: u64,
    /// Frames that did not, whatever the reason. Unit: frames.
    pub latch_declines: u64,
    /// How many wait entries the last latched frame's trace held at the latch.
    /// Unit: wait entries.
    pub latch_entry: u64,
    /// The translation the client committed on that frame, along x.
    /// Unit: device pixels, scaled by 65 536, as a two's-complement `u64`.
    pub latch_committed_x: u64,
    /// And along y. Unit: as [`Board::latch_committed_x`].
    pub latch_committed_y: u64,
    /// The translation that was submitted on that frame, along x.
    /// Unit: as [`Board::latch_committed_x`].
    pub latch_x: u64,
    /// And along y. Unit: as [`Board::latch_committed_x`].
    pub latch_y: u64,
    /// How far forward the prediction was extrapolated. Unit: nanoseconds.
    pub latch_lead_nanos: u64,
    /// The instant the last latch was aimed at, published so that a second
    /// predictor can be asked the same question. Unit: nanoseconds.
    pub latch_aim_nanos: u64,
    /// One where that position was an extrapolation rather than the last
    /// position the device reported. Unit: none — a flag.
    pub latch_extrapolated: u64,

    // --- the input ring, `E3-B04g` ------------------------------------------
    //
    // What the component took off the input driver's ring **itself**, with this
    // frame holding neither end of it. Read here and judged in
    // `kernel/src/input.rs`, which is the one boot that connects the ring; every
    // other boot reads `input_connected` zero and the rest zero beside it.
    /// One where the component adopted an input ring. Unit: none — a flag.
    pub input_connected: u64,
    /// Entries it took that were not the driver's attestation. Unit: entries.
    pub input_entries: u64,
    /// Of those, entries the decoder took. Unit: entries.
    pub input_decoded: u64,
    /// Of those, entries the decoder refused. Unit: entries.
    pub input_refused: u64,
    /// Of the decoded ones, pointer motion. Unit: entries.
    pub input_motions: u64,
    /// The component's fold over what it drained. Unit: none — a checksum.
    pub input_crossing: u64,
    /// How many entries went into it. Unit: entries.
    pub input_crossed: u64,
    /// Attestations it took off the ring. Unit: entries.
    pub input_attestations: u64,
    /// The driver's word as it arrived on the ring. Unit: none — a checksum.
    pub input_attested: u64,
    /// The count the driver attested beside it. Unit: entries.
    pub input_attested_count: u64,
    /// Entries taken after the attestation. Unit: entries.
    pub input_unattested: u64,
    /// One where the component's own comparison agreed. Unit: none — a flag.
    pub input_agreed: u64,
    /// The last latched frame's graph, folded with the pointer's translation
    /// masked, before the patch. Unit: none — a checksum.
    pub latch_unmoved_before: u64,
    /// The same fold after it. Unit: none — a checksum.
    pub latch_unmoved_after: u64,
    /// How many nodes that fold walked. Unit: nodes.
    pub latch_walked: u64,
    /// Which occupant of its place the component is, as it read the epoch off
    /// its own control ring's header, plus one — zero for a run that adopted no
    /// control ring. `reported::EPOCH` argues the word. Unit: none — an epoch
    /// plus one.
    pub epoch: u64,
    /// Deltas it refused because their frame had staged its cap, `E3-B07e`.
    /// Unit: deltas.
    pub capped: u64,
    /// Latched frames whose patch it took back out of the graph, RFC 0131.
    /// Unit: frames — UI frames.
    pub restores: u64,
    /// Latched frames whose restore the graph refused. Unit: frames — UI frames.
    pub unrestored: u64,
    /// The pointer node's translation along x as the graph held it when the run
    /// ended. Unit: device pixels, scaled by 65 536, as a two's-complement `u64`.
    pub latch_held_x: u64,
    /// The same along y. Unit: as [`Board::latch_held_x`].
    pub latch_held_y: u64,
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
            pointer_reports: board.read64(reported::POINTER_REPORTS).ok()?,
            pointer_stale: board.read64(reported::POINTER_STALE).ok()?,
            pointer_unstamped: board.read64(reported::POINTER_UNSTAMPED).ok()?,
            latches: board.read64(reported::LATCHES).ok()?,
            latch_declines: board.read64(reported::LATCH_DECLINES).ok()?,
            latch_entry: board.read64(reported::LATCH_ENTRY).ok()?,
            latch_committed_x: board.read64(reported::LATCH_COMMITTED_X).ok()?,
            latch_committed_y: board.read64(reported::LATCH_COMMITTED_Y).ok()?,
            latch_x: board.read64(reported::LATCH_X).ok()?,
            latch_y: board.read64(reported::LATCH_Y).ok()?,
            latch_lead_nanos: board.read64(reported::LATCH_LEAD_NANOS).ok()?,
            latch_aim_nanos: board.read64(reported::LATCH_AIM_NANOS).ok()?,
            latch_extrapolated: board.read64(reported::LATCH_EXTRAPOLATED).ok()?,
            input_connected: board.read64(reported::INPUT_CONNECTED).ok()?,
            input_entries: board.read64(reported::INPUT_ENTRIES).ok()?,
            input_decoded: board.read64(reported::INPUT_DECODED).ok()?,
            input_refused: board.read64(reported::INPUT_REFUSED).ok()?,
            input_motions: board.read64(reported::INPUT_MOTIONS).ok()?,
            input_crossing: board.read64(reported::INPUT_CROSSING).ok()?,
            input_crossed: board.read64(reported::INPUT_CROSSED).ok()?,
            input_attestations: board.read64(reported::INPUT_ATTESTATIONS).ok()?,
            input_attested: board.read64(reported::INPUT_ATTESTED).ok()?,
            input_attested_count: board.read64(reported::INPUT_ATTESTED_COUNT).ok()?,
            input_unattested: board.read64(reported::INPUT_UNATTESTED).ok()?,
            input_agreed: board.read64(reported::INPUT_AGREED).ok()?,
            latch_unmoved_before: board.read64(reported::LATCH_UNMOVED_BEFORE).ok()?,
            latch_unmoved_after: board.read64(reported::LATCH_UNMOVED_AFTER).ok()?,
            latch_walked: board.read64(reported::LATCH_WALKED).ok()?,
            epoch: board.read64(reported::EPOCH).ok()?,
            capped: board.read64(reported::CAPPED).ok()?,
            restores: board.read64(reported::RESTORES).ok()?,
            unrestored: board.read64(reported::UNRESTORED).ok()?,
            latch_held_x: board.read64(reported::LATCH_HELD_X).ok()?,
            latch_held_y: board.read64(reported::LATCH_HELD_Y).ok()?,
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

/// How many creations the capped half's first frame carries. Unit: deltas.
///
/// **Sixty, which is RFC 0128's own number** — *a boot sends one frame of sixty
/// deltas and reads fifty applied, ten refused* — and it has to sit between the
/// cap and the batch: above the compositor's declared fifty so the cap is
/// reached, and at or below `f_scene::commit::DELTAS_MAX`, sixty-four, so that
/// a compositor with **no** cap takes the whole frame rather than having the
/// batch poison it. That second bound is what makes the cap's deletion a
/// visible sixty applied instead of a frame lost for another reason.
const CAPPED_DELTAS: u32 = 60;

/// How many creations the capped half's second frame carries. Unit: deltas.
///
/// Eight, under the cap by a margin and over zero by one that a count carried
/// across the commit cannot hide: fifty already taken in the run would refuse
/// all eight.
const UNCAPPED_DELTAS: u32 = 8;

/// The capped half's two frames, as the client names them. See [`FRAME_ONE`].
/// Unit: none — frame identifiers.
const FRAME_CAPPED: u64 = 4;

/// See [`FRAME_CAPPED`]. Unit: none — a frame identifier.
const FRAME_UNCAPPED: u64 = 5;

/// Entries in the capped half's script: two frames of creations and their two
/// commits. Unit: entries.
const CAPPED_SCRIPT: usize = (CAPPED_DELTAS + UNCAPPED_DELTAS) as usize + 2;

/// How many frames [`Report::capped`] keeps apart. Unit: frames.
///
/// Four, which is more than any half here closes before the frame whose count
/// matters: the capped half's two, the wake half's three.
const CAPPED_FRAMES: usize = 4;

/// The capped half's script: [`CAPPED_DELTAS`] creations and a commit, then
/// [`UNCAPPED_DELTAS`] creations and a commit.
///
/// Creations rather than paints, because a creation leaves a node behind and
/// the arena counts its own nodes: `live` at the end is the graph's account of
/// how many of the sixty reached it, taken by nothing that also counted the
/// completions. Node 1 is the root and every other node hangs under it, so the
/// ten a correct compositor refuses are ten leaves whose absence disturbs
/// nothing else, and the second frame's parent is a node the first frame's
/// first delta made. Both commits have a whole scanout of room: this half is
/// not about pacing, and a late frame would move words its verdict does not
/// read for a reason it does not test.
fn capped_script() -> [Delta; CAPPED_SCRIPT] {
    let first = CAPPED_DELTAS as usize;
    let second = first + 1 + UNCAPPED_DELTAS as usize;
    core::array::from_fn(|at| {
        let commit = |named: u64| Delta {
            user_data: at as u64 + 1,
            class: 0,
            deadline: SCANOUT_PERIOD_NANOS,
            payload_offset: 0,
            flags: 0,
            body: Entry::Commit(Commit { frame_token: named }),
        };
        if at == first {
            return commit(FRAME_CAPPED);
        }
        if at == second {
            return commit(FRAME_UNCAPPED);
        }
        // Node identifiers one past the index, skipping the commit's slot, so
        // the first frame makes 1..=60 and the second 61..=68.
        let node = if at < first { at as u32 + 1 } else { at as u32 };
        let (parent, kind) = if node == 1 { (NO_NODE, kind::LAYER) } else { (1, kind::TRANSFORM) };
        Delta {
            user_data: at as u64 + 1,
            class: 0,
            deadline: 0,
            payload_offset: 0,
            flags: 0,
            body: Entry::CreateNode(CreateNode { node, parent, before: NO_NODE, kind }),
        }
    })
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
    /// Deltas the cap should refuse, `E3-B07e`. Unit: deltas.
    capped: u64,
    /// Deltas staged into the frame the walk is in, which goes back to zero at
    /// each commit exactly as the component's does. Unit: deltas.
    staged: u64,
}

impl Expected {
    /// Walk the script and count, under the cap the frame read off the
    /// compositor's record.
    ///
    /// The `match` has one arm per opcode and no wildcard, so a seventh scene
    /// opcode stops this build and asks what a boot should expect of it.
    fn of(script: &[Delta], cap: u64) -> Self {
        let mut expected = Self::seeded(REMOVED_NODES);
        expected.count(script, cap);
        expected
    }

    /// Nothing counted yet, with `removed` nodes the script's removals take —
    /// which is a fact about a subtree the walk cannot see, and is why it is
    /// seeded rather than counted. Unit of `removed`: nodes.
    const fn seeded(removed: u64) -> Self {
        Self { frames: 0, edits: 0, created: 0, removed, capped: 0, staged: 0 }
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
    fn count(&mut self, script: &[Delta], cap: u64) {
        for delta in script {
            // The cap, as RFC 0128 states it and not as the component codes it:
            // a delta that is not a commit, arriving when its frame already
            // holds `cap`, is refused and reaches nothing. Written here from the
            // sentence, so a component that counted differently disagrees with
            // this walk rather than with a copy of itself.
            if !matches!(delta.body, Entry::Commit(_)) {
                if self.staged >= cap {
                    self.capped += 1;
                    continue;
                }
                self.staged += 1;
            }
            match delta.body {
                Entry::Commit(_) => {
                    self.frames += 1;
                    self.staged = 0;
                }
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
        self.served_held()?;
        match self.half {
            Half::Mute => self.mute_verdict(),
            Half::Starved => self.starved_verdict(),
            Half::Serve => self.serve_verdict(),
            Half::Floorless => self.floorless_verdict(),
            Half::Wake => self.wake_verdict(),
            Half::Capped => self.capped_verdict(),
            Half::Timeline => self.timeline_verdict(),
        }
    }

    /// Which instance ran, for the three halves that stand one up. RFC 0129.
    ///
    /// **The component's own account of which occupant it is, against the
    /// occupant the lifecycle served.** The frame wrote the place's occupant
    /// count into the control ring's header when it spawned the instance; the
    /// component read it back through the adoption and reported it plus one.
    /// Neither number is derived from the other: one is the `Instance` the
    /// lifecycle handed a core, the other is what that core found in the ring it
    /// was given. A compositor stood up by any other builder describes its rings
    /// with whatever epoch *that* builder chose, and this is where the two part.
    ///
    /// Required on every serving half, the starved one included — a supervisor
    /// judges the starved reading exactly as it judges a serving one's, so it
    /// owes the same answer to *which instance published this*. The two halves
    /// that stand nothing up are required to report no instance at all.
    ///
    /// # Errors
    ///
    /// A sentence for the boot log.
    fn served_held(&self) -> Result<(), &'static str> {
        match (self.half, self.served) {
            (Half::Mute | Half::Floorless, None) => Ok(()),
            (Half::Mute | Half::Floorless, Some(_)) => {
                Err("a half that stands no compositor up reports an instance it served")
            }
            (Half::Serve | Half::Starved | Half::Wake | Half::Capped | Half::Timeline, None) => {
                Err("a serving half reports no instance, so nothing says it ran in its place")
            }
            (
                Half::Serve | Half::Starved | Half::Wake | Half::Capped | Half::Timeline,
                Some((epoch, _)),
            ) => {
                if self.board.epoch != u64::from(epoch) + 1 {
                    return Err("the component's own control ring names a different occupant \
                                than the one the lifecycle served, so the instance that published \
                                is not the instance in the place");
                }
                self.refill_held(epoch)
            }
        }
    }

    /// The place's next occupant, which is where [`Report::served_held`]'s
    /// comparison can fail. `E3-B05e`'s audit, and [`Refill`] is the argument.
    ///
    /// **Required only where it was served, and refused where it could not have
    /// been.** Whether the serving half's place is restarted at all is the
    /// supervisor's judgement, which RFC 0123 keeps out of the frame — so a
    /// serving half with no refill is left to `cargo xtask compositor`, which
    /// requires the restart and then this occupant's line. What the frame holds
    /// is identity, which is transport: the refill is the occupant after the one
    /// served, its component read *that* number off its own ring, and it served
    /// the script its predecessor did. A refill on any other half is a place
    /// restarted that nothing asked to restart.
    ///
    /// # Errors
    ///
    /// A sentence for the boot log.
    fn refill_held(&self, epoch: u32) -> Result<(), &'static str> {
        let Some(refilled) = self.refilled else { return Ok(()) };
        if self.half != Half::Serve {
            return Err("a half whose place is never restarted served a refilled occupant");
        }
        if u64::from(refilled.epoch) != u64::from(epoch) + 1 {
            return Err("the occupant served after the restart is not the place's next one: its \
                        epoch is not one past the occupant that timed out");
        }
        if refilled.reported != u64::from(refilled.epoch) + 1 {
            return Err("the refilled occupant's component says it is a different occupant than \
                        the one the lifecycle served — the epoch it reports is not the one the \
                        frame wrote into its control ring, so it did not read its ring, or read \
                        somebody else's");
        }
        if refilled.frames != self.board.frames
            || refilled.submitted != self.submitted
            || refilled.drained != refilled.submitted
        {
            return Err("the refilled occupant did not serve the script its predecessor served, \
                        so what identified it is a component that never ran the client's frames");
        }
        Ok(())
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
        // The cap's refusals, counted on both sides of the boundary, `E3-B07e`.
        // The component counts what it refused for the cap and the client counts
        // the completions that said so with the cap in them; on a half that
        // never reaches the cap both are zero, and that is the same relation
        // holding rather than a clause about nothing.
        if self.board.capped != self.capped.iter().sum::<u64>() {
            return Err("the deltas the component says it refused for the cap are not the capped \
                 completions the client reaped, so one side is counting something that is \
                 not the cap — or a refusal went out without the cap as its detail");
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
        let mut expected = Expected::of(&script(), self.cap);
        expected.count(&batch_script(), self.cap);

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

    /// The half that is capped, `E3-B07e`.
    ///
    /// **The order is RFC 0128's sentence.** The cap is the manifest's and not
    /// this file's, so it is checked to sit where the script can test it; then
    /// the excess refused and nothing else; then the commit accepted and the
    /// frame closed; then the rung held; then the second frame whole. The
    /// counts are walked out of the script under the cap the frame read, so a
    /// manifest declaring another cap moves the expectation with it — and the
    /// straddle clause is what stops that from quietly turning the boot into a
    /// run in which nothing was refused.
    fn capped_verdict(&self) -> Result<(), &'static str> {
        let expected = {
            let mut expected = Expected::seeded(0);
            expected.count(&capped_script(), self.cap);
            expected
        };
        if self.cap <= u64::from(UNCAPPED_DELTAS) || self.cap >= u64::from(CAPPED_DELTAS) {
            return Err(
                "the cap the compositor's manifest declares does not sit between this half's two \
                 frames, so one of them proves nothing: the first has to reach the cap and the \
                 second has to stay under it",
            );
        }
        if self.board.outcome != stopped::TOLD {
            return Err(
                "the component did not end on the frame's stop notice: its outcome word says it \
                 fell out of its loop for a reason of its own",
            );
        }
        if self.completed != self.submitted || self.board.drained != self.submitted {
            return Err(
                "the client did not get one completion per entry it submitted, or the component \
                 took a different number off the ring — a capped delta is still answered",
            );
        }
        // Nothing refused but the excess: no delta under the cap, and neither
        // commit. The commit is the clause RFC 0128 is most careful about — the
        // one that closes the frame is never counted and never refused.
        if self.refused != 0 || self.board.refused != 0 {
            return Err(
                "an entry was refused for something other than the cap: a delta under it, or a \
                 commit, which RFC 0128 never counts and never refuses because it is what \
                 closes the frame",
            );
        }
        let excess = u64::from(CAPPED_DELTAS) - self.cap;
        if self.capped[0] != excess {
            return Err(
                "the frame past the cap was not refused exactly its excess: the cap's worth of \
                 sixty applied and the rest answered RESOURCE/QUOTA_EXHAUSTED with the cap as the \
                 detail. Sixty applied is a cap nobody enforces; another count refused is a cap \
                 counted from somewhere other than the frame's first delta, or against a number \
                 other than the one on the routing page",
            );
        }
        if self.capped[1..] != [0; CAPPED_FRAMES - 1] {
            return Err(
                "the frame after the capped one was refused something, though it carried eight \
                 deltas against a larger cap — the count did not go back to zero at the commit, \
                 so it is a quota over the run rather than over a frame",
            );
        }
        if self.board.capped != expected.capped {
            return Err("the component's own count of capped deltas is not the script's excess");
        }
        // The batch was never offered the excess, so the frame closed with the
        // cap's worth in it: two frames, the cap plus the second frame's eight
        // applied, and the arena holding exactly those nodes.
        if self.board.staged != 0
            || self.board.frames != expected.frames
            || self.board.edits != expected.edits
            || self.board.created != expected.created
            || self.board.live != expected.live()
        {
            return Err(
                "the frames did not close with the cap's worth of the first and all of the \
                 second: an excess offered to the batch and refused there poisons the frame, \
                 and an excess applied leaves more nodes in the graph than the cap allows",
            );
        }
        if self.board.named != FRAME_UNCAPPED {
            return Err("the last frame the component closed is not the second one submitted");
        }
        if self.board.rung != HYBRID_RUNG {
            return Err(
                "the component is not on the rung this machine's report selects after a frame \
                 was capped: a quota is a refusal of submissions, and nothing about it is a \
                 reason to change how the compositor draws",
            );
        }
        if self.board.late != 0 || self.board.waits != 0 || self.board.timeouts != 0 {
            return Err(
                "a frame of this half was late or abandoned, though both had a whole scanout \
                 of room — so the cap cost the frame it refused into, which it must not",
            );
        }
        let published = [
            self.board.frames,
            self.board.edits,
            self.board.live,
            self.board.refused,
            self.board.rung,
            self.board.named,
        ];
        if self.tree[..published.len()] != published {
            return Err(
                "the component's published tree does not say what its board says about the \
                 frames it closed under the cap",
            );
        }
        self.crossings_and_chain_held()
    }

    /// The timeline half, `E3-B01`.
    ///
    /// **Every clause here is about whether the count can be believed, and none
    /// of them is the count.** The number — crossings per representative frame —
    /// is `claims/0039`'s to bound, and a verdict that also bounded it would be
    /// two places holding one threshold, which is how they come to disagree. What
    /// the boot owes is that the rows it prints are over the frames it says,
    /// counted on both sides, at every boundary, with nothing refused.
    ///
    /// The order is the argument. The run first: every frame closed, nothing
    /// refused, the compositor holding the whole scene. Then each cut, both
    /// sides — the component's words at every commit against the client's own
    /// counts at the same moment, which is where a side that counted something
    /// correlated with a crossing goes red frame by frame rather than in a total
    /// that could balance two wrongs. Then each frame against the reconciler: the
    /// entries a frame put on the ring are what the reconciler emitted plus a
    /// commit, so a client that sent more than it was told cannot be what the
    /// count measured. Then the cuts against the end-of-run totals, which are the
    /// same counters read a second time. Then `E3-B01j`'s own relations.
    fn timeline_verdict(&self) -> Result<(), &'static str> {
        let timeline = &self.timeline;
        if self.board.outcome != stopped::TOLD {
            return Err(
                "the component did not end on the frame's stop notice: its outcome word says it \
                 fell out of its loop for a reason of its own — between two frames of a client \
                 that reconciles 995 nodes, the idle backstop is the likeliest",
            );
        }
        if self.refused != 0
            || self.board.refused != 0
            || self.board.capped != 0
            || self.capped.iter().sum::<u64>() != 0
        {
            return Err(
                "an entry of the timeline was refused or capped: every frame the reconciler \
                 emits is under the cap and names nodes the compositor holds, so a refusal is \
                 a frame whose crossings were counted and whose edit never landed",
            );
        }
        if timeline.step == 0
            || timeline.build == 0
            || timeline.closed != timeline.build + WARM_FRAMES
        {
            return Err("the timeline did not close every frame it was built to: the build and \
                 the eight warm frames");
        }
        let nodes = f_compositor::timeline::NODES as u64;
        if timeline.nodes != nodes
            || self.board.created != nodes
            || self.board.live != nodes
            || self.board.removed != 0
        {
            return Err(
                "the compositor does not hold the scene the client built: claims/0033's census \
                 is 995 nodes, created once each and none removed, and a warm frame over a \
                 smaller graph is a frame of a different scene",
            );
        }
        for (k, cut) in timeline.cuts[..timeline.closed].iter().enumerate() {
            if cut.frames != k as u64 + 1 {
                return Err("a cut the client read is not the commit it had just reaped: the \
                     component's frame count at that moment names a different frame, so the \
                     words beside it are another frame's");
            }
            if cut.drained != cut.client_out || cut.answered != cut.client_back {
                return Err(
                    "at a frame boundary the component and the client disagree about how many \
                     entries had crossed in one direction or the other. Both are read at the \
                     same commit and neither derives from the other, so one side is counting \
                     something correlated with a crossing rather than a crossing",
                );
            }
            if cut.deltas > self.cap {
                return Err("a frame of the timeline carried more deltas than the cap it was \
                     built under");
            }
            let (out, _, _, _) = timeline.window(k);
            if out != cut.deltas + 1 {
                return Err(
                    "a frame put a different number of entries on the ring than the reconciler \
                     emitted and a commit, so what was counted is not the reconciler's frame",
                );
            }
        }
        if timeline.warm().any(|k| timeline.cuts[k].deltas == 0) {
            return Err(
                "a warm frame emitted nothing: the playhead did not move, so the frame counted \
                 is an idle one and not the frame claims/0033 describes",
            );
        }
        let last = timeline.cuts[timeline.closed - 1];
        if self.submitted != last.client_out
            || self.board.drained != last.drained
            || self.board.answered != last.answered + 1
            || self.completed != last.client_back + 1
        {
            return Err(
                "the last cut and the run's own totals are not the same counters read twice: \
                 the run ends on that commit, so its totals are the cut plus the one \
                 completion the cut is taken before",
            );
        }
        if self.board.frames != timeline.closed as u64
            || self.board.named != TIMELINE_FRAME_BASE + (timeline.closed as u64 - 1)
        {
            return Err("the component did not close the timeline's frames, ending on its last");
        }
        if self.board.late != 0 || self.board.waits != 0 || self.board.timeouts != 0 {
            return Err(
                "a frame of the timeline was late or abandoned, though every commit had a \
                 whole scanout of room",
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
        let expected = Expected::of(&script(), self.cap);
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
    // Which instance, first, because every number below is that instance's.
    // `cargo xtask compositor` reads this line and holds its epoch and page
    // against the lifecycle's `liveness` and `timeout` lines — the harness half
    // of RFC 0129's identity, and the kernel half is `Report::served_held`.
    if let Some((epoch, tree_at)) = report.served {
        crate::kprintln!(
            "  compositor    served in its place: occupant epoch {}, whose own control ring says \
             epoch {} (reported plus one: {}); its tree is the page at {:#x}",
            epoch,
            report.board.epoch.saturating_sub(1),
            report.board.epoch,
            tree_at,
        );
    }
    // And the place's next occupant, on the half that restarts it — the line
    // whose epochs can disagree. `cargo xtask compositor` holds its epoch
    // against the lifecycle's spawn line after the restart.
    if let Some(refilled) = report.refilled {
        crate::kprintln!(
            "  compositor    served its place's next occupant: occupant epoch {}, whose own \
             control ring says epoch {} (reported plus one: {}); its tree is the page at {:#x}; \
             {} frame(s) closed over {} entr(y/ies)",
            refilled.epoch,
            refilled.reported.saturating_sub(1),
            refilled.reported,
            refilled.tree_at,
            refilled.frames,
            refilled.drained,
        );
    }
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
            Half::Capped =>
                "the client sends one frame past the cap the compositor's manifest declares \
                 and one under it",
            Half::Timeline =>
                "the client builds claims/0033's scene through the reconciler and moves its \
                 playhead, every frame rebuilt whole and counted at every commit",
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
        Half::Serve | Half::Starved | Half::Wake | Half::Capped | Half::Timeline => {
            crate::kprintln!(
                "  compositor    {} entr(y/ies) submitted, {} answered, {} refused, {} drained \
                 by the component",
                report.submitted,
                report.completed,
                report.refused,
                report.board.drained,
            );
            // The cap, `E3-B07e`, on every serving half: the word the frame read
            // off the manifest's `scene` ring, and both sides' counts of what it
            // refused. Zero refused on the halves that never reach it is part of
            // the reading, not an absence of one.
            crate::kprintln!(
                "  compositor    cap {} delta(s) per frame from the manifest's scene ring; the \
                 component capped {}, the client reaped {} capped per frame (first four)",
                report.cap,
                report.board.capped,
                FramesCapped(report.capped),
            );
            // The recapped run says what it moved, on its own line so the one
            // above reads the same on every run. `cargo xtask compositor
            // recapped` requires this line and the two numbers in it to differ,
            // so a frame that ignored the parameter is not a second cap.
            if report.cap != report.declared {
                crate::kprintln!(
                    "  compositor    recapped: the record declares {} and the frame wrote {} — \
                     the same record with its scene ring's cap moved and nothing else changed",
                    report.declared,
                    report.cap,
                );
            }
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
            // `E3-B01`'s rows, on the timeline half alone and for the serving
            // half's reason: one half owns a row name.
            if report.half == Half::Timeline {
                timeline_lines(report);
            }
        }
    }
}

/// The timeline half's frames, each one, and the rows `claims/0039` reads.
///
/// **Every warm frame on its own line, with both sides' counts**, for
/// `claims/README.md`'s rule 3: a worst and a best are what the claim bounds,
/// and the line per frame is the distribution they were taken from. The build
/// frames are summarised rather than listed — forty lines of a number the claim
/// does not bound would bury the eight it does — and their worst is a row,
/// because a reader deciding whether *the first frame* is a UI frame needs it.
fn timeline_lines(report: &Report) {
    let timeline = &report.timeline;
    crate::kprintln!(
        "  compositor    the scene: {} node(s) built in {} frame(s) of {} under a cap of {}; \
         the compositor holds {}",
        timeline.nodes,
        timeline.build,
        timeline.step,
        report.cap,
        report.board.live,
    );
    let mut worst = 0;
    let mut best = u64::MAX;
    let mut sum = 0;
    let mut out_worst = 0;
    let mut back_worst = 0;
    for k in timeline.warm() {
        let (client_out, client_back, out, back) = timeline.window(k);
        crate::kprintln!(
            "  compositor    warm frame {}: {} delta(s) from the reconciler; {} out, {} back by \
             the component, {} out, {} back by the client",
            k - timeline.build + 1,
            timeline.cuts[k].deltas,
            out,
            back,
            client_out,
            client_back,
        );
        let crossings = timeline.crossings(k);
        worst = worst.max(crossings);
        best = best.min(crossings);
        sum += crossings;
        out_worst = out_worst.max(out);
        back_worst = back_worst.max(back);
    }
    let warm = timeline.warm().len() as u64;
    let cold_worst = (0..timeline.build).map(|k| timeline.crossings(k)).max().unwrap_or(0);
    let cold_sum: u64 = (0..timeline.build).map(|k| timeline.crossings(k)).sum();
    crate::kprintln!(
        "  compositor    the build: {} crossing(s) over {} frame(s), the worst {}",
        cold_sum,
        timeline.build,
        cold_worst,
    );
    crate::kprintln!("    ring_crossings_per_representative_frame_worst    {}", worst);
    crate::kprintln!(
        "    ring_crossings_per_representative_frame_best    {}",
        if warm == 0 { 0 } else { best }
    );
    crate::kprintln!(
        "    ring_crossings_per_representative_frame_x1000    {}",
        sum.saturating_mul(1000).checked_div(warm).unwrap_or(0),
    );
    crate::kprintln!("    ring_entries_out_per_representative_frame_worst    {}", out_worst);
    crate::kprintln!("    ring_entries_back_per_representative_frame_worst    {}", back_worst);
    crate::kprintln!("    representative_frames_closed    {}", warm);
    crate::kprintln!("    representative_scene_nodes_held    {}", report.board.live);
    crate::kprintln!("    build_frames_closed    {}", timeline.build);
    crate::kprintln!("    ring_crossings_per_build_frame_worst    {}", cold_worst);
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

/// The cap the compositor's record declares on the ring it serves scene deltas
/// on, `E3-B07e`.
///
/// **Found by `f_abi::manifest::FRAMED_PROTOCOL` and not by position or label**,
/// which is RFC 0128's defence against a rename: the checker and
/// `Record::read` refuse a `scene` server with no cap, and neither can see a
/// ring that stopped being called `scene`. This is the third reader and the one
/// that turns *not called `scene`* into a refusal — `None` here is
/// [`Trouble::Unframed`] and no compositor is started.
///
/// `None` also for a framed ring whose cap reads zero, which the reader refuses
/// already; it is answered here as well rather than trusted, because a zero
/// written onto the routing page is the one value the component refuses as a
/// page nobody finished.
///
/// Visible to the rest of the frame for [`found`]'s reason: `kernel/src/input.rs`
/// stands the same component up and must hand it the same word.
/// Unit: deltas per frame.
pub(crate) fn frame_cap(record: &Record) -> Option<u64> {
    let ring = record.rings().iter().find(|ring| ring.framed_server())?;
    (ring.deltas_per_frame_max != 0).then_some(u64::from(ring.deltas_per_frame_max))
}

/// The boot parameter that runs the capped half at [`RECAPPED`] rather than at
/// the manifest's own cap. `cargo xtask compositor recapped` passes it beside
/// `compositor=capped`, so the half, its script and its verdict are the capped
/// half's and the cap is the only thing that moves.
const RECAPPED_PARAMETER: &[u8] = b"compositor.recapped";

/// The second cap the capped half runs at, `E3-B07e`'s audit.
///
/// Thirty, the auditor's suggestion, and the constraints are what make it a
/// number rather than a preference: inside the range the batch allows
/// (`f_abi::manifest::FRAME_DELTAS_CAP_MAX`), strictly between the capped
/// half's two frames so the first reaches it and the second stays under it —
/// the straddle clause in [`Report::capped_verdict`] — and far enough from the
/// manifest's fifty that a compositor counting against a constant is off by
/// twenty rather than by one. Unit: deltas per frame.
const RECAPPED: u32 = 30;
const _: () = assert!(
    RECAPPED != 0
        && RECAPPED <= f_abi::manifest::FRAME_DELTAS_CAP_MAX
        && RECAPPED > UNCAPPED_DELTAS
        && RECAPPED < CAPPED_DELTAS
);

/// The compositor's record with its framed ring's cap moved to `cap`, and
/// nothing else changed.
///
/// # Why derived here and not compiled from a second manifest
///
/// **A second manifest is a second copy of three hundred lines that must stay
/// identical but for one number**, and nothing in this tree checks that two
/// manifests agree — `cargo xtask lint-manifests` judges each one alone. The
/// day the heap or the class moved in one and not the other, the second capped
/// run would be a run of a different component and would still be green. A
/// record derived here from the one the place was filled from cannot drift:
/// the difference is one field by construction, which is `mute`'s argument for
/// its own variant.
///
/// What it costs, said rather than hidden: the occupant the lifecycle spawned
/// was spawned from the record as compiled, and only the word on its routing
/// page comes from this one. That is honest today because the cap is read in
/// exactly one place — [`frame_cap`], onto the page — and nothing at spawn or
/// admission reads it. *What would reverse this:* a spawn or an admission that
/// sizes anything from the cap, at which point the variant must be a compiled
/// record the place is filled from, and the way to produce it without a second
/// file is `xtask` compiling the one manifest twice with the field overridden.
fn recapped(record: &Record, cap: u32) -> Record {
    let mut varied = *record;
    for ring in &mut varied.ring {
        if ring.framed_server() {
            ring.deltas_per_frame_max = cap;
        }
    }
    varied
}

/// Put the compositor's record past the admission a spawn performs, or describe a
/// machine below the bottom of the ladder and be refused — the two halves that
/// stand no component up.
///
/// **The three halves that do stand one up are not here any more**, and that is
/// RFC 0129. They are served from the compositor's own place by
/// `component::demonstrate`, with [`Placed`] as the client, so the occupant a
/// supervisor judges and restarts is the one whose tree it judged. This
/// function used to stand a compositor up beside the place with
/// `process::prepare_server` for all five, and the reading that crossed onto the
/// supervisor's row was that instance's — RFC 0126's narrowing, now reversed.
///
/// # Errors
///
/// [`Trouble`], every variant of which fails the boot. A serving half handed to
/// this function is [`Trouble::Placed`], by name: there is no second path that
/// stands one up beside the place.
///
/// # Safety
///
/// `frames` must be the kernel's allocator on its direct map, called on the boot
/// processor with nothing running. The direct map must be live and cover every
/// boot module.
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    half: Half,
    boot: &crate::BootInfo,
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

    match half {
        // SAFETY: the caller's guarantee about the boot processor and the
        // allocator, passed down; nothing is running.
        Half::Mute => unsafe { mute(frames, &record, image) },
        Half::Floorless => Ok(floorless(image)),
        Half::Serve | Half::Starved | Half::Wake | Half::Capped | Half::Timeline => {
            Err(Trouble::Placed)
        }
    }
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
        admitted: declared.is_ok(),
        muted: refused.err().unwrap_or(0),
        ..Report::nothing(Half::Mute, image.len() as u64)
    })
}

/// RFC 0080's refusal, before a page is spent.
///
/// What this half says the machine reports, and whether a machine reporting it
/// may be given a compositor at all. The control goes first and is the same
/// function over this boot's own machine, for `mute`'s reason: a refusal that
/// refused everything would look exactly like this one.
///
/// **Nothing is stood up whatever the answer**, and that is the clause rather
/// than a shortcut: *refused a compositor* means nothing was stood up, so the
/// verdict requires every page to be absent. Before RFC 0129 an admitted
/// floorless machine would have gone on to the stand-up below the refusal and
/// the verdict would have caught a compositor that existed; now it catches a
/// `floorless` word of zero, which is the same failure read one step earlier.
fn floorless(image: &[u8]) -> Report {
    let reported = Half::Floorless.reported();
    Report {
        backend_admitted: admit_backend(BACKEND_CAPABILITIES).is_ok(),
        floorless: admit_backend(reported).err().unwrap_or(0),
        ..Report::nothing(Half::Floorless, image.len() as u64)
    }
}

/// The client of a compositor served **from its place**: the three halves that
/// stand one up, driven by `component::demonstrate` against the place's own
/// occupant. RFC 0129.
///
/// # Why this is a client and not a stand-up
///
/// Because a stand-up is what made the reading a supervisor judged somebody
/// else's. RFC 0126 recorded it: this file used to build a compositor beside its
/// place with `process::prepare_server`, read two words off its tree, and hand
/// them to the lifecycle, which put them on the row of the place's own occupant
/// — an instance that had never run — and then stopped and restarted *that*
/// one. Every link was real and the subject was two compositors.
///
/// # Why it ran beside its place, and what answers it
///
/// **Its manifest declared no `board` and no `data` need.** A spawn maps a
/// routing page and ring memory only for a manifest that declares them, so the
/// place's occupant had nowhere to learn where its rings were and its serving
/// life would have ended `NO_ROUTING` before it looked. The file header's reason
/// — *there is no other client* — had stopped being the reason when
/// `component::Datapath` gave a place's occupant a client inside the lifecycle;
/// the missing declaration was what was left, and it is the wall `E1-B05` found
/// for `user/virtio-blk` and answered by declaring both. So is this one. A
/// manifest that declares neither is refused by name in
/// `component::demonstrate` — `Failure::Unserved` — and nothing falls back to a
/// stand-up beside the place.
///
/// # What is the same, and what is not
///
/// The script, the rings' shape, the board, the clock, the doorbell, the stop
/// and the verdict are this file's and unchanged: [`drive`] and [`drive_batch`]
/// are called exactly as they were. What moved is who built the instance — the
/// place's account paid for every page, the frame's root mounts its tree, and a
/// supervisor can end it — and two words: the occupant's `epoch`, which the
/// component reads off its own control ring and reports, and the physical page
/// its tree is, which the lifecycle prints at the copy and at the teardown.
pub struct Placed {
    /// Which half.
    half: Half,
    /// How long the image is. Unit: bytes.
    image: u64,
    /// How much heap this half describes to the component. Unit: bytes.
    described: u64,
    /// Whether this boot's own machine was admitted a compositor.
    backend_admitted: bool,
    /// What was taken before the core ran, `None` until [`Self::prepare`].
    before: Option<Before>,
    /// What the client saw while the occupant ran, `None` until `drive`.
    seen: Option<Result<Seen, Trouble>>,
    /// The one batch the waking half publishes.
    batch: Batched,
    /// The doorbell's own accounting, and the path it chose.
    rung: (&'static str, u64, u64),
    /// Which core the client drove from. Unit: none — a core index.
    client: usize,
    /// What was taken after the core came back, `None` until [`Self::ended`].
    after: Option<After>,
    /// The cap [`frame_cap`] read out of the compositor's record, written onto
    /// the routing page and carried back on every capped completion.
    /// Unit: deltas per frame.
    cap: u64,
    /// The cap the compositor's record declares as compiled — [`Placed::cap`]
    /// except on the recapped run, where the two must differ.
    /// Unit: deltas per frame.
    declared: u64,
    /// What the timeline half recorded, `E3-B01`.
    timeline: Timeline,
    /// The place's next occupant, served after the restart, `E3-B05e`.
    refill: Refill,
}

/// The same three readings as [`Placed`]'s own, of the occupant the lifecycle
/// spawned into the place after it stopped the one that timed out.
///
/// # Why a second occupant is served at all
///
/// **Because the identity check could not fail on one.** Every place's first
/// occupant is epoch zero, so the served occupant's epoch and the epoch its
/// component reads off its own control ring agree whether or not the component
/// reads anything: the audit of `E3-B05e` made the component report *zero*
/// without looking and `cargo xtask compositor serve` stayed green. The refill
/// is epoch one, so the same comparison over it is a comparison between two
/// numbers that can differ, and a component that does not read its ring now
/// says one where the frame wrote two.
///
/// Its readings are kept apart from the first occupant's, and never replace
/// them, because the verdict, the rows and `claims/0038`'s crossing count are
/// the first occupant's and the reading a supervisor judged was that one's.
/// What is taken from this one is identity and that it served.
#[derive(Clone, Copy)]
struct Refill {
    /// What was taken before its core ran.
    before: Option<Before>,
    /// What the client saw while it ran.
    seen: Option<Result<Seen, Trouble>>,
    /// What was taken after its core came back.
    after: Option<After>,
}

impl Refill {
    /// Nothing served yet.
    const NOTHING: Self = Self { before: None, seen: None, after: None };
}

/// What a [`Placed`] reads off the occupant before its first instruction.
#[derive(Clone, Copy)]
struct Before {
    /// Nodes the schema carries. Unit: nodes.
    nodes: u32,
    /// The blank tree's fold. Unit: none — a fold.
    blank: u64,
    /// The fifteen words, read while nothing had written them. Unit: as
    /// [`Report::tree`].
    words: [u64; WORDS],
    /// The worker core's four doorbell counts, which are the core's and not the
    /// run's: a place's occupant runs on a core the supervisor has already run
    /// on, so what this run caused is the difference.
    ///
    /// **A guard this boot does not exercise, said rather than implied.** Nothing
    /// that runs on the worker core before the compositor — the supervisor's
    /// consultations — halts it, so these read zero today, and a mutation that
    /// took the counts whole left the wake half green. The subtraction is here
    /// for the first earlier occupant that parks on that core, which would
    /// otherwise have its halts charged to this run. Unit: as [`Bells`].
    counts: [u64; 4],
    /// Which occupant of the place this is. Unit: instances.
    epoch: u32,
    /// The physical page its tree is. Unit: bytes, physical.
    tree_at: u64,
    /// Which core it runs on. Unit: none — a core index.
    cpu: usize,
}

/// What a [`Placed`] reads off the occupant after its core came back.
#[derive(Clone, Copy)]
struct After {
    /// Its board, or `None` where it never finished writing it.
    board: Option<Board>,
    /// What the routing page carried at `at::BACKEND_CAPABILITIES` at the end.
    reported: u64,
    /// The tree's fold at the end. Unit: none — a fold.
    fold: u64,
    /// The fifteen words at the end. Unit: as [`Report::tree`].
    words: [u64; WORDS],
    /// The worker core's four doorbell counts at the end. Unit: as [`Bells`].
    counts: [u64; 4],
}

impl Placed {
    /// A client for `half`, which must be one of the three that stand a
    /// compositor up.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoComponent`], [`Trouble::NoHeap`] and
    /// [`Trouble::HeapDisagrees`] exactly as [`demonstrate`] refuses them, before
    /// the lifecycle is asked to build anything; [`Trouble::Unframed`] for a
    /// record serving no `scene` ring, which is RFC 0128's rename made red;
    /// [`Trouble::Placed`] for a half that stands nothing up, said rather than
    /// served.
    ///
    /// # Safety
    ///
    /// The direct map must be live and cover every boot module.
    pub unsafe fn new(half: Half, boot: &crate::BootInfo) -> Result<Self, Trouble> {
        // SAFETY: the caller's guarantee.
        let (image, record) = unsafe { found(boot) }.ok_or(Trouble::NoComponent)?;
        let declared = heap_declared(&record).ok_or(Trouble::NoHeap)?;
        if declared != routing::HEAP_BYTES {
            return Err(Trouble::HeapDisagrees);
        }
        // Before the lifecycle builds anything, beside the heap check and for
        // its reason: a compositor with no cap to hand it is refused while
        // nothing has been spent, rather than started and told zero.
        let declared = frame_cap(&record).ok_or(Trouble::Unframed)?;
        // **The capped half at a second cap, `E3-B07e`'s audit.** Every boot and
        // every test held the cap at the manifest's fifty, so a compositor that
        // ignored the word on its page and wrote fifty in passed all of them. On
        // `compositor.recapped` the record is taken again with its framed ring's
        // cap moved to [`RECAPPED`] and **nothing else changed**, and the cap is
        // read out of that record by the same [`frame_cap`] — `mute`'s device for
        // its declaration, for its reason: the only difference between the two
        // runs is the thing under test, and there is no second manifest to drift
        // from the first. [`recapped`] argues why a derived record and not a
        // compiled one.
        let cap = if half == Half::Capped && boot.has_parameter(RECAPPED_PARAMETER) {
            let cap = frame_cap(&recapped(&record, RECAPPED)).ok_or(Trouble::Unframed)?;
            // A variant that lands on the declared cap tests nothing the declared
            // run did not, and would read as the second value it is not.
            if cap == declared {
                return Err(Trouble::Unframed);
            }
            cap
        } else {
            declared
        };
        let described = match half {
            // What the manifest declares, or — on the starved half — two pages,
            // which is the one number in this plan the component is asked to
            // disbelieve.
            Half::Starved => STARVED_HEAP_BYTES,
            Half::Serve | Half::Wake | Half::Capped | Half::Timeline => routing::HEAP_BYTES,
            Half::Mute | Half::Floorless => return Err(Trouble::Placed),
        };
        Ok(Self {
            half,
            image: image.len() as u64,
            described,
            backend_admitted: false,
            before: None,
            seen: None,
            batch: Batched::default(),
            rung: ("Polling", 0, 0),
            client: 0,
            after: None,
            cap,
            declared,
            timeline: Timeline::NOTHING,
            refill: Refill::NOTHING,
        })
    }

    /// Which place this client serves, by its manifest's label.
    #[must_use]
    pub const fn label() -> &'static [u8] {
        b"compositor"
    }

    /// Which life the occupant is entered at.
    #[must_use]
    pub const fn life() -> u32 {
        life::SERVE
    }

    /// Which two nodes go onto the supervisor's row, in its order: waits
    /// outstanding, then frames abandoned. The component's own ids, so the frame
    /// copies by id and never by position.
    #[must_use]
    pub const fn liveness() -> [u32; 2] {
        [node::WAITS, node::TIMEOUTS]
    }

    /// What the boot saw, once the lifecycle has served the occupant.
    ///
    /// # Errors
    ///
    /// [`Trouble::NotServed`] for a client the lifecycle never ran — which is a
    /// boot whose compositor half was asked for and never happened, and is red
    /// rather than an empty report; the client's own trouble where it had one;
    /// [`Trouble::BadReport`] for a board the component never finished.
    pub fn report(&self) -> Result<Report, Trouble> {
        let (Some(before), Some(seen), Some(after)) = (self.before, self.seen, self.after) else {
            return Err(Trouble::NotServed);
        };
        let seen = seen?;
        let board = after.board.ok_or(Trouble::BadReport)?;
        let [delivered, parks, woken, spared] = after.counts;
        let [was_delivered, was_parks, was_woken, was_spared] = before.counts;
        let (path, operations, rings) = self.rung;
        Ok(Report {
            submitted: seen.submitted,
            completed: seen.completed,
            refused: seen.refused,
            board,
            tree_nodes: before.nodes,
            tree_blank: before.blank,
            tree_before: before.words,
            tree_after: after.fold,
            tree: after.words,
            submitted_deadline: seen.deadline,
            last_tick: seen.last_tick,
            // The machine this half described was admitted a compositor, which
            // is what makes this the positive half of the pair the floorless
            // half completes: the same function, in the same build, over one
            // word.
            backend_admitted: self.backend_admitted,
            reported: after.reported,
            heap: self.described,
            bells: Bells {
                path,
                worker: before.cpu,
                client: self.client,
                operations,
                rings,
                batch_entries: self.batch.entries,
                batch_operations: self.batch.operations,
                batch_rings: self.batch.rings,
                delivered: delivered.saturating_sub(was_delivered),
                parks: parks.saturating_sub(was_parks),
                woken: woken.saturating_sub(was_woken),
                spared: spared.saturating_sub(was_spared),
            },
            served: Some((before.epoch, before.tree_at)),
            refilled: self.refilled(),
            cap: self.cap,
            declared: self.declared,
            capped: seen.capped,
            timeline: self.timeline,
            ..Report::nothing(self.half, self.image)
        })
    }

    /// Whether the occupant the lifecycle is serving now is the place's refill.
    ///
    /// Read off the first run's last reading rather than counted, because the
    /// lifecycle calls this client's three hooks in one order per occupant and
    /// the first occupant's `ended` is what closes its run. So everything after
    /// it is the next occupant's, and nothing before it can be.
    const fn refilling(&self) -> bool {
        self.after.is_some()
    }

    /// The refill's identity and what it served, or `None` where no refill was
    /// served — whole, or not at all, so a refill whose core never came back is
    /// the absence the verdict refuses rather than a half-filled row.
    fn refilled(&self) -> Option<Refilled> {
        let (Some(before), Some(Ok(seen)), Some(after)) =
            (self.refill.before, self.refill.seen, self.refill.after)
        else {
            return None;
        };
        let board = after.board.unwrap_or_default();
        Some(Refilled {
            epoch: before.epoch,
            tree_at: before.tree_at,
            reported: board.epoch,
            frames: board.frames,
            submitted: seen.submitted,
            drained: board.drained,
        })
    }
}

/// The place's next occupant, as the client that served it saw it. `E3-B05e`.
#[derive(Clone, Copy)]
pub struct Refilled {
    /// Which occupant the lifecycle handed a core: the epoch `spawn` wrote into
    /// its control ring's header. Unit: instances.
    pub epoch: u32,
    /// The page its own page tables put its tree on. Unit: bytes, physical.
    pub tree_at: u64,
    /// The epoch its component read off that control ring, plus one — zero for
    /// a component that never finished its board. Unit: none — an epoch plus one.
    pub reported: u64,
    /// Frames it closed. Unit: frames — UI frames.
    pub frames: u64,
    /// Entries the client put on its ring. Unit: entries.
    pub submitted: u64,
    /// Entries it took off that ring. Unit: entries.
    pub drained: u64,
}

/// The worker core's four doorbell counts, in [`Bells`]' order.
fn counts(cpu: usize) -> [u64; 4] {
    [
        crate::doorbell::delivered_at(cpu),
        crate::doorbell::parks_at(cpu),
        crate::doorbell::woken_at(cpu),
        crate::doorbell::spared_at(cpu),
    ]
}

impl crate::component::Datapath for Placed {
    fn prepare(
        &mut self,
        wired: crate::component::Wired,
        _frames: &FrameAllocator,
    ) -> Result<(), &'static str> {
        // RFC 0080's admission, over this boot's own machine. The positive half
        // of the pair `floorless` completes, and asked before anything is told
        // to the occupant: a machine refused here is a compositor that must not
        // be started, and nothing below this line would be true of it.
        self.backend_admitted = admit_backend(BACKEND_CAPABILITIES).is_ok();
        let reported = self.half.reported();
        if admit_backend(reported).is_err() {
            return Err("RFC 0080 refused this machine a compositor on a half that serves one");
        }
        let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0).why())?;
        if wired.board == 0 || wired.data == 0 || wired.tree == 0 || wired.heap == 0 {
            return Err("the compositor's occupant has no board, data ring, tree or heap");
        }

        // The data ring's header, written by the *grantor* and adopted by the
        // occupant — `f_ring::adopt`, RFC 0037. Here rather than in the
        // lifecycle, because the entry count is the client's business, and at
        // the occupant's own epoch, because a channel to this instance carries
        // which instance it is.
        // SAFETY: `wired.data` is the kernel address of a frame the occupant's
        // account paid for, `FRAME_SIZE` long, frame-aligned and reachable
        // through the direct map, with no core inside the occupant yet and no
        // reference into it held anywhere.
        let described =
            unsafe { Mapping::describe(wired.data as *mut u8, bytes, ENTRIES, wired.epoch, 0, 0) };
        described.map_err(|_| Trouble::Channel(0).why())?;

        // **The starved half's one difference, and it is a description.** The
        // account paid for the heap the manifest declares and `spawn` mapped all
        // of it; what the component reads is the prologue, and the prologue is
        // the frame's to write. Two pages here is the number the component is
        // asked to disbelieve — exactly what `process::prepare_server` described
        // on this half before RFC 0129, and the component's check is against
        // what it reads rather than what is mapped, which is the point of it.
        if self.half == Half::Starved {
            let starved = u32::try_from(STARVED_HEAP_BYTES).map_err(|_| Trouble::NoHeap.why())?;
            // SAFETY: `wired.heap` is the direct-map address of the run `spawn`
            // carved for this occupant's heap and described a moment after; it
            // is frame-aligned and longer than the prologue, and no core is
            // inside the occupant yet, so nothing is reading the prologue.
            unsafe { f_ring::heap::describe(wired.heap, starved) };
        }

        // The blank tree, read while it is blank. `spawn` wrote the schema out
        // of the manifest before the first instruction, so a component that is
        // served and does nothing still reads as *this component has done
        // nothing* — RFC 0065's distinction, and the reason this snapshot is
        // worth taking.
        let blank = state::Reader::at(wired.tree, FRAME_SIZE as u32)
            .map_err(|_| Trouble::StateTree(0).why())?;
        let mut words = [0u64; WORDS];
        for (slot, id) in words.iter_mut().zip(node::WRITTEN) {
            // `u64::MAX` for an id the schema does not carry, so a manifest and
            // `routing::node` that disagree fail this half's *blank* clause
            // rather than passing it with a zero nobody wrote.
            *slot = blank.value(id).unwrap_or(u64::MAX);
        }

        // --- what the component is told ---------------------------------------
        let board =
            Window::at(wired.board, routing::BYTES).map_err(|_| Trouble::Channel(0).why())?;
        for (offset, value) in [
            (at::CONTROL_AT, crate::process::SPAWN_CONTROL),
            (at::CONTROL_LEN, u64::from(bytes)),
            (at::DATA_AT, crate::process::BLK_DATA),
            (at::DATA_LEN, u64::from(bytes)),
            (at::TREE_AT, crate::process::SPAWN_TREE),
            (at::NEGOTIATED_VERSION, u64::from(ABI_VERSION)),
            (at::NEGOTIATED_FEATURES, 0),
            // The timeline half's client reconciles 995 nodes between frames, and
            // [`TIMELINE_IDLE_SPINS`] says why its backstop is wider.
            (
                at::IDLE_SPINS,
                if self.half == Half::Timeline { TIMELINE_IDLE_SPINS } else { IDLE_SPINS },
            ),
            // What the component needs to pace a frame, and none of it is
            // something a component could have found out for itself: RFC 0004
            // gives it no clock, nothing tells it about a display, and the
            // margin is a policy. The clock reading is a placeholder the client
            // overwrites before every entry.
            (at::TICK_NANOS, 0),
            (at::SCANOUT_PERIOD_NANOS, SCANOUT_PERIOD_NANOS),
            (at::PACING_MARGIN_NANOS, PACING_MARGIN_NANOS),
            // What this half says the machine reports, and the same word
            // `admit_backend` was given above — one function, so a boot cannot
            // admit one machine and describe another.
            (at::BACKEND_CAPABILITIES, reported),
            // **Whether this frame will ring, which is a statement about the
            // frame and not a mode of the component.** Only the wake half
            // rings; whether an interrupt reaches that core is a fact about a
            // vector, an interrupt controller and a second core, and all three
            // are on this side of the boundary.
            (at::DOORBELL, if self.half == Half::Wake { bell::RING } else { bell::POLL }),
            // **The cap, `E3-B07e`**, out of the record by `FRAMED_PROTOCOL` in
            // `Placed::new` and written on every half, because it is a fact about
            // the component's manifest and not about what this half tests: the
            // component refuses a page without it, so a half that left it off
            // would be testing that refusal instead.
            (at::DELTAS_PER_FRAME_MAX, self.cap),
        ] {
            board.write64(offset, value).map_err(|_| Trouble::Channel(0).why())?;
        }
        // The magic last, which is the whole of the discipline: a component that
        // reads a page this loop never finished finds a zero rather than a
        // plausible address.
        board.write64(at::MAGIC, routing::MAGIC).map_err(|_| Trouble::Channel(0).why())?;

        let before = Before {
            nodes: blank.nodes(),
            blank: blank.snapshot(),
            words,
            counts: counts(wired.cpu),
            epoch: wired.epoch,
            // The page the occupant's **own page tables** put its tree address
            // on, and not the page the lifecycle's bookkeeping says it gave it:
            // the liveness and timeout lines print that one, so the served line
            // printing the walk is what makes the harness's page comparison two
            // readings rather than one field three times. `Wired::tree_mapped`.
            tree_at: wired.tree_mapped,
            cpu: wired.cpu,
        };
        if self.refilling() {
            self.refill.before = Some(before);
        } else {
            self.before = Some(before);
        }
        Ok(())
    }

    fn drive(
        &mut self,
        frames: &mut FrameAllocator,
        wired: crate::component::Wired,
        _killer: &mut dyn crate::component::Killer,
    ) -> Result<crate::component::Drove, &'static str> {
        let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0).why())?;
        // The refill's run changes nothing the first run published: the path,
        // the doorbell's accounting and the client's core are the first
        // occupant's story, and [`Refill`] keeps only what the second one said.
        let refilling = self.refilling();
        if !refilling {
            self.client = crate::arch::x86_64::current_cpu();
        }
        // Both ends adopted rather than described, because this client is not
        // the grantor of either: `spawn` wrote the control ring's header out of
        // the occupant's own account, and `prepare` wrote the data ring's before
        // the core was told anything. A second `describe` would reset a ring the
        // occupant has already adopted.
        //
        // SAFETY: `wired.control` and `wired.data` are kernel addresses of two
        // frames the occupant's account paid for, each `FRAME_SIZE` long,
        // frame-aligned and reachable through the direct map; each has exactly
        // one other end and it is the occupant's. Every accessor hands out
        // atomics and `UnsafeCell`s rather than references.
        let control = unsafe {
            Mapping::adopt(
                wired.control as *mut u8,
                bytes,
                feature::CONTROL_EVENTS,
                feature::CONTROL_EVENTS,
            )
        };
        let Ok(control) = control else { return Err(Trouble::Channel(0).why()) };
        // SAFETY: as above.
        let client_end = unsafe { Mapping::adopt(wired.data as *mut u8, bytes, 0, 0) };
        let Ok(client_end) = client_end else { return Err(Trouble::Channel(0).why()) };
        let Ok(board) = Window::at(wired.board, routing::BYTES) else {
            return Err(Trouble::Channel(0).why());
        };
        let (Some(reaper), Some(mut producer), Some(notices)) = (
            Collector::new(client_end.completions()),
            Producer::new(client_end.channel()),
            Poster::new(control.completions()),
        ) else {
            return Err(Trouble::Channel(0).why());
        };
        let arena = client_end.arena();

        // --- the doorbell -----------------------------------------------------
        //
        // Selected the way `f_ring::doorbell::Path` says and not by this half's
        // name: what this half varies is the *hardware* half — whether one core
        // may interrupt another — because that is the honest description of the
        // difference between a boot that rings and one that does not.
        let hardware =
            Hardware { user_interrupts: false, cross_core_interrupts: self.half == Half::Wake };
        let path = Path::select(client_end.negotiated(), hardware);
        let Ok(mut doorbell) = Bell::new(path, hardware, crate::doorbell::Ipi::to(wired.cpu))
        else {
            return Err(Trouble::Channel(0).why());
        };

        // This boot's own clock, and the only one the component will ever see.
        // `PACING_SEED` is the whole of what decides the readings below, which
        // is what lets a pacing estimate be printed in a boot log at all.
        let mut env = SeededEnv::new(PACING_SEED, 0);
        let ends = Wire { reaper: &reaper, arena: &arena, board: &board, cap: self.cap };
        let tsc_khz = wired.tsc_khz;
        let driven = match self.half {
            Half::Serve => {
                drive(&producer, ends, &mut env, tsc_khz, &mut doorbell, false, &script())
            }
            Half::Capped => {
                drive(&producer, ends, &mut env, tsc_khz, &mut doorbell, false, &capped_script())
            }
            // `E3-B01`: the representative scene, built and then played, every
            // frame reconciled from a whole tree. Its working memory is frames
            // this client takes and gives back inside the call.
            Half::Timeline => drive_timeline(
                frames,
                &producer,
                ends,
                (&mut env, tsc_khz, &mut doorbell),
                &mut self.timeline,
            ),
            Half::Wake => {
                // The script first, one entry at a time and each one waited for,
                // so that every submission lands on a core this client has
                // watched stop. Then the third frame, whole, as the one batch the
                // second clause is about.
                drive(&producer, ends, &mut env, tsc_khz, &mut doorbell, true, &script()).and_then(
                    |mut seen| {
                        self.batch = drive_batch(
                            &mut producer,
                            ends,
                            &mut env,
                            tsc_khz,
                            &mut doorbell,
                            &mut seen,
                        )?;
                        Ok(seen)
                    },
                )
            }
            // A starved component ends before it adopts anything, so a client
            // that submitted would be waiting for a completion from a component
            // that has already exited.
            Half::Starved | Half::Mute | Half::Floorless => Ok(Seen::NOTHING),
        };

        // Told to stop whatever happened above, because a component left serving
        // a client that has gone is a core this boot never gets back — and rung
        // for, on the half where it may be asleep: a stop notice goes on the
        // control ring, which has no wakeup flag of its own, and a doorbell says
        // only *stop halting*.
        let told = notices.post(control::entry(control::notice::STOP, 0, 0, 0));
        if self.half == Half::Wake {
            doorbell.submitted(true);
        }
        if refilling {
            self.refill.seen = Some(driven);
        } else {
            self.rung = (
                match doorbell.path() {
                    Path::Polling => "Polling",
                    Path::KernelIpi => "KernelIpi",
                    Path::UserInterrupt => "UserInterrupt",
                },
                doorbell.operations(),
                doorbell.rings(),
            );
            self.seen = Some(driven);
        }
        if told.is_err() {
            return Err(Trouble::Channel(0).why());
        }
        match driven {
            Ok(_) => Ok(crate::component::Drove::Finished),
            Err(why) => Err(why.why()),
        }
    }

    /// Nothing: a compositor reaches no device, so there is nothing it can ask
    /// the frame for while it runs.
    fn serve(&mut self, _frames: &mut FrameAllocator) {}

    /// Nothing: every page this client touched is the occupant's account's, and
    /// it allocates none of its own.
    fn retained(&mut self, _frames: &mut FrameAllocator) -> u64 {
        0
    }

    fn ended(&mut self, wired: crate::component::Wired) {
        // **After the join and not before it, and that is what makes reading
        // another core's counters legal rather than lucky.** The mailbox word
        // the join waits on is stored with `Release` by the worker and loaded
        // with `Acquire` here, so everything that core wrote before it reported
        // finished is visible to this one.
        let counts = counts(wired.cpu);
        let Ok(board) = Window::at(wired.board, routing::BYTES) else { return };
        let Ok(reader) = state::Reader::at(wired.tree, FRAME_SIZE as u32) else { return };
        let mut words = [0u64; WORDS];
        for (slot, id) in words.iter_mut().zip(node::WRITTEN) {
            *slot = reader.value(id).unwrap_or(0);
        }
        let after = After {
            board: Board::of(&board),
            reported: board.read64(at::BACKEND_CAPABILITIES).unwrap_or(0),
            fold: reader.snapshot(),
            words,
            counts,
        };
        if self.refilling() {
            self.refill.after = Some(after);
        } else {
            self.after = Some(after);
        }
    }
}

/// What the client itself saw.
#[derive(Clone, Copy)]
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
    /// Completions that said `RESOURCE/QUOTA_EXHAUSTED` with the cap as the
    /// detail, by frame — [`Report::capped`]. Counted apart from
    /// [`Seen::refused`], so a capped half's clean clause stays *nothing else
    /// was refused*. Unit: completions.
    capped: [u64; CAPPED_FRAMES],
    /// Completions reaped before the last commit's own, `E3-B01`.
    ///
    /// **[`Seen::completed`] read at a moment, not a second count.** It is the
    /// client's half of the cut `reported::CUT_ANSWERED` is the component's
    /// half of: both are taken with the commit gone out and its completion not
    /// yet back, so the timeline half can require the two sides equal at every
    /// frame boundary rather than only at the end of the run.
    /// Unit: completions.
    at_commit: u64,
}

impl Seen {
    /// A client that submitted nothing.
    const NOTHING: Self = Self {
        submitted: 0,
        completed: 0,
        refused: 0,
        deadline: 0,
        last_tick: 0,
        capped: [0; CAPPED_FRAMES],
        at_commit: 0,
    };
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
    script: &[Delta],
) -> Result<Seen, Trouble> {
    let Wire { reaper, arena, board, cap } = wire;
    let mut seen = Seen::NOTHING;
    let mut parked = 0;
    // Which frame an answer belongs to, by the commits already submitted, for
    // `Report::capped`'s split. The last index takes everything past it.
    let mut frame = 0;
    let quota = error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED);
    for &delta in script {
        let mut delta = delta;
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
            // The client's cut, `E3-B01`: what it had reaped before this
            // commit's own completion comes back.
            seen.at_commit = seen.completed;
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
                    // The cap's answer, and only the cap's: the right entry,
                    // the quota, and the cap the frame wrote as the detail. A
                    // quota refusal carrying any other detail is a refusal this
                    // client cannot account for and is counted as one.
                    if answer.user_data == entry.user_data
                        && answer.result == quota
                        && answer.ext == cap
                    {
                        seen.capped[frame] += 1;
                    } else if answered_badly(&answer, entry.user_data) {
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
        if matches!(delta.body, Entry::Commit(_)) {
            frame = (frame + 1).min(CAPPED_FRAMES - 1);
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
    /// The cap the frame wrote on that page, which is what a capped completion
    /// must carry back as its detail. Unit: deltas per frame.
    cap: u64,
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
    let Wire { reaper, arena, board, cap: _ } = wire;
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

// --- `E3-B01`: the representative frame, through the reconciler ------------
//
// `f_compositor::timeline`'s header is the argument for the scene and for why
// the client's application lives in that crate; RFC 0133 is the argument for
// which frame is *the* UI frame. What is here is the client driving it: the
// whole tree rebuilt every frame, `f_scene`'s reconciler deciding what differs,
// and every delta it emits put on the real ring by [`drive`] — the same
// function, one entry at a time and each one answered, that `claims/0038`'s
// workload goes through. Nothing here counts a crossing. The client's counts
// are [`Seen`]'s and the component's are `E3-B01j`'s own, read at every commit
// off `reported::CUT_*`.

/// How many warm frames the timeline half plays after the scene is built.
///
/// Eight, and more than one for `claims/README.md`'s rule 3 — a distribution and
/// not a summary: one frame is one observation, and a count that was four on
/// the first warm frame and nine on the fourth would be a claim a single frame
/// could not make. Eight rather than eighty because every one of them is a full
/// reconciliation of 995 nodes by a quadratic search on the boot processor, and
/// the frames are identical in shape; the number to watch is the worst, and a
/// longer run does not move it.
/// Unit: frames — UI frames.
const WARM_FRAMES: usize = 8;

/// The most build frames this half will take before it refuses.
///
/// Forty is what the manifest's cap of fifty gives — twenty-five nodes a frame,
/// `f_compositor::timeline::build_step` — and this is room above that for a
/// manifest declaring a smaller cap, bounded so the per-cut record is a fixed
/// array. A cap small enough to need more is refused by name rather than run.
/// Unit: frames.
const BUILD_FRAMES_MAX: usize = 56;

/// Every cut the timeline half can record: the build and the warm frames.
/// Unit: frames.
const TIMELINE_CUTS: usize = BUILD_FRAMES_MAX + WARM_FRAMES;

/// The identifier of the timeline half's first frame.
///
/// Not [`FRAME_ONE`], and that is load-bearing rather than tidy: [`drive`]
/// writes a better backend report onto the page after a commit naming frame one,
/// which is `E3-B02b`'s probe on the serving half and nothing this half asks.
/// Unit: none — a frame identifier.
const TIMELINE_FRAME_BASE: u64 = 256;

/// Idle turns the component may spend between two of this half's frames.
///
/// **Fifty times [`IDLE_SPINS`], because this client does work between frames
/// that the others do not.** Every frame is a whole tree rebuilt and reconciled
/// on the boot processor while the component spins on the other core with
/// nothing on its ring, and the reconciler's search is quadratic in 995 nodes.
/// The bound is still a backstop — the stop notice ends the run — and one sized
/// for a client that reconciles nothing would end this run between frames with
/// `stopped::IDLE`, which is a harness choosing its own failure.
/// Unit: turns.
const TIMELINE_IDLE_SPINS: u64 = 50 * IDLE_SPINS;

/// The largest frame script: the reconciler's buffer and a commit. Unit: entries.
const TIMELINE_SCRIPT_MAX: usize = f_compositor::tree::FRAME_DELTAS_MAX + 1;

/// One cut of the timeline run, taken at a commit on both sides.
#[derive(Clone, Copy)]
struct Cut {
    /// Non-commit deltas the reconciler emitted for this frame. Unit: deltas.
    deltas: u64,
    /// The client's entries submitted, the commit included — cumulative.
    /// Unit: entries.
    client_out: u64,
    /// The client's completions reaped before the commit's own — cumulative.
    /// Unit: entries.
    client_back: u64,
    /// `reported::CUT_FRAMES` as the client read it after the commit's
    /// completion. Unit: frames.
    frames: u64,
    /// `reported::CUT_DRAINED`. Unit: entries.
    drained: u64,
    /// `reported::CUT_ANSWERED`. Unit: entries.
    answered: u64,
}

impl Cut {
    /// A cut before anything was sent.
    const ZERO: Self =
        Self { deltas: 0, client_out: 0, client_back: 0, frames: 0, drained: 0, answered: 0 };
}

/// What the timeline half recorded: every cut, and the build's shape.
#[derive(Clone, Copy)]
pub struct Timeline {
    /// One per frame, build frames first. Unit: as [`Cut`].
    cuts: [Cut; TIMELINE_CUTS],
    /// How many of `cuts` are meaningful. Unit: frames.
    closed: usize,
    /// How many of them were build frames. Unit: frames.
    build: usize,
    /// Nodes the build grew the tree by per frame. Unit: nodes per frame.
    step: usize,
    /// Nodes the scene has, as the client built it. Unit: nodes.
    nodes: u64,
}

impl Timeline {
    /// Nothing recorded, which every half but one reports.
    const NOTHING: Self =
        Self { cuts: [Cut::ZERO; TIMELINE_CUTS], closed: 0, build: 0, step: 0, nodes: 0 };

    /// Frame `k`'s crossings as each side counted them, between cut `k - 1` and
    /// cut `k`: `(client out, client back, component out, component back)`.
    /// Unit: entries.
    fn window(&self, k: usize) -> (u64, u64, u64, u64) {
        let now = self.cuts[k];
        let was = if k == 0 { Cut::ZERO } else { self.cuts[k - 1] };
        (
            now.client_out.saturating_sub(was.client_out),
            now.client_back.saturating_sub(was.client_back),
            now.drained.saturating_sub(was.drained),
            now.answered.saturating_sub(was.answered),
        )
    }

    /// Frame `k`'s crossings by the component's count. Unit: entries.
    fn crossings(&self, k: usize) -> u64 {
        let (_, _, out, back) = self.window(k);
        out + back
    }

    /// The warm frames, as indices into `cuts`.
    const fn warm(&self) -> core::ops::Range<usize> {
        self.build..self.closed
    }
}

/// A block of frames big enough for `bytes`, as an order.
const fn order_for(bytes: usize) -> u8 {
    let mut order = 0u8;
    while (FRAME_SIZE as usize) << order < bytes {
        order += 1;
    }
    order
}

/// The reconciler, which holds the tree the compositor holds: 995 nodes and
/// the scratch its search needs. Unit: bytes.
const RECONCILER_BYTES: usize =
    core::mem::size_of::<f_compositor::timeline::Reconciler<{ f_compositor::timeline::NODES }>>();

/// An empty reconciler, made once by its own constructor and copied from here.
///
/// **Why a copy out of the image and not a `write` of `Reconciler::new()`**:
/// the kernel is built without optimisation, so a value that is written is a
/// value that is first built on the stack — and this one is larger than half
/// the boot processor's. The first boot of this half took a double fault in
/// [`drive_timeline`] doing exactly that, with `rbp - rsp` the size of the
/// value. The price is the value's bytes in the frame's constants, which is
/// what `f_scene::reconcile`'s private fields hold empty and nothing this file
/// could spell more cheaply without reasoning about another crate's fields:
/// zeroed memory would do on today's field types and would stop being sound
/// the day one of them gained a niche. Measured by the boot's own `frame` line:
/// 1 773 568 bytes of text and rodata before this half, 1 904 640 with it.
///
/// *What would reverse this:* an in-place constructor in `f_scene` — one taking
/// `&mut MaybeUninit<Self>` — or the frame being built optimised, where the
/// write is built in place; either way this goes.
static EMPTY_RECONCILER: f_compositor::timeline::Reconciler<{ f_compositor::timeline::NODES }> =
    f_compositor::timeline::Reconciler::new();

/// The tree the application rebuilds every frame. Unit: bytes.
const TREE_BYTES: usize =
    core::mem::size_of::<[f_compositor::timeline::Node; f_compositor::timeline::NODES]>();

/// Build the representative scene through the reconciler, then play it.
///
/// **The two working sets are in frames this function allocates and gives
/// back**, because they are a quarter of a mebibyte between them and a kernel
/// stack is not: the boot processor's is 256 KiB, and `kernel/linker.ld`'s
/// comment on it records what the places added to that stack have already cost.
/// The reconciler is copied in from [`EMPTY_RECONCILER`], which its own
/// constructor made, and the tree is written a slot at a time — so no value
/// larger than one node is ever built on the stack, and the frame never holds a
/// reconciler that its constructor did not make.
///
/// # Errors
///
/// [`Trouble::NoClientMemory`] where the blocks cannot be had,
/// [`Trouble::Reconciled`] where the reconciler refuses a frame — which is the
/// client's own tree being wrong, and red rather than skipped — and whatever
/// [`drive`] answers.
fn drive_timeline(
    frames: &mut FrameAllocator,
    producer: &Producer<'_>,
    wire: Wire<'_, '_>,
    clock: (&mut SeededEnv, u64, &mut Bell<crate::doorbell::Ipi>),
    record: &mut Timeline,
) -> Result<Seen, Trouble> {
    use f_compositor::timeline::{NODES, Node, Reconciler};

    let reconciler_order =
        crate::mem::Order::new(order_for(RECONCILER_BYTES)).ok_or(Trouble::NoClientMemory)?;
    let tree_order =
        crate::mem::Order::new(order_for(TREE_BYTES)).ok_or(Trouble::NoClientMemory)?;
    let reconciler_block = frames.alloc_zeroed(reconciler_order).ok_or(Trouble::NoClientMemory)?;
    let Some(tree_block) = frames.alloc_zeroed(tree_order) else {
        // SAFETY: the block was handed out two lines up and nothing references
        // it — no pointer into it has been made.
        unsafe { frames.free(reconciler_block) };
        return Err(Trouble::NoClientMemory);
    };
    let reconciler = frames.virt(reconciler_block).cast::<Reconciler<NODES>>();
    let tree = frames.virt(tree_block).cast::<[Node; NODES]>();
    // SAFETY: `reconciler` is the direct-map address of a block this function
    // was just handed, frame-aligned — stronger than the reconciler's alignment,
    // which is a `u64`'s — and at least `RECONCILER_BYTES` long by
    // `order_for`'s construction; nothing else holds a pointer into it, and it
    // does not overlap `EMPTY_RECONCILER`, which is in the image's constants.
    // The bytes copied are the reconciler's own constructor's, evaluated at
    // compile time, so every invariant its private fields carry is one
    // `Reconciler::new` established.
    unsafe { core::ptr::copy_nonoverlapping(&raw const EMPTY_RECONCILER, reconciler, 1) };
    // **Slot by slot, and the first boot of this half is why.** Written as one
    // `[Node::UNUSED; NODES]` the array was built in a 95 520-byte temporary on
    // the boot processor's stack and copied, and the stack's guard page took a
    // double fault — `rbp - rsp` was the array's size to the byte. The
    // reconciler's own constructor above is written whole and was built in
    // place; if a toolchain ever stops doing that, the same guard page is what
    // says so, loudly, rather than a corruption.
    let slots = tree.cast::<Node>();
    for slot in 0..NODES {
        // SAFETY: `slots` is the start of the second block, frame-aligned —
        // stronger than `Node`'s alignment — and at least `TREE_BYTES` long,
        // so `slot < NODES` is inside it; nothing else holds a pointer into it.
        // The value is `Node::UNUSED`, which `f_scene::reconcile` names as a
        // slot holding no node.
        unsafe { slots.wrapping_add(slot).write(Node::UNUSED) };
    }
    // SAFETY: initialised above, in a block of its own, and this is the only
    // reference ever made over it. It is handed to `play` and cannot outlive
    // that call, which returns before the block is freed below.
    let reconciler = unsafe { &mut *reconciler };
    // SAFETY: as above, for the tree's block, every slot of which the loop
    // above wrote.
    let tree = unsafe { &mut *tree };

    let run = play(reconciler, tree, producer, wire, clock, record);

    // SAFETY: the block was handed out above, the one reference into it was
    // consumed by `play`, which has returned, and nothing else was made over it.
    unsafe { frames.free(tree_block) };
    // SAFETY: as above, for the other block.
    unsafe { frames.free(reconciler_block) };
    run
}

/// The frames themselves: build, then play. [`drive_timeline`] owns the memory
/// and this owns nothing, which is what lets the memory's lifetime be one call.
///
/// # Errors
///
/// As [`drive_timeline`].
fn play(
    reconciler: &mut f_compositor::timeline::Reconciler<{ f_compositor::timeline::NODES }>,
    tree: &mut [f_compositor::timeline::Node; f_compositor::timeline::NODES],
    producer: &Producer<'_>,
    wire: Wire<'_, '_>,
    clock: (&mut SeededEnv, u64, &mut Bell<crate::doorbell::Ipi>),
    record: &mut Timeline,
) -> Result<Seen, Trouble> {
    use f_compositor::timeline::{self, Deltas, NODES};
    let (env, tsc_khz, doorbell) = clock;

    let cap = usize::try_from(wire.cap).unwrap_or(0);
    let step = timeline::build_step(cap);
    let build = timeline::build_frames(step);
    if step == 0 || build > BUILD_FRAMES_MAX {
        return Err(Trouble::Reconciled);
    }
    *record = Timeline { step, build, ..Timeline::NOTHING };
    let mut total = Seen::NOTHING;
    let mut deltas = Deltas::<{ f_compositor::tree::FRAME_DELTAS_MAX }>::new();
    let first = TIMELINE_FRAME_BASE;
    let filler = Delta {
        user_data: 0,
        class: 0,
        deadline: 0,
        payload_offset: 0,
        flags: 0,
        body: Entry::Commit(Commit { frame_token: first }),
    };
    let mut script = [filler; TIMELINE_SCRIPT_MAX];
    let mut sequence = 0u64;
    for k in 0..build + WARM_FRAMES {
        // Immediate mode: the whole scene, every frame. During the build the
        // application presents the prefix it has; once built, the playhead
        // moves one step a frame and nothing else does.
        let moved = if k < build { 0 } else { (k - build) as u64 + 1 };
        let built = timeline::build(tree, timeline::playhead_x65536(moved));
        record.nodes = built.written as u64;
        let upto = if k < build { ((k + 1) * step).min(NODES) } else { NODES };
        reconciler.frame(&tree[..upto], &mut deltas).map_err(|_| Trouble::Reconciled)?;
        let emitted = deltas.as_slice();
        for (slot, entry) in script.iter_mut().zip(emitted) {
            sequence += 1;
            *slot = Delta { user_data: sequence, body: *entry, ..filler };
        }
        sequence += 1;
        let named = first + k as u64;
        script[emitted.len()] = Delta {
            user_data: sequence,
            deadline: SCANOUT_PERIOD_NANOS,
            body: Entry::Commit(Commit { frame_token: named }),
            ..filler
        };
        let seen = drive(producer, wire, env, tsc_khz, doorbell, false, &script[..=emitted.len()])?;

        // The client's cut and the component's, at the same commit. The
        // component's words were written before that commit's completion was
        // posted and this side has reaped it, so they are this commit's —
        // `reported::CUT_FRAMES` is the argument.
        let board = wire.board;
        record.cuts[k] = Cut {
            deltas: emitted.len() as u64,
            client_out: total.submitted + seen.submitted,
            client_back: total.completed + seen.at_commit,
            frames: board.read64(reported::CUT_FRAMES).unwrap_or(0),
            drained: board.read64(reported::CUT_DRAINED).unwrap_or(0),
            answered: board.read64(reported::CUT_ANSWERED).unwrap_or(0),
        };
        record.closed = k + 1;
        total.submitted += seen.submitted;
        total.completed += seen.completed;
        total.refused += seen.refused;
        total.deadline = seen.deadline;
        total.last_tick = seen.last_tick;
        for (sum, one) in total.capped.iter_mut().zip(seen.capped) {
            *sum += one;
        }
    }
    Ok(total)
}

/// [`Report::capped`] as the log prints it: four counts, oldest frame first.
struct FramesCapped([u64; CAPPED_FRAMES]);

impl core::fmt::Display for FramesCapped {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a} {b} {c} {d}")
    }
}

/// Is this completion anything other than *the entry it names was accepted*?
///
/// Two things at once on purpose: a refusal, and an answer to a different entry.
/// A client that checked only the result would count a completion for entry
/// three as the answer to entry four and never notice.
fn answered_badly(answer: &Cqe, expected: u64) -> bool {
    answer.result < 0 || answer.user_data != expected
}
