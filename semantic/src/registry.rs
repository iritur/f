// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The trees the system holds, the addresses that reach them, and the handles
//! that may write them.
//!
//! # The two words, and the whole of the task
//!
//! [`TreeId`] addresses. [`Handle`] authorises. They are separate types, they
//! are minted together and they stop being true at different times: [`Registry::close`]
//! moves the slot's generation on, so every handle taken before it is refused
//! from that instant, and the tree it named is read by [`Registry::read`] for as
//! long as the system keeps it.
//!
//! [`Registry::read`] takes a [`TreeId`] and **no** handle. [`Registry::apply`]
//! takes a [`Handle`] and never a [`TreeId`]. Nothing here converts one into the
//! other, and that absence is the mechanism rather than an omission — see the
//! crate comment for the reversal condition.
//!
//! # Why a slot may be adopted, and what that is for
//!
//! `restart.policy = "on_fault"` is a line a manifest may carry, and RFC 0008
//! says what happens to a component whose peer has gone. A component that dies
//! and is restarted is a *second occupant* — `f_abi::door::Entry`'s own comment
//! says it finds its capabilities at the same indices and a later generation —
//! and under section 11's inversion it should find its **tree** there too,
//! because the tree was never its to lose.
//!
//! So [`Registry::adopt`] hands the successor a handle at the new generation
//! over the tree the predecessor left. It is what makes this a system service
//! rather than a graveyard, and it is also what makes the generation check load
//! bearing: after an adopt the slot is open and writable, and the only thing
//! standing between the dead component's handle and the tree is the generation
//! it was minted at.
//!
//! # Why the registry is not `PerCpu`
//!
//! It is not kernel state. The frame *holds* one, in a page it allocated, and
//! threads its address the way `kernel/src/state.rs` threads the state tree's —
//! `CLAUDE.md`'s rule is about mutable statics under `kernel/`, and this type is
//! a value with no static anywhere. Saying so here saves the next reader the
//! trip to `lint-percpu`.

use f_abi::cap::Handle;
use f_abi::semantic::Sealed;

use crate::tree::{Applied, Rejected, Staged, Tree};

/// The address of one tree the system holds.
///
/// A slot index and nothing else — no generation. That is the difference from
/// [`Handle`] written into the type: an address does not expire, because the
/// system does not give the memory back. `kernel/src/state.rs` makes the same
/// argument about the frame's own tree at greater length — *a tree that could be
/// freed would be a mapping a reader still holds* — and this is that argument
/// applied to a tree an application declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TreeId(u16);

impl TreeId {
    /// Which slot of the registry.
    /// Unit: none — an index, and stable for the life of the machine.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.0
    }
}

/// A tree that has just been opened: where it is, and the right to write it.
///
/// The two are handed back together **once**, and that is the only moment they
/// travel as a pair. A caller that keeps the [`TreeId`] and gives the [`Handle`]
/// to an occupant has done exactly what the inversion describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    /// Where the tree is, for as long as the system keeps it.
    pub id: TreeId,
    /// The right to write it, for as long as its occupant lives.
    pub handle: Handle,
}

/// What a slot is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// No tree here.
    Vacant,
    /// A tree with a living writer.
    Open,
    /// A tree whose writer has gone: readable, addressable, and writable by
    /// nobody until somebody adopts it.
    ///
    /// **Not spelled `Sealed`**, although that is the word the rest of this
    /// system uses for a thing that has closed. `f_abi::semantic::Sealed` is the
    /// key [`Registry::apply`] demands, this crate holds both, and two `Sealed`s
    /// meaning different things one module apart is how a reader comes to
    /// believe the key is a state. *Kept* is also the truer word: the system
    /// kept it.
    Kept,
}

/// One tree, and who may write it.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// The generation a handle over this slot must carry.
    ///
    /// Zero while the slot is vacant, which is what makes
    /// [`Handle::NULL`] — and any zeroed field that has become a handle by
    /// accident — name nothing. RFC 0002's rule, and this type keeps it rather
    /// than restating it.
    /// Unit: none — a generation ordinal.
    generation: u16,
    /// What the slot is doing.
    state: State,
    /// The tree.
    tree: Tree,
}

