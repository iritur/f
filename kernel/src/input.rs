// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The input path, end to end: a fourth driver outside the frame, a real
//! pointing device, an event a person caused, and a compositor that moves a node
//! because of it.
//!
//! # What this file is, and what it finishes
//!
//! It is the **frame's half** of `E3-B04d`. `user/virtio-input` is the driver:
//! the transport handshake, one virtqueue, the evdev accumulator, the stamp and
//! the submission loop, in a crate that forbids `unsafe`. That crate was built
//! and never run — nothing stood it up and nothing drained it — so every
//! property it states was a property of source rather than of a machine. What is
//! here is everything a supervisor does around one, and then the thing the exit
//! actually asks for: the events go somewhere.
//!
//! Two lines close on it.
//!
//! `E3-B04d`'s exit is *a driver component delivers events a compositor
//! consumes, using `kernel/src/supervisor.rs`'s shared half rather than a fourth
//! copy of it.* The shared half is [`declared`], [`Registers`], [`order_for`]
//! and [`Supervising`], and this file writes none of them again — the one thing
//! it had to widen is `Supervising::reaper`, which is now an `Option` because
//! this is the first supervisor in the tree whose driver **produces rather than
//! answers** and therefore holds no client end to reap. That field's own comment
//! is the argument.
//!
//! `E3-B04a`'s exit is *one time source in the whole input path*, and RFC 0099
//! narrowed it because it was true of a path carrying nothing: `at_interrupt`
//! had no caller outside its own tests. RFC 0103 then made `cargo xtask
//! lint-stamp` count the call as well as the reading, which made the source
//! true. Both entries say the same thing about what was still owed — *true in
//! the source and not yet true in a boot*. This is the boot: the driver opens a
//! report, takes one reading for it, and every entry the frame drains carries
//! one, because `f_abi::input::Event::decode` refuses an unstamped payload and
//! this frame decodes every entry it takes.
//!
//! # Why the frame is not on `lint-stamp`'s list, and why that is true rather
//! than convenient
//!
//! `INPUT_PATH` in `xtask` names the crates on this path and forbids every one
//! of them a clock reading. `kernel/` is not on it, and the reason is that **the
//! frame on this path is a courier and not a stage**: it writes the clock the
//! driver is told to take its readings against — a seed and a tick, onto a
//! routing page — and then never asks what time it is on an event's behalf. It
//! does not mint a reading, does not take a second one, does not compare one
//! against a reading of its own, and does not name the field a reading crosses
//! in. What it does with an arriving entry is decode it, which is the consumer's
//! half of RFC 0099's sentence and takes no clock at all.
//!
//! That claim has a check behind it in both directions. `lint-stamp` fails this
//! crate the moment its source names that vocabulary, because `ON_THE_PATH` is a
//! text question rather than a dependency-graph one; and it fails
//! `user/virtio-input` the moment the one call goes away. A frame that started
//! deciding when an event happened would have to add `kernel/` to that list in a
//! diff somebody reads, and would then have to explain the timestamp counter
//! `kernel/src/smp.rs` reads to bound a spin.
//!
//! # The shape of the run, and the two costs it carries
//!
//! One worker core, so **the two components run one after the other and the
//! frame holds the events in between**. The driver is stood up, the harness
//! moves the pointer, the frame drains what the driver submits and keeps it; the
//! driver is stopped and reaped; the compositor is stood up on the same core and
//! the frame submits one `SetTransform` per event it kept, then one commit.
//!
//! Both halves of that are worth saying plainly rather than leaving for a reader
//! to discover.
//!
//! **The frame is the courier.** `E1-B05`'s ring-3 supervisor does not hand a
//! place's occupant a core *and* a peer, so the only thing in this boot that can
//! hold the far end of either channel is the frame — the arrangement every
//! datapath boot in this tree has, under the same reversal, and `CHAOS_GAP` in
//! `xtask` carries what is owed. What the frame adds here that it adds on none
//! of the others is a *translation*: an input event and a scene delta are
//! different vocabularies and something has to be the router between them. That
//! router is `E3-B04`'s parent's business and what is here is the smallest
//! honest version of it — one delta per event, injective in the event, so that a
//! compositor which dropped or coalesced one says so in its own count.
//!
//! **The events are buffered.** A machine with two worker cores would run the
//! driver and the compositor at once and relay as it drained; this one has one,
//! `-smp 2` is pinned because the core count is part of every boot log in this
//! tree, and a verb that quietly asked for a third would be demonstrating a
//! different machine. So the relay is a buffer of at most [`EVENTS_MAX`]
//! entries, and a run that fills it **fails** rather than truncating: an input
//! path that silently forgets the end of a gesture is the defect this subsystem
//! exists not to have, and a buffer that dropped quietly would make the
//! compositor's count agree with a number the frame had already discarded.
//!
//! # Two halves, and the control is the frame's own decision
//!
//! `input=deliver` is the path. The device produces, the driver translates and
//! submits, the frame drains and hands on, the compositor applies.
//!
//! `input=withheld` is **the identical run with the hand-on removed**. The same
//! device is driven, the same events are injected, the driver submits the same
//! entries, and the frame drains and decodes every one of them — and then relays
//! none. The compositor is stood up, is given the same two setup deltas and the
//! same commit, and must apply exactly those and no more.
//!
//! The control had to be something the *frame* decides, and that is why it is
//! this one rather than an unstamped entry: an entry with no reading on it is
//! already refused by `abi/src/input.rs` before it reaches anybody, so a boot
//! built around it would be re-running a decoder's test with an emulator
//! attached. What is genuinely undecided until this file decides it is whether
//! the deltas a compositor applies came off a device at all, and the only way to
//! show that is a run in which the events existed, were counted, and did not
//! arrive. It is `gpu=blank`'s shape and it is sharper than injecting nothing,
//! for `gpu=blank`'s reason: the events are real for the whole of that boot, and
//! a compositor that applied one would have got it by some route other than this
//! relay.
//!
//! # What arrived is what was sent, and how a frame says so without holding a
//! reading
//!
//! Every count in this file is a tally: how many records came off the device,
//! how many reports closed, how many entries were submitted and drained and
//! decoded. Not one of them says the entries that *arrived* are the entries
//! that were *sent*. A relay that minted a reading on arrival, handed two on
//! out of order, dropped one out of the middle or moved a coordinate by one
//! satisfies all of them — and the first of those four is the defect
//! `input/src/stamp.rs` is written against, which is not a wrong number but a
//! right-looking one that improves as the system gets slower.
//!
//! So `E3-B04f` adds a second kind of observation. The driver folds every entry
//! it puts on the ring into `f_abi::input::Crossing` and publishes the word on
//! its routing page; this frame folds every entry it drains into one of its
//! own; and `Report::driver_verdict` requires the two to agree. Neither side
//! holds the other's copy and neither word travels with the entries. What is
//! shared is the implementation, which is `E3-B04c`'s distinction: a defect
//! inside the fold moves both words the same way and is `abi`'s own corpus to
//! catch, and a defect in a relay moves one of them.
//!
//! **And this frame still names no stamp.** `Crossing::absorb` reads the
//! payload the way `Event::decode` reads it — inside `abi/`, the crate that
//! declares the field and carries an `INPUT_PATH` row for it — and hands back a
//! checksum. A checksum is not a time: it is not ordered against anything, not
//! invertible, and there is no expression in this file that could subtract one
//! from another and publish the difference as a latency. That is the whole of
//! why `kernel/` stays off `lint-stamp`'s list, and RFC 0124 is the argument at
//! length, including what would reverse it.
//!
//! # What the compositor has to do to drain this itself, and why the four
//! steps are not written here
//!
//! They are `docs/rfc/0124`'s section *What the consumer has to do*, in full,
//! with the call names spelled out. They are there and not here because
//! **writing them here is a red build**, which is a thing this file learned by
//! doing it: `cargo xtask lint-stamp` decides whether a crate is on the input
//! path by looking for that vocabulary in the crate's text, prose included, and
//! a paragraph in this module naming the stamp's type and its wire field earns
//! `kernel/` an `INPUT_PATH` row — which the one `rdtsc` in this tree, the
//! counter `crate::smp` reads to bound a spin, turns red immediately.
//!
//! That is the rule working rather than the rule in the way, and it is worth a
//! reader's attention because it is the sharper half of why the frame folds a
//! checksum instead of carrying a position: this file may not so much as
//! *describe* the number, let alone hold one. An RFC is not a workspace member,
//! so the description lives where a description belongs.
//!
//! The one-sentence version, in words this file is allowed to use: the
//! compositor holds the other end of the driver's data channel itself, decodes
//! each entry with the same decoder this frame calls, rebuilds the reading from
//! the field that entry carries, and folds what it drained so that it can check
//! the driver's published word. What it must not do is read a clock when an
//! entry arrives and call that the event's time.
//!
//! # What this demonstration does not show
//!
//! One device, one queue, relative motion, and no seat. It says nothing about
//! two input devices — [`virtio::VIRTIO_INPUT_MODERN`] cannot tell them apart
//! and the frame takes the first — nothing about focus or about which of several
//! clients an event belongs to, and nothing about latency: what an entry here
//! carries is virtual time out of a seed, as `user/virtio-input/src/clock.rs`
//! spends a page saying, and `E5-B06` is where a device times its own events.
//! What it shows is that a person moving a pointer moves a node in a scene graph
//! held at ring 3, through one reading, two rings, one translation and one
//! commit.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, rights};
use f_abi::input::{Crossing, Entry as InputEntry, Event, PAYLOAD_BYTES};
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
/// which is what a component gets when it pays for its own channel; this boot
/// drains as the entries arrive rather than sizing a ring to hold the run.
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

