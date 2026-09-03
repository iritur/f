// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Driver chaos: a place, an occupant that is killed under sustained load, and
//! a client that observes nothing except added latency.
//!
//! # The sentence this module exists to make checkable
//!
//! Gate G1: *a driver is killed under sustained load and the system does not
//! notice*. That is three claims wearing one coat, and a harness that asserted
//! the coat would be asserting nothing:
//!
//! | claim | what would break it | what checks it |
//! | --- | --- | --- |
//! | nothing is lost | an operation outstanding at the kill is never answered | [`Ledger`] closes only when every operation is answered, and [`verdict`] compares the count against what was issued |
//! | nothing is answered twice | the dying instance's completion and the new one's both arrive | an operation already answered earns [`wrote::TWICE`], which fails the run |
//! | nothing is answered wrongly | a write the dying instance had taken and not committed is answered as though it had been | the position is read back through a *later instance*, and a mismatch earns [`wrote::WRONG`] |
//!
//! and one more that is not a claim but a bound, because *only latency* with an
//! unbounded tail is a hang with better manners: the worst operation latency is
//! reported, and required to sit under the declared backoff ladder plus what the
//! same run costs with no kill in it.
//!
//! # A place survives its occupant, and the third claim is an ordering
//!
//! RFC 0008 makes an `Endpoint` a **place** rather than an instance, and
//! `kernel/src/component.rs` implements it: clients hold the endpoint, a connect
//! to an empty place pends, and a respawn answers it. [`Place`] is that, as an
//! actor: it keeps its [`ActorId`] across every occupant, so a client addresses
//! the place and never learns which instance answered. Killing is dropping the
//! occupant — which drops its registration table and every translation with it,
//! the modelled half of what `component::tear_down` does — and refilling is
//! constructing a new one under the policy the *manifest* declares.
//!
//! Behind the place is the [`Store`]: the medium, which a driver moves bytes to
//! and from and does not own. A disk's sectors do not die when its driver does.
//!
//! **But a placement is not a claim, and the first draft of this module made one
//! do the work of the other.** A store the kill path never touches produces
//! [`Report::wrong`] of zero whatever the code around it does, which is an alarm
//! wired to a wire no fault can reach — the shape `sim/src/fault.rs` records
//! having been caught at twice already. So what the third claim rests on is an
//! *ordering the code performs*, and the kill lands inside it:
//!
//! - the occupant holds the work it has **accepted and not answered**
//!   ([`Place::accepted`], keyed by the token, carrying the position off the
//!   client's own [`Sqe::offset`] rather than one this file derives);
//! - a write reaches the store **before** its completion is handed to the
//!   client, and never after — that single ordering *is* the claim;
//! - a kill therefore discards writes the dead instance had taken and not
//!   committed, counted as [`wrote::DROPPED`], and [`verdict`] refuses a run
//!   where that count is zero. The read-back's zero is then a statement about
//!   writes that were genuinely interrupted rather than about a workload that
//!   never interrupted one.
//!
//! Two negative controls say the alarm can fire, because a check nobody has
//! watched fail is indistinguishable from one that cannot. [`Chaos::lazy`] makes
//! the occupant answer out of a volatile write-back cache and commit afterwards
//! — the real restart-durability bug, and one a kill loses data through — and
//! [`Chaos::volatile`] puts the whole store inside the occupant, which is a
//! machine whose disk is erased by a segfault. Each has a test requiring the
//! read-back to go wrong and requiring [`verdict`] to name that failure and no
//! other.
//!
//! # The load is the client's and the kill is the seed's
//!
//! [`Load`] keeps `window` operations outstanding until every one of them is
//! answered, so the wire is never empty while the run is going. The kill lands
//! at a moment drawn from the run's own randomness stream, and it lands
//! **mid-flight**: [`Place`] refuses to kill an occupant with nothing
//! outstanding and reschedules instead, writing the outstanding count into the
//! trace beside every kill it does take. A chaos test that killed between
//! operations would be a chaos test of a quiescent system.
//!
//! Everything is a function of `(seed, commit)`, which is the property E1-P03's
//! sweep rests on: no clock is read, the kill instants come from
//! [`World::draw`], and the interleavings come from [`World::decide`] exactly as
//! every other scenario's do.
//!
//! # What a client does when its peer dies, and why that is not *observing*
//!
//! It reclaims its buffers on the evidence RFC 0024 requires — [`PeerGone`], and
//! nothing else — re-registers through the endpoint it already holds, and
//! submits the operations that were never answered. Its registration arrives
//! while the place is empty and **pends**, which is the mechanism gate G1's
//! sentence rests on and is exercised on every kill rather than described.
//!
//! The distinction being drawn is between the *client library* and the *client*.
//! The library sees a peer-gone and a re-registration; the application above it
//! sees an operation that took longer. This module asserts about the second,
//! and the first is in the trace so that a reader can see the difference rather
//! than take it on trust.
//!
//! # What this does not cover, said rather than left to be inferred
//!
//! The occupant is killed by the *place*, at ring 3 in the simulator's sense —
//! there is no ring 3 here, because RFC 0032 put the frame's own instructions in
//! QEMU and this crate above them. So this is the workload half of E1-P06 and
//! `cargo xtask component` is the frame half, and RFC 0041 declares the gap
//! between them as a quantity rather than leaving it as a silence.
//!
//! That quantity has been narrowed once and this paragraph is what it narrowed.
//! It used to read *`kernel/src/blk.rs` still calls `Driver::execute`, so the
//! component that the boot kills is not the component that serves the
//! datapath.* RFC 0047 ended the first half: the driver serves its client from
//! ring 3, in its own polling loop, and the frame calls no part of it. What is
//! left is the second half and it is one word narrower — the driver is
//! *scheduled* and not *spawned into a place*, so the occupant a boot can kill
//! is still not the occupant serving a client's load. `CHAOS_GAP` in xtask is
//! that residue, `cargo xtask chaos` checks it on every run, and it goes red
//! the day it stops being true.

use std::collections::{BTreeMap, VecDeque};

use f_abi::manifest::Record;
use f_abi::{ABI_VERSION, Cqe, Negotiated, Sqe, error};
use f_ring::RingError;
use f_ring::buffers::{BufferSet, Fixed, Idle, InFlight, PeerGone};
use f_ring::registry::registration;

use crate::client::BUFFERS;
use crate::deploy::{Component, Deployment};
use crate::dev::{Config, Device};
use crate::native::Native;
use crate::proto::kind;
use crate::scenario::Peer;
use crate::wire::Post;
use crate::{Actor, ActorId, Message, Outcome, Simulation, Trouble, World};

/// How many messages a chaos run may deliver before it is called stuck.
///
/// The same bound the scenario table uses and for the same reason: it is here to
/// catch a model that loops rather than to bound one that works. A run that
/// reaches it is reported as [`Trouble::Budget`], which is the difference
/// between a bug and a hang — and a client waiting forever for a place that will
/// never be refilled is exactly the failure this harness is looking for, so it
/// must arrive as a refusal and not as a wedged process.
pub const BUDGET: u32 = 1_000_000;

/// Nanoseconds in one timer tick, at the frame's own rate.
///
/// A manifest declares its backoff in milliseconds and `cargo xtask component`
/// compiles it to ticks at 1 kHz — `docs/manifest.md` is where that conversion
/// is stated and `xtask/src/manifest.rs` is where it happens. This is the same
/// conversion in the other direction, so that a pause the frame would take in
/// ticks is a pause this simulator takes in its own nanoseconds.
///
/// Unit: nanoseconds per timer tick.
pub const TICK_NS: u64 = 1_000_000;

/// What one actor says to another, beyond [`crate::proto::kind`].
pub mod kind_chaos {
    /// The place to itself: kill the occupant now.
    ///
    /// A message rather than a call, so that the kill is an event on the
    /// timeline like everything else and lands in whatever interleaving the seed
    /// chose — which is the whole point of killing *at a seeded moment*.
    pub const KILL: &str = "kill";
    /// The place to itself: the backoff has elapsed, put a new occupant in.
    pub const REFILL: &str = "refill";
    /// The place to itself, under [`super::Chaos::lazy`] only: the write-back
    /// cache's contents are now on the medium.
    ///
    /// A message rather than a call for the same reason [`KILL`] is one — it has
    /// to be an event on the timeline, so that a kill can land between the
    /// answer and the flush, which is the whole of the bug that control models.
    pub const FLUSH: &str = "flush";
}

/// What a chaos run writes into the artefact.
///
/// Every one of them is read by [`Report::of`], which is why they are constants
/// rather than literals: an assertion that matched a string a record no longer
/// carries would pass forever.
pub mod wrote {
    /// The place ended its occupant. Detail: operations outstanding at that
    /// instant, which is what makes *mid-flight* a number rather than a hope.
    pub const KILLED: &str = "killed";
    /// The place put a new occupant in. Detail: the epoch it went in at.
    pub const SPAWNED: &str = "spawned";
    /// A submission arrived while the place was empty and is waiting. Detail:
    /// how many are waiting.
    pub const PENDED: &str = "pended";
    /// The refill rang the doorbells the pending submissions were owed.
    /// Detail: how many.
    pub const RESUMED: &str = "resumed";
    /// The restart budget ran out and the place will not be refilled.
    pub const RETIRED: &str = "retired";
    /// Entries the dying occupant never took, discarded at the kill. Detail:
    /// how many. They are not lost — the client's ledger still owes them — and
    /// this is what says so out loud.
    pub const VOIDED: &str = "voided";
    /// An operation was answered for the first time. Detail: how long it took,
    /// in nanoseconds, from its first submission.
    pub const SETTLED: &str = "settled";
    /// An operation was answered a second time. **A failure.**
    pub const TWICE: &str = "twice";
    /// A completion arrived for a token the client does not hold. **A failure.**
    pub const STALE: &str = "stale";
    /// A read did not return what was written. **A failure**, and the third of
    /// the three sentences gate G1's is decomposed into.
    pub const WRONG: &str = "wrong";
    /// A buffer came back bearing another operation's stamp. **A failure**, and
    /// separated from [`WRONG`] rather than folded into it because the two are
    /// not the same finding and one of them has no way to fire today: RFC 0041
    /// concedes that no device model in this crate can reach a client's bytes,
    /// so this is a structural check kept for the milestone that adds one, and
    /// it is counted under its own name so that `operations_answered_wrongly` is
    /// a number about durability and nothing else. `claims/0005` says the same
    /// beside the metric rather than leaving a reader to find it here.
    pub const TORN: &str = "torn";
    /// Writes the dying occupant had accepted and not yet committed, discarded
    /// with it. Detail: how many.
    ///
    /// **Not a failure — the load-bearing positive.** Each of these is a write
    /// interrupted between the medium and the answer, which is the state a
    /// restart is dangerous in; the client owes it again and the read-back is
    /// what says the value that finally landed is the right one. A run where
    /// this is zero is a run whose kills never touched the durability path, and
    /// [`verdict`] refuses it for the same reason it refuses a kill that landed
    /// with nothing in flight.
    pub const DROPPED: &str = "dropped";
    /// The client was refused for a reason it cannot retry. **A failure**, and
    /// the one gate G1's sentence is most directly about.
    pub const FAILED: &str = "failed";
    /// The client answered every operation it issued and stopped.
    pub const CLOSED: &str = "closed";
}

/// What the place is called in the trace. At most [`crate::LABEL_WIDTH`] bytes.
pub const PLACE: &str = "place";

/// What the client is called in the trace.
pub const LOAD: &str = "load";

// ---------------------------------------------------------------- the policy

/// The restart policy a component declares, as this harness applies it.
///
/// Read out of a compiled manifest record and never written here, which is the
/// join this task inherits from RFC 0035: the pause the simulator takes between
/// a kill and a refill is the pause `user/virtio-blk/manifest.toml` declares,
/// and a manifest that changed it changes this run.
///
/// # The one rule that is in two places, named rather than hidden
///
/// [`Record::backoff_ticks`] and [`Record::restarts_after`] are `f_abi`'s, so
/// the ladder and RFC 0008's policy table are shared rather than copied. What is
/// written twice is the four lines that reset the window and compare the count:
/// here, and in `kernel::component::policy::decide`, which was deliberately
/// written to take a record and a tally and no kernel state so that moving it is
/// a move. *Reversal:* lift `decide` into `f_abi::manifest`, at which point both
/// readers call it and this paragraph goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy {
    /// The pause before the first respawn. Unit: timer ticks.
    pub backoff_first_ticks: u32,
    /// The cap the pause doubles up to. Unit: timer ticks.
    pub backoff_max_ticks: u32,
    /// Respawns inside the window before the place is retired. Unit: restarts.
    pub max_restarts: u32,
    /// The window that count is taken over. Unit: timer ticks.
    pub budget_window_ticks: u32,
    /// RFC 0008's policy, as an `f_abi::manifest::restart` constant.
    pub restart: u8,
}

impl Policy {
    /// The policy a compiled record declares.
    #[must_use]
    pub const fn of(record: &Record) -> Self {
        Self {
            backoff_first_ticks: record.backoff_first_ticks,
            backoff_max_ticks: record.backoff_max_ticks,
            max_restarts: record.max_restarts,
            budget_window_ticks: record.budget_window_ticks,
            restart: record.restart,
        }
    }

