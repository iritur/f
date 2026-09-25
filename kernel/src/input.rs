// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The input path, end to end: a fourth driver outside the frame, a real
//! pointing device, an event a person caused, and a compositor that takes it off
//! the driver's ring itself and moves a node because of it.
//!
//! # What this file is, and what it finishes
//!
//! It is the **frame's half** of `E3-B04d` and, since `E3-B04g`, of a boot in
//! which the frame is no longer on the input path at all. `user/virtio-input` is
//! the driver: the transport handshake, one virtqueue, the evdev accumulator,
//! the reading and the submission loop, in a crate that forbids `unsafe`.
//! `user/compositor` is the consumer. What is here is everything a supervisor
//! does around the two of them — and, since `E3-B04g`, nothing else: the frame
//! lays the driver's data channel out, hands one end to each component, and
//! never takes an entry off it.
//!
//! Five lines close on it.
//!
//! `E3-B04d`'s exit is *a driver component delivers events a compositor
//! consumes, using `kernel/src/supervisor.rs`'s shared half rather than a fourth
//! copy of it.* The shared half is [`declared`], [`Registers`], [`order_for`]
//! and [`Supervising`], and this file writes none of them again — the one thing
//! it had to widen is `Supervising::reaper`, which is an `Option` because this
//! is the first supervisor in the tree whose driver **produces rather than
//! answers** and therefore holds no client end to reap.
//!
//! `E3-B04a`'s exit is *one time source in the whole input path*, and RFC 0099
//! narrowed it because it was true of a path carrying nothing. This is the boot
//! in which it carries something: the driver opens a report, takes one reading
//! for it, and every entry the compositor drains carries one, because
//! `f_abi::input::Event::decode` refuses an entry that does not and the
//! compositor decodes every entry it takes.
//!
//! `E3-B04g`'s exit is *a boot in which `user/compositor` holds the other end of
//! the driver's data channel*, decodes and folds every entry itself, and checks
//! its fold against the driver's — **with the frame relaying no input entry at
//! all**. The last clause is structural here rather than asserted: there is no
//! consumer of that ring in this file, no decoder call, and no buffer an event
//! could be kept in. The frame's only contact with the ring is laying it out
//! and handing out its ends.
//!
//! `E3-B01i`'s boot half is *a report taken, a frame latched, and the latched
//! transform differing from the committed one by exactly the motion injected*.
//! The compositor latches from positions it took off the driver's ring; this
//! frame commits the pointer's transform at the accumulator's origin, which is
//! the only place a client that was never told a position can honestly put it;
//! and `cargo xtask input` — the process that moved the pointer — checks the
//! difference against the motion it injected.
//!
//! `E3-B04e`'s is *the latched value is the predicted one, asserted from both
//! sides of the seam*, and it closes on the `gesture=steady` run. The driver
//! holds a second predictor, fed where each report closes and asked about the
//! display's first scanout, which this frame tells it — a target, stated by
//! the party that declares the display, and never a reading. The verdict
//! requires the compositor to have aimed there too, both sides to have
//! extrapolated, and the two answers to be one value by one lead; the harness
//! closes *how far it moved* with the one number neither side holds. RFC 0134.
//!
//! # Why the frame is not on `lint-stamp`'s list, and why that is truer now
//!
//! `INPUT_PATH` in `xtask` names the crates on this path and forbids every one
//! of them a clock reading. `kernel/` is not on it, and the reason is that **the
//! frame on this path is not a stage**: it writes the clock the driver is told to
//! take its readings against — a seed and a tick, onto a routing page — and then
//! never asks what time it is on an event's behalf. Until `E3-B04g` it was a
//! courier: it decoded every entry, kept two coordinates, and relayed them. It is
//! not even that now. It reads positions — where the driver's accumulator ended
//! and where the compositor latched — and two folds, none of which is a reading.
//!
//! `lint-stamp` fails this crate the moment its source names the reading's
//! vocabulary, because `ON_THE_PATH` is a text question rather than a
//! dependency-graph one, and that includes this comment: the steps the
//! compositor follows are RFC 0124's *What the consumer has to do*, spelled with
//! the call names there, and not here.
//!
//! # The shape of the run, and what it costs
//!
//! One worker core, so **the two components run one after the other and the
//! ring holds the events in between**. The frame allocates the driver's data
//! channel and lays it out; the driver is stood up, the harness moves the
//! pointer, and the driver submits onto the ring with nobody draining it; the
//! driver is told to stop, puts its fold on the ring after its last event, and
//! is reaped; the compositor is stood up on the same core **with the same page
//! mapped as its input channel**, and drains it itself before the frame's one
//! commit arrives.
//!
//! What that costs, stated rather than left for a reader to find:
//!
//! **The ring is the buffer, and it is bounded.** Sixteen entries, one of them
//! the attestation. A gesture longer than that is an entry the driver could not
//! submit, which it counts in `dropped`, and the verdict requires `dropped` to be
//! zero — so a longer gesture is a red boot rather than a truncated one. Until
//! `E3-B04g` the frame drained as the driver submitted and held what it took in
//! an array of its own; that array is gone because an array of events in the
//! frame is a frame holding events.
//!
//! **The two do not run at once.** `E1-B05`'s ring-3 supervisor does not hand a
//! place's occupant a core *and* a peer, and `-smp 2` is pinned because the core
//! count is part of every boot log in this tree. So the compositor drains a ring
//! the driver has finished with. Every number the path carries is virtual time
//! out of a seed in any case — `user/virtio-input/src/clock.rs` spends a page on
//! that — so nothing here is a latency and nothing here claims to be. What the
//! arrangement does establish is the route: the entries the compositor decoded
//! are the entries the driver wrote, in the pages the driver wrote them, and
//! this frame read none of them.
//!
//! # Two halves, and the control is the frame's own decision
//!
//! `input=deliver` connects the ring. `input=withheld` is **the identical run
//! with the channel not connected**: the same device, the same injection, the
//! same entries submitted onto the same ring, the same attestation, and the
//! same scene the frame commits — and the compositor is not given the ring. It
//! must then take no position and decline every frame it closes. A compositor
//! that latched in that run would have got a position by some route other than
//! the ring, which is the route this boot exists to show is the only one.
//!
//! # What arrived is what was sent, twice
//!
//! The driver folds every entry it submits into `f_abi::input::Crossing`. It
//! publishes the word on its routing page, as `E3-B04f` had it do, and since
//! `E3-B04g` also puts it on the ring after its last event. The compositor folds
//! every entry it decodes, reads the driver's word off the ring, compares the
//! two, and publishes its fold, the word it read and whether they agreed. This
//! frame then requires three things, none of which it computes: the
//! compositor's own agreement; that the compositor's fold equals the driver's
//! word **as the routing page carried it**, which is a second route for the
//! same number; and that the word the compositor read off the ring is the word
//! on that page. RFC 0124 is why a fold is not a reading, and RFC 0125 is why
//! the word rides the ring.
//!
//! # What this demonstration does not show
//!
//! One device, one queue, relative motion, and no seat. It says nothing about
//! two input devices — [`virtio::VIRTIO_INPUT_MODERN`] cannot tell them apart
//! and the frame takes the first — nothing about focus or about which of several
//! clients an event belongs to, and nothing about latency. What it shows is that
//! a person moving a pointer moves a node in a scene graph held at ring 3,
//! through one reading, one ring the frame never drained, and a latch.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, rights};
use f_abi::scene::{Commit, CreateNode, Delta, Entry as SceneEntry, NO_NODE, SetTransform, kind};
use f_abi::{ABI_VERSION, Cqe, class, control, feature, state};
use f_compositor::routing as scene_routing;
use f_env::{Env, SeededEnv};
use f_interface::backend::Capability;
use f_ring::device::Window;
use f_ring::registry::Domains;
use f_ring::{Arena, Collector, Consumer, Mapping, Poster, Producer};
use f_virtio_input::routing;

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging::{AddressSpace, Features};
use crate::arch::x86_64::pci::{self, Bdf, Survey};
use crate::arch::x86_64::virtio;
use crate::arch::x86_64::vtd::Unit;
use crate::cap::Table;
use crate::compositor::Board;
use crate::iommu;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};
use crate::process::{self, DriverPlan, ServerPlan};
use crate::supervisor::{Declared, Registers, Supervising, declared, order_for};

/// The one address a driver component holds as a constant, agreed — for the
/// sixth time, in a sixth crate.
///
/// `f_virtio_input::routing::AT` is written down in the component and
/// `kernel::process::BOARD` in the frame, and they are linked separately. The
/// kernel is the one artefact that links every definition, so the agreement is a
/// check rather than a comment. RFC 0051 said at two drivers that this constant
/// belongs in `abi/`, RFC 0054 said it again at three, and `user/panel` said it
/// at five; the argument has not improved by being made a sixth time.
const _: () = assert!(
    crate::process::BOARD == routing::AT,
    "the frame and the input driver disagree about where the routing page is"
);

/// The four drivers' routing pages are told apart by their magic and by nothing
/// else.
///
/// They are mapped at one address, in one shape, by one loader. If any two
/// magics were equal, a build that routed one driver's supervisor at another
/// driver's image would find a page whose magic matched and whose fields meant
/// something else.
///
/// `user/virtio-input`'s own `the_four_drivers_do_not_share_a_routing_magic`
/// asserts the same thing against **numbers**, because that crate does not
/// depend on the other three and must not start. This is the assertion the
/// component could not make: the frame links every definition, so here the
/// constants themselves are compared and a driver that changes its magic moves
/// this line rather than a literal in somebody else's test.
const _: () = assert!(
    routing::MAGIC != f_virtio_blk::routing::MAGIC
        && routing::MAGIC != f_virtio_net::routing::MAGIC
        && routing::MAGIC != f_virtio_gpu::routing::MAGIC,
    "the drivers' routing pages are mapped at one address and must not answer to one magic"
);

/// The manifest name of the component file this boot stands up.
const DRIVER: &[u8] = b"virtio-input";

/// Entries on either ring, the driver's and the compositor's. Unit: entries.
///
/// Sixteen, which is what every channel the frame describes in this tree is: one
/// page holds sixteen entries, their completions and an arena with room for
/// ninety-odd payloads, and the driver takes the smaller of the two counts as
/// its arena modulus. The driver's manifest declares two hundred and fifty-six,
/// which is what a component gets when it pays for its own channel.
///
/// **On the driver's ring this is also the bound on a gesture**, since
/// `E3-B04g`: nobody drains that ring while the driver runs — the compositor
/// takes it after — so sixteen is how many entries the two components can
/// have between them, one of which is the driver's attestation. `cargo xtask
/// input` injects five motions, which is six entries. A seventeenth would be
/// an entry the driver counts in `dropped`, and the verdict fails on that
/// rather than quietly losing the end of a gesture.
const ENTRIES: u32 = 16;

/// How long the frame waits for a core to report finished after being told to
/// stop. Unit: microseconds.
const EXIT_MICROS: u64 = 5_000_000;

/// How long the frame holds the driver up waiting for the harness to act.
///
/// Sixty seconds, which is `kernel/src/main.rs`'s `CAPTURE_MICROS` and is chosen
/// the same way: far longer than a harness needs and far shorter than the
/// harness's own boot timeout, so a run nobody answers ends by this number
/// rather than by being killed. RFC 0046 — a hang is a count. This one is
/// counted, it is printed, and the boot carries on either way and reaches a
/// verdict that says nothing was ever injected.
/// Unit: microseconds.
const INJECT_MICROS: u64 = 60_000_000;

/// How long it keeps draining after the harness says it has finished injecting.
///
/// Two seconds. The byte on the serial port says *I have sent them*, which is
/// not the same statement as *the device has delivered them*: the emulator
/// queues an event, raises an interrupt this driver does not yet wait on, and
/// the driver finds it on its next poll. So the frame drains for a settle period
/// rather than stopping on the byte, and the period is generous because the cost
/// of it being too long is two seconds and the cost of it being too short is a
/// count that moves between runs.
/// Unit: microseconds.
const SETTLE_MICROS: u64 = 2_000_000;

/// How many turns the driver's loop spends finding nothing before it gives up.
///
/// Enormous, and a backstop here rather than the mechanism it is on the
/// component's own terms. `user/virtio-input/src/routing.rs` calls `IDLE_SPINS`
/// load-bearing, and it is — for a driver nobody is going to stop. This boot
/// *does* stop it: the frame posts RFC 0008's stop notice when the settle period
/// passes, and a run that reached this number instead would be a run in which
/// nobody touched the machine for a minute, which is a harness that did not
/// inject and which the verdict already names.
///
/// A count of turns and not a duration, for the component's reason, which is
/// stronger than RFC 0004's usual one: the only clock that component holds is
/// virtual, so a duration measured against it would be a count wearing a unit.
/// Unit: turns.
const IDLE_SPINS: u64 = 1_000_000_000_000;

