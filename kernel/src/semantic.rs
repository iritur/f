// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The inversion boot: an application declares an interface across a ring, dies,
//! and the frame still holds what it declared while the right it held is gone.
//!
//! # What this file is and what it is not
//!
//! It is the **system's half** of `E3-B06c`. `user/panel` is the application: a
//! component that holds no tree, submits six entries and one commit, and ends.
//! What is here is everything the system does around one — open a tree in a page
//! of its own, hand the component a handle to it, serve the ring while the
//! component runs, apply each closed frame whole or not at all, and then reap
//! the component and go on holding the tree.
//!
//! It is **not** a second semantic tree. There is one, in `f-semantic`, in a
//! frame this file allocated and will not free; nothing here holds a node. What
//! this file knows about the interface is what `f_panel::script` says was
//! declared, which is deliberately a different kind of knowledge from the tree's
//! — two accounts, neither derived from the other, which is the only arrangement
//! in which a comparison means anything.
//!
//! # Where the tree lives, and why that is the argument rather than a detail
//!
//! In one frame the allocator handed this function, mapped into **no** address
//! space but the kernel's own. The component is spawned afterwards and its page
//! tables never name it: an application under section 11's inversion could not
//! reach its own declared tree if it tried, and this is where that stops being a
//! sentence.
//!
//! So `process::reap` frees the component's text, stack, board, heap, rings and
//! page tables, and does not free this. That is the whole of *a component that
//! dies leaves its declared tree readable*, and it is arranged rather than
//! observed: `kernel/src/state.rs` makes the same argument about the frame's own
//! tree — *a tree that could be freed would be a mapping a reader still holds* —
//! and this is that argument applied to a tree an application declared.
//!
//! # The two halves, and why neither means anything alone
//!
//! `semantic=declare` is the exit. The component declares four nodes, a child
//! order that is not the order the entries arrived in, a state and an intent;
//! the frame applies them; the component ends; the frame reaps it; and then the
//! frame does two things in the same breath. It reads the tree **by its
//! address** and finds every node the script declared, addressable by the
//! identifier its author chose. And it offers one more frame **by the handle**
//! the component was holding — the same number the component quoted back on its
//! board — and is refused.
//!
//! `TODO.md` says why one without the other proves nothing, and it is worth
//! repeating where the code is: a tree that survives its writer is what a log
//! does, and a handle that dies with its process is what every handle does.
//!
//! `semantic=deaf` is the control. The identical component is stood up with a
//! zero where its handle goes and must refuse **before it declares anything** —
//! `f_panel::routing::stopped::NO_HANDLE` — with the tree left empty. Without it
//! the refusal above would be a refusal that might refuse everything, and the
//! component's own check of its authority would be a guard nothing in this tree
//! kills.

use f_abi::manifest::Record;
use f_abi::semantic::{
    Commit, DeclareNode, Delta, Entry, Handshake, NO_INTENT, NO_NODE, PAYLOAD_BYTES, Received,
    Session,
};
use f_abi::{ABI_VERSION, Cqe, Sqe, error, state};
use f_interface::node::Role;
use f_panel::routing::{self, at, life, node, reported, stopped};
use f_panel::script;
use f_ring::{Arena, Consumer, Mapping, Poster, Window};
use f_semantic::tree::Rejected;
use f_semantic::{Handle, Interface, Opened, Registry, Staged, TreeId};

use crate::mem::{FRAME_SIZE, FrameAllocator};
use crate::paging;
use crate::process::{self, ServerPlan};

/// The frame's address for the board, and the component's, required to agree by
/// the machine rather than by two comments.
const _: () = assert!(crate::process::BOARD == routing::AT);

/// Entries in either ring. Unit: entries.
///
/// Sixteen, which is what `user/panel/manifest.toml` declares and what every
/// other channel in this tree is.
const ENTRIES: u32 = 16;

/// How long the frame waits for the component to end. Unit: microseconds.
const EXIT_MICROS: u64 = 500_000;

/// How many turns the component spends waiting for one completion before it
/// gives up. Unit: turns.
///
/// Large, because it is a backstop: the frame answers every entry from inside
/// the join, so a run that reaches this number is a run where the *frame*
/// stopped serving. RFC 0046 — a hang is a count.
const IDLE_SPINS: u64 = 1_000_000;

/// How many trees this boot's registry holds. Unit: trees.
///
/// Two, and the second is not spare. `f_semantic::Registry::open` never reuses a
/// slot a tree was kept in, so a boot that opened one tree and then wanted a
/// second would need a second slot — and the successor clause below does not: it
/// *adopts*, which is the same slot at a later generation, and a registry of one
/// would not tell the two apart. The second slot is what makes `adopt` visibly
/// not `open`.
const TREES: usize = 2;

/// Which half of the check this boot is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    /// Declare an interface, die, and leave it behind.
    Declare,
    /// Stand the identical component up holding no handle, and require it to
    /// refuse before it declares anything.
    Deaf,
}

