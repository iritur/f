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

    /// Whether this build's frame will ring a doorbell for this component.
    ///
    /// One of the [`bell`](super::bell) constants. It is the frame's statement
    /// about *itself*, and the component believes it without checking, because
    /// there is nothing to check it against: whether a doorbell reaches this
    /// core is a fact about a vector, an interrupt controller and another core,
    /// and every one of those is on the far side of the boundary.
    ///
    /// **A component that stopped its core on a frame that will not ring would
    /// hang**, and that is the whole reason this word exists rather than the
    /// component simply always parking. `f_ring::doorbell::Path` is the frame's
    /// own name for the same decision — polling, a kernel interrupt, a user
    /// interrupt — selected at channel creation from what was negotiated *and*
    /// what the hardware reports. This is that decision, narrowed to the one bit
    /// a component can act on: does the frame ring, or must this component look
    /// for itself.
    ///
    /// Zero is [`bell::POLL`](super::bell::POLL), so a frame that never wrote
    /// this word gets the behaviour every boot before `E3-B01g` had. That is the
    /// safe default in the only sense that matters here: a component that spins
    /// when it could have slept wastes a core, and one that sleeps when nobody
    /// will ring never comes back.
    /// Unit: none — a [`bell`](super::bell) ordinal.
    pub const DOORBELL: u32 = 120;

    /// Which node of the client's scene the pointer rides.
    ///
    /// The late latch patches one node's transform between a commit closing and
    /// a submission crossing, and *which node* is a fact about a client's scene
    /// rather than about this component. So it is told, like the pacing inputs
    /// above and for a sharper version of their reason: a compositor that picked
    /// one would be picking inside somebody else's graph.
    ///
    /// Zero is `f_abi::scene::NO_NODE`, so a frame that never wrote this word
    /// gets a component that latches nothing — which is every boot before
    /// `E3-B01i`, unchanged. `crate::latch::Declined::NoNode` is what it
    /// publishes on every frame, rather than a refusal.
    /// Unit: none — a node identifier.
    pub const POINTER_NODE: u32 = 128;

    // 136, 144 and 152 were the pointer — a stamp and two coordinates the frame
    // would write before submitting an entry and the component would read after
    // taking it — and they are gone rather than kept, for `E3-B04g`'s reason.
    // No frame ever wrote them: `kernel/` may not hold a reading, and a frame
    // that wrote a stamp into this page would be exactly that. So they were
    // three words the component read every turn and nobody could honestly fill,
    // and the pointer reaches this component now the way the RFC 0124 steps
    // say it should: off the driver's own ring, at [`INPUT_AT`], decoded here.
    // The numbers are not reused, for the reason 48 and 56 are not.

    /// Where the input driver's data channel is, in this component's address
    /// space, or zero where the frame connected none.
    ///
    /// **The other end of the driver's ring, and this component holds it.** The
    /// driver holds the client's end and submits unasked; until `E3-B04g` the
    /// frame held this end, drained it, and handed on two coordinates in a
    /// scene delta. Now the frame lays the channel out, gives the driver one
    /// end and this component the other, and never takes an entry off it —
    /// which is what makes *the position the latch reads came off a device by a
    /// route the frame never touched* a property of the arrangement rather than
    /// a promise. `crate::inbound` is what is done with an entry once taken.
    ///
    /// Zero is `input=withheld`, the control: the identical boot with the
    /// channel not connected. A component told zero adopts nothing and latches
    /// nothing, and publishes that it was not connected, so *nothing arrived*
    /// and *nothing was connected* are two different words on its board.
    /// Unit: bytes, in the component's address space.
    pub const INPUT_AT: u32 = 160;
    /// How many bytes of it. Unit: bytes.
    pub const INPUT_LEN: u32 = 168;
}