    /// The record this policy would be read out of, for the arithmetic in
    /// `f_abi` to be applied to.
    ///
    /// A record and not a copy of the ladder, because [`Record::backoff_ticks`]
    /// is where the doubling and its saturation live and a second
    /// implementation of a saturating shift is a second place to get it wrong.
    fn as_record(self) -> Record {
        Record {
            backoff_first_ticks: self.backoff_first_ticks,
            backoff_max_ticks: self.backoff_max_ticks,
            max_restarts: self.max_restarts,
            budget_window_ticks: self.budget_window_ticks,
            restart: self.restart,
            ..Record::EMPTY
        }
    }

    /// The pause before the `nth` respawn. Unit: nanoseconds.
    #[must_use]
    pub fn backoff_ns(self, nth: u32) -> u64 {
        u64::from(self.as_record().backoff_ticks(nth)).saturating_mul(TICK_NS)
    }

    /// Every pause `kills` restarts would take, added up. Unit: nanoseconds.
    ///
    /// The bound the latency claim is stated against: an operation caught by
    /// every kill in the run waits out every backoff in the ladder, and nothing
    /// in this design makes it wait longer than that plus the work itself.
    #[must_use]
    pub fn ladder_ns(self, kills: u32) -> u64 {
        (0..kills).fold(0u64, |sum, nth| sum.saturating_add(self.backoff_ns(nth)))
    }

    /// What the supervisor does about an occupant that has ended, and how long
    /// it waits.
    ///
    /// `now` is the run's own clock in nanoseconds; the window is in ticks,
    /// which is why the comparison converts rather than assuming.
    fn decide(self, budget: &mut Budget, now: u64) -> Verdict {
        let record = self.as_record();
        // Every kill in this harness is a fault: the place ends its occupant
        // because the seed said to, which is the case RFC 0008 calls a fault and
        // not the case it calls an exit. A policy of `never` therefore leaves
        // the place empty, which is a legitimate declaration and a run that
        // would then wedge — reported as `Trouble::Budget` rather than hidden.
        if !record.restarts_after(true, false) {
            return Verdict::Leave;
        }
        let window = u64::from(self.budget_window_ticks).saturating_mul(TICK_NS);
        if now.saturating_sub(budget.opened_ns) >= window {
            budget.used = 0;
            budget.opened_ns = now;
        }
        if budget.used >= self.max_restarts {
            return Verdict::Retire;
        }
        let pause = self.backoff_ns(budget.used);
        budget.used += 1;
        Verdict::Restart(pause)
    }
}

/// Restarts inside the current window, and when the window opened.
#[derive(Clone, Copy, Debug, Default)]
struct Budget {
    used: u32,
    opened_ns: u64,
}

/// What the supervisor does next about a place whose occupant has ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// Leave the place empty. The policy says so.
    Leave,
    /// Spawn again, after this many nanoseconds.
    Restart(u64),
    /// The budget ran out.
    Retire,
}

// ----------------------------------------------------------------- the store

/// The state behind a component, which its death does not touch.
///
/// A sector to the value last written there. Held by the [`Place`] rather than
/// by the occupant, because that is where it is: a disk is behind its driver and
/// an object store is behind the component that serves it, and a model that put
/// either inside the instance would be modelling a machine whose disk is erased
/// by a driver bug.
///
/// `BTreeMap` for RFC 0004's reason, which is the same one every other map in
/// this crate is one for.
type Store = BTreeMap<u64, u64>;

/// One operation the current occupant has taken and not yet answered.
///
/// Held by the [`Place`] because the place is what forwards the entry, and
/// discarded whole at a kill: an occupant that dies owes nothing, and every
/// token in here is one the client's ledger still owes and will submit again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Accepted {
    /// Where on the medium, taken off the client's own [`Sqe::offset`].
    ///
    /// Off the wire and not derived from the token, and the difference matters:
    /// a position this file computed from the completion would agree with the
    /// one it computed from the submission by construction, and the read-back
    /// would be a check that two calls to one function return the same thing.
    /// Unit: per-peer — sectors for a disk, requests for anything else.
    at: u64,
    /// Whether this operation puts a value there.
    write: bool,
}

// ----------------------------------------------------------------- the place

/// One place in the topology: an address that outlives its occupants.
///
/// Everything a client addresses is here. The occupant is behind it, is
/// replaced, and is never named by anybody outside — which is what
/// `component.rs` means by *a place is a manifest's slot and an instance is
/// whichever component currently occupies it*, and is why the client's
/// [`ActorId`] for its peer never changes across a restart.
pub struct Place {
    /// How to build an occupant. A function rather than a generic parameter
    /// because a place is a slot for *a component*, and which model goes in it
    /// is the deployment's answer rather than a type this file names.
    spawn: fn(Config) -> Box<dyn Actor>,
    cfg: Config,
    policy: Policy,
    occupant: Option<Box<dyn Actor>>,
    /// Who holds the endpoint. One, because a virtqueue has exactly one driver
    /// and this place has one behind it.
    client: Option<ActorId>,
    /// Which occupant this is. Unit: instances, counting from zero — the same
    /// number `ChannelHeader::epoch` carries on the real path.
    epoch: u32,
    budget: Budget,
    retired: bool,
    /// Work forwarded to the current occupant and not yet answered.
    ///
    /// What makes *mid-flight* a number: a kill with this empty is a kill
    /// between operations, which is a weaker experiment wearing this one's name.
    /// The writes in it are what makes it a number about *durability* as well —
    /// see [`wrote::DROPPED`].
    accepted: BTreeMap<u64, Accepted>,
    /// The medium behind the component, when there is any.
    store: Option<Store>,
    /// The occupant's own write-back cache: values answered to the client and
    /// not yet on the medium.
    ///
    /// Empty on every correct run, because a write reaches the store before its
    /// completion is handed on. Non-empty only under [`Place::lazy`], and
    /// cleared by a kill — which is the bug that control exists to model.
    cache: Store,
    /// Whether the occupant answers a write before committing it.
    ///
    /// The negative control for the ordering the third claim *is*; see
    /// [`Chaos::lazy`].
    lazy: bool,
    /// Whether the medium itself dies with the occupant.
    ///
    /// `false` is the world: a disk survives its driver. `true` is the coarser
    /// of the two durability controls — see [`Chaos::volatile`], which is where
    /// it is set, and `tests::a_store_that_dies_with_its_driver_is_caught_by_the_read_back`
    /// for what it buys.
    volatile: bool,
    /// Whether the dead instance's work is answered after the refill. The
    /// negative control for *answered twice*; see [`Chaos::leaky`].
    leaky: bool,
    /// Tokens the dead instance held, kept only under [`Place::leaky`] so that
    /// its answers can arrive after its client has reclaimed them.
    leaked: Vec<u64>,
    /// Kills left in the plan. Unit: kills.
    kills_left: u32,
    /// Kills taken. Unit: kills.
    kills: u32,
    /// Occupants put in, the first one included. Unit: instances.
    spawns: u32,
    /// How many times a kill was deferred because nothing was in flight.
    /// Unit: deferrals. Bounded, so a run whose client has finished cannot
    /// reschedule for ever.
    deferrals: u32,
    /// The shortest gap between a spawn and the kill that follows it.
    /// Unit: nanoseconds.
    settle_ns: u64,
}

/// How many times a kill may be put off for want of anything in flight.
///
/// Sixteen. A kill that finds an idle occupant is not the experiment, so it is
/// rescheduled rather than taken — but a client that has finished its work will
/// never be busy again, and a scheduler that kept trying would turn a finished
/// run into [`Trouble::Budget`]. Past this the kill is abandoned, the plan is
/// short by one, and [`verdict`] fails the run for it: a chaos test that quietly
/// took fewer kills than it planned is a chaos test reporting on a smaller
/// experiment than the one it names.
const DEFERRALS_MAX: u32 = 16;

/// How long a write sits in the write-back cache before it reaches the medium,
/// as a multiple of [`Chaos::settle_ns`].
///
/// Four, and it is a negative control's parameter rather than a model of any
/// real cache: it has to be long enough that a kill reliably lands while a
/// client-acknowledged write is still only in memory, at every seed, or the
/// control would fire at some interleavings and not others — which is an alarm a
/// sweep would find and a suite would not. Unit: settle intervals.
const FLUSH_SETTLES: u64 = 4;

impl Place {
    /// A place with its first occupant not yet in it.
    ///
    /// It takes the whole [`Chaos`] rather than the six numbers it reads out of
    /// it, because those six are the run's own description and a constructor
    /// with them spelled out is a second place they can drift.
    #[must_use]
    pub fn new(spawn: fn(Config) -> Box<dyn Actor>, cfg: Config, chaos: &Chaos) -> Self {
        Self {
            spawn,
            cfg,
            policy: chaos.policy,
            occupant: None,
            client: None,
            epoch: 0,
            budget: Budget::default(),
            retired: false,
            accepted: BTreeMap::new(),
            store: chaos.durable.then(Store::new),
            cache: Store::new(),
            lazy: chaos.lazy,
            volatile: chaos.volatile,
            leaky: chaos.leaky,
            leaked: Vec::new(),
            kills_left: chaos.kills,
            kills: 0,
            spawns: 0,
            deferrals: 0,
            settle_ns: chaos.settle_ns(),
        }
    }

    /// Put an occupant in, answer whatever pended while the place was empty, and
    /// start the clock on the next kill.
    ///
    /// One function for the first occupant and every one after it, because RFC
    /// 0008 makes them the same act: a spawn into a place answers the connects
    /// that place owes, and a first spawn that skipped that would leave the very
    /// first submission unanswered whenever the seed happened to run the client
    /// before the place.
    fn fill(&mut self, world: &mut World, me: ActorId) {
        self.occupant = Some((self.spawn)(self.cfg));
        self.spawns = self.spawns.saturating_add(1);
        if self.volatile
            && let Some(store) = self.store.as_mut()
        {
            // The negative control: the state behind the place was the
            // occupant's after all, so it goes with it.
            store.clear();
        }
        world.record(me, PLACE, wrote::SPAWNED, u64::from(self.epoch), u64::from(self.spawns));

        // The connect that pended is answered by the refill, which is the first
        // of its three outcomes and the one gate G1's sentence rests on. The
        // entry never moved: it has been sitting in the shared region the whole
        // time, and what was deferred is the doorbell.
        if let Some(client) = self.client {
            let waiting = world.wire().queued(client, me);
            for _ in 0..waiting {
                world.send(
                    0,
                    me,
                    Message { from: client, kind: kind::SUBMIT, token: 0, detail: 0 },
                );
            }
            if waiting > 0 {
                world.record(me, PLACE, wrote::RESUMED, u64::from(self.epoch), u64::from(waiting));
            }
        }
        self.arm(world, me);
    }

    /// Schedule the next kill, if the plan still has one.
    ///
    /// The instant comes from the run's randomness stream rather than from a
    /// decision site: *how long* is a quantity and not a choice between
    /// alternatives, which is the split [`World::draw`] and [`World::decide`]
    /// are two functions for.
    fn arm(&mut self, world: &mut World, me: ActorId) {
        if self.kills_left == 0 {
            return;
        }
        let spread = self.settle_ns.max(1);
        let delay = self.settle_ns.saturating_add(world.draw() % spread);
        world.send(delay, me, Message { from: me, kind: kind_chaos::KILL, token: 0, detail: 0 });
    }

