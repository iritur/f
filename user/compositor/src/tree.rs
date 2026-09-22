// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The graph this component holds, and what one arriving entry does to it.
//!
//! # Why this is not in `component.rs`
//!
//! Because everything here runs on a host and nothing in `component.rs` can.
//! That module is a ring-3 entry point behind two `cfg` gates — an
//! architecture, because the door is one instruction, and the image feature,
//! because a `#[panic_handler]` is a lang item — so a test that reached into it
//! would be a test compiled on neither of the two architectures this tree
//! promises. What is here is the half a test can drive: an entry's bytes in, a
//! completion out, and a graph that changed or did not.
//!
//! The split is the one `user/objects` makes between `service` and `component`
//! and it earns its keep the same way: the data path is exercised by
//! `cargo xtask test` on both architectures, and the boot is what shows that the
//! same code answers a real client across a real ring.
//!
//! # What this type is and is not
//!
//! It is the **retained** half. The graph survives between entries, between
//! frames and — the day a doorbell exists — between sleeps; that is the whole
//! of what *retained* means and the whole of why a compositor is a component
//! rather than a library a client links. `f_scene::commit::Batch` is the frame
//! under construction and `f_scene::arena::Arena` is what a closed frame
//! reaches; this type is the two of them side by side with a tally, and it adds
//! no rule of its own to either.
//!
//! It is **not** a renderer and it holds no surface. A frame closes here and
//! nothing is drawn: `E3-B02` is what draws, and a reader looking for the
//! pixels will not find them — `crate`'s own comment says who owes them.
//!
//! It does now hold a rung and a pacing estimate, and both arrived with
//! `E3-B01k` and `E3-B01h`. Neither is acted on. The rung is *reported* and
//! never used to choose a renderer, because choosing once at start and never
//! promoting is `E3-B02b`'s clause and there is no renderer to choose; the
//! pacing estimate is *published* and never slept on, because sleeping is
//! `E3-B01g`'s doorbell. What this file holds is the arithmetic and the
//! bookkeeping, which is the half a host test can drive on both architectures.

use f_abi::scene::{PAYLOAD_BYTES, Refusal as WireRefusal};
use f_abi::{Cqe, Sqe, error, flags};
use f_interface::backend::{Capabilities, Capability, select};
use f_scene::arena::{Arena, Refusal as GraphRefusal};
use f_scene::commit::{Batch, DELTAS_MAX, Offered, Refusal, Refused};

use crate::pacing::{Decision, Pacing, Tick, degraded, degraded_word};

/// How many deltas one frame may carry before this component refuses it.
///
/// `f_scene::commit::DELTAS_MAX`, which is section 07's number and not this
/// component's: [`Batch`] is generic over the bound precisely so that a
/// component states its own, and a compositor that states something *smaller*
/// than the figure the design is drawn for would be refusing frames the design
/// says are ordinary. What that costs is [`crate::routing::HEAP_BYTES`], which
/// is where the batch lives with the graph — so the bound is paid for in a
/// manifest rather than in a stack this component does not have.
/// Unit: deltas.
pub const FRAME_DELTAS_MAX: usize = DELTAS_MAX;

/// What this component counted while it served.
///
/// Every field is a number some other reader can check. `drained` is what the
/// loop saw arrive and the client knows how many it sent; `frames`, `edits`,
/// `created` and `removed` are summed from what `f_scene::commit::Batch::commit`
/// answered, and the client knows what it asked for; `token` is the client's own
/// word handed back. None of them is this component's opinion about its own
/// success, which is the property `kernel/src/compositor.rs` rests its verdict
/// on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Entries taken off the data ring, whatever became of them.
    /// Unit: entries.
    pub drained: u64,
    /// Deltas staged into the frame under construction, which is a count that
    /// goes back to zero at every commit rather than a total.
    /// Unit: deltas.
    pub staged: u64,
    /// Frames that closed. Unit: frames — UI frames, and not memory pages.
    pub frames: u64,
    /// Deltas that reached the graph. Unit: deltas.
    pub edits: u64,
    /// Nodes that began. Unit: nodes.
    pub created: u64,
    /// Nodes that left, subtrees included. Unit: nodes.
    pub removed: u64,
    /// Entries refused, by the wire or by the graph. Unit: entries.
    pub refused: u64,
    /// The token of the last frame that closed.
    /// Unit: none — a frame identifier, not a quantity.
    pub token: u64,
    /// Frames whose pacing estimate did not fit before their own deadline.
    ///
    /// **A count and not a flag, and it is the number that makes
    /// [`Story::degraded`] checkable.** The tree carries what happened to the
    /// *last* frame, which is one observation; a reader cannot tell a compositor
    /// that decided per frame from one that answers the same way every time out
    /// of a single reading. This is the second observation, and
    /// `kernel/src/compositor.rs` requires it to be one out of two rather than
    /// two out of two.
    /// Unit: frames — UI frames.
    pub late: u64,
}

