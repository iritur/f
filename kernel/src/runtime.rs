// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cores allocated to runtimes, preemption at allocation boundaries, and the
//! counter that says what crossed.
//!
//! # The sentence this file implements
//!
//! `deadline-all-the-way-down` section 02: *the kernel does not schedule tasks.
//! It allocates cores to runtimes, and a runtime schedules its own work inside
//! that allocation with no kernel involvement — the reason async work never
//! crosses a boundary. The kernel preempts only at allocation boundaries and on
//! reservation expiry, and it notifies a runtime when it is about to lose a core
//! so the runtime can park cleanly rather than being interrupted mid-task.*
//!
//! Every clause of that is a thing this file either does or refuses to do.
//! **Allocates cores to runtimes** is [`Allocation`], which is a set of cores
//! and a reclaim promise per core rather than a queue of tasks — there is no run
//! queue in this kernel and adding one would be the reversal. **With no kernel
//! involvement** is the counter: [`process::Entries`] counts every crossing and
//! [`Report::verdict`] requires the hot-path half to be zero. **Notifies** is
//! RFC 0008's reclaim notice, published as pending state per core, and
//! **cleanly** is the runtime's own report that its queue was empty when it
//! went — which is the half a deadline cannot express.
//!
//! # Where the reclaim notice comes from, and why it is the timer
//!
//! The frame is not running while the core it gave away is running. So a notice
//! posted before entry is a notice the runtime meets at its very first polling
//! point, with no work behind it, which would demonstrate parking and not
//! parking *under load*. The frame reaches a core it has given away in exactly
//! one way, and it is the way this architecture already has: the timer
//! interrupt. [`on_ring3_tick`] posts the reclaim from inside the timer handler
//! on the core the runtime holds, and then returns to the runtime.
//!
//! That is the distinction the whole model rests on, made visible: **an
//! interrupt happened and a preemption did not.** The runtime is not
//! rescheduled, its instruction stream is not redirected, and nothing it was
//! doing is abandoned. It finds a completion entry at its next allocation
//! boundary and parks there. `cargo xtask runtime reclaim` measures how many
//! timer intervals that took.
//!
//! # Why none of these shards is a fifth cross-core word
//!
//! RFC 0016 names four `PerCpu<u64>` shards in `smp.rs` that two cores reach,
//! and says a fifth needs an argument. The shards here are not a fifth, for the
//! reason `process::JOB`, `process::OUTCOME` and `cap::TABLE` are not either:
//! the boot processor writes them into an *idle* core's slot before the mailbox
//! handoff and reads them after it, so every one of these accesses is ordered by
//! the `Release`/`Acquire` pair `smp` already owns. While the runtime is
//! running, the only code that touches them is the timer handler on the core
//! that owns them. Nothing here is a handshake, so nothing here needs an atomic.
//!
//! # What this does not do, said rather than implied
//!
//! It does not **spawn** the runtime into a place. `component::demonstrate` is
//! the lifecycle — a manifest by content hash, an account, needs checked handle
//! by handle, an endpoint clients hold — and this reads the same component file
//! for one thing: the image, and the class the manifest declares. The runtime's
//! memory comes from the frame allocator rather than from an `Untyped` a
//! supervisor supplied, no need is checked because none is supplied, and
//! admission is not run because nothing here is admitted.
//!
//! That is the same shape RFC 0033 recorded for `virtio-blk`, from the other
//! side: there, a component is spawned into a place and never scheduled; here,
//! a component is scheduled and never spawned into one. Joining the two is a
//! supervisor that sizes an account from what it was routed and then hands the
//! occupant a core — which is what `E1-P06` needs and what the two halves of
//! this epoch were each half of. *Reversal:* `component::spawn` gaining the two
//! ring mappings and this file taking its runtime from a `Place` rather than
//! from a boot module.
//!
//! # What the exit criterion excludes, and why
//!
//! *Async work under load produces zero kernel entries on the hot path,
//! counted.* [`process::Entries`] holds five numbers and the hot path is two of
//! them. RFC 0038 argues the line; the exclusions, so a reader can put them
//! back:
//!
//! - **The `EXIT` that ends the residency** is the allocation boundary rather
//!   than a crossing inside it. It is counted in its own bucket and required to
//!   be exactly one, so a build in which counting had stopped would publish a
//!   zero there and fail.
//! - **Timer interrupts** are the frame's own clock reaching a core it gave
//!   away. They are not the runtime's work crossing a boundary — nothing the
//!   runtime does makes one happen or not happen — and on this boot they are
//!   the mechanism that *delivers* the reclaim. Counted, published, never
//!   subtracted.
//! - **Every other interrupt taken at ring 3** — a shootdown, a doorbell, the
//!   spurious vector — for the timer's reason and not a weaker one. This bucket
//!   was missing when the file first landed, and it was missing in the way that
//!   is hardest to see: the three vectors were on *neither* side of the line,
//!   so the total was not a total. It is provoked as well as counted now —
//!   [`Half::Reclaim`] rings this core's own doorbell from the timer handler
//!   and [`Report::verdict`] requires the bucket to move — because a bucket
//!   nothing can move is how the first three went missing.
//!
//! Nothing else is excluded, and the check behind that sentence is that every
//! arm of `interrupt_dispatch` reachable from ring 3 counts. A door call the
//! runtime makes is on the hot path whether or not the frame implements it, and
//! a fault it takes is on the hot path whether or not it meant to take one.