/// The seed the driver's clock is built from.
///
/// Told rather than chosen, so a run reproduces from something the frame decided
/// and a reader can find. One constant in one file, for the reason
/// `kernel/src/compositor.rs` gives about its own: a seed drawn from the boot's
/// shared environment would move when an unrelated stage started drawing, and
/// every entry in this boot would then depend on what else the boot did.
/// Unit: none — a seed.
const STAMP_SEED: u64 = 0x_1A_9B_04_0D_5E_ED_00_01;

/// How far that clock advances per report drained from the device.
///
/// A hundred microseconds of virtual time, and the value matters in exactly two
/// ways. It is non-zero, which the component refuses `stopped::NO_CLOCK` when it
/// is not — the one field on that page whose wrong value produces a run that
/// works and lies. And it is a round number the component's published clock
/// reading divides by exactly, so the frame can check that the clock advanced
/// once per report instead of taking the component's word for it.
///
/// It is **not** a measurement. There is no relationship between this number and
/// how long anything takes; it is the tick of a virtual clock whose whole job is
/// to order events and reproduce from a seed.
/// Unit: nanoseconds per report.
const STAMP_TICK_NANOS: u64 = 100_000;

/// The layer the pointer's node hangs under.
const ROOT_NODE: u32 = 1;

/// The node the pointer moves.
///
/// Two, because node one is the layer it hangs under. Every node is created by
/// this boot's own setup deltas and none is derived from anything the device
/// said, which is what makes the *latched translation* the only part of the
/// frame the compositor submits that came from outside the machine.
const POINTER_NODE: u32 = 2;

/// A sibling of the pointer that never moves.
///
/// **The host fixture's sibling, carried into the boot**, and for its reason:
/// a scene with one transform in it cannot tell a latch that patched the
/// pointer from one that patched every transform it found, because both submit
/// the same frame. With a second transform committed beside the pointer's, a
/// latch that touched it moves `f_compositor::latch::unmoved`'s word — its
/// translation is not masked, only the pointer's is.
const STILL_NODE: u32 = 3;

/// How many nodes the setup deltas create. Unit: nodes.
const SETUP_NODES: u64 = 3;

/// How many transforms this client commits: the pointer's, where the driver's
/// accumulator starts, and the still sibling's. Unit: deltas.
const COMMITTED_TRANSFORMS: u64 = 2;

/// Where this client commits the pointer, along x — **and where this frame
/// tells the driver its accumulator starts**, which is the same number on
/// purpose.
///
/// Where the accumulator starts, and not a position chosen for a screen: this
/// client is never told where the pointer is — that is `E3-B04g` — so the one
/// place it can honestly commit it is where the driver's accumulator starts.
/// `commit`'s own doc is the argument, and it is what makes *latched minus
/// committed* the motion the harness injected rather than an offset from a
/// position somebody made up.
///
/// **Not zero, and that is the point of it.** Until 2026-09-25 the driver could
/// not be told a start, so it began at `(0, 0)` and this client committed
/// there; and over a committed translation of zero, a latch that *added* the
/// position to it — `tx = committed.tx + predicted` — submits exactly the frame
/// a latch that replaced it does. The harness's subtraction and this file's
/// *latched is where the driver's accumulator ended* both passed it; only the
/// host test, which commits at `(640, 360)`, caught it. It is the host
/// fixture's number here too, so the boot catches it on a device path. The
/// same shape as the identity matrix `COMMITTED_A_X65536` replaced, on the
/// other axis of the transform. RFC 0132.
/// Unit: device pixels, scaled by 65 536.
const COMMITTED_TX_X65536: i64 = 640 * 65_536;

/// And along y. Unit: device pixels, scaled by 65 536.
const COMMITTED_TY_X65536: i64 = 360 * 65_536;

// Non-zero on each axis, which is the property; different from each other, so
// a latch that crossed the axes is a number that moved; and inside `i32`, which
// is the accumulator the driver is told to start it in and refuses otherwise.
const _: () = assert!(
    COMMITTED_TX_X65536 != 0
        && COMMITTED_TY_X65536 != 0
        && COMMITTED_TX_X65536 != COMMITTED_TY_X65536
        && COMMITTED_TX_X65536 <= i32::MAX as i64
        && COMMITTED_TY_X65536 <= i32::MAX as i64
        && COMMITTED_TX_X65536 >= i32::MIN as i64
        && COMMITTED_TY_X65536 >= i32::MIN as i64
);

/// Where the still sibling is committed, along x.
///
/// Anywhere but the origin, so that a latch which wrote the pointer's position
/// into it, or zeroed it, is a word that moved.
/// Unit: device pixels, scaled by 65 536.
const STILL_TX_X65536: i64 = 7 * 65_536;

/// And along y. Unit: device pixels, scaled by 65 536.
const STILL_TY_X65536: i64 = -3 * 65_536;

/// The first of the four matrix entries this client commits, in the 16.16
/// fixed point `f_abi::scene::SetTransform` is written in — together a shear
/// with a scale, and the host fixture's own numbers.
///
/// **Not the identity, and that is the point of them.** The latch copies the
/// client's four entries and replaces two translations; a latch that instead
/// wrote a fresh identity matrix around the translation would be a compositor
/// deciding a client's scale — `f_compositor::latch`'s own comment names that
/// defect — and **over an identity commit it submits the same frame as the
/// correct latch**, so no instrument could see it. Committed as a shear, the
/// four entries the latch must not touch are four the fold can see move.
/// Unit: none — a ratio, scaled by 65 536.
const COMMITTED_A_X65536: i64 = 2 * 65_536;
/// See [`COMMITTED_A_X65536`]. Unit: none — a ratio, scaled by 65 536.
const COMMITTED_B_X65536: i64 = 17_000;
/// See [`COMMITTED_A_X65536`]. Unit: none — a ratio, scaled by 65 536.
const COMMITTED_C_X65536: i64 = -9_000;
/// See [`COMMITTED_A_X65536`]. Unit: none — a ratio, scaled by 65 536.
const COMMITTED_D_X65536: i64 = 3 * 65_536;

/// What this boot tells the compositor its display scans out.
///
/// A sixty-hertz frame, which is what `kernel/src/compositor.rs` tells it and
/// for the same reason: there is no display in this boot, so it is a statement
/// rather than a measurement, and what it has to be is a period the one frame
/// this boot commits does not come close to.
/// Unit: nanoseconds.
const SCANOUT_PERIOD_NANOS: u64 = 16_666_667;

/// The display's first scanout, which this frame tells the **driver** so that
/// its own predictor is asked the question the compositor's latch will be.
///
/// The display this boot declares scans out at every multiple of
/// [`SCANOUT_PERIOD_NANOS`] from the channel's epoch — phase zero, which is
/// `f_compositor::pacing::scanout_after`'s own assumption and has that
/// function's reversal: a display that reports its phase. So its first scanout
/// is one period in, and that is a statement about a display and not a reading
/// of anything: RFC 0120 calls the instant a compositor aims at a *target*, and
/// this is the same kind of instant said by the party that declares the
/// display.
///
/// **It is not the compositor's aim, and the verdict does not assume it is.**
/// On one worker core the driver has been reaped before the compositor mints
/// its aim, so the aim cannot reach the driver, and a frame that carried it
/// there would first have had to learn it. The compositor mints its own out of
/// the period and its clock; the steady gesture's verdict then requires that
/// aim, this instant and the instant the driver says it was asked about to be
/// one number — two predictors asked about two instants disagree for a reason
/// that is not a defect, and that is a sentence rather than a pass. RFC 0134.
/// Unit: nanoseconds.
const FIRST_SCANOUT_NANOS: u64 = SCANOUT_PERIOD_NANOS;

/// What it tells the compositor to hold back against its estimate being wrong.
/// Unit: nanoseconds.
const PACING_MARGIN_NANOS: u64 = 1_000_000;

/// How much room the one commit this boot submits leaves before its deadline.
///
/// A whole scanout period, so the frame fits and the degradation policy is never
/// asked. This boot is about where a delta came from and not about pacing, and a
/// commit that was late would put a second subject in its verdict.
/// Unit: nanoseconds.
const COMMIT_SLACK_NANOS: u64 = SCANOUT_PERIOD_NANOS;

/// The seed the compositor's clock and this boot's frame costs are drawn from.
///
/// Its own, and distinct from [`STAMP_SEED`], because the two clocks are two
/// clocks: one is what a driver times an event against and the other is what a
/// compositor paces a frame against. One seed for both would have made the
/// pointer's entries move when the pacing script changed.
/// Unit: none — a seed.
const PACING_SEED: u64 = 0x_C0_11_05_17_1A_9B_04_0D;

/// How far this boot's pacing clock moves between two entries, at most.
/// Unit: nanoseconds.
const STEP_SPREAD_NANOS: u64 = 4096;

/// What this boot tells the compositor the backend under it reports.
///
/// Compute shaders, storage buffers, a CPU and a way to present — the hybrid's
/// set, which RFC 0080's ladder answers rung two for, and the same set
/// `kernel/src/compositor.rs` describes. Built out of `Capability::index()`
/// rather than written as a bitmask, because the bit positions are the
/// vocabulary's and neither end of a routing page may invent them.
///
/// There is no backend. The rung is not this boot's subject; the report is here
/// because a compositor is refused one at all on a machine that satisfies no
/// rung, and `compositor=floorless` is where that refusal is the subject.
/// Unit: none — a bitmask of capability indices.
const BACKEND_CAPABILITIES: u64 = (1 << Capability::ComputeShaders.index())
    | (1 << Capability::StorageBuffers.index())
    | (1 << Capability::Cpu.index())
    | (1 << Capability::ImagePresent.index());

/// What this boot's one commit calls its frame.
///
/// One rather than a memorable word: a frame identifier is the submitter's and
/// is opaque to the compositor, so all this client needs of it is that it is not
/// zero — `f_abi::scene` refuses a zero, so a zeroed payload is not a commit of
/// frame zero.
/// Unit: none — a frame identifier, not a quantity.
const FRAME_ONE: u64 = 1;

/// The rights a handle to the driver's own queue region carries.
///
/// Read, write, and the right to hand the memory to a device. The third is what
/// separates this handle from one that merely names the same bytes, and the
/// separation is the whole of RFC 0047's answer to *why may a driver not grant
/// itself a device address*.
const GRANTABLE: u8 = rights::READ | rights::WRITE | rights::GRANT;

/// Which half of the check this boot is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    /// Give the compositor the other end of the driver's ring.
    Deliver,
    /// The same run with the ring not connected.
    ///
    /// **The control, and it is the frame's own decision rather than the
    /// device's.** The events exist, are submitted, and sit on the ring; the
    /// compositor is simply not given it. An entry with no reading on it was not
    /// chosen, because `abi/src/input.rs` refuses one before it reaches anybody
    /// and a boot built around it would be re-running a decoder's test.
    Withheld,
}

impl Half {
    /// A word for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Deliver => "deliver",
            Self::Withheld => "withheld",
        }
    }

    /// Does the frame connect the driver's ring to the compositor?
    #[must_use]
    pub const fn connects(self) -> bool {
        matches!(self, Self::Deliver)
    }
}

/// Which motion the harness says it injects, and so which latch the verdict
/// requires.
///
/// Not a [`Half`], because it is not a control: both gestures give the
/// compositor the ring, and what differs is whether the motion turns inside
/// the predictor's window. The frame cannot see the motion — the harness is the
/// process that moved the pointer — so the harness says which one it is, on the
/// command line beside `input=deliver`, and the verdict holds it to what it
/// said: a steady gesture that held is refused, and so is a turning one that
/// extrapolated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gesture {
    /// The motion turns inside the window, so both predictors **hold** the
    /// newest report. `E3-B01i`'s gesture: latched minus committed is exactly
    /// the motion, and the seam agrees only by copying one sample.
    Turning,
    /// The motion does not turn, so both predictors **extrapolate**. `E3-B04e`'s
    /// gesture, and the one on which a held latch is refused as vacuous.
    Steady,
}

impl Gesture {
    /// The gesture a boot's command line names: `gesture=steady`, or turning.
    #[must_use]
    pub fn of(boot: &BootInfo) -> Self {
        if boot.has_parameter(b"gesture=steady") { Self::Steady } else { Self::Turning }
    }

    /// A word for the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Turning => "turning",
            Self::Steady => "steady",
        }
    }
}