impl Slot {
    /// A slot holding nothing.
    const VACANT: Self = Self { generation: 0, state: State::Vacant, tree: Tree::EMPTY };
}

/// The trees the system holds.
///
/// Generic over the count so that the frame states its own — `kernel/src/semantic.rs`
/// sizes it against the page it puts the registry in, and a host test states two.
/// A constant here would be the frame's number in a crate the frame links, which
/// is the shape `f_scene::commit::Batch` was made generic to avoid.
#[derive(Clone, Copy, Debug)]
pub struct Registry<const SLOTS: usize> {
    slots: [Slot; SLOTS],
}

impl<const SLOTS: usize> Registry<SLOTS> {
    /// A registry holding no trees.
    pub const EMPTY: Self = Self { slots: [Slot::VACANT; SLOTS] };

    /// Open a tree for an occupant that is about to be started.
    ///
    /// `None` when every slot is taken, and a slot that has been sealed is
    /// **not** taken: the system keeps what a dead component declared, so
    /// running out of slots is what happens to a machine that has started more
    /// components than the registry has room for. A frame that wants the memory
    /// back has to say so, and nothing here says it.
    pub fn open(&mut self) -> Option<Opened> {
        let (index, slot) =
            self.slots.iter_mut().enumerate().find(|(_, slot)| slot.state == State::Vacant)?;
        slot.generation = Handle::FIRST_GENERATION;
        slot.state = State::Open;
        slot.tree = Tree::EMPTY;
        let index = u16::try_from(index).ok()?;
        Some(Opened { id: TreeId(index), handle: Handle::new(index, slot.generation) })
    }

    /// The occupant that held this tree is gone.
    ///
    /// **The one call, and it does both halves.** The generation moves on, so
    /// every handle minted over this slot is stale from here; the tree stays
    /// exactly as its writer left it, addressable by the same [`TreeId`]. There
    /// is no way to do one without the other, which is deliberate: a `seal` that
    /// left the generation alone would be a log, and a `revoke` that dropped the
    /// tree would be an ordinary handle.
    ///
    /// Answers whether there was a living writer to lose. `false` for a slot
    /// that is vacant or already sealed, so that a frame reaping the same
    /// occupant twice — which `kernel/src/process.rs` does not do, and which a
    /// future supervisor might — is a `false` rather than a second generation.
    pub fn close(&mut self, id: TreeId) -> bool {
        let Some(slot) = self.slots.get_mut(id.0 as usize) else { return false };
        if slot.state != State::Open {
            return false;
        }
        slot.state = State::Kept;
        // Saturating rather than wrapping, because a generation that wraps is a
        // stale handle that becomes valid again — RFC 0002's argument, and the
        // reason `Handle::RETIRED_GENERATION` exists. A slot that reaches it is
        // refused by `adopt` below rather than reissued.
        slot.generation = slot.generation.saturating_add(1);
        true
    }

    /// Hand a successor the tree its predecessor left.
    ///
    /// The tree is untouched — that is the point — and the handle is at the
    /// generation [`Registry::close`] moved to, so the predecessor's handle is
    /// refused while the successor's is not. `None` for a slot that is not
    /// sealed, and for one whose generation has reached
    /// [`Handle::RETIRED_GENERATION`]: a slot that has held that many writers is
    /// retired rather than wrapped, which turns a soundness hole into running
    /// out of slots.
    pub fn adopt(&mut self, id: TreeId) -> Option<Handle> {
        let slot = self.slots.get_mut(id.0 as usize)?;
        // `==` and not `>=`, because `RETIRED_GENERATION` is `u16::MAX` and
        // `close` saturates rather than wrapping, so the two are the same
        // predicate and clippy refuses the one that looks defensive.
        if slot.state != State::Kept || slot.generation == Handle::RETIRED_GENERATION {
            return None;
        }
        slot.state = State::Open;
        Some(Handle::new(id.0, slot.generation))
    }

    /// The tree at that address, whether or not anything may still write it.
    ///
    /// **Half of the exit, and it takes no handle.** A reader holding a
    /// [`TreeId`] reads a tree whose writer died in a previous boot minute; it
    /// cannot write one, because nothing here turns this argument into the other
    /// one.
    #[must_use]
    pub fn read(&self, id: TreeId) -> Option<&Tree> {
        let slot = self.slots.get(id.0 as usize)?;
        matches!(slot.state, State::Open | State::Kept).then_some(&slot.tree)
    }