impl Half {
    /// A word for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Declare => "declare",
            Self::Deaf => "deaf",
        }
    }

    /// The handle the frame writes on this half's board.
    ///
    /// The real one, or a zero — which is the *only* difference between the two
    /// halves and is what makes the control sharp: the same image, the same
    /// manifest, the same ring and the same script, and one word.
    const fn granted(self, handle: Handle) -> u64 {
        match self {
            Self::Declare => handle.bits() as u64,
            Self::Deaf => 0,
        }
    }

    /// What the component's own outcome word must say.
    const fn expects(self) -> u64 {
        match self {
            Self::Declare => stopped::SAID,
            Self::Deaf => stopped::NO_HANDLE,
        }
    }
}

/// Why the boot could not be run at all.
///
/// Distinct from a verdict: everything here is the harness failing to set the
/// experiment up, and none of it is evidence about the property.
#[derive(Clone, Copy, Debug)]
pub enum Trouble {
    /// No component file in the boot modules declares itself `panel`.
    NoComponent,
    /// Its manifest declares no heap, or declares one this build disagrees with.
    HeapDisagrees,
    /// A ring could not be described or adopted, carrying the packed refusal.
    Channel(i32),
    /// The registry had no slot, which is this file's own arithmetic being
    /// wrong rather than anything about a component.
    NoTree,
    /// The component could not be prepared, started or reaped.
    Process(process::Error),
    /// A core was not handed back inside the bound.
    Overdue,
    /// The component's state tree could not be published or read.
    StateTree(i32),
    /// The component's board did not carry its magic, so nothing on it was
    /// believed.
    BadReport,
    /// This build's own vocabulary did not admit this build's own roles.
    NoScript,
}

impl Trouble {
    /// A line for the log.
    #[must_use]
    pub const fn why(self) -> &'static str {
        match self {
            Self::NoComponent => "no boot module declares itself `panel`",
            Self::HeapDisagrees => {
                "the panel's manifest and `f_panel::routing` disagree about the heap"
            }
            Self::Channel(_) => "a ring could not be described or adopted",
            Self::NoTree => "the registry had no slot to open a tree in",
            Self::Process(_) => "the component could not be prepared, started or reaped",
            Self::Overdue => "the component did not give its core back inside the bound",
            Self::StateTree(_) => "the component's state tree could not be published or read",
            Self::BadReport => "the component's board did not carry its magic",
            Self::NoScript => "this build's vocabulary did not admit this build's own roles",
        }
    }
}

/// What the component said about itself, read out of memory the frame granted
/// it.
///
/// RFC 0013's *read, never delivered*: it was never asked.
#[derive(Clone, Copy, Debug, Default)]
pub struct Board {
    /// Entries it put on the ring. Unit: entries.
    pub submitted: u64,
    /// Completions it reaped. Unit: entries.
    pub answered: u64,
    /// Completions carrying a refusal. Unit: entries.
    pub refused: u64,
    /// The handle it was told it holds, quoted back.
    /// Unit: none — a capability handle's bits.
    pub handle: u64,
    /// Why its run ended, as a `f_panel::routing::stopped` ordinal.
    pub outcome: u64,
    /// How many of its declared state nodes it wrote a word into. Unit: nodes.
    pub published: u64,
}

impl Board {
    /// Read the component's half of its routing page, or `None` where it never
    /// finished writing one.
    fn of(board: &Window) -> Option<Self> {
        if board.read64(reported::MAGIC).ok()? != routing::MAGIC {
            return None;
        }
        Some(Self {
            submitted: board.read64(reported::SUBMITTED).ok()?,
            answered: board.read64(reported::ANSWERED).ok()?,
            refused: board.read64(reported::REFUSED).ok()?,
            handle: board.read64(reported::HANDLE).ok()?,
            outcome: board.read64(reported::OUTCOME).ok()?,
            published: board.read64(reported::PUBLISHED).ok()?,
        })
    }
}

/// What the frame found when it read the tree back after the component was
/// gone.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kept {
    /// Nodes the tree holds. Unit: nodes.
    pub live: u64,
    /// Nodes of `f_panel::script::DECLARED` found under the role the script
    /// declared them with, by identity. Unit: nodes.
    pub addressable: u64,
    /// Which of them, one bit per entry of `f_panel::script::DECLARED` at that
    /// entry's own position.
    ///
    /// **Carried so that the log can say which node is missing rather than how
    /// many are.** The first version of `report_lines` printed *node 1 is a
    /// surface and is still there* out of the script, unconditionally, and a
    /// mutation that emptied the tree produced four such lines above a count of
    /// zero — a log that stated the thing under test as though it were a fact.
    /// Unit: none — a bit set, not a quantity.
    pub found: u32,
    /// Whether the command's state is the one the dead component left it in.
    pub state_held: bool,
    /// Whether the command still carries the intent it was declared with.
    pub intent_held: bool,
    /// Whether the label sits in front of the command, which is the child order
    /// the script asked for and not the order the entries arrived in.
    pub order_held: bool,
    /// Frames that closed into the tree. Unit: frames.
    pub frames: u64,
    /// Entries that reached it. Unit: entries.
    pub edits: u64,
}