/// What the frame says it will do when this component has nothing to do.
///
/// Two values and deliberately not a boolean, for the reason
/// `f_ring::doorbell::Path` has three: the question *how is this component
/// woken* has more answers than *is it woken*, and a user-level interrupt is
/// the answer this tree is holding a row of technical debt for. A boolean here
/// would have to be widened the day that row is paid, and a word that names its
/// answers does not.
pub mod bell {
    /// Nobody rings. The component looks for itself, forever, and this is what
    /// every boot before `E3-B01g` did.
    pub const POLL: u64 = 0;
    /// The frame rings a kernel inter-processor interrupt, so the component may
    /// stop its core with `f_abi::door::WAIT` and be restarted by one.
    pub const RING: u64 = 1;
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
    /// What was given up, one answer per frame, for the last
    /// `crate::pacing::degraded::FRAMES` frames that closed.
    ///
    /// A choice from `f_scene::degrade::Criterion::ORDER`, offset by
    /// `crate::pacing::degraded::FIRST_CRITERION`, with three values below it for
    /// the answers that are not a criterion and `NONE` below those for a frame
    /// that never happened. **Not a boolean**, and `crate::pacing::degraded`
    /// argues why at length: `E3-B07b`'s whole decision is *which* effect goes
    /// first, and a node that said only *something was degraded* would be
    /// publishing the existence of a policy rather than its choice.
    ///
    /// **And not a snapshot either, which is `E3-B07d` and RFC 0118.** This node
    /// carried the last frame's answer until 2026-09-24, and that word was
    /// byte-identical between a compositor that decided per frame and one that
    /// decided once at start — so the clause *every frame carries the reduction
    /// it chose* could not be read off it at all. It is now a register: sixteen
    /// fields of four bits, newest first, packed by `crate::pacing::Record`. The
    /// alternative was a second node, and this manifest has none to give —
    /// `f_abi::manifest::STATE_NODES_MAX` is sixteen and this component declares
    /// sixteen — so the choice was between widening a wire bound every component
    /// in this system pays for and making the word that already exists carry what
    /// the line needs. RFC 0118 prices both.
    /// Unit: none — sixteen packed `crate::pacing::degraded` ordinals.
    pub const DEGRADED: u32 = 10;

    // --- the resolved theme, `E3-B06d` --------------------------------------
    //
    // Three ids, and what makes them one group is that no one of them can be
    // read alone. *One resolution* with no note count beside it says the work
    // was done once and not what it decided; a note count with no resolution
    // count beside it cannot tell a theme this layer agreed with from a theme
    // resolved so often that nobody could have read the notes; and neither says
    // anything about the one obligation RFC 0079 hands a compositor, which is
    // the third.
    //
    // **They are in the tree and not only on the board**, which is the opposite
    // of where the first draft of this task put them. The board is this
    // component's answer to *what did you do*, read once when its core comes
    // back; the tree is the machine's answer to *what is it running*, and
    // whether the interface on this machine is readable — and whether anybody
    // was told what it cost to make it so — is a question about a running
    // machine rather than about a run that ended. `E3-B01k` settled the same
    // question the same way for the frame's story, and the sentence its exit
    // used is the one this group is written to satisfy: *a boot reads them back
    // out of the component's subtree rather than out of the serial log.*
    //
    // Two more words are on the board and have no node here, and the division
    // is deliberate rather than economy. `reported::DROPPED` and
    // `reported::CLEAN` are about the **report's own integrity** — whether it
    // truncated, and whether the resolver's own judgement agrees with the count
    // — which is evidence about this component's honesty and belongs where the
    // frame checks it, not in the answer a reader of the machine gets.

