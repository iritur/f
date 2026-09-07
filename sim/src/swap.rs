// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A component replaced under sustained load: two phases, one routing word, and
//! a client that is never told it happened.
//!
//! # The sentence this module exists to make checkable
//!
//! `E2-P08`: *replace a running component under sustained load; no client
//! observes a dropped operation, and the state transfer is verified rather than
//! assumed.* Two clauses, and neither of them is checkable by watching a swap
//! succeed:
//!
//! | clause | what would break it | what checks it |
//! | --- | --- | --- |
//! | no dropped operation | an operation accepted before the swap and never answered after it | [`chaos::Load`]'s ledger, unchanged — the same client the kill test uses, so *dropped* means here exactly what it means there |
//! | nothing answered twice or wrongly | the outgoing instance's completion and the incoming one's both arrive | the ledger again, and a read-back through a *later* generation |
//! | the transfer verified | the incoming instance serving a client out of state it did not actually receive | three checks at three levels, below |
//!
//! # Why the client is `chaos::Load` and not a client of this module's own
//!
//! Because *dropped*, *doubled* and *answered wrongly* already have definitions
//! in this crate, they are the definitions gate G1's claim is written against,
//! and a swap harness with its own client would be a second opinion about what a
//! client observes. What is different here is what happens to the occupant, and
//! that is the only thing that should differ.
//!
//! It also buys the sharpest single number in this file. A kill costs the client
//! every buffer it had lent and every registration it held — [`Report::redone`]
//! counts them — and RFC 0063 says `in_place` exists precisely to make that
//! number zero. So the same counter, over the same client, is the *whole* of
//! what a swap buys over a restart, and this harness reports it for both.
//!
//! # The three levels the transfer is verified at
//!
//! One check would be a placement rather than a claim, which is the finding
//! `sim/src/chaos.rs` records about its own first draft.
//!
//! - **In the record.** `f_virtio_blk::state::Record` carries a check word over
//!   its own twenty-eight bytes, and the *reader* refuses a record that did not
//!   cross intact. [`Swap::garble`] flips a byte in the window on purpose and
//!   the run must abandon rather than commit.
//! - **In the two tables.** The outgoing instance's live registration count is
//!   read out of its real `f_ring::registry::Table` before it is retired, and
//!   the incoming instance's out of *its* real table after the replay. Two
//!   counts, on opposite sides, neither derived from the other, required equal —
//!   `claims/0012`'s discipline, for `claims/0012`'s reason.
//! - **In the client.** The client keeps submitting with the `SetId` the
//!   *outgoing* instance issued it, and never re-registers, because it is never
//!   told anything happened. A swap that handed over nothing would leave that id
//!   naming a slot in a table that has never been filled, and the client would be
//!   refused. [`Swap::amnesiac`] is that swap, and the run must go red.
//!
//! # Something has to be able to drop an operation
//!
//! [`Swap::hasty`] commits the routing word without waiting for the occupant to
//! drain — the cursors-only transfer RFC 0063 forecloses, built here so that
//! foreclosing it can be shown to matter. The outgoing instance is retired
//! holding work it accepted and never answered; the client's ledger reports it
//! as lost; [`verdict`] fails the run. A harness whose *no operation was
//! dropped* has never been able to read anything else is a harness reporting on
//! its own wiring.
//!
//! # What this does not cover, said rather than left to be inferred
//!
//! **There is no boot.** RFC 0032 puts the frame's own instructions in QEMU and
//! this crate above them, so the swap here is a swap of *modelled occupants in a
//! modelled place*, driven by the real protocol in `f_abi::swap` and handing
//! over the real state records `user/virtio-blk/src/state.rs` declares. The
//! frame's own half — `kernel/src/component.rs`, where a place lives — cannot
//! run it yet for a reason that is structural rather than unfinished, and
//! `SWAP_GAP` in `xtask/src/main.rs` is that residue as a checked quantity
//! rather than a paragraph: a swap needs *two generations of one component* in
//! one boot module set, and `cargo xtask component` produces one file per
//! component. It is the same shape as `CHAOS_GAP` one task back, for the same
//! reason, and it goes red the day it stops being true.
//!
//! **Both declarations are the same manifest.** There is one build of each
//! component in this tree, so what varies across the swap is the *instance* and
//! not the declaration. That is the honest reading of a one-build tree and it is
//! also the case RFC 0063 says is hardest to get right — two builds that agree
//! on all three fields and must therefore actually transfer. The fields being
//! *compared* are read out of the compiled record either way ([`Swap::declared`]),
//! so a manifest that changed its mode changes this run.

use std::collections::BTreeMap;

use f_abi::swap::{Abandoned, Routing, Swap as Protocol, Tally};
use f_abi::transfer::Declaration;
use f_virtio_blk::state::Record as State;

use crate::chaos::{self, Load, Policy, half, phase, value};
use crate::deploy::{Component, Deployment};
use crate::dev::{Config, Device};
use crate::native::Native;
use crate::proto::kind;
use crate::scenario::Peer;
use crate::{Actor, ActorId, Message, Outcome, Simulation, Trouble, World};

/// How many messages a swap run may deliver before it is called stuck.
///
/// The same bound `chaos` uses and for the same reason: it is here to catch a
/// model that loops rather than to bound one that works. A client waiting
/// forever on a place that paused and never resumed is exactly the failure this
/// harness looks for, so it must arrive as [`Trouble::Budget`] and not as a
/// wedged process.
pub const BUDGET: u32 = 1_000_000;

/// What one actor says to another, beyond [`crate::proto::kind`].
pub mod kind_swap {
    /// The place to itself: begin a swap now.
    ///
    /// A message rather than a call, for the reason `chaos`'s kill is one: the
    /// instant has to be an event on the timeline so that it lands in whatever
    /// interleaving the seed chose, which is the whole of *under sustained
    /// load*.
    pub const SWAP: &str = "swap";
}

/// What a swap run writes into the artefact.
///
/// Every one of them is read by [`Report::of`], which is why they are constants:
/// an assertion matching a string no record carries would pass forever.
pub mod wrote {
    /// An occupant went into the place. Detail: the generation it went in at.
    pub const SPAWNED: &str = "spawned";
    /// Phase A began: the routing word is paused and the place is holding pends.
    /// Detail: operations the occupant had accepted and not answered at that
    /// instant — **what makes *under load* a number rather than a hope.**
    pub const PAUSED: &str = "paused";
    /// The outgoing occupant wrote its state records into the window.
    /// Detail: how many.
    pub const HANDED: &str = "handed";
    /// The incoming occupant replayed them into its own table. Detail: how many.
    pub const ADOPTED: &str = "adopted";
    /// Live registrations, counted through a real table. Token: 0 on the
    /// outgoing side before it was retired, 1 on the incoming side after the
    /// replay. Detail: the count.
    ///
    /// Two records per swap and never one, because *the transfer is verified*
    /// means two tallies neither of which derives from the other.
    pub const SETS: &str = "sets";
    /// The two counts above disagreed. **A failure**, and the one that says the
    /// state did not cross.
    pub const MISMATCH: &str = "mismatch";
    /// The routing word committed. Detail: the generation it now carries.
    pub const SWAPPED: &str = "swapped";
    /// The place was refilled by a teardown and a spawn, because a declaration
    /// said `restart_only`. Detail: the generation.
    ///
    /// Counted apart from [`SWAPPED`] and never summed with it: RFC 0012 is
    /// explicit that a generation in which every place restarted is a reboot in
    /// instalments and the artefact has to show that rather than report a swap.
    pub const RESTART: &str = "restart";
    /// Phase A stopped with the outgoing occupant still serving.
    /// Detail: an [`Abandoned`] ordinal — see [`reason`].
    pub const ABANDON: &str = "abandon";
    /// A submission arrived while the routing word was paused and is waiting.
    /// Detail: how many are waiting.
    pub const PENDED: &str = "pended";
    /// The commit rang the doorbells the pending submissions were owed.
    /// Detail: how many.
    pub const RESUMED: &str = "resumed";
    /// Entries discarded because a restart voided the registration naming them.
    /// Detail: how many. They are not lost — the client's ledger still owes them.
    pub const VOIDED: &str = "voided";
}

/// The [`Abandoned`] reason a trace detail names, and back.
///
/// An ordinal in the trace rather than a string, because a trace record's detail
/// is a `u64` and the alternative is a second kind per reason — five columns for
/// one fact. Counting from one so that a detail of zero is a record that did not
/// set it.
pub mod reason {
    use f_abi::swap::Abandoned;

    /// The ordinal for one reason. Unit: none.
    #[must_use]
    pub const fn of(why: Abandoned) -> u64 {
        match why {
            Abandoned::Incompatible => 1,
            Abandoned::RingsNotEmpty => 2,
            Abandoned::NotQuiescent => 3,
            Abandoned::WindowTooSmall => 4,
            Abandoned::Refused => 5,
        }
    }