/// What this boot did and what it found.
pub struct Report {
    /// Which half.
    pub half: Half,
    /// What the component said about itself.
    pub board: Board,
    /// Nodes in the schema the frame published for it. Unit: nodes.
    pub tree_nodes: u64,
    /// The four words it wrote into that tree, in `node::WRITTEN` order.
    pub tree: [u64; 4],
    /// The handle bits the frame wrote on this half's board — the real handle,
    /// or a zero on the control half.
    /// Unit: none — a capability handle's bits.
    pub granted: u64,
    /// How the component's process ended, as a status word.
    /// Unit: none — the status it passed to `door::EXIT`.
    pub exited: u64,
    /// Whether there was a living writer for the tree when the frame closed it.
    pub had_writer: bool,
    /// Which tree it named. Unit: none — a registry slot index.
    pub slot: u16,
    /// The generation the slot stood at while the component ran.
    /// Unit: none — a generation ordinal.
    pub generation_before: u16,
    /// The generation it stands at now that the component is gone.
    /// Unit: none — a generation ordinal.
    pub generation_after: u16,
    /// The tree, read by its address after the component was reaped.
    pub kept: Kept,
    /// What the frame's posthumous frame earned, packed. Zero means it was
    /// applied, which fails the verdict.
    pub posthumous: i32,
    /// The tree, read again after the posthumous frame was refused.
    ///
    /// Beside [`Report::kept`] rather than instead of it, because *the tree is
    /// unchanged* is a comparison and a single reading cannot be one.
    pub after: Kept,
    /// Nodes the successor added over the tree it inherited. Unit: nodes.
    pub heir_added: u64,
    /// What the dead component's handle earned against the **open** slot the
    /// successor holds, packed.
    pub heir_refused_ghost: i32,
    /// How long the component's image is. Unit: bytes.
    pub image: u64,
}

impl Report {
    /// Is this what the half asked for?
    ///
    /// # Errors
    ///
    /// A sentence naming the clause that did not hold.
    pub fn verdict(&self) -> Result<(), &'static str> {
        if self.board.outcome != self.half.expects() {
            return Err("the component did not end the way this half requires");
        }
        if self.board.handle != self.granted {
            return Err(
                "the component quoted back a handle that is not the one the frame granted it",
            );
        }
        // The board and the process's exit status, required to agree. They are
        // two accounts of one thing taken on opposite sides of the boundary —
        // the board is memory the component wrote and the status is what the
        // frame recorded when it asked to end — so a component that reported one
        // outcome and ended with another is caught rather than believed.
        if self.exited != self.board.outcome {
            return Err(
                "the component's board and the status it ended with do not say the same thing",
            );
        }
        if !self.had_writer {
            return Err(
                "the frame closed a tree that had no living writer, so the close below is about                  something other than this component ending",
            );
        }
        if self.board.published != node::WRITTEN.len() as u64 {
            return Err("the component wrote fewer words into its own tree than it declares nodes");
        }
        match self.half {
            Half::Deaf => self.deaf(),
            Half::Declare => self.declared(),
        }
    }

    /// The control's clauses.
    fn deaf(&self) -> Result<(), &'static str> {
        if self.board.submitted != 0 {
            return Err(
                "a component told it holds no handle submitted an entry anyway, so its check of \
                 its own authority is decoration",
            );
        }
        if self.kept.live != 0 {
            return Err("nothing declared anything and the tree is not empty");
        }
        Ok(())
    }

    /// The exit's clauses, in the order they are argued.
    fn declared(&self) -> Result<(), &'static str> {
        let entries = script::ENTRIES as u64;
        if self.board.submitted != entries || self.board.answered != entries {
            return Err("the component did not submit and reap every entry of its script");
        }
        if self.board.refused != 0 {
            return Err("the frame refused an entry of a script both sides build from one crate");
        }
        // The frame's own account of what it applied, which is not the
        // component's: the component knows it was answered and knows nothing
        // about what the answer did.
        if self.kept.frames != 1 {
            return Err("the frame did not close exactly the one frame the script commits");
        }
        if self.kept.edits != entries - 1 {
            return Err(
                "the entries that reached the tree are not the script's, less the commit that \
                 closed it",
            );
        }

        // --- half one: readable and addressable -------------------------
        if self.kept.live != script::DECLARED.len() as u64 {
            return Err("the tree the dead component declared is not there, or is not all there");
        }
        if self.kept.addressable != script::DECLARED.len() as u64 {
            return Err(
                "a node the script declared does not answer to the identifier its author chose, \
                 which is a dump rather than an addressable tree",
            );
        }
        if !self.kept.state_held {
            return Err("the tree forgot how the dead component left its command");
        }
        if !self.kept.intent_held {
            return Err("the tree forgot what operating the dead component's command does");
        }
        if !self.kept.order_held {
            return Err(
                "the tree's child order is the order the entries arrived in, so `before` reached \
                 nothing",
            );
        }

        // --- half two: the handle did not survive -----------------------
        if self.posthumous == 0 {
            return Err(
                "the handle the dead component held still writes the tree, so nothing about it \
                 ended when the component did",
            );
        }
        if self.generation_after == self.generation_before {
            return Err("the slot's generation did not move when its writer ended");
        }
        // Both halves at once, which is the clause `TODO.md` says either one
        // alone fails to be: the tree the refused frame could not touch is the
        // same tree, node for node, that the dead component left.
        if self.after.live != self.kept.live
            || self.after.addressable != self.kept.addressable
            || self.after.frames != self.kept.frames
            || self.after.edits != self.kept.edits
        {
            return Err("a frame that was refused changed the tree anyway");
        }

        // --- and the generation is what refused it ----------------------
        if self.heir_added != 1 {
            return Err("the successor did not write the tree it inherited");
        }
        if self.heir_refused_ghost == 0 {
            return Err(
                "the dead component's handle reached a slot a successor had opened, so what \
                 refused it before was the slot being shut and not the generation",
            );
        }
        Ok(())
    }
}