use f_abi::control::{Promise, is_notice, notice, reclaim};
use f_abi::manifest::{Record, class};
use f_abi::{Cqe, error, feature};
use f_ring::{Collector, Consumer, Mapping, Poster};
use f_store::report::{self, Tally};

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging::{self, Features};
use crate::mem::{FRAME_SIZE, FrameAllocator};
use crate::percpu::{MAX_CPUS, PerCpu};

/// Entries in each of a runtime's two rings.
///
/// Sixteen, the same as a control ring elsewhere in this kernel and for the same
/// reason: the whole region has to fit in one frame. It is also twice
/// [`report::QUANTUM`], which is not a coincidence — a quantum that could fill
/// the ring would make a runtime's own executor meet `RingError::Full` on the
/// path this file is counting crossings on, and a full ring is a retry loop
/// whose length a peer chooses.
const ENTRIES: u32 = 16;

/// How much of its load a runtime must have finished before the reclaim notice
/// is posted.
///
/// A quarter of it, and it is a count of the runtime's *own work* rather than a
/// count of ticks. That is the second attempt and the first one is worth
/// recording, because it was wrong for a reason that is a property of where
/// this is measured rather than of the mechanism: posting after N timer ticks
/// measured nothing, because QEMU's translation backend compiles each block of
/// guest code the first time it is reached and the local APIC's deadline is
/// host time. The same load took 24 ticks on one run and 75 on the next, so a
/// tick threshold was sometimes before the runtime's first polling point and
/// sometimes after most of its work — and a run that reported *zero completed,
/// everything parked* is a run that demonstrated parking and not parking under
/// load.
///
/// Counting the runtime's progress instead makes the notice arrive at the same
/// point in the *workload* whatever the machine costs, which is what
/// `under load` was supposed to mean. What is measured afterwards is still time
/// — how many timer intervals it took to reach an allocation boundary — and
/// that is the number that should be a time, because it is a latency.
///
/// # The bound this is chosen against, which is a property of the emulator
///
/// The notice is posted from a timer tick, so this half needs **at least one
/// ring-3 timer tick to land between a quarter of the load and the end of it**.
/// Under QEMU the whole load takes tens of ticks — fifteen to fifty-odd across
/// the runs measured — so a tick inside that window is not close. On hardware,
/// or on a host fast enough, sixteen thousand items of a purely in-memory
/// self-queue finish inside one millisecond, no tick lands in the window at
/// all, and this half goes **red** with *no reclaim notice was posted, so
/// nothing was parked* — the harness failing rather than the mechanism.
///
/// It is written down here rather than made robust because every way of making
/// it robust changes what is being measured: a shorter timer period would make
/// the tick exclusion a different number, and a larger load would stop fitting
/// the sixteen bits [`report::pack`] gives it. *Reversal:* a one-shot APIC
/// deadline armed from the runtime's own progress, which is what E5's hardware
/// will need anyway — at which point this comment is the reason it was written.
/// Unit: work items.
const RECLAIM_AFTER_ITEMS: u32 = f_store::report::LOAD / 4;

/// How much further a runtime may get after it is told, before it parks.
///
/// One quantum, and not one more. That is the property exactly: a runtime is
/// preempted between quanta and never inside one, so a notice that lands
/// mid-quantum is acted on at the end of *that* quantum and the work it does in
/// between is bounded by [`report::QUANTUM`]. A run that got further was told
/// during a quantum it could not leave, which is the thing an allocation
/// boundary exists to make impossible.
///
/// **In work items rather than in timer intervals**, and the first attempt was
/// the second. It failed for the same reason the tick threshold above failed:
/// the runtime parks in microseconds of guest time, and the emulator's first
/// execution of the exit path costs milliseconds of host time — so the same
/// correct behaviour measured two, five or fifty timer intervals depending on
/// what else the host was doing. The tick figure is still printed, because it
/// is the latency somebody will eventually want; it is not a gate, because
/// under an emulator it is a measurement of the emulator.
/// Unit: work items.
const PARK_WITHIN_ITEMS: u32 = report::QUANTUM;

/// A core in a runtime's allocation.
#[derive(Clone, Copy)]
struct Held {
    /// Whether the runtime holds it at all.
    held: bool,
    /// Whether it was granted under a hard-class reservation.
    ///
    /// RFC 0007: a granted hard-class reservation holds a whole physical core,
    /// its sibling, a bandwidth allocation and a cache partition, and forecloses
    /// work-conserving scheduling of reserved capacity — *reserved and idle
    /// stays idle, and no later optimisation may quietly lend it out.* So this
    /// bit is what [`Allocation::reclaim`] refuses on, and it is a bit rather
    /// than a policy lookup because the refusal has to be local to the core
    /// being asked about.
    reserved: bool,
    /// What the runtime has been told about losing it.
    ///
    /// A promise and not a grade: **it may only ever move earlier**. Reclaiming
    /// core 3 and then core 7 before a drain is two facts and neither may
    /// displace the other, which is why this lives per core rather than as one
    /// word per runtime. RFC 0008.
    reclaim: f_abi::control::Promise,
}

impl Held {
    /// A core nobody holds.
    const FREE: Self =
        Self { held: false, reserved: false, reclaim: f_abi::control::Promise::NONE };
}

/// The cores one runtime holds, and what it has been told about them.
///
/// On the stack of whoever is standing a runtime up, exactly as
/// `component::demonstrate` holds a supervisor's table there. A supervisor is a
/// component and its allocation is part of what its account paid for; the frame
/// holding one for the length of a demonstration is the smallest thing that is
/// not a lie, and it keeps this file free of kernel-global state.
pub struct Allocation {
    cores: [Held; MAX_CPUS],
}

impl Allocation {
    /// A runtime holding nothing.
    pub const NONE: Self = Self { cores: [Held::FREE; MAX_CPUS] };