    /// How many times this component resolved a theme.
    ///
    /// One, for the life of an instance, and the whole reason this is a node is
    /// that *one* is checkable where *not recomputing* is not. RFC 0079
    /// resolves an ink once per ground; a compositor that re-resolved every
    /// frame would answer every colour question with the same colours and
    /// differ only in what it spent, so a reader with no count has no reading
    /// that separates the two builds.
    /// Unit: none — resolutions.
    pub const RESOLVES: u32 = 11;
    /// How many decisions the resolver made that the theme's author did not.
    ///
    /// `f_interface::token::Report::len`. Zero under a theme this layer agrees
    /// with, and one per clamp otherwise. It is the node that makes RFC 0079's
    /// *clamped, not refused* an arrangement somebody can audit rather than a
    /// promise: a compositor that resolved a theme, moved somebody's colours to
    /// clear a floor and published nothing would have made the clamp silent,
    /// which is the outcome that RFC spends four alternatives avoiding.
    /// Unit: none — notes.
    pub const NOTES: u32 = 12;
    /// How many ordered pairs of distinct grounds owe a rule between them.
    ///
    /// RFC 0079 places exactly one obligation on a compositor — *`boundary`
    /// says, for any two grounds, whether they part on their own, and carries
    /// the colour of the rule that must be drawn when they do not* — and this
    /// is the node that says the obligation was read. Six is the maximum: three
    /// grounds, ordered pairs, and a ground over itself is one region rather
    /// than a boundary.
    ///
    /// **Rules owed and never rules drawn.** Nothing in this build draws
    /// anything and `E3-B02` owes the pixels; a component that published a
    /// count of rules it had drawn would be reporting work nobody did, which is
    /// the one outcome RFC 0079 says would not be acceptable.
    /// Unit: none — ordered pairs of grounds.
    pub const RULES: u32 = 13;

    // --- the synchronisation state, `E3-B05f` -------------------------------
    //
    // Three ids, and what makes them one group is that a supervisor deciding
    // what to do about a stuck frame needs all three. *One wait outstanding*
    // with no last-reached value beside it does not say which frame is stuck;
    // a last-reached value with no outstanding count does not say whether
    // anything is waiting on the next one; and neither says whether this has
    // happened before, which is the difference between a pipeline in flight and
    // a component that abandons a frame every time. `crate::waits` is where all
    // three are produced and where the third is argued at length.
    //
    // **These three fill this component's manifest.** `f_abi::manifest::
    // STATE_NODES_MAX` is sixteen and this manifest now declares sixteen: the
    // subtree and fifteen words. That is not a problem and it is worth saying
    // where the next author will look — the next node this component wants is an
    // RFC widening that bound, with the record's fixed width as the cost, and
    // not a row quietly added to `manifest.toml`. `cargo xtask lint-manifests` is
    // what turns the attempt into a red build rather than a discovery.

    /// Waits the last frame entered and did not get out of.
    ///
    /// `f_abi::trace::Trace::unreleased` over the trace of the last frame that
    /// closed. A gauge and the state of **one frame**: a running total would
    /// answer *has this ever happened* where a reader of a live machine is asking
    /// *is it happening now*, and [`TIMEOUTS`] is the total.
    /// Unit: waits.
    pub const WAITS: u32 = 14;
    /// The highest value that has landed on the compositor's own timeline.
    ///
    /// `f_abi::sync::Timeline::signalled`, which moves at the landing and never
    /// at the submission. That is what makes it worth a node rather than being
    /// derived from [`FRAMES`]: a component that had submitted three signals and
    /// reached one publishes three frames and a one here, and a build that moved
    /// the timeline at submission would publish three and be caught by nothing
    /// else.
    ///
    /// The compositor's own and not the application's, because it is the only
    /// timeline this component produces. `crate::waits` argues that asymmetry.
    /// Unit: none — a timeline value, which is an ordinal.
    pub const SIGNALLED: u32 = 15;
    /// Frames that closed with a wait outstanding and no room left to satisfy it.
    ///
    /// **Not a timer**, and `crate::waits` spends a section on why: nothing in
    /// this component observes time, so a timeout is defined out of the deadline
    /// the client put on the wire and the estimate this component already
    /// publishes, and the frame that does not fit is the frame whose promised
    /// value is never reached. A node that could only move if a timer existed
    /// would be a published zero, which is the failure `E3-B01k` was written
    /// against.
    /// Unit: frames — UI frames.
    pub const TIMEOUTS: u32 = 16;