/// Print what happened, one subject per line.
pub fn report_lines(report: &Report) {
    crate::kprintln!(
        "  semantic      an application declaring an interface it does not own, and the {} half: \
         {}",
        report.half.name(),
        match report.half {
            Half::Declare =>
                "the component declares, dies, and the frame still holds what it declared",
            Half::Deaf =>
                "the identical component holding no handle, which must refuse before it declares",
        }
    );
    crate::kprintln!(
        "  semantic      {} entr(y/ies) submitted, {} answered, {} refused, handle {:#010x} over \
         tree {}",
        report.board.submitted,
        report.board.answered,
        report.board.refused,
        report.board.handle,
        report.slot,
    );
    crate::kprintln!(
        "  semantic      state tree {} node(s) from the manifest, {} written by the component: \
         submitted {}, answered {}, refused {}, handle {:#010x}",
        report.tree_nodes,
        report.board.published,
        report.tree[0],
        report.tree[1],
        report.tree[2],
        report.tree[3],
    );
    if report.half == Half::Declare {
        crate::kprintln!(
            "  semantic      after the component was reaped: {} node(s) kept, {} addressable by \
             the identifier their author chose, {} frame(s), {} edit(s)",
            report.kept.live,
            report.kept.addressable,
            report.kept.frames,
            report.kept.edits,
        );
        for (at, (id, role)) in script::DECLARED.into_iter().enumerate() {
            // Read out of the census and never out of the script, which is what
            // the script would say whatever the tree held.
            crate::kprintln!(
                "  semantic      node {id} was declared a {} and {}",
                role.name(),
                if report.kept.found & (1 << at) != 0 {
                    "is still there, under that role, by that identifier"
                } else {
                    "IS NOT THERE under that role"
                },
            );
        }
        crate::kprintln!(
            "  semantic      the dead component's handle: {} — generation {} then {}; the tree \
             after it: {} node(s), {} frame(s)",
            report.posthumous,
            report.generation_before,
            report.generation_after,
            report.after.live,
            report.after.frames,
        );
        crate::kprintln!(
            "  semantic      a successor adopted the tree and added {} node(s); the dead \
             handle against that open slot: {}",
            report.heir_added,
            report.heir_refused_ghost,
        );
    }
    crate::kprintln!(
        "  semantic      outcome {}, image_bytes {}",
        report.board.outcome,
        report.image,
    );
}

/// Find the component file this boot is about, by the name its manifest
/// declares.
///
/// By name and not by position, for `compositor::found`'s reason: the loader's
/// order is the loader's, `user/generation.toml` sorts one way and `xtask`'s
/// `COMPONENTS` another, and a boot that took a module by index would be reading
/// whichever component happened to be built first.
///
/// # Safety
///
/// As [`demonstrate`]: the direct map must be live and cover every boot module.
unsafe fn found(boot: &crate::BootInfo) -> Option<(&'static [u8], Record)> {
    // SAFETY: the caller's guarantee.
    let (modules, count) = unsafe { crate::component::modules(boot) };
    for module in modules.iter().take(count) {
        let Ok(record) = Record::read(module) else { continue };
        if record.name.starts_with(b"panel") && record.name.get(5) == Some(&0) {
            let image = record.image(module).ok()?;
            return Some((image, *record));
        }
    }
    None
}

/// How much heap this record declares.
///
/// `None` for a record with no such need, which is a component the spawn shape
/// cannot describe a heap for — refused before anything is spent.
/// Unit: bytes.
fn heap_declared(record: &Record) -> Option<u64> {
    for need in record.needs() {
        if need.name.starts_with(b"heap") && need.name.get(4) == Some(&0) {
            return Some(need.bytes);
        }
    }
    None
}

/// What the caller must tell this boot about the machine.
pub struct Scheduling {
    /// The frame's own state tree, threaded the way `kernel/src/state.rs`
    /// threads it. Unit: bytes, physical.
    pub tree: u64,
    /// The core the component runs on.
    pub cpu: usize,
    /// The timer's rate. Unit: hertz.
    pub hz: u32,
    /// How many ticks the component may have. Unit: ticks.
    pub target: u64,
    /// The time-stamp counter's rate, for the bounded waits.
    /// Unit: kilohertz.
    pub tsc_khz: u64,
}