    /// End the occupant, if there is anything in flight to end it in the middle
    /// of.
    ///
    /// `forced` is set when the occupant ended itself rather than being ended —
    /// a device that lost work and reset, which is `E1-P02`'s territory and
    /// which this harness arms nothing to produce. It is handled rather than
    /// ignored because R04 says an event this build does not expect is refused
    /// and not dropped: the place refills, and the kill count goes over the plan,
    /// and [`verdict`] fails the run naming the discrepancy.
    fn kill(&mut self, world: &mut World, me: ActorId, forced: bool) {
        if self.occupant.is_none() || self.retired {
            return;
        }
        if self.accepted.is_empty() && !forced {
            // Not the experiment. A kill that finds an idle occupant is a kill
            // between operations, and E1-P06's exit is about one that lands in
            // the middle of them.
            if self.deferrals < DEFERRALS_MAX {
                self.deferrals = self.deferrals.saturating_add(1);
                world.send(
                    self.settle_ns.max(1),
                    me,
                    Message { from: me, kind: kind_chaos::KILL, token: 0, detail: 0 },
                );
            }
            return;
        }

        let flying = u64::try_from(self.accepted.len()).unwrap_or(u64::MAX);
        // Writes the dead instance had taken and not committed. Recorded before
        // the state goes, because after it there is nothing left to count — and
        // a run where this is zero across every kill is a run whose kills never
        // landed on the durability path at all.
        let dropped = u64::try_from(self.accepted.values().filter(|taken| taken.write).count())
            .unwrap_or(u64::MAX);
        // Dropping the occupant is the teardown: its registration table goes,
        // and every translation in its modelled domain goes with it, which is
        // what makes a transfer the dead instance had started fault rather than
        // land in memory the client is about to reuse. RFC 0008 step one and
        // step two, in one line, because in this model they are one object.
        self.occupant = None;
        if self.leaky {
            // The negative control: the work the dead instance held is kept, so
            // that the refill can answer it after the client has reclaimed the
            // buffers it names. On the real path this is what the frame's
            // teardown makes impossible — the translations go with the
            // registration, so a transfer the dead instance had started faults
            // rather than landing.
            self.leaked.extend(self.accepted.keys().copied());
        }
        self.accepted.clear();
        // The occupant's volatile cache dies with the occupant, which is what a
        // write-back cache in a driver *is*. On a correct run it is empty, and
        // that emptiness is the third claim: a write was on the medium before
        // its client was told so.
        self.cache.clear();
        self.kills = self.kills.saturating_add(1);
        self.kills_left = self.kills_left.saturating_sub(1);
        world.record(me, PLACE, wrote::KILLED, u64::from(self.epoch), flying);
        world.record(me, PLACE, wrote::DROPPED, u64::from(self.epoch), dropped);

        // Entries the dead instance never took. Discarded here rather than left
        // to be re-rung at the refill, because they name a buffer set the next
        // occupant never issued: a client that had them served would be a client
        // whose registration outlived the component that granted it. The
        // client's ledger still owes every one of them, which is what makes this
        // a delay rather than a loss.
        let mut voided = 0u64;
        if let Some(client) = self.client {
            while world.wire().take(client, me).is_some() {
                voided += 1;
            }
        }
        while world.wire().take(me, me).is_some() {
            voided += 1;
        }
        // And the answers it had already published but not handed on. A
        // completion the dying instance produced names a token the client is
        // about to reclaim on the evidence that the instance is gone, so passing
        // it along afterwards would be two owners of one buffer with one of them
        // a device — the failure `PeerGone` is sound only because the frame
        // prevents it, and the failure `wrote::STALE` exists to catch.
        while world.wire().reap(me, me).is_some() {
            voided += 1;
        }
        world.record(me, PLACE, wrote::VOIDED, u64::from(self.epoch), voided);

        self.epoch = self.epoch.saturating_add(1);

        // The supervisor's act, under the policy the manifest declares.
        let now = world.clock();
        match self.policy.decide(&mut self.budget, now) {
            Verdict::Restart(pause) => {
                world.send(
                    pause,
                    me,
                    Message { from: me, kind: kind_chaos::REFILL, token: 0, detail: pause },
                );
            }
            Verdict::Leave | Verdict::Retire => {
                self.retired = true;
                world.record(me, PLACE, wrote::RETIRED, u64::from(self.epoch), 0);
            }
        }

        // Peer-gone, to every holder of an endpoint to this place. It is the one
        // event that lets a client take a buffer back with no completion, and a
        // place that killed quietly would leave its client holding memory it can
        // never touch and never free — a hang with a clean trace.
        if let Some(client) = self.client {
            world.send(0, client, Message { from: me, kind: kind::GONE, token: 0, detail: 0 });
        }
    }

    /// The backoff has elapsed: put a new occupant in.
    fn refill(&mut self, world: &mut World, me: ActorId) {
        if self.retired || self.occupant.is_some() {
            return;
        }
        self.fill(world, me);
        // The negative control, and nothing on a correct run reaches this: the
        // dead instance's transfers landing late, answered across a restart that
        // has already told the client every one of its tokens is void.
        if let Some(client) = self.client {
            let now = world.clock();
            for token in core::mem::take(&mut self.leaked) {
                world.wire().answer(me, client, f_ring::completion(token, 0, now));
                world.send(0, client, Message { from: me, kind: kind::CQE, token, detail: 0 });
            }
        }
    }

    /// A submission from the client: move it onto the occupant's channel and
    /// ring.
    fn forward(&mut self, world: &mut World, me: ActorId, from: ActorId) {
        // The first client to submit is the endpoint's holder, and a second one
        // is refused rather than served. R04, and not a formality: a place that
        // adopted whoever spoke last would send its peer-gone to that one, so
        // the first client would be left holding buffers it can never reclaim —
        // a hang, produced by a fail-open in three lines nobody would look at.
        // `Device::submit` refuses a second driver one layer down and this is
        // the same refusal at the layer that owns the endpoint.
        if *self.client.get_or_insert(from) != from {
            world.record(me, PLACE, wrote::VOIDED, u64::from(from.0), NOTHING);
            return;
        }
        if self.occupant.is_none() {
            // RFC 0008: a connect on an empty place pends. Nothing is refused
            // and nothing is dropped — the entry stays where the client put it,
            // and the refill rings for it.
            let waiting = world.wire().queued(from, me);
            world.record(me, PLACE, wrote::PENDED, u64::from(self.epoch), u64::from(waiting));
            return;
        }
        let Some(entry) = world.wire().take(from, me) else {
            // A doorbell with nothing behind it. Ordinary on a real ring, and
            // the refill above rings one per queued entry, so a spare is the
            // expected shape rather than an error.
            return;
        };
        let token = entry.user_data;
        // The position comes off the entry the client built, not out of the
        // token: what the store is keyed by has to be what the client asked for,
        // or the read-back compares one derivation against another.
        self.accepted
            .insert(token, Accepted { at: entry.offset, write: phase(token) == half::WRITE });
        world.wire().post(me, me, entry);
        self.deliver_down(world, me, Message { from: me, kind: kind::SUBMIT, token, detail: 0 });
    }

    /// A completion from the occupant: apply the state behind the place, and
    /// hand it to the client.
    fn answer(&mut self, world: &mut World, me: ActorId) {
        let Some(mut cqe) = world.wire().reap(me, me) else {
            return;
        };
        let token = cqe.user_data;
        // Taken off the occupant's books here and nowhere else. What is in this
        // entry is what the *client* asked for, carried since the submission,
        // and a completion naming a token that is not in it is one the occupant
        // was never handed — which the client catches as `wrote::STALE`.
        let taken = self.accepted.remove(&token);

        // The medium, which is what a driver moves bytes to and from rather than
        // what a driver holds. A write commits when the operation is answered
        // and not when it is submitted, so a driver that died before answering
        // committed nothing and the client's re-submission is the one that
        // commits — which is what makes a re-submission safe rather than a
        // second write.
        //
        // **The ordering below is the third claim**, not the placement of the
        // map: the value is on the medium before the completion leaves this
        // function, and there is no path here that answers first. `lazy` is the
        // path that does, and it is a negative control rather than an option.
        let mut flush_due = false;
        if let Some(entry) = taken
            && self.store.is_some()
            && cqe.result >= 0
        {
            let at = entry.at;
            if entry.write {
                let put = value(op(token));
                if self.lazy {
                    self.cache.insert(at, put);
                    flush_due = true;
                } else if let Some(store) = self.store.as_mut() {
                    store.insert(at, put);
                }
            } else if phase(token) == half::READ {
                // What is actually there, in the completion's own per-opcode
                // payload. `Cqe::ext` is defined as operation-specific on a
                // success, which is exactly what this is; a read that found
                // nothing answers `NOTHING`, which no write can produce.
                //
                // The cache first, because a write-back cache is readable — a
                // driver answers out of it, which is exactly why losing it is a
                // durability bug and not a visible one until the driver dies.
                // On a correct run it is empty and this reads the medium.
                cqe.ext = self
                    .cache
                    .get(&at)
                    .or_else(|| self.store.as_ref().and_then(|store| store.get(&at)))
                    .copied()
                    .unwrap_or(NOTHING);
            }
        }
        if flush_due {
            world.send(
                self.settle_ns.saturating_mul(FLUSH_SETTLES).max(1),
                me,
                Message { from: me, kind: kind_chaos::FLUSH, token: 0, detail: 0 },
            );
        }

        let Some(client) = self.client else {
            return;
        };
        world.wire().answer(me, client, cqe);
        world.send(0, client, Message { from: me, kind: kind::CQE, token, detail: 0 });
    }

    /// The write-back cache reaches the medium.
    ///
    /// Scheduled only under [`Place::lazy`], and on the correct path there is no
    /// cache and therefore no flush — which is the point. What makes this a
    /// model of the bug rather than a delay is that [`Place::kill`] clears the
    /// cache: a client told its write succeeded, an instance that died before
    /// the flush, and a medium that never heard about it.
    fn flush(&mut self) {
        if let Some(store) = self.store.as_mut() {
            store.append(&mut self.cache);
        }
        self.cache.clear();
    }

    /// Hand a message to the occupant, if there is one.
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
            kind_chaos::KILL => self.kill(world, me, false),
            kind_chaos::REFILL => self.refill(world, me),
            kind_chaos::FLUSH => self.flush(),
            // From the client: a submission for the occupant.
            kind::SUBMIT if message.from != me => self.forward(world, me, message.from),
            // From the occupant: a completion for the client. The occupant
            // believes this place is its client, which is what makes the place
            // an address rather than a proxy the client can see.
            kind::CQE if message.from == me => self.answer(world, me),
            // The occupant reset itself — a lost completion, or one of E1-P02's
            // fault classes. Treated as a death, because it is one: RFC 0008
            // says what happens to a component that has stopped speaking, and
            // the place is what does it.
            kind::GONE if message.from == me => self.kill(world, me, true),
            // The occupant talking to itself: the doorbell it rang, the service
            // time that elapsed, the used ring it harvests. Forwarded unchanged,
            // and dropped when the occupant it belonged to is gone — a timer
            // for a dead instance is not an event the next one is owed.
            _ => self.deliver_down(world, me, message),
        }
    }
}

// ---------------------------------------------------------------- the ledger

/// Which half of a logical operation a token names.
///
/// A module of constants rather than an enum because the value travels inside a
/// token, and a token is a `u64` a client mints and a place decodes.
pub mod half {
    /// Put a value at a position.
    pub const WRITE: u8 = 0;
    /// Read the position back, and require the value.
    pub const READ: u8 = 1;
    /// Not an operation: the buffer-set registration.
    pub const REGISTER: u8 = 0xFF;
}

/// The answer a read gets at a position nothing has written.
///
/// Not a value [`value`] can produce, which is what makes *nothing is there* a
/// different observation from *the wrong thing is there*. The same discipline
/// `blk.rs` applies to its status byte with `0xFF`.
const NOTHING: u64 = u64::MAX;