/// How many events the frame will hold between the two components.
///
/// Sixty-four, which is far more than this boot's script injects and small
/// enough to sit on a stack. A run that fills it fails rather than truncating —
/// [`Trouble::Overflowed`] — for the reason the module comment gives.
/// Unit: entries.
pub const EVENTS_MAX: usize = 64;

/// The layer the pointer's node hangs under.
const ROOT_NODE: u32 = 1;

/// The node the pointer moves.
///
/// Two, because node one is the layer it hangs under. Both are created by this
/// boot's own setup deltas and neither is derived from anything the device said,
/// which is what makes the *transforms* the only part of the compositor's work
/// that came from outside the machine.
const POINTER_NODE: u32 = 2;

/// How many nodes the setup deltas create. Unit: nodes.
const SETUP_NODES: u64 = 2;

/// One, in the 16.16 fixed point `f_abi::scene::SetTransform` is written in.
///
/// The matrix this boot sends is the identity with a translation, so every delta
/// differs from its neighbour in exactly the two fields the device decided and
/// in none of the four it did not. A scale that moved with the pointer would
/// make *the compositor applied this delta* and *the compositor applied a delta*
/// the same observation.
/// Unit: none — a ratio, scaled by 65 536.
const IDENTITY_X65536: i64 = 65_536;