    /// Every node this component writes a word into, in ascending id order.
    ///
    /// The component publishes exactly these and the frame requires exactly
    /// this many to carry a word, so a node added to the manifest and forgotten
    /// here is a boot that says fourteen where the schema says fifteen rather
    /// than a silence.
    /// Unit: none — node ids.
    pub const WRITTEN: [u32; 15] = [
        FRAMES, EDITS, NODES, REFUSED, RUNG, FRAME, DEADLINE, PACING, DEGRADED, RESOLVES, NOTES,
        RULES, WAITS, SIGNALLED, TIMEOUTS,
    ];
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
    /// What was given up, one `crate::pacing::degraded` ordinal per frame, for
    /// the last `crate::pacing::degraded::FRAMES` frames.
    ///
    /// The same register `super::node::DEGRADED` carries, which is where it is
    /// argued. On the board as well as in the tree for `PUBLISHED`'s reason: the
    /// frame requires the two to agree, and a component with two sets of
    /// counters is what that comparison exists to catch.
    /// Unit: none — sixteen packed ordinals.
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

    // --- the doorbell, `E3-B01g` --------------------------------------------
    //
    // **Written as they happen and not at the end**, which every other word in
    // this module is. The frame reads them while the component is still running
    // — it waits for [`PARKED`] to move before it submits the entry whose
    // doorbell is supposed to wake this component, so that *a client's commit
    // woke a parked compositor* is a sentence about a run rather than about a
    // likely interleaving.
    //
    // **Nothing rests on that read.** It is a timing observation and a racy one:
    // the word is written volatilely on one core and read volatilely on another,
    // exactly as `at::TICK_NANOS` already is in the other direction, and a frame
    // that read a stale value would ring early or late and the protocol would
    // absorb it either way. What makes the doorbell correct is the ring's own
    // arm-look-sleep and the frame's wakeup latch, neither of which consults
    // this.

    /// Times this component asked the frame to stop its core. Unit: waits.
    pub const PARKED: u32 = super::REPORT + 168;
    /// How many of those really stopped it, as the frame answered.
    ///
    /// Below [`PARKED`] by however many times a doorbell had already arrived
    /// between this component deciding to sleep and the frame acting on it —
    /// which is the race the frame latches, and a count of it is the only
    /// evidence that the latch is doing anything. Unit: waits.
    pub const HALTED: u32 = super::REPORT + 176;

    // --- the resolved theme, `E3-B06d` --------------------------------------
    //
    // Five words about one call, and three of them have a node in
    // [`super::node`] as well. The division is the one this module's own header
    // draws rather than a saving: three of these five say what a reader of a
    // *running machine* wants — it resolved its theme once, it made this many
    // decisions on its author's behalf, and it owes this many rules — and those
    // three are in the tree, where the frame requires them to equal what is
    // here. The other two are about the **report's own integrity**: whether it
    // truncated, and whether the resolver's own verdict agrees with the count.
    // Those are evidence about this component rather than about the machine, and
    // the board is where the frame checks this component.
    //
    // The first draft of this group put all five here and none in the tree, on
    // the argument that widening `node::WRITTEN` costs a manifest, a schema and
    // the frame's census. That argument is about what a diff costs and not about
    // what a reader needs, which is the wrong question: `E3-B01k` had it and
    // answered it the other way, and its exit sentence — *a boot reads them back
    // out of the component's subtree rather than out of the serial log* — is
    // what this group is written to satisfy.
    //
    // RFC 0013's *read, never delivered* applies to them exactly as it does to
    // every other word above: the frame takes them out of memory it granted and
    // this component is never asked.