impl Counters {
    /// A component that has done nothing.
    ///
    /// A `const` rather than `Default::default()` so that [`Held::new`] adds
    /// nothing to the two it is handed: what a compositor starts as is an empty
    /// graph, an empty frame and nine zeroes, and the zeroes are the only part
    /// this file gets to decide.
    pub const ZERO: Self = Self {
        drained: 0,
        staged: 0,
        frames: 0,
        edits: 0,
        created: 0,
        removed: 0,
        refused: 0,
        token: 0,
        late: 0,
    };
}

/// What the frame told this component before its first instruction.
///
/// Three numbers off the routing page, carried together because they are one
/// decision — *what this machine can draw with, how often it scans out, and how
/// much of a frame to keep in hand* — and because a constructor taking three
/// bare integers of two different units is a constructor whose arguments get
/// swapped. `crate::routing::at` is where each one is written and argued.
///
/// Told rather than computed, for the reason `crate::routing`'s own comment
/// gives about every other field on that page: a component that derived its own
/// refresh rate would be a component that is wrong on every machine but the one
/// its author had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// What the backend reports, as a bitmask of
    /// `f_interface::backend::Capability` indices.
    /// Unit: none — a bitmask.
    pub backend_bits: u64,
    /// How far apart this display's scanouts are. Unit: nanoseconds.
    pub scanout_period_nanos: u64,
    /// What the wake time holds back against the estimate being wrong.
    /// Unit: nanoseconds.
    pub margin_nanos: u64,
}

/// The bitmask the frame wrote, read back through the vocabulary.
///
/// **Through `Capability::ALL` and each capability's own `index()`, rather than
/// by casting the word.** `f_interface::backend::Capabilities` is a type in a
/// crate that deliberately depends on nothing and is not on any wire — its own
/// comment says so — so there is no layout for the frame and the component to
/// agree about, and a `transmute` would be this component inventing one. What
/// the two sides share instead is the vocabulary: a capability's index is a
/// position in a list both of them read, and a bit at a position nothing names
/// is a bit this function drops rather than a capability it invents.
///
/// Unit of `bits`: none — a bitmask of capability indices.
#[must_use]
pub fn reported_capabilities(bits: u64) -> Capabilities {
    let mut reported = Capabilities::NONE;
    for capability in Capability::ALL {
        if bits & (1 << capability.index()) != 0 {
            reported = reported.with(capability);
        }
    }
    reported
}

/// Which rung this machine gets, as the word `crate::routing::node::RUNG`
/// carries.
///
/// `Rung::index()` plus one, so that zero is *this machine satisfies no rung*
/// and is not confusable with the top one. RFC 0080 says a backend satisfying no
/// rung is **refused a compositor** rather than handed the floor; this build
/// reports rather than refuses, because refusing is `E3-B02b`'s exit and a
/// component that refused to serve here would be closing that task's clause
/// without its evidence.
/// Unit: none — a rung ordinal, not a quantity.
#[must_use]
pub fn rung_word(bits: u64) -> u64 {
    match select(reported_capabilities(bits)) {
        Ok(rung) => rung.index() as u64 + 1,
        Err(_) => 0,
    }
}

/// What this component publishes about the last frame it closed.
///
/// `E3-B01k`'s five words, minus the one [`Counters`] already keeps: the frame
/// token is a tally's business because it moves with `frames`, and the other
/// four are here. Every field is a value this component computed or was told,
/// and none of them is an opinion it holds about itself — which is the same
/// property [`Counters`] has and the reason `kernel/src/compositor.rs` can check
/// all of them against what its own script asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Story {
    /// Which rung of RFC 0080's ladder, as [`rung_word`] spells it.
    /// Unit: none — a rung ordinal.
    pub rung: u64,
    /// The deadline the last closed frame carried, out of
    /// `f_abi::scene::Frame::deadline`.
    /// Unit: nanoseconds, in the channel's epoch.
    pub deadline_nanos: u64,
    /// What was given up to fit it, as a `crate::pacing::degraded` ordinal.
    /// Unit: none.
    pub degraded: u64,
    /// The scanout it was paced against, the estimate, the margin and the wake
    /// time the three of them make.
    pub decision: Decision,
}