/// Why the demonstration could not be run.
///
/// None of these is the result being looked for. A path that could not be set up
/// is not a path that was exercised, and the boot says so rather than reporting
/// a pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// The device could not be found or routed. The finder's own reason.
    Device(virtio::Trouble),
    /// The remapping unit refused something. Its own reason, unchanged.
    Unit(crate::arch::x86_64::vtd::Refuse),
    /// The allocator had no block for the driver's region or for a channel.
    NoFrames,
    /// A capability table could not hold the handles the grant is made of. A bug
    /// here rather than a machine property.
    Authority,
    /// The frame refused a translation for memory whose holder does hold a
    /// grantable capability for it.
    Refused,
    /// A channel could not be laid out, or one of its ends could not bind.
    Channel(i32),
    /// No boot module carries a component file declaring this driver.
    NoManifest,
    /// The driver's manifest and this machine disagree about what has to be
    /// routed.
    Manifest,
    /// No boot module carries a component file named `compositor`.
    NoCompositor,
    /// The compositor's record declares no heap, so its graph has nowhere to go.
    NoHeap,
    /// That heap and `f_compositor::routing::HEAP_BYTES` are different numbers.
    HeapDisagrees,
    /// The compositor's record serves no `scene` ring, so there is no cap to
    /// hand it. `crate::compositor::Trouble::Unframed`'s reason, RFC 0128.
    Unframed,
    /// A component could not be built as a process, carrying which step.
    Process(crate::process::Error),
    /// A component's state tree could not be published or read back.
    StateTree(i32),
    /// The core a component was given never took it or never gave it back.
    Scheduled(usize),
    /// A core was still holding its job when the frame's bound passed. Carries
    /// that bound. Unit: microseconds.
    Overdue(u64),
    /// A delta was refused by the ring, or a completion never arrived.
    NotCommitted,
    /// A component published a board this build cannot read.
    BadReport,
    /// The frame's own count of what it took and what it gave back disagreed.
    Leaked,
}

impl Trouble {
    /// A sentence for the boot log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Device(why) => why.message(),
            Self::Unit(why) => why.message(),
            Self::NoFrames => "no frames for the driver's region or for a channel",
            Self::Authority => "the demonstration could not mint the capabilities it is made of",
            Self::Refused => "the frame refused a translation for memory its holder may grant",
            Self::Channel(_) => "a ring could not be laid out or bound",
            Self::NoManifest => "no boot module declares the virtio-input component",
            Self::Manifest => {
                "the driver's manifest and this machine disagree about what has to be routed"
            }
            Self::NoCompositor => "no boot module carries a component file named compositor",
            Self::NoHeap => "the compositor's manifest declares no heap for its graph",
            Self::HeapDisagrees => {
                "the manifest's heap and the compositor crate's own constant are different numbers"
            }
            Self::Unframed => {
                "the compositor's record serves no `scene` ring, so there is no \
                 deltas_per_frame_max to hand it (RFC 0128)"
            }
            Self::Process(_) => "a component could not be built as a process",
            Self::StateTree(_) => "a component's state tree could not be published or read",
            Self::Scheduled(_) => "the core a component was given never took it or gave it back",
            Self::Overdue(_) => "a component's core did not report finished inside the bound",
            Self::NotCommitted => "the compositor refused a delta the client had room for",
            Self::BadReport => "a component published a board this build cannot read",
            Self::Leaked => "the demonstration's frames did not all come back",
        }
    }

    /// The wall-clock bound this refusal is, when it is one.
    ///
    /// One variant is not a finding: it is a spin that ran out of a number
    /// derived from `tsc_khz`, so it fires for a component that is wedged and
    /// for a runner slower than the number alike, and nothing here can tell
    /// those apart. Printing it under the same sentence as the rest is how a
    /// slow machine comes to be read as a defect on this path.
    /// Unit: microseconds.
    #[must_use]
    pub const fn bound(self) -> Option<u64> {
        match self {
            Self::Overdue(micros) => Some(micros),
            _ => None,
        }
    }
}

/// The shared half's refusals, in this file's words.
///
/// One arm per variant and no message changes, which is the point:
/// `kernel/src/supervisor.rs` answers its own four-variant `Trouble` because the
/// drivers' enums are not one enum, and a conversion that reworded anything
/// would change a boot log somebody is reading against another driver's.
impl From<crate::supervisor::Trouble> for Trouble {
    fn from(trouble: crate::supervisor::Trouble) -> Self {
        match trouble {
            crate::supervisor::Trouble::NoManifest => Self::NoManifest,
            crate::supervisor::Trouble::Manifest => Self::Manifest,
            crate::supervisor::Trouble::Channel(refusal) => Self::Channel(refusal),
            // The shared half answers this for a driver that owed an answer and
            // did not give one. This driver owes none — nobody asks for an input
            // event — so the frame never calls the function that produces it,
            // and the arm is here because a total conversion is what keeps the
            // other three honest rather than because this path can reach it.
            crate::supervisor::Trouble::NoAnswer(micros) => Self::Overdue(micros),
        }
    }
}

/// Where and for how long the components run.
#[derive(Clone, Copy)]
pub struct Scheduling {
    /// Which core each of them is given, one after the other.
    /// Unit: none — a core index.
    pub cpu: usize,
    /// The rate that core arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks it asks for. Unit: timer ticks.
    pub target: u64,
    /// This machine's timestamp-counter rate, for bounding the waits.
    /// Unit: kilohertz.
    pub tsc_khz: u64,
    /// The physical address of the frame the **frame's** state tree is published
    /// in, which every shape maps read-only. Unit: bytes, physical.
    pub tree: u64,
}

/// What the driver's own board said when its run ended.
#[derive(Clone, Copy, Default)]
pub struct Reported {
    /// `virtio_input_event` records read off the device. Unit: records.
    pub records: u64,
    /// Reports that closed. Unit: reports.
    pub reports: u64,
    /// Reports that took a reading. Unit: reports.
    pub stamped: u64,
    /// Entries put on the data ring. Unit: entries.
    pub submitted: u64,
    /// Entries built and not submitted, because the ring had no room.
    /// Unit: entries.
    pub dropped: u64,
    /// Records this build has no opcode for. Unit: records.
    pub ignored: u64,
    /// Entries the component's own format check refused. Unit: entries.
    pub malformed: u64,
    /// Turns of its loop that found nothing anywhere. Unit: turns.
    pub spun: u64,
    /// How far its clock had advanced when the run ended.
    /// Unit: nanoseconds of virtual time.
    pub clock_at: u64,
    /// Why its loop ended, as a `routing::stopped` ordinal. Unit: none.
    pub outcome: u64,
    /// What it put on the data ring, folded into one word.
    ///
    /// The **producer's** half, published by the component and read here. This
    /// frame never computes it and could not: it is a fold over every entry the
    /// driver submitted, taken in the driver. Since `E3-B04g` the same word also
    /// crosses the ring, to the compositor, and this copy is the second route —
    /// the one the frame holds the compositor's fold against.
    /// Unit: none — a checksum.
    pub crossing: u64,
    /// How many entries went into that word, as the component counted them.
    /// Unit: entries.
    pub crossed: u64,
    /// One where that word also went onto the ring, after the last event.
    /// Unit: none — a flag.
    pub attested: u64,
    /// How many of the entries that crossed were pointer motion.
    /// Unit: entries.
    pub motions: u64,
    /// Where the driver's accumulator ended up, along x.
    /// Unit: device pixels, scaled by 65 536.
    pub pointer_x_x65536: i64,
    /// The same along y. Unit: device pixels, scaled by 65 536.
    pub pointer_y_x65536: i64,
    /// The scanout the driver's own predictor was asked about, as it read the
    /// word this frame wrote. Zero where it predicted nothing.
    ///
    /// **The input path's half of `E3-B04e`, and the seven fields after this
    /// one are the rest of it.** Positions, a flag, a count, a target and a
    /// lead — and no reading: this frame compares them with the compositor's
    /// and computes nothing out of either. RFC 0134.
    /// Unit: nanoseconds.
    pub predicted_for: u64,
    /// Where the driver's predictor put the pointer, along x.
    /// Unit: device pixels, scaled by 65 536.
    pub predicted_x_x65536: i64,
    /// And along y. Unit: device pixels, scaled by 65 536.
    pub predicted_y_x65536: i64,
    /// The newest report that prediction stands on, along x.
    /// Unit: device pixels, scaled by 65 536.
    pub anchor_x_x65536: i64,
    /// And along y. Unit: device pixels, scaled by 65 536.
    pub anchor_y_x65536: i64,
    /// How far forward the driver's predictor extrapolated. Unit: nanoseconds.
    pub predicted_lead_nanos: u64,
    /// One where it extrapolated, zero where it held. Unit: none — a flag.
    pub predicted_extrapolated: u64,
    /// Positions the driver's predictor took. Unit: reports.
    pub predictor_reports: u64,
}

impl Reported {
    /// Read it, and answer nothing for a component that never finished writing
    /// one.
    ///
    /// The magic is what tells those two apart, and it matters more here than on
    /// any driver before it: a component that faulted before it reached its own
    /// report would otherwise publish a count of zero readings, which is
    /// indistinguishable from a run in which nobody touched the machine.
    fn of(board: &Window) -> Option<Self> {
        let read = |offset: u32| board.read64(offset).unwrap_or(0);
        if read(routing::reported::MAGIC) != routing::MAGIC {
            return None;
        }
        Some(Self {
            records: read(routing::reported::RECORDS),
            reports: read(routing::reported::REPORTS),
            stamped: read(routing::reported::STAMPED),
            submitted: read(routing::reported::SUBMITTED),
            dropped: read(routing::reported::DROPPED),
            ignored: read(routing::reported::IGNORED),
            malformed: read(routing::reported::MALFORMED),
            spun: read(routing::reported::SPUN),
            clock_at: read(routing::reported::CLOCK_AT),
            outcome: read(routing::reported::OUTCOME),
            crossing: read(routing::reported::CROSSING),
            crossed: read(routing::reported::CROSSED),
            attested: read(routing::reported::ATTESTED),
            motions: read(routing::reported::MOTIONS),
            // Two's complement in a word, back to signed. A pointer left of the
            // origin is an ordinary place and not an enormous one.
            pointer_x_x65536: read(routing::reported::POINTER_X_X65536) as i64,
            pointer_y_x65536: read(routing::reported::POINTER_Y_X65536) as i64,
            predicted_for: read(routing::reported::PREDICTED_FOR_NANOS),
            predicted_x_x65536: read(routing::reported::PREDICTED_X_X65536) as i64,
            predicted_y_x65536: read(routing::reported::PREDICTED_Y_X65536) as i64,
            anchor_x_x65536: read(routing::reported::ANCHOR_X_X65536) as i64,
            anchor_y_x65536: read(routing::reported::ANCHOR_Y_X65536) as i64,
            predicted_lead_nanos: read(routing::reported::PREDICTED_LEAD_NANOS),
            predicted_extrapolated: read(routing::reported::PREDICTED_EXTRAPOLATED),
            predictor_reports: read(routing::reported::PREDICTOR_REPORTS),
        })
    }
}

/// What the driver stage saw.
///
/// Nothing in it came off the data ring. That is the difference `E3-B04g`
/// made to this type: it held the frame's drain counts, its fold and an array
/// of kept events, and all of that is the compositor's now.
pub struct Produced {
    /// What the component published about itself.
    pub board: Reported,
    /// Whether it ended by `EXIT` rather than in a fault.
    pub exited: bool,
    /// Operations the driver asked of the frame on its control ring.
    ///
    /// **Zero, and that is not a failure.** The three drivers before this one
    /// submit RFC 0047's `DEVICE_MAP` because they hold a client's buffer and
    /// may not grant themselves a device address. This one holds no client
    /// buffer: the only memory its device ever touches is the queue region its
    /// manifest routed, which the frame translated before the component's first
    /// instruction. Unit: operations.
    pub asked: u32,
    /// Whether anything outside the machine said it had injected.
    pub acknowledged: bool,
    /// The data ring's producer cursor when the driver had been reaped and
    /// before the compositor was stood up.
    ///
    /// **The measurement behind *the frame relayed no input entry*.** Until a
    /// mutation that had this frame take one entry off the ring went red on the
    /// wrong sentence, the boot log said *this frame took 0 entries* as a
    /// literal: true of this file, and printed whatever this file did. So it is
    /// read off the ring's own cursors instead, at the one moment nothing is
    /// running on either end.
    ///
    /// **The cursors, and not their difference.** Until 2026-09-25 this was
    /// `Producer::occupancy`, `head - tail`, and an audit showed by reading that
    /// a relay which popped all six entries and resubmitted them to the same
    /// ring left head 12 and tail 6: occupancy six, both folds unchanged, the
    /// verdict green. A ring nobody has taken from is at one place and not at a
    /// difference — tail zero, where `describe` laid it out, and head at every
    /// event and attestation the driver says it put there.
    /// Unit: entries, as a free-running cursor.
    pub head: u64,
    /// The consumer cursor at the same moment: zero, or somebody consumed.
    ///
    /// *What this cannot see:* a reader that copies entries without advancing
    /// it, or one that puts both cursors back afterwards. Neither is reachable
    /// from this file without a `Consumer` or a `Cursor::set` on this ring, and
    /// the absence of both is `E3-B04g`'s structural half; this is the half a
    /// boot can measure. Unit: entries, as a free-running cursor.
    pub tail: u64,
}

