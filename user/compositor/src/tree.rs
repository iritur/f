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
//! It is **not** a renderer, and it holds no surface, no rung and no pacing
//! estimate. A frame closes here and nothing is drawn: `E3-B02` is what draws
//! and `E3-B01h` is what decides when. A reader looking for the pixels will not
//! find them, and `crate`'s own comment says who owes them.

use f_abi::scene::{PAYLOAD_BYTES, Refusal as WireRefusal};
use f_abi::{Cqe, Sqe, error, flags};
use f_scene::arena::{Arena, Refusal as GraphRefusal};
use f_scene::commit::{Batch, DELTAS_MAX, Offered, Refusal, Refused};

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
}

impl Counters {
    /// A component that has done nothing.
    ///
    /// A `const` rather than `Default::default()` so that [`Held::new`] adds
    /// nothing to the two it is handed: what a compositor starts as is an empty
    /// graph, an empty frame and eight zeroes, and the zeroes are the only part
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
    };
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
}

impl<'a> Held<'a> {
    /// A compositor holding an empty scene and no frame.
    ///
    /// The two it is given are the caller's to place. `component.rs` puts them
    /// in the heap because that is the only place they fit; a host test puts
    /// them on its own stack, which is what makes this file testable without a
    /// frame under it.
    #[must_use]
    pub fn new(graph: &'a mut Arena, batch: &'a mut Batch<FRAME_DELTAS_MAX>) -> Self {
        Self { graph, batch, counters: Counters::ZERO }
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
    pub fn offer(&mut self, entry: &Sqe, payload: &[u8; PAYLOAD_BYTES]) -> Option<Cqe> {
        self.counters.drained += 1;
        let answered = match self.batch.offer(entry, payload) {
            Ok(Offered::Staged) => {
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
                        Ok(())
                    }
                    Err(why) => Err(refused(&why)),
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

    use super::*;

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
        let mut held = Held::new(&mut graph, &mut batch);
        let entries = scene();
        for (entry, payload) in &entries[..5] {
            assert!(held.offer(entry, payload).is_some_and(|cqe| cqe.result == 0));
            // The graph is still empty, entry after entry. This is the clause a
            // compositor that applied deltas as they arrived would fail, and it
            // is asserted inside the loop rather than after it so that the
            // failure names which entry reached the scene.
            assert_eq!(held.live(), 0, "a delta reached the graph before its frame closed");
        }
        assert_eq!(held.counters().staged, 5);
        let (commit, payload) = &entries[5];
        assert!(held.offer(commit, payload).is_some_and(|cqe| cqe.result == 0));
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
        let mut held = Held::new(&mut graph, &mut batch);
        let entries = scene();
        for (entry, payload) in &entries[..6] {
            held.offer(entry, payload);
        }
        assert_eq!(held.live(), 4);

        // Node 2 has node 3 under it, so this is two nodes and not one — the
        // difference between `removed` and *entries that said remove*, and the
        // reason the census is published beside both.
        let (remove, payload) = wire(Entry::RemoveNode(RemoveNode { node: 2 }), 0);
        assert!(held.offer(&remove, &payload).is_some_and(|cqe| cqe.result == 0));
        let (commit, payload) = &entries[6];
        assert!(held.offer(commit, payload).is_some_and(|cqe| cqe.result == 0));
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
        let mut held = Held::new(&mut graph, &mut batch);
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
        let answer = held.offer(&entry, &payload).expect("a refusal always completes");
        assert!(answer.result < 0, "a refused entry is completed with a refusal");
        assert_eq!(held.counters().refused, 1);

        // And the frame it was offered to can never close, which is the half
        // that matters: an entry lost out of a frame is not a frame with a hole
        // in it.
        let entries = scene();
        let (commit, payload) = &entries[5];
        let answer = held.offer(commit, payload).expect("a refusal always completes");
        assert!(answer.result < 0);
        assert_eq!(held.counters().frames, 0);
        assert_eq!(held.live(), 0);
    }

    #[test]
    fn an_entry_that_asked_not_to_be_told_it_worked_is_not_told() {
        let mut graph = Arena::EMPTY;
        let mut batch = Batch::new();
        let mut held = Held::new(&mut graph, &mut batch);
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
        assert!(held.offer(&entry, &payload).is_none());

        // The same flag on an entry that is refused still completes, which is
        // the asymmetry `f_abi::flags::NO_CQE` states and the reason this test
        // has two halves.
        let (mut bad, payload) = wire(Entry::RemoveNode(RemoveNode { node: NO_NODE }), 0);
        bad.flags = flags::NO_CQE;
        assert!(held.offer(&bad, &payload).is_some());
    }
}