    /// The reason an ordinal names, or `"?"` for one nothing wrote.
    #[must_use]
    pub const fn label(ordinal: u64) -> &'static str {
        match ordinal {
            1 => Abandoned::Incompatible.label(),
            2 => Abandoned::RingsNotEmpty.label(),
            3 => Abandoned::NotQuiescent.label(),
            4 => Abandoned::WindowTooSmall.label(),
            5 => Abandoned::Refused.label(),
            _ => "?",
        }
    }
}

/// What the place is called in the trace. At most [`crate::LABEL_WIDTH`] bytes.
///
/// The same word `chaos` uses, because it is the same thing: RFC 0041's place,
/// an address that outlives its occupants. What separates the two artefacts is
/// the covering header, which is in the hashed bytes.
pub const PLACE: &str = "place";

/// What the client is called in the trace. `chaos::LOAD`, because it *is* that
/// client — see the module documentation.
pub const LOAD: &str = chaos::LOAD;

/// The answer a read gets at a position nothing has written.
const NOTHING: u64 = u64::MAX;

// ----------------------------------------------------------------- the store

/// The state behind a component, which neither a swap nor a restart touches.
///
/// A disk's sectors do not move because its driver was updated. Held by the
/// place for `chaos::Place`'s reason, and it is what makes the read-back a
/// question about durability rather than about the transfer.
type Store = BTreeMap<u64, u64>;

/// One operation the current occupant has taken and not yet answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Accepted {
    /// Where on the medium, taken off the client's own `Sqe::offset` and not
    /// derived from the token. Unit: per-peer.
    at: u64,
    /// Whether this operation puts a value there.
    write: bool,
}

// -------------------------------------------------------------- the occupant

/// What a place needs of whatever occupies it, beyond being an [`Actor`].
///
/// Three questions and no more, and each of them is one RFC 0063 names: *do you
/// hold anything*, *what would you hand over*, and *can you take this*. They are
/// on a trait rather than on the two models because a place holds *a component*
/// and which model is behind it is the deployment's answer — the same argument
/// `chaos::Place::spawn` makes about a function pointer.
pub trait Occupant: Actor {
    /// Whether this instance holds nothing it has accepted and not answered.
    ///
    /// **The occupant's own assertion**, and the half no cursor can supply.
    /// The frame asks it only after its own ring-empty check has agreed, so an
    /// instance that answered `true` unconditionally still could not be swapped
    /// with work on the wire.
    fn holds_nothing(&self) -> bool;

    /// The state records this instance would hand over, or `None` when it has
    /// nothing it can honestly hand over. See `Service::handover`.
    fn hand_over(&self) -> Option<Vec<State>>;

    /// Replay a window into this instance's own table. `false` for a window
    /// that did not cross intact.
    fn take_over(&mut self, records: &[State]) -> bool;

    /// Live registrations, counted through this instance's real table.
    /// Unit: buffer sets.
    fn live_sets(&self) -> u32;
}

impl<P: crate::dev::Protocol + 'static> Occupant for Device<P> {
    fn holds_nothing(&self) -> bool {
        self.quiescent()
    }

    fn hand_over(&self) -> Option<Vec<State>> {
        self.registrations().handover()
    }

    fn take_over(&mut self, records: &[State]) -> bool {
        self.adopt(records)
    }

    fn live_sets(&self) -> u32 {
        u32::try_from(self.registrations().registered()).unwrap_or(u32::MAX)
    }
}

impl Occupant for Native {
    fn holds_nothing(&self) -> bool {
        self.quiescent()
    }

    fn hand_over(&self) -> Option<Vec<State>> {
        self.registrations().handover()
    }

    fn take_over(&mut self, records: &[State]) -> bool {
        self.adopt(records)
    }

    fn live_sets(&self) -> u32 {
        u32::try_from(self.registrations().registered()).unwrap_or(u32::MAX)
    }
}

// ----------------------------------------------------------------- the place

/// How many times a swap may be put off for want of anything in flight.
///
/// Sixteen, `chaos::DEFERRALS_MAX`'s number for its reason: a swap that finds an
/// idle occupant is not the experiment, so it is rescheduled — but a client that
/// has finished its work will never be busy again, and a scheduler that kept
/// trying would turn a finished run into [`Trouble::Budget`]. Past this the swap
/// is abandoned, the plan is short by one, and [`verdict`] fails the run for it.
const DEFERRALS_MAX: u32 = 16;

/// One place, and the generation swap that happens at it.
pub struct Place {
    spawn: fn(Config) -> Box<dyn Occupant>,
    cfg: Config,
    // No restart policy and no budget, and their absence is the decision rather
    // than an omission. RFC 0063 is explicit that neither a swap nor a planned
    // restart at a place is a *fault*: an abandonment spends no restart budget
    // and waits no backoff because the occupant it would penalise behaved
    // correctly, and a `restart_only` replacement is an update the supervisor
    // asked for rather than an occupant that died. Charging either against
    // `max_restarts` would let a few failed updates retire a healthy
    // component's place, with the manifest's own budget as the mechanism —
    // which is the sentence that RFC's *an abandonment is not a restart*
    // paragraph exists to stop. The declared ladder is still what bounds the
    // client's added latency, and `verdict` is where it is read.
    /// Where a delivery goes, as the one word phase B stores.
    ///
    /// `f_abi::swap::Routing` and not a `bool` of this file's own: the ordering
    /// argument, the paused value and the `Release`/`Acquire` pair are the
    /// protocol's, and a harness that modelled them locally would be checking
    /// its own copy. What this crate cannot check is the *ordering* — it is
    /// deterministic and single-threaded on purpose, which is RFC 0004 — so
    /// `ring/tests/litmus.rs` is where the pair is asserted and this is where
    /// the sequence is.
    routing: Routing,
    occupant: Option<Box<dyn Occupant>>,
    client: Option<ActorId>,
    /// Which occupant this is, counting from one. [`f_abi::swap::PAUSED`] is
    /// zero, so a generation is never zero. Unit: instances.
    generation: u32,
    accepted: BTreeMap<u64, Accepted>,
    store: Option<Store>,
    /// The declarations the two sides carry. Read out of the compiled record,
    /// which is what stops the manifest being decoration.
    outgoing: Declaration,
    incoming: Declaration,
    /// Phase A is in progress and the place is holding pends.
    draining: bool,
    /// Live registrations on the outgoing side, taken at the moment it was asked
    /// to hand over and before it was retired. Unit: buffer sets.
    handed_sets: u32,
    /// Swaps left in the plan, taken, and abandoned. Unit: swaps.
    swaps_left: u32,
    swaps: u32,
    abandoned: u32,
    restarts: u32,
    deferrals: u32,
    settle_ns: u64,
    /// Commit the routing word without waiting for the occupant to drain.
    /// **The control that makes a dropped operation reachable.**
    hasty: bool,
    /// Flip a byte in the window after the outgoing occupant wrote it.
    /// **The control for the check word.**
    garble: bool,
    /// Hand over nothing while declaring `in_place`.
    /// **The control that says the transferred state is load-bearing.**
    amnesiac: bool,
}

impl Place {
    /// A place with its first occupant not yet in it.
    #[must_use]
    fn new(spawn: fn(Config) -> Box<dyn Occupant>, cfg: Config, swap: &Swap) -> Self {
        Self {
            spawn,
            cfg,

            // Zero: a place with no occupant delivers to nobody and holds its
            // pends, which is the state RFC 0008's *connect against an empty
            // place* is about and the state this one starts in.
            routing: Routing::new(f_abi::swap::PAUSED),
            occupant: None,
            client: None,
            generation: 0,
            accepted: BTreeMap::new(),
            store: swap.durable.then(Store::new),
            outgoing: swap.declared,
            // The same manifest on both sides: there is one build of each
            // component in this tree, so what varies across a swap here is the
            // instance and not the declaration. `Swap::declared` says why that
            // is the honest reading rather than a shortcut, and
            // `f_abi::swap::tests` is where a *disagreeing* pair — a narrower
            // incoming window, a bumped state-record schema — is asserted,
            // because those are decided from two manifests before a client is
            // held and need no run to reach.
            incoming: swap.declared,
            draining: false,
            handed_sets: 0,
            swaps_left: swap.swaps,
            swaps: 0,
            abandoned: 0,
            restarts: 0,
            deferrals: 0,
            settle_ns: swap.settle_ns(),
            hasty: swap.hasty,
            garble: swap.garble,
            amnesiac: swap.amnesiac,
        }
    }

    /// Put the first occupant in and open the routing word.
    fn fill(&mut self, world: &mut World, me: ActorId) {
        self.occupant = Some((self.spawn)(self.cfg));
        self.generation = self.generation.saturating_add(1);
        self.routing.commit(self.generation);
        world.record(me, PLACE, wrote::SPAWNED, u64::from(self.generation), 0);
        self.ring_pends(world, me);
        self.arm(world, me);
    }