/// The value written at logical operation `op`.
///
/// Derived rather than drawn: this is a fixture, and RFC 0004's substrate is for
/// the things that have to *vary* reproducibly. Never [`NOTHING`] and never
/// zero, so that a read answering either is unambiguous.
#[must_use]
pub const fn value(op: u32) -> u64 {
    let mixed = (op as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    if mixed == NOTHING { 1 } else { mixed }
}

/// The token for one client's `op`th operation in one phase.
const fn token(who: u32, phase: u8, op: u32) -> u64 {
    ((who as u64) << 56) | ((phase as u64) << 48) | op as u64
}

/// The logical operation a token names.
#[must_use]
pub const fn op(token: u64) -> u32 {
    (token & 0xFFFF_FFFF) as u32
}

/// The phase a token names.
#[must_use]
pub const fn phase(token: u64) -> u8 {
    ((token >> 48) & 0xFF) as u8
}

/// The position a logical operation works at.
///
/// The operation's own number, so that a re-submission after a restart works at
/// the same position as the attempt that was killed — which is what a driver
/// does, and what makes the read-back a statement about the *value* rather than
/// about where it landed. The block device reads at even positions and writes at
/// odd ones, which is its own business with its device and not this harness's:
/// what crosses the place is a position and a payload either way.
///
/// Unit: per-peer, and the peer that reads it is what states which — a sector
/// for a disk, a request for anything else.
#[must_use]
pub const fn position(token: u64) -> u64 {
    op(token) as u64
}

/// One logical operation, as the client's ledger holds it.
///
/// Two of everything, because a logical operation is two ring operations and a
/// latency measured from the wrong one would be a latency measured over a wait
/// that had already been answered. The write's clock starts when the write is
/// first submitted and the read's when the read is.
#[derive(Clone, Copy, Debug, Default)]
struct Entry {
    /// Whether each half has been answered.
    answered: [bool; 2],
    /// When each half was first submitted. `None` until it has been.
    /// Unit: nanoseconds.
    first_ns: [Option<u64>; 2],
}

/// The client's ledger: one entry per logical operation, keyed by the operation
/// and not by the submission.
///
/// **That key is the whole of *answered twice*.** A re-submission after a kill is
/// one operation submitted twice, and a system that answered both would be
/// serving work on either side of a restart; a ledger keyed by submission could
/// not tell that from two operations. It is the same distinction the exit
/// criterion draws between the client library — which reconnects and re-submits
/// — and the application above it, which is what *observes* anything.
///
/// The counts [`verdict`] reads come out of the *trace* rather than out of this
/// structure, because the trace is what a reproduction hands to a person and a
/// number that existed only in memory would be a number a failing seed could not
/// report.
#[derive(Debug, Default)]
struct Ledger {
    entries: BTreeMap<u32, Entry>,
}

// ---------------------------------------------------------------- the client

/// A client that keeps a window of work in flight, survives its peer, and
/// checks every answer.
///
/// It is not [`crate::client::App`], and the difference is the whole task: `App`
/// treats a peer-gone as the end of its run, because at E1-P02 a dead peer was
/// the end of the story. This one reconnects through the endpoint it already
/// holds and re-submits what was never answered, which is what a client does
/// when the thing that died was a *place's occupant* rather than the place.
pub struct Load {
    who: u32,
    place: ActorId,
    window: u32,
    operations: u32,
    retry_ns: u64,
    buffer_bytes: u32,
    depth: u32,
    /// Whether this peer has state behind it, and therefore a read phase.
    reads_back: bool,

    idle: Vec<Idle<'static, Fixed>>,
    flight: Vec<InFlight<'static, Fixed>>,
    /// Tokens waiting to be issued, in the order they became due.
    queue: VecDeque<u64>,
    ledger: Ledger,
    /// Answered phases, across every operation. Unit: phases.
    settled: u32,
    /// How many phases this run owes altogether. Unit: phases.
    owed: u32,
    registered: bool,
    closed: bool,
}

impl Load {
    /// A client that will drive `operations` logical operations at `place`.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "every one is a workload parameter the chaos configuration states, and a struct \
                  of them would be a type that exists so that a lint passes — the argument \
                  `client::App` already makes one module over"
    )]
    pub fn new(
        who: u32,
        place: ActorId,
        window: u32,
        operations: u32,
        buffer_bytes: u32,
        retry_ns: u64,
        depth: u32,
        reads_back: bool,
    ) -> Self {
        let phases = if reads_back { 2 } else { 1 };
        Self {
            who,
            place,
            window: window.clamp(1, BUFFERS as u32),
            operations,
            retry_ns,
            buffer_bytes: buffer_bytes.max(1),
            depth: depth.max(1),
            reads_back,
            idle: Vec::new(),
            flight: Vec::new(),
            queue: VecDeque::new(),
            ledger: Ledger::default(),
            settled: 0,
            owed: operations.saturating_mul(phases),
            registered: false,
            closed: false,
        }
    }

    /// Ask the place for a buffer set.
    ///
    /// Submitted the same way after a kill as before one, and that is the point:
    /// a client reconnects through the endpoint it already holds, and the
    /// registration that arrives while the place is empty pends until a respawn
    /// refills it. Nothing here knows whether the place is occupied.
    fn register(&mut self, world: &mut World, me: ActorId) {
        let token = token(self.who, half::REGISTER, 0);
        let len = self.buffer_bytes.saturating_mul(BUFFERS as u32);
        let entry = registration(token, 0, len, BUFFERS as u32);
        world.wire().post(me, self.place, entry);
        world.record(me, LOAD, crate::proto::wrote::REGISTER, token, u64::from(len));
        world.send(0, self.place, Message { from: me, kind: kind::SUBMIT, token, detail: 0 });
    }

    /// Bind the set the occupant issued, and carve it.
    ///
    /// A new region every time, leaked at `'static` for the reason
    /// `client::App` states: a component's buffer region is granted for the life
    /// of the component and never handed back, and here a component's life ends
    /// at every kill. The leak is therefore the model being faithful rather than
    /// the model being careless — RFC 0034 argues it.
    fn bind(&mut self, world: &mut World, me: ActorId, cqe: &Cqe) {
        let Ok(naming) = Fixed::from_completion(cqe) else {
            // A refused registration is not a loss: the place may be empty or
            // the new occupant may still be settling, and a client that gave up
            // here would be a client that turned a restart into an outage.
            world.record(me, LOAD, crate::proto::wrote::REFUSED, cqe.user_data, cqe.ext);
            world.send(
                self.retry_ns,
                me,
                Message { from: me, kind: kind::RETRY, token: cqe.user_data, detail: 0 },
            );
            return;
        };
        let region: &'static mut [u8] =
            Box::leak(vec![0u8; self.buffer_bytes as usize * BUFFERS].into_boxed_slice());
        let agreed = Negotiated { version: ABI_VERSION, features: 0 };
        let Ok(set) = BufferSet::bind(naming, agreed, region) else {
            world.record(me, LOAD, wrote::FAILED, cqe.user_data, NOTHING);
            return;
        };
        let set: &'static mut BufferSet<'static, Fixed> = Box::leak(Box::new(set));
        let Ok(buffers) = set.carve::<BUFFERS>() else {
            world.record(me, LOAD, wrote::FAILED, cqe.user_data, NOTHING);
            return;
        };
        self.idle.clear();
        self.idle.extend(buffers);
        self.registered = true;
        world.record(
            me,
            LOAD,
            crate::proto::wrote::BOUND,
            cqe.user_data,
            u64::from(naming.set().bits()),
        );
        self.pump(world, me);
    }

    /// Issue as many operations as the window and the set allow.
    fn pump(&mut self, world: &mut World, me: ActorId) {
        if !self.registered {
            return;
        }
        while u32::try_from(self.flight.len()).unwrap_or(u32::MAX) < self.window {
            let Some(token) = self.next_token() else { break };
            let Some(mut buffer) = self.idle.pop() else {
                self.queue.push_front(token);
                break;
            };

            // Written before the submission and read after the completion: the
            // one place this client touches its own bytes, and what says a
            // buffer came back from the place it went to rather than merely came
            // back. `InFlight` has no method that reaches them, which is the
            // whole of RFC 0024 and the reason this is worth driving.
            if let Some(first) = buffer.bytes_mut().first_mut() {
                *first = stamp(token);
            }

            let now = world.clock();
            let slot = usize::from(phase(token) == half::READ);
            let entry = self.ledger.entries.entry(op(token)).or_default();
            if let Some(began) = entry.first_ns.get_mut(slot) {
                // The *first* submission, kept across every re-submission a kill
                // produces. A clock restarted by a retry would report the
                // latency of the last attempt, which is precisely the number a
                // client does not experience.
                //
                // How many attempts there were is deliberately not held here.
                // The artefact already answers it — one `reclaim` record per
                // buffer taken back at a kill, which is exactly the work a
                // restart made this client do again — and a second count in
                // memory would be a number a failing seed could not report.
                began.get_or_insert(now);
            }

            let sqe = Sqe {
                user_data: token,
                len: self.buffer_bytes,
                offset: position(token),
                ..Sqe::ZERO
            };
            let mut post = Post::new(world, me, self.place, self.depth);
            match buffer.submit(&mut post, sqe) {
                Ok((lent, _rang)) => {
                    self.flight.push(lent);
                    world.record(
                        me,
                        LOAD,
                        crate::proto::wrote::ISSUE,
                        token,
                        u64::try_from(self.flight.len()).unwrap_or(u64::MAX),
                    );
                    world.send(
                        0,
                        self.place,
                        Message { from: me, kind: kind::SUBMIT, token, detail: 0 },
                    );
                }
                Err((_refused, back)) => {
                    // A full ring is a retry and not a loss, and every other
                    // refusal from the ownership types is this client having
                    // built an entry they refuse — which cannot happen here,
                    // because the length is the buffer's own. Both come back the
                    // same way: the buffer is returned and the token is owed.
                    self.idle.push(back);
                    self.queue.push_front(token);
                    world.record(me, LOAD, crate::proto::wrote::FULL, token, u64::from(self.depth));
                    world.send(
                        self.retry_ns,
                        me,
                        Message { from: me, kind: kind::RETRY, token, detail: 0 },
                    );
                    return;
                }
            }
        }
        self.close_if_done(world, me);
    }

    /// The next token to issue: work owed first, then work not yet begun.
    ///
    /// Owed first, because a client that minted new tokens while old ones waited
    /// would grow its backlog under exactly the pressure a restart produces —
    /// `client::App` makes the same argument about a refusal and it is the same
    /// argument here.
    fn next_token(&mut self) -> Option<u64> {
        if let Some(again) = self.queue.pop_front() {
            return Some(again);
        }
        let fresh =
            (0..self.operations).find(|number| !self.ledger.entries.contains_key(number))?;
        Some(token(self.who, half::WRITE, fresh))
    }

    /// Match a completion against the buffers this client has out, and judge it.
    fn reap(&mut self, world: &mut World, me: ActorId, cqe: &Cqe) {
        let mut rest = Vec::with_capacity(self.flight.len());
        let mut returned = None;
        for lent in self.flight.drain(..) {
            if returned.is_some() {
                rest.push(lent);
                continue;
            }
            match lent.complete(cqe) {
                Ok(idle) => returned = Some(idle),
                Err(still) => rest.push(still),
            }
        }
        self.flight = rest;

        let token = cqe.user_data;
        let Some(buffer) = returned else {
            // **A failure.** A completion for a token this client does not hold
            // is the dying instance answering across its own death, which is the
            // one thing that would make a restart unsound: two owners of one
            // buffer, and one of them a device.
            world.record(me, LOAD, wrote::STALE, token, cqe.result as u32 as u64);
            return;
        };
        let intact = buffer.bytes().first().copied() == Some(stamp(token));
        self.idle.push(buffer);

        if let Some((domain, code)) = cqe.error() {
            if domain == error::RESOURCE {
                // Back-pressure: the peer is busy rather than broken. The token
                // names work that has not happened, so it is owed rather than
                // spent.
                self.queue.push_back(token);
                world.record(me, LOAD, crate::proto::wrote::REFUSED, token, u64::from(code));
                world.send(
                    self.retry_ns,
                    me,
                    Message { from: me, kind: kind::RETRY, token, detail: 0 },
                );
                return;
            }
            // **A failure**, and the one gate G1's sentence is most directly
            // about: a client that was told no because its driver restarted has
            // observed something other than latency.
            //
            // Settled as well as failed, and the pair is deliberate: the
            // operation *was* answered and the answer was a refusal, which is
            // two facts rather than one. Counting it only as a failure would
            // leave `Report::lost` claiming it was never answered, and *lost*
            // and *refused* are different findings with different first
            // debugging steps — `claims/0005`'s diagnosis section says so and
            // could not if one number stood for both.
            world.record(me, LOAD, wrote::FAILED, token, packed(domain, code));
            self.settle(world, me, token);
            self.pump(world, me);
            return;
        }

        let slot = usize::from(phase(token) == half::READ);
        let already = self
            .ledger
            .entries
            .get(&op(token))
            .and_then(|entry| entry.answered.get(slot).copied())
            .unwrap_or(false);
        if already {
            // **A failure.** An operation answered twice is a re-submission the
            // system served on both sides of a restart, which is the failure a
            // count of completions alone would report as a success.
            world.record(me, LOAD, wrote::TWICE, token, u64::from(op(token)));
            return;
        }

        if !intact {
            // **A failure**, and under its own name rather than folded into the
            // one below. RFC 0041 concedes that no device model in this crate
            // can reach a client's bytes, so this cannot fire today; keeping it
            // inside `wrong` would have put a check that cannot fail inside the
            // number a gating claim thresholds.
            world.record(me, LOAD, wrote::TORN, token, NOTHING);
        }
        if phase(token) == half::READ && cqe.ext != value(op(token)) {
            // **A failure.** The value read back at this position is not the
            // value written there, through a *different instance* of the driver.
            world.record(me, LOAD, wrote::WRONG, token, cqe.ext);
        }

        self.settle(world, me, token);

        // A write that is answered earns its read-back, which is the half of the
        // workload that says the state behind the place survived. Issued after
        // the write rather than beside it, so that the order is unambiguous and
        // a mismatch is a statement about durability rather than about
        // scheduling.
        if self.reads_back && phase(token) == half::WRITE {
            self.queue.push_back(self::token(self.who, half::READ, op(token)));
        }
        self.pump(world, me);
    }

    /// One phase is over: write it down, and write down how long it took.
    ///
    /// The latency is from the phase's *first* submission, which is what a
    /// client experiences: a re-submission after a kill is the same operation
    /// waiting longer, and a clock restarted by the retry would report the last
    /// attempt's cost — a number nobody waited.
    fn settle(&mut self, world: &mut World, me: ActorId, token: u64) {
        let slot = usize::from(phase(token) == half::READ);
        let now = world.clock();
        let began = self
            .ledger
            .entries
            .get(&op(token))
            .and_then(|entry| entry.first_ns.get(slot).copied().flatten())
            .unwrap_or(now);
        self.mark(token);
        self.settled = self.settled.saturating_add(1);
        world.record(me, LOAD, wrote::SETTLED, token, now.saturating_sub(began));
    }

    /// Write one phase down as answered.
    fn mark(&mut self, token: u64) {
        let slot = usize::from(phase(token) == half::READ);
        if let Some(entry) = self.ledger.entries.get_mut(&op(token))
            && let Some(flag) = entry.answered.get_mut(slot)
        {
            *flag = true;
        }
    }

    /// The place's occupant is gone: take every buffer back, and reconnect.
    ///
    /// The evidence is [`PeerGone`] and nothing else, which is RFC 0024's rule
    /// and what makes the reclaim sound: the place tore the occupant's
    /// translations down before it sent this, so a transfer the dead instance
    /// had started faults rather than landing in memory this side is about to
    /// reuse.
    fn peer_gone(&mut self, world: &mut World, me: ActorId) {
        let Some(gone) = PeerGone::of(RingError::EpochChanged) else {
            return;
        };
        for lent in self.flight.drain(..) {
            let token = lent.token();
            let _ = lent.reclaim(gone);
            // Owed again. The operation was never answered, so it is not lost —
            // it is a submission the system has yet to make good on, and the
            // ledger is what says the difference.
            self.queue.push_front(token);
            world.record(me, LOAD, crate::proto::wrote::RECLAIM, token, 0);
        }
        // The old set names a table that no longer exists. Dropped rather than
        // reused: a registration is the occupant's to issue and this client has
        // no way to know what the next one will call the same memory.
        self.idle.clear();
        self.registered = false;
        self.register(world, me);
    }

    /// Say what this client managed, once.
    fn close_if_done(&mut self, world: &mut World, me: ActorId) {
        if self.closed || self.settled < self.owed {
            return;
        }
        self.closed = true;
        world.record(me, LOAD, wrote::CLOSED, u64::from(self.who), u64::from(self.settled));
    }
}