/// What this boot tells the compositor its display scans out.
///
/// A sixty-hertz frame, which is what `kernel/src/compositor.rs` tells it and
/// for the same reason: there is no display in this boot, so it is a statement
/// rather than a measurement, and what it has to be is a period the one frame
/// this boot commits does not come close to.
/// Unit: nanoseconds.
const SCANOUT_PERIOD_NANOS: u64 = 16_666_667;

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
    /// Drain the driver and hand every event to the compositor.
    Deliver,
    /// Drain the driver, decode every event, and hand on none of them.
    ///
    /// **The control, and it is the frame's own decision rather than the
    /// device's.** The module comment argues why it is this and not an entry
    /// with no reading on it.
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

    /// Does the frame hand what it drained to the compositor?
    #[must_use]
    pub const fn hands_on(self) -> bool {
        matches!(self, Self::Deliver)
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
    /// A component could not be built as a process, carrying which step.
    Process(crate::process::Error),
    /// A component's state tree could not be published or read back.
    StateTree(i32),
    /// The core a component was given never took it or never gave it back.
    Scheduled(usize),
    /// A core was still holding its job when the frame's bound passed. Carries
    /// that bound. Unit: microseconds.
    Overdue(u64),
    /// The device produced more events than the frame is willing to hold
    /// between the two components. Carries the bound. Unit: entries.
    Overflowed(usize),
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
            Self::Process(_) => "a component could not be built as a process",
            Self::StateTree(_) => "a component's state tree could not be published or read",
            Self::Scheduled(_) => "the core a component was given never took it or gave it back",
            Self::Overdue(_) => "a component's core did not report finished inside the bound",
            Self::Overflowed(_) => {
                "the device produced more events than the frame will hold between the two \
                 components, and an input path that forgets the end of a gesture is worse than \
                 one that stops"
            }
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

/// One event, as much of it as the relay needs.
///
/// Two numbers and not an `f_abi::input::Event`, and the subtraction is the
/// decision rather than a saving. What a delta is built from is the translation,
/// so that is what is carried forward; keeping the whole decoded event would be
/// keeping the reading it arrived with, in a crate whose whole claim on this
/// path is that it never holds one. The module comment on why `kernel/` is not
/// on `lint-stamp`'s list is the same sentence from the other end, and this type
/// is where it is true rather than argued.
#[derive(Clone, Copy, Default)]
pub struct Kept {
    /// Where the pointer was when this event happened, along x.
    /// Unit: device pixels, scaled by 65 536.
    pub tx_x65536: i64,
    /// The same along y. Unit: device pixels, scaled by 65 536.
    pub ty_x65536: i64,
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
    /// Entries built and not submitted, because the peer had not drained.
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
    /// driver submitted, taken in the driver, on the component's side of the
    /// boundary. Unit: none — a checksum.
    pub crossing: u64,
    /// How many entries went into that word, as the component counted them.
    /// Unit: entries.
    pub crossed: u64,
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
        })
    }
}