/// Stand `user/panel` up, let it declare, reap it, and then ask the tree and the
/// handle the two questions this task exists to ask.
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

    // The manifest and the crate are required to be one number, for
    // `compositor::demonstrate`'s reason: nothing links a `routing` constant to
    // a manifest field, so a manifest edited downwards would be a component
    // whose heap the frame described at a size the component did not expect.
    if heap_declared(&record).ok_or(Trouble::HeapDisagrees)? != routing::HEAP_BYTES {
        return Err(Trouble::HeapDisagrees);
    }

    // **The tree's page, and it is allocated before anything else.** It is never
    // put in a `ServerPlan`, never mapped into the component's address space and
    // never handed to `reap`, which is the whole of *the system owns it*: the
    // component that is about to declare into it could not name its address.
    let page = frames.alloc_zeroed(crate::mem::Order::FRAME).ok_or(Trouble::NoTree)?;
    let registry = frames.virt(page).cast::<Registry<TREES>>();
    // SAFETY: `page` is a frame this function allocated zeroed, is frame-aligned
    // — stronger than `Registry`'s alignment, which is a `u16`'s — and is
    // `FRAME_SIZE` bytes, which the assertion below shows is room for the value
    // being written. Nothing else holds a pointer into it: it was allocated on
    // this line and is not in `prepared.pages`, so nothing frees it either.
    unsafe { registry.write(Registry::EMPTY) };
    // SAFETY: as above, and the reference produced is the only one that ever
    // exists over these bytes — this function makes no second one and hands none
    // out. It outlives `frames` deliberately: this page is not given back, which
    // `kernel/src/state.rs` argues at length for the frame's own tree.
    let registry: &mut Registry<TREES> = unsafe { &mut *registry };

    // SAFETY: the caller's guarantee, passed down unchanged, plus the tree being
    // a page this function owns and does not free.
    unsafe { run(frames, kernel, features, half, &record, image, on, registry) }
}

/// The room a registry of [`TREES`] needs, against the page it is written into.
///
/// A compile-time assertion rather than a bound check, because the alternative
/// is a write past a frame the allocator handed out — which is the one failure
/// this whole file's argument about ownership cannot survive.
const _: () = assert!(core::mem::size_of::<Registry<TREES>>() <= FRAME_SIZE as usize);