impl Actor for Load {
    fn name(&self) -> &'static str {
        LOAD
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::START => self.register(world, me),
            kind::CQE => {
                let Some(cqe) = world.wire().reap(self.place, me) else {
                    // A doorbell with nothing behind it. Ordinary, and recorded
                    // rather than ignored so that it changes the digest.
                    world.record(me, LOAD, message.kind, message.token, NOTHING);
                    return;
                };
                if phase(cqe.user_data) == half::REGISTER {
                    self.bind(world, me, &cqe);
                } else {
                    self.reap(world, me, &cqe);
                }
            }
            kind::RETRY => {
                if self.registered {
                    self.pump(world, me);
                } else {
                    self.register(world, me);
                }
            }
            kind::GONE => self.peer_gone(world, me),
            other => world.record(me, LOAD, other, message.token, NOTHING),
        }
    }
}

impl Drop for Load {
    /// The run ends and every buffer this client still holds ends with it.
    ///
    /// `InFlight`'s drop is a bomb on purpose — a *live* component abandoning a
    /// buffer the device is writing into is the bug RFC 0024 makes unwritable —
    /// and this is the other case, which `client::App` documents at length and
    /// which is the same here.
    fn drop(&mut self) {
        let Some(gone) = PeerGone::of(RingError::EpochChanged) else {
            return;
        };
        for lent in self.flight.drain(..) {
            let _ = lent.reclaim(gone);
        }
    }
}

/// The byte a client stamps into a buffer before lending it.
///
/// A function of the token, so a buffer coming back under the wrong token is
/// visible rather than plausible. Never zero, because a zeroed region is what a
/// component is handed and a stamp of zero would be indistinguishable from
/// memory nothing has touched.
const fn stamp(token: u64) -> u8 {
    ((token & 0x7F) as u8) | 0x80
}

/// Pack a domain and a code the way a completion carries them, for the trace.
const fn packed(domain: u8, code: u16) -> u64 {
    ((domain as u64) << 16) | code as u64
}

// -------------------------------------------------------------- the run

/// One chaos run's configuration.
///
/// Flat and small, for the scenario table's reason: everything here is a number
/// a person reading a failing seed's reproduction command has to be able to hold
/// in their head. What is *not* here is the restart policy, because that is the
/// component's own declaration and reading it from anywhere else would leave the
/// manifest as decoration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chaos {
    /// The component this run kills, by the name its record declares.
    pub name: &'static str,
    /// What the simulator puts behind the place.
    pub peer: Peer,
    /// The restart policy the component's manifest declares.
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
    /// How many times the occupant is killed. Unit: kills; zero is the control
    /// run, and without it the survival proves nothing.
    pub kills: u32,
    /// Whether there is state behind the place, and therefore a read-back.
    pub durable: bool,
    /// Whether the occupant answers a write out of a volatile cache and commits
    /// it afterwards.
    ///
    /// **The negative control for the ordering the third claim is.** `false` is
    /// the world — a write is on the medium before its client is told so —
    /// and `true` is the write-back cache a real driver would have, which a kill
    /// then loses. It is the sharper of the two durability controls because the
    /// bug it models is one a correct implementation actively avoids rather than
    /// one its data layout rules out.
    /// [`tests::a_write_answered_out_of_a_cache_is_caught_by_the_read_back`] is
    /// what it buys.
    pub lazy: bool,
    /// Whether the medium itself dies with the occupant.
    ///
    /// The coarser negative control for the read-back: a machine whose disk is
    /// erased by a segfault. `false` is the world; `true` is what makes the
    /// placement of [`Place::store`] a check rather than an assumption.
    pub volatile: bool,
    /// Whether the policy the manifest declares refills the place after a fault.
    ///
    /// Read out of the record rather than assumed, because `restart = never` is
    /// a value `docs/manifest.md` permits and a harness that refused it would be
    /// failing the build for a legitimate declaration — and the fix under
    /// pressure would be to widen the verdict, which is the wrong direction.
    /// [`verdict`] asks a different question of a component that declares it:
    /// the place is *expected* to retire, the client is expected to be told its
    /// peer is gone, and what must still hold is that nothing was answered
    /// twice and nothing was answered wrongly.
    pub refills: bool,
    /// Whether the work a dying occupant held is answered after the refill, as
    /// though its last transfers had landed late.
    ///
    /// **The negative control that matters most.** *Answered twice* and
    /// *answered for a token nobody holds* are the only two things standing
    /// between a restart and two owners of one buffer — and on a correct run
    /// neither of them ever fires, which is exactly the shape of an alarm nobody
    /// has watched. `true` is the failure RFC 0024's `PeerGone` is sound only
    /// because the frame prevents: the dead instance's answers arriving after
    /// its client has reclaimed the buffers they name.
    ///
    /// It is a knob in shipped source rather than a patch for the reason
    /// `runtime::Half::Provoke` is: a zero nothing can move is not evidence.
    /// [`tests::a_place_that_answers_after_a_death_is_caught`] is what it buys.
    pub leaky: bool,
}

/// How many translations one occupant's domain holds. Unit: translations.
///
/// Two, which is one more than a run needs: the client registers one set at a
/// time, and the spare is so that a domain running out is a configuration away
/// rather than a code change. The same number and the same reason as
/// `scenario::DOMAIN`.
const DOMAIN: u32 = 2;

impl Chaos {
    /// The workload every component in the deployment is driven with.
    ///
    /// One shape for every component on purpose. What differs between two runs
    /// of this harness is the component — its protocol, its ring, and above all
    /// its declared restart policy — and a workload that differed too would make
    /// two results incomparable.
    #[must_use]
    pub fn of(component: &Component, policy: Policy, kills: u32) -> Self {
        Self {
            // Leaked so that the name is `'static` like every other label in a
            // trace. One per component per process, which is a handful.
            name: Box::leak(component.name.clone().into_boxed_str()),
            peer: component.peer,
            policy,
            window: 4,
            depth: 8,
            operations: 48,
            service_ns: 400,
            spread_ns: 600,
            retry_ns: 2_000,
            buffer_bytes: 512,
            // Sectors for a disk, and larger than any position this workload
            // works at, so the only refusals in the run are the ones a kill
            // produced. A device refusing on its own terms would make the count
            // the assertions rest on ambiguous.
            extent: 4_096,
            kills,
            // Which peers have state behind them is a fact about the peer.
            // A disk's sectors outlive its driver and an object store's objects
            // outlive the component serving them; a link holds nothing and a
            // display's resources are its driver's own, which is why the
            // read-back is not asked of them. RFC 0041 records the second half
            // of that as a gap rather than as an omission.
            durable: matches!(component.peer, Peer::Blk | Peer::Native),
            lazy: false,
            volatile: false,
            // The component's own declaration, and the only field here read out
            // of the record rather than chosen by this harness.
            refills: policy.as_record().restarts_after(true, false),
            leaky: false,
        }
    }

    /// How long a place waits after a spawn before the next kill is due.
    ///
    /// Enough for the client to have work in flight and not enough for it to
    /// have finished: four service times, which at a window of four is about one
    /// window's worth of work. It is also the interval a deferred kill retries
    /// at. Unit: nanoseconds.
    const fn settle_ns(&self) -> u64 {
        self.service_ns.saturating_mul(4).saturating_add(self.spread_ns)
    }

    /// Run this configuration at `seed`.
    ///
    /// # Errors
    ///
    /// [`Trouble`] if the run does not finish inside [`BUDGET`] or a message
    /// names an actor that does not exist. A client waiting for a place that
    /// will never be refilled arrives here as [`Trouble::Budget`], which is the
    /// difference between reporting a hang and being one.
    pub fn run(&self, seed: u64) -> Result<Outcome, Trouble> {
        let mut sim = Simulation::new(seed, BUDGET);
        self.cover(&mut sim);

        let cfg = Config {
            depth: self.depth,
            service_ns: self.service_ns,
            spread_ns: self.spread_ns,
            // Nothing is lost on purpose here. A device that drops completions
            // is `E1-P02`'s `blkloss`, and a chaos run that also dropped them
            // would not be able to say which mechanism a survival came from.
            lose_one_in: 0,
            extent: self.extent,
            queue_size: crate::virtq::QUEUE_SIZE,
            domain: DOMAIN,
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

        // The place first: its occupant has to be in it before the client's
        // registration arrives, or the very first submission would exercise the
        // pending path and the run would begin with the thing it is supposed to
        // reach under load.
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
    /// The same discipline `scenario::Scenario::cover` applies: what a run
    /// covers travels in the hashed bytes, so an artefact quoted in a year says
    /// what it was rather than being read as covering the system. The seed is
    /// deliberately absent, for the reason `trace.rs` gives — a digest that moved
    /// with the seed on its own would make the negative control pass without a
    /// single decision changing.
    fn cover(&self, sim: &mut Simulation) {
        let world = sim.world();
        world.cover("f-sim artefact 1 — a driver killed under sustained load");
        world
            .cover("covers      a place, its occupants, the client above it, and the state behind");
        world.cover("not covered the frame's own kill: `cargo xtask component` boots that half,");
        world.cover("            and RFC 0041 declares the gap between the two");
        world.cover(&format!("component   {}", self.name));
        world.cover(&format!("modelled    as {}", self.peer.label()));
        world.cover(&format!(
            "policy      {} restart(s) in {} tick(s), backoff {} to {} tick(s)",
            self.policy.max_restarts,
            self.policy.budget_window_ticks,
            self.policy.backoff_first_ticks,
            self.policy.backoff_max_ticks,
        ));
        world.cover(&format!(
            "workload    {} operation(s), window {}, {} kill(s), read-back {}",
            self.operations,
            self.window,
            self.kills,
            if self.durable { "on" } else { "off" },
        ));
    }
}

/// How a peer is put into a place.
///
/// A function per peer rather than a generic parameter, because a place holds
/// *a component* and which model is behind it is the deployment's answer. It is
/// also what makes a respawn a respawn: the same function, called again, with no
/// state carried over from the instance that died.
fn spawner(peer: Peer) -> fn(Config) -> Box<dyn Actor> {
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
        // `store` and anything else a manifest names with no device under it.
        // `deploy::MODELS` is what maps a protocol to a peer, so a component
        // reaching here is one that declared a protocol modelled as `native`.
        Peer::Queue | Peer::Deployment | Peer::Native => {
            |cfg| Box::new(Native::new(cfg.depth, cfg.service_ns, cfg.spread_ns, cfg.domain))
        }
    }
}

// ------------------------------------------------------------- the verdict

/// What one chaos run produced, read out of its artefact.
///
/// Out of the artefact and not out of the actors, because the artefact is what a
/// failing seed hands to a person: a number that existed only in memory is a
/// number a reproduction cannot report.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Report {
    /// Phases answered for the first time. Unit: phases.
    pub settled: u32,
    /// Phases the client issued and this run owed. Unit: phases.
    pub owed: u32,
    /// Operations never answered. **The blast radius' first number.**
    /// Unit: phases.
    pub lost: u32,
    /// Operations answered a second time. Unit: phases.
    pub twice: u32,
    /// Completions for tokens nobody held. Unit: completions.
    pub stale: u32,
    /// Answers that disagreed with what was written. Unit: phases.
    pub wrong: u32,
    /// Buffers that came back bearing another operation's stamp.
    /// Unit: buffers.
    ///
    /// Beside [`Report::wrong`] and not inside it: RFC 0041 concedes that no
    /// device model in this crate can reach a client's bytes, so this is a
    /// structural check with no way to fire today and the number a gating claim
    /// thresholds should not have one of those folded into it.
    pub torn: u32,
    /// Writes the dying occupants had accepted and not committed, across every
    /// kill. Unit: writes.
    ///
    /// **The positive that makes [`Report::wrong`]'s zero mean something.** Each
    /// is a write interrupted between the medium and its answer; the client owes
    /// it again, and the read-back is what says the value that finally landed is
    /// the right one. Zero means the kills never landed on the durability path,
    /// and [`verdict`] refuses that for the same reason it refuses
    /// [`Report::flying_min`] of zero.
    pub dropped: u32,
    /// Refusals the client could not retry. Unit: refusals.
    pub failed: u32,
    /// Clients that observed anything other than added latency.
    /// **The number gate G1's sentence is about.** Unit: clients.
    pub clients_failed: u32,
    /// Occupants killed. Unit: kills.
    pub kills: u32,
    /// Occupants put in, the first one included. Unit: instances.
    pub spawns: u32,
    /// Places retired. Unit: places.
    pub retired: u32,
    /// The fewest operations in flight at any kill. Unit: operations.
    ///
    /// Zero would mean a kill landed between operations, which is the weaker
    /// experiment this one is named after not being.
    pub flying_min: u32,
    /// Entries a refill rang the doorbell for, added up across every refill.
    /// Unit: submissions.
    ///
    /// **Entries and not refills**, which is what makes it comparable with
    /// [`Report::pended`]: each submission that pends leaves exactly one entry
    /// sitting in the shared region, and a refill rings once per entry it finds.
    /// Counting refills instead compared a count of doorbells with a count of
    /// submissions — two different granularities — and a client that pended
    /// twice for one kill would have reddened a correct run.
    ///
    /// Beside [`Report::pended`] rather than folded into it, because *a
    /// submission met an empty place* and *the refill answered it* are two
    /// halves of one mechanism and a pend with no matching resume is a client
    /// waiting on a doorbell nobody rang — which is the failure mode
    /// endpoint-as-a-place has, and the one a single counter would hide.
    pub resumed: u32,
    /// Submissions that pended because the place was empty. Unit: submissions.
    pub pended: u32,
    /// Buffers the client took back on peer-gone evidence, across every kill.
    ///
    /// **What a restart cost in work redone.** Each of these is an operation
    /// that had been submitted and was never answered, so the client owes it
    /// again — which is the difference between a delay and a loss, counted. A
    /// run where this is zero is a run where the kills landed on nothing, and
    /// [`Report::flying_min`] is the same statement from the place's side.
    /// Unit: operations.
    pub reclaimed: u32,
    /// The worst latency any operation took. Unit: **virtual** nanoseconds.
    pub worst_ns: u64,
    /// The median client wait. Unit: **virtual** nanoseconds.
    ///
    /// The three quantiles below are `claims/0006`'s named metrics, produced by
    /// the command that claim's `[reproduce]` names — because a claim whose
    /// reproduction prints somebody else's numbers reproduces nothing. They are
    /// the *model's* clock and not a machine's, which is why claim 0006 is
    /// `pending` and why every printer of these says VIRTUAL beside them.
    pub p50_ns: u64,
    /// The 99th-percentile client wait. Unit: **virtual** nanoseconds.
    pub p99_ns: u64,
    /// The 99.9th-percentile client wait. Unit: **virtual** nanoseconds.
    pub p999_ns: u64,
    /// Where the run's clock stopped. Unit: nanoseconds.
    pub finished_ns: u64,
    /// The artefact's digest.
    pub digest: u64,
}