/// What the compositor stage saw.
pub struct Consumed {
    /// Deltas the frame put on the scene ring. Unit: deltas.
    pub submitted: u64,
    /// Completions it reaped. Unit: deltas.
    pub completed: u64,
    /// Completions carrying a refusal, or answering a delta nobody sent.
    /// Unit: deltas.
    pub refused: u64,
    /// What the component published on its board.
    pub board: Board,
    /// Nodes its declared schema carries, as the frame wrote them. Unit: nodes.
    pub tree_nodes: u32,
    /// The three words of its tree this boot reads back: frames, edits, nodes.
    /// Unit: as the component's manifest declares them.
    pub tree: [u64; 3],
}

/// What the boot saw.
pub struct Report {
    /// Which half ran.
    pub half: Half,
    /// Which gesture the harness says it injected.
    pub gesture: Gesture,
    /// What the driver's manifest declares.
    pub declared: Declared,
    /// Which function the device is.
    pub bdf: Bdf,
    /// How many pages of register window it published. Unit: pages.
    pub windows: u32,
    /// Which core both components ran on. Unit: none — a core index.
    pub cpu: usize,
    /// The driver stage.
    pub produced: Produced,
    /// The compositor stage.
    pub consumed: Consumed,
}

/// The three nodes of the compositor's tree this boot reads back, by id.
///
/// Frames, edits and nodes — the three the script has an opinion about. The five
/// `E3-B01k` added are about pacing and a rung, which this boot describes and
/// does not test; reading them here would be a second place they are asserted
/// and `compositor=serve` is the first.
///
/// Taken from `f_compositor::routing::node` rather than written down, because
/// the ids are the component's and a boot carrying its own copy is the one that
/// goes stale.
/// Unit: none — node identifiers.
const WATCHED: [u32; 3] =
    [scene_routing::node::FRAMES, scene_routing::node::EDITS, scene_routing::node::NODES];

/// A word off a page as the signed number it was written as.
///
/// The compositor publishes translations as two's-complement `u64` words, for
/// the reason `f_compositor::routing` gives about its own pointer words.
const fn signed(word: u64) -> i64 {
    word as i64
}

impl Report {
    /// How many deltas should have reached the graph. Unit: deltas.
    ///
    /// The setup nodes and the two committed transforms, on both halves: the
    /// scene this frame commits is the same whichever half runs, which is what
    /// makes the two halves differ in exactly the one thing — whether the
    /// compositor was given the ring.
    #[must_use]
    pub const fn expected_edits(&self) -> u64 {
        SETUP_NODES + COMMITTED_TRANSFORMS
    }

    /// Did the machine do what this half asked of it?
    ///
    /// # Errors
    ///
    /// A sentence for the boot log, naming the clause that did not hold.
    pub fn verdict(&self) -> Result<(), &'static str> {
        self.driver_verdict()?;
        self.compositor_verdict()?;
        match self.half {
            Half::Deliver => self.drained_verdict(),
            Half::Withheld => self.withheld_verdict(),
        }
    }

    /// The half of the verdict that is about the device, the driver and the
    /// reading, and which is **the same on both halves** — that being the whole
    /// point of the control.
    fn driver_verdict(&self) -> Result<(), &'static str> {
        let seen = &self.produced;
        if !seen.acknowledged {
            return Err(
                "nothing outside the machine said it had injected an event, so this run says \
                 only that a driver was stood up and nobody touched the pointer",
            );
        }
        if !seen.exited {
            return Err("the driver did not end by EXIT, so its own report is a page it may not \
                 have finished writing");
        }
        if seen.board.outcome != routing::stopped::TOLD {
            return Err(
                "the driver's loop ended for a reason other than the frame telling it to: a run \
                 that gave up on its own, or one that found its device or its ring disagreeing \
                 with it, is not the run this boot asked for",
            );
        }
        if seen.board.reports == 0 {
            return Err(
                "the device closed no report, so nothing the user did reached the driver and \
                 every count below is a count of nothing",
            );
        }
        // Where `E3-B04a` lands, as two readings neither of which is derived
        // from the other: the component says how many reports it timed, and the
        // clock's own position says how many times it advanced. The third — that
        // every entry that arrived carried one — is the compositor's decoder's,
        // and is in `drained_verdict`.
        if seen.board.stamped != seen.board.reports {
            return Err(
                "the driver timed a different number of reports than it closed: one reading per \
                 report is what `at_interrupt` is called once for, and two counts that must \
                 agree are the only form in which that claim can be checked",
            );
        }
        if seen.board.clock_at != seen.board.stamped.saturating_mul(STAMP_TICK_NANOS) {
            return Err(
                "the driver's clock is not its tick times the number of readings it took, so the \
                 number each event carries has come unstuck from the report it belongs to",
            );
        }
        if seen.board.malformed != 0 {
            return Err("the driver built an entry its own format check refused, which is an \
                 encoder about to submit what its peer's decoder will not take");
        }
        if seen.board.dropped != 0 {
            return Err(
                "the driver could not submit an entry because the ring had no room: the ring is \
                 the only buffer between the two components on this machine, and an event that \
                 did not fit is one the consumer never saw — the one failure on this path that \
                 leaves every other count looking healthy",
            );
        }
        if seen.board.crossed != seen.board.submitted {
            return Err(
                "the driver folded a different number of entries than it counted submitting, so \
                 its own submit path counts an entry it did not attest to or attests to one it \
                 did not send",
            );
        }
        // The attestation is the driver's on both halves: it does not know
        // whether anybody is connected to the far end, and must not.
        if seen.board.attested != 1 {
            return Err(
                "the driver did not put its fold on the ring after its last event, so a consumer \
                 that is not the frame has nothing to check what it drained against",
            );
        }
        if seen.board.motions == 0 {
            return Err(
                "no entry the driver submitted was pointer motion, so nothing in this run carries \
                 a coordinate the harness can check its own injection against",
            );
        }
        // **`E3-B04g`'s *the frame relaying no input entry at all*, measured.**
        // Every entry the driver put on the ring — its events and its
        // attestation — must still be there when the compositor is given it.
        // On both halves: the withholding half is the same ring with the same
        // entries, and a control that lost one would be a control for two
        // things. The two cursors each against where an untouched ring leaves
        // it, and not their difference — `Produced::head`'s doc has the relay
        // that a difference cannot see. The ring holds sixteen and is laid out
        // at zero, so these cursors have not wrapped and equality is exact.
        if seen.tail != 0 {
            return Err("the ring's consumer cursor had moved when the compositor was stood up: \
                 something took entries off it between the two components - whether or not it \
                 put them back, which the difference of the two cursors cannot see - and on \
                 this path nothing may, the frame least of all");
        }
        if seen.head != seen.board.submitted.saturating_add(seen.board.attested) {
            return Err(
                "the ring's producer cursor is not at every entry the driver says it put there \
                 when the compositor was stood up: something besides the driver submitted to \
                 it, or the driver's own count is wrong",
            );
        }
        // The input path's own predictor, `E3-B04e`, on every half: the driver
        // does not know whether anybody is connected, and predicts either way.
        // Fed where a report closes rather than where an entry is submitted, so
        // these are two paths through the driver agreeing.
        if seen.board.predictor_reports != seen.board.motions {
            return Err(
                "the driver's own predictor took a different number of positions than the \
                 driver sent as motion, so the input path's side of the seam predicts from a \
                 window the compositor was never given",
            );
        }
        if seen.board.predicted_for != FIRST_SCANOUT_NANOS {
            return Err(
                "the driver's own predictor was not asked about the scanout this frame told it, \
                 so whatever it answered is an answer to a different question",
            );
        }
        // The predictor's newest report and the accumulator's end are two
        // numbers inside the driver, taken on two paths, and they must be one
        // place: the anchor is where the prediction starts from.
        if seen.board.anchor_x_x65536 != seen.board.pointer_x_x65536
            || seen.board.anchor_y_x65536 != seen.board.pointer_y_x65536
        {
            return Err("the driver's own predictor stands on a report that is not where its \
                 accumulator ended, so the input path's prediction is anchored somewhere the \
                 device did not leave the pointer");
        }
        if seen.asked != 0 {
            return Err(
                "the driver asked the frame to translate something. It holds no client buffer \
                 and has nothing to translate, so a request here is a driver doing something \
                 this build has no account of",
            );
        }
        Ok(())
    }

    /// The half that is about the scene this frame committed, and which is also
    /// the same on both halves.
    fn compositor_verdict(&self) -> Result<(), &'static str> {
        let seen = &self.consumed;
        if seen.refused != 0 {
            return Err("the compositor refused a delta this frame submitted");
        }
        if seen.completed != seen.submitted {
            return Err("the compositor answered a different number of deltas than it was sent");
        }
        if seen.board.outcome != scene_routing::stopped::TOLD {
            return Err("the compositor's loop ended for a reason other than the frame telling \
                 it to");
        }
        if seen.board.refused != 0 {
            return Err("the compositor refused an entry, so a delta this frame built is not one \
                 that crate will take");
        }
        if seen.board.frames != 1 || seen.board.named != FRAME_ONE {
            return Err("the compositor did not close exactly the one frame this boot commits");
        }
        if seen.board.created != SETUP_NODES || seen.board.live != SETUP_NODES {
            return Err("the compositor's graph does not hold the three nodes this boot's setup \
                 deltas create");
        }
        // `edits` is deltas that reached the graph, counted by the component.
        // The latch is not an edit — it patches the frame going out and not the
        // client's graph through a delta — so a compositor that counted its own
        // latch, or took a position as a delta, lands here.
        if seen.board.edits != self.expected_edits() {
            return Err(
                "the compositor applied a different number of deltas than this frame sent it: \
                 three setup nodes and two committed transforms, on both halves",
            );
        }
        // And the tree, read back out of the page the component publishes into
        // rather than off its board. The two come from one set of counters,
        // which is exactly why disagreeing is worth failing on.
        if seen.tree_nodes == 0 {
            return Err("the compositor's manifest published no state tree");
        }
        if seen.tree != [seen.board.frames, seen.board.edits, seen.board.live] {
            return Err("the numbers in the compositor's published tree are not the numbers on \
                 its board, though both come out of one set of counters");
        }
        Ok(())
    }

    /// The delivering half: the compositor held the ring, and everything the
    /// driver put on it reached the latch by that route and no other.
    ///
    /// `E3-B04g` first, then `E3-B01i`. The order is the dependency: a latch
    /// clause checked against positions that did not provably come off the
    /// ring would be a latch clause about somebody's positions.
    fn drained_verdict(&self) -> Result<(), &'static str> {
        let driver = &self.produced.board;
        let seen = &self.consumed.board;
        // --- `E3-B04g`: the compositor drained the ring itself ---------------
        if seen.input_connected != 1 {
            return Err("the compositor did not adopt the input ring this half gave it");
        }
        if seen.input_entries != driver.submitted {
            return Err(
                "the driver and the compositor disagree about how many entries crossed the data \
                 ring, and the ring held every one of them when the compositor was given it — so \
                 the difference is the compositor's drain, and not a relay's",
            );
        }
        if seen.input_refused != 0 || seen.input_decoded != seen.input_entries {
            return Err(
                "an entry the compositor took off the ring did not decode: every input entry \
                 carries a reading because the decoder refuses one that does not, so a refusal \
                 here is an event that reached a consumer with no place in the latency chain",
            );
        }
        if seen.input_crossed != seen.input_decoded {
            return Err("the compositor folded a different number of entries than it decoded, \
                 which is its own arithmetic and not the driver's");
        }
        if seen.input_attestations != 1 || seen.input_unattested != 0 {
            return Err(
                "the compositor did not find exactly one attestation after the last event: none \
                 is a driver whose word never arrived, two is a producer saying two things \
                 about one crossing, and an entry after it is one the word does not cover",
            );
        }
        // The same number by two routes. The ring carried it to the component;
        // the driver's routing page carried it here, read before the reap. A
        // ring that moved the events faithfully and the word wrongly passes the
        // component's own check and fails this one.
        if seen.input_attested != driver.crossing || seen.input_attested_count != driver.crossed {
            return Err(
                "the attestation the compositor read off the ring is not the word the driver \
                 published on its routing page, so one of the two routes carried it wrongly",
            );
        }
        // **The clause `E3-B04g` is.** The compositor's fold over what it
        // drained against the driver's fold over what it submitted — two words,
        // neither computed here, neither computed from the other. The count is
        // required non-zero first, for `Crossing::agrees_with`'s reason: two
        // folds of nothing hold one basis and would agree about a crossing that
        // never happened.
        if seen.input_crossed == 0
            || seen.input_crossing != driver.crossing
            || seen.input_crossed != driver.crossed
        {
            return Err(
                "what arrived is not what was sent: the driver's fold over the entries it \
                 submitted and the compositor's fold over the entries it drained disagree, and \
                 the fold covers the opcode and the whole payload - so a reading minted on \
                 arrival, a pair handed on out of order, one dropped out of the middle, or a \
                 coordinate moved by one all land here",
            );
        }
        if seen.input_agreed != 1 {
            return Err(
                "the compositor's own comparison of what it drained against the word it read off \
                 the ring did not agree, though this frame's did: the component's check is not \
                 the check its published numbers describe",
            );
        }
        if seen.input_motions != driver.motions {
            return Err("the compositor decoded a different number of positions than the driver \
                 says it sent");
        }

        // --- `E3-B01i`: every position reached the latch, and one frame
        // latched in the window ------------------------------------------------
        if seen.pointer_reports != seen.input_motions || seen.pointer_stale != 0 {
            return Err(
                "a position the compositor decoded did not reach its predictor as a report, or \
                 one reached it twice: a relay that duplicated or reordered motion looks \
                 exactly like this, and there is no relay here",
            );
        }
        if seen.latches != 1 || seen.latch_declines != 0 {
            return Err(
                "the compositor did not latch the one frame it closed, though it was told which \
                 node carries the pointer and took every position the driver sent",
            );
        }
        if seen.latch_entry != 0 {
            return Err(
                "the latch landed past a submission in the frame's trace: zero entries is after \
                 the commit closed and before the compositor's own submission entered its wait, \
                 and that is the only moment `E3-B01i` allows",
            );
        }
        // *And by nothing else.* The component folded every field of every node
        // a renderer could see, with the pointer's two translations masked,
        // either side of the patch — `f_compositor::latch::unmoved` — and the
        // two words must be one. The walk must also have been the whole graph,
        // or two folds of too little would agree about it.
        if seen.latch_walked != seen.live || seen.latch_unmoved_before != seen.latch_unmoved_after {
            return Err(
                "the latched frame differs from the committed one in something other than the \
                 pointer's two translations: the component's fold over every node, with those \
                 two masked, moved across the patch or walked fewer nodes than the graph holds",
            );
        }
        if signed(seen.latch_committed_x) != COMMITTED_TX_X65536
            || signed(seen.latch_committed_y) != COMMITTED_TY_X65536
        {
            return Err(
                "the transform the compositor says the client committed is not the one this \
                 frame committed, and it was read back out of the graph — so the graph holds \
                 something this client did not send",
            );
        }
        match self.gesture {
            Gesture::Turning => self.held_verdict(),
            Gesture::Steady => self.seam_verdict(),
        }
    }

    /// The turning gesture: `E3-B01i`'s *by exactly the motion*, which needs
    /// the latch to hold.
    fn held_verdict(&self) -> Result<(), &'static str> {
        let driver = &self.produced.board;
        let seen = &self.consumed.board;
        // Held and not extrapolated, and the reason is the motion the harness
        // injects rather than a preference: it reverses inside the predictor's
        // window, so the predictor holds the newest position rather than running
        // a velocity that has changed sign — `f_compositor`'s own
        // `the_boots_motion_is_held_and_not_extrapolated` pins that arithmetic.
        // A held latch differs from the commit by exactly the motion that
        // arrived, which is the equality `cargo xtask input` checks; an
        // extrapolated one would differ by that plus a lead, which is
        // `E3-B04e`'s arithmetic and the steady gesture's.
        if seen.latch_extrapolated != 0 || seen.latch_lead_nanos != 0 {
            return Err(
                "the latch extrapolated, so the transform it submitted is the motion plus a \
                 lead and not the motion: the harness's equality would be testing the \
                 predictor rather than the route",
            );
        }
        // And the value, against the driver's own word for where the pointer
        // ended — which reached this frame on the driver's routing page, while
        // the compositor's reached the latch through the ring. Two sides of one
        // ring, neither copied from the other.
        if signed(seen.latch_x) != driver.pointer_x_x65536
            || signed(seen.latch_y) != driver.pointer_y_x65536
        {
            return Err("the position the compositor latched is not where the driver says its \
                 accumulator ended: the newest report the latch held is not the newest report \
                 the driver sent");
        }
        // And the input path's own predictor held too, on its own window. The
        // two agree — and that agreement is a copy of one sample on each side,
        // which is why it is checked here and counted for nothing: `E3-B04e` is
        // the steady gesture's.
        if driver.predicted_extrapolated != 0
            || signed(seen.latch_x) != driver.predicted_x_x65536
            || signed(seen.latch_y) != driver.predicted_y_x65536
        {
            return Err(
                "on a gesture that turns inside the window the driver's own predictor did not \
                 hold where the compositor's did: two instances of one predictor, over one \
                 sequence of reports, reached two different bases",
            );
        }
        Ok(())
    }

    /// The steady gesture: `E3-B04e`. **The latched value is the predicted
    /// one, asserted from both sides of the seam.**
    ///
    /// Two predictor instances, two states: the driver's, fed where each
    /// report closed and before anything crossed; the compositor's, fed from
    /// what it decoded off the ring. Neither number is copied from the other
    /// and this frame computes neither — it compares. *How far it moved* is the
    /// harness's to close, because it holds the one number neither side does:
    /// the compositor moved the node by `latched - committed`, the input path
    /// predicted `predicted - anchor` beyond its newest report, and the harness
    /// requires the first to be its own injected sum plus the second.
    fn seam_verdict(&self) -> Result<(), &'static str> {
        let driver = &self.produced.board;
        let seen = &self.consumed.board;
        // One question, asked of both. The compositor minted its aim out of the
        // period and its clock; this frame told the driver the display's first
        // scanout; the driver says which it was asked. Three numbers, and a
        // seam over two different instants would disagree for a reason that is
        // not a defect — so it is refused before the values are compared.
        if seen.latch_aim_nanos != FIRST_SCANOUT_NANOS
            || driver.predicted_for != seen.latch_aim_nanos
        {
            return Err(
                "the compositor aimed its latch at a different scanout from the one the input \
                 path's predictor was asked about, so the two predictions answer two questions \
                 and their agreement or disagreement means nothing",
            );
        }
        // **The vacuity refusal the host test makes, made in the boot.** A held
        // prediction on either side agrees with the other by copying one sample;
        // an extrapolation that did not move the pointer is the same copy under
        // another name. Both must have extrapolated, and the input path's must
        // have run the pointer off its anchor on both axes.
        if seen.latch_extrapolated != 1 || driver.predicted_extrapolated != 1 {
            return Err(
                "a steady gesture held on one side of the seam or both: a held prediction \
                 agrees with the other side by copying one sample, so this boot would prove \
                 nothing about the latched value being the predicted one",
            );
        }
        if driver.predicted_x_x65536 == driver.anchor_x_x65536
            || driver.predicted_y_x65536 == driver.anchor_y_x65536
        {
            return Err(
                "the input path's prediction did not move the pointer off its newest report on \
                 both axes, so an agreement over it is an agreement about that report and not \
                 about a prediction",
            );
        }
        // The value, from both sides.
        if signed(seen.latch_x) != driver.predicted_x_x65536
            || signed(seen.latch_y) != driver.predicted_y_x65536
        {
            return Err(
                "the value the compositor latched is not the value the input path predicted for \
                 the same scanout from the same reports: the latch submitted something other \
                 than its predictor's answer, or its predictor was fed a different window",
            );
        }
        // And the lead, which the host test also holds: two predictors that
        // agreed on a value by different leads agreed by accident.
        if seen.latch_lead_nanos != driver.predicted_lead_nanos || seen.latch_lead_nanos == 0 {
            return Err("the compositor's latch and the input path's prediction extrapolated by \
                 different leads, or by none");
        }
        Ok(())
    }

    /// The withholding half: the same run with the ring not connected, and
    /// nothing may reach the latch.
    fn withheld_verdict(&self) -> Result<(), &'static str> {
        let seen = &self.consumed.board;
        if seen.input_connected != 0 {
            return Err(
                "the withholding half connected the input ring, so it is not the control it \
                 claims to be and the delivering half's latch is evidence of nothing",
            );
        }
        if seen.input_entries != 0 || seen.input_attestations != 0 {
            return Err("the compositor took input entries on a run in which it was given no ring");
        }
        if seen.pointer_reports != 0 {
            return Err(
                "the compositor took a position on a run in which it was given no input ring, so \
                 a position reaches it by some route other than the one this boot exists to show",
            );
        }
        if seen.latches != 0 || seen.latch_declines != seen.frames {
            return Err("the compositor latched a frame with no ring connected, or declined a \
                 number of frames that is not the number it closed");
        }
        Ok(())
    }
}

