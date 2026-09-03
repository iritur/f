// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The deterministic simulator: layer 1 of `docs/design/proving-ground.html`.
//!
//! # What this simulates, and what it does not
//!
//! It simulates the system **above the frame**: components, the rings between
//! them, and the devices at the far end of those rings. It does not simulate the
//! frame's own instructions, and RFC 0032 is where that choice is argued, priced
//! and given a reversal condition. The one-line version: `kernel/Cargo.toml`
//! sets `test = false` and says why — the kernel is `no_std` with its own panic
//! handler, and a host harness links `std`, so two crates would claim the
//! `panic_impl` lang item — and the answer to *did two runs of the frame do the
//! same thing* already exists as `cargo xtask trace --hash`, which boots the
//! real kernel in QEMU twice. Building a second answer to a question that has
//! one is how a project ends up unable to say which reproduction story a failure
//! belongs to.
//!
//! So the seam is stated rather than glossed: **`cargo xtask trace` is the
//! boot's reproduction check and `cargo xtask sim` is the workload's**, and they
//! share a hash function, a printed form and a default seed.
//!
//! # What *boot-to-workload* means here, exactly
//!
//! RFC 0032 named the join and left it unbuilt; RFC 0035 builds it and is where
//! the definition below is argued. In one paragraph, so that nobody has to
//! reconstruct it from two commands:
//!
//! A **boot-to-workload run** is a pair of runs over one component set, at one
//! commit. `cargo xtask trace --hash` boots the real kernel in QEMU, which
//! spawns components from the compiled manifest records the loader hands it, and
//! hashes the log — the log in which each component's content hash is printed.
//! `cargo xtask sim --hash deployment` reads *those same component files*
//! ([`deploy`]), builds the actors the records declare, drives a workload
//! through them, and hashes its own artefact. The two artefacts quote the same
//! content hashes, and `cargo xtask sim --join` is the command that requires
//! them to. What that pair does **not** claim is that one process executed both
//! halves: the frame's instructions run only in QEMU, and every trace this crate
//! writes says so in its own header.
//!
//! An artefact that did not carry that sentence would be quoted later as
//! covering more than it does, which is why the coverage is in the hashed bytes
//! rather than in a document beside them.
//!
//! # The three things a simulator has to own
//!
//! **Virtual time.** [`time::Timeline`] holds the clock. Nothing in this crate
//! reads a host clock, and there is no interface here that could: the only way
//! to put work into the future is a *delay*, so the clock cannot be sent
//! backwards and cannot be read from anywhere else. `cargo xtask
//! lint-determinism` passes over this crate with no allow-list entry.
//!
//! **Seeded ordering.** Every point where two things could happen in either
//! order is a call into [`decide::Decisions`], which draws from a stream keyed
//! by the run's seed, the decision's *site*, and that site's own occurrence
//! count — the derivation RFC 0026 built and `env/src/split.rs` implements.
//! Every decision is written down under two names, because `E1-P03` has to
//! minimise a failure to a reproduction command and `E1-P08` has to re-enter a
//! run at a point. [`decide`] is where that is argued.
//!
//! **An artefact.** A run produces a [`trace::Trace`], the trace is text, and
//! the text has one hash. Same seed, same hash; different seed, different hash;
//! and a test asserts both, because a reproduction check nobody has watched fail
//! is indistinguishable from one that cannot.
//!
//! # An actor is delivered to, and that is not a callback
//!
//! R05 says nothing is delivered asynchronously: every event is a ring entry
//! drained at a polling point. [`Simulation::run`] *is* the polling point. It
//! takes one message from the timeline, hands it to one actor, and returns; no
//! actor registers anything, nothing is called from inside anything else, and
//! there is no second path in. That is R05's shape rather than an exception to
//! it — and it is why the simulator can claim that what it explores is the set
//! of orderings the system permits rather than the set its runtime happened to
//! produce.
//!
//! # The substitution seam, and where it is visible
//!
//! [`Actor`] is the seam. A device model and a component with no device behind
//! it are both implementations of it, chosen by the scenario at construction,
//! and the client code above does not change — which is the property user-space
//! drivers were supposed to buy and this is where it gets collected.
//! `docs/design/proving-ground.html` states it as *hardware already sits behind
//! a component boundary, so a simulated device is a component substitution
//! rather than a kernel patch*, and that sentence is a promise about the client
//! rather than about the device.
//!
//! There is therefore exactly one client — [`client::App`] — and four things it
//! can be pointed at: [`blk`], [`net`] and [`gpu`], which are virtqueues with
//! protocols on them, and [`native`], which is `f_ring::registry` with nothing
//! underneath. [`scenario::Peer`] is where the choice is made, so a reader
//! looking for *which peer is this scenario about* finds it in the scenario
//! table and not in the client.
//!
//! Underneath them: [`virtq`] is the split virtqueue, laid out the way
//! `kernel/src/arch/x86_64/dma.rs` and `user/virtio-blk/src/queue.rs` lay theirs
//! out; [`dev`] is the machinery every device shares, including the completion
//! policy the seed drives; [`service`] is the *real* `f_ring::registry::Table`
//! and a modelled IOMMU under it; and [`wire`] carries real [`f_abi::Sqe`] and
//! [`f_abi::Cqe`] between actors, because a `Message` is the occurrence of a
//! submission and not the submission itself.
//!
//! [`actors`] is what stage one shipped: a client and a bounded queue, with no
//! virtqueue and no ABI entries. It stays because it is what established that
//! the machinery underneath carries a real exchange, and its three scenarios are
//! that claim's evidence. It is not the client the substitution property is
//! about.