/// The graph, the frame under construction, and the tally.
///
/// # Why it borrows rather than owns
///
/// The arena is 131 096 bytes and a batch of sixty-four deltas is 5 672 more. A
/// component's stack is four pages and a component may hold no writable static
/// at all, so both live in the heap the frame mapped and described before the
/// first instruction — and this type is the two of them together with a tally,
/// borrowed rather than owned so that **nothing in this file allocates and
/// nothing in it needs `alloc` to compile**. `component.rs` owns the two boxes
/// and this is what holds them side by side.
///
/// **Two allocations and not one, and the reason is an image rather than a
/// preference.** `Arena::EMPTY` is all zeroes, so a box of one is an allocation
/// and a `memset`; `Batch::new()` is not — its filler is a removal of `NO_NODE`,
/// whose opcode tag is not zero — so a box of one is an allocation and a copy
/// out of a constant the linker puts in the image. Held in one value the two
/// made a component file of 152 728 bytes against the 65 536 the frame maps for
/// it; split, 147 280, and the rest of that number was the arena's own constant,
/// which is what `f_scene::kind`'s discriminants were renumbered to remove.
/// `crate`'s own comment is the whole of that story.
pub struct Held<'a> {
    /// The retained scene.
    graph: &'a mut Arena,
    /// The frame being assembled. A delta cannot reach `graph` except through a
    /// commit that closes this, which is `f_scene::commit`'s decision and not
    /// this file's.
    batch: &'a mut Batch<FRAME_DELTAS_MAX>,
    /// What has happened so far.
    counters: Counters,
    /// What the last [`crate::pacing::WINDOW`] frames cost.
    ///
    /// Owned rather than borrowed, unlike the two above, and the asymmetry is
    /// RFC 0100's: a kibibyte of zeroes is a `memset` and costs nothing in the
    /// image, while the arena and the batch are a hundred and thirty kilobytes
    /// that have to be on the heap whatever they are initialised to. A window
    /// that had to be boxed would be a third allocation for no reason.
    pacing: Pacing,
    /// What the frame told this component.
    plan: Plan,
    /// What it publishes about the last frame it closed.
    story: Story,
    /// When the first delta of the frame under construction was staged.
    ///
    /// `None` between frames, which is what makes a frame's cost the span of
    /// *that frame* rather than the span since the last commit: a compositor
    /// with nothing to do sits idle, and charging that idleness to the next
    /// frame would make the estimate a measure of how often a client submits.
    opened: Option<Tick>,
}

impl<'a> Held<'a> {
    /// A compositor holding an empty scene and no frame.
    ///
    /// The two it is given are the caller's to place. `component.rs` puts them
    /// in the heap because that is the only place they fit; a host test puts
    /// them on its own stack, which is what makes this file testable without a
    /// frame under it.
    #[must_use]
    pub fn new(graph: &'a mut Arena, batch: &'a mut Batch<FRAME_DELTAS_MAX>, plan: Plan) -> Self {
        Self {
            graph,
            batch,
            counters: Counters::ZERO,
            pacing: Pacing::ZERO,
            plan,
            // The rung is decided **once**, here, and nothing below moves it.
            // RFC 0080's whole argument is that a compositor picks a rasteriser
            // when it starts and never promotes, and the cheapest way to keep
            // that promise is for the only assignment to be in the constructor.
            story: Story { rung: rung_word(plan.backend_bits), ..Story::default() },
            opened: None,
        }
    }

    /// What it publishes about the last frame it closed.
    #[must_use]
    pub const fn story(&self) -> &Story {
        &self.story
    }

    /// How many frames the pacing estimate is taken over. Unit: samples.
    #[must_use]
    pub const fn samples(&self) -> u64 {
        self.pacing.held() as u64
    }

    /// What this component has counted.
    #[must_use]
    pub const fn counters(&self) -> &Counters {
        &self.counters
    }

    /// How many nodes the graph holds now.
    ///
    /// Asked of the arena rather than summed from `created` and `removed`,
    /// which is what makes it worth publishing: a build whose removals never
    /// reached the graph would publish the same two totals and a different
    /// census. Unit: nodes.
    #[must_use]
    pub fn live(&self) -> u64 {
        self.graph.live() as u64
    }