/// Everything after the tree exists.
///
/// # Safety
///
/// As [`demonstrate`].
#[expect(
    clippy::too_many_arguments,
    reason = "the split from `demonstrate` is what keeps the `unsafe` page write in a function a \
              reader can check on its own; folding the arguments into a struct would move them \
              away from the one call that consumes them"
)]
unsafe fn run(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    half: Half,
    record: &Record,
    image: &'static [u8],
    on: Scheduling,
    registry: &mut Registry<TREES>,
) -> Result<Report, Trouble> {
    let Scheduling { tree, cpu, hz, target, tsc_khz } = on;
    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0))?;

    // The tree, opened before the component exists. The handle goes on the
    // component's board and the address stays here — which is the two types of
    // `f_semantic` put where they belong rather than described.
    let Opened { id, handle } = registry.open().ok_or(Trouble::NoTree)?;
    let generation_before = registry.generation(id);

    // The ring. The frame writes the header and takes the **server's** end; the
    // component adopts the client's. That is the opposite of every other boot in
    // this tree, and it is what an application is.
    let wire = frames.alloc_zeroed(crate::mem::Order::FRAME).ok_or(Trouble::Channel(0))?;
    let at_wire = frames.virt(wire);
    // SAFETY: `wire` was allocated zeroed just above, is frame-aligned — stronger
    // than the cache line the layout asks for — and is `FRAME_SIZE` bytes with no
    // pointer into it held anywhere else.
    let _ =
        unsafe { Mapping::describe(at_wire, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and every
    // accessor hands out atomics and `UnsafeCell`s rather than references.
    let server_end = unsafe { Mapping::adopt(at_wire, bytes, 0, 0) }.map_err(Trouble::Channel)?;

    // SAFETY: the caller's guarantee about `kernel`, `frames` and `cpu`, plus
    // `wire` being a frame this function allocated and holds the far end of.
    let (prepared, pages) = unsafe {
        process::prepare_server(
            frames,
            kernel,
            features,
            ServerPlan {
                image,
                selector: life::DECLARE,
                tree,
                hz,
                target,
                cpu,
                data: wire.addr(),
                // No registered buffer region: this component's payloads travel
                // in the channel's own arena — `payload = "inline"` — so a
                // region here would be a page nobody ever addresses.
                buffers: 0,
                buffer_bytes: 0,
                heap_bytes: routing::HEAP_BYTES,
                // RFC 0065: every component publishes a tree. This is the
                // monitoring one, in the component's own address space, and it
                // is the one `reap` takes back — which is the contrast this boot
                // is about, one page apart.
                own_tree: true,
            },
        )
    }
    .map_err(Trouble::Process)?;

    let tree_nodes = crate::component::publish_tree(pages.own_tree as *mut u8, record)
        .map_err(|_| Trouble::StateTree(0))?;

    // --- what the component is told -----------------------------------------
    let board = Window::at(pages.board, routing::BYTES).map_err(Trouble::Channel)?;
    for (offset, value) in [
        (at::DATA_AT, crate::process::BLK_DATA),
        (at::DATA_LEN, u64::from(bytes)),
        (at::TREE_AT, crate::process::SPAWN_TREE),
        (at::TREE_HANDLE, half.granted(handle)),
        (at::TREE_SLOT, u64::from(id.index())),
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
    unsafe { crate::smp::start_on(cpu) }.map_err(|_| Trouble::Overdue)?;

    let asks = Consumer::new(server_end.channel()).ok_or(Trouble::Channel(0))?;
    let answers = Poster::new(server_end.completions()).ok_or(Trouble::Channel(0))?;
    let arena = server_end.arena();
    // A block, so that the reborrow the server holds of the registry ends before
    // the lines below take it back. It is the borrow checker's version of the
    // same fact the rest of this file is about: while the component is running,
    // the thing that may write its tree is the channel it submits on, and
    // afterwards it is nobody.
    let joined = {
        let mut serving = Serving {
            asks: &asks,
            answers: &answers,
            arena: &arena,
            registry: &mut *registry,
            handle,
            session: Session::opening(server_end.epoch()),
            staged: Staged::EMPTY,
        };

        // SAFETY: `start_on` was called for this core and nothing else has
        // joined it. The closure touches only the frame's end of one channel,
        // which is single-producer and single-consumer by construction, and the
        // registry, which is in a page this core allocated and no other core can
        // name.
        unsafe { crate::smp::join_serviced(cpu, tsc_khz, EXIT_MICROS, &mut || serving.serve()) }
    };

    let reported = Board::of(&board);
    // The component's own state tree, read before `reap` gives the page back.
    // **That sentence is the whole contrast this boot is drawn around**: this
    // tree has to be read before the component's memory goes, and the semantic
    // tree does not, because it was never the component's memory.
    let reader =
        state::Reader::at(pages.own_tree, FRAME_SIZE as u32).map_err(Trouble::StateTree)?;
    let mut tree_words = [0u64; 4];
    for (slot, id) in tree_words.iter_mut().zip(node::WRITTEN) {
        // `u64::MAX` for an id the schema does not carry, so a manifest and
        // `routing::node` that disagree fail a clause rather than passing it
        // with a zero nobody wrote.
        *slot = reader.value(id).unwrap_or(u64::MAX);
    }

    // **Checked before anything is torn down**, for `compositor::serve`'s
    // reason: a join that did not return is a core still inside this component,
    // and `reap` would give its address space back while an instruction pointer
    // was in it.
    joined.map_err(|_| Trouble::Overdue)?;

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished — which is what the join above returning `Ok` means.
    let ended = unsafe { process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    // SAFETY: allocated by this function, the component that was lent it has
    // exited — the join is what says so — and nothing else holds a pointer into
    // it.
    unsafe { frames.free(wire) };

    // --- the component is gone ----------------------------------------------
    //
    // **Here, and not earlier.** The tree is closed at the same place the
    // address space is freed, because that is the moment the writer stopped
    // existing — a close before the join would be closing a tree a running core
    // could still be writing into, and a close that never happened would be a
    // handle that outlived its holder because nobody told the registry.
    let had_a_writer = registry.close(id);
    let generation_after = registry.generation(id);

    let kept = census(registry, id);

    // Half two. One more frame, built and sealed by the frame's own session out
    // of entries nothing is wrong with, offered by the **same handle value** the
    // component quoted back on its board — which is what makes this the dead
    // component's right rather than a number this function invented.
    let ghost = posthumous(registry, handle)?;
    let after = census(registry, id);

    // And the generation is what refused it. A successor adopts the tree, so the
    // slot is open and writable; the dead handle is offered against it again and
    // must still find nothing. Without this clause the refusal above would be
    // held by the slot being shut, and a build whose generation check had been
    // deleted would pass everything else on this page.
    let (heir_added, heir_refused_ghost) = successor(registry, id, handle)?;

    let board = reported.ok_or(Trouble::BadReport)?;
    Ok(Report {
        half,
        board,
        tree_nodes: u64::from(tree_nodes),
        tree: tree_words,
        granted: half.granted(handle),
        exited: match ended.death {
            process::Death::Exited(status) => status,
            // Not a status. A component that was killed ended with an exception
            // and no word of its own, and folding that into the same field as a
            // status would make the verdict's comparison read a vector as an
            // outcome. `u64::MAX` is not a `stopped` ordinal and never will be.
            process::Death::Running | process::Death::Killed { .. } => u64::MAX,
        },
        had_writer: had_a_writer,
        slot: id.index(),
        generation_before,
        generation_after,
        kept,
        posthumous: ghost,
        after,
        heir_added,
        heir_refused_ghost,
        image: image.len() as u64,
    })
}

/// The frame's end of the channel, and what it applies entries into.
struct Serving<'a> {
    /// Entries the component has submitted.
    asks: &'a Consumer<'a>,
    /// Where their completions go.
    answers: &'a Poster<'a>,
    /// Where their payloads are.
    arena: &'a Arena<'a>,
    /// The trees the system holds.
    registry: &'a mut Registry<TREES>,
    /// The right this channel's peer holds.
    handle: Handle,
    /// The agreement, the frame under assembly and the poison. RFC 0083.
    session: Session,
    /// The entries of the frame being assembled.
    staged: Staged,
}

impl Serving<'_> {
    /// Take whatever has arrived, and answer each entry.
    ///
    /// Called from inside the join, so it runs while the component runs: an
    /// application that submits and waits is an application the frame has to
    /// answer *during* the run, not after it.
    fn serve(&mut self) {
        loop {
            let Ok(Some(entry)) = self.asks.pop() else { return };
            let answer = self.one(&entry);
            if self.answers.free().unwrap_or(0) == 0 {
                // No room to answer in. The component is waiting for a
                // completion that will never come and will end on its own
                // bound, which is `stopped::UNANSWERED` and is a different
                // outcome from every other — so nothing is invented here.
                return;
            }
            let _ = self.answers.post(answer);
        }
    }

    /// Offer one entry to the session, and apply the frame a commit closes.
    fn one(&mut self, entry: &Sqe) -> Cqe {
        // The payload, out of the ring's own arena. **Zeroed first and offered
        // whatever the copy answered**: an entry framing a payload that is not
        // in the arena is a delta whose bytes never landed, and the decoder
        // refuses a zeroed payload — which poisons the frame it belongs to, so
        // the frame that lost an entry can never close. A refusal invented here
        // instead would answer the peer and leave the session believing it still
        // had a whole frame.
        let mut payload = [0u8; PAYLOAD_BYTES];
        let _ = self.arena.copy_out(entry.offset as usize, &mut payload);

        match self.session.accept(entry, &payload, &Interface) {
            Ok(Received::Staged(delta)) => match self.staged.offer(delta) {
                Ok(()) => accepted(entry),
                Err(why) => self.refuse(entry, why),
            },
            Ok(Received::Frame { key, .. }) => {
                // **The apply path RFC 0083 names as this task's**, and the only
                // call to it in the frame. The key came from the commit and
                // cannot have come from anywhere else.
                let applied = self.registry.apply(self.handle, &self.staged, key);
                self.staged.clear();
                match applied {
                    Ok(_) => accepted(entry),
                    Err(why) => self.refuse(entry, why),
                }
            }
            Err(fault) => Cqe {
                user_data: entry.user_data,
                result: fault.refusal.packed(),
                ext: fault.refusal.detail(),
                ..Cqe::ZERO
            },
        }
    }

    /// Answer one entry the tree would not take.
    fn refuse(&self, entry: &Sqe, why: Rejected) -> Cqe {
        Cqe { user_data: entry.user_data, result: packed(why), ext: why.node(), ..Cqe::ZERO }
    }
}

/// *The entry you submitted was accepted.*
///
/// No timestamp, and the zero is a statement rather than an omission: nothing
/// here reads a clock, and a completion that carried one would be inventing a
/// reading this boot cannot reproduce. `cargo xtask trace` hashes this log.
fn accepted(entry: &Sqe) -> Cqe {
    Cqe { user_data: entry.user_data, result: 0, ..Cqe::ZERO }
}

/// A tree's refusal as a packed error.
///
/// `RESOURCE` for the two that are about room and `ARGUMENT` for the rest, which
/// is `f_compositor::tree::packed`'s split one layer up and for its reason: a
/// client told `ARGUMENT` goes looking for a field it got wrong, and a full tree
/// is not one.
fn packed(why: Rejected) -> i32 {
    match why {
        Rejected::Full | Rejected::Capacity => {
            error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED)
        }
        // **The one refusal this task exists to produce.** `AUTHORITY/REVOKED`
        // rather than anything in `ARGUMENT`, because nothing about the frame is
        // wrong and what is wrong is who is offering it — and `REVOKED` rather
        // than `NO_SUCH_CAP` because that code's own comment says the two need
        // different handling: a right that ended is a peer that should stop, and
        // a right that never was is a peer with a bug.
        Rejected::Stale => error::pack(error::AUTHORITY, error::authority::REVOKED),
        Rejected::Vacant => error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
        Rejected::NotAnEdit
        | Rejected::Declared(_)
        | Rejected::NoSuchNode(_)
        | Rejected::NoSuchParent(_)
        | Rejected::NotASibling(_)
        | Rejected::NotStored(_) => error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
    }
}

