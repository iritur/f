// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Where this component's needs landed, written by the frame and read by the
//! component, in one page neither of them has to guess the shape of.
//!
//! # Why a component is told and does not compute
//!
//! `user/virtio-blk/src/routing.rs` makes that argument in full and the two
//! driver crates after it record what writing it again taught. This file is the
//! fourth copy of the layout constant and the first that is not a driver's, so
//! the one sentence worth adding is about what did *not* have to change: a
//! component that drives no device finds its board at the same address as one
//! that does, because the address belongs to the shape the frame builds and not
//! to what the component does inside it.
//!
//! [`AT`] equals `f_virtio_blk::routing::AT`, `f_virtio_net::routing::AT` and
//! `f_virtio_gpu::routing::AT`, and all four equal `kernel::process::BOARD`.
//! `kernel/src/compositor.rs` asserts it at compile time rather than saying it
//! in a comment. RFC 0051 said at two drivers that this constant belongs in
//! `abi/` and RFC 0054 said it again at three; a fourth crate repeating it is
//! an argument for the move rather than for this file.
//!
//! # The second half is the component's, and the frame only reads it
//!
//! Offsets from [`REPORT`] up are written by the *component* and read by the
//! frame after the run. RFC 0013's *read, never delivered*.
//!
//! **What this component reports is not what it publishes.** The board is this
//! component's answer to *what did you do*, read once when its core comes back;
//! the state tree at [`at::TREE_AT`] is the machine's answer to *what is it
//! running*, and it is there for a reader who never saw this boot. Both carry
//! the same counters out of one set of numbers, so they cannot disagree without
//! one of them being wrong — which is the comparison `kernel/src/compositor.rs`
//! makes, and the reason this component writes both rather than choosing.

/// Which of this component's lives the frame asked for, in the low half of
/// `f_abi::door::Entry`.
///
/// Two, and there is deliberately no third. The negative control this task
/// needs is a *build whose manifest declares no state tree*, which is a fact
/// about a manifest and not about this component — a selector for it would be a
/// compositor with a mode in which it publishes nothing, which is a different
/// experiment wearing the control's name. `kernel/src/compositor.rs` drives the
/// real one, through the admission the frame already performs.
pub mod life {
    /// Announce and end. What a spawn into a place asks for.
    pub const ANNOUNCE: u32 = 0;
    /// Adopt both rings, hold the graph, and serve until the frame says stop.
    pub const SERVE: u32 = 1;
}

/// Where the frame maps this page in the component's address space.
///
/// Must equal `kernel::process::BOARD`. Unit: bytes, in the component's own
/// address space.
pub const AT: u64 = 0x0041_8000;

/// How many bytes the page is. One frame.
/// Unit: bytes.
pub const BYTES: u32 = 4096;

/// How many bytes the component's own state tree occupies.
///
/// One frame, which is what `kernel::component::publish_tree` writes into and
/// what `f_abi::state::Reader` is given to read back. RFC 0013 fixes the region
/// at one frame and `abi::manifest::STATE_NODES_MAX` is sized against it, so a
/// second number here would be a component disagreeing with the format.
/// Unit: bytes.
pub const TREE_BYTES: u32 = 4096;

/// A word the frame writes first and the component checks before it believes
/// anything else here.
///
/// R04, at the one place a component reads a structure it did not build: a page
/// of zeroes is what an unmapped-and-then-mapped frame looks like, and a
/// component that took a zero for a length would refuse rather than fault —
/// which reads as a peer problem. The magic makes *the frame did not fill this
/// in* a distinct answer from *the frame said zero*.
///
/// Different from the three drivers', and it has to be: every component in this
/// tree is mapped at [`AT`] by the same shape, so a build that routed the wrong
/// image into the wrong supervisor would otherwise find a page whose magic
/// matched and whose fields meant something else.
pub const MAGIC: u64 = 0x636F_6D70_726F_7574;