    /// Ring one doorbell per submission that met a paused place.
    ///
    /// The entry never moved: it has been sitting in the shared region the whole
    /// time, and what was deferred is the bell. This is the mechanism RFC 0008's
    /// pending connect rests on, exercised on every swap rather than described.
    fn ring_pends(&mut self, world: &mut World, me: ActorId) {
        let Some(client) = self.client else { return };
        let waiting = world.wire().queued(client, me);
        for _ in 0..waiting {
            world.send(0, me, Message { from: client, kind: kind::SUBMIT, token: 0, detail: 0 });
        }
        if waiting > 0 {
            world.record(me, PLACE, wrote::RESUMED, u64::from(self.generation), u64::from(waiting));
        }
    }

    /// Schedule the next swap, if the plan still has one.
    fn arm(&mut self, world: &mut World, me: ActorId) {
        if self.swaps_left == 0 {
            return;
        }
        let spread = self.settle_ns.max(1);
        let delay = self.settle_ns.saturating_add(world.draw() % spread);
        world.send(delay, me, Message { from: me, kind: kind_swap::SWAP, token: 0, detail: 0 });
    }

    /// Phase A begins: stop delivering, and let the occupant drain.
    ///
    /// It refuses to begin on an idle occupant and reschedules instead, for
    /// `chaos::Place::kill`'s reason: a swap begun between operations is a swap
    /// of a quiescent system, which is the weaker experiment this one is named
    /// after not being.
    fn begin(&mut self, world: &mut World, me: ActorId) {
        if self.occupant.is_none() || self.draining || self.swaps_left == 0 {
            return;
        }
        if self.accepted.is_empty() {
            if self.deferrals < DEFERRALS_MAX {
                self.deferrals = self.deferrals.saturating_add(1);
                world.send(
                    self.settle_ns.max(1),
                    me,
                    Message { from: me, kind: kind_swap::SWAP, token: 0, detail: 0 },
                );
            }
            return;
        }

        let flying = u64::try_from(self.accepted.len()).unwrap_or(u64::MAX);
        self.draining = true;
        self.routing.pause();
        world.record(me, PLACE, wrote::PAUSED, u64::from(self.generation), flying);

        if self.hasty {
            // The cursors-only transfer RFC 0063 forecloses. The rings are not
            // empty and the occupant has not been asked anything; the word is
            // stored anyway. Everything the outgoing instance accepted and never
            // answered goes with it, which is a dropped operation produced by
            // skipping one step.
            self.finish(world, me);
        }
    }

    /// The occupant has drained: run the rest of phase A, then phase B.
    fn finish(&mut self, world: &mut World, me: ActorId) {
        if !self.draining {
            return;
        }
        self.swaps_left = self.swaps_left.saturating_sub(1);

        match Protocol::plan(&self.outgoing, &self.incoming) {
            // `restart_only` on either side. Not an abandonment and not a
            // failure: RFC 0063 says the place gets a restart, counted as RFC
            // 0012's `places_restarted` and never summed with a swap. It is
            // decided here rather than at the pause because the decision is the
            // same either way and a second branch would be a second place for it
            // to differ.
            Err(Abandoned::Incompatible) => self.restart(world, me),
            Err(why) => self.abandon(world, me, why),
            Ok(mut swap) => match self.transfer(world, me, &mut swap) {
                Ok(()) => {}
                Err(why) => self.abandon(world, me, why),
            },
        }
        self.arm(world, me);
    }

    /// Phase A's three remaining steps and phase B, in order.
    fn transfer(
        &mut self,
        world: &mut World,
        me: ActorId,
        swap: &mut Protocol,
    ) -> Result<(), Abandoned> {
        let Some(occupant) = self.occupant.as_ref() else { return Err(Abandoned::Refused) };

        // The frame's own precondition: producer stopped, consumer caught up.
        // `accepted` is what this place forwarded and has not answered, which is
        // the completion side; the submission side is whatever the client left
        // in the shared region, which pends rather than being lost.
        // `hasty` is the deliberate defect and it is exactly one thing: the
        // frame answering both of these `true` without asking. That is the
        // cursors-only transfer RFC 0063 forecloses, and forecloses on the
        // grounds that an occupant holding work the cursors cannot see would be
        // retired with it — so the defect has to be a *lie to the protocol*
        // rather than a missing call, or it would be testing that `Swap` refuses
        // when it is asked correctly, which `f_abi::swap::tests` already does.
        let cursors = self.hasty || self.accepted.is_empty();
        swap.drained(cursors)?;
        // And the half no cursor can supply.
        swap.asserted(self.hasty || occupant.holds_nothing())?;

        // The window. Bought by the incoming instance out of its own account,
        // which is why the bytes are laid out before the outgoing occupant is
        // asked for anything: RFC 0063 refuses the frame allocating it and
        // refuses the outgoing account paying, and the sequence is what says so.
        let width = usize::try_from(self.incoming.record_bytes).unwrap_or(0);
        let capacity = usize::try_from(swap.window_bytes()).unwrap_or(0);
        let mut window = vec![0u8; capacity];

        let handed = if self.amnesiac {
            // The control: an instance that declares `in_place` and hands over
            // nothing. Every step of the protocol succeeds and the client is the
            // only thing left that can notice.
            Vec::new()
        } else {
            occupant.hand_over().ok_or(Abandoned::Refused)?
        };
        let records = u32::try_from(handed.len()).unwrap_or(u32::MAX);
        swap.wrote(records)?;
        for (nth, record) in handed.iter().enumerate() {
            let at = nth.saturating_mul(width);
            let Some(slot) = window.get_mut(at..at.saturating_add(width)) else {
                // Unreachable while `swap.wrote` has already bounded the count
                // against the window it sized, and refused rather than trusted
                // for R04's reason.
                return Err(Abandoned::WindowTooSmall);
            };
            slot.copy_from_slice(&record.to_bytes());
        }
        if self.garble
            && let Some(byte) = window.first_mut()
        {
            // The control for the check word: one byte of the window, flipped
            // after it was written and before it is read. The record's own
            // reader is what has to catch it.
            *byte ^= 0x80;
        }

        // Read back out of the window rather than from `handed`, because a
        // transfer that verified the records it still held in memory would be
        // verifying nothing about the bytes that crossed.
        let mut crossed = Vec::with_capacity(handed.len());
        for nth in 0..records as usize {
            let at = nth.saturating_mul(width);
            let Some(slot) = window.get(at..at.saturating_add(width)) else {
                return Err(Abandoned::Refused);
            };
            let bytes: [u8; 32] = slot.try_into().map_err(|_| Abandoned::Refused)?;
            crossed.push(State::from_bytes(&bytes));
        }

        // The first of the two tallies, through the outgoing instance's own
        // real table and while it is still alive.
        self.handed_sets = occupant.live_sets();
        world.record(me, PLACE, wrote::HANDED, u64::from(self.generation), u64::from(records));
        world.record(me, PLACE, wrote::SETS, 0, u64::from(self.handed_sets));

        // The incoming instance, built and paid for before the word moves.
        let mut incoming = (self.spawn)(self.cfg);
        if !incoming.take_over(&crossed) {
            // A record that did not cross intact. The incoming instance is
            // dropped here — which is the revocation RFC 0008 already describes,
            // and the whole reason an abandonment has no cleanup path of its own.
            return Err(Abandoned::Refused);
        }
        swap.acknowledged(true)?;

        // Phase B. One `Release` store, and after it the state belongs to the
        // incoming occupant.
        let next = self.generation.saturating_add(1);
        swap.commit(&self.routing, next)?;
        self.occupant = Some(incoming);
        self.generation = next;
        self.swaps = self.swaps.saturating_add(1);

        // The second tally, through the incoming instance's real table, after
        // the replay. Neither count derives from the other and the verdict
        // requires them equal.
        let adopted = self.occupant.as_ref().map_or(0, |instance| instance.live_sets());
        world.record(me, PLACE, wrote::ADOPTED, u64::from(next), u64::from(records));
        world.record(me, PLACE, wrote::SETS, 1, u64::from(adopted));
        if adopted != self.handed_sets {
            world.record(me, PLACE, wrote::MISMATCH, u64::from(next), u64::from(adopted));
        }
        world.record(me, PLACE, wrote::SWAPPED, u64::from(next), u64::from(records));

        self.draining = false;
        // Work the outgoing instance had accepted and never answered goes with
        // it. On a correct run there is none — that is what the drain was — and
        // under `hasty` there is, which is exactly the point.
        self.accepted.clear();
        self.ring_pends(world, me);
        Ok(())
    }