/// Read the tree at that address and say what a reader would find.
///
/// **Takes a [`TreeId`] and no handle**, which is the half of the exit this
/// function is: addressing a tree buys nothing towards writing it, and there is
/// no function in `f-semantic` that turns one into the other.
fn census(registry: &Registry<TREES>, id: TreeId) -> Kept {
    let Some(tree) = registry.read(id) else { return Kept::default() };
    let mut kept = Kept {
        live: tree.live() as u64,
        frames: tree.frames(),
        edits: tree.edits(),
        ..Kept::default()
    };
    for (at, (node, role)) in script::DECLARED.into_iter().enumerate() {
        let Some(held) = tree.node(node) else { continue };
        // By identity and then by role, which is what *addressable* means: the
        // node answers to the identifier its author chose and is the thing it
        // was declared to be. A walk over storage would satisfy the count above
        // and not this.
        if Role::from_index(held.role().get() as usize) == Some(role) {
            kept.addressable += 1;
            kept.found |= 1 << at;
        }
    }
    if let Some(command) = tree.node(script::COMMAND) {
        kept.state_held =
            command.state().map(f_abi::semantic::StateBits::get) == Some(script::STATE.bits());
        kept.intent_held = command.intent() == script::INTENT;
        if let Some(label) = tree.node(script::LABEL) {
            // The script declared the label first and then asked the command to
            // sit in front of it, so the command's rank is the lower one. A
            // receiver that appended everything would have them the other way
            // round and would pass every count above.
            kept.order_held = command.rank() < label.rank() && command.parent() == label.parent();
        }
    }
    kept
}