    /// The graph, for a reader that wants to walk it.
    ///
    /// Shared and never exclusive. Nothing outside this module may edit the
    /// scene, because *a delta is the only way in* is `f_scene`'s property and
    /// an `&mut Arena` handed out here would be a second door into it.
    #[must_use]
    pub const fn graph(&self) -> &Arena {
        self.graph
    }

    /// Offer one entry from the data ring, and answer the completion it earns.
    ///
    /// `None` for an entry that asked not to be told it worked —
    /// [`flags::NO_CQE`] — and succeeded. A refusal always completes, whatever
    /// the flag says, which is that flag's own rule: *I do not need to be told
    /// this worked* is not *do not tell me this was malformed*.
    ///
    /// # What it does not do
    ///
    /// It does not stamp [`Cqe::timestamp`], and the zero there is a statement
    /// rather than an omission: RFC 0004 gives a component no clock, so a
    /// compositor that filled that field would be inventing one. The deadline a
    /// commit carries is read by the wire, refused when absent, and scheduled
    /// against by nothing in this build — `E3-B01h` is the task that computes a
    /// wake time from it, and until then a frame closes when it arrives.
    pub fn offer(&mut self, entry: &Sqe, payload: &[u8; PAYLOAD_BYTES], now: Tick) -> Option<Cqe> {
        self.counters.drained += 1;
        let answered = match self.batch.offer(entry, payload) {
            Ok(Offered::Staged) => {
                // The instant the frame opened, taken at the first delta that
                // reached it rather than at the commit that closed it. A frame's
                // cost is the span of the frame; `Held::opened`'s own comment
                // says what charging the gap between frames would measure
                // instead.
                if self.opened.is_none() {
                    self.opened = Some(now);
                }
                self.counters.staged += 1;
                Ok(())
            }
            Ok(Offered::Sealed(sealed)) => {
                // The one place a delta reaches the graph, and it is a whole
                // frame or none of it. `Batch::commit` resets on both paths, so
                // the staged count goes back to zero either way — a refused
                // frame does not linger into the next one, and neither does its
                // tally.
                self.counters.staged = 0;
                match self.batch.commit(self.graph, sealed) {
                    Ok(closed) => {
                        self.counters.frames += 1;
                        self.counters.edits += closed.edits as u64;
                        self.counters.created += closed.created as u64;
                        self.counters.removed += closed.removed as u64;
                        self.counters.token = closed.frame.token;
                        self.close(now, closed.frame.deadline);
                        Ok(())
                    }
                    Err(why) => {
                        // A frame that did not reach the graph is not a frame
                        // for pacing purposes either: it cost this component
                        // something, but what it cost is the cost of a refusal
                        // rather than of a frame, and a window that held both
                        // would be estimating the wrong quantity. The open
                        // instant is dropped for the same reason `Batch::commit`
                        // resets on both paths.
                        self.opened = None;
                        Err(refused(&why))
                    }
                }
            }
            Err(refusal) => Err(packed(&refusal)),
        };

        match answered {
            Ok(()) => {
                if entry.flags & flags::NO_CQE != 0 {
                    return None;
                }
                Some(Cqe { user_data: entry.user_data, result: 0, ..Cqe::ZERO })
            }
            Err((code, node)) => {
                self.counters.refused += 1;
                // The node the graph refused, in `ext`, which is the field RFC
                // 0010 leaves to the domain. `f_scene::arena::Refusal` answers
                // `NO_NODE` for the refusals that are about the arena rather
                // than about a node, so a zero here is *this refusal names no
                // node* and not *node zero* — the wire spells the same absence
                // the same way.
                Some(Cqe {
                    user_data: entry.user_data,
                    result: code,
                    ext: u64::from(node),
                    ..Cqe::ZERO
                })
            }
        }
    }