    /// Phase A stopped. The place resumes delivering to the occupant it had.
    ///
    /// No restart is spent, no backoff is waited, and the outgoing occupant is
    /// untouched — which is the difference between this and the supervisor's
    /// fault path, and the reason it is counted under its own name.
    fn abandon(&mut self, world: &mut World, me: ActorId, why: Abandoned) {
        self.abandoned = self.abandoned.saturating_add(1);
        self.draining = false;
        // The running generation keeps the place. RFC 0012's counter goes back
        // to what it was, which is what a reader who saw zero learns.
        self.routing.commit(self.generation);
        world.record(me, PLACE, wrote::ABANDON, u64::from(self.generation), reason::of(why));
        self.ring_pends(world, me);
    }

    /// A place whose declaration says `restart_only`: torn down and refilled.
    ///
    /// The pends are held across it, the client is told its peer is gone so that
    /// it may take its buffers back on the one piece of evidence RFC 0024
    /// permits, and the new occupant answers what was waiting. It costs the
    /// client every registration it held, which is the number `in_place` exists
    /// to make zero and which this harness prints for both.
    fn restart(&mut self, world: &mut World, me: ActorId) {
        self.occupant = None;
        self.accepted.clear();
        self.draining = false;
        self.restarts = self.restarts.saturating_add(1);

        // Entries the outgoing instance never took, and answers it published and
        // never handed on. Discarded rather than re-rung: they name a buffer set
        // the next occupant never issued, and a client that had them served
        // would be a client whose registration outlived the instance that
        // granted it. The ledger still owes every one of them.
        let mut voided = 0u64;
        if let Some(client) = self.client {
            while world.wire().take(client, me).is_some() {
                voided += 1;
            }
        }
        while world.wire().take(me, me).is_some() {
            voided += 1;
        }
        while world.wire().reap(me, me).is_some() {
            voided += 1;
        }
        // And the answers this place had already published to the client and
        // that the client has not yet taken. Dropped rather than left, and this
        // is the one that is easy to miss: a completion naming a token the
        // client is about to reclaim on the evidence that its peer is gone
        // would be two owners of one buffer with one of them a device — the
        // failure `PeerGone` is sound only because the frame prevents it, and
        // the failure `chaos::wrote::STALE` exists to catch. The client's ledger
        // still owes the operation, so this is a delay and not a loss.
        if let Some(client) = self.client {
            while world.wire().reap(me, client).is_some() {
                voided += 1;
            }
        }
        world.record(me, PLACE, wrote::VOIDED, u64::from(self.generation), voided);

        if let Some(client) = self.client {
            world.send(0, client, Message { from: me, kind: kind::GONE, token: 0, detail: 0 });
        }

        self.occupant = Some((self.spawn)(self.cfg));
        self.generation = self.generation.saturating_add(1);
        self.routing.commit(self.generation);
        world.record(me, PLACE, wrote::SPAWNED, u64::from(self.generation), 0);
        world.record(me, PLACE, wrote::RESTART, u64::from(self.generation), 0);
        self.ring_pends(world, me);
    }

    /// A submission from the client: move it onto the occupant's channel.
    fn forward(&mut self, world: &mut World, me: ActorId, from: ActorId) {
        if *self.client.get_or_insert(from) != from {
            world.record(me, PLACE, wrote::VOIDED, u64::from(from.0), NOTHING);
            return;
        }
        if !self.routing.open() || self.occupant.is_none() {
            // The routing word is paused, so this pends. Nothing is refused and
            // nothing is dropped: the entry stays where the client put it and
            // the commit rings for it.
            let waiting = world.wire().queued(from, me);
            world.record(me, PLACE, wrote::PENDED, u64::from(self.generation), u64::from(waiting));
            return;
        }
        let Some(entry) = world.wire().take(from, me) else { return };
        let token = entry.user_data;
        self.accepted
            .insert(token, Accepted { at: entry.offset, write: phase(token) == half::WRITE });
        world.wire().post(me, me, entry);
        self.deliver_down(world, me, Message { from: me, kind: kind::SUBMIT, token, detail: 0 });
    }

    /// A completion from the occupant: apply the medium, and hand it on.
    fn answer(&mut self, world: &mut World, me: ActorId) {
        let Some(mut cqe) = world.wire().reap(me, me) else { return };
        let token = cqe.user_data;
        let taken = self.accepted.remove(&token);

        if let Some(entry) = taken
            && self.store.is_some()
            && cqe.result >= 0
        {
            // A write is on the medium before its completion leaves this
            // function. `chaos` is where that ordering is the claim and where
            // its negative controls live; here it is the reason a read-back
            // through a *later generation* means anything.
            if entry.write {
                if let Some(store) = self.store.as_mut() {
                    store.insert(entry.at, value(chaos::op(token)));
                }
            } else if phase(token) == half::READ {
                cqe.ext = self
                    .store
                    .as_ref()
                    .and_then(|store| store.get(&entry.at))
                    .copied()
                    .unwrap_or(NOTHING);
            }
        }

        if let Some(client) = self.client {
            world.wire().answer(me, client, cqe);
            world.send(0, client, Message { from: me, kind: kind::CQE, token, detail: 0 });
        }

        // The drain is over the moment the occupant owes nothing. Checked here
        // rather than on a timer, because *the occupant has answered everything
        // it accepted* is an event and a timer would be a guess about when it
        // happens.
        if self.draining && self.accepted.is_empty() {
            self.finish(world, me);
        }
    }

    fn deliver_down(&mut self, world: &mut World, me: ActorId, message: Message) {
        if let Some(occupant) = self.occupant.as_mut() {
            occupant.deliver(world, me, message);
        }
    }
}

impl Actor for Place {
    fn name(&self) -> &'static str {
        PLACE
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::START => self.fill(world, me),
            kind_swap::SWAP => self.begin(world, me),
            kind::SUBMIT if message.from != me => self.forward(world, me, message.from),
            kind::CQE if message.from == me => self.answer(world, me),
            // The occupant reset itself. Treated as a death, because it is one —
            // and this harness arms nothing to produce it, so a run that reaches
            // here is reporting a model that surprised it. R04: handled rather
            // than dropped, and `verdict` fails the run on the restart count.
            kind::GONE if message.from == me => self.restart(world, me),
            _ => self.deliver_down(world, me, message),
        }
    }
}

// ------------------------------------------------------------------- the run

/// How many translations one occupant's domain holds. Unit: translations.
///
/// Two, `chaos::DOMAIN`'s number for its reason: the client registers one set at
/// a time and the spare is so that a domain running out is a configuration away
/// rather than a code change. It matters more here than there — the incoming
/// instance's domain has to hold everything the replay registers.
const DOMAIN: u32 = 2;

/// Client phases every component in this deployment is driven through.
///
/// Ninety-six, `chaos::OPERATIONS`'s number and for its argument: it is the one
/// the run's *length* is made of, and length is what a swap plan needs in order
/// to land mid-flight. Held equal between components so that two results are
/// comparable. Unit: phases.
const PHASES: u32 = 96;

/// One swap run's configuration.
///
/// No `PartialEq`, unlike `chaos::Chaos`: it carries an
/// `f_abi::transfer::Declaration`, which is a wire type and deliberately has
/// none. Comparing two of those is `Declaration::transfers_to`'s job and it
/// compares three fields on purpose — a derived equality beside it would be a
/// second answer to the one question that decides whether a swap may happen.
#[derive(Clone, Copy, Debug)]
pub struct Swap {
    /// The component this run replaces, by the name its record declares.
    pub name: &'static str,
    /// What the simulator puts behind the place.
    pub peer: Peer,
    /// The transfer declaration the component's manifest carries.
    ///
    /// Read out of the compiled record and never written here, which is what
    /// keeps `user/virtio-blk/manifest.toml`'s `[transfer]` table from being
    /// decoration: a manifest that changed its mode changes this run from a swap
    /// into a restart, and the verdict asks a different question.
    pub declared: Declaration,
    /// The restart policy the manifest declares, for the latency bound.
    pub policy: Policy,
    /// How many operations the client keeps outstanding. Unit: operations.
    pub window: u32,
    /// How much the occupant will hold at once. Unit: operations.
    pub depth: u32,
    /// Logical operations the client issues. Unit: operations.
    pub operations: u32,
    /// The shortest the occupant takes over one operation. Unit: nanoseconds.
    pub service_ns: u64,
    /// How much longer than that it may take. Unit: nanoseconds.
    pub spread_ns: u64,
    /// How long a refused client waits before submitting again.
    /// Unit: nanoseconds.
    pub retry_ns: u64,
    /// Bytes in each buffer of the client's registered set. Unit: bytes.
    pub buffer_bytes: u32,
    /// How large the peer is. Unit: per-peer.
    pub extent: u64,
    /// How many times the occupant is replaced. Unit: swaps; zero is the control
    /// run, and without it the survival proves nothing.
    pub swaps: u32,
    /// Whether there is state behind the place, and therefore a read-back.
    pub durable: bool,
    /// Commit the routing word without waiting for the occupant to drain.
    ///
    /// **The negative control for the first clause of `E2-P08`'s exit.** RFC
    /// 0063 forecloses a cursors-only transfer and this is that transfer, built
    /// so that foreclosing it can be shown to matter: the outgoing instance is
    /// retired holding work it accepted and never answered, and the client's
    /// ledger reports the loss. `true` is the bug; `false` is the design.
    pub hasty: bool,
    /// Flip a byte in the transfer window after it is written.
    ///
    /// **The negative control for the check word.** A record that did not cross
    /// intact has to be refused by its *reader* — `f_virtio_blk::state::Record`
    /// — and the swap abandoned with the outgoing occupant still serving. A
    /// harness that only ever handed over intact windows would be reporting on a
    /// check nobody has watched fail.
    pub garble: bool,
    /// Hand over nothing while both sides declare `in_place`.
    ///
    /// **The negative control that says the transferred state is load-bearing.**
    /// Every step of the protocol succeeds and the routing word commits; the
    /// incoming instance's table has never been filled; the `SetId` the client
    /// still holds names nothing. If the run stays green with this on, then
    /// whatever crossed was not what the client depended on and the green run
    /// beside it means nothing.
    pub amnesiac: bool,
}