impl Report {
    /// Read one run's artefact.
    #[must_use]
    pub fn of(chaos: &Chaos, outcome: &Outcome) -> Self {
        let phases = if chaos.durable { 2 } else { 1 };
        let owed = chaos.operations.saturating_mul(phases);
        let count = |actor: &str, kind: &str| {
            u32::try_from(
                outcome
                    .trace
                    .records()
                    .iter()
                    .filter(|record| record.actor == actor && record.kind == kind)
                    .count(),
            )
            .unwrap_or(u32::MAX)
        };
        // The sum of a record kind's details, where the detail is itself a
        // count. `resumed` and `dropped` are both of that shape: one record per
        // event, carrying how many things the event was about.
        let total = |actor: &str, kind: &str| {
            u32::try_from(
                outcome
                    .trace
                    .records()
                    .iter()
                    .filter(|record| record.actor == actor && record.kind == kind)
                    .fold(0u64, |sum, record| sum.saturating_add(record.detail)),
            )
            .unwrap_or(u32::MAX)
        };
        let settled = count(LOAD, wrote::SETTLED);
        let failed = count(LOAD, wrote::FAILED);
        let twice = count(LOAD, wrote::TWICE);
        let stale = count(LOAD, wrote::STALE);
        let wrong = count(LOAD, wrote::WRONG);
        let torn = count(LOAD, wrote::TORN);
        let kills = count(PLACE, wrote::KILLED);

        let mut waits: Vec<u64> = outcome
            .trace
            .records()
            .iter()
            .filter(|record| record.actor == LOAD && record.kind == wrote::SETTLED)
            .map(|record| record.detail)
            .collect();
        waits.sort_unstable();
        let worst_ns = waits.last().copied().unwrap_or(0);
        let flying_min = outcome
            .trace
            .records()
            .iter()
            .filter(|record| record.actor == PLACE && record.kind == wrote::KILLED)
            .map(|record| u32::try_from(record.detail).unwrap_or(u32::MAX))
            .min()
            .unwrap_or(0);

        // One client, and it observed something other than added latency if any
        // of the four alarms fired for it. A count of clients rather than of
        // events, because the blast radius is *how many clients noticed* and a
        // client that noticed twice noticed once.
        let noticed = failed + twice + stale + wrong + torn;
        Self {
            settled,
            owed,
            lost: owed.saturating_sub(settled),
            twice,
            stale,
            wrong,
            torn,
            dropped: total(PLACE, wrote::DROPPED),
            failed,
            clients_failed: u32::from(noticed > 0),
            kills,
            spawns: count(PLACE, wrote::SPAWNED),
            retired: count(PLACE, wrote::RETIRED),
            flying_min,
            resumed: total(PLACE, wrote::RESUMED),
            pended: count(PLACE, wrote::PENDED),
            reclaimed: count(LOAD, crate::proto::wrote::RECLAIM),
            worst_ns,
            p50_ns: quantile(&waits, 50, 100),
            p99_ns: quantile(&waits, 99, 100),
            p999_ns: quantile(&waits, 999, 1_000),
            finished_ns: outcome.finished_ns,
            digest: outcome.digest(),
        }
    }
}

/// The wait at one quantile of a sorted list. Unit: nanoseconds.
///
/// Nearest-rank, in integer arithmetic. `claims/README.md` rule 3 asks for the
/// distribution rather than the mean, and a float here would be a second
/// arithmetic in a crate whose entire proposition is that two processes agree on
/// the last bit.
fn quantile(sorted: &[u64], numerator: u64, denominator: u64) -> u64 {
    let Ok(len) = u64::try_from(sorted.len()) else { return 0 };
    if len == 0 {
        return 0;
    }
    let rank = len.saturating_mul(numerator).div_ceil(denominator).clamp(1, len);
    usize::try_from(rank - 1).ok().and_then(|at| sorted.get(at).copied()).unwrap_or(0)
}

/// What a chaos run has to have produced, checked against what it did.
///
/// A free function rather than a method on [`Report`] because it takes the
/// *pair* — a killed run and the control run beside it — and the pair is the
/// claim. `blk`, `mutate` and `runtime` all make the same argument one
/// subsystem over: a survival with no control run beside it proves that nothing
/// went wrong, not that anything was under test.
///
/// # Errors
///
/// A sentence naming what did not hold.
pub fn verdict(chaos: &Chaos, killed: &Report, calm: &Report) -> Result<(), String> {
    // The control first, because if it fails then nothing the killed run says
    // means anything: a workload that cannot complete without a kill in it is a
    // workload, not an experiment.
    if calm.kills != 0 {
        return Err("the control run killed something, so it is not a control".into());
    }
    if calm.lost != 0 || calm.settled != calm.owed {
        return Err(format!(
            "the control run answered {} of {} operation(s) with nothing killed, so the killed \
             run's numbers are about the workload rather than about the kill",
            calm.settled, calm.owed
        ));
    }
    if calm.clients_failed != 0 {
        return Err("the control run's client observed a failure with nothing killed".into());
    }

    // Then the experiment: it has to have happened.
    if killed.kills != chaos.kills {
        return Err(format!(
            "the plan was {} kill(s) and {} landed, so the run is a smaller experiment than the \
             one it is named after",
            chaos.kills, killed.kills
        ));
    }
    if killed.flying_min == 0 {
        return Err("a kill landed with nothing in flight, which is a kill between operations \
                    rather than in the middle of them"
            .into());
    }
    // The kills have to have landed on the durability path and not merely on the
    // wire. Each of these is a write the dead instance had taken and not
    // committed, so the read-back below is asking about a value that was
    // genuinely interrupted rather than about one nothing ever threatened —
    // which is the difference between a claim and a placement.
    if killed.dropped == 0 {
        return Err("no kill interrupted a write between the medium and its answer, so the \
                    read-back is a question about a workload that never put a write at risk"
            .into());
    }

    if chaos.refills {
        // The retirement before the spawn count, because it is the cause and
        // the spawn count is the symptom: a place that ran out of budget is one
        // occupant short by construction, and a verdict that named the shortfall
        // would send a reader looking for a refill that was never owed.
        if killed.retired != 0 {
            return Err("the place was retired, so what the client observed is an outage rather \
                        than a restart"
                .into());
        }
        if killed.spawns != chaos.kills + 1 {
            return Err(format!(
                "{} kill(s) produced {} occupant(s), and the place is supposed to be refilled \
                 after every one of them",
                killed.kills, killed.spawns
            ));
        }
        if killed.pended == 0 {
            return Err("no submission ever pended, so the client never met an empty place and \
                        the mechanism gate G1 rests on was not exercised"
                .into());
        }
        if killed.resumed < killed.pended {
            return Err(format!(
                "{} submission(s) pended and the refills rang for {}. Both are counted in \
                 submissions rather than one in submissions and one in doorbells, so a shortfall \
                 is a client waiting on a bell nobody rang and its operation being carried by \
                 the retry rather than by the mechanism — which is the mechanism not working \
                 while every count above still reads clean",
                killed.pended, killed.resumed
            ));
        }
        // Nothing is lost: the first of the three sentences, and it is asked of
        // a component whose manifest says the place comes back.
        if killed.lost != 0 {
            return Err(format!(
                "{} operation(s) were submitted and never answered — the first of the three \
                 things `only latency` means",
                killed.lost
            ));
        }
    } else {
        // `restart = never` is a value `docs/manifest.md` permits, and a
        // component that declares it is asking for its place to stay empty. So
        // the question changes rather than the threshold — R04 read correctly
        // is refusing what the build does not expect, not refusing what the
        // schema allows, and the fix under pressure for the second would be to
        // widen the verdict, which is the wrong direction.
        //
        // *Nothing is lost* is therefore not asked: the client was told its peer
        // is gone and there is no peer coming, which is the declared outcome
        // rather than a failure. Everything below is still asked, and so is the
        // declaration itself — a refill here would be the supervisor ignoring a
        // manifest.
        if killed.retired == 0 {
            return Err("the component declares `restart = never` and its place was not \
                        retired, so the supervisor refilled a place the manifest asked it to \
                        leave empty"
                .into());
        }
        if killed.spawns != 1 {
            return Err(format!(
                "the component declares `restart = never` and {} occupant(s) went in, so a \
                 respawn happened that the manifest forbids",
                killed.spawns
            ));
        }
    }

    // And then the claims asked of every component whatever its policy declares,
    // separately, because *no client observes anything except added latency* is
    // three sentences and a count of completions would pass for two of them.
    if killed.twice != 0 || killed.stale != 0 {
        return Err(format!(
            "{} operation(s) were answered twice and {} completion(s) arrived for tokens nobody \
             held: a restart served work on both sides of itself",
            killed.twice, killed.stale
        ));
    }
    if killed.wrong != 0 {
        return Err(format!(
            "{} answer(s) disagreed with what was written, so the state behind the place did not \
             survive its occupant",
            killed.wrong
        ));
    }
    if killed.torn != 0 {
        return Err(format!(
            "{} buffer(s) came back bearing another operation's stamp, which is a device having \
             written into memory the client had lent to something else",
            killed.torn
        ));
    }
    if killed.failed != 0 {
        return Err(format!(
            "{} refusal(s) the client could not retry, so a client observed the restart as an \
             error rather than as a wait",
            killed.failed
        ));
    }

    // The bound. *Only latency* with an unbounded tail is a hang with better
    // manners, so the worst operation is required to sit under what the same
    // workload costs with nothing killed plus every pause the declared policy
    // takes. Not a threshold somebody tuned: both terms are declared elsewhere,
    // one in the manifest and one by the control run.
    let bound = calm.worst_ns.saturating_add(chaos.policy.ladder_ns(chaos.kills));
    if killed.worst_ns > bound {
        return Err(format!(
            "the worst operation took {} ns against a bound of {} ns — {} ns of control plus a \
             declared backoff ladder of {} ns. A latency past the ladder is a wait nothing in \
             the policy accounts for",
            killed.worst_ns,
            bound,
            calm.worst_ns,
            chaos.policy.ladder_ns(chaos.kills)
        ));
    }
    Ok(())
}

// -------------------------------------------------------------- the whole set