    /// Is there still a living writer for the tree at that address?
    ///
    /// For a report rather than for a decision: nothing in this crate consults
    /// it, because a caller that asked this and then applied would be checking a
    /// fact that can change between the two calls. [`Registry::apply`] asks its
    /// own question of the handle it was given.
    #[must_use]
    pub fn writable(&self, id: TreeId) -> bool {
        self.slots.get(id.0 as usize).is_some_and(|slot| slot.state == State::Open)
    }

    /// The generation a handle over that slot must now carry, or zero.
    ///
    /// Published so that a boot report can say *the handle was minted at one and
    /// the slot is at two* rather than only *refused*. Unit: none — a generation
    /// ordinal.
    #[must_use]
    pub fn generation(&self, id: TreeId) -> u16 {
        self.slots.get(id.0 as usize).map_or(0, |slot| slot.generation)
    }

    /// Apply one closed frame to the tree the handle names.
    ///
    /// **The apply path RFC 0083 names as this task's**: it demands the
    /// [`Sealed`] and it demands the [`Handle`], and a caller holding a
    /// [`TreeId`] has neither.
    ///
    /// # Errors
    ///
    /// [`Rejected::Stale`] for a handle whose generation this slot has moved
    /// past — which is every handle a dead occupant held — and
    /// [`Rejected::Vacant`] for one that names no tree at all. Otherwise
    /// whatever [`Tree::apply`] refuses, with the tree left as it was.
    pub fn apply(
        &mut self,
        handle: Handle,
        staged: &Staged,
        key: Sealed,
    ) -> Result<Applied, Rejected> {
        let slot = self.slots.get_mut(handle.index() as usize).ok_or(Rejected::Vacant)?;
        match slot.state {
            // Two refusals and not one, because they are different facts about
            // the world: *there was never a tree here* is a caller with a handle
            // it invented, and *the writer this handle belonged to is gone* is
            // the ordinary end of every component. A caller told one when the
            // other was true would go looking in the wrong place.
            State::Vacant => return Err(Rejected::Vacant),
            State::Kept => return Err(Rejected::Stale),
            State::Open => {}
        }
        // Asked after the state and separately from it, because a test has to be
        // able to reach each one: a sealed slot is refused by the arm above
        // whatever generation the handle carries, and an *adopted* slot is open
        // and is refused by this line alone. One condition doing both would pass
        // both tests while holding one rule.
        if handle.generation() != slot.generation {
            return Err(Rejected::Stale);
        }
        slot.tree.apply(staged, key)
    }
}

impl<const SLOTS: usize> Default for Registry<SLOTS> {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use f_abi::Sqe;
    use f_abi::semantic::StateBits;
    use f_abi::semantic::{
        Commit, DeclareNode, Delta, Entry, Handshake, NO_INTENT, NO_NODE, PAYLOAD_BYTES, Received,
        Session, SetState,
    };
    use f_interface::node::{Role, StateSet};

    use super::*;
    use crate::tree::NODES_MAX;
    use crate::vocabulary::Interface;

    /// The channel epoch every test below opens on. One, because zero is a
    /// legal epoch and a test that used it would not notice a `Session` that
    /// had stopped carrying one.
    const EPOCH: u32 = 1;

    /// A writer that encodes entries and a receiver that decodes them, over one
    /// agreement — which is the smallest thing that can produce a [`Sealed`],
    /// because there is no other way to get one.
    struct Peer {
        session: Session,
        staged: Staged,
    }

    impl Peer {
        fn new() -> Self {
            Self { session: Session::opening(EPOCH), staged: Staged::EMPTY }
        }

        /// Offer one entry, staging it. Panics on a refusal, because every entry
        /// these tests write is one this build accepts and a refusal would be a
        /// fixture that had stopped testing what it says.
        fn offer(&mut self, body: Entry) {
            let (entry, payload) = encode(body);
            match self.session.accept(&entry, &payload, &Interface) {
                Ok(Received::Staged(delta)) => {
                    self.staged.offer(delta).expect("the frame is within the bound")
                }
                Ok(Received::Frame { .. }) => panic!("that entry closed a frame"),
                Err(fault) => panic!("the fixture wrote an entry this build refuses: {fault:?}"),
            }
        }