pub mod actors;
pub mod blk;
pub mod client;
pub mod decide;
pub mod deploy;
pub mod dev;
pub mod gpu;
pub mod native;
pub mod net;
pub mod proto;
pub mod scenario;
pub mod service;
pub mod time;
pub mod trace;
pub mod virtq;
pub mod wire;

use f_env::{Env, Instant, Scheduler, WallSource, WallTime, split};

use decide::Decisions;
use time::Timeline;
use trace::{Record, Trace};
use wire::Wire;

/// The seed every reproduction check in this tree uses, unless told otherwise.
///
/// The same value `xtask`'s `TRACE_SEED` names, deliberately and not by
/// coincidence: the contract is about a *pair*, `(seed, commit)`, and a tree
/// whose two reproduction checks quoted two different seeds would be a tree
/// where a person has to ask which pair a hash belongs to. `cargo xtask sim`
/// passes this value explicitly, which is what keeps the two constants equal.
pub const DEFAULT_SEED: u64 = 0xf00d_beef_cafe_1234;

/// The identity the randomness stream is derived at.
///
/// Its own stream, split from the seed by identity, so that drawing a value
/// cannot move an interleaving decision and an interleaving decision cannot move
/// a value. That is the same independence `decide::draw` gives sites and
/// `env/src/sim.rs` gives fault sites, applied to the two things a run hands
/// out.
const RANDOM: &str = "sim.random";

/// The identity the simulated wall clock is derived at.
const WALL: &str = "sim.wall";

/// Who a message is for, or from.
///
/// An index into the simulation's actor list, in installation order. Not a
/// pointer and never a pointer: an address is a source of nondeterminism that no
/// lint can see, and the whole crate is about not having any.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ActorId(pub u32);