    /// Give this runtime a core.
    ///
    /// `class` is an `f_abi::manifest::class` constant, which is the vocabulary
    /// a *component* declares its class in — RFC 0025: the ceiling is declared
    /// in a manifest and granted once at spawn, and it is a property of the
    /// component rather than of a request. [`class::HARD`] makes the core part
    /// of a reservation, which is what [`Self::reclaim`] refuses to take back.
    ///
    /// # Errors
    ///
    /// `ADMISSION/NO_CORE` for a core index this kernel does not shard for or
    /// one this runtime already holds. Refused rather than ignored, because an
    /// allocation that silently absorbed a second grant of one core would be an
    /// allocation whose size is not the number of cores in it.
    pub fn allocate(&mut self, cpu: usize, class: u8) -> Result<(), i32> {
        let no_core = error::pack(error::ADMISSION, error::admission::NO_CORE);
        let Some(core) = self.cores.get_mut(cpu) else { return Err(no_core) };
        if core.held {
            return Err(no_core);
        }
        *core = Held { held: true, reserved: class == class::HARD, reclaim: Promise::NONE };
        Ok(())
    }

    /// Take a core back at a deadline.
    ///
    /// Answers whether the promise moved, which is what lets a caller say
    /// *which deadline it kept* rather than reporting a bare success the asker
    /// would misread: a second reclaim of one core keeps the earlier of the two,
    /// because a runtime that has begun quiescing against one must not have it
    /// withdrawn under it. R08.
    ///
    /// # Errors
    ///
    /// `ADMISSION/NO_CORE` for a core this runtime does not hold, and
    /// `ADMISSION/RESERVED` for one held under a hard-class reservation —
    /// which is RFC 0007's rule and the one clause of it this file can enforce
    /// on a machine with no cache partitioning.
    pub fn reclaim(&mut self, cpu: usize, deadline: u64) -> Result<bool, i32> {
        let Some(core) = self.cores.get_mut(cpu) else {
            return Err(error::pack(error::ADMISSION, error::admission::NO_CORE));
        };
        if !core.held {
            return Err(error::pack(error::ADMISSION, error::admission::NO_CORE));
        }
        if core.reserved {
            return Err(error::pack(error::ADMISSION, error::admission::RESERVED));
        }
        Ok(core.reclaim.promise(deadline))
    }

    /// The next reclaim notice this allocation owes, by core ascending.
    ///
    /// The third phase of `f_abi::control::ORDER`, which `cap::Table` leaves a
    /// hole for and names this as filling. One entry per core and never one for
    /// several.
    pub fn next_reclaim_notice(&mut self, timestamp: u64) -> Option<Cqe> {
        for (cpu, core) in self.cores.iter_mut().enumerate() {
            if !core.held {
                continue;
            }
            if let Some(deadline) = core.reclaim.drain() {
                return Some(reclaim::entry(cpu as u32, deadline, timestamp));
            }
        }
        None
    }

    /// The deadline a core is under, if it is under one.
    /// Unit: nanoseconds, monotonic, in the control channel's epoch.
    #[must_use]
    pub fn deadline(&self, cpu: usize) -> Option<u64> {
        self.cores.get(cpu).filter(|core| core.held).and_then(|core| core.reclaim.deadline())
    }

    /// How many cores this runtime holds. Unit: cores.
    #[must_use]
    pub fn held(&self) -> usize {
        self.cores.iter().filter(|core| core.held).count()
    }
}

/// Which experiment one boot runs.
///
/// Four, and none of them means anything alone — the argument `blk::Half` makes
/// and `mutate` makes before it. [`Half::Load`] is the positive control and the
/// exit criterion; [`Half::Provoke`] is the same run with one crossing on
/// purpose, without which the zero above is a number nothing could move;
/// [`Half::Reclaim`] is the notice arriving under load; [`Half::Hostile`] is the
/// adoption refusing bytes a peer scribbled, which is the half that says safe
/// adoption is *safe* rather than merely available.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    /// A runtime holds a core and puts [`report::LOAD`] work items through its
    /// own executor. Nothing may cross the boundary until it exits.
    Load,
    /// The same run, and the runtime makes one door call in the middle of its
    /// work loop on purpose. The hot-path count must move by exactly that many.
    Provoke,
    /// The same run, and the frame posts a reclaim notice from the timer handler
    /// after the runtime has been working for a tick. It must park at its next
    /// allocation boundary with its own queue empty — and it takes one doorbell
    /// on the way, which is the interrupt most easily mistaken for a preemption
    /// and the one that makes `Entries::interrupts` a number that moves.
    Reclaim,
    /// The frame scribbles the control ring's header before entry. The
    /// component's adoption must refuse with a structured error and exit saying
    /// so, rather than faulting, hanging or believing it.
    Hostile,
}

impl Half {
    /// The word the boot log and the harness's parameter share.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Provoke => "provoke",
            Self::Reclaim => "reclaim",
            Self::Hostile => "hostile",
        }
    }

    /// What the component is entered with.
    /// Unit: none — a selector ordinal.
    const fn selector(self) -> u32 {
        match self {
            Self::Provoke => report::PROVOKE,
            _ => report::RUN,
        }
    }
}

/// Why the runtime could not be stood up. Every one fails the boot.
#[derive(Clone, Copy, Debug)]
pub enum Trouble {
    /// No boot module carried a component file.
    NoComponent,
    /// A component file was refused, carrying the refusal.
    Manifest(f_abi::manifest::Refusal),
    /// The frame could not build the runtime, carrying which step.
    Process(crate::process::Error),
    /// A ring this build wrote it cannot read back, carrying the refusal.
    Ring(i32),
    /// The allocation refused a core the frame asked for.
    Allocation(i32),
    /// The core given the runtime never reported finished.
    NoAnswer(usize),
}