        /// Close the frame and hand back the key, which is the only way one
        /// exists.
        fn commit(&mut self, token: u64) -> f_abi::semantic::Sealed {
            let (entry, payload) = encode(Entry::Commit(Commit { frame_token: token }));
            match self.session.accept(&entry, &payload, &Interface) {
                Ok(Received::Frame { key, .. }) => key,
                other => panic!("a commit did not close the frame: {other:?}"),
            }
        }
    }

    /// One entry, as it crosses.
    fn encode(body: Entry) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        Delta { user_data: 1, class: 0, payload_offset: 0, flags: 0, body }.encode()
    }

    /// A node declaration, with the role named rather than numbered.
    fn declare(node: u64, parent: u64, role: Role) -> Entry {
        Entry::DeclareNode(DeclareNode {
            node,
            parent,
            before: NO_NODE,
            // The one place a test turns a role into an ordinal, and it goes
            // through the admission rather than around it — which is what stops
            // this fixture from being the second decoder RFC 0083 warns about.
            role: agreement()
                .admit_role(role.index() as u16, &Interface)
                .expect("the vocabulary names its own roles"),
            intent: NO_INTENT,
        })
    }

    /// The agreement an identical peer reaches.
    fn agreement() -> f_abi::semantic::Agreed {
        Handshake::HERE.negotiate().expect("this build agrees with itself")
    }

    /// The state bits a test asks for, admitted rather than invented.
    fn state(set: StateSet) -> StateBits {
        agreement()
            .admit_state(set.bits(), &Interface)
            .expect("the vocabulary names its own states")
    }

    /// The three-node tree every test below declares: a surface, a group under
    /// it, and a button under that.
    fn declared(peer: &mut Peer) {
        peer.offer(Entry::DeclareVocabulary(Handshake::HERE));
        peer.offer(declare(7, NO_NODE, Role::Surface));
        peer.offer(declare(8, 7, Role::Group));
        peer.offer(declare(9, 8, Role::Command));
        peer.offer(Entry::SetState(SetState { node: 9, state: state(StateSet::ENABLED) }));
    }

    #[test]
    fn a_tree_outlives_its_writer_and_the_handle_does_not() {
        // **The exit, and both halves are here because either one alone is the
        // ordinary behaviour of something else.** A tree that survives its
        // writer is what a log does; a handle that dies with its process is what
        // every handle does. What this test shows is the two over one structure:
        // the nodes are still addressable by the identifiers their author chose,
        // and the author's right to touch them is gone.
        let mut registry = Registry::<2>::EMPTY;
        let Opened { id, handle } = registry.open().expect("a fresh registry has a slot");

        let mut peer = Peer::new();
        declared(&mut peer);
        let key = peer.commit(0x11);
        let applied = registry.apply(handle, &peer.staged, key).expect("the frame is well formed");
        peer.staged.clear();
        assert_eq!(applied.declared, 3);
        assert_eq!(applied.stated, 1);
        assert_eq!(applied.agreements, 1);

        let before = *registry.read(id).expect("the tree was just written");
        assert_eq!(before.live(), 3);
        assert_eq!(before.node(9).expect("node nine was declared").parent(), 8);

        // --- the component dies ------------------------------------------
        assert!(registry.close(id), "there was a living writer to lose");

        // Half one: readable and addressable. Addressable, because the nodes
        // answer to the identifiers their author chose rather than to positions
        // in an array a dump would have printed.
        let after = registry.read(id).expect("the system kept the tree");
        assert_eq!(after.live(), 3);
        for (node, role) in [(7, Role::Surface), (8, Role::Group), (9, Role::Command)] {
            let held = after.node(node).expect("the node its author declared");
            assert_eq!(
                Role::from_index(held.role().get() as usize),
                Some(role),
                "the tree forgot what node {node} is",
            );
        }
        assert_eq!(
            after.node(9).and_then(|node| node.state()).map(StateBits::get),
            Some(StateSet::ENABLED.bits()),
            "the tree forgot how the dead writer left its button",
        );

        // Half two: the handle did not survive. The same value that applied a
        // frame a moment ago, offered a frame that is itself beyond reproach.
        let mut ghost = Peer::new();
        ghost.offer(Entry::DeclareVocabulary(Handshake::HERE));
        ghost.offer(declare(10, 7, Role::Label));
        let key = ghost.commit(0x12);
        assert_eq!(registry.apply(handle, &ghost.staged, key), Err(Rejected::Stale));
        // Against the snapshot taken *before* the death, so that this is a
        // comparison with a value and not a comparison of the registry with
        // itself — which is the one way the assertion could pass while the tree
        // changed under it. `refused` is exempt and is checked on its own: a
        // frame the registry turned away never reached the tree, so the tree's
        // own refusal counter did not move either.
        assert_eq!(registry.read(id), Some(&before));
        assert!(registry.read(id).expect("still there").node(10).is_none());
    }

    #[test]
    fn a_successor_inherits_the_tree_and_the_dead_handle_still_does_not_reach_it() {
        // The clause that makes the generation the load-bearing half. After an
        // adopt the slot is **open** and writable, so the only thing between the
        // dead component's handle and the tree is the generation it was minted
        // at — which is the ordinary arrangement `f_abi::door::Entry` describes
        // for a second occupant of a place, applied to a tree rather than to a
        // capability table.
        let mut registry = Registry::<2>::EMPTY;
        let Opened { id, handle } = registry.open().expect("a fresh registry has a slot");
        let mut peer = Peer::new();
        declared(&mut peer);
        let key = peer.commit(0x11);
        registry.apply(handle, &peer.staged, key).expect("the frame is well formed");

        assert!(registry.close(id));
        let heir = registry.adopt(id).expect("a sealed slot may be adopted");
        assert_ne!(heir.generation(), handle.generation());
        assert_eq!(heir.index(), handle.index());
        assert!(registry.writable(id), "the successor may write");

        // The predecessor's handle, against an open slot.
        let mut ghost = Peer::new();
        ghost.offer(Entry::DeclareVocabulary(Handshake::HERE));
        ghost.offer(declare(10, 7, Role::Label));
        let key = ghost.commit(0x12);
        assert_eq!(registry.apply(handle, &ghost.staged, key), Err(Rejected::Stale));

        // And the successor's, over the tree it inherited rather than a fresh
        // one: node ten sits under node seven, which the dead component
        // declared.
        let mut heir_peer = Peer::new();
        heir_peer.offer(Entry::DeclareVocabulary(Handshake::HERE));
        heir_peer.offer(declare(10, 7, Role::Label));
        let key = heir_peer.commit(0x13);
        let applied =
            registry.apply(heir, &heir_peer.staged, key).expect("the successor may write");
        assert_eq!(applied.declared, 1);
        let tree = registry.read(id).expect("the tree is still there");
        assert_eq!(tree.live(), 4);
        assert_eq!(tree.node(10).expect("the successor's node").parent(), 7);
    }

    #[test]
    fn a_handle_naming_a_slot_that_was_never_opened_reaches_nothing() {
        let mut registry = Registry::<2>::EMPTY;
        let mut peer = Peer::new();
        peer.offer(Entry::DeclareVocabulary(Handshake::HERE));
        peer.offer(declare(7, NO_NODE, Role::Surface));
        let key = peer.commit(0x11);
        // A slot inside the registry and a slot past it. Both answer `Vacant`
        // and that is the right answer to both: an index the registry does not
        // have and a slot nothing was ever opened at are the same fact — there
        // is no tree here — and neither is a writer that has gone.
        assert_eq!(
            registry.apply(Handle::new(0, Handle::FIRST_GENERATION), &peer.staged, key),
            Err(Rejected::Vacant),
        );
        let key = peer.commit(0x12);
        assert_eq!(
            registry.apply(Handle::new(9, Handle::FIRST_GENERATION), &peer.staged, key),
            Err(Rejected::Vacant),
        );
        assert!(registry.read(TreeId(0)).is_none());
    }

    #[test]
    fn a_refused_frame_leaves_the_tree_exactly_as_it_was() {
        // Whole or not at all, at the registry's own boundary. The frame
        // declares a node the tree will take and then one it will not, so a
        // receiver that applied as it went would leave the first behind.
        let mut registry = Registry::<1>::EMPTY;
        let Opened { id, handle } = registry.open().expect("a fresh registry has a slot");
        let mut peer = Peer::new();
        declared(&mut peer);
        let key = peer.commit(0x11);
        registry.apply(handle, &peer.staged, key).expect("the frame is well formed");
        peer.staged.clear();
        let before = *registry.read(id).expect("the tree was just written");

        peer.offer(declare(11, 7, Role::Label));
        peer.offer(declare(12, 999, Role::Label));
        let key = peer.commit(0x12);
        assert_eq!(registry.apply(handle, &peer.staged, key), Err(Rejected::NoSuchParent(999)));
        let after = registry.read(id).expect("the tree is still there");
        assert_eq!(after.live(), before.live());
        assert!(after.node(11).is_none(), "an entry before the refused one reached the tree");
        assert_eq!(after.frames(), before.frames());
        assert_eq!(after.refused(), before.refused() + 1);
    }

    #[test]
    fn a_tree_fills_up_and_says_so_without_losing_what_it_held() {
        let mut registry = Registry::<1>::EMPTY;
        let Opened { id, handle } = registry.open().expect("a fresh registry has a slot");
        let mut peer = Peer::new();
        peer.offer(Entry::DeclareVocabulary(Handshake::HERE));
        peer.offer(declare(1, NO_NODE, Role::Surface));
        let key = peer.commit(0x11);
        registry.apply(handle, &peer.staged, key).expect("one node fits");
        peer.staged.clear();

        for node in 2..=(NODES_MAX as u64 + 1) {
            peer.offer(declare(node, 1, Role::Label));
        }
        let key = peer.commit(0x12);
        assert_eq!(registry.apply(handle, &peer.staged, key), Err(Rejected::Capacity));
        assert_eq!(registry.read(id).expect("still there").live(), 1);
    }

    #[test]
    fn a_removal_takes_the_subtree_under_it() {
        let mut registry = Registry::<1>::EMPTY;
        let Opened { id, handle } = registry.open().expect("a fresh registry has a slot");
        let mut peer = Peer::new();
        declared(&mut peer);
        let key = peer.commit(0x11);
        registry.apply(handle, &peer.staged, key).expect("the frame is well formed");
        peer.staged.clear();

        // Node eight has node nine under it, so this is two and not one.
        peer.offer(Entry::Remove(f_abi::semantic::Remove { node: 8 }));
        let key = peer.commit(0x12);
        let applied = registry.apply(handle, &peer.staged, key).expect("the removal is legal");
        assert_eq!(applied.removed, 2);
        let tree = registry.read(id).expect("still there");
        assert_eq!(tree.live(), 1);
        assert!(tree.node(7).is_some(), "the surface was not under what was removed");
        assert!(tree.node(9).is_none(), "an orphan stayed behind");
    }

    #[test]
    fn an_entry_this_build_stores_nothing_of_is_refused_rather_than_dropped() {
        // `SetContent` and `SetRelations`. The crate comment argues the refusal;
        // this is the half that says it is a refusal rather than a silence, and
        // that the frame carrying it does not reach the tree.
        let mut registry = Registry::<1>::EMPTY;
        let Opened { id, handle } = registry.open().expect("a fresh registry has a slot");
        let mut peer = Peer::new();
        declared(&mut peer);
        let key = peer.commit(0x11);
        registry.apply(handle, &peer.staged, key).expect("the frame is well formed");
        peer.staged.clear();

        peer.offer(Entry::SetContent(f_abi::semantic::SetContent {
            node: 9,
            kind: agreement()
                .admit_content(1, &Interface)
                .expect("the vocabulary names its own content kinds"),
            body_offset: 0,
            body_len: 0,
        }));
        let key = peer.commit(0x12);
        assert_eq!(registry.apply(handle, &peer.staged, key), Err(Rejected::NotStored(9)));
        assert_eq!(registry.read(id).expect("still there").frames(), 1);
    }
}