/// Print what happened, one subject per line.
pub fn report_lines(report: &Report) {
    let seen = &report.produced;
    let board = &report.consumed.board;
    crate::kprintln!(
        "  input         a fourth driver outside the frame, and the {} half over a {} gesture: {}",
        report.half.name(),
        report.gesture.name(),
        match report.half {
            Half::Deliver =>
                "the compositor holds the other end of the driver's ring and this frame holds \
                 neither",
            Half::Withheld =>
                "the same events on the same ring, and the compositor is not given it - so a \
                 position it latches came from somewhere else",
        }
    );
    crate::kprintln!(
        "  input manif   virtio-input declares {} register page(s) and {} B of untyped for its \
         queue, content {:#018x}",
        report.declared.frames,
        report.declared.bytes,
        report.declared.id.bits(),
    );
    crate::kprintln!(
        "  input device  requester {:#06x}, {} page(s) of register window, core {} at ring 3",
        report.bdf.source_id(),
        report.windows,
        report.cpu,
    );
    crate::kprintln!(
        "  input driver  {} record(s) off the device, {} report(s) closed, {} ignored, {} \
         entr(y/ies) submitted, {} dropped, {} malformed, ended {} after {} idle turn(s)",
        seen.board.records,
        seen.board.reports,
        seen.board.ignored,
        seen.board.submitted,
        seen.board.dropped,
        seen.board.malformed,
        seen.board.outcome,
        seen.board.spun,
    );
    crate::kprintln!(
        "  input clock   {} report(s) timed and the clock stands at {} ns, which is {} ns per \
         report",
        seen.board.stamped,
        seen.board.clock_at,
        STAMP_TICK_NANOS,
    );
    // The ring, on its own line, because what it held between the two
    // components is the number `E3-B04g` is: every entry the driver put there,
    // read off the ring's own cursors rather than asserted by this file.
    crate::kprintln!(
        "  input ring    head {} tail {} when the compositor was stood up, for the {} \
         entr(y/ies) the driver submitted and {} attestation(s); the compositor was {} and took \
         {} entr(y/ies) and {} attestation(s): {} decoded, {} refused, {} of them pointer motion",
        seen.head,
        seen.tail,
        seen.board.submitted,
        seen.board.attested,
        if board.input_connected == 1 { "connected" } else { "not connected" },
        board.input_entries,
        board.input_attestations,
        board.input_decoded,
        board.input_refused,
        board.input_motions,
    );
    // The crossing, on its own line, because it is the only thing in this log
    // that two stages computed separately about one sequence — and since
    // `E3-B04g` it is three numbers, because the driver's word reaches two
    // places by two routes. The counts are printed beside the words so that a
    // reader of a red boot can tell *how many* from *what*.
    crate::kprintln!(
        "  input cross   the driver folded {} entr(y/ies) into {:#018x} (routing page); the \
         compositor folded {} into {:#018x} and read {:#018x} off the ring; {}",
        seen.board.crossed,
        seen.board.crossing,
        board.input_crossed,
        board.input_crossing,
        board.input_attested,
        match (report.half, board.input_agreed == 1) {
            (Half::Deliver, true) => "what arrived is what was sent, readings and bodies alike",
            (Half::Deliver, false) => "THEY DISAGREE",
            (Half::Withheld, _) => "nothing was drained, which is this half's point",
        },
    );
    crate::kprintln!(
        "  input relay   none: the frame kept no event and handed none on; the harness {}",
        if seen.acknowledged { "acknowledged" } else { "NEVER ANSWERED" },
    );
    // The line the harness checks its own injection against. It carries what
    // nothing outside the machine can derive - where the driver's accumulator
    // ended up, as the driver published it - and the harness holds the other
    // half: how far it asked the pointer to move, in whole device pixels.
    //
    // And where the accumulator started, which this frame told the driver: the
    // harness subtracts it, so its equality is about how far the pointer moved
    // and a driver that ignored the start is a difference. The words are read
    // by position, so the first eleven are a format — `xtask`'s
    // `input_pointer` checks the word before each number.
    crate::kprintln!(
        "  input pointer x {} y {} from x {} y {} in units of 1/65536 device pixel, after {} \
         motion event(s)",
        seen.board.pointer_x_x65536,
        seen.board.pointer_y_x65536,
        COMMITTED_TX_X65536,
        COMMITTED_TY_X65536,
        seen.board.motions,
    );
    crate::kprintln!(
        "  input scene   {} delta(s) submitted, {} answered, {} refused; the component applied \
         {} of {} expected, closed {} frame(s), holds {} node(s)",
        report.consumed.submitted,
        report.consumed.completed,
        report.consumed.refused,
        board.edits,
        report.expected_edits(),
        board.frames,
        board.live,
    );
    crate::kprintln!(
        "  input tree    {} node(s) from the compositor's manifest: frames {}, edits {}, nodes \
         {}; outcome {}",
        report.consumed.tree_nodes,
        report.consumed.tree[0],
        report.consumed.tree[1],
        report.consumed.tree[2],
        board.outcome,
    );
    // The late latch, `E3-B01i`, and **the second line the harness reads**.
    // Both ends of the transform and not their difference, for
    // `f_compositor::latch::Latched`'s reason: the process that injected the
    // motion does the subtraction, against the list it injected, and nothing in
    // the machine holds that list. The fields are read by position, so the
    // first twelve words of this line are a format and not prose.
    crate::kprintln!(
        "  input latch   committed x {} y {} latched x {} y {} at trace entry {} of node {}: {} \
         report(s) taken, {} stale, {} frame(s) latched, {} declined, {}",
        signed(board.latch_committed_x),
        signed(board.latch_committed_y),
        signed(board.latch_x),
        signed(board.latch_y),
        board.latch_entry,
        POINTER_NODE,
        board.pointer_reports,
        board.pointer_stale,
        board.latches,
        board.latch_declines,
        if board.latches == 0 {
            "nothing to latch from"
        } else if board.latch_extrapolated == 0 {
            "held at the newest position"
        } else {
            "EXTRAPOLATED"
        },
    );
    // *And by nothing else*, on its own line after the latch line — whose first
    // twelve words are a format — and not inside it.
    crate::kprintln!(
        "  input else    the compositor folded {} node(s) with the pointer's translation masked: \
         {:#018x} before the patch and {:#018x} after; {}",
        board.latch_walked,
        board.latch_unmoved_before,
        board.latch_unmoved_after,
        if board.latches == 0 {
            "no frame was latched"
        } else if board.latch_unmoved_before == board.latch_unmoved_after {
            "nothing else moved"
        } else {
            "SOMETHING ELSE MOVED"
        },
    );
    // The input path's own prediction, `E3-B04e`, and **the third line the
    // harness reads**: the value and the anchor it stands on, both ends and not
    // their difference, for the latch line's reason. The words are read by
    // position, so the first thirteen are a format — `xtask`'s `input_path`
    // checks the word before each number.
    crate::kprintln!(
        "  input path    predicted x {} y {} from anchor x {} y {} for the scanout at {} ns, lead \
         {} ns: {} of {} position(s) taken by the driver's own predictor, {}",
        seen.board.predicted_x_x65536,
        seen.board.predicted_y_x65536,
        seen.board.anchor_x_x65536,
        seen.board.anchor_y_x65536,
        seen.board.predicted_for,
        seen.board.predicted_lead_nanos,
        seen.board.predictor_reports,
        seen.board.motions,
        if seen.board.predicted_for == 0 {
            "NOTHING PREDICTED"
        } else if seen.board.predicted_extrapolated == 1 {
            "extrapolated"
        } else {
            "held at the newest report"
        },
    );
    crate::kprintln!(
        "  input seam    the compositor aimed at {} ns and extrapolated {} ns; the input path was \
         asked about {} ns and extrapolated {} ns; {}",
        board.latch_aim_nanos,
        board.latch_lead_nanos,
        seen.board.predicted_for,
        seen.board.predicted_lead_nanos,
        // What the two sides actually did, and not what the gesture said they
        // would: a steady gesture that held is the case this line has to be
        // able to print, because it is the one the verdict refuses.
        match (
            report.half,
            report.gesture,
            board.latch_extrapolated == 1 && seen.board.predicted_extrapolated == 1,
        ) {
            (Half::Withheld, _, _) => "no latch on this half, so there is no seam to compare",
            (Half::Deliver, Gesture::Turning, false) =>
                "a turning gesture, and a held prediction agrees by copying one sample, so this \
                 is not counted",
            (Half::Deliver, Gesture::Steady, true) =>
                "both extrapolated on a steady gesture, and the verdict compares the two",
            (Half::Deliver, Gesture::Turning, true) =>
                "BOTH EXTRAPOLATED on a gesture the harness says turns",
            (Half::Deliver, Gesture::Steady, false) =>
                "A SIDE HELD on a gesture the harness says is steady",
        },
    );
}