impl Trouble {
    /// A line for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoComponent => "no component file among the boot modules",
            Self::Manifest(_) => "the component file is not one this build can read",
            Self::Process(_) => "the runtime could not be built",
            Self::Ring(_) => "a ring this build wrote it cannot read back",
            Self::Allocation(_) => "the allocation refused a core",
            Self::NoAnswer(_) => "the core given the runtime never reported finished",
        }
    }
}

/// What one run of a runtime did.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// Which experiment this was.
    pub half: Half,
    /// The core the runtime was allocated. Unit: none — a core index.
    pub cpu: usize,
    /// How many cores it held. Unit: cores.
    pub cores: usize,
    /// Capabilities the frame put in its table, each of which is a *granted*
    /// notice it is owed. Unit: capabilities.
    pub granted: usize,
    /// Notices the frame published onto its control ring before entry.
    /// Unit: notices.
    pub posted: u32,
    /// Whether a reclaim naming a hard-class core was refused
    /// `ADMISSION/RESERVED`.
    ///
    /// Driven on every half, because RFC 0007's *reserved and idle stays idle*
    /// is a refusal nobody would notice going missing: a build that reclaimed a
    /// reserved core would pass every other check here.
    pub reserved_refused: bool,
    /// Whether a second reclaim naming a later deadline left the earlier one
    /// standing.
    pub deadline_kept: bool,
    /// Which ring-3 tick the reclaim notice was posted at, counting from one.
    /// Zero when this half posted none.
    ///
    /// Printed and not gated on. See [`PARK_WITHIN_ITEMS`].
    /// Unit: timer ticks.
    pub posted_at: u64,
    /// How much of its load the runtime had finished when the notice went out.
    ///
    /// What *parked at its next allocation boundary* is measured against: the
    /// difference between this and what it finished altogether is how much
    /// further it got after being told, and that difference is bounded by one
    /// quantum whatever the machine costs.
    /// Unit: work items.
    pub progress: u32,
    /// What crossed into the frame while the runtime held the core, in the five
    /// buckets RFC 0038 argues.
    pub entries: crate::process::Entries,
    /// Whether it ended by `EXIT` rather than by a fault.
    pub exited: bool,
    /// What it said on the way out.
    pub tally: Tally,
    /// What the frame found still on the runtime's own work ring afterwards.
    ///
    /// The frame's own reading of *cleanly*, taken rather than believed: the
    /// runtime reports [`report::QUIESCENT`] and this is the number that agrees
    /// or does not. [`u32::MAX`] when the ring no longer validates at all.
    /// Unit: entries.
    pub left_behind: u32,
}

impl Report {
    /// Whether the run produced what the half it was asked for requires.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold. Every one of them fails the boot: a
    /// boundary that was not crossed is not a smaller result than one that was,
    /// it is the opposite result.
    pub const fn verdict(&self) -> Result<(), &'static str> {
        // True on every half, and checked first because they are about the
        // measuring apparatus rather than about the experiment.
        if !self.reserved_refused {
            return Err("a core held under a hard-class reservation was reclaimed");
        }
        if !self.deadline_kept {
            return Err("a second reclaim moved a deadline later, which a promise may not do");
        }
        if !self.exited {
            return Err("the runtime did not end by EXIT, so what it reported is not its own");
        }
        // The allocation boundary, and it is required to be *exactly* one. A
        // zero here is the counting having stopped, which would publish a clean
        // hot path for a build that had stopped measuring one — the defect
        // `state::node::BLK_PROVOKED` exists to keep out of the datapath's
        // number, one subsystem over.
        if self.entries.boundary != 1 {
            return Err("the residency did not end in exactly one boundary crossing, so the \
                        counter is measuring nothing");
        }
        if self.entries.faults != 0 {
            return Err("the runtime faulted, which is a boundary crossing it did not choose");
        }