    /// Record what the frame that just closed cost, and decide about the next
    /// one.
    ///
    /// # Why the whole of `E3-B01h` is three lines
    ///
    /// Because `crate::pacing` is where the arithmetic lives and this is the one
    /// place it is fed. The cost is the span between the first delta of the
    /// frame and the commit that closed it, taken from the frame's own clock —
    /// the only clock this component ever sees, and `crate::routing::at::
    /// TICK_NANOS` says why it arrives rather than being read.
    ///
    /// The overrun the degradation policy is asked about is measured against
    /// **this frame's own deadline** rather than against the scanout the wake
    /// time is computed from, and the two are different questions on purpose: a
    /// deadline is what the client asked for and a scanout is what the display
    /// will do, so a frame can be inside one and outside the other. The word
    /// published names what happened to the client's request.
    /// Unit of `deadline_nanos`: nanoseconds, in the channel's epoch.
    fn close(&mut self, now: Tick, deadline_nanos: u64) {
        let cost_nanos = now.since(self.opened.unwrap_or(now));
        self.opened = None;
        self.pacing.observe(cost_nanos);
        self.story.deadline_nanos = deadline_nanos;
        self.story.decision =
            self.pacing.decide(now, self.plan.scanout_period_nanos, self.plan.margin_nanos);
        // How long the client left between the commit landing and its own
        // deadline. Saturating: a deadline already passed is *no time at all*
        // rather than a negative span, and a compositor handed one is a
        // compositor that is already late.
        let remaining_nanos = deadline_nanos.saturating_sub(now.nanos());
        self.story.degraded = degraded_word(self.story.decision.estimate_nanos, remaining_nanos);
        if self.story.degraded != degraded::FITTED {
            self.counters.late += 1;
        }
    }
}

/// What a client is told when its frame was not applied.
///
/// Both variants are `ARGUMENT` and the detail is what separates them, which is
/// RFC 0010's arrangement: the domain is what a caller acts on and the code is
/// what it reads. A refused frame leaves the scene exactly as it was, so there
/// is nothing for the client to undo and the ordinary answer is *submit a frame
/// the graph will have*.
///
/// `Refused::Diverged` is this build disagreeing with itself rather than a bad
/// frame, and it is reported as `MALFORMED_HEADER` in the `ARGUMENT` domain
/// rather than being given a code of its own. That is a debt and not a decision:
/// a client cannot act differently on it, and the reader who can is looking at
/// `crate::routing::reported::REFUSED` beside a frame count that did not move.
/// *What would reverse this:* a divergence a client could respond to, at which
/// point it earns a domain rather than a code.
fn refused(why: &Refused) -> (i32, u32) {
    match why {
        Refused::Whole { refusal, .. } => packed(refusal),
        Refused::Diverged { refusal, .. } => {
            (error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER), refusal.node())
        }
    }
}

/// A batch refusal as a packed error and the node it names.
///
/// The wire's refusals keep their own codes — `f_abi::scene::Refusal::packed` is
/// that mapping and this does not restate it — and the ones that belong to the
/// batch or to the graph rather than to one entry's bytes are packed here.
///
/// Two of the choices are worth stating. [`Refusal::Full`] and a graph out of
/// room are `RESOURCE` and not `ARGUMENT`: the entry is well formed and what ran
/// out is room, so a client told `ARGUMENT` would go looking for a field it got
/// wrong, and these are the two refusals `f_scene` exists to be able to make.
/// The rest are `ARGUMENT`, because a frame whose sections arrived out of order,
/// a second commit in one frame, a stale seal and an edit naming a node the
/// scene does not hold are all a peer that has lost track of what it sent.
fn packed(refusal: &Refusal) -> (i32, u32) {
    let argument = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG);
    let full = error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED);
    match refusal {
        Refusal::Wire(wire) => (wire.packed(), f_abi::scene::NO_NODE),
        Refusal::Full => (full, f_abi::scene::NO_NODE),
        Refusal::OutOfPhase | Refusal::AlreadySealed | Refusal::StaleSeal => {
            (argument, f_abi::scene::NO_NODE)
        }
        // The first refusal this batch suffered, carried to every later offer.
        // It is `RESERVED` rather than a code of its own because what the client
        // has to know is that the frame is gone and why is in the completion for
        // the entry that lost it — which it already has.
        Refusal::Poisoned(_) => (WireRefusal::Reserved.packed(), f_abi::scene::NO_NODE),
        Refusal::Graph(graph) => match graph {
            GraphRefusal::Capacity => (full, graph.node()),
            GraphRefusal::Wire(wire) => (wire.packed(), graph.node()),
            GraphRefusal::NoSuchNode(_)
            | GraphRefusal::NoSuchParent(_)
            | GraphRefusal::NotASibling(_)
            | GraphRefusal::Cycle(_)
            | GraphRefusal::KindChanged(_)
            | GraphRefusal::Mismatched => (argument, graph.node()),
        },
    }
}

#[cfg(test)]
mod tests {
    use f_abi::scene::{Commit, CreateNode, Delta, Entry, NO_NODE, RemoveNode, SetPaint, kind};
    use f_interface::ladder::Rung;

    use super::*;