/// How much heap this component's manifest declares, and the whole of what it
/// is for.
///
/// **The graph does not fit anywhere else.** `f_scene::arena::Arena` is 131 096
/// bytes; a component's stack is four pages and a component may hold no
/// writable static at all — `cargo xtask component` refuses an image with one —
/// so the one place a retained graph can live is a region the frame mapped and
/// described before the first instruction. That is `f_ring::heap`, and this is
/// how much of it this component asks for.
///
/// Thirty-five pages, and the arithmetic is [`crate::HELD_BYTES`] rather than a
/// figure to take on trust: the arena at 131 096 bytes and a sixty-four-delta
/// frame under construction at 5 672 make 136 768, and the allocator's
/// thirty-two-byte prologue and the rounding on two blocks add at most 64 more —
/// 136 832, which is thirty-four pages with 2 432 bytes to spare. The
/// thirty-fifth is the page a wider `Delta` or a longer frame spends before
/// anybody has to touch a manifest, and the const assertion in [`crate`] is what
/// turns overrunning it into a build failure rather than a component whose first
/// allocation comes back null at ring 3 with nothing in the failure naming the
/// cause.
///
/// It is written in three places and they are required to be one number: here,
/// in `user/compositor/manifest.toml`, and in the boot that stands this
/// component up outside a place. `kernel/src/compositor.rs` reads the manifest's
/// figure and refuses a boot where the two disagree, so the manifest cannot
/// drift from this constant without a red boot saying so.
/// Unit: bytes.
pub const HEAP_BYTES: u64 = 143_360;

/// Byte offsets of the fields the frame writes, each a little-endian `u64`.
///
/// Slots rather than a `repr(C)` struct, because the two sides read and write
/// them through `f_ring::device::Window`, which is a bounds-checked volatile
/// accessor and not a reference — there is no struct to borrow. Eight bytes each
/// even where four would do, so that adding a field never moves one.
pub mod at {
    /// [`super::MAGIC`]. Unit: none.
    pub const MAGIC: u32 = 0;
    /// Where the control ring is.
    /// Unit: bytes, in the component's address space.
    pub const CONTROL_AT: u32 = 8;
    /// How many bytes of it. Unit: bytes.
    pub const CONTROL_LEN: u32 = 16;
    /// Where the ring this component serves its client on is.
    /// Unit: bytes, in the component's address space.
    pub const DATA_AT: u32 = 24;
    /// How many bytes of it. Unit: bytes.
    pub const DATA_LEN: u32 = 32;
    /// Where this component's own state tree is.
    ///
    /// **Told rather than assumed, for the reason `user/virtio-blk` gives:** the
    /// tree is mapped by the shapes that publish one and by no other, so a
    /// component holding the address as a constant would fault at ring 3 on
    /// every boot that stood it up some other way. Zero means *this build
    /// published no tree for you*, which is a page not to write rather than a
    /// page at address zero.
    /// Unit: bytes, in the component's address space.
    pub const TREE_AT: u32 = 40;
    // 48 and 56 were a heap address and its length, and they are gone rather
    // than kept: `f_ring::heap::Heap::COMPONENT` names one address — that is
    // what makes it *safe*, since a constructor taking an address would let
    // component code point its allocator at its own stack — and how large the
    // region is, the frame wrote into the region's own prologue. Both slots were
    // therefore written by the frame and read by nobody, which is a field that
    // can be wrong with nothing to notice. The numbers are not reused, for the
    // reason a state node id is never reused: two readings of this page across
    // time should mean the same thing.