impl Swap {
    /// The workload every component in the deployment is driven with.
    ///
    /// One shape for every component on purpose: what differs between two runs
    /// is the component — its protocol, its ring, and above all its declared
    /// transfer mode — and a workload that differed too would make two results
    /// incomparable.
    #[must_use]
    pub fn of(component: &Component, swaps: u32) -> Self {
        let durable = matches!(component.peer, Peer::Blk | Peer::Native);
        Self {
            name: Box::leak(component.name.clone().into_boxed_str()),
            peer: component.peer,
            declared: component.transfer,
            policy: Policy {
                backoff_first_ticks: component.backoff_first_ticks,
                backoff_max_ticks: component.backoff_max_ticks,
                max_restarts: component.max_restarts,
                budget_window_ticks: component.budget_window_ticks,
                restart: component.restart,
            },
            window: 4,
            depth: 8,
            operations: PHASES / if durable { 2 } else { 1 },
            service_ns: 400,
            spread_ns: 600,
            retry_ns: 2_000,
            buffer_bytes: 512,
            extent: 4_096,
            swaps,
            durable,
            hasty: false,
            garble: false,
            amnesiac: false,
        }
    }

    /// Whether this component's declaration permits a transfer at all.
    ///
    /// The question asked of two manifests before anything drains, and the one
    /// that decides which half of [`verdict`] a run is judged by.
    #[must_use]
    pub const fn transfers(&self) -> bool {
        self.declared.in_place()
    }

    /// How long a place waits after a spawn before the next swap is due.
    ///
    /// `chaos::Chaos::settle_ns`'s arithmetic: enough for the client to have
    /// work in flight and not enough for it to have finished. Unit: nanoseconds.
    const fn settle_ns(&self) -> u64 {
        self.service_ns.saturating_mul(4).saturating_add(self.spread_ns)
    }

    /// Run this configuration at `seed`.
    ///
    /// # Errors
    ///
    /// [`Trouble`] if the run does not finish inside [`BUDGET`] or a message
    /// names an actor that does not exist. A client waiting for a place that
    /// paused and never resumed arrives here as [`Trouble::Budget`], which is
    /// the difference between reporting a hang and being one.
    pub fn run(&self, seed: u64) -> Result<Outcome, Trouble> {
        let mut sim = Simulation::new(seed, BUDGET);
        self.cover(&mut sim);

        let cfg = Config {
            depth: self.depth,
            service_ns: self.service_ns,
            spread_ns: self.spread_ns,
            lose_one_in: 0,
            extent: self.extent,
            queue_size: crate::virtq::QUEUE_SIZE,
            domain: DOMAIN,
            ordered: false,
        };
        let place = sim.install(Box::new(Place::new(spawner(self.peer), cfg, self)));
        let client = sim.install(Box::new(Load::new(
            0,
            place,
            self.window,
            self.operations,
            self.buffer_bytes,
            self.retry_ns,
            self.depth,
            self.durable,
        )));

        sim.world().send(0, place, Message { from: place, kind: kind::START, token: 0, detail: 0 });
        sim.world().send(
            0,
            client,
            Message { from: client, kind: kind::START, token: 0, detail: 0 },
        );
        sim.run()
    }

    /// Write the header that says what this run covers.
    ///
    /// The seed is deliberately absent, for `trace.rs`'s reason: a digest that
    /// moved with the seed on its own would make the reproduction check pass
    /// without a single decision changing.
    fn cover(&self, sim: &mut Simulation) {
        let world = sim.world();
        world.cover("f-sim artefact 1 — a component replaced under sustained load");
        world.cover("covers      a place, its routing word, two occupants and the state between");
        world.cover("not covered the frame's own swap: no boot carries two generations of one");
        world.cover("            component, and SWAP_GAP in xtask is that residue as a check");
        world.cover(&format!("component   {}", self.name));
        world.cover(&format!("modelled    as {}", self.peer.label()));
        world.cover(&format!(
            "declares    mode {}, schema {}, {} byte(s) per record, at most {}",
            f_abi::transfer::mode::label(self.declared.mode),
            self.declared.schema,
            self.declared.record_bytes,
            self.declared.records_max,
        ));
        world.cover(&format!(
            "workload    {} operation(s), window {}, {} swap(s), read-back {}",
            self.operations,
            self.window,
            self.swaps,
            if self.durable { "on" } else { "off" },
        ));
    }
}

/// How a peer is put into a place.
///
/// `chaos::spawner`'s shape, answering `Box<dyn Occupant>` instead of
/// `Box<dyn Actor>`: a place that can swap needs three things of its occupant
/// that a place that can only kill does not. It is also what makes an incoming
/// instance an *instance* — the same function, called again, with nothing
/// carried over except what crossed the window.
fn spawner(peer: Peer) -> fn(Config) -> Box<dyn Occupant> {
    match peer {
        Peer::Net => {
            |cfg| Box::new(Device::new(crate::net::Net, cfg).expect("the layout's own queue size"))
        }
        Peer::Gpu => |cfg| {
            Box::new(
                Device::new(crate::gpu::Gpu::default(), cfg).expect("the layout's own queue size"),
            )
        },
        Peer::Blk => {
            |cfg| Box::new(Device::new(crate::blk::Blk, cfg).expect("the layout's own queue size"))
        }
        Peer::Queue | Peer::Deployment | Peer::Native => {
            |cfg| Box::new(Native::new(cfg.depth, cfg.service_ns, cfg.spread_ns, cfg.domain))
        }
    }
}

// --------------------------------------------------------------- the verdict

/// What one swap run produced, read out of its artefact.
///
/// Out of the artefact and not out of the actors, for `chaos::Report`'s reason:
/// a number that existed only in memory is a number a failing seed cannot
/// report.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Report {
    /// Phases answered for the first time. Unit: phases.
    pub settled: u32,
    /// Phases the client issued and this run owed. Unit: phases.
    pub owed: u32,
    /// Operations never answered. **The first clause of the exit.**
    /// Unit: phases.
    pub lost: u32,
    /// Operations answered a second time. Unit: phases.
    pub twice: u32,
    /// Completions for tokens nobody held. Unit: completions.
    pub stale: u32,
    /// Answers that disagreed with what was written, read back through a later
    /// generation. Unit: phases.
    pub wrong: u32,
    /// Buffers that came back bearing another operation's stamp. Unit: buffers.
    pub torn: u32,
    /// Refusals the client could not retry. Unit: refusals.
    ///
    /// **The number [`Swap::amnesiac`] moves.** A client submitting with a
    /// `SetId` the incoming instance's table never received is refused, and a
    /// refusal is something other than added latency.
    pub failed: u32,
    /// Clients that observed anything other than added latency. Unit: clients.
    pub clients_failed: u32,
    /// Routing words committed to a newer generation in place. Unit: swaps.
    pub swapped: u32,
    /// Places torn down and refilled because a declaration said `restart_only`.
    /// Deliberately never summed with the row above. Unit: places.
    pub restarted: u32,
    /// Swaps that stopped in phase A. Unit: swaps.
    pub abandoned: u32,
    /// The reason the last abandonment gave, as a [`reason`] ordinal.
    /// Unit: none.
    pub abandoned_why: u64,
    /// The fewest operations in flight at any pause. Unit: operations.
    ///
    /// Zero would mean a swap began between operations, which is the weaker
    /// experiment this one is named after not being.
    pub flying_min: u32,
    /// State records written into a window, added up. Unit: records.
    pub handed: u32,
    /// State records replayed out of one, added up. Unit: records.
    pub adopted: u32,
    /// Swaps where the two live-registration counts disagreed. **A failure**,
    /// and the one that says the state did not cross. Unit: swaps.
    pub mismatch: u32,
    /// Live registrations counted on the outgoing side, summed over the run.
    /// Unit: buffer sets.
    pub sets_out: u32,
    /// The same on the incoming side. Unit: buffer sets.
    pub sets_in: u32,
    /// Submissions that pended because the routing word was paused.
    /// Unit: submissions.
    pub pended: u32,
    /// Entries a commit rang the doorbell for, added up. Unit: submissions.
    pub resumed: u32,
    /// Entries discarded because a restart voided the registration naming them,
    /// added up. Unit: submissions.
    ///
    /// The restart arm's half of [`Report::resumed`], and a separate counter
    /// because the two are different mechanisms with different costs: a swap
    /// *rings* for what pended and a restart *discards* it, leaving the client
    /// owing it again. One counter for both would let each arm's zero be
    /// explained by the other arm's mechanism.
    pub voided: u32,
    /// Buffers the client took back on peer-gone evidence. **What the change of
    /// occupant cost in work redone**, and the number RFC 0063 says `in_place`
    /// exists to make zero. Unit: operations.
    pub redone: u32,
    /// The worst latency any operation took. Unit: **virtual** nanoseconds.
    pub worst_ns: u64,
    /// Where the run's clock stopped. Unit: nanoseconds.
    pub finished_ns: u64,
    /// The artefact's digest.
    pub digest: u64,
}