    /// How many times this component resolved a theme.
    ///
    /// **One, and the whole of why this word exists is that one is checkable
    /// and *not recomputing* is not.** E3-B06d says a theme becomes values
    /// *once per ground*; a compositor that re-resolved every frame answers
    /// every colour question with the same colours and differs only in what it
    /// spent, so a reader with no count has no reading that separates the two.
    /// A boot that closes two frames and reads a one here has seen the
    /// difference.
    /// Unit: resolutions.
    pub const RESOLVES: u32 = super::REPORT + 184;
    /// How many decisions the resolver made that the theme's author did not.
    ///
    /// `f_interface::token::Report::len`. Zero under a theme this layer agrees
    /// with, and every clamp RFC 0079 performs adds one — which is what makes
    /// *clamped rather than refused* an arrangement somebody can audit instead
    /// of a promise. Unit: notes.
    pub const NOTES: u32 = super::REPORT + 192;
    /// How many notes did not fit in the report and were counted instead.
    ///
    /// `f_interface::token::Report::dropped`, published beside [`NOTES`] rather
    /// than folded into it: a report that truncated silently would be the one
    /// place in that module where something happens and nothing says so, and a
    /// component that published only the length would put the silence back.
    /// Unit: notes.
    pub const DROPPED: u32 = super::REPORT + 200;
    /// One when the theme survived resolution untouched, zero otherwise.
    ///
    /// `f_interface::token::Report::is_clean`, which is *no notes and none
    /// dropped*. It is published as well as the two counts above rather than
    /// derived from them by the reader, because it is the resolver's own
    /// judgement and a frame that recomputed it would be a second opinion about
    /// somebody else's type. Unit: none — a flag.
    pub const CLEAN: u32 = super::REPORT + 208;
    /// How many ordered pairs of distinct grounds owe a rule between them.
    ///
    /// RFC 0079 places exactly one obligation on a compositor — *for any two
    /// grounds, whether they part on their own, and the colour of the rule owed
    /// when they do not* — and names the signal that it has gone unimplemented:
    /// a compositor existing with no caller of `Resolved::boundary` in it. This
    /// word is what that caller produced. Six is the maximum, because there are
    /// three grounds and a ground over itself is not a boundary.
    ///
    /// **A count of rules owed and never of rules drawn.** Nothing in this build
    /// draws anything; `E3-B02` owes the pixels, and RFC 0079 says the one
    /// unacceptable outcome is a compositor that quietly reported having drawn a
    /// rule it did not.
    /// Unit: ordered pairs of grounds.
    pub const RULES: u32 = super::REPORT + 216;

    // --- the synchronisation state, `E3-B05f` -------------------------------
    //
    // The same three words `super::node` carries, on the board where the frame
    // compares them. A component whose tree and board disagreed about one of them
    // would be a component with two sets of counters, which is the comparison
    // `E3-B01k` established and the reason every word in the tree has one here.

    /// Waits the last frame entered and did not get out of.
    /// Unit: waits.
    pub const WAITS: u32 = super::REPORT + 224;
    /// The highest value that has landed on the compositor's own timeline.
    /// Unit: none — a timeline value, which is an ordinal.
    pub const SIGNALLED: u32 = super::REPORT + 232;
    /// Frames that closed with a wait outstanding and no room left to satisfy it.
    /// Unit: frames — UI frames.
    pub const TIMEOUTS: u32 = super::REPORT + 240;

    // --- the boundary crossings, `E3-B01j` ----------------------------------
    //
    // **None of these three has a node, and the reason is a bound rather than a
    // judgement.** `f_abi::manifest::STATE_NODES_MAX` is sixteen and
    // `E3-B05f`'s three words took this component's manifest to exactly that.
    // The honest consequence is that this group lives on the board alone, and
    // the honest reading of that is: a crossing count is evidence about *this
    // run* rather than a property of a running machine a reader would poll, so
    // the board is where it belongs anyway. `E3-B01j`'s exit asks for the number
    // printed at boot and carried into `claims/`, and asks nothing of the tree.
    //
    // *What would reverse this:* the day a reader wants the crossing rate of a
    // machine that is still running — a supervisor deciding that a client has
    // become chatty is the obvious one — the bound has to move first, and that
    // is an RFC with the record's fixed width as its cost.