/// One component's pair of runs, and what they produced.
#[derive(Clone, Debug)]
pub struct Pair {
    /// The configuration the killed run used.
    pub chaos: Chaos,
    /// The run with kills in it.
    pub killed: Report,
    /// The same run with none.
    pub calm: Report,
}

impl Pair {
    /// The added latency a client observed, worst case. Unit: nanoseconds.
    #[must_use]
    pub fn added_ns(&self) -> u64 {
        self.killed.worst_ns.saturating_sub(self.calm.worst_ns)
    }
}

/// How many times each component is killed in a sweep.
///
/// Three, and it is a bound rather than a preference: `user/store/manifest.toml`
/// declares a budget of three restarts in three seconds, and a fourth kill would
/// retire the place — which is the policy working and is a different experiment.
/// The retirement itself is exercised by `cargo xtask component`, which drives a
/// budget to its far end on every boot.
pub const KILLS: u32 = 3;

/// Run every component in a deployment, killed and calm, and judge each pair.
///
/// **This is *each driver component in turn*.** The set is the deployment's —
/// the component files the build produced — rather than a list in this file, so
/// a component that cannot survive being killed is a red build for whoever added
/// it.
///
/// That on its own is not enough and the first draft of this comment claimed it
/// was. A sweep over the build output and a check against the build output are
/// the same set read twice, and would both fall silently to the same smaller
/// number. What ties this to the *source tree* is two checks in `xtask` and they
/// are named here so a reader can go and see them: `lint-components` requires
/// the hand-written build list to equal the set of `manifest.toml` files under
/// `user/`, and `cargo xtask chaos` requires the number of components this sweep
/// killed to equal that same manifest count. A component crate added with a
/// manifest and left out of the build list reddens the first; one dropped from
/// the build output reddens the second.
///
/// # Errors
///
/// The first pair that did not hold, naming the component and what failed.
pub fn sweep(deployment: &Deployment, seed: u64, kills: u32) -> Result<Vec<Pair>, String> {
    if deployment.is_empty() {
        return Err(
            "no components to kill. `cargo xtask chaos` builds them first; a sweep over an \
             empty set produces a stable digest and no evidence at all."
                .into(),
        );
    }
    let mut pairs = Vec::new();
    for component in deployment.components() {
        let policy = policy_of(component);
        let chaos = Chaos::of(component, policy, kills);
        let mut control = chaos;
        control.kills = 0;

        let under_kill = chaos.run(seed).map_err(|why| {
            format!("{}: the killed run did not finish — {}", component.name, why.message())
        })?;
        let untouched = control.run(seed).map_err(|why| {
            format!("{}: the control run did not finish — {}", component.name, why.message())
        })?;

        let killed = Report::of(&chaos, &under_kill);
        let calm = Report::of(&control, &untouched);
        verdict(&chaos, &killed, &calm).map_err(|why| format!("{}: {why}", component.name))?;
        pairs.push(Pair { chaos, killed, calm });
    }
    Ok(pairs)
}

/// The restart policy a component declares.
fn policy_of(component: &Component) -> Policy {
    Policy {
        backoff_first_ticks: component.backoff_first_ticks,
        backoff_max_ticks: component.backoff_max_ticks,
        max_restarts: component.max_restarts,
        budget_window_ticks: component.budget_window_ticks,
        restart: component.restart,
    }
}