/// What the driver stage saw, from both sides of the boundary.
pub struct Produced {
    /// What the component published about itself.
    pub board: Reported,
    /// Whether it ended by `EXIT` rather than in a fault.
    pub exited: bool,
    /// Entries the frame took off the data ring. Unit: entries.
    pub drained: u64,
    /// Entries that decoded. Unit: entries.
    ///
    /// **Equal to [`Produced::drained`] or the run is a failure**, and that
    /// equality is where `E3-B04a` lands in this boot: `Event::decode` refuses a
    /// payload carrying no reading, so an entry that decoded is an entry the
    /// driver took one for.
    pub decoded: u64,
    /// Entries that did not. Unit: entries.
    pub refused: u64,
    /// How many of them were pointer motion. Unit: entries.
    pub motions: u64,
    /// Where the pointer ended up, along x.
    /// Unit: device pixels, scaled by 65 536.
    pub last_x_x65536: i64,
    /// The same along y. Unit: device pixels, scaled by 65 536.
    pub last_y_x65536: i64,
    /// Operations the driver asked of the frame on its control ring.
    ///
    /// **Zero, and that is not a failure.** The three drivers before this one
    /// submit RFC 0047's `DEVICE_MAP` because they hold a client's buffer and
    /// may not grant themselves a device address. This one holds no client
    /// buffer: the only memory its device ever touches is the queue region its
    /// manifest routed, which the frame translated before the component's first
    /// instruction. A verdict that required a translation here would be a check
    /// passing on three datapaths for a reason that is not the property it
    /// names. Unit: operations.
    pub asked: u32,
    /// Whether anything outside the machine said it had injected.
    pub acknowledged: bool,
    /// The events, in the order they arrived.
    pub kept: [Kept; EVENTS_MAX],
    /// How many of [`Produced::kept`] are filled. Unit: entries.
    pub events: usize,
    /// What arrived, folded into one word by this frame.
    ///
    /// The **consumer's** half of the crossing attestation, and the reason it
    /// is a fold rather than a copy is the one this file spends a section on:
    /// the frame may not hold the reading an entry carries. `Crossing::absorb`
    /// reads the payload the way `Event::decode` reads it — inside `abi/`,
    /// which is the crate that declares the field and is on `lint-stamp`'s
    /// path — and hands back a checksum, which is not a time, is not
    /// invertible, and is not anything this frame could publish a latency
    /// from. RFC 0124 is that argument at length.
    /// Unit: none — a checksum.
    pub crossing: Crossing,
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
    /// How many of the submitted deltas came from an input event. Unit: deltas.
    pub from_events: u64,
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

impl Report {
    /// How many deltas should have reached the graph. Unit: deltas.
    ///
    /// Derived from what the frame actually handed on rather than from the
    /// harness's intention, which is what makes the comparison below a
    /// comparison: the component counts what reached its graph and this counts
    /// what left this file, and neither number is computed from the other.
    #[must_use]
    pub fn expected_edits(&self) -> u64 {
        SETUP_NODES.saturating_add(self.consumed.from_events)
    }