    /// The ABI version the frame negotiated on the client's behalf.
    ///
    /// **Read and refused rather than carried.** A component built against a
    /// different ABI than the one its peer negotiated is a component that will
    /// misread an entry rather than fail to read one, which is the failure R04
    /// exists to turn into a refusal.
    /// Unit: none.
    pub const NEGOTIATED_VERSION: u32 = 64;
    /// The feature set beside it, whole.
    ///
    /// Refused when it is not empty, which is what this component's manifest
    /// declares: `features = []` on the one ring it serves. A feature bit
    /// arriving here is a peer that negotiated something this build does not
    /// implement, and accepting it silently is the half of a negotiation that
    /// makes the other half a decoration.
    /// Unit: none — a bitmask of `f_abi::feature` constants.
    pub const NEGOTIATED_FEATURES: u32 = 72;
    /// How many turns of its loop the component will spend with nothing on
    /// either ring before it stops.
    ///
    /// A backstop and not a mechanism: the frame's stop notice is what ends an
    /// ordinary run, and a run that reaches this number is a run where the
    /// *frame* stopped serving. It exists anyway because RFC 0046 says a hang is
    /// a count and a loop with no bound is a hang with an explanation. A count
    /// and not a duration, because RFC 0004 offers a component no clock.
    /// Unit: turns.
    pub const IDLE_SPINS: u32 = 80;

    /// The frame's clock, refreshed by the frame and read by the component.
    ///
    /// **This is the only time this component ever sees, and the arrangement is
    /// RFC 0004 rather than a convenience.** Nothing at ring 3 may observe a
    /// clock: there is no instruction a component is allowed to use for it and
    /// no `f_env::Env` on this side of the boundary, which is why
    /// `crate::tree::Held::offer` leaves `Cqe::timestamp` at zero and says so.
    /// So the frame reads its own environment and writes the reading here, and
    /// `crate::pacing` is arithmetic over what it was told.
    ///
    /// What that buys is the exit's second clause. The frame's environment is
    /// seeded, so the sequence of readings a boot produces is a function of a
    /// seed rather than of how fast the host was — and the wake time this
    /// component computes is therefore the same on a slow machine and a fast
    /// one, which is what makes it printable in a boot log `cargo xtask trace`
    /// hashes.
    ///
    /// Monotonic within one run and never compared against a reading from
    /// anywhere else, which is the caveat `f_env::Instant` already carries.
    /// Unit: nanoseconds, in the frame's epoch.
    pub const TICK_NANOS: u32 = 88;
    /// How far apart the display's scanouts are.
    ///
    /// Told rather than assumed, because a compositor that held a refresh rate
    /// as a constant would be a compositor that is wrong on every machine but
    /// one. Zero is *this build knows of no display*, which
    /// `crate::pacing::scanout_after` answers by aiming at now — a schedule
    /// nobody can mistake for a real one.
    /// Unit: nanoseconds.
    pub const SCANOUT_PERIOD_NANOS: u32 = 96;
    /// What the wake time holds back against the estimate being wrong.
    ///
    /// A policy and not a measurement, which is why it arrives from outside: the
    /// number that is right depends on how bad a missed frame is on this
    /// machine, and that is a question a component cannot answer about itself.
    /// Unit: nanoseconds.
    pub const PACING_MARGIN_NANOS: u32 = 104;
    /// What the backend under this machine reports it can do.
    ///
    /// The bits are positions, one per `f_interface::backend::Capability`, at
    /// that capability's own `index()`. **A bitmask and not the vocabulary's own
    /// representation**, because `Capabilities` is a type in a crate above the
    /// frame and a page is bytes: `crate::tree::reported_capabilities` walks
    /// `Capability::ALL` and sets what it finds, so the two sides agree through
    /// the vocabulary rather than through a layout neither of them states.
    ///
    /// A set satisfying no rung never reaches a running component, because the
    /// frame refuses it a compositor before this page is written —
    /// `ADMISSION/NO_RUNG`, RFC 0080 and `E3-B02b`, in
    /// `kernel/src/compositor.rs`. What the component does with one anyway is
    /// `crate::tree::rung_word`'s subject, and the short answer is that it
    /// publishes *no rung* rather than the floor.
    ///
    /// **Written once, before the component's first instruction, and never
    /// again by an honest frame.** The word is not `const` and nothing here can
    /// make it so, which is why `kernel/src/compositor.rs` deliberately writes a
    /// *better* report into it half way through the serving boot: the component
    /// must publish the rung it started on, and a run in which the page never
    /// changed could not tell that apart from a component that recomputes.
    /// Unit: none — a bitmask of capability indices.
    pub const BACKEND_CAPABILITIES: u32 = 112;
}