    /// Completions this component put on the data ring.
    ///
    /// **Counted where the ring accepted it and not where it was produced**,
    /// which is the difference between a crossing and an intention:
    /// `crate::tree::Held::offer` answers a completion, and a completion the ring
    /// refused never crossed anything. So this word is incremented in
    /// `crate::component`, beside the `post`, and the frame requires it to equal
    /// the completions its own client reaped.
    /// Unit: entries.
    pub const ANSWERED: u32 = super::REPORT + 248;
    /// Entries that crossed this boundary in either direction, this run.
    ///
    /// [`DRAINED`] plus [`ANSWERED`], summed by the component rather than by the
    /// frame. That is the point of the word: `E3-B01j`'s exit says *the frame and
    /// the component each count the crossings*, and a figure the frame added up
    /// out of the component's two counts would be one side counting and the other
    /// trusting it — the failure that line exists to prevent. Both are published
    /// so that a reader can check the sum, and the frame checks it.
    ///
    /// **A crossing is an entry, and not an operation and not a doorbell.**
    /// `E3-B01g` settled that a batch of four entries is one publish and one
    /// doorbell, and printed what a client charging per entry would have said —
    /// which makes the *operation* the right unit for a doorbell, because what a
    /// doorbell costs is one delivery. It is the wrong unit for a crossing: a
    /// client that batches four deltas into one publish has still moved four
    /// payloads into the arena and still takes four completions back, and the
    /// number `docs/design/ring-scene-boot.html` wants under ten per frame is the
    /// number of deltas a frame costs — which is the number `E3-B01l`'s
    /// reconciler exists to reduce. The doorbell is counted separately by
    /// `f_ring::doorbell` and printed beside this, so a reader can convert.
    ///
    /// **The data ring only.** The control ring carries the frame's notices about
    /// this component's life, which happen once per run rather than once per
    /// frame, and folding them in would make a per-frame figure depend on how
    /// often the supervisor spoke. *What would reverse this:* a control ring that
    /// carries something per frame.
    /// Unit: entries.
    pub const CROSSINGS: u32 = super::REPORT + 256;
    /// [`CROSSINGS`] per UI frame, times a thousand.
    ///
    /// Times a thousand because RFC 0004 forbids a binary fraction and this is a
    /// ratio: sixteen crossings over two frames is eight thousand here, and a
    /// reader who wants the whole number divides. The scale is in the name, which
    /// is the convention `f_interface::token`'s `contrast_x1000` established and
    /// `claims/0034` publishes a rate under.
    ///
    /// Zero where no frame closed, which is a statement and not a division by
    /// zero: a component that closed no frame has no per-frame anything, and
    /// [`FRAMES`] beside it is what tells the two apart.
    /// Unit: entries per UI frame, times one thousand.
    pub const CROSSINGS_PER_FRAME_X1000: u32 = super::REPORT + 264;

    // --- the frame trace's own integrity, `E3-B05f` -------------------------
    //
    // Four words beside the three that have a node, on the division this module
    // draws everywhere: the three in the tree say what a reader of a running
    // machine wants, and these four are evidence about whether the record those
    // three come out of can be believed at all. They are `DROPPED` and `CLEAN`'s
    // shape one layer over, and for the same reason.

    /// Waits every frame's trace has named, summed over the run.
    ///
    /// Two per frame in this build — the compositor's on the application's
    /// timeline and the present engine's on the compositor's — so the frame can
    /// require exactly twice the frame count. It is what makes `E3-B05b`'s *names
    /// every wait* checkable from outside: a component whose present stage waited
    /// and recorded nothing would publish a plausible [`WAITS`](super::node::WAITS)
    /// and half of this.
    /// Unit: waits.
    pub const TRACED: u32 = super::REPORT + 272;
    /// Waits the traces could not hold, summed over the run.
    ///
    /// `f_abi::trace::Trace::dropped_waits`. Non-zero means the bound's
    /// derivation — three stages, each submitting once — has stopped being true,
    /// and RFC 0101 is why it is a published count rather than an assertion: that
    /// kind of decay is silent and surfaces three subsystems away from the
    /// addition that caused it.
    /// Unit: waits.
    pub const TRACE_DROPPED: u32 = super::REPORT + 280;
    /// One while every trace so far has named every wait its frame entered.
    ///
    /// Held across the run rather than read off the last frame, because a build
    /// that dropped a wait in the first frame and none afterwards would publish a
    /// clean last trace. Published beside [`TRACE_DROPPED`] for [`CLEAN`]'s
    /// reason: a flag and a count that cannot disagree without one of them having
    /// been published unread.
    /// Unit: none — a flag.
    pub const TRACE_COMPLETE: u32 = super::REPORT + 288;
    /// Refusals this component's own chain produced.
    ///
    /// Zero on a build whose arithmetic is right: every value offered to the
    /// chain is this component's own frame ordinal. A non-zero word is the
    /// component contradicting itself, and the frame requires zero — which is why
    /// `crate::waits` counts the refusal instead of ending the run over it. A
    /// compositor that stopped serving because its own bookkeeping disagreed
    /// would have turned an accounting defect into a black screen.
    /// Unit: refusals.
    pub const CHAIN_REFUSALS: u32 = super::REPORT + 296;