/// What one actor hands another.
///
/// Deliberately small and untyped. A message is not an `f_abi::Sqe`: it is
/// the *occurrence* of one, and giving it the wire type would mean the
/// simulator's ordering machinery grew an opinion about the ABI. Stage 2's
/// device models carry a submission in `token` and `detail` and interpret them
/// per `kind`, exactly as an opcode space is per service.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Message {
    /// Who sent it. Unit: none — an actor index. The sender and the recipient
    /// together are the *channel*, and `time.rs` explains what that buys.
    pub from: ActorId,
    /// What it says. Unit: none — a stable label the recipient matches on.
    pub kind: &'static str,
    /// Which operation it concerns. Unit: none — an opaque token minted by the
    /// client that issued the operation.
    pub token: u64,
    /// Whatever the kind says this holds. Unit: per-kind; the actor that sends
    /// a kind is what states it.
    pub detail: u64,
}

/// Something the simulation steps.
///
/// The substitution seam: a modelled device and a real component are two
/// implementations of this, and the scenario picks which one is installed. See
/// the crate documentation for why a `deliver` method is not a callback.
pub trait Actor {
    /// What kind of thing this is, for the trace. Unit: none — a stable label,
    /// at most [`LABEL_WIDTH`] bytes so that a trace's columns cannot move.
    fn name(&self) -> &'static str;

    /// Take one message. The only entry point.
    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message);
}

/// The widest an actor name or a message kind may be, in bytes.
///
/// A trace line is fixed-width so that two otherwise identical runs cannot
/// disagree because one of them had a longer label in it. This is the bound the
/// format in `trace.rs` reserves, and `actors.rs` has a test that every label
/// this crate ships fits inside it.
pub const LABEL_WIDTH: usize = 8;

/// Everything an actor can reach: the clock, the seed, the queue and the trace.
///
/// It is also an [`Env`], which is the other half of the substitution seam —
/// code written against `&mut dyn Env` runs unchanged whether the environment is
/// this one or the kernel's.
pub struct World {
    line: Timeline,
    decisions: Decisions,
    trace: Trace,
    random: split::Stream,
    seed: u64,
    /// The ABI entries in flight between actors.
    ///
    /// Here rather than inside an actor because a ring is memory *two*
    /// components share, and there is nowhere else in this simulator that two
    /// actors can both reach. [`wire`] argues the choice and states what it is
    /// not: it is not `f_ring::Producer` over a real mapping, and `E1-P04` is
    /// the task that would make it one.
    wire: Wire,
}

impl World {
    /// A world at time zero, driven by `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            line: Timeline::new(),
            decisions: Decisions::new(seed),
            trace: Trace::new(),
            random: split::Stream::from_seed(split::derive(seed, split::label(RANDOM))),
            seed,
            wire: Wire::new(),
        }
    }

    /// The virtual clock. Unit: nanoseconds since the start of the run.
    #[must_use]
    pub fn clock(&self) -> u64 {
        self.line.clock()
    }

    /// The seed this run was driven by. Unit: none — half of the `(seed,
    /// commit)` pair a reproduction quotes.
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Send a message, to arrive `delay_ns` nanoseconds from now.
    ///
    /// A delay of zero is the same instant, and the same instant is where the
    /// interleaving decisions come from.
    pub fn send(&mut self, delay_ns: u64, to: ActorId, message: Message) {
        self.line.send(delay_ns, to, message);
    }

    /// The entries in flight between actors.
    ///
    /// A submission is put here and a [`Message`] is sent to say so, which is
    /// the doorbell and the shared memory kept apart exactly as `f_ring` keeps
    /// them apart. See [`wire`] for why a `Message` does not carry the entry.
    pub fn wire(&mut self) -> &mut Wire {
        &mut self.wire
    }

    /// Choose among `arity` alternatives at `site`, and write the choice down.
    ///
    /// Every point in a model where two things could happen in either order goes
    /// through here. `site` is a stable label so that a failing seed names where
    /// it struck, which is the same discipline `f_env::sim::Faults` applies to
    /// fault injection and for the same reason.
    pub fn decide(&mut self, site: &'static str, arity: u32) -> u32 {
        let at = self.line.clock();
        self.decisions.decide(at, site, arity)
    }

    /// The next value from the run's randomness stream.
    ///
    /// Distinct from [`World::decide`]: this is a *quantity* a model needs — a
    /// service time, a payload length — rather than a choice between
    /// alternatives. They draw from different streams so that adding one cannot
    /// move the other.
    pub fn draw(&mut self) -> u64 {
        self.random.next_u64()
    }

    /// State something this run covers, in the artefact itself.
    ///
    /// Written before the run starts and hashed with everything after it, so
    /// that what a trace covers travels with the trace. [`trace`] is where the
    /// rule that the *seed* never appears here is argued, and it is the one rule
    /// about this header that is load-bearing rather than tidy.
    pub fn cover(&mut self, line: &str) {
        self.trace.cover(line);
    }

    /// Write one line into the artefact.
    pub fn record(
        &mut self,
        who: ActorId,
        actor: &'static str,
        kind: &'static str,
        token: u64,
        detail: u64,
    ) {
        let at_ns = self.line.clock();
        self.trace.push(Record { at_ns, who: who.0, actor, kind, token, detail });
    }

    /// Every decision this run has taken, in order.
    #[must_use]
    pub fn decisions(&self) -> &[decide::Decision] {
        self.decisions.log()
    }

    /// The artefact so far.
    #[must_use]
    pub fn trace(&self) -> &Trace {
        &self.trace
    }
}