        match self.half {
            Half::Load => {
                if self.entries.hot != 0 {
                    return Err("async work under load crossed into the frame on the hot path");
                }
                if self.tally.code != report::OK {
                    return Err("the runtime stopped for a reason of its own, so its zero \
                                measures a shorter run than the one asked for");
                }
                if self.tally.completed != report::LOAD {
                    return Err("the runtime did not finish the load, so nothing was under it");
                }
                if !self.tally.quiescent() || self.left_behind != 0 {
                    return Err("the runtime left work on its own ring");
                }
                if self.tally.notices == 0 {
                    return Err("the runtime drained no notice, so the control ring was not \
                                driven by the component at all");
                }
                Ok(())
            }
            Half::Provoke => {
                if self.tally.provoked == 0 {
                    return Err("the provocation did not run, so the zero on the load half \
                                measures nothing");
                }
                // The same two the load half checks, and for a reason that is
                // not symmetry. The provocation fires after the first quantum,
                // so a build whose executor stopped immediately afterwards
                // would still make exactly one call, still report one
                // provocation, still exit with one boundary crossing, and pass
                // this half — while the run it is supposed to be *the
                // same run as* had not happened. This half is described as the load
                // with one crossing on purpose, and these are what make the
                // description true rather than nearly true.
                if self.tally.code != report::OK {
                    return Err("the runtime stopped for a reason of its own, so the crossing \
                                it counted is one out of a shorter run than the one asked for");
                }
                if self.tally.completed != report::LOAD {
                    return Err("the runtime did not finish the load, so nothing was under it");
                }
                // The two numbers are taken on opposite sides of the boundary —
                // the component counts what it did and the frame counts what
                // arrived — so requiring them equal is the check that the
                // counter counts the thing it is named after rather than
                // something correlated with it.
                if self.entries.hot != self.tally.provoked as u64 {
                    return Err("the frame and the runtime disagree about how many crossings \
                                the provocation made");
                }
                Ok(())
            }
            Half::Reclaim => {
                if self.entries.hot != 0 {
                    return Err("a runtime being reclaimed crossed into the frame on the hot \
                                path");
                }
                if self.posted_at == 0 {
                    return Err("no reclaim notice was posted, so nothing was parked");
                }
                if !self.tally.reclaimed() {
                    return Err("the runtime never saw the reclaim notice");
                }
                if self.tally.completed == 0 {
                    return Err("the runtime parked before it had done any work, so it parked \
                                and was not under load");
                }
                if self.tally.parked == 0 {
                    return Err("the runtime finished the whole load, so it stopped because it \
                                ran out of work rather than because it was told");
                }
                if !self.tally.quiescent() || self.left_behind != 0 {
                    return Err("the runtime abandoned work on its own ring rather than \
                                parking it");
                }
                if self.tally.completed.saturating_sub(self.progress) > PARK_WITHIN_ITEMS {
                    return Err("the runtime got further than one quantum past the notice, so \
                                it was told inside a quantum it could not leave");
                }
                // The fifth bucket, required to move on the one half that
                // provokes it. A doorbell went to this core while ring 3 held
                // it; it is a kernel entry, it is not on the hot path, and the
                // hot-path zero two checks above is what says the runtime did
                // not answer it. A build that stopped counting the three
                // non-timer vectors publishes zero here and fails, which is the
                // property their absence used to lack.
                if self.entries.interrupts == 0 {
                    return Err("no interrupt other than the clock reached the core, so the \
                                bucket that would hold one is a number nothing can move");
                }
                Ok(())
            }
            Half::Hostile => {
                if self.tally.code != report::NO_CONTROL {
                    return Err("a scribbled control ring header was adopted, or was refused \
                                for a reason this half is not about");
                }
                // Domain and reason, both whole and both compared. The
                // encoding this replaced kept eight bits of the reason, so two
                // refusals in one domain differing above bit seven passed this
                // check as each other; `report::refusal` carries the argument.
                if !report::refused_with(
                    &self.tally,
                    error::ARGUMENT,
                    error::argument::MALFORMED_HEADER,
                ) {
                    return Err("the adoption refused with something other than a malformed \
                                header, so it refused the wrong thing");
                }
                if self.entries.hot != 0 {
                    return Err("the runtime crossed the boundary before giving up, so its \
                                refusal was not the adoption's");
                }
                Ok(())
            }
        }
    }
}

/// The control ring a reclaim is to be posted onto, as a kernel address.
///
/// Zero when this core is not standing a reclaim. Written by the boot processor
/// into an idle core's slot before the mailbox handoff and read by the timer
/// handler on the core that owns it, which is `process::JOB`'s arrangement
/// exactly — see the module comment on why this is not a fifth cross-core word.
static RECLAIM_RING: PerCpu<u64> = PerCpu::new(0);

/// The runtime's own work ring, as a kernel address.
///
/// **The frame watching a component's progress through memory it granted**,
/// which is RFC 0013's *read, never delivered* applied to the one number that
/// says how far along a runtime is. It is not an interface the runtime offers
/// and it is not a message it sends: it is the completion cursor of its own
/// queue, in a frame the frame allocated, and reading it costs the runtime
/// nothing and tells it nothing.
///
/// *Reversal:* a runtime that publishes a state tree under RFC 0013, at which
/// point this is a node with a name instead of a cursor with a meaning.
static RECLAIM_WORK: PerCpu<u64> = PerCpu::new(0);

/// The deadline the notice carries.
/// Unit: nanoseconds, monotonic, in the control channel's epoch.
static RECLAIM_DEADLINE: PerCpu<u64> = PerCpu::new(0);

/// The core index the notice names.
static RECLAIM_CORE: PerCpu<u64> = PerCpu::new(0);

/// The ring-3 tick the notice was posted at, counting from one. Zero until it
/// has been.
static RECLAIM_POSTED: PerCpu<u64> = PerCpu::new(0);

/// How much of its load the runtime had finished at that moment.
///
/// What the parking bound is measured against, because it is the one quantity
/// that means the same thing on a fast machine and a slow one.
/// Unit: work items.
static RECLAIM_PROGRESS: PerCpu<u64> = PerCpu::new(0);

/// Arm this core to post a reclaim once ring 3 has held it for a tick.
///
/// # Safety
///
/// `cpu` must be an idle core with no process on it, and `rings` the kernel
/// addresses of the two rings that core's next runtime will be entered with.
unsafe fn arm_reclaim(cpu: usize, rings: crate::process::Rings, core: u64, deadline: u64) {
    for (shard, value) in [
        (&RECLAIM_WORK, rings.work),
        (&RECLAIM_DEADLINE, deadline),
        (&RECLAIM_CORE, core),
        (&RECLAIM_POSTED, 0),
        (&RECLAIM_PROGRESS, 0),
        // Last, because it is the word the handler reads first: a core armed
        // with a ring and no deadline would post a notice promising nothing.
        (&RECLAIM_RING, rings.control),
    ] {
        let slot = shard.at(cpu);
        // SAFETY: the caller's guarantee that `cpu` is idle, so the timer
        // handler over there has no ring-3 tick to take and cannot be reading
        // these. Volatile because the handler is the other accessor.
        unsafe { slot.write_volatile(value) };
    }
}