    /// Did the machine do what this half asked of it?
    ///
    /// # Errors
    ///
    /// A sentence for the boot log, naming the clause that did not hold.
    pub fn verdict(&self) -> Result<(), &'static str> {
        self.driver_verdict()?;
        self.compositor_verdict()
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
        // Where `E3-B04a` lands, and it is three readings none of which is
        // derived from another: the component says how many reports it timed,
        // the frame says how many entries decoded — and `Event::decode` refuses
        // a payload with no reading on it, so an entry that decoded carried one
        // — and the clock's own position says how many times it advanced.
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
        if seen.refused != 0 || seen.decoded != seen.drained {
            return Err(
                "an entry the frame took off the data ring did not decode: every input entry \
                 carries a reading because the decoder refuses one that does not, so a refusal \
                 here is an event that reached a consumer with no place in the latency chain",
            );
        }
        if seen.board.malformed != 0 {
            return Err("the driver built an entry its own format check refused, which is an \
                 encoder about to submit what its peer's decoder will not take");
        }
        if seen.board.dropped != 0 {
            return Err(
                "the driver could not submit an entry because this frame had not drained: the \
                 events exist and the consumer never saw them, which is the one failure on this \
                 path that leaves every other count looking healthy",
            );
        }
        if seen.board.submitted != seen.drained {
            return Err("the driver and the frame disagree about how many entries crossed the \
                 data ring");
        }
        // --- the crossing, `E3-B04f` ----------------------------------------
        //
        // The counts above say *how many* entries crossed and every clause so
        // far has been about a tally. None of them says the entries that
        // arrived are the entries that were sent: a relay that re-stamped on
        // arrival, handed them on in a different order, or moved a coordinate
        // by one satisfies every count in this function, and the first of those
        // three is what `input/src/stamp.rs` exists to prevent.
        //
        // So the driver folds what it submitted, this frame folds what it
        // drained, and the two words are required to agree. Neither is computed
        // from the other: the driver's is taken in `Outbound::put` on the far
        // side of the boundary and published on its routing page; this one is
        // built in `drain` out of entries that arrived. What they share is the
        // implementation, which is `E3-B04c`'s distinction and is why a defect
        // inside the fold is `abi`'s corpus to catch and not this boot's.
        if seen.board.crossed != seen.board.submitted {
            return Err(
                "the driver folded a different number of entries than it counted submitting, so \
                 its own submit path counts an entry it did not attest to or attests to one it \
                 did not send",
            );
        }
        if seen.crossing.absorbed() != seen.decoded {
            return Err("this frame folded a different number of entries than it decoded, which \
                 is this file's own arithmetic and not the driver's");
        }
        if !seen.crossing.agrees_with(seen.board.crossing) {
            return Err(
                "what arrived is not what was sent: the driver's fold over the entries it \
                 submitted and this frame's fold over the entries it drained disagree, and the \
                 fold covers the opcode and the whole payload - so a reading minted on arrival, \
                 a pair handed on out of order, one dropped out of the middle, or a coordinate \
                 moved by one all land here. `agrees_with` also refuses a fold of nothing, so a \
                 run in which no entry crossed fails on this line rather than passing quietly",
            );
        }
        if seen.motions == 0 {
            return Err(
                "no entry the frame drained was pointer motion, so nothing in this run carries \
                 a coordinate the harness can check its own injection against",
            );
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

    /// The half that is about what the compositor did with them.
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
            return Err("the compositor's graph does not hold the two nodes this boot's setup \
                 deltas create");
        }
        // **The clause both halves exist for.** `edits` is deltas that reached
        // the graph, counted by the component; `expected_edits` is the two setup
        // deltas plus however many events this half handed on, counted by this
        // file from what it drained. On `deliver` the second term is the number
        // of events the device produced and on `withheld` it is zero, and the
        // same sentence is what fails if a compositor invented an edit or if
        // this frame's relay quietly dropped one.
        if seen.board.edits != self.expected_edits() {
            return Err(
                "the compositor applied a different number of deltas than this frame handed it: \
                 on the delivering half that number is the two setup deltas plus one per input \
                 event, and on the withholding half it is the two setup deltas alone",
            );
        }
        if self.half.hands_on() {
            if seen.from_events != self.produced.decoded {
                return Err("the frame did not hand on every event it decoded, so the count the \
                     compositor agreed with is not the count the device produced");
            }
        } else if seen.from_events != 0 {
            return Err(
                "the withholding half handed an event to the compositor, so it is not the \
                 control it claims to be and the delivering half's count is evidence of nothing",
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
        // --- the late latch, `E3-B01i`, as the negative it is here -----------
        //
        // The component holds the latch and was told which node carries the
        // pointer. It was never told **where** the pointer is, and that is a
        // property of this arrangement rather than an omission: the three words
        // that would say so carry the driver's stamp, `kernel/` is deliberately
        // not on `lint-stamp`'s input path, and a frame that carried one would
        // need a row that the single `rdtsc` in this tree — the counter
        // `crate::smp` reads to bound a spin — immediately turns red.
        // `INPUT_PATH` has no exemption to write, by design.
        //
        // So this is a clause rather than a comment, and **it is meant to go
        // red**: the diff that gives the compositor a route to a stamped
        // position — a second worker core, so the driver and the compositor run
        // at once and the component decodes the entry itself, or the input
        // router `E3-B04`'s parent owes — is the diff that closes `E3-B01i`'s
        // boot half, and it arrives at this line first.
        if seen.board.pointer_reports != 0 {
            return Err("the compositor was told where the pointer is, which this frame has no \
                 stamped route to say — `E3-B01i`'s boot half is open and this is the clause \
                 that says so");
        }
        if seen.board.pointer_unstamped == 0 {
            return Err("the compositor refused no unstamped reading, so either it never looked \
                 at the pointer words or it took a page of zeroes for a position at the origin");
        }
        if seen.board.latches != 0 || seen.board.latch_declines != seen.board.frames {
            return Err("the compositor latched a frame, or declined a number of frames that is \
                 not the number it closed: with no position reported every closed frame \
                 declines exactly once");
        }
        Ok(())
    }
}

/// Print what happened, one subject per line.
pub fn report_lines(report: &Report) {
    let seen = &report.produced;
    crate::kprintln!(
        "  input         a fourth driver outside the frame, and the {} half: {}",
        report.half.name(),
        match report.half {
            Half::Deliver => "every event the device produced is handed to a compositor",
            Half::Withheld =>
                "the same events, drained and decoded, and none of them handed on - so a delta \
                 the compositor applies came from somewhere else",
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
         report; {} entr(y/ies) decoded of {} drained, {} refused",
        seen.board.stamped,
        seen.board.clock_at,
        STAMP_TICK_NANOS,
        seen.decoded,
        seen.drained,
        seen.refused,
    );
    // The crossing, on its own line, because it is the only thing in this log
    // that two stages computed separately about one sequence. The counts are
    // printed beside the words so that a reader of a red boot can tell a
    // disagreement about *how many* from a disagreement about *what*.
    crate::kprintln!(
        "  input cross   the driver folded {} entr(y/ies) into {:#018x} and this frame folded \
         {} into {:#018x}; {}",
        seen.board.crossed,
        seen.board.crossing,
        seen.crossing.absorbed(),
        seen.crossing.word(),
        if seen.crossing.agrees_with(seen.board.crossing) {
            "what arrived is what was sent, stamps and bodies alike"
        } else {
            "THEY DISAGREE"
        },
    );
    crate::kprintln!(
        "  input relay   {} event(s) kept, {} of them pointer motion, {} handed to the \
         compositor; the harness {}",
        seen.events,
        seen.motions,
        report.consumed.from_events,
        if seen.acknowledged { "acknowledged" } else { "NEVER ANSWERED" },
    );
    // The line the harness checks its own injection against. It carries what
    // nothing outside the machine can derive - where the driver's accumulator
    // ended up - and the harness holds the other half: how far it asked the
    // pointer to move, in whole device pixels. Neither side holds the other's
    // number, which is what makes the comparison a comparison.
    crate::kprintln!(
        "  input pointer x {} y {} in units of 1/65536 device pixel, after {} motion event(s)",
        seen.last_x_x65536,
        seen.last_y_x65536,
        seen.motions,
    );
    crate::kprintln!(
        "  input scene   {} delta(s) submitted, {} answered, {} refused; the component applied \
         {} of {} expected, closed {} frame(s), holds {} node(s)",
        report.consumed.submitted,
        report.consumed.completed,
        report.consumed.refused,
        report.consumed.board.edits,
        report.expected_edits(),
        report.consumed.board.frames,
        report.consumed.board.live,
    );
    crate::kprintln!(
        "  input tree    {} node(s) from the compositor's manifest: frames {}, edits {}, nodes \
         {}; outcome {}",
        report.consumed.tree_nodes,
        report.consumed.tree[0],
        report.consumed.tree[1],
        report.consumed.tree[2],
        report.consumed.board.outcome,
    );
    // The late latch, `E3-B01i`, and this line is a **negative** on both halves.
    // The component holds the mechanism and is told which node the pointer
    // rides; what it is never told is where the pointer is, because the three
    // words that would say so carry the driver's stamp and this frame may not
    // hold one. `Report::compositor_verdict` is where that is a clause rather
    // than a sentence, and the comment there says what route closes it.
    crate::kprintln!(
        "  input latch   the compositor was told node {} carries the pointer and was told no \
         position: {} report(s) taken, {} frame(s) latched, {} declined — the relay carries \
         no stamp, so there is nothing to predict from",
        POINTER_NODE,
        report.consumed.board.pointer_reports,
        report.consumed.board.latches,
        report.consumed.board.latch_declines,
    );
    crate::kprintln!(
        "  input unstamp {} reading(s) refused for carrying no stamp, which is what the page \
         this frame never wrote reads as — f_abi::input::NOT_STAMPED, at the one place a \
         position reaches a component without crossing a ring",
        report.consumed.board.pointer_unstamped,
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

    // SAFETY: allocated above, at the order each is freed at, and the device is
    // detached and stripped of bus mastering.
    unsafe { frames.free(granted) };
    // SAFETY: as above.
    unsafe { frames.free(wire) };

    let produced = outcome?;

    // And now the second component, on the core the first one has given back.
    // SAFETY: the caller's guarantee about the boot processor, the allocator,
    // the direct map and the worker core, passed down; the driver has been
    // reaped, so that core is idle again.
    let consumed = unsafe { consume(frames, space, features, half, boot, scheduling, &produced) }?;

    // Everything both stages took, back where it started, with the unit's own
    // retained tables taken out. Two numbers rather than a tolerance: a check
    // with slack in it is a check that stops noticing the first frame.
    let retained = unit.tables().len().saturating_sub(kept_tables) as u64;
    if frames.free_count().saturating_add(retained) != before {
        return Err(Trouble::Leaked);
    }

    Ok(Report {
        half,
        declared,
        bdf: found.bdf,
        windows: found.pages,
        cpu: scheduling.cpu,
        produced,
        consumed,
    })
}

/// The driver stage: stand `user/virtio-input` up, hold still while somebody
/// moves the pointer, and keep what arrives.
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
    // The data ring, and **the frame keeps the server's end of it**, which is
    // the opposite of every other datapath in this tree. The component holds the
    // client's end because nobody asks for an input event: there is a device
    // that produces and a peer that drains.
    // SAFETY: `wire` was allocated zeroed by the caller, is frame-aligned and is
    // `FRAME_SIZE` bytes with no pointer into it held anywhere else.
    let server_end =
        unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
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
    let events = Consumer::new(server_end.channel()).ok_or(Trouble::Channel(0))?;
    let arena = server_end.arena();

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

    let mut seen = Drained::default();
    let tsc_khz = setup.scheduling.tsc_khz;

    // **The line the harness waits for.** It is printed after the component is
    // running and before the frame starts draining, because the events have to
    // be injected into a machine whose driver is already serving: an event
    // delivered to a device nobody has started is an event the emulator queues
    // and the driver finds later or not at all. `kernel/src/gpu.rs`'s marker is
    // the other way round - it is printed after its verdict, because a picture
    // survives the boot and an input event does not.
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
        supervising.serve()?;
        drain(&events, &arena, &mut seen)?;

        if settle_until.is_none() && crate::arch::x86_64::serial::Serial.received().is_some() {
            seen.acknowledged = true;
            settle_until = Some(crate::smp::deadline_after(tsc_khz, SETTLE_MICROS));
        }
        match settle_until {
            Some(deadline) if crate::smp::past(deadline) => break,
            None if crate::smp::past(waiting_until) => break,
            _ => core::hint::spin_loop(),
        }
    }

    // Told to stop whatever happened above, because a driver left serving a
    // frame that has gone is a core this boot never gets back.
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
    if seen.overflowed {
        return Err(Trouble::Overflowed(EVENTS_MAX));
    }

    Ok(Produced {
        board,
        exited,
        drained: seen.drained,
        decoded: seen.decoded,
        refused: seen.refused,
        motions: seen.motions,
        last_x_x65536: seen.last_x_x65536,
        last_y_x65536: seen.last_y_x65536,
        asked,
        acknowledged: seen.acknowledged,
        kept: seen.kept,
        events: seen.events,
        crossing: seen.crossing,
    })
}

/// What the frame has taken off the data ring so far.
struct Drained {
    drained: u64,
    decoded: u64,
    refused: u64,
    motions: u64,
    last_x_x65536: i64,
    last_y_x65536: i64,
    acknowledged: bool,
    overflowed: bool,
    kept: [Kept; EVENTS_MAX],
    events: usize,
    crossing: Crossing,
}

impl Default for Drained {
    fn default() -> Self {
        Self {
            drained: 0,
            decoded: 0,
            refused: 0,
            motions: 0,
            last_x_x65536: 0,
            last_y_x65536: 0,
            acknowledged: false,
            overflowed: false,
            kept: [Kept { tx_x65536: 0, ty_x65536: 0 }; EVENTS_MAX],
            events: 0,
            crossing: Crossing::new(),
        }
    }
}

/// Take everything the driver has submitted and keep what a delta will be built
/// from.
///
/// # Why every entry is decoded and not merely counted
///
/// Because decoding is the whole of what this frame checks about the reading an
/// entry carries. `f_abi::input::Event::decode` refuses a payload whose first
/// eight bytes are the not-stamped value, and it is the same function a consumer
/// anywhere else on this path would call - so an entry that decoded here is an
/// entry the driver called the one reading for, and this file establishes that
/// without reading a clock, naming the field, or holding an opinion about what
/// time it is. `E3-B04a`, in a boot.
///
/// # Errors
///
/// [`Trouble::Channel`] for a ring that stopped validating under the frame,
/// which is a component that has stopped speaking.
fn drain(events: &Consumer<'_>, arena: &Arena<'_>, seen: &mut Drained) -> Result<(), Trouble> {
    loop {
        let Some(entry) = events.pop().map_err(|_| Trouble::Channel(0))? else { return Ok(()) };
        seen.drained = seen.drained.saturating_add(1);

        let mut payload = [0u8; PAYLOAD_BYTES];
        let Ok(offset) = usize::try_from(entry.offset) else {
            seen.refused = seen.refused.saturating_add(1);
            continue;
        };
        if !arena.copy_out(offset, &mut payload) {
            seen.refused = seen.refused.saturating_add(1);
            continue;
        }
        let Ok(event) = Event::decode(&entry, &payload) else {
            seen.refused = seen.refused.saturating_add(1);
            continue;
        };
        seen.decoded = seen.decoded.saturating_add(1);
        // The consumer's half of the attestation, folded here and from nowhere
        // else, in the order the entries came off the ring. It is taken from
        // the decoded event rather than from the bytes in the arena on purpose:
        // what is being attested is that the *event* crossed unchanged, and an
        // entry that did not decode is not an event at all — it is counted in
        // `refused`, which the verdict already requires to be zero.
        seen.crossing.absorb(&event);

        // Where the pointer is, as the driver's accumulator has it. A motion
        // event carries the position and every other opcode carries none, so an
        // event that is not motion is relayed at the position the last motion
        // left - which keeps the relay one delta per event without inventing a
        // coordinate the device never reported.
        if let InputEntry::PointerMotion(motion) = event.body {
            seen.motions = seen.motions.saturating_add(1);
            seen.last_x_x65536 = i64::from(motion.x_x65536);
            seen.last_y_x65536 = i64::from(motion.y_x65536);
        }

        let kept = Kept { tx_x65536: seen.last_x_x65536, ty_x65536: seen.last_y_x65536 };
        match seen.kept.get_mut(seen.events) {
            Some(slot) => {
                *slot = kept;
                seen.events += 1;
            }
            // Recorded rather than acted on here, because this function runs
            // inside the driver's run and stopping in the middle of it would
            // leave a core holding a job. `produce` fails the boot on it once
            // the component has been told to stop and reaped.
            None => seen.overflowed = true,
        }
    }
}

/// The compositor stage: stand `user/compositor` up on the core the driver has
/// given back, and commit one frame built out of what arrived.
///
/// # Safety
///
/// As [`demonstrate`], after the driver has been reaped.
unsafe fn consume(
    frames: &mut FrameAllocator,
    kernel: &AddressSpace,
    features: Features,
    half: Half,
    boot: &BootInfo,
    on: Scheduling,
    produced: &Produced,
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

    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Channel(0))?;
    let wire = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;
    let at = frames.virt(wire);
    // Here the frame is the **client** and the component is the server, which is
    // the other way round from the ring above it. Both ends are in this file,
    // and that is the cost `E1-B05` still owes: the courier holds one end of
    // each channel because nothing else in this boot can.
    // SAFETY: `wire` was allocated zeroed just above, is frame-aligned and is
    // `FRAME_SIZE` bytes with no pointer into it held anywhere else.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and every
    // accessor hands out atomics and `UnsafeCell`s rather than references.
    let client_end = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;