/// The address space a component is built in, and what it is stood up with.
struct Setup<'a> {
    space: &'a AddressSpace,
    features: Features,
    scheduling: Scheduling,
}

/// Run the path once.
///
/// # Errors
///
/// [`Trouble`], every variant of which means the path did not run.
///
/// # Safety
///
/// Call on the boot processor with the kernel's address space in `CR3`, `frames`
/// rebound onto its direct map, `unit` enabled, `scheduling.cpu` a started idle
/// core that is not this one, the direct map covering every boot module, and
/// nothing else in this kernel driving the device this finds.
#[expect(
    clippy::too_many_arguments,
    reason = "every one is a thing the boot path found; bundling them into a struct would be a \
              type that exists so that a lint passes"
)]
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
    unit: &mut Unit,
    window: &pci::Space,
    survey: &Survey,
    boot: &BootInfo,
    half: Half,
    scheduling: Scheduling,
) -> Result<Report, Trouble> {
    // The manifest first, before a frame is spent, because everything below is
    // sized from it - and through the shared half, which is the clause a
    // reviewer of this task checks first.
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every boot module.
    let declared = unsafe { declared(boot, DRIVER) }?;

    // The same finder the other three datapaths use, with `None` for a
    // transitional id because there is no legacy virtio-input to refuse -
    // `virtio::VIRTIO_INPUT_MODERN` is where that is argued.
    // SAFETY: the caller's guarantee, passed down.
    let found = unsafe {
        virtio::route(frames, space, features, window, survey, virtio::VIRTIO_INPUT_MODERN, None)
    }
    .map_err(Trouble::Device)?;

    // The device has to fit what the manifest declares, and the refusal is in
    // that direction on purpose: a device whose window is larger is a different
    // device and a different manifest, not a bigger number.
    if found.pages > declared.frames {
        return Err(Trouble::Manifest);
    }

    let before = frames.free_count();
    let kept_tables = unit.tables().len();
    // A domain of the component's own, before anything is allocated for it: a
    // driver with no domain is a driver whose device addresses physical memory,
    // and what this one writes into memory is every keystroke and every motion.
    // SAFETY: the caller's guarantee that frames are addressable.
    let mut domain = unsafe { unit.domain(frames) }.map_err(Trouble::Unit)?;

    let region_order = order_for(declared.bytes).ok_or(Trouble::Manifest)?;
    let granted = frames.alloc_zeroed(region_order).ok_or(Trouble::NoFrames)?;
    // The driver's data channel. **It outlives the driver**, and that is the
    // arrangement `E3-B04g` rests on: the frame allocates it, the driver holds
    // one end while it runs, and the compositor holds the other after the
    // driver is gone — so this function frees it, after both, and neither
    // component's reap can.
    let wire = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;

    // SAFETY: two frames just allocated, each held by nobody else, and the
    // caller's guarantees passed down.
    let outcome = unsafe {
        produce(
            frames,
            unit,
            &mut domain,
            &found,
            declared,
            granted,
            wire,
            Setup { space, features, scheduling },
        )
    };

    // Whatever happened, the device stops being able to address memory before
    // its domain is freed - and this driver does put its device back in reset on
    // the way out, which the display driver deliberately does not: what a
    // running virtio-input device does is write into these buffers whenever the
    // user acts, and the frame is about to take the memory back.
    // SAFETY: `found.config` is the function's configuration space.
    unsafe { pci::command_clear(found.config, pci::COMMAND_BUS_MASTER) };
    // SAFETY: the caller's guarantee, and `bdf` is the function `produce`
    // attached.
    let _ = unsafe { unit.detach(frames, found.bdf) };
    // SAFETY: nothing is attached and no device is walking these tables.
    unsafe { unit.release(frames, domain) };

    // SAFETY: allocated above, at the order it is freed at, and the device is
    // detached and stripped of bus mastering. The data channel is *not* freed
    // here: it is the compositor's input ring next.
    unsafe { frames.free(granted) };

    let produced = match outcome {
        Ok(produced) => produced,
        Err(why) => {
            // SAFETY: allocated above; the driver that held one end of it has
            // been reaped or never ran, and no compositor has been given it.
            unsafe { frames.free(wire) };
            return Err(why);
        }
    };

    // And now the second component, on the core the first one has given back,
    // holding the far end of the ring the first one wrote into — or, on the
    // withholding half, not holding it.
    // SAFETY: the caller's guarantee about the boot processor, the allocator,
    // the direct map and the worker core, passed down; the driver has been
    // reaped, so that core is idle again, and `wire` is a frame this function
    // allocated whose only other holder is gone.
    let consumed = unsafe { consume(frames, space, features, half, boot, scheduling, wire) }?;

    // SAFETY: allocated above; both components that held an end of it have been
    // joined and reaped - `consume` answering `Ok` is what says the second one
    // was - and this frame holds no pointer into it. On an error it is not
    // freed, for the reason `consume` does not free its own scene ring on one:
    // a component whose join did not return may still be inside the page, and
    // the boot is failing anyway.
    unsafe { frames.free(wire) };

    // Everything both stages took, back where it started, with the unit's own
    // retained tables taken out. Two numbers rather than a tolerance: a check
    // with slack in it is a check that stops noticing the first frame.
    let retained = unit.tables().len().saturating_sub(kept_tables) as u64;
    if frames.free_count().saturating_add(retained) != before {
        return Err(Trouble::Leaked);
    }

    Ok(Report {
        half,
        gesture: Gesture::of(boot),
        declared,
        bdf: found.bdf,
        windows: found.pages,
        cpu: scheduling.cpu,
        produced,
        consumed,
    })
}