impl Report {
    /// Read one run's artefact.
    #[must_use]
    pub fn of(swap: &Swap, outcome: &Outcome) -> Self {
        let phases = if swap.durable { 2 } else { 1 };
        let owed = swap.operations.saturating_mul(phases);
        let rows = || outcome.trace.records().iter();
        let count = |actor: &str, kind: &str| {
            u32::try_from(
                rows().filter(|record| record.actor == actor && record.kind == kind).count(),
            )
            .unwrap_or(u32::MAX)
        };
        let total = |actor: &str, kind: &str| {
            u32::try_from(
                rows()
                    .filter(|record| record.actor == actor && record.kind == kind)
                    .fold(0u64, |sum, record| sum.saturating_add(record.detail)),
            )
            .unwrap_or(u32::MAX)
        };
        // The two tallies are one record kind told apart by its token, so that
        // a run cannot report a count for one side that it never took for the
        // other: a missing row is a missing row rather than a zero.
        let sets = |side: u64| {
            u32::try_from(
                rows()
                    .filter(|record| {
                        record.actor == PLACE && record.kind == wrote::SETS && record.token == side
                    })
                    .fold(0u64, |sum, record| sum.saturating_add(record.detail)),
            )
            .unwrap_or(u32::MAX)
        };

        let settled = count(LOAD, chaos::wrote::SETTLED);
        let failed = count(LOAD, chaos::wrote::FAILED);
        let twice = count(LOAD, chaos::wrote::TWICE);
        let stale = count(LOAD, chaos::wrote::STALE);
        let wrong = count(LOAD, chaos::wrote::WRONG);
        let torn = count(LOAD, chaos::wrote::TORN);
        let noticed = failed + twice + stale + wrong + torn;

        let worst_ns = rows()
            .filter(|record| record.actor == LOAD && record.kind == chaos::wrote::SETTLED)
            .map(|record| record.detail)
            .max()
            .unwrap_or(0);
        let flying_min = rows()
            .filter(|record| record.actor == PLACE && record.kind == wrote::PAUSED)
            .map(|record| u32::try_from(record.detail).unwrap_or(u32::MAX))
            .min()
            .unwrap_or(0);
        let abandoned_why = rows()
            .filter(|record| record.actor == PLACE && record.kind == wrote::ABANDON)
            .map(|record| record.detail)
            .next_back()
            .unwrap_or(0);

        Self {
            settled,
            owed,
            lost: owed.saturating_sub(settled),
            twice,
            stale,
            wrong,
            torn,
            failed,
            clients_failed: u32::from(noticed > 0),
            swapped: count(PLACE, wrote::SWAPPED),
            restarted: count(PLACE, wrote::RESTART),
            abandoned: count(PLACE, wrote::ABANDON),
            abandoned_why,
            flying_min,
            handed: total(PLACE, wrote::HANDED),
            adopted: total(PLACE, wrote::ADOPTED),
            mismatch: count(PLACE, wrote::MISMATCH),
            sets_out: sets(0),
            sets_in: sets(1),
            pended: count(PLACE, wrote::PENDED),
            resumed: total(PLACE, wrote::RESUMED),
            voided: total(PLACE, wrote::VOIDED),
            redone: count(LOAD, crate::proto::wrote::RECLAIM),
            worst_ns,
            finished_ns: outcome.finished_ns,
            digest: outcome.digest(),
        }
    }

    /// What this run cost, in the three counters RFC 0012 and RFC 0063 keep
    /// apart.
    ///
    /// Built here rather than kept by the place, and the difference is the
    /// house rule this whole structure is under: a number that existed only in
    /// an actor's memory is a number a failing seed cannot report. Every one of
    /// these is a count of records in the artefact, so the tally a reader is
    /// handed and the tally the run produced are the same object read twice
    /// rather than two objects that can drift.
    #[must_use]
    pub const fn tally(&self) -> Tally {
        Tally {
            places_swapped: self.swapped,
            places_restarted: self.restarted,
            swaps_abandoned: self.abandoned,
        }
    }

    /// How many times the place changed its occupant, however it changed it.
    ///
    /// [`Tally::places_refilled`], and it is that function rather than a second
    /// sum for the reason the tally exists at all: *an abandonment refilled
    /// nothing* is a rule about these three numbers, and a rule written twice is
    /// a rule two readers can disagree about at the moment one of them is
    /// deciding whether an update landed.
    /// Unit: places.
    #[must_use]
    pub const fn refilled(&self) -> u32 {
        self.tally().places_refilled()
    }
}

