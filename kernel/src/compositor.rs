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

use f_abi::manifest::Record;
use f_abi::scene::{Commit, CreateNode, Delta, Entry, NO_NODE, RemoveNode, SetPaint, kind};
use f_abi::{ABI_VERSION, Cqe, control, error, feature, state};
use f_compositor::routing::{self, at, life, node, reported, stopped};
use f_ring::{Arena, Collector, Mapping, Poster, Producer, Window};

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

/// The deadline every commit in this boot carries.
/// Unit: nanoseconds, in this channel's epoch.
///
/// A literal, and it has to be one. `f_abi::scene::Commit` is refused without a
/// deadline — a frame nobody scheduled is a frame the pacing loop cannot account
/// for — and nothing in this build computes one: `E3-B01h` is the task that
/// derives a wake time backwards from the next scanout, and until it lands a
/// number taken from this machine's clock would make this boot's log differ
/// between a fast host and a slow one for a value nothing reads.
///
/// Sixteen milliseconds and two thirds is a sixty-hertz frame, which is the
/// interval the number will *mean* when something computes it.
const DEADLINE_NANOS: u64 = 16_666_667;

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
}

impl Half {
    /// A word for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Starved => "starved",
            Self::Mute => "mute",
        }
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
    /// The four words this component publishes, read before it ran a line.
    ///
    /// Required to be zero, which is what makes the four read afterwards
    /// evidence of anything: a component that published nothing into a tree
    /// somebody else had already filled in would be indistinguishable from one
    /// that published. Unit: as [`Report::tree`].
    pub tree_before: [u64; 4],
    /// The snapshot taken after it ended. Unit: none — a fold.
    pub tree_after: u64,
    /// The four words this component's manifest says it publishes, read back out
    /// of the tree by id: frames, edits, nodes, refused.
    pub tree: [u64; 4],
    /// Whether the record as its manifest declares it was admitted.
    pub admitted: bool,
    /// What the admission said about the same record with its state declaration
    /// emptied. Unit: none — a packed refusal, or zero for one that was
    /// admitted.
    pub muted: i32,
    /// How long the image is. Unit: bytes.
    pub image: u64,
    /// How much heap the frame described for this run.
    ///
    /// Reported rather than taken from the manifest, because the two differ on
    /// exactly one half and that half is about the difference.
    /// Unit: bytes.
    pub heap: u64,
}

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
}