/// The driver stage: stand `user/virtio-input` up, hold still while somebody
/// moves the pointer, and **take nothing off its ring**.
///
/// # Safety
///
/// As [`demonstrate`], and every frame must be one the caller allocated for this.
#[expect(
    clippy::too_many_arguments,
    reason = "every one is a thing the caller allocated or found; bundling them into a struct \
              would be a type that exists so that a lint passes"
)]
unsafe fn produce(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    domain: &mut crate::arch::x86_64::vtd::Domain,
    found: &virtio::Found,
    declared: Declared,
    granted: Frame,
    wire: Frame,
    setup: Setup<'_>,
) -> Result<Produced, Trouble> {
    // The driver's own table, holding the region its manifest declares. There is
    // no client table on this path and the absence is the difference: the three
    // datapaths before this one resolve a *client's* handle when a driver asks
    // for a translation, and this driver asks for nothing because it holds
    // nobody's buffer.
    let mut driver_table = Table::EMPTY;
    let region_cap = driver_table
        .grant(CapType::Frame, GRANTABLE, granted.addr(), granted.bytes())
        .map(|handle| handle.bits())
        .map_err(|_| Trouble::Authority)?;

    let region_len = u32::try_from(granted.bytes()).map_err(|_| Trouble::Authority)?;
    let region_at = {
        let mut asking = iommu::Grant {
            unit: &mut *unit,
            domain: &mut *domain,
            frames: &mut *frames,
            table: &driver_table,
        };
        asking.map(region_cap, region_len).map_err(|_| Trouble::Refused)?
    };

    // The device joins the domain last, so that a refusal above leaves nothing
    // attached, and before bus mastering, so it cannot issue a transaction until
    // there is a domain to translate it.
    // SAFETY: the caller's guarantee, and `bdf` is the function whose registers
    // are about to be driven.
    unsafe { unit.attach(frames, found.bdf, domain) }.map_err(Trouble::Unit)?;
    // SAFETY: `found.config` is the function's mapped configuration space.
    unsafe { pci::command_set(found.config, pci::COMMAND_BUS_MASTER) };

    // The register window as the component will see it - the shared half again,
    // and the second of the four things this file does not write a fourth copy
    // of.
    let registers = Registers::of(found, &declared)?;
    let plan = DriverPlan {
        image: declared.image,
        selector: routing::life::SERVE,
        tree: setup.scheduling.tree,
        hz: setup.scheduling.hz,
        target: setup.scheduling.target,
        cpu: setup.scheduling.cpu,
        registers: registers.base,
        queues: granted.addr(),
        queue_bytes: granted.bytes(),
        data: wire.addr(),
    };
    // SAFETY: the caller's guarantee, passed down; `registers.base` is the first
    // page of a device window this boot mapped and nothing else is driving,
    // `granted` and `wire` are frames the caller allocated and holds, and `cpu`
    // is a core that is up and idle.
    let (prepared, pages) =
        unsafe { process::prepare_driver(frames, setup.space, setup.features, plan) }
            .map_err(Trouble::Process)?;

    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Authority)?;
    let at = frames.virt(wire);
    // The data ring, laid out and **held by nobody on this side**. The component
    // adopts the client's end; the server's end is the compositor's, later, in
    // a different address space — `consume` maps this same frame into it. The
    // value `describe` answers is dropped on purpose: a `Mapping` kept here would
    // be a consumer this file could call, and `E3-B04g`'s exit is that no such
    // call exists.
    // SAFETY: `wire` was allocated zeroed by the caller, is frame-aligned and is
    // `FRAME_SIZE` bytes with no pointer into it held anywhere else.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: `pages.control` is the kernel address of a frame `prepare_driver`
    // allocated zeroed for this run and handed to nobody else.
    let control = unsafe {
        Mapping::describe(
            pages.control as *mut u8,
            bytes,
            ENTRIES,
            0,
            feature::CONTROL_EVENTS,
            feature::CONTROL_EVENTS,
        )
    }
    .map_err(Trouble::Channel)?;

    // --- what the component is told -----------------------------------------
    let board = Window::at(pages.board, routing::BYTES).map_err(Trouble::Channel)?;
    for (offset, value) in [
        (routing::at::REGISTERS_AT, crate::process::BLK_REGISTERS),
        (routing::at::REGISTERS_LEN, u64::from(registers.pages) * FRAME_SIZE),
        (routing::at::NOTIFY_MULTIPLIER, u64::from(found.notify_multiplier)),
        (routing::at::QUEUES_AT, crate::process::BLK_QUEUES),
        (routing::at::QUEUES_DEVICE_AT, region_at),
        (routing::at::QUEUES_LEN, granted.bytes()),
        (routing::at::CONTROL_AT, crate::process::SPAWN_CONTROL),
        (routing::at::CONTROL_LEN, u64::from(bytes)),
        (routing::at::DATA_AT, crate::process::BLK_DATA),
        (routing::at::DATA_LEN, u64::from(bytes)),
        (routing::at::NEGOTIATED_VERSION, u64::from(ABI_VERSION)),
        (routing::at::NEGOTIATED_FEATURES, 0),
        // The ceiling this component was admitted for, out of its own compiled
        // record and never restated here. RFC 0025 bound 2 from the submitting
        // end: every entry it puts on the data ring carries this class and there
        // is no field in that crate that could raise it.
        (routing::at::ADMITTED, u64::from(declared.admitted)),
        (routing::at::IDLE_SPINS, IDLE_SPINS),
        // The two words that are this driver's and no other's: the clock it has
        // none of. `user/virtio-input/src/clock.rs` is where what these numbers
        // are - and what they are not - is written down.
        (routing::at::STAMP_SEED, STAMP_SEED),
        (routing::at::STAMP_TICK_NANOS, STAMP_TICK_NANOS),
        // Where the pointer starts, which is where `commit` puts it: one number,
        // told to the driver and committed by the client, so the client has
        // still never been told where the pointer *is*. RFC 0132.
        (routing::at::ORIGIN_X_X65536, COMMITTED_TX_X65536.cast_unsigned()),
        (routing::at::ORIGIN_Y_X65536, COMMITTED_TY_X65536.cast_unsigned()),
        // The scanout the driver's own predictor is asked about, `E3-B04e`.
        // A target this frame states about the display it declares, and never
        // a reading; `FIRST_SCANOUT_NANOS` is the argument. RFC 0134.
        (routing::at::SCANOUT_AT_NANOS, FIRST_SCANOUT_NANOS),
    ] {
        board.write64(offset, value).map_err(Trouble::Channel)?;
    }
    for (slots, structure) in [
        (routing::at::COMMON_OFFSET, routing::at::COMMON_LEN),
        (routing::at::NOTIFY_OFFSET, routing::at::NOTIFY_LEN),
        (routing::at::ISR_OFFSET, routing::at::ISR_LEN),
        (routing::at::CONFIG_OFFSET, routing::at::CONFIG_LEN),
    ]
    .into_iter()
    .zip(registers.each)
    {
        board.write64(slots.0, u64::from(structure.0)).map_err(Trouble::Channel)?;
        board.write64(slots.1, u64::from(structure.1)).map_err(Trouble::Channel)?;
    }
    // Last, so that a component reading a page this loop did not finish finds a
    // zero rather than a plausible layout.
    board.write64(routing::at::MAGIC, routing::MAGIC).map_err(Trouble::Channel)?;

    // --- the driver runs -----------------------------------------------------
    // SAFETY: `cpu` reports ready, everything `process::execute` depends on was
    // put in its shards by `prepare_driver`, and this core has interrupts
    // enabled.
    unsafe { crate::smp::start_on(setup.scheduling.cpu) }.map_err(Trouble::Scheduled)?;

    let asks = Consumer::new(control.channel()).ok_or(Trouble::Channel(0))?;
    let answers = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;

    let mut supervising = Supervising {
        asks: &asks,
        answers: &answers,
        // None, and the field's own comment is the argument: this driver
        // produces rather than answers, so there is no client end for the frame
        // to reap and a collector here would be a second reader of a completion
        // ring nobody posts on.
        reaper: None,
        unit: &mut *unit,
        domain: &mut *domain,
        frames: &mut *frames,
        // The driver's own table. On the three datapaths before this one the
        // table a translation resolves against is the *client's*, which is what
        // makes a driver unable to grant itself anything; there is no client
        // here and this driver asks for nothing, so what the shared half holds
        // is a table with one handle in it that nobody will name.
        table: &driver_table,
        answered_at: 0,
        answered: 0,
    };

    let tsc_khz = setup.scheduling.tsc_khz;
    let mut acknowledged = false;

    // **The line the harness waits for.** It is printed after the component is
    // running, because the events have to be injected into a machine whose
    // driver is already serving: an event delivered to a device nobody has
    // started is an event the emulator queues and the driver finds later or not
    // at all. `kernel/src/gpu.rs`'s marker is the other way round - it is
    // printed after its verdict, because a picture survives the boot and an
    // input event does not.
    crate::kprintln!(
        "  input inject  the driver is serving on core {}; move the pointer now",
        setup.scheduling.cpu,
    );

    let waiting_until = crate::smp::deadline_after(tsc_khz, INJECT_MICROS);
    let mut settle_until = None;
    loop {
        // The shared half's polling point, on the driver's control ring. It
        // finds nothing on this path - the driver asks for nothing - and it is
        // here rather than skipped because a supervisor that stopped serving
        // while its component ran would be a supervisor this boot cannot say
        // anything about.
        //
        // **And that is the whole of what this loop does with the component.**
        // Until `E3-B04g` a drain of the data ring stood beside this call, and
        // the ring then emptied as the driver filled it. It does not now: the
        // entries stay on the ring until the compositor takes them, which is why
        // the ring's size is the bound on a gesture and why the verdict requires
        // the driver's `dropped` to be zero.
        supervising.serve()?;

        if settle_until.is_none() && crate::arch::x86_64::serial::Serial.received().is_some() {
            acknowledged = true;
            settle_until = Some(crate::smp::deadline_after(tsc_khz, SETTLE_MICROS));
        }
        match settle_until {
            Some(deadline) if crate::smp::past(deadline) => break,
            None if crate::smp::past(waiting_until) => break,
            _ => core::hint::spin_loop(),
        }
    }

    // Told to stop whatever happened above, because a driver left serving a
    // frame that has gone is a core this boot never gets back. The driver puts
    // its fold on the data ring on its way out — after this notice and before
    // its `EXIT` — which is why the stop comes before the join and not after.
    let told = supervising.stop();
    // SAFETY: `start_on` was called for this core and nothing else has joined
    // it. The closure serves the driver's control ring, whose two ends are
    // single-producer and single-consumer by construction.
    let joined = unsafe {
        crate::smp::join_serviced(setup.scheduling.cpu, tsc_khz, EXIT_MICROS, &mut || {
            let _ = supervising.serve();
        })
    };
    let asked = supervising.answered;

    // Everything the component wrote into the half of its routing page that is
    // its own, read before the address space goes back to the allocator. RFC
    // 0013's *read, never delivered*: it was never asked.
    let reported = Reported::of(&board);

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished - which is what `join_serviced` returning `Ok` means.
    let ended = unsafe { process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    let exited = matches!(ended.death, crate::process::Death::Exited(_));

    told?;
    joined.map_err(|why| match why {
        crate::smp::NotJoined::Refused(cpu) => Trouble::Scheduled(cpu),
        crate::smp::NotJoined::Overdue(_) => Trouble::Overdue(EXIT_MICROS),
    })?;

    let board = reported.ok_or(Trouble::BadReport)?;

    // Where the ring's two cursors stand now, with the driver joined and reaped
    // and the compositor not yet stood up: the one moment neither end is
    // running. Both are read and nothing is taken — no `Producer` and no
    // `Consumer` is bound, because neither has a call that answers a cursor
    // without also being a handle that could move one.
    //
    // The raw reads are the diagnostic `Cursor::raw` exists for, and `Relaxed`
    // is enough here for the reason `Producer::occupancy` gave when it stood
    // here: the driver that owned the head is joined, the join is the edge
    // that makes its last head store visible to this core, and nothing else
    // holds either end until `consume` hands the far one out. The cursors and
    // not their difference — `Produced::head`'s doc is the relay a difference
    // cannot see.
    // SAFETY: `wire` is the frame the channel was laid out in above, allocated
    // by the caller and held by nothing that runs: the driver that held one end
    // is reaped. Every accessor hands out atomics rather than references.
    let ring = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;
    let cursors = ring.channel();
    let (head, tail) = (u64::from(cursors.head.raw()), u64::from(cursors.tail.raw()));
    Ok(Produced { board, exited, asked, acknowledged, head, tail })
}