impl Scheduler for World {
    /// The unlabelled decision.
    ///
    /// `f_env::Scheduler::choose` takes no site, because it is the interface the
    /// *system* is written against and the system does not know it is being
    /// simulated. So it is recorded at one site of its own rather than being
    /// dropped: a decision that reached the model through the trait is still a
    /// decision a minimiser has to be able to name, and `env.choose` is its
    /// name. A model that wants to be aimed at calls [`World::decide`] with a
    /// site of its own.
    fn choose(&mut self, n: u32) -> u32 {
        self.decide("env.choose", n)
    }
}

impl Env for World {
    /// The virtual clock.
    ///
    /// It does **not** move when a value is drawn, which is where this
    /// environment differs from `f_env::SeededEnv`, and the difference is
    /// forced rather than chosen: the timeline sets the clock to each message's
    /// instant, so a clock that also advanced under draws would be moved
    /// *backwards* by the next dispatch. `env/src/contract.rs` assumes a clock
    /// that advances either by being used or on its own, and a discrete-event
    /// clock is neither — the test
    /// `the_env_contract_assumes_a_clock_this_one_is_not` in `scenario.rs`
    /// records that gap rather than hiding it.
    fn now(&self) -> Instant {
        Instant(self.line.clock())
    }

    fn next_u64(&mut self) -> u64 {
        self.draw()
    }

    fn wall(&self) -> Option<WallTime> {
        // Derived from the seed rather than read from anywhere, so a run that
        // stamps a wall-clock time is still byte-reproducible, and derived at
        // its own identity so the number does not depend on how many values the
        // run happened to draw first. `f_env::SeededEnv::wall` makes the same
        // two choices and states the same reason.
        Some(WallTime {
            tai_nanos: self
                .line
                .clock()
                .wrapping_mul(3)
                .wrapping_add(split::derive(self.seed, split::label(WALL))),
            uncertainty_nanos: 1_000_000_000,
            source: WallSource::Simulated,
        })
    }

    fn scheduler(&mut self) -> &mut dyn Scheduler {
        self
    }
}

/// Why a run stopped without finishing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// The step budget ran out. A run that does not end is a scenario that does
    /// not terminate, and reporting it is the difference between a bug and a
    /// hang. Carries the budget, in steps.
    Budget(u32),
    /// A message named an actor the simulation does not have. Fail closed: the
    /// alternative is a message quietly going nowhere, which is a run that looks
    /// shorter than it is. Carries the index that was named.
    NoSuchActor(u32),
    /// The deployment scenario was asked to run over no components at all.
    ///
    /// Fail closed rather than running an empty world: a run with nothing in it
    /// produces a short trace and a perfectly stable digest, which is the one
    /// result a reproduction check must never report as a pass. The component
    /// set is built by `cargo xtask component` and read by [`deploy`].
    NeedsDeployment,
}