impl Board {
    /// Read one out of the page, or `None` where the component never finished
    /// writing it.
    fn of(board: &Window) -> Option<Self> {
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
    let commit = |user_data: u64, named: u64| Delta {
        user_data,
        class: 0,
        deadline: DEADLINE_NANOS,
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
        commit(6, FRAME_ONE),
        Delta {
            user_data: 7,
            class: 0,
            deadline: 0,
            payload_offset: 0,
            flags: 0,
            body: Entry::RemoveNode(RemoveNode { node: REMOVED_ROOT }),
        },
        commit(8, FRAME_TWO),
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
        for delta in script {
            match delta.body {
                Entry::Commit(_) => expected.frames += 1,
                Entry::CreateNode(_) => {
                    expected.created += 1;
                    expected.edits += 1;
                }
                Entry::SetTransform(_)
                | Entry::SetPath(_)
                | Entry::SetPaint(_)
                | Entry::RemoveNode(_) => expected.edits += 1,
            }
        }
        expected
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
        }
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
        if self.tree_nodes as usize != node::WRITTEN.len() + 1 || self.tree_before != [0; 4] {
            return Err("the tree the frame published for a starved component is not the one its \
                 manifest declares");
        }
        if self.tree != [0; 4] {
            return Err("a component that never held a graph published words about one");
        }
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

        // --- the published tree ---------------------------------------------
        if self.tree_nodes as usize != node::WRITTEN.len() + 1 {
            return Err(
                "the schema the frame published out of the manifest does not carry the nodes \
                 this component writes, plus the subtree they hang under",
            );
        }
        if self.tree_before != [0; 4] {
            return Err(
                "the tree already carried a word before the component ran, so a word in it                  afterwards would be evidence of nothing",
            );
        }
        if self.board.published as usize != node::WRITTEN.len() {
            return Err(
                "the component wrote fewer words into its tree than it declares nodes: an id it \
                 published is not one its manifest carries",
            );
        }
        let published = [self.board.frames, self.board.edits, self.board.live, self.board.refused];
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
        if self.tree_after == self.tree_blank {
            return Err(
                "the tree is byte-identical to the one published before the component ran, so \
                 nothing was published at all",
            );
        }
        Ok(())
    }
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
        }
    );
    match report.half {
        Half::Mute => {
            crate::kprintln!(
                "  compositor    as declared: {}; declaring no tree: {} — \
                 ADMISSION/NO_STATE_TREE is {}",
                if report.admitted { "admitted" } else { "REFUSED" },
                report.muted,
                error::pack(error::ADMISSION, error::admission::NO_STATE_TREE),
            );
        }
        Half::Serve | Half::Starved => {
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
                "  compositor    outcome {}, image_bytes {}, heap_bytes {}",
                report.board.outcome,
                report.image,
                report.heap,
            );
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
/// # Safety
///
/// As [`demonstrate`]: the direct map must be live and cover every boot module.
unsafe fn found(boot: &crate::BootInfo) -> Option<(&'static [u8], Record)> {
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
/// Unit: bytes.
fn heap_declared(record: &Record) -> Option<u64> {
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
        tree_before: [0; 4],
        tree_after: 0,
        tree: [0; 4],
        admitted: declared.is_ok(),
        muted: refused.err().unwrap_or(0),
        image: image.len() as u64,
        heap: 0,
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
    // What the manifest declares, or — on the starved half — two pages, which is
    // the one number in this plan the component is asked to disbelieve.
    let described = match half {
        Half::Starved => STARVED_HEAP_BYTES,
        Half::Serve | Half::Mute => routing::HEAP_BYTES,
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
    let mut tree_before = [0u64; 4];
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
    let producer = Producer::new(client_end.channel()).ok_or(Trouble::Channel(0))?;
    let notices = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;
    let arena = client_end.arena();

    // The client, on the half that has one. A starved component ends before it
    // adopts anything, so a client that submitted would be waiting for a
    // completion from a component that has already exited — which is this
    // harness measuring its own bound rather than the refusal it came for.
    let driven = match half {
        Half::Serve | Half::Mute => drive(&producer, &reaper, &arena, tsc_khz),
        Half::Starved => Ok(Seen { submitted: 0, completed: 0, refused: 0 }),
    };

    // Told to stop whatever happened above, because a component left serving a
    // client that has gone is a core this boot never gets back.
    let told = notices.post(control::entry(control::notice::STOP, 0, 0, 0));
    // SAFETY: `start_on` was called for this core and nothing else has joined it.
    // The closure serves nothing: a compositor reaches no device, so there is
    // nothing it can ask the frame for while it runs.
    let joined = unsafe { crate::smp::join_serviced(cpu, tsc_khz, EXIT_MICROS, &mut || {}) };

    let reported = Board::of(&board);
    // The tree, read before `reap` gives the page back. RFC 0013's *read, never
    // delivered*, with the frame on the reading end.
    let reader =
        state::Reader::at(pages.own_tree, FRAME_SIZE as u32).map_err(Trouble::StateTree)?;
    let tree_after = reader.snapshot();
    let mut tree = [0u64; 4];
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
    let board = reported.ok_or(Trouble::BadReport)?;

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
        // Not this half's question. Nothing here admitted anything —
        // `prepare_server` is not a spawn — and claiming otherwise would put a
        // true-looking word in a report that had not earned it.
        admitted: false,
        muted: 0,
        image: image.len() as u64,
        heap: described,
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
    reaper: &Collector<'_>,
    arena: &Arena<'_>,
    tsc_khz: u64,
) -> Result<Seen, Trouble> {
    let mut seen = Seen { submitted: 0, completed: 0, refused: 0 };
    for delta in script() {
        let (entry, payload) = delta.encode();
        if !arena.copy_in(0, &payload) {
            return Err(Trouble::Refused);
        }
        if producer.submit(entry).is_err() {
            return Err(Trouble::Refused);
        }
        seen.submitted += 1;

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
    }
    Ok(seen)
}

/// Is this completion anything other than *the entry it names was accepted*?
///
/// Two things at once on purpose: a refusal, and an answer to a different entry.
/// A client that checked only the result would count a completion for entry
/// three as the answer to entry four and never notice.
fn answered_badly(answer: &Cqe, expected: u64) -> bool {
    answer.result < 0 || answer.user_data != expected
}