/// The compositor stage: stand `user/compositor` up on the core the driver has
/// given back, with the driver's ring as its input channel on the delivering
/// half, and commit one frame.
///
/// # Safety
///
/// As [`demonstrate`], after the driver has been reaped, with `input` the frame
/// the driver's data channel was laid out in and nothing else holding it.
unsafe fn consume(
    frames: &mut FrameAllocator,
    kernel: &AddressSpace,
    features: Features,
    half: Half,
    boot: &BootInfo,
    on: Scheduling,
    input: Frame,
) -> Result<Consumed, Trouble> {
    // The same finder `kernel/src/compositor.rs` uses, and not a second one: a
    // boot with two answers to *which module is the compositor* is a boot that
    // can stand two different components up and call them one.
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every boot module.
    let (image, record) = unsafe { crate::compositor::found(boot) }.ok_or(Trouble::NoCompositor)?;

    // The manifest and the crate are required to be one number, checked here for
    // the reason `kernel/src/compositor.rs` gives: the graph's size is a
    // compile-time fact in `f-compositor` and the heap is a field in a manifest,
    // nothing links the two, and a manifest edited downwards would be a
    // component whose first allocation fails at ring 3 with nothing naming the
    // cause.
    let declared_heap = crate::compositor::heap_declared(&record).ok_or(Trouble::NoHeap)?;
    if declared_heap != scene_routing::HEAP_BYTES {
        return Err(Trouble::HeapDisagrees);
    }
    // The cap, `E3-B07e`, by the same finder `kernel/src/compositor.rs` uses:
    // the component refuses a routing page without it, so this boot hands it
    // the word its own manifest declares rather than one written here.
    let cap = crate::compositor::frame_cap(&record).ok_or(Trouble::Unframed)?;

    // **The one line the two halves differ in.** The delivering half maps the
    // driver's data channel into the compositor and says where; the withholding
    // half maps nothing and writes zero, which the component reads as *not
    // connected*. Everything else below — the scene, the commit, the stop — is
    // the same on both, so a latch on one and not the other is the ring's doing.
    //
    // The ring rides `ServerPlan::buffers`, the one caller-held region that
    // shape maps writable into a server at `process::BLK_QUEUES`. It is named
    // for a client's buffer region and this component declares `payload =
    // "inline"`, so it has no such region and the address is otherwise unused —
    // `process::prepare_server`'s own comment says the reuse of that address
    // across meanings is deliberate. *What would reverse this:* a shape that
    // maps a second channel by name, which is `E1-B05`'s supervisor handing a
    // place's occupant its peers; on that day this is a ring in a manifest and
    // not a region in a plan.
    let (buffers, buffer_bytes, input_at, input_len) = if half.connects() {
        (input.addr(), FRAME_SIZE, crate::process::BLK_QUEUES, FRAME_SIZE)
    } else {
        (0, 0, 0, 0)
    };

    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0))?;
    let wire = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;
    let at = frames.virt(wire);
    // The scene ring, and here the frame is the **client** and the component is
    // the server — the frame's one role left on this path, and it is the role a
    // client application has, which is what the frame is standing in for.
    // SAFETY: `wire` was allocated zeroed just above, is frame-aligned and is
    // `FRAME_SIZE` bytes with no pointer into it held anywhere else.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and every
    // accessor hands out atomics and `UnsafeCell`s rather than references.
    let client_end = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;

    // SAFETY: the caller's guarantee about `kernel`, `frames` and `cpu`, plus
    // `wire` being a frame this function allocated and holds the far end of,
    // and `input` — when it is passed — being the caller's frame with nobody
    // else holding it.
    let (prepared, pages) = unsafe {
        process::prepare_server(
            frames,
            kernel,
            features,
            ServerPlan {
                image,
                selector: scene_routing::life::SERVE,
                tree: on.tree,
                hz: on.hz,
                target: on.target,
                cpu: on.cpu,
                data: wire.addr(),
                buffers,
                buffer_bytes,
                heap_bytes: scene_routing::HEAP_BYTES,
                own_tree: true,
            },
        )
    }
    .map_err(Trouble::Process)?;

    let tree_nodes = crate::component::publish_tree(pages.own_tree as *mut u8, &record)
        .map_err(|_| Trouble::StateTree(0))?;

    // SAFETY: `pages.control` is the kernel address of a frame `prepare_server`
    // allocated zeroed for this run and handed to nobody else.
    let control = unsafe {
        Mapping::describe(
            pages.control as *mut u8,
            bytes,
            ENTRIES,
            0,
            feature::CONTROL_EVENTS,
            feature::CONTROL_EVENTS,
        )
    }
    .map_err(Trouble::Channel)?;

    let board = Window::at(pages.board, scene_routing::BYTES).map_err(Trouble::Channel)?;
    for (offset, value) in [
        (scene_routing::at::CONTROL_AT, crate::process::SPAWN_CONTROL),
        (scene_routing::at::CONTROL_LEN, u64::from(bytes)),
        (scene_routing::at::DATA_AT, crate::process::BLK_DATA),
        (scene_routing::at::DATA_LEN, u64::from(bytes)),
        (scene_routing::at::TREE_AT, crate::process::SPAWN_TREE),
        (scene_routing::at::NEGOTIATED_VERSION, u64::from(ABI_VERSION)),
        (scene_routing::at::NEGOTIATED_FEATURES, 0),
        (scene_routing::at::IDLE_SPINS, IDLE_SPINS),
        (scene_routing::at::TICK_NANOS, 0),
        (scene_routing::at::SCANOUT_PERIOD_NANOS, SCANOUT_PERIOD_NANOS),
        (scene_routing::at::PACING_MARGIN_NANOS, PACING_MARGIN_NANOS),
        (scene_routing::at::BACKEND_CAPABILITIES, BACKEND_CAPABILITIES),
        // Which node the pointer rides, `E3-B01i`. Told on **both** halves, so
        // that the difference between them stays the one ring: a control whose
        // compositor had been told to latch nothing would be a control for two
        // things at once, and the withholding half's zero latches is then a fact
        // about the ring rather than about this word.
        (scene_routing::at::POINTER_NODE, u64::from(POINTER_NODE)),
        (scene_routing::at::INPUT_AT, input_at),
        (scene_routing::at::INPUT_LEN, input_len),
        (scene_routing::at::DELTAS_PER_FRAME_MAX, cap),
    ] {
        board.write64(offset, value).map_err(Trouble::Channel)?;
    }
    board.write64(scene_routing::at::MAGIC, scene_routing::MAGIC).map_err(Trouble::Channel)?;

    // SAFETY: the caller vouched `cpu` is started and idle - the driver that had
    // it was joined and reaped - and `prepare_server` has written its job.
    unsafe { crate::smp::start_on(on.cpu) }.map_err(Trouble::Scheduled)?;

    let reaper = Collector::new(client_end.completions()).ok_or(Trouble::Channel(0))?;
    let producer = Producer::new(client_end.channel()).ok_or(Trouble::Channel(0))?;
    let notices = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;
    let arena = client_end.arena();

    let mut env = SeededEnv::new(PACING_SEED, 0);
    let driven = commit(&producer, &reaper, &arena, &board, &mut env, on.tsc_khz);

    let told = notices.post(control::entry(control::notice::STOP, 0, 0, 0));
    // SAFETY: `start_on` was called for this core and nothing else has joined
    // it. The closure serves nothing: a compositor reaches no device, so there
    // is nothing it can ask the frame for while it runs.
    let joined = unsafe { crate::smp::join_serviced(on.cpu, on.tsc_khz, EXIT_MICROS, &mut || {}) };

    let reported = Board::of(&board);
    // The tree, read before `reap` gives the page back.
    let reader =
        state::Reader::at(pages.own_tree, FRAME_SIZE as u32).map_err(Trouble::StateTree)?;
    let mut tree = [0u64; 3];
    for (slot, id) in tree.iter_mut().zip(WATCHED) {
        // `u64::MAX` for an id the schema does not carry, so a manifest and
        // `f_compositor::routing::node` that disagree fail the comparison rather
        // than passing it with a zero nobody wrote.
        *slot = reader.value(id).unwrap_or(u64::MAX);
    }

    if told.is_err() {
        return Err(Trouble::Channel(0));
    }
    // Checked before anything is torn down, and that is the order rather than a
    // preference: a join that did not return is a core still inside this
    // component, and `reap` would give its address space back while an
    // instruction pointer was in it.
    joined.map_err(|_| Trouble::Overdue(EXIT_MICROS))?;

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished.
    let _ended = unsafe { process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    // SAFETY: allocated by this function, the component that was lent it has
    // exited - the join is what says so - and nothing else holds a pointer into
    // it.
    unsafe { frames.free(wire) };

    let seen = driven?;
    let board = reported.ok_or(Trouble::BadReport)?;

    Ok(Consumed {
        submitted: seen.submitted,
        completed: seen.completed,
        refused: seen.refused,
        board,
        tree_nodes,
        tree,
    })
}

/// What the client itself saw on the scene ring.
struct Sent {
    submitted: u64,
    completed: u64,
    refused: u64,
}

/// Submit the three setup deltas, the two committed transforms, and one
/// commit — the same six on both halves.
///
/// # Why the committed transform is where the accumulator starts
///
/// Because this client has never been told where the pointer is, and that is
/// the point of `E3-B04g`: the position goes from the driver to the compositor
/// and not through here. The one place a client that knows nothing can honestly
/// put the pointer is where the driver's accumulator starts — which this frame
/// told the driver, on its routing page, before it ran. `E3-B01i`'s *by exactly
/// the motion injected* is then a subtraction the harness can do against the
/// list it injected: latched minus committed is the accumulator's end minus its
/// start, and that is the sum of the motion. Why the start is not zero is
/// [`COMMITTED_TX_X65536`]'s.
///
/// # Why one at a time
///
/// Because the payload travels in the channel's arena and this client writes
/// every payload at the same offset, so a second entry submitted before the
/// first was answered would overwrite bytes the component had not read yet.
/// `kernel/src/compositor.rs` makes the same choice for the same reason.
///
/// # Errors
///
/// [`Trouble::NotCommitted`] where the ring will not take a delta, or where a
/// completion does not arrive inside the bound.
fn commit(
    producer: &Producer<'_>,
    reaper: &Collector<'_>,
    arena: &Arena<'_>,
    board: &Window,
    env: &mut SeededEnv,
    tsc_khz: u64,
) -> Result<Sent, Trouble> {
    let mut seen = Sent { submitted: 0, completed: 0, refused: 0 };
    let mut token: u64 = 0;

    let mut send = |seen: &mut Sent, body: SceneEntry, slack_nanos: u64| -> Result<(), Trouble> {
        // The clock reading, written before the entry it belongs to. The
        // ordering argument is `kernel/src/compositor.rs`'s unchanged: the
        // reading goes into the page, then the entry goes on the ring with a
        // `Release` publish and the component takes it with an `Acquire`, so the
        // reading is visible before the entry that was written after it.
        let step_nanos = 1 + env.next_u64() % STEP_SPREAD_NANOS;
        env.advance(step_nanos);
        let now = env.now().as_nanos();
        board.write64(scene_routing::at::TICK_NANOS, now).map_err(|_| Trouble::NotCommitted)?;

        token = token.saturating_add(1);
        let delta = Delta {
            user_data: token,
            class: class::BATCH,
            deadline: if slack_nanos == 0 { 0 } else { now.saturating_add(slack_nanos) },
            payload_offset: 0,
            flags: 0,
            body,
        };
        let (entry, payload) = delta.encode();
        if !arena.copy_in(0, &payload) {
            return Err(Trouble::NotCommitted);
        }
        if producer.submit(entry).is_err() {
            return Err(Trouble::NotCommitted);
        }
        seen.submitted = seen.submitted.saturating_add(1);

        let deadline = crate::smp::deadline_after(tsc_khz, EXIT_MICROS);
        loop {
            match reaper.take() {
                Ok(Some(answer)) => {
                    seen.completed = seen.completed.saturating_add(1);
                    if answered_badly(&answer, entry.user_data) {
                        seen.refused = seen.refused.saturating_add(1);
                    }
                    return Ok(());
                }
                Ok(None) => {
                    if crate::smp::past(deadline) {
                        return Err(Trouble::NotCommitted);
                    }
                    core::hint::spin_loop();
                }
                Err(_) => return Err(Trouble::NotCommitted),
            }
        }
    };

    // The surface, and the node that will move on it.
    send(
        &mut seen,
        SceneEntry::CreateNode(CreateNode {
            node: ROOT_NODE,
            parent: NO_NODE,
            before: NO_NODE,
            kind: kind::LAYER,
        }),
        0,
    )?;
    send(
        &mut seen,
        SceneEntry::CreateNode(CreateNode {
            node: POINTER_NODE,
            parent: ROOT_NODE,
            before: NO_NODE,
            kind: kind::TRANSFORM,
        }),
        0,
    )?;
    // And the sibling that never moves, after the pointer in paint order.
    send(
        &mut seen,
        SceneEntry::CreateNode(CreateNode {
            node: STILL_NODE,
            parent: ROOT_NODE,
            before: NO_NODE,
            kind: kind::TRANSFORM,
        }),
        0,
    )?;
    // The transform the client commits for the pointer, which is the one the
    // latch reads back out of the graph and patches: a shear at the driver's
    // start — see this function's doc for why there, `COMMITTED_TX_X65536` for
    // why that is not zero, and `COMMITTED_A_X65536` for why a shear.
    send(
        &mut seen,
        SceneEntry::SetTransform(SetTransform {
            node: POINTER_NODE,
            a_x65536: COMMITTED_A_X65536,
            b_x65536: COMMITTED_B_X65536,
            c_x65536: COMMITTED_C_X65536,
            d_x65536: COMMITTED_D_X65536,
            tx_x65536: COMMITTED_TX_X65536,
            ty_x65536: COMMITTED_TY_X65536,
        }),
        0,
    )?;
    // The sibling's, which nothing may move.
    send(
        &mut seen,
        SceneEntry::SetTransform(SetTransform {
            node: STILL_NODE,
            a_x65536: COMMITTED_A_X65536,
            b_x65536: COMMITTED_B_X65536,
            c_x65536: COMMITTED_C_X65536,
            d_x65536: COMMITTED_D_X65536,
            tx_x65536: STILL_TX_X65536,
            ty_x65536: STILL_TY_X65536,
        }),
        0,
    )?;

    send(&mut seen, SceneEntry::Commit(Commit { frame_token: FRAME_ONE }), COMMIT_SLACK_NANOS)?;
    Ok(seen)
}

/// Is this completion anything other than *the delta it names was accepted*?
///
/// Two things at once on purpose: a refusal, and an answer to a different entry.
/// A client that checked only the result would count a completion for delta
/// three as the answer to delta four and never notice.
fn answered_badly(answer: &Cqe, expected: u64) -> bool {
    answer.result < 0 || answer.user_data != expected
}