/// What a swap run has to have produced, checked against what it did.
///
/// A free function taking the *pair* — the run with swaps in it and the control
/// beside it — because the pair is the claim. A survival with no control run is
/// evidence that nothing went wrong, not that anything was under test.
///
/// # Errors
///
/// A sentence naming what did not hold.
pub fn verdict(swap: &Swap, moved: &Report, calm: &Report) -> Result<(), String> {
    // The control first: if it fails, nothing the other run says means anything.
    if calm.refilled() != 0 || calm.abandoned != 0 {
        return Err("the control run replaced something, so it is not a control".into());
    }
    if calm.lost != 0 || calm.settled != calm.owed {
        return Err(format!(
            "the control run answered {} of {} operation(s) with nothing replaced, so the other \
             run's numbers are about the workload rather than about the swap",
            calm.settled, calm.owed
        ));
    }
    if calm.clients_failed != 0 {
        return Err("the control run's client observed a failure with nothing replaced".into());
    }

    // The exit's own clauses first, and this is the one place the order differs
    // from `chaos::verdict`'s. That function asks *was this a real experiment*
    // before it asks *did anything break*, because a green run over a smaller
    // experiment is the failure it exists to refuse. Both orders refuse exactly
    // the same runs; what changes is which sentence a reader is handed when one
    // goes red, and here the client-visible failure is the finding while every
    // positive below it describes a run that was otherwise clean.
    if moved.lost != 0 {
        return Err(format!(
            "{} operation(s) were submitted and never answered. *No client observes a dropped \
             operation* is the first clause of E2-P08's exit and this is it failing",
            moved.lost
        ));
    }
    if moved.twice != 0 || moved.stale != 0 {
        return Err(format!(
            "{} operation(s) were answered twice and {} completion(s) arrived for tokens \
             nobody held: the place served work on both sides of the routing word",
            moved.twice, moved.stale
        ));
    }
    if moved.wrong != 0 {
        return Err(format!(
            "{} answer(s) disagreed with what was written, read back through a later \
             generation, so the state behind the place did not survive the replacement",
            moved.wrong
        ));
    }
    if moved.torn != 0 {
        return Err(format!(
            "{} buffer(s) came back bearing another operation's stamp, which is a device \
             having written into memory the client had lent to something else",
            moved.torn
        ));
    }
    if moved.failed != 0 {
        return Err(format!(
            "{} refusal(s) the client could not retry, so a client observed the replacement as \
             an error rather than as a wait",
            moved.failed
        ));
    }
    // Then the experiment: it has to have happened, and mid-flight.
    if moved.refilled() != swap.swaps {
        return Err(format!(
            "the plan was {} replacement(s) and {} happened ({} in place, {} by restart), so \
             the run is a smaller experiment than the one it is named after",
            swap.swaps,
            moved.refilled(),
            moved.swapped,
            moved.restarted
        ));
    }
    if moved.flying_min == 0 {
        return Err("a swap began with nothing in flight, which is a swap of a quiescent \
                    system rather than a component replaced under load"
            .into());
    }
    if moved.pended == 0 {
        return Err("no submission ever met a paused routing word, so the client never met a \
                    place in phase A and the mechanism a swap rests on was not exercised"
            .into());
    }
    // The declaration decides which question the run is asked, and it is read
    // out of the compiled record rather than chosen here.
    if swap.transfers() {
        if moved.swapped != swap.swaps {
            return Err(format!(
                "the manifest declares `in_place` and {} of {} replacement(s) were swaps; the \
                 rest restarted, which is a component being updated the expensive way while \
                 its declaration says otherwise",
                moved.swapped, swap.swaps
            ));
        }
        // What pended has to have been rung for, and both numbers are counted
        // in submissions rather than one in submissions and one in doorbells: a
        // shortfall is a client waiting on a bell nobody rang, with its
        // operation carried by the retry rather than by the mechanism — which
        // is the mechanism not working while every count above still reads
        // clean.
        if moved.resumed < moved.pended {
            return Err(format!(
                "{} submission(s) pended across an in-place swap and the commits rang for {}",
                moved.pended, moved.resumed
            ));
        }
        if moved.voided != 0 {
            return Err(format!(
                "an in-place swap discarded {} submission(s). Nothing may be voided here: the \
                 registration naming them crossed rather than being retired, so an entry \
                 thrown away is a client's operation carried by its own retry",
                moved.voided
            ));
        }
        if moved.handed == 0 {
            return Err("the swaps handed over no state records at all, so the transfer is a \
                        mechanism that ran over nothing and the counts beside it are about an \
                        empty window"
                .into());
        }
        if moved.adopted != moved.handed {
            return Err(format!(
                "{} record(s) were written into a window and {} were replayed out of one. A \
                 transfer that lost a record between the two is exactly the failure the check \
                 word exists to catch, arriving as a count instead",
                moved.handed, moved.adopted
            ));
        }
        // The two tallies. Taken on opposite sides, through two real
        // registration tables, neither derived from the other.
        if moved.mismatch != 0 || moved.sets_in != moved.sets_out {
            return Err(format!(
                "the outgoing instances held {} live registration(s) between them and the \
                 incoming ones held {} after the replay, over {} swap(s) with {} mismatch(es). \
                 The state did not cross, and the client is being served out of a table that \
                 is not the one it was promised",
                moved.sets_out, moved.sets_in, moved.swapped, moved.mismatch
            ));
        }
        if moved.sets_out == 0 {
            return Err("the outgoing instances held no live registrations, so the two counts \
                        above agree at zero and say nothing. A swap of an instance with an \
                        empty table is a swap that transferred nothing worth transferring"
                .into());
        }
        // The number `in_place` exists for. A restart costs the client every
        // registration it held; a swap is supposed to cost it none, and this is
        // the only place in the tree where those two are the same counter over
        // the same client.
        if moved.redone != 0 {
            return Err(format!(
                "the client took {} buffer(s) back on peer-gone evidence across an in-place \
                 swap. RFC 0063 says the whole of what `in_place` buys is that this is zero: a \
                 client that re-registered has observed the update",
                moved.redone
            ));
        }
    } else {
        if moved.restarted != swap.swaps {
            return Err(format!(
                "the manifest declares `restart_only` and {} of {} replacement(s) were \
                 restarts. A swap here would be the supervisor transferring state a component \
                 said it would not hand over",
                moved.restarted, swap.swaps
            ));
        }
        if moved.handed != 0 || moved.adopted != 0 {
            return Err(format!(
                "a `restart_only` component handed over {} record(s) and adopted {}",
                moved.handed, moved.adopted
            ));
        }
        // A restart *discards* what pended rather than ringing for it: those
        // entries name a buffer set the next occupant never issued, and a
        // client that had them served would be a client whose registration
        // outlived the instance that granted it. Its ledger still owes every
        // one of them, which is what makes the discard a delay and not a loss —
        // and asserting it here is what stops the in-place arm's `voided == 0`
        // from being a number nothing in this harness can move.
        if moved.voided == 0 {
            return Err("a `restart_only` replacement discarded nothing, so the in-place \
                        component's zero voided entries is not a comparison — the two would \
                        agree whatever the mechanism did"
                .into());
        }
        // And the cost, asserted rather than assumed: a restart *does* make the
        // client work again, which is what makes the zero above a measurement.
        if moved.redone == 0 {
            return Err("a `restart_only` replacement cost the client no re-registration, so \
                        the zero the in-place component reports is not evidence of anything — \
                        the two would be equal whatever the mechanism did"
                .into());
        }
    }

    // The bound. *Only latency* with an unbounded tail is a hang with better
    // manners. Both terms are declared elsewhere — one by the control run and
    // one by the manifest — rather than tuned here.
    let bound = calm.worst_ns.saturating_add(swap.policy.ladder_ns(swap.swaps.max(1)));
    if moved.worst_ns > bound {
        return Err(format!(
            "the worst operation took {} ns against a bound of {} ns — {} ns of control plus a \
             declared ladder of {} ns. A latency past the ladder is a wait nothing in the \
             policy accounts for",
            moved.worst_ns,
            bound,
            calm.worst_ns,
            swap.policy.ladder_ns(swap.swaps.max(1))
        ));
    }
    Ok(())
}

// ------------------------------------------------------------- the whole set

/// One component's pair of runs, and what they produced.
#[derive(Clone, Debug)]
pub struct Pair {
    /// The configuration the replaced run used.
    pub swap: Swap,
    /// The run with replacements in it.
    pub moved: Report,
    /// The same run with none.
    pub calm: Report,
}

impl Pair {
    /// The added latency a client observed, worst case. Unit: nanoseconds.
    #[must_use]
    pub const fn added_ns(&self) -> u64 {
        self.moved.worst_ns.saturating_sub(self.calm.worst_ns)
    }
}

/// How many times each component is replaced in a sweep.
///
/// Two, and it is a bound rather than a preference: the second swap is what says
/// an instance that *arrived* through a transfer can be transferred out of again
/// — which is a different claim from *a transfer works once*, and the one a
/// long-lived deployment actually depends on. A third would run the same code
/// against the same mechanism.
pub const SWAPS: u32 = 2;

/// Replace every component in a deployment under load, and judge each pair.
///
/// **This is *each component in turn*.** The set is the deployment's — the
/// component files the build produced — rather than a list in this file, so a
/// component that cannot survive being replaced is a red build for whoever added
/// it. `cargo xtask swap` is what compares the number this reached against the
/// number of `manifest.toml` files the source tree carries, which is the tie
/// that stops a sweep and its coverage check being one directory read twice.
///
/// # Errors
///
/// The first pair that did not hold, naming the component and what failed.
pub fn sweep(deployment: &Deployment, seed: u64, swaps: u32) -> Result<Vec<Pair>, String> {
    if deployment.is_empty() {
        return Err("no components to replace. `cargo xtask swap` builds them first; a sweep \
                    over an empty set produces a stable digest and no evidence at all."
            .into());
    }
    let mut pairs = Vec::new();
    for component in deployment.components() {
        let swap = Swap::of(component, swaps);
        let mut control = swap;
        control.swaps = 0;

        let replaced = swap.run(seed).map_err(|why| {
            format!("{}: the replaced run did not finish — {}", component.name, why.message())
        })?;
        let untouched = control.run(seed).map_err(|why| {
            format!("{}: the control run did not finish — {}", component.name, why.message())
        })?;

        let moved = Report::of(&swap, &replaced);
        let calm = Report::of(&control, &untouched);
        verdict(&swap, &moved, &calm).map_err(|why| format!("{}: {why}", component.name))?;
        pairs.push(Pair { swap, moved, calm });
    }
    Ok(pairs)
}