/// The one number two sweeps are compared by.
///
/// Every pair's digest, folded in component order — which is name order, and is
/// therefore a property of what the components declare rather than of what a
/// filesystem handed back. FNV-1a, the same function `trace.rs` hashes with, so
/// there is one digest arithmetic in this crate rather than two.
#[must_use]
pub fn digest(pairs: &[Pair]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for pair in pairs {
        for word in [pair.killed.digest, pair.calm.digest] {
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
    use f_abi::manifest::restart;

    /// The seeds every assertion below is made at.
    ///
    /// Three rather than one, for `fault.rs`'s reason: a response asserted at a
    /// single seed is a response asserted against one interleaving.
    const SEEDS: [u64; 3] = [DEFAULT_SEED, 7, 0x5EED_5EED_5EED_5EED];

    /// The two components this tree deploys, built in memory.
    ///
    /// In memory rather than read from `target/component/`, for the reason
    /// `deploy::fixture` gives: a test needing a build artefact fails in a fresh
    /// checkout for a reason that is not a defect, and one that skipped itself
    /// when the artefact was missing would pass in exactly the tree where it had
    /// stopped checking anything. The file path is exercised by
    /// `cargo xtask chaos`.
    fn deployment() -> Deployment {
        Deployment::of(vec![component("virtio-blk", "blk", 256), component("store", "store", 16)])
            .expect("two names")
    }

    fn one(name: &str, protocol: &str, kills: u32) -> Chaos {
        let built = component(name, protocol, 256);
        let policy = policy_of(&built);
        Chaos::of(&built, policy, kills)
    }

    fn report(chaos: &Chaos, seed: u64) -> Report {
        Report::of(chaos, &chaos.run(seed).expect("a chaos run terminates"))
    }

    #[test]
    fn a_driver_is_killed_under_load_and_the_client_observes_only_a_wait() {
        // **The exit criterion, at three seeds.** Every operation answered,
        // exactly once, with what was written, and the place refilled after
        // every kill — beside a control run of the same workload with nothing
        // killed, because a survival with no control proves that nothing went
        // wrong rather than that anything was under test.
        for seed in SEEDS {
            let pairs = sweep(&deployment(), seed, KILLS)
                .unwrap_or_else(|why| panic!("seed {seed:#018x}: {why}"));
            assert_eq!(pairs.len(), 2, "the sweep did not run every component");
            for pair in &pairs {
                assert_eq!(pair.killed.kills, KILLS);
                assert_eq!(pair.killed.lost, 0);
                assert_eq!(pair.killed.clients_failed, 0);
                // The zero above is only a statement about durability if a kill
                // actually caught a write between the medium and its answer.
                assert!(
                    pair.killed.dropped > 0,
                    "{}: no kill interrupted a write, so `wrong == 0` is about a workload that \
                     never put one at risk",
                    pair.chaos.name
                );
                assert!(
                    pair.added_ns() > 0,
                    "{}: a kill cost nothing at all, which means the client never waited for one",
                    pair.chaos.name
                );
            }
        }
    }

    #[test]
    fn the_verdict_holds_and_the_kills_keep_landing_across_a_span_of_seeds() {
        // Three seeds are three interleavings, and two of this verdict's
        // conditions are *minima* — a kill has to land with work in flight, and
        // it has to catch a write between the medium and its answer. A minimum
        // is exactly the kind of condition that holds at the seeds somebody
        // happened to pick and fails at the next one, which would make
        // `cargo xtask chaos` a gate that goes red for a reason nobody changed.
        //
        // So it is asked of a span. Thirty-two seeds, cheap because the model is
        // small, and the assertion is the whole verdict rather than one column:
        // if a seed exists where three kills cannot catch a write, this is where
        // it is found rather than in CI a month later. `E1-P03`'s sweep is what
        // would take it further, and a chaos run is already a function of
        // `(seed, commit)` for it to sweep over.
        let set = deployment();
        let mut worst_flying = u32::MAX;
        let mut worst_dropped = u32::MAX;
        for step in 0..32u64 {
            let seed = DEFAULT_SEED ^ step.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let pairs =
                sweep(&set, seed, KILLS).unwrap_or_else(|why| panic!("seed {seed:#018x}: {why}"));
            for pair in &pairs {
                worst_flying = worst_flying.min(pair.killed.flying_min);
                worst_dropped = worst_dropped.min(pair.killed.dropped);
            }
        }
        assert!(worst_flying >= 1, "a seed exists where a kill landed between operations");
        assert!(
            worst_dropped >= 1,
            "a seed exists where three kills caught no write between the medium and its \
             answer, so at that seed the read-back is about values nothing threatened"
        );
    }

    #[test]
    fn a_write_answered_out_of_a_cache_is_caught_by_the_read_back() {
        // **The negative control the third claim actually rests on.** The
        // ordinary run's `wrong == 0` has to be a property of *code* — a write
        // reaches the medium before its completion is handed on — and not a
        // property of which structure the harness happened to put the map in.
        // So the occupant is given the write-back cache a real driver would
        // have: it answers out of memory, commits afterwards, and a kill in
        // between loses a write its client was told had succeeded.
        //
        // Nothing else changes. Same place, same client, same store behind it;
        // one ordering different, and the read-back must see it.
        let mut broken = one("virtio-blk", "blk", KILLS);
        broken.lazy = true;
        let mut control = broken;
        control.kills = 0;

        for seed in SEEDS {
            let killed = report(&broken, seed);
            assert!(
                killed.wrong > 0,
                "seed {seed:#018x}: a driver answering out of a cache that {KILLS} kill(s) \
                 emptied lost nothing, so the read-back cannot see a lost write at all"
            );
            // And the control run with the same cache and no kill is clean,
            // which is what says the finding is the *kill* rather than the
            // cache: a write-back cache that is never interrupted is correct.
            let calm = report(&control, seed);
            assert_eq!(
                calm.wrong, 0,
                "seed {seed:#018x}: a cache nothing interrupted still answered wrongly, so the \
                 control is reporting the cache rather than the kill"
            );
            let why =
                verdict(&broken, &killed, &calm).expect_err("a lost cached write is a failure");
            assert!(
                why.contains("did not survive"),
                "seed {seed:#018x}: the verdict named the wrong failure: {why}"
            );
        }
    }

    #[test]
    fn a_store_that_dies_with_its_driver_is_caught_by_the_read_back() {
        // The negative control, and the reason the read-back is a check rather
        // than a sentence. `fault.rs` records the shape of the mistake this
        // avoids: an assertion that cannot fail is indistinguishable from one
        // nobody wrote, and every device model in this crate is structurally
        // unable to touch a client's buffer — so a content check resting on the
        // buffer alone would hold in every record of every run.
        //
        // Here the state behind the place is put *inside* the occupant instead,
        // which is what a model of a disk erased by a driver bug would be. The
        // read-back must go wrong, and the verdict must say so in those words.
        let mut broken = one("virtio-blk", "blk", KILLS);
        broken.volatile = true;
        let mut control = broken;
        control.kills = 0;
        control.volatile = false;

        for seed in SEEDS {
            let killed = report(&broken, seed);
            // **At least one read per kill**, and that bound is the positive
            // claim read backwards: a wiped store is only visible to a read
            // whose write was answered before the wipe and whose read is
            // answered after it, so this number is exactly *how many reads
            // crossed a restart*. One per kill is what makes the green run's
            // zero a statement about durability rather than about a workload
            // whose reads all happened to land inside one instance.
            assert!(
                killed.wrong >= KILLS,
                "seed {seed:#018x}: a store wiped by {KILLS} restart(s) was read back wrongly \
                 {} time(s), so fewer reads crossed a restart than there were restarts",
                killed.wrong
            );
            let why = verdict(&broken, &killed, &report(&control, seed))
                .expect_err("a wiped store is a failure");
            assert!(
                why.contains("did not survive"),
                "seed {seed:#018x}: the verdict named the wrong failure: {why}"
            );
        }
    }

    #[test]
    fn a_place_that_answers_after_a_death_is_caught() {
        // **The negative control that matters most.** *Answered twice* and
        // *answered for a token nobody holds* are the only two things standing
        // between a restart and two owners of one buffer, and on a correct run
        // neither of them ever fires — which is precisely the shape of an alarm
        // nobody has watched. So the place is made to forget the one line that
        // discards what the dying instance had already published, and the client
        // must see it and say so.
        //
        // This is also what says the ordinary run's zeros are about the code
        // rather than about the path being unreachable: the same branch, the
        // same client, one line different.
        let mut broken = one("virtio-blk", "blk", KILLS);
        broken.leaky = true;
        let mut control = broken;
        control.kills = 0;
        control.leaky = false;

        for seed in SEEDS {
            let killed = report(&broken, seed);
            let calm = report(&control, seed);
            // Every kill leaves the work it was holding behind, so the alarm
            // fires at least once per kill at every seed. Required at every one
            // rather than at some: an alarm that fires at one interleaving and
            // not another is an alarm a sweep would find and a suite would not.
            assert!(
                killed.stale + killed.twice >= KILLS,
                "seed {seed:#018x}: {} answer(s) arrived after a death that had voided them, \
                 against {KILLS} kill(s)",
                killed.stale + killed.twice
            );
            let why = verdict(&broken, &killed, &calm)
                .expect_err("a leaked answer from a dead instance is a failure");
            assert!(
                why.contains("both sides of itself"),
                "seed {seed:#018x}: the verdict named the wrong failure: {why}"
            );
        }
    }

    #[test]
    fn a_component_that_declares_restart_never_is_judged_by_its_own_declaration() {
        // `docs/manifest.md` permits `restart = never`, and RFC 0008 makes it a
        // component's own statement about itself. The first draft of this
        // harness applied one workload and one verdict to every component, so
        // the first such component to be deployed would have turned
        // `cargo xtask verify` red for a legitimate declaration — and the fix
        // under pressure would have been to widen the verdict, which is the
        // wrong direction. R04 is refusing what the build does not expect, not
        // refusing what the schema allows.
        //
        // So the question changes instead: the place is expected to stay empty,
        // the client is expected to be told its peer is gone, and what must
        // still hold is that nothing was answered twice and nothing wrongly.
        let mut declared = component("store", "store", 16);
        declared.restart = restart::NEVER;
        let policy = policy_of(&declared);
        // One kill, because the first one is the last one: there is no second
        // occupant to kill.
        let chaos = Chaos::of(&declared, policy, 1);
        assert!(!chaos.refills, "a `never` policy was read as a policy that refills");
        let mut control = chaos;
        control.kills = 0;

        for seed in SEEDS {
            let killed = report(&chaos, seed);
            assert_eq!(killed.spawns, 1, "a place the manifest asked to stay empty was refilled");
            assert_eq!(killed.retired, 1, "the place was not retired");
            assert!(
                killed.lost > 0,
                "seed {seed:#018x}: a place that was never refilled answered everything anyway"
            );
            assert_eq!(killed.twice + killed.stale + killed.wrong, 0);
            verdict(&chaos, &killed, &report(&control, seed))
                .unwrap_or_else(|why| panic!("seed {seed:#018x}: {why}"));
        }

        // And the declaration is checked in the other direction too: a place
        // that *did* refill against a `never` policy is the supervisor ignoring
        // a manifest, and the verdict has to say so rather than passing because
        // every count came out clean.
        let refilled = report(&one("store", "store", 1), DEFAULT_SEED);
        let why = verdict(&chaos, &refilled, &report(&control, DEFAULT_SEED))
            .expect_err("a refill against a `never` policy is a failure");
        assert!(why.contains("never"), "the verdict named the wrong failure: {why}");
    }

    #[test]
    fn the_two_refusals_no_correct_run_can_reach_are_reached_here() {
        // Two of `verdict`'s conditions cannot be produced by any configuration
        // of this model, and a condition nobody can reach is a condition nobody
        // has checked. So they are reached against a report rather than against
        // a run — which is a smaller claim, and it is stated as one: what this
        // asserts is that the branch exists and names its own failure, not that
        // the system can produce it.
        //
        // `dropped == 0` is the first. It is the row that stops the read-back
        // being a question about values nothing threatened, and on this workload
        // every kill catches writes — so the only way to see the refusal is to
        // take a real report and remove the number.
        let chaos = one("virtio-blk", "blk", KILLS);
        let mut control = chaos;
        control.kills = 0;
        let calm = report(&control, DEFAULT_SEED);
        let real = report(&chaos, DEFAULT_SEED);
        assert!(real.dropped > 0, "the real run caught no write, so this test proves nothing");

        let mut threatened_nothing = real;
        threatened_nothing.dropped = 0;
        let why = verdict(&chaos, &threatened_nothing, &calm)
            .expect_err("a run that interrupted no write is refused");
        assert!(why.contains("never put a write at risk"), "the wrong failure: {why}");

        // `torn` is the second, and RFC 0041 concedes why: no device model in
        // this crate can reach a client's bytes, so nothing here can make a
        // buffer come back under another operation's stamp. It is counted
        // separately from `wrong` precisely so that a gating threshold is not
        // carrying a number that cannot move, and this is what says the branch
        // is wired to something.
        let mut stamped_wrongly = real;
        stamped_wrongly.torn = 1;
        let why = verdict(&chaos, &stamped_wrongly, &calm)
            .expect_err("a buffer under another operation's stamp is refused");
        assert!(why.contains("stamp"), "the wrong failure: {why}");
    }

    #[test]
    fn a_refusal_the_client_cannot_retry_is_counted_and_fails_the_run() {
        // The third alarm, shown to move. A disk of four sectors refuses every
        // position past it with a status byte of its own, which reaches the
        // client as `DEVICE` — not `RESOURCE`, so not back-pressure, so not
        // something to wait out. That is a device refusing on its own terms
        // rather than a restart, and the point is only that `wrote::FAILED` is a
        // branch something takes: a counter nothing can move is how three
        // interrupt vectors came to be uncounted one subsystem over.
        let mut cramped = one("virtio-blk", "blk", 0);
        cramped.extent = 4;
        let killed = report(&cramped, DEFAULT_SEED);
        assert!(killed.failed > 0, "a four-sector disk served every position asked of it");
        assert_eq!(killed.clients_failed, 1, "a client was refused and the blast radius stayed 0");
        let why = verdict(&cramped, &killed, &killed).expect_err("a refused client is a failure");
        assert!(why.contains("control run"), "the verdict named the wrong failure: {why}");
    }

    #[test]
    fn a_budget_spent_leaves_the_place_empty_and_the_work_unanswered() {
        // The fourth alarm, and the outcome the ordinary run is configured under
        // rather than into: a kill past the declared budget retires the place,
        // the client's re-registration pends against a place that is never
        // refilled, and the run ends with work owed. That is an outage rather
        // than a restart, it is what `KILLS` is chosen to stay under, and it is
        // what says `lost` is arithmetic over something rather than arithmetic
        // over nothing.
        let policy = policy_of(&component("store", "store", 16));
        let past = policy.max_restarts + 1;
        let mut doomed = one("store", "store", past);
        // A window wide enough that the client still has work owed when the
        // budget runs out; the default workload would otherwise be finished.
        doomed.operations = 96;
        let killed = report(&doomed, DEFAULT_SEED);
        assert_eq!(killed.retired, 1, "the budget did not run out");
        assert!(killed.lost > 0, "a retired place answered everything anyway");
        let mut control = doomed;
        control.kills = 0;
        let why = verdict(&doomed, &killed, &report(&control, DEFAULT_SEED))
            .expect_err("a retired place is an outage");
        assert!(
            why.contains("retired") || why.contains("never answered"),
            "the verdict named the wrong failure: {why}"
        );
    }

    #[test]
    fn the_control_run_completes_and_the_killed_run_costs_only_time() {
        // The pair, held against each other at the level the claim is made:
        // the same operations, the same answers, and a clock that moved.
        for seed in SEEDS {
            let chaos = one("virtio-blk", "blk", KILLS);
            let mut control = chaos;
            control.kills = 0;
            let killed = report(&chaos, seed);
            let calm = report(&control, seed);
            assert_eq!(calm.settled, calm.owed, "the control run did not finish its work");
            assert_eq!(killed.settled, killed.owed, "the killed run did not finish its work");
            assert!(
                killed.finished_ns > calm.finished_ns,
                "seed {seed:#018x}: killing three occupants cost no time at all"
            );
            verdict(&chaos, &killed, &calm)
                .unwrap_or_else(|why| panic!("seed {seed:#018x}: {why}"));
        }
    }

    #[test]
    fn every_kill_lands_in_the_middle_of_something() {
        // *Under sustained load* is the load-bearing half of the exit, and it is
        // a number rather than a hope: a kill that found an idle occupant was
        // rescheduled, and the fewest operations in flight at any kill that was
        // taken has to be at least one.
        for seed in SEEDS {
            let killed = report(&one("virtio-blk", "blk", KILLS), seed);
            assert_eq!(killed.kills, KILLS, "seed {seed:#018x}: a kill was abandoned");
            assert!(killed.flying_min >= 1, "seed {seed:#018x}: a kill landed between operations");
        }
    }

    #[test]
    fn a_connect_to_an_empty_place_pends_rather_than_failing() {
        // The mechanism gate G1's sentence rests on, exercised on every kill
        // rather than described: the client re-registers the instant it is told
        // its peer is gone, and that registration arrives while the place is
        // empty. RFC 0008 gives it three outcomes and this is the first.
        for seed in SEEDS {
            let killed = report(&one("virtio-blk", "blk", KILLS), seed);
            assert!(
                killed.pended >= KILLS,
                "seed {seed:#018x}: {} submission(s) pended against {KILLS} kill(s)",
                killed.pended
            );
            assert_eq!(killed.spawns, KILLS + 1, "a place was not refilled after a kill");
        }
    }

    #[test]
    fn one_seed_reproduces_its_sweep_and_a_different_seed_moves_it() {
        // `E1-P01`'s exit criterion, which this harness must not break: a chaos
        // run is a function of `(seed, commit)` like every other scenario, or a
        // failure it finds is a symptom rather than a bug report. The
        // cross-process form is `cargo xtask chaos`, which is comparable
        // evidence to two QEMU boots.
        let set = deployment();
        let first = sweep(&set, DEFAULT_SEED, KILLS).expect("a sweep");
        let second = sweep(&set, DEFAULT_SEED, KILLS).expect("a sweep");
        assert_eq!(digest(&first), digest(&second), "one seed produced two sweeps");

        let moved = (1..=8u64)
            .filter(|step| {
                sweep(&set, DEFAULT_SEED ^ step, KILLS).map(|pairs| digest(&pairs))
                    != Ok(digest(&first))
            })
            .count();
        assert_eq!(moved, 8, "a seed change the sweep did not feel");
    }

    #[test]
    fn the_backoff_ladder_is_the_manifests_and_not_this_files() {
        // The join. `user/virtio-blk/manifest.toml` declares ten milliseconds
        // doubling to a second; the fixture declares eight ticks doubling to
        // sixty-four, which is `user/store/manifest.toml`'s. Either way the
        // number this harness waits is the number the record carries, and a
        // manifest that changed it changes the run.
        let policy = policy_of(&component("store", "store", 16));
        assert_eq!(policy.backoff_ns(0), 8 * TICK_NS);
        assert_eq!(policy.backoff_ns(1), 16 * TICK_NS);
        assert_eq!(policy.backoff_ns(2), 32 * TICK_NS);
        assert_eq!(policy.backoff_ns(3), 64 * TICK_NS);
        // Capped, and it stays capped however far it is asked.
        assert_eq!(policy.backoff_ns(9), 64 * TICK_NS);
        assert_eq!(policy.ladder_ns(3), (8 + 16 + 32) * TICK_NS);
    }

    #[test]
    fn a_budget_that_runs_out_retires_the_place_rather_than_refilling_it() {
        // The far end of the policy, which the workload above never reaches
        // because `KILLS` is chosen under it. Driven directly, because a
        // retirement that only happens in a configuration nobody runs is a
        // branch nothing takes — and because a client meeting a retired place
        // is an outage rather than a restart, which is the one outcome the
        // verdict above refuses.
        let policy = policy_of(&component("store", "store", 16));
        let mut budget = Budget::default();
        let mut verdicts = Vec::new();
        let mut now = 0u64;
        for _ in 0..=policy.max_restarts {
            let verdict = policy.decide(&mut budget, now);
            if let Verdict::Restart(pause) = verdict {
                now = now.saturating_add(pause);
            }
            verdicts.push(verdict);
        }
        assert_eq!(verdicts.last(), Some(&Verdict::Retire), "the budget never ran out");
        // And the same count once the window has elapsed does not retire, which
        // is the difference between a budget and a lifetime cap.
        let after = policy.decide(
            &mut budget,
            now.saturating_add(u64::from(policy.budget_window_ticks) * TICK_NS),
        );
        assert!(
            matches!(after, Verdict::Restart(_)),
            "an elapsed window did not reopen the budget"
        );
    }

    #[test]
    fn a_sweep_over_no_components_is_refused() {
        // Fail closed, R04. An empty sweep produces a stable digest and no
        // evidence, which is the one result a check like this must never report
        // as a pass.
        let why = sweep(&Deployment::default(), DEFAULT_SEED, KILLS)
            .expect_err("nothing to kill is not a pass");
        assert!(why.contains("cargo xtask chaos"), "the refusal must say what to run");
    }

    #[test]
    fn every_label_fits_the_trace_column_and_no_two_share_a_word() {
        // A label wider than the column shifts every field after it, and the
        // value that shifts it is by definition the one nobody tested with.
        let labels = [
            PLACE,
            LOAD,
            kind_chaos::KILL,
            kind_chaos::REFILL,
            kind_chaos::FLUSH,
            wrote::KILLED,
            wrote::SPAWNED,
            wrote::PENDED,
            wrote::RESUMED,
            wrote::RETIRED,
            wrote::VOIDED,
            wrote::SETTLED,
            wrote::TWICE,
            wrote::STALE,
            wrote::WRONG,
            wrote::TORN,
            wrote::DROPPED,
            wrote::FAILED,
            wrote::CLOSED,
        ];
        for label in labels {
            assert!(
                label.len() <= crate::LABEL_WIDTH,
                "`{label}` is {} bytes and the column is {}",
                label.len(),
                crate::LABEL_WIDTH
            );
        }
        let mut sorted = labels.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "two labels share a spelling");
    }

    #[test]
    fn a_token_says_which_operation_and_which_half_of_it() {
        // The ledger's whole arithmetic, and the one place a mistake would make
        // *answered twice* and *answered once* the same observation.
        for op_number in [0u32, 1, 47, u32::MAX] {
            for which in [half::WRITE, half::READ, half::REGISTER] {
                let token = token(3, which, op_number);
                assert_eq!(op(token), op_number);
                assert_eq!(phase(token), which);
            }
        }
        assert_ne!(value(0), value(1), "two operations wrote one value");
        assert_ne!(value(0), NOTHING, "a written value is indistinguishable from an empty read");
    }
}