/// The state nodes this component's manifest declares, by the id it declares
/// them under.
///
/// **Here rather than in `component.rs` because both sides read them.** The
/// component writes these ids and the frame reads them back out of the tree it
/// mounted, and an id written down twice is an id the two sides can come to
/// disagree about — which is the failure RFC 0013's *read, never delivered*
/// cannot catch, because both halves would still be reading a well-formed tree.
///
/// Ids are permanent and are never reused, for the reason `TODO.md` never
/// reuses a task id: the id is the only thing that makes two readings of this
/// component across time comparable at all. The root carries no word, so it is
/// not here; `user/compositor/manifest.toml` is where the shape is declared and
/// this is the half the code needs.
pub mod node {
    /// Frames that closed. Unit: frames — UI frames.
    pub const FRAMES: u32 = 2;
    /// Deltas that reached the graph. Unit: deltas.
    pub const EDITS: u32 = 3;
    /// Nodes the graph holds now. Unit: nodes.
    pub const NODES: u32 = 4;
    /// Entries refused. Unit: entries.
    pub const REFUSED: u32 = 5;

    // --- the frame's story, `E3-B01k` ---------------------------------------
    //
    // Five ids, and what makes them one group rather than five additions is
    // that a reader wants all five at once: *which renderer, which frame, what
    // it was due by, what a frame costs here, and what was given up to fit*.
    // Any one of them alone is a number nobody can act on — a pacing estimate
    // with no deadline beside it does not say whether the machine is keeping
    // up, and a degradation with no frame token beside it does not say which
    // frame gave something up.

    /// Which rung of RFC 0080's ladder this compositor is holding.
    ///
    /// `f_interface::ladder::Rung::index()` plus one, so that zero is *this
    /// machine satisfies no rung* rather than the top rung — which is the one
    /// confusion this node must not be able to cause, because the top rung is
    /// the best answer and no rung at all is the worst one.
    /// Unit: none — a rung ordinal, not a quantity.
    pub const RUNG: u32 = 6;
    /// The token of the last frame that closed.
    ///
    /// The client's own word handed back. It is published beside `frames`
    /// rather than instead of it: *two frames closed* and *the last one was
    /// called 0x12* are different claims, and a compositor that had closed the
    /// same frame twice would move one and not the other.
    /// Unit: none — a frame identifier, not a quantity.
    pub const FRAME: u32 = 7;
    /// The deadline that frame carried.
    ///
    /// Out of `f_abi::scene::Frame::deadline`, which the wire refuses a commit
    /// without — so this is the client's own statement of when the frame was
    /// due, republished where a reader who never saw the submission can find
    /// it. Unit: nanoseconds, in the channel's epoch.
    pub const DEADLINE: u32 = 8;
    /// The rolling p99 of what a frame has cost this compositor.
    ///
    /// `crate::pacing::Pacing::estimate_nanos`, over the last
    /// `crate::pacing::WINDOW` frames. Published rather than the wake time
    /// itself because the estimate is the part that is about *this machine*: a
    /// wake time is an estimate, a scanout and a margin, and the other two are
    /// things the frame told this component.
    /// Unit: nanoseconds.
    pub const PACING: u32 = 9;
    /// What was given up to make the last frame fit.
    ///
    /// A choice from `f_scene::degrade::Criterion::ORDER`, offset by
    /// `crate::pacing::degraded::FIRST_CRITERION`, with two values below it for
    /// the two answers that are not a criterion. **Not a boolean**, and
    /// `crate::pacing::degraded` argues why at length: `E3-B07b`'s whole
    /// decision is *which* effect goes first, and a node that said only
    /// *something was degraded* would be publishing the existence of a policy
    /// rather than its choice.
    /// Unit: none — a `crate::pacing::degraded` ordinal.
    pub const DEGRADED: u32 = 10;