    /// Positions the device reported that this component took.
    ///
    /// `crate::latch::LateLatch::reports`. Unit: reports.
    pub const POINTER_REPORTS: u32 = super::REPORT + 304;

    /// Positions the predictor refused as not newer than the newest it held.
    ///
    /// Published beside the count above rather than dropped, because a relay
    /// that duplicated or reordered a report is invisible in every other number
    /// here: the prediction is still made, from a window one report shorter than
    /// the run implies. Unit: reports.
    pub const POINTER_STALE: u32 = super::REPORT + 312;

    /// Frames that carried a latch. Unit: frames.
    pub const LATCHES: u32 = super::REPORT + 320;

    /// Frames that did not, whatever the reason. Unit: frames.
    pub const LATCH_DECLINES: u32 = super::REPORT + 328;

    /// How many wait entries the last latched frame's trace held when the latch
    /// happened.
    ///
    /// **The word that says the latch was in the window.** Zero is after the
    /// commit closed and before the compositor's own submission entered its
    /// wait; a one or a two is a latch that has drifted past a submission, which
    /// is what `E3-B01i`'s *between commit and submit* forbids and is a number
    /// rather than a sequence a reader has to reconstruct.
    /// Unit: wait entries.
    pub const LATCH_ENTRY: u32 = super::REPORT + 336;

    /// The translation along x the client committed on the last latched frame.
    /// Unit: device pixels, scaled by 65 536, as a two's-complement `u64`.
    pub const LATCH_COMMITTED_X: u32 = super::REPORT + 344;

    /// And along y. Unit: as [`LATCH_COMMITTED_X`].
    pub const LATCH_COMMITTED_Y: u32 = super::REPORT + 352;

    /// The translation along x that was submitted on the last latched frame.
    ///
    /// Both ends are published and the difference is left to the reader, for
    /// `crate::latch::Latched`'s reason: a component that published the motion
    /// would be publishing its own subtraction, and the frame checking it would
    /// be checking this component's arithmetic against itself.
    /// Unit: as [`LATCH_COMMITTED_X`].
    pub const LATCH_X: u32 = super::REPORT + 360;

    /// And along y. Unit: as [`LATCH_COMMITTED_X`].
    pub const LATCH_Y: u32 = super::REPORT + 368;

    /// How far forward the prediction on the last latched frame was actually
    /// extrapolated, after the predictor's own damping. Zero when the position
    /// was held. Unit: nanoseconds.
    pub const LATCH_LEAD_NANOS: u32 = super::REPORT + 376;

    /// The instant the last latch was aimed at.
    ///
    /// **Published so that the seam has a question both sides can be asked.**
    /// `E3-B04e` compares two records neither of which derives from the other,
    /// and two predictors asked about two different instants would disagree for
    /// a reason that is not a defect. This is the instant this component chose;
    /// the frame asks its own predictor the same one.
    /// Unit: nanoseconds, in the channel's epoch.
    pub const LATCH_AIM_NANOS: u32 = super::REPORT + 384;

    /// One when the last latched frame's position was an extrapolation, zero
    /// when it was the last position the device actually reported.
    ///
    /// The word that tells `E3-B01i`'s case from `E3-B04e`'s: a held position
    /// differs from the committed one by exactly the motion that arrived, and an
    /// extrapolated one differs by that motion plus the lead the predictor
    /// claimed. Unit: none — a flag.
    pub const LATCH_EXTRAPOLATED: u32 = super::REPORT + 392;