    /// What the frame tells the component in these tests.
    ///
    /// The capability set is the one RFC 0080's table gives the top rung, and it
    /// is built from the vocabulary rather than written as a literal bitmask:
    /// what `reported_capabilities` has to invert is a set of *indices*, so a
    /// test that hard-coded the bits would be asserting against the same
    /// arithmetic it is checking.
    fn plan() -> Plan {
        let mut bits = 0;
        for capability in Capability::ALL {
            bits |= 1 << capability.index();
        }
        Plan { backend_bits: bits, scanout_period_nanos: PERIOD_NANOS, margin_nanos: MARGIN_NANOS }
    }

    /// A sixty-hertz frame. Unit: nanoseconds.
    const PERIOD_NANOS: u64 = 16_666_667;

    /// What the wake time holds back. Unit: nanoseconds.
    const MARGIN_NANOS: u64 = 1_000_000;

    /// How far apart the ticks these tests hand the component are.
    ///
    /// A hundred nanoseconds per entry, so a frame of five deltas and a commit
    /// costs five hundred — a number small enough to fit inside a sixty-hertz
    /// deadline and large enough that a build which measured the span from the
    /// *previous* commit instead would produce a different one.
    /// Unit: nanoseconds.
    const STEP_NANOS: u64 = 100;