    // SAFETY: the caller's guarantee about `kernel`, `frames` and `cpu`, plus
    // `wire` being a frame this function allocated and holds the far end of.
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
                // This component's ring carries its payloads inline, in the
                // channel's own arena, so a registered region would be a page
                // nobody ever addresses.
                buffers: 0,
                buffer_bytes: 0,
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
        // that the difference between them stays the one line in `commit` that
        // hands the events on: a control whose compositor had been told to latch
        // nothing would be a control for two things at once, and the withholding
        // half's zero latches is then a fact about the events rather than about
        // this word.
        (scene_routing::at::POINTER_NODE, u64::from(POINTER_NODE)),
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
    let driven = commit(&producer, &reaper, &arena, &board, &mut env, on.tsc_khz, half, produced);

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
        from_events: seen.from_events,
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
    from_events: u64,
}

/// Submit the two setup deltas, one delta per event this half hands on, and one
/// commit.
///
/// # Why one at a time
///
/// Because the payload travels in the channel's arena and this client writes
/// every payload at the same offset, so a second entry submitted before the
/// first was answered would overwrite bytes the component had not read yet.
/// `kernel/src/compositor.rs` makes the same choice for the same reason, and
/// adds the one that matters for the epoch: batching deltas into one crossing is
/// `E3-B01`'s exit and `E3-B01j` is the task that counts the crossings, so a
/// client that batched here would be building that task's evidence without its
/// counter.
///
/// # Errors
///
/// [`Trouble::NotCommitted`] where the ring will not take a delta, or where a
/// completion does not arrive inside the bound.
#[expect(
    clippy::too_many_arguments,
    reason = "the client, its ring, its arena, its page, its clock, a bound and what it is \
              relaying; a struct holding them would be a type that exists so a lint passes"
)]
fn commit(
    producer: &Producer<'_>,
    reaper: &Collector<'_>,
    arena: &Arena<'_>,
    board: &Window,
    env: &mut SeededEnv,
    tsc_khz: u64,
    half: Half,
    produced: &Produced,
) -> Result<Sent, Trouble> {
    let mut seen = Sent { submitted: 0, completed: 0, refused: 0, from_events: 0 };
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

    // The surface, and the node that will move on it. Both sent on both halves,
    // which is what makes the difference between the halves exactly one thing.
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

    // And the events, or not. This `if` is the whole of the difference between
    // the two halves of this command, and it is deliberately one line: a control
    // that differed from its positive in two places would be a control for two
    // things at once.
    if half.hands_on() {
        for kept in produced.kept.iter().take(produced.events) {
            send(
                &mut seen,
                SceneEntry::SetTransform(SetTransform {
                    node: POINTER_NODE,
                    a_x65536: IDENTITY_X65536,
                    b_x65536: 0,
                    c_x65536: 0,
                    d_x65536: IDENTITY_X65536,
                    tx_x65536: kept.tx_x65536,
                    ty_x65536: kept.ty_x65536,
                }),
                0,
            )?;
            seen.from_events = seen.from_events.saturating_add(1);
        }
    }

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
