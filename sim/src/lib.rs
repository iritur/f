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
//! # A fault class states its response before it is injected
//!
//! [`fault`] is `E1-P02`: seven classes, each a site a model consults, a
//! scenario that arms it, and an assertion that says in advance what the system
//! must do. A scenario that injected a fault and printed what happened would be
//! an observation, and an observation needs a reader; the assertion is what
//! makes it a check. The plan is a field of the scenario rather than a flag, so
//! a reproduction command is still a name and a seed.
//!
//! Fault draws are keyed at [`decide::domain::FAULTS`], which `E1-P01` reserved
//! for exactly this and which is why arming a class cannot move an interleaving
//! a recorded seed had already selected.
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
pub mod chaos;
pub mod check;
pub mod client;
pub mod decide;
pub mod deploy;
pub mod dev;
pub mod fault;
pub mod gpu;
pub mod native;
pub mod net;
pub mod proto;
pub mod reserve;
pub mod scenario;
pub mod service;
pub mod snap;
pub mod sweep;
pub mod time;
pub mod trace;
pub mod virtq;
pub mod wire;

use f_env::{Env, Instant, Scheduler, WallSource, WallTime, split};

use snap::{Broken, Reader, Writer};

use decide::Decisions;
use fault::{Class, Fault, Injection, Injector};
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

/// The site an unlabelled decision — one taken through `f_env::Scheduler` — is
/// recorded at.
///
/// A constant rather than a literal at the one call site, because `E1-P08`
/// writes decision sites into a snapshot by index into `snap::LABELS` and a
/// label that exists only as a literal is a label somebody deletes from the
/// table without the compiler noticing.
pub const ENV_CHOOSE: &str = "env.choose";

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

    /// Write this actor's whole state into a snapshot, its `snap::tag` first.
    ///
    /// # Why the default refuses instead of writing nothing
    ///
    /// `E1-P08` re-enters a run from a file, and the property that makes such a
    /// file worth having is that the restored run is *indistinguishable* from
    /// the run that replayed. An actor that wrote nothing would produce a file
    /// that loads, restores into a world missing a participant, and diverges
    /// plausibly — which is worse than no snapshot at all, because it sends
    /// somebody looking for a bug at a point the system never reached.
    ///
    /// So the default is [`snap::Broken::Unsaveable`], naming the actor. A crate
    /// that has taught some of its actors to save and not others refuses to
    /// snapshot the runs that contain the others, by name, and says which. R04.
    ///
    /// It is a default rather than a required method because `Actor` is
    /// implemented outside the set of things a scenario installs — `chaos.rs`
    /// wraps actors in places, and a test installs a stub — and requiring the
    /// method would make *every* implementor owe a format. What keeps the
    /// shipped path honest is that `snap`'s tests snapshot every scenario in
    /// both tables at several cuts, so an actor that reaches a scenario without
    /// a save turns the suite red.
    ///
    /// # Errors
    ///
    /// [`snap::Broken::Unsaveable`] by default; otherwise whatever the writer
    /// refused with.
    fn save(&self, out: &mut Writer) -> Result<(), Broken> {
        let _ = out;
        Err(Broken::Unsaveable(self.name()))
    }
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
    /// What this run breaks, and how often.
    ///
    /// Beside the decisions rather than inside them, because the two answer
    /// different questions from different domains: `decide` chooses between
    /// things that could both happen, and this decides whether one of them
    /// happens at all. [`fault`] argues the separation and `decide::domain` is
    /// where the two streams are kept apart.
    faults: Injector,
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
            faults: Injector::new(seed),
        }
    }

    /// Arm the fault classes this run injects.
    ///
    /// Called by the scenario before a single actor is installed, so that a
    /// class is armed for the whole run or for none of it. There is deliberately
    /// no way to arm one part-way through: a plan that changed under a run would
    /// make the run a function of something other than `(seed, commit)`, which
    /// is the one property everything above this rests on.
    pub fn arm(&mut self, plan: &'static [Injection]) {
        self.faults.arm(plan);
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

    /// Consult one fault class, and write down what it answered.
    ///
    /// The third thing a world hands out, and the third stream: a class draws at
    /// [`decide::domain::FAULTS`], so consulting one cannot move an interleaving
    /// decision or a service time, and adding a class cannot move another
    /// class's answers. [`fault`] is where that is argued and tested.
    ///
    /// A strike is **recorded**, hashed with the rest of the artefact. A
    /// simulator that broke something quietly would produce a trace that
    /// reproduces perfectly and describes a run nobody can reason about — the
    /// same argument `dev.rs` makes for writing a dropped completion down. It is
    /// also what `E1-P03` reads to say what a failing seed injected: the class,
    /// the operation it struck, and which consultation of that class it was.
    ///
    /// `who` and `actor` are the model that consulted, not the injector, because
    /// a fault happens somewhere and a record that did not say where would send
    /// a reader looking through the whole trace for it.
    pub fn strike(&mut self, who: ActorId, class: Class, token: u64) -> Option<Fault> {
        let occurrence = self.faults.consulted(class);
        let fault = self.faults.strike(class)?;
        self.record(who, fault::ACTOR, class.label(), token, occurrence);
        Some(fault)
    }

    /// How many faults this run injected. Unit: faults.
    #[must_use]
    pub const fn injected(&self) -> u32 {
        self.faults.struck()
    }

    /// What this run is armed with. Unit: see [`fault::Injection`].
    #[must_use]
    pub const fn plan(&self) -> &'static [Injection] {
        self.faults.plan()
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
    ///
    /// The *log*, which after a terse restore begins part-way through the run —
    /// [`World::decided`] is the run's own count and is the number to compare
    /// two runs by. `decide::Decisions::carried` says why the two differ.
    #[must_use]
    pub fn decisions(&self) -> &[decide::Decision] {
        self.decisions.log()
    }

    /// How many decisions this run has taken. Unit: decisions.
    #[must_use]
    pub fn decided(&self) -> u32 {
        self.decisions.taken()
    }

    /// The artefact so far.
    #[must_use]
    pub fn trace(&self) -> &Trace {
        &self.trace
    }
}