    /// One delta, encoded the way a client encodes one.
    fn wire(body: Entry, deadline: u64) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        Delta { user_data: 1, class: 0, deadline, payload_offset: 0, flags: 0, body }.encode()
    }

    /// The four-node scene this module's tests build, and the boot builds the
    /// same one — `kernel/src/compositor.rs` writes it out of the same records
    /// rather than out of a copy of these lines.
    fn scene() -> [(Sqe, [u8; PAYLOAD_BYTES]); 7] {
        [
            wire(
                Entry::CreateNode(CreateNode {
                    node: 1,
                    parent: NO_NODE,
                    before: NO_NODE,
                    kind: kind::LAYER,
                }),
                0,
            ),
            wire(
                Entry::CreateNode(CreateNode {
                    node: 2,
                    parent: 1,
                    before: NO_NODE,
                    kind: kind::TRANSFORM,
                }),
                0,
            ),
            wire(
                Entry::CreateNode(CreateNode {
                    node: 3,
                    parent: 2,
                    before: NO_NODE,
                    kind: kind::DRAW,
                }),
                0,
            ),
            wire(
                Entry::CreateNode(CreateNode {
                    node: 4,
                    parent: 1,
                    before: NO_NODE,
                    kind: kind::SEMANTIC,
                }),
                0,
            ),
            wire(
                Entry::SetPaint(SetPaint {
                    node: 3,
                    red_x65535: 0x4000,
                    green_x65535: 0x8000,
                    blue_x65535: 0xC000,
                    alpha_x65535: 0xFFFF,
                    stroke_width_x65536: 0x0002_0000,
                }),
                0,
            ),
            wire(Entry::Commit(Commit { frame_token: 0x11 }), 900),
            wire(Entry::Commit(Commit { frame_token: 0x12 }), 901),
        ]
    }

    #[test]
    fn a_frame_reaches_the_graph_only_when_its_commit_arrives() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        let entries = scene();
        for (entry, payload) in &entries[..5] {
            assert!(held.offer(entry, payload, clock.next()).is_some_and(|cqe| cqe.result == 0));
            // The graph is still empty, entry after entry. This is the clause a
            // compositor that applied deltas as they arrived would fail, and it
            // is asserted inside the loop rather than after it so that the
            // failure names which entry reached the scene.
            assert_eq!(held.live(), 0, "a delta reached the graph before its frame closed");
        }
        assert_eq!(held.counters().staged, 5);
        let (commit, payload) = &entries[5];
        assert!(held.offer(commit, payload, clock.next()).is_some_and(|cqe| cqe.result == 0));
        assert_eq!(held.live(), 4);
        assert_eq!(held.counters().frames, 1);
        assert_eq!(held.counters().edits, 5);
        assert_eq!(held.counters().created, 4);
        assert_eq!(held.counters().staged, 0);
        assert_eq!(held.counters().token, 0x11);
    }

    #[test]
    fn a_removal_takes_the_subtree_under_it_and_the_census_says_so() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        let entries = scene();
        for (entry, payload) in &entries[..6] {
            held.offer(entry, payload, clock.next());
        }
        assert_eq!(held.live(), 4);

        // Node 2 has node 3 under it, so this is two nodes and not one — the
        // difference between `removed` and *entries that said remove*, and the
        // reason the census is published beside both.
        let (remove, payload) = wire(Entry::RemoveNode(RemoveNode { node: 2 }), 0);
        assert!(held.offer(&remove, &payload, clock.next()).is_some_and(|cqe| cqe.result == 0));
        let (commit, payload) = &entries[6];
        assert!(held.offer(commit, payload, clock.next()).is_some_and(|cqe| cqe.result == 0));
        assert_eq!(held.live(), 2);
        assert_eq!(held.counters().removed, 2);
        assert_eq!(held.counters().frames, 2);
        assert_eq!(held.counters().token, 0x12);
        assert_eq!(held.counters().refused, 0);
    }

    #[test]
    fn a_refused_entry_is_completed_and_poisons_the_frame_it_was_offered_to() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        // A create naming no node: the refusal a zeroed payload earns, which is
        // what a slot whose payload had not landed looks like.
        let (entry, payload) = wire(
            Entry::CreateNode(CreateNode {
                node: NO_NODE,
                parent: NO_NODE,
                before: NO_NODE,
                kind: kind::LAYER,
            }),
            0,
        );
        let answer =
            held.offer(&entry, &payload, clock.next()).expect("a refusal always completes");
        assert!(answer.result < 0, "a refused entry is completed with a refusal");
        assert_eq!(held.counters().refused, 1);

        // And the frame it was offered to can never close, which is the half
        // that matters: an entry lost out of a frame is not a frame with a hole
        // in it.
        let entries = scene();
        let (commit, payload) = &entries[5];
        let answer = held.offer(commit, payload, clock.next()).expect("a refusal always completes");
        assert!(answer.result < 0);
        assert_eq!(held.counters().frames, 0);
        assert_eq!(held.live(), 0);
    }

    /// A clock that ticks once per entry offered.
    ///
    /// This is what the *frame* is, from this component's side: something that
    /// writes a reading into the routing page before each entry it submits. The
    /// tests hold one rather than calling a clock, for the reason
    /// `crate::pacing`'s comment gives — a component has no clock, so a test
    /// that reached for one would be testing something this component cannot do.
    struct Ticking(u64);

    impl Ticking {
        const fn new() -> Self {
            Self(0)
        }

        /// The next reading. Unit: nanoseconds.
        fn next(&mut self) -> Tick {
            self.0 += STEP_NANOS;
            Tick(self.0)
        }
    }

    #[test]
    fn the_rung_is_the_one_the_reported_capabilities_select_and_is_decided_once() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        // Everything reported is the top rung, and the word is its index plus
        // one — so a build that published the index itself says 0, which is the
        // value reserved for a machine that satisfies no rung at all.
        assert_eq!(held.story().rung, Rung::ALL[0].index() as u64 + 1);
        assert_eq!(held.story().rung, 1);

        // And it does not move while frames close. RFC 0080: chosen at start,
        // never promoted, and never demoted by a frame either.
        let entries = scene();
        for (entry, payload) in &entries[..6] {
            held.offer(entry, payload, clock.next());
        }
        assert_eq!(held.counters().frames, 1);
        assert_eq!(held.story().rung, 1, "a closed frame moved the rung");
    }

    #[test]
    fn a_machine_that_satisfies_no_rung_is_reported_as_no_rung_rather_than_as_the_floor() {
        // Nothing reported. The floor of RFC 0080's ladder still asks for
        // something, so an empty set selects no rung — and the word for that is
        // zero, which is below every rung rather than equal to the last one.
        assert_eq!(rung_word(0), 0);
        // And a bit at a position no capability names is dropped rather than
        // turned into a capability: the high half of the word cannot invent one.
        assert_eq!(rung_word(1 << 63), 0);
    }

    #[test]
    fn a_frame_costs_the_span_of_its_own_deltas_and_not_the_gap_before_it() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        // Six entries open and close the first frame, so the first tick is at
        // 100 and the commit is at 600: five steps, five hundred nanoseconds.
        let entries = scene();
        for (entry, payload) in &entries[..6] {
            held.offer(entry, payload, clock.next());
        }
        assert_eq!(held.samples(), 1);
        assert_eq!(
            held.story().decision.estimate_nanos,
            5 * STEP_NANOS,
            "the first frame's cost is not the span from its first delta to its commit",
        );

        // Then a long idle gap, and a second frame of two entries. Its cost is
        // two hundred nanoseconds — the gap belongs to nobody. A build that
        // measured from the previous commit would charge the gap to this frame
        // and the estimate would move.
        clock.0 += 9_000_000;
        let (remove, payload) = wire(Entry::RemoveNode(RemoveNode { node: 2 }), 0);
        held.offer(&remove, &payload, clock.next());
        let (commit, payload) = &entries[6];
        held.offer(commit, payload, clock.next());
        assert_eq!(held.counters().frames, 2);
        assert_eq!(held.samples(), 2);
        // Two samples: five hundred and one hundred. The p99 of two by nearest
        // rank is the larger, so the estimate is still the first frame's.
        assert_eq!(held.story().decision.estimate_nanos, 5 * STEP_NANOS);
    }

    #[test]
    fn the_wake_time_is_the_scanout_less_the_estimate_less_the_margin() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        let entries = scene();
        for (entry, payload) in &entries[..6] {
            held.offer(entry, payload, clock.next());
        }
        let decision = held.story().decision;
        assert_eq!(decision.scanout_nanos, PERIOD_NANOS, "the first scanout after six hundred ns");
        assert_eq!(decision.margin_nanos, MARGIN_NANOS);
        assert_eq!(
            decision.wake_nanos,
            PERIOD_NANOS - 5 * STEP_NANOS - MARGIN_NANOS,
            "the wake time is not the scanout less the estimate less the margin",
        );
    }

    #[test]
    fn a_frame_inside_its_deadline_degrades_nothing_and_one_outside_it_is_counted_late() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        // `scene()`'s commits carry deadlines of 900 and 901 nanoseconds. The
        // first frame closes at 600 with an estimate of 500, so 300 nanoseconds
        // remain and it does not fit — this is the late one.
        let entries = scene();
        for (entry, payload) in &entries[..6] {
            held.offer(entry, payload, clock.next());
        }
        assert_eq!(held.story().deadline_nanos, 900);
        assert_eq!(held.story().degraded, degraded::SHORT);
        assert_eq!(held.counters().late, 1);

        // The second frame is given a deadline a whole scanout away, and the
        // estimate has not moved — so it fits, the word goes back to `FITTED`,
        // and the late count stays at one. A component that answered the same
        // way every frame fails the second half of this and a component that
        // never answered at all fails the first.
        let (remove, payload) = wire(Entry::RemoveNode(RemoveNode { node: 2 }), 0);
        held.offer(&remove, &payload, clock.next());
        let (commit, payload) = wire(Entry::Commit(Commit { frame_token: 0x13 }), 50_000_000);
        held.offer(&commit, &payload, clock.next());
        assert_eq!(held.counters().frames, 2);
        assert_eq!(held.story().deadline_nanos, 50_000_000);
        assert_eq!(held.story().degraded, degraded::FITTED);
        assert_eq!(held.counters().late, 1, "a frame inside its deadline was counted late");
    }

    #[test]
    fn a_frame_that_never_closed_leaves_no_sample_behind() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        // A create naming no node poisons the frame it was offered to, so the
        // commit after it is refused and the graph never changes. A refused
        // frame is not a frame, so the window must still be empty: an estimate
        // that counted refusals would be estimating the cost of being wrong.
        let (bad, payload) = wire(
            Entry::CreateNode(CreateNode {
                node: NO_NODE,
                parent: NO_NODE,
                before: NO_NODE,
                kind: kind::LAYER,
            }),
            0,
        );
        held.offer(&bad, &payload, clock.next());
        let entries = scene();
        let (commit, payload) = &entries[5];
        held.offer(commit, payload, clock.next());
        assert_eq!(held.counters().frames, 0);
        assert_eq!(held.samples(), 0, "a refused frame left a sample in the pacing window");
        assert_eq!(held.story().deadline_nanos, 0);
        assert_eq!(held.counters().late, 0);
    }

    #[test]
    fn an_entry_that_asked_not_to_be_told_it_worked_is_not_told() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch, plan());
        let mut clock = Ticking::new();
        let (mut entry, payload) = wire(
            Entry::CreateNode(CreateNode {
                node: 1,
                parent: NO_NODE,
                before: NO_NODE,
                kind: kind::LAYER,
            }),
            0,
        );
        entry.flags = flags::NO_CQE;
        assert!(held.offer(&entry, &payload, clock.next()).is_none());

        // The same flag on an entry that is refused still completes, which is
        // the asymmetry `f_abi::flags::NO_CQE` states and the reason this test
        // has two halves.
        let (mut bad, payload) = wire(Entry::RemoveNode(RemoveNode { node: NO_NODE }), 0);
        bad.flags = flags::NO_CQE;
        assert!(held.offer(&bad, &payload, clock.next()).is_some());
    }
}