    /// Every node this component writes a word into, in ascending id order.
    ///
    /// The component publishes exactly these and the frame requires exactly
    /// this many to carry a word, so a node added to the manifest and forgotten
    /// here is a boot that says eight where the schema says nine rather than a
    /// silence.
    /// Unit: none — node ids.
    pub const WRITTEN: [u32; 9] =
        [FRAMES, EDITS, NODES, REFUSED, RUNG, FRAME, DEADLINE, PACING, DEGRADED];
}

/// Where the component's own half of the page starts.
///
/// Half a page in, so that neither side can reach the other's fields by an
/// arithmetic slip of a few bytes: the frame's writes stop long before here and
/// the component's start here. It is not protection — one page is one mapping
/// and the component may write all of it — it is distance, which is what makes a
/// misplaced offset a wrong *answer* rather than a corrupted one.
/// Unit: bytes.
pub const REPORT: u32 = 2048;

/// Byte offsets of the fields the component writes.
pub mod reported {
    /// [`super::MAGIC`] again, written last, so that a frame reading a page the
    /// component never reached finds a zero rather than a plausible tally.
    pub const MAGIC: u32 = super::REPORT;
    /// Entries taken off the data ring, whatever became of them.
    ///
    /// Beside [`STAGED`] rather than derived from it, because they are two
    /// different claims: one is what the loop saw arrive and the other is what
    /// the frame under construction accepted. A build where the batch had
    /// stopped accepting publishes the same `drained` as one that refused
    /// nothing. Unit: entries.
    pub const DRAINED: u32 = super::REPORT + 8;
    /// Deltas staged into a frame that has not closed yet.
    /// Unit: deltas.
    pub const STAGED: u32 = super::REPORT + 16;
    /// Frames committed: `f_scene::commit::Batch::commit` answering `Ok`.
    /// Unit: frames — UI frames, and not memory pages.
    pub const FRAMES: u32 = super::REPORT + 24;
    /// Deltas that reached the graph, summed from what each commit applied.
    /// Unit: deltas.
    pub const EDITS: u32 = super::REPORT + 32;
    /// Nodes that began. Unit: nodes.
    pub const CREATED: u32 = super::REPORT + 40;
    /// Nodes that left, subtrees included. Unit: nodes.
    pub const REMOVED: u32 = super::REPORT + 48;
    /// How many nodes the graph holds now.
    ///
    /// Read out of `f_scene::arena::Arena::live` at the end of the run rather
    /// than summed from the two above, which is what makes it worth publishing:
    /// a build whose removals did not reach the graph publishes the same
    /// `created` and `removed` and a different census.
    /// Unit: nodes.
    pub const LIVE: u32 = super::REPORT + 56;
    /// Entries this component refused. Unit: entries.
    pub const REFUSED: u32 = super::REPORT + 64;
    /// The token of the last frame that closed.
    /// Unit: none — a frame identifier, not a quantity.
    pub const TOKEN: u32 = super::REPORT + 72;
    /// What stopped the component, as one of the [`stopped`](super::stopped)
    /// constants. Unit: none — an ordinal.
    pub const OUTCOME: u32 = super::REPORT + 80;
    /// How many of its declared state nodes this component wrote a word into.
    ///
    /// Published on the board *as well as* written into the tree, and the
    /// duplication is the point: the frame reads the tree itself and compares.
    /// A component that published nothing and claimed nine here is caught by a
    /// reader that does not take its word for it.
    /// Unit: nodes.
    pub const PUBLISHED: u32 = super::REPORT + 88;
    /// The scanout the last closed frame was paced against.
    ///
    /// Published on the board and **not** in the tree, and the division is
    /// deliberate: the tree carries what is about this component — the estimate
    /// it took over its own frames — and the board carries the whole of the
    /// arithmetic, so a reader checking `E3-B01h`'s sentence can do the
    /// subtraction rather than believe it. `crate::pacing::Decision` is the four
    /// of them together.
    /// Unit: nanoseconds.
    pub const SCANOUT: u32 = super::REPORT + 96;
    /// The rolling p99 estimate at the last frame. Unit: nanoseconds.
    pub const ESTIMATE: u32 = super::REPORT + 104;
    /// What was held back against it being wrong. Unit: nanoseconds.
    pub const MARGIN: u32 = super::REPORT + 112;
    /// The wake time: the scanout, less the estimate, less the margin.
    ///
    /// `E3-B01h`'s first clause, as a number a boot can read back. The three
    /// terms above are published beside it for the reason a claim carries its
    /// workload: an answer whose inputs are not published is an answer nobody
    /// can check.
    /// Unit: nanoseconds.
    pub const WAKE: u32 = super::REPORT + 120;
    /// How many frames the estimate was taken over.
    ///
    /// Beside the estimate because a p99 over four frames and a p99 over a
    /// hundred and twenty-eight are different claims wearing one name, and a
    /// build whose window never filled would publish a plausible number with no
    /// evidence behind it.
    /// Unit: samples.
    pub const SAMPLES: u32 = super::REPORT + 128;
    /// What was given up to fit the last frame, as a
    /// `crate::pacing::degraded` ordinal. Unit: none.
    pub const DEGRADED: u32 = super::REPORT + 136;
    /// Which rung this compositor is holding, as `Rung::index()` plus one, or
    /// zero for a machine that satisfies none. Unit: none.
    pub const RUNG: u32 = super::REPORT + 144;
    /// The deadline the last closed frame carried.
    ///
    /// On the board as well as in the tree, for [`PUBLISHED`]'s reason applied
    /// to a word rather than to a count: the frame compares the two, and a
    /// component whose tree and board disagreed about one number would be a
    /// component with two sets of counters.
    /// Unit: nanoseconds, in the channel's epoch.
    pub const DEADLINE: u32 = super::REPORT + 152;
    /// How many frames did not fit before their own deadline.
    ///
    /// **The second observation of the degradation policy, and the reason
    /// [`DEGRADED`] is checkable at all.** The tree carries what happened to the
    /// last frame, which is one reading; a component that answered the same way
    /// every frame and one that decided per frame publish the same word. This is
    /// the count, and `kernel/src/compositor.rs` requires it to be one out of
    /// two rather than two out of two.
    /// Unit: frames — UI frames.
    pub const LATE: u32 = super::REPORT + 160;
}