/// Disarm every core's reclaim.
///
/// # Safety
///
/// Call on the boot processor with no runtime running anywhere.
unsafe fn disarm_reclaim(cpu: usize) {
    let slot = RECLAIM_RING.at(cpu);
    // SAFETY: the caller's guarantee. One word, and it is the one the handler
    // reads first, so clearing it is enough to make the rest unread.
    unsafe { slot.write_volatile(0) };
}

/// Post the reclaim notice, if this core is standing one and the moment has
/// come.
///
/// **This is the frame reaching a core it gave away, and it is the only way it
/// can.** What it must not be is a preemption: nothing here touches the
/// interrupted instruction stream, redirects the runtime, or ends anything. It
/// writes a completion entry into a ring and returns, and the runtime finds it
/// when it next chooses to look.
///
/// A refusal at any step leaves the notice unposted and tries again on the next
/// tick, which is the right answer to all three of them: a full ring means the
/// runtime has not drained yet, and a header that no longer validates means the
/// runtime scribbled its own control ring — in which case it has stopped
/// speaking, and RFC 0008 says what happens to a component that has.
///
/// # Safety
///
/// Call from the timer handler on the core the runtime holds, with the tick
/// count it has just taken out of ring 3 — which is not what decides whether to
/// post, only what is recorded when it does.
pub(crate) unsafe fn on_ring3_tick(taken: u64) {
    // SAFETY: this core's slot, volatile as every access to these shards is.
    // The handler cannot interrupt itself — its gate is an interrupt gate — and
    // the boot processor only writes these while this core is idle.
    let ring = unsafe { RECLAIM_RING.mine().read_volatile() };
    if ring == 0 {
        return;
    }
    // SAFETY: as above.
    if unsafe { RECLAIM_POSTED.mine().read_volatile() } != 0 {
        return;
    }
    // SAFETY: as above.
    let work = unsafe { RECLAIM_WORK.mine().read_volatile() };
    // SAFETY: `work` is the kernel address of the frame `prepare_runtime`
    // retyped for this runtime's work ring, written into this core's slot while
    // it was idle, and the frame is live for the whole residency — or zero,
    // which `finished` answers for.
    let progress = unsafe { finished(work) };
    if progress < RECLAIM_AFTER_ITEMS {
        return;
    }
    // SAFETY: as above.
    let core = unsafe { RECLAIM_CORE.mine().read_volatile() };
    // SAFETY: as above.
    let deadline = unsafe { RECLAIM_DEADLINE.mine().read_volatile() };

    // SAFETY: `ring` is the kernel address of a frame `prepare_runtime` retyped
    // for this runtime's control ring and handed to nobody else, and it is
    // `FRAME_SIZE` bytes of the direct map. The mapping is dropped before this
    // function returns, so nothing borrowed from it survives the handler.
    let bound = unsafe {
        Mapping::adopt(
            ring as *mut u8,
            FRAME_SIZE as u32,
            feature::CONTROL_EVENTS,
            feature::CONTROL_EVENTS,
        )
    };
    let Ok(bound) = bound else { return };
    let Some(poster) = Poster::new(bound.completions()) else { return };
    // No timestamp. The boot log is a fixture and a stamp in it would be a
    // different number every run; a component that needs to know *when* reads
    // `Cqe::timestamp`, and this build has no component that does.
    if poster.post(reclaim::entry(core as u32, deadline, 0)).is_err() {
        return;
    }
    let slot = RECLAIM_PROGRESS.mine();
    // SAFETY: as the reads above. Written before the word that says a notice
    // went out, so a reader that sees the second has already seen the first.
    unsafe { slot.write_volatile(u64::from(progress)) };
    let slot = RECLAIM_POSTED.mine();
    // SAFETY: as the reads above.
    unsafe { slot.write_volatile(taken) };

    // And one interrupt that is not the clock, delivered to this core while
    // ring 3 holds it — a doorbell, which is what a peer with something to
    // announce would send. It exists for `MEMORY_FORCED`'s reason and
    // `BLK_PROVOKED`'s: `process::Entries::interrupts` counts the shootdown,
    // the doorbell and the spurious vector, and until this line nothing in any
    // boot could make it anything but zero — which is indistinguishable from a
    // bucket that does not work, and is exactly how the three vectors came to
    // be uncounted in the first place.
    //
    // It is also the sharpest form of this half's claim. A doorbell is the
    // interrupt most easily mistaken for a preemption: it arrives from outside,
    // it is about work, and it lands in the middle of a quantum. What happens
    // is that the runtime's instruction stream resumes exactly where it was,
    // the entry is counted in a bucket that is not the hot path, and the
    // parking still happens at the next allocation boundary and nowhere else.
    //
    // Sent before the timer's own end-of-interrupt, which `apic::on_tick` has
    // not reached yet, and delivered after it: the gate cleared the interrupt
    // flag, so this core takes it on the `iretq` back into ring 3 rather than
    // inside this handler.
    //
    // SAFETY: this core is running — it is the one executing this handler — and
    // its local APIC is mapped, which is what `apic::window` reading this
    // core's own slot depends on and what the timer that got here already used.
    unsafe { crate::doorbell::ring(crate::arch::x86_64::current_cpu()) };
}