impl Trouble {
    /// A sentence for a report.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            Self::Budget(budget) => {
                format!("the run did not finish inside {budget} steps")
            }
            Self::NoSuchActor(id) => format!("a message named actor {id}, which does not exist"),
            Self::NeedsDeployment => concat!(
                "this scenario runs the component set the boot spawns, and no component files ",
                "were read. `cargo xtask sim` builds them first."
            )
            .to_string(),
        }
    }
}

/// What a finished run leaves behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The seed it was driven by. Unit: none.
    pub seed: u64,
    /// Messages delivered. Unit: steps.
    pub steps: u32,
    /// Interleaving decisions taken. Unit: decisions.
    pub decisions: u32,
    /// Where the clock stopped. Unit: nanoseconds.
    pub finished_ns: u64,
    /// The artefact.
    pub trace: Trace,
    /// Every decision, in order, so `E1-P03` can shrink and `E1-P08` can
    /// re-enter.
    pub log: Vec<decide::Decision>,
}

impl Outcome {
    /// The one number two runs are compared by.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.trace.digest()
    }
}

/// A world, the actors in it, and the loop that steps them.
pub struct Simulation {
    world: World,
    actors: Vec<Box<dyn Actor>>,
    budget: u32,
}

impl Simulation {
    /// A simulation with nothing in it.
    ///
    /// `budget` bounds the run in steps. It is a bound on *messages delivered*
    /// and not on simulated time, because a scenario that fails to terminate
    /// usually does so by exchanging messages at one instant rather than by
    /// running long.
    #[must_use]
    pub fn new(seed: u64, budget: u32) -> Self {
        Self { world: World::new(seed), actors: Vec::new(), budget }
    }

    /// Put an actor in, and answer the id everything else addresses it by.
    pub fn install(&mut self, actor: Box<dyn Actor>) -> ActorId {
        let id = u32::try_from(self.actors.len()).unwrap_or(u32::MAX);
        self.actors.push(actor);
        ActorId(id)
    }

    /// The world, so a scenario can post the messages that start the run.
    pub fn world(&mut self) -> &mut World {
        &mut self.world
    }