// ---- E1-P08: a world written out and re-entered ---------------------------
//
// The save and the load are here rather than in `snap.rs` because these fields
// are private to this file, and a snapshot that reached them through accessors
// would mean six accessors nothing else in the crate wants. Each is one
// statement per field, in one order, next to the fields — which is the
// arrangement that makes a *new* field's author see the two lines they owe.

impl World {
    /// Write this world's whole state.
    ///
    /// `terse` decides whether the artefact so far travels with it or only its
    /// running hash — `trace::Carried` is where the trade is argued and RFC 0043
    /// is where it is measured. Everything else travels either way, because
    /// everything else is what the *run* is and the artefact is what it wrote.
    pub(crate) fn save(&self, out: &mut Writer, terse: bool) {
        out.u64(self.seed);
        self.line.save(out);
        self.decisions.save(out, terse);
        // The one chain in a crate of derivations. `env/src/split.rs` argues why
        // this needs five words where the ordering and fault streams need only a
        // counter each.
        for word in self.random.state() {
            out.u64(word);
        }
        self.faults.save(out);
        self.wire.save(out);
        self.trace.save(out, terse, self.line.clock());
    }

    /// Read one back.
    ///
    /// `seed` comes from the snapshot's header rather than from here, and the
    /// world's own copy is checked against it: two numbers that must agree, in
    /// a file somebody could have edited, are two chances to notice.
    pub(crate) fn load(input: &mut Reader<'_>, seed: u64) -> Self {
        if input.u64() != seed {
            input.refuse(Broken::Diverged("the world's seed and the header's"));
        }
        let line = Timeline::load(input);
        let decisions = Decisions::load(input, seed);
        let state = [input.u64(), input.u64(), input.u64(), input.u64(), input.u64()];
        let faults = Injector::load(input, seed);
        let wire = Wire::load(input);
        let trace = Trace::load(input);
        Self {
            line,
            decisions,
            trace,
            random: split::Stream::from_state(state),
            seed,
            wire,
            faults,
        }
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
        self.decide(ENV_CHOOSE, n)
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
    /// Faults injected. Unit: faults.
    ///
    /// Reported beside a failure so a seed's severity is visible without reading
    /// the artefact — the same number `f_env::sim::SimEnv::injected` answers on
    /// the other side of the tree. Zero for every scenario that arms nothing,
    /// which is most of them.
    pub injected: u32,
}

impl Outcome {
    /// The one number two runs are compared by.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.trace.digest()
    }
}

/// Where a run is asked to stop.
///
/// A cut always falls **between two steps**, which is the only place a
/// simulation has a well-defined state: [`Simulation::run_to`] takes one
/// message, hands it to one actor and returns, so between two of those nothing
/// holds a borrow and nothing is half-written. `E1-P08` is why this exists and
/// [`snap`] is where the argument is made in full.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cut {
    /// Nowhere: run until there is nothing left to do.
    Never,
    /// Before the first step whose message is due at or after this instant.
    /// Unit: nanoseconds on the simulator's clock.
    ///
    /// The unit a bisect is asked in — *minute 39* — and the reason a cut is
    /// expressed in the model's own time rather than only in steps: a person
    /// looking at a failure knows when it happened and not how many messages it
    /// took to get there.
    Clock(u64),
    /// Before this many messages have been delivered. Unit: steps.
    Steps(u32),
}

impl Cut {
    /// Should the run stop now, given how many steps it has taken and when the
    /// next message is due?
    #[must_use]
    const fn reached(self, steps: u32, due: Option<u64>) -> bool {
        match self {
            Self::Never => false,
            Self::Clock(at) => match due {
                Some(next) => next >= at,
                None => false,
            },
            Self::Steps(count) => steps >= count,
        }
    }
}