/// Offer one more frame by the dead component's handle.
///
/// The entries are the frame's own and there is nothing wrong with them: a
/// declaration of one node under a node the tree already holds, and a commit.
/// What is wrong is who is offering it, and that is the only thing this can be
/// refused for.
///
/// # Errors
///
/// [`Trouble::NoScript`] where this build's vocabulary does not admit its own
/// roles, which is a build failure and not evidence.
fn posthumous(registry: &mut Registry<TREES>, handle: Handle) -> Result<i32, Trouble> {
    let (staged, key) = one_more(&mut Session::opening(1))?;
    Ok(match registry.apply(handle, &staged, key) {
        Ok(_) => 0,
        Err(why) => packed(why),
    })
}

/// Hand the tree to a successor, and offer the dead handle against it again.
///
/// Answers what the successor added and what the ghost earned. The second number
/// is the one that matters: after an adopt the slot is **open**, so a refusal
/// here is the generation and nothing else.
///
/// # Errors
///
/// As [`posthumous`].
fn successor(
    registry: &mut Registry<TREES>,
    id: TreeId,
    dead: Handle,
) -> Result<(u64, i32), Trouble> {
    let Some(heir) = registry.adopt(id) else { return Ok((0, 0)) };
    let (staged, key) = one_more(&mut Session::opening(1))?;
    let added = registry.apply(heir, &staged, key).map_or(0, |applied| applied.declared);
    let (staged, key) = one_more(&mut Session::opening(1))?;
    let ghost = match registry.apply(dead, &staged, key) {
        Ok(_) => 0,
        Err(why) => packed(why),
    };
    Ok((added, ghost))
}

/// One well-formed frame, staged and sealed, declaring a fifth node.
///
/// The `Sealed` is minted by the commit and by nothing else — there is no
/// constructor for one outside `f_abi::semantic` — so this function is the whole
/// of how the frame gets a key, and it gets one the same way a peer would.
///
/// # Errors
///
/// [`Trouble::NoScript`] where the vocabulary refuses the role, and
/// [`Trouble::NoScript`] again where the session refuses an entry this function
/// wrote — both are this build disagreeing with itself.
fn one_more(session: &mut Session) -> Result<(Staged, f_abi::semantic::Sealed), Trouble> {
    let agreed = script::agreement().map_err(|_| Trouble::NoScript)?;
    let role = agreed
        .admit_role(u16::try_from(Role::Label.index()).unwrap_or(u16::MAX), &Interface)
        .map_err(|_| Trouble::NoScript)?;
    let mut staged = Staged::EMPTY;
    for body in [
        Entry::DeclareVocabulary(Handshake::HERE),
        Entry::DeclareNode(DeclareNode {
            node: GHOST_NODE,
            parent: script::SURFACE,
            before: NO_NODE,
            role,
            intent: NO_INTENT,
        }),
    ] {
        let (entry, payload) = crossing(body);
        match session.accept(&entry, &payload, &Interface) {
            Ok(Received::Staged(delta)) => {
                staged.offer(delta).map_err(|_| Trouble::NoScript)?;
            }
            _ => return Err(Trouble::NoScript),
        }
    }
    let (entry, payload) = crossing(Entry::Commit(Commit { frame_token: GHOST_FRAME }));
    match session.accept(&entry, &payload, &Interface) {
        Ok(Received::Frame { key, .. }) => Ok((staged, key)),
        _ => Err(Trouble::NoScript),
    }
}

/// The node a posthumous frame declares.
///
/// Outside `f_panel::script`'s range, so that a tree holding it is a tree
/// something other than the component wrote into — which is what the census
/// would show if the refusal had not held.
/// Unit: none — a node identifier.
const GHOST_NODE: u64 = 99;

/// The frame token a posthumous frame carries.
/// Unit: none — a frame identifier, not a quantity.
const GHOST_FRAME: u64 = 0x99;

/// One entry, as it crosses.
fn crossing(body: Entry) -> (Sqe, [u8; PAYLOAD_BYTES]) {
    Delta { user_data: 1, class: 0, payload_offset: 0, flags: 0, body }.encode()
}