    /// Run until nothing is due, or until the budget runs out.
    ///
    /// # Errors
    ///
    /// [`Trouble::Budget`] if the run does not finish, [`Trouble::NoSuchActor`]
    /// if a message names an actor that was never installed. Both are refusals
    /// rather than best-effort continuations: a simulator that quietly dropped a
    /// message would produce a trace that reproduces perfectly and describes a
    /// system nobody built.
    pub fn run(mut self) -> Result<Outcome, Trouble> {
        let mut steps: u32 = 0;
        while !self.world.line.idle() {
            if steps >= self.budget {
                return Err(Trouble::Budget(self.budget));
            }
            let Some(pending) = self.world.line.next(&mut self.world.decisions) else {
                break;
            };
            steps = steps.saturating_add(1);

            // Two disjoint fields borrowed at once, which is the whole reason
            // the actors live beside the world rather than inside it.
            let index = pending.to.0 as usize;
            let Some(actor) = self.actors.get_mut(index) else {
                return Err(Trouble::NoSuchActor(pending.to.0));
            };
            actor.deliver(&mut self.world, pending.to, pending.message);
        }

        Ok(Outcome {
            seed: self.world.seed,
            steps,
            decisions: self.world.decisions.taken(),
            finished_ns: self.world.line.clock(),
            trace: self.world.trace,
            log: self.world.decisions.log().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An actor that says nothing and schedules nothing, so a test can exercise
    /// the loop without a scenario.
    struct Quiet;

    impl Actor for Quiet {
        fn name(&self) -> &'static str {
            "quiet"
        }
        fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
            world.record(me, self.name(), message.kind, message.token, 0);
        }
    }

    /// An actor that sends itself a message forever, which is what a scenario
    /// that does not terminate looks like from in here.
    struct Forever;

    impl Actor for Forever {
        fn name(&self) -> &'static str {
            "forever"
        }
        fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
            world.send(1, me, Message { from: me, ..message });
        }
    }

    fn poke(sim: &mut Simulation, to: ActorId) {
        sim.world().send(0, to, Message { from: to, kind: "poke", token: 0, detail: 0 });
    }

    #[test]
    fn an_empty_run_finishes_at_time_zero() {
        let outcome = Simulation::new(1, 10).run().expect("nothing to do is not a failure");
        assert_eq!((outcome.steps, outcome.finished_ns), (0, 0));
        assert!(outcome.trace.is_empty());
    }

    #[test]
    fn a_message_to_an_actor_that_does_not_exist_is_refused() {
        // Fail closed, R04. The alternative is a message quietly going nowhere,
        // which is a trace that reproduces perfectly and describes nothing.
        let mut sim = Simulation::new(1, 10);
        let real = sim.install(Box::new(Quiet));
        assert_eq!(real, ActorId(0));
        sim.world().send(0, ActorId(7), Message { from: real, kind: "poke", token: 0, detail: 0 });
        assert_eq!(sim.run(), Err(Trouble::NoSuchActor(7)));
    }

    #[test]
    fn a_run_that_does_not_end_is_reported_rather_than_waited_on() {
        let mut sim = Simulation::new(1, 64);
        let id = sim.install(Box::new(Forever));
        poke(&mut sim, id);
        assert_eq!(sim.run(), Err(Trouble::Budget(64)));
    }

    #[test]
    fn the_world_is_an_env_and_its_clock_does_not_move_when_a_value_is_drawn() {
        // The property that forces this environment to differ from
        // `f_env::SeededEnv`: the timeline owns the clock, so a draw that moved
        // it would be moved backwards by the next dispatch.
        let mut world = World::new(0x5EED);
        let before = world.now();
        for _ in 0..64 {
            let _ = world.next_u64();
        }
        assert_eq!(world.now(), before, "a draw advanced a clock the timeline owns");
    }

    #[test]
    fn the_env_seams_are_seeded_like_everything_else() {
        let mut a = World::new(11);
        let mut b = World::new(11);
        let other = World::new(12);
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
            assert_eq!(a.scheduler().choose(7), b.scheduler().choose(7));
        }
        assert_eq!(a.wall(), b.wall(), "a seed must reproduce its wall clock");
        assert_ne!(a.wall(), other.wall(), "two seeds shared a wall clock");
        let stamp = a.wall().expect("a simulated run has a wall clock");
        assert_eq!(stamp.source, WallSource::Simulated);
    }

    #[test]
    fn a_choice_made_through_the_env_trait_is_still_written_down() {
        // `Scheduler::choose` carries no site, and a decision that is not
        // written down cannot be minimised to. It gets a site of its own rather
        // than being dropped.
        let mut world = World::new(3);
        for _ in 0..10 {
            let _ = world.scheduler().choose(4);
        }
        assert_eq!(world.decisions().len(), 10);
        assert!(world.decisions().iter().all(|d| d.site == "env.choose"));
    }

    #[test]
    fn drawing_and_deciding_do_not_move_each_other() {
        // The independence RFC 0026 is about, at the two things a world hands
        // out. If they shared a stream, adding a decision point anywhere would
        // change every service time after it.
        let quiet = {
            let mut world = World::new(0xF00D);
            (0..16).map(|_| world.draw()).collect::<Vec<_>>()
        };
        let busy = {
            let mut world = World::new(0xF00D);
            (0..16)
                .map(|_| {
                    let _ = world.decide("noise", 5);
                    world.draw()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(quiet, busy, "a decision moved the randomness stream");
    }
}