    /// Readings this component refused because they carried no stamp.
    ///
    /// A page the frame has not written is zeroes, and zero is
    /// `f_abi::input::NOT_STAMPED` — *the absence of a stamp, not a stamp of
    /// zero*. This word is how many times that was read and refused, and it is
    /// published rather than swallowed because it is the difference between a
    /// machine whose pointer route is not connected and one whose pointer has
    /// not moved: the two have the same zero everywhere else.
    /// Unit: readings.
    pub const POINTER_UNSTAMPED: u32 = super::REPORT + 400;

    // --- the input ring, `E3-B04g` -------------------------------------------
    //
    // What this component took off the driver's ring, in words the frame reads
    // after the run. None of them has a node: the manifest is full, and these
    // are evidence about *this run* rather than something a reader of a running
    // machine would poll — `E3-B01j`'s argument for its own three, for its
    // reason. **None of them is a crossing in `E3-B01j`'s sense either**:
    // [`CROSSINGS`] is the scene ring's, it is a published claim, and an input
    // entry counted into it would move `claims/0038` as a side effect of a
    // different line.

    /// One where the frame gave this component an input ring and it adopted
    /// it, zero where it gave none. Unit: none — a flag.
    pub const INPUT_CONNECTED: u32 = super::REPORT + 408;
    /// Entries taken off the input ring that were not the driver's
    /// attestation. Unit: entries.
    pub const INPUT_ENTRIES: u32 = super::REPORT + 416;
    /// Of those, the entries `f_abi::input::Event::decode` took. Unit: entries.
    pub const INPUT_DECODED: u32 = super::REPORT + 424;
    /// Of those, the entries it refused. Unit: entries.
    pub const INPUT_REFUSED: u32 = super::REPORT + 432;
    /// Of the decoded ones, pointer motion. Unit: entries.
    pub const INPUT_MOTIONS: u32 = super::REPORT + 440;
    /// What this component drained, folded in the order it arrived.
    ///
    /// **This component's half of the attestation**, and the frame compares it
    /// against the driver's word as the *driver's routing page* carried it —
    /// which is the second route that word travels by. [`INPUT_AGREED`] is this
    /// component's own comparison against the copy that arrived on the ring.
    /// Unit: none — a checksum.
    pub const INPUT_CROSSING: u32 = super::REPORT + 448;
    /// How many entries went into [`INPUT_CROSSING`]. Unit: entries.
    pub const INPUT_CROSSED: u32 = super::REPORT + 456;
    /// Attestations taken off the ring. One, or nothing was checked.
    /// Unit: entries.
    pub const INPUT_ATTESTATIONS: u32 = super::REPORT + 464;
    /// The driver's word, as it arrived on the ring. Zero where none did.
    /// Unit: none — a checksum.
    pub const INPUT_ATTESTED: u32 = super::REPORT + 472;
    /// How many entries the driver says went into it. Unit: entries.
    pub const INPUT_ATTESTED_COUNT: u32 = super::REPORT + 480;
    /// Entries taken after the attestation, which it therefore does not cover.
    /// Unit: entries.
    pub const INPUT_UNATTESTED: u32 = super::REPORT + 488;
    /// One where what this component drained agrees with what the driver
    /// attested, word and count, with exactly one attestation and nothing
    /// after it. `crate::inbound::Inbound::agrees`. Unit: none — a flag.
    pub const INPUT_AGREED: u32 = super::REPORT + 496;

    /// The last latched frame's graph, folded with the pointer's two
    /// translations masked, before the patch.
    ///
    /// `E3-B01i`'s *and by nothing else*, as a boot carries it:
    /// `crate::latch::unmoved` is the fold and the argument for what it covers.
    /// The frame requires [`LATCH_UNMOVED_AFTER`] to be the same word.
    /// Unit: none — a checksum.
    pub const LATCH_UNMOVED_BEFORE: u32 = super::REPORT + 504;
    /// The same fold after the patch. Unit: none — a checksum.
    pub const LATCH_UNMOVED_AFTER: u32 = super::REPORT + 512;
    /// How many nodes the fold after the patch walked, which the frame requires
    /// to be the graph's own count — a fold of an empty walk agrees with itself
    /// about nothing. Unit: nodes.
    pub const LATCH_WALKED: u32 = super::REPORT + 520;
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