/// Why the component's loop ended.
///
/// Written into [`reported::OUTCOME`] so that a boot can tell a compositor that
/// served its client and was told to stop from one that fell out of its loop
/// because something it read did not make sense. Both exit; only one of them is
/// the run the boot asked for, and a status word that could not tell them apart
/// would make every refusal in this component read as success.
pub mod stopped {
    /// The frame's stop notice arrived and the loop ended on it.
    pub const TOLD: u64 = 1;
    /// The routing page did not carry [`super::MAGIC`], so nothing after it was
    /// believed.
    pub const NO_ROUTING: u64 = 2;
    /// An address in the routing page could not be stated as a window or a
    /// channel.
    pub const BAD_ROUTING: u64 = 3;
    /// A ring stopped validating under the component, which is a peer that has
    /// stopped speaking.
    pub const NO_RING: u64 = 4;
    /// The heap the frame described could not answer for the graph.
    ///
    /// Its own outcome rather than a panic, because the alternative is the one
    /// failure a component cannot report: an allocation that comes back null is
    /// a `handle_alloc_error`, which in an image compiled with
    /// `panic=immediate-abort` is an instruction that stops the core with
    /// nothing written anywhere. A compositor with no graph ends here instead,
    /// and the boot reads the reason.
    pub const NO_GRAPH: u64 = 5;
    /// The loop found nothing on either ring for [`super::at::IDLE_SPINS`]
    /// turns.
    ///
    /// Its own outcome rather than [`TOLD`], for `user/virtio-gpu`'s reason: a
    /// run that ends here is a run where the frame stopped serving, which is a
    /// different event from a run that was told to stop and must not be reported
    /// as one.
    pub const IDLE: u64 = 6;
}