/// How many work items the runtime whose queue lives at `work` has reaped.
///
/// The tail cursor of its own completion ring, which is the number it advances
/// once per item it finishes. Read rather than asked for, out of memory the
/// frame granted — and read through the ordinary accessor rather than as a bare
/// word, so a peer that corrupted its own header answers zero here instead of
/// answering whatever is at a fixed offset.
///
/// # Safety
///
/// `work` must be zero, or the kernel address of a frame holding a runtime's
/// work ring for the whole of this call.
unsafe fn finished(work: u64) -> u32 {
    if work == 0 {
        return 0;
    }
    // SAFETY: the caller's guarantee. The mapping is dropped before this
    // function returns, so nothing borrowed from it survives the handler.
    let bound = unsafe { Mapping::adopt(work as *mut u8, FRAME_SIZE as u32, 0, 0) };
    let Ok(bound) = bound else { return 0 };
    bound.completions().tail.raw()
}

/// Stand a runtime up, give it a core, and require the right thing to happen.
///
/// # Errors
///
/// [`Trouble`], every variant of which means the runtime did not run.
///
/// # Safety
///
/// Call on the boot processor, with the kernel's address space in `CR3`,
/// `frames` rebound onto its direct map, the direct map covering every boot
/// module, and `cpu` a core that is up and idle.
#[expect(
    clippy::too_many_arguments,
    reason = "every one is something the boot path found and this call cannot: the machine's \
              memory, its address space, what it agreed to interpret, its command line, the \
              core the topology left free, its clocks, and the deadline. Bundling them would \
              be a type that exists so that a lint passes, which `blk::demonstrate` already \
              declined for the same reason"
)]
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    boot: &BootInfo,
    half: Half,
    cpu: usize,
    hz: u32,
    target: u64,
    tsc_khz: u64,
    deadline: u64,
    tree: u64,
) -> Result<Report, Trouble> {
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let (modules, count) = unsafe { crate::component::modules(boot) };
    let module = *modules.first().filter(|_| count > 0).ok_or(Trouble::NoComponent)?;
    let record = Record::read(module).map_err(Trouble::Manifest)?;
    let image = record.image(module).map_err(Trouble::Manifest)?;

    // ------------------------------------------------- the allocation itself
    //
    // Two cores, and the second is never entered: it exists so that RFC 0007's
    // rule has something to refuse about. A suite that only ever asked to
    // reclaim a soft-class core would pass identically on a build that had
    // never heard of a reservation.
    let mut allocation = Allocation::NONE;
    // The class the *manifest* declares, and not a constant here. RFC 0007's
    // rule is about what a component was admitted for, so a frame that decided
    // it would be a frame deciding whether its own refusal applies.
    allocation.allocate(cpu, record.class).map_err(Trouble::Allocation)?;
    let reserved = (cpu + 1) % MAX_CPUS;
    let reserved_refused = if reserved == cpu {
        // A machine this kernel shards for exactly one core of, which cannot
        // happen — `MAX_CPUS` is eight — but a check that silently passed on a
        // machine where it could not run would be worse than one that says so.
        false
    } else {
        allocation.allocate(reserved, class::HARD).map_err(Trouble::Allocation)?;
        matches!(
            allocation.reclaim(reserved, deadline).map(|_| ()).map_err(error::unpack),
            Err(Some((error::ADMISSION, error::admission::RESERVED)))
        )
    };

    // A promise may only move earlier. Driven rather than described, because
    // this is the rule R08 is about and the one a later change would relax
    // without noticing.
    let mut kept = Allocation::NONE;
    kept.allocate(cpu, class::SOFT).map_err(Trouble::Allocation)?;
    let first = kept.reclaim(cpu, deadline).map_err(Trouble::Allocation)?;
    let later = kept.reclaim(cpu, deadline.saturating_add(1)).map_err(Trouble::Allocation)?;
    let earlier = kept.reclaim(cpu, deadline.saturating_sub(1)).map_err(Trouble::Allocation)?;
    // And what that promise becomes on the wire, in `ORDER`'s third phase: one
    // entry, naming the core and the deadline that was kept, and nothing owed
    // after it. Checked here rather than only on the half that posts one, so
    // that the reading of `Cqe::user_data` a reclaim needs is exercised on every
    // boot rather than on one of four.
    let owed = kept.next_reclaim_notice(0);
    let named = owed.as_ref().and_then(reclaim::core) == Some(cpu as u32)
        && owed.as_ref().and_then(reclaim::deadline) == Some(deadline.saturating_sub(1))
        && kept.next_reclaim_notice(0).is_none();
    let deadline_kept = first
        && !later
        && earlier
        && kept.deadline(cpu) == Some(deadline.saturating_sub(1))
        && named;

    // -------------------------------------------------------- the runtime
    let plan =
        crate::process::RuntimePlan { image, selector: half.selector(), tree, hz, target, cpu };
    // SAFETY: the caller's guarantee, passed down unchanged.
    let (prepared, rings) =
        unsafe { crate::process::prepare_runtime(frames, kernel, features, plan) }
            .map_err(Trouble::Process)?;

    // The frame is the grantor, so the frame writes the headers. The runtime
    // adopts them and believes nothing, which is what it would do if the peer
    // were hostile — and on one of these halves it is.
    let control = describe(rings.control, feature::CONTROL_EVENTS)?;
    let work = describe(rings.work, 0)?;

    // Everything the table owes, published before the first instruction runs.
    // RFC 0008: a component's initial grants are notices in the ring *before*
    // it starts, because there is no submission for them to answer.
    let posted = publish(cpu, &control, &mut allocation)?;

    if half == Half::Reclaim {
        // SAFETY: `cpu` is idle — nothing has told it to run yet — and
        // `rings.control` is the control ring it is about to be entered with.
        unsafe { arm_reclaim(cpu, rings, cpu as u64, deadline) };
    }

    if half == Half::Hostile {
        // The peer scribbles the header, which is RFC 0008's second way for a
        // component to be found to have stopped speaking — used here in the
        // other direction, to make the frame the untrustworthy peer for one
        // boot. `Adopted::at` must refuse rather than fault.
        //
        // SAFETY: `rings.control` is the kernel address of a frame this call
        // retyped for the runtime, and the mapping over it above is past its
        // last use.
        unsafe { (rings.control as *mut u64).write_volatile(!f_abi::CHANNEL_MAGIC) };
    }

    // SAFETY: `cpu` reports ready, everything `process::execute` depends on was
    // put in its shards by `prepare_runtime`, and this core does not need
    // interrupts enabled to wait: `run_on`'s requirement is that a shootdown can
    // be answered, and a runtime cannot cause one — its `State::frames` is zero,
    // so every capability call that could revoke a mapping is refused before it
    // reaches the address space.
    let ran = unsafe { crate::smp::run_on(cpu, kernel.root(), tsc_khz, WAIT_MICROS) };
    // Disarmed whatever happened, so a later boot stage cannot meet a core still
    // standing a notice for a ring that has been freed.
    // SAFETY: on the boot processor, with the runtime over either way.
    unsafe { disarm_reclaim(cpu) };
    ran.map_err(Trouble::NoAnswer)?;

    // What the runtime left on its own queue, read by the frame rather than
    // taken from the runtime's word for it.
    let left_behind = leftovers(&work);

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished — which is what `run_on` returning `Ok` means.
    let report = unsafe { crate::process::reap(frames, prepared) }.map_err(Trouble::Process)?;

    // SAFETY: this core's own slot for `cpu` is read after the mailbox
    // `Release`/`Acquire` pair that `run_on` completed, so the timer handler's
    // write over there is visible here and no writer is left.
    let posted_at = unsafe { RECLAIM_POSTED.at(cpu).read_volatile() };
    // SAFETY: as above.
    let progress = unsafe { RECLAIM_PROGRESS.at(cpu).read_volatile() } as u32;

    let (exited, status) = match report.death {
        crate::process::Death::Exited(status) => (true, status),
        _ => (false, 0),
    };

    Ok(Report {
        half,
        cpu,
        cores: allocation.held(),
        granted: report.granted,
        posted,
        reserved_refused,
        deadline_kept,
        posted_at,
        progress,
        entries: report.entries,
        exited,
        tally: report::unpack(status),
        left_behind,
    })
}