/// The one number two sweeps are compared by.
///
/// Every pair's digest, folded in component order. FNV-1a, the same function
/// `trace.rs` hashes with, so there is one digest arithmetic in this crate.
#[must_use]
pub fn digest(pairs: &[Pair]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for pair in pairs {
        for word in [pair.moved.digest, pair.calm.digest] {
            for byte in word.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x1000_0000_01b3);
            }
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_SEED;
    use crate::deploy::fixture::component;
    use f_abi::transfer::mode;

    /// The seeds every assertion below is made at.
    const SEEDS: [u64; 4] = [DEFAULT_SEED, 1, 0x5EED_5EED, 0xDEAD_BEEF];

    /// The declaration `user/virtio-blk/manifest.toml` actually carries.
    ///
    /// Written out here rather than read off the fixture, because
    /// `deploy::fixture::record` declares `restart_only` on purpose — *the
    /// honest declaration, which is what a fixture not testing RFC 0063's field
    /// should carry* — and this is the one file that does test it. The real
    /// file's numbers are asserted against the manifest by
    /// `f_virtio_blk::state::tests::the_declaration_matches_the_manifest`, so
    /// this constant cannot drift from it without that test going red first.
    const IN_PLACE: Declaration = Declaration {
        schema: f_virtio_blk::state::SCHEMA,
        record_bytes: f_virtio_blk::state::RECORD_BYTES,
        records_max: f_virtio_blk::state::RECORDS_MAX,
        mode: mode::IN_PLACE,
        _reserved: [0; 3],
    };

    fn deployment() -> Deployment {
        Deployment::of(vec![component("virtio-blk", "blk", 256), component("store", "store", 16)])
            .expect("two names")
    }

    /// A component whose manifest declares an in-place transfer.
    fn blk(swaps: u32) -> Swap {
        let built = component("virtio-blk", "blk", 256);
        let mut swap = Swap::of(&built, swaps);
        swap.declared = IN_PLACE;
        swap
    }

    fn report(swap: &Swap, seed: u64) -> Report {
        Report::of(swap, &swap.run(seed).expect("a run that finishes"))
    }

    /// The exit's own sentence, at four seeds.
    #[test]
    fn a_component_is_replaced_under_load_and_the_client_observes_only_a_wait() {
        for seed in SEEDS {
            let swap = blk(SWAPS);
            let mut control = swap;
            control.swaps = 0;
            let moved = report(&swap, seed);
            let calm = report(&control, seed);
            verdict(&swap, &moved, &calm).unwrap_or_else(|why| panic!("seed {seed:#x}: {why}"));
            assert_eq!(moved.lost, 0);
            assert_eq!(moved.redone, 0, "an in-place swap costs the client no re-registration");
            assert!(moved.handed > 0 && moved.adopted == moved.handed);
            // The three counters RFC 0012 and RFC 0063 keep apart, read out
            // of the artefact: every replacement was a swap, none was a
            // restart, and nothing was abandoned.
            assert_eq!(
                moved.tally(),
                f_abi::swap::Tally {
                    places_swapped: SWAPS,
                    places_restarted: 0,
                    swaps_abandoned: 0,
                }
            );
            assert_eq!(moved.sets_in, moved.sets_out);
            assert!(moved.sets_out > 0, "the two counts must agree at something, not at zero");
        }
    }

    /// **The first clause's negative control.** A routing word committed
    /// without the drain loses the work the outgoing instance was holding.
    #[test]
    fn a_swap_that_skips_the_drain_drops_an_operation_and_is_caught() {
        // One swap and not two, so that the plan completes and the sentence the
        // verdict hands back is the one this control is about rather than *the
        // second swap never landed*, which is true as well and is a symptom.
        let mut swap = blk(1);
        swap.hasty = true;
        let mut control = swap;
        control.swaps = 0;
        control.hasty = false;

        let moved = report(&swap, DEFAULT_SEED);
        let calm = report(&control, DEFAULT_SEED);
        assert!(
            moved.lost > 0,
            "a swap that skipped the drain must actually lose something, or the green run              beside it is a claim nothing could have falsified"
        );
        assert_eq!(moved.swapped, 1, "and it must have committed rather than abandoned");
        let why = verdict(&swap, &moved, &calm).expect_err("the run stayed green with work lost");
        assert!(why.contains("never answered"), "{why}");
    }

    /// **The check word's negative control.** A byte flipped in the window is
    /// refused by the record's reader and the swap is abandoned, with the
    /// outgoing occupant still serving and the client none the wiser.
    #[test]
    fn a_garbled_window_is_refused_by_its_reader_and_the_swap_is_abandoned() {
        let mut swap = blk(SWAPS);
        swap.garble = true;
        let moved = report(&swap, DEFAULT_SEED);
        assert_eq!(moved.swapped, 0, "a corrupt window must not commit the routing word");
        assert!(moved.abandoned > 0, "and it must abandon rather than fail quietly");
        assert_eq!(
            reason::label(moved.abandoned_why),
            Abandoned::Refused.label(),
            "the abandonment has to name the record's reader and not some other step"
        );
        // The client's half: an abandonment costs latency and nothing else.
        assert_eq!(moved.lost, 0);
        assert_eq!(moved.failed, 0);
        assert_eq!(moved.clients_failed, 0);
    }

    /// **The state's negative control, and the strongest of the three.** A swap
    /// that transfers nothing commits cleanly and the *client* is what catches
    /// it — which is what makes the green run's transfer load-bearing rather
    /// than decorative.
    #[test]
    fn a_swap_that_hands_over_nothing_is_caught_by_the_client() {
        // One swap and not two, so the plan completes and the counters below
        // are about the swap that happened.
        let mut swap = blk(1);
        swap.amnesiac = true;
        let mut control = swap;
        control.swaps = 0;
        control.amnesiac = false;

        let moved = report(&swap, DEFAULT_SEED);
        let calm = report(&control, DEFAULT_SEED);
        assert_eq!(moved.swapped, 1, "the protocol itself has no way to notice; it must commit");
        assert_eq!(moved.handed, 0);
        // The harness's own half: two counts, through two real registration
        // tables, taken on opposite sides and neither derived from the other.
        assert!(moved.mismatch > 0, "the two live-registration counts must disagree");
        assert_eq!((moved.sets_out, moved.sets_in), (1, 0));
        // The mechanism's half, and the one that matters: the *client* is
        // refused for a buffer set the incoming table never received, and the
        // work it could not retry is never answered. A green run beside this one
        // is therefore a run whose transfer was load-bearing.
        assert!(moved.failed > 0, "the client noticed nothing, so nothing it needed crossed");
        assert!(moved.lost > 0);
        assert!(verdict(&swap, &moved, &calm).is_err(), "the run stayed green with no state");
    }

    /// A `restart_only` component takes a restart at the place, and it costs the
    /// client the re-registration the in-place one does not pay.
    #[test]
    fn a_component_that_declares_restart_only_is_judged_by_its_own_declaration() {
        let built = component("store", "store", 16);
        let swap = Swap::of(&built, SWAPS);
        assert!(!swap.transfers(), "the fixture declares the honest mode");
        let mut control = swap;
        control.swaps = 0;

        let moved = report(&swap, DEFAULT_SEED);
        let calm = report(&control, DEFAULT_SEED);
        verdict(&swap, &moved, &calm).expect("a restart at the place is not a failure");
        assert_eq!(moved.swapped, 0);
        assert_eq!(moved.restarted, SWAPS);
        assert_eq!(moved.handed, 0);
        assert!(
            moved.redone > 0,
            "a restart has to cost the client its registrations, or the in-place component's \
             zero is not a comparison"
        );
        assert_eq!(moved.lost, 0, "and still nothing may be dropped");
    }

    /// One seed reproduces its sweep and a different seed moves it.
    #[test]
    fn one_seed_reproduces_its_sweep_and_a_different_seed_moves_it() {
        let deployment = deployment();
        let first = sweep(&deployment, DEFAULT_SEED, SWAPS).expect("the fixture sweep");
        let again = sweep(&deployment, DEFAULT_SEED, SWAPS).expect("the fixture sweep");
        let other = sweep(&deployment, 0x1234, SWAPS).expect("the fixture sweep");
        assert_eq!(digest(&first), digest(&again));
        assert_ne!(
            digest(&first),
            digest(&other),
            "a digest over something that does not vary agrees with itself forever"
        );
    }

    #[test]
    fn a_sweep_over_no_components_is_refused() {
        // Fail closed, R04. An empty sweep produces a stable digest and no
        // evidence, which is the one result a check like this must never report
        // as a pass.
        assert!(sweep(&Deployment::default(), DEFAULT_SEED, SWAPS).is_err());
    }

    /// Every label fits the trace column, and no two record kinds share a word.
    #[test]
    fn every_label_fits_the_trace_column_and_no_two_share_a_word() {
        let kinds = [
            wrote::SPAWNED,
            wrote::PAUSED,
            wrote::HANDED,
            wrote::ADOPTED,
            wrote::SETS,
            wrote::MISMATCH,
            wrote::SWAPPED,
            wrote::RESTART,
            wrote::ABANDON,
            wrote::PENDED,
            wrote::RESUMED,
            wrote::VOIDED,
            kind_swap::SWAP,
        ];
        for (at, one) in kinds.iter().enumerate() {
            assert!(one.len() <= crate::LABEL_WIDTH, "`{one}` is wider than a trace column");
            for other in &kinds[at + 1..] {
                assert_ne!(one, other, "two events with one name");
            }
        }
        assert!(PLACE.len() <= crate::LABEL_WIDTH);
        assert!(LOAD.len() <= crate::LABEL_WIDTH);
    }

    /// Every abandonment reason has a distinct ordinal and reads back.
    #[test]
    fn a_reason_ordinal_names_exactly_one_reason() {
        let all = [
            Abandoned::Incompatible,
            Abandoned::RingsNotEmpty,
            Abandoned::NotQuiescent,
            Abandoned::WindowTooSmall,
            Abandoned::Refused,
        ];
        for why in all {
            assert_eq!(reason::label(reason::of(why)), why.label());
            assert_ne!(reason::of(why), 0, "zero is a record that set no reason");
        }
        assert_eq!(reason::label(0), "?");
    }
}