/// What a run answered when it was asked to stop somewhere.
pub enum Halt {
    /// It ran out of work before reaching the cut.
    ///
    /// Boxed because an [`Outcome`] carries a whole trace and the other variant
    /// carries a simulation; an enum as large as its largest variant would make
    /// every `run_to` move a trace-sized value.
    Finished(Box<Outcome>),
    /// It reached the cut. Here is the world at it, ready to be written out or
    /// to carry on.
    ///
    /// Boxed for the same reason the other variant is: a [`Simulation`] carries
    /// a world and a vector of actors, and an enum as large as its largest
    /// variant would make every `run_to` move all of it.
    Paused(Box<Simulation>),
}

/// A world, the actors in it, and the loop that steps them.
pub struct Simulation {
    world: World,
    actors: Vec<Box<dyn Actor>>,
    budget: u32,
    /// Messages delivered so far. Unit: steps.
    ///
    /// A field rather than a local of the run loop, because `E1-P08` stops the
    /// loop and starts it again and the count has to survive the gap: an
    /// [`Outcome`] reports the steps of the *run*, and a run that was paused
    /// twice is still one run.
    steps: u32,
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
        Self { world: World::new(seed), actors: Vec::new(), budget, steps: 0 }
    }

    /// A simulation at a point a snapshot recorded, with no actors in it yet.
    ///
    /// The caller installs the saved actors in the order they were saved, which
    /// is the order they were installed, which is what makes an [`ActorId`] mean
    /// the same thing after a restore as before it. [`snap::restore`] is the
    /// only caller and the only one there should be.
    #[must_use]
    pub fn resume(world: World, steps: u32, budget: u32) -> Self {
        Self { world, actors: Vec::new(), budget, steps }
    }

    /// The world, to read.
    #[must_use]
    pub const fn world_ref(&self) -> &World {
        &self.world
    }

    /// Every actor, in installation order.
    #[must_use]
    pub fn actors(&self) -> &[Box<dyn Actor>] {
        &self.actors
    }

    /// Messages delivered so far. Unit: steps.
    #[must_use]
    pub const fn steps(&self) -> u32 {
        self.steps
    }

    /// The bound this run is under. Unit: steps.
    #[must_use]
    pub const fn budget(&self) -> u32 {
        self.budget
    }

    /// When the next message is due, if there is one. Unit: nanoseconds.
    ///
    /// Read by a caller placing cuts along a run's simulated time, so the next
    /// cut can be put past the instant this one stopped at rather than at a
    /// boundary the clock has already passed.
    #[must_use]
    pub fn next_ns(&self) -> Option<u64> {
        self.world.line.peek()
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
    pub fn run(self) -> Result<Outcome, Trouble> {
        match self.run_to(Cut::Never)? {
            Halt::Finished(outcome) => Ok(*outcome),
            // Unreachable: `Cut::Never` is never reached. A branch and not an
            // expectation, because a panic here would be a panic in every caller
            // of the simulator, and the honest answer to *the loop stopped for a
            // reason that cannot happen* is the outcome it had.
            Halt::Paused(paused) => Ok((*paused).finished()),
        }
    }

    /// The same, stopping at `cut`.
    ///
    /// The loop `run` is: one message, one actor, return. `cut` is consulted
    /// **before** each step, and against the instant the next message is due, so
    /// a pause falls between two steps and never inside one — which is what
    /// makes the paused world exactly the sum of its parts and therefore
    /// writable. [`snap`] is where that is argued.
    ///
    /// # Errors
    ///
    /// As [`Simulation::run`]. A budget that runs out is still a refusal when
    /// the run was going to be paused later: the bound is on the whole run and
    /// not on one leg of it.
    pub fn run_to(mut self, cut: Cut) -> Result<Halt, Trouble> {
        while !self.world.line.idle() {
            if cut.reached(self.steps, self.world.line.peek()) {
                return Ok(Halt::Paused(Box::new(self)));
            }
            if self.steps >= self.budget {
                return Err(Trouble::Budget(self.budget));
            }
            let Some(pending) = self.world.line.next(&mut self.world.decisions) else {
                break;
            };
            self.steps = self.steps.saturating_add(1);

            // Two disjoint fields borrowed at once, which is the whole reason
            // the actors live beside the world rather than inside it.
            let index = pending.to.0 as usize;
            let Some(actor) = self.actors.get_mut(index) else {
                return Err(Trouble::NoSuchActor(pending.to.0));
            };
            actor.deliver(&mut self.world, pending.to, pending.message);
        }

        Ok(Halt::Finished(Box::new(self.finished())))
    }

    /// What this run leaves behind.
    fn finished(self) -> Outcome {
        let log = self.world.decisions.log().to_vec();
        Outcome {
            seed: self.world.seed,
            steps: self.steps,
            decisions: self.world.decisions.taken(),
            finished_ns: self.world.line.clock(),
            trace: self.world.trace,
            log,
            injected: self.world.faults.struck(),
        }
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