/// How long the boot processor waits for the core running a runtime.
///
/// Five seconds, the same bound `main::run_one` uses and for the same reason: it
/// is the answer to a core that is wedged rather than a schedule for one that is
/// working.
/// Unit: microseconds.
const WAIT_MICROS: u64 = 5_000_000;

/// Write a channel header into a frame the runtime is about to be handed.
fn describe(at: u64, features: u64) -> Result<Mapping, Trouble> {
    // SAFETY: `at` is the kernel address of a frame `prepare_runtime` allocated
    // zeroed and handed to nobody else; it is frame-aligned, which is stronger
    // than the cache line a header needs, and `FRAME_SIZE` bytes.
    unsafe { Mapping::describe(at as *mut u8, FRAME_SIZE as u32, ENTRIES, 0, features, features) }
        .map_err(Trouble::Ring)
}

/// Put everything a runtime's table owes onto its control ring.
///
/// In `f_abi::control::ORDER`: slots ascending, then the stop, then reclaim by
/// core ascending, then the two grades. The reclaim phase is deliberately not
/// here — on this path it arrives from the timer handler, under load, which is
/// the whole of what [`Half::Reclaim`] is about.
fn publish(cpu: usize, control: &Mapping, allocation: &mut Allocation) -> Result<u32, Trouble> {
    let poster = Poster::new(control.completions())
        .ok_or(Trouble::Ring(error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER)))?;
    let table = crate::cap::of(cpu);
    // SAFETY: the table of an idle core with no process on it, which is the
    // write `PerCpu::at` exists for and the same one `prepare_runtime` made.
    let table = unsafe { &mut *table };

    let mut posted = 0;
    while poster.free().unwrap_or(0) > 0 {
        // Phase one, then phase three. Phase two is the stop word, which nothing
        // here promises, and phase four is the two grades, which nothing here
        // sets — and both are `cap::Table`'s rather than this file's. Phase
        // three is the one `cap::Table` leaves a hole for and names this as
        // filling: it is bounded by the cores a runtime holds rather than by
        // the slots it bought, which is why it lives beside the allocation.
        let Some(entry) = table.next_slot_notice(0).or_else(|| allocation.next_reclaim_notice(0))
        else {
            break;
        };
        // R04: a kind this build does not define is not published. A frame that
        // produced one would leave a component an entry it is not permitted to
        // skip and cannot name.
        if !is_notice(&entry) || !notice::known(entry.result) {
            return Err(Trouble::Ring(entry.result));
        }
        if poster.post(entry).is_err() {
            break;
        }
        posted += 1;
    }
    Ok(posted)
}

/// What is still on a runtime's own work ring.
///
/// Both halves, because *cleanly* is about both: a submission nobody executed
/// and a completion nobody reaped are two different abandonments.
fn leftovers(work: &Mapping) -> u32 {
    let Some(consumer) = Consumer::new(work.channel()) else { return u32::MAX };
    let Some(collector) = Collector::new(work.completions()) else { return u32::MAX };
    let mut left = 0;
    while matches!(consumer.pop(), Ok(Some(_))) {
        left += 1;
    }
    while matches!(collector.take(), Ok(Some(_))) {
        left += 1;
    }
    left
}
